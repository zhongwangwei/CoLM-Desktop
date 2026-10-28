//! 应用托管的**本机 Xerj 节点**：探测二进制、选端口、启动、等就绪、读 key、停止。
//!
//! 为什么需要这一层：Xerj 是可选的本机文档检索后端（检索侧见
//! `crates/colm-agent/src/retrieval.rs`）。用户本机可能自己跑着一个 Xerj，那种情况
//! 设置里填 endpoint 即可（external 模式）；这里负责的是“应用自己起一个”：数据目录
//! 放在应用数据目录下，进程生命周期跟窗口走，窗口关掉就停，不留后台孤儿。
//!
//! 固定的 release 是 [`PINNED_XERJ_RELEASE`]。它**必须与
//! `crates/colm-agent/src/retrieval.rs::PINNED_XERJ_RELEASE` 手动保持一致**：检索侧
//! 的每一个 HTTP 请求形状（409 建索引、200+errors 的 bulk、404 删除……）都是按那个
//! release 实测的，管理端拉起别的版本而检索端仍按旧版行为假设跑，是“两个部件各自
//! 正确、拼起来错”的典型来源。版本不符时这里只提示、不阻断、绝不自动下载/替换/升级：
//! 自动升级会把“固定 release 的确定性”和用户的数据目录一起打破。
//!
//! 安全边界：
//! - key 只留在内存里，由 [`ManagedNode::api_key`] 交给调用方、由健康检查放进
//!   `Authorization` 头；不进 status/Debug/错误/日志。Xerj 自己的日志只打印
//!   `admin.key` 的路径（rc.100 实测），不打印内容。
//! - 只连 loopback；endpoint 由本模块自己拼 `http://127.0.0.1:<port>`，不接受用户
//!   输入的主机名。
//! - 只停自己 spawn 的进程组（`top_level_sidecar` 让 pgid == child.id()），不碰用户
//!   自己的 Xerj 节点、也不碰 PID 复用的进程。
//!
//! 平台：首版只在 macOS arm64 上对真实 rc.100 跑过验收；其它平台限编译通过，行为未实测。

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 唯一做过真实验收的 Xerj release。**必须与
/// `crates/colm-agent/src/retrieval.rs::PINNED_XERJ_RELEASE` 手动同步**：两处各自被
/// 不同的测试/验收守着（这边是真实节点验收，那边是检索行为实测），只改一处就会让
/// “管理的版本”和“检索端假设的版本”分叉。
pub const PINNED_XERJ_RELEASE: &str = "v1.0.0-rc.100";

/// 默认就绪超时。实测全新 data dir 冷启动（macOS arm64 / rc.100）约 1.4–2.1 秒：
/// 要建 14 个系统索引再开监听。20 秒是给慢磁盘或首次建索引留的余量，不是正常等待。
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// 用户显式指定二进制路径的环境变量（自动化与排障用；应用设置里有同等入口）。
pub(crate) const BINARY_ENV: &str = "COLM_XERJ_BINARY";

/// 端口扫描区间。9200 是 Xerj 默认的 ES-compat 端口，与检索侧的
/// `DEFAULT_ENDPOINT` 一致；9399 给“同时开多个节点”留出空间。
const PORT_SCAN_START: u16 = 9200;
const PORT_SCAN_END: u16 = 9399;
/// 一个 Xerj 节点一次占用 3 个连续端口：`--port P` 同时占用 P（ES-compat）、
/// P+1（native REST）、P+2（gRPC）。选端口必须按整组检查，否则会选中一个看起来空、
/// 实际装不下的 base。
const PORTS_PER_NODE: u16 = 3;

/// 就绪轮询间隔。100 ms 在 2 秒启动下最多二十次，对 CPU 无感，又能让启动等待更短。
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// 单次健康检查的时间预算。总预算仍由 `ready_timeout` 兜住：每次取剩余时间与它的小值。
const PROBE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(1);
/// `xerj --version` 是一个立刻退出的独立进程；3 秒是防它卡死的上限。
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// 停止时等进程组退出的上限。`terminate_process_tree` 内部已经做了一次 TERM→300ms→KILL，
/// 这里再等一轮是防脚本/信号处理器忽略 TERM 时留下孤儿。
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 健康检查响应读取上限。正常 `/_cluster/health` 只有几百字节（rc.100 实测 412 字节），
/// 读不完的响应说明对端不是我们要的节点，不能无穷读下去。
const MAX_HEALTH_BODY: usize = 64 * 1024;
/// 启动失败时只回读日志尾部：整份日志可能已经几 MB，错误文案要短、要能塞进 IPC。
const LOG_TAIL_BYTES: u64 = 4096;
const LOG_TAIL_LINES: usize = 5;
const LOG_TAIL_CHARS: usize = 1200;

