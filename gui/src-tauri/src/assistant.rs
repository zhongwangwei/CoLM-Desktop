//! AI 助手（docs/design-ai-assistant.md）：启动 `colm-agent` sidecar，经 stdio JSONL 转发消息与事件。
//!
//! 这里只做转发与设置：对话循环、工具、审批规则都在 `colm-agent` 里。API Key 存在应用配置目录的
//! `assistant-keys.json`（三个平台一样，Unix 上 0600），文件格式由 `colm-agent` 管：窗口进程只在
//! “保存 Key”那一刻经 stdin 把它交过去，Key 不回传前端。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

/// 运行中的 `colm-agent`。
#[derive(Default)]
pub struct AssistantProcess {
    inner: Mutex<Option<(Child, ChildStdin)>>,
    ui_requests: Arc<Mutex<PendingUiRequests>>,
}

#[derive(Default)]
struct PendingUiRequests {
    generation: u64,
    requests: std::collections::BTreeMap<String, PendingUiRequest>,
    approvals: std::collections::BTreeMap<String, String>,
}

fn register_approval_request(
    pending: &mut PendingUiRequests,
    generation: u64,
    value: &mut Value,
) -> Result<(), String> {
    if pending.generation != generation {
        return Err("stale assistant process".into());
    }
    let original = value["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 160 && !id.contains(char::is_control))
        .ok_or("invalid approval identity")?
        .to_owned();
    let scoped = format!("{generation}:{original}");
    if pending.approvals.contains_key(&scoped) || pending.approvals.len() >= 128 {
        return Err("duplicate or excessive approval requests".into());
    }
    pending.approvals.insert(scoped.clone(), original);
    value["id"] = json!(scoped);
    Ok(())
}

struct PendingUiRequest {
    original_id: String,
    audit: Option<colm_history::Event>,
}

fn register_ui_request(
    pending: &mut PendingUiRequests,
    generation: u64,
    value: &mut Value,
) -> Result<(), String> {
    if generation != pending.generation {
        return Err("stale assistant process".into());
    }
    let original_id = value["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("UI request has no ID")?
        .to_owned();
    let id = format!("{generation}:{original_id}");
    if pending.requests.contains_key(&id) || pending.requests.len() >= 128 {
        return Err("duplicate or excessive pending UI requests".into());
    }
    let action = value["action"].as_str().ok_or("UI request has no action")?;
    let target = if action == "workspace_remote" {
        value["args"]["operation"].as_str()
    } else {
        value["args"]["target"].as_str()
    };
    let audit = crate::history::assistant_ui_start(&id, action, target)?;
    pending
        .requests
        .insert(id.clone(), PendingUiRequest { original_id, audit });
    value["id"] = json!(id);
    Ok(())
}

impl AssistantProcess {
    /// 应用退出时结束它（stdin 一关它就会自己退出）。
    pub fn stop(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Ok(mut pending) = self.ui_requests.lock() {
                pending.generation = pending.generation.wrapping_add(1);
                pending.requests.clear();
                pending.approvals.clear();
            }
            if let Some((mut child, mut stdin)) = guard.take() {
                // 先发取消、关 stdin，让 agent 自己结束正在跑的命令组（cli_long 轮询 cancel 标志）；
                // 等不到退出再按进程组强杀，避免它启动的运行变成孤儿（GUI 审查 B-07）。
                let _ = writeln!(stdin, "{{\"type\":\"cancel\"}}");
                let _ = stdin.flush();
                drop(stdin);
                let mut exited = false;
                for _ in 0..20 {
                    match child.try_wait() {
                        Ok(Some(_)) => {
                            exited = true;
                            break;
                        }
                        _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                    }
                }
                if !exited {
                    let pid = child.id();
                    if crate::sidecar::terminate_process_tree(pid).is_err() {
                        // 进程组已经退出或没有权限时退回单进程 kill；至少不留这个直接子进程。
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                }
            }
        }
    }
}

/// 应用托管的本机 Xerj 节点。只存在窗口进程内：
/// - 只有用户显式选择 managed 且启用检索时才启动；绝不自动下载/升级二进制；
/// - 数据目录、端口与 `admin.key` 都属于应用自己，退出/停止时只结束自己启动的进程组；
/// - 用户已有的节点走 external 模式，这里不探测也不碰。
#[derive(Default, Clone)]
pub struct ManagedXerj {
    inner: Arc<Mutex<Option<crate::xerj_manager::ManagedNode>>>,
    /// 最近一次启动/停止/版本不符的原因，供设置面板解释；绝不含 key。
    detail: Arc<Mutex<Option<String>>>,
}

impl ManagedXerj {
    /// 当前设置对应的节点状态（设置面板轮询/按钮显示）。external 模式不发起网络探测。
    pub fn status_value(&self, settings: &AssistantSettings) -> Value {
        let pinned = crate::xerj_manager::PINNED_XERJ_RELEASE;
        if settings.retrieval.mode != "managed" {
            return json!({
                "mode": "external",
                "running": false,
                "endpoint": settings.retrieval.endpoint,
                "port": Value::Null,
                "version": Value::Null,
                "pinned_version": pinned,
                "pinned_match": Value::Null,
                "binary": Value::Null,
                "data_dir": Value::Null,
                "detail": "使用用户自行运行的 Xerj 节点；应用不启动也不停止它",
            });
        }
        let status = self
            .inner
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|node| node.status()));
        match status {
            Some(status) => {
                let mut value = serde_json::to_value(status).unwrap_or_default();
                value["mode"] = json!("managed");
                value["pinned_version"] = json!(pinned);
                value
            }
            None => json!({
                "mode": "managed",
                "running": false,
                "endpoint": Value::Null,
                "port": Value::Null,
                "version": Value::Null,
                "pinned_version": pinned,
                "pinned_match": Value::Null,
                "binary": Value::Null,
                "data_dir": Value::Null,
                "detail": self
                    .detail
                    .lock()
                    .ok()
                    .and_then(|detail| detail.clone())
                    .unwrap_or_else(|| "节点未启动；首次发送、保存设置或点「启动节点」时应用会启动它".into()),
            }),
        }
    }

    /// 确保 managed 节点就绪并返回实际 endpoint（external 模式直接返回配置值）。
    /// 同步阻塞（最多 ready_timeout），调用方放在 spawn_blocking 里。启动失败不阻断助手：
    /// 调用方保留 retrieval enabled，让 agent 报 not_connected 并回退关键词检索。
    pub(crate) fn ensure_started(
        &self,
        app: &tauri::AppHandle,
        settings: &AssistantSettings,
    ) -> Result<String, String> {
        if settings.retrieval.mode != "managed" {
            return Ok(settings.retrieval.endpoint.clone());
        }
        {
            let guard = self
                .inner
                .lock()
                .map_err(|_| "Xerj 节点状态锁失效".to_string())?;
            if let Some(node) = guard.as_ref() {
                return Ok(node.endpoint());
            }
        }
        let starts: Vec<PathBuf> = [
            app.path().resource_dir().ok(),
            agent_path().parent().map(PathBuf::from),
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(PathBuf::from)),
        ]
        .into_iter()
        .flatten()
        .collect();
        let config = crate::xerj_manager::ManagedConfig {
            binary_path: settings.retrieval.binary_path.clone(),
            // 按固定 Xerj 版本隔离数据目录：应用升级换随包版本时，新版本从空索引开始重建，
            // 绝不复用旧版本写下的索引/数据（Xerj 的存储格式不保证跨版本兼容）。
            data_dir: data_dir(app)?.join("xerj").join(xerj_data_dir_name()),
            preferred_port: settings.retrieval.port,
            extra_binary_dirs: starts,
            ready_timeout: crate::xerj_manager::DEFAULT_READY_TIMEOUT,
        };
        let node = crate::xerj_manager::ManagedNode::start(config).inspect_err(|error| {
            if let Ok(mut detail) = self.detail.lock() {
                *detail = Some(error.clone());
            }
        })?;
        let endpoint = node.endpoint();
        // 应用自己数据目录里的 admin.key 交给 agent 的现有 secrets（按 endpoint 存），
        // key 值只经 stdin，不经过 WebView、设置文件或日志。
        store_retrieval_key(app, &endpoint, &node.api_key())?;
        if let Ok(mut detail) = self.detail.lock() {
            *detail = if node.pinned_match() == Some(false) {
                Some(format!(
                    "节点版本 {} 与验收固定版本 {} 不同：可继续使用，但需人工替换为固定版本并重跑验收；应用不会自动升级",
                    node.version().unwrap_or_else(|| "unknown".into()),
                    crate::xerj_manager::PINNED_XERJ_RELEASE,
                ))
            } else {
                None
            };
        }
        if let Ok(mut guard) = self.inner.lock() {
            *guard = Some(node);
        }
        Ok(endpoint)
    }

    pub(crate) fn stop(&self) {
        let node = self.inner.lock().ok().and_then(|mut guard| guard.take());
        if let Some(mut node) = node {
            if let Err(error) = node.stop() {
                if let Ok(mut detail) = self.detail.lock() {
                    *detail = Some(error);
                }
            }
        }
    }

    /// 供 `spawn_blocking` 使用的独立句柄（与 Tauri state 共享同一个节点与该节点内的 Arc）。
    pub(crate) fn handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            detail: Arc::clone(&self.detail),
        }
    }
}

