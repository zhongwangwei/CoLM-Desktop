//! 实验内核的登记（docs/design-ai-assistant.md 第 5.4 节）：工作区里编出的内核，编译与测试在当前提交上通过后，
//! 内核下拉框里出现“实验内核：<工作区名>（未审阅）”。阶段指纹本来就记录内核身份，所以用它跑出的结果不会和正式
//! 结果混在一起。设为默认只能由人在界面上做（D 级）。

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::layout::Workspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Experimental {
    pub workspace: String,
    pub preset: String,
    pub dir: PathBuf,
    pub head: String,
    /// 下拉框里显示的名字。
    pub label: String,
    /// 回归有没有做过（界面上提示“未回归”）。
    pub regression_done: bool,
    pub reviewed: bool,
    /// 清单里的身份：界面按编译宏匹配内核，所以要带上。
    pub generator_args: String,
    pub macros: Vec<String>,
    pub colm_git_sha: String,
    pub platform: String,
}

/// 读内核清单里界面要用的几项。
fn manifest_fields(dir: &Path) -> (String, Vec<String>, String, String) {
    let manifest: serde_json::Value = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let text = |key: &str| manifest[key].as_str().unwrap_or_default().to_owned();
    (
        text("generator_args"),
        manifest["macros"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        text("colm_git_sha"),
        text("platform"),
    )
}

/// 工作区里已编出的内核目录（有完整的三个程序与清单）。
pub fn built_presets(workspace: &Workspace) -> Vec<String> {
    let mut presets: Vec<String> = std::fs::read_dir(workspace.kernels())
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let dir = e.path();
                    dir.join("manifest.json").is_file()
                        && ["colm.x", "mkinidata.x", "mksrfdata.x"].iter().all(|p| {
                            dir.join(p).is_file() || dir.join(p.replace(".x", ".exe")).is_file()
                        })
                })
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    presets.sort();
    presets
}

/// 根目录下所有可登记的实验内核。
pub fn experimental(root: &Path) -> Result<Vec<Experimental>> {
    let mut out = Vec::new();
    for summary in Workspace::list(root)? {
        let workspace = Workspace::open(root, &summary.name)?;
        let head = workspace.head()?;
        if !workspace.info.gates.may_register(&head) {
            continue;
        }
        let regression_done = workspace
            .info
            .gates
            .regression
            .as_ref()
            .is_some_and(|r| r.commit == head);
        for preset in built_presets(&workspace) {
            // 这个预设自己的编译记录要在当前提交上通过。
            let compiled = workspace
                .info
                .gates
                .kernels
                .get(&preset)
                .is_some_and(|g| g.ok && g.commit == head);
            if !compiled {
                continue;
            }
            let dir = workspace.kernels().join(&preset);
            let (generator_args, macros, colm_git_sha, platform) = manifest_fields(&dir);
            out.push(Experimental {
                label: format!("实验内核：{}（未审阅）· {preset}", workspace.info.name),
                workspace: workspace.info.name.clone(),
                dir,
                generator_args,
                macros,
                colm_git_sha,
                platform,
                head: head.clone(),
                preset,
                regression_done,
                reviewed: false,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "kernels_tests.rs"]
mod kernels_tests;