/// 启动托管节点所需的配置。`data_dir` 必须是应用数据目录下的**应用自有**目录：
/// 不要指向用户自己节点的数据目录，否则会拿到别人的 `node.lock`、甚至写坏别人的索引。
#[derive(Debug, Clone)]
pub struct ManagedConfig {
    /// 用户显式路径（为空/`None` = 自动探测）。
    pub binary_path: Option<String>,
    /// 应用数据目录下的应用自有 Xerj 数据目录。
    pub data_dir: PathBuf,
    /// 用户偏好端口；`None` = 从 9200 起自动选。
    pub preferred_port: Option<u16>,
    /// 例如 resource dir、当前可执行文件目录 —— 打包后的应用把 `xerj` 放在那里。
    pub extra_binary_dirs: Vec<PathBuf>,
    /// 等待“admin.key 已生成 + /_cluster/health 可用”的总时长。
    pub ready_timeout: Duration,
}

/// 托管节点对外可见的状态。**没有 key 字段**：key 通过 [`ManagedNode::api_key`] 单独取。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ManagedStatus {
    pub running: bool,
    /// 含 scheme+host+port 的 base，例如 `http://127.0.0.1:9310`。
    pub endpoint: Option<String>,
    pub port: Option<u16>,
    pub binary: Option<String>,
    /// 例如 `v1.0.0-rc.100`；读不到 `xerj --version` 时为 `None`。
    pub version: Option<String>,
    /// `None` = 版本未知；`false` = 与固定版本不同（只警告，不阻断）。
    pub pinned_match: Option<bool>,
    pub data_dir: Option<String>,
    /// 启动失败/版本不符/停止失败的可读原因，**不含 key**。
    pub detail: Option<String>,
}

/// 子进程状态。`Child` 需要 `&mut` 才能 `try_wait`，而 `status()` 只有 `&self`，
/// 所以用 `Mutex` 而不是把 Child 暴露成可变字段：状态查询与停止都从同一把锁走。
///
/// 只保存 `Option<Child>`：已在 `try_wait`/`wait` 里收尸的进程直接从 `child` 里拿掉，
/// `None` 就是“本节点已不再持有活进程”，`stop()` 的幂等和 PID 复用防护都由它保证。
struct ProcessState {
    child: Option<Child>,
}

/// 一个正在运行的托管节点。`Drop` 会尽力停止自己启动的进程组，避免孤儿。
pub struct ManagedNode {
    process: Mutex<ProcessState>,
    endpoint: String,
    port: u16,
    binary: PathBuf,
    version: Option<String>,
    pinned_match: Option<bool>,
    data_dir: PathBuf,
    detail: Option<String>,
    key: String,
}

