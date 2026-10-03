//! 土地覆盖变化（LULCC）的同类型赋值方案（SAT，`DEF_LULCC_SCHEME = 1`）。
//!
//! 上游 `REST_LulccTimeVariables`（`MOD_Lulcc_Vars_TimeVariables.F90`）：年末按新一年的
//! 土地覆盖整套冷启动之后，把旧状态里**同一单元、同一 `patchclass`** 的 patch 的一组时间变量
//! 抄到新 patch 上；新出现的类型保持冷启动值。这里在两份时间重启之间做同一件事：
//! 输入是旧年份的续跑重启与新年份的冷启动重启，输出是写回新重启时的替换值。
//!
//! PFT/PC（[`pft_same_type_assignment`]）：两侧都是土壤 patch 时再按 `pftclass` 逐 PFT 配对。
//! 城市分支在上游另有一套逐城市类型的配对，调用方先拒绝。

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context, Result};

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
    /// `DEF_URBAN_RUN`：每个 patch 的城市类型（`landurban%settyp`，非城市 patch 为 0）。
    /// 有它时城市 patch 还要按城市类型对齐（`MOD_Lulcc_Vars_TimeVariables.F90:706-724`）。
    pub urban_class: Option<&'a [i64]>,
}

/// `maxsnl = -5`：雪层数。
const SNOW_LAYERS: usize = 5;

/// IGBP 城市类（`URBAN`）。
pub const URBAN: i64 = 13;

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
            // 城市 patch 还要同一城市类型：旧的类型小就跳旧的（消失），新的小就跳新的（新增）。
            if let (Some(new_urban), Some(old_urban)) = (new.urban_class, old.urban_class) {
                if new.patch_class[np] == URBAN {
                    if new_urban[np] > old_urban[np_] {
                        np_ += 1;
                        continue;
                    }
                    if new_urban[np] < old_urban[np_] {
                        np += 1;
                        continue;
                    }
                }
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

/// PFT 一侧：PFT 时间重启、每个 PFT 的 `pftclass`、每个 patch 的 PFT 区间（`patch_pft_s/e`，
/// 非土壤 patch 为空区间）与 `patchtype`。
pub struct PftSatSide<'a> {
    pub time: &'a RestartFile,
    pub pft_class: &'a [i64],
    pub ranges: &'a [std::ops::Range<usize>],
    pub patch_type: &'a [i64],
}

/// [`pft_same_type_assignment`] 的结果：PFT 时间重启的替换项，与 `(patch, 新 ldew)`。
pub type PftSatResult = (Vec<RestartOverride>, Vec<(usize, f64)>);

/// 逐 PFT 照抄的变量（`REST_LulccTimeVariables` 的 PFT 段，`MOD_Lulcc_Vars_TimeVariables.F90:803-861`）。
const PFT_ALWAYS: [&str; 10] = [
    "tleaf_p",
    "ldew_p",
    "ldew_rain_p",
    "ldew_snow_p",
    "fwet_snow_p",
    "sigf_p",
    "tref_p",
    "qref_p",
    "rst_p",
    "z0m_p",
];
const PFT_PLANT_HYDRAULICS: [&str; 3] = ["vegwp_p", "gs0sun_p", "gs0sha_p"];
const PFT_OZONE_STRESS: [&str; 3] = ["lai_old_p", "o3uptakesun_p", "o3uptakesha_p"];

/// 一个 PFT 在盘上占几个值（PFT 轴之外各轴的乘积）。
fn pft_row_length(new: &RestartFile, old: &RestartFile, name: &str) -> Result<usize> {
    let dims = new
        .variable_dimensions(name)
        .with_context(|| format!("the new PFT restart has no {name}"))?;
    ensure!(
        dims == old.variable_dimensions(name)?,
        "{name} has a different layout in the old PFT restart"
    );
    ensure!(
        dims.first().map(String::as_str) == Some("pft"),
        "{name} is not a PFT-major variable ({dims:?})"
    );
    let mut length = 1;
    for dimension in &dims[1..] {
        length *= new.dimension(dimension)?;
    }
    Ok(length)
}

