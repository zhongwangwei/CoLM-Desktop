use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-live-runs-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, key: &str, pid: u32, program: &str) {
    let entry = Entry {
        key: key.into(),
        pid,
        program: program.into(),
        operation_id: None,
        recorded_ms: None,
    };
    std::fs::write(
        dir.join(format!("{pid}.json")),
        serde_json::to_vec(&entry).unwrap(),
    )
    .unwrap();
}

#[test]
fn records_of_dead_or_reused_pids_are_dropped_without_stopping_anything() {
    let dir = scratch("stale");
    write(&dir, "/p/dead", 11, "colm-cli");
    write(&dir, "/p/reused", 12, "colm-cli");
    std::fs::write(dir.join("junk.json"), "not json").unwrap();
    let stopped = sweep(
        &dir,
        "colm-cli",
        |pid| {
            // 12 号被一个不相干的程序复用了。
            (pid == 12).then(|| "/usr/bin/vim notes.txt".to_owned())
        },
        None,
    );
    assert!(stopped.is_empty(), "{stopped:?}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_run_left_by_a_crashed_window_is_stopped_on_the_next_start() {
    let dir = scratch("orphan");
    let mut command = std::process::Command::new("sleep");
    command.arg("30");
    let mut child = colm_kernel::run::top_level_sidecar(&mut command)
        .spawn()
        .unwrap();
    let pid = child.id();
    write(&dir, "/p/orphan", pid, "sleep");
    let stopped = sweep(&dir, "colm-cli", alive_colm_cli, None);
    assert_eq!(stopped, vec!["/p/orphan".to_string()]);
    let status = child.wait().unwrap();
    assert!(!status.success(), "{status}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn interruption_settles_only_the_recorded_local_operation() {
    let dir = scratch("ledger");
    let ledger = colm_history::History::open(dir.join("history.sqlite3")).unwrap();
    let mut old = colm_history::Event::start("run");
    old.case_path = Some("/case/a".into());
    old.location = Some("local".into());
    ledger.append(&old).unwrap();
    let mut newer = colm_history::Event::start("run");
    newer.case_path = old.case_path.clone();
    ledger.append(&newer).unwrap();
    let entry = Entry {
        key: "/case/a".into(),
        pid: 42,
        program: "colm-cli".into(),
        operation_id: Some(old.operation_id.clone()),
        recorded_ms: Some(colm_history::now_ms()),
    };
    settle_interrupted(&ledger, &entry).unwrap();
    let result = ledger.detail(&old.operation_id).unwrap().unwrap();
    assert_eq!(result.operation.latest.status, "interrupted");
    assert!(!result.operation.needs_reconciliation);
    assert_eq!(
        ledger
            .detail(&newer.operation_id)
            .unwrap()
            .unwrap()
            .operation
            .latest
            .status,
        "running"
    );
    settle_interrupted(&ledger, &entry).unwrap();
    assert_eq!(
        ledger
            .detail(&old.operation_id)
            .unwrap()
            .unwrap()
            .events
            .len(),
        2
    );
    drop(ledger);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_registration_does_not_guess_between_unfinished_operations() {
    let dir = scratch("ambiguous");
    let ledger = colm_history::History::open(dir.join("history.sqlite3")).unwrap();
    for _ in 0..2 {
        let mut event = colm_history::Event::start("run");
        event.case_path = Some("/case/a".into());
        ledger.append(&event).unwrap();
    }
    let entry = Entry {
        key: "/case/a".into(),
        pid: 42,
        program: "colm-cli".into(),
        operation_id: None,
        recorded_ms: Some(colm_history::now_ms()),
    };
    assert!(settle_interrupted(&ledger, &entry).is_err());
    assert!(ledger
        .query(&Default::default())
        .unwrap()
        .items
        .iter()
        .all(|op| op.latest.status == "running"));
    drop(ledger);
    std::fs::remove_dir_all(dir).unwrap();
}
