//! Runtime lake-column adjustment from `MOD_Lake.F90`.

use anyhow::{ensure, Result};

use crate::snow::{snow_interface_slot, snow_layer_slot, validate_runtime_snow_column};
use crate::{
    combine_snow_layers, compact_snow_layers, divide_snow_layers, snow_water, RuntimeSnowColumn,
    SnowToSoilTransfer, SnowWaterInput,
};

const LAKE_LAYERS: usize = 10;
const DEFAULT_THICKNESS_M: [f64; LAKE_LAYERS] =
    [0.1, 1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 7.0, 10.45, 10.45];
const FREEZING_K: f64 = 273.16;
const LIQUID_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const FUSION_HEAT_J_KG: f64 = 0.3336e6;

/// Mutable ten-layer lake state in top-to-bottom order.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeColumn {
    pub thickness_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    /// Frozen mass fraction of each lake layer.
    pub ice_fraction: Vec<f64>,
}

/// Inputs to `MOD_Lake:newsnow_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeNewSnowInput {
    pub use_dynamic_lake: bool,
    pub time_step_seconds: f64,
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub new_snow_bulk_density_kg_m3: f64,
}

/// Precipitation left after lake-surface phase change.
///
/// This is the mutated `pg_rain`/`pg_snow` pair from `newsnow_lake`; callers
/// pass it on to CoLM's later lake snow-water step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeNewSnowOutcome {
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
}

/// Mutable soil state below a lake used by `MOD_Lake:snowwater_lake`.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeSnowWaterSoil {
    pub thickness_m: Vec<f64>,
    pub porosity: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
}

/// In/out surface fluxes used by `MOD_Lake:snowwater_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSnowWaterFluxes {
    pub sensible_heat_w_m2: f64,
    pub ground_heat_w_m2: f64,
    pub snow_melt_kg_m2_s: f64,
}

/// Inputs to `MOD_Lake:snowwater_lake` after `newsnow_lake` has partitioned precipitation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSnowWaterInput<'a> {
    pub use_dynamic_lake: bool,
    pub time_step_seconds: f64,
    pub irreducible_saturation: f64,
    pub impermeable_porosity: f64,
    pub rainfall_kg_m2_s: f64,
    pub evaporation_kg_m2_s: f64,
    pub sublimation_kg_m2_s: f64,
    pub dew_kg_m2_s: f64,
    pub frost_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    /// CoLM `imelt` over the currently active snow layers, surface first.
    pub melted: &'a [bool],
}

/// Water drained from the snow bottom during the lake hydrology step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSnowWaterOutcome {
    pub bottom_drainage_kg_m2_s: f64,
}

/// Inputs to `MOD_Lake:roughness_lake` for one lake surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeRoughnessInput {
    /// CoLM's signed snow-layer count (`0`, `-1`, …).
    pub snow_layer_count: i32,
    pub ground_temperature_k: f64,
    pub lake_surface_temperature_k: f64,
    pub surface_pressure_pa: f64,
    pub charnock_parameter: f64,
    pub friction_velocity_m_s: f64,
}

/// Lake momentum, sensible-heat, and latent-heat roughness lengths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeRoughness {
    pub momentum_m: f64,
    pub sensible_heat_m: f64,
    pub latent_heat_m: f64,
}

/// Inputs to `MOD_Lake:hConductivity_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeConductivityInput<'a> {
    pub snow_layer_count: i32,
    pub ground_temperature_k: f64,
    /// Lake-node depths (m), top to bottom.
    pub node_depth_m: &'a [f64],
    pub latitude_radians: f64,
    pub friction_velocity_m_s: f64,
    pub momentum_roughness_m: f64,
    /// `lakedepth`, retained separately from the mutable layer geometry.
    pub lake_depth_m: f64,
    pub deep_lake_threshold_m: f64,
}

/// Effective thermal conductivity of every lake layer and top eddy term.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeConductivity {
    pub thermal_conductivity_w_m_k: Vec<f64>,
    pub top_eddy_conductivity_w_m_k: f64,
}

