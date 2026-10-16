use super::*;
use serde_json::json;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("colm-history-{}", new_id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.0.join(name);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn ledger() -> History {
    History::open(":memory:").unwrap()
}

#[test]
fn event_replay_collision_and_append_only_storage() {
    let history = ledger();
    let mut start = Event::start("run");
    start.time_ms = Some(1000);
    start.case_id = Some("site-A".into());
    assert!(history.append(&start).unwrap());
    assert!(!history.append(&start).unwrap());
    let mut collision = start.clone();
    collision.status = "failed".into();
    assert!(history.append(&collision).is_err());
    assert!(history
        .connection
        .execute("DELETE FROM history_events", [])
        .is_err());
    assert!(history
        .connection
        .execute("UPDATE history_events SET payload='{}'", [])
        .is_err());
    let pending = history.query(&Query::default()).unwrap();
    assert!(pending.items[0].needs_reconciliation);
    let mut end = start.next(EventPhase::Finished, "succeeded");
    end.time_ms = Some(4000);
    end.duration_ms = None;
    history.append(&end).unwrap();
    let detail = history.detail(&start.operation_id).unwrap().unwrap();
    assert_eq!(detail.events.len(), 2);
    assert_eq!(detail.operation.latest.duration_ms, Some(3000));
    assert!(!detail.operation.needs_reconciliation);
    assert_eq!(detail.operation.latest.case_id.as_deref(), Some("site-A"));
}
#[test]
fn failed_batch_rolls_back_and_independent_connections_observe_commit() {
    let dir = Temp::new();
    let h = History::open(dir.0.join("history.db")).unwrap();
    let a = Event::start("run");
    let mut invalid = Event::start("run");
    invalid.event_id.clear();
    assert!(h.append_batch(&[a.clone(), invalid]).is_err());
    assert_eq!(h.query(&Query::default()).unwrap().total, 0);
    let other = History::open(dir.0.join("history.db")).unwrap();
    h.append(&a).unwrap();
    assert_eq!(other.query(&Query::default()).unwrap().total, 1);
}
#[cfg(unix)]
#[test]
fn existing_case_alias_includes_canonical_history_in_pages_and_exports() {
    let dir = Temp::new();
    let actual = dir.0.join("actual case");
    std::fs::create_dir(&actual).unwrap();
    let alias = dir.0.join("case ' alias");
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let canonical = actual.canonicalize().unwrap();
    let history = ledger();
    let mut ids = Vec::new();
    for (index, path) in [&alias, &canonical, &dir.0.join("another case")]
        .into_iter()
        .enumerate()
    {
        let mut event = Event::start("run");
        event.time_ms = Some(index as i64);
        event.case_path = Some(path.to_string_lossy().into_owned());
        event.host = Some("local".into());
        ids.push(event.operation_id.clone());
        history.append(&event).unwrap();
    }
    let mut query = Query {
        case_path: Some(alias.to_string_lossy().into_owned()),
        ascending: true,
        limit: 1,
        ..Query::default()
    };
    let first = history.query(&query).unwrap();
    assert_eq!(first.total, 2);
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].latest.operation_id, ids[0]);
    query.offset = 1;
    let second = history.query(&query).unwrap();
    assert_eq!(second.total, 2);
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].latest.operation_id, ids[1]);
    let exported: Value = serde_json::from_str(&history.export_json(&query).unwrap()).unwrap();
    assert_eq!(exported["total"], 2);
    assert_eq!(exported["items"].as_array().unwrap().len(), 2);
    let csv = history.export_csv(&query).unwrap();
    assert_eq!(csv.lines().count(), 3);
    assert!(csv.contains(&ids[0]));
    assert!(csv.contains(&ids[1]));
    assert!(!csv.contains(&ids[2]));

    query.case_path = Some(canonical.to_string_lossy().into_owned());
    query.offset = 0;
    let canonical_page = history.query(&query).unwrap();
    assert_eq!(
        canonical_page.total, 2,
        "canonical lookup must also include existing aliases"
    );
    assert_eq!(canonical_page.items[0].latest.operation_id, ids[0]);
    let canonical_export: Value =
        serde_json::from_str(&history.export_json(&query).unwrap()).unwrap();
    assert_eq!(canonical_export["total"], 2);
    assert_eq!(canonical_export["items"].as_array().unwrap().len(), 2);

    let mut other_host = Event::start("run");
    other_host.case_path = Some(canonical.to_string_lossy().into_owned());
    other_host.host = Some("other-host".into());
    history.append(&other_host).unwrap();
    query.host = Some("local".into());
    assert_eq!(history.query(&query).unwrap().total, 2);
    assert!(!history
        .export_csv(&query)
        .unwrap()
        .contains(&other_host.operation_id));

    std::fs::remove_file(&alias).unwrap();
    query.case_path = Some(alias.to_string_lossy().into_owned());
    query.offset = 0;
    let missing_alias = history.query(&query).unwrap();
    assert_eq!(
        missing_alias.total, 1,
        "missing paths retain exact-string lookup"
    );
    assert_eq!(missing_alias.items[0].latest.operation_id, ids[0]);
}

