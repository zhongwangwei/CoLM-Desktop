//! 从一对经纬度建一份能跑的站点文件（阶段 B，`docs/plan-prep-b.md`）。
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
    args.push("--json".into());
    args.push("1".into());
    args
}

/// 从经纬度建一份能跑的 `site.nc`：12 个必需字段由 rawdata 栅格或标称假设
/// 补齐，每个都带着来自哪里的说明。
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
) -> Result<SiteReport, String> {
    let args = build_site_new_args(&out, lon, lat, landtype, rawdata.as_deref());
    let json = crate::sidecar::capture(&args)?;
    serde_json::from_str(&json).map_err(|e| {
        // 说清楚是**解析**失败而不是建站点失败 —— 照 `probe_forcing`/`scan_sites`
        // 的措辞，两者的处置完全不同：前者是我们两边的结构体对不上了，
        // 后者是用户给的经纬度或 rawdata 有问题。
        format!("colm-cli site-new 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
}

#[cfg(test)]
#[path = "sitedata_tests.rs"]
mod sitedata_tests;
