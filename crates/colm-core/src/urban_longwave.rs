//! Urban longwave transfer from `MOD_Urban_Longwave.F90`.
//!
//! The transfer matrix is intentionally separate from the thermal iteration:
//! geometry is built once per urban step, while canopy leaf temperature changes
//! repeatedly.  Both paths use the same small solver and shadow functions as the
//! Rust urban shortwave kernel.

use anyhow::{ensure, Context, Result};

use crate::{
    pc_radiation::canopy_transmittance,
    urban_radiation::{solve, tree_shadow, wall_shadow_diffuse, wall_shadow_direct},
};

const SURFACES: usize = 5;
const STEFAN_BOLTZMANN: f64 = 5.67e-8;

/// Optional tree canopy carried by the urban longwave transfer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanLongwaveVegetation {
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub cover_fraction: f64,
    /// `(htop + hbot) / 2`, constrained to the building height.
    pub center_height_m: f64,
}

/// Inputs to one `UrbanOnlyLongwave` or `UrbanVegLongwave` transfer build.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanLongwaveInput {
    pub zenith_angle_radians: f64,
    pub building_height_to_length: f64,
    pub roof_fraction: f64,
    pub pervious_ground_fraction: f64,
    pub roof_height_m: f64,
    pub downward_longwave_w_m2: f64,
    pub sunlit_wall_temperature_k: f64,
    pub shaded_wall_temperature_k: f64,
    pub impervious_temperature_k: f64,
    pub pervious_temperature_k: f64,
    pub wall_emissivity: f64,
    pub impervious_emissivity: f64,
    pub pervious_emissivity: f64,
    pub vegetation: Option<UrbanLongwaveVegetation>,
}

/// Geometry-dependent longwave system retained across a leaf-temperature iteration.
///
/// All five-element buffers use `[sunlit wall, shaded wall, impervious ground,
/// pervious ground, tree]`.  Without trees, only the first four entries are live.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanLongwaveTransfer {
    pub surface_count: usize,
    pub inverse: [[f64; SURFACES]; SURFACES],
    /// CoLM's `B`.  The tree entry is its coefficient and needs `T_leaf^4`.
    pub source: [f64; SURFACES],
    /// CoLM's `B1`.  The tree entry is its coefficient and needs `T_leaf^4`.
    pub emitted: [f64; SURFACES],
    /// CoLM's `dBdT`.  The tree entry is its coefficient and needs `T_leaf^3`.
    pub temperature_derivative: [f64; SURFACES],
    pub sky_view_factor: [f64; SURFACES],
    pub vegetation_view_factor: [f64; SURFACES],
    /// `[roof, sunlit wall, shaded wall, impervious ground, pervious ground, tree]`.
    pub cover_fraction: [f64; SURFACES + 1],
    pub surface_emissivity: [f64; SURFACES],
    pub vegetation_emissivity: f64,
    pub ground_fraction: f64,
    pub downward_longwave_w_m2: f64,
}

/// Longwave fluxes at one leaf temperature after the matrix transfer is solved.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanLongwaveFluxes {
    /// Per-unit-surface absorption, in the transfer surface order.
    pub absorbed_w_m2: [f64; SURFACES],
    pub outgoing_longwave_w_m2: f64,
    pub transfer_radiance_w_m2: [f64; SURFACES],
    /// `absorption + outgoing - incoming`, before any prior-step thermal correction.
    pub energy_balance_error_w_m2: f64,
}

/// Builds the shared urban longwave matrix.
pub fn urban_longwave_transfer(input: UrbanLongwaveInput) -> Result<UrbanLongwaveTransfer> {
    validate(input)?;
    match input.vegetation {
        Some(vegetation) => vegetated_transfer(input, vegetation),
        None => bare_transfer(input),
    }
}

