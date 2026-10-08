use super::*;

/// HDF5 不能在多个线程里同时用：所有读写 NetCDF 的测试先拿这把锁。
pub(crate) static NC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 并行测试里别的线程在 `fork` 子进程（git、sh、sandbox-exec）时，HDF5 刚打开的文件的描述符会被子进程继承
/// （HDF5 打开文件不带 CLOEXEC），子进程活着期间 HDF5 的文件锁一直不放，随后的打开就报 -101。
/// 这是仓库里一直记着的“并行跑撞 HDF error -101”的根因。测试里关掉 HDF5 的文件锁（要在第一次用 HDF5 之前设）。
pub(crate) fn nc_lock() -> std::sync::MutexGuard<'static, ()> {
    static INIT: std::sync::Once = std::sync::Once::new();
    let guard = NC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    INIT.call_once(|| std::env::set_var("HDF5_USE_FILE_LOCKING", "FALSE"));
    guard
}

pub(crate) fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-ws-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 一个带两份源文件的小仓库，当作“CoLM 源码”。
pub(crate) fn source_repo(dir: &Path) {
    std::fs::create_dir_all(dir.join("vendor/CoLM202X/main")).unwrap();
    std::fs::create_dir_all(dir.join("crates/colm-core/src")).unwrap();
    std::fs::write(
        dir.join("vendor/CoLM202X/main/MOD_Demo.F90"),
        "MODULE MOD_Demo\n  IMPLICIT NONE\n  REAL(8), PARAMETER :: k = 2.0\nCONTAINS\n  SUBROUTINE twice(x, y)\n    REAL(8), INTENT(in) :: x\n    REAL(8), INTENT(out) :: y\n    y = k * x\n  END SUBROUTINE twice\nEND MODULE MOD_Demo\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("crates/colm-core/src/demo.rs"),
        "pub const K: f64 = 2.0;\n\npub fn twice(x: f64) -> f64 {\n    K * x\n}\n",
    )
    .unwrap();
    git::run(dir, &["init", "-q", "-b", "main"]).unwrap();
    git::set_identity(dir).unwrap();
    git::run(dir, &["add", "-A"]).unwrap();
    git::run(dir, &["commit", "-q", "-m", "initial"]).unwrap();
}

#[test]
fn names_are_checked_before_they_reach_a_path() {
    for ok in ["demo", "fix-1", "a_b", "A1"] {
        assert!(validate_name(ok).is_ok(), "{ok}");
    }
    for bad in ["", "../x", "a/b", "-x", "_x", "a b", &"x".repeat(41), "a.b"] {
        assert!(validate_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_workspace_is_cloned_listed_and_deleted() {
    let root = temp("life");
    let repo = root.join("source");
    source_repo(&repo);
    let ws_root = root.join("ws");
    let ws = Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).unwrap();
    assert_eq!(ws.info.branch, "ws/demo");
    assert!(ws.src().join("vendor/CoLM202X/main/MOD_Demo.F90").is_file());
    for sub in ["kernels", "bin", "runs", "reports"] {
        assert!(ws.dir.join(sub).is_dir(), "{sub}");
    }
    assert_eq!(git::run(&ws.src(), &["branch", "--show-current"]).unwrap(), "ws/demo");
    assert_eq!(ws.info.base_commit, ws.head().unwrap());

    // 重名拒绝，半成品不留下；来源不是仓库也拒绝。
    assert!(Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).is_err());
    assert!(Workspace::create(&ws_root, "bad", root.to_str().unwrap(), None).is_err());
    assert!(!ws_root.join("bad").exists());

    let list = Workspace::list(&ws_root).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].name.as_str(), list[0].commits, list[0].dirty), ("demo", 0, false));

    // 读回来的信息与保存的一致。
    let again = Workspace::open(&ws_root, "demo").unwrap();
    assert_eq!(again.info, ws.info);

    // 删除只认带 workspace.json 的、根目录下的那个目录。
    std::fs::create_dir_all(ws_root.join("other")).unwrap();
    assert!(Workspace::delete(&ws_root, "other").is_err());
    assert!(ws_root.join("other").is_dir());
    Workspace::delete(&ws_root, "demo").unwrap();
    assert!(!ws_root.join("demo").exists());
    assert!(repo.join(".git").is_dir(), "the source repository is untouched");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_source_package_becomes_a_repository_with_a_base_commit() {
    let root = temp("tar");
    let repo = root.join("source");
    source_repo(&repo);
    let tarball = root.join("colm-src.tar.gz");
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&tarball)
        .arg("-C")
        .arg(&repo)
        .args(["vendor", "crates"])
        .status()
        .unwrap();
    assert!(status.success());
    let ws = Workspace::create(&root.join("ws"), "fromtar", tarball.to_str().unwrap(), None).unwrap();
    assert!(ws.src().join("crates/colm-core/src/demo.rs").is_file());
    assert_eq!(git::commits_since(&ws.src(), &ws.info.base_commit).unwrap().len(), 0);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(&root);
}
