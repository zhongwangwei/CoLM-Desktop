//! A 级只读工具：读算例、看指标、查参数、看 Study、搜文档、查环境。不改任何东西，不需审批。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{object, opt_str, req_str, Tier, Tool, ToolContext, MAX_RESULT_CHARS};
use crate::retrieval;

#[path = "knowledge.rs"]
pub(crate) mod knowledge;

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
        let current = ctx.selected_context()?;
        let ctx = &current;
        let dir = case_dir(ctx, args, "case")?;
        let effective = ctx.effective_case(&dir)?;
        let dir = &effective.dir;
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
            let path = if Path::new(forcing).is_absolute() {
                PathBuf::from(forcing)
            } else {
                dir.join(forcing)
            };
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
        let overrides = ctx
            .model_context
            .as_ref()
            .and_then(|model| model.model["overlay"]["parameters"].as_array())
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["raw_key"].as_str().is_some_and(wanted))
                    .cloned()
                    .collect::<Vec<_>>()
            });
        Ok(
            json!({ "case": effective.original, "fields": out, "mode": land_mode(&case),
            "model_version":ctx.model_context.as_ref().map(|model| &model.model["version"]),"model_overrides":overrides }),
        )
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
        let selected = ctx.selected_context()?;
        let ctx = &selected;
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
            bail!("{name} is neither a namelist field nor a catalogued parameter. Read read_case_config for an existing case or search_code to discover exact field names; do not guess aliases. Spin-up uses DEF_simulation_time%spinup_year/month/day/sec/repeat; spinup_year is a cutoff year, not a duration. Use the visible case settings returned by ui_state to edit spin-up.");
        }
        let parameter = colm_case::parameters::find(name);
        let key = parameter.map_or(name, |p| p.raw_key.as_str());
        let mut current = None;
        let mut applicability = json!({
            "status": "unknown", "reason": "no case supplied",
            "calibration_eligible_for_case": null,
        });
        let supplied = opt_str(args, "case")
            .map(|path| ctx.resolve(path))
            .or_else(|| {
                ctx.model_context
                    .as_ref()
                    .and_then(|model| model.source_case.clone())
            });
        if let Some(original) = supplied {
            let original = if original.ends_with("case.nml") {
                original
                    .parent()
                    .context("case has no parent")?
                    .to_path_buf()
            } else {
                original
            };
            let effective = ctx.effective_case(&original)?;
            let dir = &effective.dir;
            let document = read_document(&dir.join("case.nml"))?;
            current = document.get(key).map(ToString::to_string);
            applicability = parameter_applicability(ctx, dir, &document, key)?;
            if let Some(model) = &ctx.model_context {
                let application = ctx.cli_json(&["ws-source-info", "--source", "app"]).ok();
                if application.as_ref().map(|source| &source["content_sha256"])
                    != Some(&model.model["code"]["source_sha256"])
                {
                    applicability = json!({"status":"unknown","reason":"app catalogue/activity validator does not establish applicability in this frozen source; inspect the selected source with search_code/read_file","calibration_eligible_for_case":null});
                }
            }
            if applicability["status"] == "active" {
                applicability["calibration_eligible_for_case"] =
                    json!(parameter.map(|p| p.calibration_eligible));
            } else if applicability["status"] == "inactive" {
                applicability["calibration_eligible_for_case"] = json!(false);
            }
        }
        Ok(
            json!({ "parameter": descriptor, "field": field, "current_value": current,
            "applicability": applicability,"catalog_source":"application_package",
            "model_version":ctx.model_context.as_ref().map(|model| &model.model["version"]),
            "model_overrides":ctx.model_context.as_ref().and_then(|model| model.model["overlay"]["parameters"].as_array())
                .map(|rows| rows.iter().filter(|row| row["raw_key"].as_str().is_some_and(|raw|raw.eq_ignore_ascii_case(key))).cloned().collect::<Vec<_>>()) }),
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
        let spec = status["manifest"]["spec"].clone();
        let state = status["state"].clone();
        let best = state["best_member"].as_str();
        let members = status["manifest"]["members"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // 当前应用的选择只有界面桥知道；CLI 的 evidence 里 current_selection 永远是空。
        // 刷新失败时仍返回 Study 数据，只把身份对比标成 unknown。
        let selected = ctx.selected_context();
        let selection_error = selected.as_ref().err().map(|error| format!("{error:#}"));
        let evidence =
            merge_study_evidence(&status, selected.as_ref().ok(), selection_error.as_deref());
        // 旧兼容字段 baseline_objective 优先用 evidence 里按 baseline 标记定位的成绩：
        // evidence 存在但 baseline 不可用时保持 null，不退回按 m000000 猜分数；
        // 只有旧 CLI 完全没有 evidence 时才走旧的 candidates 读取。
        let baseline = if evidence["status"] == "missing" {
            state["candidates"]["m000000"]["calibration"].as_f64()
        } else {
            evidence["comparison"]["baseline"]["calibration"]["score"].as_f64()
        };
        // 最优参数优先由证据供数（读全部样本代、按 Log 尺度与带索引的键算位置）；
        // 旧引擎快照没有 evidence 时退回 manifest.members 快照。
        let positions = evidence_study_parameters(&evidence)
            .unwrap_or_else(|| legacy_best_parameters(&spec, best, &members));
        let mut result = json!({
            "study": dir_s, "kind": spec["kind"], "method": spec["method"],
            "status": state["status"], "generation": state["generation"],
            "base_cases": spec["base_cases"], "targets": spec["targets"],
            "best_member": best, "best_objective": state["best_objective"],
            "baseline_objective": baseline, "best_parameters": positions,
            "hybrid": spec.get("hybrid"), "warnings": state["warnings"],
            "evidence": evidence,
        });
        fit_study_summary(&mut result);
        Ok(result)
    }
}

