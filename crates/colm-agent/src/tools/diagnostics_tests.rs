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
