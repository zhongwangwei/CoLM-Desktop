//! 上游 `CoLMMAIN.F90` 每步末尾的「Preparation for the next time step」一节
//! （`CoLMMAIN.F90:2068-2200`）。
//!
//! 这一段由**本步结束时的状态**重算**下一步**要用的东西：冠层几何（`lai`/`sai`）、
//! 雪盖（`wt`/`sigf`/`fsno`）、雪龄 `sag`，以及地面与冠层光学系数
//! （`alb`/`ssun`/`ssha`/`ssoi`/`ssno`/`thermk`/`extkb`/`extkd`）。
//!
//! 为什么它必须单独成层：它读的是本步的**输出**（`z0m`、`t_grnd`、`scv`），写出的是
//! 下一步的**输入**。上游把它放在时间循环体末尾而不是任何物理内核里。本仓库原先只在
//! `mkinidata` 里调过一次冷启动版本，于是运行期的 `alb`/`extkb`/`fsno`/`sag` 永远停在
//! 启动时刻 —— 实测对齐算例 528 步之后 Fortran 的 `fsno = 0.0176`、`sag = 0.00124`，
//! 而 Rust 两个都是 0，`extkb` 还停在冷启动那次调用的值上。

use anyhow::{ensure, Result};

use crate::radiation::{
    aged_snow_albedo, broadband_radiation_from_ground_using, soil_albedo, TwoStreamKind,
};
use crate::snow::snow_fraction;
use crate::{ColdStartGroundAlbedo, ColdStartRadiation, LeafOptics, SoilReflectance};

/// 一次「下一步表面光学」计算的全部输入。
///
/// 「时间变量」（`tlai`/`tsai`）与「上一步折算出的」（`lai`/`sai`）分开命名：
/// `snowfraction` 吃 `tlai`/`tsai` 吐 `lai`/`sai`，混用会让 `sigf` 每步被重复乘一次。
#[derive(Debug, Clone, Copy)]
pub struct SurfaceOpticsInput {
    pub patch_type: i32,
    pub time_step_seconds: f64,
    /// `coszen`，**未截断**：上游先用它判夜间（`coszen <= -0.3` 直接返回）。
    pub cosine_zenith: f64,
    /// 本步能量链算出的地面温度 `t_grnd` [K]。
    pub ground_temperature_k: f64,
    /// 时间变量 `tlai`（遥感/物候给的总叶面积）。
    pub temporal_leaf_area_index: f64,
    /// 时间变量 `tsai`。
    pub temporal_stem_area_index: f64,
    /// 本步 `THERMAL` 算出的冠层动量粗糙度 `z0m` [m]。
    pub momentum_roughness_m: f64,
    /// 裸土粗糙度 `zlnd`（`param%z0s`）[m]。
    pub soil_roughness_m: f64,
    /// `DEF_TUNING_SNOW_COVER_EXPONENT` [-]，默认 1。
    pub snow_cover_exponent: f64,
    pub soil: SoilReflectance,
    /// 第一层土壤的液态水量与厚度，用来算 `ssw`。
    pub soil_liquid_water_kg_m2: f64,
    pub soil_thickness_m: f64,
    pub optics: LeafOptics,
    /// 本步的 `fwet_snow`。
    pub wet_snow_fraction: f64,
    pub snow_water_equivalent_mm: f64,
    /// **本步开始时**的 `scv`，即上游的 `scvold`（`CoLMMAIN.F90:814`）。
    pub previous_snow_water_equivalent_mm: f64,
    pub snow_depth_m: f64,
    /// 本步结束时的雪层数（`snl`）；`0` 表示只有"没有层"的浅雪。
    pub snow_layers: i32,
    /// 本步开始时的雪龄 `sag`。
    pub snow_age: f64,
    pub use_lct: bool,
    pub usgs_land_cover: bool,
    pub vegetation_snow: bool,
    /// `DEF_USE_LAIFEEDBACK`（PFT/PC）：`lai_p` 由 BGC 维护，准备段不再用 `tlai_p` 重置它，
    /// patch `lai = sum(lai_p·pftfrac)`（`CoLMMAIN.F90:2116-2117`）。
    pub lai_feedback: bool,
}

/// 「下一步」的冠层几何与雪盖。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceOptics {
    /// `lai`：`vegetation_snow` 打开时是 `tlai*sigf`，否则就是 `tlai`。
    pub leaf_area_index: f64,
    /// `sai = tsai*sigf`。
    pub stem_area_index: f64,
    pub vegetation_snow_fraction: f64,
    pub vegetation_free_fraction: f64,
    pub ground_snow_fraction: f64,
    pub snow_age: f64,
}

