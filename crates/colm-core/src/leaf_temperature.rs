//! Standard land-cover canopy energy balance from `MOD_LeafTemperature.F90`.
//!
//! This is the reusable LCT/PFT two-big-leaf path used by the native runtime;
//! it contains no NetCDF or process orchestration.  Plant hydraulics and ozone
//! are separate upstream feature branches and are intentionally not silently
//! approximated here.

use anyhow::{ensure, Result};

use crate::{
    canopy_diffusivity_resistance_analytic, canopy_monin_obukhov_with_scheme, canopy_roughness,
    canopy_wetness, effective_canopy_wind, initialize_monin_obukhov, saturation_specific_humidity,
    stomata, CanopyDiffusivityProfileInput, CanopyMoninObukhovInput, CanopyWater,
    CanopyWindProfileInput, LeafBiochemistry, LeafPhotosynthesisInput, MoninObukhovInitialInput,
    MoninObukhovInput, StomataInput, StomataOptions, SurfaceLayerScheme, FREEZING_K,
};

const VON_KARMAN: f64 = 0.4;
const GRAVITY_M_S2: f64 = 9.80616;
const LATENT_HEAT_VAPORIZATION_J_KG: f64 = 2.5104e6;
const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const STEFAN_BOLTZMANN: f64 = 5.67e-8;
const FUSION_HEAT_J_KG: f64 = 0.3336e6;
const MAX_ITERATIONS: usize = 40;
const MIN_ITERATIONS: usize = 6;
const MAX_TEMPERATURE_STEP_K: f64 = 3.0;
const TEMPERATURE_TOLERANCE_K: f64 = 0.01;
const FLUX_TOLERANCE_W_M2: f64 = 0.1;

/// Whether forcing observation heights are absolute elevations or heights above
/// the canopy top, matching `DEF_forcing%HEIGHT_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationHeightMode {
    Absolute,
    RelativeToCanopy,
}

/// Runtime switches that materially change the standard leaf-temperature path.
#[derive(Debug, Clone, Copy)]
pub struct LeafTemperatureOptions {
    pub observation_height_mode: ObservationHeightMode,
    pub vegetation_snow: bool,
    pub split_soil_snow: bool,
    /// `DEF_RSS_SCHEME == 4`: `soil_surface_resistance` is a conductance factor.
    pub soil_resistance_is_conductance: bool,
    pub surface_layer_scheme: SurfaceLayerScheme,
    pub stomata: StomataOptions,
}

impl Default for LeafTemperatureOptions {
    fn default() -> Self {
        Self {
            observation_height_mode: ObservationHeightMode::Absolute,
            vegetation_snow: false,
            split_soil_snow: false,
            soil_resistance_is_conductance: false,
            surface_layer_scheme: SurfaceLayerScheme::Standard,
            stomata: StomataOptions::default(),
        }
    }
}

/// Immutable forcing, surface, and vegetation parameters for one canopy step.
#[derive(Debug, Clone, Copy)]
pub struct LeafTemperatureInput {
    pub time_step_seconds: f64,
    pub maximum_dew_mm: f64,
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub canopy_top_height_m: f64,
    pub inverse_sqrt_leaf_dimension_m_neg_half: f64,
    pub biochemistry: LeafBiochemistry,
    pub soil_water_stress_sunlit: f64,
    pub soil_water_stress_shaded: f64,
    pub wue_lambda: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub reference_air_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub virtual_potential_temperature_k: f64,
    pub reference_specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub air_density_kg_m3: f64,
    pub sunlit_absorbed_par_w_m2: f64,
    pub shaded_absorbed_par_w_m2: f64,
    pub canopy_absorbed_solar_w_m2: f64,
    pub atmospheric_longwave_w_m2: f64,
    pub sunlit_fraction: f64,
    pub canopy_longwave_gap_fraction: f64,
    pub oxygen_partial_pressure_pa: f64,
    pub atmospheric_co2_pa: f64,
    pub soil_roughness_m: f64,
    pub snow_roughness_m: f64,
    pub snow_cover_fraction: f64,
    pub ground_obukhov_length_m: f64,
    pub transpiration_limit_kg_m2_s: f64,
    pub ground_temperature_k: f64,
    pub soil_surface_temperature_k: f64,
    pub snow_surface_temperature_k: f64,
    pub ground_specific_humidity: f64,
    pub soil_specific_humidity: f64,
    pub snow_specific_humidity: f64,
    pub ground_humidity_temperature_slope_k: f64,
    pub soil_surface_resistance_s_m: f64,
    pub ground_emissivity: f64,
    pub precipitation_temperature_k: f64,
    pub intercepted_rain_kg_m2_s: f64,
    pub intercepted_snow_kg_m2_s: f64,
    pub ground_latent_heat_j_kg: f64,
    pub options: LeafTemperatureOptions,
}

/// Persistent state updated by one leaf-temperature solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafTemperatureState {
    pub leaf_temperature_k: f64,
    pub canopy_water: CanopyWater,
}

/// Fluxes and diagnostic state produced by one converged canopy solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafTemperatureOutput {
    pub wet_snow_fraction: f64,
    pub eastward_stress_kg_m_s2: f64,
    pub northward_stress_kg_m_s2: f64,
    pub ground_sensible_heat_w_m2: f64,
    pub soil_sensible_heat_w_m2: f64,
    pub snow_sensible_heat_w_m2: f64,
    pub ground_evaporation_kg_m2_s: f64,
    pub soil_evaporation_kg_m2_s: f64,
    pub snow_evaporation_kg_m2_s: f64,
    pub ground_flux_temperature_slope_w_m2_k: f64,
    pub ground_sensible_temperature_slope_w_m2_k: f64,
    pub ground_latent_temperature_slope_kg_m2_s_k: f64,
    pub air_temperature_2m_k: f64,
    pub air_specific_humidity_2m: f64,
    pub canopy_stomatal_resistance_s_m: f64,
    pub assimilation_mol_m2_s: f64,
    pub respiration_mol_m2_s: f64,
    pub leaf_sensible_heat_w_m2: f64,
    pub leaf_evaporation_kg_m2_s: f64,
    pub transpiration_kg_m2_s: f64,
    pub sunlit_transpiration_kg_m2_s: f64,
    pub shaded_transpiration_kg_m2_s: f64,
    pub sunlit_assimilation_mol_m2_s: f64,
    pub shaded_assimilation_mol_m2_s: f64,
    pub downward_longwave_w_m2: f64,
    pub upward_longwave_w_m2: f64,
    pub precipitation_heat_w_m2: f64,
    pub canopy_heat_storage_w_m2: f64,
    pub momentum_roughness_m: f64,
    pub zol: f64,
    pub bulk_richardson: f64,
    pub friction_velocity_m_s: f64,
    pub humidity_scale: f64,
    pub temperature_scale_k: f64,
    pub momentum_similarity: f64,
    pub heat_similarity: f64,
    pub moisture_similarity: f64,
    pub reference_to_canopy_moisture_resistance_s_m: f64,
    pub energy_balance_error_w_m2: f64,
    pub iterations: usize,
}