/// Ports `MOD_Lake:newsnow_lake`.
///
/// The snow state is the shared CoLM snow prefix used by land patches.  It
/// keeps the initializer and runtime on one snow-column representation while
/// preserving the lake-specific rain/snow and surface-ice energy exchange.
pub fn add_lake_new_snow(
    input: LakeNewSnowInput,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
) -> Result<LakeNewSnowOutcome> {
    validate(lake)?;
    validate_runtime_snow_column(snow)?;
    ensure!(
        lake.thickness_m[0] > 0.0,
        "lake surface layer must have positive thickness"
    );
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.rainfall_kg_m2_s.is_finite()
            && input.rainfall_kg_m2_s >= 0.0
            && input.snowfall_kg_m2_s.is_finite()
            && input.snowfall_kg_m2_s >= 0.0
            && input.precipitation_temperature_k.is_finite()
            && input.new_snow_bulk_density_kg_m3.is_finite()
            && input.new_snow_bulk_density_kg_m3 > 0.0,
        "lake new-snow inputs are invalid"
    );

    let mut rainfall = input.rainfall_kg_m2_s;
    let mut snowfall = input.snowfall_kg_m2_s;
    let snowfall_depth_rate = snowfall / input.new_snow_bulk_density_kg_m3;
    snow.depth_m += snowfall_depth_rate * input.time_step_seconds;
    snow.water_equivalent_kg_m2 += snowfall * input.time_step_seconds;
    snow.interface_depth_m[snow_interface_slot(0)] = 0.0;
    let mut new_node = false;

    if snow.layer_count == 0 && snow.depth_m < 0.01 {
        snow.age = 0.0;
        exchange_precipitation_with_lake(
            input.time_step_seconds,
            input.precipitation_temperature_k,
            input.new_snow_bulk_density_kg_m3,
            &mut rainfall,
            &mut snowfall,
            snow,
            lake,
        );
        if snow.depth_m >= 0.01 {
            new_lake_snow_node(snow, lake.temperature_k[0]);
            new_node = true;
        }
        if input.use_dynamic_lake && snow.layer_count == 0 {
            let liquid_depth = lake.thickness_m[0] * (1.0 - lake.ice_fraction[0])
                + rainfall * input.time_step_seconds * 1.0e-3;
            let ice_depth = lake.thickness_m[0] * lake.ice_fraction[0];
            lake.thickness_m[0] = liquid_depth + ice_depth;
            lake.ice_fraction[0] = ice_depth / lake.thickness_m[0];
            adjust_lake_layers(lake)?;
        }
    } else if snow.layer_count == 0 {
        new_lake_snow_node(snow, FREEZING_K.min(input.precipitation_temperature_k));
        new_node = true;
    }

    if snow.layer_count < 0 && !new_node {
        let top_layer = snow.layer_count + 1;
        let top = snow_layer_slot(top_layer);
        let old_heat_capacity = snow.ice_water_kg_m2[top] * ICE_HEAT_CAPACITY_J_KG_K
            + snow.liquid_water_kg_m2[top] * LIQUID_HEAT_CAPACITY_J_KG_K;
        let precipitation_heat_capacity = input.time_step_seconds
            * (rainfall * LIQUID_HEAT_CAPACITY_J_KG_K + snowfall * ICE_HEAT_CAPACITY_J_KG_K);
        ensure!(
            old_heat_capacity + precipitation_heat_capacity > 0.0,
            "lake snow top layer has no heat capacity"
        );
        snow.temperature_k[top] = ((old_heat_capacity * snow.temperature_k[top])
            + precipitation_heat_capacity * input.precipitation_temperature_k)
            / (old_heat_capacity + precipitation_heat_capacity);
        snow.temperature_k[top] = snow.temperature_k[top].min(FREEZING_K);
        snow.ice_water_kg_m2[top] += input.time_step_seconds * snowfall;
        snow.thickness_m[top] += snowfall_depth_rate * input.time_step_seconds;
        snow.node_depth_m[top] =
            snow.interface_depth_m[snow_interface_slot(top_layer)] - 0.5 * snow.thickness_m[top];
        snow.interface_depth_m[snow_interface_slot(top_layer - 1)] =
            snow.interface_depth_m[snow_interface_slot(top_layer)] - snow.thickness_m[top];
    }

    Ok(LakeNewSnowOutcome {
        rainfall_kg_m2_s: rainfall,
        snowfall_kg_m2_s: snowfall,
    })
}

fn new_lake_snow_node(snow: &mut RuntimeSnowColumn, temperature_k: f64) {
    snow.layer_count = -1;
    let top = snow_layer_slot(0);
    snow.thickness_m[top] = snow.depth_m;
    snow.node_depth_m[top] = -0.5 * snow.thickness_m[top];
    snow.interface_depth_m[snow_interface_slot(-1)] = -snow.thickness_m[top];
    snow.age = 0.0;
    snow.temperature_k[top] = temperature_k;
    snow.ice_water_kg_m2[top] = snow.water_equivalent_kg_m2;
    snow.liquid_water_kg_m2[top] = 0.0;
    snow.previous_ice_fraction[top] = 1.0;
}

