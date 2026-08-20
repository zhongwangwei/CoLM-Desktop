//! 列出一个 Sitedata 目录里的站点，并读出它们的经纬度。
//!
//! 两个抽取工具（`extract-urban-soil` 与 `extract-urban-extra`）都要做这件事，
//! 而**站点集合与经纬度必须是同一份**：两张表按经纬度对齐，两边各读各的话，
//! 一处改了另一处不跟着改就会错位 —— 而错位之后两张表仍然都能编过。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 一个站点：代号、文件、经纬度。
pub struct Site {
    pub name: String,
    pub file: PathBuf,
    pub lon: f64,
    pub lat: f64,
}

/// `<Sitedata>/*_site_v1.nc`，按站点名排序。
///
/// 经纬度走 [`colm_srfdata::site::location`]，而不是在这里另读一遍：
/// Urban-PLUMBER 的 `longitude` 是 `(y, x)` 而 PLUMBER2 的是 0 维标量，
/// 那个函数已经把这条差异处理掉了。
pub fn urban_sites(dir: &Path) -> Result<Vec<Site>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        let Some(stem) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(name) = stem.strip_suffix("_site_v1.nc") else {
            continue;
        };
        let loc = colm_srfdata::site::location(&path)
            .with_context(|| format!("cannot read the location of {}", path.display()))?;
        out.push(Site {
            name: name.to_string(),
            file: path,
            lon: loc.lon,
            lat: loc.lat,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
