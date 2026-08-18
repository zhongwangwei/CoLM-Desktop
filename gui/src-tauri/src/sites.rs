//! 站点目录扫描。
//!
//! 真正读文件的是 `colm-cli scan`（那边要 netcdf），这里只负责调它、
//! **把结果解析成有类型的东西**，再交给前端。
//!
//! 为什么要在这里再声明一遍结构体，而不是把 JSON 原样透传：
//! 透传的话，`colm-cli` 哪天少写一个字段，前端会拿到 `undefined` 然后
//! 在界面上显示成空白 —— 一个没人报错的故障。解析成结构体则当场失败，
//! 而 `sites_tests` 里那条拿**真 CLI 输出**跑的测试会先一步红。

use serde::{Deserialize, Serialize};

/// 一个站点。**字段必须与 `colm-cli` 的 `SiteInfo` 一一对应。**
///
/// 两边各声明一次是分层的代价：`colm-cli` 在引擎 workspace、GUI 在另一个，
/// 两者不互相依赖（`design.md` §4.2）。代价由
/// `it_parses_what_the_real_cli_prints` 兜住 —— 那条跑真 CLI、解析真输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub name: String,
    pub site_file: String,
    /// 找不到强迫场就跑不了。界面据此置灰「运行」并说明原因，
    /// 而不是等用户点下去才报错。
    pub met_file: Option<String>,
    /// 没有观测就不能自动评估。这一条决定评估按钮的死活。
    pub obs_file: Option<String>,
    /// 城市站点（站点文件不带 `IGBP_classification`）。城市算例必须给
    /// rawdata 与 runtime 目录，界面据此决定问不问。
    pub urban: bool,
    pub lon: f64,
    pub lat: f64,
    pub landtype: Option<i32>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub step_seconds: Option<f64>,
    /// 读这个站点时出的问题。**一个坏文件不该毁掉整次扫描** ——
    /// 其余站点照常列出，坏的那个把原文带上来。
    pub problem: Option<String>,
}

/// 扫一个 `Sitedata` 目录。
///
/// `quick` 跳过强迫场文件，只读站点文件。实测 90 个 PLUMBER2 站点：
/// 完整 0.35 秒、`--quick` 0.07 秒（macOS ARM，热缓存）。两者都够快到
/// 可以同步调，`quick` 留给网络盘或机械盘。
#[tauri::command]
pub async fn scan_sites(dir: String, quick: bool) -> Result<Vec<Site>, String> {
    let mut args = vec!["scan".to_string(), "--dir".into(), dir];
    if quick {
        args.push("--quick".into());
        args.push("1".into());
    }
    let json = crate::sidecar::capture(&args)?;
    serde_json::from_str(&json).map_err(|e| {
        // 说清楚是**解析**失败而不是扫描失败 —— 两者的处置完全不同：
        // 前者是我们两边的结构体对不上了，后者是用户给的目录有问题。
        format!("colm-cli scan 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
}

#[cfg(test)]
#[path = "sites_tests.rs"]
mod sites_tests;
