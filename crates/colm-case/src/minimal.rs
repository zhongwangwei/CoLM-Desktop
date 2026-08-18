//! 决定一个字段要不要写进生成的 namelist。
//!
//! 判据是**这份输入的正确值是否等于 CoLM 声明的默认值** —— 相同就不写，
//! CoLM 会用同一个值。实测：`oracle/cases/CN-Cng/case.nml` 的 43 个字段里
//! 22 个与默认值相同，把它们删掉重跑，history 与黄金文件
//! `identical: 129 variables`。
//!
//! **「哪些字段冗余」是逐算例算出来的，不是一张固定清单。**
//! `DEF_simulation_time%timestep` 默认 `1800.`，实测 90 个 PLUMBER2 强迫场里
//! 88 个是 1800 秒、2 个是 3600 秒（`US-Ne3` 与 `US-MMS`）。在多数站点上它冗余，
//! 在那两个上必须写 —— 漏了的话模型按 1800 秒推进而强迫场是 3600 秒。

use colm_namelist::Value;
use colm_schema::{find, Default as D};

/// 这个取值是否与 CoLM 声明的默认值相同。
///
/// `None` 表示 `colm-schema` 不认识这个字段名 —— 那种情况必须写出去并让
/// CoLM 自己去拒绝，静默丢弃一个我们不认识的字段是最坏的处置。
pub fn is_default(path: &str, v: &Value) -> Option<bool> {
    let f = find(path)?;
    Some(match (&f.default, v) {
        (D::Logical(a), Value::Bool(b)) => a == b,
        (D::Integer(a), Value::Int(b)) => a == b,
        // Real 必须**按数值比**：1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，
        // 按文本比会把 1800.0 判成偏离，于是每个生成的算例都多带一堆
        // 本可省略的行，diff 里全是噪声。
        (D::Real(a), Value::Real { text }) => match (as_f64(a), as_f64(text)) {
            (Some(x), Some(y)) => x == y,
            _ => a.trim() == text.trim(),
        },
        (D::Str(a), Value::Str(b)) => a == b,
        // 类型对不上就当作「不同」，让它写出去 —— 这是我们理解错了字段类型，
        // 而 CoLM 报一个类型错远好过静默省略。
        _ => false,
    })
}

/// 从一组字段里筛出**必须写**的那些，保持传入顺序。
pub fn required<'a>(fields: &'a [(String, Value)]) -> Vec<&'a (String, Value)> {
    fields
        .iter()
        .filter(|(p, v)| is_default(p, v) != Some(true))
        .collect()
}

/// Fortran 的实数字面量 -> f64。`_r8` 后缀与 `d` 指数都要处理。
fn as_f64(s: &str) -> Option<f64> {
    s.trim()
        .trim_end_matches("_r8")
        .replace(['d', 'D'], "e")
        .parse()
        .ok()
}

#[cfg(test)]
#[path = "minimal_tests.rs"]
mod minimal_tests;
