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

#[test]
fn the_snapshot_carries_the_linux_build_script() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(Source::Checkout(repo)) = Source::find_checkout(here) else {
        panic!("no checkout above {}", here.display());
    };
    let snapshot = snapshot(&Source::Checkout(repo)).unwrap();
    assert!(snapshot
        .files
        .iter()
        .any(|f| f == "scripts/build-engine-linux.sh"));
}

#[test]
fn prebuilt_packages_are_found_by_arch_and_identified_by_content() {
    let dir = std::env::temp_dir().join(format!("colm-remote-pre-{}", std::process::id()));
    let cache = dir.join("cache");
    let exe = dir.join("app");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(exe.join("engine")).unwrap();
    std::env::remove_var("COLM_RESOURCE_DIR");
    let dirs = resource_dirs(&exe);
    std::env::set_var("COLM_ENGINE_CACHE", &cache);
    assert_eq!(prebuilt_name("aarch64"), "colm-engine-linux-aarch64.tar.gz");
    assert_eq!(cache_dir(), cache);

    // 什么都没有。
    assert!(find_prebuilt("x86_64", Some("abc"), &dirs).is_none());

    // 缓存里的包带快照标识：标识对得上才用，源码改了就不匹配。
    std::fs::write(cache.join(cached_name("x86_64", "abc")), b"engine one").unwrap();
    let cached = find_prebuilt("x86_64", Some("abc"), &dirs).unwrap();
    assert!(cached.id.starts_with("pre-") && cached.id.len() == 20);
    assert!(find_prebuilt("x86_64", Some("def"), &dirs).is_none());
    assert!(find_prebuilt("aarch64", Some("abc"), &dirs).is_none());
    assert!(find_prebuilt("x86_64", None, &dirs).is_none());

    // 应用随附的包不核对快照，并且优先于缓存。
    std::fs::write(
        exe.join("engine").join(prebuilt_name("x86_64")),
        b"engine shipped",
    )
    .unwrap();
    let shipped = find_prebuilt("x86_64", Some("abc"), &dirs).unwrap();
    assert_ne!(shipped.id, cached.id);
    assert_eq!(
        shipped.tarball,
        exe.join("engine").join(prebuilt_name("x86_64"))
    );
    assert_eq!(bundled_source(&dirs), None);
    std::fs::write(exe.join("colm-src.tar.gz"), b"src").unwrap();
    assert_eq!(bundled_source(&dirs), Some(exe.join("colm-src.tar.gz")));
    // GUI 告诉的资源目录排在最前面。
    let res = dir.join("res");
    std::fs::create_dir_all(res.join("engine")).unwrap();
    std::fs::write(
        res.join("engine").join(prebuilt_name("x86_64")),
        b"engine from the gui dir",
    )
    .unwrap();
    std::env::set_var("COLM_RESOURCE_DIR", &res);
    let from_gui = find_prebuilt("x86_64", None, &resource_dirs(&exe)).unwrap();
    assert_eq!(
        from_gui.tarball,
        res.join("engine").join(prebuilt_name("x86_64"))
    );
    std::env::remove_var("COLM_RESOURCE_DIR");
    assert_eq!(find_prebuilt("x86_64", None, &dirs).unwrap().id, shipped.id);

    // 标识只看内容，不看文件名与时间。
    let copy = dir.join("copy.tar.gz");
    std::fs::write(&copy, b"engine shipped").unwrap();
    assert_eq!(Prebuilt::open(&copy).unwrap().id, shipped.id);
    std::env::remove_var("COLM_ENGINE_CACHE");
    let _ = std::fs::remove_dir_all(&dir);
}
