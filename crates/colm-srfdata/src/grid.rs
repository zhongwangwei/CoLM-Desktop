//! CoLM 全球规则网格上的单点索引。
//!
//! CoLM 取点走 `find_nearest_west` / `find_nearest_south` 两个二分查找
//! （`share/MOD_Utils.F90`），网格由 `grid_define_by_ndims` 生成
//! （`share/MOD_Grid.F90`）：经度西边界升序、纬度南边界降序，都是等距。
//!
//! 等距网格上二分查找有闭式解，但**朴素的那个是错的**：
//! `floor((90-y)/dlat)+1` 在纬度恰好落在格边界时比 CoLM 多一格，赤道就是
//! 这种情形（CoLM 给 21600，朴素式给 21601）。改成 `ceil` 之后仍不够 ——
//! 极点附近 `90.0 - lat` 会发生灾难性抵消，`ceil` 又会跳掉一格。
//! 所以这里用解析式起步、再拿**真实的边界值**校正一两步。
//! `grid_tests.rs` 里有一份二分查找的移植逐点比对着这件事。
//!
//! 索引是 **1-based**，与 Fortran 一致 —— 抽取时要直接喂给
//! `nf90_get_var` 的 start 向量，换成 0-based 只会在交界处埋一个 off-by-one。

/// 一个等距的全球经纬网格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub nlon: usize,
    pub nlat: usize,
}

/// `colm_500m`：`grid_define_by_ndims(86400, 43200)`。
/// 三个 rawdata 栅格（lake_depth / soil_brightness / topography）都用它，
/// `urban_type/` 与 `urban_lai_500m/` 的 5x5 瓦片也是。
pub const COLM_500M: Grid = Grid {
    nlon: 86400,
    nlat: 43200,
};

/// `colm_5km`：`grid_define_by_ndims(8640, 4320)`。
///
/// 只有 `urban/LUCY_regionid.nc` 用它（`MOD_SingleSrfdata.F90:1861`）。
/// **网格名跟着文件走，不跟着模块走** —— 同一个 `read_point_var_2d_real8`
/// 在别处配的是 `colm_500m`，用错网格不会报错，只会取到另一个像元。
pub const COLM_5KM: Grid = Grid {
    nlon: 8640,
    nlat: 4320,
};

/// 一个 5°x5° 瓦片里的落点：文件名词干与瓦片内的 1-based 下标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile5x5 {
    /// `RG_<north>_<west>_<south>_<east>`，不含 `.<sfx>.nc`。
    pub stem: String,
    /// 瓦片内的经度下标，1-based。
    pub ilon: usize,
    /// 瓦片内的纬度下标，1-based。
    pub ilat: usize,
}

impl Grid {
    pub fn dlon(&self) -> f64 {
        360.0 / self.nlon as f64
    }

    pub fn dlat(&self) -> f64 {
        180.0 / self.nlat as f64
    }

    /// 返回 (ilon, ilat)，1-based。
    pub fn index_of(&self, lon: f64, lat: f64) -> (usize, usize) {
        (self.ilon(lon), self.ilat(lat))
    }

    /// 第 i 格的西边界（1-based），与 `grid_define_by_ndims` 算法一致。
    pub fn lon_w(&self, i: usize) -> f64 {
        -180.0 + self.dlon() * ((i - 1) as f64)
    }

    /// 第 i 格的东边界；全球最后一格按 CoLM 规范化回 -180°。
    pub fn lon_e(&self, i: usize) -> f64 {
        if i == self.nlon {
            -180.0
        } else {
            -180.0 + self.dlon() * (i as f64)
        }
    }

    /// 第 j 格的南边界（1-based），同上。纬度是降序的。
    pub fn lat_s(&self, j: usize) -> f64 {
        90.0 - self.dlat() * (j as f64)
    }

    /// 第 j 格的北边界（1-based）。
    pub fn lat_n(&self, j: usize) -> f64 {
        90.0 - self.dlat() * ((j - 1) as f64)
    }

    fn ilon(&self, lon: f64) -> usize {
        let n = self.nlon;
        let mut i =
            ((((lon + 180.0) / self.dlon()).floor() as i64) + 1).clamp(1, n as i64) as usize;
        // 解析式只是起点，判据是真实的边界值：见 ilat 的说明。
        while i > 1 && self.lon_w(i) > lon {
            i -= 1;
        }
        while i < n && self.lon_w(i + 1) <= lon {
            i += 1;
        }
        i
    }

    /// 站点落在哪个 5°x5° 瓦片的哪一格。
    ///
    /// 逐行照抄 `share/MOD_NetCDFPoint.F90` 的 `get_5x5_filename`：
    ///
    /// ```text
    /// nxbox = nlon/360*5           ibox = (ilon-1)/nxbox + 1
    /// nybox = nlat/180*5           jbox = (ilat-1)/nybox + 1
    /// RG_<(19-jbox)*5>_<(ibox-37)*5>_<(18-jbox)*5>_<(ibox-36)*5>
    /// ```
    ///
    /// 四个数字的次序是**北、西、南、东**，而 `(19-jbox)*5` 是北、
    /// `(18-jbox)*5` 是南 —— 纬度下标是降序的，所以 jbox 越大越靠南。
    /// 照几何直觉写成「南在前」会得到一个存在的、但错了 5 度的文件名。
    pub fn tile_5x5(&self, lon: f64, lat: f64) -> Tile5x5 {
        let (ilon, ilat) = self.index_of(lon, lat);
        let nxbox = self.nlon / 360 * 5;
        let nybox = self.nlat / 180 * 5;
        let ibox = (ilon - 1) / nxbox + 1;
        let jbox = (ilat - 1) / nybox + 1;
        let (north, west) = ((19 - jbox as i64) * 5, (ibox as i64 - 37) * 5);
        let (south, east) = ((18 - jbox as i64) * 5, (ibox as i64 - 36) * 5);
        Tile5x5 {
            stem: format!("RG_{north}_{west}_{south}_{east}"),
            ilon: ilon - (ibox - 1) * nxbox,
            ilat: ilat - (jbox - 1) * nybox,
        }
    }

    fn ilat(&self, lat: f64) -> usize {
        let n = self.nlat;
        if lat >= self.lat_s(1) {
            return 1;
        }
        if lat <= self.lat_s(n) {
            return n;
        }
        // ceil 而不是 floor+1 —— 见模块文档。但解析式只能当起点：
        // 90.0 - dlat*j 在极点附近做减法会发生灾难性抵消，(90-lat)/dlat
        // 算出 2.000000000001 而不是 2，ceil 就跳掉一格。实测 j=2
        // （纬度 89.99166666666666）就是这种情形。
        //
        // 所以起点之后用**真实的边界值**校正：CoLM 的二分查找比较的是
        // 算出来的 90 - dlat*j，不是数学上的那个数，照它比才对得上。
        let mut j = (((90.0 - lat) / self.dlat()).ceil() as i64).clamp(1, n as i64) as usize;
        while j > 1 && self.lat_s(j - 1) <= lat {
            j -= 1;
        }
        while j < n && self.lat_s(j) > lat {
            j += 1;
        }
        j
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod grid_tests;
