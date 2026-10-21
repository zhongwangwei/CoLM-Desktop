//! 起 `colm-cli` 子进程，把它的输出**降速之后**再交给前端。
//!
//! 为什么必须降速：实测 CN-Cng 冬季窗口（528 个模型步）的 `colm.log` 有
//! 39215 行 / 3.3 MB，其中 **33357 行（85%）是 `Check vector data`**，
//! 也就是 RangeCheck 的逐变量播报。外推到完整两年（35088 步）约
//! **260 万行 / 220 MB**，而进度行是每模型步一行、约 110 行/秒。
//! 逐行发给 webview 会把它打死。
//!
//! 于是三条处置：
//! 1. RangeCheck 行**直接丢弃** —— 它只在越界时才有信息，而越界会被
//!    `colm-kernel` 的失败标记抓住（`with NAN` / `Out of Range!`）；
//! 2. 进度行解析成步数与日期，**节流到最多每 100 毫秒一次**；
//! 3. 其余行进环形缓冲区，并**按批发送**，同样每 100 毫秒一批。
//!
//! 第 3 条为什么是批量而不是逐行：丢掉 RangeCheck 之后仍有约 10 行/步
//! （`Checking forcing` / `Time elapsed` / `VSF scheme this step` …），
//! 完整两年就是 35 万行、约 1180 事件/秒 —— 照样打死 webview。
//! 而**按前缀去列举「哪些是逐步碎语」是脆的**：CoLM 把 automatically 拼成
//! automaticlly 这件事已经教过一次，上游随时会改措辞。批量节流不判断任何一行
//! 的价值，只保证事件率有上界。
//!
//! 逐行抽取必须在**独立线程**里做 —— 管道满了之后不读就会死锁。
//!
//! **`RangeCheck`（连带 `CoLMDEBUG`）现在是运行时开关**
//! （`DEF_USE_RangeCheck` / `DEF_USE_CoLMDEBUG`，`case.nml` 里设，
//! 默认 `.false.`），不再是编译期宏。上面「39215 行 / 33357 行」是
//! 开着调试时的实测数——默认关闭之后这条子栏跑出来的 `colm.log`
//! 基本不会再有 `Check vector data` 那一路。但这段丢弃逻辑仍然要留着：
//! 用户随时可能在 `case.nml` 里把 `DEF_USE_RangeCheck` 打开去调试，
//! 这时候同一个内核照样会吐出那 85%，丢弃与节流两条路都还用得上。

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager};

/// 流式运行保留的日志行上限。这里是每个 reader 线程自己的短暂缓冲，
/// 不再暴露全局共享 RunLog，避免并发算例互相清空。
const LOG_CAPACITY: usize = 4_000;

/// 事件最快多久发一次。进度与日志各自受这个上界约束，于是无论子进程
/// 打得多快，webview 每秒最多收到约 20 个事件。
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default, Clone)]
pub struct RunProcesses {
    inner: Arc<Mutex<RunProcessState>>,
}

/// Creation has its own registry so canceling runs never cancels a draft being created.
#[derive(Default, Clone)]
pub struct CaseCreations(RunProcesses);

const CASE_CREATION_TIMEOUT: Duration = Duration::from_secs(120);

fn creation_key(dir: &str) -> Result<String, String> {
    let mut ancestor = std::path::absolute(dir).map_err(|e| e.to_string())?;
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut path) => {
                for tail in missing.into_iter().rev() {
                    path.push(tail);
                }
                return Ok(path.to_string_lossy().into_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| format!("cannot resolve creation directory: {dir}"))?
                        .to_os_string(),
                );
                if !ancestor.pop() {
                    return Err(error.to_string());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[tauri::command]
pub async fn case_creation_cancel(
    creations: tauri::State<'_, CaseCreations>,
    dir: String,
) -> Result<usize, String> {
    let processes = creations.0.clone();
    let key = creation_key(&dir)?;
    tauri::async_runtime::spawn_blocking(move || processes.cancel(Some(vec![key])))
        .await
        .map_err(|e| e.to_string())?
}

/// Cancels the creations writing these local directories (blocking).
pub(crate) fn cancel_creations(
    creations: &CaseCreations,
    dirs: &[String],
) -> Result<usize, String> {
    let keys = dirs
        .iter()
        .map(|dir| creation_key(dir))
        .collect::<Result<Vec<_>, _>>()?;
    creations.0.cancel(Some(keys))
}

impl CaseCreations {
    pub(crate) fn cancel_on_shutdown(&self) -> Result<usize, String> {
        self.0.cancel(None)
    }
    /// Hold the creation registry so no case creation starts while `action` runs.
    pub(crate) fn with_idle_because<T>(
        &self,
        busy: &str,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.0.with_idle_because(busy, action)
    }
}

#[derive(Clone)]
pub(crate) struct CaseCreation(Arc<CreationState>);

struct CreationState {
    processes: RunProcesses,
    key: String,
    fresh: bool,
    deadline: Instant,
    complete: std::sync::atomic::AtomicBool,
    writers_reaped: std::sync::atomic::AtomicBool,
}

impl Drop for CreationState {
    fn drop(&mut self) {
        if self.fresh
            && !self.complete.load(Ordering::SeqCst)
            && self.writers_reaped.load(Ordering::SeqCst)
        {
            let _ = std::fs::remove_dir_all(&self.key);
        }
        if self.writers_reaped.load(Ordering::SeqCst) {
            let _ = self.processes.forget(&self.key);
        }
    }
}

impl CaseCreation {
    pub(crate) fn begin(
        creations: &CaseCreations,
        dir: &str,
        timeout: Duration,
    ) -> Result<Self, String> {
        let key = creation_key(dir)?;
        creations.0.prepare(std::slice::from_ref(&key))?;
        // The reserved key prevents another creator from claiming this output while
        // its existence is checked and until cleanup has finished.
        let fresh = !std::path::Path::new(&key).exists();
        Ok(Self(Arc::new(CreationState {
            fresh,
            processes: creations.0.clone(),
            key,
            deadline: Instant::now() + timeout,
            complete: std::sync::atomic::AtomicBool::new(false),
            writers_reaped: std::sync::atomic::AtomicBool::new(true),
        })))
    }
    pub(crate) fn validate_destination(&self, require_fresh: bool) -> Result<(), String> {
        let path = std::path::Path::new(&self.0.key);
        if require_fresh && !self.0.fresh {
            return Err(format!(
                "case directory already exists; choose a new case name: {}",
                path.display()
            ));
        }
        if path.join("case.nml").exists() {
            return Err(format!(
                "{} 里已经有一个算例，换一个算例名或根目录再建",
                path.display()
            ));
        }
        Ok(())
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        let state = self
            .0
            .processes
            .inner
            .lock()
            .map_err(|_| "creation registry poisoned")?;
        if state.cancelled.contains(&self.0.key) {
            return Err("算例创建已取消".into());
        }
        if Instant::now() >= self.0.deadline {
            return Err("算例创建超时；请检查系统文件访问授权和数据路径后重试".into());
        }
        Ok(())
    }
    fn complete(&self) -> Result<(), String> {
        self.check()?;
        let mut state = self
            .0
            .processes
            .inner
            .lock()
            .map_err(|_| "creation registry poisoned")?;
        if state.cancelled.contains(&self.0.key) {
            return Err("算例创建已取消".into());
        }
        state.pending.remove(&self.0.key);
        self.0.complete.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn failure(&self, error: String) -> String {
        if !self.0.writers_reaped.load(Ordering::SeqCst) {
            return format!("{error}; 创建进程未停止，禁止同目录重建，请查看进程/权限");
        }
        if self.0.fresh {
            if let Err(cleanup) = std::fs::remove_dir_all(&self.0.key) {
                if cleanup.kind() != std::io::ErrorKind::NotFound {
                    return format!("{error}; 无法清理未完成算例 {}: {cleanup}", self.0.key);
                }
            }
        }
        error
    }
    pub(crate) fn finish(&self, result: Result<String, String>) -> Result<String, String> {
        result
            .and_then(|output| self.complete().map(|_| output))
            .map_err(|error| self.failure(error))
    }
    pub(crate) async fn capture(&self, args: Vec<String>) -> Result<String, String> {
        self.check()?;
        let creation = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_with(&args, |args| {
                let mut command = sidecar_command(resolve_cli());
                command.args(args);
                creation.capture_command(&mut command)
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    fn remember_child(&self, pid: u32) -> Result<(), String> {
        let mut state = self
            .0
            .processes
            .inner
            .lock()
            .map_err(|_| "creation registry poisoned")?;
        if state.pids.contains_key(&self.0.key) {
            return Err("creation already has an active child".into());
        }
        // Cancellation can win before registration. Keep the PID visible so a failed
        // termination can be retried; check() will stop the newly registered child.
        state.pids.insert(self.0.key.clone(), pid);
        state.pending.remove(&self.0.key);
        Ok(())
    }
    fn capture_command(&self, command: &mut std::process::Command) -> Result<String, String> {
        use std::io::Read;
        self.check()?;
        let mut child = colm_kernel::run::top_level_sidecar(command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        self.0.writers_reaped.store(false, Ordering::SeqCst);
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let output = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        });
        let errors = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        });
        let outcome: Result<std::process::ExitStatus, String> = (|| {
            self.remember_child(child.id())?;
            loop {
                self.check()?;
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    return Ok(status);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        })();
        // Reap before returning or allowing the outer guard to clean the case directory.
        if outcome.is_err() && child.try_wait().map_err(|e| e.to_string())?.is_none() {
            terminate_process_tree(child.id())
                .map_err(|error| format!("无法终止创建进程 {}: {error}", child.id()))?;
        }
        let waited = child.wait().map_err(|e| e.to_string());
        // A helper must not outlive the creation command and keep its pipes open.
        #[cfg(unix)]
        if process_group_alive(child.id()) {
            terminate_process_tree(child.id())
                .map_err(|error| format!("无法终止创建子进程组 {}: {error}", child.id()))?;
        }
        waited?;
        self.0.writers_reaped.store(true, Ordering::SeqCst);
        {
            let mut state = self
                .0
                .processes
                .inner
                .lock()
                .map_err(|_| "creation registry poisoned")?;
            state.pids.remove(&self.0.key);
            state.pending.insert(self.0.key.clone());
        }
        let stdout = output
            .join()
            .map_err(|_| "creation stdout reader panicked")?
            .map_err(|e| e.to_string())?;
        let stderr = errors
            .join()
            .map_err(|_| "creation stderr reader panicked")?
            .map_err(|e| e.to_string())?;
        let status = outcome?;
        self.check()?;
        if !status.success() {
            return Err(String::from_utf8_lossy(&stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }
}

#[derive(Default)]
struct RunProcessState {
    pids: HashMap<String, u32>,
    pending: HashSet<String>,
    cancelled: HashSet<String>,
}

impl RunProcesses {
    /// Hold the launch registry while removing generated output, so no GUI run starts mid-delete.
    pub(crate) fn with_idle<T>(
        &self,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_idle_because("请等待正在运行或准备中的任务结束后再删除文件夹", action)
    }

    /// [`Self::with_idle`] with the refusal the caller's work needs.
    pub(crate) fn with_idle_because<T>(
        &self,
        busy: &str,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let state = self
            .inner
            .lock()
            .map_err(|_| "run process registry poisoned")?;
        if !state.pids.is_empty() || !state.pending.is_empty() {
            return Err(busy.into());
        }
        action()
    }

    /// Reject a second live process for the same key before starting a run.
    fn prepare(&self, keys: &[String]) -> Result<(), String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "run process registry poisoned")?;
        let mut unique = HashSet::with_capacity(keys.len());
        if let Some(key) = keys.iter().find(|key| !unique.insert(key.as_str())) {
            return Err(format!("duplicate case in one run request: {key}"));
        }
        if let Some(key) = keys
            .iter()
            .find(|key| state.pids.contains_key(*key) || state.pending.contains(*key))
        {
            return Err(format!("{key} is already running"));
        }
        state.pending.extend(keys.iter().cloned());
        Ok(())
    }

    /// Returns false when cancellation won the race between spawn and PID
    /// registration. The caller must terminate the just-spawned process tree.
    fn remember(&self, key: &str, pid: u32) -> Result<bool, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "run process registry poisoned")?;
        if state.cancelled.contains(key) {
            return Ok(false);
        }
        if state.pids.contains_key(key) {
            return Err(format!("{key} is already running"));
        }
        state.pending.remove(key);
        state.pids.insert(key.to_string(), pid);
        Ok(true)
    }

    fn take_cancelled(&self, key: &str) -> Result<bool, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "run process registry poisoned")?;
        let cancelled = state.cancelled.remove(key);
        if cancelled {
            state.pending.remove(key);
        }
        Ok(cancelled)
    }

    fn forget(&self, key: &str) -> Result<bool, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "run process registry poisoned")?;
        state.pids.remove(key);
        state.pending.remove(key);
        Ok(state.cancelled.remove(key))
    }

    fn running_pid(&self, key: &str) -> Result<Option<u32>, String> {
        self.inner
            .lock()
            .map_err(|_| "run process registry poisoned".to_string())
            .map(|state| state.pids.get(key).copied())
    }

    pub(crate) fn cancel_on_shutdown(&self) -> Result<usize, String> {
        let active = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "run process registry poisoned")?;
            state
                .pids
                .keys()
                .chain(state.pending.iter())
                .cloned()
                .collect::<HashSet<_>>()
        };
        let mut cancelled = 0;
        let mut failures = Vec::new();
        for key in active.iter().filter(|key| key.starts_with("study:")) {
            let study_dir = key.trim_start_matches("study:").to_string();
            let pid = match self.running_pid(key) {
                Ok(pid) => pid,
                Err(error) => {
                    failures.push(format!("{key}: {error}"));
                    continue;
                }
            };
            let _ = capture(&["study-cancel".into(), study_dir.clone()]);
            match self.cancel(Some(vec![key.clone()])) {
                Ok(count) => {
                    cancelled += count;
                    if let Some(pid) = pid {
                        if let Err(error) = capture(&[
                            "study-finalize-cancel".into(),
                            study_dir,
                            "--pid".into(),
                            pid.to_string(),
                        ]) {
                            failures.push(format!("{key}: {error}"));
                        }
                    }
                }
                Err(error) => failures.push(format!("{key}: {error}")),
            }
        }
        let ordinary = active
            .into_iter()
            .filter(|key| !key.starts_with("study:"))
            .collect::<Vec<_>>();
        match self.cancel(Some(ordinary)) {
            Ok(count) => cancelled += count,
            Err(error) => failures.push(error),
        }
        if failures.is_empty() {
            Ok(cancelled)
        } else {
            Err(failures.join("; "))
        }
    }

    pub(crate) fn cancel(&self, keys: Option<Vec<String>>) -> Result<usize, String> {
        let (requested, targets) = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "run process registry poisoned")?;
            let active = state
                .pids
                .keys()
                .chain(state.pending.iter())
                .cloned()
                .collect::<HashSet<_>>();
            let requested = keys
                .map(|keys| {
                    keys.into_iter()
                        .filter(|key| active.contains(key))
                        .collect::<HashSet<_>>()
                })
                .unwrap_or(active);
            let targets = requested
                .iter()
                .filter_map(|key| state.pids.get(key).copied().map(|pid| (key.clone(), pid)))
                .collect::<Vec<_>>();
            state.cancelled.extend(requested.iter().cloned());
            (requested, targets)
        };
        let mut failures = Vec::new();
        for (key, pid) in targets {
            if let Err(error) = terminate_process_tree(pid) {
                failures.push(format!("{key}: {error}"));
                if let Ok(mut state) = self.inner.lock() {
                    state.cancelled.remove(&key);
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("; "));
        }
        Ok(requested.len())
    }
}

