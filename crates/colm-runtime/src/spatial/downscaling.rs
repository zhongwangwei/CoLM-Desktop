//! 空间强迫降尺度（`MOD_Forcing.F90` 的 `DEF_USE_Forcing_Downscaling(_Simple)` 分支，非 `SinglePoint`）。
//!
//! 初始化（`forcing_init`，`MOD_Forcing.F90:227-276`）：
//! * `forc_topo = elvmean`（`spval` 换成 0）；
//! * `topo_grid = pset2grid(forc_topo, msk=patchmask) / get_sumarea(patchmask)`，面积不为正处是 `spval`
//!   （`block_data_division`）；`maxelv_grid = pset2grid_max(forc_topo, msk=patchmask)`；
//! * 冰川标志 `patchtype == 3`。
//!
//! 每步（`read_forcing`，`MOD_Forcing.F90:715-981`）：
//! 1. 逐量 `grid2pset`（`pco2m/po2m/us/vs/psrf/sols/soll/solsd/solld/solarin/hgt/t/pbot/q/frl/hpbl`），
//!    逐份 `grid2part`；
//! 2. 逐 patch 用**步首**的 `alb` 与映射来的四波段短波算 `balb`，逐个面积为正的份：截断温度、算格点
//!    `rho`/`th`，按 patch 坐标算 `coszen`/`cosazi`，调 `downscale_forcings`；
//! 3. `part2pset` 得到 `t/q/pbot/rhoair/prc/prl/frl/swrad/us/vs`，`forc_psrf = forc_pbot`；
//! 4. 风场再逐 patch 降尺度（`us`/`vs` 为 `spval` 的跳过）；
//! 5. `#ifndef SinglePoint` 的守恒段：每份降水改成 patch 平均值，再按强迫格 `normalize` 降水、短波
//!    与长波（份值乘 `格值 / part2grid(份值)`），重新 `part2pset`；
//! 6. 按降尺度后的 `forc_swrad` 重拆四波段短波（`isnan` 时取 0）。
//!
//! 求和次序（GIMPLE，latlon 内核）：`part2pset` 与 `part2grid` 是从 0 起逐份的
//! `.FMA (sdata, areapart, acc)`；`pset2grid`（2D、无 `spv`）是 `acc + (pdata/1.)*areapart`，**不**融合；
//! `get_sumarea` 是普通加法。份的次序就是映射里每个 set 的份次序，set 按 patch 次序。

use colm_numeric::Contract;
use std::collections::HashMap;

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    downscale_forcings, downscale_wind, downscale_wind_simple, orbital_cosine_azimuth,
    orbital_cosine_zenith, potential_temperature_k, DownscalingSolarGeometry, DownscalingTerrain,
    ForcingDownscalingConfig, ForcingDownscalingInput, FullTerrain, GridForcing,
    LongwaveDownscaling, PrecipitationDownscaling, ShadowMask, SimpleTerrain,
    StandardLctSnowSoilState, ASPECT_TYPES, AZIMUTH_BINS, MISSING, SHADOW_CURVE_PARAMETERS,
    SLOPE_TYPES, ZENITH_BINS,
};
use colm_namelist::Document;

use super::forcing::{map_to_patches, CellForcing, GriddedForcing, PatchForcing};
use super::mapping::AreaWeightedMapping;

/// 降尺度方案与 `DEF_DS_*` 参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownscalingSettings {
    /// `DEF_USE_Forcing_Downscaling_Simple`（九坡向简单地形）；否则是完整地形方案。
    pub simple: bool,
    pub config: ForcingDownscalingConfig,
}

