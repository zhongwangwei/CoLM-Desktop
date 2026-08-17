//! 补齐一个 PLUMBER2 站点文件，产出 CoLM 单点能直接读的增广站点文件。
//!
//! 用法: site-fill <站点文件> <输出> [rawdata 目录] [Observation 文件]
//!
//! 取值优先级是**站点自有 > 栅格 > 模块默认**，输出会分三行列清楚哪个字段
//! 走了哪条路。不给 rawdata 目录时，栅格那部分退到 CoLM 的模块默认值 ——
//! 那样的文件能跑，但土壤反照率与地形是名义值。
//!
//! Observation 文件不给时按 PLUMBER2 的目录约定推：把站点文件路径里的
//! `Sitedata` 换成 `Observation`、`_site.nc` 换成 `_Flux.nc`。推不到就跳过，
//! 高程退到栅格。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colm_srfdata::site::{fill, missing_fields};

/// 按 PLUMBER2 的目录约定，从站点文件路径推出同站的 Observation 文件。
fn guess_observation(site: &Path) -> Option<PathBuf> {
    let name = site.file_name()?.to_str()?;
    let obs_name = name.strip_suffix("_site.nc")?.to_string() + "_Flux.nc";
    let dir = site.parent()?.parent()?.join("Observation");
    let p = dir.join(obs_name);
    p.exists().then_some(p)
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let src = PathBuf::from(
        args.next()
            .context("usage: site-fill <site.nc> <out.nc> [rawdata] [observation.nc]")?,
    );
    let dst = PathBuf::from(
        args.next()
            .context("usage: site-fill <site.nc> <out.nc> [rawdata] [observation.nc]")?,
    );
    let raw = args.next().map(PathBuf::from);
    let obs = args
        .next()
        .map(PathBuf::from)
        .or_else(|| guess_observation(&src));

    let missing = missing_fields(&src)?;
    println!(
        "{} is missing {} required field(s)",
        src.display(),
        missing.len()
    );

    let r = fill(&src, &dst, raw.as_deref(), obs.as_deref())?;
    println!(
        "soil texture: {} ({}), BVIC {} from sand {:.2}% / silt {:.2}% / clay {:.2}%",
        r.texture, r.texture_name, r.bvic, r.fine_earth.0, r.fine_earth.1, r.fine_earth.2
    );
    // 栅格与站点自己的土壤不一致是常态（不同的土壤产品），但不该藏起来。
    if let Some(t) = r.raster_texture {
        if t != r.texture {
            println!(
                "note: CoLM's own raster says {} ({}); the site's own soil wins",
                t,
                colm_srfdata::CLASS_NAMES[(t - 1) as usize]
            );
        }
    }
    if !r.from_site.is_empty() {
        println!("from site   : {}", r.from_site.join(", "));
    }
    if !r.from_raster.is_empty() {
        println!("from raster : {}", r.from_raster.join(", "));
    }
    if !r.from_default.is_empty() {
        println!(
            "from default: {}  <-- nominal values, not measured at this site",
            r.from_default.join(", ")
        );
    }
    println!("wrote {}", dst.display());
    Ok(())
}