impl std::fmt::Debug for ManagedNode {
    /// 手写 Debug 而不是 derive：改天有人把 `ManagedNode` 塞进日志/panic 信息时，
    /// key 不能跟着出去。Debug 是那些路径最可能的入口。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedNode")
            .field("endpoint", &self.endpoint)
            .field("port", &self.port)
            .field("binary", &self.binary)
            .field("version", &self.version)
            .field("pinned_match", &self.pinned_match)
            .field("data_dir", &self.data_dir)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl ManagedNode {
    /// 同步阻塞：探测二进制、选端口、spawn（进程组）、等 admin.key 与
    /// `/_cluster/health` 就绪。超时/失败会终止**自己启动的**进程组并返回可读错误
    /// （含最后错误、退出码与日志尾部；绝不打印 key）。
    ///
    /// 允许换端口重试一次：`pick_base_port` 的可用性检查与真正 bind 之间有竞态
    /// （另一个进程可能刚好占了端口），一次重试比把竞态直接抛给用户划算。
    pub fn start(config: ManagedConfig) -> Result<ManagedNode, String> {
        if config.data_dir.as_os_str().is_empty() {
            return Err("Xerj 数据目录为空：请先指定应用数据目录下的 Xerj 专用目录".to_owned());
        }
        std::fs::create_dir_all(&config.data_dir).map_err(|error| {
            format!(
                "无法创建 Xerj 数据目录 {}：{error}",
                config.data_dir.display()
            )
        })?;
        // 绝对路径：子进程的工作目录将来可能变（打包后从 Finder 启动时 cwd 是 `/`），
        // 相对路径传 `--data-dir` 会指到别处。
        let data_dir =
            std::fs::canonicalize(&config.data_dir).unwrap_or_else(|_| config.data_dir.clone());
        if !data_dir.is_dir() {
            return Err(format!("Xerj 数据目录不是目录：{}", data_dir.display()));
        }
        let binary = resolve_binary(config.binary_path.as_deref(), &config.extra_binary_dirs)?;
        // 版本探测在 spawn 之前：版本不符要能出现在本次 status 的 detail 里，
        // 而不是等节点起来后才补一条没人看的事件。读不到版本（None）不算错，只标未知。
        let version = read_binary_version(&binary);
        let pinned_match = version
            .as_deref()
            .map(|version| version == PINNED_XERJ_RELEASE);
        let detail = match (pinned_match, version.as_deref()) {
            (Some(false), Some(version)) => Some(format!(
                "Xerj 版本 {version} 与固定版本 {PINNED_XERJ_RELEASE} 不一致：需人工替换为固定版本并重跑验收；当前不阻断启动"
            )),
            _ => None,
        };
        // 配置里缺字段时 serde 会给 Duration::ZERO；把它当默认值，而不是“立刻超时”，
        // 否则旧配置文件会让节点永远起不来。
        let ready_timeout = if config.ready_timeout.is_zero() {
            DEFAULT_READY_TIMEOUT
        } else {
            config.ready_timeout
        };
        let mut last_error = String::new();
        let mut retried_from = None;
        for attempt in 0..2u8 {
            let port = if attempt == 0 {
                pick_base_port(config.preferred_port)?
            } else {
                pick_base_port_avoiding(None, retried_from)?
            };
            retried_from = Some(port);
            match spawn_and_wait(&binary, &data_dir, port, ready_timeout) {
                Ok((child, key)) => {
                    return Ok(ManagedNode {
                        process: Mutex::new(ProcessState { child: Some(child) }),
                        endpoint: format!("http://127.0.0.1:{port}"),
                        port,
                        binary,
                        version,
                        pinned_match,
                        data_dir,
                        detail,
                        key,
                    });
                }
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    pub fn endpoint(&self) -> String {
        self.endpoint.clone()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn version(&self) -> Option<String> {
        self.version.clone()
    }

    pub fn pinned_match(&self) -> Option<bool> {
        self.pinned_match
    }

    /// 应用自己数据目录里的 `admin.key`（trim 后非空；Xerj 生成 64 个 hex 字符）。
    /// 启动就绪时读一次并缓存：Xerj 不会在运行中轮换 key，而已经交给 agent 的 key
    /// 必须和当前节点实际接受的一致，重读文件反而可能在文件被删时给出空值。
    pub fn api_key(&self) -> String {
        self.key.clone()
    }

    pub fn status(&self) -> ManagedStatus {
        let mut state = lock_process(&self.process);
        let mut running = false;
        // 先把 Child 拿出来再 try_wait：try_wait 需要 &mut，而观察到退出后要清空
        // `child`（它已经被收尸），借用检查不允许同时借用同一个字段。
        if let Some(mut child) = state.child.take() {
            match child.try_wait() {
                Ok(None) => {
                    state.child = Some(child);
                    running = true;
                }
                Ok(Some(_)) => {}
                // 读不到进程状态时按“还在跑”处理：停不掉的节点比过早显示已停更危险。
                Err(_) => {
                    state.child = Some(child);
                    running = true;
                }
            }
        }
        ManagedStatus {
            running,
            endpoint: Some(self.endpoint()),
            port: Some(self.port()),
            binary: Some(self.binary.display().to_string()),
            version: self.version(),
            pinned_match: self.pinned_match(),
            data_dir: Some(self.data_dir.display().to_string()),
            detail: self.detail.clone(),
        }
    }

    /// 只终止自己启动的进程组：`terminate_process_tree(child.id())`（TERM → 最多等 5 秒
    /// → 补 KILL）。幂等：已经停过/已自然退出的节点再调一次返回 `Ok(())`。
    /// 不碰其它 PID、不碰用户自己的 Xerj 节点 —— 要发信号的 pid 只来自本节点持有的 Child。
    pub fn stop(&mut self) -> Result<(), String> {
        let result = {
            let mut state = lock_process(&self.process);
            stop_locked(&mut state)
        };
        if let Err(error) = &result {
            let reason = format!("停止 Xerj 进程组失败：{error}");
            self.detail = Some(match self.detail.take() {
                Some(previous) => format!("{previous}；{reason}"),
                None => reason,
            });
        }
        result
    }
}

impl Drop for ManagedNode {
    fn drop(&mut self) {
        // 尽力而为：Drop 里不能 panic，也不能因为停止失败让上层退出流程中断。
        let _ = self.stop();
    }
}

fn lock_process(process: &Mutex<ProcessState>) -> MutexGuard<'_, ProcessState> {
    process
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn stop_locked(state: &mut ProcessState) -> Result<(), String> {
    let Some(mut child) = state.child.take() else {
        return Ok(());
    };
    match child.try_wait() {
        // 已经自然退出：Child 已被收尸，直接丢句柄就是“不再持有活进程”。
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(error) => {
            state.child = Some(child);
            return Err(format!("无法读取 Xerj 进程状态：{error}"));
        }
    }
    let pid = child.id();
    let terminate_error = crate::sidecar::terminate_process_tree(pid).err();
    let mut exit = wait_for_exit(&mut child, STOP_TIMEOUT);
    if exit.is_none() {
        // terminate_process_tree 已经 KILL 过进程组；这里再直接 kill 一次持有的 Child 是
        // 最后手段（比如进程组被杀、但 leader 因某种原因还在）。仍然只碰自己的子进程。
        let _ = child.kill();
        exit = wait_for_exit(&mut child, Duration::from_secs(1));
    }
    match exit {
        Some(_) => Ok(()),
        None => {
            state.child = Some(child);
            let trailing = terminate_error
                .map(|error| format!("（{error}）"))
                .unwrap_or_default();
            Err(format!(
                "进程组 {pid} 在 {} 秒内没有退出{trailing}",
                STOP_TIMEOUT.as_secs()
            ))
        }
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(STOP_POLL_INTERVAL);
    }
}

/// 按“显式路径 → `COLM_XERJ_BINARY` → extra_dirs 下的 `xerj`/`xerj.exe` → PATH”找二进制。
///
/// 全部找不到时返回可读错误（说明该放哪里/怎么设），**不自动下载**：首版只支持用户
/// 自己准备固定 release 的二进制，自动下载会把“固定版本”变成一句空话。
pub fn resolve_binary(explicit: Option<&str>, extra_dirs: &[PathBuf]) -> Result<PathBuf, String> {
    resolve_binary_from(
        explicit,
        std::env::var_os(BINARY_ENV).as_deref(),
        extra_dirs,
        std::env::var_os("PATH").as_deref(),
    )
}

/// `resolve_binary` 的可注入版本：env/PATH 显式传进来，测试不用改进程环境。
fn resolve_binary_from(
    explicit: Option<&str>,
    env_binary: Option<&OsStr>,
    extra_dirs: &[PathBuf],
    path_value: Option<&OsStr>,
) -> Result<PathBuf, String> {
    // 设置里空串表示“自动探测”，不能当成“当前目录下的空文件名”。
    if let Some(raw) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return explicit_binary(PathBuf::from(raw));
    }
    if let Some(raw) = env_binary.filter(|value| !value.is_empty()) {
        return explicit_binary(PathBuf::from(raw))
            .map_err(|error| format!("{BINARY_ENV} {error}"));
    }
    for dir in extra_dirs {
        for name in binary_names() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if let Some(path_value) = path_value {
        for dir in std::env::split_paths(path_value) {
            // POSIX 里 PATH 的空项等价于“当前目录”。搜索可执行文件时跳过它，
            // 避免从启动目录（打包后可能是用户主目录）误捡一个 `xerj` 执行。
            if dir.as_os_str().is_empty() {
                continue;
            }
            for name in binary_names() {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    Err(format!(
        "没有找到 Xerj 二进制：{BINARY_ENV} 未设置，应用资源目录与 PATH 里也没有 xerj。请把固定 release 的 xerj 放到应用资源目录，或在设置里指定绝对路径；本应用不会自动下载或安装 Xerj"
    ))
}

fn binary_names() -> [&'static str; 2] {
    if cfg!(windows) {
        ["xerj.exe", "xerj"]
    } else {
        ["xerj", "xerj.exe"]
    }
}

fn explicit_binary(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path);
    }
    if path.exists() {
        return Err(format!("指定的 Xerj 路径不是文件：{}", path.display()));
    }
    Err(format!("指定的 Xerj 二进制不存在：{}", path.display()))
}

/// 选一个“连续 3 个端口都空”的 base。顺序：`preferred` 满足就用 → 9200–9399 扫描
/// → 绑 `127.0.0.1:0` 拿系统临时端口再确认 +1/+2。逐个 bind 后立即 drop 有微小竞态，
/// 由 `start` 的一次换端口重试兜底。
pub fn pick_base_port(preferred: Option<u16>) -> Result<u16, String> {
    pick_base_port_avoiding(preferred, None)
}

fn pick_base_port_avoiding(preferred: Option<u16>, avoid: Option<u16>) -> Result<u16, String> {
    // 0 不是合法的监听端口（bind 到 0 是“让 OS 选”），不能当成用户偏好的 base。
    if let Some(preferred) = preferred.filter(|port| *port != 0) {
        if Some(preferred) != avoid && three_ports_available(preferred) {
            return Ok(preferred);
        }
    }
    for base in PORT_SCAN_START..=PORT_SCAN_END {
        if Some(base) != avoid && three_ports_available(base) {
            return Ok(base);
        }
    }
    // 固定区间被占满（比如同时开了很多节点）时退到系统临时端口。
    for _ in 0..8 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("无法向系统申请临时端口：{error}"))?;
        let base = listener
            .local_addr()
            .map_err(|error| format!("无法读取系统临时端口：{error}"))?
            .port();
        drop(listener);
        if Some(base) != avoid && three_ports_available(base) {
            return Ok(base);
        }
    }
    Err(format!(
        "找不到连续 {PORTS_PER_NODE} 个可用本机端口（{PORT_SCAN_START}–{PORT_SCAN_END} 与系统临时端口都被占用）"
    ))
}

fn three_ports_available(base: u16) -> bool {
    // 0 是“让 OS 选”，不是可用的监听端口；65534/65535 没有 +1/+2。
    if base == 0 || base > u16::MAX - (PORTS_PER_NODE - 1) {
        return false;
    }
    let mut listeners = Vec::with_capacity(PORTS_PER_NODE as usize);
    for offset in 0..PORTS_PER_NODE {
        match TcpListener::bind((Ipv4Addr::LOCALHOST, base + offset)) {
            // 已 bind 的 listener 在这个函数返回时一并 drop，把端口还回去。
            Ok(listener) => listeners.push(listener),
            Err(_) => return false,
        }
    }
    drop(listeners);
    true
}

/// 最小 HTTP/1.1 健康检查：`GET {endpoint}/_cluster/health`。
///
/// 只接受 HTTP 200 且 JSON 里 `status` 为 `green`/`yellow`；`red` 视为未就绪。
/// key 只进 `Authorization: ApiKey` 头，错误文本里绝不出现。
pub fn probe_health(endpoint: &str, api_key: &str, timeout: Duration) -> Result<(), String> {
    if timeout.is_zero() {
        return Err("健康检查超时：timeout 为 0".to_owned());
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "健康检查超时：timeout 过大".to_owned())?;
    let (host, address) = parse_http_endpoint(endpoint)?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| format!("无法连接 Xerj 节点 {endpoint}：{error}"))?;
    let _ = stream.set_nodelay(true);
    write_health_request(&mut stream, &host, api_key, deadline)?;
    let (status, body) = read_http_response(&mut stream, deadline)?;
    judge_health_response(status, &body)
}

/// 只接受 `http://` + 显式端口的本机地址，且不能带 path/query/fragment（健康检查路径
/// 由本模块固定拼）。为什么不宽松：endpoint 将来若被设置文件污染，至少不会把带 key 的
/// 请求发到外网主机。
fn parse_http_endpoint(endpoint: &str) -> Result<(String, SocketAddr), String> {
    let endpoint = endpoint.trim();
    let rest = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| "Xerj endpoint 必须以 http:// 开头（本机节点不走 TLS）".to_owned())?;
    // 允许一个结尾斜杠，其余路径一律拒绝：base 才能安全拼接 /_cluster/health。
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() {
        return Err("Xerj endpoint 缺少主机与端口".to_owned());
    }
    if rest
        .chars()
        .any(|character| matches!(character, '/' | '?' | '#' | '@'))
    {
        return Err(
            "Xerj endpoint 只能是 http://主机:端口 的 base，不能带路径/查询/凭据".to_owned(),
        );
    }
    if let Some(rest) = rest.strip_prefix('[') {
        // [::1]:port
        let (host, port) = rest
            .split_once("]:")
            .ok_or_else(|| "Xerj endpoint 的 IPv6 地址缺少 `]:端口`".to_owned())?;
        let address = host
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| format!("Xerj endpoint 的主机无法识别：{host}"))?;
        if !address.is_loopback() {
            return Err(format!("Xerj endpoint 必须指向本机回环地址：{host}"));
        }
        let port = parse_port(port)?;
        return Ok((
            format!("[{host}]:{port}"),
            SocketAddr::from((address, port)),
        ));
    }
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| "Xerj endpoint 缺少端口".to_owned())?;
    let port = parse_port(port)?;
    let address = if host.eq_ignore_ascii_case("localhost") {
        Ipv4Addr::LOCALHOST
    } else {
        host.parse::<Ipv4Addr>()
            .map_err(|_| format!("Xerj endpoint 的主机必须是本机地址：{host}"))?
    };
    if !address.is_loopback() {
        return Err(format!("Xerj endpoint 必须指向本机回环地址：{host}"));
    }
    Ok((format!("{host}:{port}"), SocketAddr::from((address, port))))
}

