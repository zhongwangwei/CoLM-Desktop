use super::*;

fn line(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

/// 第 634 轮在 codex-cli 0.160.1 上录下的消息（略去无关字段）。
#[test]
fn notifications_become_panel_events() {
    let mut state = CodexState::default();
    let delta = line(
        r#"{"method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","itemId":"m","delta":"I"}}"#,
    );
    assert_eq!(
        map_notification(&delta, &mut state),
        [Outbound::AssistantDelta { text: "I".into() }]
    );

    // colm 的 MCP 调用由转发层出卡片，这里不出；shell 命令出卡片并对上结果。
    let colm = line(
        r#"{"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"exec-1","server":"colm","tool":"list_cases","arguments":{"root":"/d"}}}}"#,
    );
    assert!(map_notification(&colm, &mut state).is_empty());
    let shell = line(
        r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"exec-2","command":"/bin/zsh -lc 'printf hi > hello2.txt'","cwd":"/w"}}}"#,
    );
    let events = map_notification(&shell, &mut state);
    assert!(
        matches!(&events[0], Outbound::ToolCall { id, name, tier: Tier::Code, .. } if id == "exec-2" && name == "shell")
    );
    let done = line(
        r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"exec-2","status":"completed","exitCode":0,"aggregatedOutput":"","durationMs":12}}}"#,
    );
    assert!(matches!(
        &map_notification(&done, &mut state)[0],
        Outbound::ToolResult {
            ok: true,
            elapsed_ms: 12,
            ..
        }
    ));

    // 只有最终回答进结果，过程说明（commentary）不进。
    let commentary = line(
        r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m1","text":"I'll list the cases","phase":"commentary"}}}"#,
    );
    let answer = line(
        r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m2","text":"4 cases.","phase":"final_answer"}}}"#,
    );
    map_notification(&commentary, &mut state);
    map_notification(&answer, &mut state);
    assert_eq!(state.content, "4 cases.");

    let usage = line(
        r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"total":{"inputTokens":32642,"outputTokens":64}}}}"#,
    );
    map_notification(&usage, &mut state);
    assert_eq!(
        state.usage,
        Usage {
            prompt_tokens: 32642,
            completion_tokens: 64
        }
    );

    let completed = line(
        r#"{"method":"turn/completed","params":{"turn":{"status":"completed","error":null}}}"#,
    );
    map_notification(&completed, &mut state);
    assert_eq!(state.finished, Some(Ok(())));
    let mut failed = CodexState::default();
    map_notification(
        &line(
            r#"{"method":"turn/completed","params":{"turn":{"status":"failed","error":{"message":"usage limit reached"}}}}"#,
        ),
        &mut failed,
    );
    assert_eq!(failed.finished, Some(Err("usage limit reached".into())));
}

#[test]
fn requests_become_approvals_or_automatic_answers() {
    let command = json!({ "itemId": "exec-3", "threadId": "t", "turnId": "u", "command": "rm -rf build", "reason": "clean" });
    let request = approval_request("item/commandExecution/requestApproval", &command).unwrap();
    assert!(
        matches!(&request, Outbound::ApprovalRequest { id, tier: Tier::Code, summary, .. }
        if id == "exec-3" && summary == "执行命令：rm -rf build（clean）")
    );
    assert_eq!(
        approval_answer(
            "item/commandExecution/requestApproval",
            &command,
            &Decision::ApproveForSession
        ),
        json!({ "decision": "acceptForSession" })
    );
    assert_eq!(
        approval_answer(
            "item/fileChange/requestApproval",
            &json!({}),
            &Decision::Deny(None)
        ),
        json!({ "decision": "decline" })
    );
    let permissions = json!({ "itemId": "p", "permissions": { "network": true } });
    assert_eq!(
        approval_answer(
            "item/permissions/requestApproval",
            &permissions,
            &Decision::Deny(None)
        ),
        json!({ "permissions": {} })
    );
    // colm 工具调用的确认自动同意；别的服务的确认与问用户的问题拒绝。
    let colm = json!({ "serverName": "colm", "_meta": { "codex_approval_kind": "mcp_tool_call" } });
    assert!(approval_request("mcpServer/elicitation/request", &colm).is_none());
    assert_eq!(
        automatic_answer("mcpServer/elicitation/request", &colm)["action"],
        "accept"
    );
    let other =
        json!({ "serverName": "evil", "_meta": { "codex_approval_kind": "mcp_tool_call" } });
    assert_eq!(
        automatic_answer("mcpServer/elicitation/request", &other)["action"],
        "decline"
    );
    assert_eq!(
        automatic_answer("execCommandApproval", &json!({}))["decision"],
        "denied"
    );
}

#[test]
fn usage_is_reported_per_turn_not_per_thread() {
    let first = Usage {
        prompt_tokens: 167_725,
        completion_tokens: 227,
    };
    let second = Usage {
        prompt_tokens: 207_025,
        completion_tokens: 271,
    };
    assert_eq!(per_turn(first, Usage::default()), first);
    assert_eq!(
        per_turn(second, first),
        Usage {
            prompt_tokens: 39_300,
            completion_tokens: 44
        }
    );
    assert_eq!(per_turn(Usage::default(), first), Usage::default());
}

#[test]
fn web_search_cards_show_the_query_and_count_as_success_without_a_status() {
    let mut state = CodexState::default();
    let started = line(
        r#"{"method":"item/started","params":{"item":{"type":"webSearch","id":"ws_1","query":"","action":{"type":"search","query":"HESS 29 3119"}}}}"#,
    );
    assert!(matches!(&map_notification(&started, &mut state)[0],
        Outbound::ToolCall { summary, tier: Tier::Read, .. } if summary == "联网搜索：HESS 29 3119"));
    let done = line(
        r#"{"method":"item/completed","params":{"item":{"type":"webSearch","id":"ws_1","results":[{"domain":"hess.copernicus.org"}]}}}"#,
    );
    assert!(matches!(
        &map_notification(&done, &mut state)[0],
        Outbound::ToolResult { ok: true, .. }
    ));
    let empty =
        line(r#"{"method":"item/started","params":{"item":{"type":"webSearch","id":"ws_2"}}}"#);
    assert!(matches!(&map_notification(&empty, &mut state)[0],
        Outbound::ToolCall { summary, .. } if summary == "联网搜索"));
}
