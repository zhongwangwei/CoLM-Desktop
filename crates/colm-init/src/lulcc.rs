//! 土地覆盖变化（LULCC）的同类型赋值方案（SAT，`DEF_LULCC_SCHEME = 1`）。
//!
//! 上游 `REST_LulccTimeVariables`（`MOD_Lulcc_Vars_TimeVariables.F90`）：年末按新一年的
//! 土地覆盖整套冷启动之后，把旧状态里**同一单元、同一 `patchclass`** 的 patch 的一组时间变量
//! 抄到新 patch 上；新出现的类型保持冷启动值。这里在两份时间重启之间做同一件事：
//! 输入是旧年份的续跑重启与新年份的冷启动重启，输出是写回新重启时的替换值。
//!
//! 只做 LCT 的默认路径；PFT/PC 与城市分支在上游另有一套逐 PFT/城市类型的配对，调用方先拒绝。

use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};

use crate::restart_read::{RestartFile, RestartOverride};

/// 决定抄哪些可选变量的开关（对应上游 `IF (DEF_USE_...)` 包着的那几组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatOptions {
    /// `DEF_USE_PLANTHYDRAULICS`：`vegwp`、`gs0sun`、`gs0sha`。
    pub plant_hydraulics: bool,
    /// `DEF_USE_OZONESTRESS`：`lai_old`、`o3uptakesun`、`o3uptakesha`。
    pub ozone_stress: bool,
    /// `DEF_USE_IRRIGATION`：`sum_irrig`、`sum_irrig_count`。
    pub irrigation: bool,
}

/// 一侧（旧年份或新年份）的一个分块：时间重启、每个 patch 的 `patchclass` 与所在单元。
pub struct SatSide<'a> {
    pub time: &'a RestartFile,
    pub patch_class: &'a [i64],
    pub element: &'a [i64],
}

/// 整行照抄的变量，按上游赋值的顺序（盘上名字；`lake_icefrac` 在重启里叫 `lake_icefrc`）。
const ALWAYS: [&str; 24] = [
    "z_sno",
    "dz_sno",
    "t_soisno",
    "wliq_soisno",
    "wice_soisno",
    "scv",
    "smp",
    "hk",
    "t_grnd",
    "tleaf",
    "ldew",
    "ldew_rain",
    "ldew_snow",
    "fwet_snow",
    "sag",
    "snowdp",
    "fsno",
    "zwt",
    "wa",
    "wdsrf",
    "rss",
    "t_lake",
    "lake_icefrc",
    "savedtke1",
];
const PLANT_HYDRAULICS: [&str; 3] = ["vegwp", "gs0sun", "gs0sha"];
const OZONE_STRESS: [&str; 3] = ["lai_old", "o3uptakesun", "o3uptakesha"];
const SNOW_AEROSOL: [&str; 9] = [
    "snw_rds",
    "mss_bcpho",
    "mss_bcphi",
    "mss_ocpho",
    "mss_ocphi",
    "mss_dst1",
    "mss_dst2",
    "mss_dst3",
    "mss_dst4",
];
const SURFACE_DIAGNOSTICS: [&str; 14] = [
    "trad", "tref", "qref", "rst", "emis", "z0m", "zol", "rib", "ustar", "qstar", "tstar", "fm",
    "fh", "fq",
];
const IRRIGATION: [&str; 2] = ["sum_irrig", "sum_irrig_count"];

/// 同一单元里新旧 patch 的配对（新下标, 旧下标），按上游的双指针遍历。
///
/// 每个单元的 patch 区间取该单元 patch 下标的最小与最大值（`grid_patch_s/e`）；
/// 区间内按 `patchclass` 升序对齐：旧的类型小就跳旧的（类型消失），新的小就跳新的（类型新增）。
pub fn match_patches(new: &SatSide<'_>, old: &SatSide<'_>) -> Result<Vec<(usize, usize)>> {
    for side in [new, old] {
        ensure!(
            side.patch_class.len() == side.element.len(),
            "patchclass has {} entries but the element index has {}",
            side.patch_class.len(),
            side.element.len()
        );
    }
    let span = |element: &[i64]| {
        let mut spans = BTreeMap::<i64, (usize, usize)>::new();
        for (patch, &e) in element.iter().enumerate() {
            spans
                .entry(e)
                .and_modify(|(first, last)| {
                    *first = (*first).min(patch);
                    *last = (*last).max(patch);
                })
                .or_insert((patch, patch));
        }
        spans
    };
    let old_spans = span(old.element);
    let mut pairs = Vec::new();
    for (element, (first, last)) in span(new.element) {
        let Some(&(first_, last_)) = old_spans.get(&element) else {
            continue;
        };
        let (mut np, mut np_) = (first, first_);
        while np <= last && np_ <= last_ {
            if new.patch_class[np] > old.patch_class[np_] {
                np_ += 1;
                continue;
            }
            if new.patch_class[np] < old.patch_class[np_] {
                np += 1;
                continue;
            }
            pairs.push((np, np_));
            np += 1;
            np_ += 1;
        }
    }
    Ok(pairs)
}

