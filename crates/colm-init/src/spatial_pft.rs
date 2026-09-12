//! Spatial PFT constant-restart adapter for Rust `mksrfdata` block artifacts.
//!
//! The common land-patch restart is written by [`crate::spatial_static`].
//! This module writes its separate `landpft` companion, exactly as CoLM's
//! `WRITE_PFTimeInvariants` does after `pct_readin` and `HTOP_readin`.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_namelist::parse;

use crate::single_point::pft_canopy;
use crate::spatial_static::{block_path, values_f64, values_i32};
use crate::{write_pft_constant_restart, PftConstantRestartInput};

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
