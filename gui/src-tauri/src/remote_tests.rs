use super::*;

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
        maps: vec![PathMap {
            local: "/Volumes/Data/Data/PLUMBER2s".into(),
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
        fetch_args("/c".into(), Some(&server), false),
        ["remote-fetch", "/c"]
    );
    server.fetch_vars = " f_fsena,f_rnet ".into();
    assert_eq!(
        fetch_args("/c".into(), Some(&server), false),
        ["remote-fetch", "/c", "--vars", "f_fsena,f_rnet"]
    );
    // 要全部：不加 --vars。找不到服务器配置时也一样。
    assert_eq!(
        fetch_args("/c".into(), Some(&server), true),
        ["remote-fetch", "/c"]
    );
    assert_eq!(fetch_args("/c".into(), None, false), ["remote-fetch", "/c"]);
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
