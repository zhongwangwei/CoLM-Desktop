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

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Manager};

/// 环形缓冲区的上限。够看清一次失败的来龙去脉，又不会把内存吃掉。
const LOG_CAPACITY: usize = 4_000;

/// 事件最快多久发一次。进度与日志各自受这个上界约束，于是无论子进程
/// 打得多快，webview 每秒最多收到约 20 个事件。
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct RunLog {
    lines: Mutex<VecDeque<String>>,
}

#[derive(Serialize, Clone)]
struct Progress {
    step: u64,
    /// CoLM 打印的 `YYYY-MM-DD-SSSSS`，原样传，不解释
    date: String,
}

#[derive(Serialize, Clone)]
struct Done {
    code: i32,
    /// 子进程一共打了多少行
    total: usize,
    /// 其中多少行被当作 RangeCheck 丢掉了
    dropped: usize,
}

/// `TIMESTEP = 1 | DATE = 2008-01-01-00000` -> `(1, "2008-01-01-00000")`。
fn parse_progress(line: &str) -> Option<Progress> {
    let rest = line.trim().strip_prefix("TIMESTEP =")?;
    let (step, date) = rest.split_once('|')?;
    Some(Progress {
        step: step.trim().parse().ok()?,
        date: date.trim().strip_prefix("DATE =")?.trim().to_string(),
    })
}

/// RangeCheck 的逐变量播报。占实测日志的 85%，且无信息 ——
/// 真出问题时它会在同一行尾部追加 ` with NAN` 或 ` Out of Range!`，
/// 而那两句是 `colm-kernel` 的失败标记，运行会被判失败。
fn is_rangecheck_noise(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("Check vector data:") && !t.contains("NAN") && !t.contains("Out of Range")
}

/// 找 `colm-cli`。顺序照 EarthMesh 的 `resolve_mkgrd`：
/// 环境变量 → 应用自身目录（打包进去的 sidecar 在那儿）→ 仓库构建产物 → PATH。
pub fn resolve_cli(app: &tauri::AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("COLM_CLI") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return p;
        }
    }
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join(exe_name());
        if p.is_file() {
            return p;
        }
    }
    for rel in ["../../target/debug", "../../target/release"] {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(rel)
            .join(exe_name());
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe_name()) // 交给 PATH
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "colm-cli.exe"
    } else {
        "colm-cli"
    }
}

/// 跑一个算例。返回子进程的退出码。
#[tauri::command]
pub async fn run_case(
    app: tauri::AppHandle,
    log: tauri::State<'_, RunLog>,
    case: String,
    kernel: String,
) -> Result<i32, String> {
    let cli = resolve_cli(&app);
    let mut child = std::process::Command::new(&cli)
        .args(["run", &case, "--kernel", &kernel])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", cli.display()))?;
    let out = child.stdout.take().ok_or("no stdout")?;

    log.lines.lock().map_err(|_| "log poisoned")?.clear();
    let h = app.clone();
    let reader = std::thread::spawn(move || {
        let (mut total, mut dropped) = (0usize, 0usize);
        let mut last_progress = Instant::now() - EMIT_INTERVAL;
        let mut last_lines = Instant::now() - EMIT_INTERVAL;
        let mut buf: VecDeque<String> = VecDeque::with_capacity(LOG_CAPACITY);
        let mut pending: Vec<String> = Vec::new();
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            total += 1;
            if is_rangecheck_noise(&line) {
                dropped += 1;
                continue;
            }
            // 空行不承载信息，却占掉环形缓冲区一半的容量（实测 2644/5330）。
            if line.trim().is_empty() {
                dropped += 1;
                continue;
            }
            if let Some(p) = parse_progress(&line) {
                if last_progress.elapsed() >= EMIT_INTERVAL {
                    last_progress = Instant::now();
                    let _ = h.emit("run://progress", p);
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
                let _ = h.emit("run://lines", std::mem::take(&mut pending));
            }
        }
        if !pending.is_empty() {
            let _ = h.emit("run://lines", pending);
        }
        (total, dropped, buf)
    });

    let status = child.wait().map_err(|e| e.to_string())?;
    let (total, dropped, buf) = reader.join().map_err(|_| "reader thread panicked")?;
    *log.lines.lock().map_err(|_| "log poisoned")? = buf;
    let code = status.code().unwrap_or(-1);
    let _ = app.emit(
        "run://done",
        Done {
            code,
            total,
            dropped,
        },
    );
    Ok(code)
}

/// 新建一个算例。sidecar 会补齐站点文件、生成强迫场与算例 namelist。
///
/// 经纬度、地类、时间步长、默认窗口都由 `colm-cli new` 从文件里读出来 ——
/// 界面上只需要问「选哪个站」「叫什么名字」，以及可选的窗口收窄。
#[tauri::command]
pub async fn new_case(
    app: tauri::AppHandle,
    site: String,
    out: String,
    name: Option<String>,
    start: Option<String>,
    end: Option<String>,
) -> Result<String, String> {
    let mut args = vec![
        "new".to_string(),
        "--site".into(),
        site,
        "--out".into(),
        out,
    ];
    for (flag, v) in [("--name", name), ("--start", start), ("--end", end)] {
        if let Some(v) = v {
            if !v.trim().is_empty() {
                args.push(flag.into());
                args.push(v);
            }
        }
    }
    capture(&app, &args)
}

/// 取绘图数据。
///
/// 走 sidecar 而不是在这里读 —— 窗口进程不链接 netcdf，
/// 不该为了画一条曲线把整个静态 HDF5 拖进来。
#[tauri::command]
pub async fn series(app: tauri::AppHandle, case: String, vars: String) -> Result<String, String> {
    capture(&app, &["series".to_string(), case, "--vars".into(), vars])
}

/// 跑一次 sidecar，把 stdout 整个收回来。
///
/// 用于短命令（`new` / `series`）；跑模型那种长命令走 `run_case` 的流式路径。
fn capture(app: &tauri::AppHandle, args: &[String]) -> Result<String, String> {
    let cli = resolve_cli(app);
    let out = std::process::Command::new(&cli)
        .args(args)
        .output()
        .map_err(|e| format!("cannot start {}: {e}", cli.display()))?;
    if !out.status.success() {
        // sidecar 的错误信息在 stderr，原样交给用户 —— 它比我们能编的更具体
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 环形缓冲区里的最后 `n` 行。
#[tauri::command]
pub fn run_log_tail(log: tauri::State<'_, RunLog>, n: usize) -> Result<Vec<String>, String> {
    let l = log.lines.lock().map_err(|_| "log poisoned")?;
    Ok(l.iter().rev().take(n).rev().cloned().collect())
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod sidecar_tests;