#[test]
fn filters_pagination_relations_and_safe_csv() {
    let h = ledger();
    let mut parent = Event::start("study");
    parent.case_path = Some("=HYPERLINK(1)".into());
    parent.time_ms = Some(1);
    h.append(&parent).unwrap();
    for i in 2..5 {
        let mut e = Event::start("run");
        e.time_ms = Some(i);
        e.parent_operation_id = Some(parent.operation_id.clone());
        e.host = Some("7920".into());
        h.append(&e).unwrap();
    }
    let query = Query {
        kind: Some("run".into()),
        host: Some("7920".into()),
        limit: 1,
        offset: 1,
        ascending: true,
        ..Query::default()
    };
    let page = h.query(&query).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.items[0].latest.time_ms, Some(3));
    assert_eq!(
        h.detail(&parent.operation_id)
            .unwrap()
            .unwrap()
            .related
            .len(),
        3
    );
    let csv = h
        .export_csv(&Query {
            kind: Some("study".into()),
            ..Query::default()
        })
        .unwrap();
    assert!(csv.contains("\"'=HYPERLINK(1)\""));
    assert_eq!(
        serde_json::from_str::<Value>(&h.export_json(&query).unwrap()).unwrap()["total"],
        3
    );
    assert_eq!(
        h.query(&Query {
            search: Some("' OR 1=1 --".into()),
            ..Query::default()
        })
        .unwrap()
        .total,
        0
    );
}
#[test]
fn structured_secrets_removed_without_changing_science_values() {
    let h = ledger();
    let mut e = Event::start("run");
    e.detail = json!({"api_key":"secret123","nested":{"password":"pass123","temperature":273.15},"environment":{"MY_TOKEN":"abc"},"note":"token flux and password-independent parameter","log_path":"/logs/run.log","stdout":"secret output"});
    e.command = Some(Command {
        program: "colm-cli".into(),
        argv: vec![
            "run".into(),
            "--api-key=secret123".into(),
            "--password".into(),
            "pass123".into(),
            "--temperature".into(),
            "273.15".into(),
            "OPENAI_API_KEY=abc".into(),
        ],
        cwd: None,
    });
    h.append(&e).unwrap();
    let stored = h.detail(&e.operation_id).unwrap().unwrap().events.remove(0);
    let serialized = serde_json::to_string(&stored).unwrap();
    assert!(!serialized.contains("secret123"));
    assert!(!serialized.contains("pass123"));
    assert!(!serialized.contains("secret output"));
    assert_eq!(stored.detail["nested"]["temperature"], 273.15);
    assert_eq!(stored.detail["note"], e.detail["note"]);
    assert_eq!(stored.command.unwrap().argv[5], "273.15");
}
#[test]
fn legacy_import_deduplicates_provenance_and_reports_invalid_lines() {
    let dir = Temp::new();
    let h = ledger();
    let record = json!({"started":10,"finished":14,"ok":false,"version":{"name":"model","head":"abc"},"location":{"kind":"local"}});
    let runs = dir.file(".colm/runs.jsonl", &record);
    let provenance = dir.file("out/site/produced-by.json", &record);
    assert_eq!(
        h.import_file(runs, ImportKind::Runs, Some(&dir.0))
            .unwrap()
            .imported,
        1
    );
    assert_eq!(
        h.import_file(provenance, ImportKind::Provenance, Some(&dir.0))
            .unwrap()
            .duplicates,
        1
    );
    let op = &h.query(&Query::default()).unwrap().items[0];
    assert_eq!(op.latest.status, "failed");
    assert!(!op.needs_reconciliation);
    assert_eq!(op.latest.duration_ms, Some(4000));
    let bad = dir.0.join("bad.jsonl");
    std::fs::write(&bad, format!("{}\n{{broken", json!({"ok":true}))).unwrap();
    assert!(h.import_file(bad, ImportKind::Runs, None).is_err());
    assert_eq!(h.query(&Query::default()).unwrap().total, 1);
}
#[test]
fn remote_submission_never_infers_completion_and_gate_import_is_honest() {
    let dir = Temp::new();
    let h = ledger();
    let remote=dir.file("remote.json",&json!({"job":"123","host":"7920","submitted_at":20,"provenance":{"ok":true},"ssh_auth":{"password":"pw-7f3c-never-stored"}}));
    h.import_file(remote, ImportKind::RemoteRun, None).unwrap();
    let op = &h.query(&Query::default()).unwrap().items[0];
    assert!(op.needs_reconciliation);
    assert_eq!(op.latest.status, "needs_reconciliation");
    // A distinctive value: temp paths on macOS contain "/private", so a word would collide.
    assert!(!serde_json::to_string(op)
        .unwrap()
        .contains("pw-7f3c-never-stored"));
    let gates=dir.file("gates.json",&json!({"engine":{"ok":false,"at":10,"commit":"a"},"tests":{"core":{"ok":true,"at":12,"commit":"b"}}}));
    assert_eq!(
        h.import_file(&gates, ImportKind::Gates, None)
            .unwrap()
            .imported,
        2
    );
    assert_eq!(
        h.import_file(gates, ImportKind::Gates, None)
            .unwrap()
            .duplicates,
        2
    );
    let unknown = dir.file("unknown.json", &json!({"version":{"kind":"official"}}));
    h.import_file(unknown, ImportKind::Provenance, None)
        .unwrap();
    let unknown = h
        .query(&Query {
            status: Some("unknown".into()),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(unknown.items[0].latest.time_ms, None);
    assert!(unknown.items[0].needs_reconciliation);
}

#[test]
fn study_generations_share_operation_and_old_import_cannot_rewind_state() {
    let dir = Temp::new();
    let h = ledger();
    let manifest = dir.file(
        "manifest.json",
        &json!({"id":"study-A","created_unix":10,"members":[{"large":"metadata"}]}),
    );
    h.import_file(manifest, ImportKind::Study, None).unwrap();
    for (sequence, status) in [(2, "completed"), (1, "running")] {
        let payload = json!({"study_id":"study-A","status":status,"tasks":{}});
        let envelope = json!({"schema_version":1,"sequence":sequence,"payload_sha256":hash(&serde_json::to_vec(&payload).unwrap()),"payload":payload});
        let path = dir.file(&format!("checkpoints/state/{sequence:012}.json"), &envelope);
        h.import_file(path, ImportKind::Study, None).unwrap();
    }
    let page = h.query(&Query::default()).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].latest.status, "completed");
    assert_eq!(page.items[0].started_ms, Some(10000));
    assert_eq!(page.items[0].event_count, 3);
    assert!(!page.items[0].needs_reconciliation);
}

