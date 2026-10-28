use super::*;

fn network(slot: &str, output: &str) -> HybridStudySpec {
    serde_json::from_value(serde_json::json!({
        "slot": slot,
        "features": ["pftfrac", "clim_tair"],
        "outputs": [{"name": output, "range": [10.0, 150.0]}],
    }))
    .unwrap()
}

#[test]
fn tap_rows_keep_their_class_and_named_values() {
    let rows = parse_tap(
        "patch,pft,class,pftfrac,porsl[1],physics:DEF_PFT_VMAX25\n\
         0,0,2,6.9e-1,5.0e-1,2.65e1\n\
         0,1,11,3.0e-1,5.0e-1,4.85e1\n",
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].class, 11);
    assert_eq!(rows[1].values["pftfrac"], 0.3);
    assert_eq!(rows[0].values["porsl[1]"], 0.5);
    assert!(!rows[0].values.contains_key("pft"));
    assert!(parse_tap("patch,pft\n0,0\n").is_err());
    assert!(parse_tap("patch,pft,class\n0,0\n").is_err());
}

#[test]
fn targets_follow_the_tuned_class_keys() {
    let mut parameters = BTreeMap::new();
    parameters.insert("DEF_PFT_VMAX25(3)".to_owned(), 41.0);
    parameters.insert("DEF_LC_VMAX25".to_owned(), 55.0);
    let none = BTreeMap::new();
    let row = |class: i64, physics: f64| TapRow {
        class,
        values: [
            ("physics:DEF_PFT_VMAX25".to_owned(), physics),
            ("physics:DEF_LC_VMAX25".to_owned(), physics),
        ]
        .into_iter()
        .collect(),
    };
    // PFT 类 2 的键是 (3)；类 11 没有率定，这一行不进样本。
    let pft = network("pft", "DEF_PFT_VMAX25");
    assert_eq!(
        row_targets(&pft, &parameters, &none, &row(2, 20.5)),
        Some(vec![41.0])
    );
    assert_eq!(row_targets(&pft, &parameters, &none, &row(11, 20.5)), None);
    // 相对输出：率定值 ÷ 有效查表值。
    let mut relative = pft.clone();
    relative.outputs[0].relative = true;
    assert_eq!(
        row_targets(&relative, &parameters, &none, &row(2, 20.5)),
        Some(vec![2.0])
    );
    assert_eq!(
        row_targets(&relative, &parameters, &none, &row(2, 0.0)),
        None
    );
    // 地类参数只用于 Study 里它所属的地类。
    let lc = network("land_class", "DEF_LC_VMAX25");
    let mut scoped = BTreeMap::new();
    scoped.insert("DEF_LC_VMAX25".to_owned(), 1);
    assert_eq!(
        row_targets(&lc, &parameters, &scoped, &row(1, 1.0)),
        Some(vec![55.0])
    );
    assert_eq!(row_targets(&lc, &parameters, &scoped, &row(4, 1.0)), None);
    assert_eq!(
        row_targets(&lc, &parameters, &none, &row(4, 1.0)),
        Some(vec![55.0])
    );
}

#[test]
fn leave_one_study_out_reports_each_held_out_group() {
    // 三个“站”，目标是特征的线性函数：留一站也能预测准。
    let net = network("pft", "DEF_PFT_VMAX25");
    let outputs = output_specs(&net);
    let rows: Vec<Row> = (0..30)
        .map(|i| {
            let (a, b) = (f64::from(i % 10) / 10.0, 270.0 + f64::from(i));
            let raw = 0.5 * a + 0.02 * (b - 285.0);
            Row {
                group: i as usize % 3,
                features: vec![a, b],
                targets: vec![colm_hybrid::apply_output(&outputs[0], raw).unwrap()],
                weight: 1.0,
            }
        })
        .collect();
    let outcome = fit_rows(&rows, &outputs, &FitOptions::default(), 3).unwrap();
    assert!(outcome.train[0] < 1e-6, "{:?}", outcome.train);
    assert_eq!(outcome.held_out.len(), 3);
    for rmse in &outcome.held_out {
        assert!(rmse.as_ref().unwrap()[0] < 1e-6);
    }
    // 关系是真的：网络远好于均值基准，过门槛。
    let (network, baseline) = outcome.pooled.clone().unwrap();
    assert!(network[0] < baseline[0] / 100.0, "{network:?} {baseline:?}");
    assert_eq!(outcome.passes_gate(), Some(true));
    // 只有一组时不做交叉验证。
    let one: Vec<Row> = rows
        .iter()
        .cloned()
        .map(|row| Row { group: 0, ..row })
        .collect();
    let outcome = fit_rows(&one, &outputs, &FitOptions::default(), 1).unwrap();
    assert_eq!(outcome.held_out, vec![None]);
    assert_eq!(outcome.passes_gate(), None);
}

