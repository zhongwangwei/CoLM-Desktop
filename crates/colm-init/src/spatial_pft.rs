//! Spatial PFT constant-restart adapter for Rust `mksrfdata` block artifacts.
//!
//! The common land-patch restart is written by [`crate::spatial_static`].
//! This module writes its separate `landpft` companion, exactly as CoLM's
//! `WRITE_PFTimeInvariants` does after `pct_readin` and `HTOP_readin`.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};

use crate::single_point::pft_canopy;
use crate::spatial_static::{
    block_path, values_f64, values_i32, write_spatial_lct_constant_restart, SpatialLctStaticConfig,
};
use crate::{
    write_pft_constant_restart, ConstantRestartFiles, HydraulicModel, LandCoverScheme,
    PftConstantRestartInput,
};

/// Arguments for one already-addressed spatial `landpft` block.
#[derive(Debug, Clone, Copy)]
pub struct SpatialPftStaticConfig<'a> {
    pub namelist: &'a Path,
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    /// CoLM suffix without the leading underscore, e.g. `w180_s90`.
    pub block_label: &'a str,
}

/// The common and PFT-specific constant restart blocks for one spatial PFT case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialPftConstantRestartFiles {
    pub common: ConstantRestartFiles,
    pub pft: PathBuf,
}

impl<'a> SpatialPftStaticConfig<'a> {
    pub fn new(
        namelist: &'a Path,
        landdata: &'a Path,
        restart_dir: &'a Path,
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
    ) -> Self {
        Self {
            namelist,
            landdata,
            restart_dir,
            case_name,
            land_cover_year,
            block_label,
        }
    }
}

/// Writes the PFT/PC constant restart for one Rust `mksrfdata-rs spatial-pft` block.
pub fn write_spatial_pft_constant_restart(config: SpatialPftStaticConfig<'_>) -> Result<PathBuf> {
    let class = read_i32(
        config.landdata,
        "landpft",
        "landpft",
        "settyp",
        config.land_cover_year,
        config.block_label,
    )?;
    let fraction = read_f64(
        config.landdata,
        "pctpft",
        "pct_pfts",
        "pct_pfts",
        config.land_cover_year,
        config.block_label,
    )?;
    let observed_height_m = read_f64(
        config.landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        config.land_cover_year,
        config.block_label,
    )?;
    ensure!(
        !class.is_empty()
            && class.len() == fraction.len()
            && class.len() == observed_height_m.len(),
        "spatial landpft, pct_pfts, and htop_pfts vectors must be nonempty and have equal lengths"
    );
    let text = std::fs::read_to_string(config.namelist)
        .with_context(|| format!("cannot read case namelist {}", config.namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", config.namelist.display()))?;
    let canopy = pft_canopy(&document, &class, &observed_height_m)?;
    write_pft_constant_restart(
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        PftConstantRestartInput {
            class: &class,
            fraction: &fraction,
            canopy_top_m: &canopy.top_m,
            canopy_bottom_m: &canopy.bottom_m,
            crop_fraction: None,
        },
    )
}

/// Writes both restart families required by a spatial PFT cold start.
///
/// PFT landpatches retain IGBP's water and glacier classes, so the common
/// restart uses the IGBP static adapter rather than duplicating its soil,
/// lake, and terrain mapping.  The soil model is read from the same case
/// namelist that supplies PFT canopy overrides.
pub fn write_spatial_pft_constant_restarts(
    config: SpatialPftStaticConfig<'_>,
    use_bedrock: bool,
    use_hyperspectral: bool,
) -> Result<SpatialPftConstantRestartFiles> {
    let hydraulic_model = pft_hydraulic_model(config.namelist)?;
    let mut common = SpatialLctStaticConfig::new(
        config.landdata,
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        LandCoverScheme::Igbp,
        hydraulic_model,
    );
    common.use_bedrock = use_bedrock;
    common.use_hyperspectral = use_hyperspectral;
    let common = write_spatial_lct_constant_restart(common)?;
    let pft = write_spatial_pft_constant_restart(config)?;
    Ok(SpatialPftConstantRestartFiles { common, pft })
}

fn pft_hydraulic_model(namelist: &Path) -> Result<HydraulicModel> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", namelist.display()))?;
    match document.get("DEF_USE_Campbell_SOIL_MODEL") {
        Some(Value::Bool(true)) => Ok(HydraulicModel::Campbell),
        Some(Value::Bool(false)) | None => Ok(HydraulicModel::VanGenuchten),
        Some(_) => bail!("DEF_USE_Campbell_SOIL_MODEL must be a logical value"),
    }
}

fn read_i32(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
) -> Result<Vec<i32>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    values_i32(&file, variable)
}

fn read_f64(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
) -> Result<Vec<f64>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    values_f64(&file, variable)
}

#[cfg(test)]
#[path = "spatial_pft_tests.rs"]
mod spatial_pft_tests;
