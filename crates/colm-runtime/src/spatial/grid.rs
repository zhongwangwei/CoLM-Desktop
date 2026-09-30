//! `MOD_Grid` 里空间主循环要用的那部分：由格心定义强迫网格（`grid_define_by_center`）、
//! 规整化（`grid_normalize`）与格心弧度（`grid_set_rlon`/`grid_set_rlat`）。
//!
//! 边界的每一步都照上游的运算顺序写：格边是相邻格心的 `(a + b) * 0.5`，跨日界线时
//! `(a + b + 360) * 0.5`；这些边直接进 `areaquad`，差 1 ULP 就会让映射权重不同。

use anyhow::{ensure, Result};

/// 经纬网格（1 起的上游下标在这里是 0 起）。
#[derive(Debug, Clone, PartialEq)]
pub struct LatLonGrid {
    pub lat_s: Vec<f64>,
    pub lat_n: Vec<f64>,
    pub lon_w: Vec<f64>,
    pub lon_e: Vec<f64>,
    /// 纬度方向：`1` 自南向北，`-1` 自北向南。
    pub yinc: i32,
    /// 格心经度（弧度）。
    pub rlon: Vec<f64>,
    /// 格心纬度（弧度）。
    pub rlat: Vec<f64>,
}

/// 区域网格的外边界（`DEF_forcing%regbnd`）；全球网格为 `None`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridBounds {
    pub south: f64,
    pub north: f64,
    pub west: f64,
    pub east: f64,
}

impl LatLonGrid {
    pub fn nlat(&self) -> usize {
        self.lat_s.len()
    }

    pub fn nlon(&self) -> usize {
        self.lon_w.len()
    }

    /// `grid_define_by_center` + `normalize` + `set_rlon`/`set_rlat`。
    pub fn define_by_center(
        lat_in: &[f64],
        lon_in: &[f64],
        bounds: Option<GridBounds>,
    ) -> Result<Self> {
        let nlat = lat_in.len();
        let nlon = lon_in.len();
        ensure!(
            nlat > 0 && nlon > 0,
            "a forcing grid needs latitudes and longitudes"
        );
        ensure!(
            lat_in.iter().chain(lon_in).all(|value| value.is_finite()),
            "forcing grid coordinates must be finite"
        );
        let yinc = if lat_in[0] > lat_in[nlat - 1] { -1 } else { 1 };
        let mut lat_s = vec![0.0; nlat];
        let mut lat_n = vec![0.0; nlat];
        let north = bounds.map_or(90.0, |b| b.north);
        let south = bounds.map_or(-90.0, |b| b.south);
        for ilat in 0..nlat {
            if yinc == 1 {
                lat_n[ilat] = if ilat + 1 < nlat {
                    (lat_in[ilat] + lat_in[ilat + 1]) * 0.5
                } else {
                    north
                };
                lat_s[ilat] = if ilat > 0 {
                    (lat_in[ilat - 1] + lat_in[ilat]) * 0.5
                } else {
                    south
                };
            } else {
                lat_n[ilat] = if ilat > 0 {
                    (lat_in[ilat - 1] + lat_in[ilat]) * 0.5
                } else {
                    north
                };
                lat_s[ilat] = if ilat + 1 < nlat {
                    (lat_in[ilat] + lat_in[ilat + 1]) * 0.5
                } else {
                    south
                };
            }
        }
        let lon_n = lon_in
            .iter()
            .map(|&lon| normalize_longitude(lon))
            .collect::<Result<Vec<_>>>()?;
        let mut lon_w = vec![0.0; nlon];
        let mut lon_e = vec![0.0; nlon];
        for ilon in 0..nlon {
            let ilone = (ilon + 1) % nlon;
            lon_e[ilon] = if lon_n[ilon] > lon_n[ilone] {
                (lon_n[ilon] + lon_n[ilone] + 360.0) * 0.5
            } else {
                (lon_n[ilon] + lon_n[ilone]) * 0.5
            };
            if ilon == nlon - 1 {
                if let Some(bounds) = bounds {
                    lon_e[ilon] = bounds.east;
                }
            }
            let ilonw = if ilon == 0 { nlon - 1 } else { ilon - 1 };
            lon_w[ilon] = if lon_n[ilonw] > lon_n[ilon] {
                (lon_n[ilonw] + lon_n[ilon] + 360.0) * 0.5
            } else {
                (lon_n[ilonw] + lon_n[ilon]) * 0.5
            };
            if ilon == 0 {
                if let Some(bounds) = bounds {
                    lon_w[0] = bounds.west;
                }
            }
        }
        let mut grid = Self {
            lat_s,
            lat_n,
            lon_w,
            lon_e,
            yinc,
            rlon: Vec::new(),
            rlat: Vec::new(),
        };
        grid.normalize()?;
        grid.set_centers()?;
        Ok(grid)
    }

