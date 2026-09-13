//! Urban cold-start shortwave transfer shared by initialization and runtime.
//!
//! This is the `MOD_Urban_Albedo` / `MOD_Urban_Shortwave` path.  Keeping it in
//! `colm-core` means the restart writer and the Rust time-stepper use precisely
//! the same geometry, snow optics, and multiple-reflection calculation.

use anyhow::{ensure, Result};

use crate::{pc_radiation::canopy_scattering, pc_radiation::canopy_transmittance, LeafOptics};

const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;
const FREEZING_K: f64 = 273.16;

/// Inputs to CoLM's urban albedo initialization.  Arrays use CoLM's native
/// `[band][direct, diffuse]` order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanRadiationInput {
    pub roof_fraction: f64,
    pub pervious_ground_fraction: f64,
    pub water_fraction: f64,
    pub building_height_to_length: f64,
    pub roof_height_m: f64,
    pub roof_albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub wall_albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub impervious_albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub pervious_albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub leaf_optics: LeafOptics,
    pub vegetation_fraction: f64,
    pub vegetation_center_height_m: f64,
    pub lai: f64,
    pub sai: f64,
    pub wet_snow_fraction: f64,
    pub vegetation_snow: bool,
    pub cosine_zenith: f64,
    /// Wall sunlit fraction from the previous time step.
    pub previous_sunlit_wall_fraction: f64,
    pub lake_temperature_k: f64,
    pub roof_snow_fraction: f64,
    pub impervious_snow_fraction: f64,
    pub pervious_snow_fraction: f64,
    pub lake_snow_fraction: f64,
    pub roof_snow_water_mm: f64,
    pub impervious_snow_water_mm: f64,
    pub pervious_snow_water_mm: f64,
    pub lake_snow_water_mm: f64,
    pub roof_snow_age: f64,
    pub impervious_snow_age: f64,
    pub pervious_snow_age: f64,
    pub lake_snow_age: f64,
}

/// Results of `UrbanIniTimeVar`'s `alburban` call.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanRadiationState {
    pub sunlit_wall_fraction: f64,
    pub change_in_sunlit_wall_fraction: f64,
    pub diffuse_extinction: f64,
    pub albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub sunlit_tree_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub shaded_tree_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub roof_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub sunlit_wall_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub shaded_wall_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub impervious_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub pervious_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub lake_absorption: [[f64; RADIATION_TYPES]; BANDS],
}

/// Ports `MOD_Urban_Albedo::alburban` without I/O or mutable global state.
pub fn cold_start_urban_radiation(input: UrbanRadiationInput) -> Result<UrbanRadiationState> {
    validate(input)?;
    let mut state = UrbanRadiationState {
        sunlit_wall_fraction: input.previous_sunlit_wall_fraction,
        change_in_sunlit_wall_fraction: 0.0,
        diffuse_extinction: 0.718,
        albedo: [[1.0; RADIATION_TYPES]; BANDS],
        sunlit_tree_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        shaded_tree_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        roof_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        sunlit_wall_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        shaded_wall_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        impervious_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        pervious_absorption: [[0.0; RADIATION_TYPES]; BANDS],
        lake_absorption: [[0.0; RADIATION_TYPES]; BANDS],
    };
    // CoLM intentionally leaves these initialized values untouched at night.
    if input.cosine_zenith <= -0.3 {
        return Ok(state);
    }

    let czen = input.cosine_zenith.max(0.01);
    let snow = |water: f64, age: f64| snow_albedo(czen, water, age);
    let roof = mix_snow(
        input.roof_albedo,
        snow(input.roof_snow_water_mm, input.roof_snow_age),
        input.roof_snow_fraction,
    );
    let impervious = mix_snow(
        input.impervious_albedo,
        snow(input.impervious_snow_water_mm, input.impervious_snow_age),
        input.impervious_snow_fraction,
    );
    let pervious = mix_snow(
        input.pervious_albedo,
        snow(input.pervious_snow_water_mm, input.pervious_snow_age),
        input.pervious_snow_fraction,
    );
    let mut lake = lake_albedo(czen, input.lake_temperature_k);
    lake = mix_snow(
        lake,
        snow(input.lake_snow_water_mm, input.lake_snow_age),
        input.lake_snow_fraction,
    );
    state.lake_absorption = complement(lake);

    let leaf_area = input.lai + input.sai;
    let effective_optics = effective_optics(input, leaf_area);
    let theta = czen.acos();
    for band in 0..BANDS {
        let result = if leaf_area > 1.0e-6 && input.vegetation_fraction > 0.0 {
            urban_vegetated_shortwave(
                theta,
                input,
                roof[band][0],
                input.wall_albedo[band][0],
                impervious[band][0],
                pervious[band][0],
                effective_optics.0[band],
                effective_optics.1[band],
            )?
        } else {
            urban_only_shortwave(
                theta,
                input,
                roof[band][0],
                input.wall_albedo[band][0],
                impervious[band][0],
                pervious[band][0],
            )?
        };
        state.sunlit_wall_fraction = result.sunlit_wall_fraction;
        state.roof_absorption[band] = result.roof;
        state.sunlit_wall_absorption[band] = result.sunlit_wall;
        state.shaded_wall_absorption[band] = result.shaded_wall;
        state.impervious_absorption[band] = result.impervious;
        state.pervious_absorption[band] = result.pervious;
        state.sunlit_tree_absorption[band] = result.tree;
        state.albedo[band] = result.albedo;
    }
    state.change_in_sunlit_wall_fraction =
        state.sunlit_wall_fraction - input.previous_sunlit_wall_fraction;
    for (albedo, lake) in state.albedo.iter_mut().zip(lake) {
        for (albedo, lake) in albedo.iter_mut().zip(lake) {
            *albedo = (1.0 - input.water_fraction) * *albedo + input.water_fraction * lake;
        }
    }
    Ok(state)
}