/// Solves a pre-built longwave transfer at the current optional leaf temperature.
pub fn urban_longwave_fluxes(
    transfer: &UrbanLongwaveTransfer,
    leaf_temperature_k: Option<f64>,
) -> Result<UrbanLongwaveFluxes> {
    ensure!(
        (transfer.surface_count == 4 || transfer.surface_count == SURFACES)
            && transfer.ground_fraction > 0.0,
        "urban longwave transfer has an invalid surface layout"
    );
    let mut source = transfer.source;
    let mut emitted = transfer.emitted;
    if transfer.surface_count == SURFACES {
        let temperature_k =
            leaf_temperature_k.context("vegetated urban longwave needs leaf temperature")?;
        ensure!(
            temperature_k.is_finite() && temperature_k > 0.0,
            "urban leaf temperature must be finite and positive"
        );
        let temperature4 = temperature_k.powi(4);
        source[4] *= temperature4;
        emitted[4] *= temperature4;
    } else {
        ensure!(
            leaf_temperature_k.is_none(),
            "bare urban longwave must not receive a leaf temperature"
        );
    }
    let radiance = matrix_vector(transfer.inverse, source);
    let mut absorption = [0.0; SURFACES];
    for surface in 0..4 {
        absorption[surface] = (transfer.surface_emissivity[surface] * radiance[surface]
            - emitted[surface])
            / (1.0 - transfer.surface_emissivity[surface]);
    }
    if transfer.surface_count == SURFACES {
        absorption[4] = ((radiance[..4]
            .iter()
            .zip(&transfer.vegetation_view_factor[..4])
            .map(|(radiance, factor)| radiance * factor)
            .sum::<f64>()
            + transfer.downward_longwave_w_m2 * transfer.vegetation_view_factor[4])
            * transfer.vegetation_emissivity)
            - emitted[4];
    }
    let outgoing_longwave_w_m2 = radiance
        .iter()
        .zip(&transfer.sky_view_factor)
        .map(|(radiance, factor)| radiance * factor)
        .sum();
    let raw_absorption = absorption;
    for (surface, value) in absorption
        .iter_mut()
        .enumerate()
        .take(transfer.surface_count)
    {
        let cover = transfer.cover_fraction[surface + 1];
        if cover > 0.0 {
            *value = *value / cover * transfer.ground_fraction;
        }
    }
    Ok(UrbanLongwaveFluxes {
        absorbed_w_m2: absorption,
        outgoing_longwave_w_m2,
        transfer_radiance_w_m2: radiance,
        energy_balance_error_w_m2: raw_absorption[..transfer.surface_count].iter().sum::<f64>()
            + outgoing_longwave_w_m2
            - transfer.downward_longwave_w_m2,
    })
}

fn bare_transfer(input: UrbanLongwaveInput) -> Result<UrbanLongwaveTransfer> {
    let base = Base::new(input);
    let shadow = wall_shadow_direct(
        base.roof_fraction / base.ground_fraction,
        input.building_height_to_length,
        input.zenith_angle_radians,
    );
    let sunlit = 0.5 * (shadow * base.ground_fraction + base.roof_fraction)
        / (4.0 / std::f64::consts::PI
            * base.roof_fraction
            * input.building_height_to_length
            * input.zenith_angle_radians.tan()
            + base.roof_fraction);
    let shaded = 1.0 - sunlit;
    let mut matrix = identity();
    let wall_reflection = 1.0 - input.wall_emissivity;
    let imp_reflection = 1.0 - input.impervious_emissivity;
    let per_reflection = 1.0 - input.pervious_emissivity;
    matrix[0] = [
        1.0 - base.wall_to_wall * sunlit * wall_reflection,
        -base.wall_to_wall * sunlit * wall_reflection,
        -base.ground_to_wall * sunlit * wall_reflection,
        -base.ground_to_wall * sunlit * wall_reflection,
        0.0,
    ];
    matrix[1] = [
        -base.wall_to_wall * shaded * wall_reflection,
        1.0 - base.wall_to_wall * shaded * wall_reflection,
        -base.ground_to_wall * shaded * wall_reflection,
        -base.ground_to_wall * shaded * wall_reflection,
        0.0,
    ];
    matrix[2] = [
        -base.wall_to_ground * base.impervious_fraction * imp_reflection,
        -base.wall_to_ground * base.impervious_fraction * imp_reflection,
        1.0,
        0.0,
        0.0,
    ];
    matrix[3] = [
        -base.wall_to_ground * input.pervious_ground_fraction * per_reflection,
        -base.wall_to_ground * input.pervious_ground_fraction * per_reflection,
        0.0,
        1.0,
        0.0,
    ];
    build_transfer(
        4,
        matrix,
        input,
        [sunlit, shaded],
        [base.diffuse_wall_shadow, base.sky_to_ground],
        [base.wall_to_sky, base.ground_to_sky],
        [0.0; SURFACES],
        0.0,
        0.0,
        0.0,
        0.0,
    )
}

