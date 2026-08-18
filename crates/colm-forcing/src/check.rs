//! 开跑之前的契约校验。
//!
//! 校验的重点不是「文件坏了」—— 实测 90 个真实强迫场文件零 NaN、零填充值、
//! 步长在文件内均匀。重点是几种**能跑完却给出错误结果**的配置。
//!
//! 最要紧的一种 CoLM 自己写在注释里（`MOD_Forcing.F90:1107`）：
//! 模拟窗口跑过强迫场末端时「show a Warning but still try to run」，
//! 而 `colm-kernel` 的失败标记里没有 `Warning:` —— 那样的运行会被判成功，
//! 产出一份完整而错误的 history 文件。

use crate::civil::Stamp;

/// CoLM 单点必需的 7 个强迫变量（PLUMBER2 的拼法）。
///
/// **保留只为兼容**：必填性现在由 `slots::resolve` 判断，那里按槽位而不是
/// 按名字，于是 `Rainf`（Urban-PLUMBER 的降水）与 `Psurf`/`PSurf` 两种拼法
/// 都认得。按固定名字列表判断会把另一个数据集判成「缺变量」，而它其实只是
/// 换了个名字。
pub const REQUIRED_VARS: [&str; 7] = [
    "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
];

/// 从强迫场文件读出来的元数据。`met.rs` 负责填它，本模块只做纯计算。
#[derive(Debug, Clone)]
pub struct MetSummary {
    pub time_units: String,
    pub start: Stamp,
    pub steps: usize,
    pub step_seconds: f64,
    pub step_uniform: bool,
    pub height_v: f64,
    pub height_t: f64,
    pub height_q: f64,
    pub variables: Vec<String>,
    /// 全局属性 `time_shown_in` 的原文。
    ///
    /// 决定算例里的 `DEF_simulation_time%greenwich`：Urban-PLUMBER 显式写
    /// `"UTC"`，而 PLUMBER2 **没有这个属性** —— 它的地方时是隐含约定
    /// （见 design.md §2.10）。所以「有且是 UTC」才是格林尼治时，
    /// 其余一律按地方时。搞反会把整个模拟平移一个时区。
    pub time_shown_in: Option<String>,
}

impl MetSummary {
    /// 强迫场覆盖的最后一个时刻。
    pub fn end(&self) -> Stamp {
        let n = self.steps.saturating_sub(1) as i64;
        self.start.plus_seconds(n * self.step_seconds as i64)
    }

    /// 算例 namelist 里 `DEF_simulation_time%timestep` 该取的值。
    pub fn timestep_hint(&self) -> i64 {
        self.step_seconds as i64
    }

    /// 时间轴是不是格林尼治时。
    ///
    /// 只有文件**明说** UTC 才算。没说就是地方时 —— PLUMBER2 的 90 个文件
    /// 全都没有这个属性，而它们确实是地方时。
    pub fn is_greenwich(&self) -> bool {
        self.time_shown_in
            .as_deref()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("UTC"))
    }
}

/// 检查一份强迫场描述，可选地连同模拟窗口一起检查。返回全部问题；空即通过。
pub fn check(m: &MetSummary, window: Option<(Stamp, Stamp)>) -> Vec<String> {
    let mut p = Vec::new();

    // CoLM 按固定字符位置解析这个字符串，所以形状必须一模一样。
    if !units_parseable(&m.time_units) {
        p.push(format!(
            "time units {:?} is not the exact form CoLM parses; \
             it reads fixed character positions and needs \"seconds since YYYY-MM-DD HH:MM:SS\"",
            m.time_units
        ));
    }

    // 按**槽位**判必填，不按名字：同一个量在不同数据集里叫法不同
    // （Precip / Rainf、Psurf / PSurf），按名字列表判会把它们误报成缺失。
    let (_, missing) = crate::slots::resolve(&m.variables);
    p.extend(missing);

    if !m.step_uniform {
        p.push(
            "the time step is not uniform; CoLM samples the axis at a fixed stride and would \
             read the wrong instants without saying so"
                .to_string(),
        );
    }

    if m.steps == 0 {
        p.push("the forcing file has no time steps".to_string());
    }

    if let Some((from, to)) = window {
        let start = m.start;
        let end = m.end();
        if before(&from, &start) {
            p.push(format!(
                "the simulation starts {from:?} which is before the forcing begins at {start:?}"
            ));
        }
        if before(&end, &to) {
            p.push(format!(
                "the simulation ends {to:?} which is beyond the forcing, which stops at {end:?}; \
                 CoLM would print a warning and keep running"
            ));
        }
    }

    p
}

/// CoLM 的解析靠固定字符位置（`MOD_Forcing.F90:1253`），所以这里也按位置检查。
fn units_parseable(u: &str) -> bool {
    let b = u.as_bytes();
    if b.len() < 33 || !u.starts_with("seconds since ") {
        return false;
    }
    let digits = |a: usize, z: usize| b[a - 1..z].iter().all(|c| c.is_ascii_digit());
    digits(15, 18)
        && digits(20, 21)
        && digits(23, 24)
        && digits(26, 27)
        && digits(29, 30)
        && digits(32, 33)
}

fn before(a: &Stamp, b: &Stamp) -> bool {
    key(a) < key(b)
}

fn key(s: &Stamp) -> (i32, u32, u32, u32, u32, u32) {
    (s.year, s.month, s.day, s.hour, s.minute, s.second)
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod check_tests;
