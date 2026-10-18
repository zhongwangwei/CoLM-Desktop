//! 记住上次用过的目录。
//!
//! 界面上有五个要填绝对路径的框，第一次打开全是空的。手打一次
//! `/Users/…/PLUMBER2s/Sitedata` 已经够烦，每次打开都手打就没人会用了。
//!
//! **只记目录，不记算例内容。** 这里存的是「上次在哪儿找数据」，
//! 是使用习惯不是配置；模型配置始终在各算例的 `case.nml` 里。
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
        .and_then(|p| read_recent(&p).ok())
        .unwrap_or_default()
}

/// The remembered values; a missing file is empty, an unreadable one is an error.
fn read_recent(path: &std::path::Path) -> Result<BTreeMap<String, String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Add one value without ever replacing a file it could not read, and replace it atomically,
/// so a truncated or unreadable recent.json never wipes every remembered folder.
fn remember(path: &std::path::Path, key: String, value: String) -> Result<(), String> {
    let mut all =
        read_recent(path).map_err(|e| format!("最近使用的目录记录无法读取，未覆盖：{e}"))?;
    all.insert(key, value);
    crate::config::write_export_text_atomic(
        path,
        serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?,
    )
}

/// 记一个值。整份读改写 —— 五个键而已。
#[tauri::command]
pub fn save_recent(app: tauri::AppHandle, key: String, value: String) -> Result<(), String> {
    // 空值不记：它会把上次那个有用的值覆盖掉，而用户清空一个框
    // 通常是想重新填，不是想忘掉历史。
    if value.trim().is_empty() {
        return Ok(());
    }
    remember(&path(&app)?, key, value)
}

#[cfg(test)]
#[path = "recent_tests.rs"]
mod recent_tests;

/// 选一个文件夹。返回 `None` 表示用户取消 —— **取消不是错误**，
/// 报成错误会让状态栏冒出一行红字，而用户只是改了主意。
///
/// 起始目录用上次记住的那个：选路径这件事十有八九是在同一棵树里换个位置。
#[tauri::command]
pub async fn pick_folder(app: tauri::AppHandle, key: String) -> Option<String> {
    #[cfg(feature = "e2e-driver")]
    if let Some(path) = crate::e2e_driver::take_pick() {
        return Some(path);
    }
    let start = load_recent(app).get(&key).cloned();
    let mut d = rfd::AsyncFileDialog::new();
    if let Some(s) = start.filter(|s| std::path::Path::new(s).is_dir()) {
        d = d.set_directory(s);
    }
    d.pick_folder()
        .await
        .map(|h| h.path().display().to_string())
}

/// 选一个文件。`filter` 是逗号分隔的后缀名，如 `nc,nc4`。
#[tauri::command]
pub async fn pick_file(
    app: tauri::AppHandle,
    key: String,
    filter: Option<String>,
) -> Option<String> {
    #[cfg(feature = "e2e-driver")]
    if let Some(path) = crate::e2e_driver::take_pick() {
        return Some(path);
    }
    // 起始目录取上次那个**值所在的目录** —— 记住的是文件路径时，
    // 直接当目录用会打不开。
    let start = load_recent(app).get(&key).cloned().and_then(|s| {
        let p = std::path::PathBuf::from(&s);
        if p.is_dir() {
            Some(p)
        } else {
            p.parent().filter(|d| d.is_dir()).map(|d| d.to_path_buf())
        }
    });
    let mut d = rfd::AsyncFileDialog::new();
    if let Some(s) = start {
        d = d.set_directory(s);
    }
    if let Some(f) = filter.as_deref() {
        let extensions: Vec<&str> = f
            .split(',')
            .map(str::trim)
            .filter(|extension| !extension.is_empty())
            .collect();
        if !extensions.is_empty() {
            d = d.add_filter("data", &extensions);
        }
    }
    d.pick_file().await.map(|h| h.path().display().to_string())
}

fn text_export_filter(suggested_name: &str, mime_type: &str) -> Result<&'static str, String> {
    if suggested_name.is_empty()
        || suggested_name.trim() != suggested_name
        || matches!(suggested_name, "." | "..")
        || suggested_name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
    {
        return Err("导出文件名必须是单个文件名，不能包含路径".into());
    }
    match mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "text/csv" => Ok("csv"),
        "application/json" => Ok("json"),
        "text/markdown" => Ok("md"),
        "text/html" => Ok("html"),
        "text/plain" => Ok("txt"),
        _ => Err(format!("不支持的文本导出类型：{mime_type}")),
    }
}

fn finish_text_export(
    selected: Option<std::path::PathBuf>,
    content: String,
) -> Result<Option<String>, String> {
    let Some(path) = selected else {
        return Ok(None);
    };
    crate::config::write_export_text_atomic(&path, content)?;
    Ok(Some(path.display().to_string()))
}

/// Native save authorization avoids WebKit's sandboxed anchor-download path.
#[tauri::command]
pub async fn export_text_file(
    suggested_name: String,
    content: String,
    mime_type: String,
) -> Result<Option<String>, String> {
    let extension = text_export_filter(&suggested_name, &mime_type)?;
    #[cfg(feature = "e2e-driver")]
    if let Some(dir) = crate::e2e_driver::save_dir() {
        let path = dir.join(&suggested_name);
        return tauri::async_runtime::spawn_blocking(move || {
            finish_text_export(Some(path), content)
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    let selected = rfd::AsyncFileDialog::new()
        .set_title("导出文件")
        .set_file_name(suggested_name)
        .add_filter(extension.to_ascii_uppercase(), &[extension])
        .save_file()
        .await
        .map(|file| file.path().to_path_buf());
    tauri::async_runtime::spawn_blocking(move || finish_text_export(selected, content))
        .await
        .map_err(|error| error.to_string())?
}