fn sidecar_command(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    colm_kernel::run::no_console(&mut command);
    command.env("COLM_AUDIT_BY_PARENT", "1");
    command
}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) -> Result<(), String> {
    // `top_level_sidecar` made the process group id equal to `pid`; negative pid
    // addresses that group, so the active Fortran child dies with colm-cli.
    let group = format!("-{pid}");
    let term = sidecar_command("kill")
        .args(["-TERM", "--", group.as_str()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("cannot signal process group {pid}: {e}"))?;
    if !term.success() && process_group_alive(pid) {
        return Err(format!("cannot terminate process group {pid}: {term}"));
    }
    std::thread::sleep(Duration::from_millis(300));
    if process_group_alive(pid) {
        let kill = sidecar_command("kill")
            .args(["-KILL", "--", group.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("cannot kill process group {pid}: {e}"))?;
        std::thread::sleep(Duration::from_millis(50));
        if !kill.success() || process_group_alive(pid) {
            return Err(format!("cannot kill process group {pid}: {kill}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn terminate_process_tree(pid: u32) -> Result<(), String> {
    let status = sidecar_command("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("cannot run taskkill: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("taskkill failed for pid {pid}: {status}"))
    }
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(pid: u32) -> Result<(), String> {
    let status = sidecar_command("kill")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("cannot run kill: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("kill failed for pid {pid}: {status}"))
    }
}

#[cfg(unix)]
fn process_group_alive(pid: u32) -> bool {
    let Ok(output) = sidecar_command("ps")
        .args(["-A", "-o", "pgid=,stat="])
        .output()
    else {
        return false;
    };
    let group = pid.to_string();
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(group.as_str())
                && fields.next().is_some_and(|stat| !stat.starts_with('Z'))
        })
}

#[tauri::command]
pub fn cancel_runs(
    processes: tauri::State<'_, RunProcesses>,
    cases: Option<Vec<String>>,
) -> Result<usize, String> {
    processes.cancel(cases)
}

fn study_process_key(study_dir: &str) -> String {
    let path = PathBuf::from(study_dir);
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    format!("study:{}", path.display())
}

fn remember_process(
    processes: &RunProcesses,
    key: &str,
    child: &mut std::process::Child,
) -> Result<(), String> {
    let pid = child.id();
    match processes.remember(key, pid) {
        Ok(true) => Ok(()),
        Ok(false) => terminate_process_tree(pid).inspect_err(|_| {
            let _ = child.kill();
            let _ = processes.forget(key);
        }),
        Err(error) => {
            let _ = terminate_process_tree(pid);
            let _ = child.kill();
            Err(error)
        }
    }
}

fn wait_process(
    child: &mut std::process::Child,
    processes: &RunProcesses,
    key: &str,
) -> Result<(std::process::ExitStatus, bool), String> {
    match child.wait() {
        Ok(status) => Ok((status, processes.forget(key)?)),
        Err(error) => {
            let _ = terminate_process_tree(child.id());
            let _ = processes.forget(key);
            Err(error.to_string())
        }
    }
}

fn take_process_pipes(
    child: &mut std::process::Child,
    processes: &RunProcesses,
    key: &str,
) -> Result<(std::process::ChildStdout, std::process::ChildStderr), String> {
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = terminate_process_tree(child.id());
        let _ = processes.forget(key);
        return Err("sidecar did not expose its stdout/stderr pipes".into());
    };
    Ok((out, err))
}

/// 三个事件都带 `case`。**批量跑的时候这是唯一能分辨来源的东西** ——
/// 事件是全局广播的，90 个站点同时在跑时前端收到的是一锅粥。
///
/// 加字段而不是改事件名：现有三个 `listen` 是 `xtask check-gui`
/// 静态守着的接口，加字段不破坏它，改名会。
#[derive(Serialize, Clone)]
struct Progress {
    /// 一次前端运行请求的标识；同一算例重跑时用它丢弃上一轮迟到事件。
    run_id: String,
    /// 算例目录，唯一标识
    case: String,
    /// `mksrfdata` / `mkinidata` / `colm`。来自 `colm-cli` 自己打的阶段标记，
    /// **不是**从 CoLM 的输出措辞里猜的。
    stage: String,
    step: u64,
    /// 由 case.nml 的起止时刻、步长与预热轮数精确算出。
    total_steps: u64,
    /// CoLM 打印的 `YYYY-MM-DD-SSSSS`，原样传，不解释
    date: String,
    /// 预热轮次 `(第几轮, 共几轮)`。正常推进时是 `None`。
    ///
    /// CoLM 在 spin-up 期间用另一种 format（`CoLM.F90:747`），行尾多一段
    /// `Spinup (cycle 1 of 3)`。不单独认的话那段会整个留在 `date` 里，
    /// 界面既分不出正在预热，进度条还会跨轮次单调增长而看不出重来过。
    spinup: Option<(u32, u32)>,
}

/// 某一段开始或结束。批量运行时界面靠它画三段式进度。
#[derive(Serialize, Clone)]
struct StageMark {
    run_id: String,
    case: String,
    stage: String,
    /// `begin` / `ok` / `failed`
    state: String,
}

#[derive(Serialize, Clone)]
struct Lines {
    run_id: String,
    case: String,
    lines: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Done {
    run_id: String,
    case: String,
    /// `None` 表示三段完整流程；有值表示这次只请求了其中一段。
    requested_stage: Option<String>,
    code: i32,
    /// 子进程一共打了多少行
    total: usize,
    /// 其中多少行被当作 RangeCheck 丢掉了
    dropped: usize,
    /// 失败原因。**`colm-cli` 的错误走 stderr，而这里原来只读 stdout** ——
    /// 于是界面上只剩一句「失败（退出码 1）」，用户要自己去磁盘上翻
    /// `colm.log` 才知道是哪一段、为什么。实测踩过一次
    /// （`Forcing does not cover simulation period!`）。
    ///
    /// 成功时是 `None`：stderr 上偶尔有无关紧要的东西，
    /// 把它当成「原因」显示出来，比不显示更误导。
    reason: Option<String>,
    cancelled: bool,
}

pub(crate) use colm_kernel::progress::{parse_log, parse_progress, parse_stage};

/// 日志窗里一段开始时的标题行。
fn stage_heading(stage: &str) -> String {
    let what = match stage {
        "input-check" => "检查实际输入",
        "input-verify" => "核验运行输入",
        "mksrfdata" => "制作地表数据",
        "mkinidata" => "制作初始场",
        "colm" => "模拟",
        _ => "",
    };
    if what.is_empty() {
        format!("──── {stage} ────")
    } else {
        format!("──── {what}（{stage}）────")
    }
}

/// RangeCheck 的逐变量播报。**判据只有一份**，在 `colm-kernel` 里 ——
/// 那边已经在源头挡掉了这些行（不进日志也不进回调），所以这里通常一行都
/// 收不到。留着这一层是因为它们仍可能从别处来：用户手上更老的内核、
/// 或者将来某个不走 `run_stage_streaming` 的路径。
///
/// 两处各写一份判据的话，「哪一行算噪声」迟早会分叉，
/// 而分叉的表现是界面上冒出几百万行、或者反过来吞掉一条 Out of Range。
fn is_rangecheck_noise(line: &str) -> bool {
    colm_kernel::run::is_benign_rangecheck(line)
}

/// 找 `colm-cli`。发行版顺序：环境变量 → 自己旁边（打包进去的 sidecar）
/// → 仓库构建产物 → PATH。开发版把仓库产物放在自己旁边之前，避免
/// `gui/src-tauri/target/debug/colm-cli` 中残留的旧暂存版本遮住当前 CLI。
///
/// **第二条必须是 `current_exe()` 的同级目录，不是 `resource_dir()`。**
/// Tauri 把 `externalBin` 放在主二进制**旁边** —— macOS 的
/// `Contents/MacOS/colm-cli`，而 `resource_dir()` 指的是
/// `Contents/Resources/`，那里只有图标。实测打包出来的 `.app` 因此一路
/// 回落到 PATH，解析成裸名 `colm-cli`：开发机上仓库产物那条兜住了，
/// 装到别人机器上第一次点「运行」就是 `cannot start colm-cli`。
///
/// 不再需要 `AppHandle`：四条回落全部只看进程自身与环境，跟 Tauri 无关。
pub fn resolve_cli() -> PathBuf {
    if let Ok(p) = std::env::var("COLM_CLI") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return p;
        }
    }
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from));
    for p in cli_candidates(sibling) {
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe_name()) // 交给 PATH
}

fn cli_candidates(sibling_dir: Option<PathBuf>) -> Vec<PathBuf> {
    let workspace = ["../../target/debug", "../../target/release"].map(|rel| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(rel)
            .join(exe_name())
    });
    let sibling = sibling_dir.map(|dir| dir.join(exe_name()));
    // 仓库里的 debug 与 release 产物：先用最近编译的那个。固定先 debug 的话，release 刚编好，
    // 界面却还在用几小时前的 debug 版（实测助手因此缺了新加的工具）。
    let mut workspace = workspace.to_vec();
    workspace.sort_by_key(|path| {
        let has_stamp = path
            .parent()
            .map(|dir| dir.join("colm-build-source.json").is_file())
            .unwrap_or(false);
        (
            !has_stamp,
            std::cmp::Reverse(
                std::fs::metadata(path)
                    .and_then(|meta| meta.modified())
                    .ok(),
            ),
        )
    });
    if cfg!(debug_assertions) {
        workspace.into_iter().chain(sibling).collect()
    } else {
        sibling.into_iter().chain(workspace).collect()
    }
}

/// 一个可选的物理预设，交给前端做下拉框。
#[derive(Serialize)]
pub struct KernelEntry {
    /// `manifest.json` 里的 preset 名 —— 不是目录名。两者一致时也以清单为准，
    /// 因为跑起来认的是清单。
    pub preset: String,
    pub dir: String,
    /// 编译期宏组合。**这才是预设的身份** —— 目录叫什么无所谓，
    /// 「这个内核到底编进了什么物理」只有这一行说了算。
    pub generator_args: String,
    /// 预处理后实际生效的宏；前端匹配内核只能看它，不能相信请求参数。
    pub macros: Vec<String>,
    pub colm_git_sha: String,
    pub platform: String,
}

/// 列出能用的物理预设。
///
/// 发行版顺序：环境变量 → 随程序打包的那份 → 仓库构建产物。开发版把
/// 仓库放在资源目录前，因为 `target/debug/kernels` 可能残留上次暂存的
/// 不完整预设。打包资源仍让「用户什么都不用装」成立 —— Fortran 内核
/// 是构建产物，用户不该为了跑一个站点去装 gfortran 与 netcdf-fortran。
///
/// 这里用 `resource_dir()` 是**对的**，与 `resolve_cli` 不同：Tauri 把
/// `bundle.resources` 放进 `Contents/Resources/`，而把 `externalBin`
/// 放在主二进制旁边。两个目录不同，两处各按各的来。
#[tauri::command]
pub fn list_kernels(app: tauri::AppHandle) -> Vec<KernelEntry> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("COLM_KERNELS") {
        roots.push(PathBuf::from(p));
    }
    roots.extend(kernel_roots(app.path().resource_dir().ok()));

    let mut out: Vec<KernelEntry> = Vec::new();
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut found: Vec<KernelEntry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                // 走 `Kernel::open` 而不是自己读 json：它会连三个二进制的
                // sha256 一起校验。列出一个校验不过的内核，等于把
                // 「构建过但被换了」推迟到用户点下运行的那一刻才暴露。
                let k = colm_kernel::Kernel::open(&e.path()).ok()?;
                Some(KernelEntry {
                    preset: k.manifest.preset.clone(),
                    dir: k.dir.to_string_lossy().into_owned(),
                    generator_args: k.manifest.generator_args.clone(),
                    macros: k.manifest.macros.clone(),
                    colm_git_sha: k.manifest.colm_git_sha.clone(),
                    platform: k.manifest.platform.clone(),
                })
            })
            .collect();
        found.sort_by(|a, b| a.preset.cmp(&b.preset));
        if !found.is_empty() {
            // 记一行。跟 `resolve_cli` 同一个道理：仓库那条回落在开发机上
            // 永远命中，不把选中的那一层打出来，就分不清「装出来的程序自带
            // 内核」与「它其实在读源码树」。下拉框是空的时候，这也是唯一线索。
            eprintln!(
                "colm-desktop: {} preset(s) from {}",
                found.len(),
                root.display()
            );
            return found; // 先命中的那一层赢，不混着列
        }
        out.append(&mut found);
    }
    out
}

fn kernel_roots(resource_dir: Option<PathBuf>) -> Vec<PathBuf> {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../kernels");
    let resource = resource_dir.map(|dir| dir.join("kernels"));
    if cfg!(debug_assertions) {
        std::iter::once(repository).chain(resource).collect()
    } else {
        resource
            .into_iter()
            .chain(std::iter::once(repository))
            .collect()
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "colm-cli.exe"
    } else {
        "colm-cli"
    }
}

fn validate_run_stage(stage: Option<&str>) -> Result<(), String> {
    match stage {
        None | Some("mksrfdata" | "mkinidata" | "colm") => Ok(()),
        Some(other) => Err(format!(
            "未知运行阶段 {other:?}；只能选择 mksrfdata、mkinidata 或 colm"
        )),
    }
}

pub(crate) fn validate_run_id(run_id: &str) -> Result<(), String> {
    (!run_id.trim().is_empty())
        .then_some(())
        .ok_or_else(|| "run_id must not be empty".to_string())
}

/// GUI 的单算例与批量路径必须构造完全相同的 sidecar 参数。
fn run_args(
    case: &str,
    kernel: &str,
    force: bool,
    stage: Option<&str>,
    mpi_ranks: usize,
    engine: Option<&str>,
    model_version: Option<&str>,
) -> Result<Vec<String>, String> {
    validate_run_stage(stage)?;
    validate_engine(engine)?;
    validate_model_version(model_version)?;
    if mpi_ranks == 0 {
        return Err("mpiRanks must be at least 1".into());
    }
    let mut args = vec![
        "run".to_string(),
        case.to_string(),
        "--kernel".to_string(),
        kernel.to_string(),
        "--stream".to_string(),
        "1".to_string(),
        "--ranks".to_string(),
        mpi_ranks.to_string(),
    ];
    if let Some(stage) = stage {
        args.extend(["--stage".to_string(), stage.to_string()]);
    }
    if force {
        args.extend(["--force".to_string(), "1".to_string()]);
    }
    // 不给就不写：`colm-cli` 自己的默认值（rust）是唯一的来源，界面不再复述一份。
    if let Some(engine) = engine {
        args.extend(["--engine".to_string(), engine.to_string()]);
    }
    // 官方版不写：与没有这一项的旧调用完全相同。
    if let Some(version) = model_version.filter(|v| *v != "official") {
        args.extend(["--model-version".to_string(), version.to_string()]);
    }
    Ok(args)
}

