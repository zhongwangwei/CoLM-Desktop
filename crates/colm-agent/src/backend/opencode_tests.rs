use super::*;
use crate::agent::Decision;
#[test]
fn models_only_include_enabled_models_and_variants() {
    let models = model_list(
        &json!({"data":[{"providerID":"openai","id":"gpt","name":"GPT","enabled":true,"variants":[{"id":"high"}]},{"providerID":"other","id":"private","enabled":false}]}),
    );
    assert_eq!(
        models,
        json!([{"id":"openai/gpt","name":"GPT","efforts":["high"]}])
    );
}
#[test]
fn completion_requires_idle_success_and_untruncated_answer() {
    let answer = |finish: &str, outcome: &str| {
        vec![
            json!({"type":"assistant","finish":finish,"tokens":{"input":3,"output":4},"content":[{"type":"text","text":"done"}]}),
            json!({"type":"idle","outcome":outcome}),
        ]
    };
    assert!(final_answer(&[]).unwrap().is_none());
    assert!(final_answer(&answer("length", "succeeded")).is_err());
    assert!(final_answer(&answer("tool-calls", "succeeded")).is_err());
    assert!(final_answer(&answer("stop", "interrupted")).is_err());
    let out = final_answer(&answer("stop", "succeeded")).unwrap().unwrap();
    assert_eq!(out.content, "done");
    assert_eq!(out.usage.prompt_tokens, 3);
}
#[test]
fn v2_contract_required_and_identifiers_are_safe() {
    let doc: Value =
        serde_json::from_str(include_str!("testdata/opencode-v2-contract.json")).unwrap();
    validate_contract(&doc).unwrap();
    assert!(validate_contract(
        &json!({"paths":{"/session/{id}/permissions/{permissionID}":{"post":{}}}})
    )
    .is_err());
    for id in ["../x", "a/b", "a?token=x", ""] {
        assert!(identifier(id).is_err());
    }
    assert!(identifier("ses_abCD-123").is_ok());
    let config = configuration(None);
    assert_eq!(config["permissions"][0]["effect"], "deny");
    assert_eq!(
        config["agents"]["colm"]["permissions"],
        config["permissions"]
    );
}
#[cfg(unix)]
struct Fixture {
    root: PathBuf,
    exe: PathBuf,
}
#[cfg(unix)]
impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("colm-oc-test-{}", crate::bridge::random_hex(12)));
        std::fs::create_dir(&root).unwrap();
        let exe = root.join("opencode");
        std::fs::write(&exe, include_str!("testdata/opencode_v2.py")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            root.join("contract.json"),
            include_str!("testdata/opencode-v2-contract.json"),
        )
        .unwrap();
        Self { root, exe }
    }
    fn session(&self, resume: Option<String>) -> OpenCodeSession {
        let launch = Launch {
            cwd: self.root.clone(),
            mcp_exe: PathBuf::from("/not-run-mcp"),
            bridge_addr: "127.0.0.1:1".into(),
            bridge_token: "secret-test-bridge".into(),
            instructions: "CoLM".into(),
            web: false,
        };
        let mut session = OpenCodeSession::new(launch, resume, self.root.join("profile"));
        session.set_choice(ExternalChoice {
            model: Some("mock/model".into()),
            effort: Some("high".into()),
        });
        if let Err(error) = session.connect_with(&self.exe) {
            let log = std::fs::read_to_string(self.root.join("stderr.log")).unwrap_or_default();
            panic!("{error:#}\n--- fake OpenCode stderr ---\n{log}");
        }
        session
    }
    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[derive(Default)]
