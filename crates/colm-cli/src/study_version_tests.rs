use super::*;
use colm_workspace::version::{Readiness, Touches};

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-study-version-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn readiness(root: &Path, kernel: PathBuf) -> Readiness {
    let bin = root.join("ws/bin");
    std::fs::create_dir_all(&bin).unwrap();
    for program in PROGRAMS {
        std::fs::write(bin.join(exe(program)), program).unwrap();
    }
    Readiness {
        name: "snow".into(),
        head: "0123456789abcdef".into(),
        base_commit: "fedcba98".into(),
        origin: "app".into(),
        note: "lower aged-snow albedo".into(),
        based_on: None,
        touches: Touches {
            engine: true,
            kernel: false,
        },
        ready: true,
        items: Vec::new(),
        cli: bin.join(exe("colm-cli")),
        kernel,
        preset: "default".into(),
    }
}

#[test]
fn a_study_pins_a_copy_of_the_versions_programs() {
    let root = temp("pin");
    let study = root.join("study");
    std::fs::create_dir_all(&study).unwrap();
    let official = root.join("kernels/default");
    let ready = readiness(&root, official.clone());
    let pinned = pin(&study, &ready, &official).unwrap();
    assert_eq!(pinned.cli, study.join("version/bin").join(exe("colm-cli")));
    assert_eq!(
        pinned.kernel, official,
        "Fortran unchanged: the official kernel"
    );
    assert_eq!(pinned.note, "lower aged-snow albedo");
    for program in PROGRAMS {
        assert_eq!(
            std::fs::read_to_string(study.join("version/bin").join(exe(program))).unwrap(),
            program
        );
    }
    // 快照是拷贝：之后工作区重编不影响研究。
    std::fs::write(&ready.cli, "rebuilt").unwrap();
    assert_eq!(std::fs::read_to_string(&pinned.cli).unwrap(), "colm-cli");
    assert_eq!(super::pinned(&study).unwrap(), Some(pinned.clone()));

    // 已固定：同名或不给都用快照，换名字报错。
    let ws_root = root.join("workspaces");
    for requested in [None, Some("snow")] {
        assert_eq!(
            ensure(&study, requested, &ws_root, "rust", "rust", &official).unwrap(),
            Some(pinned.clone())
        );
    }
    let other = ensure(&study, Some("other"), &ws_root, "rust", "rust", &official).unwrap_err();
    assert!(format!("{other:#}").contains("started with model version snow"));
    let official_again = ensure(
        &study,
        Some("official"),
        &ws_root,
        "rust",
        "rust",
        &official,
    )
    .unwrap_err();
    assert!(format!("{official_again:#}").contains("cannot switch to official"));
}

#[test]
fn a_fortran_change_pins_the_versions_own_kernel_and_official_studies_pin_nothing() {
    let root = temp("kernel");
    let study = root.join("study");
    std::fs::create_dir_all(&study).unwrap();
    let official = root.join("kernels/default");
    let own = root.join("ws/kernels/default");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("manifest.json"), "{}").unwrap();
    let pinned = pin(&study, &readiness(&root, own), &official).unwrap();
    assert_eq!(pinned.kernel, study.join("version/kernel"));
    assert!(study.join("version/kernel/manifest.json").is_file());

    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    for requested in [None, Some("official"), Some("")] {
        assert_eq!(
            ensure(&plain, requested, &root, "rust", "rust", &official).unwrap(),
            None
        );
    }
    assert!(!plain.join(FILE).exists());
}

fn side(version: &str, calibration: Option<f64>, validation: Option<f64>) -> Side {
    Side {
        study: format!("/studies/{version}"),
        version: version.into(),
        head: None,
        status: "completed".into(),
        best_member: calibration.map(|_| "m1".into()),
        calibration,
        validation,
        baseline_calibration: Some(1.0),
        baseline_validation: Some(1.0),
    }
}

