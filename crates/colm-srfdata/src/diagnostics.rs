//! Gridded `DEF_USE_SrfdataDiag` outputs for the spatial surface builder.
//!
//! These files are diagnostic-only: CoLM never reads them during a run.  They
//! nevertheless use the same area-weighted pixelset-to-grid reduction as the
//! upstream `MOD_SrfdataDiag`, so a requested diagnostic is evidence about the
//! surface that Rust actually wrote.

use std::{collections::BTreeMap, path::Path};

use anyhow::{ensure, Context, Result};

use crate::{FlatLandPatches, SpatialGrid, SpatialInputKind, SpatialTopology};

pub const DIAGNOSTIC_MISSING: f64 = -1.0e36;

#[cfg(not(test))]
const MAX_DENSE_DIAGNOSTIC_VALUES: usize = 1_000_000;
#[cfg(test)]
const MAX_DENSE_DIAGNOSTIC_VALUES: usize = 4;

/// How an upstream `srfdata_map_and_write` call reduces a pixelset field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticStatistic {
    Mean,
    Fraction,
}

/// A complete in-memory equivalent of one upstream diagnostic field.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedDiagnostic {
    /// Values in `type, latitude, longitude` order.  A one-type result omits
    /// `TypeIndex` on disk, but retains this shape here.
    pub values: Vec<f64>,
    /// The upstream `*_grid` companion: the all-type area-weighted mean.
    pub grid: Vec<f64>,
    pub longitude: Vec<f64>,
    pub latitude: Vec<f64>,
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
    pub ntypes: usize,
    sparse: Option<BTreeMap<usize, SparseDiagnosticCell>>,
    fill_value: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct SparseDiagnosticCell {
    values: Vec<f64>,
    grid: f64,
}

#[derive(Debug)]
struct SparseAccumulator {
    numerator: Vec<f64>,
    weight: Vec<f64>,
    grid_numerator: f64,
    grid_weight: f64,
}