/// 一个 patch 在盘上占几个值（patch 轴之外各轴的乘积），并核对新旧两侧形状相同。
fn row_length(new: &RestartFile, old: &RestartFile, name: &str) -> Result<usize> {
    let dims = new
        .variable_dimensions(name)
        .with_context(|| format!("the new restart has no {name}"))?;
    let dims_ = old
        .variable_dimensions(name)
        .with_context(|| format!("the old restart has no {name}"))?;
    ensure!(
        dims == dims_,
        "{name} is laid out as {dims:?} in the new restart but {dims_:?} in the old one"
    );
    ensure!(
        dims.first().map(String::as_str) == Some("patch"),
        "{name} is not a patch-major variable ({dims:?})"
    );
    let mut length = 1;
    for dimension in &dims[1..] {
        let (a, b) = (new.dimension(dimension)?, old.dimension(dimension)?);
        ensure!(
            a == b,
            "dimension {dimension} is {a} in the new restart but {b} in the old one"
        );
        length *= a;
    }
    Ok(length)
}

/// 读一个变量的全部值（浮点或整型都按 f64 给出，整型写回时由 `write_with` 还原）。
fn values(file: &RestartFile, name: &str) -> Result<Vec<f64>> {
    if file.floats(name).is_ok() {
        Ok(file.floats(name)?.to_vec())
    } else {
        Ok(file.integers(name)?.iter().map(|&v| v as f64).collect())
    }
}

/// SAT：返回写回新重启时要替换的变量。
pub fn same_type_assignment(
    new: &SatSide<'_>,
    old: &SatSide<'_>,
    options: SatOptions,
) -> Result<Vec<RestartOverride>> {
    let pairs = match_patches(new, old)?;
    let mut names: Vec<&str> = ALWAYS.to_vec();
    if options.plant_hydraulics {
        names.extend(PLANT_HYDRAULICS);
    }
    if options.ozone_stress {
        names.extend(OZONE_STRESS);
    }
    names.extend(SNOW_AEROSOL);
    names.extend(SURFACE_DIAGNOSTICS);
    if options.irrigation {
        names.extend(IRRIGATION);
    }
    let mut overrides = Vec::with_capacity(names.len() + 2);
    for name in names {
        let row = row_length(new.time, old.time, name)?;
        let mut merged = values(new.time, name)?;
        let source = values(old.time, name)?;
        for &(np, np_) in &pairs {
            merged[np * row..(np + 1) * row].copy_from_slice(&source[np_ * row..(np_ + 1) * row]);
        }
        overrides.push(RestartOverride::new(name, merged));
    }
    // `sigf`：照抄之后，若它为 0 而新 patch 的 `lai + sai > 0`（今年才长出叶子），置 1。
    {
        row_length(new.time, old.time, "sigf")?;
        let mut sigf = new.time.floats("sigf")?.to_vec();
        let source = old.time.floats("sigf")?;
        let lai = new.time.floats("lai")?;
        let sai = new.time.floats("sai")?;
        for &(np, np_) in &pairs {
            sigf[np] = source[np_];
            if sigf[np] == 0.0 && lai[np] + sai[np] > 0.0 {
                sigf[np] = 1.0;
            }
        }
        overrides.push(RestartOverride::new("sigf", sigf));
    }
    // `ssno_lyr(2,2,:,np)`：内存里是 `(band, rtyp, snowp1, patch)`，盘上反序
    // `(patch, snowp1, rtyp, band)`；只抄 band = 2、rtyp = 2 那一列。
    {
        let row = row_length(new.time, old.time, "ssno_lyr")?;
        let dims = new.time.variable_dimensions("ssno_lyr")?;
        let (rtyp, band) = (new.time.dimension(&dims[2])?, new.time.dimension(&dims[3])?);
        ensure!(
            rtyp == 2 && band == 2,
            "ssno_lyr has {rtyp} radiation types and {band} bands; SAT copies the (2, 2) column"
        );
        let layers = row / (rtyp * band);
        let mut merged = new.time.floats("ssno_lyr")?.to_vec();
        let source = old.time.floats("ssno_lyr")?;
        for &(np, np_) in &pairs {
            for layer in 0..layers {
                let offset = (layer * rtyp + 1) * band + 1;
                merged[np * row + offset] = source[np_ * row + offset];
            }
        }
        overrides.push(RestartOverride::new("ssno_lyr", merged));
    }
    Ok(overrides)
}

#[cfg(test)]
#[path = "lulcc_tests.rs"]
mod lulcc_tests;