#[derive(Debug, Clone, Copy)]
struct BandResult {
    sunlit_wall_fraction: f64,
    roof: [f64; RADIATION_TYPES],
    sunlit_wall: [f64; RADIATION_TYPES],
    shaded_wall: [f64; RADIATION_TYPES],
    impervious: [f64; RADIATION_TYPES],
    pervious: [f64; RADIATION_TYPES],
    tree: [f64; RADIATION_TYPES],
    albedo: [f64; RADIATION_TYPES],
}

fn urban_only_shortwave(
    theta: f64,
    input: UrbanRadiationInput,
    roof: f64,
    wall: f64,
    impervious: f64,
    pervious: f64,
) -> Result<BandResult> {
    let building = input.roof_fraction;
    let ground = 1.0 - building;
    let impervious_fraction = 1.0 - input.pervious_ground_fraction;
    let sky_to_wall = wall_shadow_diffuse(building / ground, input.building_height_to_length);
    let sky_to_ground = 1.0 - sky_to_wall;
    let wall_to_sky =
        sky_to_wall * ground / building / (2.0 * input.building_height_to_length) * 0.75;
    let wall_to_ground =
        sky_to_wall * ground / building / (2.0 * input.building_height_to_length) * 0.25;
    let wall_to_wall = 1.0 - wall_to_sky - wall_to_ground;
    let shadow = wall_shadow_direct(building / ground, input.building_height_to_length, theta);
    let sunlit = 0.5 * (shadow * ground + building)
        / (4.0 / std::f64::consts::PI * building * input.building_height_to_length * theta.tan()
            + building);
    let shaded = 1.0 - sunlit;
    let matrix = [
        [
            1.0 - wall_to_wall * sunlit * wall,
            -wall_to_wall * sunlit * wall,
            -sky_to_wall * sunlit * wall,
            -sky_to_wall * sunlit * wall,
        ],
        [
            -wall_to_wall * shaded * wall,
            1.0 - wall_to_wall * shaded * wall,
            -sky_to_wall * shaded * wall,
            -sky_to_wall * shaded * wall,
        ],
        [
            -wall_to_ground * impervious_fraction * impervious,
            -wall_to_ground * impervious_fraction * impervious,
            1.0,
            0.0,
        ],
        [
            -wall_to_ground * input.pervious_ground_fraction * pervious,
            -wall_to_ground * input.pervious_ground_fraction * pervious,
            0.0,
            1.0,
        ],
    ];
    let direct = solve(
        matrix,
        [
            shadow * wall,
            0.0,
            (1.0 - shadow) * impervious_fraction * impervious,
            (1.0 - shadow) * input.pervious_ground_fraction * pervious,
        ],
    )?;
    let diffuse = solve(
        matrix,
        [
            sky_to_wall * sunlit * wall,
            sky_to_wall * shaded * wall,
            sky_to_ground * impervious_fraction * impervious,
            sky_to_ground * input.pervious_ground_fraction * pervious,
        ],
    )?;
    Ok(band_result(
        sunlit,
        roof,
        wall,
        impervious,
        pervious,
        building,
        ground,
        input.pervious_ground_fraction,
        input.building_height_to_length,
        [wall_to_sky, sky_to_ground],
        direct,
        diffuse,
        None,
    ))
}

