//! B 级运行操作：建站点、建算例、改参数、运行、建与跑 Study。每次调用都先经用户批准
//! （审批卡片显示 [`Tool::summary`] 给出的真实动作）。另有只读的 `scan_sites`，帮助找数据。

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{object, opt_str, req_str, Finished, Tier, Tool, ToolContext};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ScanSites),
        Box::new(CreateSite),
        Box::new(CreateCase),
        Box::new(SetCaseFields),
        Box::new(RunCase),
        Box::new(CreateStudy),
        Box::new(RunStudy),
        Box::new(StudyControl),
    ]
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn nullable(kind: &str, description: &str) -> Value {
    json!({ "type": [kind, "null"], "description": description })
}

fn finished(command: &str, done: Finished) -> Value {
    json!({
        "command": command,
        "success": done.success,
        "output_tail": done.stdout_tail,
        "errors_tail": done.stderr_tail,
    })
}

fn kernel(ctx: &ToolContext) -> Result<String> {
    Ok(ctx
        .kernel_dir
        .as_ref()
        .context("no kernel is selected in the application; choose one in Basic setup first")?
        .display()
        .to_string())
}

/// 算例目录（必须有 case.nml）。
fn case_dir(ctx: &ToolContext, args: &Value) -> Result<std::path::PathBuf> {
    let dir = ctx.resolve(req_str(args, "case")?);
    if !dir.join("case.nml").is_file() {
        bail!("{} has no case.nml", dir.display());
    }
    Ok(dir)
}

struct ScanSites;

impl Tool for ScanSites {
    fn name(&self) -> &'static str {
        "scan_sites"
    }
    fn description(&self) -> &'static str {
        "List the site files in a directory (e.g. a PLUMBER2 Sitedata folder): site name, location, years, land cover, and the matching forcing file when a forcing directory is given. Use it to find inputs before creating a case."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "dir": string("directory with site files (*_site.nc)"),
            "forcing_dir": nullable("string", "directory with forcing files (*_Met.nc), or null"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = ctx.resolve(req_str(args, "dir")?).display().to_string();
        let forcing = opt_str(args, "forcing_dir").map(|f| ctx.resolve(f).display().to_string());
        let mut cli = vec!["scan", "--dir", dir.as_str()];
        match &forcing {
            Some(forcing) => cli.extend(["--forcing-dir", forcing.as_str()]),
            None => cli.extend(["--quick", "1"]),
        }
        let text = ctx.cli(&cli)?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

struct CreateSite;

impl Tool for CreateSite {
    fn name(&self) -> &'static str {
        "create_site"
    }
    fn description(&self) -> &'static str {
        "Create a site file for a location (longitude, latitude); land cover and soil come from the raw data directory when given, otherwise nominal assumptions. Needs approval."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "out": string("path of the site file to write (*.nc)"),
            "lon": { "type": "number", "description": "longitude, degrees" },
            "lat": { "type": "number", "description": "latitude, degrees" },
            "landtype": nullable("integer", "land cover class, or null to leave it to CoLM"),
            "mode": nullable("string", "igbp, usgs, pft, pc, urban…; null for igbp"),
            "rawdata": nullable("string", "CoLM raw data directory, or null"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "新建站点文件 {}（经度 {}，纬度 {}，模式 {}）",
            args["out"].as_str().unwrap_or("?"),
            args["lon"],
            args["lat"],
            args["mode"].as_str().unwrap_or("igbp")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let out = ctx.resolve(req_str(args, "out")?).display().to_string();
        let lon = args["lon"].as_f64().context("lon is required")?.to_string();
        let lat = args["lat"].as_f64().context("lat is required")?.to_string();
        let mut cli = vec![
            "site-new",
            "--out",
            out.as_str(),
            "--lon",
            lon.as_str(),
            "--lat",
            lat.as_str(),
            "--json",
            "1",
        ];
        let landtype = args["landtype"].as_i64().map(|v| v.to_string());
        if let Some(landtype) = &landtype {
            cli.extend(["--landtype", landtype.as_str()]);
        }
        if let Some(mode) = opt_str(args, "mode") {
            cli.extend(["--mode", mode]);
        }
        let rawdata = opt_str(args, "rawdata").map(|r| ctx.resolve(r).display().to_string());
        if let Some(rawdata) = &rawdata {
            cli.extend(["--rawdata", rawdata.as_str()]);
        }
        let text = ctx.cli(&cli)?;
        Ok(
            json!({ "site": out, "report": serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)) }),
        )
    }
}