/// SAT 的 PFT 段：返回 PFT 时间重启的替换项，以及 patch 级 `ldew` 的新值
/// （`ldew(np) = sum(ldew_p(ps:pe)*pftfrac(ps:pe))`，GIMPLE 是从 0 起的顺序 FMA）。
///
/// 只处理两侧都是土壤 patch（`patchtype == 0`）的配对；同一 patch 里按 `pftclass` 升序对齐，
/// 旧的类别小就跳旧的（PFT 消失），新的小就跳新的（PFT 新增，保持冷启动值）。
pub fn pft_same_type_assignment(
    new_patch: &SatSide<'_>,
    old_patch: &SatSide<'_>,
    new: &PftSatSide<'_>,
    old: &PftSatSide<'_>,
    new_pftfrac: &[f64],
    options: SatOptions,
) -> Result<PftSatResult> {
    let pairs = match_patches(new_patch, old_patch)?;
    let mut names: Vec<&str> = PFT_ALWAYS.to_vec();
    if options.plant_hydraulics {
        names.extend(PFT_PLANT_HYDRAULICS);
    }
    if options.ozone_stress {
        names.extend(PFT_OZONE_STRESS);
    }
    // 先求出 PFT 配对，各变量共用。
    let mut pft_pairs = Vec::new();
    let mut soil_pairs = Vec::new();
    for &(np, np_) in &pairs {
        if new.patch_type[np] != 0 || old.patch_type[np_] != 0 {
            continue;
        }
        let (range, range_) = (&new.ranges[np], &old.ranges[np_]);
        ensure!(
            !range.is_empty() && !range_.is_empty(),
            "Error in REST_LulccTimeVariables LULC_IGBP_PFT|LULC_IGBP_PC! (soil patch {np} or \
             {np_} has no PFT)"
        );
        let (mut ip, mut ip_) = (range.start, range_.start);
        while ip < range.end && ip_ < range_.end {
            if new.pft_class[ip] > old.pft_class[ip_] {
                ip_ += 1;
                continue;
            }
            if new.pft_class[ip] < old.pft_class[ip_] {
                ip += 1;
                continue;
            }
            pft_pairs.push((ip, ip_));
            ip += 1;
            ip_ += 1;
        }
        soil_pairs.push(np);
    }
    let mut overrides = Vec::with_capacity(names.len());
    let mut ldew_p = Vec::new();
    for name in names {
        let row = pft_row_length(new.time, old.time, name)?;
        let mut merged = values(new.time, name)?;
        let source = values(old.time, name)?;
        for &(ip, ip_) in &pft_pairs {
            merged[ip * row..(ip + 1) * row].copy_from_slice(&source[ip_ * row..(ip_ + 1) * row]);
        }
        if name == "ldew_p" {
            ldew_p.clone_from(&merged);
        }
        overrides.push(RestartOverride::new(name, merged));
    }
    let ldew = soil_pairs
        .into_iter()
        .map(|np| {
            let value = new.ranges[np]
                .clone()
                .fold(0.0f64, |acc, ip| ldew_p[ip].mul_add(new_pftfrac[ip], acc));
            (np, value)
        })
        .collect();
    Ok((overrides, ldew))
}

/// 城市一侧：城市时间重启与每个 patch 的城市单元号（`patch2urban`，非城市 patch 为 `None`）。
pub struct UrbanSide<'a> {
    pub time: &'a RestartFile,
    pub patch_to_urban: &'a [Option<usize>],
}

/// 城市 patch 配对时整行照抄的城市变量（`MOD_Lulcc_Vars_TimeVariables.F90:886-962`，
/// MEC 的 `:1000-1077` 是同一组）。`tree_lai`/`tree_sai` 不在里面，保持冷启动值。
pub const URBAN_COPIED: [&str; 66] = [
    "fwsun", "dfwsun", "sroof", "swsun", "swsha", "sgimp", "sgper", "slake", "lwsun", "lwsha",
    "lgimp", "lgper", "lveg", "z_sno_roof", "z_sno_gimp", "z_sno_gper", "z_sno_lake",
    "dz_sno_roof", "dz_sno_gimp", "dz_sno_gper", "dz_sno_lake", "t_roofsno", "t_wallsun",
    "t_wallsha", "t_gimpsno", "t_gpersno", "t_lakesno", "troof_inner", "twsun_inner",
    "twsha_inner", "wliq_roofsno", "wice_roofsno", "wliq_gimpsno", "wice_gimpsno",
    "wliq_gpersno", "wice_gpersno", "wliq_lakesno", "wice_lakesno", "sag_roof", "sag_gimp",
    "sag_gper", "sag_lake", "scv_roof", "scv_gimp", "scv_gper", "scv_lake", "fsno_roof",
    "fsno_gimp", "fsno_gper", "fsno_lake", "snowdp_roof", "snowdp_gimp", "snowdp_gper",
    "snowdp_lake", "Fhac", "Fwst", "Fach", "Fahe", "Fhah", "vehc", "meta", "t_room", "t_roof",
    "t_wall", "tafu", "urb_green",
];

/// 城市变量每个城市单元占几个值（`urban` 轴之外各轴的乘积）。
pub fn urban_row(file: &RestartFile, name: &str) -> Result<usize> {
    let dims = file
        .variable_dimensions(name)
        .with_context(|| format!("the urban restart has no {name}"))?;
    ensure!(
        dims.first().map(String::as_str) == Some("urban"),
        "{name} is not an urban-major variable ({dims:?})"
    );
    dims[1..]
        .iter()
        .map(|dimension| file.dimension(dimension))
        .product()
}

/// 城市 SAT 的结果：城市时间重启的替换值，与 (新 patch, 新城市单元) 列表。
pub type UrbanSatResult = (Vec<RestartOverride>, Vec<(usize, usize)>);

