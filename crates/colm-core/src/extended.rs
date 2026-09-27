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

#[cfg(test)]
#[path = "extended_tests.rs"]
mod extended_tests;
