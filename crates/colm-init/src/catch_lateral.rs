//! Cold-start restart output for CoLM's `CatchLateralFlow` kernel.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_srfdata::{mesh_cell_area_weights, read_mesh_coordinate_raster_f64, FlatMesh, PixelAxes};
use netcdf::types::{FloatType, IntType, NcVariableType};

use crate::RestartDate;

/// Inputs needed for a CatchLateralFlow cold restart.
#[derive(Debug, Clone, Copy)]
pub struct CatchLateralColdStartConfig<'a> {
    pub catchment_mesh: &'a Path,
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    pub date: RestartDate,
    pub estimated_river_depth: bool,
    /// Directory containing the native `runoff_clim.nc` when river depth is
    /// estimated instead of read from the catchment mesh.
    pub runtime_dir: Option<&'a Path>,
}

/// The basin restart produced by [`write_catch_lateral_cold_restart`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchLateralColdStartFile {
    pub path: PathBuf,
}

struct CatchLateralColdState {
    basin: Vec<i64>,
    hru_basin: Vec<i64>,
    hru_type: Vec<i32>,
    basin_depth: Vec<f64>,
    hru_depth: Vec<f64>,
}

struct Hru {
    basin: i64,
    kind: i32,
    start: i32,
    end: i32,
    lake_depth: Option<f64>,
}

/// Write the native four-vector CatchLateralFlow cold state.
///
/// The surface stage's `landhru` files define the actual regional HRU order;
/// using them avoids reintroducing HRUs outside a clipped domain.
pub fn write_catch_lateral_cold_restart(
    config: CatchLateralColdStartConfig<'_>,
) -> Result<CatchLateralColdStartFile> {
    ensure!(
        !config.case_name.is_empty()
            && !config.case_name.contains('/')
            && !config.case_name.contains('\\'),
        "CatchLateralFlow restart case name must be a filename component"
    );
    ensure!(
        (0..=9999).contains(&config.land_cover_year) && (0..=9999).contains(&config.date.year),
        "CatchLateralFlow restart years must fit CoLM's four-digit filename convention"
    );
    ensure!(
        (1..=366).contains(&config.date.julian_day) && config.date.seconds < 86_400,
        "CatchLateralFlow restart date is invalid"
    );
    let estimated_depth = config
        .estimated_river_depth
        .then(|| {
            let runtime = config.runtime_dir.context(
                "DEF_USE_EstimatedRiverDepth requires DEF_dir_runtime for runoff_clim.nc",
            )?;
            estimate_river_depths(
                config.catchment_mesh,
                config.landdata,
                config.land_cover_year,
                runtime,
            )
        })
        .transpose()?;
    let state = read_cold_state(
        config.catchment_mesh,
        config.landdata,
        config.land_cover_year,
        estimated_depth.as_deref(),
    )?;
    let date = format!(
        "{:04}-{:03}-{:05}",
        config.date.year, config.date.julian_day, config.date.seconds
    );
    let directory = config.restart_dir.join(&date);
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "cannot create CatchLateralFlow restart directory {}",
            directory.display()
        )
    })?;
    let path = directory.join(format!(
        "{}_restart_basin_{date}_lc{:04}.nc",
        config.case_name, config.land_cover_year
    ));
    let mut file = netcdf::create(&path)
        .with_context(|| format!("cannot create CatchLateralFlow restart {}", path.display()))?;
    file.add_dimension("basin", state.basin.len())?;
    file.add_dimension("hydrounit", state.hru_basin.len())?;
    put_i64(&mut file, "basin", &["basin"], &state.basin)?;
    file.variable_mut("basin")
        .expect("the basin identity was just written")
        .put_attribute("long_name", "basin index")?;
    put_i64(&mut file, "bsn_hru", &["hydrounit"], &state.hru_basin)?;
    file.variable_mut("bsn_hru")
        .expect("the HRU basin identity was just written")
        .put_attribute("long_name", "basin index of hydrological units")?;
    put_i32(&mut file, "hru_type", &["hydrounit"], &state.hru_type)?;
    file.variable_mut("hru_type")
        .expect("the HRU type was just written")
        .put_attribute("long_name", "index of hydrological units inside basin")?;
    let basin_zero = vec![0.0; state.basin.len()];
    let hru_zero = vec![0.0; state.hru_basin.len()];
    put_f64(&mut file, "veloc_riv", &["basin"], &basin_zero)?;
    put_f64(&mut file, "wdsrf_bsn_prev", &["basin"], &state.basin_depth)?;
    put_f64(&mut file, "veloc_hru", &["hydrounit"], &hru_zero)?;
    put_f64(
        &mut file,
        "wdsrf_hru_prev",
        &["hydrounit"],
        &state.hru_depth,
    )?;
    file.close()
        .with_context(|| format!("cannot close CatchLateralFlow restart {}", path.display()))?;
    Ok(CatchLateralColdStartFile { path })
}

