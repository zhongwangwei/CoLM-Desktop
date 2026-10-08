//! 远程运行（R1）：服务器配置、测试连接、提交、查状态、取消、取回结果。实际工作都由
//! `colm-cli remote-*` 做（经用户自己的 ssh），这里只存配置、转调命令、把日志末尾解析成阶段与进度。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;

/// 一条路径对应：本机前缀 → 服务器前缀。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathMap {
    pub local: String,
    pub remote: String,
}

/// 一台服务器。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// ssh 配置里的别名（或 `user@host`）。
    pub host: String,
    /// 服务器上的工作根目录（绝对路径）。
    pub root: String,
    #[serde(default)]
    pub maps: Vec<PathMap>,
    #[serde(default = "default_threads")]
    pub threads: u32,
}

fn default_threads() -> u32 {
    8
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfig {
    #[serde(default)]
    pub servers: Vec<Server>,
}

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.join("remote.json"))
}

/// 配置校验：别名不能被 ssh 当成选项，目录要是绝对路径且不含空格（CoLM 的路径不许空格）。
pub(crate) fn validate(config: &RemoteConfig) -> Result<(), String> {
    for server in &config.servers {
        let host = server.host.trim();
        if host.is_empty() || host.starts_with('-') || host.contains(char::is_whitespace) {
            return Err(format!("服务器名不合法：{host:?}"));
        }
        let root = server.root.trim();
        if !root.starts_with('/') || root == "/" || root.contains(char::is_whitespace) {
            return Err(format!(
                "{host} 的工作目录要是服务器上的绝对路径、不含空格：{root:?}"
            ));
        }
        for map in &server.maps {
            if !std::path::Path::new(map.local.trim()).is_absolute()
                || !map.remote.trim().starts_with('/')
            {
                return Err(format!(
                    "路径对应要两边都是绝对路径：{} = {}",
                    map.local, map.remote
                ));
            }
        }
        if server.threads == 0 || server.threads > 256 {
            return Err(format!("{host} 的线程数要在 1–256 之间"));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn remote_config(app: tauri::AppHandle) -> RemoteConfig {
    config_path(&app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn remote_save_config(app: tauri::AppHandle, config: RemoteConfig) -> Result<(), String> {
    validate(&config)?;
    let path = config_path(&app)?;
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// ssh 配置里的主机别名（不含通配符），给“选择服务器”用；只读别名，不读别的。
pub(crate) fn ssh_aliases(text: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("Host ")
            .or_else(|| line.strip_prefix("Host\t"))
            .or_else(|| line.strip_prefix("host "))
        else {
            continue;
        };
        for name in rest.split_whitespace() {
            if !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                hosts.push(name.to_owned());
            }
        }
    }
    hosts
}

#[tauri::command]
pub fn remote_ssh_hosts() -> Vec<String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    std::fs::read_to_string(home.join(".ssh").join("config"))
        .map(|text| ssh_aliases(&text))
        .unwrap_or_default()
}

/// 调 `colm-cli`，取最后一行 JSON；失败时把 stderr 的末尾作为错误。
async fn cli_json(args: Vec<String>) -> Result<Value, String> {
    let mut command = std::process::Command::new(crate::sidecar::resolve_cli());
    command.args(&args);
    colm_kernel::run::no_console(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let output = command
            .output()
            .map_err(|e| format!("无法启动 colm-cli：{e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
            return Err(tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("{}");
        serde_json::from_str(last).map_err(|e| format!("colm-cli 的输出不是 JSON：{e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn remote_probe(host: String, root: String) -> Result<Value, String> {
    cli_json(vec![
        "remote-probe".into(),
        "--host".into(),
        host,
        "--root".into(),
        root,
    ])
    .await
}

#[tauri::command]
pub async fn remote_run(
    app: tauri::AppHandle,
    case: String,
    host: String,
    kernel: String,
    stage: Option<String>,
    force: bool,
) -> Result<Value, String> {
    let config = remote_config(app);
    let server = config
        .servers
        .iter()
        .find(|s| s.host == host)
        .ok_or_else(|| format!("没有配置服务器 {host}"))?;
    let mut args = vec![
        "remote-run".into(),
        case,
        "--host".into(),
        server.host.clone(),
        "--root".into(),
        server.root.clone(),
        "--kernel".into(),
        kernel,
        "--threads".into(),
        server.threads.to_string(),
    ];
    for map in &server.maps {
        args.push("--map".into());
        args.push(format!("{}={}", map.local.trim(), map.remote.trim()));
    }
    if let Some(stage) = stage {
        args.push("--stage".into());
        args.push(stage);
    }
    if force {
        args.push("--force".into());
        args.push("1".into());
    }
    cli_json(args).await
}

/// 远程作业的状态，加上从日志末尾解析出的阶段与进度（和本机运行同一套解析）。
#[tauri::command]
pub async fn remote_status(case: String) -> Result<Value, String> {
    let mut answer = cli_json(vec![
        "remote-status".into(),
        case.clone(),
        "--lines".into(),
        "200".into(),
    ])
    .await?;
    let log = answer["status"]["log_tail"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let (stages, progress) = crate::sidecar::parse_log(&log);
    let total_steps = crate::config::read_timing(vec![case])
        .ok()
        .map(|t| t.total_steps);
    answer["stages"] = json!(stages);
    answer["progress"] = json!(progress.map(|p| json!({
        "step": p.0, "date": p.1, "total_steps": total_steps,
    })));
    Ok(answer)
}

#[tauri::command]
pub async fn remote_cancel(case: String) -> Result<Value, String> {
    cli_json(vec!["remote-cancel".into(), case]).await
}

#[tauri::command]
pub async fn remote_fetch(case: String) -> Result<Value, String> {
    cli_json(vec!["remote-fetch".into(), case]).await
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod remote_tests;
