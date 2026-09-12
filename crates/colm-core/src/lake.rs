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

/// Inputs to `MOD_Lake:roughness_lake` for one lake surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeRoughnessInput {
    /// CoLM's signed snow-layer count (`0`, `-1`, …).
    pub snow_layer_count: i32,
    pub ground_temperature_k: f64,
    pub lake_surface_temperature_k: f64,
    pub surface_pressure_pa: f64,
    pub charnock_parameter: f64,
    pub friction_velocity_m_s: f64,
}

/// Lake momentum, sensible-heat, and latent-heat roughness lengths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeRoughness {
    pub momentum_m: f64,
    pub sensible_heat_m: f64,
    pub latent_heat_m: f64,
}

/// Inputs to `MOD_Lake:hConductivity_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeConductivityInput<'a> {
    pub snow_layer_count: i32,
    pub ground_temperature_k: f64,
    /// Lake-node depths (m), top to bottom.
    pub node_depth_m: &'a [f64],
    pub latitude_radians: f64,
    pub friction_velocity_m_s: f64,
    pub momentum_roughness_m: f64,
    /// `lakedepth`, retained separately from the mutable layer geometry.
    pub lake_depth_m: f64,
    pub deep_lake_threshold_m: f64,
}

/// Effective thermal conductivity of every lake layer and top eddy term.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeConductivity {
    pub thermal_conductivity_w_m_k: Vec<f64>,
    pub top_eddy_conductivity_w_m_k: f64,
}

/// Ports `MOD_Lake:roughness_lake`.
pub fn lake_roughness(input: LakeRoughnessInput) -> Result<LakeRoughness> {
    ensure!(
        input.snow_layer_count <= 0
            && input.ground_temperature_k.is_finite()
            && input.lake_surface_temperature_k.is_finite()
            && input.surface_pressure_pa.is_finite()
            && input.surface_pressure_pa > 0.0
            && input.charnock_parameter.is_finite()
            && input.charnock_parameter >= 0.0
            && input.friction_velocity_m_s.is_finite()
            && input.friction_velocity_m_s >= 0.0,
        "lake roughness inputs are invalid"
    );
    if input.ground_temperature_k > FREEZING_K
        && input.lake_surface_temperature_k > FREEZING_K
        && input.snow_layer_count == 0
    {
        let viscosity = 1.51e-5 * (input.ground_temperature_k / 293.15).powf(1.5) * 1.013e5
            / input.surface_pressure_pa;
        let momentum_m = (0.1 * viscosity / input.friction_velocity_m_s.max(1.0e-4))
            .max(input.charnock_parameter * input.friction_velocity_m_s.powi(2) / 9.80616)
            .max(1.0e-5);
        let roughness_reynolds_sqrt = (momentum_m * input.friction_velocity_m_s / viscosity)
            .max(0.1)
            .sqrt();
        return Ok(LakeRoughness {
            momentum_m,
            sensible_heat_m: (momentum_m
                * (-0.4 / 0.713 * (4.0 * roughness_reynolds_sqrt - 3.2)).exp())
            .max(1.0e-5),
            latent_heat_m: (momentum_m
                * (-0.4 / 0.66 * (4.0 * roughness_reynolds_sqrt - 4.2)).exp())
            .max(1.0e-5),
        });
    }
    let momentum_m = if input.snow_layer_count == 0 {
        0.001
    } else {
        0.0024
    };
    let heat =
        momentum_m / (0.13 * (input.friction_velocity_m_s * momentum_m / 1.5e-5).powf(0.45)).exp();
    Ok(LakeRoughness {
        momentum_m,
        sensible_heat_m: heat,
        latent_heat_m: heat,
    })
}

