use super::*;

/// 与 300 位 mpmath 比：误差必须在 2⁻¹⁰⁰ 相对量级以内（远小于 f64 的一个 ulp）。
fn assert_close(actual: DoubleDouble, hi: f64, lo: f64) {
    let error = (actual.hi - hi) + (actual.lo - lo);
    assert!(
        error.abs() <= hi.abs() * 1.0e-30,
        "{actual:?} vs ({hi:e}, {lo:e}), error {error:e}"
    );
}

#[test]
fn exp_ln_and_pow_match_a_high_precision_reference() {
    // mpmath（300 位）拆成 hi + lo 的参照值。
    assert_close(
        DoubleDouble::new(1.0).exp(),
        std::f64::consts::E,
        1.4456468917292502e-16,
    );
    assert_close(
        DoubleDouble::new(10.0).ln(),
        std::f64::consts::LN_10,
        -2.1707562233822494e-16,
    );
    assert_close(
        DoubleDouble::new(2.0).powf(DoubleDouble::new(0.5)),
        std::f64::consts::SQRT_2,
        -9.667293313452913e-17,
    );
    // VIC `calc_Q12` 的第一段 `(init-resid)**(1-expt)`（expt = 0.1；参照值用两者的 double 精确值）。
    assert_close(
        DoubleDouble::new(26.487_994_682_743_185)
            .powf(DoubleDouble::new(1.0) - DoubleDouble::new(0.1)),
        19.087285629639975,
        7.197841773826158e-16,
    );
}

#[test]
fn arithmetic_keeps_the_low_word() {
    // 1 + 2⁻⁶⁰ 在 f64 里丢失，双倍双精度保住它。
    let tiny = f64::powi(2.0, -60);
    let sum = DoubleDouble::new(1.0) + DoubleDouble::new(tiny);
    assert_eq!(sum.hi, 1.0);
    assert_eq!(sum.lo, tiny);
    let back = sum - DoubleDouble::new(1.0);
    assert_eq!(back.to_f64(), tiny);
    let third = DoubleDouble::new(1.0) / DoubleDouble::new(3.0);
    assert_close(third * DoubleDouble::new(3.0), 1.0, 0.0);
}
