use super::*;

/// 严格模式的要求：顶层是对象，全部属性列入 `required`，不许额外属性；名字唯一；描述非空。
#[test]
fn every_standard_tool_has_a_strict_mode_schema() {
    let registry = Registry::standard_with(true, true);
    let mut names = std::collections::BTreeSet::new();
    for tool in registry.tools() {
        assert!(names.insert(tool.name()), "duplicate tool {}", tool.name());
        assert!(
            tool.name().len() <= 64
                && tool
                    .name()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
        );
        assert!(!tool.description().is_empty());
        let schema = tool.parameters();
        assert_eq!(schema["type"], "object", "{}", tool.name());
        assert_eq!(schema["additionalProperties"], false, "{}", tool.name());
        let properties: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(properties.len(), required.len(), "{}", tool.name());
        for property in properties {
            assert!(
                required.contains(&property.as_str()),
                "{} {property}",
                tool.name()
            );
        }
        // C 级只有开发工作区里的这几个（第 4 节）：别的工具不该悄悄变成代码操作。
        if tool.tier() == Tier::Code {
            assert!(
                [
                    "workspace_create",
                    "apply_patch",
                    "revert",
                    "build_engine",
                    "build_kernel",
                    "run_tests",
                    "run_case_with",
                    "parity_check",
                    "regression_check",
                ]
                .contains(&tool.name()),
                "{} is a code-tier tool but not a workspace tool",
                tool.name()
            );
        }
    }
    let api = registry.api_tools(true);
    assert_eq!(api.len(), names.len());
    assert_eq!(api[0]["type"], "function");
    assert_eq!(api[0]["function"]["strict"], true);
    assert!(registry.api_tools(false)[0]["function"]
        .get("strict")
        .is_none());
}

#[test]
fn long_results_are_truncated_with_a_note() {
    let small = json!({"a": 1});
    assert_eq!(result_text(&small), r#"{"a":1}"#);
    let big = json!({ "text": "中".repeat(MAX_RESULT_CHARS) });
    let text = result_text(&big);
    assert!(text.contains("[truncated"));
    assert!(text.chars().count() < MAX_RESULT_CHARS + 200);
}

#[test]
fn string_nulls_from_the_model_count_as_missing() {
    let args = serde_json::json!({ "name": "null", "start": " ", "end": "None", "mode": "pc" });
    assert_eq!(opt_str(&args, "name"), None);
    assert_eq!(opt_str(&args, "start"), None);
    assert_eq!(opt_str(&args, "end"), None);
    assert_eq!(opt_str(&args, "missing"), None);
    assert_eq!(opt_str(&args, "mode"), Some("pc"));
}

#[test]
fn driving_the_window_replaces_background_case_creation() {
    let names = |registry: &Registry| -> Vec<&str> { registry.tools().map(|t| t.name()).collect() };
    let plain = Registry::standard_with(false, false);
    assert!(names(&plain).contains(&"create_case"));
    assert!(!names(&plain).contains(&"ui_state"));
    let window = Registry::standard_with(true, true);
    assert!(!names(&window).contains(&"create_case"));
    for tool in [
        "ui_state",
        "ui_commit",
        "web_search",
        "run_case",
        "set_case_fields",
    ] {
        assert!(names(&window).contains(&tool), "{tool}");
    }
}
