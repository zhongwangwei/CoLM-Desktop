//! 城市辐射矩阵求逆：上游 `MOD_Urban_Shortwave:MatrixInverse`（`DGETRF` + `DGETRI`，`lwork = n`）。
//!
//! 短波、长波与 BEM 共用这一个函数，之后都接 `X = matmul(Ainv, B)`。要与 Fortran 内核逐位一致，
//! 求逆就必须与内核**链的那一份 LAPACK** 算出同样的位 —— 而那一份随平台不同：
//!
//! - **macOS**：`Makeoptions` 的 `-llapack -lblas` 解析到 Accelerate（`otool -L colm.x` →
//!   `vecLib.framework/.../libLAPACK.dylib`）。它是闭源的：实测 LU 分解能按参考算法 + FMA 逐位复刻
//!   （4000/4000），`DGETRI` 的三角求逆却没有一种参考变体超过 2857/4000
//!   （`docs/implementation-verification.md` 第 410 轮），所以这里 FFI 直接调它（[`accelerate`]）。
//! - **其它平台**：Windows 内核由 MSYS2 构建，链的是 netlib 参考 LAPACK/BLAS；那是开源的，
//!   逐句移植（[`reference`]）即可，不必让 MSVC 工具链与 GUI 打包依赖一个系统库。
//!
//! 工作区对 `unsafe_code` 是 `forbid`；本 crate 单独降为 `deny`，只有 `accelerate` 与 `libm`（`erf` 等标准库没包装的 libm 函数）两个模块放行。

use anyhow::{ensure, Result};
use colm_numeric::Contract;

#[cfg(target_os = "macos")]
mod accelerate;
pub mod libm;
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod reference;

/// `MatrixInverse(A)`：`matrix[i][j]` 是 `A(i+1, j+1)`，只取左上 `n×n`（`n` 之外按 0 返回）。
///
/// 奇异或失败时报错，对应上游的 `CoLM_stop('Matrix is numerically singular!')` /
/// `CoLM_stop('Matrix inversion failed!')`。
pub fn matrix_inverse<const N: usize>(matrix: &[[f64; N]; N], n: usize) -> Result<[[f64; N]; N]> {
    ensure!((1..=N).contains(&n), "matrix order {n} is outside 1..={N}");
    ensure!(
        matrix[..n]
            .iter()
            .all(|row| row[..n].iter().all(|value| value.is_finite())),
        "the urban radiation matrix is not finite"
    );
    // Fortran 列主序
    let mut a = vec![0.0; n * n];
    for column in 0..n {
        for row in 0..n {
            a[column * n + row] = matrix[row][column];
        }
    }
    invert_column_major(&mut a, n)?;
    let mut inverse = [[0.0; N]; N];
    for column in 0..n {
        for row in 0..n {
            inverse[row][column] = a[column * n + row];
        }
    }
    Ok(inverse)
}

/// `X = matmul(Ainv, B)`：gfortran `-O2` 把它内联成按列的 FMA 累加
/// （`MOD_Urban_Shortwave.F90:189`、`MOD_Urban_BEM.F90:184` 的 GIMPLE：`x = {}` 之后
/// 对 `j = 1..n` 做 `x(i) = .FMA (Ainv(i,j), b(j), x(i))`）。第一项 `FMA(a, b, 0)` 就是正确舍入的乘积。
pub fn matmul<const N: usize>(inverse: &[[f64; N]; N], vector: &[f64; N], n: usize) -> [f64; N] {
    let mut out = [0.0; N];
    for column in 0..n {
        for (row, value) in out.iter_mut().enumerate().take(n) {
            *value = inverse[row][column].contract(vector[column], *value);
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn invert_column_major(a: &mut [f64], n: usize) -> Result<()> {
    accelerate::invert(a, n)
}

#[cfg(not(target_os = "macos"))]
fn invert_column_major(a: &mut [f64], n: usize) -> Result<()> {
    reference::invert(a, n)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
