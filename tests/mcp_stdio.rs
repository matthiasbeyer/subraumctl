#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn real_stdio_keeps_startup_warnings_off_the_protocol_stream() {
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args(["--quiet", "mcp", "--config"])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/mcp-warnings.toml"
            ))
            .env("RUST_LOG", "off")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut stdin = child.0.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "stdio-test", "version": "1"}
            }
        })
    )
    .unwrap();
    let response: Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    assert_eq!(response["result"]["serverInfo"]["name"], "subraumctl");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    )
    .unwrap();
    let response: Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    assert_eq!(response["id"], 2);
    let names: Vec<_> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "alert_led",
            "alert_radiation",
            "alert_sob",
            "climate_history",
            "climate_status",
            "dmx",
            "heizung_get",
            "heizung_rooms",
            "heizung_set",
            "heizung_status",
            "knob_get",
            "knob_list",
            "knob_set",
            "music_next",
            "music_pause",
            "music_play",
            "music_previous",
            "music_station",
            "music_stations",
            "music_status",
            "outlet",
            "pizzamat_add",
            "pizzamat_clear",
            "pizzamat_commit",
            "pizzamat_join",
            "pizzamat_list",
            "pizzamat_lock",
            "pizzamat_remove",
            "pizzamat_reset",
            "pizzamat_service_add",
            "pizzamat_service_remove",
            "pizzamat_services",
            "pizzamat_set_default",
            "pizzamat_start",
            "pizzamat_unlock",
            "power_history",
            "power_status",
            "smoothiesaur_status",
            "status",
            "system_telinit",
            "white_light_apply",
            "white_light_cells",
            "white_light_status"
        ]
    );
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "MCP did not exit on EOF");
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
    assert!(rx.try_recv().is_err(), "unexpected stdout output");
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.contains("'outlet' is enabled: this is dangerous and might break things"));
    assert!(stderr.contains("'smoothiesaur' is enabled: this is dangerous and might break things"));
    assert!(stderr.contains("'publish' is write-only; permission 'r' disables it"));
    assert_eq!(stderr.lines().count(), 3, "{stderr}");
}

#[test]
fn missing_config_fails_before_writing_protocol_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args(["mcp", "--config"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/Cargo.toml/missing-config.toml"
        ))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read MCP config"));
}
