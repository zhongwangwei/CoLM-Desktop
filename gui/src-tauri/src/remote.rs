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

pub(crate) fn validate_connection(server: &Server) -> Result<(), String> {
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
    Ok(())
}

/// 配置校验：别名不能被 ssh 当成选项，目录要是绝对路径且不含空格（CoLM 的路径不许空格）。
pub(crate) fn validate(config: &RemoteConfig) -> Result<(), String> {
    for server in &config.servers {
        validate_connection(server)?;
        let host = server.host.trim();
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

fn read_remote_config(path: &std::path::Path) -> Result<RemoteConfig, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RemoteConfig::default());
        }
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

#[tauri::command]
pub fn remote_config(app: tauri::AppHandle) -> Result<RemoteConfig, String> {
    read_remote_config(&config_path(&app)?)
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
pub(crate) fn resource_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
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
    let Some((record, pending, _)) = run_record(case)? else {
        return Ok(None);
    };
    if pending
        && args
            .first()
            .is_some_and(|command| matches!(command.as_str(), "remote-fetch" | "remote-cancel"))
    {
        return Err("远程提交尚未确认，请先核对原作业状态；确认前不能取消或取回".into());
    }
    server_from_record(config, &record).map(Some)
}

fn run_source(case: &str) -> Result<PathBuf, String> {
    let case = PathBuf::from(case)
        .canonicalize()
        .map_err(|e| format!("无法打开算例目录：{e}"))?;
    let manifest = case.join(colm_model::RUN_MANIFEST);
    let bytes = match std::fs::read(&manifest) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(case),
        Err(error) => return Err(format!("读取模型运行身份失败：{error}")),
    };
    let run: colm_model::MaterializedRun =
        serde_json::from_slice(&bytes).map_err(|_| "模型运行身份不合法")?;
    if run.run_case.canonicalize().map_err(|e| e.to_string())? != case {
        return Err("模型运行目录与记录身份不一致".into());
    }
    let source = run.source_case.canonicalize().map_err(|e| e.to_string())?;
    if !source.join("case.nml").is_file() {
        return Err("模型运行的原算例不可用".into());
    }
    Ok(source)
}

/// The source intent takes precedence even when an older completed record still exists.
fn run_record(case: &str) -> Result<Option<(Value, bool, String)>, String> {
    let source = run_source(case)?;
    let pending = source.join(".colm-remote-pending.json");
    match std::fs::read(&pending) {
        Ok(bytes) => {
            let record: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "远程提交意图不完整，请核对原作业后再运行")?;
            if record["schema"] != 1
                || record["source_case"].as_str() != source.to_str()
                || !record["run_case"]
                    .as_str()
                    .is_some_and(|p| std::path::Path::new(p).is_absolute())
                || record["provenance"]["operation_id"]
                    .as_str()
                    .is_none_or(str::is_empty)
            {
                return Err("远程提交意图与原算例身份不一致".into());
            }
            let connection = server_from_record(&RemoteConfig::default(), &record)?;
            validate(&RemoteConfig {
                servers: vec![connection],
            })?;
            if !["job", "engine", "remote_case"].iter().all(|key| {
                record[*key]
                    .as_str()
                    .is_some_and(|value| !value.is_empty() && !value.contains(char::is_control))
            }) {
                return Err("远程提交意图缺少完整作业身份".into());
            }
            return Ok(Some((record, true, source.display().to_string())));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(format!("读取远程提交意图失败：{error}")),
    }
    let mut target = PathBuf::from(case);
    let mut path = target.join(".colm-remote.json");
    if target.canonicalize().map_err(|e| e.to_string())? == source {
        let history = colm_history::History::open_default().map_err(|error| error.to_string())?;
        if let Some((recorded, unresolved)) = recorded_run_case(&history, &source)? {
            if unresolved || !path.try_exists().map_err(|error| error.to_string())? {
                target = recorded;
                path = target.join(".colm-remote.json");
                if !path.try_exists().map_err(|error| error.to_string())? {
                    return Err("历史中仍有原远程作业，但它的身份文件已缺失；请恢复原记录后核对，不能重新提交".into());
                }
            }
        }
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取远程作业记录失败：{error}")),
    };
    let record = serde_json::from_slice(&bytes).map_err(|_| "远程作业记录不合法")?;
    Ok(Some((record, false, target.display().to_string())))
}

