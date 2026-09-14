//! Cold-start restart output for CoLM's `GridRiverLakeFlow` kernel branch.
//!
//! This is the base state written by `MOD_Initialize.F90` and
//! `MOD_Grid_RiverLakeTimeVars.F90`: channel depth starts from `topo_rivhgt`,
//! velocity and pending runoff start at zero, and the restart carries the
//! unit-catchment identity used to reject a mismatched later restart.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use netcdf::types::{FloatType, IntType, NcVariableType};

use crate::RestartDate;

const RESTART_SCHEMA_VERSION: i32 = 2;
const UCATCH_IDENTITY_VERSION: f64 = 1.0;

/// Inputs needed for the base gridded river/lake cold restart.
#[derive(Debug, Clone, Copy)]
pub struct GridRiverColdStartConfig<'a> {
    pub unit_catchment: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    pub date: RestartDate,
    pub bifurcation: bool,
    pub levee: bool,
    pub tracer: bool,
    pub reservoir_method: i32,
}

/// The base grid-river restart produced by [`write_gridriver_cold_restart`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRiverColdStartFile {
    pub path: PathBuf,
}

/// Write the schema-v2 GridRiverLake cold state consumed by a matching CoLM kernel.
///
/// Bifurcation, levee, tracer, and reservoir modes own additional restart payloads
/// upstream; refusing them is safer than emitting a transaction that claims them
/// disabled or complete.
pub fn write_gridriver_cold_restart(
    config: GridRiverColdStartConfig<'_>,
) -> Result<GridRiverColdStartFile> {
    ensure!(
        !config.case_name.is_empty()
            && !config.case_name.contains('/')
            && !config.case_name.contains('\\'),
        "GridRiverLake restart case name must be a filename component"
    );
    ensure!(
        (0..=9999).contains(&config.land_cover_year) && (0..=9999).contains(&config.date.year),
        "GridRiverLake restart years must fit CoLM's four-digit filename convention"
    );
    ensure!(
        (1..=366).contains(&config.date.julian_day) && config.date.seconds < 86_400,
        "GridRiverLake restart date is invalid"
    );
    ensure!(
        !config.bifurcation && !config.levee && !config.tracer && config.reservoir_method == 0,
        "Rust GridRiverLake cold restart currently supports only the base routing state; bifurcation, levee, tracer, and reservoir modes require their native restart payloads"
    );

    let source = netcdf::open(config.unit_catchment).with_context(|| {
        format!(
            "cannot open GridRiverLake unit-catchment file {}",
            config.unit_catchment.display()
        )
    })?;
    let x = read_i32(&source, "seq_x")?;
    let y = read_i32(&source, "seq_y")?;
    let next = read_i32(&source, "seq_next")?;
    let channel_depth = read_f64(&source, "topo_rivhgt")?;
    let count = x.len();
    ensure!(
        count > 0,
        "GridRiverLake unit-catchment file has no catchments"
    );
    ensure!(
        y.len() == count && next.len() == count && channel_depth.len() == count,
        "GridRiverLake unit-catchment vectors must have the same length"
    );
    ensure!(
        x.iter().all(|value| *value > 0) && y.iter().all(|value| *value > 0),
        "GridRiverLake seq_x and seq_y must be positive"
    );
    ensure!(
        channel_depth
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "GridRiverLake topo_rivhgt must contain finite non-negative depths"
    );

    let date = format!(
        "{:04}-{:03}-{:05}",
        config.date.year, config.date.julian_day, config.date.seconds
    );
    let directory = config.restart_dir.join(&date);
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "cannot create GridRiverLake restart directory {}",
            directory.display()
        )
    })?;
    let path = directory.join(format!(
        "{}_restart_gridriver_{date}_lc{:04}.nc",
        config.case_name, config.land_cover_year
    ));

    let mut file = netcdf::create(&path)
        .with_context(|| format!("cannot create GridRiverLake restart {}", path.display()))?;
    file.add_dimension("ucatch", count)?;
    file.add_dimension("gridriver_ucatch_identity_field", 4)?;
    put_i32(
        &mut file,
        "gridriver_restart_schema",
        &[],
        &[RESTART_SCHEMA_VERSION],
    )?;
    put_i32(&mut file, "gridriver_restart_complete", &[], &[0])?;
    put_i32(
        &mut file,
        "gridriver_restart_feature_bifurcation",
        &[],
        &[0],
    )?;
    put_i32(&mut file, "gridriver_restart_feature_levee", &[], &[0])?;
    let mut identity = Vec::with_capacity(4 * count);
    identity.extend(std::iter::repeat_n(UCATCH_IDENTITY_VERSION, count));
    identity.extend(x.iter().map(|value| f64::from(*value)));
    identity.extend(y.iter().map(|value| f64::from(*value)));
    identity.extend(next.iter().map(|value| f64::from(*value)));
    put_f64(
        &mut file,
        "gridriver_ucatch_identity",
        &["gridriver_ucatch_identity_field", "ucatch"],
        &identity,
    )?;
    let zeros = vec![0.0; count];
    put_f64(&mut file, "wdsrf_ucat", &["ucatch"], &channel_depth)?;
    put_f64(&mut file, "veloc_riv", &["ucatch"], &zeros)?;
    put_f64(&mut file, "acctime_rnof", &[], &[0.0])?;
    put_f64(&mut file, "acc_rnof_uc", &["ucatch"], &zeros)?;
    put_f64(&mut file, "volwater_ucat", &["ucatch"], &zeros)?;
    // MOD_Grid_RiverLakeHist flushes these vectors to zero before mkinidata
    // writes the cold restart.  Preserve the concrete fields rather than
    // relying on the native optional-read fallback.
    for name in [
        "hist_acctime_ucat",
        "hist_wdsrf_ucat",
        "hist_veloc_riv",
        "hist_discharge",
        "hist_floodarea",
        "hist_rivsto",
        "hist_fldsto",
        "hist_flddph",
        "hist_storge",
        "hist_sfcelv",
    ] {
        put_f64(&mut file, name, &["ucatch"], &zeros)?;
    }
    file.variable_mut("gridriver_restart_complete")
        .expect("the GridRiverLake completion marker was just written")
        .put_values(&[1], ..)?;
    file.close()
        .with_context(|| format!("cannot close GridRiverLake restart {}", path.display()))?;
    Ok(GridRiverColdStartFile { path })
}

fn read_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("GridRiverLake unit-catchment file is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Int(IntType::I32) => Ok(variable.get_values::<i32, _>(..)?),
        kind => bail!("GridRiverLake {name} must be int32, got {kind:?}"),
    }
}

fn read_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("GridRiverLake unit-catchment file is missing {name}"))?;
    match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => Ok(variable.get_values::<f64, _>(..)?),
        NcVariableType::Float(FloatType::F32) => Ok(variable
            .get_values::<f32, _>(..)?
            .into_iter()
            .map(f64::from)
            .collect()),
        kind => bail!("GridRiverLake {name} must be floating-point, got {kind:?}"),
    }
}

fn put_i32(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[i32],
) -> Result<()> {
    file.add_variable::<i32>(name, dimensions)?
        .put_values(values, ..)?;
    Ok(())
}

fn put_f64(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[f64],
) -> Result<()> {
    file.add_variable::<f64>(name, dimensions)?
        .put_values(values, ..)?;
    Ok(())
}

#[cfg(test)]
#[path = "gridriver_tests.rs"]
mod gridriver_tests;
