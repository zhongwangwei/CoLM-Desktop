//! Point-forcing preparation shared by the native driver and cold-start code.
//!
//! `MOD_Forcing:read_forcing` turns one canonical meteorological record into
//! the quantities consumed by `CoLMMAIN`. Keeping that hand-off here prevents
//! the Rust runtime and initializer from making incompatible wind,
//! precipitation, or broadband-shortwave assumptions.

use anyhow::{ensure, Result};

use crate::{
    orbital_cosine_zenith, partition_precipitation, PrecipitationInput, PrecipitationPhaseScheme,
    PrecipitationState, ShortwaveForcing,
};

/// Canonical one-point forcing before CoLM's runtime preparation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeForcingInput {
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    /// A northward component when `wind_is_vector`, otherwise scalar speed.
    pub northward_or_scalar_wind_m_s: f64,
    pub wind_is_vector: bool,
    pub downward_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    /// `calendarday(idate)`, already expressed in CoLM's orbital calendar.
    pub calendar_day: f64,
    /// **站点**（`patchlonr`/`patchlatr`）的经度（弧度）。
    ///
    /// 用来算 [`RuntimeForcing::cosine_zenith`]，也就是上游在
    /// `MOD_Forcing.F90:797`（地形降尺度的 `coszen` 与 `cosazi`）和
    /// `CoLMMAIN.F90:2076` 上用的那一份。
    pub longitude_radians: f64,
    /// 站点的纬度（弧度）。
    pub latitude_radians: f64,
    /// 强迫**网格单元中心**（`gforc%rlon`/`rlat`）的经度（弧度），见
    /// [`forcing_grid_center_degrees`]。
    ///
    /// **只给短波直散拆分用。** 上游 `MOD_Forcing.F90:621` 的那一处读的是网格中心，
    /// 与站点相差可达 0.5°；在拆分公式里被放大到 1.8%（实测 CN-Cng 冬季窗口的
    /// `f_solvd` 等 12 个量）。两个坐标不是一回事，别合并成一个。
    pub grid_longitude_radians: f64,
    /// 强迫网格单元中心的纬度（弧度）。
    pub grid_latitude_radians: f64,
    /// `forc_hpbl`：`DEF_USE_CBL_HEIGHT` 打开时上游额外读进来的那个强迫变量
    /// （`MOD_UserSpecifiedForcing.F90:96` 把 `NVAR` 加一）。
    ///
    /// `None` 表示强迫文件里没有它 —— 默认算例（`DEF_USE_CBL_HEIGHT = .false.`）
    /// 就是这个状态，此时只有 `Standard` 近地层方案可用。
    pub boundary_layer_height_m: Option<f64>,
}

/// One forcing record in the form consumed by CoLM's physical kernels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeForcing {
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    /// `forc_psrf`, copied from the source pressure field.
    pub surface_pressure_pa: f64,
    /// `forc_pbot`, copied from the same source pressure field.
    pub bottom_pressure_pa: f64,
    /// `forc_prc = precipitation / 3`.
    pub convective_precipitation_kg_m2_s: f64,
    /// `forc_prl = precipitation * 2 / 3`.
    pub large_scale_precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub downward_longwave_w_m2: f64,
    pub shortwave: ShortwaveForcing,
    /// `forc_solarin`：驱动里那一列**总**短波，原样带走。
    ///
    /// 必须单独留一份，不能拿 [`Self::shortwave`] 四个波段相加顶替 ——
    /// 拆波段是"总量 × 权重"再四舍五入，加回去不保证逐位回到总量。实测
    /// `CN-Cng` 冬季窗口 264 条里有 13 条的 `f_xy_solarin` 因此差 1 ULP
    /// （tier0 是逐位比较，会红）。上游 `MOD_Forcing` 是反着来的：先有总量
    /// `forc_solarin`，再拆出四个波段，而 `f_xy_solarin` 照抄的是总量。
    pub solar_in_w_m2: f64,
    pub cosine_zenith: f64,
    /// `forc_rhoair` from `MOD_Forcing`, the density every THERMAL branch carries.
    ///
    /// It belongs to the forcing hand-off rather than to a caller: `CoLMMAIN` reads
    /// it from the same module that prepared the other forcings, and recomputing it
    /// per branch is how the ground, leaf and soil-resistance paths would drift apart.
    pub air_density_kg_m3: f64,
    /// `forc_hpbl` 原样带走；选不选 LES 是近地层方案的事，不是强迫场的事。
    pub boundary_layer_height_m: Option<f64>,
}

impl RuntimeForcing {
    /// Builds the existing rain/snow kernel's input without a second mapping.
    pub fn precipitation_input(
        self,
        patch_type: i32,
        scheme: PrecipitationPhaseScheme,
    ) -> PrecipitationInput {
        PrecipitationInput {
            patch_type,
            air_temperature_k: self.air_temperature_k,
            specific_humidity: self.specific_humidity,
            surface_pressure_pa: self.surface_pressure_pa,
            convective_precipitation_kg_m2_s: self.convective_precipitation_kg_m2_s,
            large_scale_precipitation_kg_m2_s: self.large_scale_precipitation_kg_m2_s,
            eastward_wind_m_s: self.eastward_wind_m_s,
            northward_wind_m_s: self.northward_wind_m_s,
            scheme,
        }
    }