/// Port of `MOD_LeafTemperature:LeafTemperature` for the normal LCT/PFT path.
///
/// This solver deliberately uses the existing pure canopy, Monin-Obukhov,
/// humidity, and stomata kernels.  The caller owns interception and ground
/// thermal updates; this function only updates the leaf temperature and dew
/// pools that belong to the leaf energy balance.
pub fn leaf_temperature(
    input: LeafTemperatureInput,
    state: &mut LeafTemperatureState,
) -> Result<LeafTemperatureOutput> {
    validate(input, *state)?;
    let lai = input.leaf_area_index;
    let sai = input.stem_area_index;
    let lsai = lai + sai;
    let fsha = 1.0 - input.sunlit_fraction;
    let laisun = lai * input.sunlit_fraction;
    let laisha = lai * fsha;
    let cintsun = canopy_scaling(input.direct_extinction, input.diffuse_extinction, lai);
    let cintsha = [
        integrated_extinction(0.110, lai) - cintsun[0],
        integrated_extinction(input.diffuse_extinction, lai) - cintsun[1],
        lai - cintsun[2],
    ];
    let clai = if input.options.vegetation_snow {
        0.2 * lsai * WATER_HEAT_CAPACITY_J_KG_K
            + state.canopy_water.rain_mm * WATER_HEAT_CAPACITY_J_KG_K
            + state.canopy_water.snow_mm * ICE_HEAT_CAPACITY_J_KG_K
    } else {
        0.0
    };
    let wetness = canopy_wetness(
        lai,
        sai,
        input.maximum_dew_mm,
        state.canopy_water,
        input.options.vegetation_snow,
    )?;
    let fwet = wetness.wet_fraction;
    let roughness = canopy_roughness(lsai, input.canopy_top_height_m, 1.0)?;
    let z0mv = roughness.momentum_roughness_m;
    let displacement = roughness.displacement_height_m;
    let displasink = (input.canopy_top_height_m / 2.0).max(displacement);
    let hsink = z0mv + displasink;
    let z0mg = (1.0 - input.snow_cover_fraction) * input.soil_roughness_m
        + input.snow_cover_fraction * input.snow_roughness_m;
    let frontal_area = 1.0 - (-0.5 * lsai).exp();
    let sqrt_drag = (0.003 + 0.3 * frontal_area).sqrt().min(0.3);
    let attenuation = input.canopy_top_height_m
        / (input.canopy_top_height_m - displacement)
        / (VON_KARMAN / sqrt_drag);
    let (wind_height, temperature_height, humidity_height) =
        match input.options.observation_height_mode {
            ObservationHeightMode::Absolute => (
                input.wind_height_m.max(input.canopy_top_height_m + 1.0),
                input
                    .temperature_height_m
                    .max(input.canopy_top_height_m + 1.0),
                input.humidity_height_m.max(input.canopy_top_height_m + 1.0),
            ),
            ObservationHeightMode::RelativeToCanopy => (
                input.canopy_top_height_m + input.wind_height_m,
                input.canopy_top_height_m + input.temperature_height_m,
                input.canopy_top_height_m + input.humidity_height_m,
            ),
        };
    let reference_height = wind_height - displacement;
    let mut canopy_air_temperature =
        0.5 * (input.ground_temperature_k + input.reference_air_temperature_k);
    let mut canopy_air_humidity =
        0.5 * (input.reference_specific_humidity + input.ground_specific_humidity);
    let mut canopy_air_co2 = input.atmospheric_co2_pa;
    let reference_wind = input
        .eastward_wind_m_s
        .hypot(input.northward_wind_m_s)
        .max(0.1);
    let mut temperature_difference = input.reference_air_temperature_k - canopy_air_temperature;
    let mut humidity_difference = input.reference_specific_humidity - canopy_air_humidity;
    let virtual_temperature_difference = temperature_difference
        * (1.0 + 0.61 * input.reference_specific_humidity)
        + 0.61 * input.potential_temperature_k * humidity_difference;
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: reference_wind,
        potential_temperature_k: input.potential_temperature_k,
        reference_temperature_k: input.reference_air_temperature_k,
        virtual_potential_temperature_k: input.virtual_potential_temperature_k,
        temperature_difference_k: temperature_difference,
        humidity_difference_kg_kg: humidity_difference,
        virtual_temperature_difference_k: virtual_temperature_difference,
        reference_height_m: reference_height,
        momentum_roughness_m: z0mv,
    })?;
    let mut stability_wind = initial.stability_adjusted_wind_m_s;
    let mut obukhov = initial.obukhov_length_m;
    let mut prior_obukhov = 0.0;
    let mut obukhov_sign_changes = 0;
    let mut prior_temperature_change = 0.0;
    let mut prior_flux_change = 0.0;
    let mut prior_leaf_evaporation = 0.0;
    let mut previous_leaf_temperature = state.leaf_temperature_k;
    let mut dtl = [0.0; MAX_ITERATIONS + 2];
    let mut iteration = 1;
    let mut last = Iteration::default();

    while iteration <= MAX_ITERATIONS {
        previous_leaf_temperature = state.leaf_temperature_k;
        let profile = canopy_monin_obukhov_with_scheme(
            CanopyMoninObukhovInput {
                surface: MoninObukhovInput {
                    wind_height_m: wind_height,
                    temperature_height_m: temperature_height,
                    humidity_height_m: humidity_height,
                    displacement_height_m: displacement,
                    momentum_roughness_m: z0mv,
                    heat_roughness_m: z0mv,
                    moisture_roughness_m: z0mv,
                    obukhov_length_m: obukhov,
                    stability_adjusted_wind_m_s: stability_wind,
                },
                top_layer_displacement_m: displasink,
                top_layer_roughness_m: z0mv,
                canopy_top_height_m: input.canopy_top_height_m,
            },
            input.options.surface_layer_scheme,
        )?;
        let surface = profile.surface;
        let ram = 1.0 / (surface.friction_velocity_m_s.powi(2) / stability_wind);
        let rah = 1.0
            / (VON_KARMAN / (surface.heat - profile.heat_at_top_layer)
                * surface.friction_velocity_m_s);
        let raw = 1.0
            / (VON_KARMAN / (surface.moisture - profile.moisture_at_top_layer)
                * surface.friction_velocity_m_s);
        let z0hg = z0mg / (0.13 * (surface.friction_velocity_m_s * z0mg / 1.5e-5).powf(0.45)).exp();
        let z0qg = z0hg;
        let wind_at_top =
            surface.friction_velocity_m_s / VON_KARMAN * profile.momentum_at_canopy_top;
        let effective_wind = effective_canopy_wind(CanopyWindProfileInput {
            wind_at_canopy_top_m_s: wind_at_top,
            canopy_cover_fraction: 1.0,
            canopy_blend_weight: 1.0,
            attenuation_coefficient: attenuation,
            ground_momentum_roughness_m: z0mg,
            canopy_top_height_m: input.canopy_top_height_m,
            canopy_bottom_height_m: z0mg,
        })?;
        let leaf_boundary_resistance =
            1.0 / (0.01 * input.inverse_sqrt_leaf_dimension_m_neg_half * effective_wind.sqrt());
        let ktop =
            VON_KARMAN * (input.canopy_top_height_m - displacement) * surface.friction_velocity_m_s
                / profile.canopy_top_heat_similarity;
        let ground_to_canopy_resistance = canopy_diffusivity_resistance_analytic(
            CanopyDiffusivityProfileInput {
                diffusivity_at_canopy_top_m2_s: ktop,
                canopy_cover_fraction: 1.0,
                canopy_blend_weight: 1.0,
                attenuation_coefficient: attenuation,
                displacement_height_m: displacement / input.canopy_top_height_m,
                canopy_top_height_m: input.canopy_top_height_m,
                canopy_bottom_height_m: z0qg,
                obukhov_length_m: input.ground_obukhov_length_m,
                friction_velocity_m_s: surface.friction_velocity_m_s,
            },
            hsink,
            z0qg,
            z0qg,
        )?;
        let leaf_saturation =
            saturation_specific_humidity(state.leaf_temperature_k, input.surface_pressure_pa)?;
        let canopy_vapor_pressure =
            canopy_air_humidity * input.surface_pressure_pa / (0.622 + 0.378 * canopy_air_humidity);
        let sunlit_resistance = stomatal_resistance(
            input,
            StomataStep {
                leaf_temperature_k: state.leaf_temperature_k,
                leaf_boundary_resistance_s_m: leaf_boundary_resistance,
                absorbed_par_w_m2: input.sunlit_absorbed_par_w_m2,
                soil_water_stress: input.soil_water_stress_sunlit,
                canopy_scaling: cintsun,
                canopy_air_co2_pa: canopy_air_co2,
                canopy_vapor_pressure_pa: canopy_vapor_pressure,
                leaf_vapor_pressure_pa: leaf_saturation.vapor_pressure_pa,
            },
        )?;
        let shaded_resistance = stomatal_resistance(
            input,
            StomataStep {
                leaf_temperature_k: state.leaf_temperature_k,
                leaf_boundary_resistance_s_m: leaf_boundary_resistance,
                absorbed_par_w_m2: input.shaded_absorbed_par_w_m2,
                soil_water_stress: input.soil_water_stress_shaded,
                canopy_scaling: cintsha,
                canopy_air_co2_pa: canopy_air_co2,
                canopy_vapor_pressure_pa: canopy_vapor_pressure,
                leaf_vapor_pressure_pa: leaf_saturation.vapor_pressure_pa,
            },
        )?;
        let leaf_sunlit_resistance = sunlit_resistance.stomatal_resistance_s_m * laisun;
        let leaf_shaded_resistance = shaded_resistance.stomatal_resistance_s_m * laisha;
        let evaporation_sign = if leaf_saturation.specific_humidity > canopy_air_humidity {
            1.0
        } else {
            0.0
        };
        let canopy_air_heat_conductance = 1.0 / rah;
        let ground_heat_conductance = 1.0 / ground_to_canopy_resistance;
        let leaf_heat_conductance = lsai / leaf_boundary_resistance;
        let canopy_air_moisture_conductance = 1.0 / raw;
        let ground_moisture_conductance = if input.ground_specific_humidity < canopy_air_humidity {
            1.0 / ground_to_canopy_resistance
        } else if input.options.soil_resistance_is_conductance {
            input.soil_surface_resistance_s_m / ground_to_canopy_resistance
        } else {
            1.0 / (ground_to_canopy_resistance + input.soil_surface_resistance_s_m)
        };
        let leaf_moisture_conductance = (1.0 - evaporation_sign * (1.0 - fwet)) * lsai
            / leaf_boundary_resistance
            + (1.0 - fwet)
                * evaporation_sign
                * (laisun / (leaf_boundary_resistance + leaf_sunlit_resistance)
                    + laisha / (leaf_boundary_resistance + leaf_shaded_resistance));
        let heat_weight =
            1.0 / (canopy_air_heat_conductance + ground_heat_conductance + leaf_heat_conductance);
        let moisture_weight = 1.0
            / (canopy_air_moisture_conductance
                + ground_moisture_conductance
                + leaf_moisture_conductance);
        let air_heat_weight = canopy_air_heat_conductance * heat_weight;
        let ground_heat_weight = ground_heat_conductance * heat_weight;
        let leaf_heat_weight = leaf_heat_conductance * heat_weight;
        let air_moisture_weight = canopy_air_moisture_conductance * moisture_weight;
        let ground_moisture_weight = ground_moisture_conductance * moisture_weight;
        let leaf_moisture_weight = leaf_moisture_conductance * moisture_weight;
        let longwave_factor = 1.0 - input.canopy_longwave_gap_fraction;
        let (net_longwave, net_longwave_temperature_slope) =
            longwave(input, state.leaf_temperature_k, longwave_factor);
        let leaf_sensible_heat = input.air_density_kg_m3
            * AIR_HEAT_CAPACITY_J_KG_K
            * leaf_heat_conductance
            * ((air_heat_weight + ground_heat_weight) * state.leaf_temperature_k
                - air_heat_weight * input.reference_air_temperature_k
                - ground_heat_weight * input.ground_temperature_k);
        let leaf_sensible_temperature_slope = input.air_density_kg_m3
            * AIR_HEAT_CAPACITY_J_KG_K
            * leaf_heat_conductance
            * (air_heat_weight + ground_heat_weight);
        let humidity_gradient = (air_moisture_weight + ground_moisture_weight)
            * leaf_saturation.specific_humidity
            - air_moisture_weight * input.reference_specific_humidity
            - ground_moisture_weight * input.ground_specific_humidity;
        let mut transpiration = input.air_density_kg_m3
            * (1.0 - fwet)
            * evaporation_sign
            * (laisun / (leaf_boundary_resistance + leaf_sunlit_resistance)
                + laisha / (leaf_boundary_resistance + leaf_shaded_resistance))
            * humidity_gradient;
        let mut sunlit_transpiration =
            input.air_density_kg_m3 * (1.0 - fwet) * evaporation_sign * laisun
                / (leaf_boundary_resistance + leaf_sunlit_resistance)
                * humidity_gradient;
        let mut shaded_transpiration =
            input.air_density_kg_m3 * (1.0 - fwet) * evaporation_sign * laisha
                / (leaf_boundary_resistance + leaf_shaded_resistance)
                * humidity_gradient;
        let mut transpiration_temperature_slope = input.air_density_kg_m3
            * (1.0 - fwet)
            * evaporation_sign
            * (laisun / (leaf_boundary_resistance + leaf_sunlit_resistance)
                + laisha / (leaf_boundary_resistance + leaf_shaded_resistance))
            * (air_moisture_weight + ground_moisture_weight)
            * leaf_saturation.specific_humidity_temperature_slope_k;
        if transpiration >= input.transpiration_limit_kg_m2_s {
            let scale = if transpiration > 0.0 {
                input.transpiration_limit_kg_m2_s / transpiration
            } else {
                0.0
            };
            transpiration = input.transpiration_limit_kg_m2_s;
            sunlit_transpiration *= scale;
            shaded_transpiration *= scale;
            transpiration_temperature_slope = 0.0;
        }
        let mut wet_evaporation =
            input.air_density_kg_m3 * (1.0 - evaporation_sign * (1.0 - fwet)) * lsai
                / leaf_boundary_resistance
                * humidity_gradient;
        let mut wet_evaporation_temperature_slope =
            input.air_density_kg_m3 * (1.0 - evaporation_sign * (1.0 - fwet)) * lsai
                / leaf_boundary_resistance
                * (air_moisture_weight + ground_moisture_weight)
                * leaf_saturation.specific_humidity_temperature_slope_k;
        if wet_evaporation >= state.canopy_water.total_mm / input.time_step_seconds {
            wet_evaporation = state.canopy_water.total_mm / input.time_step_seconds;
            wet_evaporation_temperature_slope = 0.0;
        }
        let leaf_evaporation_unadjusted = transpiration + wet_evaporation;
        let mut leaf_evaporation = leaf_evaporation_unadjusted;
        let leaf_evaporation_temperature_slope =
            transpiration_temperature_slope + wet_evaporation_temperature_slope;
        let mut evaporation_imbalance = 0.0;
        if leaf_evaporation * prior_leaf_evaporation < 0.0 {
            evaporation_imbalance = -0.9 * leaf_evaporation;
            leaf_evaporation *= 0.1;
        }
        let denominator = clai / input.time_step_seconds - net_longwave_temperature_slope
            + leaf_sensible_temperature_slope
            + LATENT_HEAT_VAPORIZATION_J_KG * leaf_evaporation_temperature_slope
            + WATER_HEAT_CAPACITY_J_KG_K * input.intercepted_rain_kg_m2_s
            + ICE_HEAT_CAPACITY_J_KG_K * input.intercepted_snow_kg_m2_s;
        ensure!(
            denominator.is_finite() && denominator != 0.0,
            "leaf energy denominator is invalid"
        );
        dtl[iteration] = (input.canopy_absorbed_solar_w_m2 + net_longwave
            - leaf_sensible_heat
            - LATENT_HEAT_VAPORIZATION_J_KG * leaf_evaporation
            + WATER_HEAT_CAPACITY_J_KG_K
                * input.intercepted_rain_kg_m2_s
                * (input.precipitation_temperature_k - state.leaf_temperature_k)
            + ICE_HEAT_CAPACITY_J_KG_K
                * input.intercepted_snow_kg_m2_s
                * (input.precipitation_temperature_k - state.leaf_temperature_k))
            / denominator;
        let unbounded_temperature_change = dtl[iteration];
        if dtl[iteration].abs() > MAX_TEMPERATURE_STEP_K {
            dtl[iteration] = MAX_TEMPERATURE_STEP_K * dtl[iteration].signum();
        }
        if iteration >= 2 && dtl[iteration - 1] * dtl[iteration] <= 0.0 {
            dtl[iteration] = 0.5 * (dtl[iteration - 1] + dtl[iteration]);
        }
        state.leaf_temperature_k = previous_leaf_temperature + dtl[iteration];
        let temperature_change = dtl[iteration].abs();
        let flux_change = (dtl[iteration].powi(2)
            * (net_longwave_temperature_slope.powi(2)
                + leaf_sensible_temperature_slope.powi(2)
                + (LATENT_HEAT_VAPORIZATION_J_KG * leaf_evaporation_temperature_slope).powi(2)))
        .sqrt();
        let updated_saturation =
            saturation_specific_humidity(state.leaf_temperature_k, input.surface_pressure_pa)?;
        canopy_air_temperature = air_heat_weight * input.reference_air_temperature_k
            + ground_heat_weight * input.ground_temperature_k
            + leaf_heat_weight * state.leaf_temperature_k;
        canopy_air_humidity = air_moisture_weight * input.reference_specific_humidity
            + ground_moisture_weight * input.ground_specific_humidity
            + leaf_moisture_weight * updated_saturation.specific_humidity;
        let pressure_conversion = 44.6 * 273.16 * input.surface_pressure_pa / 1.013e5;
        let air_conductance = 1.0 / raw * pressure_conversion / input.reference_air_temperature_k;
        canopy_air_co2 = input.atmospheric_co2_pa
            - 1.37 * input.surface_pressure_pa / air_conductance.max(0.446)
                * (sunlit_resistance.assimilation_mol_m2_s
                    + shaded_resistance.assimilation_mol_m2_s
                    - sunlit_resistance.respiration_mol_m2_s
                    - shaded_resistance.respiration_mol_m2_s
                    - 0.22e-6);
        temperature_difference = input.reference_air_temperature_k - canopy_air_temperature;
        humidity_difference = input.reference_specific_humidity - canopy_air_humidity;
        let temperature_scale =
            VON_KARMAN / (surface.heat - profile.heat_at_top_layer) * temperature_difference;
        let humidity_scale =
            VON_KARMAN / (surface.moisture - profile.moisture_at_top_layer) * humidity_difference;
        let virtual_temperature_scale = temperature_scale
            * (1.0 + 0.61 * input.reference_specific_humidity)
            + 0.61 * input.potential_temperature_k * humidity_scale;
        let mut zeta = reference_height * VON_KARMAN * GRAVITY_M_S2 * virtual_temperature_scale
            / (surface.friction_velocity_m_s.powi(2) * input.virtual_potential_temperature_k);
        zeta = if zeta >= 0.0 {
            zeta.clamp(1.0e-6, 2.0)
        } else {
            zeta.clamp(-100.0, -1.0e-6)
        };
        obukhov = reference_height / zeta;
        stability_wind = if zeta >= 0.0 {
            reference_wind.max(0.1)
        } else {
            let boundary_height = match input.options.surface_layer_scheme {
                SurfaceLayerScheme::Standard => 1000.0,
                SurfaceLayerScheme::LargeEddy {
                    boundary_layer_height_m,
                } => (5.0 * wind_height).max(boundary_layer_height_m),
            };
            let convective_velocity = (-GRAVITY_M_S2
                * surface.friction_velocity_m_s
                * virtual_temperature_scale
                * boundary_height
                / input.virtual_potential_temperature_k)
                .powf(1.0 / 3.0);
            (reference_wind.powi(2) + convective_velocity.powi(2)).sqrt()
        };
        if prior_obukhov * obukhov < 0.0 {
            obukhov_sign_changes += 1;
        }
        if obukhov_sign_changes >= 4 {
            obukhov = reference_height / -0.01;
        }
        prior_obukhov = obukhov;
        last = Iteration {
            ram,
            raw,
            surface,
            top_heat: profile.heat_at_top_layer,
            top_moisture: profile.moisture_at_top_layer,
            heat_at_2m: surface.heat_at_2m,
            moisture_at_2m: surface.moisture_at_2m,
            zeta,
            leaf_sensible_heat,
            leaf_sensible_temperature_slope,
            leaf_evaporation_unadjusted,
            leaf_evaporation_temperature_slope,
            transpiration,
            transpiration_temperature_slope,
            sunlit_transpiration,
            shaded_transpiration,
            wet_evaporation,
            wet_evaporation_temperature_slope,
            net_longwave,
            net_longwave_temperature_slope,
            unbounded_temperature_change,
            evaporation_imbalance,
            canopy_air_temperature,
            canopy_air_humidity,
            air_heat_weight,
            ground_heat_weight,
            leaf_heat_weight,
            air_moisture_weight,
            ground_moisture_weight,
            leaf_moisture_weight,
            ground_heat_conductance,
            ground_moisture_conductance,
            sunlit_resistance,
            shaded_resistance,
            leaf_sunlit_resistance,
            leaf_shaded_resistance,
        };
        iteration += 1;
        if iteration > MIN_ITERATIONS {
            prior_leaf_evaporation = leaf_evaporation;
            if temperature_change.max(prior_temperature_change) < TEMPERATURE_TOLERANCE_K
                && flux_change.max(prior_flux_change) < FLUX_TOLERANCE_W_M2
            {
                break;
            }
        }
        prior_temperature_change = temperature_change;
        prior_flux_change = flux_change;
    }

    let final_temperature_change = dtl[iteration - 1];
    let leaf_sensible_heat = last.leaf_sensible_heat
        + last.leaf_sensible_temperature_slope * final_temperature_change
        + (last.unbounded_temperature_change - final_temperature_change)
            * (clai / input.time_step_seconds - last.net_longwave_temperature_slope
                + last.leaf_sensible_temperature_slope
                + LATENT_HEAT_VAPORIZATION_J_KG * last.leaf_evaporation_temperature_slope
                + WATER_HEAT_CAPACITY_J_KG_K * input.intercepted_rain_kg_m2_s
                + ICE_HEAT_CAPACITY_J_KG_K * input.intercepted_snow_kg_m2_s)
        + LATENT_HEAT_VAPORIZATION_J_KG * last.evaporation_imbalance;
    let mut transpiration =
        last.transpiration + last.transpiration_temperature_slope * final_temperature_change;
    let mut wet_evaporation =
        last.wet_evaporation + last.wet_evaporation_temperature_slope * final_temperature_change;
    let leaf_evaporation = last.leaf_evaporation_unadjusted
        + last.leaf_evaporation_temperature_slope * final_temperature_change;
    let wet_evaporation_limit = state.canopy_water.total_mm / input.time_step_seconds;
    let excessive_wet_evaporation = (wet_evaporation - wet_evaporation_limit).max(0.0);
    wet_evaporation = wet_evaporation.min(wet_evaporation_limit);
    let leaf_evaporation = leaf_evaporation - excessive_wet_evaporation;
    let leaf_sensible_heat =
        leaf_sensible_heat + LATENT_HEAT_VAPORIZATION_J_KG * excessive_wet_evaporation;
    let sunlit_transpiration = last.sunlit_transpiration;
    let shaded_transpiration = last.shaded_transpiration;
    state.canopy_water.total_mm =
        (state.canopy_water.total_mm - wet_evaporation * input.time_step_seconds).max(0.0);
    let wet_snow_fraction = update_canopy_water(input, state, wet_evaporation)?;
    let ground_sensible_heat = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * (input.ground_temperature_k - last.canopy_air_temperature);
    let soil_sensible_heat = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * ((1.0 - last.ground_heat_weight) * input.soil_surface_temperature_k
            - last.air_heat_weight * input.reference_air_temperature_k
            - last.leaf_heat_weight * state.leaf_temperature_k);
    let snow_sensible_heat = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * ((1.0 - last.ground_heat_weight) * input.snow_surface_temperature_k
            - last.air_heat_weight * input.reference_air_temperature_k
            - last.leaf_heat_weight * state.leaf_temperature_k);
    let ground_evaporation = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * (input.ground_specific_humidity - last.canopy_air_humidity);
    let soil_evaporation = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * ((1.0 - last.ground_moisture_weight) * input.soil_specific_humidity
            - last.air_moisture_weight * input.reference_specific_humidity
            - last.leaf_moisture_weight
                * saturation_specific_humidity(
                    state.leaf_temperature_k,
                    input.surface_pressure_pa,
                )?
                .specific_humidity);
    let snow_evaporation = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * ((1.0 - last.ground_moisture_weight) * input.snow_specific_humidity
            - last.air_moisture_weight * input.reference_specific_humidity
            - last.leaf_moisture_weight
                * saturation_specific_humidity(
                    state.leaf_temperature_k,
                    input.surface_pressure_pa,
                )?
                .specific_humidity);
    let downward_longwave = input.canopy_longwave_gap_fraction * input.atmospheric_longwave_w_m2
        + STEFAN_BOLTZMANN
            * (1.0 - input.canopy_longwave_gap_fraction)
            * previous_leaf_temperature.powi(3)
            * (previous_leaf_temperature + 4.0 * final_temperature_change);
    let upward_longwave = upward_longwave(
        input,
        previous_leaf_temperature,
        final_temperature_change,
        1.0 - input.canopy_longwave_gap_fraction,
    );
    let precipitation_heat = WATER_HEAT_CAPACITY_J_KG_K
        * input.intercepted_rain_kg_m2_s
        * (input.precipitation_temperature_k - state.leaf_temperature_k)
        + ICE_HEAT_CAPACITY_J_KG_K
            * input.intercepted_snow_kg_m2_s
            * (input.precipitation_temperature_k - state.leaf_temperature_k);
    let canopy_heat_storage = clai / input.time_step_seconds * final_temperature_change;
    let energy_balance_error = input.canopy_absorbed_solar_w_m2
        + last.net_longwave
        + last.net_longwave_temperature_slope * final_temperature_change
        - leaf_sensible_heat
        - LATENT_HEAT_VAPORIZATION_J_KG * leaf_evaporation
        + precipitation_heat
        - canopy_heat_storage;
    let canopy_stomatal_resistance =
        1.0 / (laisun / last.leaf_sunlit_resistance + laisha / last.leaf_shaded_resistance);
    let bulk_richardson = (last.zeta * last.surface.friction_velocity_m_s.powi(2)
        / (VON_KARMAN.powi(2) / last.surface.heat * stability_wind.powi(2)))
    .min(5.0);
    transpiration = transpiration.max(0.0);
    Ok(LeafTemperatureOutput {
        wet_snow_fraction,
        eastward_stress_kg_m_s2: -input.air_density_kg_m3 * input.eastward_wind_m_s / last.ram,
        northward_stress_kg_m_s2: -input.air_density_kg_m3 * input.northward_wind_m_s / last.ram,
        ground_sensible_heat_w_m2: ground_sensible_heat,
        soil_sensible_heat_w_m2: soil_sensible_heat,
        snow_sensible_heat_w_m2: snow_sensible_heat,
        ground_evaporation_kg_m2_s: ground_evaporation,
        soil_evaporation_kg_m2_s: soil_evaporation,
        snow_evaporation_kg_m2_s: snow_evaporation,
        ground_flux_temperature_slope_w_m2_k: AIR_HEAT_CAPACITY_J_KG_K
            * input.air_density_kg_m3
            * last.ground_heat_conductance
            * (1.0 - last.ground_heat_weight)
            + input.air_density_kg_m3
                * last.ground_moisture_conductance
                * (1.0 - last.ground_moisture_weight)
                * input.ground_humidity_temperature_slope_k
                * input.ground_latent_heat_j_kg,
        ground_sensible_temperature_slope_w_m2_k: AIR_HEAT_CAPACITY_J_KG_K
            * input.air_density_kg_m3
            * last.ground_heat_conductance
            * (1.0 - last.ground_heat_weight),
        ground_latent_temperature_slope_kg_m2_s_k: input.air_density_kg_m3
            * last.ground_moisture_conductance
            * (1.0 - last.ground_moisture_weight)
            * input.ground_humidity_temperature_slope_k,
        air_temperature_2m_k: input.reference_air_temperature_k
            + VON_KARMAN / (last.surface.heat - last.top_heat)
                * temperature_difference
                * (last.heat_at_2m / VON_KARMAN - last.surface.heat / VON_KARMAN),
        air_specific_humidity_2m: input.reference_specific_humidity
            + VON_KARMAN / (last.surface.moisture - last.top_moisture)
                * humidity_difference
                * (last.moisture_at_2m / VON_KARMAN - last.surface.moisture / VON_KARMAN),
        canopy_stomatal_resistance_s_m: canopy_stomatal_resistance,
        assimilation_mol_m2_s: last.sunlit_resistance.assimilation_mol_m2_s
            + last.shaded_resistance.assimilation_mol_m2_s,
        respiration_mol_m2_s: last.sunlit_resistance.respiration_mol_m2_s
            + last.shaded_resistance.respiration_mol_m2_s,
        leaf_sensible_heat_w_m2: leaf_sensible_heat,
        leaf_evaporation_kg_m2_s: leaf_evaporation,
        transpiration_kg_m2_s: transpiration,
        sunlit_transpiration_kg_m2_s: sunlit_transpiration,
        shaded_transpiration_kg_m2_s: shaded_transpiration,
        sunlit_assimilation_mol_m2_s: last.sunlit_resistance.assimilation_mol_m2_s,
        shaded_assimilation_mol_m2_s: last.shaded_resistance.assimilation_mol_m2_s,
        downward_longwave_w_m2: downward_longwave,
        upward_longwave_w_m2: upward_longwave,
        precipitation_heat_w_m2: precipitation_heat,
        canopy_heat_storage_w_m2: canopy_heat_storage,
        momentum_roughness_m: z0mv,
        zol: last.zeta,
        bulk_richardson,
        friction_velocity_m_s: last.surface.friction_velocity_m_s,
        humidity_scale: VON_KARMAN / (last.surface.moisture - last.top_moisture)
            * humidity_difference,
        temperature_scale_k: VON_KARMAN / (last.surface.heat - last.top_heat)
            * temperature_difference,
        momentum_similarity: last.surface.momentum,
        heat_similarity: last.surface.heat,
        moisture_similarity: last.surface.moisture,
        reference_to_canopy_moisture_resistance_s_m: last.raw.max(0.0),
        energy_balance_error_w_m2: energy_balance_error,
        iterations: iteration - 1,
    })
}

