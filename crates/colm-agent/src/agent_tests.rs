use std::sync::Mutex;

use super::*;
use crate::message::ToolCall;
use crate::provider::Turn;
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