/// 托管节点数据目录的版本子目录名：只保留文件系统安全字符，其余替换为 `_`。
/// 应用升级换了随包 Xerj 版本时必须用新目录（旧索引/存储格式不保证兼容），
/// 这一层与 `xerj_manager` 的二进制探测共同实现“装好即用、版本不混用”。
pub(crate) fn xerj_data_dir_name() -> String {
    crate::xerj_manager::PINNED_XERJ_RELEASE
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// 把应用托管的 Xerj key 交给 `colm-agent` 写进本地 Key 文件（与 assistant_set_key 同一条路径）。
fn store_retrieval_key(app: &tauri::AppHandle, endpoint: &str, key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("托管 Xerj 节点没有生成 API key".into());
    }
    let mut command = key_command(app, "--set-key", endpoint)?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(format!("{}\n", key.trim()).as_bytes())
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Xerj 文档检索设置（应用级，不随 API 服务商 profile 切换）。默认关闭；字段与
/// `crates/colm-agent/src/retrieval.rs` 的 `RetrievalSettings` 一一对应（mode/binary_path/port
/// 只由窗口进程使用，不传给 agent），旧 `assistant.json` 没有这个对象时整份按默认关闭加载。
/// Key 不在这里：external 模式沿用 `assistant-keys.json` 按 endpoint 登记；managed 模式由应用
/// 自己读取应用数据目录里的 `admin.key` 并登记，不需要用户输入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalSettings {
    #[serde(default)]
    pub enabled: bool,
    /// "external"（默认，使用用户自己运行的节点）或 "managed"（应用启动并托管自己的 Xerj）。
    #[serde(default = "default_retrieval_mode")]
    pub mode: String,
    #[serde(default = "default_retrieval_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_retrieval_index")]
    pub index: String,
    #[serde(default = "default_retrieval_timeout_ms")]
    pub timeout_ms: u64,
    /// managed 模式的可选 Xerj 二进制路径；空 = 依次探测 COLM_XERJ_BINARY、资源目录、PATH。
    #[serde(default)]
    pub binary_path: Option<String>,
    /// managed 模式的首选端口；空 = 自动选连续 3 个空闲 loopback 端口（Xerj 还会占用 +1/+2）。
    #[serde(default)]
    pub port: Option<u16>,
}

impl Default for RetrievalSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_retrieval_mode(),
            endpoint: default_retrieval_endpoint(),
            index: default_retrieval_index(),
            timeout_ms: default_retrieval_timeout_ms(),
            binary_path: None,
            port: None,
        }
    }
}

fn default_retrieval_mode() -> String {
    "external".into()
}

fn default_retrieval_endpoint() -> String {
    "http://127.0.0.1:9200".into()
}

fn default_retrieval_index() -> String {
    "colm-docs-v1".into()
}

fn default_retrieval_timeout_ms() -> u64 {
    1500
}

/// 助手设置（不含 Key），存在配置目录的 `assistant.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantSettings {
    pub base_url: String,
    pub model: String,
    /// 当前端点/模型/协议的上下文窗口（token）。None = 用文档确认的预设，未知模型则发送前要求填写。
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default = "default_api_format")]
    pub api_format: String,
    #[serde(default = "default_api_options")]
    pub api_options: Value,
    #[serde(default = "default_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub api_profiles: std::collections::BTreeMap<String, ApiProfile>,
    /// 服务商对应的思考开关；`None` 用服务端默认。
    #[serde(default)]
    pub thinking: Option<bool>,
    /// 服务商和模型对应的思考强度；`None` 用服务端默认。
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 运行操作的审批：`ask` 每次询问（审批卡可选本会话不再询问），`auto` 直接执行。
    #[serde(default = "default_approval")]
    pub approval: String,
    /// 联网搜索（DeepSeek 原生搜索，用 DeepSeek 的 Key）与读网页。
    #[serde(default = "default_web_search")]
    pub web_search: bool,
    /// 每个会话最多多少条用户消息；0 = 不限（B-12 的会话预算）。
    #[serde(default = "default_session_turn_limit")]
    pub session_turn_limit: u32,
    /// 每个会话累计 prompt+completion token 上限；0 = 不限。
    #[serde(default = "default_session_token_limit")]
    pub session_token_limit: u64,
    /// 后端：API 服务或用户本机配置的 Codex、Claude Code、OpenCode。
    #[serde(default = "default_backend")]
    pub backend: String,
    /// Xerj 文档检索（可选，应用级）：关闭时完全不访问本机 Xerj。
    #[serde(default)]
    pub retrieval: RetrievalSettings,
    /// 用户已确认过“数据会发给模型服务商”的那个服务地址。
    #[serde(default)]
    pub egress_acknowledged: Option<String>,
    /// 外部后端各自的模型与思考强度；OpenCode 使用 provider/model 与模型 variant。
    #[serde(default)]
    pub external: std::collections::BTreeMap<String, ExternalChoice>,
}

