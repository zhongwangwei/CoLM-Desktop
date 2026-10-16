use super::*;

#[test]
fn automatic_scheduler_does_not_treat_unverified_qsub_as_a_bare_server() {
    let mut probe = colm_remote::probe::Probe::default();
    assert_eq!(detected_scheduler(&probe).unwrap(), Scheduler::Bare);
    probe.qsub_unverified = true;
    assert!(detected_scheduler(&probe)
        .unwrap_err()
        .to_string()
        .contains("qsub"));
    probe.schedulers = vec!["lsf".into()];
    assert_eq!(detected_scheduler(&probe).unwrap(), Scheduler::Lsf);
    probe.qsub_unverified = false;
    probe.schedulers = vec!["pbs".into(), "lsf".into()];
    assert_eq!(detected_scheduler(&probe).unwrap(), Scheduler::Pbs);
}

// 映射与算例路径用的是 Unix 本机路径（Windows 上不是绝对路径，`parse_mapping` 会拒绝）。
#[cfg(unix)]
fn maps() -> Vec<Mapping> {
    vec![
        parse_mapping(
            "/Volumes/Data/Data/PLUMBER2s=/media/zhwei/data02/zhwei/training2026/PLUMBER2s",
        )
        .unwrap(),
        parse_mapping("/Volumes/Data/Data=/media/zhwei/data02/zhwei").unwrap(),
    ]
}

#[cfg(unix)]
#[test]
fn paths_go_into_the_case_copy_or_through_the_longest_mapping() {
    let case = Path::new("/Users/me/Desktop/CA-Qfo-pc");
    let remote = "/srv/colm/cases/CA-Qfo-pc-1234abcd";
    // 算例目录内：换成副本目录，目录值保留结尾的 `/`。
    assert_eq!(
        place("/Users/me/Desktop/CA-Qfo-pc/site.nc", case, remote, &maps()),
        Placed::Remote(format!("{remote}/site.nc"))
    );
    assert_eq!(
        place(
            "/Users/me/Desktop/CA-Qfo-pc/rawdata_unused/",
            case,
            remote,
            &maps()
        ),
        Placed::Remote(format!("{remote}/rawdata_unused/"))
    );
    assert_eq!(
        place("site.nc", case, remote, &maps()),
        Placed::Remote(format!("{remote}/site.nc"))
    );
    // 更具体的对应优先。
    assert_eq!(
        place(
            "/Volumes/Data/Data/PLUMBER2s/Forcing/",
            case,
            remote,
            &maps()
        ),
        Placed::Remote("/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Forcing/".into())
    );
    assert_eq!(
        place("/Volumes/Data/Data/CoLMrawdata/", case, remote, &maps()),
        Placed::Remote("/media/zhwei/data02/zhwei/CoLMrawdata/".into())
    );
    // 按路径分量比：/Volumes/Data/DataX 不在 /Volumes/Data/Data 之下。
    assert_eq!(
        place("/Volumes/Data/DataX/f.nc", case, remote, &maps()),
        Placed::Keep
    );
    // 对应不上、本机存在：报出来。
    let here = std::env::temp_dir();
    assert!(matches!(
        place(&here.to_string_lossy(), case, remote, &maps()),
        Placed::Unmapped(_)
    ));
    for bad in ["relative=/x", "/a", "/a=relative"] {
        assert!(parse_mapping(bad).is_err(), "{bad}");
    }
}

