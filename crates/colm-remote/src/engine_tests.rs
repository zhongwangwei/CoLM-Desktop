use super::*;

#[test]
fn snapshot_ids_follow_content_not_time() {
    let dir = std::env::temp_dir().join(format!("colm-remote-id-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("crates")).unwrap();
    std::fs::write(dir.join("Cargo.lock"), "lock").unwrap();
    std::fs::write(dir.join("crates/a.rs"), "fn a() {}").unwrap();
    let files = vec!["Cargo.lock".to_owned(), "crates/a.rs".to_owned()];
    let first = content_id(&dir, &files).unwrap();
    assert_eq!(first.len(), 16);
    // 重写同样的内容（修改时间变了）：标识不变；改一个字节：标识变。
    std::fs::write(dir.join("crates/a.rs"), "fn a() {}").unwrap();
    assert_eq!(content_id(&dir, &files).unwrap(), first);
    std::fs::write(dir.join("crates/a.rs"), "fn b() {}").unwrap();
    assert_ne!(content_id(&dir, &files).unwrap(), first);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_checkout_is_found_and_the_build_script_is_locked_and_complete() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(Source::Checkout(repo)) = Source::find_checkout(here) else {
        panic!("no checkout above {}", here.display());
    };
    assert!(repo.join("crates/colm-remote").is_dir());
    let snapshot = snapshot(&Source::Checkout(repo)).unwrap();
    // 编译期读入的 vendor 文件在快照里（第 637 轮：缺了就编译失败）。
    assert!(snapshot
        .files
        .iter()
        .any(|f| f == "vendor/CoLM202X/run/forcing/ERA5.nml"));
    assert!(snapshot
        .files
        .iter()
        .any(|f| f == "crates/colm-cli/Cargo.toml"));
    assert!(!snapshot.files.iter().any(|f| f.starts_with("gui/")));

    let script = ensure_script("/data/colm", "abc", 32);
    assert!(script.contains("flock 9"));
    assert!(script.contains("CARGO_TARGET_DIR='/data/colm/target'"));
    assert!(script.contains("--release --locked -j 32"));
    for (package, binary) in BINARIES {
        assert!(
            script.contains(&format!("-p {package} --bin {binary}")),
            "{binary}"
        );
    }
    assert_eq!(engine_dir("/data/colm/", "abc"), "/data/colm/engine/abc");
}
