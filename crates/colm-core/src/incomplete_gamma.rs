//! 不完全伽马函数比 P(a,x)、Q(a,x)，移植自 `share/MOD_IncompleteGamma.F90`
//! （ACM Algorithm 654，A. H. Morris Jr.，TOMS 13(3):318）。
//!
//! TOPMODEL 饱和面积（`MOD_Runoff.F90`、`MOD_BGC_Veg_CNFireLi2016.F90`）调用
//! `GRATIO(alp, x, P, Q, 0)`。本文件逐条照搬 `gfortran -fdefault-real-8 -fdefault-double-8 -O2`
//! （macOS arm64）对该模块的 GIMPLE 优化转储（`*.273t.optimized`），目标是与
//! Fortran 内核按位一致：
//!
//! - GIMPLE 中的 `.FMA/.FMS/.FNMA` 一律写成 `mul_add`，其余乘加保持分开；
//!   运算次序、括号和分支按 GIMPLE 而非源码文本。
//! - `-fdefault-real-8 -fdefault-double-8`（upstream-bugs 第 17 条）下默认 `REAL`、
//!   `DOUBLE PRECISION` 和带 `D` 指数的字面量都是 real(8)，所以 `5.E-15`、
//!   `.398942280401433`、`0.7D0` 等都是最接近的 f64。`GLOG`、`GAMMA` 的大参数
//!   分支以及 `RLOG` 的 `DBLE(X) - 0.7D0`、`0.75D0*DBLE(X) - 1.D0` 是普通 f64
//!   运算（只有 `-fdefault-real-8` 时它们曾被提升成 real(16) 软浮点）。
//! - `EXPARG(0)` 在 GIMPLE 中折成常数 `709.775615066255…`，`0.99999*EXPARG(0)`
//!   折成 `709.7685173101044…`；`E = epsilon(1.)` 即 `f64::EPSILON`。
//! - `GAMMA` 负参数分支的 `SIN` 走 [`crate::atmosphere::fortran_sin`]，避免与
//!   同参数余弦合并。
//!
//! 出错返回（`ANS = 2`）时 Fortran 不写 `QANS`，调用方变量保留旧值；需要
//! 这一语义的调用方用 [`gratio_fortran`]。

// `!(a >= b)` 形式的比较照搬 GIMPLE 的分支方向（NaN 时与 Fortran 走同一支）；
// `t = (x / apn) * t`、`sum = wk[k] + sum` 保留 GIMPLE 的操作数次序。
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::assign_op_pattern,
    clippy::manual_range_contains
)]

use crate::atmosphere::fortran_sin;

/// `E = epsilon(1.)`（real(8)）。
const EPS: f64 = f64::EPSILON;
/// `2.0*E`。
const TWO_EPS: f64 = 4.440892098500626e-16;
/// `5.0*E`（GIMPLE 中折为 `1.1102230246251565e-15`）。
const FIVE_EPS: f64 = 1.1102230246251565e-15;

const ACC0: [f64; 3] = [5.0e-15, 5.0e-7, 5.0e-4];
const BIG: [f64; 3] = [20.0, 14.0, 10.0];
const E00: [f64; 3] = [0.25e-3, 0.25e-1, 0.14];
const X00: [f64; 3] = [31.0, 17.0, 9.7];

// 上游 DATA 的截断字面量（LN(10)），故意不用 std 常数。
#[allow(clippy::approx_constant)]
const ALOG10: f64 = 2.30258509299405;
const RT2PIN: f64 = 0.398942280401433;
const RTPI: f64 = 1.77245385090552;
const THIRD: f64 = 0.333333333333333;

const D0: [f64; 13] = [
    0.833333333333333e-01,
    -0.148148148148148e-01,
    0.115740740740741e-02,
    0.352733686067019e-03,
    -0.178755144032922e-03,
    0.391926317852244e-04,
    -0.218544851067999e-05,
    -0.185406221071516e-05,
    0.829671134095309e-06,
    -0.176659527368261e-06,
    0.670785354340150e-08,
    0.102618097842403e-07,
    -0.438203601845335e-08,
];
const D10: f64 = -0.185185185185185e-02;
const D1: [f64; 12] = [
    -0.347222222222222e-02,
    0.264550264550265e-02,
    -0.990226337448560e-03,
    0.205761316872428e-03,
    -0.401877572016461e-06,
    -0.180985503344900e-04,
    0.764916091608111e-05,
    -0.161209008945634e-05,
    0.464712780280743e-08,
    0.137863344691572e-06,
    -0.575254560351770e-07,
    0.119516285997781e-07,
];
const D20: f64 = 0.413359788359788e-02;
const D2: [f64; 10] = [
    -0.268132716049383e-02,
    0.771604938271605e-03,
    0.200938786008230e-05,
    -0.107366532263652e-03,
    0.529234488291201e-04,
    -0.127606351886187e-04,
    0.342357873409614e-07,
    0.137219573090629e-05,
    -0.629899213838006e-06,
    0.142806142060642e-06,
];
const D30: f64 = 0.649434156378601e-03;
const D3: [f64; 8] = [
    0.229472093621399e-03,
    -0.469189494395256e-03,
    0.267720632062839e-03,
    -0.756180167188398e-04,
    -0.239650511386730e-06,
    0.110826541153473e-04,
    -0.567495282699160e-05,
    0.142309007324359e-05,
];
const D40: f64 = -0.861888290916712e-03;
const D4: [f64; 6] = [
    0.784039221720067e-03,
    -0.299072480303190e-03,
    -0.146384525788434e-05,
    0.664149821546512e-04,
    -0.396836504717943e-04,
    0.113757269706784e-04,
];
const D50: f64 = -0.336798553366358e-03;
const D5: [f64; 4] = [
    -0.697281375836586e-04,
    0.277275324495939e-03,
    -0.199325705161888e-03,
    0.679778047793721e-04,
];
const D60: f64 = 0.531307936463992e-03;
const D6: [f64; 2] = [-0.592166437353694e-03, 0.270878209671804e-03];
const D70: f64 = 0.344367606892378e-03;

