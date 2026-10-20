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

/// 生成表的来源：改了它们（或生成器、生成出的表），界面与引擎里那几张表要重新生成，一致性检查（drift）
/// 必须在当前提交上通过——否则界面认识的 namelist 字段、history 变量、地类表和代码对不上。
const GENERATED_SOURCES: &[&str] = &[
    "vendor/CoLM202X/share/MOD_Namelist.F90",
    "vendor/CoLM202X/main/MOD_Hist.F90",
    "vendor/CoLM202X/main/MOD_Const_LC.F90",
    "vendor/CoLM202X/main/MOD_MonthlyinSituCO2MaunaLoa.F90",
    "xtask/",
    "crates/colm-schema/src/generated.rs",
    "crates/colm-hist/src/generated.rs",
    "crates/colm-core/src/land_cover_generated.rs",
    "crates/colm-core/src/co2_generated.rs",
];

/// 冻结前的检查：冻结以后这个版本就固定下来给算例和服务器用，所以要先确定它编得出、表对得上、
/// 继承来的参数改值在新代码里还有效。`parameter_keys` 是草稿里改了值的 namelist 变量（`DEF_…`）。
#[derive(Debug, Clone, Serialize)]
pub struct FreezeCheck {
    pub ready: bool,
    pub touches: Touches,
    pub items: Vec<Item>,
}

pub fn freeze_check(workspace: &Workspace, parameter_keys: &[String]) -> Result<FreezeCheck> {
    let head = workspace.head()?;
    let short = &head[..head.len().min(8)];
    let src = workspace.src();
    let dirty = crate::git::is_dirty(&src)?;
    let changed = workspace.changes_from_official()?;
    let paths: Vec<&str> = changed.iter().map(|c| c.path.as_str()).collect();
    let touches = touches(paths.iter().copied());
    let gates = &workspace.info.gates;
    let at_head =
        |gate: Option<&crate::gates::GateRun>| gate.is_some_and(|g| g.ok && g.commit == head);
    let mut items = Vec::new();
    items.push(Item {
        code: "clean",
        ok: !dirty,
        blocking: dirty,
        message: if dirty {
            "源码里有还没提交的改动：先保存（提交）或撤销".into()
        } else {
            format!("源码干净，当前提交 {short}")
        },
    });
    // 修改版跑的是它自己编出的全部程序（见本模块开头），所以不管改没改 Rust 都要在当前提交上编过。
    let engine = at_head(gates.engine.as_ref()) && workspace.bin().join(exe("colm-cli")).is_file();
    items.push(Item {
        code: "engine_built",
        ok: engine,
        blocking: !engine,
        message: if engine {
            "Rust 引擎与前处理已在当前提交上编好".into()
        } else {
            "先在当前提交上编译 Rust 引擎（检查 → build-engine）".into()
        },
    });
    if touches.kernel {
        let built: Vec<&String> = gates
            .kernels
            .iter()
            .filter(|(preset, gate)| {
                gate.ok
                    && gate.commit == head
                    && workspace
                        .kernels()
                        .join(preset)
                        .join("manifest.json")
                        .is_file()
            })
            .map(|(preset, _)| preset)
            .collect();
        items.push(Item {
            code: "kernel_built",
            ok: !built.is_empty(),
            blocking: built.is_empty(),
            message: if built.is_empty() {
                "改了 Fortran：先在当前提交上编译要用的内核预设（检查 → build-kernel）".into()
            } else {
                format!(
                    "改过的 Fortran 内核已在当前提交上编好：{}",
                    built
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                )
            },
        });
    }
    if paths.iter().any(|path| under(path, GENERATED_SOURCES)) {
        let drift = at_head(gates.tests.get("drift"));
        items.push(Item {
            code: "drift",
            ok: drift,
            blocking: !drift,
            message: if drift {
                "生成表与 Fortran 源码一致（drift 检查已在当前提交上通过）".into()
            } else {
                "改了生成表的来源（namelist、history 变量、地类或 CO₂ 表）：先重新生成并通过 drift 检查（检查 → test-drift）".into()
            },
        });
    }
    let missing = missing_namelist_keys(&src, parameter_keys)?;
    if !parameter_keys.is_empty() {
        items.push(Item {
            code: "parameters",
            ok: missing.is_empty(),
            blocking: !missing.is_empty(),
            message: if missing.is_empty() {
                "继承来的参数改值在新代码里都还有对应的 namelist 变量".into()
            } else {
                format!(
                    "新代码里找不到这些参数改值对应的 namelist 变量：{}；在草稿里删掉或改名后再冻结",
                    missing.join("、")
                )
            },
        });
    }
    if !touches.engine && !touches.kernel {
        items.push(Item {
            code: "no_model_change",
            ok: true,
            blocking: false,
            message: "还没改会影响计算结果的代码：冻结后结果应与父版本相同".into(),
        });
    }
    Ok(FreezeCheck {
        ready: !items.iter().any(|item| item.blocking),
        touches,
        items,
    })
}

/// 草稿里改了值的 `DEF_…` 变量在这份代码的 `MOD_Namelist.F90` 里找不到的那些（`DEF_a%b` 按 `DEF_a` 查）。
fn missing_namelist_keys(src: &Path, keys: &[String]) -> Result<Vec<String>> {
    let file = src.join("vendor/CoLM202X/share/MOD_Namelist.F90");
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text.to_ascii_lowercase(),
        // 没有这个文件（不是 CoLM 源码树）就无从核对，不拦。
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut missing = Vec::new();
    for key in keys {
        let name = key.split('%').next().unwrap_or(key).to_ascii_lowercase();
        if !name.starts_with("def_") {
            continue;
        }
        let found = text.match_indices(&name).any(|(at, _)| {
            let after = text[at + name.len()..].chars().next();
            let before = text[..at].chars().next_back();
            !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                && !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        if !found && !missing.contains(key) {
            missing.push(key.clone());
        }
    }
    Ok(missing)
}

#[cfg(test)]
#[path = "version_tests.rs"]
mod version_tests;
