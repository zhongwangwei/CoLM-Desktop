//! A 级只读工具：读算例、看指标、查参数、看 Study、搜文档、查环境。不改任何东西，不需审批。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{object, opt_str, req_str, Tier, Tool, ToolContext, MAX_RESULT_CHARS};

#[path = "knowledge.rs"]
mod knowledge;

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ListCases),
        Box::new(ReadCaseConfig),
        Box::new(ExplainParameter),
        Box::new(RunStatus),
        Box::new(Metrics),
        Box::new(SeriesStats),
        Box::new(CompareCases),
        Box::new(StudySummary),
        Box::new(HybridInfo),
        Box::new(HybridCheck),
        Box::new(SearchDocs),
        Box::new(EnvironmentDoctor),
    ]
}

fn nullable_string(description: &str) -> Value {
    json!({ "type": ["string", "null"], "description": description })
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// 算例目录：参数给的路径，或它下面的 `case.nml` 所在目录。
fn case_dir(ctx: &ToolContext, args: &Value, name: &str) -> Result<PathBuf> {
    let dir = ctx.resolve(req_str(args, name)?);
    let dir = if dir.ends_with("case.nml") {
        dir.parent().map(Path::to_path_buf).unwrap_or(dir)
    } else {
        dir
    };
    if !dir.join("case.nml").is_file() {
        bail!("{} has no case.nml", dir.display());
    }
    Ok(dir)
}

fn read_document(path: &Path) -> Result<colm_namelist::Document> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    colm_namelist::parse(&text).with_context(|| format!("cannot parse {}", path.display()))
}

fn logical(document: &colm_namelist::Document, name: &str) -> bool {
    match document.get(name) {
        Some(colm_namelist::Value::Bool(value)) => *value,
        _ => matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Logical(true))
        ),
    }
}

fn land_mode(document: &colm_namelist::Document) -> &'static str {
    if logical(document, "DEF_USE_PC") {
        "pc"
    } else if logical(document, "DEF_USE_PFT") {
        "pft"
    } else {
        "lct"
    }
}

/// 文件末尾若干行。
fn tail(path: &Path, lines: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let all: Vec<&str> = text.lines().collect();
    Some(all[all.len().saturating_sub(lines)..].join("\n"))
}

struct ListCases;

impl Tool for ListCases {
    fn name(&self) -> &'static str {
        "list_cases"
    }
    fn description(&self) -> &'static str {
        "List the CoLM cases under a directory (default: the project directory): name, path, land-surface mode (lct/pft/pc), spatial or single point, whether outputs, metrics and an AI parameterization exist."
    }
    fn parameters(&self) -> Value {
        object(
            json!({ "root": nullable_string("directory to scan; null for the project directory") }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let root =
            opt_str(args, "root").map_or_else(|| ctx.project_root.clone(), |r| ctx.resolve(r));
        let mut cases = Vec::new();
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if dir.join("case.nml").is_file() {
                let nml = dir.join("case.nml");
                let document = read_document(&nml).ok();
                cases.push(json!({
                    "name": dir.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "dir": dir.display().to_string(),
                    "mode": document.as_ref().map(land_mode),
                    "spatial": colm_case::is_spatial_case(&nml).ok(),
                    "has_outputs": dir.join("out").is_dir(),
                    "has_metrics": dir.join("metrics.json").is_file(),
                    "has_hybrid": dir.join("hybrid.toml").is_file(),
                }));
                continue;
            }
            if depth >= 3 {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let hidden = path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'));
                if path.is_dir() && !hidden {
                    stack.push((path, depth + 1));
                }
            }
        }
        cases.sort_by(|a, b| a["dir"].as_str().cmp(&b["dir"].as_str()));
        Ok(json!({ "root": root.display().to_string(), "cases": cases }))
    }
}

struct ReadCaseConfig;

