use super::*;

#[test]
fn development_prefers_the_workspace_cli_over_a_stale_staged_sidecar() {
    let sibling = std::path::PathBuf::from("/app");
    let candidates = cli_candidates(Some(sibling.clone()));
    let staged = sibling.join(exe_name());
    let staged_position = candidates.iter().position(|path| path == &staged).unwrap();
    let workspace_position = candidates
        .iter()
        .position(|path| path.ends_with(format!("target/debug/{}", exe_name())))
        .unwrap();
    if cfg!(debug_assertions) {
        assert!(workspace_position < staged_position);
    } else {
        assert!(staged_position < workspace_position);
    }
}

#[test]
fn development_prefers_repository_kernels_over_stale_staged_resources() {
    let resource = std::path::PathBuf::from("/staged");
    let roots = kernel_roots(Some(resource.clone()));
    let staged = resource.join("kernels");
    let staged_position = roots.iter().position(|root| root == &staged).unwrap();
    let repository_position = roots
        .iter()
        .position(|root| root.ends_with("../../kernels"))
        .unwrap();
    if cfg!(debug_assertions) {
        assert!(repository_position < staged_position);
    } else {
        assert!(staged_position < repository_position);
    }
}

#[test]
fn crop_wizard_fields_enable_crop_site_auditing() {
    let fields = [crate::config::FieldChange {
        path: "DEF_TUNING_CROP_PLANTING_DAY".into(),
        value: "120".into(),
    }];
    assert!(is_crop_case(&fields));
    assert!(!is_crop_case(&[]));
}

#[test]
fn bgc_case_requires_its_runtime_inputs() {
    let mut fields = vec![crate::config::FieldChange {
        path: "DEF_USE_BGC".into(),
        value: ".true.".into(),
    }];
    assert!(validate_bgc_runtime(None, &fields)
        .unwrap_err()
        .contains("运行时数据目录"));

    let root = std::env::temp_dir().join(format!("colm-bgc-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ndep = root
        .join("ndep")
        .join("fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
    std::fs::create_dir_all(ndep.parent().unwrap()).unwrap();
    std::fs::write(&ndep, []).unwrap();
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("硝化数据"));

    fields.push(crate::config::FieldChange {
        path: "DEF_USE_NITRIF".into(),
        value: ".false.".into(),
    });
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());
    fields.pop();
    for family in ["CONC_O2_UNSAT", "O2_DECOMP_DEPTH_UNSAT"] {
        let dir = root.join("nitrif").join(family);
        std::fs::create_dir_all(&dir).unwrap();
        for layer in 1..=10 {
            std::fs::write(dir.join(format!("{family}_l{layer:02}.nc")), []).unwrap();
        }
    }
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());

    fields.push(crate::config::FieldChange {
        path: "DEF_NDEP_FREQUENCY".into(),
        value: "2".into(),
    });
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("fndep_colm_monthly.nc"));
    std::fs::write(root.join("ndep/fndep_colm_monthly.nc"), []).unwrap();
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());
    fields.pop();

    fields.push(crate::config::FieldChange {
        path: "DEF_USE_FIRE".into(),
        value: ".true.".into(),
    });
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("abm_colm_double_fillcoast.nc"));
    for name in [
        "fire/abm_colm_double_fillcoast.nc",
        "fire/peatf_colm_360x720_c100428.nc",
        "fire/gdp_colm_360x720_c100428.nc",
        "fire/colmforc.Li_2017_HYDEv3.2_CMIP6_hdm_0.5x0.5_AVHRR_simyr1850-2016_c180202.nc",
        "fire/clmforc.Li_2012_climo1995-2011.T62.lnfm_Total_c140423.nc",
    ] {
        let file = root.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, []).unwrap();
    }
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());
    fields.pop();

    fields.push(crate::config::FieldChange {
        path: "DEF_TUNING_CROP_PLANTING_DAY".into(),
        value: "120".into(),
    });
    assert!(
        validate_bgc_runtime(root.to_str(), &fields).is_ok(),
        "colm-cli new supplies the disabled CROP-management defaults"
    );
    fields.pop();

    fields.extend([
        crate::config::FieldChange {
            path: "DEF_TUNING_CROP_PLANTING_DAY".into(),
            value: "0".into(),
        },
        crate::config::FieldChange {
            path: "DEF_USE_FERT".into(),
            value: ".true.".into(),
        },
        crate::config::FieldChange {
            path: "DEF_FERT_SOURCE".into(),
            value: "1".into(),
        },
    ]);
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("播种日期运行时目录缺少数据"));
    std::fs::create_dir_all(root.join("crop")).unwrap();
    std::fs::write(root.join("crop/plantdt-colm-64cfts-rice2_fillcoast.nc"), []).unwrap();
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("fertnitro_fillcoast.nc"));
    std::fs::write(root.join("crop/fertnitro_fillcoast.nc"), []).unwrap();
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());
    fields.truncate(1);

    fields.extend([
        crate::config::FieldChange {
            path: "DEF_TUNING_CROP_PLANTING_DAY".into(),
            value: "120".into(),
        },
        crate::config::FieldChange {
            path: "DEF_USE_FERT".into(),
            value: ".false.".into(),
        },
        crate::config::FieldChange {
            path: "DEF_USE_IRRIGATION".into(),
            value: ".true.".into(),
        },
        crate::config::FieldChange {
            path: "DEF_IRRIGATION_ALLOCATION".into(),
            value: "1".into(),
        },
    ]);
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("surfdata_irrigation_method_96x144.nc"));
    std::fs::write(root.join("crop/surfdata_irrigation_method_96x144.nc"), []).unwrap();
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());

    fields.last_mut().unwrap().value = "3".into();
    assert!(validate_bgc_runtime(root.to_str(), &fields)
        .unwrap_err()
        .contains("surfdata_irrigation_allocation.nc"));
    std::fs::write(root.join("crop/surfdata_irrigation_allocation.nc"), []).unwrap();
    assert!(validate_bgc_runtime(root.to_str(), &fields).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

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
        run_id: "run-1".into(),
        case: "/tmp/a".into(),
        stage: "colm".into(),
        step: 1,
        total_steps: 48,
        date: "2008-01-01-00000".into(),
        spinup: None,
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"case\":\"/tmp/a\""));
    assert!(json.contains("\"run_id\":\"run-1\""));
    assert!(json.contains("\"total_steps\":48"));
    let d = Done {
        run_id: "run-1".into(),
        case: "/tmp/a".into(),
        requested_stage: Some("mkinidata".into()),
        code: 0,
        total: 10,
        dropped: 3,
        reason: None,
        cancelled: false,
    };
    let done_json = serde_json::to_string(&d).unwrap();
    assert!(done_json.contains("\"case\""));
    assert!(done_json.contains("\"requested_stage\":\"mkinidata\""));
    assert!(done_json.contains("\"cancelled\":false"));
    let l = Lines {
        run_id: "run-1".into(),
        case: "/tmp/a".into(),
        lines: vec!["x".into()],
    };
    let s = serde_json::to_string(&l).unwrap();
    assert!(s.contains("\"case\"") && s.contains("\"lines\""), "{s}");
    let m = StageMark {
        run_id: "run-1".into(),
        case: "/tmp/a".into(),
        stage: "colm".into(),
        state: "ok".into(),
    };
    assert!(serde_json::to_string(&m)
        .unwrap()
        .contains("\"stage\":\"colm\""));
}

