//! macOS：直接调 Accelerate 的 `dgetrf_`/`dgetri_`，与 Fortran 内核同一份实现。
//!
//! 显式链 `Accelerate` framework，而不是 `-llapack`：后者会被 `LIBRARY_PATH` 里的
//! Homebrew `lapack`/`openblas` 抢走，而内核链接时 SDK 解析到的是 Accelerate。
#![allow(unsafe_code)]

use anyhow::{ensure, Result};

#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn dgetrf_(
        m: *const i32,
        n: *const i32,
        a: *mut f64,
        lda: *const i32,
        ipiv: *mut i32,
        info: *mut i32,
    );
    fn dgetri_(
        n: *const i32,
        a: *mut f64,
        lda: *const i32,
        ipiv: *const i32,
        work: *mut f64,
        lwork: *const i32,
        info: *mut i32,
    );
}

/// 就地求逆列主序的 `n×n` 矩阵；`lwork = n` 照抄上游。
pub(crate) fn invert(a: &mut [f64], n: usize) -> Result<()> {
    ensure!(a.len() == n * n, "matrix storage does not match its order");
    let order = i32::try_from(n)?;
    let mut ipiv = vec![0_i32; n];
    let mut work = vec![0.0; n];
    let mut info = 0_i32;
    // SAFETY: `a` 是 `n*n` 个元素、`lda = n`；`ipiv` 与 `work` 各 `n` 个、`lwork = n`。
    // LAPACK 只在这些范围内读写，调用返回后不保留任何指针。
    unsafe {
        dgetrf_(
            &order,
            &order,
            a.as_mut_ptr(),
            &order,
            ipiv.as_mut_ptr(),
            &mut info,
        );
    }
    ensure!(
        info == 0,
        "the urban radiation matrix is numerically singular (DGETRF info {info})"
    );
    // SAFETY: 同上；`ipiv` 是上一步 `dgetrf_` 写出的主元。
    unsafe {
        dgetri_(
            &order,
            a.as_mut_ptr(),
            &order,
            ipiv.as_ptr(),
            work.as_mut_ptr(),
            &order,
            &mut info,
        );
    }
    ensure!(
        info == 0,
        "the urban radiation matrix inversion failed (DGETRI info {info})"
    );
    Ok(())
}