/// 按 GIMPLE 的 `.FMA` 链求 Horner 多项式：`coeffs` 从最高次到常数项，
/// 每步 `acc = fma(acc, z, c)`。首项为 1 时（`ERFC1` 的 `Q(1)`）GCC 把
/// 第一步折成加法，`fma(1, z, c)` 与 `z + c` 结果相同。
#[inline]
fn horner(z: f64, coeffs: &[f64]) -> f64 {
    let mut acc = coeffs[0];
    for &c in &coeffs[1..] {
        acc = acc.mul_add(z, c);
    }
    acc
}

/// P(a,x) 与 Q(a,x)，即 `CALL GRATIO(A, X, ANS, QANS, 0)` 的 `(ANS, QANS)`。
///
/// 出错（`a`/`x` 为负、`a = x = 0`、或二者不可定）时返回 `(2.0, NaN)`：
/// Fortran 此时只写 `ANS = 2`、不动 `QANS`，要保留调用方旧 `QANS` 的语义请用
/// [`gratio_fortran`]。
pub fn gratio(a: f64, x: f64) -> (f64, f64) {
    let mut ans = 0.0;
    let mut qans = f64::NAN;
    gratio_fortran(a, x, &mut ans, &mut qans, 0);
    (ans, qans)
}

/// 与 `SUBROUTINE GRATIO(A, X, ANS, QANS, IND)` 完全同语义：出错时只写
/// `ans = 2`，`qans` 保持调用前的值。
pub fn gratio_fortran(a: f64, x: f64, ans: &mut f64, qans: &mut f64, ind: i32) {
    match gratio_core(a, x, ind) {
        Some((p, q)) => {
            *ans = p;
            *qans = q;
        }
        // 标号 400
        None => *ans = 2.0,
    }
}

/// 标号 300 / 310 / 331。
const P0_Q1: (f64, f64) = (0.0, 1.0);
const P1_Q0: (f64, f64) = (1.0, 0.0);

#[inline]
fn label_331(a: f64, x: f64) -> (f64, f64) {
    // GIMPLE：if (a >= x) 300 else 310
    if a >= x {
        P0_Q1
    } else {
        P1_Q0
    }
}

#[inline]
fn p_from_q(q: f64) -> (f64, f64) {
    ((0.5 - q) + 0.5, q)
}

#[inline]
fn q_from_p(p: f64) -> (f64, f64) {
    (p, (0.5 - p) + 0.5)
}

/// `None` 表示标号 400（`ANS = 2`，`QANS` 不写）。
fn gratio_core(a: f64, x: f64, ind: i32) -> Option<(f64, f64)> {
    if a < 0.0 || x < 0.0 {
        return None;
    }
    if a == 0.0 && x == 0.0 {
        return None;
    }
    let ax = a * x;
    if ax == 0.0 {
        return Some(label_331(a, x));
    }

    // IOP = IND + 1；IND 不是 0/1 时 IOP = 3（GIMPLE：(unsigned) ind > 1）
    let (iop, acc, e0, x0) = if (ind as u32) > 1 {
        (3_usize, ACC0[2], E00[2], X00[2])
    } else {
        let k = ind as usize;
        // MAX_EXPR <acc0, E>
        let acc = if ACC0[k] > EPS { ACC0[k] } else { EPS };
        (k + 1, acc, E00[k], X00[k])
    };

    if a >= 1.0 {
        // 标号 10
        if a >= BIG[iop - 1] {
            return label_20(a, x, iop, acc, e0, x0);
        }
        if !(a > x) && !(x >= x0) {
            let twoa = a * 2.0;
            let m = twoa as i32;
            if m as f64 == twoa {
                let i = m >> 1;
                // GIMPLE 把 EXP(-X) 提到 140/150 分叉之前
                let emx = (-x).exp();
                return Some(if a == i as f64 {
                    finite_sum(x, i, emx, emx, 1, 0.0)
                } else {
                    // 标号 150
                    let rtx = x.sqrt();
                    let sum = erfc1(0, rtx);
                    let t = emx / (rtx * RTPI);
                    finite_sum(x, i, sum, t, 0, -0.5)
                });
            }
        }
        // 标号 11
        let t1 = a.mul_add(x.ln(), -x);
        let r = t1.exp() / gamma(a);
        return Some(label_30(a, x, r, acc, x0));
    }

    if a == 0.5 {
        // 标号 320
        let rtx = x.sqrt();
        return Some(if x >= 0.25 {
            p_from_q(erfc1(0, rtx))
        } else {
            q_from_p(erf(rtx))
        });
    }
    if x < 1.1 {
        return Some(taylor_110(a, x, ax, acc));
    }
    let t1 = a.mul_add(x.ln(), -x);
    let u = a * t1.exp();
    if u == 0.0 {
        return Some(P1_Q0);
    }
    let r = (gam1(a) + 1.0) * u;
    Some(cont_frac_170(a, x, r, acc))
}