#[allow(clippy::too_many_arguments)]
fn urban_vegetated_shortwave(
    theta: f64,
    input: UrbanRadiationInput,
    roof: f64,
    wall: f64,
    impervious: f64,
    pervious: f64,
    reflectance: f64,
    transmittance: f64,
) -> Result<BandResult> {
    let building = input.roof_fraction;
    let ground = 1.0 - building;
    let impervious_fraction = 1.0 - input.pervious_ground_fraction;
    let length = input.roof_height_m / input.building_height_to_length;
    let canopy_area = input.lai + input.sai;
    let tree_depth = canopy_area * input.vegetation_fraction
        / (std::f64::consts::PI / 3.0).cos()
        / tree_shadow(input.vegetation_fraction, std::f64::consts::PI / 3.0);
    let tree_transmission = canopy_transmittance(3.0 / 8.0 * tree_depth);
    let (tree_albedo, _, _) = canopy_scattering(
        3.0 / 8.0 * tree_depth,
        transmittance + reflectance,
        transmittance,
        reflectance,
    );

    let sky_to_wall = wall_shadow_diffuse(building / ground, input.building_height_to_length);
    let sky_to_ground = 1.0 - sky_to_wall;
    let wall_to_sky =
        sky_to_wall * ground / building / (2.0 * input.building_height_to_length) * 0.75;
    let wall_to_ground =
        sky_to_wall * ground / building / (2.0 * input.building_height_to_length) * 0.25;
    let wall_to_wall = 1.0 - wall_to_sky - wall_to_ground;

    let first_shadow = wall_shadow_diffuse(building / ground, input.building_height_to_length);
    let upper_shadow = wall_shadow_diffuse(
        building / ground,
        (input.roof_height_m - input.vegetation_center_height_m) / length,
    );
    let mut tree_fraction = input.vegetation_fraction - input.vegetation_fraction * upper_shadow;
    let mut tree_shadow_factor = tree_shadow(tree_fraction, std::f64::consts::PI / 3.0);
    let mut overlap = tree_shadow_factor * (first_shadow - upper_shadow);
    tree_shadow_factor = (tree_shadow_factor / ground).min(1.0);
    if tree_shadow_factor + first_shadow - overlap > 1.0 {
        overlap = tree_shadow_factor + first_shadow - 1.0;
    }
    let sky_to_tree = tree_shadow_factor;
    let sky_tree_wall = overlap;
    let sky_tree_ground = sky_to_tree - sky_tree_wall;
    let tree_to_sky = 0.5 * (1.0 - upper_shadow);

    let lower_shadow =
        wall_shadow_diffuse(building / ground, input.vegetation_center_height_m / length);
    tree_fraction = input.vegetation_fraction - input.vegetation_fraction * lower_shadow;
    tree_shadow_factor = tree_shadow(tree_fraction, std::f64::consts::PI / 3.0);
    overlap = tree_shadow_factor * (first_shadow - lower_shadow);
    tree_shadow_factor = (tree_shadow_factor / ground).min(1.0);
    if tree_shadow_factor + first_shadow - overlap > 1.0 {
        overlap = tree_shadow_factor + first_shadow - 1.0;
    }
    let ground_to_tree = tree_shadow_factor;
    let ground_tree_wall = overlap;
    let ground_tree_sky = ground_to_tree - ground_tree_wall;
    let tree_to_ground = 0.5 * (1.0 - lower_shadow);
    let tree_to_wall = 1.0 - tree_to_sky - tree_to_ground;

    let mut wall_to_tree = input
        .vegetation_fraction
        .max(0.5 * (sky_to_tree + ground_to_tree))
        * 2.0
        * ground
        * tree_to_wall
        / (4.0 * input.building_height_to_length * building);
    wall_to_tree = wall_to_tree.min(0.8);
    let top = input.vegetation_center_height_m / input.roof_height_m;
    let bottom = (input.roof_height_m - input.vegetation_center_height_m) / input.roof_height_m;
    let mut wall_tree_wall = wall_to_tree
        / (1.0 + wall_to_sky * top / wall_to_wall + wall_to_ground * bottom / wall_to_wall);
    let mut wall_tree_sky = wall_to_sky * top / wall_to_wall * wall_tree_wall;
    let mut wall_tree_ground = wall_to_ground * bottom / wall_to_wall * wall_tree_wall;
    wall_tree_wall = wall_tree_wall.min(wall_to_wall);
    wall_tree_sky = wall_tree_sky.min(wall_to_sky);
    wall_tree_ground = wall_tree_ground.min(wall_to_ground);
    wall_to_tree = wall_tree_wall + wall_tree_sky + wall_tree_ground;

    let sky_to_wall_adjusted = sky_to_wall - sky_tree_wall + sky_tree_wall * tree_transmission;
    let sky_to_ground_adjusted =
        sky_to_ground - sky_tree_ground + sky_tree_ground * tree_transmission;
    let ground_to_wall_adjusted =
        sky_to_wall - ground_tree_wall + ground_tree_wall * tree_transmission;
    let ground_to_sky_adjusted =
        sky_to_ground - ground_tree_sky + ground_tree_sky * tree_transmission;
    let wall_to_ground_adjusted =
        wall_to_ground - wall_tree_ground + wall_tree_ground * tree_transmission;
    let wall_to_wall_adjusted = wall_to_wall - wall_tree_wall + wall_tree_wall * tree_transmission;
    let wall_to_sky_adjusted = wall_to_sky - wall_tree_sky + wall_tree_sky * tree_transmission;

    let initial_shadow =
        wall_shadow_direct(building / ground, input.building_height_to_length, theta);
    let lower_direct_shadow = wall_shadow_direct(
        building / ground,
        (input.roof_height_m - input.vegetation_center_height_m) / length,
        theta,
    );
    let direct_tree_fraction =
        input.vegetation_fraction - input.vegetation_fraction * lower_direct_shadow;
    let mut direct_tree_shadow = tree_shadow(direct_tree_fraction, theta);
    let mut direct_overlap = (initial_shadow - lower_direct_shadow) * direct_tree_shadow;
    direct_tree_shadow = (direct_tree_shadow / ground).min(1.0);
    if direct_tree_shadow + initial_shadow - direct_overlap > 1.0 {
        direct_overlap = direct_tree_shadow + initial_shadow - 1.0;
    }
    let direct_wall_shadow = initial_shadow - direct_overlap;
    let sunlit = 0.5 * (direct_wall_shadow * ground + building)
        / (4.0 / std::f64::consts::PI * building * input.building_height_to_length * theta.tan()
            + building);
    let shaded = 1.0 - sunlit;
    let matrix = [
        [
            1.0 - wall_to_wall_adjusted * sunlit * wall,
            -wall_to_wall_adjusted * sunlit * wall,
            -ground_to_wall_adjusted * sunlit * wall,
            -ground_to_wall_adjusted * sunlit * wall,
            -tree_to_wall * sunlit * wall,
        ],
        [
            -wall_to_wall_adjusted * shaded * wall,
            1.0 - wall_to_wall_adjusted * shaded * wall,
            -ground_to_wall_adjusted * shaded * wall,
            -ground_to_wall_adjusted * shaded * wall,
            -tree_to_wall * shaded * wall,
        ],
        [
            -wall_to_ground_adjusted * impervious_fraction * impervious,
            -wall_to_ground_adjusted * impervious_fraction * impervious,
            1.0,
            0.0,
            -tree_to_ground * impervious_fraction * impervious,
        ],
        [
            -wall_to_ground_adjusted * input.pervious_ground_fraction * pervious,
            -wall_to_ground_adjusted * input.pervious_ground_fraction * pervious,
            0.0,
            1.0,
            -tree_to_ground * input.pervious_ground_fraction * pervious,
        ],
        [
            -wall_to_tree * tree_albedo,
            -wall_to_tree * tree_albedo,
            -ground_to_tree * tree_albedo,
            -ground_to_tree * tree_albedo,
            1.0,
        ],
    ];
    let direct_ground = 1.0 - direct_wall_shadow - direct_tree_shadow
        + (direct_tree_shadow - direct_overlap) * tree_transmission;
    let direct = solve(
        matrix,
        [
            direct_wall_shadow * wall,
            direct_overlap * tree_transmission * wall,
            direct_ground * impervious_fraction * impervious,
            direct_ground * input.pervious_ground_fraction * pervious,
            direct_tree_shadow * tree_albedo,
        ],
    )?;
    let diffuse = solve(
        matrix,
        [
            sky_to_wall_adjusted * sunlit * wall,
            sky_to_wall_adjusted * shaded * wall,
            sky_to_ground_adjusted * impervious_fraction * impervious,
            sky_to_ground_adjusted * input.pervious_ground_fraction * pervious,
            sky_to_tree * tree_albedo,
        ],
    )?;
    Ok(band_result(
        sunlit,
        roof,
        wall,
        impervious,
        pervious,
        building,
        ground,
        input.pervious_ground_fraction,
        input.building_height_to_length,
        [wall_to_sky_adjusted, ground_to_sky_adjusted],
        direct,
        diffuse,
        Some((
            input.vegetation_fraction,
            tree_albedo,
            tree_transmission,
            tree_to_sky,
        )),
    ))
}

