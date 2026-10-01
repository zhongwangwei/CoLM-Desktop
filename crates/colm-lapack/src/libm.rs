//! 系统 libm 里 Rust 标准库没有包装的函数（gfortran 的内建同样落到这里）。
#![allow(unsafe_code)]

extern "C" {
    #[link_name = "erf"]
    fn c_erf(x: f64) -> f64;
}

/// `erf(x)`：系统 libm 的实现，与 Fortran 内核调用的是同一个符号。
pub fn erf(x: f64) -> f64 {
    // SAFETY: `erf` 是纯函数，对任意 f64 都有定义，不读写调用方内存。
    unsafe { c_erf(x) }
}
