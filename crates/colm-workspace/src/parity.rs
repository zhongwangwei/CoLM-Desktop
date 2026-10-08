//! 用工作区编出的引擎或内核跑算例的副本；Rust 对 Fortran 的对齐检查；改动前后的回归对比
//! （docs/design-ai-assistant.md 第 4–6 节：`run_case_with`、`parity_check`、回归门槛）。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

use crate::build::{run_logged, Outcome};
use crate::compare::{self, Report, Tolerance};
use crate::gates::{ChangeKind, ParityRecord, Regression};
use crate::layout::{now, Workspace};

/// 复制算例时不带的东西：上次的输出、阶段指纹、远程运行的记录。
fn skipped(name: &str) -> bool {
    matches!(name, "out" | "stages.json" | ".colm-remote.json" | ".colm-fetch.json")
        || name.starts_with(".colm-")
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(from, to).with_context(|| format!("cannot copy {}", from.display()))?;
    }
    Ok(())
}

/// 复制算例（不带输出），并把 namelist 里的旧算例路径换成新路径，这样副本的输出写到副本里。
pub fn copy_case(src: &Path, dst: &Path) -> Result<()> {
    ensure!(src.join("case.nml").is_file(), "{} has no case.nml", src.display());
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !skipped(&name) {
            copy_tree(&entry.path(), &dst.join(&name))?;
        }
    }
    let new = dst.canonicalize()?;
    let mut olds = vec![src.to_string_lossy().into_owned()];
    if let Ok(canonical) = src.canonicalize() {
        olds.push(canonical.to_string_lossy().into_owned());
    }
    olds.sort();
    olds.dedup();
    // 长的先换，免得 `/a/b` 先把 `/a/bc` 的前缀吃掉。
    olds.sort_by_key(|o| std::cmp::Reverse(o.len()));
    for entry in std::fs::read_dir(dst)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "nml") {
            let mut text = std::fs::read_to_string(&path)?;
            for old in &olds {
                text = text.replace(old.trim_end_matches('/'), &new.to_string_lossy());
            }
            std::fs::write(&path, text)?;
        }
    }
    Ok(())
}

/// `case.nml` 里的 `DEF_CASE_NAME`。
pub fn case_name(case: &Path) -> Result<String> {
    let text = std::fs::read_to_string(case.join("case.nml"))
        .with_context(|| format!("cannot read {}/case.nml", case.display()))?;
    text.lines()
        .find(|l| l.trim_start().starts_with("DEF_CASE_NAME"))
        .and_then(|l| l.split('\'').nth(1))
        .map(str::to_owned)
        .context("case.nml has no DEF_CASE_NAME")
}

/// 下一个空的 `runs/<编号>/` 目录。
pub fn new_run_dir(workspace: &Workspace) -> Result<PathBuf> {
    std::fs::create_dir_all(workspace.runs())?;
    let mut id = now();
    loop {
        let dir = workspace.runs().join(id.to_string());
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            return Ok(dir);
        }
        id += 1;
    }
}

/// 一次运行：谁跑的、算例副本在哪、history 在哪。
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub label: String,
    pub engine: String,
    pub case_copy: PathBuf,
    pub history: PathBuf,
    pub outcome: Outcome,
}

