#![allow(clippy::field_reassign_with_default)]
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
    assert_eq!(
        git::run(&ws.src(), &["branch", "--show-current"]).unwrap(),
        "ws/demo"
    );
    assert_eq!(ws.info.base_commit, ws.head().unwrap());

    // 重名拒绝，半成品不留下；来源不是仓库也拒绝。
    assert!(Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).is_err());
    assert!(Workspace::create(&ws_root, "bad", root.to_str().unwrap(), None).is_err());
    assert!(!ws_root.join("bad").exists());

    let list = Workspace::list(&ws_root).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].name.as_str(), list[0].commits, list[0].dirty),
        ("demo", 0, false)
    );

    // 读回来的信息与保存的一致。
    let again = Workspace::open(&ws_root, "demo").unwrap();
    assert_eq!(again.info, ws.info);

    // 删除只认带 workspace.json 的、根目录下的那个目录。
    std::fs::create_dir_all(ws_root.join("other")).unwrap();
    assert!(Workspace::delete(&ws_root, "other").is_err());
    assert!(ws_root.join("other").is_dir());
    Workspace::delete(&ws_root, "demo").unwrap();
    assert!(!ws_root.join("demo").exists());
    assert!(
        repo.join(".git").is_dir(),
        "the source repository is untouched"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_workspace_can_be_cloned_under_a_canonical_root() {
    let root = temp("canonical-clone");
    let repo = root.join("source");
    source_repo(&repo);
    let canonical = root.canonicalize().unwrap();
    let ws =
        Workspace::create(&canonical.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    assert_eq!(ws.head().unwrap(), git::head(&repo).unwrap());
    assert!(ws.src().join("crates/colm-core/src/demo.rs").is_file());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_relative_local_source_is_resolved_before_changing_clone_directory() {
    let root = PathBuf::from(format!("colm-ws-relative-clone-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let repo = root.join("source");
    source_repo(&repo);
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    assert_eq!(ws.head().unwrap(), git::head(&repo).unwrap());
    assert_eq!(ws.info.origin, repo.to_str().unwrap());
    assert!(ws.src().join("crates/colm-core/src/demo.rs").is_file());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_creation_reserves_one_workspace_without_deleting_the_winner() {
    let root = temp("concurrent-create");
    let repo = root.join("source");
    source_repo(&repo);
    let ws_root = root.join("ws");
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None)
        });
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    let workspace = Workspace::open(&ws_root, "demo").unwrap();
    assert!(workspace
        .src()
        .join("crates/colm-core/src/demo.rs")
        .is_file());
    assert_eq!(workspace.head().unwrap(), workspace.info.base_commit);
    std::fs::remove_dir_all(root).unwrap();
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
    let ws =
        Workspace::create(&root.join("ws"), "fromtar", tarball.to_str().unwrap(), None).unwrap();
    assert!(ws.src().join("crates/colm-core/src/demo.rs").is_file());
    assert_eq!(
        git::commits_since(&ws.src(), &ws.info.base_commit)
            .unwrap()
            .len(),
        0
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(&root);
}

/// 几条命令同时结束、各记各的门槛：谁也不能把别人的记录覆盖掉。
#[test]
fn concurrent_gate_updates_do_not_lose_each_other() {
    use crate::gates::GateRun;
    let root = temp("lock");
    let repo = root.join("source");
    source_repo(&repo);
    let ws_root = root.join("ws");
    let ws = Workspace::create(&ws_root, "demo", repo.to_str().unwrap(), None).unwrap();
    let dir = ws.dir.clone();
    let threads: Vec<_> = (0..12)
        .map(|i| {
            let ws_root = ws_root.clone();
            std::thread::spawn(move || {
                // 每个线程各自“读入”一份旧的工作区信息，再各改自己那一项。
                let mut ws = Workspace::open(&ws_root, "demo").unwrap();
                let run = GateRun {
                    ok: true,
                    at: i,
                    commit: "c".into(),
                    detail: String::new(),
                };
                ws.update(|info| {
                    info.gates.kernels.insert(format!("preset{i}"), run);
                })
                .unwrap();
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let after = Workspace::open(&ws_root, "demo").unwrap();
    assert_eq!(
        after.info.gates.kernels.len(),
        12,
        "{:?}",
        after.info.gates.kernels.keys().collect::<Vec<_>>()
    );
    assert!(!dir.join("workspace.lock").exists(), "the lock is released");
    assert!(std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
    // 崩溃留下的旧锁文件：超过一分钟的会被清掉，不会卡死。
    std::fs::write(dir.join("workspace.lock"), "").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join("workspace.lock"))
        .unwrap();
    file.set_modified(old).unwrap();
    drop(file);
    let mut ws = Workspace::open(&ws_root, "demo").unwrap();
    ws.update(|info| info.adopted.clear()).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_workspace_starts_from_a_frozen_plain_source_snapshot() {
    let root = temp("snapshot-plain");
    let snapshot = root.join("cache/source");
    std::fs::create_dir_all(snapshot.join("vendor/CoLM202X/main")).unwrap();
    std::fs::write(
        snapshot.join("vendor/CoLM202X/main/MOD_A.F90"),
        "MODULE MOD_A\nEND MODULE\n",
    )
    .unwrap();
    // Ignored names in the snapshot still belong to it: the frozen identity covered them.
    std::fs::write(snapshot.join(".gitignore"), "*.inc\n").unwrap();
    std::fs::write(snapshot.join("vendor/CoLM202X/main/kept.inc"), "x\n").unwrap();
    let ws =
        Workspace::create_from_snapshot(&root.join("ws"), "ver-1", &snapshot, "official r95", None)
            .unwrap();
    assert_eq!(ws.info.origin, "official r95");
    assert!(!git::is_dirty(&ws.src()).unwrap());
    assert_eq!(
        std::fs::read_to_string(ws.src().join("vendor/CoLM202X/main/MOD_A.F90")).unwrap(),
        "MODULE MOD_A\nEND MODULE\n"
    );
    let tracked = git::run(&ws.src(), &["ls-files"]).unwrap();
    assert!(tracked.contains("kept.inc"), "{tracked}");
    assert_eq!(ws.info.branch, "ws/ver-1");
}

/// 源码检出运行时官方版本的缓存是只 init + add、没有提交的 `.git`；原来被当仓库 clone，
/// rev-parse HEAD 失败，"开始修改代码"打不开（复验 A2-02）。
#[test]
fn a_snapshot_with_an_index_but_no_commit_is_copied_as_plain_files() {
    let root = temp("snapshot-index-only");
    let snapshot = root.join("cache/source");
    std::fs::create_dir_all(snapshot.join("main")).unwrap();
    std::fs::write(
        snapshot.join("main/MOD_A.F90"),
        "MODULE MOD_A\nEND MODULE\n",
    )
    .unwrap();
    git::run(&snapshot, &["init", "-q"]).unwrap();
    git::run(&snapshot, &["add", "-A", "-f"]).unwrap();
    assert!(!git::has_head(&snapshot));
    let ws =
        Workspace::create_from_snapshot(&root.join("ws"), "ver-5", &snapshot, "official r24", None)
            .unwrap();
    assert!(git::has_head(&ws.src()));
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let tracked = git::run(&ws.src(), &["ls-files"]).unwrap();
    assert_eq!(tracked, "main/MOD_A.F90");
    assert_eq!(ws.info.branch, "ws/ver-5");
}

#[test]
fn a_workspace_starts_from_the_frozen_commit_of_a_git_snapshot() {
    let root = temp("snapshot-git");
    let repo = root.join("frozen/src");
    source_repo(&repo);
    let frozen = git::head(&repo).unwrap();
    std::fs::write(repo.join("later.txt"), "after freezing\n").unwrap();
    git::run(&repo, &["add", "-A"]).unwrap();
    git::run(&repo, &["commit", "-q", "-m", "later"]).unwrap();
    let ws =
        Workspace::create_from_snapshot(&root.join("ws"), "ver-2", &repo, "demo r3", Some(&frozen))
            .unwrap();
    assert_eq!(ws.head().unwrap(), frozen);
    assert!(!ws.src().join("later.txt").exists());
}

#[cfg(unix)]
#[test]
fn a_snapshot_keeps_its_inner_links_but_refuses_links_that_leave_it() {
    let root = temp("snapshot-link");
    let snapshot = root.join("cache/source");
    std::fs::create_dir_all(snapshot.join("include")).unwrap();
    // Like vendor/CoLM202X/include/Makeoptions -> Makeoptions.github upstream.
    std::fs::write(
        snapshot.join("include/Makeoptions.github"),
        "FC = gfortran\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("Makeoptions.github", snapshot.join("include/Makeoptions")).unwrap();
    let ws =
        Workspace::create_from_snapshot(&root.join("ws"), "ver-3", &snapshot, "x", None).unwrap();
    let link = ws.src().join("include/Makeoptions");
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_to_string(&link).unwrap(), "FC = gfortran\n");
    for (name, target) in [("abs", "/etc"), ("up", "../../outside")] {
        let bad = root.join(format!("cache-{name}"));
        std::fs::create_dir_all(&bad).unwrap();
        std::os::unix::fs::symlink(target, bad.join("elsewhere")).unwrap();
        let error = Workspace::create_from_snapshot(&root.join("ws"), "ver-4", &bad, "x", None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("leaves it"), "{error}");
        assert!(!root.join("ws/ver-4").exists());
    }
}

#[test]
fn git_gets_registry_paths_without_the_windows_verbatim_prefix() {
    assert_eq!(
        git_spelling(r"\\?\C:\Users\a\models\sources\x\workspaces\soil\src"),
        r"C:\Users\a\models\sources\x\workspaces\soil\src"
    );
    assert_eq!(
        git_spelling(r"\\?\UNC\server\share\models"),
        r"\\server\share\models"
    );
    assert_eq!(git_spelling("/Users/a/models/src"), "/Users/a/models/src");
    assert_eq!(git_spelling("source.tar.gz"), "source.tar.gz");
}
