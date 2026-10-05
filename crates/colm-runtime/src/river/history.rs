//! 河道 history（`MOD_Grid_RiverLakeHist` 与 `MOD_Hist.F90:4749-4790`，`DEF_HIST_mode = 'one'`）。
//!
//! 每条 history 记录写两处：
//! - `<case>_hist_unitcat_<suffix>.nc`：单元流域网格（`lon_ucat`×`lat_ucat`，截到模拟范围）上的 12 个量，
//!   单元流域的值铺在它的 `(seq_x, seq_y)`，其余是 `spval`；
//! - 网格 history 里的 6 个量与静态的 `mask_complete_upstream_regird`：先把单元流域的值经
//!   `push_ucat2grid`/`push_ucat2inpm` 与 `remap_patch2inpm` 回到 patch，再按各自的过滤与分母聚合。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use colm_core::tracer::TRC_TINY;
use colm_hist::history::{set_history_compression, set_window_chunking, HistoryGrid};
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

/// 向量 history（非结构网格）里由河道写出的量：`(闸门表名, input_mode = 'total')`。
/// 向量写出不写 `f_floodarea`（`MOD_Hist.F90:4783` 只在 `Gridded` 时写）。
pub const VECTOR_RIVER_VARIABLES: [(&str, bool); 5] = [
    ("wdpth_ucat_regrid", false),
    ("veloc_riv_regrid", false),
    ("discharge", true),
    ("discharge_rivermouth_regrid", true),
    ("floodfrc", false),
];

/// 河道 history 的静态部分（`hist_grid_riverlake_init`）与 unitcat 文件的落点。
pub struct RiverHistoryWriter {
    /// 网格 history；向量 history 时为 `None`（河道量逐 patch 交给会话聚合到单元）。
    grid: Option<Arc<HistoryGrid>>,
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
    /// unitcat 文件的输出窗口（`route_hist_window`）：格心落在模拟范围里的 15′ 格子，0 起的
    /// `(x0, y0, nlon, nlat)`，与陆面 history 对齐。河系照旧整条汇流，窗口外的只是不写。
    window: (usize, usize, usize, usize),
    /// 全球 15′ 网格的格心（窗口按它截取）。
    lon_all: Vec<f64>,
    lat_all: Vec<f64>,
    /// 开水库时的 `dam_GRAND_ID`（按水库序号），建 unitcat 文件骨架时写成 `resv_GRAND_ID`。
    pub reservoir_ids: Option<Vec<i32>>,
    /// `DEF_HIST_CompressLevel`：unitcat 文件里逐时间量（河道量、分汊矩阵、示踪物/泥沙量、水库量）
    /// 的 deflate 级别（`route_hist_write_*` → `ncio_write_serial_time(..., DEF_HIST_CompressLevel)`）。
    pub hist_compress_level: u8,
    /// `DEF_REST_CompressLevel`：无时间维的 `mask_complete_upstream` 走
    /// `vector_gather_map2grid_and_write` 的无 `itime` 分支，上游在那里传的是重启文件的级别
    /// （`MOD_Vector_ReadWrite.F90:357-358`）。
    pub rest_compress_level: u8,
}

/// `route_hist_window`：格心严格落在 `(west, east, south, north)` 里的连续下标段（`lat` 自北向南）。
/// 跨日界线（`west >= east`）或一个格子都没有时取整张网格。
fn domain_window(
    lon: &[f64],
    lat: &[f64],
    bounds: (f64, f64, f64, f64),
) -> (usize, usize, usize, usize) {
    let (west, east, south, north) = bounds;
    let full = (0, 0, lon.len(), lat.len());
    if west >= east {
        return full;
    }
    let span = |values: &[f64], low: f64, high: f64| {
        let inside: Vec<usize> = (0..values.len())
            .filter(|&i| values[i] > low && values[i] < high)
            .collect();
        Some((*inside.first()?, *inside.last()? - inside.first()? + 1))
    };
    match (span(lon, west, east), span(lat, south, north)) {
        (Some((x0, nx)), Some((y0, ny))) => (x0, y0, nx, ny),
        _ => full,
    }
}

