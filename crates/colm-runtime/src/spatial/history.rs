//! 空间算例的网格 history（`HistForm = 'Gridded'`，`MOD_HistGridded`）。
//!
//! 会话、累加与区间平均与单点完全相同（[`crate::history::HistorySession`]）；不同的只有两处：
//! 写文件时按 `mp2g_hist` 的面积权重把 patch 聚合到 history 网格（[`HistoryGrid`]），以及
//! 近地面诊断先在每个网格元里按 patch 面积份额聚合一次（`MOD_Vars_1DAccFluxes.F90:2696-2805`）。

use std::ops::Range;

use anyhow::{ensure, Context, Result};
use colm_hist::history::HistoryGrid;

use super::grid::{areaquad, GridBounds, LatLonGrid};
use super::mapping::AreaWeightedMapping;
use super::topology::SpatialTopology;

/// `DEF_HIST_lon_res`/`DEF_HIST_lat_res`/`DEF_HIST_grid_as_forcing` 与 `DEF_domain`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryGridConfig {
    pub lon_res: f64,
    pub lat_res: f64,
    pub as_forcing: bool,
    pub domain: GridBounds,
    /// `DEF_HIST_CompressLevel`（0..=9）：逐时间 history 量的 deflate 级别。
    pub compress_level: u8,
}

impl HistoryGridConfig {
    pub fn read(case: &colm_namelist::Document) -> Result<Self> {
        Ok(Self {
            lon_res: crate::required_real(case, "DEF_HIST_lon_res")?,
            lat_res: crate::required_real(case, "DEF_HIST_lat_res")?,
            as_forcing: crate::required_bool(case, "DEF_HIST_grid_as_forcing")?,
            domain: GridBounds {
                south: crate::required_real(case, "DEF_domain%edges")?,
                north: crate::required_real(case, "DEF_domain%edgen")?,
                west: crate::required_real(case, "DEF_domain%edgew")?,
                east: crate::required_real(case, "DEF_domain%edgee")?,
            },
            compress_level: hist_compress_level(case)?,
        })
    }
}

/// `DEF_HIST_CompressLevel`：网格、向量、示踪物与 unitcat history 的 deflate 级别（netCDF 只认 0..=9）。
pub fn hist_compress_level(case: &colm_namelist::Document) -> Result<u8> {
    let level = crate::required_integer(case, "DEF_HIST_CompressLevel")?;
    ensure!(
        (0..=9).contains(&level),
        "DEF_HIST_CompressLevel must be within 0..=9, got {level}"
    );
    Ok(level as u8)
}