/// Opt-in only: older IPC callers omit this field and skip bulk input hashing.
pub(crate) fn add_input_fingerprints(args: &mut Vec<String>, enabled: Option<bool>) {
    if enabled == Some(true) {
        args.extend(["--record-input-fingerprints".into(), "1".into()]);
    }
}

/// 模型版本：`official` 或一个工作区名（与工作区名同一套校验，拼进命令行前挡住）。
fn validate_model_version(version: Option<&str>) -> Result<(), String> {
    match version {
        None | Some("official") => Ok(()),
        Some(name) => crate::workspace::valid_name(name),
    }
}

/// `colm` 第三段的引擎。拼错的值在这里拦下，而不是让 sidecar 起来之后才报。
fn validate_engine(engine: Option<&str>) -> Result<(), String> {
    match engine {
        None | Some("rust") | Some("fortran") => Ok(()),
        Some(other) => Err(format!("未知模拟引擎 {other:?}；只能选择 rust 或 fortran")),
    }
}

/// 跑一个算例。返回子进程的退出码。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn run_case(
    app: tauri::AppHandle,
    processes: tauri::State<'_, RunProcesses>,
    run_id: String,
    case: String,
    kernel: String,
    force: bool,
    stage: Option<String>,
    mpi_ranks: usize,
    engine: Option<String>,
    threads: Option<usize>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
    record_input_fingerprints: Option<bool>,
) -> Result<i32, String> {
    validate_run_id(&run_id)?;
    let processes = processes.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_case_blocking(
            app,
            processes,
            run_id,
            case,
            kernel,
            force,
            stage,
            mpi_ranks,
            engine,
            threads,
            model_version,
            model_revision,
            preprocessors,
            record_input_fingerprints,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 在自己的线程上读 stderr。
///
/// **必须另起一条线程。** 与 stdout 轮流读的话，一边的管道写满时子进程
/// 会阻塞在那儿，而我们正卡在另一边等 —— 一个不报错的死锁。
/// `colm-kernel::run` 里是同样的处理。
///
/// 普通命令只归集错误；运行命令同时实时发给日志窗。
fn read_stderr_lines(reader: impl std::io::BufRead, mut on_line: impl FnMut(&str)) -> Vec<String> {
    let mut errors = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            on_line(&line);
            errors.push(line);
        }
    }
    errors
}

fn drain_stderr(err: std::process::ChildStderr) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || read_stderr_lines(BufReader::new(err), |_| {}))
}

/// stderr 里哪一段值得当作「失败原因」摆到状态栏上。
///
/// 取最后几行而不是第一行：anyhow 的链条是**由外向内**打的，
/// 最里面那层才是真正的原因（「强迫场覆盖不到」而不是「阶段 colm 失败」）。
fn failure_reason(err: &[String]) -> Option<String> {
    let tail: Vec<&str> = err.iter().rev().take(4).map(String::as_str).rev().collect();
    let s = tail.join(" / ");
    (!s.trim().is_empty()).then_some(s)
}

/// 读子进程的 stdout，边读边发事件，返回读完之后的统计与日志缓冲区。
///
/// **单算例与批量共用这一份。** 复制一份的话，「阶段标记要不要丢进日志窗」
/// 这类判断会在两处各写一次，然后慢慢分叉 —— 而两处的差异只会在
/// 「批量跑时日志不对」这种最难查的形式下暴露。
fn pump(
    app: &tauri::AppHandle,
    run_id: &str,
    case: &str,
    out: std::process::ChildStdout,
) -> std::thread::JoinHandle<(usize, usize, VecDeque<String>)> {
    let h = app.clone();
    let run_id = run_id.to_string();
    let case_id = case.to_string();
    let total_steps = crate::config::read_timing(vec![case_id.clone()])
        .map(|t| t.total_steps)
        .unwrap_or(0);
    std::thread::spawn(move || {
        let (events, received) = std::sync::mpsc::channel::<PumpEvent>();
        let emitter = {
            let (h, run_id, case_id) = (h.clone(), run_id.clone(), case_id.clone());
            std::thread::spawn(move || {
                batch_pump_events(received, EMIT_INTERVAL, |event| match event {
                    PumpEvent::Lines(lines) => {
                        let _ = h.emit(
                            "run://lines",
                            Lines {
                                run_id: run_id.clone(),
                                case: case_id.clone(),
                                lines,
                            },
                        );
                    }
                    PumpEvent::Stage(stage, state) => {
                        let _ = h.emit(
                            "run://stage",
                            StageMark {
                                run_id: run_id.clone(),
                                case: case_id.clone(),
                                stage,
                                state,
                            },
                        );
                    }
                    PumpEvent::Progress(progress) => {
                        let _ = h.emit("run://progress", progress);
                    }
                    PumpEvent::Line(_) => {}
                })
            })
        };
        let (mut total, mut dropped) = (0usize, 0usize);
        let mut last_progress = Instant::now() - EMIT_INTERVAL;
        let mut buf: VecDeque<String> = VecDeque::with_capacity(LOG_CAPACITY);
        // 当前在哪一段。各阶段串行，所以一个变量就够。
        let mut stage = String::from("mksrfdata");
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            total += 1;
            // 阶段标记先认：它不进日志窗，也不该被当成噪声丢掉。
            if let Some((name, state)) = parse_stage(&line) {
                dropped += 1;
                // 日志窗里给每段一个醒目的标题。
                if state == "begin" {
                    let heading = stage_heading(&name);
                    if buf.len() == LOG_CAPACITY {
                        buf.pop_front();
                    }
                    buf.push_back(heading.clone());
                    let _ = events.send(PumpEvent::Line(heading));
                }
                stage = name.clone();
                let _ = events.send(PumpEvent::Stage(name, state));
                continue;
            }
            if is_rangecheck_noise(&line) {
                dropped += 1;
                continue;
            }
            // 空行不承载信息，却占掉环形缓冲区一半的容量（实测 2644/5330）。
            if line.trim().is_empty() {
                dropped += 1;
                continue;
            }
            if let Some(s) = parse_progress(&line) {
                if last_progress.elapsed() >= EMIT_INTERVAL {
                    last_progress = Instant::now();
                    let _ = events.send(PumpEvent::Progress(Progress {
                        run_id: run_id.clone(),
                        case: case_id.clone(),
                        stage: stage.clone(),
                        step: s.step,
                        total_steps,
                        date: s.date,
                        spinup: s.spinup,
                    }));
                }
                continue;
            }
            if buf.len() == LOG_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(line.clone());
            let _ = events.send(PumpEvent::Line(line));
        }
        // Everything read is emitted before the caller reports completion.
        drop(events);
        let _ = emitter.join();
        (total, dropped, buf)
    })
}

/// What the stdout reader hands to the emitter, in output order.
enum PumpEvent {
    Line(String),
    Lines(Vec<String>),
    Stage(String, String),
    Progress(Progress),
}

/// Batch log lines into at most one event per `interval` (the header's rate bound) without
/// holding a line back for more than `interval`, even when the program then stays quiet for
/// minutes. Pending lines are flushed before a stage mark or progress event to keep order.
fn batch_pump_events(
    received: std::sync::mpsc::Receiver<PumpEvent>,
    interval: Duration,
    mut emit: impl FnMut(PumpEvent),
) {
    use std::sync::mpsc::RecvTimeoutError;
    let mut pending = Vec::new();
    let mut deadline: Option<Instant> = None;
    let flush = |pending: &mut Vec<String>, emit: &mut dyn FnMut(PumpEvent)| {
        if !pending.is_empty() {
            emit(PumpEvent::Lines(std::mem::take(pending)));
        }
    };
    loop {
        let next = match deadline {
            Some(at) => received.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => received.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(PumpEvent::Line(line)) => {
                pending.push(line);
                deadline.get_or_insert_with(|| Instant::now() + interval);
            }
            Ok(PumpEvent::Lines(lines)) => {
                pending.extend(lines);
                deadline.get_or_insert_with(|| Instant::now() + interval);
            }
            Ok(other) => {
                flush(&mut pending, &mut emit);
                deadline = None;
                emit(other);
            }
            Err(RecvTimeoutError::Timeout) => {
                flush(&mut pending, &mut emit);
                deadline = None;
            }
            Err(RecvTimeoutError::Disconnected) => {
                flush(&mut pending, &mut emit);
                return;
            }
        }
    }
}

/// 同时最多跑几个算例。
///
fn batch_width(requested: usize, available: usize, ranks_per_case: usize) -> usize {
    requested.clamp(1, (available / ranks_per_case.max(1)).max(1))
}

// Stages run sequentially, so reserve their largest CPU demand.
fn batch_cpu_budget(
    stage: Option<&str>,
    engine: Option<&str>,
    preprocessors: Option<&str>,
    mpi_ranks: usize,
    threads: Option<usize>,
) -> usize {
    let model = stage.is_none() || stage == Some("colm");
    let preprocessing = stage != Some("colm");
    let rust = (model && engine.unwrap_or("rust") == "rust")
        || (preprocessing && preprocessors.unwrap_or("rust") == "rust");
    let fortran =
        (model && engine == Some("fortran")) || (preprocessing && preprocessors == Some("fortran"));
    let rust_threads = if rust { threads.unwrap_or(1) } else { 1 };
    rust_threads.max(if fortran { mpi_ranks } else { 1 })
}

/// 批量里每个算例给 Rust 引擎的线程数（`RAYON_NUM_THREADS`）：同时跑的算例平分 CPU 核。
///
/// `colm-rs` 默认取全部逻辑核；同时跑 `width` 个算例时不限的话，每个都开满核，
/// 线程数是核数的 `width` 倍，互相抢 CPU。单算例运行不设，仍用全部核。
fn threads_per_case(available: usize, width: usize) -> usize {
    (available / width.max(1)).max(1)
}

fn grid_dimensions(dlon: f64, dlat: f64) -> Result<(usize, usize), String> {
    if !dlon.is_finite() || !dlat.is_finite() || dlon <= 0.0 || dlat <= 0.0 {
        return Err("dlon and dlat must be positive finite numbers".into());
    }
    let (nlon, nlat) = (360.0 / dlon, 180.0 / dlat);
    if (nlon - nlon.round()).abs() > 1e-9 || (nlat - nlat.round()).abs() > 1e-9 {
        return Err("dlon and dlat must divide the global 360° × 180° grid".into());
    }
    if nlon > usize::MAX as f64 || nlat > usize::MAX as f64 {
        return Err("grid dimensions exceed this platform's address space".into());
    }
    let dimensions = (nlon.round() as usize, nlat.round() as usize);
    let cells = dimensions
        .0
        .checked_mul(dimensions.1)
        .ok_or_else(|| "grid cell count overflows this platform".to_string())?;
    i64::try_from(cells).map_err(|_| "grid element identity exceeds int64".to_string())?;
    Ok(dimensions)
}

#[derive(Debug, Serialize)]
pub struct BatchSummary {
    total: usize,
    succeeded: usize,
    failed: usize,
}

