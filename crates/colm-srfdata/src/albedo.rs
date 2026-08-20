//! 土壤颜色档到四个反照率的查表。
//!
//! 表来自 `mkinidata/MOD_SoilColorRefl.F90`（Lawrence & Chase 2007）。
//! 档位 `isc` 不是猜的，也不是固定的：CoLM 从 `rawdata/soil_brightness.nc`
//! 取站点像元（`MOD_SingleSrfdata.F90:727-731`）。实测 90 个 PLUMBER2 站点
//! 的 isc 落在 7–20，集中于 14–16，其中只有 1 个是 10 —— 而先前的 Python
//! 脚本把 10 写死了，于是另外 89 个站点的土壤反照率都是错的。
//!
//! 水体与冰盖不查表，四个值保持 CoLM 的缺省标记，见 `albedo` 的返回值。

/// 饱和土壤的可见光反照率，按颜色档 1..=20。
pub const SOIL_S_V_REFL: [f64; 20] = [
    0.26, 0.24, 0.22, 0.20, 0.19, 0.18, 0.17, 0.16, 0.15, 0.14, 0.13, 0.12, 0.11, 0.10, 0.09, 0.08,
    0.07, 0.06, 0.05, 0.04,
];

/// 干土壤的可见光反照率。
pub const SOIL_D_V_REFL: [f64; 20] = [
    0.37, 0.35, 0.33, 0.31, 0.30, 0.29, 0.28, 0.27, 0.26, 0.25, 0.24, 0.23, 0.22, 0.21, 0.20, 0.19,
    0.18, 0.17, 0.16, 0.15,
];

/// 饱和土壤的近红外反照率。
pub const SOIL_S_N_REFL: [f64; 20] = [
    0.52, 0.48, 0.44, 0.40, 0.38, 0.36, 0.34, 0.32, 0.30, 0.28, 0.26, 0.24, 0.22, 0.20, 0.18, 0.16,
    0.14, 0.12, 0.10, 0.08,
];

/// 干土壤的近红外反照率。
pub const SOIL_D_N_REFL: [f64; 20] = [
    0.63, 0.59, 0.55, 0.51, 0.49, 0.47, 0.45, 0.43, 0.41, 0.39, 0.37, 0.35, 0.33, 0.31, 0.29, 0.27,
    0.25, 0.23, 0.21, 0.19,
];

/// IGBP 的水体类别，`MOD_SingleSrfdata.F90:735`。
pub const IGBP_WATER: i32 = 17;
/// IGBP 的冰盖类别，同上。
pub const IGBP_ICE: i32 = 15;
/// IGBP 的「城市与建成区」。URBAN 路径把地类强制成它
/// （`MOD_SingleSrfdata.F90:1548`），而它既不是水体也不是冰盖 ——
/// 所以城市站点只要有颜色档就一定查得到四个反照率。
pub const IGBP_URBAN: i32 = 13;

/// 一个站点的四个土壤反照率。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilAlbedo {
    pub s_v: f64,
    pub d_v: f64,
    pub s_n: f64,
    pub d_n: f64,
}

/// 按颜色档与地表类型查四个反照率。
///
/// 水体、冰盖，或颜色档越出 1..=20 时返回 `None` —— CoLM 在这三种情况下
/// 让四个值停在 `spval`，所以这里也不能凑一个出来。
pub fn albedo(isc: i32, igbp_landtype: i32) -> Option<SoilAlbedo> {
    if igbp_landtype == IGBP_WATER || igbp_landtype == IGBP_ICE {
        return None;
    }
    if !(1..=20).contains(&isc) {
        return None;
    }
    let i = (isc - 1) as usize;
    Some(SoilAlbedo {
        s_v: SOIL_S_V_REFL[i],
        d_v: SOIL_D_V_REFL[i],
        s_n: SOIL_S_N_REFL[i],
        d_n: SOIL_D_N_REFL[i],
    })
}

#[cfg(test)]
#[path = "albedo_tests.rs"]
mod albedo_tests;
