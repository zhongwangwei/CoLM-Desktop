//! 对话循环：模型 → 工具调用 →（需要时）审批 → 结果回填；重复查询或接近限额时收束答复。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::message::{Message, Usage};
use crate::protocol::{Outbound, ToolEffect};
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
    /// 本轮的收尾是否由生成步数上限触发；正常完成或重复读取收尾为 false。
    pub limit_reached: bool,
    pub step_limit: usize,
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
        let mut limit_reached = false;
        for step in 1..=self.limits.max_steps {
            if cancel.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            let finalizing = step == self.limits.max_steps || repeated_reads >= 3;
            if finalizing {
                if repeated_reads < 3 && step == self.limits.max_steps {
                    limit_reached = true;
                }
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
            // 总结请求发空工具集；这里固定一个空切片，避免 `&[]` 临时值跨语句借用的麻烦。
            let empty_tools: [Value; 0] = [];
            let (messages, send_tools): (&[Message], &[Value]) = if finalizing {
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
                (&summary_history, &empty_tools)
            } else {
                (&*history, &tools)
            };
            // 每步发送前解析窗口预算。解析失败（未知窗口、api_options 输出键冲突）必须在这里
            // 结束本轮：不在解析失败后调用模型，也不继续派生新的工具调用。
            let budget = self.provider.budget_limits()?;
            let request_messages = match budget {
                Some(limits) => {
                    // 估算用 Provider 实际会发送的请求体（与 complete_* 同一个 build_request），
                    // 折叠只改发送副本，history/会话记录保持原文。
                    let build = |messages: &[Message], tools: &[Value]| {
                        self.provider.request_body(messages, tools)?.ok_or_else(|| {
                            anyhow::anyhow!(
                                "the model provider declares a context budget but does not expose the request body used to estimate it; enable the context window only for providers that can be estimated"
                            )
                        })
                    };
                    let prepared = crate::context::prepare_within_budget(
                        messages, send_tools, &limits, build,
                    )?;
                    emit(Outbound::Context {
                        estimated_tokens: prepared.estimated_tokens,
                        window_tokens: limits.window_tokens,
                        compressed_tools: prepared.compressed_tools,
                        available_input_tokens: Some(
                            limits
                                .window_tokens
                                .saturating_sub(prepared.estimated_tokens)
                                .saturating_sub(limits.output_tokens)
                                .saturating_sub(limits.safety_tokens),
                        ),
                        output_reserve_tokens: limits.output_tokens,
                        safety_margin_tokens: limits.safety_tokens,
                        window_source: Some(limits.source.as_str().to_owned()),
                    });
                    prepared.messages
                }
                None => {
                    // 没有声明窗口的 Provider（脚本化测试 Provider、旧配置）保持原来的固定阈值折叠，
                    // 事件里 available_input_tokens/window_source 为 None 表示未启用预算。
                    let prepared = crate::context::prepare(messages);
                    let (request_messages, estimated_tokens, compressed_tools) = match prepared {
                        Some(prepared) => (
                            prepared.messages,
                            prepared.estimated_tokens,
                            prepared.compressed_tools,
                        ),
                        None => (
                            messages.to_vec(),
                            crate::context::estimate_tokens(messages),
                            0,
                        ),
                    };
                    emit(Outbound::Context {
                        estimated_tokens,
                        window_tokens: crate::context::CONTEXT_WINDOW_TOKENS,
                        compressed_tools,
                        available_input_tokens: None,
                        output_reserve_tokens: 0,
                        safety_margin_tokens: 0,
                        window_source: None,
                    });
                    request_messages
                }
            };
            let mut final_events = Vec::new();
            let mut turn = self.provider.complete_cancellable(
                &request_messages,
                send_tools,
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
                outcome.limit_reached = limit_reached;
                outcome.step_limit = self.limits.max_steps;
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
        // 参数解析失败和未知工具都在发 `tool_call` 之前结束：审计日志里不会出现这个动作，
        // 恢复逻辑也就不会把它记成"未确认的写操作"。它们确定没有开始执行。
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
        return finish_not_started(
            call,
            "error: cancelled before execution; inspect existing state before retrying",
            emit,
        );
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
            return finish_not_started(call, "error: cancelled before approval", emit);
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
            return finish_not_started(call, &text, emit);
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return finish_not_started(call, "error: cancelled", emit);
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
    // 副作用结局只由结构事实决定（工具是否真的被调用、tier 是否只读、返回是否成功），
    // 不从结果文本推断。只读工具失败不会改任何状态；有副作用工具在开始执行后失败，
    // 失败前可能已经做了一半，只能保守记为 effect_unknown，等恢复时先核对。
    let effect = match (ok, tier) {
        (true, _) => ToolEffect::Completed,
        (false, Tier::Read) => ToolEffect::FailedNoEffect,
        (false, _) => ToolEffect::EffectUnknown,
    };
    // 先发结构化的结局，再发给人看的 `tool_result`：进程在这两条之间中断时，
    // 恢复逻辑已经知道该不该要求核对，而不是只看到一条没有语义的失败结果。
    emit(Outbound::ToolOutcome {
        id: call.id.clone(),
        name: call.name.clone(),
        effect,
    });
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

/// 执行前就结束（取消/拒绝）：发结构化的 `not_started` 结局并关闭工具卡片。
///
/// 为什么必须显式发：只留下 `tool_call` 事件的话，恢复逻辑只能把它当成"没有结果的写操作"
/// （即可能已经执行），制造一个并不存在的待核对项。拒绝和取消前工具确实没被调用。
fn finish_not_started(
    call: &crate::message::ToolCall,
    text: &str,
    emit: &mut dyn FnMut(Outbound),
) -> String {
    emit(Outbound::ToolOutcome {
        id: call.id.clone(),
        name: call.name.clone(),
        effect: ToolEffect::NotStarted,
    });
    emit(Outbound::ToolResult {
        id: call.id.clone(),
        name: call.name.clone(),
        ok: false,
        result: text.to_owned(),
        elapsed_ms: 0,
    });
    text.to_owned()
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod agent_tests;
