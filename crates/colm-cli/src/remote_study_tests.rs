use super::*;

#[test]
fn pending_submission_reappears_after_restart_without_exposing_authentication() {
    let root = std::env::temp_dir().join(format!(
        "colm-pending-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join(PENDING), json!({
        "host":"7920ts", "root":"/data/colm", "job":"study-1",
        "remote_project":"/data/colm/s/abc", "engine":"abc123",
        "ssh_auth":{"host":"7920ts","username":"private-user","port":22,"auth":"key","identity_file":"/private/key"},
        "password":"never-show-this", "provenance":{"operation_id":"study-op"}
    }).to_string()).unwrap();
    let value = pending_status(&root).unwrap().unwrap();
    assert_eq!(value["job"], "study-1");
    assert_eq!(value["needs_reconciliation"], true);
    let shown = value.to_string();
    for secret in [
        "ssh_auth",
        "private-user",
        "/private/key",
        "never-show-this",
    ] {
        assert!(!shown.contains(secret));
    }
    let pending = read_pending(&root).unwrap();
    let script = pending_identity_script(&pending).unwrap();
    assert!(script.contains("# colm-study-root:"));
    assert!(script.contains("# colm-engine-snapshot: abc123"));
    assert!(!script.contains("study-run"));
    assert!(!script.contains("nohup"));
    // A complete record is authoritative even if deleting its old intent was interrupted.
    std::fs::write(root.join(RECORD), "{}").unwrap();
    assert!(pending_status(&root).unwrap().is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_and_comparison_paths_stay_within_the_bound_job() {
    let mut pending = PendingStudy {
        ssh_auth: None,
        host: "server".into(),
        root: "/data/colm".into(),
        job: "study-1".into(),
        remote_project: "/data/colm/s/abc".into(),
        engine: None,
        provenance: Value::Null,
    };
    pending_identity_script(&pending).unwrap();
    pending.remote_project = "/data/colm/s/../other".into();
    assert!(pending_identity_script(&pending).is_err());
    pending.remote_project = "/data/colm/s/abc".into();
    pending.job = "../other".into();
    assert!(pending_identity_script(&pending).is_err());
    let record: StudyRecord = serde_json::from_value(json!({
        "host":"server", "root":"/data/colm", "job":"study-1", "remote_project":"/data/colm/s/abc",
        "engine":"abc123", "submitted_at":1, "scheduler":"bare", "provenance":{}
    }))
    .unwrap();
    let command = comparison_command(
        &record,
        "/data/colm/s/abc/.colm/studies/s1",
        std::slice::from_ref(&record.remote_project),
        None,
    )
    .unwrap();
    assert!(command.contains("'/data/colm/engine/abc123/bin/colm-cli' study-verify-comparison '/data/colm/s/abc/.colm/studies/s1'"));
    assert!(!command.contains("--member"));
    assert!(!command.contains("study-run"));
    // 选定成员时服务器命令必须带上 --member，不能只查 best/baseline。
    let command = comparison_command(
        &record,
        "/data/colm/s/abc/.colm/studies/s1",
        std::slice::from_ref(&record.remote_project),
        Some("m000003"),
    )
    .unwrap();
    assert!(
        command.ends_with(
            "study-verify-comparison '/data/colm/s/abc/.colm/studies/s1' --member m000003\n"
        ),
        "{command}"
    );
    assert!(comparison_command(
        &record,
        "/data/colm/s/abc/.colm/studies/s1",
        std::slice::from_ref(&record.remote_project),
        Some("m000003; rm -rf /"),
    )
    .is_err());
    // 没有空白也可能注入：只放行字母数字与 `-`/`_`。
    assert!(comparison_command(
        &record,
        "/data/colm/s/abc/.colm/studies/s1",
        std::slice::from_ref(&record.remote_project),
        Some("m000003;reboot"),
    )
    .is_err());
    assert!(comparison_command(
        &record,
        "/other/studies/s1",
        std::slice::from_ref(&record.remote_project),
        None
    )
    .is_err());
    assert!(comparison_command(
        &record,
        "/data/colm/s/abc/.colm/studies/../../outside",
        std::slice::from_ref(&record.remote_project),
        None
    )
    .is_err());
}

#[test]
fn fetched_metrics_must_equal_the_fresh_full_content_server_proof() {
    let root = std::env::temp_dir().join(format!(
        "colm-result-proof-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("results/provenance")).unwrap();
    std::fs::write(root.join("results/metrics.json"), b"real-evaluation-bytes").unwrap();
    std::fs::write(
        root.join("results/provenance/member.json"),
        b"full-identity-bytes",
    )
    .unwrap();
    let expected =
        serde_json::to_value(crate::study_version::comparison_results_sha256(&root).unwrap())
            .unwrap();
    verify_result_tree(&root.join("results"), &expected).unwrap();
    std::fs::write(
        root.join("results/metrics.json"),
        b"changed-evaluation-bytes",
    )
    .unwrap();
    assert!(verify_result_tree(&root.join("results"), &expected).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn observations_keep_their_site_so_same_named_files_do_not_collide() {
    assert_eq!(
        remote_observation_name("siteA", "/data/obs/Flux.nc"),
        "siteA-Flux.nc"
    );
    assert_eq!(
        remote_observation_name("siteB", "/other/Flux.nc"),
        "siteB-Flux.nc"
    );
}

#[test]
fn the_job_creates_the_study_on_the_server_records_its_root_and_runs_it() {
    let body = study_job_body(
        "/srv/colm",
        "abc123",
        "/srv/colm/studies/s-1-ff",
        "/srv/colm/kernels/default-1",
        8,
        4,
        "rust",
        "rust",
    );
    assert!(body.contains("'/srv/colm/engine/abc123/bin'/colm-cli study-create '/srv/colm/studies/s-1-ff' --spec '/srv/colm/studies/s-1-ff/spec.json'"), "{body}");
    assert!(body.contains("tail -n 1 study_create.out > study_root"));
    assert!(body.contains("study-run \"$STUDY\" --kernel '/srv/colm/kernels/default-1' --stream 1 --jobs 4 --engine rust --preprocessors rust"), "{body}");
    assert!(body.find(" fingerprint-policy").unwrap() < body.find("study-create").unwrap());
    let ensure = engine::ensure_script("/srv/colm", "abc123", 8);
    assert!(
        body.find(&ensure).unwrap() + ensure.len() <= body.find(" fingerprint-policy").unwrap()
    );
    assert!(body.find("study-create").unwrap() < body.find("study-run").unwrap());
}

fn member(id: &str, baseline: bool, value: f64) -> study::spec::MemberPlan {
    study::spec::MemberPlan {
        id: id.into(),
        generation: 0,
        candidate_index: 0,
        baseline,
        parameters: [("DEF_X".to_owned(), value)].into_iter().collect(),
    }
}

#[test]
fn fetched_results_are_used_only_when_the_server_built_the_same_members() {
    let local = vec![member("m0", true, 0.5), member("m1", false, 0.41)];
    let same = serde_json::to_value(&local).unwrap();
    assert!(same_members(&local, &same).unwrap());
    let other =
        serde_json::to_value(vec![member("m0", true, 0.5), member("m1", false, 0.42)]).unwrap();
    assert!(!same_members(&local, &other).unwrap());
    let fewer = serde_json::to_value(vec![member("m0", true, 0.5)]).unwrap();
    assert!(!same_members(&local, &fewer).unwrap());
}

#[test]
fn the_server_state_is_rewritten_to_this_study() {
    let state = json!({
        "study_id": "s-remote",
        "tasks": {
            "m1/siteA": { "member": "m1", "site": "siteA", "case_dir": "/srv/p/.colm/studies/s-remote/members/m1/siteA", "status": "succeeded", "process": { "pid": 9 } },
            "m2/siteA": { "member": "m2", "site": "siteA", "case_dir": "/elsewhere/m2", "status": "failed" },
            "m3/siteA": { "member": "m3", "site": "siteA", "case_dir": "/srv/p/.colm/studies/s-remote-other/members/m3/siteA", "status": "failed" }
        }
    });
    let local = Path::new("/Users/me/p/.colm/studies/s-local");
    let moved = transplant_state(state, "s-local", "/srv/p/.colm/studies/s-remote", local);
    assert_eq!(moved["study_id"], "s-local");
    assert_eq!(
        moved["tasks"]["m1/siteA"]["case_dir"],
        json!(local.join("members/m1/siteA"))
    );
    assert!(moved["tasks"]["m1/siteA"].get("process").is_none());
    assert_eq!(moved["tasks"]["m2/siteA"]["case_dir"], "/elsewhere/m2");
    assert_eq!(
        moved["tasks"]["m3/siteA"]["case_dir"],
        "/srv/p/.colm/studies/s-remote-other/members/m3/siteA"
    );
}

#[test]
fn only_studies_inside_a_project_can_be_sent() {
    let study = Path::new("/p/.colm/studies/s-1");
    assert_eq!(project_of(study).unwrap(), Path::new("/p"));
    assert!(project_of(Path::new("/p/studies/s-1")).is_err());
}

#[test]
fn submitted_study_ledger_uses_captured_version_and_real_local_directory() {
    let root =
        std::env::temp_dir().join(format!("colm-remote-study-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    colm_workspace::git::run(&source, &["init", "-q"]).unwrap();
    colm_workspace::git::set_identity(&source).unwrap();
    std::fs::write(source.join("README"), "initial").unwrap();
    colm_workspace::git::run(&source, &["add", "."]).unwrap();
    colm_workspace::git::run(&source, &["commit", "-qm", "initial"]).unwrap();
    let ws =
        colm_workspace::Workspace::create(&root.join("ws"), "snow", source.to_str().unwrap(), None)
            .unwrap();
    let study = root.join("local-study");
    std::fs::create_dir(&study).unwrap();
    let version = super::super::RemoteVersion {
        kind: colm_model::CodeKind::Workspace,
        name: "snow".into(),
        head: ws.head().unwrap(),
        src: ws.src(),
        touches: colm_workspace::version::Touches {
            engine: true,
            kernel: false,
        },
        value: json!({}),
    };
    record_version_use(None, &study, "local-id", "7920ts", 10).unwrap();
    assert!(!ws.dir.join("studies.jsonl").exists());
    std::fs::write(ws.src().join("README"), "later").unwrap();
    colm_workspace::git::run(&ws.src(), &["commit", "-qam", "later"]).unwrap();
    record_version_use(Some(&version), &study, "local-id", "7920ts", 10).unwrap();
    let rows = colm_workspace::studies::list(&ws).unwrap();
    assert_eq!(rows[0].dir, study.canonicalize().unwrap());
    assert_eq!(rows[0].head, version.head);
    assert_eq!(rows[0].location, "7920ts");
    assert!(!rows[0].current);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn study_body_declares_json_encoded_project_reference() {
    let project = "/srv/colm/s/quote\"and\nnewline";
    let body = study_job_body(
        "/srv/colm",
        "0123456789abcdef",
        project,
        "/kernel",
        1,
        1,
        "rust",
        "rust",
    );
    let marker = body
        .lines()
        .next()
        .unwrap()
        .strip_prefix("# colm-study-root: ")
        .unwrap();
    assert_eq!(serde_json::from_str::<String>(marker).unwrap(), project);
}

// Import fixtures exercise metadata integrity only; no model or scientific
// output is simulated, and the observation bytes are never evaluated.
fn fetch_fixture() -> (
    PathBuf,
    study::spec::Manifest,
    PathBuf,
    study::state::StudyState,
) {
    let root = std::env::temp_dir().join(format!(
        "colm-fetch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("site")).unwrap();
    std::fs::write(root.join("site/case.nml"), "&nl_colm\n DEF_CASE_NAME='site'\n DEF_dir_output='out'\n DEF_forcing_namelist='forcing.nml'\n DEF_TUNING_CNFAC=0.5\n/\n").unwrap();
    std::fs::write(root.join("site/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
    std::fs::write(root.join("obs.nc"), b"observation metadata fixture").unwrap();
    let spec = json!({"kind":"tuning","method":"differential-evolution","base_cases":["site"],"observations":{"site":"obs.nc"},"parameters":[{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],"targets":[{"key":"Qle","variable":"Qle","from":0,"to":3600,"min_pairs":2}],"budget":{"population":4,"generations":1}});
    std::fs::write(root.join("spec.json"), spec.to_string()).unwrap();
    let manifest = study::engine::create(&root, &root.join("spec.json")).unwrap();
    let staged = root.join("staged");
    std::fs::create_dir_all(staged.join("samples")).unwrap();
    std::fs::create_dir_all(staged.join("results")).unwrap();
    std::fs::copy(
        Path::new(&manifest.root).join("samples/g000000.csv"),
        staged.join("samples/g000000.csv"),
    )
    .unwrap();
    let mut imported = manifest.clone();
    imported.root = staged.to_string_lossy().into_owned();
    // write_generation is the production immutable CSV/checksum writer. Avoid
    // its member materialization by writing its documented format directly.
    let csv = "member,baseline,generation,candidate,DEF_TUNING_CNFAC\nm000005,false,1,5,0.45\nm000006,false,1,6,0.46\nm000007,false,1,7,0.47\nm000008,false,1,8,0.48\n";
    std::fs::write(staged.join("samples/g000001.csv"), csv).unwrap();
    std::fs::write(
        staged.join("samples/g000001.csv.sha256"),
        format!("{}\n", colm_kernel::sha256_hex(csv.as_bytes())),
    )
    .unwrap();
    let members = study::generation::load_members(&imported).unwrap();
    let tasks = members.iter().map(|member| study::state::TaskState {
        member: member.id.clone(),
        site: "site".into(),
        case_dir: Path::new(&manifest.root)
            .join("members")
            .join(&member.id)
            .join("site")
            .to_string_lossy()
            .into_owned(),
        status: study::state::TaskStatus::Materialized,
        stage: None,
        reason: None,
        objective: None,
        validation_objective: None,
        process: None,
    });
    let state = study::state::StudyState::new(manifest.id.clone(), tasks).unwrap();
    (root, manifest, staged, state)
}

#[test]
fn fetched_de_generations_retain_vectors_and_immutable_hashes() {
    let (root, manifest, staged, state) = fetch_fixture();
    study::engine::verify_frozen_inputs(&manifest).unwrap();
    validate_fetched_samples(&manifest, &state, &staged).unwrap();
    let study = Path::new(&manifest.root);
    std::fs::create_dir_all(staged.join("checkpoints/state")).unwrap();
    study::checkpoint::write_next(&staged.join("checkpoints/state"), &state).unwrap();
    std::fs::write(staged.join(RECORD), b"installed record fixture").unwrap();
    std::fs::write(staged.join(SOURCE), b"installed remote source fixture").unwrap();
    std::fs::write(staged.join(EVIDENCE), b"installed evidence fixture").unwrap();
    std::fs::write(staged.join("results/metrics.json"), b"new result fixture").unwrap();
    install_fetched_study(study, &staged).unwrap();
    // 持久来源与取回证据一起原子安装，之后的活动记录归档不会带走它们。
    assert_eq!(
        std::fs::read(study.join(SOURCE)).unwrap(),
        b"installed remote source fixture"
    );
    assert_eq!(
        std::fs::read(study.join(EVIDENCE)).unwrap(),
        b"installed evidence fixture"
    );
    study::generation::verify_generation_hash(&study.join("samples/g000001.csv")).unwrap();
    let members = study::generation::load_members(&manifest).unwrap();
    assert_eq!(members.len(), 9);
    assert_eq!(members[5].parameters["DEF_TUNING_CNFAC"], 0.45);
    assert_eq!(
        std::fs::read(study.join("samples/g000000.csv")).unwrap(),
        std::fs::read(staged.join("previous-1/g000000.csv")).unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tampered_or_conflicting_fetch_is_rejected_without_overwriting_local_data() {
    let (root, manifest, staged, state) = fetch_fixture();
    let local = Path::new(&manifest.root);
    std::fs::create_dir_all(local.join("results")).unwrap();
    std::fs::write(local.join("results/preserved"), b"old successful data").unwrap();
    let before = regular_tree_files(local, true).unwrap();
    let sample = staged.join("samples/g000001.csv");
    let good = std::fs::read(&sample).unwrap();
    std::fs::write(&sample, b"tampered").unwrap();
    assert!(validate_fetched_samples(&manifest, &state, &staged).is_err());
    assert_eq!(regular_tree_files(local, true).unwrap(), before);
    std::fs::write(&sample, good).unwrap();
    std::fs::write(
        local.join("samples/g000001.csv"),
        b"different local generation",
    )
    .unwrap();
    let before_conflict = regular_tree_files(local, true).unwrap();
    assert!(validate_fetched_samples(&manifest, &state, &staged).is_err());
    assert_eq!(regular_tree_files(local, true).unwrap(), before_conflict);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_fetch_install_restores_previous_results_samples_and_state() {
    let (root, manifest, staged, state) = fetch_fixture();
    validate_fetched_samples(&manifest, &state, &staged).unwrap();
    let local = Path::new(&manifest.root);
    std::fs::create_dir_all(local.join("results")).unwrap();
    std::fs::write(local.join("results/preserved"), b"old successful data").unwrap();
    let before = regular_tree_files(local, true).unwrap();
    // No staged checkpoint: installation fails after the first two directory
    // swaps, which must be rolled back without deleting old results or samples.
    assert!(install_fetched_study(local, &staged).is_err());
    assert_eq!(regular_tree_files(local, true).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fetched_generation_rows_must_match_the_file_number_and_population() {
    let (root, manifest, staged, state) = fetch_fixture();
    let sample = staged.join("samples/g000001.csv");
    let csv = std::fs::read_to_string(&sample).unwrap();
    for changed in [
        csv.replace(",false,1,", ",false,0,"),
        csv.lines().take(4).collect::<Vec<_>>().join("\n"),
    ] {
        std::fs::write(&sample, &changed).unwrap();
        std::fs::write(
            sample.with_extension("csv.sha256"),
            colm_kernel::sha256_hex(changed.as_bytes()),
        )
        .unwrap();
        assert!(validate_fetched_samples(&manifest, &state, &staged).is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn result_tree_validation_does_not_read_large_outputs() {
    let root = std::env::temp_dir().join(format!(
        "colm-fetch-large-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    // Validation inspects metadata only; output contents are not retained.
    std::fs::File::create(root.join("large.nc"))
        .unwrap()
        .set_len(8 * 1024 * 1024)
        .unwrap();
    let files = regular_tree_files(&root, false).unwrap();
    assert!(files[Path::new("large.nc")].is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn fetch_rejects_a_symlink_checkpoint_parent_without_touching_external_state() {
    use std::os::unix::fs::symlink;
    let (root, manifest, staged, state) = fetch_fixture();
    validate_fetched_samples(&manifest, &state, &staged).unwrap();
    let local = Path::new(&manifest.root);
    let outside = root.join("outside");
    std::fs::rename(local.join("checkpoints"), &outside).unwrap();
    symlink(&outside, local.join("checkpoints")).unwrap();
    let before = regular_tree_files(&outside, true).unwrap();
    assert!(ensure_fetch_boundary(local).is_err());
    assert!(install_fetched_study(local, &staged).is_err());
    assert_eq!(regular_tree_files(&outside, true).unwrap(), before);
    assert!(local.join("checkpoints").is_symlink());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn comparison_probes_forcing_exclusion_before_dispatching_evidence() {
    for policy in [
        "{}",
        "{\"forcing_content_policy\":\"full\"}",
        "unknown command",
    ] {
        let mut commands = Vec::new();
        let result = scoped_comparison_evidence("/engine/colm-cli", "EVIDENCE", |command| {
            commands.push(command.to_string());
            Ok(policy.into())
        });
        assert!(result.is_err());
        assert_eq!(commands.len(), 1);
        assert!(commands[0].ends_with(" fingerprint-policy"));
    }
    let mut commands = Vec::new();
    let result = scoped_comparison_evidence("/engine/colm-cli", "EVIDENCE", |command| {
        commands.push(command.to_string());
        Ok(if command.ends_with(" fingerprint-policy") {
            r#"{"forcing_content_policy":"excluded-v1"}"#.into()
        } else {
            r#"{"scope":"sha256-nonforcing-v1","forcing_contents_verified":false}"#.into()
        })
    })
    .unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[1], "EVIDENCE");
    assert_eq!(result["forcing_contents_verified"], false);
}

#[test]
fn only_a_truly_lost_job_can_be_closed() {
    let status = |state: job::State, detail: Option<&str>| job::Status {
        state,
        phase: None,
        detail: detail.map(str::to_owned),
        log_tail: String::new(),
    };
    assert!(closable(&status(job::State::Running, None)).is_err());
    assert!(closable(&status(job::State::Finished { exit_code: 0 }, None)).is_err());
    // 作业已经结束、退出码还没经 NFS 传过来（land02 实测 sacct 是 COMPLETED|0:0）：不能当丢失关掉。
    let lagging = closable(&status(job::State::Lost, Some("COMPLETED|0:0"))).unwrap_err();
    assert!(
        lagging.to_string().contains("check the status again"),
        "{lagging}"
    );
    closable(&status(job::State::Lost, Some("CANCELLED by 1026|0:0"))).unwrap();
    closable(&status(job::State::Lost, None)).unwrap();
    closable(&status(job::State::Unknown, None)).unwrap();
}

#[test]
fn the_job_probe_tells_a_lost_submission_from_one_on_the_server() {
    assert_eq!(
        parse_job_probe("colm-job-probe=absent\n"),
        Some((false, String::new(), None))
    );
    assert_eq!(
        parse_job_probe(
            "colm-job-probe=present\ncolm-job-scheduler=slurm\ncolm-job-sched-id=119890\n"
        ),
        Some((true, "slurm".into(), Some("119890".into())))
    );
    // 直接后台运行的作业目录里没有 scheduler 文件；登录横幅不影响识别。
    assert_eq!(
        parse_job_probe("Welcome to land\ncolm-job-probe=present\n"),
        Some((true, "bare".into(), None))
    );
    assert_eq!(parse_job_probe("Welcome\n"), None);
    assert_eq!(parse_job_probe(""), None);
}

// ---- P1-7a：研究资产清单覆盖所有观测源 ----

fn temporary_dir(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "colm-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// 一份手工研究清单：一个基础算例、一条站点观测、两个同名不同目录的格点目标观测。
/// 不跑 `study-create`（那要完整空间算例）；staging 只依赖清单与基础算例目录，手工构造更直接。
fn observation_staging_fixture() -> (PathBuf, study::spec::Manifest) {
    let root = temporary_dir("staging");
    std::fs::create_dir_all(root.join("cases/site")).unwrap();
    std::fs::write(
        root.join("cases/site/case.nml"),
        "&nl_colm\n DEF_CASE_NAME='site'\n DEF_dir_output='out'\n DEF_forcing_namelist='forcing.nml'\n DEF_TUNING_CNFAC=0.5\n/\n",
    )
    .unwrap();
    std::fs::write(root.join("cases/site/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
    let obs = root.join("obs");
    std::fs::create_dir_all(obs.join("a")).unwrap();
    std::fs::create_dir_all(obs.join("b")).unwrap();
    std::fs::write(obs.join("site.nc"), b"site observation bytes").unwrap();
    std::fs::write(obs.join("a/Pool.nc"), b"grid product a").unwrap();
    std::fs::write(obs.join("b/Pool.nc"), b"grid product b").unwrap();
    let study_dir = root.join("study");
    std::fs::create_dir_all(&study_dir).unwrap();
    // `study::model::base_case` 从研究目录里的 `model-inputs.json` 找基础算例；放错一层会回退到
    // `<project>/.colm/studies` 的父链，最后拿到一个不存在的路径。
    std::fs::write(
        study_dir.join("model-inputs.json"),
        json!({"base_root": root.join("cases")}).to_string(),
    )
    .unwrap();
    let spec: study::spec::StudySpec = serde_json::from_value(json!({
        "kind": "tuning",
        "method": "differential-evolution",
        "base_cases": ["site"],
        "observations": {"site": obs.join("site.nc")},
        "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
        "targets": [
            {"key":"Qle","variable":"Qle","from":0,"to":3600,"min_pairs":2},
            {"key":"Gpp","variable":"GPP","from":0,"to":3600,"min_pairs":2,
             "source":{"kind":"grid","path":obs.join("a/Pool.nc"),"variable":"GPP"}},
            {"key":"Nee","variable":"NEE","from":0,"to":3600,"min_pairs":2,
             "source":{"kind":"grid","path":obs.join("b/Pool.nc"),"variable":"NEE"}}
        ],
        "budget": {"population":4,"generations":1},
        "parent_study": {
            "study": "/p/.colm/studies/s-uncertainty",
            "spec_sha256": "a".repeat(64),
            "model_revision": {"id": "trace", "revision": 7}
        }
    }))
    .unwrap();
    let manifest = study::spec::Manifest {
        schema_version: 1,
        id: "s-staging".into(),
        root: study_dir.to_string_lossy().into_owned(),
        created_unix: 0,
        spec,
        members: Vec::new(),
        provenance: Default::default(),
    };
    (root, manifest)
}

#[test]
fn every_local_observation_source_is_staged_under_a_unique_name() {
    let (root, manifest) = observation_staging_fixture();
    let staging = root.join("staging");
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &[], false).unwrap();
    // 站点观测与两个目标观测都上传；两个目标的文件都叫 Pool.nc，不能相撞。
    let names = std::fs::read_dir(staging.join("obs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        names.contains(&"site-site.nc".to_owned()),
        "staged observations: {names:?}"
    );
    let grid_names = names
        .iter()
        .filter(|name| name.ends_with("-Pool.nc"))
        .collect::<Vec<_>>();
    assert_eq!(grid_names.len(), 2, "staged observations: {names:?}");
    assert_ne!(grid_names[0], grid_names[1]);
    for name in &grid_names {
        assert_eq!(
            std::fs::read(staging.join("obs").join(name)).unwrap().len(),
            if *name == "target-2-Gpp-Pool.nc" {
                b"grid product a".len()
            } else {
                b"grid product b".len()
            }
        );
    }
    // 服务器引擎可能来自较旧快照：审计元数据 parent_study 不进入给服务器的设定。
    assert!(staged.spec.get("parent_study").is_none(), "{}", staged.spec);
    let staged_spec = serde_json::to_value(&staged.spec).unwrap();
    assert_eq!(
        staged_spec["observations"]["site"],
        json!("/srv/colm/s/abc/obs/site-site.nc")
    );
    assert_eq!(
        staged_spec["targets"][1]["source"]["path"],
        json!("/srv/colm/s/abc/obs/target-2-Gpp-Pool.nc")
    );
    assert_eq!(
        staged_spec["targets"][2]["source"]["path"],
        json!("/srv/colm/s/abc/obs/target-3-Nee-Pool.nc")
    );
    // bytes 报的是整份研究随之上传的字节：基础算例复制时的文件大小 + 三份观测。
    // 算例里的路径改写是随后原地写的，不计入 bytes（与 remote-run 的既有口径一致）。
    let case_bytes = std::fs::metadata(root.join("cases/site/case.nml"))
        .unwrap()
        .len()
        + std::fs::metadata(root.join("cases/site/forcing.nml"))
            .unwrap()
            .len();
    let observation_bytes = (b"site observation bytes".len()
        + b"grid product a".len()
        + b"grid product b".len()) as u64;
    assert_eq!(staged.bytes, case_bytes + observation_bytes);
    assert!(staged.remote_inputs.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

/// 7a：staging 会把 parent_study 清掉（服务器引擎可能不认识），设计比较必须同样忽略它，
/// 否则带谱系的派生研究从服务器取回时会被判成设计不一致而拒绝。
#[test]
fn a_staged_study_without_parent_study_still_matches_the_local_design() {
    let (root, manifest) = observation_staging_fixture();
    assert!(
        manifest.spec.parent_study.is_some(),
        "fixture 必须带谱系，否则测不到 7a"
    );
    let staging = root.join("staging");
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &[], false).unwrap();
    let remote: study::spec::StudySpec = serde_json::from_value(staged.spec).unwrap();
    assert!(remote.parent_study.is_none(), "staging 必须剥掉谱系");
    assert!(
        same_study_design(&manifest.spec, &remote).unwrap(),
        "本地带谱系、服务器不带谱系仍是同一执行设计"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mapped_observation_sources_keep_the_server_path_and_are_not_copied() {
    let (root, manifest) = observation_staging_fixture();
    let maps = vec![parse_mapping(&format!("{}=/srv/shared", root.join("obs").display())).unwrap()];
    let staging = root.join("staging");
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &maps, false).unwrap();
    let staged_spec = serde_json::to_value(&staged.spec).unwrap();
    // 映射命中的观测保留服务器路径，不上传；目标源同一条规则。
    assert_eq!(
        staged_spec["observations"]["site"],
        json!("/srv/shared/site.nc")
    );
    assert_eq!(
        staged_spec["targets"][1]["source"]["path"],
        json!("/srv/shared/a/Pool.nc")
    );
    assert_eq!(
        staged_spec["targets"][2]["source"]["path"],
        json!("/srv/shared/b/Pool.nc")
    );
    assert!(!staging.join("obs/site-site.nc").exists());
    // 基础算例文件照常复制并计入 bytes；观测文件一个都没传。
    assert!(staged.bytes > 0);
    assert!(
        staging.join("obs").read_dir().unwrap().next().is_none(),
        "no observation file should be copied when every source is mapped"
    );
    assert_eq!(
        staged.remote_inputs,
        vec![
            "/srv/shared/a/Pool.nc".to_owned(),
            "/srv/shared/b/Pool.nc".to_owned(),
            "/srv/shared/site.nc".to_owned(),
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_missing_local_observation_source_says_what_to_do_instead() {
    let (root, mut manifest) = observation_staging_fixture();
    let missing = root.join("obs/nowhere.nc");
    manifest.spec.targets[1]
        .source
        .as_mut()
        .unwrap()
        .set_path(missing.to_string_lossy().into_owned());
    let staging = root.join("staging");
    let error = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &[], false).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("nowhere.nc"), "{text}");
    assert!(text.contains("--map"), "{text}");
    // 失败路径也要把暂存目录清掉，别把本机文件副本留在临时目录。
    assert!(!staging.exists());
    std::fs::remove_dir_all(root).unwrap();
}

fn frozen_target_hashes(
    manifest: &mut study::spec::Manifest,
) -> std::collections::BTreeMap<String, String> {
    for target in &manifest.spec.targets {
        if let Some(source) = &target.source {
            let file = crate::fingerprint::full_file(Path::new(source.path()), "target").unwrap();
            manifest
                .provenance
                .target_source_sha256
                .insert(target.key.clone(), file.sha256);
        }
    }
    manifest.provenance.target_source_sha256.clone()
}

#[test]
fn mapped_target_sources_are_checked_for_content_not_merely_existence() {
    let (root, mut manifest) = observation_staging_fixture();
    let frozen = frozen_target_hashes(&mut manifest);
    let maps = vec![parse_mapping(&format!("{}=/srv/shared", root.join("obs").display())).unwrap()];
    let staging = root.join("staging");
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &maps, false).unwrap();
    let checks = frozen_target_sources(&staged.spec, &frozen).unwrap();
    assert_eq!(checks.len(), 2, "{checks:?}");
    assert_eq!(checks[0].path, "/srv/shared/a/Pool.nc");
    assert_eq!(checks[1].path, "/srv/shared/b/Pool.nc");
    verify_server_target_sources(&checks, |path| {
        Ok(frozen[&checks.iter().find(|check| check.path == path).unwrap().key].clone())
    })
    .unwrap();

    let wrong = checks[0].path.clone();
    let error = verify_server_target_sources(&checks, |path| {
        Ok(if path == wrong {
            "0".repeat(64)
        } else {
            frozen[&checks.iter().find(|check| check.path == path).unwrap().key].clone()
        })
    })
    .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("differs"), "{text}");
    assert!(text.contains("Gpp"), "{text}");

    let error = verify_server_target_sources(&checks, |_| Ok("MISSING".into())).unwrap_err();
    assert!(format!("{error:#}").contains("missing"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn uploaded_target_sources_use_the_server_project_paths_for_the_same_check() {
    let (root, mut manifest) = observation_staging_fixture();
    let frozen = frozen_target_hashes(&mut manifest);
    let staging = root.join("staging");
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &[], false).unwrap();
    let checks = frozen_target_sources(&staged.spec, &frozen).unwrap();
    assert!(
        checks
            .iter()
            .all(|check| check.path.starts_with("/srv/colm/s/abc/obs/")),
        "{checks:?}"
    );
    verify_server_target_sources(&checks, |path| {
        assert!(
            staging
                .join("obs")
                .join(Path::new(path).file_name().unwrap())
                .is_file(),
            "the uploaded source must exist before its server hash is checked"
        );
        Ok(frozen[&checks.iter().find(|check| check.path == path).unwrap().key].clone())
    })
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_study_without_frozen_target_hashes_cannot_submit_them_to_a_server() {
    let (root, manifest) = observation_staging_fixture();
    let staging = root.join("staging");
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &[], false).unwrap();
    let error =
        frozen_target_sources(&staged.spec, &manifest.provenance.target_source_sha256).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("target observation"), "{text}");
    assert!(text.contains("create a new Study"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

fn target_source_file(path: &str, sha256: &str, bytes: u64) -> crate::fingerprint::FullInputFile {
    crate::fingerprint::FullInputFile {
        path: PathBuf::from(path),
        sha256: sha256.into(),
        bytes,
        category: "target-observation".into(),
    }
}

#[test]
fn target_observation_identity_is_compared_by_key_not_by_host_specific_paths() {
    let mut local: study::model::StudyInputIdentity =
        serde_json::from_value(json!({"schema":1,"base_cases":{},"observations":{}})).unwrap();
    local.target_sources.insert(
        "Gpp".into(),
        target_source_file("/Users/me/GPP.nc", &"a".repeat(64), 12),
    );
    let mut remote = local.clone();
    remote.target_sources.insert(
        "Gpp".into(),
        target_source_file("/srv/colm/obs/target-1-Gpp-GPP.nc", &"a".repeat(64), 12),
    );
    ensure_same_target_sources(&local, &remote).unwrap();
    remote.target_sources.insert(
        "Gpp".into(),
        target_source_file("/srv/colm/obs/target-1-Gpp-GPP.nc", &"b".repeat(64), 12),
    );
    let error = ensure_same_target_sources(&local, &remote).unwrap_err();
    assert!(format!("{error:#}").contains("Gpp"), "{error}");
    // 服务器研究有、本机没有的目标源同样拒绝，不能悄悄跳过。
    remote.target_sources.clear();
    let error = ensure_same_target_sources(&local, &remote).unwrap_err();
    assert!(format!("{error:#}").contains("did not record"), "{error}");
}

#[test]
fn dry_run_discards_staged_observation_copies() {
    let (root, manifest) = observation_staging_fixture();
    let staging = staging_dir(&manifest.id, std::process::id());
    let _ = std::fs::remove_dir_all(&staging);
    let staged = stage_study_assets(&manifest, &staging, "/srv/colm/s/abc", &[], false).unwrap();
    assert!(staged.bytes > 0);
    assert!(staging.join("obs").read_dir().unwrap().next().is_some());
    // dry-run 与任何失败路径共用同一个清理函数：本机观测副本不能留在 /tmp。
    discard_staging(&staging);
    assert!(!staging.exists());
    std::fs::remove_dir_all(root).unwrap();
}

// ---- P1-7b：按研究类型核验取回 ----

struct UncertaintyFixture {
    root: PathBuf,
    manifest: study::spec::Manifest,
    staged: PathBuf,
    remote_root: String,
    remote: Value,
    installed: study::state::StudyState,
}

fn uncertainty_member_ids(method: &str) -> Vec<String> {
    let count = match method {
        "oat" => 2,
        "morris" => 4,
        "lhs" => 4,
        other => panic!("unknown method {other}"),
    };
    (0..=count).map(|index| format!("m{index:06}")).collect()
}

fn write_uncertainty_results(staged: &Path, remote_root: &str, ids: &[String]) {
    for id in ids {
        let task = json!({
            "member": id, "site": "site",
            "outputs": [{"variable":"f_qle","units":"W/m2","count":3,"mean":1.5,"min":1.0,"max":2.0}],
            "calibration": [], "validation": [], "validation_errors": []
        });
        let bytes = serde_json::to_vec_pretty(&task).unwrap();
        let relative = format!("tasks/{id}/site.json");
        let path = staged.join("results").join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let proof = json!({
            "schema": 1,
            "execution": {"complete": true},
            "outputs": {},
            "result": {
                "path": format!("{remote_root}/results/{relative}"),
                "sha256": colm_kernel::sha256_hex(&bytes),
                "bytes": bytes.len(),
                "category": "evaluation"
            }
        });
        let proof_path = staged.join("results/provenance").join(id).join("site.json");
        std::fs::create_dir_all(proof_path.parent().unwrap()).unwrap();
        std::fs::write(proof_path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
    }
}

fn result_inventory_json(staged: &Path) -> Vec<Value> {
    let root = staged.join("results");
    // `regular_tree_files` 的 BTreeMap 键本身有序，顺序也与服务器的 `study-status.results` 无关。
    super::regular_tree_files(&root, false)
        .unwrap()
        .into_keys()
        .map(|relative| {
            let relative = relative.to_string_lossy().replace('\\', "/");
            let bytes = std::fs::metadata(root.join(&relative)).unwrap().len();
            json!({"path": relative, "bytes": bytes})
        })
        .collect()
}

fn uncertainty_fixture(method: &str) -> UncertaintyFixture {
    let root = temporary_dir("uncertainty");
    let local = root.join("proj/.colm/studies/s-local");
    std::fs::create_dir_all(local.join("samples")).unwrap();
    let obs = root.join("obs.nc");
    let obs_bytes = b"uncertainty observation bytes";
    std::fs::write(&obs, obs_bytes).unwrap();
    let ids = uncertainty_member_ids(method);
    let mut csv = String::from("member,baseline,generation,candidate,DEF_TUNING_CNFAC\n");
    let mut members = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let value = 0.5 + index as f64 * 0.01;
        csv.push_str(&format!("{id},{},0,{index},{value}\n", index == 0));
        members.push(study::spec::MemberPlan {
            id: id.clone(),
            generation: 0,
            candidate_index: index,
            baseline: index == 0,
            parameters: [("DEF_TUNING_CNFAC".to_owned(), value)]
                .into_iter()
                .collect(),
        });
    }
    std::fs::write(local.join("samples/design.csv"), &csv).unwrap();
    let spec: study::spec::StudySpec = serde_json::from_value(json!({
        "kind": "uncertainty",
        "method": method,
        "base_cases": ["site"],
        "observations": {"site": obs},
        "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
        "outputs": ["f_qle"],
        "budget": {"jobs": 1}
    }))
    .unwrap();
    let remote_root = "/srv/colm/s/abc/.colm/studies/s-remote".to_owned();
    let mut provenance = study::spec::ManifestProvenance {
        samples_sha256: colm_kernel::sha256_hex(csv.as_bytes()),
        // 写回版本前会过 `verify_frozen_inputs`：输出哈希必须是 spec 的真实哈希（空值会被跳过）。
        outputs_sha256: colm_kernel::sha256_hex(&serde_json::to_vec(&spec.outputs).unwrap()),
        ..Default::default()
    };
    provenance
        .observation_sha256
        .insert("site".into(), colm_kernel::sha256_hex(obs_bytes));
    let manifest = study::spec::Manifest {
        schema_version: 1,
        id: "s-local".into(),
        root: local.to_string_lossy().into_owned(),
        created_unix: 0,
        spec,
        members,
        provenance: provenance.clone(),
    };
    // 服务器上的清单：同一份设计，路径换成服务器路径；状态属于服务器研究编号。
    let mut remote_spec = serde_json::to_value(&manifest.spec).unwrap();
    remote_spec["observations"]["site"] = json!("/srv/colm/s/abc/obs/site-obs.nc");
    let mut remote_manifest = serde_json::to_value(&manifest).unwrap();
    remote_manifest["id"] = json!("s-remote");
    remote_manifest["root"] = json!(remote_root);
    remote_manifest["spec"] = remote_spec;
    let mut tasks = serde_json::Map::new();
    let mut candidates = serde_json::Map::new();
    for member in &manifest.members {
        tasks.insert(
            format!("{}/site", member.id),
            json!({
                "member": member.id,
                "site": "site",
                "case_dir": format!("{remote_root}/members/{}/site", member.id),
                "status": "succeeded"
            }),
        );
        candidates.insert(
            member.id.clone(),
            json!({"generation": 0, "feasible": true, "calibration": null, "validation": null, "reason": null}),
        );
    }
    let state = json!({
        "schema_version": 1,
        "study_id": "s-remote",
        "status": "completed",
        "generation": 0,
        "tasks": tasks,
        "best_member": null,
        "best_objective": null,
        "completed_candidates": manifest.members.len() - 1,
        "population": [],
        "candidates": candidates,
        "no_improvement_generations": 0,
        "warnings": []
    });
    // 下载暂存：样本与结果树都从服务器搬下来的字节。
    let staged = root.join("staged");
    std::fs::create_dir_all(staged.join("samples")).unwrap();
    std::fs::write(staged.join("samples/design.csv"), &csv).unwrap();
    write_uncertainty_results(&staged, &remote_root, &ids);
    std::fs::create_dir_all(staged.join("results/envelopes/site")).unwrap();
    std::fs::write(
        staged.join("results/envelopes/site/f_qle.json"),
        br#"{"site":"site","variable":"f_qle","time":[0],"p05":[1.0]}"#,
    )
    .unwrap();
    let inventory = result_inventory_json(&staged);
    let remote = json!({
        "manifest": remote_manifest,
        "state": state,
        "results": inventory,
        "model_version": {"model_revision": null}
    });
    let transplanted = transplant_state(state, &manifest.id, &remote_root, &local);
    let installed = serde_json::from_value(transplanted).unwrap();
    UncertaintyFixture {
        root,
        manifest,
        staged,
        remote_root,
        remote,
        installed,
    }
}

#[test]
fn uncertainty_fetch_verification_accepts_oat_lhs_and_morris_without_a_best_member() {
    for method in ["oat", "lhs", "morris"] {
        let fixture = uncertainty_fixture(method);
        let evidence = uncertainty_fetch_evidence(
            &fixture.manifest,
            &fixture.remote_root,
            &fixture.remote,
            &fixture.staged,
            &fixture.installed,
        )
        .unwrap_or_else(|error| panic!("{method}: {error:#}"));
        assert_eq!(evidence["schema"], 2, "{method}");
        assert_eq!(evidence["kind"], "uncertainty", "{method}");
        assert_eq!(evidence["method"], method, "{method}");
        // 不确定性研究按设计没有最优成员：证据里不出现也不声称最优成员。
        assert!(evidence.get("best_member").is_none(), "{method}");
        assert!(
            evidence["selection"].get("best_member").is_none(),
            "{method}"
        );
        let member_results = evidence["member_results"].as_object().unwrap();
        assert_eq!(member_results.len(), fixture.manifest.members.len());
        for member in &fixture.manifest.members {
            let entry = member_results
                .get(&member.id)
                .unwrap_or_else(|| panic!("{method}: missing {}", member.id));
            assert!(entry["tree_sha256"].as_str().unwrap().len() == 64);
        }
        std::fs::remove_dir_all(fixture.root).unwrap();
    }
}

#[test]
fn tampered_uncertainty_member_results_are_rejected() {
    let fixture = uncertainty_fixture("lhs");
    let result = fixture.staged.join("results/tasks/m000001/site.json");
    let original = std::fs::read(&result).unwrap();
    // 改成同样长度的字节：文件清单（路径+大小）对得上，只有内容身份能发现篡改。
    let tampered = String::from_utf8(original)
        .unwrap()
        .replace("\"W/m2\"", "\"W/m3\"");
    assert_ne!(
        String::from_utf8(std::fs::read(&result).unwrap()).unwrap(),
        tampered
    );
    std::fs::write(&result, tampered.as_bytes()).unwrap();
    let error = uncertainty_fetch_evidence(
        &fixture.manifest,
        &fixture.remote_root,
        &fixture.remote,
        &fixture.staged,
        &fixture.installed,
    )
    .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("evaluation result"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn tuning_evidence_still_binds_the_best_member_and_rejects_a_tampered_state() {
    let state = json!({
        "schema_version": 1,
        "study_id": "s-local",
        "status": "completed",
        "generation": 1,
        "tasks": {
            "m000001/site": {
                "member": "m000001", "site": "site",
                "case_dir": "/srv/colm/s/abc/.colm/studies/s-remote/members/m000001/site",
                "status": "succeeded"
            }
        },
        "best_member": "m000001",
        "best_objective": 0.5
    });
    let typed: study::state::StudyState = serde_json::from_value(state.clone()).unwrap();
    let state_sha256 = colm_kernel::sha256_hex(&serde_json::to_vec(&typed).unwrap());
    let evidence = json!({
        "schema": 1,
        "root": "/srv/colm/s/abc/.colm/studies/s-remote",
        "study_id": "s-local",
        "state": state,
        "state_sha256": state_sha256,
        "best_member": "m000001"
    });
    evidence_state(&evidence, "/srv/colm/s/abc/.colm/studies/s-remote").unwrap();
    let mut wrong_best = evidence.clone();
    wrong_best["best_member"] = json!("m000002");
    let error = evidence_state(&wrong_best, "/srv/colm/s/abc/.colm/studies/s-remote").unwrap_err();
    assert!(format!("{error}").contains("member identity"), "{error}");
    let mut tampered = evidence.clone();
    tampered["state"]["tasks"]["m000001/site"]["case_dir"] = json!("/elsewhere");
    let error = evidence_state(&tampered, "/srv/colm/s/abc/.colm/studies/s-remote").unwrap_err();
    assert!(format!("{error}").contains("state hash"), "{error}");
    // 旧比较证据（schema 1）没有 kind 字段：仍按率定接收；明确写错类型才拒绝。
    ensure_evidence_kind(&evidence, study::spec::StudyKind::Tuning).unwrap();
    let mut declared = evidence.clone();
    declared["kind"] = json!("uncertainty");
    assert!(ensure_evidence_kind(&declared, study::spec::StudyKind::Tuning).is_err());
}

#[test]
fn old_server_evidence_missing_the_new_identities_is_rejected_explicitly() {
    let mut fixture = uncertainty_fixture("lhs");
    // 服务器清单没带目标观测身份：本机有格点目标观测时无法核对，必须明确拒绝而不是悄悄跳过。
    // fixture 的 provenance 本来就不序列化空的 target_source_sha256，这里显式去掉以覆盖"旧证据"。
    let _ = fixture.remote["manifest"]["provenance"]
        .as_object_mut()
        .unwrap()
        .remove("target_source_sha256");
    let mut spec = serde_json::to_value(&fixture.manifest.spec).unwrap();
    spec["targets"] = json!([{
        "key": "Gpp", "variable": "GPP", "from": 0, "to": 3600, "min_pairs": 2,
        "source": {"kind": "grid", "path": "/local/GPP.nc", "variable": "GPP"}
    }]);
    fixture.manifest.spec = serde_json::from_value(spec).unwrap();
    fixture
        .manifest
        .provenance
        .target_source_sha256
        .insert("Gpp".into(), "local-target-observation-sha256".into());
    let error = uncertainty_fetch_evidence(
        &fixture.manifest,
        &fixture.remote_root,
        &fixture.remote,
        &fixture.staged,
        &fixture.installed,
    )
    .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("target observation"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();

    // 连服务器清单都没有（更旧的 status 输出）：同样明确拒绝。
    let mut fixture = uncertainty_fixture("oat");
    let _ = fixture.remote.as_object_mut().unwrap().remove("manifest");
    let error = uncertainty_fetch_evidence(
        &fixture.manifest,
        &fixture.remote_root,
        &fixture.remote,
        &fixture.staged,
        &fixture.installed,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("manifest"), "{error}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn tuning_fetch_compares_target_source_hashes_from_the_remote_manifest() {
    let (root, mut local) = observation_staging_fixture();
    local
        .provenance
        .target_source_sha256
        .insert("Gpp".into(), "a".repeat(64));
    let mut remote = local.clone();
    ensure_remote_target_sources(&local, &remote).unwrap();
    remote
        .provenance
        .target_source_sha256
        .insert("Gpp".into(), "b".repeat(64));
    let error = ensure_remote_target_sources(&local, &remote).unwrap_err();
    assert!(format!("{error:#}").contains("Gpp"), "{error}");
    remote.provenance.target_source_sha256.clear();
    let error = ensure_remote_target_sources(&local, &remote).unwrap_err();
    assert!(format!("{error:#}").contains("did not record"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

// ---- P1-7c：取回的研究写回版本（统一本地/远端成员核验） ----

fn copy_tree_into(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree_into(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// 假服务器：只回答成员写回核验要问的问题（身份、作业状态、研究目录、study-status、
/// 率定比较证据、小样本文件），不连 SSH。真实 SSH 端到端不跑。
struct FakeStudyRemote {
    unreachable: bool,
    exit_code: Option<i32>,
    root: Option<String>,
    study_status: Value,
    comparison_evidence: Option<Value>,
    /// 模拟服务器 CLI 拒绝 `--member`（旧引擎的 usage/unknown-argument 错误）。
    comparison_failure: Option<String>,
    /// 每次 `comparison_evidence` 收到的成员参数：用来断言 best/baseline 不带、选定成员带。
    comparison_members: std::cell::RefCell<Vec<Option<String>>>,
    files: std::collections::BTreeMap<String, String>,
    reads: std::cell::RefCell<Vec<String>>,
}

impl StudyRemote for FakeStudyRemote {
    fn verify_identity(&self, _record: &StudyRecord) -> Result<()> {
        if self.unreachable {
            bail!("ssh: connect to host: Connection refused");
        }
        Ok(())
    }

    fn job_status(&self, _record: &StudyRecord) -> Result<job::Status> {
        Ok(job::Status {
            state: match self.exit_code {
                None => job::State::Running,
                Some(exit_code) => job::State::Finished { exit_code },
            },
            phase: None,
            detail: None,
            log_tail: String::new(),
        })
    }

    fn study_root(&self, _record: &StudyRecord) -> Result<Option<String>> {
        Ok(self.root.clone())
    }

    fn study_status(&self, _record: &StudyRecord, _study_root: &str) -> Result<Value> {
        Ok(self.study_status.clone())
    }

    fn comparison_evidence(
        &self,
        _record: &StudyRecord,
        _study_root: &str,
        member: Option<&str>,
    ) -> Result<Value> {
        self.comparison_members
            .borrow_mut()
            .push(member.map(str::to_owned));
        if let Some(message) = &self.comparison_failure {
            bail!("{message}");
        }
        self.comparison_evidence
            .clone()
            .context("the fake server has no comparison evidence")
    }

    fn read_text_file(&self, path: &str) -> Result<String> {
        self.reads.borrow_mut().push(path.to_owned());
        self.files
            .get(path)
            .cloned()
            .with_context(|| format!("no file {path} on the fake server"))
    }
}

struct RemoteStudyFixture {
    root: PathBuf,
    manifest: study::spec::Manifest,
    local: PathBuf,
    fake: FakeStudyRemote,
}

/// 把一次 P1-7b fixture 摆成"已取回"的样子：本机研究目录有 manifest/checkpoint/样本/结果树/
/// 取回证据/远程记录，假服务器持有同一份服务器状态。
fn remote_fetched_study(method: &str) -> RemoteStudyFixture {
    let fixture = uncertainty_fixture(method);
    let local = PathBuf::from(&fixture.manifest.root)
        .canonicalize()
        .unwrap();
    std::fs::write(
        local.join("manifest.json"),
        serde_json::to_vec_pretty(&fixture.manifest).unwrap(),
    )
    .unwrap();
    copy_tree_into(&fixture.staged.join("samples"), &local.join("samples"));
    copy_tree_into(&fixture.staged.join("results"), &local.join("results"));
    std::fs::create_dir_all(local.join("checkpoints/state")).unwrap();
    study::checkpoint::write_next(&local.join("checkpoints/state"), &fixture.installed).unwrap();
    // 与 remote-study-fetch 一样，核验通过时写下取回证据。
    let evidence = uncertainty_fetch_evidence(
        &fixture.manifest,
        &fixture.remote_root,
        &fixture.remote,
        &fixture.staged,
        &fixture.installed,
    )
    .unwrap();
    std::fs::write(
        local.join(EVIDENCE),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    let record = StudyRecord {
        ssh_auth: None,
        host: "server".into(),
        root: "/srv/colm".into(),
        job: "study-1".into(),
        remote_project: "/srv/colm/s/abc".into(),
        engine: "0123456789abcdef".into(),
        submitted_at: 1,
        scheduler: "bare".into(),
        scheduler_id: None,
        provenance: json!({"schema": 1, "model_revision": null}),
        fetched_at: Some(1),
    };
    std::fs::write(
        local.join(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    let mut files = std::collections::BTreeMap::new();
    files.insert(
        format!("{}/samples/design.csv", fixture.remote_root),
        String::from_utf8(std::fs::read(local.join("samples/design.csv")).unwrap()).unwrap(),
    );
    let fake = FakeStudyRemote {
        unreachable: false,
        exit_code: Some(0),
        root: Some(fixture.remote_root.clone()),
        study_status: fixture.remote.clone(),
        comparison_evidence: None,
        comparison_failure: None,
        comparison_members: std::cell::RefCell::new(Vec::new()),
        files,
        reads: std::cell::RefCell::new(Vec::new()),
    };
    RemoteStudyFixture {
        root: fixture.root,
        manifest: fixture.manifest,
        local,
        fake,
    }
}

#[test]
fn remote_fetched_members_are_verified_on_the_server() {
    let fixture = remote_fetched_study("lhs");
    let verified =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap();
    assert_eq!(verified.member.id, "m000001");
    assert_eq!(verified.evidence["location"]["kind"], "server");
    assert_eq!(verified.evidence["location"]["host"], "server");
    assert_eq!(verified.evidence["location"]["job"], "study-1");
    assert_eq!(verified.evidence["member"]["id"], "m000001");
    assert_eq!(verified.evidence["tasks"]["site"]["status"], "succeeded");
    assert_eq!(
        verified.evidence["results"]["site"]["task"]["path"],
        "tasks/m000001/site.json"
    );
    assert_eq!(
        verified.evidence["results"]["site"]["task"]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(verified.evidence["stored_evidence"], "matched");
    assert_eq!(
        verified.evidence["results_tree_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    // 成员参数核对只读服务器上的小样本文件，不拉 history。
    assert!(
        fixture
            .fake
            .reads
            .borrow()
            .iter()
            .any(|path| path.ends_with("/samples/design.csv")),
        "reads: {:?}",
        fixture.fake.reads.borrow()
    );
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn remote_members_still_verify_without_the_stored_fetch_evidence() {
    let fixture = remote_fetched_study("oat");
    std::fs::remove_file(fixture.local.join(EVIDENCE)).unwrap();
    let verified =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap();
    assert_eq!(verified.evidence["stored_evidence"], "absent");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn tampered_remote_member_result_is_rejected() {
    let fixture = remote_fetched_study("lhs");
    let result = fixture.local.join("results/tasks/m000001/site.json");
    let original = std::fs::read_to_string(&result).unwrap();
    // 同样长度的替换：结果文件清单（大小）对得上，只有内容哈希能发现。
    let tampered = original.replace("\"W/m2\"", "\"W/m3\"");
    assert_ne!(original, tampered);
    std::fs::write(&result, tampered).unwrap();
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("evaluation result"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn tampered_fetch_evidence_is_rejected() {
    let fixture = remote_fetched_study("lhs");
    let path = fixture.local.join(EVIDENCE);
    let mut evidence: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    evidence["results_tree_sha256"] = json!("tampered");
    std::fs::write(&path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("evidence"), "{text}");
    assert!(text.contains("fetch"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn tampered_local_member_parameters_are_rejected() {
    let fixture = remote_fetched_study("morris");
    let samples = fixture.local.join("samples/design.csv");
    let original = std::fs::read_to_string(&samples).unwrap();
    let tampered = original.replacen(",0.51", ",0.52", 1);
    assert_ne!(original, tampered);
    std::fs::write(&samples, tampered).unwrap();
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("sample"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn an_unreachable_server_is_reported_actionably() {
    let mut fixture = remote_fetched_study("lhs");
    fixture.fake.unreachable = true;
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("unreachable"), "{text}");
    assert!(text.contains("server"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_failed_server_job_cannot_be_written_back() {
    let mut fixture = remote_fetched_study("lhs");
    fixture.fake.exit_code = Some(2);
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("failed with exit code 2"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_missing_server_study_root_is_reported() {
    let mut fixture = remote_fetched_study("lhs");
    fixture.fake.root = None;
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("did not create the Study"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

/// 像 `remote-study-fetch` 成功时那样，把来源随证据落进研究目录（不动活动记录）。
fn persist_fixture_source(fixture: &RemoteStudyFixture) {
    let record = read_record(&fixture.local).unwrap();
    let evidence: Value =
        serde_json::from_slice(&std::fs::read(fixture.local.join(EVIDENCE)).unwrap()).unwrap();
    write_study_source(
        &fixture.local.join(SOURCE),
        &record,
        &fixture.manifest,
        &evidence,
    )
    .unwrap();
}

#[test]
fn an_archived_remote_record_still_verifies_members_from_the_persisted_source() {
    let fixture = remote_fetched_study("lhs");
    persist_fixture_source(&fixture);
    // GUI 的“归档”就是把活动记录改名留档；来源文件不随它移动。
    let archived = fixture.local.join(".colm-remote-study-closed-123.json");
    std::fs::rename(fixture.local.join(RECORD), &archived).unwrap();
    let verified =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap();
    assert_eq!(verified.member.id, "m000001");
    assert_eq!(verified.evidence["location"]["kind"], "server");
    assert_eq!(
        verified.evidence["verification"]["record_source"],
        "archived-source"
    );
    // 归档后的来源仍要与那次取回证据交叉核对。
    assert_eq!(verified.evidence["stored_evidence"], "matched");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn an_active_record_must_agree_with_its_persisted_source() {
    let fixture = remote_fetched_study("oat");
    persist_fixture_source(&fixture);
    let mut record = read_record(&fixture.local).unwrap();
    record.job = "study-2".into();
    std::fs::write(
        fixture.local.join(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("persisted source"), "{text}");
    assert!(text.contains("disagree"), "{text}");
    assert!(text.contains("job"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

/// 取回成功后记录里写下的模型版本身份（fixture 默认只有 model_revision）。
fn stamp_workspace_version(study_dir: &Path, name: &str, head: &str) {
    let mut record = read_record(study_dir).unwrap();
    record.provenance["version"] = json!({
        "kind": "workspace",
        "name": name,
        "head": head,
        "base_commit": "fedcba98",
    });
    std::fs::write(
        study_dir.join(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
}

/// 归档后（活动记录改名、来源原位保留）仍要能从来源读出模型版本身份，
/// 否则 `study-compare` 与证据页会把修改版结果错认成官方版/未知。
#[test]
fn an_archived_remote_record_still_reports_the_model_version_from_the_persisted_source() {
    let fixture = remote_fetched_study("lhs");
    stamp_workspace_version(&fixture.local, "snow", "0123456789abcdef");
    persist_fixture_source(&fixture);
    let expected = Some(RecordedModelVersion::Workspace {
        name: "snow".into(),
        head: "0123456789abcdef".into(),
        base_commit: Some("fedcba98".into()),
    });
    // 活动记录与来源同时存在：读活动记录。
    assert_eq!(recorded_model_version(&fixture.local).unwrap(), expected);
    std::fs::rename(
        fixture.local.join(RECORD),
        fixture.local.join(".colm-remote-study-closed-123.json"),
    )
    .unwrap();
    // 只剩来源：必须仍能给出同一身份。
    assert_eq!(recorded_model_version(&fixture.local).unwrap(), expected);
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn an_archived_official_source_still_reports_the_official_version() {
    let fixture = remote_fetched_study("lhs");
    let mut record = read_record(&fixture.local).unwrap();
    record.provenance["version"] = json!({"kind": "official"});
    std::fs::write(
        fixture.local.join(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    persist_fixture_source(&fixture);
    std::fs::rename(
        fixture.local.join(RECORD),
        fixture.local.join(".colm-remote-study-closed-123.json"),
    )
    .unwrap();
    assert_eq!(
        recorded_model_version(&fixture.local).unwrap(),
        Some(RecordedModelVersion::Official)
    );
    std::fs::remove_dir_all(fixture.root).unwrap();
}

/// `study-compare` 的完整输出：两侧活动记录都归档后，仍按来源里的修改版名称/提交标记，
/// 而不是退回官方版。
#[test]
fn study_compare_still_names_archived_workspace_versions() {
    let left = remote_fetched_study("lhs");
    stamp_workspace_version(&left.local, "snow", "0123456789abcdef");
    persist_fixture_source(&left);
    std::fs::rename(left.local.join(RECORD), left.local.join("closed-left.json")).unwrap();
    let right = remote_fetched_study("lhs");
    stamp_workspace_version(&right.local, "rain", "fedcba9876543210");
    persist_fixture_source(&right);
    std::fs::rename(
        right.local.join(RECORD),
        right.local.join("closed-right.json"),
    )
    .unwrap();
    let result = crate::study_version::compare(&left.local, &right.local).unwrap();
    assert_eq!(result["a"]["version"], "snow");
    assert_eq!(result["a"]["head"], "0123456789abcdef");
    assert_eq!(result["b"]["version"], "rain");
    assert_eq!(result["b"]["head"], "fedcba9876543210");
    std::fs::remove_dir_all(left.root).unwrap();
    std::fs::remove_dir_all(right.root).unwrap();
}

#[test]
fn a_record_and_source_that_disagree_about_the_model_version_are_refused() {
    let fixture = remote_fetched_study("lhs");
    stamp_workspace_version(&fixture.local, "snow", "0123456789abcdef");
    persist_fixture_source(&fixture);
    // 记录与来源同属一次取回后才可能版本不同；模拟记录被另一份取回记录替换。
    stamp_workspace_version(&fixture.local, "rain", "89abcdef01234567");
    let error = recorded_model_version(&fixture.local).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("disagree"), "{text}");
    assert!(text.contains("model version"), "{text}");
    assert!(text.contains("record"), "{text}");
    assert!(text.contains("source"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn without_record_or_source_the_recorded_model_version_is_absent() {
    let fixture = remote_fetched_study("lhs");
    std::fs::remove_file(fixture.local.join(RECORD)).unwrap();
    // fixture 默认没有 SOURCE；连同证据一起清掉，模拟一份从未取回的本地研究。
    let _ = std::fs::remove_file(fixture.local.join(SOURCE));
    assert_eq!(recorded_model_version(&fixture.local).unwrap(), None);
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_persisted_source_must_match_its_fetch_evidence() {
    let fixture = remote_fetched_study("lhs");
    persist_fixture_source(&fixture);
    std::fs::rename(
        fixture.local.join(RECORD),
        fixture.local.join("closed.json"),
    )
    .unwrap();
    let evidence = fixture.local.join(EVIDENCE);
    let mut bytes = std::fs::read(&evidence).unwrap();
    let last = bytes.len() - 1;
    bytes[last] = b' ';
    std::fs::write(&evidence, bytes).unwrap();
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("does not match its fetch evidence"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn without_record_or_source_the_local_member_verification_still_applies() {
    let fixture = remote_fetched_study("lhs");
    std::fs::remove_file(fixture.local.join(RECORD)).unwrap();
    std::fs::remove_file(fixture.local.join(EVIDENCE)).unwrap();
    let error =
        verified_study_member_with(&fixture.local, "m000001", Some(&fixture.fake)).unwrap_err();
    // 没有记录/来源时绝不悄悄走服务器；本地分支也不会问假服务器。
    assert!(fixture.fake.reads.borrow().is_empty());
    let text = format!("{error:#}");
    assert!(!text.contains("remote submission record"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_missing_remote_record_is_reported_instead_of_falling_back_to_local_outputs() {
    let fixture = remote_fetched_study("lhs");
    std::fs::remove_file(fixture.local.join(RECORD)).unwrap();
    let error = verified_study_member(&fixture.local, "m000001").unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("remote submission record"), "{text}");
    assert!(text.contains("fetch the Study again"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_missing_remote_member_output_is_reported() {
    let fixture = remote_fetched_study("lhs");
    let member = study::generation::load_members(&fixture.manifest)
        .unwrap()
        .into_iter()
        .find(|member| member.id == "m000001")
        .unwrap();
    let error = member_result_files(
        &fixture.manifest,
        &member,
        &fixture.local,
        &std::collections::BTreeMap::new(),
        true,
    )
    .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("missing on the server"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

// ---- 指定成员写回：服务器现场比较只重算 best+baseline ----

struct TuningRemoteFixture {
    root: PathBuf,
    manifest: study::spec::Manifest,
    local: PathBuf,
    best: String,
    selected: String,
    fake: FakeStudyRemote,
}

fn write_tuning_member_result(local: &Path, remote_root: &str, member: &str) {
    let result = json!({
        "member": member,
        "site": "site",
        "outputs": [],
        "calibration": [{
            "key": "Qle",
            "site": "site",
            "variable": "Qle",
            "period": "calibration",
            "metric": "nrmse",
            "weight": 1.0,
            "min_pairs": 2,
            "pairs": 3,
            "value": 0.5,
            "observation_sd": 1.0,
            "loss": 0.5,
            "model_mean": 0.0,
            "observation_mean": 0.0,
            "support_hash": "fixture-pair-support"
        }],
        "validation": [],
    });
    let bytes = serde_json::to_vec_pretty(&result).unwrap();
    let relative = format!("tasks/{member}/site.json");
    let path = local.join("results").join(&relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let proof = json!({
        "schema": 1,
        "execution": {"complete": true},
        "outputs": {},
        "result": {
            "path": format!("{remote_root}/results/{relative}"),
            "sha256": colm_kernel::sha256_hex(&bytes),
            "bytes": bytes.len(),
            "category": "evaluation"
        }
    });
    let proof_path = local
        .join("results/provenance")
        .join(member)
        .join("site.json");
    std::fs::create_dir_all(proof_path.parent().unwrap()).unwrap();
    std::fs::write(proof_path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
}

/// 一份“已取回”的率定研究：真实 `study-create` 产生的清单/样本/冻结输入，外加一份完整的
/// 服务器状态与假比较证据。只用来验证写回分流，不模拟模型或真实分数。
fn tuning_fetched_study() -> TuningRemoteFixture {
    let (root, manifest, _staged, _initial) = fetch_fixture();
    let local = Path::new(&manifest.root).to_path_buf();
    study::model::capture_inputs(&manifest).unwrap();
    let best = manifest
        .members
        .iter()
        .find(|member| !member.baseline)
        .unwrap()
        .id
        .clone();
    let selected = manifest
        .members
        .iter()
        .filter(|member| !member.baseline)
        .nth(1)
        .unwrap()
        .id
        .clone();
    let remote_root = "/srv/colm/s/abc/.colm/studies/s-remote".to_owned();
    let mut tasks = std::collections::BTreeMap::new();
    let mut candidates = std::collections::BTreeMap::new();
    for (index, member) in manifest.members.iter().enumerate() {
        tasks.insert(
            study::state::task_id(&member.id, "site"),
            study::state::TaskState {
                member: member.id.clone(),
                site: "site".into(),
                case_dir: local
                    .join("members")
                    .join(&member.id)
                    .join("site")
                    .to_string_lossy()
                    .into_owned(),
                status: study::state::TaskStatus::Succeeded,
                stage: None,
                reason: None,
                objective: None,
                validation_objective: None,
                process: None,
            },
        );
        candidates.insert(
            member.id.clone(),
            study::state::CandidateState {
                generation: member.generation,
                feasible: true,
                calibration: Some(index as f64),
                validation: None,
                reason: None,
            },
        );
    }
    let mut state =
        study::state::StudyState::new(manifest.id.clone(), tasks.values().cloned()).unwrap();
    state.status = study::state::StudyStatus::Completed;
    state.best_member = Some(best.clone());
    state.best_objective = Some(1.0);
    state.completed_candidates = manifest.members.len().saturating_sub(1);
    state.candidates = candidates;
    study::checkpoint::write_next(&local.join("checkpoints/state"), &state).unwrap();

    // `resolve_apply_member` 会重算率定状态，要求每个已成功任务都有评估结果；全部写上。
    for member in &manifest.members {
        write_tuning_member_result(&local, &remote_root, &member.id);
    }
    let results_sha256 = crate::study_version::comparison_results_sha256(&local).unwrap();
    let mut server_state = state.clone();
    for task in server_state.tasks.values_mut() {
        task.case_dir = format!("{remote_root}/members/{}/site", task.member);
    }
    let state_value = serde_json::to_value(&server_state).unwrap();
    // 证据里的哈希按服务器的类型化状态算；`Value` 的键序与结构体字段序不同，不能拿
    // `to_vec(&state_value)` 冒充同一份字节。
    let state_sha256 = colm_kernel::sha256_hex(&serde_json::to_vec(&server_state).unwrap());
    let input_identity = study::model::verified_inputs(&manifest).unwrap();
    // 支持 `--member` 的服务器会附带该成员自己的证据条目；分数取自同一份服务器状态，
    // 摘要按服务器结果树算——与真实命令的输出保持同一形状。
    let selected_candidate = state.candidates.get(&selected).unwrap();
    let selected_evidence =
        crate::study_version::member_evidence_digest(&results_sha256, &selected).unwrap();
    let evidence = json!({
        "schema": 1,
        "root": remote_root,
        "study_id": manifest.id,
        "spec": manifest.spec,
        "state": state_value,
        "state_sha256": state_sha256,
        "best_member": best,
        "input_identity": input_identity,
        "model_revision": Value::Null,
        "results_sha256": results_sha256,
        "forcing_content_policy": "excluded-v1",
        "forcing_contents_verified": false,
        "members": [{
            "id": selected,
            "kind": "selected",
            "scores": {
                "feasible": selected_candidate.feasible,
                "calibration": selected_candidate.calibration,
                "validation": selected_candidate.validation,
            },
            "evidence_sha256": selected_evidence,
            "metrics_recomputed": false,
        }],
    });
    let record = StudyRecord {
        ssh_auth: None,
        host: "server".into(),
        root: "/srv/colm".into(),
        job: "study-1".into(),
        remote_project: "/srv/colm/s/abc".into(),
        engine: "0123456789abcdef".into(),
        submitted_at: 1,
        scheduler: "bare".into(),
        scheduler_id: None,
        provenance: json!({"schema": 1, "model_revision": null}),
        fetched_at: Some(1),
    };
    std::fs::write(
        local.join(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    let sample = std::fs::read_to_string(local.join("samples/g000000.csv")).unwrap();
    let files = [(format!("{remote_root}/samples/g000000.csv"), sample)]
        .into_iter()
        .collect();
    let fake = FakeStudyRemote {
        unreachable: false,
        exit_code: Some(0),
        root: Some(remote_root),
        study_status: Value::Null,
        comparison_evidence: Some(evidence),
        comparison_failure: None,
        comparison_members: std::cell::RefCell::new(Vec::new()),
        files,
        reads: std::cell::RefCell::new(Vec::new()),
    };
    TuningRemoteFixture {
        root,
        manifest,
        local,
        best,
        selected,
        fake,
    }
}

/// 给已取回的率定 fixture 加一个格点目标源，并同步清单 provenance 与假服务器证据。
/// 只动元数据，不重跑 `study-create`：模拟“建研究时就带目标观测”的合法研究。
fn add_tuning_target_source(fixture: &mut TuningRemoteFixture, contents: &[u8]) {
    let directory = fixture.root.join("targets");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("gpp.nc");
    std::fs::write(&source, contents).unwrap();
    fixture.manifest.spec.targets.push(
        serde_json::from_value(json!({
            "key": "Gpp",
            "variable": "GPP",
            "from": 0,
            "to": 3600,
            "min_pairs": 2,
            "source": {"kind": "grid", "path": source, "variable": "GPP"}
        }))
        .unwrap(),
    );
    fixture.manifest.provenance.spec_sha256 =
        colm_kernel::sha256_hex(&serde_json::to_vec(&fixture.manifest.spec).unwrap());
    fixture.manifest.provenance.required_targets_sha256 =
        colm_kernel::sha256_hex(&serde_json::to_vec(&fixture.manifest.spec.targets).unwrap());
    let file = crate::fingerprint::full_file(&source, "target-observation").unwrap();
    fixture
        .manifest
        .provenance
        .target_source_sha256
        .insert("Gpp".into(), file.sha256);
    std::fs::write(
        fixture.local.join("manifest.json"),
        serde_json::to_vec_pretty(&fixture.manifest).unwrap(),
    )
    .unwrap();
    study::model::capture_inputs(&fixture.manifest).unwrap();
    // 每个已成功任务都必须给新目标一行评估结果，否则 `resolve_apply_member` 重算状态时会拒绝；
    // 结果字节变了，评估身份与证据里的结果树哈希也要同步。
    for member in &fixture.manifest.members {
        let task_path = fixture
            .local
            .join("results/tasks")
            .join(&member.id)
            .join("site.json");
        let mut result: Value =
            serde_json::from_slice(&std::fs::read(&task_path).unwrap()).unwrap();
        result["calibration"].as_array_mut().unwrap().push(json!({
            "key": "Gpp",
            "site": "site",
            "variable": "GPP",
            "period": "calibration",
            "metric": "nrmse",
            "weight": 1.0,
            "min_pairs": 2,
            "pairs": 3,
            "value": 0.4,
            "observation_sd": 1.0,
            "loss": 0.4,
            "model_mean": 0.0,
            "observation_mean": 0.0,
            "support_hash": "fixture-pair-support"
        }));
        let bytes = serde_json::to_vec_pretty(&result).unwrap();
        std::fs::write(&task_path, &bytes).unwrap();
        let proof_path = fixture
            .local
            .join("results/provenance")
            .join(&member.id)
            .join("site.json");
        let mut proof: Value =
            serde_json::from_slice(&std::fs::read(&proof_path).unwrap()).unwrap();
        proof["result"]["sha256"] = json!(colm_kernel::sha256_hex(&bytes));
        proof["result"]["bytes"] = json!(bytes.len());
        std::fs::write(&proof_path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
    }
    let evidence = fixture.fake.comparison_evidence.as_mut().unwrap();
    evidence["spec"] = serde_json::to_value(&fixture.manifest.spec).unwrap();
    evidence["input_identity"] =
        serde_json::to_value(study::model::verified_inputs(&fixture.manifest).unwrap()).unwrap();
    evidence["results_sha256"] = serde_json::to_value(
        crate::study_version::comparison_results_sha256(&fixture.local).unwrap(),
    )
    .unwrap();
    // 结果字节变了，成员证据摘要也必须跟着服务器结果树重算，否则会在“未绑定结果文件”处拒绝。
    let results: std::collections::BTreeMap<String, String> =
        serde_json::from_value(evidence["results_sha256"].clone()).unwrap();
    evidence["members"][0]["evidence_sha256"] = serde_json::json!(
        crate::study_version::member_evidence_digest(&results, &fixture.selected).unwrap()
    );
}

#[test]
fn fake_server_target_source_contents_must_match_the_frozen_identity_for_writeback() {
    let mut fixture = tuning_fetched_study();
    add_tuning_target_source(&mut fixture, b"frozen target observation");
    // 内容一致：写回路径仍通过，且目标源身份确实参与了比较。
    verified_study_member_with(&fixture.local, &fixture.selected, Some(&fixture.fake)).unwrap();

    let mut evidence = fixture.fake.comparison_evidence.clone().unwrap();
    evidence["input_identity"]["target_sources"]["Gpp"]["sha256"] = json!("0".repeat(64));
    let fake = FakeStudyRemote {
        comparison_evidence: Some(evidence),
        ..fixture.fake
    };
    let error =
        verified_study_member_with(&fixture.local, &fixture.selected, Some(&fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("Gpp"), "{text}");
    assert!(text.contains("differs"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_non_best_member_is_verified_against_server_state_results_and_samples() {
    let fixture = tuning_fetched_study();
    let verified =
        verified_study_member_with(&fixture.local, &fixture.selected, Some(&fixture.fake)).unwrap();
    assert_eq!(verified.member.id, fixture.selected);
    // 非 best/baseline 成员必须让服务器按成员核验，而不是只查 state 分数。
    assert_eq!(
        fixture.fake.comparison_members.borrow().as_slice(),
        &[Some(fixture.selected.clone())]
    );
    // 服务器状态里的分数被核验并随证据带出（不是只查任务成功）。
    assert!(verified.objective.is_some(), "{:?}", verified.objective);
    assert_eq!(
        verified.evidence["verification"]["metrics_recomputed"],
        false
    );
    assert_eq!(
        verified.evidence["verification"]["member_scope"],
        "server-selected-member-evidence"
    );
    assert!(
        verified.evidence["verification"]["note"]
            .as_str()
            .unwrap()
            .contains("same member-evidence check"),
        "{}",
        verified.evidence["verification"]
    );
    // 结果树身份与样本身份都由假服务器证据/样本核对过。
    assert!(
        verified.evidence["results_tree_sha256"]
            .as_str()
            .unwrap()
            .len()
            == 64
    );
    assert!(fixture
        .fake
        .reads
        .borrow()
        .iter()
        .any(|path| path.ends_with("/samples/g000000.csv")));
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn the_best_member_is_still_marked_as_having_its_metric_recomputed() {
    let fixture = tuning_fetched_study();
    let verified =
        verified_study_member_with(&fixture.local, &fixture.best, Some(&fixture.fake)).unwrap();
    assert_eq!(verified.member.id, fixture.best);
    // best 不走按成员核验的新参数：旧服务器命令与旧证据形状保持不变。
    assert_eq!(fixture.fake.comparison_members.borrow().as_slice(), &[None]);
    assert_eq!(
        verified.evidence["verification"]["metrics_recomputed"],
        true
    );
    assert_eq!(
        verified.evidence["verification"]["member_scope"],
        "best-or-baseline-metrics"
    );
    assert!(verified.evidence["verification"]["note"].is_null());
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn an_old_server_evidence_without_member_entries_still_serves_best_and_baseline() {
    let fixture = tuning_fetched_study();
    // 旧引擎的 schema 1 证据没有 `members` 字段：best/baseline 写回必须照常可读。
    let mut evidence = fixture.fake.comparison_evidence.clone().unwrap();
    evidence.as_object_mut().unwrap().remove("members");
    let fake = FakeStudyRemote {
        comparison_evidence: Some(evidence),
        ..fixture.fake
    };
    verified_study_member_with(&fixture.local, &fixture.best, Some(&fake)).unwrap();
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn selected_member_writeback_refuses_a_server_without_member_evidence() {
    let fixture = tuning_fetched_study();
    // 旧服务器引擎会接受并忽略 `--member`，返回的仍是只有 best+baseline 的旧证据。
    let mut evidence = fixture.fake.comparison_evidence.clone().unwrap();
    evidence.as_object_mut().unwrap().remove("members");
    let fake = FakeStudyRemote {
        comparison_evidence: Some(evidence),
        ..fixture.fake
    };
    let error =
        verified_study_member_with(&fixture.local, &fixture.selected, Some(&fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(
        text.contains("predates per-member comparison verification"),
        "{text}"
    );
    assert!(
        text.contains("write back the best or baseline member"),
        "{text}"
    );
    // 客户端确实把 --member 送给了服务器；是服务器的旧证据里没有条目，所以拒绝。
    assert_eq!(
        fake.comparison_members.borrow().as_slice(),
        &[Some(fixture.selected.clone())]
    );
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_server_that_rejects_the_member_argument_is_reported_as_outdated() {
    let fixture = tuning_fetched_study();
    let fake = FakeStudyRemote {
        comparison_failure: Some(
            "colm-cli: unknown argument --member\nusage: colm-cli study-verify-comparison \
             <study-dir>"
                .into(),
        ),
        ..fixture.fake
    };
    let error =
        verified_study_member_with(&fixture.local, &fixture.selected, Some(&fake)).unwrap_err();
    let text = format!("{error:#}");
    // 明确拒绝并给可行动说明，而不是静默退回弱核验。
    assert!(
        text.contains("predates per-member comparison verification"),
        "{text}"
    );
    assert!(
        text.contains("Upgrade or rebuild the remote engine"),
        "{text}"
    );
    assert!(text.contains("unknown argument"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn selected_member_evidence_must_be_bound_to_the_verified_result_tree() {
    let fixture = tuning_fetched_study();
    let mut evidence = fixture.fake.comparison_evidence.clone().unwrap();
    evidence["members"][0]["evidence_sha256"] = json!("0".repeat(64));
    let fake = FakeStudyRemote {
        comparison_evidence: Some(evidence),
        ..fixture.fake
    };
    let error =
        verified_study_member_with(&fixture.local, &fixture.selected, Some(&fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(
        text.contains("not bound to its verified result files"),
        "{text}"
    );
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn a_tuning_member_without_a_server_objective_cannot_be_written_back() {
    let fixture = tuning_fetched_study();
    let mut evidence = fixture.fake.comparison_evidence.clone().unwrap();
    // 本机检查点与服务器证据要同时缺这个候选，否则会先在“状态已变”处拒绝，测不到分数缺失。
    let checkpoint_dir = fixture.local.join("checkpoints/state");
    let mut local_state =
        study::checkpoint::load_latest::<study::state::StudyState>(&checkpoint_dir)
            .unwrap()
            .unwrap()
            .payload;
    local_state.candidates.remove(&fixture.selected);
    study::checkpoint::write_next(&checkpoint_dir, &local_state).unwrap();
    let mut server_state: study::state::StudyState =
        serde_json::from_value(evidence["state"].clone()).unwrap();
    server_state.candidates.remove(&fixture.selected);
    evidence["state"] = serde_json::to_value(&server_state).unwrap();
    // 状态哈希按类型化状态算，否则会在状态哈希核对处拒绝。
    evidence["state_sha256"] = json!(colm_kernel::sha256_hex(
        &serde_json::to_vec(&server_state).unwrap()
    ));
    let fake = FakeStudyRemote {
        comparison_evidence: Some(evidence),
        ..fixture.fake
    };
    let error =
        verified_study_member_with(&fixture.local, &fixture.selected, Some(&fake)).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("records no objective"), "{text}");
    assert!(text.contains(&fixture.selected), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

#[test]
fn local_comparison_evidence_rejects_unknown_and_already_verified_members() {
    let fixture = tuning_fetched_study();
    let unknown = crate::study_version::local_comparison_evidence(&fixture.local, Some("m999999"))
        .unwrap_err();
    assert!(format!("{unknown:#}").contains("unknown Study member m999999"));
    let best = format!(
        "{:#}",
        crate::study_version::local_comparison_evidence(&fixture.local, Some(&fixture.best))
            .unwrap_err()
    );
    assert!(best.contains("already verified without --member"), "{best}");
    // 旧调用（不传 --member）不会落在成员参数检查上：这个 fixture 没有真实执行证据，预期报
    // 输入/证据错误，但不能是 --member 相关错误。
    let error = crate::study_version::local_comparison_evidence(&fixture.local, None).unwrap_err();
    let text = format!("{error:#}");
    assert!(!text.contains("--member"), "{text}");
    std::fs::remove_dir_all(fixture.root).unwrap();
}

// ---- P2-10：远程研究资源预算 = 每候选线程 × 并发 ----

fn resource_opts(flags: &[(&str, &str)]) -> crate::Opts {
    crate::Opts {
        flags: flags
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        positional: Vec::new(),
    }
}

#[test]
fn study_resources_default_to_one_candidate_threads_times_concurrency() {
    let opts = resource_opts(&[]);
    let resources = study_resources(&opts, 8, 4).unwrap();
    // 4 个成员同时跑、每个 8 线程：申请 32 核，而不是原来只按 max(8,4)=8 核（会被集群限核/OOM）。
    assert_eq!(resources.resources.cpus, 32);
    assert_eq!(resources.threads, 8);
    assert_eq!(resources.jobs, 4);
    let report = resources_report(&resources.resources, resources.threads, resources.jobs);
    assert_eq!(report["cpus"], 32);
    assert_eq!(report["cores_per_candidate"], 8);
    assert_eq!(report["concurrent_candidates"], 4);
}

#[test]
fn one_concurrent_candidate_requests_only_its_own_threads() {
    let opts = resource_opts(&[]);
    assert_eq!(study_resources(&opts, 8, 1).unwrap().resources.cpus, 8);
}

#[test]
fn explicit_cpus_win_when_they_cover_the_concurrency() {
    // 申请必须覆盖实际并行：4 个成员同时跑、每个 8 线程。显式给足 32 或更多时以用户为准。
    let enough = resource_opts(&[("--cpus", "32")]);
    let resources = study_resources(&enough, 8, 4).unwrap();
    assert_eq!(resources.resources.cpus, 32);
    // 多申请（整节点、共享分区）仍以用户为准，不拒绝。
    let roomy = resource_opts(&[("--cpus", "64")]);
    assert_eq!(study_resources(&roomy, 8, 4).unwrap().resources.cpus, 64);
}

#[test]
fn explicit_cpus_below_threads_times_jobs_is_rejected() {
    // 原实现只校验 cpus >= threads：16 核放行，但作业脚本里仍是 4 个成员 × 8 线程 = 32，
    // 超配的部分会被集群限核/OOM 打掉——与 P2-10 的 max(threads, jobs) 是同一类错误。
    let between = resource_opts(&[("--cpus", "16")]);
    let error = study_resources(&between, 8, 4).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("--cpus (16)"), "{text}");
    assert!(text.contains("--threads"), "{text}");
    assert!(text.contains("--jobs"), "{text}");
    assert!(
        text.contains("32"),
        "the message names the cores to request: {text}"
    );

    // 比一个成员的线程数还少，同样拒绝（旧断言，保留）。
    let too_few = resource_opts(&[("--cpus", "4")]);
    assert!(study_resources(&too_few, 8, 4).is_err());
}

#[test]
fn threads_and_jobs_must_be_positive() {
    let opts = resource_opts(&[]);
    assert!(study_resources(&opts, 0, 4).is_err(), "threads=0");
    assert!(study_resources(&opts, 8, 0).is_err(), "jobs=0");
}

#[test]
fn the_dry_run_report_names_the_resource_budget_and_threads() {
    let opts = resource_opts(&[]);
    let resources = study_resources(&opts, 8, 4).unwrap();
    let report = dry_run_report(
        "job script".into(),
        "/srv/colm/s/abc",
        "engine",
        &json!({"kind": "tuning"}),
        &resources,
    );
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["threads"], 8);
    assert_eq!(report["jobs"], 4);
    assert_eq!(report["resources"]["cpus"], 32);
    assert_eq!(report["resources"]["cores_per_candidate"], 8);
    assert_eq!(report["resources"]["concurrent_candidates"], 4);
}
