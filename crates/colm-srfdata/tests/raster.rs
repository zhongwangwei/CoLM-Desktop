//! 读真实栅格的测试。放在 tests/ 而不是 src/，理由同
//! colm-forcing/tests/met.rs：`--lib --bins` 会把 src/ 里的
//! `#[cfg(test)]` 模块带进每个 PR 的作业，而那里没有 rawdata。

use std::path::PathBuf;

use colm_srfdata::raster::{point_f64, point_i32};

/// rawdata 的位置。缺失时测试**失败**而不是跳过 ——
/// 里程碑 1 的教训是「跳过会被读成通过」。CI 上这些测试只在
/// golden 作业里跑，那里数据是齐的。
fn rawdata() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("COLM_RAWDATA")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/rawdata".to_string()),
    );
    assert!(
        p.join("soil_brightness.nc").exists(),
        "rawdata not found at {}; set COLM_RAWDATA",
        p.display()
    );
    p
}

const CN_CNG_LON: f64 = 123.509_201_049_804_69;
const CN_CNG_LAT: f64 = 44.593_299_865_722_656;

#[test]
fn cn_cng_soil_brightness_is_ten() {
    let v = point_i32(
        &rawdata().join("soil_brightness.nc"),
        "soil_brightness",
        CN_CNG_LON,
        CN_CNG_LAT,
    )
    .unwrap();
    assert_eq!(v, 10);
}

#[test]
fn cn_cng_topography_matches_the_measured_pixel() {
    let f = rawdata().join("topography.nc");
    let e = point_f64(&f, "elevation", CN_CNG_LON, CN_CNG_LAT).unwrap();
    let s = point_f64(&f, "elvstd", CN_CNG_LON, CN_CNG_LAT).unwrap();
    let g = point_f64(&f, "slope", CN_CNG_LON, CN_CNG_LAT).unwrap();
    assert!((e - 144.144_454_956_054_7).abs() < 1e-6, "elevation {e}");
    assert!((s - 0.496_343_106_031_417_85).abs() < 1e-9, "elvstd {s}");
    assert!((g - 0.003_575_807_437_300_682).abs() < 1e-12, "slope {g}");
}

#[test]
fn cn_cng_is_not_a_lake() {
    let v = point_f64(
        &rawdata().join("lake_depth.nc"),
        "lake_depth",
        CN_CNG_LON,
        CN_CNG_LAT,
    )
    .unwrap();
    assert_eq!(v, 0.0);
}

#[test]
fn a_fill_value_pixel_is_an_error_not_an_elevation() {
    // 南太平洋中部，远离任何陆地：topography 在那里是 _FillValue。
    // 若这里返回 -9999 而不是报错，它就会被当成高程写进站点文件。
    let e = point_f64(&rawdata().join("topography.nc"), "elevation", -140.0, -30.0);
    match e {
        Err(err) => assert!(format!("{err:#}").contains("_FillValue"), "{err:#}"),
        Ok(v) => panic!("expected an error, got elevation {v}"),
    }
}

#[test]
fn a_missing_variable_is_an_error_not_a_zero() {
    let e = point_f64(
        &rawdata().join("lake_depth.nc"),
        "no_such_variable",
        CN_CNG_LON,
        CN_CNG_LAT,
    );
    assert!(e.is_err());
}
