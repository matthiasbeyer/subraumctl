//! Exercise the real HTTP reader and CLI against a local InfluxDB fixture.
use serde_json::{Value, json};
use std::{process::Command, time::Duration};
use subraumctl::{climate, influx::Mode};

mod common;
use common::server;

fn fixture() -> Value {
    json!({"results": [
        {"statement_id": 1, "series": [
            {"name": "hum", "tags": {"location": "hack"}, "columns": ["time", "value"], "values": [[1000, 45.5]]},
            {"name": "hum", "tags": {"location": "make"}, "columns": ["time", "value"], "values": [[2000, null]]}
        ]},
        {"statement_id": 0, "series": [
            {"name": "temp", "tags": {"location": "hack"}, "columns": ["value", "time"], "values": [[21.25, 3000]]},
            {"name": "temp", "tags": {"location": "elab"}, "columns": ["time", "value"], "values": [[4000, 0.0]]}
        ]}
    ]})
}

#[test]
fn merges_locations_and_matches_columns_and_statements_by_name() {
    let snapshot = climate::decode(&fixture(), Mode::Latest, 10_000).unwrap();
    assert_eq!(
        snapshot
            .rooms
            .iter()
            .map(|r| r.location.as_str())
            .collect::<Vec<_>>(),
        ["elab", "hack", "make"]
    );
    assert_eq!(snapshot.rooms[0].temperature_celsius, Some(0.0));
    assert_eq!(snapshot.rooms[0].humidity_percent, None);
    assert_eq!(snapshot.rooms[1].temperature_celsius, Some(21.25));
    assert_eq!(snapshot.rooms[1].humidity_percent, Some(45.5));
    assert_eq!(snapshot.rooms[1].temperature_time_ms, Some(3000));
    assert_eq!(snapshot.rooms[1].humidity_time_ms, Some(1000));
    assert_eq!(snapshot.rooms[2].humidity_percent, None);
    let average = climate::decode(&fixture(), Mode::Average, 10_000)
        .unwrap()
        .json();
    assert_eq!(average["window_minutes"], 15);
    assert!(average["rooms"][1]["temperature_time_ms"].is_null());
    let empty = json!({"results": [{"statement_id": 0}, {"statement_id": 1}]});
    assert!(
        climate::decode(&empty, Mode::Average, 10_000)
            .unwrap()
            .rooms
            .is_empty()
    );
}

#[test]
fn malformed_or_partial_data_is_not_presented_as_missing_or_zero() {
    for value in [
        json!({"error": "PRIVATE"}),
        json!({"results": [{"statement_id": 0, "error": "PRIVATE"}, {"statement_id": 1}]}),
        json!({"results": [{"statement_id": 0}]}),
        json!({"results": [{"statement_id": 0}, {"statement_id": 0}]}),
    ] {
        let error = climate::decode(&value, Mode::Latest, 10_000)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE"));
    }
    for (field, value) in [
        ("values", json!([["not numeric", 3000]])),
        ("values", json!([[21, 30_000]])),
        ("values", json!([[21]])),
        ("values", json!([[21, 3000], [22, 4000]])),
        ("columns", json!(["value", "value"])),
        ("partial", json!(true)),
        ("tags", json!({})),
    ] {
        let mut value_with_error = fixture();
        value_with_error["results"][1]["series"][0][field] = value;
        assert!(
            climate::decode(&value_with_error, Mode::Latest, 10_000).is_err(),
            "{field}"
        );
    }
}

#[tokio::test]
async fn reader_sends_encoded_batched_queries_and_preserves_sample_times() {
    for mode in [Mode::Average, Mode::Latest] {
        let (options, server) = server(200, fixture().to_string(), Duration::ZERO);
        let snapshot = climate::read(&options, mode).await.unwrap();
        assert_eq!(snapshot.mode, mode);
        let request = server.join().unwrap();
        let target = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = reqwest::Url::parse(&format!("http://localhost{target}")).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["db"], "climate");
        assert_eq!(pairs["epoch"], "ms");
        assert_eq!(pairs["q"], climate::query(mode, snapshot.queried_at_ms));
        assert!(request.starts_with("GET /query?"));
        assert_eq!(pairs["q"].contains("time >="), mode == Mode::Average);
    }
}

#[tokio::test]
async fn http_json_query_errors_and_timeouts_are_bounded_and_redacted() {
    for (status, body) in [
        (500, "PRIVATE"),
        (200, "not json PRIVATE"),
        (200, "{\"results\":[{\"error\":\"PRIVATE\"}]}"),
    ] {
        let (options, server) = server(status, body.into(), Duration::ZERO);
        let error = climate::read(&options, Mode::Average)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE"));
        server.join().unwrap();
    }
    let (mut options, server) = server(200, fixture().to_string(), Duration::from_millis(150));
    options.timeout = Duration::from_millis(30);
    assert!(
        climate::read(&options, Mode::Average)
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    server.join().unwrap();
}

#[test]
fn cli_supports_live_json_modes_and_offline_dry_runs() {
    let (options, server) = server(200, fixture().to_string(), Duration::ZERO);
    let result = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args([
            "--influx-url",
            &options.url,
            "--json",
            "climate",
            "status",
            "--mode",
            "latest",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["mode"], "latest");
    assert_eq!(value["rooms"][1]["temperature_celsius"], 21.25);
    server.join().unwrap();
    for mode in ["average", "latest"] {
        let result = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args([
                "--influx-url",
                "invalid://unused",
                "--dry-run",
                "--json",
                "climate",
                "status",
                "--watch",
                "--mode",
                mode,
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(value["mode"], mode);
        assert_eq!(value["dry_run"], true);
    }
    for args in [vec!["--mode", "invalid"], vec!["--interval", "0"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args(["--dry-run", "climate", "status"])
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
}
