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

/// 三个事件都带 `case`。**批量跑的时候这是唯一能分辨来源的东西** ——
/// 事件是全局广播的，90 个站点同时在跑时前端收到的是一锅粥。
///
/// 加字段而不是改事件名：现有三个 `listen` 是 `xtask check-gui`
/// 静态守着的接口，加字段不破坏它，改名会。
#[derive(Serialize, Clone)]
struct Progress {
    /// 算例目录，唯一标识
    case: String,
    /// `mksrfdata` / `mkinidata` / `colm`。来自 `colm-cli` 自己打的阶段标记，
    /// **不是**从 CoLM 的输出措辞里猜的。
    stage: String,
    step: u64,
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
    case: String,
    stage: String,
    /// `begin` / `ok` / `failed`
    state: String,
}

#[derive(Serialize, Clone)]
struct Lines {
    case: String,
    lines: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Done {
    case: String,
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

/// 找 `colm-cli`。顺序照 EarthMesh 的 `resolve_mkgrd`：
/// 环境变量 → 自己旁边（打包进去的 sidecar 在那儿）→ 仓库构建产物 → PATH。
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
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        let p = dir.join(exe_name());
        if p.is_file() {
            return p;
        }
    }
    // 开发构建走这条：`cargo tauri dev` 的可执行文件在
    // `gui/src-tauri/target/debug/`，而 sidecar 还在 `binaries/` 里没搬过去。
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
    pub colm_git_sha: String,
    pub platform: String,
}