/// 建 history 网格：`ghist` 的窗口、每个 patch 落在窗口里的份（`(格子, 面积)`，映射顺序）与
/// 只写一次的静态场。
///
/// 静态场都是 `get_sumarea` 的结果（从 0 起、按 patch 与份的顺序普通相加）：
/// `landarea` 的过滤是 `patchtype < 99 .and. patchmask`，`landfraction` 不过滤、再除以格子面积
/// （`block_data_division`：面积不为正写 `spval`），`area_wetland` 是 `patchtype == 2 .and. patchmask`，
/// `area_lake` 是 `patchtype == 4`（`MOD_Hist.F90:4499` 没有与 `patchmask`）。
///
/// `crop_classes` 是 CROP 内核下每个 patch 的 `patchclass`（不开 CROP 时为 `None`）：
/// 多写 `croparea`（`patchclass == 12 .and. patchmask`，`MOD_Hist.F90:424-445`）与 `irrigarea`。
/// `irrigated` 是 `DEF_USE_IRRIGATION` 时每个 patch 的 `filter_irrig`（`patchclass == 12` 且首个 PFT
/// `>= npcropmin` 且为偶数，即灌溉型作物）；不开灌溉时 `filter_irrig` 全假，写 0（上游原来用未初始化的
/// 值，upstream-bugs 第 37 条，vendor 已修）。
#[allow(clippy::too_many_arguments)]
pub fn build_history_grid(
    config: &HistoryGridConfig,
    forcing_grid: &LatLonGrid,
    topology: &SpatialTopology,
    patch_types: &[i32],
    patch_mask: &[bool],
    forcing_mask: &[bool],
    crop_classes: Option<&[usize]>,
    irrigated: Option<&[bool]>,
) -> Result<HistoryGrid> {
    let patches = topology.patch_count();
    ensure!(
        patch_types.len() == patches
            && patch_mask.len() == patches
            && forcing_mask.len() == patches,
        "one patch type, one patch mask and one forcing mask per patch are needed"
    );
    // 过滤里的 `patchmask` 只在 `DEF_URBAN_ONLY` 与 2m WMO 虚拟 patch 时为假；这两样都没移植。
    ensure!(
        patch_mask.iter().all(|&mask| mask),
        "the Rust gridded history assumes every patch is unmasked (no DEF_URBAN_ONLY or 2m WMO patches)"
    );
    // 各静态面积的 `filter` 在强迫有缺测时都与上 `forcmask_pch`（`MOD_Hist.F90:399/429/462/874/4501`）；
    // 没缺测时 `forcing_mask` 全真。
    let patch_mask = &patch_mask
        .iter()
        .zip(forcing_mask)
        .map(|(&mask, &active)| mask && active)
        .collect::<Vec<_>>();
    let grid = if config.as_forcing {
        forcing_grid.clone()
    } else {
        LatLonGrid::define_by_res(config.lon_res, config.lat_res)?
    };
    let (rows, columns) = grid.domain_window(config.domain)?;
    let nlon = columns.len();
    let mut window = std::collections::HashMap::new();
    for (row, &ilat) in rows.iter().enumerate() {
        for (column, &ilon) in columns.iter().enumerate() {
            window.insert((ilat, ilon), row * nlon + column);
        }
    }
    let mapping = AreaWeightedMapping::build(
        &grid,
        &topology.pixel,
        &topology.cells,
        &topology.shared_fraction,
    )?;
    let parts = mapping
        .parts
        .iter()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    window
                        .get(&(part.ilat, part.ilon))
                        .map(|&cell| (cell, part.area))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let cells = rows.len() * nlon;
    let sumarea = |keep: &dyn Fn(usize) -> bool| {
        let mut area = vec![0.0; cells];
        for (patch, parts) in parts.iter().enumerate() {
            if !keep(patch) {
                continue;
            }
            for &(cell, part) in parts {
                area[cell] += part;
            }
        }
        area
    };
    let landarea = sumarea(&|patch| patch_types[patch] < 99 && patch_mask[patch]);
    let everything = sumarea(&|_| true);
    let mut landfraction = vec![0.0; cells];
    for (row, &ilat) in rows.iter().enumerate() {
        for (column, &ilon) in columns.iter().enumerate() {
            let cell = row * nlon + column;
            let area = areaquad(
                grid.lat_s[ilat],
                grid.lat_n[ilat],
                grid.lon_w[ilon],
                grid.lon_e[ilon],
            );
            landfraction[cell] = if area > 0.0 {
                everything[cell] / area
            } else {
                colm_core::MISSING
            };
        }
    }
    let area_wetland = sumarea(&|patch| patch_types[patch] == 2 && patch_mask[patch]);
    let area_lake = sumarea(&|patch| patch_types[patch] == 4 && forcing_mask[patch]);
    let mut first_record_statics = Vec::new();
    if let Some(classes) = crop_classes {
        ensure!(
            classes.len() == patches,
            "one patch class per patch is needed for croparea"
        );
        first_record_statics.push((
            "croparea".to_owned(),
            "crop area".to_owned(),
            "km2".to_owned(),
            sumarea(&|patch| classes[patch] == 12 && patch_mask[patch]),
        ));
        first_record_statics.push((
            "irrigarea".to_owned(),
            "irrigation area".to_owned(),
            "km2".to_owned(),
            match irrigated {
                Some(irrigated) => {
                    ensure!(
                        irrigated.len() == patches,
                        "one irrigation flag per patch is needed for irrigarea"
                    );
                    sumarea(&|patch| irrigated[patch] && patch_mask[patch])
                }
                None => vec![0.0; cells],
            },
        ));
    }
    let mut lon = Vec::with_capacity(nlon);
    for &ilon in &columns {
        let (west, east) = (grid.lon_w[ilon], grid.lon_e[ilon]);
        let mut center = (west + east) * 0.5;
        if west > east {
            center = super::grid::normalize_longitude(center + 180.0)?;
        }
        // 跨日界线时往东接着数（`set_grid_concat`）。
        if let Some(&previous) = lon.last() {
            if center < previous && center < 0.0 {
                center += 360.0;
            }
        }
        lon.push(center);
    }
    let text = |value: &str| value.to_owned();
    Ok(HistoryGrid {
        lat: rows
            .iter()
            .map(|&ilat| (grid.lat_s[ilat] + grid.lat_n[ilat]) * 0.5)
            .collect(),
        lon,
        lat_s: rows.iter().map(|&ilat| grid.lat_s[ilat]).collect(),
        lat_n: rows.iter().map(|&ilat| grid.lat_n[ilat]).collect(),
        lon_w: columns.iter().map(|&ilon| grid.lon_w[ilon]).collect(),
        lon_e: columns.iter().map(|&ilon| grid.lon_e[ilon]).collect(),
        patch_area: mapping
            .parts
            .iter()
            .map(|parts| parts.iter().fold(0.0, |sum, part| sum + part.area))
            .collect(),
        parts,
        statics: vec![
            (text("landarea"), text("land area"), text("km2"), landarea),
            (
                text("landfraction"),
                text("land fraction"),
                text("-"),
                landfraction,
            ),
            (
                text("area_wetland"),
                text("area of wetland"),
                text("km2"),
                area_wetland,
            ),
            (
                text("area_lake"),
                text("area of lake"),
                text("km2"),
                area_lake,
            ),
        ],
        first_record_statics,
        compress_level: config.compress_level,
    })
}