#[test]
fn cancellation_covers_a_batch_case_before_it_spawns() {
    let processes = RunProcesses::default();
    let case = "/tmp/queued".to_string();
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    assert_eq!(processes.cancel(Some(vec![case.clone()])).unwrap(), 1);
    assert!(processes.take_cancelled(&case).unwrap());
    assert!(!processes.take_cancelled(&case).unwrap());
}

#[test]
fn one_batch_cannot_schedule_the_same_case_twice() {
    let processes = RunProcesses::default();
    let case = "/tmp/duplicate".to_string();
    let error = processes.prepare(&[case.clone(), case]).unwrap_err();
    assert!(error.contains("duplicate case"), "{error}");
}

#[test]
fn cancellation_wins_the_spawn_registration_race() {
    let processes = RunProcesses::default();
    let case = "/tmp/racing".to_string();
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    processes.cancel(Some(vec![case.clone()])).unwrap();
    assert!(!processes.remember(&case, u32::MAX).unwrap());
    assert!(processes.forget(&case).unwrap());
}

#[test]
fn a_study_cancel_can_identify_the_scheduler_it_killed() {
    let processes = RunProcesses::default();
    let key = study_process_key("/tmp/study");
    processes.prepare(std::slice::from_ref(&key)).unwrap();
    assert!(processes.remember(&key, 42).unwrap());
    assert_eq!(processes.running_pid(&key).unwrap(), Some(42));
    processes.forget(&key).unwrap();
    assert_eq!(processes.running_pid(&key).unwrap(), None);
}

#[test]
fn study_process_keys_normalize_equivalent_paths() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "colm-study-process-key-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let plain = study_process_key(root.to_str().unwrap());
    let slash = study_process_key(&format!("{}/", root.display()));
    assert_eq!(plain, slash);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelling_a_finished_case_does_not_poison_its_next_run() {
    let processes = RunProcesses::default();
    let case = "/tmp/finished".to_string();
    assert_eq!(processes.cancel(Some(vec![case.clone()])).unwrap(), 0);
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    assert!(!processes.take_cancelled(&case).unwrap());
}