#[derive(Debug, Clone, Copy)]
struct Iteration {
    ram: f64,
    raw: f64,
    surface: crate::MoninObukhovState,
    top_heat: f64,
    top_moisture: f64,
    heat_at_2m: f64,
    moisture_at_2m: f64,
    zeta: f64,
    leaf_sensible_heat: f64,
    leaf_sensible_temperature_slope: f64,
    leaf_evaporation_unadjusted: f64,
    leaf_evaporation_temperature_slope: f64,
    transpiration: f64,
    transpiration_temperature_slope: f64,
    sunlit_transpiration: f64,
    shaded_transpiration: f64,
    wet_evaporation: f64,
    wet_evaporation_temperature_slope: f64,
    net_longwave: f64,
    net_longwave_temperature_slope: f64,
    unbounded_temperature_change: f64,
    evaporation_imbalance: f64,
    canopy_air_temperature: f64,
    canopy_air_humidity: f64,
    air_heat_weight: f64,
    ground_heat_weight: f64,
    leaf_heat_weight: f64,
    air_moisture_weight: f64,
    ground_moisture_weight: f64,
    leaf_moisture_weight: f64,
    ground_heat_conductance: f64,
    ground_moisture_conductance: f64,
    sunlit_resistance: crate::StomataState,
    shaded_resistance: crate::StomataState,
    leaf_sunlit_resistance: f64,
    leaf_shaded_resistance: f64,
}

