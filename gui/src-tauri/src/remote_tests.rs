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
        fetch_vars: String::new(),
        env_script: String::new(),
        directives: Vec::new(),
    };
    let ok = RemoteConfig {
        servers: vec![server("7920land", "/media/zhwei/data02/zhwei/colm-desktop")],
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
                servers: vec![server(host, root)]
            })
            .is_err(),
            "{host} {root}"
        );
    }
    let mut bad_map = server("h", "/a");
    bad_map.maps[0].remote = "relative".into();
    assert!(validate(&RemoteConfig {
        servers: vec![bad_map]
    })
    .is_err());
    let mut no_threads = server("h", "/a");
    no_threads.threads = 0;
    assert!(validate(&RemoteConfig {
        servers: vec![no_threads]
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
                servers: vec![server.clone()]
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
fn connection_options_reject_invalid_modes_users_and_relative_keys() {
    let mut server: Server = serde_json::from_str(r#"{"host":"c1","root":"/data/colm"}"#).unwrap();
    for user in ["-root", "alice@host", "alice bob", "alice\nroot"] {
        server.username = user.into();
        assert!(validate(&RemoteConfig {
            servers: vec![server.clone()]
        })
        .is_err());
    }
    server.username = "alice".into();
    server.auth = "unknown".into();
    assert!(validate(&RemoteConfig {
        servers: vec![server.clone()]
    })
    .is_err());
    server.auth = "key".into();
    server.identity_file = "relative-key".into();
    assert!(validate(&RemoteConfig {
        servers: vec![server.clone()]
    })
    .is_err());
    server.identity_file.clear();
    assert!(validate(&RemoteConfig {
        servers: vec![server]
    })
    .is_ok());
}

#[test]
fn saved_jobs_keep_their_original_login_when_a_profile_changes() {
    let changed: Server = serde_json::from_value(json!({"host":"c1", "root":"/data/new", "auth":"key", "username":"bob", "port":22, "fetch_vars":"f_rnet"})).unwrap();
    let config = RemoteConfig {
        servers: vec![changed],
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
        servers: vec![server.clone()]
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
            servers: vec![server.clone()]
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
