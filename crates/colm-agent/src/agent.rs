//! 对话循环：模型 → 工具调用 →（需要时）审批 → 结果回填；重复查询或接近限额时收束答复。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::message::{Message, Usage};
use crate::protocol::Outbound;
use crate::provider::{Provider, StreamEvent};
use crate::tools::{result_text, Registry, Tier, ToolContext};

/// 审批结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approve,
    /// 批准，并在本会话里不再询问同名操作。
    ApproveForSession,
    Deny(Option<String>),
}

/// 审批来源：GUI（经 stdio）或测试里的脚本。
pub trait Approver {
    fn decide(&mut self, request: &Outbound) -> Decision;
    /// 这个操作不用问（审批策略为自动，或本会话已选过“不再询问”）。
    fn preapproved(&self, _name: &str, _tier: Tier) -> bool {
        false
    }
    /// 整体审批策略是否为“自动”。它只放行普通后端操作，不替代危险操作的逐次确认；
    /// 与“本会话允许同类操作”的按工具名记忆分开判断（GUI 审查 R13/B-04）。
    fn auto_policy(&self) -> bool {
        false
    }
    fn recovery_requires_approval(&self) -> bool {
        false
    }
    /// 记下“本会话不再询问”。
    fn remember(&mut self, _name: &str) {}
}

/// 每轮的限额。
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// 一轮里最多几次“模型生成”，最后一次预留给不调用工具的总结。
    pub max_steps: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_steps: 40 }
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
        anyhow::ensure!(self.limits.max_steps > 0, "max_steps must be positive");
        let mut previous_reads = Vec::new();
        let mut repeated_reads = 0;
        for step in 1..=self.limits.max_steps {
            if cancel.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            let finalizing = step == self.limits.max_steps || repeated_reads >= 3;
            if finalizing {
                self.context.note_failure(
                    if repeated_reads >= 3 {
                        "repeated_read_queries"
                    } else {
                        "turn_budget"
                    },
                    "",
                    "execution_limit",
                );
            }
            let mut summary_history;
            let messages = if finalizing {
                // 运行控制提示只用于本次请求，不伪装成用户保存的消息。
                summary_history = history.clone();
                let reason = if repeated_reads >= 3 {
                    "The same read-only queries have been repeated three times with unchanged results."
                } else {
                    "This is the last model generation allowed in this turn."
                };
                summary_history.push(Message::User {
                    content: format!("[Turn execution limit] {reason} Tools are unavailable for this response. Give a concise final answer in the user's language using the evidence already collected: confirmed progress, unresolved work, and the next useful step. Do not claim completion or scientific validation without evidence. If a job is still running, report its last observed status; do not pretend to keep monitoring after this reply."),
                });
                &summary_history
            } else {
                &*history
            };
            let mut final_events = Vec::new();
            let mut turn = self.provider.complete_cancellable(
                messages,
                if finalizing { &[] } else { &tools },
                &mut |event| {
                    if finalizing {
                        final_events.push(event);
                        return;
                    }
                    emit(match event {
                        StreamEvent::Content(text) => Outbound::AssistantDelta { text },
                        StreamEvent::Reasoning(text) => Outbound::ReasoningDelta { text },
                    })
                },
                cancel,
            )?;
            // 停止之后才返回的这次生成不算数：不记进历史，也不当成正常结束发 usage 与 turn_done（G-02）。
            if cancel.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            outcome.steps = step;
            outcome.usage.prompt_tokens += turn.usage.prompt_tokens;
            outcome.usage.completion_tokens += turn.usage.completion_tokens;
            if finalizing {
                if cancel.load(Ordering::SeqCst) {
                    bail!("cancelled");
                }
                if !turn.tool_calls.is_empty() || turn.content.trim().is_empty() {
                    // 服务商忽略无工具请求时，也不能执行新操作或保存未配对的工具调用。
                    turn = crate::provider::Turn {
                        content: "本轮已停止继续调用工具，任务尚未确认全部完成。已有操作和结果已保留；请核对最后的运行状态，再继续未完成的步骤。\nThis turn stopped calling tools before completion was confirmed. Existing actions and results are retained; check the last run status before continuing.".into(),
                        ..Default::default()
                    };
                } else {
                    for event in final_events {
                        if let StreamEvent::Reasoning(text) = event {
                            emit(Outbound::ReasoningDelta { text });
                        }
                    }
                }
                // GUI 依赖增量事件显示正文；不依赖服务商一定发送内容增量。
                emit(Outbound::AssistantDelta {
                    text: turn.content.clone(),
                });
            }
            history.push(turn.message());
            if turn.tool_calls.is_empty() {
                outcome.content = turn.content;
                return Ok(outcome);
            }
            let mut results = Vec::new();
            for call in &turn.tool_calls {
                let content = self.call_tool(call, emit, approver, cancel);
                results.push(
                    serde_json::from_str::<Value>(&content)
                        .unwrap_or_else(|_| Value::String(content.clone())),
                );
                history.push(Message::Tool {
                    tool_call_id: call.id.clone(),
                    content,
                });
            }
            let reads: Option<Vec<_>> = turn
                .tool_calls
                .iter()
                .zip(results)
                .map(|(call, result)| {
                    let tool = self.registry.find(&call.name)?;
                    if tool.tier() != Tier::Read {
                        return None;
                    }
                    let args = serde_json::from_str::<Value>(&call.arguments).ok()?;
                    // Read-tier UI actions can advance pages without changing their arguments.
                    Some((call.name.clone(), args, result))
                })
                .collect();
            if let Some(reads) = reads {
                repeated_reads = if reads == previous_reads {
                    repeated_reads + 1
                } else {
                    1
                };
                previous_reads = reads;
            } else {
                previous_reads.clear();
                repeated_reads = 0;
            }
        }
        unreachable!("the reserved final generation always ends the turn")
    }

    fn call_tool(
        &self,
        call: &crate::message::ToolCall,
        emit: &mut dyn FnMut(Outbound),
        approver: &mut dyn Approver,
        cancel: &AtomicBool,
    ) -> String {
        execute_tool(self.registry, &self.context, call, emit, approver, cancel)
    }
}