impl Default for Iteration {
    fn default() -> Self {
        Self {
            ram: 0.0,
            raw: 0.0,
            surface: crate::MoninObukhovState {
                friction_velocity_m_s: 0.0,
                heat_at_2m: 0.0,
                moisture_at_2m: 0.0,
                momentum_at_10m: 0.0,
                momentum: 0.0,
                heat: 0.0,
                moisture: 0.0,
            },
            top_heat: 0.0,
            top_moisture: 0.0,
            heat_at_2m: 0.0,
            moisture_at_2m: 0.0,
            zeta: 0.0,
            leaf_sensible_heat: 0.0,
            leaf_sensible_temperature_slope: 0.0,
            leaf_evaporation_unadjusted: 0.0,
            leaf_evaporation_temperature_slope: 0.0,
            transpiration: 0.0,
            transpiration_temperature_slope: 0.0,
            sunlit_transpiration: 0.0,
            shaded_transpiration: 0.0,
            wet_evaporation: 0.0,
            wet_evaporation_temperature_slope: 0.0,
            net_longwave: 0.0,
            net_longwave_temperature_slope: 0.0,
            unbounded_temperature_change: 0.0,
            evaporation_imbalance: 0.0,
            canopy_air_temperature: 0.0,
            canopy_air_humidity: 0.0,
            air_heat_weight: 0.0,
            ground_heat_weight: 0.0,
            leaf_heat_weight: 0.0,
            air_moisture_weight: 0.0,
            ground_moisture_weight: 0.0,
            leaf_moisture_weight: 0.0,
            ground_heat_conductance: 0.0,
            ground_moisture_conductance: 0.0,
            sunlit_resistance: crate::StomataState {
                assimilation_mol_m2_s: 0.0,
                respiration_mol_m2_s: 0.0,
                stomatal_resistance_s_m: 0.0,
            },
            shaded_resistance: crate::StomataState {
                assimilation_mol_m2_s: 0.0,
                respiration_mol_m2_s: 0.0,
                stomatal_resistance_s_m: 0.0,
            },
            leaf_sunlit_resistance: 0.0,
            leaf_shaded_resistance: 0.0,
        }
    }
}

