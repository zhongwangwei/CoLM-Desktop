//! Ground-temperature conduction and phase change from `MOD_GroundTemperature.F90`.
//!
//! The state is packed top-to-bottom: leading snow layers, followed by soil.
//! This keeps the solver independent of Fortran's negative snow indices while
//! directly sharing `soil_thermal_properties`, `solve_tridiagonal`, and
//! `phase_change` with the rest of the Rust model.

use anyhow::{ensure, Result};

use crate::{
    phase_change, soil_thermal_properties, solve_tridiagonal, PhaseChangeInput, PhaseChangeState,
    SoilHydraulicModel, SoilThermalInput, ThermalConductivityScheme,
};

const WATER_DENSITY_KG_M3: f64 = 1000.0;
const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const AIR_THERMAL_CONDUCTIVITY_W_M_K: f64 = 0.023;
const ICE_THERMAL_CONDUCTIVITY_W_M_K: f64 = 2.290;
const STEFAN_BOLTZMANN_W_M2_K4: f64 = 5.67e-8;

/// Inputs to one `MOD_GroundTemperature:GroundTemperature` update.
#[derive(Debug, Clone, Copy)]
pub struct GroundTemperatureInput<'a> {
    pub patch_type: i32,
    pub is_dry_lake: bool,
    pub time_step_seconds: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    /// One static soil-thermal input per soil layer. Its temperature and water
    /// fractions are replaced from the packed dynamic state for this update.
    pub soil_thermal_inputs: &'a [SoilThermalInput],
    pub soil_porosity: &'a [f64],
    pub soil_residual_water: &'a [f64],
    pub soil_suction_mm: &'a [f64],
    pub soil_hydraulic_model: &'a [SoilHydraulicModel],
    /// Leading snow layers followed by soil layers.
    pub snow_layers: usize,
    pub layer_thickness_m: &'a [f64],
    pub node_depth_m: &'a [f64],
    /// One more value than `node_depth_m`, from the top to the bottom interface.
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub snow_cover_fraction: f64,
    pub use_split_soil_snow: bool,
    /// `Some` selects the SNICAR branch and supplies absorption for each packed
    /// layer; `None` selects the standard branch.
    pub snow_layer_absorption_w_m2: Option<&'a [f64]>,
    pub absorbed_ground_shortwave_w_m2: f64,
    pub absorbed_soil_shortwave_w_m2: f64,
    pub absorbed_snow_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    pub sensible_ground_w_m2: f64,
    pub sensible_soil_w_m2: f64,
    pub sensible_snow_w_m2: f64,
    pub evaporation_ground_kg_m2_s: f64,
    pub evaporation_soil_kg_m2_s: f64,
    pub evaporation_snow_kg_m2_s: f64,
    pub ground_flux_temperature_derivative_w_m2_k: f64,
    pub vaporization_heat_j_kg: f64,
    pub ground_emissivity: f64,
    pub rain_on_ground_kg_m2_s: f64,
    pub snow_on_ground_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub ground_temperature_k: f64,
    pub soil_surface_temperature_k: f64,
    pub snow_surface_temperature_k: f64,
    pub supercool_water: bool,
}

/// Dynamic state and diagnostic fields returned by [`ground_temperature`].
#[derive(Debug, Clone, PartialEq)]
pub struct GroundTemperatureState {
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub snow_melt_rate_kg_m2_s: f64,
    pub latent_heat_flux_w_m2: f64,
    pub phase_flag: Vec<i32>,
    pub thaw_mass_kg_m2: Vec<f64>,
    pub freeze_mass_kg_m2: Vec<f64>,
    /// Snow-only positive freezing rate, in top-to-bottom snow-layer order.
    pub snow_freezing_rate_kg_m2_s: Vec<f64>,
    pub layer_factor_seconds_per_j_m2_k: Vec<f64>,
    pub interface_conductivity_w_m_k: Vec<f64>,
}

