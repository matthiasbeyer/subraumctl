#![cfg(feature = "strichliste")]

#[path = "common/strichliste.rs"]
mod fixture;

use serde_json::{Value, json};
use std::{process::Command, time::Duration};
use subraumctl::strichliste::{Client, Options, Request, execute_mcp};

fn options(url: String) -> Options {
    Options {
        url: Some(url),
        user_id: Some(27),
        timeout: Duration::from_secs(2),
    }
}

fn body(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn account_history_is_own_recent_transactions_in_euros_newest_first() {
    let mut history = fixture::transactions();
    history["entries"].as_array_mut().unwrap().reverse();
    let (url, server) = fixture::server(vec![(200, fixture::user()), (200, history)]);
    let account = Client::new(options(url))
        .unwrap()
        .account_snapshot()
        .await
        .unwrap();
    assert_eq!(account.balance.unwrap(), 12.345);
    let transactions = account.transactions.unwrap().entries;
    assert_eq!(
        transactions.iter().map(|item| item.id).collect::<Vec<_>>(),
        [104, 103, 102, 101]
    );
    assert_eq!(
        transactions
            .iter()
            .map(|item| item.kind())
            .collect::<Vec<_>>(),
        ["Transfer in", "Transfer out", "Add credit", "Pay"]
    );
    assert_eq!(transactions[1].value_eur, -0.01);
    assert_eq!(transactions[3].value_eur, -1.2345);
    assert!(transactions[3].comment.is_none());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /strich-api/user/27 "));
    assert!(requests[1].starts_with("GET /strich-api/user/27/transaction?limit=50&offset=0 "));
}

#[tokio::test]
async fn bad_history_does_not_hide_balance_or_leak_other_accounts() {
    for (field, invalid) in [
        ("userId", json!(30)),
        ("value", json!("NaN")),
        ("createDate", Value::Null),
        ("comment", json!({"private": "PRIVATE"})),
        ("id", json!(103)),
    ] {
        let mut history = fixture::transactions();
        history["entries"][0][field] = invalid;
        let (url, server) = fixture::server(vec![(200, fixture::user()), (200, history)]);
        let account = Client::new(options(url))
            .unwrap()
            .account_snapshot()
            .await
            .unwrap();
        assert_eq!(account.balance.unwrap(), 12.345);
        let error = account.transactions.unwrap_err().to_string();
        assert!(!error.contains("PRIVATE"));
        assert_eq!(server.join().unwrap().len(), 2);
    }
    for (history, valid) in [(json!([]), true), (Value::Null, false)] {
        let (url, server) = fixture::server(vec![(200, fixture::user()), (200, history)]);
        let account = Client::new(options(url))
            .unwrap()
            .account_snapshot()
            .await
            .unwrap();
        assert_eq!(account.transactions.is_ok(), valid);
        server.join().unwrap();
    }
    let mut user = fixture::user();
    user["id"] = json!(30);
    let (url, server) = fixture::server(vec![(200, user)]);
    assert!(
        Client::new(options(url))
            .unwrap()
            .account_snapshot()
            .await
            .is_err()
    );
    server.join().unwrap();
}

#[test]
fn config_defaults_to_deployment_url_and_uses_only_numeric_identity() {
    let help = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .env_remove("SUBRAUMCTL_STRICHLISTE_URL")
        .args(["--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("https://mate.wahrhe.it/strich-api"));
    assert!(!help.contains("SUBRAUMCTL_STRICHLISTE_USER="));
    let missing_id = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .env_remove("SUBRAUMCTL_STRICHLISTE_USER_ID")
        .env("SUBRAUMCTL_STRICHLISTE_USER", "Me")
        .args(["strichliste", "balance"])
        .output()
        .unwrap();
    assert!(!missing_id.status.success());
    assert!(
        String::from_utf8_lossy(&missing_id.stderr).contains("configure --strichliste-user-id")
    );
    let (url, server) = fixture::server(vec![(200, fixture::user())]);
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .env("SUBRAUMCTL_STRICHLISTE_URL", url)
        .env("SUBRAUMCTL_STRICHLISTE_USER_ID", "27")
        .env("SUBRAUMCTL_STRICHLISTE_USER", "ignored old setting")
        .args(["--json", "strichliste", "balance"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"balance_eur": 12.345})
    );
    assert!(server.join().unwrap()[0].starts_with("GET /strich-api/user/27 "));
}

#[test]
fn cli_transactions_projects_history_and_supports_offline_preview() {
    for json_output in [true, false] {
        let (url, server) = fixture::server(vec![(200, fixture::transactions())]);
        let mut command = Command::new(env!("CARGO_BIN_EXE_subraumctl"));
        command.args(["--strichliste-url", &url, "--strichliste-user-id", "27"]);
        if json_output {
            command.arg("--json");
        }
        let output = command
            .args(["strichliste", "transactions"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("DO NOT EXPOSE"));
        if json_output {
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["transactions"].as_array().unwrap().len(), 4);
            assert_eq!(value["transactions"][3]["value_eur"], -1.2345);
            assert_eq!(value["transactions"][0]["comment"], "from Other: test");
            assert!(value.get("balance").is_none());
        } else {
            for expected in [
                "Transfer in",
                "Transfer out",
                "Add credit",
                "Pay",
                "-1.23 EUR",
                "to Other: test",
            ] {
                assert!(text.contains(expected), "{text}");
            }
        }
        assert_eq!(server.join().unwrap().len(), 1);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args(["--dry-run", "--json", "strichliste", "transactions"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"dry_run": true})
    );
}

#[tokio::test]
async fn transaction_pages_follow_explicit_offsets_and_validate_metadata() {
    let (url, server) = fixture::server(vec![
        (200, fixture::transaction_page(0, 50, 112)),
        (200, fixture::transaction_page(50, 50, 112)),
        (200, fixture::transaction_page(100, 50, 112)),
    ]);
    let client = Client::new(options(url)).unwrap();
    let mut ids = Vec::new();
    let mut offset = 0;
    loop {
        let page = client.transactions(50, offset).await.unwrap();
        assert_eq!(page.total, Some(112));
        ids.extend(page.entries.iter().map(|entry| entry.id));
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(ids, (1..=112).rev().collect::<Vec<_>>());
    let requests = server.join().unwrap();
    for (request, offset) in requests.iter().zip([0, 50, 100]) {
        assert!(request.starts_with(&format!(
            "GET /strich-api/user/27/transaction?limit=50&offset={offset} "
        )));
    }
    for (field, invalid) in [
        ("offset", json!(1)),
        ("limit", json!(0)),
        ("overallCount", json!(2)),
        ("entries", json!([])),
    ] {
        let mut page = fixture::transactions();
        page[field] = invalid;
        let (url, server) = fixture::server(vec![(200, page)]);
        assert!(
            Client::new(options(url))
                .unwrap()
                .transactions(50, 0)
                .await
                .is_err()
        );
        server.join().unwrap();
    }
    // Servers may cap page size; advance by the count actually returned.
    let (url, server) = fixture::server(vec![(200, fixture::transaction_page(0, 10, 112))]);
    let page = Client::new(options(url))
        .unwrap()
        .transactions(50, 0)
        .await
        .unwrap();
    assert_eq!(page.next_offset, Some(10));
    server.join().unwrap();
}

#[test]
fn cli_transaction_limit_and_offset_are_used_and_invalid_limits_rejected() {
    let (url, server) = fixture::server(vec![(200, fixture::transaction_page(10, 10, 112))]);
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args([
            "--strichliste-url",
            &url,
            "--strichliste-user-id",
            "27",
            "--json",
            "strichliste",
            "transactions",
            "--limit",
            "10",
            "--offset",
            "10",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["transactions"].as_array().unwrap().len(), 10);
    assert_eq!(value["next_offset"], 20);
    assert_eq!(value["overall_count"], 112);
    assert!(
        server.join().unwrap()[0]
            .starts_with("GET /strich-api/user/27/transaction?limit=10&offset=10 ")
    );
    for limit in ["0", "1001", "-1"] {
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_subraumctl"))
                .args(["--dry-run", "strichliste", "transactions", "--limit", limit])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[tokio::test]
async fn euro_amounts_keep_fractional_cents_and_use_one_transaction_post() {
    for (request, expected) in [
        (Request::Pay(0.01), -0.01),
        (Request::Add(0.01), 0.01),
        (Request::Pay(1.0), -1.0),
        (Request::Add(1.0), 1.0),
        (Request::Pay(0.001), -0.001),
        (Request::Pay(std::f64::consts::PI), -std::f64::consts::PI),
    ] {
        let (url, server) = fixture::server(vec![(200, fixture::user()), (201, fixture::users())]);
        let result = Client::new(options(url))
            .unwrap()
            .execute(&request, "test")
            .await
            .unwrap();
        assert_eq!(result, json!({"submitted": true, "value_eur": expected}));
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /strich-api/user/27 "));
        assert!(requests[1].starts_with("POST /strich-api/user/27/transaction "));
        assert_eq!(
            body(&requests[1]),
            json!({"value": expected, "comment": "test"})
        );
    }
}

#[tokio::test]
async fn transfer_matches_the_supplied_contract() {
    let (url, server) = fixture::server(vec![
        (200, fixture::users()),
        (200, fixture::user()),
        (201, json!({})),
    ]);
    let client = Client::new(options(url)).unwrap();
    let users = client.users().await.unwrap();
    client.transfer(&users[1], 0.01, "test").await.unwrap();
    let requests = server.join().unwrap();
    assert!(requests[2].starts_with("POST /strich-api/user/27/transaction "));
    assert_eq!(
        body(&requests[2]),
        json!({
            "value": -0.01, "comment": "to Other: test", "toUserId": 30,
            "toUserComment": "from Me: test",
        })
    );
}

#[tokio::test]
async fn balance_rank_ties_and_mcp_projection() {
    let (url, server) = fixture::server(vec![(200, fixture::user())]);
    let result = execute_mcp(options(url), &Request::Balance, false)
        .await
        .unwrap();
    assert_eq!(result, json!({"balance_eur": 12.345}));
    server.join().unwrap();

    let (url, server) = fixture::server(vec![(200, fixture::user()), (200, fixture::users())]);
    let client = Client::new(options(url)).unwrap();
    assert_eq!(
        client.execute(&Request::Balance, "").await.unwrap(),
        json!({"balance_eur": 12.345})
    );
    assert_eq!(
        client.execute(&Request::Rank, "").await.unwrap(),
        json!({"rank": 2, "weighted_count_of_purchases": 10.5})
    );
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /strich-api/user/27 "));
    assert!(requests[1].starts_with("GET /strich-api/user "));
}

#[tokio::test]
async fn deployed_balance_is_euros_and_legacy_bare_arrays_still_work() {
    let mut user = fixture::user();
    user["balance"] = json!(-2.4181945088546075);
    let (url, server) = fixture::server(vec![
        (200, user),
        (200, fixture::users()["entries"].clone()),
    ]);
    let client = Client::new(options(url)).unwrap();
    assert_eq!(
        client.execute(&Request::Balance, "").await.unwrap(),
        json!({"balance_eur": -2.4181945088546075})
    );
    assert_eq!(client.users().await.unwrap().len(), 4);
    server.join().unwrap();
}

#[tokio::test]
async fn rank_includes_configured_id_on_later_user_pages() {
    let entries = fixture::users()["entries"].clone();
    let pages = vec![
        (
            200,
            json!({"overallCount": 4, "offset": 0, "limit": 2, "entries": [entries[1], entries[3]]}),
        ),
        (
            200,
            json!({"overallCount": 4, "offset": 2, "limit": 2, "entries": [entries[0], entries[2]]}),
        ),
    ];
    let (url, server) = fixture::server(pages);
    let result = Client::new(options(url))
        .unwrap()
        .execute(&Request::Rank, "")
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"rank": 2, "weighted_count_of_purchases": 10.5})
    );
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /strich-api/user "));
    assert!(requests[1].starts_with("GET /strich-api/user?limit=2&offset=2 "));
}

#[tokio::test]
async fn inconsistent_pages_fail_instead_of_returning_partial_rank_or_accounts() {
    let entries = fixture::users()["entries"].clone();
    let first =
        json!({"overallCount": 4, "offset": 0, "limit": 2, "entries": [entries[0], entries[1]]});
    for second in [
        json!({"overallCount": 4, "offset": 2, "limit": 2, "entries": []}),
        json!({"overallCount": 4, "offset": 2, "limit": 2, "entries": [entries[0], entries[2]]}),
        json!({"overallCount": 5, "offset": 2, "limit": 2, "entries": [entries[2], entries[3]]}),
        json!({"overallCount": 4, "offset": 0, "limit": 2, "entries": [entries[2], entries[3]]}),
        json!([entries[2], entries[3]]),
    ] {
        let (url, server) = fixture::server(vec![(200, first.clone()), (200, second)]);
        assert!(
            Client::new(options(url))
                .unwrap()
                .execute(&Request::Rank, "")
                .await
                .is_err()
        );
        assert_eq!(server.join().unwrap().len(), 2);
    }
    for invalid in [
        json!({"entries": []}),
        json!({"overallCount": -1, "entries": []}),
        json!({"overallCount": 100001, "entries": []}),
        json!({"overallCount": 0, "entries": [entries[0]]}),
        json!({"overallCount": 4, "limit": 0, "entries": []}),
        json!({"overallCount": 4, "offset": "PRIVATE", "entries": []}),
    ] {
        let (url, server) = fixture::server(vec![(200, invalid)]);
        let error = Client::new(options(url))
            .unwrap()
            .users()
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn read_rejections_report_route_and_status_without_response_bodies() {
    for status in [302, 401, 404, 500] {
        let (url, server) = fixture::server(vec![(status, json!({"error": "PRIVATE"}))]);
        let error = Client::new(options(url))
            .unwrap()
            .execute(&Request::Balance, "")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("GET /user/27"));
        assert!(error.contains(&format!("HTTP {status}")));
        assert!(!error.contains("PRIVATE"));
        assert!(!error.contains("127.0.0.1"));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn mcp_writes_force_the_audit_comment_and_hide_transaction_response() {
    for request in [Request::Pay(1.0), Request::Add(1.0)] {
        let (url, server) = fixture::server(vec![(200, fixture::user()), (201, fixture::users())]);
        let result = execute_mcp(options(url), &request, false).await.unwrap();
        assert_eq!(
            result,
            json!({"submitted": true, "value_eur": request.value().unwrap()})
        );
        let requests = server.join().unwrap();
        assert_eq!(body(&requests[1])["comment"], "triggered by mcp");
        assert_eq!(body(&requests[1]).as_object().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn missing_scores_and_mismatched_ids_fail_closed_but_duplicate_names_are_allowed() {
    let mut missing_score = fixture::users();
    missing_score["entries"][1]
        .as_object_mut()
        .unwrap()
        .remove("weightedCountOfPurchases");
    let mut ambiguous = fixture::users();
    ambiguous["entries"][1]["name"] = json!("me");
    for (users, valid) in [(missing_score, false), (ambiguous, true)] {
        let (url, server) = fixture::server(vec![(200, users)]);
        let client = Client::new(options(url)).unwrap();
        assert_eq!(client.execute(&Request::Rank, "").await.is_ok(), valid);
        assert_eq!(server.join().unwrap().len(), 1);
    }
    let mut wrong = fixture::user();
    wrong["id"] = json!(30);
    let (url, server) = fixture::server(vec![(200, wrong)]);
    assert!(
        Client::new(options(url))
            .unwrap()
            .execute(&Request::Pay(1.0), "")
            .await
            .is_err()
    );
    server.join().unwrap();
}

#[tokio::test]
async fn null_purchase_scores_count_as_zero_and_malformed_scores_still_fail() {
    let mut users = fixture::users();
    users["entries"][0]["weightedCountOfPurchases"] = Value::Null;
    let (url, server) = fixture::server(vec![(200, users.clone())]);
    let result = execute_mcp(options(url), &Request::Rank, false)
        .await
        .unwrap();
    // Other and Tie have purchases; the configured user ties with Lower at zero.
    assert_eq!(
        result,
        json!({"rank": 3, "weighted_count_of_purchases": 0.0})
    );
    server.join().unwrap();
    for invalid in [json!("NaN"), json!("unknown"), json!(false)] {
        users["entries"][1]["weightedCountOfPurchases"] = invalid;
        let (url, server) = fixture::server(vec![(200, users.clone())]);
        assert!(
            execute_mcp(options(url), &Request::Rank, false)
                .await
                .is_err()
        );
        server.join().unwrap();
    }
}

#[tokio::test]
async fn invalid_amounts_dry_run_and_errors_never_leak_raw_data() {
    for amount in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            execute_mcp(
                options("invalid://unused".into()),
                &Request::Pay(amount),
                true
            )
            .await
            .is_err()
        );
    }
    let preview = execute_mcp(
        options("invalid://unused".into()),
        &Request::Add(0.01),
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        preview,
        json!({"dry_run": true, "submitted": false, "value_eur": 0.01, "comment": "triggered by mcp"})
    );
    for status in [302, 400, 500] {
        let (url, server) = fixture::server(vec![
            (200, fixture::user()),
            (status, json!({"error": "PRIVATE"})),
        ]);
        let error = Client::new(options(url))
            .unwrap()
            .execute(&Request::Add(1.0), "")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        assert!(error.to_string().contains("POST /user/27/transaction"));
        assert!(error.to_string().contains(&format!("HTTP {status}")));
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[test]
fn cli_bookings_send_euros_and_print_the_same_amount() {
    for (verb, expected) in [("pay", -1.25), ("add", 1.25)] {
        let (url, server) = fixture::server(vec![(200, fixture::user()), (201, json!({}))]);
        let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args([
                "--strichliste-url",
                &url,
                "--strichliste-user-id",
                "27",
                "strichliste",
                verb,
                "1.25",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("Booked: {expected} €")
        );
        let requests = server.join().unwrap();
        assert_eq!(
            body(&requests[1]),
            json!({"value": expected, "comment": ""})
        );
    }
}

#[test]
fn cli_live_balance_offline_previews_and_noninteractive_send() {
    let (url, server) = fixture::server(vec![(200, fixture::user())]);
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args([
            "--strichliste-url",
            &url,
            "--strichliste-user-id",
            "27",
            "--json",
            "strichliste",
            "balance",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"balance_eur": 12.345})
    );
    server.join().unwrap();
    for verb in ["pay", "add", "send"] {
        let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args(["--dry-run", "--json", "strichliste", verb, "0.01"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["submitted"], false);
        assert_eq!(value["value_eur"], if verb == "add" { 0.01 } else { -0.01 });
    }
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args(["strichliste", "send", "1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
}