#[test]
fn noise_fails_the_cross_validation_gate() {
    // 目标与特征无关（确定性的伪随机）：网络留一组时不会比均值好。
    let net = network("pft", "DEF_PFT_VMAX25");
    let outputs = output_specs(&net);
    let rows: Vec<Row> = (0..24)
        .map(|i| {
            let noise = ((i * 7919 + 13) % 97) as f64 / 97.0;
            Row {
                group: i as usize % 6,
                features: vec![f64::from(i % 5), 270.0 + f64::from((i * 3) % 11)],
                targets: vec![30.0 + 40.0 * noise],
                weight: 1.0,
            }
        })
        .collect();
    let outcome = fit_rows(&rows, &outputs, &FitOptions::default(), 6).unwrap();
    let (network, baseline) = outcome.pooled.clone().unwrap();
    assert!(network[0] >= baseline[0], "{network:?} {baseline:?}");
    assert_eq!(outcome.passes_gate(), Some(false));
}

// ==== P1-6：特征提取读研究创建时冻结的输入，而不是原项目算例 ====

/// 每个测试一个临时项目根；canonicalize 掉 macOS 的 /var → /private/var 符号链接，
/// 否则 `engine::status` 的 root 与目录比对会失败。
fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "colm-hybrid-fit-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write_case(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("case.nml"),
        "&nl_colm\nDEF_CASE_NAME='caseA'\nSITE_fsitedata='site.nc'\n\
         DEF_forcing_namelist='forcing.nml'\nDEF_TUNING_CNFAC=0.5\n/\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("forcing.nml"),
        "&nl_colm_forcing\nDEF_dir_forcing='.'\nDEF_forcing_fprefix='forcing.nc'\n/\n",
    )
    .unwrap();
    std::fs::write(dir.join("site.nc"), b"site contents").unwrap();
    std::fs::write(dir.join("forcing.nc"), vec![0_u8; 256 * 1024]).unwrap();
}

fn copy_case(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for name in ["case.nml", "forcing.nml", "site.nc", "forcing.nc"] {
        std::fs::copy(from.join(name), to.join(name)).unwrap();
    }
}

struct Fixture {
    project: PathBuf,
    study: PathBuf,
    frozen: PathBuf,
    manifest: crate::study::spec::Manifest,
}

fn pft_parameters() -> serde_json::Value {
    serde_json::json!([{
        "name": "DEF_PFT_VMAX25",
        "scope_instance": {"kind": "pft-type", "index": 0},
        "sample_min": 20.0,
        "sample_max": 100.0,
    }])
}

fn case_scalar_parameters() -> serde_json::Value {
    serde_json::json!([{"name": "DEF_TUNING_CNFAC", "sample_min": 0.4, "sample_max": 0.6}])
}

fn sha_json<T: serde::Serialize>(value: &T) -> String {
    colm_kernel::sha256_hex(&serde_json::to_vec(value).unwrap())
}

