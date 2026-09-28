//! 双倍双精度（double-double，约 106 位有效数字），用来复现内核里被提升成
//! **四倍精度**的那几处表达式。
//!
//! 上游全部用 `-fdefault-real-8` 编译而**没有** `-fdefault-double-8`，于是源码里的
//! `1.0d0` 这类 `DOUBLE PRECISION` 字面量被提升成 `real(kind=16)`，所在表达式整条
//! 在 binary128 里求值（`powq` 等），最后才转回 double。全内核的 GIMPLE 里有 7 个模块
//! 这样（VIC 的 `calc_Q12`、PC 三维冠层辐射、不完全 Gamma、城市植被长短波、
//! `MOD_Utils` 的 Levenberg–Marquardt、PROSPECT），见 `docs/upstream-bugs.md` 第 17 条。
//!
//! 用 106 位代替 113 位：这些表达式里最坏的相消只吃掉十几位，转回 f64 时与 binary128
//! 的结果相同，除非精确值恰好落在 f64 舍入中点附近约 2⁻⁴⁰ 的范围内。
//! `exp`/`ln` 按 QD 库的做法：`exp` 先按 ln2 取整缩放、再二分 10 次后用 Taylor 级数，
//! `ln` 用一次以 f64 为初值的 Newton 迭代（每次把精度翻倍）再补一次。

use std::ops::{Add, Div, Mul, Neg, Sub};

/// `hi + lo`，`|lo| <= ulp(hi)/2`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoubleDouble {
    pub hi: f64,
    pub lo: f64,
}

const LN2: DoubleDouble = DoubleDouble {
    hi: std::f64::consts::LN_2,
    lo: 2.319_046_813_846_299_6e-17,
};

impl DoubleDouble {
    pub const fn new(value: f64) -> Self {
        Self { hi: value, lo: 0.0 }
    }

    /// 转回 f64：`hi + lo` 的正确舍入（`|lo| <= ulp(hi)/2` 时就是 `hi + lo`）。
    pub fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    fn normalized(hi: f64, lo: f64) -> Self {
        let sum = hi + lo;
        Self {
            hi: sum,
            lo: lo - (sum - hi),
        }
    }

    pub fn exp(self) -> Self {
        if self.hi == 0.0 && self.lo == 0.0 {
            return Self::new(1.0);
        }
        let k = (self.hi / std::f64::consts::LN_2).round();
        let reduced = self - LN2 * Self::new(k);
        // 再缩小 2¹⁰ 倍，让 Taylor 级数在 ~25 项内收敛到 2⁻¹⁰⁶ 以下。
        const HALVINGS: i32 = 10;
        let scale = f64::powi(2.0, -HALVINGS);
        let r = Self {
            hi: reduced.hi * scale,
            lo: reduced.lo * scale,
        };
        let mut term = r;
        let mut sum = r;
        for n in 2..40 {
            term = term * r / Self::new(n as f64);
            sum = sum + term;
            if term.hi.abs() < 1.0e-36 {
                break;
            }
        }
        // (1 + sum)^(2^HALVINGS) = exp(r·2^HALVINGS)：每次 (1+s)² - 1 = 2s + s²，保住低位。
        for _ in 0..HALVINGS {
            sum = sum * Self::new(2.0) + sum * sum;
        }
        let result = sum + Self::new(1.0);
        let factor = f64::powi(2.0, k as i32);
        Self {
            hi: result.hi * factor,
            lo: result.lo * factor,
        }
    }

    /// 自然对数，要求 `self > 0`。
    pub fn ln(self) -> Self {
        let mut y = Self::new(self.hi.ln());
        for _ in 0..2 {
            // Newton：y ← y + x·exp(-y) - 1
            y = y + self * (-y).exp() - Self::new(1.0);
        }
        y
    }

    /// `self ** exponent`（`self > 0`），即 `exp(exponent·ln(self))`。
    pub fn powf(self, exponent: Self) -> Self {
        (exponent * self.ln()).exp()
    }
}

impl From<f64> for DoubleDouble {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

impl Neg for DoubleDouble {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            hi: -self.hi,
            lo: -self.lo,
        }
    }
}