/// Reproduce `calc_riverdepth_from_runoff` for a serial cold start.
///
/// Upstream maps monthly runoff to `landelm`, turns it into basin discharge
/// with the generated land-patch area, then accumulates discharge downstream.
/// A CatchLateral surface contains every basin from its mesh; rejecting a
/// partial surface is preferable to silently changing upstream discharge.
fn estimate_river_depths(
    catchment_mesh: &Path,
    landdata: &Path,
    land_cover_year: i32,
    runtime_dir: &Path,
) -> Result<Vec<f64>> {
    let source = netcdf::open(catchment_mesh).with_context(|| {
        format!(
            "cannot open CatchLateralFlow mesh {}",
            catchment_mesh.display()
        )
    })?;
    let downstream = read_i64(&source, "basin_downstream")?;
    ensure!(
        !downstream.is_empty(),
        "CatchLateralFlow basin_downstream must not be empty"
    );
    let basins = read_active_basins(landdata, land_cover_year)?;
    ensure!(
        basins.len() == downstream.len()
            && basins
                .iter()
                .copied()
                .eq(1..=i64::try_from(downstream.len())?),
        "DEF_USE_EstimatedRiverDepth requires landhru to contain every CatchLateralFlow basin"
    );
    let (mesh, pixel) = read_estimation_mesh(landdata, land_cover_year, &basins)?;
    let cell_area = mesh_cell_area_weights(&mesh, &pixel)?;
    let runoff =
        read_mesh_coordinate_raster_f64(&runtime_dir.join("runoff_clim.nc"), "ro", &mesh, &pixel)?;
    ensure!(
        runoff.len() == cell_area.len(),
        "runoff_clim mesh mapping and cell-area vectors disagree"
    );
    let patch_area = read_native_patch_areas(landdata, land_cover_year, &mesh, &cell_area)?;
    let mut runoff_discharge = vec![0.0; downstream.len()];
    let mut offset = 0;
    for element in 0..mesh.len() {
        let id = mesh.element_id(element)?;
        let count = mesh.pixel_count(element)?;
        let area = &cell_area[offset..offset + count];
        let values = &runoff[offset..offset + count];
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "runoff_clim contains non-finite runoff for CatchLateralFlow basin {id}"
        );
        let area_sum = area.iter().sum::<f64>();
        ensure!(
            area_sum.is_finite() && area_sum > 0.0,
            "CatchLateralFlow basin {id} has non-positive area"
        );
        let runoff_mean = values
            .iter()
            .zip(area)
            .map(|(runoff, area)| runoff.max(0.0) * area)
            .sum::<f64>()
            / area_sum;
        let total_area = *patch_area
            .get(&id)
            .with_context(|| format!("CatchLateralFlow basin {id} has no landpatch area"))?;
        let index = usize::try_from(id - 1)
            .context("CatchLateralFlow basin index cannot address runoff discharge")?;
        runoff_discharge[index] = runoff_mean / 86_400.0 * total_area;
        offset += count;
    }
    let discharge = accumulate_downstream(&downstream, runoff_discharge)?;
    Ok(discharge
        .into_iter()
        .map(|value| (0.1 * value.sqrt()).max(1.0))
        .collect())
}

fn read_active_basins(landdata: &Path, land_cover_year: i32) -> Result<BTreeSet<i64>> {
    let directory = landdata
        .join("landhru")
        .join(format!("{land_cover_year:04}"));
    let files = netcdf_block_files(&directory, "landhru_")?;
    let mut basins = BTreeSet::new();
    for path in files {
        basins.extend(read_i64(&netcdf::open(&path)?, "eindex")?);
    }
    ensure!(
        basins.iter().all(|basin| *basin > 0),
        "CatchLateralFlow landhru basin IDs must be positive"
    );
    Ok(basins)
}