#[test]
fn command_credentials_in_urls_and_shell_are_not_stored() {
    let mut command = Command {
        program: "sh".into(),
        argv: vec![
            "-c".into(),
            "curl https://user:password@server --token xyz".into(),
        ],
        cwd: None,
    };
    command.sanitize().unwrap();
    assert_eq!(command.argv[1], "[REDACTED]");
    command.program = "colm-cli".into();
    command.argv = vec![
        "https://server/input?access_token=xyz".into(),
        "https://server/input?temperature=273.15".into(),
    ];
    command.sanitize().unwrap();
    // The token is removed whether the whole URL or only its value is redacted.
    assert!(!command.argv[0].contains("xyz"), "{}", command.argv[0]);
    assert!(command.argv[0].contains("[REDACTED"), "{}", command.argv[0]);
    assert_eq!(command.argv[1], "https://server/input?temperature=273.15");
}

#[test]
fn complete_exports_ignore_ui_pagination_and_cross_batch_boundaries() {
    let h = ledger();
    let rows: Vec<_> = (0..1003)
        .map(|i| {
            let mut event = Event::start(if i == 1002 { "check" } else { "run" });
            event.time_ms = Some(i - 1);
            event
        })
        .collect();
    h.append_batch(&rows).unwrap();
    let query = Query {
        kind: Some("run".into()),
        offset: 900,
        limit: 1,
        ascending: true,
        ..Query::default()
    };
    assert_eq!(h.query(&query).unwrap().items.len(), 1);
    let exported: Value = serde_json::from_str(&h.export_json(&query).unwrap()).unwrap();
    assert_eq!(exported["total"], 1002);
    let items = exported["items"].as_array().unwrap();
    assert_eq!(items.len(), 1002);
    assert_eq!(items[0]["time_ms"], -1);
    assert_eq!(items[1001]["time_ms"], 1000);
    let ids: std::collections::HashSet<_> = items
        .iter()
        .map(|item| item["operation_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 1002);
    let csv = h.export_csv(&query).unwrap();
    assert_eq!(csv.lines().count(), 1003);
    assert!(csv.lines().nth(1).unwrap().contains(",\"-1\","));
    assert!(!csv.contains("'-1"));
    assert!(h
        .query(&Query {
            limit: 10001,
            ..Query::default()
        })
        .is_err());
}

#[test]
fn verified_legacy_typed_checkpoint_import_preserves_generation_and_deduplication() {
    #[derive(Serialize)]
    struct LegacyState<'a> {
        study_id: &'a str,
        status: &'a str,
        generation: u64,
    }
    let state = LegacyState {
        study_id: "old-study",
        status: "completed",
        generation: 3,
    };
    let typed_bytes = serde_json::to_vec(&state).unwrap();
    let value: Value =
        serde_json::from_str(r#"{"generation":3,"status":"completed","study_id":"old-study"}"#)
            .unwrap();
    let typed_hash = hash(&typed_bytes);
    assert_ne!(typed_hash, hash(&serde_json::to_vec(&value).unwrap()));
    let envelope = json!({"sequence":7,"payload":value,"payload_sha256":typed_hash});
    let h = ledger();
    let dir = Temp::new();
    let path = dir.file("legacy.json", &envelope);
    assert!(h.import_file(&path, ImportKind::Study, None).is_err());
    assert_eq!(
        h.import_verified_record(
            &envelope,
            ImportKind::Study,
            "study:old-study/state:7",
            None
        )
        .unwrap()
        .imported,
        1
    );
    assert_eq!(
        h.import_verified_record(
            &envelope,
            ImportKind::Study,
            "study:old-study/state:7",
            None
        )
        .unwrap()
        .duplicates,
        1
    );
    let operation = h.query(&Query::default()).unwrap().items.remove(0);
    assert_eq!(operation.latest.detail["source_generation"], 7);
    assert_eq!(operation.latest.status, "completed");
    assert!(!operation.needs_reconciliation);
    assert!(h
        .import_verified_record(&json!({}), ImportKind::Study, "", None)
        .is_err());
}

#[test]
fn csv_protects_text_formulas_without_rewriting_typed_numbers() {
    assert_eq!(csv_cell("  =1+1", true), "\"'  =1+1\"");
    assert_eq!(csv_cell("\t@SUM(1)", true), "\"'\t@SUM(1)\"");
    assert_eq!(
        csv_cell("science, \"water\"", true),
        "\"science, \"\"water\"\"\""
    );
    assert_eq!(csv_cell("-5", false), "\"-5\"");
}

#[test]
fn verified_and_file_checkpoint_imports_share_stable_event_identity() {
    let dir = Temp::new();
    let h = ledger();
    let payload = json!({"study_id":"stable-study","status":"completed"});
    let envelope = json!({"schema_version":1,"sequence":4,"payload_sha256":hash(&serde_json::to_vec(&payload).unwrap()),"previous_sha256":null,"payload":payload});
    let file = dir.file("checkpoint.json", &envelope);
    assert_eq!(
        h.import_file(&file, ImportKind::Study, None)
            .unwrap()
            .imported,
        1
    );
    let normalized = json!({"sequence":4,"payload":payload});
    assert_eq!(
        h.import_verified_record(
            &normalized,
            ImportKind::Study,
            "verified/stable-study/4",
            None
        )
        .unwrap()
        .duplicates,
        1
    );
    assert_eq!(h.query(&Query::default()).unwrap().items[0].event_count, 1);
}

#[test]
fn two_connections_cannot_downgrade_a_completion_with_a_stale_observation() {
    let dir = Temp::new();
    let first = History::open(dir.0.join("history.db")).unwrap();
    let second = History::open(dir.0.join("history.db")).unwrap();
    let started = Event::start("run");
    first.append(&started).unwrap();
    let observed = first
        .detail(&started.operation_id)
        .unwrap()
        .unwrap()
        .operation
        .latest;
    let finished = started.next(EventPhase::Finished, "succeeded");
    assert!(second
        .append_if_latest(&started.event_id, &finished)
        .unwrap());
    let stale = observed.next(EventPhase::Status, "running");
    assert!(!first.append_if_latest(&observed.event_id, &stale).unwrap());
    assert!(
        !first.append_if_latest(&finished.event_id, &stale).unwrap(),
        "even a current caller cannot downgrade Finished to Status"
    );
    let detail = first.detail(&started.operation_id).unwrap().unwrap();
    assert_eq!(detail.operation.latest.event_id, finished.event_id);
    assert_eq!(detail.events.len(), 2);
    assert!(!detail.operation.needs_reconciliation);
    assert!(
        !first
            .append_if_latest(&finished.event_id, &finished)
            .unwrap(),
        "duplicate delivery stays idempotent"
    );
    let mut conflicting = finished.clone();
    conflicting.status = "failed".into();
    assert!(first
        .append_if_latest(&finished.event_id, &conflicting)
        .is_err());
}

#[test]
fn two_connections_retry_competing_status_from_the_new_current_event() {
    let dir = Temp::new();
    let first = History::open(dir.0.join("history.db")).unwrap();
    let second = History::open(dir.0.join("history.db")).unwrap();
    let started = Event::start("run");
    first.append(&started).unwrap();
    let running = started.next(EventPhase::Status, "running");
    let cancelling = started.next(EventPhase::Status, "cancel_requested");
    assert!(first.append_if_latest(&started.event_id, &running).unwrap());
    assert!(!second
        .append_if_latest(&started.event_id, &cancelling)
        .unwrap());
    let current = second
        .detail(&started.operation_id)
        .unwrap()
        .unwrap()
        .operation
        .latest;
    let refreshed = current.next(EventPhase::Status, "cancel_requested");
    assert!(second
        .append_if_latest(&current.event_id, &refreshed)
        .unwrap());
    let other = Event::start("run");
    assert!(
        !first.append_if_latest(&refreshed.event_id, &other).unwrap(),
        "CAS token belongs to this operation only"
    );
    assert_eq!(
        first
            .detail(&started.operation_id)
            .unwrap()
            .unwrap()
            .operation
            .latest
            .status,
        "cancel_requested"
    );
}

#[test]
fn large_provenance_is_bound_by_file_content_without_bloating_the_ledger() {
    let dir = Temp::new();
    let value = json!({"schema":1,"ok":true,"operation_id":"large-run","engine":"rust","full_input_identity":{"schema":1,"algorithm":"sha256-full-v1","complete":true,"data_sha256":"a".repeat(64),"files":{"large-inventory":"x".repeat(2*1024*1024)}}});
    let path = dir.file("produced-by.json", &value);
    let reference = capture_provenance(&path).unwrap();
    assert!(serde_json::to_vec(&reference).unwrap().len() < 4096);
    assert_eq!(load_provenance(&reference).unwrap(), value);
    let history = ledger();
    let mut event = Event::start("run");
    event.detail = json!({"provenance":reference});
    history.append(&event).unwrap();
    let stored = history.detail(&event.operation_id).unwrap().unwrap();
    assert_eq!(
        load_provenance(&stored.operation.latest.detail["provenance"]).unwrap(),
        value
    );
    std::fs::write(&path, serde_json::to_vec(&json!({"ok":false})).unwrap()).unwrap();
    assert!(load_provenance(&stored.operation.latest.detail["provenance"]).is_err());
    std::fs::remove_file(path).unwrap();
    assert!(load_provenance(&stored.operation.latest.detail["provenance"]).is_err());
}

#[test]
fn large_legacy_jsonl_and_provenance_import_are_small_idempotent_and_content_bound() {
    let dir = Temp::new();
    let row = json!({"schema":1,"ok":true,"started":1,"finished":2,"engine":"rust","version":{"kind":"official"},"full_input_identity":{"complete":true,"files":{"large":"x".repeat(2*1024*1024)}}});
    let produced = dir.file("produced-by.json", &row);
    let lines = dir.0.join("runs.jsonl");
    std::fs::write(
        &lines,
        format!("\n{}\n", serde_json::to_string(&row).unwrap()),
    )
    .unwrap();
    let history = ledger();
    assert_eq!(
        history
            .import_file(&lines, ImportKind::Runs, Some(&dir.0))
            .unwrap()
            .imported,
        1
    );
    assert_eq!(
        history
            .import_file(&lines, ImportKind::Runs, Some(&dir.0))
            .unwrap()
            .duplicates,
        1
    );
    assert_eq!(
        history
            .import_file(&produced, ImportKind::Provenance, Some(&dir.0))
            .unwrap()
            .duplicates,
        1
    );
    let operation = history.query(&Query::default()).unwrap().items.remove(0);
    assert!(serde_json::to_vec(&operation.latest).unwrap().len() < 8192);
    let reference = &operation.latest.detail["legacy"];
    assert_eq!(reference["jsonl_line"], 1);
    assert_eq!(load_provenance(reference).unwrap(), row);
    // The row is immutable evidence; later runs extend the log, not this row.
    use std::io::Write;
    let appended = json!({"ok":true,"started":3,"finished":4,"engine":"rust"});
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&lines)
            .unwrap(),
        "{}",
        serde_json::to_string(&appended).unwrap()
    )
    .unwrap();
    assert_eq!(load_provenance(reference).unwrap(), row);
    let mut old_reference = reference.clone();
    old_reference
        .as_object_mut()
        .unwrap()
        .remove("jsonl_row_sha256");
    old_reference
        .as_object_mut()
        .unwrap()
        .remove("jsonl_row_bytes");
    assert_eq!(load_provenance(&old_reference).unwrap(), row);
    let report = history
        .import_file(&lines, ImportKind::Runs, Some(&dir.0))
        .unwrap();
    assert_eq!((report.imported, report.duplicates), (1, 1));
    assert_eq!(load_provenance(reference).unwrap(), row);
    let mut changed = row.clone();
    changed["engine"] = json!("fortran");
    std::fs::write(
        &lines,
        format!(
            "\n{}\n{}\n",
            serde_json::to_string(&changed).unwrap(),
            serde_json::to_string(&appended).unwrap()
        ),
    )
    .unwrap();
    assert!(load_provenance(reference).is_err());
    assert!(load_provenance(&old_reference).is_err());
    std::fs::write(&lines, "{}\n").unwrap();
    assert!(load_provenance(reference).is_err());
}

