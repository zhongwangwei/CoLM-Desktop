use super::*;

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
            "m2/siteA": { "member": "m2", "site": "siteA", "case_dir": "/elsewhere/m2", "status": "failed" }
        }
    });
    let local = Path::new("/Users/me/p/.colm/studies/s-local");
    let moved = transplant_state(state, "s-local", "/srv/p/.colm/studies/s-remote", local);
    assert_eq!(moved["study_id"], "s-local");
    assert_eq!(
        moved["tasks"]["m1/siteA"]["case_dir"],
        "/Users/me/p/.colm/studies/s-local/members/m1/siteA"
    );
    assert!(moved["tasks"]["m1/siteA"].get("process").is_none());
    assert_eq!(moved["tasks"]["m2/siteA"]["case_dir"], "/elsewhere/m2");
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
