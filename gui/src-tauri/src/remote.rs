//! 远程运行（R1）：服务器配置、测试连接、提交、查状态、取消、取回结果。实际工作都由
//! `colm-cli remote-*` 做（经用户自己的 ssh），这里只存配置、转调命令、把日志末尾解析成阶段与进度。

use std::collections::{BTreeMap, HashMap};
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
    /// 界面显示名称（例如"摆渡船"）；空时界面按集群预设或 host 显示。
    #[serde(default)]
    pub name: String,
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
    /// 每个节点的核数（摆渡船是 48）；0 表示不知道。节点数没指定而总核数超过它时，自动算出要占几个节点。
    #[serde(default)]
    pub cores_per_node: u32,
    /// 自动取回结果时只取这些变量（逗号分隔，例如 `f_fsena,f_rnet`）；空表示取回全部。
    #[serde(default)]
    pub fetch_vars: String,
    /// 作业开头先执行的环境准备，例如 `module load rust`。
    #[serde(default)]
    pub env_script: String,
    /// 原样追加的调度指令选项，每行一条，例如 `--constraint=cpu`。
    #[serde(default)]
    pub directives: Vec<String>,
    /// 新建模拟时预填的项目目录（服务器路径）；空表示沿用工作目录。
    #[serde(default)]
    pub project_dir: String,
    /// 新建模拟时预填的地表数据目录（服务器路径）；空表示不预填。
    #[serde(default)]
    pub rawdata_dir: String,
    /// 新建模拟时预填的 Runtime 数据目录。
    #[serde(default)]
    pub runtime_dir: String,
    /// 新建模拟时预填的强迫场数据路径。
    #[serde(default)]
    pub forcing_dir: String,
    /// 按模拟类型记住的上次用过的路径，见 [`KindPaths`]。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kind_paths: KindPaths,
}

/// 模拟类型（site/watershed/region/global）→ 字段名 → 上次用过的值。
///
/// 只由 `remote_remember_kind_paths`/`remote_forget_kind_path` 改；`remote_save_config`
/// 保存服务器列表时一律沿用文件里已有的值，编辑连接设置的界面不会把它清掉。
pub type KindPaths = BTreeMap<String, BTreeMap<String, String>>;

const PATH_KINDS: [&str; 4] = ["site", "watershed", "region", "global"];
/// 可以记住的字段。除 `forcing_dataset`（数据集名）外都是路径。
const KIND_PATH_FIELDS: [&str; 8] = [
    "sitedir",
    "forcingdir",
    "shapefile",
    "catchment",
    "mesh",
    "spatial_forcingdir",
    "forcing_dataset",
    "forcing",
];

/// 本机新建模拟时预填的数据路径，字段含义同 [`Server`] 的同名字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataDefaults {
    #[serde(default)]
    pub project_dir: String,
    #[serde(default)]
    pub rawdata_dir: String,
    #[serde(default)]
    pub runtime_dir: String,
    #[serde(default)]
    pub forcing_dir: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kind_paths: KindPaths,
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
    /// 本机的默认数据路径。保存时缺省表示沿用文件里已有的值，
    /// 这样只编辑服务器列表的界面不会把它清掉。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<DataDefaults>,
}

fn config_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("remote.json"))
}

fn cases_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("server-cases.json"))
}

/// 服务器配置"案例"：把一台服务器的非敏感配置存进软件，之后从"集群预设"里重新套用。
/// 密码本来就不在配置文件里；认证方式与私钥路径属于本机凭据，保存时清空（用户要求默认只带
/// 非敏感配置）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCase {
    pub id: String,
    /// 案例名称，默认用服务器的显示名称。
    pub name: String,
    pub server: Server,
    /// 保存时间（毫秒，UTC）。
    pub saved_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ServerCaseFile {
    #[serde(default)]
    cases: Vec<ServerCase>,
}

fn read_server_cases(path: &std::path::Path) -> Result<ServerCaseFile, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ServerCaseFile::default())
        }
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_server_cases(path: &std::path::Path, file: &ServerCaseFile) -> Result<(), String> {
    crate::config::write_export_text_atomic(
        path,
        serde_json::to_string_pretty(file).map_err(|error| error.to_string())?,
    )
}

/// 把服务器配置变成可保存的案例：只排除本机凭据（认证方式、私钥路径），其余按默认带上去。
pub(crate) fn case_from_server(mut server: Server, name: &str) -> Result<Server, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.contains(['\n', '\r']) {
        return Err("案例名称要填 1–80 个字符".into());
    }
    server.auth = "config".into();
    server.identity_file.clear();
    server.kind_paths.clear();
    Ok(server)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// 软件里保存的服务器案例（非敏感配置），给"集群预设"下拉复用。
#[tauri::command]
pub fn remote_cases(app: tauri::AppHandle) -> Result<Vec<ServerCase>, String> {
    Ok(read_server_cases(&cases_path(&app)?)?.cases)
}

/// 案例按名称去重：同一个名字再保存就更新，不堆重复项。
fn upsert_case(file: &mut ServerCaseFile, record: ServerCase) {
    match file.cases.iter_mut().find(|item| item.name == record.name) {
        Some(existing) => *existing = record,
        None => file.cases.push(record),
    }
}

/// 把一台服务器的配置保存为案例：同名覆盖，默认只排除认证方式与私钥路径。
#[tauri::command]
pub fn remote_case_save(
    app: tauri::AppHandle,
    name: String,
    server: Server,
) -> Result<ServerCase, String> {
    let server = case_from_server(server, &name)?;
    let path = cases_path(&app)?;
    let mut file = read_server_cases(&path)?;
    let saved_at = now_millis();
    let record = ServerCase {
        id: format!("case-{saved_at}-{}", file.cases.len()),
        name: name.trim().to_owned(),
        server,
        saved_at,
    };
    upsert_case(&mut file, record.clone());
    write_server_cases(&path, &file)?;
    Ok(record)
}

