use super::*;
use crate::layout::layout_tests::{source_repo, temp};
use std::path::PathBuf;

/// 把 Fortran 里的常数 2.0 改成 3.0 的补丁（路径相对仓库根）。
const FORTRAN_DIFF: &str = "diff --git a/vendor/CoLM202X/main/MOD_Demo.F90 b/vendor/CoLM202X/main/MOD_Demo.F90\n--- a/vendor/CoLM202X/main/MOD_Demo.F90\n+++ b/vendor/CoLM202X/main/MOD_Demo.F90\n@@ -1,5 +1,5 @@\n MODULE MOD_Demo\n   IMPLICIT NONE\n-  REAL(8), PARAMETER :: k = 2.0\n+  REAL(8), PARAMETER :: k = 3.0\n CONTAINS\n   SUBROUTINE twice(x, y)\n     REAL(8), INTENT(in) :: x\n";

fn fresh(tag: &str) -> (PathBuf, Workspace) {
    let root = temp(tag);
    let repo = root.join("source");
    source_repo(&repo);
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    (root, ws)
}

#[test]
fn paths_are_extracted_and_the_forbidden_ones_refused() {
    let paths = touched_paths(FORTRAN_DIFF).unwrap();
    assert_eq!(paths, ["vendor/CoLM202X/main/MOD_Demo.F90"]);
    let created = "diff --git a/new.rs b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1 @@\n+fn x() {}\n";
    assert_eq!(touched_paths(created).unwrap(), ["new.rs"]);
    assert!(touched_paths("just words\n").is_err());
    for bad in [
        "../outside",
        "/etc/passwd",
        "a/../../b",
        ".git/config",
        "src/.git/hooks/x",
        "oracle/golden/CN-Cng.nc",
        "",
        "a\\b",
    ] {
        assert!(check_path(bad).is_err(), "{bad:?}");
    }
    for ok in [
        "crates/colm-core/src/demo.rs",
        "vendor/CoLM202X/main/MOD_Demo.F90",
        "oracle/scripts/x.sh",
    ] {
        assert!(check_path(ok).is_ok(), "{ok}");
    }
}