struct Sink {
    events: Vec<Outbound>,
    approvals: usize,
    remember: bool,
}
impl TurnSink for Sink {
    fn emit(&mut self, event: Outbound) {
        self.events.push(event);
    }
    fn approve(&mut self, event: Outbound) -> Decision {
        self.approvals += 1;
        self.events.push(event);
        if self.remember {
            Decision::ApproveForSession
        } else {
            Decision::Deny(None)
        }
    }
}
#[test]
#[cfg(unix)]
fn fake_cli_v2_native_denial_streaming_resume_and_cleanup() {
    let fixture = Fixture::new();
    let mut session = fixture.session(None);
    let configroot = session.server.as_ref().unwrap().root.clone();
    let mut sink = Sink {
        remember: true,
        ..Default::default()
    };
    for _ in 0..2 {
        assert_eq!(
            session
                .turn("hello", &mut sink, &AtomicBool::new(false))
                .unwrap()
                .content,
            "done"
        );
    }
    assert_eq!(sink.approvals, 0);
    assert!(sink
        .events
        .iter()
        .any(|e| matches!(e, Outbound::ToolResult { ok: true, .. })));
    assert!(sink
        .events
        .iter()
        .any(|e| matches!(e, Outbound::ReasoningDelta { .. })));
    let requests = fixture.requests();
    assert!(requests
        .iter()
        .any(|r| r["body"]["permissions"] == permission(false)));
    let replies: Vec<_> = requests
        .iter()
        .filter(|r| r["path"].as_str().unwrap().ends_with("/reply"))
        .collect();
    assert_eq!(replies.len(), 2);
    assert!(replies.iter().all(|r| r["body"]["decision"] == "reject"));
    let id = session.resume_id();
    drop(session);
    assert!(!configroot.exists());
    let mut resumed = fixture.session(id);
    let mut deny = Sink::default();
    resumed
        .turn("hello", &mut deny, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(deny.approvals, 0);
    assert!(fixture
        .requests()
        .iter()
        .any(|r| r["body"]["decision"] == "reject"));
}
#[test]
#[cfg(unix)]
fn fake_cli_v2_cancel_interrupts_and_drops_owned_server() {
    let fixture = Fixture::new();
    let mut session = fixture.session(None);
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        signal.store(true, Ordering::SeqCst);
    });
    assert!(session
        .turn("cancel", &mut Sink::default(), &cancel)
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    thread.join().unwrap();
    assert!(session.server.is_none());
    assert!(fixture
        .requests()
        .iter()
        .any(|r| r["path"] == "/api/session/ses_mock/interrupt"));
}
#[test]
#[cfg(unix)]
fn fake_cli_mcp_permissions_are_forwarded_without_persistent_allow() {
    let fixture = Fixture::new();
    let mut session = fixture.session(None);
    let config = configuration(Some(&session.launch));
    assert_eq!(config["mcp"]["servers"]["colm"]["codemode"], false);
    assert_eq!(
        config["permissions"][1],
        json!({"action":"colm_*","resource":"*","effect":"allow"})
    );
    session
        .turn("colm", &mut Sink::default(), &AtomicBool::new(false))
        .unwrap();
    assert!(fixture
        .requests()
        .iter()
        .any(|r| r["body"]["decision"] == "once"));
    assert!(!fixture
        .requests()
        .iter()
        .any(|r| r["body"]["decision"] == "always"));
}
#[test]
#[cfg(unix)]
fn rejected_http_does_not_expose_basic_auth_or_bridge_tokens() {
    let fixture = Fixture::new();
    let session = fixture.session(None);
    let error = session
        .server
        .as_ref()
        .unwrap()
        .get("/secret-error")
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 404"));
    assert!(!error.contains("http:"));
    assert!(!error.contains("secret-test-bridge"));
}
#[test]
#[cfg(unix)]
fn invalid_runtime_contract_is_rejected_before_any_prompt() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("contract.json"), "{}").unwrap();
    let error = match Server::start(&fixture.exe, &fixture.root, None) {
        Ok(_) => panic!("accepted unsupported server"),
        Err(e) => e,
    };
    assert!(
        error
            .to_string()
            .contains("unsupported OpenCode V2 server contract"),
        "{error:#}"
    );
    assert!(!fixture.root.join("requests.jsonl").exists());
}
#[test]
#[cfg(target_os = "linux")]
fn version_probe_retries_a_temporarily_busy_executable() {
    let fixture = Fixture::new();
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&fixture.exe)
        .unwrap();
    assert_eq!(
        Command::new(&fixture.exe)
            .arg("--version")
            .output()
            .unwrap_err()
            .raw_os_error(),
        Some(26)
    );
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        drop(writer);
    });
    let result = verify_cli_version(&fixture.exe, &fixture.root);
    release.join().unwrap();
    result.unwrap();
}
#[test]
fn provider_failure_keeps_actionable_status_without_sensitive_body() {
    let messages = vec![
        json!({"type":"idle","outcome":"failed","error":{"type":"api","status":429,"message":"key sk-secret at https://private.example","response":{"body":"secret-test-bridge"}}}),
    ];
    let error = final_answer(&messages).unwrap_err().to_string();
    assert!(error.contains("429"));
    assert!(error.contains("quota"));
    assert!(!error.contains("sk-secret"));
    assert!(!error.contains("private.example"));
    assert!(!error.contains("secret-test-bridge"));
}
#[test]
#[cfg(unix)]
fn unavailable_model_is_rejected_before_sending_prompt() {
    let fixture = Fixture::new();
    let mut session = fixture.session(None);
    session.set_choice(ExternalChoice {
        model: Some("other/private".into()),
        effort: None,
    });
    let error = session
        .turn("do work", &mut Sink::default(), &AtomicBool::new(false))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not enabled"));
    assert!(!fixture
        .requests()
        .iter()
        .any(|r| r["path"].as_str().unwrap().ends_with("/prompt")));
}
#[test]
#[cfg(unix)]
fn fake_cli_cannot_redirect_authenticated_requests_off_loopback() {
    let fixture = Fixture::new();
    std::fs::write(
        &fixture.exe,
        "#!/usr/bin/env python3\nimport sys\nif sys.argv[1:] == [\"--version\"]: print(\"2.0.6\"); sys.exit(0)\nprint('{\"url\":\"https://example.com:443\"}', flush=True)\n",
    )
    .unwrap();
    let error = match Server::start(&fixture.exe, &fixture.root, None) {
        Ok(_) => panic!("accepted nonlocal server"),
        Err(e) => e,
    };
    assert!(error.to_string().contains("IPv4 loopback"));
}
#[test]
#[cfg(unix)]
fn private_profile_survives_server_and_does_not_reuse_host_data_or_home() {
    let fixture = Fixture::new();
    let session = fixture.session(None);
    let profile = fixture.root.join("profile");
    let actual: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.root.join("environment.json")).unwrap(),
    )
    .unwrap();
    for (key, path) in profile_environment(&profile) {
        assert_eq!(actual[key], json!(path));
    }
    assert_eq!(actual["OPENCODE_CONFIG_PROJECT_DISABLE"], "true");
    assert_ne!(
        actual["HOME"].as_str(),
        std::env::var("HOME").ok().as_deref()
    );
    let config = configuration(Some(&session.launch));
    assert_eq!(
        config["mcp"]["servers"]["colm"]["environment"]["HOME"].as_str(),
        std::env::var("HOME").ok().as_deref()
    );
    drop(session);
    assert!(profile.is_dir());
    assert!(setup_command(&fixture.exe, &profile).contains("--standalone"));
}
#[test]
fn profile_path_is_absolute_and_setup_command_quotes_shell_metacharacters() {
    assert!(profile_dir(Some(Path::new("relative")))
        .unwrap()
        .is_absolute());
    let command = setup_command(Path::new("/a b/opencode"), Path::new("/profile's dir"));
    assert!(command.contains("OPENCODE_DB"));
    #[cfg(not(windows))]
    assert!(command.contains("'\"'\"'"));
}

