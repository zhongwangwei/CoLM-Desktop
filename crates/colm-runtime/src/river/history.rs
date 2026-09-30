//! 河道 history（`MOD_Grid_RiverLakeHist` 与 `MOD_Hist.F90:4749-4790`，`DEF_HIST_mode = 'one'`）。
//!
//! 每条 history 记录写两处：
//! - `<case>_hist_unitcat_<suffix>.nc`：全球单元流域网格（`lon_ucat`×`lat_ucat`）上的 12 个量，
//!   单元流域的值铺在它的 `(seq_x, seq_y)`，其余是 `spval`；
//! - 网格 history 里的 6 个量与静态的 `mask_complete_upstream_regird`：先把单元流域的值经
//!   `push_ucat2grid`/`push_ucat2inpm` 与 `remap_patch2inpm` 回到 patch，再按各自的过滤与分母聚合。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use colm_hist::history::HistoryGrid;
use colm_hist::schedule::ScheduledRecord;

use super::network::{RiverNetwork, RunoffRouting, RIVER_MOUTH};
use super::RiverHistory;

const SPVAL: f64 = colm_core::MISSING;

/// 网格 history 里由河道写出的量（闸门表名），按写出顺序。
pub const GRIDDED_RIVER_VARIABLES: [&str; 6] = [
    "wdpth_ucat_regrid",
    "veloc_riv_regrid",
    "discharge",
    "discharge_rivermouth_regrid",
    "floodfrc",
    "floodarea",
];

/// 河道 history 的静态部分（`hist_grid_riverlake_init`）与 unitcat 文件的落点。
pub struct RiverHistoryWriter {
    grid: Arc<HistoryGrid>,
    filter_ucat: Vec<bool>,
    filter_inpm: Vec<bool>,
    /// 区域网格上的 `sum_grid_area`/`sum_rmth_area`（没有贡献的网格是 `spval`）。
    sum_grid_area: Vec<f64>,
    sum_rmth_area: Vec<f64>,
    /// history 网格窗口上的 `sumarea_ucat`/`sumarea_inpm`。
    sumarea_ucat: Vec<f64>,
    sumarea_inpm: Vec<f64>,
    /// 单元流域的 `allups_mask_ucat`（1 或 0）。
    allups_mask: Vec<f64>,
    directory: PathBuf,
    stem: String,
    lon: Vec<f64>,
    lat: Vec<f64>,
}

/// `worker_remap_data_grid2pset`：`average` 除以非缺测份的面积和，`sum` 不除。
fn grid_to_patches(routing: &RunoffRouting, grid: &[f64], average: bool) -> Vec<f64> {
    routing
        .patch_parts
        .iter()
        .map(|parts| {
            let mut value = SPVAL;
            let mut area_sum = 0.0;
            for &(k, area) in parts {
                if grid[k] == SPVAL {
                    continue;
                }
                value = if value == SPVAL {
                    grid[k] * area
                } else {
                    value + grid[k] * area
                };
                area_sum += area;
            }
            if average && value != SPVAL && area_sum > 0.0 {
                value / area_sum
            } else {
                value
            }
        })
        .collect()
}

/// `push_ucat2grid`：每个区域网格取格心在其中的单元流域的值。
fn catchments_to_grid(routing: &RunoffRouting, values: &[f64]) -> Vec<f64> {
    routing
        .ucat_at_grid
        .iter()
        .map(|ucat| ucat.map_or(SPVAL, |i| values[i]))
        .collect()
}

/// `push_ucat2inpm`（`average`）：按单元流域的份面积加权平均，跳过缺测。
fn catchments_to_inpm(routing: &RunoffRouting, values: &[f64]) -> Vec<f64> {
    routing
        .grid_catchments
        .iter()
        .map(|entries| {
            let mut value = SPVAL;
            let mut area_sum = 0.0;
            for &(i, area) in entries {
                if values[i] == SPVAL {
                    continue;
                }
                value = if value == SPVAL {
                    values[i] * area
                } else {
                    value + values[i] * area
                };
                area_sum += area;
            }
            if value != SPVAL && area_sum > 0.0 {
                value / area_sum
            } else {
                value
            }
        })
        .collect()
}

