//! netlib 参考 LAPACK 3.x 的 `DGETRF` + `DGETRI`，按参考 BLAS 的循环逐句展开。
//!
//! 对上游的调用（`n <= 5`、`lwork = n`）走的都是非分块路径：`DGETRF` 的 `NB = 64 >= min(m,n)`
//! → `DGETRF2`（递归）；`DGETRI` 的 `NB >= N` → 非分块；`DTRTRI` 的 `NB >= N` → `DTRTI2`。
//! 各 BLAS 操作的更新都是 `c = c + t*a` 形式。参考库在 x86-64 上由 gfortran `-O2` 编译，
//! 不带 `-march` 就没有 FMA 指令，这些更新是分开舍入的；aarch64 上 gfortran 默认
//! `-ffp-contract=fast`，会收成 FMA。[`FUSED`] 按目标架构选。
//!
//! 在 macOS 上它不参与计算（那里用 Accelerate），只由测试覆盖。

// 循环照参考 LAPACK/BLAS 的下标逐句展开，改成迭代器会让与 netlib 源码的对照失去意义。
#![allow(clippy::needless_range_loop)]
use anyhow::{ensure, Result};

/// 参考 BLAS 的 `c + t*a` 是否被编译器收成 FMA（见模块说明）。
const FUSED: bool = cfg!(target_arch = "aarch64");

/// `DLAMCH('S')`：`1/huge < tiny`，所以就是最小正规数。
const SAFE_MINIMUM: f64 = f64::MIN_POSITIVE;

fn update(accumulator: f64, factor: f64, value: f64, fused: bool) -> f64 {
    if fused {
        factor.mul_add(value, accumulator)
    } else {
        accumulator + factor * value
    }
}

/// 就地求逆列主序的 `n×n` 矩阵。
pub(crate) fn invert(a: &mut [f64], n: usize) -> Result<()> {
    invert_with(a, n, FUSED)
}

pub(crate) fn invert_with(a: &mut [f64], n: usize, fused: bool) -> Result<()> {
    ensure!(a.len() == n * n, "matrix storage does not match its order");
    let mut pivots = vec![0; n];
    let singular = getrf2(a, 0, n, n, n, &mut pivots, fused);
    ensure!(
        !singular,
        "the urban radiation matrix is numerically singular (DGETRF)"
    );
    getri(a, n, &pivots, fused);
    Ok(())
}

/// `IDAMAX`：第一个绝对值最大的下标（严格大于才替换）。
fn idamax(a: &[f64], offset: usize, len: usize) -> usize {
    let mut index = 0;
    let mut largest = a[offset].abs();
    for i in 1..len {
        if a[offset + i].abs() > largest {
            largest = a[offset + i].abs();
            index = i;
        }
    }
    index
}