fn recorded_run_case(
    history: &colm_history::History,
    source: &std::path::Path,
) -> Result<Option<(PathBuf, bool)>, String> {
    let mut query = colm_history::Query {
        case_path: Some(source.display().to_string()),
        limit: 1000,
        ..Default::default()
    };
    let mut completed = None;
    loop {
        let page = history.query(&query).map_err(|error| error.to_string())?;
        let count = page.items.len() as u64;
        for operation in page.items {
            let event = &operation.latest;
            if !matches!(event.kind.as_str(), "run" | "remote-run" | "history-replay")
                || event.location.as_deref() != Some("remote")
            {
                continue;
            }
            let binding = &event.detail["remote_record"];
            if !["host", "root", "job", "engine", "remote_case"]
                .iter()
                .all(|key| {
                    binding[*key]
                        .as_str()
                        .is_some_and(|value| !value.is_empty())
                })
            {
                // A materialized copy can exist before submission; it is not a remote job.
                continue;
            }
            let Some(value) = event.detail.get("model_run") else {
                continue;
            };
            let run: colm_model::MaterializedRun =
                serde_json::from_value(value.clone()).map_err(|_| "历史中的模型运行身份不合法")?;
            if run.operation_id != event.operation_id
                || run.source_case.canonicalize().map_err(|e| e.to_string())? != source
            {
                return Err("历史中的模型运行与原算例身份不一致".into());
            }
            if run.run_case == source {
                continue;
            }
            if operation.needs_reconciliation {
                return verified_run_case(&run).map(|case| Some((case, true)));
            }
            if completed.is_none() {
                completed = Some(run);
            }
        }
        query.offset += count;
        if count == 0 || query.offset >= page.total {
            break;
        }
    }
    completed
        .map(|run| verified_run_case(&run).map(|case| (case, false)))
        .transpose()
}

fn verified_run_case(run: &colm_model::MaterializedRun) -> Result<PathBuf, String> {
    let actual: colm_model::MaterializedRun = serde_json::from_slice(
        &std::fs::read(run.run_case.join(colm_model::RUN_MANIFEST))
            .map_err(|e| format!("原模型运行身份文件不可用：{e}"))?,
    )
    .map_err(|_| "原模型运行身份文件不合法")?;
    if serde_json::to_value(&actual).map_err(|e| e.to_string())?
        != serde_json::to_value(run).map_err(|e| e.to_string())?
        || actual.run_case.canonicalize().map_err(|e| e.to_string())? != run.run_case
    {
        return Err("原模型运行身份已变化".into());
    }
    Ok(run.run_case.clone())
}

