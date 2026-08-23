//! 单位换算。
//!
//! **认识的才换，不认识的报错。** 放行一个不认识的单位，模型会拿着量纲
//! 错误的数跑完，而界面上什么都看不出来 —— 那正是这个项目反复要避免的
//! 「跑得完却给出错误结果」。
//!
//! 换算表按 `(from, to)` 精确匹配，不做别名归一化 —— `degC` 与 `celsius`
//! 是两条独立的表项。名字的模糊匹配放在调用方（界面上让人确认），
//! 这里只做确定的算术。

use anyhow::{bail, Context, Result};

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
    convert_units_with_step(from, to, values, None)
}

/// Convert units that may represent an amount accumulated over one source
/// record. Bare `mm`/`kg m-2` are common tower exports; their rate is defined
/// only together with the actual record cadence.
pub fn convert_units_with_step(
    from: &str,
    to: &str,
    values: &[f64],
    step_seconds: Option<f64>,
) -> Result<Vec<f64>> {
    if from == to {
        return Ok(values.to_vec());
    }
    if to == "kg/m2/s"
        && matches!(
            from.trim(),
            "mm" | "kg/m2" | "kg m-2" | "kg m^-2" | "kg m**-2"
        )
    {
        let step = step_seconds
            .filter(|step| step.is_finite() && *step > 0.0)
            .context(
                "an interval-accumulated precipitation unit needs a positive source time step",
            )?;
        return Ok(values.iter().map(|value| value / step).collect());
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

/// Convert relative humidity or vapour-pressure deficit to CoLM specific
/// humidity. Temperature and pressure must already be in K and Pa.
pub fn humidity_to_specific(
    source_name: &str,
    source_units: &str,
    values: &[f64],
    temperature: &[f64],
    pressure: &[f64],
) -> Result<Option<Vec<f64>>> {
    if values.len() != temperature.len() || values.len() != pressure.len() {
        bail!("humidity, temperature, and pressure series must have the same length");
    }
    let (relative, deficit) = humidity_source_kind(source_name);
    if !relative && !deficit {
        return Ok(None);
    }
    let humidity = if relative {
        match source_units.trim() {
            "%" | "percent" | "percentage" => values.iter().map(|value| value / 100.0).collect(),
            "1" | "fraction" => values.to_vec(),
            other => bail!("relative humidity unit {other:?} is not % or a 0..1 fraction"),
        }
    } else {
        convert_units(source_units, "Pa", values)?
    };
    Ok(Some(
        humidity
            .into_iter()
            .zip(temperature)
            .zip(pressure)
            .map(|((humidity, temperature), pressure)| {
                if !humidity.is_finite() || !temperature.is_finite() || !pressure.is_finite() {
                    return f64::NAN;
                }
                let saturation =
                    611.2 * (17.67 * (temperature - 273.15) / (temperature - 29.65)).exp();
                let vapour = if relative {
                    if !(0.0..=1.0).contains(&humidity) {
                        return f64::NAN;
                    }
                    humidity * saturation
                } else {
                    if humidity < 0.0 || humidity > saturation {
                        return f64::NAN;
                    }
                    saturation - humidity
                };
                let denominator = pressure - (1.0 - 0.622) * vapour;
                if denominator > 0.0 {
                    0.622 * vapour / denominator
                } else {
                    f64::NAN
                }
            })
            .collect(),
    ))
}

pub fn humidity_from_specific(
    source_name: &str,
    source_units: &str,
    values: &[f64],
    temperature: &[f64],
    pressure: &[f64],
) -> Result<Option<Vec<f64>>> {
    if values.len() != temperature.len() || values.len() != pressure.len() {
        bail!("humidity, temperature, and pressure series must have the same length");
    }
    let (relative, deficit) = humidity_source_kind(source_name);
    if !relative && !deficit {
        return Ok(None);
    }
    let pascals = values
        .iter()
        .zip(temperature)
        .zip(pressure)
        .map(|((humidity, temperature), pressure)| {
            if !humidity.is_finite() || !temperature.is_finite() || !pressure.is_finite() {
                return f64::NAN;
            }
            let vapour = humidity * pressure / (0.622 + (1.0 - 0.622) * humidity);
            let saturation = 611.2 * (17.67 * (temperature - 273.15) / (temperature - 29.65)).exp();
            if relative {
                vapour / saturation
            } else {
                saturation - vapour
            }
        })
        .collect::<Vec<_>>();
    if relative {
        match source_units.trim() {
            "%" | "percent" | "percentage" => Ok(Some(
                pascals.into_iter().map(|value| value * 100.0).collect(),
            )),
            "1" | "fraction" => Ok(Some(pascals)),
            other => bail!("relative humidity unit {other:?} is not % or a 0..1 fraction"),
        }
    } else {
        Ok(Some(from_canonical("Pa", source_units, &pascals)?))
    }
}

fn humidity_source_kind(source_name: &str) -> (bool, bool) {
    let name = source_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let compact = name.replace('_', "");
    let relative = compact.contains("relativehumidity")
        || name == "rh"
        || name.starts_with("rh_")
        || name.ends_with("_rh");
    let deficit = name.contains("vpd")
        || compact.contains("vaporpressuredeficit")
        || compact.contains("vapourpressuredeficit");
    (relative, deficit)
}

/// 把规范槽位单位换回源文件单位。缺测修复发生在格式标准化之前，ERA5-Land
/// donor 用的是规范单位，而中间产物必须仍保持源文件的单位契约。
pub fn from_canonical(canonical: &str, target: &str, values: &[f64]) -> Result<Vec<f64>> {
    from_canonical_with_step(canonical, target, values, None)
}

pub fn from_canonical_with_step(
    canonical: &str,
    target: &str,
    values: &[f64],
    step_seconds: Option<f64>,
) -> Result<Vec<f64>> {
    if canonical == target {
        return Ok(values.to_vec());
    }
    if canonical == "kg/m2/s"
        && matches!(
            target.trim(),
            "mm" | "kg/m2" | "kg m-2" | "kg m^-2" | "kg m**-2"
        )
    {
        let step = step_seconds
            .filter(|step| step.is_finite() && *step > 0.0)
            .context(
                "an interval-accumulated precipitation unit needs a positive source time step",
            )?;
        return Ok(values.iter().map(|value| value * step).collect());
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
