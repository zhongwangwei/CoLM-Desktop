use super::*;

#[test]
fn source_identity_tracks_fortran_include_content() {
    let root = std::env::temp_dir().join(format!(
        "colm-source-include-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let include = root.join("vendor/CoLM202X/include/pft_override_fields.inc");
    std::fs::create_dir_all(include.parent().unwrap()).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    let before = source_content_sha256(&root).unwrap();
    std::fs::write(&include, "real :: pft_parameter = 1.0\n").unwrap();
    let original = source_content_sha256(&root).unwrap();
    assert_ne!(original, before);
    std::fs::write(&include, "real :: pft_parameter = 2.0\n").unwrap();
    assert_ne!(source_content_sha256(&root).unwrap(), original);
    std::fs::write(&include, "real :: pft_parameter = 1.0\n").unwrap();
    assert_eq!(source_content_sha256(&root).unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
fn linked_source_fixture(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "colm-source-links-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for (path, bytes) in [
        ("Cargo.toml", "[workspace]\n"),
        ("Cargo.lock", "lock\n"),
        ("vendor/CoLM202X/Makefile", "include include/Makeoptions\n"),
        ("vendor/CoLM202X/include/Makeoptions.github", "FF=mpif90\n"),
        (
            "vendor/CoLM202X/include/Makeoptions.alternative",
            "FF=mpif90\n",
        ),
        (
            "vendor/CoLM202X/run/scripts/batch.github.config",
            "queue=short\n",
        ),
        (
            "vendor/CoLM202X/run/scripts/machine-github.config",
            "compiler=gnu\n",
        ),
        (".github/workflows/build.yml", "compiler: gnu\n"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    for (path, target) in [
        ("vendor/CoLM202X/include/Makeoptions", "Makeoptions.github"),
        (
            "vendor/CoLM202X/run/scripts/batch.config",
            "batch.github.config",
        ),
        (
            "vendor/CoLM202X/run/scripts/machine.config",
            "machine-github.config",
        ),
    ] {
        std::os::unix::fs::symlink(target, root.join(path)).unwrap();
    }
    root
}

#[cfg(unix)]
#[test]
fn real_colm_link_layout_has_one_identity_in_checkout_and_extracted_source() {
    let root = linked_source_fixture("archive");
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .status()
            .unwrap()
            .success())
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    let identity = source_content_sha256(&root).unwrap();
    std::fs::write(root.join(".gitignore"), "oracle/work/\n").unwrap();
    std::fs::create_dir_all(root.join("oracle/work")).unwrap();
    std::fs::write(root.join("oracle/work/stages.json"), "{\"generated\":true}").unwrap();
    assert_eq!(source_content_sha256(&root).unwrap(), identity);
    let snapshot = snapshot(&Source::Checkout(root.clone())).unwrap();
    let archive = root.join("source.tar.gz");
    pack_source(&snapshot, &archive).unwrap();
    assert_eq!(tarball_content_id(&archive).unwrap(), snapshot.id);
    let extracted = root.join("extracted");
    std::fs::create_dir(&extracted).unwrap();
    assert!(Command::new("tar")
        .args(["-xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(&extracted)
        .status()
        .unwrap()
        .success());
    assert_eq!(source_content_sha256(&extracted).unwrap(), identity);
    for path in [
        "vendor/CoLM202X/include/Makeoptions",
        "vendor/CoLM202X/run/scripts/batch.config",
        "vendor/CoLM202X/run/scripts/machine.config",
    ] {
        assert_eq!(
            std::fs::read_link(root.join(path)).unwrap(),
            std::fs::read_link(extracted.join(path)).unwrap()
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn compile_configuration_content_and_link_selection_both_change_source_identity() {
    let root = linked_source_fixture("changes");
    let before = source_content_sha256(&root).unwrap();
    let target = root.join("vendor/CoLM202X/include/Makeoptions.github");
    std::fs::write(&target, "FF=gfortran\n").unwrap();
    assert_ne!(source_content_sha256(&root).unwrap(), before);
    std::fs::write(target, "FF=mpif90\n").unwrap();
    assert_eq!(source_content_sha256(&root).unwrap(), before);
    let link = root.join("vendor/CoLM202X/include/Makeoptions");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("Makeoptions.alternative", &link).unwrap();
    assert_ne!(
        source_content_sha256(&root).unwrap(),
        before,
        "retargeting to equal bytes still changes compilation provenance"
    );
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("Makeoptions.github", &link).unwrap();
    std::fs::write(
        root.join(".github/workflows/build.yml"),
        "compiler: clang\n",
    )
    .unwrap();
    assert_ne!(
        source_content_sha256(&root).unwrap(),
        before,
        "build configuration is part of source identity"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn unsafe_source_links_fail_before_their_contents_can_enter_a_snapshot() {
    let root = linked_source_fixture("unsafe");
    let bad = root.join("vendor/CoLM202X/bad.rs");
    let outside = root.with_extension("outside.rs");
    std::fs::write(&outside, "external").unwrap();
    for target in [
        outside.clone(),
        PathBuf::from("../../../").join(outside.file_name().unwrap()),
        PathBuf::from("missing.rs"),
        PathBuf::from("include"),
        PathBuf::from("bad.rs"),
    ] {
        std::os::unix::fs::symlink(&target, &bad).unwrap();
        assert!(source_content_sha256(&root).is_err(), "{target:?}");
        assert!(content_id(&root, &["vendor/CoLM202X/bad.rs".into()]).is_err());
        std::fs::remove_file(&bad).unwrap();
    }
    let internal = root.join("vendor/CoLM202X/include/Makeoptions.github");
    std::os::unix::fs::symlink(internal, &bad).unwrap();
    assert!(
        source_content_sha256(&root).is_err(),
        "absolute links are rejected even if their target is internal"
    );
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_file(outside).unwrap();
}

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
    assert!(script.starts_with(crate::probe::BUILD_PATH_PRELUDE));
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

fn write_prebuilt_fixture(path: &Path, entries: &[(&str, &str)]) {
    let file = std::fs::File::create(path).unwrap();
    let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    for (name, text) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(text.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, name, text.as_bytes())
            .unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

#[test]
fn prebuilt_source_metadata_is_required_unambiguous_and_arch_specific() {
    let dir = std::env::temp_dir().join(format!("colm-pre-metadata-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("engine.tar.gz");
    for (entries, matches) in [
        (
            vec![("./ENGINE", "arch=x86_64\nsource=abc\nglibc=2.17\n")],
            true,
        ),
        (vec![("ENGINE", "arch=x86_64\nsource=other\n")], false),
        (vec![("ENGINE", "arch=aarch64\nsource=abc\n")], false),
        (vec![("bin/ENGINE", "arch=x86_64\nsource=abc\n")], false),
        (vec![("ENGINE", "arch=x86_64\n")], false),
        (vec![], false),
    ] {
        write_prebuilt_fixture(&path, &entries);
        assert_eq!(
            Prebuilt::open(&path)
                .unwrap()
                .matches_source("x86_64", "abc")
                .unwrap(),
            matches
        );
    }
    for entries in [
        vec![("ENGINE", "arch=x86_64\nsource=abc\nsource=abc\n")],
        vec![("ENGINE", "arch=x86_64\narch=x86_64\nsource=abc\n")],
        vec![
            ("ENGINE", "arch=x86_64\nsource=abc\n"),
            ("./ENGINE", "arch=x86_64\nsource=abc\n"),
        ],
    ] {
        write_prebuilt_fixture(&path, &entries);
        assert!(Prebuilt::open(&path)
            .unwrap()
            .matches_source("x86_64", "abc")
            .is_err());
    }
    write_prebuilt_fixture(&path, &[("ENGINE", &"a".repeat(4097))]);
    assert!(Prebuilt::open(&path)
        .unwrap()
        .matches_source("x86_64", "abc")
        .is_err());
    let file = std::fs::File::create(&path).unwrap();
    let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    archive
        .append_link(&mut header, "ENGINE", "outside-metadata")
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    assert!(Prebuilt::open(&path)
        .unwrap()
        .matches_source("x86_64", "abc")
        .is_err());
    std::fs::remove_dir_all(dir).unwrap();
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
    write_prebuilt_fixture(
        &cache.join(cached_name("x86_64", "abc")),
        &[("ENGINE", "arch=x86_64\nsource=abc\n")],
    );
    let cached = find_prebuilt("x86_64", Some("abc"), &dirs).unwrap();
    assert!(cached.id.starts_with("pre-") && cached.id.len() == 20);
    assert!(find_prebuilt("x86_64", Some("def"), &dirs).is_none());
    assert!(find_prebuilt("aarch64", Some("abc"), &dirs).is_none());
    assert!(find_prebuilt("x86_64", None, &dirs).is_none());

    // Invalid shipped candidates must not hide a valid cache entry.
    let shipped_path = exe.join("engine").join(prebuilt_name("x86_64"));
    for metadata in [
        "arch=x86_64\nsource=old\n",
        "arch=aarch64\nsource=abc\n",
        "",
    ] {
        write_prebuilt_fixture(&shipped_path, &[("ENGINE", metadata)]);
        assert_eq!(
            find_prebuilt("x86_64", Some("abc"), &dirs).unwrap().id,
            cached.id
        );
    }
    std::fs::write(&shipped_path, b"damaged archive").unwrap();
    assert_eq!(
        find_prebuilt("x86_64", Some("abc"), &dirs).unwrap().id,
        cached.id
    );
    // Matching shipped packages take priority over the cache.
    write_prebuilt_fixture(
        &shipped_path,
        &[("ENGINE", "arch=x86_64\nsource=abc\nbuilt=now\n")],
    );
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
    std::fs::copy(&shipped_path, &copy).unwrap();
    assert_eq!(Prebuilt::open(&copy).unwrap().id, shipped.id);
    std::env::remove_var("COLM_ENGINE_CACHE");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 只改文档：远程快照的标识不变（服务器不必重传、重编）；安装包的源码包里仍带着文档。
#[test]
fn docs_stay_out_of_the_snapshot_id_but_ship_in_the_source_pack() {
    let dir = std::env::temp_dir().join(format!("colm-remote-docs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, body) in [
        ("Cargo.toml", "[workspace]\n"),
        ("Cargo.lock", "lock"),
        ("crates/colm-cli/src/main.rs", "fn main() {}"),
        ("docs/guide.md", "first"),
    ] {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    let source = Source::Checkout(dir.clone());
    let before = snapshot(&source).unwrap();
    assert!(
        before.files.iter().all(|f| !f.starts_with("docs/")),
        "{:?}",
        before.files
    );
    std::fs::write(dir.join("docs/guide.md"), "second").unwrap();
    assert_eq!(
        snapshot(&source).unwrap().id,
        before.id,
        "a docs edit must not change the remote engine"
    );
    std::fs::write(dir.join("crates/colm-cli/src/main.rs"), "fn main() { }").unwrap();
    assert_ne!(
        snapshot(&source).unwrap().id,
        before.id,
        "a source edit must change it"
    );
    let pack = dir.join("pack.tar.gz");
    let checkout = snapshot(&source).unwrap();
    pack_source(&checkout, &pack).unwrap();
    let listing = Command::new("tar").arg("-tzf").arg(&pack).output().unwrap();
    let listing = String::from_utf8_lossy(&listing.stdout);
    assert!(listing.lines().any(|l| l == "docs/guide.md"), "{listing}");
    assert!(
        listing.lines().any(|l| l == "crates/colm-cli/src/main.rs"),
        "{listing}"
    );
    // 安装版（读源码包）与开发版（读仓库）对同一份源码给出同一个标识。
    let installed = |path: &Path| snapshot(&Source::Tarball(path.to_path_buf())).unwrap().id;
    assert_eq!(installed(&pack), checkout.id);
    // 重新打包（压缩包里的修改时间等元数据不同）：标识不变。
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
    let again = dir.join("again.tar.gz");
    pack_source(&snapshot(&source).unwrap(), &again).unwrap();
    assert_ne!(
        std::fs::read(&pack).unwrap(),
        std::fs::read(&again).unwrap(),
        "the archives themselves differ"
    );
    assert_eq!(installed(&again), checkout.id);
    // 只改文档的安装包：标识也不变。
    std::fs::write(dir.join("docs/guide.md"), "third").unwrap();
    let docs = dir.join("docs.tar.gz");
    pack_source(&snapshot(&source).unwrap(), &docs).unwrap();
    assert_eq!(installed(&docs), checkout.id);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run `ensure_script` with a `cargo` that records its arguments, and return them.
#[cfg(unix)]
fn cargo_arguments_of_a_build(root: &Path, vendor: bool) -> String {
    let root_s = root.to_str().unwrap();
    let src = root.join("engine/abc/src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("Cargo.lock"), "version = 4\n").unwrap();
    let key = crate::offline::lock_key("version = 4\n");
    if vendor {
        let dir = crate::offline::vendor_dir(root_s, &key);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Path::new(&dir).join(".complete"), "").unwrap();
    }
    let release = root.join("target/release");
    std::fs::create_dir_all(&release).unwrap();
    for (_, binary) in BINARIES {
        std::fs::write(release.join(binary), "").unwrap();
    }
    let mocks = format!(
        "flock() {{ :; }}\nrustc() {{ :; }}\ncmake() {{ echo 'cmake version 4.4.4'; }}\n\
         cargo() {{ [ \"$1\" = --version ] || printf '%s\\n' \"$@\" > {}; }}\ncd {}\n",
        crate::ssh::quote(&format!("{root_s}/arguments")),
        crate::ssh::quote(root_s)
    );
    let output = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "set -e\n{mocks}{}",
            ensure_script(root_s, "abc", 4)
        ))
        .env("HOME", root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read_to_string(root.join("arguments")).unwrap()
}

#[cfg(unix)]
#[test]
fn a_vendor_directory_for_the_snapshot_lock_turns_the_build_offline() {
    let base = std::env::temp_dir().join(format!("colm-offline-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    // A root with a space and a double quote: the path crosses both shell and TOML quoting.
    let root = base.join("work \"root\"");
    let online = cargo_arguments_of_a_build(&base.join("plain"), false);
    assert!(!online.contains("--offline"), "{online}");
    let offline = cargo_arguments_of_a_build(&root, true);
    let lines: Vec<&str> = offline.lines().collect();
    let at = lines
        .iter()
        .position(|l| *l == "--offline")
        .expect(&offline);
    assert_eq!(lines[at + 1], "--config");
    assert_eq!(
        lines[at + 2],
        "source.crates-io.replace-with=\"colm-vendor\""
    );
    assert_eq!(lines[at + 3], "--config");
    let key = crate::offline::lock_key("version = 4\n");
    let toml = format!(
        "source.colm-vendor.directory=\"{}\"",
        crate::offline::vendor_dir(root.to_str().unwrap(), &key).replace('"', "\\\"")
    );
    assert_eq!(lines[at + 4], toml);
    assert!(root.join("engine/abc/bin/colm-rs").is_file());
    std::fs::remove_dir_all(base).unwrap();
}
