//! Surface-water flux partition from `MOD_Thermal.F90`.
//!
//! `GroundTemperature` has updated the upper packed layer before this function
//! is called.  This keeps the water availability cap and its compensating
//! sensible-heat correction in one shared runtime kernel.

use anyhow::{ensure, Result};

use crate::FREEZING_K;

/// Inputs to the non-split `MOD_Thermal` surface-water partition.
///
/// `corrected_ground_evaporation_kg_m2_s` is `fevpg` after the solved ground
/// temperature correction.  The upper packed layer is soil when snow is absent
/// and the top snow layer otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalWaterInput {
    pub corrected_ground_evaporation_kg_m2_s: f64,
    pub upper_liquid_water_kg_m2: f64,
    pub upper_ice_water_kg_m2: f64,
    pub upper_temperature_k: f64,
    pub time_step_seconds: f64,
    pub ground_latent_heat_j_kg: f64,
}

/// Water and energy diagnostics from [`partition_no_split_thermal_water`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ThermalWaterFluxes {
    /// `fevpg` after limiting removal to the upper-layer water inventory.
    pub ground_evaporation_kg_m2_s: f64,
    /// Positive liquid-water removal (`qseva`).
    pub evaporation_kg_m2_s: f64,
    /// Positive ice removal (`qsubl`).
    pub sublimation_kg_m2_s: f64,
    /// Positive liquid-water addition (`qsdew`).
    pub dew_kg_m2_s: f64,
    /// Positive ice-water addition (`qfros`).
    pub frost_kg_m2_s: f64,
    /// Evaporation demand that the upper layer cannot supply (`egidif`).
    pub water_limited_evaporation_kg_m2_s: f64,
    /// `htvp * egidif`, to be added to the ground sensible heat (`fseng`).
    pub sensible_heat_correction_w_m2: f64,
}

/// Inputs to the split soil/snow section of `MOD_Thermal`.
///
/// 四个面量都已经加过 `tinc*cgrnds`/`tinc*cgrndl`（`MOD_Thermal.F90:1347-1352`，
/// 乘积各算一次、**平铺**加上）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitThermalWaterInput {
    pub snow_layer_exists: bool,
    pub snow_cover_fraction: f64,
    pub corrected_soil_sensible_heat_w_m2: f64,
    pub corrected_snow_sensible_heat_w_m2: f64,
    pub corrected_soil_evaporation_kg_m2_s: f64,
    pub corrected_snow_evaporation_kg_m2_s: f64,
    pub soil_liquid_water_kg_m2: f64,
    pub soil_ice_water_kg_m2: f64,
    pub soil_temperature_k: f64,
    pub snow_liquid_water_kg_m2: f64,
    pub snow_ice_water_kg_m2: f64,
    pub snow_temperature_k: f64,
    pub time_step_seconds: f64,
    pub ground_latent_heat_j_kg: f64,
}

/// Area-mean split soil/snow fluxes passed to `WATER_2014`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitThermalWaterFluxes {
    /// 合成后的 `fseng`：有雪层 `FMA(fseng_soil, 1-fsno, fseng_snow*fsno)`，无雪层就是 `fseng_soil`。
    pub ground_sensible_heat_w_m2: f64,
    /// 合成后的 `fevpg`，同上。
    pub ground_evaporation_kg_m2_s: f64,
    /// Soil component, already weighted by the uncovered fraction when snow exists.
    pub soil: ThermalWaterFluxes,
    /// Snow component, already weighted by snow cover when a snow layer exists.
    pub snow: ThermalWaterFluxes,
}

