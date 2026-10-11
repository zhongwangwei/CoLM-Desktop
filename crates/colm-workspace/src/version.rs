//! 模型版本：工作流第 3 步“运行方式”里的“用哪一版模型”。
//!
//! 版本只有两种：官方版（应用自带的引擎与 `kernels/` 下的正式内核），以及一个工作区（“修改版”）。修改版跑的是
//! **它自己编出的全部程序**：`bin/` 下的 `colm-cli` 连同旁边的 `colm-rs` 与两个 Rust 前处理，Fortran 内核按需取它
//! `kernels/` 下同一预设的那份。这样“改了代码就跑改了的代码”，和 `ws-run` 走的是同一条路。
//!
//! [`check`] 只读：说明这个版本现在能不能按选定的引擎跑、缺什么、该怎么补，给界面的就绪清单和 `colm-cli run
//! --model-version` 共用。

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::layout::Workspace;

/// Fortran 内核的源码与编译脚本：改了它们，正式内核就不能代表这个版本。
const KERNEL_PATHS: &[&str] = &["vendor/CoLM202X/", "oracle/scripts/build_kernel.sh"];

fn under(path: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| path == *prefix || (prefix.ends_with('/') && path.starts_with(prefix)))
}

/// 改动落在哪里：Rust 引擎（计算部分）、Fortran 内核，或都没碰（只改了界面、助手、文档等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Touches {
    pub engine: bool,
    pub kernel: bool,
}

pub fn touches<'a>(paths: impl IntoIterator<Item = &'a str>) -> Touches {
    let mut out = Touches {
        engine: false,
        kernel: false,
    };
    for path in paths {
        if under(path, KERNEL_PATHS) {
            out.kernel = true;
        } else if crate::gates::parity_needed([path]) {
            out.engine = true;
        }
    }
    out
}

/// 就绪清单的一项。`blocking` 为真时这个版本现在跑不了。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    /// 稳定的代号，界面按它给出操作按钮。
    pub code: &'static str,
    pub ok: bool,
    pub blocking: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Readiness {
    pub name: String,
    pub head: String,
    pub base_commit: String,
    pub origin: String,
    /// 这个版本的目的（见 `Info::note`）。
    pub note: String,
    pub based_on: Option<String>,
    pub touches: Touches,
    /// 能跑吗（没有任何阻塞项）。
    pub ready: bool,
    pub items: Vec<Item>,
    /// 这个版本自己的 `colm-cli`。
    pub cli: PathBuf,
    /// 跑这个版本要用的内核目录：Fortran 没改时就是传进来的正式内核。
    pub kernel: PathBuf,
    /// 内核预设（来自正式内核的清单）。
    pub preset: String,
}

fn manifest_preset(kernel: &Path) -> Option<String> {
    let text = std::fs::read_to_string(kernel.join("manifest.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value["preset"].as_str().map(str::to_owned)
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// 检查工作区 `name` 能不能用 `engine`（rust/fortran）与 `preprocessors`（rust/fortran）跑，`official_kernel` 是
/// 按算例配置选出的正式内核（决定预设）。
pub fn check(
    root: &Path,
    name: &str,
    engine: &str,
    preprocessors: &str,
    official_kernel: &Path,
) -> Result<Readiness> {
    let workspace = Workspace::open(root, name)?;
    let head = workspace.head()?;
    let src = workspace.src();
    let dirty = crate::git::is_dirty(&src)?;
    let changed = workspace.changes_from_official()?;
    let touches = touches(changed.iter().map(|c| c.path.as_str()));
    let gates = &workspace.info.gates;
    let short = &head[..head.len().min(8)];
    let mut items = Vec::new();

    items.push(Item {
        code: "clean",
        ok: !dirty,
        blocking: dirty,
        message: if dirty {
            "源码里有还没提交的改动：先提交或撤销，跑出的结果才对得上一个确定的提交".into()
        } else {
            format!("源码干净，当前提交 {short}")
        },
    });

    // 编排、前处理与 Rust 引擎都用这个版本自己的程序，所以不管选哪个引擎都要在当前提交上编过。
    let cli = workspace.bin().join(exe("colm-cli"));
    let engine_built = gates
        .engine
        .as_ref()
        .is_some_and(|g| g.ok && g.commit == head)
        && cli.is_file();
    items.push(Item {
        code: "engine_built",
        ok: engine_built,
        blocking: !engine_built,
        message: if engine_built {
            "Rust 引擎与前处理已在当前提交上编好".into()
        } else {
            "Rust 引擎还没在当前提交上编好：在“改代码”里编译引擎".into()
        },
    });

    let preset = manifest_preset(official_kernel).unwrap_or_else(|| {
        official_kernel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let needs_fortran = engine == "fortran" || preprocessors == "fortran";
    let own_kernel = workspace.kernels().join(&preset);
    let own_kernel_built = gates
        .kernels
        .get(&preset)
        .is_some_and(|g| g.ok && g.commit == head)
        && own_kernel.join("manifest.json").is_file();
    let kernel = if touches.kernel {
        // 改了 Fortran：不管哪个引擎都用自己的内核（Rust 引擎也要它的清单与宏）。
        items.push(Item {
            code: "kernel_built",
            ok: own_kernel_built,
            blocking: !own_kernel_built,
            message: if own_kernel_built {
                format!("改过的 Fortran 内核（{preset}）已在当前提交上编好")
            } else {
                format!("这个版本改了 Fortran：先在“改代码”里编译 {preset} 内核")
            },
        });
        own_kernel
    } else {
        items.push(Item {
            code: "kernel_official",
            ok: true,
            blocking: false,
            message: format!("没改 Fortran，用正式的 {preset} 内核"),
        });
        official_kernel.to_path_buf()
    };

    // 引擎与改动不对口：跑得了，但结果和官方版一样，多半不是本意。
    if touches.engine && !touches.kernel && engine == "fortran" {
        items.push(Item {
            code: "engine_mismatch",
            ok: false,
            blocking: false,
            message: "这个版本只改了 Rust 引擎，用 Fortran 内核跑的模拟段和官方版相同".into(),
        });
    }
    if touches.kernel && !touches.engine && !needs_fortran {
        items.push(Item {
            code: "engine_mismatch",
            ok: false,
            blocking: false,
            message: "这个版本只改了 Fortran 内核，用 Rust 引擎跑的结果和官方版相同".into(),
        });
    }
    if !touches.engine && !touches.kernel {
        items.push(Item {
            code: "no_model_change",
            ok: true,
            blocking: false,
            message: "这个版本没改会影响计算结果的代码，结果应与官方版相同".into(),
        });
    }

    let ready = !items.iter().any(|i| i.blocking);
    Ok(Readiness {
        name: workspace.info.name.clone(),
        head,
        base_commit: workspace.info.base_commit.clone(),
        origin: workspace.info.origin.clone(),
        note: workspace.info.note.clone(),
        based_on: workspace.info.based_on.clone(),
        touches,
        ready,
        items,
        cli,
        kernel,
        preset,
    })
}

#[cfg(test)]
#[path = "version_tests.rs"]
mod version_tests;
