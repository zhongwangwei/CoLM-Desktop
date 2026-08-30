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

#[derive(Default)]
struct RunProcessState {
    pids: HashMap<String, u32>,
    pending: HashSet<String>,
    cancelled: HashSet<String>,
}

impl RunProcesses {
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
        state.pending.remove(key);
        if state.cancelled.contains(key) {
            return Ok(false);
        }
        if state.pids.contains_key(key) {
            return Err(format!("{key} is already running"));
        }
        state.pids.insert(key.to_string(), pid);
        Ok(true)
    }

    fn take_cancelled(&self, key: &str) -> Result<bool, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "run process registry poisoned")?;
        state.pending.remove(key);
        Ok(state.cancelled.remove(key))
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

/// `TIMESTEP = 1 | DATE = 2008-01-01-00000` -> 步数与日期。
///
/// 预热期间 CoLM 用另一种 format，行尾多一段 `Spinup (cycle 1 of 3)`
/// （`CoLM.F90:747` 与 `:749` 是两条不同的 format 语句）。两种都要认，
/// 否则那段尾巴会整个留在 `date` 里 —— 不崩，但界面分不出正在预热。
///
/// `case` 与 `stage` 由调用方补：这个函数只认得一行文本。
struct Step {
    step: u64,
    date: String,
    spinup: Option<(u32, u32)>,
}

fn parse_progress(line: &str) -> Option<Step> {
    let rest = line.trim().strip_prefix("TIMESTEP =")?;
    let (step, date) = rest.split_once('|')?;
    let step: u64 = step.trim().parse().ok()?;
    let rest = date.trim().strip_prefix("DATE =")?.trim();
    // 日期本身不含空格，所以第一个空格就是它的边界。
    let (date, tail) = match rest.split_once(' ') {
        Some((d, t)) => (d, t.trim()),
        None => (rest, ""),
    };
    let spinup = tail
        .strip_prefix("Spinup (cycle ")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.split_once(" of "))
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
    Some(Step {
        step,
        date: date.to_string(),
        spinup,
    })
}

/// `=== colm-stage mksrfdata begin ===` -> `("mksrfdata", "begin")`。
///
/// 标记由 `colm-cli run --stream` 自己打。实测 CoLM 的 34180 行输出里
/// 没有一行以 `===` 开头，也没有一处出现 `colm-stage`，所以不会撞。
fn parse_stage(line: &str) -> Option<(String, String)> {
    let s = line
        .trim()
        .strip_prefix("=== colm-stage ")?
        .strip_suffix(" ===")?;
    let (name, state) = s.rsplit_once(' ')?;
    Some((name.trim().to_string(), state.trim().to_string()))
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

fn validate_run_id(run_id: &str) -> Result<(), String> {
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
) -> Result<Vec<String>, String> {
    validate_run_stage(stage)?;
    let mut args = vec![
        "run".to_string(),
        case.to_string(),
        "--kernel".to_string(),
        kernel.to_string(),
        "--stream".to_string(),
        "1".to_string(),
    ];
    if let Some(stage) = stage {
        args.extend(["--stage".to_string(), stage.to_string()]);
    }
    if force {
        args.extend(["--force".to_string(), "1".to_string()]);
    }
    Ok(args)
}

/// 跑一个算例。返回子进程的退出码。
#[tauri::command]
pub async fn run_case(
    app: tauri::AppHandle,
    processes: tauri::State<'_, RunProcesses>,
    run_id: String,
    case: String,
    kernel: String,
    force: bool,
    stage: Option<String>,
) -> Result<i32, String> {
    validate_run_id(&run_id)?;
    let processes = processes.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_case_blocking(app, processes, run_id, case, kernel, force, stage)
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
/// 收在内存里而不是边收边发：stderr 上的东西几乎只有最后那句致命错误，
/// 与 stdout 交错发出去只会让日志窗的顺序变得不可复现。
fn drain_stderr(err: std::process::ChildStderr) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        BufReader::new(err)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .collect()
    })
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
        let (mut total, mut dropped) = (0usize, 0usize);
        let mut last_progress = Instant::now() - EMIT_INTERVAL;
        let mut last_lines = Instant::now() - EMIT_INTERVAL;
        let mut buf: VecDeque<String> = VecDeque::with_capacity(LOG_CAPACITY);
        let mut pending: Vec<String> = Vec::new();
        // 当前在哪一段。三段串行，所以一个变量就够。
        let mut stage = String::from("mksrfdata");
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            total += 1;
            // 阶段标记先认：它不进日志窗，也不该被当成噪声丢掉。
            if let Some((name, state)) = parse_stage(&line) {
                dropped += 1;
                stage = name.clone();
                let _ = h.emit(
                    "run://stage",
                    StageMark {
                        run_id: run_id.clone(),
                        case: case_id.clone(),
                        stage: name,
                        state,
                    },
                );
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
                    let _ = h.emit(
                        "run://progress",
                        Progress {
                            run_id: run_id.clone(),
                            case: case_id.clone(),
                            stage: stage.clone(),
                            step: s.step,
                            total_steps,
                            date: s.date,
                            spinup: s.spinup,
                        },
                    );
                }
                continue;
            }
            if buf.len() == LOG_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(line.clone());
            pending.push(line);
            if last_lines.elapsed() >= EMIT_INTERVAL {
                last_lines = Instant::now();
                let _ = h.emit(
                    "run://lines",
                    Lines {
                        run_id: run_id.clone(),
                        case: case_id.clone(),
                        lines: std::mem::take(&mut pending),
                    },
                );
            }
        }
        if !pending.is_empty() {
            let _ = h.emit(
                "run://lines",
                Lines {
                    run_id,
                    case: case_id.clone(),
                    lines: pending,
                },
            );
        }
        (total, dropped, buf)
    })
}