fn write_manifest(
    study: &Path,
    project: &Path,
    parameters: serde_json::Value,
    hybrid: Option<serde_json::Value>,
) -> crate::study::spec::Manifest {
    let mut spec = serde_json::json!({
        "kind": "tuning",
        "method": "differential-evolution",
        "seed": 7,
        "base_cases": ["caseA"],
        "observations": {"caseA": project.join("obs.nc").to_string_lossy()},
        "parameters": parameters,
        "targets": [{"key": "qle", "variable": "Qle", "from": 0, "to": 3600, "min_pairs": 2}],
        "budget": {"population": 4, "generations": 1},
    });
    if let Some(hybrid) = hybrid {
        spec["hybrid"] = hybrid;
    }
    let spec: crate::study::spec::StudySpec = serde_json::from_value(spec).unwrap();
    let samples = "member,baseline,generation,candidate\n";
    std::fs::create_dir_all(study.join("samples")).unwrap();
    std::fs::write(study.join("samples/g000000.csv"), samples).unwrap();
    let mut manifest = crate::study::spec::Manifest {
        schema_version: 1,
        id: study.file_name().unwrap().to_string_lossy().into_owned(),
        root: study.canonicalize().unwrap().display().to_string(),
        created_unix: 0,
        spec,
        members: Vec::new(),
        provenance: Default::default(),
    };
    manifest.provenance.spec_sha256 = sha_json(&manifest.spec);
    manifest.provenance.required_targets_sha256 = sha_json(&manifest.spec.targets);
    manifest.provenance.outputs_sha256 = sha_json(&manifest.spec.outputs);
    manifest.provenance.samples_sha256 = colm_kernel::sha256_hex(samples.as_bytes());
    std::fs::write(
        study.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    manifest
}

/// 一个“第一步”调优 Study：原项目里有一份算例，创建时把它冻到 `.colm/model-inputs/`
/// （与 `study-create --model-version` 的真实路径相同），再按冻结副本记输入身份。
fn frozen_fixture(label: &str, parameters: serde_json::Value) -> Fixture {
    frozen_fixture_with(label, parameters, None)
}

fn frozen_fixture_with(
    label: &str,
    parameters: serde_json::Value,
    hybrid: Option<serde_json::Value>,
) -> Fixture {
    let project = scratch(label);
    write_case(&project.join("caseA"));
    std::fs::write(project.join("obs.nc"), b"observation fixture").unwrap();
    let study = project.join(".colm/studies").join(label);
    std::fs::create_dir_all(&study).unwrap();
    let frozen = project.join(".colm/model-inputs/op");
    copy_case(&project.join("caseA"), &frozen.join("caseA"));
    let manifest = write_manifest(&study, &project, parameters, hybrid);
    crate::study::model::write_inputs(&study, &frozen).unwrap();
    crate::study::model::capture_inputs(&manifest).unwrap();
    Fixture {
        project,
        study,
        frozen,
        manifest,
    }
}

/// 一个假的 `colm-rs`：`FeatureProgram` 只读它的内容算 hash，测试里不会真的执行。
fn program(project: &Path, bytes: &[u8]) -> FeatureProgram {
    let path = project.join("colm-rs");
    std::fs::write(&path, bytes).unwrap();
    FeatureProgram::of(&path)
}

fn member() -> MemberPlan {
    MemberPlan {
        id: "m000001".into(),
        generation: 0,
        candidate_index: 0,
        baseline: false,
        parameters: BTreeMap::from([("DEF_PFT_VMAX25(1)".to_owned(), 42.0)]),
    }
}

/// 假 tap：从算例目录的 `tap_value.txt` 读特征值。真实路径上这里是 colm-rs 空跑，
/// 测试要断言的是“读的是哪个算例目录、哪个副本里的数据”。
fn marker_tap(case: &Path) -> Result<String> {
    let value: f64 = std::fs::read_to_string(case.join("tap_value.txt"))?
        .trim()
        .parse()?;
    Ok(format!(
        "patch,pft,class,pftfrac,clim_tair\n0,0,0,{value},285.0\n"
    ))
}

#[test]
fn features_come_from_the_frozen_inputs_after_the_source_case_changed() {
    let fixture = frozen_fixture("frozen-inputs", pft_parameters());
    let program = program(&fixture.project, b"colm-rs fixture");
    // 冻结输入与原项目不同：冻结副本里是 3.5，原项目算例被改成了 9.9、换了一份站点数据与强迫。
    std::fs::write(fixture.frozen.join("caseA/tap_value.txt"), "3.5\n").unwrap();
    std::fs::write(fixture.project.join("caseA/tap_value.txt"), "9.9\n").unwrap();
    std::fs::write(
        fixture.project.join("caseA/site.nc"),
        b"source site changed",
    )
    .unwrap();
    std::fs::write(
        fixture.project.join("caseA/forcing.nc"),
        b"source forcing changed",
    )
    .unwrap();

    let prepared = prepare_study(&fixture.study).unwrap();
    assert_eq!(
        prepared.base_root,
        fixture.frozen.canonicalize().unwrap(),
        "the frozen input root recorded at creation must win over the source project"
    );
    assert_eq!(
        prepared.source_project_root.as_deref(),
        Some(fixture.project.as_path()),
        "the report still names the source project, but no row is read from it"
    );

    let network = network("pft", "DEF_PFT_VMAX25");
    let mut rows = Vec::new();
    let mut sources = Vec::new();
    let mut tapped = Vec::new();
    {
        let mut tap_case = |case: &Path| -> Result<String> {
            tapped.push(case.to_path_buf());
            marker_tap(case)
        };
        collect_study_rows(
            0,
            &prepared,
            &member(),
            &network,
            None,
            &program,
            &mut tap_case,
            &mut rows,
            &mut sources,
        )
        .unwrap();
    }

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].features, vec![3.5, 285.0]);
    assert_eq!(
        tapped,
        vec![fixture.frozen.canonicalize().unwrap().join("caseA")]
    );
    // sources 本身带可追溯身份：冻结根、规格哈希、逐站数据哈希。
    let identity = &prepared.identity.base_cases["caseA"];
    assert_eq!(
        sources[0]["base_root"],
        fixture.frozen.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        sources[0]["data_sha256"],
        serde_json::json!(identity.data_sha256)
    );
    assert_eq!(
        sources[0]["spec_sha256"],
        fixture.manifest.provenance.spec_sha256
    );
    assert_eq!(
        sources[0]["feature_program_sha256"],
        serde_json::json!(program.sha256)
    );
    std::fs::remove_dir_all(&fixture.project).unwrap();
}