#[test]
fn jsonl_row_reference_preserves_raw_bytes_and_allows_a_final_line_separator() {
    let dir = Temp::new();
    let row = json!({"ok":true,"input":"x".repeat(2*1024*1024)});
    let text = format!(" {} ", serde_json::to_string(&row).unwrap());
    let lines = dir.0.join("runs.jsonl");
    std::fs::write(&lines, &text).unwrap();
    let history = ledger();
    history.import_file(&lines, ImportKind::Runs, None).unwrap();
    let operation = history.query(&Query::default()).unwrap().items.remove(0);
    let reference = &operation.latest.detail["legacy"];
    assert_eq!(reference["jsonl_row_sha256"], hash(text.as_bytes()));
    assert_eq!(reference["jsonl_row_bytes"], text.len());
    assert_eq!(
        operation.latest.detail["reproducibility"]["source_sha256"],
        hash(&serde_json::to_vec(&row).unwrap())
    );
    std::fs::write(&lines, format!("{text}\n{{\"ok\":false}}\n")).unwrap();
    assert_eq!(load_provenance(reference).unwrap(), row);
    // Equal parsed JSON cannot hide a change to the recorded source bytes.
    std::fs::write(
        &lines,
        format!(
            "{}\n{{\"ok\":false}}\n",
            serde_json::to_string(&row).unwrap()
        ),
    )
    .unwrap();
    assert!(load_provenance(reference).is_err());
}

