#![cfg_attr(windows, allow(dead_code))]
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

#[test]
fn same_design_ignores_parent_study_lineage_metadata() {
    let mut local: study::spec::StudySpec = serde_json::from_value(serde_json::json!({
        "kind":"uncertainty","method":"oat","base_cases":["caseA"],
        "parameters":[],"observations":{"caseA":"/local/obs.nc"},"outputs":["f_tref"],
        "parent_study":{
            "study":"/p/.colm/studies/s-uncertainty",
            "spec_sha256":"a".repeat(64)
        }
    }))
    .unwrap();
    let mut remote = local.clone();
    // 远程 staging 故意把 parent_study 清成 None（服务器引擎可能不认识）；谱系是审计元数据，
    // 不参与执行设计相等判断，否则派生研究取回时会被误拒。
    remote.parent_study = None;
    assert!(
        same_design(&local, &remote).unwrap(),
        "parent_study Some vs None 应是同一设计"
    );
    local.parent_study = None;
    let mut remote = local.clone();
    remote.parent_study = Some(study::spec::ParentStudy {
        study: "/p/.colm/studies/s-uncertainty".into(),
        spec_sha256: "a".repeat(64),
        model_revision: None,
    });
    assert!(
        same_design(&local, &remote).unwrap(),
        "parent_study None vs Some 也应是同一设计"
    );
    remote.seed = 1;
    assert!(
        !same_design(&local, &remote).unwrap(),
        "执行语义（seed）不同仍必须判为不同设计"
    );
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
    // 普通的预检错误不在这里判（交给当前程序的预检）：当前程序也拒绝。
    let spec = write("differential-evolution", false, "bad");
    ensure_revision_reads_spec_with(&cli, &root, &spec, &revision, || false).unwrap();
    // 旧程序按旧规则拒绝、当前程序却接受：是旧代码做不了，拒绝建研究并带上旧程序的原因（复验 B2-04）。
    let error = ensure_revision_reads_spec_with(&cli, &root, &spec, &revision, || true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("draft-a r3"), "{error}");
    assert!(
        error.contains("target Qle has 1 usable observation"),
        "{error}"
    );
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

// ---------------------------------------------------------------- 创建/预检交给运行研究的程序
//
// P1-8：研究由当前程序创建、由冻结修订自己的 colm-cli 运行时，manifest 里的参数来源哈希是当前程序
// 算的，修订程序用自己内嵌的参数目录重算就对不上——研究建得成却跑不起来。所以 workspace 型修订的
// `study-create`/`study-preflight` 整条命令交给 `revision_cli` 指的那份程序（见
// [`delegate_creation`]），由它写 manifest 与 provenance；官方修订和当前程序自己就是那份程序时保持
// 本地路径。

fn revision_code(workspace: Option<&str>) -> colm_model::CodeRef {
    colm_model::CodeRef {
        kind: if workspace.is_some() {
            colm_model::CodeKind::Workspace
        } else {
            colm_model::CodeKind::Official
        },
        workspace: workspace.map(str::to_owned),
        commit: Some("0".repeat(40)),
        source_sha256: "a".repeat(64),
    }
}

fn workspace_revision(id: &str, revision: u64) -> colm_model::FrozenRevision {
    serde_json::from_value(serde_json::json!({
        "schema": colm_model::SCHEMA,
        "version": {"id": id, "revision": revision},
        "name": id,
        "note": "",
        "parent": null,
        "code": {"kind": "workspace", "workspace": "ws", "commit": "0".repeat(40), "source_sha256": "a".repeat(64)},
        "overlay": {},
        "overlay_sha256": "",
        "fingerprint": "",
        "created_unix": 0,
        "applicability": {}
    }))
    .unwrap()
}

#[test]
fn creation_executor_follows_the_revisions_own_program() {
    let current = Path::new("/app/bin/colm-cli");
    let official_cli = Path::new("/models/sources/abc/bin/colm-cli");
    let workspace_cli = Path::new("/models/sources/def/workspaces/ws/bin/colm-cli");
    // 运行侧官方与 workspace 修订都用 `revision_cli` 指的那份程序（study-run 的 `delegate`）；
    // 创建/预检必须由同一份程序写 manifest 与 provenance，否则运行时 `verify_frozen_inputs`
    // 用另一份程序内嵌的参数目录重算，哈希对不上（P1-8）。官方修订也可能是另一种构建，
    // 不能再按“官方就不换程序”特判。
    for cli in [official_cli, workspace_cli] {
        assert_eq!(
            creation_executor(current, cli),
            CreationExecutor::Delegate(cli.to_path_buf())
        );
    }
    // 当前程序就是那份修订的 CLI（例如由它自己转交回来，或正跑在它的 bin 里）：按本地处理。
    assert_eq!(creation_executor(current, current), CreationExecutor::Local);
    // 缓存还没准备好：决策仍是转交，由 delegate_creation 先按运行侧同一路径补齐，补不齐就拒绝。
    let missing = Path::new("/models/sources/abc/bin/missing-colm-cli");
    assert_eq!(
        creation_executor(current, missing),
        CreationExecutor::Delegate(missing.to_path_buf())
    );
}

#[test]
fn creation_delegate_args_forward_the_original_case_and_spec() {
    let flags: Vec<(String, String)> = vec![
        ("--model-root".into(), "/models".into()),
        ("--model-id".into(), "ws".into()),
        ("--model-revision".into(), "2".into()),
        ("--history-db".into(), "/history.db".into()),
    ];
    assert_eq!(
        creation_delegate_args(
            "study-create",
            Path::new("/case"),
            Path::new("/tmp/spec.json"),
            &flags
        ),
        vec![
            "study-create",
            "/case",
            "--model-root",
            "/models",
            "--model-id",
            "ws",
            "--model-revision",
            "2",
            "--history-db",
            "/history.db",
            "--spec",
            "/tmp/spec.json",
        ]
    );
    // 设定是位置参数（老命令行用法）时补成 --spec；已经是 --spec 时不要写两遍。
    assert_eq!(
        creation_delegate_args(
            "study-preflight",
            Path::new("/case"),
            Path::new("/design.json"),
            &[]
        ),
        vec!["study-preflight", "/case", "--spec", "/design.json"]
    );
    let flags: Vec<(String, String)> = vec![("--spec".into(), "/design.json".into())];
    assert_eq!(
        creation_delegate_args(
            "study-create",
            Path::new("/case"),
            Path::new("/design.json"),
            &flags
        ),
        vec!["study-create", "/case", "--spec", "/design.json"]
    );
}

#[test]
fn a_study_without_a_frozen_revision_stays_local() {
    let opts = crate::Opts {
        flags: Vec::new(),
        positional: vec!["/case".into()],
    };
    assert!(delegate_creation(
        "study-create",
        &opts,
        Path::new("/case"),
        Path::new("/spec.json")
    )
    .unwrap()
    .is_none());
}

/// 官方修订的研究将来由 `cached_official_cli` 那份程序运行（`study-run` 的 `delegate`），
/// 创建/预检也必须交给同一条路径上的程序；否则当前程序写下的 provenance 与运行侧算出的对不上。
#[cfg(unix)]
#[test]
fn an_official_revision_study_is_created_by_its_cached_cli() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("delegate-official-cached");
    let models = root.join("models");
    let store = colm_model::Registry::open(&models).unwrap();
    let official = store.install_official(revision_code(None)).unwrap();
    let case_root = root.join("case");
    std::fs::create_dir_all(&case_root).unwrap();
    let spec = valid_spec(&root, false);
    let opts = crate::Opts {
        flags: vec![
            ("--model-root".into(), models.to_string_lossy().into_owned()),
            ("--model-id".into(), "official".into()),
            (
                "--model-revision".into(),
                official.version.revision.to_string(),
            ),
        ],
        positional: vec![case_root.to_string_lossy().into_owned()],
    };
    let cli = crate::model_version::cached_official_cli(&opts, &official.code).unwrap();
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    let study = case_root.join(".colm/studies/official-created");
    let template = student_template(
        &root,
        Some(serde_json::json!({
            "model_revision": {"version": {"id": "official", "revision": official.version.revision}}
        })),
    );
    let log = root.join("args.log");
    let env_log = root.join("env.log");
    let script = format!(
        "printf '%s\\n' \"$@\" > '{log}'\n\
         printf '%s %s\\n' \"$COLM_STUDY_BY_PARENT\" \"$COLM_AUDIT_BY_PARENT\" > '{env_log}'\n\
         mkdir -p '{studies}'\n\
         cp -R '{template}' '{study}'\n\
         printf '%s\\n' '{study}'\n",
        log = log.display(),
        env_log = env_log.display(),
        studies = study.parent().unwrap().display(),
        template = template.display(),
        study = study.display(),
    );
    std::fs::write(&cli, format!("#!/bin/sh\n{script}")).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

    let stdout = delegate_creation("study-create", &opts, &case_root, &spec)
        .unwrap()
        .expect("官方修订也必须交给运行侧那份 colm-cli");
    assert_eq!(stdout, format!("{}\n", study.display()));
    let args: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    let revision = official.version.revision.to_string();
    assert_eq!(
        args[..8],
        [
            "study-create",
            case_root.to_str().unwrap(),
            "--model-root",
            models.to_str().unwrap(),
            "--model-id",
            "official",
            "--model-revision",
            revision.as_str(),
        ][..]
    );
    assert_eq!(args[8], "--spec");
    assert_ne!(
        Path::new(&args[9]),
        spec,
        "交出去的仍是剥掉新审计字段的副本"
    );
    assert!(!Path::new(&args[9]).exists(), "转交用的临时副本事后要删掉");
    assert_eq!(
        std::fs::read_to_string(&env_log).unwrap().trim(),
        "1 1",
        "子进程要标明由父进程转交，且不再单独记审计"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 缓存里的程序就是当前这个可执行文件时（符号链接指回来）：再转交只会绕圈，保持本地路径。
#[cfg(unix)]
#[test]
fn an_official_revision_that_is_this_program_stays_local() {
    let root = temp("delegate-official-current");
    let models = root.join("models");
    let store = colm_model::Registry::open(&models).unwrap();
    let official = store.install_official(revision_code(None)).unwrap();
    let case_root = root.join("case");
    std::fs::create_dir_all(&case_root).unwrap();
    let spec = valid_spec(&root, false);
    let opts = crate::Opts {
        flags: vec![
            ("--model-root".into(), models.to_string_lossy().into_owned()),
            ("--model-id".into(), "official".into()),
            (
                "--model-revision".into(),
                official.version.revision.to_string(),
            ),
        ],
        positional: vec![case_root.to_string_lossy().into_owned()],
    };
    let cli = crate::model_version::cached_official_cli(&opts, &official.code).unwrap();
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &cli).unwrap();
    assert!(delegate_creation("study-create", &opts, &case_root, &spec)
        .unwrap()
        .is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 修订的 CLI 准备不出来（缓存残缺/来源对不上）时不能退回当前程序：明确拒绝并给可行动提示。
#[test]
fn a_revision_cli_that_cannot_be_prepared_is_refused_with_an_actionable_error() {
    let root = temp("delegate-unpreparable");
    let models = root.join("models");
    let store = colm_model::Registry::open(&models).unwrap();
    let official = store.install_official(revision_code(None)).unwrap();
    let opts = crate::Opts {
        flags: vec![
            ("--model-root".into(), models.to_string_lossy().into_owned()),
            ("--model-id".into(), "official".into()),
            (
                "--model-revision".into(),
                official.version.revision.to_string(),
            ),
        ],
        positional: vec![root.join("case").to_string_lossy().into_owned()],
    };
    // 缓存里放另一份来源的 code.json：`verify_code` 当场判身份不符，不必去算当前源码。
    let cache = crate::model_version::cache_dir(&opts, &official.code).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let mut wrong = revision_code(None);
    wrong.source_sha256 = "b".repeat(64);
    std::fs::write(cache.join("code.json"), serde_json::to_vec(&wrong).unwrap()).unwrap();
    let error = delegate_creation(
        "study-create",
        &opts,
        Path::new(&opts.positional[0]),
        Path::new("/nonexistent/spec.json"),
    )
    .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("official r1"), "{text}");
    assert!(text.contains("could not be prepared"), "{text}");
    assert!(text.contains("cached source identity mismatch"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
fn fake_cli(root: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let cli = root.join("revision-colm-cli");
    std::fs::write(&cli, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    cli
}

/// 能通过 `read_spec` 的最小设定；`include_baseline` 是 GUI 自动加的增强（见
/// `ensure_revision_reads_spec`）。
#[cfg(unix)]
fn valid_spec(root: &Path, include_baseline: bool) -> PathBuf {
    let path = root.join("spec.json");
    let value = serde_json::json!({
        "kind": "uncertainty",
        "method": "oat",
        "base_cases": ["site"],
        "observations": {"site": "obs.nc"},
        "parameters": [],
        "targets": [{"key": "Qle", "variable": "Qle", "from": 0, "to": 10}],
        "budget": {"population": 4, "generations": 1, "include_baseline": include_baseline},
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}

/// 假 CLI 建研究的方式：把测试准备好的模板目录复制成研究目录，并打印它的路径。
#[cfg(unix)]
fn student_template(root: &Path, pin: Option<serde_json::Value>) -> PathBuf {
    let template = root.join("template");
    std::fs::create_dir_all(&template).unwrap();
    std::fs::write(template.join("manifest.json"), b"{}").unwrap();
    if let Some(pin) = pin {
        std::fs::write(
            template.join("model-version.json"),
            serde_json::to_vec_pretty(&pin).unwrap(),
        )
        .unwrap();
    }
    template
}

#[cfg(unix)]
#[test]
fn a_workspace_revision_study_is_created_by_its_own_cli() {
    let root = temp("delegate-create");
    let case_root = root.join("case");
    std::fs::create_dir_all(&case_root).unwrap();
    let spec = valid_spec(&root, false);
    let revision = workspace_revision("ws", 2);
    let study = case_root.join(".colm/studies/created");
    let template = student_template(
        &root,
        Some(serde_json::json!({"model_revision":{"version":{"id":"ws","revision":2}}})),
    );
    let log = root.join("args.log");
    let env_log = root.join("env.log");
    let script = format!(
        "printf '%s\\n' \"$@\" > '{log}'\n\
         printf '%s %s\\n' \"$COLM_STUDY_BY_PARENT\" \"$COLM_AUDIT_BY_PARENT\" > '{env_log}'\n\
         mkdir -p '{studies}'\n\
         cp -R '{template}' '{study}'\n\
         printf '%s\\n' '{study}'\n",
        log = log.display(),
        env_log = env_log.display(),
        studies = study.parent().unwrap().display(),
        template = template.display(),
        study = study.display(),
    );
    let cli = fake_cli(&root, &script);
    let flags: Vec<(String, String)> = vec![
        ("--model-root".into(), "/models".into()),
        ("--model-id".into(), "ws".into()),
        ("--model-revision".into(), "2".into()),
        ("--history-db".into(), "/history.db".into()),
    ];
    let stdout =
        create_with_revision_cli("study-create", &revision, &cli, &case_root, &spec, &flags)
            .unwrap();
    // 研究目录原样交回给 GUI（父程序不解析、不改写）。
    assert_eq!(stdout, format!("{}\n", study.display()));
    // 转交的是原始算例目录；设定是一份剥掉新审计字段的临时副本（旧修订 deny_unknown_fields
    // 读不了 parent_study），原始设定文件不被改写。
    let args: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        args[..args.len() - 1],
        [
            "study-create",
            case_root.to_str().unwrap(),
            "--model-root",
            "/models",
            "--model-id",
            "ws",
            "--model-revision",
            "2",
            "--history-db",
            "/history.db",
            "--spec",
        ][..]
    );
    let handed = Path::new(args.last().unwrap());
    assert_ne!(handed, spec, "不能把原始设定直接交给旧修订");
    assert!(!handed.exists(), "转交用的临时副本事后要删掉");
    assert_eq!(
        std::fs::read_to_string(&env_log).unwrap().trim(),
        "1 1",
        "子进程要标明由父进程转交，避免它再转交一次"
    );
    assert!(study.join("model-version.json").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// GUI 的 study-create/study-preflight 一直把设定放在 flags 的 `--spec 原文件` 里；转交旧修订时
/// 若原样透传这个 flag，剥离 parent_study 的副本根本送不出去（5a）。
#[cfg(unix)]
#[test]
fn gui_flags_hand_the_sanitized_spec_to_an_older_revision() {
    let root = temp("delegate-gui-spec");
    let spec = valid_spec(&root, true);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spec).unwrap()).unwrap();
    value["parent_study"] = serde_json::json!({
        "study": "/p/.colm/studies/s-uncertainty",
        "spec_sha256": "a".repeat(64)
    });
    std::fs::write(&spec, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let before = std::fs::read(&spec).unwrap();
    let args_log = root.join("args.log");
    let handed_log = root.join("handed.json");
    let script = format!(
        "spec=''\nprev=''\n\
         for arg in \"$@\"; do\n\
           printf '%s\\n' \"$arg\" >> '{args}'\n\
           if [ \"$prev\" = \"--spec\" ]; then spec=\"$arg\"; fi\n\
           prev=\"$arg\"\n\
         done\n\
         cp \"$spec\" '{handed}'\n\
         echo ok\n",
        args = args_log.display(),
        handed = handed_log.display(),
    );
    let cli = fake_cli(&root, &script);
    let revision = workspace_revision("ws", 2);
    let flags = vec![("--spec".to_string(), spec.to_string_lossy().into_owned())];
    let stdout =
        create_with_revision_cli("study-preflight", &revision, &cli, &root, &spec, &flags).unwrap();
    assert_eq!(stdout.trim(), "ok");
    let args: Vec<String> = std::fs::read_to_string(&args_log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        args.iter().filter(|arg| *arg == "--spec").count(),
        1,
        "不能把 --spec 写两遍: {args:?}"
    );
    let handed = Path::new(&args[args.iter().position(|arg| arg == "--spec").unwrap() + 1]);
    assert_ne!(
        handed, spec,
        "GUI 传来的 --spec 必须换成剥掉 parent_study 的副本"
    );
    assert!(!handed.exists(), "转交用的临时副本事后要删掉");
    let handed_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&handed_log).unwrap()).unwrap();
    assert!(
        handed_value.get("parent_study").is_none(),
        "旧修订收到的设定不能带 parent_study: {handed_value}"
    );
    assert_eq!(
        std::fs::read(&spec).unwrap(),
        before,
        "用户给的设定绝不被改写"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn an_older_revision_drops_the_baseline_seed_but_never_rewrites_the_given_spec() {
    let root = temp("delegate-baseline");
    let spec = valid_spec(&root, true);
    let before = std::fs::read(&spec).unwrap();
    let log = root.join("specs.log");
    let script = format!(
        "spec=''\nprev=''\n\
         for arg in \"$@\"; do\n\
           if [ \"$prev\" = \"--spec\" ]; then spec=\"$arg\"; fi\n\
           prev=\"$arg\"\n\
         done\n\
         has=0\n\
         if grep -q '\"include_baseline\"' \"$spec\"; then has=1; fi\n\
         printf '%s %s\\n' \"$spec\" \"$has\" >> '{log}'\n\
         if [ \"$has\" = \"1\" ]; then\n\
           echo 'Error: unknown field `include_baseline`, expected one of `jobs`' >&2\n\
           exit 1\n\
         fi\n\
         echo ok\n",
        log = log.display(),
    );
    let cli = fake_cli(&root, &script);
    let revision = workspace_revision("ws", 2);
    let stdout = create_with_revision_cli(
        "study-preflight",
        &revision,
        &cli,
        &root,
        &spec,
        &Vec::new(),
    )
    .unwrap();
    assert_eq!(stdout.trim(), "ok");
    // 旧程序的 deny_unknown_fields 只认键存在，值是 true 还是 false 都拒绝；重试副本必须删掉
    // 这个键，而不是写成 false。用户给的设定文件绝不改写。
    assert_eq!(std::fs::read(&spec).unwrap(), before);
    let attempts: Vec<(PathBuf, bool)> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| {
            let (path, has) = line.rsplit_once(' ').unwrap();
            (PathBuf::from(path), has == "1")
        })
        .collect();
    assert_eq!(
        attempts.len(),
        2,
        "先按剥掉新字段的副本试一次，去掉增强后再试一次"
    );
    assert_ne!(attempts[0].0, spec, "第一次也不能直接交原始设定");
    assert!(attempts[0].1, "第一次仍保留 GUI 的 include_baseline 增强");
    assert_ne!(attempts[1].0, spec);
    assert_ne!(attempts[1].0, attempts[0].0, "重试使用新的副本");
    assert!(
        !attempts[1].1,
        "重试的设定里必须没有 include_baseline 键（写 false 旧程序也拒绝）"
    );
    assert!(!attempts[0].0.exists(), "第一次的临时副本事后要删掉");
    assert!(!attempts[1].0.exists(), "重试用的临时副本事后要删掉");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn an_older_revision_that_cannot_read_the_design_reports_an_actionable_error() {
    let root = temp("delegate-old");
    let spec = valid_spec(&root, false);
    let cli = fake_cli(
        &root,
        "echo 'Error: unknown field `morris`, expected one of `oat`' >&2\nexit 3\n",
    );
    let revision = workspace_revision("ws", 2);
    let error =
        create_with_revision_cli("study-create", &revision, &cli, &root, &spec, &Vec::new())
            .unwrap_err()
            .to_string();
    assert!(error.contains("model version ws r2"), "{error}");
    assert!(error.contains("older code"), "{error}");
    assert!(error.contains("unknown field `morris`"), "{error}");
    assert!(error.contains("official version"), "{error}");
    assert!(
        error.contains("rebuild this version on the current code"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn an_ordinary_failure_still_reports_the_revision_and_its_own_message() {
    let root = temp("delegate-ordinary");
    let spec = valid_spec(&root, false);
    let cli = fake_cli(
        &root,
        "echo 'Error: kernel_dir does not exist: /k' >&2\nexit 2\n",
    );
    let revision = workspace_revision("ws", 2);
    let error =
        create_with_revision_cli("study-create", &revision, &cli, &root, &spec, &Vec::new())
            .unwrap_err()
            .to_string();
    assert!(error.contains("model version ws r2"), "{error}");
    assert!(error.contains("could not create this Study"), "{error}");
    assert!(error.contains("kernel_dir does not exist"), "{error}");
    // 普通失败不能归咎于旧代码。
    assert!(!error.contains("older code"), "{error}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_revision_cli_that_ignores_the_revision_cannot_leave_a_broken_study() {
    let root = temp("delegate-unbound");
    let case_root = root.join("case");
    std::fs::create_dir_all(&case_root).unwrap();
    let spec = valid_spec(&root, false);
    let revision = workspace_revision("ws", 2);
    let study = case_root.join(".colm/studies/orphan");
    // 只建研究、不写 model-version.json：这是还不会在创建时绑定修订的旧程序的行为。
    let template = student_template(&root, None);
    let script = format!(
        "mkdir -p '{studies}'\ncp -R '{template}' '{study}'\nprintf '%s\\n' '{study}'\n",
        studies = study.parent().unwrap().display(),
        template = template.display(),
        study = study.display(),
    );
    let cli = fake_cli(&root, &script);
    let error = create_with_revision_cli(
        "study-create",
        &revision,
        &cli,
        &case_root,
        &spec,
        &Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("model version ws r2"), "{error}");
    assert!(error.contains("did not bind"), "{error}");
    assert!(error.contains("official version"), "{error}");
    assert!(
        !study.exists(),
        "绑定不了修订的研究只会建得成跑不起来，不能留给用户"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_spec_handed_to_an_older_program_drops_parent_study_and_optionally_include_baseline() {
    let root = temp("delegate-strip");
    let spec = root.join("spec.json");
    let text = serde_json::json!({
        "kind": "tuning",
        "method": "differential-evolution",
        "base_cases": ["case"],
        "parameters": [{"name": "DEF_TUNING_CNFAC", "sample_min": 0.4, "sample_max": 0.6}],
        "targets": [{"key": "Qle", "variable": "Qle", "from": 0, "to": 3600}],
        "budget": {"population": 4, "generations": 1, "include_baseline": true},
        "parent_study": {
            "study": "/p/.colm/studies/s-uncertainty",
            "spec_sha256": "a".repeat(64),
            "model_revision": {"id": "trace", "revision": 7}
        }
    });
    std::fs::write(&spec, serde_json::to_vec(&text).unwrap()).unwrap();
    let stripped = spec_for_older_program(&spec, false).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&stripped).unwrap()).unwrap();
    assert!(value.get("parent_study").is_none(), "{value}");
    // 第一次尝试仍保留 GUI 的 include_baseline 增强，只有旧程序明确拒绝时才去掉。
    assert_eq!(value["budget"]["include_baseline"], true);
    std::fs::remove_file(&stripped).unwrap();

    let retried = spec_for_older_program(&spec, true).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&retried).unwrap()).unwrap();
    // 旧程序的 deny_unknown_fields 只看键在不在：写 `false` 一样被拒，必须整键删除。
    assert!(
        value["budget"].get("include_baseline").is_none(),
        "重试副本仍带 include_baseline 键: {value}"
    );
    assert!(value.get("parent_study").is_none());
    // 用户给的设定文件本身绝不被改写。
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spec).unwrap()).unwrap();
    assert!(original.get("parent_study").is_some());
    std::fs::remove_file(&retried).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// `study-compare` 读版本身份时也要能穿过 GUI 归档：活动记录被改名后，取回时写下的
/// 持久来源还在，比较页必须仍显示提交时的修改版，而不是退回“官方版”。
#[test]
fn an_archived_remote_source_still_names_the_comparison_version() {
    let root = temp("archived-source-version");
    let study = root.join("proj/.colm/studies/s-archive");
    std::fs::create_dir_all(&study).unwrap();
    let local = study.canonicalize().unwrap();
    let write_source = |version: serde_json::Value| {
        let source = serde_json::json!({
            "schema": 1,
            "host": "server",
            "root": "/srv/colm",
            "job": "study-1",
            "remote_project": "/srv/colm/s/abc",
            "engine": "0123456789abcdef",
            "scheduler": "bare",
            "submitted_at": 1,
            "fetched_at": 1,
            "provenance": {"version": version, "model_revision": null},
            "local_study": local.to_string_lossy(),
            "study_id": "s-archive",
            "remote_study": "/srv/colm/s/abc/.colm/studies/s-remote",
            "evidence_sha256": "a".repeat(64),
        });
        std::fs::write(
            study.join(crate::remote_cmd::remote_study::SOURCE),
            serde_json::to_vec_pretty(&source).unwrap(),
        )
        .unwrap();
    };
    write_source(serde_json::json!({
        "kind": "workspace", "name": "snow", "head": "0123456789abcdef", "base_commit": "fedcba98"
    }));
    assert_eq!(
        comparison_version(&study).unwrap(),
        ("snow".into(), Some("0123456789abcdef".into()))
    );
    // 也覆盖官方版：只有 kind=official 才判官方，不能因为来源缺字段就默认。
    write_source(serde_json::json!({"kind": "official"}));
    assert_eq!(
        comparison_version(&study).unwrap(),
        ("official".into(), None)
    );
    // 没有来源也没有活动记录时保持旧语义（比较页按官方版处理），不因本改动改变。
    std::fs::remove_file(study.join(crate::remote_cmd::remote_study::SOURCE)).unwrap();
    assert_eq!(
        comparison_version(&study).unwrap(),
        ("official".into(), None)
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------- 身份敏感命令交给研究 pin 的程序
//
// 这些命令会调用 `verify_frozen_inputs`：它用程序内嵌的参数目录与来源文件重算 provenance 哈希。
// 研究 pin 到旧修订后，当前程序算出的值与 pin 时由旧程序算出的不同，会被误报成“输入变了”
// （#6b，P1-8 同类）。所以只要研究 pin 的 CLI 与当前程序不同，整条命令交给 pin 的 CLI 执行。

fn pinned_study(root: &Path, cli: PathBuf) -> PathBuf {
    let study = root.join("study");
    std::fs::create_dir_all(&study).unwrap();
    let pin = Pinned {
        name: "snow".into(),
        head: "abc".into(),
        base_commit: "def".into(),
        note: String::new(),
        based_on: None,
        touches: Touches {
            engine: false,
            kernel: false,
        },
        cli,
        kernel: PathBuf::from("/k/default"),
        pinned_unix: 0,
        model_revision: None,
        model_root: None,
    };
    std::fs::write(study.join(FILE), serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    study
}

fn pinned_command_opts(command: &str, study: &Path) -> crate::Opts {
    let study = study.to_string_lossy().into_owned();
    match command {
        "study-save-version" => crate::Opts {
            positional: vec![study],
            flags: vec![
                ("--member".into(), "best".into()),
                ("--name".into(), "v1".into()),
            ],
        },
        "remote-study-run" => crate::Opts {
            positional: vec![study],
            flags: vec![
                ("--host".into(), "server".into()),
                ("--dry-run".into(), "1".into()),
                // 同一个 flag 出现多次时要按原顺序全部转交，不能只送第一条。
                ("--map".into(), "/data/a=/srv/a".into()),
                ("--map".into(), "/data/b=/srv/b".into()),
            ],
        },
        "remote-study-fetch" => crate::Opts {
            positional: vec![study],
            flags: vec![("--host".into(), "server".into())],
        },
        "model-import-study" => crate::Opts {
            positional: Vec::new(),
            flags: vec![
                ("--study-dir".into(), study),
                ("--model-id".into(), "draft".into()),
                ("--expected-generation".into(), "1".into()),
            ],
        },
        // 只读证据命令：参数只有研究目录，但 GUI 仍会附上 history 库与 operation id；
        // 转交要原样带上，否则子 CLI 会写另一份审计库。
        "study-verified-side" | "study-verify-comparison" => crate::Opts {
            positional: vec![study],
            flags: vec![
                ("--history-db".into(), "/tmp/history.sqlite".into()),
                ("--operation-id".into(), "op-1".into()),
            ],
        },
        // 复制研究也在 pin 的 CLI 上做：它要读 pin、复制版本目录，并在冻结模型修订上重算输入身份。
        "study-clone" => crate::Opts {
            positional: vec![study],
            flags: vec![("--operation-id".into(), "op-1".into())],
        },
        other => panic!("test does not know {other}"),
    }
}

/// 同一个 flag 可以重复（`--map` 等）；转交要按原值、原顺序全部送过去，不能只送第一条。
fn pinned_command_args(command: &str, opts: &crate::Opts) -> Vec<String> {
    let mut args = vec![command.to_owned()];
    args.extend(opts.positional.iter().cloned());
    for (flag, value) in &opts.flags {
        args.extend([flag.clone(), value.clone()]);
    }
    args
}

fn delegated_program(command: &str, opts: &crate::Opts) -> PathBuf {
    match pinned_command_plan(command, opts).unwrap() {
        PinnedCommand::Delegate(program) => program,
        other => panic!("{command} 应该交给 pin 的 CLI，实际 {other:?}"),
    }
}

fn assert_local(command: &str, opts: &crate::Opts) {
    assert_eq!(
        pinned_command_plan(command, opts).unwrap(),
        PinnedCommand::Local,
        "{command} 应该由当前程序执行"
    );
}

fn refusal(command: &str, opts: &crate::Opts) -> String {
    match pinned_command_plan(command, opts).unwrap() {
        PinnedCommand::Refuse(reason) => reason,
        other => panic!("{command} 应该拒绝执行，实际 {other:?}"),
    }
}

fn hybrid_fit_opts(studies: &[&Path]) -> crate::Opts {
    let studies = studies
        .iter()
        .map(|study| study.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(",");
    crate::Opts {
        positional: Vec::new(),
        flags: vec![
            ("--studies".into(), studies),
            ("--network".into(), "/net.json".into()),
            ("--kernel".into(), "/k".into()),
            ("--out".into(), "/out.mlp.json".into()),
        ],
    }
}

fn comparison_opts(a: &Path, b: &Path) -> crate::Opts {
    crate::Opts {
        positional: vec![
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ],
        flags: vec![
            ("--history-db".into(), "/tmp/history.sqlite".into()),
            ("--operation-id".into(), "op-1".into()),
        ],
    }
}

struct DelegatedEnv(Option<std::ffi::OsString>);

impl DelegatedEnv {
    fn set() -> Self {
        let previous = std::env::var_os(DELEGATED_ENV);
        std::env::set_var(DELEGATED_ENV, "1");
        Self(previous)
    }
}

impl Drop for DelegatedEnv {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var(DELEGATED_ENV, value),
            None => std::env::remove_var(DELEGATED_ENV),
        }
    }
}

#[cfg(unix)]
#[test]
fn identity_sensitive_study_commands_run_on_the_pinned_cli() {
    for command in [
        "study-save-version",
        "model-import-study",
        "remote-study-run",
        "remote-study-fetch",
        "study-verified-side",
        "study-verify-comparison",
        "study-clone",
    ] {
        let root = temp(&format!("pin-cli-{command}"));
        let log = root.join("args.log");
        let env_log = root.join("env.log");
        let cli = fake_cli(
            &root,
            &format!(
                "printf '%s\\n' \"$@\" > '{log}'\n\
                 printf '%s %s\\n' \"$COLM_STUDY_BY_PARENT\" \"$COLM_AUDIT_BY_PARENT\" > '{env_log}'\n\
                 exit 0\n",
                log = log.display(),
                env_log = env_log.display(),
            ),
        );
        let study = pinned_study(&root, cli);
        let opts = pinned_command_opts(command, &study);
        let program = delegated_program(command, &opts);
        assert_eq!(
            run_pinned_command(&program, command, &opts, None).unwrap(),
            0,
            "{command} 应该交给 pin 的 CLI"
        );
        let args: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(args, pinned_command_args(command, &opts), "{command}");
        assert_eq!(
            std::fs::read_to_string(&env_log).unwrap().trim(),
            "1 1",
            "{command} 要防循环且不再单独记审计"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(unix)]
#[test]
fn a_delegated_identity_command_returns_the_pinned_clis_exit_status() {
    for command in [
        "remote-study-fetch",
        "study-verified-side",
        "study-verify-comparison",
    ] {
        let root = temp(&format!("pin-cli-exit-{command}"));
        let cli = fake_cli(&root, "exit 7\n");
        let study = pinned_study(&root, cli);
        let opts = pinned_command_opts(command, &study);
        let program = delegated_program(command, &opts);
        assert_eq!(
            run_pinned_command(&program, command, &opts, None).unwrap(),
            7,
            "{command} 的退出码要原样交回"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn a_pin_that_names_the_running_program_stays_local() {
    let root = temp("pin-cli-current");
    for command in [
        "study-save-version",
        "study-verified-side",
        "study-verify-comparison",
        "study-clone",
    ] {
        let study = pinned_study(&root.join(command), std::env::current_exe().unwrap());
        let opts = pinned_command_opts(command, &study);
        assert_local(command, &opts);
    }
    // 只读命令不重算 provenance，也不该整条转交。
    let cli_elsewhere = root.join("other-colm-cli");
    std::fs::write(&cli_elsewhere, "not a program").unwrap();
    let study = pinned_study(&root.join("readonly"), cli_elsewhere);
    let opts = pinned_command_opts("study-save-version", &study);
    assert_local("study-status", &opts);
    // `study-run` 自己按 `--model-version` 拍快照、按快照转交；不能在这里再被整条转交一次。
    assert_local("study-run", &opts);
    // 没有 model-version.json 的研究没有 pin，按当前程序执行。
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let opts = crate::Opts {
        positional: vec![plain.to_string_lossy().into_owned()],
        flags: Vec::new(),
    };
    assert_local("study-save-version", &opts);
    let _ = std::fs::remove_dir_all(&root);
}

/// 两侧 pin 到同一份程序：它是唯一能按正确参数目录同时核验两侧的程序，整条命令交给它。
#[cfg(unix)]
#[test]
fn a_comparison_bound_to_one_pinned_cli_is_delegated_to_it() {
    let root = temp("pin-cli-compare-delegate");
    let log = root.join("args.log");
    let cli = fake_cli(
        &root,
        &format!("printf '%s\\n' \"$@\" > '{}'\nexit 0\n", log.display()),
    );
    let study_a = pinned_study(&root.join("a"), cli.clone());
    let study_b = pinned_study(&root.join("b"), cli.clone());
    let opts = comparison_opts(&study_a, &study_b);
    let program = delegated_program("study-compare", &opts);
    assert_eq!(program, cli);
    assert_eq!(
        run_pinned_command(&program, "study-compare", &opts, None).unwrap(),
        0
    );
    let args: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    // 两个位置参数与 flags 都要按原顺序过去：少一个就变成“只比较一侧”。
    assert_eq!(args, pinned_command_args("study-compare", &opts));
    let _ = std::fs::remove_dir_all(&root);
}

/// 两侧绑定不同程序时，任何一份 CLI 都只能代表自己那一版；比较两个模型版本是受支持
/// 的功能，所以判定退回本地（compare 自己把 inputs_verified/same_inputs 降级为 false），
/// 而不是拒绝整个比较；同时确认不会启动任何一侧的程序。
#[cfg(unix)]
#[test]
fn a_comparison_with_mixed_pins_stays_local_without_starting_either_program() {
    let current = std::env::current_exe().unwrap();
    for case in ["one_sided", "different", "side_is_current"] {
        let root = temp(&format!("pin-cli-compare-mixed-{case}"));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let log_a = root.join("a.log");
        let log_b = root.join("b.log");
        let cli_a = fake_cli(
            &root.join("a"),
            &format!("printf 'ran\\n' > '{}'\n", log_a.display()),
        );
        let study_a = pinned_study(&root.join("a"), cli_a);
        let study_b = match case {
            "one_sided" => {
                let dir = root.join("b/study");
                std::fs::create_dir_all(&dir).unwrap();
                dir
            }
            "different" => {
                let cli_b = fake_cli(
                    &root.join("b"),
                    &format!("printf 'ran\\n' > '{}'\n", log_b.display()),
                );
                pinned_study(&root.join("b"), cli_b)
            }
            "side_is_current" => pinned_study(&root.join("b"), current.clone()),
            other => panic!("unknown case {other}"),
        };
        let opts = comparison_opts(&study_a, &study_b);
        assert_local("study-compare", &opts);
        assert!(
            !log_a.exists() && !log_b.exists(),
            "{case}: 不该启动任何一侧的程序"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 两侧都没有 pin（或都指当前程序）时没有身份差异，按原路径比较。
#[test]
fn a_comparison_without_pins_stays_local() {
    let root = temp("pin-cli-compare-local");
    let current = std::env::current_exe().unwrap();
    let plain_a = root.join("a/study");
    let plain_b = root.join("b/study");
    std::fs::create_dir_all(&plain_a).unwrap();
    std::fs::create_dir_all(&plain_b).unwrap();
    assert_local("study-compare", &comparison_opts(&plain_a, &plain_b));
    let study_a = pinned_study(&root.join("pinned-a"), current.clone());
    let study_b = pinned_study(&root.join("pinned-b"), current);
    assert_local("study-compare", &comparison_opts(&study_a, &study_b));
    let _ = std::fs::remove_dir_all(&root);
}

/// 所有 `--studies` 都 pin 到同一份程序：整条命令交给它，特征提取与逐研究的
/// `verify_frozen_inputs` 才会用同一版参数目录与 `colm-rs`。
#[cfg(unix)]
#[test]
fn hybrid_fit_over_studies_bound_to_one_program_is_delegated() {
    let root = temp("pin-cli-hybrid-delegate");
    let log = root.join("args.log");
    let env_log = root.join("env.log");
    let cli = fake_cli(
        &root,
        &format!(
            "printf '%s\\n' \"$@\" > '{}'\n             printf '%s %s\\n' \"$COLM_STUDY_BY_PARENT\" \"$COLM_AUDIT_BY_PARENT\" > '{}'\n             exit 0\n",
            log.display(),
            env_log.display()
        ),
    );
    let study_a = pinned_study(&root.join("a"), cli.clone());
    let study_b = pinned_study(&root.join("b"), cli.clone());
    let opts = hybrid_fit_opts(&[&study_a, &study_b]);
    let program = delegated_program("hybrid-fit", &opts);
    assert_eq!(program, cli);
    assert_eq!(
        run_pinned_command(&program, "hybrid-fit", &opts, None).unwrap(),
        0
    );
    let args: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    // `--studies a,b` 是一个逗号列表，必须原样作为一个参数交给子 CLI。
    assert_eq!(args, pinned_command_args("hybrid-fit", &opts));
    assert_eq!(
        std::fs::read_to_string(&env_log).unwrap().trim(),
        "1 1",
        "防循环且审计由父进程记"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 混合 pin：落到本地会拿当前程序校验旧研究的输入、抽特征；落到任何一份旧程序又会用错
/// 另一批。没有一份程序能代表全部研究，只能拒绝并让使用者先统一来源 revision。
#[cfg(unix)]
#[test]
fn hybrid_fit_with_mixed_pins_is_refused() {
    let current = std::env::current_exe().unwrap();
    for case in ["one_sided", "different"] {
        let root = temp(&format!("pin-cli-hybrid-mixed-{case}"));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let log_a = root.join("a.log");
        let log_b = root.join("b.log");
        let cli_a = fake_cli(
            &root.join("a"),
            &format!("printf 'ran\\n' > '{}'\n", log_a.display()),
        );
        let study_a = pinned_study(&root.join("a"), cli_a);
        let study_b = match case {
            "one_sided" => {
                let dir = root.join("b/study");
                std::fs::create_dir_all(&dir).unwrap();
                dir
            }
            "different" => {
                let cli_b = fake_cli(
                    &root.join("b"),
                    &format!("printf 'ran\\n' > '{}'\n", log_b.display()),
                );
                pinned_study(&root.join("b"), cli_b)
            }
            other => panic!("unknown case {other}"),
        };
        let opts = hybrid_fit_opts(&[&study_a, &study_b]);
        let reason = refusal("hybrid-fit", &opts);
        assert!(
            reason.contains("different model-version programs"),
            "{case}: {reason}"
        );
        assert!(
            reason.contains("feature extraction and frozen-input verification"),
            "{case}: {reason}"
        );
        assert!(reason.contains("revision"), "{case}: {reason}");
        assert!(
            !log_a.exists() && !log_b.exists(),
            "{case}: 不该启动任何程序"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
    // 全部空 pin 或全部指当前程序：没有身份差异，按原路径执行。
    let root = temp("pin-cli-hybrid-local");
    let plain_a = root.join("a/study");
    let plain_b = root.join("b/study");
    std::fs::create_dir_all(&plain_a).unwrap();
    std::fs::create_dir_all(&plain_b).unwrap();
    assert_local("hybrid-fit", &hybrid_fit_opts(&[&plain_a, &plain_b]));
    let study_a = pinned_study(&root.join("pinned-a"), current.clone());
    let study_b = pinned_study(&root.join("pinned-b"), current);
    assert_local("hybrid-fit", &hybrid_fit_opts(&[&study_a, &study_b]));
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_second_delegation_is_stopped_by_the_parent_marker() {
    for command in [
        "remote-study-run",
        "study-verified-side",
        "study-verify-comparison",
        "study-clone",
    ] {
        let root = temp(&format!("pin-cli-loop-{command}"));
        let log = root.join("args.log");
        let cli = fake_cli(&root, &format!("printf 'ran\\n' > '{}'\n", log.display()));
        let study = pinned_study(&root, cli);
        let opts = pinned_command_opts(command, &study);
        let _guard = DelegatedEnv::set();
        assert_local(command, &opts);
        assert!(!log.exists(), "父进程交出来的程序不能再转交一次");
        drop(_guard);
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 多研究命令同样受父进程标记约束：父进程已经交出整条命令后，子进程不能再把它交回去。
#[cfg(unix)]
#[test]
fn a_parent_marker_also_stops_multi_study_delegation() {
    let root = temp("pin-cli-loop-multi");
    let log = root.join("args.log");
    let cli = fake_cli(&root, &format!("printf 'ran\\n' > '{}'\n", log.display()));
    let study_a = pinned_study(&root.join("a"), cli.clone());
    let study_b = pinned_study(&root.join("b"), cli);
    let compare = comparison_opts(&study_a, &study_b);
    let hybrid = hybrid_fit_opts(&[&study_a, &study_b]);
    let _guard = DelegatedEnv::set();
    assert_local("study-compare", &compare);
    assert_local("hybrid-fit", &hybrid);
    assert!(!log.exists(), "父进程交出来的程序不能再转交一次");
    drop(_guard);
    let _ = std::fs::remove_dir_all(&root);
}

/// 远程命令的 SSH 凭据只能从 stdin 读一次：父进程替它读走之后必须原样转给真正执行的程序。
#[cfg(unix)]
#[test]
fn a_delegated_remote_command_receives_the_stdin_credentials() {
    let root = temp("pin-cli-auth");
    let log = root.join("stdin.log");
    let cli = fake_cli(&root, &format!("cat > '{}'\n", log.display()));
    let study = pinned_study(&root, cli);
    let opts = pinned_command_opts("remote-study-fetch", &study);
    let program = delegated_program("remote-study-fetch", &opts);
    let payload = "{\"host\":\"server\",\"auth\":\"password\"}";
    assert_eq!(
        run_pinned_command(&program, "remote-study-fetch", &opts, Some(payload)).unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!("{payload}\n")
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_pin_whose_program_is_missing_is_refused_with_an_actionable_error() {
    let root = temp("pin-cli-missing");
    let missing = root.join("deleted-colm-cli");
    let study = pinned_study(&root, missing.clone());
    let opts = pinned_command_opts("remote-study-fetch", &study);
    let error = pinned_command_plan("remote-study-fetch", &opts).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("snow"), "{text}");
    assert!(text.contains(missing.to_str().unwrap()), "{text}");
    assert!(text.contains("restore"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}
