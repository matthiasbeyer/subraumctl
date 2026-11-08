use super::{tests::start, *};
use clap::Parser;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn history_tools_validate_windows_schemas_permissions_and_dry_runs() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let global = crate::cli::Cli::try_parse_from([
            "subraumctl",
            "--influx-url",
            "invalid://PRIVATE",
            "--dry-run",
            "mcp",
        ])
        .unwrap()
        .global;
        for permission in ["r", "w", "x"] {
            let mut server = McpServer::new(
                toml::from_str(&format!(
                    "[tools]\nclimate = '{permission}'\npower = '{permission}'"
                ))
                .unwrap(),
            );
            tools::register(
                &mut server,
                Arc::new(backend::DeviceBackend::new(&global).unwrap()),
            )
            .unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            let listed = client.request("tools/list", json!({})).await;
            for (name, domain, database) in [
                ("climate_history", "climate", "climate"),
                ("power_history", "power", "subraum-stromzähler"),
            ] {
                let tool = listed["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name);
                assert_eq!(tool.is_some(), permission != "x");
                if let Some(tool) = tool {
                    assert_eq!(tool["annotations"]["readOnlyHint"], true);
                    let schema = &tool["inputSchema"];
                    assert_eq!(schema["additionalProperties"], false);
                    assert_eq!(schema["properties"]["minutes"]["minimum"], 1);
                    assert_eq!(schema["properties"]["minutes"]["maximum"], 43200);
                    assert_eq!(schema["properties"]["minutes"]["default"], 60);
                }
                for (args, minutes) in [
                    (json!({}), 60),
                    (json!({"minutes": 1}), 1),
                    (json!({"minutes": 90}), 90),
                    (json!({"minutes": 43200}), 43200),
                ] {
                    let result = client
                        .request("tools/call", json!({"name": name, "arguments": args}))
                        .await;
                    if permission == "x" {
                        assert_eq!(result["error"]["code"], -32601);
                    } else {
                        let content = &result["result"]["structuredContent"];
                        assert_eq!(content["dry_run"], true);
                        assert_eq!(content["domain"], domain);
                        assert_eq!(content["database"], database);
                        assert_eq!(content["window_minutes"], minutes);
                        assert!(
                            content["query"]
                                .as_str()
                                .unwrap()
                                .contains("GROUP BY time(")
                        );
                        assert!(!result.to_string().contains("PRIVATE"));
                    }
                }
                if permission != "x" {
                    for args in [
                        json!({"minutes": 0}),
                        json!({"minutes": 43201}),
                        json!({"minutes": u64::MAX}),
                        json!({"minutes": -1}),
                        json!({"minutes": 1.5}),
                        json!({"minutes": "60"}),
                        json!({"minutes": null}),
                        json!({"query": "arbitrary"}),
                        json!({"url": "http://elsewhere"}),
                        json!({"watch": true}),
                        json!({"mode": "latest"}),
                    ] {
                        let result = client
                            .request("tools/call", json!({"name": name, "arguments": args}))
                            .await;
                        assert_eq!(result["result"]["isError"], true, "{result}");
                    }
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
async fn live_history_errors_hide_endpoint_details() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let global = crate::cli::Cli::try_parse_from([
            "subraumctl",
            "--influx-url",
            "invalid://PRIVATE",
            "mcp",
        ])
        .unwrap()
        .global;
        let mut server = McpServer::new(Config::default());
        tools::register(
            &mut server,
            Arc::new(backend::DeviceBackend::new(&global).unwrap()),
        )
        .unwrap();
        let (mut client, task) = start(server);
        client.initialize().await;
        for name in ["climate_history", "power_history"] {
            let result = client.request("tools/call", json!({"name": name})).await;
            assert_eq!(result["result"]["isError"], true);
            assert!(!result.to_string().contains("PRIVATE"));
        }
        drop(client);
        task.await.unwrap();
    })
    .await
    .unwrap();
}
