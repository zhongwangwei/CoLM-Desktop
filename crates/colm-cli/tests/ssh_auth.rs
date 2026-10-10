#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use serde_json::json;

#[test]
fn packaged_cli_serves_only_the_target_password_without_exposing_it() {
    let dir = std::env::temp_dir().join(format!("colm-cli-ssh-auth-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let stub = dir.join("ssh");
    std::fs::write(&stub, r#"#!/bin/bash
set -eu
if [ "$1" = '-G' ]; then
  printf 'user researcher\nhostname server\nhostkeyalias trusted-target\n'
  exit 0
fi
[[ "$*" != *'test-only secret'* ]]
! env | /usr/bin/grep -q 'test-only secret'
[[ "$*" == *'StrictHostKeyChecking=yes'* ]]
if [[ "$*" == *'BatchMode=no'* ]]; then
  [[ "$*" == *'ControlPath=none'* ]]
  [[ "$SSH_ASKPASS_REQUIRE" = force ]]
  if "$SSH_ASKPASS" "researcher@jump-host's password: "; then exit 91; fi
  password=$("$SSH_ASKPASS" "researcher@trusted-target's password: ")
  [[ "$password" = 'test-only secret' ]]
  if [[ "$*" == *'fail-only'* ]]; then
    printf '%s\n' "$password" >&2
    exit 255
  fi
else
  [[ "$*" == *'BatchMode=yes'* ]]
  [[ -z "${COLM_SSH_ASKPASS_TOKEN:-}" ]]
fi
cat >/dev/null
printf 'hostname=stub\nos=Linux\narch=x86_64\ncargo=cargo\ncmake=cmake\ncc=cc\nroot_exists=1\nroot_writable=1\n'
"#).unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let call = |mode: &str, host: &str| {
        let payload = json!({"host":host,"username":"researcher","port":2222,
            "auth":mode,"identity_file":"","password":if mode == "password" { Some("test-only secret") } else { None }});
        let mut child = Command::new(env!("CARGO_BIN_EXE_colm-cli"))
            .args(["remote-probe", "--host", host, "--root", "/data/colm"])
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("COLM_SSH_AUTH_STDIN", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
        child.wait_with_output().unwrap()
    };
    for mode in ["config", "key", "password"] {
        let output = call(mode, "server");
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(answer["hostname"], "stub");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("test-only secret"));
    }
    let failed = call("password", "fail-only");
    assert!(!failed.status.success());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("test-only secret"));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("[redacted]"));
    std::fs::remove_dir_all(dir).unwrap();
}