#[test]
fn two_versions_are_compared_on_the_validation_period_when_both_have_it() {
    let v = verdict(
        side("official", Some(0.3), Some(0.5)),
        side("snow", Some(0.4), Some(0.45)),
        true,
    );
    assert_eq!(v["period"], "validation");
    assert_eq!(
        v["better"], "b",
        "lower validation objective wins even with a worse fit"
    );
    assert!(v["notes"].as_array().unwrap().is_empty());

    let v = verdict(
        side("official", Some(0.3), None),
        side("snow", Some(0.4), Some(0.2)),
        true,
    );
    assert_eq!(v["period"], "calibration");
    assert_eq!(v["better"], "a");
    assert!(v["notes"][0]
        .as_str()
        .unwrap()
        .contains("no validation score"));

    let v = verdict(
        side("snow", Some(0.3), Some(0.3)),
        side("snow", Some(0.3), Some(0.3)),
        false,
    );
    assert!(
        v["better"].is_null(),
        "different study designs cannot produce a winner or tie"
    );
    let notes: Vec<&str> = v["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert!(notes.iter().any(|n| n.contains("not a fair comparison")));
    assert!(notes.iter().any(|n| n.contains("same model version")));

    let v = verdict(
        side("official", None, None),
        side("snow", Some(0.4), Some(0.4)),
        true,
    );
    assert!(v["better"].is_null());
}

/// 交给快照里的 colm-cli：同一条 study-run，换上固定的内核，并标明由外层转交（里层不再转交）。
#[cfg(unix)]
#[test]
fn a_pinned_study_runs_with_the_snapshot_cli() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("delegate");
    let log = root.join("args.txt");
    let cli = root.join("colm-cli");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\nif [ \"$1\" = fingerprint-policy ]; then printf '%s\n' '{{\"forcing_content_policy\":\"excluded-v1\"}}'; exit 0; fi\necho \"$@ by-parent=$COLM_STUDY_BY_PARENT\" > '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pinned = Pinned {
        model_revision: None,
        model_root: None,
        name: "snow".into(),
        head: "abc".into(),
        base_commit: "def".into(),
        note: String::new(),
        based_on: None,
        touches: Touches {
            engine: true,
            kernel: false,
        },
        cli,
        kernel: PathBuf::from("/k/default"),
        pinned_unix: 0,
    };
    let args: Vec<String> = ["/study", "--stream", "1"].map(String::from).to_vec();
    assert_eq!(delegate(&pinned, &args).unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "study-run /study --stream 1 --kernel /k/default by-parent=1"
    );
    std::fs::remove_file(&log).unwrap();
    std::fs::write(&pinned.cli, format!("#!/bin/sh\nif [ \"$1\" = fingerprint-policy ]; then echo '{{}}'; exit 0; fi\necho unsafe-study-run > '{}'\n", log.display())).unwrap();
    assert!(delegate(&pinned, &args)
        .unwrap_err()
        .to_string()
        .contains("update the model code"));
    assert!(
        !log.exists(),
        "an unsupported CLI must never receive study-run"
    );
}

#[test]
fn resuming_a_pin_repairs_its_ledger_without_changing_the_captured_commit() {
    let root = temp("ledger-resume");
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    colm_workspace::git::run(&source, &["init", "-q"]).unwrap();
    colm_workspace::git::set_identity(&source).unwrap();
    std::fs::write(source.join("README"), "initial").unwrap();
    colm_workspace::git::run(&source, &["add", "."]).unwrap();
    colm_workspace::git::run(&source, &["commit", "-qm", "initial"]).unwrap();
    let ws_root = root.join("workspaces");
    let ws = colm_workspace::Workspace::create(&ws_root, "snow", source.to_str().unwrap(), None)
        .unwrap();
    let study = root.join("study");
    std::fs::create_dir(&study).unwrap();
    let official = root.join("kernel");
    let mut ready = readiness(&root, official.clone());
    ready.head = ws.head().unwrap();
    let saved = pin(&study, &ready, &official).unwrap();
    for _ in 0..2 {
        ensure(&study, Some("snow"), &ws_root, "rust", "rust", &official).unwrap();
    }
    let rows = colm_workspace::studies::list(&ws).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].head, saved.head);
    assert!(rows[0].current);
    std::fs::write(ws.src().join("README"), "later").unwrap();
    colm_workspace::git::run(&ws.src(), &["commit", "-qam", "later"]).unwrap();
    std::fs::remove_file(ws.dir.join("studies.jsonl")).unwrap();
    assert!(ensure(&study, Some("other"), &ws_root, "rust", "rust", &official).is_err());
    assert!(!ws.dir.join("studies.jsonl").exists());
    ensure(&study, None, &ws_root, "rust", "rust", &official).unwrap();
    let rows = colm_workspace::studies::list(&ws).unwrap();
    assert_eq!(rows[0].head, saved.head);
    assert!(!rows[0].current);
}

fn captured_remote_version(study: &Path, version: serde_json::Value) {
    let record = serde_json::json!({
        "host":"server","root":"/srv/colm","job":"study-1",
        "remote_project":"/srv/colm/s/one","engine":"0123456789abcdef",
        "submitted_at":1,"scheduler":"direct","provenance":{"version":version}
    });
    std::fs::write(
        study.join(crate::remote_cmd::remote_study::RECORD),
        record.to_string(),
    )
    .unwrap();
}