/// `worker_remap_data_pset2grid`（填充值 `spval`）：过滤掉的与缺测的 patch 不参与。
fn patches_to_grid(routing: &RunoffRouting, values: &[f64], filter: &[bool]) -> Vec<f64> {
    let mut grid = vec![SPVAL; routing.grids.len()];
    for ((parts, &value), &keep) in routing.patch_parts.iter().zip(values).zip(filter) {
        if !keep || value == SPVAL {
            continue;
        }
        for &(k, area) in parts {
            let term = value * area;
            grid[k] = if grid[k] == SPVAL {
                term
            } else {
                grid[k] + term
            };
        }
    }
    grid
}

impl RiverHistoryWriter {
    /// `hist_grid_riverlake_init`。`basic[p]` 是 `patchtype < 99 .and. patchmask`。
    pub fn new(
        network: &RiverNetwork,
        routing: &RunoffRouting,
        grid: Arc<HistoryGrid>,
        basic: &[bool],
        directory: impl AsRef<Path>,
        stem: impl Into<String>,
    ) -> Result<Self> {
        let n = network.len();
        let ones = vec![1.0; n];
        let covered = grid_to_patches(routing, &catchments_to_grid(routing, &ones), true);
        let filter_ucat = basic
            .iter()
            .zip(&covered)
            .map(|(&keep, &value)| keep && value != SPVAL)
            .collect::<Vec<_>>();
        let unit = filter_ucat
            .iter()
            .map(|&keep| if keep { 1.0 } else { SPVAL })
            .collect::<Vec<_>>();
        let sum_grid_area = patches_to_grid(routing, &unit, &filter_ucat);
        let mouths = network
            .next
            .iter()
            .map(|&next| if next == RIVER_MOUTH { 1.0 } else { SPVAL })
            .collect::<Vec<_>>();
        let at_mouth = grid_to_patches(routing, &catchments_to_grid(routing, &mouths), true);
        let filter_rivmth = filter_ucat
            .iter()
            .zip(&at_mouth)
            .map(|(&keep, &value)| keep && value != SPVAL)
            .collect::<Vec<_>>();
        let unit_mouth = filter_rivmth
            .iter()
            .map(|&keep| if keep { 1.0 } else { SPVAL })
            .collect::<Vec<_>>();
        let sum_rmth_area = patches_to_grid(routing, &unit_mouth, &filter_rivmth);
        let inpm = grid_to_patches(routing, &catchments_to_inpm(routing, &ones), true);
        let filter_inpm = basic
            .iter()
            .zip(&inpm)
            .map(|(&keep, &value)| keep && value != SPVAL)
            .collect::<Vec<_>>();
        let sumarea_ucat = sumarea(&grid, &filter_ucat);
        let sumarea_inpm = sumarea(&grid, &filter_inpm);
        let allups_mask = complete_upstream_mask(network, routing);
        let lon_count = network.nlon;
        let lat_count = network.nlat;
        let ucat_grid = crate::spatial::grid::LatLonGrid::define_by_ndims(lon_count, lat_count)?;
        let lat = ucat_grid
            .lat_s
            .iter()
            .zip(&ucat_grid.lat_n)
            .map(|(s, n)| (s + n) * 0.5)
            .collect();
        let lon = ucat_grid
            .lon_w
            .iter()
            .zip(&ucat_grid.lon_e)
            .map(|(&w, &e)| {
                if w > e {
                    crate::spatial::grid::normalize_longitude((w + e + 360.0) * 0.5)
                } else {
                    Ok((w + e) * 0.5)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            grid,
            filter_ucat,
            filter_inpm,
            sum_grid_area,
            sum_rmth_area,
            sumarea_ucat,
            sumarea_inpm,
            allups_mask,
            directory: directory.as_ref().to_path_buf(),
            stem: stem.into(),
            lon,
            lat,
        })
    }

    /// 网格 history 的静态场 `mask_complete_upstream_regird`（每个文件第一条记录时写）。
    pub fn upstream_mask_static(
        network: &RiverNetwork,
        routing: &RunoffRouting,
        grid: &HistoryGrid,
        basic: &[bool],
    ) -> Vec<f64> {
        let n = network.len();
        let covered = grid_to_patches(routing, &catchments_to_grid(routing, &vec![1.0; n]), true);
        let filter_ucat = basic
            .iter()
            .zip(&covered)
            .map(|(&keep, &value)| keep && value != SPVAL)
            .collect::<Vec<_>>();
        let mask = complete_upstream_mask(network, routing);
        let mask_patch = grid_to_patches(routing, &catchments_to_grid(routing, &mask), true);
        let sumarea_ucat = sumarea(grid, &filter_ucat);
        // `hist_grid_riverlake_init`：`pset2grid` 之后 `WHERE (sumarea_ucat > 0) val = val/sumarea`，
        // 阈值是 0 而不是 `1e-5`，缺测也照除。
        let mut sum = vec![SPVAL; sumarea_ucat.len()];
        for (patch, (parts, &value)) in grid.parts.iter().zip(&mask_patch).enumerate() {
            if value == SPVAL || !filter_ucat[patch] {
                continue;
            }
            for &(cell, part) in parts {
                let term = value / 1.0 * part;
                sum[cell] = if sum[cell] == SPVAL {
                    term
                } else {
                    sum[cell] + term
                };
            }
        }
        sum.iter()
            .zip(&sumarea_ucat)
            .map(|(&value, &area)| if area > 0.0 { value / area } else { SPVAL })
            .collect()
    }

    /// 一条记录（`hist_grid_riverlake_out`）：写 unitcat 文件，把 6 个网格量交给会话，然后清零累加。
    pub fn write_record(
        &self,
        network: &RiverNetwork,
        routing: &RunoffRouting,
        history: &mut RiverHistory,
        record: &ScheduledRecord,
        end: colm_core::CalendarTime,
        session: &mut crate::history::HistorySession,
    ) -> Result<()> {
        let n = network.len();
        let window_seconds = history
            .acctime
            .iter()
            .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let mean = |values: &[f64]| {
            values
                .iter()
                .zip(&history.acctime)
                .map(|(&v, &t)| if t > 0.0 { v / t } else { SPVAL })
                .collect::<Vec<_>>()
        };
        let mean_or_keep = |values: &[f64]| {
            values
                .iter()
                .zip(&history.acctime)
                .map(|(&v, &t)| if t > 0.0 { v / t } else { v })
                .collect::<Vec<_>>()
        };
        let wdsrf = mean(&history.wdsrf);
        let veloc = mean(&history.veloc);
        let discharge = mean(&history.discharge);
        let floodarea = mean(&history.floodarea);
        let floodfrc = floodarea
            .iter()
            .zip(&network.area)
            .map(|(&a, &area)| if area > 0.0 { a / area } else { SPVAL })
            .collect::<Vec<_>>();
        let rivermouth = discharge
            .iter()
            .zip(&network.next)
            .map(|(&d, &next)| if next == RIVER_MOUTH { d } else { SPVAL })
            .collect::<Vec<_>>();
        let unitcat = [
            (
                "f_wdpth_ucat",
                "deepest water depth in river and flood plain",
                "m",
                wdsrf.clone(),
            ),
            (
                "f_veloc_riv",
                "water velocity in river",
                "m/s",
                veloc.clone(),
            ),
            (
                "f_discharge",
                "discharge in river and flood plain",
                "m^3/s",
                discharge.clone(),
            ),
            (
                "f_discharge_rivermouth",
                "river mouth discharge into ocean",
                "m^3/s",
                rivermouth.clone(),
            ),
            ("f_floodarea", "flooded area", "m^2", floodarea),
            ("f_floodfrc", "flooded area fraction", "-", floodfrc.clone()),
            (
                "f_rivsto",
                "below-bank river channel storage",
                "m^3",
                mean_or_keep(&history.rivsto),
            ),
            (
                "f_fldsto",
                "visible overbank storage excluding levee-protected storage",
                "m^3",
                mean_or_keep(&history.fldsto),
            ),
            (
                "f_flddph",
                "visible river-side floodplain water depth excluding levee-protected depth",
                "m",
                mean_or_keep(&history.flddph),
            ),
            (
                "f_storge",
                "total water storage (river+floodplain+levee)",
                "m^3",
                mean_or_keep(&history.storge),
            ),
            (
                "f_sfcelv",
                "water surface elevation",
                "m",
                mean_or_keep(&history.sfcelv),
            ),
        ];
        self.write_unitcat(network, record, end, window_seconds, &unitcat)?;

        // 回到 patch（`*_pch`），再按各自的过滤与分母聚合到 history 网格。
        let per_area = |values: &[f64], area: &[f64]| {
            catchments_to_grid(routing, values)
                .into_iter()
                .zip(area)
                .map(|(v, &a)| {
                    if a != SPVAL && v != SPVAL {
                        v / a
                    } else {
                        SPVAL
                    }
                })
                .collect::<Vec<_>>()
        };
        let wdsrf_pch = grid_to_patches(routing, &catchments_to_grid(routing, &wdsrf), true);
        let veloc_pch = grid_to_patches(routing, &catchments_to_grid(routing, &veloc), true);
        let discharge_pch =
            grid_to_patches(routing, &per_area(&discharge, &self.sum_grid_area), false);
        let mouth_pch =
            grid_to_patches(routing, &per_area(&rivermouth, &self.sum_rmth_area), false);
        let floodfrc_pch = grid_to_patches(routing, &catchments_to_inpm(routing, &floodfrc), true);
        let ones = vec![1.0; self.grid.lat.len() * self.grid.lon.len()];
        let fields = [
            (
                "wdpth_ucat_regrid",
                aggregate(
                    &self.grid,
                    &wdsrf_pch,
                    &self.filter_ucat,
                    &self.sumarea_ucat,
                    false,
                ),
            ),
            (
                "veloc_riv_regrid",
                aggregate(
                    &self.grid,
                    &veloc_pch,
                    &self.filter_ucat,
                    &self.sumarea_ucat,
                    false,
                ),
            ),
            (
                "discharge",
                aggregate(&self.grid, &discharge_pch, &self.filter_ucat, &ones, true),
            ),
            (
                "discharge_rivermouth_regrid",
                aggregate(&self.grid, &mouth_pch, &self.filter_ucat, &ones, true),
            ),
            (
                "floodfrc",
                aggregate(
                    &self.grid,
                    &floodfrc_pch,
                    &self.filter_inpm,
                    &self.sumarea_inpm,
                    false,
                ),
            ),
            (
                "floodarea",
                aggregate(&self.grid, &floodfrc_pch, &self.filter_inpm, &ones, false),
            ),
        ];
        for (name, values) in fields {
            session.stage_gridded(name, values)?;
        }
        debug_assert_eq!(history.acctime.len(), n);
        *history = RiverHistory::zeros(n);
        Ok(())
    }

    fn write_unitcat(
        &self,
        network: &RiverNetwork,
        record: &ScheduledRecord,
        end: colm_core::CalendarTime,
        window_seconds: f64,
        fields: &[(&str, &str, &str, Vec<f64>)],
    ) -> Result<()> {
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create {}", self.directory.display()))?;
        let path = self
            .directory
            .join(format!("{}_hist_unitcat_{}.nc", self.stem, record.suffix));
        let (nlon, nlat) = (network.nlon, network.nlat);
        let to_grid = |values: &[f64]| {
            let mut grid = vec![SPVAL; nlon * nlat];
            for (i, &value) in values.iter().enumerate() {
                let x = network.x[i] as usize - 1;
                let y = network.y[i] as usize - 1;
                grid[y * nlon + x] = value;
            }
            grid
        };
        let first = record.record == 0;
        let mut file = if first {
            let mut file = netcdf::create(&path)
                .with_context(|| format!("cannot create {}", path.display()))?;
            file.add_unlimited_dimension("time")?;
            file.add_dimension("lat_ucat", nlat)?;
            file.add_dimension("lon_ucat", nlon)?;
            file.add_variable::<f64>("lat_ucat", &["lat_ucat"])?
                .put_values(&self.lat, ..)?;
            file.add_variable::<f64>("lon_ucat", &["lon_ucat"])?
                .put_values(&self.lon, ..)?;
            let mut time = file.add_variable::<i32>("time", &["time"])?;
            time.put_attribute("long_name", "time")?;
            time.put_attribute("units", "minutes since 1900-1-1 0:0:0")?;
            let mut window = file.add_variable::<f64>("history_window_seconds", &["time"])?;
            window.put_attribute("units", "s")?;
            window.put_attribute(
                "long_name",
                "elapsed window ending at history_window_end_minutes; terminal and resumed records can overlap",
            )?;
            file.add_variable::<f64>("history_window_end_minutes", &["time"])?
                .put_attribute("units", "minutes since 1900-1-1 0:0:0")?;
            let mut mask =
                file.add_variable::<f64>("mask_complete_upstream", &["lat_ucat", "lon_ucat"])?;
            mask.put_attribute("missing_value", SPVAL)?;
            mask.put_attribute(
                "long_name",
                "Mask of grids with all upstream located in simulation region",
            )?;
            mask.put_attribute("units", "100%")?;
            mask.put_values(&to_grid(&self.allups_mask), ..)?;
            for (name, long_name, units, _) in fields {
                let mut variable =
                    file.add_variable::<f64>(name, &["time", "lat_ucat", "lon_ucat"])?;
                variable.put_attribute("missing_value", SPVAL)?;
                variable.put_attribute("long_name", *long_name)?;
                variable.put_attribute("units", *units)?;
            }
            file
        } else {
            netcdf::append(&path).with_context(|| format!("cannot reopen {}", path.display()))?
        };
        let t = record.record;
        let label = i32::try_from(record.label_minutes).context("history label overflows i32")?;
        file.variable_mut("time")
            .context("time disappeared")?
            .put_values(&[label], t..t + 1)?;
        file.variable_mut("history_window_seconds")
            .context("history_window_seconds disappeared")?
            .put_values(&[window_seconds], t..t + 1)?;
        let minutes = colm_hist::time::minutes_from_1900(end.year)
            + (i64::from(end.julian_day) - 1) * 1440
            + i64::from(end.seconds / 60);
        let end_minutes = minutes as f64 + f64::from(end.seconds % 60) / 60.0;
        file.variable_mut("history_window_end_minutes")
            .context("history_window_end_minutes disappeared")?
            .put_values(&[end_minutes], t..t + 1)?;
        for (name, _, _, values) in fields {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared"))?
                .put_values(&to_grid(values), (t..t + 1, .., ..))?;
        }
        Ok(())
    }
}

/// `get_sumarea (sumarea, filter)`：过滤后的 patch 份面积逐份相加。
fn sumarea(grid: &HistoryGrid, filter: &[bool]) -> Vec<f64> {
    let mut area = vec![0.0; grid.lat.len() * grid.lon.len()];
    for (parts, &keep) in grid.parts.iter().zip(filter) {
        if !keep {
            continue;
        }
        for &(cell, part) in parts {
            area[cell] += part;
        }
    }
    area
}

/// `flux_map_and_write_2d`：`pset2grid`（`total` 模式先除以 patch 的全部份面积）再除以分母；
/// 分母不超过 `1e-5` 的格子写 `spval`。
fn aggregate(
    grid: &HistoryGrid,
    values: &[f64],
    filter: &[bool],
    denominator: &[f64],
    total: bool,
) -> Vec<f64> {
    let mut sum = vec![SPVAL; denominator.len()];
    for (patch, (parts, &value)) in grid.parts.iter().zip(values).enumerate() {
        if value == SPVAL || !filter[patch] {
            continue;
        }
        let weight = if total && !parts.is_empty() {
            grid.patch_area[patch]
        } else {
            1.0
        };
        for &(cell, part) in parts {
            let term = value / weight * part;
            sum[cell] = if sum[cell] == SPVAL {
                term
            } else {
                sum[cell] + term
            };
        }
    }
    sum.iter()
        .zip(denominator)
        .map(|(&value, &area)| {
            if area > 0.00001 {
                if value != SPVAL {
                    value / area
                } else {
                    value
                }
            } else {
                SPVAL
            }
        })
        .collect()
}

/// `allups_mask_ucat`：单元流域落在区域里（份面积大于 0）且全部上游都已为 1 时为 1。
fn complete_upstream_mask(network: &RiverNetwork, routing: &RunoffRouting) -> Vec<f64> {
    let n = network.len();
    let mut mask = vec![0.0; n];
    // 拓扑序：一个单元流域在它的全部上游之后。
    let mut pending = network.upstream.iter().map(Vec::len).collect::<Vec<_>>();
    let mut ready = (0..n).filter(|&i| pending[i] == 0).collect::<Vec<_>>();
    let mut complete = vec![0usize; n];
    while let Some(i) = ready.pop() {
        if routing.catchment_area[i] > 0.0 && complete[i] == network.upstream[i].len() {
            mask[i] = 1.0;
        }
        let next = network.next[i];
        if next >= 0 {
            let j = next as usize;
            if mask[i] == 1.0 {
                complete[j] += 1;
            }
            pending[j] -= 1;
            if pending[j] == 0 {
                ready.push(j);
            }
        }
    }
    mask
}
