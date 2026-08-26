//! 从站点身份与经纬度建立标准站点文件，并返回模式感知的运行就绪审计。
//!
//! **走 sidecar 而不是直接调 `colm_srfdata::site`。** GUI 进程里不能有 netcdf
//! （`Cargo.toml` 那条量化过的注释：`colm-srfdata` 7 个、`colm-cli` 9 个
//! netcdf/hdf5 依赖节点，窗口进程该链接的那几层都是 0），所以建站点文件的事
//! 一律交给 `colm-cli site-new` 子进程，与 `forcing.rs` 的 `probe_forcing`
//! 同一条路。

use serde::{Deserialize, Serialize};

/// `colm-cli site-new --json 1` 的输出。**字段必须与那边拼的 JSON 一一对应。**
///
/// 两边各声明一次是分层的代价：`colm-cli` 在引擎 workspace、GUI 在另一个，
/// 两者不互相依赖。代价由 `sitedata_tests` 里那条拿真 CLI 输出跑的测试兜住 ——
/// 见 `sites_tests.rs`/`forcing_tests.rs` 上同一句话，这里抄一遍是因为道理
/// 完全一样。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteReport {
    pub path: String,
    pub texture: u8,
    pub texture_name: String,
    pub bvic: f64,
    /// 沙 / 粉 / 黏粒的百分比。站点没有自己的土壤剖面时没有意义，值是 (0,0,0)——
    /// `site-new` 造的是只有经纬度的最小文件，从不带土壤剖面。
    pub sand_silt_clay: [f64; 3],
    /// 地类没给就是 `None`（JSON 里是 `null`）—— **这是有意的**，不是缺失。
    /// `colm-case` 立的规矩：地类只在站点文件说得出时才写，说不出就整条不写，
    /// 让 CoLM 走自己的回落路径；写一个猜的值比不写更糟。界面上不该替
    /// `landtype` 塞一个默认值。
    pub landtype: Option<i32>,
    /// 12 个必需字段里，站点文件自己有的那些。`site-new` 造的是只有经纬度的
    /// 最小文件，没有自己的土壤剖面，所以这里恒为空——留着这个字段是因为
    /// `colm-cli` 那边的 `Report` 就是这么分的，界面要能原样显示。
    pub from_site: Vec<String>,
    /// 从 `--rawdata` 栅格抽到的。
    pub from_raster: Vec<String>,
    /// **标称假设，不是这个站点实测的。** 没给 `--rawdata`，或者栅格在这一点
    /// 上也没有数据时，落到这一级——12 个必需字段全从这三个列表里出，
    /// 三者加起来恒为 12。
    pub from_default: Vec<String>,
    /// CoLM 自带查表得到的字段（目前仅 IGBP 冠层高度）。
    pub from_lookup: Vec<String>,
    /// 文件本身没有、运行时需要由 rawdata 提供的完整 mksrfdata 契约。
    pub needs_external: Vec<String>,
    pub site_kind: String,
    pub mode: String,
    /// `self_contained` / `ready_with_rawdata` / `blocked`。
    pub readiness: String,
    pub self_contained: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PftComponentReport {
    pub pft_type: u8,
    pub fraction: f64,
    pub name_zh: String,
    pub name_en: String,
}

