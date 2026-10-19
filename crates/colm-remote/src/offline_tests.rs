use super::*;

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-offline-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const SUM_A: &str = "49f1f14873335454500d59611f1cf4a4b0f786f9ac11f4312a78e4cf2566695b";

#[test]
fn the_lock_file_names_its_registry_packages_and_skips_workspace_members() {
    let lock = format!(
        "version = 4\n\n[[package]]\nname = \"colm-cli\"\nversion = \"0.1.0\"\ndependencies = [\n \"itoa\",\n]\n\n\
         [[package]]\nname = \"itoa\"\nversion = \"1.0.11\"\nsource = \"{CRATES_IO}\"\nchecksum = \"{}\"\n",
        SUM_A.to_ascii_uppercase()
    );
    assert_eq!(
        crates_in_lock(&lock).unwrap(),
        vec![Crate {
            name: "itoa".into(),
            version: "1.0.11".into(),
            checksum: SUM_A.into(),
        }]
    );
}

#[test]
fn git_sources_missing_checksums_and_odd_names_are_refused() {
    let git = "[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nsource = \"git+https://example.org/x#abc\"\n";
    assert!(crates_in_lock(git)
        .unwrap_err()
        .to_string()
        .contains("cannot be vendored"));
    let unchecked =
        format!("[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nsource = \"{CRATES_IO}\"\n");
    assert!(crates_in_lock(&unchecked)
        .unwrap_err()
        .to_string()
        .contains("no checksum"));
    let short = format!(
        "[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nsource = \"{CRATES_IO}\"\nchecksum = \"abc\"\n"
    );
    assert!(crates_in_lock(&short).is_err());
    // The name becomes a URL and a path on the server.
    let escape = format!(
        "[[package]]\nname = \"../x\"\nversion = \"1.0.0\"\nsource = \"{CRATES_IO}\"\nchecksum = \"{SUM_A}\"\n"
    );
    assert!(crates_in_lock(&escape).is_err());
}

#[test]
fn the_repository_lock_file_can_be_vendored() {
    let lock =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .unwrap();
    let crates = crates_in_lock(&lock).unwrap();
    assert_eq!(
        crates.len(),
        lock.matches("\nsource = \"registry+").count(),
        "every registry package is listed once"
    );
    assert!(crates.iter().any(|c| c.name == "ureq"));
    assert_eq!(lock_key(&lock).len(), 16);
    assert_eq!(lock_key(&lock), lock_key(&lock));
    assert_ne!(lock_key(&lock), lock_key(&format!("{lock}\n")));
}

#[test]
fn downloads_are_the_pinned_official_archives() {
    let rust = rust_downloads("x86_64").unwrap();
    assert_eq!(
        rust.iter().map(|d| d.file.as_str()).collect::<Vec<_>>(),
        [
            "rustc-1.97.1-x86_64-unknown-linux-gnu.tar.gz",
            "cargo-1.97.1-x86_64-unknown-linux-gnu.tar.gz",
            "rust-std-1.97.1-x86_64-unknown-linux-gnu.tar.gz",
        ]
    );
    assert!(rust
        .iter()
        .all(|d| d.url == format!("https://static.rust-lang.org/dist/{}", d.file)));
    let arm = rust_downloads("aarch64").unwrap();
    assert!(arm.iter().zip(&rust).all(|(a, x)| a.sha256 != x.sha256));
    // The same archive and checksum the online installer pins.
    for arch in ["x86_64", "aarch64"] {
        let cmake = cmake_download(arch).unwrap();
        assert!(crate::toolchain::install_script().contains(cmake.sha256));
        assert!(cmake
            .url
            .ends_with(&format!("cmake-4.4.4-linux-{arch}.tar.gz")));
    }
    assert!(rust_downloads("ppc64le").is_err());
    assert!(cmake_download("arm64").is_err());
}

fn write_crate(path: &Path, entries: &[(&str, &str)]) {
    let gzip = flate2::write::GzEncoder::new(
        std::fs::File::create(path).unwrap(),
        flate2::Compression::default(),
    );
    let mut builder = tar::Builder::new(gzip);
    for (name, text) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(text.len() as u64);
        header.set_mode(if name.ends_with(".sh") { 0o755 } else { 0o644 });
        header.set_cksum();
        builder
            .append_data(&mut header, name, text.as_bytes())
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

fn read_tarball(path: &Path) -> BTreeMap<String, (String, u32)> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(
        std::fs::File::open(path).unwrap(),
    ));
    archive
        .entries()
        .unwrap()
        .map(|entry| {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().display().to_string();
            let mode = entry.header().mode().unwrap();
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            (name, (text, mode))
        })
        .collect()
}

