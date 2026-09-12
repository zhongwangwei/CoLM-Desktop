//! Canopy roughness and displacement from `MOD_CanopyLayerProfile:cal_z0_displa`.

use anyhow::{ensure, Result};

const fn f77(value: f32) -> f64 {
    value as f64
}

const VON_KARMAN: f64 = f77(0.4);
const LEAF_DRAG_COEFFICIENT: f64 = f77(0.2);
const DISPLACEMENT_COEFFICIENT: f64 = f77(7.5);
const PSI_H: f64 = f77(0.193);

/// Roughness and zero-plane displacement returned by CoLM's canopy geometry kernel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyRoughness {
    pub momentum_roughness_m: f64,
    pub displacement_height_m: f64,
}

/// Ports `MOD_CanopyLayerProfile:cal_z0_displa`.
pub fn canopy_roughness(
    leaf_area_index: f64,
    canopy_height_m: f64,
    canopy_cover_fraction: f64,
) -> Result<CanopyRoughness> {
    ensure!(
        [leaf_area_index, canopy_height_m, canopy_cover_fraction]
            .iter()
            .all(|value| value.is_finite())
            && leaf_area_index >= 0.0
            && canopy_height_m > 0.0
            && canopy_cover_fraction > 0.0
            && canopy_cover_fraction <= 1.0,
        "canopy roughness inputs are invalid"
    );
    let mut square_root_drag = -VON_KARMAN / ((f77(0.01) / canopy_height_m).ln() - PSI_H);
    square_root_drag = square_root_drag.max(f77(0.0031_f32.powf(0.5)));
    let initial_area_index = if square_root_drag <= f77(0.3) {
        ((square_root_drag.powi(2) - f77(0.003)) / f77(0.3))
            .min(canopy_cover_fraction * (1.0 - f77(-20.0).exp()))
    } else {
        // Preserve CoLM's fallback after its diagnostic print.
        f77(0.29)
    };
    let initial_lai = -(1.0 - initial_area_index / canopy_cover_fraction).ln() / 0.5;
    let initial_temp = (2.0 * DISPLACEMENT_COEFFICIENT * initial_area_index).sqrt();
    let delta = -canopy_height_m
        * (canopy_cover_fraction
            * f77(1.1)
            * (1.0 + (LEAF_DRAG_COEFFICIENT * initial_lai * canopy_cover_fraction).powf(0.25))
                .ln()
            + (1.0 - canopy_cover_fraction) * (1.0 - (1.0 - (-initial_temp).exp()) / initial_temp));
    let area_index = canopy_cover_fraction * (1.0 - (-0.5 * leaf_area_index).exp());
    square_root_drag = (f77(0.003) + f77(0.3) * area_index).sqrt().min(f77(0.3));
    let temp = (2.0 * DISPLACEMENT_COEFFICIENT * area_index).sqrt();
    let geometry = canopy_cover_fraction
        * f77(1.1)
        * (1.0 + (LEAF_DRAG_COEFFICIENT * leaf_area_index * canopy_cover_fraction).powf(0.25)).ln()
        + (1.0 - canopy_cover_fraction) * (1.0 - (1.0 - (-temp).exp()) / temp);
    let mut displacement_height_m = if leaf_area_index > initial_lai {
        delta + canopy_height_m * geometry
    } else {
        canopy_height_m * geometry
    }
    .max(0.0);
    let mut momentum_roughness_m =
        (canopy_height_m - displacement_height_m) * (-VON_KARMAN / square_root_drag + PSI_H).exp();
    if momentum_roughness_m < f77(0.01) {
        momentum_roughness_m = f77(0.01);
        displacement_height_m = 0.0;
    }
    Ok(CanopyRoughness {
        momentum_roughness_m,
        displacement_height_m,
    })
}

#[cfg(test)]
#[path = "canopy_roughness_tests.rs"]
mod canopy_roughness_tests;
