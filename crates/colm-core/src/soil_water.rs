//! Campbell/Richards soil-water solve from `MOD_SoilSnowHydrology:soilwater`.
//!
//! This is the non-VSF branch used by `WATER_2014`.  It owns no restart or
//! forcing I/O, so the initializer and the eventual Rust `colm` driver share
//! the same tridiagonal solve.

use anyhow::{ensure, Context, Result};

use crate::{solve_tridiagonal, FREEZING_K};

/// One `soilwater` call. All layer vectors use top-to-bottom order; water and
/// flux lengths are in millimetres and seconds, matching the upstream routine.
#[derive(Debug, Clone, Copy)]
pub struct CampbellSoilWaterInput<'a> {
    pub patch_type: i32,
    pub time_step_seconds: f64,
    pub impermeable_porosity: f64,
    pub minimum_potential_mm: f64,
    pub infiltration_mm_s: f64,
    pub transpiration_mm_s: f64,
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water: &'a [f64],
    pub ice_fraction: &'a [f64],
    pub effective_porosity: &'a [f64],
    pub porosity: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub root_fraction: &'a [f64],
    pub root_flux_mm_s: &'a [f64],
    pub plant_hydraulics: bool,
    pub urban_run: bool,
    pub soil_ice_impedance: f64,
}

/// Resolved hydraulic state and fluxes from one `soilwater` call.
#[derive(Debug, Clone, PartialEq)]
pub struct CampbellSoilWaterState {
    pub liquid_water_change: Vec<f64>,
    pub recharge_mm_s: f64,
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
    /// Surface through bottom interface, positive downward.
    pub interface_flux_mm_s: Vec<f64>,
    pub root_uptake_mm_s: Vec<f64>,
    pub root_uptake_amount_mm: Vec<f64>,
}