fn lock_for(crates: &[(&str, &str, &Path)]) -> String {
    crates
        .iter()
        .map(|(name, version, file)| {
            format!(
                "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"{CRATES_IO}\"\nchecksum = \"{}\"\n\n",
                sha256_file(file).unwrap()
            )
        })
        .collect()
}

#[test]
fn verified_crates_become_a_directory_source_without_downloading() {
    let cache = scratch("vendor");
    let crates = cache.join("crates");
    std::fs::create_dir_all(&crates).unwrap();
    let a = crates.join("alpha-1.0.0.crate");
    write_crate(
        &a,
        &[
            ("alpha-1.0.0/Cargo.toml", "[package]\nname = \"alpha\"\n"),
            ("alpha-1.0.0/src/lib.rs", "pub fn a() {}\n"),
            ("alpha-1.0.0/build.sh", "#!/bin/sh\n"),
            // Only a re-pack on macOS adds these; build scripts that glob a directory choke on them.
            ("alpha-1.0.0/src/._lib.rs", "apple double"),
        ],
    );
    let b = crates.join("beta-0.2.1.crate");
    write_crate(
        &b,
        &[("beta-0.2.1/Cargo.toml", "[package]\nname = \"beta\"\n")],
    );
    let lock = lock_for(&[("alpha", "1.0.0", &a), ("beta", "0.2.1", &b)]);

    // Already-verified files in the cache mean no network: the URLs here would fail.
    let tarball = vendor_tarball_in(&cache, &lock, &|_| {}).unwrap();
    assert_eq!(
        tarball.file_name().unwrap().to_str().unwrap(),
        format!("vendor-{}.tar.gz", lock_key(&lock))
    );
    let files = read_tarball(&tarball);
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "alpha-1.0.0/.cargo-checksum.json",
            "alpha-1.0.0/Cargo.toml",
            "alpha-1.0.0/build.sh",
            "alpha-1.0.0/src/lib.rs",
            "beta-0.2.1/.cargo-checksum.json",
            "beta-0.2.1/Cargo.toml",
        ]
    );
    // An empty file list is accepted by cargo; the package checksum is still checked against the
    // lock file (2026-10-19, cargo 1.97.1, an altered checksum is refused).
    assert_eq!(
        files["alpha-1.0.0/.cargo-checksum.json"].0,
        format!(
            "{{\"files\":{{}},\"package\":\"{}\"}}",
            sha256_file(&a).unwrap()
        )
    );
    assert_eq!(files["alpha-1.0.0/build.sh"].1 & 0o777, 0o755);
    // A second call reuses the tarball.
    let again = vendor_tarball_in(&cache, &lock, &|_| panic!("rebuilt")).unwrap();
    assert_eq!(again, tarball);
    std::fs::remove_dir_all(cache).unwrap();
}

#[test]
fn a_crate_that_writes_outside_its_own_directory_is_refused() {
    let cache = scratch("escape");
    let crates = cache.join("crates");
    std::fs::create_dir_all(&crates).unwrap();
    let a = crates.join("alpha-1.0.0.crate");
    write_crate(
        &a,
        &[
            ("alpha-1.0.0/Cargo.toml", ""),
            ("beta-0.2.1/Cargo.toml", "planted"),
        ],
    );
    let lock = lock_for(&[("alpha", "1.0.0", &a)]);
    let error = vendor_tarball_in(&cache, &lock, &|_| {}).unwrap_err();
    assert!(error.to_string().contains("unexpected path"), "{error}");
    assert!(!cache
        .join(format!("vendor-{}.tar.gz", lock_key(&lock)))
        .exists());
    std::fs::remove_dir_all(cache).unwrap();
}

