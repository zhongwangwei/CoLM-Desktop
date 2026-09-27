//! Ground humidity from `MOD_Thermal.F90` section 2 (non-split and `DEF_SPLIT_SOILSNOW`).

use anyhow::{ensure, Result};

use crate::{saturation_specific_humidity, soil_psi_from_vliq, SoilHydraulicModel};

const WATER_DENSITY_KG_M3: f64 = 1000.0;
const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_GAS_GRAVITY_MM_K: f64 = 4.71047e4;

/// Inputs used to derive the non-split `THERMAL` lower humidity boundary.
#[derive(Debug, Clone, Copy)]
pub struct GroundHumidityInput {
    /// `t_grnd`：非 split 是 `t_soisno(lb)`，split 是 `fsno*t_snow + (1-fsno)*t_soil`。
    /// 两支的 `hr = exp(psit/roverg/t_grnd)` 都用它。
    pub ground_temperature_k: f64,
    /// `t_soil = t_soisno(1)`，只有 split 用（`qsadv(t_soil, …)`）。
    pub soil_temperature_k: f64,
    /// `t_snow = t_soisno(lb)`，只有 split 用（`qsadv(t_snow, …)`）。
    pub snow_temperature_k: f64,
    pub surface_pressure_pa: f64,
    pub air_specific_humidity: f64,
    pub snow_cover_fraction: f64,
    pub top_layer_thickness_m: f64,
    pub top_layer_liquid_water_kg_m2: f64,
    pub top_layer_ice_water_kg_m2: f64,
    pub top_layer_porosity: f64,
    pub top_layer_residual_water: f64,
    pub saturated_soil_suction_mm: f64,
    pub hydraulic_model: SoilHydraulicModel,
}

/// Ground humidity and temperature derivative passed to turbulent exchange.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundHumidityState {
    pub relative_humidity: f64,
    pub humidity_reduction: f64,
    pub saturation_specific_humidity: f64,
    pub ground_specific_humidity: f64,
    pub ground_humidity_temperature_slope_kg_kg_k: f64,
    /// `q_soil`：非 split 时就是 `qg`（`MOD_Thermal.F90` 那一支末尾 `q_soil = qg`）。
    pub soil_specific_humidity: f64,
    /// `q_snow`：非 split 时同样等于 `qg`。
    pub snow_specific_humidity: f64,
}

/// Ports `MOD_Thermal`'s non-split `qred`, `qg`, and `dqgdT` calculation.
///
/// The source prevents a supersaturated lower boundary when atmospheric
/// humidity lies strictly between the soil-reduced and saturated values; that
/// exact clamp is retained here.
pub fn non_split_ground_humidity(input: GroundHumidityInput) -> Result<GroundHumidityState> {
    validate(input)?;
    let relative_humidity = relative_humidity(input);
    // `MOD_Thermal…:583` 的 GIMPLE 是 `qred = .FMA(1-fsno, hr, fsno)`。
    let humidity_reduction =
        (1.0 - input.snow_cover_fraction).mul_add(relative_humidity, input.snow_cover_fraction);
    let saturation =
        saturation_specific_humidity(input.ground_temperature_k, input.surface_pressure_pa)?;
    let reduced_humidity = humidity_reduction * saturation.specific_humidity;
    let (ground_specific_humidity, ground_humidity_temperature_slope_kg_kg_k) =
        if saturation.specific_humidity > input.air_specific_humidity
            && input.air_specific_humidity > reduced_humidity
        {
            (input.air_specific_humidity, 0.0)
        } else {
            (
                reduced_humidity,
                humidity_reduction * saturation.specific_humidity_temperature_slope_k,
            )
        };
    Ok(GroundHumidityState {
        relative_humidity,
        humidity_reduction,
        saturation_specific_humidity: saturation.specific_humidity,
        ground_specific_humidity,
        ground_humidity_temperature_slope_kg_kg_k,
        soil_specific_humidity: ground_specific_humidity,
        snow_specific_humidity: ground_specific_humidity,
    })
}

