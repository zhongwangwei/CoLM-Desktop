//! 远程运行（R1）：服务器配置、测试连接、提交、查状态、取消、取回结果。实际工作都由
//! `colm-cli remote-*` 做（经用户自己的 ssh），这里只存配置、转调命令、把日志末尾解析成阶段与进度。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

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
    #[serde(default)]
    pub username: String,
    /// 0 means use the SSH configuration or default port.
    #[serde(default)]
    pub port: u16,
    #[serde(default = "default_auth")]
    pub auth: String,
    #[serde(default)]
    pub identity_file: String,
    /// 服务器上的工作根目录（绝对路径）。
    pub root: String,
    #[serde(default)]
    pub maps: Vec<PathMap>,
    #[serde(default = "default_threads")]
    pub threads: u32,
    /// auto（问服务器有什么调度系统）、bare（直接后台运行）、slurm、pbs 或 lsf。
    #[serde(default = "default_scheduler")]
    pub scheduler: String,
    /// 分区（Slurm）或队列（PBS、LSF）。
    #[serde(default)]
    pub partition: String,
    #[serde(default)]
    pub account: String,
    /// `HH:MM:SS` 或 `D-HH:MM:SS`；空表示不限（用调度系统的默认）。
    #[serde(default)]
    pub walltime: String,
    /// 申请的核数；0 表示和线程数一样。
    #[serde(default)]
    pub cpus: u32,
    /// 申请的内存（GB，每个节点）；0 表示不指定。
    #[serde(default)]
    pub memory_gb: u32,
    /// MPI 运行占几个节点；0 表示不指定（Slurm 自己分配，其余按一个节点）。
    #[serde(default)]
    pub nodes: u32,
    /// 自动取回结果时只取这些变量（逗号分隔，例如 `f_fsena,f_rnet`）；空表示取回全部。
    #[serde(default)]
    pub fetch_vars: String,
    /// 作业开头先执行的环境准备，例如 `module load rust`。
    #[serde(default)]
    pub env_script: String,
    /// 原样追加的调度指令选项，每行一条，例如 `--constraint=cpu`。
    #[serde(default)]
    pub directives: Vec<String>,
}

fn default_auth() -> String {
    "config".into()
}

// Passwords live only in this process, bound to the exact connection.
type Passwords = HashMap<(String, String, u16), String>;
static PASSWORDS: OnceLock<Mutex<Passwords>> = OnceLock::new();
fn passwords() -> &'static Mutex<Passwords> {
    PASSWORDS.get_or_init(Mutex::default)
}

fn validate_login(username: &str) -> Result<(), String> {
    if !username.is_empty()
        && (username.starts_with('-')
            || !username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)))
    {
        return Err("登录用户名只能含字母、数字、下划线、点和连字符".into());
    }
    Ok(())
}

#[tauri::command]
pub fn remote_set_password(
    host: String,
    username: String,
    port: u16,
    password: String,
) -> Result<(), String> {
    if host.is_empty() || host.starts_with('-') || host.contains(char::is_whitespace) {
        return Err("服务器名不合法".into());
    }
    validate_login(&username)?;
    if password.is_empty() || password.contains(['\n', '\r', '\0']) || password.len() > 4096 {
        return Err("密码不能为空或包含换行，长度不能超过 4096 字节".into());
    }
    passwords()
        .lock()
        .map_err(|_| "密码会话不可用")?
        .insert((host, username, port), password);
    Ok(())
}

fn default_threads() -> u32 {
    8
}

