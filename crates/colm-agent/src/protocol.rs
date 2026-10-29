//! GUI 与 `colm-agent` 之间的 stdio JSONL 协议：每行一个 JSON 对象，`type` 字段区分。

use serde::{Deserialize, Serialize};

use crate::provider::ProviderConfig;
use crate::tools::Tier;

/// 一个工具调用执行到哪一步、副作用是什么结局。
///
/// 为什么不从 `ok=false` 或错误文本推断：同一个“失败”既可能是审批被拒（什么都没做），也可能是
/// 远程任务已经提交、只是回执解析失败（可能已部分执行）。恢复时把后者当“没执行”会导致误重试，
/// 把前者当“可能执行”会制造虚假的待核对写。执行器在结束调用时知道的结构事实（是否通过审批、
/// 是否真的调用了工具、工具 tier）足以决定这个枚举，不需要看文案。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    /// 执行前就结束：用户拒绝审批、等审批时取消、执行前取消。确定没有任何副作用。
    NotStarted,
    /// 工具正常返回（返回里说运行失败也算正常返回：动作已经执行完，结局已知）。
    Completed,
    /// 工具失败，但 tier 是只读（A 类）——不可能改动状态，因此确定没有副作用。
    FailedNoEffect,
    /// 有副作用的工具开始执行后失败/panic，或进程中断没来得及记结果：副作用可能不完整。
    /// 保守默认值取它：宁可多核对一次实际状态，不能假设“没执行”。
    #[default]
    EffectUnknown,
}

impl ToolEffect {
    /// 是否需要在恢复/继续时先核对实际状态。只对会改状态的动作有意义。
    pub fn needs_reconciliation(self) -> bool {
        matches!(self, Self::EffectUnknown)
    }
}

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
        provider: Box<ProviderConfig>,
        project_root: String,
        #[serde(default)]
        kernel_dir: Option<String>,
        #[serde(default)]
        docs_root: Option<String>,
        #[serde(default)]
        model_context: Option<Box<crate::tools::ModelContext>>,
        #[serde(default)]
        approval: ApprovalPolicy,
        /// 联网（`web_search` 走 DeepSeek 原生搜索，用 DeepSeek 的 Key；`fetch_url` 读网页）。
        #[serde(default)]
        web_search: bool,
        /// GUI 能被驱动（引导模式的 `ui_*` 工具）。
        #[serde(default)]
        ui: bool,
        /// Headless frontend can answer remote workspace bridge requests.
        #[serde(default)]
        remote: bool,
        /// 用哪个后端回答：内置（默认）、用户本机的 Codex 或 Claude Code。
        #[serde(default)]
        backend: crate::backend::BackendKind,
        /// 外部后端的模型与思考强度（内置后端用 `provider` 里的）。
        #[serde(default)]
        external: crate::backend::ExternalChoice,
        /// 每个会话最多多少条用户消息；0 = 不限（GUI 设置里的会话上限，B-12）。
        #[serde(default = "default_session_turn_limit")]
        session_turn_limit: u32,
        /// 每个会话累计 prompt+completion token 上限；0 = 不限。
        #[serde(default = "default_session_token_limit")]
        session_token_limit: u64,
        /// 可选的 Xerj 文档检索后端；缺省/None = 关闭（旧配置兼容）。http key 不经这里传，
        /// agent 自己从本地 Key 文件按 endpoint 取出。
        #[serde(default)]
        retrieval: Option<crate::retrieval::RetrievalSettings>,
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
    /// GUI 对一次 `ui_request` 的回话。
    UiResult {
        id: String,
        ok: bool,
        #[serde(default)]
        result: serde_json::Value,
    },
    /// 停止当前这一轮（正在生成或在等审批时都可以）。
    Cancel,
    /// 开新会话（清空对话历史，保留配置）。
    NewSession,
    /// 续接一个存过的会话（读回历史，接着聊）。已经是当前会话或没存过时留在当前会话；
    /// 都以 `ready`（带当前会话号）回复。
    ResumeSession { id: String },
    /// U04：用户已明确核对某些“未知副作用”动作。只记录“已核对”声明与备注；不表示动作成功，
    /// 不改写原始 effect_unknown 证据（审计里原有 tool_outcome/tool_result 保留），也不重跑工具。
    /// `ids` 由 agent 校验与去重（非空、≤32、单个 ≤160 字符且不含控制字符）。
    ReconcileEffects {
        ids: Vec<String>,
        #[serde(default)]
        note: Option<String>,
    },
    /// 查询或刷新文档检索索引状态（只在应用设置里触发，不暴露给 LLM）。
    DocsIndex {
        /// "status" | "refresh"；其它值由 agent 报可读错误。
        action: String,
    },
}