/// `DGETRF2(M, N, A(off), LDA, IPIV, INFO)`，返回是否遇到零主元（`INFO > 0`）。
///
/// `pivots` 是 0 起的**局部**行号（相对子块首行）。
fn getrf2(
    a: &mut [f64],
    offset: usize,
    lda: usize,
    m: usize,
    n: usize,
    pivots: &mut [usize],
    fused: bool,
) -> bool {
    let at = |i: usize, j: usize| offset + i + j * lda;
    if m == 1 {
        pivots[0] = 0;
        return a[at(0, 0)] == 0.0;
    }
    if n == 1 {
        let p = idamax(a, at(0, 0), m);
        pivots[0] = p;
        if a[at(p, 0)] == 0.0 {
            return true;
        }
        if p != 0 {
            a.swap(at(0, 0), at(p, 0));
        }
        let pivot = a[at(0, 0)];
        if pivot.abs() >= SAFE_MINIMUM {
            // `DSCAL(M-1, ONE/A(1,1), A(2,1), 1)`：乘倒数，不是逐个除
            let reciprocal = 1.0 / pivot;
            for i in 1..m {
                a[at(i, 0)] *= reciprocal;
            }
        } else {
            for i in 1..m {
                a[at(i, 0)] /= pivot;
            }
        }
        return false;
    }
    let n1 = m.min(n) / 2;
    let n2 = n - n1;
    let mut singular = getrf2(a, offset, lda, m, n1, &mut pivots[..n1], fused);
    // `DLASWP(N2, A(1,N1+1), LDA, 1, N1, IPIV, 1)`
    for k in 0..n1 {
        let p = pivots[k];
        if p != k {
            for j in n1..n {
                a.swap(at(k, j), at(p, j));
            }
        }
    }
    // `DTRSM('L','L','N','U', N1, N2, ONE, A, LDA, A(1,N1+1), LDA)`
    for j in n1..n {
        for k in 0..n1 {
            let b = a[at(k, j)];
            if b != 0.0 {
                for i in (k + 1)..n1 {
                    a[at(i, j)] = update(a[at(i, j)], -b, a[at(i, k)], fused);
                }
            }
        }
    }
    // `DGEMM('N','N', M-N1, N2, N1, -ONE, A(N1+1,1), LDA, A(1,N1+1), LDA, ONE, A(N1+1,N1+1), LDA)`
    for j in n1..n {
        for l in 0..n1 {
            let t = -a[at(l, j)];
            for i in n1..m {
                a[at(i, j)] = update(a[at(i, j)], t, a[at(i, l)], fused);
            }
        }
    }
    let tail = m.min(n) - n1;
    let mut inner = vec![0; tail];
    singular |= getrf2(a, at(n1, n1), lda, m - n1, n2, &mut inner, fused);
    for (k, p) in inner.into_iter().enumerate() {
        pivots[n1 + k] = p + n1;
    }
    // `DLASWP(N1, A(1,1), LDA, N1+1, MIN(M,N), IPIV, 1)`
    for k in n1..m.min(n) {
        let p = pivots[k];
        if p != k {
            for j in 0..n1 {
                a.swap(at(k, j), at(p, j));
            }
        }
    }
    singular
}

/// `DGETRI` 非分块路径（含 `DTRTI2`）。
fn getri(a: &mut [f64], n: usize, pivots: &[usize], fused: bool) {
    let at = |i: usize, j: usize| i + j * n;
    // `DTRTI2('Upper', 'Non-unit')`
    for j in 0..n {
        a[at(j, j)] = 1.0 / a[at(j, j)];
        let ajj = -a[at(j, j)];
        // `DTRMV('Upper','No transpose','Non-unit', J-1, A, LDA, A(1,J), 1)`
        for k in 0..j {
            let t = a[at(k, j)];
            if t != 0.0 {
                for i in 0..k {
                    a[at(i, j)] = update(a[at(i, j)], t, a[at(i, k)], fused);
                }
                a[at(k, j)] *= a[at(k, k)];
            }
        }
        // `DSCAL(J-1, AJJ, A(1,J), 1)`
        for i in 0..j {
            a[at(i, j)] *= ajj;
        }
    }
    // 解 `inv(A)*L = inv(U)`
    let mut work = vec![0.0; n];
    for j in (0..n).rev() {
        for i in (j + 1)..n {
            work[i] = a[at(i, j)];
            a[at(i, j)] = 0.0;
        }
        // `DGEMV('No transpose', N, N-J, -ONE, A(1,J+1), LDA, WORK(J+1), 1, ONE, A(1,J), 1)`
        for k in (j + 1)..n {
            let t = -work[k];
            for i in 0..n {
                a[at(i, j)] = update(a[at(i, j)], t, a[at(i, k)], fused);
            }
        }
    }
    // 列交换还原主元
    for j in (0..n.saturating_sub(1)).rev() {
        let p = pivots[j];
        if p != j {
            for i in 0..n {
                a.swap(at(i, j), at(i, p));
            }
        }
    }
}