impl Add for DoubleDouble {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        // Knuth two-sum 加上两个低位。
        let sum = self.hi + other.hi;
        let shifted = sum - self.hi;
        let error = (self.hi - (sum - shifted)) + (other.hi - shifted);
        Self::normalized(sum, error + self.lo + other.lo)
    }
}

impl Sub for DoubleDouble {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self + (-other)
    }
}

impl Mul for DoubleDouble {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        let product = self.hi * other.hi;
        let error = self.hi.mul_add(other.hi, -product);
        Self::normalized(product, error + (self.hi * other.lo + self.lo * other.hi))
    }
}

impl Div for DoubleDouble {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        let first = self.hi / other.hi;
        let remainder = self - other * Self::new(first);
        let second = remainder.hi / other.hi;
        let remainder = remainder - other * Self::new(second);
        let third = remainder.hi / other.hi;
        Self::normalized(first, second) + Self::new(third)
    }
}

/// `MOD_3DCanopyRadiation:tee`：`DDH*(DD1/tau/tau-(DD1/tau/tau+DD2/tau)*exp(-DD2*tau))`，
/// 参数与全部常数都是 `real(r16)`，整条在四精度里求值，返回时舍入到 real(8)。
///
/// 小 `tau` 时两项相消（`tau = 1e-3` 吃掉约 20 位），106 位仍远多于 53 位。
/// 调用方要把 `tau` 本身也按四精度给：城市的 `tee(DD1*3/8.*lsai)` 里 `lsai*0.375`
/// 在 f64 下不一定精确，在四精度下是（GIMPLE：`(real(kind=16)) lsai * 3.75e-1`）。
pub fn tee(tau: DoubleDouble) -> f64 {
    let one_over_square = DoubleDouble::new(1.0) / tau / tau;
    let two = DoubleDouble::new(2.0);
    (DoubleDouble::new(0.5)
        * (one_over_square - (one_over_square + two / tau) * (-(two * tau)).exp()))
    .to_f64()
}

/// 编译期折好的四精度常数（`bb = 1.74_r8` 提升为 real(16) 后的 `1/(bb+1)` 等），
/// 取自 `MOD_3DCanopyRadiation.F90:1374/1379` 的 GIMPLE，拆成 `hi + lo`（与四精度相差 ~1e-33）。
const fn dd(hi: u64, lo: u64) -> DoubleDouble {
    DoubleDouble {
        hi: f64::from_bits(hi),
        lo: f64::from_bits(lo),
    }
}
const INV_BB_PLUS_1: DoubleDouble = dd(0x3fd7_5b8f_e21a_291c, 0x3c49_dc4a_b0a1_b5c6);
const INV_BB_MINUS_1: DoubleDouble = dd(0x3ff5_9f22_9837_59f2, 0x3c8e_1b4d_3ae7_80d7);
const TWO_INV_BB2_MINUS_1: DoubleDouble = dd(0x3fef_907d_3f61_9f56, 0x3c8c_7d88_8fdd_657b);
const TWO_BB_INV_BB2_MINUS_1: DoubleDouble = dd(0x3ffb_7606_90bd_e439, 0xbc80_fda1_ec67_7961);
const INV_SQ_SUM: DoubleDouble = dd(0x3fff_597e_29a9_b528, 0x3c77_929d_cbe0_0dc0);
const INV_BB_MINUS_1_SQ: DoubleDouble = dd(0x3ffd_37e9_8f6d_64cb, 0xbc78_d1f6_91ba_1542);
const INV_BB_PLUS_1_SQ: DoubleDouble = dd(0x3fc1_0ca4_d1e2_82ea, 0xbc6f_36d7_44cb_b9fd);
/// `DD1 + bb` 与 `bb + DD2`（`1.74_r8` 在四精度里加整数，结果不是 f64 能表示的）
const BB_PLUS_1: DoubleDouble = dd(0x4005_eb85_1eb8_51ec, 0xbcb0_0000_0000_0000);
const BB_PLUS_2: DoubleDouble = dd(0x400d_eb85_1eb8_51ec, 0xbcb0_0000_0000_0000);

