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
    /// DeepSeek 思考模式；`None` 用服务端默认（开启）。
    #[serde(default)]
    pub thinking: Option<bool>,
    /// 思考强度 low / high / max；`None` 用服务端默认（high）。
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 运行操作的审批：`ask` 每次询问（审批卡可选本会话不再询问），`auto` 直接执行。
    #[serde(default = "default_approval")]
    pub approval: String,
    /// 联网搜索（DeepSeek 原生搜索，用 DeepSeek 的 Key）与读网页。
    #[serde(default = "default_web_search")]
    pub web_search: bool,
    /// 用户已确认过“数据会发给模型服务商”的那个服务地址。
    #[serde(default)]
    pub egress_acknowledged: Option<String>,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-flash".into(),
            thinking: None,
            reasoning_effort: None,
            approval: default_approval(),
            web_search: default_web_search(),
            egress_acknowledged: None,
        }
    }
}

fn default_approval() -> String {
    "ask".into()
}

fn default_web_search() -> bool {
    true
}

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

/// DeepSeek 实际生效的三档（medium 会被归到 high，xhigh、ultra 归到 max，列出来没有意义）。
const REASONING_EFFORTS: [&str; 3] = ["low", "high", "max"];

/// 可供 `search_docs` 检索的项目文档目录：从 `colm-agent` 所在目录与当前目录往上找含
/// `docs/design-ai-assistant.md` 的仓库（开发环境）。安装包里没有仓库文档时为空。
pub(crate) fn docs_root(starts: &[PathBuf]) -> Option<PathBuf> {
    starts.iter().find_map(|start| {
        start
            .ancestors()
            .map(|dir| dir.join("docs"))
            .find(|docs| docs.join("design-ai-assistant.md").is_file())
    })
}

/// 设置校验：服务地址只接受 https，或本机回环的 http（本地模型）。
pub(crate) fn validate_settings(settings: &AssistantSettings) -> Result<(), String> {
    let url = settings.base_url.trim();
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| url.starts_with(prefix));
    if !(url.starts_with("https://") || local) {
        return Err("服务地址必须是 https://，或本机的 http://127.0.0.1 / localhost".into());
    }
    if settings.model.trim().is_empty() {
        return Err("请填写模型名".into());
    }
    if !matches!(settings.approval.as_str(), "ask" | "auto") {
        return Err("审批方式只能是 ask 或 auto".into());
    }
    if let Some(effort) = &settings.reasoning_effort {
        if !REASONING_EFFORTS.contains(&effort.as_str()) {
            return Err(format!("思考强度只能是 {}", REASONING_EFFORTS.join("、")));
        }
    }
    Ok(())
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

/// 历史会话列表，最近的在前。
#[tauri::command]
pub async fn assistant_sessions(app: tauri::AppHandle) -> Result<Value, String> {
    let text = session_command(&app, vec!["--list-sessions".into()]).await?;
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
            "thinking": settings.thinking,
            "reasoning_effort": settings.reasoning_effort,
        },
        "approval": settings.approval,
        "web_search": settings.web_search,
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