#[cfg(unix)]
#[test]
fn staging_copies_inputs_only_and_rewrites_every_path() {
    let root = std::env::temp_dir().join(format!("colm-remote-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let case = root.join("CA-Qfo-pc");
    std::fs::create_dir_all(case.join("out/CA-Qfo-pc/history")).unwrap();
    std::fs::write(case.join("out/CA-Qfo-pc/history/h.nc"), "big").unwrap();
    std::fs::write(case.join("colm.log"), "log").unwrap();
    std::fs::write(case.join("stages.json"), "{}").unwrap();
    std::fs::write(case.join("site.nc"), "site").unwrap();
    let c = case.display();
    std::fs::write(
        case.join("case.nml"),
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'CA-Qfo-pc'\n SITE_fsitedata = '{c}/site.nc'\n DEF_dir_rawdata = '{c}/rawdata_unused/'\n DEF_dir_output = '{c}/out/'\n DEF_forcing_namelist = '{c}/forcing.nml'\n DEF_simulation_time%start_year = 2004\n/\n"
        ),
    )
    .unwrap();
    std::fs::write(
        case.join("forcing.nml"),
        "&nl_colm_forcing\n DEF_dir_forcing = '/Volumes/Data/Data/PLUMBER2s/Forcing/'\n DEF_forcing%fprefix(1) = 'CA-Qfo_2004-2010_FLUXNET2015_Met.nc'\n/\n",
    )
    .unwrap();
    let staging = root.join("staging");
    let remote = "/srv/colm/cases/CA-Qfo-pc-1234abcd";
    let staged = stage_case(&case, &staging, remote, &maps(), false).unwrap();
    // 输出、日志、阶段指纹不随算例上传。
    assert!(
        !staging.join("out").exists()
            && !staging.join("colm.log").exists()
            && !staging.join("stages.json").exists()
    );
    assert!(staging.join("site.nc").is_file());
    let case_nml = std::fs::read_to_string(staging.join("case.nml")).unwrap();
    assert!(
        case_nml.contains(&format!("SITE_fsitedata = '{remote}/site.nc'")),
        "{case_nml}"
    );
    assert!(
        case_nml.contains(&format!("DEF_dir_output = '{remote}/out/'")),
        "{case_nml}"
    );
    assert!(case_nml.contains(&format!("DEF_forcing_namelist = '{remote}/forcing.nml'")));
    assert!(case_nml.contains(&format!("DEF_dir_rawdata = '{remote}/rawdata_unused/'")));
    assert!(case_nml.contains("start_year = 2004"));
    let forcing = std::fs::read_to_string(staging.join("forcing.nml")).unwrap();
    assert!(
        forcing.contains(
            "DEF_dir_forcing = '/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Forcing/'"
        ),
        "{forcing}"
    );
    assert!(
        forcing.contains("fprefix(1) = 'CA-Qfo_2004-2010_FLUXNET2015_Met.nc'"),
        "file-name prefixes stay as they are"
    );
    // 指向服务器数据的路径交给提交前核对。
    assert_eq!(
        staged.remote_inputs,
        ["/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Forcing/"]
    );
    assert!(staged.unmapped.is_empty());

    // 对应不上的本机文件：默认报出来；允许上传时随算例一起传。
    let outside = root.join("extra.nc");
    std::fs::write(&outside, "x").unwrap();
    let nml = std::fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace(
            " DEF_simulation_time",
            &format!(
                " DEF_file_SoilInit = '{}'\n DEF_simulation_time",
                outside.display()
            ),
        );
    std::fs::write(case.join("case.nml"), nml).unwrap();
    let _ = std::fs::remove_dir_all(&staging);
    assert_eq!(
        stage_case(&case, &staging, remote, &maps(), false)
            .unwrap()
            .unmapped,
        std::slice::from_ref(&outside)
    );
    let _ = std::fs::remove_dir_all(&staging);
    let uploaded = stage_case(&case, &staging, remote, &maps(), true).unwrap();
    assert_eq!(uploaded.uploaded_extra, [outside]);
    assert!(staging.join("inputs/extra.nc").is_file());
    assert!(std::fs::read_to_string(staging.join("case.nml"))
        .unwrap()
        .contains(&format!("'{remote}/inputs/extra.nc'")));
    let _ = std::fs::remove_dir_all(&root);
}

fn opts(args: &[&str]) -> Opts {
    Opts::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap()
}

#[test]
fn the_run_plan_defaults_to_rust_and_only_fortran_may_use_mpi() {
    let plan = RunPlan::from_opts(&opts(&[])).unwrap();
    assert_eq!(
        (plan.engine.as_str(), plan.ranks, plan.nodes),
        ("rust", 1, 0)
    );
    assert_eq!(plan.preprocessors, "rust");
    let plan = RunPlan::from_opts(&opts(&[
        "--engine", "fortran", "--ranks", "8", "--nodes", "2",
    ]))
    .unwrap();
    assert_eq!((plan.ranks, plan.nodes), (8, 2));
    assert!(
        RunPlan::from_opts(&opts(&["--ranks", "4"])).is_err(),
        "the Rust engine uses threads"
    );
    assert!(RunPlan::from_opts(&opts(&["--engine", "fortran", "--ranks", "0"])).is_err());
    assert!(RunPlan::from_opts(&opts(&["--engine", "cobol"])).is_err());
    assert!(RunPlan::from_opts(&opts(&["--launcher", "ssh"])).is_err());
}

#[test]
fn the_job_body_launches_mpi_through_srun_only_where_that_fits() {
    let body = |args: &[&str], scheduler| {
        let plan = RunPlan::from_opts(&opts(args)).unwrap();
        job_body(
            "/r",
            "eng",
            "/r/cases/c",
            "/r/kernels/k",
            4,
            &plan,
            scheduler,
            &opts(args),
        )
        .unwrap()
    };
    let slurm = body(&["--engine", "fortran", "--ranks", "8"], Scheduler::Slurm);
    assert!(slurm.contains("export COLM_MPIEXEC=srun\n"), "{slurm}");
    assert!(
        slurm.contains("--engine fortran --preprocessors rust --ranks 8"),
        "{slurm}"
    );
    // 没有调度系统或别的调度系统：用 PATH 里的 mpiexec，不设变量。
    let bare = body(&["--engine", "fortran", "--ranks", "8"], Scheduler::Bare);
    assert!(
        !bare.contains("COLM_MPIEXEC") && bare.contains("--ranks 8"),
        "{bare}"
    );
    // 显式选择优先于调度系统。
    let forced = body(
        &[
            "--engine",
            "fortran",
            "--ranks",
            "8",
            "--launcher",
            "mpiexec",
        ],
        Scheduler::Slurm,
    );
    assert!(!forced.contains("COLM_MPIEXEC"));
    let srun_bare = body(
        &["--engine", "fortran", "--ranks", "8", "--launcher", "srun"],
        Scheduler::Bare,
    );
    assert!(srun_bare.contains("COLM_MPIEXEC=srun"));
    // 一个进程不用 MPI，也就不写启动器和 --ranks。
    let one = body(&["--engine", "fortran"], Scheduler::Slurm);
    assert!(
        !one.contains("COLM_MPIEXEC") && !one.contains("--ranks"),
        "{one}"
    );
    // Rust 引擎的作业体不变。
    let rust = body(&[], Scheduler::Slurm);
    assert!(rust.contains("--engine rust --preprocessors rust") && !rust.contains("--ranks"));
    assert!(rust.contains(" --location server"), "{rust}");
    // 编引擎的并行度至少 8，与运行线程数无关。
    assert!(rust.contains("-j 8"), "{rust}");
}

#[test]
fn source_digest_tracks_bytes_and_paths_not_metadata_or_creation_order() {
    let root = std::env::temp_dir().join(format!("colm-content-hash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in ["a", "b"] {
        std::fs::create_dir_all(root.join(name).join("crates/demo/src")).unwrap();
        std::fs::write(root.join(name).join("Cargo.toml"), "[workspace]\n").unwrap();
    }
    for (name, order) in [("a", ["a.rs", "b.rs"]), ("b", ["b.rs", "a.rs"])] {
        for file in order {
            std::fs::write(root.join(name).join("crates/demo/src").join(file), file).unwrap();
        }
    }
    let a = root.join("a");
    let first = source_content_sha256(&a).unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(first, source_content_sha256(&root.join("b")).unwrap());
    std::fs::write(a.join("crates/demo/src/a.rs"), "a.rs").unwrap();
    assert_eq!(first, source_content_sha256(&a).unwrap());
    std::fs::create_dir_all(a.join("crates/demo/target")).unwrap();
    std::fs::write(a.join("crates/demo/target/generated.rs"), "ignored").unwrap();
    std::fs::write(a.join("crates/demo/data.nc"), "ignored").unwrap();
    assert_eq!(first, source_content_sha256(&a).unwrap());
    std::fs::write(a.join("crates/demo/src/a.rs"), "z.rs").unwrap();
    assert_ne!(first, source_content_sha256(&a).unwrap());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("b"), a.join("crates/link")).unwrap();
        assert!(source_content_sha256(&a).is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn app_source_cache_uses_digest_and_rejects_partial_snapshots() {
    let root = std::env::temp_dir().join(format!("colm-source-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let source = root.join("source");
    let cache = root.join("cache");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("Cargo.toml"), "[workspace]\n").unwrap();
    std::fs::write(source.join("Cargo.lock"), "first").unwrap();
    let tarball = root.join("source.tar.gz");
    let pack = || {
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&tarball)
            .arg("-C")
            .arg(&source)
            .args(["Cargo.toml", "Cargo.lock"])
            .status()
            .unwrap()
            .success());
        let mut bytes = std::fs::read(&tarball).unwrap();
        bytes.resize(16384, 0);
        std::fs::write(&tarball, bytes).unwrap();
    };
    pack();
    let workers: Vec<_> = (0..3)
        .map(|_| {
            let tarball = tarball.clone();
            let cache = cache.clone();
            std::thread::spawn(move || unpack_app_source(&tarball, &cache).unwrap())
        })
        .collect();
    let paths: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(paths.iter().all(|path| path == &paths[0]));
    let first = paths[0].clone();
    assert_eq!(first, unpack_app_source(&tarball, &cache).unwrap());
    std::fs::write(source.join("Cargo.lock"), "other").unwrap();
    pack();
    let second = unpack_app_source(&tarball, &cache).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        std::fs::read_to_string(first.join("Cargo.lock")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(second.join("Cargo.lock")).unwrap(),
        "other"
    );
    std::fs::remove_file(second.join(".colm-source-sha256")).unwrap();
    assert!(unpack_app_source(&tarball, &cache).is_err());
    assert!(second.join("Cargo.lock").exists());
    std::fs::remove_dir_all(root).unwrap();
}

/// 本机结果比远程运行新时不自动覆盖；明确要求才覆盖；没有本机结果或本机结果更旧时照常取回。
#[test]
fn fetching_refuses_to_overwrite_local_results_newer_than_the_remote_job() {
    assert!(refuse_to_overwrite_newer(None, 100, false).is_ok());
    assert!(refuse_to_overwrite_newer(Some(90), 100, false).is_ok());
    assert!(refuse_to_overwrite_newer(Some(100), 100, false).is_ok());
    let err = refuse_to_overwrite_newer(Some(101), 100, false).unwrap_err();
    assert!(err.to_string().contains("newer than this remote job"));
    assert!(refuse_to_overwrite_newer(Some(101), 100, true).is_ok());
}

#[test]
fn job_connection_binding_is_optional_and_contains_no_password() {
    let legacy = serde_json::json!({
        "host": "server", "root": "/work", "job": "job-1", "remote_case": "/work/case",
        "case_name": "case", "engine": "rust", "submitted_at": 1
    });
    let mut record: Record = serde_json::from_value(legacy).unwrap();
    assert!(record.ssh_auth.is_none());
    record.ssh_auth = Some(colm_remote::auth::Connection {
        host: "server".into(),
        username: "user".into(),
        port: 2222,
        auth: "password".into(),
        identity_file: String::new(),
    });
    let encoded = serde_json::to_value(&record).unwrap();
    assert!(encoded["ssh_auth"].get("password").is_none());
    assert_eq!(serde_json::from_value::<Record>(encoded).unwrap(), record);
}

fn remote_kernel(name: &str, sha: &str, modified: u64) -> colm_remote::kernel::RemoteKernel {
    colm_remote::kernel::RemoteKernel {
        name: name.into(),
        dir: format!("/srv/kernels/{name}"),
        preset: "default".into(),
        macros: Vec::new(),
        platform: "Linux-x86_64".into(),
        colm_git_sha: sha.into(),
        full: true,
        modified,
    }
}

fn version(touches_kernel: bool) -> RemoteVersion {
    RemoteVersion {
        name: "snow".into(),
        head: "0123456789abcdef".into(),
        src: PathBuf::from("src"),
        touches: colm_workspace::version::Touches {
            engine: true,
            kernel: touches_kernel,
        },
        value: json!({ "kind": "workspace", "name": "snow" }),
    }
}

/// 修改版改了 Fortran：只认用它自己的提交编的内核；官方运行的 auto 不挑这种内核，哪怕它最新。
#[test]
fn modified_versions_and_official_runs_never_share_a_fortran_kernel() {
    let kernels = vec![
        remote_kernel("default-a", "f59ffdce", 10),
        remote_kernel("default-b", "ws-snow-01234567", 30),
        remote_kernel("default-c", "ws-other-89abcdef", 40),
    ];
    assert_eq!(version(true).kernel_identity(), "ws-snow-01234567");
    assert_eq!(
        version_kernel(&kernels, "default", Some(&version(true))).unwrap(),
        Some("/srv/kernels/default-b".into())
    );
    // 没改 Fortran 的修改版与官方版一样用正式内核。
    assert_eq!(
        version_kernel(&kernels, "default", Some(&version(false))).unwrap(),
        None
    );
    assert_eq!(version_kernel(&kernels, "default", None).unwrap(), None);
    let official = official_kernels(&kernels);
    assert_eq!(
        colm_remote::kernel::newest_full(&official, "default").map(|k| k.name.as_str()),
        Some("default-a")
    );
    let missing = version_kernel(&kernels[..1], "default", Some(&version(true))).unwrap_err();
    assert!(format!("{missing:#}").contains("remote-kernel --preset default --model-version snow"));
}

#[test]
fn fetched_results_record_the_server_jobs_outcome() {
    let submitted = json!({ "schema": 1, "version": { "kind": "official" }, "location": { "kind": "server", "host": "7920" } });
    let ok = fetched_provenance(&submitted, Some(job::State::Finished { exit_code: 0 }));
    assert_eq!(ok["ok"], true);
    assert!(ok["error"].is_null());
    assert_eq!(ok["location"]["host"], "7920");
    assert!(ok["finished"].as_u64().is_some());
    let failed = fetched_provenance(&submitted, Some(job::State::Finished { exit_code: 3 }));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"], "the server job exited with status 3");
    assert!(fetched_provenance(&submitted, None)["ok"].is_null());
}

fn completed_output_fixture(label: &str) -> PendingFixture {
    let (fixture, _) = PendingFixture::new(label);
    let out = fixture.0.join("out/fixture");
    std::fs::create_dir_all(out.join("history")).unwrap();
    std::fs::write(
        out.join("history/fixture_hist_2004.nc"),
        vec![b'A'; 3 * 1024 * 1024],
    )
    .unwrap();
    crate::provenance::write_value(
        &fixture.0,
        &json!({"schema":1,"ok":true,"model_run":{"revision":1},"execution_snapshot":"0123456789abcdef"}),
    );
    fixture
}

#[test]
fn completed_outputs_reject_same_size_middle_tampering_without_resealing() {
    use std::io::{Seek, SeekFrom, Write};
    let fixture = completed_output_fixture("completed-tamper");
    let proof = verify_completed_outputs(&fixture.0).unwrap();
    let file = fixture.0.join("out/fixture/history/fixture_hist_2004.nc");
    let mut output = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
    output.seek(SeekFrom::Start(1_500_000)).unwrap();
    output.write_all(b"B").unwrap();
    drop(output);
    assert!(verify_completed_outputs(&fixture.0)
        .unwrap_err()
        .to_string()
        .contains("original completed output hashes"));
    assert_eq!(
        crate::provenance::read(&fixture.0).unwrap().unwrap(),
        proof["provenance"]
    );
    assert_eq!(std::fs::metadata(file).unwrap().len(), 3 * 1024 * 1024);
}

#[test]
fn completed_outputs_reject_missing_extra_and_symlink_history_members() {
    let fixture = completed_output_fixture("completed-members");
    let history = fixture.0.join("out/fixture/history");
    let original = history.join("fixture_hist_2004.nc");
    let saved = fixture.0.join("original.nc");
    std::fs::rename(&original, &saved).unwrap();
    assert!(verify_completed_outputs(&fixture.0).is_err());
    std::fs::rename(&saved, &original).unwrap();
    std::fs::write(history.join("extra.nc"), b"extra").unwrap();
    assert!(verify_completed_outputs(&fixture.0).is_err());
    std::fs::remove_file(history.join("extra.nc")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&original, history.join("unrecorded-link.nc")).unwrap();
        assert!(verify_completed_outputs(&fixture.0)
            .unwrap_err()
            .to_string()
            .contains("symlink"));
        std::fs::remove_file(history.join("unrecorded-link.nc")).unwrap();
        std::fs::rename(&original, &saved).unwrap();
        std::os::unix::fs::symlink(&saved, &original).unwrap();
        assert!(verify_completed_outputs(&fixture.0)
            .unwrap_err()
            .to_string()
            .contains("symlink"));
    }
}

#[test]
fn staged_fetch_requires_the_original_complete_history_table() {
    let fixture = completed_output_fixture("staged-completion");
    let proof = verify_completed_outputs(&fixture.0).unwrap();
    let expected = recorded_history_hashes(&proof["provenance"]).unwrap();
    let stage = FetchStage::new(&fixture.0).unwrap();
    let history = stage.0.join("history");
    std::fs::create_dir(&history).unwrap();
    let name = "fixture_hist_2004.nc";
    std::fs::copy(
        fixture.0.join("out/fixture/history").join(name),
        history.join(name),
    )
    .unwrap();
    verify_history_hashes(&history, &expected).unwrap();
    std::fs::write(history.join(name), b"modified during transfer").unwrap();
    assert!(verify_history_hashes(&history, &expected).is_err());
    // A transfer failure leaves the old result and its original provenance intact.
    assert_eq!(
        verify_completed_outputs(&fixture.0).unwrap()["provenance"],
        proof["provenance"]
    );
}

#[test]
fn transient_output_changes_are_detected_even_when_completed_bytes_are_restored() {
    let fixture = completed_output_fixture("completed-restored");
    let before = verify_completed_outputs(&fixture.0).unwrap();
    let path = fixture.0.join("out/fixture/history/fixture_hist_2004.nc");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"different output read during fetch").unwrap();
    std::fs::write(&path, bytes).unwrap();
    let after = verify_completed_outputs(&fixture.0).unwrap();
    assert_eq!(before["history_sha256"], after["history_sha256"]);
    assert_ne!(before["history_inventory"], after["history_inventory"]);
    assert_ne!(before, after);
}