impl DownscalingSettings {
    /// 从 case namelist 读；两个开关都关时为 `None`。
    pub fn from_case(case: &Document) -> Result<Option<Self>> {
        let full = crate::optional_bool_or(case, "DEF_USE_Forcing_Downscaling", false)?;
        let simple = crate::optional_bool_or(case, "DEF_USE_Forcing_Downscaling_Simple", false)?;
        if !full && !simple {
            return Ok(None);
        }
        // `MOD_Forcing.F90:827/857`：`IF (full) … ELSEIF (simple)`，两个都开时走完整方案；
        // mksrfdata 已经拒绝了这种组合，这里照上游的优先次序。
        let scheme = |key: &str, default: &str| -> Result<String> {
            Ok(match case.get(key) {
                Some(colm_namelist::Value::Str(text)) => text.trim().to_owned(),
                Some(other) => bail!("{key} must be a string, got {other}"),
                None => default.to_owned(),
            })
        };
        let precipitation = match scheme("DEF_DS_precipitation_adjust_scheme", "I")?.as_str() {
            "I" => PrecipitationDownscaling::ElevationFraction,
            "II" => PrecipitationDownscaling::ListonElder,
            // 方案 III 经 MPI 把强迫送给外部 Python 进程算降水（`MOD_Forcing.F90:899-927`），
            // 没有可对齐的 Fortran 结果；其余取值在 `downscale_forcings` 里两支都不进，
            // `forc_prc_c/forc_prl_c` 是未赋值的 `intent(out)`。
            "III" => bail!(
                "DEF_DS_precipitation_adjust_scheme = 'III' sends the forcing to an external Python \
                 process over MPI; it is not ported to the Rust runtime"
            ),
            other => bail!(
                "DEF_DS_precipitation_adjust_scheme = {other:?} leaves the downscaled precipitation \
                 unassigned upstream; use 'I' or 'II'"
            ),
        };
        // `MOD_ForcingDownscaling.F90:604`：只认 'I'，其余都走方案 II。
        let longwave = if scheme("DEF_DS_longwave_adjust_scheme", "II")? == "I" {
            LongwaveDownscaling::ClearSky
        } else {
            LongwaveDownscaling::LapseRate
        };
        Ok(Some(Self {
            simple: !full,
            // 未写的 `DEF_DS_*` 取 schema 里登记的上游默认值。
            config: ForcingDownscalingConfig {
                temperature_lapse_rate_k_m: crate::required_real(case, "DEF_DS_TEMP_LAPSE_RATE")?,
                glacier_longwave_lapse_rate_w_m2_m: crate::required_real(
                    case,
                    "DEF_DS_LONGWAVE_LAPSE_RATE",
                )?,
                longwave_limit: crate::required_real(case, "DEF_DS_LONGWAVE_LIMIT")?,
                full_shortwave_limit: crate::required_real(case, "DEF_DS_SHORTWAVE_LIMIT")?,
                simple_shortwave_limit: crate::required_real(
                    case,
                    "DEF_DS_SHORTWAVE_SIMPLE_LIMIT",
                )?,
                longwave,
                precipitation,
            },
        }))
    }
}

/// 一个 patch 的地形因子（常数重启里的 `*_patches`）。
#[derive(Debug, Clone, PartialEq)]
pub enum PatchTerrain {
    /// 完整方案：四个坡型的坡度/坡向（弧度）与面积分数、天空视角因子、曲率、阴影曲线。
    Full {
        slope: [f64; SLOPE_TYPES],
        aspect: [f64; SLOPE_TYPES],
        area: [f64; SLOPE_TYPES],
        sky_view: f64,
        curvature: f64,
        shadow: PatchShadow,
    },
    /// 简单方案：九个坡向的坡度正切与 `asp_type_patches`、曲率。
    Simple {
        slope: [f64; ASPECT_TYPES],
        aspect: [f64; ASPECT_TYPES],
        curvature: f64,
    },
}

/// 完整方案的阴影：空间是三参数曲线 `sf_curve_patches`，单点是查表 `sf_lut_patches`。
#[derive(Debug, Clone, PartialEq)]
pub enum PatchShadow {
    Curve(Box<[[f64; SHADOW_CURVE_PARAMETERS]; AZIMUTH_BINS]>),
    Lookup(Box<[[f64; ZENITH_BINS]; AZIMUTH_BINS]>),
}

impl PatchShadow {
    fn mask(&self) -> ShadowMask<'_> {
        match self {
            Self::Curve(curve) => ShadowMask::Curve(curve),
            Self::Lookup(table) => ShadowMask::Lookup(table),
        }
    }
}

