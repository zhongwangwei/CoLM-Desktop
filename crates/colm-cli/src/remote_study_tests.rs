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
    )
    .unwrap();
    assert!(command.contains("'/data/colm/engine/abc123/bin/colm-cli' study-verify-comparison '/data/colm/s/abc/.colm/studies/s1'"));
    assert!(!command.contains("study-run"));
    assert!(comparison_command(
        &record,
        "/other/studies/s1",
        std::slice::from_ref(&record.remote_project)
    )
    .is_err());
    assert!(comparison_command(
        &record,
        "/data/colm/s/abc/.colm/studies/../../outside",
        std::slice::from_ref(&record.remote_project)
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
    std::fs::write(staged.join("results/metrics.json"), b"new result fixture").unwrap();
    install_fetched_study(study, &staged).unwrap();
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

    fn comparison_evidence(&self, _record: &StudyRecord, _study_root: &str) -> Result<Value> {
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
fn explicit_cpus_win_but_must_fit_one_candidate() {
    let sixteen = resource_opts(&[("--cpus", "16")]);
    let resources = study_resources(&sixteen, 8, 4).unwrap();
    assert_eq!(resources.resources.cpus, 16);
    let too_few = resource_opts(&[("--cpus", "4")]);
    let error = study_resources(&too_few, 8, 4).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("--cpus"), "{text}");
    assert!(text.contains("--threads"), "{text}");
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