/// 列出能用的物理预设。
///
/// 顺序：环境变量 → 随程序打包的那份 → 仓库构建产物。第二条是让
/// 「用户什么都不用装」成立的那一条 —— Fortran 内核是构建产物，
/// 用户不该为了跑一个站点去装 gfortran 与 netcdf-fortran。
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
    if let Ok(d) = app.path().resource_dir() {
        roots.push(d.join("kernels"));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../kernels"));

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
    force: bool,
) -> Result<i32, String> {
    let cli = resolve_cli();
    let mut child = std::process::Command::new(&cli)
        // `--stream` 不是可选的润色：不加的话 `colm-cli run` 只在每段跑完
        // 之后打一句摘要，一次真实运行总共 39 行，而且全在结束时到达 ——
        // 下面这整套「解析 TIMESTEP、限流、批量发送」就没有输入可处理，
        // 进度条从 0 直接跳到 100，日志窗在运行期间一片空白。
        // 实测同一次城市算例：不加 39 行，加了 34180 行（含 528 条 TIMESTEP）。
        .args(["run", &case, "--kernel", &kernel, "--stream", "1"])
        .args(if force {
            &["--force", "1"][..]
        } else {
            &[][..]
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", cli.display()))?;
    let out = child.stdout.take().ok_or("no stdout")?;
    let err = child.stderr.take().ok_or("no stderr")?;

    log.lines.lock().map_err(|_| "log poisoned")?.clear();

    let reader = pump(&app, &case, out);
    let errs = drain_stderr(err);

    let status = child.wait().map_err(|e| e.to_string())?;
    let (total, dropped, mut buf) = reader.join().map_err(|_| "reader thread panicked")?;
    let err = errs.join().map_err(|_| "stderr thread panicked")?;
    let code = status.code().unwrap_or(-1);
    // stderr 也进日志窗。失败时它是**唯一**说清楚原因的东西，
    // 而用户在界面上能拿到的就只有这一窗。
    if !err.is_empty() {
        let _ = app.emit(
            "run://lines",
            Lines {
                case: case.clone(),
                lines: err.clone(),
            },
        );
        buf.extend(err.iter().cloned());
    }
    *log.lines.lock().map_err(|_| "log poisoned")? = buf;
    let _ = app.emit(
        "run://done",
        Done {
            case,
            code,
            total,
            dropped,
            reason: (code != 0).then(|| failure_reason(&err)).flatten(),
        },
    );
    Ok(code)
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
    case: &str,
    out: std::process::ChildStdout,
) -> std::thread::JoinHandle<(usize, usize, VecDeque<String>)> {
    let h = app.clone();
    let case_id = case.to_string();
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
                            case: case_id.clone(),
                            stage: stage.clone(),
                            step: s.step,
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
/// **这个数没被测过。** 每个子进程都读同一份 rawdata、写各自的输出，
/// 瓶颈大概率在磁盘而不是 CPU，但没量过。要调之前先量 ——
/// 别把猜的数留成看起来经过调优的样子。
const MAX_CONCURRENT: usize = 2;

/// 排队跑一批算例。**返回时批次还没跑完** —— 进度全靠事件，
/// 每条都带 `case`，前端据此更新对应那一行。
///
/// 一个算例失败**不中止整批**：90 个站点里有一个跑不通，其余 89 个仍要跑完。
/// 失败信息随那个算例自己的 `run://done`（`code != 0`）到达。
#[tauri::command]
pub async fn run_batch(
    app: tauri::AppHandle,
    log: tauri::State<'_, RunLog>,
    cases: Vec<String>,
    kernel: String,
) -> Result<usize, String> {
    let mut done = 0usize;
    for chunk in cases.chunks(MAX_CONCURRENT) {
        let mut handles = Vec::new();
        for case in chunk {
            let (a, c, k) = (app.clone(), case.clone(), kernel.clone());
            handles.push(std::thread::spawn(move || run_one(&a, &c, &k)));
        }
        for h in handles {
            // 线程 panic 也不该让整批停下 —— 那正是「一个坏算例毁掉一批」。
            if h.join().is_ok() {
                done += 1;
            }
        }
    }
    let _ = log; // 环形缓冲区只服务单算例视图，批量时不共享
    Ok(done)
}

/// `run_case` 里除去日志缓冲区那部分的逻辑，批量与单算例共用。
fn run_one(app: &tauri::AppHandle, case: &str, kernel: &str) -> Result<i32, String> {
    let cli = resolve_cli();
    let out = std::process::Command::new(&cli)
        .args(["run", case, "--kernel", kernel, "--stream", "1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            let out = c.stdout.take().expect("piped");
            let err = c.stderr.take().expect("piped");
            let reader = pump(app, case, out);
            let errs = drain_stderr(err);
            let st = c.wait();
            let _ = reader.join(); // 读完再走，否则最后一批日志会丢
            Ok((st?, errs.join().unwrap_or_default()))
        })
        .map_err(|e| format!("cannot start {}: {e}", cli.display()))?;
    let (status, err) = out;
    let code = status.code().unwrap_or(-1);
    if !err.is_empty() {
        let _ = app.emit(
            "run://lines",
            Lines {
                case: case.to_string(),
                lines: err.clone(),
            },
        );
    }
    let _ = app.emit(
        "run://done",
        Done {
            case: case.to_string(),
            code,
            total: 0,
            dropped: 0,
            reason: (code != 0).then(|| failure_reason(&err)).flatten(),
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
    site: String,
    out: String,
    name: Option<String>,
    start: Option<String>,
    end: Option<String>,
    // 城市站点必须给这两个：土壤剖面、湖深、土壤反照率与 LCZ 分类
    // 只能从全球栅格取，站点文件里没有。非城市站点传空即可。
    rawdata: Option<String>,
    runtime: Option<String>,
) -> Result<String, String> {
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
    ] {
        if let Some(v) = v {
            if !v.trim().is_empty() {
                args.push(flag.into());
                args.push(v);
            }
        }
    }
    capture(&args)
}

/// 评估：把模型与观测配对，出指标与配对点。
///
/// 走 sidecar 而不是在这里算 —— 要读两个 NetCDF 文件。
/// **一次拿全**：指标表、双线图、散点图用的是同一批配对结果，
/// 分三次跑等于把同一份文件读三遍，而且三者可能因参数不一致而对不上。
#[tauri::command]
pub async fn metrics(
    case: String,
    obs: String,
    spinup: usize,
    corrected: bool,
) -> Result<String, String> {
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
    capture(&args)
}

/// 取绘图数据。
///
/// 走 sidecar 而不是在这里读 —— 窗口进程不链接 netcdf，
/// 不该为了画一条曲线把整个静态 HDF5 拖进来。
#[tauri::command]
pub async fn series(case: String, vars: String) -> Result<String, String> {
    capture(&["series".to_string(), case, "--vars".into(), vars])
}

/// 跑一次 sidecar，把 stdout 整个收回来。
///
/// 用于短命令（`new` / `series`）；跑模型那种长命令走 `run_case` 的流式路径。
pub(crate) fn capture(args: &[String]) -> Result<String, String> {
    let cli = resolve_cli();
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