fn exchange_precipitation_with_lake(
    time_step_seconds: f64,
    precipitation_temperature_k: f64,
    new_snow_bulk_density_kg_m3: f64,
    rainfall: &mut f64,
    snowfall: &mut f64,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
) {
    let a = LIQUID_HEAT_CAPACITY_J_KG_K
        * *rainfall
        * time_step_seconds
        * (precipitation_temperature_k - FREEZING_K);
    let b = *rainfall * time_step_seconds * FUSION_HEAT_J_KG;
    let c = ICE_HEAT_CAPACITY_J_KG_K
        * 1000.0
        * lake.thickness_m[0]
        * lake.ice_fraction[0]
        * (FREEZING_K - lake.temperature_k[0]);
    let d = 1000.0 * lake.thickness_m[0] * lake.ice_fraction[0] * FUSION_HEAT_J_KG;
    let e = ICE_HEAT_CAPACITY_J_KG_K
        * *snowfall
        * time_step_seconds
        * (FREEZING_K - precipitation_temperature_k);
    let f = *snowfall * time_step_seconds * FUSION_HEAT_J_KG;
    let g = LIQUID_HEAT_CAPACITY_J_KG_K
        * 1000.0
        * lake.thickness_m[0]
        * (1.0 - lake.ice_fraction[0])
        * (lake.temperature_k[0] - FREEZING_K);
    let h = 1000.0 * lake.thickness_m[0] * (1.0 - lake.ice_fraction[0]) * FUSION_HEAT_J_KG;

    if lake.ice_fraction[0] > 0.999 {
        if a + b <= c {
            let precipitation_temperature = FREEZING_K.min(precipitation_temperature_k);
            lake.temperature_k[0] = (a
                + b
                + ICE_HEAT_CAPACITY_J_KG_K
                    * (*rainfall + *snowfall)
                    * time_step_seconds
                    * precipitation_temperature
                + ICE_HEAT_CAPACITY_J_KG_K
                    * 1000.0
                    * lake.thickness_m[0]
                    * lake.temperature_k[0]
                    * lake.ice_fraction[0])
                / (ICE_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0] * lake.ice_fraction[0]
                    + ICE_HEAT_CAPACITY_J_KG_K * (*rainfall + *snowfall) * time_step_seconds);
            snow.water_equivalent_kg_m2 += *rainfall * time_step_seconds;
            snow.depth_m += *rainfall * time_step_seconds / new_snow_bulk_density_kg_m3;
            *snowfall += *rainfall;
            *rainfall = 0.0;
        } else if a <= c {
            lake.temperature_k[0] = FREEZING_K;
            let frozen_rain = (c - a) / FUSION_HEAT_J_KG;
            snow.water_equivalent_kg_m2 += frozen_rain;
            snow.depth_m += frozen_rain / new_snow_bulk_density_kg_m3;
            *snowfall += (*rainfall).min(frozen_rain / time_step_seconds);
            *rainfall = 0.0_f64.max(*rainfall - frozen_rain / time_step_seconds);
        } else if a <= c + d {
            lake.temperature_k[0] = FREEZING_K;
            let ice_mass = 1000.0 * lake.thickness_m[0] - (a - c) / FUSION_HEAT_J_KG;
            lake.ice_fraction[0] = ice_mass / (ice_mass + (a - c) / FUSION_HEAT_J_KG);
        } else {
            lake.temperature_k[0] = (LIQUID_HEAT_CAPACITY_J_KG_K
                * *rainfall
                * time_step_seconds
                * precipitation_temperature_k
                + LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0] * FREEZING_K
                - c
                - d)
                / (LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0]
                    + LIQUID_HEAT_CAPACITY_J_KG_K * *rainfall * time_step_seconds);
            lake.ice_fraction[0] = 0.0;
        }
    } else if lake.ice_fraction[0] >= 0.001 {
        if *rainfall > 0.0 && *snowfall > 0.0 {
            lake.temperature_k[0] = FREEZING_K;
        } else if *rainfall > 0.0 {
            if a >= d {
                lake.temperature_k[0] = (LIQUID_HEAT_CAPACITY_J_KG_K
                    * *rainfall
                    * time_step_seconds
                    * precipitation_temperature_k
                    + LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0] * FREEZING_K
                    - d)
                    / (LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0]
                        + LIQUID_HEAT_CAPACITY_J_KG_K * *rainfall * time_step_seconds);
                lake.ice_fraction[0] = 0.0;
            } else {
                lake.temperature_k[0] = FREEZING_K;
                let ice_mass =
                    1000.0 * lake.thickness_m[0] * lake.ice_fraction[0] - a / FUSION_HEAT_J_KG;
                let liquid_mass = 1000.0 * lake.thickness_m[0] * (1.0 - lake.ice_fraction[0])
                    + a / FUSION_HEAT_J_KG;
                lake.ice_fraction[0] = ice_mass / (ice_mass + liquid_mass);
            }
        } else if *snowfall > 0.0 {
            if e >= h {
                lake.temperature_k[0] = (h
                    + ICE_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0] * FREEZING_K
                    + ICE_HEAT_CAPACITY_J_KG_K
                        * *snowfall
                        * time_step_seconds
                        * precipitation_temperature_k)
                    / (ICE_HEAT_CAPACITY_J_KG_K * *snowfall * time_step_seconds
                        + ICE_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0]);
                lake.ice_fraction[0] = 1.0;
            } else {
                lake.temperature_k[0] = FREEZING_K;
                let ice_mass =
                    1000.0 * lake.thickness_m[0] * lake.ice_fraction[0] + e / FUSION_HEAT_J_KG;
                let liquid_mass = 1000.0 * lake.thickness_m[0] * (1.0 - lake.ice_fraction[0])
                    - e / FUSION_HEAT_J_KG;
                lake.ice_fraction[0] = ice_mass / (ice_mass + liquid_mass);
            }
        }
    } else if e + f <= g {
        let precipitation_temperature = FREEZING_K.max(precipitation_temperature_k);
        lake.temperature_k[0] = (LIQUID_HEAT_CAPACITY_J_KG_K
            * 1000.0
            * lake.thickness_m[0]
            * lake.temperature_k[0]
            * (1.0 - lake.ice_fraction[0])
            + LIQUID_HEAT_CAPACITY_J_KG_K
                * (*rainfall + *snowfall)
                * time_step_seconds
                * precipitation_temperature
            - e
            - f)
            / (LIQUID_HEAT_CAPACITY_J_KG_K * (*rainfall + *snowfall) * time_step_seconds
                + LIQUID_HEAT_CAPACITY_J_KG_K
                    * 1000.0
                    * lake.thickness_m[0]
                    * (1.0 - lake.ice_fraction[0]));
        snow.water_equivalent_kg_m2 -= *snowfall * time_step_seconds;
        snow.depth_m -= *snowfall * time_step_seconds / new_snow_bulk_density_kg_m3;
        *rainfall += *snowfall;
        *snowfall = 0.0;
    } else if e <= g {
        lake.temperature_k[0] = FREEZING_K;
        let melted_snow = (g - e) / FUSION_HEAT_J_KG;
        snow.water_equivalent_kg_m2 -= melted_snow;
        snow.depth_m -= melted_snow / new_snow_bulk_density_kg_m3;
        *rainfall += (*snowfall).min(melted_snow / time_step_seconds);
        *snowfall = 0.0_f64.max(*snowfall - melted_snow / time_step_seconds);
    } else if e <= g + h {
        lake.temperature_k[0] = FREEZING_K;
        let ice_mass = (e - g) / FUSION_HEAT_J_KG;
        lake.ice_fraction[0] = ice_mass / (ice_mass + 1000.0 * lake.thickness_m[0] - ice_mass);
    } else {
        lake.temperature_k[0] = (g
            + h
            + ICE_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0] * FREEZING_K
            + ICE_HEAT_CAPACITY_J_KG_K
                * *snowfall
                * time_step_seconds
                * precipitation_temperature_k)
            / (ICE_HEAT_CAPACITY_J_KG_K * *snowfall * time_step_seconds
                + ICE_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0]);
        lake.ice_fraction[0] = 1.0;
    }
}

