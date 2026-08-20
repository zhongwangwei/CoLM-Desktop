//! 单位换算。
//!
//! **认识的才换，不认识的报错。** 放行一个不认识的单位，模型会拿着量纲
//! 错误的数跑完，而界面上什么都看不出来 —— 那正是这个项目反复要避免的
//! 「跑得完却给出错误结果」。
//!
//! 换算表按 `(from, to)` 精确匹配，不做别名归一化 —— `degC` 与 `celsius`
//! 是两条独立的表项。名字的模糊匹配放在调用方（界面上让人确认），
//! 这里只做确定的算术。

use anyhow::{bail, Result};

/// `(from, to, scale, offset)`：`out = in * scale + offset`
const TABLE: &[(&str, &str, f64, f64)] = &[
    // 温度
    ("degC", "K", 1.0, 273.15),
    ("celsius", "K", 1.0, 273.15),
    ("C", "K", 1.0, 273.15),
    // 气压
    ("hPa", "Pa", 100.0, 0.0),
    ("mb", "Pa", 100.0, 0.0),
    ("kPa", "Pa", 1000.0, 0.0),
    // 降水：CoLM 要率（mm/s，等价于 kg/m2/s）
    ("mm/hr", "mm/s", 1.0 / 3600.0, 0.0),
    ("mm/h", "mm/s", 1.0 / 3600.0, 0.0),
    ("mm/day", "mm/s", 1.0 / 86400.0, 0.0),
    // 比湿：无量纲与 g/kg
    ("g/kg", "kg/kg", 0.001, 0.0),
];

/// 把 `values` 从 `from` 换算成 `to`。
///
/// `from == to` 时**原样返回**，不做 `* 1.0 + 0.0` —— 那会让
/// 非规格化的浮点值发生变化，而这条管道的地基正是逐位复现。
pub fn convert_units(from: &str, to: &str, values: &[f64]) -> Result<Vec<f64>> {
    if from == to {
        return Ok(values.to_vec());
    }
    match TABLE.iter().find(|(f, t, _, _)| *f == from && *t == to) {
        Some((_, _, scale, offset)) => Ok(values.iter().map(|v| v * scale + offset).collect()),
        None => bail!(
            "no known conversion from {from:?} to {to:?}; \
             add it to units::TABLE or fix the unit attribute in the source file"
        ),
    }
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod units_tests;
