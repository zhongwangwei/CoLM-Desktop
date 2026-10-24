//! "比较"页：用 OpenBench 在模拟之间、版本之间做评估，并载入它生成的报告。
//!
//! OpenBench 是 Python 包（`colm-openbench`）。环境装在应用数据目录下的 `openbench/` 里，不碰系统：
//! - `tools/uv`：固定版本的 uv，从官方发布页下载，按写死的 SHA-256 校验后才解压（不执行远程脚本）；
//! - `python/`：uv 管理的专用 Python（`--python-preference only-managed`，不借用系统里的 Python）；
//! - `env/`：虚拟环境，OpenBench 和它的全部依赖装在这里；
//! - `projects/<名字>/`：一次比较的 `openbench.yaml` 与输出，报告在 `output/<项目>/reports/`。
//!
//! 窗口进程不链接 NetCDF：OpenBench 是独立的 Python 进程，这里只负责起进程、转日志、找报告。
//! 每个子进程都登记成 [`ChildOwner::Bench`]，停止按钮和关窗都能终止它。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use colm_bench::{
    local_value, strict_json, uv_archive, valid_name, valid_station, INSPECT_SCRIPT, METRIC_SCRIPT,
    OPENBENCH_VERSION, PREPARE_SCRIPT, PROGRAM_ENV, PYTHON_VERSION, UV_ENV, UV_VERSION,
};

use crate::sidecar::{register_child, ChildOwner};

/// 在软件里直接显示的报告大小上限；独立报告把图嵌在 HTML 里，更大的交给浏览器打开。
const INLINE_REPORT_LIMIT: u64 = 40 * 1024 * 1024;

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
fn output(mut command: Command) -> Result<std::process::Output, String> {
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
        return Err("已停止".into());
    }
    Ok(output)
}

