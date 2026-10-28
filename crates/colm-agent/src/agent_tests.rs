use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use super::*;
use crate::context::{safety_margin, BudgetLimits};
use crate::message::ToolCall;
use crate::provider::{build_request, effective_max_output, ApiFormat, ProviderConfig, Turn};
use crate::tools::{object, Tier, Tool};

/// 按脚本依次给出的模型回复；记下每次收到的对话，供断言。
struct Scripted {
    turns: Mutex<Vec<Turn>>,
    seen: Mutex<Vec<Vec<Message>>>,
    seen_tools: Mutex<Vec<Vec<Value>>>,
    stream_content: bool,
}

impl Scripted {
    fn new(mut turns: Vec<Turn>) -> Self {
        turns.reverse();
        Self {
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
            seen_tools: Mutex::new(Vec::new()),
            stream_content: true,
        }
    }
}

impl Provider for Scripted {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[Value],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Turn> {
        self.seen.lock().unwrap().push(messages.to_vec());
        self.seen_tools.lock().unwrap().push(tools.to_vec());
        let turn = self.turns.lock().unwrap().pop().expect("script ran out");
        if self.stream_content && !turn.content.is_empty() {
            on_event(StreamEvent::Content(turn.content.clone()));
        }
        Ok(turn)
    }
}

struct Echo(Tier);

impl Tool for Echo {
    fn name(&self) -> &'static str {
        if self.0 == Tier::Read {
            "echo"
        } else {
            "act"
        }
    }
    fn description(&self) -> &'static str {
        "test tool"
    }
    fn parameters(&self) -> Value {
        object(serde_json::json!({ "x": { "type": "integer" } }))
    }
    fn tier(&self) -> Tier {
        self.0
    }
    fn call(&self, args: &Value, _ctx: &ToolContext) -> Result<Value> {
        if args["x"] == -1 {
            anyhow::bail!("x must not be -1");
        }
        Ok(serde_json::json!({ "doubled": args["x"].as_i64().unwrap_or(0) * 2 }))
    }
}

/// 故意 panic 的工具：验证工具 panic 不会让整轮/助手进程卡死。
struct Panics;

impl Tool for Panics {
    fn name(&self) -> &'static str {
        "panic_tool"
    }
    fn description(&self) -> &'static str {
        "test tool that panics"
    }
    fn parameters(&self) -> Value {
        object(serde_json::json!({}))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, _args: &Value, _ctx: &ToolContext) -> Result<Value> {
        panic!("boom from a tool")
    }
}

struct Answers(Vec<Decision>, usize);

impl Approver for Answers {
    fn decide(&mut self, _request: &Outbound) -> Decision {
        self.1 += 1;
        self.0.remove(0)
    }
}

fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.into(),
    }
}

fn tool_turn(calls: Vec<ToolCall>) -> Turn {
    Turn {
        reasoning: Some("thinking".into()),
        tool_calls: calls,
        ..Turn::default()
    }
}

fn answer(text: &str) -> Turn {
    Turn {
        content: text.into(),
        ..Turn::default()
    }
}

fn agent<'a>(provider: &'a Scripted, registry: &'a Registry) -> Agent<'a> {
    Agent {
        provider,
        registry,
        context: ToolContext::default(),
        limits: Limits { max_steps: 5 },
        strict: false,
    }
}

