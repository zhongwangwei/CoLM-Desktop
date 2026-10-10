use super::*;
use crate::layout::layout_tests::{source_repo, temp};

#[test]
fn only_known_presets_are_accepted() {
    for ok in ["default", "latlon", "catchment-hyper"] {
        assert!(check_preset(ok).is_ok());
    }
    for bad in ["", "../x", "default; rm -rf /", "Default", "latlon "] {
        assert!(check_preset(bad).is_err(), "{bad:?}");
    }
    assert_eq!(KERNEL_PRESETS.len(), 17);
}

/// 命令的输出进日志文件，失败如实报告，门槛记在当前提交上。
#[cfg(unix)]
#[test]
fn a_logged_command_records_its_output_and_a_gate_on_the_current_commit() {
    let root = temp("build");
    let repo = root.join("source");
    source_repo(&repo);
    let mut ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    let good = run_logged(
        &ws,
        "echo",
        Path::new("/bin/sh"),
        &["-c".into(), "echo hello; echo oops >&2".into()],
        &ws.src(),
        &[],
        false,
        None,
    )
    .unwrap();
    assert!(good.ok && good.tail.contains("hello") && good.tail.contains("oops"));
    let log = std::fs::read_to_string(&good.log).unwrap();
    assert!(log.contains("hello") && log.contains("--- stderr ---"));
    let bad = run_logged(
        &ws,
        "fail",
        Path::new("/bin/sh"),
        &["-c".into(), "echo broken >&2; exit 3".into()],
        &ws.src(),
        &[],
        false,
        None,
    )
    .unwrap();
    assert!(!bad.ok);
    let run = gate(&ws, &ws.head().unwrap(), &bad).unwrap();
    assert!(!run.ok && run.detail.contains("broken") && run.commit == ws.head().unwrap());
    ws.info.gates.engine = Some(run);
    assert_eq!(
        ws.info.gates.lights(&ws.head().unwrap()).compile,
        crate::gates::Light::Fail
    );

    let built_commit = ws.head().unwrap();
    crate::git::run(
        &ws.src(),
        &["commit", "--allow-empty", "-m", "concurrent edit"],
    )
    .unwrap();
    let stale = gate(&ws, &built_commit, &good).unwrap();
    assert!(!stale.ok);
    assert_eq!(stale.commit, built_commit);
    std::fs::write(ws.src().join("uncommitted.rs"), "changed").unwrap();
    assert!(!gate(&ws, &ws.head().unwrap(), &good).unwrap().ok);

    // 取消标志：长命令被中止。
    let flag = std::sync::atomic::AtomicBool::new(true);
    let cancelled = run_logged(
        &ws,
        "sleep",
        Path::new("/bin/sleep"),
        &["30".into()],
        &ws.src(),
        &[],
        false,
        Some(&flag),
    );
    assert!(cancelled.is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_kernel_build_needs_the_script_and_a_known_preset() {
    let root = temp("kernel");
    let repo = root.join("source");
    source_repo(&repo);
    let mut ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    assert!(build_kernel(&mut ws, "nope", None).is_err());
    let err = build_kernel(&mut ws, "default", None).unwrap_err();
    assert!(format!("{err}").contains("build_kernel.sh"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 记录判了不通过（编译或测试期间源码变了）时，返回给调用方（助手）的结果也必须是失败。
#[test]
fn a_gate_that_fails_after_a_successful_command_turns_the_outcome_into_a_failure() {
    let mut outcome = Outcome {
        ok: true,
        command: "cargo build".into(),
        log: std::path::PathBuf::from("/tmp/log"),
        tail: "Finished".into(),
        seconds: 1.0,
        sandbox: crate::sandbox::detect(),
    };
    let passed = crate::gates::GateRun {
        ok: true,
        at: 0,
        commit: "c".into(),
        detail: "1 s".into(),
    };
    disown_if_failed(&mut outcome, &passed);
    assert!(outcome.ok);
    let changed = crate::gates::GateRun {
        ok: false,
        at: 0,
        commit: "c".into(),
        detail: "source changed during build; rebuild the current commit".into(),
    };
    disown_if_failed(&mut outcome, &changed);
    assert!(!outcome.ok);
    assert!(outcome
        .tail
        .ends_with("source changed during build; rebuild the current commit"));
}
