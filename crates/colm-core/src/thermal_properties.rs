//! Per-layer soil thermal properties from MOD_SoilThermalParameters.F90.

use anyhow::{ensure, Result};

use crate::FREEZING_K;

const fn f77(value: f32) -> f64 {
    value as f64
}

/// CoLM runtime selector DEF_THERMAL_CONDUCTIVITY_SCHEME.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalConductivityScheme {
    Oleson,
    Johansen,
    CoteKonrad,
    BallandArp,
    Lu,
    TarnawskiLeong,
    DeVries,
    YanHe,
}

/// Scalar inputs to soil_hcap_cond for one soil layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilThermalInput {
    pub gravel_volume_fraction_of_solids: f64,
    pub organic_volume_fraction_of_solids: f64,
    pub sand_volume_fraction_of_solids: f64,
    pub pore_volume_fraction: f64,
    pub gravel_mass_fraction: f64,
    pub sand_mass_fraction: f64,
    pub solid_conductivity_w_m_k: f64,
    pub dry_heat_capacity_j_m3_k: f64,
    pub dry_conductivity_w_m_k: f64,
    pub saturated_unfrozen_conductivity_w_m_k: f64,
    pub saturated_frozen_conductivity_w_m_k: f64,
    pub balland_alpha: f64,
    pub balland_beta: f64,
    pub temperature_k: f64,
    pub liquid_volume_fraction: f64,
    pub ice_volume_fraction: f64,
}

/// soil_hcap_cond outputs for one soil layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilThermalProperties {
    pub heat_capacity_j_m3_k: f64,
    pub conductivity_w_m_k: f64,
}

/// Port of MOD_SoilThermalParameters:soil_hcap_cond.
pub fn soil_thermal_properties(
    input: SoilThermalInput,
    scheme: ThermalConductivityScheme,
) -> Result<SoilThermalProperties> {
    validate(input)?;
    let heat_capacity_j_m3_k = input.dry_heat_capacity_j_m3_k
        + input.liquid_volume_fraction * f77(4.188e6)
        + input.ice_volume_fraction * f77(1.94153e6);
    let saturation = ((input.liquid_volume_fraction + input.ice_volume_fraction)
        / input.pore_volume_fraction)
        .min(1.0);
    let conductivity_w_m_k = if saturation < f77(1.0e-10) {
        input.dry_conductivity_w_m_k
    } else {
        conductivity_for_saturated_soil(input, saturation, scheme)
    };
    Ok(SoilThermalProperties {
        heat_capacity_j_m3_k,
        conductivity_w_m_k,
    })
}

fn conductivity_for_saturated_soil(
    input: SoilThermalInput,
    saturation: f64,
    scheme: ThermalConductivityScheme,
) -> f64 {
    let unfrozen = input.temperature_k > FREEZING_K;
    match scheme {
        ThermalConductivityScheme::Oleson
        | ThermalConductivityScheme::Johansen
        | ThermalConductivityScheme::CoteKonrad
        | ThermalConductivityScheme::BallandArp
        | ThermalConductivityScheme::Lu => {
            let kersten = kersten_number(input, saturation, scheme, unfrozen).clamp(0.0, 1.0);
            let saturated = if unfrozen {
                input.saturated_unfrozen_conductivity_w_m_k
            } else {
                input.saturated_frozen_conductivity_w_m_k
            };
            (saturated - input.dry_conductivity_w_m_k) * kersten + input.dry_conductivity_w_m_k
        }
        ThermalConductivityScheme::TarnawskiLeong => tarnawski_leong(input, saturation, unfrozen),
        ThermalConductivityScheme::DeVries => de_vries(input, saturation, unfrozen),
        ThermalConductivityScheme::YanHe => {
            let beta = -f77(0.303) * input.saturated_unfrozen_conductivity_w_m_k
                - f77(0.201) * input.sand_mass_fraction
                + f77(1.532);
            let kersten = if input.liquid_volume_fraction > f77(0.01) {
                (1.0 + (input.pore_volume_fraction / beta).powf(-beta))
                    / (1.0 + (input.liquid_volume_fraction / beta).powf(-beta))
            } else {
                0.0
            }
            .clamp(0.0, 1.0);
            let saturated = if unfrozen {
                input.saturated_unfrozen_conductivity_w_m_k
            } else {
                input.saturated_frozen_conductivity_w_m_k
            };
            (saturated - input.dry_conductivity_w_m_k) * kersten + input.dry_conductivity_w_m_k
        }
    }
}

