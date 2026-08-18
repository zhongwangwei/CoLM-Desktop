use super::*;

#[test]
fn a_progress_line_is_parsed_into_step_and_date() {
    // 实测格式：`TIMESTEP = 1 | DATE = 2008-01-01-00000`，
    // 末段是 YYYY-MM-DD-SSSSS（当天秒数）。日期原样传，不在这里解释。
    let p = parse_progress(" TIMESTEP = 1 | DATE = 2008-01-01-00000").expect("parses");
    assert_eq!(p.step, 1);
    assert_eq!(p.date, "2008-01-01-00000");
    let p = parse_progress("TIMESTEP = 528 | DATE = 2008-01-11-84600").expect("parses");
    assert_eq!(p.step, 528);
}

#[test]
fn an_ordinary_line_is_not_progress() {
    assert!(parse_progress(" CoLM Execution Completed.").is_none());
    assert!(parse_progress(" Note: something changed").is_none());
    assert!(parse_progress("").is_none());
}

#[test]
fn rangecheck_chatter_is_dropped_but_its_alarms_are_not() {
    // 85% 的日志是这种行。丢掉它们是安全的**只因为**越界时同一行尾部会
    // 追加 ` with NAN` / ` Out of Range!`，而那两句是 colm-kernel 的失败标记。
    // 若把带告警的那种也丢了，一次带 NaN 的运行在界面上就完全看不出来。
    let quiet = " Check vector data:    lakedepth    [m]      is in (    0.1000000000E+00,    0.1000000000E+00)";
    assert!(is_rangecheck_noise(quiet));

    let nan =
        " Check vector data:    t_soisno     [K]      is in (    0.1E+00,    0.1E+00) with NAN";
    let oor = " Check vector data:    wliq_soisno  [kg/m2]  is in (   -0.1E+03,    0.1E+03) Out of Range!";
    assert!(!is_rangecheck_noise(nan), "an alarm must reach the log");
    assert!(!is_rangecheck_noise(oor), "an alarm must reach the log");
}

#[test]
fn other_lines_survive_the_filter() {
    for l in [
        " CoLM Execution Completed.",
        " Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.",
        " Warning: energy balance violation    1.23      10",
        "  mksrfdata  ok",
    ] {
        assert!(!is_rangecheck_noise(l), "{l}");
        assert!(parse_progress(l).is_none(), "{l}");
    }
}

#[test]
fn the_executable_name_follows_the_platform() {
    let n = exe_name();
    if cfg!(windows) {
        assert!(n.ends_with(".exe"));
    } else {
        assert!(!n.contains('.'));
    }
}

#[test]
fn a_blank_line_carries_nothing_and_is_dropped() {
    // 实测：丢掉 RangeCheck 之后剩的 5330 行里 2644 行是空行 —— 一半的
    // 环形缓冲区容量会被它们占掉，而它们什么也不说明。
    // 这不是按文本猜价值，是「空的就是空的」。
    for l in ["", "   ", "\t"] {
        assert!(l.trim().is_empty());
        assert!(!is_rangecheck_noise(l));
        assert!(parse_progress(l).is_none());
    }
}
