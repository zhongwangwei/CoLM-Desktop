//! Non-I/O urban initialization from `MOD_UrbanReadin.F90`.

use anyhow::{ensure, Result};

/// Runtime switches used by the geometry section of `Urban_readin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrbanConfig {
    pub water_enabled: bool,
    pub trees_enabled: bool,
    pub building_energy_model: bool,
}

/// Region-indexed LUCY inputs from `LUCY_rawdata.nc`.
///
/// Source fields are `region * components + component`, matching the Fortran
/// `l... (region, :)` reads.  Output fields are component-major because CoLM
/// stores `component, urban_patch` arrays.
#[derive(Debug, Clone, Copy)]
pub struct UrbanLucyInput<'a> {
    pub region_id: &'a [i32],
    pub population_density: &'a [f64],
    pub region_count: usize,
    pub vehicles_per_thousand: &'a [f64],
    pub week_holiday: &'a [f64],
    pub weekend_traffic_profile: &'a [f64],
    pub weekday_traffic_profile: &'a [f64],
    pub human_metabolic_profile: &'a [f64],
    pub fixed_holiday: &'a [f64],
}

/// Per-urban-patch LUCY state ready for the time-invariant restart writer.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanLucyState {
    pub population_density: Vec<f64>,
    pub vehicles_per_thousand: Vec<f64>,
    pub week_holiday: Vec<f64>,
    pub weekend_traffic_profile: Vec<f64>,
    pub weekday_traffic_profile: Vec<f64>,
    pub human_metabolic_profile: Vec<f64>,
    pub fixed_holiday: Vec<f64>,
}

/// Applies the LUCY section of `Urban_readin`.
///
/// `region_id` is one-based, as it is in the NetCDF landdata.  The Fortran
/// implementation leaves an enabled zero region uninitialized; Rust rejects
/// that malformed landdata instead of serializing an indeterminate restart.
pub fn derive_urban_lucy(input: UrbanLucyInput<'_>, enabled: bool) -> Result<UrbanLucyState> {
    let urban_count = input.region_id.len();
    ensure!(
        input.population_density.len() == urban_count,
        "LUCY population density must have one value per urban patch"
    );
    if !enabled {
        return Ok(empty_lucy_state(urban_count));
    }
    ensure!(
        input.region_count > 0,
        "enabled LUCY needs at least one region"
    );
    validate_lucy_source(input)?;
    for &region_id in input.region_id {
        ensure!(
            region_id > 0 && (region_id as usize) <= input.region_count,
            "enabled LUCY region id {region_id} is outside 1..={}",
            input.region_count
        );
    }
    Ok(UrbanLucyState {
        population_density: input.population_density.to_vec(),
        vehicles_per_thousand: copy_lucy_field(
            input.vehicles_per_thousand,
            input.region_id,
            input.region_count,
            3,
        ),
        week_holiday: copy_lucy_field(input.week_holiday, input.region_id, input.region_count, 7),
        weekend_traffic_profile: copy_lucy_field(
            input.weekend_traffic_profile,
            input.region_id,
            input.region_count,
            24,
        ),
        weekday_traffic_profile: copy_lucy_field(
            input.weekday_traffic_profile,
            input.region_id,
            input.region_count,
            24,
        ),
        human_metabolic_profile: copy_lucy_field(
            input.human_metabolic_profile,
            input.region_id,
            input.region_count,
            24,
        ),
        fixed_holiday: copy_lucy_field(
            input.fixed_holiday,
            input.region_id,
            input.region_count,
            365,
        ),
    })
}

fn empty_lucy_state(urban_count: usize) -> UrbanLucyState {
    UrbanLucyState {
        population_density: vec![0.0; urban_count],
        vehicles_per_thousand: vec![0.0; 3 * urban_count],
        week_holiday: vec![0.0; 7 * urban_count],
        weekend_traffic_profile: vec![0.0; 24 * urban_count],
        weekday_traffic_profile: vec![0.0; 24 * urban_count],
        human_metabolic_profile: vec![0.0; 24 * urban_count],
        fixed_holiday: vec![0.0; 365 * urban_count],
    }
}