/// 城市 SAT：配对里两边都是城市的 patch，把旧城市单元的状态整行抄到新单元上，
/// 返回写回新城市时间重启时的替换值（冷启动值为底）。
/// 返回值里还带上被抄过的 (新 patch, 新城市单元) 列表，供随后重组 patch 的水量。
pub fn urban_same_type_assignment(
    pairs: &[(usize, usize)],
    new: &UrbanSide<'_>,
    old: &UrbanSide<'_>,
) -> Result<UrbanSatResult> {
    let mut urban_pairs = Vec::new();
    let mut targets = Vec::new();
    for &(np, np_) in pairs {
        match (new.patch_to_urban[np], old.patch_to_urban[np_]) {
            (Some(u), Some(u_)) => {
                urban_pairs.push((u, u_));
                targets.push((np, u));
            }
            (None, None) => {}
            // `:868-871`：两边 patch 类型相同却只有一边有城市单元，上游停机。
            _ => bail!("Error in REST_LulccTimeVariables URBAN_MODEL: patch {np} pairs a non-urban patch"),
        }
    }
    let overrides = copy_urban_rows(&urban_pairs, new.time, old.time)?;
    Ok((overrides, targets))
}

/// 把 `(新单元, 旧单元)` 的城市状态整行抄过去（`URBAN_COPIED` 逐个变量）。
pub fn copy_urban_rows(
    urban_pairs: &[(usize, usize)],
    new: &RestartFile,
    old: &RestartFile,
) -> Result<Vec<RestartOverride>> {
    let mut overrides = Vec::new();
    for name in URBAN_COPIED {
        let row = urban_row(new, name)?;
        ensure!(
            urban_row(old, name)? == row,
            "{name} has different urban rows in the two years"
        );
        let mut merged = values(new, name)?;
        let source = values(old, name)?;
        for &(u, u_) in urban_pairs {
            merged[u * row..(u + 1) * row].copy_from_slice(&source[u_ * row..(u_ + 1) * row]);
        }
        overrides.push(RestartOverride::new(name, merged));
    }
    Ok(overrides)
}

/// 由城市分量重组城市 patch 的 `wliq_soisno`、`wice_soisno` 与 `scv`（`:964-979`）。
///
/// `(:1)` 是雪层加第一层土（`maxsnl+1..1`，前 6 个值）。GIMPLE：屋顶项是普通乘法，
/// 透水与不透水地面项是 `FMA(x*(1-froof), fgper 或 1-fgper, 累加值)`；
/// `scv = FMA(gimp*(1-froof), 1-fgper, FMA(roof, froof, (gper*(1-froof))*fgper))`。
pub fn recompose_urban_patch_water(
    targets: &[(usize, usize)],
    urban: &[RestartOverride],
    urban_time: &RestartFile,
    froof: &[f64],
    fgper: &[f64],
    patch: &mut [RestartOverride],
    patch_row: usize,
) -> Result<()> {
    let get = |name: &str| -> Result<(&[f64], usize)> {
        let entry = urban
            .iter()
            .find(|entry| entry.name == name)
            .with_context(|| format!("the urban overrides have no {name}"))?;
        Ok((&entry.values, urban_row(urban_time, name)?))
    };
    let top = SNOW_LAYERS + 1;
    for (water, roof, gper, gimp) in [
        ("wliq_soisno", "wliq_roofsno", "wliq_gpersno", "wliq_gimpsno"),
        ("wice_soisno", "wice_roofsno", "wice_gpersno", "wice_gimpsno"),
    ] {
        let (roof, roof_row) = get(roof)?;
        let (gper, gper_row) = get(gper)?;
        let (gimp, gimp_row) = get(gimp)?;
        ensure!(
            gper_row == patch_row && roof_row >= top && gimp_row >= top,
            "urban water rows do not fit the patch soil-snow column"
        );
        let target = patch
            .iter_mut()
            .find(|entry| entry.name == water)
            .with_context(|| format!("the patch overrides have no {water}"))?;
        for &(np, u) in targets {
            let (open, pervious) = (1.0 - froof[u], fgper[u]);
            let column = &mut target.values[np * patch_row..(np + 1) * patch_row];
            column.fill(0.0);
            for l in 0..top {
                column[l] = roof[u * roof_row + l] * froof[u];
            }
            for l in 0..patch_row {
                column[l] = (gper[u * gper_row + l] * open).mul_add(pervious, column[l]);
            }
            for l in 0..top {
                column[l] = (gimp[u * gimp_row + l] * open).mul_add(1.0 - pervious, column[l]);
            }
        }
    }
    let (roof, _) = get("scv_roof")?;
    let (gper, _) = get("scv_gper")?;
    let (gimp, _) = get("scv_gimp")?;
    let scv = patch
        .iter_mut()
        .find(|entry| entry.name == "scv")
        .context("the patch overrides have no scv")?;
    for &(np, u) in targets {
        let open = 1.0 - froof[u];
        let inner = roof[u].mul_add(froof[u], (gper[u] * open) * fgper[u]);
        scv.values[np] = (gimp[u] * open).mul_add(1.0 - fgper[u], inner);
    }
    Ok(())
}

#[cfg(test)]
#[path = "lulcc_tests.rs"]
mod lulcc_tests;
