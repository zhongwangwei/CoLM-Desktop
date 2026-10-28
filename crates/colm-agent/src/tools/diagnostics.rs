//! Fixed, read-only diagnostic recipes. Collecting evidence never completes a causal diagnosis.
use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::read::study_evidence_core;
use super::{object, opt_str, req_str, Tier, Tool, ToolContext};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(DiagnosticPlan)]
}

struct DiagnosticPlan;

fn next(tool: &str, args: Value) -> Value {
    json!({"tool": tool, "arguments": args})
}

fn pending(id: &str, phase: &str, title: &str, missing: &str, next_tool: Value) -> Value {
    json!({"id": id, "phase": phase, "title": title, "status": "pending",
        "evidence": null, "missing": [missing], "next_tool": next_tool})
}

// Keep the surrounding result valid JSON instead of relying on the registry's text truncation.
fn bounded(value: Value) -> Value {
    let text = value.to_string();
    if text.len() <= 1800 {
        value
    } else {
        json!({"truncated": true, "original_bytes": text.len(),
            "preview": text.chars().take(240).collect::<String>(),
            "notice": "Evidence abbreviated; call the source tool for details."})
    }
}

/// 每步证据的传输上限：共同证据核心（身份/来源/未复核范围）+ 该检查自己的分数。
const STEP_EVIDENCE_MAX_BYTES: usize = 4_000;

/// 任何一步证据超限时只留共同证据核心的说明；身份与限制不能被压缩掉。
const CORE_ONLY_NOTICE: &str = "Diagnostic detail was omitted to fit the response budget; Study/model identity, period relations, missing reasons and the not-re-verified scope are preserved in the evidence core.";