/// Ports `MOD_Lake:snowwater_lake` without SNICAR aerosols.
///
/// It reuses the common snow percolation, compaction, combining, and division
/// kernels, then applies only the lake-specific melting and saturated-bed rules.
pub fn lake_snow_water(
    input: LakeSnowWaterInput<'_>,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
    soil: &mut LakeSnowWaterSoil,
    fluxes: &mut LakeSnowWaterFluxes,
) -> Result<LakeSnowWaterOutcome> {
    validate(lake)?;
    validate_lake_soil(soil)?;
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.irreducible_saturation.is_finite()
            && (0.0..=1.0).contains(&input.irreducible_saturation)
            && input.impermeable_porosity.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.rainfall_kg_m2_s.is_finite()
            && input.evaporation_kg_m2_s.is_finite()
            && input.sublimation_kg_m2_s.is_finite()
            && input.dew_kg_m2_s.is_finite()
            && input.frost_kg_m2_s.is_finite()
            && input.eastward_wind_m_s.is_finite()
            && input.northward_wind_m_s.is_finite()
            && fluxes.sensible_heat_w_m2.is_finite()
            && fluxes.ground_heat_w_m2.is_finite()
            && fluxes.snow_melt_kg_m2_s.is_finite(),
        "lake snow-water inputs are invalid"
    );
    let had_snow = snow.layer_count < 0;
    ensure!(
        input.melted.len()
            == if had_snow {
                snow.layer_count.unsigned_abs() as usize
            } else {
                0
            },
        "lake snow-water melt flags do not match the snow column"
    );
    let mut bottom_drainage = 0.0;

    if had_snow {
        bottom_drainage = snow_water(
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
        let mut lake_surface = SnowToSoilTransfer {
            liquid_water_kg_m2: soil.liquid_water_kg_m2[0],
            ice_water_kg_m2: soil.ice_water_kg_m2[0],
        };
        combine_snow_layers(snow, &mut lake_surface)?;
        soil.liquid_water_kg_m2[0] = lake_surface.liquid_water_kg_m2;
        soil.ice_water_kg_m2[0] = lake_surface.ice_water_kg_m2;
        if snow.layer_count < 0 {
            divide_snow_layers(snow)?;
        }
        remove_all_liquid_snow(snow, fluxes, input.time_step_seconds, &mut bottom_drainage);
    }

    melt_snow_into_unfrozen_lake(
        snow,
        lake,
        fluxes,
        input.time_step_seconds,
        &mut bottom_drainage,
    );
    let soil_water_change = saturate_lake_soil(soil);
    if input.use_dynamic_lake {
        adjust_dynamic_lake_water(
            had_snow,
            input,
            bottom_drainage,
            soil_water_change,
            lake,
            fluxes,
        )?;
    }
    Ok(LakeSnowWaterOutcome {
        bottom_drainage_kg_m2_s: bottom_drainage,
    })
}

fn remove_all_liquid_snow(
    snow: &mut RuntimeSnowColumn,
    fluxes: &mut LakeSnowWaterFluxes,
    time_step_seconds: f64,
    bottom_drainage: &mut f64,
) {
    if snow.layer_count != -1 || snow.ice_water_kg_m2[snow_layer_slot(0)] != 0.0 {
        return;
    }
    let top = snow_layer_slot(0);
    let heat = LIQUID_HEAT_CAPACITY_J_KG_K
        * snow.liquid_water_kg_m2[top]
        * (snow.temperature_k[top] - FREEZING_K);
    fluxes.sensible_heat_w_m2 += heat / time_step_seconds;
    fluxes.ground_heat_w_m2 -= heat / time_step_seconds;
    *bottom_drainage += snow.liquid_water_kg_m2[top] / time_step_seconds;
    snow.layer_count = 0;
    snow.water_equivalent_kg_m2 = 0.0;
    snow.depth_m = 0.0;
    snow.liquid_water_kg_m2[top] = 0.0;
}