#[allow(clippy::too_many_arguments)]
fn band_result<const N: usize>(
    sunlit_wall_fraction: f64,
    roof: f64,
    wall: f64,
    impervious: f64,
    pervious: f64,
    building: f64,
    ground: f64,
    pervious_ground: f64,
    height_to_length: f64,
    [wall_to_sky, ground_to_sky]: [f64; 2],
    direct: [f64; N],
    diffuse: [f64; N],
    tree: Option<(f64, f64, f64, f64)>,
) -> BandResult {
    let shaded = 1.0 - sunlit_wall_fraction;
    let impervious_ground = 1.0 - pervious_ground;
    let (tree_absorption, tree_albedo) = match tree {
        Some((fraction, albedo, transmission, tree_to_sky)) => (
            [
                direct[4] / albedo * (1.0 - albedo - transmission) / fraction * ground,
                diffuse[4] / albedo * (1.0 - albedo - transmission) / fraction * ground,
            ],
            [direct[4] * tree_to_sky, diffuse[4] * tree_to_sky],
        ),
        None => ([0.0; RADIATION_TYPES], [0.0; RADIATION_TYPES]),
    };
    let mut result = BandResult {
        sunlit_wall_fraction,
        roof: [1.0 - roof; RADIATION_TYPES],
        sunlit_wall: [
            direct[0] / wall * (1.0 - wall),
            diffuse[0] / wall * (1.0 - wall),
        ],
        shaded_wall: [
            direct[1] / wall * (1.0 - wall),
            diffuse[1] / wall * (1.0 - wall),
        ],
        impervious: [
            direct[2] / impervious * (1.0 - impervious),
            diffuse[2] / impervious * (1.0 - impervious),
        ],
        pervious: [
            direct[3] / pervious * (1.0 - pervious),
            diffuse[3] / pervious * (1.0 - pervious),
        ],
        tree: tree_absorption,
        albedo: [
            roof * building
                + ground
                    * (direct[0] * wall_to_sky
                        + direct[1] * wall_to_sky
                        + direct[2] * ground_to_sky
                        + direct[3] * ground_to_sky
                        + tree_albedo[0]),
            roof * building
                + ground
                    * (diffuse[0] * wall_to_sky
                        + diffuse[1] * wall_to_sky
                        + diffuse[2] * ground_to_sky
                        + diffuse[3] * ground_to_sky
                        + tree_albedo[1]),
        ],
    };
    if building > 0.0 {
        let scale = ground / (4.0 * sunlit_wall_fraction * height_to_length * building);
        result.sunlit_wall[0] *= scale;
        result.sunlit_wall[1] *= scale;
        let shaded_scale = ground / (4.0 * shaded * height_to_length * building);
        result.shaded_wall[0] *= shaded_scale;
        result.shaded_wall[1] *= shaded_scale;
    }
    if impervious_ground > 0.0 {
        result.impervious[0] /= impervious_ground;
        result.impervious[1] /= impervious_ground;
    }
    if pervious_ground > 0.0 {
        result.pervious[0] /= pervious_ground;
        result.pervious[1] /= pervious_ground;
    }
    result
}

