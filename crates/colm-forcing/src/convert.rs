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
        copy_attributes(&v, &mut out)?;
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
    /// 还要**加到这个槽位上**的变量（同单位）。
    ///
    /// 为降水而设：Urban-PLUMBER 把降水分成 `Rainf` 与 `Snowf`，
    /// 而槽位机制一个槽位只能指向一个变量名。不合并就丢掉全部降雪 ——
    /// 实测 FI-Kumpula 少 24.7%，而模型照样跑得完。
    ///
    /// **合并之后源变量仍然原样保留在产物里**（见 `convert` 末尾那段）。
    pub also_add: Vec<String>,
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
        copy_attributes(&t, &mut out)?;
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
        let mut raw: Vec<f64> = v
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {}", sp.source_name))?;

        // 多源合成：同单位相加。
        for extra in &sp.also_add {
            let e = fin.variable(extra).with_context(|| {
                format!(
                    "{} has no variable {} (named in also_add)",
                    src.display(),
                    extra
                )
            })?;
            let add: Vec<f64> = e.get_values(netcdf::Extents::All)?;
            if add.len() != raw.len() {
                anyhow::bail!(
                    "{} has {} steps but {} has {} — cannot add them",
                    sp.source_name,
                    raw.len(),
                    extra,
                    add.len()
                );
            }
            for (a, b) in raw.iter_mut().zip(add.iter()) {
                *a += *b;
            }
        }

        let want_units = canonical_units(slot.index);
        let vals = convert_units(&sp.source_units, want_units, &raw)?;

        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let mut out = fout
            .add_variable::<f64>(canonical, &dim_refs)
            .with_context(|| format!("cannot add variable {canonical}"))?;
        out.put_attribute("units", want_units)?;
        let note = if sp.also_add.is_empty() {
            format!(
                "converted from {:?} ({}) by colm-forcing",
                sp.source_name, sp.source_units
            )
        } else {
            format!(
                "sum of {:?} and {:?} ({}), all kept in this file; \
                 CoLM re-derives phase by wet-bulb temperature (MOD_RainSnowTemp.F90)",
                sp.source_name, sp.also_add, sp.source_units
            )
        };
        out.put_attribute("source", note.as_str())?;
        out.put_values(&vals, netcdf::Extents::All)?;
    }

    // **源变量原样保留。** 转换可以增加信息，不能减少信息 ——
    // 观测给的相态是实测事实，而 CoLM 判出来的是参数化推断，
    // 合成之后把原变量丢掉等于用后者永久换掉前者。
    let mut kept: Vec<String> = Vec::new();
    for sp in &plan.slots {
        if sp.also_add.is_empty() {
            continue;
        }
        kept.push(sp.source_name.clone());
        kept.extend(sp.also_add.iter().cloned());
    }
    for name in kept {
        if fout.variable(&name).is_some() {
            continue;
        }
        let Some(v) = fin.variable(&name) else {
            continue;
        };
        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let vals: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        let mut out = fout.add_variable::<f64>(&name, &dim_refs)?;
        copy_attributes(&v, &mut out)?;
        out.put_values(&vals, netcdf::Extents::All)?;
    }
    Ok(())
}

/// 把一个变量的属性原样搬到另一个变量上。
///
/// **`_FillValue` 要单独处理。** 它是 netCDF 的保留属性，`put_attribute`
/// 写不进去（悄悄没了，不报错），专用的是 `set_fill_value`，而且必须在
/// 写数据**之前**调用 —— 写过数据之后是 late define，会失败。
///
/// 抽成一个函数不是为了整齐：这段逻辑原本在三个地方各抄了一遍
/// （`identity`、时间轴、保留源变量），三处都只搬了 `Str` 类型的属性，
/// 于是三处都把 `_FillValue` 丢在了源文件里。**同一段代码抄三遍，
/// 错也会有三份。**
///
/// 源文件常是 `float32` 而产物统一 `f64`，所以两种类型都试一遍。
fn copy_attributes(from: &netcdf::Variable, to: &mut netcdf::VariableMut) -> Result<()> {
    let fill = from
        .fill_value::<f64>()
        .ok()
        .flatten()
        .or_else(|| from.fill_value::<f32>().ok().flatten().map(f64::from));
    if let Some(fill) = fill {
        to.set_fill_value(fill)?;
    }
    for a in from.attributes() {
        if a.name() == "_FillValue" {
            continue; // 上面用专用 API 设过了
        }
        if let Ok(val) = a.value() {
            to.put_attribute(a.name(), val)?;
        }
    }
    Ok(())
}

/// CoLM 期望每个槽位用什么单位。
fn canonical_units(index: usize) -> &'static str {
    match index {
        1 => "K",     // 气温
        2 => "kg/kg", // 比湿
        3 => "Pa",    // 气压
        // **降水用 `kg/m2/s`，不是 `mm/s`。** PLUMBER2 与 Urban-PLUMBER
        // 都是它，黄金回归那条直读路径上 CoLM 拿到的也是它。两者数值恒等，
        // 所以标错不会报错 —— 只会让转换产物和直读的看起来是两种量。
        4 => "kg/m2/s",  // 降水率
        5 | 6 => "m/s",  // 风
        7 | 8 => "W/m2", // 辐射
        _ => "",
    }
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod convert_tests;