/// `DEF_SPLIT_SOILSNOW` 那一支（`MOD_Thermal.F90:634-652`）。
///
/// 土面与雪面各按自己的温度求饱和比湿：`q_soil = hr*qsat(t_soil)`、`q_snow = qsat(t_snow)`，
/// `qg` 是按雪盖加权的平均，`dqgdT` 同样两面加权。**过饱和夹取只作用在土面**
/// （`forc_q` 落在 `hr*qsatg` 与 `qsatg` 之间时 `q_soil = forc_q` 且**整个** `dqgdT` 清零，
/// 随后雪面那一份再加回来）—— 顺序照抄，不能提前合并。
pub fn split_ground_humidity(input: GroundHumidityInput) -> Result<GroundHumidityState> {
    validate(input)?;
    let relative_humidity = relative_humidity(input);
    let humidity_reduction =
        (1.0 - input.snow_cover_fraction).mul_add(relative_humidity, input.snow_cover_fraction);
    let soil = saturation_specific_humidity(input.soil_temperature_k, input.surface_pressure_pa)?;
    let mut soil_specific_humidity = relative_humidity * soil.specific_humidity;
    let mut slope = (1.0 - input.snow_cover_fraction)
        * relative_humidity
        * soil.specific_humidity_temperature_slope_k;
    if soil.specific_humidity > input.air_specific_humidity
        && input.air_specific_humidity > relative_humidity * soil.specific_humidity
    {
        soil_specific_humidity = input.air_specific_humidity;
        slope = 0.0;
    }
    let snow = saturation_specific_humidity(input.snow_temperature_k, input.surface_pressure_pa)?;
    let snow_specific_humidity = snow.specific_humidity;
    // `dqgdT = dqgdT + fsno*qsatgdT`：`变量 + 乘积`，内核收成 FMA。
    let slope = input
        .snow_cover_fraction
        .mul_add(snow.specific_humidity_temperature_slope_k, slope);
    // `qg = (1.-fsno)*q_soil + fsno*q_snow`：GIMPLE 是
    // `.FMA (1-fsno, q_soil, qsatg*fsno)` —— 熔进去的是**左边**那个乘积。
    let ground_specific_humidity = (1.0 - input.snow_cover_fraction).mul_add(
        soil_specific_humidity,
        snow_specific_humidity * input.snow_cover_fraction,
    );
    Ok(GroundHumidityState {
        relative_humidity,
        humidity_reduction,
        saturation_specific_humidity: soil.specific_humidity,
        ground_specific_humidity,
        ground_humidity_temperature_slope_kg_kg_k: slope,
        soil_specific_humidity,
        snow_specific_humidity,
    })
}

/// `hr = exp(psit/roverg/t_grnd)`（`MOD_Thermal.F90:600-619`），两支共用。
fn relative_humidity(input: GroundHumidityInput) -> f64 {
    let water_fraction = (input.top_layer_liquid_water_kg_m2 / WATER_DENSITY_KG_M3
        + input.top_layer_ice_water_kg_m2 / ICE_DENSITY_KG_M3)
        / input.top_layer_thickness_m;
    let saturation_fraction = if input.top_layer_porosity < 1.0e-6 {
        0.001
    } else {
        (water_fraction / input.top_layer_porosity).clamp(0.001, 1.0)
    };
    let soil_potential_mm = match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => {
            input.saturated_soil_suction_mm * saturation_fraction.powf(-bsw)
        }
        model => soil_psi_from_vliq(
            // `MOD_Thermal…:579` 的 GIMPLE 是 `_86 = .FMA(porsl(1)-theta_r(1), fac, theta_r(1))`
            // —— 乘积被收进加法；平铺写 `fac*(porsl-theta_r) + theta_r` 会多一次舍入。
            (input.top_layer_porosity - input.top_layer_residual_water)
                .mul_add(saturation_fraction, input.top_layer_residual_water),
            input.top_layer_porosity,
            input.top_layer_residual_water,
            input.saturated_soil_suction_mm,
            model,
        ),
    }
    .max(-1.0e8);
    (soil_potential_mm / WATER_GAS_GRAVITY_MM_K / input.ground_temperature_k).exp()
}

fn validate(input: GroundHumidityInput) -> Result<()> {
    ensure!(
        [
            input.ground_temperature_k,
            input.soil_temperature_k,
            input.snow_temperature_k,
            input.surface_pressure_pa,
            input.air_specific_humidity,
            input.snow_cover_fraction,
            input.top_layer_thickness_m,
            input.top_layer_liquid_water_kg_m2,
            input.top_layer_ice_water_kg_m2,
            input.top_layer_porosity,
            input.top_layer_residual_water,
            input.saturated_soil_suction_mm,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.ground_temperature_k > 0.0
            && input.soil_temperature_k > 0.0
            && input.snow_temperature_k > 0.0
            && input.surface_pressure_pa > 0.0
            && (0.0..=1.0).contains(&input.air_specific_humidity)
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.top_layer_thickness_m > 0.0
            && input.top_layer_liquid_water_kg_m2 >= 0.0
            && input.top_layer_ice_water_kg_m2 >= 0.0
            && input.top_layer_porosity >= 0.0
            && input.top_layer_residual_water >= 0.0
            && input.top_layer_residual_water <= input.top_layer_porosity
            && input.saturated_soil_suction_mm < 0.0
            && (input.top_layer_porosity >= 1.0e-6
                || matches!(input.hydraulic_model, SoilHydraulicModel::Campbell { .. })),
        "ground-humidity inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "ground_humidity_tests.rs"]
mod ground_humidity_tests;
