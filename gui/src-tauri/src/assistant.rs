//! AI 助手（docs/design-ai-assistant.md）：启动 `colm-agent` sidecar，经 stdio JSONL 转发消息与事件。
//!
//! 这里只做转发与设置：对话循环、工具、审批规则都在 `colm-agent` 里。API Key 由 `colm-agent`
//! 自己存进系统钥匙串，窗口进程只在“保存 Key”那一刻经 stdin 把它交过去，不落盘、不回传前端。

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
    /// 用户已确认过“数据会发给模型服务商”的那个服务地址。
    #[serde(default)]
    pub egress_acknowledged: Option<String>,
    /// 已经存过 Key 的服务地址（只是标记，不含 Key）。界面据此显示“已保存”，
    /// 不必为了这一点去读钥匙串——每读一次 macOS 都可能弹授权框。
    #[serde(default)]
    pub key_saved_for: Vec<String>,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-flash".into(),
            thinking: None,
            egress_acknowledged: None,
            key_saved_for: Vec::new(),
        }
    }
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.join("assistant.json"))
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
    mut settings: AssistantSettings,
) -> Result<(), String> {
    validate_settings(&settings)?;
    // “存过 Key”的标记只由保存、删除 Key 改，前端保存其他设置时不能把它冲掉。
    settings.key_saved_for = assistant_settings(app.clone()).key_saved_for;
    let path = settings_path(&app)?;
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

fn key_command(flag: &str, base_url: &str) -> std::process::Command {
    let mut command = std::process::Command::new(agent_path());
    command.args([flag, base_url]);
    colm_kernel::run::no_console(&mut command);
    command
}

/// 把 Key 交给 `colm-agent` 存进钥匙串（经 stdin，不出现在命令行或日志里）。
fn normalized(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// 记下（或去掉）“这个服务存过 Key”的标记。
fn mark_key(app: &tauri::AppHandle, base_url: &str, saved: bool) -> Result<(), String> {
    let mut settings = assistant_settings(app.clone());
    let url = normalized(base_url);
    settings.key_saved_for.retain(|u| u != &url);
    if saved {
        settings.key_saved_for.push(url);
    }
    let path = settings_path(app)?;
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

#[tauri::command]
pub async fn assistant_set_key(
    app: tauri::AppHandle,
    base_url: String,
    key: String,
) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("Key 是空的".into());
    }
    let url = base_url.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut child = key_command("--set-key", &base_url)
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
    mark_key(&app, &url, true)
}

/// 这个服务存过 Key 吗？只看设置里的标记，不读钥匙串。
#[tauri::command]
pub fn assistant_has_key(app: tauri::AppHandle, base_url: String) -> bool {
    assistant_settings(app)
        .key_saved_for
        .contains(&normalized(&base_url))
}

#[tauri::command]
pub async fn assistant_delete_key(app: tauri::AppHandle, base_url: String) -> Result<(), String> {
    let url = base_url.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output = key_command("--delete-key", &base_url)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    mark_key(&app, &url, false)
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
        },
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
    send(
        &process,
        &configure_message(&settings, &project_root, kernel_dir.as_deref(), None),
    )
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
) -> Result<(), String> {
    send(
        &process,
        &json!({ "type": "approval_decision", "id": id, "approve": approve, "note": note }),
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
