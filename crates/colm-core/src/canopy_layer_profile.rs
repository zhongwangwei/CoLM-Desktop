//! Canopy wind and exchange-coefficient profiles from `MOD_CanopyLayerProfile.F90`.
//!
//! Roughness and displacement are in [`crate::canopy_roughness`].  These routines
//! deliberately share typed profile inputs so the leaf-temperature solver and any
//! initialization diagnostic use identical canopy transport equations.

use anyhow::{ensure, Result};

use crate::{integrated_monin_obukhov_diffusivity, monin_obukhov_diffusivity};

const fn f77(value: f32) -> f64 {
    value as f64
}

const INTEGRATION_STEP_M: f64 = f77(0.001);
const ROOT_TOLERANCE_M: f64 = f77(0.01);
const DISPLACEMENT_BLEND_CENTER_M: f64 = f77(0.4);
const DISPLACEMENT_BLEND_WIDTH_M: f64 = f77(0.08);

/// Inputs shared by CoLM's `uprofile`, `uintegral`, and `ueffect` functions.
#[derive(Debug, Clone, Copy)]
pub struct CanopyWindProfileInput {
    pub wind_at_canopy_top_m_s: f64,
    pub canopy_cover_fraction: f64,
    pub canopy_blend_weight: f64,
    pub attenuation_coefficient: f64,
    pub ground_momentum_roughness_m: f64,
    pub canopy_top_height_m: f64,
    pub canopy_bottom_height_m: f64,
}

/// Inputs shared by CoLM's `kprofile`, `kintegral`, and `frd` functions.
#[derive(Debug, Clone, Copy)]
pub struct CanopyDiffusivityProfileInput {
    pub diffusivity_at_canopy_top_m2_s: f64,
    pub canopy_cover_fraction: f64,
    pub canopy_blend_weight: f64,
    pub attenuation_coefficient: f64,
    pub displacement_height_m: f64,
    pub canopy_top_height_m: f64,
    pub canopy_bottom_height_m: f64,
    pub obukhov_length_m: f64,
    pub friction_velocity_m_s: f64,
}

/// The at-most-two roots retained by CoLM's recursive profile splitters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyProfileRoots {
    pub heights_m: [f64; 2],
    pub count: usize,
}

/// Port of `MOD_CanopyLayerProfile:uprofile`.
pub fn canopy_wind_speed(input: CanopyWindProfileInput, height_m: f64) -> Result<f64> {
    validate_wind_profile(input, height_m)?;
    let logarithmic = input.wind_at_canopy_top_m_s
        * (height_m / input.ground_momentum_roughness_m).ln()
        / (input.canopy_top_height_m / input.ground_momentum_roughness_m).ln();
    let exponential = input.wind_at_canopy_top_m_s
        * (-input.attenuation_coefficient
            * (1.0
                - (height_m - input.canopy_bottom_height_m)
                    / (input.canopy_top_height_m - input.canopy_bottom_height_m)))
            .exp();
    Ok(
        input.canopy_blend_weight * input.canopy_cover_fraction * exponential.min(logarithmic)
            + (1.0 - input.canopy_blend_weight * input.canopy_cover_fraction) * logarithmic,
    )
}

/// Port of `MOD_CanopyLayerProfile:kprofile`.
pub fn canopy_diffusivity(input: CanopyDiffusivityProfileInput, height_m: f64) -> Result<f64> {
    validate_diffusivity_profile(input, height_m)?;
    let linear = input.diffusivity_at_canopy_top_m2_s * height_m / input.canopy_top_height_m;
    let blend = diffusivity_blend(input.displacement_height_m);
    let monin_obukhov = monin_obukhov_diffusivity(
        0.0,
        input.obukhov_length_m,
        input.friction_velocity_m_s,
        height_m,
    )?;
    let combined = 1.0 / (blend / linear + (1.0 - blend) / monin_obukhov);
    let exponential = input.diffusivity_at_canopy_top_m2_s
        * (-input.attenuation_coefficient * (input.canopy_top_height_m - height_m)
            / (input.canopy_top_height_m - input.canopy_bottom_height_m))
            .exp();
    Ok(1.0
        / (input.canopy_blend_weight * input.canopy_cover_fraction / exponential.min(combined)
            + (1.0 - input.canopy_blend_weight * input.canopy_cover_fraction) / combined))
}

