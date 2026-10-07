//! 对话循环：模型 → 工具调用 →（需要时）审批 → 结果回填 → 直到模型不再调工具，或超出限额、被取消。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::message::{Message, Usage};
use crate::protocol::Outbound;
use crate::provider::{Provider, StreamEvent};
use crate::tools::{result_text, Registry, ToolContext};

/// 审批结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approve,
    Deny(Option<String>),
}

/// 审批来源：GUI（经 stdio）或测试里的脚本。
pub trait Approver {
    fn decide(&mut self, request: &Outbound) -> Decision;
}

/// 每轮的限额。
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// 一轮里最多几次“模型生成”（每次可以带多个工具调用）。
    pub max_steps: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_steps: 24 }
    }
}

pub struct Agent<'a> {
    pub provider: &'a dyn Provider,
    pub registry: &'a Registry,
    pub context: ToolContext,
    pub limits: Limits,
    pub strict: bool,
}

/// 一轮的结果。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TurnOutcome {
    pub content: String,
    pub steps: usize,
    pub usage: Usage,
}

impl Agent<'_> {
    /// 跑一轮：`history` 末尾应是这一轮的用户消息；所有新消息都追加进去。
    pub fn run_turn(
        &self,
        history: &mut Vec<Message>,
        emit: &mut dyn FnMut(Outbound),
        approver: &mut dyn Approver,
        cancel: &AtomicBool,
    ) -> Result<TurnOutcome> {
        let tools = self.registry.api_tools(self.strict);
        let mut outcome = TurnOutcome::default();
        for step in 1..=self.limits.max_steps {
            if cancel.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            let turn = self.provider.complete(history, &tools, &mut |event| {
                emit(match event {
                    StreamEvent::Content(text) => Outbound::AssistantDelta { text },
                    StreamEvent::Reasoning(text) => Outbound::ReasoningDelta { text },
                })
            })?;
            outcome.steps = step;
            outcome.usage.prompt_tokens += turn.usage.prompt_tokens;
            outcome.usage.completion_tokens += turn.usage.completion_tokens;
            history.push(turn.message());
            if turn.tool_calls.is_empty() {
                outcome.content = turn.content;
                return Ok(outcome);
            }
            for call in &turn.tool_calls {
                let content = self.call_tool(call, emit, approver, cancel);
                history.push(Message::Tool {
                    tool_call_id: call.id.clone(),
                    content,
                });
            }
        }
        bail!(
            "stopped after {} model steps without a final answer; ask a narrower question or raise the limit",
            self.limits.max_steps
        )
    }

    /// 执行一个工具调用，返回回填给模型的文本。出错、被拒都作为结果告诉模型，不中断这一轮。
    fn call_tool(
        &self,
        call: &crate::message::ToolCall,
        emit: &mut dyn FnMut(Outbound),
        approver: &mut dyn Approver,
        cancel: &AtomicBool,
    ) -> String {
        let Some(tool) = self.registry.find(&call.name) else {
            return format!("error: there is no tool named {}", call.name);
        };
        let args: Value = match serde_json::from_str(&call.arguments) {
            Ok(args) => args,
            Err(error) => return format!("error: the arguments are not valid JSON ({error})"),
        };
        let summary = tool.summary(&args);
        emit(Outbound::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            tier: tool.tier(),
            summary: summary.clone(),
        });
        if tool.tier().needs_approval() {
            let request = Outbound::ApprovalRequest {
                id: call.id.clone(),
                name: call.name.clone(),
                tier: tool.tier(),
                summary,
                arguments: call.arguments.clone(),
            };
            emit(request.clone());
            if let Decision::Deny(note) = approver.decide(&request) {
                let text = match note {
                    Some(note) => format!("the user declined this action: {note}"),
                    None => "the user declined this action".to_owned(),
                };
                emit(Outbound::ToolResult {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    ok: false,
                    result: text.clone(),
                    elapsed_ms: 0,
                });
                return text;
            }
        }
        if cancel.load(Ordering::SeqCst) {
            return "error: cancelled".to_owned();
        }
        let started = Instant::now();
        let (ok, text) = match tool.call(&args, &self.context) {
            Ok(value) => (true, result_text(&value)),
            Err(error) => (false, format!("error: {error:#}")),
        };
        emit(Outbound::ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            ok,
            result: text.clone(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
        text
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod agent_tests;