/// 标号 20：A >= BIG(IOP)。
fn label_20(a: f64, x: f64, iop: usize, acc: f64, e0: f64, x0: f64) -> Option<(f64, f64)> {
    let l = x / a;
    if l == 0.0 {
        return Some(P0_Q1);
    }
    let s = (0.5 - l) + 0.5;
    let z = rlog(l);
    let abs_s = s.abs();
    if z >= 700.0 / a {
        // 标号 330
        if abs_s <= TWO_EPS {
            return None;
        }
        return Some(label_331(a, x));
    }
    let y = a * z;
    let rta = a.sqrt();
    if e0 / rta >= abs_s {
        return temme_250(a, l, y, z, rta, iop);
    }
    if abs_s <= 0.4 {
        return temme_200(a, l, y, z, rta, abs_s, iop);
    }
    let inv = 1.0 / a;
    let t = inv * inv;
    let t1 = horner(t, &[0.75, -1.0, 3.5, -105.0]) / (a * 1260.0);
    let t1 = t1 - y;
    let r = (rta * RT2PIN) * t1.exp();
    Some(label_30(a, x, r, acc, x0))
}

/// 标号 30：按 X 选 Taylor（50）、连分式（170）或渐近展开（80）。
fn label_30(a: f64, x: f64, r: f64, acc: f64, x0: f64) -> (f64, f64) {
    if r == 0.0 {
        return label_331(a, x);
    }
    // MAX_EXPR <a, ALOG10>
    let bound = if a > ALOG10 { a } else { ALOG10 };
    if x <= bound {
        taylor_50(a, x, r, acc)
    } else if x < x0 {
        cont_frac_170(a, x, r, acc)
    } else {
        asymptotic_80(a, x, r, acc)
    }
}

/// 标号 50：P/R 的 Taylor 级数。
fn taylor_50(a: f64, x: f64, r: f64, acc: f64) -> (f64, f64) {
    let mut wk = [0.0_f64; 20];
    let mut apn = a + 1.0;
    let mut t = x / apn;
    wk[0] = t;
    let mut n = 20_usize;
    for nn in 2..=20 {
        apn += 1.0;
        t = (x / apn) * t;
        if t <= 1.0e-3 {
            n = nn;
            break;
        }
        wk[nn - 1] = t;
    }
    // 标号 60
    let mut sum = t;
    let tol = acc * 0.5;
    loop {
        apn += 1.0;
        t = (x / apn) * t;
        sum += t;
        if !(tol < t) {
            break;
        }
    }
    // SUM = SUM + WK(N-1), ..., WK(1)
    for k in (0..n - 1).rev() {
        sum = wk[k] + sum;
    }
    q_from_p((r / a) * (sum + 1.0))
}

/// 标号 80：渐近展开。
fn asymptotic_80(a: f64, x: f64, r: f64, acc: f64) -> (f64, f64) {
    let mut wk = [0.0_f64; 20];
    let mut amn = a - 1.0;
    let mut t = amn / x;
    wk[0] = t;
    let mut n = 20_usize;
    for nn in 2..=20 {
        amn -= 1.0;
        t = (amn / x) * t;
        if t.abs() <= 1.0e-3 {
            n = nn;
            break;
        }
        wk[nn - 1] = t;
    }
    // 标号 90 / 91
    let mut sum = t;
    while !(t.abs() <= acc) {
        amn -= 1.0;
        t = (amn / x) * t;
        sum += t;
    }
    // 标号 100
    for k in (0..n - 1).rev() {
        sum = wk[k] + sum;
    }
    p_from_q((r / x) * (sum + 1.0))
}