/// Each API service retains its own editable configuration; credentials stay in the key file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiProfile {
    pub base_url: String,
    pub model: String,
    /// 这份配置自己的窗口；直接改模型/地址/协议时前端清空，只有恢复完整匹配的 profile 才取回。
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub thinking: Option<bool>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default = "default_api_format")]
    pub api_format: String,
    #[serde(default = "default_api_options")]
    pub api_options: Value,
    #[serde(default = "default_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub strict: bool,
}

fn default_api_format() -> String {
    "chat_completions".into()
}
fn default_api_options() -> Value {
    json!({})
}
fn default_output_tokens() -> u32 {
    16384
}
fn default_timeout() -> u64 {
    600
}

impl AssistantSettings {
    fn api_profile(&self) -> ApiProfile {
        ApiProfile {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            context_window: self.context_window,
            thinking: self.thinking,
            reasoning_effort: self.reasoning_effort.clone(),
            api_format: self.api_format.clone(),
            api_options: self.api_options.clone(),
            max_output_tokens: self.max_output_tokens,
            timeout_seconds: self.timeout_seconds,
            strict: self.strict,
        }
    }
}

/// 外部后端的模型与思考强度。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalChoice {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-flash".into(),
            context_window: None,
            provider_id: "deepseek".into(),
            api_format: default_api_format(),
            api_options: default_api_options(),
            max_output_tokens: default_output_tokens(),
            timeout_seconds: default_timeout(),
            strict: false,
            api_profiles: Default::default(),
            thinking: None,
            reasoning_effort: None,
            approval: default_approval(),
            web_search: default_web_search(),
            session_turn_limit: default_session_turn_limit(),
            session_token_limit: default_session_token_limit(),
            backend: default_backend(),
            retrieval: RetrievalSettings::default(),
            egress_acknowledged: None,
            external: Default::default(),
        }
    }
}

fn default_approval() -> String {
    "ask".into()
}

fn default_web_search() -> bool {
    true
}

fn default_session_turn_limit() -> u32 {
    100
}

fn default_session_token_limit() -> u64 {
    5_000_000
}

fn default_backend() -> String {
    "builtin".into()
}

const BACKENDS: [&str; 4] = ["builtin", "codex", "claude_code", "opencode"];

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.join("assistant.json"))
}

/// 本地 Key 文件。
fn key_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(settings_path(app)?.with_file_name("assistant-keys.json"))
}

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("找不到数据目录：{e}"))?
        .join("assistant");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// `colm-agent` 与 `colm-cli` 放在一起。
fn agent_path() -> PathBuf {
    let name = if cfg!(windows) {
        "colm-agent.exe"
    } else {
        "colm-agent"
    };
    let cli = crate::sidecar::resolve_cli();
    match cli.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}

const PROVIDERS: [&str; 9] = [
    "deepseek",
    "openai",
    "anthropic",
    "grok",
    "glm",
    "gemini",
    "kimi",
    "qwen",
    "custom",
];

pub(crate) fn validate_api_address(base_url: &str) -> Result<(), String> {
    let url = tauri::Url::parse(base_url.trim()).map_err(|_| "服务地址不是有效的网址")?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if url.host_str().is_none() || !(url.scheme() == "https" || (url.scheme() == "http" && local)) {
        return Err("服务地址必须是 https://，或本机的 http://127.0.0.1 / localhost".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("服务地址不能含用户名、密码、查询参数或片段；API Key 单独保存".into());
    }
    Ok(())
}

fn validate_api_profile(profile: &ApiProfile) -> Result<(), String> {
    validate_api_address(&profile.base_url)?;
    if profile.model.trim().is_empty()
        || profile.model.len() > 256
        || profile.model.contains(char::is_whitespace)
    {
        return Err("请填写不含空白的模型名（最多 256 字节）".into());
    }
    if !matches!(
        profile.api_format.as_str(),
        "chat_completions" | "responses" | "anthropic"
    ) {
        return Err("不支持这个 API 接口格式".into());
    }
    if !(1..=3600).contains(&profile.timeout_seconds)
        || !(1..=1_000_000).contains(&profile.max_output_tokens)
    {
        return Err("超时须为 1–3600 秒，最大输出须为 1–1000000 token".into());
    }
    // 窗口是预算的唯一依据，空着表示交给内置的已确认预设；显式值必须有合理下界，
    // 否则输出预留加安全余量就占满窗口，任何请求都发不出去。
    if profile
        .context_window
        .is_some_and(|window| !(1_024..=10_000_000).contains(&window))
    {
        return Err("上下文窗口须为 1024–10000000 token，留空表示使用内置预设".into());
    }
    if profile.reasoning_effort.as_ref().is_some_and(|e| {
        e.is_empty() || e.len() > 32 || !e.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    }) {
        return Err("思考程度须为不超过 32 字节的英文档位名".into());
    }
    let options = profile
        .api_options
        .as_object()
        .ok_or("额外参数须为 JSON 对象")?;
    if profile.api_options.to_string().len() > 16384 {
        return Err("额外参数超过 16 KiB".into());
    }
    if options.keys().any(|key| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "model"
                | "messages"
                | "input"
                | "instructions"
                | "system"
                | "tools"
                | "stream"
                | "api_key"
                | "authorization"
                | "headers"
                | "store"
                | "include"
                | "previous_response_id"
        )
    }) {
        return Err("额外参数不能覆盖模型、对话、工具、流式设置或凭据".into());
    }
    Ok(())
}