fn parse_port(text: &str) -> Result<u16, String> {
    text.parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("Xerj endpoint 的端口无效：{text}"))
}

fn write_health_request(
    stream: &mut TcpStream,
    host: &str,
    api_key: &str,
    deadline: Instant,
) -> Result<(), String> {
    let mut request = format!(
        "GET /_cluster/health HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if !api_key.is_empty() {
        request.push_str("Authorization: ApiKey ");
        request.push_str(api_key);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    set_stream_deadline(stream, deadline)?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("向 Xerj 节点 {host} 发送健康检查请求失败：{error}"))
}

fn set_stream_deadline(stream: &TcpStream, deadline: Instant) -> Result<(), String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("健康检查超时".to_owned());
    }
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| format!("设置健康检查超时失败：{error}"))?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| format!("设置健康检查超时失败：{error}"))?;
    Ok(())
}

/// 读完整 HTTP 响应（受总 deadline 约束，最多 [`MAX_HEALTH_BODY`] 字节）。
/// 有 `Content-Length` 时按它的长度读完即停，不依赖对端关连接 —— Xerj 的响应带
/// `Content-Length`，但“对端一定守 Connection: close”不值得赌。
fn read_http_response(stream: &mut TcpStream, deadline: Instant) -> Result<(u16, Vec<u8>), String> {
    let mut data: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut expected_total: Option<usize> = None;
    loop {
        if let Some(total) = expected_total {
            if data.len() >= total {
                break;
            }
        }
        set_stream_deadline(stream, deadline)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                data.extend_from_slice(&chunk[..read]);
                if data.len() > MAX_HEALTH_BODY {
                    return Err(response_too_large());
                }
                if expected_total.is_none() {
                    if let Some(header_end) = find_header_end(&data) {
                        let content_length = parse_content_length(&data[..header_end]);
                        if let Some(length) = content_length {
                            let total = header_end + 4 + length;
                            if total > MAX_HEALTH_BODY {
                                return Err(response_too_large());
                            }
                            expected_total = Some(total);
                        }
                    }
                }
            }
            Err(error) if is_io_timeout(&error) => return Err("健康检查超时".to_owned()),
            Err(error) => return Err(format!("读取健康检查响应失败：{error}")),
        }
    }
    let header_end =
        find_header_end(&data).ok_or_else(|| "健康检查响应缺少完整的 HTTP 头".to_owned())?;
    let head = std::str::from_utf8(&data[..header_end])
        .map_err(|_| "健康检查响应的 HTTP 头不是合法 UTF-8".to_owned())?;
    let status_line = head.lines().next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let protocol = parts.next().unwrap_or("");
    if !protocol.starts_with("HTTP/") {
        return Err(format!(
            "健康检查响应不是 HTTP：{}",
            truncate_chars(status_line, 80)
        ));
    }
    let status = parts
        .next()
        .and_then(|text| text.parse::<u16>().ok())
        .ok_or_else(|| {
            format!(
                "健康检查响应缺少状态码：{}",
                truncate_chars(status_line, 80)
            )
        })?;
    let body_start = header_end + 4;
    let body_end = expected_total.unwrap_or(data.len()).min(data.len());
    let body = if body_start <= body_end {
        data[body_start..body_end].to_vec()
    } else {
        Vec::new()
    };
    Ok((status, body))
}