#[test]
fn large_study_members_are_referenced_before_the_ledger_projection_is_cleaned() {
    let dir = Temp::new();
    let payload = json!({"study_id":"large-study","status":"completed","created_unix":10,"members":[{"task_evidence":"x".repeat(2*1024*1024)}]});
    let envelope = json!({"schema_version":1,"sequence":2,"payload_sha256":hash(&serde_json::to_vec(&payload).unwrap()),"payload":payload});
    let path = dir.file("checkpoints/state/000000000002.json", &envelope);
    let original = std::fs::read(&path).unwrap();
    let history = ledger();
    let report = history.import_file(&path, ImportKind::Study, None).unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(
        history
            .import_file(&path, ImportKind::Study, None)
            .unwrap()
            .duplicates,
        1
    );
    let operation = history.query(&Query::default()).unwrap().items.remove(0);
    assert!(serde_json::to_vec(&operation.latest).unwrap().len() < 8192);
    assert_eq!(operation.latest.detail["source_generation"], 2);
    let reference = &operation.latest.detail["legacy"];
    assert_eq!(reference["kind"], "provenance-reference");
    assert_eq!(
        reference["row_sha256"],
        hash(&serde_json::to_vec(&payload).unwrap())
    );
    assert_eq!(load_provenance(reference).unwrap(), payload);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::write(&path, serde_json::to_vec(&json!({"payload":{}})).unwrap()).unwrap();
    assert!(load_provenance(reference).is_err());
}