fn read_estimation_mesh(
    landdata: &Path,
    land_cover_year: i32,
    basins: &BTreeSet<i64>,
) -> Result<(FlatMesh, PixelAxes)> {
    let pixel_file = netcdf::open(landdata.join("pixel.nc"))?;
    let lon_w = read_f64(&pixel_file, "lon_w")?;
    let lon_e = read_f64(&pixel_file, "lon_e")?;
    let lat_s = read_f64(&pixel_file, "lat_s")?;
    let lat_n = read_f64(&pixel_file, "lat_n")?;
    ensure!(
        lon_w.len() == lon_e.len()
            && lat_s.len() == lat_n.len()
            && !lon_w.is_empty()
            && !lat_s.is_empty(),
        "CatchLateralFlow pixel edge vectors are invalid"
    );
    let pixel = PixelAxes {
        edge_south: lat_s.iter().copied().fold(f64::INFINITY, f64::min),
        edge_north: lat_n.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        edge_west: lon_w[0],
        edge_east: *lon_e.last().expect("non-empty longitude edge vector"),
        lon_w,
        lon_e,
        lat_s,
        lat_n,
    };
    let directory = landdata.join("mesh").join(format!("{land_cover_year:04}"));
    let mut elements = BTreeMap::<i64, (Vec<i32>, Vec<i32>)>::new();
    for path in netcdf_block_files(&directory, "mesh_")? {
        let file = netcdf::open(&path)?;
        let ids = read_i64(&file, "elmindex")?;
        let counts = read_i32(&file, "elmnpxl")?;
        let points = read_i32_coordinate_pairs(&file, "elmpixels")?;
        ensure!(
            ids.len() == counts.len(),
            "CatchLateralFlow mesh block {} has inconsistent element vectors",
            path.display()
        );
        let mut point_offset = 0;
        for (&id, &count) in ids.iter().zip(&counts) {
            let count = usize::try_from(count)
                .context("CatchLateralFlow mesh pixel count must be positive")?;
            ensure!(
                point_offset + count <= points.len(),
                "CatchLateralFlow mesh block {} has truncated coordinates",
                path.display()
            );
            if basins.contains(&id) {
                let (x, y): (Vec<_>, Vec<_>) = points[point_offset..point_offset + count]
                    .iter()
                    .copied()
                    .unzip();
                ensure!(
                    elements.insert(id, (x, y)).is_none(),
                    "CatchLateralFlow mesh has duplicate basin {id}"
                );
            }
            point_offset += count;
        }
        ensure!(
            point_offset == points.len(),
            "CatchLateralFlow mesh block {} has excess coordinates",
            path.display()
        );
    }
    ensure!(
        elements.len() == basins.len() && elements.keys().eq(basins.iter()),
        "CatchLateralFlow mesh and landhru basin sets disagree"
    );
    let mut ids = Vec::with_capacity(elements.len());
    let mut offsets = Vec::with_capacity(elements.len() + 1);
    let mut ilon = Vec::new();
    let mut ilat = Vec::new();
    offsets.push(0);
    for (id, (x, y)) in elements {
        ids.push(id);
        ilon.extend(x);
        ilat.extend(y);
        offsets.push(ilon.len());
    }
    Ok((FlatMesh::new(ids, offsets, ilon, ilat)?, pixel))
}

