#![allow(clippy::field_reassign_with_default)]
use super::*;
use crate::gates::GateRun;
use crate::layout::layout_tests::{source_repo, temp};

fn gate(ok: bool, commit: &str) -> GateRun {
    GateRun {
        ok,
        at: 1,
        commit: commit.into(),
        detail: String::new(),
    }
}

fn install_kernel(ws: &Workspace, preset: &str) {
    let dir = ws.kernels().join(preset);
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["colm.x", "mkinidata.x", "mksrfdata.x"] {
        std::fs::write(dir.join(file), "x").unwrap();
    }
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"preset":"default","generator_args":"SinglePoint LULC_IGBP CaMaOFF CROPOFF","macros":["LULC_IGBP","SinglePoint"],"colm_git_sha":"abc1234","platform":"Darwin-arm64"}"#,
    )
    .unwrap();
}

#[test]
fn only_kernels_that_compiled_and_passed_tests_on_the_current_commit_are_registered() {
    let root = temp("experimental");
    let repo = root.join("source");
    source_repo(&repo);
    let ws_root = root.join("ws");
    let mut ws = Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).unwrap();
    install_kernel(&ws, "default");
    assert!(
        experimental(&ws_root).unwrap().is_empty(),
        "nothing measured yet"
    );

    let head = ws.head().unwrap();
    ws.info.gates.engine = Some(gate(true, &head));
    ws.info
        .gates
        .kernels
        .insert("default".into(), gate(true, &head));
    ws.info
        .gates
        .tests
        .insert("cargo:colm-core".into(), gate(true, &head));
    ws.save().unwrap();
    let list = experimental(&ws_root).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].preset, "default");
    assert_eq!(list[0].label, "实验内核：demo（未审阅）· default");
    assert!(!list[0].regression_done && !list[0].reviewed);
    assert_eq!(list[0].dir, ws.kernels().join("default"));
    // 界面按编译宏匹配内核，所以清单里的身份要带上。
    assert_eq!(list[0].macros, ["LULC_IGBP", "SinglePoint"]);
    assert_eq!(
        (list[0].colm_git_sha.as_str(), list[0].platform.as_str()),
        ("abc1234", "Darwin-arm64")
    );

    // 一个没有编译记录的内核目录（手工放进去的）不登记。
    install_kernel(&ws, "usgs");
    assert_eq!(experimental(&ws_root).unwrap().len(), 1);

    // 回归不通过：不登记。
    ws.info.gates.regression = Some(crate::gates::Regression {
        kind: crate::gates::ChangeKind::Refactor,
        ok: false,
        at: 2,
        commit: head.clone(),
        case: "ref".into(),
        identical: 1,
        changed: 1,
        verdict: "no".into(),
    });
    ws.save().unwrap();
    assert!(experimental(&ws_root).unwrap().is_empty());

    // 之后又有新提交：旧的门槛作废，不再登记。
    ws.info.gates.regression = None;
    ws.save().unwrap();
    assert_eq!(experimental(&ws_root).unwrap().len(), 1);
    std::fs::write(ws.src().join("new.txt"), "x").unwrap();
    crate::git::run(&ws.src(), &["add", "new.txt"]).unwrap();
    crate::git::run(&ws.src(), &["commit", "-q", "-m", "more"]).unwrap();
    assert!(experimental(&ws_root).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
