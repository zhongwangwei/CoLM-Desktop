use super::*;

#[test]
fn the_epoch_offset_matches_the_real_history_file() {
    // CN-Cng 冬季窗口的 history 首点是 56802270 分（since 1900-1-1）。
    // 1900→2008 的偏移必须正好比它小 30 分 —— 那 30 分就是半区间回移。
    assert_eq!(minutes_from_1900(2008), 56_802_240);
    let first_label = 56_802_270.0;
    let sec = model_seconds(&[first_label], 2008);
    assert_eq!(sec[0], 1800.0, "首点应落在 2008-01-01 00:30");
}

#[test]
fn leap_years_follow_the_gregorian_rule() {
    // 1900 不是闰年（能被 100 整除且不能被 400 整除），2000 是。
    // 弄错任何一个，2008 的偏移就会差一整天 1440 分，配对全错位。
    assert!(!is_leap(1900));
    assert!(is_leap(2000));
    assert!(is_leap(2008));
    assert!(!is_leap(2100));
    // 1900..2008 共 108 年，其中 **26** 个闰年：1904、1908 … 2004。
    // 1900 不算（能被 100 整除、不能被 400 整除），2000 算。
    // 数成 27 会让偏移多出整整一天（56803680 而不是 56802240），
    // 而那正是本测试要防的错位。
    let leaps = (1900..2008).filter(|&y| is_leap(y)).count();
    assert_eq!(leaps, 26);
    assert_eq!(minutes_from_1900(2008), (108 * 365 + 26) * 24 * 60);
}

#[test]
fn an_hourly_label_covers_the_two_half_hours_before_and_at_it() {
    // 标签 00:30（1800 秒）覆盖 00:00–01:00，由观测的 00:00 与 00:30 两点组成。
    assert_eq!(observation_slots(1800.0), [0.0, 1800.0]);
    // 标签 01:30（5400 秒）覆盖 01:00–02:00。
    assert_eq!(observation_slots(5400.0), [3600.0, 5400.0]);
}