/// 网格元与其 patch：`elm_patch%substt/subend` 与面积份额 `subfrc`（`MOD_Pixelset.F90:subset_build`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ElementGroups {
    /// 每个网格元的 patch 下标区间（patch 按网格元连续排列）。
    pub ranges: Vec<Range<usize>>,
    /// 每个 patch 在所属网格元里的面积份额。
    pub fractions: Vec<f64>,
}

impl ElementGroups {
    /// `subfrc`：patch 各像元的 `areaquad` 从 0 起相加、乘 `pctshared`（`has_shared` 时），
    /// 再除以本网格元的和（同样从 0 起）。
    pub fn from_topology(topology: &SpatialTopology) -> Result<Self> {
        let pixel = &topology.pixel;
        let areas = topology
            .cells
            .iter()
            .zip(&topology.shared_fraction)
            .map(|(cells, &shared)| {
                cells
                    .iter()
                    .try_fold(0.0, |sum, &(ilon, ilat)| {
                        let x = usize::try_from(ilon - 1).context("pixel longitude index")?;
                        let y = usize::try_from(ilat - 1).context("pixel latitude index")?;
                        Ok::<f64, anyhow::Error>(
                            sum + areaquad(
                                pixel.lat_s[y],
                                pixel.lat_n[y],
                                pixel.lon_w[x],
                                pixel.lon_e[x],
                            ),
                        )
                    })
                    .map(|area| area * shared)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut ranges = Vec::new();
        let mut start = 0;
        for index in 1..=topology.element.len() {
            if index == topology.element.len() || topology.element[index] != topology.element[start]
            {
                ranges.push(start..index);
                start = index;
            }
        }
        let mut fractions = areas.clone();
        for range in &ranges {
            let total = areas[range.clone()]
                .iter()
                .fold(0.0, |sum, area| sum + area);
            ensure!(total > 0.0, "an element has no pixel area");
            for fraction in &mut fractions[range.clone()] {
                *fraction /= total;
            }
        }
        Ok(Self { ranges, fractions })
    }
}

/// 空间主循环的 history：会话加网格元分组。
pub struct SpatialHistory {
    pub session: crate::history::HistorySession,
    /// 河道 history（`GridRiverLakeFlow`）：记录时刻写 unitcat 文件与网格里的河道量。
    pub river: Option<crate::river::history::RiverHistoryWriter>,
    pub elements: ElementGroups,
    /// 已写出的 history 文件。
    pub files: Vec<std::path::PathBuf>,
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