/// agent → GUI。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    /// A checkpoint, not a command to replay any recorded action.
    TaskState {
        task: serde_json::Value,
    },
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
    /// 工具执行的副作用结局（审计与恢复用；界面可以不显示）。
    ///
    /// 它和 `tool_result` 是两条独立事件：`tool_result.ok` 只说明“成功/失败”，这条说明“这次失败
    /// 是否留下了副作用”。恢复逻辑用这里的结构值决定是否要求先核对实际状态，不再按 `ok` 或
    /// 错误文案猜。
    ToolOutcome {
        id: String,
        name: String,
        effect: ToolEffect,
    },
    ApprovalRequest {
        id: String,
        name: String,
        tier: Tier,
        summary: String,
        arguments: String,
        #[serde(default)]
        explicit_only: bool,
    },
    /// 请 GUI 在窗口上做一件事（引导模式）；GUI 以 `ui_result` 回话。
    UiRequest {
        id: String,
        action: String,
        args: serde_json::Value,
    },
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
        session_prompt_tokens: u64,
        session_completion_tokens: u64,
        /// 这个会话已经完成的用户轮数；旧事件没有这个字段时按 0 处理。
        #[serde(default)]
        session_turns: u32,
    },
    /// 单次请求的上下文估算；GUI 在接近窗口时提示，并显示自动折叠了多少条旧工具结果。
    Context {
        estimated_tokens: u64,
        window_tokens: u64,
        compressed_tools: usize,
        /// 窗口减去估算输入、输出预留与安全余量后还能容纳的输入估算；None = 未启用预算
        /// （脚本化测试 Provider、外部后端）。
        #[serde(default)]
        available_input_tokens: Option<u64>,
        /// 预算为最终请求体预留的输出 token（与 body 里的有效输出上限一致）；未启用预算时为 0。
        #[serde(default)]
        output_reserve_tokens: u64,
        /// 协议/估算误差的安全余量；未启用预算时为 0。
        #[serde(default)]
        safety_margin_tokens: u64,
        /// "user"（用户填写）| "preset"（文档确认的服务商预设）；未启用预算时 None。
        #[serde(default)]
        window_source: Option<String>,
    },
    TurnDone {
        content: String,
        steps: usize,
        /// 本轮是因为达到生成步数上限而收尾的（不是正常完成）。
        #[serde(default)]
        limit_reached: bool,
        /// 本轮的生成步数上限；旧事件没有时按 0 处理。
        #[serde(default)]
        step_limit: usize,
    },
    /// 可选文档检索后端的索引状态；GUI 在助手设置里显示。不包含服务地址的 key。
    DocsIndexStatus {
        /// 固定 "xerj"。
        backend: String,
        enabled: bool,
        /// disabled|invalid|not_connected|timeout|invalid_response|pending|building|ready|stale|error
        state: String,
        #[serde(default)]
        scope: Option<serde_json::Value>,
        endpoint: String,
        index: String,
        #[serde(default)]
        documents: u64,
        #[serde(default)]
        chunks: u64,
        #[serde(default)]
        generation: Option<u64>,
        #[serde(default)]
        indexed_at_unix_ms: Option<u64>,
        #[serde(default)]
        detail: Option<String>,
    },
    Error {
        message: String,
    },
}

fn default_session_turn_limit() -> u32 {
    100
}

fn default_session_token_limit() -> u64 {
    5_000_000
}
