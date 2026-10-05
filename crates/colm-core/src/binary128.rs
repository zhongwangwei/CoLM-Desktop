//! IEEE 754 binary128（gfortran `real(16)`）的软件实现：加、减、乘、除、比较与 f64 互转，
//! 一律按"就近、平局取偶"舍入。
//!
//! 用在上游**显式**声明成 `real(r16)` 的累加上（`MOD_Catch_RiverLakeFlow` 的界面通量与上游求和）。
//! 那里的结果要先在 113 位上舍入、再转回 f64，是双重舍入；[`crate::extended::DoubleDouble`]
//! 的 106 位做不到逐位相同，所以这里按 binary128 的定义逐位实现。稳定版 Rust 还没有 `f128`。
//!
//! 只实现用得到的有限数与零；NaN、无穷按 IEEE 传播，但不区分 NaN 载荷。
//! 正确性由 gfortran `real(16)` 生成的向量逐位核对（`binary128_tests.rs`）。

use std::cmp::Ordering;
use std::ops::{Add, Div, Mul, Neg, Sub};

const SIG_BITS: u32 = 112; // 显式尾数位数
const EXP_BIAS: i32 = 16383;
const EXP_MAX: i32 = 0x7fff;
const SIG_MASK: u128 = (1u128 << SIG_BITS) - 1;
const HIDDEN: u128 = 1u128 << SIG_BITS;

/// binary128 的位模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quad(pub u128);

/// 拆开后的有限数：`(-1)^sign · sig · 2^exp`。
#[derive(Debug, Clone, Copy)]
struct Unpacked {
    sign: bool,
    exp: i32,
    sig: u128,
}

enum Kind {
    Zero(bool),
    Finite(Unpacked),
    Inf(bool),
    Nan,
}

impl Quad {
    pub const ZERO: Quad = Quad(0);

    fn sign(self) -> bool {
        self.0 >> 127 == 1
    }

    fn kind(self) -> Kind {
        let sign = self.sign();
        let biased = ((self.0 >> SIG_BITS) & EXP_MAX as u128) as i32;
        let frac = self.0 & SIG_MASK;
        match biased {
            0 if frac == 0 => Kind::Zero(sign),
            0 => Kind::Finite(Unpacked {
                sign,
                exp: 1 - EXP_BIAS - SIG_BITS as i32,
                sig: frac,
            }),
            EXP_MAX if frac == 0 => Kind::Inf(sign),
            EXP_MAX => Kind::Nan,
            _ => Kind::Finite(Unpacked {
                sign,
                exp: biased - EXP_BIAS - SIG_BITS as i32,
                sig: frac | HIDDEN,
            }),
        }
    }

    fn zero(sign: bool) -> Quad {
        Quad(u128::from(sign) << 127)
    }

    fn inf(sign: bool) -> Quad {
        Quad((u128::from(sign) << 127) | ((EXP_MAX as u128) << SIG_BITS))
    }

    fn nan() -> Quad {
        Quad(((EXP_MAX as u128) << SIG_BITS) | (1u128 << (SIG_BITS - 1)))
    }

    pub fn is_nan(self) -> bool {
        matches!(self.kind(), Kind::Nan)
    }

    /// f64 → binary128，精确。
    pub fn from_f64(value: f64) -> Quad {
        let bits = value.to_bits();
        let sign = bits >> 63 == 1;
        let biased = ((bits >> 52) & 0x7ff) as i32;
        let frac = u128::from(bits & ((1u64 << 52) - 1));
        match biased {
            0 if frac == 0 => Quad::zero(sign),
            0x7ff if frac == 0 => Quad::inf(sign),
            0x7ff => Quad::nan(),
            0 => round_pack(sign, -1022 - 52, frac, false),
            _ => round_pack(sign, biased - 1023 - 52, frac | (1u128 << 52), false),
        }
    }