struct CreateCase;

impl Tool for CreateCase {
    fn name(&self) -> &'static str {
        "create_case"
    }
    fn description(&self) -> &'static str {
        "Create a single-point CoLM case directory from a site file and a forcing file: simulation period, spin-up and land-surface mode (igbp, usgs, pft, pc, urban…). Needs approval. Afterwards the user can open it in the workbench."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "site": string("site file (*_site.nc)"),
            "out": string("new case directory to create"),
            "name": nullable("string", "case name, or null for the directory name"),
            "start": nullable("string", "start date YYYY-MM-DD, or null for the forcing start"),
            "end": nullable("string", "end date YYYY-MM-DD, or null for the forcing end"),
            "met": nullable("string", "forcing file (*_Met.nc), or null to find it by naming convention"),
            "mode": nullable("string", "igbp, usgs, pft, pc, urban…; null for igbp"),
            "spinup_years": nullable("integer", "spin-up years per cycle, or null for 1"),
            "spinup_repeat": nullable("integer", "spin-up cycles, or null for 1"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "新建算例 {}：站点 {}，模式 {}，时段 {} 至 {}，强迫 {}，预热 {} 年 × {} 遍",
            args["out"].as_str().unwrap_or("?"),
            args["site"].as_str().unwrap_or("?"),
            args["mode"].as_str().unwrap_or("igbp"),
            args["start"].as_str().unwrap_or("强迫起点"),
            args["end"].as_str().unwrap_or("强迫终点"),
            args["met"].as_str().unwrap_or("按命名约定查找"),
            args["spinup_years"].as_i64().unwrap_or(1),
            args["spinup_repeat"].as_i64().unwrap_or(1)
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let site = ctx.resolve(req_str(args, "site")?).display().to_string();
        let out = ctx.resolve(req_str(args, "out")?);
        if out.join("case.nml").exists() {
            bail!(
                "{} already has a case.nml; choose a new directory",
                out.display()
            );
        }
        let out = out.display().to_string();
        let mut cli = vec!["new", "--site", site.as_str(), "--out", out.as_str()];
        for (flag, key) in [
            ("--name", "name"),
            ("--start", "start"),
            ("--end", "end"),
            ("--mode", "mode"),
        ] {
            if let Some(value) = opt_str(args, key) {
                cli.extend([flag, value]);
            }
        }
        let met = opt_str(args, "met").map(|m| ctx.resolve(m).display().to_string());
        if let Some(met) = &met {
            cli.extend(["--met", met.as_str()]);
        }
        let years = args["spinup_years"].as_i64().map(|v| v.to_string());
        let repeat = args["spinup_repeat"].as_i64().map(|v| v.to_string());
        if let Some(years) = &years {
            cli.extend(["--spinup-years", years.as_str()]);
        }
        if let Some(repeat) = &repeat {
            cli.extend(["--spinup-repeat", repeat.as_str()]);
        }
        let report = ctx.cli(&cli)?;
        Ok(
            json!({ "case": out, "created": Path::new(&out).join("case.nml").is_file(), "report": report.trim() }),
        )
    }
}

struct SetCaseFields;