fn validate_lucy_source(input: UrbanLucyInput<'_>) -> Result<()> {
    for (name, source, components) in [
        ("vehicles", input.vehicles_per_thousand, 3),
        ("week holidays", input.week_holiday, 7),
        ("weekend traffic profile", input.weekend_traffic_profile, 24),
        ("weekday traffic profile", input.weekday_traffic_profile, 24),
        ("human metabolic profile", input.human_metabolic_profile, 24),
        ("fixed holidays", input.fixed_holiday, 365),
    ] {
        ensure!(
            source.len() == input.region_count * components,
            "LUCY {name} has {} values; expected {} regions x {components}",
            source.len(),
            input.region_count
        );
    }
    Ok(())
}

fn copy_lucy_field(
    source: &[f64],
    region_id: &[i32],
    region_count: usize,
    components: usize,
) -> Vec<f64> {
    let urban_count = region_id.len();
    let mut output = vec![0.0; components * urban_count];
    for (urban, &id) in region_id.iter().enumerate() {
        let region = id as usize - 1;
        debug_assert!(region < region_count);
        for component in 0..components {
            output[component * urban_count + urban] = source[region * components + component];
        }
    }
    output
}

/// Layer-major urban landdata supplied by the future NetCDF adapter.
#[derive(Debug, Clone, Copy)]
pub struct UrbanInput<'a> {
    pub urban_to_patch: &'a [usize],
    pub roof_fraction: &'a [f64],
    pub roof_height_m: &'a [f64],
    pub building_height_to_width: &'a [f64],
    pub pervious_road_fraction: &'a [f64],
    /// Percent of the urban patch covered by water.
    pub water_percent: &'a [f64],
    /// Percent of the non-water urban patch covered by trees.
    pub tree_percent: &'a [f64],
    pub tree_top_m: &'a [f64],
    /// `impervious_layers * urban_count`, layer-major.
    pub impervious_heat_capacity: &'a [f64],
    pub impervious_layers: usize,
    pub roof_thickness_m: &'a [f64],
    pub wall_thickness_m: &'a [f64],
    pub room_max_k: &'a [f64],
    pub room_min_k: &'a [f64],
}

/// Normalized urban state, with each layer-major field ready for restart serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanState {
    pub roof_fraction: Vec<f64>,
    pub roof_height_m: Vec<f64>,
    pub building_height_to_width: Vec<f64>,
    pub pervious_road_fraction: Vec<f64>,
    pub water_fraction: Vec<f64>,
    pub tree_fraction: Vec<f64>,
    pub patch_tree_fraction: Vec<f64>,
    pub tree_top_m: Vec<f64>,
    pub tree_bottom_m: Vec<f64>,
    pub roof_node_depth_m: Vec<f64>,
    pub roof_layer_thickness_m: Vec<f64>,
    pub wall_node_depth_m: Vec<f64>,
    pub wall_layer_thickness_m: Vec<f64>,
    pub room_max_k: Vec<f64>,
    pub room_min_k: Vec<f64>,
}