fn validate_api_reasoning(profile: &ApiProfile, provider: &str) -> Result<(), String> {
    let url = tauri::Url::parse(&profile.base_url).map_err(|_| "服务地址不是有效的网址")?;
    let provider = if provider.is_empty() {
        match url.host_str() {
            Some("api.openai.com") => "openai",
            Some("api.anthropic.com") => "anthropic",
            Some("generativelanguage.googleapis.com") => "gemini",
            Some("open.bigmodel.cn" | "api.z.ai") => "glm",
            Some("api.moonshot.cn" | "api.moonshot.ai") => "kimi",
            Some(host)
                if host == "dashscope.aliyuncs.com"
                    || host == "dashscope-intl.aliyuncs.com"
                    || host.ends_with(".dashscope.aliyuncs.com")
                    || host.ends_with(".maas.aliyuncs.com") =>
            {
                "qwen"
            }
            _ => "custom",
        }
    } else {
        provider
    };
    let model = profile.model.as_str();
    let sol = provider == "openai" && model == "gpt-6.1-sol";
    let sonnet = provider == "anthropic" && model == "claude-sonnet-5-5";
    let gemini = provider == "gemini" && matches!(model, "gemini-3.7-flash" | "gemini-3.8-flash");
    let glm = provider == "glm" && matches!(model, "glm-5.3" | "glm-5.3-flash" | "glm-5.3-flashx");
    let kimi = provider == "kimi" && model == "kimi-k3";
    let qwen = provider == "qwen"
        && matches!(
            model,
            "qwen3.8-max"
                | "qwen3.8-max-0902"
                | "qwen3.8-flash"
                | "qwen3.8-2.4t-a95b"
                | "qwen3.8-27b"
                | "qwen3.8-omni-flash"
        );
    if (sol || sonnet || gemini || glm || kimi || (qwen && model == "qwen3.8-2.4t-a95b"))
        && profile.thinking == Some(false)
    {
        return Err(format!(
            "{model} 不支持关闭思考，请改用服务默认或支持的思考强度"
        ));
    }
    if sol && profile.api_format != "responses" {
        return Err("gpt-6.1-sol 的助手工具调用须使用 Responses 接口".into());
    }
    if sol
        && ["temperature", "top_p", "top_logprobs", "logprobs"]
            .iter()
            .any(|key| profile.api_options.get(*key).is_some())
    {
        return Err(
            "gpt-6.1-sol 的思考模式不支持 temperature、top_p 或 logprobs 等采样选项，请移除".into(),
        );
    }
    if sonnet && profile.api_format != "anthropic" {
        return Err("claude-sonnet-5-5 须使用 Anthropic Messages 接口".into());
    }
    let efforts: &[&str] = if sol || sonnet {
        &["low", "medium", "high", "xhigh", "max"]
    } else if gemini {
        &["low", "medium", "high"]
    } else if glm || kimi {
        &["low", "high", "max"]
    } else if qwen {
        &["none", "minimal", "low", "medium", "high", "xhigh", "max"]
    } else {
        return Ok(());
    };
    let extra_effort = match profile.api_format.as_str() {
        "responses" => &profile.api_options["reasoning"]["effort"],
        "anthropic" => &profile.api_options["output_config"]["effort"],
        _ => &profile.api_options["reasoning_effort"],
    };
    if !extra_effort.is_null() && !extra_effort.is_string() {
        return Err("额外参数的思考强度须为字符串".into());
    }
    for effort in [profile.reasoning_effort.as_deref(), extra_effort.as_str()]
        .into_iter()
        .flatten()
    {
        if !efforts.contains(&effort) {
            return Err(format!(
                "{model} 不支持思考强度 {effort:?}；可用：{}",
                efforts.join("、")
            ));
        }
    }
    let effective_effort = extra_effort
        .as_str()
        .or(profile.reasoning_effort.as_deref());
    let thinking = profile.api_options["thinking"]["type"].as_str();
    if sonnet {
        if profile.api_options.get("tool_choice").is_some()
            && !matches!(
                profile.api_options["tool_choice"]["type"].as_str(),
                Some("auto" | "none")
            )
        {
            return Err(
                "Claude Sonnet 5.5 的 tool_choice 仅支持 auto 或 none，不支持 any 或强制指定工具"
                    .into(),
            );
        }
        if profile
            .api_options
            .get("temperature")
            .is_some_and(|value| value.as_f64() != Some(1.0))
        {
            return Err("Claude Sonnet 5.5 仅接受默认温度，请省略 temperature 或设为 1".into());
        }
        if thinking.is_some_and(|mode| !matches!(mode, "adaptive" | "between_tools")) {
            return Err(
                "Claude Sonnet 5.5 仅支持 adaptive 或 between_tools，不能使用 disabled".into(),
            );
        }
        if thinking == Some("between_tools") && matches!(effective_effort, Some("xhigh" | "max")) {
            return Err("Claude Sonnet 5.5 的 between_tools 仅支持 low、medium、high".into());
        }
    } else if (glm || kimi) && thinking == Some("disabled") {
        return Err(format!("{model} 不支持关闭思考"));
    }
    if gemini {
        let config = profile.api_options["extra_body"]["google"]
            .get("thinking_config")
            .or_else(|| profile.api_options["google"].get("thinking_config"));
        if let Some(level) = config.and_then(|config| config.get("thinking_level")) {
            if !matches!(level.as_str(), Some("low" | "medium" | "high")) {
                return Err(format!(
                    "{model} 的 thinking_level 仅支持 low、medium、high"
                ));
            }
        }
    }
    if qwen {
        let sends_effort = profile.thinking == Some(false) || effective_effort.is_some();
        if sends_effort && profile.api_options.get("thinking_budget").is_some() {
            return Err("Qwen 3.8 的 reasoning_effort 不能与 thinking_budget 同时设置".into());
        }
        if profile.thinking == Some(false)
            && extra_effort.as_str().is_some_and(|effort| effort != "none")
        {
            return Err("Qwen 3.8 关闭思考与额外思考强度冲突；关闭请用 none".into());
        }
        if model == "qwen3.8-2.4t-a95b"
            && (effective_effort == Some("none") || profile.api_options["enable_thinking"] == false)
        {
            return Err("qwen3.8-2.4t-a95b 不支持关闭思考".into());
        }
    }
    Ok(())
}

/// Claude Code 的 `--effort` 可选值（Claude Code 2.1.293）；Codex 的可选值随模型而定，由 `model/list` 给出。
const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// 优先使用当前应用资源中的文档；开发运行才从二进制和工作目录向上找仓库。
pub(crate) fn docs_root(starts: &[PathBuf]) -> Option<PathBuf> {
    let is_docs = |docs: &PathBuf| docs.join("design-ai-assistant.md").is_file();
    starts
        .iter()
        .flat_map(|start| {
            [
                Some(start.join("docs")),
                start.parent().map(|p| p.join("Resources/docs")),
            ]
        })
        .flatten()
        .find(is_docs)
        .or_else(|| {
            starts
                .iter()
                .find_map(|start| start.ancestors().map(|dir| dir.join("docs")).find(is_docs))
        })
}

/// 设置校验：服务地址只接受 https，或本机回环的 http（本地模型）。
/// 数据外发确认的规范化目标：必须与前端 `egressTarget` 完全一致。内置后端联网时额外写明
/// 搜索请求会发往 DeepSeek Search API；外部后端用它们自带的搜索/模型，目标就是后端本身。
pub(crate) fn egress_target(settings: &AssistantSettings) -> String {
    let model = match settings.backend.as_str() {
        "builtin" => settings.base_url.clone(),
        "opencode" => format!(
            "opencode:{}",
            settings
                .external
                .get("opencode")
                .and_then(|choice| choice.model.clone())
                .unwrap_or_default()
        ),
        backend => backend.to_owned(),
    };
    if settings.backend == "builtin" && settings.web_search {
        format!("{model} (web search & fetch: DeepSeek APIs)")
    } else {
        model
    }
}

