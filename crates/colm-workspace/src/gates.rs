//! 采纳前的门槛（docs/design-ai-assistant.md 第 6 节）：编译、测试、回归、对齐，各自记下“在哪个提交上测的”，
//! 改了代码就作废。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 一次门槛检查的结果。`commit` 是测它时工作区的提交：之后又有新提交就不再算数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRun {
    pub ok: bool,
    pub at: u64,
    pub commit: String,
    pub detail: String,
}

/// 一次对齐检查（Rust 引擎对 Fortran 内核）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityRecord {
    pub ok: bool,
    pub at: u64,
    pub commit: String,
    pub preset: String,
    pub case: String,
    /// 第一个出现差异的变量与时间步；逐位一致时为空。
    pub first_difference: Option<String>,
    /// 判定用的容差；0 表示逐位。老记录没有这两项，当作 0。
    #[serde(default)]
    pub rtol: f64,
    #[serde(default)]
    pub atol: f64,
    /// 只比较了每个文件的前几条记录；`None` 是全部。
    #[serde(default)]
    pub first_records: Option<usize>,
    #[serde(default)]
    pub ignored: Vec<String>,
}

/// 回归检查的判定类型：重构要求逐位一致，物理修改只列出变了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Refactor,
    Physics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Regression {
    pub kind: ChangeKind,
    pub ok: bool,
    pub at: u64,
    pub commit: String,
    pub case: String,
    pub identical: usize,
    pub changed: usize,
    pub verdict: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gates {
    pub engine: Option<GateRun>,
    /// 预设 → 编译结果。
    #[serde(default)]
    pub kernels: BTreeMap<String, GateRun>,
    /// 测试种类（`cargo:colm-core`、`oracle`、`check-gui`）→ 结果。
    #[serde(default)]
    pub tests: BTreeMap<String, GateRun>,
    pub regression: Option<Regression>,
    pub parity: Option<ParityRecord>,
}

/// 一道门在当前提交上的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    /// 还没测过。
    Unknown,
    /// 在当前提交上通过。
    Pass,
    /// 在当前提交上不通过。
    Fail,
    /// 测过，但之后又有新提交。
    Stale,
    /// 这次的改动用不着这道门（只用于“两版一致”：没改会影响计算结果的代码，见 [`parity_needed`]）。
    NotNeeded,
}

/// 会影响两个引擎计算结果的路径：Fortran 上游与内核的编译脚本、Rust 引擎的计算部分与依赖版本。
/// 只改这些以外的（AI 推理 `colm-hybrid`、界面、助手、工作区、远程、命令行外壳、文档、测试），
/// 改后的 Fortran 与 Rust 仍是原来那两份，“两版一致”就用不着做。
const RESULT_PATHS: &[&str] = &[
    "vendor/CoLM202X/",
    "oracle/scripts/build_kernel.sh",
    "crates/colm-core/",
    "crates/colm-runtime/",
    "crates/colm-init/",
    "crates/colm-srfdata/",
    "crates/colm-forcing/",
    "crates/colm-hist/",
    "crates/colm-lapack/",
    "crates/colm-numeric/",
    "crates/colm-namelist/",
    "crates/colm-schema/",
    "crates/colm-case/",
    "crates/colm-kernel/",
    "crates/colm-h5chunk/",
    "crates/colm-ncchar/",
    "Cargo.lock",
    "Cargo.toml",
    "rust-toolchain.toml",
];

/// 这些改动（相对基线的文件路径）需不需要“两版一致”：碰了会影响计算结果的代码才需要。
pub fn parity_needed<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    paths.into_iter().any(|path| {
        RESULT_PATHS
            .iter()
            .any(|prefix| path == *prefix || (prefix.ends_with('/') && path.starts_with(prefix)))
    })
}

fn light(ok: Option<bool>, measured: Option<&str>, head: &str) -> Light {
    match (ok, measured) {
        (None, _) | (_, None) => Light::Unknown,
        (Some(_), Some(commit)) if commit != head => Light::Stale,
        (Some(true), _) => Light::Pass,
        (Some(false), _) => Light::Fail,
    }
}

/// 界面上的四盏灯。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Lights {
    pub compile: Light,
    pub tests: Light,
    pub regression: Light,
    pub parity: Light,
}

impl Gates {
    /// 在 `head` 这个提交上，各道门的状态，“两版一致”一律当作需要（见 [`Gates::lights_for`]）。
    pub fn lights(&self, head: &str) -> Lights {
        self.lights_for(head, true)
    }

    /// 在 `head` 这个提交上，各道门的状态。编译灯看引擎与所有预设的内核：有一个不通过就是不通过，
    /// 全部通过才是通过。`parity_needed` 为假（这次没改会影响计算结果的代码）时，“两版一致”显示
    /// “不需要”——除非在当前提交上真的做过，那就照实显示。
    pub fn lights_for(&self, head: &str, parity_needed: bool) -> Lights {
        let runs: Vec<&GateRun> = self.engine.iter().chain(self.kernels.values()).collect();
        let compile = if runs.is_empty() {
            Light::Unknown
        } else {
            let each: Vec<Light> = runs
                .iter()
                .map(|r| light(Some(r.ok), Some(&r.commit), head))
                .collect();
            if each.contains(&Light::Fail) {
                Light::Fail
            } else if each.contains(&Light::Stale) {
                Light::Stale
            } else {
                Light::Pass
            }
        };
        let tests = if self.tests.is_empty() {
            Light::Unknown
        } else {
            let each: Vec<Light> = self
                .tests
                .values()
                .map(|r| light(Some(r.ok), Some(&r.commit), head))
                .collect();
            if each.contains(&Light::Fail) {
                Light::Fail
            } else if each.contains(&Light::Stale) {
                Light::Stale
            } else {
                Light::Pass
            }
        };
        Lights {
            compile,
            tests,
            regression: light(
                self.regression.as_ref().map(|r| r.ok),
                self.regression.as_ref().map(|r| r.commit.as_str()),
                head,
            ),
            parity: match light(
                self.parity.as_ref().map(|p| p.ok),
                self.parity.as_ref().map(|p| p.commit.as_str()),
                head,
            ) {
                Light::Unknown | Light::Stale if !parity_needed => Light::NotNeeded,
                other => other,
            },
        }
    }

    /// 能不能登记为实验内核：编译与测试在当前提交上通过，回归没有判不通过。
    /// （回归没做过也能登记，但界面上会显示“未回归”；采纳前的门槛由人在界面上看。）
    pub fn may_register(&self, head: &str) -> bool {
        let lights = self.lights(head);
        lights.compile == Light::Pass
            && lights.tests == Light::Pass
            && !matches!(lights.regression, Light::Fail)
    }
}

#[cfg(test)]
#[path = "gates_tests.rs"]
mod gates_tests;