/// Ports `MOD_GroundTemperature:GroundTemperature` as a pure packed-column
/// update. The result may be supplied directly to the shared hydrology,
/// snow, and restart paths without a second phase-change implementation.
pub fn ground_temperature(input: GroundTemperatureInput<'_>) -> Result<GroundTemperatureState> {
    let layers = validate(input)?;
    let soil_offset = input.snow_layers;
    let use_snicar = input.snow_layer_absorption_w_m2.is_some();
    let previous_temperature = input.temperature_k.to_vec();
    let snow_ice_before = input.ice_water_kg_m2[..input.snow_layers].to_vec();

    let (heat_capacity, mut conductivity) = layer_thermal_properties(input)?;
    let mut layer_capacity = heat_capacity;
    if input.snow_layers == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        layer_capacity[0] += ICE_HEAT_CAPACITY_J_KG_K * input.snow_water_equivalent_kg_m2;
    }
    ensure!(
        layer_capacity
            .iter()
            .all(|value| *value > 0.0 && value.is_finite()),
        "ground-temperature layer heat capacities must be positive"
    );

    let mut interface_conductivity = vec![0.0; layers];
    for layer in 0..layers - 1 {
        let interface = layer + 1;
        interface_conductivity[layer] = if layer + 1 == input.snow_layers
            && input.node_depth_m[layer + 1] - input.interface_depth_m[interface]
                < input.interface_depth_m[interface] - input.node_depth_m[layer]
        {
            let harmonic = 2.0 * conductivity[layer] * conductivity[layer + 1]
                / (conductivity[layer] + conductivity[layer + 1]);
            harmonic.max(0.5 * conductivity[layer + 1])
        } else {
            conductivity[layer]
                * conductivity[layer + 1]
                * (input.node_depth_m[layer + 1] - input.node_depth_m[layer])
                / (conductivity[layer]
                    * (input.node_depth_m[layer + 1] - input.interface_depth_m[interface])
                    + conductivity[layer + 1]
                        * (input.interface_depth_m[interface] - input.node_depth_m[layer]))
        };
    }
    conductivity[layers - 1] = 0.0;

    let (surface_heat_flux, soil_heat_flux, snow_heat_flux, flux_derivative) =
        surface_fluxes(input, use_snicar)?;
    let mut factor = vec![0.0; layers];
    factor[0] = input.time_step_seconds / layer_capacity[0] * input.layer_thickness_m[0]
        / (0.5
            * (input.node_depth_m[0] - input.interface_depth_m[0]
                + input.surface_temperature_factor
                    * (input.node_depth_m[1] - input.interface_depth_m[0])));
    for layer in 1..layers {
        factor[layer] = input.time_step_seconds / layer_capacity[layer];
    }
    ensure!(
        factor.iter().all(|value| *value > 0.0 && value.is_finite()),
        "ground-temperature layer factors must be positive"
    );

    let before_flux = interface_fluxes(
        &interface_conductivity,
        input.node_depth_m,
        input.temperature_k,
    )?;
    let (subdiagonal, diagonal, superdiagonal, rhs) = temperature_system(
        input,
        &factor,
        &interface_conductivity,
        &before_flux,
        surface_heat_flux,
        soil_heat_flux,
        snow_heat_flux,
        flux_derivative,
        use_snicar,
    );
    let solved_temperature = solve_tridiagonal(&subdiagonal, &diagonal, &superdiagonal, &rhs)
        .map_err(anyhow::Error::msg)?;
    let after_flux = interface_fluxes(
        &interface_conductivity,
        input.node_depth_m,
        &solved_temperature,
    )?;
    let residual_heat_flux =
        residual_heat_fluxes(input.crank_nicolson_factor, &before_flux, &after_flux);
    let phase = phase_change(PhaseChangeInput {
        patch_type: input.patch_type,
        is_dry_lake: input.is_dry_lake,
        time_step_seconds: input.time_step_seconds,
        fact_seconds_per_j_m2_k: &factor,
        residual_heat_flux_w_m2: &residual_heat_flux,
        snow_layer_absorption_w_m2: input.snow_layer_absorption_w_m2,
        surface_heat_flux_w_m2: surface_heat_flux,
        soil_heat_flux_w_m2: soil_heat_flux,
        snow_heat_flux_w_m2: snow_heat_flux,
        snow_cover_fraction: input.snow_cover_fraction,
        surface_heat_flux_temperature_derivative_w_m2_k: flux_derivative,
        previous_temperature_k: &previous_temperature,
        temperature_k: &solved_temperature,
        liquid_water_kg_m2: input.liquid_water_kg_m2,
        ice_water_kg_m2: input.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_layers: input.snow_layers,
        split_soil_snow: input.use_split_soil_snow,
        supercool_water: input.supercool_water,
        soil_layer_thickness_m: &input.layer_thickness_m[soil_offset..],
        soil_porosity: input.soil_porosity,
        soil_residual_water: input.soil_residual_water,
        soil_suction_mm: input.soil_suction_mm,
        soil_hydraulic_model: input.soil_hydraulic_model,
    })?;
    Ok(state_from_phase(
        phase,
        snow_ice_before,
        input.time_step_seconds,
        factor,
        interface_conductivity,
    ))
}