/// `MOD_3DCanopyRadiation:phi(.true., tau, omg, tau_p, rho_p, phi_tot, phi_dif, pa2)`：
/// 返回 `(phi_tot, phi_dif, pa2)`。
///
/// 每条语句按 GIMPLE（`:1350-1412`）分精度：凡是碰到 `DD*`（real(16)）常数的运算在四精度里做，
/// 赋给 real(8) 变量时舍入；`tee` 返回 real(8)。纯 real(8) 的子式照写，而且**没有 FMA**
/// （`(rho_p*phi_1b + tau_p*phi_1f)/(tau_p+rho_p)`、`omg*pac*phi_2a` 都是分开舍入）。
pub fn canopy_scattering_runmode(tau: f64, omg: f64, tau_p: f64, rho_p: f64) -> (f64, f64, f64) {
    let q = DoubleDouble::new;
    let t = q(tau);
    // `:1350` `phi_1f = DD1/tau/tau - (DD1/tau/tau + DD2/tau + DD2)*exp(-DD2*tau)`
    let inv2 = q(1.0) / t / t;
    let phi_1f = (inv2 - ((inv2 + q(2.0) / t) + q(2.0)) * (-(t * q(2.0))).exp()).to_f64();
    // `tee` 的实参都是四精度
    let tee_2 = tee(t * q(2.0));
    let tee_1 = tee(t);
    let tee_b1 = tee(t * BB_PLUS_1);
    let tee_bb = tee(t * q(1.74));
    let tee_b2 = tee(t * BB_PLUS_2);
    // `:1353` `phi_1b = DDH*(DD1 - tee(DD2*tau))`
    let phi_1b = ((q(1.0) - q(tee_2)) * q(0.5)).to_f64();
    // `:1374` `phi_2b = aa*(1/(bb+1) - 1/(bb-1)*tee(2τ) + 2/(bb+1)/(bb-1)*tee((1+bb)τ))`
    let phi_2b = (((INV_BB_PLUS_1 - q(tee_2) * INV_BB_MINUS_1) + q(tee_b1) * TWO_INV_BB2_MINUS_1)
        * q(0.7))
    .to_f64();
    // `:1379`
    let phi_2f = ((((q(phi_1f) * TWO_BB_INV_BB2_MINUS_1) - q(tee_1) * INV_SQ_SUM)
        + q(tee_bb) * INV_BB_MINUS_1_SQ)
        + q(tee_b2) * INV_BB_PLUS_1_SQ)
        * q(0.7);
    let phi_2f = phi_2f.to_f64();
    // `:1383` real(8)：`(phi_2b + phi_2f)*0.5`
    let phi_2a = (phi_2b + phi_2f) * 0.5;
    // `:1392` `pac = DD1 - phi_2a/(DD1 - tee(tau) - (rho_p*phi_1b + tau_p*phi_1f)/(tau_p+rho_p))`
    let backward = rho_p * phi_1b;
    let forward = tau_p * phi_1f;
    let ratio = (backward + forward) / (rho_p + tau_p);
    let pac = (q(1.0) - q(phi_2a) / ((q(1.0) - q(tee_1)) - q(ratio))).to_f64();
    // `max(min(pac, D1), D0)`：保留两步的写法（`clamp` 在 NaN 上语义不同）
    #[allow(clippy::manual_clamp)]
    let pac = pac.min(1.0).max(0.0);
    // `:1401-1402`：`omg*pac`、`*phi_2a` 在 real(8)，除法与加法在四精度
    let scattered = omg * pac;
    let multiple = q(scattered * phi_2a) / (q(1.0) - q(scattered));
    let phi_mf = (q(phi_2f) + multiple).to_f64();
    let phi_mb = (multiple + q(phi_2b)).to_f64();
    // `:1408-1409` `tau_p*phi_1f + DDH*omg*omg*phi_mf`：`omg*(omg*0.5)` 在四精度
    let half_omega_sq = q(omg) * (q(omg) * q(0.5));
    let phi_tf = (q(forward) + half_omega_sq * q(phi_mf)).to_f64();
    let phi_tb = (q(backward) + half_omega_sq * q(phi_mb)).to_f64();
    (phi_tf + phi_tb, phi_tf - phi_tb, pac)
}

#[cfg(test)]
#[path = "extended_tests.rs"]
mod extended_tests;
