use super::flooded_fraction;

#[test]
fn flooded_fraction_follows_the_profile_steps() {
    let prof = [0.1, 0.4, 0.9, 1.6];
    // 低于第一级：按第一级线性外推的平方根。
    assert_eq!(
        flooded_fraction(&prof, 0.05),
        ((0.05_f64 / 0.1) / 16.0).sqrt()
    );
    // 越过最后一级：全淹。
    assert_eq!(flooded_fraction(&prof, 2.0), 1.0);
    // 第 2、3 级之间（s = 2）。
    let w = 0.6_f64;
    let expect = ((((w - 0.4) / (0.9 - 0.4)) * 5.0) / 16.0 + 4.0 / 16.0).sqrt();
    assert_eq!(flooded_fraction(&prof, w), expect);
}
