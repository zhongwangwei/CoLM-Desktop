use super::*;
use crate::layout::layout_tests::{source_repo, temp};
use std::path::PathBuf;

fn fresh(tag: &str) -> (PathBuf, Workspace) {
    let root = temp(tag);
    let repo = root.join("source");
    source_repo(&repo);
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    (root, ws)
}

#[test]
fn search_finds_lines_in_both_languages_and_respects_the_glob() {
    let (root, ws) = fresh("search");
    let all = search(&ws, "twice", None).unwrap();
    assert!(all
        .iter()
        .any(|h| h.path.ends_with("MOD_Demo.F90") && h.line == 5));
    assert!(all.iter().any(|h| h.path.ends_with("demo.rs")));
    let only_rust = search(&ws, "twice", Some("*.rs")).unwrap();
    assert!(only_rust.iter().all(|h| h.path.ends_with(".rs")) && !only_rust.is_empty());
    assert!(search(&ws, "no_such_symbol_anywhere", None)
        .unwrap()
        .is_empty());
    assert!(search(&ws, "", None).is_err());
    assert!(search(&ws, "x", Some("../etc")).is_err());
    assert!(search(&ws, "x", Some("-O")).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reading_is_confined_to_the_workspace_and_capped() {
    let (root, ws) = fresh("read");
    let part = read_lines(&ws, "crates/colm-core/src/demo.rs", 3, 4).unwrap();
    assert_eq!((part.from, part.to, part.total_lines), (3, 4, 5));
    assert!(part.text.contains("     3  pub fn twice") && part.text.contains("     4      K * x"));
    // 越界的结束行被收到文件末尾。
    assert_eq!(
        read_lines(&ws, "crates/colm-core/src/demo.rs", 4, 999)
            .unwrap()
            .to,
        5
    );
    assert!(read_lines(&ws, "crates/colm-core/src/demo.rs", 0, 3).is_err());
    assert!(read_lines(&ws, "crates/colm-core/src/demo.rs", 9, 3).is_err());
    assert!(read_lines(&ws, "../../../etc/passwd", 1, 5).is_err());
    assert!(read_lines(&ws, ".git/config", 1, 5).is_err());
    assert!(
        read_lines(&ws, "crates", 1, 5).is_err(),
        "a directory is not a file"
    );
    assert!(read_lines(&ws, "nope.rs", 1, 5).is_err());
    // 一次最多 400 行。
    let long: String = (1..=1000).map(|i| format!("line {i}\n")).collect();
    std::fs::write(ws.src().join("long.txt"), long).unwrap();
    let excerpt = read_lines(&ws, "long.txt", 1, 1000).unwrap();
    assert_eq!(excerpt.to, MAX_READ_LINES);
    // 符号链接跑出工作区也不行。
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hosts", ws.src().join("link.txt")).unwrap();
        assert!(read_lines(&ws, "link.txt", 1, 3).is_err());
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn symbols_are_listed_for_fortran_and_rust() {
    let (root, ws) = fresh("symbols");
    let fortran = symbols(&ws, "vendor/CoLM202X/main/MOD_Demo.F90").unwrap();
    let names: Vec<(&str, &str)> = fortran
        .iter()
        .map(|s| (s.kind.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [("module", "MOD_Demo"), ("subroutine", "twice")],
        "END lines are not symbols"
    );
    let rust = symbols(&ws, "crates/colm-core/src/demo.rs").unwrap();
    let names: Vec<(&str, &str)> = rust
        .iter()
        .map(|s| (s.kind.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(names, [("const", "K"), ("fn", "twice")]);
    assert!(symbols(&ws, "Cargo.toml").is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fortran_and_rust_keywords_with_modifiers_are_recognised() {
    assert_eq!(
        fortran_symbol("RECURSIVE SUBROUTINE Solve(a)"),
        Some(("subroutine", "Solve".into()))
    );
    assert_eq!(
        fortran_symbol("pure elemental function sq(x)"),
        Some(("function", "sq".into()))
    );
    assert_eq!(
        fortran_symbol("real(r8) function calc(x)"),
        Some(("function", "calc".into()))
    );
    assert_eq!(fortran_symbol("module procedure foo"), None);
    assert_eq!(fortran_symbol("END SUBROUTINE solve"), None);
    assert_eq!(fortran_symbol("! subroutine commented"), None);
    assert_eq!(
        fortran_symbol("fm,    &! integral of profile function for momentum"),
        None
    );
    assert_eq!(
        fortran_symbol("SUBROUTINE THERMAL (a, b) ! the main function for heat"),
        Some(("subroutine", "THERMAL".into()))
    );
    assert_eq!(
        fortran_symbol("type :: patch_t"),
        Some(("type", "patch_t".into()))
    );
    assert_eq!(fortran_symbol("type(patch_t) :: p"), None);
    assert_eq!(
        rust_symbol("pub(crate) async fn go()"),
        Some(("fn", "go".into()))
    );
    assert_eq!(
        rust_symbol("pub const fn k() -> u8 {"),
        Some(("fn", "k".into()))
    );
    assert_eq!(
        rust_symbol("impl<T> Thing<T> {"),
        Some(("impl", "Thing".into()))
    );
    assert_eq!(rust_symbol("let fn_name = 3;"), None);
}
