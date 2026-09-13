//! Roof and impervious-road water routing from `MOD_Urban_Hydrology.F90`.
//!
//! This owns only the source module's common sealed-surface section. Pervious
//! soil and lake routing already have distinct shared solvers and remain their
//! own callers.

use anyhow::{ensure, Result};

use crate::{snow_water, RuntimeSnowColumn, SnowWaterInput};

/// Hydrologic forcing and surface fluxes for one roof or impervious road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanSealedHydrologyInput {
    pub time_step_seconds: f64,
    pub irreducible_saturation: f64,
    pub impermeable_porosity: f64,
    pub rainfall_kg_m2_s: f64,
    pub snow_melt_kg_m2_s: f64,
    pub surface_evaporation_kg_m2_s: f64,
    pub dew_kg_m2_s: f64,
    pub sublimation_kg_m2_s: f64,
    pub frost_kg_m2_s: f64,
}

/// Mutable top substrate beneath a sealed surface's optional snow column.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanSealedSurfaceState {
    /// Explicit snow layers (`lbr/lbi < 1`), if present.
    pub snow: Option<RuntimeSnowColumn>,
    /// Liquid water in the first roof/road substrate layer.
    pub substrate_liquid_water_kg_m2: f64,
    /// Ice in the first roof/road substrate layer.
    pub substrate_ice_water_kg_m2: f64,
}

/// Water routed through snow and excess liquid runoff from a sealed surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanSealedHydrologyState {
    pub incoming_water_kg_m2_s: f64,
    pub surface_runoff_kg_m2_s: f64,
    pub total_runoff_kg_m2_s: f64,
}

/// Ports section 2 of `MOD_Urban_Hydrology:UrbanHydrology`.
pub fn urban_sealed_hydrology(
    input: UrbanSealedHydrologyInput,
    state: &mut UrbanSealedSurfaceState,
) -> Result<UrbanSealedHydrologyState> {
    validate(input, state)?;
    let has_snow = state.snow.is_some();
    let incoming_water_kg_m2_s = if let Some(snow) = state.snow.as_mut() {
        snow_water(
            SnowWaterInput {
                time_step_seconds: input.time_step_seconds,
                irreducible_saturation: input.irreducible_saturation,
                impermeable_porosity: input.impermeable_porosity,
                rainfall_kg_m2_s: input.rainfall_kg_m2_s,
                evaporation_kg_m2_s: input.surface_evaporation_kg_m2_s,
                dew_kg_m2_s: input.dew_kg_m2_s,
                sublimation_kg_m2_s: input.sublimation_kg_m2_s,
                frost_kg_m2_s: input.frost_kg_m2_s,
            },
            snow,
        )?
        .bottom_drainage_kg_m2_s
    } else {
        input.rainfall_kg_m2_s + input.snow_melt_kg_m2_s - input.surface_evaporation_kg_m2_s
    };

    state.substrate_liquid_water_kg_m2 += incoming_water_kg_m2_s * input.time_step_seconds;
    if !has_snow {
        state.substrate_liquid_water_kg_m2 = (state.substrate_liquid_water_kg_m2
            + input.dew_kg_m2_s * input.time_step_seconds)
            .max(0.0);
        state.substrate_ice_water_kg_m2 = (state.substrate_ice_water_kg_m2
            + (input.frost_kg_m2_s - input.sublimation_kg_m2_s) * input.time_step_seconds)
            .max(0.0);
    }

    let excess_liquid_kg_m2 = (state.substrate_liquid_water_kg_m2 - 1.0).max(0.0);
    state.substrate_liquid_water_kg_m2 = state.substrate_liquid_water_kg_m2.min(1.0);
    let surface_runoff_kg_m2_s = excess_liquid_kg_m2 / input.time_step_seconds;
    Ok(UrbanSealedHydrologyState {
        incoming_water_kg_m2_s,
        surface_runoff_kg_m2_s,
        total_runoff_kg_m2_s: surface_runoff_kg_m2_s,
    })
}

fn validate(input: UrbanSealedHydrologyInput, state: &UrbanSealedSurfaceState) -> Result<()> {
    ensure!(
        [
            input.time_step_seconds,
            input.irreducible_saturation,
            input.impermeable_porosity,
            input.rainfall_kg_m2_s,
            input.snow_melt_kg_m2_s,
            input.surface_evaporation_kg_m2_s,
            input.dew_kg_m2_s,
            input.sublimation_kg_m2_s,
            input.frost_kg_m2_s,
            state.substrate_liquid_water_kg_m2,
            state.substrate_ice_water_kg_m2,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && (0.0..=1.0).contains(&input.irreducible_saturation)
            && input.impermeable_porosity >= 0.0
            && state.substrate_liquid_water_kg_m2 >= 0.0
            && state.substrate_ice_water_kg_m2 >= 0.0,
        "urban sealed-hydrology inputs are invalid"
    );
    if let Some(snow) = &state.snow {
        ensure!(
            snow.layer_count < 0,
            "urban sealed-hydrology snow state needs active snow layers"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "urban_sealed_hydrology_tests.rs"]
mod urban_sealed_hydrology_tests;