/// Map a patch vector to CoLM's diagnostic grid.
///
/// GRIDBASED cases retain their mesh grid.  UNSTRUCTURED and CATCHMENT cases
/// deliberately follow upstream and use the fixed 0.1-degree global grid.
/// `pctshared` is the pixelset share used by CROP PFT/patch outputs; ordinary
/// non-shared sets pass `None`.
#[allow(clippy::too_many_arguments)]
pub fn map_patch_diagnostic(
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    values: &[f64],
    type_indices: &[i32],
    statistic: DiagnosticStatistic,
    pctshared: Option<&[f64]>,
    missing: f64,
    default: Option<f64>,
) -> Result<MappedDiagnostic> {
    ensure!(
        values.len() == patches.len(),
        "diagnostic field has {} values for {} patches",
        values.len(),
        patches.len()
    );
    ensure!(
        !type_indices.is_empty(),
        "diagnostic field needs at least one TypeIndex"
    );
    ensure!(
        type_indices.windows(2).all(|pair| pair[0] < pair[1]),
        "diagnostic TypeIndex values must be strictly increasing"
    );
    if let Some(shares) = pctshared {
        ensure!(
            shares.len() == patches.len()
                && shares
                    .iter()
                    .all(|share| share.is_finite() && *share >= 0.0),
            "diagnostic pctshared must be finite, non-negative, and match patches"
        );
    }

    let target = diagnostic_grid(topology);
    let nlon = target.lon_w.len();
    let nlat = target.lat_s.len();
    let ncell = nlon
        .checked_mul(nlat)
        .context("diagnostic grid cell count overflows usize")?;
    let ntypes = type_indices.len();
    let size = ntypes
        .checked_mul(ncell)
        .context("diagnostic type-grid cell count overflows usize")?;
    if size > MAX_DENSE_DIAGNOSTIC_VALUES {
        return map_patch_diagnostic_sparse(
            topology,
            patches,
            values,
            type_indices,
            statistic,
            pctshared,
            missing,
            default,
            target,
        );
    }
    let mut numerator = vec![0.0; size];
    let mut weight = vec![0.0; size];
    let mut grid_numerator = vec![0.0; ncell];
    let mut grid_weight = vec![0.0; ncell];

    for patch in 0..patches.len() {
        let value = values[patch];
        if value == missing {
            continue;
        }
        ensure!(
            value.is_finite(),
            "diagnostic field has a non-finite value at patch {patch}"
        );
        let type_slot = type_indices
            .binary_search(&patches.set_type[patch])
            .map_err(|_| {
                anyhow::anyhow!(
                    "patch {patch} type {} is absent from diagnostic TypeIndex",
                    patches.set_type[patch]
                )
            })?;
        let element = patches.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("patch {patch} has zero element index"))?;
        ensure!(
            topology.mesh.element_id(element)? == patches.element_ids[patch],
            "patch {patch} does not match its mesh element"
        );
        let (xs, ys) = topology.mesh.pixels(element)?;
        let range = patches.owned_pixel_range(patch, xs.len())?;
        let range = if range.is_empty() {
            // Upstream `build_arealweighted` treats a 2 m WMO virtual patch
            // (`ipxstt=ipxend=-1`) as covering the whole mesh element for
            // diagnostics.  The scientific aggregators still skip/copy those
            // virtual rows separately, but the diagnostic denominator must
            // include their whole-element geometry.
            0..xs.len()
        } else {
            range
        };
        let start = range.start;
        let end = range.end;
        ensure!(
            start < end && end <= xs.len() && xs.len() == ys.len(),
            "patch {patch} has an invalid mesh-pixel range"
        );
        let share = pctshared.map_or(1.0, |shares| shares[patch]);
        for cell in start..end {
            let x = usize::try_from(xs[cell])
                .context("diagnostic mesh longitude is negative")?
                .checked_sub(1)
                .context("diagnostic mesh longitude is zero")?;
            let y = usize::try_from(ys[cell])
                .context("diagnostic mesh latitude is negative")?
                .checked_sub(1)
                .context("diagnostic mesh latitude is zero")?;
            let lon = midpoint_longitude(
                *topology
                    .pixel
                    .lon_w
                    .get(x)
                    .context("diagnostic mesh longitude is outside pixel axes")?,
                *topology
                    .pixel
                    .lon_e
                    .get(x)
                    .context("diagnostic mesh longitude is outside pixel axes")?,
            );
            let lat = midpoint_latitude(
                *topology
                    .pixel
                    .lat_s
                    .get(y)
                    .context("diagnostic mesh latitude is outside pixel axes")?,
                *topology
                    .pixel
                    .lat_n
                    .get(y)
                    .context("diagnostic mesh latitude is outside pixel axes")?,
            );
            let target_x = longitude_index(lon, &target)?;
            let target_y = latitude_index(lat, &target)?;
            let target_cell = target_y * nlon + target_x;
            let area = pixel_area(
                topology.pixel.lon_w[x],
                topology.pixel.lon_e[x],
                topology.pixel.lat_s[y],
                topology.pixel.lat_n[y],
            )? * share;
            if area == 0.0 {
                continue;
            }
            let index = type_slot * ncell + target_cell;
            match statistic {
                DiagnosticStatistic::Mean => {
                    numerator[index] += area * value;
                    weight[index] += area;
                    grid_numerator[target_cell] += area * value;
                    grid_weight[target_cell] += area;
                }
                DiagnosticStatistic::Fraction => {
                    // `MOD_SrfdataDiag` normalizes the mapped pixelset weights,
                    // not the already-normalized vector value.  The vector is
                    // still consulted above for its upstream missing sentinel.
                    weight[index] += area;
                    grid_weight[target_cell] += area;
                }
            }
        }
    }

    let mut mapped = vec![missing; size];
    let mut mapped_grid = vec![missing; ncell];
    match statistic {
        DiagnosticStatistic::Mean => {
            for index in 0..size {
                if weight[index] > 0.0 {
                    mapped[index] = numerator[index] / weight[index];
                }
            }
            for index in 0..ncell {
                if grid_weight[index] > 0.0 {
                    mapped_grid[index] = grid_numerator[index] / grid_weight[index];
                }
            }
        }
        DiagnosticStatistic::Fraction => {
            for target_cell in 0..ncell {
                let total = grid_weight[target_cell];
                if total > 0.0 {
                    for type_slot in 0..ntypes {
                        mapped[type_slot * ncell + target_cell] =
                            weight[type_slot * ncell + target_cell] / total;
                    }
                    mapped_grid[target_cell] = 1.0;
                }
            }
        }
    }
    if let Some(default) = default {
        for value in &mut mapped {
            if *value == missing {
                *value = default;
            }
        }
        for value in &mut mapped_grid {
            if *value == missing {
                *value = default;
            }
        }
    }

    Ok(MappedDiagnostic {
        values: mapped,
        grid: mapped_grid,
        longitude: target
            .lon_w
            .iter()
            .zip(&target.lon_e)
            .map(|(&west, &east)| midpoint_longitude(west, east))
            .collect(),
        latitude: target
            .lat_s
            .iter()
            .zip(&target.lat_n)
            .map(|(&south, &north)| midpoint_latitude(south, north))
            .collect(),
        lon_w: target.lon_w,
        lon_e: target.lon_e,
        lat_s: target.lat_s,
        lat_n: target.lat_n,
        ntypes,
        sparse: None,
        fill_value: default.unwrap_or(missing),
    })
}

