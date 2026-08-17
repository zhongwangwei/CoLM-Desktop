//! 比对两个 CoLM history 文件：变量数据、维度、属性。
//!
//! 用法: golden-compare <golden.nc> <produced.nc>
//!
//! 本文件只负责取参数与打印；比对逻辑在 `oracle::judge`，那里有自动化测试。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use oracle::judge::{compare, VOLATILE_ATTRIBUTES};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let a_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced>")?,
    );
    let b_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced>")?,
    );

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
