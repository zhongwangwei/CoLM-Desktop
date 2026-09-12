//! Glacier water update from `MOD_Glacier.F90`.

use anyhow::{ensure, Result};

use crate::{
    combine_snow_layers, compact_snow_layers, divide_snow_layers, snow_water, RuntimeSnowColumn,
    SnowToSoilTransfer, SnowWaterInput,
};

/// Liquid and ice water held at the glacier surface node (Fortran index `1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlacierSurfaceWater {
    pub liquid_water_kg_m2: f64,
    pub ice_water_kg_m2: f64,
}

/// Inputs to `MOD_Glacier:GLACIER_WATER` after the thermal step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlacierWaterInput<'a> {
    pub time_step_seconds: f64,
    pub irreducible_saturation: f64,
    pub impermeable_porosity: f64,
    pub rainfall_kg_m2_s: f64,
    pub snow_melt_kg_m2_s: f64,
    pub evaporation_kg_m2_s: f64,
    pub dew_kg_m2_s: f64,
    pub sublimation_kg_m2_s: f64,
    pub frost_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    /// CoLM `imelt` for active snow layers, from surface to base.
    pub melted: &'a [bool],
}

/// Ports `MOD_Glacier:GLACIER_WATER` without SNICAR aerosols.
pub fn glacier_water(
    input: GlacierWaterInput<'_>,
    snow: &mut RuntimeSnowColumn,
    surface: &mut GlacierSurfaceWater,
) -> Result<f64> {
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.irreducible_saturation.is_finite()
            && (0.0..=1.0).contains(&input.irreducible_saturation)
            && input.impermeable_porosity.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.rainfall_kg_m2_s.is_finite()
            && input.snow_melt_kg_m2_s.is_finite()
            && input.evaporation_kg_m2_s.is_finite()
            && input.dew_kg_m2_s.is_finite()
            && input.sublimation_kg_m2_s.is_finite()
            && input.frost_kg_m2_s.is_finite()
            && input.eastward_wind_m_s.is_finite()
            && input.northward_wind_m_s.is_finite()
            && surface.liquid_water_kg_m2.is_finite()
            && surface.ice_water_kg_m2.is_finite(),
        "glacier-water inputs are invalid"
    );
    let had_snow = snow.layer_count < 0;
    ensure!(
        input.melted.len()
            == if had_snow {
                snow.layer_count.unsigned_abs() as usize
            } else {
                0
            },
        "glacier-water melt flags do not match the snow column"
    );
    if !had_snow {
        surface.liquid_water_kg_m2 =
            (surface.liquid_water_kg_m2 + input.dew_kg_m2_s * input.time_step_seconds).max(1.0e-8);
        surface.ice_water_kg_m2 = (surface.ice_water_kg_m2
            + (input.frost_kg_m2_s - input.sublimation_kg_m2_s) * input.time_step_seconds)
            .max(1.0e-8);
        return Ok(input.rainfall_kg_m2_s + input.snow_melt_kg_m2_s - input.evaporation_kg_m2_s);
    }

    let drainage = snow_water(
        SnowWaterInput {
            time_step_seconds: input.time_step_seconds,
            irreducible_saturation: input.irreducible_saturation,
            impermeable_porosity: input.impermeable_porosity,
            rainfall_kg_m2_s: input.rainfall_kg_m2_s,
            evaporation_kg_m2_s: input.evaporation_kg_m2_s,
            dew_kg_m2_s: input.dew_kg_m2_s,
            sublimation_kg_m2_s: input.sublimation_kg_m2_s,
            frost_kg_m2_s: input.frost_kg_m2_s,
        },
        snow,
    )?
    .bottom_drainage_kg_m2_s;
    compact_snow_layers(
        snow,
        input.time_step_seconds,
        input.eastward_wind_m_s,
        input.northward_wind_m_s,
        input.melted,
    )?;
    let mut glacier_surface = SnowToSoilTransfer {
        liquid_water_kg_m2: surface.liquid_water_kg_m2,
        ice_water_kg_m2: surface.ice_water_kg_m2,
    };
    combine_snow_layers(snow, &mut glacier_surface)?;
    surface.liquid_water_kg_m2 = glacier_surface.liquid_water_kg_m2;
    surface.ice_water_kg_m2 = glacier_surface.ice_water_kg_m2;
    if snow.layer_count < 0 {
        divide_snow_layers(snow)?;
    }
    Ok(drainage)
}

#[cfg(test)]
#[path = "glacier_tests.rs"]
mod glacier_tests;
