//! 三段编排：mksrfdata → mkinidata → colm。
//!
//! 每一段都是「跑 → 收日志 → 判成败 → 抽覆盖」。判成败在 `outcome`，
//! 抽覆盖在 `overrides`，本模块只负责把它们串起来并落一份日志。
//!
//! stdout 与 stderr 都要收。gfortran 运行时的错误只走 stderr，所以
//! `FAILURE_MARKERS` 里的 `Fortran runtime error` 与 `Error termination`
//! 在只读 stdout 时**永远不可能命中**；实测 namelist 文件缺失时 stdout 是
//! 0 字节而 stderr 有 302 字节，日志会空得看不出原因。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// 跑一段，只在结束时拿到全部输出。
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
    run_stage_streaming(kernel, stage, namelist, work, artifacts, &mut |_| {})
}

/// 同上，但每读到 stdout 的一行就交给 `on_line` 一次。
///
/// **为什么需要它。** `colm.x` 在一次 528 步的运行里打出 5330 行，其中
/// 528 行是 `TIMESTEP = n | DATE = ...`；GUI 的进度条与日志窗全靠它们。
/// 用 `Command::output()` 的话这些行要等整段跑完才一起到达，进度条从
/// 0 直接跳到 100，日志窗在运行期间一片空白 —— 界面那边的限流、批量发送、
/// `TIMESTEP` 解析全都建在一个永远不会按时到达的输入上。
///
/// 传给 `on_line` 的是**去掉行尾换行的一行**；写进日志的仍是原始字节，
/// 逐字节与 `Command::output()` 那条路相同（下面按 `read_until` 收，
/// 保留行尾符，不重新拼接）。
///
/// stderr 由一个单独的线程读到底。**必须是单独的线程**：两个管道都由本进程
/// 读，如果先把 stdout 读完再读 stderr，子进程在 stderr 管道写满时就会阻塞，
/// 而本进程正等着一个再也不会来的 stdout —— 双方各等各的。
/// 代价是 stderr 不参与逐行回调，它整块在末尾追加。这是可以接受的：
/// gfortran 的运行时错误意味着这一段已经结束了，没有「实时」可言。
pub fn run_stage_streaming(
    kernel: &Kernel,
    stage: Stage,
    namelist: &Path,
    work: &Path,
    artifacts: &[PathBuf],
    on_line: &mut dyn FnMut(&str),
) -> Result<StageReport> {
    let exe = kernel.program(stage.program());
    let mut child = Command::new(&exe)
        .arg(namelist)
        .current_dir(work)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", exe.display()))?;

    let mut err_pipe = child.stderr.take().context("no stderr pipe")?;
    let errs = std::thread::spawn(move || {
        let mut raw = Vec::new();
        let _ = err_pipe.read_to_end(&mut raw);
        raw
    });

    let mut text = String::new();
    {
        let out = child.stdout.take().context("no stdout pipe")?;
        let mut reader = BufReader::new(out);
        let mut raw = Vec::new();
        while reader.read_until(b'\n', &mut raw).unwrap_or(0) > 0 {
            let chunk = String::from_utf8_lossy(&raw);
            on_line(chunk.trim_end_matches(['\n', '\r']));
            text.push_str(&chunk);
            raw.clear();
        }
    }

    let status = child.wait().context("cannot wait for the child")?;
    let raw_err = errs
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))?;
    let stderr = String::from_utf8_lossy(&raw_err);
    if !stderr.is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&stderr);
    }

    let log = work.join(format!("{}.log", stage.program()));
    std::fs::write(&log, text.as_bytes())
        .with_context(|| format!("cannot write {}", log.display()))?;

    Ok(StageReport {
        stage,
        outcome: adjudicate(stage, status.code(), &text, artifacts),
        log,
        overrides: extract(&text),
    })
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
