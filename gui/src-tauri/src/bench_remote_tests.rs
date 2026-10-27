use std::path::{Path, PathBuf};

use super::*;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-remote-fetch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn mirror(base: &Path) -> PathBuf {
    base.join("projects").join("qfo_era5@7920ts")
}

/// 一份"上次成功取回"的本机镜像：有报告、有配置，还有一张这次取回可能已经没有的旧图。
fn previous_mirror(base: &Path) -> PathBuf {
    let local = mirror(base);
    std::fs::create_dir_all(local.join("output/older/reports")).unwrap();
    std::fs::write(local.join("openbench.yaml"), "old config").unwrap();
    std::fs::write(
        local.join("output/older/reports/evaluation_report.html"),
        "<p>old</p>",
    )
    .unwrap();
    std::fs::write(
        local.join("stale.png"),
        "old image the next fetch no longer has",
    )
    .unwrap();
    local
}

/// 在一次取回的临时目录里放一份新报告，返回报告路径。
fn staged_report(staging: &Path, project: &str, body: &str) -> PathBuf {
    let reports = staging.join("output").join(project).join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    let report = reports.join("evaluation_report_standalone.html");
    std::fs::write(&report, body).unwrap();
    report
}

fn leftovers(base: &Path) -> Vec<String> {
    match std::fs::read_dir(base.join(FETCH_WORK_DIR)) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn fetched_results_land_in_a_folder_named_after_comparison_and_server() {
    assert_eq!(
        mirror_name("qfo_era5", "7920ts").unwrap(),
        "qfo_era5@7920ts"
    );
    // 服务器名里的点、@ 之类换掉，不会多出一层目录。
    assert_eq!(
        mirror_name("a", "user@host.example/x").unwrap(),
        "a@user_host_example_x"
    );
    assert!(mirror_name("../a", "h").is_err());
    assert!(mirror_name("a", "").is_err());
}

#[test]
fn only_this_pages_jobs_can_be_watched_or_cancelled() {
    check_job("bench-run-84692-1792749735388362000").unwrap();
    for bad in ["run-123", "bench-../../x", "bench-a b", ""] {
        assert!(check_job(bad).is_err(), "{bad:?}");
    }
    assert!(server_dir("/data/x").is_ok());
    assert!(server_dir("data/x").is_err());
}

#[test]
fn a_fetch_stages_outside_the_projects_folder_and_leaves_the_old_mirror_alone() {
    let base = temp("staging");
    let local = previous_mirror(&base);
    let staging = FetchStaging::begin(&local).unwrap();
    assert!(
        staging.dir().starts_with(base.join(FETCH_WORK_DIR)),
        "临时目录应在 {} 下：{}",
        FETCH_WORK_DIR,
        staging.dir().display()
    );
    // 旧实现第一件事就是 remove_dir_all；下载期间旧镜像必须原样还在。
    assert_eq!(
        read(local.join("output/older/reports/evaluation_report.html")),
        "<p>old</p>"
    );
    assert!(local.join("stale.png").is_file());
    let staged = staging.dir().to_path_buf();
    drop(staging);
    assert!(!staged.exists());
    assert!(local.join("stale.png").is_file());
    assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
}

#[test]
fn a_failed_fetch_keeps_the_previous_report_openable_and_says_so() {
    let base = temp("failed");
    let local = previous_mirror(&base);
    for (tag, detail) in [
        ("auth", "Permission denied (publickey)"),
        (
            "connect",
            "ssh: connect to host 7920ts port 22: Connection timed out",
        ),
        ("interrupted", "download interrupted: Broken pipe"),
    ] {
        let staging = FetchStaging::begin(&local).unwrap();
        // 下载到一半断开：临时目录里可能已经有半份内容，绝不能进本机镜像。
        let partial = staged_report(staging.dir(), "newer", "half a report");
        let error = staging.finish(Err(detail.to_string())).unwrap_err();
        assert!(error.contains("取回失败"), "{tag}: {error}");
        assert!(error.contains(detail), "{tag}: {error}");
        assert!(
            error.contains("本机旧报告未改动，仍是上次成功版本"),
            "{tag}: {error}"
        );
        assert!(!partial.exists(), "{tag}: 本次临时内容应清理");
        assert_eq!(read(local.join("openbench.yaml")), "old config");
        assert_eq!(
            read(local.join("output/older/reports/evaluation_report.html")),
            "<p>old</p>"
        );
        assert_eq!(
            crate::bench::find_report(&local.join("output")),
            Some(local.join("output/older/reports/evaluation_report.html")),
            "{tag}: 旧报告必须仍然可打开"
        );
    }
    assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
}

#[test]
fn a_successful_fetch_replaces_the_whole_mirror_without_mixing_two_reports() {
    let base = temp("replace");
    let local = previous_mirror(&base);
    let staging = FetchStaging::begin(&local).unwrap();
    let staged = staged_report(staging.dir(), "newer", "<p>new</p>");
    std::fs::create_dir_all(staging.dir().join("sim")).unwrap();
    std::fs::write(staging.dir().join("sim/site.csv"), "new sim").unwrap();
    std::fs::write(staging.dir().join("openbench.yaml"), "new config").unwrap();
    let result = staging
        .finish(Ok(json!({"entries": [{"metric": "rmse", "value": 1.5}]})))
        .unwrap();
    let installed = local.join("output/newer/reports/evaluation_report_standalone.html");
    assert_eq!(Path::new(result["dir"].as_str().unwrap()), local.as_path());
    assert_eq!(
        Path::new(result["report"].as_str().unwrap()),
        installed.as_path()
    );
    assert!(installed.is_file());
    // 整体替换：旧报告目录、旧图、旧配置一个都不该残留，否则就是两次报告混在一起。
    assert!(!local.join("output/older").exists());
    assert!(!local.join("stale.png").exists());
    assert_eq!(read(local.join("openbench.yaml")), "new config");
    assert_eq!(read(local.join("sim/site.csv")), "new sim");
    assert_eq!(result["entries"][0]["metric"], "rmse");
    assert!(!staged.exists());
    assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
}

#[test]
fn a_fetched_mirror_without_a_report_is_not_installed_as_the_current_one() {
    let base = temp("noreport");
    let local = previous_mirror(&base);
    let staging = FetchStaging::begin(&local).unwrap();
    std::fs::create_dir_all(staging.dir().join("output/newer")).unwrap();
    std::fs::write(staging.dir().join("output/newer/scores.json"), "{}").unwrap();
    let error = staging.finish(Ok(json!({"entries": []}))).unwrap_err();
    assert!(error.contains("取回失败"), "{error}");
    assert!(error.contains("没有找到可显示的报告"), "{error}");
    assert!(error.contains("仍是上次成功版本"), "{error}");
    assert_eq!(read(local.join("openbench.yaml")), "old config");
    assert_eq!(
        crate::bench::find_report(&local.join("output")),
        Some(local.join("output/older/reports/evaluation_report.html"))
    );
    assert!(!local.join("output/newer").exists());
    assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
}

#[test]
fn the_first_fetch_creates_the_mirror_when_there_is_nothing_to_preserve() {
    let base = temp("first");
    let local = mirror(&base);
    let staging = FetchStaging::begin(&local).unwrap();
    staged_report(staging.dir(), "only", "<p>only</p>");
    let result = staging.finish(Ok(json!({"entries": []}))).unwrap();
    assert!(local
        .join("output/only/reports/evaluation_report_standalone.html")
        .is_file());
    assert_eq!(Path::new(result["dir"].as_str().unwrap()), local.as_path());
    assert!(result["report"]
        .as_str()
        .unwrap()
        .starts_with(local.to_string_lossy().as_ref()));
    assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
}

#[test]
fn the_same_report_is_fetched_once_at_a_time_while_other_reports_proceed() {
    let base = temp("concurrent");
    let local = previous_mirror(&base);
    let held = FetchStaging::begin(&local).unwrap();
    let error = FetchStaging::begin(&local).unwrap_err();
    assert!(error.contains("正在取回中"), "{error}");
    // 同一比较名换一台服务器是另一份镜像，不该被一起挡住。
    let other_host = base.join("projects").join("qfo_era5@otherhost");
    let other_host_staging = FetchStaging::begin(&other_host).unwrap();
    drop(other_host_staging);
    let staged = held.dir().to_path_buf();
    drop(held);
    assert!(!staged.exists());
    // 上一次取回结束（成功或失败）后可以立刻重试。
    let retry = FetchStaging::begin(&local).unwrap();
    drop(retry);
}

#[cfg(unix)]
#[test]
fn a_blocked_replacement_leaves_the_old_report_in_place() {
    use std::os::unix::fs::PermissionsExt;

    let base = temp("blocked");
    let local = previous_mirror(&base);
    let staging = FetchStaging::begin(&local).unwrap();
    staged_report(staging.dir(), "newer", "<p>new</p>");
    // projects/ 不可写时，"把旧报告移开"第一步就失败：此时旧报告必须原地不动。
    let projects = local.parent().unwrap().to_path_buf();
    let original = std::fs::metadata(&projects).unwrap().permissions();
    std::fs::set_permissions(&projects, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = staging.finish(Ok(json!({"entries": []})));
    std::fs::set_permissions(&projects, original).unwrap();
    let error = outcome.unwrap_err();
    assert!(error.contains("取回失败"), "{error}");
    assert!(error.contains("无法把本机旧报告移开"), "{error}");
    assert!(error.contains("仍是上次成功版本"), "{error}");
    assert_eq!(
        read(local.join("output/older/reports/evaluation_report.html")),
        "<p>old</p>"
    );
    assert!(local.join("stale.png").is_file());
    assert!(leftovers(&base).is_empty(), "{:?}", leftovers(&base));
}