fn layer_thermal_properties(input: GroundTemperatureInput<'_>) -> Result<(Vec<f64>, Vec<f64>)> {
    let layers = input.temperature_k.len();
    let mut heat_capacity = vec![0.0; layers];
    let mut conductivity = vec![0.0; layers];
    for soil in 0..input.soil_thermal_inputs.len() {
        let layer = input.snow_layers + soil;
        let mut thermal = input.soil_thermal_inputs[soil];
        thermal.temperature_k = input.temperature_k[layer];
        thermal.liquid_volume_fraction = input.liquid_water_kg_m2[layer]
            / (input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3);
        thermal.ice_volume_fraction =
            input.ice_water_kg_m2[layer] / (input.layer_thickness_m[layer] * ICE_DENSITY_KG_M3);
        let properties = soil_thermal_properties(thermal, input.thermal_conductivity_scheme)?;
        heat_capacity[layer] = properties.heat_capacity_j_m3_k * input.layer_thickness_m[layer];
        conductivity[layer] = properties.conductivity_w_m_k;
    }
    for layer in 0..input.snow_layers {
        heat_capacity[layer] = WATER_HEAT_CAPACITY_J_KG_K * input.liquid_water_kg_m2[layer]
            + ICE_HEAT_CAPACITY_J_KG_K * input.ice_water_kg_m2[layer];
        let density = (input.liquid_water_kg_m2[layer] + input.ice_water_kg_m2[layer])
            / input.layer_thickness_m[layer];
        conductivity[layer] = AIR_THERMAL_CONDUCTIVITY_W_M_K
            + (7.75e-5 * density + 1.105e-6 * density * density)
                * (ICE_THERMAL_CONDUCTIVITY_W_M_K - AIR_THERMAL_CONDUCTIVITY_W_M_K);
    }
    Ok((heat_capacity, conductivity))
}

