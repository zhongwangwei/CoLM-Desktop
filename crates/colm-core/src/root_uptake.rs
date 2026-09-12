//! Root water-stress and transpiration limits from `MOD_Eroot.F90`.

use anyhow::{ensure, Result};

use crate::{soil_psi_from_vliq, soil_vliq_from_psi, SoilHydraulicModel, FREEZING_K};

const fn f77(value: f32) -> f64 {
    value as f64
}

const WATER_DENSITY_KG_M3: f64 = f77(1000.0);
const WILTING_POTENTIAL_MM: f64 = f77(-1.5e5);
const FIELD_CAPACITY_POTENTIAL_MM: f64 = f77(-3.3e3);
const ROOT_NORMALIZATION_FLOOR: f64 = f77(1.0e-10);

/// Inputs to CoLM's `eroot` root-resistance calculation.
#[derive(Debug, Clone, Copy)]
pub struct RootUptakeInput<'a> {
    pub maximum_transpiration_mm_s: f64,
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_soil_suction_mm: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub root_fraction: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    /// `DEF_RSTFAC`: 1=matric-potential stress; 2=wilting-to-field-capacity stress.
    pub stress_scheme: i32,
}

/// Outputs of CoLM's `eroot` routine.
#[derive(Debug, Clone, PartialEq)]
pub struct RootUptakeState {
    /// Per-layer root resistance normalized to sum to one when active.
    pub layer_fraction: Vec<f64>,
    pub maximum_transpiration_mm_s: f64,
    pub soil_water_stress: f64,
}

/// Ports `MOD_Eroot:eroot` and reuses the shared Campbell/VG hydraulic curves.
pub fn root_uptake(input: RootUptakeInput<'_>) -> Result<RootUptakeState> {
    let layers = validate(input)?;
    let mut layer_fraction = vec![0.0; layers];
    let mut root_total = ROOT_NORMALIZATION_FLOOR;
    for (layer, fraction) in layer_fraction.iter_mut().enumerate() {
        if input.temperature_k[layer] <= FREEZING_K || input.porosity[layer] < f77(1.0e-6) {
            continue;
        }
        let resistance = match input.stress_scheme {
            1 => potential_stress(input, layer),
            2 => capacity_stress(input, layer),
            _ => unreachable!("validated stress scheme"),
        };
        *fraction = input.root_fraction[layer] * resistance;
        root_total += *fraction;
    }
    for fraction in &mut layer_fraction {
        *fraction /= root_total;
    }
    Ok(RootUptakeState {
        layer_fraction,
        maximum_transpiration_mm_s: input.maximum_transpiration_mm_s * root_total,
        soil_water_stress: root_total,
    })
}

fn potential_stress(input: RootUptakeInput<'_>, layer: usize) -> f64 {
    let saturation = (input.liquid_water_kg_m2[layer]
        / (WATER_DENSITY_KG_M3 * input.layer_thickness_m[layer] * input.porosity[layer]))
        .clamp(0.001, 1.0);
    let potential = match input.hydraulic_model[layer] {
        SoilHydraulicModel::Campbell { bsw } => {
            input.saturated_soil_suction_mm[layer] * saturation.powf(-bsw)
        }
        model => soil_psi_from_vliq(
            saturation * (input.porosity[layer] - input.residual_water[layer])
                + input.residual_water[layer],
            input.porosity[layer],
            input.residual_water[layer],
            input.saturated_soil_suction_mm[layer],
            model,
        ),
    }
    .max(WILTING_POTENTIAL_MM);
    (1.0 - potential / WILTING_POTENTIAL_MM)
        / (1.0 - input.saturated_soil_suction_mm[layer] / WILTING_POTENTIAL_MM)
}

fn capacity_stress(input: RootUptakeInput<'_>, layer: usize) -> f64 {
    let (wilting_water, field_capacity_water) = match input.hydraulic_model[layer] {
        SoilHydraulicModel::Campbell { bsw } => (
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * input.porosity[layer]
                * (WILTING_POTENTIAL_MM / input.saturated_soil_suction_mm[layer]).powf(-1.0 / bsw),
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * input.porosity[layer]
                * (FIELD_CAPACITY_POTENTIAL_MM / input.saturated_soil_suction_mm[layer])
                    .powf(-1.0 / bsw),
        ),
        model => (
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * soil_vliq_from_psi(
                    WILTING_POTENTIAL_MM,
                    input.porosity[layer],
                    input.residual_water[layer],
                    input.saturated_soil_suction_mm[layer],
                    model,
                ),
            WATER_DENSITY_KG_M3
                * input.layer_thickness_m[layer]
                * soil_vliq_from_psi(
                    FIELD_CAPACITY_POTENTIAL_MM,
                    input.porosity[layer],
                    input.residual_water[layer],
                    input.saturated_soil_suction_mm[layer],
                    model,
                ),
        ),
    };
    ((input.liquid_water_kg_m2[layer] - wilting_water) / (field_capacity_water - wilting_water))
        .clamp(0.0, 1.0)
}

fn validate(input: RootUptakeInput<'_>) -> Result<usize> {
    let layers = input.porosity.len();
    ensure!(layers > 0, "root uptake needs one or more soil layers");
    for values in [
        input.residual_water,
        input.saturated_soil_suction_mm,
        input.root_fraction,
        input.layer_thickness_m,
        input.temperature_k,
        input.liquid_water_kg_m2,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "root-uptake vectors must be finite and have equal lengths"
        );
    }
    ensure!(
        input.hydraulic_model.len() == layers
            && input.maximum_transpiration_mm_s.is_finite()
            && input.porosity.iter().all(|value| *value >= 0.0)
            && input.residual_water.iter().all(|value| *value >= 0.0)
            && input
                .saturated_soil_suction_mm
                .iter()
                .all(|value| *value < 0.0)
            && input.root_fraction.iter().all(|value| *value >= 0.0)
            && input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0)
            && (1..=2).contains(&input.stress_scheme),
        "root-uptake inputs are invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "root_uptake_tests.rs"]
mod root_uptake_tests;
