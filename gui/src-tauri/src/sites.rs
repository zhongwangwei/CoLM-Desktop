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

/// 扫描结果。**空结果要能自己解释** —— 用户指错目录（比如指了
/// `Forcing` 而站点文件在 `Sitedata`）时，一个空列表什么都没说，
/// 而那正是第一次用最容易发生的事。
#[derive(Serialize)]
pub struct ScanResult {
    pub sites: Vec<Site>,
    /// 一句人话，说清楚找的是什么、以及旁边哪个目录像是对的。
    pub hint: Option<String>,
    /// 建议改用的目录。界面据此给一个「改用它」的按钮 ——
    /// 只提示不给按钮，等于让人再走一遍选择流程。
    pub suggest: Option<String>,
}

/// 找一个**含有站点文件**的目录：先看自己的子目录，再看兄弟目录。
///
/// 只看一层。再深就会在一棵大树上乱翻，而这两层已经覆盖了实际会发生的
/// 两种指错：指了数据集根目录（`Sitedata` 是子目录），或指了同级的
/// `Forcing` / `Observation`（`Sitedata` 是兄弟）。
fn looks_like_sitedata(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with("_site.nc") || n.ends_with("_site_v1.nc"))
        })
    })
}

fn suggest_nearby(dir: &std::path::Path) -> Option<String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        candidates.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    if let Some(parent) = dir.parent() {
        if let Ok(rd) = std::fs::read_dir(parent) {
            candidates.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates.retain(|p| p != dir && looks_like_sitedata(p));
    // 名字叫 Sitedata 的优先。父目录下可能不止一个含站点文件的目录
    // （实测：临时目录里堆着别的测试留下的东西），那时按字典序挑第一个
    // 是**任意的** —— 而一条说不出理由的建议，用户照做之后只会更糊涂。
    candidates
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("sitedata"))
        })
        .or_else(|| candidates.first())
        .map(|p| p.display().to_string())
}

/// 扫一个 `Sitedata` 目录。
///
/// `quick` 跳过强迫场文件，只读站点文件。实测 90 个 PLUMBER2 站点：
/// 完整 0.35 秒、`--quick` 0.07 秒（macOS ARM，热缓存）。两者都够快到
/// 可以同步调，`quick` 留给网络盘或机械盘。
#[tauri::command]
pub async fn scan_sites(
    dir: String,
    forcing_dir: Option<String>,
    quick: bool,
) -> Result<ScanResult, String> {
    let mut args = vec!["scan".to_string(), "--dir".into(), dir.clone()];
    if let Some(forcing_dir) = forcing_dir.filter(|path| !path.trim().is_empty()) {
        args.push("--forcing-dir".into());
        args.push(forcing_dir);
    }
    if quick {
        args.push("--quick".into());
        args.push("1".into());
    }
    let json = crate::sidecar::capture_async(args).await?;
    let sites: Vec<Site> = serde_json::from_str(&json).map_err(|e| {
        // 说清楚是**解析**失败而不是扫描失败 —— 两者的处置完全不同：
        // 前者是我们两边的结构体对不上了，后者是用户给的目录有问题。
        format!("colm-cli scan 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })?;
    if !sites.is_empty() {
        return Ok(ScanResult {
            sites,
            hint: None,
            suggest: None,
        });
    }
    let p = std::path::Path::new(&dir);
    let suggest = suggest_nearby(p);
    let hint = Some(match &suggest {
        Some(s) => format!(
            "这个目录里没有站点文件（找的是 *_site.nc 或 *_site_v1.nc）。\
             旁边的 {s} 里有 —— 是不是要选它？"
        ),
        None => "这个目录里没有站点文件（找的是 *_site.nc 或 *_site_v1.nc）。\
                 PLUMBER2 的站点文件在 Sitedata 目录里，Forcing 与 Observation 里的是别的东西。"
            .to_string(),
    });
    Ok(ScanResult {
        sites,
        hint,
        suggest,
    })
}

#[cfg(test)]
#[path = "sites_tests.rs"]
mod sites_tests;
