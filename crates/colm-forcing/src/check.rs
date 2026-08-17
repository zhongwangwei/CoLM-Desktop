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

/// CoLM 单点必需的 7 个强迫变量。第 5 槽（u 风）在 PLUMBER2 下是 `NULL`，
/// 标量 `Wind` 进第 6 槽，所以这里只有 7 个而不是 8 个。
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

    for v in REQUIRED_VARS {
        if !m.variables.iter().any(|x| x == v) {
            p.push(format!("required forcing variable {v} is missing"));
        }
    }

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
