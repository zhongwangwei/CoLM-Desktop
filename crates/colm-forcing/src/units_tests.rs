//! 单位换算的测试。
//!
//! **换算是这条管道里最容易出错、也最难发现的一环** —— 温度差 273.15、
//! 降水差 3600 倍，跑出来的结果都还在「看着像那么回事」的范围内。

#[test]
fn celsius_becomes_kelvin() {
    // **这里不能用 `assert_eq!` 比字面量。** `-40.0 + 273.15` 与直接写
    // 下的字面量 `233.15` 差 1 ULP —— `273.15` 的最近 f64 比真值小，
    // `233.15` 的最近 f64 比真值大，两边独立舍入的方向不一致，加法
    // 补不平这个缝。这正是模块文档说的「最容易出错、也最难发现」：
    // 换算公式本身没错，逐位比较字面量却会撞上二进制小数的老问题。
    let v = super::convert_units("degC", "K", &[0.0, 25.0, -40.0]).unwrap();
    let want = [273.15, 298.15, 233.15];
    for (got, want) in v.iter().zip(want.iter()) {
        assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
    }
}

#[test]
fn an_already_correct_unit_is_returned_untouched() {
    // **不是「换算成自己」，是原样返回。** 乘 1.0 再加 0.0 会让
    // 非规格化的浮点值发生变化，而这条管道的地基正是逐位复现。
    let vals = [1.8337343205163141, 0.1 + 0.2];
    let v = super::convert_units("K", "K", &vals).unwrap();
    assert_eq!(v, vals.to_vec());
}

#[test]
fn hourly_accumulated_precipitation_becomes_a_rate() {
    // mm/hr → mm/s（CoLM 要的是率）
    let v = super::convert_units("mm/hr", "mm/s", &[3.6]).unwrap();
    assert!((v[0] - 0.001).abs() < 1e-12, "got {}", v[0]);
}

#[test]
fn an_unknown_pair_is_refused_rather_than_silently_passed_through() {
    // **拒绝比放行安全。** 放行一个不认识的单位，模型会拿着量纲错误的
    // 数跑完，而界面上什么都看不出来。
    let e = super::convert_units("furlongs", "K", &[1.0]).unwrap_err();
    assert!(
        e.to_string().contains("furlongs"),
        "报错要点名那个单位：{e}"
    );
}

#[test]
fn a_rename_only_conversion_does_not_touch_the_numbers() {
    // `kg/m2/s` 与 `mm/s` 是同一个量的两个名字（水的密度 1000 kg/m3，
    // 1 kg/m2 就是 1 mm 水深）。**改名不该动数** —— 走 `* 1.0 + 0.0`
    // 会把 `-0.0` 变成 `0.0`，而这条管道的地基是逐位复现。
    let vals = [1.8337343205163141, 0.1 + 0.2, -0.0];
    let v = super::convert_units("mm/s", "kg/m2/s", &vals).unwrap();
    assert_eq!(v, vals.to_vec());
    // `assert_eq!` 认为 -0.0 == 0.0，所以符号位要单独验。
    assert!(v[2].is_sign_negative(), "-0.0 的符号位不该丢：{}", v[2]);
}

#[test]
fn accumulated_precipitation_becomes_the_rate_colm_reads() {
    // **目标是 `kg/m2/s` 而不是 `mm/s`** —— PLUMBER2 的 `Precip` 就是它
    // （实测 90 个站全是），黄金回归那条直读路径上 CoLM 拿到的也是它。
    // 转换产物标成别的，同一个模型就要认两套单位。
    let v = super::convert_units("mm/hr", "kg/m2/s", &[3.6]).unwrap();
    assert!((v[0] - 0.001).abs() < 1e-12, "got {}", v[0]);
}

#[test]
fn common_cf_unit_spellings_convert_without_changing_values() {
    for (from, to) in [
        ("kg m-2 s-1", "kg/m2/s"),
        ("kg kg-1", "kg/kg"),
        ("m s-1", "m/s"),
        ("W m-2", "W/m2"),
    ] {
        assert_eq!(super::convert_units(from, to, &[1.25]).unwrap(), [1.25]);
        assert_eq!(super::from_canonical(to, from, &[1.25]).unwrap(), [1.25]);
    }
}

#[test]
fn interval_accumulated_precipitation_uses_the_actual_time_step() {
    let half_hour = super::convert_units_with_step("mm", "kg/m2/s", &[1.8], Some(1800.0)).unwrap();
    assert!((half_hour[0] - 0.001).abs() < 1e-12);
    let hourly = super::convert_units_with_step("kg m-2", "kg/m2/s", &[3.6], Some(3600.0)).unwrap();
    assert!((hourly[0] - 0.001).abs() < 1e-12);
}

#[test]
fn interval_accumulation_without_a_time_step_is_refused() {
    let error = super::convert_units_with_step("mm", "kg/m2/s", &[1.0], None).unwrap_err();
    assert!(format!("{error:#}").contains("time step"));
}

#[test]
fn canonical_rate_converts_back_to_an_interval_amount() {
    let got =
        super::from_canonical_with_step("kg/m2/s", "mm", &[1.0 / 1800.0], Some(1800.0)).unwrap();
    assert_eq!(got, vec![1.0]);
}

#[test]
fn relative_humidity_and_vpd_become_specific_humidity() {
    let temperature = [293.15];
    let pressure = [100_000.0];
    let from_rh = super::humidity_to_specific("RH", "%", &[50.0], &temperature, &pressure)
        .unwrap()
        .unwrap();
    let saturation = 611.2_f64 * (17.67_f64 * 20.0 / (293.15_f64 - 29.65)).exp();
    let deficit_kpa = saturation / 2000.0;
    let from_vpd =
        super::humidity_to_specific("VPD", "kPa", &[deficit_kpa], &temperature, &pressure)
            .unwrap()
            .unwrap();
    assert!((from_rh[0] - from_vpd[0]).abs() < 1e-12);
    assert!(from_rh[0] > 0.0 && from_rh[0] < 0.02);
    let round_trip = super::humidity_from_specific("RH", "%", &from_rh, &temperature, &pressure)
        .unwrap()
        .unwrap();
    assert!((round_trip[0] - 50.0).abs() < 1e-10);
}

#[test]
fn humidity_names_accept_common_spacing_and_british_spelling() {
    let temperature = [293.15];
    let pressure = [100_000.0];
    assert!(super::humidity_to_specific(
        "relative humidity",
        "%",
        &[50.0],
        &temperature,
        &pressure,
    )
    .unwrap()
    .is_some());
    assert!(super::humidity_to_specific(
        "vapour-pressure-deficit",
        "kPa",
        &[1.0],
        &temperature,
        &pressure,
    )
    .unwrap()
    .is_some());
}