pub(crate) fn server_from_record(config: &RemoteConfig, record: &Value) -> Result<Server, String> {
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
pub(crate) async fn ssh_cli_json(
    args: Vec<String>,
    resources: Option<PathBuf>,
    server: Option<&Server>,
) -> Result<Value, String> {
    ssh_cli_json_for_run(args, resources, server, None).await
}

async fn ssh_cli_json_for_run(
    mut args: Vec<String>,
    resources: Option<PathBuf>,
    server: Option<&Server>,
    run: Option<(&tauri::AppHandle, &str, &str)>,
) -> Result<Value, String> {
    if !args.iter().any(|a| a == "--history-db") {
        crate::model::options(&mut args, None)?;
    }
    let mut event = crate::history::command_start_for_run(&args, run.map(|(_, id, _)| id))?;
    let study_cancel = args
        .first()
        .is_some_and(|command| matches!(command.as_str(), "remote-study-cancel" | "remote-cancel"));
    let submitted = matches!(
        args.first().map(String::as_str),
        Some("remote-run" | "remote-study-run" | "remote-build-kernel")
    );
    if let Some(event) = &event {
        args.extend(["--operation-id".into(), event.operation_id.clone()]);
    }
    let result = async {
        if let (Some((app, run_id, case)), Some(event)) = (run, &mut event) {
            tauri::Emitter::emit(
                app,
                "run://started",
                &json!({
                    "run_id": run_id, "case": case, "source_case": case,
                    "operation_id": event.operation_id, "location": "remote",
                }),
            )
            .map_err(|error| error.to_string())?;
        }

        ssh_cli_json_untracked(args, resources, server).await
    }
    .await;
    if let (Some(event), Ok(value)) = (&mut event, &result) {
        crate::history::observe_remote_response(event, value);
    }
    let status = audit_result_status(submitted, study_cancel, &result);
    crate::history::command_finish(event.as_ref(), result, status)
}
fn audit_result_status(
    submitted: bool,
    study_cancel: bool,
    result: &Result<Value, String>,
) -> &'static str {
    if submitted {
        if result.is_ok() {
            "submitted"
        } else {
            "submission_uncertain"
        }
    } else if study_cancel {
        match result {
            Ok(value) if value["terminal"] == true => "terminal_confirmed",
            Ok(value) if value["request_accepted"] == true => "cancel_requested",
            Ok(_) => "cancel_uncertain",
            Err(_) => "cancel_uncertain",
        }
    } else {
        "succeeded"
    }
}
async fn ssh_cli_json_untracked(
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
    command.args(&args).env("COLM_AUDIT_BY_PARENT", "1");
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

pub(crate) async fn check_server_paths(
    app: tauri::AppHandle,
    host: &str,
    checks: Value,
) -> Result<(), String> {
    let server = server_for(app.clone(), host)?;
    ssh_cli_json(
        vec![
            "remote-check-paths".into(),
            "--host".into(),
            server.host.clone(),
            "--checks".into(),
            checks.to_string(),
        ],
        resource_dir(&app),
        Some(&server),
    )
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn remote_browse(
    app: tauri::AppHandle,
    host: String,
    path: Option<String>,
    server: Option<Server>,
) -> Result<Value, String> {
    let server = match server {
        Some(server) => server,
        None => server_for(app.clone(), &host)?,
    };
    let args = browse_args(&server, &host, path)?;
    ssh_cli_json(args, resource_dir(&app), Some(&server)).await
}

fn browse_args(server: &Server, host: &str, path: Option<String>) -> Result<Vec<String>, String> {
    if server.host != host {
        return Err("浏览目录的服务器必须与当前连接一致".into());
    }
    validate_connection(server)?;
    let path = path.unwrap_or_default();
    if !path.is_empty() && (!path.starts_with('/') || path.chars().any(char::is_control)) {
        return Err("Expected an absolute server path without control characters".into());
    }
    Ok(vec![
        "remote-browse".into(),
        "--host".into(),
        server.host.clone(),
        "--path".into(),
        path,
    ])
}

#[tauri::command]
pub async fn remote_mkdir(
    app: tauri::AppHandle,
    host: String,
    path: String,
    name: String,
    server: Option<Server>,
) -> Result<Value, String> {
    let server = match server {
        Some(server) => server,
        None => server_for(app.clone(), &host)?,
    };
    let args = mkdir_args(&server, &host, path, name)?;
    ssh_cli_json(args, resource_dir(&app), Some(&server)).await
}

fn mkdir_args(
    server: &Server,
    host: &str,
    path: String,
    name: String,
) -> Result<Vec<String>, String> {
    if path.is_empty()
        || name.is_empty()
        || [".", ".."].contains(&name.as_str())
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err("请选择父目录并输入单一文件夹名称，不能包含路径分隔符或控制字符".into());
    }
    let mut args = browse_args(server, host, Some(path))?;
    args[0] = "remote-mkdir".into();
    args.extend(["--name".into(), name]);
    Ok(args)
}

#[tauri::command]
pub async fn remote_probe(app: tauri::AppHandle, server: Server) -> Result<Value, String> {
    let (command, configuration_error) = probe_args(&server)?;
    let mut result = ssh_cli_json(command, resource_dir(&app), Some(&server)).await?;
    if let Some(error) = configuration_error {
        result["configuration_error"] = Value::String(error);
    }
    Ok(result)
}

fn probe_args(server: &Server) -> Result<(Vec<String>, Option<String>), String> {
    validate_connection(server)?;
    let configuration_error = validate(&RemoteConfig {
        servers: vec![server.clone()],
    })
    .err();
    let mut command = vec!["remote-probe".into(), "--host".into(), server.host.clone()];
    // Incomplete run settings must not prevent collecting actual SSH evidence.
    if configuration_error.is_none() {
        command.extend(["--root".into(), server.root.clone()]);
        for map in &server.maps {
            command.extend(["--map".into(), format!("{}={}", map.local, map.remote)]);
        }
    }
    Ok((command, configuration_error))
}

fn engine_resource_args(
    command: &str,
    server: &Server,
    snapshot: Option<&str>,
) -> Result<Vec<String>, String> {
    validate(&RemoteConfig {
        servers: vec![server.clone()],
    })?;
    let mut args = vec![
        command.into(),
        "--host".into(),
        server.host.clone(),
        "--root".into(),
        server.root.clone(),
    ];
    if let Some(snapshot) = snapshot {
        if snapshot.len() != 16
            || !snapshot
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("Invalid engine snapshot identifier".into());
        }
        args.extend(["--snapshot".into(), snapshot.into()]);
    }
    Ok(args)
}