fn quiet(command: Command) -> Option<String> {
    let output = output(command).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 跑一个子进程，逐行把输出转给界面（`bench://log`），登记成可停止的任务。失败时带回最后几行输出。
fn stream(app: &AppHandle, mut command: Command, stage: &str) -> Result<(), String> {
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
        return Err("已停止".into());
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
    tauri::async_runtime::spawn_blocking(|| -> Result<Value, String> {
        let root = root()?;
        let uv = uv(&root);
        // 如实报 uv 自己说的版本：文件可能是旧版本留下的（复验 D2-03）。
        let uv_version = uv
            .as_ref()
            .and_then(|uv| {
                let mut command = Command::new(uv);
                command.arg("--version");
                quiet(command)
            })
            .and_then(|text| text.split_whitespace().nth(1).map(str::to_owned));
        let python = env_bin(&root, "python");
        let python_version = python
            .is_file()
            .then(|| {
                let mut command = env_command(&root, "python");
                command.arg("--version");
                quiet(command)
            })
            .flatten();
        let openbench = python
            .is_file()
            .then(|| {
                let mut command = env_command(&root, "python");
                command.args(["-c", "import openbench; print(openbench.__version__)"]);
                quiet(command)
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
        .map(|entry| {
            let dir = entry.path();
            let modified = std::fs::metadata(dir.join("openbench.yaml"))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            json!({
                "name": entry.file_name().to_string_lossy(),
                "dir": dir,
                "config": dir.join("openbench.yaml").is_file().then(|| dir.join("openbench.yaml")),
                "report": find_report(&dir.join("output")),
                "modified": modified,
            })
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
    out
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
    tauri::async_runtime::spawn_blocking(move || {
        let root = root()?;
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        match step.as_str() {
            "uv" => install_uv(&app, &root).map(|path| path.display().to_string()),
            "python" => {
                let uv = uv(&root).ok_or("先装 uv")?;
                let mut command = uv_command(&root, &uv);
                command
                    .args(["venv", "--python", PYTHON_VERSION])
                    .args(["--python-preference", "only-managed", "--allow-existing"])
                    .arg(root.join("env"));
                stream(&app, command, "python")?;
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
                stream(&app, command, "openbench")?;
                say(&app, "openbench", "预取地图底图");
                let mut prefetch = env_command(&root, "python");
                prefetch.args(["-c", colm_bench::PREFETCH_SCRIPT]);
                stream(&app, prefetch, "openbench")?;
                Ok("ok".into())
            }
            other => Err(format!("没有这一步：{other}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn install_uv(app: &AppHandle, root: &Path) -> Result<PathBuf, String> {
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
    stream(app, curl, "uv")?;
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
    stream(app, tar, "uv")?;
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

/// 停掉正在跑的安装或评估。
#[tauri::command]
pub fn bench_cancel() -> Result<usize, String> {
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
    tauri::async_runtime::spawn_blocking(|| -> Result<Value, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "openbench");
        command.args(["ref", "list", "--format", "json"]);
        let output = output(command)?;
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
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let python = env_bin(&root, "python");
        if !python.is_file() {
            return Err("先装专用 Python 和 OpenBench".into());
        }
        let mut command = env_command(&root, "python");
        command.args(["-c", INSPECT_SCRIPT]).arg(&dir);
        let output = output(command)?;
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
#[derive(serde::Deserialize)]
pub struct BenchSite {
    /// 文件名与 YAML 键（只有字母、数字、_、-）。
    pub id: String,
    /// 站点 ID：OpenBench 按它和参考数据里的站点对上，要用站点自己的名字（如 FLUXNET 的 `AT-Neu`），
    /// 不能用整理过的键（实测用 `AT_Neu` 报 "Station AT_Neu not found in merged station file"）。
    pub station: String,
    pub history_dir: String,
    pub prefix: String,
}

/// 写下 `openbench.yaml`，先把单点模拟并成站点文件，再 `openbench check`、`openbench run`，返回报告路径。
/// 在比较目录里运行，配置里的相对路径（`./output`、`sim/…`）都落在这个比较自己的目录下。
#[tauri::command]
pub async fn bench_run(
    app: AppHandle,
    name: String,
    config: String,
    sites: Option<Vec<BenchSite>>,
    ref_root: Option<String>,
) -> Result<Value, String> {
    valid_project(&name)?;
    for site in sites.iter().flatten() {
        valid_project(&site.id)?;
        if !valid_station(&site.station) {
            return Err(format!("站点名不合适：{:?}", site.station));
        }
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let dir = root.join("projects").join(&name);
        std::fs::create_dir_all(dir.join("sim")).map_err(|e| e.to_string())?;
        let file = dir.join("openbench.yaml");
        std::fs::write(&file, config).map_err(|e| e.to_string())?;
        for site in sites.unwrap_or_default() {
            say(&app, "prepare", &format!("合并 {} 的 history", site.id));
            let mut command = env_command(&root, "python");
            command
                .args(["-c", PREPARE_SCRIPT])
                .arg(&site.history_dir)
                .arg(&site.prefix)
                .arg(dir.join("sim").join(format!("{}.nc", site.id)))
                .arg(dir.join("sim").join(format!("{}.csv", site.id)))
                .arg(&site.station);
            stream(&app, command, "prepare")?;
        }
        // 内置登记表的路径写成 `${OPENBENCH_REF_ROOT}/…`；不设它，`check` 直接报错（2026-10-23 窗口实测）。
        let ref_root = ref_root.filter(|r| !r.trim().is_empty());
        for stage in ["check", "run"] {
            say(&app, stage, &format!("openbench {stage} openbench.yaml"));
            let mut command = env_command(&root, "openbench");
            command.arg(stage).arg("openbench.yaml").current_dir(&dir);
            if let Some(root) = &ref_root {
                command.env("OPENBENCH_REF_ROOT", root);
            }
            stream(&app, command, stage)?;
        }
        Ok(json!({
            "dir": dir,
            "config": file,
            "report": find_report(&dir.join("output")),
        }))
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
        let output = output(command)?;
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
        let output = output(command)?;
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
    tauri::async_runtime::spawn_blocking(|| -> Result<Value, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "python");
        command.args(["-c", METRIC_SCRIPT]);
        let output = output(command)?;
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
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = env_command(&root, "openbench");
        command.args(["ref", "scan", &dir, "--dry-run", "--allow-skip"]);
        let output = output(command)?;
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
