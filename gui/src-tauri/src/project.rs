//! 算例目录的发现与读写。
//!
//! 一个算例就是一个含 `case.nml` 的目录 —— 不引入独立的索引文件，
//! 因为那会立刻带来「索引与磁盘不一致」这个新问题，而目录本身就是真相。

use std::path::{Path, PathBuf};

use serde::Serialize;

pub(crate) fn validate_case_name(name: &str) -> Result<(), String> {
    colm_case::validate_case_name(name).map_err(|error| format!("{error:#}"))
}

#[derive(Debug, Serialize)]
pub struct CaseEntry {
    pub name: String,
    pub dir: String,
    /// 跑过没有 —— 有 history 文件就算跑过
    pub has_history: bool,
    /// Read from the namelist, not inferred from the selected wizard or comments.
    pub spatial: bool,
}

/// 扫一个目录下的算例（只看一层，不递归）。
#[tauri::command]
pub fn list_cases(root: String) -> Result<Vec<CaseEntry>, String> {
    let root = PathBuf::from(root);
    let mut out = Vec::new();
    let rd = std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    for e in rd.flatten() {
        let d = e.path();
        if !d.join("case.nml").is_file() {
            continue;
        }
        let name = colm_case::case_name(&d.join("case.nml"))
            .ok()
            .filter(|name| validate_case_name(name).is_ok())
            .unwrap_or_else(|| {
                d.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        out.push(CaseEntry {
            // Keep malformed cases discoverable for repair; Study validation
            // reports their parse errors before any model can be launched.
            spatial: colm_case::is_spatial_case(&d.join("case.nml")).unwrap_or(false),
            has_history: history_of(&d, &name).is_some() && !colm_case::results_are_stale(&d),
            dir: d.to_string_lossy().into_owned(),
            name,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 打开已有算例时从 `case.nml`（和跑过时的 `stages.json`）反推出的向导选择，前端据此恢复会话。
#[derive(Debug, Serialize, PartialEq)]
pub struct CaseProfile {
    pub spatial: bool,
    /// `latlon` / `unstructured` / `catchment`；站点为 `None`。
    pub grid: Option<&'static str>,
    /// `IGBP` / `USGS` / `PFT` / `PC`。
    pub subgrid: &'static str,
    /// `vg` / `campbell`。
    pub soil: &'static str,
    pub urban: bool,
    pub lulcc: bool,
    pub bgc: bool,
    pub crop: bool,
    pub methane: bool,
    pub river: bool,
    pub rangecheck: bool,
    pub colmdebug: bool,
    pub srfdatadiag: bool,
    /// 空间算例的 `DEF_domain`（西、东、南、北）。
    pub domain: Option<[f64; 4]>,
    /// 经纬度网格的分辨率（`DEF_GRIDBASED_lon_res/lat_res`）。
    pub resolution: Option<[f64; 2]>,
    /// 上次运行用的内核预设（`stages.json` 里记录的 `preset=`）；没跑过为 `None`。
    pub kernel_preset: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OpenedCase {
    pub entry: CaseEntry,
    /// 算例所在的根目录（`root`），新建算例与查重名都在它下面。
    pub root: String,
    pub profile: CaseProfile,
}

/// 「打开已有算例」：`dir` 必须是含 `case.nml` 的算例目录。
#[tauri::command]
pub fn open_case(dir: String) -> Result<OpenedCase, String> {
    let dir = PathBuf::from(dir);
    let nml = dir.join("case.nml");
    if !nml.is_file() {
        return Err(format!("{} 不是算例目录（没有 case.nml）", dir.display()));
    }
    let text = std::fs::read_to_string(&nml).map_err(|e| format!("{}: {e}", nml.display()))?;
    let stages = std::fs::read_to_string(dir.join("stages.json")).ok();
    let root = dir
        .parent()
        .ok_or_else(|| format!("{} 没有上级目录", dir.display()))?
        .to_string_lossy()
        .into_owned();
    let entry = list_cases(root.clone())?
        .into_iter()
        .find(|c| Path::new(&c.dir) == dir)
        .ok_or_else(|| format!("{} 读不出算例", dir.display()))?;
    let profile = case_profile(&text, entry.spatial, stages.as_deref())?;
    Ok(OpenedCase {
        entry,
        root,
        profile,
    })
}

/// 反推规则：网格看哪种网格字段写了；次网格看 `DEF_USE_PFT/PC`，LCT 再看上次内核是不是 USGS；
/// 甲烷看示踪物名单；河湖汇流只对经纬度与非结构网格有意义（缺省开）。
pub(crate) fn case_profile(
    text: &str,
    spatial: bool,
    stages: Option<&str>,
) -> Result<CaseProfile, String> {
    use crate::config::{character, logical, real};
    let doc = colm_namelist::parse(text).map_err(|e| format!("case.nml: {e}"))?;
    let has = |name: &str| doc.get(name).is_some();
    // 只看算例里显式写了的值：schema 缺省是占位路径，不代表用了这种网格。
    let set = |name: &str| match doc.get(name) {
        Some(colm_namelist::Value::Str(value)) => {
            !value.trim().is_empty() && !value.trim().eq_ignore_ascii_case("null")
        }
        _ => false,
    };
    let grid = if !spatial {
        None
    } else if set("DEF_CatchmentMesh_data") {
        Some("catchment")
    } else if has("DEF_GRIDBASED_lon_res") || has("DEF_GRIDBASED_lat_res") {
        Some("latlon")
    } else {
        Some("unstructured")
    };
    // `stages.json` 每段都记着同一个内核：`preset=latlon;...;args=GRID LULC_USGS ...;macros=...`。
    let kernel = stages
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| {
            value.as_object().and_then(|stages| {
                stages
                    .values()
                    .find_map(|stage| stage.get("kernel")?.as_str().map(str::to_owned))
            })
        });
    let kernel_field = |key: &str| {
        kernel.as_deref().and_then(|k| {
            k.split(';')
                .find_map(|part| part.strip_prefix(key).map(str::to_owned))
        })
    };
    let usgs = kernel_field("macros=").is_some_and(|m| m.split(',').any(|x| x == "LULC_USGS"));
    let subgrid = if logical(&doc, "DEF_USE_PFT") {
        "PFT"
    } else if logical(&doc, "DEF_USE_PC") {
        "PC"
    } else if usgs {
        "USGS"
    } else {
        "IGBP"
    };
    let methane = logical(&doc, "DEF_USE_TRACER")
        && character(&doc, "DEF_TRACER_NAMES").split(',').any(|name| {
            name.trim()
                .trim_matches(['\'', '"'])
                .eq_ignore_ascii_case("CH4")
        });
    let gridded_routing = matches!(grid, Some("latlon" | "unstructured"));
    let finite = |v: f64| v.is_finite().then_some(v);
    let domain = spatial
        .then(|| {
            Some([
                finite(real(&doc, "DEF_domain%edgew"))?,
                finite(real(&doc, "DEF_domain%edgee"))?,
                finite(real(&doc, "DEF_domain%edges"))?,
                finite(real(&doc, "DEF_domain%edgen"))?,
            ])
        })
        .flatten();
    let resolution = (grid == Some("latlon"))
        .then(|| {
            Some([
                finite(real(&doc, "DEF_GRIDBASED_lon_res"))?,
                finite(real(&doc, "DEF_GRIDBASED_lat_res"))?,
            ])
        })
        .flatten();
    Ok(CaseProfile {
        spatial,
        grid,
        subgrid,
        soil: if logical(&doc, "DEF_USE_Campbell_SOIL_MODEL") {
            "campbell"
        } else {
            "vg"
        },
        urban: logical(&doc, "DEF_URBAN_RUN"),
        lulcc: logical(&doc, "DEF_USE_LULCC"),
        bgc: logical(&doc, "DEF_USE_BGC"),
        crop: logical(&doc, "DEF_USE_CROP"),
        methane,
        river: gridded_routing && logical(&doc, "DEF_USE_GridRiverLakeFlow"),
        rangecheck: logical(&doc, "DEF_USE_RangeCheck"),
        colmdebug: logical(&doc, "DEF_USE_CoLMDEBUG"),
        srfdatadiag: logical(&doc, "DEF_USE_SrfdataDiag"),
        domain,
        resolution,
        kernel_preset: kernel_field("preset="),
    })
}

/// 参数已写入、新的 colm 还没有成功时，旧 history 不能跨重启重新变成“可用”。
#[tauri::command]
pub fn mark_results_stale(dirs: Vec<String>) -> Result<(), String> {
    let cases: Vec<PathBuf> = dirs.into_iter().map(PathBuf::from).collect();
    for case in &cases {
        if !case.join("case.nml").is_file() {
            return Err(format!("{} 不是算例目录", case.display()));
        }
    }
    for case in &cases {
        colm_case::mark_results_stale(case)
            .map_err(|error| format!("{}: {error}", case.display()))?;
    }
    Ok(())
}

/// 算例里那个唯一的 `*_hist_*.nc`，没有就是没跑过。
fn history_of(case: &Path, name: &str) -> Option<PathBuf> {
    validate_case_name(name).ok()?;
    let dir = case.join("out").join(name).join("history");
    let mut h: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_hist_") && n.ends_with(".nc"))
        })
        .collect();
    h.sort();
    h.pop()
}

/// 读一份文本文件（前端拿 case.nml 来编辑）。
#[tauri::command]
pub fn read_text(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))
}

// 这里原来有一个 `write_text` —— 前端拿它把改过的 case.nml 写回去。
// 删掉了：参数改动一律由 `config::set_field_batch`
// 在后端读改写，**前端不再持有落盘的能力**。留着一个通用的"写任意路径"
// 命令，等于给"只改了第一个算例"那类 bug 留一条随时可走的路。

#[cfg(test)]
#[path = "project_tests.rs"]
mod project_tests;