/// Ports the non-split section of `MOD_Thermal` after `GroundTemperature`.
///
/// Split soil/snow has separate soil and snow budgets and therefore belongs to
/// its own branch; it is intentionally not represented by these one-surface
/// diagnostics.
pub fn partition_no_split_thermal_water(input: ThermalWaterInput) -> Result<ThermalWaterFluxes> {
    validate(input)?;

    let maximum_removal =
        (input.upper_liquid_water_kg_m2 + input.upper_ice_water_kg_m2) / input.time_step_seconds;
    let water_limited_evaporation =
        (input.corrected_ground_evaporation_kg_m2_s - maximum_removal).max(0.0);
    let ground_evaporation = input
        .corrected_ground_evaporation_kg_m2_s
        .min(maximum_removal);
    let (evaporation, sublimation, dew, frost) = if ground_evaporation >= 0.0 {
        let evaporation =
            (input.upper_liquid_water_kg_m2 / input.time_step_seconds).min(ground_evaporation);
        (evaporation, ground_evaporation - evaporation, 0.0, 0.0)
    } else if input.upper_temperature_k < FREEZING_K {
        (0.0, 0.0, 0.0, ground_evaporation.abs())
    } else {
        (0.0, 0.0, ground_evaporation.abs(), 0.0)
    };

    Ok(ThermalWaterFluxes {
        ground_evaporation_kg_m2_s: ground_evaporation,
        evaporation_kg_m2_s: evaporation,
        sublimation_kg_m2_s: sublimation,
        dew_kg_m2_s: dew,
        frost_kg_m2_s: frost,
        water_limited_evaporation_kg_m2_s: water_limited_evaporation,
        sensible_heat_correction_w_m2: input.ground_latent_heat_j_kg * water_limited_evaporation,
    })
}

