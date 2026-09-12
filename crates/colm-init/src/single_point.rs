//! Native static single-point initialization for the common CoLM restart family.
//!
//! This deliberately accepts only the complete `srfdata.nc` contract.  A scientific
//! field missing from landdata is an error here; the Rust `mksrfdata` path owns rawdata
//! completion before this stage runs.

use std::path::Path;

use anyhow::{ensure, Result};

use crate::{
    derive_igbp_canopy, derive_lake_layers, derive_soil_parameters, derive_usgs_canopy,
    normalize_soil_texture, read_single_point_surface, write_constant_restart,
    ConstantRestartFiles, ConstantRestartInput, HydraulicModel, LandCoverScheme, RestartDimensions,
    RestartPatchFields, RestartTuning, SoilAlbedo,
};

/// Immutable single-point arguments that affect the common constant restart files.
#[derive(Debug, Clone, Copy)]
pub struct SinglePointStaticConfig<'a> {
    pub case_name: &'a str,
    pub land_cover_year: i32,
    pub block_label: &'a str,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub tuning: RestartTuning,
}

impl<'a> SinglePointStaticConfig<'a> {
    /// Standard CoLM dimensions and namelist tuning defaults for one native surface.
    pub fn new(
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
        land_cover: LandCoverScheme,
        hydraulic_model: HydraulicModel,
    ) -> Self {
        Self {
            case_name,
            land_cover_year,
            block_label,
            land_cover,
            hydraulic_model,
            tuning: RestartTuning::default(),
        }
    }
}

/// Writes the common constant restart pair for one self-contained `srfdata.nc` file.
///
/// This is the native equivalent of `MOD_Initialize.F90` sections 1.1--1.6 for a
/// single non-PFT/non-urban patch.  Feature-specific restart families stay separate so
/// callers cannot accidentally emit a common restart and mistake it for PFT or urban
/// initialization.
pub fn write_single_point_constant_restart(
    surface: impl AsRef<Path>,
    restart_dir: impl AsRef<Path>,
    config: SinglePointStaticConfig<'_>,
) -> Result<ConstantRestartFiles> {
    let surface = read_single_point_surface(surface, config.land_cover, config.hydraulic_model)?;
    let class = [surface.land_class];
    let kind = [patch_type(config.land_cover, surface.land_class)?];
    let lake = derive_lake_layers(
        &[surface.lake_depth_m],
        RestartDimensions::default().lake_layers,
    )?;
    let soil = derive_soil_parameters(
        &surface.soil_layers,
        &kind,
        RestartDimensions::default().soil_layers,
        config.hydraulic_model,
    )?;
    let observed_top = [surface.canopy_height_m];
    let canopy = match config.land_cover {
        LandCoverScheme::Igbp => {
            derive_igbp_canopy(&class, &kind, &observed_top, &IGBP_TOP, &IGBP_BOTTOM, None)?
        }
        LandCoverScheme::Usgs => derive_usgs_canopy(&class, &USGS_TOP, &USGS_BOTTOM)?,
    };
    let mut texture = [surface.soil_texture];
    normalize_soil_texture(&mut texture);
    let bvic = [BVIC_USDA[texture[0] as usize]];
    let longitude_radians = [surface.longitude_degrees.to_radians()];
    let latitude_radians = [surface.latitude_degrees.to_radians()];
    let albedo = [surface.albedo];
    let elevation = [surface.elevation_m];
    let elevation_std = [surface.elevation_std_m];
    let slope = [surface.slope_ratio];
    let zeros = [0.0];

    write_constant_restart(
        restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        ConstantRestartInput {
            dimensions: RestartDimensions::default(),
            patch: RestartPatchFields {
                class: &class,
                kind: &kind,
                mask: &[true],
                longitude_radians: &longitude_radians,
                latitude_radians: &latitude_radians,
                albedo: SoilAlbedo {
                    saturated_visible: &[albedo[0].saturated_visible],
                    dry_visible: &[albedo[0].dry_visible],
                    saturated_near_infrared: &[albedo[0].saturated_near_infrared],
                    dry_near_infrared: &[albedo[0].dry_near_infrared],
                },
                bvic: &bvic,
                soil_texture: &texture,
                vic_b_infilt: &zeros,
                vic_dsmax: &zeros,
                vic_ds: &zeros,
                vic_ws: &zeros,
                vic_c: &zeros,
                elevation_mean_m: &elevation,
                elevation_std_m: &elevation_std,
                slope_ratio: &slope,
            },
            lake: &lake,
            soil: &soil,
            canopy: &canopy,
            tuning: config.tuning,
            uses_van_genuchten: config.hydraulic_model == HydraulicModel::VanGenuchten,
            bedrock: None,
            topmodel: None,
            terrain: None,
            simple_terrain: None,
            hyperspectral_albedo: None,
        },
    )
}

fn patch_type(land_cover: LandCoverScheme, class: i32) -> Result<i32> {
    let types = match land_cover {
        LandCoverScheme::Igbp => &IGBP_PATCH_TYPE[..],
        LandCoverScheme::Usgs => &USGS_PATCH_TYPE[..],
    };
    let index =
        usize::try_from(class).map_err(|_| anyhow::anyhow!("land class {class} is negative"))?;
    ensure!(
        index > 0 && index < types.len(),
        "land class {class} is outside the selected CoLM land-cover table"
    );
    Ok(types[index])
}

// `main/MOD_Const_LC.F90`.  Index zero is deliberately unused because CoLM's land
// classifications are one-based, unlike its `patchtype` values.
const IGBP_PATCH_TYPE: [i32; 18] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 3, 0, 4];
const USGS_PATCH_TYPE: [i32; 25] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 2, 2, 0, 0, 0, 0, 0, 3,
];
const IGBP_TOP: [f64; 18] = [
    0.0, 17.0, 35.0, 17.0, 20.0, 20.0, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5,
];
const IGBP_BOTTOM: [f64; 18] = [
    0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
const USGS_TOP: [f64; 25] = [
    0.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 20.0, 17.0, 35.0, 17.0, 20.0, 0.5, 0.5,
    17.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
];
const USGS_BOTTOM: [f64; 25] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
const BVIC_USDA: [f64; 13] = [
    1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180, 0.100, 0.090, 0.150, 0.080, 0.050,
];

#[cfg(test)]
#[path = "single_point_tests.rs"]
mod single_point_tests;