fn read_native_patch_areas(
    landdata: &Path,
    land_cover_year: i32,
    mesh: &FlatMesh,
    cell_area: &[f64],
) -> Result<BTreeMap<i64, f64>> {
    let mut spans = BTreeMap::new();
    let mut offset = 0;
    for element in 0..mesh.len() {
        let id = mesh.element_id(element)?;
        let count = mesh.pixel_count(element)?;
        spans.insert(id, (offset, count));
        offset += count;
    }
    ensure!(
        offset == cell_area.len(),
        "CatchLateralFlow mesh and cell-area vectors disagree"
    );
    let directory = landdata
        .join("landpatch")
        .join(format!("{land_cover_year:04}"));
    let mut areas = BTreeMap::new();
    for path in netcdf_block_files(&directory, "landpatch_")? {
        let file = netcdf::open(&path)?;
        let ids = read_i64(&file, "eindex")?;
        let starts = read_i32(&file, "ipxstt")?;
        let ends = read_i32(&file, "ipxend")?;
        ensure!(
            ids.len() == starts.len() && starts.len() == ends.len(),
            "CatchLateralFlow landpatch block {} has inconsistent topology vectors",
            path.display()
        );
        for ((id, start), end) in ids.into_iter().zip(starts).zip(ends) {
            let Some(&(offset, count)) = spans.get(&id) else {
                continue;
            };
            let start = usize::try_from(start)
                .context("CatchLateralFlow landpatch start must be positive")?;
            let end =
                usize::try_from(end).context("CatchLateralFlow landpatch end must be positive")?;
            ensure!(
                start > 0 && start <= end && end <= count,
                "CatchLateralFlow landpatch range is outside basin {id}"
            );
            // MOD_Initialize.F90 sums each landpatch directly; unlike the main
            // time-step setup, its cold-start path does not apply pctshared.
            *areas.entry(id).or_insert(0.0) += cell_area[offset + start - 1..offset + end]
                .iter()
                .sum::<f64>()
                * 1.0e6; // mesh_cell_area_weights returns km²; discharge needs m².
        }
    }
    ensure!(
        areas.len() == mesh.len() && areas.values().all(|area| area.is_finite() && *area > 0.0),
        "CatchLateralFlow landpatch area does not cover every basin"
    );
    Ok(areas)
}

fn accumulate_downstream(downstream: &[i64], mut discharge: Vec<f64>) -> Result<Vec<f64>> {
    ensure!(
        discharge.len() == downstream.len()
            && discharge
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "CatchLateralFlow runoff discharge is invalid"
    );
    let mut upstream = vec![0_usize; downstream.len()];
    for (index, target) in downstream.iter().copied().enumerate() {
        if target > 0 {
            let target = usize::try_from(target - 1)
                .context("CatchLateralFlow downstream basin index is invalid")?;
            ensure!(
                target < downstream.len() && target != index,
                "CatchLateralFlow downstream basin index is invalid"
            );
            upstream[target] += 1;
        }
    }
    let mut ready = (0..downstream.len())
        .filter(|&index| upstream[index] == 0)
        .collect::<VecDeque<_>>();
    let mut completed = 0;
    while let Some(index) = ready.pop_front() {
        completed += 1;
        if downstream[index] > 0 {
            let target =
                usize::try_from(downstream[index] - 1).expect("validated downstream index");
            discharge[target] += discharge[index];
            upstream[target] -= 1;
            if upstream[target] == 0 {
                ready.push_back(target);
            }
        }
    }
    ensure!(
        completed == downstream.len(),
        "CatchLateralFlow basin_downstream must be acyclic"
    );
    Ok(discharge)
}

fn netcdf_block_files(directory: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
    let mut files = std::fs::read_dir(directory)
        .with_context(|| {
            format!(
                "cannot read CatchLateralFlow directory {}",
                directory.display()
            )
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| {
        path.extension().is_some_and(|extension| extension == "nc")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
    });
    files.sort();
    ensure!(
        !files.is_empty(),
        "CatchLateralFlow needs {prefix} NetCDF blocks in {}",
        directory.display()
    );
    Ok(files)
}

fn read_i32_coordinate_pairs(file: &netcdf::File, name: &str) -> Result<Vec<(i32, i32)>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("CatchLateralFlow input is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2 && dimensions[1].len() == 2,
        "CatchLateralFlow {name} must use native (pixel, ncoor=2) dimensions"
    );
    let values = match variable.vartype() {
        NcVariableType::Int(IntType::I32) => variable.get_values::<i32, _>(..)?.into_iter(),
        kind => bail!("CatchLateralFlow {name} must be an int32 matrix, got {kind:?}"),
    };
    Ok(values
        .collect::<Vec<_>>()
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect())
}

