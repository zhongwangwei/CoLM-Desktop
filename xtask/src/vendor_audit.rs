//! 三向审计 vendored CoLM：找出"桌面 fork 新增、但当前树已消失"的功能行。
//!
//! 背景：2026-11 同步 4c28107 时，`d79ddcbf` 用上游版本替换了几个桌面已打补丁
//! 的文件（`HYDRO/MOD_Hydro_SoilWater.F90`、`MOD_SoilSnowHydrology.F90` 的
//! 不透水表层块），静默丢了补丁；站点黄金一度把它误当成求解器数值噪声。
//! 三向比对（基准导入 → 同步前桌面树 → 当前 HEAD）把这类丢失变成可检查项：
//! 只有"`fork` 相对 `base` 新增、且 `HEAD` 里逐行找不到"的**功能行**才报。
//!
//! 声明行、纯注释、`TRACER/`（material always-on 重构刻意删除）与
//! `mksrfdata/`（重构后行号/结构都换了）不参与判定 —— 这一筛选规则本身就是
//! 那次审计踩出来的：如果只按注释标记找，恰好会漏掉没有注释的补丁块。
//!
//! 用法:
//!   cargo run -p xtask -- audit-vendor --base <基准导入 rev> --fork <同步前桌面 rev>
//! 默认 `HEAD` 为当前工作树指向的提交；输出按文件汇总，`--fail-on-found` 时
//! 有命中就以非零码退出，方便在合并上游快照的流程里当闸门。

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// 审计报告里每个文件最多打印多少行命中，避免刷屏；剩余数量照样汇报。
const MAX_LINES_PER_FILE: usize = 8;

/// 声明行的前缀：4c 重构大面积重新声明变量，这些行出现/消失不代表补丁丢失。
const DECLARATION_PREFIXES: [&str; 7] = [
    "real(",
    "integer",
    "logical",
    "character",
    "use ",
    "public ",
    "private ",
];

/// 重构路径：TRACER 分支在 material always-on 重构里被刻意删除；mksrfdata 的
/// 预处理代码结构整体重排，逐行比对只有噪声。
const EXCLUDED_PATH_FRAGMENTS: [&str; 3] = ["/TRACER/", "/mksrfdata/", "/MATERIAL/"];

/// 审计参数。
pub struct AuditRequest {
    pub base: String,
    pub fork: String,
    pub head: String,
}

/// 一个文件的命中行。
pub struct FileReport {
    pub path: String,
    pub lines: Vec<String>,
}

/// 解析 `git diff -U0` 输出里的新增行（不含 `+++` 文件头）。
fn added_lines(diff: &str) -> Vec<String> {
    diff.lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .map(|line| line[1..].to_string())
        .collect()
}