#[test]
fn remote_comparison_uses_the_captured_workspace_identity_and_same_version_note() {
    let root = temp("remote-version");
    captured_remote_version(
        &root,
        serde_json::json!({"kind":"workspace","name":"snow","label":"snow","head":"0123456789abcdef"}),
    );
    let (name, head) = comparison_version(&root).unwrap();
    assert_eq!(name, "snow");
    assert_eq!(head.as_deref(), Some("0123456789abcdef"));
    let mut remote = side(&name, Some(0.4), Some(0.5));
    remote.head = head.clone();
    let mut local = side("snow", Some(0.4), Some(0.5));
    local.head = head;
    let result = verdict(local, remote, true);
    assert_eq!(result["a"]["version"], result["b"]["version"]);
    assert_eq!(result["a"]["head"], result["b"]["head"]);
    assert!(result["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().unwrap().contains("same model version")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_remote_version_is_not_silently_official_and_local_pin_has_priority() {
    let root = temp("remote-invalid");
    assert_eq!(
        comparison_version(&root).unwrap(),
        ("official".into(), None)
    );
    captured_remote_version(
        &root,
        serde_json::json!({"kind":"official","label":"官方版"}),
    );
    assert_eq!(
        comparison_version(&root).unwrap(),
        ("official".into(), None)
    );
    for version in [
        serde_json::json!({}),
        serde_json::json!({"kind":"workspace","name":"snow"}),
        serde_json::json!({"kind":"unknown"}),
    ] {
        captured_remote_version(&root, version);
        assert!(comparison_version(&root).is_err());
    }
    std::fs::write(
        root.join(crate::remote_cmd::remote_study::RECORD),
        "broken json",
    )
    .unwrap();
    assert!(comparison_version(&root).is_err());
    let kernel = root.join("kernel");
    let ready = readiness(&root, kernel.clone());
    pin(&root, &ready, &kernel).unwrap();
    assert_eq!(
        comparison_version(&root).unwrap(),
        (ready.name, Some(ready.head))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn captured_versions_from_different_password_servers_compare_without_credentials() {
    let root = temp("remote-password-metadata");
    let original_auth = colm_remote::auth::current_connection();
    for (name, host) in [("snow", "server-a"), ("soil", "server-b")] {
        let study = root.join(name);
        std::fs::create_dir(&study).unwrap();
        captured_remote_version(
            &study,
            serde_json::json!({"kind":"workspace","name":name,"head":"0123456789abcdef"}),
        );
        let path = study.join(crate::remote_cmd::remote_study::RECORD);
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record["ssh_auth"] = serde_json::json!({"host":host,"username":"researcher","port":22,"auth":"password","identity_file":""});
        std::fs::write(&path, record.to_string()).unwrap();
        assert_eq!(
            comparison_version(&study).unwrap(),
            (name.into(), Some("0123456789abcdef".into()))
        );
    }
    assert_eq!(colm_remote::auth::current_connection(), original_auth);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn identical_specs_without_full_input_and_output_evidence_report_no_winner() {
    let root = temp("compare-legacy-evidence");
    let spec: study::spec::StudySpec = serde_json::from_value(serde_json::json!({
        "kind":"uncertainty","method":"oat","base_cases":["caseA"],
        "parameters":[{"name":"DEF_TUNING_CNFAC","sample_min":0.2,"sample_max":0.8}],"outputs":["f_tref"],
    })).unwrap();
    for name in ["a", "b"] {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("samples")).unwrap();
        std::fs::write(dir.join("samples/design.csv"), "member,DEF_TUNING_CNFAC\n").unwrap();
        let manifest = study::spec::Manifest {
            schema_version: 1,
            id: name.into(),
            root: dir.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: spec.clone(),
            members: Vec::new(),
            provenance: Default::default(),
        };
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut state = study::state::StudyState::new(name.into(), []).unwrap();
        state.status = study::state::StudyStatus::Completed;
        state.best_member = Some("m000001".into());
        state.best_objective = Some(if name == "a" { 0.1 } else { 0.2 });
        study::checkpoint::write_next(&dir.join("checkpoints/state"), &state).unwrap();
    }
    let result = compare(&root.join("a"), &root.join("b")).unwrap();
    assert_eq!(result["same_design"], true);
    assert!(result["better"].is_null());
    assert_eq!(result["inputs_verified"], false);
    assert_eq!(result["forcing_contents_verified"], false);
    assert!(result["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("could not be verified")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_design_compares_observation_contents_separately_from_their_host_paths() {
    let left: study::spec::StudySpec = serde_json::from_value(serde_json::json!({
        "kind":"uncertainty","method":"oat","base_cases":["caseA"],
        "parameters":[],"observations":{"caseA":"/local/obs.nc"},"outputs":["f_tref"],
    }))
    .unwrap();
    let mut right = left.clone();
    right
        .observations
        .insert("caseA".into(), "/server/obs.nc".into());
    assert!(same_design(&left, &right).unwrap());
    right.observations.clear();
    assert!(!same_design(&left, &right).unwrap());
}

/// 早于新设定的修订程序：设定里出现它不认得的字段就像旧版 serde 那样报错。
#[cfg(unix)]
fn old_cli(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let cli = root.join("old-colm-cli");
    std::fs::write(
        &cli,
        "#!/bin/sh\nspec=\"$4\"\nif grep -q '\"include_baseline\"' \"$spec\"; then echo 'Error: unknown field `include_baseline`, expected one of `jobs`' >&2; exit 1; fi\nif grep -q '\"morris\"' \"$spec\"; then echo 'Error: cannot parse\\n\\nCaused by:\\n    unknown variant `morris`, expected one of `oat`' >&2; exit 1; fi\nif grep -q '\"bad\"' \"$spec\"; then echo 'Error: target Qle has 1 usable observation' >&2; exit 1; fi\necho ok\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    cli
}

#[cfg(unix)]
#[test]
fn an_older_revision_drops_the_baseline_seed_but_refuses_designs_it_cannot_read() {
    let root = temp("old-cli");
    let cli = old_cli(&root);
    let revision: colm_model::FrozenRevision = serde_json::from_value(serde_json::json!({
        "schema": colm_model::SCHEMA,
        "version": {"id": "draft-a", "revision": 3},
        "name": "draft-a",
        "note": "",
        "parent": null,
        "code": {"kind": "official", "workspace": null, "commit": null, "source_sha256": "x"},
        "overlay": {},
        "overlay_sha256": "",
        "fingerprint": "",
        "created_unix": 0,
        "applicability": {}
    }))
    .unwrap();
    let write = |method: &str, include_baseline: bool, key: &str| {
        let spec = root.join("spec.json");
        let mut value = serde_json::json!({
            "kind": if method == "differential-evolution" { "tuning" } else { "uncertainty" },
            "method": method,
            "base_cases": ["site"],
            "observations": {"site": "obs.nc"},
            "parameters": [],
            "targets": [{"key": key, "variable": "Qle", "from": 0, "to": 10}],
            "budget": {"population": 4, "generations": 1},
        });
        if include_baseline {
            value["budget"]["include_baseline"] = true.into();
        }
        std::fs::write(&spec, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        spec
    };
    // 只是不认得 include_baseline：去掉它，研究照常建。
    let spec = write("differential-evolution", true, "Qle");
    ensure_revision_reads_spec(&cli, &root, &spec, &revision).unwrap();
    assert!(
        !crate::study::spec::read_spec(&spec)
            .unwrap()
            .budget
            .include_baseline
    );
    // 不认得 Morris：拒绝，并说清是哪个版本、读不了什么。
    let spec = write("morris", false, "Qle");
    let error = ensure_revision_reads_spec(&cli, &root, &spec, &revision)
        .unwrap_err()
        .to_string();
    assert!(error.contains("draft-a r3"), "{error}");
    assert!(error.contains("unknown variant `morris`"), "{error}");
    // 普通的预检错误不在这里判（交给当前程序的预检）。
    let spec = write("differential-evolution", false, "bad");
    ensure_revision_reads_spec(&cli, &root, &spec, &revision).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_first_run_freezes_engine_and_preprocessors_for_later_runs() {
    let root = temp("run-options");
    // 没记过：按请求记下（不给就是 Rust）。
    let first = frozen_run_options(&root, Some("fortran"), None).unwrap();
    assert_eq!(
        first,
        RunOptions {
            engine: "fortran".into(),
            preprocessors: "rust".into()
        }
    );
    // 续跑不给参数：沿用记下的，而不是退回缺省的 Rust。
    assert_eq!(frozen_run_options(&root, None, None).unwrap(), first);
    assert_eq!(
        frozen_run_options(&root, Some("fortran"), Some("rust")).unwrap(),
        first
    );
    // 明确要另一种：拒绝，不让一个研究混两种引擎的结果。
    let error = frozen_run_options(&root, Some("rust"), None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("runs with the fortran engine"), "{error}");
    assert_eq!(run_options(&root).unwrap(), Some(first));
    let _ = std::fs::remove_dir_all(&root);
}
