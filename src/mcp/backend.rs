//! Device IO for MCP. No terminal output or backend response bodies are returned.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use super::snapshot::Snapshot;
use eyre::{Result, ensure, eyre};
use rmcp::{RoleServer, service::RequestContext};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::action::{Action, ControlCommand};
use crate::cli::GlobalArgs;
use crate::mqtt::{self, MqttOptions};

pub(super) trait Backend: Send + Sync + 'static {
    fn heizung(
        &self,
        request: crate::heizung::Request,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<Value>> + Send;
    fn pizzamat_user(&self) -> &str;
    fn pizzamat(
        &self,
        request: crate::pizzamat::Request,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<Value>> + Send;
    fn send(
        &self,
        command: ControlCommand,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<Value>> + Send;
    fn snapshot(
        &self,
        snapshot: Snapshot,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<Value>> + Send;
}

pub(super) struct DeviceBackend {
    mqtt: MqttOptions,
    smoothie_url: String,
    heizung: crate::heizung::Options,
    pizzamat: crate::pizzamat::Options,
    http: reqwest::Client,
    dry_run: bool,
    wait: Duration,
    // Serialize device operations, including blocking work after caller cancellation.
    gate: Arc<Mutex<()>>,
}

impl DeviceBackend {
    pub fn new(global: &GlobalArgs) -> Result<Self> {
        ensure!(!global.no_clean_session, "MCP requires clean MQTT sessions");
        let mut mqtt = MqttOptions::new(global.broker.clone());
        if let Some(id) = &global.client_id {
            mqtt.client_id = id.clone();
        }
        mqtt.username = global.username.clone();
        mqtt.password = global.password.clone();
        mqtt.keep_alive = Duration::from_secs(global.keep_alive.max(1));
        mqtt.connect_timeout = Duration::from_secs(global.connect_timeout.clamp(1, 30));
        Ok(Self {
            mqtt,
            smoothie_url: global.smoothiesaur_url.clone(),
            heizung: crate::heizung::Options::from_global(global),
            pizzamat: crate::pizzamat::Options::from_global(global),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
            dry_run: global.dry_run,
            wait: Duration::from_secs(global.wait.clamp(1, 30)),
            gate: Arc::new(Mutex::new(())),
        })
    }
}

impl Backend for DeviceBackend {
    async fn heizung(
        &self,
        request: crate::heizung::Request,
        context: RequestContext<RoleServer>,
    ) -> Result<Value> {
        let _guard = tokio::select! {
            guard = self.gate.lock() => guard,
            _ = context.ct.cancelled() => return Err(eyre!("request cancelled")),
        };
        ensure!(!context.ct.is_cancelled(), "request cancelled");
        // Finish the bounded exchange and close its exclusive queue even if
        // the caller cancels after submission. Never replay a setting.
        crate::heizung::transport::execute(&self.heizung, &request, self.dry_run).await
    }

    fn pizzamat_user(&self) -> &str {
        &self.pizzamat.user
    }

    async fn pizzamat(
        &self,
        request: crate::pizzamat::Request,
        context: RequestContext<RoleServer>,
    ) -> Result<Value> {
        let _guard = tokio::select! {
            guard = self.gate.lock() => guard,
            _ = context.ct.cancelled() => return Err(eyre!("request cancelled")),
        };
        ensure!(!context.ct.is_cancelled(), "request cancelled");
        // Finish the bounded exchange and close its private reply queue even if
        // the caller cancels after submission. Never replay order mutations.
        crate::pizzamat::transport::execute(&self.pizzamat, &request, self.dry_run).await
    }

    async fn snapshot(
        &self,
        snapshot: Snapshot,
        context: RequestContext<RoleServer>,
    ) -> Result<Value> {
        if self.dry_run {
            return Ok(json!({"dry_run": true}));
        }
        let guard = tokio::select! {
            guard = self.gate.clone().lock_owned() => guard,
            _ = context.ct.cancelled() => return Err(eyre!("request cancelled")),
        };
        let options = self.mqtt.clone();
        let wait = self.wait;
        tokio::task::spawn_blocking(move || -> Result<Value> {
            let _guard = guard;
            ensure!(!context.ct.is_cancelled(), "request cancelled");
            let mut client =
                mqtt::connect_once(&options).map_err(|_| eyre!("MQTT connection failed"))?;
            client.set_timeout(Duration::from_secs(5));
            let receiver = client.start_consuming();
            let topics = snapshot.topics();
            let result = (|| {
                client
                    .subscribe_many_same_qos(&topics, 0)
                    .map_err(|_| eyre!("MQTT subscription failed"))?;
                let deadline = std::time::Instant::now() + wait;
                let mut messages = std::collections::HashMap::new();
                let mut received = 0;
                while messages.len() < topics.len() && received < 512 {
                    ensure!(!context.ct.is_cancelled(), "request cancelled");
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
                        Ok(Some(message)) => {
                            received += 1;
                            if !topics.iter().any(|topic| topic == message.topic())
                                || message.payload().len() > 4096
                            {
                                continue;
                            }
                            if let Ok(payload) = std::str::from_utf8(message.payload()) {
                                messages.insert(message.topic().to_string(), payload.to_string());
                            }
                        }
                        Ok(None) => return Err(eyre!("MQTT connection closed")),
                        Err(_) => ensure!(client.is_connected(), "MQTT connection closed"),
                    }
                }
                Ok(snapshot.decode(&messages))
            })();
            mqtt::disconnect(&client);
            result
        })
        .await
        .map_err(|_| eyre!("MQTT worker failed"))?
    }

    async fn send(
        &self,
        command: ControlCommand,
        context: RequestContext<RoleServer>,
    ) -> Result<Value> {
        let actions = command.expand(&self.smoothie_url)?;
        let count = actions.len();
        if self.dry_run {
            return Ok(json!({"dry_run": true, "actions": count, "submitted": false}));
        }
        let guard = tokio::select! {
            guard = self.gate.clone().lock_owned() => guard,
            _ = context.ct.cancelled() => return Err(eyre!("request cancelled")),
        };
        ensure!(!context.ct.is_cancelled(), "request cancelled");
        if actions
            .iter()
            .all(|action| matches!(action, Action::Publish { .. }))
        {
            let options = self.mqtt.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let _guard = guard;
                ensure!(!context.ct.is_cancelled(), "request cancelled");
                let mut client =
                    mqtt::connect_once(&options).map_err(|_| eyre!("MQTT connection failed"))?;
                client.set_timeout(Duration::from_secs(5));
                let result = (|| {
                    for action in actions {
                        ensure!(
                            !context.ct.is_cancelled(),
                            "request cancelled; earlier actions may have been submitted"
                        );
                        if let Action::Publish {
                            topic,
                            payload,
                            retain,
                        } = action
                        {
                            mqtt::publish(&client, &topic, &payload, retain).map_err(|_| {
                                eyre!(
                                    "MQTT publish failed; earlier actions may have been submitted"
                                )
                            })?;
                        }
                    }
                    Ok(())
                })();
                mqtt::disconnect(&client);
                result
            })
            .await
            .map_err(|_| eyre!("MQTT worker failed"))??;
        } else {
            let _guard = guard;
            for action in actions {
                ensure!(!context.ct.is_cancelled(), "request cancelled");
                let Action::Http { url, body } = action else {
                    return Err(eyre!("unsupported mixed transport operation"));
                };
                let response = self
                    .http
                    .post(url)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(body)
                    .send()
                    .await
                    .map_err(|_| eyre!("device HTTP request failed"))?;
                ensure!(
                    response.status().is_success(),
                    "device HTTP command rejected"
                );
            }
        }
        Ok(json!({"dry_run": false, "actions": count, "submitted": true}))
    }
}
