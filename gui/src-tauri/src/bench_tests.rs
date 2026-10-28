use super::*;

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
    assert_eq!(fresh["report"].as_str().unwrap(), report.to_string_lossy());
    assert_eq!(fresh["latest_run"]["run_id"], second.id());
    assert_eq!(fresh["latest_run"]["state"], "failed");
    assert_eq!(fresh["latest_run"]["report"], Value::Null);
    assert!(fresh["latest_run"]["error"]
        .as_str()
        .unwrap()
        .contains("check 失败"));
    assert_eq!(fresh["latest_success"]["run_id"], first.id());
    assert_eq!(
        fresh["latest_success"]["report"].as_str().unwrap(),
        report.to_string_lossy()
    );
    // 配置显示的是最新一次运行的冻结文本（B 提交的那份），不是项目根的历史配置。
    assert_eq!(
        fresh["config"].as_str().unwrap(),
        second.config_file().to_string_lossy()
    );

    let legacy_row = list.iter().find(|row| row["name"] == "legacy").unwrap();
    assert_eq!(
        legacy_row["report"].as_str().unwrap(),
        legacy_report.to_string_lossy()
    );
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