fn read_cold_state(
    catchment_mesh: &Path,
    landdata: &Path,
    land_cover_year: i32,
    estimated_depth: Option<&[f64]>,
) -> Result<CatchLateralColdState> {
    let source = netcdf::open(catchment_mesh).with_context(|| {
        format!(
            "cannot open CatchLateralFlow mesh {}",
            catchment_mesh.display()
        )
    })?;
    let lake_id = read_i64(&source, "lake_id")?;
    let basin_numhru = read_i64(&source, "basin_numhru")?
        .into_iter()
        .map(|value| {
            usize::try_from(value).context("CatchLateralFlow basin_numhru must be positive")
        })
        .collect::<Result<Vec<_>>>()?;
    let river_depth = estimated_depth
        .map(<[f64]>::to_vec)
        .map(Ok)
        .unwrap_or_else(|| read_f64(&source, "river_depth"))?;
    ensure!(
        river_depth.len() == lake_id.len()
            && river_depth.len() == basin_numhru.len()
            && !river_depth.is_empty(),
        "CatchLateralFlow river depth, lake_id, and basin_numhru must have the same non-zero length"
    );
    ensure!(
        basin_numhru.iter().all(|value| *value > 0),
        "CatchLateralFlow basin_numhru must be positive"
    );
    ensure!(
        river_depth
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "CatchLateralFlow river_depth must be finite and non-negative"
    );
    let network_type = read_i32_matrix_basin_major(&source, "hydrounit_index", river_depth.len())?;
    let network_hand = read_f64_matrix_basin_major(&source, "hydrounit_hand", river_depth.len())?;
    ensure!(
        network_type
            .iter()
            .zip(&network_hand)
            .all(|(kind, hand)| kind.len() == hand.len()),
        "CatchLateralFlow hydrounit_index and hydrounit_hand dimensions must match"
    );

    let directory = landdata
        .join("landhru")
        .join(format!("{land_cover_year:04}"));
    let mut files = std::fs::read_dir(&directory)
        .with_context(|| {
            format!(
                "cannot read CatchLateralFlow HRU directory {}",
                directory.display()
            )
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.retain(|path| path.extension().is_some_and(|extension| extension == "nc"));
    files.sort();
    ensure!(
        !files.is_empty(),
        "CatchLateralFlow needs at least one landhru NetCDF block in {}",
        directory.display()
    );

    let mut hru = Vec::<Hru>::new();
    for path in files {
        let file = netcdf::open(&path).with_context(|| {
            format!("cannot open CatchLateralFlow HRU block {}", path.display())
        })?;
        let basin = read_i64(&file, "eindex")?;
        let kind = read_i32(&file, "settyp")?;
        let start = read_i32(&file, "ipxstt")?;
        let end = read_i32(&file, "ipxend")?;
        ensure!(
            basin.len() == kind.len() && kind.len() == start.len() && start.len() == end.len(),
            "CatchLateralFlow HRU block {} has inconsistent topology vectors",
            path.display()
        );
        let lake_depth = read_lake_depths(
            landdata,
            land_cover_year,
            &path,
            &basin,
            &kind,
            &start,
            &end,
        )?;
        hru.extend(
            basin
                .into_iter()
                .zip(kind)
                .zip(start)
                .zip(end)
                .zip(lake_depth)
                .map(|((((basin, kind), start), end), lake_depth)| Hru {
                    basin,
                    kind,
                    start,
                    end,
                    lake_depth,
                }),
        );
    }
    hru.sort_unstable_by_key(|entry| (entry.basin, entry.kind.unsigned_abs()));
    ensure!(
        hru.windows(2).all(|pair| {
            (pair[0].basin, pair[0].kind.unsigned_abs())
                != (pair[1].basin, pair[1].kind.unsigned_abs())
        }),
        "CatchLateralFlow landhru blocks contain duplicate basin/HRU entries"
    );

    let mut basin = Vec::new();
    let mut hru_basin = Vec::with_capacity(hru.len());
    let mut hru_type = Vec::with_capacity(hru.len());
    let mut basin_depth = Vec::new();
    let mut hru_depth = Vec::with_capacity(hru.len());
    let mut offset = 0;
    while offset < hru.len() {
        let basin_id = hru[offset].basin;
        let index = usize::try_from(basin_id - 1)
            .context("CatchLateralFlow landhru basin index cannot address mesh metadata")?;
        ensure!(
            index < river_depth.len(),
            "CatchLateralFlow landhru basin {basin_id} exceeds mesh metadata"
        );
        let end = hru[offset..]
            .iter()
            .position(|entry| entry.basin != basin_id)
            .map_or(hru.len(), |length| offset + length);
        let entries = &hru[offset..end];
        let kinds = entries
            .iter()
            .map(|entry| {
                entry
                    .kind
                    .checked_abs()
                    .context("CatchLateralFlow HRU type cannot be int32::MIN")
            })
            .collect::<Result<Vec<_>>>()?;
        let is_lake = lake_id[index] > 0;
        ensure!(
            entries.iter().all(|entry| (entry.kind < 0) == is_lake
                && entry.start > 0
                && entry.start <= entry.end),
            "CatchLateralFlow basin {basin_id} has a lake/HRU topology sign mismatch"
        );
        let expected = basin_numhru[index];
        ensure!(
            kinds.len() == expected,
            "CatchLateralFlow basin {basin_id} has {} regional HRUs but native mesh requires {expected}",
            kinds.len()
        );
        ensure!(
            expected <= network_type[index].len() && expected <= network_hand[index].len(),
            "CatchLateralFlow basin {basin_id} exceeds hydrounit network dimensions"
        );
        ensure!(
            network_type[index][..expected] == kinds,
            "CatchLateralFlow basin {basin_id} HRU ordering disagrees with native hydrounit_index"
        );
        let mut hand = network_hand[index][..expected].to_vec();
        ensure!(
            hand.iter().all(|value| value.is_finite()),
            "CatchLateralFlow basin {basin_id} hydrounit_hand must be finite"
        );
        let mut depth = vec![0.0; expected];
        let river_stage = if is_lake {
            for (depth, entry) in depth.iter_mut().zip(entries) {
                *depth = entry.lake_depth.with_context(|| {
                    format!("CatchLateralFlow lake basin {basin_id} has no lakedepth source")
                })?;
            }
            depth.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        } else {
            if lake_id[index] == 0 {
                depth[0] = river_depth[index];
                for value in hand.iter_mut().skip(1) {
                    *value += river_depth[index];
                }
            }
            let hand_min = hand.iter().copied().fold(f64::INFINITY, f64::min);
            hand.iter()
                .zip(&depth)
                .map(|(height, water)| height + water)
                .fold(f64::INFINITY, f64::min)
                - hand_min
        };
        basin.push(basin_id);
        basin_depth.push(river_stage);
        hru_basin.extend(std::iter::repeat_n(basin_id, expected));
        hru_type.extend(&kinds);
        hru_depth.extend(depth);
        offset = end;
    }
    ensure!(
        !basin.is_empty(),
        "CatchLateralFlow landhru blocks have no active hydrological units"
    );
    Ok(CatchLateralColdState {
        basin,
        hru_basin,
        hru_type,
        basin_depth,
        hru_depth,
    })
}

fn read_lake_depths(
    landdata: &Path,
    land_cover_year: i32,
    hru_path: &Path,
    hru_basin: &[i64],
    hru_kind: &[i32],
    hru_start: &[i32],
    hru_end: &[i32],
) -> Result<Vec<Option<f64>>> {
    if hru_kind.iter().all(|kind| *kind >= 0) {
        return Ok(vec![None; hru_kind.len()]);
    }
    let filename = hru_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("CatchLateralFlow HRU block filename is not UTF-8")?;
    let block = filename
        .strip_prefix("landhru_")
        .and_then(|name| name.strip_suffix(".nc"))
        .context("CatchLateralFlow HRU block must be named landhru_<block>.nc")?;
    let year = format!("{land_cover_year:04}");
    let patch_path = landdata
        .join("landpatch")
        .join(&year)
        .join(format!("landpatch_{block}.nc"));
    let depth_path = landdata
        .join("lakedepth")
        .join(year)
        .join(format!("lakedepth_patches_{block}.nc"));
    let patches = netcdf::open(&patch_path).with_context(|| {
        format!(
            "cannot open CatchLateralFlow patch block {}",
            patch_path.display()
        )
    })?;
    let patch_basin = read_i64(&patches, "eindex")?;
    let patch_start = read_i32(&patches, "ipxstt")?;
    let patch_end = read_i32(&patches, "ipxend")?;
    let depths = read_f64(
        &netcdf::open(&depth_path).with_context(|| {
            format!(
                "cannot open CatchLateralFlow lake-depth block {}",
                depth_path.display()
            )
        })?,
        "lakedepth_patches",
    )?;
    ensure!(
        patch_basin.len() == patch_start.len()
            && patch_start.len() == patch_end.len()
            && patch_end.len() == depths.len(),
        "CatchLateralFlow lake patch vectors disagree in block {block}"
    );
    ensure!(
        depths
            .iter()
            .all(|depth| depth.is_finite() && *depth >= 0.0),
        "CatchLateralFlow lake depths in block {block} must be finite and non-negative"
    );
    hru_basin
        .iter()
        .zip(hru_kind)
        .zip(hru_start)
        .zip(hru_end)
        .map(|(((basin, kind), start), end)| {
            if *kind >= 0 {
                return Ok(None);
            }
            patch_basin
                .iter()
                .zip(&patch_start)
                .zip(&patch_end)
                .zip(&depths)
                .filter(|(((patch_basin, patch_start), patch_end), _)| {
                    **patch_basin == *basin && **patch_start >= *start && **patch_end <= *end
                })
                .map(|(_, depth)| *depth)
                .max_by(f64::total_cmp)
                .map(Some)
                .with_context(|| {
                    format!(
                        "CatchLateralFlow lake HRU {basin}/{} has no matching lakedepth patch",
                        kind.unsigned_abs()
                    )
                })
        })
        .collect()
}

fn read_i32_matrix_basin_major(
    file: &netcdf::File,
    name: &str,
    basins: usize,
) -> Result<Vec<Vec<i32>>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("CatchLateralFlow input is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2 && dimensions[0].len() == basins && dimensions[1].len() > 0,
        "CatchLateralFlow {name} must use native (basin, hydrounit) dimensions"
    );
    let values = match variable.vartype() {
        NcVariableType::Int(IntType::I32) => variable.get_values::<i32, _>(..)?,
        NcVariableType::Int(IntType::I64) => variable
            .get_values::<i64, _>(..)?
            .into_iter()
            .map(|value| {
                i32::try_from(value).context("CatchLateralFlow hydrounit index does not fit int32")
            })
            .collect::<Result<Vec<_>>>()?,
        kind => bail!("CatchLateralFlow {name} must be an integer matrix, got {kind:?}"),
    };
    Ok(values
        .chunks_exact(dimensions[1].len())
        .map(<[i32]>::to_vec)
        .collect())
}

fn read_f64_matrix_basin_major(
    file: &netcdf::File,
    name: &str,
    basins: usize,
) -> Result<Vec<Vec<f64>>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("CatchLateralFlow input is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2 && dimensions[0].len() == basins && dimensions[1].len() > 0,
        "CatchLateralFlow {name} must use native (basin, hydrounit) dimensions"
    );
    let values = match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => variable.get_values::<f64, _>(..)?,
        NcVariableType::Float(FloatType::F32) => variable
            .get_values::<f32, _>(..)?
            .into_iter()
            .map(f64::from)
            .collect(),
        kind => bail!("CatchLateralFlow {name} must be a floating-point matrix, got {kind:?}"),
    };
    Ok(values
        .chunks_exact(dimensions[1].len())
        .map(<[f64]>::to_vec)
        .collect())
}