fn kersten_number(
    input: SoilThermalInput,
    saturation: f64,
    scheme: ThermalConductivityScheme,
    unfrozen: bool,
) -> f64 {
    let coarse_fraction =
        input.gravel_volume_fraction_of_solids + input.sand_volume_fraction_of_solids;
    match scheme {
        ThermalConductivityScheme::Oleson => {
            if unfrozen {
                saturation.log10() + 1.0
            } else {
                saturation
            }
        }
        ThermalConductivityScheme::Johansen => {
            if !unfrozen {
                saturation
            } else if coarse_fraction > f77(0.4) {
                f77(0.7) * saturation.max(f77(0.05)).log10() + 1.0
            } else {
                saturation.max(f77(0.1)).log10() + 1.0
            }
        }
        ThermalConductivityScheme::CoteKonrad => {
            let kappa = if unfrozen {
                if coarse_fraction > f77(0.40) {
                    f77(4.60)
                } else if coarse_fraction > f77(0.25) {
                    f77(3.55)
                } else if coarse_fraction > f77(0.01) {
                    f77(1.90)
                } else {
                    f77(0.60)
                }
            } else if coarse_fraction > f77(0.40) {
                f77(1.70)
            } else if coarse_fraction > f77(0.25) {
                f77(0.95)
            } else if coarse_fraction > f77(0.01) {
                f77(0.85)
            } else {
                f77(0.25)
            };
            kappa * saturation / (1.0 + (kappa - 1.0) * saturation)
        }
        ThermalConductivityScheme::BallandArp => {
            if unfrozen {
                saturation.powf(
                    f77(0.5)
                        * (1.0 + input.organic_volume_fraction_of_solids
                            - input.balland_alpha * input.sand_volume_fraction_of_solids
                            - input.gravel_volume_fraction_of_solids),
                ) * ((1.0 / (1.0 + (-input.balland_beta * saturation).exp())).powi(3)
                    - ((1.0 - saturation) / f77(2.0)).powi(3))
                .powf(1.0 - input.organic_volume_fraction_of_solids)
            } else {
                saturation.powf(1.0 + input.organic_volume_fraction_of_solids)
            }
        }
        ThermalConductivityScheme::Lu => {
            let (alpha, beta) = if coarse_fraction > f77(0.4) {
                (f77(0.728), f77(1.165))
            } else {
                (f77(0.37), f77(1.29))
            };
            if unfrozen {
                (alpha * (1.0 - saturation.powf(alpha - beta))).exp()
            } else {
                saturation
            }
        }
        ThermalConductivityScheme::TarnawskiLeong
        | ThermalConductivityScheme::DeVries
        | ThermalConductivityScheme::YanHe => unreachable!("handled by its direct formula"),
    }
}

