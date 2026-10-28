use super::*;
use crate::message::ToolCall;
use serde_json::{json, Value};

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

#[test]
fn safety_margin_is_two_percent_clamped_between_one_and_sixteen_thousand() {
    // 2% 太小（<1k）时取 1k，太大（>16k）时封顶 16k：1M 窗口下 20k 的 2% 不必全留。
    assert_eq!(safety_margin(1_000), 1_024);
    assert_eq!(safety_margin(51_200), 1_024);
    assert_eq!(safety_margin(100_000), 2_000);
    assert_eq!(safety_margin(819_200), 16_384);
    assert_eq!(safety_margin(1_000_000), 16_384);
    assert_eq!(safety_margin(10_000_000), 16_384);
}

#[test]
fn window_source_serializes_as_the_documented_snake_case_names() {
    assert_eq!(
        serde_json::to_string(&WindowSource::User).unwrap(),
        "\"user\""
    );
    assert_eq!(
        serde_json::to_string(&WindowSource::Preset).unwrap(),
        "\"preset\""
    );
    assert_eq!(WindowSource::User.as_str(), "user");
    assert_eq!(WindowSource::Preset.as_str(), "preset");
}

#[test]
fn body_estimate_counts_compact_json_characters_plus_a_fixed_protocol_margin() {
    let body = json!({"messages":[{"role":"user","content":"中文"}],"tools":[]});
    assert_eq!(
        estimate_body_tokens(&body),
        body.to_string().chars().count() as u64 + BODY_PROTOCOL_MARGIN_TOKENS
    );
    // 中文按字符计而不是字节计：1 字符保守按 1 token 上界，不能因为 UTF-8 字节多就虚增 3 倍。
    let chinese = json!({"text":"中文中文中文"});
    let english = json!({"text":"abcdefghijklmnopqrstuvwxyz"});
    assert!(estimate_body_tokens(&chinese) < estimate_body_tokens(&english));
}

fn history_with_tool_results(count: usize, content_chars: usize) -> Vec<Message> {
    let mut history = vec![Message::User {
        content: "inspect the case".into(),
    }];
    for index in 0..count {
        history.push(Message::Assistant {
            content: String::new(),
            reasoning_content: Some(format!("read {index}")),
            provider_state: None,
            tool_calls: vec![ToolCall {
                id: format!("call-{index}"),
                name: "read_text_file".into(),
                arguments: format!(r#"{{"path":"f{index}"}}"#),
            }],
        });
        history.push(tool(&format!("call-{index}"), "x".repeat(content_chars)));
    }
    history
}

/// 与 provider::build_request 同口径的最小构造：把消息和工具 schema 序列化进请求体。
/// 上下文模块不依赖 Provider；测试用这个闭包模拟“每级重建实际请求体”。
fn build(messages: &[Message], tools: &[Value]) -> Result<Value> {
    Ok(json!({"messages": messages, "tools": tools}))
}

fn budget(window_tokens: u64, output_tokens: u64) -> BudgetLimits {
    BudgetLimits {
        window_tokens,
        output_tokens,
        safety_tokens: safety_margin(window_tokens),
        source: WindowSource::User,
    }
}

#[test]
fn a_large_window_sends_the_history_unchanged() {
    let history = history_with_tool_results(12, 400);
    let prepared = prepare_within_budget(&history, &[], &budget(1_000_000, 16_384), build).unwrap();
    assert_eq!(prepared.compressed_tools, 0);
    assert_eq!(prepared.messages, history);
}

#[test]
fn tighter_windows_fold_more_older_tool_results_and_keep_the_original_intact() {
    // 每条约 10k 字符。12 条工具结果：保留 8 条要折 4 条，保留 4 条折 8 条，全折到只剩摘要。
    let history = history_with_tool_results(12, 10_000);
    let original = history.clone();
    for (window, expected_compressed, expected_kept) in
        [(100_000, 4, 8), (50_000, 8, 4), (20_000, 12, 0)]
    {
        let prepared = prepare_within_budget(&history, &[], &budget(window, 1_000), build)
            .unwrap_or_else(|error| panic!("window {window} should fit: {error:#}"));
        assert_eq!(
            prepared.compressed_tools, expected_compressed,
            "window {window}"
        );
        assert_eq!(prepared.messages.len(), history.len());
        let tools: Vec<&Message> = prepared
            .messages
            .iter()
            .filter(|message| matches!(message, Message::Tool { .. }))
            .collect();
        assert_eq!(tools.len(), 12, "no tool result may disappear");
        let folded = tools
            .iter()
            .filter(|message| {
                matches!(message, Message::Tool { content, .. } if content.contains("older tool result compressed"))
            })
            .count();
        assert_eq!(folded, expected_compressed, "window {window}");
        // 保留下来的最近结果原文一字不改。
        let kept: Vec<&Message> = tools.iter().rev().take(expected_kept).copied().collect();
        for message in kept {
            assert!(
                matches!(message, Message::Tool { content, .. } if content.starts_with('x')),
                "recent tool result must stay verbatim"
            );
        }
        // 预算判断用的是请求体字符数，满足时一定放得下。
        let body = build(&prepared.messages, &[]).unwrap();
        assert!(
            estimate_body_tokens(&body)
                + budget(window, 1_000).output_tokens
                + safety_margin(window)
                <= window
        );
    }
    // 只改发送副本：原 history 一个字都没动。
    assert_eq!(history, original);
}

#[test]
fn still_over_budget_is_an_error_with_actionable_guidance_not_a_retry() {
    let history = history_with_tool_results(12, 10_000);
    let error = prepare_within_budget(&history, &[], &budget(3_000, 2_000), build).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("context window"), "{text}");
    assert!(text.contains("folding older tool results"), "{text}");
    assert!(text.contains("assistant model settings"), "{text}");
    assert!(text.contains("maximum output tokens"), "{text}");
}

