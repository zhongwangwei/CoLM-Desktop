//! Cold-start restart output for CoLM's `GridRiverLakeFlow` kernel branch.
//!
//! This is the base state written by `MOD_Initialize.F90` and
//! `MOD_Grid_RiverLakeTimeVars.F90`: channel depth starts from `topo_rivhgt`,
//! velocity and pending runoff start at zero, and the restart carries the
//! unit-catchment identity used to reject a mismatched later restart.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use netcdf::types::{FloatType, IntType, NcVariableType};

use crate::{restart::validate_restart_compression, RestartDate};

const RESTART_SCHEMA_VERSION: i32 = 2;
const UCATCH_IDENTITY_VERSION: f64 = 1.0;
const BIFURCATION_SIGNATURE_VERSION: f64 = 1.0;
const RESERVOIR_IDENTITY_VERSION: f64 = 1.0;
const MISSING_RESERVOIR_VOLUME: f64 = -1.0e36;

struct BifurcationColdState {
    pathways: usize,
    levels: usize,
    signature: Vec<f64>,
}

impl BifurcationColdState {
    fn active(&self) -> bool {
        self.pathways > 0 && self.levels > 0
    }
}

struct ReservoirColdState {
    identity: Vec<f64>,
    volume: Vec<f64>,
}

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
    pub reservoir_method: i32,
    pub reservoir_parameters: Option<&'a Path>,
    pub compression_level: u8,
}

/// The base grid-river restart produced by [`write_gridriver_cold_restart`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRiverColdStartFile {
    pub path: PathBuf,
}

/// Write the schema-v2 GridRiverLake cold state consumed by a matching CoLM kernel.
///
/// Native `mkinidata` does not initialize route tracers, so its cold file has
/// no tracer transaction. The runtime detects that absence and seeds tracer
/// state from water. Bifurcation, levee, and reservoir state cold-start to
/// their native values here.
pub fn write_gridriver_cold_restart(
    config: GridRiverColdStartConfig<'_>,
) -> Result<GridRiverColdStartFile> {
    validate_restart_compression(config.compression_level)?;
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
        matches!(config.reservoir_method, 0 | 1),
        "Rust GridRiverLake cold restart currently supports base routing, levee, bifurcation, and reservoir method 1"
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
    let bifurcation = config
        .bifurcation
        .then(|| read_bifurcation_cold_state(&source, count))
        .transpose()?;
    let reservoir = (config.reservoir_method == 1)
        .then(|| {
            read_reservoir_cold_state(
                config
                    .reservoir_parameters
                    .context("GridRiverLake reservoir method 1 needs DEF_ReservoirPara_file")?,
                count,
                config.date.year,
            )
        })
        .transpose()?;

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
        &[i32::from(config.bifurcation)],
    )?;
    put_i32(
        &mut file,
        "gridriver_restart_feature_levee",
        &[],
        &[i32::from(config.levee)],
    )?;
    // `ncio_write_serial` receives `(field, ucatch)` from Fortran, which
    // stores as `(ucatch, field)` in NetCDF. Keep the same object-major
    // layout used by the other Rust restart writers.
    let mut identity = Vec::with_capacity(4 * count);
    for index in 0..count {
        identity.extend([
            UCATCH_IDENTITY_VERSION,
            f64::from(x[index]),
            f64::from(y[index]),
            f64::from(next[index]),
        ]);
    }
    put_f64_compressed(
        &mut file,
        "gridriver_ucatch_identity",
        &["ucatch", "gridriver_ucatch_identity_field"],
        &identity,
        config.compression_level,
    )?;
    let zeros = vec![0.0; count];
    put_f64_compressed(
        &mut file,
        "wdsrf_ucat",
        &["ucatch"],
        &channel_depth,
        config.compression_level,
    )?;
    put_f64_compressed(
        &mut file,
        "veloc_riv",
        &["ucatch"],
        &zeros,
        config.compression_level,
    )?;
    put_f64(&mut file, "acctime_rnof", &[], &[0.0])?;
    put_f64_compressed(
        &mut file,
        "acc_rnof_uc",
        &["ucatch"],
        &zeros,
        config.compression_level,
    )?;
    put_f64_compressed(
        &mut file,
        "volwater_ucat",
        &["ucatch"],
        &zeros,
        config.compression_level,
    )?;
    if let Some(bifurcation) = &bifurcation {
        put_f64_compressed(
            &mut file,
            "hist_bifout",
            &["ucatch"],
            &zeros,
            config.compression_level,
        )?;
        if bifurcation.active() {
            put_f64_compressed(
                &mut file,
                "wdsrf_ucat_prev",
                &["ucatch"],
                &channel_depth,
                config.compression_level,
            )?;
            write_bifurcation_cold_state(&mut file, bifurcation, config.compression_level)?;
        }
    }
    if config.levee {
        put_f64_compressed(
            &mut file,
            "levsto",
            &["ucatch"],
            &zeros,
            config.compression_level,
        )?;
    }
    if let Some(reservoir) = &reservoir {
        write_reservoir_cold_state(&mut file, reservoir, config.compression_level)?;
    }
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
        put_f64_compressed(
            &mut file,
            name,
            &["ucatch"],
            &zeros,
            config.compression_level,
        )?;
    }
    if config.levee {
        put_f64_compressed(
            &mut file,
            "hist_levsto",
            &["ucatch"],
            &zeros,
            config.compression_level,
        )?;
        put_f64_compressed(
            &mut file,
            "hist_levdph",
            &["ucatch"],
            &zeros,
            config.compression_level,
        )?;
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
    ensure!(
        variable.dimensions().len() == 1,
        "GridRiverLake {name} must be one-dimensional"
    );
    match variable.vartype() {
        NcVariableType::Int(IntType::I32) => Ok(variable.get_values::<i32, _>(..)?),
        kind => bail!("GridRiverLake {name} must be int32, got {kind:?}"),
    }
}

