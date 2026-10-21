//! `DEF_LC_X(lc)`：每个地类一格的地类表覆盖（`MOD_Namelist.F90` 的 `LC_OVERRIDE_SLOTS`，
//! `MOD_Const_LC.F90` 的 `apply_lc_overrides`）。
//!
//! 原来是单点本站地类的一个数（只在 `#ifdef SinglePoint` 里起作用）；2026-10-21 起 vendor 改成逐地类、
//! 单点、流域、区域、全球都生效。初始化（单点与格点）和主循环都从这里读，免得各读各的、各自理解。

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    LandClassOverrideTable, LandClassOverrides, PlantHydraulicOverrideTable,
    PlantHydraulicOverrides,
};
use colm_namelist::{Document, Value};

/// 上游的 `LC_OVERRIDE_UNSET`：显式写成哨兵也算没设。
const UNSET: f64 = -1.0e36;
/// 上游的 `LC_OVERRIDE_SLOTS`：USGS 24 类，IGBP 用前 17 格。
pub const LC_OVERRIDE_SLOTS: usize = 24;

/// 植物水力九列（`apply_lc_overrides` 末尾那九行）。
pub const PLANT_HYDRAULIC_NAMES: [&str; 9] = [
    "DEF_LC_KMAX_SUN",
    "DEF_LC_KMAX_SHA",
    "DEF_LC_KMAX_XYL",
    "DEF_LC_KMAX_ROOT",
    "DEF_LC_PSI50_SUN",
    "DEF_LC_PSI50_SHA",
    "DEF_LC_PSI50_XYL",
    "DEF_LC_PSI50_ROOT",
    "DEF_LC_CK",
];

/// 读出各地类的覆盖。`classes` 是本方案的地类数（IGBP 17、USGS 24）：写到更后面的格，Fortran 会
/// 悄悄不用，这里报错，免得写错的地类号看着像生效了。
pub fn land_class_overrides(
    document: &Document,
    classes: usize,
) -> Result<(LandClassOverrideTable, PlantHydraulicOverrideTable)> {
    let mut table = LandClassOverrideTable::default();
    for name in LandClassOverrides::REAL_NAMES {
        for (class, value) in slots(document, name, classes)? {
            let value = real(name, class, value)?;
            if value != UNSET {
                table.class_mut(class).set_real(name, value)?;
            }
        }
    }
    for (class, value) in slots(document, "DEF_LC_C3C4", classes)? {
        match value {
            Value::Int(-1) => {}
            Value::Int(value @ (0 | 1)) => {
                table.class_mut(class).c3c4 = Some(i32::try_from(*value)?)
            }
            other => bail!("DEF_LC_C3C4({class}) must be the integer -1, 0, or 1, got {other:?}"),
        }
    }
    let mut plant = PlantHydraulicOverrideTable::default();
    for name in PLANT_HYDRAULIC_NAMES {
        for (class, value) in slots(document, name, classes)? {
            let value = real(name, class, value)?;
            if value != UNSET {
                set_plant_hydraulic(plant.class_mut(class), name, value)?;
            }
        }
    }
    Ok((table, plant))
}

/// 有没有写任何地类覆盖（含写成哨兵的格）。
pub fn has_land_class_overrides(document: &Document) -> bool {
    document.paths().iter().any(|path| {
        let name = path.split_once('(').map_or(path.as_str(), |(name, _)| name);
        is_land_class_override(name)
    })
}

fn is_land_class_override(name: &str) -> bool {
    LandClassOverrides::REAL_NAMES
        .iter()
        .chain(&PLANT_HYDRAULIC_NAMES)
        .chain(&["DEF_LC_C3C4"])
        .any(|known| known.eq_ignore_ascii_case(name))
}

/// `name(lc)` 的每一格。不带下标的 `name = v` 在 Fortran 里只落到第 1 格（namelist 给数组赋一个数），
/// 多半是"只改本站地类"的旧写法：拒绝，让写的人说清是哪个地类。
fn slots<'a>(
    document: &'a Document,
    name: &str,
    classes: usize,
) -> Result<Vec<(usize, &'a Value)>> {
    let mut out = Vec::new();
    for path in document.paths() {
        let (head, index) = match path.split_once('(') {
            Some((head, rest)) => (head, Some(rest.trim_end_matches(')'))),
            None => (path.as_str(), None),
        };
        if !head.eq_ignore_ascii_case(name) {
            continue;
        }
        let Some(index) = index else {
            bail!(
                "{name} needs a land-class index: write {name}(lc) = value; a bare {name} would only change land class 1"
            );
        };
        let class: usize = index
            .trim()
            .parse()
            .with_context(|| format!("{path}: the land-class index must be a single integer"))?;
        ensure!(
            (1..=classes).contains(&class),
            "{path}: land class {class} is outside 1..={classes} for this land-cover scheme"
        );
        let value = document.get(&path).context("namelist entry vanished")?;
        out.push((class, value));
    }
    Ok(out)
}

fn real(name: &str, class: usize, value: &Value) -> Result<f64> {
    let value = value
        .as_f64()
        .with_context(|| format!("{name}({class}) must be a real value"))?;
    ensure!(value.is_finite(), "{name}({class}) must be finite");
    Ok(value)
}

fn set_plant_hydraulic(target: &mut PlantHydraulicOverrides, name: &str, value: f64) -> Result<()> {
    let field = match name {
        "DEF_LC_KMAX_SUN" => &mut target.maximum_sunlit_leaf_conductance,
        "DEF_LC_KMAX_SHA" => &mut target.maximum_shaded_leaf_conductance,
        "DEF_LC_KMAX_XYL" => &mut target.maximum_xylem_conductance,
        "DEF_LC_KMAX_ROOT" => &mut target.maximum_root_conductance,
        "DEF_LC_PSI50_SUN" => &mut target.sunlit_leaf_psi50_mm,
        "DEF_LC_PSI50_SHA" => &mut target.shaded_leaf_psi50_mm,
        "DEF_LC_PSI50_XYL" => &mut target.xylem_psi50_mm,
        "DEF_LC_PSI50_ROOT" => &mut target.root_psi50_mm,
        "DEF_LC_CK" => &mut target.vulnerability_shape,
        other => bail!("{other} is not a plant-hydraulic land-class override"),
    };
    *field = Some(value);
    Ok(())
}

#[cfg(test)]
#[path = "land_class_tests.rs"]
mod land_class_tests;
