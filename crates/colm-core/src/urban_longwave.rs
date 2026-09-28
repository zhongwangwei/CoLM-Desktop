//! Urban longwave transfer from `MOD_Urban_Longwave.F90`.
//!
//! The transfer matrix is intentionally separate from the thermal iteration:
//! geometry is built once per urban step, while canopy leaf temperature changes
//! repeatedly.  Both paths share the shadow functions with the urban shortwave
//! kernel; the inverse is `colm_lapack::matrix_inverse` (upstream `MatrixInverse`).

use anyhow::{ensure, Context, Result};

use crate::urban_radiation::{tree_shadow, wall_shadow_diffuse, wall_shadow_direct};

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
    let sunlit = sunlit_wall_fraction(input, &base, shadow);
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
    // `:396` `cos(PI/3)` 由 gfortran 在编译期折成 `0.5000000000000001`（MPFR 正确舍入）
    let optical_depth = leaf_stem_area * vegetation.cover_fraction
        / COS_PI_OVER_3
        / tree_shadow(vegetation.cover_fraction, std::f64::consts::PI / 3.0);
    // `:397` `tee(DD1*3/8.*lsai)`：`DD1` 是 real(r16)，`lsai*0.375` 与整个 `tee` 都在四精度里
    let transmission = crate::extended::tee(
        crate::extended::DoubleDouble::new(optical_depth)
            * crate::extended::DoubleDouble::new(0.375),
    );
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

    // `:500-506` `F_ = F - Fv + Fv*Td`：`.FMA (Fv, Td, F - Fv)`
    let through = |total: f64, blocked: f64| blocked.mul_add(transmission, total - blocked);
    let sky_to_wall = through(base.diffuse_wall_shadow, sky.wall);
    let sky_to_ground = through(base.sky_to_ground, sky.ground);
    let ground_to_wall = through(base.ground_to_wall, ground.wall);
    let ground_to_sky = through(base.ground_to_sky, ground.sky);
    let wall_to_ground = through(base.wall_to_ground, wall_tree_ground);
    let wall_to_wall = through(base.wall_to_wall, wall_tree_wall);
    let wall_to_sky = through(base.wall_to_sky, wall_tree_sky);

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
    let sunlit = sunlit_wall_fraction(input, &base, direct_wall_shadow);
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
    // `:582` `max(2*fv/fg, Fsv+Fgv)`：`MAX_EXPR <Fsv+Fgv, (fv*2)/fg>`
    let tree_view_sum =
        (sky.cover + ground.cover).max((vegetation.cover_fraction * 2.0) / base.ground_fraction);
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
        tree_view_sum,
        vegetation.cover_fraction,
    )
}

/// `cos(PI/3)` 的编译期折叠值（`:396` GIMPLE 的除数 `5.0000000000000011102e-1`）。
const COS_PI_OVER_3: f64 = f64::from_bits(0x3FE0_0000_0000_0001);

