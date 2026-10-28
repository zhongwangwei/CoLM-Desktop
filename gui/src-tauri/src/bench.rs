//! "比较"页：用 OpenBench 在模拟之间、版本之间做评估，并载入它生成的报告。
//!
//! OpenBench 是 Python 包（`colm-openbench`）。环境装在应用数据目录下的 `openbench/` 里，不碰系统：
//! - `tools/uv`：固定版本的 uv，从官方发布页下载，按写死的 SHA-256 校验后才解压（不执行远程脚本）；
//! - `python/`：uv 管理的专用 Python（`--python-preference only-managed`，不借用系统里的 Python）；
//! - `env/`：虚拟环境，OpenBench 和它的全部依赖装在这里；
//! - `projects/<名字>/`：一次比较。每次运行一个不可变快照 `runs/<run_id>/`（冻结的 `openbench.yaml`、
//!   准备出来的 NC/CSV、站点/输入清单、`meta.json` 与输出），`latest-success.json` 指向上次成功的那次；
//!   输出根来自配置的 `project.output_dir`，归一后记进 `meta.json`，报告只按这次运行的记录发现。
//!   旧目录没有 `runs/`，报告仍按 `output/` 里第一份 HTML 兼容显示；远程取回的本机镜像根下就是那次
//!   运行的 `meta.json`，同样按记录发现。失败不改上次成功指针（比较页审查报告第 5 条）。
//!
//! 窗口进程不链接 NetCDF：OpenBench 是独立的 Python 进程，这里只负责起进程、转日志、找报告。
//! 每个子进程都登记成 [`ChildOwner::Bench`]，停止按钮和关窗都能终止它；取消还记一个运行级代次 ——
//! prepare→check→run 之间没有活进程的那一刻点的停止也生效，后续阶段不再启动（比较页复查 C21）。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use colm_bench::{
    local_value, strict_json, uv_archive, valid_name, valid_station, INSPECT_SCRIPT, METRIC_SCRIPT,
    OPENBENCH_VERSION, PREPARE_SCRIPT, PROGRAM_ENV, PYTHON_VERSION, UV_ENV, UV_VERSION,
};

use crate::bench_runs::{self, RunSite, RunSnapshot};
use crate::sidecar::{register_child, ChildOwner};

/// 在软件里直接显示的报告大小上限；独立报告把图嵌在 HTML 里，更大的交给浏览器打开。
const INLINE_REPORT_LIMIT: u64 = 40 * 1024 * 1024;

/// 取消返回的固定文字：前端按它把错误识别成"已停止"（Stopped），措辞不能改。
const STOPPED: &str = "已停止";

/// 运行级取消代次。`bench_cancel` 推进它，已经开始的运行在下一个阶段边界就能看见。只终止登记表
/// 里的子进程不够：prepare(sites)→check→run 之间没有活进程，那一瞬间点的停止会漏掉后续整个阶段，
/// 界面显示"已停止"而 OpenBench 还在跑（比较页复查 C21）。
static CANCEL_EPOCH: AtomicU64 = AtomicU64::new(0);