    /// binary128 → f64，就近取偶（含 f64 的非规格化与溢出）。
    pub fn to_f64(self) -> f64 {
        match self.kind() {
            Kind::Zero(sign) => {
                if sign {
                    -0.0
                } else {
                    0.0
                }
            }
            Kind::Inf(sign) => {
                if sign {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            }
            Kind::Nan => f64::NAN,
            Kind::Finite(u) => round_to_f64(u),
        }
    }
}

impl From<f64> for Quad {
    fn from(value: f64) -> Self {
        Quad::from_f64(value)
    }
}

/// 把 `(-1)^sign · sig · 2^exp` 舍入成 binary128。`sticky` 表示 `sig` 之下还有非零位（数值比
/// `sig · 2^exp` 略大一点点）；它只在舍入判断里起作用：余数正好一半时有粘滞位就进位。
fn round_pack(sign: bool, exp: i32, sig: u128, sticky: bool) -> Quad {
    if sig == 0 {
        return Quad::zero(sign);
    }
    let len = 128 - sig.leading_zeros() as i32; // 位长
                                                // value 在 [2^top, 2^(top+1))。
    let top = exp + len - 1;
    let mut biased = top + EXP_BIAS;
    // 目标：113 位有效数字；非规格化时少几位。
    let mut keep = SIG_BITS as i32 + 1;
    if biased <= 0 {
        keep -= 1 - biased;
        biased = 0;
    }
    let shift = len - keep; // >0 右移舍入，<=0 左移（精确，粘滞位只会出现在右移的路径上）
    let mut result = if shift <= 0 {
        debug_assert!(
            !sticky,
            "a short significand never carries a sticky bit here"
        );
        sig << (-shift) as u32
    } else if keep <= 0 {
        // 不到最小非规格化数：keep == 0 时最高位正好是半个最小单位。
        let half = 1u128 << (len - 1);
        let above_half = keep == 0 && (sig > half || (sig == half && sticky));
        return if above_half {
            Quad((u128::from(sign) << 127) | 1)
        } else {
            Quad::zero(sign)
        };
    } else {
        let s = shift as u32;
        let rest = if s >= 128 {
            sig
        } else {
            sig & ((1u128 << s) - 1)
        };
        let half = 1u128 << (s - 1);
        let mut q = if s >= 128 { 0 } else { sig >> s };
        if rest > half || (rest == half && (sticky || q & 1 == 1)) {
            q += 1;
        }
        q
    };
    // 进位溢出到下一位。
    if biased == 0 {
        if result >> SIG_BITS != 0 {
            // 非规格化进位成最小规格化数：编码自然衔接。
            biased = 1;
            result &= SIG_MASK;
        }
    } else if result >> (SIG_BITS + 1) != 0 {
        result >>= 1;
        biased += 1;
    }
    if biased >= EXP_MAX {
        return Quad::inf(sign);
    }
    let frac = result & SIG_MASK;
    Quad((u128::from(sign) << 127) | ((biased as u128) << SIG_BITS) | frac)
}

fn round_to_f64(u: Unpacked) -> f64 {
    let len = 128 - u.sig.leading_zeros() as i32;
    let top = u.exp + len - 1;
    let mut biased = top + 1023;
    let mut keep = 53;
    if biased <= 0 {
        keep -= 1 - biased;
        biased = 0;
    }
    let shift = len - keep;
    let mut result: u128 = if shift <= 0 {
        u.sig << (-shift) as u32
    } else if keep <= 0 {
        let above_half = keep == 0 && u.sig > (1u128 << (len - 1));
        let tiny = if above_half { f64::from_bits(1) } else { 0.0 };
        return if u.sign { -tiny } else { tiny };
    } else {
        let s = shift as u32;
        let rest = u.sig & ((1u128 << s) - 1);
        let half = 1u128 << (s - 1);
        let mut q = u.sig >> s;
        if rest > half || (rest == half && q & 1 == 1) {
            q += 1;
        }
        q
    };
    if biased == 0 {
        if result >> 52 != 0 {
            biased = 1;
            result &= (1u128 << 52) - 1;
        }
    } else if result >> 53 != 0 {
        result >>= 1;
        biased += 1;
    }
    if biased >= 0x7ff {
        return if u.sign {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let bits =
        (u64::from(u.sign) << 63) | ((biased as u64) << 52) | (result as u64 & ((1u64 << 52) - 1));
    f64::from_bits(bits)
}

/// `sig >> shift`，被移出的非零位并进最低位（"jamming"）。
fn shift_right_jam(sig: u128, shift: u32) -> u128 {
    if shift == 0 {
        sig
    } else if shift >= 128 {
        u128::from(sig != 0)
    } else {
        (sig >> shift) | u128::from(sig & ((1u128 << shift) - 1) != 0)
    }
}

fn add_finite(a: Unpacked, b: Unpacked) -> Quad {
    // 让 a 的量级不小于 b（按最高位的指数比较）。
    let top = |u: Unpacked| u.exp + 127 - u.sig.leading_zeros() as i32;
    let (a, b) =
        if (top(a), a.sig << a.sig.leading_zeros()) >= (top(b), b.sig << b.sig.leading_zeros()) {
            (a, b)
        } else {
            (b, a)
        };
    // 两边都左移到第 126 位，留足保护位；b 再按指数差右移并粘滞。
    let la = a.sig.leading_zeros() - 1;
    let lb = b.sig.leading_zeros() - 1;
    let sa = a.sig << la;
    let ea = a.exp - la as i32;
    let sb = b.sig << lb;
    let eb = b.exp - lb as i32;
    let diff = (ea - eb) as u32; // a 量级不小，ea >= eb
    let sb = shift_right_jam(sb, diff);
    if a.sign == b.sign {
        // 两个不超过 2^127 的数相加可能进到第 127 位：先各右移一位（带粘滞）。
        let sum = (sa >> 1) + (sb >> 1) + (sa & sb & 1);
        let low = (sa ^ sb) & 1;
        round_pack(a.sign, ea + 1, sum, low != 0)
    } else {
        let difference = sa - sb;
        if difference == 0 {
            return Quad::zero(false);
        }
        round_pack(a.sign, ea, difference, false)
    }
}

impl Add for Quad {
    type Output = Quad;
    fn add(self, other: Quad) -> Quad {
        match (self.kind(), other.kind()) {
            (Kind::Nan, _) | (_, Kind::Nan) => Quad::nan(),
            (Kind::Inf(s), Kind::Inf(t)) => {
                if s == t {
                    Quad::inf(s)
                } else {
                    Quad::nan()
                }
            }
            (Kind::Inf(s), _) | (_, Kind::Inf(s)) => Quad::inf(s),
            (Kind::Zero(s), Kind::Zero(t)) => Quad::zero(s && t),
            (Kind::Zero(_), _) => other,
            (_, Kind::Zero(_)) => self,
            (Kind::Finite(a), Kind::Finite(b)) => add_finite(a, b),
        }
    }
}

impl Neg for Quad {
    type Output = Quad;
    fn neg(self) -> Quad {
        Quad(self.0 ^ (1u128 << 127))
    }
}

impl Sub for Quad {
    type Output = Quad;
    fn sub(self, other: Quad) -> Quad {
        self + (-other)
    }
}

/// 128×128 → 256 位乘积，返回 (高, 低)。
fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let (a1, a0) = (a >> 64, a & u128::from(u64::MAX));
    let (b1, b0) = (b >> 64, b & u128::from(u64::MAX));
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & u128::from(u64::MAX)) + (p10 & u128::from(u64::MAX));
    let lo = (p00 & u128::from(u64::MAX)) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

impl Mul for Quad {
    type Output = Quad;
    fn mul(self, other: Quad) -> Quad {
        let sign = self.sign() != other.sign();
        match (self.kind(), other.kind()) {
            (Kind::Nan, _) | (_, Kind::Nan) => Quad::nan(),
            (Kind::Inf(_), Kind::Zero(_)) | (Kind::Zero(_), Kind::Inf(_)) => Quad::nan(),
            (Kind::Inf(_), _) | (_, Kind::Inf(_)) => Quad::inf(sign),
            (Kind::Zero(_), _) | (_, Kind::Zero(_)) => Quad::zero(sign),
            (Kind::Finite(a), Kind::Finite(b)) => {
                let (hi, lo) = mul_wide(a.sig, b.sig);
                let exp = a.exp + b.exp;
                if hi == 0 {
                    // 乘积不到 128 位：只有非规格化输入才会，最多 126 位时直接舍入。
                    let shift = (128 - lo.leading_zeros()).saturating_sub(126);
                    let sticky = shift > 0 && lo & ((1u128 << shift) - 1) != 0;
                    return round_pack(sign, exp + shift as i32, lo >> shift, sticky);
                }
                // 取最高 126 位，其余折成粘滞位。
                let len = 256 - hi.leading_zeros();
                let shift = len - 126; // >= 2
                let (top, sticky) = if shift >= 128 {
                    let s = shift - 128;
                    let rest_hi = if s == 0 { 0 } else { hi & ((1u128 << s) - 1) };
                    (hi >> s, rest_hi != 0 || lo != 0)
                } else {
                    let top = (hi << (128 - shift)) | (lo >> shift);
                    (top, lo & ((1u128 << shift) - 1) != 0)
                };
                round_pack(sign, exp + shift as i32, top, sticky)
            }
        }
    }
}

impl Div for Quad {
    type Output = Quad;
    fn div(self, other: Quad) -> Quad {
        let sign = self.sign() != other.sign();
        match (self.kind(), other.kind()) {
            (Kind::Nan, _) | (_, Kind::Nan) => Quad::nan(),
            (Kind::Inf(_), Kind::Inf(_)) | (Kind::Zero(_), Kind::Zero(_)) => Quad::nan(),
            (Kind::Inf(_), _) | (_, Kind::Zero(_)) => Quad::inf(sign),
            (Kind::Zero(_), _) | (_, Kind::Inf(_)) => Quad::zero(sign),
            (Kind::Finite(a), Kind::Finite(b)) => {
                // 两边都规格化到最高位在第 112 位。
                let na = a.sig.leading_zeros() as i32 - 15;
                let nb = b.sig.leading_zeros() as i32 - 15;
                let sa = a.sig << na;
                let sb = b.sig << nb;
                let ea = a.exp - na;
                let eb = b.exp - nb;
                // q = floor(sa·2^120 / sb)，逐位长除；r < sb < 2^113，r<<1 不溢出。
                let mut r = sa;
                let mut q: u128 = 0;
                // 先处理整数部分（sa/sb ∈ [0.5, 2)）。
                if r >= sb {
                    q = 1;
                    r -= sb;
                }
                for _ in 0..120 {
                    r <<= 1;
                    q <<= 1;
                    if r >= sb {
                        r -= sb;
                        q |= 1;
                    }
                }
                round_pack(sign, ea - eb - 120, q, r != 0)
            }
        }
    }
}

impl PartialOrd for Quad {
    fn partial_cmp(&self, other: &Quad) -> Option<Ordering> {
        if self.is_nan() || other.is_nan() {
            return None;
        }
        let magnitude = |q: Quad| q.0 & !(1u128 << 127);
        let (a, b) = (magnitude(*self), magnitude(*other));
        if a == 0 && b == 0 {
            return Some(Ordering::Equal);
        }
        Some(match (self.sign(), other.sign()) {
            (false, false) => a.cmp(&b),
            (true, true) => b.cmp(&a),
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
        })
    }
}

impl Quad {
    /// Fortran `max(a, b)`（`MAX_EXPR`，两数都不是 NaN 时取大者；相等时取第一个）。
    pub fn max(self, other: Quad) -> Quad {
        if other > self {
            other
        } else {
            self
        }
    }

    /// Fortran `min(a, b)`。
    pub fn min(self, other: Quad) -> Quad {
        if other < self {
            other
        } else {
            self
        }
    }
}

#[cfg(test)]
#[path = "binary128_tests.rs"]
mod binary128_tests;