#[test]
fn tampered_frozen_inputs_are_rejected() {
    let fixture = frozen_fixture("tampered", pft_parameters());
    prepare_study(&fixture.study).unwrap();
    // 冻结副本里被 tap 读到的数据（站点文件，`sha256-nonforcing-v1` 逐文件入账）在创建后被改：
    // 全内容身份必须拦住。强迫字节按仓库策略不进身份（tap 只读重启常数与 hybrid_climate，
    // 后者同样是入账文件），所以这里改的是 site.nc。
    let site = fixture.frozen.join("caseA/site.nc");
    let mut bytes = std::fs::read(&site).unwrap();
    bytes[0] = b'X';
    std::fs::write(&site, &bytes).unwrap();
    let error = prepare_study(&fixture.study).unwrap_err();
    assert!(
        format!("{error:#}").contains("changed"),
        "unexpected error: {error:#}"
    );
    std::fs::remove_dir_all(&fixture.project).unwrap();
}

#[test]
fn a_study_without_a_full_input_identity_is_rejected() {
    let fixture = frozen_fixture("legacy-inputs", pft_parameters());
    std::fs::remove_file(fixture.study.join(crate::study::model::IDENTITY_FILE)).unwrap();
    let error = prepare_study(&fixture.study).unwrap_err();
    assert!(
        format!("{error:#}").contains("input identity"),
        "unexpected error: {error:#}"
    );
    std::fs::remove_dir_all(&fixture.project).unwrap();
}

#[test]
fn a_study_that_trains_its_own_network_cannot_be_a_first_step() {
    let hybrid = serde_json::json!({
        "slot": "pft",
        "features": ["pftfrac"],
        "outputs": [{"name": "DEF_PFT_VMAX25", "range": [10.0, 150.0]}],
        "normalize": {"mean": [0.5], "std": [0.2]},
    });
    let fixture = frozen_fixture_with("hybrid-study", serde_json::json!([]), Some(hybrid));
    let error = prepare_study(&fixture.study).unwrap_err();
    assert!(
        format!("{error:#}").contains("hybrid"),
        "unexpected error: {error:#}"
    );
    std::fs::remove_dir_all(&fixture.project).unwrap();
}