/// 后端也要校验数据外发确认：只靠前端调用点的话，绕过面板的直接 invoke（e2e、脚本、未来调用）
/// 就能在用户没看过提示时把消息和页面上下文发出去（GUI 审查 B-03）。
pub(crate) fn require_egress_acknowledged(settings: &AssistantSettings) -> Result<(), String> {
    let target = egress_target(settings);
    if settings.egress_acknowledged.as_deref() != Some(target.as_str()) {
        return Err("请先确认数据外发目标（模型服务与联网搜索）后再使用助手。".into());
    }
    Ok(())
}

/// Xerj 文档检索的格式校验（与 `crates/colm-agent/src/retrieval.rs` 的 `validate` 同一套规则）。
/// 只连本机回环的 http，且必须写明端口：9200 不是 http 的默认端口，漏写端口会打到别的服务上。
/// 即使当前关闭也校验 —— 关着时存进非法值、等到打开才发现就太晚了。
fn validate_retrieval(settings: &RetrievalSettings) -> Result<(), String> {
    let url =
        tauri::Url::parse(settings.endpoint.trim()).map_err(|_| "Xerj 服务地址不是有效的网址")?;
    if url.scheme() != "http"
        || !matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        )
    {
        return Err(
            "Xerj 服务地址只能是本机回环的 http://127.0.0.1、http://localhost 或 http://[::1]"
                .into(),
        );
    }
    if url.port().is_none() {
        return Err("Xerj 服务地址必须写明端口，例如 http://127.0.0.1:9200".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err("Xerj 服务地址不能含用户名、密码、路径、查询参数或片段（可带末尾斜杠）".into());
    }
    // 索引名会被拼进 URL 和 ES 索引，只允许 ES 安全字符；首字符不能是 -/_（避免被当成选项或特殊名）。
    let index = settings.index.as_str();
    let index_valid = index.len() <= 64
        && index
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && index.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
    if !index_valid {
        return Err(
            "Xerj 索引名只能含小写字母、数字、下划线或连字符，1–64 字节，且以字母或数字开头".into(),
        );
    }
    if !(100..=10_000).contains(&settings.timeout_ms) {
        return Err("Xerj 检索超时须为 100–10000 毫秒".into());
    }
    if !matches!(settings.mode.as_str(), "external" | "managed") {
        return Err("Xerj 接入方式只能是「使用已运行的本机节点」或「由应用启动并管理节点」".into());
    }
    if let Some(path) = settings.binary_path.as_deref() {
        let path = path.trim();
        if path.is_empty() || path.chars().count() > 4096 || path.chars().any(char::is_control) {
            return Err("Xerj 二进制路径不能为空、不能含控制字符（最多 4096 字符）".into());
        }
        if !std::path::Path::new(path).is_absolute() {
            return Err("Xerj 二进制路径须为绝对路径；留空则由应用自动探测".into());
        }
    }
    if let Some(port) = settings.port {
        if !(1024..=65_000).contains(&port) {
            return Err("Xerj 首选端口须为 1024–65000 的整数；留空则自动选择".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_settings(settings: &AssistantSettings) -> Result<(), String> {
    validate_api_profile(&settings.api_profile())?;
    if !settings.provider_id.is_empty() && !PROVIDERS.contains(&settings.provider_id.as_str()) {
        return Err("没有这个 API 服务预设；其他服务请选择自定义".into());
    }
    for (id, profile) in &settings.api_profiles {
        if !PROVIDERS.contains(&id.as_str()) {
            return Err(format!("没有这个 API 服务预设：{id}"));
        }
        validate_api_profile(profile)?;
    }
    if !BACKENDS.contains(&settings.backend.as_str()) {
        return Err(format!("后端只能是 {}", BACKENDS.join("、")));
    }
    if settings.backend == "builtin" {
        validate_api_reasoning(&settings.api_profile(), &settings.provider_id)?;
    }
    if !matches!(settings.approval.as_str(), "ask" | "auto") {
        return Err("审批方式只能是 ask 或 auto".into());
    }
    if settings.session_turn_limit > 100_000 || settings.session_token_limit > 1_000_000_000 {
        return Err("会话上限过大".into());
    }
    validate_retrieval(&settings.retrieval)?;
    for (backend, choice) in &settings.external {
        if !matches!(backend.as_str(), "codex" | "claude_code" | "opencode") {
            return Err(format!("没有这个外部后端：{backend}"));
        }
        // 这两个值会成为 CLI 的参数，不能像选项、不能含空白。
        for value in [&choice.model, &choice.effort].into_iter().flatten() {
            if value.is_empty() || value.starts_with('-') || value.contains(char::is_whitespace) {
                return Err(format!("模型名或思考强度不合法：{value:?}"));
            }
        }
        if backend == "claude_code" {
            if let Some(effort) = &choice.effort {
                if !CLAUDE_EFFORTS.contains(&effort.as_str()) {
                    return Err(format!(
                        "Claude Code 的思考强度只能是 {}",
                        CLAUDE_EFFORTS.join("、")
                    ));
                }
            }
        }
        if backend == "opencode" {
            if let Some(model) = &choice.model {
                if !valid_opencode_model(model) {
                    return Err("OpenCode 模型须为 provider/model".into());
                }
            }
        }
    }
    if settings.backend == "opencode"
        && settings
            .external
            .get("opencode")
            .and_then(|c| c.model.as_deref())
            .is_none()
    {
        return Err("请先选择 OpenCode 模型。".into());
    }
    Ok(())
}

fn valid_opencode_model(model: &str) -> bool {
    model.len() <= 256
        && model
            .split_once('/')
            .is_some_and(|(provider, name)| !provider.is_empty() && !name.is_empty())
        && !model.contains(['?', '#', '\\'])
}

#[tauri::command]
pub fn assistant_settings(app: tauri::AppHandle) -> AssistantSettings {
    settings_path(&app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn assistant_save_settings(
    app: tauri::AppHandle,
    settings: AssistantSettings,
) -> Result<(), String> {
    validate_settings(&settings)?;
    let path = settings_path(&app)?;
    let event = crate::history::start(
        "assistant_settings",
        None,
        json!({
            "backend":settings.backend,"api_format":settings.api_format,"model":settings.model,
            "approval":settings.approval,
        }),
    )?;
    // 与 remote.json 同理：替换而不是原地截断重写。
    let result = crate::config::write_export_text_atomic(
        &path,
        serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?,
    );
    finish_assistant_action(&event, result)
}

fn finish_assistant_action(
    event: &colm_history::Event,
    result: Result<(), String>,
) -> Result<(), String> {
    let mut end = event.next(
        colm_history::EventPhase::Finished,
        if result.is_ok() {
            "succeeded"
        } else {
            "failed"
        },
    );
    // Transport errors may contain secrets; the UI gets the original error, the ledger only its presence.
    if result.is_err() {
        end.detail["error_present"] = json!(true);
    }
    colm_history::History::open_default()
        .and_then(|history| history.append(&end))
        .map_err(|e| format!("assistant action outcome could not be recorded: {e}"))?;
    result
}

fn key_command(
    app: &tauri::AppHandle,
    flag: &str,
    base_url: &str,
) -> Result<std::process::Command, String> {
    let mut command = std::process::Command::new(agent_path());
    command
        .arg("--key-file")
        .arg(key_file(app)?)
        .args([flag, base_url]);
    colm_kernel::run::no_console(&mut command);
    Ok(command)
}

/// 用 `colm-agent` 的会话子命令读写历史（不需要 Key，助手进程不在也能用）。
async fn session_command(app: &tauri::AppHandle, args: Vec<String>) -> Result<String, String> {
    let mut command = std::process::Command::new(agent_path());
    command.arg("--data-dir").arg(data_dir(app)?).args(args);
    colm_kernel::run::no_console(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let output = command
            .output()
            .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读一份附件最多等这么久：64 MB、300 页的 PDF 在本机几十秒内能抽完，旧 `.doc` 转换器自己另有 60 s 上限。
const DOCUMENT_EXTRACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// 本地文档附件：后端弹文件对话框，让 `colm-agent` 抽用户亲自选的那份 PDF/DOCX/DOC/文本，原件不经过网络。
/// 返回 `{path, name, title, text, pages, truncated, warnings}`；取消时返回空。路径不由前端传入：原来的
/// `assistant_extract_document(path)` 收任意路径、认 `.json`，前端就能把明文存 Key 的 `assistant-keys.json`
/// 抽出来发给模型服务（DeepSeek 改动复查，同 `pick_parameter_file`）。
#[tauri::command]
pub async fn assistant_pick_document(app: tauri::AppHandle) -> Result<Option<Value>, String> {
    let Some(path) = crate::recent::pick_file(
        app,
        "assistant-attachment".into(),
        Some("pdf,docx,doc,txt,md,markdown,csv,json".into()),
    )
    .await
    else {
        return Ok(None);
    };
    let mut command = std::process::Command::new(agent_path());
    command
        .arg("--extract-document")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    colm_kernel::run::top_level_sidecar(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let child = command
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
        // 抽取要有上限：损坏或巨大的 PDF、卡住的 .doc 转换器会让附件一直"读取中"（GUI 审查 D-07）。
        let pid = child.id();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        let output = match receiver.recv_timeout(DOCUMENT_EXTRACT_TIMEOUT) {
            Ok(output) => output.map_err(|e| e.to_string())?,
            Err(_) => {
                let _ = crate::sidecar::terminate_process_tree(pid);
                return Err(format!(
                    "reading the document took longer than {} seconds and was stopped",
                    DOCUMENT_EXTRACT_TIMEOUT.as_secs()
                ));
            }
        };
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        let mut document: Value =
            serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        document["path"] = Value::String(path);
        document["name"] = Value::String(name);
        Ok(Some(document))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 本机外部后端的安装与配置状态（问 CLI 自己，不读凭据）。
#[tauri::command]
pub async fn assistant_backend_status(app: tauri::AppHandle) -> Result<Value, String> {
    let mut command = std::process::Command::new(agent_path());
    command
        .arg("--backend-status")
        .arg("--data-dir")
        .arg(data_dir(&app)?);
    colm_kernel::run::no_console(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let output = command
            .output()
            .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 文档索引操作只有两种：查状态、强制重建。其它值来自前端拼错或旧调用方，直接拒绝，
/// 不要把它们当成"未知命令"悄悄发给 agent。
fn validate_docs_index_action(action: &str) -> Result<(), String> {
    if matches!(action, "status" | "refresh") {
        Ok(())
    } else {
        Err(format!(
            "文档索引操作只能是 status 或 refresh，收到 {action:?}"
        ))
    }
}

fn docs_index_message(action: &str) -> Value {
    json!({ "type": "docs_index", "action": action })
}

/// 让助手查询/重建 Xerj 文档索引。GUI 只发请求，构建与 HTTP 都在 `colm-agent` 里；
/// 结果经 `assistant://event` 的 `docs_index_status` 回来，前端不轮询。
#[tauri::command]
pub fn assistant_docs_index(
    process: tauri::State<'_, AssistantProcess>,
    action: String,
) -> Result<(), String> {
    validate_docs_index_action(&action)?;
    let event = crate::history::start("assistant_docs_index", None, json!({"action": &action}))?;
    let result = send(&process, &docs_index_message(&action));
    finish_assistant_action(&event, result)
}

/// 托管节点操作只有三种：查状态、启动、停止。拼错或旧调用方直接拒绝，不静默当成 status。
fn validate_xerj_node_action(action: &str) -> Result<(), String> {
    if matches!(action, "status" | "start" | "stop") {
        Ok(())
    } else {
        Err(format!(
            "Xerj 节点操作只能是 start、stop 或 status，收到 {action:?}"
        ))
    }
}

/// 应用托管的 Xerj 节点：查询状态、启动、停止。managed 模式由窗口进程启动固定版本二进制并
/// 自己保管 `admin.key`；版本不符只提示不阻断，绝不自动下载/替换/升级。external 模式不探测用户节点。
#[tauri::command]
pub async fn assistant_xerj_node(
    app: tauri::AppHandle,
    managed: tauri::State<'_, ManagedXerj>,
    action: String,
) -> Result<Value, String> {
    validate_xerj_node_action(&action)?;
    let settings = assistant_settings(app.clone());
    match action.as_str() {
        "status" => Ok(managed.status_value(&settings)),
        "start" => {
            if settings.retrieval.mode != "managed" {
                return Err(
                    "当前接入方式是「使用已运行的本机节点」；先选择「由应用启动并管理节点」再启动"
                        .into(),
                );
            }
            let node = managed.handle();
            let settings_for_start = settings.clone();
            let app_for_start = app.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                node.ensure_started(&app_for_start, &settings_for_start)
            })
            .await
            .map_err(|e| e.to_string())?;
            result?;
            Ok(managed.status_value(&assistant_settings(app)))
        }
        "stop" => {
            managed.stop();
            Ok(managed.status_value(&settings))
        }
        other => Err(format!(
            "Xerj 节点操作只能是 start、stop 或 status，收到 {other:?}"
        )),
    }
}

/// 本机 Codex 可用的模型与各自支持的思考强度（`colm-agent --codex-models` 问 Codex 自己）。
#[tauri::command]
pub async fn assistant_codex_models(app: tauri::AppHandle) -> Result<Value, String> {
    external_models(&app, "--codex-models").await
}

/// OpenCode 已配置的服务商模型与 variant；不发送模型请求。
#[tauri::command]
pub async fn assistant_opencode_models(app: tauri::AppHandle) -> Result<Value, String> {
    external_models(&app, "--opencode-models").await
}

async fn external_models(app: &tauri::AppHandle, flag: &str) -> Result<Value, String> {
    let mut command = std::process::Command::new(agent_path());
    command.arg(flag).arg("--data-dir").arg(data_dir(app)?);
    colm_kernel::run::no_console(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let output = command
            .output()
            .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 历史会话列表，最近的在前。
#[tauri::command]
pub async fn assistant_sessions(app: tauri::AppHandle) -> Result<Value, String> {
    let text = session_command(&app, vec!["--list-sessions".into()]).await?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// 从已保存的会话读取最近发送的用户输入（从旧到新）。
#[tauri::command]
pub async fn assistant_input_history(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let text = session_command(&app, vec!["--input-history".into()]).await?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// 一个历史会话的对话记录。
#[tauri::command]
pub async fn assistant_transcript(app: tauri::AppHandle, id: String) -> Result<Value, String> {
    let text = session_command(&app, vec!["--transcript".into(), id]).await?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assistant_delete_session(app: tauri::AppHandle, id: String) -> Result<(), String> {
    session_command(&app, vec!["--delete-session".into(), id])
        .await
        .map(|_| ())
}

/// 引导模式：把窗口执行界面请求的结果回给助手。
#[tauri::command]
pub fn assistant_ui_result(
    process: tauri::State<'_, AssistantProcess>,
    id: String,
    ok: bool,
    result: Value,
) -> Result<(), String> {
    let (generation, request) = {
        let mut pending = process.ui_requests.lock().map_err(|e| e.to_string())?;
        let request = pending
            .requests
            .remove(&id)
            .ok_or("unregistered or already answered UI request")?;
        (pending.generation, request)
    };
    send_in_generation(
        &process,
        Some(generation),
        &json!({ "type": "ui_result", "id": request.original_id, "ok": ok, "result": result }),
    )?;
    if let Some(event) = request.audit {
        crate::history::assistant_ui_finish(&event, ok, &result)?;
    }
    Ok(())
}

/// 让正在运行的助手续接一个历史会话。
#[tauri::command]
pub fn assistant_resume(
    process: tauri::State<'_, AssistantProcess>,
    id: String,
) -> Result<(), String> {
    send(&process, &json!({ "type": "resume_session", "id": id }))
}

/// 把 Key 交给 `colm-agent` 存进本地 Key 文件（经 stdin，不出现在命令行或日志里）。
/// 存好后结束正在运行的助手进程：它缓存着旧 Key，下次发送时用新 Key 重启。
#[tauri::command]
pub async fn assistant_set_key(
    app: tauri::AppHandle,
    process: tauri::State<'_, AssistantProcess>,
    base_url: String,
    key: String,
) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("Key 是空的".into());
    }
    validate_api_address(&base_url)?;
    let mut command = key_command(&app, "--set-key", &base_url)?;
    let event = crate::history::start("assistant_key", None, json!({"action":"set"}))?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
        child
            .stdin
            .take()
            .ok_or("no stdin")?
            .write_all(format!("{}\n", key.trim()).as_bytes())
            .map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    if result.is_ok() {
        process.stop();
    }
    finish_assistant_action(&event, result)
}

#[tauri::command]
pub async fn assistant_has_key(app: tauri::AppHandle, base_url: String) -> Result<bool, String> {
    validate_api_address(&base_url)?;
    let mut command = key_command(&app, "--has-key", &base_url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let output = command
            .output()
            .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn assistant_delete_key(
    app: tauri::AppHandle,
    process: tauri::State<'_, AssistantProcess>,
    base_url: String,
) -> Result<(), String> {
    validate_api_address(&base_url)?;
    let mut command = key_command(&app, "--delete-key", &base_url)?;
    let event = crate::history::start("assistant_key", None, json!({"action":"delete"}))?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let output = command.output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    if result.is_ok() {
        process.stop();
    }
    finish_assistant_action(&event, result)
}

/// Refresh through the sidecar so the saved API key never enters the WebView.
#[tauri::command]
pub async fn assistant_api_models(
    app: tauri::AppHandle,
    base_url: String,
    api_format: String,
) -> Result<Vec<String>, String> {
    validate_api_address(&base_url)?;
    if !matches!(
        api_format.as_str(),
        "chat_completions" | "responses" | "anthropic"
    ) {
        return Err("不支持这个 API 接口格式".into());
    }
    let mut command = key_command(&app, "--list-models", base_url.trim())?;
    command.args(["--api-format", &api_format]);
    tauri::async_runtime::spawn_blocking(move || {
        let output = command
            .output()
            .map_err(|e| format!("cannot start model refresh: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 发一行给 `colm-agent`。
fn send(process: &AssistantProcess, message: &Value) -> Result<(), String> {
    send_in_generation(process, None, message)
}

fn send_in_generation(
    process: &AssistantProcess,
    generation: Option<u64>,
    message: &Value,
) -> Result<(), String> {
    let mut guard = process.inner.lock().map_err(|e| e.to_string())?;
    if let Some(generation) = generation {
        if process
            .ui_requests
            .lock()
            .map_err(|e| e.to_string())?
            .generation
            != generation
        {
            return Err("stale assistant response".into());
        }
    }
    let (_, stdin) = guard.as_mut().ok_or("助手还没有启动")?;
    writeln!(stdin, "{message}").map_err(|e| format!("助手进程已退出：{e}"))?;
    stdin.flush().map_err(|e| e.to_string())
}

/// `configure` 消息。
pub(crate) fn configure_message(
    settings: &AssistantSettings,
    project_root: &str,
    kernel_dir: Option<&str>,
    docs_root: Option<&str>,
    model_context: Option<&Value>,
) -> Value {
    json!({
        "type": "configure",
        "provider": {
            "base_url": settings.base_url.trim(),
            "model": settings.model.trim(),
            // 只有显式值才下传；None 由 agent 按文档确认的预设解析，未知模型在发送前要求填写。
            "context_window": settings.context_window,
            "provider_id":settings.provider_id,
            "api_format":settings.api_format,
            "api_options":settings.api_options,
            "max_output_tokens":settings.max_output_tokens,
            "timeout_seconds":settings.timeout_seconds,
            "strict":settings.strict,
            "thinking": settings.thinking,
            "reasoning_effort": settings.reasoning_effort,
        },
        "approval": settings.approval,
        "web_search": settings.web_search,
        "session_turn_limit": settings.session_turn_limit,
        "session_token_limit": settings.session_token_limit,
        "backend": settings.backend,
        "external": settings.external.get(&settings.backend).cloned().unwrap_or_default(),
        // 检索是应用级设置，与用哪个后端回答无关；关闭时也照常下发，agent 用 enabled=false 跳过。
        "retrieval": {
            "enabled": settings.retrieval.enabled,
            "endpoint": settings.retrieval.endpoint.trim(),
            "index": settings.retrieval.index,
            "timeout_ms": settings.retrieval.timeout_ms,
        },
        // 窗口能被助手驱动（引导模式）。
        "ui": true,
        "project_root": project_root,
        "kernel_dir": kernel_dir,
        "docs_root": docs_root,
        "model_context": model_context,
    })
}

/// 启动（或重新配置）助手。进程不在就起一个，并把它的每行输出作为 `assistant://event` 发给前端。
#[tauri::command]
pub async fn assistant_start(
    app: tauri::AppHandle,
    process: tauri::State<'_, AssistantProcess>,
    project_root: String,
    kernel_dir: Option<String>,
    resume: Option<String>,
    case_dir: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<(), String> {
    let mut settings = assistant_settings(app.clone());
    validate_settings(&settings)?;
    require_egress_acknowledged(&settings)?;
    // managed 模式：先把应用自己的 Xerj 节点拉起来、把应用自管的 key 交给 agent，再把实际
    // endpoint 写进 configure。失败不阻断助手：保留 enabled，让 agent 报 not_connected 并回退。
    if settings.retrieval.enabled && settings.retrieval.mode == "managed" {
        let node = app.state::<ManagedXerj>().handle();
        let settings_for_start = settings.clone();
        let app_for_start = app.clone();
        let started = tauri::async_runtime::spawn_blocking(move || {
            node.ensure_started(&app_for_start, &settings_for_start)
        })
        .await
        .map_err(|e| e.to_string())?;
        match started {
            Ok(endpoint) => settings.retrieval.endpoint = endpoint,
            Err(error) => eprintln!("managed Xerj node is unavailable: {error}"),
        }
    }
    let model_context = match model_revision {
        Some(version) => Some(crate::model::assistant_context(version, case_dir.as_deref()).await?),
        None => None,
    };
    {
        let mut guard = process.inner.lock().map_err(|e| e.to_string())?;
        let alive = guard
            .as_mut()
            .is_some_and(|(child, _)| matches!(child.try_wait(), Ok(None)));
        if !alive {
            let mut command = std::process::Command::new(agent_path());
            command
                .arg("--data-dir")
                .arg(data_dir(&app)?)
                .arg("--cli")
                .arg(crate::sidecar::resolve_cli())
                .arg("--key-file")
                .arg(key_file(&app)?)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // agent 领导自己的进程组：应用退出/换 Key 停它时，连它启动的命令与运行一起结束，
            // 不留下孙进程（GUI 审查 B-07）。
            colm_kernel::run::top_level_sidecar(&mut command);
            let mut child = command
                .spawn()
                .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let stderr = child.stderr.take().ok_or("no stderr")?;
            let ui_requests = Arc::clone(&process.ui_requests);
            let generation = {
                let mut pending = ui_requests.lock().map_err(|e| e.to_string())?;
                pending.generation = pending.generation.wrapping_add(1);
                pending.requests.clear();
                pending.approvals.clear();
                pending.generation
            };
            let events = app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let mut value = serde_json::from_str::<Value>(&line).ok();
                    let result = {
                        let Ok(mut pending) = ui_requests.lock() else {
                            break;
                        };
                        if pending.generation != generation {
                            break;
                        }
                        match value.as_mut() {
                            Some(value) if value["type"] == "ui_request" => {
                                register_ui_request(&mut pending, generation, value)
                            }
                            Some(value) if value["type"] == "approval_request" => {
                                register_approval_request(&mut pending, generation, value)
                            }
                            Some(value)
                                if value["type"] == "tool_call"
                                    || value["type"] == "tool_result" =>
                            {
                                if let Some(id) = value["id"].as_str() {
                                    let id = format!("{generation}:{id}");
                                    if value["type"] == "tool_result" {
                                        pending.approvals.remove(&id);
                                    }
                                    value["id"] = json!(id);
                                }
                                Ok(())
                            }
                            _ => Ok(()),
                        }
                    };
                    if let Err(error) = result {
                        if let Some(value) = value {
                            let response = if value["type"] == "approval_request" {
                                json!({"type":"approval_decision","id":value["id"],"approve":false,"remember":false,"note":error})
                            } else {
                                json!({"type":"ui_result","id":value["id"],"ok":false,"result":error})
                            };
                            let _ = send_in_generation(
                                &events.state::<AssistantProcess>(),
                                Some(generation),
                                &response,
                            );
                        }
                        continue;
                    }
                    let _ = events.emit(
                        "assistant://event",
                        value.map_or(line, |value| value.to_string()),
                    );
                }
                if ui_requests
                    .lock()
                    .is_ok_and(|pending| pending.generation == generation)
                {
                    let _ =
                        events.emit("assistant://event", json!({ "type": "exited" }).to_string());
                }
            });
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("colm-agent: {line}");
                }
            });
            *guard = Some((child, stdin));
        }
    }
    let starts: Vec<PathBuf> = [
        app.path().resource_dir().ok(),
        agent_path().parent().map(PathBuf::from),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let docs = docs_root(&starts).map(|d| d.display().to_string());
    send(
        &process,
        &configure_message(
            &settings,
            &project_root,
            kernel_dir.as_deref(),
            docs.as_deref(),
            model_context.as_ref(),
        ),
    )?;
    // 续接界面上正显示的那段历史（重启进程后也接得上）。
    match resume.filter(|id| !id.is_empty()) {
        Some(id) => send(&process, &json!({ "type": "resume_session", "id": id })),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn assistant_send(
    app: tauri::AppHandle,
    process: tauri::State<'_, AssistantProcess>,
    text: String,
    context: Option<String>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("消息是空的".into());
    }
    require_egress_acknowledged(&assistant_settings(app))?;
    send(
        &process,
        &json!({ "type": "user_message", "text": text, "context": context }),
    )
}

#[tauri::command]
pub fn assistant_approve(
    process: tauri::State<'_, AssistantProcess>,
    id: String,
    approve: bool,
    note: Option<String>,
    remember: Option<bool>,
) -> Result<(), String> {
    let (generation, original_id) = {
        let mut pending = process.ui_requests.lock().map_err(|e| e.to_string())?;
        let original = pending
            .approvals
            .remove(&id)
            .ok_or("unregistered or already answered approval request")?;
        (pending.generation, original)
    };
    let event = crate::history::start(
        "assistant_approval",
        None,
        json!({"request_id":id,"approve":approve,"remember":remember.unwrap_or(false)}),
    )?;
    let result = send_in_generation(
        &process,
        Some(generation),
        &json!({
            "type": "approval_decision",
            "id": original_id,
            "approve": approve,
            "note": note,
            "remember": remember.unwrap_or(false),
        }),
    );
    finish_assistant_action(&event, result)
}

#[tauri::command]
pub fn assistant_cancel(process: tauri::State<'_, AssistantProcess>) -> Result<(), String> {
    send(&process, &json!({ "type": "cancel" }))
}

#[tauri::command]
pub fn assistant_new_session(process: tauri::State<'_, AssistantProcess>) -> Result<(), String> {
    send(&process, &json!({ "type": "new_session" }))
}

#[cfg(test)]
#[path = "assistant_tests.rs"]
mod assistant_tests;
