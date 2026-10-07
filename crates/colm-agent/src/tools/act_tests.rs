use super::*;

fn temp_case(label: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-agent-act-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), format!("&nl_colm\n{body}/\n")).unwrap();
    dir
}

#[test]
fn values_are_typed_by_the_field_definitions() {
    let (value, group) = typed_value("DEF_USE_PC", "true").unwrap();
    assert_eq!(value, colm_namelist::Value::Bool(true));
    assert_eq!(group, Some("nl_colm"));
    assert_eq!(
        typed_value("DEF_USE_PC", ".FALSE.").unwrap().0,
        colm_namelist::Value::Bool(false)
    );
    assert!(typed_value("DEF_USE_PC", "maybe").is_err());
    assert!(typed_value("NOT_A_FIELD", "1").is_err());
    let (pft, _) = typed_value("DEF_PFT_VMAX25(3)", "41.5").unwrap();
    assert_eq!(pft.as_f64(), Some(41.5));
    assert!(typed_value("DEF_PFT_VMAX25(3)", "abc").is_err());
}

#[test]
fn fields_are_set_in_place_or_inserted_and_bad_ones_change_nothing() {
    let dir = temp_case("fields", " DEF_CASE_NAME = 'x'\n DEF_USE_PC = .false.\n");
    let ctx = ToolContext {
        project_root: dir.clone(),
        ..ToolContext::default()
    };
    let result = SetCaseFields
        .call(
            &json!({ "case": dir.display().to_string(), "fields": [
                { "name": "DEF_USE_PC", "value": "true" },
                { "name": "DEF_PFT_VMAX25(3)", "value": "40" },
            ]}),
            &ctx,
        )
        .unwrap();
    assert_eq!(result["changed"][0]["old"], ".false.");
    let text = std::fs::read_to_string(dir.join("case.nml")).unwrap();
    let document = colm_namelist::parse(&text).unwrap();
    assert_eq!(
        document.get("DEF_USE_PC"),
        Some(&colm_namelist::Value::Bool(true))
    );
    assert_eq!(
        document.get("DEF_PFT_VMAX25(3)").and_then(|v| v.as_f64()),
        Some(40.0)
    );
    // 第二个字段不合法时整份不写（先全部校验、最后才写盘）。
    let before = std::fs::read_to_string(dir.join("case.nml")).unwrap();
    assert!(SetCaseFields
        .call(
            &json!({ "case": dir.display().to_string(), "fields": [
                { "name": "DEF_USE_PC", "value": "false" },
                { "name": "DEF_USE_PC_TYPO", "value": "1" },
            ]}),
            &ctx,
        )
        .is_err());
    assert_eq!(
        std::fs::read_to_string(dir.join("case.nml")).unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn acting_tools_need_approval_and_describe_the_real_action() {
    for tool in tools() {
        if tool.name() == "scan_sites" {
            assert_eq!(tool.tier(), Tier::Read);
        } else {
            assert_eq!(tool.tier(), Tier::Act, "{}", tool.name());
        }
    }
    let summary = CreateCase.summary(&json!({
        "site": "/d/CA-Qfo_site.nc", "out": "/p/CA-Qfo", "name": null, "start": "2004-01-01",
        "end": "2005-12-31", "met": null, "mode": "pc", "spinup_years": 1, "spinup_repeat": 1,
    }));
    assert!(
        summary.contains("/p/CA-Qfo") && summary.contains("pc") && summary.contains("2004-01-01"),
        "{summary}"
    );
    assert!(RunCase
        .summary(&json!({ "case": "/p/x", "stage": null, "engine": null, "force": true }))
        .contains("强制重跑"));
}

#[test]
fn running_needs_a_kernel_and_a_real_case() {
    let ctx = ToolContext::default();
    let dir = temp_case("run", "");
    let error = RunCase
        .call(&json!({ "case": dir.display().to_string(), "stage": null, "engine": null, "force": false }), &ctx)
        .unwrap_err();
    assert!(error.to_string().contains("no kernel is selected"));
    assert!(RunCase
        .call(
            &json!({ "case": "/nowhere", "stage": null, "engine": null, "force": false }),
            &ctx
        )
        .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
