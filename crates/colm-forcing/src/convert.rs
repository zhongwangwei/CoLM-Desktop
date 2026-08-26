//! 把一份强迫场文件转成 CoLM 认的约定。
//!
//! **只转认不出来的数据。** PLUMBER2 继续直读（`lib.rs` 开头那段说明），
//! 转它只会多一份 50 MB 拷贝和一次误差机会，而黄金回归正靠它。
//!
//! **产物与源文件分开存放，原始数据永不改动**（前处理页立的约束）。

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// 原样复制一份强迫场文件。
///
/// **这是转换管道的地基，也是它的第一条判据。** 恒等转换必须逐位复现 ——
/// 若这一步就丢精度，后面所有换算的正确性都无从谈起。
///
/// 恒等路径不需要重新编码 NetCDF：直接复制才能同时保住数值、类型、压缩、
/// 分块和用户自定义类型，且不会引入任何舍入。
pub fn identity(src: &Path, dst: &Path) -> Result<()> {
    if crate::same_existing_file(src, dst) {
        bail!("forcing identity destination must differ from its source");
    }
    ensure_parent(dst)?;
    std::fs::copy(src, dst)
        .with_context(|| format!("cannot copy {} to {}", src.display(), dst.display()))?;
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
    validate_slot_additions(index, name, &extra)?;
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
    if [v, t, q]
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        bail!("--height {spec:?} values must all be finite and greater than zero");
    }
    Ok(Heights { v, t, q })
}

/// 整份转换方案。
pub struct Plan {
    pub slots: Vec<SlotPlan>,
    /// 源文件没有 `reference_height_*` 时用它兜底；源文件带着的不覆盖
    /// （见 `convert` 末尾那段）。
    pub heights: Option<Heights>,
}

