//! The regular-soil `WATER_2014` orchestration used by CoLMMAIN.
//!
//! This is the non-snow, non-irrigated `patchtype = 0|1` branch.  It connects
//! the already ported runoff, Richards, and groundwater kernels without
//! reimplementing their equations in a runtime driver.

use anyhow::{ensure, Result};

use crate::{
    simple_vic_runoff, solve_campbell_soil_water, topmodel_surface_runoff, update_groundwater,
    update_groundwater_topmodel, xinanjiang_runoff, CampbellSoilWaterInput, GroundwaterInput,
    StorageRunoffInput, TopmodelMethod, TopmodelSubsurfaceInput,
};

const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_DENSITY_KG_M3: f64 = 1000.0;

/// The active non-VSF runoff branch in `WATER_2014`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Water2014Runoff {
    Topmodel {
        saturated_fraction_max: f64,
        saturated_fraction_decay_m_inv: f64,
        decay_tuning: f64,
        subsurface_method: TopmodelMethod,
    },
    XinAnJiang {
        elevation_standard_deviation_m: f64,
    },
    SimpleVic {
        bvic: f64,
    },
}

/// Ground fluxes handed from `THERMAL` to the no-snow `WATER_2014` branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Water2014SoilFluxes {
    pub ground_rain_kg_m2_s: f64,
    pub snowmelt_kg_m2_s: f64,
    pub ground_evaporation_kg_m2_s: f64,
    pub transpiration_kg_m2_s: f64,
    pub soil_dew_kg_m2_s: f64,
    pub soil_frost_kg_m2_s: f64,
    pub soil_sublimation_kg_m2_s: f64,
}

/// Immutable regular-soil inputs to one `WATER_2014` call.
#[derive(Debug, Clone, Copy)]
pub struct Water2014SoilInput<'a> {
    pub patch_type: i32,
    pub urban_run: bool,
    pub plant_hydraulics: bool,
    pub time_step_seconds: f64,
    pub impermeable_porosity: f64,
    pub ponding_limit_mm: f64,
    pub minimum_soil_potential_mm: f64,
    pub soil_ice_impedance: f64,
    pub runoff: Water2014Runoff,
    pub fluxes: Water2014SoilFluxes,
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub root_fraction: &'a [f64],
    pub root_flux_mm_s: &'a [f64],
}

/// Persistent regular-soil water state shared by every native time step.
#[derive(Debug, Clone, PartialEq)]
pub struct Water2014SoilState {
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub water_table_depth_m: f64,
    pub aquifer_water_mm: f64,
    pub surface_water_mm: f64,
}

/// Diagnostics from one no-snow regular-soil `WATER_2014` call.
#[derive(Debug, Clone, PartialEq)]
pub struct Water2014SoilOutput {
    pub water_input_mm_s: f64,
    pub infiltration_mm_s: f64,
    pub surface_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub total_runoff_mm_s: f64,
    pub saturated_fraction: f64,
    pub recharge_mm_s: f64,
    pub soil_interface_flux_mm_s: Vec<f64>,
    pub root_uptake_mm_s: Vec<f64>,
    pub root_uptake_amount_mm: Vec<f64>,
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
}