/// 执行一个工具调用（含审批与审计事件），返回回填给模型的文本。出错、被拒都作为结果返回，不中断这一轮。
/// 内置后端的对话循环与外部后端（经 `colm-mcp` 转发）共用这一处，所以规则一样。
pub fn execute_tool(
    registry: &Registry,
    context: &ToolContext,
    call: &crate::message::ToolCall,
    emit: &mut dyn FnMut(Outbound),
    approver: &mut dyn Approver,
    cancel: &AtomicBool,
) -> String {
    let Some(tool) = registry.find(&call.name) else {
        context.note_failure(&call.name, &call.id, "unknown_tool");
        return format!("error: there is no tool named {}", call.name);
    };
    let args: Value = match serde_json::from_str(&call.arguments) {
        Ok(args) => args,
        Err(error) => {
            context.note_failure(&call.name, &call.id, "invalid_arguments");
            return format!("error: the arguments are not valid JSON ({error})");
        }
    };
    let summary = tool.summary(&args);
    let tier = tool.tier_for(&args);
    let session_allowance = tool.session_allowance_for(&args);
    let allowance = tool.allowance_key(&args);
    let preapproved = tier.needs_approval()
        && !tool.requires_explicit_approval()
        && !approver.recovery_requires_approval()
        && (session_allowance || (approver.auto_policy() && tier == Tier::Act))
        && approver.preapproved(&allowance, tier);
    emit(Outbound::ToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        tier,
        summary: summary.clone(),
        preapproved,
    });
    if cancel.load(Ordering::SeqCst) {
        return "error: cancelled before execution; inspect existing state before retrying".into();
    }
    if tier.needs_approval() && !preapproved {
        let request = Outbound::ApprovalRequest {
            id: call.id.clone(),
            name: call.name.clone(),
            tier,
            summary,
            arguments: call.arguments.clone(),
            explicit_only: tool.requires_explicit_approval()
                || !session_allowance
                || approver.recovery_requires_approval(),
        };
        emit(request.clone());
        if cancel.load(Ordering::SeqCst) {
            return "error: cancelled before approval".into();
        }
        let decision = approver.decide(&request);
        if decision == Decision::ApproveForSession
            && session_allowance
            && !approver.recovery_requires_approval()
        {
            approver.remember(&allowance);
        }
        if let Decision::Deny(note) = decision {
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
    // 一个工具 panic 不能让整个助手进程卡死：捕获后变成一条错误结果，本轮继续。
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.call(&args, context)));
    let (ok, text) = match outcome {
        Ok(Ok(value)) => (true, result_text(&value)),
        Ok(Err(error)) => (false, format!("error: {error:#}")),
        Err(_) => (
            false,
            "error: internal tool failure (the tool panicked); its effects may be incomplete; inspect the state before retrying".to_owned(),
        ),
    };
    emit(Outbound::ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        ok,
        result: text.clone(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
    if !ok {
        context.note_failure(&call.name, &call.id, "tool_failed");
    }
    text
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod agent_tests;
