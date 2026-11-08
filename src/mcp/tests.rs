use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

use super::*;

#[derive(Default)]
pub(super) struct RecordingBackend(
    std::sync::Mutex<Vec<crate::action::ControlCommand>>,
    std::sync::Mutex<Vec<crate::heizung::Request>>,
);

impl backend::Backend for RecordingBackend {
    async fn history(
        &self,
        domain: crate::history::Domain,
        minutes: u64,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        crate::history::preview(domain, minutes, 4_000_000_000)
    }
    async fn power(
        &self,
        mode: crate::influx::Mode,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        Ok(json!({"mode": mode.name(), "total_watts": null}))
    }
    async fn climate(
        &self,
        mode: crate::influx::Mode,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        Ok(json!({"mode": mode.name(), "rooms": []}))
    }
    async fn heizung(
        &self,
        request: crate::heizung::Request,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        self.1.lock().unwrap().push(request);
        Ok(json!({"submitted": true}))
    }
    fn pizzamat_user(&self) -> &str {
        "test-user"
    }

    async fn pizzamat(
        &self,
        request: crate::pizzamat::Request,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        Ok(
            json!({"command": request.operation.command(), "user": request.user, "args": request.args,
            "message": "Pizzamat reply"}),
        )
    }

    async fn snapshot(
        &self,
        snapshot: snapshot::Snapshot,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        Ok(snapshot.decode(&std::collections::HashMap::new()))
    }
    async fn send(
        &self,
        command: crate::action::ControlCommand,
        _: RequestContext<RoleServer>,
    ) -> eyre::Result<Value> {
        self.0.lock().unwrap().push(command);
        Ok(json!({"submitted": true}))
    }
}

pub(super) struct Client {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl Client {
    async fn send(&mut self, message: Value) {
        self.writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .await;
        let mut line = String::new();
        assert_ne!(self.reader.read_line(&mut line).await.unwrap(), 0);
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 1);
        response
    }

    pub(super) async fn initialize(&mut self) {
        let response = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"}
                }),
            )
            .await;
        assert_eq!(response["result"]["serverInfo"]["name"], "subraumctl");
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"]["resources"].is_null());
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
    }
}