#[test]
fn verified_large_study_keeps_its_original_checkpoint_after_reader_normalization() {
    let dir = Temp::new();
    let payload = json!({"study_id":"normalized-study","status":"running","tasks":{"one":{"evidence":"x".repeat(2*1024*1024)}}});
    let path = dir.file(
        "checkpoints/state/000000000003.json",
        &json!({"sequence":3,"payload_sha256":"legacy-typed-hash","payload":payload}),
    );
    let before = std::fs::read(&path).unwrap();
    let mut normalized = payload.clone();
    normalized["reader_default"] = json!([]);
    let verified = json!({"sequence":3,"payload":normalized});
    let source = path.to_str().unwrap();
    let history = ledger();
    assert_eq!(
        history
            .import_verified_record(&verified, ImportKind::Study, source, None)
            .unwrap()
            .imported,
        1
    );
    assert_eq!(
        history
            .import_verified_record(&verified, ImportKind::Study, source, None)
            .unwrap()
            .duplicates,
        1
    );
    let operation = history.query(&Query::default()).unwrap().items.remove(0);
    assert!(serde_json::to_vec(&operation.latest).unwrap().len() < 8192);
    let reference = &operation.latest.detail["legacy"];
    assert_eq!(reference["normalized_projection"], true);
    assert_eq!(
        reference["row_sha256"],
        hash(&serde_json::to_vec(&payload).unwrap())
    );
    assert_eq!(load_provenance(reference).unwrap(), payload);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn corrupt_jsonl_import_keeps_all_prior_rows_out_of_the_ledger() {
    let dir = Temp::new();
    let file = dir.0.join("runs.jsonl");
    std::fs::write(&file, "{\"ok\":true}\nnot-json\n").unwrap();
    let history = ledger();
    assert!(history.import_file(file, ImportKind::Runs, None).is_err());
    assert_eq!(history.query(&Query::default()).unwrap().total, 0);
}

#[test]
fn concurrent_first_open_preserves_every_audit_event() {
    let temp = Temp::new();
    for round in 0..8 {
        let path = temp.0.join(format!("concurrent-{round}.sqlite3"));
        let barrier = std::sync::Barrier::new(12);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..12)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        let history = History::open(&path).context("concurrent schema setup")?;
                        history
                            .append(&Event::start("parallel-audit"))
                            .context("concurrent audit append")?;
                        Ok::<_, anyhow::Error>(())
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        for result in results {
            result.unwrap();
        }
        assert_eq!(
            History::open(&path)
                .unwrap()
                .query(&Query::default())
                .unwrap()
                .total,
            12
        );
    }
}

