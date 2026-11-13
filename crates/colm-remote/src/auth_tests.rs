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
        "StrictHostKeyChecking=yes",
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
    assert!(command.get_args().any(|a| a == "StrictHostKeyChecking=yes"));
    assert!(command.get_envs().all(|(key, _)| key != TOKEN));
}

#[test]
fn config_mode_preserves_host_key_policy_from_openssh_config() {
    let mut command = Command::new("ssh");
    auth("config").apply(&mut command).unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect();
    assert!(args.iter().any(|arg| arg == "BatchMode=yes"));
    for setting in ["StrictHostKeyChecking", "UserKnownHostsFile", "ProxyJump"] {
        assert!(
            !args.iter().any(|arg| arg.starts_with(setting)),
            "{setting}"
        );
    }
    let mut fallback = Command::new("ssh");
    configure(&mut fallback).unwrap();
    assert!(fallback
        .get_args()
        .any(|arg| arg == "StrictHostKeyChecking=yes"));
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
        for mode in ["key", "config", "password", "key-password"] {
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
    if matches!(mode.as_str(), "password" | "key-password") {
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

#[test]
fn gui_relay_forwards_prompts_waits_for_humans_and_redacts_answers() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let gui_address = listener.local_addr().unwrap().to_string();
    let gui_token = "b".repeat(64);
    let prompts = [
        "Enter passphrase for key '/tmp/encrypted': ",
        "jump@host's password: ",
        "Password: ",
    ];
    let server = std::thread::spawn(move || {
        for (index, prompt) in prompts.iter().enumerate() {
            let (mut stream, _) = listener.accept().unwrap();
            let mut token = [0; 64];
            stream.read_exact(&mut token).unwrap();
            assert_eq!(&token, gui_token.as_bytes());
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            assert_eq!(line, format!("{prompt}\n"));
            if index == 0 {
                std::thread::sleep(Duration::from_millis(3200));
                stream.write_all(b"relay-test-passphrase").unwrap();
            } else if index == 1 {
                stream.write_all(b"relay-test-password").unwrap();
            } // The final request is cancelled by EOF.
        }
    });
    let (address, token) = start_gui_relay(gui_address, "b".repeat(64)).unwrap();
    assert!(request_secret(&address, &"0".repeat(64), prompts[0]).is_err());
    assert!(request_secret(
        &address,
        &token,
        "Are you sure you want to continue connecting (yes/no)? "
    )
    .is_err());
    assert!(request_secret(&address, &token, "Password:\n").is_err());
    assert_eq!(
        request_secret(&address, &token, prompts[0]).unwrap(),
        b"relay-test-passphrase"
    );
    assert_eq!(
        request_secret(&address, &token, prompts[1]).unwrap(),
        b"relay-test-password"
    );
    assert!(request_secret(&address, &token, prompts[2]).is_err());
    server.join().unwrap();
    assert_eq!(
        redact("relay-test-passphrase relay-test-password"),
        "[redacted] [redacted]"
    );
    assert!(validate_endpoint("192.0.2.1:1234", &token).is_err());
    assert!(validate_endpoint("127.0.0.1:1234", "bad").is_err());
}

#[test]
fn combined_mode_keeps_identity_and_password_negotiation() {
    let mut a = auth("key-password");
    assert!(a.validate().is_err());
    a.password = Some("combined secret".into());
    a.identity_file = std::env::temp_dir()
        .join("identity")
        .to_string_lossy()
        .into_owned();
    a.broker = Some(("127.0.0.1:1234".into(), "a".repeat(64)));
    a.validate().unwrap();
    let mut command = Command::new("ssh");
    a.apply(&mut command).unwrap();
    let args: Vec<_> = command.get_args().collect();
    assert!(args
        .iter()
        .any(|a| *a == "PreferredAuthentications=publickey,password,keyboard-interactive"));
    assert!(args.iter().any(|a| *a == "IdentitiesOnly=yes"));
    assert!(args.iter().any(|a| *a == "BatchMode=no"));
    assert!(!format!("{command:?}").contains("combined secret"));
}

#[test]
fn gui_initialization_supports_no_stdin_and_missing_password() {
    const CHILD: &str = "COLM_AUTH_GUI_CHILD";
    let Ok(mode) = std::env::var(CHILD) else {
        for mode in ["none", "config", "key", "password", "key-password"] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "auth::tests::gui_initialization_supports_no_stdin_and_missing_password",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .env(GUI_BROKER, "127.0.0.1:1234")
                .env(GUI_TOKEN, "a".repeat(64))
                .env(
                    "COLM_SSH_AUTH_STDIN",
                    if mode == "none" { "0" } else { "1" },
                )
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("{{\"host\":\"server\",\"auth\":\"{mode}\"}}\n").as_bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        return;
    };
    initialize_from_stdin().unwrap();
    assert_eq!(current_connection().is_none(), mode == "none");
    let mut command = Command::new("ssh");
    configure(&mut command).unwrap();
    assert!(command.get_args().any(|arg| arg == "BatchMode=no"));
    assert!(command
        .get_args()
        .any(|arg| arg == "StrictHostKeyChecking=yes"));
    assert!(!command
        .get_args()
        .any(|arg| arg == "NumberOfPasswordPrompts=1"));
    for key in [GUI_BROKER, GUI_TOKEN] {
        assert!(command
            .get_envs()
            .any(|(name, value)| name == key && value.is_none()));
    }
    assert!(command
        .get_envs()
        .any(|(name, value)| name == TOKEN && value.unwrap() != "a".repeat(64).as_str()));
}

#[test]
fn gui_relay_rejects_untrusted_prompts_and_handles_concurrent_requests() {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (relay_address, token) = start_gui_relay(address, "c".repeat(64)).unwrap();
    for prompt in [
        "Are you sure you want to continue connecting (yes/no)? ",
        "Password:\r",
        "Enter passphrase for key 'bad\0':",
    ] {
        let mut stream = TcpStream::connect(&relay_address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(token.as_bytes()).unwrap();
        writeln!(stream, "{prompt}").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.is_empty());
    }
    let server = std::thread::spawn(move || {
        let mut streams = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut supplied = [0; 64];
            stream.read_exact(&mut supplied).unwrap();
            let mut prompt = String::new();
            BufReader::new(&mut stream).read_line(&mut prompt).unwrap();
            assert_eq!(prompt, "Password: \n");
            streams.push(stream);
        }
        for mut stream in streams {
            stream.write_all(b"concurrent-test-secret").unwrap();
        }
    });
    let requests: Vec<_> = (0..2)
        .map(|_| {
            let (address, token) = (relay_address.clone(), token.clone());
            std::thread::spawn(move || request_secret(&address, &token, "Password: ").unwrap())
        })
        .collect();
    for request in requests {
        assert_eq!(request.join().unwrap(), b"concurrent-test-secret");
    }
    server.join().unwrap();
}

#[cfg(unix)]
#[test]
fn gui_registered_password_is_target_bound_and_used_only_once() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "COLM_AUTH_GUI_PREFILL_CHILD";
    let mode = std::env::var(CHILD);
    if mode.is_err() {
        for mode in ["direct", "proxyjump", "proxycommand"] {
            let root =
                std::env::temp_dir().join(format!("colm-auth-prefill-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            let ssh = root.join("ssh");
            std::fs::write(
                &ssh,
                &format!(
                    "#!/bin/sh\nprintf 'user scientist\\nhostname target\\n{}'\n",
                    if mode == "direct" {
                        "".to_owned()
                    } else {
                        format!("{mode} bridge\\n")
                    }
                ),
            )
            .unwrap();
            std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let server = std::thread::spawn(move || {
                let mut prompts = vec![
                    ("jump@host's password: ", "jump secret"),
                    ("Enter passphrase for key '/tmp/key': ", "key secret"),
                    ("scientist@target's password: ", "corrected target secret"),
                ];
                if mode != "direct" {
                    prompts.insert(
                        2,
                        (
                            "scientist@target's password: ",
                            "GUI target-shaped jump reply",
                        ),
                    );
                }
                for (expected, answer) in prompts {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut token = [0; 64];
                    stream.read_exact(&mut token).unwrap();
                    assert_eq!(token, [b'd'; 64]);
                    let mut prompt = String::new();
                    BufReader::new(&mut stream).read_line(&mut prompt).unwrap();
                    assert_eq!(prompt, format!("{expected}\n"));
                    stream.write_all(answer.as_bytes()).unwrap();
                }
            });
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "auth::tests::gui_registered_password_is_target_bound_and_used_only_once",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .env("PATH", &root)
                .env(GUI_BROKER, address)
                .env(GUI_TOKEN, "d".repeat(64))
                .env("COLM_SSH_AUTH_STDIN", "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(b"{\"host\":\"server\",\"auth\":\"key-password\",\"password\":\"registered target secret\"}\n").unwrap();
            let result = child.wait_with_output().unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                result.status.success(),
                "{} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            server.join().unwrap();
        }
        return;
    }
    let direct = mode.unwrap() == "direct";
    initialize_from_stdin().unwrap();
    assert_eq!(GUI_TARGET_PASSWORD.lock().unwrap().is_some(), direct);
    let (address, token) = GUI_RELAY.get().unwrap();
    for (prompt, answer) in [
        ("jump@host's password: ", "jump secret"),
        ("Enter passphrase for key '/tmp/key': ", "key secret"),
        (
            "scientist@target's password: ",
            if direct {
                "registered target secret"
            } else {
                "GUI target-shaped jump reply"
            },
        ),
        ("scientist@target's password: ", "corrected target secret"),
    ] {
        assert_eq!(
            request_secret(address, token, prompt).unwrap(),
            answer.as_bytes()
        );
        assert_eq!(redact(answer), "[redacted]");
    }
    assert!(GUI_TARGET_PASSWORD.lock().unwrap().is_none());
}

#[test]
fn only_direct_connections_allow_registered_password_prefill() {
    for proxy in ["", "proxyjump none\nproxycommand none\n", "proxyjump \n"] {
        assert_eq!(
            direct_password_prompt(&format!("user scientist\nhostname target\n{proxy}")).unwrap(),
            Some("scientist@target's password: ".into())
        );
    }
    for proxy in ["proxyjump bridge", "proxycommand ssh -W %h:%p bridge"] {
        assert_eq!(
            direct_password_prompt(&format!("user scientist\nhostname target\n{proxy}\n")).unwrap(),
            None
        );
    }
}
