use super::*;
use crate::civil::Stamp;

fn ok_met() -> MetSummary {
    MetSummary {
        time_units: "seconds since 2008-01-01 00:00:00".into(),
        start: Stamp {
            year: 2008,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        steps: 35041,
        step_seconds: 1800.0,
        step_uniform: true,
        height_v: 6.0,
        height_t: 6.0,
        height_q: 6.0,
        variables: [
            "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect(),
        // PLUMBER2 的文件没有这个属性 —— 它的地方时是隐含约定
        time_shown_in: None,
    }
}

#[test]
fn a_healthy_file_has_no_problems() {
    assert!(check(&ok_met(), None).is_empty());
}

#[test]
fn a_units_string_colm_cannot_parse_is_reported() {
    // CoLM 在硬编码字符偏移处解析（MOD_Forcing.F90:1253）。实测 hours since、
    // days since、以及不补零的 seconds since 都让 read 返回 iostat 5010，
    // 而 CoLM 没有 iostat，于是以 Fortran 运行期错误终止。
    // 报错是响亮的，但一句人话好过一个崩溃栈。
    for bad in [
        "hours since 2008-01-01 00:00:00",
        "days since 2008-01-01 00:00:00",
        "seconds since 2008-1-1 0:0:0",
    ] {
        let mut m = ok_met();
        m.time_units = bad.into();
        let p = check(&m, None);
        assert!(p.iter().any(|x| x.contains("time units")), "{bad}: {p:?}");
    }
}

#[test]
fn a_missing_variable_is_reported_by_name() {
    let mut m = ok_met();
    m.variables.retain(|v| v != "LWdown");
    let p = check(&m, None);
    assert!(p.iter().any(|x| x.contains("LWdown")), "{p:?}");
}

#[test]
fn an_uneven_time_step_is_reported() {
    // CoLM 按固定步长在时间轴上取样；步长不均匀会让它取到错误的时刻，
    // 而不会报错。
    let mut m = ok_met();
    m.step_uniform = false;
    assert!(check(&m, None).iter().any(|x| x.contains("uniform")));
}

#[test]
fn a_window_past_the_end_of_the_forcing_is_reported() {
    // 这是本 crate 存在的头号理由。CoLM 自己的注释是
    // "when reaching the END of forcing data, show a Warning but still try to run"
    // （MOD_Forcing.F90:1107），而 colm-kernel 的失败标记里没有 Warning:。
    // 那样的运行会被判成功，产出一份完整而错误的 history。
    let m = ok_met(); // 覆盖 2008-01-01 起 35041 个半小时步，约到 2009-12-31
    let window = (
        Stamp {
            year: 2009,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Stamp {
            year: 2011,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
    );
    let p = check(&m, Some(window));
    assert!(p.iter().any(|x| x.contains("beyond the forcing")), "{p:?}");
}

#[test]
fn a_window_before_the_start_is_reported_too() {
    let m = ok_met();
    let window = (
        Stamp {
            year: 2007,
            month: 6,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Stamp {
            year: 2008,
            month: 6,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
    );
    assert!(check(&m, Some(window))
        .iter()
        .any(|x| x.contains("before the forcing")));
}

#[test]
fn a_window_inside_the_coverage_is_fine() {
    let m = ok_met();
    let window = (
        Stamp {
            year: 2008,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Stamp {
            year: 2008,
            month: 1,
            day: 11,
            hour: 0,
            minute: 0,
            second: 0,
        },
    );
    assert!(check(&m, Some(window)).is_empty());
}

#[test]
fn an_hourly_file_is_not_a_problem_by_itself() {
    // 实测 90 个站点里有 2 个是 3600 s。那本身没问题 —— 但算例里的
    // DEF_simulation_time%timestep 必须跟着改，所以校验要说出步长。
    let mut m = ok_met();
    m.step_seconds = 3600.0;
    assert!(check(&m, None).is_empty());
    assert_eq!(m.timestep_hint(), 3600);
}

#[test]
fn only_a_file_that_says_utc_is_treated_as_greenwich() {
    // Urban-PLUMBER 的强迫场写 `:time_shown_in = "UTC"`（还带
    // `local_utc_offset_hours = 10.`）；PLUMBER2 的 90 个文件**一个都没有**
    // 这个属性，而它们确实是地方时。所以判据是「有且是 UTC」。
    // 搞反会把整个模拟平移一个时区 —— 而 design.md §2.8 已经量过，
    // 时区错 8 小时能把 Rnet 的 R² 从 0.986 打到 0.146。
    let mut m = ok_met();
    assert!(!m.is_greenwich(), "no attribute means local time");

    m.time_shown_in = Some("UTC".into());
    assert!(m.is_greenwich());
    m.time_shown_in = Some("  utc ".into());
    assert!(m.is_greenwich(), "trimmed and case-insensitive");

    m.time_shown_in = Some("local".into());
    assert!(!m.is_greenwich());
    m.time_shown_in = Some("local standard time".into());
    assert!(!m.is_greenwich());
}
