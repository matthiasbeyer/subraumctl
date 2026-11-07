//! Meter fixtures exercise scaling, Unicode InfluxDB parameters and the real CLI.
use serde_json::{Value, json};
use std::{process::Command, time::Duration};
use subraumctl::{influx::Mode, power};

mod common;
use common::server;

fn fixture() -> Value {
    json!({"results": [{"statement_id": 0, "series": [{
        "name": "modbus", "columns": ["total", "l3", "time", "l1", "l2"],
        "values": [[3501250, null, 5000, 1234567, -1000]]
    }]}]})
}

#[test]
fn decoder_scales_by_column_name_and_preserves_missing_and_negative_values() {
    let snapshot = power::decode(&fixture(), Mode::Latest, 10_000).unwrap();
    assert_eq!(snapshot.l1_watts, Some(1234.567));
    assert_eq!(snapshot.l2_watts, Some(-1.0));
    assert_eq!(snapshot.l3_watts, None);
    assert_eq!(snapshot.total_watts, Some(3501.25));
    assert_eq!(snapshot.sample_time_ms, Some(5000));
    assert!(snapshot.json()["window_minutes"].is_null());
    let average = power::decode(&fixture(), Mode::Average, 10_000)
        .unwrap()
        .json();
    assert_eq!(average["window_minutes"], 15);
    assert!(average["sample_time_ms"].is_null());
    let mut missing_total = fixture();
    missing_total["results"][0]["series"][0]["values"][0] = json!([null, 1000, 5000, 2000, 3000]);
    assert!(
        power::decode(&missing_total, Mode::Latest, 10_000)
            .unwrap()
            .total_watts
            .is_none()
    );
    for value in [
        json!({"results": [{"statement_id": 0}]}),
        json!({"results": [{"statement_id": 0, "series": []}]}),
    ] {
        let snapshot = power::decode(&value, Mode::Latest, 10_000).unwrap();
        assert!(snapshot.total_watts.is_none());
        assert!(snapshot.sample_time_ms.is_none());
    }
    let mut zero = fixture();
    zero["results"][0]["series"][0]["values"][0] = json!([0, 0, 5000, 0, 0]);
    assert_eq!(
        power::decode(&zero, Mode::Latest, 10_000)
            .unwrap()
            .total_watts,
        Some(0.0)
    );
}

#[test]
fn malformed_partial_and_unexpected_results_fail() {
    for value in [
        json!({"error": "PRIVATE"}),
        json!({"results": []}),
        json!({"results": [{"statement_id": 1}]}),
        json!({"results": [{"statement_id": 0}, {"statement_id": 1}]}),
        json!({"results": [{"statement_id": 0, "partial": true}]}),
        json!({"results": [{"statement_id": 0, "error": "PRIVATE"}]}),
    ] {
        let error = power::decode(&value, Mode::Latest, 10_000)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE"));
    }
    for (field, value) in [
        ("name", json!("other")),
        ("partial", json!(true)),
        ("columns", json!(["total", "l3", "time", "l1", "l1"])),
        ("values", json!([["NaN", 0, 5000, 1000, 2000]])),
        ("values", json!([[1000, 0, 20_000, 1000, 2000]])),
        ("values", json!([[1000, 0, null, 1000, 2000]])),
        ("values", json!([[1000]])),
        (
            "values",
            json!([[1000, 0, 5000, 1000, 2000], [1000, 0, 4000, 1000, 2000]]),
        ),
    ] {
        let mut response = fixture();
        response["results"][0]["series"][0][field] = value;
        assert!(
            power::decode(&response, Mode::Latest, 10_000).is_err(),
            "{field}"
        );
    }
    let mut duplicate = fixture();
    let series = duplicate["results"][0]["series"][0].clone();
    duplicate["results"][0]["series"]
        .as_array_mut()
        .unwrap()
        .push(series);
    assert!(power::decode(&duplicate, Mode::Latest, 10_000).is_err());
}

#[tokio::test]
async fn http_query_encodes_unicode_database_and_fields_for_both_modes() {
    for mode in [Mode::Average, Mode::Latest] {
        let (options, server) = server(200, fixture().to_string(), Duration::ZERO);
        let snapshot = power::read(&options, mode).await.unwrap();
        assert_eq!(snapshot.total_watts, Some(3501.25));
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /query?"));
        let target = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = reqwest::Url::parse(&format!("http://localhost{target}")).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["db"], "subraum-stromzähler");
        assert_eq!(pairs["epoch"], "ms");
        let query = &pairs["q"];
        assert_eq!(query, &power::query(mode, snapshot.queried_at_ms));
        assert!(query.contains("\"P∑\""));
        assert!(query.contains("(\"name\" = 'stromzähler')"));
        if mode == Mode::Average {
            assert!(query.contains("mean(\"P1\")"));
            assert!(query.contains(&format!("time >= {}ms", snapshot.queried_at_ms - 900_000)));
        } else {
            assert!(query.ends_with("ORDER BY time DESC LIMIT 1"));
            assert!(!query.contains("time >="));
        }
    }
}

#[tokio::test]
async fn reader_handles_http_json_query_errors_and_timeout() {
    for (status, body) in [
        (503, "PRIVATE"),
        (200, "PRIVATE not JSON"),
        (200, "{\"results\":[{\"error\":\"PRIVATE\"}]}"),
    ] {
        let (options, server) = server(status, body.into(), Duration::ZERO);
        let error = power::read(&options, Mode::Average)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("PRIVATE"));
        server.join().unwrap();
    }
    let (mut options, server) = server(200, fixture().to_string(), Duration::from_millis(150));
    options.timeout = Duration::from_millis(30);
    assert!(
        power::read(&options, Mode::Average)
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    server.join().unwrap();
}

#[test]
fn cli_live_reads_and_offline_previews_support_modes_and_quiet() {
    for json in [true, false] {
        let (options, server) = server(200, fixture().to_string(), Duration::ZERO);
        let mut command = Command::new(env!("CARGO_BIN_EXE_subraumctl"));
        command.args(["--influx-url", &options.url]);
        if json {
            command.arg("--json");
        }
        let result = command
            .args(["power", "get", "--mode", "latest"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if json {
            let value: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(value["mode"], "latest");
            assert_eq!(value["l1_watts"], 1234.567);
            assert_eq!(value["total_watts"], 3501.25);
            assert!(value["l3_watts"].is_null());
        } else {
            let output = String::from_utf8(result.stdout).unwrap();
            assert!(output.contains("L1: 1234.57 W"));
            assert!(output.contains("L3: ? W"));
            assert!(output.contains("Total: 3501.25 W"));
            assert!(output.contains("s old"));
        }
        server.join().unwrap();
    }
    for mode in ["average", "latest"] {
        let result = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args([
                "--influx-url",
                "invalid://unused",
                "--dry-run",
                "--json",
                "power",
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
        assert_eq!(value["database"], "subraum-stromzähler");
        assert_eq!(value["dry_run"], true);
    }
    let result = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args(["--quiet", "--dry-run", "power", "status"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    for args in [vec!["--mode", "invalid"], vec!["--interval", "0"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args(["--dry-run", "power", "status"])
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
}
