//! GUI 与 `colm-agent` 之间的 stdio JSONL 协议：每行一个 JSON 对象，`type` 字段区分。

use serde::{Deserialize, Serialize};

use crate::provider::ProviderConfig;
use crate::tools::Tier;

/// GUI → agent。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    /// 设定（或更换）模型服务与工具环境。Key 不经这里传，agent 自己从钥匙串取。
    Configure {
        provider: ProviderConfig,
        project_root: String,
        #[serde(default)]
        kernel_dir: Option<String>,
        #[serde(default)]
        docs_root: Option<String>,
    },
    /// 用户的一条消息；`context` 是界面自动附上的当前页面信息（选中的算例、Study 等）。
    UserMessage {
        text: String,
        #[serde(default)]
        context: Option<String>,
    },
    ApprovalDecision {
        id: String,
        approve: bool,
        #[serde(default)]
        note: Option<String>,
    },
    /// 停止当前这一轮（正在生成或在等审批时都可以）。
    Cancel,
    /// 开新会话（清空对话历史，保留配置）。
    NewSession,
}

/// agent → GUI。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    Ready {
        session: String,
        model: String,
    },
    AssistantDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        tier: Tier,
        summary: String,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        result: String,
        elapsed_ms: u64,
    },
    ApprovalRequest {
        id: String,
        name: String,
        tier: Tier,
        summary: String,
        arguments: String,
    },
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
        session_prompt_tokens: u64,
        session_completion_tokens: u64,
    },
    TurnDone {
        content: String,
        steps: usize,
    },
    Error {
        message: String,
    },
}