/// 推进取消代次：此刻之前开始的运行全部失效（之后 `CancelEpoch::begin` 取的新快照不受影响）。
/// `bench_cancel` 与关窗路径（`lib.rs` 的 `stop_everything`）都要调：关窗那条路只终止已登记子进程，
/// 卡在阶段之间的运行仍会开下一阶段。调用方必须"先推进代次再杀已登记的子进程"：反过来，卡在阶段
/// 之间、当时没有任何子进程可杀的那次运行看不到任何变化，仍会开下一阶段（C21 根因）。
pub(crate) fn advance_cancel_epoch() {
    CANCEL_EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// 一次运行开始时取的取消快照。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CancelEpoch(u64);

impl CancelEpoch {
    /// 取当前代次。只有"运行开始之后"发生的取消才算数：上一轮留下的取消不能拦住新运行。
    fn begin() -> Self {
        Self(CANCEL_EPOCH.load(Ordering::SeqCst))
    }

    /// 起新进程或进新阶段之前核对。Err 是取消而不是失败，调用方必须原样返回 [`STOPPED`]。
    fn check(self) -> Result<(), String> {
        if CANCEL_EPOCH.load(Ordering::SeqCst) == self.0 {
            Ok(())
        } else {
            Err(STOPPED.into())
        }
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

pub(crate) fn root() -> Result<PathBuf, String> {
    Ok(crate::model::root()?
        .parent()
        .ok_or("model registry has no parent directory")?
        .join("openbench"))
}

fn this_uv_archive() -> Result<(&'static str, &'static str), String> {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    uv_archive(&platform)
        .ok_or_else(|| format!("没有给 {platform} 准备 uv，请自行安装 Python 与 colm-openbench"))
}

fn own_uv(root: &Path) -> PathBuf {
    root.join("tools").join(exe("uv"))
}

/// 只用自己装的 uv：系统里的 uv 版本不受控，还会读用户自己的 `uv.toml`，装出来的东西也就不全在这个目录下。
fn uv(root: &Path) -> Option<PathBuf> {
    let own = own_uv(root);
    own.is_file().then_some(own)
}

fn env_bin(root: &Path, name: &str) -> PathBuf {
    let env = root.join("env");
    if cfg!(windows) {
        env.join("Scripts").join(exe(name))
    } else {
        env.join("bin").join(name)
    }
}

fn uv_command(root: &Path, uv: &Path) -> Command {
    let mut command = Command::new(uv);
    for (key, value) in UV_ENV {
        command.env(key, local_value(root, value));
    }
    command
}

/// 专用环境里的程序（python、openbench），环境变量见 [`colm_bench::PROGRAM_ENV`]。
fn env_command(root: &Path, name: &str) -> Command {
    let mut command = Command::new(env_bin(root, name));
    for (key, value) in PROGRAM_ENV {
        command.env(key, local_value(root, value));
    }
    command
}

/// 跑一个短命令收齐输出。也登记成 [`ChildOwner::Bench`]、进自己的进程组：读登记表、看 history 这些
/// 命令要起 Python，慢的时候几十秒，原来不登记，停止按钮和关窗都杀不到（复验 D2-02）。
fn output(cancel: CancelEpoch, mut command: Command) -> Result<std::process::Output, String> {
    // 取消后一个进程都不再起：命令还在排队、拼参数时点的停止不能漏掉这次启动（比较页复查 C21）。
    cancel.check()?;
    command
        .env("PYTHONIOENCODING", "utf-8")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    colm_kernel::run::top_level_sidecar(&mut command);
    let child = command.spawn().map_err(|e| format!("无法启动：{e}"))?;
    let guard = register_child(child.id(), ChildOwner::Bench);
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() && guard.stopped() {
        return Err(STOPPED.into());
    }
    Ok(output)
}

fn quiet(cancel: CancelEpoch, command: Command) -> Option<String> {
    let output = output(cancel, command).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 跑一个子进程，逐行把输出转给界面（`bench://log`），登记成可停止的任务。失败时带回最后几行输出。
fn stream(
    app: &AppHandle,
    cancel: CancelEpoch,
    mut command: Command,
    stage: &str,
) -> Result<(), String> {
    // 阶段之间的空档没有登记中的子进程，只靠登记表杀不到；每次 spawn 前先核对代次（C21）。
    cancel.check()?;
    command
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    colm_kernel::run::top_level_sidecar(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 {stage}：{e}"))?;
    let guard = register_child(child.id(), ChildOwner::Bench);
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err("子进程没有输出管道".into());
    };
    let tail = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let forward = |reader: Box<dyn std::io::Read + Send>| {
        let app = app.clone();
        let stage = stage.to_owned();
        let tail = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(reader).lines().map_while(Result::ok) {
                let _ = app.emit("bench://log", json!({ "stage": stage, "line": line }));
                if let Ok(mut tail) = tail.lock() {
                    tail.push_back(line);
                    if tail.len() > 40 {
                        tail.pop_front();
                    }
                }
            }
        })
    };
    let readers = [forward(Box::new(out)), forward(Box::new(err))];
    let status = child.wait().map_err(|e| e.to_string())?;
    for reader in readers {
        let _ = reader.join();
    }
    if status.success() {
        return Ok(());
    }
    if guard.stopped() {
        return Err(STOPPED.into());
    }
    let tail = tail
        .lock()
        .map(|tail| tail.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    Err(format!("{stage} 失败（{status}）：\n{tail}"))
}

fn say(app: &AppHandle, stage: &str, line: &str) {
    let _ = app.emit("bench://log", json!({ "stage": stage, "line": line }));
}

/// 环境装到了哪一步、有哪些比较项目。
#[tauri::command]
pub async fn bench_status() -> Result<Value, String> {
    // 版本探测也是登记成 Bench 的子进程：取快照，取消后不再起新的探测。
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let uv = uv(&root);
        // 如实报 uv 自己说的版本：文件可能是旧版本留下的（复验 D2-03）。
        let uv_version = uv
            .as_ref()
            .and_then(|uv| {
                let mut command = Command::new(uv);
                command.arg("--version");
                quiet(cancel, command)
            })
            .and_then(|text| text.split_whitespace().nth(1).map(str::to_owned));
        let python = env_bin(&root, "python");
        let python_version = python
            .is_file()
            .then(|| {
                let mut command = env_command(&root, "python");
                command.arg("--version");
                quiet(cancel, command)
            })
            .flatten();
        let openbench = python
            .is_file()
            .then(|| {
                let mut command = env_command(&root, "python");
                command.args(["-c", "import openbench; print(openbench.__version__)"]);
                quiet(cancel, command)
            })
            .flatten();
        Ok(json!({
            "root": root,
            "uv": uv,
            "uv_own": own_uv(&root).is_file(),
            "uv_version": uv_version,
            "uv_pinned": UV_VERSION,
            "openbench_pinned": OPENBENCH_VERSION,
            "python": python_version,
            "openbench": openbench,
            "projects": projects(&root),
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn projects(root: &Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(root.join("projects")) else {
        return Vec::new();
    };
    let mut out = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| project_json(&entry.path()))
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    out
}

/// 比较项目在状态页上的一行。新目录的报告只认 `latest-success.json`（失败/进行中的运行不顶掉
/// 上次成功的报告）；远程取回的本机镜像没有 `runs/`，根下的 `meta.json` 就是那次运行的记录，
/// 同样按记录里的报告路径发现；只有连运行记录都没有的旧目录才回退到旧的 `output/` 扫描。
fn project_json(dir: &Path) -> Value {
    let runs = bench_runs::has_runs(dir);
    let flat = (!runs).then(|| bench_runs::flat_run(dir)).flatten();
    let latest = bench_runs::latest_run(dir).or(flat);
    let success = bench_runs::latest_success(dir).or_else(|| {
        latest
            .as_ref()
            .and_then(bench_runs::RunRecord::succeeded_latest)
    });
    // 旧目录才回退：新目录即使 `latest-success.json` 还没有（第一次运行失败），也不能把项目根
    // `output/` 里旧版本留下的 HTML 当成当前报告 —— 那正是"报告没绑到运行"的老毛病。
    let legacy = !runs && latest.is_none();
    let legacy_report = legacy.then(|| find_report(&dir.join("output"))).flatten();
    let report = success
        .as_ref()
        .and_then(|success| success.report_path(dir))
        .or(legacy_report);
    let legacy_config = dir.join("openbench.yaml");
    let config = latest
        .as_ref()
        .map(|run| run.dir().join(bench_runs::CONFIG_FILE))
        .filter(|path| path.is_file())
        .or_else(|| legacy_config.is_file().then_some(legacy_config));
    // 排序键：新目录用最新一次运行的创建时间；旧目录还用配置文件的修改时间。
    let modified = latest
        .as_ref()
        .map(|run| run.meta().created_unix)
        .or_else(|| modified_secs(dir.join("openbench.yaml")));
    json!({
        "name": dir.file_name().map(|name| name.to_string_lossy()).unwrap_or_default(),
        "dir": dir,
        "config": config,
        "report": report,
        "latest_success": success.as_ref().map(|success| success.to_json(dir)),
        "latest_run": latest.as_ref().map(|run| run.to_json()),
        "modified": modified,
    })
}

fn modified_secs(path: PathBuf) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
}

/// `output/<项目>/reports/` 下的报告：优先把图嵌在里面的独立版。
pub(crate) fn find_report(output: &Path) -> Option<PathBuf> {
    let projects = std::fs::read_dir(output).ok()?;
    let mut found = None;
    for project in projects.filter_map(Result::ok) {
        let reports = project.path().join("reports");
        for name in [
            "evaluation_report_standalone.html",
            "evaluation_report.html",
        ] {
            let path = reports.join(name);
            if path.is_file() {
                found = Some(path);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    found
}

/// 装环境的一步：`uv`（下载并校验 uv）、`python`（专用 Python 与虚拟环境）、`openbench`（OpenBench 与全部依赖）。
/// `source` 只对 `openbench` 有意义：空着装 PyPI 发布版，给一个本地目录就从源码装（可编辑安装，改了源码立即生效）。
/// `index` 是 PyPI 镜像（见 [`colm_bench::valid_index_url`]），空着用官方 PyPI。
#[tauri::command]
pub async fn bench_install(
    app: AppHandle,
    step: String,
    source: Option<String>,
    index: Option<String>,
) -> Result<String, String> {
    // 安装可能停在两步之间（例如 openbench 的 pip 装完、预取底图还没起）：取快照，取消后不再起下一步。
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || {
        // 排队期间已经点了停止，就不开始下载/安装。
        cancel.check()?;
        let root = root()?;
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        match step.as_str() {
            "uv" => install_uv(&app, &root, cancel).map(|path| path.display().to_string()),
            "python" => {
                let uv = uv(&root).ok_or("先装 uv")?;
                let mut command = uv_command(&root, &uv);
                command
                    .args(["venv", "--python", PYTHON_VERSION])
                    .args(["--python-preference", "only-managed", "--allow-existing"])
                    .arg(root.join("env"));
                stream(&app, cancel, command, "python")?;
                Ok(env_bin(&root, "python").display().to_string())
            }
            "openbench" => {
                let uv = uv(&root).ok_or("先装 uv")?;
                let python = env_bin(&root, "python");
                if !python.is_file() {
                    return Err("先装专用 Python".into());
                }
                let mut command = uv_command(&root, &uv);
                command.args(["pip", "install", "--python"]).arg(&python);
                if let Some(url) = index.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty()) {
                    if !colm_bench::valid_index_url(&url) {
                        return Err(format!("镜像地址不合适：{url}"));
                    }
                    command.args(["--default-index", &url]);
                }
                match source
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                {
                    None => {
                        command.arg(format!("colm-openbench=={OPENBENCH_VERSION}"));
                    }
                    Some(dir) => {
                        let dir = PathBuf::from(dir);
                        if !dir.join("pyproject.toml").is_file() {
                            return Err(format!(
                                "{} 不是 OpenBench 源码目录（没有 pyproject.toml）",
                                dir.display()
                            ));
                        }
                        command.arg("-e").arg(dir);
                    }
                }
                stream(&app, cancel, command, "openbench")?;
                say(&app, "openbench", "预取地图底图");
                let mut prefetch = env_command(&root, "python");
                prefetch.args(["-c", colm_bench::PREFETCH_SCRIPT]);
                stream(&app, cancel, prefetch, "openbench")?;
                Ok("ok".into())
            }
            other => Err(format!("没有这一步：{other}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn install_uv(app: &AppHandle, root: &Path, cancel: CancelEpoch) -> Result<PathBuf, String> {
    let (archive, sha) = this_uv_archive()?;
    let tools = root.join("tools");
    std::fs::create_dir_all(&tools).map_err(|e| e.to_string())?;
    let file = tools.join(archive);
    let url = format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{archive}");
    say(app, "uv", &format!("下载 {url}"));
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "--retry", "3", "-o"])
        .arg(&file)
        .arg(&url);
    stream(app, cancel, curl, "uv")?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    let actual = colm_kernel::manifest::sha256_hex(&bytes);
    if actual != sha {
        let _ = std::fs::remove_file(&file);
        return Err(format!(
            "下载的 uv 校验不对（{actual}，应为 {sha}），已删除，请重试"
        ));
    }
    say(app, "uv", "校验通过，解压");
    let unpack = tools.join("unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    std::fs::create_dir_all(&unpack).map_err(|e| e.to_string())?;
    // macOS、Linux 的 tar 和 Windows 10 起自带的 tar（bsdtar）都能解 .tar.gz 与 .zip。
    let mut tar = Command::new("tar");
    tar.arg("-xf").arg(&file).arg("-C").arg(&unpack);
    stream(app, cancel, tar, "uv")?;
    let found = find_file(&unpack, &exe("uv")).ok_or("压缩包里没有 uv")?;
    let target = own_uv(root);
    std::fs::copy(&found, &target).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_file(&file);
    say(app, "uv", &format!("uv {UV_VERSION} 装好了"));
    Ok(target)
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if entry.file_name().to_string_lossy() == name {
            return Some(path);
        }
    }
    None
}

/// 停掉正在跑的安装或评估。除了终止已登记的子进程，还推进取消代次让阶段切换不再启动新进程。
#[tauri::command]
pub fn bench_cancel() -> Result<usize, String> {
    // 顺序不能反：先推进代次，卡在 prepare/check/run 之间的运行才会在下一个阶段边界停下；
    // 再终止已登记的子进程，正在跑的那一段立即结束。
    advance_cancel_epoch();
    crate::sidecar::cancel_bench_children()
}

/// 删掉环境：`env` 只删虚拟环境（OpenBench 与依赖），`all` 连 uv、专用 Python 和下载缓存一起删。
/// 比较项目和报告不删。
#[tauri::command]
pub async fn bench_uninstall(what: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root()?;
        let targets: Vec<PathBuf> = match what.as_str() {
            "env" => vec![root.join("env")],
            "all" => vec![
                root.join("env"),
                root.join("python"),
                root.join("cache"),
                root.join("tools"),
            ],
            other => return Err(format!("没有这一项：{other}")),
        };
        for target in targets {
            match std::fs::remove_dir_all(&target) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("删不掉 {}：{e}", target.display())),
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// OpenBench 登记表里的参考数据（`openbench ref list --format json`），按变量挑参考用。
#[tauri::command]
pub async fn bench_references() -> Result<Value, String> {
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "openbench");
        command.args(["ref", "list", "--format", "json"]);
        let output = output(cancel, command)?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&strict_json(&text)).map_err(|e| format!("参考数据列表读不懂：{e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

pub(crate) fn valid_project(name: &str) -> Result<(), String> {
    valid_name(name)
        .then_some(())
        .ok_or_else(|| format!("比较名只能用字母、数字、- 和 _：{name:?}"))
}

/// 一个 history 目录的概况（[`INSPECT_SCRIPT`]）。比较页加模拟时调，填前缀、时间分辨率、年份与种类。
#[tauri::command]
pub async fn bench_inspect(dir: String) -> Result<Value, String> {
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let python = env_bin(&root, "python");
        if !python.is_file() {
            return Err("先装专用 Python 和 OpenBench".into());
        }
        let mut command = env_command(&root, "python");
        command.args(["-c", INSPECT_SCRIPT]).arg(&dir);
        let output = output(cancel, command)?;
        if !output.status.success() {
            let text = String::from_utf8_lossy(&output.stderr);
            return Err(text.lines().last().unwrap_or("读不了这个目录").to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 要先并成一个文件的单点模拟（[`PREPARE_SCRIPT`]）：并到比较目录的 `sim/<id>.nc`，站点清单 `sim/<id>.csv`。
/// 这两个位置由前端生成的配置文本里的绝对路径决定（OpenBench 3.0.8 的 `check` 按 root_dir 解析 fulllist、
/// `run` 却按工作目录打开，相对路径总有一边找不到 —— 2026-10-23 窗口实测），不跟着本次运行的 cwd 走。
#[derive(serde::Deserialize)]
pub struct BenchSite {
    /// 文件名与 YAML 键（只有字母、数字、_、-）。
    pub id: String,
    /// 站点 ID：OpenBench 按它和参考数据里的站点对上，要用站点自己的名字（如 FLUXNET 的 `AT-Neu`），
    /// 不能用整理过的键（实测用 `AT_Neu` 报 "Station AT_Neu not found in merged station file"）。
    pub station: String,
    pub history_dir: String,
    pub prefix: String,
    /// 站点经纬度（十进制度）：PREPARE 优先用它们写站点清单，缺省时回退到 history 文件里的坐标。
    /// 两者要么都给、要么都不给 —— PREPARE 签名末尾的 `[lon] [lat]` 是成对的可选参数。
    #[serde(default)]
    pub lon: Option<f64>,
    #[serde(default)]
    pub lat: Option<f64>,
}

/// 经纬度：有限值、|lat| <= 90、|lon| <= 360（允许 0–360 的经度写法）；只给一个就没法用，直接拒绝。
fn valid_coordinates(lon: Option<f64>, lat: Option<f64>) -> Result<(), String> {
    match (lon, lat) {
        (None, None) => Ok(()),
        (Some(lon), Some(lat)) => {
            if !lon.is_finite() || !lat.is_finite() {
                return Err(format!("经纬度必须是有限数值（lon={lon}, lat={lat}）"));
            }
            if lon.abs() > 360.0 || lat.abs() > 90.0 {
                return Err(format!("经纬度超出范围（lon={lon}, lat={lat}）"));
            }
            Ok(())
        }
        _ => Err("经纬度要成对给：lon 和 lat 要么都有、要么都没有".into()),
    }
}

/// 本机与服务器两个入口共用的站点参数校验：id、站点名、经纬度。
pub(crate) fn validate_sites(sites: &[BenchSite]) -> Result<(), String> {
    for site in sites {
        valid_project(&site.id)?;
        if !valid_station(&site.station) {
            return Err(format!("站点名不合适：{:?}", site.station));
        }
        valid_coordinates(site.lon, site.lat)
            .map_err(|error| format!("站点 {}：{error}", site.id))?;
    }
    Ok(())
}

/// 把 IPC 参数转成运行快照里的站点记录（多存一份 history 目录与前缀作为输入清单）。
fn run_sites(sites: &[BenchSite]) -> Vec<RunSite> {
    sites
        .iter()
        .map(|site| RunSite {
            id: site.id.clone(),
            station: site.station.clone(),
            lon: site.lon,
            lat: site.lat,
            history_dir: site.history_dir.clone(),
            prefix: site.prefix.clone(),
        })
        .collect()
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

/// 本次运行的准备产物位置（NC 与站点清单）。抽出来是为了让"准备不该写共享的 `<项目>/sim/`"
/// 这条约束有一个不依赖 Tauri/OpenBench 的回归测试（R04）。
fn station_files(run: &RunSnapshot, site_id: &str) -> (PathBuf, PathBuf) {
    let dir = run.sim_dir();
    (
        dir.join(format!("{site_id}.nc")),
        dir.join(format!("{site_id}.csv")),
    )
}

/// 依次执行一组阶段：每个阶段开始时核对取消代次 —— 第一次迭代是"循环开始前"，之后每次是
/// "两个阶段之间"；取消后不再进入任何阶段。`bench_run` 的 prepare(sites)/check/run 用它串起来；
/// 抽成独立函数是为了能单测"取消后不再 spawn"——`bench_run` 本身要 Tauri 的 AppHandle
/// 和装好的 OpenBench，单测里起不了。
fn for_each_stage<T>(
    cancel: CancelEpoch,
    items: &[T],
    mut run: impl FnMut(&T) -> Result<(), String>,
) -> Result<(), String> {
    // 每个阶段开始时核对：第一次迭代就是"循环开始前"，之后每次就是"两个阶段之间"。这些空档里
    // 没有登记中的子进程，只靠终止登记表覆盖不到（C21）。最后一个阶段结束后不再核对：那时
    // 工作已经做完了，不能再把一次成功的运行记成"已停止"。
    for item in items {
        cancel.check()?;
        run(item)?;
    }
    Ok(())
}

/// 先把单点模拟并成站点文件，再 `openbench check`、`openbench run`，返回报告路径与本次运行身份。
///
/// 每次运行在 `<比较目录>/runs/<run_id>/` 建一个不可变快照并把配置文本冻结进去，`check`/`run` 在
/// 这个目录里跑 —— 配置里的 `./output` 相对 cwd，落进本次运行的 `output/`，报告只从这一次的
/// `output/` 发现，绝不跨 run 取。失败只写自己的 meta，上次成功指针一个字节都不动（审查报告第 5 条）。
#[tauri::command]
pub async fn bench_run(
    app: AppHandle,
    name: String,
    config: String,
    sites: Option<Vec<BenchSite>>,
    ref_root: Option<String>,
) -> Result<Value, String> {
    // 快照在命令入口取：从 IPC 入队到真正开始跑之间点的停止，也算这次运行期间（C21）。
    let cancel = CancelEpoch::begin();
    valid_project(&name)?;
    let sites = sites.unwrap_or_default();
    validate_sites(&sites)?;
    let frozen_sites = run_sites(&sites);
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        // 排队期间已经点了停止：连运行记录都不建，直接按取消返回。
        cancel.check()?;
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let project_dir = root.join("projects").join(&name);
        // 配置文本里的 `fulllist` 是项目 `sim/<id>.csv` 的绝对路径（OpenBench 的 check 与 run 对
        // 相对路径解析不一致，见 [`BenchSite`]），这里只建目录；真正的准备产物落在本次运行目录，
        // 再把清单复制过去（R04）。
        let sim = project_dir.join("sim");
        std::fs::create_dir_all(&sim).map_err(|e| e.to_string())?;
        let mut run = RunSnapshot::create(
            &project_dir,
            &name,
            &config,
            &frozen_sites,
            ref_root.as_deref(),
        )?;
        say(
            &app,
            "run",
            &format!(
                "本次运行 {}（配置 sha {}）；报告只从这次运行的输出目录里取",
                run.id(),
                short_sha(run.config_sha()),
            ),
        );
        let outcome = (|| -> Result<Option<PathBuf>, String> {
            // 站点合并阶段：每个站点起一个 Python 进程，取消后不再处理下一个站点。
            for_each_stage(cancel, &sites, |site| {
                say(&app, "prepare", &format!("合并 {} 的 history", site.id));
                // 准备产物写进本次运行目录：NC 与 CSV 一起冻结，下一次准备写的是新 run 目录，
                // 已归档运行不会被覆盖（R04）。
                let (nc, csv) = station_files(&run, &site.id);
                let mut command = env_command(&root, "python");
                command
                    .args(["-c", PREPARE_SCRIPT])
                    .arg(&site.history_dir)
                    .arg(&site.prefix)
                    .arg(&nc)
                    .arg(&csv)
                    .arg(&site.station);
                // PREPARE 签名末尾是可选的 [lon] [lat]；只给一个的输入前面已拒绝，这里成对追加。
                if let (Some(lon), Some(lat)) = (site.lon, site.lat) {
                    command.arg(lon.to_string()).arg(lat.to_string());
                }
                stream(&app, cancel, command, "prepare")?;
                // 配置里的 fulllist 位置必须存在：把本次运行的清单复制过去；清单里的 DIR 指向
                // 本运行目录的 NC，历史 run 的数据不会被下一次准备改掉。
                run.publish_station_manifest(&site.id)?;
                Ok(())
            })?;
            // 内置登记表的路径写成 `${OPENBENCH_REF_ROOT}/…`；不设它，`check` 直接报错（2026-10-23 窗口实测）。
            let ref_root = ref_root.filter(|r| !r.trim().is_empty());
            // check→run：每换一个阶段都重新核对取消，空档里点的停止不会再起进程（C21）。
            for_each_stage(cancel, &["check", "run"], |stage| {
                let stage = *stage;
                say(
                    &app,
                    stage,
                    &format!("openbench {stage}（运行 {}）", run.id()),
                );
                let mut command = env_command(&root, "openbench");
                command
                    .arg(stage)
                    .arg(bench_runs::CONFIG_FILE)
                    .current_dir(run.dir());
                if let Some(root) = &ref_root {
                    command.env("OPENBENCH_REF_ROOT", root);
                }
                stream(&app, cancel, command, stage)?;
                Ok(())
            })?;
            Ok(run.discover_report())
        })();
        match outcome {
            Ok(report) => {
                let latest = run.succeed(report.as_deref()).map_err(|error| {
                    format!(
                        "运行 {} 已结束，但没能记下这次运行或更新上次成功指针：{error}\n运行目录：{}",
                        run.id(),
                        run.dir().display(),
                    )
                })?;
                Ok(json!({
                    "dir": project_dir,
                    "config": run.config_file(),
                    "report": report,
                    "run_id": run.id(),
                    "run_dir": run.dir(),
                    "config_sha": run.config_sha(),
                    "state": "succeeded",
                    "latest_success": latest.to_json(&project_dir),
                }))
            }
            Err(detail) => {
                let cancelled = detail == STOPPED;
                if let Err(meta_error) = run.fail(&detail) {
                    return Err(format!(
                        "运行 {} 失败，运行记录也没能写成：{meta_error}\n{detail}",
                        run.id(),
                    ));
                }
                // 取消保持原文：前端按固定文字 recognize 成 Stopped。普通失败带上 run_id 与运行目录，
                // 否则用户不知道刚才那次运行把现场留在哪。
                if cancelled {
                    return Err(STOPPED.into());
                }
                let previous = if bench_runs::latest_success(&project_dir).is_some() {
                    "上次成功的报告未受影响。"
                } else {
                    "这个比较还没有成功过的报告。"
                };
                Err(format!(
                    "运行 {} 失败：{detail}\n运行目录：{}；配置与站点清单已冻结在这次运行里。{previous}",
                    run.id(),
                    run.dir().display(),
                ))
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 登记一个参考数据集（`openbench ref register`，不交互）。`variables` 每项是 `变量:文件里的名字:单位`，
/// 如 `Latent_Heat:LE:W m-2`。扫描遇到一个文件里有多个候选变量时会停下来问，这里直接写清楚。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn bench_register(
    name: String,
    root_dir: String,
    data_type: String,
    tim_res: String,
    years: (i32, i32),
    category: Option<String>,
    fulllist: Option<String>,
    variables: Vec<String>,
) -> Result<String, String> {
    valid_project(&name)?;
    if !["grid", "stn"].contains(&data_type.as_str()) {
        return Err("类型只能是 grid 或 stn".into());
    }
    if variables.is_empty() {
        return Err("至少给一个变量".into());
    }
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "openbench");
        command
            .args(["ref", "register", &name, "--root-dir", &root_dir])
            .args(["--data-type", &data_type, "--tim-res", &tim_res])
            .args(["--years", &years.0.to_string(), &years.1.to_string()]);
        if let Some(category) = category.filter(|c| !c.trim().is_empty()) {
            command.args(["--category", &category]);
        }
        if let Some(list) = fulllist.filter(|f| !f.trim().is_empty()) {
            command.args(["--fulllist", &list]);
        }
        for variable in &variables {
            command.args(["-v", variable]);
        }
        let output = output(cancel, command)?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success() {
            Ok(text)
        } else {
            Err(text.trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 给 OpenBench 的模型 profile 增加一个变量映射（新模块加进 CoLM 后，profile 里没有这个变量）。
#[tauri::command]
pub async fn bench_register_model_variable(
    model: String,
    variable: String,
    varname: String,
    unit: String,
) -> Result<String, String> {
    if !valid_model(&model) {
        return Err(format!("模型名不合适：{model:?}"));
    }
    for (label, value) in [("变量", &variable), ("变量名", &varname), ("单位", &unit)] {
        if !valid_mapping_field(value) {
            return Err(format!("{label}不合适：{value:?}"));
        }
    }
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let spec = format!("{}:{}:{}", variable.trim(), varname.trim(), unit.trim());
        let mut command = env_command(&root, "openbench");
        command
            .args(["model", "register", &model])
            .arg("-v")
            .arg(&spec)
            .arg("--append-only");
        let output = output(cancel, command)?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success() {
            Ok(text)
        } else {
            Err(text.trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 三段经 ':' 拼成 OpenBench 的 `-v` 规格；带冒号会被拆成 prefix/suffix 段，控制字符会弄坏命令行。
fn valid_mapping_field(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control) && !value.contains(':')
}

fn valid_model(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// 问已安装的 OpenBench 有哪些可选指标和评分（名称 + 显示名），用于比较页的多选。
#[tauri::command]
pub async fn bench_metric_options() -> Result<Value, String> {
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "python");
        command.args(["-c", METRIC_SCRIPT]);
        let output = output(cancel, command)?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| format!("指标列表读不懂：{e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 预览一个参考数据目录里能自动认出哪些数据集（`openbench ref scan --dry-run`，不写登记表）。
#[tauri::command]
pub async fn bench_scan_preview(dir: String) -> Result<String, String> {
    let cancel = CancelEpoch::begin();
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "openbench");
        command.args(["ref", "scan", &dir, "--dry-run", "--allow-skip"]);
        let output = output(cancel, command)?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // 扫描失败要报错，不能当成预览结果显示（复验 D2 P3：原来忽略退出码）。
        if output.status.success() {
            Ok(text)
        } else {
            Err(format!("扫描失败：{}", text.trim()))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读一份这里跑出来的报告在软件里显示：只认 `projects/` 下的 `.html`（规范化后再比，`..` 和符号链接
/// 绕不出去）。前端不持有读任意路径的能力（复验 D2-05）；别处的报告走 [`bench_pick_report`]。
#[tauri::command]
pub async fn bench_report(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let projects = root()?.join("projects");
        let path = PathBuf::from(path);
        if !inside(&projects, &path) {
            return Err("只能显示这里跑出来的报告；别处的报告请用\"载入报告…\"".into());
        }
        read_report(&path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "载入报告…"：用户在文件对话框里亲自选一份报告，返回路径和内容。路径由后端的对话框给出，
/// 不由前端传入。
#[tauri::command]
pub async fn bench_pick_report(app: AppHandle) -> Result<Option<Value>, String> {
    let Some(path) =
        crate::recent::pick_file(app, "openbench-report".into(), Some("html,htm".into())).await
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let html = read_report(Path::new(&path))?;
        Ok(Some(json!({ "path": path, "html": html })))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn inside(dir: &Path, path: &Path) -> bool {
    match (dir.canonicalize(), path.canonicalize()) {
        (Ok(dir), Ok(path)) => path.starts_with(dir),
        _ => false,
    }
}

/// 只读 `.html`，过大的请用浏览器打开。
fn read_report(path: &Path) -> Result<String, String> {
    let is_html = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"));
    if !is_html {
        return Err("报告应是 .html 文件".into());
    }
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > INLINE_REPORT_LIMIT {
        return Err(format!(
            "报告有 {} MB，太大不在软件里显示；请用浏览器打开",
            size / 1024 / 1024
        ));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "bench_tests.rs"]
mod bench_tests;
