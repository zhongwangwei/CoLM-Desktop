//! `MOD_SpatialMapping:spatial_mapping_build_arealweighted` 与 `grid2pset`：强迫网格到 pixelset（patch）
//! 的面积加权映射。
//!
//! 每个 set 逐像元、逐与之相交的网格格子累加重叠面积 `areaquad`；格子在 set 内按 `(ilat, ilon)`
//! 有序（`insert_into_sorted_list2`），`grid2pset` 也按这个顺序累加 —— 求和顺序就是逐位的关键。

use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::spatial_grid::{
    areaquad, find_nearest_east, find_nearest_north, find_nearest_south, find_nearest_west,
    lon_between_ceil, lon_between_floor, normalize_longitude, LatLonGrid,
};

/// 一个 set 覆盖的一个网格格子（0 起下标）与重叠面积。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MappingPart {
    pub ilon: usize,
    pub ilat: usize,
    pub area: f64,
}

/// 像元轴（`pixel.nc`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PixelAxes {
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
}

/// 面积加权映射：`parts[iset]` 是 set 的格子（有序），`area[iset] = sum(areapart)`。
#[derive(Debug, Clone, PartialEq)]
pub struct AreaWeightedMapping {
    pub parts: Vec<Vec<MappingPart>>,
    pub area: Vec<f64>,
}

impl AreaWeightedMapping {
    /// `build_arealweighted (fgrid, pixelset)`。`cells[iset]` 是 set 的像元 `(ilon, ilat)`（1 起），
    /// `shared_fraction` 是 `pctshared`（`has_shared` 为假时各为 1，乘 1 不改变数值）。
    pub fn build(
        grid: &LatLonGrid,
        pixel: &PixelAxes,
        cells: &[Vec<(i32, i32)>],
        shared_fraction: &[f64],
    ) -> Result<Self> {
        ensure!(
            cells.len() == shared_fraction.len(),
            "the mapping needs one shared fraction per pixel set"
        );
        let ys = pixel
            .lat_s
            .iter()
            .map(|&lat| find_nearest_south(lat, &grid.lat_s))
            .collect::<Vec<_>>();
        let yn = pixel
            .lat_n
            .iter()
            .map(|&lat| find_nearest_north(lat, &grid.lat_n))
            .collect::<Vec<_>>();
        let xw = pixel
            .lon_w
            .iter()
            .map(|&lon| find_nearest_west(lon, &grid.lon_w))
            .collect::<Vec<_>>();
        let xe = pixel
            .lon_e
            .iter()
            .map(|&lon| find_nearest_east(lon, &grid.lon_e))
            .collect::<Vec<_>>();
        let nlon = grid.nlon();
        let mut parts = Vec::with_capacity(cells.len());
        for (set_cells, &shared) in cells.iter().zip(shared_fraction) {
            let mut list: Vec<MappingPart> = Vec::new();
            for &(px, py) in set_cells {
                ensure!(
                    px >= 1
                        && py >= 1
                        && (px as usize) <= pixel.lon_w.len()
                        && (py as usize) <= pixel.lat_s.len(),
                    "mesh pixel ({px}, {py}) is outside the pixel axes"
                );
                let ilon = (px - 1) as usize;
                let ilat = (py - 1) as usize;
                // `DO iy = ys(ilat), yn(ilat), fgrid%yinc`
                let mut iy = ys[ilat] as i64;
                let last = yn[ilat] as i64;
                let step = i64::from(grid.yinc);
                while (step > 0 && iy <= last) || (step < 0 && iy >= last) {
                    let y = iy as usize;
                    iy += step;
                    let lat_s = grid.lat_s[y].max(pixel.lat_s[ilat]);
                    let lat_n = grid.lat_n[y].min(pixel.lat_n[ilat]);
                    if (lat_n - lat_s) < 1.0e-6 {
                        continue;
                    }
                    let mut ix = xw[ilon];
                    loop {
                        let lon_w = if ix == xw[ilon] {
                            pixel.lon_w[ilon]
                        } else {
                            grid.lon_w[ix]
                        };
                        let lon_e = if ix == xe[ilon] {
                            pixel.lon_e[ilon]
                        } else {
                            grid.lon_e[ix]
                        };
                        let skip = if !(lon_between_floor(lon_w, pixel.lon_w[ilon], lon_e)
                            && lon_between_ceil(lon_e, lon_w, pixel.lon_e[ilon]))
                        {
                            true
                        } else if lon_e > lon_w {
                            (lon_e - lon_w) < 1.0e-6
                        } else {
                            (lon_e + 360.0 - lon_w) < 1.0e-6
                        };
                        if !skip {
                            let area = areaquad(lat_s, lat_n, lon_w, lon_e);
                            insert_sorted(&mut list, ix, y, area);
                        }
                        if ix == xe[ilon] {
                            break;
                        }
                        ix = (ix + 1) % nlon;
                    }
                }
            }
            // `areapart = afrac * pctshared`（只在 `has_shared` 时乘；乘 1 是恒等）。
            for part in &mut list {
                part.area *= shared;
            }
            parts.push(list);
        }
        // `areapset = sum(areapart)`：顺序累加。
        let area = parts
            .iter()
            .map(|list| list.iter().fold(0.0, |sum, part| sum + part.area))
            .collect();
        Ok(Self { parts, area })
    }