fn default_scheduler() -> String {
    "auto".into()
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
        validate_login(&server.username)?;
        if !["config", "key", "password"].contains(&server.auth.as_str()) {
            return Err("认证方式只能是 SSH 配置、密钥或密码".into());
        }
        if !server.identity_file.is_empty()
            && (!std::path::Path::new(&server.identity_file).is_absolute()
                || server.identity_file.contains(['\n', '\r', '\0']))
        {
            return Err("密钥文件要是本机的绝对路径".into());
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
        if !["auto", "bare", "slurm", "pbs", "lsf"].contains(&server.scheduler.as_str()) {
            return Err(format!(
                "{host} 的调度系统只能是 auto、bare、slurm、pbs 或 lsf：{:?}",
                server.scheduler
            ));
        }
        for (what, value) in [("分区或队列", &server.partition), ("账户", &server.account)] {
            let v = value.trim();
            if !v.is_empty()
                && (v.starts_with('-')
                    || !v
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_.-@:+/".contains(c)))
            {
                return Err(format!(
                    "{host} 的{what}只能含字母、数字和 _ . - @ : + /：{v:?}"
                ));
            }
        }
        let wall = server.walltime.trim();
        if !wall.is_empty()
            && (wall.starts_with('-')
                || !wall
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ':' || c == '-'))
        {
            return Err(format!(
                "{host} 的时限要写成 HH:MM:SS 或 D-HH:MM:SS：{wall:?}"
            ));
        }
        let vars = server.fetch_vars.trim();
        if !vars.is_empty()
            && !vars
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ',')
        {
            return Err(format!(
                "{host} 取回的变量只能写变量名，用逗号分隔：{vars:?}"
            ));
        }
        if server.nodes > 4096 {
            return Err(format!("{host} 的节点数不合理"));
        }
        if server.cpus > 1024 || server.memory_gb > 100_000 {
            return Err(format!("{host} 的核数或内存数不合理"));
        }
        for line in &server.directives {
            if !line.trim().is_empty()
                && (!line.trim().starts_with('-') || line.contains(['\n', '\r']))
            {
                return Err(format!("{host} 的调度指令每行要以 - 开头：{line:?}"));
            }
        }
        if server.env_script.contains("COLM_JOB_EOF") {
            return Err(format!("{host} 的环境准备里不能出现 COLM_JOB_EOF"));
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
    .map_err(|e| format!("{}: {e}", path.display()))?;
    passwords()
        .lock()
        .map_err(|_| "密码会话不可用")?
        .retain(|(host, username, port), _| {
            config.servers.iter().any(|s| {
                s.auth == "password"
                    && &s.host == host
                    && &s.username == username
                    && &s.port == port
            })
        });
    Ok(())
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

/// 安装包的资源目录；`colm-cli` 在里面找随附的源码包与预编引擎（`COLM_RESOURCE_DIR`）。
fn resource_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok()
}

fn auth_payload(server: &Server) -> Result<Value, String> {
    let password = if server.auth == "password" {
        Some(
            passwords()
                .lock()
                .map_err(|_| "密码会话不可用")?
                .get(&(server.host.clone(), server.username.clone(), server.port))
                .cloned()
                .ok_or_else(|| format!("请在服务器设置中为 {} 的登录账号 {}（端口 {}）重新输入密码；密码只保留到应用关闭", server.host, if server.username.is_empty() { "SSH 配置中的账号" } else { &server.username }, if server.port == 0 { "SSH 配置或 22".to_owned() } else { server.port.to_string() }))?,
        )
    } else {
        None
    };
    Ok(
        json!({"host": server.host, "username": server.username, "port": server.port,
        "auth": server.auth, "identity_file": server.identity_file, "password": password}),
    )
}

fn command_server(config: &RemoteConfig, args: &[String]) -> Result<Option<Server>, String> {
    if let Some(at) = args.iter().position(|a| a == "--host") {
        return Ok(args
            .get(at + 1)
            .and_then(|host| config.servers.iter().find(|s| &s.host == host))
            .cloned());
    }
    let Some(case) = args.get(1) else {
        return Ok(None);
    };
    let path = PathBuf::from(case).join(".colm-remote.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let record: Value = serde_json::from_str(&text).map_err(|_| "远程作业记录不合法")?;
    server_from_record(config, &record).map(Some)
}

fn server_from_record(config: &RemoteConfig, record: &Value) -> Result<Server, String> {
    let host = record["host"].as_str().ok_or("远程作业记录缺少主机")?;
    let saved = config.servers.iter().find(|s| s.host == host).cloned();
    let Some(binding) = record.get("ssh_auth").filter(|v| !v.is_null()) else {
        // Legacy jobs were submitted through the original SSH alias/configuration.
        let mut server = saved
            .or_else(|| serde_json::from_value(json!({"host": host, "root": record["root"]})).ok())
            .ok_or_else(|| "远程作业记录缺少工作目录".to_owned())?;
        server.username.clear();
        server.port = 0;
        server.auth = default_auth();
        server.identity_file.clear();
        server.root = record["root"]
            .as_str()
            .ok_or("远程作业记录缺少工作目录")?
            .to_owned();
        return Ok(server);
    };
    if binding["host"].as_str() != Some(host) {
        return Err("远程作业认证主机与作业主机不一致".into());
    }
    let mut fields = binding.clone();
    fields["root"] = record["root"].clone();
    let connection: Server =
        serde_json::from_value(fields).map_err(|_| "远程作业认证信息不合法")?;
    validate(&RemoteConfig {
        servers: vec![connection.clone()],
    })?;
    let mut server = saved.unwrap_or_else(|| connection.clone());
    server.username = connection.username;
    server.port = connection.port;
    server.auth = connection.auth;
    server.identity_file = connection.identity_file;
    server.root = connection.root;
    Ok(server)
}

// Shared sidecar calls without SSH credentials (development workspaces).
pub(crate) async fn cli_json(
    args: Vec<String>,
    resources: Option<PathBuf>,
) -> Result<Value, String> {
    ssh_cli_json(args, resources, None).await
}

/// 调 `colm-cli`，取最后一行 JSON；失败时把 stderr 的末尾作为错误。
async fn ssh_cli_json(
    args: Vec<String>,
    resources: Option<PathBuf>,
    server: Option<&Server>,
) -> Result<Value, String> {
    let payload = server.map(auth_payload).transpose()?;
    let password = payload
        .as_ref()
        .and_then(|p| p["password"].as_str())
        .map(str::to_owned);
    let payload = payload.map(|p| p.to_string());
    let mut command = std::process::Command::new(crate::sidecar::resolve_cli());
    command.args(&args);
    if payload.is_some() {
        command
            .env("COLM_SSH_AUTH_STDIN", "1")
            .stdin(Stdio::piped());
    }
    if let Some(dir) = resources {
        command.env("COLM_RESOURCE_DIR", dir);
    }
    colm_kernel::run::no_console(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动 colm-cli：{e}"))?;
        if let Some(payload) = payload {
            let result = child.stdin.take().ok_or("no stdin").and_then(|mut stdin| {
                writeln!(stdin, "{payload}").map_err(|_| "无法传递 SSH 登录信息")
            });
            if let Err(error) = result {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|e| format!("colm-cli：{e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
            let mut error = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            if let Some(password) = &password {
                error = error.replace(password, "[redacted]");
            }
            return Err(error);
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
pub async fn remote_probe(app: tauri::AppHandle, server: Server) -> Result<Value, String> {
    validate(&RemoteConfig {
        servers: vec![server.clone()],
    })?;
    ssh_cli_json(
        vec![
            "remote-probe".into(),
            "--host".into(),
            server.host.clone(),
            "--root".into(),
            server.root.clone(),
        ],
        resource_dir(&app),
        Some(&server),
    )
    .await
}

/// `remote-run` 的命令行参数。预览（`--dry-run 1`）与真正提交用同一份，所以用户看到的就是要提交的。
/// 引擎与 MPI 进程数（只有 Fortran 引擎用进程数）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Engine {
    pub engine: Option<String>,
    pub ranks: Option<u32>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_args(
    server: &Server,
    case: String,
    kernel: String,
    stage: Option<String>,
    force: bool,
    dry_run: bool,
    engine: &Engine,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
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
        "--scheduler".into(),
        server.scheduler.clone(),
    ];
    for map in &server.maps {
        args.push("--map".into());
        args.push(format!("{}={}", map.local.trim(), map.remote.trim()));
    }
    let mut optional = |flag: &str, value: &str| {
        if !value.trim().is_empty() {
            args.push(flag.into());
            args.push(value.trim().to_owned());
        }
    };
    optional("--partition", &server.partition);
    optional("--account", &server.account);
    optional("--walltime", &server.walltime);
    optional("--env-script", &server.env_script);
    if server.cpus > 0 {
        args.push("--cpus".into());
        args.push(server.cpus.to_string());
    }
    if server.memory_gb > 0 {
        args.push("--mem-gb".into());
        args.push(server.memory_gb.to_string());
    }
    for line in server.directives.iter().filter(|l| !l.trim().is_empty()) {
        args.push("--directive".into());
        args.push(line.trim().to_owned());
    }
    if let Some(stage) = stage {
        args.push("--stage".into());
        args.push(stage);
    }
    if force {
        args.push("--force".into());
        args.push("1".into());
    }
    if dry_run {
        args.push("--dry-run".into());
        args.push("1".into());
    }
    if engine.engine.as_deref() == Some("fortran") {
        args.push("--engine".into());
        args.push("fortran".into());
        if let Some(ranks) = engine.ranks.filter(|r| *r > 1) {
            args.push("--ranks".into());
            args.push(ranks.to_string());
            if server.nodes > 0 {
                args.push("--nodes".into());
                args.push(server.nodes.to_string());
            }
        }
    }
    args
}

/// 调度与环境的参数（编内核、跑算例共用）。
fn scheduler_args(server: &Server) -> Vec<String> {
    let mut args: Vec<String> = vec!["--scheduler".into(), server.scheduler.clone()];
    let mut optional = |flag: &str, value: &str| {
        if !value.trim().is_empty() {
            args.push(flag.into());
            args.push(value.trim().to_owned());
        }
    };
    optional("--partition", &server.partition);
    optional("--account", &server.account);
    optional("--walltime", &server.walltime);
    optional("--env-script", &server.env_script);
    if server.cpus > 0 {
        args.push("--cpus".into());
        args.push(server.cpus.to_string());
    }
    if server.memory_gb > 0 {
        args.push("--mem-gb".into());
        args.push(server.memory_gb.to_string());
    }
    for line in server.directives.iter().filter(|l| !l.trim().is_empty()) {
        args.push("--directive".into());
        args.push(line.trim().to_owned());
    }
    args
}

fn server_for(app: tauri::AppHandle, host: &str) -> Result<Server, String> {
    remote_config(app)
        .servers
        .into_iter()
        .find(|s| s.host == host)
        .ok_or_else(|| format!("没有配置服务器 {host}"))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_run(
    app: tauri::AppHandle,
    case: String,
    host: String,
    kernel: String,
    stage: Option<String>,
    force: bool,
    engine: Option<String>,
    ranks: Option<u32>,
) -> Result<Value, String> {
    let resources = resource_dir(&app);
    let server = server_for(app, &host)?;
    let engine = Engine { engine, ranks };
    ssh_cli_json(
        run_args(&server, case, kernel, stage, force, false, &engine),
        resources,
        Some(&server),
    )
    .await
}

/// 服务器上有哪些内核（完整的 Fortran 内核才能给 Fortran 引擎用）。
#[tauri::command]
pub async fn remote_kernels(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    let server = server_for(app, &host)?;
    ssh_cli_json(
        vec![
            "remote-kernels".into(),
            "--host".into(),
            server.host.clone(),
            "--root".into(),
            server.root.clone(),
        ],
        None,
        Some(&server),
    )
    .await
}

/// 在服务器上编一个 Fortran 内核（要 gfortran、mpif90 与 netCDF-Fortran，用服务器设置里的“环境准备”载入）。
/// 编一次要十几分钟，命令一直等到编完。
#[tauri::command]
pub async fn remote_build_kernel(
    app: tauri::AppHandle,
    host: String,
    preset: String,
) -> Result<Value, String> {
    let resources = resource_dir(&app);
    let server = server_for(app, &host)?;
    let mut args: Vec<String> = vec![
        "remote-kernel".into(),
        "--host".into(),
        server.host.clone(),
        "--root".into(),
        server.root.clone(),
        "--preset".into(),
        preset,
    ];
    args.extend(scheduler_args(&server));
    ssh_cli_json(args, resources, Some(&server)).await
}

/// 提交前给用户看的作业脚本全文（不上传、不提交任何东西）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_preview(
    app: tauri::AppHandle,
    case: String,
    host: String,
    kernel: String,
    stage: Option<String>,
    force: bool,
    engine: Option<String>,
    ranks: Option<u32>,
) -> Result<Value, String> {
    let resources = resource_dir(&app);
    let server = server_for(app, &host)?;
    let engine = Engine { engine, ranks };
    ssh_cli_json(
        run_args(&server, case, kernel, stage, force, true, &engine),
        resources,
        Some(&server),
    )
    .await
}

/// Recover job identity before connecting, so a missing session password stays visible.
#[tauri::command]
pub fn remote_job_record(case: String) -> Result<Option<Value>, String> {
    let path = PathBuf::from(case).join(".colm-remote.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取远程作业记录失败：{error}")),
    };
    let record: Value = serde_json::from_str(&text).map_err(|_| "远程作业记录不合法")?;
    let host = record["host"].as_str().ok_or("远程作业记录缺少主机")?;
    let job = record["job"].as_str().ok_or("远程作业记录缺少作业编号")?;
    Ok(Some(json!({"host":host,"job":job})))
}

/// 远程作业的状态，加上从日志末尾解析出的阶段与进度（和本机运行同一套解析）。
#[tauri::command]
pub async fn remote_status(app: tauri::AppHandle, case: String) -> Result<Value, String> {
    let server = command_server(&remote_config(app), &["remote-status".into(), case.clone()])?;
    let mut answer = ssh_cli_json(
        vec![
            "remote-status".into(),
            case.clone(),
            "--lines".into(),
            "200".into(),
        ],
        None,
        server.as_ref(),
    )
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
pub async fn remote_cancel(app: tauri::AppHandle, case: String) -> Result<Value, String> {
    let args = vec!["remote-cancel".into(), case];
    let server = command_server(&remote_config(app), &args)?;
    ssh_cli_json(args, None, server.as_ref()).await
}

/// `remote-fetch` 的参数：服务器设置里配了“取回的变量”就只取这些，`all` 为真时取全部。
pub(crate) fn fetch_args(
    case: String,
    server: Option<&Server>,
    all: bool,
    overwrite_newer: bool,
) -> Vec<String> {
    let mut args = vec!["remote-fetch".to_owned(), case];
    // 自动取回不带它：本机结果比这次远程运行新时，colm-cli 拒绝覆盖；用户手动点“取回”才带。
    if overwrite_newer {
        args.push("--overwrite-newer".into());
        args.push("1".into());
    }
    if let Some(server) = server.filter(|_| !all) {
        let vars = server.fetch_vars.trim();
        if !vars.is_empty() {
            args.push("--vars".into());
            args.push(vars.to_owned());
        }
    }
    args
}

#[tauri::command]
pub async fn remote_fetch(
    app: tauri::AppHandle,
    case: String,
    host: Option<String>,
    all: Option<bool>,
    overwrite_newer: Option<bool>,
) -> Result<Value, String> {
    // Authentication follows the persisted job host, never the UI selection.
    let server = command_server(&remote_config(app), &["remote-fetch".into(), case.clone()])?;
    let _ = host;
    ssh_cli_json(
        fetch_args(
            case,
            server.as_ref(),
            all.unwrap_or(false),
            overwrite_newer.unwrap_or(false),
        ),
        None,
        server.as_ref(),
    )
    .await
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod remote_tests;
