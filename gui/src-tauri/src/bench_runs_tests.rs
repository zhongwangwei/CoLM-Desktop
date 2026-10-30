use super::*;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-bench-runs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn project_dir(base: &Path) -> PathBuf {
    base.join("projects").join("cmp")
}

fn site(id: &str, station: &str) -> RunSite {
    RunSite {
        id: id.to_owned(),
        station: station.to_owned(),
        lon: None,
        lat: None,
        history_dir: format!("/data/{station}"),
        prefix: format!("{station}_hist_"),
    }
}

fn read_meta(run_dir: &Path) -> RunMeta {
    serde_json::from_str(&std::fs::read_to_string(run_dir.join(META_FILE)).unwrap()).unwrap()
}

/// 带 `project.output_dir` 的配置；`dir` 是原样的 YAML 值。
fn config_with_output_dir(dir: &str) -> String {
    format!("project:\n  name: cmp\n  output_dir: {dir}\n")
}

/// 模拟一次 PREPARE：在运行目录里写 NC 与站点清单，返回 NC 的实际路径。
/// 真实 PREPARE 写在 `run/sim/` 下（R04），这里用文件内容代表"这份归档数据"。
fn prepare_into_run(run: &RunSnapshot, site_id: &str, first_value: &str) -> PathBuf {
    let dir = run.sim_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let nc = dir.join(format!("{site_id}.nc"));
    std::fs::write(&nc, first_value).unwrap();
    let csv = dir.join(format!("{site_id}.csv"));
    std::fs::write(
        &csv,
        format!(
            "ID,SYEAR,EYEAR,LON,LAT,DIR\nAT-Neu,2000,2002,11.310000,47.120000,{}\n",
            nc.display()
        ),
    )
    .unwrap();
    nc
}

/// 站点清单里的 DIR 列（CSV 最后一列）：归档数据到底指向哪个文件。
fn csv_dir(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    text.lines()
        .nth(1)
        .and_then(|line| line.rsplit(',').next())
        .unwrap()
        .trim()
        .to_owned()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// 目录里残留的 `.xxx.tmp-*` 临时文件（原子写在 rename 后不该留下任何东西）。
fn dot_entries(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default()
}

/// 在运行目录的 `output/` 下放一份"成功运行该有"的报告。
fn put_report(run: &RunSnapshot) -> PathBuf {
    let reports = run.dir().join("output/cmp/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report_standalone.html");
    std::fs::write(&report, "<p>ok</p>").unwrap();
    report
}

/// 手工写一份 meta：测排序/容错时不必真的跑一次运行。
fn write_run_meta(project: &Path, run_id: &str, created_unix: u64, state: RunState) -> PathBuf {
    let dir = project.join(RUNS_DIR).join(run_id);
    std::fs::create_dir_all(&dir).unwrap();
    let meta = RunMeta {
        run_id: run_id.to_owned(),
        project: "cmp".to_owned(),
        created_at: iso8601_utc(created_unix),
        created_unix,
        finished_at: None,
        finished_unix: None,
        state,
        config_sha: "abc".to_owned(),
        ref_root: None,
        sites: Vec::new(),
        machine: Machine::default(),
        output_dir: DEFAULT_OUTPUT_DIR.to_owned(),
        report: None,
        error: None,
    };
    write_listed_meta(&dir, &meta);
    dir
}

/// 手工写一份带报告/失败原因的 meta：测 `runs[]` 的报告可见性，不必真的跑一次运行。
fn write_run_with_report(
    project: &Path,
    run_id: &str,
    created_unix: u64,
    state: RunState,
    report: Option<&str>,
    error: Option<&str>,
) -> PathBuf {
    let dir = project.join(RUNS_DIR).join(run_id);
    std::fs::create_dir_all(&dir).unwrap();
    let meta = RunMeta {
        run_id: run_id.to_owned(),
        project: "cmp".to_owned(),
        created_at: iso8601_utc(created_unix),
        created_unix,
        finished_at: Some(iso8601_utc(created_unix + 5)),
        finished_unix: Some(created_unix + 5),
        state,
        config_sha: "abc".to_owned(),
        ref_root: None,
        sites: Vec::new(),
        machine: Machine::default(),
        output_dir: DEFAULT_OUTPUT_DIR.to_owned(),
        report: report.map(str::to_owned),
        error: error.map(str::to_owned),
    };
    write_listed_meta(&dir, &meta);
    dir
}

fn write_listed_meta(run_dir: &Path, meta: &RunMeta) {
    std::fs::write(
        run_dir.join(META_FILE),
        serde_json::to_string_pretty(meta).unwrap(),
    )
    .unwrap();
}

/// JSON 里的路径由原生目录拼接，Windows 上两种分隔符会混在一行；断言按分隔符归一后比较。
fn same_path(actual: &str, expected: &Path) -> bool {
    actual.replace('\\', "/") == expected.to_string_lossy().replace('\\', "/")
}

#[test]
fn utc_timestamps_match_known_instants() {
    assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601_utc(946_684_800), "2000-01-01T00:00:00Z");
    // 闰日：2024-02-29 是 21 世纪第一个闰年的 2 月最后一天。
    assert_eq!(iso8601_utc(1_709_206_496), "2024-02-29T11:34:56Z");
    assert_eq!(iso8601_utc(1_730_000_000), "2024-10-27T03:33:20Z");
    assert_eq!(compact_utc(946_684_800), "20000101T000000Z");
}