fn effective_optics(input: UrbanRadiationInput, area: f64) -> ([f64; BANDS], [f64; BANDS]) {
    let mut reflectance = [0.0; BANDS];
    let mut transmittance = [0.0; BANDS];
    if area > 1.0e-6 && input.vegetation_fraction > 0.0 {
        for band in 0..BANDS {
            reflectance[band] = (input.leaf_optics.reflectance[band][0] * input.lai
                + input.leaf_optics.reflectance[band][1] * input.sai)
                / area;
            transmittance[band] = (input.leaf_optics.transmittance[band][0] * input.lai
                + input.leaf_optics.transmittance[band][1] * input.sai)
                / area;
        }
    }
    if input.vegetation_snow {
        for band in 0..BANDS {
            let (snow_reflectance, snow_transmittance) =
                if band == 0 { (0.5, 0.3) } else { (0.2, 0.2) };
            reflectance[band] = (1.0 - input.wet_snow_fraction) * reflectance[band]
                + input.wet_snow_fraction * snow_reflectance;
            transmittance[band] = (1.0 - input.wet_snow_fraction) * transmittance[band]
                + input.wet_snow_fraction * snow_transmittance;
        }
    }
    (reflectance, transmittance)
}

fn snow_albedo(czen: f64, snow_water_mm: f64, snow_age: f64) -> [[f64; RADIATION_TYPES]; BANDS] {
    if snow_water_mm <= 0.0 {
        return [[0.0; RADIATION_TYPES]; BANDS];
    }
    let age = 1.0 - 1.0 / (1.0 + snow_age);
    let diffuse_visible = 0.85 * (1.0 - 0.2 * age);
    let diffuse_near_infrared = 0.65 * (1.0 - 0.5 * age);
    let correction = ((1.0 + 0.5) / (1.0 + czen * 4.0) - 0.5).max(0.0) * 0.4;
    [
        [
            diffuse_visible + correction * (1.0 - diffuse_visible),
            diffuse_visible,
        ],
        [
            diffuse_near_infrared + correction * (1.0 - diffuse_near_infrared),
            diffuse_near_infrared,
        ],
    ]
}

