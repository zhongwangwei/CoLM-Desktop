use super::*;

#[test]
fn pending_study_status_uses_its_configured_connection_after_restart() {
    let root = std::env::temp_dir().join(format!(
        "colm-gui-pending-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join(".colm-remote-study-pending.json"), json!({"host":"server","root":"/data/colm","job":"study-1","remote_project":"/data/colm/s/abc"}).to_string()).unwrap();
    let server:Server = serde_json::from_value(json!({"host":"server","root":"/data/colm","username":"researcher","auth":"password","port":2222})).unwrap();
    let config = RemoteConfig {
        servers: vec![server.clone()],
        local: None,
    };
    let recovered = study_server(&config, root.to_str().unwrap()).unwrap();
    assert_eq!(recovered.host, server.host);
    assert_eq!(recovered.username, server.username);
    assert_eq!(recovered.auth, "password");
    assert_eq!(recovered.port, 2222);
    assert!(study_server(&RemoteConfig::default(), root.to_str().unwrap()).is_err());
    // A broken formal record cannot fall back to an intent for another identity.
    std::fs::write(root.join(".colm-remote-study.json"), b"broken").unwrap();
    assert!(study_server(&config, root.to_str().unwrap()).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ssh_aliases_are_read_without_wildcards_or_duplicates() {
    let text = "Host macpro\n  Hostname 1.2.3.4\nHost 7920 7920land\n  User zhwei\nHost *\n  ServerAliveInterval 30\nHost tms1? !bad\nhost 7920land\n";
    assert_eq!(ssh_aliases(text), ["macpro", "7920", "7920land"]);
}

#[test]
fn server_settings_are_validated() {
    let server = |host: &str, root: &str| Server {
        host: host.into(),
        name: String::new(),
        root: root.into(),
        username: String::new(),
        port: 0,
        auth: "config".into(),
        identity_file: String::new(),
        maps: vec![PathMap {
            // 本机一侧按本机的绝对路径规则校验（Windows 要带盘符）。
            local: if cfg!(windows) {
                r"D:\Data\PLUMBER2s"
            } else {
                "/Volumes/Data/Data/PLUMBER2s"
            }
            .into(),
            remote: "/media/zhwei/data02/zhwei/training2026/PLUMBER2s".into(),
        }],
        threads: 8,
        scheduler: "auto".into(),
        partition: String::new(),
        account: String::new(),
        walltime: String::new(),
        cpus: 0,
        memory_gb: 0,
        nodes: 0,
        cores_per_node: 0,
        fetch_vars: String::new(),
        env_script: String::new(),
        directives: Vec::new(),
        project_dir: String::new(),
        rawdata_dir: String::new(),
        runtime_dir: String::new(),
        forcing_dir: String::new(),
        kind_paths: Default::default(),
    };
    let ok = RemoteConfig {
        servers: vec![server("7920land", "/media/zhwei/data02/zhwei/colm-desktop")],
        local: None,
    };
    assert!(validate(&ok).is_ok());
    for (host, root) in [
        ("", "/a"),
        ("-oProxyCommand=x", "/a"),
        ("a b", "/a"),
        ("h", "relative"),
        ("h", "/"),
        ("h", "/a b"),
    ] {
        assert!(
            validate(&RemoteConfig {
                servers: vec![server(host, root)],
                local: None,
            })
            .is_err(),
            "{host} {root}"
        );
    }
    let mut bad_map = server("h", "/a");
    bad_map.maps[0].remote = "relative".into();
    assert!(validate(&RemoteConfig {
        servers: vec![bad_map],
        local: None,
    })
    .is_err());
    let mut no_threads = server("h", "/a");
    no_threads.threads = 0;
    assert!(validate(&RemoteConfig {
        servers: vec![no_threads],
        local: None,
    })
    .is_err());
    // 旧配置没有 threads 字段：默认 8；调度系统默认 auto，其余为空。
    let old: Server = serde_json::from_str(r#"{"host":"h","root":"/a"}"#).unwrap();
    assert_eq!(old.threads, 8);
    assert_eq!(old.scheduler, "auto");
    assert!(old.partition.is_empty() && old.directives.is_empty() && old.cpus == 0);
}

#[test]
fn scheduler_settings_are_validated() {
    let base: Server = serde_json::from_str(r#"{"host":"h","root":"/a"}"#).unwrap();
    let check = |edit: &dyn Fn(&mut Server)| {
        let mut server = base.clone();
        edit(&mut server);
        validate(&RemoteConfig {
            servers: vec![server],
            local: None,
        })
    };
    assert!(check(&|s| {
        s.scheduler = "slurm".into();
        s.partition = "cpu_short".into();
        s.account = "proj-a".into();
        s.walltime = "1-02:00:00".into();
        s.directives = vec!["--constraint=ib".into(), String::new()];
    })
    .is_ok());
    assert!(check(&|s| s.scheduler = "condor".into()).is_err());
    assert!(check(&|s| s.partition = "-evil".into()).is_err());
    assert!(check(&|s| s.partition = "a b".into()).is_err());
    assert!(check(&|s| s.account = "x;rm".into()).is_err());
    assert!(check(&|s| s.walltime = "soon".into()).is_err());
    assert!(check(&|s| s.directives = vec!["no-dash".into()]).is_err());
    assert!(check(&|s| s.env_script = "cat <<COLM_JOB_EOF".into()).is_err());
}

#[test]
fn the_node_size_reaches_the_run_so_big_mpi_jobs_plan_their_nodes() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    server.cores_per_node = 48;
    let args = run_args(
        &server,
        "/c".into(),
        "/k".into(),
        None,
        false,
        false,
        &Engine::default(),
    );
    let at = args.iter().position(|a| a == "--cores-per-node").unwrap();
    assert_eq!(args[at + 1], "48");
    assert!(scheduler_args(&server).contains(&"--cores-per-node".to_string()));
    let mut invalid = server.clone();
    invalid.cores_per_node = 5000;
    assert!(validate(&RemoteConfig {
        servers: vec![invalid],
        local: None,
    })
    .is_err());
}

#[test]
fn run_arguments_carry_the_scheduler_request_and_the_preview_flag() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    server.scheduler = "slurm".into();
    server.partition = "cpu".into();
    server.walltime = "04:00:00".into();
    server.memory_gb = 32;
    server.env_script = "module load rust".into();
    server.directives = vec!["--constraint=ib".into()];
    let args = run_args(
        &server,
        "/c".into(),
        "/k".into(),
        None,
        false,
        false,
        &Engine::default(),
    );
    let after = |flag: &str| {
        let at = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} in {args:?}"));
        args[at + 1].clone()
    };
    assert_eq!(after("--scheduler"), "slurm");
    assert_eq!(after("--partition"), "cpu");
    assert_eq!(after("--walltime"), "04:00:00");
    assert_eq!(after("--mem-gb"), "32");
    assert_eq!(after("--env-script"), "module load rust");
    assert_eq!(after("--directive"), "--constraint=ib");
    assert!(!args.contains(&"--account".to_string()) && !args.contains(&"--cpus".to_string()));
    // Unknown node size: nothing is sent; known, the server plans nodes from it.
    assert!(!args.contains(&"--cores-per-node".to_string()));
    assert!(!args.contains(&"--dry-run".to_string()));
    let preview = run_args(
        &server,
        "/c".into(),
        "/k".into(),
        Some("colm".into()),
        true,
        true,
        &Engine::default(),
    );
    assert!(preview.windows(2).any(|w| w == ["--dry-run", "1"]));
    assert!(preview.windows(2).any(|w| w == ["--stage", "colm"]));
    assert!(preview.windows(2).any(|w| w == ["--force", "1"]));
}

#[test]
fn remote_logs_are_parsed_like_local_runs() {
    let log = "kernel: default\n=== colm-stage mksrfdata begin ===\n=== colm-stage mksrfdata ok ===\n=== colm-stage mkinidata skipped ===\n=== colm-stage colm begin ===\nTIMESTEP = 120 | DATE = 2004-01-03-86400\nTIMESTEP = 121 | DATE = 2004-01-04-1800\n";
    let (stages, progress) = crate::sidecar::parse_log(log);
    assert_eq!(
        stages,
        [
            ("mksrfdata".to_owned(), "ok".to_owned()),
            ("mkinidata".to_owned(), "skipped".to_owned()),
            ("colm".to_owned(), "begin".to_owned()),
        ]
    );
    assert_eq!(progress, Some((121, "2004-01-04-1800".to_owned())));
}

#[test]
fn the_fortran_engine_passes_ranks_and_nodes_only_when_it_uses_mpi() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    server.nodes = 2;
    let go = |engine: Option<&str>, ranks: Option<u32>| {
        run_args(
            &server,
            "/c".into(),
            "/k".into(),
            None,
            false,
            false,
            &Engine {
                engine: engine.map(str::to_owned),
                ranks,
                model_version: None,
            },
        )
    };
    let value = |args: &[String], flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .map(|i| args[i + 1].clone())
    };
    let fortran = go(Some("fortran"), Some(8));
    assert_eq!(value(&fortran, "--engine").as_deref(), Some("fortran"));
    assert_eq!(value(&fortran, "--ranks").as_deref(), Some("8"));
    assert_eq!(value(&fortran, "--nodes").as_deref(), Some("2"));
    // 一个进程不用 MPI，也就不申请节点。
    let one = go(Some("fortran"), Some(1));
    assert!(value(&one, "--ranks").is_none() && value(&one, "--nodes").is_none());
    // Rust 引擎：什么都不加，沿用 colm-cli 的默认。
    let rust = go(Some("rust"), Some(8));
    assert!(value(&rust, "--engine").is_none() && value(&rust, "--ranks").is_none());
    assert!(value(&go(None, None), "--engine").is_none());
}

#[test]
fn fetching_only_the_configured_variables_unless_everything_is_asked_for() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    // 没配：取全部。
    assert_eq!(
        fetch_args("/c".into(), Some(&server), false, false),
        ["remote-fetch", "/c"]
    );
    server.fetch_vars = " f_fsena,f_rnet ".into();
    assert_eq!(
        fetch_args("/c".into(), Some(&server), false, false),
        ["remote-fetch", "/c", "--vars", "f_fsena,f_rnet"]
    );
    // 要全部：不加 --vars。找不到服务器配置时也一样。
    assert_eq!(
        fetch_args("/c".into(), Some(&server), true, false),
        ["remote-fetch", "/c"]
    );
    assert_eq!(
        fetch_args("/c".into(), None, false, false),
        ["remote-fetch", "/c"]
    );
    // 手动取回才允许覆盖比远程运行更新的本机结果。
    assert_eq!(
        fetch_args("/c".into(), None, true, true),
        ["remote-fetch", "/c", "--overwrite-newer", "1"]
    );
    for bad in ["f_a;rm", "f a", "f_a -x"] {
        server.fetch_vars = bad.into();
        assert!(
            validate(&RemoteConfig {
                servers: vec![server.clone()],
                local: None,
            })
            .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn legacy_servers_keep_ssh_config_and_password_is_never_serialized() {
    let server: Server = serde_json::from_str(
        r#"{"host":"legacy","root":"/data/colm","password":"should-never-save"}"#,
    )
    .unwrap();
    assert_eq!(server.auth, "config");
    assert!(server.username.is_empty());
    assert_eq!(server.port, 0);
    let saved = serde_json::to_string(&server).unwrap();
    assert!(!saved.contains("password\":") && !saved.contains("should-never-save"));
}

#[test]
fn passwords_are_session_only_and_bound_to_the_connection() {
    let mut server: Server = serde_json::from_str(r#"{"host":"password-test-only","root":"/data/colm","auth":"password","username":"alice","port":2222}"#).unwrap();
    let missing = auth_payload(&server).unwrap_err();
    assert!(missing.contains("alice") && missing.contains("2222"));
    remote_set_password(
        server.host.clone(),
        server.username.clone(),
        server.port,
        "test-only-secret".into(),
    )
    .unwrap();
    assert_eq!(
        auth_payload(&server).unwrap()["password"],
        "test-only-secret"
    );
    server.username = "bob".into();
    assert!(auth_payload(&server).is_err());
    server.username = "alice".into();
    server.port = 22;
    assert!(auth_payload(&server).is_err());
    server.auth = "key".into();
    assert!(auth_payload(&server).unwrap()["password"].is_null());
    for password in ["", "one\ntwo", "one\rtwo", "one\0two"] {
        assert!(remote_set_password("p-test".into(), "alice".into(), 22, password.into()).is_err());
    }
    assert!(remote_set_password("-option".into(), "alice".into(), 22, "valid".into()).is_err());
    assert!(remote_set_password("host".into(), "-option".into(), 22, "valid".into()).is_err());
    passwords()
        .lock()
        .unwrap()
        .remove(&("password-test-only".into(), "alice".into(), 2222));
}

#[test]
fn auth_payload_only_sends_an_identity_file_in_key_mode() {
    let mut server: Server = serde_json::from_value(json!({
        "host": "identity-mode-test",
        "root": "/data/colm",
        "auth": "config",
        "identity_file": "/tmp/id_ed25519"
    }))
    .unwrap();
    assert_eq!(auth_payload(&server).unwrap()["identity_file"], "");
    server.auth = "key".into();
    assert_eq!(
        auth_payload(&server).unwrap()["identity_file"],
        "/tmp/id_ed25519"
    );
    server.auth = "password".into();
    remote_set_password(
        server.host.clone(),
        server.username.clone(),
        server.port,
        "identity-test-secret".into(),
    )
    .unwrap();
    assert_eq!(auth_payload(&server).unwrap()["identity_file"], "");
    passwords()
        .lock()
        .unwrap()
        .remove(&(server.host, server.username, server.port));
}

#[test]
fn connection_options_reject_invalid_modes_users_and_relative_keys() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    for user in ["-root", "alice@host", "alice bob", "alice\nroot"] {
        server.username = user.into();
        assert!(validate(&RemoteConfig {
            servers: vec![server.clone()],
            local: None,
        })
        .is_err());
    }
    server.username = "alice".into();
    server.auth = "unknown".into();
    assert!(validate(&RemoteConfig {
        servers: vec![server.clone()],
        local: None,
    })
    .is_err());
    server.auth = "key".into();
    server.identity_file = "relative-key".into();
    assert!(validate(&RemoteConfig {
        servers: vec![server.clone()],
        local: None,
    })
    .is_err());
    server.identity_file.clear();
    assert!(validate(&RemoteConfig {
        servers: vec![server],
        local: None,
    })
    .is_ok());
}

#[test]
fn saved_jobs_keep_their_original_login_when_a_profile_changes() {
    let changed: Server = serde_json::from_value(json!({"host":"c1", "root":"/data/new", "auth":"key", "username":"bob", "port":22, "fetch_vars":"f_rnet"})).unwrap();
    let config = RemoteConfig {
        servers: vec![changed],
        local: None,
    };
    let record = json!({"host":"c1", "root":"/data/original", "ssh_auth":{"host":"c1", "username":"alice", "port":2222, "auth":"password", "identity_file":""}});
    let original = server_from_record(&config, &record).unwrap();
    assert_eq!(
        (
            original.username.as_str(),
            original.port,
            original.auth.as_str()
        ),
        ("alice", 2222, "password")
    );
    assert_eq!(original.root, "/data/original");
    assert_eq!(original.fetch_vars, "f_rnet");
    assert_eq!(
        server_from_record(&RemoteConfig::default(), &record)
            .unwrap()
            .port,
        2222
    );
    let mut wrong_host = record.clone();
    wrong_host["ssh_auth"]["host"] = json!("other");
    assert!(server_from_record(&config, &wrong_host).is_err());
    let old = json!({"host":"c1", "root":"/data/original"});
    assert_eq!(server_from_record(&config, &old).unwrap().username, "");
    assert_eq!(
        server_from_record(&RemoteConfig::default(), &old)
            .unwrap()
            .auth,
        "config"
    );
}

#[test]
fn job_identity_can_be_recovered_without_credentials() {
    let dir = std::env::temp_dir().join(format!("colm-gui-auth-record-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.to_string_lossy().into_owned();
    assert!(remote_job_record(case.clone()).unwrap().is_none());
    let record = dir.join(".colm-remote.json");
    std::fs::write(
        &record,
        r#"{"host":"c1","job":"run-1","password":"never-return"}"#,
    )
    .unwrap();
    let identity = remote_job_record(case.clone()).unwrap().unwrap();
    assert_eq!(identity["host"], "c1");
    assert_eq!(identity["job"], "run-1");
    assert_eq!(identity["pending"], false);
    assert!(!identity.to_string().contains("never-return"));
    std::fs::write(&record, "broken").unwrap();
    assert!(remote_job_record(case).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_source_pending_run_outranks_old_records_and_exposes_only_status_recovery() {
    let dir = std::env::temp_dir().join(format!(
        "colm-run-intent-{}",
        colm_history::Event::start("test").operation_id
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(dir.join("case.nml"), "&nl_colm\n/\n").unwrap();
    std::fs::write(
        dir.join(".colm-remote.json"),
        json!({"host":"old-server","job":"old-completed"}).to_string(),
    )
    .unwrap();
    let intent = json!({"schema":1,"source_case":dir,"run_case":dir,"host":"server","root":"/data/colm","job":"new-uncertain","engine":"engine","remote_case":"/data/colm/cases/a","provenance":{"operation_id":"original-op"},"ssh_auth":{"host":"server","username":"alice","port":2222,"auth":"password","identity_file":""}});
    std::fs::write(dir.join(".colm-remote-pending.json"), intent.to_string()).unwrap();
    let mut configured: Server = serde_json::from_value(
        json!({"host":"server","root":"/data/changed","username":"bob","port":22,"auth":"config"}),
    )
    .unwrap();
    configured.fetch_vars = "f_rnet".into();
    let config = RemoteConfig {
        servers: vec![configured],
        local: None,
    };
    let case = dir.display().to_string();
    let record = remote_job_record(case.clone()).unwrap().unwrap();
    assert_eq!(record["job"], "new-uncertain");
    assert_eq!(record["pending"], true);
    assert_eq!(record["operation_id"], "original-op");
    assert!(record.get("ssh_auth").is_none());
    let recovered = command_server(&config, &["remote-status".into(), case.clone()])
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            recovered.username.as_str(),
            recovered.port,
            recovered.auth.as_str()
        ),
        ("alice", 2222, "password")
    );
    assert_eq!(recovered.root, "/data/colm");
    for action in ["remote-fetch", "remote-cancel"] {
        assert!(command_server(&config, &[action.into(), case.clone()]).is_err());
    }
    let mut wrong = intent;
    wrong["source_case"] = json!(dir.join("another"));
    std::fs::write(dir.join(".colm-remote-pending.json"), wrong.to_string()).unwrap();
    assert!(remote_job_record(case).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

fn run_identity_fixture(
    source: &std::path::Path,
    run_case: &std::path::Path,
    operation: &str,
) -> colm_model::MaterializedRun {
    colm_model::MaterializedRun {
        schema: 1,
        operation_id: operation.into(),
        source_case: source.into(),
        run_case: run_case.into(),
        version: colm_model::VersionRef {
            id: "model".into(),
            revision: 1,
        },
        code: colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        },
        case_sha256: "a".repeat(64),
        effective_nml_sha256: "b".repeat(64),
        overlay_sha256: "c".repeat(64),
        revision_fingerprint: "d".repeat(64),
        artifact_paths: vec![],
        inputs: Default::default(),
    }
}

#[test]
fn frozen_run_paths_recover_the_source_intent_and_reject_changed_manifests() {
    let dir = std::env::temp_dir().join(format!(
        "colm-copy-intent-{}",
        colm_history::Event::start("test").operation_id
    ));
    std::fs::create_dir_all(dir.join("source")).unwrap();
    std::fs::create_dir_all(dir.join("copy")).unwrap();
    let source = dir.join("source").canonicalize().unwrap();
    let copy = dir.join("copy").canonicalize().unwrap();
    std::fs::write(source.join("case.nml"), "&nl_colm\n/\n").unwrap();
    let mut run = run_identity_fixture(&source, &copy, "op");
    std::fs::write(
        copy.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec(&run).unwrap(),
    )
    .unwrap();
    std::fs::write(
        copy.join(".colm-remote.json"),
        json!({"host":"old-server","job":"old"}).to_string(),
    )
    .unwrap();
    std::fs::write(source.join(".colm-remote-pending.json"),json!({"schema":1,"source_case":source,"run_case":copy,"host":"server","root":"/data/colm","job":"new","engine":"engine","remote_case":"/data/colm/cases/a","provenance":{"operation_id":"op"}}).to_string()).unwrap();
    let (record, pending, target) = run_record(copy.to_str().unwrap()).unwrap().unwrap();
    assert!(pending);
    assert_eq!(record["job"], "new");
    assert_eq!(target, source.display().to_string());
    run.run_case = source.clone();
    std::fs::write(
        copy.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec(&run).unwrap(),
    )
    .unwrap();
    assert!(run_record(copy.to_str().unwrap()).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn source_run_lookup_prefers_an_unfinished_operation_over_a_newer_completed_copy() {
    let dir = std::env::temp_dir().join(format!(
        "colm-recorded-copy-{}",
        colm_history::Event::start("test").operation_id
    ));
    for name in ["source", "pending", "completed"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
    let source = dir.join("source").canonicalize().unwrap();
    std::fs::write(source.join("case.nml"), "&nl_colm\n/\n").unwrap();
    let history = colm_history::History::open(dir.join("history.sqlite3")).unwrap();
    for (name, time, phase, status) in [
        (
            "pending",
            10,
            colm_history::EventPhase::Status,
            "needs_reconciliation",
        ),
        (
            "completed",
            20,
            colm_history::EventPhase::Finished,
            "succeeded",
        ),
    ] {
        let mut event = colm_history::Event::start("run");
        event.time_ms = Some(time);
        event.phase = phase;
        event.status = status.into();
        event.case_path = Some(source.display().to_string());
        event.location = Some("remote".into());
        let copy = dir.join(name).canonicalize().unwrap();
        let run = run_identity_fixture(&source, &copy, &event.operation_id);
        std::fs::write(
            copy.join(colm_model::RUN_MANIFEST),
            serde_json::to_vec(&run).unwrap(),
        )
        .unwrap();
        event.detail = json!({"action":"remote-run","model_run":run,"remote_record":{"host":"server","root":"/data/colm","job":name,"engine":"engine","remote_case":format!("/data/colm/cases/{name}")}});
        history.append(&event).unwrap();
    }
    let (copy, unresolved) = recorded_run_case(&history, &source).unwrap().unwrap();
    assert!(unresolved);
    assert_eq!(copy, dir.join("pending").canonicalize().unwrap());
    let mut run: Value =
        serde_json::from_slice(&std::fs::read(copy.join(colm_model::RUN_MANIFEST)).unwrap())
            .unwrap();
    run["operation_id"] = json!("different");
    std::fs::write(copy.join(colm_model::RUN_MANIFEST), run.to_string()).unwrap();
    assert!(recorded_run_case(&history, &source).is_err());
    drop(history);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_presubmit_materialized_copy_without_a_job_never_hides_a_real_completed_run() {
    let dir = std::env::temp_dir().join(format!(
        "colm-presubmit-copy-{}",
        colm_history::Event::start("test").operation_id
    ));
    for name in ["source", "ghost", "completed"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
    let source = dir.join("source").canonicalize().unwrap();
    std::fs::write(source.join("case.nml"), "&nl_colm\n/\n").unwrap();
    let history = colm_history::History::open(dir.join("history.sqlite3")).unwrap();
    for (name, time, phase, status) in [
        (
            "completed",
            10,
            colm_history::EventPhase::Finished,
            "succeeded",
        ),
        (
            "ghost",
            20,
            colm_history::EventPhase::Status,
            "needs_reconciliation",
        ),
    ] {
        let mut event = colm_history::Event::start("run");
        event.time_ms = Some(time);
        event.phase = phase;
        event.status = status.into();
        event.case_path = Some(source.display().to_string());
        event.location = Some("remote".into());
        let copy = dir.join(name).canonicalize().unwrap();
        let run = run_identity_fixture(&source, &copy, &event.operation_id);
        event.detail = json!({"action":"remote-run","model_run":run});
        if name == "completed" {
            std::fs::write(
                copy.join(colm_model::RUN_MANIFEST),
                serde_json::to_vec(&run).unwrap(),
            )
            .unwrap();
            event.detail["remote_record"] = json!({"host":"server","root":"/data/colm","job":"completed-job","engine":"engine","remote_case":"/data/colm/cases/a"});
        }
        history.append(&event).unwrap();
    }
    let (copy, unresolved) = recorded_run_case(&history, &source).unwrap().unwrap();
    assert!(!unresolved);
    assert_eq!(copy, dir.join("completed").canonicalize().unwrap());
    drop(history);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_remote_run_of_a_modified_version_names_it_and_the_official_one_adds_nothing() {
    let server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    let go = |version: Option<&str>| {
        run_args(
            &server,
            "/c".into(),
            "/k".into(),
            None,
            false,
            false,
            &Engine {
                engine: None,
                ranks: None,
                model_version: version.map(str::to_owned),
            },
        )
    };
    let modified = go(Some("snow"));
    assert!(modified
        .windows(2)
        .any(|w| w == ["--model-version", "snow"]));
    for version in [None, Some("official"), Some(" ")] {
        assert!(!go(version).iter().any(|a| a == "--model-version"));
    }
    assert_eq!(
        model_version_args(Some("snow")).unwrap(),
        ["--model-version", "snow"]
    );
    assert!(model_version_args(Some("official")).unwrap().is_empty());
    assert!(model_version_args(Some("../x")).is_err());
    assert!(model_version_args(Some("-x")).is_err());
}

#[test]
fn a_study_goes_to_the_server_with_the_run_settings_and_one_process_per_member() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    server.nodes = 2;
    let args = study_args(
        &server,
        "/p/.colm/studies/s-1".into(),
        "/k".into(),
        &Engine {
            engine: Some("fortran".into()),
            ranks: Some(8),
            model_version: Some("snow".into()),
        },
        Some(4),
    );
    assert_eq!(args[0], "remote-study-run");
    assert_eq!(args[1], "/p/.colm/studies/s-1");
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .map(|i| args[i + 1].clone())
    };
    assert_eq!(value("--host").as_deref(), Some("c1"));
    assert_eq!(value("--engine").as_deref(), Some("fortran"));
    assert_eq!(value("--jobs").as_deref(), Some("4"));
    assert_eq!(value("--model-version").as_deref(), Some("snow"));
    assert!(value("--ranks").is_none() && value("--nodes").is_none());
    assert!(absolute_study("rel/study").is_err());
}

#[test]
fn directory_browsing_accepts_incomplete_run_settings_but_validates_connection_and_path() {
    let mut server: Server =
        serde_json::from_str(r#"{"host":"tms17l","root":"","scheduler":"unfinished","threads":0}"#)
            .unwrap();
    assert_eq!(
        browse_args(&server, "tms17l", None).unwrap(),
        ["remote-browse", "--host", "tms17l", "--path", ""]
    );
    assert_eq!(
        browse_args(&server, "tms17l", Some("/data/work".into())).unwrap()[4],
        "/data/work"
    );
    assert!(validate(&RemoteConfig {
        servers: vec![server.clone()],
        local: None,
    })
    .is_err());
    assert!(browse_args(&server, "other", None).is_err());
    for path in ["relative", "/data/\nwork", "/data/\0work"] {
        assert!(browse_args(&server, "tms17l", Some(path.into())).is_err());
    }
    server.auth = "invalid".into();
    assert!(browse_args(&server, "tms17l", None).is_err());
    server.auth = "config".into();
    server.host = "-invalid".into();
    assert!(browse_args(&server, "-invalid", None).is_err());
}

#[test]
fn mkdir_uses_draft_connection_and_rejects_traversal() {
    let server: Server = serde_json::from_str(r#"{"host":"tms17l","root":""}"#).unwrap();
    assert_eq!(
        mkdir_args(&server, "tms17l", "/data".into(), "中文 folder".into()).unwrap(),
        [
            "remote-mkdir",
            "--host",
            "tms17l",
            "--path",
            "/data",
            "--name",
            "中文 folder"
        ]
    );
    assert!(mkdir_args(&server, "other", "/data".into(), "folder".into()).is_err());
    for name in ["", ".", "..", "../outside", "a/b", "a\\b", "a\n"] {
        assert!(mkdir_args(&server, "tms17l", "/data".into(), name.into()).is_err());
    }
    for path in ["", "relative", "/data\n"] {
        assert!(mkdir_args(&server, "tms17l", path.into(), "folder".into()).is_err());
    }
}

#[test]
fn connection_checks_still_reach_ssh_with_incomplete_run_settings() {
    let mut server: Server = serde_json::from_str(r#"{"host":"tms17l","root":""}"#).unwrap();
    for change in 0..3 {
        if change == 1 {
            server.root = "/data/colm".into();
            server.threads = 0;
        } else if change == 2 {
            server.threads = 8;
            server.maps.push(PathMap {
                local: "relative".into(),
                remote: "/data".into(),
            });
        }
        let (args, error) = probe_args(&server).unwrap();
        assert_eq!(args, ["remote-probe", "--host", "tms17l"]);
        assert!(error.is_some());
        assert!(validate(&RemoteConfig {
            servers: vec![server.clone()],
            local: None,
        })
        .is_err());
    }
    server.host = "-invalid".into();
    assert!(probe_args(&server).is_err());
}

#[test]
fn engine_resource_commands_validate_snapshot_and_preserve_connection() {
    let server: Server = serde_json::from_str(r#"{"host":"legacy","root":"/data/colm"}"#).unwrap();
    assert_eq!(
        engine_resource_args("remote-engine-clean", &server, Some("0123456789abcdef")).unwrap(),
        [
            "remote-engine-clean",
            "--host",
            "legacy",
            "--root",
            "/data/colm",
            "--snapshot",
            "0123456789abcdef"
        ]
    );
    for bad in ["../x", "", "pre-0123456789abcdef", "0123456789abcdef;rm"] {
        assert!(engine_resource_args("remote-engine-clean", &server, Some(bad)).is_err());
    }
    assert_eq!(
        engine_resource_args("remote-engine-recover", &server, None).unwrap(),
        [
            "remote-engine-recover",
            "--host",
            "legacy",
            "--root",
            "/data/colm"
        ]
    );
}

#[test]
fn remote_study_cancel_audit_does_not_claim_a_running_job_finished() {
    let requested = Ok(serde_json::json!({"request_accepted":true,"terminal":false}));
    assert_eq!(
        super::audit_result_status(false, true, &requested),
        "cancel_requested"
    );
    let uncertain = Ok(
        serde_json::json!({"request_accepted":false,"terminal":false,"cancel_error":"connection lost"}),
    );
    assert_eq!(
        super::audit_result_status(false, true, &uncertain),
        "cancel_uncertain"
    );
    let finished = Ok(serde_json::json!({"request_accepted":false,"terminal":true}));
    assert_eq!(
        super::audit_result_status(false, true, &finished),
        "terminal_confirmed"
    );
}

#[test]
fn uncertain_remote_transport_does_not_finalize_submission_or_cancellation() {
    let lost = Err("SSH connection lost after sending request".to_string());
    assert_eq!(
        super::audit_result_status(true, false, &lost),
        "submission_uncertain"
    );
    assert_eq!(
        super::audit_result_status(false, true, &lost),
        "cancel_uncertain"
    );
}

#[test]
fn remote_config_only_defaults_when_the_file_is_missing() {
    let root = std::env::temp_dir().join(format!(
        "colm-remote-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("remote.json");
    assert!(read_remote_config(&path).unwrap().servers.is_empty());
    std::fs::write(&path, b"{broken").unwrap();
    assert!(read_remote_config(&path)
        .unwrap_err()
        .contains("remote.json"));
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    std::fs::write(
        &path,
        br#"{"servers":[{"host":"saved","root":"/data/colm"}]}"#,
    )
    .unwrap();
    assert_eq!(read_remote_config(&path).unwrap().servers[0].host, "saved");
    // A directory at the config path is an I/O error, not a new installation.
    assert!(read_remote_config(&root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn per_run_resources_override_flags_without_mutating_saved_connection() {
    let saved: Server = serde_json::from_value(json!({
        "host":"server", "root":"/data/colm", "threads":2, "cpus":4,
        "scheduler":"auto", "auth":"config", "username":"researcher", "partition":"cpu"
    }))
    .unwrap();
    let original = saved.clone();
    let effective =
        run_server_overrides(saved.clone(), Some(8), Some(12), Some("slurm".into())).unwrap();
    assert_eq!(saved, original);
    assert_eq!(effective.auth, saved.auth);
    assert_eq!(effective.username, saved.username);
    assert_eq!(effective.partition, saved.partition);
    for dry_run in [false, true] {
        let args = run_args(
            &effective,
            "/case".into(),
            "/kernel".into(),
            None,
            false,
            dry_run,
            &Engine::default(),
        );
        for pair in [
            ["--threads", "8"],
            ["--cpus", "12"],
            ["--scheduler", "slurm"],
        ] {
            assert!(args.windows(2).any(|window| window == pair));
        }
        assert_eq!(args.iter().any(|arg| arg == "--dry-run"), dry_run);
    }
    assert_eq!(
        run_server_overrides(saved.clone(), None, None, None).unwrap(),
        saved
    );
}

#[test]
fn per_run_resources_reject_invalid_overrides_and_keep_saved_automatic_cpus() {
    let saved: Server =
        serde_json::from_value(json!({"host":"server", "root":"/data/colm"})).unwrap();
    for threads in [0, 257] {
        assert!(run_server_overrides(saved.clone(), Some(threads), None, None).is_err());
    }
    for cpus in [0, 1025] {
        assert!(run_server_overrides(saved.clone(), None, Some(cpus), None).is_err());
    }
    for scheduler in ["", "direct", "invalid", "slurm --other"] {
        assert!(run_server_overrides(saved.clone(), None, None, Some(scheduler.into())).is_err());
    }
    for scheduler in ["auto", "bare", "slurm", "pbs", "lsf"] {
        let effective =
            run_server_overrides(saved.clone(), Some(1), None, Some(scheduler.into())).unwrap();
        assert_eq!(effective.cpus, 0);
        assert_eq!(effective.scheduler, scheduler);
    }
}

#[test]
fn data_path_defaults_follow_the_chosen_resource_and_keep_unset_values() {
    let mut config: RemoteConfig =
        serde_json::from_str(r#"{"servers":[{"host":"tms17l","root":"/tera11/zhwei"}]}"#).unwrap();
    assert!(config.local.is_none());
    let local_root = if cfg!(windows) {
        r"D:\Data"
    } else {
        "/Volumes/Data"
    };
    let rawdata = format!("{local_root}/CoLMrawdata");
    let project = format!("{local_root}/projects");
    assert!(remember_data_paths(
        &mut config,
        None,
        [Some(&project), Some(&rawdata), None, Some("")]
    )
    .unwrap());
    let local = config.local.clone().unwrap();
    assert_eq!(
        (local.project_dir.as_str(), local.rawdata_dir.as_str()),
        (project.as_str(), rawdata.as_str())
    );
    assert_eq!(
        (local.runtime_dir.as_str(), local.forcing_dir.as_str()),
        ("", "")
    );
    assert!(remember_data_paths(
        &mut config,
        Some("tms17l"),
        [
            Some("/tera11/zhwei/Personal/CoLM/colm-desktop"),
            Some("/shr03/CoLMrawdata"),
            Some("/shr03/CoLMruntime"),
            Some("/shr03/CoLM_Forcing/JRA3Q"),
        ],
    )
    .unwrap());
    let server = &config.servers[0];
    assert_eq!(
        server.project_dir,
        "/tera11/zhwei/Personal/CoLM/colm-desktop"
    );
    assert_eq!(server.forcing_dir, "/shr03/CoLM_Forcing/JRA3Q");
    // Same values are not a change; unknown servers and relative paths are refused.
    assert!(!remember_data_paths(
        &mut config,
        Some("tms17l"),
        [None, Some("/shr03/CoLMrawdata"), None, None]
    )
    .unwrap());
    assert!(
        remember_data_paths(&mut config, Some("absent"), [Some("/x"), None, None, None]).is_err()
    );
    assert!(remember_data_paths(
        &mut config,
        Some("tms17l"),
        [Some("relative"), None, None, None]
    )
    .is_err());
    assert_eq!(config.local.unwrap().rawdata_dir, rawdata);
}

#[test]
fn kind_paths_are_kept_per_resource_and_kind_and_survive_saving_servers() {
    let mut config: RemoteConfig = serde_json::from_str(
        r#"{"servers":[{"host":"tms17l","root":"/tera11/zhwei"},{"host":"bd","root":"/share"}]}"#,
    )
    .unwrap();
    let site = std::collections::BTreeMap::from([
        (
            "sitedir".to_string(),
            "/shr03/PLUMBER2s/Sitedata".to_string(),
        ),
        (
            "forcingdir".to_string(),
            " /shr03/PLUMBER2s/Forcing ".to_string(),
        ),
        ("mesh".to_string(), "".to_string()),
    ]);
    assert!(remember_kind_paths(&mut config, Some("tms17l"), "site", &site).unwrap());
    assert!(!remember_kind_paths(&mut config, Some("tms17l"), "site", &site).unwrap());
    let watershed = std::collections::BTreeMap::from([
        (
            "shapefile".to_string(),
            "/shr03/basins/heihe.shp".to_string(),
        ),
        ("forcing_dataset".to_string(), "JRA3Q".to_string()),
    ]);
    assert!(remember_kind_paths(&mut config, Some("tms17l"), "watershed", &watershed).unwrap());
    let saved = &config.servers[0].kind_paths;
    assert_eq!(saved["site"]["forcingdir"], "/shr03/PLUMBER2s/Forcing");
    assert!(
        !saved["site"].contains_key("mesh"),
        "empty values are not remembered"
    );
    assert_eq!(saved["watershed"]["forcing_dataset"], "JRA3Q");
    assert!(
        config.servers[1].kind_paths.is_empty(),
        "other servers untouched"
    );

    // Unknown kinds or fields, relative paths and unknown servers are refused.
    let bad = |key: &str, value: &str| {
        std::collections::BTreeMap::from([(key.to_string(), value.to_string())])
    };
    assert!(
        remember_kind_paths(&mut config, Some("tms17l"), "planet", &bad("mesh", "/m")).is_err()
    );
    assert!(
        remember_kind_paths(&mut config, Some("tms17l"), "region", &bad("root", "/r")).is_err()
    );
    assert!(remember_kind_paths(
        &mut config,
        Some("tms17l"),
        "region",
        &bad("mesh", "mesh.nc")
    )
    .is_err());
    assert!(
        remember_kind_paths(&mut config, Some("absent"), "region", &bad("mesh", "/m")).is_err()
    );

    // Saving the server list from an editor that loaded an older copy keeps the paths.
    let mut edited: RemoteConfig = serde_json::from_str(
        r#"{"servers":[{"host":"tms17l","root":"/tera11/zhwei","threads":16},{"host":"new","root":"/n"}]}"#,
    )
    .unwrap();
    keep_kind_paths(&mut edited, config.clone());
    assert_eq!(edited.servers[0].threads, 16);
    assert_eq!(edited.servers[0].kind_paths, config.servers[0].kind_paths);
    assert!(edited.servers[1].kind_paths.is_empty());

    // Local paths live under `local` and are kept when only servers are saved.
    let local_dir = if cfg!(windows) {
        r"D:\sites"
    } else {
        "/Volumes/Data/sites"
    };
    assert!(remember_kind_paths(&mut config, None, "site", &bad("sitedir", local_dir)).unwrap());
    let mut servers_only = RemoteConfig {
        servers: config.servers.clone(),
        local: None,
    };
    keep_kind_paths(&mut servers_only, config.clone());
    assert_eq!(
        servers_only.local.unwrap().kind_paths["site"]["sitedir"],
        local_dir
    );

    assert!(forget_kind_path(&mut config, Some("tms17l"), "site", Some("sitedir")).unwrap());
    assert!(!config.servers[0].kind_paths["site"].contains_key("sitedir"));
    assert!(forget_kind_path(&mut config, Some("tms17l"), "site", Some("forcingdir")).unwrap());
    assert!(
        !config.servers[0].kind_paths.contains_key("site"),
        "empty kinds are dropped"
    );
    assert!(forget_kind_path(&mut config, Some("tms17l"), "watershed", None).unwrap());
    assert!(config.servers[0].kind_paths.is_empty());
    assert!(!serde_json::to_string(&config.servers[0])
        .unwrap()
        .contains("kind_paths"));
}

#[test]
fn a_server_run_is_read_on_the_server_until_its_results_are_fetched() {
    let run = std::env::temp_dir().join(format!("colm-results-on-server-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&run);
    std::fs::create_dir_all(&run).unwrap();
    let case = run.to_string_lossy().into_owned();
    assert_eq!(results_on_server(&case), None, "a local run");
    std::fs::write(
        run.join(".colm-remote.json"),
        r#"{"host":"tianhe","job":"run-1"}"#,
    )
    .unwrap();
    assert_eq!(results_on_server(&case).as_deref(), Some("tianhe"));
    std::fs::create_dir_all(run.join("out/AU-Cpr/history")).unwrap();
    std::fs::write(run.join("out/AU-Cpr/history/AU-Cpr_hist_2011-01.nc"), "").unwrap();
    assert_eq!(
        results_on_server(&case),
        None,
        "fetched results are read locally"
    );
    std::fs::remove_dir_all(&run).unwrap();
}

#[test]
fn a_resent_station_submission_waits_for_the_first_instead_of_submitting_again() {
    let key = format!("batch-{}\n/case/AU-Cpr", std::process::id());
    let (first, submits) = claim_submission(&key);
    assert!(submits);
    let (again, resubmits) = claim_submission(&key);
    assert!(!resubmits, "the resent request does not submit");
    let waiter = std::thread::spawn(move || wait_for_submission(&again));
    std::thread::sleep(std::time::Duration::from_millis(20));
    settle_submission(&first, &Ok(serde_json::json!({"job": "run-1"})));
    assert_eq!(waiter.join().unwrap().unwrap()["job"], "run-1");
    // A request arriving after the first finished gets the same answer, not a second job.
    let (late, submits_late) = claim_submission(&key);
    assert!(!submits_late);
    assert_eq!(wait_for_submission(&late).unwrap()["job"], "run-1");
    // Another station of the batch is its own submission.
    assert!(claim_submission(&format!("batch-{}\n/case/AU-How", std::process::id())).1);
}

#[test]
fn an_interrupted_first_submission_still_answers_the_resent_request() {
    let key = format!("batch-interrupted-{}\n/case", std::process::id());
    let (first, submits) = claim_submission(&key);
    assert!(submits);
    let (again, resubmits) = claim_submission(&key);
    assert!(!resubmits);
    let waiter = std::thread::spawn(move || wait_for_submission(&again));
    // The first request's task goes away (panic or dropped future) before it settles.
    drop(SettleOnDrop {
        outcome: first,
        settled: false,
    });
    let answer = waiter.join().unwrap();
    assert!(answer.unwrap_err().contains("interrupted"));
}

// 评估在哪算：结果取回了、但观测只在服务器上（服务器项目的默认观测是服务器路径），仍到服务器上算。
#[test]
fn evaluation_runs_where_both_the_results_and_the_observation_are() {
    let dir = std::env::temp_dir().join(format!("colm-evaluation-server-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.to_str().unwrap();
    let local_obs = dir.join("obs.nc");
    std::fs::write(&local_obs, "").unwrap();
    let server_obs = "/srv/data/Observation/US-MMS_Flux.nc";
    assert_eq!(evaluation_server(case, server_obs), None, "a local run");
    std::fs::write(dir.join(".colm-remote.json"), r#"{"host":"7920"}"#).unwrap();
    // 没有本地 history，结果在服务器上。
    assert_eq!(
        evaluation_server(case, local_obs.to_str().unwrap()).as_deref(),
        Some("7920")
    );
    assert_eq!(evaluation_server(case, server_obs).as_deref(), Some("7920"));
    // 结果已取回本机：观测在本机就本机算，观测只在服务器上就还去服务器算。
    std::fs::create_dir_all(dir.join("out/US-MMS/history")).unwrap();
    std::fs::write(dir.join("out/US-MMS/history/h.nc"), "").unwrap();
    assert_eq!(evaluation_server(case, local_obs.to_str().unwrap()), None);
    assert_eq!(evaluation_server(case, server_obs).as_deref(), Some("7920"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn server_cases_keep_non_sensitive_configuration_and_drop_credentials() {
    let server: Server = serde_json::from_value(json!({
        "host":"bd",
        "root":"/tera11/zhwei/CoLM",
        "username":"zhwei",
        "auth":"key",
        "identity_file":"/home/zhwei/.ssh/id_ed25519",
        "scheduler":"lsf",
        "partition":"normal",
        "cores_per_node":48,
        "project_dir":"/tera11/zhwei/projects",
        "kind_paths":{"site":{"sitedir":"/tera11/zhwei/sites"}}
    }))
    .unwrap();
    let saved = case_from_server(server, "  摆渡船（LSF） ").unwrap();
    // 认证方式与私钥属于本机凭据，不能进案例；其余配置默认带上去。
    assert_eq!(saved.auth, "config");
    assert_eq!(saved.identity_file, "");
    assert_eq!(saved.host, "bd");
    assert_eq!(saved.username, "zhwei");
    assert_eq!(saved.scheduler, "lsf");
    assert_eq!(saved.partition, "normal");
    assert_eq!(saved.cores_per_node, 48);
    assert_eq!(saved.project_dir, "/tera11/zhwei/projects");
    assert!(saved.kind_paths.is_empty());
    assert!(case_from_server(saved, "  ").is_err());
}

#[test]
fn server_cases_file_roundtrips_and_updates_by_name() {
    let dir = std::env::temp_dir().join(format!(
        "colm-gui-cases-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("server-cases.json");
    assert!(read_server_cases(&path).unwrap().cases.is_empty());
    let server: Server =
        serde_json::from_value(json!({"host":"tms17l","root":"/tera11/zhwei/Personal/CoLM"}))
            .unwrap();
    let mut file = ServerCaseFile::default();
    upsert_case(
        &mut file,
        ServerCase {
            id: "case-1".into(),
            name: "天河".into(),
            server: server.clone(),
            saved_at: 1,
        },
    );
    upsert_case(
        &mut file,
        ServerCase {
            id: "case-2".into(),
            name: "摆渡船".into(),
            server: server.clone(),
            saved_at: 2,
        },
    );
    upsert_case(
        &mut file,
        ServerCase {
            id: "case-3".into(),
            name: "天河".into(),
            server: server.clone(),
            saved_at: 3,
        },
    );
    assert_eq!(
        file.cases.len(),
        2,
        "same name replaces instead of piling up"
    );
    assert_eq!(
        file.cases.iter().find(|c| c.name == "天河").unwrap().id,
        "case-3"
    );
    write_server_cases(&path, &file).unwrap();
    assert_eq!(read_server_cases(&path).unwrap().cases, file.cases);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- P1-9 B: 远程研究失败状态机与可执行下一步 ------------------------------------------------

fn remote_record(fetched: bool) -> Value {
    json!({
        "host": "server",
        "root": "/data/colm",
        "job": "study-1",
        "remote_project": "/data/colm/s/abc",
        "fetched_at": if fetched { json!(1_700_000_000u64) } else { Value::Null },
    })
}

fn study_status_reply(record: Value, status: Value, study: Value) -> Value {
    json!({"record": record, "status": status, "study": study})
}

fn finished(exit_code: i32, phase: Value) -> Value {
    json!({"state": "finished", "exit_code": exit_code, "phase": phase, "log_tail": "log"})
}

#[test]
fn a_build_failure_without_a_study_directory_points_at_the_log_and_close() {
    let reply = study_status_reply(
        remote_record(false),
        finished(1, json!("preparing the engine")),
        Value::Null,
    );
    let recovery = remote_study_recovery(&reply);
    assert_eq!(recovery.outcome, RemoteStudyOutcome::BuildFailed);
    assert!(recovery.terminal && recovery.finished && !recovery.study_created);
    // 构建失败：可以看日志、归档关闭、修复后重提；取回仍然允许点击，但会直接给可行动说明。
    assert!(recovery.permissions.fetch);
    assert!(recovery.permissions.close);
    assert!(recovery.permissions.resubmit);
    assert!(!recovery.permissions.submit);
    assert!(!recovery.permissions.cancel);
    let message = fetch_blocked_message(&recovery).expect("fetch explains why it cannot work");
    assert!(message.contains("没有研究目录"), "{message}");
    assert!(message.contains("no Study directory"), "{message}");
    assert!(
        message.contains("日志") && message.contains("log tail"),
        "{message}"
    );
    assert!(message.contains("归档关闭"), "{message}");
    assert!(recovery.next_zh.contains("构建") && recovery.next_en.contains("build"));
}

#[test]
fn early_failures_are_split_by_the_phase_the_job_reached() {
    let creation = remote_study_recovery(&study_status_reply(
        remote_record(false),
        finished(2, json!("creating the Study")),
        Value::Null,
    ));
    assert_eq!(creation.outcome, RemoteStudyOutcome::StudyCreateFailed);
    assert!(!creation.study_created);

    // 阶段标记都没写：作业体在写第一个标记前就退出，属于启动/构建准备阶段。
    let startup = remote_study_recovery(&study_status_reply(
        remote_record(false),
        finished(127, Value::Null),
        Value::Null,
    ));
    assert_eq!(startup.outcome, RemoteStudyOutcome::StartFailed);
    assert!(fetch_blocked_message(&startup).unwrap().contains("启动"));

    // 成员运行失败时研究目录已经存在：取回是有意义的诊断动作，不能再给“没有研究目录”。
    let run = remote_study_recovery(&study_status_reply(
        remote_record(false),
        finished(3, json!("running the Study")),
        json!({"status": "running"}),
    ));
    assert_eq!(run.outcome, RemoteStudyOutcome::MemberRunFailed);
    assert!(run.study_created);
    assert!(run.permissions.fetch && run.permissions.close && run.permissions.resubmit);
    assert!(fetch_blocked_message(&run).is_none());

    // 旧作业没有 phase：有研究目录按成员失败，没有目录按启动前的失败。
    let legacy_with_study = remote_study_recovery(&study_status_reply(
        remote_record(false),
        finished(1, Value::Null),
        json!({"status": "failed"}),
    ));
    assert_eq!(
        legacy_with_study.outcome,
        RemoteStudyOutcome::MemberRunFailed
    );
    let legacy_without_study = remote_study_recovery(&study_status_reply(
        remote_record(false),
        finished(1, Value::Null),
        Value::Null,
    ));
    assert_eq!(
        legacy_without_study.outcome,
        RemoteStudyOutcome::StartFailed
    );
}

#[test]
fn a_successful_run_must_be_fetched_before_its_record_can_be_archived() {
    let reply = study_status_reply(
        remote_record(false),
        finished(0, json!("running the Study")),
        json!({"status": "completed"}),
    );
    let recovery = remote_study_recovery(&reply);
    assert_eq!(recovery.outcome, RemoteStudyOutcome::Succeeded);
    assert!(recovery.terminal && recovery.study_created);
    assert!(recovery.permissions.fetch);
    assert!(
        !recovery.permissions.close,
        "successful results must be fetched first"
    );
    assert!(!recovery.permissions.resubmit);
    assert!(close_blocked_message(&recovery).contains("先取回结果"));

    // 取回之后记录才可以归档。
    let fetched = remote_study_recovery(&study_status_reply(
        remote_record(true),
        finished(0, json!("running the Study")),
        json!({"status": "completed"}),
    ));
    assert!(fetched.permissions.close);
    assert!(!fetched.permissions.fetch);
}

#[test]
fn cancelled_and_lost_terminals_stay_archivable() {
    let cancelled = remote_study_recovery(&study_status_reply(
        remote_record(false),
        json!({"state":"lost","detail":"CANCELLED by request (exit 1)","phase":"running the Study","log_tail":""}),
        Value::Null,
    ));
    assert_eq!(cancelled.outcome, RemoteStudyOutcome::Cancelled);
    assert!(!cancelled.terminal);
    assert!(cancelled.permissions.close);
    assert!(!cancelled.permissions.fetch && !cancelled.permissions.poll);
    assert!(
        !cancelled.permissions.resubmit,
        "cancelled must be closed explicitly"
    );

    let lost = remote_study_recovery(&study_status_reply(
        remote_record(false),
        json!({"state":"lost","phase":"running the Study","log_tail":""}),
        Value::Null,
    ));
    assert_eq!(lost.outcome, RemoteStudyOutcome::Lost);
    assert!(lost.permissions.close);
    assert!(!lost.permissions.poll);

    let unknown = remote_study_recovery(&study_status_reply(
        remote_record(false),
        json!({"state":"unknown","log_tail":""}),
        Value::Null,
    ));
    assert_eq!(unknown.outcome, RemoteStudyOutcome::Unknown);
    assert!(unknown.permissions.close);
    assert!(
        unknown.permissions.poll,
        "an unconfirmed state keeps polling"
    );
}

#[test]
fn a_live_server_job_can_neither_be_archived_nor_resubmitted() {
    for state in ["queued", "running"] {
        let reply = study_status_reply(
            remote_record(false),
            json!({"state": state, "phase": "running the Study", "log_tail": ""}),
            Value::Null,
        );
        let recovery = remote_study_recovery(&reply);
        assert_eq!(recovery.outcome, RemoteStudyOutcome::Running);
        assert!(!recovery.permissions.close, "{state} must not be archived");
        assert!(
            !recovery.permissions.resubmit,
            "{state} must not be resubmitted"
        );
        assert!(!recovery.permissions.submit);
        assert!(recovery.permissions.cancel, "{state} can be cancelled");
        assert!(recovery.permissions.poll);
        assert!(close_blocked_message(&recovery).contains("取消"));
    }
    // 提交意图还在、作业查不到：不能重提（可能重复提交），但用户确认后可以关闭。
    let pending = json!({"record": Value::Null, "pending": {"host":"server","root":"/data/colm","job":"study-1"}, "status": {"state":"unknown","log_tail":""}, "study": Value::Null});
    let recovery = remote_study_recovery(&pending);
    assert_eq!(recovery.outcome, RemoteStudyOutcome::Unknown);
    assert!(!recovery.permissions.submit && !recovery.permissions.resubmit);
    assert!(recovery.permissions.close);
}

#[test]
fn a_never_submitted_study_can_be_submitted_again() {
    let reply = json!({"record": Value::Null, "pending": Value::Null, "outcome": "never_submitted", "status": {"state":"unknown"}, "study": Value::Null});
    let recovery = remote_study_recovery(&reply);
    assert_eq!(recovery.outcome, RemoteStudyOutcome::NotSubmitted);
    assert!(recovery.permissions.submit);
    assert!(!recovery.permissions.close && !recovery.permissions.fetch);
    assert!(recovery.next_en.contains("submit"));
}

#[test]
fn every_outcome_has_a_bilingual_next_step() {
    let cases = [
        json!({"record": Value::Null, "pending": Value::Null, "outcome":"never_submitted", "status": {"state":"unknown"}, "study": Value::Null}),
        study_status_reply(remote_record(false), finished(1, Value::Null), Value::Null),
        study_status_reply(
            remote_record(false),
            finished(1, json!("preparing the engine")),
            Value::Null,
        ),
        study_status_reply(
            remote_record(false),
            finished(1, json!("creating the Study")),
            Value::Null,
        ),
        study_status_reply(
            remote_record(false),
            finished(1, json!("running the Study")),
            json!({"status":"failed"}),
        ),
        study_status_reply(
            remote_record(false),
            finished(0, json!("running the Study")),
            json!({"status":"completed"}),
        ),
        study_status_reply(
            remote_record(false),
            json!({"state":"lost","detail":"CANCELLED by request (exit 1)"}),
            Value::Null,
        ),
        study_status_reply(remote_record(false), json!({"state":"lost"}), Value::Null),
        study_status_reply(
            remote_record(false),
            json!({"state":"running"}),
            Value::Null,
        ),
        study_status_reply(
            remote_record(false),
            json!({"state":"unknown"}),
            Value::Null,
        ),
    ];
    for reply in cases {
        let recovery = remote_study_recovery(&reply);
        assert!(
            !recovery.next_zh.trim().is_empty(),
            "{:?}",
            recovery.outcome
        );
        assert!(
            !recovery.next_en.trim().is_empty(),
            "{:?}",
            recovery.outcome
        );
        assert!(recovery.next_en.is_ascii(), "{:?}", recovery.outcome);
    }
}

#[test]
fn status_replies_carry_the_recovery_classification() {
    let mut reply = study_status_reply(
        remote_record(false),
        finished(1, json!("creating the Study")),
        Value::Null,
    );
    let recovery = attach_study_recovery(&mut reply);
    assert_eq!(reply["recovery"]["outcome"], "study_create_failed");
    assert_eq!(reply["recovery"]["permissions"]["resubmit"], true);
    assert_eq!(reply["recovery"]["permissions"]["close"], true);
    assert_eq!(
        reply["recovery"]["fetch_error_zh"],
        serde_json::to_value(&recovery.fetch_error_zh).unwrap()
    );
}

#[test]
fn archiving_a_confirmed_failed_record_renames_it_without_losing_data() {
    let dir = std::env::temp_dir().join(format!(
        "colm-remote-archive-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let local = remote_record(false);
    std::fs::write(
        dir.join(".colm-remote-study.json"),
        serde_json::to_vec_pretty(&local).unwrap(),
    )
    .unwrap();
    let mut status = study_status_reply(
        remote_record(false),
        finished(1, json!("creating the Study")),
        Value::Null,
    );
    attach_study_recovery(&mut status);

    verify_local_record_matches_status(dir.to_str().unwrap(), &status).unwrap();
    let archived = archive_study_record(dir.to_str().unwrap()).unwrap();
    assert_eq!(archived.len(), 1);
    assert!(!dir.join(".colm-remote-study.json").exists());
    let target = std::path::Path::new(&archived[0]);
    assert!(target.is_file());
    let name = target.file_name().unwrap().to_string_lossy();
    // 与 CLI `remote-study-close` 的留档命名一致，两个工具留下的文件可以互认。
    assert!(
        name.starts_with(".colm-remote-study-closed-") && name.ends_with(".json"),
        "{name}"
    );
    let kept: Value = serde_json::from_slice(&std::fs::read(target).unwrap()).unwrap();
    assert_eq!(kept["job"], local["job"]);
    assert_eq!(kept["root"], local["root"]);

    // 归档后记录不再挡路：从未提交的状态可以直接重新提交。
    let recovery = remote_study_recovery(
        &json!({"record": Value::Null, "pending": Value::Null, "outcome": "never_submitted", "status": {"state":"unknown"}, "study": Value::Null}),
    );
    assert!(recovery.permissions.submit);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn archiving_refuses_a_status_that_verifies_another_job() {
    let dir = std::env::temp_dir().join(format!(
        "colm-remote-archive-mismatch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join(".colm-remote-study.json"),
        serde_json::to_vec_pretty(&remote_record(false)).unwrap(),
    )
    .unwrap();
    let mut other = remote_record(false);
    other["job"] = json!("study-2");
    let status = study_status_reply(other, finished(1, json!("creating the Study")), Value::Null);
    let error = verify_local_record_matches_status(dir.to_str().unwrap(), &status).unwrap_err();
    assert!(error.contains("对不上"), "{error}");
    assert!(
        dir.join(".colm-remote-study.json").is_file(),
        "nothing was archived"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// 正常率定/不确定性研究（运行中、排队、成功）的旧权限矩阵不能因为失败状态机而收紧。
#[test]
fn the_permission_matrix_for_normal_studies_is_unchanged() {
    let running = remote_study_recovery(&study_status_reply(
        remote_record(false),
        json!({"state":"running"}),
        json!({"status":"running"}),
    ));
    assert!(running.permissions.cancel && running.permissions.poll);
    assert!(!running.permissions.fetch && !running.permissions.close);

    let queued = remote_study_recovery(&study_status_reply(
        remote_record(false),
        json!({"state":"queued"}),
        Value::Null,
    ));
    assert!(queued.permissions.cancel);

    let succeeded = remote_study_recovery(&study_status_reply(
        remote_record(false),
        finished(0, json!("running the Study")),
        json!({"status":"completed"}),
    ));
    assert!(
        succeeded.permissions.fetch && !succeeded.permissions.cancel && !succeeded.permissions.poll
    );

    let fetched = remote_study_recovery(&study_status_reply(
        remote_record(true),
        finished(0, json!("running the Study")),
        json!({"status":"completed"}),
    ));
    assert!(!fetched.permissions.fetch && fetched.permissions.close);

    // 没有远程记录时 submit 照旧放行。
    let local_only = remote_study_recovery(
        &json!({"record": Value::Null, "pending": Value::Null, "status": Value::Null, "study": Value::Null}),
    );
    assert!(local_only.permissions.submit);
    assert!(!local_only.permissions.fetch);
}
