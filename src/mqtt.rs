//! Normal TCP MQTT transport and the TUI reconnect supervisor.
//!
//! WebSockets are intentionally not supported. The supervisor lives on a
//! dedicated thread because paho-mqtt's connect/disconnect calls are blocking;
//! the TUI stays responsive and simply consumes [`MqttEvent`]s.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use paho_mqtt as paho;
use thiserror::Error;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError};

use crate::action::TOPIC_ALERT_CONDITION;

/// Topics the TUI subscribes to after every (re)connect.
pub const STATUS_TOPICS: &[&str] = &[
    TOPIC_ALERT_CONDITION,
    "light/esp/status/#",
    "light/esp/state",
    "knob/get/#",
    "smoothie/#",
];

pub const DEFAULT_BROKER: &str = "tcp://mqtt.subraum.c3pb.de:1883";

#[derive(Clone, Debug)]
pub struct MqttOptions {
    pub broker: String,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub keep_alive: Duration,
    pub connect_timeout: Duration,
    pub clean_session: bool,
}

impl MqttOptions {
    pub fn new(broker: impl Into<String>) -> Self {
        Self {
            broker: broker.into(),
            client_id: generate_client_id(),
            username: None,
            password: None,
            keep_alive: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(5),
            clean_session: true,
        }
    }

    pub fn connect(&self) -> Result<paho::Client, MqttError> {
        let create = paho::CreateOptionsBuilder::new()
            .server_uri(self.broker.as_str())
            .client_id(self.client_id.as_str())
            .persistence(paho::PersistenceType::None)
            .finalize();

        let client = paho::Client::new(create).map_err(|err| MqttError::Create(err.to_string()))?;

        let mut connect_builder = paho::ConnectOptionsBuilder::new();
        connect_builder
            .keep_alive_interval(self.keep_alive)
            .clean_session(self.clean_session)
            .connect_timeout(self.connect_timeout);
        if let Some(username) = &self.username {
            connect_builder.user_name(username.as_str());
        }
        if let Some(password) = &self.password {
            connect_builder.password(password.as_str());
        }

        client
            .connect(connect_builder.finalize())
            .map_err(|err| MqttError::Connect(err.to_string()))?;

        Ok(client)
    }
}

fn generate_client_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let mixed = (nanos ^ (pid << 32) ^ (counter << 16) ^ 0x9e37_79b9_7f4a_7c15) as u64;
    format!("subraumctl-{:08x}", mixed & 0xffff_ffff)
}

#[derive(Debug, Error)]
pub enum MqttError {
    #[error("failed to create MQTT client: {0}")]
    Create(String),
    #[error("failed to connect: {0}")]
    Connect(String),
    #[error("failed to publish: {0}")]
    Publish(String),
    #[error("failed to subscribe: {0}")]
    Subscribe(String),
}

#[derive(Debug)]
pub enum MqttCommand {
    Connect,
    Disconnect,
    Reconnect,
    Publish {
        topic: String,
        payload: String,
        retain: bool,
    },
}

#[derive(Debug)]
pub enum MqttEvent {
    Connecting {
        attempt: u32,
    },
    Connected {
        generation: u64,
    },
    ConnectionLost {
        generation: u64,
        reason: String,
    },
    Disconnected {
        reason: String,
    },
    ConnectFailed {
        attempt: u32,
        reason: String,
    },
    Reconnecting {
        attempt: u32,
        delay: Duration,
    },
    Message {
        generation: u64,
        topic: String,
        payload: String,
    },
    Published {
        topic: String,
        payload: String,
        retain: bool,
    },
    PublishFailed {
        topic: String,
        payload: String,
        reason: String,
    },
    PublishRejected {
        topic: String,
        payload: String,
        reason: String,
    },
}

pub fn connect_once(options: &MqttOptions) -> Result<paho::Client, MqttError> {
    options.connect()
}

pub fn publish(
    client: &paho::Client,
    topic: &str,
    payload: &str,
    retain: bool,
) -> Result<(), MqttError> {
    let payload = payload.as_bytes().to_vec();
    let message = if retain {
        paho::Message::new_retained(topic, payload, 0)
    } else {
        paho::Message::new(topic, payload, 0)
    };
    client
        .publish(message)
        .map_err(|err| MqttError::Publish(err.to_string()))
}

pub fn subscribe(client: &paho::Client, topic: &str) -> Result<(), MqttError> {
    client
        .subscribe(topic, 0)
        .map(|_| ())
        .map_err(|err| MqttError::Subscribe(err.to_string()))
}

pub fn disconnect(client: &paho::Client) {
    client.stop_consuming();
    let _ = client.disconnect(None);
}

fn stop_client(client: &mut Option<paho::Client>) {
    if let Some(client) = client.take() {
        disconnect(&client);
    }
}