/// 执行 `CoLMMAIN.F90` 的「Preparation for the next time step」。
///
/// `radiation` 就地更新为这一步算出的光学系数。夜间那一支同样要写回默认值 ——
/// 上游的 `alb`/`ssun`/`ssha`/`ssoi`/`ssno`/`tran`/`thermk`/`extkb`/`extkd` 都是
/// module 变量，`albland` 一进门就把它们置默认，然后才在 `coszen <= -0.3` 处返回。
pub fn prepare_surface_optics(
    input: SurfaceOpticsInput,
    radiation: &mut ColdStartRadiation,
) -> Result<SurfaceOptics> {
    prepare_surface_optics_with_snicar(input, radiation, None)
}

/// [`prepare_surface_optics`] 带 `DEF_USE_SNICAR` 的雪反照率（见 [`crate::SnicarAlbedoHook`]）。
pub fn prepare_surface_optics_with_snicar(
    input: SurfaceOpticsInput,
    radiation: &mut ColdStartRadiation,
    snicar: Option<&mut crate::SnicarAlbedoHook<'_>>,
) -> Result<SurfaceOptics> {
    ensure!(
        input.patch_type >= 0
            && input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.cosine_zenith.is_finite()
            && input.ground_temperature_k.is_finite()
            && input.temporal_leaf_area_index.is_finite()
            && input.temporal_stem_area_index.is_finite()
            && input.soil_liquid_water_kg_m2.is_finite()
            && input.soil_liquid_water_kg_m2 >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
            && input.soil_thickness_m.is_finite()
            && input.soil_thickness_m > 0.0
            && input.snow_water_equivalent_mm.is_finite()
            && input.snow_water_equivalent_mm >= 0.0
            && input.previous_snow_water_equivalent_mm.is_finite()
            && input.previous_snow_water_equivalent_mm >= 0.0
            && input.snow_age.is_finite()
            && input.snow_age >= 0.0
            && input.wet_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&input.wet_snow_fraction)
            && (-5..=0).contains(&input.snow_layers),
        "surface-optics inputs are invalid"
    );

    let fraction = snow_fraction(
        input.temporal_leaf_area_index,
        input.temporal_stem_area_index,
        input.momentum_roughness_m,
        input.soil_roughness_m,
        input.snow_water_equivalent_mm,
        input.snow_depth_m,
        input.snow_cover_exponent,
    )?;
    // `CoLMMAIN.F90:2097-2102`：`sai` 一律乘 `sigf`；`lai` 只有在 `DEF_VEG_SNOW`
    // 打开（"用无雪的 LAI"）时才乘。
    let leaf_area_index = if input.vegetation_snow {
        input.temporal_leaf_area_index * fraction.vegetation_free_fraction
    } else {
        input.temporal_leaf_area_index
    };
    let stem_area_index = input.temporal_stem_area_index * fraction.vegetation_free_fraction;

    // `CoLMMAIN.F90:2135`：`ssw = min(1., 1e-3*wliq_soisno(1)/dz_soisno(1))`；
    // `patchtype >= 3`（水体/冰面）直接取 1 —— 没有"土壤湿度"这一说。
    let mut surface_wetness =
        (1.0e-3 * input.soil_liquid_water_kg_m2 / input.soil_thickness_m).min(1.0);
    if input.patch_type >= 3 {
        surface_wetness = 1.0;
    }

    let snow_age = albland(
        &input,
        fraction.ground_snow_fraction,
        surface_wetness,
        leaf_area_index,
        stem_area_index,
        radiation,
        snicar,
    )?;
    Ok(SurfaceOptics {
        leaf_area_index,
        stem_area_index,
        vegetation_snow_fraction: fraction.vegetation_snow_fraction,
        vegetation_free_fraction: fraction.vegetation_free_fraction,
        ground_snow_fraction: fraction.ground_snow_fraction,
        snow_age,
    })
}

