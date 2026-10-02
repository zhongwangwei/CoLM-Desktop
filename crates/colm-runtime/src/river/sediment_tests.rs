use super::*;

#[test]
fn critical_shear_squares_follow_the_size_bands() {
    // 粗砂一段是一次幂，只乘系数（编译期把 `**1` 去掉）。
    assert_eq!(critical_shear_vel_sq(0.004), 80.9 * (0.004 * 100.0));
    assert_eq!(
        critical_shear_vel_sq(0.0002),
        8.41 * (0.0002f64 * 100.0).lpow(11.0 / 32.0)
    );
    assert_eq!(critical_shear_vel_sq(0.00001), 226.0 * (0.00001 * 100.0));
}

#[test]
fn water_accumulator_keeps_the_first_start_and_last_end() {
    let mut acc = WaterAcc::default();
    acc.add(10.0, 2.0, 1.0, 5.0, 6.0, -3.0, 7.0);
    acc.add(10.0, 1.0, 1.0, 6.0, 8.0, 1.0, 7.0);
    assert_eq!(acc.rivsto_start, 5.0);
    assert_eq!(acc.rivsto_end, 8.0);
    assert_eq!(acc.time, 20.0);
    assert_eq!(acc.v2, 50.0);
    assert_eq!(acc.rivout, -20.0);
    assert_eq!(acc.abs_rivout, 40.0);
}

#[test]
fn log10_19_is_the_folded_constant() {
    assert_eq!(LOG10_19, 19f64.log10());
}
