//! Standard LCT energy sequence assembled from CoLM's shared kernels.
//!
//! This is the common `CoLMMAIN → THERMAL` path up to, and including, the
//! ground-temperature solve: forcing partition, canopy interception, shortwave,
//! root stress, leaf energy balance, then ground conduction.  It deliberately
//! stops before the separate soil/snow water-transport stage; that stage owns the
//! removal and redistribution of the returned evaporation and transpiration.

use anyhow::{ensure, Result};

use crate::{
    ground_fluxes, ground_temperature, intercept_canopy, net_solar, root_uptake,
    soil_surface_resistance, CanopyInterceptionFluxes, CanopyInterceptionInput, ColdStartRadiation,
    GroundFluxInput, GroundFluxState, GroundHumidityInput, GroundHumidityState,
    GroundTemperatureInput, GroundTemperatureState, LeafTemperatureInput, LeafTemperatureOutput,
    LeafTemperatureState, NetSolarFluxes, NetSolarInput, PrecipitationPhaseScheme,
    PrecipitationState, RootUptakeInput, RootUptakeState, RuntimeForcing,
    SoilSurfaceResistanceInput, ThermalWaterFluxes, ThermalWaterInput, Water2014SoilInput,
    Water2014SoilOutput, Water2014SoilState,
};

const AIR_GAS_CONSTANT_J_KG_K: f64 = 287.04;
const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;

/// Immutable inputs to one standard LCT energy update.
///
/// The nested inputs retain the public interfaces of their source kernels.  This
/// sequence overwrites their shared hand-offs from `forcing`, rather than asking
/// a caller to reproduce them: shortwave, rain/snow, reference meteorology,
/// root stress, ground resistance, and the converged leaf-to-ground fluxes.
#[derive(Debug, Clone, Copy)]
pub struct StandardLctEnergyInput<'a> {
    pub forcing: RuntimeForcing,
    pub precipitation_scheme: PrecipitationPhaseScheme,
    pub interception: CanopyInterceptionInput,
    pub solar: NetSolarInput,
    pub root_uptake: RootUptakeInput<'a>,
    pub soil_surface_resistance: SoilSurfaceResistanceInput,
    pub ground_flux: GroundFluxInput,
    pub leaf_temperature: LeafTemperatureInput<'a>,
    pub ground_temperature: GroundTemperatureInput<'a>,
}

/// Persistent radiation and canopy state for [`standard_lct_energy_step`].
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctEnergyState {
    /// Broadband optical coefficients carried from cold start through every step.
    pub radiation: ColdStartRadiation,
    /// Leaf temperature and canopy water pools carried between time steps.
    pub leaf: LeafTemperatureState,
}

/// Persistent no-snow standard-LCT state for [`standard_lct_soil_step`].
///
/// The ground temperature and soil-water arrays live here once, so the energy
/// and hydrology calls cannot diverge by receiving separate dynamic columns.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctSoilState {
    pub energy: StandardLctEnergyState,
    pub temperature_k: Vec<f64>,
    pub water: Water2014SoilState,
}

/// The component results of one standard LCT energy update.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctEnergyOutput {
    pub precipitation: PrecipitationState,
    pub interception: CanopyInterceptionFluxes,
    pub shortwave: NetSolarFluxes,
    /// Runtime-derived lower humidity boundary for a non-split surface.
    /// Split soil/snow still has separate soil and snow boundaries.
    pub ground_humidity: Option<GroundHumidityState>,
    pub root_uptake: RootUptakeState,
    pub soil_surface_resistance_s_m: f64,
    /// Bare-ground exchange used as the `LeafTemperature` lower boundary.
    pub preliminary_ground_flux: GroundFluxState,
    pub leaf: LeafTemperatureOutput,
    pub ground: GroundTemperatureState,
    /// Fluxes corrected to the just-solved ground temperature, as in
    /// `MOD_Thermal.F90` section 6.  For non-split surfaces, water availability
    /// is also applied before these totals are returned.
    pub corrected_ground_sensible_heat_w_m2: f64,
    pub corrected_ground_evaporation_kg_m2_s: f64,
    /// Source-equivalent upper-layer phase partition for a non-split surface.
    /// Split soil/snow carries two independent surface budgets and returns
    /// `None` until that separate branch is ported.
    pub thermal_water: Option<ThermalWaterFluxes>,
    pub total_sensible_heat_w_m2: f64,
    pub total_evaporation_kg_m2_s: f64,
}

