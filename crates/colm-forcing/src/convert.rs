//! 把一份强迫场文件转成 CoLM 认的约定。
//!
//! **只转认不出来的数据。** PLUMBER2 继续直读（`lib.rs` 开头那段说明），
//! 转它只会多一份 50 MB 拷贝和一次误差机会，而黄金回归正靠它。
//!
//! **产物与源文件分开存放，原始数据永不改动**（前处理页立的约束）。

use std::path::Path;

use anyhow::{bail, Context, Result};

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
#[derive(Debug, Clone)]
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

/// 解析一条 `--slot N=name:units[+extra[+extra...]]`。
///
/// 独立 bin `forcing-convert` 与 `colm-cli forcing-convert` 都要这份
/// 解析——抽出来共用，别抄两份。抄两份意味着两处要同步改：
/// `copy_attributes`（本文件上面）就是同一段代码抄三遍、错也有三份的
/// 前车之鉴。
///
/// **报错要给出正确的形状**（`N=name:units`），不能只说「格式错误」——
/// 用户下一步要用的正是那个形状。
pub fn parse_slot_spec(spec: &str) -> Result<SlotPlan> {
    let (idx, rest) = spec
        .split_once('=')
        .with_context(|| format!("--slot {spec:?} is not N=name:units"))?;
    let (name, units) = rest
        .split_once(':')
        .with_context(|| format!("--slot {spec:?} is not N=name:units (missing :units)"))?;
    // `--slot 4=Rainf:kg/m2/s+Snowf` —— 加号后面是要合并进同一个槽位的
    // 变量（合并降水相态，见 `SlotPlan::also_add` 上的说明）。
    let (units, extra) = match units.split_once('+') {
        Some((u, e)) => (u, e.split('+').map(str::to_string).collect()),
        None => (units, Vec::new()),
    };
    let index: usize = idx.parse().with_context(|| {
        format!("--slot {spec:?} is not N=name:units ({idx:?} is not a slot number)")
    })?;
    Ok(SlotPlan {
        index,
        source_name: name.to_string(),
        source_units: units.to_string(),
        also_add: extra,
    })
}

/// 观测高度。源文件没有 `reference_height_*` 时由用户在界面上填。
///
/// **三个分开而不是一个值**：CoLM 的 `DEF_forcing%HEIGHT_V/T/Q` 本来
/// 就是三个，风的观测高度与温湿的常常不同（塔上不同层）。
#[derive(Debug, Clone, Copy)]
pub struct Heights {
    pub v: f64,
    pub t: f64,
    pub q: f64,
}

/// 解析一条 `--height V,T,Q`。
pub fn parse_heights(spec: &str) -> Result<Heights> {
    let n: Vec<f64> = spec
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("--height {spec:?} is not V,T,Q"))?;
    let [v, t, q] = n[..] else {
        bail!(
            "--height {spec:?} needs exactly three numbers, got {}",
            n.len()
        );
    };
    Ok(Heights { v, t, q })
}

/// 整份转换方案。
pub struct Plan {
    pub slots: Vec<SlotPlan>,
    /// 源文件没有 `reference_height_*` 时用它兜底；源文件带着的不覆盖
    /// （见 `convert` 末尾那段）。
    pub heights: Option<Heights>,
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

    // **槽位没消费的变量原样搬过去。** 转换可以增加信息，不能减少信息。
    //
    // 两件事都靠这一条：
    //
    // 1. **CoLM 读的不止那八个槽位。** `reference_height_v/t/q` 是标量，
    //    `met::summarize` 要读它们填 forcing.nml 的 `DEF_forcing%HEIGHT_*`。
    //    丢了就回落成 NaN 写进 namelist，而 CoLMDEBUG 内核的 RangeCheck
    //    会直接 SIGILL —— 报出来的是「内核编进了 CoLMDEBUG」，看不出问题
    //    在强迫场少了三个标量。
    //
    // 2. **合成过的槽位要保留相态。** `Rainf`/`Snowf` 合并进第 4 槽之后
    //    原样留在产物里：观测给的相态是实测事实，而 CoLM 判出来的是
    //    参数化推断，丢掉原变量等于用后者永久换掉前者。
    //
    // **单源槽位的源变量不搬** —— 它已经以规范名落地了（`TA_F` → `Tair`），
    // 再留一份原名只会让人分不清哪个是准的。合成的那些不在此列：它们的
    // 源变量带的是槽位表达不了的信息。
    let consumed: Vec<&str> = plan
        .slots
        .iter()
        .filter(|sp| sp.also_add.is_empty())
        .map(|sp| sp.source_name.as_str())
        .collect();
    let names: Vec<String> = fin.variables().map(|v| v.name()).collect();
    for name in names {
        if name == "time" || fout.variable(&name).is_some() || consumed.contains(&name.as_str()) {
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

    // **手填的高度只在源文件没有时写。** 源文件说了的是量出来的，
    // 界面填的是人估的 —— 让后者覆盖前者是在拿估计换掉测量。
    if let Some(h) = &plan.heights {
        for (name, val) in [
            ("reference_height_v", h.v),
            ("reference_height_t", h.t),
            ("reference_height_q", h.q),
        ] {
            if fout.variable(name).is_some() {
                continue; // 源文件带着，已经搬过去了
            }
            let mut out = fout.add_variable::<f64>(name, &[])?;
            out.put_attribute("units", "m")?;
            out.put_attribute("source", "given by hand in the prep page")?;
            out.put_values(&[val], netcdf::Extents::All)?;
        }
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
///
/// `pub`：`colm-cli forcing-probe` 的 `wants` 字段要报出同一份期望，
/// 抄一份常量表意味着两处要同步改。
pub fn canonical_units(index: usize) -> &'static str {
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
