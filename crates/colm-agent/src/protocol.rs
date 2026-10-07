//! GUI 与 `colm-agent` 之间的 stdio JSONL 协议：每行一个 JSON 对象，`type` 字段区分。

use serde::{Deserialize, Serialize};

use crate::provider::ProviderConfig;
use crate::tools::Tier;

/// 运行操作（B 类工具）怎么审批。代码操作（C 类）不受影响，始终逐次审批。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// 每次询问；审批卡上可选“本会话不再询问这类操作”。
    #[default]
    Ask,
    /// 不询问，直接执行（工具卡片照常显示，审计日志照常记录）。
    Auto,
}

/// GUI → agent。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    /// 设定（或更换）模型服务与工具环境。Key 不经这里传，agent 自己从本地 Key 文件取。
    Configure {
        provider: ProviderConfig,
        project_root: String,
        #[serde(default)]
        kernel_dir: Option<String>,
        #[serde(default)]
        docs_root: Option<String>,
        #[serde(default)]
        approval: ApprovalPolicy,
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
        /// 批准并在本会话里不再询问同名操作。
        #[serde(default)]
        remember: bool,
    },
    /// 停止当前这一轮（正在生成或在等审批时都可以）。
    Cancel,
    /// 开新会话（清空对话历史，保留配置）。
    NewSession,
    /// 续接一个存过的会话（读回历史，接着聊）。已经是当前会话或没存过时留在当前会话；
    /// 都以 `ready`（带当前会话号）回复。
    ResumeSession { id: String },
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
        /// 按审批策略或本会话的“不再询问”直接放行，没有弹审批。
        #[serde(default)]
        preapproved: bool,
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
