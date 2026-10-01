//! Soil evaporation resistance from `MOD_SoilSurfaceResistance.F90`.

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::{
    soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi, SoilHydraulicModel,
};

use crate::f77;

const WATER_DENSITY_KG_M3: f64 = f77(1000.0);
const ICE_DENSITY_KG_M3: f64 = f77(917.0);

/// Inputs for the top soil layer consumed by `SoilSurfaceResistance`.
#[derive(Debug, Clone, Copy)]
pub struct SoilSurfaceResistanceInput {
    pub air_density_kg_m3: f64,
    pub saturated_hydraulic_conductivity_mm_s: f64,
    pub porosity: f64,
    pub saturated_soil_suction_mm: f64,
    pub residual_water: f64,
    pub hydraulic_model: SoilHydraulicModel,
    pub layer_thickness_m: f64,
    pub temperature_k: f64,
    pub liquid_water_kg_m2: f64,
    pub ice_water_kg_m2: f64,
    pub snow_cover_fraction: f64,
    pub ground_specific_humidity: f64,
    /// `DEF_RSS_SCHEME`: 1=SL14, 2=SZ09, 3=TR13, 4=LP92, 5=S92.
    pub scheme: i32,
}

/// Port of `MOD_SoilSurfaceResistance:SoilSurfaceResistance` for one patch.
pub fn soil_surface_resistance(input: SoilSurfaceResistanceInput) -> Result<f64> {
    validate(input)?;
    let liquid_volume =
        input.liquid_water_kg_m2.max(f77(1.0e-6)) / (WATER_DENSITY_KG_M3 * input.layer_thickness_m);
    let saturation = (liquid_volume / input.porosity).min(1.0);
    // `:138 eff_porosity = max(0.01, porsl - min(porsl, wice/(dz*denice)))`：先换算成体积
    // 再与 `porsl` 取小（原先是先把 `wice` 截到 `porsl*dz*denice` 再除，舍入不同）。
    let effective_porosity = (input.porosity
        - input
            .porosity
            .min(input.ice_water_kg_m2 / (input.layer_thickness_m * ICE_DENSITY_KG_M3)))
    .max(0.01);
    let (matric_potential_m, conductivity_m_s, air_dry_water) = match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => (
            input.saturated_soil_suction_mm / 1000.0 * saturation.lpow(-bsw),
            input.saturated_hydraulic_conductivity_mm_s / 1000.0
                // `:143` 的指数是 `.FMA (bsw, 2, 3)`。
                * (liquid_volume / input.porosity).lpow(bsw.mul_add(2.0, 3.0)),
            input.porosity * (input.saturated_soil_suction_mm / -f77(1.0e7)).lpow(1.0 / bsw),
        ),
        model => {
            let matric_potential_mm = soil_psi_from_vliq(
                saturation * (input.porosity - input.residual_water) + input.residual_water,
                input.porosity,
                input.residual_water,
                input.saturated_soil_suction_mm,
                model,
            );
            (
                matric_potential_mm / 1000.0,
                soil_hydraulic_conductivity(
                    matric_potential_mm,
                    input.saturated_soil_suction_mm,
                    input.saturated_hydraulic_conductivity_mm_s,
                    model,
                ) / 1000.0,
                soil_vliq_from_psi(
                    -f77(1.0e7),
                    input.porosity,
                    input.residual_water,
                    input.saturated_soil_suction_mm,
                    model,
                ),
            )
        }
    };
    let open_air_diffusivity = f77(2.12e-5) * (input.temperature_k / f77(273.15)).lpow(f77(1.75));
    let air_filled_porosity = input.porosity - air_dry_water;
    ensure!(
        air_filled_porosity > 0.0,
        "soil surface resistance needs positive air-filled porosity"
    );
    let tortuosity = match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => {
            air_filled_porosity.powi(2)
                * (air_filled_porosity / input.porosity).lpow(3.0 / 3.0_f64.max(bsw))
        }
        model => {
            let air_at_1000 = input.porosity
                - soil_vliq_from_psi(
                    -1000.0,
                    input.porosity,
                    input.residual_water,
                    input.saturated_soil_suction_mm,
                    model,
                );
            ensure!(
                air_at_1000 > 0.0 && air_at_1000 != input.porosity,
                "van Genuchten soil resistance has a singular air-pore ratio"
            );
            input.porosity.powi(2)
                * (air_filled_porosity / input.porosity)
                    .lpow(2.0 + (air_at_1000.lpow(0.25)).ln() / (air_at_1000 / input.porosity).ln())
        }
    };
    let gas_diffusivity = open_air_diffusivity * tortuosity;
    ensure!(
        gas_diffusivity > 0.0 && gas_diffusivity.is_finite(),
        "soil surface resistance has invalid gas diffusivity"
    );
    let aqueous_diffusivity =
        aqueous_diffusivity(input, matric_potential_m, conductivity_m_s, liquid_volume);
    let mut resistance = match input.scheme {
        1 => {
            // `:231-232`：分子分母两处 `0.8*x - y` 都是 `.FMS (x, 0.8, y)`；
            // `max(…)*dz` 再除（第 406 轮，Campbell 算例的 `rss` 差 1 ulp）。
            let dry_layer = (effective_porosity
                .mul_add(f77(0.8), -liquid_volume)
                .max(f77(1.0e-6))
                * input.layer_thickness_m
                / input
                    .porosity
                    .mul_add(f77(0.8), -air_dry_water)
                    .max(f77(1.0e-6)))
            .clamp(0.0, 0.2);
            dry_layer / gas_diffusivity
        }
        2 => {
            let dry_layer = (input.layer_thickness_m
                * ((1.0 - liquid_volume / input.porosity).powi(5).exp() - 1.0)
                / (1.0_f64.exp() - 1.0))
                .clamp(0.0, 0.2);
            dry_layer / gas_diffusivity
        }
        3 => {
            let bunsen =
                WATER_DENSITY_KG_M3 / (input.ground_specific_humidity * input.air_density_kg_m3);
            let gas_conductance =
                2.0 * gas_diffusivity * air_filled_porosity / input.layer_thickness_m;
            let water_conductance =
                2.0 * aqueous_diffusivity * bunsen * liquid_volume / input.layer_thickness_m;
            1.0 / (gas_conductance + water_conductance)
        }
        4 => lp92_beta(input),
        5 => {
            let water_ice_volume = (input.liquid_water_kg_m2.max(f77(1.0e-6))
                / WATER_DENSITY_KG_M3
                + input.ice_water_kg_m2 / ICE_DENSITY_KG_M3)
                / input.layer_thickness_m;
            let fraction = (water_ice_volume / input.porosity).clamp(0.001, 1.0);
            (f77(8.206) - 6.0 * fraction).exp()
        }
        _ => unreachable!("validated scheme"),
    };
    if input.scheme == 4 {
        resistance = (1.0 - input.snow_cover_fraction) * resistance + input.snow_cover_fraction;
    } else {
        // `:299` `.FMA (fsno, rss, 1-fsno)`
        let denominator = input
            .snow_cover_fraction
            .mul_add(resistance, 1.0 - input.snow_cover_fraction);
        resistance = if denominator > 0.0 {
            resistance / denominator
        } else {
            0.0
        };
        resistance = resistance.min(f77(1.0e6));
    }
    Ok(resistance)
}

