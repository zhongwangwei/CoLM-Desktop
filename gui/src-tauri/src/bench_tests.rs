use super::*;

/// 状态 JSON 里的路径由原生目录 + 记录里的 POSIX 相对路径拼成，Windows 上两种分隔符会混在
/// 一行；断言按分隔符归一后比较，而不是比较字符串原文。
fn same_path(actual: &str, expected: &std::path::Path) -> bool {
    actual.replace('\\', "/") == expected.to_string_lossy().replace('\\', "/")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-bench-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn comparison_names_stay_inside_the_projects_folder() {
    valid_project("AT-Neu_vs_draft2").unwrap();
    for bad in ["", "../x", "a/b", "a b", "名字"] {
        assert!(valid_project(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_standalone_report_is_preferred_and_projects_list_newest_first() {
    let root = temp("projects");
    let older = root.join("projects/older");
    let newer = root.join("projects/newer");
    for dir in [&older, &newer] {
        std::fs::create_dir_all(dir.join("output/run/reports")).unwrap();
    }
    std::fs::write(older.join("openbench.yaml"), "a").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(newer.join("openbench.yaml"), "b").unwrap();
    std::fs::write(older.join("output/run/reports/evaluation_report.html"), "x").unwrap();
    std::fs::write(newer.join("output/run/reports/evaluation_report.html"), "x").unwrap();
    std::fs::write(
        newer.join("output/run/reports/evaluation_report_standalone.html"),
        "x",
    )
    .unwrap();
    let list = projects(&root);
    assert_eq!(list[0]["name"], "newer");
    assert!(list[0]["report"]
        .as_str()
        .unwrap()
        .ends_with("evaluation_report_standalone.html"));
    assert!(list[1]["report"]
        .as_str()
        .unwrap()
        .ends_with("evaluation_report.html"));
    assert!(projects(&root.join("missing")).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reports_are_read_only_from_inside_the_projects_folder() {
    let root = temp("inside");
    let projects = root.join("projects");
    std::fs::create_dir_all(projects.join("a/output")).unwrap();
    let report = projects.join("a/output/r.html");
    std::fs::write(&report, "<p>ok</p>").unwrap();
    let outside = root.join("secret.html");
    std::fs::write(&outside, "no").unwrap();
    assert!(inside(&projects, &report));
    assert!(!inside(&projects, &outside));
    // `..` 规范化后出了 projects 就不认；不存在的路径也不认。
    assert!(!inside(&projects, &projects.join("a/../../secret.html")));
    assert!(!inside(&projects, &projects.join("missing.html")));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, projects.join("link.html")).unwrap();
        assert!(!inside(&projects, &projects.join("link.html")));
    }
    assert!(read_report(&projects.join("a/output/r.txt")).is_err());
    assert_eq!(read_report(&report).unwrap(), "<p>ok</p>");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_variable_registration_accepts_only_safe_names_and_colon_free_fields() {
    assert!(valid_model("CoLM2024"));
    for bad in ["", "../CoLM", "a b", "a:b", "名字"] {
        assert!(!valid_model(bad), "{bad:?}");
    }
    // 变量、变量名、单位经 ':' 拼成 OpenBench 的 -v 规格；带冒号会把字段拆错，命令层直接拒绝。
    for value in ["Latent_Heat", "f_lfevpa", "W m-2"] {
        assert!(valid_mapping_field(value), "{value:?}");
    }
    for bad in ["", "  ", "f:lfevpa", "W\nm-2"] {
        assert!(!valid_mapping_field(bad), "{bad:?}");
    }
}

fn bench_site(id: &str, station: &str, lon: Option<f64>, lat: Option<f64>) -> BenchSite {
    BenchSite {
        id: id.to_owned(),
        station: station.to_owned(),
        history_dir: format!("/data/{station}"),
        prefix: format!("{station}_hist_"),
        lon,
        lat,
    }
}

#[test]
fn site_coordinates_are_validated_as_a_finite_pair() {
    assert!(valid_coordinates(None, None).is_ok());
    assert!(valid_coordinates(Some(11.31), Some(47.12)).is_ok());
    // 0–360 的经度写法与 ±180 都接受。
    assert!(valid_coordinates(Some(359.9), Some(-89.9)).is_ok());
    assert!(valid_coordinates(Some(-180.0), Some(90.0)).is_ok());
    for (lon, lat) in [(Some(1.0), None), (None, Some(1.0))] {
        let error = valid_coordinates(lon, lat).unwrap_err();
        assert!(error.contains("成对"), "{error}");
    }
    for (lon, lat) in [
        (Some(f64::NAN), Some(1.0)),
        (Some(1.0), Some(f64::INFINITY)),
        (Some(361.0), Some(1.0)),
        (Some(1.0), Some(-90.1)),
    ] {
        assert!(valid_coordinates(lon, lat).is_err(), "{lon:?},{lat:?}");
    }
    // 成对入口的错误要说清是哪个站点出的问题。
    let bad = [bench_site("a", "AT-Neu", Some(999.0), Some(0.0))];
    let error = validate_sites(&bad).unwrap_err();
    assert!(error.contains("站点 a"), "{error}");
}

#[test]
fn sites_without_coordinates_still_deserialize() {
    // 旧前端（或格点模拟）不带 lon/lat：可选字段，不能因此拒绝整次运行。
    let site: BenchSite = serde_json::from_value(json!({
        "id": "AT_Neu",
        "station": "AT-Neu",
        "history_dir": "/data/AT-Neu/history",
        "prefix": "AT-Neu_hist_",
    }))
    .unwrap();
    assert_eq!(site.lon, None);
    assert_eq!(site.lat, None);
    let with: BenchSite = serde_json::from_value(json!({
        "id": "AT_Neu",
        "station": "AT-Neu",
        "history_dir": "/data/AT-Neu/history",
        "prefix": "AT-Neu_hist_",
        "lon": 11.31,
        "lat": 47.12,
    }))
    .unwrap();
    assert_eq!(with.lon, Some(11.31));
    assert_eq!(with.lat, Some(47.12));
}

#[test]
fn remote_site_payload_carries_the_confirmed_coordinates() {
    let sites = [
        bench_site("a", "AT-Neu", Some(11.31), Some(47.12)),
        bench_site("b", "CH-Lae", None, None),
    ];
    let payload = crate::bench_remote::sites_payload(&sites);
    assert_eq!(payload[0]["id"], "a");
    assert_eq!(payload[0]["station"], "AT-Neu");
    assert_eq!(payload[0]["history_dir"], "/data/AT-Neu");
    assert_eq!(payload[0]["prefix"], "AT-Neu_hist_");
    assert_eq!(payload[0]["lon"], 11.31);
    assert_eq!(payload[0]["lat"], 47.12);
    // 没有坐标就明确是 null，远程侧按"没给"处理。
    assert_eq!(payload[1]["lon"], Value::Null);
    assert_eq!(payload[1]["lat"], Value::Null);
}

#[test]
fn new_style_projects_bind_the_report_to_the_last_success_not_the_last_run() {
    let root = temp("status-runs");
    let projects_dir = root.join("projects");
    // 旧目录（没有 runs/）：报告仍在项目根的 output/ 下，兼容发现。
    let legacy = projects_dir.join("legacy");
    std::fs::create_dir_all(legacy.join("output/old/reports")).unwrap();
    std::fs::write(legacy.join("openbench.yaml"), "legacy").unwrap();
    let legacy_report = legacy.join("output/old/reports/evaluation_report.html");
    std::fs::write(&legacy_report, "old").unwrap();

    // 新目录：项目根还留着旧版本 output/，但有 runs/ 之后就不能再被当成当前报告。
    let project = projects_dir.join("fresh");
    let stale = project.join("output/stale/reports/evaluation_report_standalone.html");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, "<p>stale</p>").unwrap();
    let mut first =
        crate::bench_runs::RunSnapshot::create(&project, "fresh", "cfg: first\n", &[], None)
            .unwrap();
    // 第一次成功之前：有 runs/ 但没有 latest-success，报告必须是 null（不能回退 stale）。
    let running = projects(&root);
    let row = running.iter().find(|row| row["name"] == "fresh").unwrap();
    assert_eq!(row["report"], Value::Null);
    assert_eq!(row["latest_run"]["state"], "running");
    assert_eq!(row["latest_success"], Value::Null);

    let reports = first.dir().join("output/fresh/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report_standalone.html");
    std::fs::write(&report, "<p>fresh</p>").unwrap();
    first.succeed(first.discover_report().as_deref()).unwrap();

    // 第二次运行失败：latest_run 是失败的 B，报告仍是 A 的。
    let mut second =
        crate::bench_runs::RunSnapshot::create(&project, "fresh", "cfg: second\n", &[], None)
            .unwrap();
    second.fail("check 失败（exit status: 1）：\nboom").unwrap();

    let list = projects(&root);
    assert_eq!(list.len(), 2);
    let fresh = list.iter().find(|row| row["name"] == "fresh").unwrap();
    assert!(same_path(fresh["report"].as_str().unwrap(), &report));
    assert_eq!(fresh["latest_run"]["run_id"], second.id());
    assert_eq!(fresh["latest_run"]["state"], "failed");
    assert_eq!(fresh["latest_run"]["report"], Value::Null);
    assert!(fresh["latest_run"]["error"]
        .as_str()
        .unwrap()
        .contains("check 失败"));
    assert_eq!(fresh["latest_success"]["run_id"], first.id());
    assert!(same_path(
        fresh["latest_success"]["report"].as_str().unwrap(),
        &report
    ));
    // 配置显示的是最新一次运行的冻结文本（B 提交的那份），不是项目根的历史配置。
    assert_eq!(
        fresh["config"].as_str().unwrap(),
        second.config_file().to_string_lossy()
    );

    let legacy_row = list.iter().find(|row| row["name"] == "legacy").unwrap();
    assert!(same_path(
        legacy_row["report"].as_str().unwrap(),
        &legacy_report
    ));
    assert_eq!(legacy_row["latest_run"], Value::Null);
    assert_eq!(legacy_row["latest_success"], Value::Null);
    assert_eq!(
        legacy_row["config"].as_str().unwrap(),
        legacy.join("openbench.yaml").to_string_lossy()
    );

    assert!(crate::bench_runs::has_runs(&project));
    assert!(!crate::bench_runs::has_runs(&legacy));
    let _ = std::fs::remove_dir_all(&root);
}

/// R03：状态页的报告必须来自 run 记录里记的输出目录，不能假设项目根或运行根下的 `output/`。
#[test]
fn the_status_list_reads_the_output_dir_recorded_in_the_run() {
    let root = temp("status-output-dir");
    let project = root.join("projects/custom");
    let config = "project:\n  name: custom\n  output_dir: ./results\n";
    let mut run =
        crate::bench_runs::RunSnapshot::create(&project, "custom", config, &[], None).unwrap();
    let reports = run.dir().join("results/custom/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report_standalone.html");
    std::fs::write(&report, "<p>custom</p>").unwrap();
    run.succeed(run.discover_report().as_deref()).unwrap();

    let row = projects(&root)
        .into_iter()
        .find(|row| row["name"] == "custom")
        .unwrap();
    assert!(same_path(row["report"].as_str().unwrap(), &report));
    assert_eq!(row["latest_run"]["state"], "succeeded");
    assert!(same_path(
        row["latest_success"]["report"].as_str().unwrap(),
        &report
    ));
    let _ = std::fs::remove_dir_all(&root);
}

/// 远程取回的本机镜像没有 `runs/`：根目录的 `meta.json` 就是那次运行的记录，
/// 报告按它记的相对路径发现，不要再猜 `output/`（R03）。
#[test]
fn a_flat_fetched_mirror_finds_its_report_through_its_run_record() {
    let root = temp("status-flat-mirror");
    let mirror = root.join("projects/cmp@host");
    let reports = mirror.join("custom/mirror/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report.html");
    std::fs::write(&report, "<p>mirror</p>").unwrap();
    std::fs::write(
        mirror.join("meta.json"),
        json!({
            "run_id": "bench-run-1-2",
            "project": "cmp",
            "created_at": "2023-11-14T22:13:20Z",
            "created_unix": 1_700_000_000u64,
            "finished_at": "2023-11-14T22:13:21Z",
            "finished_unix": 1_700_000_001u64,
            "state": "succeeded",
            "config_sha": "abc",
            "output_dir": "custom",
            "report": "custom/mirror/reports/evaluation_report.html",
        })
        .to_string(),
    )
    .unwrap();

    let row = projects(&root)
        .into_iter()
        .find(|row| row["name"] == "cmp@host")
        .unwrap();
    assert!(same_path(row["report"].as_str().unwrap(), &report));
    assert_eq!(row["latest_run"]["run_id"], "bench-run-1-2");
    assert_eq!(row["latest_success"]["run_id"], "bench-run-1-2");
    assert!(same_path(
        row["latest_success"]["report"].as_str().unwrap(),
        &report
    ));
    let _ = std::fs::remove_dir_all(&root);
}

/// R04：真正的 PREPARE 输出必须落在运行目录，不能写回共享的 `<项目>/sim/`（那里只放清单副本）。
#[test]
fn prepared_station_files_live_inside_the_run_not_the_shared_sim_folder() {
    let root = temp("prepare-paths");
    let project = root.join("projects/cmp");
    let run = crate::bench_runs::RunSnapshot::create(
        &project,
        "cmp",
        "project:\n  name: cmp\n",
        &[],
        None,
    )
    .unwrap();
    let (nc, csv) = station_files(&run, "at_neu");
    assert_eq!(nc, run.sim_dir().join("at_neu.nc"));
    assert_eq!(csv, run.sim_dir().join("at_neu.csv"));
    assert!(nc.starts_with(run.dir()), "{}", nc.display());
    assert!(!nc.starts_with(project.join("sim")), "{}", nc.display());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn projects_list_new_style_runs_by_their_creation_time() {
    let root = temp("status-order");
    for (name, created) in [("older", 1_700_000_000u64), ("newer", 1_700_000_100)] {
        let run = root.join("projects").join(name).join("runs").join("r0");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(
            run.join("meta.json"),
            json!({
                "run_id": "r0",
                "project": name,
                "created_at": "2026-01-01T00:00:00Z",
                "created_unix": created,
                "state": "running",
                "config_sha": "x",
                "sites": [],
                "machine": {"os": "macos", "arch": "arm64", "app_version": "0", "pid": 1},
            })
            .to_string(),
        )
        .unwrap();
    }
    let list = projects(&root);
    assert_eq!(list[0]["name"], "newer");
    assert_eq!(list[1]["name"], "older");
    assert_eq!(list[0]["modified"].as_u64(), Some(1_700_000_100));
    let _ = std::fs::remove_dir_all(&root);
}

// ---- 比较页取消代次（比较页复查 C21）----

/// 取消代次是进程级全局状态，这几个测试会推进它，必须串行跑：cargo test 默认并行，
/// 不锁的话一个测试的 cancel 会让另一个测试的 check 失败。
fn epoch_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn an_uncancelled_epoch_lets_the_run_continue() {
    let _lock = epoch_lock();
    let epoch = CancelEpoch::begin();
    epoch.check().unwrap();
    // 没有取消就没有失效：再取一次快照还是同一代。
    CancelEpoch::begin().check().unwrap();
    epoch.check().unwrap();
}

#[test]
fn cancelling_the_epoch_stops_runs_that_started_before_it() {
    let _lock = epoch_lock();
    let epoch = CancelEpoch::begin();
    epoch.check().unwrap();
    advance_cancel_epoch();
    assert_eq!(epoch.check().unwrap_err(), STOPPED);
    // 连续取消仍是同一条固定文字（前端按它识别 Stopped）。
    advance_cancel_epoch();
    assert_eq!(epoch.check().unwrap_err(), STOPPED);
}

#[test]
fn a_run_started_after_the_cancel_is_not_affected_by_the_old_epoch() {
    let _lock = epoch_lock();
    let old = CancelEpoch::begin();
    advance_cancel_epoch();
    assert!(old.check().is_err());
    // 新一轮运行在取消之后开始：取到新代次，上一轮的取消管不到它。
    let fresh = CancelEpoch::begin();
    fresh.check().unwrap();
    assert_eq!(old.check().unwrap_err(), STOPPED);
}

#[test]
fn a_cancelled_epoch_does_not_start_a_new_process() {
    let _lock = epoch_lock();
    let dir = temp("cancelled-spawn");
    let marker = dir.join("ran");
    let epoch = CancelEpoch::begin();
    advance_cancel_epoch();
    let mut command = Command::new("sh");
    command.args(["-c", &format!("echo ran > {}", marker.display())]);
    // spawn 之前核对代次：连进程都不该起，痕迹文件自然不该出现。
    assert_eq!(output(epoch, command).unwrap_err(), STOPPED);
    assert!(!marker.exists(), "取消后仍然启动了子进程");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stages_after_the_cancel_are_never_entered() {
    let _lock = epoch_lock();
    // 没取消：每个阶段都进。
    let epoch = CancelEpoch::begin();
    let mut entered = Vec::new();
    for_each_stage(epoch, &["prepare", "check", "run"], |stage| {
        entered.push(*stage);
        Ok(())
    })
    .unwrap();
    assert_eq!(entered, ["prepare", "check", "run"]);

    // 在 prepare 阶段里点了停止：check/run 一个都不进 —— C21 漏的就是这两个阶段。
    let epoch = CancelEpoch::begin();
    let mut entered = Vec::new();
    let error = for_each_stage(epoch, &["prepare", "check", "run"], |stage| {
        entered.push(*stage);
        if *stage == "prepare" {
            advance_cancel_epoch();
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error, STOPPED);
    assert_eq!(entered, ["prepare"]);

    // 循环开始前就已经取消：执行器一次都不调用。
    let epoch = CancelEpoch::begin();
    advance_cancel_epoch();
    let mut calls = 0;
    let error = for_each_stage(epoch, &["check", "run"], |_| {
        calls += 1;
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error, STOPPED);
    assert_eq!(calls, 0);
}

#[test]
fn ordinary_stage_failures_keep_their_own_text() {
    let _lock = epoch_lock();
    let epoch = CancelEpoch::begin();
    let error = for_each_stage(epoch, &["check"], |_| {
        Err("check 失败（exit status: 1）：\nboom".into())
    })
    .unwrap_err();
    // 普通失败不经过取消路径，格式原样带回（前端报错与运行记录都靠它）。
    assert_eq!(error, "check 失败（exit status: 1）：\nboom");
}

/// 真实子进程：`sh -c 'sleep 5'` 经 [`output`] 登记后点停止，子进程要被终止、`output` 返回"已停止"。
/// 不取消的话这条命令会睡满 5 秒 —— 用返回时间上界与进程是否还在证明它是被杀掉的，不是自然结束。
#[cfg(unix)]
#[test]
fn cancelling_kills_the_registered_child_and_reports_stopped() {
    let _lock = epoch_lock();
    let dir = temp("cancel-child");
    let pid_file = dir.join("pid");
    let epoch = CancelEpoch::begin();
    let mut command = Command::new("sh");
    command.args(["-c", &format!("echo $$ > {}; sleep 5", pid_file.display())]);
    let started = std::time::Instant::now();
    let worker = std::thread::spawn(move || output(epoch, command));
    // 等它起来：pid 文件在 exec 之后、sleep 之前写出；登记发生在 spawn 返回之后，再给登记表
    // 一点时间，免得取消落在"进程已起、还没登记"的缝里（那样就测成了取消漏杀）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !pid_file.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "子进程没能在 3 秒内起来"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    // 真正的停止按钮：先推进取消代次，再终止已登记的子进程。
    let stopped = bench_cancel().unwrap();
    assert!(stopped >= 1, "登记表里应该有一个比较页子进程");
    assert_eq!(worker.join().unwrap().unwrap_err(), STOPPED);
    // 被杀掉而不是睡满：整条命令 4 秒内返回，登记的进程也没了。
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "取消没生效，子进程跑完了：{:?}",
        started.elapsed()
    );
    let pid = std::fs::read_to_string(&pid_file).unwrap();
    assert!(!process_exists(pid.trim()), "子进程 {} 还活着", pid.trim());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn process_exists(pid: &str) -> bool {
    std::process::Command::new("ps")
        .args(["-p", pid, "-o", "stat="])
        .output()
        .map(|output| {
            let stat = String::from_utf8_lossy(&output.stdout);
            let stat = stat.trim();
            // 僵尸进程已经死了（只是暂时没人回收），不算"还活着"。
            output.status.success() && !stat.is_empty() && !stat.starts_with('Z')
        })
        .unwrap_or(false)
}

/// 历史运行列表：状态页每项的 `runs` 与 `list_runs` 完全一致（不各算一套）；旧目录/远程镜像
/// 没有 `runs/` 时是空数组，前端不必区分"没有历史"和"读取失败"。
#[test]
fn projects_expose_the_recent_runs_history_list() {
    let root = temp("status-runs-history");
    let project = root.join("projects/fresh");
    let mut first =
        crate::bench_runs::RunSnapshot::create(&project, "fresh", "cfg: first\n", &[], None)
            .unwrap();
    let reports = first.dir().join("output/fresh/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report_standalone.html");
    std::fs::write(&report, "<p>fresh</p>").unwrap();
    first.succeed(first.discover_report().as_deref()).unwrap();
    // 失败的第二跑：error 保留、report=null，并排在成功那次前面。
    let mut second =
        crate::bench_runs::RunSnapshot::create(&project, "fresh", "cfg: second\n", &[], None)
            .unwrap();
    second.fail("check 失败（exit status: 1）：\nboom").unwrap();

    let row = projects(&root)
        .into_iter()
        .find(|row| row["name"] == "fresh")
        .unwrap();
    let runs = row["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["run_id"], second.id());
    assert_eq!(runs[0]["state"], "failed");
    assert_eq!(runs[0]["report"], Value::Null);
    assert!(runs[0]["error"].as_str().unwrap().contains("check 失败"));
    assert_eq!(runs[1]["run_id"], first.id());
    assert_eq!(runs[1]["state"], "succeeded");
    assert!(same_path(runs[1]["report"].as_str().unwrap(), &report));
    // 与纯列表函数一致（同一份后端数据）。
    assert_eq!(
        row["runs"],
        json!(crate::bench_runs::list_runs(&project, 20))
    );

    // 旧目录（`runs/` 不存在）与远程镜像：空数组。
    let legacy = root.join("projects/legacy");
    std::fs::create_dir_all(legacy.join("output/old/reports")).unwrap();
    std::fs::write(legacy.join("openbench.yaml"), "legacy").unwrap();
    std::fs::write(
        legacy.join("output/old/reports/evaluation_report.html"),
        "old",
    )
    .unwrap();
    let legacy_row = projects(&root)
        .into_iter()
        .find(|row| row["name"] == "legacy")
        .unwrap();
    assert_eq!(legacy_row["runs"], json!([]));
    let _ = std::fs::remove_dir_all(&root);
}