/// 旧 CLI（没有 `evidence`）时的最优参数：manifest.members 只是建研究时的快照，
/// 后续代成员不在里面，线性位置对 Log 尺度也不对 —— 所以只作为 fallback。
fn legacy_best_parameters(spec: &Value, best: Option<&str>, members: &[Value]) -> Value {
    let best_params = best.and_then(|id| {
        members
            .iter()
            .find(|m| m["id"] == id)
            .map(|m| m["parameters"].clone())
    });
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
    json!(positions)
}

/// 把 CLI evidence 合并成助手读的形状：补当前选择、身份对比；evidence 缺失时给显式原因。
fn merge_study_evidence(
    status: &Value,
    selected: Option<&ToolContext>,
    selection_error: Option<&str>,
) -> Value {
    let Some(evidence) = status.get("evidence").filter(|value| value.is_object()) else {
        return json!({
            "status": "missing",
            "reason": "colm-cli study-status returned no evidence (older CLI or remote engine snapshot); best_parameters fall back to the manifest.members snapshot",
        });
    };
    let mut evidence = evidence.clone();
    let current_selection = study_current_selection(selected, selection_error);
    let comparison = compare_study_identity(
        &evidence["model_identity"]["study_record"],
        &current_selection,
    );
    evidence["model_identity"]["current_selection"] = current_selection;
    evidence["model_identity"]["comparison"] = comparison;
    evidence
}

/// 当前界面选择：没有冻结模型上下文时应用跑的是官方版，身份未知而不是"匹配"。
fn study_current_selection(selected: Option<&ToolContext>, selection_error: Option<&str>) -> Value {
    let Some(error) = selection_error else {
        let Some(ctx) = selected else {
            return json!({
                "status": "unknown",
                "reason": "the application selection is unavailable in this context",
            });
        };
        let Some(model) = ctx.model_context.as_ref() else {
            return json!({
                "status": "unknown",
                "reason": "no frozen model is selected in the application (the official version is in use); the Study's frozen identity cannot be compared",
            });
        };
        let version = &model.model["version"];
        let id = version["id"].as_str();
        let revision = version["revision"].as_u64();
        let code = model.model["code"]["source_sha256"].as_str();
        return match (id, revision, code) {
            (Some(id), Some(revision), Some(code)) => json!({
                "status": "frozen",
                "version": {"id": id, "revision": revision},
                "code_source_sha256": code,
            }),
            _ => json!({
                "status": "unknown",
                "reason": "the selected model context has no complete version id, revision and source hash",
            }),
        };
    };
    json!({
        "status": "unknown",
        "reason": format!(
            "cannot refresh the application selection: {}",
            error.chars().take(240).collect::<String>()
        ),
    })
}

