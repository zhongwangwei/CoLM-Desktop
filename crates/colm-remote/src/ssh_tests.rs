use super::*;

#[test]
fn hosts_are_checked_and_strings_quoted_for_the_remote_shell() {
    assert_eq!(Ssh::new(" 7920land ").unwrap().host, "7920land");
    assert!(Ssh::new("zhwei@172.16.100.17").is_ok());
    for bad in ["", "-oProxyCommand=evil", "a b"] {
        assert!(Ssh::new(bad).is_err(), "{bad}");
    }
    assert_eq!(quote("/media/data02/x y"), "'/media/data02/x y'");
    assert_eq!(quote("it's"), r"'it'\''s'");
}

#[test]
fn connection_failures_say_what_to_do() {
    let output = |code, stderr: &str| Output {
        success: false,
        code: Some(code),
        stdout: String::new(),
        stderr: stderr.into(),
    };
    let unreachable = connection_hint(
        "7920l",
        &output(
            255,
            "kex_exchange_identification: Connection closed by remote host",
        ),
    );
    assert!(
        unreachable.contains("cannot connect") && unreachable.contains("7920l"),
        "{unreachable}"
    );
    let password = connection_hint(
        "tianhe",
        &output(255, "Permission denied (publickey,keyboard-interactive)."),
    );
    assert!(
        password.contains("log in once in a terminal (ssh tianhe)"),
        "{password}"
    );
    let hostkey = connection_hint("x", &output(255, "Host key verification failed."));
    assert!(hostkey.contains("host key"), "{hostkey}");
    assert!(connection_hint("x", &output(1, "boom")).starts_with("the remote command failed"));
}

#[cfg(unix)]
#[test]
fn staged_source_upload_retries_partial_transfers_without_local_cat() {
    use crate::engine::{self, Snapshot, Source};
    use std::os::unix::fs::PermissionsExt;
    // Isolate PATH overrides from other tests and never contact a real SSH host.
    const CHILD: &str = "COLM_UPLOAD_STUB_CHILD";
    let root = std::env::temp_dir().join(format!("colm-upload-stub-{}", std::process::id()));
    if std::env::var_os(CHILD).is_none() {
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for (name, script) in [
            ("ssh", "#!/bin/bash\nexec /bin/bash -c \"${!#}\"\n"),
            (
                "cat",
                "#!/bin/bash\necho local-cat-forbidden >&2; exit 99\n",
            ),
            // Production Linux supplies flock; tests run sequentially on macOS too.
            ("flock", "#!/bin/bash\nexit 0\n"),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ssh::ssh_tests::staged_source_upload_retries_partial_transfers_without_local_cat",
                "--nocapture",
            ])
            .env(CHILD, &root)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os(CHILD).unwrap());
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("Cargo.lock"), "lock").unwrap();
    std::fs::write(source.join("complete.rs"), "complete").unwrap();
    let tarball = root.join("source.tar.gz");
    assert!(Command::new("tar")
        .args(["-czf"])
        .arg(&tarball)
        .arg("-C")
        .arg(&source)
        .arg(".")
        .status()
        .unwrap()
        .success());
    let server = root.join("server").to_string_lossy().into_owned();
    let ssh = Ssh::new("stub-only").unwrap();
    let snapshot = Snapshot {
        id: "test-snapshot".into(),
        source: Source::Tarball(tarball.clone()),
        files: vec![],
    };
    let dest = std::path::PathBuf::from(engine::engine_dir(&server, &snapshot.id)).join("src");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("Cargo.lock"), "partial transfer").unwrap();
    assert!(!engine::uploaded(&ssh, &server, &snapshot.id).unwrap());
    let valid = std::fs::read(&tarball).unwrap();
    std::fs::write(&tarball, &valid[..valid.len() / 2]).unwrap();
    assert!(engine::upload(&ssh, &server, &snapshot).is_err());
    assert!(!engine::uploaded(&ssh, &server, &snapshot.id).unwrap());
    assert_eq!(
        std::fs::read_to_string(dest.join("Cargo.lock")).unwrap(),
        "partial transfer"
    );
    std::fs::write(&tarball, valid).unwrap();
    assert!(engine::upload(&ssh, &server, &snapshot).unwrap());
    assert!(dest.join("complete.rs").is_file());
    assert!(engine::uploaded(&ssh, &server, &snapshot.id).unwrap());
    assert!(!engine::upload(&ssh, &server, &snapshot).unwrap());
}
