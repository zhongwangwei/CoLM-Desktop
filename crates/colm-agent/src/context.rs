//! 单次请求的上下文预算：DeepSeek V4 的窗口是 1M token，但历史会一直增长；接近窗口前自动折叠
//! 较早的工具结果，保留最近结果和全部对话骨架。这里只改发给模型的历史副本，不改会话记录。

use crate::message::Message;

/// `deepseek-flash` / `deepseek-v4-pro` 的标称上下文窗口（docs/design-ai-assistant.md）。
pub const CONTEXT_WINDOW_TOKENS: u64 = 1_000_000;
/// 到 80% 就提示用户，到 90% 再真正折叠。
pub const CONTEXT_WARN_TOKENS: u64 = 800_000;
pub const CONTEXT_COMPACT_TOKENS: u64 = 900_000;
/// 最近几轮工具结果保留原文；再多就优先折叠更早的。
const KEEP_RECENT_TOOL_RESULTS: [usize; 3] = [8, 4, 2];

/// 压缩后的请求历史和统计。
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedHistory {
    pub messages: Vec<Message>,
    /// 折叠前的估算 token 数（按字符数保守估，中文接近真实值，英文偏高）。
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

/// 超过阈值时返回折叠后的副本；没有可折叠内容或未超阈值时返回 `None`。
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
        // 先保留 8 条最近结果；如果仍然超预算，再逐步折叠到 4、2、0 条。
        if estimate_tokens(&prepared.messages) < CONTEXT_COMPACT_TOKENS {
            return Some(prepared);
        }
        last = Some(prepared);
    }
    last
}

fn compressed_stub(content: &str) -> String {
    let chars = content.chars().count();
    let head: String = content.chars().take(240).collect();
    format!("[older tool result compressed: {chars} chars; head: {head}]")
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod context_tests;
