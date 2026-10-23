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
    let stopped = sweep(&dir, "colm-cli", |pid| {
        // 12 号被一个不相干的程序复用了。
        (pid == 12).then(|| "/usr/bin/vim notes.txt".to_owned())
    });
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
    let stopped = sweep(&dir, "colm-cli", alive_colm_cli);
    assert_eq!(stopped, vec!["/p/orphan".to_string()]);
    let status = child.wait().unwrap();
    assert!(!status.success(), "{status}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(dir).unwrap();
}