fn aqueous_diffusivity(
    input: SoilSurfaceResistanceInput,
    matric_potential_m: f64,
    conductivity_m_s: f64,
    liquid_volume: f64,
) -> f64 {
    match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => {
            -conductivity_m_s * bsw * matric_potential_m / liquid_volume
        }
        SoilHydraulicModel::VanGenuchten {
            alpha_vgm, n_vgm, ..
        } => {
            let m = 1.0 - 1.0 / n_vgm;
            let saturation = (1.0 + (-alpha_vgm * matric_potential_m).lpow(n_vgm)).lpow(-m);
            -conductivity_m_s * (m - 1.0)
                / (alpha_vgm * m * (input.porosity - input.residual_water))
                * saturation.lpow(-1.0 / m)
                * (1.0 - saturation.lpow(1.0 / m)).lpow(-m)
        }
    }
}

#[allow(clippy::approx_constant)] // CoLM uses the 3.1415926 source literal.
fn lp92_beta(input: SoilSurfaceResistanceInput) -> f64 {
    let water_ice_volume = (input.liquid_water_kg_m2.max(f77(1.0e-6)) / WATER_DENSITY_KG_M3
        + input.ice_water_kg_m2 / ICE_DENSITY_KG_M3)
        / input.layer_thickness_m;
    let field_capacity = match input.hydraulic_model {
        SoilHydraulicModel::Campbell { bsw } => {
            input.porosity
                * (f77(0.1) / (86400.0 * input.saturated_hydraulic_conductivity_mm_s))
                    .lpow(1.0 / (2.0 * bsw + 3.0))
        }
        SoilHydraulicModel::VanGenuchten {
            alpha_vgm, n_vgm, ..
        } => {
            input.residual_water
                + (input.porosity - input.residual_water)
                    * (1.0 + (alpha_vgm * f77(339.9)).lpow(n_vgm)).lpow(1.0 / n_vgm - 1.0)
        }
    };
    if water_ice_volume < field_capacity {
        let fraction = (water_ice_volume / field_capacity).clamp(0.001, 1.0);
        0.25 * (1.0 - (fraction * f77(3.1415926)).cos()).powi(2)
    } else {
        1.0
    }
}

fn validate(input: SoilSurfaceResistanceInput) -> Result<()> {
    let values = [
        input.air_density_kg_m3,
        input.saturated_hydraulic_conductivity_mm_s,
        input.porosity,
        input.saturated_soil_suction_mm,
        input.residual_water,
        input.layer_thickness_m,
        input.temperature_k,
        input.liquid_water_kg_m2,
        input.ice_water_kg_m2,
        input.snow_cover_fraction,
        input.ground_specific_humidity,
    ];
    ensure!(
        values.iter().all(|value| value.is_finite())
            && input.air_density_kg_m3 > 0.0
            && input.saturated_hydraulic_conductivity_mm_s > 0.0
            && input.porosity > 0.0
            && input.saturated_soil_suction_mm < 0.0
            && input.residual_water >= 0.0
            && input.layer_thickness_m > 0.0
            && input.temperature_k > 0.0
            && input.liquid_water_kg_m2 >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
            && input.ice_water_kg_m2 >= 0.0
            && input.ground_specific_humidity > 0.0
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && (1..=5).contains(&input.scheme),
        "soil surface resistance inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "soil_surface_resistance_tests.rs"]
mod soil_surface_resistance_tests;