/// Ports `MOD_SoilSnowHydrology.F90:soilwater` for the Campbell hydraulic model.
///
/// The upstream routine deliberately always uses free drainage at the bottom;
/// its former water-table boundary branch is commented out, so water-table
/// depth is not an input to this active equation.
pub fn solve_campbell_soil_water(
    input: CampbellSoilWaterInput<'_>,
) -> Result<CampbellSoilWaterState> {
    let layers = validate(input)?;
    let root_uptake_mm_s = if input.plant_hydraulics && (input.patch_type != 1 || !input.urban_run)
    {
        input.root_flux_mm_s.to_vec()
    } else {
        input
            .root_fraction
            .iter()
            .map(|fraction| input.transpiration_mm_s * fraction)
            .collect()
    };
    let root_uptake_amount_mm = root_uptake_mm_s
        .iter()
        .map(|flux| flux.max(0.0) * input.time_step_seconds)
        .collect();

    let mut matric_potential_mm = vec![0.0; layers];
    let mut potential_derivative = vec![0.0; layers];
    for layer in 0..layers {
        if input.temperature_k[layer] >= FREEZING_K {
            if input.porosity[layer] < 1.0e-6 {
                matric_potential_mm[layer] = input.saturated_potential_mm[layer];
            } else {
                let saturation =
                    (input.liquid_water[layer] / input.porosity[layer]).clamp(0.01, 1.0);
                let potential = input.saturated_potential_mm[layer]
                    * saturation.powf(-input.clapp_hornberger_b[layer]);
                matric_potential_mm[layer] = potential.max(input.minimum_potential_mm);
                potential_derivative[layer] = -input.clapp_hornberger_b[layer]
                    * matric_potential_mm[layer]
                    / (saturation * input.porosity[layer]);
            }
        } else {
            matric_potential_mm[layer] = (1.0e3 * 0.3336e6 / 9.80616
                * (input.temperature_k[layer] - FREEZING_K)
                / input.temperature_k[layer])
                .max(input.minimum_potential_mm);
        }
    }

    let mut separation_mm = vec![0.0; layers];
    let mut gradient = vec![0.0; layers];
    let mut hydraulic_conductivity_mm_s = vec![0.0; layers];
    let mut conductivity_lower_derivative = vec![0.0; layers];
    let mut conductivity_upper_derivative = vec![0.0; layers];
    for layer in 0..layers {
        if layer + 1 < layers {
            separation_mm[layer] =
                (input.node_depth_m[layer + 1] - input.node_depth_m[layer]) * 1000.0;
            gradient[layer] = (matric_potential_mm[layer + 1] - matric_potential_mm[layer])
                / separation_mm[layer]
                - 1.0;
        }
        if input.effective_porosity[layer] < input.impermeable_porosity
            || input.effective_porosity[(layer + 1).min(layers - 1)] < input.impermeable_porosity
            || input.liquid_water[layer] <= 1.0e-3
        {
            continue;
        }
        let source = if layer + 1 < layers && gradient[layer] > 0.0 {
            layer + 1
        } else {
            layer
        };
        let saturation = input.liquid_water[source] / input.porosity[source];
        let exponent = 2.0 * input.clapp_hornberger_b[source] + 3.0;
        let conductivity =
            input.saturated_hydraulic_conductivity_mm_s[source] * saturation.powf(exponent);
        let derivative = input.saturated_hydraulic_conductivity_mm_s[source]
            * exponent
            * saturation.powf(exponent - 1.0)
            / input.porosity[source];
        let impedance = 10_f64.powf(
            -input.soil_ice_impedance
                * 0.5
                * (input.ice_fraction[layer] + input.ice_fraction[(layer + 1).min(layers - 1)]),
        );
        hydraulic_conductivity_mm_s[layer] = impedance * conductivity;
        if source == layer {
            conductivity_lower_derivative[layer] = impedance * derivative;
        } else {
            conductivity_upper_derivative[layer] = impedance * derivative;
        }
    }

    let thickness_mm = input
        .layer_thickness_m
        .iter()
        .map(|thickness| thickness * 1000.0)
        .collect::<Vec<_>>();
    let mut lower = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut upper = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    let mut outflow = vec![0.0; layers];
    let mut outflow_lower_derivative = vec![0.0; layers];
    let mut outflow_upper_derivative = vec![0.0; layers];

    outflow[0] = -hydraulic_conductivity_mm_s[0] * gradient[0];
    outflow_lower_derivative[0] = -(gradient[0] * conductivity_lower_derivative[0]
        - hydraulic_conductivity_mm_s[0] * potential_derivative[0] / separation_mm[0]);
    outflow_upper_derivative[0] = -(gradient[0] * conductivity_upper_derivative[0]
        + hydraulic_conductivity_mm_s[0] * potential_derivative[1] / separation_mm[0]);
    diagonal[0] = thickness_mm[0] / input.time_step_seconds + outflow_lower_derivative[0];
    upper[0] = outflow_upper_derivative[0];
    rhs[0] = input.infiltration_mm_s - outflow[0] - root_uptake_mm_s[0];

    for layer in 1..layers - 1 {
        let inflow = -hydraulic_conductivity_mm_s[layer - 1] * gradient[layer - 1];
        let inflow_lower_derivative = -(gradient[layer - 1]
            * conductivity_lower_derivative[layer - 1]
            - hydraulic_conductivity_mm_s[layer - 1] * potential_derivative[layer - 1]
                / separation_mm[layer - 1]);
        let inflow_upper_derivative = -(gradient[layer - 1]
            * conductivity_upper_derivative[layer - 1]
            + hydraulic_conductivity_mm_s[layer - 1] * potential_derivative[layer]
                / separation_mm[layer - 1]);
        outflow[layer] = -hydraulic_conductivity_mm_s[layer] * gradient[layer];
        outflow_lower_derivative[layer] = -(gradient[layer] * conductivity_lower_derivative[layer]
            - hydraulic_conductivity_mm_s[layer] * potential_derivative[layer]
                / separation_mm[layer]);
        outflow_upper_derivative[layer] = -(gradient[layer] * conductivity_upper_derivative[layer]
            + hydraulic_conductivity_mm_s[layer] * potential_derivative[layer + 1]
                / separation_mm[layer]);
        lower[layer] = -inflow_lower_derivative;
        diagonal[layer] = thickness_mm[layer] / input.time_step_seconds - inflow_upper_derivative
            + outflow_lower_derivative[layer];
        upper[layer] = outflow_upper_derivative[layer];
        rhs[layer] = inflow - outflow[layer] - root_uptake_mm_s[layer];
    }

    let last = layers - 1;
    let inflow = -hydraulic_conductivity_mm_s[last - 1] * gradient[last - 1];
    let inflow_lower_derivative = -(gradient[last - 1] * conductivity_lower_derivative[last - 1]
        - hydraulic_conductivity_mm_s[last - 1] * potential_derivative[last - 1]
            / separation_mm[last - 1]);
    let inflow_upper_derivative = -(gradient[last - 1] * conductivity_upper_derivative[last - 1]
        + hydraulic_conductivity_mm_s[last - 1] * potential_derivative[last]
            / separation_mm[last - 1]);
    outflow[last] = hydraulic_conductivity_mm_s[last];
    outflow_lower_derivative[last] = conductivity_lower_derivative[last];
    lower[last] = -inflow_lower_derivative;
    diagonal[last] = thickness_mm[last] / input.time_step_seconds - inflow_upper_derivative
        + outflow_lower_derivative[last];
    rhs[last] = inflow - outflow[last] - root_uptake_mm_s[last];

    let liquid_water_change = solve_tridiagonal(&lower, &diagonal, &upper, &rhs)
        .map_err(anyhow::Error::msg)
        .context("soilwater tridiagonal solve failed")?;
    ensure!(
        liquid_water_change.iter().all(|value| value.is_finite()),
        "soilwater solve produced a non-finite water-content change"
    );
    let recharge_mm_s = outflow[last] + outflow_lower_derivative[last] * liquid_water_change[last];
    let mut interface_flux_mm_s = Vec::with_capacity(layers + 1);
    interface_flux_mm_s.push(input.infiltration_mm_s);
    for layer in 0..last {
        interface_flux_mm_s.push(
            outflow[layer]
                + outflow_lower_derivative[layer] * liquid_water_change[layer]
                + outflow_upper_derivative[layer] * liquid_water_change[layer + 1],
        );
    }
    interface_flux_mm_s.push(recharge_mm_s);
    ensure!(
        recharge_mm_s.is_finite() && interface_flux_mm_s.iter().all(|value| value.is_finite()),
        "soilwater solve produced a non-finite flux"
    );
    Ok(CampbellSoilWaterState {
        liquid_water_change,
        recharge_mm_s,
        matric_potential_mm,
        hydraulic_conductivity_mm_s,
        interface_flux_mm_s,
        root_uptake_mm_s,
        root_uptake_amount_mm,
    })
}