fn vegetated_transfer(
    input: UrbanLongwaveInput,
    vegetation: UrbanLongwaveVegetation,
) -> Result<UrbanLongwaveTransfer> {
    let base = Base::new(input);
    let length = input.roof_height_m / input.building_height_to_length;
    let leaf_stem_area = vegetation.leaf_area_index + vegetation.stem_area_index;
    let optical_depth = leaf_stem_area * vegetation.cover_fraction
        / (std::f64::consts::PI / 3.0).cos()
        / tree_shadow(vegetation.cover_fraction, std::f64::consts::PI / 3.0);
    let transmission = canopy_transmittance(3.0 / 8.0 * optical_depth);
    let vegetation_emissivity = 1.0 - transmission;

    let upper_shadow = wall_shadow_diffuse(
        base.roof_fraction / base.ground_fraction,
        (input.roof_height_m - vegetation.center_height_m) / length,
    );
    let sky = tree_view(
        base.diffuse_wall_shadow,
        upper_shadow,
        vegetation.cover_fraction,
        base.ground_fraction,
        std::f64::consts::PI / 3.0,
    );
    let tree_to_sky = 0.5 * (1.0 - upper_shadow);
    let lower_shadow = wall_shadow_diffuse(
        base.roof_fraction / base.ground_fraction,
        vegetation.center_height_m / length,
    );
    let ground = tree_view(
        base.diffuse_wall_shadow,
        lower_shadow,
        vegetation.cover_fraction,
        base.ground_fraction,
        std::f64::consts::PI / 3.0,
    );
    let tree_to_ground = 0.5 * (1.0 - lower_shadow);
    let tree_to_wall = 1.0 - tree_to_sky - tree_to_ground;

    let mut wall_to_tree = vegetation
        .cover_fraction
        .max(0.5 * (sky.cover + ground.cover))
        * 2.0
        * base.ground_fraction
        * tree_to_wall
        / (4.0 * input.building_height_to_length * base.roof_fraction);
    wall_to_tree = wall_to_tree.min(0.8);
    let upper_height_fraction = vegetation.center_height_m / input.roof_height_m;
    let lower_height_fraction =
        (input.roof_height_m - vegetation.center_height_m) / input.roof_height_m;
    let mut wall_tree_wall = wall_to_tree
        / (1.0
            + base.wall_to_sky * upper_height_fraction / base.wall_to_wall
            + base.wall_to_ground * lower_height_fraction / base.wall_to_wall);
    let mut wall_tree_sky =
        base.wall_to_sky * upper_height_fraction / base.wall_to_wall * wall_tree_wall;
    let mut wall_tree_ground =
        base.wall_to_ground * lower_height_fraction / base.wall_to_wall * wall_tree_wall;
    wall_tree_wall = wall_tree_wall.min(base.wall_to_wall);
    wall_tree_sky = wall_tree_sky.min(base.wall_to_sky);
    wall_tree_ground = wall_tree_ground.min(base.wall_to_ground);
    wall_to_tree = wall_tree_wall + wall_tree_sky + wall_tree_ground;

    let sky_to_wall = base.diffuse_wall_shadow - sky.wall + sky.wall * transmission;
    let sky_to_ground = base.sky_to_ground - sky.ground + sky.ground * transmission;
    let ground_to_wall = base.ground_to_wall - ground.wall + ground.wall * transmission;
    let ground_to_sky = base.ground_to_sky - ground.sky + ground.sky * transmission;
    let wall_to_ground = base.wall_to_ground - wall_tree_ground + wall_tree_ground * transmission;
    let wall_to_wall = base.wall_to_wall - wall_tree_wall + wall_tree_wall * transmission;
    let wall_to_sky = base.wall_to_sky - wall_tree_sky + wall_tree_sky * transmission;

    let direct_shadow = wall_shadow_direct(
        base.roof_fraction / base.ground_fraction,
        input.building_height_to_length,
        input.zenith_angle_radians,
    );
    let lower_direct_shadow = wall_shadow_direct(
        base.roof_fraction / base.ground_fraction,
        (input.roof_height_m - vegetation.center_height_m) / length,
        input.zenith_angle_radians,
    );
    let direct_tree = tree_view(
        direct_shadow,
        lower_direct_shadow,
        vegetation.cover_fraction,
        base.ground_fraction,
        input.zenith_angle_radians,
    );
    let direct_wall_shadow = direct_shadow - direct_tree.wall;
    let sunlit = 0.5 * (direct_wall_shadow * base.ground_fraction + base.roof_fraction)
        / (4.0 / std::f64::consts::PI
            * base.roof_fraction
            * input.building_height_to_length
            * input.zenith_angle_radians.tan()
            + base.roof_fraction);
    let shaded = 1.0 - sunlit;

    let wall_reflection = 1.0 - input.wall_emissivity;
    let imp_reflection = 1.0 - input.impervious_emissivity;
    let per_reflection = 1.0 - input.pervious_emissivity;
    let mut matrix = identity();
    matrix[0] = [
        1.0 - wall_to_wall * sunlit * wall_reflection,
        -wall_to_wall * sunlit * wall_reflection,
        -ground_to_wall * sunlit * wall_reflection,
        -ground_to_wall * sunlit * wall_reflection,
        -tree_to_wall * sunlit * wall_reflection,
    ];
    matrix[1] = [
        -wall_to_wall * shaded * wall_reflection,
        1.0 - wall_to_wall * shaded * wall_reflection,
        -ground_to_wall * shaded * wall_reflection,
        -ground_to_wall * shaded * wall_reflection,
        -tree_to_wall * shaded * wall_reflection,
    ];
    matrix[2] = [
        -wall_to_ground * base.impervious_fraction * imp_reflection,
        -wall_to_ground * base.impervious_fraction * imp_reflection,
        1.0,
        0.0,
        -tree_to_ground * base.impervious_fraction * imp_reflection,
    ];
    matrix[3] = [
        -wall_to_ground * input.pervious_ground_fraction * per_reflection,
        -wall_to_ground * input.pervious_ground_fraction * per_reflection,
        0.0,
        1.0,
        -tree_to_ground * input.pervious_ground_fraction * per_reflection,
    ];
    let tree_coefficient = (2.0 * vegetation.cover_fraction / base.ground_fraction)
        .max(sky.cover + ground.cover)
        * STEFAN_BOLTZMANN
        * vegetation_emissivity;
    build_transfer(
        SURFACES,
        matrix,
        input,
        [sunlit, shaded],
        [sky_to_wall, sky_to_ground],
        [wall_to_sky, ground_to_sky],
        [
            wall_to_tree,
            wall_to_tree,
            ground.cover,
            ground.cover,
            sky.cover,
        ],
        tree_to_sky,
        vegetation_emissivity,
        tree_coefficient,
        vegetation.cover_fraction,
    )
}