/// 一个 patch 降尺度要的静态量。
#[derive(Debug, Clone, PartialEq)]
pub struct DownscalingPatch {
    /// `forc_topo`：`elvmean`，`spval` 已换成 0。
    pub elevation_m: f64,
    /// `glacierss = patchtype == 3`。
    pub glacier: bool,
    /// `patchmask`（`topo_grid`/`maxelv_grid` 只用为真的 patch）。
    pub patch_mask: bool,
    pub terrain: PatchTerrain,
}

impl DownscalingPatch {
    /// 一个份上的 `downscale_forcings`。`solar` 是 `(calday, coszen, cosazi)`。
    fn downscale(
        &self,
        grid: GridForcing,
        blue_sky_albedo: f64,
        (calendar_day, cosine_zenith, cosine_azimuth): (f64, f64, f64),
        config: ForcingDownscalingConfig,
    ) -> Result<colm_core::DownscaledForcing> {
        let terrain = match &self.terrain {
            PatchTerrain::Full {
                slope,
                aspect,
                area,
                sky_view,
                shadow,
                ..
            } => DownscalingTerrain::Full(FullTerrain {
                slope_radians: slope,
                aspect_radians: aspect,
                area_fraction: area,
                sky_view_factor: *sky_view,
                blue_sky_albedo,
                shadow: shadow.mask(),
            }),
            // 上游把 `asp_type_patches` 传给了被调方的 `area_type_c`，照抄这个别名。
            PatchTerrain::Simple { slope, aspect, .. } => {
                DownscalingTerrain::Simple(SimpleTerrain {
                    slope_tangent: slope,
                    area_fraction: aspect,
                })
            }
        };
        downscale_forcings(
            ForcingDownscalingInput {
                glacier: self.glacier,
                grid,
                column_surface_elevation_m: self.elevation_m,
                solar: DownscalingSolarGeometry {
                    calendar_day,
                    cosine_zenith,
                    cosine_azimuth,
                },
                terrain,
            },
            config,
        )
    }

    /// patch 上的 `downscale_wind` / `downscale_wind_simple`。
    fn downscale_wind(&self, us: f64, vs: f64) -> Result<(f64, f64)> {
        match &self.terrain {
            PatchTerrain::Full {
                slope,
                aspect,
                area,
                curvature,
                ..
            } => downscale_wind(us, vs, slope, aspect, area, *curvature),
            PatchTerrain::Simple {
                slope,
                aspect,
                curvature,
            } => downscale_wind_simple(us, vs, slope, aspect, *curvature),
        }
    }
}

/// `balb`：四波段短波加权的 patch 反照率（`shortwave = [sols, solsd, soll, solld]`，`alb[band][type]`）。
/// GIMPLE：`FMA(solld, alb22, FMA(soll, alb21, FMA(sols, alb11, solsd*alb12)))/total`，
/// `total = ((sols+solsd)+soll)+solld`，为 0 时 `balb = 0`。
fn blue_sky_albedo(shortwave: [f64; 4], alb: [[f64; 2]; 2]) -> f64 {
    let [sols, solsd, soll, solld] = shortwave;
    let total = ((sols + solsd) + soll) + solld;
    if total == 0.0 {
        0.0
    } else {
        solld.contract(
            alb[1][1],
            soll.contract(alb[1][0], sols.contract(alb[0][0], solsd * alb[0][1])),
        ) / total
    }
}

/// 格点强迫：截断温度后算 `rho`（`(pbot - 0.378*q*pbot/(0.622+0.378*q))/(rgas*t)`，不融合）与 `th`。
#[allow(clippy::too_many_arguments)]
fn grid_forcing(
    (surface_elevation_m, maximum_elevation_m): (f64, f64),
    t: f64,
    q: f64,
    pbot: f64,
    (prc, prl): (f64, f64),
    frl: f64,
    reference_height_m: f64,
    solarin: f64,
    (us, vs): (f64, f64),
) -> GridForcing {
    let air_temperature_k = t.clamp(180.0, 326.0);
    GridForcing {
        surface_elevation_m,
        maximum_elevation_m,
        air_temperature_k,
        potential_temperature_k: potential_temperature_k(air_temperature_k, pbot),
        specific_humidity: q,
        bottom_pressure_pa: pbot,
        density_kg_m3: colm_core::air_density_kg_m3(pbot, q, air_temperature_k),
        convective_precipitation_kg_m2_s: prc,
        large_scale_precipitation_kg_m2_s: prl,
        downward_longwave_w_m2: frl,
        reference_height_m,
        downward_shortwave_w_m2: solarin,
        eastward_wind_m_s: us,
        northward_wind_m_s: vs,
    }
}