fn melt_snow_into_unfrozen_lake(
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
    fluxes: &mut LakeSnowWaterFluxes,
    time_step_seconds: f64,
    bottom_drainage: &mut f64,
) {
    if snow.layer_count >= 0 || lake.temperature_k[0] <= FREEZING_K || lake.ice_fraction[0] >= 0.001
    {
        return;
    }
    let mut ice = 0.0;
    let mut liquid = 0.0;
    let mut cooling = 0.0;
    for layer in snow.layer_count + 1..=0 {
        let slot = snow_layer_slot(layer);
        ice += snow.ice_water_kg_m2[slot];
        liquid += snow.liquid_water_kg_m2[slot];
        cooling += snow.ice_water_kg_m2[slot]
            * ICE_HEAT_CAPACITY_J_KG_K
            * (FREEZING_K - snow.temperature_k[slot])
            + snow.liquid_water_kg_m2[slot]
                * LIQUID_HEAT_CAPACITY_J_KG_K
                * (FREEZING_K - snow.temperature_k[slot]);
    }
    let melt_energy = cooling + ice * FUSION_HEAT_J_KG;
    let lake_warming = (lake.temperature_k[0] - FREEZING_K)
        * LIQUID_HEAT_CAPACITY_J_KG_K
        * 1000.0
        * lake.thickness_m[0];
    let lake_freezing = 1000.0 * lake.thickness_m[0] * FUSION_HEAT_J_KG;
    if lake_warming < melt_energy && lake_warming + lake_freezing < melt_energy {
        return;
    }
    if lake_warming >= melt_energy {
        lake.temperature_k[0] = (LIQUID_HEAT_CAPACITY_J_KG_K
            * (1000.0 * lake.thickness_m[0] * lake.temperature_k[0] + (ice + liquid) * FREEZING_K)
            - cooling
            - ice * FUSION_HEAT_J_KG)
            / (LIQUID_HEAT_CAPACITY_J_KG_K * (1000.0 * lake.thickness_m[0] + ice + liquid));
    } else {
        lake.temperature_k[0] = FREEZING_K;
        lake.ice_fraction[0] = (melt_energy - lake_warming) / lake_freezing;
    }
    fluxes.snow_melt_kg_m2_s += snow.water_equivalent_kg_m2 / time_step_seconds;
    *bottom_drainage += snow.water_equivalent_kg_m2 / time_step_seconds;
    snow.layer_count = 0;
    snow.water_equivalent_kg_m2 = 0.0;
    snow.depth_m = 0.0;
}

fn validate_lake_soil(soil: &LakeSnowWaterSoil) -> Result<()> {
    ensure!(
        !soil.thickness_m.is_empty()
            && soil.thickness_m.len() == soil.porosity.len()
            && soil.thickness_m.len() == soil.liquid_water_kg_m2.len()
            && soil.thickness_m.len() == soil.ice_water_kg_m2.len()
            && soil
                .thickness_m
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && soil
                .porosity
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            && soil
                .liquid_water_kg_m2
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && soil
                .ice_water_kg_m2
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "lake soil state is invalid"
    );
    Ok(())
}

fn saturate_lake_soil(soil: &mut LakeSnowWaterSoil) -> f64 {
    let mut water_change = 0.0;
    for layer in 0..soil.thickness_m.len() {
        let thickness = soil.thickness_m[layer];
        let before = soil.liquid_water_kg_m2[layer] + soil.ice_water_kg_m2[layer];
        let saturation = soil.liquid_water_kg_m2[layer] / (thickness * 1000.0)
            + soil.ice_water_kg_m2[layer] / (thickness * 917.0);
        if saturation < soil.porosity[layer] {
            soil.liquid_water_kg_m2[layer] =
                (soil.porosity[layer] * thickness - soil.ice_water_kg_m2[layer] / 917.0).max(0.0)
                    * 1000.0;
            soil.ice_water_kg_m2[layer] = (soil.porosity[layer] * thickness
                - soil.liquid_water_kg_m2[layer] / 1000.0)
                .max(0.0)
                * 917.0;
        } else {
            soil.liquid_water_kg_m2[layer] = (soil.liquid_water_kg_m2[layer]
                - (saturation - soil.porosity[layer]) * 1000.0 * thickness)
                .max(0.0);
            soil.ice_water_kg_m2[layer] = (soil.porosity[layer] * thickness
                - soil.liquid_water_kg_m2[layer] / 1000.0)
                .max(0.0)
                * 917.0;
        }
        if soil.liquid_water_kg_m2[layer] > soil.porosity[layer] * 1000.0 * thickness {
            soil.liquid_water_kg_m2[layer] = soil.porosity[layer] * 1000.0 * thickness;
            soil.ice_water_kg_m2[layer] = 0.0;
        }
        water_change += before - soil.liquid_water_kg_m2[layer] - soil.ice_water_kg_m2[layer];
    }
    water_change
}