/// 归一化：去掉行尾 `!` 注释、折叠空白、转小写。
///
/// Fortran 对大小写与词间空白不敏感，4c 重构常把 `vmax25(lc)` 排成
/// `vmax25     (lc)`、把 `CASE ('MJ79', 'mj79')` 收掉空格 —— 这些不是补丁
/// 丢失，归一化后再比对能把这类噪声滤掉（本仓库第一次跑三向审计时就漏报了
/// 一个只差空白的命中，所以这里刻意做得宽容）。
fn normalize(line: &str) -> String {
    let code = match line.find('!') {
        // Fortran 字符串里的 `!` 不该截断，但审计只接收单行简单语句；
        // 4c 重构里没有含 `!` 的字符串常量会参与本判定。
        Some(index) => &line[..index],
        None => line,
    };
    code.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// 是否是值得报告的功能行：非空、够长、不是声明、不是纯注释。
fn is_functional(code: &str) -> bool {
    if code.len() <= 20 {
        return false;
    }
    let lower = code.to_ascii_lowercase();
    !lower.starts_with("!") && !DECLARATION_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// 该路径是否参与审计。
fn included_path(path: &str) -> bool {
    path.ends_with(".F90") && !EXCLUDED_PATH_FRAGMENTS.iter().any(|f| path.contains(f))
}

/// 纯函数部分：给定基准/同步前/当前三份文件内容，返回"fork 新增且当前缺失"的行。
///
/// `fork_lines` 是 `base..fork` 的新增行（已由 [`added_lines`] 解析）；返回保持
/// 原顺序、去重后的命中列表。判定规则是**三向**的：行必须在 `base` 里不存在
/// （否则是上游自己写的旧行被 4c 重构改写，不是丢失的补丁），并且归一化后在
/// `head` 里逐行找不到。
pub fn missing_fork_lines(base: &str, fork_lines: &[String], head: &str) -> Vec<String> {
    let base_set: BTreeSet<String> = base.lines().map(normalize).collect();
    let head_set: BTreeSet<String> = head.lines().map(normalize).collect();
    let mut seen = BTreeSet::new();
    let mut missing = Vec::new();
    for raw in fork_lines {
        let code = normalize(raw);
        if !is_functional(&code) {
            continue;
        }
        if base_set.contains(&code) || head_set.contains(&code) {
            continue;
        }
        if seen.insert(code.clone()) {
            missing.push(code);
        }
    }
    missing
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .with_context(|| format!("cannot run git {args:?}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {args:?} failed: {stderr}");
    }
    String::from_utf8(output.stdout).context("git output is not UTF-8")
}

fn show(root: &Path, rev: &str, path: &str) -> Result<Option<String>> {
    let spec = format!("{rev}:{path}");
    let output = Command::new("git")
        .current_dir(root)
        .args(["show", &spec])
        .output()
        .with_context(|| format!("cannot run git show {spec}"))?;
    if !output.status.success() {
        // 文件在该 rev 不存在（上游新增/删除）不算审计对象。
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8(output.stdout).with_context(|| format!("{spec} is not UTF-8"))?,
    ))
}

/// 跑完整审计，返回命中文件列表。
pub fn run(root: &Path, request: &AuditRequest) -> Result<Vec<FileReport>> {
    let range = format!("{}..{}", request.base, request.fork);
    let changed = git(
        root,
        &["diff", "--name-only", &range, "--", "vendor/CoLM202X"],
    )?;
    let mut reports = Vec::new();
    for path in changed.lines().map(str::trim).filter(|p| !p.is_empty()) {
        if !included_path(path) {
            continue;
        }
        let head = match show(root, &request.head, path)? {
            Some(content) => content,
            None => {
                // 当前树里整个文件都没了，直接记为全部新增行缺失会刷屏；
                // 交由调用者人工确认（文件名照样列出来）。
                reports.push(FileReport {
                    path: path.to_string(),
                    lines: vec!["<file missing from HEAD>".to_string()],
                });
                continue;
            }
        };
        let base = show(root, &request.base, path)?.unwrap_or_default();
        let diff = git(root, &["diff", "-U0", &range, "--", path])?;
        let fork_lines = added_lines(&diff);
        let missing = missing_fork_lines(&base, &fork_lines, &head);
        if !missing.is_empty() {
            reports.push(FileReport {
                path: path.to_string(),
                lines: missing,
            });
        }
    }
    reports.sort_by(|a, b| {
        b.lines
            .len()
            .cmp(&a.lines.len())
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(reports)
}

/// 命令行入口：`xtask audit-vendor --base <rev> --fork <rev> [--head <rev>] [--fail-on-found]`。
pub fn command(root: &Path) -> Result<()> {
    let mut base = None;
    let mut fork = None;
    let mut head = "HEAD".to_string();
    let mut fail_on_found = false;
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => base = Some(args.next().context("--base needs a revision")?),
            "--fork" => fork = Some(args.next().context("--fork needs a revision")?),
            "--head" => head = args.next().context("--head needs a revision")?,
            "--fail-on-found" => fail_on_found = true,
            other => bail!("unknown audit-vendor argument: {other}"),
        }
    }
    let request = AuditRequest {
        base: base.context("audit-vendor needs --base <基准导入 rev>")?,
        fork: fork.context("audit-vendor needs --fork <同步前桌面 rev>")?,
        head,
    };
    let reports = run(root, &request)?;
    if reports.is_empty() {
        println!(
            "audit-vendor: no fork-added functional line is missing from {} (base={}, fork={})",
            request.head, request.base, request.fork
        );
        return Ok(());
    }
    let mut total = 0usize;
    for report in &reports {
        total += report.lines.len();
        println!("\n{}: {} missing line(s)", report.path, report.lines.len());
        for line in report.lines.iter().take(MAX_LINES_PER_FILE) {
            println!("   - {line}");
        }
        if report.lines.len() > MAX_LINES_PER_FILE {
            println!("   ... {} more", report.lines.len() - MAX_LINES_PER_FILE);
        }
    }
    println!(
        "\naudit-vendor: {} line(s) across {} file(s) may be lost desktop patches",
        total,
        reports.len()
    );
    if fail_on_found {
        bail!("audit-vendor found possibly lost desktop patches");
    }
    Ok(())
}

#[cfg(test)]
#[path = "vendor_audit_tests.rs"]
mod tests;