/// 标号 110：A < 1、X < 1.1 时 P(A,X)/X**A 的 Taylor 级数（含 130 / 135）。
fn taylor_110(a: f64, x: f64, ax: f64, acc: f64) -> (f64, f64) {
    let mut an = 3.0;
    let mut c = x;
    let mut sum = x / (a + 3.0);
    let tol = (acc * 3.0) / (a + 1.0);
    loop {
        an += 1.0;
        c = -((x / an) * c);
        let t = c / (a + an);
        sum += t;
        if !(t.abs() > tol) {
            break;
        }
    }
    let j = ax * x.mul_add(sum / 6.0 - 0.5 / (a + 2.0), 1.0 / (a + 1.0));

    let z = a * x.ln();
    let h = gam1(a);
    let g = h + 1.0;
    let to_135 = if x < 0.25 { z > -0.13394 } else { a < x / 2.59 };
    if !to_135 {
        // 标号 130
        let w = z.exp();
        return q_from_p((g * w) * ((0.5 - j) + 0.5));
    }
    // 标号 135
    let l = rexp(z);
    let w = (l + 0.5) + 0.5;
    let q = j.mul_add(w, -l).mul_add(g, -h);
    if q < 0.0 {
        return P1_Q0;
    }
    p_from_q(q)
}

/// 标号 140 / 150 之后的有限和（2A 为整数）。
fn finite_sum(x: f64, i: i32, mut sum: f64, mut t: f64, mut n: i32, mut c: f64) -> (f64, f64) {
    // 标号 160
    while n != i {
        n += 1;
        c += 1.0;
        t = (x * t) / c;
        sum += t;
    }
    p_from_q(sum)
}

/// 标号 170：连分式展开。
fn cont_frac_170(a: f64, x: f64, r: f64, acc: f64) -> (f64, f64) {
    // MAX_EXPR <acc, 5E>
    let tol = if acc > FIVE_EPS { acc } else { FIVE_EPS };
    let mut a2nm1 = 1.0_f64;
    let mut a2n = 1.0_f64;
    let mut b2nm1 = x;
    let mut b2n = x + (1.0 - a);
    let mut c = 1.0_f64;
    let an0 = loop {
        a2nm1 = x.mul_add(a2n, a2nm1 * c);
        b2nm1 = x.mul_add(b2n, b2nm1 * c);
        let am0 = a2nm1 / b2nm1;
        c += 1.0;
        let cma = c - a;
        a2n = a2n.mul_add(cma, a2nm1);
        b2n = b2n.mul_add(cma, b2nm1);
        let an0 = a2n / b2n;
        if !((an0 - am0).abs() >= tol * an0) {
            break an0;
        }
    };
    p_from_q(r * an0)
}

/// 标号 240：Temme 展开的收尾。
#[inline]
fn temme_240(l: f64, c: f64, w: f64, t: f64, rta: f64) -> (f64, f64) {
    if l < 1.0 {
        q_from_p((w - (t * RT2PIN) / rta) * c)
    } else {
        p_from_q(((t * RT2PIN) / rta + w) * c)
    }
}

/// 标号 200：一般 Temme 展开（|S| <= 0.4）。
fn temme_200(
    a: f64,
    l: f64,
    y: f64,
    z: f64,
    rta: f64,
    abs_s: f64,
    iop: usize,
) -> Option<(f64, f64)> {
    if abs_s <= TWO_EPS && (a * EPS) * EPS > 3.28e-3 {
        return None;
    }
    let c = (-y).exp();
    let w = erfc1(1, y.sqrt()) * 0.5;
    let u = 1.0 / a;
    let mut z = (z * 2.0).sqrt();
    if l < 1.0 {
        z = -z;
    }
    let t = match iop {
        // 标号 210
        1 => {
            if abs_s <= 1.0e-3 {
                temme_260(z, u)
            } else {
                let c0 = horner(
                    z,
                    &[
                        D0[12], D0[11], D0[10], D0[9], D0[8], D0[7], D0[6], D0[5], D0[4], D0[3],
                        D0[2], D0[1], D0[0], -THIRD,
                    ],
                );
                let c1 = horner(
                    z,
                    &[
                        D1[11], D1[10], D1[9], D1[8], D1[7], D1[6], D1[5], D1[4], D1[3], D1[2],
                        D1[1], D1[0], D10,
                    ],
                );
                let c2 = horner(
                    z,
                    &[
                        D2[9], D2[8], D2[7], D2[6], D2[5], D2[4], D2[3], D2[2], D2[1], D2[0], D20,
                    ],
                );
                let c3 = horner(
                    z,
                    &[D3[7], D3[6], D3[5], D3[4], D3[3], D3[2], D3[1], D3[0], D30],
                );
                let c4 = horner(z, &[D4[5], D4[4], D4[3], D4[2], D4[1], D4[0], D40]);
                let c5 = horner(z, &[D5[3], D5[2], D5[1], D5[0], D50]);
                let c6 = horner(z, &[D6[1], D6[0], D60]);
                horner(u, &[D70, c6, c5, c4, c3, c2, c1, c0])
            }
        }
        // 标号 220
        2 => {
            let c0 = horner(z, &[D0[5], D0[4], D0[3], D0[2], D0[1], D0[0], -THIRD]);
            let c1 = horner(z, &[D1[3], D1[2], D1[1], D1[0], D10]);
            let c2 = horner(z, &[D2[0], D20]);
            horner(u, &[c2, c1, c0])
        }
        // 标号 230
        _ => horner(z, &[D0[2], D0[1], D0[0], -THIRD]),
    };
    Some(temme_240(l, c, w, t, rta))
}

