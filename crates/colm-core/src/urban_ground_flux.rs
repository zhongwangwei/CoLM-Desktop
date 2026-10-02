//! Urban bare-ground similarity state from `MOD_Urban_GroundFlux.F90`.
//!
//! It composes the already shared Monin-Obukhov kernels and only owns the
//! urban-specific impervious/pervious weighting and roughness iteration.

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::{initialize_monin_obukhov, monin_obukhov, MoninObukhovInitialInput, MoninObukhovInput};

use crate::f77;

const VON_KARMAN: f64 = f77(0.4);
const GRAVITY_M_S2: f64 = f77(9.80616);

/// Inputs to `UrbanGroundFlux`, excluding source arguments it does not read
/// (`tm`, air density, and surface pressure).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanGroundFluxInput {
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub reference_specific_humidity: f64,
    pub reference_wind_m_s: f64,
    pub reference_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub virtual_potential_temperature_k: f64,
    pub land_roughness_m: f64,
    pub snow_roughness_m: f64,
    pub impervious_snow_fraction: f64,
    /// True when the impervious column contains explicit snow layers (`lbi < 1`).
    pub impervious_has_snow_layers: bool,
    pub impervious_surface_liquid_water_kg_m2: f64,
    pub impervious_surface_ice_kg_m2: f64,
    /// CoLM's `fcover(0:5)` urban coverage vector.
    pub cover_fraction: [f64; 6],
    pub impervious_temperature_k: f64,
    pub pervious_temperature_k: f64,
    pub impervious_specific_humidity: f64,
    pub pervious_specific_humidity: f64,
}

/// The similarity diagnostics returned by `UrbanGroundFlux`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanGroundFluxState {
    pub reference_temperature_k: f64,
    pub reference_specific_humidity: f64,
    pub momentum_roughness_m: f64,
    pub heat_roughness_m: f64,
    pub dimensionless_height: f64,
    pub friction_velocity_m_s: f64,
    pub moisture_scale: f64,
    pub temperature_scale_k: f64,
    pub momentum_similarity: f64,
    pub heat_similarity: f64,
    pub moisture_similarity: f64,
}

/// Ports `MOD_Urban_GroundFlux:UrbanGroundFlux`.
pub fn urban_ground_flux(input: UrbanGroundFluxInput) -> Result<UrbanGroundFluxState> {
    validate(input)?;
    let momentum_roughness_m = if input.impervious_snow_fraction > 0.0 {
        input.snow_roughness_m
    } else {
        input.land_roughness_m
    };
    let ground_fraction = 1.0 - input.cover_fraction[0];
    let impervious_fraction = input.cover_fraction[3] / ground_fraction;
    let pervious_fraction = input.cover_fraction[4] / ground_fraction;
    // `MOD_Urban_GroundFlux.F90:141` `.FMA (tgimp, fgimp, tgper*fgper)`
    let ground_temperature_k = input.impervious_temperature_k.mul_add(
        impervious_fraction,
        input.pervious_temperature_k * pervious_fraction,
    );
    let mut impervious_wet_fraction = if input.impervious_has_snow_layers {
        input.impervious_snow_fraction
    } else {
        ((input.impervious_surface_liquid_water_kg_m2 + input.impervious_surface_ice_kg_m2)
            .max(0.0))
        .lpow(f77(2.0 / 3.0))
        .min(1.0)
    };
    if input.reference_specific_humidity > input.impervious_specific_humidity {
        impervious_wet_fraction = 1.0;
    }
    let wet_fraction = pervious_fraction + impervious_wet_fraction * impervious_fraction;
    // `:160` `.FMA (qgimp*fgimp, fwet, qgper*fgper) / fwetfac`
    let ground_specific_humidity =
        (input.impervious_specific_humidity * impervious_fraction).mul_add(
            impervious_wet_fraction,
            input.pervious_specific_humidity * pervious_fraction,
        ) / wet_fraction;
    let temperature_difference_k = input.reference_temperature_k - ground_temperature_k;
    let humidity_difference_kg_kg = input.reference_specific_humidity - ground_specific_humidity;
    // `:172` `.FMA (dth, .FMA (qm, 0.61, 1), dqh*(th*0.61))`
    let moist_factor = input.reference_specific_humidity.mul_add(f77(0.61), 1.0);
    let potential_factor = input.potential_temperature_k * f77(0.61);
    let virtual_temperature_difference_k = temperature_difference_k
        .mul_add(moist_factor, humidity_difference_kg_kg * potential_factor);
    let mut stability = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: input.reference_wind_m_s,
        potential_temperature_k: input.potential_temperature_k,
        reference_temperature_k: input.reference_temperature_k,
        virtual_potential_temperature_k: input.virtual_potential_temperature_k,
        temperature_difference_k,
        humidity_difference_kg_kg,
        virtual_temperature_difference_k,
        reference_height_m: input.wind_height_m,
        momentum_roughness_m,
    })?;
    let mut heat_roughness_m = momentum_roughness_m;
    let mut dimensionless_height = 0.0;
    let mut surface = None;
    let mut sign_changes = 0;
    let mut previous_obukhov_length_m = 0.0;
    for _ in 0..6 {
        let profile = monin_obukhov(MoninObukhovInput {
            wind_height_m: input.wind_height_m,
            temperature_height_m: input.temperature_height_m,
            humidity_height_m: input.humidity_height_m,
            displacement_height_m: 0.0,
            momentum_roughness_m,
            heat_roughness_m,
            moisture_roughness_m: heat_roughness_m,
            obukhov_length_m: stability.obukhov_length_m,
            stability_adjusted_wind_m_s: stability.stability_adjusted_wind_m_s,
            // 城市分支固定走 `Standard` 廓线，不读 `hpbl`。
            boundary_layer_height_m: None,
        })?;
        let temperature_scale_k = VON_KARMAN / profile.heat * temperature_difference_k;
        let moisture_scale = VON_KARMAN / profile.moisture * humidity_difference_kg_kg;
        heat_roughness_m = momentum_roughness_m
            / (f77(0.13)
                * (profile.friction_velocity_m_s * momentum_roughness_m / f77(1.5e-5))
                    .lpow(f77(0.45)))
            .exp();
        // `:193` `.FMA (1+0.61*qm, tstar, (th*0.61)*qstar)`
        let virtual_temperature_scale =
            moist_factor.mul_add(temperature_scale_k, potential_factor * moisture_scale);
        let raw_zeta = input.wind_height_m * VON_KARMAN * GRAVITY_M_S2 * virtual_temperature_scale
            / (profile.friction_velocity_m_s.powi(2) * input.virtual_potential_temperature_k);
        dimensionless_height = if raw_zeta >= 0.0 {
            raw_zeta.clamp(f77(1.0e-6), 2.0)
        } else {
            raw_zeta.clamp(-100.0, -f77(1.0e-6))
        };
        stability.obukhov_length_m = input.wind_height_m / dimensionless_height;
        stability.stability_adjusted_wind_m_s = if dimensionless_height >= 0.0 {
            input.reference_wind_m_s.max(f77(0.1))
        } else {
            let convective_velocity = (-GRAVITY_M_S2
                * profile.friction_velocity_m_s
                * virtual_temperature_scale
                * 1000.0
                / input.virtual_potential_temperature_k)
                .lpow(f77(1.0 / 3.0));
            // `:207` `sqrt(.FMA (ur, ur, wc*wc))`
            input
                .reference_wind_m_s
                .mul_add(
                    input.reference_wind_m_s,
                    convective_velocity * convective_velocity,
                )
                .sqrt()
        };
        if previous_obukhov_length_m * stability.obukhov_length_m < 0.0 {
            sign_changes += 1;
        }
        surface = Some((profile, temperature_scale_k, moisture_scale));
        if sign_changes >= 4 {
            break;
        }
        previous_obukhov_length_m = stability.obukhov_length_m;
    }
    let (profile, temperature_scale_k, moisture_scale) =
        surface.expect("urban ground-flux loop always performs at least one iteration");
    Ok(UrbanGroundFluxState {
        // `:223-224` `.FMA (tstar, fh2m/vonkar - fh/vonkar, thm)`
        reference_temperature_k: temperature_scale_k.mul_add(
            profile.heat_at_2m / VON_KARMAN - profile.heat / VON_KARMAN,
            input.reference_temperature_k,
        ),
        reference_specific_humidity: moisture_scale.mul_add(
            profile.moisture_at_2m / VON_KARMAN - profile.moisture / VON_KARMAN,
            input.reference_specific_humidity,
        ),
        momentum_roughness_m,
        heat_roughness_m,
        dimensionless_height,
        friction_velocity_m_s: profile.friction_velocity_m_s,
        moisture_scale,
        temperature_scale_k,
        momentum_similarity: profile.momentum,
        heat_similarity: profile.heat,
        moisture_similarity: profile.moisture,
    })
}