/// Static inputs and forcing to one no-snow standard-LCT time step.
///
/// The dynamic arrays embedded in `energy.ground_temperature` and `water` are
/// templates only; this driver replaces them with [`StandardLctSoilState`].
#[derive(Debug, Clone, Copy)]
pub struct StandardLctSoilInput<'a> {
    pub energy: StandardLctEnergyInput<'a>,
    pub water: Water2014SoilInput<'a>,
}

/// Results from one linked `THERMAL → WATER_2014` no-snow time step.
#[derive(Debug, Clone, PartialEq)]
pub struct StandardLctSoilOutput {
    pub energy: StandardLctEnergyOutput,
    pub water: Water2014SoilOutput,
}

/// Runs the normal LCT `CoLMMAIN → THERMAL` energy chain without duplicating a
/// physics kernel in a runtime or initializer.
///
/// This supports both normal LCT and the PHS leaf branch. PFT/PC aggregation,
/// ozone, and the downstream soil/snow-water solver remain their own source
/// branches and are not approximated here.
pub fn standard_lct_energy_step(
    input: StandardLctEnergyInput<'_>,
    state: &mut StandardLctEnergyState,
) -> Result<StandardLctEnergyOutput> {
    validate(input)?;

    let ground_humidity = non_split_ground_humidity_input(input)?;
    let precipitation = input
        .forcing
        .partition_precipitation(0, input.precipitation_scheme)?;
    let shortwave = net_solar(
        NetSolarInput {
            forcing: input.forcing.shortwave,
            ..input.solar
        },
        &mut state.radiation,
    )?;
    let interception = intercept_canopy(
        CanopyInterceptionInput {
            convective_rain_kg_m2_s: precipitation.convective_rain_kg_m2_s,
            convective_snow_kg_m2_s: precipitation.convective_snow_kg_m2_s,
            large_scale_rain_kg_m2_s: precipitation.large_scale_rain_kg_m2_s,
            large_scale_snow_kg_m2_s: precipitation.large_scale_snow_kg_m2_s,
            leaf_temperature_k: state.leaf.leaf_temperature_k,
            ..input.interception
        },
        &mut state.leaf.canopy_water,
    )?;
    let root_uptake = root_uptake_input(input)?;
    let soil_surface_resistance_s_m = soil_surface_resistance_input(input, ground_humidity)?;
    let mut ground_flux_input = ground_flux_input(
        input.ground_flux,
        input.forcing,
        soil_surface_resistance_s_m,
    );
    let (ground_temperature_k, soil_temperature_k, snow_temperature_k) =
        surface_temperatures(input.ground_temperature);
    ground_flux_input.ground_temperature_k = ground_temperature_k;
    ground_flux_input.soil_temperature_k = soil_temperature_k;
    ground_flux_input.snow_temperature_k = snow_temperature_k;
    if let Some(humidity) = ground_humidity {
        ground_flux_input.ground_specific_humidity = humidity.ground_specific_humidity;
        ground_flux_input.soil_specific_humidity = humidity.ground_specific_humidity;
        ground_flux_input.snow_specific_humidity = humidity.ground_specific_humidity;
        ground_flux_input.ground_humidity_temperature_derivative_kg_kg_k =
            humidity.ground_humidity_temperature_slope_kg_kg_k;
    }
    let preliminary_ground_flux = ground_fluxes(ground_flux_input)?;
    let leaf_input = leaf_input(
        input.leaf_temperature,
        input.forcing,
        &state.radiation,
        shortwave,
        interception,
        root_uptake.soil_water_stress,
        root_uptake.maximum_transpiration_mm_s,
        soil_surface_resistance_s_m,
        ground_flux_input,
        preliminary_ground_flux,
    );
    let leaf = crate::leaf_temperature(leaf_input, &mut state.leaf)?;
    let ground = ground_temperature(GroundTemperatureInput {
        time_step_seconds: input.interception.time_step_seconds,
        absorbed_ground_shortwave_w_m2: shortwave.ground_absorbed_w_m2,
        absorbed_soil_shortwave_w_m2: shortwave.soil_absorbed_w_m2,
        absorbed_snow_shortwave_w_m2: shortwave.snow_absorbed_w_m2,
        downward_longwave_w_m2: leaf.downward_longwave_w_m2,
        sensible_ground_w_m2: leaf.ground_sensible_heat_w_m2,
        sensible_soil_w_m2: leaf.soil_sensible_heat_w_m2,
        sensible_snow_w_m2: leaf.snow_sensible_heat_w_m2,
        evaporation_ground_kg_m2_s: leaf.ground_evaporation_kg_m2_s,
        evaporation_soil_kg_m2_s: leaf.soil_evaporation_kg_m2_s,
        evaporation_snow_kg_m2_s: leaf.snow_evaporation_kg_m2_s,
        ground_flux_temperature_derivative_w_m2_k: leaf.ground_flux_temperature_slope_w_m2_k,
        vaporization_heat_j_kg: leaf_input.ground_latent_heat_j_kg,
        ground_emissivity: leaf_input.ground_emissivity,
        rain_on_ground_kg_m2_s: interception.ground_rain_kg_m2_s,
        snow_on_ground_kg_m2_s: interception.ground_snow_kg_m2_s,
        precipitation_temperature_k: precipitation.precipitation_temperature_k,
        ground_temperature_k,
        soil_surface_temperature_k: soil_temperature_k,
        snow_surface_temperature_k: snow_temperature_k,
        ..input.ground_temperature
    })?;
    let ground_temperature_change =
        solved_ground_temperature_change(input.ground_temperature, &ground)?;
    let mut corrected_ground_sensible_heat_w_m2 = leaf.ground_sensible_heat_w_m2
        + ground_temperature_change * leaf.ground_sensible_temperature_slope_w_m2_k;
    let mut corrected_ground_evaporation_kg_m2_s = leaf.ground_evaporation_kg_m2_s
        + ground_temperature_change * leaf.ground_latent_temperature_slope_kg_m2_s_k;
    let thermal_water = if input.ground_temperature.use_split_soil_snow {
        None
    } else {
        let water = crate::partition_no_split_thermal_water(ThermalWaterInput {
            corrected_ground_evaporation_kg_m2_s,
            upper_liquid_water_kg_m2: ground.liquid_water_kg_m2[0],
            upper_ice_water_kg_m2: ground.ice_water_kg_m2[0],
            upper_temperature_k: ground.temperature_k[0],
            time_step_seconds: input.ground_temperature.time_step_seconds,
            ground_latent_heat_j_kg: leaf_input.ground_latent_heat_j_kg,
        })?;
        corrected_ground_sensible_heat_w_m2 += water.sensible_heat_correction_w_m2;
        corrected_ground_evaporation_kg_m2_s = water.ground_evaporation_kg_m2_s;
        Some(water)
    };
    let total_sensible_heat_w_m2 =
        leaf.leaf_sensible_heat_w_m2 + corrected_ground_sensible_heat_w_m2;
    let total_evaporation_kg_m2_s =
        leaf.leaf_evaporation_kg_m2_s + corrected_ground_evaporation_kg_m2_s;

    Ok(StandardLctEnergyOutput {
        precipitation,
        interception,
        shortwave,
        ground_humidity,
        root_uptake,
        soil_surface_resistance_s_m,
        preliminary_ground_flux,
        leaf,
        ground,
        corrected_ground_sensible_heat_w_m2,
        corrected_ground_evaporation_kg_m2_s,
        thermal_water,
        total_sensible_heat_w_m2,
        total_evaporation_kg_m2_s,
    })
}

