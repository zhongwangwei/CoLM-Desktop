//! Surface-layer similarity functions from `MOD_FrictionVelocity.F90`.
//!
//! Ground, lake, urban, and canopy fluxes call these pure kernels rather than
//! carrying separate translations of the Monin-Obukhov profile equations.

use anyhow::{ensure, Result};

// CoLM's physical constants are unsuffixed Fortran literals assigned to r8.
// Preserve that source rounding for bit-level differential checks.
const fn f77(value: f32) -> f64 {
    value as f64
}

const VON_KARMAN: f64 = f77(0.4);
const GRAVITY_M_S2: f64 = f77(9.80616);

/// Inputs shared by CoLM's `moninobuk` and `moninobukm` routines.
#[derive(Debug, Clone, Copy)]
pub struct MoninObukhovInput {
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub displacement_height_m: f64,
    pub momentum_roughness_m: f64,
    pub heat_roughness_m: f64,
    pub moisture_roughness_m: f64,
    pub obukhov_length_m: f64,
    pub stability_adjusted_wind_m_s: f64,
}

/// CoLM's selectable surface-layer profile. `LargeEddy` is
/// `MOD_TurbulenceLEddy`'s LZD2022 branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceLayerScheme {
    Standard,
    LargeEddy { boundary_layer_height_m: f64 },
}

/// Outputs of CoLM's `moninobuk` routine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoninObukhovState {
    pub friction_velocity_m_s: f64,
    pub heat_at_2m: f64,
    pub moisture_at_2m: f64,
    pub momentum_at_10m: f64,
    pub momentum: f64,
    pub heat: f64,
    pub moisture: f64,
}

/// Additional top-layer geometry required by CoLM's `moninobukm` routine.
#[derive(Debug, Clone, Copy)]
pub struct CanopyMoninObukhovInput {
    pub surface: MoninObukhovInput,
    pub top_layer_displacement_m: f64,
    pub top_layer_roughness_m: f64,
    pub canopy_top_height_m: f64,
}

/// Outputs of CoLM's `moninobukm` routine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyMoninObukhovState {
    pub surface: MoninObukhovState,
    pub momentum_at_canopy_top: f64,
    pub heat_at_top_layer: f64,
    pub moisture_at_top_layer: f64,
    pub canopy_top_heat_similarity: f64,
}

/// Inputs to CoLM's `moninobukini` first-guess routine.
#[derive(Debug, Clone, Copy)]
pub struct MoninObukhovInitialInput {
    pub reference_wind_m_s: f64,
    pub potential_temperature_k: f64,
    pub reference_temperature_k: f64,
    pub virtual_potential_temperature_k: f64,
    pub temperature_difference_k: f64,
    pub humidity_difference_kg_kg: f64,
    pub virtual_temperature_difference_k: f64,
    pub reference_height_m: f64,
    pub momentum_roughness_m: f64,
}

/// Initial wind speed and Obukhov length from CoLM's `moninobukini`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoninObukhovInitialState {
    pub stability_adjusted_wind_m_s: f64,
    pub obukhov_length_m: f64,
}

/// Port of `MOD_FrictionVelocity:moninobuk`.
pub fn monin_obukhov(input: MoninObukhovInput) -> Result<MoninObukhovState> {
    monin_obukhov_with_scheme(input, SurfaceLayerScheme::Standard)
}

/// Ports `moninobuk` and `MOD_TurbulenceLEddy:moninobuk_leddy` through one
/// profile contract, so a caller's CBL switch changes only the momentum branch.
pub fn monin_obukhov_with_scheme(
    input: MoninObukhovInput,
    scheme: SurfaceLayerScheme,
) -> Result<MoninObukhovState> {
    validate_surface(input)?;
    let momentum_scheme = MomentumScheme::new(input, scheme)?;
    let momentum = momentum_scheme.integral(
        input.wind_height_m - input.displacement_height_m,
        input.momentum_roughness_m,
        input.obukhov_length_m,
    );
    let friction_velocity_m_s = VON_KARMAN * input.stability_adjusted_wind_m_s / momentum;
    let momentum_at_10m = momentum_scheme.integral(
        10.0 + input.momentum_roughness_m,
        input.momentum_roughness_m,
        input.obukhov_length_m,
    );
    let heat = heat_integral(
        input.temperature_height_m - input.displacement_height_m,
        input.heat_roughness_m,
        input.obukhov_length_m,
    );
    let heat_at_2m = heat_integral(
        2.0 + input.heat_roughness_m,
        input.heat_roughness_m,
        input.obukhov_length_m,
    );
    let moisture = heat_integral(
        input.humidity_height_m - input.displacement_height_m,
        input.moisture_roughness_m,
        input.obukhov_length_m,
    );
    let moisture_at_2m = heat_integral(
        2.0 + input.heat_roughness_m,
        input.moisture_roughness_m,
        input.obukhov_length_m,
    );
    Ok(MoninObukhovState {
        friction_velocity_m_s,
        heat_at_2m,
        moisture_at_2m,
        momentum_at_10m,
        momentum,
        heat,
        moisture,
    })
}