    /// `build_bilinear (fgrid, pixelset)`（`MOD_SpatialMapping.F90:537-989`，`DEF_Forcing_Interp_Method
    /// = 'bilinear'`）：每个 set 固定 4 份，依次是西北、东北、西南、东南四个格心，份面积是 set 面积
    /// （像元 `areaquad` 从 0 起相加、乘 `pctshared`）乘南北权重再乘东西权重。权重按 set 中心到两侧格心的
    /// 大圆距离反比（`arclen`）。`coordinates[iset]` 是 set 中心 `(rlon, rlat)`（弧度，即 `patchlonr/latr`）。
    ///
    /// 四份不合并：两侧取到同一个格心时（落在格心带之外、或只有一行/一列），它照样占两份、另一份权重为 0。
    ///
    /// 邻行/邻列不在 `DEF_domain` 的块覆盖里（`grid_set_blocks` 给的 `yblk/xblk = 0`，即
    /// [`LatLonGrid::domain_window`] 之外）时，该方向退化成只用覆盖内那一侧（upstream-bugs 第 42 条：
    /// 上游原来照取，`gblock%pio(xblk, 0)` 越界；vendor 已修）。
    pub fn build_bilinear(
        grid: &LatLonGrid,
        pixel: &PixelAxes,
        cells: &[Vec<(i32, i32)>],
        shared_fraction: &[f64],
        coordinates: &[(f64, f64)],
        domain: crate::spatial_grid::GridBounds,
    ) -> Result<Self> {
        ensure!(
            cells.len() == shared_fraction.len() && cells.len() == coordinates.len(),
            "the bilinear mapping needs one shared fraction and one centre per pixel set"
        );
        let nlat = grid.rlat.len();
        let nlon = grid.nlon();
        ensure!(nlat > 0 && nlon > 0, "the forcing grid is empty");
        let degrees = |radians: f64| radians * 180.0 / std::f64::consts::PI;
        let (rows, columns) = grid.domain_window(domain)?;
        let mut row_covered = vec![false; nlat];
        for row in rows {
            row_covered[row] = true;
        }
        let mut column_covered = vec![false; nlon];
        for column in columns {
            column_covered[column] = true;
        }
        let mut parts = Vec::with_capacity(cells.len());
        let mut area = Vec::with_capacity(cells.len());
        for ((set_cells, &shared), &(rlon, rlat)) in
            cells.iter().zip(shared_fraction).zip(coordinates)
        {
            // 南北：格心按纬度单调，找夹住 set 中心的两行。
            let (mut yn, mut ys) = if grid.rlat[0] > grid.rlat[nlat - 1] {
                let mut ilat = 0;
                while rlat < grid.rlat[ilat] && ilat < nlat - 1 {
                    ilat += 1;
                }
                if rlat >= grid.rlat[ilat] {
                    (ilat.saturating_sub(1), ilat)
                } else {
                    (nlat - 1, nlat - 1)
                }
            } else {
                let mut ilat = nlat - 1;
                while rlat < grid.rlat[ilat] && ilat > 0 {
                    ilat -= 1;
                }
                if rlat >= grid.rlat[ilat] {
                    ((ilat + 1).min(nlat - 1), ilat)
                } else {
                    (0, 0)
                }
            };
            if !row_covered[yn] {
                yn = ys;
            }
            if !row_covered[ys] {
                ys = yn;
            }
            let (nwgt, swgt) = if yn != ys {
                let distn = arclen(rlat, rlon, grid.rlat[yn], rlon);
                let dists = arclen(rlat, rlon, grid.rlat[ys], rlon);
                (dists / (dists + distn), distn / (dists + distn))
            } else {
                (1.0, 0.0)
            };
            // 东西：找第一个使 set 中心落在 [格心 iwest, 格心 iwest+1) 的 iwest（经度归一到 [-180, 180)）。
            let lon = normalize_longitude(degrees(rlon))?;
            let mut found = None;
            for iwest in 0..nlon {
                let lonw = normalize_longitude(degrees(grid.rlon[iwest]))?;
                let ieast = (iwest + 1) % nlon;
                let lone = normalize_longitude(degrees(grid.rlon[ieast]))?;
                if lon_between_floor(lon, lonw, lone) {
                    found = Some((iwest, ieast, lonw, lone));
                    break;
                }
            }
            let Some((iwest, ieast, lonw, lone)) = found else {
                anyhow::bail!(
                    "no forcing grid column brackets the pixel-set centre at {lon} degrees"
                );
            };
            let (mut xw, mut xe) = (iwest, ieast);
            // 区域网格最后一列与第一列之间不连通：取近的那一列，不插值。
            if iwest == nlon - 1
                && nlon > 1
                && lon_between_floor(grid.lon_e[nlon - 1], lonw, grid.lon_w[0])
            {
                let mut diffw = lon - lonw;
                if diffw < 0.0 {
                    diffw += 360.0;
                }
                let mut diffe = lone - lon;
                if diffe < 0.0 {
                    diffe += 360.0;
                }
                if diffw > diffe {
                    (xw, xe) = (ieast, ieast);
                } else {
                    (xw, xe) = (iwest, iwest);
                }
            }
            if !column_covered[xw] {
                xw = xe;
            }
            if !column_covered[xe] {
                xe = xw;
            }
            let (wwgt, ewgt) = if xw != xe {
                let distw = arclen(rlat, rlon, rlat, grid.rlon[xw]);
                let diste = arclen(rlat, rlon, rlat, grid.rlon[xe]);
                (diste / (distw + diste), distw / (distw + diste))
            } else {
                (1.0, 0.0)
            };
            let mut areathis = 0.0;
            for &(px, py) in set_cells {
                ensure!(
                    px >= 1
                        && py >= 1
                        && (px as usize) <= pixel.lon_w.len()
                        && (py as usize) <= pixel.lat_s.len(),
                    "mesh pixel ({px}, {py}) is outside the pixel axes"
                );
                let (ilon, ilat) = ((px - 1) as usize, (py - 1) as usize);
                areathis += areaquad(
                    pixel.lat_s[ilat],
                    pixel.lat_n[ilat],
                    pixel.lon_w[ilon],
                    pixel.lon_e[ilon],
                );
            }
            areathis *= shared;
            let list = vec![
                MappingPart {
                    ilon: xw,
                    ilat: yn,
                    area: areathis * nwgt * wwgt,
                },
                MappingPart {
                    ilon: xe,
                    ilat: yn,
                    area: areathis * nwgt * ewgt,
                },
                MappingPart {
                    ilon: xw,
                    ilat: ys,
                    area: areathis * swgt * wwgt,
                },
                MappingPart {
                    ilon: xe,
                    ilat: ys,
                    area: areathis * swgt * ewgt,
                },
            ];
            // `areapset = sum(areapart)`：从 0 起依次相加。
            area.push(list.iter().fold(0.0, |sum, part| sum + part.area));
            parts.push(list);
        }
        Ok(Self { parts, area })
    }