fn canopy_scaling(direct: f64, diffuse: f64, lai: f64) -> [f64; 3] {
    [
        integrated_extinction(0.110 + direct, lai),
        integrated_extinction(direct + diffuse, lai),
        integrated_extinction(direct, lai),
    ]
}

fn integrated_extinction(extinction: f64, lai: f64) -> f64 {
    if extinction.abs() <= f64::EPSILON {
        lai
    } else {
        (1.0 - (-extinction * lai).exp()) / extinction
    }
}

struct StomataStep {
    leaf_temperature_k: f64,
    leaf_boundary_resistance_s_m: f64,
    absorbed_par_w_m2: f64,
    soil_water_stress: f64,
    canopy_scaling: [f64; 3],
    canopy_air_co2_pa: f64,
    canopy_vapor_pressure_pa: f64,
    leaf_vapor_pressure_pa: f64,
}

fn stomatal_resistance(
    input: LeafTemperatureInput,
    step: StomataStep,
) -> Result<crate::StomataState> {
    stomata(
        StomataInput {
            photosynthesis: LeafPhotosynthesisInput {
                biochemistry: LeafBiochemistry {
                    canopy_scaling: step.canopy_scaling,
                    ..input.biochemistry
                },
                leaf_temperature_k: step.leaf_temperature_k,
                oxygen_partial_pressure_pa: input.oxygen_partial_pressure_pa,
                absorbed_par_w_m2: step.absorbed_par_w_m2,
                air_pressure_pa: input.surface_pressure_pa,
                soil_water_stress: step.soil_water_stress,
                leaf_boundary_resistance_s_m: step.leaf_boundary_resistance_s_m,
            },
            atmospheric_co2_pa: input.atmospheric_co2_pa,
            canopy_air_co2_pa: step.canopy_air_co2_pa,
            canopy_air_vapor_pressure_pa: step.canopy_vapor_pressure_pa,
            leaf_saturation_vapor_pressure_pa: step.leaf_vapor_pressure_pa,
            wue_lambda: input.wue_lambda,
        },
        input.options.stomata,
    )
}

