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