impl Tool for ReadCaseConfig {
    fn name(&self) -> &'static str {
        "read_case_config"
    }
    fn description(&self) -> &'static str {
        "Read namelist settings of a case (case.nml, and its forcing namelist). Give exact field names, or a case-insensitive substring filter such as 'VMAX' or 'simulation_time'. Returns at most 200 fields."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "case": string("case directory"),
            "names": { "type": ["array", "null"], "items": { "type": "string" }, "description": "exact field names, or null" },
            "filter": nullable_string("case-insensitive substring of field names, or null for all"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args, "case")?;
        let names: Vec<String> = args["names"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let filter = opt_str(args, "filter").map(str::to_ascii_lowercase);
        let wanted = |name: &str| {
            (names.is_empty() && filter.is_none())
                || names.iter().any(|n| n.eq_ignore_ascii_case(name))
                || filter
                    .as_ref()
                    .is_some_and(|f| name.to_ascii_lowercase().contains(f))
        };
        let mut out = BTreeMap::new();
        let case = read_document(&dir.join("case.nml"))?;
        let mut files = vec![("case.nml".to_owned(), case.clone())];
        if let Some(colm_namelist::Value::Str(forcing)) = case.get("DEF_forcing_namelist") {
            let path = ctx.resolve(forcing);
            if let Ok(document) = read_document(&path) {
                files.push(("forcing.nml".to_owned(), document));
            }
        }
        let mut count = 0usize;
        for (file, document) in &files {
            let mut fields = BTreeMap::new();
            for name in document.paths() {
                if wanted(&name) && count < 200 {
                    if let Some(value) = document.get(&name) {
                        fields.insert(name.clone(), value.to_string());
                        count += 1;
                    }
                }
            }
            out.insert(file.clone(), fields);
        }
        Ok(json!({ "case": dir.display().to_string(), "fields": out, "mode": land_mode(&case) }))
    }
}

struct ExplainParameter;

impl Tool for ExplainParameter {
    fn name(&self) -> &'static str {
        "explain_parameter"
    }
    fn description(&self) -> &'static str {
        "Explain a CoLM namelist field or parameter: meaning, unit, default, valid range, scope, whether it can be calibrated, and where it is defined in the source. Optionally give a case to also see its current value there."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": string("field or parameter name, e.g. DEF_PFT_VMAX25 or DEF_USE_PC"),
            "case": nullable_string("case directory to read the current value from, or null"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let name = req_str(args, "name")?;
        let descriptor = colm_case::parameters::find(name).map(|d| {
            json!({
                "id": d.id, "key": d.raw_key, "label_zh": d.label_zh, "label_en": d.label_en,
                "section": d.section, "scope": d.scope, "unit": d.unit, "default": d.default,
                "default_provider": d.default_provider, "validation": d.validation,
                "calibration_eligible": d.calibration_eligible, "structural": d.structural_parameter,
                "visibility": d.visibility, "activation": d.activation,
                "doc_zh": d.doc_zh, "doc_en": d.doc_en, "source": d.source_location,
            })
        });
        let field = colm_schema::find(name).map(|f| {
            json!({
                "name": f.name, "kind": format!("{:?}", f.kind), "default": format!("{:?}", f.default),
                "doc": f.doc, "group": f.group, "arity": f.arity,
            })
        });
        if descriptor.is_none() && field.is_none() {
            bail!("{name} is neither a namelist field nor a catalogued parameter");
        }
        let parameter = colm_case::parameters::find(name);
        let key = parameter.map_or(name, |p| p.raw_key.as_str());
        let mut current = None;
        let mut applicability = json!({
            "status": "unknown", "reason": "no case supplied",
            "calibration_eligible_for_case": null,
        });
        if opt_str(args, "case").is_some() {
            let dir = case_dir(ctx, args, "case")?;
            let document = read_document(&dir.join("case.nml"))?;
            current = document.get(key).map(ToString::to_string);
            applicability = parameter_applicability(ctx, &dir, &document, key)?;
            if applicability["status"] == "active" {
                applicability["calibration_eligible_for_case"] =
                    json!(parameter.map(|p| p.calibration_eligible));
            } else if applicability["status"] == "inactive" {
                applicability["calibration_eligible_for_case"] = json!(false);
            }
        }
        Ok(
            json!({ "parameter": descriptor, "field": field, "current_value": current,
            "applicability": applicability }),
        )
    }
}