/// 标号 250：L 接近 1 时的 Temme 展开。
fn temme_250(a: f64, l: f64, y: f64, z: f64, rta: f64, iop: usize) -> Option<(f64, f64)> {
    if (a * EPS) * EPS > 3.28e-3 {
        return None;
    }
    let c = (0.5 - y) + 0.5;
    let w = (0.5 - (y.sqrt() * ((0.5 - y / 3.0) + 0.5)) / RTPI) / c;
    let u = 1.0 / a;
    let mut z = (z * 2.0).sqrt();
    if l < 1.0 {
        z = -z;
    }
    let t = match iop {
        1 => temme_260(z, u),
        // 标号 270
        2 => {
            let c0 = horner(z, &[D0[1], D0[0], -THIRD]);
            let c1 = horner(z, &[D1[0], D10]);
            horner(u, &[D20, c1, c0])
        }
        // 标号 280
        _ => horner(z, &[D0[0], -THIRD]),
    };
    Some(temme_240(l, c, w, t, rta))
}

/// 标号 260。
fn temme_260(z: f64, u: f64) -> f64 {
    let c0 = horner(
        z,
        &[D0[6], D0[5], D0[4], D0[3], D0[2], D0[1], D0[0], -THIRD],
    );
    let c1 = horner(z, &[D1[5], D1[4], D1[3], D1[2], D1[1], D1[0], D10]);
    let c2 = horner(z, &[D2[4], D2[3], D2[2], D2[1], D2[0], D20]);
    let c3 = horner(z, &[D3[3], D3[2], D3[1], D3[0], D30]);
    let c4 = horner(z, &[D4[1], D4[0], D40]);
    let c5 = horner(z, &[D5[1], D5[0], D50]);
    let c6 = horner(z, &[D6[0], D60]);
    horner(u, &[D70, c6, c5, c4, c3, c2, c1, c0])
}

// ---------------------------------------------------------------------------
// ERF / ERFC1
// ---------------------------------------------------------------------------

const ERF_A: [f64; 4] = [
    -1.65581836870402e-4,
    3.25324098357738e-2,
    1.02201136918406e-1,
    1.12837916709552e00,
];
const ERF_B: [f64; 4] = [
    4.64988945913179e-3,
    7.01333417158511e-2,
    4.23906732683201e-1,
    1.00000000000000e00,
];
const ERF_P: [f64; 8] = [
    -1.36864857382717e-7,
    5.64195517478974e-1,
    7.21175825088309e00,
    4.31622272220567e01,
    1.52989285046940e02,
    3.39320816734344e02,
    4.51918953711873e02,
    3.00459261020162e02,
];
const ERF_Q: [f64; 8] = [
    1.00000000000000e00,
    1.27827273196294e01,
    7.70001529352295e01,
    2.77585444743988e02,
    6.38980264465631e02,
    9.31354094850610e02,
    7.90950925327898e02,
    3.00459260956983e02,
];
const ERF_R: [f64; 5] = [
    2.10144126479064e00,
    2.62370141675169e01,
    2.13688200555087e01,
    4.65807828718470e00,
    2.82094791773523e-1,
];
const ERF_S: [f64; 5] = [
    9.41537750555460e01,
    1.87114811799590e02,
    9.90191814623914e01,
    1.80124575948747e01,
    1.00000000000000e00,
];
const ERF_C: f64 = 5.64189583547756e-1;

/// `FUNCTION ERF(X)`（模块内函数，遮蔽同名内在函数）。
fn erf(x: f64) -> f64 {
    let ax = x.abs();
    let x2 = x * x;
    if !(ax >= 0.5) {
        let top = horner(x2, &ERF_A);
        let bot = horner(x2, &ERF_B);
        return (x * top) / bot;
    }
    if !(ax > 4.0) {
        let top = horner(ax, &ERF_P);
        let bot = horner(ax, &ERF_Q);
        let e = 1.0 - ((-x2).exp() * top) / bot;
        return if x < 0.0 { -e } else { e };
    }
    // 标号 20
    let mut e = 1.0;
    if !(ax >= 5.54) {
        let t = 1.0 / x2;
        let top = horner(t, &ERF_R);
        let bot = horner(t, &ERF_S);
        let v = ERF_C - top / (x2 * bot);
        e = 1.0 - (v * (-x2).exp()) / ax;
    }
    // 标号 21
    if x < 0.0 {
        -e
    } else {
        e
    }
}