/// 同时最多跑几个算例。
///
fn batch_width(requested: usize, available: usize) -> usize {
    requested.clamp(1, available.max(1))
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
) -> Result<BatchSummary, String> {
    validate_run_stage(stage.as_deref())?;
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let total = cases.len();
    processes.prepare(&cases)?;
    let width = batch_width(max_concurrent, available).min(total.max(1));
    let queue = Arc::new(Mutex::new(VecDeque::from(cases)));
    let succeeded = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::with_capacity(width);
    for _ in 0..width {
        let (a, r, k, q, ok, requested_stage, p) = (
            app.clone(),
            run_id.clone(),
            kernel.clone(),
            Arc::clone(&queue),
            Arc::clone(&succeeded),
            stage.clone(),
            processes.clone(),
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
                run_one(&a, &p, &r, &case, &k, force, requested_stage.as_deref())
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

fn run_case_blocking(
    app: tauri::AppHandle,
    processes: RunProcesses,
    run_id: String,
    case: String,
    kernel: String,
    force: bool,
    stage: Option<String>,
) -> Result<i32, String> {
    processes.prepare(std::slice::from_ref(&case))?;
    run_one(
        &app,
        &processes,
        &run_id,
        &case,
        &kernel,
        force,
        stage.as_deref(),
    )
}

/// `run_case` 里除去日志缓冲区那部分的逻辑，批量与单算例共用。
fn run_one(
    app: &tauri::AppHandle,
    processes: &RunProcesses,
    run_id: &str,
    case: &str,
    kernel: &str,
    force: bool,
    stage: Option<&str>,
) -> Result<i32, String> {
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
    let cli = resolve_cli();
    let mut cmd = sidecar_command(&cli);
    let args = run_args(case, kernel, force, stage)?;
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
    let reader = pump(app, run_id, case, out);
    let errs = drain_stderr(err_pipe);
    let (status, cancelled) = wait_process(&mut child, processes, case)?;
    let (total, dropped, _) = reader.join().map_err(|_| "reader thread panicked")?;
    let err = errs.join().map_err(|_| "stderr thread panicked")?;
    let code = status.code().unwrap_or(-1);
    if !err.is_empty() {
        let _ = app.emit(
            "run://lines",
            Lines {
                run_id: run_id.to_string(),
                case: case.to_string(),
                lines: err.clone(),
            },
        );
    }
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
) -> Result<String, String> {
    validate_bgc_runtime(runtime.as_deref(), &fields)?;
    let case_dir = out.clone();
    let case_existed = PathBuf::from(&case_dir).exists();
    let mut args = vec![
        "new".to_string(),
        "--site".into(),
        site,
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
    let output = match capture_async(args.clone()).await {
        Ok(output) => output,
        Err(error) => {
            // `colm-cli new` creates the case directory before it validates the
            // complete site/forcing contract. A failed preflight must not leave a
            // ghost case that later appears runnable in the GUI.
            if !case_existed {
                let _ = std::fs::remove_dir_all(&case_dir);
            }
            return Err(error);
        }
    };
    if let Err(error) = crate::config::apply_fields(&case_dir, &fields) {
        if !case_existed {
            let _ = std::fs::remove_dir_all(&case_dir);
        }
        return Err(error);
    }
    Ok(output)
}

fn is_crop_case(fields: &[crate::config::FieldChange]) -> bool {
    fields
        .iter()
        .any(|field| field.path == "DEF_TUNING_CROP_PLANTING_DAY")
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
    case: String,
    vars: String,
    from: Option<i64>,
    to: Option<i64>,
    max_points: Option<usize>,
) -> Result<String, String> {
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

/// 轻量结果索引：变量、单位、维度与时间覆盖。数值仍由 `series` 按需读取。
#[tauri::command]
pub async fn history_catalog(case: String) -> Result<String, String> {
    capture_async(vec!["history-catalog".to_string(), case]).await
}

#[tauri::command]
pub async fn study_params() -> Result<String, String> {
    capture_async(vec!["study-params".to_string()]).await
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
) -> Result<Vec<StudyParameterContextRow>, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
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
        && lct
            .iter()
            .all(|context| context.scheme == lct[0].scheme && context.class_index == lct[0].class_index)
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
            let states = crate::config::pft_parameter_states(
                dirs.clone(),
                pft_type,
                kernel_dir.clone(),
            )?;
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

fn study_row(
    descriptor: &colm_case::parameters::ParameterDescriptor,
    scope: &'static str,
    scope_instance: crate::config::ParameterScopeInstance,
    default: f64,
) -> Result<StudyParameterContextRow, String> {
    let tuning = colm_case::tuning::find(&descriptor.raw_key)
        .map_err(|error| format!("{error:#}"))?;
    let pft = colm_case::pft::parameter(&descriptor.raw_key);
    let lc = colm_case::land_cover::parameter(&descriptor.raw_key);
    let min = tuning.and_then(|meta| meta.min.map(|bound| bound.value))
        .or_else(|| pft.and_then(|meta| meta.min))
        .or_else(|| lc.and_then(|meta| meta.min));
    let max = tuning.and_then(|meta| meta.max.map(|bound| bound.value))
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

#[tauri::command]
pub async fn study_create_json(case_root: String, spec_json: String) -> Result<String, String> {
    capture_study_spec("study-create", case_root, spec_json).await
}

#[tauri::command]
pub async fn study_preflight_json(case_root: String, spec_json: String) -> Result<String, String> {
    capture_study_spec("study-preflight", case_root, spec_json).await
}

async fn capture_study_spec(
    command: &str,
    case_root: String,
    spec_json: String,
) -> Result<String, String> {
    let path = write_temp_study_spec(&spec_json)?;
    let result = capture_async(vec![
        command.to_string(),
        case_root,
        "--spec".into(),
        path.display().to_string(),
    ])
    .await;
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
pub async fn study_run(
    app: tauri::AppHandle,
    processes: tauri::State<'_, RunProcesses>,
    study_dir: String,
    kernel: String,
    stream: bool,
    jobs: Option<usize>,
    retry_failed: Option<bool>,
) -> Result<String, String> {
    let key = study_process_key(&study_dir);
    processes.prepare(std::slice::from_ref(&key))?;
    let args = study_run_args(study_dir, kernel, stream, jobs, retry_failed);
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

pub(crate) async fn capture_async(args: Vec<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || capture(&args))
        .await
        .map_err(|e| e.to_string())?
}

/// 跑一次 sidecar，把 stdout 整个收回来。
///
/// 用于短命令（`new` / `series`）；跑模型那种长命令走 `run_case` 的流式路径。
pub(crate) fn capture(args: &[String]) -> Result<String, String> {
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

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod sidecar_tests;
