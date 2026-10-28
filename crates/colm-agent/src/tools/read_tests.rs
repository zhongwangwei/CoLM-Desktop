use super::*;

#[cfg(unix)]
#[test]
fn selected_model_configuration_refreshes_after_edits_without_redirecting_run_status() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("effective-model");
    let original = case(&root, "A", " DEF_USE_PFT=.false.\n SITE_lat=1.0\n");
    std::fs::write(original.join("run.out"), "original run log").unwrap();
    let cli = root.join("cli");
    std::fs::write(
        &cli,
        r#"#!/bin/sh
[ "$1" = model-preview-context ] || exit 1
shift
while [ "$#" -gt 0 ]; do
 case "$1" in
 --model-root|--model-id|--model-revision|--operation-id) shift ;;
 --out) out="$2"; shift ;;
 *) src="$1" ;;
 esac
 shift
done
mkdir -p "$out"
sed 's/DEF_USE_PFT=.false./DEF_USE_PFT=.true./' "$src/case.nml" > "$out/case.nml"
printf '{"effective_case":"%s","run":{"version":{"id":"modified","revision":1}}}\n' "$out"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        project_root: root.clone(),
        cli,
        model_context: Some(crate::tools::ModelContext {
            model: json!({"version":{"id":"modified","revision":1},"code":{"source_sha256":"a".repeat(64)},"overlay":{"parameters":[]}}),
            model_root: root.join("models"),
            source_dir: root.join("source"),
            docs_dir: None,
            source_case: Some(original.clone()),
        }),
        ..ToolContext::default()
    };
    let read = || {
        ReadCaseConfig
            .call(&json!({"case":"A","names":null,"filter":null}), &ctx)
            .unwrap()
    };
    assert_eq!(read()["fields"]["case.nml"]["DEF_USE_PFT"], ".true.");
    std::fs::write(
        original.join("case.nml"),
        "&nl_colm\n DEF_USE_PFT=.false.\n SITE_lat=42.0\n/\n",
    )
    .unwrap();
    let fresh = read();
    assert_eq!(
        fresh["fields"]["case.nml"]["SITE_lat"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        42.0
    );
    assert_eq!(fresh["model_version"]["revision"], 1);
    assert!(std::fs::read_to_string(original.join("case.nml"))
        .unwrap()
        .contains("DEF_USE_PFT=.false."));
    let status = RunStatus.call(&json!({"case":"A"}), &ctx).unwrap();
    assert_eq!(status["log_tails"]["run.out"], "original run log");
    assert_eq!(
        std::fs::read_dir(root.join("models/assistant-previews"))
            .unwrap()
            .count(),
        0
    );
    let _ = std::fs::remove_dir_all(root);
}

fn temp(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-agent-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn case(root: &Path, name: &str, body: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), format!("&nl_colm\n{body}/\n")).unwrap();
    dir
}

#[test]
fn months_follow_the_civil_calendar() {
    assert_eq!(year_month(0), (1970, 1));
    assert_eq!(year_month(1_041_381_000), (2003, 1)); // 2003-01-01 00:30 UTC
    assert_eq!(year_month(951_782_400), (2000, 2)); // 2000-02-29
    assert_eq!(year_month(-1), (1969, 12));
}

#[test]
fn series_summaries_skip_missing_values() {
    let jan = 1_041_381_000;
    let feb = jan + 31 * 86_400;
    let stats = summarize(&[jan, jan + 3600, feb], &[Some(1.0), None, Some(4.0)]);
    assert_eq!(stats["count"], 3);
    assert_eq!(stats["nan_or_missing"], 1);
    assert_eq!(stats["mean"], 2.5);
    assert_eq!(stats["min"], 1.0);
    assert_eq!(stats["max"], 4.0);
    assert_eq!(stats["monthly_mean"]["2003-01"], 1.0);
    assert_eq!(stats["monthly_mean"]["2003-02"], 4.0);
}