/// 用固定数量的工作线程跑一批算例，始终把空出来的 CPU 核补上。
///
/// 一个算例失败**不中止整批**：90 个站点里有一个跑不通，其余 89 个仍要跑完。
/// 失败信息随那个算例自己的 `run://done`（`code != 0`）到达。
#[tauri::command]
// Tauri exposes these as named IPC arguments; wrapping them would only move the
// same fields into a second frontend/backend shape.
#[allow(clippy::too_many_arguments)]
pub async fn run_batch(
    app: tauri::AppHandle,
    processes: tauri::State<'_, RunProcesses>,
    run_id: String,
    cases: Vec<String>,
    kernel: String,
    max_concurrent: usize,
    force: bool,
    stage: Option<String>,
    mpi_ranks: usize,
    engine: Option<String>,
    threads: Option<usize>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
    record_input_fingerprints: Option<bool>,
) -> Result<BatchSummary, String> {
    validate_run_id(&run_id)?;
    let processes = processes.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_batch_blocking(
            app,
            processes,
            run_id,
            cases,
            kernel,
            max_concurrent,
            force,
            stage,
            mpi_ranks,
            engine,
            threads,
            model_version,
            model_revision,
            preprocessors,
            record_input_fingerprints,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

// Mirrors the command's named arguments; a wrapper would only duplicate them.
#[allow(clippy::too_many_arguments)]
fn run_batch_blocking(
    app: tauri::AppHandle,
    processes: RunProcesses,
    run_id: String,
    cases: Vec<String>,
    kernel: String,
    max_concurrent: usize,
    force: bool,
    stage: Option<String>,
    mpi_ranks: usize,
    engine: Option<String>,
    threads: Option<usize>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
    record_input_fingerprints: Option<bool>,
) -> Result<BatchSummary, String> {
    validate_run_stage(stage.as_deref())?;
    validate_engine(engine.as_deref())?;
    validate_model_version(model_version.as_deref())?;
    if mpi_ranks == 0 {
        return Err("mpiRanks must be at least 1".into());
    }
    if threads == Some(0) {
        return Err("threads must be at least 1".into());
    }
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    // run_one preserves an inherited Rayon setting over the GUI thread count.
    let threads = std::env::var_os("RAYON_NUM_THREADS")
        .map(|value| {
            value
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(available)
        })
        .or(threads);
    let total = cases.len();
    processes.prepare(&cases)?;
    let per_case = batch_cpu_budget(
        stage.as_deref(),
        engine.as_deref(),
        preprocessors.as_deref(),
        mpi_ranks,
        threads,
    );
    let width = batch_width(max_concurrent, available, per_case).min(total.max(1));
    let threads = threads.unwrap_or_else(|| threads_per_case(available, width));
    let queue = Arc::new(Mutex::new(VecDeque::from(cases)));
    let succeeded = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::with_capacity(width);
    for _ in 0..width {
        let version = model_version.clone();
        let revision = model_revision.clone();
        let preprocessors = preprocessors.clone();
        let (a, r, k, q, ok, requested_stage, p, e) = (
            app.clone(),
            run_id.clone(),
            kernel.clone(),
            Arc::clone(&queue),
            Arc::clone(&succeeded),
            stage.clone(),
            processes.clone(),
            engine.clone(),
        );
        workers.push(std::thread::spawn(move || loop {
            // 一个 worker 的 panic 不该把队列锁永久毒死；恢复锁后其余站点
            // 仍能继续排队运行。
            let case = q
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front();
            let Some(case) = case else { break };
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_one(
                    &a,
                    &p,
                    &r,
                    &case,
                    &k,
                    force,
                    requested_stage.as_deref(),
                    mpi_ranks,
                    e.as_deref(),
                    Some(threads),
                    version.as_deref(),
                    revision.as_ref(),
                    preprocessors.as_deref(),
                    record_input_fingerprints,
                )
            }));
            match outcome {
                Ok(Ok(0)) => {
                    ok.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    // 子进程连启动都失败时没有 stdout 可供 pump 发 done；这里补齐，
                    // 否则这个站点会永久停在“待运行”。
                    let _ = a.emit(
                        "run://done",
                        Done {
                            run_id: r.clone(),
                            case,
                            requested_stage: requested_stage.clone(),
                            code: -1,
                            total: 0,
                            dropped: 0,
                            reason: Some(error),
                            cancelled: false,
                        },
                    );
                }
                Err(_) => {
                    let _ = p.cancel(Some(vec![case.clone()]));
                    let _ = p.forget(&case);
                    // catch_unwind 保证这一站即使触发内部 panic，也会收到终态，
                    // UI 不会永远把它留在“运行中”，worker 还可继续取下一站。
                    let _ = a.emit(
                        "run://done",
                        Done {
                            run_id: r.clone(),
                            case,
                            requested_stage: requested_stage.clone(),
                            code: -1,
                            total: 0,
                            dropped: 0,
                            reason: Some("运行线程异常退出".into()),
                            cancelled: false,
                        },
                    );
                }
            }
        }));
    }
    for worker in workers {
        let _ = worker.join(); // 单个 worker 异常不能让其余 worker 停下
    }
    let succeeded = succeeded.load(Ordering::Relaxed);
    Ok(BatchSummary {
        total,
        succeeded,
        failed: total.saturating_sub(succeeded),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_case_blocking(
    app: tauri::AppHandle,
    processes: RunProcesses,
    run_id: String,
    case: String,
    kernel: String,
    force: bool,
    stage: Option<String>,
    mpi_ranks: usize,
    engine: Option<String>,
    threads: Option<usize>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
    record_input_fingerprints: Option<bool>,
) -> Result<i32, String> {
    if threads == Some(0) {
        return Err("threads must be at least 1".into());
    }
    processes.prepare(std::slice::from_ref(&case))?;
    run_one(
        &app,
        &processes,
        &run_id,
        &case,
        &kernel,
        force,
        stage.as_deref(),
        mpi_ranks,
        engine.as_deref(),
        threads,
        model_version.as_deref(),
        model_revision.as_ref(),
        preprocessors.as_deref(),
        record_input_fingerprints,
    )
}

/// `run_case` 里除去日志缓冲区那部分的逻辑，批量与单算例共用。
#[allow(clippy::too_many_arguments)]
fn run_one(
    app: &tauri::AppHandle,
    processes: &RunProcesses,
    run_id: &str,
    case: &str,
    kernel: &str,
    force: bool,
    stage: Option<&str>,
    mpi_ranks: usize,
    engine: Option<&str>,
    threads: Option<usize>,
    model_version: Option<&str>,
    model_revision: Option<&colm_model::VersionRef>,
    preprocessors: Option<&str>,
    record_input_fingerprints: Option<bool>,
) -> Result<i32, String> {
    crate::remote::server_case_args(case, "")?;
    let cli = resolve_cli();
    let mut cmd = sidecar_command(&cli);
    // 用户自己在环境里设了就听用户的。
    if let Some(threads) = threads.filter(|_| std::env::var_os("RAYON_NUM_THREADS").is_none()) {
        cmd.env("RAYON_NUM_THREADS", threads.to_string());
    }
    let prepared = (|| {
        let mut args = run_args(case, kernel, force, stage, mpi_ranks, engine, model_version)?;
        add_input_fingerprints(&mut args, record_input_fingerprints);
        crate::model::options(&mut args, model_revision)?;
        add_preprocessors(&mut args, preprocessors)?;
        let event = crate::history::command_start_for_run(&args, Some(run_id))?;
        Ok::<_, String>((args, event))
    })();
    let (mut args, event) = prepared.inspect_err(|_| {
        let _ = processes.forget(case);
    })?;
    if let Some(event) = &event {
        args.extend(["--operation-id".into(), event.operation_id.clone()]);
    }
    let result = (|| {
        if processes.take_cancelled(case)? {
            let _ = app.emit(
                "run://done",
                Done {
                    run_id: run_id.to_string(),
                    case: case.to_string(),
                    requested_stage: stage.map(str::to_string),
                    code: -1,
                    total: 0,
                    dropped: 0,
                    reason: Some("运行已取消".into()),
                    cancelled: true,
                },
            );
            return Ok(-1);
        }
        let mut child = colm_kernel::run::top_level_sidecar(&mut cmd)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                let _ = processes.forget(case);
                format!("cannot start {}: {e}", cli.display())
            })?;
        remember_process(processes, case, &mut child)?;
        let (out, err_pipe) = take_process_pipes(&mut child, processes, case)?;
        let _ = app.emit(
            "run://started",
            serde_json::json!({ "run_id": run_id, "case": case, "source_case":case,"operation_id":event.as_ref().map(|event|&event.operation_id) }),
        );
        let reader = pump(app, run_id, case, out);
        let error_app = app.clone();
        let error_run_id = run_id.to_string();
        let error_case = case.to_string();
        let errs = std::thread::spawn(move || {
            read_stderr_lines(BufReader::new(err_pipe), |line| {
                let _ = error_app.emit(
                    "run://lines",
                    Lines {
                        run_id: error_run_id.clone(),
                        case: error_case.clone(),
                        lines: vec![line.to_string()],
                    },
                );
            })
        });
        let (status, cancelled) = wait_process(&mut child, processes, case)?;
        let (total, dropped, _) = reader.join().map_err(|_| "reader thread panicked")?;
        let err = errs.join().map_err(|_| "stderr thread panicked")?;
        let code = status.code().unwrap_or(-1);
        let _ = app.emit(
            "run://done",
            Done {
                run_id: run_id.to_string(),
                case: case.to_string(),
                requested_stage: stage.map(str::to_string),
                code,
                total,
                dropped,
                reason: if cancelled {
                    Some("运行已取消".into())
                } else {
                    (code != 0).then(|| failure_reason(&err)).flatten()
                },
                cancelled,
            },
        );
        Ok(code)
    })();
    let status = match &result {
        Ok(0) => "succeeded",
        Ok(-1) => "cancelled",
        _ => "failed",
    };
    crate::history::command_finish(event.as_ref(), result, status)
}

/// 新建一个算例。sidecar 会补齐站点文件、生成强迫场与算例 namelist。
///
/// 经纬度、地类、时间步长、默认窗口都由 `colm-cli new` 从文件里读出来 ——
/// 界面上只需要问「选哪个站」「叫什么名字」，以及可选的窗口收窄。
#[tauri::command]
// **参数多是因为它就是 `colm-cli new` 的命令行**，不是可以随手打包的
// 内部函数签名。打成一个结构体会让前端的 `invoke` 从「一组具名参数」
// 变成「一个嵌套对象」，而 `xtask check-gui` 正是逐个比对参数名的 ——
// 那层静态检查比这条 lint 值钱。
#[allow(clippy::too_many_arguments)]
pub async fn new_case(
    creations: tauri::State<'_, CaseCreations>,
    site: String,
    out: String,
    name: Option<String>,
    start: Option<String>,
    end: Option<String>,
    // rawdata 对任何缺少地类、LAI/SAI 或土壤变量的站点都可能需要；runtime
    // 供 URBAN/BGC 等过程读取。两者都由前处理/基本设定按当前契约传入。
    rawdata: Option<String>,
    runtime: Option<String>,
    // 强迫场文件。空就不传 —— `colm-cli new` 会走命名约定（`Sitedata`
    // 的兄弟目录 `Forcing/`），内置数据集正常。只有「用自己的数据」才
    // 需要显式指定：前处理页转出来的产物不在那个位置也不叫那个名字，
    // 而约定失败的方式是**推出原始强迫场并静默用它** —— 用户以为跑的
    // 是自己转换的数据，实际跑的是原始的。
    met: Option<String>,
    // 当前向导选择的运行契约。必须在 `new` 阶段传给 CLI；PFT/PC 字段稍后
    // 才批量写入 case.nml，若不显式传，站点就绪检查只能误按 IGBP 处理。
    mode: Option<String>,
    // 进门向导选出的运行时初值。只在新建时写；已有算例绝不能被启动向导覆盖。
    fields: Vec<crate::config::FieldChange>,
    model_revision: Option<colm_model::VersionRef>,
    engine: Option<String>,
    run_host: Option<String>,
) -> Result<String, String> {
    let run_host = crate::project::validate_run_host(run_host)?;
    let fields = crate::model::creation_fields(fields, model_revision.as_ref())?;
    validate_bgc_runtime(runtime.as_deref(), &fields)?;
    let recorded_mode = mode.clone();
    let case_dir = out.clone();
    // 目录里已经有一个算例（例如名字不同、`list_cases` 里显示成别的名字）：`colm-cli new` 会在它上面
    // 重写 case.nml，把别人的算例改掉。
    if PathBuf::from(&case_dir).join("case.nml").is_file() {
        return Err(format!(
            "{case_dir} 里已经有一个算例，换一个算例名或根目录再建"
        ));
    }
    let creation = CaseCreation::begin(&creations, &case_dir, CASE_CREATION_TIMEOUT)?;
    let result = async {
        creation.validate_destination(false)?;
        let mut args = vec![
            "new".to_string(),
            "--site".into(),
            site.clone(),
            "--out".into(),
            out,
        ];
        for (flag, v) in [
            ("--name", name),
            ("--start", start),
            ("--end", end),
            ("--rawdata", rawdata),
            ("--runtime", runtime),
            ("--met", met),
            ("--mode", mode),
        ] {
            if let Some(v) = v {
                if !v.trim().is_empty() {
                    args.push(flag.into());
                    args.push(v);
                }
            }
        }
        if is_crop_case(&fields) {
            args.push("--crop".into());
            args.push("1".into());
        }
        let output = creation.capture(args).await?;
        creation.check()?;
        crate::config::apply_fields(&case_dir, &fields)?;
        // 记下所选计算核心与站点；同目录约定下的观测文件作为结果页的默认对照。
        let mut record = crate::project::CaseRecord {
            mode: recorded_mode,
            run_host,
            engine,
            site: Some(site.clone()),
            ..Default::default()
        };
        record.obs_file =
            crate::project::case_default_observation(std::path::Path::new(&case_dir), &record)
                .or_else(|| {
                    crate::project::sibling(std::path::Path::new(&site), "Observation", 1)
                        .map(|p| p.display().to_string())
                });
        crate::project::record_case(&case_dir, &record)?;
        creation.check()?;
        crate::model::bind_new_case(&case_dir, model_revision, &creation).await?;
        Ok(output)
    }
    .await;
    creation.finish(result)
}

struct SpatialCaseRequest {
    domain: String,
    grid_kind: String,
    shapefile: Option<String>,
    west: Option<f64>,
    east: Option<f64>,
    south: Option<f64>,
    north: Option<f64>,
    dlon: Option<f64>,
    dlat: Option<f64>,
    mesh_file: Option<String>,
    catchment_file: Option<String>,
    out: String,
    name: String,
    /// 已有的 forcing namelist；与 `forcing_dataset` 二选一。
    forcing: Option<String>,
    /// 标准网格强迫数据集名（`colm_case::forcing_templates`）与本机数据目录：建例时写出 `<case>/forcing.nml`。
    forcing_dataset: Option<String>,
    forcing_dir: Option<String>,
    rawdata: String,
    runtime: String,
    start: String,
    end: String,
    timestep: f64,
    mode: String,
    fields: Vec<crate::config::FieldChange>,
}

/// 可选的标准网格强迫数据集（上游 `run/forcing/*.nml` 模板，站点用的 `POINT` 除外）。
#[tauri::command]
pub fn forcing_datasets() -> Vec<String> {
    colm_case::forcing_templates::forcing_datasets()
        .map(str::to_owned)
        .collect()
}

