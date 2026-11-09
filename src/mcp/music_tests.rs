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
async fn music_tools_validate_stations_and_permissions() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for permission in ["r", "w", "x"] {
            let config = toml::from_str(&format!("[tools]\nmusic = '{permission}'")).unwrap();
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
                .filter(|name| name.starts_with("music_"))
                .collect();
            assert_eq!(
                names.len(),
                match permission {
                    "w" => 7,
                    "r" => 2,
                    _ => 0,
                }
            );

            for (name, args, read) in [
                ("music_status", json!({}), true),
                ("music_stations", json!({}), true),
                ("music_play", json!({}), false),
                ("music_pause", json!({}), false),
                ("music_next", json!({}), false),
                ("music_previous", json!({}), false),
                ("music_station", json!({"station": "groovesalad"}), false),
            ] {
                let response = call(&mut client, name, args).await;
                if permission == "w" || (permission == "r" && read) {
                    assert_eq!(response["result"]["isError"], false, "{response}");
                } else {
                    assert_eq!(response["error"]["code"], -32601, "{response}");
                }
            }

            if permission != "x" {
                let stations = call(&mut client, "music_stations", json!({})).await;
                assert_eq!(
                    stations["result"]["structuredContent"]["stations"]
                        .as_array()
                        .unwrap()
                        .len(),
                    crate::music::STATIONS.len()
                );
                for (name, args) in [
                    ("music_status", json!({"topic": "#"})),
                    ("music_stations", json!({"watch": true})),
                    ("music_station", json!({"station": "unknown"})),
                ] {
                    if name == "music_station" && permission != "w" {
                        continue;
                    }
                    let response = call(&mut client, name, args).await;
                    assert_eq!(response["result"]["isError"], true, "{response}");
                }
            }
            drop(client);
            task.await.unwrap();
        }
    })
    .await
    .unwrap();
}
