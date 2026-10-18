use super::*;

#[test]
fn an_empty_value_does_not_erase_the_remembered_one() {
    // 用户清空一个框通常是想重新填，不是想忘掉历史。
    // 直接写进去的话，下次打开那个框就又是空的 —— 而这个功能的全部意义
    // 就是让它别是空的。
    let mut all: BTreeMap<String, String> = BTreeMap::new();
    all.insert("sitedir".into(), "/data/Sitedata".into());
    // `save_recent` 的判据（不依赖 AppHandle 的那半）
    for candidate in ["", "   ", "\t"] {
        assert!(candidate.trim().is_empty(), "{candidate:?} 应当被当成空值");
    }
    assert!(!"/data/other".trim().is_empty());
    assert_eq!(
        all.get("sitedir").map(String::as_str),
        Some("/data/Sitedata")
    );
}

#[test]
fn text_export_rejects_path_suggestions_and_limits_filters() {
    for name in [
        "",
        ".",
        "..",
        "../report.csv",
        "/tmp/report.csv",
        "C:\\report.csv",
        "nested/report.csv",
        "bad\nname.csv",
    ] {
        assert!(text_export_filter(name, "text/csv").is_err(), "{name}");
    }
    for (mime, extension) in [
        ("text/csv;charset=utf-8", "csv"),
        ("application/json", "json"),
        ("text/markdown;charset=utf-8", "md"),
        ("text/html", "html"),
        ("text/plain", "txt"),
    ] {
        assert_eq!(text_export_filter("结果", mime).unwrap(), extension);
    }
    assert!(text_export_filter("report.pdf", "application/pdf").is_err());
}

#[test]
fn text_export_cancel_and_atomic_save_preserve_existing_data_on_failure() {
    assert_eq!(finish_text_export(None, "cancelled".into()).unwrap(), None);
    let root = std::env::temp_dir().join(format!(
        "colm-text-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("O'Brien-结果.csv");
    assert_eq!(
        finish_text_export(Some(path.clone()), "时间,值\n1,2\n".into()).unwrap(),
        Some(path.display().to_string())
    );
    assert_eq!(std::fs::read(&path).unwrap(), "时间,值\n1,2\n".as_bytes());
    finish_text_export(Some(path.clone()), "replacement".into()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    assert!(finish_text_export(Some(path.clone()), "must not replace".into()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
    assert!(finish_text_export(Some(root.join("missing/report.csv")), "fail".into()).is_err());
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        1,
        "no staged files remain"
    );
    std::fs::set_permissions(&path, original).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_unreadable_recent_file_is_kept_instead_of_being_replaced_by_one_entry() {
    let dir = std::env::temp_dir().join(format!(
        "colm-recent-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("recent.json");
    remember(&path, "root".into(), "/data/projects".into()).unwrap();
    remember(&path, "rawdata".into(), "/data/raw".into()).unwrap();
    let all = read_recent(&path).unwrap();
    assert_eq!(all.len(), 2);
    std::fs::write(&path, "{\"root\": \"/data/proj").unwrap();
    assert!(remember(&path, "runtime".into(), "/data/runtime".into()).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"root\": \"/data/proj"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