/// `REAL FUNCTION ERFC1(IND, X)`：IND = 0 时为 erfc(x)，否则 exp(x*x)*erfc(x)。
fn erfc1(ind: i32, x: f64) -> f64 {
    let ax = x.abs();
    let x2 = x * x;
    if !(ax >= 0.47) {
        let top = horner(x2, &ERF_A);
        let bot = horner(x2, &ERF_B);
        let e = 1.0 - (x * top) / bot;
        return if ind != 0 { e * x2.exp() } else { e };
    }
    let e = if !(ax > 4.0) {
        let top = horner(ax, &ERF_P);
        let bot = horner(ax, &ERF_Q);
        top / bot
    } else {
        // 标号 20
        if x <= -5.33 {
            // 标号 30
            return if ind != 0 { x2.exp() * 2.0 } else { 2.0 };
        }
        let t = 1.0 / x2;
        let top = horner(t, &ERF_R);
        let bot = horner(t, &ERF_S);
        (ERF_C - top / (x2 * bot)) / ax
    };
    if ind == 0 {
        // 标号 11
        let e = (-x2).exp() * e;
        return if x < 0.0 { 2.0 - e } else { e };
    }
    if x < 0.0 {
        x2.exp().mul_add(2.0, -e)
    } else {
        e
    }
}

// ---------------------------------------------------------------------------
// REXP / RLOG
// ---------------------------------------------------------------------------

/// `REAL FUNCTION REXP(X)`：exp(x) - 1。
fn rexp(x: f64) -> f64 {
    const P1: f64 = 0.914041914819518e-09;
    const P2: f64 = 0.238082361044469e-01;
    const Q1: f64 = -0.499999999085958e+00;
    const Q2: f64 = 0.107141568980644e+00;
    const Q3: f64 = -0.119041179760821e-01;
    const Q4: f64 = 0.595130811860248e-03;
    if !(x.abs() > 0.15) {
        let num = horner(x, &[P2, P1, 1.0]);
        let den = horner(x, &[Q4, Q3, Q2, Q1, 1.0]);
        return x * (num / den);
    }
    // 标号 10
    let w = x.exp();
    if x > 0.0 {
        // 标号 20
        ((0.5 - 1.0 / w) + 0.5) * w
    } else {
        (w - 0.5) - 0.5
    }
}

/// `REAL FUNCTION RLOG(X)`：x - 1 - ln(x)。
fn rlog(x: f64) -> f64 {
    const A: f64 = 0.566749439387324e-01;
    const B: f64 = 0.456512608815524e-01;
    const P0: f64 = 0.333333333333333e+00;
    const P1: f64 = -0.224696413112536e+00;
    const P2: f64 = 0.620886815375787e-02;
    const Q1: f64 = -0.127408923933623e+01;
    const Q2: f64 = 0.354508718369557e+00;
    if x < 0.61 || x > 1.57 {
        // 标号 100
        let r = (x - 0.5) - 0.5;
        return r - x.ln();
    }
    let (u, w1) = if x < 0.82 {
        // 标号 10：U = DBLE(X) - 0.7D0，再 U = U/0.7（两个 0.7 都是 f64）
        let u = x - 0.7;
        let u = u / 0.7;
        (u, (-u).mul_add(0.3, A))
    } else if x > 1.18 {
        // 标号 20：U = 0.75D0*DBLE(X) - 1.D0，GIMPLE 为 `.FMA (x, 0.75, -1.0)`
        let u = x.mul_add(0.75, -1.0);
        (u, u / 3.0 + B)
    } else {
        ((x - 0.5) - 0.5, 0.0)
    };
    // 标号 30
    let r = u / (u + 2.0);
    let t = r * r;
    let w = horner(t, &[P2, P1, P0]) / horner(t, &[Q2, Q1, 1.0]);
    (t * 2.0).mul_add((-r).mul_add(w, 1.0 / (1.0 - r)), w1)
}

// ---------------------------------------------------------------------------
// GAMMA / GLOG / GAM1
// ---------------------------------------------------------------------------

/// `0.99999*EXPARG(0)`，GIMPLE 折叠后的常数。
const EXPARG_LIMIT: f64 = 709.7685173101044711074791848659515380859375;

/// `REAL FUNCTION GAMMA(A)`（模块内函数，遮蔽同名内在函数）。
fn gamma(a: f64) -> f64 {
    const P: [f64; 7] = [
        0.539637273585445e-03,
        0.261939260042690e-02,
        0.204493667594920e-01,
        0.730981088720487e-01,
        0.279648642639792e+00,
        0.553413866010467e+00,
        1.0,
    ];
    const Q: [f64; 7] = [
        -0.832979206704073e-03,
        0.470059485860584e-02,
        0.225211131035340e-01,
        -0.170458969313360e+00,
        -0.567902761974940e-01,
        0.113062953091122e+01,
        1.0,
    ];
    let mut x = a;
    if a.abs() >= 15.0 {
        return gamma_large(a);
    }
    let mut t;
    let m = (a as i32) - 1;
    if m >= 0 {
        t = 1.0;
        for _ in 0..m {
            x -= 1.0;
            t *= x;
        }
        // 标号 12
        x -= 1.0;
    } else {
        // 标号 20
        t = a;
        if !(a > 0.0) {
            // M = -M - 1 = -INT(A)
            let m2 = -(a as i32);
            for _ in 0..m2 {
                x += 1.0;
                t *= x;
            }
            // 标号 22
            x = (x + 0.5) + 0.5;
            t *= x;
            if t == 0.0 {
                return 0.0;
            }
        }
        // 标号 30
        if !(t.abs() >= 1.0e-30) {
            if t.abs() * f64::MAX <= 1.0001 {
                return 0.0;
            }
            return 1.0 / t;
        }
    }
    // 标号 40：GAMMA(1 + X)，0 <= X < 1
    let top = horner(x, &P);
    let bot = horner(x, &Q);
    let g = top / bot;
    if a < 1.0 {
        g / t
    } else {
        g * t
    }
}