pub fn convert(src: &Path, dst: &Path, plan: &Plan) -> Result<()> {
    if crate::same_existing_file(src, dst) {
        bail!("forcing conversion destination must differ from its source");
    }
    validate_plan_additions(plan)?;
    ensure_parent(dst)?;
    let file_name = dst
        .file_name()
        .and_then(|name| name.to_str())
        .context("forcing conversion destination has no filename")?;
    let temporary = dst.with_file_name(format!(".{file_name}.convert-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    if let Err(error) = convert_into(src, &temporary, plan) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    crate::gapfill::install_repaired_file(&temporary, dst)
}

/// 按方案把源文件转成 CoLM 认的约定。
///
/// **落地用规范名**（槽位候选名的第一个），不是用户的名字 —— 转换的
/// 目的正是让下游只认一套约定。
///
/// 每个转换过的变量带一条 `source` 属性，说出它从哪个变量、哪个单位来。
/// **换算过的必须标出来**，否则读文件的人会以为那就是源数据里的值。
fn convert_into(src: &Path, dst: &Path, plan: &Plan) -> Result<()> {
    use crate::slots::SLOTS;
    use crate::units::convert_units_with_step;

    let fin = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let step_seconds = record_step_seconds(&fin);
    ensure_parent(dst)?;
    let mut fout =
        netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;
    copy_global_attributes(&fin, &mut fout)?;

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
        crate::gapfill::normalize_declared_missing(&fin, &sp.source_name, &mut raw);
        if let Some(actual) = string_attribute(&v, "units")? {
            if actual != sp.source_units {
                bail!(
                    "{} says {} uses unit {:?}, but the conversion plan says {:?}; re-probe the file",
                    src.display(),
                    sp.source_name,
                    actual,
                    sp.source_units
                );
            }
        }

        let want_units = canonical_units(slot.index);
        let mut vals = if slot.index == 2 {
            let temperature = plan_slot_values(&fin, src, plan, 1, step_seconds)?;
            let pressure = plan_slot_values(&fin, src, plan, 3, step_seconds)?;
            match crate::units::humidity_to_specific(
                &sp.source_name,
                &sp.source_units,
                &raw,
                &temperature,
                &pressure,
            )? {
                Some(values) => values,
                None => convert_units_with_step(&sp.source_units, want_units, &raw, step_seconds)?,
            }
        } else {
            convert_units_with_step(&sp.source_units, want_units, &raw, step_seconds)?
        };

        // 多源先各自按自己的单位转到槽位规范单位，再相加。
        for extra in &sp.also_add {
            let e = fin.variable(extra).with_context(|| {
                format!(
                    "{} has no variable {} (named in also_add)",
                    src.display(),
                    extra
                )
            })?;
            let mut raw_add: Vec<f64> = e.get_values(netcdf::Extents::All)?;
            crate::gapfill::normalize_declared_missing(&fin, extra, &mut raw_add);
            if raw_add.len() != vals.len() {
                anyhow::bail!(
                    "{} has {} steps but {} has {} — cannot add them",
                    sp.source_name,
                    vals.len(),
                    extra,
                    raw_add.len()
                );
            }
            let extra_units = string_attribute(&e, "units")?.with_context(|| {
                format!(
                    "{extra} has no units attribute; cannot safely add it to {}",
                    sp.source_name
                )
            })?;
            let add = convert_units_with_step(&extra_units, want_units, &raw_add, step_seconds)?;
            for (a, b) in vals.iter_mut().zip(add.iter()) {
                *a += *b;
            }
        }
        let invalid = vals.iter().filter(|value| !value.is_finite()).count();
        if invalid > 0 {
            bail!(
                "slot {} ({}) produces {invalid} non-finite value(s); run gap diagnosis and repair before converting",
                sp.index,
                sp.source_name
            );
        }

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
                "sum of {:?} ({}) and {:?}, each converted to {}, all kept in this file; \
                 CoLM re-derives phase by wet-bulb temperature (MOD_RainSnowTemp.F90)",
                sp.source_name, sp.source_units, sp.also_add, want_units
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
        copy_variable(&v, &mut fout)?;
    }

    // **手填的高度只在源文件没有时写。** 源文件说了的是量出来的，
    // 界面填的是人估的 —— 让后者覆盖前者是在拿估计换掉测量。
    if let Some(h) = &plan.heights {
        if [h.v, h.t, h.q]
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            bail!("forcing measurement heights must all be finite and greater than zero");
        }
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

fn validate_plan_additions(plan: &Plan) -> Result<()> {
    for slot in &plan.slots {
        validate_slot_additions(slot.index, &slot.source_name, &slot.also_add)?;
    }
    Ok(())
}

fn validate_slot_additions(index: usize, source_name: &str, also_add: &[String]) -> Result<()> {
    if source_name.trim().is_empty() {
        bail!("slot {index} has an empty source variable name");
    }
    let mut seen = BTreeSet::new();
    for extra in also_add {
        let extra = extra.trim();
        if extra.is_empty() {
            bail!("slot {index} has an empty also_add variable name");
        }
        if extra == source_name.trim() {
            bail!(
                "slot {index} names {source_name:?} both as its source and in also_add — \
                 it would be added twice, silently doubling the values"
            );
        }
        if !seen.insert(extra) {
            bail!("slot {index} names also_add variable {extra:?} more than once");
        }
    }
    Ok(())
}

/// 产物的父目录不存在就建出来。
///
/// `netcdf::create` 只建文件，父目录得自己管 —— 而界面给的默认产物目录
/// （`~/CoLM-forcing`）第一次用必然不存在，于是「不用打字直接点转换」
/// 那条路径撞的是 `No such file or directory`。**真机验收才发现的**：
/// 单测一直往 `std::env::temp_dir()` 写，那个目录永远存在。
///
/// 与 `colm-cli new` 对 `--out` 的处置一致：用户给了一个目标路径，
/// 意图显然是「写到那儿」，不是「先替我确认它存在」。
///
/// **抽成函数是因为 `identity` 与 `convert` 的开头长得一样**
/// （都是 `netcdf::open` + `netcdf::create`）。修这个 bug 时我的
/// 查找替换先命中了 `identity` 那一份，对着没改到的地方调试了几轮 ——
/// 和上一次修 `_FillValue` 时踩的是同一个坑。
pub(crate) fn ensure_parent(dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
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
pub(crate) fn copy_attributes(from: &netcdf::Variable, to: &mut netcdf::VariableMut) -> Result<()> {
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

fn copy_non_fill_attributes(from: &netcdf::Variable, to: &mut netcdf::VariableMut) -> Result<()> {
    for attribute in from.attributes() {
        if attribute.name() != "_FillValue" {
            to.put_attribute(attribute.name(), attribute.value()?)?;
        }
    }
    Ok(())
}

fn copy_typed<T>(
    from: &netcdf::Variable,
    to: &mut netcdf::FileMut,
    dimensions: &[&str],
) -> Result<()>
where
    T: netcdf::types::NcTypeDescriptor + Copy,
{
    let values: Vec<T> = from.get_values(netcdf::Extents::All)?;
    let mut output = to.add_variable::<T>(&from.name(), dimensions)?;
    if let Some(fill) = from.fill_value::<T>()? {
        output.set_fill_value(fill)?;
    }
    copy_non_fill_attributes(from, &mut output)?;
    output.put_values(&values, netcdf::Extents::All)?;
    Ok(())
}

/// Ancillary variables are part of the source contract. Preserve every common
/// primitive type exactly; refuse uncommon NetCDF user types rather than
/// silently coercing them to `f64`.
fn copy_variable(from: &netcdf::Variable, to: &mut netcdf::FileMut) -> Result<()> {
    use netcdf::types::{FloatType, IntType, NcVariableType};

    let names: Vec<String> = from
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect();
    let dimensions: Vec<&str> = names.iter().map(String::as_str).collect();
    match from.vartype() {
        NcVariableType::Int(IntType::U8) => copy_typed::<u8>(from, to, &dimensions),
        NcVariableType::Int(IntType::U16) => copy_typed::<u16>(from, to, &dimensions),
        NcVariableType::Int(IntType::U32) => copy_typed::<u32>(from, to, &dimensions),
        NcVariableType::Int(IntType::U64) => copy_typed::<u64>(from, to, &dimensions),
        NcVariableType::Int(IntType::I8) => copy_typed::<i8>(from, to, &dimensions),
        NcVariableType::Int(IntType::I16) => copy_typed::<i16>(from, to, &dimensions),
        NcVariableType::Int(IntType::I32) => copy_typed::<i32>(from, to, &dimensions),
        NcVariableType::Int(IntType::I64) => copy_typed::<i64>(from, to, &dimensions),
        NcVariableType::Float(FloatType::F32) => copy_typed::<f32>(from, to, &dimensions),
        NcVariableType::Float(FloatType::F64) => copy_typed::<f64>(from, to, &dimensions),
        kind => bail!(
            "cannot preserve ancillary variable {} with NetCDF type {kind:?}; convert or remove that variable explicitly",
            from.name()
        ),
    }
}

fn string_attribute(variable: &netcdf::Variable, name: &str) -> Result<Option<String>> {
    let Some(attribute) = variable.attribute(name) else {
        return Ok(None);
    };
    match attribute.value()? {
        netcdf::AttributeValue::Str(value) => Ok(Some(value)),
        netcdf::AttributeValue::Strs(values) => Ok(values.into_iter().next()),
        other => bail!(
            "{}.{} must be a string, got {other:?}",
            variable.name(),
            name
        ),
    }
}

fn record_step_seconds(file: &netcdf::File) -> Option<f64> {
    let time = file.variable("time")?;
    let values: Vec<f64> = time.get_values(netcdf::Extents::All).ok()?;
    let first = *values.get(1)? - values[0];
    let unit = string_attribute(&time, "units").ok()??;
    let scale = match unit
        .split_whitespace()
        .next()?
        .to_ascii_lowercase()
        .as_str()
    {
        "second" | "seconds" => 1.0,
        "minute" | "minutes" => 60.0,
        "hour" | "hours" => 3600.0,
        "day" | "days" => 86_400.0,
        _ => return None,
    };
    (first.is_finite()
        && first > 0.0
        && values
            .windows(2)
            .all(|window| (window[1] - window[0] - first).abs() < 1e-6))
    .then_some(first * scale)
}

fn plan_slot_values(
    file: &netcdf::File,
    path: &Path,
    plan: &Plan,
    index: usize,
    step_seconds: Option<f64>,
) -> Result<Vec<f64>> {
    let slot = plan
        .slots
        .iter()
        .find(|slot| slot.index == index)
        .with_context(|| format!("humidity derivation also needs forcing slot {index}"))?;
    let variable = file
        .variable(&slot.source_name)
        .with_context(|| format!("{} has no variable {}", path.display(), slot.source_name))?;
    if let Some(actual) = string_attribute(&variable, "units")? {
        if actual != slot.source_units {
            bail!(
                "{} says {} uses unit {:?}, but the conversion plan says {:?}; re-probe the file",
                path.display(),
                slot.source_name,
                actual,
                slot.source_units
            );
        }
    }
    let mut raw: Vec<f64> = variable.get_values(netcdf::Extents::All)?;
    crate::gapfill::normalize_declared_missing(file, &slot.source_name, &mut raw);
    crate::units::convert_units_with_step(
        &slot.source_units,
        canonical_units(index),
        &raw,
        step_seconds,
    )
}

fn copy_global_attributes(from: &netcdf::File, to: &mut netcdf::FileMut) -> Result<()> {
    for attribute in from.attributes() {
        if let Ok(value) = attribute.value() {
            to.add_attribute(attribute.name(), value)?;
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
