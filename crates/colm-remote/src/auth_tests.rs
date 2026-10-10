use super::*;

fn auth(mode: &str) -> Auth {
    Auth {
        host: "server".into(),
        username: "researcher".into(),
        port: 2222,
        auth: mode.into(),
        identity_file: String::new(),
        password: None,
        broker: None,
    }
}

#[test]
fn credentials_are_validated_without_echoing_input() {
    let mut a = auth("password");
    assert!(a.validate().is_err());
    a.password = Some("test secret".into());
    assert!(a.validate().is_ok());
    a.auth = "key".into();
    assert!(a.validate().is_err());
    a.password = None;
    a.identity_file = "relative/key".into();
    assert!(a.validate().is_err());
    a.identity_file = std::env::temp_dir()
        .join("key")
        .to_string_lossy()
        .into_owned();
    assert!(a.validate().is_ok());
    a.username = "-malicious".into();
    assert!(a.validate().is_err());
    let error = parse(br#"{"password":"test secret","auth":false}"#.as_slice())
        .err()
        .unwrap();
    assert!(!format!("{error:#}").contains("test secret"));
    assert!(parse(vec![b'x'; LIMIT as usize + 1].as_slice()).is_err());
}

#[test]
fn explicit_auth_sets_identity_without_password_in_command() {
    let mut a = auth("password");
    a.password = Some("test secret".into());
    a.broker = Some(("127.0.0.1:1234".into(), "a".repeat(64)));
    let mut command = Command::new("ssh");
    a.apply(&mut command).unwrap();
    let args: Vec<_> = command.get_args().map(|v| v.to_string_lossy()).collect();
    for expected in [
        "BatchMode=no",
        "ControlPath=none",
        "PreferredAuthentications=password",
        "KbdInteractiveAuthentication=no",
        "researcher",
        "2222",
    ] {
        assert!(args.iter().any(|arg| arg == expected), "{expected}");
    }
    assert!(!format!("{command:?}").contains("test secret"));
    let a = auth("key");
    let mut command = Command::new("ssh");
    a.apply(&mut command).unwrap();
    assert!(command.get_args().any(|a| a == "BatchMode=yes"));
    assert!(command.get_envs().all(|(key, _)| key != TOKEN));
}

#[test]
fn broker_requires_random_token_and_keeps_password_in_memory() {
    let (address, token) =
        start_broker("test secret".into(), "user@server's password: ".into()).unwrap();
    assert_eq!(token.len(), 64);
    assert_ne!(token, random_token().unwrap());
    let mut invalid = TcpStream::connect(&address).unwrap();
    invalid.write_all(&[b'!'; 64]).unwrap();
    let mut reply = String::new();
    invalid.read_to_string(&mut reply).unwrap();
    assert!(reply.is_empty());
    let mut wrong_prompt = TcpStream::connect(&address).unwrap();
    wrong_prompt.write_all(token.as_bytes()).unwrap();
    wrong_prompt
        .write_all(b"user@jump-host's password: \n")
        .unwrap();
    wrong_prompt.read_to_string(&mut reply).unwrap();
    assert!(reply.is_empty());
    let mut valid = TcpStream::connect(&address).unwrap();
    valid.write_all(token.as_bytes()).unwrap();
    valid.write_all(b"user@server's password: \n").unwrap();
    valid.read_to_string(&mut reply).unwrap();
    assert_eq!(reply, "test secret");
}

#[test]
fn initialized_credentials_are_bound_to_host_and_job_connection() {
    const CHILD: &str = "COLM_AUTH_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "auth::tests::initialized_credentials_are_bound_to_host_and_job_connection",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("COLM_SSH_AUTH_STDIN", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"{\"host\":\"server\",\"username\":\"user\",\"port\":2222,\"auth\":\"key\",\"password\":null}\n").unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    initialize_from_stdin().unwrap();
    assert!(crate::Ssh::new("server").is_ok());
    assert!(crate::Ssh::new("another-server").is_err());
    let binding = current_connection().unwrap();
    assert!(check_connection(Some(&binding)).is_ok());
    let mut changed = binding.clone();
    changed.port = 22;
    assert!(check_connection(Some(&changed)).is_err());
    assert!(!serde_json::to_string(&binding)
        .unwrap()
        .contains("password"));
    assert!(check_connection(None).is_ok());
}

#[test]
fn recorded_key_and_config_connections_resume_without_gui_credentials() {
    const CHILD: &str = "COLM_AUTH_RESTORE_CHILD";
    let Ok(mode) = std::env::var(CHILD) else {
        for mode in ["key", "config", "password"] {
            let result = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "auth::tests::recorded_key_and_config_connections_resume_without_gui_credentials", "--nocapture"])
                .env(CHILD, mode).env_remove("COLM_SSH_AUTH_STDIN")
                .output().unwrap();
            assert!(
                result.status.success(),
                "{} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        return;
    };
    let binding = Connection {
        host: "server".into(),
        username: "user".into(),
        port: 2222,
        auth: mode.clone(),
        identity_file: String::new(),
    };
    if mode == "password" {
        assert!(check_connection(Some(&binding)).is_err());
        assert!(current_connection().is_none());
    } else {
        check_connection(Some(&binding)).unwrap();
        assert_eq!(current_connection().as_ref(), Some(&binding));
        assert!(crate::Ssh::new("server").is_ok());
        assert!(crate::Ssh::new("other").is_err());
    }
}

#[test]
fn password_prompt_uses_openssh_host_key_alias_when_configured() {
    for (config, expected) in [
        (
            "user scientist\nhostname 192.0.2.1\n",
            "scientist@192.0.2.1's password: ",
        ),
        (
            "user scientist\nhostname 192.0.2.1\nhostkeyalias known-server\n",
            "scientist@known-server's password: ",
        ),
        (
            "user scientist\nhostname 192.0.2.1\nhostkeyalias none\n",
            "scientist@192.0.2.1's password: ",
        ),
        (
            "user scientist\nhostname 192.0.2.1\nhostkeyalias \n",
            "scientist@192.0.2.1's password: ",
        ),
    ] {
        assert_eq!(prompt_from_config(config).unwrap(), expected);
    }
    assert!(prompt_from_config("hostname 192.0.2.1\n").is_err());
}

#[cfg(unix)]
#[test]
fn password_redaction_preserves_protocol_stdout() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "COLM_AUTH_STDOUT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let root = std::env::temp_dir().join(format!("colm-auth-stdout-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let ssh = root.join("ssh");
        std::fs::write(&ssh, "#!/bin/sh\nexec /bin/sh\n").unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "auth::tests::password_redaction_preserves_protocol_stdout",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("PATH", &root)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let mut auth = auth("password");
    auth.password = Some("1".into());
    auth.broker = Some(("127.0.0.1:1".into(), "a".repeat(64)));
    assert!(AUTH.set(auth).is_ok());
    let ssh = crate::Ssh::new("server").unwrap();
    let output = ssh
        .run("printf 'job=123\\n'; printf 'password=1\\n' >&2\n")
        .unwrap();
    assert!(output.success);
    assert_eq!(output.stdout, "job=123\n");
    assert_eq!(output.stderr, "password=[redacted]\n");
    let failure = ssh.run("printf 'password=1\\n' >&2; exit 1\n").unwrap();
    assert!(!failure.success);
    assert_eq!(failure.stderr, "password=[redacted]\n");
    let status = crate::job::parse_status("state=finished 1\nphase=run\n---log---\npassword=1\n");
    assert_eq!(status.state, crate::job::State::Finished { exit_code: 1 });
    assert_eq!(status.log_tail, "password=[redacted]\n");
}