#[derive(Debug, Clone, Copy)]
struct Base {
    roof_fraction: f64,
    ground_fraction: f64,
    impervious_fraction: f64,
    diffuse_wall_shadow: f64,
    sky_to_ground: f64,
    ground_to_wall: f64,
    ground_to_sky: f64,
    wall_to_wall: f64,
    wall_to_ground: f64,
    wall_to_sky: f64,
}

impl Base {
    fn new(input: UrbanLongwaveInput) -> Self {
        let ground_fraction = 1.0 - input.roof_fraction;
        let diffuse_wall_shadow = wall_shadow_diffuse(
            input.roof_fraction / ground_fraction,
            input.building_height_to_length,
        );
        let sky_to_ground = 1.0 - diffuse_wall_shadow;
        let wall_to_sky = diffuse_wall_shadow * ground_fraction
            / input.roof_fraction
            / (4.0 * input.building_height_to_length);
        let wall_to_ground = wall_to_sky;
        Self {
            roof_fraction: input.roof_fraction,
            ground_fraction,
            impervious_fraction: 1.0 - input.pervious_ground_fraction,
            diffuse_wall_shadow,
            sky_to_ground,
            ground_to_wall: diffuse_wall_shadow,
            ground_to_sky: sky_to_ground,
            wall_to_wall: 1.0 - wall_to_sky - wall_to_ground,
            wall_to_ground,
            wall_to_sky,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TreeView {
    cover: f64,
    wall: f64,
    ground: f64,
    sky: f64,
}

fn tree_view(
    full_shadow: f64,
    partial_shadow: f64,
    cover_fraction: f64,
    ground_fraction: f64,
    zenith_angle_radians: f64,
) -> TreeView {
    let tree_fraction = cover_fraction - cover_fraction * partial_shadow;
    let shadow = tree_shadow(tree_fraction, zenith_angle_radians);
    let cover = (shadow / ground_fraction).min(1.0);
    let mut wall = (full_shadow - partial_shadow) * shadow;
    if full_shadow + cover - wall > 1.0 {
        wall = full_shadow + cover - 1.0;
    }
    let ground = cover - wall;
    // Keep the original derived labels together: for sky geometry `cover` is
    // Fsv; for ground geometry it is Fgv.  The caller chooses the counterpart.
    let sky = ground;
    TreeView {
        cover,
        wall,
        ground,
        sky,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_transfer(
    surface_count: usize,
    matrix: [[f64; SURFACES]; SURFACES],
    input: UrbanLongwaveInput,
    wall_fraction: [f64; 2],
    incident_view_factor: [f64; 2],
    sky: [f64; 2],
    vegetation_view_factor: [f64; SURFACES],
    vegetation_sky_view_factor: f64,
    vegetation_emissivity: f64,
    tree_coefficient: f64,
    vegetation_cover: f64,
) -> Result<UrbanLongwaveTransfer> {
    let base = Base::new(input);
    let impervious_ground = base.impervious_fraction;
    let pervious_ground = input.pervious_ground_fraction;
    let incident_wall = input.downward_longwave_w_m2 * incident_view_factor[0];
    let incident_ground = input.downward_longwave_w_m2 * incident_view_factor[1];
    let wall_scale =
        4.0 * input.building_height_to_length * input.roof_fraction / base.ground_fraction;
    let source = [
        incident_wall * wall_fraction[0] * (1.0 - input.wall_emissivity)
            + wall_scale
                * wall_fraction[0]
                * STEFAN_BOLTZMANN
                * input.wall_emissivity
                * input.sunlit_wall_temperature_k.powi(4),
        incident_wall * wall_fraction[1] * (1.0 - input.wall_emissivity)
            + wall_scale
                * wall_fraction[1]
                * STEFAN_BOLTZMANN
                * input.wall_emissivity
                * input.shaded_wall_temperature_k.powi(4),
        incident_ground * impervious_ground * (1.0 - input.impervious_emissivity)
            + impervious_ground
                * STEFAN_BOLTZMANN
                * input.impervious_emissivity
                * input.impervious_temperature_k.powi(4),
        incident_ground * pervious_ground * (1.0 - input.pervious_emissivity)
            + pervious_ground
                * STEFAN_BOLTZMANN
                * input.pervious_emissivity
                * input.pervious_temperature_k.powi(4),
        tree_coefficient,
    ];
    let emitted = [
        wall_scale
            * wall_fraction[0]
            * STEFAN_BOLTZMANN
            * input.wall_emissivity
            * input.sunlit_wall_temperature_k.powi(4),
        wall_scale
            * wall_fraction[1]
            * STEFAN_BOLTZMANN
            * input.wall_emissivity
            * input.shaded_wall_temperature_k.powi(4),
        impervious_ground
            * STEFAN_BOLTZMANN
            * input.impervious_emissivity
            * input.impervious_temperature_k.powi(4),
        pervious_ground
            * STEFAN_BOLTZMANN
            * input.pervious_emissivity
            * input.pervious_temperature_k.powi(4),
        tree_coefficient,
    ];
    let temperature_derivative = [
        4.0 * wall_scale
            * wall_fraction[0]
            * STEFAN_BOLTZMANN
            * input.wall_emissivity
            * input.sunlit_wall_temperature_k.powi(3),
        4.0 * wall_scale
            * wall_fraction[1]
            * STEFAN_BOLTZMANN
            * input.wall_emissivity
            * input.shaded_wall_temperature_k.powi(3),
        4.0 * impervious_ground
            * STEFAN_BOLTZMANN
            * input.impervious_emissivity
            * input.impervious_temperature_k.powi(3),
        4.0 * pervious_ground
            * STEFAN_BOLTZMANN
            * input.pervious_emissivity
            * input.pervious_temperature_k.powi(3),
        4.0 * tree_coefficient,
    ];
    Ok(UrbanLongwaveTransfer {
        surface_count,
        inverse: inverse(matrix)?,
        source,
        emitted,
        temperature_derivative,
        sky_view_factor: [
            sky[0],
            sky[0],
            sky[1],
            sky[1],
            if surface_count == SURFACES {
                vegetation_sky_view_factor
            } else {
                0.0
            },
        ],
        vegetation_view_factor,
        cover_fraction: [
            input.roof_fraction,
            wall_scale * wall_fraction[0] * base.ground_fraction,
            wall_scale * wall_fraction[1] * base.ground_fraction,
            base.ground_fraction * impervious_ground,
            base.ground_fraction * pervious_ground,
            vegetation_cover,
        ],
        surface_emissivity: [
            input.wall_emissivity,
            input.wall_emissivity,
            input.impervious_emissivity,
            input.pervious_emissivity,
            vegetation_emissivity,
        ],
        vegetation_emissivity,
        ground_fraction: base.ground_fraction,
        downward_longwave_w_m2: input.downward_longwave_w_m2,
    })
}

fn identity() -> [[f64; SURFACES]; SURFACES] {
    std::array::from_fn(|row| std::array::from_fn(|column| f64::from(row == column)))
}

fn inverse(matrix: [[f64; SURFACES]; SURFACES]) -> Result<[[f64; SURFACES]; SURFACES]> {
    let mut inverse = [[0.0; SURFACES]; SURFACES];
    for column in 0..SURFACES {
        let mut rhs = [0.0; SURFACES];
        rhs[column] = 1.0;
        let solution = solve(matrix, rhs)?;
        for row in 0..SURFACES {
            inverse[row][column] = solution[row];
        }
    }
    Ok(inverse)
}

fn matrix_vector(matrix: [[f64; SURFACES]; SURFACES], vector: [f64; SURFACES]) -> [f64; SURFACES] {
    std::array::from_fn(|row| {
        matrix[row]
            .iter()
            .zip(vector)
            .map(|(left, right)| left * right)
            .sum()
    })
}

fn validate(input: UrbanLongwaveInput) -> Result<()> {
    ensure!(
        input.zenith_angle_radians.is_finite()
            && input.zenith_angle_radians > 0.0
            && input.zenith_angle_radians < std::f64::consts::FRAC_PI_2
            && input.building_height_to_length.is_finite()
            && input.building_height_to_length > 0.0
            && input.roof_fraction.is_finite()
            && input.roof_fraction > 0.0
            && input.roof_fraction < 1.0
            && input.pervious_ground_fraction.is_finite()
            && (0.0..=1.0).contains(&input.pervious_ground_fraction)
            && input.roof_height_m.is_finite()
            && input.roof_height_m > 0.0
            && input.downward_longwave_w_m2.is_finite()
            && input.downward_longwave_w_m2 >= 0.0,
        "urban longwave geometry or forcing is invalid"
    );
    for value in [
        input.sunlit_wall_temperature_k,
        input.shaded_wall_temperature_k,
        input.impervious_temperature_k,
        input.pervious_temperature_k,
    ] {
        ensure!(
            value.is_finite() && value > 0.0,
            "urban surface temperature must be finite and positive"
        );
    }
    for value in [
        input.wall_emissivity,
        input.impervious_emissivity,
        input.pervious_emissivity,
    ] {
        ensure!(
            value.is_finite() && (0.0..1.0).contains(&value),
            "urban longwave emissivity must be finite in [0, 1)"
        );
    }
    if let Some(vegetation) = input.vegetation {
        ensure!(
            vegetation.leaf_area_index.is_finite()
                && vegetation.leaf_area_index >= 0.0
                && vegetation.stem_area_index.is_finite()
                && vegetation.stem_area_index >= 0.0
                && vegetation.leaf_area_index + vegetation.stem_area_index > 0.0
                && vegetation.cover_fraction.is_finite()
                && vegetation.cover_fraction > 0.0
                && vegetation.cover_fraction <= 1.0 - input.roof_fraction
                && vegetation.center_height_m.is_finite()
                && vegetation.center_height_m > 0.0
                && vegetation.center_height_m < input.roof_height_m,
            "urban vegetation longwave inputs are invalid"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "urban_longwave_tests.rs"]
mod urban_longwave_tests;