fn read_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("GridRiverLake unit-catchment file is missing {name}"))?;
    ensure!(
        variable.dimensions().len() == 1,
        "GridRiverLake {name} must be one-dimensional"
    );
    read_f64_values(&variable, name)
}

fn read_matrix_f64(
    file: &netcdf::File,
    name: &str,
    rows: usize,
    columns: usize,
) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("GridRiverLake unit-catchment file is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2 && dimensions[0].len() == rows && dimensions[1].len() == columns,
        "GridRiverLake {name} must use native (bifurcation_pathway, bifurcation_level) dimensions"
    );
    read_f64_values(&variable, name)
}

fn read_f64_values(variable: &netcdf::Variable<'_>, name: &str) -> Result<Vec<f64>> {
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

fn read_bifurcation_cold_state(
    file: &netcdf::File,
    catchments: usize,
) -> Result<BifurcationColdState> {
    let upstream = read_i32(file, "bifurcation_upst")?;
    let pathways = upstream.len();
    let downstream = read_i32(file, "bifurcation_down")?;
    let distance = read_f64(file, "bifurcation_distance")?;
    let manning = read_f64(file, "bifurcation_manning")?;
    let levels = manning.len();
    let elevation = read_matrix_f64(file, "bifurcation_elevation", pathways, levels)?;
    let width = read_matrix_f64(file, "bifurcation_width", pathways, levels)?;
    ensure!(
        downstream.len() == pathways && distance.len() == pathways,
        "GridRiverLake bifurcation pathway vectors must have the same length"
    );
    ensure!(
        distance
            .iter()
            .all(|value| value.is_finite() && *value > 0.0),
        "GridRiverLake bifurcation distance must be finite and positive"
    );
    ensure!(
        elevation.iter().all(|value| value.is_finite()),
        "GridRiverLake bifurcation elevation must be finite"
    );
    ensure!(
        width.iter().all(|value| value.is_finite() && *value >= 0.0),
        "GridRiverLake bifurcation width must be finite and non-negative"
    );
    ensure!(
        manning.iter().all(|value| value.is_finite()),
        "GridRiverLake bifurcation Manning coefficient must be finite"
    );
    for path in 0..pathways {
        ensure!(
            (1..=catchments as i32).contains(&upstream[path])
                && downstream[path] <= catchments as i32
                && downstream[path] != upstream[path],
            "GridRiverLake bifurcation path {} has invalid catchment endpoints",
            path + 1
        );
        let first = path * levels;
        ensure!(
            width[first..first + levels]
                .iter()
                .any(|value| *value > 0.0),
            "GridRiverLake bifurcation path {} has no active level",
            path + 1
        );
        for level in 0..levels {
            if width[first + level] > 0.0 {
                ensure!(
                    manning[level] > 0.0,
                    "GridRiverLake bifurcation active level requires positive Manning coefficient"
                );
            }
            if level > 0 && width[first + level] > 0.0 {
                ensure!(
                    width[first + level - 1] > 0.0
                        && elevation[first + level] >= elevation[first + level - 1],
                    "GridRiverLake bifurcation active levels must be contiguous with non-decreasing elevation"
                );
            }
        }
    }

    let mut signature = Vec::with_capacity(pathways * (4 + 3 * levels));
    for path in 0..pathways {
        let first = path * levels;
        signature.extend([
            BIFURCATION_SIGNATURE_VERSION,
            f64::from(upstream[path]),
            f64::from(downstream[path]),
            distance[path],
        ]);
        signature.extend_from_slice(&elevation[first..first + levels]);
        signature.extend_from_slice(&width[first..first + levels]);
        signature.extend_from_slice(&manning);
    }
    Ok(BifurcationColdState {
        pathways,
        levels,
        signature,
    })
}

fn write_bifurcation_cold_state(
    file: &mut netcdf::FileMut,
    state: &BifurcationColdState,
    compression_level: u8,
) -> Result<()> {
    if !state.active() {
        return Ok(());
    }
    file.add_dimension("bifurcation_signature_field", 4 + 3 * state.levels)?;
    file.add_dimension("bifurcation_level", state.levels)?;
    file.add_dimension("bifurcation_pathway", state.pathways)?;
    put_f64_compressed(
        file,
        "bif_path_signature",
        &["bifurcation_pathway", "bifurcation_signature_field"],
        &state.signature,
        compression_level,
    )?;
    let zeros = vec![0.0; state.pathways * state.levels];
    for name in ["pth_veloc", "pth_momen"] {
        put_f64_compressed(
            file,
            name,
            &["bifurcation_pathway", "bifurcation_level"],
            &zeros,
            compression_level,
        )?;
    }
    Ok(())
}

fn read_reservoir_cold_state(
    parameters: &Path,
    catchments: usize,
    start_year: i32,
) -> Result<ReservoirColdState> {
    let file = netcdf::open(parameters).with_context(|| {
        format!(
            "cannot open GridRiverLake reservoir parameter file {}",
            parameters.display()
        )
    })?;
    let grand_id = read_i32(&file, "dam_GRAND_ID")?;
    let sequence = read_i32(&file, "dam_seq")?;
    let build_year = read_i32(&file, "dam_year")?;
    let total_volume = read_f64(&file, "dam_TotalVol_mcm")?;
    let conservation_volume = read_f64(&file, "dam_ConVol_mcm")?;
    let normal_outflow = read_f64(&file, "dam_Qn")?;
    let flood_outflow = read_f64(&file, "dam_Qf")?;
    let rows = sequence.len();
    ensure!(
        grand_id.len() == rows
            && build_year.len() == rows
            && total_volume.len() == rows
            && conservation_volume.len() == rows
            && normal_outflow.len() == rows
            && flood_outflow.len() == rows,
        "GridRiverLake reservoir parameter vectors must have the same length"
    );
    let mut sorted_sequence = sequence.clone();
    sorted_sequence.sort_unstable();
    ensure!(
        sorted_sequence.windows(2).all(|pair| pair[0] != pair[1]),
        "GridRiverLake reservoir parameter file has duplicate dam_seq entries"
    );

    let mut identity = Vec::new();
    let mut volume = Vec::new();
    for index in 0..rows {
        if !(1..=catchments as i32).contains(&sequence[index]) {
            continue;
        }
        let total = total_volume[index] * 1.0e6;
        let conservation = conservation_volume[index] * 1.0e6;
        ensure!(
            total.is_finite()
                && total > 0.0
                && conservation.is_finite()
                && conservation > 0.0
                && normal_outflow[index].is_finite()
                && normal_outflow[index] >= 0.0
                && flood_outflow[index].is_finite()
                && flood_outflow[index] >= 0.0,
            "GridRiverLake reservoir parameters are invalid for active dam sequence {}",
            sequence[index]
        );
        identity.extend([RESERVOIR_IDENTITY_VERSION, f64::from(sequence[index])]);
        volume.push(if start_year >= build_year[index] {
            (0.7 * total).min(conservation)
        } else {
            MISSING_RESERVOIR_VOLUME
        });
    }
    Ok(ReservoirColdState { identity, volume })
}

fn write_reservoir_cold_state(
    file: &mut netcdf::FileMut,
    state: &ReservoirColdState,
    compression_level: u8,
) -> Result<()> {
    if state.volume.is_empty() {
        return Ok(());
    }
    let reservoirs = state.volume.len();
    file.add_dimension("reservoir", reservoirs)?;
    file.add_dimension("gridriver_reservoir_identity_field", 2)?;
    put_f64_compressed(
        file,
        "gridriver_reservoir_identity",
        &["reservoir", "gridriver_reservoir_identity_field"],
        &state.identity,
        compression_level,
    )?;
    put_f64_compressed(
        file,
        "volresv",
        &["reservoir"],
        &state.volume,
        compression_level,
    )?;
    let zeros = vec![0.0; reservoirs];
    for name in [
        "hist_acctime_resv",
        "hist_volresv",
        "hist_qresv_in",
        "hist_qresv_out",
    ] {
        put_f64_compressed(file, name, &["reservoir"], &zeros, compression_level)?;
    }
    Ok(())
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

fn put_f64_compressed(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[f64],
    compression_level: u8,
) -> Result<()> {
    let mut variable = file.add_variable::<f64>(name, dimensions)?;
    variable.set_compression(compression_level.into(), false)?;
    variable.put_values(values, ..)?;
    Ok(())
}

#[cfg(test)]
#[path = "gridriver_tests.rs"]
mod gridriver_tests;