fn parameter_applicability(
    ctx: &ToolContext,
    dir: &Path,
    document: &colm_namelist::Document,
    key: &str,
) -> Result<Value> {
    let unknown = |reason: &str| {
        json!({ "status": "unknown", "reason": reason,
        "calibration_eligible_for_case": null })
    };
    if colm_case::tuning::find(key)?.is_none() {
        return Ok(unknown("no case activity validator for this parameter; catalog visibility and activation are metadata, not proof of runtime use"));
    }
    let manifest_path = ctx.kernel_dir.as_ref().map(|p| p.join("manifest.json"));
    let manifest = manifest_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let macros = manifest
        .as_ref()
        .filter(|m| m["schema"] == 1)
        .and_then(|m| m["macros"].as_array())
        .and_then(|a| {
            a.iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        });
    let Some(macros) = macros else {
        return Ok(unknown("selected kernel manifest macros are unavailable"));
    };
    let landtype = match document.get("SITE_landtype") {
        Some(colm_namelist::Value::Int(value)) if *value > 0 => Some(*value),
        _ => None,
    };
    if macros.iter().any(|m| m == "SinglePoint")
        && (landtype.is_none() || document.get("SITE_fsitedata").is_some())
    {
        return Ok(unknown("single-point runtime land type may be inherited from site data; resolve it before judging activity"));
    }
    let result = colm_case::tuning::validate_case_parameter_activity(
        &dir.join("case.nml"),
        &[key.to_owned()],
        &macros,
        landtype,
    );
    let reason = result.err().map(|e| e.to_string());
    let status = match reason.as_deref() {
        None => "active",
        Some(reason)
            if reason.ends_with("is inactive for the current case/kernel configuration") =>
        {
            "inactive"
        }
        Some(_) => "unknown",
    };
    Ok(json!({
        "status": status,
        "reason": reason,
        "mode": land_mode(document), "kernel_manifest": manifest_path,
        "kernel_macros": macros, "landtype": landtype,
        "validator": "crates/colm-case/src/tuning.rs::validate_case_parameter_activity",
        "qualification": "activity according to the registered case/kernel guards, not proof of output sensitivity",
        "calibration_eligible_for_case": null,
    }))
}

struct RunStatus;

