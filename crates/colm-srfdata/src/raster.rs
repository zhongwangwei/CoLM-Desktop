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

/// Reads one one-based time slice at a site from CoLM's regular 500 m grid.
///
/// This is the `read_point_var_2d_time_real8` path used by the native
/// eight-day LAI input.  Its NetCDF order is `(time, lat, lon)`.
pub fn point_time_f64(file: &Path, var: &str, lon: f64, lat: f64, itime: usize) -> Result<f64> {
    validate_lon_lat(lon, lat)?;
    let (ilon, ilat) = COLM_500M.index_of(lon, lat);
    read_pixel(file, var, ilon, ilat, Some(itime))
}

/// 同上，但网格由调用方指定。
///
/// **网格名跟着文件走。** `urban/LUCY_regionid.nc` 是 `colm_5km`
/// （`MOD_SingleSrfdata.F90:1861`），其余几个 rawdata 栅格是 `colm_500m`。
/// 用错网格既不会报错也不会越界 —— 只会安静地取到另一个像元。
pub fn point_f64_on(grid: Grid, file: &Path, var: &str, lon: f64, lat: f64) -> Result<f64> {
    validate_lon_lat(lon, lat)?;
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
    if ilon == 0 || ilat == 0 || itime == Some(0) {
        bail!("NetCDF point indices are 1-based; got ({ilon},{ilat},{itime:?})");
    }
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
pub fn tile_5x5_path(dir: &Path, sfx: &str, lon: f64, lat: f64) -> Result<(PathBuf, usize, usize)> {
    validate_lon_lat(lon, lat)?;
    let t = COLM_500M.tile_5x5(lon, lat);
    Ok((dir.join(format!("{}.{sfx}.nc", t.stem)), t.ilon, t.ilat))
}

/// 从 5x5 瓦片里取一个整型像元（`read_point_5x5_var_2d_int32`）。
pub fn point_5x5_i32(dir: &Path, sfx: &str, var: &str, lon: f64, lat: f64) -> Result<i32> {
    let (file, ilon, ilat) = tile_5x5_path(dir, sfx, lon, lat)?;
    pixel_to_i32(read_pixel(&file, var, ilon, ilat, None)?, var)
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
    let (file, ilon, ilat) = tile_5x5_path(dir, sfx, lon, lat)?;
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
    pixel_to_i32(point_f64(file, var, lon, lat)?, var)
}

fn validate_lon_lat(lon: f64, lat: f64) -> Result<()> {
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        bail!("raster longitude must be finite and within -180..=180, got {lon}");
    }
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        bail!("raster latitude must be finite and within -90..=90, got {lat}");
    }
    Ok(())
}

fn pixel_to_i32(value: f64, var: &str) -> Result<i32> {
    let rounded = value.round();
    if !rounded.is_finite() || rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        bail!("{var} value {value} cannot be represented as i32");
    }
    Ok(rounded as i32)
}

#[cfg(test)]
mod raster_tests {
    use super::*;

    fn tmp_nc(name: &str, value: f64) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "colm-srfdata-raster-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.nc"));
        let _ = std::fs::remove_file(&path);
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        let mut var = file.add_variable::<f64>("pixel", &["lat", "lon"]).unwrap();
        var.put_values(&[value], netcdf::Extents::All).unwrap();
        path
    }

    #[test]
    fn raster_points_reject_invalid_coordinates_before_indexing() {
        let err = point_f64(
            Path::new("does-not-need-to-exist.nc"),
            "pixel",
            f64::NAN,
            0.0,
        )
        .unwrap_err();
        assert!(err.to_string().contains("longitude"), "{err:#}");
        let err = tile_5x5_path(Path::new("."), "URBTYP", 0.0, f64::INFINITY).unwrap_err();
        assert!(err.to_string().contains("latitude"), "{err:#}");
    }

    #[test]
    fn raster_time_indices_are_one_based() {
        let err = read_pixel(
            Path::new("does-not-need-to-exist.nc"),
            "pixel",
            1,
            1,
            Some(0),
        )
        .unwrap_err();
        assert!(err.to_string().contains("1-based"), "{err:#}");
    }

    #[test]
    fn timed_raster_reads_one_based_time_slice() {
        let path = std::env::temp_dir().join(format!(
            "colm-srfdata-raster-time-{}-{:?}.nc",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 2).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        file.add_variable::<f64>("pixel", &["time", "lat", "lon"])
            .unwrap()
            .put_values(&[1.0, 2.0], ..)
            .unwrap();
        file.close().unwrap();

        assert_eq!(
            point_time_f64(&path, "pixel", -180.0, 90.0, 2).unwrap(),
            2.0
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn integer_pixels_reject_nonfinite_and_overflow_values() {
        let nan = tmp_nc("nan", f64::NAN);
        let err = point_i32(&nan, "pixel", -180.0, 90.0).unwrap_err();
        assert!(err.to_string().contains("i32"), "{err:#}");

        let huge = tmp_nc("huge", i32::MAX as f64 + 1024.0);
        let err = point_i32(&huge, "pixel", -180.0, 90.0).unwrap_err();
        assert!(err.to_string().contains("i32"), "{err:#}");
    }
}