#[test]
fn partial_and_legacy_fetches_preserve_execution_success_but_cannot_rank() {
    let fixture = completed_output_fixture("uncertified-fetch");
    let proof = verify_completed_outputs(&fixture.0).unwrap();
    let submitted = json!({"schema":1,"location":{"kind":"server"}});
    for (proof, partial) in [(Some(&proof), true), (None, false)] {
        let mut fetched =
            fetched_provenance(&submitted, Some(job::State::Finished { exit_code: 0 }));
        fetch_certification(&mut fetched, proof, partial);
        assert_eq!(fetched["execution_ok"], true);
        assert_eq!(fetched["ok"], false);
        assert_eq!(fetched["output_verification"]["status"], "unavailable");
        if let Some(proof) = proof {
            assert_eq!(
                fetched["remote_execution_provenance"]["output_sha256"],
                proof["provenance"]["output_sha256"]
            );
        }
        let inputs = crate::fingerprint::FullInputIdentity::unavailable("fixture");
        assert!(verify_recorded_inputs(&fixture.0, &fetched, &inputs)
            .unwrap_err()
            .to_string()
            .contains("partial or lacks"));
    }
    let mut fetched = fetched_provenance(&submitted, Some(job::State::Finished { exit_code: 0 }));
    fetch_certification(&mut fetched, Some(&proof), false);
    assert_eq!(fetched["ok"], true);
    let mut failed_proof = proof;
    failed_proof["provenance"]["ok"] = json!(false);
    fetch_certification(&mut fetched, Some(&failed_proof), false);
    assert_eq!(fetched["ok"], false);
}