/// `GAMMA` 的 |A| >= 15 分支（标号 60 / 70），`D`、`G`、`Z`、`LNX` 为 `DOUBLE PRECISION`（f64）。
fn gamma_large(a: f64) -> f64 {
    // 上游 DATA PI /3.1415926535898/，故意不用 std::f64::consts::PI。
    #[allow(clippy::approx_constant)]
    const PI: f64 = 3.1415926535898;
    const R1: f64 = 0.820756370353826e-03;
    const R2: f64 = -0.595156336428591e-03;
    const R3: f64 = 0.793650663183693e-03;
    const R4: f64 = -0.277777777770481e-02;
    const R5: f64 = 0.833333333333333e-01;
    /// `D = .41893853320467274178D0`。
    #[allow(clippy::excessive_precision)]
    const D: f64 = 0.41893853320467274178;
    if a.abs() >= 1.0e3 {
        return 0.0;
    }
    let mut x = a;
    let mut s = 0.0;
    if !(a > 0.0) {
        x = -a;
        let n = x as i32;
        let mut t = x - n as f64;
        if t > 0.9 {
            t = 1.0 - t;
        }
        s = fortran_sin(t * PI) / PI;
        if n & 1 == 0 {
            s = -s;
        }
        if s == 0.0 {
            return 0.0;
        }
    }
    // 标号 70
    let t = 1.0 / (x * x);
    let g = horner(t, &[R1, R2, R3, R4, R5]) / x;
    let lnx = glog(x);
    // `G = (D + G) + (Z - 0.5D0)*(LNX - 1.D0)`：GIMPLE 为 `.FMA (z - 0.5, lnx - 1, g + D)`
    let g = (x - 0.5).mul_add(lnx - 1.0, g + D);
    // `W = G`、`T = G - DBLE(W)`：同为 f64，有限时 T 恒为 0，照 GIMPLE 保留
    let w = g;
    if w > EXPARG_LIMIT {
        return 0.0;
    }
    let t = g - w;
    let gam = w.exp() * (t + 1.0);
    if a < 0.0 {
        (1.0 / (gam * s)) / x
    } else {
        gam
    }
}

/// `W(J) = LN(J + 14)` 的 `DOUBLE PRECISION` DATA 字面量（f64），取自 GIMPLE
/// 转储中 `static real(kind=8) w[163]` 的初值。
#[rustfmt::skip]
#[allow(clippy::approx_constant)]
const GLOG_W: [f64; 163] = [
    2.70805020110221, 2.772588722239781, 2.833213344056216,
    2.8903717578961645, 2.9444389791664403, 2.995732273553991,
    3.044522437723423, 3.091042453358316, 3.1354942159291497,
    3.1780538303479458, 3.2188758248682006, 3.258096538021482,
    3.295836866004329, 3.332204510175204, 3.367295829986474,
    3.4011973816621555, 3.4339872044851463, 3.4657359027997265,
    3.4965075614664802, 3.5263605246161616, 3.5553480614894135,
    3.58351893845611, 3.6109179126442243, 3.6375861597263857,
    3.6635616461296463, 3.6888794541139363, 3.713572066704308,
    3.7376696182833684, 3.7612001156935624, 3.784189633918261,
    3.8066624897703196, 3.828641396489095, 3.8501476017100584,
    3.871201010907891, 3.8918202981106265, 3.912023005428146,
    3.9318256327243257, 3.9512437185814275, 3.970291913552122,
    3.9889840465642745, 4.007333185232471, 4.02535169073515,
    4.04305126783455, 4.060443010546419, 4.07753744390572,
    4.0943445622221, 4.110873864173311, 4.127134385045092,
    4.143134726391533, 4.1588830833596715, 4.174387269895637,
    4.189654742026425, 4.204692619390966, 4.219507705176107,
    4.23410650459726, 4.248495242049359, 4.2626798770413155,
    4.276666119016055, 4.290459441148391, 4.30406509320417,
    4.31748811353631, 4.330733340286331, 4.343805421853684,
    4.356708826689592, 4.3694478524670215, 4.382026634673881,
    4.394449154672439, 4.406719247264253, 4.418840607796598,
    4.430816798843313, 4.442651256490317, 4.454347296253507,
    4.465908118654584, 4.477336814478207, 4.48863636973214,
    4.499809670330265, 4.51085950651685, 4.5217885770490405,
    4.532599493153256, 4.543294782270004, 4.553876891600541,
    4.564348191467836, 4.574710978503383, 4.584967478670572,
    4.59511985013459, 4.605170185988092, 4.61512051684126,
    4.624972813284271, 4.634728988229636, 4.6443908991413725,
    4.653960350157523, 4.663439094112067, 4.672828834461906,
    4.68213122712422, 4.6913478822291435, 4.700480365792417,
    4.709530201312334, 4.718498871295094, 4.727387818712341,
    4.736198448394496, 4.74493212836325, 4.7535901911063645,
    4.762173934797756, 4.770684624465665, 4.77912349311153,
    4.787491742782046, 4.795790545596741, 4.804021044733257,
    4.812184355372417, 4.820281565605037, 4.8283137373023015,
    4.836281906951478, 4.844187086458591, 4.852030263919617,
    4.859812404361672, 4.867534450455582, 4.875197323201151,
    4.882801922586371, 4.890349128221754, 4.897839799950911,
    4.90527477843843, 4.912654885736052, 4.919980925828125,
    4.927253685157205, 4.9344739331306915, 4.941642422609304,
    4.948759890378168, 4.955827057601261, 4.962844630259907,
    4.969813299576001, 4.976733742420574, 4.983606621708336,
    4.990432586778736, 4.997212273764115, 5.003946305945459,
    5.0106352940962555, 5.017279836814924, 5.0238805208462765,
    5.030437921392435, 5.0369526024136295, 5.043425116919247,
    5.049856007249537, 5.056245805348308, 5.062595033026967,
    5.0689042022202315, 5.075173815233827, 5.081404364984463,
    5.087596335232384, 5.093750200806762, 5.099866427824199,
    5.10594547390058, 5.111987788356544, 5.117993812416755,
    5.123963979403259, 5.1298987149230735, 5.135798437050262,
    5.14166355650266, 5.147494476813453, 5.153291594497779,
    5.159055299214529, 5.1647859739235145, 5.170483995038151,
    5.176149732573829,
];