/// 从一个分块的常数重启读本块各 patch 的降尺度静态量（`patch_types`、`patch_mask` 与重启行一一对应）。
pub fn read_block_patches(
    constant: &colm_init::RestartFile,
    simple: bool,
    patch_types: &[i32],
    patch_mask: &[bool],
) -> Result<Vec<DownscalingPatch>> {
    let patches = patch_types.len();
    ensure!(
        patch_mask.len() == patches,
        "the downscaling patch mask needs one entry per patch"
    );
    let read = |name: &str, per_patch: usize| -> Result<&[f64]> {
        let values = constant
            .floats(name)
            .with_context(|| format!("forcing downscaling needs {name} in the constant restart"))?;
        ensure!(
            values.len() == per_patch * patches,
            "{name} has {} values, expected {per_patch} per patch for {patches} patches",
            values.len()
        );
        Ok(values)
    };
    let elevation = read("elvmean", 1)?;
    let curvature = read("cur_patches", 1)?;
    let mut out = Vec::with_capacity(patches);
    if simple {
        let slope = read("slp_type_patches", ASPECT_TYPES)?;
        let aspect = read("asp_type_patches", ASPECT_TYPES)?;
        for patch in 0..patches {
            let row = |values: &[f64]| -> [f64; ASPECT_TYPES] {
                std::array::from_fn(|k| values[patch * ASPECT_TYPES + k])
            };
            out.push(PatchTerrain::Simple {
                slope: row(slope),
                aspect: row(aspect),
                curvature: curvature[patch],
            });
        }
    } else {
        let slope = read("slp_type_patches", SLOPE_TYPES)?;
        let aspect = read("asp_type_patches", SLOPE_TYPES)?;
        let area = read("area_type_patches", SLOPE_TYPES)?;
        let sky_view = read("svf_patches", 1)?;
        // 盘上是 `(patch, zen_p, azi)` / `(patch, zen, azi)`：每个 patch 先排天顶、再排方位。
        let lookup = !constant.contains("sf_curve_patches");
        let shadow = if lookup {
            read("sf_lut_patches", ZENITH_BINS * AZIMUTH_BINS)?
        } else {
            read("sf_curve_patches", SHADOW_CURVE_PARAMETERS * AZIMUTH_BINS)?
        };
        for patch in 0..patches {
            let row = |values: &[f64]| -> [f64; SLOPE_TYPES] {
                std::array::from_fn(|k| values[patch * SLOPE_TYPES + k])
            };
            let shadow = if lookup {
                let base = patch * ZENITH_BINS * AZIMUTH_BINS;
                PatchShadow::Lookup(Box::new(std::array::from_fn(|azimuth| {
                    std::array::from_fn(|zenith| shadow[base + zenith * AZIMUTH_BINS + azimuth])
                })))
            } else {
                let base = patch * SHADOW_CURVE_PARAMETERS * AZIMUTH_BINS;
                PatchShadow::Curve(Box::new(std::array::from_fn(|azimuth| {
                    std::array::from_fn(|parameter| {
                        shadow[base + parameter * AZIMUTH_BINS + azimuth]
                    })
                })))
            };
            out.push(PatchTerrain::Full {
                slope: row(slope),
                aspect: row(aspect),
                area: row(area),
                sky_view: sky_view[patch],
                curvature: curvature[patch],
                shadow,
            });
        }
    }
    Ok(out
        .into_iter()
        .enumerate()
        .map(|(patch, terrain)| DownscalingPatch {
            // `WHERE(forc_topo == spval) forc_topo = 0.`
            elevation_m: if elevation[patch] == MISSING {
                0.0
            } else {
                elevation[patch]
            },
            glacier: patch_types[patch] == 3,
            patch_mask: patch_mask[patch],
            terrain,
        })
        .collect())
}