#[test]
fn only_an_old_cli_without_the_verifier_can_return_unverified_diagnostics() {
    let failed = |code, message: &str| colm_remote::ssh::Output {
        success: false,
        code,
        stdout: String::new(),
        stderr: message.into(),
    };
    assert!(parse_output_verification(failed(
        Some(1),
        "Error: unknown command: verify-completed-outputs\nusage"
    ))
    .unwrap()
    .is_none());
    for (code, message) in [
        (
            Some(1),
            "Error: history differs from original completed output hashes",
        ),
        (Some(255), "Permission denied"),
        (
            Some(255),
            "Error: unknown command: verify-completed-outputs",
        ),
        (Some(1), "unknown command: other-command"),
    ] {
        assert!(parse_output_verification(failed(code, message)).is_err());
    }
    let fixture = completed_output_fixture("proof-parser");
    let mut proof = verify_completed_outputs(&fixture.0).unwrap();
    proof["history_sha256"]["history/fixture_hist_2004.nc"] = json!("b".repeat(64));
    assert!(parse_output_verification(colm_remote::ssh::Output {
        success: true,
        code: Some(0),
        stdout: proof.to_string(),
        stderr: String::new()
    })
    .is_err());
    assert!(recorded_history_hashes(
        &json!({"output_sha256":{"../history/escape.nc":"a".repeat(64)}})
    )
    .is_err());
    assert_eq!(
        recorded_history_hashes(
            &json!({"output_sha256":{"history\\fixture_hist_2004.nc":"a".repeat(64)}})
        )
        .unwrap(),
        recorded_history_hashes(
            &json!({"output_sha256":{"history/fixture_hist_2004.nc":"a".repeat(64)}})
        )
        .unwrap()
    );
}