    /// Runs CoLM's existing rain/snow partition directly from this prepared record.
    pub fn partition_precipitation(
        self,
        patch_type: i32,
        scheme: PrecipitationPhaseScheme,
    ) -> Result<PrecipitationState> {
        partition_precipitation(self.precipitation_input(patch_type, scheme))
    }
}

/// 站点坐标 → 它所落在的**强迫网格单元中心**（度）。
///
/// CoLM 的 `SinglePoint` 构型把强迫网格钉成 360×180 的 1° 全球网格
/// （`MOD_Namelist.F90` 的 `#ifdef SinglePoint` 把 `DEF_nx_blocks`/`DEF_ny_blocks`
/// 直接赋成 360/180），再由 `MOD_Grid.F90::grid_define_by_ndims` 生成边界：
/// `lat_s = 90 - ilat`、`lat_n = 91 - ilat`、`lon_w = -180 + (ilon-1)`、`lon_e = -180 + ilon`。
/// 所以单元中心就是"下界 + 0.5"，而包含 `x` 的那个单元的判据是 `lat_s <= x < lat_n`
/// —— 对 `x` 取 `floor(x) + 0.5` 正好（负数也对：−44.5933 落在 [−45,−44]，中心 −44.5）。
///
/// `MOD_Forcing` 的短波拆分读的是 `gforc%rlat`/`rlon`（网格单元中心），
/// 而 `CoLMMAIN` 的 `coszen` 读的是 `patchlatr`/`patchlonr`（站点）。实测 CN-Cng：
/// 站点 44.5933/123.5092，网格中心 44.5/123.5，短波拆分因此差 1.77%。
///
/// 只在 `x` 恰好取到网格外边界（±90、±180）时 `floor` 与上游的搜索规则会分界不同；
/// 这两个值在 1° 网格上是退化情形，本仓库的站点算例碰不到。
pub fn forcing_grid_center_degrees(latitude_degrees: f64, longitude_degrees: f64) -> (f64, f64) {
    (
        latitude_degrees.floor() + 0.5,
        longitude_degrees.floor() + 0.5,
    )
}

/// Ports the non-downscaled, all-band branch of `MOD_Forcing:read_forcing`.
///
/// CoLM represents scalar wind as equal east/north components, splits source
/// precipitation into one-third convective and two-thirds large-scale, then
/// derives visible/NIR and direct/diffuse shortwave components.
pub fn prepare_runtime_forcing(input: RuntimeForcingInput) -> Result<RuntimeForcing> {
    validate(input)?;
    let (eastward_wind_m_s, northward_wind_m_s) = if input.wind_is_vector {
        (input.eastward_wind_m_s, input.northward_or_scalar_wind_m_s)
    } else {
        // **乘 `1/sqrt(2)` 而不是除以 `sqrt(2)`。** 上游是
        // `forc_xy_us = forcn(6) * (1/sqrt(2.0_r8))`（`MOD_Forcing.F90:547-549`），
        // gfortran 把 `1/sqrt(2)` 折成一个 f64 常量再乘。除法与"乘倒数"在末位
        // 会分叉，而 `f_xy_us`/`f_xy_vs` 是 tier0（逐位）比较 —— 实测 264 条里
        // 39 条因此差 1 ULP。
        let component = input.northward_or_scalar_wind_m_s * (1.0 / 2.0_f64.sqrt());
        (component, component)
    };
    // 站点坐标：上游 `MOD_Forcing.F90:797` 的地形降尺度 `coszen`/`cosazi`
    // 与 `CoLMMAIN.F90:2076` 都用它。
    let cosine_zenith = orbital_cosine_zenith(
        input.calendar_day,
        input.longitude_radians,
        input.latitude_radians,
    );
    // **短波拆分另用网格单元中心。** `MOD_Forcing.F90:621` 是
    // `sunang = orb_coszen(calday, gforc%rlon, gforc%rlat)`；站点与网格中心
    // 相差可达 0.5°，拆分对 `sunang` 强非线性，实测差到 1.77%。
    let sun_angle = orbital_cosine_zenith(
        input.calendar_day,
        input.grid_longitude_radians,
        input.grid_latitude_radians,
    );
    Ok(RuntimeForcing {
        air_temperature_k: input.air_temperature_k,
        specific_humidity: input.specific_humidity,
        surface_pressure_pa: input.surface_pressure_pa,
        bottom_pressure_pa: input.surface_pressure_pa,
        convective_precipitation_kg_m2_s: input.precipitation_kg_m2_s / 3.0,
        large_scale_precipitation_kg_m2_s: input.precipitation_kg_m2_s * 2.0 / 3.0,
        eastward_wind_m_s,
        northward_wind_m_s,
        downward_longwave_w_m2: input.downward_longwave_w_m2,
        solar_in_w_m2: input.downward_shortwave_w_m2,
        shortwave: split_broadband_shortwave(input.downward_shortwave_w_m2, sun_angle),
        cosine_zenith,
        air_density_kg_m3: air_density_kg_m3(
            input.surface_pressure_pa,
            input.specific_humidity,
            input.air_temperature_k,
        ),
        boundary_layer_height_m: input.boundary_layer_height_m,
    })
}

