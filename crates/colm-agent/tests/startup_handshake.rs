//! Startup IDs belong to the command that produced each event, including protocol failures.
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn stdio_echoes_each_startup_id_and_keeps_legacy_clients_compatible() {
    let root = std::env::temp_dir().join(format!(
        "colm-startup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_colm-agent"))
        .args(["--data-dir", root.to_str().unwrap(), "--cli"])
        .arg(env!("CARGO_BIN_EXE_colm-agent"))
        .arg("--key-file")
        .arg(root.join("keys.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for command in [
        json!({"type":"configure", "startup_id":"old", "provider":{"base_url":"https://example.invalid", "model":"offline"}, "project_root":root}),
        json!({"type":"new_session", "startup_id":"old"}),
        json!({"type":"configure", "startup_id":"failed"}),
        json!({"type":"new_session", "startup_id":"new"}),
        json!({"type":"resume_session", "id":"not-saved", "startup_id":"resume"}),
        json!({"type":"new_session"}),
    ] {
        writeln!(input, "{command}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|event: &Value| event["type"] == "ready" || event["type"] == "error")
        .collect();
    assert_eq!(events.len(), 6, "{events:?}");
    assert_eq!(
        events
            .iter()
            .map(|event| event["startup_id"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("old"),
            json!("old"),
            json!("failed"),
            json!("new"),
            json!("resume"),
            Value::Null
        ]
    );
    assert_eq!(events[2]["type"], "error");
    assert_ne!(events[1]["session"], events[3]["session"]);
    assert_eq!(events[3]["session"], events[4]["session"]);
    assert!(!events[5].as_object().unwrap().contains_key("startup_id"));
}