fn validate(input: CampbellSoilWaterInput<'_>) -> Result<usize> {
    let layers = input.node_depth_m.len();
    ensure!(layers >= 2, "soilwater needs at least two soil layers");
    for values in [
        input.layer_thickness_m,
        input.temperature_k,
        input.liquid_water,
        input.ice_fraction,
        input.effective_porosity,
        input.porosity,
        input.saturated_hydraulic_conductivity_mm_s,
        input.clapp_hornberger_b,
        input.saturated_potential_mm,
        input.root_fraction,
        input.root_flux_mm_s,
    ] {
        ensure!(
            values.len() == layers,
            "soilwater vectors must have equal lengths"
        );
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "soilwater vectors must be finite"
        );
    }
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.impermeable_porosity.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.minimum_potential_mm.is_finite()
            && input.minimum_potential_mm < 0.0
            && input.infiltration_mm_s.is_finite()
            && input.transpiration_mm_s.is_finite()
            && input.soil_ice_impedance.is_finite()
            && input.soil_ice_impedance > 0.0,
        "soilwater scalar inputs are invalid"
    );
    for layer in 0..layers {
        ensure!(
            input.layer_thickness_m[layer] > 0.0
                && input.porosity[layer] >= 0.0
                && input.effective_porosity[layer] >= 0.0
                && input.effective_porosity[layer] <= input.porosity[layer]
                && input.liquid_water[layer] >= 0.0
                && (0.0..=1.0).contains(&input.ice_fraction[layer])
                && input.saturated_hydraulic_conductivity_mm_s[layer] >= 0.0
                && input.clapp_hornberger_b[layer] > 0.0
                && input.saturated_potential_mm[layer] < 0.0,
            "soilwater layer inputs are invalid"
        );
        if layer > 0 {
            ensure!(
                input.node_depth_m[layer] > input.node_depth_m[layer - 1],
                "soilwater node depths must increase"
            );
        }
    }
    Ok(layers)
}

#[cfg(test)]
#[path = "soil_water_tests.rs"]
mod soil_water_tests;
