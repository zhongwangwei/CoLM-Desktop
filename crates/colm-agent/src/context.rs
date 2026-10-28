//! 单次请求的上下文预算：DeepSeek V4 的窗口是 1M token，但历史会一直增长；接近窗口前自动折叠
//! 较早的工具结果，保留最近结果和全部对话骨架。这里只改发给模型的历史副本，不改会话记录。
//!
//! 预算有两种来源：用户/服务商配置给出的真实窗口（`BudgetLimits`，按实际请求体估算），
//! 以及脚本化测试 Provider 与旧配置的固定阈值行为（[`CONTEXT_COMPACT_TOKENS`]）。

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;

use crate::message::Message;

/// `deepseek-flash` / `deepseek-v4-pro` 的标称上下文窗口（docs/design-ai-assistant.md）。
pub const CONTEXT_WINDOW_TOKENS: u64 = 1_000_000;
/// 到 80% 就提示用户，到 90% 再真正折叠。
pub const CONTEXT_WARN_TOKENS: u64 = 800_000;
pub const CONTEXT_COMPACT_TOKENS: u64 = 900_000;
/// 最近几轮工具结果保留原文；再多就优先折叠更早的。
const KEEP_RECENT_TOOL_RESULTS: [usize; 3] = [8, 4, 2];
/// 有真实窗口时逐级尝试保留的最近工具结果条数。最后一级 0 表示全部折叠：
/// 再不合就只能报错，不无限重试（每次重估都要重新构造请求体，成本是真金白银）。
const BUDGET_KEEP_RECENT_TOOL_RESULTS: [usize; 4] = [8, 4, 2, 0];

/// 针对请求体估算时额外预留的协议开销（角色标记、SSE 包装等）。
pub const BODY_PROTOCOL_MARGIN_TOKENS: u64 = 64;

/// 窗口的来源：用户显式填写，或文档确认的服务商预设。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowSource {
    User,
    Preset,
}

impl WindowSource {
    /// 协议事件里的 `window_source` 文本（GUI 按固定文字显示来源）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Preset => "preset",
        }
    }
}

/// 一次请求的预算上限，由 `Provider::budget_limits` 在每次发送前解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    pub window_tokens: u64,
    /// 最终请求体里真正生效的输出上限；预算预留与实际发送必须是同一个值。
    pub output_tokens: u64,
    /// 给服务端计数误差与协议包装留的余量，见 [`safety_margin`]。
    pub safety_tokens: u64,
    pub source: WindowSource,
}

/// 安全余量：窗口的 2%（1k–16k）。字符数估算本身不精确，服务端按真实 token 计数，
/// 留 2% 是为了 1M 窗口下的估算偏差不把请求顶出窗外。
pub fn safety_margin(window_tokens: u64) -> u64 {
    (window_tokens / 50).clamp(1024, 16_384)
}

/// 压缩后的请求历史和统计。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedHistory {
    pub messages: Vec<Message>,
    /// 折叠前的估算 token 数（按字符数保守估，中文接近真实值，英文偏高）。
    pub estimated_tokens: u64,
    pub compressed_tools: usize,
}

/// 预算内准备好的发送副本与统计。
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetedHistory {
    pub messages: Vec<Message>,
    /// 实际请求体（含工具 schema）的保守 token 上界，见 [`estimate_body_tokens`]。
    pub estimated_tokens: u64,
    pub compressed_tools: usize,
}

/// 估算 token：没有 tokenizer，用字符数作为上限估计。DeepSeek 中文约 1 字符 1 token，
/// 英文约 4 字符 1 token；保守估计宁可早压缩，也不让请求超窗。
pub fn estimate_tokens(messages: &[Message]) -> u64 {
    messages.iter().map(message_chars).sum::<usize>() as u64
}

fn message_chars(message: &Message) -> usize {
    match message {
        Message::System { content } | Message::User { content } => content.chars().count() + 16,
        Message::Assistant {
            content,
            reasoning_content,
            tool_calls,
            ..
        } => {
            content.chars().count()
                + reasoning_content
                    .as_deref()
                    .map_or(0, |text| text.chars().count())
                + tool_calls
                    .iter()
                    .map(|call| {
                        call.id.chars().count()
                            + call.name.chars().count()
                            + call.arguments.chars().count()
                            + 32
                    })
                    .sum::<usize>()
                + 24
        }
        Message::Tool {
            tool_call_id,
            content,
        } => tool_call_id.chars().count() + content.chars().count() + 16,
    }
}

/// 有真实窗口时的估算口径：用实际要发送的请求体的紧凑 JSON 字符数作为输入 token 的保守上界。
/// 1 字符 ≈ 1 token 对中文接近真实、对英文明显偏高；不冒充精确 token 计数。
/// 固定加 [`BODY_PROTOCOL_MARGIN_TOKENS`] 给角色标记与流式包装留余量。GUI 显示的 estimated_tokens
/// 用的就是这个口径，预算判断与事件必须是同一个数。
pub fn estimate_body_tokens(body: &Value) -> u64 {
    body.to_string().chars().count() as u64 + BODY_PROTOCOL_MARGIN_TOKENS
}