/// 重定向算例的输出目录至外部自定义路径（通过软链接连接 `<case>/out`）。
#[tauri::command]
pub fn redirect_output_dir(case_dir: String, target_dir: String) -> Result<(), String> {
    let case_path = PathBuf::from(&case_dir);
    let out_link = case_path.join("out");
    let target_path = PathBuf::from(&target_dir);

    let out_canonical = out_link.canonicalize().unwrap_or_else(|_| out_link.clone());
    let target_canonical = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());
    if out_canonical == target_canonical {
        return Ok(());
    }

    std::fs::create_dir_all(&target_path)
        .map_err(|e| format!("无法创建输出目录 {}: {e}", target_path.display()))?;

    if out_link.is_symlink() {
        #[cfg(unix)]
        let _ = std::fs::remove_file(&out_link);
        #[cfg(windows)]
        let _ = std::fs::remove_dir(&out_link);
    } else if out_link.is_dir() {
        let is_empty = std::fs::read_dir(&out_link)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if is_empty {
            let _ = std::fs::remove_dir(&out_link);
        } else {
            return Err(format!(
                "默认输出目录 {} 已存在且非空，未做链接",
                out_link.display()
            ));
        }
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target_path, &out_link)
        .map_err(|e| format!("无法创建输出目录软链接至 {}: {e}", target_path.display()))?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target_path, &out_link)
        .map_err(|e| format!("无法创建输出目录软链接至 {}: {e}", target_path.display()))?;

    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn new_spatial_case(
    creations: tauri::State<'_, CaseCreations>,
    domain: String,
    grid_kind: String,
    shapefile: Option<String>,
    west: Option<f64>,
    east: Option<f64>,
    south: Option<f64>,
    north: Option<f64>,
    dlon: Option<f64>,
    dlat: Option<f64>,
    mesh_file: Option<String>,
    catchment_file: Option<String>,
    out: String,
    name: String,
    forcing: Option<String>,
    forcing_dataset: Option<String>,
    forcing_dir: Option<String>,
    rawdata: String,
    runtime: String,
    start: String,
    end: String,
    timestep: f64,
    mode: String,
    fields: Vec<crate::config::FieldChange>,
    model_revision: Option<colm_model::VersionRef>,
    engine: Option<String>,
    run_host: Option<String>,
) -> Result<String, String> {
    let run_host = crate::project::validate_run_host(run_host)?;
    let mut fields = crate::model::creation_fields(fields, model_revision.as_ref())?;
    preserve_generated_forcing(forcing_dataset.as_deref(), &mut fields);
    let request = SpatialCaseRequest {
        domain,
        grid_kind,
        shapefile,
        west,
        east,
        south,
        north,
        dlon,
        dlat,
        mesh_file,
        catchment_file,
        out,
        name,
        forcing,
        forcing_dataset,
        forcing_dir,
        rawdata,
        runtime,
        start,
        end,
        timestep,
        mode,
        fields,
    };
    validate_bgc_runtime(Some(&request.runtime), &request.fields)?;
    if !matches!(request.domain.as_str(), "watershed" | "region" | "global") {
        return Err("domain must be watershed, region, or global".into());
    }
    if !matches!(
        request.grid_kind.as_str(),
        "latlon" | "unstructured" | "catchment"
    ) {
        return Err("gridKind must be latlon, unstructured, or catchment".into());
    }
    let case_dir = PathBuf::from(&request.out);
    if case_dir.exists() {
        return Err(format!(
            "case directory already exists; choose a new case name: {}",
            case_dir.display()
        ));
    }
    let creation = CaseCreation::begin(&creations, &request.out, CASE_CREATION_TIMEOUT)?;
    let result = async {
        creation.validate_destination(true)?;
        std::fs::create_dir_all(&case_dir)
            .map_err(|e| format!("cannot create {}: {e}", case_dir.display()))?;
        let cleanup = |error: String| creation.failure(error);
        let existing_mesh =
            existing_unstructured_mesh(&request.grid_kind, request.mesh_file.clone())
                .map_err(cleanup)?;
        let mesh = if let Some(mesh) = existing_mesh {
            mesh
        } else if request.grid_kind == "catchment" {
            request
                .catchment_file
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| cleanup("catchmentFile is required".into()))?
        } else {
            let dlon = request
                .dlon
                .ok_or_else(|| cleanup("dlon is required".into()))?;
            let dlat = request
                .dlat
                .ok_or_else(|| cleanup("dlat is required".into()))?;
            let (nlon, nlat) = grid_dimensions(dlon, dlat).map_err(cleanup)?;
            let mesh = case_dir.join("mesh.nc").to_string_lossy().into_owned();
            let mut args = vec![
                "mesh-new".into(),
                "--out".into(),
                mesh.clone(),
                "--grid-kind".into(),
                request.grid_kind.clone(),
                "--nlon".into(),
                nlon.to_string(),
                "--nlat".into(),
                nlat.to_string(),
            ];
            // 不传非海洋 mask：`DEF_LANDONLY`（缺省开）已按 RawData 的地表覆盖剔除海洋，上游的
            // `MOD_MeshFilter` 也是可选的。`colm-cli mesh-new --non-ocean-mask` 留给命令行的高级用法。
            match request.domain.as_str() {
                "watershed" => {
                    let shp = request
                        .shapefile
                        .clone()
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| cleanup("shapefile is required".into()))?;
                    args.extend(["--shp".into(), shp]);
                }
                "region" => {
                    for (flag, value) in [
                        ("--west", request.west),
                        ("--east", request.east),
                        ("--south", request.south),
                        ("--north", request.north),
                    ] {
                        args.extend([
                            flag.into(),
                            value
                                .ok_or_else(|| cleanup(format!("{flag} is required")))?
                                .to_string(),
                        ]);
                    }
                }
                "global" => {}
                _ => unreachable!(),
            }
            creation.capture(args).await.map_err(cleanup)?;
            mesh
        };
        let manifest = case_dir
            .join("spatial-input.manifest.json")
            .to_string_lossy()
            .into_owned();
        creation
            .capture(vec![
                "spatial-preflight".into(),
                "--grid-kind".into(),
                request.grid_kind.clone(),
                "--input".into(),
                mesh.clone(),
                "--out".into(),
                manifest,
            ])
            .await
            .map_err(cleanup)?;
        // 强迫场：选了标准数据集就按模板写出 `<case>/forcing.nml`；否则用用户给的 namelist。
        let forcing = match request
            .forcing_dataset
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(dataset) => {
                let dir = request
                    .forcing_dir
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| cleanup("forcingDir is required with forcingDataset".into()))?;
                if !std::path::Path::new(dir).is_absolute() || !std::path::Path::new(dir).is_dir() {
                    return Err(cleanup(format!(
                        "choose an existing absolute forcing data directory: {dir}"
                    )));
                }
                let text = colm_case::forcing_templates::render_forcing_namelist(dataset, dir)
                    .map_err(|e| cleanup(format!("{e:#}")))?;
                let path = case_dir.join("forcing.nml");
                std::fs::write(&path, text)
                    .map_err(|e| cleanup(format!("cannot write {}: {e}", path.display())))?;
                path.to_string_lossy().into_owned()
            }
            None => request
                .forcing
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    cleanup("choose a forcing dataset or an existing forcing namelist".into())
                })?,
        };
        let recorded = crate::project::CaseRecord {
            mode: Some(request.mode.clone()),
            run_host,
            domain: Some(request.domain.clone()),
            shapefile: request.shapefile.clone(),
            engine,
            ..Default::default()
        };
        let mut args = vec![
            "spatial-new".into(),
            "--grid-kind".into(),
            request.grid_kind.clone(),
            "--mesh".into(),
            mesh,
            "--out".into(),
            request.out.clone(),
            "--name".into(),
            request.name,
            "--forcing".into(),
            forcing,
            "--rawdata".into(),
            request.rawdata,
            "--runtime".into(),
            request.runtime,
            "--start".into(),
            request.start,
            "--end".into(),
            request.end,
            "--timestep".into(),
            request.timestep.to_string(),
            "--mode".into(),
            request.mode,
        ];
        if request.grid_kind == "latlon" {
            args.extend([
                "--dlon".into(),
                request.dlon.expect("validated above").to_string(),
                "--dlat".into(),
                request.dlat.expect("validated above").to_string(),
            ]);
        }
        let output = creation.capture(args).await.map_err(cleanup)?;
        crate::config::apply_fields(&request.out, &request.fields).map_err(cleanup)?;
        crate::project::record_case(&request.out, &recorded)?;
        creation.check()?;
        crate::model::bind_new_case(&request.out, model_revision, &creation).await?;
        Ok(output)
    }
    .await;
    creation.finish(result)
}

fn existing_unstructured_mesh(
    grid_kind: &str,
    mesh_file: Option<String>,
) -> Result<Option<String>, String> {
    let mesh_file = mesh_file.filter(|value| !value.trim().is_empty());
    match (grid_kind, mesh_file) {
        ("unstructured", Some(mesh)) => Ok(Some(mesh)),
        ("unstructured", None) => Err("meshFile is required for an unstructured grid".into()),
        (_, Some(_)) => Err("已有 mesh NetCDF 目前仅支持非结构网格".into()),
        (_, None) => Ok(None),
    }
}

fn is_crop_case(fields: &[crate::config::FieldChange]) -> bool {
    fields.iter().any(|field| {
        field
            .path
            .eq_ignore_ascii_case("DEF_TUNING_CROP_PLANTING_DAY")
    })
}

fn validate_bgc_runtime(
    runtime: Option<&str>,
    fields: &[crate::config::FieldChange],
) -> Result<(), String> {
    let field = |name: &str| {
        fields
            .iter()
            .rev()
            .find(|field| field.path == name)
            .map(|field| field.value.trim())
    };
    let logical = |name: &str, default| {
        field(name)
            .map(|value| matches!(value.to_ascii_lowercase().as_str(), ".true." | "true" | "t"))
            .unwrap_or(default)
    };
    let integer = |name: &str, default| {
        field(name)
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(default)
    };
    let real = |name: &str, default| {
        field(name)
            .and_then(|value| value.split('_').next())
            .and_then(|value| value.replace(['d', 'D'], "e").parse::<f64>().ok())
            .unwrap_or(default)
    };
    let bgc = logical("DEF_USE_BGC", false);
    if !bgc {
        return Ok(());
    }
    let root = runtime
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    crate::config::validate_bgc_runtime_dir(
        root.as_deref(),
        bgc,
        integer("DEF_NDEP_FREQUENCY", 1),
        logical("DEF_USE_NITRIF", true),
        logical("DEF_USE_FIRE", false),
    )?;
    let root = root.expect("BGC runtime was validated above");
    if is_crop_case(fields) {
        for (name, label) in crate::config::crop_runtime_files(
            real("DEF_TUNING_CROP_PLANTING_DAY", 0.0),
            logical("DEF_USE_FERT", false),
            integer("DEF_FERT_SOURCE", 1),
            logical("DEF_USE_IRRIGATION", false),
            integer("DEF_IRRIGATION_ALLOCATION", 1),
        ) {
            let file = root.join(name);
            if !file.is_file() {
                return Err(format!("{label}运行时目录缺少数据：{}", file.display()));
            }
        }
    }
    Ok(())
}

/// 评估：把模型与观测配对，出指标与配对点。
///
/// 走 sidecar 而不是在这里算 —— 要读两个 NetCDF 文件。
/// 单站点**一次拿全**：指标表、双线图、散点图用同一批配对结果。
/// 多站点排名传 `summary_only`，避免把每站几十万配对点送进 WebView。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn metrics(
    case: String,
    obs: String,
    spinup: usize,
    corrected: bool,
    summary_only: bool,
    pair_vars: Option<Vec<String>>,
    max_points: Option<usize>,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<String, String> {
    if let Some(host) = crate::remote::results_on_server(&case) {
        return Err(format!(
            "这次运行的结果在服务器 {host} 上；在服务器上算评估指标还没做，请先在结果页点\"取回结果\""
        ));
    }
    capture_async(metrics_args(
        case,
        obs,
        spinup,
        corrected,
        summary_only,
        pair_vars,
        max_points,
        from,
        to,
    ))
    .await
}

#[allow(clippy::too_many_arguments)]
fn metrics_args(
    case: String,
    obs: String,
    spinup: usize,
    corrected: bool,
    summary_only: bool,
    pair_vars: Option<Vec<String>>,
    max_points: Option<usize>,
    from: Option<i64>,
    to: Option<i64>,
) -> Vec<String> {
    let mut args = vec![
        "metrics".to_string(),
        case,
        "--obs".into(),
        obs,
        "--spinup".into(),
        spinup.to_string(),
        "--json".into(),
        "1".into(),
    ];
    // 能量闭合订正。**默认关**：design.md §2.8 的目标值是拿未订正的观测算的，
    // 换默认会让那些数字集体失效。但订正版回答的是另一个问题 ——
    // 实测 AT-Neu：未订正时 Qle 偏差 +19.8 W/m²，订正后是 -1.2。
    if corrected {
        args.push("--corrected".into());
        args.push("1".into());
    }
    if summary_only {
        args.push("--summary-only".into());
        args.push("1".into());
    }
    for pair_var in pair_vars.unwrap_or_default() {
        args.push("--pairs-var".into());
        args.push(pair_var);
    }
    if let Some(max_points) = max_points {
        args.push("--max-points".into());
        args.push(max_points.to_string());
    }
    for (flag, value) in [("--from", from), ("--to", to)] {
        if let Some(value) = value {
            args.push(flag.into());
            args.push(value.to_string());
        }
    }
    args
}

/// 当前算例与观测文件共同支持哪些评估变量。只读首个 history 文件的结构，
/// 不加载长时间序列，供 GUI 在真正计算前展示完整可选清单和缺失原因。
#[tauri::command]
pub async fn evaluation_catalog(case: String, obs: String) -> Result<String, String> {
    capture_async(vec![
        "evaluation-catalog".to_string(),
        case,
        "--obs".into(),
        obs,
    ])
    .await
}

fn evaluation_plan_args(case: String, obs: String, kernel_dir: String) -> Vec<String> {
    vec![
        "evaluation-plan".to_string(),
        case,
        "--obs".into(),
        obs,
        "--kernel".into(),
        kernel_dir,
    ]
}

/// 未运行算例也能预览哪些评估目标可用于不确定性分析/参数调优。
#[tauri::command]
pub async fn evaluation_plan(
    case: String,
    obs: String,
    kernel_dir: String,
) -> Result<String, String> {
    capture_async(evaluation_plan_args(case, obs, kernel_dir)).await
}

/// 取绘图数据。
///
/// 走 sidecar 而不是在这里读 —— 窗口进程不链接 netcdf，
/// 不该为了画一条曲线把整个静态 HDF5 拖进来。
#[tauri::command]
pub async fn series(
    app: tauri::AppHandle,
    case: String,
    vars: String,
    from: Option<i64>,
    to: Option<i64>,
    max_points: Option<usize>,
) -> Result<String, String> {
    if let Some(host) = crate::remote::results_on_server(&case) {
        let mut flags = vec![("--vars", vars)];
        for (flag, value) in [
            ("--from", from.map(|v| v.to_string())),
            ("--to", to.map(|v| v.to_string())),
            ("--max-points", max_points.map(|v| v.to_string())),
        ] {
            if let Some(value) = value {
                flags.push((flag, value));
            }
        }
        return crate::remote::read_on_server(&app, &host, case, "series", flags).await;
    }
    let mut args = vec!["series".to_string(), case, "--vars".into(), vars];
    for (flag, value) in [
        ("--from", from.map(|v| v.to_string())),
        ("--to", to.map(|v| v.to_string())),
        ("--max-points", max_points.map(|v| v.to_string())),
    ] {
        if let Some(value) = value {
            args.push(flag.into());
            args.push(value);
        }
    }
    capture_async(args).await
}