/// `--model-version`：源码要干净；出处里带着版本、提交与改动范围。
#[test]
fn a_remote_model_version_must_be_committed_and_carries_its_provenance() {
    use colm_workspace::git;
    let root = std::env::temp_dir().join(format!("colm-remote-version-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("source");
    std::fs::create_dir_all(repo.join("vendor/CoLM202X/main")).unwrap();
    std::fs::write(repo.join("vendor/CoLM202X/main/MOD_A.F90"), "! a\n").unwrap();
    git::run(&repo, &["init", "-q", "-b", "main"]).unwrap();
    git::set_identity(&repo).unwrap();
    git::run(&repo, &["add", "-A"]).unwrap();
    git::run(&repo, &["commit", "-q", "-m", "base"]).unwrap();
    let ws_root = root.join("ws");
    let ws =
        colm_workspace::Workspace::create(&ws_root, "snow", repo.to_str().unwrap(), None).unwrap();
    std::fs::write(ws.src().join("vendor/CoLM202X/main/MOD_A.F90"), "! b\n").unwrap();
    let opts = Opts::parse(&[
        "--model-version".into(),
        "snow".into(),
        "--workspace-root".into(),
        ws_root.display().to_string(),
    ])
    .unwrap();
    let dirty = remote_version(&opts)
        .err()
        .expect("uncommitted changes are refused");
    assert!(format!("{dirty:#}").contains("uncommitted"));
    git::run(&ws.src(), &["commit", "-q", "-am", "fortran"]).unwrap();
    let version = remote_version(&opts).unwrap().expect("a modified version");
    assert!(version.touches.kernel && !version.touches.engine);
    assert_eq!(version.value["name"], "snow");
    assert_eq!(version.value["head"], version.head.as_str());
    assert_eq!(version.src, ws.src());
    let official = Opts::parse(&["--model-version".into(), "official".into()]).unwrap();
    assert!(remote_version(&official).unwrap().is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn inventory_does_not_hide_broken_workspace_metadata_or_root_errors() {
    let root = std::env::temp_dir().join(format!("colm-strict-inventory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    assert!(inventory_workspace_versions(&root).unwrap().is_empty());
    std::fs::create_dir_all(root.join("broken")).unwrap();
    std::fs::write(root.join("broken/workspace.json"), "not JSON").unwrap();
    assert!(inventory_workspace_versions(&root).is_err());
    let file = root.join("not-a-directory");
    std::fs::write(&file, "x").unwrap();
    assert!(inventory_workspace_versions(&file).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn successful_cleanup_preserves_removed_fact_when_unlock_fails() {
    let value = cleanup_result(
        json!({"removed":"0123456789abcdef"}),
        Err(anyhow::anyhow!("SSH disconnected")),
    )
    .unwrap();
    assert_eq!(value["removed"], "0123456789abcdef");
    assert!(value["warning"]
        .as_str()
        .unwrap()
        .contains("SSH disconnected"));
    assert!(cleanup_result(json!({"refused":["referenced"]}), Ok(())).is_err());
}

struct PendingFixture(PathBuf);
impl PendingFixture {
    fn new(label: &str) -> (Self, PendingRun) {
        let path = std::env::temp_dir().join(format!(
            "colm-pending-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'fixture'\n/\n",
        )
        .unwrap();
        let path = path.canonicalize().unwrap();
        let pending = PendingRun {
            schema: 1,
            source_case: path.clone(),
            run_case: path.clone(),
            record: Record {
                ssh_auth: None,
                host: "test-server".into(),
                root: "/data/colm".into(),
                job: "run-original-operation".into(),
                remote_case: "/data/colm/cases/fixture-one".into(),
                case_name: "fixture".into(),
                engine: "0123456789abcdef".into(),
                submitted_at: 1,
                scheduler: "bare".into(),
                scheduler_id: None,
                provenance: Some(json!({"operation_id":"original-operation"})),
            },
        };
        (Self(path), pending)
    }
    fn retry(&self) -> Opts {
        Opts {
            positional: vec![self.0.to_string_lossy().into_owned()],
            flags: vec![
                ("--operation-id".into(), "new-operation".into()),
                ("--model-id".into(), "new-model".into()),
                ("--model-revision".into(), "2".into()),
            ],
        }
    }
}
impl Drop for PendingFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ssh_success_response_loss_survives_restart_and_blocks_a_new_model_run() {
    let (fixture, pending) = PendingFixture::new("response-loss");
    let submissions = std::cell::Cell::new(0);
    let failed: Result<()> = submit_with_intent(&pending, || {
        // The server accepted the launch, then the SSH response was lost.
        submissions.set(submissions.get() + 1);
        Err(anyhow::anyhow!(
            "SSH connection lost after remote acceptance"
        ))
    });
    assert!(failed.is_err());
    let reopened = read_pending(&fixture.0).unwrap().unwrap();
    assert_eq!(reopened.record.job, pending.record.job);
    assert_eq!(reopened.record.provenance, pending.record.provenance);
    assert!(cmd_run(&fixture.retry())
        .unwrap_err()
        .to_string()
        .contains("unresolved outcome"));
    assert!(submit_with_intent(&pending, || {
        submissions.set(submissions.get() + 1);
        Ok(())
    })
    .is_err());
    assert_eq!(submissions.get(), 1);
    assert!(read_record(&fixture.0).is_err());
}

#[test]
fn local_record_failure_retains_the_original_job_until_reconciliation() {
    let (fixture, pending) = PendingFixture::new("record-failure");
    let submissions = std::cell::Cell::new(0);
    submit_with_intent(&pending, || {
        submissions.set(submissions.get() + 1);
        Ok(())
    })
    .unwrap();
    // A completed SSH submission must stay reserved when the final local write fails.
    std::fs::create_dir(fixture.0.join(RECORD)).unwrap();
    assert!(finish_pending(&pending, &pending.record).is_err());
    assert!(cmd_run(&fixture.retry())
        .unwrap_err()
        .to_string()
        .contains("unresolved outcome"));
    assert_eq!(submissions.get(), 1);
    assert_eq!(
        read_pending(&fixture.0).unwrap().unwrap().record.job,
        pending.record.job
    );
    std::fs::remove_dir(fixture.0.join(RECORD)).unwrap();
    finish_pending(&pending, &pending.record).unwrap();
    assert!(read_pending(&fixture.0).unwrap().is_none());
    assert_eq!(read_record(&fixture.0).unwrap().job, pending.record.job);
}

#[cfg(unix)]
#[test]
fn pending_status_rejects_a_server_script_for_another_case_or_operation() {
    let (fixture, mut pending) = PendingFixture::new("identity");
    pending.record.root = fixture.0.to_string_lossy().into_owned();
    pending.record.remote_case = format!("{}/cases/fixture", pending.record.root);
    let dir = PathBuf::from(job::job_dir(&pending.record.root, &pending.record.job));
    std::fs::create_dir_all(&dir).unwrap();
    let correct=format!("# colm-engine-snapshot: {}\n# colm-run-case: {}\n# colm-operation-id: original-operation\n",pending.record.engine,serde_json::to_string(&pending.record.remote_case).unwrap());
    std::fs::write(dir.join("script.sh"), &correct).unwrap();
    std::fs::write(dir.join("pid"), "123\n").unwrap();
    let run = || {
        Command::new("bash")
            .arg("-c")
            .arg(pending_identity_script(&pending).unwrap())
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    std::fs::write(
        dir.join("script.sh"),
        correct.replace("original-operation", "different-operation"),
    )
    .unwrap();
    assert!(!run().status.success());
    std::fs::write(
        dir.join("script.sh"),
        correct.replace("fixture\"", "different\""),
    )
    .unwrap();
    assert!(!run().status.success());
}

#[test]
fn packaged_sources_take_precedence_over_an_enclosing_checkout() {
    let root = std::env::temp_dir().join(format!(
        "colm-package-source-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("crates/colm-cli")).unwrap();
    std::fs::write(root.join("Cargo.lock"), "version = 3").unwrap();
    let executable = root.join("Example.app/Contents/MacOS");
    let resource = root.join("Example.app/Contents/Resources/engine");
    std::fs::create_dir_all(&executable).unwrap();
    std::fs::create_dir_all(&resource).unwrap();
    let archive = resource.join("colm-src.tar.gz");
    std::fs::write(&archive, b"selection fixture").unwrap();
    assert!(
        matches!(engine_source_at(&executable, std::slice::from_ref(&resource)).unwrap(), Source::Tarball(path) if path == archive)
    );
    std::fs::remove_file(&archive).unwrap();
    assert!(
        matches!(engine_source_at(&executable, &[resource]).unwrap(), Source::Checkout(path) if path == root)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_remote_manifest_rejects_classification_crop_and_unknown_metadata() {
    let root = std::env::temp_dir().join(format!(
        "remote-kernel-check-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let registry = colm_model::Registry::open(&root).unwrap();
    let mut revision = registry
        .install_official(colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        })
        .unwrap();
    revision
        .overlay
        .physics
        .insert("DEF_USE_USGS".into(), ".true.".into());
    let mut metadata = serde_json::json!({"schema":1,"preset":"same-preset","platform":"Linux-x86_64","colm_git_sha":"fixture","generator_args":"","macros":["SinglePoint","LULC_IGBP"],"built_with":"fixture","netcdf_c":"","netcdf_fortran":"","hdf5":"","sha256":{}});
    // Historical case selectors cannot dictate a model's remote kernel requirements.
    check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision).unwrap();
    revision
        .overlay
        .parameters
        .push(colm_model::ParameterOverrideRecord {
            parameter_id: colm_case::parameters::find("DEF_LC_VMAX25")
                .unwrap()
                .id
                .clone(),
            raw_key: "DEF_LC_VMAX25".into(),
            scope_instance: colm_model::ParameterScopeInstance {
                kind: "land-cover-class".into(),
                scheme: Some("USGS".into()),
                index: Some(1),
                type_name: None,
                process_file: None,
            },
            value: "40".into(),
            expected_old_value: None,
            unit: None,
        });
    assert!(
        check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision).is_err()
    );
    metadata["macros"] = serde_json::json!(["SinglePoint", "LULC_USGS"]);
    check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision).unwrap();
    revision
        .overlay
        .parameters
        .push(colm_model::ParameterOverrideRecord {
            parameter_id: "pft:DEF_PFT_VMAX25".into(),
            raw_key: "DEF_PFT_VMAX25".into(),
            scope_instance: colm_model::ParameterScopeInstance {
                kind: "pft".into(),
                scheme: None,
                index: Some(16),
                type_name: None,
                process_file: None,
            },
            value: "40".into(),
            expected_old_value: None,
            unit: None,
        });
    assert!(
        check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision).is_err()
    );
    metadata["macros"] = serde_json::json!(["SinglePoint", "LULC_USGS", "CROP"]);
    check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision).unwrap();
    for macros in [
        serde_json::json!([]),
        serde_json::json!(["LULC_IGBP", "LULC_USGS"]),
    ] {
        metadata["macros"] = macros;
        assert!(
            check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision)
                .is_err()
        );
    }
    assert!(check_remote_kernel_manifest("{}", "/server/kernel", &revision).is_err());
    metadata["schema"] = serde_json::json!(999);
    assert!(
        check_remote_kernel_manifest(&metadata.to_string(), "/server/kernel", &revision).is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_materialized_run_recovers_verified_revision_without_selection_flags() {
    let root = std::env::temp_dir().join(format!(
        "remote-revision-check-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let registry = colm_model::Registry::open(root.join("registry")).unwrap();
    let revision = registry
        .install_official(colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        })
        .unwrap();
    let case = root.join("case");
    std::fs::create_dir_all(&case).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME='demo'\n DEF_dir_output='out'\n/\n",
    )
    .unwrap();
    assert!(materialized_revision(&case).unwrap().is_none());
    std::fs::create_dir_all(registry.root().join("runs")).unwrap();
    let run = registry
        .materialize_run(
            &revision.version,
            &case,
            &registry.root().join("runs/op"),
            "op",
        )
        .unwrap();
    assert_eq!(
        materialized_revision(&run.run_case).unwrap(),
        Some(revision)
    );
    let mut corrupt = run.clone();
    corrupt.overlay_sha256 = "b".repeat(64);
    std::fs::write(
        run.run_case.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec(&corrupt).unwrap(),
    )
    .unwrap();
    assert!(materialized_revision(&run.run_case).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn server_destination_is_confined_to_project_runs() {
    assert_eq!(
        server_destination("/data/My project/", Some("run-123")).unwrap(),
        "/data/My project/runs/run-123"
    );
    assert_eq!(
        server_destination("/data/project", None).unwrap(),
        "/data/project"
    );
    for bad in [
        "/",
        "relative",
        "/data/../other",
        "/data/./other",
        "/data/\nother",
        "/data\\other",
    ] {
        assert!(server_destination(bad, Some("run-123")).is_err(), "{bad}");
    }
    assert!(server_destination("/data/project", Some("../escape")).is_err());
}

#[cfg(unix)]
#[test]
fn server_inputs_ignore_local_existence_and_user_maps() {
    let root = std::env::temp_dir().join(format!("colm-server-inputs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let case = root.join("mirror");
    std::fs::create_dir_all(case.join("runs/old")).unwrap();
    let external = root.join("server-dataset.nc");
    std::fs::write(&external, b"local file with same name must not be uploaded").unwrap();
    std::fs::write(case.join("case.nml"), format!("&nl_colm\n DEF_CASE_NAME='demo'\n SITE_fsitedata='{}'\n DEF_forcing_namelist='{}/forcing.nml'\n DEF_dir_output='out/'\n/\n", external.display(), case.display())).unwrap();
    std::fs::write(
        case.join("forcing.nml"),
        "&nl_colm_forcing\n DEF_dir_forcing='/server/data/'\n/\n",
    )
    .unwrap();
    std::fs::write(case.join("runs/old/huge.nc"), b"do not stage old results").unwrap();
    let staging = root.join("stage");
    let maps = vec![Mapping {
        local: root.to_string_lossy().into_owned(),
        remote: "/wrong".into(),
    }];
    let staged = stage_case_inputs(
        &case,
        &staging,
        "/server/project/runs/new",
        &maps,
        false,
        true,
    )
    .unwrap();
    let doc =
        colm_namelist::parse(&std::fs::read_to_string(staging.join("case.nml")).unwrap()).unwrap();
    assert_eq!(
        doc.get("SITE_fsitedata"),
        Some(&Value::Str(external.to_string_lossy().into_owned()))
    );
    assert_eq!(
        doc.get("DEF_forcing_namelist"),
        Some(&Value::Str("/server/project/runs/new/forcing.nml".into()))
    );
    assert_eq!(
        doc.get("DEF_dir_output"),
        Some(&Value::Str("/server/project/runs/new/out/".into()))
    );
    assert!(staged.unmapped.is_empty());
    assert!(staged
        .remote_inputs
        .contains(&external.to_string_lossy().into_owned()));
    assert!(!staging.join("runs").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn server_destination_claim_rejects_unowned_existing_and_escaped_directories() {
    let root = std::env::temp_dir().join(format!("colm-server-claim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("runs/run-1");
    let execute = |claim: &str| {
        std::process::Command::new("python3")
            .args(["-c", CLAIM_SERVER_RUN])
            .arg(&root)
            .arg(&target)
            .arg(claim)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    assert!(!execute("0"));
    std::fs::write(
        root.join(".colm-desktop-server.json"),
        serde_json::to_vec(&json!({"schema":1,"remote_directory":root.to_string_lossy()})).unwrap(),
    )
    .unwrap();
    assert!(execute("0"));
    assert!(!target.exists());
    assert!(execute("1"));
    std::fs::write(target.join("keep"), b"original").unwrap();
    assert!(!execute("1"));
    assert_eq!(std::fs::read(target.join("keep")).unwrap(), b"original");
    std::fs::remove_dir_all(root.join("runs")).unwrap();
    std::os::unix::fs::symlink(&root, root.join("runs")).unwrap();
    assert!(!execute("0"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn server_configuration_maps_windows_cache_paths_before_server_path_validation() {
    let case = Path::new(r"\\?\C:\Users\user\case\runs\run-1");
    let remote = "/data/project/runs/run-1";
    for path in [
        r"C:\Users\user\case\runs\run-1\forcing.nml",
        r"\\?\C:\Users\user\case\runs\run-1\forcing.nml",
        "C:/Users/user/case/runs/run-1/forcing.nml",
    ] {
        assert_eq!(
            server_place(path, case, remote, &[]).unwrap(),
            Placed::Remote(format!("{remote}/forcing.nml"))
        );
    }
    let aliases = vec![Mapping {
        local: r"C:\short-run".into(),
        remote: remote.into(),
    }];
    assert_eq!(
        server_place(r"C:\short-run\params\", case, remote, &aliases).unwrap(),
        Placed::Remote(format!("{remote}/params/"))
    );
    let unc_case = Path::new(r"\\?\UNC\server\share\case");
    assert_eq!(
        server_place(r"\\server\share\case\forcing.nml", unc_case, remote, &[]).unwrap(),
        Placed::Remote(format!("{remote}/forcing.nml"))
    );
    for path in [
        r"C:\elsewhere\forcing.nc",
        "C:/elsewhere/forcing.nc",
        r"/data\forcing.nc",
        r"C:\short-run\..\escape.nc",
        r"C:\short-run-other\forcing.nc",
    ] {
        assert!(
            server_place(path, case, remote, &aliases).is_err(),
            "{path}"
        );
    }
    assert_eq!(
        server_place("/data/forcing.nc", case, remote, &aliases).unwrap(),
        Placed::Remote("/data/forcing.nc".into())
    );
}
