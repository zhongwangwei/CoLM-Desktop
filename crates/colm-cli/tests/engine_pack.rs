#![cfg(unix)]

use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use colm_remote::engine::{snapshot, Source};

#[test]
fn engine_pack_accepts_an_explicit_checkout_outside_the_binary_directory() {
    let root = std::env::temp_dir().join(format!(
        "colm-pack-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let checkout = root.join("source with spaces");
    fs::create_dir_all(checkout.join("crates/colm-cli/src")).unwrap();
    fs::write(checkout.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    fs::write(checkout.join("Cargo.lock"), "version = 3\n").unwrap();
    fs::write(
        checkout.join("crates/colm-cli/src/main.rs"),
        "fn main() {}\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .arg(&checkout)
        .status()
        .unwrap()
        .success());
    let binary = root.join("colm-cli");
    fs::copy(env!("CARGO_BIN_EXE_colm-cli"), &binary).unwrap();
    let archive = root.join("source.tar.gz");
    let output = Command::new(&binary)
        .env("COLM_AUDIT_BY_PARENT", "1")
        .arg("engine-pack")
        .arg("--source")
        .arg(&checkout)
        .arg("--out")
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        snapshot(&Source::Tarball(archive.clone())).unwrap().id,
        snapshot(&Source::Checkout(checkout)).unwrap().id
    );
    fs::remove_file(&archive).unwrap();
    for arguments in [vec![], vec!["--source", "missing-checkout"]] {
        let output = Command::new(&binary)
            .env("COLM_AUDIT_BY_PARENT", "1")
            .arg("engine-pack")
            .args(arguments)
            .arg("--out")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!archive.exists());
    }
    assert!(!Command::new(&binary)
        .env("COLM_AUDIT_BY_PARENT", "1")
        .arg("engine-pack")
        .arg("--source")
        .arg(&root)
        .arg("--out")
        .arg(&archive)
        .output()
        .unwrap()
        .status
        .success());
    assert!(!archive.exists());
    fs::remove_dir_all(root).unwrap();
}
