//! 三段编排：mksrfdata → mkinidata → colm。
//!
//! 每一段都是「跑 → 收日志 → 判成败 → 抽覆盖」。判成败在 `outcome`，
//! 抽覆盖在 `overrides`，本模块只负责把它们串起来并落一份日志。
//!
//! stdout 与 stderr 都要收。gfortran 运行时的错误只走 stderr，所以
//! `FAILURE_MARKERS` 里的 `Fortran runtime error` 与 `Error termination`
//! 在只读 stdout 时**永远不可能命中**；实测 namelist 文件缺失时 stdout 是
//! 0 字节而 stderr 有 302 字节，日志会空得看不出原因。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::manifest::Kernel;
use crate::outcome::{adjudicate, Outcome, Stage};
use crate::overrides::{extract, Override};

/// 一段跑完之后知道的一切。
#[derive(Debug)]
pub struct StageReport {
    pub stage: Stage,
    pub outcome: Outcome,
    /// 日志落盘的位置。失败时报给用户看的就是它。
    pub log: PathBuf,
    /// CoLM 在这一段里声明的静默覆盖。
    pub overrides: Vec<Override>,
}

impl StageReport {
    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, Outcome::Succeeded)
    }
}

/// 跑一段。
///
/// `artifacts` 是这一段必须产出的文件，交给 `adjudicate` 做硬校验 ——
/// 必须列到**文件**，不能只列目录：目录在程序写任何东西之前就已存在，
/// 于是「跑完了但什么都没写」恰好抓不到。
pub fn run_stage(
    kernel: &Kernel,
    stage: Stage,
    namelist: &Path,
    work: &Path,
    artifacts: &[PathBuf],
) -> Result<StageReport> {
    let exe = kernel.program(stage.program());
    let output = Command::new(&exe)
        .arg(namelist)
        .current_dir(work)
        .output()
        .with_context(|| format!("failed to spawn {}", exe.display()))?;

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&stderr);
    }

    let log = work.join(format!("{}.log", stage.program()));
    std::fs::write(&log, text.as_bytes())
        .with_context(|| format!("cannot write {}", log.display()))?;

    Ok(StageReport {
        stage,
        outcome: adjudicate(stage, output.status.code(), &text, artifacts),
        log,
        overrides: extract(&text),
    })
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