#[test]
fn studies_tuning_the_wrong_parameter_scope_are_rejected_together() {
    let pft = frozen_fixture("compat-pft", pft_parameters());
    let scalar = frozen_fixture("compat-scalar", case_scalar_parameters());
    let network = network("pft", "DEF_PFT_VMAX25");
    let studies = [
        prepare_study(&pft.study).unwrap(),
        prepare_study(&scalar.study).unwrap(),
    ];
    let error = check_compatibility(&studies, &network).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("pft"), "unexpected error: {text}");
    assert!(text.contains("compat-scalar"), "unexpected error: {text}");
    std::fs::remove_dir_all(&pft.project).unwrap();
    std::fs::remove_dir_all(&scalar.project).unwrap();
}

#[test]
fn a_missing_frozen_feature_program_is_a_reported_warning() {
    let pinned = frozen_fixture("program-pinned", pft_parameters());
    let legacy = frozen_fixture("program-legacy", pft_parameters());
    let bin = pinned.study.join("version/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    };
    std::fs::write(bin.join(exe("colm-rs")), b"another colm-rs").unwrap();
    let pin = crate::study_version::Pinned {
        name: "fixture".into(),
        head: "head".into(),
        base_commit: "base".into(),
        note: String::new(),
        based_on: None,
        touches: colm_workspace::version::Touches {
            engine: false,
            kernel: false,
        },
        cli: bin.join(exe("colm-cli")),
        kernel: bin.clone(),
        pinned_unix: 0,
        model_revision: None,
        model_root: None,
    };
    std::fs::write(
        pinned.study.join(crate::study_version::FILE),
        serde_json::to_vec_pretty(&pin).unwrap(),
    )
    .unwrap();
    let program = program(&pinned.project, b"current colm-rs");
    let network = network("pft", "DEF_PFT_VMAX25");
    let studies = [
        prepare_study(&pinned.study).unwrap(),
        prepare_study(&legacy.study).unwrap(),
    ];
    let warnings = check_compatibility(&studies, &network).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("colm-rs"), "{}", warnings[0]);
    assert!(warnings[0].contains("program-legacy"), "{}", warnings[0]);
    // 没有 registry 修订的旧研究：报告标记 legacy，不报错；固定版本里的程序如实记下。
    let report = inputs_report(&studies, &network, &program, &warnings).unwrap();
    assert_eq!(report["studies"][0]["legacy_model_revision"], true);
    assert_eq!(report["studies"][0]["model_source"]["name"], "fixture");
    assert_eq!(
        report["studies"][0]["frozen_feature_program"]["path"],
        bin.join(exe("colm-rs")).display().to_string()
    );
    assert!(report["studies"][1]["frozen_feature_program"].is_null());
    assert_eq!(report["warnings"][0], warnings[0]);
    std::fs::remove_dir_all(&pinned.project).unwrap();
    std::fs::remove_dir_all(&legacy.project).unwrap();
}

