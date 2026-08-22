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

#[test]
fn it_tells_a_spinup_step_from_a_normal_one() {
    // CoLM.F90:749 与 :747 是两条不同的 format 语句。只认前者的话，
    // 预热行的整段尾巴会留在 `date` 里 —— 实测变成
    // "2008-01-01-00000 Spinup (cycle 1 of 3)"。不崩，但界面分不出正在预热，
    // 进度条还会跨轮次单调增长而看不出重来过。
    let p = parse_progress("TIMESTEP = 1 | DATE = 2008-01-01-00000").unwrap();
    assert_eq!(
        (p.step, p.date.as_str(), p.spinup),
        (1, "2008-01-01-00000", None)
    );

    let p = parse_progress("TIMESTEP = 7 | DATE = 2008-01-01-10800 Spinup (cycle 2 of 3)").unwrap();
    assert_eq!(p.step, 7);
    assert_eq!(p.date, "2008-01-01-10800", "日期不该混进轮次计数");
    assert_eq!(p.spinup, Some((2, 3)));
}

#[test]
fn the_stage_marker_is_ours_and_does_not_collide() {
    // 标记由 colm-cli 自己打。实测 CoLM 的 34180 行输出里没有一行以 `===`
    // 开头，也没有一处出现 `colm-stage` —— 所以不必去认 CoLM 的措辞，
    // 而那正是要避免的：上游把 automatically 拼成 automaticlly 已经教过一次。
    assert_eq!(
        parse_stage("=== colm-stage mksrfdata begin ==="),
        Some(("mksrfdata".into(), "begin".into()))
    );
    assert_eq!(
        parse_stage("=== colm-stage colm failed ==="),
        Some(("colm".into(), "failed".into()))
    );
    // CoLM 自己的输出一行都不该被认成标记
    for l in [
        "TIMESTEP = 1 | DATE = 2008-01-01-00000",
        " Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.",
        "=== something else ===",
    ] {
        assert_eq!(parse_stage(l), None, "{l:?} 不该被认成阶段标记");
    }
}

#[test]
fn every_run_event_says_which_case_it_came_from() {
    // 批量跑 90 个站点时，事件是全局广播的 —— 没有这个字段，前端收到的是
    // 一锅粥。加字段而不是改事件名：三个 listen 是 check-gui 守着的接口。
    let p = Progress {
        case: "/tmp/a".into(),
        stage: "colm".into(),
        step: 1,
        total_steps: 48,
        date: "2008-01-01-00000".into(),
        spinup: None,
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"case\":\"/tmp/a\""));
    assert!(json.contains("\"total_steps\":48"));
    let d = Done {
        case: "/tmp/a".into(),
        requested_stage: Some("mkinidata".into()),
        code: 0,
        total: 10,
        dropped: 3,
        reason: None,
    };
    let done_json = serde_json::to_string(&d).unwrap();
    assert!(done_json.contains("\"case\""));
    assert!(done_json.contains("\"requested_stage\":\"mkinidata\""));
    let l = Lines {
        case: "/tmp/a".into(),
        lines: vec!["x".into()],
    };
    let s = serde_json::to_string(&l).unwrap();
    assert!(s.contains("\"case\"") && s.contains("\"lines\""), "{s}");
    let m = StageMark {
        case: "/tmp/a".into(),
        stage: "colm".into(),
        state: "ok".into(),
    };
    assert!(serde_json::to_string(&m)
        .unwrap()
        .contains("\"stage\":\"colm\""));
}

#[test]
fn a_failed_run_says_why_not_just_the_exit_code() {
    // 退出码 1 对用户什么都没说。真正的原因在 `colm-cli` 的 stderr 上 ——
    // 而那条管道原来是 `piped()` 了却从来没人读，于是原因被丢掉，
    // 界面只剩「失败（退出码 1）」。实测踩过：真实原因是
    // `Forcing does not cover simulation period!`。
    let err = vec![
        "Error: 阶段 colm 失败".to_string(),
        "Caused by:".to_string(),
        "    Forcing does not cover simulation period!".to_string(),
    ];
    let why = failure_reason(&err).expect("有 stderr 就该有原因");
    assert!(why.contains("Forcing does not cover"), "{why}");

    // 空 stderr 不能编出一个原因来 —— 那比不说更误导。
    assert_eq!(failure_reason(&[]), None);
    assert_eq!(failure_reason(&["   ".to_string()]), None);
}

#[test]
fn requested_batch_cpu_count_is_bounded_by_the_machine() {
    assert_eq!(batch_width(0, 8), 1);
    assert_eq!(batch_width(4, 8), 4);
    assert_eq!(batch_width(99, 8), 8);
    assert_eq!(batch_width(4, 0), 1);
}

#[test]
fn batch_summary_distinguishes_success_from_attempted() {
    let summary = BatchSummary {
        total: 4,
        succeeded: 3,
        failed: 1,
    };
    let json = serde_json::to_string(&summary).unwrap();
    assert_eq!(json, r#"{"total":4,"succeeded":3,"failed":1}"#);
}

#[test]
fn a_requested_stage_is_forwarded_to_the_cli_without_changing_full_runs() {
    assert_eq!(
        run_args("/case", "/kernel", false, Some("mksrfdata")).unwrap(),
        [
            "run",
            "/case",
            "--kernel",
            "/kernel",
            "--stream",
            "1",
            "--stage",
            "mksrfdata",
        ]
    );
    assert_eq!(
        run_args("/case", "/kernel", true, None).unwrap(),
        ["run", "/case", "--kernel", "/kernel", "--stream", "1", "--force", "1",]
    );
    assert!(run_args("/case", "/kernel", false, Some("unknown")).is_err());
}