    /// `grid2pset`：`value(ilon, ilat)` 取网格值；面积为 0 的 set 得 `spval`。
    /// `set_missing_value (gdata, missing_value, pmask)`：网格值等于 `missing` 的份面积清零，
    /// `areapset` 按剩下的份从 0 起重算；返回 `pmask = areapset > 0`。
    pub fn set_missing_value(
        &mut self,
        value: impl Fn(usize, usize) -> f64,
        missing: f64,
    ) -> Vec<bool> {
        let mut mask = Vec::with_capacity(self.parts.len());
        for (parts, area) in self.parts.iter_mut().zip(&mut self.area) {
            *area = 0.0;
            for part in parts.iter_mut() {
                if value(part.ilon, part.ilat) == missing {
                    part.area = 0.0;
                } else {
                    *area += part.area;
                }
            }
            mask.push(*area > 0.0);
        }
        mask
    }

    pub fn grid_to_set(&self, iset: usize, value: impl Fn(usize, usize) -> f64) -> f64 {
        if self.area[iset] > 0.0 {
            let mut sum = 0.0;
            for part in &self.parts[iset] {
                if part.area > 0.0 {
                    // GIMPLE（latlon 内核）：`.FMA (areapart, pbuff, pdata)`。
                    sum = part.area.contract(value(part.ilon, part.ilat), sum);
                }
            }
            sum / self.area[iset]
        } else {
            colm_core::MISSING
        }
    }
}