#[test]
fn the_report_records_each_study_input_identity_and_the_feature_definition() {
    let first = frozen_fixture("report-a", pft_parameters());
    let second = frozen_fixture("report-b", pft_parameters());
    let program = program(&first.project, b"colm-rs fixture");
    let network = network("pft", "DEF_PFT_VMAX25");
    let studies = [
        prepare_study(&first.study).unwrap(),
        prepare_study(&second.study).unwrap(),
    ];
    let warnings = vec!["a compatibility warning".to_owned()];
    let report = inputs_report(&studies, &network, &program, &warnings).unwrap();

    assert_eq!(
        report["feature_definition_sha256"],
        feature_definition_sha256(&network).unwrap()
    );
    assert_eq!(report["feature_program"]["path"], program.path);
    assert_eq!(
        report["feature_program"]["sha256"],
        serde_json::json!(program.sha256)
    );
    assert_eq!(report["cli_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["warnings"][0], "a compatibility warning");
    let rows = report["studies"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (row, study) in rows.iter().zip(&studies) {
        assert_eq!(row["spec_sha256"], study.manifest.provenance.spec_sha256);
        assert_eq!(row["base_root"], study.base_root.display().to_string());
        assert_eq!(
            row["source_project_root"],
            serde_json::json!(study
                .source_project_root
                .as_ref()
                .map(|root| root.display().to_string()))
        );
        let case = &study.identity.base_cases["caseA"];
        assert_eq!(row["base_cases"]["caseA"]["algorithm"], case.algorithm);
        assert_eq!(
            row["base_cases"]["caseA"]["data_sha256"],
            serde_json::json!(case.data_sha256)
        );
        assert_eq!(row["base_cases"]["caseA"]["complete"], true);
        let observation = &study.identity.observations["caseA"];
        assert_eq!(row["observations"]["caseA"]["sha256"], observation.sha256);
        assert_eq!(row["legacy_model_revision"], true);
        assert!(row["model_revision"].is_null());
        assert_eq!(row["feature_program"]["path"], program.path);
    }
    std::fs::remove_dir_all(&first.project).unwrap();
    std::fs::remove_dir_all(&second.project).unwrap();
}

#[test]
fn a_changed_source_project_without_a_frozen_copy_is_rejected() {
    // 研究创建时没选模型版本：没有 materialized 冻结副本，base_case 兜底回原项目根。
    // 原项目数据一旦被改，全内容身份必须拒绝，而不是拿新数据抽特征配旧参数。
    let project = scratch("source-only");
    write_case(&project.join("caseA"));
    std::fs::write(project.join("obs.nc"), b"observation fixture").unwrap();
    let study = project.join(".colm/studies/source-only");
    std::fs::create_dir_all(&study).unwrap();
    let manifest = write_manifest(&study, &project, pft_parameters(), None);
    crate::study::model::capture_inputs(&manifest).unwrap();
    std::fs::write(project.join("caseA/site.nc"), b"changed site").unwrap();
    let error = prepare_study(&study).unwrap_err();
    assert!(
        format!("{error:#}").contains("changed"),
        "unexpected error: {error:#}"
    );
    std::fs::remove_dir_all(&project).unwrap();
}

fn frozen_revision(id: &str, revision: u64) -> colm_model::FrozenRevision {
    serde_json::from_value(serde_json::json!({
        "schema": 1,
        "version": {"id": id, "revision": revision},
        "name": id,
        "note": "",
        "parent": null,
        "code": {"kind": "official", "workspace": null, "commit": null, "source_sha256": "a".repeat(64)},
        "overlay": {"parameters": [], "physics": {}, "hybrid": null, "process_files": []},
        "overlay_sha256": "b".repeat(64),
        "fingerprint": "c".repeat(64),
        "created_unix": 0,
        "applicability": {"sites": [], "land_classes": [], "calibration": null, "validation": null},
        "derived_from": null,
    }))
    .unwrap()
}

fn pin_revision(fixture: &Fixture, revision: colm_model::FrozenRevision) {
    let pin = crate::study_version::Pinned {
        name: revision.name.clone(),
        head: "head".into(),
        base_commit: "base".into(),
        note: String::new(),
        based_on: None,
        touches: colm_workspace::version::Touches {
            engine: false,
            kernel: false,
        },
        cli: fixture.study.join("version/bin/colm-cli"),
        kernel: fixture.study.join("version/bin"),
        pinned_unix: 0,
        model_revision: Some(revision),
        model_root: None,
    };
    std::fs::write(
        fixture.study.join(crate::study_version::FILE),
        serde_json::to_vec_pretty(&pin).unwrap(),
    )
    .unwrap();
}

#[test]
fn studies_from_different_frozen_revisions_are_reported_and_traced() {
    let first = frozen_fixture("revision-a", pft_parameters());
    let second = frozen_fixture("revision-b", pft_parameters());
    pin_revision(&first, frozen_revision("rev-a", 1));
    pin_revision(&second, frozen_revision("rev-b", 2));
    let program = program(&first.project, b"colm-rs fixture");
    let network = network("pft", "DEF_PFT_VMAX25");
    let studies = [
        prepare_study(&first.study).unwrap(),
        prepare_study(&second.study).unwrap(),
    ];
    let warnings = check_compatibility(&studies, &network).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("different frozen model revisions"),
        "{}",
        warnings[0]
    );
    let report = inputs_report(&studies, &network, &program, &warnings).unwrap();
    assert_eq!(
        report["studies"][0]["model_revision"]["version"]["id"],
        "rev-a"
    );
    assert_eq!(report["studies"][0]["legacy_model_revision"], false);
    assert_eq!(
        report["studies"][1]["model_revision"]["version"]["revision"],
        2
    );
    std::fs::remove_dir_all(&first.project).unwrap();
    std::fs::remove_dir_all(&second.project).unwrap();
}