/// `worker_remap_data_grid2pset`（填充值 `spval`）：`average` 除以非缺测份的面积和，`sum` 不除。
fn grid_to_patches(routing: &RunoffRouting, grid: &[f64], average: bool) -> Vec<f64> {
    super::remap::grid_to_patches(routing, grid, SPVAL, average)
}

/// `push_ucat2grid`：每个区域网格取格心在其中的单元流域的值。
fn catchments_to_grid(routing: &RunoffRouting, values: &[f64]) -> Vec<f64> {
    routing
        .ucat_at_grid
        .iter()
        .map(|ucat| ucat.map_or(SPVAL, |i| values[i]))
        .collect()
}

/// `push_ucat2inpm`（`average`，填充值 `spval`）：按单元流域的份面积加权平均，跳过缺测。
fn catchments_to_inpm(routing: &RunoffRouting, values: &[f64]) -> Vec<f64> {
    super::remap::catchments_to_inpm(routing, values, SPVAL, true)
}

/// `worker_remap_data_pset2grid`（填充值 `spval`）：过滤掉的与缺测的 patch 不参与。
fn patches_to_grid(routing: &RunoffRouting, values: &[f64], filter: &[bool]) -> Vec<f64> {
    super::remap::patches_to_grid(routing, values, filter, SPVAL)
}

