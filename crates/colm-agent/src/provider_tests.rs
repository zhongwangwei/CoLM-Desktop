use super::*;

#[test]
fn streamed_tool_call_fragments_are_joined_by_index() {
    let stream = concat!(
        ": keep-alive\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"check \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"metrics\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Looking\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"metrics\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"case\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"type\":\"function\",\"function\":{\"name\":\"run_status\",\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"/p/x\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":30}}\n\n",
        "data: [DONE]\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ignored after DONE\"}}]}\n\n",
    );
    let mut assembler = StreamAssembler::default();
    let mut events = Vec::new();
    read_sse(stream.as_bytes(), |data| {
        events.extend(assembler.push(&serde_json::from_str(data)?));
        Ok(())
    })
    .unwrap();
    let turn = assembler.finish();
    assert_eq!(turn.content, "Looking");
    assert_eq!(turn.reasoning.as_deref(), Some("check metrics"));
    assert_eq!(turn.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(
        turn.usage,
        Usage {
            prompt_tokens: 120,
            completion_tokens: 30
        }
    );
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[0].id, "call_a");
    assert_eq!(turn.tool_calls[0].name, "metrics");
    assert_eq!(turn.tool_calls[0].arguments, r#"{"case":"/p/x"}"#);
    assert_eq!(turn.tool_calls[1].name, "run_status");
    assert_eq!(
        events,
        vec![
            StreamEvent::Reasoning("check ".into()),
            StreamEvent::Reasoning("metrics".into()),
            StreamEvent::Content("Looking".into()),
        ]
    );
    // 下一轮回传的助手消息带着推理内容。
    let api = turn.message().to_api();
    assert_eq!(api["reasoning_content"], "check metrics");
}

#[test]
fn request_bodies_stream_with_usage_and_only_send_thinking_when_set() {
    let mut config = ProviderConfig::deepseek("deepseek-flash", "k".into());
    let messages = [Message::User {
        content: "hi".into(),
    }];
    let body = request_body(&config, &messages, &[]);
    assert_eq!(body["model"], "deepseek-flash");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert!(body.get("tools").is_none());
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    config.reasoning_effort = Some("max".into());
    assert_eq!(
        request_body(&config, &messages, &[])["reasoning_effort"],
        "max"
    );
    config.thinking = Some(false);
    let tools = [json!({"type": "function", "function": {"name": "x"}})];
    let body = request_body(&config, &messages, &tools);
    assert_eq!(body["thinking"]["type"], "disabled");
    // 关了思考就不发强度。
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["tools"][0]["function"]["name"], "x");
    // Key 不进序列化的配置。
    assert!(!serde_json::to_string(&config).unwrap().contains("\"k\""));
}

#[test]
fn a_call_without_arguments_gets_an_empty_object() {
    let mut assembler = StreamAssembler::default();
    assembler.push(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"list_cases"}}]}}]}));
    let turn = assembler.finish();
    assert_eq!(turn.tool_calls[0].arguments, "{}");
}