#[tauri::command]
pub fn remote_case_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let path = cases_path(&app)?;
    let mut file = read_server_cases(&path)?;
    let before = file.cases.len();
    file.cases.retain(|item| item.id != id);
    if file.cases.len() == before {
        return Err("案例不存在".into());
    }
    write_server_cases(&path, &file)
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
        if server.cores_per_node > 4096 {
            return Err(format!("{host} 的每节点核数不合理"));
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
    let mut config = config;
    validate(&config)?;
    let path = config_path(&app)?;
    let saved = read_remote_config(&path)?;
    keep_kind_paths(&mut config, saved);
    // 先写临时文件再替换：直接 `fs::write` 中途崩溃或磁盘满会把 remote.json 截断，服务器设置整份丢失。
    crate::config::write_export_text_atomic(
        &path,
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
    )?;
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

/// 保存时沿用文件里的 `local` 与各处的 `kind_paths`：它们由各自的命令维护，
/// 发来的配置里带的值（可能是界面加载时的旧副本）一律不算数。
pub(crate) fn keep_kind_paths(config: &mut RemoteConfig, saved: RemoteConfig) {
    let RemoteConfig {
        servers: saved_servers,
        local: saved_local,
    } = saved;
    let mut saved_paths: HashMap<String, KindPaths> = saved_servers
        .into_iter()
        .map(|server| (server.host, server.kind_paths))
        .collect();
    for server in &mut config.servers {
        server.kind_paths = saved_paths.remove(&server.host).unwrap_or_default();
    }
    let saved_local_paths = saved_local
        .as_ref()
        .map(|local| local.kind_paths.clone())
        .unwrap_or_default();
    match config.local.as_mut() {
        Some(local) => local.kind_paths = saved_local_paths,
        None => config.local = saved_local,
    }
}

fn kind_paths_of<'a>(
    config: &'a mut RemoteConfig,
    host: Option<&str>,
) -> Result<&'a mut KindPaths, String> {
    match host.map(str::trim).filter(|host| !host.is_empty()) {
        Some(host) => config
            .servers
            .iter_mut()
            .find(|server| server.host == host)
            .map(|server| &mut server.kind_paths)
            .ok_or_else(|| format!("计算资源里没有服务器 {host}")),
        None => Ok(&mut config
            .local
            .get_or_insert_with(DataDefaults::default)
            .kind_paths),
    }
}

