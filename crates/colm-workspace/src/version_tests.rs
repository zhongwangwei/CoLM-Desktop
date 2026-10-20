use super::*;
use crate::gates::GateRun;
use crate::git;
use crate::layout::layout_tests::{source_repo, temp};

fn gate(commit: &str) -> GateRun {
    GateRun {
        ok: true,
        at: 1,
        commit: commit.into(),
        detail: String::new(),
    }
}

fn official_kernel(root: &Path) -> PathBuf {
    let dir = root.join("kernels/default");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"preset":"default"}"#).unwrap();
    dir
}

fn edit_and_commit(ws: &Workspace, path: &str) {
    let file = ws.src().join(path);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("{text}\n")).unwrap();
    git::set_identity(&ws.src()).unwrap();
    git::run(&ws.src(), &["commit", "-q", "-am", "change"]).unwrap();
}

fn mark_engine_built(ws: &mut Workspace) {
    let head = ws.head().unwrap();
    std::fs::create_dir_all(ws.bin()).unwrap();
    std::fs::write(ws.bin().join(exe("colm-cli")), "x").unwrap();
    ws.info.gates.engine = Some(gate(&head));
    ws.save().unwrap();
}

fn codes(r: &Readiness) -> Vec<(&'static str, bool)> {
    r.items.iter().map(|i| (i.code, i.blocking)).collect()
}

#[test]
fn changed_paths_are_sorted_into_engine_and_kernel() {
    assert_eq!(
        touches(["crates/colm-core/src/x.rs", "README.md"]),
        Touches {
            engine: true,
            kernel: false
        }
    );
    assert_eq!(
        touches(["vendor/CoLM202X/main/MOD_X.F90"]),
        Touches {
            engine: false,
            kernel: true
        }
    );
    assert_eq!(
        touches([
            "gui/dist/app/shell.js",
            "docs/a.md",
            "crates/colm-hybrid/src/a.rs"
        ]),
        Touches {
            engine: false,
            kernel: false
        }
    );
}

#[test]
fn a_rust_only_change_runs_with_the_official_kernel_once_the_engine_is_built() {
    let root = temp("version-rust");
    let repo = root.join("source");
    source_repo(&repo);
    let kernel = official_kernel(&root);
    let ws_root = root.join("ws");
    let mut ws = Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).unwrap();
    edit_and_commit(&ws, "crates/colm-core/src/demo.rs");

    let r = check(&ws_root, "demo", "rust", "rust", &kernel).unwrap();
    assert!(!r.ready, "engine not built yet");
    assert!(codes(&r).contains(&("engine_built", true)));

    mark_engine_built(&mut ws);
    let r = check(&ws_root, "demo", "rust", "rust", &kernel).unwrap();
    assert!(r.ready, "{:?}", r.items);
    assert_eq!(r.kernel, kernel);
    assert_eq!(r.preset, "default");
    assert!(r.touches.engine && !r.touches.kernel);
    assert_eq!(r.cli, ws.bin().join(exe("colm-cli")));

    // Fortran 引擎跑一个只改了 Rust 的版本：能跑，但提示。
    let r = check(&ws_root, "demo", "fortran", "rust", &kernel).unwrap();
    assert!(r.ready);
    assert!(codes(&r).contains(&("engine_mismatch", false)));

    // 新提交之后，旧的编译记录不算数。
    edit_and_commit(&ws, "crates/colm-core/src/demo.rs");
    let r = check(&ws_root, "demo", "rust", "rust", &kernel).unwrap();
    assert!(!r.ready);

    // 未提交的改动挡住运行。
    mark_engine_built(&mut ws);
    std::fs::write(ws.src().join("crates/colm-core/src/demo.rs"), "dirty").unwrap();
    let r = check(&ws_root, "demo", "rust", "rust", &kernel).unwrap();
    assert!(codes(&r).contains(&("clean", true)));
    assert!(!r.ready);
}

