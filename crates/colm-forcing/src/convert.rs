//! 把一份强迫场文件转成 CoLM 认的约定。
//!
//! **只转认不出来的数据。** PLUMBER2 继续直读（`lib.rs` 开头那段说明），
//! 转它只会多一份 50 MB 拷贝和一次误差机会，而黄金回归正靠它。
//!
//! **产物与源文件分开存放，原始数据永不改动**（前处理页立的约束）。

use std::path::Path;

use anyhow::{Context, Result};

/// 原样复制一份强迫场文件。
///
/// **这是转换管道的地基，也是它的第一条判据。** 恒等转换必须逐位复现 ——
/// 若这一步就丢精度，后面所有换算的正确性都无从谈起。
///
/// 实现上是「读出来再写进去」而不是 `std::fs::copy`：`fs::copy` 复现的是
/// 字节，证明不了「我们的读写路径不丢精度」，而后者才是要验的东西。
pub fn identity(src: &Path, dst: &Path) -> Result<()> {
    let fin = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let mut fout =
        netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;

    for d in fin.dimensions() {
        fout.add_dimension(&d.name(), d.len())
            .with_context(|| format!("cannot add dimension {}", d.name()))?;
    }

    for v in fin.variables() {
        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let values: Vec<f64> = v
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {}", v.name()))?;
        let mut out = fout
            .add_variable::<f64>(&v.name(), &dim_refs)
            .with_context(|| format!("cannot add variable {}", v.name()))?;
        for a in v.attributes() {
            if let Ok(netcdf::AttributeValue::Str(s)) = a.value() {
                out.put_attribute(a.name(), s.as_str())?;
            }
        }
        out.put_values(&values, netcdf::Extents::All)
            .with_context(|| format!("cannot write {}", v.name()))?;
    }
    Ok(())
}

/// 一个槽位怎么从源文件取。
pub struct SlotPlan {
    /// 1-based，与 `slots::SLOTS` 的 `index` 对齐。
    pub index: usize,
    /// 源文件里的变量名。
    pub source_name: String,
    /// 源文件里的单位（`units` 属性的原文）。
    pub source_units: String,
}

/// 整份转换方案。
pub struct Plan {
    pub slots: Vec<SlotPlan>,
}

/// 按方案把源文件转成 CoLM 认的约定。
///
/// **落地用规范名**（槽位候选名的第一个），不是用户的名字 —— 转换的
/// 目的正是让下游只认一套约定。
///
/// 每个转换过的变量带一条 `source` 属性，说出它从哪个变量、哪个单位来。
/// **换算过的必须标出来**，否则读文件的人会以为那就是源数据里的值。
pub fn convert(src: &Path, dst: &Path, plan: &Plan) -> Result<()> {
    use crate::slots::SLOTS;
    use crate::units::convert_units;

    let fin = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let mut fout =
        netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;

    for d in fin.dimensions() {
        fout.add_dimension(&d.name(), d.len())
            .with_context(|| format!("cannot add dimension {}", d.name()))?;
    }

    // 时间轴原样搬过去 —— 重采样不在这一阶段（见 design-prep.md §6）。
    if let Some(t) = fin.variable("time") {
        let dims: Vec<String> = t.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let vals: Vec<f64> = t
            .get_values(netcdf::Extents::All)
            .with_context(|| "cannot read time".to_string())?;
        let mut out = fout
            .add_variable::<f64>("time", &dim_refs)
            .with_context(|| "cannot add variable time".to_string())?;
        for a in t.attributes() {
            if let Ok(netcdf::AttributeValue::Str(s)) = a.value() {
                out.put_attribute(a.name(), s.as_str())?;
            }
        }
        out.put_values(&vals, netcdf::Extents::All)?;
    }

    for sp in &plan.slots {
        let slot = SLOTS
            .iter()
            .find(|s| s.index == sp.index)
            .with_context(|| format!("no slot {}", sp.index))?;
        let canonical = slot.candidates[0];

        let v = fin
            .variable(&sp.source_name)
            .with_context(|| format!("{} has no variable {}", src.display(), sp.source_name))?;
        let raw: Vec<f64> = v
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {}", sp.source_name))?;
        let want_units = canonical_units(slot.index);
        let vals = convert_units(&sp.source_units, want_units, &raw)?;

        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let mut out = fout
            .add_variable::<f64>(canonical, &dim_refs)
            .with_context(|| format!("cannot add variable {canonical}"))?;
        out.put_attribute("units", want_units)?;
        out.put_attribute(
            "source",
            format!(
                "converted from {:?} ({}) by colm-forcing",
                sp.source_name, sp.source_units
            )
            .as_str(),
        )?;
        out.put_values(&vals, netcdf::Extents::All)?;
    }
    Ok(())
}

/// CoLM 期望每个槽位用什么单位。
fn canonical_units(index: usize) -> &'static str {
    match index {
        1 => "K",        // 气温
        2 => "kg/kg",    // 比湿
        3 => "Pa",       // 气压
        4 => "mm/s",     // 降水率
        5 | 6 => "m/s",  // 风
        7 | 8 => "W/m2", // 辐射
        _ => "",
    }
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod convert_tests;
