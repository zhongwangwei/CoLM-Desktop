//! `MOD_SpatialMapping:spatial_mapping_build_arealweighted` 与 `grid2pset`：强迫网格到 pixelset（patch）
//! 的面积加权映射。
//!
//! 每个 set 逐像元、逐与之相交的网格格子累加重叠面积 `areaquad`；格子在 set 内按 `(ilat, ilon)`
//! 有序（`insert_into_sorted_list2`），`grid2pset` 也按这个顺序累加 —— 求和顺序就是逐位的关键。

use anyhow::{ensure, Result};

use crate::spatial_grid::{
    areaquad, find_nearest_east, find_nearest_north, find_nearest_south, find_nearest_west,
    lon_between_ceil, lon_between_floor, LatLonGrid,
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
                    sum = part.area.mul_add(value(part.ilon, part.ilat), sum);
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