/// 研究身份只能用研究记录里冻结的版本；同名不算匹配，必须 id + revision + code.source_sha256 全等。
fn compare_study_identity(record: &Value, current: &Value) -> Value {
    if current["status"].as_str() != Some("frozen") {
        return json!({
            "status": "unknown",
            "matches": null,
            "basis": "no comparable current frozen selection",
            "reason": current["reason"],
        });
    }
    if record["status"].as_str() != Some("pinned") {
        return json!({
            "status": "unknown",
            "matches": null,
            "basis": "the Study record has no frozen revision identity (official version or unreadable record)",
            "reason": record["reason"],
        });
    }
    let study_version = &record["version"];
    if !study_version.is_object() {
        return json!({
            "status": "unknown",
            "matches": null,
            "basis": "the pinned Study record was written before the frozen revision identity was recorded",
            "reason": "the Study record has a model name and head but no version identity",
        });
    }
    let current_version = &current["version"];
    let study_id = study_version["id"].as_str();
    let current_id = current_version["id"].as_str();
    let version_matches = study_id.is_some()
        && study_id == current_id
        && study_version["revision"].as_u64().is_some()
        && study_version["revision"].as_u64() == current_version["revision"].as_u64();
    let code_matches = record["code_source_sha256"]
        .as_str()
        .zip(current["code_source_sha256"].as_str())
        .is_some_and(|(study, current)| study == current);
    let matches = version_matches && code_matches;
    json!({
        "status": if matches { "matches" } else { "mismatch" },
        "matches": matches,
        "basis": "compared the frozen version id, revision and code.source_sha256; an equal display name alone is not a match",
        "study_version": {"id": study_version["id"], "revision": study_version["revision"]},
        "current_version": {"id": current_version["id"], "revision": current_version["revision"]},
    })
}

/// 证据里的最优参数：每个维度带 range/position/position_scale/at_bound。
/// 没有证据（旧 CLI）或没有最优成员时返回 None，让调用方用旧 fallback。
fn evidence_study_parameters(evidence: &Value) -> Option<Value> {
    let parameters = &evidence["parameters"];
    let values = parameters["best"]["values"].as_array()?;
    let dims = parameters["dims"].as_array()?;
    let range = |key: &str| {
        dims.iter().find(|dim| dim["key"] == key).and_then(|dim| {
            let low = dim["range"][0].as_f64()?;
            let high = dim["range"][1].as_f64()?;
            Some((low, high))
        })
    };
    let positions = values
        .iter()
        .filter_map(|value| {
            let key = value["key"].as_str()?;
            let number = value["value"].as_f64()?;
            let (low, high) = range(key)?;
            Some(json!({
                "parameter": key,
                "value": number,
                "range": [low, high],
                "position": value["position"],
                "at_bound": value["at_bound"],
                "position_scale": value["position_scale"],
            }))
        })
        .collect::<Vec<_>>();
    (!positions.is_empty()).then(|| json!(positions))
}

fn truncate_json_array(value: &mut Value, path: &[&str], keep: usize) -> bool {
    let mut current = value;
    for key in path {
        let Some(next) = current.get_mut(*key) else {
            return false;
        };
        current = next;
    }
    let Some(array) = current.as_array_mut() else {
        return false;
    };
    if array.len() <= keep {
        return false;
    }
    array.truncate(keep);
    true
}

fn compact_study_targets(targets: &Value) -> Value {
    let Some(targets) = targets.as_array() else {
        return targets.clone();
    };
    Value::Array(
        targets
            .iter()
            .map(|target| {
                json!({
                    "key": target["key"],
                    "variable": target["variable"],
                    "metric": target["metric"],
                    "weight": target["weight"],
                    "from": target["from"],
                    "to": target["to"],
                    "validation_from": target["validation_from"],
                    "validation_to": target["validation_to"],
                })
            })
            .collect(),
    )
}

