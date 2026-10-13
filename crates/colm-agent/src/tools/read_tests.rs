use super::*;

#[cfg(unix)]
#[test]
fn selected_model_configuration_refreshes_after_edits_without_redirecting_run_status() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("effective-model");
    let original = case(&root, "A", " DEF_USE_PFT=.false.\n SITE_lat=1.0\n");
    std::fs::write(original.join("run.out"), "original run log").unwrap();
    let cli = root.join("cli");
    std::fs::write(
        &cli,
        r#"#!/bin/sh
[ "$1" = model-preview-context ] || exit 1
shift
while [ "$#" -gt 0 ]; do
 case "$1" in
 --model-root|--model-id|--model-revision|--operation-id) shift ;;
 --out) out="$2"; shift ;;
 *) src="$1" ;;
 esac
 shift
done
mkdir -p "$out"
sed 's/DEF_USE_PFT=.false./DEF_USE_PFT=.true./' "$src/case.nml" > "$out/case.nml"
printf '{"effective_case":"%s","run":{"version":{"id":"modified","revision":1}}}\n' "$out"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        project_root: root.clone(),
        cli,
        model_context: Some(crate::tools::ModelContext {
            model: json!({"version":{"id":"modified","revision":1},"code":{"source_sha256":"a".repeat(64)},"overlay":{"parameters":[]}}),
            model_root: root.join("models"),
            source_dir: root.join("source"),
            docs_dir: None,
            source_case: Some(original.clone()),
        }),
        ..ToolContext::default()
    };
    let read = || {
        ReadCaseConfig
            .call(&json!({"case":"A","names":null,"filter":null}), &ctx)
            .unwrap()
    };
    assert_eq!(read()["fields"]["case.nml"]["DEF_USE_PFT"], ".true.");
    std::fs::write(
        original.join("case.nml"),
        "&nl_colm\n DEF_USE_PFT=.false.\n SITE_lat=42.0\n/\n",
    )
    .unwrap();
    let fresh = read();
    assert_eq!(
        fresh["fields"]["case.nml"]["SITE_lat"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        42.0
    );
    assert_eq!(fresh["model_version"]["revision"], 1);
    assert!(std::fs::read_to_string(original.join("case.nml"))
        .unwrap()
        .contains("DEF_USE_PFT=.false."));
    let status = RunStatus.call(&json!({"case":"A"}), &ctx).unwrap();
    assert_eq!(status["log_tails"]["run.out"], "original run log");
    assert_eq!(
        std::fs::read_dir(root.join("models/assistant-previews"))
            .unwrap()
            .count(),
        0
    );
    let _ = std::fs::remove_dir_all(root);
}

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
        cli: root.join("missing-cli"),
        ..ToolContext::default()
    };
    let found = SearchDocs.call(&json!({ "query": "vcmax" }), &ctx).unwrap();
    let matches = found["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["file"], "a.md");
    assert_eq!(matches[0]["line"], 2);
    assert_eq!(matches[0]["context_start_line"], 1);
    assert_eq!(
        matches[0]["context"],
        json!(["intro", "Vcmax compensates", "end"])
    );
    assert_eq!(matches[0]["path"], root.join("a.md").display().to_string());
    assert_eq!(
        found["provenance"]["application_source"]["version_status"],
        "unknown"
    );
    assert!(found["provenance"]["application_source"]["revision"].is_null());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parameter_activity_uses_selected_kernel_and_case_guards() {
    let root = temp("parameter-activity");
    let dir = case(
        &root,
        "A",
        " SITE_landtype=1\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.true.\n",
    );
    let kernel = root.join("kernel");
    std::fs::create_dir(&kernel).unwrap();
    std::fs::write(
        kernel.join("manifest.json"),
        r#"{"schema":1,"macros":["SinglePoint","LULC_IGBP"]}"#,
    )
    .unwrap();
    let ctx = ToolContext {
        project_root: root.clone(),
        kernel_dir: Some(kernel),
        ..ToolContext::default()
    };
    let explain = |name| {
        ExplainParameter
            .call(&json!({"name":name,"case":"A"}), &ctx)
            .unwrap()
    };
    assert_eq!(
        explain("DEF_WUE_LAMBDA")["applicability"]["status"],
        "active"
    );
    assert_eq!(
        explain("DEF_BALL_BERRY_GRADM")["applicability"]["status"],
        "inactive"
    );
    assert_eq!(
        explain("DEF_BALL_BERRY_GRADM")["applicability"]["calibration_eligible_for_case"],
        false
    );
    assert_eq!(explain("DEF_USE_PC")["applicability"]["status"], "unknown");
    let without_kernel = ToolContext {
        project_root: root.clone(),
        ..ToolContext::default()
    };
    assert_eq!(
        ExplainParameter
            .call(
                &json!({"name":"DEF_WUE_LAMBDA","case":"A"}),
                &without_kernel
            )
            .unwrap()["applicability"]["status"],
        "unknown"
    );
    std::fs::write(dir.join("case.nml"), "&nl_colm\n DEF_USE_WUEST=.true.\n/\n").unwrap();
    assert_eq!(
        explain("DEF_WUE_LAMBDA")["applicability"]["status"],
        "unknown"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn docs_version_is_reported_by_application_source_not_project_git() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("docs-version");
    let cli = root.join("cli");
    std::fs::write(&cli, "#!/bin/sh\n[ \"$*\" = 'ws-source-info --source app' ] || exit 1\nprintf '%s\\n' '{\"source\":\"/application/source\",\"revision\":\"app-sha\",\"tag\":\"v1\",\"dirty\":true,\"version_status\":\"identified\"}'\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        root.join("guide.md"),
        "energy closure\nphase change matters\n",
    )
    .unwrap();
    let ctx = ToolContext {
        cli,
        docs_root: Some(root.clone()),
        project_root: root.join("unrelated-project"),
        ..ToolContext::default()
    };
    let result = SearchDocs.call(&json!({"query":"closure"}), &ctx).unwrap();
    let source = &result["provenance"]["application_source"];
    assert_eq!(source["revision"], "app-sha");
    assert_eq!(source["dirty"], true);
    assert_eq!(source["source"], "/application/source");
    assert_eq!(result["matches"][0]["context"][1], "phase change matters");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn large_document_search_preserves_valid_json_and_explicit_truncation() {
    let root = temp("docs-budget");
    std::fs::write(
        root.join("guide.md"),
        ("soil ".to_owned() + &"quoted \\\" text ".repeat(40) + "\n").repeat(80),
    )
    .unwrap();
    let ctx = ToolContext {
        docs_root: Some(root.clone()),
        ..ToolContext::default()
    };
    let result = SearchDocs.call(&json!({"query":"soil"}), &ctx).unwrap();
    assert_eq!(result["truncated"], true);
    assert!(!result["matches"].as_array().unwrap().is_empty());
    let text = super::super::result_text(&result);
    assert!(text.chars().count() <= MAX_RESULT_CHARS);
    assert!(serde_json::from_str::<Value>(&text).is_ok());
    assert!(SearchDocs
        .call(&json!({"query":"x".repeat(1025)}), &ctx)
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn guessed_spinup_parameter_is_rejected_with_real_controls() {
    let error = ExplainParameter
        .call(
            &json!({"name":"DEF_SPINUP_YEARS","case":null}),
            &ToolContext::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("neither a namelist field"));
    assert!(error.contains("read_case_config"));
    assert!(error.contains("cutoff year, not a duration"));
    assert!(error.contains("visible case settings returned by ui_state"));
    let actual = ExplainParameter
        .call(
            &json!({"name":"DEF_simulation_time%spinup_year","case":null}),
            &ToolContext::default(),
        )
        .unwrap();
    assert_eq!(actual["field"]["name"], "DEF_simulation_time%spinup_year");
}

#[cfg(unix)]
#[test]
fn knowledge_search_checks_the_selected_workspace_and_withholds_unknown_cards() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("knowledge-workspace");
    let source = root.join("workspace-source");
    let docs = source.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(source.join("crates/core")).unwrap();
    let path = "crates/core/flux.rs";
    let text = "# energy\nOld formula in `crates/core/flux.rs`\n";
    std::fs::write(docs.join("colm-process-knowledge.md"), text).unwrap();
    std::fs::write(source.join(path), "fn energy() { 2 }\n").unwrap();
    let manifest = json!({"schema":1,"bound_at_unix_ms":1,"documents":{"colm-process-knowledge.md":{
        "document_sha256":knowledge::digest(text.as_bytes()),"sources":{path:knowledge::digest(b"fn energy() { 1 }\n")}}}});
    std::fs::write(docs.join("knowledge-sources.json"), manifest.to_string()).unwrap();
    let cli = root.join("cli");
    let info = json!({"source":source,"revision":"edited","dirty":true});
    std::fs::write(&cli, format!("#!/bin/sh\n[ \"$1\" = ws-source-info ] && [ \"$2\" = --name ] && [ \"$3\" = experiment ] || exit 1\nprintf '%s\\n' '{}'\n", info)).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        cli,
        docs_root: Some(docs.clone()),
        ..Default::default()
    };
    let result = SearchDocs
        .call(&json!({"query":"energy","name":"experiment"}), &ctx)
        .unwrap();
    assert_eq!(result["scope"]["kind"], "workspace");
    assert_eq!(
        result["knowledge_checks"]["colm-process-knowledge.md"]["status"],
        "needs_review"
    );
    assert!(!result["matches"][0]["context"]
        .to_string()
        .contains("Old formula"));
    assert_eq!(
        result["knowledge_checks"]["colm-process-knowledge.md"]["source_checks"][0]["next_tool"]
            ["arguments"]["name"],
        "experiment"
    );
    std::fs::write(docs.join("knowledge-sources.json"), "broken").unwrap();
    let unknown = SearchDocs
        .call(&json!({"query":"energy","name":"experiment"}), &ctx)
        .unwrap();
    assert_eq!(unknown["matches"][0]["knowledge_status"], "unknown");
    assert!(!unknown["matches"][0]["context"]
        .to_string()
        .contains("Old formula"));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn hybrid_checks_dispatch_original_case_to_selected_frozen_runtime() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("frozen-hybrid");
    let original = case(&root, "A", " DEF_USE_PFT=.false.\n");
    let before = std::fs::read(original.join("case.nml")).unwrap();
    let cli = root.join("cli");
    std::fs::write(
        &cli,
        r#"#!/bin/sh
[ "$1" = model-hybrid-check ] || exit 1
printf '{"argv":"%s"}\n' "$*"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ctx = ToolContext {
        cli,
        project_root: root.clone(),
        kernel_dir: Some(root.join("actual-kernel")),
        preprocessors: Some("fortran".into()),
        model_context: Some(crate::tools::ModelContext {
            model: json!({"version":{"id":"old-workspace","revision":1},"code":{"source_sha256":"b".repeat(64)}}),
            model_root: root.join("models"),
            source_dir: root.join("frozen-source"),
            docs_dir: None,
            source_case: Some(original.clone()),
        }),
        ..ToolContext::default()
    };
    let result = HybridCheck.call(&json!({"case":"A"}), &ctx).unwrap();
    let argv = result["argv"].as_str().unwrap();
    assert!(argv.starts_with("model-hybrid-check --model-root "));
    assert!(argv.contains("--model-id old-workspace --model-revision 1"));
    assert!(argv.contains(&format!(
        "{} --kernel {}",
        original.display(),
        root.join("actual-kernel").display()
    )));
    assert!(argv.ends_with("--preprocessors fortran"));
    assert_eq!(std::fs::read(original.join("case.nml")).unwrap(), before);
    assert!(
        !root.join("models/assistant-previews").exists(),
        "fixed CLI owns its isolated preview"
    );
    std::fs::remove_dir_all(root).unwrap();
}