    /// `grid_define_by_res`：`nint(360/lon_res)` × `nint(180/lat_res)` 个格子。
    pub fn define_by_res(lon_res: f64, lat_res: f64) -> Result<Self> {
        ensure!(
            lon_res > 0.0 && lat_res > 0.0,
            "grid resolutions must be positive"
        );
        Self::define_by_ndims(
            (360.0 / lon_res).round() as usize,
            (180.0 / lat_res).round() as usize,
        )
    }

    /// `grid_define_by_ndims`：自北向南、自 -180° 向东。内核以 `-fdefault-real-8` 编译，
    /// `180.0 / n` 是双精度。
    pub fn define_by_ndims(nlon: usize, nlat: usize) -> Result<Self> {
        ensure!(nlon > 0 && nlat > 0, "a grid needs cells");
        let del_lat = 180.0 / nlat as f64;
        let del_lon = 360.0 / nlon as f64;
        let lat_s = (1..=nlat).map(|i| 90.0 - del_lat * i as f64).collect();
        let lat_n = (1..=nlat)
            .map(|i| 90.0 - del_lat * (i - 1) as f64)
            .collect();
        let lon_w = (1..=nlon)
            .map(|i| -180.0 + del_lon * (i - 1) as f64)
            .collect();
        let mut lon_e: Vec<f64> = (1..=nlon).map(|i| -180.0 + del_lon * i as f64).collect();
        lon_e[nlon - 1] = -180.0;
        let mut grid = Self {
            lat_s,
            lat_n,
            lon_w,
            lon_e,
            yinc: 1,
            rlon: Vec::new(),
            rlat: Vec::new(),
        };
        grid.normalize()?;
        grid.set_centers()?;
        Ok(grid)
    }

    /// `grid_set_blocks` 选出的、落在 `DEF_domain` 里的行与列（0 起，按文件顺序）；
    /// history 文件只写这一窗（`set_grid_concat`）。
    pub fn domain_window(&self, bounds: GridBounds) -> Result<(Vec<usize>, Vec<usize>)> {
        let west = normalize_longitude(bounds.west)?;
        let east = normalize_longitude(bounds.east)?;
        let nlat = self.nlat();
        let nlon = self.nlon();
        let mut rows = Vec::new();
        if self.yinc == 1 {
            let mut ilat = if bounds.south < self.lat_s[0] {
                0
            } else {
                find_nearest_south(bounds.south, &self.lat_s)
            };
            while ilat < nlat && self.lat_s[ilat] < bounds.north {
                rows.push(ilat);
                ilat += 1;
            }
        } else {
            let mut ilat = if bounds.north > self.lat_n[0] {
                0
            } else {
                find_nearest_north(bounds.north, &self.lat_n)
            };
            while ilat < nlat && self.lat_n[ilat] > bounds.south {
                rows.push(ilat);
                ilat += 1;
            }
        }
        let first = if self.lon_w[0] != self.lon_e[nlon - 1]
            && lon_between_floor(west, self.lon_e[nlon - 1], self.lon_w[0])
        {
            0
        } else {
            find_nearest_west(west, &self.lon_w)
        };
        let mut columns = vec![first];
        let mut ilon = (first + 1) % nlon;
        while ilon != first && lon_between_floor(self.lon_w[ilon], west, east) {
            columns.push(ilon);
            ilon = (ilon + 1) % nlon;
        }
        ensure!(
            !rows.is_empty(),
            "the history grid has no row inside the domain"
        );
        Ok((rows, columns))
    }

