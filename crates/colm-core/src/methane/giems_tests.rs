use super::*;

#[test]
fn fill_flags_count_as_zero_in_the_climatology() {
    // 两年：1 月一次 0.4、一次雪（−998）→ 气候态 0.2；2 月一次 NaN、一次 0.6 → 0.3。
    let mut samples = vec![0.0f32; 24];
    samples[0] = 0.4;
    samples[12] = -998.0;
    samples[1] = f32::NAN;
    samples[13] = 0.6;
    let patch = GiemsPatch::from_samples(&samples).unwrap();
    assert_eq!(patch.climatology[0], f64::from(0.4f32) / 2.0);
    assert_eq!(patch.climatology[1], f64::from(0.6f32) / 2.0);
    assert_eq!(patch.series[12], 0.0);
    assert_eq!(patch.series[1], 0.0);
}

#[test]
fn out_of_range_values_stop_like_upstream() {
    assert!(GiemsPatch::from_samples(&[1.5]).is_err());
    assert!(GiemsPatch::from_samples(&[-1.0]).is_err());
}

#[test]
fn years_outside_the_record_fall_back_to_the_climatology() {
    let samples: Vec<f32> = (0..GIEMS_MONTHS).map(|t| (t % 100) as f32 / 100.0).collect();
    let patch = GiemsPatch::from_samples(&samples).unwrap();
    // 1992-02-15（第 46 天）：第 2 个月。
    assert_eq!(patch.finundated(1992, 46).unwrap(), f64::from(samples[1]));
    // 2020-12-31（闰年第 366 天）：最后一个月。
    assert_eq!(patch.finundated(2020, 366).unwrap(), f64::from(samples[347]));
    // 2021 与 1991 退到气候态。
    assert_eq!(patch.finundated(2021, 1).unwrap(), patch.climatology[0]);
    assert_eq!(patch.finundated(1991, 365).unwrap(), patch.climatology[11]);
}