/// 空间降尺度的运行态：patch 静态量与强迫格上的 `topo_grid`/`maxelv_grid`/`areagrid`。
#[derive(Debug, Clone)]
pub struct SpatialDownscaling {
    settings: DownscalingSettings,
    patches: Vec<DownscalingPatch>,
    /// 映射里出现过的强迫格（含面积为 0 的份），按首次出现排序。
    cell_of: HashMap<(usize, usize), usize>,
    topo_grid: Vec<f64>,
    maxelv_grid: Vec<f64>,
    /// `mg2p_forc%areagrid`：所有 set 的份面积普通相加（缺测格的份面积已清零，故为 0）。
    area_grid: Vec<f64>,
}

impl SpatialDownscaling {
    pub fn new(
        settings: DownscalingSettings,
        patches: Vec<DownscalingPatch>,
        mapping: &AreaWeightedMapping,
    ) -> Result<Self> {
        ensure!(
            patches.len() == mapping.parts.len(),
            "forcing downscaling has {} patches but the forcing mapping {}",
            patches.len(),
            mapping.parts.len()
        );
        let mut cell_of = HashMap::new();
        for part in mapping.parts.iter().flatten() {
            let next = cell_of.len();
            cell_of.entry((part.ilon, part.ilat)).or_insert(next);
        }
        let cells = cell_of.len();
        let mut topo_sum = vec![0.0; cells];
        let mut masked_area = vec![0.0; cells];
        let mut maxelv_grid = vec![MISSING; cells];
        let mut area_grid = vec![0.0; cells];
        for (parts, patch) in mapping.parts.iter().zip(&patches) {
            for part in parts {
                let cell = cell_of[&(part.ilon, part.ilat)];
                area_grid[cell] += part.area;
                if !patch.patch_mask {
                    continue;
                }
                // `pset2grid`：`pbuff + pdata/sumwt*areapart`（`sumwt = 1.`，除以 1 是恒等），不融合。
                topo_sum[cell] += patch.elevation_m * part.area;
                masked_area[cell] += part.area;
                // `pset2grid_max`：不看份面积。
                maxelv_grid[cell] = if maxelv_grid[cell] != MISSING {
                    patch.elevation_m.max(maxelv_grid[cell])
                } else {
                    patch.elevation_m
                };
            }
        }
        // `block_data_division`：面积为正处相除，其余 `spval`。
        let topo_grid = topo_sum
            .iter()
            .zip(&masked_area)
            .map(|(&sum, &area)| if area > 0.0 { sum / area } else { MISSING })
            .collect();
        Ok(Self {
            settings,
            patches,
            cell_of,
            topo_grid,
            maxelv_grid,
            area_grid,
        })
    }