    /// `grid_normalize`。
    fn normalize(&mut self) -> Result<()> {
        for lon in self.lon_w.iter_mut().chain(self.lon_e.iter_mut()) {
            *lon = normalize_longitude(*lon)?;
        }
        for lat in self.lat_s.iter_mut().chain(self.lat_n.iter_mut()) {
            *lat = 90.0_f64.min(*lat).max(-90.0);
        }
        let nlat = self.nlat();
        let nlon = self.nlon();
        self.yinc = if self.lat_s[0] <= self.lat_s[nlat - 1] {
            1
        } else {
            -1
        };
        for ilon in 0..nlon.saturating_sub(1) {
            if lon_between_ceil(self.lon_e[ilon], self.lon_w[ilon + 1], self.lon_e[ilon + 1]) {
                self.lon_e[ilon] = self.lon_w[ilon + 1];
            } else {
                self.lon_w[ilon + 1] = self.lon_e[ilon];
            }
        }
        if nlon > 1 {
            let ilon = nlon - 1;
            if lon_between_ceil(self.lon_e[ilon], self.lon_w[0], self.lon_e[0]) {
                self.lon_e[ilon] = self.lon_w[0];
            }
        }
        for ilat in 0..nlat.saturating_sub(1) {
            if self.yinc == 1 {
                self.lat_n[ilat] = self.lat_n[ilat].max(self.lat_s[ilat + 1]);
                self.lat_s[ilat + 1] = self.lat_n[ilat];
            } else {
                self.lat_s[ilat] = self.lat_s[ilat].min(self.lat_n[ilat + 1]);
                self.lat_n[ilat + 1] = self.lat_s[ilat];
            }
        }
        Ok(())
    }

    /// `grid_set_rlon` + `grid_set_rlat`：`lon / 180 * pi`，`(s + n) * 0.5 / 180 * pi`。
    fn set_centers(&mut self) -> Result<()> {
        let pi = std::f64::consts::PI;
        self.rlon = self
            .lon_w
            .iter()
            .zip(&self.lon_e)
            .map(|(&west, &east)| {
                let lon = if west <= east {
                    (west + east) * 0.5
                } else {
                    (west + east) * 0.5 + 180.0
                };
                Ok(normalize_longitude(lon)? / 180.0 * pi)
            })
            .collect::<Result<Vec<_>>>()?;
        self.rlat = self
            .lat_s
            .iter()
            .zip(&self.lat_n)
            .map(|(&south, &north)| (south + north) * 0.5 / 180.0 * pi)
            .collect();
        Ok(())
    }
}

/// `normalize_longitude`：落到 `[-180, 180)`；`modulo(lon, 360)` 取与除数同号的余数。
pub fn normalize_longitude(lon: f64) -> Result<f64> {
    ensure!(lon.is_finite(), "longitude must be finite");
    ensure!(
        lon.abs() - 360.0 != lon.abs(),
        "longitude magnitude cannot resolve a full revolution"
    );
    if !(-180.0..180.0).contains(&lon) {
        let mut value = lon.rem_euclid(360.0);
        if value >= 180.0 {
            value -= 360.0;
        }
        return Ok(value);
    }
    Ok(lon)
}

