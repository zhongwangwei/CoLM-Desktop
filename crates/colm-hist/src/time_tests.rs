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
fn a_midyear_observation_origin_is_not_reduced_to_january_first() {
    // AU-Preston 的观测不是从元旦开始，而是从 2003-08-12 03:30 开始。
    // 模型 04:00 的标签相对这个原点只能是 1800 秒；如果只解析年份，
    // 会错出 223 天，得到一批看似“模型特别差”的伪指标。
    let at_four_am = minutes_from_1900(2003) as f64
        + (31 + 28 + 31 + 30 + 31 + 30 + 31 + 11) as f64 * 1440.0
        + 4.0 * 60.0;
    let sec = model_seconds_from_units(&[at_four_am], "seconds since 2003-08-12T03:30:00")
        .expect("valid CF time units");
    assert_eq!(sec, vec![1800.0]);
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

#[test]
fn the_epoch_shift_lands_on_the_measured_first_record() {
    // CN-Cng 冬季窗口 history 的首点是 56802270 分（since 1900-1-1），
    // 也就是 2008-01-01 00:30 —— 标签在区间中点。
    // 换成 Unix 秒实测是 1199147400，对应 2008-01-01T00:30:00Z。
    assert_eq!(unix_seconds(&[56_802_270.0]), [1_199_147_400]);
}

#[test]
fn the_day_count_between_1900_and_the_epoch_is_right() {
    // 1900-01-01 到 1970-01-01 共 70 年，其中 17 个闰年
    // （1904…1968；1900 不算，能被 100 整除而不能被 400 整除）。
    // 数错一个就整体平移一天，曲线的日期标签全错。
    let leaps = (1900..1970).filter(|&y| is_leap(y)).count();
    assert_eq!(leaps, 17);
    assert_eq!(DAYS_1900_TO_EPOCH, -((70 * 365 + 17) as i64));
}

#[test]
fn an_hourly_series_stays_hourly_after_conversion() {
    // 换算是仿射的，步长必须原样保留 —— 差一点点就会让 uPlot 的
    // 等距假设失效，画出来的曲线在时间上是歪的。
    let m: Vec<f64> = (0..5).map(|i| 56_802_270.0 + (i as f64) * 60.0).collect();
    let u = unix_seconds(&m);
    for w in u.windows(2) {
        assert_eq!(w[1] - w[0], 3600);
    }
}

#[test]
fn time_units_parse_spaced_and_attached_zone_offsets() {
    let base = minutes_from_1900(2008) as f64;
    for units in [
        "seconds since 2008-01-01 00:00:00 +08:00",
        "seconds since 2008-01-01T00:00:00+08:00",
        // 无冒号写法在 CF 站点文件里也见过，不能解析失败。
        "seconds since 2008-01-01 00:00:00+0800",
    ] {
        let parsed = parse_time_units(units).expect("offset origin should parse");
        assert_eq!(parsed.civil_minutes, base, "{units}");
        assert_eq!(parsed.utc_offset_seconds, 8 * 3_600, "{units}");
        assert_eq!(parsed.unit_seconds, 1.0, "{units}");
        assert_eq!(parsed.calendar, Calendar::Standard);
    }

    // Z / UTC 是「零偏移」，和完全不写不同：前面那个 0 是**声明**，可用于确认基准。
    for units in [
        "seconds since 2008-01-01T00:00:00Z",
        "seconds since 2008-01-01 00:00:00 UTC",
    ] {
        let parsed = parse_time_units(units).expect("UTC origin should parse");
        assert_eq!(parsed.civil_minutes, base, "{units}");
        assert_eq!(parsed.utc_offset_seconds, 0, "{units}");
    }

    let days = parse_time_units("days since 2003-01-01").expect("day units");
    assert_eq!(days.civil_minutes, minutes_from_1900(2003) as f64);
    assert_eq!(days.unit_seconds, 86_400.0);
    let hours = parse_time_units("hours since 2003-01-01 06:30").expect("hour units");
    assert_eq!(
        hours.civil_minutes,
        minutes_from_1900(2003) as f64 + 6.0 * 60.0 + 30.0
    );
    assert_eq!(hours.unit_seconds, 3_600.0);
}

#[test]
fn named_zones_and_calendar_dependent_units_are_refused() {
    // 命名时区没有时区数据库可查，猜一个偏移比拒绝更危险。
    let error = parse_time_units("seconds since 2003-01-01 00:00:00 EST").unwrap_err();
    assert!(error.to_string().contains("named time zones"), "{error}");
    // 月/年长度随公历变化，不能折成固定秒数。
    for units in ["months since 2003-01-01", "years since 2003-01-01"] {
        let error = parse_time_units(units).unwrap_err();
        assert!(error.to_string().contains("calendar-dependent"), "{error}");
    }
    assert!(parse_time_units("seconds since 2008-02-30").is_err());
    assert!(parse_time_units("seconds since 2008-01-01 24:00:00").is_err());
    // 两个互相矛盾的时区声明不能被静默合并。
    let error = parse_time_units("seconds since 2008-01-01 00:00:00 UTC +08:00").unwrap_err();
    assert!(error.to_string().contains("conflicting"), "{error}");
}

#[test]
fn utc_model_with_offset_observation_shifts_by_the_declared_offset() {
    let base = minutes_from_1900(2008) as f64;
    // 模型标签 2008-01-01T00:30（真 UTC）；观测原点写的是 +08:00 的 00:00。
    let origin = parse_time_units("seconds since 2008-01-01 00:00:00 +08:00").unwrap();
    let seconds = model_seconds_from_basis(&[base + 30.0], &origin, TimeBasis::Utc).unwrap();
    // 旧实现会得到 1800 s（错 8 小时）；正确值是 1800 + 28800。
    assert_eq!(seconds, vec![30_600.0]);
}

#[test]
fn local_standard_time_with_the_same_offset_does_not_shift_twice() {
    let base = minutes_from_1900(2008) as f64;
    let origin = parse_time_units("seconds since 2008-01-01 00:00:00 +08:00").unwrap();
    let seconds = model_seconds_from_basis(
        &[base + 30.0],
        &origin,
        TimeBasis::LocalStandardTime {
            utc_offset_seconds: 8 * 3_600,
        },
    )
    .unwrap();
    // 模型与观测都是同一地方标准时：两边的偏移相消，仍是 00:30。
    assert_eq!(seconds, vec![1_800.0]);
}

#[test]
fn unknown_model_clock_refuses_a_declared_observation_offset() {
    let base = minutes_from_1900(2008) as f64;
    let origin = parse_time_units("seconds since 2008-01-01 00:00:00 +08:00").unwrap();
    let error = model_seconds_from_basis(&[base], &origin, TimeBasis::ModelClock).unwrap_err();
    assert!(
        error.to_string().contains("no recorded UTC offset"),
        "{error}"
    );
    // 非秒单位同样不能进模型钟路径：旧实现只支持秒，静默换算是另一类错。
    let days = parse_time_units("days since 2008-01-01").unwrap();
    let error = model_seconds_from_basis(&[base], &days, TimeBasis::ModelClock).unwrap_err();
    assert!(error.to_string().contains("seconds since"), "{error}");
}

#[test]
fn non_second_units_expose_the_factor_and_still_return_seconds() {
    let base = minutes_from_1900(2008) as f64;
    // 因子由 ParsedTimeUnits 交给调用方：调用方把 obs raw 值乘成秒再配对。
    assert_eq!(
        parse_time_units("days since 2008-01-01")
            .unwrap()
            .unit_seconds,
        86_400.0
    );
    assert_eq!(
        parse_time_units("hours since 2008-01-01")
            .unwrap()
            .unit_seconds,
        3_600.0
    );
    assert_eq!(
        parse_time_units("minutes since 2008-01-01")
            .unwrap()
            .unit_seconds,
        60.0
    );
    // model_seconds_from_basis 只返回秒（不除单位因子），单位换算留给调用方，
    // 否则配对层会对天/小时单位的观测缩放两次。
    let origin = parse_time_units("days since 2008-01-01 00:00:00").unwrap();
    let seconds =
        model_seconds_from_basis(&[base, base + 1_440.0], &origin, TimeBasis::Utc).unwrap();
    assert_eq!(seconds, vec![0.0, 86_400.0]);
}

#[test]
fn the_legacy_wrapper_keeps_the_old_conversion_and_rejects_offsets() {
    let base = minutes_from_1900(2008) as f64;
    let seconds =
        model_seconds_from_units(&[base + 30.0], "seconds since 2008-01-01 00:00:00").unwrap();
    assert_eq!(seconds, vec![1_800.0]);
    // 旧实现静默丢掉 +08:00；现在宁可返回 None 让调用方报「不支持」，
    // 也不能拿一个错 8 小时的轴去算指标。
    assert!(
        model_seconds_from_units(&[base], "seconds since 2008-01-01 00:00:00 +08:00").is_none()
    );
}