/// Runs the no-snow regular-soil branch of `MOD_SoilSnowHydrology:WATER_2014`.
///
/// Snow, split soil/snow, irrigation, wetland, glacier, SNICAR, and VSF are
/// separate CoLM branches and are deliberately rejected instead of approximated.
pub fn water_2014_soil_step(
    input: Water2014SoilInput<'_>,
    state: &mut Water2014SoilState,
) -> Result<Water2014SoilOutput> {
    let layers = validate(input, state)?;
    let (effective_porosity, ice_fraction, liquid_volume_fraction) = soil_volumes(input, state);
    let water_input_mm_s = input.fluxes.ground_rain_kg_m2_s + input.fluxes.snowmelt_kg_m2_s
        - input.fluxes.ground_evaporation_kg_m2_s;
    let (surface_runoff_mm_s, initial_subsurface_runoff_mm_s, saturated_fraction) = runoff(
        input,
        state,
        &effective_porosity,
        &ice_fraction,
        &liquid_volume_fraction,
    )?;
    let infiltration_mm_s =
        water_input_mm_s - surface_runoff_mm_s - state.surface_water_mm / input.time_step_seconds;
    let soil = solve_campbell_soil_water(CampbellSoilWaterInput {
        patch_type: input.patch_type,
        time_step_seconds: input.time_step_seconds,
        impermeable_porosity: input.impermeable_porosity,
        minimum_potential_mm: input.minimum_soil_potential_mm,
        infiltration_mm_s,
        transpiration_mm_s: input.fluxes.transpiration_kg_m2_s,
        node_depth_m: input.node_depth_m,
        layer_thickness_m: input.layer_thickness_m,
        temperature_k: input.temperature_k,
        liquid_water: &liquid_volume_fraction,
        ice_fraction: &ice_fraction,
        effective_porosity: &effective_porosity,
        porosity: input.porosity,
        saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
        clapp_hornberger_b: input.clapp_hornberger_b,
        saturated_potential_mm: input.saturated_potential_mm,
        root_fraction: input.root_fraction,
        root_flux_mm_s: input.root_flux_mm_s,
        plant_hydraulics: input.plant_hydraulics,
        urban_run: input.urban_run,
        soil_ice_impedance: input.soil_ice_impedance,
    })?;
    for layer in 0..layers {
        state.liquid_water_kg_m2[layer] +=
            soil.liquid_water_change[layer] * input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3;
    }
    let groundwater_input = GroundwaterInput {
        time_step_seconds: input.time_step_seconds,
        ponding_limit_mm: input.ponding_limit_mm,
        effective_porosity: &effective_porosity,
        layer_thickness_m: input.layer_thickness_m,
        interface_depth_m: input.interface_depth_m,
        ice_water_kg_m2: &state.ice_water_kg_m2,
        liquid_water_kg_m2: &state.liquid_water_kg_m2,
        porosity: input.porosity,
        saturated_potential_mm: input.saturated_potential_mm,
        clapp_hornberger_b: input.clapp_hornberger_b,
        water_table_depth_m: state.water_table_depth_m,
        aquifer_water_mm: state.aquifer_water_mm,
        recharge_mm_s: soil.recharge_mm_s,
        subsurface_runoff_mm_s: initial_subsurface_runoff_mm_s,
    };
    let groundwater = match input.runoff {
        Water2014Runoff::Topmodel {
            decay_tuning,
            subsurface_method,
            ..
        } => update_groundwater_topmodel(
            groundwater_input,
            TopmodelSubsurfaceInput {
                method: subsurface_method,
                layer_thickness_m: input.layer_thickness_m,
                interface_depth_m: input.interface_depth_m,
                ice_fraction: &ice_fraction,
                saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
                decay_tuning,
                water_table_depth_m: state.water_table_depth_m,
            },
        )?,
        Water2014Runoff::XinAnJiang { .. } | Water2014Runoff::SimpleVic { .. } => {
            update_groundwater(groundwater_input)?
        }
    };
    state.liquid_water_kg_m2 = groundwater.liquid_water_kg_m2;
    state.water_table_depth_m = groundwater.water_table_depth_m;
    state.aquifer_water_mm = groundwater.aquifer_water_mm;
    state.liquid_water_kg_m2[0] = (state.liquid_water_kg_m2[0]
        + input.fluxes.soil_dew_kg_m2_s * input.time_step_seconds)
        .max(0.0);
    state.ice_water_kg_m2[0] = (state.ice_water_kg_m2[0]
        + (input.fluxes.soil_frost_kg_m2_s - input.fluxes.soil_sublimation_kg_m2_s)
            * input.time_step_seconds)
        .max(0.0);

    Ok(Water2014SoilOutput {
        water_input_mm_s,
        infiltration_mm_s,
        surface_runoff_mm_s,
        subsurface_runoff_mm_s: groundwater.subsurface_runoff_mm_s,
        total_runoff_mm_s: surface_runoff_mm_s + groundwater.subsurface_runoff_mm_s,
        saturated_fraction,
        recharge_mm_s: soil.recharge_mm_s,
        soil_interface_flux_mm_s: soil.interface_flux_mm_s,
        root_uptake_mm_s: soil.root_uptake_mm_s,
        root_uptake_amount_mm: soil.root_uptake_amount_mm,
        matric_potential_mm: soil.matric_potential_mm,
        hydraulic_conductivity_mm_s: soil.hydraulic_conductivity_mm_s,
    })
}

fn runoff(
    input: Water2014SoilInput<'_>,
    state: &Water2014SoilState,
    effective_porosity: &[f64],
    ice_fraction: &[f64],
    liquid_volume_fraction: &[f64],
) -> Result<(f64, f64, f64)> {
    let storage = StorageRunoffInput {
        layer_thickness_m: input.layer_thickness_m,
        effective_porosity,
        liquid_volume_fraction,
        water_input_mm_s: input.fluxes.ground_rain_kg_m2_s + input.fluxes.snowmelt_kg_m2_s
            - input.fluxes.ground_evaporation_kg_m2_s,
        time_step_seconds: input.time_step_seconds,
    };
    match input.runoff {
        Water2014Runoff::Topmodel {
            saturated_fraction_max,
            saturated_fraction_decay_m_inv,
            decay_tuning,
            ..
        } => {
            let runoff = topmodel_surface_runoff(crate::TopmodelSurfaceInput {
                impermeable_porosity: input.impermeable_porosity,
                saturated_hydraulic_conductivity_mm_s: input.saturated_hydraulic_conductivity_mm_s,
                effective_porosity,
                ice_fraction,
                saturated_fraction_max,
                saturated_fraction_decay_m_inv,
                decay_tuning,
                water_table_depth_m: state.water_table_depth_m,
                water_input_mm_s: storage.water_input_mm_s,
            })?;
            Ok((runoff.surface_runoff_mm_s, 0.0, runoff.saturated_fraction))
        }
        Water2014Runoff::XinAnJiang {
            elevation_standard_deviation_m,
        } => {
            let runoff = xinanjiang_runoff(storage, elevation_standard_deviation_m)?;
            Ok((
                runoff.surface_runoff_mm_s,
                runoff.subsurface_runoff_mm_s,
                runoff.saturated_fraction,
            ))
        }
        Water2014Runoff::SimpleVic { bvic } => {
            let runoff = simple_vic_runoff(storage, bvic)?;
            Ok((
                runoff.surface_runoff_mm_s,
                runoff.subsurface_runoff_mm_s,
                runoff.saturated_fraction,
            ))
        }
    }
}

