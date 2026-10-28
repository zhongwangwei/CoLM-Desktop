use super::*;

fn args(workflow: &str) -> Value {
    json!({"workflow":workflow,"case":"case","variable":null,"study":null,"workspace":null})
}

fn fixture(label: &str) -> ToolContext {
    let root =
        std::env::temp_dir().join(format!("colm-diagnostics-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("case")).unwrap();
    std::fs::write(
        root.join("case/case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'test'\n/\n",
    )
    .unwrap();
    ToolContext {
        project_root: root,
        cli: "/missing/colm-cli".into(),
        ..ToolContext::default()
    }
}

fn step<'a>(result: &'a Value, id: &str) -> &'a Value {
    result["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()
}

#[test]
fn five_recipes_keep_required_phases_and_report_missing_evidence() {
    let ctx = fixture("recipes");
    for workflow in ["startup", "closure", "flux", "calibration", "parity"] {
        let result = DiagnosticPlan.call(&args(workflow), &ctx).unwrap();
        assert_eq!(result["version"], 1);
        assert_eq!(result["inputs"]["case"], "case");
        assert_eq!(
            result["phases"],
            json!(["checkdata", "localise", "validate", "report"])
        );
        assert_eq!(result["status"], "incomplete");
        assert_eq!(step(&result, "run_status")["status"], "missing");
        assert_eq!(step(&result, "case_config")["status"], "collected");
        assert_eq!(step(&result, "minimal_validation")["status"], "pending");
        assert_eq!(step(&result, "report")["status"], "pending");
        for phase in ["checkdata", "localise", "validate", "report"] {
            assert!(
                result["steps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["phase"] == phase),
                "{workflow}: {phase}"
            );
        }
        for s in result["steps"].as_array().unwrap() {
            for key in ["id", "title", "status", "evidence", "missing", "next_tool"] {
                assert!(s.get(key).is_some());
            }
        }
        if workflow == "closure" {
            assert_eq!(step(&result, "series")["status"], "missing");
            assert_eq!(
                step(&result, "series")["missing"].as_array().unwrap().len(),
                3
            );
        }
        if workflow == "flux" {
            assert_eq!(step(&result, "observation_alignment")["status"], "pending");
            assert!(step(&result, "regimes")["missing"]
                .to_string()
                .contains("day/night"));
            assert_eq!(result["hypotheses"][0]["status"], "unverified");
        }
    }
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[cfg(unix)]
fn fake_cli(ctx: &mut ToolContext, series: &str) {
    use std::os::unix::fs::PermissionsExt;
    let script = ctx.project_root.join("fake-cli");
    std::fs::write(&script,format!("#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'/calls\ncase \"$1\" in\nseries) cat <<'JSON'\n{series}\nJSON\n;;\nstudy-status) echo '{{\"manifest\":{{\"spec\":{{}}}},\"state\":{{}}}}' ;;\nws-status) echo '{{\"head\":\"abc\",\"gates\":{{}}}}' ;;\n*) exit 9 ;;\nesac\n",ctx.project_root.display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    ctx.cli = script;
}

#[test]
#[cfg(unix)]
fn fake_cli_collects_read_only_evidence_without_claiming_diagnosis() {
    let mut ctx = fixture("cli");
    fake_cli(
        &mut ctx,
        r#"{"time":[0,3600],"vars":{"f_xerr":[0,0],"f_zerr":[0,1],"f_assim":[1,2]}}"#,
    );
    for workflow in ["startup", "closure", "flux", "calibration", "parity"] {
        let mut request = args(workflow);
        request["variable"] = json!("f_assim");
        request["study"] = json!("study");
        request["workspace"] = json!("ws");
        let result = DiagnosticPlan.call(&request, &ctx).unwrap();
        assert_eq!(result["status"], "incomplete");
        if ["closure", "flux"].contains(&workflow) {
            assert_eq!(step(&result, "series")["status"], "collected");
        }
        if workflow == "calibration" {
            assert_eq!(step(&result, "study")["status"], "missing");
        }
        if workflow == "parity" {
            assert_eq!(step(&result, "build_gate")["status"], "collected");
        }
    }
    let calls = std::fs::read_to_string(ctx.project_root.join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        vec!["series", "series", "study-status", "ws-status"]
    );
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn closure_requires_both_finite_diagnostics_and_preserves_cli_errors() {
    let mut ctx = fixture("closure");
    for series in [
        r#"{"time":[0],"vars":{}}"#,
        r#"{"time":[0],"vars":{"f_xerr":[0],"f_zerr":[null]}}"#,
        r#"{"time":[0],"vars":{"f_xerr":[0],"f_zerr":[0,1]}}"#,
    ] {
        fake_cli(&mut ctx, series);
        let result = DiagnosticPlan.call(&args("closure"), &ctx).unwrap();
        assert_eq!(step(&result, "series")["status"], "missing");
        assert!(step(&result, "series")["missing"]
            .to_string()
            .contains("f_zerr"));
    }
    ctx.cli = "/does/not/exist".into();
    let result = DiagnosticPlan.call(&args("parity"), &ctx).unwrap();
    assert_eq!(step(&result, "build_gate")["status"], "pending");
    let mut request = args("calibration");
    request["study"] = json!("study");
    let result = DiagnosticPlan.call(&request, &ctx).unwrap();
    assert_eq!(step(&result, "study")["status"], "missing");
    assert!(step(&result, "study")["missing"]
        .to_string()
        .contains("cannot start"));
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
fn oversized_evidence_stays_valid_json_and_arguments_are_bounded() {
    let ctx = fixture("bounds");
    std::fs::write(
        ctx.project_root.join("case/run.out"),
        format!("error {}\n", "x".repeat(500)).repeat(40),
    )
    .unwrap();
    let result = DiagnosticPlan.call(&args("startup"), &ctx).unwrap();
    let serialized = super::super::result_text(&result);
    assert!(serialized.chars().count() < super::super::MAX_RESULT_CHARS);
    assert!(serde_json::from_str::<Value>(&serialized).is_ok());
    assert_eq!(step(&result, "run_status")["evidence"]["truncated"], true);
    let mut request = args("unknown");
    assert!(DiagnosticPlan.call(&request, &ctx).is_err());
    request = args("flux");
    request["variable"] = json!(123);
    assert!(DiagnosticPlan.call(&request, &ctx).is_err());
    request = args("startup");
    request["case"] = json!("x".repeat(1025));
    assert!(DiagnosticPlan.call(&request, &ctx).is_err());
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn flux_evidence_updates_steps_but_experiment_stays_unverified() {
    use std::os::unix::fs::PermissionsExt;
    let mut ctx = fixture("flux-steps");
    let script = ctx.project_root.join("fake-cli");
    std::fs::write(&script,r#"#!/bin/sh
if [ "$1" != 'flux-diagnose' ]; then exit 5; fi
cat <<'JSON'
{"alignment":{"status":"computed"},"metrics":{"status":"computed","n":12},"regimes":{"calendar_month":{"status":"computed"},"daylight":{"status":"computed"},"wetness":{"status":"missing"}},"hypotheses":[],"evidence_id":"proof","experiment":{"status":"unverified","cause_verified":false}}
JSON
"#).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    ctx.cli = script;
    let mut request = args("flux");
    request["variable"] = json!("f_lfevpa");
    request["flux"] = json!({"obs":"obs.nc"});
    let result = DiagnosticPlan.call(&request, &ctx).unwrap();
    assert_eq!(step(&result, "series")["status"], "collected");
    assert_eq!(
        step(&result, "observation_alignment")["status"],
        "collected"
    );
    assert_eq!(step(&result, "regimes")["status"], "partial");
    assert_eq!(step(&result, "minimal_validation")["status"], "unverified");
    assert_eq!(result["status"], "incomplete");
    let script_text = std::fs::read_to_string(&ctx.cli)
        .unwrap()
        .replace(
            "\"calendar_month\":{\"status\":\"computed\"}",
            "\"calendar_month\":{\"status\":\"partial\",\"missing\":[\"month_02\"]}",
        )
        .replace(
            "\"wetness\":{\"status\":\"missing\"}",
            "\"wetness\":{\"status\":\"computed\"}",
        );
    std::fs::write(&ctx.cli, script_text).unwrap();
    let partial_month = DiagnosticPlan.call(&request, &ctx).unwrap();
    assert_eq!(step(&partial_month, "regimes")["status"], "partial");
    assert!(!step(&partial_month, "regimes")["missing"]
        .as_array()
        .unwrap()
        .is_empty());
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn annual_flux_and_experiment_evidence_remain_parseable_under_transport_budget() {
    use std::os::unix::fs::PermissionsExt;
    let mut ctx = fixture("flux-budget");
    let metric = json!({"status":"computed","n":1400,"bias":0.12345678912345678,"rmse":9.987654321098765,"mae":1.34567890123456,"correlation":0.8712345678901234,"model_mean":117.123456789,"obs_mean":116.987654321});
    let groups: serde_json::Map<String, Value> = (1..=12)
        .map(|month| (format!("month_{month:02}"), metric.clone()))
        .collect();
    let regimes = json!({"calendar_month":{"status":"computed","groups":groups},"daylight":{"status":"computed","day":metric,"night":metric},"wetness":{"status":"computed","wet":metric,"dry":metric}});
    let mut data = json!({"alignment":{"status":"computed","paired":17520},"metrics":metric,"regimes":regimes,"hypotheses":[],"evidence_id":"proof","minimal_experiment":{"status":"proposed_not_executed","cause_verified":false},"experiment":{"status":"unverified","cause_verified":false,"regimes":regimes,"metrics":metric,"missing":["Fresh output identity is unverified."]}});
    let script = ctx.project_root.join("fake-cli");
    let mut request = args("flux");
    request["variable"] = json!("f_lfevpa");
    request["flux"] = json!({"obs":"obs.nc"});
    for scenario in 0..3 {
        let oversized = scenario > 0;
        if oversized {
            data["experiment"]["configuration_differences"] = json!("x".repeat(40000));
        }
        if scenario == 2 {
            data.as_object_mut().unwrap().remove("experiment");
            data["regimes"]["large_detail"] = json!("x".repeat(40000));
        }
        std::fs::write(&script, format!("#!/bin/sh\ncat <<'JSON'\n{data}\nJSON\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        ctx.cli = script.clone();
        let value = DiagnosticPlan.call(&request, &ctx).unwrap();
        let transported = super::super::result_text(&value);
        assert!(transported.chars().count() < 24000);
        let decoded: Value = serde_json::from_str(&transported).unwrap();
        let proof_key = if scenario == 2 {
            "minimal_experiment"
        } else {
            "experiment"
        };
        if scenario == 2 {
            assert!(decoded["flux_evidence"].get("experiment").is_none());
        }
        assert_eq!(decoded["flux_evidence"][proof_key]["cause_verified"], false);
        assert!(!decoded["flux_evidence"][proof_key]["missing"]
            .as_array()
            .unwrap()
            .is_empty());
        if !oversized {
            assert_eq!(
                decoded["flux_evidence"]["regimes"]["calendar_month"]["groups"]
                    .as_object()
                    .unwrap()
                    .len(),
                12
            );
        } else {
            assert_eq!(decoded["flux_evidence"]["status"], "abbreviated");
        }
        assert_eq!(
            step(&decoded, "regimes")["evidence"]["ref"],
            "flux_evidence.regimes"
        );
    }
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

// ---- P1-A：calibration 工作流真实消费 Study evidence ----

#[cfg(unix)]
fn fake_study_cli(ctx: &mut ToolContext, status: Value) {
    use std::os::unix::fs::PermissionsExt;
    let status_path = ctx.project_root.join("study-status.json");
    std::fs::write(&status_path, status.to_string()).unwrap();
    let script = ctx.project_root.join("fake-study-cli");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1\" = study-status ] || exit 9\ncat '{}'\n",
            status_path.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    ctx.cli = script;
}

// 这些夹具只被 cfg(unix) 的假 CLI 测试使用；Windows 上不编译那些测试，
// 不挂 cfg 会让 -D warnings 把帮助函数当 dead_code。
#[cfg(unix)]
fn calibration_study_status(evidence: Value) -> Value {
    json!({
        "manifest": {
            "spec": {
                "kind": "tuning",
                "method": "differential-evolution",
                "base_cases": ["case"],
                "parameters": [{"name": "DEF_TUNING_CNFAC", "sample_min": 0.1, "sample_max": 0.9}],
                "targets": [{"key": "Qle", "variable": "Qle", "from": 0, "to": 10, "validation_from": 10, "validation_to": 20}],
            },
            "members": [],
        },
        "state": {
            "status": "completed",
            "generation": 1,
            "best_member": "m000001",
            "best_objective": 0.3,
            "candidates": {"m000000": {"calibration": 0.8}},
            "warnings": [],
        },
        "evidence": evidence,
    })
}

fn calibration_validation(status: &str, configured: bool, reasons: Value) -> Value {
    json!({
        "configured": configured,
        "status": status,
        "windows": if configured { json!([{"target": "Qle", "from": 10, "to": 20}]) } else { json!([]) },
        "reasons": reasons,
    })
}

fn calibration_evidence(
    kind: &str,
    calibration_relation: &str,
    validation_relation: &str,
    validation: Value,
) -> Value {
    json!({
        "schema": 1,
        "study": {"kind": kind, "method": "differential-evolution", "status": "completed", "generation": 1},
        "execution": {"state": "completed", "tasks": {"total": 2}, "partial_failures": false, "failed_tasks": []},
        "selection": {
            "best_member": "m000001",
            "best_objective": 0.3,
            "provisional": false,
            "no_best_reason": null,
        },
        "objective": {"direction": "minimize", "loss": "weighted mean of per-target losses", "targets": [{"key": "Qle"}]},
        "comparison": {
            "basis": "same period only",
            "baseline": {"member": "m000000",
                "calibration": {"status": "recorded", "score": 0.8, "targets": [{"key": "Qle", "loss": 0.8}]},
                "validation": {"status": "recorded", "score": 0.6, "targets": [{"key": "Qle", "loss": 0.6}]}},
            "best": {"member": "m000001",
                "calibration": {"status": "recorded", "score": 0.3, "targets": [{"key": "Qle", "loss": 0.3}]},
                "validation": {"status": "recorded", "score": 0.9, "targets": [{"key": "Qle", "loss": 0.9}]}},
            "relation": {"calibration": calibration_relation, "validation": validation_relation},
        },
        "validation": validation,
        "parameters": {
            "status": "recorded",
            "dimension_count": 1,
            "hybrid_weight_count": 0,
            "dims": [{"key": "DEF_TUNING_CNFAC", "range": [0.1, 0.9], "scale": "linear"}],
            "baseline": {"status": "recorded", "values": []},
            "best": {"status": "recorded", "values": [
                {"key": "DEF_TUNING_CNFAC", "value": 0.895, "position": 0.99, "position_scale": "linear", "at_bound": true}
            ]},
        },
        "model_identity": {
            "study_record": {"status": "official", "reason": "official version"},
            "current_selection": null,
            "comparison": {"status": "unknown", "matches": null, "basis": "no selection"},
        },
        "verification": {"recorded_result_files": 2, "reverified": false},
        "remote": {"status": "none"},
        "source_paths": {"root": "/study"},
        "warnings": [],
        "missing": [],
    })
}

// 这些夹具只被 cfg(unix) 的假 CLI 测试使用；Windows 上不编译那些测试，
// 不挂 cfg 会让 -D warnings 把帮助函数当 dead_code。
#[cfg(unix)]
fn calibration_request() -> Value {
    let mut request = args("calibration");
    request["study"] = json!("study");
    request
}

#[test]
#[cfg(unix)]
fn calibration_consumes_dual_period_evidence_and_reports_divergence() {
    let mut ctx = fixture("calibration-dual");
    fake_study_cli(
        &mut ctx,
        calibration_study_status(calibration_evidence(
            "tuning",
            "lower_loss",
            "higher_loss",
            calibration_validation("available", true, json!([])),
        )),
    );
    let result = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    assert_eq!(result["status"], "incomplete");
    let study = step(&result, "study");
    assert_eq!(study["status"], "collected");
    assert_eq!(
        study["evidence"]["calibration"]["baseline_score"].as_f64(),
        Some(0.8)
    );
    assert_eq!(
        study["evidence"]["calibration"]["best_score"].as_f64(),
        Some(0.3)
    );
    assert_eq!(study["evidence"]["validation"]["status"], "available");
    assert_eq!(study["evidence"]["selection"]["best_member"], "m000001");
    assert_eq!(
        study["evidence"]["parameters"]["best_at_bound"],
        json!(["DEF_TUNING_CNFAC"])
    );
    // 有分数不等于通过：率定改善 + 验证变差必须作为显式分歧。
    let dual = step(&result, "dual_period");
    assert_eq!(dual["status"], "divergent");
    assert_eq!(dual["evidence"]["validation"]["relation"], "higher_loss");
    assert!(dual["missing"]
        .to_string()
        .contains("do not claim improved generalisation"));
    assert!(result["hypotheses"].to_string().contains("Overfitting"));
    assert!(result["hypotheses"]
        .as_array()
        .unwrap()
        .iter()
        .all(|hypothesis| hypothesis["status"] == "unverified"));
    assert!(!result["confirmed_facts"]
        .to_string()
        .contains("confirmed cause"));
    // 原有的未完成检查保留。
    for id in ["bounds", "held_out", "minimal_validation", "report"] {
        assert!(result["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == id));
    }
    assert_eq!(step(&result, "minimal_validation")["status"], "pending");
    assert_eq!(step(&result, "report")["status"], "pending");
    assert_eq!(step(&result, "held_out")["status"], "partial");
    assert!(step(&result, "held_out")["missing"]
        .to_string()
        .contains("physical closure"));
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn calibration_keeps_validation_unconfigured_and_unavailable_distinct() {
    let mut ctx = fixture("calibration-validation");
    fake_study_cli(
        &mut ctx,
        calibration_study_status(calibration_evidence(
            "tuning",
            "lower_loss",
            "unknown",
            calibration_validation(
                "not_configured",
                false,
                json!(["The Study spec has no validation windows; only in-sample calibration fit is recorded."]),
            ),
        )),
    );
    let unconfigured = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    assert_eq!(step(&unconfigured, "study")["status"], "partial");
    assert!(step(&unconfigured, "study")["missing"]
        .to_string()
        .contains("generalisation was not checked"));
    assert_eq!(step(&unconfigured, "dual_period")["status"], "missing");
    assert!(step(&unconfigured, "dual_period")["missing"]
        .to_string()
        .contains("fit improved, generalisation was not checked"));
    assert_eq!(step(&unconfigured, "held_out")["status"], "missing");
    assert!(step(&unconfigured, "held_out")["missing"]
        .to_string()
        .contains("no validation windows"));

    fake_study_cli(
        &mut ctx,
        calibration_study_status(calibration_evidence(
            "tuning",
            "lower_loss",
            "unknown",
            calibration_validation(
                "unavailable",
                true,
                json!(["validation target Qle unavailable: window has no paired values"]),
            ),
        )),
    );
    let unavailable = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    assert_eq!(step(&unavailable, "held_out")["status"], "missing");
    assert!(step(&unavailable, "held_out")["missing"]
        .to_string()
        .contains("configured but recorded validation losses are unavailable"));
    assert!(step(&unavailable, "held_out")["missing"]
        .to_string()
        .contains("no paired values"));
    assert!(step(&unavailable, "dual_period")["missing"]
        .to_string()
        .contains("recorded validation losses are unavailable"));
    assert!(unavailable["hypotheses"]
        .to_string()
        .contains("In-sample fit improvement"));
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn calibration_limits_both_periods_and_never_ranks_uncertainty() {
    let mut ctx = fixture("calibration-scope");
    let mut improved = calibration_evidence(
        "tuning",
        "lower_loss",
        "lower_loss",
        calibration_validation("available", true, json!([])),
    );
    improved["comparison"]["best"]["validation"]["score"] = json!(0.2);
    fake_study_cli(&mut ctx, calibration_study_status(improved));
    let improved = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    assert_eq!(step(&improved, "dual_period")["status"], "collected");
    assert!(step(&improved, "dual_period")["missing"]
        .to_string()
        .contains("limited to this Study"));

    let mut uncertainty = calibration_evidence(
        "uncertainty",
        "unknown",
        "unknown",
        calibration_validation("not_configured", false, json!([])),
    );
    uncertainty["selection"]["best_member"] = Value::Null;
    uncertainty["selection"]["no_best_reason"] = json!({"code": "uncertainty", "detail": "an uncertainty analysis ranks parameter effects; it has no best member"});
    fake_study_cli(&mut ctx, calibration_study_status(uncertainty));
    let uncertainty = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    assert_eq!(step(&uncertainty, "dual_period")["status"], "missing");
    assert!(step(&uncertainty, "dual_period")["missing"]
        .to_string()
        .contains("Uncertainty analysis has no best member"));
    assert!(!uncertainty["hypotheses"]
        .to_string()
        .contains("Overfitting"));
    assert_eq!(uncertainty["status"], "incomplete");
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn calibration_evidence_output_stays_valid_json_within_budget() {
    let mut ctx = fixture("calibration-budget");
    let mut evidence = calibration_evidence(
        "tuning",
        "lower_loss",
        "higher_loss",
        calibration_validation(
            "partial",
            true,
            json!(["one validation target could not be scored; the aggregate validation score is not comparable"]),
        ),
    );
    evidence["warnings"] = Value::Array(
        (0..40)
            .map(|index| json!(format!("warning {index}: {}", "x".repeat(2000))))
            .collect(),
    );
    evidence["objective"]["targets"] = Value::Array(
        (0..30)
            .map(|index| json!({"key": format!("target_{index:02}"), "variable": "Qle"}))
            .collect(),
    );
    evidence["validation"]["windows"] = Value::Array(
        (0..25)
            .map(|index| json!({"target": format!("target_{index:02}"), "from": 10, "to": 20}))
            .collect(),
    );
    evidence["comparison"]["best"]["validation"]["targets"] = Value::Array(
        (0..30)
            .map(|index| json!({"key": format!("target_{index:02}"), "loss": 0.5}))
            .collect(),
    );
    fake_study_cli(&mut ctx, calibration_study_status(evidence));
    let result = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    assert_eq!(result["status"], "incomplete");
    let serialized = super::super::result_text(&result);
    assert!(
        serialized.chars().count() <= super::super::MAX_RESULT_CHARS,
        "{}",
        serialized.chars().count()
    );
    let decoded: Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(decoded["status"], "incomplete");
    assert_eq!(step(&decoded, "study")["status"], "partial");
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

// ---- R03/R04：两期关系措辞与共同证据核心 ----

fn recorded_hash() -> String {
    "a".repeat(64)
}

/// 记录模型身份/核验范围/来源路径的证据；read.rs 会按当前选择补 current_selection 与 comparison。
fn with_recorded_identity(mut evidence: Value) -> Value {
    evidence["model_identity"] = json!({
        "study_record": {
            "status": "pinned",
            "name": "CoLM-2026",
            "head": "head-sha",
            "base_commit": "base-sha",
            "version": {"id": "ver-7", "revision": 7},
            "code_source_sha256": recorded_hash(),
            "revision_recorded": true,
        },
        "current_selection": Value::Null,
        "comparison": {"status": "unknown", "matches": Value::Null, "basis": "completed by the assistant"},
    });
    evidence["verification"] = json!({
        "recorded_result_files": 4,
        "succeeded_tasks": 2,
        "reverified": false,
        "note": "scores and losses are read from recorded task-result files; they were not re-verified against history output, observations or the frozen kernel by this status read",
    });
    evidence["source_paths"] = json!({
        "root": "/studies/s-1", "manifest": "manifest.json", "samples": "samples",
        "state": "checkpoints/state", "results": "results/tasks",
    });
    evidence
}

#[cfg(unix)]
fn frozen_model_context(root: &std::path::Path) -> crate::tools::ModelContext {
    crate::tools::ModelContext {
        model: json!({
            "version": {"id": "ver-7", "revision": 7},
            "code": {"source_sha256": recorded_hash()},
        }),
        model_root: root.join("model"),
        source_dir: root.join("source"),
        docs_dir: None,
        source_case: None,
    }
}

#[test]
#[cfg(unix)]
fn calibration_improved_equal_worse_and_unknown_are_reported_separately() {
    let mut ctx = fixture("calibration-relations");
    let available = || calibration_validation("available", true, json!([]));
    let unavailable = || {
        calibration_validation(
            "unavailable",
            true,
            json!(["validation target Qle unavailable: window has no paired values"]),
        )
    };
    let not_configured = || {
        calibration_validation(
            "not_configured",
            false,
            json!(["The Study spec has no validation windows; only in-sample calibration fit is recorded."]),
        )
    };
    // (calibration relation, validation relation, validation fixture, status, required, forbidden)
    type RelationCase = (
        &'static str,
        &'static str,
        Value,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
    );
    let cases: Vec<RelationCase> = vec![
        (
            "lower_loss",
            "lower_loss",
            available(),
            "collected",
            &["Both recorded periods improved", "limited to this Study"],
            &["unchanged (equal)"],
        ),
        (
            "lower_loss",
            "equal",
            available(),
            "partial",
            &[
                "unchanged (equal)",
                "Do not state that both periods improved",
            ],
            &["Both recorded periods improved"],
        ),
        (
            "lower_loss",
            "higher_loss",
            available(),
            "divergent",
            &[
                "explicit divergence",
                "do not claim improved generalisation",
            ],
            &["Both recorded periods improved"],
        ),
        (
            "lower_loss",
            "unknown",
            unavailable(),
            "missing",
            &[
                "recorded validation losses are unavailable",
                "fit improved, generalisation was not checked",
            ],
            &["Both recorded periods improved"],
        ),
        (
            "lower_loss",
            "unknown",
            not_configured(),
            "missing",
            &[
                "validation windows are not configured",
                "fit improved, generalisation was not checked",
            ],
            &["Both recorded periods improved"],
        ),
        (
            "equal",
            "lower_loss",
            available(),
            "missing",
            &["unchanged (equal)", "not a calibration improvement"],
            &["Both recorded periods improved"],
        ),
        (
            "equal",
            "equal",
            available(),
            "missing",
            &["unchanged (equal)", "Held-out validation relation: equal"],
            &["Both recorded periods improved"],
        ),
        (
            "equal",
            "higher_loss",
            available(),
            "missing",
            &[
                "unchanged (equal)",
                "Held-out validation relation: higher_loss",
            ],
            &["Both recorded periods improved"],
        ),
        (
            "equal",
            "unknown",
            unavailable(),
            "missing",
            &["unchanged (equal)", "Held-out validation relation: unknown"],
            &["Both recorded periods improved"],
        ),
        (
            "higher_loss",
            "lower_loss",
            available(),
            "divergent",
            &[
                "higher calibration loss",
                "do not present it as an improvement",
            ],
            &["Both recorded periods improved"],
        ),
        (
            "unknown",
            "lower_loss",
            available(),
            "missing",
            &["Calibration scores are unavailable"],
            &["Both recorded periods improved"],
        ),
    ];
    for (calibration, validation, fixture_status, status, required, forbidden) in cases {
        fake_study_cli(
            &mut ctx,
            calibration_study_status(calibration_evidence(
                "tuning",
                calibration,
                validation,
                fixture_status,
            )),
        );
        let result = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
        let dual = step(&result, "dual_period");
        assert_eq!(
            dual["status"], status,
            "calibration={calibration} validation={validation}"
        );
        let text = dual["missing"].to_string();
        for needle in required {
            assert!(
                text.contains(needle),
                "calibration={calibration} validation={validation}: {needle:?} missing from {text}"
            );
        }
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "calibration={calibration} validation={validation}: unexpectedly claimed {needle:?}"
            );
        }
    }
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn calibration_projection_keeps_actual_model_identity_and_not_reverified() {
    let mut ctx = fixture("calibration-identity");
    ctx.model_context = Some(frozen_model_context(&ctx.project_root));
    fake_study_cli(
        &mut ctx,
        calibration_study_status(with_recorded_identity(calibration_evidence(
            "tuning",
            "lower_loss",
            "lower_loss",
            calibration_validation("available", true, json!([])),
        ))),
    );
    let result = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    let hash = recorded_hash();
    for id in ["study", "dual_period", "held_out"] {
        let evidence = &step(&result, id)["evidence"];
        let identity = &evidence["model_identity"];
        assert_eq!(
            identity["study_record"]["version"]["id"], "ver-7",
            "{id}: {evidence}"
        );
        assert_eq!(identity["study_record"]["version"]["revision"], 7, "{id}");
        assert_eq!(
            identity["study_record"]["code_source_sha256"].as_str(),
            Some(hash.as_str()),
            "{id}"
        );
        assert_eq!(identity["current_selection"]["status"], "frozen", "{id}");
        assert_eq!(
            identity["current_selection"]["version"]["id"], "ver-7",
            "{id}"
        );
        assert_eq!(
            identity["current_selection"]["code_source_sha256"].as_str(),
            Some(hash.as_str()),
            "{id}"
        );
        assert_eq!(identity["comparison"]["status"], "matches", "{id}");
        assert_eq!(evidence["verification"]["reverified"], false, "{id}");
        let scope = evidence["verification"]["scope"].to_string();
        assert!(scope.contains("not re-verified"), "{id}: {scope}");
        assert_eq!(evidence["source_paths"]["root"], "/studies/s-1", "{id}");
        assert_eq!(evidence["objective"]["direction"], "minimize", "{id}");
        assert_eq!(
            evidence["comparison"]["relation"]["calibration"], "lower_loss",
            "{id}"
        );
        assert_eq!(
            evidence["comparison"]["relation"]["validation"], "lower_loss",
            "{id}"
        );
    }
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn calibration_budget_reduction_keeps_identity_and_not_reverified() {
    let mut ctx = fixture("calibration-identity-budget");
    let mut evidence = with_recorded_identity(calibration_evidence(
        "tuning",
        "lower_loss",
        "higher_loss",
        calibration_validation(
            "partial",
            true,
            json!(["one validation target could not be scored; the aggregate validation score is not comparable"]),
        ),
    ));
    evidence["warnings"] = Value::Array(
        (0..40)
            .map(|index| json!(format!("warning {index}: {}", "x".repeat(2000))))
            .collect(),
    );
    evidence["objective"]["targets"] = Value::Array(
        (0..30)
            .map(|index| json!({"key": format!("target_{index:02}"), "variable": "Qle"}))
            .collect(),
    );
    evidence["validation"]["windows"] = Value::Array(
        (0..25)
            .map(|index| json!({"target": format!("target_{index:02}"), "from": 10, "to": 20}))
            .collect(),
    );
    evidence["comparison"]["best"]["validation"]["targets"] = Value::Array(
        (0..30)
            .map(|index| json!({"key": format!("target_{index:02}"), "loss": 0.5}))
            .collect(),
    );
    fake_study_cli(&mut ctx, calibration_study_status(evidence));
    let result = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    let serialized = super::super::result_text(&result);
    assert!(
        serialized.chars().count() <= super::super::MAX_RESULT_CHARS,
        "{}",
        serialized.chars().count()
    );
    let decoded: Value = serde_json::from_str(&serialized).unwrap();
    let hash = recorded_hash();
    for id in ["study", "dual_period", "held_out"] {
        let evidence = &step(&decoded, id)["evidence"];
        assert_eq!(
            evidence["model_identity"]["study_record"]["version"]["id"], "ver-7",
            "{id}: {evidence}"
        );
        assert_eq!(
            evidence["model_identity"]["study_record"]["code_source_sha256"].as_str(),
            Some(hash.as_str()),
            "{id}"
        );
        assert_eq!(evidence["verification"]["reverified"], false, "{id}");
        assert_eq!(evidence["source_paths"]["root"], "/studies/s-1", "{id}");
    }
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
#[cfg(unix)]
fn calibration_reports_missing_identity_and_verification_as_explicit_unknown() {
    let mut ctx = fixture("calibration-missing-identity");
    let mut evidence = calibration_evidence(
        "tuning",
        "lower_loss",
        "lower_loss",
        calibration_validation("available", true, json!([])),
    );
    // 旧证据可能完全没有这些字段：压缩后必须给出具体“不足”原因，不能变成肯定结论。
    evidence["model_identity"] = json!({});
    let object = evidence.as_object_mut().unwrap();
    object.remove("verification");
    object.remove("source_paths");
    fake_study_cli(&mut ctx, calibration_study_status(evidence));
    let result = DiagnosticPlan.call(&calibration_request(), &ctx).unwrap();
    let study = &step(&result, "study")["evidence"];
    assert!(
        study["model_identity"]["study_record"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("does not record"),
        "{study}"
    );
    assert!(study["verification"]["reverified"].is_null(), "{study}");
    assert!(
        study["verification"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("does not record"),
        "{study}"
    );
    assert!(
        study["verification"]["scope"]
            .as_str()
            .unwrap_or_default()
            .contains("did not re-verify"),
        "{study}"
    );
    assert!(
        study["source_paths"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("does not record"),
        "{study}"
    );
    assert!(!study.to_string().contains("\"matches\":true"), "{study}");
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}

#[test]
fn evidence_core_stays_bounded_and_keeps_identity_under_maximum_details() {
    let mut evidence = with_recorded_identity(calibration_evidence(
        "tuning",
        "lower_loss",
        "equal",
        calibration_validation("available", true, json!([])),
    ));
    evidence["study"]["id"] = json!("s".repeat(500));
    evidence["study"]["method"] = json!("m".repeat(500));
    evidence["selection"]["rule"] = json!("r".repeat(500));
    evidence["selection"]["no_best_reason"] =
        json!({"code": "c".repeat(200), "detail": "d".repeat(500)});
    evidence["objective"]["loss"] = json!("l".repeat(500));
    evidence["comparison"]["basis"] = json!("b".repeat(500));
    evidence["comparison"]["baseline"]["reason"] = json!("x".repeat(500));
    evidence["comparison"]["best"]["reason"] = json!("y".repeat(500));
    evidence["model_identity"]["study_record"]["reason"] = json!("i".repeat(500));
    evidence["warnings"] = Value::Array(
        (0..20)
            .map(|index| json!(format!("{index}: {}", "w".repeat(500))))
            .collect(),
    );
    evidence["missing"] = Value::Array(
        (0..20)
            .map(|index| json!(format!("{index}: {}", "m".repeat(500))))
            .collect(),
    );
    evidence["validation"]["reasons"] =
        Value::Array((0..20).map(|_| json!("v".repeat(500))).collect());
    evidence["validation"]["windows"] = Value::Array(
        (0..20)
            .map(|index| json!({"target": "t".repeat(300), "from": index, "to": index + 1}))
            .collect(),
    );
    let core = crate::tools::read::study_evidence_core(&evidence);
    let text = core.to_string();
    assert!(
        text.len() <= crate::tools::read::EVIDENCE_CORE_MAX_BYTES,
        "{} bytes: {text}",
        text.len()
    );
    let hash = recorded_hash();
    assert_eq!(
        core["model_identity"]["study_record"]["version"]["id"],
        "ver-7"
    );
    assert_eq!(
        core["model_identity"]["study_record"]["version"]["revision"],
        7
    );
    assert_eq!(
        core["model_identity"]["study_record"]["code_source_sha256"].as_str(),
        Some(hash.as_str())
    );
    assert_eq!(core["verification"]["reverified"], false);
    assert!(core["verification"]["scope"]
        .as_str()
        .unwrap_or_default()
        .contains("not re-verified"));
    assert_eq!(core["source_paths"]["root"], "/studies/s-1");
    assert_eq!(core["objective"]["direction"], "minimize");
    assert_eq!(core["study"]["kind"], "tuning");
    assert_eq!(core["comparison"]["relation"]["calibration"], "lower_loss");
    assert_eq!(core["comparison"]["relation"]["validation"], "equal");
}

#[test]
fn step_evidence_fallback_keeps_the_shared_core() {
    let evidence = with_recorded_identity(calibration_evidence(
        "tuning",
        "lower_loss",
        "equal",
        calibration_validation("available", true, json!([])),
    ));
    let core = crate::tools::read::study_evidence_core(&evidence);
    let huge = json!({"padding": "p".repeat(10_000), "model_identity": {"status": "deleted"}});
    let reduced = super::bounded_step_evidence(huge, &core);
    assert_eq!(reduced["status"], "abbreviated");
    assert!(reduced.to_string().len() <= super::STEP_EVIDENCE_MAX_BYTES);
    assert_eq!(
        reduced["model_identity"]["study_record"]["version"]["id"],
        "ver-7"
    );
    assert_eq!(reduced["verification"]["reverified"], false);
    assert!(reduced["verification"]["scope"]
        .as_str()
        .unwrap_or_default()
        .contains("not re-verified"));
    assert_eq!(reduced["source_paths"]["root"], "/studies/s-1");
    assert_eq!(reduced["comparison"]["relation"]["validation"], "equal");
}

#[test]
#[cfg(unix)]
fn study_summary_budget_fallback_keeps_identity_and_not_reverified() {
    let mut ctx = fixture("summary-fallback");
    let mut evidence = with_recorded_identity(calibration_evidence(
        "tuning",
        "lower_loss",
        "equal",
        calibration_validation("available", true, json!([])),
    ));
    // 触发 StudySummary 的最后一道 24k 预算保险：身份和“未重新核验”不能被换成一句 abbreviated。
    evidence["study"]["id"] = json!("s".repeat(30_000));
    fake_study_cli(&mut ctx, calibration_study_status(evidence));
    let tool = super::super::read::tools()
        .into_iter()
        .find(|tool| tool.name() == "study_summary")
        .expect("study_summary tool");
    let summary = tool.call(&json!({"study": "study"}), &ctx).unwrap();
    assert_eq!(summary["truncated"], true);
    let evidence = &summary["evidence"];
    assert_eq!(evidence["truncated"], true, "{evidence}");
    let hash = recorded_hash();
    assert_eq!(
        evidence["model_identity"]["study_record"]["version"]["id"],
        "ver-7"
    );
    assert_eq!(
        evidence["model_identity"]["study_record"]["version"]["revision"],
        7
    );
    assert_eq!(
        evidence["model_identity"]["study_record"]["code_source_sha256"].as_str(),
        Some(hash.as_str())
    );
    assert_eq!(evidence["verification"]["reverified"], false);
    assert_eq!(evidence["source_paths"]["root"], "/studies/s-1");
    let text = super::super::result_text(&summary);
    assert!(text.chars().count() <= super::super::MAX_RESULT_CHARS);
    assert!(serde_json::from_str::<Value>(&text).is_ok());
    std::fs::remove_dir_all(ctx.project_root).unwrap();
}
