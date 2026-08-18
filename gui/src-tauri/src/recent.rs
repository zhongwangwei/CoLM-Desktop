//! 记住上次用过的目录。
//!
//! 界面上有五个要填绝对路径的框，第一次打开全是空的。手打一次
//! `/Users/…/PLUMBER2s/Sitedata` 已经够烦，每次打开都手打就没人会用了。
//!
//! **只记目录，不记算例内容。** 这里存的是「上次在哪儿找数据」，
//! 是使用习惯不是配置；配置在 `case.nml` 里，预设在 `presets/`。
//! 三者混在一起的话，删掉一个会连带影响另外两个。

use std::collections::BTreeMap;

use tauri::Manager;

fn path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let d = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    Ok(d.join("recent.json"))
}

/// 全部记住的值，键是界面上那个框的 id。
///
/// 读失败一律当空 —— **这份东西没有一项是必须的**，为它报错、或者更糟，
/// 让界面起不来，都不成比例。
#[tauri::command]
pub fn load_recent(app: tauri::AppHandle) -> BTreeMap<String, String> {
    path(&app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 记一个值。整份读改写 —— 五个键而已。
#[tauri::command]
pub fn save_recent(app: tauri::AppHandle, key: String, value: String) -> Result<(), String> {
    // 空值不记：它会把上次那个有用的值覆盖掉，而用户清空一个框
    // 通常是想重新填，不是想忘掉历史。
    if value.trim().is_empty() {
        return Ok(());
    }
    let mut all = load_recent(app.clone());
    all.insert(key, value);
    let p = path(&app)?;
    std::fs::write(
        &p,
        serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", p.display()))
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod recent_tests;
