use super::*;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-bench-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn every_supported_platform_has_one_pinned_uv_archive() {
    // 这台机器的平台一定在表里（发布要覆盖 macOS、Linux、Windows 的 x86_64 与 aarch64）。
    let (archive, sha) = uv_archive().unwrap();
    assert!(archive.starts_with("uv-"));
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    let mut keys = UV_ARCHIVES
        .iter()
        .map(|(key, _, _)| *key)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), UV_ARCHIVES.len());
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
fn python_style_infinity_and_nan_become_null_outside_strings() {
    let text = r#"[{"max_uparea": Infinity, "low": -Infinity, "x": [NaN, 1], "note": "NaN, Infinity stay"}]"#;
    let value: serde_json::Value = serde_json::from_str(&strict_json(text)).unwrap();
    assert!(value[0]["max_uparea"].is_null());
    assert!(value[0]["low"].is_null());
    assert!(value[0]["x"][0].is_null());
    assert_eq!(value[0]["x"][1], 1);
    assert_eq!(value[0]["note"], "NaN, Infinity stay");
}
