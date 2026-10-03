//! colm-rs 驱动里的纯函数。

use super::*;

fn day(year: i32, julian_day: u16) -> CalendarTime {
    CalendarTime {
        year,
        julian_day,
        seconds: 0,
    }
}

/// 2000 年以后每个年末都换土地覆盖；以前只在换入 1990、1995、2000 时换。
#[test]
fn lulcc_year_ends_follow_the_next_year() {
    let end = |start| {
        let boundary = lulcc_year_end(start);
        (boundary.year, boundary.julian_day, boundary.seconds)
    };
    assert_eq!(end(day(2005, 100)), (2005, 365, 86_400));
    assert_eq!(end(day(2003, 1)), (2003, 365, 86_400));
    assert_eq!(end(day(1995, 364)), (1999, 365, 86_400));
    assert_eq!(end(day(1990, 1)), (1994, 365, 86_400));
    assert_eq!(end(day(1985, 1)), (1989, 365, 86_400));
    assert_eq!(end(day(1996, 1)), (1999, 365, 86_400));
    // 闰年的年末是第 366 天。
    assert_eq!(end(day(2008, 1)), (2008, 366, 86_400));
}