#[test]
fn a_cached_download_with_the_wrong_checksum_is_not_used() {
    let dir = scratch("fetch");
    let dest = dir.join("x.tar.gz");
    std::fs::write(&dest, "tampered").unwrap();
    // The cached file does not match, so this tries the network, and an unroutable URL fails.
    let error = fetch("http://127.0.0.1:9/x.tar.gz", SUM_A, &dest).unwrap_err();
    assert!(error.to_string().contains("cannot download"), "{error}");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "tampered");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "no part file left"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_server_state_is_read_from_its_report() {
    let state =
        parse_state("home=/home/u\narch=x86_64\nrust=1\ncmake=\nvendor=\nonline=no\n").unwrap();
    assert_eq!(
        state,
        State {
            home: "/home/u".into(),
            arch: "x86_64".into(),
            present: Present {
                rust: true,
                cmake: false,
                vendor: false,
            },
            online: false,
        }
    );
    assert!(
        parse_state("home=/home/u\narch=x86_64\nonline=yes\n")
            .unwrap()
            .online
    );
    assert!(parse_state("arch=x86_64\n").is_err());
    assert!(parse_state("home=relative\n").is_err());
}

#[test]
fn network_failures_are_told_apart_from_compile_errors() {
    for message in [
        "error: failed to download from `https://static.crates.io/crates/ureq/3.4.2/download`",
        "Caused by: [28] Timeout was reached (Operation timed out after 30000 milliseconds)",
        "Toolchain download failed. Check server HTTPS access or install Rust/CMake manually",
        "warning: spurious network error (3 tries remaining): [6] Could not resolve host: index.crates.io",
    ] {
        assert!(looks_like_network_failure(message), "{message}");
    }
    for message in [
        "error[E0308]: mismatched types",
        "error: could not compile `colm-core` (lib) due to 1 previous error",
        "CMake Error at CMakeLists.txt:1",
    ] {
        assert!(!looks_like_network_failure(message), "{message}");
    }
}

#[cfg(unix)]
mod shell {
    use super::*;

    fn run(script: &str, home: &Path) -> std::process::Output {
        std::process::Command::new("bash")
            .arg("-c")
            .arg(script)
            .env("HOME", home)
            .output()
            .unwrap()
    }

    #[test]
    fn the_state_report_stops_early_for_a_built_engine_and_skips_the_network_with_a_vendor_dir() {
        let root = scratch("state");
        let root_s = root.to_str().unwrap();
        let key = "0123456789abcdef";
        let engine = crate::engine::engine_dir(root_s, "abc");
        // `curl` that records being called: the network check must not run once the vendor is there.
        let mock = format!(
            "curl() {{ echo called >> {}; return 1; }}\n",
            quote(&format!("{root_s}/curl-calls"))
        );
        let report = |engine: Option<&str>| {
            let output = run(
                &format!("{mock}{}", state_script(root_s, key, engine)),
                &root,
            );
            String::from_utf8(output.stdout).unwrap()
        };
        let text = report(Some(&engine));
        assert!(text.contains("vendor=\n"), "{text}");
        assert!(text.contains("online=no"), "{text}");
        assert!(root.join("curl-calls").exists());

        std::fs::remove_file(root.join("curl-calls")).unwrap();
        std::fs::create_dir_all(vendor_dir(root_s, key)).unwrap();
        std::fs::write(Path::new(&vendor_dir(root_s, key)).join(".complete"), "").unwrap();
        let text = report(Some(&engine));
        assert!(text.contains("vendor=1"), "{text}");
        assert!(!text.contains("online="), "{text}");
        assert!(!root.join("curl-calls").exists());

        std::fs::create_dir_all(format!("{engine}/bin")).unwrap();
        let binary = format!("{engine}/bin/colm-rs");
        std::fs::write(&binary, "").unwrap();
        std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        assert_eq!(report(Some(&engine)), "built=1\n");
        assert!(report(None).contains("home="));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_finished_vendor_directory_is_never_replaced() {
        let root = scratch("publish");
        let dest = root.join("offline/vendor-k");
        let stage = root.join("offline/vendor-k.upload.1");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("new"), "").unwrap();
        // macOS has no flock(1); the lock itself is exercised on Linux servers.
        let script = |stage: &Path| {
            format!(
                "flock() {{ :; }}\n{}",
                publish_vendor_script(stage.to_str().unwrap(), dest.to_str().unwrap())
            )
        };
        assert!(run(&script(&stage), &root).status.success());
        assert!(dest.join(".complete").is_file() && dest.join("new").is_file());
        assert!(!stage.exists());

        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("other"), "").unwrap();
        assert!(run(&script(&stage), &root).status.success());
        assert!(!dest.join("other").exists(), "published copy kept");
        assert!(!stage.exists(), "stage removed");
        std::fs::remove_dir_all(root).unwrap();
    }
}