fn surface_fluxes(
    input: GroundTemperatureInput<'_>,
    use_snicar: bool,
) -> Result<(f64, f64, f64, f64)> {
    let precipitation_heat = |temperature: f64| {
        WATER_HEAT_CAPACITY_J_KG_K
            * input.rain_on_ground_kg_m2_s
            * (input.precipitation_temperature_k - temperature)
            + ICE_HEAT_CAPACITY_J_KG_K
                * input.snow_on_ground_kg_m2_s
                * (input.precipitation_temperature_k - temperature)
    };
    let snow_top_absorption = input
        .snow_layer_absorption_w_m2
        .map(|values| values[0])
        .unwrap_or(0.0);
    let mut surface = if use_snicar && input.snow_layers > 0 {
        snow_top_absorption + input.absorbed_soil_shortwave_w_m2
    } else {
        input.absorbed_ground_shortwave_w_m2
    } + input.downward_longwave_w_m2 * input.ground_emissivity
        - (input.sensible_ground_w_m2
            + input.evaporation_ground_kg_m2_s * input.vaporization_heat_j_kg)
        + precipitation_heat(input.ground_temperature_k);
    let derivative = -input.ground_flux_temperature_derivative_w_m2_k
        - 4.0
            * input.ground_emissivity
            * STEFAN_BOLTZMANN_W_M2_K4
            * input.ground_temperature_k.powi(3)
        - WATER_HEAT_CAPACITY_J_KG_K * input.rain_on_ground_kg_m2_s
        - ICE_HEAT_CAPACITY_J_KG_K * input.snow_on_ground_kg_m2_s;
    if !input.use_split_soil_snow {
        surface -=
            input.ground_emissivity * STEFAN_BOLTZMANN_W_M2_K4 * input.ground_temperature_k.powi(4);
        return Ok((surface, 0.0, 0.0, derivative));
    }

    surface -= input.snow_cover_fraction
        * input.ground_emissivity
        * STEFAN_BOLTZMANN_W_M2_K4
        * input.snow_surface_temperature_k.powi(4)
        + (1.0 - input.snow_cover_fraction)
            * input.ground_emissivity
            * STEFAN_BOLTZMANN_W_M2_K4
            * input.soil_surface_temperature_k.powi(4);
    let soil = (input.downward_longwave_w_m2 * input.ground_emissivity
        - input.ground_emissivity
            * STEFAN_BOLTZMANN_W_M2_K4
            * input.soil_surface_temperature_k.powi(4)
        - (input.sensible_soil_w_m2
            + input.evaporation_soil_kg_m2_s * input.vaporization_heat_j_kg)
        + precipitation_heat(input.soil_surface_temperature_k))
        * (1.0 - input.snow_cover_fraction)
        + input.absorbed_soil_shortwave_w_m2;
    let snow_absorption = if use_snicar && input.snow_layers > 0 {
        snow_top_absorption
    } else {
        input.absorbed_snow_shortwave_w_m2
    };
    let snow = (input.downward_longwave_w_m2 * input.ground_emissivity
        - input.ground_emissivity
            * STEFAN_BOLTZMANN_W_M2_K4
            * input.snow_surface_temperature_k.powi(4)
        - (input.sensible_snow_w_m2
            + input.evaporation_snow_kg_m2_s * input.vaporization_heat_j_kg)
        + precipitation_heat(input.snow_surface_temperature_k))
        * input.snow_cover_fraction
        + snow_absorption;
    ensure!(
        (input.absorbed_soil_shortwave_w_m2 + input.absorbed_snow_shortwave_w_m2
            - input.absorbed_ground_shortwave_w_m2)
            .abs()
            <= 1.0e-6
            && (soil + snow - surface).abs() <= 1.0e-6,
        "split soil and snow surface fluxes are not energy-consistent"
    );
    Ok((surface, soil, snow, derivative))
}