/// 超过固定阈值时返回折叠后的副本；没有可折叠内容或未超阈值时返回 `None`。
/// 没有声明真实窗口的 Provider（脚本化测试 Provider、旧配置）走这条路径，行为保持不变。
pub fn prepare(messages: &[Message]) -> Option<PreparedHistory> {
    let estimated = estimate_tokens(messages);
    if estimated < CONTEXT_COMPACT_TOKENS {
        return None;
    }
    let tool_positions: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| matches!(message, Message::Tool { .. }).then_some(index))
        .collect();
    if tool_positions.is_empty() {
        return None;
    }
    let mut last: Option<PreparedHistory> = None;
    for keep in KEEP_RECENT_TOOL_RESULTS {
        let mut folded = messages.to_vec();
        let cutoff = tool_positions.len().saturating_sub(keep);
        let mut compressed = 0;
        for &index in &tool_positions[..cutoff] {
            if let Some(Message::Tool { content, .. }) = folded.get_mut(index) {
                *content = compressed_stub(content);
                compressed += 1;
            }
        }
        if compressed == 0 {
            continue;
        }
        let prepared = PreparedHistory {
            messages: folded,
            estimated_tokens: estimated,
            compressed_tools: compressed,
        };
        // 先保留 8 条最近结果；仍然超预算就只保留 4 条、再 2 条。最近的结果是模型下一步要用的，不折到 0。
        if estimate_tokens(&prepared.messages) < CONTEXT_COMPACT_TOKENS {
            return Some(prepared);
        }
        last = Some(prepared);
    }
    last
}

/// 折叠除最近 `keep_recent` 条之外的工具结果：只替换 `content`，工具调用 id、消息顺序、
/// 助手消息（含 reasoning / provider_state / 签名）和工具参数 JSON 全部原样保留 ——
/// 服务端会拒绝配对不上的历史，而签名一旦改写就会被判无效。
pub fn fold_older_tool_results(messages: &[Message], keep_recent: usize) -> (Vec<Message>, usize) {
    let tool_positions: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| matches!(message, Message::Tool { .. }).then_some(index))
        .collect();
    let cutoff = tool_positions.len().saturating_sub(keep_recent);
    let mut folded = messages.to_vec();
    let mut compressed = 0;
    for &index in &tool_positions[..cutoff] {
        if let Some(Message::Tool { content, .. }) = folded.get_mut(index) {
            *content = compressed_stub(content);
            compressed += 1;
        }
    }
    (folded, compressed)
}

/// 在给定窗口预算内准备发送副本：先按原样估算，超出才逐级折叠较早工具结果，
/// 每级都重新构造**实际要发送的**请求体再估算。仍超限就报错说明怎么缩小输入或调整配置，
/// 不把超窗请求发给服务商，也不无限重试。原 `messages` 不会被修改。
pub fn prepare_within_budget(
    messages: &[Message],
    tools: &[Value],
    limits: &BudgetLimits,
    build: impl Fn(&[Message], &[Value]) -> Result<Value>,
) -> Result<BudgetedHistory> {
    // 候选按折叠强度递增：原样 → 保留最近 8/4/2/0 条工具结果。由弱到强试，
    // 一旦满足预算就停：能少折就少折，最近工具结果对下一步最有用。
    let mut candidates: Vec<(Vec<Message>, usize)> = vec![(messages.to_vec(), 0)];
    for keep in BUDGET_KEEP_RECENT_TOOL_RESULTS {
        let (folded, compressed) = fold_older_tool_results(messages, keep);
        // 这一级没有可折的（工具结果本来就不多于 keep 条）时与上一级相同，不必重复估算。
        if compressed == 0 {
            continue;
        }
        candidates.push((folded, compressed));
    }
    let reserve = limits.output_tokens + limits.safety_tokens;
    let mut smallest = u64::MAX;
    for (candidate, compressed_tools) in candidates {
        let body = build(&candidate, tools)?;
        let estimated_tokens = estimate_body_tokens(&body);
        if estimated_tokens.saturating_add(reserve) <= limits.window_tokens {
            return Ok(BudgetedHistory {
                messages: candidate,
                estimated_tokens,
                compressed_tools,
            });
        }
        smallest = smallest.min(estimated_tokens);
    }
    bail!(
        "the request needs about {smallest} estimated input tokens plus {} reserved output tokens and {} safety tokens, which exceeds the {}-token context window even after folding older tool results; shorten the attached evidence or conversation, raise the context window in the assistant model settings, or lower the maximum output tokens",
        limits.output_tokens,
        limits.safety_tokens,
        limits.window_tokens
    )
}

fn compressed_stub(content: &str) -> String {
    let chars = content.chars().count();
    let head: String = content.chars().take(240).collect();
    format!("[older tool result compressed: {chars} chars; head: {head}]")
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod context_tests;