impl Tool for RunStatus {
    fn name(&self) -> &'static str {
        "run_status"
    }
    fn description(&self) -> &'static str {
        "Status of a case's last run: per-stage fingerprints and whether stages need rerunning (stages.json), the tail of each stage log, and lines that look like errors."
    }
    fn parameters(&self) -> Value {
        object(json!({ "case": string("case directory") }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args, "case")?;
        let stages = std::fs::read_to_string(dir.join("stages.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let mut logs = BTreeMap::new();
        let mut errors = Vec::new();
        for log in ["mksrfdata.log", "mkinidata.log", "colm.log", "run.out"] {
            let path = dir.join(log);
            if let Some(text) = tail(&path, 40) {
                for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
                    let lower = line.to_ascii_lowercase();
                    if (lower.contains("error")
                        || lower.contains("fatal")
                        || lower.contains("panicked"))
                        && errors.len() < 30
                    {
                        errors.push(format!("{log}: {}", line.trim()));
                    }
                }
                logs.insert(log, text);
            }
        }
        Ok(
            json!({ "case": dir.display().to_string(), "stages": stages, "log_tails": logs, "error_lines": errors }),
        )
    }
}

struct Metrics;

impl Tool for Metrics {
    fn name(&self) -> &'static str {
        "metrics"
    }
    fn description(&self) -> &'static str {
        "Skill metrics of a case against an observation file (RMSE, bias, correlation, R2, NSE, KGE, model and observed means) per variable. Optional time window in Unix seconds."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "case": string("case directory"),
            "obs": string("observation NetCDF file (e.g. a PLUMBER2 *_Flux.nc)"),
            "from": { "type": ["integer", "null"], "description": "window start, Unix seconds, or null" },
            "to": { "type": ["integer", "null"], "description": "window end, Unix seconds, or null" },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args, "case")?;
        let obs = ctx.resolve(req_str(args, "obs")?);
        let dir_s = dir.display().to_string();
        let obs_s = obs.display().to_string();
        let mut cli = vec![
            "metrics",
            dir_s.as_str(),
            "--obs",
            obs_s.as_str(),
            "--json",
            "1",
            "--summary-only",
            "1",
        ];
        let from = args["from"].as_i64().map(|v| v.to_string());
        let to = args["to"].as_i64().map(|v| v.to_string());
        if let Some(from) = &from {
            cli.extend(["--from", from.as_str()]);
        }
        if let Some(to) = &to {
            cli.extend(["--to", to.as_str()]);
        }
        let rows = ctx.cli_json(&cli)?;
        let keep = [
            "name",
            "units",
            "n",
            "rmse",
            "bias",
            "correlation",
            "r2",
            "nse",
            "kge",
            "model_mean",
            "obs_mean",
            "model_sd",
            "obs_sd",
        ];
        let rows: Vec<Value> = rows
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| {
                let mut out = serde_json::Map::new();
                for key in keep {
                    if let Some(value) = row.get(key) {
                        out.insert(key.to_owned(), value.clone());
                    }
                }
                Value::Object(out)
            })
            .collect();
        Ok(json!({ "case": dir_s, "obs": obs_s, "metrics": rows }))
    }
}

struct SeriesStats;

impl Tool for SeriesStats {
    fn name(&self) -> &'static str {
        "series_stats"
    }
    fn description(&self) -> &'static str {
        "Statistics of history variables of a case (e.g. f_fsena, f_fevpa, f_assim): count, NaN count, mean, min, max, and monthly means. Use history names (f_*)."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "case": string("case directory"),
            "vars": { "type": "array", "items": { "type": "string" }, "description": "history variable names" },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args, "case")?;
        let vars: Vec<&str> = args["vars"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if vars.is_empty() {
            bail!("give at least one history variable");
        }
        let dir_s = dir.display().to_string();
        let joined = vars.join(",");
        let series = ctx.cli_json(&["series", &dir_s, "--vars", &joined])?;
        let times: Vec<i64> = series["time"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        let mut stats = BTreeMap::new();
        for var in vars {
            let values: Vec<Option<f64>> = series["vars"][var]
                .as_array()
                .map(|a| a.iter().map(Value::as_f64).collect())
                .unwrap_or_default();
            stats.insert(var.to_owned(), summarize(&times, &values));
        }
        Ok(json!({ "case": dir_s, "records": times.len(), "stats": stats }))
    }
}

/// 一条序列的统计；月份按 Unix 时间换算的 UTC 月份。
pub(crate) fn summarize(times: &[i64], values: &[Option<f64>]) -> Value {
    let finite: Vec<f64> = values
        .iter()
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    let nan = values.len() - finite.len();
    let mut monthly: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for (time, value) in times.iter().zip(values) {
        if let Some(value) = value.filter(|v| v.is_finite()) {
            let (year, month) = year_month(*time);
            let entry = monthly.entry(format!("{year:04}-{month:02}")).or_default();
            entry.0 += value;
            entry.1 += 1;
        }
    }
    let mean = if finite.is_empty() {
        None
    } else {
        Some(finite.iter().sum::<f64>() / finite.len() as f64)
    };
    json!({
        "count": values.len(),
        "nan_or_missing": nan,
        "mean": mean,
        "min": finite.iter().copied().fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.min(v)))),
        "max": finite.iter().copied().fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))),
        "monthly_mean": monthly.into_iter().map(|(k, (s, n))| (k, s / n as f64)).collect::<BTreeMap<_, _>>(),
    })
}