#[allow(clippy::too_many_arguments)]
fn map_patch_diagnostic_sparse(
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    values: &[f64],
    type_indices: &[i32],
    statistic: DiagnosticStatistic,
    pctshared: Option<&[f64]>,
    missing: f64,
    default: Option<f64>,
    target: SpatialGrid,
) -> Result<MappedDiagnostic> {
    let nlon = target.lon_w.len();
    let ntypes = type_indices.len();
    let mut cells = BTreeMap::<usize, SparseAccumulator>::new();
    for patch in 0..patches.len() {
        let value = values[patch];
        if value == missing {
            continue;
        }
        ensure!(
            value.is_finite(),
            "diagnostic field has a non-finite value at patch {patch}"
        );
        let type_slot = type_indices
            .binary_search(&patches.set_type[patch])
            .map_err(|_| {
                anyhow::anyhow!(
                    "patch {patch} type {} is absent from diagnostic TypeIndex",
                    patches.set_type[patch]
                )
            })?;
        let element = patches.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("patch {patch} has zero element index"))?;
        ensure!(
            topology.mesh.element_id(element)? == patches.element_ids[patch],
            "patch {patch} does not match its mesh element"
        );
        let (xs, ys) = topology.mesh.pixels(element)?;
        let range = patches.owned_pixel_range(patch, xs.len())?;
        let range = if range.is_empty() {
            // Upstream `build_arealweighted` treats a 2 m WMO virtual patch
            // (`ipxstt=ipxend=-1`) as covering the whole mesh element for
            // diagnostics.  The scientific aggregators still skip/copy those
            // virtual rows separately, but the diagnostic denominator must
            // include their whole-element geometry.
            0..xs.len()
        } else {
            range
        };
        let start = range.start;
        let end = range.end;
        ensure!(
            start < end && end <= xs.len() && xs.len() == ys.len(),
            "patch {patch} has an invalid mesh-pixel range"
        );
        let share = pctshared.map_or(1.0, |shares| shares[patch]);
        for cell in start..end {
            let x = usize::try_from(xs[cell])
                .context("diagnostic mesh longitude is negative")?
                .checked_sub(1)
                .context("diagnostic mesh longitude is zero")?;
            let y = usize::try_from(ys[cell])
                .context("diagnostic mesh latitude is negative")?
                .checked_sub(1)
                .context("diagnostic mesh latitude is zero")?;
            let longitude = midpoint_longitude(
                *topology
                    .pixel
                    .lon_w
                    .get(x)
                    .context("diagnostic mesh longitude is outside pixel axes")?,
                *topology
                    .pixel
                    .lon_e
                    .get(x)
                    .context("diagnostic mesh longitude is outside pixel axes")?,
            );
            let latitude = midpoint_latitude(
                *topology
                    .pixel
                    .lat_s
                    .get(y)
                    .context("diagnostic mesh latitude is outside pixel axes")?,
                *topology
                    .pixel
                    .lat_n
                    .get(y)
                    .context("diagnostic mesh latitude is outside pixel axes")?,
            );
            let target_cell =
                latitude_index(latitude, &target)? * nlon + longitude_index(longitude, &target)?;
            let area = pixel_area(
                topology.pixel.lon_w[x],
                topology.pixel.lon_e[x],
                topology.pixel.lat_s[y],
                topology.pixel.lat_n[y],
            )? * share;
            if area == 0.0 {
                continue;
            }
            let accumulator = cells
                .entry(target_cell)
                .or_insert_with(|| SparseAccumulator {
                    numerator: vec![0.0; ntypes],
                    weight: vec![0.0; ntypes],
                    grid_numerator: 0.0,
                    grid_weight: 0.0,
                });
            match statistic {
                DiagnosticStatistic::Mean => {
                    accumulator.numerator[type_slot] += area * value;
                    accumulator.weight[type_slot] += area;
                    accumulator.grid_numerator += area * value;
                    accumulator.grid_weight += area;
                }
                DiagnosticStatistic::Fraction => {
                    accumulator.weight[type_slot] += area;
                    accumulator.grid_weight += area;
                }
            }
        }
    }
    let fill_value = default.unwrap_or(missing);
    let mut sparse = BTreeMap::new();
    for (cell, accumulator) in cells {
        let mut cell_values = vec![fill_value; ntypes];
        let mut grid = fill_value;
        match statistic {
            DiagnosticStatistic::Mean => {
                for (slot, value) in cell_values.iter_mut().enumerate() {
                    if accumulator.weight[slot] > 0.0 {
                        *value = accumulator.numerator[slot] / accumulator.weight[slot];
                    }
                }
                if accumulator.grid_weight > 0.0 {
                    grid = accumulator.grid_numerator / accumulator.grid_weight;
                }
            }
            DiagnosticStatistic::Fraction if accumulator.grid_weight > 0.0 => {
                for (slot, value) in cell_values.iter_mut().enumerate() {
                    *value = accumulator.weight[slot] / accumulator.grid_weight;
                }
                grid = 1.0;
            }
            DiagnosticStatistic::Fraction => {}
        }
        sparse.insert(
            cell,
            SparseDiagnosticCell {
                values: cell_values,
                grid,
            },
        );
    }
    Ok(MappedDiagnostic {
        values: Vec::new(),
        grid: Vec::new(),
        longitude: target
            .lon_w
            .iter()
            .zip(&target.lon_e)
            .map(|(&west, &east)| midpoint_longitude(west, east))
            .collect(),
        latitude: target
            .lat_s
            .iter()
            .zip(&target.lat_n)
            .map(|(&south, &north)| midpoint_latitude(south, north))
            .collect(),
        lon_w: target.lon_w,
        lon_e: target.lon_e,
        lat_s: target.lat_s,
        lat_n: target.lat_n,
        ntypes,
        sparse: Some(sparse),
        fill_value,
    })
}