fn soil_volumes(
    input: Water2014SoilInput<'_>,
    state: &Water2014SoilState,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut effective_porosity = Vec::with_capacity(input.porosity.len());
    let mut ice_fraction = Vec::with_capacity(input.porosity.len());
    let mut liquid_volume_fraction = Vec::with_capacity(input.porosity.len());
    for layer in 0..input.porosity.len() {
        let ice_volume = (state.ice_water_kg_m2[layer]
            / (input.layer_thickness_m[layer] * ICE_DENSITY_KG_M3))
            .min(input.porosity[layer]);
        let effective = (input.porosity[layer] - ice_volume).max(0.01);
        effective_porosity.push(effective);
        liquid_volume_fraction.push(
            (state.liquid_water_kg_m2[layer]
                / (input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3))
                .min(effective),
        );
        ice_fraction.push(if input.porosity[layer] < 1.0e-6 {
            0.0
        } else {
            (ice_volume / input.porosity[layer]).min(1.0)
        });
    }
    (effective_porosity, ice_fraction, liquid_volume_fraction)
}

fn validate(input: Water2014SoilInput<'_>, state: &Water2014SoilState) -> Result<usize> {
    ensure!(
        matches!(input.patch_type, 0 | 1),
        "water_2014_soil_step supports only soil and urban patches"
    );
    let layers = input.layer_thickness_m.len();
    ensure!(
        layers >= 2,
        "water_2014_soil_step needs at least two soil layers"
    );
    for values in [
        input.node_depth_m,
        input.temperature_k,
        input.porosity,
        input.residual_water,
        input.saturated_hydraulic_conductivity_mm_s,
        input.clapp_hornberger_b,
        input.saturated_potential_mm,
        input.root_fraction,
        input.root_flux_mm_s,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "water_2014_soil_step soil vectors must be finite and equally sized"
        );
    }
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input.interface_depth_m[0] == 0.0
            && input
                .interface_depth_m
                .windows(2)
                .all(|pair| pair[1].is_finite() && pair[1] > pair[0]),
        "water_2014_soil_step needs increasing soil interfaces starting at zero"
    );
    ensure!(
        state.liquid_water_kg_m2.len() == layers
            && state.ice_water_kg_m2.len() == layers
            && state
                .liquid_water_kg_m2
                .iter()
                .chain(&state.ice_water_kg_m2)
                .all(|value| value.is_finite() && *value >= 0.0),
        "water_2014_soil_step state layers are invalid"
    );
    ensure!(
        [
            input.time_step_seconds,
            input.impermeable_porosity,
            input.ponding_limit_mm,
            input.minimum_soil_potential_mm,
            input.soil_ice_impedance,
            state.water_table_depth_m,
            state.aquifer_water_mm,
            state.surface_water_mm,
            input.fluxes.ground_rain_kg_m2_s,
            input.fluxes.snowmelt_kg_m2_s,
            input.fluxes.ground_evaporation_kg_m2_s,
            input.fluxes.transpiration_kg_m2_s,
            input.fluxes.soil_dew_kg_m2_s,
            input.fluxes.soil_frost_kg_m2_s,
            input.fluxes.soil_sublimation_kg_m2_s,
        ]
        .iter()
        .all(|value| value.is_finite()),
        "water_2014_soil_step scalars must be finite"
    );
    ensure!(
        input.time_step_seconds > 0.0
            && input.impermeable_porosity >= 0.0
            && input.ponding_limit_mm >= 0.0
            && input.minimum_soil_potential_mm < 0.0
            && input.soil_ice_impedance > 0.0
            && state.water_table_depth_m >= 0.0
            && state.surface_water_mm >= 0.0,
        "water_2014_soil_step scalar bounds are invalid"
    );
    for layer in 0..layers {
        ensure!(
            input.layer_thickness_m[layer] > 0.0
                && input.porosity[layer] >= 0.01
                && input.residual_water[layer] >= 0.0
                && input.residual_water[layer] <= input.porosity[layer]
                && input.saturated_hydraulic_conductivity_mm_s[layer] >= 0.0
                && input.clapp_hornberger_b[layer] > 0.0
                && input.saturated_potential_mm[layer] < 0.0,
            "water_2014_soil_step soil layer is invalid"
        );
    }
    Ok(layers)
}

#[cfg(test)]
#[path = "water_2014_tests.rs"]
mod tests;