/// Runs the no-snow regular-soil `CoLMMAIN → THERMAL → WATER_2014` sequence.
///
/// This deliberately accepts the branch for which both source components are
/// ported: a normal soil LCT patch without snow or split soil/snow.  It does
/// not approximate PFT/PC aggregation, snow, irrigation, VSF, or wetland
/// hydrology.
pub fn standard_lct_soil_step(
    input: StandardLctSoilInput<'_>,
    state: &mut StandardLctSoilState,
) -> Result<StandardLctSoilOutput> {
    validate_soil_step(input, state)?;
    let mut energy_input = input.energy;
    energy_input.ground_temperature = GroundTemperatureInput {
        temperature_k: &state.temperature_k,
        liquid_water_kg_m2: &state.water.liquid_water_kg_m2,
        ice_water_kg_m2: &state.water.ice_water_kg_m2,
        ..input.energy.ground_temperature
    };
    let energy = standard_lct_energy_step(energy_input, &mut state.energy)?;
    state.temperature_k = energy.ground.temperature_k.clone();
    state.water.liquid_water_kg_m2 = energy.ground.liquid_water_kg_m2.clone();
    state.water.ice_water_kg_m2 = energy.ground.ice_water_kg_m2.clone();

    let thermal_water = energy
        .thermal_water
        .expect("validated no-split energy step supplies thermal water");
    let root_flux_mm_s = if input.water.plant_hydraulics {
        ensure!(
            energy.leaf.root_flux_kg_m2_s.len() == state.temperature_k.len(),
            "plant-hydraulic leaf output must provide one root flux per soil layer"
        );
        &energy.leaf.root_flux_kg_m2_s
    } else {
        input.water.root_flux_mm_s
    };
    let water = crate::water_2014_soil_step(
        Water2014SoilInput {
            time_step_seconds: input.energy.interception.time_step_seconds,
            fluxes: crate::Water2014SoilFluxes {
                ground_rain_kg_m2_s: energy.interception.ground_rain_kg_m2_s,
                snowmelt_kg_m2_s: energy.ground.snow_melt_rate_kg_m2_s,
                ground_evaporation_kg_m2_s: thermal_water.evaporation_kg_m2_s,
                transpiration_kg_m2_s: energy.leaf.transpiration_kg_m2_s,
                soil_dew_kg_m2_s: thermal_water.dew_kg_m2_s,
                soil_frost_kg_m2_s: thermal_water.frost_kg_m2_s,
                soil_sublimation_kg_m2_s: thermal_water.sublimation_kg_m2_s,
            },
            temperature_k: &state.temperature_k,
            root_flux_mm_s,
            ..input.water
        },
        &mut state.water,
    )?;
    Ok(StandardLctSoilOutput { energy, water })
}