/// 把一个值的文本按字段类型解析成 namelist 值；类型不符时报错。
pub(crate) fn typed_value(
    name: &str,
    text: &str,
) -> Result<(colm_namelist::Value, Option<&'static str>)> {
    if colm_case::pft::is_override_path(name) {
        let value: f64 = text
            .trim()
            .parse()
            .with_context(|| format!("{name} needs a number"))?;
        return Ok((
            colm_namelist::Value::Real {
                text: format!("{value:.17e}"),
            },
            Some("nl_colm"),
        ));
    }
    let field =
        colm_schema::find(name).with_context(|| format!("{name} is not a CoLM namelist field"))?;
    let group = field
        .group
        .with_context(|| format!("{name} is derived by CoLM and cannot be set from a namelist"))?;
    let trimmed = text.trim();
    let value = match field.kind {
        colm_schema::FieldKind::Logical => match trimmed.to_ascii_lowercase().trim_matches('.') {
            "true" | "t" => colm_namelist::Value::Bool(true),
            "false" | "f" => colm_namelist::Value::Bool(false),
            _ => bail!("{name} is logical; use true or false"),
        },
        colm_schema::FieldKind::Integer => colm_namelist::Value::Int(
            trimmed
                .parse()
                .with_context(|| format!("{name} needs an integer"))?,
        ),
        colm_schema::FieldKind::Real => {
            let value: f64 = trimmed
                .parse()
                .with_context(|| format!("{name} needs a number"))?;
            if !value.is_finite() {
                bail!("{name} must be finite");
            }
            colm_namelist::Value::Real {
                text: format!("{value:.17e}"),
            }
        }
        colm_schema::FieldKind::Character { len } => {
            let raw = trimmed.trim_matches('\'').trim_matches('"');
            if raw.chars().count() > len {
                bail!("{name} holds at most {len} characters");
            }
            colm_namelist::Value::Str(raw.to_owned())
        }
    };
    Ok((value, Some(group)))
}

impl Tool for SetCaseFields {
    fn name(&self) -> &'static str {
        "set_case_fields"
    }
    fn description(&self) -> &'static str {
        "Change namelist fields in a case's case.nml (values as text: true/false, numbers, or strings). Each field is checked against CoLM's field definitions. Needs approval. Stages affected by the change rerun on the next run."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "case": string("case directory"),
            "fields": {
                "type": "array",
                "description": "fields to set",
                "items": object(json!({ "name": string("field name, e.g. DEF_USE_PC or DEF_PFT_VMAX25(3)"), "value": string("new value as text") })),
            },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        let changes: Vec<String> = args["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| {
                format!(
                    "{} = {}",
                    f["name"].as_str().unwrap_or("?"),
                    f["value"].as_str().unwrap_or("?")
                )
            })
            .collect();
        format!(
            "修改 {} 的 case.nml：{}",
            args["case"].as_str().unwrap_or("?"),
            changes.join("；")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args)?;
        let path = dir.join("case.nml");
        let text = std::fs::read_to_string(&path)?;
        let mut document = colm_namelist::parse(&text)?;
        let fields = args["fields"].as_array().context("fields must be a list")?;
        if fields.is_empty() {
            bail!("no fields given");
        }
        let mut changed = Vec::new();
        for field in fields {
            let name = req_str(field, "name")?;
            let text = field["value"]
                .as_str()
                .context("each field needs a value")?;
            let (value, group) = typed_value(name, text)?;
            let old = document.get(name).map(ToString::to_string);
            if document.get(name).is_some() {
                document.set(name, value)?;
            } else {
                document.insert(name, value, group.unwrap_or("nl_colm"))?;
            }
            changed.push(json!({ "name": name, "old": old, "new": document.get(name).map(ToString::to_string) }));
        }
        std::fs::write(&path, document.to_string())
            .with_context(|| format!("cannot write {}", path.display()))?;
        Ok(json!({ "case": dir.display().to_string(), "changed": changed }))
    }
}

struct RunCase;

impl Tool for RunCase {
    fn name(&self) -> &'static str {
        "run_case"
    }
    fn description(&self) -> &'static str {
        "Run a case with the selected kernel: all stages (mksrfdata, mkinidata, colm; unchanged stages are skipped) or one stage. Can take minutes; the user can stop it. Needs approval."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "case": string("case directory"),
            "stage": nullable("string", "mksrfdata, mkinidata or colm; null for all stages"),
            "engine": nullable("string", "rust or fortran for the colm stage; null for rust"),
            "force": { "type": "boolean", "description": "rerun even stages whose inputs did not change" },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "运行算例 {}（阶段：{}，引擎：{}{}）",
            args["case"].as_str().unwrap_or("?"),
            args["stage"].as_str().unwrap_or("全部"),
            args["engine"].as_str().unwrap_or("rust"),
            if args["force"] == true {
                "，强制重跑"
            } else {
                ""
            }
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let dir = case_dir(ctx, args)?.display().to_string();
        let kernel = kernel(ctx)?;
        let mut cli = vec!["run", dir.as_str(), "--kernel", kernel.as_str()];
        if let Some(stage) = opt_str(args, "stage") {
            if !["mksrfdata", "mkinidata", "colm"].contains(&stage) {
                bail!("stage must be mksrfdata, mkinidata or colm");
            }
            cli.extend(["--stage", stage]);
        }
        if let Some(engine) = opt_str(args, "engine") {
            if !["rust", "fortran"].contains(&engine) {
                bail!("engine must be rust or fortran");
            }
            cli.extend(["--engine", engine]);
        }
        if args["force"] == true {
            cli.extend(["--force", "1"]);
        }
        Ok(finished("run", ctx.cli_long(&cli)?))
    }
}