fn mapped_shape_is_valid(mapped: &MappedDiagnostic) -> bool {
    mapped.sparse.is_some()
        || (mapped.values.len() == mapped.ntypes * mapped.lon_w.len() * mapped.lat_s.len()
            && mapped.grid.len() == mapped.lon_w.len() * mapped.lat_s.len())
}

fn write_sparse_static(
    variable: &mut netcdf::VariableMut<'_>,
    mapped: &MappedDiagnostic,
    type_axis: bool,
) -> Result<()> {
    let Some(cells) = &mapped.sparse else {
        return Ok(());
    };
    let nlon = mapped.lon_w.len();
    for (&cell, field) in cells {
        let (latitude, longitude) = (cell / nlon, cell % nlon);
        if type_axis {
            for (type_index, &value) in field.values.iter().enumerate() {
                variable.put_value(value, (type_index, latitude, longitude))?;
            }
        } else {
            variable.put_value(field.grid, (latitude, longitude))?;
        }
    }
    Ok(())
}

fn write_sparse_dimension(
    variable: &mut netcdf::VariableMut<'_>,
    mapped: &MappedDiagnostic,
    record: usize,
    type_axis: bool,
) -> Result<()> {
    let Some(cells) = &mapped.sparse else {
        return Ok(());
    };
    let nlon = mapped.lon_w.len();
    for (&cell, field) in cells {
        let (latitude, longitude) = (cell / nlon, cell % nlon);
        if type_axis {
            for (type_index, &value) in field.values.iter().enumerate() {
                variable.put_value(value, (record, type_index, latitude, longitude))?;
            }
        } else {
            variable.put_value(field.grid, (record, latitude, longitude))?;
        }
    }
    Ok(())
}

/// Append one static upstream-style diagnostic variable to `path`.
pub fn write_patch_diagnostic(
    path: impl AsRef<Path>,
    name: &str,
    type_indices: &[i32],
    mapped: &MappedDiagnostic,
    missing: f64,
) -> Result<()> {
    ensure!(!name.is_empty(), "diagnostic variable name cannot be empty");
    ensure!(
        mapped.ntypes == type_indices.len() && mapped_shape_is_valid(mapped),
        "diagnostic mapped field shape is inconsistent"
    );
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create diagnostic directory {}", parent.display()))?;
    }
    let create = !path.exists();
    let mut file = if create {
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?
    } else {
        netcdf::append(path).with_context(|| format!("cannot append {}", path.display()))?
    };
    if create {
        if mapped.ntypes > 1 {
            file.add_dimension("TypeIndex", mapped.ntypes)?;
            file.add_variable::<i32>("TypeIndex", &["TypeIndex"])?
                .put_values(type_indices, ..)?;
        }
        file.add_dimension("lon", mapped.lon_w.len())?;
        file.add_dimension("lat", mapped.lat_s.len())?;
        put_coordinate(
            &mut file,
            "lon",
            "lon",
            &mapped.longitude,
            "longitude",
            "degrees_east",
        )?;
        put_coordinate(
            &mut file,
            "lat",
            "lat",
            &mapped.latitude,
            "latitude",
            "degrees_north",
        )?;
        put_coordinate(
            &mut file,
            "lat_s",
            "lat",
            &mapped.lat_s,
            "southern latitude boundary",
            "degrees_north",
        )?;
        put_coordinate(
            &mut file,
            "lat_n",
            "lat",
            &mapped.lat_n,
            "northern latitude boundary",
            "degrees_north",
        )?;
        put_coordinate(
            &mut file,
            "lon_w",
            "lon",
            &mapped.lon_w,
            "western longitude boundary",
            "degrees_east",
        )?;
        put_coordinate(
            &mut file,
            "lon_e",
            "lon",
            &mapped.lon_e,
            "eastern longitude boundary",
            "degrees_east",
        )?;
    }
    ensure!(
        file.variable(name).is_none(),
        "diagnostic {} already has variable {name}",
        path.display()
    );
    if mapped.ntypes > 1 {
        let mut variable = file.add_variable::<f64>(name, &["TypeIndex", "lat", "lon"])?;
        if mapped.sparse.is_some() {
            variable.set_fill_value(mapped.fill_value)?;
            write_sparse_static(&mut variable, mapped, true)?;
        } else {
            variable.put_values(&mapped.values, (.., .., ..))?;
        }
        variable.put_attribute("missing_value", missing)?;
        let mut grid = file.add_variable::<f64>(&format!("{name}_grid"), &["lat", "lon"])?;
        if mapped.sparse.is_some() {
            grid.set_fill_value(mapped.fill_value)?;
            write_sparse_static(&mut grid, mapped, false)?;
        } else {
            grid.put_values(&mapped.grid, (.., ..))?;
        }
        grid.put_attribute("missing_value", missing)?;
    } else {
        let mut variable = file.add_variable::<f64>(name, &["lat", "lon"])?;
        if mapped.sparse.is_some() {
            variable.set_fill_value(mapped.fill_value)?;
            write_sparse_static(&mut variable, mapped, false)?;
        } else {
            variable.put_values(&mapped.values, (.., ..))?;
        }
        variable.put_attribute("missing_value", missing)?;
    }
    file.close()?;
    Ok(())
}