#[tauri::command]
pub async fn remote_engines(app: tauri::AppHandle, server: Server) -> Result<Value, String> {
    ssh_cli_json(
        engine_resource_args("remote-engines", &server, None)?,
        resource_dir(&app),
        Some(&server),
    )
    .await
}

/// Explicit human-confirmed deletion; deliberately absent from assistant tools.
#[tauri::command]
pub async fn remote_engine_clean(
    app: tauri::AppHandle,
    server: Server,
    snapshot: String,
) -> Result<Value, String> {
    ssh_cli_json(
        engine_resource_args("remote-engine-clean", &server, Some(&snapshot))?,
        resource_dir(&app),
        Some(&server),
    )
    .await
}

/// Explicit human-confirmed removal of submission records and a cleanup lock left behind by interrupted
/// operations (older than an hour). They only block engine cleanup; deliberately absent from assistant tools.
#[tauri::command]
pub async fn remote_engine_recover(app: tauri::AppHandle, server: Server) -> Result<Value, String> {
    ssh_cli_json(
        engine_resource_args("remote-engine-recover", &server, None)?,
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
    /// 模型版本：`official`（或不给）是官方版，否则是一个修改版的名字（服务器上用它的源码编、用它跑）。
    pub model_version: Option<String>,
}

/// 修改版名字拼进命令行前先校验；官方版不加参数。
fn model_version_args(version: Option<&str>) -> Result<Vec<String>, String> {
    match version
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "official")
    {
        None => Ok(Vec::new()),
        Some(name) => {
            crate::workspace::valid_name(name)?;
            Ok(vec!["--model-version".into(), name.into()])
        }
    }
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
    // 名字在命令入口已经校验过（`model_version_args`）；这里只是拼上。
    if let Some(name) = engine
        .model_version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "official")
    {
        args.push("--model-version".into());
        args.push(name.to_owned());
    }
    args
}

/// Recover server ownership from a frozen run's source configuration mirror.
pub(crate) fn server_case_owner(case: &str) -> Result<Option<(String, String)>, String> {
    let mut source = PathBuf::from(case);
    let manifest = source.join(colm_model::RUN_MANIFEST);
    if manifest.is_file() {
        let run: colm_model::MaterializedRun =
            serde_json::from_slice(&std::fs::read(&manifest).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        source = run.source_case;
    }
    let metadata = source.join(".colm-desktop.json");
    let bytes = match std::fs::read(metadata) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let profile: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let Some(directory) = profile["remote_directory"].as_str() else {
        return Ok(None);
    };
    let host = profile["run_host"]
        .as_str()
        .filter(|h| {
            !h.is_empty()
                && !h.starts_with('-')
                && !h.contains(|c: char| c.is_whitespace() || c.is_control())
        })
        .ok_or("服务器算例缺少有效的所属服务器")?;
    if !directory.starts_with('/')
        || directory.trim_end_matches('/').is_empty()
        || directory.contains(|c: char| c.is_control() || c == '\\')
        || directory.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err("无效的服务器算例目录".into());
    }
    Ok(Some((host.into(), directory.trim_end_matches('/').into())))
}

pub(crate) fn server_case_args(case: &str, host: &str) -> Result<Vec<String>, String> {
    let Some((owner, directory)) = server_case_owner(case)? else {
        return Ok(Vec::new());
    };
    if owner != host {
        return Err("服务器算例只能在创建它的服务器上运行".into());
    }
    Ok(vec!["--server-case".into(), directory])
}

