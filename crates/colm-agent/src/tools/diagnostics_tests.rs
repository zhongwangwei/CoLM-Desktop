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
