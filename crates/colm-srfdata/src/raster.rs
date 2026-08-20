//! 从全球栅格里取单个像元。
//!
//! CoLM 的对应物是 `share/MOD_NetCDFPoint.F90` 的 `read_point_var_2d_*`：
//! 算出 (ilon, ilat) 之后 `nf90_get_var(..., start=(/ilon,ilat/), count=(/1,1/))`。
//! 这里做同一件事，索引由 `grid` 模块给出。
//!
//! 这么做的理由是数据量：`topography.nc` 是 38 GB 的 43200×86400 网格，
//! 而单点只要 1 个像元。抽出来的站点参数包每站几 KB。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::grid::{Grid, COLM_500M};

/// 从 `colm_500m` 栅格里取站点像元，按 f64 读出。
///
/// 读到 `_FillValue` 时**报错**，不把它当成数据返回。三个栅格都带这个属性
/// （`lake_depth` 是 -32767，`elevation` 与 `elvstd` 是 -9999），而海上或
/// 无数据的像元就是这个值。90 个 PLUMBER2 站点都没踩到，但靠海的站点会 ——
/// 把 -9999 当成高程写进站点文件，模型会照单全收地算下去。
pub fn point_f64(file: &Path, var: &str, lon: f64, lat: f64) -> Result<f64> {
    point_f64_on(COLM_500M, file, var, lon, lat)
}

/// 同上，但网格由调用方指定。
///
/// **网格名跟着文件走。** `urban/LUCY_regionid.nc` 是 `colm_5km`
/// （`MOD_SingleSrfdata.F90:1861`），其余几个 rawdata 栅格是 `colm_500m`。
/// 用错网格既不会报错也不会越界 —— 只会安静地取到另一个像元。
pub fn point_f64_on(grid: Grid, file: &Path, var: &str, lon: f64, lat: f64) -> Result<f64> {
    let (ilon, ilat) = grid.index_of(lon, lat);
    read_pixel(file, var, ilon, ilat, None)
}

/// 从一个已知下标的像元读一个数，并对 `_FillValue` 报错。
///
/// `itime` 是第三维（1-based）；`None` 表示这是个二维变量。
/// CoLM 的对应物分别是 `read_point_var_2d_real8` 与
/// `read_point_5x5_var_2d_time_real8`，两者只差这一维。
fn read_pixel(
    file: &Path,
    var: &str,
    ilon: usize,
    ilat: usize,
    itime: Option<usize>,
) -> Result<f64> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let v = f
        .variable(var)
        .with_context(|| format!("{var} not in {}", file.display()))?;
    // netcdf crate 的下标是 0-based，而 grid 给的是 1-based（与 Fortran 一致）。
    // 维度次序是 C 序，与 Fortran 的 `(/ilon,ilat,itime/)` 正好相反。
    let mut ranges = Vec::with_capacity(3);
    if let Some(t) = itime {
        ranges.push((t - 1)..t);
    }
    ranges.push((ilat - 1)..ilat);
    ranges.push((ilon - 1)..ilon);
    let vals: Vec<f64> = v
        .get_values(netcdf::Extents::from(&ranges[..]))
        .with_context(|| format!("cannot read {var} at ({ilon},{ilat}) in {}", file.display()))?;
    let x = vals
        .first()
        .copied()
        .with_context(|| format!("{var} returned no value at ({ilon},{ilat})"))?;
    if let Some(fill) = fill_value(&v) {
        if x == fill {
            bail!(
                "{var} is _FillValue ({fill}) at pixel ({ilon},{ilat}) of {}; \
                 this site has no data here",
                file.display()
            );
        }
    }
    Ok(x)
}

/// 站点落在哪个 5x5 瓦片文件上。
///
/// `sfx` 是 CoLM 那边的 `sfx` 参数：`URBTYP`、`URBLAI_2000`、`URBSRF2020` ……
/// 文件名是 `<dir>/RG_<north>_<west>_<south>_<east>.<sfx>.nc`。
pub fn tile_5x5_path(dir: &Path, sfx: &str, lon: f64, lat: f64) -> (PathBuf, usize, usize) {
    let t = COLM_500M.tile_5x5(lon, lat);
    (dir.join(format!("{}.{sfx}.nc", t.stem)), t.ilon, t.ilat)
}

/// 从 5x5 瓦片里取一个整型像元（`read_point_5x5_var_2d_int32`）。
pub fn point_5x5_i32(dir: &Path, sfx: &str, var: &str, lon: f64, lat: f64) -> Result<i32> {
    let (file, ilon, ilat) = tile_5x5_path(dir, sfx, lon, lat);
    Ok(read_pixel(&file, var, ilon, ilat, None)?.round() as i32)
}

/// 从 5x5 瓦片里取一个带时间维的实型像元（`read_point_5x5_var_2d_time_real8`）。
///
/// `itime` 是 1-based，与 Fortran 一致。
pub fn point_5x5_time_f64(
    dir: &Path,
    sfx: &str,
    var: &str,
    lon: f64,
    lat: f64,
    itime: usize,
) -> Result<f64> {
    let (file, ilon, ilat) = tile_5x5_path(dir, sfx, lon, lat);
    read_pixel(&file, var, ilon, ilat, Some(itime))
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