#[test]
fn a_fortran_change_needs_its_own_kernel_of_the_same_preset() {
    let root = temp("version-fortran");
    let repo = root.join("source");
    source_repo(&repo);
    let kernel = official_kernel(&root);
    let ws_root = root.join("ws");
    let mut ws = Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).unwrap();
    edit_and_commit(&ws, "vendor/CoLM202X/main/MOD_Demo.F90");
    mark_engine_built(&mut ws);

    let r = check(&ws_root, "demo", "fortran", "rust", &kernel).unwrap();
    assert!(!r.ready);
    assert!(codes(&r).contains(&("kernel_built", true)));

    let own = ws.kernels().join("default");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("manifest.json"), r#"{"preset":"default"}"#).unwrap();
    let head = ws.head().unwrap();
    ws.info.gates.kernels.insert("default".into(), gate(&head));
    ws.save().unwrap();
    let r = check(&ws_root, "demo", "fortran", "rust", &kernel).unwrap();
    assert!(r.ready, "{:?}", r.items);
    assert_eq!(r.kernel, own);

    // Rust 引擎跑一个只改了 Fortran 的版本：提示结果与官方版相同。
    let r = check(&ws_root, "demo", "rust", "rust", &kernel).unwrap();
    assert!(codes(&r).contains(&("engine_mismatch", false)));
}

/// 从另一个修改版分出来的版本：改动范围相对官方源码算，父版本改过的 Fortran 不能漏。
#[test]
fn a_derived_version_counts_its_parents_changes_from_the_official_source() {
    let root = temp("version-derived");
    let repo = root.join("source");
    source_repo(&repo);
    let kernel = official_kernel(&root);
    let ws_root = root.join("ws");
    let parent = Workspace::create(&ws_root, "parent", repo.to_str().unwrap(), None).unwrap();
    edit_and_commit(&parent, "vendor/CoLM202X/main/MOD_Demo.F90");
    let mut child =
        Workspace::create(&ws_root, "child", parent.src().to_str().unwrap(), None).unwrap();
    child
        .update(|info| {
            info.based_on = Some("parent".into());
            info.root_commit = Some(parent.official_base().to_owned());
            info.note = "try a Rust-only tweak on top".into();
        })
        .unwrap();
    edit_and_commit(&child, "crates/colm-core/src/demo.rs");
    mark_engine_built(&mut child);

    let r = check(&ws_root, "child", "rust", "rust", &kernel).unwrap();
    assert_eq!(
        r.touches,
        Touches {
            engine: true,
            kernel: true
        }
    );
    assert!(codes(&r).contains(&("kernel_built", true)), "{:?}", r.items);
    assert_eq!(r.based_on.as_deref(), Some("parent"));
    assert_eq!(r.note, "try a Rust-only tweak on top");
    let summary = child.summary().unwrap();
    assert!(summary.touches.kernel && summary.touches.engine);
    assert_eq!(
        summary.commits, 1,
        "commits still count from the parent's head"
    );
}

fn blocking(check: &FreezeCheck) -> Vec<&'static str> {
    check
        .items
        .iter()
        .filter(|i| i.blocking)
        .map(|i| i.code)
        .collect()
}

