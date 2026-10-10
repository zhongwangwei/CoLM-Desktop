//! AI 助手（docs/design-ai-assistant.md）：启动 `colm-agent` sidecar，经 stdio JSONL 转发消息与事件。
//!
//! 这里只做转发与设置：对话循环、工具、审批规则都在 `colm-agent` 里。API Key 存在应用配置目录的
//! `assistant-keys.json`（三个平台一样，Unix 上 0600），文件格式由 `colm-agent` 管：窗口进程只在
//! “保存 Key”那一刻经 stdin 把它交过去，Key 不回传前端。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

/// 运行中的 `colm-agent`。
#[derive(Default)]
pub struct AssistantProcess {
    inner: Mutex<Option<(Child, ChildStdin)>>,
}

impl AssistantProcess {
    /// 应用退出时结束它（stdin 一关它就会自己退出）。
    pub fn stop(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some((mut child, stdin)) = guard.take() {
                drop(stdin);
                let _ = child.kill();
            }
        }
    }
}

/// 助手设置（不含 Key），存在配置目录的 `assistant.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantSettings {
    pub base_url: String,
    pub model: String,
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
    /// 后端：API 服务或用户本机配置的 Codex、Claude Code、OpenCode。
    #[serde(default = "default_backend")]
    pub backend: String,
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
            backend: default_backend(),
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
    if !matches!(settings.approval.as_str(), "ask" | "auto") {
        return Err("审批方式只能是 ask 或 auto".into());
    }
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
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))
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
    send(
        &process,
        &json!({ "type": "ui_result", "id": id, "ok": ok, "result": result }),
    )
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
    let mut command = key_command(&app, "--set-key", &base_url)?;
    tauri::async_runtime::spawn_blocking(move || {
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
    .map_err(|e| e.to_string())??;
    process.stop();
    Ok(())
}

#[tauri::command]
pub async fn assistant_has_key(app: tauri::AppHandle, base_url: String) -> Result<bool, String> {
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
    let mut command = key_command(&app, "--delete-key", &base_url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let output = command.output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    process.stop();
    Ok(())
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
    let mut guard = process.inner.lock().map_err(|e| e.to_string())?;
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
) -> Value {
    json!({
        "type": "configure",
        "provider": {
            "base_url": settings.base_url.trim(),
            "model": settings.model.trim(),
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
        "backend": settings.backend,
        "external": settings.external.get(&settings.backend).cloned().unwrap_or_default(),
        // 窗口能被助手驱动（引导模式）。
        "ui": true,
        "project_root": project_root,
        "kernel_dir": kernel_dir,
        "docs_root": docs_root,
    })
}

/// 启动（或重新配置）助手。进程不在就起一个，并把它的每行输出作为 `assistant://event` 发给前端。
#[tauri::command]
pub fn assistant_start(
    app: tauri::AppHandle,
    process: tauri::State<'_, AssistantProcess>,
    project_root: String,
    kernel_dir: Option<String>,
    resume: Option<String>,
) -> Result<(), String> {
    let settings = assistant_settings(app.clone());
    validate_settings(&settings)?;
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
            colm_kernel::run::no_console(&mut command);
            let mut child = command
                .spawn()
                .map_err(|e| format!("cannot start {}: {e}", agent_path().display()))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let stderr = child.stderr.take().ok_or("no stderr")?;
            let events = app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let _ = events.emit("assistant://event", line);
                }
                let _ = events.emit("assistant://event", json!({ "type": "exited" }).to_string());
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
    process: tauri::State<'_, AssistantProcess>,
    text: String,
    context: Option<String>,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("消息是空的".into());
    }
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
    send(
        &process,
        &json!({
            "type": "approval_decision",
            "id": id,
            "approve": approve,
            "note": note,
            "remember": remember.unwrap_or(false),
        }),
    )
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