    /// `read_forcing` 在 patch 上的降尺度那一半。`albedo[p]` 是步首 patch 的 `alb(band, type)`，
    /// `calendar_day` 是步首的 `calendarday(idate)`。
    pub fn map_to_patches(
        &self,
        mapping: &AreaWeightedMapping,
        forcing: &GriddedForcing,
        cells: &CellForcing,
        states: &[StandardLctSnowSoilState],
        coordinates: &[(f64, f64)],
        calendar_day: f64,
    ) -> Result<Vec<PatchForcing>> {
        let sets = mapping.parts.len();
        ensure!(
            states.len() == sets && coordinates.len() == sets,
            "forcing downscaling needs one state and one coordinate per patch"
        );
        let config = self.settings.config;
        // 第 1 步：`grid2pset` 的那些量照常映射（`t` 与 `rhoair` 下面会被 `part2pset` 覆盖）。
        let mut patch_forcing = map_to_patches(mapping, forcing, cells);
        let reference_height_m = forcing.config().height_temperature_m;
        // 份值（面积为 0 的份不赋值，求和时乘 0 不起作用）。
        let mut parts: Vec<Vec<Option<PartForcing>>> = Vec::with_capacity(sets);
        for (index, set) in mapping.parts.iter().enumerate() {
            let patch = &self.patches[index];
            let mapped = &patch_forcing[index];
            let blue_sky_albedo = blue_sky_albedo(
                [mapped.sols, mapped.solsd, mapped.soll, mapped.solld],
                states[index].energy.radiation.albedo,
            );
            let (lon, lat) = coordinates[index];
            let cosine_zenith = orbital_cosine_zenith(calendar_day, lon, lat);
            let cosine_azimuth = orbital_cosine_azimuth(calendar_day, lon, lat, cosine_zenith);
            let mut row = Vec::with_capacity(set.len());
            for part in set {
                if part.area == 0.0 {
                    row.push(None);
                    continue;
                }
                let at = forcing.cell_index(part.ilon, part.ilat);
                let cell = self.cell_of[&(part.ilon, part.ilat)];
                let grid = grid_forcing(
                    (self.topo_grid[cell], self.maxelv_grid[cell]),
                    cells.t[at],
                    cells.q[at],
                    cells.pbot[at],
                    (cells.prc[at], cells.prl[at]),
                    cells.frl[at],
                    reference_height_m,
                    cells.solarin[at],
                    (cells.us[at], cells.vs[at]),
                );
                let out = patch
                    .downscale(
                        grid,
                        blue_sky_albedo,
                        (calendar_day, cosine_zenith, cosine_azimuth),
                        config,
                    )
                    .with_context(|| format!("forcing downscaling of patch {index}"))?;
                row.push(Some(PartForcing {
                    t: out.air_temperature_k,
                    q: out.specific_humidity,
                    pbot: out.bottom_pressure_pa,
                    rhoair: out.density_kg_m3,
                    prc: out.convective_precipitation_kg_m2_s,
                    prl: out.large_scale_precipitation_kg_m2_s,
                    frl: out.downward_longwave_w_m2,
                    swrad: out.downward_shortwave_w_m2,
                    us: out.eastward_wind_m_s,
                    vs: out.northward_wind_m_s,
                }));
            }
            parts.push(row);
        }
        // 第 3 步：`part2pset`。
        let part_to_set = |field: fn(&PartForcing) -> f64, parts: &[Vec<Option<PartForcing>>]| {
            (0..sets)
                .map(|iset| part_to_set(mapping, iset, &parts[iset], field))
                .collect::<Vec<_>>()
        };
        let t = part_to_set(|p| p.t, &parts);
        let q = part_to_set(|p| p.q, &parts);
        let pbot = part_to_set(|p| p.pbot, &parts);
        let rhoair = part_to_set(|p| p.rhoair, &parts);
        let prc = part_to_set(|p| p.prc, &parts);
        let prl = part_to_set(|p| p.prl, &parts);
        let mut us = part_to_set(|p| p.us, &parts);
        let mut vs = part_to_set(|p| p.vs, &parts);
        // 第 4 步：风场降尺度。
        for (index, patch) in self.patches.iter().enumerate() {
            if us[index] == MISSING || vs[index] == MISSING {
                continue;
            }
            (us[index], vs[index]) = patch
                .downscale_wind(us[index], vs[index])
                .with_context(|| format!("wind downscaling of patch {index}"))?;
        }
        // 第 5 步：降水的份值改成 patch 平均值，再与短波、长波一起按强迫格守恒。
        for (row, (&prc, &prl)) in parts.iter_mut().zip(prc.iter().zip(&prl)) {
            for part in row.iter_mut().flatten() {
                part.prc = prc;
                part.prl = prl;
            }
        }
        self.normalize(mapping, forcing, &cells.prc, &mut parts, |p| &mut p.prc);
        self.normalize(mapping, forcing, &cells.prl, &mut parts, |p| &mut p.prl);
        let prc = part_to_set(|p| p.prc, &parts);
        let prl = part_to_set(|p| p.prl, &parts);
        self.normalize(mapping, forcing, &cells.solarin, &mut parts, |p| {
            &mut p.swrad
        });
        self.normalize(mapping, forcing, &cells.frl, &mut parts, |p| &mut p.frl);
        let frl = part_to_set(|p| p.frl, &parts);
        let swrad = part_to_set(|p| p.swrad, &parts);
        for (index, out) in patch_forcing.iter_mut().enumerate() {
            out.t = t[index];
            out.q = q[index];
            out.pbot = pbot[index];
            // `forc_psrf = forc_pbot`
            out.psrf = pbot[index];
            out.rhoair = rhoair[index];
            out.prc = prc[index];
            out.prl = prl[index];
            out.frl = frl[index];
            out.us = us[index];
            out.vs = vs[index];
            // 第 6 步：`a = forc_swrad; IF (isnan_ud(a)) a = 0`，按步首天顶角重拆。
            let total = if swrad[index].is_nan() {
                0.0
            } else {
                swrad[index]
            };
            let (lon, lat) = coordinates[index];
            let split = colm_core::split_broadband_shortwave(
                total,
                orbital_cosine_zenith(calendar_day, lon, lat),
            );
            out.sols = split.direct_visible_w_m2;
            out.soll = split.direct_near_infrared_w_m2;
            out.solsd = split.diffuse_visible_w_m2;
            out.solld = split.diffuse_near_infrared_w_m2;
        }
        Ok(patch_forcing)
    }

