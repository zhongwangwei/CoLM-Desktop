//! Canopy roughness and displacement from `MOD_CanopyLayerProfile:cal_z0_displa`.

use anyhow::{ensure, Result};

use crate::f77;

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
    square_root_drag = square_root_drag.max(f77(0.0031_f64.powf(0.5)));
    let initial_area_index = if square_root_drag <= f77(0.3) {
        // `fai = (sqrtdragc**2-0.003)/0.3`：GIMPLE（`cal_z0_displa` 第 1 处）是
        // `FMA(sqrtdragc, sqrtdragc, -0.003)` —— 平方被吸收、常数先舍入。
        (square_root_drag.mul_add(square_root_drag, -f77(0.003)) / f77(0.3))
            .min(canopy_cover_fraction * (1.0 - f77(-20.0).exp()))
    } else {
        // Preserve CoLM's fallback after its diagnostic print.
        f77(0.29)
    };
    let initial_lai = -(1.0 - initial_area_index / canopy_cover_fraction).ln() / 0.5;
    let initial_temp = (2.0 * DISPLACEMENT_COEFFICIENT * initial_area_index).sqrt();
    // GIMPLE（第 4 处）：`FMA(fc*1.1, log(1+(Cd*lai0*fc)**0.25), (1-fc)*poly(初始 temp1))`
    // —— 冠层那一支被吸收、裸土那一支先舍入。`delta` 与它只差 `-h`，
    // 但 dump 里 GCC 并不单独舍入 `-h*geometry`，而是在下面用 `FNMA(h, ·, ·)`。
    let initial_geometry = (canopy_cover_fraction * f77(1.1)).mul_add(
        (1.0 + (LEAF_DRAG_COEFFICIENT * initial_lai * canopy_cover_fraction).powf(0.25)).ln(),
        (1.0 - canopy_cover_fraction) * (1.0 - (1.0 - (-initial_temp).exp()) / initial_temp),
    );
    let area_index = canopy_cover_fraction * (1.0 - (-0.5 * leaf_area_index).exp());
    // `sqrtdragc = min((0.003+0.3*fai)**0.5, 0.3)`：GIMPLE（第 2 处）是
    // `FMA(fai, 0.3, 0.003)` —— `0.3*fai` 被吸收。
    square_root_drag = (f77(0.3).mul_add(area_index, f77(0.003)))
        .sqrt()
        .min(f77(0.3));
    let temp = (2.0 * DISPLACEMENT_COEFFICIENT * area_index).sqrt();
    // 第 3 处：与 `initial_geometry` 同型，只是换成实测 `lai` 与新 `temp1`。
    let geometry = (canopy_cover_fraction * f77(1.1)).mul_add(
        (1.0 + (LEAF_DRAG_COEFFICIENT * leaf_area_index * canopy_cover_fraction).powf(0.25)).ln(),
        (1.0 - canopy_cover_fraction) * (1.0 - (1.0 - (-temp).exp()) / temp),
    );
    let scaled_geometry = canopy_height_m * geometry;
    let mut displacement_height_m = if leaf_area_index > initial_lai {
        // 第 5 处：`FNMA(h, initial_geometry, h*geometry)` = `delta + h*geometry`，
        // `h*initial_geometry` 被吸收（**不是**先算好 `delta` 再相加）。
        canopy_height_m.mul_add(-initial_geometry, scaled_geometry)
    } else {
        scaled_geometry
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