/// `lon_between_floor`：`[west, east)`，`west >= east` 时跨日界线。
pub fn lon_between_floor(lon: f64, west: f64, east: f64) -> bool {
    if west >= east {
        lon >= west || lon < east
    } else {
        lon >= west && lon < east
    }
}

/// `lon_between_ceil`：`(west, east]`。
pub fn lon_between_ceil(lon: f64, west: f64, east: f64) -> bool {
    if west >= east {
        lon > west || lon <= east
    } else {
        lon > west && lon <= east
    }
}

/// `find_nearest_south`（0 起）。
pub fn find_nearest_south(y: f64, lat: &[f64]) -> usize {
    let n = lat.len();
    if lat[0] < lat[n - 1] {
        if y <= lat[0] {
            0
        } else if y >= lat[n - 1] {
            n - 1
        } else {
            let (mut left, mut right) = (0, n - 1);
            while right - left > 1 {
                let i = (right + left) / 2;
                if y >= lat[i] {
                    left = i;
                } else {
                    right = i;
                }
            }
            left
        }
    } else if y >= lat[0] {
        0
    } else if y <= lat[n - 1] {
        n - 1
    } else {
        let (mut left, mut right) = (0, n - 1);
        while right - left > 1 {
            let i = (right + left) / 2;
            if y >= lat[i] {
                right = i;
            } else {
                left = i;
            }
        }
        right
    }
}

/// `find_nearest_north`（0 起）。
pub fn find_nearest_north(y: f64, lat: &[f64]) -> usize {
    let n = lat.len();
    if lat[0] < lat[n - 1] {
        if y <= lat[0] {
            0
        } else if y >= lat[n - 1] {
            n - 1
        } else {
            let (mut left, mut right) = (0, n - 1);
            while right - left > 1 {
                let i = (right + left) / 2;
                if y > lat[i] {
                    left = i;
                } else {
                    right = i;
                }
            }
            right
        }
    } else if y >= lat[0] {
        0
    } else if y <= lat[n - 1] {
        n - 1
    } else {
        let (mut left, mut right) = (0, n - 1);
        while right - left > 1 {
            let i = (right + left) / 2;
            if y > lat[i] {
                right = i;
            } else {
                left = i;
            }
        }
        left
    }
}

/// `find_nearest_west`（0 起）。
pub fn find_nearest_west(x: f64, lon: &[f64]) -> usize {
    let n = lon.len();
    if n == 1 {
        return 0;
    }
    if lon_between_floor(x, lon[n - 1], lon[0]) {
        return n - 1;
    }
    let (mut left, mut right) = (0, n - 1);
    while right - left > 1 {
        let i = (right + left) / 2;
        if lon_between_floor(x, lon[i], lon[right]) {
            left = i;
        } else {
            right = i;
        }
    }
    left
}

/// `find_nearest_east`（0 起）。
pub fn find_nearest_east(x: f64, lon: &[f64]) -> usize {
    let n = lon.len();
    if n == 1 {
        return 0;
    }
    if lon_between_ceil(x, lon[n - 1], lon[0]) {
        return 0;
    }
    let (mut left, mut right) = (0, n - 1);
    while right - left > 1 {
        let i = (right + left) / 2;
        if lon_between_ceil(x, lon[i], lon[right]) {
            left = i;
        } else {
            right = i;
        }
    }
    right
}

/// `areaquad`（km²）：`dx * dy * re * re`，左结合。
pub fn areaquad(lat_s: f64, lat_n: f64, lon_w: f64, lon_e: f64) -> f64 {
    const RE: f64 = 6.37122e3;
    const DEG2RAD: f64 = 1.745_329_251_994_33e-2;
    let dx = if lon_e < lon_w {
        (lon_e + 360.0 - lon_w) * DEG2RAD
    } else {
        (lon_e - lon_w) * DEG2RAD
    };
    let dy = (lat_n * DEG2RAD).sin() - (lat_s * DEG2RAD).sin();
    dx * dy * RE * RE
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod grid_tests;
