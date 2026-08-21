//! 算例目录的发现与读写。
//!
//! 一个算例就是一个含 `case.nml` 的目录 —— 不引入独立的索引文件，
//! 因为那会立刻带来「索引与磁盘不一致」这个新问题，而目录本身就是真相。

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CaseEntry {
    pub name: String,
    pub dir: String,
    /// 跑过没有 —— 有 history 文件就算跑过
    pub has_history: bool,
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
        let name = colm_case::case_name(&d.join("case.nml")).unwrap_or_else(|_| {
            d.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
        out.push(CaseEntry {
            has_history: history_of(&d, &name).is_some(),
            dir: d.to_string_lossy().into_owned(),
            name,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 算例里那个唯一的 `*_hist_*.nc`，没有就是没跑过。
fn history_of(case: &Path, name: &str) -> Option<PathBuf> {
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