#[test]
fn failed_schema_initialization_does_not_publish_partial_tables() {
    let temp = Temp::new();
    let path = temp.0.join("schema-conflict.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE VIEW history_operation AS SELECT 1;")
        .unwrap();
    let error = History::open(&path)
        .err()
        .expect("conflicting schema must fail");
    assert!(error
        .to_string()
        .contains("initialize operation ledger schema"));
    let table_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='history_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 0, "failed setup must roll back its new table");
    connection
        .execute_batch("DROP VIEW history_operation;")
        .unwrap();
    let history = History::open(&path).unwrap();
    history.append(&Event::start("after-repair")).unwrap();
    assert_eq!(history.query(&Query::default()).unwrap().total, 1);
}

#[test]
fn removal_persists_without_deleting_events_or_leaking_through_reads() {
    let dir = Temp::new();
    let path = dir.0.join("history.db");
    let history = History::open(&path).unwrap();
    let mut parent = Event::start("study");
    parent.case_path = Some("/case".into());
    history.append(&parent).unwrap();
    let mut child = Event::start("run");
    child.parent_operation_id = Some(parent.operation_id.clone());
    child.case_path = parent.case_path.clone();
    history.append(&child).unwrap();
    history
        .append(&child.next(EventPhase::Finished, "succeeded"))
        .unwrap();
    let other = Event::start("run").next(EventPhase::Finished, "failed");
    history.append(&other).unwrap();
    assert_eq!(
        history
            .detail(&parent.operation_id)
            .unwrap()
            .unwrap()
            .related
            .len(),
        1
    );
    assert!(history.remove_operation(&child.operation_id).unwrap());
    assert!(!history.remove_operation(&child.operation_id).unwrap());
    drop(history);
    let history = History::open(path).unwrap();
    assert!(history.detail(&child.operation_id).unwrap().is_none());
    assert!(history
        .detail(&parent.operation_id)
        .unwrap()
        .unwrap()
        .related
        .is_empty());
    let query = Query {
        limit: 1,
        offset: 1,
        ..Query::default()
    };
    let page = history.query(&query).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.items.len(), 1);
    assert!(history.detail(&other.operation_id).unwrap().is_some());
    let json: Value = serde_json::from_str(&history.export_json(&query).unwrap()).unwrap();
    assert_eq!(json["total"], 2);
    assert_eq!(json["items"].as_array().unwrap().len(), 2);
    for item in json["items"].as_array().unwrap() {
        assert_ne!(item["operation_id"], child.operation_id);
    }
    let csv = history.export_csv(&query).unwrap();
    assert_eq!(csv.lines().count(), 3);
    assert!(!csv.contains(&child.operation_id));
    let count: i64 = history
        .connection
        .query_row(
            "SELECT count(*) FROM history_events WHERE operation_id=?1",
            [&child.operation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    assert!(history
        .connection
        .execute("DELETE FROM history_events", [])
        .is_err());
}

#[test]
fn removed_import_stays_hidden_after_replay_and_new_generation() {
    let dir = Temp::new();
    let history = ledger();
    let path = dir.file(
        "state.json",
        &json!({"study_id":"removed-study","status":"completed"}),
    );
    history.import_file(&path, ImportKind::Study, None).unwrap();
    let operation = history.query(&Query::default()).unwrap().items.remove(0);
    history
        .remove_operation(&operation.latest.operation_id)
        .unwrap();
    assert_eq!(
        history
            .import_file(&path, ImportKind::Study, None)
            .unwrap()
            .duplicates,
        1
    );
    dir.file(
        "state.json",
        &json!({"study_id":"removed-study","status":"failed"}),
    );
    assert_eq!(
        history
            .import_file(&path, ImportKind::Study, None)
            .unwrap()
            .imported,
        1
    );
    assert_eq!(history.query(&Query::default()).unwrap().total, 0);
    assert!(history
        .detail(&operation.latest.operation_id)
        .unwrap()
        .is_none());
    let count: i64 = history
        .connection
        .query_row("SELECT count(*) FROM history_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn removal_rejects_unconfirmed_operations_and_unknown_ids() {
    let history = ledger();
    for (phase, status) in [
        (EventPhase::Started, "running"),
        (EventPhase::Status, "needs_reconciliation"),
        (EventPhase::Imported, "unknown"),
        (EventPhase::Finished, "running"),
        (EventPhase::Status, "succeeded"),
    ] {
        let event = Event::start("run").next(phase, status);
        history.append(&event).unwrap();
        assert!(history
            .remove_operation(&event.operation_id)
            .unwrap_err()
            .to_string()
            .contains("completion is unconfirmed"));
        assert!(history.detail(&event.operation_id).unwrap().is_some());
    }
    assert!(history
        .remove_operation("unknown-id")
        .unwrap_err()
        .to_string()
        .contains("not found"));
    assert!(history.remove_operation("").is_err());
    assert_eq!(history.query(&Query::default()).unwrap().total, 5);
}

#[test]
fn audit_retains_hidden_roots_and_new_active_generations() {
    let history = ledger();
    let mut study = Event::start("study").next(EventPhase::Imported, "completed");
    study.host = Some("server".into());
    study.case_path = Some("/old-root/case".into());
    study.detail = json!({"source_generation": 1, "root": "/old-root"});
    history.append(&study).unwrap();
    history.remove_operation(&study.operation_id).unwrap();
    let retained = history.detail_audit(&study.operation_id).unwrap().unwrap();
    assert_eq!(retained.operation.latest.detail["root"], "/old-root");
    let mut active = study.next(EventPhase::Imported, "running");
    active.detail["source_generation"] = json!(2);
    history.append(&active).unwrap();
    let query = Query {
        host: Some("server".into()),
        case_path: Some("/old-root/case".into()),
        needs_reconciliation: Some(true),
        ..Query::default()
    };
    let audit = history.query_audit(&query).unwrap();
    assert_eq!(audit.total, 1);
    assert_eq!(audit.items[0].latest.status, "running");
    let detail = history.detail_audit(&study.operation_id).unwrap().unwrap();
    assert_eq!(detail.events.len(), 2);
    assert!(detail.operation.needs_reconciliation);
    let mut child = Event::start("run");
    child.parent_operation_id = Some(study.operation_id.clone());
    history.append(&child).unwrap();
    assert_eq!(
        history
            .detail_audit(&child.operation_id)
            .unwrap()
            .unwrap()
            .related
            .len(),
        1
    );
    assert!(history
        .detail(&child.operation_id)
        .unwrap()
        .unwrap()
        .related
        .is_empty());
    assert_eq!(history.query(&query).unwrap().total, 0);
    assert!(history.detail(&study.operation_id).unwrap().is_none());
    let export: Value = serde_json::from_str(&history.export_json(&query).unwrap()).unwrap();
    assert_eq!(export["total"], 0);
    assert_eq!(history.export_csv(&query).unwrap().lines().count(), 1);
}
