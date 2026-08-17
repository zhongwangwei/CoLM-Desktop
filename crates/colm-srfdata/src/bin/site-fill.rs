//! 补齐一个 PLUMBER2 站点文件，产出 CoLM 单点能直接读的增广站点文件。
//!
//! 用法: site-fill <站点文件> <输出> [rawdata 目录]
//!
//! 不给 rawdata 目录时，8 个栅格字段退到 CoLM 的模块默认值，
//! 并在输出里逐个说明 —— 那样的文件能跑，但土壤反照率与地形是名义值。

use std::path::PathBuf;

use anyhow::{Context, Result};
use colm_srfdata::site::{fill, missing_fields};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let src = PathBuf::from(
        args.next()
            .context("usage: site-fill <site.nc> <out.nc> [rawdata]")?,
    );
    let dst = PathBuf::from(
        args.next()
            .context("usage: site-fill <site.nc> <out.nc> [rawdata]")?,
    );
    let raw = args.next().map(PathBuf::from);

    let missing = missing_fields(&src)?;
    println!(
        "{} is missing {} required field(s)",
        src.display(),
        missing.len()
    );

    let r = fill(&src, &dst, raw.as_deref())?;
    println!(
        "soil texture: {} ({}), BVIC {} from sand {:.2}% / silt {:.2}% / clay {:.2}%",
        r.texture, r.texture_name, r.bvic, r.fine_earth.0, r.fine_earth.1, r.fine_earth.2
    );
    println!("from raster : {}", r.from_raster.join(", "));
    if !r.from_default.is_empty() {
        println!(
            "from default: {}  <-- nominal values, not measured at this site",
            r.from_default.join(", ")
        );
    }
    println!("wrote {}", dst.display());
    Ok(())
}