/// `MOD_Forcing`'s `forc_rhoair`, including its own `forc_t > 326` guard.
///
/// The guard exists in the upstream reader because density is evaluated from an
/// air temperature the reader is still allowed to clamp; applying it here keeps
/// that clamp with the formula instead of leaking a corrected temperature into
/// the rest of `RuntimeForcing`.
pub(crate) fn air_density_kg_m3(
    surface_pressure_pa: f64,
    specific_humidity: f64,
    air_temperature_k: f64,
) -> f64 {
    const AIR_GAS_CONSTANT_J_KG_K: f64 = 287.04;
    let temperature_k = air_temperature_k.min(326.0);
    (surface_pressure_pa
        - 0.378 * specific_humidity * surface_pressure_pa / (0.622 + 0.378 * specific_humidity))
        / (AIR_GAS_CONSTANT_J_KG_K * temperature_k)
}

fn validate(input: RuntimeForcingInput) -> Result<()> {
    for value in [
        input.air_temperature_k,
        input.specific_humidity,
        input.surface_pressure_pa,
        input.precipitation_kg_m2_s,
        input.eastward_wind_m_s,
        input.northward_or_scalar_wind_m_s,
        input.downward_shortwave_w_m2,
        input.downward_longwave_w_m2,
        input.calendar_day,
        input.longitude_radians,
        input.latitude_radians,
        input.grid_longitude_radians,
        input.grid_latitude_radians,
    ] {
        ensure!(value.is_finite(), "runtime forcing must be finite");
    }
    ensure!(
        input.air_temperature_k > 0.0
            && (0.0..1.0).contains(&input.specific_humidity)
            && input.surface_pressure_pa > 0.0
            && input.precipitation_kg_m2_s >= 0.0
            && input.downward_shortwave_w_m2 >= 0.0
            && input.downward_longwave_w_m2 >= 0.0
            && input.latitude_radians.abs() <= std::f64::consts::FRAC_PI_2
            && input.longitude_radians.abs() <= std::f64::consts::PI
            && input.grid_latitude_radians.abs() <= std::f64::consts::FRAC_PI_2
            && input.grid_longitude_radians.abs() <= std::f64::consts::PI
            && (input.wind_is_vector || input.northward_or_scalar_wind_m_s >= 0.0),
        "runtime forcing is physically invalid"
    );
    // `hpbl` 可以缺（默认算例就没有），但给了就必须是正的有限值 ——
    // 它是 LZD2022 廓线的长度尺度，0 或负数只会让 `boundary_zeta` 除出垃圾。
    if let Some(boundary_layer_height_m) = input.boundary_layer_height_m {
        ensure!(
            boundary_layer_height_m.is_finite() && boundary_layer_height_m > 0.0,
            "forc_hpbl must be a positive finite height, got {boundary_layer_height_m}"
        );
    }
    Ok(())
}

pub(crate) fn split_broadband_shortwave(total_w_m2: f64, cosine_zenith: f64) -> ShortwaveForcing {
    let mut cloud = if cosine_zenith == 0.0 {
        0.0
    } else {
        (1160.0 * cosine_zenith - total_w_m2) / (963.0 * cosine_zenith)
    };
    cloud = cloud.max(0.0001);
    cloud = cloud.min(1.0);
    cloud = cloud.max(0.58);
    let mut diffuse_fraction = 0.0604 / (cosine_zenith - 0.0223) + 0.0683;
    diffuse_fraction = diffuse_fraction.max(0.0);
    diffuse_fraction = diffuse_fraction.min(1.0);
    diffuse_fraction += (1.0 - diffuse_fraction) * cloud;
    let visible_fraction =
        (580.0 - cloud * 464.0) / ((580.0 - cloud * 499.0) + (580.0 - cloud * 464.0));
    ShortwaveForcing {
        direct_visible_w_m2: total_w_m2 * (1.0 - diffuse_fraction) * visible_fraction,
        direct_near_infrared_w_m2: total_w_m2 * (1.0 - diffuse_fraction) * (1.0 - visible_fraction),
        diffuse_visible_w_m2: total_w_m2 * diffuse_fraction * visible_fraction,
        diffuse_near_infrared_w_m2: total_w_m2 * diffuse_fraction * (1.0 - visible_fraction),
    }
}

#[cfg(test)]
#[path = "runtime_forcing_tests.rs"]
mod runtime_forcing_tests;
