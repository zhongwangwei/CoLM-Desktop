use super::*;

fn temp(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-agent-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn case(root: &Path, name: &str, body: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), format!("&nl_colm\n{body}/\n")).unwrap();
    dir
}

#[test]
fn months_follow_the_civil_calendar() {
    assert_eq!(year_month(0), (1970, 1));
    assert_eq!(year_month(1_041_381_000), (2003, 1)); // 2003-01-01 00:30 UTC
    assert_eq!(year_month(951_782_400), (2000, 2)); // 2000-02-29
    assert_eq!(year_month(-1), (1969, 12));
}

#[test]
fn series_summaries_skip_missing_values() {
    let jan = 1_041_381_000;
    let feb = jan + 31 * 86_400;
    let stats = summarize(&[jan, jan + 3600, feb], &[Some(1.0), None, Some(4.0)]);
    assert_eq!(stats["count"], 3);
    assert_eq!(stats["nan_or_missing"], 1);
    assert_eq!(stats["mean"], 2.5);
    assert_eq!(stats["min"], 1.0);
    assert_eq!(stats["max"], 4.0);
    assert_eq!(stats["monthly_mean"]["2003-01"], 1.0);
    assert_eq!(stats["monthly_mean"]["2003-02"], 4.0);
}

#[test]
fn cases_are_listed_read_and_compared() {
    let root = temp("cases");
    let a = case(
        &root,
        "A",
        " DEF_USE_PC = .true.\n DEF_CASE_NAME = 'A'\n DEF_PFT_VMAX25(3) = 41.0\n",
    );
    let b = case(
        &root.join("group"),
        "B",
        " DEF_CASE_NAME = 'B'\n DEF_PFT_VMAX25(3) = 20.0\n",
    );
    std::fs::write(b.join("hybrid.toml"), "").unwrap();
    std::fs::create_dir_all(root.join(".colm/studies/x")).unwrap();
    case(&root.join(".colm/studies/x"), "hidden", "");
    let ctx = ToolContext {
        project_root: root.clone(),
        ..ToolContext::default()
    };

    let listed = ListCases.call(&json!({ "root": null }), &ctx).unwrap();
    let cases = listed["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2, "{listed}");
    assert_eq!(cases[0]["mode"], "pc");
    assert_eq!(cases[1]["mode"], "lct");
    assert_eq!(cases[1]["has_hybrid"], true);

    let config = ReadCaseConfig
        .call(
            &json!({ "case": "A", "names": null, "filter": "vmax" }),
            &ctx,
        )
        .unwrap();
    let fields = &config["fields"]["case.nml"];
    assert_eq!(fields.as_object().unwrap().len(), 1, "{config}");
    assert!(fields
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .contains("VMAX25"));
    assert!(ReadCaseConfig
        .call(
            &json!({ "case": "nowhere", "names": null, "filter": null }),
            &ctx
        )
        .is_err());

    let diff = CompareCases
        .call(
            &json!({ "a": a.display().to_string(), "b": b.display().to_string() }),
            &ctx,
        )
        .unwrap();
    let differing = diff["differing_fields"].as_object().unwrap();
    assert!(differing.keys().any(|k| k.contains("VMAX25")), "{diff}");
    assert!(differing.keys().any(|k| k == "DEF_USE_PC"), "{diff}");
    // 算例名属于路径类字段，不算实质差异。
    assert!(!differing.contains_key("DEF_CASE_NAME"));
    assert_eq!(diff["hybrid"], json!([false, true]));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parameters_are_explained_from_the_catalog() {
    let ctx = ToolContext::default();
    let explained = ExplainParameter
        .call(&json!({ "name": "DEF_USE_PC", "case": null }), &ctx)
        .unwrap();
    assert!(explained["field"]["name"] == "DEF_USE_PC" || !explained["parameter"].is_null());
    assert!(ExplainParameter
        .call(&json!({ "name": "NOT_A_FIELD_AT_ALL", "case": null }), &ctx)
        .is_err());
}

#[test]
fn docs_are_searched_line_by_line() {
    let root = temp("docs");
    std::fs::write(root.join("a.md"), "intro\nVcmax compensates\nend\n").unwrap();
    std::fs::write(root.join("b.txt"), "Vcmax in a non-markdown file\n").unwrap();
    let ctx = ToolContext {
        docs_root: Some(root.clone()),
        ..ToolContext::default()
    };
    let found = SearchDocs.call(&json!({ "query": "vcmax" }), &ctx).unwrap();
    let matches = found["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["file"], "a.md");
    assert_eq!(matches[0]["line"], 2);
    let _ = std::fs::remove_dir_all(&root);
}
