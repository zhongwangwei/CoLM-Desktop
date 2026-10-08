use std::sync::Mutex;

use super::*;
use crate::message::ToolCall;
use crate::provider::Turn;
use crate::tools::{object, Tier, Tool};

/// 按脚本依次给出的模型回复；记下每次收到的对话，供断言。
struct Scripted {
    turns: Mutex<Vec<Turn>>,
    seen: Mutex<Vec<Vec<Message>>>,
}

impl Scripted {
    fn new(mut turns: Vec<Turn>) -> Self {
        turns.reverse();
        Self {
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Provider for Scripted {
    fn complete(
        &self,
        messages: &[Message],
        _tools: &[Value],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Turn> {
        self.seen.lock().unwrap().push(messages.to_vec());
        let turn = self.turns.lock().unwrap().pop().expect("script ran out");
        if !turn.content.is_empty() {
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
fn cancelling_and_runaway_loops_stop_the_turn() {
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

    let provider = Scripted::new(
        (0..5)
            .map(|i| tool_turn(vec![call(&i.to_string(), "echo", r#"{"x":1}"#)]))
            .collect(),
    );
    let error = agent(&provider, &registry)
        .run_turn(
            &mut vec![Message::User {
                content: "x".into(),
            }],
            &mut |_| {},
            &mut Answers(Vec::new(), 0),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert!(error.to_string().contains("stopped after 5 model steps"));
}