fn longwave(input: LeafTemperatureInput, leaf_temperature_k: f64, factor: f64) -> (f64, f64) {
    let ground_longwave = if input.options.split_soil_snow {
        (1.0 - input.snow_cover_fraction)
            * input.ground_emissivity
            * STEFAN_BOLTZMANN
            * input.soil_surface_temperature_k.powi(4)
            + input.snow_cover_fraction
                * input.ground_emissivity
                * STEFAN_BOLTZMANN
                * input.snow_surface_temperature_k.powi(4)
    } else {
        input.ground_emissivity * STEFAN_BOLTZMANN * input.ground_temperature_k.powi(4)
    };
    (
        (input.atmospheric_longwave_w_m2 - 2.0 * STEFAN_BOLTZMANN * leaf_temperature_k.powi(4)
            + ground_longwave)
            * factor
            + (1.0 - input.ground_emissivity)
                * input.canopy_longwave_gap_fraction
                * factor
                * input.atmospheric_longwave_w_m2
            + (1.0 - input.ground_emissivity)
                * (1.0 - input.canopy_longwave_gap_fraction)
                * factor
                * STEFAN_BOLTZMANN
                * leaf_temperature_k.powi(4),
        -8.0 * STEFAN_BOLTZMANN * leaf_temperature_k.powi(3) * factor
            + 4.0
                * (1.0 - input.ground_emissivity)
                * (1.0 - input.canopy_longwave_gap_fraction)
                * factor
                * STEFAN_BOLTZMANN
                * leaf_temperature_k.powi(3),
    )
}