#[test]
fn read_tools_run_without_approval_and_results_feed_the_next_step() {
    let provider = Scripted::new(vec![
        tool_turn(vec![
            call("a", "echo", r#"{"x":2}"#),
            call("b", "echo", r#"{"x":-1}"#),
            call("c", "missing", "{}"),
            call("d", "echo", "{not json"),
        ]),
        answer("x doubled is 4"),
    ]);
    let registry = Registry::with(vec![Box::new(Echo(Tier::Read))]);
    let mut history = vec![Message::User {
        content: "double 2".into(),
    }];
    let mut events = Vec::new();
    let mut approver = Answers(Vec::new(), 0);
    let outcome = agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut approver,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.content, "x doubled is 4");
    assert_eq!(outcome.steps, 2);
    assert_eq!(approver.1, 0);
    // 第二步发给模型的对话里：带推理的助手消息、四条工具结果（出错的也如实回填）。
    let second = &provider.seen.lock().unwrap()[1];
    assert!(
        matches!(&second[1], Message::Assistant { reasoning_content: Some(r), .. } if r == "thinking")
    );
    let results: Vec<&str> = second[2..]
        .iter()
        .map(|m| match m {
            Message::Tool { content, .. } => content.as_str(),
            _ => panic!("expected tool messages"),
        })
        .collect();
    assert_eq!(results[0], r#"{"doubled":4}"#);
    assert!(results[1].contains("x must not be -1"));
    assert!(results[2].contains("no tool named missing"));
    assert!(results[3].contains("not valid JSON"));
    assert!(events
        .iter()
        .any(|e| matches!(e, Outbound::ToolResult { ok: true, .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, Outbound::AssistantDelta { text } if text == "x doubled is 4")));
}

#[test]
fn a_panicking_tool_becomes_an_error_result_and_the_turn_continues() {
    let provider = Scripted::new(vec![
        tool_turn(vec![call("a", "panic_tool", "{}")]),
        answer("recovered from the tool panic"),
    ]);
    let registry = Registry::with(vec![Box::new(Panics)]);
    let mut history = vec![Message::User {
        content: "go".into(),
    }];
    let mut events = Vec::new();
    let mut approver = Answers(Vec::new(), 0);
    let outcome = agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut approver,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.content, "recovered from the tool panic");
    assert_eq!(outcome.steps, 2);
    let second = &provider.seen.lock().unwrap()[1];
    let tool_text = second
        .iter()
        .find_map(|message| match message {
            Message::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .expect("the panicking tool result should be fed back");
    assert!(tool_text.contains("internal tool failure"), "{tool_text}");
    assert!(events
        .iter()
        .any(|event| matches!(event, Outbound::ToolResult { ok: false, .. })));
}

#[test]
fn acting_tools_wait_for_approval_and_a_denial_reaches_the_model() {
    let provider = Scripted::new(vec![
        tool_turn(vec![
            call("a", "act", r#"{"x":1}"#),
            call("b", "act", r#"{"x":3}"#),
        ]),
        answer("one ran, one was declined"),
    ]);
    let registry = Registry::with(vec![Box::new(Echo(Tier::Act))]);
    let mut history = vec![Message::User {
        content: "go".into(),
    }];
    let mut events = Vec::new();
    let mut approver = Answers(
        vec![Decision::Approve, Decision::Deny(Some("too long".into()))],
        0,
    );
    agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut approver,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(approver.1, 2);
    let requests = events
        .iter()
        .filter(|e| matches!(e, Outbound::ApprovalRequest { .. }))
        .count();
    assert_eq!(requests, 2);
    let tool_messages: Vec<String> = history
        .iter()
        .filter_map(|m| match m {
            Message::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tool_messages[0], r#"{"doubled":2}"#);
    assert_eq!(tool_messages[1], "the user declined this action: too long");
}

/// 记住“本会话不再询问”的审批者（和 colm-agent 服务里的一样只放行运行操作）。
struct Remembering(Vec<Decision>, Vec<String>);

impl Approver for Remembering {
    fn decide(&mut self, _request: &Outbound) -> Decision {
        self.0.remove(0)
    }
    fn preapproved(&self, name: &str, tier: Tier) -> bool {
        tier == Tier::Act && self.1.iter().any(|n| n == name)
    }
    fn remember(&mut self, name: &str) {
        self.1.push(name.to_owned());
    }
}

#[test]
fn approving_for_the_session_skips_later_requests_for_that_tool() {
    let provider = Scripted::new(vec![
        tool_turn(vec![call("a", "act", r#"{"x":1}"#)]),
        tool_turn(vec![call("b", "act", r#"{"x":2}"#)]),
        answer("both ran"),
    ]);
    let registry = Registry::with(vec![Box::new(Echo(Tier::Act))]);
    let mut history = vec![Message::User {
        content: "go".into(),
    }];
    let mut events = Vec::new();
    let mut approver = Remembering(vec![Decision::ApproveForSession], Vec::new());
    agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut approver,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(approver.1, ["act"]);
    let requests = events
        .iter()
        .filter(|e| matches!(e, Outbound::ApprovalRequest { .. }))
        .count();
    assert_eq!(requests, 1);
    let preapproved: Vec<bool> = events
        .iter()
        .filter_map(|e| match e {
            Outbound::ToolCall { preapproved, .. } => Some(*preapproved),
            _ => None,
        })
        .collect();
    assert_eq!(preapproved, [false, true]);
    let ran = history
        .iter()
        .filter(|m| matches!(m, Message::Tool { content, .. } if content.contains("doubled")))
        .count();
    assert_eq!(ran, 2);
}

/// 改源码的操作：和 Echo 一样，但不许“本会话都允许”。
struct Patch;

impl Tool for Patch {
    fn name(&self) -> &'static str {
        "patch"
    }
    fn description(&self) -> &'static str {
        "test patch tool"
    }
    fn parameters(&self) -> Value {
        object(serde_json::json!({ "x": { "type": "integer" } }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn session_allowance(&self) -> bool {
        false
    }
    fn call(&self, args: &Value, _ctx: &ToolContext) -> Result<Value> {
        Ok(serde_json::json!({ "patched": args["x"] }))
    }
}

/// 即使审批者对“补丁”这类工具按会话放行也不生效：每个补丁都要单独问（设计稿第 4 节）。
struct EagerRemembering(Vec<Decision>, Vec<String>);

impl Approver for EagerRemembering {
    fn decide(&mut self, _request: &Outbound) -> Decision {
        self.0.remove(0)
    }
    fn preapproved(&self, name: &str, tier: Tier) -> bool {
        tier == Tier::Code && self.1.iter().any(|n| n == name)
    }
    fn remember(&mut self, name: &str) {
        self.1.push(name.to_owned());
    }
}

#[test]
fn a_patch_is_asked_about_every_time_even_if_the_user_chose_always_allow() {
    let provider = Scripted::new(vec![
        tool_turn(vec![call("a", "patch", r#"{"x":1}"#)]),
        tool_turn(vec![call("b", "patch", r#"{"x":2}"#)]),
        answer("both patched"),
    ]);
    let registry = Registry::with(vec![Box::new(Patch)]);
    let mut history = vec![Message::User {
        content: "go".into(),
    }];
    let mut events = Vec::new();
    // 第一次点“本会话都允许”，第二次仍然要问（这里批准一次）。
    let mut approver = EagerRemembering(
        vec![Decision::ApproveForSession, Decision::Approve],
        Vec::new(),
    );
    agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut approver,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(approver.1.is_empty(), "nothing was remembered for a patch");
    let requests = events
        .iter()
        .filter(|e| matches!(e, Outbound::ApprovalRequest { .. }))
        .count();
    assert_eq!(requests, 2, "the second patch was asked about again");
}

#[test]
fn every_generation_reports_the_context_estimate_and_window() {
    let provider = Scripted::new(vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let mut events = Vec::new();
    agent(&provider, &registry)
        .run_turn(
            &mut vec![Message::User {
                content: "inspect".into(),
            }],
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    let context = events
        .iter()
        .find_map(|event| match event {
            Outbound::Context {
                estimated_tokens,
                window_tokens,
                compressed_tools,
                available_input_tokens,
                output_reserve_tokens,
                safety_margin_tokens,
                window_source,
            } => Some((
                *estimated_tokens,
                *window_tokens,
                *compressed_tools,
                *available_input_tokens,
                *output_reserve_tokens,
                *safety_margin_tokens,
                window_source.clone(),
            )),
            _ => None,
        })
        .expect("a context event must be emitted before the provider call");
    assert!(context.0 > 0);
    assert_eq!(context.1, crate::context::CONTEXT_WINDOW_TOKENS);
    assert_eq!(context.2, 0);
    // 没有声明窗口的 Provider 走旧的固定阈值路径：事件明确说“未启用预算”。
    assert_eq!(context.3, None);
    assert_eq!(context.4, 0);
    assert_eq!(context.5, 0);
    assert_eq!(context.6, None);
}

#[test]
fn cancelling_stops_the_turn_and_budget_exhaustion_returns_a_summary() {
    let registry = Registry::with(vec![Box::new(Echo(Tier::Read))]);
    let provider = Scripted::new(vec![answer("never")]);
    let cancelled = AtomicBool::new(true);
    let error = agent(&provider, &registry)
        .run_turn(
            &mut vec![Message::User {
                content: "x".into(),
            }],
            &mut |_| {},
            &mut Answers(Vec::new(), 0),
            &cancelled,
        )
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));

    let mut turns: Vec<_> = (0..4)
        .map(|i| {
            tool_turn(vec![call(
                &i.to_string(),
                "echo",
                &format!(r#"{{"x":{i}}}"#),
            )])
        })
        .collect();
    turns.push(answer(
        "Checked four inputs; the remaining work is unverified.",
    ));
    let provider = Scripted::new(turns);
    let mut history = vec![Message::User {
        content: "x".into(),
    }];
    let outcome = agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |_| {},
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.steps, 5);
    assert!(outcome.limit_reached);
    assert_eq!(outcome.step_limit, 5);
    assert!(outcome.content.contains("unverified"));
    assert!(provider.seen_tools.lock().unwrap()[4].is_empty());
    assert!(
        matches!(provider.seen.lock().unwrap()[4].last(), Some(Message::User { content }) if content.contains("last model generation"))
    );
    assert_eq!(
        history
            .iter()
            .filter(|m| matches!(m, Message::User { .. }))
            .count(),
        1
    );
}

#[test]
fn repeated_read_queries_finalize_early_with_canonical_arguments_and_usage() {
    let mut turns: Vec<_> = [r#"{"x":1}"#, r#"{ "x" : 1 }"#, r#"{"x":1}"#]
        .iter()
        .enumerate()
        .map(|(i, args)| tool_turn(vec![call(&i.to_string(), "echo", args)]))
        .collect();
    turns.push(answer("Still running; completion is unverified."));
    for turn in &mut turns {
        turn.usage = Usage {
            prompt_tokens: 10,
            completion_tokens: 2,
        };
    }
    let provider = Scripted::new(turns);
    let registry = Registry::with(vec![Box::new(Echo(Tier::Read))]);
    let mut history = vec![Message::User {
        content: "wait".into(),
    }];
    let mut events = Vec::new();
    let outcome = agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.steps, 4);
    assert_eq!(
        outcome.usage,
        Usage {
            prompt_tokens: 40,
            completion_tokens: 8
        }
    );
    assert!(provider.seen_tools.lock().unwrap()[3].is_empty());
    assert!(
        matches!(provider.seen.lock().unwrap()[3].last(), Some(Message::User { content }) if content.contains("repeated three times"))
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Outbound::ToolCall { .. }))
            .count(),
        3
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, Outbound::AssistantDelta { text } if text == &outcome.content)));
}

#[test]
fn identical_ui_requests_with_changing_results_keep_tools_available() {
    use crate::tools::ui::{UiBridge, UiHandle};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    struct Advancing(AtomicUsize);
    impl UiBridge for Advancing {
        fn request(&self, _action: &str, _args: Value) -> Result<Value> {
            let page = self.0.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({ "step": page }))
        }
    }

    for (tool, args) in [
        ("ui_click", r#"{"target":"next-step"}"#),
        ("ui_state", "{}"),
    ] {
        let mut turns: Vec<_> = (0..4)
            .map(|i| tool_turn(vec![call(&i.to_string(), tool, args)]))
            .collect();
        turns.push(answer("The workflow advanced."));
        let provider = Scripted::new(turns);
        let registry = Registry::standard_with(false, true);
        let mut agent = agent(&provider, &registry);
        agent.limits.max_steps = 6;
        agent.context.ui = Some(UiHandle(Arc::new(Advancing(AtomicUsize::new(0)))));
        let outcome = agent
            .run_turn(
                &mut vec![Message::User {
                    content: "continue".into(),
                }],
                &mut |_| {},
                &mut Answers(Vec::new(), 0),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.steps, 5, "{tool}");
        assert_eq!(outcome.content, "The workflow advanced.");
        assert!(provider
            .seen_tools
            .lock()
            .unwrap()
            .iter()
            .all(|tools| !tools.is_empty()));
    }
}

#[test]
fn different_reads_and_intervening_actions_do_not_trigger_repetition_summary() {
    let provider = Scripted::new(vec![
        tool_turn(vec![call("1", "echo", r#"{"x":1}"#)]),
        tool_turn(vec![call("2", "echo", r#"{"x":2}"#)]),
        tool_turn(vec![call("3", "echo", r#"{"x":2}"#)]),
        tool_turn(vec![call("4", "act", r#"{"x":2}"#)]),
        tool_turn(vec![call("5", "echo", r#"{"x":2}"#)]),
        answer("Checked after the change."),
    ]);
    let registry = Registry::with(vec![Box::new(Echo(Tier::Read)), Box::new(Echo(Tier::Act))]);
    let mut agent = agent(&provider, &registry);
    agent.limits.max_steps = 8;
    let mut approvals = Answers(vec![Decision::Approve], 0);
    let outcome = agent
        .run_turn(
            &mut vec![Message::User {
                content: "check".into(),
            }],
            &mut |_| {},
            &mut approvals,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.steps, 6);
    assert_eq!(approvals.1, 1);
    assert!(provider
        .seen_tools
        .lock()
        .unwrap()
        .iter()
        .all(|tools| !tools.is_empty()));
}

#[test]
fn final_generation_never_executes_unsolicited_tools_or_persists_control_prompts() {
    for reply in [
        Turn::default(),
        tool_turn(vec![call("unexpected", "act", r#"{"x":1}"#)]),
    ] {
        let provider = Scripted::new(vec![reply]);
        let registry = Registry::with(vec![Box::new(Echo(Tier::Act))]);
        let mut agent = agent(&provider, &registry);
        agent.limits.max_steps = 1;
        let mut history = vec![Message::User {
            content: "check".into(),
        }];
        let mut events = Vec::new();
        let outcome = agent
            .run_turn(
                &mut history,
                &mut |e| events.push(e),
                &mut Answers(Vec::new(), 0),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.steps, 1);
        assert!(outcome.content.contains("before completion was confirmed"));
        assert!(events.iter().all(|e| !matches!(
            e,
            Outbound::ToolCall { .. } | Outbound::ApprovalRequest { .. }
        )));
        assert_eq!(history.len(), 2);
        assert!(
            matches!(history.last(), Some(Message::Assistant { tool_calls, .. }) if tool_calls.is_empty())
        );
    }
}

#[test]
fn default_turn_budget_allows_forty_generations() {
    assert_eq!(Limits::default().max_steps, 40);
}

#[test]
fn zero_step_budget_is_rejected_before_calling_the_provider() {
    let provider = Scripted::new(vec![]);
    let registry = Registry::with(vec![]);
    let mut agent = agent(&provider, &registry);
    agent.limits.max_steps = 0;
    assert!(agent
        .run_turn(
            &mut vec![],
            &mut |_| {},
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("max_steps must be positive"));
    assert!(provider.seen.lock().unwrap().is_empty());
}

#[test]
fn summary_is_visible_when_provider_returns_text_without_content_deltas() {
    let mut provider = Scripted::new(vec![answer("The job is still running.")]);
    provider.stream_content = false;
    let registry = Registry::with(vec![]);
    let mut agent = agent(&provider, &registry);
    agent.limits.max_steps = 1;
    let mut events = Vec::new();
    let outcome = agent
        .run_turn(
            &mut vec![Message::User {
                content: "wait".into(),
            }],
            &mut |e| events.push(e),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, Outbound::AssistantDelta { text } if text == &outcome.content)));
}

#[test]
fn basic_file_actions_run_through_approval_and_feed_verified_results_back() {
    let root = std::env::temp_dir().join(format!("colm-agent-files-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let provider = Scripted::new(vec![
        tool_turn(vec![call(
            "mkdir",
            "create_directory",
            r#"{"path":"reports"}"#,
        )]),
        tool_turn(vec![call(
            "write",
            "write_text_file",
            r#"{"path":"reports/result.txt","content":"checked\n"}"#,
        )]),
        tool_turn(vec![call(
            "copy",
            "copy_file",
            r#"{"source":"reports/result.txt","destination":"reports/copy.txt"}"#,
        )]),
        tool_turn(vec![call(
            "read",
            "read_text_file",
            r#"{"path":"reports/copy.txt"}"#,
        )]),
        answer("The report was created and its copied contents were checked."),
    ]);
    let registry = Registry::standard();
    let mut runner = agent(&provider, &registry);
    runner.context.project_root = root.clone();
    let mut history = vec![Message::User {
        content: "Create and copy a report.".into(),
    }];
    let mut events = Vec::new();
    let mut approver = Answers(vec![Decision::Approve; 3], 0);
    runner
        .run_turn(
            &mut history,
            &mut |e| events.push(e),
            &mut approver,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(approver.1, 3);
    assert_eq!(
        std::fs::read_to_string(root.join("reports/copy.txt")).unwrap(),
        "checked\n"
    );
    assert!(events
        .iter()
        .filter_map(|e| match e {
            Outbound::ToolResult { ok, .. } => Some(*ok),
            _ => None,
        })
        .all(|ok| ok));
    assert!(provider.seen.lock().unwrap().last().unwrap().iter().any(|m| {
        matches!(m, Message::Tool { tool_call_id, content } if tool_call_id == "read" && content.contains("checked"))
    }));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn denying_a_directory_creation_changes_nothing() {
    let root = std::env::temp_dir().join(format!("colm-agent-files-denied-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let registry = Registry::standard();
    let context = ToolContext {
        project_root: root.clone(),
        ..ToolContext::default()
    };
    let result = execute_tool(
        &registry,
        &context,
        &call("mkdir", "create_directory", r#"{"path":"not-created"}"#),
        &mut |_| {},
        &mut Answers(vec![Decision::Deny(None)], 0),
        &AtomicBool::new(false),
    );
    assert!(result.contains("declined"));
    assert!(!root.join("not-created").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recycle_actions_require_each_approval_even_when_everything_is_preapproved() {
    struct Automatic(usize);
    impl Approver for Automatic {
        fn preapproved(&self, _: &str, _: Tier) -> bool {
            true
        }
        fn decide(&mut self, _: &Outbound) -> Decision {
            self.0 += 1;
            Decision::ApproveForSession
        }
        fn remember(&mut self, _: &str) {
            panic!("recycle approval must never be remembered");
        }
    }
    let root = std::env::temp_dir().join(format!("colm-recycle-approval-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("report.txt"), "preserved").unwrap();
    let ctx = ToolContext {
        project_root: root.clone(),
        ..ToolContext::default()
    };
    let registry = Registry::standard();
    let mut approver = Automatic(0);
    let mut events = Vec::new();
    let mut run = |name: &str, args: Value| {
        execute_tool(
            &registry,
            &ctx,
            &call(&format!("{name}-call"), name, &args.to_string()),
            &mut |event| events.push(event),
            &mut approver,
            &AtomicBool::new(false),
        )
    };
    let first: Value =
        serde_json::from_str(&run("trash_path", serde_json::json!({"path":"report.txt"}))).unwrap();
    assert!(!root.join("report.txt").exists());
    let restored: Value = serde_json::from_str(&run(
        "restore_trash",
        serde_json::json!({"id": first["id"]}),
    ))
    .unwrap();
    assert_eq!(restored["restored"], true);
    run("trash_path", serde_json::json!({"path":"report.txt"}));
    assert_eq!(approver.0, 3);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(
                e,
                Outbound::ApprovalRequest {
                    explicit_only: true,
                    ..
                }
            ))
            .count(),
        3
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_checkpoint_failure_cancel_stops_before_approval_or_action() {
    let registry = Registry::with(vec![Box::new(Echo(Tier::Act))]);
    let cancel = AtomicBool::new(false);
    let mut approver = Answers(Vec::new(), 0);
    let result = execute_tool(
        &registry,
        &ToolContext::default(),
        &call("x", "act", r#"{"x":1}"#),
        &mut |event| {
            if matches!(event, Outbound::ToolCall { .. }) {
                cancel.store(true, Ordering::SeqCst);
            }
        },
        &mut approver,
        &cancel,
    );
    assert!(result.contains("cancelled before execution"));
    assert_eq!(approver.1, 0);
}

#[test]
fn code_page_commits_require_fresh_decisions_despite_auto_or_remembered_ui_commit() {
    struct Automatic(usize);
    impl Approver for Automatic {
        fn preapproved(&self, _: &str, _: Tier) -> bool {
            true
        }
        fn auto_policy(&self) -> bool {
            true
        }
        fn decide(&mut self, request: &Outbound) -> Decision {
            assert!(matches!(
                request,
                Outbound::ApprovalRequest {
                    tier: Tier::Code,
                    explicit_only: true,
                    ..
                }
            ));
            self.0 += 1;
            Decision::ApproveForSession
        }
        fn remember(&mut self, _: &str) {
            panic!("code page commits must never remember ui_commit");
        }
    }
    struct Window;
    impl crate::tools::ui::UiBridge for Window {
        fn request(&self, action: &str, _: Value) -> Result<Value> {
            assert_eq!(action, "commit");
            Ok(serde_json::json!({"ok":true}))
        }
    }
    let registry = Registry::standard_for(false, true, false);
    let context = ToolContext {
        ui: Some(crate::tools::ui::UiHandle(std::sync::Arc::new(Window))),
        ..ToolContext::default()
    };
    let mut approver = Automatic(0);
    let mut events = Vec::new();
    let targets = [
        "workspace-create",
        "workspace-patch",
        "workspace-check",
        "version-branch-code",
        "version-import-process",
        "resource-build",
        "resource-workspace-submit",
        "version-fix-drift",
        "code-file-save",
    ];
    for target in targets {
        let result = execute_tool(
            &registry,
            &context,
            &call(
                target,
                "ui_commit",
                &serde_json::json!({"target":target,"purpose":"check"}).to_string(),
            ),
            &mut |event| events.push(event),
            &mut approver,
            &AtomicBool::new(false),
        );
        assert!(result.contains("true"));
    }
    assert_eq!(approver.0, targets.len());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                Outbound::ToolCall {
                    tier: Tier::Code,
                    preapproved: false,
                    ..
                }
            ))
            .count(),
        targets.len()
    );
    let result = execute_tool(
        &registry,
        &context,
        &call(
            "run",
            "ui_commit",
            r#"{"target":"run-submit","purpose":"run"}"#,
        ),
        &mut |_| {},
        &mut approver,
        &AtomicBool::new(false),
    );
    assert!(result.contains("true"));
    assert_eq!(
        approver.0,
        targets.len(),
        "ordinary Act commits still honor Auto"
    );
}

/// 声明真实窗口的脚本 Provider：请求体用生产代码的 build_request 构造（与发送同一口径），
/// 记录每次实际收到的消息、工具，以及估算时传入的工具集与调用次数。
struct Budgeted {
    config: ProviderConfig,
    turns: Mutex<Vec<Turn>>,
    seen: Mutex<Vec<Vec<Message>>>,
    seen_tools: Mutex<Vec<Vec<Value>>>,
    estimated_tools: Mutex<Vec<Vec<Value>>>,
    calls: AtomicUsize,
}

impl Budgeted {
    fn new(config: ProviderConfig, mut turns: Vec<Turn>) -> Self {
        turns.reverse();
        Self {
            config,
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
            seen_tools: Mutex::new(Vec::new()),
            estimated_tools: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }
}

impl Provider for Budgeted {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[Value],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Turn> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(messages.to_vec());
        self.seen_tools.lock().unwrap().push(tools.to_vec());
        let turn = self.turns.lock().unwrap().pop().expect("script ran out");
        if !turn.content.is_empty() {
            on_event(StreamEvent::Content(turn.content.clone()));
        }
        Ok(turn)
    }
    fn budget_limits(&self) -> Result<Option<BudgetLimits>> {
        let (window_tokens, source) = self.config.effective_context_window()?;
        let body = build_request(&self.config, &[], &[])?;
        let output_tokens = effective_max_output(&body, self.config.api_format)?;
        Ok(Some(BudgetLimits {
            window_tokens,
            output_tokens,
            safety_tokens: safety_margin(window_tokens),
            source,
        }))
    }
    fn request_body(&self, messages: &[Message], tools: &[Value]) -> Result<Option<Value>> {
        self.estimated_tools.lock().unwrap().push(tools.to_vec());
        build_request(&self.config, messages, tools).map(Some)
    }
}

fn budgeted_config(window: Option<u64>) -> ProviderConfig {
    let mut config = ProviderConfig::deepseek("deepseek-flash", "test-key".into());
    config.base_url = "https://api.deepseek.com".into();
    config.context_window = window;
    config
}

fn long_history(count: usize, chars: usize) -> Vec<Message> {
    let mut history = vec![Message::System {
        content: "system".into(),
    }];
    for index in 0..count {
        history.push(Message::Assistant {
            content: String::new(),
            reasoning_content: Some(format!("reason {index}")),
            provider_state: None,
            tool_calls: vec![call(
                &format!("call-{index}"),
                "read_text_file",
                &format!(r#"{{"path":"f{index}"}}"#),
            )],
        });
        history.push(Message::Tool {
            tool_call_id: format!("call-{index}"),
            content: "x".repeat(chars),
        });
    }
    history.push(Message::User {
        content: "continue".into(),
    });
    history
}

fn budgeted_agent<'a>(provider: &'a Budgeted, registry: &'a Registry) -> Agent<'a> {
    Agent {
        provider,
        registry,
        context: ToolContext::default(),
        limits: Limits { max_steps: 5 },
        strict: false,
    }
}

fn context_event(events: &[Outbound]) -> (u64, u64, usize, Option<u64>, u64, u64, Option<String>) {
    events
        .iter()
        .find_map(|event| match event {
            Outbound::Context {
                estimated_tokens,
                window_tokens,
                compressed_tools,
                available_input_tokens,
                output_reserve_tokens,
                safety_margin_tokens,
                window_source,
            } => Some((
                *estimated_tokens,
                *window_tokens,
                *compressed_tools,
                *available_input_tokens,
                *output_reserve_tokens,
                *safety_margin_tokens,
                window_source.clone(),
            )),
            _ => None,
        })
        .expect("a context event must be emitted before the provider call")
}

#[test]
fn a_real_window_folds_older_tool_results_without_rewriting_the_session() {
    let provider = Budgeted::new(budgeted_config(Some(120_000)), vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let mut history = long_history(12, 10_000);
    let original_history = history.clone();
    let mut events = Vec::new();
    let outcome = budgeted_agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.content, "done");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    // 折叠尝试有上界（原样 + 保留 8/4/2/0 条），估算次数不会随历史增长而失控。
    assert!(provider.estimated_tools.lock().unwrap().len() <= 5);
    let (estimated, window, compressed, available, output, safety, source) = context_event(&events);
    assert_eq!(window, 120_000);
    assert_eq!(output, 16_384);
    assert_eq!(safety, safety_margin(120_000));
    assert_eq!(source.as_deref(), Some("user"));
    assert_eq!(compressed, 4, "12 results, keep the recent 8");
    assert!(estimated > 0);
    assert_eq!(
        available,
        Some(
            120_000u64
                .saturating_sub(estimated)
                .saturating_sub(output)
                .saturating_sub(safety)
        )
    );
    // 发送副本：工具调用 id、顺序、配对与助手消息原样；只折叠最早 4 条结果。
    let sent = &provider.seen.lock().unwrap()[0];
    assert_eq!(sent.len(), original_history.len());
    let sent_tools: Vec<&Message> = sent
        .iter()
        .filter(|message| matches!(message, Message::Tool { .. }))
        .collect();
    assert_eq!(sent_tools.len(), 12);
    for (index, message) in sent_tools.iter().enumerate() {
        let Message::Tool {
            tool_call_id,
            content,
        } = message
        else {
            unreachable!()
        };
        assert_eq!(tool_call_id, &format!("call-{index}"));
        if index < 4 {
            assert!(
                content.contains("older tool result compressed"),
                "index {index}"
            );
        } else {
            assert_eq!(content, &"x".repeat(10_000), "index {index}");
        }
    }
    let assistants_before: Vec<&Message> = original_history
        .iter()
        .filter(|message| matches!(message, Message::Assistant { .. }))
        .collect();
    let assistants_after: Vec<&Message> = sent
        .iter()
        .filter(|message| matches!(message, Message::Assistant { .. }))
        .collect();
    assert_eq!(assistants_before, assistants_after);
    // 会话历史保持原文：只在末尾追加了模型答复。
    assert_eq!(history.len(), original_history.len() + 1);
    for (left, right) in original_history.iter().zip(history.iter()) {
        assert_eq!(left, right);
    }
    assert!(history
        .iter()
        .filter(|message| matches!(message, Message::Tool { .. }))
        .all(|message| matches!(message, Message::Tool { content, .. } if content == &"x".repeat(10_000))));
}

#[test]
fn a_window_too_small_for_any_folding_is_rejected_before_the_provider_runs() {
    let provider = Budgeted::new(budgeted_config(Some(1_024)), vec![answer("never")]);
    let registry = Registry::with(vec![]);
    let mut history = vec![Message::User {
        content: "hello".into(),
    }];
    let mut events = Vec::new();
    let error = budgeted_agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("context window"), "{text}");
    assert!(text.contains("folding older tool results"), "{text}");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        0,
        "an over-budget request must not reach the provider"
    );
    assert!(provider.seen.lock().unwrap().is_empty());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Outbound::Context { .. })),
        "no context event for a request that was never sent"
    );
    assert_eq!(
        provider.estimated_tools.lock().unwrap().len(),
        1,
        "with no tool results there is only the unfolded candidate; no retry loop"
    );
    assert_eq!(
        history.len(),
        1,
        "the rejected turn must not append a model message"
    );
}

#[test]
fn an_unknown_window_is_reported_before_the_provider_is_called() {
    let mut config = budgeted_config(None);
    config.model = "mystery-model".into();
    config.base_url = "https://custom.example/v1".into();
    config.provider_id = "custom".into();
    let provider = Budgeted::new(config, vec![answer("never")]);
    let registry = Registry::with(vec![]);
    let mut history = vec![Message::User {
        content: "hi".into(),
    }];
    let error = budgeted_agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |_| {},
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    let text = format!("{error:#}");
    assert!(
        text.contains("context window") && text.contains("settings"),
        "{text}"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn output_option_overrides_reserve_exactly_what_is_sent() {
    let mut config = budgeted_config(Some(200_000));
    config.api_options = serde_json::json!({"max_tokens": 8192});
    let provider = Budgeted::new(config, vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let mut events = Vec::new();
    budgeted_agent(&provider, &registry)
        .run_turn(
            &mut vec![Message::User {
                content: "hi".into(),
            }],
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    let (_, window, _, _, output, safety, source) = context_event(&events);
    assert_eq!(window, 200_000);
    assert_eq!(output, 8_192, "reservation must equal the body override");
    assert_eq!(safety, safety_margin(200_000));
    assert_eq!(source.as_deref(), Some("user"));
    let limits = provider.budget_limits().unwrap().unwrap();
    assert_eq!(limits.output_tokens, 8_192);
    let sent = provider.seen.lock().unwrap()[0].clone();
    let body = provider
        .request_body(&sent, &provider.seen_tools.lock().unwrap()[0])
        .unwrap()
        .unwrap();
    assert_eq!(
        effective_max_output(&body, provider.config.api_format).unwrap(),
        8_192,
        "the reserved output limit must be the one in the request body"
    );
}

#[test]
fn responses_output_override_is_reserved_exactly_and_sent() {
    let mut config = budgeted_config(Some(200_000));
    config.api_format = ApiFormat::Responses;
    config.api_options = serde_json::json!({"max_output_tokens": 2048});
    let provider = Budgeted::new(config, vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let mut events = Vec::new();
    budgeted_agent(&provider, &registry)
        .run_turn(
            &mut vec![Message::User {
                content: "hi".into(),
            }],
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    let (_, _, _, _, output, _, _) = context_event(&events);
    assert_eq!(
        output, 2_048,
        "reservation must equal the effective body limit"
    );
    let sent = provider.seen.lock().unwrap()[0].clone();
    let body = build_request(&provider.config, &sent, &[]).unwrap();
    assert_eq!(body["max_output_tokens"].as_u64(), Some(2_048));
}

#[test]
fn conflicting_output_options_are_rejected_before_the_provider_is_called() {
    for (format, options, expected) in [
        (
            ApiFormat::ChatCompletions,
            serde_json::json!({"max_completion_tokens": 4096}),
            "max_completion_tokens",
        ),
        (
            ApiFormat::Responses,
            serde_json::json!({"max_tokens": 4096}),
            "max_tokens",
        ),
        (
            ApiFormat::Anthropic,
            serde_json::json!({"max_output_tokens": 4096}),
            "max_output_tokens",
        ),
    ] {
        let mut config = budgeted_config(Some(200_000));
        config.api_format = format;
        config.api_options = options;
        let provider = Budgeted::new(config, vec![answer("never")]);
        let registry = Registry::with(vec![]);
        let error = budgeted_agent(&provider, &registry)
            .run_turn(
                &mut vec![Message::User {
                    content: "hi".into(),
                }],
                &mut |_| {},
                &mut Answers(Vec::new(), 0),
                &AtomicBool::new(false),
            )
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains(expected), "{format:?}: {text}");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0, "{format:?}");
    }
}

#[test]
fn the_final_summary_is_estimated_with_an_empty_tool_set_under_the_same_budget() {
    let provider = Budgeted::new(
        budgeted_config(Some(110_000)),
        vec![answer("final summary")],
    );
    let registry = Registry::with(vec![Box::new(Echo(Tier::Read))]);
    let mut history = long_history(10, 10_000);
    let mut runner = budgeted_agent(&provider, &registry);
    runner.limits.max_steps = 1; // 第一步就是预留的总结生成
    let mut events = Vec::new();
    let outcome = runner
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.content, "final summary");
    assert!(outcome.limit_reached);
    assert!(provider.seen_tools.lock().unwrap()[0].is_empty());
    assert!(provider
        .estimated_tools
        .lock()
        .unwrap()
        .iter()
        .all(|tools| tools.is_empty()));
    let sent = &provider.seen.lock().unwrap()[0];
    assert!(matches!(
        sent.last(),
        Some(Message::User { content }) if content.contains("last model generation")
    ));
    assert!(history.iter().all(
        |message| !matches!(message, Message::User { content } if content.contains("[Turn execution limit]"))
    ));
    let (_, window, compressed, _, output, safety, _) = context_event(&events);
    assert_eq!(window, 110_000);
    assert_eq!(output, 16_384);
    assert_eq!(safety, safety_margin(110_000));
    assert!(
        compressed > 0,
        "the summary request must still be folded to fit"
    );
}

#[test]
fn a_documented_preset_window_sends_the_history_unchanged() {
    let provider = Budgeted::new(budgeted_config(None), vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let mut history = long_history(4, 500);
    let original = history.clone();
    let mut events = Vec::new();
    budgeted_agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(&provider.seen.lock().unwrap()[0][..], &original[..]);
    let (_, window, compressed, _, output, safety, source) = context_event(&events);
    assert_eq!(window, crate::context::CONTEXT_WINDOW_TOKENS);
    assert_eq!(compressed, 0);
    assert_eq!(output, 16_384);
    assert_eq!(safety, safety_margin(window));
    assert_eq!(source.as_deref(), Some("preset"));
}

#[test]
fn a_provider_without_budget_keeps_the_fixed_threshold_behaviour() {
    // 旧的固定阈值：约 1M 字符触发折叠，保留最近 8 条工具结果，事件说“未启用预算”。
    let provider = Scripted::new(vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let mut history = long_history(100, 10_000);
    let original = history.clone();
    let mut events = Vec::new();
    agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    let sent = &provider.seen.lock().unwrap()[0];
    assert_eq!(sent.len(), original.len());
    let tools: Vec<&Message> = sent
        .iter()
        .filter(|message| matches!(message, Message::Tool { .. }))
        .collect();
    assert_eq!(tools.len(), 100);
    let folded = tools
        .iter()
        .filter(|message| {
            matches!(message, Message::Tool { content, .. } if content.contains("older tool result compressed"))
        })
        .count();
    assert_eq!(folded, 92);
    for message in tools.iter().skip(92) {
        assert!(matches!(message, Message::Tool { content, .. } if content == &"x".repeat(10_000)));
    }
    let (estimated, window, compressed, available, output, safety, source) = context_event(&events);
    assert!(estimated > 0);
    assert_eq!(window, crate::context::CONTEXT_WINDOW_TOKENS);
    assert_eq!(compressed, 92);
    assert_eq!(available, None);
    assert_eq!(output, 0);
    assert_eq!(safety, 0);
    assert_eq!(source, None);
    // 会话历史未被折叠：固定阈值路径也只改发送副本。
    assert!(history
        .iter()
        .filter(|message| matches!(message, Message::Tool { .. }))
        .all(
            |message| matches!(message, Message::Tool { content, .. } if content.len() == 10_000)
        ));
}

#[test]
fn the_reported_estimate_is_the_unicode_body_estimate_with_tool_schemas() {
    let provider = Budgeted::new(budgeted_config(Some(200_000)), vec![answer("done")]);
    let registry = Registry::with(vec![Box::new(Echo(Tier::Read))]);
    let mut history = long_history(3, 2_000);
    // 中文工具结果：估算按字符数（≈token 上界），不能按 UTF-8 字节虚增 3 倍。
    if let Some(Message::Tool { content, .. }) = history
        .iter_mut()
        .find(|message| matches!(message, Message::Tool { .. }))
    {
        *content = "中文".repeat(1_000);
    }
    let mut events = Vec::new();
    budgeted_agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    let (estimated, ..) = context_event(&events);
    let sent = provider.seen.lock().unwrap()[0].clone();
    let sent_tools = provider.seen_tools.lock().unwrap()[0].clone();
    let body = build_request(&provider.config, &sent, &sent_tools).unwrap();
    assert_eq!(estimated, crate::context::estimate_body_tokens(&body));
    // 中文、推理内容与工具 schema 都在实际请求体里，因而都在估算里。
    let text = body.to_string();
    assert!(
        text.contains("中文"),
        "unicode content must be estimated by chars"
    );
    assert!(
        text.contains("reason 0"),
        "reasoning_content is replayed and counted"
    );
    assert!(
        text.contains("read_text_file"),
        "tool schema is part of the estimate"
    );
    // 工具 schema 计入估算：同一历史不带工具时明显更小。
    let without_tools = build_request(&provider.config, &sent, &[]).unwrap();
    assert!(estimated > crate::context::estimate_body_tokens(&without_tools));
}

#[test]
fn provider_state_replaces_assistant_messages_in_native_estimation_without_double_counting() {
    let mut config = budgeted_config(Some(200_000));
    config.api_format = ApiFormat::Responses;
    let provider = Budgeted::new(config, vec![answer("done")]);
    let registry = Registry::with(vec![]);
    let state = serde_json::json!({
        "format": "responses",
        "base_url": "https://api.deepseek.com",
        "model": "deepseek-flash",
        "provider_id": "deepseek",
        "items": [
            {"type":"reasoning","id":"rs_1","encrypted_content":"e".repeat(5_000)},
            {"type":"function_call","call_id":"call-0","name":"read_text_file","arguments":"{}"}
        ]
    });
    let mut history = vec![
        Message::User {
            content: "inspect".into(),
        },
        Message::Assistant {
            content: "assistant text that must not be sent twice".into(),
            reasoning_content: Some("r".repeat(1_000)),
            provider_state: Some(state.clone()),
            tool_calls: vec![call("call-0", "read_text_file", "{}")],
        },
        Message::Tool {
            tool_call_id: "call-0".into(),
            content: "result".into(),
        },
        Message::User {
            content: "continue".into(),
        },
    ];
    let mut events = Vec::new();
    budgeted_agent(&provider, &registry)
        .run_turn(
            &mut history,
            &mut |event| events.push(event),
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap();
    let (estimated, window, _, _, _, _, source) = context_event(&events);
    assert_eq!(window, 200_000);
    assert_eq!(source.as_deref(), Some("user"));
    let sent = provider.seen.lock().unwrap()[0].clone();
    let body = build_request(&provider.config, &sent, &[]).unwrap();
    // 估算口径就是实际请求体：签名/加密推理的 provider_state 计入，且被它替代的
    // 助手正文与推理不会重复发送（不重复计算 token）。
    assert_eq!(estimated, crate::context::estimate_body_tokens(&body));
    let text = body.to_string();
    assert!(text.contains(&"e".repeat(5_000)));
    assert!(!text.contains("assistant text that must not be sent twice"));
    assert!(!text.contains(&"r".repeat(1_000)));
    // 压缩不会破坏请求体：仍是可解析的 JSON。
    serde_json::from_str::<Value>(&text).unwrap();
}

#[test]
fn legacy_context_events_without_budget_fields_still_deserialize() {
    // GUI 的旧审计事件/旧进程不包含预算字段：serde(default) 必须让它们照常可读。
    let old: Outbound = serde_json::from_value(serde_json::json!({
        "type": "context",
        "estimated_tokens": 123,
        "window_tokens": 1_000_000,
        "compressed_tools": 2
    }))
    .unwrap();
    match old {
        Outbound::Context {
            estimated_tokens,
            window_tokens,
            compressed_tools,
            available_input_tokens,
            output_reserve_tokens,
            safety_margin_tokens,
            window_source,
        } => {
            assert_eq!(estimated_tokens, 123);
            assert_eq!(window_tokens, 1_000_000);
            assert_eq!(compressed_tools, 2);
            assert_eq!(available_input_tokens, None);
            assert_eq!(output_reserve_tokens, 0);
            assert_eq!(safety_margin_tokens, 0);
            assert_eq!(window_source, None);
        }
        other => panic!("expected a context event, got {other:?}"),
    }
    // 新事件也保持同一套字段名，GUI 直接按 snake_case 读取。
    let budgeted = Outbound::Context {
        estimated_tokens: 10,
        window_tokens: 1_000_000,
        compressed_tools: 0,
        available_input_tokens: Some(963_552),
        output_reserve_tokens: 16_384,
        safety_margin_tokens: 16_384,
        window_source: Some("preset".into()),
    };
    let round: Outbound = serde_json::from_str(&serde_json::to_string(&budgeted).unwrap()).unwrap();
    assert_eq!(round, budgeted);
}
