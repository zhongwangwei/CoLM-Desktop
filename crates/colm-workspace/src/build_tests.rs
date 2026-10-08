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
    let run = gate(&ws, &bad).unwrap();
    assert!(!run.ok && run.detail.contains("broken") && run.commit == ws.head().unwrap());
    ws.info.gates.engine = Some(run);
    assert_eq!(
        ws.info.gates.lights(&ws.head().unwrap()).compile,
        crate::gates::Light::Fail
    );

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