#[test]
fn a_patch_becomes_a_commit_and_can_be_reverted() {
    let (root, ws) = fresh("patch");
    let applied = apply(&ws, FORTRAN_DIFF, "k = 3").unwrap();
    assert_eq!(applied.files, ["vendor/CoLM202X/main/MOD_Demo.F90"]);
    let text = std::fs::read_to_string(ws.src().join("vendor/CoLM202X/main/MOD_Demo.F90")).unwrap();
    assert!(text.contains("k = 3.0") && !text.contains("k = 2.0"));
    let commits = git::commits_since(&ws.src(), &ws.info.base_commit).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].subject, "ws: k = 3");
    assert!(!git::is_dirty(&ws.src()).unwrap());

    // 再打同一个补丁：已经改过了，应用不上，文件不动。
    let err = apply(&ws, FORTRAN_DIFF, "again").unwrap_err();
    assert!(format!("{err:#}").contains("does not apply"), "{err:#}");
    assert_eq!(
        git::commits_since(&ws.src(), &ws.info.base_commit)
            .unwrap()
            .len(),
        1
    );

    // 撤回到基线：文件回到原样，提交被丢弃。
    revert_to(&ws, &ws.info.base_commit).unwrap();
    let text = std::fs::read_to_string(ws.src().join("vendor/CoLM202X/main/MOD_Demo.F90")).unwrap();
    assert!(text.contains("k = 2.0"));
    assert!(git::commits_since(&ws.src(), &ws.info.base_commit)
        .unwrap()
        .is_empty());
    // 只能撤回到工作区自己的提交；乱写的提交号也拒绝。
    assert!(revert_to(&ws, "zzzz").is_err());
    assert!(revert_to(&ws, "").is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bad_patches_change_nothing() {
    let (root, ws) = fresh("bad");
    let before = git::head(&ws.src()).unwrap();
    // 越界、碰黄金、二进制、超大、空提交说明、碰不存在的内容。
    let outside = FORTRAN_DIFF.replace("vendor/CoLM202X/main/MOD_Demo.F90", "../evil.F90");
    assert!(apply(&ws, &outside, "x").is_err());
    let golden = FORTRAN_DIFF.replace("vendor/CoLM202X/main/MOD_Demo.F90", "oracle/golden/a.nc");
    assert!(apply(&ws, &golden, "x").is_err());
    assert!(apply(&ws, &format!("{FORTRAN_DIFF}GIT binary patch\n"), "x").is_err());
    assert!(apply(&ws, &"x".repeat(MAX_PATCH_BYTES + 1), "x").is_err());
    assert!(apply(&ws, FORTRAN_DIFF, "").is_err());
    assert!(apply(&ws, FORTRAN_DIFF, "two\nlines").is_err());
    let wrong = FORTRAN_DIFF.replace("k = 2.0", "k = 9.0");
    assert!(apply(&ws, &wrong, "x").is_err());
    assert_eq!(git::head(&ws.src()).unwrap(), before);
    assert!(!git::is_dirty(&ws.src()).unwrap());

    // 工作区里有没提交的改动时拒绝，免得把别人的改动混进补丁提交。
    std::fs::write(ws.src().join("stray.txt"), "x").unwrap();
    git::run(&ws.src(), &["add", "stray.txt"]).unwrap();
    assert!(apply(&ws, FORTRAN_DIFF, "x").is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_whole_change_can_be_exported_as_one_patch() {
    let (root, ws) = fresh("export");
    apply(&ws, FORTRAN_DIFF, "k = 3").unwrap();
    let exported = export(&ws).unwrap();
    assert!(exported.contains("-  REAL(8), PARAMETER :: k = 2.0"));
    assert!(exported.contains("+  REAL(8), PARAMETER :: k = 3.0"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn quoted_protected_paths_are_rejected_before_any_file_changes() {
    let (root, ws) = fresh("quoted");
    let path = ws.src().join("oracle/golden/reference.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "original\n").unwrap();
    git::run(&ws.src(), &["add", "."]).unwrap();
    git::run(&ws.src(), &["commit", "-m", "fixture"]).unwrap();
    let before = ws.head().unwrap();
    let diff = "diff --git \"a/oracle/golden/reference.txt\" \"b/oracle/golden/reference.txt\"\n--- \"a/oracle/golden/reference.txt\"\n+++ \"b/oracle/golden/reference.txt\"\n@@ -1 +1 @@\n-original\n+tampered\n";
    let err = apply(&ws, diff, "tamper").unwrap_err();
    assert!(err.to_string().contains("quoted"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");
    assert_eq!(ws.head().unwrap(), before);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

/// 只改权限的一段没有 `---`/`+++`，路径只在 `diff --git` 头里；它和一段正常改动拼在一起时也要被挡住。
#[test]
fn mode_only_sections_on_protected_paths_are_rejected() {
    let (root, ws) = fresh("modeonly");
    let golden = ws.src().join("oracle/golden/reference.txt");
    std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
    std::fs::write(&golden, "original\n").unwrap();
    crate::git::run(&ws.src(), &["add", "-A"]).unwrap();
    crate::git::run(&ws.src(), &["commit", "-q", "-m", "golden"]).unwrap();
    let diff = format!(
        "diff --git a/oracle/golden/reference.txt b/oracle/golden/reference.txt\nold mode 100644\nnew mode 100755\n{FORTRAN_DIFF}"
    );
    assert!(touched_paths(&diff)
        .unwrap()
        .iter()
        .any(|p| p == "oracle/golden/reference.txt"));
    assert!(apply(&ws, &diff, "x").is_err());
    let mode = std::fs::metadata(&golden).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            mode.permissions().mode() & 0o111,
            0,
            "the protected file must keep its mode"
        );
    }
    let _ = mode;
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diff_headers_with_spaces_and_renames_are_split_correctly() {
    assert_eq!(
        header_paths("a/my dir/x.F90 b/my dir/x.F90").unwrap(),
        ["my dir/x.F90"]
    );
    assert_eq!(
        header_paths("a/old.rs b/new.rs").unwrap(),
        ["old.rs", "new.rs"]
    );
    assert!(header_paths("old.rs new.rs").is_err());
}