fn adjust_dynamic_lake_water(
    had_snow: bool,
    input: LakeSnowWaterInput<'_>,
    bottom_drainage: f64,
    soil_water_change: f64,
    lake: &mut LakeColumn,
    fluxes: &LakeSnowWaterFluxes,
) -> Result<()> {
    let (mut liquid_depth, mut ice_depth) = if had_snow {
        (
            lake.thickness_m[0] * (1.0 - lake.ice_fraction[0])
                + bottom_drainage * input.time_step_seconds * 1.0e-3,
            lake.thickness_m[0] * lake.ice_fraction[0],
        )
    } else {
        (
            lake.thickness_m[0] * (1.0 - lake.ice_fraction[0])
                + (fluxes.snow_melt_kg_m2_s + input.dew_kg_m2_s - input.evaporation_kg_m2_s)
                    * input.time_step_seconds
                    * 1.0e-3,
            lake.thickness_m[0] * lake.ice_fraction[0]
                + (input.frost_kg_m2_s - input.sublimation_kg_m2_s)
                    * input.time_step_seconds
                    * 1.0e-3,
        )
    };
    if liquid_depth < 0.0 {
        ice_depth += liquid_depth;
        liquid_depth = 0.0;
    }
    if ice_depth < 0.0 {
        liquid_depth += ice_depth;
        ice_depth = 0.0;
    }
    lake.thickness_m[0] = (liquid_depth + ice_depth).max(1.0e-6);
    lake.ice_fraction[0] = (ice_depth / lake.thickness_m[0]).clamp(0.0, 1.0);
    let bottom = lake.thickness_m.len() - 1;
    lake.thickness_m[bottom] += soil_water_change * 1.0e-3;
    let mut layer = bottom;
    while lake.thickness_m[layer] < 0.0 {
        if layer > 0 {
            lake.thickness_m[layer - 1] += lake.thickness_m[layer];
        }
        lake.thickness_m[layer] = 0.0;
        if layer == 0 {
            break;
        }
        layer -= 1;
    }
    adjust_lake_layers(lake)
}

/// Ports `MOD_Lake:roughness_lake`.
pub fn lake_roughness(input: LakeRoughnessInput) -> Result<LakeRoughness> {
    ensure!(
        input.snow_layer_count <= 0
            && input.ground_temperature_k.is_finite()
            && input.lake_surface_temperature_k.is_finite()
            && input.surface_pressure_pa.is_finite()
            && input.surface_pressure_pa > 0.0
            && input.charnock_parameter.is_finite()
            && input.charnock_parameter >= 0.0
            && input.friction_velocity_m_s.is_finite()
            && input.friction_velocity_m_s >= 0.0,
        "lake roughness inputs are invalid"
    );
    if input.ground_temperature_k > FREEZING_K
        && input.lake_surface_temperature_k > FREEZING_K
        && input.snow_layer_count == 0
    {
        let viscosity = 1.51e-5 * (input.ground_temperature_k / 293.15).powf(1.5) * 1.013e5
            / input.surface_pressure_pa;
        let momentum_m = (0.1 * viscosity / input.friction_velocity_m_s.max(1.0e-4))
            .max(input.charnock_parameter * input.friction_velocity_m_s.powi(2) / 9.80616)
            .max(1.0e-5);
        let roughness_reynolds_sqrt = (momentum_m * input.friction_velocity_m_s / viscosity)
            .max(0.1)
            .sqrt();
        return Ok(LakeRoughness {
            momentum_m,
            sensible_heat_m: (momentum_m
                * (-0.4 / 0.713 * (4.0 * roughness_reynolds_sqrt - 3.2)).exp())
            .max(1.0e-5),
            latent_heat_m: (momentum_m
                * (-0.4 / 0.66 * (4.0 * roughness_reynolds_sqrt - 4.2)).exp())
            .max(1.0e-5),
        });
    }
    let momentum_m = if input.snow_layer_count == 0 {
        0.001
    } else {
        0.0024
    };
    let heat =
        momentum_m / (0.13 * (input.friction_velocity_m_s * momentum_m / 1.5e-5).powf(0.45)).exp();
    Ok(LakeRoughness {
        momentum_m,
        sensible_heat_m: heat,
        latent_heat_m: heat,
    })
}