/// Port of `MOD_FrictionVelocity:moninobukm` for canopy-layer callers.
pub fn canopy_monin_obukhov(input: CanopyMoninObukhovInput) -> Result<CanopyMoninObukhovState> {
    canopy_monin_obukhov_with_scheme(input, SurfaceLayerScheme::Standard)
}

/// Ports `moninobukm` and `MOD_TurbulenceLEddy:moninobukm_leddy` through the
/// same canopy profile interface.
pub fn canopy_monin_obukhov_with_scheme(
    input: CanopyMoninObukhovInput,
    scheme: SurfaceLayerScheme,
) -> Result<CanopyMoninObukhovState> {
    validate_surface(input.surface)?;
    ensure!(
        input.top_layer_displacement_m.is_finite()
            && input.top_layer_roughness_m.is_finite()
            && input.top_layer_roughness_m > 0.0
            && input.canopy_top_height_m.is_finite()
            && input.canopy_top_height_m > input.surface.displacement_height_m
            && input.top_layer_displacement_m + input.top_layer_roughness_m
                > input.surface.displacement_height_m,
        "canopy Monin-Obukhov geometry is invalid"
    );
    let surface = monin_obukhov_with_scheme(input.surface, scheme)?;
    let momentum_at_canopy_top = MomentumScheme::new(input.surface, scheme)?.integral(
        input.canopy_top_height_m - input.surface.displacement_height_m,
        input.surface.momentum_roughness_m,
        input.surface.obukhov_length_m,
    );
    let heat_at_top_layer = heat_integral(
        input.top_layer_displacement_m + input.top_layer_roughness_m
            - input.surface.displacement_height_m,
        input.surface.heat_roughness_m,
        input.surface.obukhov_length_m,
    );
    let moisture_at_top_layer = heat_integral(
        input.top_layer_displacement_m + input.top_layer_roughness_m
            - input.surface.displacement_height_m,
        input.surface.moisture_roughness_m,
        input.surface.obukhov_length_m,
    );
    let canopy_top_heat_similarity = heat_similarity(
        (input.canopy_top_height_m - input.surface.displacement_height_m)
            / input.surface.obukhov_length_m,
    );
    Ok(CanopyMoninObukhovState {
        surface,
        momentum_at_canopy_top,
        heat_at_top_layer,
        moisture_at_top_layer,
        canopy_top_heat_similarity,
    })
}

/// Port of `MOD_FrictionVelocity:kmoninobuk`.
pub fn monin_obukhov_diffusivity(
    displacement_height_m: f64,
    obukhov_length_m: f64,
    friction_velocity_m_s: f64,
    height_m: f64,
) -> Result<f64> {
    ensure!(
        [
            displacement_height_m,
            obukhov_length_m,
            friction_velocity_m_s,
            height_m,
        ]
        .iter()
        .all(|value| value.is_finite())
            && obukhov_length_m != 0.0
            && friction_velocity_m_s >= 0.0,
        "Monin-Obukhov diffusivity inputs are invalid"
    );
    if height_m <= displacement_height_m {
        return Ok(0.0);
    }
    Ok(
        VON_KARMAN * (height_m - displacement_height_m) * friction_velocity_m_s
            / heat_similarity((height_m - displacement_height_m) / obukhov_length_m),
    )
}

/// Port of `MOD_FrictionVelocity:kintmoninobuk`.
pub fn integrated_monin_obukhov_diffusivity(
    displacement_height_m: f64,
    heat_roughness_m: f64,
    obukhov_length_m: f64,
    friction_velocity_m_s: f64,
    top_height_m: f64,
    bottom_height_m: f64,
) -> Result<f64> {
    ensure!(
        [
            displacement_height_m,
            heat_roughness_m,
            obukhov_length_m,
            friction_velocity_m_s,
            top_height_m,
            bottom_height_m,
        ]
        .iter()
        .all(|value| value.is_finite())
            && heat_roughness_m > 0.0
            && obukhov_length_m != 0.0
            && friction_velocity_m_s > 0.0
            && top_height_m > displacement_height_m
            && bottom_height_m > displacement_height_m,
        "integrated Monin-Obukhov diffusivity inputs are invalid"
    );
    Ok((heat_integral(
        top_height_m - displacement_height_m,
        heat_roughness_m,
        obukhov_length_m,
    ) - heat_integral(
        bottom_height_m - displacement_height_m,
        heat_roughness_m,
        obukhov_length_m,
    )) / (VON_KARMAN * friction_velocity_m_s))
}

