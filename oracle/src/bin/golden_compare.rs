//! 比对两个 CoLM history 文件：变量数据、维度、属性。
//!
//! 用法: golden-compare <golden.nc> <produced.nc> [--tolerances <表.toml>]
//!
//! 默认**逐位**比较（里程碑 1 的判据）。给了 `--tolerances` 就按
//! `tolerances.toml` 的分层判值；schema 一侧两者一样严。
//!
//! 本文件只负责取参数与打印；比对逻辑在 `oracle::judge` 与 `oracle::tier_compare`，
//! 那里都有自动化测试。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use oracle::judge::{compare, VOLATILE_ATTRIBUTES};
use oracle::tier_compare::{compare_within, report};
use oracle::tolerances::Tolerances;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let a_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced> [--tolerances <table>]")?,
    );
    let b_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced> [--tolerances <table>]")?,
    );
    let mut tolerances = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--tolerances" => {
                let path = args
                    .next()
                    .context("--tolerances needs the tolerance table path")?;
                tolerances = Some(Tolerances::load(path)?);
            }
            other => bail!("unknown argument {other:?}"),
        }
    }
    if let Some(tolerances) = &tolerances {
        let outcome = compare_within(&a_path, &b_path, tolerances)?;
        return report(&outcome, tolerances);
    }

    let report = compare(&a_path, &b_path)?;
    if report.is_identical() {
        println!(
            "identical: {} variables, {} dimensions (ignoring {:?})",
            report.compared, report.dimensions, VOLATILE_ATTRIBUTES
        );
        return Ok(());
    }
    eprintln!("{} problem(s):", report.problems.len());
    for p in &report.problems {
        eprintln!("  {p}");
    }
    bail!("golden comparison failed");
}
