//! 从全球栅格里取单个像元。
//!
//! CoLM 的对应物是 `share/MOD_NetCDFPoint.F90` 的 `read_point_var_2d_*`：
//! 算出 (ilon, ilat) 之后 `nf90_get_var(..., start=(/ilon,ilat/), count=(/1,1/))`。
//! 这里做同一件事，索引由 `grid` 模块给出。
//!
//! 这么做的理由是数据量：`topography.nc` 是 38 GB 的 43200×86400 网格，
//! 而单点只要 1 个像元。抽出来的站点参数包每站几 KB。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::grid::COLM_500M;

/// 从 `colm_500m` 栅格里取站点像元，按 f64 读出。
///
/// 读到 `_FillValue` 时**报错**，不把它当成数据返回。三个栅格都带这个属性
/// （`lake_depth` 是 -32767，`elevation` 与 `elvstd` 是 -9999），而海上或
/// 无数据的像元就是这个值。90 个 PLUMBER2 站点都没踩到，但靠海的站点会 ——
/// 把 -9999 当成高程写进站点文件，模型会照单全收地算下去。
pub fn point_f64(file: &Path, var: &str, lon: f64, lat: f64) -> Result<f64> {
    let (ilon, ilat) = COLM_500M.index_of(lon, lat);
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let v = f
        .variable(var)
        .with_context(|| format!("{var} not in {}", file.display()))?;
    // netcdf crate 的下标是 0-based，而 grid 给的是 1-based（与 Fortran 一致）
    let vals: Vec<f64> = v
        .get_values(netcdf::Extents::from(
            &[(ilat - 1)..(ilat), (ilon - 1)..(ilon)][..],
        ))
        .with_context(|| format!("cannot read {var} at ({ilon},{ilat})"))?;
    let x = vals
        .first()
        .copied()
        .with_context(|| format!("{var} returned no value at ({ilon},{ilat})"))?;
    if let Some(fill) = fill_value(&v) {
        if x == fill {
            bail!(
                "{var} is _FillValue ({fill}) at pixel ({ilon},{ilat}); this site has no data here"
            );
        }
    }
    Ok(x)
}

/// 变量的 `_FillValue`，按 f64 读出；没有该属性或它不是数值时返回 `None`。
fn fill_value(v: &netcdf::Variable) -> Option<f64> {
    use netcdf::AttributeValue as A;
    match v.attribute("_FillValue")?.value().ok()? {
        A::Uchar(x) => Some(x as f64),
        A::Schar(x) => Some(x as f64),
        A::Ushort(x) => Some(x as f64),
        A::Short(x) => Some(x as f64),
        A::Uint(x) => Some(x as f64),
        A::Int(x) => Some(x as f64),
        A::Ulonglong(x) => Some(x as f64),
        A::Longlong(x) => Some(x as f64),
        A::Float(x) => Some(x as f64),
        A::Double(x) => Some(x),
        _ => None,
    }
}

/// 同上，按 i32 读出（`soil_brightness` 与 `soiltexture` 是整型）。
pub fn point_i32(file: &Path, var: &str, lon: f64, lat: f64) -> Result<i32> {
    Ok(point_f64(file, var, lon, lat)?.round() as i32)
}

#[cfg(test)]
#[path = "raster_tests.rs"]
mod raster_tests;