/// 调度与环境的参数（编内核、跑算例共用）。
pub(crate) fn scheduler_args(server: &Server) -> Vec<String> {
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

pub(crate) fn server_for(app: tauri::AppHandle, host: &str) -> Result<Server, String> {
    remote_config(app)?
        .servers
        .into_iter()
        .find(|s| s.host == host)
        .ok_or_else(|| format!("没有配置服务器 {host}"))
}

fn run_server_overrides(
    mut server: Server,
    threads: Option<u32>,
    cpus: Option<u32>,
    scheduler: Option<String>,
) -> Result<Server, String> {
    if let Some(threads) = threads {
        server.threads = threads;
    }
    if let Some(cpus) = cpus {
        if cpus == 0 {
            return Err("本次运行申请的核数必须大于 0 / Run CPUs must be positive".into());
        }
        server.cpus = cpus;
    }
    if let Some(scheduler) = scheduler {
        server.scheduler = scheduler;
    }
    validate(&RemoteConfig {
        servers: vec![server.clone()],
    })?;
    Ok(server)
}

async fn validate_server_case_inputs(
    app: &tauri::AppHandle,
    case: &str,
    host: &str,
    kernel: &str,
    revision: Option<colm_model::VersionRef>,
) -> Result<(), String> {
    if server_case_owner(case)?.is_none() {
        return Ok(());
    }
    let preview = if PathBuf::from(case).join(colm_model::RUN_MANIFEST).is_file() {
        None
    } else {
        Some(
            crate::model::model_case_context(case.to_owned(), revision)
                .await?
                .entry
                .dir,
        )
    };
    let result = async {
        let dir = preview.as_deref().unwrap_or(case);
        let text = std::fs::read_to_string(PathBuf::from(dir).join("case.nml"))
            .map_err(|error| error.to_string())?;
        let doc = colm_namelist::parse(&text).map_err(|error| error.to_string())?;
        let checks = crate::config::server_runtime_paths(
            &doc,
            std::path::Path::new(case),
            (!kernel.is_empty()).then_some(kernel),
        )?;
        check_server_paths(app.clone(), host, checks).await
    }
    .await;
    if let Some(preview) = preview {
        let _ = std::fs::remove_dir_all(preview);
    }
    result
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_run(
    app: tauri::AppHandle,
    run_id: Option<String>,
    case: String,
    host: String,
    kernel: String,
    stage: Option<String>,
    force: bool,
    engine: Option<String>,
    ranks: Option<u32>,
    threads: Option<u32>,
    cpus: Option<u32>,
    scheduler: Option<String>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
    record_input_fingerprints: Option<bool>,
) -> Result<Value, String> {
    if let Some(run_id) = &run_id {
        crate::sidecar::validate_run_id(run_id)?;
    }
    model_version_args(model_version.as_deref())?;
    let resources = resource_dir(&app);
    let server = run_server_overrides(server_for(app.clone(), &host)?, threads, cpus, scheduler)?;
    let engine = Engine {
        engine,
        ranks,
        model_version,
    };
    let server_args = server_case_args(&case, &host)?;
    validate_server_case_inputs(&app, &case, &host, &kernel, model_revision.clone()).await?;
    let mut args = run_args(&server, case.clone(), kernel, stage, force, false, &engine);
    crate::sidecar::add_input_fingerprints(&mut args, record_input_fingerprints);
    args.extend(server_args);
    crate::model::options(&mut args, model_revision.as_ref())?;
    crate::sidecar::add_preprocessors(&mut args, preprocessors.as_deref())?;
    ssh_cli_json_for_run(
        args,
        resources,
        Some(&server),
        run_id.as_deref().map(|id| (&app, id, case.as_str())),
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
    model_version: Option<String>,
) -> Result<Value, String> {
    let version = model_version_args(model_version.as_deref())?;
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
    args.extend(version);
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
    threads: Option<u32>,
    cpus: Option<u32>,
    scheduler: Option<String>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
    record_input_fingerprints: Option<bool>,
) -> Result<Value, String> {
    model_version_args(model_version.as_deref())?;
    let resources = resource_dir(&app);
    let server = run_server_overrides(server_for(app.clone(), &host)?, threads, cpus, scheduler)?;
    let engine = Engine {
        engine,
        ranks,
        model_version,
    };
    let server_args = server_case_args(&case, &host)?;
    validate_server_case_inputs(&app, &case, &host, &kernel, model_revision.clone()).await?;
    let mut args = run_args(&server, case, kernel, stage, force, true, &engine);
    crate::sidecar::add_input_fingerprints(&mut args, record_input_fingerprints);
    args.extend(server_args);
    crate::model::options(&mut args, model_revision.as_ref())?;
    crate::sidecar::add_preprocessors(&mut args, preprocessors.as_deref())?;
    ssh_cli_json(args, resources, Some(&server)).await
}

/// Recover job identity before connecting, so a missing session password stays visible.
#[tauri::command]
pub fn remote_job_record(case: String) -> Result<Option<Value>, String> {
    let Some((record, pending, target)) = run_record(&case)? else {
        return Ok(None);
    };
    let host = record["host"].as_str().ok_or("远程作业记录缺少主机")?;
    let job = record["job"].as_str().ok_or("远程作业记录缺少作业编号")?;
    Ok(Some(
        json!({"host":host,"job":job,"pending":pending,"needs_reconciliation":pending,
        "source_case":run_source(&case)?,"run_case":record.get("run_case").cloned().unwrap_or(json!(target)),
        "operation_id":record["provenance"]["operation_id"]}),
    ))
}

/// 远程作业的状态，加上从日志末尾解析出的阶段与进度（和本机运行同一套解析）。
#[tauri::command]
pub async fn remote_status(app: tauri::AppHandle, case: String) -> Result<Value, String> {
    let server = command_server(
        &remote_config(app)?,
        &["remote-status".into(), case.clone()],
    )?;
    let target = run_record(&case)?
        .map(|(_, _, target)| target)
        .unwrap_or_else(|| case.clone());
    let mut answer = ssh_cli_json(
        vec![
            "remote-status".into(),
            target.clone(),
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
    let total_steps = crate::config::read_timing(vec![target])
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
    let target = run_record(&case)?
        .map(|(_, _, target)| target)
        .unwrap_or_else(|| case.clone());
    let args = vec!["remote-cancel".into(), target];
    let server = command_server(&remote_config(app)?, &args)?;
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
    let server = command_server(&remote_config(app)?, &["remote-fetch".into(), case.clone()])?;
    let target = run_record(&case)?
        .map(|(_, _, target)| target)
        .unwrap_or(case);
    let _ = host;
    ssh_cli_json(
        fetch_args(
            target,
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

// ---- 在服务器上跑研究（`remote-study-*`）-----------------------------------------------------

/// `remote-study-run` 的参数：服务器设置（目录对应、调度、资源）与 `remote-run` 相同；研究成员各用一个进程。
pub(crate) fn study_args(
    server: &Server,
    study: String,
    kernel: String,
    engine: &Engine,
    jobs: Option<usize>,
) -> Vec<String> {
    let single = Engine {
        engine: engine.engine.clone(),
        ranks: None,
        model_version: engine.model_version.clone(),
    };
    let mut args = run_args(server, study, kernel, None, false, false, &single);
    args[0] = "remote-study-run".into();
    if let Some(jobs) = jobs.filter(|j| *j > 0) {
        args.push("--jobs".into());
        args.push(jobs.to_string());
    }
    args
}

/// 研究交给了哪台服务器（研究目录里的 `.colm-remote-study.json`）。
pub(crate) fn study_server(config: &RemoteConfig, study: &str) -> Result<Server, String> {
    let directory = PathBuf::from(study);
    let (text, pending) = match std::fs::read_to_string(directory.join(".colm-remote-study.json")) {
        Ok(text) => (text, false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            std::fs::read_to_string(directory.join(".colm-remote-study-pending.json"))
                .map_err(|_| "这个研究没有服务器提交记录")?,
            true,
        ),
        Err(_) => return Err("无法读取服务器研究记录".into()),
    };
    let record: Value = serde_json::from_str(&text).map_err(|_| "服务器研究记录不合法")?;
    if pending && record.get("ssh_auth").is_none_or(Value::is_null) {
        // Older intents contain only an SSH alias; reuse its configured credentials for this read.
        let host = record["host"].as_str().ok_or("待核对研究记录缺少主机")?;
        let root = record["root"]
            .as_str()
            .ok_or("待核对研究记录缺少工作目录")?;
        let server = config
            .servers
            .iter()
            .find(|s| s.host == host && s.root == root)
            .cloned()
            .ok_or("请恢复待核对研究的服务器连接设置")?;
        validate(&RemoteConfig {
            servers: vec![server.clone()],
        })?;
        return Ok(server);
    }
    server_from_record(config, &record)
}

fn absolute_study(study: &str) -> Result<(), String> {
    if study.contains('\0') || !PathBuf::from(study).is_absolute() {
        return Err("研究目录须为绝对路径".into());
    }
    Ok(())
}

/// 把研究交给服务器：同一份设定在那里重建并运行。
#[tauri::command]
// These are the command's named IPC fields, shared with the CLI.
#[allow(clippy::too_many_arguments)]
pub async fn remote_study_run(
    app: tauri::AppHandle,
    study_dir: String,
    host: String,
    kernel: String,
    engine: Option<String>,
    jobs: Option<usize>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
) -> Result<Value, String> {
    absolute_study(&study_dir)?;
    model_version_args(model_version.as_deref())?;
    let resources = resource_dir(&app);
    let server = server_for(app, &host)?;
    let engine = Engine {
        engine,
        ranks: None,
        model_version,
    };
    let mut args = study_args(&server, study_dir, kernel, &engine, jobs);
    crate::model::options(&mut args, model_revision.as_ref())?;
    crate::sidecar::add_preprocessors(&mut args, preprocessors.as_deref())?;
    ssh_cli_json(args, resources, Some(&server)).await
}

/// 服务器上研究的作业状态与进度。
#[tauri::command]
pub async fn remote_study_status(
    app: tauri::AppHandle,
    study_dir: String,
) -> Result<Value, String> {
    absolute_study(&study_dir)?;
    let server = study_server(&remote_config(app)?, &study_dir)?;
    ssh_cli_json(
        vec!["remote-study-status".into(), study_dir],
        None,
        Some(&server),
    )
    .await
}

/// 跑完后取回：核对成员一致，把状态与结果移植进本机研究。
#[tauri::command]
pub async fn remote_study_fetch(app: tauri::AppHandle, study_dir: String) -> Result<Value, String> {
    absolute_study(&study_dir)?;
    let server = study_server(&remote_config(app)?, &study_dir)?;
    ssh_cli_json(
        vec!["remote-study-fetch".into(), study_dir],
        None,
        Some(&server),
    )
    .await
}

/// Request cancellation of the recorded server job; status remains authoritative.
#[tauri::command]
pub async fn remote_study_cancel(
    app: tauri::AppHandle,
    study_dir: String,
) -> Result<Value, String> {
    absolute_study(&study_dir)?;
    let server = study_server(&remote_config(app)?, &study_dir)?;
    ssh_cli_json(
        vec!["remote-study-cancel".into(), study_dir],
        None,
        Some(&server),
    )
    .await
}

#[cfg(test)]
mod server_case_routing_tests {
    use super::*;
    #[test]
    fn server_case_requires_owning_host_and_absolute_project() {
        let root = std::env::temp_dir().join(format!("colm-server-route-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let case = root.to_string_lossy();
        std::fs::write(
            root.join(".colm-desktop.json"),
            br#"{"run_host":"server","remote_directory":"/data/project/"}"#,
        )
        .unwrap();
        assert_eq!(
            server_case_args(&case, "server").unwrap(),
            vec!["--server-case", "/data/project"]
        );
        assert!(server_case_args(&case, "").is_err());
        assert!(server_case_args(&case, "other").is_err());
        std::fs::write(
            root.join(".colm-desktop.json"),
            br#"{"run_host":"server","remote_directory":"/data/../other"}"#,
        )
        .unwrap();
        assert!(server_case_args(&case, "server").is_err());
        std::fs::write(root.join(".colm-desktop.json"), br#"{"run_host":"server"}"#).unwrap();
        assert!(server_case_args(&case, "server").unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tauri::command]
pub async fn probe_forcing_directory(
    app: tauri::AppHandle,
    path: String,
    host: Option<String>,
    dataset: Option<String>,
) -> Result<Value, String> {
    let mut args = vec!["forcing-directory-probe".into(), "--path".into(), path];
    if let Some(dataset) = dataset.filter(|value| !value.is_empty()) {
        args.extend(["--dataset".into(), dataset]);
    }
    let server = host
        .as_deref()
        .map(|host| server_for(app.clone(), host))
        .transpose()?;
    if let Some(server) = &server {
        args.extend([
            "--host".into(),
            server.host.clone(),
            "--root".into(),
            server.root.clone(),
        ]);
    }
    ssh_cli_json(args, resource_dir(&app), server.as_ref()).await
}
