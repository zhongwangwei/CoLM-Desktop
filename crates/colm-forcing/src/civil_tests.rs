use super::*;

#[test]
fn the_epoch_round_trips() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(civil_from_days(0), (1970, 1, 1));
}

#[test]
fn a_leap_day_is_a_real_day() {
    // 2008 是闰年；2100 不是（能被 100 整除而不能被 400 整除）
    assert_eq!(civil_from_days(days_from_civil(2008, 2, 29)), (2008, 2, 29));
    assert_eq!(
        days_from_civil(2008, 3, 1) - days_from_civil(2008, 2, 28),
        2
    );
    assert_eq!(
        days_from_civil(2100, 3, 1) - days_from_civil(2100, 2, 28),
        1
    );
}

#[test]
fn every_day_across_two_centuries_round_trips() {
    // 逐日全覆盖，不抽样。日历换算的错法几乎全在边界上（月末、闰年、世纪），
    // 抽样正好躲开它们。
    let from = days_from_civil(1900, 1, 1);
    let to = days_from_civil(2100, 1, 1);
    for d in from..to {
        let (y, m, day) = civil_from_days(d);
        assert_eq!(days_from_civil(y, m, day), d, "day {d} -> {y}-{m}-{day}");
        assert!((1..=12).contains(&m), "month {m} out of range at day {d}");
        assert!((1..=31).contains(&day), "day {day} out of range at day {d}");
    }
}

#[test]
fn a_stamp_advances_by_whole_seconds() {
    // 强迫场时间轴是「自起点的秒数」，所以这是本模块唯一被外部调用的入口。
    let start = Stamp {
        year: 2008,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    assert_eq!(start.plus_seconds(0), start);
    assert_eq!(
        start.plus_seconds(1800),
        Stamp {
            year: 2008,
            month: 1,
            day: 1,
            hour: 0,
            minute: 30,
            second: 0
        }
    );
    // 2008 是闰年：366 天 = 17568 个半小时步
    assert_eq!(
        start.plus_seconds(17568 * 1800),
        Stamp {
            year: 2009,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0
        }
    );
}

#[test]
fn crossing_a_year_boundary_lands_on_the_right_month() {
    // endyr/endmo 就是这么算出来的，错一天就会写错 namelist 的结束月
    let s = Stamp {
        year: 2008,
        month: 12,
        day: 31,
        hour: 23,
        minute: 30,
        second: 0,
    };
    let t = s.plus_seconds(1800);
    assert_eq!((t.year, t.month, t.day), (2009, 1, 1));
    assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
}