struct CreateStudy;

impl Tool for CreateStudy {
    fn name(&self) -> &'static str {
        "create_study"
    }
    fn description(&self) -> &'static str {
        "Create an uncertainty or tuning Study under a project directory from a Study spec (JSON object, same format as colm-cli study-create). The spec is preflight-checked first. Needs approval."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "root": string("project directory that contains the base cases"),
            "spec_json": string("the Study spec as a JSON text"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        let spec: Value =
            serde_json::from_str(args["spec_json"].as_str().unwrap_or("{}")).unwrap_or_default();
        format!(
            "在 {} 下建 Study：{}，方法 {}，基础算例 {}，参数 {} 个",
            args["root"].as_str().unwrap_or("?"),
            spec["kind"].as_str().unwrap_or("?"),
            spec["method"].as_str().unwrap_or("?"),
            spec["base_cases"],
            spec["parameters"].as_array().map_or(0, Vec::len)
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let root = ctx.resolve(req_str(args, "root")?).display().to_string();
        let spec: Value = serde_json::from_str(req_str(args, "spec_json")?)
            .context("spec_json is not valid JSON")?;
        let file =
            std::env::temp_dir().join(format!("colm-agent-study-{}.json", std::process::id()));
        std::fs::write(&file, serde_json::to_string_pretty(&spec)?)?;
        let file_s = file.display().to_string();
        let result = ctx
            .cli(&["study-preflight", &root, "--spec", &file_s])
            .and_then(|preflight| {
                Ok((
                    preflight,
                    ctx.cli(&["study-create", &root, "--spec", &file_s])?,
                ))
            });
        let _ = std::fs::remove_file(&file);
        let (preflight, created) = result?;
        Ok(json!({ "preflight": preflight.trim(), "study": created.trim().lines().last() }))
    }
}

struct RunStudy;

impl Tool for RunStudy {
    fn name(&self) -> &'static str {
        "run_study"
    }
    fn description(&self) -> &'static str {
        "Run (or resume) a Study with the selected kernel and the Rust engine. Can take a long time; the user can stop it. Needs approval."
    }
    fn parameters(&self) -> Value {
        object(json!({ "study": string("Study directory") }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "运行 Study {}（Rust 引擎，可能耗时较长）",
            args["study"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let study = ctx.resolve(req_str(args, "study")?).display().to_string();
        let kernel = kernel(ctx)?;
        Ok(finished(
            "study-run",
            ctx.cli_long(&["study-run", &study, "--kernel", &kernel, "--engine", "rust"])?,
        ))
    }
}

struct StudyControl;

impl Tool for StudyControl {
    fn name(&self) -> &'static str {
        "study_control"
    }
    fn description(&self) -> &'static str {
        "Pause, resume or cancel a Study. Needs approval."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "study": string("Study directory"),
            "action": { "type": "string", "enum": ["pause", "resume", "cancel"], "description": "what to do" },
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Act
    }
    fn summary(&self, args: &Value) -> String {
        let action = match args["action"].as_str() {
            Some("pause") => "暂停",
            Some("resume") => "恢复",
            Some("cancel") => "取消",
            _ => "?",
        };
        format!("{action} Study {}", args["study"].as_str().unwrap_or("?"))
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let study = ctx.resolve(req_str(args, "study")?).display().to_string();
        let action = req_str(args, "action")?;
        if !["pause", "resume", "cancel"].contains(&action) {
            bail!("action must be pause, resume or cancel");
        }
        let output = ctx.cli(&[&format!("study-{action}"), &study])?;
        Ok(json!({ "study": study, "action": action, "output": output.trim() }))
    }
}

#[cfg(test)]
#[path = "act_tests.rs"]
mod act_tests;