#[cfg(unix)]
#[test]
fn cancellation_terminates_the_sidecar_and_its_child_process_group() {
    let processes = RunProcesses::default();
    let key = "/tmp/process-tree".to_string();
    processes.prepare(std::slice::from_ref(&key)).unwrap();
    let mut child = colm_kernel::run::top_level_sidecar(&mut std::process::Command::new("sh"))
        .args(["-c", "sleep 30 & wait"])
        .spawn()
        .unwrap();
    let pid = child.id();
    assert!(processes.remember(&key, pid).unwrap());
    assert_eq!(processes.cancel(Some(vec![key.clone()])).unwrap(), 1);
    child.wait().unwrap();
    assert!(
        !process_group_alive(pid),
        "process group {pid} survived cancellation"
    );
    assert!(processes.forget(&key).unwrap());
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
fn evaluation_plan_forwards_case_obs_and_kernel() {
    assert_eq!(
        evaluation_plan_args("/case".into(), "/obs.nc".into(), "/kernel".into()),
        [
            "evaluation-plan",
            "/case",
            "--obs",
            "/obs.nc",
            "--kernel",
            "/kernel",
        ]
    );
}

#[test]
fn selected_evaluation_variables_are_forwarded_individually() {
    let args = metrics_args(
        "/case".into(),
        "/obs.nc".into(),
        0,
        false,
        true,
        Some(vec!["Rnet".into(), "GPP".into(), "NEE".into()]),
        None,
        None,
        None,
    );
    let pairs = args
        .windows(2)
        .filter(|window| window[0] == "--pairs-var")
        .map(|window| window[1].as_str())
        .collect::<Vec<_>>();
    assert_eq!(pairs, ["Rnet", "GPP", "NEE"]);
}

#[test]
fn evaluation_date_window_is_forwarded_as_half_open_unix_bounds() {
    let args = metrics_args(
        "/case".into(),
        "/obs.nc".into(),
        0,
        false,
        true,
        None,
        None,
        Some(1_600_000_000),
        Some(1_700_000_000),
    );
    assert!(args.windows(2).any(|pair| pair == ["--from", "1600000000"]));
    assert!(args.windows(2).any(|pair| pair == ["--to", "1700000000"]));
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

#[test]
fn study_run_always_uses_streaming_ndjson() {
    let args = study_run_args(
        "/study".into(),
        "/kernel".into(),
        false,
        Some(4),
        Some(true),
    );
    assert_eq!(
        args,
        [
            "study-run",
            "/study",
            "--kernel",
            "/kernel",
            "--stream",
            "1",
            "--jobs",
            "4",
            "--retry-failed",
            "1",
        ]
    );

    let args = study_run_args("/study".into(), "  ".into(), true, None, None);
    assert_eq!(args, ["study-run", "/study", "--stream", "1"]);
}

#[test]
fn study_apply_preview_forwards_member_without_output_path() {
    assert_eq!(
        study_apply_preview_args("/study".into(), "best".into()),
        ["study-apply-preview", "/study", "--member", "best"]
    );
    assert_eq!(
        study_apply_args("/study".into(), "m000001".into(), "/out".into()),
        [
            "study-apply",
            "/study",
            "--member",
            "m000001",
            "--out",
            "/out"
        ]
    );
}

#[test]
fn study_events_are_strict_json_lines() {
    let event = parse_study_event_line(r#"{"type":"task_done","member":"m000001"}"#).unwrap();
    assert_eq!(event["type"], "task_done");
    assert_eq!(event["member"], "m000001");
    assert!(parse_study_event_line("not json").is_none());
    assert!(parse_study_event_line("   ").is_none());
}

#[test]
fn only_raw_study_task_logs_are_suppressed_from_the_gui() {
    let log = serde_json::json!({"kind":"task_log","line":"TIMESTEP = 1"});
    let done = serde_json::json!({"kind":"task_done","member":"m000001"});
    assert!(is_study_task_log(&log));
    assert!(!is_study_task_log(&done));
}

#[test]
fn study_task_logs_are_forwarded_at_a_bounded_rate() {
    let start = Instant::now();
    let mut last = None;
    assert!(should_forward_study_task_log(&mut last, start));
    assert!(!should_forward_study_task_log(
        &mut last,
        start + EMIT_INTERVAL / 2
    ));
    assert!(should_forward_study_task_log(
        &mut last,
        start + EMIT_INTERVAL
    ));
}

#[test]
fn shutdown_marks_pending_runs_as_cancelled() {
    let processes = RunProcesses::default();
    let key = "/tmp/shutdown-pending".to_string();
    processes.prepare(std::slice::from_ref(&key)).unwrap();
    assert_eq!(processes.cancel_on_shutdown().unwrap(), 1);
    assert!(processes.take_cancelled(&key).unwrap());
}

#[test]
fn study_spec_temp_names_are_unique_and_json_suffixed() {
    let a = write_temp_study_spec("{}").unwrap();
    let b = write_temp_study_spec("{}").unwrap();
    assert_ne!(a, b);
    assert_eq!(a.extension().and_then(|s| s.to_str()), Some("json"));
    assert!(a
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("colm-study-"));
    std::fs::remove_file(a).unwrap();
    std::fs::remove_file(b).unwrap();
}