fn validate_soil_step(input: StandardLctSoilInput<'_>, state: &StandardLctSoilState) -> Result<()> {
    let ground = input.energy.ground_temperature;
    ensure!(
        ground.patch_type == 0
            && input.water.patch_type == 0
            && !ground.use_split_soil_snow
            && ground.snow_layers == 0
            && ground.snow_water_equivalent_kg_m2 == 0.0
            && ground.snow_depth_m == 0.0
            && ground.snow_cover_fraction == 0.0
            && !input.water.urban_run
            && (input.energy.interception.time_step_seconds - input.water.time_step_seconds).abs()
                <= 1.0e-12
            && state.temperature_k.len() == ground.temperature_k.len()
            && state.water.liquid_water_kg_m2.len() == state.temperature_k.len()
            && state.water.ice_water_kg_m2.len() == state.temperature_k.len()
            && input.water.layer_thickness_m.len() == state.temperature_k.len()
            && input.water.root_flux_mm_s.len() == state.temperature_k.len(),
        "standard_lct_soil_step supports one no-snow regular-soil state"
    );
    Ok(())
}

fn non_split_ground_humidity_input(
    input: StandardLctEnergyInput<'_>,
) -> Result<Option<GroundHumidityState>> {
    let ground = input.ground_temperature;
    if ground.use_split_soil_snow {
        return Ok(None);
    }
    let soil = ground.snow_layers;
    Ok(Some(crate::non_split_ground_humidity(
        GroundHumidityInput {
            ground_temperature_k: surface_temperatures(ground).0,
            surface_pressure_pa: input.forcing.surface_pressure_pa,
            air_specific_humidity: input.forcing.specific_humidity,
            snow_cover_fraction: ground.snow_cover_fraction,
            top_layer_thickness_m: ground.layer_thickness_m[soil],
            top_layer_liquid_water_kg_m2: ground.liquid_water_kg_m2[soil],
            top_layer_ice_water_kg_m2: ground.ice_water_kg_m2[soil],
            top_layer_porosity: ground.soil_porosity[0],
            top_layer_residual_water: ground.soil_residual_water[0],
            saturated_soil_suction_mm: ground.soil_suction_mm[0],
            hydraulic_model: ground.soil_hydraulic_model[0],
        },
    )?))
}