/// 24k 传输上限内的合法 JSON：先裁证据列表，最后才把完整 spec 目标换成紧凑投影。
fn fit_study_summary(result: &mut Value) {
    fn text_len(value: &Value) -> usize {
        serde_json::to_string(value).map_or(usize::MAX, |text| text.chars().count())
    }
    if text_len(result) <= MAX_RESULT_CHARS {
        return;
    }
    result["truncated"] = json!(true);
    let trims: &[(&[&str], usize)] = &[
        (&["evidence", "warnings"], 3),
        (&["evidence", "missing"], 3),
        (&["evidence", "execution", "failed_tasks"], 2),
        (
            &[
                "evidence",
                "comparison",
                "baseline",
                "calibration",
                "targets",
            ],
            2,
        ),
        (
            &[
                "evidence",
                "comparison",
                "baseline",
                "validation",
                "targets",
            ],
            2,
        ),
        (
            &["evidence", "comparison", "best", "calibration", "targets"],
            2,
        ),
        (
            &["evidence", "comparison", "best", "validation", "targets"],
            2,
        ),
        (&["evidence", "parameters", "dims"], 8),
        (&["evidence", "parameters", "baseline", "values"], 8),
        (&["evidence", "parameters", "best", "values"], 8),
        (&["evidence", "objective", "targets"], 4),
        (&["warnings"], 4),
        (&["evidence", "parameters", "dims"], 4),
        (&["evidence", "parameters", "baseline", "values"], 4),
        (&["evidence", "parameters", "best", "values"], 4),
        (&["targets"], 8),
        (&["best_parameters"], 8),
    ];
    for (path, keep) in trims {
        if text_len(result) <= MAX_RESULT_CHARS {
            break;
        }
        truncate_json_array(result, path, *keep);
    }
    if text_len(result) > MAX_RESULT_CHARS {
        result["targets"] = compact_study_targets(&result["targets"]);
        truncate_json_array(result, &["targets"], 8);
    }
    if text_len(result) > MAX_RESULT_CHARS {
        result["evidence"] = json!({
            "status": "abbreviated",
            "reason": "the Study evidence exceeded the tool result budget; inspect the Study directory or ask for one aspect (calibration vs validation, parameters, identity)",
        });
        result["targets"] = compact_study_targets(&result["targets"]);
        truncate_json_array(result, &["targets"], 4);
        truncate_json_array(result, &["warnings"], 2);
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
        let selected = ctx.selected_context()?;
        let ctx = &selected;
        let dir = case_dir(ctx, args, "case")?;
        let effective = ctx.effective_case(&dir)?;
        let mut result = ctx.cli_json(&["hybrid-info", &effective.dir.display().to_string()])?;
        result["model_version"] = json!(ctx
            .model_context
            .as_ref()
            .map(|model| &model.model["version"]));
        Ok(result)
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
        let selected = ctx.selected_context()?;
        let ctx = &selected;
        let dir = case_dir(ctx, args, "case")?;
        let kernel = ctx
            .kernel_dir
            .as_ref()
            .context("no kernel is selected; choose one in Basic setup")?;
        if let Some(model) = &ctx.model_context {
            let mut args = model.args("model-hybrid-check")?;
            args.extend([
                dir.display().to_string(),
                "--kernel".into(),
                kernel.display().to_string(),
            ]);
            if let Some(preprocessors) = &ctx.preprocessors {
                args.extend(["--preprocessors".into(), preprocessors.clone()]);
            }
            return ctx.cli_json(&args.iter().map(String::as_str).collect::<Vec<_>>());
        }
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
        "Search preserved documentation and generated knowledge for the selected frozen model (name=null; app without a selection), or explicitly named mutable development code. Missing frozen documents never fall back to current app documents. Checks source hashes and withholds obsolete assertions. Use exact returned scope for knowledge_update; AI drafts are not scientific validation. Use remote_workspace_knowledge for server edits."
    }
    fn parameters(&self) -> Value {
        object(
            json!({ "query": string("case-insensitive text to find"), "name": nullable_string("mutable development workspace name, or null for selected frozen model source") }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let selected = ctx.selected_context()?;
        let ctx = &selected;
        let query = req_str(args, "query")?.to_lowercase();
        if query.len() > 1024 {
            bail!("documentation query exceeds 1024 bytes");
        }
        let name = opt_str(args, "name");
        let mut provenance = documentation_provenance(ctx, name)?;
        let root = if name.is_some() {
            provenance["knowledge_source"]["source"]
                .as_str()
                .map(|source| Path::new(source).join("docs"))
        } else {
            match &ctx.model_context {
                Some(model) => model.docs_dir.clone(),
                None => ctx.docs_root.clone(),
            }
        };
        provenance["docs_root"] = json!(root);
        let scope = provenance["scope"].clone();
        let drafts = if ctx.knowledge_root.is_some() {
            super::knowledge_drafts::search(ctx, &scope, &query)?
        } else {
            Vec::new()
        };
        let Some(root) = root.filter(|root| root.is_dir()) else {
            return Ok(json!({
                "query":query,"matches":[],"knowledge_checks":{},"drafts":drafts,"scope":scope,"provenance":provenance,
                "retrieval": retrieval_keyword_diagnostic(ctx),
                "documentation_status":"missing",
                "reason":"No documentation snapshot is available for this selected source. Current application documents are not evidence for this frozen model; inspect its source with search_code/read_file."
            }));
        };
        let bindings = knowledge::Bindings::load(&root, &provenance["knowledge_source"]);
        // 可选的 Xerj 召回：候选只决定“先看哪些块”。路径、行段、文件 hash 与知识资格仍由下面的
        // 本地重读与 Bindings.check 决定；Xerj 返回的 _source 不能自证正确。
        let (mut retrieval, candidate_chunks) =
            retrieval_candidates(ctx, &provenance, &scope, &root, &query);
        let mut candidate_chunks_by_file: BTreeMap<String, Vec<retrieval::Candidate>> =
            BTreeMap::new();
        for (candidate, file) in candidate_chunks {
            candidate_chunks_by_file
                .entry(file)
                .or_default()
                .push(candidate);
        }
        let mut knowledge_checks = BTreeMap::new();
        let mut hits = Vec::new();
        let mut rejected_candidates = 0usize;
        let mut candidate_hits = 0usize;
        let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
            .with_context(|| format!("cannot read {}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        files.sort();
        'outer: for file in files {
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let lines: Vec<_> = text.lines().collect();
            let file_name = file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut exact_lines = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                if line.to_lowercase().contains(&query) {
                    exact_lines.push(index + 1);
                    let check = knowledge_checks
                        .entry(file_name.clone())
                        .or_insert_with(|| bindings.check(&file_name, &text, &query));
                    let stale = check["status"] == "needs_review"
                        || (check["status"] == "unknown" && bindings.is_bound(&file_name));
                    hits.push(json!({
                        "file": file_name,
                        "path": file.display().to_string(),
                        "line": index + 1,
                        "context_start_line": index.saturating_sub(3) + 1,
                        "context": if stale { vec!["Stored knowledge withheld: source/document changed or cannot be verified. Use knowledge_checks fresh_source_evidence and inspect relevant current code before answering.".to_owned()] } else { lines[index.saturating_sub(3)..(index + 5).min(lines.len())]
                            .iter().map(|s| s.chars().take(400).collect::<String>()).collect::<Vec<_>>()
                        },
                        "text": if stale { "Stored assertion requires review".to_owned() } else {line.chars().take(400).collect::<String>()},
                        "knowledge_status":check["status"],
                        "may_support_current_answer":check["may_support_current_answer"],
                        "match":"exact",
                    }));
                    if hits.len() >= 60 {
                        break 'outer;
                    }
                }
            }
            let Some(chunks) = candidate_chunks_by_file.remove(&file_name) else {
                continue;
            };
            // 文档在索引后可能被改过：重读的真实字节必须仍与候选清单里的 hash 一致，
            // 且行段仍落在文件内；否则旧行段不可用，只能丢弃候选（不回退成伪造出处）。
            let digest = knowledge::digest(text.as_bytes());
            for candidate in chunks {
                let range_ok = candidate.line_start >= 1
                    && candidate.line_end >= candidate.line_start
                    && candidate.line_end <= lines.len();
                let unchanged = candidate.document_sha256 == digest;
                let already_exact = exact_lines
                    .iter()
                    .any(|line| (candidate.line_start..=candidate.line_end).contains(line));
                if !range_ok || !unchanged || already_exact {
                    rejected_candidates += 1;
                    continue;
                }
                let check = knowledge_checks
                    .entry(file_name.clone())
                    .or_insert_with(|| bindings.check(&file_name, &text, &query));
                let stale = check["status"] == "needs_review"
                    || (check["status"] == "unknown" && bindings.is_bound(&file_name));
                let start = candidate.line_start - 1;
                // BM25 可能靠块内别的词命中；块内找不到原查询时从块首给 8 行，并明确这一条是候选块。
                let anchor = (start..candidate.line_end).find(|index| {
                    lines
                        .get(*index)
                        .is_some_and(|line| line.to_lowercase().contains(&query))
                });
                let (context_start, line_number) = match anchor {
                    Some(index) => (index.saturating_sub(3), index + 1),
                    None => (start, candidate.line_start),
                };
                let context_end = context_start.saturating_add(8).min(lines.len());
                hits.push(json!({
                    "file": file_name,
                    "path": file.display().to_string(),
                    "line": line_number,
                    "context_start_line": context_start + 1,
                    "context": if stale { vec!["Stored knowledge withheld: source/document changed or cannot be verified. Use knowledge_checks fresh_source_evidence and inspect relevant current code before answering.".to_owned()] } else { lines[context_start..context_end]
                        .iter().map(|s| s.chars().take(400).collect::<String>()).collect::<Vec<_>>()
                    },
                    "text": if stale { "Stored assertion requires review".to_owned() } else { lines.get(line_number.saturating_sub(1)).map(|line| line.chars().take(400).collect::<String>()).unwrap_or_default() },
                    "knowledge_status":check["status"],
                    "may_support_current_answer":check["may_support_current_answer"],
                    "match":"candidate",
                    "chunk_id": candidate.chunk_id,
                }));
                candidate_hits += 1;
                if hits.len() >= 60 {
                    break 'outer;
                }
            }
        }
        retrieval["rejected_candidates"] = json!(rejected_candidates);
        retrieval["candidate_hits"] = json!(candidate_hits);
        for check in knowledge_checks.values_mut() {
            for source in check["source_checks"].as_array_mut().into_iter().flatten() {
                if !source["next_tool"].is_null() {
                    source["next_tool"]["arguments"]["name"] = json!(name);
                }
            }
        }
        let mut result = json!({ "query": query, "matches": hits, "provenance": provenance, "knowledge_checks":knowledge_checks,
            "drafts":drafts,"scope":scope,"retrieval":retrieval,
            "refresh_instruction":"On changed or uncovered topics, inspect current source and automatically rewrite a concise card with knowledge_update, exact source hashes and this scope. Keep units, switches, formulas, output effects and validation limits. Do not certify drafts or replace the reviewed baseline.",
            "truncated": hits.len() >= 60 });
        while serde_json::to_string(&result)?.chars().count() > MAX_RESULT_CHARS
            && !result["drafts"].as_array().unwrap().is_empty()
        {
            result["drafts"].as_array_mut().unwrap().pop();
            result["truncated"] = json!(true);
        }
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

fn documentation_provenance(ctx: &ToolContext, name: Option<&str>) -> Result<Value> {
    let resolved = knowledge::local_source(ctx, name);
    let (scope, source) = if ctx.model_context.is_some() && name.is_none() {
        resolved?
    } else {
        resolved.unwrap_or_else(|error| (
        json!({"kind":if name.is_some() {"workspace"} else {"app"},"name":name,"host":null,"source":null}),
        json!({"version_status":"unknown","revision":null,"tag":null,"dirty":null,"reason":error.to_string()})
    ))
    };
    Ok(json!({
        "agent_package_version": env!("CARGO_PKG_VERSION"),
        "docs_root": ctx.docs_root,
        "application_source": if name.is_none() && ctx.model_context.is_none() {source.clone()} else {Value::Null},
        "selected_model":if name.is_none() {ctx.model_context.as_ref().map(|model| &model.model["version"])} else {None},
        "knowledge_source":source,"scope":scope,
        "verification": "Use search_code/read_file with the same source choice (null for selected frozen model; explicitly named workspaces are mutable). Documents and package version alone do not establish runtime sensitivity or applicability.",
    }))
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

/// 检索身份：把可信 provenance 里的来源身份与 scope 拼成稳定 key。不同 id/revision、
/// 不同源码内容、不同 workspace 名字得到不同 key；资料集合变化另由 snapshot_identity 判断
/// （同一 scope 内触发重建新 generation，旧块不复用）。
pub(crate) fn retrieval_identity(provenance: &Value) -> (String, String, String) {
    let scope = &provenance["scope"];
    let kind = scope["kind"].as_str().unwrap_or("unknown");
    let name = scope["name"].as_str().unwrap_or("");
    let project_key = if name.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind}:{name}")
    };
    let source = &provenance["knowledge_source"];
    // 只取稳定身份字段；不把 checked_at 一类易变字段混进 scope key。
    let identity = json!({
        "schema": "colm-docs-source-identity-v1",
        "scope_kind": kind,
        "scope_name": name,
        "source": source["source"],
        "content_sha256": source["content_sha256"],
        "revision": source["revision"],
        "tag": source["tag"],
        "dirty": source["dirty"],
        "version_status": source["version_status"],
        "selected_model": provenance["selected_model"],
    });
    let source_identity = knowledge::digest(identity.to_string().as_bytes());
    let scope_key = knowledge::digest(
        format!("colm-docs-scope-v1\0{project_key}\0{source_identity}").as_bytes(),
    );
    (project_key, source_identity, scope_key)
}

/// 检索关闭或不可用时的诊断字段：原关键词路径照常工作，不能让错误伪装成“没有资料”。
fn retrieval_keyword_diagnostic(ctx: &ToolContext) -> Value {
    let enabled = ctx
        .retrieval
        .as_ref()
        .is_some_and(|handle| handle.settings().enabled);
    json!({"backend":"keyword","enabled":enabled,"state":if enabled {"not_ready"} else {"disabled"}})
}

/// 向 Xerj 要候选块；任何失败都返回诊断并回退关键词路径，不中断 search_docs。
fn retrieval_candidates(
    ctx: &ToolContext,
    provenance: &Value,
    scope: &Value,
    root: &Path,
    query: &str,
) -> (Value, Vec<(retrieval::Candidate, String)>) {
    let Some(handle) = ctx
        .retrieval
        .as_ref()
        .filter(|handle| handle.settings().enabled)
    else {
        return (retrieval_keyword_diagnostic(ctx), Vec::new());
    };
    let settings = handle.settings();
    let (project_key, source_identity, scope_key) = retrieval_identity(provenance);
    let mut diagnostic = json!({
        "backend":"xerj",
        "enabled":true,
        "state":"pending",
        "endpoint":settings.endpoint,
        "index":settings.index,
        "project":project_key,
    });
    let snapshot = match retrieval::collect_snapshot(
        &project_key,
        &scope_key,
        scope,
        root,
        &source_identity,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            diagnostic["state"] = json!("invalid");
            diagnostic["backend"] = json!("keyword");
            diagnostic["fallback_reason"] = json!(format!(
                "cannot enumerate documentation snapshot: {error:#}"
            ));
            return (diagnostic, Vec::new());
        }
    };
    let status = handle.ensure(snapshot.clone());
    diagnostic["state"] = status["state"].clone();
    diagnostic["generation"] = status["generation"].clone();
    match handle.candidates(&snapshot, query, 20) {
        Ok(candidates) => {
            diagnostic["candidates"] = json!(candidates.len());
            let files = candidates
                .into_iter()
                .map(|candidate| {
                    let file = candidate.relative_path.clone();
                    (candidate, file)
                })
                .collect();
            (diagnostic, files)
        }
        Err(error) => {
            // 候选不可用时仍回退原关键词路径：backend 必须切回 keyword，state/reason 说明原因，
            // 不能让人误以为结果来自索引。
            diagnostic["backend"] = json!("keyword");
            diagnostic["state"] = json!(error.state());
            diagnostic["fallback_reason"] = json!(error.reason());
            (diagnostic, Vec::new())
        }
    }
}

/// 当前应用选择（frozen model 或 app）的文档快照，供 GUI 的“更新文档索引/查询状态”
/// 在没有工具调用时建索引。只枚举当前允许的 docs 目录，不扩大文件访问范围。
pub(crate) fn selection_docs_snapshot(ctx: &ToolContext) -> Result<retrieval::DocSnapshot> {
    let provenance = documentation_provenance(ctx, None)?;
    let scope = provenance["scope"].clone();
    let root = match &ctx.model_context {
        Some(model) => model.docs_dir.clone(),
        None => ctx.docs_root.clone(),
    }
    .context("no documentation snapshot is available for the current source")?;
    if !root.is_dir() {
        bail!("documentation snapshot directory does not exist for the current source");
    }
    let (project_key, source_identity, scope_key) = retrieval_identity(&provenance);
    retrieval::collect_snapshot(&project_key, &scope_key, &scope, &root, &source_identity)
        .map_err(|error| anyhow::anyhow!("cannot enumerate documentation snapshot: {error:#}"))
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod read_tests;