/// Ports the `DEF_SPLIT_SOILSNOW` water-limit and phase-partition block in
/// `MOD_Thermal` after `GroundTemperature`（`MOD_Thermal.F90:1398-1455`）。
///
/// 逐句照上游的顺序与收缩（GIMPLE）：
/// * 有雪层：雪面 `egidif = max(0, fevpg_snow-egsmax)`、`fevpg_snow = min(…)`、
///   `fseng_snow = .FMA (egidif, htvp, fseng_snow)`；
/// * 无雪层：`fevpg_soil = .FMA (fevpg_soil, 1-fsno, fevpg_snow*fsno)`；
/// * 土面同样限水，`fseng_soil = .FMA (egidif, htvp, fseng_soil)`；
/// * 合成：有雪层 `.FMA (土面, 1-fsno, 雪面*fsno)`，无雪层直接取土面（雪面清零）；
/// * 分相：雪面 `min(wliq(lb)/deltim, fevpg_snow)` 等乘 `fsno`，土面按 `t_soisno(1)`，
///   有雪层时再乘 `1-fsno`。
pub fn partition_split_thermal_water(
    input: SplitThermalWaterInput,
) -> Result<SplitThermalWaterFluxes> {
    validate_split(input)?;
    let dt = input.time_step_seconds;
    let htvp = input.ground_latent_heat_j_kg;
    let fsno = input.snow_cover_fraction;
    let mut fseng_soil = input.corrected_soil_sensible_heat_w_m2;
    let mut fseng_snow = input.corrected_snow_sensible_heat_w_m2;
    let mut fevpg_soil = input.corrected_soil_evaporation_kg_m2_s;
    let mut fevpg_snow = input.corrected_snow_evaporation_kg_m2_s;

    if input.snow_layer_exists {
        let egsmax = (input.snow_ice_water_kg_m2 + input.snow_liquid_water_kg_m2) / dt;
        let egidif = (fevpg_snow - egsmax).max(0.0);
        fevpg_snow = fevpg_snow.min(egsmax);
        fseng_snow = egidif.mul_add(htvp, fseng_snow);
    } else {
        fevpg_soil = fevpg_soil.mul_add(1.0 - fsno, fevpg_snow * fsno);
    }
    let egsmax = (input.soil_ice_water_kg_m2 + input.soil_liquid_water_kg_m2) / dt;
    let egidif = (fevpg_soil - egsmax).max(0.0);
    fevpg_soil = fevpg_soil.min(egsmax);
    fseng_soil = egidif.mul_add(htvp, fseng_soil);

    let (fseng, fevpg) = if input.snow_layer_exists {
        (
            fseng_soil.mul_add(1.0 - fsno, fseng_snow * fsno),
            fevpg_soil.mul_add(1.0 - fsno, fevpg_snow * fsno),
        )
    } else {
        fevpg_snow = 0.0;
        (fseng_soil, fevpg_soil)
    };

    let mut snow = ThermalWaterFluxes {
        ground_evaporation_kg_m2_s: fevpg_snow,
        ..ThermalWaterFluxes::default()
    };
    if fevpg_snow >= 0.0 {
        let evaporation = (input.snow_liquid_water_kg_m2 / dt).min(fevpg_snow);
        snow.evaporation_kg_m2_s = evaporation * fsno;
        snow.sublimation_kg_m2_s = (fevpg_snow - evaporation) * fsno;
    } else if input.snow_temperature_k < FREEZING_K {
        snow.frost_kg_m2_s = (fevpg_snow * fsno).abs();
    } else {
        snow.dew_kg_m2_s = (fevpg_snow * fsno).abs();
    }

    let mut soil = ThermalWaterFluxes {
        ground_evaporation_kg_m2_s: fevpg_soil,
        ..ThermalWaterFluxes::default()
    };
    if fevpg_soil >= 0.0 {
        let evaporation = (input.soil_liquid_water_kg_m2 / dt).min(fevpg_soil);
        soil.evaporation_kg_m2_s = evaporation;
        soil.sublimation_kg_m2_s = fevpg_soil - evaporation;
    } else if input.soil_temperature_k < FREEZING_K {
        soil.frost_kg_m2_s = fevpg_soil.abs();
    } else {
        soil.dew_kg_m2_s = fevpg_soil.abs();
    }
    if input.snow_layer_exists {
        let uncovered = 1.0 - fsno;
        soil.evaporation_kg_m2_s *= uncovered;
        soil.sublimation_kg_m2_s *= uncovered;
        soil.frost_kg_m2_s *= uncovered;
        soil.dew_kg_m2_s *= uncovered;
    }
    Ok(SplitThermalWaterFluxes {
        ground_sensible_heat_w_m2: fseng,
        ground_evaporation_kg_m2_s: fevpg,
        soil,
        snow,
    })
}
fn validate(input: ThermalWaterInput) -> Result<()> {
    ensure!(
        [
            input.corrected_ground_evaporation_kg_m2_s,
            input.upper_liquid_water_kg_m2,
            input.upper_ice_water_kg_m2,
            input.upper_temperature_k,
            input.time_step_seconds,
            input.ground_latent_heat_j_kg,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.upper_liquid_water_kg_m2 >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
            && input.upper_ice_water_kg_m2 >= 0.0
            && input.time_step_seconds > 0.0
            && input.ground_latent_heat_j_kg >= 0.0,
        "thermal-water inputs are invalid"
    );
    Ok(())
}

fn validate_split(input: SplitThermalWaterInput) -> Result<()> {
    ensure!(
        [
            input.snow_cover_fraction,
            input.corrected_soil_sensible_heat_w_m2,
            input.corrected_snow_sensible_heat_w_m2,
            input.corrected_soil_evaporation_kg_m2_s,
            input.corrected_snow_evaporation_kg_m2_s,
            input.soil_liquid_water_kg_m2,
            input.soil_ice_water_kg_m2,
            input.soil_temperature_k,
            input.snow_liquid_water_kg_m2,
            input.snow_ice_water_kg_m2,
            input.snow_temperature_k,
            input.time_step_seconds,
            input.ground_latent_heat_j_kg,
        ]
        .iter()
        .all(|value| value.is_finite())
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.soil_liquid_water_kg_m2 >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
            && input.soil_ice_water_kg_m2 >= 0.0
            && input.snow_liquid_water_kg_m2 >= 0.0
            && input.snow_ice_water_kg_m2 >= 0.0,
        "split thermal-water inputs are invalid"
    );
    validate(ThermalWaterInput {
        corrected_ground_evaporation_kg_m2_s: input.corrected_soil_evaporation_kg_m2_s,
        upper_liquid_water_kg_m2: input.soil_liquid_water_kg_m2,
        upper_ice_water_kg_m2: input.soil_ice_water_kg_m2,
        upper_temperature_k: input.soil_temperature_k,
        time_step_seconds: input.time_step_seconds,
        ground_latent_heat_j_kg: input.ground_latent_heat_j_kg,
    })
}

#[cfg(test)]
#[path = "thermal_water_tests.rs"]
mod thermal_water_tests;