fn root_uptake_input(input: StandardLctEnergyInput<'_>) -> Result<RootUptakeState> {
    let ground = input.ground_temperature;
    let soil = ground.snow_layers;
    root_uptake(RootUptakeInput {
        temperature_k: &ground.temperature_k[soil..],
        liquid_water_kg_m2: &ground.liquid_water_kg_m2[soil..],
        ..input.root_uptake
    })
}

fn soil_surface_resistance_input(
    input: StandardLctEnergyInput<'_>,
    ground_humidity: Option<GroundHumidityState>,
) -> Result<f64> {
    let Some(humidity) = ground_humidity else {
        return soil_surface_resistance(input.soil_surface_resistance);
    };
    let ground = input.ground_temperature;
    let soil = ground.snow_layers;
    soil_surface_resistance(SoilSurfaceResistanceInput {
        porosity: ground.soil_porosity[0],
        saturated_soil_suction_mm: ground.soil_suction_mm[0],
        residual_water: ground.soil_residual_water[0],
        hydraulic_model: ground.soil_hydraulic_model[0],
        layer_thickness_m: ground.layer_thickness_m[soil],
        temperature_k: surface_temperatures(ground).1,
        liquid_water_kg_m2: ground.liquid_water_kg_m2[soil],
        ice_water_kg_m2: ground.ice_water_kg_m2[soil],
        snow_cover_fraction: ground.snow_cover_fraction,
        ground_specific_humidity: humidity.ground_specific_humidity,
        ..input.soil_surface_resistance
    })
}

fn ground_flux_input(
    input: GroundFluxInput,
    forcing: RuntimeForcing,
    soil_surface_resistance_s_m: f64,
) -> GroundFluxInput {
    let potential_temperature_k = forcing.air_temperature_k
        * (100_000.0 / forcing.surface_pressure_pa)
            .powf(AIR_GAS_CONSTANT_J_KG_K / AIR_HEAT_CAPACITY_J_KG_K);
    GroundFluxInput {
        eastward_wind_m_s: forcing.eastward_wind_m_s,
        northward_wind_m_s: forcing.northward_wind_m_s,
        air_specific_humidity: forcing.specific_humidity,
        reference_wind_m_s: forcing
            .eastward_wind_m_s
            .hypot(forcing.northward_wind_m_s)
            .max(0.1),
        reference_temperature_k: forcing.air_temperature_k,
        potential_temperature_k,
        virtual_potential_temperature_k: potential_temperature_k
            * (1.0 + 0.61 * forcing.specific_humidity),
        soil_surface_resistance_s_m,
        ..input
    }
}

