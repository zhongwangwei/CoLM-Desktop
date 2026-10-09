//! Fixed, read-only diagnostic recipes. Collecting evidence never completes a causal diagnosis.
use anyhow::{bail, Result};
use serde_json::{json, Value};

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
            "workspace": {"type":["string","null"], "description":"development workspace name for parity"}
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
                !["workflow", "case", "variable", "study", "workspace"].contains(&k.as_str())
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
                if vars.is_empty() {
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
                if let Some(study) = opt_str(args, "study") {
                    let (mut step, data) = collect(
                        "study",
                        "Collect Study baseline, objective and parameter ranges",
                        "study_summary",
                        json!({"study":study}),
                        ctx,
                    );
                    if data.as_ref().is_some_and(|v| {
                        v["baseline_objective"].is_null() || v["best_objective"].is_null()
                    }) {
                        step["status"] = json!("missing");
                        step["missing"] = json!(["Baseline and best objectives are both required; Study data are incomplete."]);
                    }
                    steps.push(step);
                } else {
                    steps.push(pending("study", "checkdata", "Identify the Study", "Study directory is required to inspect objectives and bounds.", json!({"tool":"study_summary","arguments":{"study":null},"requires":["study"]})));
                }
                steps.push(pending("bounds", "localise", "Check parameter bounds, identifiability and objective tradeoffs", "Verify the Study includes this case and each parameter is active in its mode/build. A boundary hit is not proof of compensation or causality; inspect repeated candidates, sensitivity, baseline and other target variables.", Value::Null));
                steps.push(pending("held_out", "validate", "Verify held-out time/site performance and physical closure", "Independent validation, observation alignment and closure evidence are required before adopting tuned parameters.", Value::Null));
                hypotheses.push(json!({"status":"unverified","candidate":"Weak identifiability or compensating parameters","basis":"Bounds or improved calibration fit alone cannot verify this candidate."}));
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
        let confirmed: Vec<_> = steps.iter().filter(|s| s["status"] == "collected")
            .map(|s| json!({"step":s["id"],"fact":"Source evidence collected; interpretation remains subject to required checks."})).collect();
        Ok(
            json!({"version":1,"workflow":workflow,"inputs":{"case":case,"variable":args["variable"],"study":args["study"],"workspace":args["workspace"]},
            "phases":["checkdata","localise","validate","report"],"status":"incomplete",
            "steps":steps,"confirmed_facts":confirmed,"hypotheses":hypotheses,
            "minimal_experiment":{"status":"proposed_not_executed","description":experiment}}),
        )
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