fn response_too_large() -> String {
    format!("健康检查响应超过 {} KiB 上限", MAX_HEALTH_BODY / 1024)
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse::<usize>().ok()
        } else {
            None
        }
    })
}

fn is_io_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn judge_health_response(status: u16, body: &[u8]) -> Result<(), String> {
    if status != 200 {
        // 不把响应体带进错误：401 的响应体可能含服务端给出的提示，但更稳妥的做法是
        // 一律不带，避免将来 Xerj 改文案时把不该外露的内容带进 IPC/日志。
        return Err(format!("健康检查返回 HTTP {status}"));
    }
    let text =
        std::str::from_utf8(body).map_err(|_| "健康检查响应不是合法的 UTF-8 JSON".to_owned())?;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "健康检查响应不是合法 JSON".to_owned())?;
    match value.get("status").and_then(serde_json::Value::as_str) {
        Some("green" | "yellow") => Ok(()),
        Some("red") => Err("Xerj 集群状态为 red（未就绪）".to_owned()),
        Some(other) => Err(format!(
            "Xerj 集群状态为 {}（未就绪）",
            truncate_chars(other, 40)
        )),
        None => Err("健康检查响应缺少 status 字段（未就绪）".to_owned()),
    }
}

/// 从 `xerj --version` 输出里提取形如 `vX.Y.Z-...` 的版本串；不匹配返回 `None`。
///
/// 实测 rc.100 输出：`xerj v1.0.0-rc.100`（一行）。不引 regex：这里只需按空白切词，
/// 找一个以 `v` 开头、核心是三段数字、后面允许 `-`/`+`/字母数字/点的 token。
pub fn parse_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(version_token)
}

