use super::tests::{Client, RecordingBackend, start};
use super::*;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

async fn call(client: &mut Client, name: &str, arguments: Value) -> Value {
    client
        .request("tools/call", json!({"name": name, "arguments": arguments}))
        .await
}

#[tokio::test]
async fn pizzamat_tools_validate_identity_arguments_and_permissions() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for permission in ["r", "w", "x"] {
            let config = toml::from_str(&format!("[tools]\npizzamat = '{permission}'")).unwrap();
            let mut server = McpServer::new(config);
            tools::register(&mut server, Arc::new(RecordingBackend::default())).unwrap();
            let (mut client, task) = start(server);
            client.initialize().await;
            let listed = client.request("tools/list", json!({})).await;
            let names: Vec<_> = listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .filter(|name| name.starts_with("pizzamat_"))
                .collect();
            assert_eq!(
                names.len(),
                match permission {
                    "w" => 14,
                    "r" => 2,
                    _ => 0,
                }
            );
            for (name, args, command, expected_args, read) in [
                ("pizzamat_list", json!({}), "pizza-list", "", true),
                ("pizzamat_services", json!({}), "pizza-services", "", true),
                (
                    "pizzamat_start",
                    json!({"service":"pizzeria"}),
                    "pizza-init",
                    "pizzeria",
                    false,
                ),
                (
                    "pizzamat_add",
                    json!({"food":"Quattro formaggi"}),
                    "pizza-add",
                    "Quattro formaggi",
                    false,
                ),
                (
                    "pizzamat_remove",
                    json!({"food":"formaggi"}),
                    "pizza-rm",
                    "formaggi",
                    false,
                ),
                ("pizzamat_clear", json!({}), "pizza-clear", "", false),
                ("pizzamat_join", json!({}), "pizza-join", "", false),
                ("pizzamat_commit", json!({}), "pizza-commit", "", false),
                ("pizzamat_lock", json!({}), "pizza-lock", "", false),
                ("pizzamat_unlock", json!({}), "pizza-unlock", "", false),
                ("pizzamat_reset", json!({}), "pizza-reset", "", false),
                (
                    "pizzamat_set_default",
                    json!({"service":"pizzeria","food":"Quattro formaggi"}),
                    "pizza-set-default",
                    "pizzeria Quattro formaggi",
                    false,
                ),
                (
                    "pizzamat_service_remove",
                    json!({"service":"pizzeria"}),
                    "pizza-service-rm",
                    "pizzeria",
                    false,
                ),
                (
                    "pizzamat_service_add",
                    json!({"name":"pizzeria","realname":"Pizza Place"}),
                    "pizza-service-add",
                    "\"pizzeria\" \"Pizza Place\" \"-\" \"-\" \"-\"",
                    false,
                ),
            ] {
                let response = call(&mut client, name, args).await;
                if permission == "w" || (permission == "r" && read) {
                    let result = &response["result"]["structuredContent"];
                    assert_eq!(result["command"], command, "{response}");
                    assert_eq!(result["args"], expected_args);
                    assert_eq!(result["user"], "test-user");
                } else {
                    assert_eq!(response["error"]["code"], -32601, "{response}");
                }
            }
            if permission == "w" {
                for (name, arguments) in [
                    ("pizzamat_add", json!({"food":" "})),
                    (
                        "pizzamat_add",
                        json!({"food":"pizza","user":"someone-else"}),
                    ),
                    ("pizzamat_remove", json!({})),
                    ("pizzamat_start", json!({"service":"\n"})),
                    (
                        "pizzamat_set_default",
                        json!({"service":"two words","food":"pizza"}),
                    ),
                    ("pizzamat_list", json!({"topic":"#"})),
                    ("pizzamat_clear", json!({"user":"someone-else"})),
                    ("pizzamat_service_add", json!({"name":""})),
                ] {
                    assert_eq!(
                        call(&mut client, name, arguments).await["result"]["isError"],
                        true
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
async fn pizzamat_dry_run_needs_no_broker_and_uses_configured_user() {
    use clap::Parser;
    let global = crate::cli::Cli::try_parse_from([
        "subraumctl",
        "--dry-run",
        "--pizzamat-user",
        "alice",
        "--pizzamat-debug",
        "--pizzamat-amqp-url",
        "invalid://PRIVATE_SECRET@invalid",
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
    let response = call(&mut client, "pizzamat_add", json!({"food":"pizza"})).await;
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["command"], "puzza-add");
    assert_eq!(result["user"], "alice");
    assert_eq!(result["submitted"], false);
    assert!(!response.to_string().contains("PRIVATE_SECRET"));
    drop(client);
    task.await.unwrap();
}