pub(super) fn start(server: McpServer) -> (Client, tokio::task::JoinHandle<()>) {
    let (client, transport) = tokio::io::duplex(8192);
    let task = tokio::spawn(async move {
        server
            .serve(transport)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    let (reader, writer) = tokio::io::split(client);
    (
        Client {
            reader: BufReader::new(reader),
            writer,
        },
        task,
    )
}

#[tokio::test]
async fn catalogue_tools_return_structured_data_and_reject_extra_arguments() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut server = McpServer::new(Config::default());
        tools::register(&mut server, Arc::new(RecordingBackend::default())).unwrap();
        let (mut client, task) = start(server);
        client.initialize().await;
        let cells = client
            .request("tools/call", json!({"name": "white_light_cells"}))
            .await;
        assert_eq!(
            cells["result"]["structuredContent"]["cells"]
                .as_array()
                .unwrap()
                .len(),
            15
        );
        let knobs = client
            .request("tools/call", json!({"name": "knob_list"}))
            .await;
        assert_eq!(
            knobs["result"]["structuredContent"]["knobs"]
                .as_array()
                .unwrap()
                .len(),
            29
        );
        let invalid = client
            .request(
                "tools/call",
                json!({"name": "knob_list", "arguments": {"topic": "#"}}),
            )
            .await;
        assert_eq!(invalid["result"]["isError"], true);
        drop(client);
        task.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn heating_validates_before_io_and_enforces_permissions() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for permission in ["r", "w", "x"] {
            let backend = Arc::new(RecordingBackend::default());
            let mut server = McpServer::new(
                toml::from_str(&format!("[tools]\nheizung = '{permission}'")).unwrap(),
            );
            tools::register(&mut server, backend.clone()).unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            let listed = client.request("tools/list", json!({})).await;
            let tools = listed["result"]["tools"].as_array().unwrap();
            for name in [
                "heizung_get",
                "heizung_status",
                "heizung_rooms",
                "heizung_set",
            ] {
                let enabled = if name == "heizung_set" {
                    permission == "w"
                } else {
                    permission != "x"
                };
                let tool = tools.iter().find(|tool| tool["name"] == name);
                assert_eq!(tool.is_some(), enabled);
                if let Some(tool) = tool {
                    assert_eq!(tool["annotations"]["readOnlyHint"], name != "heizung_set");
                } else {
                    let result = client.request("tools/call", json!({"name": name})).await;
                    assert_eq!(result["error"]["code"], -32601);
                }
            }
            if permission != "x" {
                for name in ["heizung_get", "heizung_status"] {
                    for args in [
                        json!({"rooms": ["unknown"]}),
                        json!({"rooms": ["hack", "hack"]}),
                        json!({"watch": true}),
                        json!({"amqp_url": "other"}),
                    ] {
                        let result = client
                            .request("tools/call", json!({"name": name, "arguments": args}))
                            .await;
                        assert_eq!(result["result"]["isError"], true);
                    }
                }
            }
            if permission == "w" {
                for args in [
                    json!({"temperature": 20}),
                    json!({"rooms": ["hack"], "temperature": 26}),
                    json!({"rooms": ["hack"], "temperature": 21.55}),
                    json!({"rooms": ["hack"], "temperature": 21, "duration_minutes": 35792}),
                    json!({"rooms": ["hack"], "all": true, "temperature": 21}),
                    json!({"rooms": ["hack"], "temperature": 21, "src": "other"}),
                ] {
                    let result = client
                        .request(
                            "tools/call",
                            json!({"name": "heizung_set", "arguments": args}),
                        )
                        .await;
                    assert_eq!(result["result"]["isError"], true, "{result}");
                }
            }
            assert!(backend.1.lock().unwrap().is_empty());
            if permission != "x" {
                for (name, args) in [
                    ("heizung_get", json!({"rooms": ["hack", "make"]})),
                    ("heizung_status", json!({})),
                ] {
                    let result = client
                        .request("tools/call", json!({"name": name, "arguments": args}))
                        .await;
                    assert_eq!(result["result"]["structuredContent"]["submitted"], true);
                }
                let requests = backend.1.lock().unwrap().clone();
                assert_eq!(requests[0].args(), "get hack make");
                assert_eq!(requests[1].args(), "get make hack cook elab shit storage");
            }
            if permission == "w" {
                for args in [
                    json!({"rooms": ["hack", "make"], "temperature": 21.5}),
                    json!({"all": true, "temperature": 20, "duration_minutes": 0}),
                ] {
                    let result = client
                        .request(
                            "tools/call",
                            json!({"name": "heizung_set", "arguments": args}),
                        )
                        .await;
                    assert_eq!(result["result"]["structuredContent"]["submitted"], true);
                }
                let requests = backend.1.lock().unwrap().clone();
                assert_eq!(requests[2].args(), "set hack make 21.5 -d 180");
                assert_eq!(
                    requests[3].args(),
                    "set make hack cook elab shit storage 20.0 -d 0"
                );
            }
            drop(client);
            task.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn climate_modes_permissions_and_dry_runs() {
    use clap::Parser;
    tokio::time::timeout(Duration::from_secs(5), async {
        for permission in ["r", "w", "x"] {
            let mut global = crate::cli::Cli::try_parse_from([
                "subraumctl",
                "--influx-url",
                "invalid://PRIVATE",
                "--dry-run",
                "climate",
                "status",
            ])
            .unwrap()
            .global;
            let mut server = McpServer::new(
                toml::from_str(&format!("[tools]\nclimate = '{permission}'")).unwrap(),
            );
            tools::register(
                &mut server,
                Arc::new(backend::DeviceBackend::new(&global).unwrap()),
            )
            .unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            let listed = client.request("tools/list", json!({})).await;
            let tool = listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == "climate_status");
            assert_eq!(tool.is_some(), permission != "x");
            if let Some(tool) = tool {
                assert_eq!(tool["annotations"]["readOnlyHint"], true);
            }
            for (args, mode) in [
                (json!({}), "average"),
                (json!({"mode": "average"}), "average"),
                (json!({"mode": "latest"}), "latest"),
            ] {
                let result = client
                    .request(
                        "tools/call",
                        json!({"name": "climate_status", "arguments": args}),
                    )
                    .await;
                if permission == "x" {
                    assert_eq!(result["error"]["code"], -32601);
                } else {
                    assert_eq!(result["result"]["structuredContent"]["mode"], mode);
                    assert_eq!(result["result"]["structuredContent"]["dry_run"], true);
                }
            }
            if permission != "x" {
                for args in [
                    json!({"mode": "invalid"}),
                    json!({"query": "DROP DATABASE climate"}),
                    json!({"url": "http://elsewhere"}),
                    json!({"watch": true}),
                ] {
                    let result = client
                        .request(
                            "tools/call",
                            json!({"name": "climate_status", "arguments": args}),
                        )
                        .await;
                    assert_eq!(result["result"]["isError"], true);
                }
            }
            drop(client);
            task.await.unwrap();
            if permission == "r" {
                global.dry_run = false;
                let mut server = McpServer::new(Config::default());
                tools::register(
                    &mut server,
                    Arc::new(backend::DeviceBackend::new(&global).unwrap()),
                )
                .unwrap();
                let (mut client, task) = start(server);
                client.initialize().await;
                let result = client
                    .request("tools/call", json!({"name": "climate_status"}))
                    .await;
                assert_eq!(result["result"]["isError"], true);
                assert!(!result.to_string().contains("PRIVATE"));
                drop(client);
                task.await.unwrap();
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn power_modes_permissions_and_dry_runs() {
    use clap::Parser;
    tokio::time::timeout(Duration::from_secs(5), async {
        for permission in ["r", "w", "x"] {
            let mut global = crate::cli::Cli::try_parse_from([
                "subraumctl",
                "--influx-url",
                "invalid://PRIVATE",
                "--dry-run",
                "power",
                "status",
            ])
            .unwrap()
            .global;
            let mut server = McpServer::new(
                toml::from_str(&format!("[tools]\npower = '{permission}'")).unwrap(),
            );
            tools::register(
                &mut server,
                Arc::new(backend::DeviceBackend::new(&global).unwrap()),
            )
            .unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            let listed = client.request("tools/list", json!({})).await;
            let tool = listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == "power_status");
            assert_eq!(tool.is_some(), permission != "x");
            if let Some(tool) = tool {
                assert_eq!(tool["annotations"]["readOnlyHint"], true);
            }
            for (args, mode) in [
                (json!({}), "average"),
                (json!({"mode": "average"}), "average"),
                (json!({"mode": "latest"}), "latest"),
            ] {
                let result = client
                    .request(
                        "tools/call",
                        json!({"name": "power_status", "arguments": args}),
                    )
                    .await;
                if permission == "x" {
                    assert_eq!(result["error"]["code"], -32601);
                } else {
                    let content = &result["result"]["structuredContent"];
                    assert_eq!(content["mode"], mode);
                    assert_eq!(content["dry_run"], true);
                    assert_eq!(content["database"], "subraum-stromzähler");
                }
            }
            if permission != "x" {
                for args in [
                    json!({"mode": "invalid"}),
                    json!({"query": "arbitrary query"}),
                    json!({"url": "http://elsewhere"}),
                    json!({"watch": true}),
                ] {
                    let result = client
                        .request(
                            "tools/call",
                            json!({"name": "power_status", "arguments": args}),
                        )
                        .await;
                    assert_eq!(result["result"]["isError"], true);
                }
            }
            drop(client);
            task.await.unwrap();
            if permission == "r" {
                global.dry_run = false;
                let mut server = McpServer::new(Config::default());
                tools::register(
                    &mut server,
                    Arc::new(backend::DeviceBackend::new(&global).unwrap()),
                )
                .unwrap();
                let (mut client, task) = start(server);
                client.initialize().await;
                let result = client
                    .request("tools/call", json!({"name": "power_status"}))
                    .await;
                assert_eq!(result["result"]["isError"], true);
                assert!(!result.to_string().contains("PRIVATE"));
                drop(client);
                task.await.unwrap();
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn heating_dry_run_and_connection_errors_hide_endpoint_credentials() {
    use clap::Parser;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut global = crate::cli::Cli::try_parse_from([
            "subraumctl",
            "--heizung-amqp-url",
            "invalid://PRIVATE_CREDENTIAL@invalid",
            "heizung",
            "status",
        ])
        .unwrap()
        .global;
        for dry_run in [true, false] {
            global.dry_run = dry_run;
            let mut server = McpServer::new(Config::default());
            tools::register(
                &mut server,
                Arc::new(backend::DeviceBackend::new(&global).unwrap()),
            )
            .unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            for (name, args) in [
                ("heizung_status", json!({})),
                ("heizung_set", json!({"rooms": ["hack"], "temperature": 21})),
            ] {
                let result = client
                    .request("tools/call", json!({"name": name, "arguments": args}))
                    .await;
                assert!(!result.to_string().contains("PRIVATE_CREDENTIAL"));
                if dry_run {
                    assert_eq!(result["result"]["structuredContent"]["submitted"], false);
                    assert_eq!(result["result"]["structuredContent"]["dry_run"], true);
                } else {
                    assert_eq!(result["result"]["isError"], true);
                    assert_eq!(
                        result["result"]["content"][0]["text"],
                        "heizung AMQP connection failed"
                    );
                }
            }
            drop(client);
            task.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lighting_validates_before_device_io() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let backend = Arc::new(RecordingBackend::default());
        let mut server = McpServer::new(Config::default());
        tools::register(&mut server, backend.clone()).unwrap();
        let (mut client, task) = start(server);
        client.initialize().await;
        for (name, arguments) in [
            ("dmx", json!({"group": "#", "animation": "rainbow"})),
            (
                "dmx",
                json!({"group": "subraum.all", "animation": "rainbow", "topic": "#"}),
            ),
            (
                "white_light_apply",
                json!({"warm": 256, "cold": 0, "all": true}),
            ),
            ("white_light_apply", json!({"warm": 1, "cold": 0})),
            (
                "white_light_apply",
                json!({"warm": 1, "cold": 0, "cells": ["#"]}),
            ),
            (
                "white_light_apply",
                json!({"warm": 1, "cold": 0, "all": true, "cells": ["LM3_A020A63498E7"]}),
            ),
        ] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": arguments}))
                .await;
            assert_eq!(response["result"]["isError"], true, "{response}");
        }
        assert!(backend.0.lock().unwrap().is_empty());
        for (name, arguments) in [
            (
                "dmx",
                json!({"group": "subraum.all", "animation": "rainbow"}),
            ),
            (
                "white_light_apply",
                json!({"warm": 128, "cold": 200, "all": true}),
            ),
        ] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": arguments}))
                .await;
            assert_eq!(response["result"]["structuredContent"]["submitted"], true);
        }
        let commands = backend.0.lock().unwrap().clone();
        assert_eq!(
            commands[0].expand("").unwrap()[0].payload(),
            r#"{"args":"subraum.all rainbow"}"#
        );
        assert_eq!(commands[1].expand("").unwrap().len(), 15);
        drop(client);
        task.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn control_writes_use_domain_commands_and_respect_read_only_groups() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let backend = Arc::new(RecordingBackend::default());
        let config = toml::from_str("[tools]\nknob = 'r'\nsystem = 'x'").unwrap();
        let mut server = McpServer::new(config);
        tools::register(&mut server, backend.clone()).unwrap();
        let (mut client, task) = start(server);
        client.initialize().await;
        for (name, arguments) in [
            ("knob_set", json!({"knob": "cx", "value": 42})),
            ("system_telinit", json!({"state": "off"})),
        ] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": arguments}))
                .await;
            assert_eq!(response["error"]["code"], -32601);
        }
        for (name, arguments) in [
            ("alert_sob", json!({"sound": "noot-noot"})),
            ("alert_radiation", json!({"target": "make"})),
        ] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": arguments}))
                .await;
            assert_eq!(response["result"]["structuredContent"]["submitted"], true);
        }
        let commands = backend.0.lock().unwrap().clone();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0].expand("").unwrap()[0].target(),
            crate::action::TOPIC_SOB
        );
        assert_eq!(
            commands[1].expand("").unwrap()[0].target(),
            crate::action::TOPIC_RADIATION_MAKE
        );
        drop(client);
        task.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn opt_ins_are_disabled_by_default_and_publish_is_unfiltered_when_enabled() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for enabled in [false, true] {
            let config = if enabled {
                toml::from_str("[tools]\npublish = 'w'\nsmoothiesaur = 'w'").unwrap()
            } else {
                Config::default()
            };
            let backend = Arc::new(RecordingBackend::default());
            let mut server = McpServer::new(config);
            tools::register(&mut server, backend.clone()).unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            let listed = client.request("tools/list", json!({})).await;
            let names: Vec<_> = listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect();
            assert!(!names.contains(&"subscribe"));
            assert!(!names.contains(&"outlet"));
            assert_eq!(names.contains(&"publish"), enabled);
            for (name, arguments) in [
                (
                    "publish",
                    json!({"topic": "edi/cmd/act", "payload": "unfiltered", "retain": true}),
                ),
                ("smoothiesaur_laser", json!({"power": 128})),
                (
                    "smoothiesaur_relay",
                    json!({"relay": "pump-primary", "state": "on"}),
                ),
                ("smoothiesaur_zero", json!({})),
            ] {
                let response = client
                    .request("tools/call", json!({"name": name, "arguments": arguments}))
                    .await;
                if enabled {
                    assert_eq!(response["result"]["structuredContent"]["submitted"], true);
                } else {
                    assert_eq!(response["error"]["code"], -32601);
                }
            }
            if enabled {
                for (name, arguments) in [
                    (
                        "publish",
                        json!({"topic": "any", "payload_file": "/etc/passwd"}),
                    ),
                    ("smoothiesaur_laser", json!({"power": 256})),
                    (
                        "smoothiesaur_laser",
                        json!({"power": 1, "url": "http://elsewhere"}),
                    ),
                ] {
                    let response = client
                        .request("tools/call", json!({"name": name, "arguments": arguments}))
                        .await;
                    assert_eq!(response["result"]["isError"], true);
                }
                let commands = backend.0.lock().unwrap().clone();
                assert_eq!(commands.len(), 4);
                let publish = &commands[0].expand("").unwrap()[0];
                assert_eq!(publish.target(), "edi/cmd/act");
                assert_eq!(publish.payload(), "unfiltered");
                assert!(publish.retain());
            } else {
                assert!(backend.0.lock().unwrap().is_empty());
            }
            drop(client);
            task.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn read_only_groups_expose_bounded_reads_without_topic_or_watch_arguments() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let config = toml::from_str("[tools]\nalert = 'r'\nwhite_light = 'r'\nknob = 'r'\nsmoothiesaur = 'r'\ndmx = 'x'\nsystem = 'x'\nheizung = 'r'\npizzamat = 'x'").unwrap();
        let mut server = McpServer::new(config);
        tools::register(&mut server, Arc::new(RecordingBackend::default())).unwrap();
        let (mut client, task) = start(server);
        client.initialize().await;
        let listed = client.request("tools/list", json!({})).await;
        let names: Vec<_> = listed["result"]["tools"].as_array().unwrap().iter()
            .map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["alert_led", "climate_history", "climate_status", "heizung_get", "heizung_rooms", "heizung_status", "knob_get", "knob_list", "power_history", "power_status", "smoothiesaur_status", "status", "white_light_cells", "white_light_status"]);
        for name in names {
            let arguments = if name == "knob_get" { json!({"knob": "cx"}) } else { json!({}) };
            let result = client.request("tools/call", json!({"name": name, "arguments": arguments})).await;
            assert!(result["result"]["structuredContent"].is_object(), "{result}");
            let invalid = client.request("tools/call", json!({"name": name, "arguments": {"topic": "#", "watch": true}})).await;
            assert_eq!(invalid["result"]["isError"], true);
        }
        for name in ["subscribe", "monitor", "knob_watch", "smoothiesaur_laser", "white_light_apply", "alert_sob", "knob_set", "heizung_set"] {
            let result = client.request("tools/call", json!({"name": name})).await;
            assert_eq!(result["error"]["code"], -32601);
        }
        drop(client);
        task.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn dry_run_skips_io_and_backend_errors_do_not_expose_connection_details() {
    use clap::Parser;
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut global = crate::cli::Cli::try_parse_from([
            "subraumctl",
            "--broker",
            "invalid://PRIVATE_CREDENTIAL@invalid",
            "status",
        ])
        .unwrap()
        .global;
        global.no_clean_session = true;
        assert!(backend::DeviceBackend::new(&global).is_err());
        global.no_clean_session = false;
        for dry_run in [true, false] {
            global.dry_run = dry_run;
            let mut server = McpServer::new(Config::default());
            tools::register(
                &mut server,
                Arc::new(backend::DeviceBackend::new(&global).unwrap()),
            )
            .unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            for (name, arguments) in [
                (
                    "dmx",
                    json!({"group": "subraum.all", "animation": "rainbow"}),
                ),
                ("status", json!({})),
            ] {
                let response = client
                    .request("tools/call", json!({"name": name, "arguments": arguments}))
                    .await;
                assert!(!response.to_string().contains("PRIVATE_CREDENTIAL"));
                if dry_run {
                    assert_eq!(response["result"]["structuredContent"]["dry_run"], true);
                } else {
                    assert_eq!(response["result"]["isError"], true);
                    assert_eq!(
                        response["result"]["content"][0]["text"],
                        "MQTT connection failed"
                    );
                }
            }
            drop(client);
            task.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn foundation_negotiates_lists_no_tools_and_rejects_calls() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (mut client, server) = start(McpServer::new(Config::default()));
        client.initialize().await;
        assert_eq!(
            client.request("tools/list", json!({})).await["result"]["tools"],
            json!([])
        );
        for name in ["publish", "subscribe", "outlet", "smoothiesaur", "unknown"] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": {}}))
                .await;
            assert_eq!(response["error"]["code"], -32601);
        }
        drop(client);
        server.await.unwrap();
    })
    .await
    .expect("MCP handshake/EOF shutdown timed out");
}

#[tokio::test]
async fn listing_and_calls_enforce_the_same_permissions() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let config: Config = toml::from_str("[tools]\nknob = 'r'\ndmx = 'r'\nalert = 'x'").unwrap();
        let mut server = McpServer::new(config);
        let calls = Arc::new(AtomicUsize::new(0));
        for (name, group, access) in [
            ("knob_get", Group::Knob, Access::Read),
            ("knob_set", Group::Knob, Access::Write),
            ("white_light_status", Group::WhiteLight, Access::Read),
            ("white_light_apply", Group::WhiteLight, Access::Write),
            ("dmx", Group::Dmx, Access::Write),
            ("outlet", Group::Outlet, Access::Write),
            ("publish", Group::Publish, Access::Write),
            ("smoothiesaur_status", Group::Smoothiesaur, Access::Read),
            ("alert_led", Group::Alert, Access::Read),
        ] {
            let calls = calls.clone();
            let definition = Tool::new(
                name,
                "test fixture",
                json!({"type": "object"}).as_object().unwrap().clone(),
            );
            server
                .register(definition, group, access, move |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    async { Ok(CallToolResult::success(vec![]).into()) }
                })
                .unwrap();
        }
        let (mut client, task) = start(server);
        client.initialize().await;
        let listed = client.request("tools/list", json!({})).await;
        let names: Vec<_> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["knob_get", "white_light_apply", "white_light_status"]
        );
        for name in [
            "knob_set",
            "dmx",
            "outlet",
            "publish",
            "smoothiesaur_status",
            "alert_led",
        ] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": {}}))
                .await;
            assert_eq!(response["error"]["code"], -32601, "{name}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for name in names {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": {}}))
                .await;
            assert!(response.get("error").is_none(), "{response}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        drop(client);
        task.await.unwrap();
    })
    .await
    .expect("MCP permission test timed out");
}