/// Write all records of an upstream `lastdimname = 'Itime'` diagnostic at
/// once.  Surface materialization already holds the twelve monthly vectors in
/// memory, so this avoids repeated NetCDF header rewrites while preserving the
/// upstream time-indexed diagnostic contract.
#[allow(clippy::too_many_arguments)]
pub fn write_patch_diagnostic_time(
    path: impl AsRef<Path>,
    name: &str,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    frames: &[Vec<f64>],
    type_indices: &[i32],
    statistic: DiagnosticStatistic,
    pctshared: Option<&[f64]>,
    missing: f64,
    default: Option<f64>,
) -> Result<()> {
    let records = (1..=i32::try_from(frames.len())?).collect::<Vec<_>>();
    write_patch_diagnostic_dimension(
        path,
        name,
        topology,
        patches,
        frames,
        type_indices,
        statistic,
        pctshared,
        missing,
        default,
        "Itime",
        &records,
    )
}

/// Write diagnostic frames along the upstream `lastdimname` axis.
///
/// This is the shared implementation for monthly fields (`Itime`) and fixed
/// physical layers (`ulev` or `source_patch`).  Multiple variables may share
/// one file and one axis, matching repeated `srfdata_map_and_write` calls.
#[allow(clippy::too_many_arguments)]
pub fn write_patch_diagnostic_dimension(
    path: impl AsRef<Path>,
    name: &str,
    topology: &SpatialTopology,
    patches: &FlatLandPatches,
    frames: &[Vec<f64>],
    type_indices: &[i32],
    statistic: DiagnosticStatistic,
    pctshared: Option<&[f64]>,
    missing: f64,
    default: Option<f64>,
    dimension: &str,
    dimension_values: &[i32],
) -> Result<()> {
    ensure!(!name.is_empty(), "diagnostic variable name cannot be empty");
    ensure!(
        !frames.is_empty(),
        "diagnostic dimension needs at least one frame"
    );
    ensure!(
        !dimension.is_empty() && !dimension.contains('/'),
        "diagnostic dimension must be one NetCDF name component"
    );
    ensure!(
        frames.len() == dimension_values.len(),
        "diagnostic dimension {dimension} has {} frames but {} values",
        frames.len(),
        dimension_values.len()
    );
    let mapped = frames
        .iter()
        .map(|values| {
            map_patch_diagnostic(
                topology,
                patches,
                values,
                type_indices,
                statistic,
                pctshared,
                missing,
                default,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let first = mapped.first().expect("nonempty diagnostic frames");
    ensure!(
        mapped.iter().all(|frame| {
            frame.ntypes == first.ntypes
                && frame.lon_w == first.lon_w
                && frame.lat_s == first.lat_s
                && frame.sparse.is_some() == first.sparse.is_some()
        }),
        "time diagnostic frames do not share one grid"
    );
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create diagnostic directory {}", parent.display()))?;
    }
    let create = !path.exists();
    let mut file = if create {
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?
    } else {
        netcdf::append(path).with_context(|| format!("cannot append {}", path.display()))?
    };
    if create {
        if first.ntypes > 1 {
            file.add_dimension("TypeIndex", first.ntypes)?;
            file.add_variable::<i32>("TypeIndex", &["TypeIndex"])?
                .put_values(type_indices, ..)?;
        }
        file.add_dimension("lon", first.lon_w.len())?;
        file.add_dimension("lat", first.lat_s.len())?;
        put_coordinate(
            &mut file,
            "lon",
            "lon",
            &first.longitude,
            "longitude",
            "degrees_east",
        )?;
        put_coordinate(
            &mut file,
            "lat",
            "lat",
            &first.latitude,
            "latitude",
            "degrees_north",
        )?;
        put_coordinate(
            &mut file,
            "lat_s",
            "lat",
            &first.lat_s,
            "southern latitude boundary",
            "degrees_north",
        )?;
        put_coordinate(
            &mut file,
            "lat_n",
            "lat",
            &first.lat_n,
            "northern latitude boundary",
            "degrees_north",
        )?;
        put_coordinate(
            &mut file,
            "lon_w",
            "lon",
            &first.lon_w,
            "western longitude boundary",
            "degrees_east",
        )?;
        put_coordinate(
            &mut file,
            "lon_e",
            "lon",
            &first.lon_e,
            "eastern longitude boundary",
            "degrees_east",
        )?;
    }
    if let Some(length) = file.dimension_len(dimension) {
        ensure!(
            length == dimension_values.len(),
            "diagnostic dimension {dimension} has length {length}; expected {}",
            dimension_values.len()
        );
        let stored = file
            .variable(dimension)
            .with_context(|| format!("diagnostic dimension {dimension} has no coordinate"))?
            .get_values::<i32, _>(..)?;
        ensure!(
            stored == dimension_values,
            "diagnostic dimension {dimension} has incompatible coordinate values"
        );
    } else {
        file.add_unlimited_dimension(dimension)?;
        file.add_variable::<i32>(dimension, &[dimension])?
            .put_values(dimension_values, ..dimension_values.len())?;
    }
    ensure!(
        file.variable(name).is_none(),
        "diagnostic {} already has variable {name}",
        path.display()
    );
    if first.ntypes > 1 {
        let mut variable =
            file.add_variable::<f64>(name, &[dimension, "TypeIndex", "lat", "lon"])?;
        if first.sparse.is_some() {
            variable.set_fill_value(first.fill_value)?;
            for (record, frame) in mapped.iter().enumerate() {
                write_sparse_dimension(&mut variable, frame, record, true)?;
            }
        } else {
            let values = mapped
                .iter()
                .flat_map(|frame| frame.values.iter().copied())
                .collect::<Vec<_>>();
            variable.put_values(&values, (.., .., .., ..))?;
        }
        variable.put_attribute("missing_value", missing)?;
        let mut all_type =
            file.add_variable::<f64>(&format!("{name}_grid"), &[dimension, "lat", "lon"])?;
        if first.sparse.is_some() {
            all_type.set_fill_value(first.fill_value)?;
            for (record, frame) in mapped.iter().enumerate() {
                write_sparse_dimension(&mut all_type, frame, record, false)?;
            }
        } else {
            let grid = mapped
                .iter()
                .flat_map(|frame| frame.grid.iter().copied())
                .collect::<Vec<_>>();
            all_type.put_values(&grid, (.., .., ..))?;
        }
        all_type.put_attribute("missing_value", missing)?;
    } else {
        let mut variable = file.add_variable::<f64>(name, &[dimension, "lat", "lon"])?;
        if first.sparse.is_some() {
            variable.set_fill_value(first.fill_value)?;
            for (record, frame) in mapped.iter().enumerate() {
                write_sparse_dimension(&mut variable, frame, record, false)?;
            }
        } else {
            let values = mapped
                .iter()
                .flat_map(|frame| frame.values.iter().copied())
                .collect::<Vec<_>>();
            variable.put_values(&values, (.., .., ..))?;
        }
        variable.put_attribute("missing_value", missing)?;
    }
    file.close()?;
    Ok(())
}

fn diagnostic_grid(topology: &SpatialTopology) -> SpatialGrid {
    match topology.kind {
        SpatialInputKind::GridBased => topology.grid.clone(),
        SpatialInputKind::Unstructured | SpatialInputKind::Catchment => regular_tenth_degree_grid(),
    }
}

fn regular_tenth_degree_grid() -> SpatialGrid {
    let lon_w = (0..3600).map(|index| -180.0 + index as f64 * 0.1).collect();
    let lon_e = (1..=3600)
        .map(|index| {
            if index == 3600 {
                -180.0
            } else {
                -180.0 + index as f64 * 0.1
            }
        })
        .collect();
    let lat_s = (0..1800).map(|index| -90.0 + index as f64 * 0.1).collect();
    let lat_n = (1..=1800).map(|index| -90.0 + index as f64 * 0.1).collect();
    SpatialGrid {
        lon_w,
        lon_e,
        lat_s,
        lat_n,
    }
}

fn longitude_index(value: f64, grid: &SpatialGrid) -> Result<usize> {
    grid.lon_w
        .iter()
        .zip(&grid.lon_e)
        .position(|(&west, &east)| {
            if west >= east {
                value >= west || value < east
            } else {
                value >= west && value < east
            }
        })
        .with_context(|| format!("longitude {value} is outside diagnostic grid"))
}

fn latitude_index(value: f64, grid: &SpatialGrid) -> Result<usize> {
    grid.lat_s
        .iter()
        .zip(&grid.lat_n)
        .position(|(&south, &north)| value >= south && (value < north || nearly_equal(value, 90.0)))
        .with_context(|| format!("latitude {value} is outside diagnostic grid"))
}

fn pixel_area(west: f64, east: f64, south: f64, north: f64) -> Result<f64> {
    let mut width = east - west;
    if width <= 0.0 {
        width += 360.0;
    }
    let area = width.to_radians() * (north.to_radians().sin() - south.to_radians().sin());
    ensure!(
        area.is_finite() && area > 0.0,
        "diagnostic pixel has invalid area"
    );
    Ok(area)
}

fn midpoint_longitude(west: f64, east: f64) -> f64 {
    let mut east = east;
    if east <= west {
        east += 360.0;
    }
    (west + (east - west) * 0.5 + 180.0).rem_euclid(360.0) - 180.0
}

fn midpoint_latitude(south: f64, north: f64) -> f64 {
    (south + north) * 0.5
}

fn nearly_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-9_f64.max(left.abs().max(right.abs()) * 1e-12)
}

fn put_coordinate(
    file: &mut netcdf::FileMut,
    name: &str,
    dimension: &str,
    values: &[f64],
    long_name: &str,
    units: &str,
) -> Result<()> {
    // ncio_define_dimension predefines center coordinates as NF90_FLOAT;
    // edge coordinates and all scientific fields retain f64.
    let mut variable = if matches!(name, "lon" | "lat") {
        file.add_variable::<f32>(name, &[dimension])?
    } else {
        file.add_variable::<f64>(name, &[dimension])?
    };
    variable.put_values(values, ..)?;
    variable.put_attribute("long_name", long_name)?;
    variable.put_attribute("units", units)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{FlatMesh, PixelAxes};

    fn topology() -> SpatialTopology {
        let mesh = FlatMesh::new(vec![1, 2], vec![0, 1, 2], vec![1, 2], vec![1, 1]).unwrap();
        SpatialTopology {
            kind: SpatialInputKind::GridBased,
            grid: SpatialGrid {
                lon_w: vec![-180.0],
                lon_e: vec![-180.0],
                lat_s: vec![-90.0],
                lat_n: vec![90.0],
            },
            pixel: PixelAxes {
                edge_south: -90.0,
                edge_north: 90.0,
                edge_west: -180.0,
                edge_east: -180.0,
                lon_w: vec![-180.0, 0.0],
                lon_e: vec![0.0, -180.0],
                lat_s: vec![-90.0],
                lat_n: vec![90.0],
            },
            source: None,
            element_block_owners: None,
            land_elements: mesh.land_elements(),
            mesh,
        }
    }

    fn patches() -> FlatLandPatches {
        FlatLandPatches {
            element_ids: vec![1, 2],
            pixel_start: vec![1, 1],
            pixel_end: vec![1, 1],
            set_type: vec![1, 2],
            element_index: vec![1, 2],
        }
    }

    #[test]
    fn maps_and_writes_type_split_means_like_upstream_diag() {
        let mapped = map_patch_diagnostic(
            &topology(),
            &patches(),
            &[1.0, 3.0],
            &[1, 2],
            DiagnosticStatistic::Mean,
            None,
            DIAGNOSTIC_MISSING,
            Some(0.0),
        )
        .unwrap();
        assert_eq!(mapped.values, vec![1.0, 3.0]);
        assert_eq!(mapped.grid, vec![2.0]);

        let path = std::env::temp_dir().join(format!(
            "colm-srfdata-diagnostic-{}-{}.nc",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_patch_diagnostic(&path, "field", &[1, 2], &mapped, DIAGNOSTIC_MISSING).unwrap();
        let file = netcdf::open(&path).unwrap();
        use netcdf::types::{FloatType, NcVariableType};
        assert_eq!(
            file.variables()
                .map(|variable| variable.name())
                .collect::<Vec<_>>(),
            [
                "TypeIndex",
                "lon",
                "lat",
                "lat_s",
                "lat_n",
                "lon_w",
                "lon_e",
                "field",
                "field_grid"
            ]
        );
        for name in ["lon", "lat"] {
            assert_eq!(
                file.variable(name).unwrap().vartype(),
                NcVariableType::Float(FloatType::F32)
            );
        }
        for name in ["lon_w", "lon_e", "lat_s", "lat_n", "field", "field_grid"] {
            assert_eq!(
                file.variable(name).unwrap().vartype(),
                NcVariableType::Float(FloatType::F64)
            );
        }
        assert_eq!(
            file.variable("TypeIndex")
                .unwrap()
                .get_values::<i32, _>(..)
                .unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            file.variable("field")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![1.0, 3.0]
        );
        assert_eq!(
            file.variable("field_grid")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![2.0]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn fractions_use_shared_pixelset_area_not_pre_normalized_values() {
        let mapped = map_patch_diagnostic(
            &topology(),
            &patches(),
            &[0.2, 0.8],
            &[1, 2],
            DiagnosticStatistic::Fraction,
            Some(&[0.2, 0.8]),
            DIAGNOSTIC_MISSING,
            Some(0.0),
        )
        .unwrap();
        assert_eq!(mapped.values, vec![0.2, 0.8]);
        assert_eq!(mapped.grid, vec![1.0]);
    }

    #[test]
    fn wmo_virtual_patches_use_whole_element_geometry_in_diagnostic_mean() {
        let mut topology = topology();
        topology.mesh = FlatMesh::new(vec![1], vec![0, 1], vec![1], vec![1]).unwrap();
        topology.land_elements = topology.mesh.land_elements();
        let patches = FlatLandPatches {
            element_ids: vec![1, 1],
            pixel_start: vec![1, 0],
            pixel_end: vec![1, 0],
            set_type: vec![1, 1],
            element_index: vec![1, 1],
        };
        let mapped = map_patch_diagnostic(
            &topology,
            &patches,
            &[1.0, 0.0],
            &[1],
            DiagnosticStatistic::Mean,
            None,
            DIAGNOSTIC_MISSING,
            Some(0.0),
        )
        .unwrap();
        assert_eq!(mapped.values, vec![0.5]);
        assert_eq!(mapped.grid, vec![0.5]);
    }

    #[test]
    fn sparse_mapping_keeps_global_defaults_without_dense_buffers() {
        let path = std::env::temp_dir().join(format!(
            "colm-srfdata-diagnostic-sparse-{}-{}.nc",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mapped = map_patch_diagnostic(
            &topology(),
            &patches(),
            &[1.0, 3.0],
            &[1, 2, 3, 4, 5],
            DiagnosticStatistic::Mean,
            None,
            DIAGNOSTIC_MISSING,
            Some(0.0),
        )
        .unwrap();
        assert!(mapped.values.is_empty());
        write_patch_diagnostic(
            &path,
            "field",
            &[1, 2, 3, 4, 5],
            &mapped,
            DIAGNOSTIC_MISSING,
        )
        .unwrap();
        let file = netcdf::open(&path).unwrap();
        assert_eq!(
            file.variable("field")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![1.0, 3.0, 0.0, 0.0, 0.0]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn writes_monthly_type_split_diagnostics_as_time_records() {
        let path = std::env::temp_dir().join(format!(
            "colm-srfdata-diagnostic-time-{}-{}.nc",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_patch_diagnostic_time(
            &path,
            "LAI",
            &topology(),
            &patches(),
            &[vec![1.0, 3.0], vec![2.0, 4.0]],
            &[1, 2],
            DiagnosticStatistic::Mean,
            None,
            DIAGNOSTIC_MISSING,
            Some(0.0),
        )
        .unwrap();
        let file = netcdf::open(&path).unwrap();
        assert_eq!(
            file.variable("Itime")
                .unwrap()
                .get_values::<i32, _>(..)
                .unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            file.variable("LAI")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![1.0, 3.0, 2.0, 4.0]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn appends_layered_fields_to_one_diagnostic_file() {
        let path = std::env::temp_dir().join(format!(
            "colm-srfdata-diagnostic-layers-{}-{}.nc",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let frames = [vec![1.0, 3.0], vec![2.0, 4.0]];
        write_patch_diagnostic_dimension(
            &path,
            "CV_ROOF",
            &topology(),
            &patches(),
            &frames,
            &[1, 2],
            DiagnosticStatistic::Mean,
            None,
            DIAGNOSTIC_MISSING,
            Some(0.0),
            "ulev",
            &[1, 2],
        )
        .unwrap();
        write_patch_diagnostic_dimension(
            &path,
            "TK_ROOF",
            &topology(),
            &patches(),
            &frames,
            &[1, 2],
            DiagnosticStatistic::Mean,
            None,
            DIAGNOSTIC_MISSING,
            Some(0.0),
            "ulev",
            &[1, 2],
        )
        .unwrap();
        let file = netcdf::open(&path).unwrap();
        assert_eq!(file.dimension("ulev").unwrap().len(), 2);
        assert!(file.dimension("ulev").unwrap().is_unlimited());
        assert_eq!(
            file.variable("TK_ROOF")
                .unwrap()
                .get_values::<f64, _>(..)
                .unwrap(),
            vec![1.0, 3.0, 2.0, 4.0]
        );
        let _ = std::fs::remove_file(path);
    }
}