#[test]
#[cfg(unix)]
fn profile_guard_blocks_organization_sources_without_reading_credentials() {
    let fixture = Fixture::new();
    let profile = fixture.root.join("profile");
    create_private_profile(&profile).unwrap();
    let db = profile.join("opencode.db");
    let sql = |query: &str| {
        Command::new("sqlite3")
            .arg(&db)
            .arg(query)
            .output()
            .unwrap()
    };
    assert!(sql("CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT); INSERT INTO kv VALUES ('credential:provider', 'do-not-read-secret');").status.success());
    check_profile_sources(&profile).unwrap();
    assert!(!sql("INSERT INTO kv VALUES ('wellknown:sources','[]');")
        .status
        .success());
    assert!(
        !sql("UPDATE kv SET key='wellknown:sources' WHERE key='credential:provider';")
            .status
            .success()
    );
    assert!(
        sql("UPDATE kv SET value='refreshed-secret' WHERE key='credential:provider';")
            .status
            .success()
    );
    check_profile_sources(&profile).unwrap();
    // A conflicting trigger cannot silently replace the verified guard.
    assert!(sql("DROP TRIGGER colm_block_wellknown_insert; CREATE TRIGGER colm_block_wellknown_insert BEFORE INSERT ON kv BEGIN SELECT 1; END;").status.success());
    assert!(check_profile_sources(&profile).is_err());
}

#[test]
#[cfg(unix)]
fn legacy_and_existing_organization_profiles_are_rejected_before_cli_launch() {
    let fixture = Fixture::new();
    let profile = fixture.root.join("profile");
    create_private_profile(&profile).unwrap();
    std::fs::create_dir_all(profile.join("data/opencode")).unwrap();
    let legacy = profile.join("data/opencode/auth.json");
    std::fs::write(&legacy, "not parsed: fake legacy credentials").unwrap();
    assert!(Server::start(&fixture.exe, &profile, None).is_err());
    assert!(!fixture.root.join("environment.json").exists());
    std::fs::remove_file(legacy).unwrap();
    assert!(Command::new("sqlite3").arg(profile.join("opencode.db")).arg("CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT); INSERT INTO kv VALUES ('wellknown:sources','never read this value');").status().unwrap().success());
    assert!(Server::start(&fixture.exe, &profile, None).is_err());
    assert!(!fixture.root.join("environment.json").exists());
}

#[test]
#[cfg(unix)]
fn unknown_version_cannot_modify_the_profile_database() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.exe, "#!/bin/sh\nprintf '2.0.7\\n'\n").unwrap();
    let profile = fixture.root.join("profile");
    create_private_profile(&profile).unwrap();
    let db = profile.join("opencode.db");
    assert!(Command::new("sqlite3")
        .arg(&db)
        .arg("CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT);")
        .status()
        .unwrap()
        .success());
    let before = std::fs::read(&db).unwrap();
    assert!(Server::start(&fixture.exe, &profile, None).is_err());
    assert_eq!(before, std::fs::read(&db).unwrap());
}
