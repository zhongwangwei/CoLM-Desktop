//! 融合乘加的策略。
//!
//! 数值 crate 里原来直接写 `a.mul_add(b, c)`，因为 gfortran 的 `-O2` 在默认就带 FMA 指令的目标（arm64）上会把
//! `a*b + c` 收缩成一条融合指令，Rust 要逐位对齐就得照着收缩。把所有这些调用统一成 [`Contract::contract`]
//! 之后，收缩与否只在这里决定一次，**按目标架构选默认**，与同一目标上 gfortran 的默认构建一致：
//!
//! - `aarch64`：融合。Rust 里的每一处融合都是照 arm64 gfortran 的 GIMPLE 逐条对出来的。
//! - 其余（x86-64 基线没有 FMA 指令，gfortran 不收缩）：`a * b + c`，乘与加各舍入一次。
//!
//! 特性 `fused` / `unfused` 可以强行指定（`unfused` 在 aarch64 上用来对照；`fused` 在 x86-64 上对应
//! 用 `-mfma` 编译的 Fortran 内核，但融合决策依赖目标平台，那条路并不逐位一致，只在调试时用）。
//! 同时指定两个特性是编译错误。
//!
//! 融合在没有 FMA 硬件的 CPU 上走 libm 的 `fma()`，结果仍是精确舍入，所以同一策略下各平台上的 Rust 数值一致。

#[cfg(all(feature = "fused", feature = "unfused"))]
compile_error!("colm-numeric: features `fused` and `unfused` are mutually exclusive");

/// 乘加是否融合（编译期常量）。
pub const FUSED: bool =
    cfg!(feature = "fused") || (!cfg!(feature = "unfused") && cfg!(target_arch = "aarch64"));

/// `self * mul + add`，按 [`FUSED`] 决定一次舍入还是两次。
pub trait Contract: Sized {
    fn contract(self, mul: Self, add: Self) -> Self;
}

impl Contract for f64 {
    #[inline(always)]
    fn contract(self, mul: f64, add: f64) -> f64 {
        if FUSED {
            self.mul_add(mul, add)
        } else {
            self * mul + add
        }
    }
}

impl Contract for f32 {
    #[inline(always)]
    fn contract(self, mul: f32, add: f32) -> f32 {
        if FUSED {
            self.mul_add(mul, add)
        } else {
            self * mul + add
        }
    }
}

/// 测试里用：参照值来自 arm64（融合乘加）的 gfortran 时，在不融合的目标上直接跳过。
///
/// 这类夹具逐位比对 Mac 上编出来的 Fortran 结果；Rust 在 x86-64 上不融合（与 x86-64 的 gfortran 默认构建一致），
/// 数值差一两个 ULP 是预期的，不是错误。x86-64 上的逐位一致由对拍整个 Fortran 内核来保证
/// （`ws-parity`，docs/implementation-verification.md 第 649 轮）。
#[macro_export]
macro_rules! skip_unless_fused {
    () => {
        if !$crate::FUSED {
            eprintln!(
                "skipped: the reference values come from arm64 gfortran (fused multiply-add); \
                 this build is unfused"
            );
            return;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 选一组乘积需要多于 53 位才能表示的数：融合与不融合的结果差一个 ULP。
    #[test]
    fn the_policy_matches_the_feature() {
        let (a, b, c) = (
            1.0 + 2f64.powi(-30),
            1.0 + 2f64.powi(-30),
            -1.0 - 2f64.powi(-29),
        );
        let fused = a.mul_add(b, c);
        let plain = a * b + c;
        assert_ne!(fused, plain, "the probe must separate the two roundings");
        assert_eq!(a.contract(b, c), if FUSED { fused } else { plain });
        let expected = match (cfg!(feature = "fused"), cfg!(feature = "unfused")) {
            (true, _) => true,
            (_, true) => false,
            _ => cfg!(target_arch = "aarch64"),
        };
        assert_eq!(FUSED, expected);
        // 与融合无关的情形两者相同。
        assert_eq!(2.0f64.contract(3.0, 1.0), 7.0);
        assert_eq!(2.0f32.contract(3.0, 1.0), 7.0);
    }
}
