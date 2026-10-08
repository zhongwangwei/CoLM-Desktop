use super::*;

fn line(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// 第 634 轮在 Claude Code 2.1.293 上录下的消息（略去无关字段）。
#[test]
fn stream_json_becomes_panel_events() {
    let mut state = ClaudeState::default();
    let init = line(
        r#"{"type":"system","subtype":"init","session_id":"4844562a-80d3-4f2e-9901-649fea21b2f0","apiKeySource":"none"}"#,
    );
    assert!(map_event(&init, &mut state).is_empty());
    assert_eq!(
        state.session_id.as_deref(),
        Some("4844562a-80d3-4f2e-9901-649fea21b2f0")
    );

    let text = line(
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"4"}}}"#,
    );
    assert_eq!(
        map_event(&text, &mut state),
        [Outbound::AssistantDelta { text: "4".into() }]
    );
    let thinking = line(
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}}"#,
    );
    assert_eq!(
        map_event(&thinking, &mut state),
        [Outbound::ReasoningDelta { text: "hm".into() }]
    );

    // Claude Code 自己的工具出卡片；colm 的工具与 ToolSearch 不出（转发层出 / 内部步骤）。
    let uses = line(
        r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","id":"toolu_1","name":"Write","input":{"file_path":"/w/hello.txt","content":"hi"}},
        {"type":"tool_use","id":"toolu_2","name":"mcp__colm__list_cases","input":{"root":"/d"}},
        {"type":"tool_use","id":"toolu_3","name":"ToolSearch","input":{"query":"select:x"}}]}}"#,
    );
    let events = map_event(&uses, &mut state);
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], Outbound::ToolCall { id, name, tier: Tier::Code, summary, .. }
        if id == "toolu_1" && name == "Write" && summary == "写文件：/w/hello.txt")
    );

    let results = line(
        r#"{"type":"user","message":{"content":[
        {"type":"tool_result","tool_use_id":"toolu_1","content":"File created successfully"},
        {"type":"tool_result","tool_use_id":"toolu_2","content":[{"type":"text","text":"{}"}]}]}}"#,
    );
    let events = map_event(&results, &mut state);
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], Outbound::ToolResult { id, ok: true, result, .. }
        if id == "toolu_1" && result == "File created successfully")
    );

    let result = line(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"4","usage":{"input_tokens":6,"cache_creation_input_tokens":100,"cache_read_input_tokens":900,"output_tokens":42}}"#,
    );
    map_event(&result, &mut state);
    assert_eq!(state.result, Some((true, "4".to_owned())));
    assert_eq!(state.usage.prompt_tokens, 1006);
    assert_eq!(state.usage.completion_tokens, 42);

    let mut failed = ClaudeState::default();
    map_event(
        &line(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#),
        &mut failed,
    );
    assert_eq!(failed.result, Some((false, "error_max_turns".to_owned())));
}

#[test]
fn permission_answers_and_session_ids_have_the_expected_shape() {
    let input = json!({ "file_path": "/w/a", "content": "x" });
    let allow: Value = serde_json::from_str(&permission_answer(true, &input, None)).unwrap();
    assert_eq!(allow, json!({ "behavior": "allow", "updatedInput": input }));
    let deny: Value = serde_json::from_str(&permission_answer(false, &input, Some("no"))).unwrap();
    assert_eq!(deny, json!({ "behavior": "deny", "message": "no" }));
    let id = new_session_id();
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(
        parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12]
    );
    assert!(parts[2].starts_with('4'));
    assert!("89ab".contains(&parts[3][..1]));
    assert_ne!(new_session_id(), id);
    assert_eq!(summary("Bash", &json!({ "command": "ls" })), "执行命令：ls");
}