fn lake_albedo(czen: f64, lake_temperature_k: f64) -> [[f64; RADIATION_TYPES]; BANDS] {
    if lake_temperature_k < FREEZING_K {
        [[0.6; RADIATION_TYPES], [0.4; RADIATION_TYPES]]
    } else {
        [[0.05 / (czen + 0.15), 0.1], [0.05 / (czen + 0.15), 0.1]]
    }
}

fn mix_snow(
    surface: [[f64; RADIATION_TYPES]; BANDS],
    snow: [[f64; RADIATION_TYPES]; BANDS],
    fraction: f64,
) -> [[f64; RADIATION_TYPES]; BANDS] {
    std::array::from_fn(|band| {
        std::array::from_fn(|radiation_type| {
            (1.0 - fraction) * surface[band][radiation_type] + fraction * snow[band][radiation_type]
        })
    })
}

fn complement(values: [[f64; RADIATION_TYPES]; BANDS]) -> [[f64; RADIATION_TYPES]; BANDS] {
    std::array::from_fn(|band| {
        std::array::from_fn(|radiation_type| 1.0 - values[band][radiation_type])
    })
}

fn wall_shadow_direct(fraction: f64, height_to_length: f64, theta: f64) -> f64 {
    1.0 - (-4.0 / std::f64::consts::PI * fraction * height_to_length * theta.tan()).exp()
}

fn wall_shadow_diffuse(fraction: f64, height_to_length: f64) -> f64 {
    let angle =
        (53.0 - (fraction * height_to_length * 100.0).sqrt()) / 180.0 * std::f64::consts::PI;
    1.0 - (-4.0 / std::f64::consts::PI * fraction * height_to_length * angle.tan()).exp()
}

fn tree_shadow(fraction: f64, theta: f64) -> f64 {
    let cosine = theta.cos();
    fraction.max((1.0 - (-fraction / cosine).exp()) / (1.0 - fraction * (-1.0 / cosine).exp()))
}