fn read_i64(file: &netcdf::File, name: &str) -> Result<Vec<i64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("CatchLateralFlow input is missing {name}"))?;
    ensure!(
        variable.dimensions().len() == 1,
        "CatchLateralFlow {name} must be one-dimensional"
    );
    match variable.vartype() {
        NcVariableType::Int(IntType::I64) => Ok(variable.get_values::<i64, _>(..)?),
        NcVariableType::Int(IntType::I32) => Ok(variable
            .get_values::<i32, _>(..)?
            .into_iter()
            .map(i64::from)
            .collect()),
        kind => bail!("CatchLateralFlow {name} must be an integer vector, got {kind:?}"),
    }
}

fn read_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let values = read_i64(file, name)?;
    values
        .into_iter()
        .map(|value| i32::try_from(value).context("CatchLateralFlow HRU type does not fit int32"))
        .collect()
}

fn read_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("CatchLateralFlow input is missing {name}"))?;
    ensure!(
        variable.dimensions().len() == 1,
        "CatchLateralFlow {name} must be one-dimensional"
    );
    match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => Ok(variable.get_values::<f64, _>(..)?),
        NcVariableType::Float(FloatType::F32) => Ok(variable
            .get_values::<f32, _>(..)?
            .into_iter()
            .map(f64::from)
            .collect()),
        kind => bail!("CatchLateralFlow {name} must be a floating-point vector, got {kind:?}"),
    }
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

fn put_i64(
    file: &mut netcdf::FileMut,
    name: &str,
    dimensions: &[&str],
    values: &[i64],
) -> Result<()> {
    file.add_variable::<i64>(name, dimensions)?
        .put_values(values, ..)?;
    Ok(())
}

#[cfg(test)]
#[path = "catch_lateral_tests.rs"]
mod catch_lateral_tests;