#[test]
fn a_run_snapshot_freezes_its_config_sites_and_inputs() {
    let base = temp("freeze");
    let project = project_dir(&base);
    let config = "# CoLM Desktop 生成\nproject:\n  name: cmp\n  output_dir: ./output\n";
    let mut sites = vec![site("at_neu", "AT-Neu"), site("ch_lae", "CH-Lae")];
    sites[0].lon = Some(11.31);
    sites[0].lat = Some(47.12);
    let run = RunSnapshot::create(&project, "cmp", config, &sites, Some("/refs")).unwrap();

    // 布局：runs/<run_id>/ 下就是这次运行的全部身份。
    assert_eq!(run.dir(), project.join(RUNS_DIR).join(run.id()));
    assert!(run.config_file().starts_with(run.dir()));
    // 配置逐字节冻结：check/run 用的就是这份文本，后面读回来也必须是同样字节。
    assert_eq!(std::fs::read(run.config_file()).unwrap(), config.as_bytes());
    assert_eq!(run.config_sha(), colm_kernel::sha256_hex(config.as_bytes()));

    let meta = read_meta(run.dir());
    assert_eq!(meta.run_id, run.id());
    assert_eq!(meta.project, "cmp");
    assert_eq!(meta.state, RunState::Running);
    assert!(meta.created_at.ends_with('Z'));
    assert_eq!(
        meta.created_at.len(),
        20,
        "ISO-8601 秒级 UTC：{}",
        meta.created_at
    );
    assert_eq!(iso8601_utc(meta.created_unix), meta.created_at);
    assert_eq!(meta.config_sha, run.config_sha());
    assert_eq!(meta.ref_root.as_deref(), Some("/refs"));
    assert_eq!(meta.sites, sites);
    assert!(meta.finished_at.is_none() && meta.finished_unix.is_none());
    assert!(meta.report.is_none() && meta.error.is_none());
    assert_eq!(meta.machine.os, std::env::consts::OS);
    assert_eq!(meta.machine.arch, std::env::consts::ARCH);
    assert_eq!(meta.machine.app_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(meta.machine.pid, std::process::id());

    let sites_snapshot = read_json(&run.dir().join(SITES_FILE));
    assert_eq!(sites_snapshot[0]["station"], "AT-Neu");
    assert_eq!(sites_snapshot[0]["lon"], 11.31);
    assert_eq!(sites_snapshot[1]["lon"], Value::Null);
    let inputs = read_json(&run.dir().join(INPUTS_FILE));
    assert_eq!(inputs["config_sha"], run.config_sha());
    assert_eq!(inputs["ref_root"], "/refs");
    assert_eq!(inputs["sites"][0]["history_dir"], "/data/AT-Neu");
    assert_eq!(inputs["sites"][0]["prefix"], "AT-Neu_hist_");

    // 站点清单是 PREPARE 写进本次运行目录的小 CSV；发布到项目 sim/ 后，内容（含 DIR）不变。
    let frozen_nc = prepare_into_run(&run, "at_neu", "10");
    run.publish_station_manifest("at_neu").unwrap();
    assert_eq!(
        std::fs::read(run.dir().join("sim/at_neu.csv")).unwrap(),
        std::fs::read(project.join("sim/at_neu.csv")).unwrap()
    );
    assert_eq!(
        csv_dir(&project.join("sim/at_neu.csv")),
        frozen_nc.to_string_lossy()
    );

    // 运行目录里只有快照文件，没有原子写的临时残骸。
    let mut names = std::fs::read_dir(run.dir())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "inputs.json",
            "meta.json",
            "openbench.yaml",
            "sim",
            "sites.json"
        ]
    );
    assert!(
        dot_entries(run.dir()).is_empty(),
        "{:?}",
        dot_entries(run.dir())
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn run_ids_are_unique_and_name_ordered() {
    let base = temp("ids");
    let project = project_dir(&base);
    let first = RunSnapshot::create(&project, "cmp", "a: 1\n", &[], None).unwrap();
    let second = RunSnapshot::create(&project, "cmp", "b: 2\n", &[], None).unwrap();
    assert_ne!(first.id(), second.id());
    // UTC 时间戳在前：名字的字典序就是运行先后（同秒靠递增序号）。
    assert!(
        first.id() < second.id(),
        "{} vs {}",
        first.id(),
        second.id()
    );
    let parts = first.id().split('-').collect::<Vec<_>>();
    assert_eq!(parts.len(), 3, "{}", first.id());
    assert!(
        parts[0].ends_with('Z') && parts[0].contains('T'),
        "{}",
        first.id()
    );
    assert_eq!(parts[1], std::process::id().to_string());
    parts[2].parse::<u64>().unwrap();
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_successful_run_updates_the_pointer_and_records_its_own_report() {
    let base = temp("success");
    let project = project_dir(&base);
    let mut first = RunSnapshot::create(
        &project,
        "cmp",
        "cfg: first\n",
        &[site("a", "AT-Neu")],
        None,
    )
    .unwrap();
    let report = put_report(&first);
    assert_eq!(first.discover_report().as_deref(), Some(report.as_path()));
    let latest = first.succeed(first.discover_report().as_deref()).unwrap();

    assert_eq!(latest.run_id, first.id());
    assert_eq!(latest.config_sha, first.config_sha());
    // 报告路径相对运行目录存：整个 openbench 目录搬家后指针仍然有效。
    assert_eq!(
        latest.report.as_deref(),
        Some("output/cmp/reports/evaluation_report_standalone.html")
    );
    let meta = read_meta(first.dir());
    assert_eq!(meta.state, RunState::Succeeded);
    assert_eq!(
        meta.finished_at.as_deref(),
        Some(latest.finished_at.as_str())
    );
    assert_eq!(meta.report, latest.report);
    assert!(meta.error.is_none());

    // 指针文件读回来与返回值一致，且能解析到本次运行目录里的报告。
    let stored = latest_success(&project).unwrap();
    assert_eq!(stored, latest);
    assert_eq!(
        stored.report_path(&project).as_deref(),
        Some(report.as_path())
    );
    let record = latest_run(&project).unwrap();
    assert_eq!(record.meta().state, RunState::Succeeded);
    assert_eq!(record.report_path().as_deref(), Some(report.as_path()));
    let json = record.to_json();
    assert_eq!(json["run_id"], first.id());
    assert_eq!(json["state"], "succeeded");
    assert_eq!(json["config_sha"], first.config_sha());
    // 记录里的相对路径用 `/`，与原生目录拼出来的绝对路径在 Windows 上会混用两种分隔符。
    let json_report = json["report"].as_str().unwrap().replace('\\', "/");
    let expected_report = report.to_string_lossy().replace('\\', "/");
    assert_eq!(json_report, expected_report);
    assert_eq!(json["error"], Value::Null);

    // 指针与 meta 都是原子写：目标目录里没有以点开头的临时残骸。
    assert!(
        dot_entries(&project).is_empty(),
        "{:?}",
        dot_entries(&project)
    );
    assert!(
        dot_entries(first.dir()).is_empty(),
        "{:?}",
        dot_entries(first.dir())
    );
    let mut project_names = std::fs::read_dir(&project)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    project_names.sort();
    assert_eq!(project_names, ["latest-success.json", "runs"]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_failed_run_keeps_the_previous_success_pointer_and_its_own_diagnosis() {
    let base = temp("failed");
    let project = project_dir(&base);
    let mut first = RunSnapshot::create(&project, "cmp", "cfg: first\n", &[], None).unwrap();
    let report = put_report(&first);
    first.succeed(first.discover_report().as_deref()).unwrap();
    let pointer = std::fs::read_to_string(project.join(LATEST_SUCCESS_FILE)).unwrap();

    // B 改了设置再跑：配置冻结进 B 自己的目录，跑失败。半份 output 里即使有 HTML 也不算报告。
    let mut second = RunSnapshot::create(&project, "cmp", "cfg: second\n", &[], None).unwrap();
    let half = second
        .dir()
        .join("output/cmp/reports/evaluation_report.html");
    std::fs::create_dir_all(half.parent().unwrap()).unwrap();
    std::fs::write(&half, "<p>half written</p>").unwrap();
    second
        .fail("prepare 失败（exit status: 1）：\nboom")
        .unwrap();

    // 上次成功指针一个字节都没动。
    assert_eq!(
        std::fs::read_to_string(project.join(LATEST_SUCCESS_FILE)).unwrap(),
        pointer
    );
    let stored = latest_success(&project).unwrap();
    assert_eq!(stored.run_id, first.id());
    assert_eq!(
        stored.report_path(&project).as_deref(),
        Some(report.as_path())
    );

    let meta = read_meta(second.dir());
    assert_eq!(meta.state, RunState::Failed);
    assert_eq!(meta.config_sha, colm_kernel::sha256_hex(b"cfg: second\n"));
    assert!(meta.finished_at.is_some());
    assert!(meta.report.is_none());
    assert!(meta.error.as_deref().unwrap().contains("prepare 失败"));

    // 状态页的"最近一次运行"是失败的 B，报告为 null；成功的 A 仍由上次成功指针给出。
    let record = latest_run(&project).unwrap();
    assert_eq!(record.to_json()["run_id"], second.id());
    assert_eq!(record.to_json()["state"], "failed");
    assert_eq!(record.to_json()["report"], Value::Null);
    assert!(record.to_json()["error"]
        .as_str()
        .unwrap()
        .contains("prepare 失败"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn reports_are_only_discovered_in_their_own_run() {
    let base = temp("own-run");
    let project = project_dir(&base);
    let mut first = RunSnapshot::create(&project, "cmp", "cfg: a\n", &[], None).unwrap();
    put_report(&first);
    first.succeed(first.discover_report().as_deref()).unwrap();

    // 旧版本留在项目根的 output/：新目录有 runs/ 之后绝不能再被当成当前报告。
    let legacy = project.join("output/legacy/reports/evaluation_report_standalone.html");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "<p>legacy</p>").unwrap();

    let second = RunSnapshot::create(&project, "cmp", "cfg: b\n", &[], None).unwrap();
    assert!(
        second.discover_report().is_none(),
        "不能跨 run 或跨旧目录取报告"
    );
    // 失败/进行中的运行在状态页上报告就是 null，哪怕同一项目里有 A 的成功报告。
    let record = latest_run(&project).unwrap();
    assert_eq!(record.meta().state, RunState::Running);
    assert_eq!(record.report_path(), None);
    assert_eq!(record.to_json()["report"], Value::Null);
    assert!(latest_success(&project)
        .unwrap()
        .report_path(&project)
        .is_some());

    // 本次运行的 output 里出报告之后才算数。
    let own = put_report(&second);
    assert_eq!(second.discover_report().as_deref(), Some(own.as_path()));
    assert!(
        !own.starts_with(project.join("output")),
        "报告必须在本 run 目录里"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_successful_run_without_a_report_keeps_a_null_report_but_still_records_identity() {
    let base = temp("no-report");
    let project = project_dir(&base);
    let mut run = RunSnapshot::create(&project, "cmp", "cfg: x\n", &[], None).unwrap();
    assert!(run.discover_report().is_none());
    let latest = run.succeed(None).unwrap();
    assert!(latest.report.is_none());
    assert!(latest.report_path(&project).is_none());
    let stored = latest_success(&project).unwrap();
    assert_eq!(stored.run_id, run.id());
    assert_eq!(stored.report, None);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_long_failure_reason_is_truncated_in_meta() {
    let base = temp("truncate");
    let project = project_dir(&base);
    let mut run = RunSnapshot::create(&project, "cmp", "cfg: x\n", &[], None).unwrap();
    let detail = "x".repeat(ERROR_LIMIT * 3);
    run.fail(&detail).unwrap();
    let meta = read_meta(run.dir());
    let error = meta.error.unwrap();
    assert_eq!(error.chars().count(), ERROR_LIMIT + 1);
    assert!(error.ends_with('…'), "{}", &error[error.len() - 8..]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_newer_run_wins_even_if_its_meta_is_readable_but_the_newer_one_is_corrupt() {
    let base = temp("ordering");
    let project = project_dir(&base);
    let old = write_run_meta(
        &project,
        "20260101T000000Z-1-0",
        1_767_225_600,
        RunState::Succeeded,
    );
    let new = write_run_meta(
        &project,
        "20260102T000000Z-1-0",
        1_767_312_000,
        RunState::Failed,
    );
    // 更晚但 meta 损坏的目录：跳过它，不能把整页状态打挂，也不能让旧记录冒充最新。
    let broken = project.join(RUNS_DIR).join("20260103T000000Z-1-0");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join(META_FILE), "{ not json").unwrap();
    // 非目录条目也跳过。
    std::fs::write(project.join(RUNS_DIR).join("stray"), "x").unwrap();

    let record = latest_run(&project).unwrap();
    assert_eq!(record.dir(), new.as_path());
    assert_eq!(record.meta().state, RunState::Failed);
    assert!(old.exists());

    // 指针坏了当作没有（状态页只是少一行），不能让读取失败。
    std::fs::write(project.join(LATEST_SUCCESS_FILE), "not a pointer").unwrap();
    assert!(latest_success(&project).is_none());
    // 没有 runs/ 的旧目录不算新格式。
    let legacy = project.parent().unwrap().join("legacy");
    std::fs::create_dir_all(&legacy).unwrap();
    assert!(!has_runs(&legacy));
    assert!(latest_run(&legacy).is_none());
    assert!(latest_success(&legacy).is_none());
    let _ = std::fs::remove_dir_all(&base);
}

// ---- R03：output_dir 不能带报告跑出这次运行 ----

#[test]
fn a_custom_output_dir_is_normalized_inside_the_run_directory() {
    let base = temp("output-dir-ok");
    let project = project_dir(&base);
    let config = config_with_output_dir("./custom-output/");
    let mut run = RunSnapshot::create(&project, "cmp", &config, &[], None).unwrap();
    assert_eq!(run.meta.output_dir, "custom-output");
    // 配置文本逐字节冻结，但报告发现按归一后的目录走。
    assert_eq!(std::fs::read(run.config_file()).unwrap(), config.as_bytes());

    let reports = run.dir().join("custom-output/cmp/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report_standalone.html");
    std::fs::write(&report, "<p>ok</p>").unwrap();
    assert_eq!(run.discover_report().as_deref(), Some(report.as_path()));
    run.succeed(run.discover_report().as_deref()).unwrap();
    assert_eq!(
        latest_run(&project).unwrap().report_path().as_deref(),
        Some(report.as_path())
    );
    assert_eq!(
        latest_success(&project)
            .unwrap()
            .report_path(&project)
            .as_deref(),
        Some(report.as_path())
    );

    // `a/../output` 这类能在词法上落回运行目录的写法安全归一，不拒绝。
    let base = temp("output-dir-dotdot");
    let project = project_dir(&base);
    let run = RunSnapshot::create(
        &project,
        "cmp",
        &config_with_output_dir("sim/../nested/output"),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(run.meta.output_dir, "nested/output");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_run_refuses_an_output_dir_that_can_escape_or_leaves_the_run_root() {
    let base = temp("output-dir-bad");
    let project = project_dir(&base);
    let absolute = if cfg!(windows) {
        "C:\\Users\\shared\\output"
    } else {
        "/tmp/shared-output"
    };
    for raw in [
        "../shared",
        "../../outside",
        "sim/../../outside",
        absolute,
        // 归一后就是运行目录本身：不要跟冻结文件混在一起。
        ".",
        "sim/..",
        // 对面一旦展开 ~ 就跑到运行目录外。
        "~/shared",
    ] {
        let error = RunSnapshot::create(&project, "cmp", &config_with_output_dir(raw), &[], None)
            .err()
            .expect("不安全的 output_dir 必须被拒绝");
        assert!(
            error.contains("output_dir"),
            "{raw:?} 的拒绝理由要说清是 output_dir：{error}"
        );
        assert!(
            !project.join(RUNS_DIR).exists(),
            "{raw:?} 被拒绝前不能留下运行目录"
        );
    }
    // `output_dir:` 空值在 YAML 是 null，按 OpenBench 默认的 output 处理（不是"跳到别处"）。
    let defaulted =
        RunSnapshot::create(&project, "cmp", &config_with_output_dir(""), &[], None).unwrap();
    assert_eq!(defaulted.meta.output_dir, DEFAULT_OUTPUT_DIR);
    // 不写在 project 段里的 output_dir 不能猜：它可能是别的块的，OpenBench 也不会读它。
    let stray = "output_dir: ../shared\nproject:\n  name: cmp\n";
    assert!(RunSnapshot::create(&project, "cmp", stray, &[], None).is_err());
    // 这些写法能让 OpenBench 看到另一份 output_dir 而逐行扫描看不到：一律拒绝，不猜。
    for unsupported in [
        "project: {name: cmp, output_dir: /tmp/x}\n",
        "project:\n  <<: *defaults\n",
        "---\nproject:\n  output_dir: ./x\n",
        "project:\n  name: cmp\nproject:\n  output_dir: ./x\n",
    ] {
        assert!(
            RunSnapshot::create(&project, "cmp", unsupported, &[], None).is_err(),
            "{unsupported:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_config_without_output_dir_keeps_the_default() {
    let base = temp("output-dir-default");
    let project = project_dir(&base);
    let run = RunSnapshot::create(&project, "cmp", "project:\n  name: cmp\n", &[], None).unwrap();
    assert_eq!(run.meta.output_dir, "output");
    let reports = run.dir().join("output/cmp/reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report.html");
    std::fs::write(&report, "<p>ok</p>").unwrap();
    assert_eq!(run.discover_report().as_deref(), Some(report.as_path()));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn an_old_meta_without_output_dir_still_finds_the_default_report() {
    let base = temp("output-dir-old-meta");
    let project = project_dir(&base);
    let dir = write_run_meta(&project, "r0", 1_700_000_000, RunState::Succeeded);
    // 旧 meta 没有 output_dir 字段：解析成默认 output，不能因此读不出状态页。
    let record = latest_run(&project).unwrap();
    assert_eq!(record.meta().output_dir, DEFAULT_OUTPUT_DIR);
    assert_eq!(record.dir(), dir.as_path());
    let _ = std::fs::remove_dir_all(&base);
}

// ---- R04：归档运行的数据不能被下一次准备覆盖 ----

#[test]
fn a_later_prepare_cannot_change_the_data_an_archived_run_used() {
    let base = temp("freeze-input");
    let project = project_dir(&base);
    let config = config_with_output_dir("./output");

    // A：首值 10，归档进自己的运行目录；配置里的 fulllist 位置（项目 sim/）指向归档 NC。
    let mut run_a =
        RunSnapshot::create(&project, "cmp", &config, &[site("a", "AT-Neu")], None).unwrap();
    let archived_nc = prepare_into_run(&run_a, "a", "10");
    run_a.publish_station_manifest("a").unwrap();
    run_a.succeed(None).unwrap();
    let published_a = project.join("sim/a.csv");
    assert_eq!(csv_dir(&published_a), archived_nc.to_string_lossy());
    assert!(std::fs::read_to_string(&published_a)
        .unwrap()
        .contains("10"));

    // B：覆盖准备（首值 90）。项目 sim/ 是共享路径，会被重写成 B 的清单；A 的归档不能被碰到。
    let run_b =
        RunSnapshot::create(&project, "cmp", &config, &[site("a", "AT-Neu")], None).unwrap();
    let prepared_b = prepare_into_run(&run_b, "a", "90");
    run_b.publish_station_manifest("a").unwrap();
    let published_b = project.join("sim/a.csv");
    assert_eq!(csv_dir(&published_b), prepared_b.to_string_lossy());
    assert_ne!(archived_nc, prepared_b);

    // 按 A 的归档读：清单仍指向 A 的 NC，内容仍是 10。
    assert_eq!(
        csv_dir(&run_a.sim_dir().join("a.csv")),
        archived_nc.to_string_lossy()
    );
    assert_eq!(std::fs::read_to_string(&archived_nc).unwrap(), "10");
    // 按 B 的归档读：90；两次运行互不覆盖。
    assert_eq!(std::fs::read_to_string(&prepared_b).unwrap(), "90");
    assert_eq!(std::fs::read_to_string(&archived_nc).unwrap(), "10");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_flat_mirror_with_a_run_record_is_not_treated_as_a_legacy_project() {
    let base = temp("flat-mirror");
    let mirror = base.join("projects").join("cmp@host");
    let meta = RunMeta {
        run_id: "bench-run-1-2".to_owned(),
        project: "cmp".to_owned(),
        created_at: iso8601_utc(1_700_000_000),
        created_unix: 1_700_000_000,
        finished_at: Some(iso8601_utc(1_700_000_100)),
        finished_unix: Some(1_700_000_100),
        state: RunState::Succeeded,
        config_sha: "abc".to_owned(),
        ref_root: None,
        sites: Vec::new(),
        machine: Machine::default(),
        output_dir: "custom".to_owned(),
        report: Some("custom/cmp/reports/evaluation_report.html".to_owned()),
        error: None,
    };
    std::fs::create_dir_all(mirror.join("custom/cmp/reports")).unwrap();
    let report = mirror.join("custom/cmp/reports/evaluation_report.html");
    std::fs::write(&report, "<p>ok</p>").unwrap();
    std::fs::write(
        mirror.join(META_FILE),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();
    assert!(!has_runs(&mirror));
    let record = flat_run(&mirror).unwrap();
    assert_eq!(record.meta(), &meta);
    assert_eq!(record.report_path().as_deref(), Some(report.as_path()));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn list_runs_is_newest_first_and_capped() {
    let base = temp("list-order");
    let project = project_dir(&base);
    // 25 条不同时刻的运行，状态混着来：列表按时间排，不按状态挑。
    for index in 0..25u64 {
        let state = match index % 3 {
            0 => RunState::Succeeded,
            1 => RunState::Failed,
            _ => RunState::Running,
        };
        write_run_meta(
            &project,
            &format!("20260101T0000{index:02}Z-1-{index:06}"),
            1_700_000_000 + index,
            state,
        );
    }
    // 同一秒的两条：run_id（时间戳+补零序号）大的在前。
    let same_second_early = "20260102T000000Z-1-000001";
    let same_second_late = "20260102T000000Z-1-000002";
    write_run_meta(&project, same_second_early, 1_700_000_100, RunState::Failed);
    write_run_meta(
        &project,
        same_second_late,
        1_700_000_100,
        RunState::Succeeded,
    );

    let mut expected = (0..25u64)
        .map(|index| {
            (
                1_700_000_000 + index,
                format!("20260101T0000{index:02}Z-1-{index:06}"),
            )
        })
        .collect::<Vec<_>>();
    expected.push((1_700_000_100, same_second_early.to_owned()));
    expected.push((1_700_000_100, same_second_late.to_owned()));
    expected.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let expected = expected
        .iter()
        .take(20)
        .map(|(_, run_id)| run_id.as_str())
        .collect::<Vec<_>>();
    let actual = list_runs(&project, 20)
        .iter()
        .map(|run| run["run_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    // limit=0 一条都不要；给大了就是全部（27 条）。
    assert_eq!(list_runs(&project, 0), Vec::<Value>::new());
    assert_eq!(list_runs(&project, 100).len(), 27);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn list_runs_only_links_the_report_of_a_succeeded_run_that_still_lives_inside_it() {
    let base = temp("list-report");
    let project = project_dir(&base);
    let relative_report = "output/cmp/reports/evaluation_report_standalone.html";
    // 成功且报告文件还在：给绝对路径。
    let ok = write_run_with_report(
        &project,
        "run-ok",
        500,
        RunState::Succeeded,
        Some(relative_report),
        None,
    );
    std::fs::create_dir_all(ok.join("output/cmp/reports")).unwrap();
    let report = ok.join(relative_report);
    std::fs::write(&report, "<p>ok</p>").unwrap();
    // 成功但报告文件被手工删掉：null。
    write_run_with_report(
        &project,
        "run-gone",
        400,
        RunState::Succeeded,
        Some("output/cmp/reports/gone.html"),
        None,
    );
    // 失败：即使 meta 残留路径、那个半份 HTML 也还在，也不当可打开的报告。
    let failed = write_run_with_report(
        &project,
        "run-failed",
        300,
        RunState::Failed,
        Some("output/cmp/reports/evaluation_report.html"),
        Some("check 失败（exit status: 1）：\nboom"),
    );
    std::fs::create_dir_all(failed.join("output/cmp/reports")).unwrap();
    std::fs::write(
        failed.join("output/cmp/reports/evaluation_report.html"),
        "<p>half</p>",
    )
    .unwrap();
    // 绝对路径（即使指向真实存在的文件）与 `..` 逃逸都不认。
    let outside = base.join("outside.html");
    std::fs::write(&outside, "outside").unwrap();
    write_run_with_report(
        &project,
        "run-absolute",
        200,
        RunState::Succeeded,
        Some(outside.to_str().unwrap()),
        None,
    );
    let escaped = format!("../run-ok/{relative_report}");
    write_run_with_report(
        &project,
        "run-dotdot",
        100,
        RunState::Succeeded,
        Some(&escaped),
        None,
    );

    let runs = list_runs(&project, 20);
    assert_eq!(runs.len(), 5);
    assert_eq!(runs[0]["run_id"], "run-ok");
    let by_id = |run_id: &str| {
        runs.iter()
            .find(|run| run["run_id"] == run_id)
            .unwrap_or_else(|| panic!("{run_id} 不在列表里"))
    };
    // 前端 ProjectRun 的每个字段都在。
    for key in [
        "run_id",
        "state",
        "report",
        "finished_at",
        "created_at",
        "config_sha",
        "error",
    ] {
        assert!(by_id("run-ok").get(key).is_some(), "缺少字段 {key}");
    }
    assert_eq!(by_id("run-ok")["state"], "succeeded");
    assert!(same_path(
        by_id("run-ok")["report"].as_str().unwrap(),
        &report
    ));
    assert_eq!(by_id("run-ok")["created_at"], iso8601_utc(500));
    assert_eq!(by_id("run-ok")["finished_at"], iso8601_utc(505));
    assert_eq!(by_id("run-ok")["config_sha"], "abc");
    assert_eq!(by_id("run-ok")["error"], Value::Null);
    assert_eq!(by_id("run-gone")["report"], Value::Null);
    assert_eq!(by_id("run-failed")["report"], Value::Null);
    assert!(by_id("run-failed")["error"]
        .as_str()
        .unwrap()
        .contains("check 失败"));
    assert_eq!(by_id("run-absolute")["report"], Value::Null);
    assert_eq!(by_id("run-dotdot")["report"], Value::Null);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn list_runs_skips_bad_meta_and_degrades_old_records_without_losing_the_rest() {
    let base = temp("list-bad");
    let project = project_dir(&base);
    write_run_meta(&project, "good", 100, RunState::Succeeded);
    let runs_dir = project.join(RUNS_DIR);
    // 坏 JSON、缺 meta、超大文件、runs/ 下的普通文件：都跳过，但不能影响别的 run。
    let bad = runs_dir.join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join(META_FILE), "{ not json").unwrap();
    std::fs::create_dir_all(runs_dir.join("in-progress")).unwrap();
    let huge = runs_dir.join("huge");
    std::fs::create_dir_all(&huge).unwrap();
    std::fs::write(
        huge.join(META_FILE),
        "x".repeat((META_LIST_BYTES_LIMIT + 1) as usize),
    )
    .unwrap();
    std::fs::write(runs_dir.join("stray"), "x").unwrap();
    // 缺 created_unix/created_at/finished_at 的旧 meta：降级为 0 排最后，但字段不丢。
    let legacy = runs_dir.join("legacy");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join(META_FILE),
        json!({
            "run_id": "legacy",
            "project": "cmp",
            "state": "failed",
            "config_sha": "old-sha",
            "error": "旧版失败原因",
        })
        .to_string(),
    )
    .unwrap();

    let runs = list_runs(&project, 20);
    let ids = runs
        .iter()
        .map(|run| run["run_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["good", "legacy"]);
    let legacy = &runs[1];
    assert_eq!(legacy["state"], "failed");
    assert_eq!(legacy["error"], "旧版失败原因");
    assert_eq!(legacy["config_sha"], "old-sha");
    assert_eq!(legacy["created_at"], Value::Null);
    assert_eq!(legacy["finished_at"], Value::Null);
    assert_eq!(legacy["report"], Value::Null);

    // meta 里的 run_id 为空时用目录名兜底（目录名才是运行身份）。
    let unnamed = runs_dir.join("dir-name-is-the-id");
    std::fs::create_dir_all(&unnamed).unwrap();
    std::fs::write(
        unnamed.join(META_FILE),
        json!({"run_id": "", "state": "running", "created_unix": 1}).to_string(),
    )
    .unwrap();
    let runs = list_runs(&project, 20);
    let ids = runs
        .iter()
        .map(|run| run["run_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["good", "dir-name-is-the-id", "legacy"]);
    assert_eq!(runs[1]["state"], "running");
    assert_eq!(runs[1]["report"], Value::Null);

    // 旧目录没有 runs/：空数组，不是错误。
    let old = base.join("projects").join("old-style");
    std::fs::create_dir_all(&old).unwrap();
    assert!(list_runs(&old, 20).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}