#[allow(clippy::too_many_arguments)]
fn temperature_system(
    input: GroundTemperatureInput<'_>,
    factor: &[f64],
    conductivity: &[f64],
    flux: &[f64],
    surface_heat_flux: f64,
    soil_heat_flux: f64,
    snow_heat_flux: f64,
    derivative: f64,
    use_snicar: bool,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let layers = input.temperature_k.len();
    let mut sub = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut super_ = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    let implicit = 1.0 - input.crank_nicolson_factor;
    let top_distance = input.node_depth_m[1] - input.node_depth_m[0];
    super_[0] = -implicit * factor[0] * conductivity[0] / top_distance;
    if input.snow_layers > 0 && input.use_split_soil_snow {
        diagonal[0] = 1.0 + implicit * factor[0] * conductivity[0] / top_distance
            - factor[0] * input.snow_cover_fraction * derivative;
        rhs[0] = input.temperature_k[0]
            + factor[0]
                * (snow_heat_flux
                    - input.snow_cover_fraction * derivative * input.temperature_k[0]
                    + input.crank_nicolson_factor * flux[0]);
    } else {
        diagonal[0] =
            1.0 + implicit * factor[0] * conductivity[0] / top_distance - factor[0] * derivative;
        rhs[0] = input.temperature_k[0]
            + factor[0]
                * (surface_heat_flux - derivative * input.temperature_k[0]
                    + input.crank_nicolson_factor * flux[0]);
    }
    for layer in 1..layers - 1 {
        let lower_distance = input.node_depth_m[layer] - input.node_depth_m[layer - 1];
        let upper_distance = input.node_depth_m[layer + 1] - input.node_depth_m[layer];
        let fortran_layer = layer as isize - input.snow_layers as isize + 1;
        sub[layer] = -implicit * factor[layer] * conductivity[layer - 1] / lower_distance;
        super_[layer] = -implicit * factor[layer] * conductivity[layer] / upper_distance;
        if fortran_layer < 1 {
            diagonal[layer] = 1.0
                + implicit
                    * factor[layer]
                    * (conductivity[layer] / upper_distance
                        + conductivity[layer - 1] / lower_distance);
            rhs[layer] = input.temperature_k[layer]
                + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1])
                + if use_snicar {
                    input.snow_layer_absorption_w_m2.unwrap()[layer] * factor[layer]
                } else {
                    0.0
                };
        } else if fortran_layer == 1 && input.use_split_soil_snow {
            diagonal[layer] = 1.0
                + implicit
                    * factor[layer]
                    * (conductivity[layer] / upper_distance
                        + conductivity[layer - 1] / lower_distance)
                - (1.0 - input.snow_cover_fraction) * derivative * factor[layer];
            rhs[layer] = input.temperature_k[layer]
                + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1])
                + factor[layer]
                    * (soil_heat_flux
                        - (1.0 - input.snow_cover_fraction)
                            * derivative
                            * input.temperature_k[layer]);
        } else {
            diagonal[layer] = 1.0
                + implicit
                    * factor[layer]
                    * (conductivity[layer] / upper_distance
                        + conductivity[layer - 1] / lower_distance);
            rhs[layer] = input.temperature_k[layer]
                + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1]);
        }
    }
    let bottom = layers - 1;
    let lower_distance = input.node_depth_m[bottom] - input.node_depth_m[bottom - 1];
    sub[bottom] = -implicit * factor[bottom] * conductivity[bottom - 1] / lower_distance;
    diagonal[bottom] = 1.0 + implicit * factor[bottom] * conductivity[bottom - 1] / lower_distance;
    rhs[bottom] = input.temperature_k[bottom]
        - input.crank_nicolson_factor * factor[bottom] * flux[bottom - 1];
    (sub, diagonal, super_, rhs)
}

fn interface_fluxes(conductivity: &[f64], depth: &[f64], temperature: &[f64]) -> Result<Vec<f64>> {
    let mut flux = vec![0.0; temperature.len()];
    for layer in 0..temperature.len() - 1 {
        let distance = depth[layer + 1] - depth[layer];
        ensure!(
            distance > 0.0 && distance.is_finite(),
            "ground-temperature node depths must increase"
        );
        flux[layer] =
            conductivity[layer] * (temperature[layer + 1] - temperature[layer]) / distance;
    }
    Ok(flux)
}

fn residual_heat_fluxes(cnfac: f64, before: &[f64], after: &[f64]) -> Vec<f64> {
    let mut residual = vec![cnfac * before[0] + (1.0 - cnfac) * after[0]];
    for layer in 1..before.len() {
        residual.push(
            cnfac * (before[layer] - before[layer - 1])
                + (1.0 - cnfac) * (after[layer] - after[layer - 1]),
        );
    }
    residual
}