fn evidence_bytes(value: &Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

/// 双期/留出检查也带上共同证据核心里的来源：成绩对应哪个模型版本、这次是否重新核验。
fn provenance_core(core: &Value) -> Value {
    json!({
        "study": core["study"],
        "objective": core["objective"],
        "comparison": core["comparison"],
        "model_identity": core["model_identity"],
        "verification": core["verification"],
        "source_paths": core["source_paths"],
        "missing": core["missing"],
    })
}

fn with_provenance(mut value: Value, core: &Value) -> Value {
    let provenance = provenance_core(core);
    if let (Some(value), Some(provenance)) = (value.as_object_mut(), provenance.as_object()) {
        for (key, field) in provenance {
            value.insert(key.clone(), field.clone());
        }
    }
    value
}

/// 超限时只留共同证据核心：可以删明细，但身份、两期关系、缺失原因与
/// `verification.reverified`/核验范围必须还在。
fn bounded_step_evidence(value: Value, core: &Value) -> Value {
    if evidence_bytes(&value) <= STEP_EVIDENCE_MAX_BYTES {
        return value;
    }
    json!({
        "status": "abbreviated",
        "notice": CORE_ONLY_NOTICE,
        "study": core["study"],
        "execution": core["execution"],
        "selection": core["selection"],
        "objective": core["objective"],
        "comparison": core["comparison"],
        "validation": core["validation"],
        "model_identity": core["model_identity"],
        "verification": core["verification"],
        "source_paths": core["source_paths"],
        "warnings": core["warnings"],
        "missing": core["missing"],
    })
}

fn collect(
    id: &str,
    title: &str,
    tool: &str,
    args: Value,
    ctx: &ToolContext,
) -> (Value, Option<Value>) {
    let mut tools = super::read::tools();
    tools.extend(super::code::tools());
    let result = tools
        .iter()
        .find(|t| t.name() == tool && t.tier() == Tier::Read)
        .ok_or_else(|| anyhow::anyhow!("read-only tool {tool} unavailable"))
        .and_then(|t| t.call(&args, ctx));
    match result {
        Ok(value) => (
            json!({"id": id, "phase": "checkdata", "title": title,
            "status": "collected", "evidence": bounded(value.clone()), "missing": [],
            "next_tool": next(tool, args)}),
            Some(value),
        ),
        Err(error) => (
            json!({"id": id, "phase": "checkdata", "title": title,
            "status": "missing", "evidence": null,
            "missing": [format!("{error:#}").chars().take(240).collect::<String>()],
            "next_tool": next(tool, args)}),
            None,
        ),
    }
}

impl Tool for DiagnosticPlan {
    fn name(&self) -> &'static str {
        "diagnostic_plan"
    }
    fn description(&self) -> &'static str {
        "Collect a read-only diagnosis snapshot and required checklist for startup, closure, flux, calibration or parity. Always reads run status and configuration. Missing evidence remains explicit; no runs, builds or modifications."
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn parameters(&self) -> Value {
        object(json!({
            "workflow": {"type":"string", "enum":["startup","closure","flux","calibration","parity"]},
            "case": {"type":"string", "description":"case directory"},
            "variable": {"type":["string","null"], "description":"history variable for flux, e.g. f_assim"},
            "study": {"type":["string","null"], "description":"Study directory for calibration"},
            "workspace": {"type":["string","null"], "description":"development workspace name for parity"},
            "flux": flux_parameters()
        }))
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let workflow = req_str(args, "workflow")?;
        if !["startup", "closure", "flux", "calibration", "parity"].contains(&workflow) {
            bail!("unknown diagnostic workflow: {workflow}");
        }
        let case = req_str(args, "case")?;
        if args.as_object().is_none_or(|a| {
            a.keys().any(|k| {
                !["workflow", "case", "variable", "study", "workspace", "flux"]
                    .contains(&k.as_str())
            })
        }) {
            bail!("unexpected diagnostic argument");
        }
        for key in ["variable", "study", "workspace"] {
            if !args[key].is_null() && !args[key].is_string() {
                bail!("{key} must be a string or null");
            }
        }
        for key in ["case", "variable", "study", "workspace"] {
            if args[key].as_str().is_some_and(|s| {
                s.len() > 1024 || serde_json::to_string(s).is_ok_and(|text| text.len() > 2048)
            }) {
                bail!("{key} exceeds 1024 bytes");
            }
        }
        if !args["flux"].is_null() && (workflow != "flux" || !args["flux"].is_object()) {
            bail!("flux must be an object for the flux workflow only");
        }
        if args["flux"].to_string().len() > 16384 {
            bail!("flux request exceeds 16 KiB");
        }
        let mut steps = Vec::new();
        let (mut status, run) = collect(
            "run_status",
            "Read stage state and log evidence",
            "run_status",
            json!({"case":case}),
            ctx,
        );
        if run.as_ref().is_some_and(|v| {
            v["stages"].is_null() && v["log_tails"].as_object().is_none_or(|m| m.is_empty())
        }) {
            status["status"] = json!("missing");
            status["missing"] = json!([
                "No readable stage state or run logs; successful execution is not established."
            ]);
        }
        steps.push(status);
        steps.push(
            collect(
                "case_config",
                "Read case and forcing configuration",
                "read_case_config",
                json!({"case":case,"names":null,"filter":null}),
                ctx,
            )
            .0,
        );
        let mut hypotheses = Vec::new();
        let experiment;
        // calibration 的共同证据核心：最后的总预算保险也要用它保留身份与未复核范围。
        let mut study_core: Option<Value> = None;
        match workflow {
            "startup" => {
                steps.push(pending("inputs", "checkdata", "Verify forcing, surface and restart inputs", "Configuration paths alone do not prove files exist, cover the requested time, or match the case.", next("read_case_config",json!({"case":case,"names":null,"filter":"DEF_"}))));
                steps.push(pending("first_failure", "localise", "Locate the earliest failed stage and first error", "Inspect stage order, exit evidence and preceding log context; error words alone do not identify a cause.", next("run_status",json!({"case":case}))));
                experiment = "After identifying the first failed stage, propose one input or configuration correction and the smallest stage rerun; do not execute it in this snapshot.";
            }
            "closure" | "flux" => {
                let vars = if workflow == "closure" {
                    vec!["f_xerr", "f_zerr"]
                } else {
                    opt_str(args, "variable").into_iter().collect()
                };
                if workflow == "flux" && args["flux"].is_object() {
                    steps.push(pending(
                        "series",
                        "checkdata",
                        "Collect bounded flux evidence",
                        "Awaiting the bounded flux diagnostic",
                        Value::Null,
                    ));
                } else if vars.is_empty() {
                    steps.push(pending(
                        "series",
                        "checkdata",
                        "Select the target flux history variable",
                        "A variable is required; no flux statistics were examined.",
                        Value::Null,
                    ));
                } else {
                    let (mut step, data) = collect(
                        "series",
                        "Collect finite history statistics",
                        "series_stats",
                        json!({"case":case,"vars":vars}),
                        ctx,
                    );
                    let missing: Vec<_> = vars
                        .iter()
                        .filter(|var| {
                            data.as_ref().is_none_or(|v| {
                                let stats = &v["stats"][**var];
                                v["records"].as_u64().unwrap_or(0) == 0
                                    || stats["count"] != v["records"]
                                    || stats["count"].as_u64().unwrap_or(0)
                                        <= stats["nan_or_missing"].as_u64().unwrap_or(0)
                                    || stats["mean"].as_f64().is_none()
                            })
                        })
                        .map(|v| format!("No finite timed samples for {v}"))
                        .collect();
                    if !missing.is_empty() {
                        step["status"] = json!("missing");
                        if data.is_some() {
                            step["missing"] = json!(missing);
                        } else {
                            step["missing"]
                                .as_array_mut()
                                .unwrap()
                                .extend(missing.into_iter().map(Value::String));
                        }
                    }
                    steps.push(step);
                }
                if workflow == "closure" {
                    steps.push(pending("closure_scales", "localise", "Check water and energy closure separately", "Require BOTH f_xerr (mm/s) and f_zerr (W/m2), time step, integration interval and agreed scale-aware thresholds. Means alone cannot establish closure or localise a spike.", next("series_stats",json!({"case":case,"vars":["f_xerr","f_zerr"]}))));
                    experiment = "Inspect the first anomalous time window and its water/energy terms; propose a single short-window rerun only after thresholds and inputs are verified.";
                } else {
                    steps.push(pending("observation_alignment", "checkdata", "Verify observation file, time alignment, units and sign conventions", "No observation file supplied. Confirm timestamps, timezone, averaging interval, units, sign, missing values and observation quality masks before metrics.", json!({"tool":"metrics","arguments":{"case":case,"obs":null,"from":null,"to":null},"requires":["obs"]})));
                    steps.push(pending("regimes", "localise", "Compare seasonal, day/night and wet/dry residuals", "Aggregate/monthly statistics do not establish seasonal alignment, day/night or wet/dry residual checks; aligned observations and regime definitions are required.", Value::Null));
                    hypotheses.push(json!({"status":"unverified","candidate":"Observation alignment, forcing, state or process-parameter mismatch","basis":"Checklist candidate only; correlation does not establish causality."}));
                    experiment = "After observation alignment and regime checks, propose one physically motivated parameter or forcing perturbation over the smallest informative window, retaining a baseline.";
                }
            }
            "calibration" => {
                let mut study_path = String::new();
                let mut study_data: Option<Value> = None;
                if let Some(study) = opt_str(args, "study") {
                    study_path = study.to_owned();
                    let (mut step, data) = collect(
                        "study",
                        "Collect Study baseline, objective and parameter ranges",
                        "study_summary",
                        json!({"study":study}),
                        ctx,
                    );
                    let evidence = data.as_ref().and_then(study_evidence_object).cloned();
                    study_data = evidence.clone();
                    match evidence.as_ref() {
                        Some(evidence) => {
                            let core = study_evidence_core(evidence);
                            let missing = calibration_study_missing(evidence);
                            // 完整诊断与预算缩减都要能交代身份与未复核范围：证据核心不可丢。
                            step["evidence"] = bounded_step_evidence(
                                calibration_projection(evidence, &core),
                                &core,
                            );
                            step["status"] = json!(if missing.is_empty() {
                                "collected"
                            } else {
                                "partial"
                            });
                            step["missing"] = json!(missing);
                            study_core = Some(core);
                        }
                        None => {
                            // 保留 collect 写下的 CLI/工具错误（例如 "cannot start …"），
                            // 只追加"为什么不能做双期比较"，不能让真实错误消失。
                            step["status"] = json!("missing");
                            step["evidence"] = Value::Null;
                            let mut missing =
                                step["missing"].as_array().cloned().unwrap_or_default();
                            missing.push(json!(
                                "Structured Study evidence is unavailable (older CLI, remote engine snapshot or unreadable Study); baseline and best objectives cannot be verified."
                            ));
                            step["missing"] = Value::Array(missing);
                        }
                    }
                    steps.push(step);
                    steps.push(calibration_dual_period_step(
                        &study_path,
                        evidence.as_ref(),
                        study_core.as_ref(),
                    ));
                } else {
                    steps.push(pending("study", "checkdata", "Identify the Study", "Study directory is required to inspect objectives and bounds.", json!({"tool":"study_summary","arguments":{"study":null},"requires":["study"]})));
                }
                steps.push(pending("bounds", "localise", "Check parameter bounds, identifiability and objective tradeoffs", "Verify the Study includes this case and each parameter is active in its mode/build. A boundary hit is not proof of compensation or causality; inspect repeated candidates, sensitivity, baseline and other target variables.", Value::Null));
                if study_data.is_some() {
                    steps.push(calibration_held_out_step(
                        &study_path,
                        study_data.as_ref(),
                        study_core.as_ref(),
                    ));
                } else {
                    steps.push(pending("held_out", "validate", "Verify held-out time/site performance and physical closure", "Independent validation, observation alignment and closure evidence are required before adopting tuned parameters.", Value::Null));
                }
                hypotheses.push(json!({"status":"unverified","candidate":"Weak identifiability or compensating parameters","basis":"Bounds or improved calibration fit alone cannot verify this candidate."}));
                if let Some(evidence) = study_data.as_ref() {
                    if let Some(hypothesis) = calibration_divergence_hypothesis(evidence) {
                        hypotheses.push(hypothesis);
                    }
                    if let Some(hypothesis) = calibration_identity_hypothesis(evidence) {
                        hypotheses.push(hypothesis);
                    }
                }
                experiment = "Propose one bounded sensitivity perturbation against the physics baseline and held-out observations; never auto-adopt a best member.";
            }
            "parity" => {
                if let Some(workspace) = opt_str(args, "workspace") {
                    steps.push(
                        collect(
                            "build_gate",
                            "Read workspace platform/build context and current-commit gates",
                            "workspace_status",
                            json!({"name":workspace}),
                            ctx,
                        )
                        .0,
                    );
                } else {
                    steps.push(pending(
                        "build_gate",
                        "checkdata",
                        "Identify workspace and current-commit build gates",
                        "Workspace required; no build or gate evidence examined.",
                        next("workspace_list", json!({})),
                    ));
                }
                steps.push(pending("platform", "checkdata", "Verify platform, compiler flags, revisions and gate freshness first", "Confirm platform, same inputs, built executable revisions and current-commit gates before interpreting a mismatch. Apple Silicon and x86_64 Linux supported builds use bitwise comparison; other platforms require an explicit platform policy.", Value::Null));
                steps.push(pending("first_difference", "localise", "Locate first differing variable, record and value range", "Require both existing output paths and first divergent record; inspect finite ranges/NaN before tracing the generating code. compare_outputs accepts a,b,rtol,atol; first_records belongs to parity_check, which runs models and is NOT invoked here.", json!({"tool":"compare_outputs","arguments":{"a":null,"b":null,"rtol":null,"atol":null},"requires":["a","b"]})));
                experiment = "After platform/build/gate verification, propose a minimal comparison. For unsupported platforms only, the documented parity_check policy is rtol=1e-9, first_records=2, ignore=f_frcsat; verify applicability and request run approval separately. Never relax tolerances to hide a supported-platform mismatch.";
            }
            _ => unreachable!(),
        }
        steps.push(pending(
            "minimal_validation",
            "validate",
            "Design and review one discriminating minimal experiment",
            experiment,
            Value::Null,
        ));
        steps.push(pending("report", "report", "Report confirmed facts, unverified hypotheses and remaining evidence", "Evidence collection is not a completed diagnosis; resolve each pending/missing check before stating a cause or physical pass.", Value::Null));
        let mut flux_evidence = Value::Null;
        if workflow == "flux" && args["flux"].is_object() {
            let mut request = args["flux"].clone();
            request["variable"] = json!(req_str(args, "variable")?);
            request["obs"] = json!(ctx.resolve(req_str(&request, "obs")?));
            if request["experiment"].is_object() {
                request["experiment"]["case"] =
                    json!(ctx.resolve(req_str(&request["experiment"], "case")?));
            }
            let case_path = ctx.resolve(case);
            let result = ctx.cli_json(&[
                "flux-diagnose",
                &case_path.to_string_lossy(),
                "--request",
                &request.to_string(),
            ]);
            match result {
                Ok(data) => {
                    for step in &mut steps {
                        let key = match step["id"].as_str() {
                            Some("observation_alignment") => Some("alignment"),
                            Some("series") => Some("metrics"),
                            Some("regimes") => Some("regimes"),
                            _ => None,
                        };
                        if let Some(key) = key {
                            step["status"] = json!(if ["alignment", "metrics"].contains(&key)
                                && data[key]["status"] == "computed"
                            {
                                "collected"
                            } else if key == "regimes" {
                                "partial"
                            } else {
                                "missing"
                            });
                            if key == "regimes"
                                && ["calendar_month", "daylight", "wetness"]
                                    .iter()
                                    .all(|k| data[key][*k]["status"] == "computed")
                            {
                                step["status"] = json!("collected");
                            }
                            step["evidence"] = json!({"evidence_id":data["evidence_id"],"ref":format!("flux_evidence.{key}")});
                            step["missing"] = if step["status"] == "collected" {
                                json!([])
                            } else {
                                json!(["See structured evidence for missing or insufficient observations/regime conditions."])
                            };
                        }
                        if step["id"] == "minimal_validation" {
                            step["evidence"] = json!({"evidence_id":data["evidence_id"],"ref":if data.get("experiment").is_some(){"flux_evidence.experiment"}else{"flux_evidence.minimal_experiment"}});
                            step["status"] = json!(if data.get("experiment").is_some() {
                                "unverified"
                            } else {
                                "proposed"
                            });
                            step["missing"]=json!(["Experimental identity, causal mechanism and independent validation remain unverified."]);
                        }
                        if step["id"] == "report" {
                            step["status"] = json!("collected");
                            step["evidence"] = json!({"scope":"Descriptive evidence and experiment proposal only; no confirmed cause.","evidence_id":data["evidence_id"]});
                            step["missing"] = json!([]);
                        }
                    }
                    hypotheses = data["hypotheses"].as_array().cloned().unwrap_or_default();
                    flux_evidence = data;
                }
                Err(e) => {
                    for step in &mut steps {
                        if step["id"] == "observation_alignment" {
                            step["status"] = json!("missing");
                            step["missing"] =
                                json!([e.to_string().chars().take(1000).collect::<String>()]);
                        }
                    }
                }
            }
        }
        let confirmed: Vec<_> = steps.iter().filter(|s| s["status"] == "collected")
            .map(|s| json!({"step":s["id"],"fact":"Source evidence collected; interpretation remains subject to required checks."})).collect();
        let mut result = json!({"version":1,"workflow":workflow,"inputs":{"case":case,"variable":args["variable"],"study":args["study"],"workspace":args["workspace"]},
            "phases":["checkdata","localise","validate","report"],"status":"incomplete",
            "steps":steps,"flux_evidence":flux_evidence,"confirmed_facts":confirmed,"hypotheses":hypotheses,
            "minimal_experiment":if flux_evidence.is_null(){json!({"status":"proposed_not_executed","description":experiment})}else{json!({"status":"proposed_not_executed","ref":"flux_evidence.minimal_experiment"})}});
        // Keep JSON parseable under the registry's 24k-character transport limit.
        if !flux_evidence.is_null() && result.to_string().len() > 22000 {
            result["flux_evidence"] = json!({"status":"abbreviated","evidence_id":flux_evidence["evidence_id"],"metrics":flux_evidence["metrics"],"notice":"Detailed evidence exceeded the response budget; missing referenced sections were omitted. Request a narrower diagnostic or inspect flux-diagnose directly.","minimal_experiment":{"status":"proposed_not_executed","cause_verified":false,"missing":["Detailed evidence omitted; no validation or causal claim is established."]}});
            if !flux_evidence["experiment"].is_null() {
                result["flux_evidence"]["experiment"] = json!({"status":"unverified","cause_verified":false,"missing":["Detailed experimental evidence omitted; no validation or causal claim is established."]});
            }
        }
        // calibration 的 Study 证据也走同一条有界输出路径；这里只是最后一道保险。
        // 即使总结果超限，也保留共同证据核心里的身份、两期关系与未复核范围。
        if result.to_string().len() > 22000 {
            if let Some(core) = study_core.as_ref() {
                if let Some(steps) = result["steps"].as_array_mut() {
                    for step in steps {
                        if matches!(
                            step["id"].as_str(),
                            Some("study" | "dual_period" | "held_out")
                        ) {
                            step["evidence"] = json!({
                                "status": "abbreviated",
                                "notice": "Evidence exceeded the diagnostic response budget; the shared evidence core keeps Study/model identity, period relations, missing reasons and the not-re-verified scope.",
                                "study": core["study"],
                                "selection": core["selection"],
                                "objective": core["objective"],
                                "comparison": core["comparison"],
                                "validation": core["validation"],
                                "model_identity": core["model_identity"],
                                "verification": core["verification"],
                                "source_paths": core["source_paths"],
                                "missing": core["missing"],
                            });
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

fn short(text: &str, limit: usize) -> String {
    let mut out = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        out.push('…');
    }
    out
}

/// `study_summary` 结果里的结构化证据；旧 CLI / 远端引擎快照没有 `evidence` 时返回 None。
fn study_evidence_object(value: &Value) -> Option<&Value> {
    value
        .get("evidence")
        .filter(|evidence| evidence.is_object() && evidence["status"] != "missing")
}

/// 率定检查用的紧凑投影：在共同证据核心之上补两期分数、目标覆盖与参数位置。
/// 身份、来源、目标方向、baseline/best、缺失原因与未复核范围由证据核心保留，不再只留状态。
fn calibration_projection(evidence: &Value, core: &Value) -> Value {
    let at_bound = evidence["parameters"]["best"]["values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|value| value["at_bound"].as_bool() == Some(true))
        .filter_map(|value| value["key"].as_str())
        .collect::<Vec<_>>();
    let mut projection = core.clone();
    projection["status"] = evidence["study"]["status"].clone();
    projection["calibration"] = json!({
        "baseline_score": evidence["comparison"]["baseline"]["calibration"]["score"],
        "best_score": evidence["comparison"]["best"]["calibration"]["score"],
        "relation": evidence["comparison"]["relation"]["calibration"],
    });
    projection["validation"]["baseline_score"] =
        evidence["comparison"]["baseline"]["validation"]["score"].clone();
    projection["validation"]["best_score"] =
        evidence["comparison"]["best"]["validation"]["score"].clone();
    projection["validation"]["relation"] = evidence["comparison"]["relation"]["validation"].clone();
    projection["parameters"] = json!({
        "dimension_count": evidence["parameters"]["dimension_count"],
        "hybrid_weight_count": evidence["parameters"]["hybrid_weight_count"],
        "best_at_bound": at_bound,
    });
    projection
}

/// Study 证据本身缺什么：无最优、验证未配置/不可用、身份冲突都要分开说，不能当成"通过"。
fn calibration_study_missing(evidence: &Value) -> Vec<String> {
    let mut missing = Vec::new();
    if evidence["selection"]["best_member"].is_null() {
        let code = evidence["selection"]["no_best_reason"]["code"]
            .as_str()
            .unwrap_or("unknown");
        let detail = evidence["selection"]["no_best_reason"]["detail"]
            .as_str()
            .unwrap_or("");
        missing.push(short(
            &format!("No best member is recorded ({code}): {detail}"),
            240,
        ));
    }
    if evidence["validation"]["configured"].as_bool() == Some(false) {
        missing.push("Validation windows are not configured in the Study spec: calibration improvement is in-sample fit only; generalisation was not checked.".into());
    } else if evidence["validation"]["status"].as_str() != Some("available") {
        missing.push(short(
            &format!(
                "Validation evidence is incomplete ({}): {}",
                evidence["validation"]["status"]
                    .as_str()
                    .unwrap_or("unknown"),
                evidence["validation"]["reasons"]
            ),
            240,
        ));
    }
    if evidence["model_identity"]["comparison"]["status"].as_str() == Some("mismatch") {
        missing.push("The Study's recorded model identity does not match the current selection; its scores do not describe the currently selected model.".into());
    }
    match evidence["comparison"]["relation"]["calibration"]
        .as_str()
        .unwrap_or("unknown")
    {
        "lower_loss" => {}
        "equal" => missing.push(
            "The recorded best member's calibration loss is unchanged from the baseline (equal); it is not a calibration improvement.".into(),
        ),
        "higher_loss" => missing.push(
            "The recorded best member has a higher calibration loss than the baseline; it is not an improvement.".into(),
        ),
        _ => missing.push(
            "The calibration-loss relation between the best member and the baseline is unknown; whether calibration improved cannot be established.".into(),
        ),
    }
    missing.truncate(6);
    missing
}

/// 双期比较的独立检查：率定×验证分别按 improved/equal/worse/unknown 处理。
/// 只有两期都 lower_loss 才说两期改善；验证持平必须明确说持平，率定持平/变差/未知各有表述。
/// 证据里同时带上共同证据核心，助手能交代成绩对应哪个模型版本、这次是否重新核验。
fn calibration_dual_period_step(
    study: &str,
    evidence: Option<&Value>,
    core: Option<&Value>,
) -> Value {
    let next_tool = next("study_summary", json!({"study": study}));
    let (Some(evidence), Some(core)) = (evidence, core) else {
        return json!({"id":"dual_period","phase":"localise",
            "title":"Compare calibration and held-out validation losses",
            "status":"missing","evidence":null,
            "missing":["Structured Study evidence is unavailable; calibration improvement cannot be compared with validation."],
            "next_tool":next_tool});
    };
    let kind = evidence["study"]["kind"].as_str().unwrap_or("unknown");
    let execution = evidence["execution"]["state"].as_str().unwrap_or("unknown");
    let relation = &evidence["comparison"]["relation"];
    let calibration = relation["calibration"].as_str().unwrap_or("unknown");
    let validation = relation["validation"].as_str().unwrap_or("unknown");
    let configured = evidence["validation"]["configured"]
        .as_bool()
        .unwrap_or(false);
    let (status, missing) = if kind != "tuning" {
        ("missing", vec!["Uncertainty analysis has no best member; it does not select or rank a best parameter set.".to_owned()])
    } else if evidence["selection"]["best_member"].is_null() {
        let detail = evidence["selection"]["no_best_reason"]["detail"]
            .as_str()
            .unwrap_or("no best member");
        (
            "missing",
            vec![short(
                &format!("No best member is recorded ({execution}): {detail}; no dual-period comparison exists."),
                240,
            )],
        )
    } else {
        // 两期各自判断；任何分支都不把"率定改善"或"验证改善"单独说成两期改善。
        match calibration {
            "unknown" => (
                "missing",
                vec!["Calibration scores are unavailable; no calibration-versus-validation comparison was made.".to_owned()],
            ),
            "higher_loss" => (
                "divergent",
                vec!["The recorded best member has a higher calibration loss than the baseline; do not present it as an improvement even if validation improved.".to_owned()],
            ),
            "equal" => (
                "missing",
                vec![format!(
                    "Calibration loss was unchanged (equal) between the recorded best member and the baseline; the best member is not a calibration improvement, so no calibration-versus-validation improvement claim is supported. Held-out validation relation: {validation}."
                )],
            ),
            "lower_loss" => match validation {
                "lower_loss" => (
                    "collected",
                    vec!["Both recorded periods improved: calibration and held-out validation losses were lower for the best member than for the baseline. The claim is limited to this Study, its targets and these windows, and independent validation is still required.".to_owned()],
                ),
                "equal" => (
                    "partial",
                    vec!["Calibration loss was lower while held-out validation loss was unchanged (equal); in-sample fit improved, but held-out generalisation did not improve. Do not state that both periods improved.".to_owned()],
                ),
                "higher_loss" => (
                    "divergent",
                    vec!["Calibration loss fell while held-out validation loss rose. Report this as an explicit divergence: do not claim improved generalisation and do not diagnose overfitting from this evidence alone.".to_owned()],
                ),
                "unknown" if configured => (
                    "missing",
                    vec![short(
                        &format!(
                            "Calibration loss fell but recorded validation losses are unavailable (validation status: {}): {}; fit improved, generalisation was not checked.",
                            evidence["validation"]["status"]
                                .as_str()
                                .unwrap_or("unknown"),
                            evidence["validation"]["reasons"]
                        ),
                        320,
                    )],
                ),
                "unknown" => (
                    "missing",
                    vec!["Calibration loss fell but validation windows are not configured in the Study spec; fit improved, generalisation was not checked.".to_owned()],
                ),
                other => (
                    "missing",
                    vec![format!("Unrecognised validation relation {other:?}; no calibration-versus-validation improvement claim is supported.")],
                ),
            },
            other => (
                "missing",
                vec![format!("Unrecognised calibration relation {other:?}; no calibration improvement claim is supported.")],
            ),
        }
    };
    let evidence_out = with_provenance(
        json!({
            "calibration": {
                "baseline": evidence["comparison"]["baseline"]["calibration"]["score"],
                "best": evidence["comparison"]["best"]["calibration"]["score"],
                "relation": calibration,
            },
            "validation": {
                "configured": configured,
                "status": evidence["validation"]["status"],
                "baseline": evidence["comparison"]["baseline"]["validation"]["score"],
                "best": evidence["comparison"]["best"]["validation"]["score"],
                "relation": validation,
            },
            "selection": {
                "best_member": evidence["selection"]["best_member"],
                "provisional": evidence["selection"]["provisional"],
            },
        }),
        core,
    );
    json!({"id":"dual_period","phase":"localise",
        "title":"Compare calibration and held-out validation losses",
        "status":status,
        "evidence":bounded_step_evidence(evidence_out, core),
        "missing":missing,
        "next_tool":next_tool})
}

/// held_out 检查真实消费验证状态：未配置与已配置但不可用给不同缺失原因；
/// 证据里同样带上共同证据核心，保留身份、来源与"未重新核验"的范围。
fn calibration_held_out_step(study: &str, evidence: Option<&Value>, core: Option<&Value>) -> Value {
    let title = "Verify held-out time/site performance and physical closure";
    let next_tool = next("study_summary", json!({"study": study}));
    let (Some(evidence), Some(core)) = (evidence, core) else {
        return pending(
            "held_out",
            "validate",
            title,
            "Independent validation, observation alignment and closure evidence are required before adopting tuned parameters.",
            next_tool,
        );
    };
    let configured = evidence["validation"]["configured"]
        .as_bool()
        .unwrap_or(false);
    let status = evidence["validation"]["status"]
        .as_str()
        .unwrap_or("unknown");
    if !configured {
        let evidence_out = with_provenance(
            json!({
                "validation": {"configured": false, "status": "not_configured"},
                "calibration": {
                    "baseline": evidence["comparison"]["baseline"]["calibration"]["score"],
                    "best": evidence["comparison"]["best"]["calibration"]["score"],
                },
            }),
            core,
        );
        return json!({"id":"held_out","phase":"validate","title":title,"status":"missing",
            "evidence":bounded_step_evidence(evidence_out, core),
            "missing":["The Study spec has no validation windows; held-out performance was never measured, so generalisation is unverified."],
            "next_tool":next_tool});
    }
    if status == "available" {
        let evidence_out = with_provenance(
            json!({
                "validation": {
                    "configured": true,
                    "status": "available",
                    "windows": evidence["validation"]["windows"],
                    "baseline": evidence["comparison"]["baseline"]["validation"]["score"],
                    "best": evidence["comparison"]["best"]["validation"]["score"],
                    "relation": evidence["comparison"]["relation"]["validation"],
                },
            }),
            core,
        );
        return json!({"id":"held_out","phase":"validate","title":title,"status":"partial",
            "evidence":bounded_step_evidence(evidence_out, core),
            "missing":["Recorded Study validation windows are the only held-out evidence; physical closure, observation alignment and an independent period/site are still required before adopting tuned parameters."],
            "next_tool":next_tool});
    }
    let evidence_out = with_provenance(
        json!({
            "validation": {
                "configured": true,
                "status": status,
                "reasons": evidence["validation"]["reasons"],
            },
        }),
        core,
    );
    json!({"id":"held_out","phase":"validate","title":title,"status":"missing",
        "evidence":bounded_step_evidence(evidence_out, core),
        "missing":[short(&format!(
            "Validation windows are configured but recorded validation losses are unavailable ({status}): {}",
            evidence["validation"]["reasons"]
        ), 320)],
        "next_tool":next_tool})
}

/// 率定改善但验证变差/缺失：只能作为未验证假设，不能当诊断结论。
fn calibration_divergence_hypothesis(evidence: &Value) -> Option<Value> {
    if evidence["study"]["kind"].as_str() != Some("tuning") {
        return None;
    }
    let relation = &evidence["comparison"]["relation"];
    let calibration = relation["calibration"].as_str().unwrap_or("unknown");
    let validation = relation["validation"].as_str().unwrap_or("unknown");
    if calibration == "lower_loss" && validation == "higher_loss" {
        return Some(
            json!({"status":"unverified","candidate":"Overfitting or compensating parameter bias",
            "basis":"Calibration loss fell while held-out validation loss rose. This is an explicit divergence, not a confirmed cause: do not claim improved generalisation and do not diagnose overfitting from this evidence alone."}),
        );
    }
    if calibration == "lower_loss" && validation == "equal" {
        return Some(
            json!({"status":"unverified","candidate":"In-sample fit improvement without a held-out improvement",
            "basis":"Calibration loss fell while held-out validation loss was unchanged (equal); the recorded windows show no generalisation improvement, and this is not evidence of degradation either."}),
        );
    }
    if calibration == "lower_loss" && validation == "unknown" {
        return Some(
            json!({"status":"unverified","candidate":"In-sample fit improvement beyond the recorded evidence",
            "basis":"Calibration improved but validation was not configured or recorded; generalisation is unverified."}),
        );
    }
    None
}

fn calibration_identity_hypothesis(evidence: &Value) -> Option<Value> {
    if evidence["model_identity"]["comparison"]["status"].as_str() != Some("mismatch") {
        return None;
    }
    Some(
        json!({"status":"unverified","candidate":"Model-identity mismatch between the Study record and the current selection",
        "basis":"The Study's frozen revision identity (version id, revision and code.source_sha256) differs from the current selection; its recorded scores describe the Study's model, not the one currently selected."}),
    )
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;

fn flux_parameters() -> Value {
    let nullable = |mut v: Value| {
        v["type"] = json!(["object", "null"]);
        v
    };
    let text = json!({"type":"string"});
    let number = json!({"type":"number"});
    nullable(object(json!({
        "obs":{"type":"string","description":"Observation NetCDF; single point only."},
        "observation_variable":text,"observation_qc":{"type":["string","null"],"description":"QC=0 is measured; null explicitly permits finite-only observations."},
        "from":{"type":"integer","description":"Inclusive Unix UTC seconds."},"to":{"type":"integer","description":"Exclusive Unix UTC seconds, at most 366 days after from."},
        "model_units":text,"obs_units":text,"output_units":text,"model_scale":number,"obs_scale":number,
        "sign_convention":{"type":"string","description":"Explicit common physical sign after applying scales; never infer."},
        "model_utc_offset_seconds":{"type":"integer","description":"Model clock minus UTC; whole-minute offset only, 0 required for Greenwich case."},
        "observation_utc_offset_seconds":{"type":"integer","description":"Observation clock minus UTC, explicit whole-minute fixed offset; no DST."},
        "alignment":{"type":"string","enum":["colm_plumber2_fixed_offset"]},
        "model_interval_seconds":{"type":"integer","enum":[1800,3600]},"observation_interval_seconds":{"type":"integer","enum":[1800]},
        "daylight":nullable(object(json!({"variable":text,"qc_variable":text,"units":{"type":"string","enum":["W m-2"]},"threshold":number}))),
        "wetness":nullable(object(json!({"variable":text,"qc_variable":text,"units":{"type":"string","enum":["kg m-2 s-1"]},"window_hours":{"type":"integer","minimum":1,"maximum":168},"threshold_mm":number}))),
        "experiment":nullable(object(json!({"case":text,"field":text,"expected_direction":{"type":"string","enum":["bias_increase","bias_decrease","rmse_decrease"]}})))
    })))
}