fn tarnawski_leong(input: SoilThermalInput, saturation: f64, unfrozen: bool) -> f64 {
    let coarse_fraction = input.gravel_mass_fraction + input.sand_mass_fraction;
    let solid_path = f77(0.0237) - f77(0.0175) * coarse_fraction.powi(3);
    let micro_pore_fraction = f77(0.088) - f77(0.037) * coarse_fraction.powi(3);
    let exponent = f77(0.6) - f77(0.3) * coarse_fraction.powi(3);
    let wet_micro_pores = if saturation < f77(1.0e-6) {
        0.0
    } else {
        (1.0 - saturation.powf(-exponent)).exp()
    };
    let fluid_conductivity = if unfrozen { f77(0.57) } else { f77(2.29) };
    let air_conductivity = f77(0.024);
    input.solid_conductivity_w_m_k * solid_path
        + (1.0 - input.pore_volume_fraction - solid_path + micro_pore_fraction).powi(2)
            / ((1.0 - input.pore_volume_fraction - solid_path) / input.solid_conductivity_w_m_k
                + micro_pore_fraction
                    / (fluid_conductivity * wet_micro_pores
                        + air_conductivity * (1.0 - wet_micro_pores)))
        + fluid_conductivity
            * (input.pore_volume_fraction * saturation - micro_pore_fraction * wet_micro_pores)
        + air_conductivity
            * (input.pore_volume_fraction * (1.0 - saturation)
                - micro_pore_fraction * (1.0 - wet_micro_pores))
}

fn de_vries(input: SoilThermalInput, saturation: f64, unfrozen: bool) -> f64 {
    let air_conductivity = f77(0.024);
    let fluid_conductivity = if unfrozen { f77(0.57) } else { f77(2.29) };
    let shape_air = if saturation * input.pore_volume_fraction <= f77(0.09) {
        f77(0.013) + f77(0.944) * saturation * input.pore_volume_fraction
    } else {
        f77(0.333)
            - (1.0 - saturation) * input.pore_volume_fraction / input.pore_volume_fraction
                * f77(0.333 - 0.035)
    };
    let shape_solid = 1.0 - f77(2.0) * shape_air;
    let fluid_shape = (f77(2.0)
        / (1.0 + (air_conductivity / fluid_conductivity - 1.0) * shape_air)
        + 1.0 / (1.0 + (air_conductivity / fluid_conductivity - 1.0) * shape_solid))
        / f77(3.0);
    let solid_shape = (f77(2.0)
        / (1.0 + (input.solid_conductivity_w_m_k / fluid_conductivity - 1.0) * f77(0.125))
        + 1.0
            / (1.0
                + (input.solid_conductivity_w_m_k / fluid_conductivity - 1.0)
                    * (1.0 - f77(2.0) * f77(0.125))))
        / f77(3.0);
    (saturation * input.pore_volume_fraction * fluid_conductivity
        + (1.0 - saturation) * input.pore_volume_fraction * fluid_shape * air_conductivity
        + (1.0 - input.pore_volume_fraction) * solid_shape * input.solid_conductivity_w_m_k)
        / (saturation * input.pore_volume_fraction
            + (1.0 - saturation) * input.pore_volume_fraction * fluid_shape
            + (1.0 - input.pore_volume_fraction) * solid_shape)
}

fn validate(input: SoilThermalInput) -> Result<()> {
    let values = [
        input.gravel_volume_fraction_of_solids,
        input.organic_volume_fraction_of_solids,
        input.sand_volume_fraction_of_solids,
        input.pore_volume_fraction,
        input.gravel_mass_fraction,
        input.sand_mass_fraction,
        input.solid_conductivity_w_m_k,
        input.dry_heat_capacity_j_m3_k,
        input.dry_conductivity_w_m_k,
        input.saturated_unfrozen_conductivity_w_m_k,
        input.saturated_frozen_conductivity_w_m_k,
        input.balland_alpha,
        input.balland_beta,
        input.temperature_k,
        input.liquid_volume_fraction,
        input.ice_volume_fraction,
    ];
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "soil thermal inputs must be finite"
    );
    ensure!(
        input.pore_volume_fraction > 0.0
            && input.solid_conductivity_w_m_k > 0.0
            && input.dry_conductivity_w_m_k >= 0.0
            && input.saturated_unfrozen_conductivity_w_m_k >= 0.0
            && input.saturated_frozen_conductivity_w_m_k >= 0.0
            && input.liquid_volume_fraction >= 0.0
            && input.ice_volume_fraction >= 0.0,
        "soil thermal inputs are physically invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "thermal_properties_tests.rs"]
mod thermal_properties_tests;