/// 把新建模拟用过的、属于某一模拟类型的路径记到所选计算资源下（`host` 为空即本机）。
/// 空值不覆盖已有记录；返回是否有改动。
pub(crate) fn remember_kind_paths(
    config: &mut RemoteConfig,
    host: Option<&str>,
    kind: &str,
    paths: &BTreeMap<String, String>,
) -> Result<bool, String> {
    if !PATH_KINDS.contains(&kind) {
        return Err(format!("未知的模拟类型：{kind:?}"));
    }
    let remote = host.map(str::trim).is_some_and(|host| !host.is_empty());
    let mut clean = Vec::new();
    for (key, value) in paths {
        if !KIND_PATH_FIELDS.contains(&key.as_str()) {
            return Err(format!("不能记住的字段：{key:?}"));
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.contains(['\n', '\r', '\0']) || value.len() > 4096 {
            return Err(format!("{key} 的值不合法"));
        }
        let absolute = if remote {
            value.starts_with('/')
        } else {
            std::path::Path::new(value).is_absolute()
        };
        if key != "forcing_dataset" && !absolute {
            return Err(format!("{key} 要是绝对路径：{value:?}"));
        }
        clean.push((key.clone(), value.to_owned()));
    }
    let slot = kind_paths_of(config, host)?
        .entry(kind.to_owned())
        .or_default();
    let mut changed = false;
    for (key, value) in clean {
        if slot.get(&key) != Some(&value) {
            slot.insert(key, value);
            changed = true;
        }
    }
    Ok(changed)
}

/// 忘掉某一类型下的一项记录；`key` 为空时忘掉这个类型的全部记录。
pub(crate) fn forget_kind_path(
    config: &mut RemoteConfig,
    host: Option<&str>,
    kind: &str,
    key: Option<&str>,
) -> Result<bool, String> {
    let paths = kind_paths_of(config, host)?;
    let changed = match key {
        Some(key) => {
            let removed = paths
                .get_mut(kind)
                .is_some_and(|slot| slot.remove(key).is_some());
            if paths.get(kind).is_some_and(BTreeMap::is_empty) {
                paths.remove(kind);
            }
            removed
        }
        None => paths.remove(kind).is_some(),
    };
    Ok(changed)
}

fn update_remote_config(
    app: &tauri::AppHandle,
    change: impl FnOnce(&mut RemoteConfig) -> Result<bool, String>,
) -> Result<bool, String> {
    let path = config_path(app)?;
    let mut config = read_remote_config(&path)?;
    let changed = change(&mut config)?;
    if changed {
        crate::config::write_export_text_atomic(
            &path,
            serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(changed)
}

#[tauri::command]
pub fn remote_remember_kind_paths(
    app: tauri::AppHandle,
    host: Option<String>,
    kind: String,
    paths: BTreeMap<String, String>,
) -> Result<bool, String> {
    update_remote_config(&app, |config| {
        remember_kind_paths(config, host.as_deref(), &kind, &paths)
    })
}

#[tauri::command]
pub fn remote_forget_kind_path(
    app: tauri::AppHandle,
    host: Option<String>,
    kind: String,
    key: Option<String>,
) -> Result<bool, String> {
    update_remote_config(&app, |config| {
        forget_kind_path(config, host.as_deref(), &kind, key.as_deref())
    })
}

/// 新建模拟用过的数据路径写回所选计算资源（`host` 为空即本机）；空值不覆盖已有默认。
pub(crate) fn remember_data_paths(
    config: &mut RemoteConfig,
    host: Option<&str>,
    paths: [Option<&str>; 4],
) -> Result<bool, String> {
    let [project, rawdata, runtime, forcing] = paths;
    let host = host.map(str::trim).filter(|host| !host.is_empty());
    for value in paths.into_iter().flatten() {
        let value = value.trim();
        let absolute = if host.is_some() {
            value.starts_with('/')
        } else {
            std::path::Path::new(value).is_absolute()
        };
        if !value.is_empty() && (!absolute || value.contains(['\n', '\r', '\0'])) {
            return Err(format!("默认数据路径要是绝对路径：{value:?}"));
        }
    }
    let local = config.local.get_or_insert_with(DataDefaults::default);
    let (project_dir, rawdata_dir, runtime_dir, forcing_dir) = match host {
        Some(host) => {
            let server = config
                .servers
                .iter_mut()
                .find(|server| server.host == host)
                .ok_or_else(|| format!("计算资源里没有服务器 {host}"))?;
            (
                &mut server.project_dir,
                &mut server.rawdata_dir,
                &mut server.runtime_dir,
                &mut server.forcing_dir,
            )
        }
        None => (
            &mut local.project_dir,
            &mut local.rawdata_dir,
            &mut local.runtime_dir,
            &mut local.forcing_dir,
        ),
    };
    let mut changed = false;
    for (slot, value) in [
        (project_dir, project),
        (rawdata_dir, rawdata),
        (runtime_dir, runtime),
        (forcing_dir, forcing),
    ] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            if slot != value {
                *slot = value.to_owned();
                changed = true;
            }
        }
    }
    Ok(changed)
}

#[tauri::command]
pub fn remote_remember_data_paths(
    app: tauri::AppHandle,
    host: Option<String>,
    project: Option<String>,
    rawdata: Option<String>,
    runtime: Option<String>,
    forcing: Option<String>,
) -> Result<bool, String> {
    let path = config_path(&app)?;
    let mut config = read_remote_config(&path)?;
    let changed = remember_data_paths(
        &mut config,
        host.as_deref(),
        [
            project.as_deref(),
            rawdata.as_deref(),
            runtime.as_deref(),
            forcing.as_deref(),
        ],
    )?;
    if changed {
        crate::config::write_export_text_atomic(
            &path,
            serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(changed)
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
    // 旧配置可能保留了切到 config/password 前选过的密钥路径；CLI 的 Auth::validate
    // 只在 key 模式接受 identity_file，这里必须按当前认证方式重新决定是否发送。
    let identity_file = if server.auth == "key" {
        server.identity_file.as_str()
    } else {
        ""
    };
    Ok(
        json!({"host": server.host, "username": server.username, "port": server.port,
        "auth": server.auth, "identity_file": identity_file, "password": password}),
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
                local: None,
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
        local: None,
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

        ssh_cli_json_untracked(
            args,
            resources,
            server,
            run.map(|(_, _, case)| case.to_owned()),
        )
        .await
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
    case: Option<String>,
) -> Result<Value, String> {
    let payload = server.map(auth_payload).transpose()?;
    let password = payload
        .as_ref()
        .and_then(|p| p["password"].as_str())
        .map(str::to_owned);
    let payload = payload.map(|p| p.to_string());
    let mut command = std::process::Command::new(crate::sidecar::resolve_cli());
    command.args(&args).env("COLM_AUDIT_BY_PARENT", "1");
    // The server's environment preparation (module load, PATH) applies to every command run there.
    if let Some(server) = server.filter(|server| !server.env_script.trim().is_empty()) {
        command.env("COLM_REMOTE_ENV_SCRIPT", &server.env_script);
    }
    if payload.is_some() {
        command
            .env("COLM_SSH_AUTH_STDIN", "1")
            .stdin(Stdio::piped());
    }
    if let Some(dir) = resources {
        command.env("COLM_RESOURCE_DIR", dir);
    }
    // 远程子进程也要自成一个进程组：关窗/退出时按组终止，SSH 与远端提交一起停。
    colm_kernel::run::top_level_sidecar(&mut command);
    tauri::async_runtime::spawn_blocking(move || {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("无法启动 colm-cli：{e}"))?;
        let _remote_child = crate::sidecar::register_remote_child(child.id(), case);
        if let Some(payload) = payload {
            let result = child.stdin.take().ok_or("no stdin").and_then(|mut stdin| {
                writeln!(stdin, "{payload}").map_err(|_| "无法传递 SSH 登录信息")
            });
            if let Err(error) = result {
                let _ = crate::sidecar::kill_remote_child(child.id());
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
        local: None,
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
        local: None,
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
    if server.cores_per_node > 0 {
        args.push("--cores-per-node".into());
        args.push(server.cores_per_node.to_string());
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
    if server.cores_per_node > 0 {
        args.push("--cores-per-node".into());
        args.push(server.cores_per_node.to_string());
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
        local: None,
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

/// 一次提交的结果，给同一批次、同一站点的重复请求共用。
type SharedOutcome = std::sync::Arc<(Mutex<Option<Result<Value, String>>>, std::sync::Condvar)>;
static SUBMISSIONS: OnceLock<Mutex<std::collections::HashMap<String, SharedOutcome>>> =
    OnceLock::new();

/// 同一批次、同一站点的"提交运行"只执行一次。Tauri 前端调用在自定义协议出错时会改走 postMessage，
/// 把同一个请求再发一次（tauri `scripts/ipc-protocol.js`）。land02 实测（2026-10-19）：5 个站点的提交
/// 等登录节点编引擎 3 分多钟，21:59:44 同时被重发，AU-Cpr 交了两个作业，其余留下永远"运行中"的记录。
/// 重复的请求不再提交，等第一个的结果原样返回；记录在本次运行期间一直保留，晚到的重发也一样处理。
fn claim_submission(key: &str) -> (SharedOutcome, bool) {
    let mut map = SUBMISSIONS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(outcome) = map.get(key) {
        return (outcome.clone(), false);
    }
    let outcome: SharedOutcome = Default::default();
    map.insert(key.to_owned(), outcome.clone());
    (outcome, true)
}

fn settle_submission(outcome: &SharedOutcome, result: &Result<Value, String>) {
    let (slot, ready) = &**outcome;
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result.clone());
    ready.notify_all();
}

/// 第一个请求不管怎么结束——正常返回、出错、panic、任务被丢弃——都给等着的重复请求一个回答，
/// 否则它们的阻塞线程会一直等下去。
struct SettleOnDrop {
    outcome: SharedOutcome,
    settled: bool,
}

impl Drop for SettleOnDrop {
    fn drop(&mut self) {
        if !self.settled {
            settle_submission(
                &self.outcome,
                &Err("提交在完成前被中断；请先核对服务器作业状态再重试 / The submission was interrupted; check the server job before retrying".into()),
            );
        }
    }
}

fn wait_for_submission(outcome: &SharedOutcome) -> Result<Value, String> {
    let (slot, ready) = &**outcome;
    let mut value = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    while value.is_none() {
        value = ready
            .wait(value)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    value.clone().expect("checked above")
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
    pack: Option<bool>,
) -> Result<Value, String> {
    let Some(id) = run_id.clone() else {
        return submit_run(
            app,
            None,
            case,
            host,
            kernel,
            stage,
            force,
            engine,
            ranks,
            threads,
            cpus,
            scheduler,
            model_version,
            model_revision,
            preprocessors,
            record_input_fingerprints,
            pack,
        )
        .await;
    };
    let (outcome, first) = claim_submission(&format!("{id}\n{case}"));
    if !first {
        return tauri::async_runtime::spawn_blocking(move || wait_for_submission(&outcome))
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut guard = SettleOnDrop {
        outcome: outcome.clone(),
        settled: false,
    };
    let result = submit_run(
        app,
        run_id,
        case,
        host,
        kernel,
        stage,
        force,
        engine,
        ranks,
        threads,
        cpus,
        scheduler,
        model_version,
        model_revision,
        preprocessors,
        record_input_fingerprints,
        pack,
    )
    .await;
    settle_submission(&outcome, &result);
    guard.settled = true;
    result
}

#[allow(clippy::too_many_arguments)]
async fn submit_run(
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
    pack: Option<bool>,
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
    // 同一批的站点共用一个节点队列，队列名就是这次批量运行的编号。
    if let (Some(true), Some(run_id)) = (pack, &run_id) {
        args.extend(["--pack-batch".into(), run_id.clone()]);
    }
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

/// 服务器上正在编的引擎编到哪一步：等待引擎时每隔几秒问一次，画进度条。
#[tauri::command]
pub async fn remote_engine_progress(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    let server = server_for(app.clone(), &host)?;
    validate_connection(&server)?;
    ssh_cli_json(
        vec![
            "remote-engine-progress".into(),
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

/// "核对服务器状态"时一并看服务器忙不忙（`colm-cli remote-load`）。只读，不进历史。
#[tauri::command]
pub async fn remote_load(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    let server = server_for(app.clone(), &host)?;
    validate_connection(&server)?;
    ssh_cli_json(
        vec!["remote-load".into(), "--host".into(), server.host.clone()],
        resource_dir(&app),
        Some(&server),
    )
    .await
}

/// 结果还在服务器上、本机没取回的运行目录：返回它的服务器。结果页对这样的运行改在服务器上读
/// （`colm-cli remote-read`），只把算好的 JSON 传回来；取回过的照旧读本机。
pub(crate) fn results_on_server(case: &str) -> Option<String> {
    let dir = std::path::Path::new(case);
    if crate::model::has_local_history(dir) {
        return None;
    }
    let record: Value =
        serde_json::from_slice(&std::fs::read(dir.join(".colm-remote.json")).ok()?).ok()?;
    record["host"].as_str().map(str::to_owned)
}

/// 评估该在哪台服务器上算：结果还在服务器上；或者结果已取回、但观测文件只在服务器上（服务器项目的默认观测是
/// 服务器路径，本机打不开）。本机运行、或观测在本机的，返回 `None`，照旧在本机算。
pub(crate) fn evaluation_server(case: &str, obs: &str) -> Option<String> {
    if let Some(host) = results_on_server(case) {
        return Some(host);
    }
    if std::path::Path::new(obs).is_file() {
        return None;
    }
    let record: Value = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(case).join(".colm-remote.json")).ok()?,
    )
    .ok()?;
    record["host"].as_str().map(str::to_owned)
}

pub(crate) async fn read_on_server(
    app: &tauri::AppHandle,
    host: &str,
    case: String,
    command: &str,
    flags: Vec<(&str, String)>,
) -> Result<String, String> {
    let server = server_for(app.clone(), host)?;
    validate_connection(&server)?;
    let mut args = vec![
        "remote-read".to_owned(),
        case,
        "--command".to_owned(),
        command.to_owned(),
    ];
    for (flag, value) in flags {
        args.extend([flag.to_owned(), value]);
    }
    let value = ssh_cli_json(args, resource_dir(app), Some(&server)).await?;
    serde_json::to_string(&value).map_err(|e| e.to_string())
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

// ---- 远程研究的失败阶段与可执行下一步（P1-9 B）----------------------------------------------
//
// 构建/创建研究失败时作业非零退出、服务器上没有研究目录。旧组合里：原记录阻止重提、界面允许
// 取回、取回却要求研究目录存在（报一句 no such directory 就结束）、关闭记录又拒绝所有已结束
// 作业 —— 用户没有任何出口。这里只在 GUI 侧读 CLI 已经公开的字段做分类，不改命令行接口。
//
// 只读的字段（`remote-study-status` 的回复）：`record`/`pending` 是否存在、作业
// `status.state` / `exit_code` / `phase` / `detail`、服务器上有没有研究会话目录（`study` 非 null）。
// 限制：`phase` 是作业脚本写的三个英文阶段名（preparing the engine / creating the Study /
// running the Study），旧作业或缺阶段标记时只能按“有没有研究目录”粗分；更细的原因要到日志尾部看。

/// 服务器作业停在哪个阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RemoteStudyOutcome {
    /// 提交从未到达服务器：没有作业，也没有研究目录。
    NotSubmitted,
    /// 作业存在，但在写第一个阶段标记前就退出：提交/启动阶段失败。
    StartFailed,
    /// 阶段停在 `preparing the engine`：服务器构建引擎失败。
    BuildFailed,
    /// 阶段停在 `creating the Study`：研究创建失败，服务器上没有研究目录。
    StudyCreateFailed,
    /// 阶段已到 `running the Study`：研究建好、成员运行失败（可以取回诊断结果）。
    MemberRunFailed,
    Succeeded,
    Cancelled,
    Lost,
    Running,
    Unknown,
}

/// 界面上按钮的放行矩阵。存档条件在 [`remote_study_recovery`] 里按状态算好，界面只照着开关。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RemoteStudyPermissions {
    /// 没有服务器记录，可以直接提交。
    pub submit: bool,
    /// 有“已确认失败”的终态记录：GUI 先归档旧记录再重新提交（修复后重提）。
    pub resubmit: bool,
    pub cancel: bool,
    /// 已确认结束且没有取回过：成功结果或失败诊断都从这里取回。
    pub fetch: bool,
    /// 可以归档关闭记录（不改服务器，只把本机记录改名留档）。
    pub close: bool,
    pub poll: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RemoteStudyRecovery {
    pub outcome: RemoteStudyOutcome,
    /// 作业已确认结束（有退出码）；`lost`/`unknown` 不算确认。
    pub terminal: bool,
    /// 作业状态是 `finished`（有退出码）。归档时用它区分“CLI close 会拒绝的已结束作业”。
    pub finished: bool,
    /// 服务器上已经有研究会话目录（取回的前提）。
    pub study_created: bool,
    pub fetched: bool,
    pub exit_code: Option<i32>,
    pub phase: Option<String>,
    pub job: Option<String>,
    pub permissions: RemoteStudyPermissions,
    pub next_zh: String,
    pub next_en: String,
    /// 取回会被拒绝时的可行动说明（没有研究目录、作业还在跑等）；能取回时为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_error_zh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_error_en: Option<String>,
}

fn object_at<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|value| value.is_object())
}

fn job_state(status: Option<&Value>) -> Option<&str> {
    status
        .and_then(|status| status.get("state"))
        .and_then(Value::as_str)
}

/// 分类结果给日志/拒绝文案用的中文名。
fn study_outcome_zh(outcome: RemoteStudyOutcome) -> &'static str {
    match outcome {
        RemoteStudyOutcome::NotSubmitted => "从未提交",
        RemoteStudyOutcome::StartFailed => "启动失败",
        RemoteStudyOutcome::BuildFailed => "构建失败",
        RemoteStudyOutcome::StudyCreateFailed => "研究创建失败",
        RemoteStudyOutcome::MemberRunFailed => "成员运行失败",
        RemoteStudyOutcome::Succeeded => "成功结束",
        RemoteStudyOutcome::Cancelled => "已取消",
        RemoteStudyOutcome::Lost => "已丢失",
        RemoteStudyOutcome::Running => "运行中",
        RemoteStudyOutcome::Unknown => "无法核对",
    }
}

fn study_outcome_en(outcome: RemoteStudyOutcome) -> &'static str {
    match outcome {
        RemoteStudyOutcome::NotSubmitted => "never submitted",
        RemoteStudyOutcome::StartFailed => "startup failed",
        RemoteStudyOutcome::BuildFailed => "build failed",
        RemoteStudyOutcome::StudyCreateFailed => "Study creation failed",
        RemoteStudyOutcome::MemberRunFailed => "member run failed",
        RemoteStudyOutcome::Succeeded => "finished",
        RemoteStudyOutcome::Cancelled => "cancelled",
        RemoteStudyOutcome::Lost => "lost",
        RemoteStudyOutcome::Running => "running",
        RemoteStudyOutcome::Unknown => "unconfirmed",
    }
}

/// 失败阶段：先认作业脚本写的阶段标记，认不出时按“服务器上有没有研究目录”粗分。
fn study_failure_stage(phase: Option<&str>, study_created: bool) -> RemoteStudyOutcome {
    match phase {
        Some("preparing the engine") => RemoteStudyOutcome::BuildFailed,
        Some("creating the Study") => RemoteStudyOutcome::StudyCreateFailed,
        Some("running the Study") => RemoteStudyOutcome::MemberRunFailed,
        // 作业体只写这三个阶段；不认识的状态（旧作业、脚本中途死掉）按有没有研究目录归类：
        // 没有目录说明还没走到成员运行，属于创建/启动前的失败。
        _ if study_created => RemoteStudyOutcome::MemberRunFailed,
        _ => RemoteStudyOutcome::StartFailed,
    }
}

/// 把 `remote-study-status` 的回复分类成“到了哪一步、下一步能做什么”。
pub(crate) fn remote_study_recovery(reply: &Value) -> RemoteStudyRecovery {
    let record = object_at(reply, "record");
    let pending = object_at(reply, "pending");
    let status = object_at(reply, "status");
    let state = job_state(status);
    let exit_code = status
        .and_then(|status| status.get("exit_code"))
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());
    let phase = status
        .and_then(|status| status.get("phase"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|phase| !phase.is_empty())
        .map(str::to_owned);
    let detail = status
        .and_then(|status| status.get("detail"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let study_created = reply.get("study").is_some_and(Value::is_object);
    let fetched = record
        .and_then(|record| record.get("fetched_at"))
        .is_some_and(|fetched| !fetched.is_null());
    let job = record
        .and_then(|record| record.get("job"))
        .or_else(|| pending.and_then(|pending| pending.get("job")))
        .and_then(Value::as_str)
        .filter(|job| !job.is_empty())
        .map(str::to_owned);
    let exists = record.is_some() || pending.is_some();
    let finished = state == Some("finished");
    let terminal = finished && exit_code.is_some();
    let cancelled = detail.to_ascii_uppercase().contains("CANCELLED");
    let outcome = if reply.get("outcome").and_then(Value::as_str) == Some("never_submitted") {
        RemoteStudyOutcome::NotSubmitted
    } else if matches!(state, Some("queued" | "running")) {
        RemoteStudyOutcome::Running
    } else if finished {
        match exit_code {
            Some(0) => RemoteStudyOutcome::Succeeded,
            _ => study_failure_stage(phase.as_deref(), study_created),
        }
    } else if state == Some("lost") && cancelled {
        RemoteStudyOutcome::Cancelled
    } else if state == Some("lost") {
        RemoteStudyOutcome::Lost
    } else {
        RemoteStudyOutcome::Unknown
    };
    let failed = matches!(
        outcome,
        RemoteStudyOutcome::StartFailed
            | RemoteStudyOutcome::BuildFailed
            | RemoteStudyOutcome::StudyCreateFailed
            | RemoteStudyOutcome::MemberRunFailed
    );
    let permissions = RemoteStudyPermissions {
        submit: !exists,
        resubmit: record.is_some() && failed,
        cancel: record.is_some() && matches!(outcome, RemoteStudyOutcome::Running) && !fetched,
        fetch: terminal && !fetched,
        close: if fetched {
            exists
        } else {
            match outcome {
                // 成功结束必须先取回：关闭本地记录是学生研究结果的唯一入口，不能先丢。
                RemoteStudyOutcome::Succeeded => false,
                RemoteStudyOutcome::Running | RemoteStudyOutcome::NotSubmitted => false,
                // 已结束但失败、已取消、已丢失、状态无法核对：用户确认后可以归档留档。
                RemoteStudyOutcome::StartFailed
                | RemoteStudyOutcome::BuildFailed
                | RemoteStudyOutcome::StudyCreateFailed
                | RemoteStudyOutcome::MemberRunFailed
                | RemoteStudyOutcome::Cancelled
                | RemoteStudyOutcome::Lost
                | RemoteStudyOutcome::Unknown => exists,
            }
        },
        poll: exists
            && !fetched
            && !terminal
            && !matches!(
                outcome,
                RemoteStudyOutcome::Lost | RemoteStudyOutcome::Cancelled
            ),
    };
    let (next_zh, next_en) = study_next_step(outcome);
    let fetch_error = study_fetch_error(outcome, exit_code);
    RemoteStudyRecovery {
        outcome,
        terminal,
        finished,
        study_created,
        fetched,
        exit_code,
        phase,
        job,
        permissions,
        next_zh,
        next_en,
        fetch_error_zh: fetch_error.as_ref().map(|(zh, _)| zh.clone()),
        fetch_error_en: fetch_error.map(|(_, en)| en),
    }
}

/// 每种状态给一步可执行动作：重提、看日志、取回诊断、归档关闭或重新核对。
fn study_next_step(outcome: RemoteStudyOutcome) -> (String, String) {
    match outcome {
        RemoteStudyOutcome::NotSubmitted => (
            "提交没有到达服务器：服务器上没有这次提交的作业。修正设置后可以直接重新提交。".into(),
            "The submission never reached the server: there is no such job. Fix the settings and submit again.".into(),
        ),
        RemoteStudyOutcome::StartFailed => (
            "作业在开始构建前就退出了（构建/启动阶段失败），服务器上没有研究目录。看下方日志尾部定位原因；修正后归档关闭这条记录并重新提交。".into(),
            "The job exited before the build started (startup failed), so no Study directory exists. Check the log tail below; after fixing, close this record and submit again.".into(),
        ),
        RemoteStudyOutcome::BuildFailed => (
            "服务器在构建引擎阶段失败，没有创建研究目录。看下方日志尾部定位原因；修正后归档关闭这条记录并重新提交。".into(),
            "The server failed while building the engine, so no Study directory was created. Check the log tail below; after fixing, close this record and submit again.".into(),
        ),
        RemoteStudyOutcome::StudyCreateFailed => (
            "服务器在创建研究阶段失败，没有创建研究目录。看下方日志尾部定位原因；修正后归档关闭这条记录并重新提交。".into(),
            "The server failed while creating the Study, so no Study directory was created. Check the log tail below; after fixing, close this record and submit again.".into(),
        ),
        RemoteStudyOutcome::MemberRunFailed => (
            "研究已在服务器上建好，但成员运行失败。可以取回诊断结果，或归档关闭这条记录后修正重提。".into(),
            "The Study was created on the server but its members failed. Fetch the diagnostic results, or close this record and resubmit after fixing.".into(),
        ),
        RemoteStudyOutcome::Succeeded => (
            "服务器研究成功结束。取回结果；取回后可以归档关闭这条远程记录。".into(),
            "The server Study finished successfully. Fetch its results; after fetching, this remote record can be archived.".into(),
        ),
        RemoteStudyOutcome::Cancelled => (
            "服务器作业已取消并退出，没有可核验的研究结果。可以归档关闭这条记录后重新提交。".into(),
            "The server job was cancelled and exited; there are no verified results. Close this record and submit again if needed.".into(),
        ),
        RemoteStudyOutcome::Lost => (
            "服务器上已经查不到这个作业（或没有退出码），结果无法再取回。确认作业丢失后归档关闭记录。".into(),
            "The server can no longer find this job (or has no exit code), so its results cannot be fetched. Confirm it is lost and close the record.".into(),
        ),
        RemoteStudyOutcome::Running => (
            "服务器作业还在排队或运行。等它结束再取回；要停止请先请求取消，确认退出后才能关闭记录。".into(),
            "The server job is still queued or running. Fetch after it ends; to stop it, request cancellation and close the record only after it exits.".into(),
        ),
        RemoteStudyOutcome::Unknown => (
            "服务器状态还无法核对，不能当作完成、失败或已取消。先重新核对状态；确认作业已经丢失后才能归档关闭。".into(),
            "The server state is unconfirmed, so it is not treated as finished, failed or cancelled. Check the status again; close the record only once the job is confirmed lost.".into(),
        ),
    }
}

/// 取回会被拒绝时的可行动说明。成功结束/成员运行失败可以取回，不会走到这里。
fn study_fetch_error(
    outcome: RemoteStudyOutcome,
    exit_code: Option<i32>,
) -> Option<(String, String)> {
    let code = exit_code.unwrap_or(-1);
    match outcome {
        // 构建/创建阶段失败：服务器上没有研究目录，CLI 取回只会报“没有创建研究”，
        // 用户看不出下一步。这里直接说清可看的日志与可做的动作。
        RemoteStudyOutcome::StartFailed => Some((
            format!(
                "服务器上没有研究目录（启动/构建准备阶段失败，退出码 {code}），没有结果可取回。可查看下方作业日志尾部；或归档关闭这条记录后修正重提。"
            ),
            format!(
                "The server has no Study directory (startup failed, exit code {code}), so there is nothing to fetch. Check the job log tail below, or close this record and retry after fixing."
            ),
        )),
        RemoteStudyOutcome::BuildFailed => Some((
            format!(
                "服务器上没有研究目录（构建引擎阶段失败，退出码 {code}），没有结果可取回。可查看下方作业日志尾部；或归档关闭这条记录后修正重提。"
            ),
            format!(
                "The server has no Study directory (engine build failed, exit code {code}), so there is nothing to fetch. Check the job log tail below, or close this record and retry after fixing."
            ),
        )),
        RemoteStudyOutcome::StudyCreateFailed => Some((
            format!(
                "服务器上没有研究目录（创建研究阶段失败，退出码 {code}），没有结果可取回。可查看下方作业日志尾部；或归档关闭这条记录后修正重提。"
            ),
            format!(
                "The server has no Study directory (Study creation failed, exit code {code}), so there is nothing to fetch. Check the job log tail below, or close this record and retry after fixing."
            ),
        )),
        RemoteStudyOutcome::Running => Some((
            "服务器作业还没结束，取回要等它退出。可以继续核对状态，或先请求取消。".into(),
            "The server job has not ended yet; fetch only after it exits. Check the status again, or request cancellation first.".into(),
        )),
        RemoteStudyOutcome::Lost | RemoteStudyOutcome::Cancelled => Some((
            "服务器作业没有正常退出（已取消或已丢失），没有可核验的退出码与结果。需要重新运行请先归档关闭这条记录。".into(),
            "The server job did not exit normally (cancelled or lost), so there is no verified exit code or result to fetch. Close this record before running it again.".into(),
        )),
        RemoteStudyOutcome::Succeeded | RemoteStudyOutcome::NotSubmitted => Some((
            "这次取回不需要或已经完成：提交从未到达服务器时没有结果可取回；已取回过的结果不用重复取回。".into(),
            "There is nothing to fetch here: a submission that never reached the server has no results, and results already fetched need not be fetched again.".into(),
        )),
        RemoteStudyOutcome::Unknown => Some((
            "服务器状态还无法核对，不能确认作业已经结束。先重新核对状态，确认后再取回。".into(),
            "The server state is unconfirmed, so the job's completion cannot be trusted. Check the status again before fetching.".into(),
        )),
        RemoteStudyOutcome::MemberRunFailed => None,
    }
}

/// 把分类结果写进 `remote-study-status` 的回复（前端按公开字段渲染，后端不用再算一遍）。
pub(crate) fn attach_study_recovery(reply: &mut Value) -> RemoteStudyRecovery {
    let recovery = remote_study_recovery(reply);
    if let Some(object) = reply.as_object_mut() {
        object.insert(
            "recovery".into(),
            serde_json::to_value(&recovery).unwrap_or(Value::Null),
        );
    }
    recovery
}

/// 取回之前先分类：没有研究目录/作业没结束时直接给可行动说明，不再落到 CLI 的
/// “the server job did not create the Study; see its log”。成功结束与成员运行失败照常取回。
///
/// 注意这是“拒绝并解释”，不是“权限矩阵里的 fetch=false”：构建/创建失败的按钮仍然可点
/// （矩阵里 fetch=true），点了正好看到“没有研究目录 + 看日志/归档关闭/重提”的说明，
/// 否则用户只会看到一个灰按钮，不知道为什么。
fn fetch_blocked_message(recovery: &RemoteStudyRecovery) -> Option<String> {
    if let (Some(zh), Some(en)) = (&recovery.fetch_error_zh, &recovery.fetch_error_en) {
        return Some(format!("{zh} {en}"));
    }
    if recovery.permissions.fetch {
        return None;
    }
    Some("现在不能取回。 Nothing can be fetched now.".into())
}

/// 关闭被拒绝时的说明：说清必须先做什么，而不是只说“不能关闭”。
fn close_blocked_message(recovery: &RemoteStudyRecovery) -> String {
    let (zh, en) = match recovery.outcome {
        RemoteStudyOutcome::Running => (
            "服务器作业还在排队或运行：先请求取消，等它确认退出后再归档关闭记录。",
            "The server job is still queued or running: request cancellation first, then archive the record after it exits.",
        ),
        RemoteStudyOutcome::Succeeded if !recovery.fetched => (
            "服务器研究已成功结束：先取回结果再归档关闭，否则唯一一次取回机会就没了。",
            "The server Study finished successfully: fetch its results before archiving, otherwise the only chance to fetch them is lost.",
        ),
        RemoteStudyOutcome::NotSubmitted => (
            "这次提交从未到达服务器，没有需要关闭的服务器作业；可以直接重新提交。",
            "This submission never reached the server, so there is no server job to close; submit again directly.",
        ),
        _ => (
            "现在不能归档关闭这条记录；先重新核对状态。",
            "This record cannot be archived now; check the server status again first.",
        ),
    };
    format!("{zh} {en}")
}

/// 本机记录与刚核验过的服务器状态必须指向同一个作业：归档是不可逆的本地动作，不能凭
/// 过期/别的记录里的 job 字段动手。`remote-study-status` 已经让 CLI 核对了作业身份
/// （`cancel_identity_script`），这里只再对齐本机记录里的 job 与工作目录。
fn verify_local_record_matches_status(study_dir: &str, status: &Value) -> Result<(), String> {
    let path = PathBuf::from(study_dir).join(".colm-remote-study.json");
    let text =
        std::fs::read_to_string(&path).map_err(|_| "这个研究没有服务器提交记录".to_string())?;
    let local: Value =
        serde_json::from_str(&text).map_err(|_| "服务器研究记录不合法".to_string())?;
    let local_job = local.get("job").and_then(Value::as_str);
    let local_root = local.get("root").and_then(Value::as_str);
    let verified = status.get("record");
    let verified_job = verified
        .and_then(|record| record.get("job"))
        .and_then(Value::as_str);
    let verified_root = verified
        .and_then(|record| record.get("root"))
        .and_then(Value::as_str);
    if local_job.is_none()
        || local_job != verified_job
        || local_root.is_none()
        || local_root != verified_root
    {
        return Err(format!(
            "服务器状态与本机记录对不上（本机作业 {}，服务器核对的是 {}）；先重新核对状态，未归档。",
            local_job.unwrap_or("—"),
            verified_job.unwrap_or("—")
        ));
    }
    Ok(())
}

/// 归档本机远程研究记录：改名留档，不删数据。命名与 CLI `remote-study-close` 一致
/// （`.colm-remote-study-closed-<stamp>.json`、`.colm-remote-study-pending-closed-<stamp>.json`），
/// 两个工具留下的留档文件可以互认。
///
/// 只动活动记录与待核对意图；`.colm-remote-study-source.json` 刻意不在名单里。取回成功后
/// 本机没有成员原始输出，写回版本要靠这份持久来源回服务器核验；把它一起归档会让“归档”看起来
/// 成功，之后写回却退回本机核验并失败。
fn archive_study_record(study_dir: &str) -> Result<Vec<String>, String> {
    let directory = PathBuf::from(study_dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut archived = Vec::new();
    for (name, suffix) in [
        (".colm-remote-study.json", ""),
        (".colm-remote-study-pending.json", "-pending"),
    ] {
        let path = directory.join(name);
        if !path.is_file() {
            continue;
        }
        let target = directory.join(format!(".colm-remote-study{suffix}-closed-{stamp}.json"));
        std::fs::rename(&path, &target)
            .map_err(|error| format!("无法归档服务器研究记录 {}: {error}", path.display()))?;
        archived.push(target.to_string_lossy().into_owned());
    }
    Ok(archived)
}

/// 重新提交之前：已有“已确认失败”的终态记录就先归档，让 CLI 的“已有服务器提交”检查通过。
/// 只对确认失败（有退出码、非零）这么做；排队/运行中/未确认丢失的记录一律拒绝重复提交。
async fn archive_failed_remote_study_before_submit(
    study_dir: &str,
    server: &Server,
) -> Result<(), String> {
    if !PathBuf::from(study_dir)
        .join(".colm-remote-study.json")
        .is_file()
    {
        return Ok(());
    }
    let mut status = ssh_cli_json(
        vec!["remote-study-status".into(), study_dir.to_string()],
        None,
        Some(server),
    )
    .await?;
    let recovery = attach_study_recovery(&mut status);
    if recovery.permissions.resubmit {
        verify_local_record_matches_status(study_dir, &status)?;
        archive_study_record(study_dir)?;
        return Ok(());
    }
    let (next_zh, next_en) = study_next_step(recovery.outcome);
    Err(format!(
        "{next_zh} {next_en}（服务器状态 / server state: {}/{}）",
        study_outcome_zh(recovery.outcome),
        study_outcome_en(recovery.outcome)
    ))
}

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
            local: None,
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
    // 原记录会让 CLI 拒绝再次提交；已确认失败（有非零退出码）的终态记录先归档再提交，
    // 这是“修复后重提”的唯一出口。排队/运行中/未确认丢失的记录仍然拒绝，不能重复提交。
    archive_failed_remote_study_before_submit(&study_dir, &server).await?;
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
    let mut reply = ssh_cli_json(
        vec!["remote-study-status".into(), study_dir],
        None,
        Some(&server),
    )
    .await?;
    // 阶段/权限/下一步放在回复里一次算好：前端不需要再解析英文阶段名，也不会两处逻辑分叉。
    attach_study_recovery(&mut reply);
    Ok(reply)
}

/// 跑完后取回：核对成员一致，把状态与结果移植进本机研究。
#[tauri::command]
pub async fn remote_study_fetch(app: tauri::AppHandle, study_dir: String) -> Result<Value, String> {
    absolute_study(&study_dir)?;
    let server = study_server(&remote_config(app)?, &study_dir)?;
    // 先分类再取回：构建/创建研究失败时服务器上没有研究目录，CLI 只会报一句
    // “did not create the Study”，界面拿不到“看日志/归档关闭/重试”的下一步。
    let mut status = ssh_cli_json(
        vec!["remote-study-status".into(), study_dir.clone()],
        None,
        Some(&server),
    )
    .await?;
    let recovery = attach_study_recovery(&mut status);
    if let Some(message) = fetch_blocked_message(&recovery) {
        return Err(message);
    }
    ssh_cli_json(
        vec!["remote-study-fetch".into(), study_dir],
        None,
        Some(&server),
    )
    .await
}

/// 关闭/归档本机远程研究记录。
///
/// 旧实现把关闭委托给 CLI，而 CLI 只允许 Lost/Unknown：已结束但失败的作业（构建/创建/成员失败）
/// 和已取回的成功结果都永远关不掉，记录一直挡着重提。这里先读一次状态并分类：
/// - 已取回，或已确认结束但失败：CLI 会拒绝，GUI 在核对过作业身份后直接归档本机记录
///   （改名留档、不删数据，命名与 CLI 一致）；
/// - 丢失/无法核对：仍走 CLI close，保留它对服务器作业身份的核对语义。
#[tauri::command]
pub async fn remote_study_close(app: tauri::AppHandle, study_dir: String) -> Result<Value, String> {
    absolute_study(&study_dir)?;
    let server = study_server(&remote_config(app)?, &study_dir)?;
    let mut status = ssh_cli_json(
        vec!["remote-study-status".into(), study_dir.clone()],
        None,
        Some(&server),
    )
    .await?;
    let recovery = attach_study_recovery(&mut status);
    if !recovery.permissions.close {
        return Err(close_blocked_message(&recovery));
    }
    if recovery.fetched || recovery.finished || recovery.permissions.resubmit {
        // 归档是不可逆的本地动作：只用刚读到、且 CLI 已核对过作业身份的 status 来确认
        // 本机记录指向同一个作业；不接受用本地记录里的旧 job 字段单独决定。
        verify_local_record_matches_status(&study_dir, &status)?;
        let archived = archive_study_record(&study_dir)?;
        return Ok(json!({
            "outcome": "closed",
            "job": recovery.job,
            "previous_state": status.get("status").cloned().unwrap_or(Value::Null),
            "archived": archived,
            "archived_by": "gui",
        }));
    }
    ssh_cli_json(
        vec!["remote-study-close".into(), study_dir],
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