/// `insert_into_sorted_list2`：按 `(y, x)` 升序；已有则累加面积。
fn insert_sorted(list: &mut Vec<MappingPart>, x: usize, y: usize, area: f64) {
    let key = |part: &MappingPart| (part.ilat, part.ilon);
    match list.binary_search_by(|part| key(part).cmp(&(y, x))) {
        Ok(index) => list[index].area += area,
        Err(index) => list.insert(
            index,
            MappingPart {
                ilon: x,
                ilat: y,
                area,
            },
        ),
    }
}

#[cfg(test)]
#[path = "spatial_mapping_tests.rs"]
mod spatial_mapping_tests;

/// `arclen`（`MOD_Utils.F90:1077-1092`，km）：GIMPLE 是
/// `tmp = .FMA (sin lat1, sin lat2, (cos lat1 * cos lat2) * cos (lon1 - lon2))`，再
/// `MIN_EXPR (MAX_EXPR (tmp, -1), 1)`（不换成 `clamp`：两者对 NaN 的处理不同）。
///
/// 同参的 `sin`/`cos` 在 gfortran 里是 `__builtin_cexpi`，macOS 上落到 `cexp(i·x)`，与独立的
/// `sin()`/`cos()` 逐位相同；Rust 的 `sin_cos()`（或相邻的 `sin`/`cos`）在 release 下会被 LLVM
/// 并成 `__sincos_stret`，其 sin 对约 1% 的输入差 1 ULP。两点同纬（东西向距离）时 `tmp` 贴近 1，
/// `acos` 把这 1 ULP 放大到距离的 ~1e-12 相对量级，双线性权重随之错位（g1fbil 边缘几格的末位差）。
/// 所以走下面两个不内联的 `sin`/`cos`。
#[allow(clippy::manual_clamp)]
pub(crate) fn arclen(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (s1, c1) = (libm_sin(lat1), libm_cos(lat1));
    let (s2, c2) = (libm_sin(lat2), libm_cos(lat2));
    let tmp = s1.contract(s2, c1 * c2 * (lon1 - lon2).cos());
    let tmp = tmp.max(-1.0).min(1.0);
    6.37122e3 * tmp.acos()
}

/// 不内联的 libm `sin`：阻止 LLVM 与同参 `cos` 合并成 `__sincos_stret`（见 [`arclen`]）。
#[inline(never)]
fn libm_sin(value: f64) -> f64 {
    value.sin()
}

/// 不内联的 libm `cos`（见 [`libm_sin`]）。
#[inline(never)]
fn libm_cos(value: f64) -> f64 {
    value.cos()
}
