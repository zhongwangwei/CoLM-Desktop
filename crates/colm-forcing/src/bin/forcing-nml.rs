//! 从一个 PLUMBER2 强迫场文件生成 CoLM 的 nl_colm_forcing。
//!
//! 用法: forcing-nml <Met 文件> [输出路径]
//!
//! 不给输出路径就打到标准输出。契约问题一律打到标准错误并以非零码退出 ——
//! 其中「模拟窗口跑过强迫场末端」那一种，CoLM 自己只会警告然后继续跑。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_forcing::{check, render, summarize, ForcingSpec};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let met = PathBuf::from(
        args.next()
            .context("usage: forcing-nml <Met.nc> [out.nml]")?,
    );
    let out = args.next().map(PathBuf::from);

    let summary = summarize(&met)?;
    let problems = check(&summary, None);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("  {p}");
        }
        bail!("{} problem(s) with {}", problems.len(), met.display());
    }

    let dir = met
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let file = met
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .to_string();
    let text = render(&ForcingSpec {
        dir,
        file,
        met: summary.clone(),
    });

    match out {
        Some(p) => {
            std::fs::write(&p, &text).with_context(|| format!("cannot write {}", p.display()))?;
            eprintln!(
                "wrote {} covering {}-{:02} to {}-{:02}, step {} s",
                p.display(),
                summary.start.year,
                summary.start.month,
                summary.end().year,
                summary.end().month,
                summary.timestep_hint()
            );
        }
        None => print!("{text}"),
    }
    Ok(())
}