/// Applies the geometry and fraction normalization section of `Urban_readin`.
///
/// `urban_class_top_m` and `urban_class_bottom_m` are `htop0(URBAN)` and `hbot0(URBAN)`.
/// The compiled Fortran implementation requires at least two roof and wall layers because
/// its end-layer stencil accesses the second node; this function rejects other dimensions.
pub fn derive_urban_geometry(
    input: UrbanInput<'_>,
    config: UrbanConfig,
    roof_layers: usize,
    wall_layers: usize,
    urban_class_top_m: f64,
    urban_class_bottom_m: f64,
    patch_count: usize,
) -> Result<UrbanState> {
    ensure!(
        roof_layers >= 2 && wall_layers >= 2,
        "urban roof and wall need at least two layers"
    );
    let urban_count = input.urban_to_patch.len();
    ensure!(
        input.roof_fraction.len() == urban_count
            && input.roof_height_m.len() == urban_count
            && input.building_height_to_width.len() == urban_count
            && input.pervious_road_fraction.len() == urban_count
            && input.water_percent.len() == urban_count
            && input.tree_percent.len() == urban_count
            && input.tree_top_m.len() == urban_count
            && input.roof_thickness_m.len() == urban_count
            && input.wall_thickness_m.len() == urban_count
            && input.room_max_k.len() == urban_count
            && input.room_min_k.len() == urban_count,
        "all urban scalar fields must have one value per urban patch"
    );
    ensure!(
        input.impervious_heat_capacity.len() == input.impervious_layers * urban_count,
        "impervious heat-capacity layout does not match its dimensions"
    );
    ensure!(
        input
            .urban_to_patch
            .iter()
            .all(|&patch| patch < patch_count),
        "urban-to-patch index exceeds the patch count"
    );

    let mut pervious_road_fraction = input.pervious_road_fraction.to_vec();
    let mut water_fraction = vec![0.0; urban_count];
    let mut tree_fraction = vec![0.0; urban_count];
    let mut patch_tree_fraction = vec![0.0; patch_count];
    let mut tree_top_m = Vec::with_capacity(urban_count);
    let mut tree_bottom_m = Vec::with_capacity(urban_count);
    let mut roof_node_depth_m = vec![0.0; roof_layers * urban_count];
    let mut roof_layer_thickness_m = vec![0.0; roof_layers * urban_count];
    let mut wall_node_depth_m = vec![0.0; wall_layers * urban_count];
    let mut wall_layer_thickness_m = vec![0.0; wall_layers * urban_count];
    let mut room_max_k = input.room_max_k.to_vec();
    let mut room_min_k = input.room_min_k.to_vec();

    for urban in 0..urban_count {
        if (0..input.impervious_layers)
            .all(|layer| input.impervious_heat_capacity[layer * urban_count + urban] == 0.0)
        {
            pervious_road_fraction[urban] = 1.0;
        }
        water_fraction[urban] = if config.water_enabled {
            input.water_percent[urban] / 100.0
        } else {
            0.0
        };
        tree_fraction[urban] = if config.trees_enabled {
            let mut fraction = input.tree_percent[urban] / 100.0;
            fraction = if water_fraction[urban] < 1.0 {
                fraction / (1.0 - water_fraction[urban])
            } else {
                0.0
            };
            fraction.min(1.0 - input.roof_fraction[urban])
        } else {
            0.0
        };
        patch_tree_fraction[input.urban_to_patch[urban]] = tree_fraction[urban];

        let top = input.tree_top_m[urban]
            .min(input.roof_height_m[urban])
            .max(2.0);
        let bottom = (top * urban_class_bottom_m / urban_class_top_m).max(1.0);
        tree_top_m.push(top);
        tree_bottom_m.push(bottom);

        fill_uniform_layers(
            input.roof_thickness_m[urban],
            roof_layers,
            urban,
            urban_count,
            &mut roof_node_depth_m,
            &mut roof_layer_thickness_m,
        );
        fill_uniform_layers(
            input.wall_thickness_m[urban],
            wall_layers,
            urban,
            urban_count,
            &mut wall_node_depth_m,
            &mut wall_layer_thickness_m,
        );
        if !config.building_energy_model {
            room_max_k[urban] = 373.16;
            room_min_k[urban] = 180.0;
        }
    }

    Ok(UrbanState {
        roof_fraction: input.roof_fraction.to_vec(),
        roof_height_m: input.roof_height_m.to_vec(),
        building_height_to_width: input.building_height_to_width.to_vec(),
        pervious_road_fraction,
        water_fraction,
        tree_fraction,
        patch_tree_fraction,
        tree_top_m,
        tree_bottom_m,
        roof_node_depth_m,
        roof_layer_thickness_m,
        wall_node_depth_m,
        wall_layer_thickness_m,
        room_max_k,
        room_min_k,
    })
}

fn fill_uniform_layers(
    total_thickness_m: f64,
    layers: usize,
    item: usize,
    item_count: usize,
    node_depth_m: &mut [f64],
    layer_thickness_m: &mut [f64],
) {
    for layer in 0..layers {
        node_depth_m[layer * item_count + item] =
            (layer as f64 + 0.5) * total_thickness_m / layers as f64;
    }
    layer_thickness_m[item] = 0.5 * (node_depth_m[item] + node_depth_m[item_count + item]);
    for layer in 1..layers - 1 {
        layer_thickness_m[layer * item_count + item] = 0.5
            * (node_depth_m[(layer + 1) * item_count + item]
                - node_depth_m[(layer - 1) * item_count + item]);
    }
    layer_thickness_m[(layers - 1) * item_count + item] = node_depth_m
        [(layers - 1) * item_count + item]
        - node_depth_m[(layers - 2) * item_count + item];
}

#[cfg(test)]
#[path = "urban_tests.rs"]
mod urban_tests;