fn pft_site_input(case_dir: &std::path::Path) -> Result<(std::path::PathBuf, Option<i64>), String> {
    let file = case_dir.join("case.nml");
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{}: {e:#}", file.display()))?;
    let enabled = |name| matches!(doc.get(name), Some(colm_namelist::Value::Bool(true)));
    if !enabled("DEF_USE_PFT") && !enabled("DEF_USE_PC") {
        return Err("当前算例没有使用 PFT 或 PC 次网格".into());
    }
    let Some(colm_namelist::Value::Str(path)) = doc.get("SITE_fsitedata") else {
        return Err("当前算例没有 SITE_fsitedata".into());
    };
    let path = std::path::Path::new(path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        case_dir.join(path)
    };
    let landtype = match doc.get("SITE_landtype") {
        Some(colm_namelist::Value::Int(value)) if *value >= 0 => Some(*value),
        _ => None,
    };
    Ok((path, landtype))
}

/// Read a case's active PFT composition through the netcdf-owning sidecar.
#[tauri::command]
pub async fn site_pfts(dir: String, kernel_dir: String) -> Result<Vec<PftComponentReport>, String> {
    let (site, landtype) = pft_site_input(std::path::Path::new(&dir))?;
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|error| format!("{error:#}"))?;
    let mut args = vec!["site-pfts".to_string(), site.to_string_lossy().to_string()];
    if kernel.manifest.macros.iter().any(|name| name == "CROP") {
        args.extend(["--crop".to_string(), "1".to_string()]);
    }
    if let Some(landtype) = landtype {
        args.extend(["--landtype".to_string(), landtype.to_string()]);
    }
    let json = crate::sidecar::capture_async(args).await?;
    #[derive(Deserialize)]
    struct Raw {
        pft_type: u8,
        fraction: f64,
    }
    let raw: Vec<Raw> = serde_json::from_str(&json)
        .map_err(|e| format!("colm-cli site-pfts 的输出解析不了：{e}"))?;
    raw.into_iter()
        .map(|p| {
            let names = colm_case::pft::pft_name(p.pft_type)
                .ok_or_else(|| format!("PFT {} 超出常量表范围", p.pft_type))?;
            Ok(PftComponentReport {
                pft_type: p.pft_type,
                fraction: p.fraction,
                name_zh: names.zh.to_string(),
                name_en: names.en.to_string(),
            })
        })
        .collect()
}

/// 拼 `colm-cli site-new --json 1` 的参数列表。
///
/// 抽成同步函数是为了不引入 tokio 就能测 —— `#[tauri::command]` 的
/// `async fn` 不好直接测，命令本身只做薄壳（照 `forcing.rs` 的
/// `build_convert_args`）。
fn build_site_new_args(
    out: &str,
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    rawdata: Option<&str>,
    mode: &str,
    crop: bool,
) -> Vec<String> {
    let mut args = vec![
        "site-new".to_string(),
        "--out".to_string(),
        out.to_string(),
        "--lon".to_string(),
        lon.to_string(),
        "--lat".to_string(),
        lat.to_string(),
    ];
    if let Some(lt) = landtype {
        args.push("--landtype".into());
        args.push(lt.to_string());
    }
    if let Some(r) = rawdata {
        args.push("--rawdata".into());
        args.push(r.to_string());
    }
    if crop {
        args.push("--crop".into());
        args.push("1".into());
    }
    args.push("--mode".into());
    args.push(mode.to_string());
    args.push("--json".into());
    args.push("1".into());
    args
}

