//! `MOD_BGC_Daylength.F90`：日长。物候本体（含作物）由 `oracle/scripts/bgc_port/regen.py` 生成，
//! 见 [`crate::bgc_cn_phenology`]；这里只保留转写器处理不了的 `daylength`（`sin`/`cos` 的取法见下）。

use crate::atmosphere::{fortran_cos, fortran_sin};

/// `daylength(dlat, idate2)`（秒）。
///
/// gfortran 把 `my_lat` 的 `sin`/`cos` 合成 `cexpi`，macOS 上落到 `cexp(i·x)`；实测它与独立的
/// `sin`/`cos` 逐位相同（2000 万个样本），而 LLVM 在 release 下会把 Rust 的相邻 `sin`/`cos`
/// 并成 `__sincos_stret`（sin 约 0.9% 差 1 ULP），所以一律走不内联的 `fortran_sin/cos`。
/// `cos(decl)` 被改写成 `cos(|decl|)`；`2·secs_per_radian`、`(23.44/180)·π`、`2π/365` 都已折叠。
pub fn daylength(dlat: f64, idate2: i32) -> f64 {
    const PI: f64 = std::f64::consts::PI;
    const POLE_PLUS_EPS: f64 =
        1.570_796_326_794_898_778_445_030_984_585_173_428_058_624_267_578_125;
    const OFFSET_POLE: f64 = 1.570_796_326_794_894_3;
    const POLE: f64 = std::f64::consts::FRAC_PI_2;
    let lat = dlat / 180.0 * PI;
    if lat.abs() >= POLE_PLUS_EPS {
        return -9999.0;
    }
    let decl_abs =
        fortran_cos(f64::from(idate2 + 10) * 0.017_214_206_321_039_96) * 0.409_105_176_667_470_87;
    if decl_abs.abs() >= POLE {
        return -9999.0;
    }
    let decl = -decl_abs;
    let my_lat = lat.clamp(-OFFSET_POLE, OFFSET_POLE);
    let (sin_lat, cos_lat) = (fortran_sin(my_lat), fortran_cos(my_lat));
    let temp = -((sin_lat * fortran_sin(decl)) / (cos_lat * fortran_cos(decl_abs)));
    temp.clamp(-1.0, 1.0).acos() * 2.750_197_42e4
}