/// Ports `MOD_Lake:hConductivity_lake`, including density-dependent mixing.
pub fn lake_thermal_conductivity(
    input: LakeConductivityInput<'_>,
    column: &LakeColumn,
) -> Result<LakeConductivity> {
    validate(column)?;
    ensure!(
        input.snow_layer_count <= 0
            && input.ground_temperature_k.is_finite()
            && input.node_depth_m.len() == LAKE_LAYERS
            && input.node_depth_m.iter().all(|value| value.is_finite())
            && input
                .node_depth_m
                .windows(2)
                .all(|depths| depths[1] > depths[0])
            && input.latitude_radians.is_finite()
            && input.latitude_radians.abs() <= std::f64::consts::FRAC_PI_2
            && input.friction_velocity_m_s.is_finite()
            && input.friction_velocity_m_s >= 0.0
            && input.momentum_roughness_m.is_finite()
            && input.momentum_roughness_m > 0.0
            && input.lake_depth_m.is_finite()
            && input.lake_depth_m >= 0.0
            && input.deep_lake_threshold_m.is_finite()
            && input.deep_lake_threshold_m >= 0.0,
        "lake conductivity inputs are invalid"
    );
    let water_density = column
        .temperature_k
        .iter()
        .zip(&column.ice_fraction)
        .map(|(&temperature, &ice_fraction)| lake_water_density(temperature, ice_fraction))
        .collect::<Vec<_>>();
    let water_heat_capacity = 4188.0 * 1000.0;
    let effective_ice_conductivity = 2.29 * 917.0 / 1000.0;
    let molecular_diffusivity = 0.6 / water_heat_capacity;
    let wind_2m =
        (input.friction_velocity_m_s / 0.4 * (2.0 / input.momentum_roughness_m).ln()).max(0.1);
    let surface_water_velocity = 1.2e-3 * wind_2m;
    let decay = 6.6 * input.latitude_radians.sin().abs().sqrt() * wind_2m.powf(-1.84);
    let deep_lake = input.lake_depth_m >= input.deep_lake_threshold_m;
    let open_water = input.ground_temperature_k > FREEZING_K
        && column.temperature_k[0] > FREEZING_K
        && input.snow_layer_count == 0;
    let mut eddy_diffusivity = [0.0; LAKE_LAYERS];
    let mut thermal_conductivity_w_m_k = [0.0; LAKE_LAYERS];

    for layer in 0..LAKE_LAYERS - 1 {
        let density_gradient = (water_density[layer + 1] - water_density[layer])
            / (input.node_depth_m[layer + 1] - input.node_depth_m[layer]);
        let buoyancy_frequency = (9.80616 / water_density[layer] * density_gradient).max(7.5e-5);
        let numerator = 40.0 * buoyancy_frequency * (0.4 * input.node_depth_m[layer]).powi(2);
        let denominator = (surface_water_velocity.powi(2)
            * (-2.0 * decay * input.node_depth_m[layer]).max(-40.0).exp())
        .max(1.0e-10);
        let richardson = (-1.0 + (1.0 + numerator / denominator).max(0.0).sqrt()) / 20.0;
        let fang_stefan = 1.039e-8 * buoyancy_frequency.powf(-0.43);
        let mut diffusivity = molecular_diffusivity + fang_stefan;
        if open_water {
            let eddy = 0.4
                * surface_water_velocity
                * input.node_depth_m[layer]
                * (-decay * input.node_depth_m[layer]).max(-40.0).exp()
                / (1.0 + 37.0 * richardson.powi(2));
            diffusivity += eddy;
        }
        if deep_lake {
            diffusivity *= 5.0;
        }
        eddy_diffusivity[layer] = diffusivity;
        thermal_conductivity_w_m_k[layer] = if open_water {
            diffusivity * water_heat_capacity
        } else {
            frozen_thermal_conductivity(
                diffusivity,
                water_heat_capacity,
                effective_ice_conductivity,
                column.ice_fraction[layer],
            )
        };
    }
    eddy_diffusivity[LAKE_LAYERS - 1] = eddy_diffusivity[LAKE_LAYERS - 2];
    thermal_conductivity_w_m_k[LAKE_LAYERS - 1] = if open_water {
        thermal_conductivity_w_m_k[LAKE_LAYERS - 2]
    } else {
        frozen_thermal_conductivity(
            eddy_diffusivity[LAKE_LAYERS - 1],
            water_heat_capacity,
            effective_ice_conductivity,
            column.ice_fraction[LAKE_LAYERS - 1],
        )
    };
    Ok(LakeConductivity {
        thermal_conductivity_w_m_k: thermal_conductivity_w_m_k.to_vec(),
        top_eddy_conductivity_w_m_k: eddy_diffusivity[0] * water_heat_capacity,
    })
}

fn lake_water_density(temperature_k: f64, ice_fraction: f64) -> f64 {
    (1.0 - ice_fraction) * 1000.0 * (1.0 - 1.9549e-5 * (temperature_k - 277.0).abs().powf(1.68))
        + ice_fraction * 917.0
}

fn frozen_thermal_conductivity(
    diffusivity: f64,
    water_heat_capacity: f64,
    effective_ice_conductivity: f64,
    ice_fraction: f64,
) -> f64 {
    diffusivity * water_heat_capacity * effective_ice_conductivity
        / ((1.0 - ice_fraction) * effective_ice_conductivity
            + diffusivity * water_heat_capacity * ice_fraction)
}