#[test]
fn folding_keeps_tool_call_pairing_arguments_and_assistant_state() {
    let mut history = vec![Message::System {
        content: "system".into(),
    }];
    for index in 0..5 {
        history.push(Message::Assistant {
            content: format!("checking {index}"),
            reasoning_content: Some(format!("reason {index}")),
            provider_state: Some(json!({"format":"chat_completions","items":[index]})),
            tool_calls: vec![
                ToolCall {
                    id: format!("call-{index}-a"),
                    name: "first".into(),
                    arguments: format!(r#"{{"step":{index}}}"#),
                },
                ToolCall {
                    id: format!("call-{index}-b"),
                    name: "second".into(),
                    arguments: format!(r#"{{"step":{index},"extra":true}}"#),
                },
            ],
        });
        history.push(tool(&format!("call-{index}-a"), "a".repeat(4_000)));
        history.push(tool(&format!("call-{index}-b"), "b".repeat(4_000)));
    }
    let assistants: Vec<Message> = history
        .iter()
        .filter(|message| matches!(message, Message::Assistant { .. }))
        .cloned()
        .collect();
    let ids: Vec<String> = history
        .iter()
        .filter_map(|message| match message {
            Message::Tool { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    let (folded, compressed) = fold_older_tool_results(&history, 2);
    assert_eq!(compressed, 8);
    assert_eq!(folded.len(), history.len());
    // 助手消息（reasoning/provider_state/参数 JSON）与工具调用 id 顺序都不动。
    let folded_assistants: Vec<Message> = folded
        .iter()
        .filter(|message| matches!(message, Message::Assistant { .. }))
        .cloned()
        .collect();
    assert_eq!(folded_assistants, assistants);
    let folded_ids: Vec<String> = folded
        .iter()
        .filter_map(|message| match message {
            Message::Tool { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(folded_ids, ids);
    // 最近两条工具结果保持原文，其余变成带头部的摘要。
    let contents: Vec<&str> = folded
        .iter()
        .filter_map(|message| match message {
            Message::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert!(contents[8].starts_with('a') && contents[8].len() == 4_000);
    assert!(contents[9].starts_with('b') && contents[9].len() == 4_000);
    for content in &contents[..8] {
        assert!(
            content.contains("older tool result compressed"),
            "{content}"
        );
    }
}
