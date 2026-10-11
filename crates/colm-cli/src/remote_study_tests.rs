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
