//! 结果的出处：每次 `run` 在 `out/<算例名>/produced-by.json` 写下这批产物是哪一版模型、哪个引擎、在哪里跑出来的。
//! 结果页据此显示出处标签，助手据此回答“这是哪个版本的结果”。
//!
//! `--model-version NAME` 用工作区 NAME 跑（见 `colm_workspace::version`）：先检查就绪，再把同一条 `run` 交给那个
//! 版本自己编出的 `colm-cli`，它旁边的 `colm-rs` 与前处理就是改过的那份。出处由外层这一个进程写。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_workspace::version::Readiness;
use serde_json::{json, Value};

pub(crate) const FILE: &str = "produced-by.json";
/// 交给版本自己的 `colm-cli` 时设上：出处由外层写，里层不写。
const DELEGATED_ENV: &str = "COLM_PROVENANCE_BY_PARENT";

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// 这次 `run` 用的是哪一版。
pub(crate) enum Version {
    Official,
    Workspace(Box<Readiness>),
}

pub(crate) struct Record<'a> {
    pub case: &'a Path,
    pub kernel: &'a Path,
    pub engine: &'a str,
    pub preprocessors: &'a str,
    pub ranks: usize,
    pub stage: Option<&'a str>,
    pub force: bool,
    pub location: &'a str,
    pub version: &'a Version,
    pub started: u64,
}

fn kernel_identity(kernel: &Path) -> Value {
    match colm_kernel::Kernel::open_manifest(kernel) {
        Ok(k) => json!({
            "dir": kernel,
            "identity": k.manifest.identity(),
            "preset": k.manifest.preset,
            "colm_git_sha": k.manifest.colm_git_sha,
        }),
        Err(_) => json!({ "dir": kernel }),
    }
}

fn version_value(version: &Version) -> Value {
    match version {
        Version::Official => json!({
            "kind": "official",
            "label": "官方版",
            "colm_cli": env!("CARGO_PKG_VERSION"),
        }),
        Version::Workspace(r) => json!({
            "kind": "workspace",
            "name": r.name,
            "label": r.name,
            "head": r.head,
            "base_commit": r.base_commit,
            "origin": r.origin,
            "touches": r.touches,
        }),
    }
}

/// 出处记录本身（不含写盘），测试直接看它。
pub(crate) fn value(record: &Record<'_>, ok: bool, error: Option<&str>) -> Value {
    json!({
        "schema": 1,
        "ok": ok,
        "error": error,
        "started": record.started,
        "finished": now(),
        "version": version_value(record.version),
        "engine": record.engine,
        "preprocessors": record.preprocessors,
        "ranks": record.ranks,
        "stage": record.stage,
        "force": record.force,
        "location": { "kind": record.location },
        "kernel": kernel_identity(record.kernel),
    })
}

/// 写 `out/<算例名>/produced-by.json`。尽力而为：算例名都读不出来（多半是在更早的地方就失败了）时不写，
/// 写不了只警告，不改变这次运行的成败。
pub(crate) fn write(record: &Record<'_>, ok: bool, error: Option<&str>) {
    if std::env::var_os(DELEGATED_ENV).is_some() {
        return;
    }
    let result = (|| -> Result<()> {
        let layout = colm_case::Layout::new(record.case);
        let name = colm_case::case_name(&layout.case_nml())?;
        let dir = layout.out().join(name);
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(&value(record, ok, error))?;
        let tmp = dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, dir.join(FILE))?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("warning: cannot record {FILE}: {e:#}");
    }
}

/// 读一个算例最近一次运行的出处；没跑过（或是旧版应用跑的）返回 `None`。
pub(crate) fn read(case: &Path) -> Result<Option<Value>> {
    let layout = colm_case::Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let path = layout.out().join(name).join(FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text)?))
}

pub(crate) fn workspace_root(root: Option<String>) -> PathBuf {
    root.map(PathBuf::from)
        .unwrap_or_else(colm_workspace::layout::default_root)
}

/// 检查版本 `name` 能不能跑；不能就把阻塞项一条条列出来。
pub(crate) fn resolve(
    root: &Path,
    name: &str,
    engine: &str,
    preprocessors: &str,
    kernel: &Path,
) -> Result<Readiness> {
    let readiness = colm_workspace::version::check(root, name, engine, preprocessors, kernel)?;
    if !readiness.ready {
        let blocking: Vec<&str> = readiness
            .items
            .iter()
            .filter(|i| i.blocking)
            .map(|i| i.message.as_str())
            .collect();
        bail!(
            "model version {name} is not ready to run:\n  - {}",
            blocking.join("\n  - ")
        );
    }
    Ok(readiness)
}

/// 用版本自己的 `colm-cli` 跑同一条 `run`。`args` 是去掉了 `--model-version`、`--workspace-root` 与 `--kernel` 的
/// 原参数（不含子命令），内核换成版本解析出的那个。返回子进程的退出码。
pub(crate) fn delegate(readiness: &Readiness, args: &[String]) -> Result<i32> {
    let status = std::process::Command::new(&readiness.cli)
        .arg("run")
        .args(args)
        .arg("--kernel")
        .arg(&readiness.kernel)
        .env(DELEGATED_ENV, "1")
        .status()
        .with_context(|| format!("cannot start {}", readiness.cli.display()))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod provenance_tests;