impl RiverHistoryWriter {
    /// `hist_grid_riverlake_init`。`basic[p]` 是 `patchtype < 99 .and. patchmask`。
    pub fn new(
        network: &RiverNetwork,
        routing: &RunoffRouting,
        grid: Option<Arc<HistoryGrid>>,
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
        let (sumarea_ucat, sumarea_inpm) = match grid.as_deref() {
            Some(grid) => (sumarea(grid, &filter_ucat), sumarea(grid, &filter_inpm)),
            None => (Vec::new(), Vec::new()),
        };
        let allups_mask = complete_upstream_mask(network, routing);
        let lon_count = network.nlon;
        let lat_count = network.nlat;
        let ucat_grid = crate::spatial::grid::LatLonGrid::define_by_ndims(lon_count, lat_count)?;
        let lat: Vec<f64> = ucat_grid
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
        let window = (0, 0, lon.len(), lat.len());
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
            lon_all: lon.clone(),
            lat_all: lat.clone(),
            lon,
            lat,
            window,
            reservoir_ids: None,
            // namelist 默认值（`MOD_Namelist.F90:750-751`）；调用方按算例覆盖。
            hist_compress_level: 1,
            rest_compress_level: 1,
        })
    }

    /// unitcat 文件只写模拟范围（`DEF_domain`）里的格子，与陆面 history 对齐。
    pub fn with_domain(mut self, bounds: (f64, f64, f64, f64)) -> Self {
        let window = domain_window(&self.lon_all, &self.lat_all, bounds);
        let (x0, y0, nx, ny) = window;
        self.lon = self.lon_all[x0..x0 + nx].to_vec();
        self.lat = self.lat_all[y0..y0 + ny].to_vec();
        self.window = window;
        self
    }

    /// 向量 history 的 `mask_complete_upstream_regird`：逐 patch 的掩码与 `filter_ucat`
    /// （`aggregate_to_vector_and_write_2d(allups_mask_pch, …, -1, filter_ucat)`）。
    pub fn upstream_mask_patches(
        network: &RiverNetwork,
        routing: &RunoffRouting,
        basic: &[bool],
    ) -> (Vec<f64>, Vec<bool>) {
        let n = network.len();
        let covered = grid_to_patches(routing, &catchments_to_grid(routing, &vec![1.0; n]), true);
        let filter_ucat = basic
            .iter()
            .zip(&covered)
            .map(|(&keep, &value)| keep && value != SPVAL)
            .collect::<Vec<_>>();
        let mask = complete_upstream_mask(network, routing);
        let mask_patch = grid_to_patches(routing, &catchments_to_grid(routing, &mask), true);
        (mask_patch, filter_ucat)
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
    #[allow(clippy::too_many_arguments)]
    pub fn write_record(
        &self,
        network: &RiverNetwork,
        routing: &RunoffRouting,
        history: &mut RiverHistory,
        tracers: Option<&mut super::tracer::RiverTracers>,
        sediment: Option<&mut super::sediment::Sediment>,
        record: &ScheduledRecord,
        end: colm_core::CalendarTime,
        session: &mut crate::history::HistorySession,
    ) -> Result<()> {
        let n = network.len();
        let mut tracer_fields = tracers
            .as_deref()
            .map(|tracers| {
                self.tracer_fields(tracers, history.levsto.is_some(), history.bifout.is_some())
            })
            .unwrap_or_default();
        // `tracer_lifecycle_route_write_history`：泥沙的 unitcat 量（`write_sediment_history`）。
        if let Some(sediment) = sediment.as_deref() {
            tracer_fields.extend(sediment.history_fields());
        }
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
        let mut unitcat = vec![
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
        // `DEF_USE_LEVEE`：`WHERE (acctime_ucat > 0) a/acctime ELSEWHERE 0`。
        if let (Some(levsto), Some(levdph)) = (&history.levsto, &history.levdph) {
            let mean_or_zero = |values: &[f64]| {
                values
                    .iter()
                    .zip(&history.acctime)
                    .map(|(&v, &t)| if t > 0.0 { v / t } else { 0.0 })
                    .collect::<Vec<_>>()
            };
            unitcat.push((
                "f_levsto",
                "water storage in levee-protected area",
                "m^3",
                mean_or_zero(levsto),
            ));
            unitcat.push((
                "f_levdph",
                "water depth in levee-protected area",
                "m",
                mean_or_zero(levdph),
            ));
        }
        // `DEF_USE_BIFURCATION`：`f_bifout` 与路径层矩阵 `f_bifflw_lev`（逐路径按自己的累加时长平均）。
        let mut bifflw = None;
        if let (Some(bifout), Some(lev), Some(acctime)) = (
            &history.bifout,
            &history.bifflw_lev,
            &history.bifflw_acctime,
        ) {
            unitcat.push((
                "f_bifout",
                "net bifurcation outflow",
                "m^3/s",
                bifout
                    .iter()
                    .zip(&history.acctime)
                    .map(|(&v, &t)| if t > 0.0 { v / t } else { 0.0 })
                    .collect(),
            ));
            let levels = lev.len() / acctime.len().max(1);
            let mean = lev
                .chunks(levels.max(1))
                .zip(acctime)
                .flat_map(|(row, &t)| row.iter().map(move |&v| if t > 0.0 { v / t } else { 0.0 }))
                .collect::<Vec<_>>();
            bifflw = Some((levels, mean));
        }
        // 水库（`totalnumresv > 0`）：逐水库按 `acctime_resv` 平均，没累加时为 `spval`。
        let mut reservoirs = Vec::new();
        if let Some(acctime) = history.acctime_resv.as_ref().filter(|a| !a.is_empty()) {
            for (name, long_name, units, values) in [
                ("volresv", "reservoir water volume", "m^3", &history.volresv),
                ("qresv_in", "reservoir inflow", "m^3/s", &history.qresv_in),
                (
                    "qresv_out",
                    "reservoir outflow",
                    "m^3/s",
                    &history.qresv_out,
                ),
            ] {
                let mean = values
                    .as_ref()
                    .expect("reservoir history")
                    .iter()
                    .zip(acctime)
                    .map(|(&v, &t)| if t > 0.0 { v / t } else { SPVAL })
                    .collect::<Vec<_>>();
                reservoirs.push((name, long_name, units, mean));
            }
        }
        self.write_unitcat(
            network,
            record,
            end,
            window_seconds,
            &unitcat,
            bifflw,
            &tracer_fields,
            &reservoirs,
        )?;

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
        let Some(grid) = self.grid.clone() else {
            // 向量 history：逐 patch 交给会话，按单元聚合（`aggregate_to_vector_and_write_2d`）。
            for (name, values, filter) in [
                ("wdpth_ucat_regrid", wdsrf_pch, &self.filter_ucat),
                ("veloc_riv_regrid", veloc_pch, &self.filter_ucat),
                ("discharge", discharge_pch, &self.filter_ucat),
                ("discharge_rivermouth_regrid", mouth_pch, &self.filter_ucat),
                ("floodfrc", floodfrc_pch, &self.filter_inpm),
            ] {
                session.stage_patch_field(name, values, filter.clone())?;
            }
            history.reset();
            if let Some(tracers) = tracers {
                tracers.history.reset();
            }
            if let Some(sediment) = sediment {
                sediment.flush_history();
            }
            return Ok(());
        };
        let ones = vec![1.0; grid.lat.len() * grid.lon.len()];
        let fields = [
            (
                "wdpth_ucat_regrid",
                aggregate(
                    &grid,
                    &wdsrf_pch,
                    &self.filter_ucat,
                    &self.sumarea_ucat,
                    false,
                ),
            ),
            (
                "veloc_riv_regrid",
                aggregate(
                    &grid,
                    &veloc_pch,
                    &self.filter_ucat,
                    &self.sumarea_ucat,
                    false,
                ),
            ),
            (
                "discharge",
                aggregate(&grid, &discharge_pch, &self.filter_ucat, &ones, true),
            ),
            (
                "discharge_rivermouth_regrid",
                aggregate(&grid, &mouth_pch, &self.filter_ucat, &ones, true),
            ),
            (
                "floodfrc",
                aggregate(
                    &grid,
                    &floodfrc_pch,
                    &self.filter_inpm,
                    &self.sumarea_inpm,
                    false,
                ),
            ),
            (
                "floodarea",
                aggregate(&grid, &floodfrc_pch, &self.filter_inpm, &ones, false),
            ),
        ];
        for (name, values) in fields {
            session.stage_gridded(name, values)?;
        }
        debug_assert_eq!(history.acctime.len(), n);
        history.reset();
        // `flush_acc_fluxes_riverlake` 同时清零示踪物累加（`tracer_flush_acc`）。
        if let Some(tracers) = tracers {
            tracers.history.reset();
        }
        // `tracer_lifecycle_route_flush_history`。
        if let Some(sediment) = sediment {
            sediment.flush_history();
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_unitcat(
        &self,
        network: &RiverNetwork,
        record: &ScheduledRecord,
        end: colm_core::CalendarTime,
        window_seconds: f64,
        fields: &[(&str, &str, &str, Vec<f64>)],
        bifflw: Option<(usize, Vec<f64>)>,
        tracer_fields: &[(String, String, String, Vec<f64>)],
        reservoirs: &[(&str, &str, &str, Vec<f64>)],
    ) -> Result<()> {
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create {}", self.directory.display()))?;
        let path = self
            .directory
            .join(format!("{}_hist_unitcat_{}.nc", self.stem, record.suffix));
        let (x0, y0, nlon, nlat) = self.window;
        let to_grid = |values: &[f64]| {
            let mut grid = vec![SPVAL; nlon * nlat];
            for (i, &value) in values.iter().enumerate() {
                // 窗口外的单元流域照常汇流，只是不写。
                let (x, y) = (network.x[i] as usize - 1, network.y[i] as usize - 1);
                if (x0..x0 + nlon).contains(&x) && (y0..y0 + nlat).contains(&y) {
                    grid[(y - y0) * nlon + (x - x0)] = value;
                }
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
            set_window_chunking(&mut window)?;
            window.put_attribute("units", "s")?;
            window.put_attribute(
                "long_name",
                "elapsed window ending at history_window_end_minutes; terminal and resumed records can overlap",
            )?;
            let mut end = file.add_variable::<f64>("history_window_end_minutes", &["time"])?;
            set_window_chunking(&mut end)?;
            end.put_attribute("units", "minutes since 1900-1-1 0:0:0")?;
            let mut mask =
                file.add_variable::<f64>("mask_complete_upstream", &["lat_ucat", "lon_ucat"])?;
            set_history_compression(&mut mask, self.rest_compress_level)?;
            mask.put_attribute("missing_value", SPVAL)?;
            mask.put_attribute(
                "long_name",
                "Mask of grids with all upstream located in simulation region",
            )?;
            mask.put_attribute("units", "100%")?;
            mask.put_values(&to_grid(&self.allups_mask), ..)?;
            // 水库轴属于文件骨架（`route_hist_begin`）：`reservoir` 维与 `resv_GRAND_ID`。
            if let Some(ids) = self.reservoir_ids.as_ref().filter(|ids| !ids.is_empty()) {
                file.add_dimension("reservoir", ids.len())?;
                let mut variable = file.add_variable::<i32>("resv_GRAND_ID", &["reservoir"])?;
                variable.put_values(ids, ..)?;
                variable.put_attribute("long_name", "reservoir GRAND ID")?;
            }
            for (name, long_name, units, _) in fields {
                let mut variable =
                    file.add_variable::<f64>(name, &["time", "lat_ucat", "lon_ucat"])?;
                set_history_compression(&mut variable, self.hist_compress_level)?;
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
        // `route_hist_write_bif_matrix`（单文件）：`ncio_write_serial_time`，属性只在第一条记录写。
        if let Some((levels, values)) = bifflw {
            let paths = values.len() / levels.max(1);
            if first {
                file.add_dimension("bifurcation_level", levels)?;
                file.add_dimension("bifurcation_pathway", paths)?;
                let mut variable = file.add_variable::<f64>(
                    "f_bifflw_lev",
                    &["time", "bifurcation_pathway", "bifurcation_level"],
                )?;
                set_history_compression(&mut variable, self.hist_compress_level)?;
                variable.put_attribute("long_name", "effective bifurcation pathway-layer flow")?;
                variable.put_attribute("units", "m^3/s")?;
            }
            file.variable_mut("f_bifflw_lev")
                .context("f_bifflw_lev disappeared")?
                .put_values(&values, (t..t + 1, .., ..))?;
        }
        // `write_tracer_history`：二维 unitcat 量（属性同主量），在分汊矩阵之后、水库之前定义。
        for (name, long_name, units, values) in tracer_fields {
            if first {
                let mut variable =
                    file.add_variable::<f64>(name, &["time", "lat_ucat", "lon_ucat"])?;
                set_history_compression(&mut variable, self.hist_compress_level)?;
                variable.put_attribute("missing_value", SPVAL)?;
                variable.put_attribute("long_name", long_name.as_str())?;
                variable.put_attribute("units", units.as_str())?;
            }
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared"))?
                .put_values(&to_grid(values), (t..t + 1, .., ..))?;
        }
        // `route_hist_write_resv`（单文件）：`vector_gather_and_write` 到 `(time, reservoir)`。
        for (name, long_name, units, values) in reservoirs {
            if first {
                if file.dimension("reservoir").is_none() {
                    file.add_dimension("reservoir", values.len())?;
                }
                let mut variable = file.add_variable::<f64>(name, &["time", "reservoir"])?;
                set_history_compression(&mut variable, self.hist_compress_level)?;
                variable.put_attribute("long_name", *long_name)?;
                variable.put_attribute("units", *units)?;
                variable.put_attribute("missing_value", SPVAL)?;
            }
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared"))?
                .put_values(values, (t..t + 1, ..))?;
        }
        Ok(())
    }
}

impl RiverHistoryWriter {
    /// `write_tracer_history`：每个输运示踪物的浓度（水量加权）、同位素 δ、出流，开堤防时
    /// 堤内储量与其 δ（`trc_hist_fp_dust = 1e-12` 的显示清理只作用于输出）。
    fn tracer_fields(
        &self,
        tracers: &super::tracer::RiverTracers,
        levee: bool,
        bifurcation: bool,
    ) -> Vec<(String, String, String, Vec<f64>)> {
        const DUST: f64 = 1.0e-12;
        const DELTA_VMIN: f64 = 1.0;
        const SANITY: f64 = 2.0e3;
        let dust = |v: Vec<f64>| -> Vec<f64> {
            v.into_iter()
                .map(|x| if x.abs() < DUST { 0.0 } else { x })
                .collect()
        };
        let h = &tracers.history;
        let n = h.acctime.len();
        let mut out = Vec::new();
        for itrc in tracers.set.transport_indices() {
            let tracer = &tracers.set.tracers[itrc];
            let name = tracer.name.trim();
            let delta = tracer.is_isotope();
            let (word, conc_units, mass_units, flux_units) = if delta {
                ("ratio", "R".to_owned(), "R*m3", "R*m3/s")
            } else {
                (
                    "concentration",
                    colm_core::tracer::hist::concentration_units(tracer).to_owned(),
                    "tracer",
                    "tracer/s",
                )
            };
            let ratio_delta = |mass: &[f64], water: &[f64]| -> Vec<f64> {
                (0..n)
                    .map(|i| {
                        if self.allups_mask[i] < 0.5 || h.acctime[i] <= 0.0 {
                            return SPVAL;
                        }
                        if water[i] <= DELTA_VMIN * h.acctime[i] || tracer.ref_ratio <= TRC_TINY {
                            return SPVAL;
                        }
                        let ratio = mass[i] / water[i];
                        if ratio <= TRC_TINY {
                            return SPVAL;
                        }
                        let delta = (ratio / tracer.ref_ratio - 1.0) * 1000.0;
                        if delta.abs() <= SANITY {
                            delta
                        } else {
                            SPVAL
                        }
                    })
                    .collect()
            };
            let conc = (0..n)
                .map(|i| {
                    if h.acctime[i] <= 0.0 || h.water_storage[i] <= DELTA_VMIN * h.acctime[i] {
                        SPVAL
                    } else {
                        h.storage_mass[itrc][i] / h.water_storage[i]
                    }
                })
                .collect();
            out.push((
                format!("f_trc_conc_{name}"),
                format!("tracer {word} ({name})"),
                conc_units,
                dust(conc),
            ));
            if delta {
                out.push((
                    format!("f_trc_delta_{name}"),
                    format!("tracer delta ({name})"),
                    "permil".to_owned(),
                    ratio_delta(&h.storage_mass[itrc], &h.water_storage),
                ));
            }
            let per_time = |values: &[f64]| -> Vec<f64> {
                (0..n)
                    .map(|i| {
                        if h.acctime[i] > 0.0 {
                            values[i] / h.acctime[i]
                        } else {
                            SPVAL
                        }
                    })
                    .collect()
            };
            out.push((
                format!("f_trc_flux_{name}"),
                format!("tracer outflux ({name})"),
                flux_units.to_owned(),
                dust(per_time(&h.out[itrc])),
            ));
            if levee {
                out.push((
                    format!("f_trc_levsto_{name}"),
                    format!("protected-side levee tracer storage ({name})"),
                    mass_units.to_owned(),
                    dust(per_time(&h.levsto_mass[itrc])),
                ));
                if delta {
                    out.push((
                        format!("f_trc_levdelta_{name}"),
                        format!("protected-side levee tracer delta ({name})"),
                        "permil".to_owned(),
                        ratio_delta(&h.levsto_mass[itrc], &h.levsto_water),
                    ));
                }
            }
            if bifurcation {
                out.push((
                    format!("f_trc_bifout_{name}"),
                    format!("tracer net bifurcation outflux ({name})"),
                    flux_units.to_owned(),
                    dust(per_time(&h.bifout[itrc])),
                ));
            }
        }
        out
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

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