/// 用一个 `colm-cli` 和内核目录跑算例的副本（`runs/…/<label>/`）。
#[allow(clippy::too_many_arguments)]
pub fn run_copy(
    workspace: &Workspace,
    case: &Path,
    run_dir: &Path,
    label: &str,
    cli: &Path,
    kernel: &Path,
    engine: &str,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<RunResult> {
    ensure!(["rust", "fortran"].contains(&engine), "engine must be rust or fortran");
    ensure!(cli.is_file(), "{} does not exist; build the engine first", cli.display());
    ensure!(
        kernel.join("manifest.json").is_file(),
        "{} has no manifest.json; build or choose a kernel first",
        kernel.display()
    );
    let copy = run_dir.join(label);
    copy_case(case, &copy)?;
    let args = vec![
        "run".to_owned(),
        copy.display().to_string(),
        "--kernel".into(),
        kernel.display().to_string(),
        "--engine".into(),
        engine.to_owned(),
        "--preprocessors".into(),
        "rust".into(),
        "--force".into(),
        "1".into(),
    ];
    let outcome = run_logged(workspace, &format!("run-{label}"), cli, &args, &copy, &[], false, cancel)?;
    let history = copy.join("out").join(case_name(&copy)?).join("history");
    Ok(RunResult {
        label: label.to_owned(),
        engine: engine.to_owned(),
        case_copy: copy,
        history,
        outcome,
    })
}

/// `run_case_with`：用工作区编出的引擎（和内核）跑算例的副本。
pub fn run_case_with(
    workspace: &Workspace,
    case: &Path,
    engine: &str,
    preset: &str,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<RunResult> {
    crate::build::check_preset(preset)?;
    let kernel = workspace.kernels().join(preset);
    let run_dir = new_run_dir(workspace)?;
    run_copy(
        workspace,
        case,
        &run_dir,
        engine,
        &workspace.bin().join("colm-cli"),
        &kernel,
        engine,
        cancel,
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct ParityReport {
    pub ok: bool,
    pub case: String,
    pub preset: String,
    pub run_dir: PathBuf,
    pub rust: RunResult,
    pub fortran: RunResult,
    pub compare: Option<Report>,
    pub first_difference: Option<String>,
    pub report: PathBuf,
}

fn write_report(workspace: &Workspace, name: &str, value: &impl Serialize) -> Result<PathBuf> {
    std::fs::create_dir_all(workspace.reports())?;
    let path = workspace.reports().join(format!("{name}-{}.json", now()));
    std::fs::write(&path, serde_json::to_string_pretty(value)?)?;
    Ok(path)
}

/// `parity_check`：同一个算例分别用工作区的 Rust 引擎和 Fortran 内核跑，逐位对比，找出第一个出现差异的
/// 变量与时间步。两边用同一套 Rust 前处理，所以差异只来自 colm 主循环。
pub fn parity_check(
    workspace: &mut Workspace,
    case: &Path,
    preset: &str,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<ParityReport> {
    crate::build::check_preset(preset)?;
    let kernel = workspace.kernels().join(preset);
    let cli = workspace.bin().join("colm-cli");
    let run_dir = new_run_dir(workspace)?;
    let rust = run_copy(workspace, case, &run_dir, "rust", &cli, &kernel, "rust", cancel)?;
    let fortran = run_copy(workspace, case, &run_dir, "fortran", &cli, &kernel, "fortran", cancel)?;
    let (comparison, first, ok) = if rust.outcome.ok && fortran.outcome.ok {
        let report = compare::compare(&rust.history, &fortran.history, Tolerance::default())?;
        let first = report.first_difference.as_ref().map(compare::First::describe);
        let ok = report.bitwise_identical();
        (Some(report), first, ok)
    } else {
        (
            None,
            Some(format!(
                "a run failed ({}): see the logs",
                if rust.outcome.ok { "Fortran" } else { "Rust" }
            )),
            false,
        )
    };
    let mut report = ParityReport {
        ok,
        case: case.display().to_string(),
        preset: preset.to_owned(),
        run_dir,
        rust,
        fortran,
        compare: comparison,
        first_difference: first.clone(),
        report: PathBuf::new(),
    };
    report.report = write_report(workspace, "parity", &report)?;
    workspace.info.gates.parity = Some(ParityRecord {
        ok,
        at: now(),
        commit: workspace.head()?,
        preset: preset.to_owned(),
        case: case.display().to_string(),
        first_difference: first,
    });
    workspace.save()?;
    Ok(report)
}

/// 改动前的基线：应用自带的（或正式仓库里的）`colm-cli` 和内核。
#[derive(Debug, Clone)]
pub struct Baseline {
    pub cli: PathBuf,
    pub kernel: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegressionReport {
    pub ok: bool,
    pub kind: ChangeKind,
    pub engine: String,
    pub verdict: String,
    pub baseline: RunResult,
    pub candidate: RunResult,
    pub compare: Option<Report>,
    /// 闭合诊断（水量 `f_xerr`、能量 `f_zerr`）的最大绝对值：基线、改动后。
    pub closure: Vec<ClosureCheck>,
    pub report: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClosureCheck {
    pub variable: String,
    pub baseline: f64,
    pub candidate: f64,
    pub ok: bool,
}

/// 闭合允许的上限：基线的十倍，但不低于 1e-6（基线本身常常就是舍入误差量级）。
fn closure_limit(baseline: f64) -> f64 {
    (10.0 * baseline).max(1e-6)
}

/// 回归门槛：在参考算例上，工作区的构建与基线逐变量对比。
/// 重构要求逐位一致；物理修改列出哪些变量变了、变了多少，出现 NaN/无穷大或闭合变差直接不通过。
#[allow(clippy::too_many_arguments)]
pub fn regress(
    workspace: &mut Workspace,
    case: &Path,
    preset: &str,
    engine: &str,
    baseline: &Baseline,
    kind: ChangeKind,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<RegressionReport> {
    crate::build::check_preset(preset)?;
    let run_dir = new_run_dir(workspace)?;
    let base = run_copy(workspace, case, &run_dir, "baseline", &baseline.cli, &baseline.kernel, engine, cancel)?;
    // 工作区没编这个预设的内核时，Rust 引擎只需要清单，借基线的内核目录。
    let ws_kernel = workspace.kernels().join(preset);
    let kernel = if ws_kernel.join("manifest.json").is_file() {
        ws_kernel
    } else if engine == "rust" {
        baseline.kernel.clone()
    } else {
        bail!("the Fortran engine needs a kernel built in the workspace; run build_kernel first");
    };
    let cand = run_copy(
        workspace,
        case,
        &run_dir,
        "candidate",
        &workspace.bin().join("colm-cli"),
        &kernel,
        engine,
        cancel,
    )?;
    let mut closure = Vec::new();
    let (comparison, verdict, ok) = if base.outcome.ok && cand.outcome.ok {
        let report = compare::compare(&base.history, &cand.history, Tolerance::default())?;
        let names = ["f_xerr", "f_zerr"];
        let before = compare::max_abs(&base.history, &names)?;
        let after = compare::max_abs(&cand.history, &names)?;
        for name in names {
            if let (Some(&b), Some(&c)) = (before.get(name), after.get(name)) {
                closure.push(ClosureCheck {
                    variable: name.to_owned(),
                    baseline: b,
                    candidate: c,
                    ok: c <= closure_limit(b),
                });
            }
        }
        let closure_ok = closure.iter().all(|c| c.ok);
        let (ok, verdict) = judge(kind, &report, closure_ok);
        (Some(report), verdict, ok)
    } else {
        (
            None,
            format!(
                "the {} run failed; see its log",
                if base.outcome.ok { "candidate" } else { "baseline" }
            ),
            false,
        )
    };
    let mut result = RegressionReport {
        ok,
        kind,
        engine: engine.to_owned(),
        verdict: verdict.clone(),
        baseline: base,
        candidate: cand,
        compare: comparison.clone(),
        closure,
        report: PathBuf::new(),
    };
    result.report = write_report(workspace, "regression", &result)?;
    workspace.info.gates.regression = Some(Regression {
        kind,
        ok,
        at: now(),
        commit: workspace.head()?,
        case: case.display().to_string(),
        identical: comparison.as_ref().map_or(0, |c| c.identical),
        changed: comparison
            .as_ref()
            .map_or(0, |c| c.differs + c.within_tolerance),
        verdict,
    });
    workspace.save()?;
    Ok(result)
}

/// 判定：重构要逐位一致；物理修改只要求没有 NaN/无穷大、闭合没变差。
pub fn judge(kind: ChangeKind, report: &Report, closure_ok: bool) -> (bool, String) {
    if report.new_nonfinite > 0 {
        return (false, format!("{} values became NaN or infinite", report.new_nonfinite));
    }
    if !closure_ok {
        return (false, "water or energy closure got worse (f_xerr / f_zerr)".into());
    }
    let changed = report.differs + report.within_tolerance;
    match kind {
        ChangeKind::Refactor => {
            if report.bitwise_identical() {
                (true, format!("bitwise identical in all {} variables", report.identical))
            } else {
                let first = report
                    .first_difference
                    .as_ref()
                    .map(compare::First::describe)
                    .unwrap_or_else(|| "a file or variable is missing".into());
                (false, format!("a refactor must be bitwise identical, but {changed} variables changed (first: {first})"))
            }
        }
        ChangeKind::Physics => {
            let top: Vec<String> = report
                .changed
                .iter()
                .take(5)
                .map(|c| format!("{} (max rel {:.2e})", c.name, c.max_rel))
                .collect();
            (
                true,
                format!(
                    "{changed} variables changed, {} identical; largest: {}",
                    report.identical,
                    if top.is_empty() { "none".into() } else { top.join(", ") }
                ),
            )
        }
    }
}

#[cfg(test)]
#[path = "parity_tests.rs"]
mod parity_tests;
