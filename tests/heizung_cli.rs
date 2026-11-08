use std::process::Command;

use serde_json::Value;

#[test]
fn dry_runs_validate_and_preview_without_connecting() {
    for (args, wire) in [
        (vec!["get", "hack", "make"], "get hack make"),
        (vec!["status"], "get make hack cook elab shit storage"),
        (vec!["status", "hack", "--watch"], "get hack"),
        (
            vec!["set", "hack", "make", "21.5", "-d", "120"],
            "set hack make 21.5 -d 120",
        ),
        (
            vec!["set", "--all", "20"],
            "set make hack cook elab shit storage 20.0 -d 180",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args([
                "--heizung-amqp-url",
                "invalid://PRIVATE",
                "--dry-run",
                "--json",
                "heizung",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["args"], wire);
        assert_eq!(value["submitted"], false);
        assert_eq!(value["dry_run"], true);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE"));
    }
    for args in [
        vec!["set", "hack", "NaN"],
        vec!["set", "make", "26"],
        vec!["set", "make", "21.25"],
        vec!["set", "hack", "hack", "21"],
        vec!["get", "hack", "hack"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
            .args(["--dry-run", "heizung"])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn rooms_are_available_offline_and_quiet_suppresses_human_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args([
            "--heizung-amqp-url",
            "invalid://",
            "--json",
            "heizung",
            "rooms",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 6);
    let output = Command::new(env!("CARGO_BIN_EXE_subraumctl"))
        .args(["--quiet", "--dry-run", "heizung", "set", "hack", "21"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}
