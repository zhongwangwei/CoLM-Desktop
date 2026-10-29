use super::*;

#[test]
fn run_arguments_forward_plain_mpi_rank_count() {
    let args = run_args("/case", "/kernel", false, None, 8, None, None).expect("args");
    assert_eq!(
        args.windows(2).find(|pair| pair[0] == "--ranks"),
        Some(&["--ranks".into(), "8".into()][..])
    );
    assert!(!args
        .iter()
        .any(|arg| matches!(arg.as_str(), "master" | "io" | "worker")));
}

#[test]
fn batch_parallelism_accounts_for_ranks_per_case() {
    assert_eq!(batch_width(8, 12, 4), 3);
    assert_eq!(batch_width(8, 12, 1), 8);
}

#[test]
fn concurrent_rust_cases_split_the_cores_as_threads() {
    assert_eq!(threads_per_case(16, 4), 4);
    assert_eq!(threads_per_case(16, 3), 5);
    assert_eq!(threads_per_case(4, 8), 1);
    assert_eq!(threads_per_case(16, 1), 16);
    assert_eq!(threads_per_case(16, 0), 16);
}

#[test]
fn spatial_grid_dimensions_reject_partial_global_cells() {
    assert_eq!(grid_dimensions(0.5, 0.25).unwrap(), (720, 720));
    assert!(grid_dimensions(7.0, 0.5).is_err());
    assert!(grid_dimensions(0.0, 0.5).is_err());
    assert!(grid_dimensions(f64::MIN_POSITIVE, 0.5).is_err());
}