/// `fwsun = 0.5*(Sw*fg+fb) / (4/PI*fb*HL*tan(theta) + fb)`（`:538`）：
/// 分子 `.FMA (fg, Sw, fb)`，分母 `.FMA ((fb*(4/PI))*HL, tan, fb)`。
fn sunlit_wall_fraction(input: UrbanLongwaveInput, base: &Base, shadow: f64) -> f64 {
    let numerator = base.ground_fraction.mul_add(shadow, base.roof_fraction) * 0.5;
    let slope = base.roof_fraction * (4.0 / std::f64::consts::PI) * input.building_height_to_length;
    numerator / slope.mul_add(input.zenith_angle_radians.tan(), base.roof_fraction)
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
    // `:426` `fv_ = fv - fv*Sw_`：`.FNMA (fv, Sw_, fv)`
    let tree_fraction = (-cover_fraction).mul_add(partial_shadow, cover_fraction);
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
    tree_view_sum: f64,
    vegetation_cover: f64,
) -> Result<UrbanLongwaveTransfer> {
    let base = Base::new(input);
    let impervious_ground = base.impervious_fraction;
    let pervious_ground = input.pervious_ground_fraction;
    let roof = input.roof_fraction;
    let hl = input.building_height_to_length;
    let fg = base.ground_fraction;
    // GIMPLE（`:564-613`，无树 `:182-219` 同形）：`t**4 = (t*t)*(t*t)`、`t**3 = (t*t)*t`；
    // 墙面 `4*fw*HL*fb/fg*stefnc*ewall*t**4` 收成 `ewall*(((fb*(HL*(fw*4)))/fg)*stefnc)*t⁴`，
    // `fb*(HL*(fw*4))` 同时就是 `fcover`；地面 `egimp*(fgimp*stefnc)*t⁴`；
    // `B = .FMA (1-e, I, B1)`，`I` 是 `(LW*Fsw_)*fw` 与 `fgimp*(LW*Fsg_)`。
    let fourth = |t: f64| {
        let square = t * t;
        square * square
    };
    let cube = |t: f64| (t * t) * t;
    let wall_cover = |fraction: f64| roof * (hl * (fraction * 4.0));
    let wall_emitted = |fraction: f64, t: f64| {
        input.wall_emissivity * ((wall_cover(fraction) / fg) * STEFAN_BOLTZMANN) * fourth(t)
    };
    let wall_derivative = |fraction: f64, t: f64| {
        input.wall_emissivity
            * (((roof * (hl * (fraction * 16.0))) / fg) * STEFAN_BOLTZMANN)
            * cube(t)
    };
    let ground_emitted =
        |fraction: f64, e: f64, t: f64| e * (fraction * STEFAN_BOLTZMANN) * fourth(t);
    let ground_derivative =
        |fraction: f64, e: f64, t: f64| e * ((fraction * 4.0) * STEFAN_BOLTZMANN) * cube(t);
    let tree = surface_count == SURFACES;
    let emitted = [
        wall_emitted(wall_fraction[0], input.sunlit_wall_temperature_k),
        wall_emitted(wall_fraction[1], input.shaded_wall_temperature_k),
        ground_emitted(
            impervious_ground,
            input.impervious_emissivity,
            input.impervious_temperature_k,
        ),
        ground_emitted(
            pervious_ground,
            input.pervious_emissivity,
            input.pervious_temperature_k,
        ),
        // `:582` `max(2*fv/fg, Fsv+Fgv)*stefnc*ev`
        if tree {
            (tree_view_sum * STEFAN_BOLTZMANN) * vegetation_emissivity
        } else {
            0.0
        },
    ];
    let incident_wall = input.downward_longwave_w_m2 * incident_view_factor[0];
    let incident_ground = input.downward_longwave_w_m2 * incident_view_factor[1];
    let incident = [
        incident_wall * wall_fraction[0],
        incident_wall * wall_fraction[1],
        impervious_ground * incident_ground,
        pervious_ground * incident_ground,
    ];
    let reflectance = [
        1.0 - input.wall_emissivity,
        1.0 - input.wall_emissivity,
        1.0 - input.impervious_emissivity,
        1.0 - input.pervious_emissivity,
    ];
    let mut source = emitted;
    for i in 0..4 {
        source[i] = reflectance[i].mul_add(incident[i], emitted[i]);
    }
    let temperature_derivative = [
        wall_derivative(wall_fraction[0], input.sunlit_wall_temperature_k),
        wall_derivative(wall_fraction[1], input.shaded_wall_temperature_k),
        ground_derivative(
            impervious_ground,
            input.impervious_emissivity,
            input.impervious_temperature_k,
        ),
        ground_derivative(
            pervious_ground,
            input.pervious_emissivity,
            input.pervious_temperature_k,
        ),
        // `:598` `4*max(…)*stefnc*ev` → `ev*((max*4)*stefnc)`
        if tree {
            vegetation_emissivity * ((tree_view_sum * 4.0) * STEFAN_BOLTZMANN)
        } else {
            0.0
        },
    ];
    Ok(UrbanLongwaveTransfer {
        surface_count,
        inverse: colm_lapack::matrix_inverse(&matrix, surface_count)?,
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
            wall_cover(wall_fraction[0]),
            wall_cover(wall_fraction[1]),
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