/// 从经纬度建立 `site.nc`：先补齐 12 个结构字段，再由 CLI 按当前模式检查
/// 完整 mksrfdata 契约。缺少的科学数据只会列为 rawdata 依赖，不会被编造。
///
/// **地类不给就不传 `--landtype`。** 界面上不该替用户猜——`colm-cli` 那边
/// 已经把这条规矩定死：不给就整条不写，让 CoLM 走自己的回落路径。
#[tauri::command]
pub async fn make_site(
    out: String,
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    rawdata: Option<String>,
    mode: String,
    crop: bool,
) -> Result<SiteReport, String> {
    let args = build_site_new_args(&out, lon, lat, landtype, rawdata.as_deref(), &mode, crop);
    let json = crate::sidecar::capture_async(args).await?;
    serde_json::from_str(&json).map_err(|e| {
        // 说清楚是**解析**失败而不是建站点失败 —— 照 `probe_forcing`/`scan_sites`
        // 的措辞，两者的处置完全不同：前者是我们两边的结构体对不上了，
        // 后者是用户给的经纬度或 rawdata 有问题。
        format!("colm-cli site-new 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
}

fn install_pair(
    staged: [&std::path::Path; 2],
    final_paths: [&std::path::Path; 2],
) -> Result<(), String> {
    if staged[0] == staged[1] || final_paths[0] == final_paths[1] {
        return Err("站点与强迫场必须使用不同文件".into());
    }
    for (source, destination) in staged.iter().zip(final_paths.iter()) {
        if !source.is_file() {
            return Err(format!("待安装产物不存在：{}", source.display()));
        }
        if source == destination {
            return Err("待安装产物不能与目标路径相同".into());
        }
        if destination.exists() && !destination.is_file() {
            return Err(format!("产物目标不是文件：{}", destination.display()));
        }
        let source_parent = source
            .parent()
            .ok_or_else(|| format!("待安装产物没有父目录：{}", source.display()))?
            .canonicalize()
            .map_err(|error| format!("无法检查 {}：{error}", source.display()))?;
        let destination_parent = destination
            .parent()
            .ok_or_else(|| format!("产物目标没有父目录：{}", destination.display()))?
            .canonicalize()
            .map_err(|error| format!("无法检查 {}：{error}", destination.display()))?;
        if source_parent != destination_parent {
            return Err(format!(
                "待安装产物必须先写在目标目录中：{}",
                source.display()
            ));
        }
    }

    let backups = final_paths.map(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        (0..)
            .map(|index| {
                path.with_file_name(format!(
                    ".{name}.colm-backup-{}-{index}",
                    std::process::id()
                ))
            })
            .find(|candidate| !candidate.exists())
            .expect("an unused backup filename")
    });
    let mut preserved = [false; 2];
    for index in 0..2 {
        if final_paths[index].exists() {
            if let Err(error) = std::fs::rename(final_paths[index], &backups[index]) {
                for previous in (0..index).rev().filter(|previous| preserved[*previous]) {
                    let _ = std::fs::rename(&backups[previous], final_paths[previous]);
                }
                return Err(format!(
                    "无法保留旧产物 {}：{error}",
                    final_paths[index].display()
                ));
            }
            preserved[index] = true;
        }
    }

    for index in 0..2 {
        if let Err(error) = std::fs::rename(staged[index], final_paths[index]) {
            let mut rollback_errors = Vec::new();
            for installed in (0..index).rev() {
                if let Err(rollback) = std::fs::rename(final_paths[installed], staged[installed]) {
                    rollback_errors.push(rollback.to_string());
                }
            }
            for previous in (0..2).rev().filter(|previous| preserved[*previous]) {
                if let Err(rollback) = std::fs::rename(&backups[previous], final_paths[previous]) {
                    rollback_errors.push(rollback.to_string());
                }
            }
            return Err(format!(
                "无法安装产物 {}：{error}{}",
                final_paths[index].display(),
                if rollback_errors.is_empty() {
                    String::new()
                } else {
                    format!("；回滚失败：{}", rollback_errors.join("；"))
                }
            ));
        }
    }
    for (index, backup) in backups.iter().enumerate() {
        if preserved[index] {
            let _ = std::fs::remove_file(backup);
        }
    }
    Ok(())
}

/// 批量前处理先把站点与强迫场都写到各自目标目录中的隐藏文件；两份都成功后
/// 才一起替换正式产物，任一份失败都会恢复原文件。
#[tauri::command]
pub fn install_prepared_pair(
    site_staged: String,
    site_final: String,
    forcing_staged: String,
    forcing_final: String,
) -> Result<(), String> {
    install_pair(
        [
            std::path::Path::new(&site_staged),
            std::path::Path::new(&forcing_staged),
        ],
        [
            std::path::Path::new(&site_final),
            std::path::Path::new(&forcing_final),
        ],
    )
}

#[cfg(test)]
#[path = "sitedata_tests.rs"]
mod sitedata_tests;