fn state_from_phase(
    phase: PhaseChangeState,
    snow_ice_before: Vec<f64>,
    time_step_seconds: f64,
    layer_factor_seconds_per_j_m2_k: Vec<f64>,
    interface_conductivity_w_m_k: Vec<f64>,
) -> GroundTemperatureState {
    let snow_freezing_rate_kg_m2_s = phase.ice_water_kg_m2[..snow_ice_before.len()]
        .iter()
        .zip(snow_ice_before)
        .zip(&phase.phase_flag)
        .map(|((after, before), flag)| {
            if *flag == 2 {
                (after - before).max(0.0) / time_step_seconds
            } else {
                0.0
            }
        })
        .collect();
    GroundTemperatureState {
        temperature_k: phase.temperature_k,
        liquid_water_kg_m2: phase.liquid_water_kg_m2,
        ice_water_kg_m2: phase.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: phase.snow_water_equivalent_kg_m2,
        snow_depth_m: phase.snow_depth_m,
        snow_melt_rate_kg_m2_s: phase.snow_melt_rate_kg_m2_s,
        latent_heat_flux_w_m2: phase.latent_heat_flux_w_m2,
        phase_flag: phase.phase_flag,
        thaw_mass_kg_m2: phase.thaw_mass_kg_m2,
        freeze_mass_kg_m2: phase.freeze_mass_kg_m2,
        snow_freezing_rate_kg_m2_s,
        layer_factor_seconds_per_j_m2_k,
        interface_conductivity_w_m_k,
    }
}

fn validate(input: GroundTemperatureInput<'_>) -> Result<usize> {
    let layers = input.temperature_k.len();
    let soil_layers = input.soil_thermal_inputs.len();
    ensure!(
        layers >= 2 && layers == input.snow_layers + soil_layers,
        "ground temperature needs at least two packed layers with all soil layers present"
    );
    for values in [
        input.layer_thickness_m,
        input.node_depth_m,
        input.temperature_k,
        input.liquid_water_kg_m2,
        input.ice_water_kg_m2,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "ground-temperature packed vectors must be finite and match"
        );
    }
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input
                .interface_depth_m
                .iter()
                .all(|value| value.is_finite()),
        "ground-temperature interfaces must be finite and have one extra boundary"
    );
    ensure!(
        input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0),
        "ground-temperature thicknesses and water masses are invalid"
    );
    for values in [
        input.soil_porosity,
        input.soil_residual_water,
        input.soil_suction_mm,
    ] {
        ensure!(
            values.len() == soil_layers && values.iter().all(|value| value.is_finite()),
            "ground-temperature soil vectors must match the soil layers"
        );
    }
    ensure!(
        input.soil_hydraulic_model.len() == soil_layers
            && input.soil_porosity.iter().all(|value| *value >= 0.0)
            && input.soil_residual_water.iter().all(|value| *value >= 0.0)
            && input.soil_suction_mm.iter().all(|value| *value < 0.0),
        "ground-temperature soil hydraulics are invalid"
    );
    if let Some(values) = input.snow_layer_absorption_w_m2 {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "SNICAR absorption must be finite and match packed layers"
        );
    }
    let scalars = [
        input.time_step_seconds,
        input.surface_temperature_factor,
        input.crank_nicolson_factor,
        input.snow_water_equivalent_kg_m2,
        input.snow_depth_m,
        input.snow_cover_fraction,
        input.absorbed_ground_shortwave_w_m2,
        input.absorbed_soil_shortwave_w_m2,
        input.absorbed_snow_shortwave_w_m2,
        input.downward_longwave_w_m2,
        input.sensible_ground_w_m2,
        input.sensible_soil_w_m2,
        input.sensible_snow_w_m2,
        input.evaporation_ground_kg_m2_s,
        input.evaporation_soil_kg_m2_s,
        input.evaporation_snow_kg_m2_s,
        input.ground_flux_temperature_derivative_w_m2_k,
        input.vaporization_heat_j_kg,
        input.ground_emissivity,
        input.rain_on_ground_kg_m2_s,
        input.snow_on_ground_kg_m2_s,
        input.precipitation_temperature_k,
        input.ground_temperature_k,
        input.soil_surface_temperature_k,
        input.snow_surface_temperature_k,
    ];
    ensure!(
        scalars.iter().all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && input.surface_temperature_factor > 0.0
            && (0.0..=1.0).contains(&input.crank_nicolson_factor)
            && input.snow_water_equivalent_kg_m2 >= 0.0
            && input.snow_depth_m >= 0.0
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.ground_emissivity >= 0.0,
        "ground-temperature scalar inputs are invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "ground_temperature_tests.rs"]
mod ground_temperature_tests;