#[test]
fn a_version_freezes_only_when_built_consistent_and_its_parameters_still_exist() {
    let root = temp("freeze-check");
    let repo = root.join("source");
    source_repo(&repo);
    std::fs::create_dir_all(repo.join("vendor/CoLM202X/share")).unwrap();
    std::fs::write(
        repo.join("vendor/CoLM202X/share/MOD_Namelist.F90"),
        "LOGICAL :: DEF_USE_PLANTHYDRAULICS = .false.\nREAL(r8) :: DEF_Kmax_sun = 2.0\n",
    )
    .unwrap();
    git::run(&repo, &["add", "-A"]).unwrap();
    git::run(&repo, &["commit", "-q", "-m", "namelist"]).unwrap();
    let mut ws = Workspace::create(&root.join("ws"), "v", repo.to_str().unwrap(), None).unwrap();
    // Untouched code still needs this version's own engine built.
    let check = freeze_check(&ws, &[]).unwrap();
    assert_eq!(blocking(&check), ["engine_built"]);
    assert!(check.items.iter().any(|i| i.code == "no_model_change"));
    mark_engine_built(&mut ws);
    assert!(freeze_check(&ws, &[]).unwrap().ready);
    // Fortran changed: a kernel built on this commit is needed, and the engine again.
    edit_and_commit(&ws, "vendor/CoLM202X/main/MOD_Demo.F90");
    assert_eq!(
        blocking(&freeze_check(&ws, &[]).unwrap()),
        ["engine_built", "kernel_built"]
    );
    mark_engine_built(&mut ws);
    let head = ws.head().unwrap();
    std::fs::create_dir_all(ws.kernels().join("default")).unwrap();
    std::fs::write(ws.kernels().join("default/manifest.json"), "{}").unwrap();
    ws.info.gates.kernels.insert("default".into(), gate(&head));
    ws.save().unwrap();
    assert!(freeze_check(&ws, &[]).unwrap().ready);
    // The namelist source changed: the generated tables must pass the drift check on this commit.
    std::fs::write(
        ws.src().join("vendor/CoLM202X/share/MOD_Namelist.F90"),
        "LOGICAL :: DEF_USE_PLANTHYDRAULICS = .false.\n",
    )
    .unwrap();
    git::run(&ws.src(), &["commit", "-q", "-am", "drop Kmax"]).unwrap();
    mark_engine_built(&mut ws);
    let head = ws.head().unwrap();
    ws.info.gates.kernels.insert("default".into(), gate(&head));
    ws.save().unwrap();
    let keys = vec![
        "DEF_Kmax_sun".to_owned(),
        "DEF_USE_PLANTHYDRAULICS".to_owned(),
    ];
    let check = freeze_check(&ws, &keys).unwrap();
    assert_eq!(blocking(&check), ["drift", "parameters"]);
    let parameters = check.items.iter().find(|i| i.code == "parameters").unwrap();
    assert!(
        parameters.message.contains("DEF_Kmax_sun")
            && !parameters.message.contains("PLANTHYDRAULICS")
    );
    ws.info.gates.tests.insert("drift".into(), gate(&head));
    ws.save().unwrap();
    assert_eq!(blocking(&freeze_check(&ws, &keys).unwrap()), ["parameters"]);
    assert!(freeze_check(&ws, &keys[1..]).unwrap().ready);
    // Uncommitted edits block.
    std::fs::write(ws.src().join("scratch.txt"), "x").unwrap();
    assert!(blocking(&freeze_check(&ws, &[]).unwrap()).contains(&"clean"));
}

#[test]
fn namelist_variables_added_or_removed_by_a_version_are_reported() {
    let root = temp("freeze-namelist");
    let repo = root.join("source");
    source_repo(&repo);
    let namelist = "vendor/CoLM202X/share/MOD_Namelist.F90";
    std::fs::create_dir_all(repo.join("vendor/CoLM202X/share")).unwrap();
    std::fs::write(
        repo.join(namelist),
        "namelist /nl_colm/ DEF_USE_PLANTHYDRAULICS, &\n   DEF_Kmax_sun\n",
    )
    .unwrap();
    git::run(&repo, &["add", "-A"]).unwrap();
    git::run(&repo, &["commit", "-q", "-m", "namelist"]).unwrap();
    let ws = Workspace::create(&root.join("ws"), "v", repo.to_str().unwrap(), None).unwrap();
    assert!(!freeze_check(&ws, &[])
        .unwrap()
        .items
        .iter()
        .any(|i| i.code == "namelist"));
    std::fs::write(
        ws.src().join(namelist),
        "namelist /nl_colm/ DEF_USE_PLANTHYDRAULICS, DEF_USE_OZONESTRESS, &\n   DEF_My_switch\n",
    )
    .unwrap();
    git::set_identity(&ws.src()).unwrap();
    git::run(&ws.src(), &["commit", "-q", "-am", "my switch"]).unwrap();
    let check = freeze_check(&ws, &[]).unwrap();
    let item = check.items.iter().find(|i| i.code == "namelist").unwrap();
    // Informational only: the new switch works with its default in the code.
    assert!(!item.ok && !item.blocking, "{item:?}");
    let parts: Vec<&str> = item.message.split('；').collect();
    assert_eq!(parts.len(), 3, "{}", item.message);
    assert!(parts[0].contains("不认识") && parts[0].ends_with("DEF_My_switch"));
    assert!(parts[1].contains("已认识") && parts[1].ends_with("DEF_USE_OZONESTRESS"));
    assert!(parts[2].contains("删掉") && parts[2].ends_with("DEF_Kmax_sun"));
}