/// Remaps a lake column to CoLM's depth-scaled standard ten-layer geometry.
///
/// The overlap remap and mixed liquid/ice energy reconciliation preserve
/// `MOD_Lake:adjust_lake_layer`. A zero-depth column is intentionally untouched.
pub fn adjust_lake_layers(column: &mut LakeColumn) -> Result<()> {
    validate(column)?;
    let total_depth_m: f64 = column.thickness_m.iter().sum();
    ensure!(total_depth_m.is_finite(), "lake depth must be finite");
    if total_depth_m == 0.0 {
        return Ok(());
    }

    let target_thickness_m = target_thickness(total_depth_m);
    let mut target_temperature_k = vec![0.0; LAKE_LAYERS];
    let mut target_ice_fraction = vec![0.0; LAKE_LAYERS];
    let mut source_layer = 0;
    let mut source_remaining_m = column.thickness_m[source_layer];

    for target_layer in 0..LAKE_LAYERS {
        let mut target_remaining_m = target_thickness_m[target_layer];
        let mut ice_temperature_sum = 0.0;
        let mut liquid_temperature_sum = 0.0;
        let mut ice_mass = 0.0;
        let mut liquid_mass = 0.0;
        while target_remaining_m > 0.0 {
            if source_remaining_m == 0.0 {
                if source_layer + 1 == LAKE_LAYERS {
                    // MOD_Lake exits its overlap loop at the final source layer.
                    // Preserve that behavior when binary roundoff leaves only a
                    // sub-ulp target remainder after the conserved remap.
                    break;
                }
                source_layer += 1;
                source_remaining_m = column.thickness_m[source_layer];
                continue;
            }
            let overlap_m = target_remaining_m.min(source_remaining_m);
            let source_ice = column.ice_fraction[source_layer];
            let source_temperature = column.temperature_k[source_layer];
            ice_temperature_sum += overlap_m * source_ice * source_temperature;
            ice_mass += overlap_m * source_ice;
            liquid_temperature_sum += overlap_m * (1.0 - source_ice) * source_temperature;
            liquid_mass += overlap_m * (1.0 - source_ice);
            target_remaining_m -= overlap_m;
            source_remaining_m -= overlap_m;
        }

        let (temperature_k, ice_mass) = reconcile_phase(
            target_thickness_m[target_layer],
            ice_temperature_sum,
            liquid_temperature_sum,
            ice_mass,
            liquid_mass,
        );
        target_temperature_k[target_layer] = temperature_k;
        target_ice_fraction[target_layer] = ice_mass / target_thickness_m[target_layer];
    }
    column.thickness_m = target_thickness_m;
    column.temperature_k = target_temperature_k;
    column.ice_fraction = target_ice_fraction;
    Ok(())
}

fn validate(column: &LakeColumn) -> Result<()> {
    ensure!(
        column.thickness_m.len() == LAKE_LAYERS
            && column.temperature_k.len() == LAKE_LAYERS
            && column.ice_fraction.len() == LAKE_LAYERS,
        "CoLM lake adjustment requires ten matching layers"
    );
    ensure!(
        column
            .thickness_m
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && column.temperature_k.iter().all(|value| value.is_finite())
            && column
                .ice_fraction
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value)),
        "lake column is physically invalid"
    );
    Ok(())
}

fn target_thickness(total_depth_m: f64) -> Vec<f64> {
    if total_depth_m <= 1.0 {
        return vec![total_depth_m / LAKE_LAYERS as f64; LAKE_LAYERS];
    }
    let depth_ratio = total_depth_m / DEFAULT_THICKNESS_M.iter().sum::<f64>();
    let mut thickness = DEFAULT_THICKNESS_M
        .iter()
        .map(|value| value * depth_ratio)
        .collect::<Vec<_>>();
    thickness[0] = DEFAULT_THICKNESS_M[0];
    thickness[LAKE_LAYERS - 1] -= thickness[0] - DEFAULT_THICKNESS_M[0] * depth_ratio;
    thickness
}

fn reconcile_phase(
    layer_thickness_m: f64,
    ice_temperature_sum: f64,
    liquid_temperature_sum: f64,
    mut ice_mass: f64,
    mut liquid_mass: f64,
) -> (f64, f64) {
    if ice_mass == 0.0 {
        return (liquid_temperature_sum / liquid_mass, ice_mass);
    }
    if liquid_mass == 0.0 {
        return (ice_temperature_sum / ice_mass, ice_mass);
    }
    let ice_temperature_k = ice_temperature_sum / ice_mass;
    let liquid_temperature_k = liquid_temperature_sum / liquid_mass;
    let liquid_heat =
        LIQUID_HEAT_CAPACITY_J_KG_K * liquid_mass * (liquid_temperature_k - FREEZING_K);
    let ice_heat = ICE_HEAT_CAPACITY_J_KG_K * ice_mass * (FREEZING_K - ice_temperature_k);
    let ice_fusion = ice_mass * FUSION_HEAT_J_KG;
    let liquid_fusion = liquid_mass * FUSION_HEAT_J_KG;
    if liquid_heat >= ice_heat + ice_fusion {
        ice_mass = 0.0;
        liquid_mass = layer_thickness_m;
        (
            FREEZING_K
                + (liquid_heat - ice_heat - ice_fusion)
                    / (liquid_mass * LIQUID_HEAT_CAPACITY_J_KG_K),
            ice_mass,
        )
    } else if liquid_heat >= ice_heat {
        ice_mass -= (liquid_heat - ice_heat) / FUSION_HEAT_J_KG;
        (FREEZING_K, ice_mass)
    } else if liquid_heat + liquid_fusion < ice_heat {
        ice_mass = layer_thickness_m;
        (
            FREEZING_K
                - (ice_heat - liquid_heat - liquid_fusion) / (ice_mass * ICE_HEAT_CAPACITY_J_KG_K),
            ice_mass,
        )
    } else {
        ice_mass += (ice_heat - liquid_heat) / FUSION_HEAT_J_KG;
        (FREEZING_K, ice_mass)
    }
}

#[cfg(test)]
#[path = "lake_tests.rs"]
mod lake_tests;
