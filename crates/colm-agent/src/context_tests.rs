use super::*;
use crate::message::ToolCall;

fn tool(id: &str, content: String) -> Message {
    Message::Tool {
        tool_call_id: id.into(),
        content,
    }
}

#[test]
fn short_history_is_left_alone() {
    let history = vec![Message::User {
        content: "hello".into(),
    }];
    assert!(prepare(&history).is_none());
    assert_eq!(estimate_tokens(&history), 21);
}

#[test]
fn long_history_keeps_recent_tool_results_and_marks_old_ones() {
    let mut history = vec![Message::System {
        content: "system".into(),
    }];
    for index in 0..12 {
        history.push(Message::Assistant {
            content: String::new(),
            reasoning_content: None,
            provider_state: None,
            tool_calls: vec![ToolCall {
                id: format!("call-{index}"),
                name: "read".into(),
                arguments: "{}".into(),
            }],
        });
        history.push(tool(
            &format!("call-{index}"),
            "x".repeat(CONTEXT_COMPACT_TOKENS as usize / 8),
        ));
    }
    let prepared = prepare(&history).expect("large history must be compacted");
    assert!(prepared.compressed_tools > 0);
    assert_eq!(prepared.messages.len(), history.len());
    let tool_ids: Vec<_> = prepared
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::Tool { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(tool_ids.len(), 12);
    assert_eq!(tool_ids[0], "call-0");
    assert!(prepared.messages.iter().any(|message| matches!(
        message,
        Message::Tool { content, .. } if content.contains("older tool result compressed")
    )));
    // 最近一组工具结果仍然完整。
    assert!(prepared.messages.iter().any(|message| matches!(
        message,
        Message::Tool { tool_call_id, content }
            if tool_call_id == "call-11" && content.starts_with('x') && content.len() > 100
    )));
    assert!(estimate_tokens(&prepared.messages) < CONTEXT_COMPACT_TOKENS);
}