/// 全量能量通量统计与同一数据的绘图样本；窗口进程不读取 NetCDF。
#[tauri::command]
pub async fn energy_summary(
    app: tauri::AppHandle,
    case: String,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<String, String> {
    if let Some(host) = crate::remote::results_on_server(&case) {
        let flags = [("--from", from), ("--to", to)]
            .into_iter()
            .filter_map(|(flag, value)| value.map(|v| (flag, v.to_string())))
            .collect();
        return crate::remote::read_on_server(&app, &host, case, "energy-summary", flags).await;
    }
    let mut args = vec!["energy-summary".to_string(), case];
    for (flag, value) in [("--from", from), ("--to", to)] {
        if let Some(value) = value {
            args.extend([flag.into(), value.to_string()]);
        }
    }
    capture_async(args).await
}

/// 轻量结果索引：变量、单位、维度与时间覆盖。数值仍由 `series` 按需读取。
#[tauri::command]
pub async fn history_catalog(app: tauri::AppHandle, case: String) -> Result<String, String> {
    if let Some(host) = crate::remote::results_on_server(&case) {
        return crate::remote::read_on_server(&app, &host, case, "history-catalog", Vec::new())
            .await;
    }
    capture_async(vec!["history-catalog".to_string(), case]).await
}

/// 格点变量某一时间步的整张图（`colm-cli history-map`）。服务器上的运行照样在服务器上读，只传回这一张图。
#[tauri::command]
pub async fn history_map(
    app: tauri::AppHandle,
    case: String,
    var: String,
    index: usize,
) -> Result<String, String> {
    let flags = vec![("--var", var), ("--index", index.to_string())];
    if let Some(host) = crate::remote::results_on_server(&case) {
        return crate::remote::read_on_server(&app, &host, case, "history-map", flags).await;
    }
    let mut args = vec!["history-map".to_string(), case];
    for (flag, value) in flags {
        args.push(flag.into());
        args.push(value);
    }
    capture_async(args).await
}

#[derive(Serialize)]
pub struct StudyParameterContextRow {
    name: String,
    id: String,
    scope: &'static str,
    scope_instance: crate::config::ParameterScopeInstance,
    catalog_version: u32,
    default: f64,
    default_provider: String,
    label_zh: String,
    label_en: String,
    scale: String,
    review: &'static str,
    min: Option<f64>,
    min_inclusive: Option<bool>,
    max: Option<f64>,
    max_inclusive: Option<bool>,
    sentinel: Option<f64>,
    sentinel_meaning: Option<&'static str>,
}

/// Contextual Study selector rows derived from the same catalog and value
/// readers used by manual editing. Indexed rows are emitted only when every
/// selected case shares that exact LCT class or PFT/PC component.
#[tauri::command]
pub async fn study_parameter_contexts(
    dirs: Vec<String>,
    kernel_dir: String,
    types: Option<Vec<u8>>,
) -> Result<Vec<StudyParameterContextRow>, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
    }
    let choices = study_type_choices_for(&dirs, &kernel_dir, false).await?;
    if choices.spatial {
        return spatial_study_rows(&dirs, &kernel_dir, &choices, &types.unwrap_or_default());
    }
    let field_states = crate::config::field_states_batch(dirs.clone(), kernel_dir.clone())?;
    let fields = field_states
        .iter()
        .map(|state| (state.name.to_ascii_lowercase(), state))
        .collect::<HashMap<_, _>>();
    let mut rows = Vec::new();
    for descriptor in colm_case::parameters::all().iter().filter(|descriptor| {
        descriptor.calibration_eligible
            && !descriptor.structural_parameter
            && matches!(
                descriptor.scope,
                colm_case::parameters::ParameterScope::CaseScalar
            )
    }) {
        let Some(state) = fields.get(&descriptor.raw_key.to_ascii_lowercase()) else {
            continue;
        };
        if !matches!(state.mode, crate::config::FieldMode::Editable)
            || state.mixed
            || state.effective_mixed
        {
            continue;
        }
        let Some(default) = state.effective_value.as_deref().and_then(parse_number) else {
            continue;
        };
        rows.push(study_row(
            descriptor,
            "case-scalar",
            crate::config::ParameterScopeInstance {
                kind: "case-scalar".into(),
                scheme: None,
                index: None,
                type_name: None,
                process_file: None,
            },
            default,
        )?);
    }

    let lct = crate::config::land_cover_contexts(dirs.clone(), kernel_dir.clone())?;
    if lct.len() == dirs.len()
        && lct.iter().all(|context| {
            context.scheme == lct[0].scheme && context.class_index == lct[0].class_index
        })
    {
        let context = &lct[0];
        for descriptor in colm_case::parameters::land_cover_descriptors()
            .into_iter()
            .filter(|descriptor| {
                descriptor.calibration_eligible
                    && descriptor
                        .id
                        .split(':')
                        .nth(1)
                        .is_some_and(|scheme| scheme == context.scheme)
            })
        {
            let Some(state) = fields.get(&descriptor.raw_key.to_ascii_lowercase()) else {
                continue;
            };
            if !matches!(state.mode, crate::config::FieldMode::Editable)
                || state.mixed
                || state.default_mixed
                || state.effective_mixed
            {
                continue;
            }
            let Some(default) = state.effective_value.as_deref().and_then(parse_number) else {
                continue;
            };
            rows.push(study_row(
                descriptor,
                "land-cover-class",
                crate::config::ParameterScopeInstance {
                    kind: "land-cover-class".into(),
                    scheme: Some(context.scheme.into()),
                    index: Some(context.class_index),
                    type_name: None,
                    process_file: None,
                },
                default,
            )?);
        }
    }

    let mut common_pfts: Option<HashMap<u8, crate::sitedata::PftComponentReport>> = None;
    for dir in &dirs {
        let components = match crate::sitedata::site_pfts(dir.clone(), kernel_dir.clone()).await {
            Ok(components) => components,
            Err(_) => {
                common_pfts = None;
                break;
            }
        };
        let current = components
            .into_iter()
            .filter(|component| component.pft_type != 0)
            .map(|component| (component.pft_type, component))
            .collect::<HashMap<_, _>>();
        match &mut common_pfts {
            Some(common) => common.retain(|pft, _| current.contains_key(pft)),
            None => common_pfts = Some(current),
        }
    }
    if let Some(common_pfts) = common_pfts {
        let mut pft_types = common_pfts.keys().copied().collect::<Vec<_>>();
        pft_types.sort_unstable();
        for pft_type in pft_types {
            let states =
                crate::config::pft_parameter_states(dirs.clone(), pft_type, kernel_dir.clone())?;
            for state in states
                .into_iter()
                .filter(|state| !state.default_mixed && !state.mixed)
            {
                let scope = if state.scope_kind == "pc-pft" {
                    colm_case::parameters::ParameterScope::PcPftComponent
                } else {
                    colm_case::parameters::ParameterScope::PftType
                };
                let Some(descriptor) = colm_case::parameters::all().iter().find(|descriptor| {
                    descriptor.raw_key.eq_ignore_ascii_case(state.name)
                        && descriptor.scope == scope
                        && descriptor.calibration_eligible
                        && !descriptor.structural_parameter
                }) else {
                    continue;
                };
                let Some(default) = parse_number(&state.effective_value) else {
                    continue;
                };
                let component = &common_pfts[&pft_type];
                rows.push(study_row(
                    descriptor,
                    if state.scope_kind == "pc-pft" {
                        "pc-pft-component"
                    } else {
                        "pft-type"
                    },
                    crate::config::ParameterScopeInstance {
                        kind: if state.scope_kind == "pc-pft" {
                            "pc-pft-component"
                        } else {
                            "pft-type"
                        }
                        .into(),
                        scheme: None,
                        index: Some(pft_type),
                        type_name: Some(component.name_en.clone()),
                        process_file: None,
                    },
                    default,
                )?);
            }
        }
    }
    rows.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then(left.scope_instance.index.cmp(&right.scope_instance.index))
    });
    Ok(rows)
}

/// 研究页选地类或植物类型用：这批算例是单点还是格点、按哪套类型（IGBP、USGS、PFT、PC）、格点算例里实际有哪些地类。
#[derive(Debug, Serialize)]
pub struct StudyTypeChoices {
    spatial: bool,
    /// `igbp`、`usgs`、`pft`、`pc`。
    kind: &'static str,
    /// 地类或植物类型号的上限（含）：IGBP 17、USGS 24、PFT 15（CROP 内核 78）。
    max: u8,
    /// 格点 LCT 算例 landdata 里各算例都有的地类；没做过前处理或不是 LCT 时为空（那就全部可选）。
    present: Option<Vec<u8>>,
}

#[tauri::command]
pub async fn study_type_choices(
    dirs: Vec<String>,
    kernel_dir: String,
) -> Result<StudyTypeChoices, String> {
    study_type_choices_for(&dirs, &kernel_dir, true).await
}

async fn study_type_choices_for(
    dirs: &[String],
    kernel_dir: &str,
    read_present: bool,
) -> Result<StudyTypeChoices, String> {
    let kernel = colm_kernel::Kernel::open_manifest(std::path::Path::new(kernel_dir))
        .map_err(|error| format!("{error:#}"))?;
    let has = |name: &str| kernel.manifest.macros.iter().any(|item| item == name);
    let mut spatial = None;
    let mut subgrid = None;
    for dir in dirs {
        let case_nml = std::path::Path::new(dir).join("case.nml");
        let this = colm_case::is_spatial_case(&case_nml).map_err(|error| format!("{error:#}"))?;
        if spatial.replace(this).is_some_and(|before| before != this) {
            return Err("一次研究里不能同时有单点和格点算例".into());
        }
        let text = std::fs::read_to_string(&case_nml).map_err(|error| format!("{dir}: {error}"))?;
        let doc = colm_namelist::parse(&text).map_err(|error| format!("{dir}: {error:#}"))?;
        let logical = |name: &str| matches!(doc.get(name), Some(colm_namelist::Value::Bool(true)));
        let kind = if logical("DEF_USE_PC") {
            "pc"
        } else if logical("DEF_USE_PFT") {
            "pft"
        } else if has("LULC_USGS") {
            "usgs"
        } else {
            "igbp"
        };
        if subgrid.replace(kind).is_some_and(|before| before != kind) {
            return Err("所选算例的次网格结构（LCT、PFT、PC）不同；请分开研究".into());
        }
    }
    let spatial = spatial.unwrap_or(false);
    let kind = subgrid.unwrap_or("igbp");
    let max = match kind {
        "usgs" => 24,
        "igbp" => 17,
        "pft" | "pc" if has("CROP") => 78,
        "pft" | "pc" => 15,
        other => return Err(format!("unknown subgrid structure {other}")),
    };
    let mut present: Option<Vec<u8>> = None;
    if read_present && spatial && matches!(kind, "igbp" | "usgs") {
        for dir in dirs {
            let json = capture_async(vec!["case-land-classes".into(), dir.clone()]).await?;
            let value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| format!("colm-cli case-land-classes 的输出解析不了：{error}"))?;
            let Some(classes) = value["classes"].as_array() else {
                present = None;
                break;
            };
            let here = classes
                .iter()
                .filter_map(|class| class.as_u64().and_then(|class| u8::try_from(class).ok()))
                .collect::<Vec<_>>();
            present = Some(match present {
                Some(before) => before
                    .into_iter()
                    .filter(|class| here.contains(class))
                    .collect(),
                None => here,
            });
        }
    }
    Ok(StudyTypeChoices {
        spatial,
        kind,
        max,
        present,
    })
}

/// 格点算例的研究参数：整算例系数照常从字段状态取；地类（LCT）或植物类型（PFT/PC）参数按研究页选的类型号
/// 逐个给一行，默认值取各类型的表值。活动条件（植物水力、气孔方案）按算例开关筛；地类在不在 landdata 里由
/// 建研究时的检查报。
fn spatial_study_rows(
    dirs: &[String],
    kernel_dir: &str,
    choices: &StudyTypeChoices,
    types: &[u8],
) -> Result<Vec<StudyParameterContextRow>, String> {
    let field_states = crate::config::field_states_batch(dirs.to_vec(), kernel_dir.to_string())?;
    let fields = field_states
        .iter()
        .map(|state| (state.name.to_ascii_lowercase(), state))
        .collect::<HashMap<_, _>>();
    let mut rows = Vec::new();
    for descriptor in colm_case::parameters::all().iter().filter(|descriptor| {
        descriptor.calibration_eligible
            && !descriptor.structural_parameter
            && matches!(
                descriptor.scope,
                colm_case::parameters::ParameterScope::CaseScalar
            )
    }) {
        let Some(state) = fields.get(&descriptor.raw_key.to_ascii_lowercase()) else {
            continue;
        };
        if !matches!(state.mode, crate::config::FieldMode::Editable)
            || state.mixed
            || state.effective_mixed
        {
            continue;
        }
        let Some(default) = state.effective_value.as_deref().and_then(parse_number) else {
            continue;
        };
        rows.push(study_row(
            descriptor,
            "case-scalar",
            crate::config::ParameterScopeInstance {
                kind: "case-scalar".into(),
                scheme: None,
                index: None,
                type_name: None,
                process_file: None,
            },
            default,
        )?);
    }
    let docs = dirs
        .iter()
        .map(|dir| {
            let text = std::fs::read_to_string(std::path::Path::new(dir).join("case.nml"))
                .map_err(|error| format!("{dir}: {error}"))?;
            colm_namelist::parse(&text).map_err(|error| format!("{dir}: {error:#}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let all = |name: &str, default: bool| {
        docs.iter().all(|doc| match doc.get(name) {
            Some(colm_namelist::Value::Bool(value)) => *value,
            _ => default,
        })
    };
    let schema_default = |name: &str| {
        matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Logical(true))
        )
    };
    let plant_hydraulics = all(
        "DEF_USE_PLANTHYDRAULICS",
        schema_default("DEF_USE_PLANTHYDRAULICS"),
    );
    let medlyn = all("DEF_USE_MEDLYNST", schema_default("DEF_USE_MEDLYNST"));
    let wue = all("DEF_USE_WUEST", schema_default("DEF_USE_WUEST"));
    let types = types
        .iter()
        .copied()
        .filter(|index| *index >= 1 && *index <= choices.max)
        .collect::<Vec<_>>();
    if matches!(choices.kind, "igbp" | "usgs") {
        let scheme = choices.kind.to_ascii_uppercase();
        let usgs = choices.kind == "usgs";
        for descriptor in colm_case::parameters::land_cover_descriptors()
            .into_iter()
            .filter(|descriptor| {
                descriptor.calibration_eligible
                    && descriptor.id.split(':').nth(1) == Some(scheme.as_str())
            })
        {
            use colm_case::land_cover::Condition;
            let Some(meta) = colm_case::land_cover::parameter(&descriptor.raw_key) else {
                continue;
            };
            let active = match meta.condition {
                Condition::Always => true,
                Condition::PlantHydraulics => plant_hydraulics,
                Condition::BallBerry => !medlyn && !wue,
                Condition::Medlyn => medlyn && !wue,
                Condition::Wue => wue && !medlyn,
            };
            if !active {
                continue;
            }
            for &class in &types {
                let Some(default) = colm_case::land_cover::default_value(
                    &descriptor.raw_key,
                    usgs,
                    i64::from(class),
                )
                .map_err(|error| format!("{error:#}"))?
                else {
                    continue;
                };
                rows.push(study_row(
                    descriptor,
                    "land-cover-class",
                    crate::config::ParameterScopeInstance {
                        kind: "land-cover-class".into(),
                        scheme: Some(scheme.clone()),
                        index: Some(class),
                        type_name: None,
                        process_file: None,
                    },
                    default,
                )?);
            }
        }
    } else {
        let pc = choices.kind == "pc";
        let campbell = all(
            "DEF_USE_Campbell_SOIL_MODEL",
            schema_default("DEF_USE_Campbell_SOIL_MODEL"),
        );
        let scope = if pc {
            colm_case::parameters::ParameterScope::PcPftComponent
        } else {
            colm_case::parameters::ParameterScope::PftType
        };
        let label = if pc { "pc-pft-component" } else { "pft-type" };
        for descriptor in colm_case::parameters::all().iter().filter(|descriptor| {
            descriptor.scope == scope
                && descriptor.calibration_eligible
                && !descriptor.structural_parameter
        }) {
            for &pft in &types {
                let Some(default) =
                    colm_case::pft::default_value(&descriptor.raw_key, pft, campbell, pc)
                        .map_err(|error| format!("{error:#}"))?
                        .filter(|value| *value != -999.0 && (*value + 999.9).abs() >= 1e-9)
                else {
                    continue;
                };
                rows.push(study_row(
                    descriptor,
                    label,
                    crate::config::ParameterScopeInstance {
                        kind: label.into(),
                        scheme: None,
                        index: Some(pft),
                        type_name: colm_case::pft::pft_name(pft).map(|name| name.en.to_string()),
                        process_file: None,
                    },
                    default,
                )?);
            }
        }
    }
    rows.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then(left.scope_instance.index.cmp(&right.scope_instance.index))
    });
    Ok(rows)
}

fn study_row(
    descriptor: &colm_case::parameters::ParameterDescriptor,
    scope: &'static str,
    scope_instance: crate::config::ParameterScopeInstance,
    default: f64,
) -> Result<StudyParameterContextRow, String> {
    let tuning =
        colm_case::tuning::find(&descriptor.raw_key).map_err(|error| format!("{error:#}"))?;
    let pft = colm_case::pft::parameter(&descriptor.raw_key);
    let lc = colm_case::land_cover::parameter(&descriptor.raw_key);
    let min = tuning
        .and_then(|meta| meta.min.map(|bound| bound.value))
        .or_else(|| pft.and_then(|meta| meta.min))
        .or_else(|| lc.and_then(|meta| meta.min));
    let max = tuning
        .and_then(|meta| meta.max.map(|bound| bound.value))
        .or_else(|| pft.and_then(|meta| meta.max))
        .or_else(|| lc.and_then(|meta| meta.max));
    let min_inclusive = min.map(|bound| {
        tuning
            .and_then(|meta| meta.min.map(|value| value.inclusive))
            .unwrap_or_else(|| validate_bound(descriptor, bound))
    });
    let max_inclusive = max.map(|bound| {
        tuning
            .and_then(|meta| meta.max.map(|value| value.inclusive))
            .unwrap_or_else(|| validate_bound(descriptor, bound))
    });
    let sentinel = tuning
        .and_then(|meta| meta.sentinel.map(|value| value.value))
        .or_else(|| lc.map(|meta| meta.sentinel));
    let sentinel_meaning = tuning
        .and_then(|meta| meta.sentinel.map(|value| value.meaning))
        .or_else(|| lc.map(|_| "inherit contextual default"));
    Ok(StudyParameterContextRow {
        name: descriptor.raw_key.clone(),
        id: descriptor.id.clone(),
        scope,
        scope_instance,
        catalog_version: descriptor.catalog_version,
        default,
        default_provider: descriptor.default_provider.clone(),
        label_zh: descriptor.label_zh.clone(),
        label_en: descriptor.label_en.clone(),
        scale: descriptor
            .recommended_scale
            .clone()
            .unwrap_or_else(|| "linear".into()),
        review: "expert_range_only",
        min,
        min_inclusive,
        max,
        max_inclusive,
        sentinel,
        sentinel_meaning,
    })
}

