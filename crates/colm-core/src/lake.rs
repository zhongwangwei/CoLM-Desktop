//! Runtime lake-column adjustment from `MOD_Lake.F90`.

use anyhow::{ensure, Result};

const LAKE_LAYERS: usize = 10;
const DEFAULT_THICKNESS_M: [f64; LAKE_LAYERS] =
    [0.1, 1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 7.0, 10.45, 10.45];
const FREEZING_K: f64 = 273.16;
const LIQUID_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const FUSION_HEAT_J_KG: f64 = 0.3336e6;

/// Mutable ten-layer lake state in top-to-bottom order.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeColumn {
    pub thickness_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    /// Frozen mass fraction of each lake layer.
    pub ice_fraction: Vec<f64>,
}

/// Remaps a lake column to CoLM's depth-scaled standard ten-layer geometry.
///
/// The overlap remap and mixed liquid/ice energy reconciliation preserve
/// `MOD_Lake:adjust_lake_layer`. A zero-depth column is intentionally untouched.
pub fn adjust_lake_layers(column: &mut LakeColumn) -> Result<()> {
    validate(column)?;
    let total_depth_m: f64 = column.thickness_m.iter().sum();
    ensure!(total_depth_m.is_finite(), "lake depth must be finite");
    if total_depth_m == 0.0 {
        return Ok(());
    }

    let target_thickness_m = target_thickness(total_depth_m);
    let mut target_temperature_k = vec![0.0; LAKE_LAYERS];
    let mut target_ice_fraction = vec![0.0; LAKE_LAYERS];
    let mut source_layer = 0;
    let mut source_remaining_m = column.thickness_m[source_layer];

    for target_layer in 0..LAKE_LAYERS {
        let mut target_remaining_m = target_thickness_m[target_layer];
        let mut ice_temperature_sum = 0.0;
        let mut liquid_temperature_sum = 0.0;
        let mut ice_mass = 0.0;
        let mut liquid_mass = 0.0;
        while target_remaining_m > 0.0 {
            if source_remaining_m == 0.0 {
                ensure!(
                    source_layer + 1 < LAKE_LAYERS,
                    "lake remap exhausted source thickness before target layer"
                );
                source_layer += 1;
                source_remaining_m = column.thickness_m[source_layer];
                continue;
            }
            let overlap_m = target_remaining_m.min(source_remaining_m);
            let source_ice = column.ice_fraction[source_layer];
            let source_temperature = column.temperature_k[source_layer];
            ice_temperature_sum += overlap_m * source_ice * source_temperature;
            ice_mass += overlap_m * source_ice;
            liquid_temperature_sum += overlap_m * (1.0 - source_ice) * source_temperature;
            liquid_mass += overlap_m * (1.0 - source_ice);
            target_remaining_m -= overlap_m;
            source_remaining_m -= overlap_m;
        }

        let (temperature_k, ice_mass) = reconcile_phase(
            target_thickness_m[target_layer],
            ice_temperature_sum,
            liquid_temperature_sum,
            ice_mass,
            liquid_mass,
        );
        target_temperature_k[target_layer] = temperature_k;
        target_ice_fraction[target_layer] = ice_mass / target_thickness_m[target_layer];
    }
    column.thickness_m = target_thickness_m;
    column.temperature_k = target_temperature_k;
    column.ice_fraction = target_ice_fraction;
    Ok(())
}

fn validate(column: &LakeColumn) -> Result<()> {
    ensure!(
        column.thickness_m.len() == LAKE_LAYERS
            && column.temperature_k.len() == LAKE_LAYERS
            && column.ice_fraction.len() == LAKE_LAYERS,
        "CoLM lake adjustment requires ten matching layers"
    );
    ensure!(
        column
            .thickness_m
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && column.temperature_k.iter().all(|value| value.is_finite())
            && column
                .ice_fraction
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value)),
        "lake column is physically invalid"
    );
    Ok(())
}

fn target_thickness(total_depth_m: f64) -> Vec<f64> {
    if total_depth_m <= 1.0 {
        return vec![total_depth_m / LAKE_LAYERS as f64; LAKE_LAYERS];
    }
    let depth_ratio = total_depth_m / DEFAULT_THICKNESS_M.iter().sum::<f64>();
    let mut thickness = DEFAULT_THICKNESS_M
        .iter()
        .map(|value| value * depth_ratio)
        .collect::<Vec<_>>();
    thickness[0] = DEFAULT_THICKNESS_M[0];
    thickness[LAKE_LAYERS - 1] -= thickness[0] - DEFAULT_THICKNESS_M[0] * depth_ratio;
    thickness
}

fn reconcile_phase(
    layer_thickness_m: f64,
    ice_temperature_sum: f64,
    liquid_temperature_sum: f64,
    mut ice_mass: f64,
    mut liquid_mass: f64,
) -> (f64, f64) {
    if ice_mass == 0.0 {
        return (liquid_temperature_sum / liquid_mass, ice_mass);
    }
    if liquid_mass == 0.0 {
        return (ice_temperature_sum / ice_mass, ice_mass);
    }
    let ice_temperature_k = ice_temperature_sum / ice_mass;
    let liquid_temperature_k = liquid_temperature_sum / liquid_mass;
    let liquid_heat =
        LIQUID_HEAT_CAPACITY_J_KG_K * liquid_mass * (liquid_temperature_k - FREEZING_K);
    let ice_heat = ICE_HEAT_CAPACITY_J_KG_K * ice_mass * (FREEZING_K - ice_temperature_k);
    let ice_fusion = ice_mass * FUSION_HEAT_J_KG;
    let liquid_fusion = liquid_mass * FUSION_HEAT_J_KG;
    if liquid_heat >= ice_heat + ice_fusion {
        ice_mass = 0.0;
        liquid_mass = layer_thickness_m;
        (
            FREEZING_K
                + (liquid_heat - ice_heat - ice_fusion)
                    / (liquid_mass * LIQUID_HEAT_CAPACITY_J_KG_K),
            ice_mass,
        )
    } else if liquid_heat >= ice_heat {
        ice_mass -= (liquid_heat - ice_heat) / FUSION_HEAT_J_KG;
        (FREEZING_K, ice_mass)
    } else if liquid_heat + liquid_fusion < ice_heat {
        ice_mass = layer_thickness_m;
        (
            FREEZING_K
                - (ice_heat - liquid_heat - liquid_fusion) / (ice_mass * ICE_HEAT_CAPACITY_J_KG_K),
            ice_mass,
        )
    } else {
        ice_mass += (ice_heat - liquid_heat) / FUSION_HEAT_J_KG;
        (FREEZING_K, ice_mass)
    }
}

#[cfg(test)]
#[path = "lake_tests.rs"]
mod lake_tests;