/// `MOD_Albedo.F90:28-455` 的 `albland`，非 SNICAR、非 PFT/PC 那一支。
///
/// 返回值是**新的 `sag`**（上游的 `intent(inout)`）：夜间提前返回时不变。
fn albland(
    input: &SurfaceOpticsInput,
    ground_snow_fraction: f64,
    surface_wetness: f64,
    leaf_area_index: f64,
    stem_area_index: f64,
    radiation: &mut ColdStartRadiation,
    snicar: Option<&mut crate::SnicarAlbedoHook<'_>>,
) -> Result<f64> {
    let Some((ground, previous_thermal_gap_fraction, czen)) = albland_ground(
        input,
        ground_snow_fraction,
        surface_wetness,
        leaf_area_index + stem_area_index,
        radiation,
        snicar,
    )?
    else {
        return Ok(input.snow_age);
    };
    // 第 4 节：叠冠层两流。
    *radiation = broadband_radiation_from_ground_using(
        input.patch_type,
        ground,
        input.optics,
        leaf_area_index,
        stem_area_index,
        input.wet_snow_fraction,
        czen,
        input.use_lct,
        input.vegetation_snow,
        TwoStreamKind::LandCover {
            usgs_land_cover: input.usgs_land_cover,
        },
        previous_thermal_gap_fraction,
    )?;
    Ok(ground.snow_age)
}

/// `albland` 的前三节：默认化、夜间返回与地面（土 + 雪）反照率。
///
/// 夜间返回 `None`（`radiation` 已写成默认值）；白天返回地面反照率、
/// 按冠层面积重置过的上一步 `thermk` 与截断后的 `czen`。
fn albland_ground(
    input: &SurfaceOpticsInput,
    ground_snow_fraction: f64,
    surface_wetness: f64,
    leaf_stem_area: f64,
    radiation: &mut ColdStartRadiation,
    mut snicar: Option<&mut crate::SnicarAlbedoHook<'_>>,
) -> Result<Option<(ColdStartGroundAlbedo, f64, f64)>> {
    // 第 1 节：`thermk` 只在无冠层时被重置为 1（`MOD_Albedo.F90:225-228`，注释写明
    // "夜间长波用上一步的值"）；有冠层时它保留上一次调用的结果。
    let previous_thermal_gap_fraction = if leaf_stem_area <= 1.0e-6 {
        1.0
    } else {
        radiation.thermal_gap_fraction
    };

    // SNICAR（`MOD_Albedo.F90:266-292`）：`AerosolMasses` 与 `SnowAge_grain` 在夜间返回**之前**，
    // 每步都跑；`ssno_lyr` 在入口清零。
    if let Some(hook) = snicar.as_deref_mut() {
        hook.before_night_return(ground_snow_fraction)?;
    }
    // 夜间：上游在算任何物理量之前就返回，但 module 变量已经被写成默认值。
    // 实测对齐算例 2008-01-12 00:00（`coszen = -0.922`）的重启里 `alb = 1`、
    // `ssun`/`ssha`/`ssoi`/`ssno` 全 0、`extkb = 1`、`extkd = 0.718`，
    // 而 `thermk = 0.547` 是上一步白天的值。
    if input.cosine_zenith <= -0.3 {
        radiation.thermal_gap_fraction = previous_thermal_gap_fraction;
        radiation.albedo = [[1.0; 2]; 2];
        radiation.sunlit_absorption = [[0.0; 2]; 2];
        radiation.shaded_absorption = [[0.0; 2]; 2];
        radiation.soil_absorption = [[0.0; 2]; 2];
        radiation.snow_absorption = [[0.0; 2]; 2];
        radiation.transmission = Some([[0.0, 1.0, 1.0]; 2]);
        radiation.direct_extinction = 1.0;
        radiation.diffuse_extinction = 0.718;
        return Ok(None);
    }
    let czen = input.cosine_zenith.max(0.001);

    // 第 2 节：地面反照率。`albsoi` 在雪盖混合**之前**存档，最后的 `ssoi` 用它。
    let soil = soil_albedo(
        input.patch_type,
        input.soil,
        surface_wetness,
        input.ground_temperature_k,
        czen,
    )?;

    // 第 3 节：非 SNICAR 的雪面反照率。`snl == 0` 时上游先把雪龄清零再老化一步
    // —— 没有雪层就没有承载雪龄的地方；`scv <= 0` 时 `albsno` 保持初值 1、`sag` 不动。
    let (snow, snow_age) =
        if let (true, Some(hook)) = (input.snow_water_equivalent_mm > 0.0, snicar.as_mut()) {
            // SNICAR 那一支不调 `snowage`，`sag` 原样保留（`:338-373`）；下垫面是土壤漫射反照率。
            (
                hook.snow_albedo(czen, [soil[0][1], soil[1][1]])?,
                input.snow_age,
            )
        } else if input.snow_water_equivalent_mm > 0.0 {
            let previous_age = if input.snow_layers == 0 {
                0.0
            } else {
                input.snow_age
            };
            aged_snow_albedo(
                input.snow_water_equivalent_mm,
                input.previous_snow_water_equivalent_mm,
                input.ground_temperature_k,
                czen,
                input.time_step_seconds,
                previous_age,
            )?
        } else {
            ([[1.0; 2]; 2], input.snow_age)
        };

    // 第 3.1 节：按雪盖比例混合。
    let ground = crate::mix_ground_albedo(soil, snow, ground_snow_fraction);
    Ok(Some((
        ColdStartGroundAlbedo {
            soil,
            snow,
            ground,
            snow_age,
        },
        previous_thermal_gap_fraction,
        czen,
    )))
}