/// Port of `MOD_FrictionVelocity:moninobukini`.
pub fn initialize_monin_obukhov(
    input: MoninObukhovInitialInput,
) -> Result<MoninObukhovInitialState> {
    ensure!(
        [
            input.reference_wind_m_s,
            input.potential_temperature_k,
            input.reference_temperature_k,
            input.virtual_potential_temperature_k,
            input.temperature_difference_k,
            input.humidity_difference_kg_kg,
            input.virtual_temperature_difference_k,
            input.reference_height_m,
            input.momentum_roughness_m,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.reference_wind_m_s >= 0.0
            && input.virtual_potential_temperature_k > 0.0
            && input.reference_height_m > input.momentum_roughness_m
            && input.momentum_roughness_m > 0.0,
        "initial Monin-Obukhov inputs are invalid"
    );
    let stability_adjusted_wind_m_s = if input.virtual_temperature_difference_k >= 0.0 {
        input.reference_wind_m_s.max(f77(0.1))
    } else {
        (input.reference_wind_m_s.powi(2) + 0.5_f64.powi(2)).sqrt()
    };
    let richardson =
        GRAVITY_M_S2 * input.reference_height_m * input.virtual_temperature_difference_k
            / (input.virtual_potential_temperature_k * stability_adjusted_wind_m_s.powi(2));
    let zeta = if richardson >= 0.0 {
        (richardson * (input.reference_height_m / input.momentum_roughness_m).ln()
            / (1.0 - 5.0 * richardson.min(f77(0.19))))
        .clamp(f77(1.0e-6), 2.0)
    } else {
        (richardson * (input.reference_height_m / input.momentum_roughness_m).ln())
            .clamp(-100.0, -f77(1.0e-6))
    };
    Ok(MoninObukhovInitialState {
        stability_adjusted_wind_m_s,
        obukhov_length_m: input.reference_height_m / zeta,
    })
}

#[derive(Debug, Clone, Copy)]
enum MomentumScheme {
    Standard,
    LargeEddy { transition: f64, coefficient: f64 },
}

impl MomentumScheme {
    fn new(input: MoninObukhovInput, scheme: SurfaceLayerScheme) -> Result<Self> {
        match scheme {
            SurfaceLayerScheme::Standard => Ok(Self::Standard),
            SurfaceLayerScheme::LargeEddy {
                boundary_layer_height_m,
            } => {
                ensure!(
                    boundary_layer_height_m.is_finite() && boundary_layer_height_m > 0.0,
                    "large-eddy boundary-layer height must be positive"
                );
                let boundary_zeta = (5.0 * input.wind_height_m).max(boundary_layer_height_m)
                    / input.obukhov_length_m;
                let boundary_zeta = if boundary_zeta >= 0.0 {
                    boundary_zeta.clamp(f77(1.0e-5), 200.0)
                } else {
                    boundary_zeta.clamp(f77(-1.0e4), f77(-1.0e-5))
                };
                let coefficient = f77(0.0047) * -boundary_zeta + f77(0.1854);
                let transition = (0.5
                    * coefficient.powi(4)
                    * (-16.0 - (256.0 + 4.0 / coefficient.powi(4)).sqrt()))
                .min(f77(-0.13));
                Ok(Self::LargeEddy {
                    transition,
                    coefficient: coefficient.max(f77(0.2722)),
                })
            }
        }
    }

    fn integral(self, distance_m: f64, roughness_m: f64, obukhov_length_m: f64) -> f64 {
        match self {
            Self::Standard => momentum_integral(distance_m, roughness_m, obukhov_length_m),
            Self::LargeEddy {
                transition,
                coefficient,
            } => {
                let zeta = distance_m / obukhov_length_m;
                if zeta < transition {
                    (transition * obukhov_length_m / roughness_m).ln() - psi(1, transition)
                        + psi(1, roughness_m / obukhov_length_m)
                        - 2.0 * coefficient * ((-zeta).powf(-0.5) - (-transition).powf(-0.5))
                } else if zeta < 0.0 {
                    (distance_m / roughness_m).ln() - psi(1, zeta)
                        + psi(1, roughness_m / obukhov_length_m)
                } else if zeta <= 1.0 {
                    (distance_m / roughness_m).ln() + 5.0 * zeta
                        - 5.0 * roughness_m / obukhov_length_m
                } else {
                    (obukhov_length_m / roughness_m).ln() + 5.0
                        - 5.0 * roughness_m / obukhov_length_m
                        + (5.0 * zeta.ln() + zeta - 1.0)
                }
            }
        }
    }
}

fn momentum_integral(distance_m: f64, roughness_m: f64, obukhov_length_m: f64) -> f64 {
    let zeta = distance_m / obukhov_length_m;
    if zeta < -f77(1.574) {
        (-f77(1.574) * obukhov_length_m / roughness_m).ln() - psi(1, -f77(1.574))
            + psi(1, roughness_m / obukhov_length_m)
            + f77(1.14) * ((-zeta).powf(f77(0.333)) - f77(1.574).powf(f77(0.333)))
    } else if zeta < 0.0 {
        (distance_m / roughness_m).ln() - psi(1, zeta) + psi(1, roughness_m / obukhov_length_m)
    } else if zeta <= 1.0 {
        (distance_m / roughness_m).ln() + 5.0 * zeta - 5.0 * roughness_m / obukhov_length_m
    } else {
        (obukhov_length_m / roughness_m).ln() + 5.0 - 5.0 * roughness_m / obukhov_length_m
            + (5.0 * zeta.ln() + zeta - 1.0)
    }
}

fn heat_integral(distance_m: f64, roughness_m: f64, obukhov_length_m: f64) -> f64 {
    let zeta = distance_m / obukhov_length_m;
    if zeta < -f77(0.465) {
        (-f77(0.465) * obukhov_length_m / roughness_m).ln() - psi(2, -f77(0.465))
            + psi(2, roughness_m / obukhov_length_m)
            + f77(0.8) * (f77(0.465).powf(-f77(0.333)) - (-zeta).powf(-f77(0.333)))
    } else if zeta < 0.0 {
        (distance_m / roughness_m).ln() - psi(2, zeta) + psi(2, roughness_m / obukhov_length_m)
    } else if zeta <= 1.0 {
        (distance_m / roughness_m).ln() + 5.0 * zeta - 5.0 * roughness_m / obukhov_length_m
    } else {
        (obukhov_length_m / roughness_m).ln() + 5.0 - 5.0 * roughness_m / obukhov_length_m
            + (5.0 * zeta.ln() + zeta - 1.0)
    }
}

fn heat_similarity(zeta: f64) -> f64 {
    if zeta < -f77(0.465) {
        f77(0.9) * VON_KARMAN.powf(f77(1.333)) * (-zeta).powf(-f77(0.333))
    } else if zeta < 0.0 {
        (1.0 - 16.0 * zeta).powf(-0.5)
    } else if zeta <= 1.0 {
        1.0 + 5.0 * zeta
    } else {
        5.0 + zeta
    }
}

fn psi(kind: i32, zeta: f64) -> f64 {
    let chik = (1.0 - 16.0 * zeta).powf(0.25);
    if kind == 1 {
        2.0 * ((1.0 + chik) * 0.5).ln() + ((1.0 + chik * chik) * 0.5).ln() - 2.0 * chik.atan()
            + 2.0 * 1.0_f64.atan()
    } else {
        2.0 * ((1.0 + chik * chik) * 0.5).ln()
    }
}

fn validate_surface(input: MoninObukhovInput) -> Result<()> {
    ensure!(
        [
            input.wind_height_m,
            input.temperature_height_m,
            input.humidity_height_m,
            input.displacement_height_m,
            input.momentum_roughness_m,
            input.heat_roughness_m,
            input.moisture_roughness_m,
            input.obukhov_length_m,
            input.stability_adjusted_wind_m_s,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.wind_height_m > input.displacement_height_m
            && input.temperature_height_m > input.displacement_height_m
            && input.humidity_height_m > input.displacement_height_m
            && input.momentum_roughness_m > 0.0
            && input.heat_roughness_m > 0.0
            && input.moisture_roughness_m > 0.0
            && input.obukhov_length_m != 0.0
            && input.stability_adjusted_wind_m_s > 0.0,
        "Monin-Obukhov surface inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "monin_obukhov_tests.rs"]
mod monin_obukhov_tests;