/// Ports `MOD_Lake:hConductivity_lake`, including density-dependent mixing.
pub fn lake_thermal_conductivity(
    input: LakeConductivityInput<'_>,
    column: &LakeColumn,
) -> Result<LakeConductivity> {
    validate(column)?;
    ensure!(
        input.snow_layer_count <= 0
            && input.ground_temperature_k.is_finite()
            && input.node_depth_m.len() == LAKE_LAYERS
            && input.node_depth_m.iter().all(|value| value.is_finite())
            && input
                .node_depth_m
                .windows(2)
                .all(|depths| depths[1] > depths[0])
            && input.latitude_radians.is_finite()
            && input.latitude_radians.abs() <= std::f64::consts::FRAC_PI_2
            && input.friction_velocity_m_s.is_finite()
            && input.friction_velocity_m_s >= 0.0
            && input.momentum_roughness_m.is_finite()
            && input.momentum_roughness_m > 0.0
            && input.lake_depth_m.is_finite()
            && input.lake_depth_m >= 0.0
            && input.deep_lake_threshold_m.is_finite()
            && input.deep_lake_threshold_m >= 0.0,
        "lake conductivity inputs are invalid"
    );
    let water_density = column
        .temperature_k
        .iter()
        .zip(&column.ice_fraction)
        .map(|(&temperature, &ice_fraction)| lake_water_density(temperature, ice_fraction))
        .collect::<Vec<_>>();
    let water_heat_capacity = 4188.0 * 1000.0;
    let effective_ice_conductivity = 2.29 * 917.0 / 1000.0;
    let molecular_diffusivity = 0.6 / water_heat_capacity;
    let wind_2m =
        (input.friction_velocity_m_s / 0.4 * (2.0 / input.momentum_roughness_m).ln()).max(0.1);
    let surface_water_velocity = 1.2e-3 * wind_2m;
    let decay = 6.6 * input.latitude_radians.sin().abs().sqrt() * wind_2m.powf(-1.84);
    let deep_lake = input.lake_depth_m >= input.deep_lake_threshold_m;
    let open_water = input.ground_temperature_k > FREEZING_K
        && column.temperature_k[0] > FREEZING_K
        && input.snow_layer_count == 0;
    let mut eddy_diffusivity = [0.0; LAKE_LAYERS];
    let mut thermal_conductivity_w_m_k = [0.0; LAKE_LAYERS];

    for layer in 0..LAKE_LAYERS - 1 {
        let density_gradient = (water_density[layer + 1] - water_density[layer])
            / (input.node_depth_m[layer + 1] - input.node_depth_m[layer]);
        let buoyancy_frequency = (9.80616 / water_density[layer] * density_gradient).max(7.5e-5);
        let numerator = 40.0 * buoyancy_frequency * (0.4 * input.node_depth_m[layer]).powi(2);
        let denominator = (surface_water_velocity.powi(2)
            * (-2.0 * decay * input.node_depth_m[layer]).max(-40.0).exp())
        .max(1.0e-10);
        let richardson = (-1.0 + (1.0 + numerator / denominator).max(0.0).sqrt()) / 20.0;
        let fang_stefan = 1.039e-8 * buoyancy_frequency.powf(-0.43);
        let mut diffusivity = molecular_diffusivity + fang_stefan;
        if open_water {
            let eddy = 0.4
                * surface_water_velocity
                * input.node_depth_m[layer]
                * (-decay * input.node_depth_m[layer]).max(-40.0).exp()
                / (1.0 + 37.0 * richardson.powi(2));
            diffusivity += eddy;
        }
        if deep_lake {
            diffusivity *= 5.0;
        }
        eddy_diffusivity[layer] = diffusivity;
        thermal_conductivity_w_m_k[layer] = if open_water {
            diffusivity * water_heat_capacity
        } else {
            frozen_thermal_conductivity(
                diffusivity,
                water_heat_capacity,
                effective_ice_conductivity,
                column.ice_fraction[layer],
            )
        };
    }
    eddy_diffusivity[LAKE_LAYERS - 1] = eddy_diffusivity[LAKE_LAYERS - 2];
    thermal_conductivity_w_m_k[LAKE_LAYERS - 1] = if open_water {
        thermal_conductivity_w_m_k[LAKE_LAYERS - 2]
    } else {
        frozen_thermal_conductivity(
            eddy_diffusivity[LAKE_LAYERS - 1],
            water_heat_capacity,
            effective_ice_conductivity,
            column.ice_fraction[LAKE_LAYERS - 1],
        )
    };
    Ok(LakeConductivity {
        thermal_conductivity_w_m_k: thermal_conductivity_w_m_k.to_vec(),
        top_eddy_conductivity_w_m_k: eddy_diffusivity[0] * water_heat_capacity,
    })
}

fn lake_water_density(temperature_k: f64, ice_fraction: f64) -> f64 {
    (1.0 - ice_fraction) * 1000.0 * (1.0 - 1.9549e-5 * (temperature_k - 277.0).abs().powf(1.68))
        + ice_fraction * 917.0
}

fn frozen_thermal_conductivity(
    diffusivity: f64,
    water_heat_capacity: f64,
    effective_ice_conductivity: f64,
    ice_fraction: f64,
) -> f64 {
    diffusivity * water_heat_capacity * effective_ice_conductivity
        / ((1.0 - ice_fraction) * effective_ice_conductivity
            + diffusivity * water_heat_capacity * ice_fraction)
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