#[allow(clippy::too_many_arguments)]
fn leaf_input<'a>(
    input: LeafTemperatureInput<'a>,
    forcing: RuntimeForcing,
    radiation: &ColdStartRadiation,
    shortwave: NetSolarFluxes,
    interception: CanopyInterceptionFluxes,
    soil_water_stress: f64,
    transpiration_limit_kg_m2_s: f64,
    soil_surface_resistance_s_m: f64,
    ground_flux: GroundFluxInput,
    preliminary_ground_flux: GroundFluxState,
) -> LeafTemperatureInput<'a> {
    let direct_leaf_optical_depth = (radiation.direct_extinction * input.leaf_area_index).min(40.0);
    let canopy_absorbed_solar_w_m2 =
        shortwave.sunlit_absorbed_w_m2 + shortwave.shaded_absorbed_w_m2;
    let sunlit_fraction = if forcing.cosine_zenith <= 0.0 || canopy_absorbed_solar_w_m2 < 1.0 {
        0.5
    } else {
        (1.0 - (-direct_leaf_optical_depth).exp()) / direct_leaf_optical_depth.max(1.0e-6)
    };
    LeafTemperatureInput {
        time_step_seconds: input.time_step_seconds,
        direct_extinction: radiation.direct_extinction,
        diffuse_extinction: radiation.diffuse_extinction,
        eastward_wind_m_s: forcing.eastward_wind_m_s,
        northward_wind_m_s: forcing.northward_wind_m_s,
        reference_air_temperature_k: forcing.air_temperature_k,
        potential_temperature_k: ground_flux.potential_temperature_k,
        virtual_potential_temperature_k: ground_flux.virtual_potential_temperature_k,
        reference_specific_humidity: forcing.specific_humidity,
        surface_pressure_pa: forcing.surface_pressure_pa,
        sunlit_absorbed_par_w_m2: shortwave.par_sunlit_w_m2,
        shaded_absorbed_par_w_m2: shortwave.par_shaded_w_m2,
        canopy_absorbed_solar_w_m2,
        atmospheric_longwave_w_m2: forcing.downward_longwave_w_m2,
        sunlit_fraction,
        canopy_longwave_gap_fraction: radiation.thermal_gap_fraction,
        soil_roughness_m: ground_flux.soil_roughness_m,
        snow_roughness_m: ground_flux.snow_roughness_m,
        snow_cover_fraction: ground_flux.snow_cover_fraction,
        ground_obukhov_length_m: ground_flux.wind_height_m
            / preliminary_ground_flux.dimensionless_height,
        transpiration_limit_kg_m2_s,
        ground_temperature_k: ground_flux.ground_temperature_k,
        soil_surface_temperature_k: ground_flux.soil_temperature_k,
        snow_surface_temperature_k: ground_flux.snow_temperature_k,
        ground_specific_humidity: ground_flux.ground_specific_humidity,
        soil_specific_humidity: ground_flux.soil_specific_humidity,
        snow_specific_humidity: ground_flux.snow_specific_humidity,
        ground_humidity_temperature_slope_k: ground_flux
            .ground_humidity_temperature_derivative_kg_kg_k,
        soil_surface_resistance_s_m,
        ground_emissivity: input.ground_emissivity,
        precipitation_temperature_k: input.precipitation_temperature_k,
        intercepted_rain_kg_m2_s: interception.retained_rain_kg_m2_s,
        intercepted_snow_kg_m2_s: interception.retained_snow_kg_m2_s,
        ground_latent_heat_j_kg: ground_flux.vaporization_heat_j_kg,
        soil_water_stress_sunlit: soil_water_stress,
        soil_water_stress_shaded: soil_water_stress,
        ..input
    }
}

