//! Shared history contract: timestamped, unit-labeled bucket means with null gaps.
use serde_json::json;
use std::{collections::HashMap, time::Duration};
use subraumctl::history::{self, Domain};

mod common;

#[test]
fn history_json_preserves_exact_timestamps_nulls_scaling_and_units() {
    let now = 1_000_051;
    let response = json!({"results": [{"statement_id": 0, "series": [{
        "name": "modbus", "columns": ["total", "l2", "time", "l3", "l1"],
        "values": [[1234567, -1000, 940051, null, 2000], [null, 0, 940151, null, null]]
    }]}]});
    let value = history::decode(&response, Domain::Power, 1, now)
        .unwrap()
        .json();
    assert_eq!(value["start_ms"], 940051);
    assert_eq!(value["end_ms"], now);
    assert_eq!(value["bucket_ms"], 100);
    assert_eq!(value["aggregation"], "mean");
    let series = value["series"].as_array().unwrap();
    let total = series.iter().find(|s| s["name"] == "Total").unwrap();
    assert_eq!(total["unit"], "W");
    assert_eq!(total["metric"], "power");
    assert_eq!(
        total["samples"],
        json!([
            {"time_ms": 940051, "value": 1234.567}, {"time_ms": 940151, "value": null}
        ])
    );
    let l2 = series.iter().find(|s| s["name"] == "L2").unwrap();
    assert_eq!(l2["samples"][0]["value"], -1.0);
    assert_eq!(l2["samples"][1]["value"], 0.0);
    let l3 = series.iter().find(|s| s["name"] == "L3").unwrap();
    assert!(l3["samples"][0]["value"].is_null());

    let response = json!({"results": [
        {"statement_id": 1, "series": [{"name": "hum", "tags": {"location": "hack"},
            "columns": ["value", "time"], "values": [[null, 940051], [45.5, 940151]]}]},
        {"statement_id": 0, "series": [{"name": "temp", "tags": {"location": "hack"},
            "columns": ["time", "value"], "values": [[940051, 21.25]]}]}
    ]});
    let value = history::decode(&response, Domain::Climate, 1, now)
        .unwrap()
        .json();
    assert_eq!(value["series"][0]["metric"], "temperature");
    assert_eq!(value["series"][0]["unit"], "°C");
    assert_eq!(value["series"][0]["name"], "hack");
    assert_eq!(value["series"][0]["samples"][0]["value"], 21.25);
    assert_eq!(value["series"][1]["metric"], "humidity");
    assert_eq!(value["series"][1]["unit"], "%");
    assert_eq!(value["series"][1]["samples"][1]["value"], 45.5);
}

#[test]
fn invalid_windows_fail_before_arithmetic_or_io() {
    for domain in [Domain::Climate, Domain::Power] {
        for minutes in [0, 43201, u64::MAX] {
            assert!(history::query(domain, minutes, 1_000_000).is_err());
            assert!(history::preview(domain, minutes, 1_000_000).is_err());
            assert!(history::decode(&json!({}), domain, minutes, 1_000_000).is_err());
        }
    }
}

#[tokio::test]
async fn shared_reader_uses_bounded_encoded_queries_and_reports_missing_history() {
    for domain in [Domain::Climate, Domain::Power] {
        let body = if domain == Domain::Climate {
            json!({"results": [{"statement_id": 0}, {"statement_id": 1}]})
        } else {
            json!({"results": [{"statement_id": 0}]})
        };
        let (options, server) = common::server(200, body.to_string(), Duration::ZERO);
        let history = history::read(&options, domain, 90).await.unwrap();
        assert!(history.series.is_empty());
        assert_eq!(history.json()["series"], json!([]));
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
        let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["db"], domain.database());
        assert_eq!(params["epoch"], "ms");
        assert_eq!(
            params["q"],
            history::query(domain, 90, history.end_ms).unwrap()
        );
    }
}
