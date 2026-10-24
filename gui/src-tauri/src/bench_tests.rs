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