/// Port of `MOD_CanopyLayerProfile:uintegral`; returns the canopy-layer mean wind.
pub fn mean_canopy_wind(input: CanopyWindProfileInput) -> Result<f64> {
    mean_canopy_wind_between(
        input,
        input.canopy_top_height_m,
        input.canopy_bottom_height_m,
    )
}

/// Port of `MOD_CanopyLayerProfile:uintegralz`; returns the selected-layer mean wind.
pub fn mean_canopy_wind_between(
    input: CanopyWindProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<f64> {
    validate_wind_interval(input, top_height_m, bottom_height_m)?;
    let mut step = INTEGRATION_STEP_M;
    let intervals = ((top_height_m - bottom_height_m) / step) as usize + 1;
    let mut mean = 0.0;
    for index in 1..=intervals {
        let height = if index < intervals {
            top_height_m - (index as f64 - 0.5) * step
        } else {
            step = top_height_m - bottom_height_m - (intervals - 1) as f64 * step;
            bottom_height_m + 0.5 * step
        };
        mean +=
            canopy_wind_speed(input, height)?.max(0.0) * step / (top_height_m - bottom_height_m);
    }
    Ok(mean)
}

/// Port of `MOD_CanopyLayerProfile:ueffect`.
pub fn effective_canopy_wind(input: CanopyWindProfileInput) -> Result<f64> {
    effective_canopy_wind_between(
        input,
        input.canopy_top_height_m,
        input.canopy_bottom_height_m,
    )
}

/// Port of `MOD_CanopyLayerProfile:ueffectz`.
pub fn effective_canopy_wind_between(
    input: CanopyWindProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<f64> {
    validate_wind_interval(input, top_height_m, bottom_height_m)?;
    let roots = canopy_wind_roots_between(input, top_height_m, bottom_height_m)?;
    let integral = match roots.count {
        0 => canopy_wind_integral(input, top_height_m, bottom_height_m)?,
        1 => {
            canopy_wind_integral(input, top_height_m, roots.heights_m[0])?
                + canopy_wind_integral(input, roots.heights_m[0], bottom_height_m)?
        }
        _ => {
            canopy_wind_integral(input, top_height_m, roots.heights_m[0])?
                + canopy_wind_integral(input, roots.heights_m[0], roots.heights_m[1])?
                + canopy_wind_integral(input, roots.heights_m[1], bottom_height_m)?
        }
    };
    Ok(integral / (top_height_m - bottom_height_m))
}

/// Port of `MOD_CanopyLayerProfile:fuint`.
pub fn canopy_wind_integral(
    input: CanopyWindProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<f64> {
    validate_wind_interval(input, top_height_m, bottom_height_m)?;
    let logarithmic = input.wind_at_canopy_top_m_s
        / (input.canopy_top_height_m / input.ground_momentum_roughness_m).ln()
        * (top_height_m * (top_height_m / input.ground_momentum_roughness_m).ln()
            - bottom_height_m * (bottom_height_m / input.ground_momentum_roughness_m).ln()
            + bottom_height_m
            - top_height_m);
    if canopy_wind_difference(input, 0.5 * (top_height_m + bottom_height_m))? <= 0.0 {
        let exponential = input.wind_at_canopy_top_m_s
            * (input.canopy_top_height_m - input.canopy_bottom_height_m)
            / input.attenuation_coefficient
            * ((-input.attenuation_coefficient * (input.canopy_top_height_m - top_height_m)
                / (input.canopy_top_height_m - input.canopy_bottom_height_m))
                .exp()
                - (-input.attenuation_coefficient * (input.canopy_top_height_m - bottom_height_m)
                    / (input.canopy_top_height_m - input.canopy_bottom_height_m))
                    .exp());
        Ok(
            input.canopy_blend_weight * input.canopy_cover_fraction * exponential
                + (1.0 - input.canopy_blend_weight * input.canopy_cover_fraction) * logarithmic,
        )
    } else {
        Ok(logarithmic)
    }
}

/// Port of `MOD_CanopyLayerProfile:udiff`.
pub fn canopy_wind_difference(input: CanopyWindProfileInput, height_m: f64) -> Result<f64> {
    validate_wind_profile(input, height_m)?;
    let exponential = input.wind_at_canopy_top_m_s
        * (-input.attenuation_coefficient * (input.canopy_top_height_m - height_m)
            / (input.canopy_top_height_m - input.canopy_bottom_height_m))
            .exp();
    let logarithmic = input.wind_at_canopy_top_m_s
        * (height_m / input.ground_momentum_roughness_m).ln()
        / (input.canopy_top_height_m / input.ground_momentum_roughness_m).ln();
    Ok(exponential - logarithmic)
}

/// Port of `MOD_CanopyLayerProfile:ufindroots`.
pub fn canopy_wind_roots_between(
    input: CanopyWindProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<CanopyProfileRoots> {
    validate_wind_interval(input, top_height_m, bottom_height_m)?;
    let mut roots = CanopyProfileRoots {
        heights_m: [0.0; 2],
        count: 0,
    };
    find_wind_roots(
        input,
        top_height_m,
        bottom_height_m,
        0.5 * (top_height_m + bottom_height_m),
        &mut roots,
    )?;
    Ok(roots)
}

/// Port of `MOD_CanopyLayerProfile:kintegral`; returns `∫ dz / K`.
pub fn canopy_diffusivity_resistance(
    input: CanopyDiffusivityProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<f64> {
    validate_diffusivity_interval(input, top_height_m, bottom_height_m)?;
    let mut step = INTEGRATION_STEP_M;
    let intervals = ((top_height_m - bottom_height_m) / step) as usize + 1;
    let mut resistance = 0.0;
    for index in 1..=intervals {
        let height = if index < intervals {
            top_height_m - (index as f64 - 0.5) * step
        } else {
            step = top_height_m - bottom_height_m - (intervals - 1) as f64 * step;
            bottom_height_m + 0.5 * step
        };
        resistance += step / canopy_diffusivity(input, height)?;
    }
    Ok(resistance)
}

/// Port of `MOD_CanopyLayerProfile:frd`.
pub fn canopy_diffusivity_resistance_analytic(
    input: CanopyDiffusivityProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
    heat_roughness_m: f64,
) -> Result<f64> {
    validate_diffusivity_interval(input, top_height_m, bottom_height_m)?;
    ensure!(
        heat_roughness_m.is_finite() && heat_roughness_m > 0.0,
        "canopy heat roughness is invalid"
    );
    let blend = diffusivity_blend(input.displacement_height_m);
    let roots = canopy_diffusivity_roots_between(input, top_height_m, bottom_height_m)?;
    match roots.count {
        0 => canopy_diffusivity_profile_integral(
            input,
            top_height_m,
            bottom_height_m,
            heat_roughness_m,
            blend,
        ),
        1 => Ok(canopy_diffusivity_profile_integral(
            input,
            top_height_m,
            roots.heights_m[0],
            heat_roughness_m,
            blend,
        )? + canopy_diffusivity_profile_integral(
            input,
            roots.heights_m[0],
            bottom_height_m,
            heat_roughness_m,
            blend,
        )?),
        _ => Ok(canopy_diffusivity_profile_integral(
            input,
            top_height_m,
            roots.heights_m[0],
            heat_roughness_m,
            blend,
        )? + canopy_diffusivity_profile_integral(
            input,
            roots.heights_m[0],
            roots.heights_m[1],
            heat_roughness_m,
            blend,
        )? + canopy_diffusivity_profile_integral(
            input,
            roots.heights_m[1],
            bottom_height_m,
            heat_roughness_m,
            blend,
        )?),
    }
}

/// Port of `MOD_CanopyLayerProfile:fkint`.
pub fn canopy_diffusivity_profile_integral(
    input: CanopyDiffusivityProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
    heat_roughness_m: f64,
    blend: f64,
) -> Result<f64> {
    validate_diffusivity_interval(input, top_height_m, bottom_height_m)?;
    ensure!(
        heat_roughness_m.is_finite()
            && heat_roughness_m > 0.0
            && blend.is_finite()
            && (0.0..=1.0).contains(&blend),
        "canopy diffusivity integral inputs are invalid"
    );
    let combined = blend * input.canopy_top_height_m / input.diffusivity_at_canopy_top_m2_s
        * (top_height_m.ln() - bottom_height_m.ln())
        + (1.0 - blend)
            * integrated_monin_obukhov_diffusivity(
                0.0,
                heat_roughness_m,
                input.obukhov_length_m,
                input.friction_velocity_m_s,
                top_height_m,
                bottom_height_m,
            )?;
    if canopy_diffusivity_difference(input, 0.5 * (top_height_m + bottom_height_m), blend)? <= 0.0 {
        let exponential = if input.attenuation_coefficient > 0.0 {
            -(input.canopy_top_height_m - input.canopy_bottom_height_m)
                / input.attenuation_coefficient
                / input.diffusivity_at_canopy_top_m2_s
                * ((input.attenuation_coefficient * (input.canopy_top_height_m - top_height_m)
                    / (input.canopy_top_height_m - input.canopy_bottom_height_m))
                    .exp()
                    - (input.attenuation_coefficient
                        * (input.canopy_top_height_m - bottom_height_m)
                        / (input.canopy_top_height_m - input.canopy_bottom_height_m))
                        .exp())
        } else {
            (top_height_m - bottom_height_m) / input.diffusivity_at_canopy_top_m2_s
        };
        Ok(
            input.canopy_blend_weight * input.canopy_cover_fraction * exponential
                + (1.0 - input.canopy_blend_weight * input.canopy_cover_fraction) * combined,
        )
    } else {
        Ok(combined)
    }
}

/// Port of `MOD_CanopyLayerProfile:kdiff`.
pub fn canopy_diffusivity_difference(
    input: CanopyDiffusivityProfileInput,
    height_m: f64,
    blend: f64,
) -> Result<f64> {
    validate_diffusivity_profile(input, height_m)?;
    ensure!(
        blend.is_finite() && (0.0..=1.0).contains(&blend),
        "canopy diffusivity blend is invalid"
    );
    let exponential = input.diffusivity_at_canopy_top_m2_s
        * (-input.attenuation_coefficient * (input.canopy_top_height_m - height_m)
            / (input.canopy_top_height_m - input.canopy_bottom_height_m))
            .exp();
    let linear = input.diffusivity_at_canopy_top_m2_s * height_m / input.canopy_top_height_m;
    let monin_obukhov = monin_obukhov_diffusivity(
        0.0,
        input.obukhov_length_m,
        input.friction_velocity_m_s,
        height_m,
    )?;
    let combined = 1.0 / (blend / linear + (1.0 - blend) / monin_obukhov);
    Ok(exponential - combined)
}

/// Port of `MOD_CanopyLayerProfile:kfindroots`.
pub fn canopy_diffusivity_roots_between(
    input: CanopyDiffusivityProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<CanopyProfileRoots> {
    validate_diffusivity_interval(input, top_height_m, bottom_height_m)?;
    let mut roots = CanopyProfileRoots {
        heights_m: [0.0; 2],
        count: 0,
    };
    let blend = diffusivity_blend(input.displacement_height_m);
    find_diffusivity_roots(
        input,
        top_height_m,
        bottom_height_m,
        0.5 * (top_height_m + bottom_height_m),
        blend,
        &mut roots,
    )?;
    Ok(roots)
}

fn find_wind_roots(
    input: CanopyWindProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
    middle_height_m: f64,
    roots: &mut CanopyProfileRoots,
) -> Result<()> {
    let upper = canopy_wind_difference(input, top_height_m)?;
    let middle = canopy_wind_difference(input, middle_height_m)?;
    if upper * middle == 0.0 {
        if middle == 0.0 {
            add_root(roots, middle_height_m);
        }
    } else if upper * middle < 0.0 {
        if top_height_m - middle_height_m < ROOT_TOLERANCE_M {
            add_root(roots, 0.5 * (top_height_m + middle_height_m));
        } else {
            find_wind_roots(
                input,
                top_height_m,
                middle_height_m,
                0.5 * (top_height_m + middle_height_m),
                roots,
            )?;
        }
    }
    let middle = canopy_wind_difference(input, middle_height_m)?;
    let lower = canopy_wind_difference(input, bottom_height_m)?;
    if middle * lower == 0.0 {
        if middle == 0.0 {
            add_root(roots, middle_height_m);
        }
    } else if middle * lower < 0.0 {
        if middle_height_m - bottom_height_m < ROOT_TOLERANCE_M {
            add_root(roots, 0.5 * (middle_height_m + bottom_height_m));
        } else {
            find_wind_roots(
                input,
                middle_height_m,
                bottom_height_m,
                0.5 * (middle_height_m + bottom_height_m),
                roots,
            )?;
        }
    }
    Ok(())
}

fn find_diffusivity_roots(
    input: CanopyDiffusivityProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
    middle_height_m: f64,
    blend: f64,
    roots: &mut CanopyProfileRoots,
) -> Result<()> {
    let upper = canopy_diffusivity_difference(input, top_height_m, blend)?;
    let middle = canopy_diffusivity_difference(input, middle_height_m, blend)?;
    if upper * middle == 0.0 {
        if middle == 0.0 {
            add_root(roots, middle_height_m);
        }
    } else if upper * middle < 0.0 {
        if top_height_m - middle_height_m < ROOT_TOLERANCE_M {
            add_root(roots, 0.5 * (top_height_m + middle_height_m));
        } else {
            find_diffusivity_roots(
                input,
                top_height_m,
                middle_height_m,
                0.5 * (top_height_m + middle_height_m),
                blend,
                roots,
            )?;
        }
    }
    let middle = canopy_diffusivity_difference(input, middle_height_m, blend)?;
    let lower = canopy_diffusivity_difference(input, bottom_height_m, blend)?;
    if middle * lower == 0.0 {
        if middle == 0.0 {
            add_root(roots, middle_height_m);
        }
    } else if middle * lower < 0.0 {
        if middle_height_m - bottom_height_m < ROOT_TOLERANCE_M {
            add_root(roots, 0.5 * (middle_height_m + bottom_height_m));
        } else {
            find_diffusivity_roots(
                input,
                middle_height_m,
                bottom_height_m,
                0.5 * (middle_height_m + bottom_height_m),
                blend,
                roots,
            )?;
        }
    }
    Ok(())
}

fn add_root(roots: &mut CanopyProfileRoots, height_m: f64) {
    if roots.count < roots.heights_m.len() {
        roots.heights_m[roots.count] = height_m;
        roots.count += 1;
    }
}

fn diffusivity_blend(displacement_height_m: f64) -> f64 {
    1.0 / (1.0
        + (-(displacement_height_m - DISPLACEMENT_BLEND_CENTER_M) / DISPLACEMENT_BLEND_WIDTH_M)
            .exp())
}

fn validate_wind_profile(input: CanopyWindProfileInput, height_m: f64) -> Result<()> {
    ensure!(
        [
            input.wind_at_canopy_top_m_s,
            input.canopy_cover_fraction,
            input.canopy_blend_weight,
            input.attenuation_coefficient,
            input.ground_momentum_roughness_m,
            input.canopy_top_height_m,
            input.canopy_bottom_height_m,
            height_m,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.ground_momentum_roughness_m > 0.0
            && input.attenuation_coefficient > 0.0
            && input.canopy_top_height_m > input.canopy_bottom_height_m
            && input.canopy_bottom_height_m >= input.ground_momentum_roughness_m
            && height_m >= input.ground_momentum_roughness_m,
        "canopy wind profile inputs are invalid"
    );
    Ok(())
}

fn validate_wind_interval(
    input: CanopyWindProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<()> {
    validate_wind_profile(input, top_height_m)?;
    validate_wind_profile(input, bottom_height_m)?;
    ensure!(
        top_height_m > bottom_height_m,
        "canopy wind interval must have positive height"
    );
    Ok(())
}

fn validate_diffusivity_profile(input: CanopyDiffusivityProfileInput, height_m: f64) -> Result<()> {
    ensure!(
        [
            input.diffusivity_at_canopy_top_m2_s,
            input.canopy_cover_fraction,
            input.canopy_blend_weight,
            input.attenuation_coefficient,
            input.displacement_height_m,
            input.canopy_top_height_m,
            input.canopy_bottom_height_m,
            input.obukhov_length_m,
            input.friction_velocity_m_s,
            height_m,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.diffusivity_at_canopy_top_m2_s > 0.0
            && input.canopy_top_height_m > input.canopy_bottom_height_m
            && input.canopy_bottom_height_m > 0.0
            && input.obukhov_length_m != 0.0
            && input.friction_velocity_m_s > 0.0
            && height_m > 0.0,
        "canopy diffusivity profile inputs are invalid"
    );
    Ok(())
}

fn validate_diffusivity_interval(
    input: CanopyDiffusivityProfileInput,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<()> {
    validate_diffusivity_profile(input, top_height_m)?;
    validate_diffusivity_profile(input, bottom_height_m)?;
    ensure!(
        top_height_m > bottom_height_m,
        "canopy diffusivity interval must have positive height"
    );
    Ok(())
}

#[cfg(test)]
#[path = "canopy_layer_profile_tests.rs"]
mod canopy_layer_profile_tests;
