//! One native single-point preprocessing call shared by Rust executables.
//!
//! `mksrfdata` owns completion of `srfdata.nc`; `mkinidata` owns the restart
//! state. Keeping the hand-off here means a future Rust `colm` driver does not
//! need to duplicate their namelist/path contract.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Result};
use colm_srfdata::{
    materialize_single_point_surface_from_namelist, SinglePointSurfaceRun, SiteMode,
};

use crate::{
    single_point_cold_start_run_from_namelist, write_single_point_cold_time_restarts,
    write_single_point_constant_restarts, SinglePointColdStartRun, SinglePointConstantRestartFiles,
    SinglePointTimeRestartFiles,
};

/// Resolved stages of one native single-point preprocessing run.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointPreprocessRun {
    pub surface: SinglePointSurfaceRun,
    pub cold_start: SinglePointColdStartRun,
}

/// Files written by [`prepare_single_point_case`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinglePointPreprocessFiles {
    pub surface: PathBuf,
    pub constants: SinglePointConstantRestartFiles,
    pub time: SinglePointTimeRestartFiles,
}

/// Materializes single-point landdata, then initializes its compatible restart.
///
/// This is the native Rust equivalent of the Desktop preprocessing sequence
/// `mksrfdata → mkinidata`. Spatial cases retain their block-addressed inputs
/// and outputs, and therefore their dedicated drivers.
pub fn prepare_single_point_case(
    namelist: impl AsRef<Path>,
    lct_mode_override: Option<SiteMode>,
    crop_enabled: bool,
    observation: Option<&Path>,
) -> Result<(SinglePointPreprocessRun, SinglePointPreprocessFiles)> {
    let namelist = namelist.as_ref();
    let (surface, _) = materialize_single_point_surface_from_namelist(
        namelist,
        lct_mode_override,
        crop_enabled,
        observation,
    )?;
    let cold_start = single_point_cold_start_run_from_namelist(namelist, None, None)?;
    let surface_file = surface.landdata_dir.join("srfdata.nc");
    ensure!(
        cold_start.static_run.surface == surface_file,
        "mksrfdata wrote {}, but mkinidata resolves {}",
        surface_file.display(),
        cold_start.static_run.surface.display()
    );
    let constants = write_single_point_constant_restarts(&cold_start)?;
    let time = write_single_point_cold_time_restarts(&cold_start)?;
    Ok((
        SinglePointPreprocessRun {
            surface,
            cold_start,
        },
        SinglePointPreprocessFiles {
            surface: surface_file,
            constants,
            time,
        },
    ))
}
