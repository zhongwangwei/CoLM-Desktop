//! 读 PLUMBER2 的 `Observation/*_Flux.nc` 与模型的 `*_hist_*.nc`。
//!
//! 实测的观测文件形状（CN-Cng，2008-2009）：
//! `time = 35088` 半小时步长、`x = y = 1`；通量 `Rnet` / `Qle` / `Qh` / `Qg` /
//! `SWup` 各带一个 `<name>_qc`；`_FillValue = -9999`。
//! `GPP` / `Resp` 只有 `_se` 没有 `_qc`，本模块不处理它们。
//!
//! **不用 `_cor` 能量闭合订正版本**：design.md §2.8 / §2.8b 的目标值是用
//! 未订正版算的，用订正版复现不出来。

use anyhow::{Context, Result};
use std::path::Path;

/// 观测与模型的变量对应。全部 W/m2。
pub const FLUX_PAIRS: [(&str, &str); 5] = [
    ("Rnet", "f_rnet"),  // §2.8 指定的关键验证信号
    ("Qh", "f_fsena"),   // 感热
    ("Qle", "f_lfevpa"), // 潜热
    ("Qg", "f_fgrnd"),   // 地表热通量
    ("SWup", "f_sr"),    // 反射短波
];

pub fn read_1d(path: &Path, name: &str) -> Result<Vec<f64>> {
    let f = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let v = f
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    Ok(v.get_values::<f64, _>(..)?)
}

/// 读 `time` 变量的 `units` 属性。
///
/// 观测文件的时间原点写在这里（实测 `"seconds since 2008-01-01 00:00:00"`），
/// 而模型 history 的原点固定是 1900 —— 两边换算到同一原点才谈得上配对。
pub fn time_units(path: &Path) -> Result<String> {
    let f = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let v = f
        .variable("time")
        .with_context(|| format!("{} has no time variable", path.display()))?;
    match v.attribute("units").and_then(|a| a.value().ok()) {
        Some(netcdf::AttributeValue::Str(s)) => Ok(s),
        other => anyhow::bail!(
            "time:units in {} is {other:?}, not a string",
            path.display()
        ),
    }
}
