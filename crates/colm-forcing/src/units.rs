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
    ("degree_Celsius", "K", 1.0, 273.15),
    // 气压
    ("hPa", "Pa", 100.0, 0.0),
    ("mb", "Pa", 100.0, 0.0),
    ("kPa", "Pa", 1000.0, 0.0),
    // 降水：**CoLM 用 `kg/m2/s`**。实测 PLUMBER2 的 90 个站与
    // Urban-PLUMBER 的 21 个站全是它，黄金回归那条直读路径上 CoLM
    // 拿到的也是它。数值上等同 `mm/s`（水的密度 1000 kg/m3，
    // 1 kg/m2 就是 1 mm 水深），但**单位属性要跟直读那条路一致** ——
    // CoLM 不检查单位，标错不报错，只会让人以为转换产物和直读的是两种量。
    ("mm/hr", "kg/m2/s", 1.0 / 3600.0, 0.0),
    ("mm/h", "kg/m2/s", 1.0 / 3600.0, 0.0),
    ("mm/day", "kg/m2/s", 1.0 / 86400.0, 0.0),
    ("mm h-1", "kg/m2/s", 1.0 / 3600.0, 0.0),
    ("mm d-1", "kg/m2/s", 1.0 / 86400.0, 0.0),
    ("mm/s", "kg/m2/s", 1.0, 0.0),
    ("kg m-2 s-1", "kg/m2/s", 1.0, 0.0),
    ("kg m^-2 s^-1", "kg/m2/s", 1.0, 0.0),
    ("kg m**-2 s**-1", "kg/m2/s", 1.0, 0.0),
    // 以 `mm/s` 为目标的几条留着：用户可能明确要一份 mm/s 的产物。
    // 表是精确匹配的，多几条不冲突。
    ("mm/hr", "mm/s", 1.0 / 3600.0, 0.0),
    ("mm/h", "mm/s", 1.0 / 3600.0, 0.0),
    ("mm/day", "mm/s", 1.0 / 86400.0, 0.0),
    // 比湿：无量纲与 g/kg
    ("g/kg", "kg/kg", 0.001, 0.0),
    ("1", "kg/kg", 1.0, 0.0),
    ("kg kg-1", "kg/kg", 1.0, 0.0),
    ("kg kg^-1", "kg/kg", 1.0, 0.0),
    // CF 常见的空格/指数写法。
    ("m s-1", "m/s", 1.0, 0.0),
    ("m s^-1", "m/s", 1.0, 0.0),
    ("m s**-1", "m/s", 1.0, 0.0),
    ("W m-2", "W/m2", 1.0, 0.0),
    ("W m^-2", "W/m2", 1.0, 0.0),
    ("W m**-2", "W/m2", 1.0, 0.0),
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
        // **恒等换算走原样返回。** `kg/m2/s` 与 `mm/s` 是同一个量的两个
        // 名字，scale=1 offset=0。走乘加会把 `-0.0` 变成 `0.0` ——
        // 名字变了，数不能变。
        Some((_, _, s, o)) if *s == 1.0 && *o == 0.0 => Ok(values.to_vec()),
        Some((_, _, scale, offset)) => Ok(values.iter().map(|v| v * scale + offset).collect()),
        None => bail!(
            "no known conversion from {from:?} to {to:?}; \
             add it to units::TABLE or fix the unit attribute in the source file"
        ),
    }
}

/// 把规范槽位单位换回源文件单位。缺测修复发生在格式标准化之前，ERA5-Land
/// donor 用的是规范单位，而中间产物必须仍保持源文件的单位契约。
pub fn from_canonical(canonical: &str, target: &str, values: &[f64]) -> Result<Vec<f64>> {
    if canonical == target {
        return Ok(values.to_vec());
    }
    match TABLE
        .iter()
        .find(|(f, t, _, _)| *f == target && *t == canonical)
    {
        Some((_, _, scale, offset)) if *scale != 0.0 => {
            Ok(values.iter().map(|v| (v - offset) / scale).collect())
        }
        _ => bail!(
            "no known conversion from canonical {canonical:?} back to source unit {target:?}; \
             fix the source unit before gap repair"
        ),
    }
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod units_tests;