fn upward_longwave(
    input: LeafTemperatureInput,
    previous_leaf_temperature_k: f64,
    leaf_temperature_change_k: f64,
    factor: f64,
) -> f64 {
    let ground_term = if input.options.split_soil_snow {
        (1.0 - input.snow_cover_fraction)
            * input.canopy_longwave_gap_fraction
            * input.ground_emissivity
            * input.soil_surface_temperature_k.powi(4)
            + input.snow_cover_fraction
                * input.canopy_longwave_gap_fraction
                * input.ground_emissivity
                * input.snow_surface_temperature_k.powi(4)
    } else {
        input.canopy_longwave_gap_fraction
            * input.ground_emissivity
            * input.ground_temperature_k.powi(4)
    };
    STEFAN_BOLTZMANN
        * (factor
            * previous_leaf_temperature_k.powi(3)
            * (previous_leaf_temperature_k + 4.0 * leaf_temperature_change_k)
            + ground_term)
        + (1.0 - input.ground_emissivity)
            * input.canopy_longwave_gap_fraction.powi(2)
            * input.atmospheric_longwave_w_m2
        + (1.0 - input.ground_emissivity)
            * input.canopy_longwave_gap_fraction
            * factor
            * STEFAN_BOLTZMANN
            * previous_leaf_temperature_k.powi(4)
        + 4.0
            * (1.0 - input.ground_emissivity)
            * input.canopy_longwave_gap_fraction
            * factor
            * STEFAN_BOLTZMANN
            * previous_leaf_temperature_k.powi(3)
            * leaf_temperature_change_k
}

