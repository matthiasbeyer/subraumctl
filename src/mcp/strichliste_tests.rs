use super::tests::start;
use super::*;
use clap::Parser;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

use crate::strichliste::test_fixture as fixture;

#[tokio::test]
async fn only_own_account_tools_and_double_opt_in_writes_are_exposed() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for permission in [None, Some("r"), Some("w"), Some("x")] {
            for writes in [false, true] {
                let mut args = vec!["subraumctl", "--dry-run", "mcp"];
                if writes {
                    args.push("--strichliste-mcp-writes");
                }
                let global = crate::cli::Cli::try_parse_from(args).unwrap().global;
                let config = permission
                    .map(|p| toml::from_str(&format!("[tools]\nstrichliste = '{p}'")).unwrap())
                    .unwrap_or_default();
                let mut server = McpServer::new(config);
                tools::register(
                    &mut server,
                    Arc::new(backend::DeviceBackend::new(&global).unwrap()),
                )
                .unwrap();
                let (mut client, task) = start(server);
                client.initialize().await;
                let list = client.request("tools/list", json!({})).await;
                let tools = list["result"]["tools"].as_array().unwrap();
                for (name, write) in [
                    ("strichliste_balance", false),
                    ("strichliste_rank", false),
                    ("strichliste_pay", true),
                    ("strichliste_add", true),
                ] {
                    let enabled = if write {
                        permission == Some("w") && writes
                    } else {
                        permission != Some("x")
                    };
                    assert_eq!(tools.iter().any(|tool| tool["name"] == name), enabled);
                    let arguments = if write {
                        json!({"amount": 0.01})
                    } else {
                        json!({})
                    };
                    let response = client
                        .request("tools/call", json!({"name": name, "arguments": arguments}))
                        .await;
                    if enabled {
                        assert_eq!(response["result"]["isError"], false, "{response}");
                        if write {
                            assert_eq!(
                                response["result"]["structuredContent"]["comment"],
                                "triggered by mcp"
                            );
                        }
                        let bad_args: Vec<Value> = if write {
                            vec![
                                json!({"amount": -1}),
                                json!({"amount": 0}),
                                json!({"amount": 1, "toUserId": 30}),
                                json!({"amount": 1, "comment": "overwrite"}),
                                json!({"amount": 1, "user_id": 30}),
                            ]
                        } else {
                            vec![
                                json!({"user_id": 30}),
                                json!({"user": "Other"}),
                                json!({"url": "http://other"}),
                            ]
                        };
                        for arguments in bad_args {
                            let response = client
                                .request(
                                    "tools/call",
                                    json!({"name": name, "arguments": arguments}),
                                )
                                .await;
                            assert_eq!(response["result"]["isError"], true, "{response}");
                        }
                    } else {
                        assert_eq!(response["error"]["code"], -32601);
                    }
                }
                for name in [
                    "strichliste_send",
                    "strichliste_users",
                    "strichliste_search",
                    "strichliste_transactions",
                ] {
                    assert!(!tools.iter().any(|tool| tool["name"] == name));
                    let response = client.request("tools/call", json!({"name": name})).await;
                    assert_eq!(response["error"]["code"], -32601);
                }
                drop(client);
                task.await.unwrap();
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn real_backend_returns_only_own_balance_rank_and_fixed_comment_writes() {
    let (url, http) = fixture::server(vec![
        (200, fixture::user()),
        (200, fixture::users()),
        (200, fixture::user()),
        (201, fixture::users()),
    ]);
    let global = crate::cli::Cli::try_parse_from([
        "subraumctl",
        "--strichliste-url",
        &url,
        "--strichliste-user-id",
        "27",
        "--strichliste-mcp-writes",
        "mcp",
    ])
    .unwrap()
    .global;
    let mut server = McpServer::new(toml::from_str("[tools]\nstrichliste = 'w'").unwrap());
    tools::register(
        &mut server,
        Arc::new(backend::DeviceBackend::new(&global).unwrap()),
    )
    .unwrap();
    let (mut client, task) = start(server);
    tokio::time::timeout(Duration::from_secs(5), async {
        client.initialize().await;
        for (name, arguments, expected) in [
            (
                "strichliste_balance",
                json!({}),
                json!({"balance_eur": 12.345}),
            ),
            (
                "strichliste_rank",
                json!({}),
                json!({"rank": 2, "weighted_count_of_purchases": 10.5}),
            ),
            (
                "strichliste_pay",
                json!({"amount": 0.01}),
                json!({"submitted": true, "value_eur": -0.01}),
            ),
        ] {
            let response = client
                .request("tools/call", json!({"name": name, "arguments": arguments}))
                .await;
            assert_eq!(response["result"]["structuredContent"], expected);
            assert!(!response.to_string().contains("Other"));
            assert!(!response.to_string().contains("DO NOT EXPOSE"));
        }
    })
    .await
    .unwrap();
    drop(client);
    task.await.unwrap();
    let requests = http.join().unwrap();
    assert!(requests[3].starts_with("POST /strich-api/user/27/transaction "));
    let body: Value = serde_json::from_str(requests[3].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body, json!({"value": -0.01, "comment": "triggered by mcp"}));
}