/// PFT patch 的「Preparation for the next time step」（`CoLMMAIN.F90:2112-2130` 与
/// `albland` 的 PFT 段）。
///
/// `input` 里 patch 级的 `temporal_leaf_area_index` 是 `tlai(ipatch)`（关掉
/// `DEF_VEG_SNOW` 时 `lai` 直接取它）；`momentum_roughness_m`、`optics`、
/// `wet_snow_fraction` 不读 —— 那三样都换成了逐 PFT 的量。
pub fn prepare_pft_surface_optics(
    input: SurfaceOpticsInput,
    patch: &mut crate::PftPatch,
    radiation: &mut ColdStartRadiation,
) -> Result<SurfaceOptics> {
    prepare_pft_surface_optics_with_snicar(input, patch, radiation, None)
}

/// [`prepare_pft_surface_optics`] 带 `DEF_USE_SNICAR` 的雪反照率。
pub fn prepare_pft_surface_optics_with_snicar(
    input: SurfaceOpticsInput,
    patch: &mut crate::PftPatch,
    radiation: &mut ColdStartRadiation,
    snicar: Option<&mut crate::SnicarAlbedoHook<'_>>,
) -> Result<SurfaceOptics> {
    ensure!(
        input.patch_type == 0
            && input.cosine_zenith.is_finite()
            && input.soil_thickness_m > 0.0
            && input.snow_water_equivalent_mm >= 0.0
            && (-5..=0).contains(&input.snow_layers),
        "PFT surface-optics inputs are invalid"
    );
    let fraction = crate::pft_snow_fraction(
        patch,
        input.soil_roughness_m,
        input.snow_water_equivalent_mm,
        input.snow_depth_m,
        input.snow_cover_exponent,
        input.vegetation_snow,
    )?;
    for column in &mut patch.columns {
        if !input.lai_feedback {
            column.leaf_area_index = if input.vegetation_snow {
                column.temporal_leaf_area_index * column.vegetation_free_fraction
            } else {
                column.temporal_leaf_area_index
            };
        }
        column.stem_area_index = column.temporal_stem_area_index * column.vegetation_free_fraction;
    }
    let leaf_area_index = if input.vegetation_snow || input.lai_feedback {
        patch.sum(|column| column.leaf_area_index)
    } else {
        input.temporal_leaf_area_index
    };
    let stem_area_index = patch.sum(|column| column.stem_area_index);

    let surface_wetness =
        (1.0e-3 * input.soil_liquid_water_kg_m2 / input.soil_thickness_m).min(1.0);
    // `albland` 入口对 PFT 量的默认化在夜间返回**之前**（`MOD_Albedo.F90:247-257`）。
    crate::pft::reset_pft_radiation(patch);
    let snow_age = match albland_ground(
        &input,
        fraction.ground_snow_fraction,
        surface_wetness,
        leaf_area_index + stem_area_index,
        radiation,
        snicar,
    )? {
        None => input.snow_age,
        Some((ground, previous_thermal_gap_fraction, czen)) => {
            radiation.thermal_gap_fraction = previous_thermal_gap_fraction;
            crate::pft::pft_canopy_radiation(
                patch,
                ground,
                czen,
                leaf_area_index + stem_area_index,
                input.vegetation_snow,
                radiation,
            )?;
            ground.snow_age
        }
    };
    Ok(SurfaceOptics {
        leaf_area_index,
        stem_area_index,
        vegetation_snow_fraction: fraction.vegetation_snow_fraction,
        vegetation_free_fraction: fraction.vegetation_free_fraction,
        ground_snow_fraction: fraction.ground_snow_fraction,
        snow_age,
    })
}

#[cfg(test)]
#[path = "surface_optics_tests.rs"]
mod surface_optics_tests;