fn update_canopy_water(
    input: LeafTemperatureInput,
    state: &mut LeafTemperatureState,
    wet_evaporation_kg_m2_s: f64,
) -> Result<f64> {
    if !input.options.vegetation_snow {
        let components = state.canopy_water.rain_mm + state.canopy_water.snow_mm;
        if components > 1.0e-10 {
            state.canopy_water.rain_mm *= state.canopy_water.total_mm / components;
            state.canopy_water.snow_mm = state.canopy_water.total_mm - state.canopy_water.rain_mm;
        } else if state.canopy_water.total_mm > 0.0 {
            if state.leaf_temperature_k > FREEZING_K {
                state.canopy_water.rain_mm = state.canopy_water.total_mm;
                state.canopy_water.snow_mm = 0.0;
            } else {
                state.canopy_water.rain_mm = 0.0;
                state.canopy_water.snow_mm = state.canopy_water.total_mm;
            }
        } else {
            state.canopy_water.rain_mm = 0.0;
            state.canopy_water.snow_mm = 0.0;
        }
        return Ok(0.0);
    }
    let lsai = input.leaf_area_index + input.stem_area_index;
    let mut wet_snow_fraction = if state.canopy_water.snow_mm > 0.0 {
        ((10.0 / (48.0 * lsai)) * state.canopy_water.snow_mm)
            .powf(0.666_666_666_666)
            .min(1.0)
    } else {
        0.0
    };
    if state.leaf_temperature_k > FREEZING_K {
        let evaporation = wet_evaporation_kg_m2_s.max(0.0);
        let dew = (-wet_evaporation_kg_m2_s).max(0.0);
        let mut sublimation = 0.0;
        let mut evaporation = evaporation;
        if evaporation > state.canopy_water.rain_mm / input.time_step_seconds {
            sublimation = evaporation - state.canopy_water.rain_mm / input.time_step_seconds;
            evaporation = state.canopy_water.rain_mm / input.time_step_seconds;
        }
        state.canopy_water.rain_mm += (dew - evaporation) * input.time_step_seconds;
        state.canopy_water.snow_mm =
            (state.canopy_water.snow_mm - sublimation * input.time_step_seconds).max(0.0);
    } else {
        let sublimation = wet_evaporation_kg_m2_s.max(0.0);
        let frost = (-wet_evaporation_kg_m2_s).max(0.0);
        let mut sublimation = sublimation;
        let mut evaporation = 0.0;
        if sublimation > state.canopy_water.snow_mm / input.time_step_seconds {
            evaporation = sublimation - state.canopy_water.snow_mm / input.time_step_seconds;
            sublimation = state.canopy_water.snow_mm / input.time_step_seconds;
        }
        state.canopy_water.rain_mm =
            (state.canopy_water.rain_mm - evaporation * input.time_step_seconds).max(0.0);
        state.canopy_water.snow_mm += (frost - sublimation) * input.time_step_seconds;
    }
    if state.canopy_water.snow_mm > 1.0e-6 && state.leaf_temperature_k > FREEZING_K {
        let melt = (state.canopy_water.snow_mm / input.time_step_seconds).min(
            (state.leaf_temperature_k - FREEZING_K)
                * ICE_HEAT_CAPACITY_J_KG_K
                * state.canopy_water.snow_mm
                / (input.time_step_seconds * FUSION_HEAT_J_KG),
        );
        state.canopy_water.snow_mm =
            (state.canopy_water.snow_mm - melt * input.time_step_seconds).max(0.0);
        state.canopy_water.rain_mm += melt * input.time_step_seconds;
        state.leaf_temperature_k =
            wet_snow_fraction * FREEZING_K + (1.0 - wet_snow_fraction) * state.leaf_temperature_k;
    }
    if state.canopy_water.rain_mm > 1.0e-6 && state.leaf_temperature_k < FREEZING_K {
        let freeze = (state.canopy_water.rain_mm / input.time_step_seconds).min(
            (FREEZING_K - state.leaf_temperature_k)
                * WATER_HEAT_CAPACITY_J_KG_K
                * state.canopy_water.rain_mm
                / (input.time_step_seconds * FUSION_HEAT_J_KG),
        );
        state.canopy_water.rain_mm =
            (state.canopy_water.rain_mm - freeze * input.time_step_seconds).max(0.0);
        state.canopy_water.snow_mm += freeze * input.time_step_seconds;
        state.leaf_temperature_k =
            wet_snow_fraction * FREEZING_K + (1.0 - wet_snow_fraction) * state.leaf_temperature_k;
    }
    state.canopy_water.total_mm = state.canopy_water.rain_mm + state.canopy_water.snow_mm;
    wet_snow_fraction = wet_snow_fraction.min(1.0);
    Ok(wet_snow_fraction)
}

fn validate(input: LeafTemperatureInput, state: LeafTemperatureState) -> Result<()> {
    let scalars = [
        input.time_step_seconds,
        input.maximum_dew_mm,
        input.leaf_area_index,
        input.stem_area_index,
        input.canopy_top_height_m,
        input.inverse_sqrt_leaf_dimension_m_neg_half,
        input.soil_water_stress_sunlit,
        input.soil_water_stress_shaded,
        input.wue_lambda,
        input.direct_extinction,
        input.diffuse_extinction,
        input.wind_height_m,
        input.temperature_height_m,
        input.humidity_height_m,
        input.eastward_wind_m_s,
        input.northward_wind_m_s,
        input.reference_air_temperature_k,
        input.potential_temperature_k,
        input.virtual_potential_temperature_k,
        input.reference_specific_humidity,
        input.surface_pressure_pa,
        input.air_density_kg_m3,
        input.sunlit_absorbed_par_w_m2,
        input.shaded_absorbed_par_w_m2,
        input.canopy_absorbed_solar_w_m2,
        input.atmospheric_longwave_w_m2,
        input.sunlit_fraction,
        input.canopy_longwave_gap_fraction,
        input.oxygen_partial_pressure_pa,
        input.atmospheric_co2_pa,
        input.soil_roughness_m,
        input.snow_roughness_m,
        input.snow_cover_fraction,
        input.ground_obukhov_length_m,
        input.transpiration_limit_kg_m2_s,
        input.ground_temperature_k,
        input.soil_surface_temperature_k,
        input.snow_surface_temperature_k,
        input.ground_specific_humidity,
        input.soil_specific_humidity,
        input.snow_specific_humidity,
        input.ground_humidity_temperature_slope_k,
        input.soil_surface_resistance_s_m,
        input.ground_emissivity,
        input.precipitation_temperature_k,
        input.intercepted_rain_kg_m2_s,
        input.intercepted_snow_kg_m2_s,
        input.ground_latent_heat_j_kg,
        state.leaf_temperature_k,
        state.canopy_water.total_mm,
        state.canopy_water.rain_mm,
        state.canopy_water.snow_mm,
    ];
    ensure!(
        scalars.iter().all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && input.maximum_dew_mm > 0.0
            && input.leaf_area_index > 0.001
            && input.stem_area_index >= 0.0
            && input.canopy_top_height_m > input.soil_roughness_m.max(input.snow_roughness_m)
            && input.inverse_sqrt_leaf_dimension_m_neg_half > 0.0
            && (0.0..=1.0).contains(&input.soil_water_stress_sunlit)
            && (0.0..=1.0).contains(&input.soil_water_stress_shaded)
            && input.wue_lambda > 0.0
            && input.direct_extinction > 0.0
            && input.diffuse_extinction > 0.0
            && input.reference_air_temperature_k > 0.0
            && input.potential_temperature_k > 0.0
            && input.virtual_potential_temperature_k > 0.0
            && (0.0..1.0).contains(&input.reference_specific_humidity)
            && input.surface_pressure_pa > 0.0
            && input.air_density_kg_m3 > 0.0
            && (0.0..1.0).contains(&input.sunlit_fraction)
            && input.sunlit_fraction > 0.0
            && input.sunlit_fraction < 1.0
            && (0.0..=1.0).contains(&input.canopy_longwave_gap_fraction)
            && input.oxygen_partial_pressure_pa >= 0.0
            && input.atmospheric_co2_pa >= 0.0
            && input.soil_roughness_m > 0.0
            && input.snow_roughness_m > 0.0
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.ground_obukhov_length_m != 0.0
            && input.transpiration_limit_kg_m2_s >= 0.0
            && (0.0..1.0).contains(&input.ground_specific_humidity)
            && (0.0..1.0).contains(&input.soil_specific_humidity)
            && (0.0..1.0).contains(&input.snow_specific_humidity)
            && input.soil_surface_resistance_s_m >= 0.0
            && (0.0..=1.0).contains(&input.ground_emissivity)
            && input.intercepted_rain_kg_m2_s >= 0.0
            && input.intercepted_snow_kg_m2_s >= 0.0
            && input.ground_latent_heat_j_kg > 0.0
            && state.leaf_temperature_k > 0.0
            && state.canopy_water.total_mm >= 0.0
            && state.canopy_water.rain_mm >= 0.0
            && state.canopy_water.snow_mm >= 0.0,
        "leaf-temperature inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "leaf_temperature_tests.rs"]
mod leaf_temperature_tests;