#[test]
fn cases_are_listed_read_and_compared() {
    let root = temp("cases");
    let a = case(
        &root,
        "A",
        " DEF_USE_PC = .true.\n DEF_CASE_NAME = 'A'\n DEF_PFT_VMAX25(3) = 41.0\n",
    );
    let b = case(
        &root.join("group"),
        "B",
        " DEF_CASE_NAME = 'B'\n DEF_PFT_VMAX25(3) = 20.0\n",
    );
    std::fs::write(b.join("hybrid.toml"), "").unwrap();
    std::fs::create_dir_all(root.join(".colm/studies/x")).unwrap();
    case(&root.join(".colm/studies/x"), "hidden", "");
    let ctx = ToolContext {
        project_root: root.clone(),
        ..ToolContext::default()
    };

    let listed = ListCases.call(&json!({ "root": null }), &ctx).unwrap();
    let cases = listed["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2, "{listed}");
    assert_eq!(cases[0]["mode"], "pc");
    assert_eq!(cases[1]["mode"], "lct");
    assert_eq!(cases[1]["has_hybrid"], true);

    let config = ReadCaseConfig
        .call(
            &json!({ "case": "A", "names": null, "filter": "vmax" }),
            &ctx,
        )
        .unwrap();
    let fields = &config["fields"]["case.nml"];
    assert_eq!(fields.as_object().unwrap().len(), 1, "{config}");
    assert!(fields
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .contains("VMAX25"));
    assert!(ReadCaseConfig
        .call(
            &json!({ "case": "nowhere", "names": null, "filter": null }),
            &ctx
        )
        .is_err());

    let diff = CompareCases
        .call(
            &json!({ "a": a.display().to_string(), "b": b.display().to_string() }),
            &ctx,
        )
        .unwrap();
    let differing = diff["differing_fields"].as_object().unwrap();
    assert!(differing.keys().any(|k| k.contains("VMAX25")), "{diff}");
    assert!(differing.keys().any(|k| k == "DEF_USE_PC"), "{diff}");
    // 算例名属于路径类字段，不算实质差异。
    assert!(!differing.contains_key("DEF_CASE_NAME"));
    assert_eq!(diff["hybrid"], json!([false, true]));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parameters_are_explained_from_the_catalog() {
    let ctx = ToolContext::default();
    let explained = ExplainParameter
        .call(&json!({ "name": "DEF_USE_PC", "case": null }), &ctx)
        .unwrap();
    assert!(explained["field"]["name"] == "DEF_USE_PC" || !explained["parameter"].is_null());
    assert!(ExplainParameter
        .call(&json!({ "name": "NOT_A_FIELD_AT_ALL", "case": null }), &ctx)
        .is_err());
}

#[test]
fn docs_are_searched_line_by_line() {
    let root = temp("docs");
    std::fs::write(root.join("a.md"), "intro\nVcmax compensates\nend\n").unwrap();
    std::fs::write(root.join("b.txt"), "Vcmax in a non-markdown file\n").unwrap();
    let ctx = ToolContext {
        docs_root: Some(root.clone()),
        cli: root.join("missing-cli"),
        ..ToolContext::default()
    };
    let found = SearchDocs.call(&json!({ "query": "vcmax" }), &ctx).unwrap();
    let matches = found["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["file"], "a.md");
    assert_eq!(matches[0]["line"], 2);
    assert_eq!(matches[0]["context_start_line"], 1);
    assert_eq!(
        matches[0]["context"],
        json!(["intro", "Vcmax compensates", "end"])
    );
    assert_eq!(matches[0]["path"], root.join("a.md").display().to_string());
    assert_eq!(
        found["provenance"]["application_source"]["version_status"],
        "unknown"
    );
    assert!(found["provenance"]["application_source"]["revision"].is_null());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parameter_activity_uses_selected_kernel_and_case_guards() {
    let root = temp("parameter-activity");
    let dir = case(
        &root,
        "A",
        " SITE_landtype=1\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.true.\n",
    );
    let kernel = root.join("kernel");
    std::fs::create_dir(&kernel).unwrap();
    std::fs::write(
        kernel.join("manifest.json"),
        r#"{"schema":1,"macros":["SinglePoint","LULC_IGBP"]}"#,
    )
    .unwrap();
    let ctx = ToolContext {
        project_root: root.clone(),
        kernel_dir: Some(kernel),
        ..ToolContext::default()
    };
    let explain = |name| {
        ExplainParameter
            .call(&json!({"name":name,"case":"A"}), &ctx)
            .unwrap()
    };
    assert_eq!(
        explain("DEF_WUE_LAMBDA")["applicability"]["status"],
        "active"
    );
    assert_eq!(
        explain("DEF_BALL_BERRY_GRADM")["applicability"]["status"],
        "inactive"
    );
    assert_eq!(
        explain("DEF_BALL_BERRY_GRADM")["applicability"]["calibration_eligible_for_case"],
        false
    );
    assert_eq!(explain("DEF_USE_PC")["applicability"]["status"], "unknown");
    let without_kernel = ToolContext {
        project_root: root.clone(),
        ..ToolContext::default()
    };
    assert_eq!(
        ExplainParameter
            .call(
                &json!({"name":"DEF_WUE_LAMBDA","case":"A"}),
                &without_kernel
            )
            .unwrap()["applicability"]["status"],
        "unknown"
    );
    std::fs::write(dir.join("case.nml"), "&nl_colm\n DEF_USE_WUEST=.true.\n/\n").unwrap();
    assert_eq!(
        explain("DEF_WUE_LAMBDA")["applicability"]["status"],
        "unknown"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn docs_version_is_reported_by_application_source_not_project_git() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("docs-version");
    let cli = root.join("cli");
    std::fs::write(&cli, "#!/bin/sh\n[ \"$*\" = 'ws-source-info --source app' ] || exit 1\nprintf '%s\\n' '{\"source\":\"/application/source\",\"revision\":\"app-sha\",\"tag\":\"v1\",\"dirty\":true,\"version_status\":\"identified\"}'\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        root.join("guide.md"),
        "energy closure\nphase change matters\n",
    )
    .unwrap();
    let ctx = ToolContext {
        cli,
        docs_root: Some(root.clone()),
        project_root: root.join("unrelated-project"),
        ..ToolContext::default()
    };
    let result = SearchDocs.call(&json!({"query":"closure"}), &ctx).unwrap();
    let source = &result["provenance"]["application_source"];
    assert_eq!(source["revision"], "app-sha");
    assert_eq!(source["dirty"], true);
    assert_eq!(source["source"], "/application/source");
    assert_eq!(result["matches"][0]["context"][1], "phase change matters");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn large_document_search_preserves_valid_json_and_explicit_truncation() {
    let root = temp("docs-budget");
    std::fs::write(
        root.join("guide.md"),
        ("soil ".to_owned() + &"quoted \\\" text ".repeat(40) + "\n").repeat(80),
    )
    .unwrap();
    let ctx = ToolContext {
        docs_root: Some(root.clone()),
        ..ToolContext::default()
    };
    let result = SearchDocs.call(&json!({"query":"soil"}), &ctx).unwrap();
    assert_eq!(result["truncated"], true);
    assert!(!result["matches"].as_array().unwrap().is_empty());
    let text = super::super::result_text(&result);
    assert!(text.chars().count() <= MAX_RESULT_CHARS);
    assert!(serde_json::from_str::<Value>(&text).is_ok());
    assert!(SearchDocs
        .call(&json!({"query":"x".repeat(1025)}), &ctx)
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn guessed_spinup_parameter_is_rejected_with_real_controls() {
    let error = ExplainParameter
        .call(
            &json!({"name":"DEF_SPINUP_YEARS","case":null}),
            &ToolContext::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("neither a namelist field"));
    assert!(error.contains("read_case_config"));
    assert!(error.contains("cutoff year, not a duration"));
    assert!(error.contains("visible case settings returned by ui_state"));
    let actual = ExplainParameter
        .call(
            &json!({"name":"DEF_simulation_time%spinup_year","case":null}),
            &ToolContext::default(),
        )
        .unwrap();
    assert_eq!(actual["field"]["name"], "DEF_simulation_time%spinup_year");
}

#[cfg(unix)]
#[test]
fn knowledge_search_checks_the_selected_workspace_and_withholds_unknown_cards() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("knowledge-workspace");
    let source = root.join("workspace-source");
    let docs = source.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(source.join("crates/core")).unwrap();
    let path = "crates/core/flux.rs";
    let text = "# energy\nOld formula in `crates/core/flux.rs`\n";
    std::fs::write(docs.join("colm-process-knowledge.md"), text).unwrap();
    std::fs::write(source.join(path), "fn energy() { 2 }\n").unwrap();
    let manifest = json!({"schema":1,"bound_at_unix_ms":1,"documents":{"colm-process-knowledge.md":{
        "document_sha256":knowledge::digest(text.as_bytes()),"sources":{path:knowledge::digest(b"fn energy() { 1 }\n")}}}});
    std::fs::write(docs.join("knowledge-sources.json"), manifest.to_string()).unwrap();
    let cli = root.join("cli");
    let info = json!({"source":source,"revision":"edited","dirty":true});
    std::fs::write(&cli, format!("#!/bin/sh\n[ \"$1\" = ws-source-info ] && [ \"$2\" = --name ] && [ \"$3\" = experiment ] || exit 1\nprintf '%s\\n' '{}'\n", info)).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        cli,
        docs_root: Some(docs.clone()),
        ..Default::default()
    };
    let result = SearchDocs
        .call(&json!({"query":"energy","name":"experiment"}), &ctx)
        .unwrap();
    assert_eq!(result["scope"]["kind"], "workspace");
    assert_eq!(
        result["knowledge_checks"]["colm-process-knowledge.md"]["status"],
        "needs_review"
    );
    assert!(!result["matches"][0]["context"]
        .to_string()
        .contains("Old formula"));
    assert_eq!(
        result["knowledge_checks"]["colm-process-knowledge.md"]["source_checks"][0]["next_tool"]
            ["arguments"]["name"],
        "experiment"
    );
    std::fs::write(docs.join("knowledge-sources.json"), "broken").unwrap();
    let unknown = SearchDocs
        .call(&json!({"query":"energy","name":"experiment"}), &ctx)
        .unwrap();
    assert_eq!(unknown["matches"][0]["knowledge_status"], "unknown");
    assert!(!unknown["matches"][0]["context"]
        .to_string()
        .contains("Old formula"));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hybrid_checks_dispatch_original_case_to_selected_frozen_runtime() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("frozen-hybrid");
    let original = case(&root, "A", " DEF_USE_PFT=.false.\n");
    let before = std::fs::read(original.join("case.nml")).unwrap();
    let cli = root.join("cli");
    std::fs::write(
        &cli,
        r#"#!/bin/sh
[ "$1" = model-hybrid-check ] || exit 1
printf '{"argv":"%s"}\n' "$*"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        cli,
        project_root: root.clone(),
        kernel_dir: Some(root.join("actual-kernel")),
        preprocessors: Some("fortran".into()),
        model_context: Some(crate::tools::ModelContext {
            model: json!({"version":{"id":"old-workspace","revision":1},"code":{"source_sha256":"b".repeat(64)}}),
            model_root: root.join("models"),
            source_dir: root.join("frozen-source"),
            docs_dir: None,
            source_case: Some(original.clone()),
        }),
        ..ToolContext::default()
    };
    let result = HybridCheck.call(&json!({"case":"A"}), &ctx).unwrap();
    let argv = result["argv"].as_str().unwrap();
    assert!(argv.starts_with("model-hybrid-check --model-root "));
    assert!(argv.contains("--model-id old-workspace --model-revision 1"));
    assert!(argv.contains(&format!(
        "{} --kernel {}",
        original.display(),
        root.join("actual-kernel").display()
    )));
    assert!(argv.ends_with("--preprocessors fortran"));
    assert_eq!(std::fs::read(original.join("case.nml")).unwrap(), before);
    assert!(
        !root.join("models/assistant-previews").exists(),
        "fixed CLI owns its isolated preview"
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---- P1-A：study_summary 消费 study-status.evidence ----

#[cfg(unix)]
fn fake_study_cli(root: &Path, status: &Value) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let status_path = root.join("study-status.json");
    std::fs::write(&status_path, status.to_string()).unwrap();
    let script = root.join("fake-cli");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1\" = study-status ] || exit 9\ncat '{}'\n",
            status_path.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    script
}

// 只被 cfg(unix) 的假 CLI 测试使用；Windows 上不编译那些测试。
#[cfg(unix)]
fn study_evidence(study_record: Value, dims: Value, best_values: Value, warnings: Value) -> Value {
    json!({
        "schema": 1,
        "study": {"kind": "tuning", "method": "differential-evolution", "status": "completed", "generation": 1},
        "execution": {"state": "completed", "tasks": {"total": 4}, "partial_failures": false, "failed_tasks": []},
        "selection": {"best_member": "m000005", "best_objective": 0.3, "provisional": false, "no_best_reason": null},
        "objective": {"direction": "minimize", "loss": "weighted mean of per-target losses", "targets": [{"key": "Qle"}]},
        "comparison": {
            "basis": "same period only",
            "baseline": {"member": "m000000",
                "calibration": {"status": "recorded", "score": 0.8, "targets": [{"key": "Qle", "loss": 0.8}]},
                "validation": {"status": "recorded", "score": 0.6, "targets": [{"key": "Qle", "loss": 0.6}]}},
            "best": {"member": "m000005",
                "calibration": {"status": "recorded", "score": 0.3, "targets": [{"key": "Qle", "loss": 0.3}]},
                "validation": {"status": "recorded", "score": 0.9, "targets": [{"key": "Qle", "loss": 0.9}]}},
            "relation": {"calibration": "lower_loss", "validation": "higher_loss"},
        },
        "validation": {"configured": true, "status": "available", "windows": [{"target": "Qle", "from": 10, "to": 20}], "reasons": []},
        "parameters": {
            "status": "recorded", "dimension_count": dims.as_array().map_or(0, Vec::len), "hybrid_weight_count": 0,
            "dims": dims,
            "baseline": {"status": "recorded", "values": [
                {"key": "DEF_TUNING_CNFAC", "value": 0.5, "position": 0.5, "position_scale": "linear", "at_bound": false}
            ]},
            "best": {"status": "recorded", "values": best_values},
        },
        "model_identity": {"study_record": study_record, "current_selection": null, "comparison": {"status": "unknown"}},
        "verification": {"recorded_result_files": 2, "reverified": false},
        "remote": {"status": "none"},
        "source_paths": {"root": "/study"},
        "warnings": warnings,
        "missing": [],
    })
}

#[cfg(unix)]
fn study_status_fixture(evidence: Option<Value>) -> Value {
    let mut status = json!({
        "manifest": {
            "spec": {
                "kind": "tuning",
                "method": "differential-evolution",
                "base_cases": ["site"],
                "parameters": [{"name": "DEF_TUNING_CNFAC", "sample_min": 0.1, "sample_max": 0.9, "scale": "linear"}],
                "targets": [{"key": "Qle", "variable": "Qle", "from": 0, "to": 10, "validation_from": 10, "validation_to": 20}],
            },
            "members": [{
                "id": "m000005",
                "parameters": {"DEF_TUNING_CNFAC": 0.5},
            }],
        },
        "state": {
            "status": "completed",
            "generation": 1,
            "best_member": "m000005",
            "best_objective": 0.3,
            "candidates": {"m000000": {"calibration": 0.8}},
            "warnings": [],
        },
    });
    if let Some(evidence) = evidence {
        status["evidence"] = evidence;
    }
    status
}

#[cfg(unix)]
fn study_model_context(root: &Path, revision: u64, code: &str) -> crate::tools::ModelContext {
    crate::tools::ModelContext {
        model: json!({"version": {"id": "workspace-x", "revision": revision}, "code": {"source_sha256": code}, "overlay": {}}),
        model_root: root.join("models"),
        source_dir: root.join("source"),
        docs_dir: None,
        source_case: None,
    }
}

#[cfg(unix)]
#[test]
fn study_summary_merges_evidence_parameters_and_current_identity() {
    let root = temp("study-evidence");
    let dims = json!([
        {"key": "DEF_TUNING_CNFAC", "range": [0.1, 0.9], "scale": "linear"},
        {"key": "DEF_TUNING_ZLND", "range": [0.001, 0.1], "scale": "log"},
    ]);
    let best_values = json!([
        {"key": "DEF_TUNING_CNFAC", "value": 0.895, "position": 0.99375, "position_scale": "linear", "at_bound": true, "bound_side": "upper"},
        {"key": "DEF_TUNING_ZLND", "value": 0.001, "position": 0.0, "position_scale": "log", "at_bound": true, "bound_side": "lower"},
    ]);
    let evidence = study_evidence(
        json!({
            "status": "pinned",
            "name": "modified",
            "version": {"id": "workspace-x", "revision": 3},
            "code_source_sha256": "a".repeat(64),
        }),
        dims,
        best_values,
        json!(["bound: DEF_TUNING_ZLND lower 0.001"]),
    );
    let mut status = study_status_fixture(Some(evidence));
    // state.candidates 的 m000000 与 evidence 基线值不同：旧字段必须跟 evidence 走。
    status["state"]["candidates"]["m000000"]["calibration"] = json!(0.123);
    let script = fake_study_cli(&root, &status);
    let matching = ToolContext {
        project_root: root.clone(),
        cli: script.clone(),
        model_context: Some(study_model_context(&root, 3, &"a".repeat(64))),
        ..ToolContext::default()
    };
    let result = StudySummary
        .call(&json!({"study": "study"}), &matching)
        .unwrap();
    // 旧字段全部保留。
    assert_eq!(result["kind"], "tuning");
    assert_eq!(result["method"], "differential-evolution");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["generation"], 1);
    assert_eq!(result["base_cases"], json!(["site"]));
    assert_eq!(result["best_member"], "m000005");
    assert_eq!(result["best_objective"].as_f64(), Some(0.3));
    // baseline_objective 用 evidence 里按 baseline 标记定位的成绩，不是 state.candidates 的 m000000。
    assert_eq!(result["baseline_objective"].as_f64(), Some(0.8));
    assert!(result.get("targets").is_some());
    assert!(result.get("hybrid").is_some());
    assert!(result.get("warnings").is_some());
    // 参数由 evidence 供数：Log 位置和贴边来自记录，不按线性区间重算。
    let positions = result["best_parameters"].as_array().unwrap();
    assert_eq!(positions.len(), 2, "{result}");
    let zlnd = positions
        .iter()
        .find(|row| row["parameter"] == "DEF_TUNING_ZLND")
        .unwrap();
    assert_eq!(zlnd["position"].as_f64(), Some(0.0));
    assert_eq!(zlnd["at_bound"], true);
    assert_eq!(zlnd["position_scale"], "log");
    // 当前选择被补进 evidence，并与研究记录按 id+revision+sha 对比。
    let identity = &result["evidence"]["model_identity"];
    assert_eq!(identity["current_selection"]["status"], "frozen");
    assert_eq!(identity["comparison"]["status"], "matches");
    assert_eq!(identity["comparison"]["matches"], true);
    assert_eq!(
        identity["current_selection"]["version"]["revision"].as_u64(),
        Some(3)
    );

    // 同名但 revision 不同：mismatch，不是 matches。
    let stale = ToolContext {
        model_context: Some(study_model_context(&root, 4, &"a".repeat(64))),
        ..matching.clone()
    };
    let mismatch = StudySummary
        .call(&json!({"study": "study"}), &stale)
        .unwrap();
    assert_eq!(
        mismatch["evidence"]["model_identity"]["comparison"]["status"],
        "mismatch"
    );
    assert_eq!(
        mismatch["evidence"]["model_identity"]["comparison"]["matches"],
        false
    );

    // revision 相同但源码 hash 不同：仍然 mismatch。
    let edited = ToolContext {
        model_context: Some(study_model_context(&root, 3, &"b".repeat(64))),
        ..matching.clone()
    };
    let mismatch = StudySummary
        .call(&json!({"study": "study"}), &edited)
        .unwrap();
    assert_eq!(
        mismatch["evidence"]["model_identity"]["comparison"]["status"],
        "mismatch"
    );

    // 官方版 / 没有冻结模型上下文：身份 unknown 并给原因，不当成匹配。
    let official = ToolContext {
        project_root: root.clone(),
        cli: script.clone(),
        ..ToolContext::default()
    };
    let unknown = StudySummary
        .call(&json!({"study": "study"}), &official)
        .unwrap();
    assert_eq!(
        unknown["evidence"]["model_identity"]["current_selection"]["status"],
        "unknown"
    );
    assert_eq!(
        unknown["evidence"]["model_identity"]["comparison"]["status"],
        "unknown"
    );
    assert!(
        unknown["evidence"]["model_identity"]["comparison"]["reason"]
            .as_str()
            .unwrap()
            .contains("official")
    );

    // evidence 里基线不可用时保持 null，不退回 state.candidates 的 m000000 分数。
    let mut unavailable_baseline = status.clone();
    unavailable_baseline["evidence"]["comparison"]["baseline"]["calibration"]["status"] =
        json!("unavailable");
    unavailable_baseline["evidence"]["comparison"]["baseline"]["calibration"]["score"] =
        Value::Null;
    std::fs::write(
        root.join("study-status.json"),
        unavailable_baseline.to_string(),
    )
    .unwrap();
    let unavailable = StudySummary
        .call(&json!({"study": "study"}), &matching)
        .unwrap();
    assert!(unavailable["baseline_objective"].is_null(), "{unavailable}");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn study_summary_missing_evidence_falls_back_and_stays_within_the_transport_budget() {
    let root = temp("study-evidence-budget");
    // 旧 CLI：没有 evidence，best_parameters 退回 manifest.members 快照（线性位置）。
    let status = study_status_fixture(None);
    let script = fake_study_cli(&root, &status);
    let ctx = ToolContext {
        project_root: root.clone(),
        cli: script.clone(),
        ..ToolContext::default()
    };
    let fallback = StudySummary.call(&json!({"study": "study"}), &ctx).unwrap();
    assert_eq!(fallback["evidence"]["status"], "missing");
    assert!(fallback["evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("no evidence"));
    let positions = fallback["best_parameters"].as_array().unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0]["parameter"], "DEF_TUNING_CNFAC");
    assert_eq!(positions[0]["position"].as_f64(), Some(0.5));
    // 旧 CLI 没有 evidence：baseline_objective 仍走旧 candidates fallback。
    assert_eq!(fallback["baseline_objective"].as_f64(), Some(0.8));

    // 大证据：输出必须仍是 ≤24k 个字符的合法 JSON。
    let dims = Value::Array(
        (0..40)
            .map(|index| {
                json!({"key": format!("DEF_TUNING_PARAMETER_{index:02}"), "range": [0.0, 1.0], "scale": "linear"})
            })
            .collect(),
    );
    let best_values = Value::Array(
        (0..40)
            .map(|index| {
                json!({"key": format!("DEF_TUNING_PARAMETER_{index:02}"), "value": 0.5, "position": 0.5, "position_scale": "linear", "at_bound": false})
            })
            .collect(),
    );
    let warnings = Value::Array(
        (0..20)
            .map(|index| json!(format!("warning {index}: {}", "x".repeat(5000))))
            .collect(),
    );
    let mut evidence = study_evidence(
        json!({"status": "official", "reason": "no frozen model"}),
        dims,
        best_values,
        warnings,
    );
    evidence["objective"]["targets"] = Value::Array(
        (0..12)
            .map(|index| json!({"key": format!("target_{index:02}"), "variable": "Qle"}))
            .collect(),
    );
    std::fs::write(
        root.join("study-status.json"),
        study_status_fixture(Some(evidence)).to_string(),
    )
    .unwrap();
    let large = StudySummary.call(&json!({"study": "study"}), &ctx).unwrap();
    assert_eq!(large["truncated"], true);
    let serialized = super::super::result_text(&large);
    assert!(serialized.chars().count() <= super::super::MAX_RESULT_CHARS);
    assert!(serde_json::from_str::<Value>(&serialized).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

// ---- 可选 Xerj 召回：search_docs 集成（假 HTTP 服务；真实 Xerj 验收在 tests/xerj_acceptance.rs） ----

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum FakeXerjMode {
    Normal,
    UnknownIds,
    Oversize,
    SearchFailure,
}

struct FakeXerjState {
    docs: HashMap<String, Value>,
    mode: FakeXerjMode,
    requests: usize,
}

struct FakeXerj {
    endpoint: String,
    state: Arc<Mutex<FakeXerjState>>,
}

impl FakeXerj {
    fn set_mode(&self, mode: FakeXerjMode) {
        self.state.lock().unwrap().mode = mode;
    }
    fn requests(&self) -> usize {
        self.state.lock().unwrap().requests
    }
}

fn fake_bytes_contain(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 从 fake 收到的 ES 查询里取文本条件：生产候选查询是 must.bool.should 的两个 match，
/// 发布前验证查询则没有文本条件（等价 match_all）。
fn fake_match_text(item: &Value) -> Option<&str> {
    item["match"]["body"]
        .as_str()
        .or_else(|| item["match"]["title"].as_str())
}

fn fake_http_respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn fake_xerj_spawn() -> FakeXerj {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(FakeXerjState {
        docs: HashMap::new(),
        mode: FakeXerjMode::Normal,
        requests: 0,
    }));
    let shared = Arc::clone(&state);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let _ = fake_xerj_handle(stream, shared);
            });
        }
    });
    FakeXerj { endpoint, state }
}

fn fake_xerj_handle(
    mut stream: TcpStream,
    state: Arc<Mutex<FakeXerjState>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if let Some(position) = fake_bytes_contain(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 16 * 1024 * 1024 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.lines();
    let request = lines.next().unwrap_or_default();
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let length = lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let end = (header_end + length).min(buffer.len());
    let body = String::from_utf8_lossy(&buffer[header_end..end]).into_owned();
    state.lock().unwrap().requests += 1;
    fake_xerj_route(&mut stream, &method, &path, &body, &state)
}

fn fake_xerj_route(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    body: &str,
    state: &Arc<Mutex<FakeXerjState>>,
) -> std::io::Result<()> {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if method == "GET" && path == "/_cluster/health" {
        fake_http_respond(
            stream,
            "200 OK",
            r#"{"cluster_name":"fake","status":"green","number_of_nodes":1}"#,
        );
        return Ok(());
    }
    if method == "PUT" && segments.len() == 1 {
        fake_http_respond(stream, "200 OK", r#"{"acknowledged":true}"#);
        return Ok(());
    }
    if method == "DELETE" && segments.len() == 1 {
        state.lock().unwrap().docs.clear();
        fake_http_respond(stream, "200 OK", r#"{"acknowledged":true}"#);
        return Ok(());
    }
    if method == "DELETE" && segments.len() == 3 && segments[1] == "_doc" {
        let removed = state.lock().unwrap().docs.remove(segments[2]).is_some();
        fake_http_respond(
            stream,
            "200 OK",
            &format!(
                r#"{{"result":"{}"}}"#,
                if removed { "deleted" } else { "not_found" }
            ),
        );
        return Ok(());
    }
    if method == "POST" && segments.last() == Some(&"_bulk") {
        let mut locked = state.lock().unwrap();
        let index = segments.first().copied().unwrap_or("docs");
        let mut items = Vec::new();
        let mut errors = false;
        let lines: Vec<&str> = body
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        for pair in lines.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            let Ok(action) = serde_json::from_str::<Value>(pair[0]) else {
                continue;
            };
            let Some(id) = action["index"]["_id"].as_str().map(str::to_owned) else {
                continue;
            };
            let Ok(doc) = serde_json::from_str::<Value>(pair[1]) else {
                continue;
            };
            if doc["body"]
                .as_str()
                .is_some_and(|body| body.contains("__fail"))
            {
                errors = true;
                items.push(json!({"index":{"_index":index,"_id":id,"status":400,
                    "error":{"type":"document_parsing_exception","reason":"fake partial failure"}}}));
            } else {
                locked.docs.insert(id.clone(), doc);
                items.push(json!({"index":{"_index":index,"_id":id,"status":201}}));
            }
        }
        drop(locked);
        fake_http_respond(
            stream,
            "200 OK",
            &json!({"took":1,"errors":errors,"items":items}).to_string(),
        );
        return Ok(());
    }
    if method == "POST" && segments.last() == Some(&"_search") {
        let request: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let (mode, docs) = {
            let locked = state.lock().unwrap();
            (locked.mode, locked.docs.clone())
        };
        match mode {
            FakeXerjMode::SearchFailure => fake_http_respond(
                stream,
                "500 Internal Server Error",
                r#"{"error":{"type":"internal_server_error","reason":"fake search failure"}}"#,
            ),
            FakeXerjMode::Oversize => {
                let body = format!(
                    r#"{{"hits":{{"total":{{"value":1,"relation":"eq"}},"max_score":1.0,"hits":[{{"_id":"big","_score":1.0,"_source":{{"body":"{}"}}}}]}}}}"#,
                    "x".repeat(3_000_000)
                );
                fake_http_respond(stream, "200 OK", &body);
            }
            FakeXerjMode::UnknownIds => fake_http_respond(
                stream,
                "200 OK",
                &json!({
                    "took":1,
                    "hits":{"total":{"value":1,"relation":"eq"},"max_score":1.0,
                        "hits":[{"_index":"fake","_id":"unknown-chunk-xyz","_score":1.0,
                            "_source":{"relative_path":"../../forged.md",
                                "document_sha256":"0".repeat(64),"line_start":1,"line_end":999999}}]}
                })
                .to_string(),
            ),
            FakeXerjMode::Normal => {
                let filters = request["query"]["bool"]["filter"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let mut scope_key = None;
                let mut generation = None;
                for filter in &filters {
                    if let Some(value) = filter["term"]["scope_key"].as_str() {
                        scope_key = Some(value.to_owned());
                    }
                    if let Some(value) = filter["term"]["generation"].as_u64() {
                        generation = Some(value);
                    }
                }
                // 生产查询是 must:[{bool:{should:[match body, match title]}}]；验证查询没有 must。
                // 两种都支持，否则要么中文候选取不到，要么发布前的 match_all 验证永远返回 0。
                let query_text = request["query"]["bool"]["must"]
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| {
                        fake_match_text(item).or_else(|| {
                            item["bool"]["should"].as_array().and_then(|should| {
                                should.iter().find_map(|clause| fake_match_text(clause))
                            })
                        })
                    })
                    .unwrap_or_default()
                    .to_lowercase();
                // 发布前的验证查询只有 scope/generation filter、没有文本查询（相当于 match_all）：
                // 真实 Xerj 会返回该代全部文档，fake 也必须这样，否则索引永远发布不出去。
                let match_all = query_text.trim().is_empty();
                let tokens = query_text.split_whitespace().collect::<Vec<_>>();
                let size = request["size"].as_u64().unwrap_or(5).min(40) as usize;
                let mut scored = docs
                    .into_iter()
                    .filter_map(|(id, doc)| {
                        if scope_key.as_deref() != doc["scope_key"].as_str() {
                            return None;
                        }
                        if generation != doc["generation"].as_u64() {
                            return None;
                        }
                        if match_all {
                            return Some((1.0, id));
                        }
                        let haystack = format!(
                            "{} {}",
                            doc["title"].as_str().unwrap_or_default(),
                            doc["body"].as_str().unwrap_or_default()
                        )
                        .to_lowercase();
                        let score = tokens
                            .iter()
                            .filter(|token| !token.is_empty() && haystack.contains(**token))
                            .count() as f64;
                        (score > 0.0).then_some((score, id))
                    })
                    .collect::<Vec<_>>();
                scored.sort_by(|left, right| {
                    right
                        .0
                        .partial_cmp(&left.0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| left.1.cmp(&right.1))
                });
                let hits: Vec<Value> = scored
                    .into_iter()
                    .take(size)
                    .map(|(score, id)| {
                        json!({"_index":"fake","_id":id,"_score":score,
                            // 故意伪造：应用必须只信本地清单，不信 _source。
                            "_source":{"relative_path":"../../forged.md",
                                "document_sha256":"0".repeat(64),"line_start":1,"line_end":999999}})
                    })
                    .collect();
                fake_http_respond(
                    stream,
                    "200 OK",
                    &json!({"took":1,"timed_out":false,
                        "hits":{"total":{"value":hits.len(),"relation":"eq"},"max_score":1.0,"hits":hits}})
                    .to_string(),
                );
            }
        }
        return Ok(());
    }
    fake_http_respond(stream, "404 Not Found", r#"{"error":"no route"}"#);
    Ok(())
}

fn wait_xerj_ready(handle: &crate::retrieval::Handle, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let status = handle.status();
        if matches!(
            status["state"].as_str(),
            Some("ready" | "error" | "invalid")
        ) {
            return status;
        }
        if Instant::now() >= deadline {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn xerj_fake_cli(root: &Path, source: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join("fake-xerj-cli");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1\" = ws-source-info ] || exit 9\nprintf '%s\\n' '{{\"source\":\"{}\",\"revision\":\"abc123\",\"tag\":null,\"dirty\":false,\"content_sha256\":\"{}\",\"version_status\":\"identified\"}}'\n",
            source.display(),
            "a".repeat(64)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    script
}

fn xerj_context(
    label: &str,
    enabled: bool,
    endpoint: String,
    docs_source: Option<&Path>,
) -> (PathBuf, ToolContext) {
    let root = temp(label);
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(
        docs.join("a.md"),
        "# A\n\nenergy closure is checked in this document.\nsecond line\n",
    )
    .unwrap();
    std::fs::write(
        docs.join("b.md"),
        "# B\n\nclosure appears without the full phrase here.\nupdatedtoken line\n",
    )
    .unwrap();
    let source = docs_source
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.clone());
    let cli = xerj_fake_cli(&root, &source);
    let handle = crate::retrieval::Handle::new(
        crate::retrieval::RetrievalSettings {
            enabled,
            endpoint,
            index: "colm-docs-integration".into(),
            timeout_ms: 800,
        },
        String::new(),
        None,
    );
    let ctx = ToolContext {
        project_root: root.clone(),
        cli,
        docs_root: Some(docs),
        retrieval: Some(handle),
        ..ToolContext::default()
    };
    (root, ctx)
}

#[cfg(unix)]
#[test]
fn search_docs_merges_xerj_chunks_and_ignores_forged_index_metadata() {
    let fake = fake_xerj_spawn();
    let (root, ctx) = xerj_context("xerj-merge", true, fake.endpoint.clone(), None);
    let handle = ctx.retrieval.as_ref().unwrap().clone();
    let request = json!({"query":"energy closure","name":null});
    let _ = super::SearchDocs.call(&request, &ctx).unwrap();
    let status = wait_xerj_ready(&handle, Duration::from_secs(10));
    assert_eq!(status["state"], "ready", "{status}");
    let result = super::SearchDocs.call(&request, &ctx).unwrap();
    assert_eq!(result["retrieval"]["backend"], "xerj");
    assert!(
        result["retrieval"]["candidates"].as_u64().unwrap_or(0) >= 1,
        "{result}"
    );
    let matches = result["matches"].as_array().unwrap();
    assert!(matches
        .iter()
        .any(|hit| hit["match"] == "exact" && hit["file"] == "a.md"));
    let candidate = matches
        .iter()
        .find(|hit| hit["match"] == "candidate")
        .expect("a BM25-only chunk from b.md must be returned");
    assert_eq!(candidate["file"], "b.md");
    // 索引里伪造成 ../../forged.md 和假 hash/行段：输出必须来自本地清单与真实重读。
    assert!(!candidate["path"]
        .as_str()
        .unwrap_or_default()
        .contains("forged"));
    assert!(Path::new(candidate["path"].as_str().unwrap()).starts_with(root.join("docs")));
    assert!(candidate["line"].as_u64().unwrap_or(0) >= 1);
    assert!(candidate["knowledge_status"].is_string());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn search_docs_falls_back_with_a_reason_when_the_backend_is_down() {
    let (root, ctx) = xerj_context("xerj-down", true, "http://127.0.0.1:1".into(), None);
    let handle = ctx.retrieval.as_ref().unwrap().clone();
    let request = json!({"query":"energy closure","name":null});
    let first = super::SearchDocs.call(&request, &ctx).unwrap();
    assert_eq!(first["retrieval"]["backend"], "keyword");
    assert!(!first["matches"].as_array().unwrap().is_empty());
    let status = wait_xerj_ready(&handle, Duration::from_secs(5));
    assert_ne!(status["state"], "ready");
    let result = super::SearchDocs.call(&request, &ctx).unwrap();
    assert_eq!(result["retrieval"]["backend"], "keyword");
    assert!(result["documentation_status"].is_null());
    assert!(
        result["retrieval"]["fallback_reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "{result}"
    );
    assert!(result["retrieval"]["state"] != "ready");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn search_docs_rejects_unknown_oversized_and_failed_candidates_without_faking_evidence() {
    let fake = fake_xerj_spawn();
    let (root, ctx) = xerj_context("xerj-bad", true, fake.endpoint.clone(), None);
    let handle = ctx.retrieval.as_ref().unwrap().clone();
    let request = json!({"query":"energy closure","name":null});
    let _ = super::SearchDocs.call(&request, &ctx).unwrap();
    assert_eq!(
        wait_xerj_ready(&handle, Duration::from_secs(10))["state"],
        "ready"
    );
    for mode in [
        FakeXerjMode::UnknownIds,
        FakeXerjMode::Oversize,
        FakeXerjMode::SearchFailure,
    ] {
        fake.set_mode(mode);
        let result = super::SearchDocs.call(&request, &ctx).unwrap();
        assert_eq!(result["retrieval"]["backend"], "keyword");
        assert!(
            result["matches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|hit| hit["match"] != "candidate"),
            "untrusted index responses must not enter the model context"
        );
        assert!(
            result["retrieval"]["fallback_reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()),
            "{result}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn search_docs_reenumerates_after_a_document_change_and_drops_old_chunks() {
    let fake = fake_xerj_spawn();
    let (root, ctx) = xerj_context("xerj-change", true, fake.endpoint.clone(), None);
    let handle = ctx.retrieval.as_ref().unwrap().clone();
    let _ = super::SearchDocs
        .call(&json!({"query":"energy closure","name":null}), &ctx)
        .unwrap();
    assert_eq!(
        wait_xerj_ready(&handle, Duration::from_secs(10))["state"],
        "ready"
    );
    // 文件内容改变（长度也变）：旧清单与旧行段不能继续当证据。
    std::fs::write(
        root.join("docs/b.md"),
        "# B\n\nthe old phrase is gone; only the updatedtoken remains.\n",
    )
    .unwrap();
    let changed = super::SearchDocs
        .call(&json!({"query":"energy closure","name":null}), &ctx)
        .unwrap();
    assert_eq!(changed["retrieval"]["backend"], "keyword");
    assert!(
        changed["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit["match"] != "candidate"),
        "{changed}"
    );
    assert!(
        changed["retrieval"]["fallback_reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "{changed}"
    );
    let status = wait_xerj_ready(&handle, Duration::from_secs(10));
    assert_eq!(status["state"], "ready", "{status}");
    let refreshed = super::SearchDocs
        .call(&json!({"query":"updatedtoken","name":null}), &ctx)
        .unwrap();
    assert_eq!(refreshed["retrieval"]["backend"], "xerj");
    // 新词本身就在原文同一行，关键词精确路径已经命中，因此这里不要求 candidate 类型；
    // 但要证明重建后的索引确实返回了该块（candidates>0），且结果来自新原文。
    assert!(
        refreshed["retrieval"]["candidates"].as_u64().unwrap_or(0) >= 1,
        "{refreshed}"
    );
    let updated = refreshed["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hit| hit["file"] == "b.md")
        .expect("the rebuilt index must return the updated document");
    assert!(
        updated["text"]
            .as_str()
            .unwrap_or_default()
            .contains("updatedtoken"),
        "{updated}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn disabled_retrieval_never_contacts_the_local_backend() {
    let fake = fake_xerj_spawn();
    let (root, ctx) = xerj_context("xerj-disabled", false, fake.endpoint.clone(), None);
    let result = super::SearchDocs
        .call(&json!({"query":"energy closure","name":null}), &ctx)
        .unwrap();
    assert_eq!(result["retrieval"]["backend"], "keyword");
    assert_eq!(result["retrieval"]["enabled"], false);
    assert_eq!(result["retrieval"]["state"], "disabled");
    assert!(!result["matches"].as_array().unwrap().is_empty());
    assert_eq!(fake.requests(), 0, "disabled retrieval must not touch HTTP");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn missing_workspace_docs_do_not_fall_back_to_app_documents_even_with_retrieval_on() {
    let fake = fake_xerj_spawn();
    let root = temp("xerj-frozen-missing");
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("app.md"), "# App\n\nenergy closure in app docs\n").unwrap();
    let workspace_source = root.join("workspace-src");
    std::fs::create_dir_all(&workspace_source).unwrap();
    let cli = xerj_fake_cli(&root, &workspace_source);
    let handle = crate::retrieval::Handle::new(
        crate::retrieval::RetrievalSettings {
            enabled: true,
            endpoint: fake.endpoint.clone(),
            index: "colm-docs-integration".into(),
            timeout_ms: 800,
        },
        String::new(),
        None,
    );
    let ctx = ToolContext {
        project_root: root.clone(),
        cli,
        docs_root: Some(docs),
        retrieval: Some(handle),
        ..ToolContext::default()
    };
    let result = super::SearchDocs
        .call(&json!({"query":"energy closure","name":"ws"}), &ctx)
        .unwrap();
    assert_eq!(result["documentation_status"], "missing");
    assert!(result["matches"].as_array().unwrap().is_empty());
    assert!(
        result["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("Current application documents are not evidence")),
        "{result}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