    /// `spatial_mapping_normalize (gdata, sdata)`：`sumdata = part2grid(sdata)`（逐份 FMA 累加、
    /// 除以 `areagrid`），非 0 非缺测处 `sumdata = gdata/sumdata`，份值乘回 `sumdata`。
    fn normalize(
        &self,
        mapping: &AreaWeightedMapping,
        forcing: &GriddedForcing,
        grid_values: &[f64],
        parts: &mut [Vec<Option<PartForcing>>],
        field: fn(&mut PartForcing) -> &mut f64,
    ) {
        let mut sum = vec![0.0; self.area_grid.len()];
        for (set, row) in mapping.parts.iter().zip(parts.iter_mut()) {
            for (part, value) in set.iter().zip(row.iter_mut()) {
                if let Some(value) = value {
                    let cell = self.cell_of[&(part.ilon, part.ilat)];
                    sum[cell] = field(value).contract(part.area, sum[cell]);
                }
            }
        }
        // `part2grid` 的除法：`areagrid > 0` 处相除，其余是缺测（它只被面积为 0 的份引用）。
        let scale = sum
            .iter()
            .zip(&self.area_grid)
            .map(|(&sum, &area)| (area > 0.0).then_some(sum / area))
            .collect::<Vec<_>>();
        for (set, row) in mapping.parts.iter().zip(parts.iter_mut()) {
            for (part, value) in set.iter().zip(row.iter_mut()) {
                if let Some(value) = value {
                    let cell = self.cell_of[&(part.ilon, part.ilat)];
                    let mut factor = scale[cell].unwrap_or(MISSING);
                    if factor != MISSING && factor != 0.0 {
                        factor = grid_values[forcing.cell_index(part.ilon, part.ilat)] / factor;
                    }
                    *field(value) *= factor;
                }
            }
        }
    }
}

/// 单点（`SinglePoint`）的降尺度：`build_arealweighted` 在单点下每个 set 只有一份、面积为 1
/// （`MOD_SpatialMapping.F90:164-199`），`part2pset` 是恒等，也没有 `#ifndef SinglePoint` 的守恒段；
/// 阴影用查表 `sf_lut_patches`。
#[derive(Debug, Clone)]
pub struct PointDownscaling {
    settings: DownscalingSettings,
    patch: DownscalingPatch,
    /// `topo_grid`：`(0 + forc_topo*1)/(0 + 1)`；`patchmask` 为假时是 `spval`。
    grid_elevation_m: f64,
    maximum_elevation_m: f64,
    /// `forc_xy_hgt_t`。
    reference_height_m: f64,
}

impl PointDownscaling {
    pub fn new(
        settings: DownscalingSettings,
        patches: Vec<DownscalingPatch>,
        reference_height_m: f64,
    ) -> Result<Self> {
        // 多 patch 的单点（作物站点）每个 patch 的 `balb` 不同、强迫各异，而单点主循环所有 patch
        // 共用一份强迫；还没有这样的上游对照算例，先拒绝。
        let [patch] = <[DownscalingPatch; 1]>::try_from(patches).map_err(|patches| {
            anyhow::anyhow!(
                "forcing downscaling on a single point with {} patches is not ported to the Rust \
                 runtime (each patch would need its own forcing)",
                patches.len()
            )
        })?;
        let (grid_elevation_m, maximum_elevation_m) = if patch.patch_mask {
            (patch.elevation_m, patch.elevation_m)
        } else {
            (MISSING, MISSING)
        };
        Ok(Self {
            settings,
            patch,
            grid_elevation_m,
            maximum_elevation_m,
            reference_height_m,
        })
    }