fn validate(input: UrbanGroundFluxInput) -> Result<()> {
    for value in [
        input.wind_height_m,
        input.temperature_height_m,
        input.humidity_height_m,
        input.reference_specific_humidity,
        input.reference_wind_m_s,
        input.reference_temperature_k,
        input.potential_temperature_k,
        input.virtual_potential_temperature_k,
        input.land_roughness_m,
        input.snow_roughness_m,
        input.impervious_snow_fraction,
        input.impervious_surface_liquid_water_kg_m2,
        input.impervious_surface_ice_kg_m2,
        input.impervious_temperature_k,
        input.pervious_temperature_k,
        input.impervious_specific_humidity,
        input.pervious_specific_humidity,
    ] {
        ensure!(value.is_finite(), "urban ground-flux inputs must be finite");
    }
    ensure!(
        input.wind_height_m > 0.0
            && input.temperature_height_m > 0.0
            && input.humidity_height_m > 0.0
            && input.reference_wind_m_s >= 0.0
            && input.virtual_potential_temperature_k > 0.0
            && input.land_roughness_m > 0.0
            && input.snow_roughness_m > 0.0
            && (0.0..=1.0).contains(&input.impervious_snow_fraction)
            && input.impervious_surface_liquid_water_kg_m2 >= 0.0
            && input.impervious_surface_ice_kg_m2 >= 0.0,
        "urban ground-flux physical inputs are invalid"
    );
    ensure!(
        // 墙的 `fcover(1:2) = 4*fw*HL*fb` 是墙面积对地面积之比，可以大于 1（`MOD_Urban_Longwave.F90:216`）。
        input
            .cover_fraction
            .iter()
            .enumerate()
            .all(|(surface, value)| {
                value.is_finite() && *value >= 0.0 && (matches!(surface, 1 | 2) || *value <= 1.0)
            })
            && input.cover_fraction[0] < 1.0
            && input.cover_fraction[3] + input.cover_fraction[4] > 0.0,
        "urban ground-flux cover fractions are invalid: {:?}",
        input.cover_fraction
    );
    Ok(())
}

#[cfg(test)]
#[path = "urban_ground_flux_tests.rs"]
mod urban_ground_flux_tests;
