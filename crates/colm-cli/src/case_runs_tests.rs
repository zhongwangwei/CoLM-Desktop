use super::*;

fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-case-runs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn runs_are_kept_newest_first_and_capped() {
    let case = temp("history");
    for i in 0..(KEEP + 5) {
        append(&case, &json!({ "started": i })).unwrap();
    }
    let rows = history(&case);
    assert_eq!(rows.len(), KEEP);
    assert_eq!(rows[0]["started"], KEEP + 4);
    assert_eq!(rows.last().unwrap()["started"], 5);
}

#[test]
fn a_modified_versions_run_is_flagged_when_its_source_changed_afterwards() {
    let rows = vec![
        json!({ "version": { "kind": "workspace", "name": "snow", "head": "aaa" } }),
        json!({ "version": { "kind": "workspace", "name": "snow", "head": "bbb" } }),
        json!({ "version": { "kind": "workspace", "name": "gone", "head": "ccc" } }),
        json!({ "version": { "kind": "official" } }),
    ];
    let rows = mark_source_changes(rows, |name| (name == "snow").then(|| "bbb".to_owned()));
    assert_eq!(rows[0]["source_changed"], true);
    assert_eq!(rows[1]["source_changed"], false);
    assert_eq!(
        rows[2]["source_changed"], false,
        "a deleted version is not reported as changed"
    );
    assert!(rows[3].get("source_changed").is_none());
}

#[test]
fn a_case_without_history_still_shows_where_it_came_from() {
    let project = temp("summary");
    let case = project.join("siteA");
    std::fs::create_dir_all(&case).unwrap();
    std::fs::write(case.join(DERIVED_FILE), r#"{"study":"s-1","member":"m3"}"#).unwrap();
    let value = summary(&case, &project.join("ws")).unwrap();
    assert_eq!(value["derived_from"]["member"], "m3");
    assert!(value["runs"].as_array().unwrap().is_empty());
    assert!(value["studies"].as_array().unwrap().is_empty());
}
