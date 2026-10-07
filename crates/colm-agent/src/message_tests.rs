use super::*;

#[test]
fn assistant_messages_keep_reasoning_and_tool_calls_for_the_next_turn() {
    let message = Message::Assistant {
        content: String::new(),
        reasoning_content: Some("look at the metrics first".into()),
        tool_calls: vec![ToolCall {
            id: "call_1".into(),
            name: "metrics".into(),
            arguments: r#"{"case":"/p/CA-Qfo"}"#.into(),
        }],
    };
    let api = message.to_api();
    assert_eq!(api["role"], "assistant");
    assert_eq!(api["reasoning_content"], "look at the metrics first");
    assert_eq!(api["tool_calls"][0]["type"], "function");
    assert_eq!(api["tool_calls"][0]["function"]["name"], "metrics");
    // 参数原样回传，不重新序列化。
    assert_eq!(
        api["tool_calls"][0]["function"]["arguments"],
        r#"{"case":"/p/CA-Qfo"}"#
    );

    let plain = Message::Assistant {
        content: "done".into(),
        reasoning_content: None,
        tool_calls: Vec::new(),
    }
    .to_api();
    assert!(plain.get("reasoning_content").is_none());
    assert!(plain.get("tool_calls").is_none());

    let tool = Message::Tool {
        tool_call_id: "call_1".into(),
        content: "{}".into(),
    }
    .to_api();
    assert_eq!(tool["role"], "tool");
    assert_eq!(tool["tool_call_id"], "call_1");
}

#[test]
fn messages_round_trip_through_the_transcript_format() {
    let messages = vec![
        Message::System {
            content: "s".into(),
        },
        Message::User {
            content: "u".into(),
        },
        Message::Assistant {
            content: "a".into(),
            reasoning_content: Some("r".into()),
            tool_calls: vec![ToolCall {
                id: "1".into(),
                name: "t".into(),
                arguments: "{}".into(),
            }],
        },
        Message::Tool {
            tool_call_id: "1".into(),
            content: "x".into(),
        },
    ];
    let text = serde_json::to_string(&messages).unwrap();
    let back: Vec<Message> = serde_json::from_str(&text).unwrap();
    assert_eq!(back, messages);
}
