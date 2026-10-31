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

fn parse_arguments(args: &[&str]) -> Result<Arguments> {
    Arguments::parse(args.iter().map(|arg| (*arg).to_owned()))
}

#[test]
fn spmd_flags_are_parsed_and_must_come_together() {
    let arguments = parse_arguments(&[
        "case",
        "--land-cover",
        "igbp",
        "--case-outputs",
        "--spmd-check",
        "--rank",
        "2",
        "--ranks",
        "4",
    ])
    .unwrap();
    assert!(arguments.spmd_check);
    assert_eq!(arguments.rank, Some(2));
    assert_eq!(arguments.ranks, Some(4));

    let error = parse_arguments(&[
        "case",
        "--land-cover",
        "igbp",
        "--case-outputs",
        "--rank",
        "1",
    ])
    .err()
    .expect("--rank without --ranks must be rejected");
    assert!(format!("{error:#}").contains("--rank and --ranks"));

    let error = parse_arguments(&[
        "case",
        "--land-cover",
        "igbp",
        "--case-outputs",
        "--rank",
        "4",
        "--ranks",
        "4",
    ])
    .err()
    .expect("an out-of-range rank must be rejected");
    assert!(format!("{error:#}").contains("outside 0..4"));
}

#[test]
fn explicit_ranks_are_recognized_and_size_one_stays_single_process() {
    let arguments = parse_arguments(&[
        "case",
        "--land-cover",
        "igbp",
        "--case-outputs",
        "--rank",
        "1",
        "--ranks",
        "2",
    ])
    .unwrap();
    let info = requested_rank_info(&arguments)
        .unwrap()
        .expect("two ranks must be recognized");
    assert_eq!((info.rank(), info.size()), (1, 2));

    let arguments = parse_arguments(&[
        "case",
        "--land-cover",
        "igbp",
        "--case-outputs",
        "--rank",
        "0",
        "--ranks",
        "1",
    ])
    .unwrap();
    assert!(requested_rank_info(&arguments).unwrap().is_none());
}

#[test]
fn spmd_rendezvous_path_is_unique_per_slurm_job_and_step() {
    let env = |name: &str| match name {
        "SLURM_JOB_ID" => Some("job123".to_owned()),
        "SLURM_STEP_ID" => Some("456".to_owned()),
        _ => None,
    };
    assert_eq!(
        spmd_rendezvous_path(std::path::Path::new("/out"), env),
        std::path::PathBuf::from("/out/.colm-spmd/endpoint-jobjob123-step456")
    );
    let plain = spmd_rendezvous_path(std::path::Path::new("/out"), |_| None);
    assert_eq!(plain, std::path::PathBuf::from("/out/.colm-spmd/endpoint"));
    let custom = spmd_rendezvous_path(std::path::Path::new("/out"), |name| {
        (name == "COLM_SPMD_RENDEZVOUS").then(|| "/shared/endpoint".to_owned())
    });
    assert_eq!(custom, std::path::PathBuf::from("/shared/endpoint"));
}

/// 实测过的上游行为：`DEF_simulation_time%end_sec = 0` 时，Fortran 的
/// `2005-01-01 00:00 → 2005-01-02 00:00` 窗口跑 48 个 30 分钟步，最后一步的
/// `end_time` 在 `RuntimeClock` 里记成「1 月 1 日 86400 秒」。逐个 step 的
/// `end_time` 必须用同一种表示去比较，否则窗口跑完也认为没跑过任何一步。
#[test]
fn a_midnight_window_end_is_compared_in_the_clock_representation() {
    use colm_core::{CalendarTime, RuntimeClock};

    let start = CalendarTime {
        year: 2005,
        julian_day: 1,
        seconds: 0,
    };
    let end = CalendarTime {
        year: 2005,
        julian_day: 2,
        seconds: 0,
    };
    let clock_end = clock_style_end(end);
    assert_eq!(
        clock_end,
        CalendarTime {
            year: 2005,
            julian_day: 1,
            seconds: 86_400
        }
    );

    let mut clock = RuntimeClock::new(start, end, start, 1800.0, 0).unwrap();
    let mut last = None;
    while let Some(step) = clock.next_step() {
        last = Some(step.end_time);
    }
    assert_eq!(
        last,
        Some(clock_end),
        "the window must end at the clock's own representation"
    );
}

/// 年初的 `sec = 0` 终点要落到上一年的最后一天 86400 秒（闰年 366）。
#[test]
fn a_new_year_midnight_end_falls_back_to_the_previous_year() {
    let end = CalendarTime {
        year: 2004,
        julian_day: 1,
        seconds: 0,
    };
    assert_eq!(
        clock_style_end(end),
        CalendarTime {
            year: 2003,
            julian_day: 365,
            seconds: 86_400
        }
    );
    let leap_end = CalendarTime {
        year: 2005,
        julian_day: 1,
        seconds: 0,
    };
    assert_eq!(
        clock_style_end(leap_end),
        CalendarTime {
            year: 2004,
            julian_day: 366,
            seconds: 86_400
        }
    );
}