fn version_token(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    for (start, byte) in bytes.iter().enumerate() {
        if *byte != b'v' {
            continue;
        }
        let Some(next) = bytes.get(start + 1) else {
            continue;
        };
        if !next.is_ascii_digit() {
            continue;
        }
        let mut end = start + 1;
        while let Some(byte) = bytes.get(end) {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+') {
                end += 1;
            } else {
                break;
            }
        }
        let candidate = token[start..end].trim_end_matches(['.', '-', '+']);
        if is_semver_like(candidate) {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn is_semver_like(candidate: &str) -> bool {
    let core = candidate.trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// 用 `binary --version` 读版本：spawn 一个独立进程，3 秒内没结束就杀掉它本身
/// （版本读不到只是警告，不能把启动拖住）。
fn read_binary_version(binary: &Path) -> Option<String> {
    let mut command = Command::new(binary);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = colm_kernel::run::top_level_sidecar(&mut command)
        .spawn()
        .ok()?;
    let pid = child.id();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let output = match receiver.recv_timeout(VERSION_TIMEOUT) {
        Ok(Ok(output)) => Some(output),
        _ => {
            let _ = crate::sidecar::terminate_process_tree(pid);
            // 给读取线程一点时间收尾；拿不到就放弃（版本未知）。
            receiver
                .recv_timeout(Duration::from_secs(1))
                .ok()
                .and_then(|result| result.ok())
        }
    }?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    parse_version(&text)
}

enum ReadyOutcome {
    Ready(String),
    Exited(ExitStatus),
    Timeout { key_seen: bool, last_error: String },
    ChildError(String),
}

/// 等“admin.key 已生成 + 健康检查通过”。两者都成功才返回 key。
///
/// `try_wait` 由调用方注入（start 传真实 Child，测试传常量）：每轮先检查进程是否提前
/// 退出，再读 key、再做健康检查 —— 进程挂了就不再空等满 timeout。
fn wait_until_ready(
    data_dir: &Path,
    endpoint: &str,
    timeout: Duration,
    mut try_wait: impl FnMut() -> std::io::Result<Option<ExitStatus>>,
) -> ReadyOutcome {
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let mut key: Option<String> = None;
    let mut last_error = "尚未来得及做健康检查".to_owned();
    loop {
        match try_wait() {
            Ok(Some(status)) => return ReadyOutcome::Exited(status),
            Ok(None) => {}
            Err(error) => return ReadyOutcome::ChildError(error.to_string()),
        }
        if key.is_none() {
            key = read_admin_key(data_dir);
        }
        if let Some(current) = key.as_deref() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let probe_timeout = std::cmp::min(remaining, PROBE_ATTEMPT_TIMEOUT);
            match probe_health(endpoint, current, probe_timeout) {
                Ok(()) => return ReadyOutcome::Ready(current.to_owned()),
                Err(error) => last_error = error,
            }
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep(std::cmp::min(
            deadline.saturating_duration_since(now),
            POLL_INTERVAL,
        ));
    }
    ReadyOutcome::Timeout {
        key_seen: key.is_some(),
        last_error,
    }
}

fn read_admin_key(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(data_dir.join("admin.key")).ok()?;
    let key = text.trim();
    if key.is_empty() {
        None
    } else {
        Some(key.to_owned())
    }
}

fn spawn_and_wait(
    binary: &Path,
    data_dir: &Path,
    port: u16,
    ready_timeout: Duration,
) -> Result<(Child, String), String> {
    let log_path = data_dir.join("xerj.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("无法写入 Xerj 日志 {}：{error}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("无法写入 Xerj 日志 {}：{error}", log_path.display()))?;
    let mut command = Command::new(binary);
    command
        .arg("--port")
        .arg(port.to_string())
        .arg("--bind")
        .arg("127.0.0.1")
        // 显式 lexical：默认 embed-mode 在将来版本可能是 neural，那会下载 ~90 MB 模型并
        // 需要网络，与“固定 release、离线可用、行为确定”相冲突。
        .arg("--embed-mode")
        .arg("lexical")
        .arg("--disable-feedback")
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let mut child = colm_kernel::run::top_level_sidecar(&mut command)
        .spawn()
        .map_err(|error| format!("无法启动 Xerj（{}）：{error}", binary.display()))?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let outcome = wait_until_ready(data_dir, &endpoint, ready_timeout, || child.try_wait());
    match outcome {
        ReadyOutcome::Ready(key) => Ok((child, key)),
        failure => {
            let pid = child.id();
            // 已经失败的进程不允许留在后台：按自己的进程组停掉，并把 Child 收回（reap），
            // 否则这里返回后它会变成孤儿、还占着端口。
            let terminate_error = crate::sidecar::terminate_process_tree(pid).err();
            let exit_status = wait_for_exit(&mut child, STOP_TIMEOUT);
            let log_tail = read_log_tail(&log_path);
            Err(build_start_failure(
                &failure,
                exit_status,
                terminate_error,
                &log_tail,
                ready_timeout,
                data_dir,
            ))
        }
    }
}

fn build_start_failure(
    failure: &ReadyOutcome,
    exit_status: Option<ExitStatus>,
    terminate_error: Option<String>,
    log_tail: &str,
    ready_timeout: Duration,
    data_dir: &Path,
) -> String {
    let reason = match failure {
        ReadyOutcome::Exited(status) => format!("Xerj 进程在就绪前退出（{status}）"),
        ReadyOutcome::Timeout {
            key_seen,
            last_error,
        } => {
            if *key_seen {
                format!(
                    "等待 Xerj 就绪超时（{ready_timeout:?}）：admin.key 已生成，但健康检查未通过：{last_error}"
                )
            } else {
                format!(
                    "等待 Xerj 就绪超时（{ready_timeout:?}）：{} 里没有读到 admin.key",
                    data_dir.display()
                )
            }
        }
        ReadyOutcome::ChildError(error) => format!("无法检查 Xerj 进程状态：{error}"),
        ReadyOutcome::Ready(_) => "Xerj 已就绪却走入失败路径（内部错误）".to_owned(),
    };
    let mut message = format!("Xerj 启动失败：{reason}");
    if let Some(status) = exit_status {
        message.push_str(&format!("；退出状态：{status}"));
    }
    if let Some(error) = terminate_error {
        message.push_str(&format!("；终止进程组失败：{error}"));
    }
    if !log_tail.is_empty() {
        message.push_str(&format!("；日志尾部：{log_tail}"));
    }
    message
}

/// 只回读日志尾部（最多 [`LOG_TAIL_BYTES`] 字节、最后几行）：错误文案要短。
/// Xerj 日志实测不含 key（只打印 admin.key 路径），不会把 secret 带进错误。
fn read_log_tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let Ok(metadata) = file.metadata() else {
        return String::new();
    };
    if metadata.len() > LOG_TAIL_BYTES {
        let _ = file.seek(SeekFrom::Start(metadata.len() - LOG_TAIL_BYTES));
    }
    let mut buffer = Vec::new();
    if file.take(LOG_TAIL_BYTES).read_to_end(&mut buffer).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&buffer);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(LOG_TAIL_LINES);
    truncate_chars(&lines[start..].join(" / "), LOG_TAIL_CHARS)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
#[path = "xerj_manager_tests.rs"]
mod xerj_manager_tests;
