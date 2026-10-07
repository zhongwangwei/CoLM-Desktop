use super::*;

/// 严格模式的要求：顶层是对象，全部属性列入 `required`，不许额外属性；名字唯一；描述非空。
#[test]
fn every_standard_tool_has_a_strict_mode_schema() {
    let registry = Registry::standard();
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
        assert_ne!(
            tool.tier(),
            Tier::Code,
            "{} (no code tools yet)",
            tool.name()
        );
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