fn validate_bound(descriptor: &colm_case::parameters::ParameterDescriptor, value: f64) -> bool {
    match descriptor.scope {
        colm_case::parameters::ParameterScope::LandCoverClass => {
            colm_case::land_cover::validate_override(&descriptor.raw_key, value).is_ok()
        }
        colm_case::parameters::ParameterScope::PftType
        | colm_case::parameters::ParameterScope::PcPftComponent => {
            colm_case::pft::validate_override(&descriptor.raw_key, value).is_ok()
        }
        _ => true,
    }
}

fn parse_number(value: &str) -> Option<f64> {
    value
        .trim()
        .split('_')
        .next()
        .unwrap_or(value)
        .replace(['d', 'D'], "e")
        .parse()
        .ok()
}

/// 比较两个版本：用同一份设定再建一个研究。返回新研究的目录。
#[tauri::command]
pub async fn study_clone(study_dir: String) -> Result<String, String> {
    if study_dir.contains('\0') || !PathBuf::from(&study_dir).is_absolute() {
        return Err("研究目录须为绝对路径".into());
    }
    Ok(capture_async(vec!["study-clone".into(), study_dir])
        .await?
        .trim()
        .to_owned())
}

/// 比较两个版本：两个研究最优成员的率定期与验证期成绩。
#[tauri::command]
pub async fn study_compare(
    app: tauri::AppHandle,
    a: String,
    b: String,
) -> Result<serde_json::Value, String> {
    for dir in [&a, &b] {
        if dir.contains('\0') || !PathBuf::from(dir).is_absolute() {
            return Err("研究目录须为绝对路径".into());
        }
    }
    let mut event = crate::history::start(
        "study-compare",
        Some(&a),
        serde_json::json!({"action":"study-compare","a":a,"b":b}),
    )?;
    let result = async {
        let left = verified_study_side(&app, &a).await?;
        let right = verified_study_side(&app, &b).await?;
        compare_verified_study_sides(left, right)
    }
    .await;
    if let Ok(value) = &result {
        event.detail["comparison"] = serde_json::json!({"same_design":value["same_design"],"same_inputs":value["same_inputs"],"inputs_verified":value["inputs_verified"],"better":value["better"],"period":value["period"]});
    }
    crate::history::finish(&event, result, "succeeded")
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StudyComparisonSide {
    study: String,
    version: String,
    head: Option<String>,
    status: String,
    best_member: Option<String>,
    calibration: Option<f64>,
    validation: Option<f64>,
    baseline_calibration: Option<f64>,
    baseline_validation: Option<f64>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedStudySide {
    schema: u32,
    side: StudyComparisonSide,
    spec: serde_json::Value,
    input_identity: Option<serde_json::Value>,
    verified: bool,
    notes: Vec<String>,
}

async fn verified_study_side(
    app: &tauri::AppHandle,
    dir: &str,
) -> Result<VerifiedStudySide, String> {
    let canonical =
        std::fs::canonicalize(dir).map_err(|e| format!("cannot open Study for comparison: {e}"))?;
    let dir = canonical.to_str().ok_or("Study path is not UTF-8")?;
    let config = crate::remote::remote_config(app.clone())?;
    let mut recorded_remote = false;
    for file in [".colm-remote-study.json", ".colm-remote-study-pending.json"] {
        match std::fs::symlink_metadata(PathBuf::from(dir).join(file)) {
            Ok(_) => {
                recorded_remote = true;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(format!("cannot inspect Study connection binding: {error}")),
        }
    }
    let server = if recorded_remote {
        Some(crate::remote::study_server(&config, dir)?)
    } else {
        None
    };
    // Separate sidecars keep each captured SSH connection out of the other side's auth state.
    let value = crate::remote::ssh_cli_json(
        vec!["study-verified-side".into(), dir.into()],
        crate::remote::resource_dir(app),
        server.as_ref(),
    )
    .await?;
    let side: VerifiedStudySide =
        serde_json::from_value(value).map_err(|e| format!("invalid verified Study side: {e}"))?;
    if side.schema != 1 {
        return Err("unsupported verified Study side schema".into());
    }
    if !side.spec.is_object() || side.side.study != dir {
        return Err("verified Study side does not match the requested Study".into());
    }
    Ok(side)
}

fn same_study_inputs(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    if a["schema"] != 1 || b["schema"] != 1 {
        return false;
    }
    let (Some(left), Some(right), Some(left_obs), Some(right_obs)) = (
        a["base_cases"].as_object(),
        b["base_cases"].as_object(),
        a["observations"].as_object(),
        b["observations"].as_object(),
    ) else {
        return false;
    };
    let sha = |value: &serde_json::Value| {
        value
            .as_str()
            .is_some_and(|value| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()))
    };
    !left.is_empty()
        && left.len() == right.len()
        && left_obs.len() == right_obs.len()
        && left.iter().all(|(site, a)| {
            right.get(site).is_some_and(|b| {
                a["schema"] == 1
                    && b["schema"] == 1
                    && a["complete"] == true
                    && b["complete"] == true
                    && a["algorithm"] == "sha256-full-v1"
                    && b["algorithm"] == a["algorithm"]
                    && sha(&a["data_sha256"])
                    && a["data_sha256"] == b["data_sha256"]
                    && a["data_settings"].is_object()
                    && a["data_settings"] == b["data_settings"]
                    && a["unresolved"].as_array().is_some_and(Vec::is_empty)
                    && b["unresolved"].as_array().is_some_and(Vec::is_empty)
            })
        })
        && left_obs.iter().all(|(site, a)| {
            right_obs.get(site).is_some_and(|b| {
                sha(&a["sha256"])
                    && a["sha256"] == b["sha256"]
                    && a["bytes"].as_u64().is_some()
                    && a["bytes"] == b["bytes"]
            })
        })
}

fn comparison_design(spec: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut value = spec.clone();
    let map = value
        .as_object_mut()
        .ok_or("Study design must be an object")?;
    map.remove("kernel_dir");
    let observations = map
        .get("observations")
        .and_then(serde_json::Value::as_object)
        .ok_or("Study design has no observation mapping")?;
    let mut sites = observations.keys().cloned().collect::<Vec<_>>();
    sites.sort();
    map.insert("observations".into(), serde_json::json!(sites));
    Ok(value)
}

fn compare_verified_study_sides(
    left: VerifiedStudySide,
    right: VerifiedStudySide,
) -> Result<serde_json::Value, String> {
    if left.schema != 1 || right.schema != 1 {
        return Err("unsupported verified Study side schema".into());
    }
    let same_design = comparison_design(&left.spec)? == comparison_design(&right.spec)?;
    let inputs_verified = left.verified
        && right.verified
        && left
            .input_identity
            .as_ref()
            .zip(right.input_identity.as_ref())
            .is_some_and(|(a, b)| same_study_inputs(a, a) && same_study_inputs(b, b));
    let same_inputs = inputs_verified
        && left
            .input_identity
            .as_ref()
            .zip(right.input_identity.as_ref())
            .is_some_and(|(a, b)| same_study_inputs(a, b));
    let mut notes = left.notes;
    notes.extend(right.notes);
    if !same_design {
        notes.push("the Studies differ in parameters, budget, seed, targets or periods; no version winner is reported".into());
    }
    if !inputs_verified {
        notes.push("full scientific input, observation and output identities could not be verified; no version winner is reported".into());
    } else if !same_inputs {
        notes.push("the Studies use different scientific input or observation contents; no version winner is reported".into());
    }
    if left.side.version == right.side.version {
        notes.push("both Studies use the same model version".into());
    }
    let has_validation = left.side.validation.is_some() && right.side.validation.is_some();
    if !has_validation {
        notes.push("no validation score on both sides; comparison uses the calibration period and does not establish independent validation improvement".into());
    }
    let (a, b) = if has_validation {
        (left.side.validation, right.side.validation)
    } else {
        (left.side.calibration, right.side.calibration)
    };
    let better = match (a, b) {
        (Some(a), Some(b)) if same_design && same_inputs && a.is_finite() && b.is_finite() => {
            Some(if (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0) {
                "tie"
            } else if a < b {
                "a"
            } else {
                "b"
            })
        }
        _ => None,
    };
    if same_design && same_inputs && better.is_none() {
        notes.push("at least one Study has no finite best-member score".into());
    }
    Ok(
        serde_json::json!({"a":left.side,"b":right.side,"same_design":same_design,"same_inputs":same_inputs,"inputs_verified":inputs_verified,"period":if has_validation { "validation" } else { "calibration" },"better":better,"notes":notes}),
    )
}

#[tauri::command]
pub async fn study_create_json(
    case_root: String,
    spec_json: String,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<String, String> {
    capture_study_spec("study-create", case_root, spec_json, model_revision).await
}

#[tauri::command]
pub async fn study_preflight_json(
    case_root: String,
    spec_json: String,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<String, String> {
    capture_study_spec("study-preflight", case_root, spec_json, model_revision).await
}

async fn capture_study_spec(
    command: &str,
    case_root: String,
    spec_json: String,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<String, String> {
    let path = write_temp_study_spec(&spec_json)?;
    let mut args = vec![
        command.to_string(),
        case_root,
        "--spec".into(),
        path.display().to_string(),
    ];
    let result = match crate::model::options(&mut args, model_revision.as_ref()) {
        Ok(()) => capture_async(args).await,
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(path);
    result
}

fn write_temp_study_spec(spec_json: &str) -> Result<PathBuf, String> {
    use std::io::Write;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    for n in 0..100u32 {
        let path = std::env::temp_dir().join(format!(
            "colm-study-{}-{nanos}-{n}.json",
            std::process::id(),
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(spec_json.as_bytes()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(error.to_string());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("cannot allocate a temporary Study spec file".into())
}

#[tauri::command]
pub async fn study_status(study_dir: String) -> Result<String, String> {
    capture_async(vec!["study-status".to_string(), study_dir]).await
}

#[tauri::command]
// Named IPC arguments, as for `run_batch`: a wrapper struct would only add a
// second frontend/backend shape for the same fields.
#[allow(clippy::too_many_arguments)]
pub async fn study_run(
    app: tauri::AppHandle,
    processes: tauri::State<'_, RunProcesses>,
    study_dir: String,
    kernel: String,
    stream: bool,
    jobs: Option<usize>,
    retry_failed: Option<bool>,
    engine: Option<String>,
    model_version: Option<String>,
    model_revision: Option<colm_model::VersionRef>,
) -> Result<String, String> {
    validate_engine(engine.as_deref())?;
    validate_model_version(model_version.as_deref())?;
    let key = study_process_key(&study_dir);
    let mut args = study_run_args(study_dir, kernel, stream, jobs, retry_failed, engine);
    // 研究第一次给修改版时拍快照，之后续跑沿用（`colm-cli study-run --model-version`）；官方版不加参数。
    if let Some(version) = model_version.filter(|v| v != "official") {
        args.extend(["--model-version".into(), version]);
    }
    crate::model::options(&mut args, model_revision.as_ref())?;
    processes.prepare(std::slice::from_ref(&key))?;
    let processes = processes.inner().clone();
    tauri::async_runtime::spawn_blocking(move || study_run_blocking(app, processes, args))
        .await
        .map_err(|e| e.to_string())?
}

fn study_run_args(
    study_dir: String,
    kernel: String,
    _stream: bool,
    jobs: Option<usize>,
    retry_failed: Option<bool>,
    engine: Option<String>,
) -> Vec<String> {
    let mut args = vec!["study-run".to_string(), study_dir];
    if !kernel.trim().is_empty() {
        args.extend(["--kernel".into(), kernel]);
    }
    // Study runs are always streamed: progress is NDJSON and the GUI listens
    // to one `study://event` channel instead of waiting for a giant capture.
    args.extend(["--stream".into(), "1".into()]);
    if let Some(jobs) = jobs {
        args.push("--jobs".into());
        args.push(jobs.to_string());
    }
    if retry_failed.unwrap_or(false) {
        args.push("--retry-failed".into());
        args.push("1".into());
    }
    if let Some(engine) = engine {
        args.extend(["--engine".into(), engine]);
    }
    args
}

fn parse_study_event_line(line: &str) -> Option<serde_json::Value> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(line).ok()
}

fn is_study_task_log(payload: &serde_json::Value) -> bool {
    payload.get("kind").and_then(serde_json::Value::as_str) == Some("task_log")
}

fn should_forward_study_task_log(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_none_or(|previous| now.saturating_duration_since(previous) >= EMIT_INTERVAL) {
        *last = Some(now);
        true
    } else {
        false
    }
}

fn append_study_event(study_dir: &str, payload: &serde_json::Value) {
    let Ok(line) = serde_json::to_string(payload) else {
        return;
    };
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(PathBuf::from(study_dir).join("study.log"))
    {
        let _ = writeln!(log, "{line}");
    }
}

fn study_run_blocking(
    app: tauri::AppHandle,
    processes: RunProcesses,
    args: Vec<String>,
) -> Result<String, String> {
    let mut args = args;
    let event = crate::history::command_start(&args).inspect_err(|_| {
        if let Some(dir) = args.get(1) {
            let _ = processes.forget(&study_process_key(dir));
        }
    })?;
    if let Some(event) = &event {
        args.extend(["--operation-id".into(), event.operation_id.clone()]);
    }
    let result = study_run_untracked(app, processes, args);
    let status = result
        .as_deref()
        .map(study_run_status)
        .unwrap_or("succeeded");
    crate::history::command_finish(event.as_ref(), result, status)
}

/// History status for a study-run that exited normally: the CLI also exits 0 when the Study
/// pauses, finishes with failures or needs review, and says so in its final `study_done`.
fn study_run_status(output: &str) -> &'static str {
    let event: serde_json::Value = serde_json::from_str(output).unwrap_or_default();
    match event["status"].as_str() {
        Some("completed") => "completed",
        Some("completed_with_failures") => "completed_with_failures",
        Some("cancelled") => "cancelled",
        Some("paused") => "paused",
        Some("needs_review") => "needs_review",
        _ => "succeeded",
    }
}
fn study_run_untracked(
    app: tauri::AppHandle,
    processes: RunProcesses,
    args: Vec<String>,
) -> Result<String, String> {
    let cli = resolve_cli();
    let study_dir = args.get(1).cloned().unwrap_or_default();
    let study_key = study_process_key(&study_dir);
    let mut cmd = sidecar_command(&cli);
    let mut child = colm_kernel::run::top_level_sidecar(&mut cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            let _ = processes.forget(&study_key);
            format!("cannot start {}: {e}", cli.display())
        })?;
    remember_process(&processes, &study_key, &mut child)?;
    let (out, err) = take_process_pipes(&mut child, &processes, &study_key)?;
    let errs = drain_stderr(err);
    let mut last = None;
    let mut last_task_log_emit = None;
    for line in BufReader::new(out).lines().map_while(Result::ok) {
        let mut payload = parse_study_event_line(&line)
            .unwrap_or_else(|| serde_json::json!({"type":"log","kind":"log","line":line}));
        if let Some(object) = payload.as_object_mut() {
            object
                .entry("study_dir")
                .or_insert_with(|| serde_json::Value::String(study_dir.clone()));
        }
        // Member logs already live on disk; forward only a heartbeat sample so
        // the GUI has live output without flooding the webview.
        if is_study_task_log(&payload) {
            if should_forward_study_task_log(&mut last_task_log_emit, Instant::now()) {
                let _ = app.emit("study://event", payload);
            }
            continue;
        }
        last = Some(payload.clone());
        let _ = app.emit("study://event", payload);
    }
    let (status, cancelled) = wait_process(&mut child, &processes, &study_key)?;
    let err = errs.join().map_err(|_| "stderr thread panicked")?;
    if cancelled {
        let payload = serde_json::json!({
            "type":"study_cancelled",
            "kind":"study_cancelled",
            "study_dir":study_dir,
            "reason":"运行已取消"
        });
        let _ = app.emit("study://event", payload.clone());
        return Err(serde_json::to_string(&payload).unwrap_or_else(|_| "运行已取消".into()));
    }
    if !status.success() {
        let reason = failure_reason(&err).unwrap_or_else(|| format!("study-run failed: {status}"));
        let payload = serde_json::json!({
            "type":"study_failed",
            "kind":"study_failed",
            "study_dir":study_dir,
            "reason":reason
        });
        append_study_event(&study_dir, &payload);
        let _ = app.emit("study://event", payload.clone());
        return Err(serde_json::to_string(&payload).unwrap_or(reason));
    }
    Ok(last.map(|value| value.to_string()).unwrap_or_else(|| {
        serde_json::json!({"type":"study_done","kind":"study_done"}).to_string()
    }))
}

#[tauri::command]
pub async fn study_pause(study_dir: String) -> Result<String, String> {
    capture_async(vec!["study-pause".to_string(), study_dir]).await
}

#[tauri::command]
pub async fn study_resume(study_dir: String) -> Result<String, String> {
    capture_async(vec!["study-resume".to_string(), study_dir]).await
}

#[tauri::command]
pub async fn study_cancel(
    processes: tauri::State<'_, RunProcesses>,
    study_dir: String,
) -> Result<String, String> {
    let out = capture_async(vec!["study-cancel".to_string(), study_dir.clone()]).await?;
    let key = study_process_key(&study_dir);
    let pid = processes.running_pid(&key)?;
    processes.cancel(Some(vec![key]))?;
    if let Some(pid) = pid {
        capture_async(vec![
            "study-finalize-cancel".to_string(),
            study_dir,
            "--pid".into(),
            pid.to_string(),
        ])
        .await?;
    } else {
        capture_async(vec!["study-finalize-idle-cancel".to_string(), study_dir]).await?;
    }
    Ok(out)
}

#[tauri::command]
pub async fn study_retry(
    study_dir: String,
    include_review: Option<bool>,
) -> Result<String, String> {
    let mut args = vec!["study-retry".to_string(), study_dir];
    if include_review.unwrap_or(false) {
        args.push("--include-review".into());
        args.push("1".into());
    }
    capture_async(args).await
}

#[tauri::command]
pub async fn study_export(study_dir: String, out: String) -> Result<String, String> {
    capture_async(vec![
        "study-export".to_string(),
        study_dir,
        "--out".into(),
        out,
    ])
    .await
}

#[tauri::command]
pub async fn study_apply(
    study_dir: String,
    member: String,
    out: String,
    name: Option<String>,
) -> Result<String, String> {
    let mut args = study_apply_args(study_dir, member, out);
    if let Some(name) = name {
        args.push("--name".into());
        args.push(name);
    }
    capture_async(args).await
}

fn study_apply_args(study_dir: String, member: String, out: String) -> Vec<String> {
    vec![
        "study-apply".to_string(),
        study_dir,
        "--member".into(),
        member,
        "--out".into(),
        out,
    ]
}

#[tauri::command]
pub async fn study_apply_preview(study_dir: String, member: String) -> Result<String, String> {
    capture_async(study_apply_preview_args(study_dir, member)).await
}

fn study_apply_preview_args(study_dir: String, member: String) -> Vec<String> {
    vec![
        "study-apply-preview".to_string(),
        study_dir,
        "--member".into(),
        member,
    ]
}

#[tauri::command]
pub async fn study_result(study_dir: String, path: String) -> Result<String, String> {
    capture_async(vec![
        "study-result".to_string(),
        study_dir,
        "--path".into(),
        path,
    ])
    .await
}

/// 算例的 AI 参数化（混合模型）配置，JSON；没有配置时 `installed` 为 false。
#[tauri::command]
pub async fn hybrid_info(case: String) -> Result<String, String> {
    capture_async(vec!["hybrid-info".to_string(), case]).await
}

/// 在算例上空跑模型（不模拟），JSON：各插槽的行数与特征、输出的范围。
#[tauri::command]
pub async fn hybrid_check(
    case: String,
    kernel_dir: String,
    model_revision: Option<colm_model::VersionRef>,
    preprocessors: Option<String>,
) -> Result<String, String> {
    capture_async(hybrid_check_args(
        case,
        kernel_dir,
        model_revision.as_ref(),
        preprocessors.as_deref(),
    )?)
    .await
}

fn hybrid_check_args(
    case: String,
    kernel_dir: String,
    model_revision: Option<&colm_model::VersionRef>,
    preprocessors: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut args = vec![
        if model_revision.is_some() {
            "model-hybrid-check"
        } else {
            "hybrid-check"
        }
        .to_string(),
        case,
        "--kernel".into(),
        kernel_dir,
    ];
    if model_revision.is_some() {
        crate::model::options(&mut args, model_revision)?;
        if let Some(value) = preprocessors {
            if !matches!(value, "rust" | "fortran") {
                return Err("preprocessors must be rust or fortran".into());
            }
            args.extend(["--preprocessors".into(), value.into()]);
        }
    }
    Ok(args)
}

/// 把同一个模型装到每个算例上。参数名用 `dirs`：`ipc.js` 据此拦下正在运行的算例。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn hybrid_install(
    dirs: Vec<String>,
    model: String,
    slot: String,
    features: Vec<String>,
    outputs: Vec<String>,
    normalize: Option<String>,
    force: bool,
    outside_physics: bool,
) -> Result<String, String> {
    let mut log = String::new();
    for dir in dirs {
        let mut args = hybrid_install_args(
            dir.clone(),
            model.clone(),
            slot.clone(),
            &features,
            &outputs,
            normalize.clone(),
            force,
        )?;
        if outside_physics {
            args.extend(["--outside".to_string(), "physics".to_string()]);
        }
        log += &capture_async(args)
            .await
            .map_err(|e| format!("{dir}: {e}"))?;
    }
    Ok(log)
}

fn hybrid_install_args(
    case: String,
    model: String,
    slot: String,
    features: &[String],
    outputs: &[String],
    normalize: Option<String>,
    force: bool,
) -> Result<Vec<String>, String> {
    let features: Vec<&str> = features
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .collect();
    if features.is_empty() {
        return Err("至少要一个输入特征".into());
    }
    if outputs.is_empty() {
        return Err("至少要一个输出参数".into());
    }
    let mut args = vec![
        "hybrid-install".to_string(),
        case,
        "--model".into(),
        model,
        "--slot".into(),
        slot,
        "--features".into(),
        features.join(","),
    ];
    for output in outputs {
        args.push("--output".into());
        args.push(output.clone());
    }
    if let Some(normalize) = normalize.filter(|n| !n.trim().is_empty()) {
        args.push("--normalize".into());
        args.push(normalize);
    }
    if force {
        args.push("--force".into());
        args.push("1".into());
    }
    Ok(args)
}

/// 给每个算例算气候特征（`hybrid_climate/`），供 `clim_*` 特征使用。写算例目录，所以参数名用 `dirs`。
#[tauri::command]
pub async fn hybrid_climate(dirs: Vec<String>, kernel_dir: String) -> Result<String, String> {
    let mut log = String::new();
    for dir in dirs {
        log += &capture_async(hybrid_climate_args(dir.clone(), kernel_dir.clone()))
            .await
            .map_err(|e| format!("{dir}: {e}"))?;
    }
    Ok(log)
}

fn hybrid_climate_args(case: String, kernel_dir: String) -> Vec<String> {
    vec![
        "hybrid-climate".to_string(),
        case,
        "--kernel".into(),
        kernel_dir,
    ]
}

/// 删掉每个算例的 `hybrid.toml` 及它引用的模型文件。
#[tauri::command]
pub async fn hybrid_remove(dirs: Vec<String>) -> Result<String, String> {
    let mut log = String::new();
    for dir in dirs {
        log += &capture_async(vec!["hybrid-remove".to_string(), dir.clone()])
            .await
            .map_err(|e| format!("{dir}: {e}"))?;
    }
    Ok(log)
}

/// 项目里的调优 Study（两步法的第一步），JSON 数组。
#[tauri::command]
pub async fn hybrid_studies(root: String) -> Result<String, String> {
    capture_async(vec!["hybrid-studies".to_string(), root]).await
}

/// 两步法的拟合选项：线性网络用岭回归闭式解（`ridge`），有隐藏层时用 Adam（`epochs`、`learning_rate`）。
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct FitRequest {
    pub studies: Vec<String>,
    /// 与 Study 的 `hybrid` 段同格式。
    pub network: serde_json::Value,
    pub kernel_dir: String,
    /// 输出目录与名字：写出 `<名字>.mlp.json`、`.norm.json`、`.fit.json` 与 `.network.json`。
    pub out_dir: String,
    pub name: String,
    #[serde(default)]
    pub ridge: Option<f64>,
    #[serde(default)]
    pub epochs: Option<u32>,
    #[serde(default)]
    pub learning_rate: Option<f64>,
    #[serde(default)]
    pub weight: Option<String>,
}

pub(crate) fn hybrid_fit_args(
    request: &FitRequest,
    network_file: &str,
    model: &str,
) -> Result<Vec<String>, String> {
    if request.studies.is_empty() {
        return Err("至少选一个调优 Study".into());
    }
    if request.name.is_empty()
        || !request
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("模型名只能用字母、数字、- 和 _".into());
    }
    let mut args = vec![
        "hybrid-fit".to_string(),
        "--studies".into(),
        request.studies.join(","),
        "--network".into(),
        network_file.to_owned(),
        "--kernel".into(),
        request.kernel_dir.clone(),
        "--out".into(),
        model.to_owned(),
    ];
    if let Some(ridge) = request.ridge {
        args.extend(["--ridge".into(), ridge.to_string()]);
    }
    if let Some(epochs) = request.epochs {
        args.extend(["--epochs".into(), epochs.to_string()]);
    }
    if let Some(rate) = request.learning_rate {
        args.extend(["--learning-rate".into(), rate.to_string()]);
    }
    if let Some(weight) = request.weight.as_ref().filter(|w| !w.is_empty()) {
        args.extend(["--weight".into(), weight.clone()]);
    }
    Ok(args)
}

/// 两步法：在选中的调优 Study 上拟合“特征 → 参数”网络。返回 `.fit.json` 报告（含留一 Study 交叉验证与
/// 均值基准）和三个文件的路径；不过门槛也照样写出，由界面决定能不能装到算例。
#[tauri::command]
pub async fn hybrid_fit(request: FitRequest) -> Result<serde_json::Value, String> {
    let dir = std::path::PathBuf::from(&request.out_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem = dir.join(&request.name);
    let network_file = format!("{}.network.json", stem.display());
    let model = format!("{}.mlp.json", stem.display());
    let args = hybrid_fit_args(&request, &network_file, &model)?;
    std::fs::write(
        &network_file,
        serde_json::to_string_pretty(&request.network).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{network_file}: {e}"))?;
    capture_async(args).await?;
    let report_file = format!("{}.fit.json", stem.display());
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&report_file).map_err(|e| format!("{report_file}: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "report": report,
        "model": model,
        "normalize": format!("{}.norm.json", stem.display()),
        "network": network_file,
        "report_file": report_file,
    }))
}

pub(crate) async fn capture_async(args: Vec<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || capture(&args))
        .await
        .map_err(|e| e.to_string())?
}

/// 跑一次 sidecar，把 stdout 整个收回来。
///
/// 用于短命令（`new` / `series`）；跑模型那种长命令走 `run_case` 的流式路径。
pub(crate) fn capture(args: &[String]) -> Result<String, String> {
    capture_with(args, capture_untracked)
}

fn capture_with(
    args: &[String],
    execute: impl FnOnce(&[String]) -> Result<String, String>,
) -> Result<String, String> {
    let mut args = args.to_vec();
    if !args.iter().any(|a| a == "--history-db") {
        crate::model::options(&mut args, None)?;
    }
    let mut event = crate::history::command_start(&args)?;
    if let Some(event) = &event {
        args.extend(["--operation-id".into(), event.operation_id.clone()]);
    }
    let result = execute(&args);
    let status = crate::history::capture_audit_status(event.as_mut(), &result);
    crate::history::command_finish(event.as_ref(), result, status)
}
fn capture_untracked(args: &[String]) -> Result<String, String> {
    let cli = resolve_cli();
    let mut cmd = sidecar_command(&cli);
    let out = cmd
        .args(args)
        .output()
        .map_err(|e| format!("cannot start {}: {e}", cli.display()))?;
    if !out.status.success() {
        // sidecar 的错误信息在 stderr，原样交给用户 —— 它比我们能编的更具体
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// Template generation owns this path; stale advanced fields must not replace it.
pub(crate) fn preserve_generated_forcing(
    dataset: Option<&str>,
    fields: &mut Vec<crate::config::FieldChange>,
) {
    if dataset.is_some_and(|value| !value.trim().is_empty()) {
        fields.retain(|field| {
            !field
                .path
                .trim()
                .eq_ignore_ascii_case("DEF_forcing_namelist")
        });
    }
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod sidecar_tests;

/// Preserve the CLI's default while rejecting misspelled explicit selections.
pub(crate) fn add_preprocessors(args: &mut Vec<String>, value: Option<&str>) -> Result<(), String> {
    match value {
        None => Ok(()),
        Some("rust" | "fortran") => {
            args.extend(["--preprocessors".into(), value.unwrap().into()]);
            Ok(())
        }
        Some(_) => Err("preprocessors must be rust or fortran".into()),
    }
}

#[cfg(test)]
mod deletion_guard_tests {
    use super::*;

    #[test]
    fn deletion_requires_no_pending_or_running_process() {
        let processes = RunProcesses::default();
        assert!(processes.with_idle(|| Ok(())).is_ok());
        processes.prepare(&["case".into()]).unwrap();
        assert!(processes.with_idle(|| Ok(())).is_err());
        processes.remember("case", 123).unwrap();
        assert!(processes.with_idle(|| Ok(())).is_err());
        processes.forget("case").unwrap();
        assert!(processes.with_idle(|| Ok(())).is_ok());
    }
}