/// Spawn the blocking MQTT supervisor thread.
///
/// The thread processes commands, pumps incoming messages through the paho
/// receiver and reconnects with exponential backoff (1 s … 30 s) after a
/// connection loss. Manual disconnects pause automatic reconnects until a
/// `Connect`/`Reconnect` command arrives.
pub fn spawn_supervisor(
    options: MqttOptions,
    mut command_rx: UnboundedReceiver<MqttCommand>,
    event_tx: UnboundedSender<MqttEvent>,
) {
    std::thread::Builder::new()
        .name("subraumctl-mqtt".to_string())
        .spawn(move || {
            let mut client: Option<paho::Client> = None;
            let mut receiver = None;
            let mut connected = false;
            let mut manual_disconnect = false;
            let mut generation: u64 = 0;
            let mut attempt: u32 = 0;
            let mut backoff = Duration::from_secs(1);
            let mut next_retry: Option<Instant> = None;

            loop {
                let mut stop = false;
                loop {
                    match command_rx.try_recv() {
                        Ok(MqttCommand::Connect) => {
                            manual_disconnect = false;
                            if connected {
                                // Re-announce the current generation so the
                                // UI can leave its optimistic connecting state.
                                let _ = event_tx.send(MqttEvent::Connected { generation });
                            } else {
                                next_retry = Some(Instant::now());
                            }
                        }
                        Ok(MqttCommand::Disconnect) => {
                            manual_disconnect = true;
                            next_retry = None;
                            receiver = None;
                            stop_client(&mut client);
                            connected = false;
                            let _ = event_tx.send(MqttEvent::Disconnected {
                                reason: "disconnected by user".to_string(),
                            });
                        }
                        Ok(MqttCommand::Reconnect) => {
                            manual_disconnect = false;
                            receiver = None;
                            stop_client(&mut client);
                            connected = false;
                            attempt = 0;
                            backoff = Duration::from_secs(1);
                            next_retry = Some(Instant::now());
                        }
                        Ok(MqttCommand::Publish {
                            topic,
                            payload,
                            retain,
                        }) => {
                            if connected {
                                if let Some(client) = &client {
                                    match publish(client, &topic, &payload, retain) {
                                        Ok(()) => {
                                            let _ = event_tx.send(MqttEvent::Published {
                                                topic,
                                                payload,
                                                retain,
                                            });
                                        }
                                        Err(err) => {
                                            let _ = event_tx.send(MqttEvent::PublishFailed {
                                                topic,
                                                payload,
                                                reason: err.to_string(),
                                            });
                                        }
                                    }
                                } else {
                                    let _ = event_tx.send(MqttEvent::PublishRejected {
                                        topic,
                                        payload,
                                        reason: "client is not available".to_string(),
                                    });
                                }
                            } else {
                                let _ = event_tx.send(MqttEvent::PublishRejected {
                                    topic,
                                    payload,
                                    reason: "not connected".to_string(),
                                });
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            stop = true;
                            break;
                        }
                    }
                }

                if stop {
                    stop_client(&mut client);
                    break;
                }

                if connected {
                    let alive = client
                        .as_ref()
                        .map(|client| client.is_connected())
                        .unwrap_or(false);
                    if !alive {
                        connected = false;
                        let _ = event_tx.send(MqttEvent::ConnectionLost {
                            generation,
                            reason: "connection lost".to_string(),
                        });
                        receiver = None;
                        stop_client(&mut client);
                        if !manual_disconnect {
                            let delay = backoff;
                            next_retry = Some(Instant::now() + delay);
                            let _ = event_tx.send(MqttEvent::Reconnecting { attempt, delay });
                            backoff = backoff.saturating_mul(2).min(Duration::from_secs(30));
                        }
                    }
                }

                let retry_now = !connected
                    && !manual_disconnect
                    && next_retry
                        .map(|deadline| Instant::now() >= deadline)
                        .unwrap_or(false);

                if retry_now {
                    attempt = attempt.saturating_add(1);
                    let _ = event_tx.send(MqttEvent::Connecting { attempt });
                    match options.connect() {
                        Ok(new_client) => {
                            generation = generation.wrapping_add(1);
                            for topic in STATUS_TOPICS {
                                let _ = new_client.subscribe(topic, 0);
                            }
                            // Announce the new generation before the consumer
                            // thread can forward the first retained message.
                            let _ = event_tx.send(MqttEvent::Connected { generation });
                            let new_receiver = new_client.start_consuming();
                            client = Some(new_client);
                            receiver = Some(new_receiver);
                            connected = true;
                            attempt = 0;
                            backoff = Duration::from_secs(1);
                            next_retry = None;
                        }
                        Err(err) => {
                            let reason = err.to_string();
                            let _ = event_tx.send(MqttEvent::ConnectFailed { attempt, reason });
                            let delay = backoff;
                            next_retry = Some(Instant::now() + delay);
                            let _ = event_tx.send(MqttEvent::Reconnecting { attempt, delay });
                            backoff = backoff.saturating_mul(2).min(Duration::from_secs(30));
                        }
                    }
                }

                // Drain after the connection block so the receiver type is
                // inferred from the `start_consuming` assignment above before
                // this method call is resolved.
                if let Some(receiver) = &receiver {
                    while let Ok(Some(message)) = receiver.try_recv() {
                        let topic = message.topic().to_string();
                        let payload = String::from_utf8_lossy(message.payload()).into_owned();
                        let _ = event_tx.send(MqttEvent::Message {
                            generation,
                            topic,
                            payload,
                        });
                    }
                }

                std::thread::sleep(Duration::from_millis(25));
            }
        })
        .expect("failed to spawn MQTT supervisor thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_client_ids_are_prefixed_and_unique_enough() {
        let first = generate_client_id();
        let second = generate_client_id();
        assert!(first.starts_with("subraumctl-"));
        assert!(second.starts_with("subraumctl-"));
        assert_ne!(first, second);
    }
}
