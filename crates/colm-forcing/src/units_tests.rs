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