    /// 把站点强迫换成降尺度后的列强迫。`albedo` 是步首 patch 的 `alb`，`calendar_day` 是步首的
    /// `calendarday(idate)`；`coszen`/`cosazi` 用 `step` 里按站点坐标算好的那两个。
    pub fn apply(
        &self,
        step: crate::PointRuntimeStep,
        albedo: [[f64; 2]; 2],
        calendar_day: f64,
    ) -> Result<crate::PointRuntimeStep> {
        let forcing = step.forcing;
        let sw = forcing.shortwave;
        let grid = grid_forcing(
            (self.grid_elevation_m, self.maximum_elevation_m),
            forcing.air_temperature_k,
            forcing.specific_humidity,
            forcing.bottom_pressure_pa,
            (
                forcing.convective_precipitation_kg_m2_s,
                forcing.large_scale_precipitation_kg_m2_s,
            ),
            forcing.downward_longwave_w_m2,
            self.reference_height_m,
            forcing.solar_in_w_m2,
            (forcing.eastward_wind_m_s, forcing.northward_wind_m_s),
        );
        let out = self
            .patch
            .downscale(
                grid,
                blue_sky_albedo(
                    [
                        sw.direct_visible_w_m2,
                        sw.diffuse_visible_w_m2,
                        sw.direct_near_infrared_w_m2,
                        sw.diffuse_near_infrared_w_m2,
                    ],
                    albedo,
                ),
                (calendar_day, forcing.cosine_zenith, step.cosine_azimuth),
                self.settings.config,
            )
            .context("forcing downscaling of the site")?;
        let (us, vs) = self
            .patch
            .downscale_wind(out.eastward_wind_m_s, out.northward_wind_m_s)
            .context("wind downscaling of the site")?;
        let total = if out.downward_shortwave_w_m2.is_nan() {
            0.0
        } else {
            out.downward_shortwave_w_m2
        };
        Ok(crate::PointRuntimeStep {
            forcing: colm_core::RuntimeForcing {
                air_temperature_k: out.air_temperature_k,
                specific_humidity: out.specific_humidity,
                surface_pressure_pa: out.bottom_pressure_pa,
                bottom_pressure_pa: out.bottom_pressure_pa,
                convective_precipitation_kg_m2_s: out.convective_precipitation_kg_m2_s,
                large_scale_precipitation_kg_m2_s: out.large_scale_precipitation_kg_m2_s,
                eastward_wind_m_s: us,
                northward_wind_m_s: vs,
                downward_longwave_w_m2: out.downward_longwave_w_m2,
                // 重拆用 `orb_coszen(calday, patchlonr, patchlatr)`，即站点的步首天顶角。
                shortwave: colm_core::split_broadband_shortwave(total, forcing.cosine_zenith),
                // `forc_rhoair` 是降尺度给出的列密度，不是由列量重算的。
                air_density_kg_m3: out.density_kg_m3,
                ..forcing
            },
            ..step
        })
    }
}

/// 一个份上降尺度后的强迫（`forc_*_part`）。
#[derive(Debug, Clone, Copy)]
struct PartForcing {
    t: f64,
    q: f64,
    pbot: f64,
    rhoair: f64,
    prc: f64,
    prl: f64,
    frl: f64,
    swrad: f64,
    us: f64,
    vs: f64,
}

/// `part2pset`：`sum(sdata*areapart)/areapset`（GIMPLE：从 0 起逐份 `.FMA`），`areapset <= 0` 为 `spval`。
fn part_to_set(
    mapping: &AreaWeightedMapping,
    iset: usize,
    row: &[Option<PartForcing>],
    field: fn(&PartForcing) -> f64,
) -> f64 {
    if mapping.area[iset] > 0.0 {
        let mut sum = 0.0;
        for (part, value) in mapping.parts[iset].iter().zip(row) {
            if let Some(value) = value {
                sum = field(value).contract(part.area, sum);
            }
        }
        sum / mapping.area[iset]
    } else {
        MISSING
    }
}

#[cfg(test)]
#[path = "downscaling_tests.rs"]
mod downscaling_tests;
