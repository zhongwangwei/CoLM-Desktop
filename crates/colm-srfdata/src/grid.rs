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
/// 三个 rawdata 栅格（lake_depth / soil_brightness / topography）都用它。
pub const COLM_500M: Grid = Grid {
    nlon: 86400,
    nlat: 43200,
};

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
    fn lon_w(&self, i: usize) -> f64 {
        -180.0 + self.dlon() * ((i - 1) as f64)
    }

    /// 第 j 格的南边界（1-based），同上。纬度是降序的。
    fn lat_s(&self, j: usize) -> f64 {
        90.0 - self.dlat() * (j as f64)
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