fn solved_ground_temperature_change(
    input: GroundTemperatureInput<'_>,
    state: &GroundTemperatureState,
) -> Result<f64> {
    ensure!(
        state.temperature_k.len() == input.temperature_k.len(),
        "ground-temperature solver returned a different layer count"
    );
    let current_ground_temperature_k = if input.use_split_soil_snow {
        input.snow_cover_fraction * state.temperature_k[0]
            + (1.0 - input.snow_cover_fraction) * state.temperature_k[input.snow_layers]
    } else {
        state.temperature_k[0]
    };
    Ok(current_ground_temperature_k - surface_temperatures(input).0)
}

fn surface_temperatures(input: GroundTemperatureInput<'_>) -> (f64, f64, f64) {
    let snow_temperature_k = input.temperature_k[0];
    let soil_temperature_k = input.temperature_k[input.snow_layers];
    let ground_temperature_k = if input.use_split_soil_snow {
        input.snow_cover_fraction * snow_temperature_k
            + (1.0 - input.snow_cover_fraction) * soil_temperature_k
    } else {
        snow_temperature_k
    };
    (ground_temperature_k, soil_temperature_k, snow_temperature_k)
}

fn validate(input: StandardLctEnergyInput<'_>) -> Result<()> {
    let leaf = input.leaf_temperature;
    let ground_flux = input.ground_flux;
    let ground = input.ground_temperature;
    ensure!(
        input.solar.patch_type == 0
            && ground.patch_type == 0
            && input.interception.time_step_seconds > 0.0
            && input.solar.time_step_seconds > 0
            && same(
                input.interception.time_step_seconds,
                f64::from(input.solar.time_step_seconds),
            )
            && same(
                input.interception.time_step_seconds,
                ground.time_step_seconds
            )
            && same(input.interception.time_step_seconds, leaf.time_step_seconds)
            && same(input.solar.leaf_area_index, leaf.leaf_area_index)
            && same(input.solar.stem_area_index, leaf.stem_area_index)
            && same(input.interception.leaf_area_index, leaf.leaf_area_index)
            && same(input.interception.stem_area_index, leaf.stem_area_index)
            && same(input.solar.snow_fraction, ground_flux.snow_cover_fraction)
            && same(input.solar.snow_fraction, ground.snow_cover_fraction)
            && same(
                input.soil_surface_resistance.air_density_kg_m3,
                ground_flux.air_density_kg_m3,
            )
            && (!ground.use_split_soil_snow
                || same(
                    input.soil_surface_resistance.ground_specific_humidity,
                    ground_flux.ground_specific_humidity,
                ))
            && input.soil_surface_resistance.scheme == ground_flux.surface_resistance_scheme
            && leaf.options.split_soil_snow == ground.use_split_soil_snow
            && leaf.options.soil_resistance_is_conductance
                == (ground_flux.surface_resistance_scheme == 4)
            && same(leaf.air_density_kg_m3, ground_flux.air_density_kg_m3)
            && same(leaf.ground_emissivity, ground.ground_emissivity)
            && leaf.leaf_area_index + leaf.stem_area_index > 1.0e-6
            && ground.temperature_k.len() > ground.snow_layers,
        "standard LCT energy step needs one consistent soil-patch state and one shared time step"
    );
    let snow_temperature_k = ground.temperature_k[0];
    let soil_temperature_k = ground.temperature_k[ground.snow_layers];
    let ground_temperature_k = if ground.use_split_soil_snow {
        ground.snow_cover_fraction * snow_temperature_k
            + (1.0 - ground.snow_cover_fraction) * soil_temperature_k
    } else {
        snow_temperature_k
    };
    ensure!(
        !ground.use_split_soil_snow
            || (same(ground_flux.ground_temperature_k, ground_temperature_k)
                && same(ground_flux.soil_temperature_k, soil_temperature_k)
                && same(ground_flux.snow_temperature_k, snow_temperature_k)),
        "ground-flux and ground-temperature inputs must describe the same surface state"
    );
    Ok(())
}

fn same(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-12 * left.abs().max(right.abs()).max(1.0)
}

#[cfg(test)]
#[path = "standard_lct_step_tests.rs"]
mod standard_lct_step_tests;