/// Unix 秒 → (年, 月)，UTC。
pub(crate) fn year_month(seconds: i64) -> (i64, u32) {
    let days = seconds.div_euclid(86_400);
    // Howard Hinnant 的 civil_from_days。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32)
}

struct CompareCases;

impl Tool for CompareCases {
    fn name(&self) -> &'static str {
        "compare_cases"
    }
    fn description(&self) -> &'static str {
        "Compare two cases: namelist fields that differ (case.nml), whether each has an AI parameterization, and, when both have metrics.json, the metric differences per variable."
    }
    fn parameters(&self) -> Value {
        object(json!({ "a": string("first case directory"), "b": string("second case directory") }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let a = case_dir(ctx, args, "a")?;
        let b = case_dir(ctx, args, "b")?;
        let da = read_document(&a.join("case.nml"))?;
        let db = read_document(&b.join("case.nml"))?;
        let mut names = da.paths();
        names.extend(db.paths());
        names.sort();
        names.dedup();
        // 路径类字段（各自的目录）几乎总是不同，单独列出，不混进实质差异。
        let path_like = |n: &str| {
            let n = n.to_ascii_lowercase();
            n.contains("dir_")
                || n.contains("fsitedata")
                || n.contains("namelist")
                || n == "def_case_name"
        };
        let (mut differ, mut paths) = (BTreeMap::new(), Vec::new());
        for name in names {
            let va = da.get(&name).map(ToString::to_string);
            let vb = db.get(&name).map(ToString::to_string);
            if va != vb {
                if path_like(&name) {
                    paths.push(name);
                } else {
                    differ.insert(name, json!({ "a": va, "b": vb }));
                }
            }
        }
        let metrics = |dir: &Path| -> Option<BTreeMap<String, Value>> {
            let rows: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("metrics.json")).ok()?)
                    .ok()?;
            Some(
                rows.as_array()?
                    .iter()
                    .filter_map(|r| Some((r["name"].as_str()?.to_owned(), r.clone())))
                    .collect(),
            )
        };
        let metric_diff = match (metrics(&a), metrics(&b)) {
            (Some(ma), Some(mb)) => Some(
                ma.iter()
                    .filter_map(|(name, ra)| {
                        let rb = mb.get(name)?;
                        let pick = |r: &Value, k: &str| r[k].as_f64();
                        Some((
                            name.clone(),
                            json!({
                                "kge": [pick(ra, "kge"), pick(rb, "kge")],
                                "rmse": [pick(ra, "rmse"), pick(rb, "rmse")],
                                "bias": [pick(ra, "bias"), pick(rb, "bias")],
                            }),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>(),
            ),
            _ => None,
        };
        Ok(json!({
            "a": a.display().to_string(), "b": b.display().to_string(),
            "differing_fields": differ, "differing_path_fields": paths,
            "hybrid": [a.join("hybrid.toml").is_file(), b.join("hybrid.toml").is_file()],
            "metrics_a_vs_b": metric_diff,
        }))
    }
}

struct StudySummary;

impl Tool for StudySummary {
    fn name(&self) -> &'static str {
        "study_summary"
    }
    fn description(&self) -> &'static str {
        "Summarize a tuning or uncertainty Study: status, best member and objective versus the physics-only baseline, the best parameter values and where they sit in their search ranges (values at a bound suggest the parameter is compensating for another bias), and warnings."
    }
    fn parameters(&self) -> Value {
        object(json!({ "study": string("Study directory (…/.colm/studies/s-…)") }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = ctx.resolve(req_str(args, "study")?);
        let dir_s = dir.display().to_string();
        let status = ctx.cli_json(&["study-status", &dir_s])?;
        let spec = &status["manifest"]["spec"];
        let state = &status["state"];
        let best = state["best_member"].as_str();
        let baseline = state["candidates"]["m000000"]["calibration"].as_f64();
        let members = status["manifest"]["members"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let best_params = best.and_then(|id| {
            members
                .iter()
                .find(|m| m["id"] == id)
                .map(|m| m["parameters"].clone())
        });
        // 最优值在搜索区间里的位置：0 下界，1 上界。
        let mut positions = Vec::new();
        if let Some(params) = best_params.as_ref().and_then(Value::as_object) {
            for p in spec["parameters"].as_array().into_iter().flatten() {
                let (lo, hi) = (p["sample_min"].as_f64(), p["sample_max"].as_f64());
                let name = p["name"].as_str().unwrap_or_default();
                let key = params
                    .keys()
                    .find(|k| k.starts_with(name))
                    .cloned()
                    .unwrap_or_else(|| name.to_owned());
                if let (Some(lo), Some(hi), Some(v)) =
                    (lo, hi, params.get(&key).and_then(Value::as_f64))
                {
                    let at = (v - lo) / (hi - lo);
                    positions.push(json!({
                        "parameter": key, "value": v, "range": [lo, hi], "position": at,
                        "at_bound": !(0.02..=0.98).contains(&at),
                    }));
                }
            }
        }
        Ok(json!({
            "study": dir_s, "kind": spec["kind"], "method": spec["method"],
            "status": state["status"], "generation": state["generation"],
            "base_cases": spec["base_cases"], "targets": spec["targets"],
            "best_member": best, "best_objective": state["best_objective"],
            "baseline_objective": baseline, "best_parameters": positions,
            "hybrid": spec.get("hybrid"), "warnings": state["warnings"],
        }))
    }
}

struct HybridInfo;

impl Tool for HybridInfo {
    fn name(&self) -> &'static str {
        "hybrid_info"
    }
    fn description(&self) -> &'static str {
        "The AI parameterization (hybrid model) installed on a case: slots, features, outputs, whether the model file passes its checksum, whether climate features exist."
    }
    fn parameters(&self) -> Value {
        object(json!({ "case": string("case directory") }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args, "case")?;
        ctx.cli_json(&["hybrid-info", &dir.display().to_string()])
    }
}

struct HybridCheck;

impl Tool for HybridCheck {
    fn name(&self) -> &'static str {
        "hybrid_check"
    }
    fn description(&self) -> &'static str {
        "Dry-run a case's AI parameterization without simulating: rows affected, feature and output ranges, and rows outside the training range. Needs the case's preprocessing to have run."
    }
    fn parameters(&self) -> Value {
        object(json!({ "case": string("case directory") }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args, "case")?;
        let kernel = ctx
            .kernel_dir
            .as_ref()
            .context("no kernel is selected; choose one in Basic setup")?;
        ctx.cli_json(&[
            "hybrid-check",
            &dir.display().to_string(),
            "--kernel",
            &kernel.display().to_string(),
        ])
    }
}

struct SearchDocs;

impl Tool for SearchDocs {
    fn name(&self) -> &'static str {
        "search_docs"
    }
    fn description(&self) -> &'static str {
        "Search project documentation with file/line citations, source content identity and knowledge_checks. Changed source bindings mark knowledge needs_review, withhold stored assertions, and return freshly read source excerpts and read_file follow-ups. Unbound/unknown documents cannot establish current-version behavior."
    }
    fn parameters(&self) -> Value {
        object(json!({ "query": string("case-insensitive text to find") }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let query = req_str(args, "query")?.to_lowercase();
        if query.len() > 1024 {
            bail!("documentation query exceeds 1024 bytes");
        }
        let root = ctx
            .docs_root
            .as_ref()
            .context("no documentation directory is configured")?;
        let provenance = documentation_provenance(ctx);
        let bindings = knowledge::Bindings::load(root, &provenance["application_source"]);
        let mut knowledge_checks = BTreeMap::new();
        let mut hits = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(root)
            .with_context(|| format!("cannot read {}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        files.sort();
        'outer: for file in files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let lines: Vec<_> = text.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if line.to_lowercase().contains(&query) {
                    let name = file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let check = knowledge_checks
                        .entry(name.clone())
                        .or_insert_with(|| bindings.check(&name, &text, &query));
                    let stale = check["status"] == "needs_review";
                    hits.push(json!({
                        "file": file.file_name().map(|n| n.to_string_lossy().into_owned()),
                        "path": file.display().to_string(),
                        "line": index + 1,
                        "context_start_line": index.saturating_sub(3) + 1,
                        "context": if stale { vec!["Stored knowledge withheld: source/document changed. Use knowledge_checks fresh_source_evidence and inspect relevant current code before answering.".to_owned()] } else { lines[index.saturating_sub(3)..(index + 5).min(lines.len())]
                            .iter().map(|s| s.chars().take(400).collect::<String>()).collect::<Vec<_>>()
                        },
                        "text": if stale { "Stored assertion requires review".to_owned() } else {line.chars().take(400).collect::<String>()},
                        "knowledge_status":check["status"],
                        "may_support_current_answer":check["may_support_current_answer"],
                    }));
                    if hits.len() >= 60 {
                        break 'outer;
                    }
                }
            }
        }
        let mut result = json!({ "query": query, "matches": hits, "provenance": provenance, "knowledge_checks":knowledge_checks,
            "truncated": hits.len() >= 60 });
        while serde_json::to_string(&result)?.chars().count() > MAX_RESULT_CHARS {
            result["matches"]
                .as_array_mut()
                .context("matches must be an array")?
                .pop()
                .context("documentation provenance exceeds result limit")?;
            let remaining: std::collections::BTreeSet<String> = result["matches"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|hit| hit["file"].as_str().map(str::to_owned))
                .collect();
            result["knowledge_checks"]
                .as_object_mut()
                .unwrap()
                .retain(|file, _| remaining.contains(file));
            result["truncated"] = json!(true);
        }
        Ok(result)
    }
}

fn documentation_provenance(ctx: &ToolContext) -> Value {
    json!({
        "agent_package_version": env!("CARGO_PKG_VERSION"),
        "docs_root": ctx.docs_root,
        "application_source": ctx.cli_json(&["ws-source-info", "--source", "app"])
            .unwrap_or_else(|error| json!({ "version_status": "unknown", "revision": null,
                "tag": null, "dirty": null, "reason": error.to_string() })),
        "verification": "Use search_code/read_file with name=null for the application's source; documentation and package version alone do not establish its revision or the selected simulation kernel revision.",
    })
}

struct EnvironmentDoctor;

impl Tool for EnvironmentDoctor {
    fn name(&self) -> &'static str {
        "environment_doctor"
    }
    fn description(&self) -> &'static str {
        "Report the local toolchain and setup: colm-cli and kernel paths, and whether gfortran, mpif90, nf-config, cargo, rustc, git, rg and the T7920 runner are installed (with versions)."
    }
    fn parameters(&self) -> Value {
        object(json!({}))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, _args: &Value, ctx: &ToolContext) -> Result<Value> {
        let probe = |program: &str, arg: &str| -> Value {
            match Command::new(program).arg(arg).output() {
                Ok(out) => {
                    let text = String::from_utf8_lossy(if out.stdout.is_empty() {
                        &out.stderr
                    } else {
                        &out.stdout
                    })
                    .into_owned();
                    json!({ "found": true, "version": text.lines().next().unwrap_or_default().trim() })
                }
                Err(_) => json!({ "found": false }),
            }
        };
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let runner = home.join(".local/bin/run-7920");
        Ok(json!({
            "colm_cli": ctx.cli.display().to_string(),
            "colm_cli_exists": ctx.cli.is_file(),
            "kernel_dir": ctx.kernel_dir.as_ref().map(|p| p.display().to_string()),
            "project_root": ctx.project_root.display().to_string(),
            "tools": {
                "gfortran": probe("gfortran", "--version"),
                "mpif90": probe("mpif90", "--version"),
                "nf-config": probe("nf-config", "--version"),
                "cargo": probe("cargo", "--version"),
                "rustc": probe("rustc", "--version"),
                "git": probe("git", "--version"),
                "rg": probe("rg", "--version"),
            },
            "remote_runner_run_7920": runner.is_file(),
        }))
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod read_tests;