#[test]
fn existing_mesh_is_only_accepted_for_unstructured_cases() {
    assert_eq!(
        existing_unstructured_mesh("unstructured", Some("/data/PearlRiver.nc".into())).unwrap(),
        Some("/data/PearlRiver.nc".into())
    );
    assert!(existing_unstructured_mesh("unstructured", None).is_err());
    assert!(existing_unstructured_mesh("latlon", Some("/data/mesh.nc".into())).is_err());
}

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
fn sidecar_failures_are_persisted_in_the_study_log() {
    let root = std::env::temp_dir().join(format!(
        "colm-study-sidecar-log-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    append_study_event(
        root.to_str().unwrap(),
        &serde_json::json!({"kind":"study_failed","reason":"signal: 9"}),
    );
    let event: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(root.join("study.log"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(event["reason"], "signal: 9");
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
    assert_eq!(batch_width(0, 8, 1), 1);
    assert_eq!(batch_width(4, 8, 1), 4);
    assert_eq!(batch_width(99, 8, 1), 8);
    assert_eq!(batch_width(4, 0, 1), 1);
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
        run_args("/case", "/kernel", false, Some("mksrfdata"), 1, None, None).unwrap(),
        [
            "run",
            "/case",
            "--kernel",
            "/kernel",
            "--stream",
            "1",
            "--ranks",
            "1",
            "--stage",
            "mksrfdata",
        ]
    );
    assert_eq!(
        run_args("/case", "/kernel", true, None, 1, None, None).unwrap(),
        ["run", "/case", "--kernel", "/kernel", "--stream", "1", "--ranks", "1", "--force", "1",]
    );
    assert!(run_args("/case", "/kernel", false, Some("unknown"), 1, None, None).is_err());
}

#[test]
fn study_run_always_uses_streaming_ndjson() {
    let args = study_run_args(
        "/study".into(),
        "/kernel".into(),
        false,
        Some(4),
        Some(true),
        None,
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

    let args = study_run_args("/study".into(), "  ".into(), true, None, None, None);
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

#[test]
fn contextual_study_rows_keep_catalog_id_and_pft_index() {
    let descriptor = colm_case::parameters::all()
        .iter()
        .find(|descriptor| descriptor.id == "pft:DEF_PFT_VMAX25")
        .unwrap();
    let row = study_row(
        descriptor,
        "pft-type",
        crate::config::ParameterScopeInstance {
            kind: "pft-type".into(),
            scheme: None,
            index: Some(2),
            type_name: Some("needleleaf evergreen temperate tree".into()),
            process_file: None,
        },
        45.0,
    )
    .unwrap();
    assert_eq!(row.id, "pft:DEF_PFT_VMAX25");
    assert_eq!(row.scope_instance.index, Some(2));
    assert_eq!(row.default, 45.0);
    assert_eq!(row.min, Some(0.0));
}

#[test]
fn engine_is_forwarded_only_when_chosen_and_must_be_known() {
    let args = run_args("/case", "/kernel", false, None, 1, Some("fortran"), None).unwrap();
    assert!(args.ends_with(&["--engine".to_string(), "fortran".to_string()]));
    let args = run_args("/case", "/kernel", false, None, 1, None, None).unwrap();
    assert!(!args.iter().any(|arg| arg == "--engine"));
    assert!(run_args("/case", "/kernel", false, None, 1, Some("auto"), None).is_err());
    let args = study_run_args(
        "/s".into(),
        "/k".into(),
        true,
        None,
        None,
        Some("rust".into()),
    );
    assert!(args.ends_with(&["--engine".to_string(), "rust".to_string()]));
}

#[test]
fn stage_headings_name_the_step_in_plain_words() {
    assert_eq!(
        stage_heading("input-check"),
        "──── 检查实际输入（input-check）────"
    );
    assert_eq!(
        stage_heading("input-verify"),
        "──── 核验运行输入（input-verify）────"
    );
    assert_eq!(
        super::stage_heading("mksrfdata"),
        "──── 制作地表数据（mksrfdata）────"
    );
    assert_eq!(super::stage_heading("colm"), "──── 模拟（colm）────");
    assert_eq!(super::stage_heading("other"), "──── other ────");
}

#[test]
fn hybrid_commands_forward_case_kernel_and_install_options() {
    assert_eq!(
        hybrid_check_args("/case".into(), "/kernel".into(), None, None).unwrap(),
        ["hybrid-check", "/case", "--kernel", "/kernel"]
    );
    let revision = colm_model::VersionRef {
        id: "old-workspace".into(),
        revision: 1,
    };
    let selected = hybrid_check_args(
        "/case".into(),
        "/kernel".into(),
        Some(&revision),
        Some("fortran"),
    )
    .unwrap();
    assert_eq!(
        &selected[..4],
        ["model-hybrid-check", "/case", "--kernel", "/kernel"]
    );
    assert!(selected
        .windows(4)
        .any(|flags| flags == ["--model-id", "old-workspace", "--model-revision", "1"]));
    assert!(selected.ends_with(&["--preprocessors".into(), "fortran".into()]));
    assert!(hybrid_check_args(
        "/case".into(),
        "/kernel".into(),
        Some(&revision),
        Some("app")
    )
    .is_err());
    assert!(hybrid_check_args(
        "/case".into(),
        "/kernel".into(),
        Some(&colm_model::VersionRef {
            id: "old-workspace".into(),
            revision: 0
        }),
        None
    )
    .is_err());
    assert_eq!(
        hybrid_climate_args("/case".into(), "/kernel".into()),
        ["hybrid-climate", "/case", "--kernel", "/kernel"]
    );
    let args = hybrid_install_args(
        "/case".into(),
        "/m.mlp.json".into(),
        "pft".into(),
        &[" pftclass".into(), "".into(), "porsl[1] ".into()],
        &["DEF_PFT_VMAX25:10:80:sigmoid".into()],
        Some("/norm.json".into()),
        true,
    )
    .unwrap();
    assert_eq!(
        args,
        [
            "hybrid-install",
            "/case",
            "--model",
            "/m.mlp.json",
            "--slot",
            "pft",
            "--features",
            "pftclass,porsl[1]",
            "--output",
            "DEF_PFT_VMAX25:10:80:sigmoid",
            "--normalize",
            "/norm.json",
            "--force",
            "1"
        ]
    );
    let bare = hybrid_install_args(
        "/case".into(),
        "/m.onnx".into(),
        "land_class".into(),
        &["patchclass".into()],
        &["DEF_LC_VMAX25".into()],
        Some(" ".into()),
        false,
    )
    .unwrap();
    assert!(!bare.contains(&"--normalize".to_string()) && !bare.contains(&"--force".to_string()));
    assert!(hybrid_install_args(
        "/c".into(),
        "/m".into(),
        "pft".into(),
        &[" ".into()],
        &["X".into()],
        None,
        false
    )
    .is_err());
}

#[test]
fn hybrid_fit_arguments_follow_the_request() {
    let request = FitRequest {
        studies: vec!["/p/.colm/studies/s-1".into(), "/p/.colm/studies/s-2".into()],
        network: serde_json::json!({ "slot": "pft" }),
        kernel_dir: "/k".into(),
        out_dir: "/p/ai-models".into(),
        name: "vcmax-climate".into(),
        ridge: Some(0.1),
        epochs: None,
        learning_rate: None,
        weight: Some("pftfrac".into()),
    };
    let args = hybrid_fit_args(
        &request,
        "/p/ai-models/vcmax-climate.network.json",
        "/p/ai-models/vcmax-climate.mlp.json",
    )
    .unwrap();
    assert_eq!(
        args,
        [
            "hybrid-fit",
            "--studies",
            "/p/.colm/studies/s-1,/p/.colm/studies/s-2",
            "--network",
            "/p/ai-models/vcmax-climate.network.json",
            "--kernel",
            "/k",
            "--out",
            "/p/ai-models/vcmax-climate.mlp.json",
            "--ridge",
            "0.1",
            "--weight",
            "pftfrac",
        ]
    );
    // 不许没有 Study，模型名不许带路径。
    let empty = FitRequest {
        studies: vec![],
        ..request.clone()
    };
    assert!(hybrid_fit_args(&empty, "n", "m").is_err());
    let bad = FitRequest {
        name: "../x".into(),
        ..request
    };
    assert!(hybrid_fit_args(&bad, "n", "m").is_err());
}

#[test]
fn a_model_version_is_passed_only_when_it_is_not_the_official_one() {
    let args = run_args("/case", "/kernel", false, None, 1, None, Some("snow")).unwrap();
    assert!(args.windows(2).any(|w| w == ["--model-version", "snow"]));
    for version in [None, Some("official")] {
        let args = run_args("/case", "/kernel", false, None, 1, None, version).unwrap();
        assert!(!args.iter().any(|a| a == "--model-version"));
    }
    assert!(run_args("/case", "/kernel", false, None, 1, None, Some("../x")).is_err());
    assert!(run_args("/case", "/kernel", false, None, 1, None, Some("-x")).is_err());
}

#[test]
fn study_runs_only_name_a_modified_version() {
    assert!(validate_model_version(Some("snow")).is_ok());
    assert!(validate_model_version(Some("official")).is_ok());
    assert!(validate_model_version(Some("../x")).is_err());
}

#[test]
fn explicit_preprocessor_preserves_default_and_rejects_unknown_backend() {
    let mut args = vec!["run".to_string()];
    super::add_preprocessors(&mut args, None).unwrap();
    assert_eq!(args, ["run"]);
    super::add_preprocessors(&mut args, Some("fortran")).unwrap();
    assert_eq!(args, ["run", "--preprocessors", "fortran"]);
    let previous = args.clone();
    assert!(super::add_preprocessors(&mut args, Some("shell-command")).is_err());
    assert_eq!(args, previous);
}

#[test]
fn crop_detection_accepts_case_insensitive_frozen_parameter_keys() {
    assert!(super::is_crop_case(&[crate::config::FieldChange {
        path: "def_tuning_crop_planting_day".into(),
        value: "120".into(),
    }]));
}
fn verified_comparison_fixture(version: &str, observation_sha: &str) -> VerifiedStudySide {
    serde_json::from_value(serde_json::json!({
        "schema":1,
        "side":{"study":"/project/.colm/studies/a","version":version,"head":null,"status":"completed","best_member":"candidate","calibration":2.0,"validation":1.0,"baseline_calibration":3.0,"baseline_validation":2.0},
        "spec":{"kernel_dir":"/local/kernel","observations":{"site":"/local/obs.nc"},"base_cases":["site"],"parameters":["p"],"seed":1,"targets":["Qle"]},
        "input_identity":{"schema":1,"base_cases":{"site":{"schema":1,"algorithm":"sha256-full-v1","complete":true,"data_sha256":"11".repeat(32),"data_settings":{"period":"same"},"unresolved":[],"files":{"data":{"path":"/local/input.nc","sha256":"11".repeat(32),"category":"data","bytes":100},"model":{"path":"/local/model.nml","sha256":version,"category":"model","bytes":100}}}},"observations":{"site":{"path":"/local/obs.nc","sha256":observation_sha,"bytes":100,"category":"observation"}}},
        "verified":true,"notes":[]
    })).unwrap()
}

#[test]
fn study_comparison_allows_relocated_data_and_different_model_parameters() {
    let a = verified_comparison_fixture("version-a", &"aa".repeat(32));
    let mut b = verified_comparison_fixture("version-b", &"aa".repeat(32));
    b.spec["kernel_dir"] = serde_json::json!("/server/kernel");
    b.spec["observations"]["site"] = serde_json::json!("/server/obs.nc");
    b.input_identity.as_mut().unwrap()["base_cases"]["site"]["files"]["data"]["path"] =
        serde_json::json!("/server/input.nc");
    b.input_identity.as_mut().unwrap()["observations"]["site"]["path"] =
        serde_json::json!("/server/obs.nc");
    b.side.validation = Some(2.0);
    let result = compare_verified_study_sides(a, b).unwrap();
    assert_eq!(result["same_design"], true);
    assert_eq!(result["same_inputs"], true);
    assert_eq!(result["better"], "a");
    assert_eq!(result["period"], "validation");
}

#[test]
fn study_comparison_never_ranks_changed_or_unverified_evidence() {
    for change in [
        "observation",
        "data",
        "settings",
        "partial",
        "unverified",
        "design",
    ] {
        let a = verified_comparison_fixture("version-a", &"aa".repeat(32));
        let mut b = verified_comparison_fixture("version-b", &"aa".repeat(32));
        match change {
            "observation" => {
                b.input_identity.as_mut().unwrap()["observations"]["site"]["sha256"] =
                    serde_json::json!("bb".repeat(32))
            }
            "data" => {
                b.input_identity.as_mut().unwrap()["base_cases"]["site"]["data_sha256"] =
                    serde_json::json!("bb".repeat(32))
            }
            "settings" => {
                b.input_identity.as_mut().unwrap()["base_cases"]["site"]["data_settings"]
                    ["period"] = serde_json::json!("different")
            }
            "partial" => {
                b.input_identity.as_mut().unwrap()["base_cases"]["site"]["complete"] =
                    serde_json::json!(false)
            }
            "unverified" => b.verified = false,
            "design" => b.spec["seed"] = serde_json::json!(2),
            _ => unreachable!(),
        }
        b.side.validation = Some(2.0);
        let result = compare_verified_study_sides(a, b).unwrap();
        assert!(result["better"].is_null(), "{change}: {result}");
    }
}

#[test]
fn study_comparison_uses_only_finite_scores_and_labels_missing_validation() {
    let mut a = verified_comparison_fixture("version-a", &"aa".repeat(32));
    let mut b = verified_comparison_fixture("version-b", &"aa".repeat(32));
    a.side.validation = None;
    b.side.validation = None;
    a.side.calibration = Some(f64::INFINITY);
    let result = compare_verified_study_sides(a, b).unwrap();
    assert!(result["better"].is_null());
    assert_eq!(result["period"], "calibration");
    assert!(result["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("independent validation")));
}

#[cfg(unix)]
fn creation_test_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "colm-create-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[cfg(unix)]
#[test]
fn creation_timeout_reaps_writers_before_removing_fresh_case() {
    let dir = creation_test_dir("timeout");
    let creations = CaseCreations::default();
    let creation = CaseCreation::begin(
        &creations,
        dir.to_str().unwrap(),
        Duration::from_millis(150),
    )
    .unwrap();
    std::fs::create_dir(&dir).unwrap();
    let mut command = std::process::Command::new("sh");
    command.current_dir(&dir).args([
        "-c",
        "echo $$ > pid; (sleep 30; echo late > late-write) & wait",
    ]);
    let error = creation.capture_command(&mut command).unwrap_err();
    assert!(error.contains("超时"), "{error}");
    let pid: u32 = std::fs::read_to_string(dir.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!process_group_alive(pid));
    assert!(!dir.join("late-write").exists());
    drop(creation);
    assert!(!dir.exists());
    assert!(creations.0.inner.lock().unwrap().pending.is_empty());
}

#[cfg(unix)]
#[test]
fn creation_cancel_stops_only_its_own_process_and_blocks_later_stages() {
    let dir = creation_test_dir("cancel");
    let creations = CaseCreations::default();
    let runs = RunProcesses::default();
    runs.prepare(&["unrelated-run".into()]).unwrap();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(10)).unwrap();
    assert!(
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(10)).is_err()
    );
    std::fs::create_dir(&dir).unwrap();
    let worker = creation.clone();
    let handle = std::thread::spawn(move || {
        worker.capture_command(std::process::Command::new("sh").args(["-c", "sleep 30 & wait"]))
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while creations.0.running_pid(&creation.0.key).unwrap().is_none() {
        assert!(Instant::now() < deadline, "child did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        creations
            .0
            .cancel(Some(vec![creation.0.key.clone()]))
            .unwrap(),
        1
    );
    assert!(handle.join().unwrap().unwrap_err().contains("取消"));
    assert!(runs.inner.lock().unwrap().pending.contains("unrelated-run"));
    assert!(creation
        .capture_command(std::process::Command::new("sh").args(["-c", "exit 0"]))
        .unwrap_err()
        .contains("取消"));
    drop(creation);
    assert!(!dir.exists());
}

#[cfg(unix)]
#[test]
fn creation_completion_keeps_outputs_and_failure_preserves_existing_directory() {
    let dir = creation_test_dir("success");
    let creations = CaseCreations::default();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    std::fs::create_dir(&dir).unwrap();
    for _ in 0..2 {
        let output = creation
            .capture_command(
                std::process::Command::new("sh")
                    .args(["-c", "printf result; printf diagnostic >&2"]),
            )
            .unwrap();
        assert_eq!(output, "result");
    }
    creation.complete().unwrap();
    drop(creation);
    assert!(dir.exists());
    std::fs::write(dir.join("existing"), "keep").unwrap();
    let existing =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    assert!(existing
        .capture_command(std::process::Command::new("sh").args(["-c", "echo failure >&2; exit 1"]))
        .unwrap_err()
        .contains("failure"));
    drop(existing);
    assert_eq!(
        std::fs::read_to_string(dir.join("existing")).unwrap(),
        "keep"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn creation_cancel_between_steps_does_not_spawn_the_next_writer() {
    let dir = creation_test_dir("between");
    let creations = CaseCreations::default();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    std::fs::create_dir(&dir).unwrap();
    creation
        .capture_command(std::process::Command::new("sh").args(["-c", "exit 0"]))
        .unwrap();
    assert_eq!(
        creations
            .0
            .cancel(Some(vec![creation.0.key.clone()]))
            .unwrap(),
        1
    );
    let mut command = std::process::Command::new("sh");
    command
        .current_dir(&dir)
        .args(["-c", "echo unexpected > late-write"]);
    assert!(creation
        .capture_command(&mut command)
        .unwrap_err()
        .contains("取消"));
    assert!(!dir.join("late-write").exists());
    drop(creation);
    assert!(!dir.exists());
}

#[cfg(unix)]
#[test]
fn creation_drains_both_pipes_and_reaps_leftover_children() {
    let dir = creation_test_dir("pipes");
    let creations = CaseCreations::default();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    std::fs::create_dir(&dir).unwrap();
    let mut command = std::process::Command::new("sh");
    command.current_dir(&dir).args(["-c", "echo $$ > pid; i=0; while [ $i -lt 10000 ]; do printf abcdefgh; printf ijklmnop >&2; i=$((i + 1)); done; sleep 30 & exit 0"]);
    let stdout = creation.capture_command(&mut command).unwrap();
    assert_eq!(stdout.len(), 80000);
    let pid: u32 = std::fs::read_to_string(dir.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!process_group_alive(pid));
    drop(creation);
    assert!(!dir.exists());
}

#[cfg(unix)]
#[test]
fn creation_aliases_share_one_reservation_and_cancellation_key() {
    let root = creation_test_dir("aliases");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let link = root.join("alias");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    let dir = root.join("new");
    let creations = CaseCreations::default();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    for alias in [root.join("sub/../new"), link.join("new")] {
        assert_eq!(
            creation_key(alias.to_str().unwrap()).unwrap(),
            creation.0.key
        );
        assert!(
            CaseCreation::begin(&creations, alias.to_str().unwrap(), Duration::from_secs(5))
                .is_err()
        );
    }
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), "owned").unwrap();
    let worker = creation.clone();
    let handle = std::thread::spawn(move || {
        worker.capture_command(std::process::Command::new("sh").args(["-c", "sleep 30 & wait"]))
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while creations.0.running_pid(&creation.0.key).unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        creations
            .0
            .cancel(Some(vec![
                creation_key(link.join("new").to_str().unwrap()).unwrap()
            ]))
            .unwrap(),
        1
    );
    assert!(handle.join().unwrap().unwrap_err().contains("取消"));
    assert!(creation.finish(Err("cancelled".into())).is_err());
    drop(creation);
    assert!(!dir.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn creation_reports_cleanup_failure_and_keeps_unreaped_writer_registered() {
    let dir = creation_test_dir("cleanup-error");
    let creations = CaseCreations::default();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    // A file at the output path makes directory cleanup fail on every platform/user.
    std::fs::write(&dir, "keep").unwrap();
    let error = creation.finish(Err("cancelled".into())).unwrap_err();
    assert!(error.contains("cancelled") && error.contains("无法清理"));
    creation.0.writers_reaped.store(false, Ordering::SeqCst);
    let key = creation.0.key.clone();
    let error = creation
        .finish(Err("termination failed".into()))
        .unwrap_err();
    assert!(error.contains("创建进程未停止") && error.contains("禁止同目录重建"));
    drop(creation);
    assert!(creations.0.inner.lock().unwrap().pending.contains(&key));
    assert!(dir.is_file());
    std::fs::remove_file(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn creation_cancellation_before_pid_registration_keeps_the_writer_addressable() {
    let dir = creation_test_dir("spawn-race");
    let creations = CaseCreations::default();
    let creation =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    let mut child = colm_kernel::run::top_level_sidecar(&mut std::process::Command::new("sh"))
        .args(["-c", "sleep 30 & wait"])
        .spawn()
        .unwrap();
    assert_eq!(
        creations
            .0
            .cancel(Some(vec![creation.0.key.clone()]))
            .unwrap(),
        1
    );
    creation.remember_child(child.id()).unwrap();
    assert_eq!(
        creations.0.running_pid(&creation.0.key).unwrap(),
        Some(child.id())
    );
    assert!(creation.check().unwrap_err().contains("取消"));
    assert!(
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).is_err()
    );
    // A later cancellation can still target the child after the registration race.
    assert_eq!(
        creations
            .0
            .cancel(Some(vec![creation.0.key.clone()]))
            .unwrap(),
        1
    );
    child.wait().unwrap();
    assert!(!process_group_alive(child.id()));
}

#[cfg(unix)]
#[test]
fn creation_rechecks_destination_after_reserving_it() {
    let dir = creation_test_dir("completed-before-reservation");
    let creations = CaseCreations::default();
    // A second request can pass its initial check before the first request completes.
    assert!(!dir.exists());
    let first =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), "first case").unwrap();
    first.finish(Ok(String::new())).unwrap();
    drop(first);
    let second =
        CaseCreation::begin(&creations, dir.to_str().unwrap(), Duration::from_secs(5)).unwrap();
    assert!(second
        .validate_destination(false)
        .unwrap_err()
        .contains("已经有一个算例"));
    assert!(second
        .validate_destination(true)
        .unwrap_err()
        .contains("already exists"));
    drop(second);
    assert_eq!(
        std::fs::read_to_string(dir.join("case.nml")).unwrap(),
        "first case"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn redirect_output_dir_creates_symlink_to_target() {
    let temp = std::env::temp_dir().join(format!("colm-redirect-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let case_dir = temp.join("demo_case");
    let target_dir = temp.join("external_output");
    std::fs::create_dir_all(&case_dir).unwrap();
    std::fs::create_dir_all(case_dir.join("out")).unwrap();

    redirect_output_dir(
        case_dir.to_str().unwrap().into(),
        target_dir.to_str().unwrap().into(),
    )
    .unwrap();

    assert!(case_dir.join("out").is_symlink());
    std::fs::write(case_dir.join("out").join("test.txt"), "hello").unwrap();
    assert_eq!(
        std::fs::read_to_string(target_dir.join("test.txt")).unwrap(),
        "hello"
    );
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn startup_keeps_reservation_until_pid_registration() {
    let processes = RunProcesses::default();
    let case = "/tmp/starting".to_string();
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    assert!(!processes.take_cancelled(&case).unwrap());
    assert!(processes.prepare(std::slice::from_ref(&case)).is_err());
    assert_eq!(processes.cancel(Some(vec![case.clone()])).unwrap(), 1);
    assert!(!processes.remember(&case, u32::MAX).unwrap());
    assert!(processes.prepare(std::slice::from_ref(&case)).is_err());
    assert!(processes.forget(&case).unwrap());
    processes.prepare(std::slice::from_ref(&case)).unwrap();
}

#[test]
fn preparation_failure_releases_reservation_for_retry() {
    let processes = RunProcesses::default();
    let case = "/tmp/invalid-start".to_string();
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    assert!(!processes.take_cancelled(&case).unwrap());
    assert!(run_args(&case, "kernel", false, Some("invalid"), 1, None, None).is_err());
    processes.forget(&case).unwrap();
    assert_eq!(processes.cancel(Some(vec![case.clone()])).unwrap(), 0);
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    assert!(!processes.take_cancelled(&case).unwrap());
}

#[test]
fn a_case_with_broken_metadata_does_not_stay_running() {
    let dir = std::env::temp_dir().join(format!("colm-broken-meta-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".colm-desktop.json"), "{ not json").unwrap();
    let case = dir.to_string_lossy().into_owned();
    let processes = RunProcesses::default();
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    assert!(released_on_error(&processes, &case, || {
        crate::remote::server_case_args(&case, "").map(|_| ())
    })
    .is_err());
    processes.prepare(std::slice::from_ref(&case)).unwrap();
    processes
        .with_idle(|| Ok(()))
        .expect_err("the retry holds the reservation");
    processes.forget(&case).unwrap();
    processes.with_idle(|| Ok(())).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_budget_tracks_the_selected_stage_implementations() {
    let budget = batch_cpu_budget(None, Some("fortran"), Some("rust"), 4, Some(1));
    assert_eq!(batch_width(8, 8, budget), 2);
    assert_eq!(
        batch_cpu_budget(None, Some("fortran"), Some("rust"), 4, Some(6)),
        6
    );
    assert_eq!(
        batch_cpu_budget(None, Some("rust"), Some("fortran"), 4, Some(1)),
        4
    );
    assert_eq!(
        batch_cpu_budget(Some("colm"), Some("fortran"), Some("rust"), 4, Some(8)),
        4
    );
    assert_eq!(
        batch_cpu_budget(Some("mkinidata"), Some("fortran"), Some("rust"), 4, Some(2)),
        2
    );
    assert_eq!(
        batch_cpu_budget(Some("mksrfdata"), Some("rust"), Some("fortran"), 4, Some(8)),
        4
    );
    assert_eq!(batch_cpu_budget(None, None, None, 4, Some(2)), 2);
    assert_eq!(batch_cpu_budget(None, None, None, 4, None), 1);
}

#[test]
fn template_creation_preserves_generated_forcing_path() {
    let fields = vec![
        crate::config::FieldChange {
            path: " def_FORCING_namelist ".into(),
            value: "old.nml".into(),
        },
        crate::config::FieldChange {
            path: "DEF_USE_CROP".into(),
            value: ".true.".into(),
        },
    ];
    let mut generated = fields.clone();
    preserve_generated_forcing(Some("ERA5"), &mut generated);
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].path, "DEF_USE_CROP");
    let mut imported = fields.clone();
    preserve_generated_forcing(None, &mut imported);
    assert_eq!(imported.len(), 2);
}

#[test]
fn stderr_capture_preserves_nonempty_lines_for_failure_reason() {
    let mut emitted = Vec::new();
    let captured = read_stderr_lines(
        std::io::Cursor::new("checking inputs\n \nError: missing data\n"),
        |line| {
            emitted.push(line.to_string());
        },
    );
    assert_eq!(captured, ["checking inputs", "Error: missing data"]);
    assert_eq!(emitted, captured);
    assert_eq!(
        failure_reason(&captured).as_deref(),
        Some("checking inputs / Error: missing data")
    );
}

#[cfg(unix)]
#[test]
fn stderr_is_emitted_before_the_process_closes_its_pipe() {
    use std::io::Write;
    let (mut writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        read_stderr_lines(BufReader::new(reader), |line| {
            sender.send(line.to_string()).unwrap();
        })
    });
    writeln!(writer, "checking inputs").unwrap();
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        "checking inputs"
    );
    writeln!(writer, "Error: missing data").unwrap();
    drop(writer);
    assert_eq!(
        thread.join().unwrap(),
        ["checking inputs", "Error: missing data"]
    );
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        ["Error: missing data"]
    );
}

#[test]
fn input_fingerprints_are_opt_in_for_ipc_run_arguments() {
    for enabled in [None, Some(false), Some(true)] {
        let mut args = run_args("/case", "/kernel", false, None, 1, None, None).unwrap();
        add_input_fingerprints(&mut args, enabled);
        assert_eq!(
            args.iter().any(|arg| arg == "--record-input-fingerprints"),
            enabled == Some(true)
        );
        if enabled == Some(true) {
            assert!(args.ends_with(&["--record-input-fingerprints".into(), "1".into()]));
        }
    }
}

#[test]
fn log_lines_are_batched_in_order_and_never_held_back_while_quiet() {
    let (send, received) = std::sync::mpsc::channel();
    let collector = std::thread::spawn(move || {
        let mut out = Vec::new();
        batch_pump_events(received, Duration::from_millis(500), |event| {
            out.push(match event {
                PumpEvent::Lines(lines) => format!("lines:{}", lines.len()),
                PumpEvent::Stage(stage, state) => format!("stage:{stage}:{state}"),
                _ => "other".into(),
            })
        });
        out
    });
    // A burst of 3000 lines becomes a handful of events, not 3000.
    for index in 0..3000 {
        send.send(PumpEvent::Line(format!("line {index}"))).unwrap();
    }
    send.send(PumpEvent::Stage("colm".into(), "begin".into()))
        .unwrap();
    send.send(PumpEvent::Line("after stage".into())).unwrap();
    // The last line is delivered within the interval although nothing else arrives.
    std::thread::sleep(Duration::from_millis(1500));
    drop(send);
    let events = collector.join().unwrap();
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0], "lines:3000");
    assert_eq!(events[1], "stage:colm:begin");
    assert_eq!(events[2], "lines:1");
}

#[test]
fn a_quiet_program_still_gets_its_pending_line_flushed_by_the_timer() {
    let (send, received) = std::sync::mpsc::channel();
    let (seen, observed) = std::sync::mpsc::channel();
    let collector = std::thread::spawn(move || {
        batch_pump_events(received, Duration::from_millis(30), |event| {
            if let PumpEvent::Lines(lines) = event {
                seen.send(lines).unwrap();
            }
        })
    });
    send.send(PumpEvent::Line("preparing".into())).unwrap();
    // Sender still open: only the timer can deliver it.
    let lines = observed.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(lines, vec!["preparing".to_string()]);
    drop(send);
    collector.join().unwrap();
}

#[test]
fn study_run_history_records_the_studys_actual_end_state() {
    let done = |status: &str| {
        serde_json::json!({"type":"study_done","kind":"study_done","study":"s","status":status})
            .to_string()
    };
    assert_eq!(study_run_status(&done("completed")), "completed");
    assert_eq!(
        study_run_status(&done("completed_with_failures")),
        "completed_with_failures"
    );
    assert_eq!(study_run_status(&done("paused")), "paused");
    assert_eq!(study_run_status(&done("needs_review")), "needs_review");
    assert_eq!(
        study_run_status(r#"{"type":"study_done","kind":"study_done"}"#),
        "succeeded"
    );
    assert_eq!(study_run_status("not json"), "succeeded");
}

#[test]
fn model_tidy_waits_for_runs_and_creations_and_holds_both_off_while_it_works() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let runs = RunProcesses::default();
    let creations = CaseCreations::default();
    let tidy = |runs: &RunProcesses, creations: &CaseCreations| {
        crate::model::while_idle(runs, creations, || Ok("tidied"))
    };
    let case = "/tmp/tidy-run".to_string();
    runs.prepare(std::slice::from_ref(&case)).unwrap();
    let error = tidy(&runs, &creations).unwrap_err();
    assert!(error.contains("整理模型缓存"), "{error}");
    runs.forget(&case).unwrap();
    let created = "/tmp/tidy-new-case".to_string();
    creations.0.prepare(std::slice::from_ref(&created)).unwrap();
    assert!(
        tidy(&runs, &creations).is_err(),
        "a case being created blocks it"
    );
    creations.0.forget(&created).unwrap();
    assert_eq!(tidy(&runs, &creations).unwrap(), "tidied");
    // While it works, a run cannot register; it starts once the tidy returns.
    let registered = AtomicBool::new(false);
    std::thread::scope(|scope| {
        crate::model::while_idle(&runs, &creations, || {
            let waiter = scope.spawn(|| {
                runs.prepare(std::slice::from_ref(&case)).unwrap();
                registered.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(std::time::Duration::from_millis(100));
            assert!(!registered.load(Ordering::SeqCst), "run started mid-tidy");
            Ok(waiter)
        })
        .unwrap()
        .join()
        .unwrap();
    });
    assert!(registered.load(Ordering::SeqCst));
}

// 表是进程级静态的：两种归属放在同一条测试里，免得并行测试互相终止对方登记的子进程。
#[cfg(unix)]
#[test]
fn detached_children_stop_by_owner_and_all_stop_when_the_app_exits() {
    let sleeper = || {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30"]);
        colm_kernel::run::top_level_sidecar(&mut command);
        command.spawn().unwrap()
    };
    let mut remote = sleeper();
    let remote_guard = register_remote_child(remote.id(), Some("/case/a".into()));
    let mut preparation = sleeper();
    let preparation_guard = register_child(preparation.id(), ChildOwner::Preparation);
    // 别的算例的停止不碰它，数据准备的停止也不碰远程提交。
    assert_eq!(cancel_remote_submission("/case/b".into()).unwrap(), 0);
    assert_eq!(cancel_preparation().unwrap(), 1);
    assert!(
        !preparation.wait().unwrap().success(),
        "preparation was terminated"
    );
    assert!(
        preparation_guard.stopped(),
        "a stopped preparation reports why it failed"
    );
    assert!(!remote_guard.stopped());
    drop(preparation_guard);
    assert_eq!(
        cancel_detached_children().unwrap(),
        1,
        "exit cancels the remaining remote child"
    );
    assert!(
        !remote.wait().unwrap().success(),
        "the child was terminated"
    );
    drop(remote_guard);
    assert_eq!(
        cancel_detached_children().unwrap(),
        0,
        "the guard removes a finished child from the registry"
    );
}

// 只有这一条测试安装窗口那份登记表（进程级 OnceLock），其它测试用各自的 RunProcesses。
#[test]
fn config_writes_into_a_running_case_are_refused() {
    let root =
        std::env::temp_dir().join(format!("colm-running-case-writes-{}", std::process::id()));
    let running = root.join("running");
    let idle = root.join("idle");
    std::fs::create_dir_all(&running).unwrap();
    std::fs::create_dir_all(&idle).unwrap();
    let processes = RunProcesses::default();
    install_run_registry(&processes);
    let key = running.display().to_string();
    processes.prepare(std::slice::from_ref(&key)).unwrap();
    let error = refuse_writes_into_running_cases(&[&running.join("case.nml")]).unwrap_err();
    assert!(error.contains("算例正在运行"), "{error}");
    // 规范化后才比较：带 `..` 的写法也拦得住。
    let roundabout = idle.join("..").join("running").join("forcing.nml");
    assert!(refuse_writes_into_running_cases(&[&roundabout]).is_err());
    refuse_writes_into_running_cases(&[&idle.join("case.nml")]).unwrap();
    processes.forget(&key).unwrap();
    refuse_writes_into_running_cases(&[&running.join("case.nml")]).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn many_process_groups_stop_together_not_one_after_another() {
    let children: Vec<_> = (0..20)
        .map(|_| {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "sleep 30"]);
            colm_kernel::run::top_level_sidecar(&mut command);
            command.spawn().unwrap()
        })
        .collect();
    let started = std::time::Instant::now();
    let outcomes = terminate_all(
        children
            .iter()
            .map(|child| (child.id(), child.id()))
            .collect(),
    );
    // 串行是 20 × 约 300 ms；两批并行只需要两次等待。
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(outcomes.len(), 20);
    assert!(outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
    for mut child in children {
        assert!(!child.wait().unwrap().success());
    }
}

/// 一个仍在运行的子进程，自己是一个进程组（模拟 GUI 启动的外层 CLI）。
#[cfg(unix)]
fn grouped_sleeper() -> std::process::Child {
    // `&& wait` 让 sh 本身留在进程里（单条命令时 sh 会 exec 掉 sleep，进程名会变成
    // sleep，锁文件里记录的可执行文件对不上，反倒测不到补杀路径）。
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 30 & wait"]);
    colm_kernel::run::top_level_sidecar(&mut command);
    command.spawn().unwrap()
}

/// 已经退出、只留 PID 的子进程（模拟随进程组退出的内层 CLI）。
#[cfg(unix)]
fn exited_child_id() -> u32 {
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    assert!(child.wait().unwrap().success());
    pid
}

fn temporary_study_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_run_lock(dir: &std::path::Path, pid: u32, run_id: Option<&str>) {
    write_run_lock_for(dir, pid, run_id, "/bin/sh");
}

fn write_run_lock_for(dir: &std::path::Path, pid: u32, run_id: Option<&str>, executable: &str) {
    let mut value = serde_json::json!({
        "pid": pid,
        "executable": executable,
        "argv_sha256": "sha",
        "started_unix": 1,
        "heartbeat_unix": 1,
    });
    if let Some(run_id) = run_id {
        value["run_id"] = serde_json::Value::String(run_id.into());
    }
    std::fs::write(dir.join("run.lock"), serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn a_study_run_keeps_its_run_id_until_cancellation_finishes() {
    let processes = RunProcesses::default();
    let key = "study:/tmp/locked-study";
    processes.prepare_study_run(key, "run-1").unwrap();
    assert_eq!(processes.running_identity(key).unwrap(), None);
    // 取消可以在 spawn 与 PID 登记之间到达：那时只有运行 ID 能说明“这次运行还没启动”。
    processes.cancel(Some(vec![key.into()])).unwrap();
    assert!(!processes.remember(key, 4242).unwrap());
    let identity = processes.running_identity(key).unwrap();
    assert_eq!(
        identity, None,
        "a cancelled run never exposes a live identity"
    );
    processes.forget(key).unwrap();
    assert_eq!(processes.running_identity(key).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn a_study_run_reports_both_the_outer_pid_and_the_run_id() {
    let mut child = grouped_sleeper();
    let processes = RunProcesses::default();
    let key = "study:/tmp/locked-study";
    processes.prepare_study_run(key, "run-outer-inner").unwrap();
    assert!(processes.remember(key, child.id()).unwrap());
    let identity = processes.running_identity(key).unwrap().unwrap();
    assert_eq!(identity.pid, child.id());
    assert_eq!(identity.run_id.as_deref(), Some("run-outer-inner"));
    processes.forget(key).unwrap();
    assert_eq!(processes.running_identity(key).unwrap(), None);
    terminate_process_tree(child.id()).unwrap();
    let _ = child.wait();
}

#[test]
fn read_study_lock_keeps_only_a_usable_identity() {
    let dir = temporary_study_dir("study-lock-snapshot");
    let study = dir.to_string_lossy().to_string();
    assert_eq!(read_study_lock(&study), None);
    write_run_lock_for(&dir, 4242, Some("run-1"), "/bin/sh");
    assert_eq!(
        read_study_lock(&study),
        Some(StudyLockSnapshot {
            pid: 4242,
            executable: Some("/bin/sh".into()),
            run_id: Some("run-1".into()),
        })
    );
    // 旧格式锁没有 run_id：快照必须保留 None，收尾才能回落到 PID。
    write_run_lock(&dir, 4242, None);
    assert_eq!(
        read_study_lock(&study),
        Some(StudyLockSnapshot {
            pid: 4242,
            executable: Some("/bin/sh".into()),
            run_id: None,
        })
    );
    std::fs::write(dir.join("run.lock"), b"{broken").unwrap();
    assert_eq!(read_study_lock(&study), None);
    std::fs::write(dir.join("run.lock"), br#"{"pid":"4242"}"#).unwrap();
    assert_eq!(read_study_lock(&study), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lock_identity_rejects_another_runs_lock_before_any_termination() {
    let other = StudyLockSnapshot {
        pid: 10,
        executable: Some("/bin/sh".into()),
        run_id: Some("run-other".into()),
    };
    // 锁里带运行 ID 时它说了算：即使 PID 恰好等于本次登记的那个，也不是这次运行的锁。
    let error = lock_identity_matches(Some(&other), Some("run-ours"), Some(10)).unwrap_err();
    assert!(error.contains("run-other"), "{error}");
    assert!(error.contains("run-ours"), "{error}");
    // 冻结修订时外层 PID 与锁里的内层 PID 不同，但运行 ID 能认领。
    assert!(lock_identity_matches(Some(&other), Some("run-other"), Some(99)).is_ok());
    assert!(lock_identity_matches(None, Some("run-ours"), Some(99)).is_ok());
}

#[test]
fn a_bare_lock_is_only_claimed_by_its_own_pid() {
    let bare = StudyLockSnapshot {
        pid: 10,
        executable: Some("/bin/sh".into()),
        run_id: None,
    };
    // 旧锁没有运行 ID，只能按锁里的 PID 认。冻结修订时 GUI 登记的外层 PID 与它不同，
    // 调用方会把锁 owner 的 PID 传进来，而不是拿外层 PID 去否决。
    assert!(lock_identity_matches(Some(&bare), Some("run-ours"), Some(10)).is_ok());
    assert!(lock_identity_matches(Some(&bare), None, Some(10)).is_ok());
    // PID 对不上，或没有 PID：没有证据就不认领（不杀进程，让 CLI 拒绝并保留锁）。
    assert!(lock_identity_matches(Some(&bare), Some("run-ours"), Some(11)).is_err());
    assert!(lock_identity_matches(Some(&bare), Some("run-ours"), None).is_err());
}

#[test]
fn finalize_cancel_uses_the_lock_owner_pid_for_a_legacy_bare_lock() {
    let with_id = StudyLockSnapshot {
        pid: 4242,
        executable: Some("/bin/sh".into()),
        run_id: Some("run-1".into()),
    };
    // 锁里有运行 ID：外层 PID 只用于定位进程组，认领靠环境变量里的运行 ID。
    let (args, env) = finalize_cancel_invocation("/tmp/study", 99, Some("run-1"), Some(&with_id));
    assert_eq!(args[0], "study-finalize-cancel");
    assert_eq!(args[1], "/tmp/study");
    assert_eq!(&args[2..], ["--pid", "99"]);
    assert_eq!(
        env,
        vec![(STUDY_RUN_ID_ENV.to_string(), "run-1".to_string())]
    );

    // 旧锁没有运行 ID：必须把锁 owner 的 PID 交给 CLI 按 PID 认领；硬塞锁里不存在的
    // 运行 ID 会被 CLI 当成“调度器变了”拒绝，留下进程已停、锁还在。
    let bare = StudyLockSnapshot {
        pid: 4242,
        executable: Some("/bin/sh".into()),
        run_id: None,
    };
    let (args, env) = finalize_cancel_invocation("/tmp/study", 99, Some("run-1"), Some(&bare));
    assert_eq!(&args[2..], ["--pid", "4242"]);
    assert!(env.is_empty());

    // 没有锁（进程还没写锁，或已被收尾）：保持外层 PID + 运行 ID。
    let (args, env) = finalize_cancel_invocation("/tmp/study", 99, Some("run-1"), None);
    assert_eq!(&args[2..], ["--pid", "99"]);
    assert_eq!(
        env,
        vec![(STUDY_RUN_ID_ENV.to_string(), "run-1".to_string())]
    );

    // 手工/旧路径没有运行 ID 时不能凭空造一个。
    assert!(finalize_cancel_invocation("/tmp/study", 99, None, None)
        .1
        .is_empty());
}

#[cfg(unix)]
#[test]
fn cancelling_a_delegated_study_claims_the_lock_by_run_id_and_stops_the_tree() {
    let dir = temporary_study_dir("delegated-cancel");
    let study = dir.to_string_lossy().to_string();
    let processes = RunProcesses::default();
    let key = study_process_key(&study);
    let mut outer = grouped_sleeper();
    processes
        .prepare_study_run(&key, "run-outer-inner")
        .unwrap();
    assert!(processes.remember(&key, outer.id()).unwrap());
    // 锁里是内层 CLI 的 PID（已随进程组退出），和 GUI 登记的外层 PID 不同。
    write_run_lock(&dir, exited_child_id(), Some("run-outer-inner"));

    let calls = std::sync::Mutex::new(Vec::new());
    let out = cancel_study_run_with(&processes, &study, |args, env| {
        calls.lock().unwrap().push((args.to_vec(), env.to_vec()));
        Ok("cancelled".into())
    })
    .unwrap();
    assert_eq!(out, "cancelled");
    let calls = calls.into_inner().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0[0], "study-cancel");
    assert!(calls[0].1.is_empty());
    let (finalize, env) = &calls[1];
    assert_eq!(finalize[0], "study-finalize-cancel");
    assert_eq!(finalize[1], study);
    assert_eq!(&finalize[2..], ["--pid", &outer.id().to_string()]);
    assert_eq!(
        env,
        &vec![(STUDY_RUN_ID_ENV.to_string(), "run-outer-inner".into())]
    );
    // 进程组被终止：外层 CLI 与它启动的成员一起退出。
    assert!(!outer.wait().unwrap().success());
    processes.forget(&key).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn cancelling_kills_the_inner_scheduler_even_outside_the_terminated_group() {
    let dir = temporary_study_dir("escaped-inner");
    let study = dir.to_string_lossy().to_string();
    let processes = RunProcesses::default();
    let key = study_process_key(&study);
    let mut outer = grouped_sleeper();
    processes.prepare_study_run(&key, "run-1").unwrap();
    assert!(processes.remember(&key, outer.id()).unwrap());
    // 内层 CLI 理论上随外层进程组退出；万一它跑到别的进程组里，锁文件里的 PID 是唯一线索。
    let mut escaped = grouped_sleeper();
    let escaped_pid = escaped.id();
    // 锁里记的可执行文件必须与这个 PID 真正在跑的程序一致，补杀才允许动手。
    let image = process_image_name(escaped_pid).expect("the escaped process has a known image");
    write_run_lock_for(&dir, escaped_pid, Some("run-1"), &image);
    assert!(process_alive(escaped_pid));

    cancel_study_run_with(&processes, &study, |_, _| Ok("cancelled".into())).unwrap();
    assert!(
        !process_alive(escaped_pid),
        "the lock owner must be stopped"
    );
    assert!(!escaped.wait().unwrap().success());
    assert!(!outer.wait().unwrap().success());
    processes.forget(&key).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_reused_pid_with_another_program_is_not_killed_from_the_lock_file() {
    let dir = temporary_study_dir("reused-lock-pid");
    let study = dir.to_string_lossy().to_string();
    // 记录的调度器已经不在了，PID 被一个无关程序复用：锁里写的可执行文件对不上，
    // 补杀必须放弃（否则会误杀别的进程），让 CLI 收尾自己拒绝。
    let mut reused = grouped_sleeper();
    let reused_pid = reused.id();
    write_run_lock_for(&dir, reused_pid, Some("run-1"), "/usr/local/bin/colm-cli");
    let lock = read_study_lock(&study);
    terminate_study_lock_owner(lock.as_ref()).unwrap();
    assert!(process_alive(reused_pid), "a reused pid must not be killed");
    terminate_process_tree(reused_pid).unwrap();
    let _ = reused.wait();
    std::fs::remove_dir_all(dir).unwrap();
}

/// 同名可执行文件但锁里的运行 ID 是另一轮：必须在终止任何进程前拒绝。先组杀再比对会把
/// 本次登记的进程组和另一轮持锁的调度器都杀掉，这正是“迟到的取消误杀后来者”。
#[cfg(unix)]
#[test]
fn a_lock_from_another_run_is_rejected_before_any_process_is_stopped() {
    let dir = temporary_study_dir("other-run-lock");
    let study = dir.to_string_lossy().to_string();
    let processes = RunProcesses::default();
    let key = study_process_key(&study);
    let mut outer = grouped_sleeper();
    let mut other = grouped_sleeper();
    processes.prepare_study_run(&key, "run-ours").unwrap();
    assert!(processes.remember(&key, outer.id()).unwrap());
    let image = process_image_name(other.id()).expect("the other scheduler has a known image");
    write_run_lock_for(&dir, other.id(), Some("run-other"), &image);

    let calls = std::sync::Mutex::new(Vec::new());
    let error = cancel_study_run_with(&processes, &study, |args, env| {
        calls.lock().unwrap().push((args.to_vec(), env.to_vec()));
        Ok("cancelled".into())
    })
    .unwrap_err();
    assert!(error.contains("run-other"), "{error}");
    assert!(
        calls.lock().unwrap().is_empty(),
        "refusal must happen before cancel.request or finalize"
    );
    assert!(
        process_alive(outer.id()),
        "the registered tree must not be killed"
    );
    assert!(
        process_alive(other.id()),
        "another run's scheduler must not be killed"
    );

    terminate_process_tree(outer.id()).unwrap();
    let _ = outer.wait();
    terminate_process_tree(other.id()).unwrap();
    let _ = other.wait();
    processes.forget(&key).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

/// 旧锁没有运行 ID，只能靠可执行文件确认锁 owner 仍是那个调度器：一致就允许补杀，
/// 并用锁里的 PID（不是外层 PID）收尾，否则 CLI 会拒绝，进程停了锁还在。
#[cfg(unix)]
#[test]
fn a_legacy_bare_lock_with_a_matching_executable_is_still_stopped() {
    let dir = temporary_study_dir("legacy-bare-cancel");
    let study = dir.to_string_lossy().to_string();
    let processes = RunProcesses::default();
    let key = study_process_key(&study);
    let mut outer = grouped_sleeper();
    let mut legacy = grouped_sleeper();
    processes.prepare_study_run(&key, "run-ours").unwrap();
    assert!(processes.remember(&key, outer.id()).unwrap());
    let legacy_pid = legacy.id();
    let image = process_image_name(legacy_pid).expect("the legacy scheduler has a known image");
    write_run_lock_for(&dir, legacy_pid, None, &image);
    assert!(process_alive(legacy_pid));

    let calls = std::sync::Mutex::new(Vec::new());
    cancel_study_run_with(&processes, &study, |args, env| {
        calls.lock().unwrap().push((args.to_vec(), env.to_vec()));
        Ok("cancelled".into())
    })
    .unwrap();
    assert!(
        !process_alive(legacy_pid),
        "the legacy lock owner must be stopped"
    );
    assert!(!outer.wait().unwrap().success());
    let calls = calls.into_inner().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0[0], "study-finalize-cancel");
    assert_eq!(&calls[1].0[2..], ["--pid", &legacy_pid.to_string()]);
    assert!(
        calls[1].1.is_empty(),
        "a bare lock must not claim a run id it never wrote"
    );
    let _ = legacy.wait();
    processes.forget(&key).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn cancelling_without_a_registered_process_finalizes_idle_state_only() {
    let dir = temporary_study_dir("idle-cancel");
    let study = dir.to_string_lossy().to_string();
    let processes = RunProcesses::default();
    let calls = std::sync::Mutex::new(Vec::new());
    cancel_study_run_with(&processes, &study, |args, env| {
        calls.lock().unwrap().push((args.to_vec(), env.to_vec()));
        Ok(String::new())
    })
    .unwrap();
    let calls = calls.into_inner().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0[0], "study-cancel");
    assert_eq!(
        calls[1].0,
        vec!["study-finalize-idle-cancel".to_string(), study.clone()]
    );
    assert!(calls[1].1.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
