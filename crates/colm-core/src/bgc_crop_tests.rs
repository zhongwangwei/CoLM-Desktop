use super::*;

/// 折叠常数与它们的定义自洽（误差在正确舍入量级之内）：`alpha = ln2/ln((15.7+1.3)/(4.9+1.3))`。
#[test]
fn folded_constants_match_their_definitions() {
    let alpha = 2.0_f64.ln() / ((15.7_f64 + 1.3) / (4.9 + 1.3)).ln();
    assert!((alpha - ALPHA).abs() <= 2.0 * f64::EPSILON * ALPHA);
    assert!((2.0 * ALPHA - TWO_ALPHA).abs() <= f64::EPSILON * TWO_ALPHA);
    assert!(
        ((4.9_f64 + 1.3).powf(ALPHA) - OPT_POW_ALPHA).abs() <= 4.0 * f64::EPSILON * OPT_POW_ALPHA
    );
    assert!(
        ((4.9_f64 + 1.3).powf(TWO_ALPHA) - OPT_POW_TWO_ALPHA).abs()
            <= 4.0 * f64::EPSILON * OPT_POW_TWO_ALPHA
    );
    assert_eq!(22.5_f64.powi(5), VF_HALF_POW_5);
}
