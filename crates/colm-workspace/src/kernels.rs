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
                        && ["colm.x", "mkinidata.x", "mksrfdata.x"]
                            .iter()
                            .all(|p| dir.join(p).is_file() || dir.join(p.replace(".x", ".exe")).is_file())
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
            out.push(Experimental {
                label: format!("实验内核：{}（未审阅）· {preset}", workspace.info.name),
                workspace: workspace.info.name.clone(),
                dir: workspace.kernels().join(&preset),
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