fn solve<const N: usize>(mut matrix: [[f64; N]; N], mut rhs: [f64; N]) -> Result<[f64; N]> {
    for pivot in 0..N {
        let row = (pivot..N)
            .max_by(|&left, &right| {
                matrix[left][pivot]
                    .abs()
                    .total_cmp(&matrix[right][pivot].abs())
            })
            .expect("nonempty pivot range");
        ensure!(
            matrix[row][pivot].abs() > 1.0e-14,
            "urban radiation matrix is singular"
        );
        if row != pivot {
            matrix.swap(row, pivot);
            rhs.swap(row, pivot);
        }
        let pivot_row = matrix[pivot];
        for lower in (pivot + 1)..N {
            let factor = matrix[lower][pivot] / pivot_row[pivot];
            for (column, value) in matrix[lower].iter_mut().enumerate().skip(pivot) {
                *value -= factor * pivot_row[column];
            }
            rhs[lower] -= factor * rhs[pivot];
        }
    }
    let mut result = [0.0; N];
    for row in (0..N).rev() {
        result[row] = (rhs[row]
            - ((row + 1)..N)
                .map(|column| matrix[row][column] * result[column])
                .sum::<f64>())
            / matrix[row][row];
    }
    Ok(result)
}

fn validate(input: UrbanRadiationInput) -> Result<()> {
    let values = [
        input.roof_fraction,
        input.pervious_ground_fraction,
        input.water_fraction,
        input.building_height_to_length,
        input.roof_height_m,
        input.vegetation_fraction,
        input.vegetation_center_height_m,
        input.lai,
        input.sai,
        input.wet_snow_fraction,
        input.cosine_zenith,
        input.previous_sunlit_wall_fraction,
        input.lake_temperature_k,
        input.roof_snow_fraction,
        input.impervious_snow_fraction,
        input.pervious_snow_fraction,
        input.lake_snow_fraction,
        input.roof_snow_water_mm,
        input.impervious_snow_water_mm,
        input.pervious_snow_water_mm,
        input.lake_snow_water_mm,
        input.roof_snow_age,
        input.impervious_snow_age,
        input.pervious_snow_age,
        input.lake_snow_age,
    ];
    ensure!(
        values.into_iter().all(f64::is_finite),
        "urban radiation inputs must be finite"
    );
    ensure!(
        input.roof_fraction > 0.0 && input.roof_fraction < 1.0,
        "urban roof fraction must be in (0, 1)"
    );
    ensure!(
        (0.0..=1.0).contains(&input.pervious_ground_fraction),
        "urban pervious-ground fraction must be in [0, 1]"
    );
    ensure!(
        (0.0..=1.0).contains(&input.water_fraction),
        "urban water fraction must be in [0, 1]"
    );
    ensure!(
        input.building_height_to_length > 0.0 && input.roof_height_m > 0.0,
        "urban building geometry must be positive"
    );
    ensure!(
        (0.0..=1.0).contains(&input.vegetation_fraction),
        "urban vegetation fraction must be in [0, 1]"
    );
    ensure!(
        (0.0..=input.roof_height_m).contains(&input.vegetation_center_height_m),
        "urban vegetation center must be within building height"
    );
    ensure!(
        input.lai >= 0.0 && input.sai >= 0.0,
        "urban LAI and SAI must be nonnegative"
    );
    ensure!(
        (0.0..=1.0).contains(&input.wet_snow_fraction),
        "urban vegetation snow fraction must be in [0, 1]"
    );
    for (name, fraction) in [
        ("roof", input.roof_snow_fraction),
        ("impervious", input.impervious_snow_fraction),
        ("pervious", input.pervious_snow_fraction),
        ("lake", input.lake_snow_fraction),
    ] {
        ensure!(
            (0.0..=1.0).contains(&fraction),
            "urban {name} snow fraction must be in [0, 1]"
        );
    }
    ensure!(
        [
            input.roof_snow_water_mm,
            input.impervious_snow_water_mm,
            input.pervious_snow_water_mm,
            input.lake_snow_water_mm,
            input.roof_snow_age,
            input.impervious_snow_age,
            input.pervious_snow_age,
            input.lake_snow_age,
        ]
        .into_iter()
        .all(|value| value >= 0.0),
        "urban snow water and age must be nonnegative"
    );
    for (name, surface) in [
        ("roof", input.roof_albedo),
        ("wall", input.wall_albedo),
        ("impervious", input.impervious_albedo),
        ("pervious", input.pervious_albedo),
    ] {
        ensure!(
            surface
                .into_iter()
                .flatten()
                .all(|value| (0.0..1.0).contains(&value)),
            "urban {name} albedo must be in [0, 1)"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "urban_radiation_tests.rs"]
mod urban_radiation_tests;
