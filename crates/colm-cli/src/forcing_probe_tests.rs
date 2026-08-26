use super::present;

#[test]
fn probe_never_serializes_non_finite_heights() {
    assert_eq!(present(f64::NAN), None);
    assert_eq!(present(f64::INFINITY), None);
    assert_eq!(present(10.0), Some(10.0));
}