/// `DOUBLE PRECISION FUNCTION GLOG(X)`：x >= 15 时的 ln(x)。
fn glog(x: f64) -> f64 {
    const C1: f64 = 0.286228750476730;
    const C2: f64 = 0.399999628131494;
    const C3: f64 = 0.666666666752663;
    if x >= 178.0 {
        return x.ln();
    }
    let n = x as i32;
    let nf = n as f64;
    let t = (x - nf) / (x + nf);
    let t2 = t * t;
    // `GLOG = W(N - 14) + Z`：`Z = (...)*T` 的乘法并入 `.FMA (poly, t, w)`
    horner(t2, &[C1, C2, C3, 2.0]).mul_add(t, GLOG_W[(n - 15) as usize])
}

/// `REAL FUNCTION GAM1(A)`：1/Γ(a+1) - 1，-0.5 <= a <= 1.5。
fn gam1(a: f64) -> f64 {
    const P: [f64; 7] = [
        0.577215664901533e+00,
        -0.409078193005776e+00,
        -0.230975380857675e+00,
        0.597275330452234e-01,
        0.766968181649490e-02,
        -0.514889771323592e-02,
        0.589597428611429e-03,
    ];
    const Q: [f64; 5] = [
        0.100000000000000e+01,
        0.427569613095214e+00,
        0.158451672430138e+00,
        0.261132021441447e-01,
        0.423244297896961e-02,
    ];
    const R: [f64; 9] = [
        -0.422784335098468e+00,
        -0.771330383816272e+00,
        -0.244757765222226e+00,
        0.118378989872749e+00,
        0.930357293360349e-03,
        -0.118290993445146e-01,
        0.223047661158249e-02,
        0.266505979058923e-03,
        -0.132674909766242e-03,
    ];
    const S1: f64 = 0.273076135303957e+00;
    const S2: f64 = 0.559398236957378e-01;
    let mut t = a;
    let d = a - 0.5;
    if d > 0.0 {
        t = d - 0.5;
    }
    if t < 0.0 {
        // 标号 30
        let top = horner(t, &[R[8], R[7], R[6], R[5], R[4], R[3], R[2], R[1], R[0]]);
        let bot = horner(t, &[S2, S1, 1.0]);
        let w = top / bot;
        if d > 0.0 {
            (t * w) / a
        } else {
            ((w + 0.5) + 0.5) * a
        }
    } else if t == 0.0 {
        0.0
    } else {
        // 标号 20
        let top = horner(t, &[P[6], P[5], P[4], P[3], P[2], P[1], P[0]]);
        let bot = horner(t, &[Q[4], Q[3], Q[2], Q[1], 1.0]);
        let w = top / bot;
        if d > 0.0 {
            (t / a) * ((w - 0.5) - 0.5)
        } else {
            a * w
        }
    }
}

#[cfg(test)]
#[path = "incomplete_gamma_tests.rs"]
mod incomplete_gamma_tests;
