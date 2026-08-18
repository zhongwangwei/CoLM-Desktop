//! 读强迫场文件的元数据。
//!
//! 只读元数据与时间轴，不读那几十万步的场数据 —— 最大的文件有 333121 个
//! 时间步，而这一层要的只是「起点、步长、步数、三个高度、有哪些变量」。
//!
//! 时间轴要全读一遍，因为「步长是否均匀」只能这样确认，而不均匀的步长会让
//! CoLM 取到错误的时刻却不报错。实测 90 个文件都是均匀的。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::check::MetSummary;
use crate::civil::Stamp;

/// 读一个 PLUMBER2 强迫场文件的元数据。
pub fn summarize(file: &Path) -> Result<MetSummary> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;

    let time = f
        .variable("time")
        .with_context(|| format!("no time variable in {}", file.display()))?;
    let units = time
        .attribute("units")
        .context("time has no units attribute")?
        .value()?;
    // NC_CHAR 与 NC_STRING 是两个变体。实测 PLUMBER2 的文件用的是前者
    // （HDF5 层面 |S33，正好是 "seconds since YYYY-MM-DD HH:MM:SS" 的长度），
    // 但那个语料是被 ncatted 预处理过的，别人的文件未必如此 —— 两种都接。
    let time_units = match units {
        netcdf::AttributeValue::Str(s) => s,
        netcdf::AttributeValue::Strs(v) => v
            .into_iter()
            .next()
            .context("time units attribute is an empty string array")?,
        other => bail!("time units is not a string: {other:?}"),
    };

    let t: Vec<f64> = time.get_values(netcdf::Extents::All)?;
    if t.is_empty() {
        bail!("{} has an empty time axis", file.display());
    }
    let step_seconds = if t.len() > 1 { t[1] - t[0] } else { 0.0 };
    let step_uniform = t
        .windows(2)
        .all(|w| (w[1] - w[0] - step_seconds).abs() < 1e-6);

    let scalar = |n: &str| -> Option<f64> {
        f.variable(n)
            .and_then(|v| v.get_values::<f64, _>(netcdf::Extents::All).ok())
            .and_then(|x: Vec<f64>| x.first().copied())
    };

    let variables: Vec<String> = f.variables().map(|v| v.name()).collect();

    // 全局属性 `time_shown_in`。Urban-PLUMBER 写 "UTC"，PLUMBER2 没有这一项。
    let time_shown_in = f.attribute("time_shown_in").and_then(|a| match a.value() {
        Ok(netcdf::AttributeValue::Str(s)) => Some(s),
        Ok(netcdf::AttributeValue::Strs(v)) => v.into_iter().next(),
        _ => None,
    });

    Ok(MetSummary {
        start: parse_units_start(&time_units)?,
        time_units,
        steps: t.len(),
        step_seconds,
        step_uniform,
        height_v: scalar("reference_height_v").unwrap_or(f64::NAN),
        height_t: scalar("reference_height_t").unwrap_or(f64::NAN),
        height_q: scalar("reference_height_q").unwrap_or(f64::NAN),
        variables,
        time_shown_in,
    })
}

/// 按 CoLM 的方式解析起点：固定字符位置，不做通用解析。
///
/// 刻意与 `MOD_Forcing.F90:1253-1255` 一致 —— 这里要回答的是「CoLM 会读到
/// 什么」，而不是「这个字符串按 CF 约定是什么意思」。两者在畸形输入上会分歧，
/// 而分歧的那一侧正是要报告的。
fn parse_units_start(u: &str) -> Result<Stamp> {
    let b = u.as_bytes();
    if b.len() < 33 {
        bail!("time units {u:?} is too short for CoLM's fixed-position parse");
    }
    let num = |a: usize, z: usize| -> Result<u32> {
        std::str::from_utf8(&b[a - 1..z])
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .with_context(|| format!("time units {u:?} has no number at characters {a}..={z}"))
    };
    Ok(Stamp {
        year: num(15, 18)? as i32,
        month: num(20, 21)?,
        day: num(23, 24)?,
        hour: num(26, 27)?,
        minute: num(29, 30)?,
        second: num(32, 33)?,
    })
}
