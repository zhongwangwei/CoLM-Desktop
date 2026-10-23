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

use crate::sidecar::{register_child, ChildOwner};

/// 固定的 uv 版本（2026-08-14 发布）。换版本要连同下面的校验和一起换，并用官方 `.sha256` 与实际下载核对。
const UV_VERSION: &str = "0.12.5";
/// (平台, 发布页上的压缩包, SHA-256)。2026-10-23 从发布页取得，并下载 aarch64-apple-darwin 一份核对过。
const UV_ARCHIVES: [(&str, &str, &str); 6] = [
    (
        "macos-aarch64",
        "uv-aarch64-apple-darwin.tar.gz",
        "5bb0e5fe008a773c3dbcb97ff79cd89e1241464fe9d2f986d52ad8f1b037bd62",
    ),
    (
        "macos-x86_64",
        "uv-x86_64-apple-darwin.tar.gz",
        "b3b2137477cf96c9686ebfb71524614cec780c673fd73e59bce099aef02e70e8",
    ),
    (
        "linux-x86_64",
        "uv-x86_64-unknown-linux-gnu.tar.gz",
        "68a509da24b06b4223a1c0175fb5eb5bc79342b76cbeff0cfe51ac3f5b17b6b2",
    ),
    (
        "linux-aarch64",
        "uv-aarch64-unknown-linux-gnu.tar.gz",
        "9bf43b4d1a07665bf64d4c4e710930b382321a785e0eb10aac07f46471f86a31",
    ),
    (
        "windows-x86_64",
        "uv-x86_64-pc-windows-msvc.zip",
        "4c4d49d8738847d9b71ba319e49a5688c93eac0fe6204b1df24e98528dddf39a",
    ),
    (
        "windows-aarch64",
        "uv-aarch64-pc-windows-msvc.zip",
        "724279317fee6e5fa8ad1908e4eba2bbe764ef1ece5b3f4597927b62b1fe562a",
    ),
];
/// OpenBench 要求 Python ≥ 3.10；3.12 的科学计算轮子（cartopy、netCDF4）三个平台都齐。
const PYTHON_VERSION: &str = "3.12";
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

fn uv_archive() -> Result<(&'static str, &'static str), String> {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    UV_ARCHIVES
        .iter()
        .find(|(key, _, _)| *key == platform)
        .map(|(_, archive, sha)| (*archive, *sha))
        .ok_or_else(|| format!("没有给 {platform} 准备 uv，请自行安装 Python 与 colm-openbench"))
}

fn own_uv(root: &Path) -> PathBuf {
    root.join("tools").join(exe("uv"))
}

/// 用哪个 uv：自己装的优先，没有就用系统里已有的。
fn uv(root: &Path) -> Option<PathBuf> {
    let own = own_uv(root);
    if own.is_file() {
        return Some(own);
    }
    which("uv")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe(name)))
        .find(|candidate| candidate.is_file())
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
    command
        .env("UV_PYTHON_INSTALL_DIR", root.join("python"))
        .env("UV_CACHE_DIR", root.join("cache"))
        .env("UV_NO_PROGRESS", "1");
    command
}

fn quiet(mut command: Command) -> Option<String> {
    colm_kernel::run::no_console(&mut command);
    let output = command.stderr(Stdio::null()).output().ok()?;
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
        let python = env_bin(&root, "python");
        let python_version = python
            .is_file()
            .then(|| {
                let mut command = Command::new(&python);
                command.arg("--version");
                quiet(command)
            })
            .flatten();
        let openbench = python
            .is_file()
            .then(|| {
                let mut command = Command::new(&python);
                command.args(["-c", "import openbench; print(openbench.__version__)"]);
                quiet(command)
            })
            .flatten();
        Ok(json!({
            "root": root,
            "uv": uv,
            "uv_own": own_uv(&root).is_file(),
            "uv_version": UV_VERSION,
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
fn find_report(output: &Path) -> Option<PathBuf> {
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
#[tauri::command]
pub async fn bench_install(
    app: AppHandle,
    step: String,
    source: Option<String>,
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
                match source
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                {
                    None => {
                        command.arg("colm-openbench");
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
                Ok("ok".into())
            }
            other => Err(format!("没有这一步：{other}")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn install_uv(app: &AppHandle, root: &Path) -> Result<PathBuf, String> {
    let (archive, sha) = uv_archive()?;
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
        let mut command = Command::new(openbench);
        command
            .args(["ref", "list", "--format", "json"])
            .env("NO_COLOR", "1");
        colm_kernel::run::no_console(&mut command);
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&strict_json(&text)).map_err(|e| format!("参考数据列表读不懂：{e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// OpenBench 用 Python 的 `json.dumps` 输出，值里会有 `NaN`、`Infinity`（实测登记表里 23 处，如
/// `max_uparea: Infinity`），严格的 JSON 解析器不认。字符串外、作为值出现的这几个记号换成 `null`。
fn strict_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut last = ' ';
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            rest = &rest[c.len_utf8()..];
            continue;
        }
        if matches!(last, ':' | ',' | '[') {
            if let Some(token) = ["-Infinity", "Infinity", "NaN"]
                .iter()
                .find(|token| rest.starts_with(**token))
            {
                out.push_str("null");
                rest = &rest[token.len()..];
                last = 'l';
                continue;
            }
        }
        if c == '"' {
            in_string = true;
        }
        if !c.is_whitespace() {
            last = c;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn valid_project(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 60
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then_some(())
        .ok_or_else(|| format!("比较名只能用字母、数字、- 和 _：{name:?}"))
}

/// 看一个 history 目录：文件前缀、时间分辨率、年份、是不是格点、单点的经纬度。用专用环境里的 xarray 读，
/// 窗口进程不链接 NetCDF。
const INSPECT_SCRIPT: &str = r#"
import glob, json, os, sys
import numpy as np, xarray as xr
folder = sys.argv[1]
files = sorted(glob.glob(os.path.join(folder, "*_hist_*.nc")))
if not files:
    # 给的是算例目录：history 在 out/<DEF_CASE_NAME>/history（名字不一定和目录名一样）。
    nested = sorted(glob.glob(os.path.join(folder, "out", "*", "history", "*_hist_*.nc")))
    if nested:
        folder = os.path.dirname(nested[0])
        files = sorted(glob.glob(os.path.join(folder, "*_hist_*.nc")))
if not files:
    raise SystemExit(f"no *_hist_*.nc in {folder}")
name = os.path.basename(files[0])
prefix = name[: name.index("_hist_") + len("_hist_")]
with xr.open_dataset(files[0]) as first:
    times = first["time"].values
    step = float((times[1] - times[0]) / np.timedelta64(1, "s")) if len(times) > 1 else 2.6e6
    spatial = any(first.sizes.get(d, 1) > 1 for d in ("lat", "lon", "lat_cama", "lon_cama"))
    lon = float(first["lon"].values.ravel()[0]) if "lon" in first and not spatial else None
    lat = float(first["lat"].values.ravel()[0]) if "lat" in first and not spatial else None
    start = int(str(times[0])[:4])
with xr.open_dataset(files[-1]) as last:
    end = int(str(last["time"].values[-1])[:4])
tim = "Hour" if step <= 3600 else "3Hour" if step <= 3 * 3600 else "Day" if step <= 86400 else "Month"
print(json.dumps({"history_dir": os.path.abspath(folder), "prefix": prefix, "tim_res": tim, "syear": start, "eyear": end,
                  "spatial": spatial, "lon": lon, "lat": lat, "files": len(files)}))
"#;

/// 单点算例的 history 是每月一个文件；OpenBench 的站点模拟要一个站一个文件。只并只随时间变的地表变量
/// （维度只有 `time`、`patch`）：带上土壤层、积雪层这些维度一起并，实测 AT-Neu 48 个文件吃到 17 GB 内存
/// 十分钟还没完；只并地表变量 9.5 秒、360 MB。站点清单（ID,SYEAR,EYEAR,LON,LAT,DIR）用文件自己的经纬度与年份。
const PREPARE_SCRIPT: &str = r#"
import glob, os, sys
import xarray as xr
folder, prefix, out_nc, out_csv, station = sys.argv[1:6]
files = sorted(glob.glob(os.path.join(folder, prefix + "*.nc")))
if not files:
    raise SystemExit(f"no {prefix}*.nc in {folder}")
parts = []
for path in files:
    with xr.open_dataset(path) as ds:
        keep = [v for v in ds.data_vars if "time" in ds[v].dims and set(ds[v].dims) <= {"time", "patch"}]
        part = ds[keep]
        if "patch" in part.dims:
            part = part.squeeze("patch", drop=True)
        parts.append(part.load())
merged = xr.concat(parts, dim="time")
with xr.open_dataset(files[0]) as first:
    lon = float(first["lon"].values.ravel()[0])
    lat = float(first["lat"].values.ravel()[0])
merged["lon"] = lon
merged["lat"] = lat
merged.to_netcdf(out_nc)
start = int(str(merged["time"].values[0])[:4])
end = int(str(merged["time"].values[-1])[:4])
with open(out_csv, "w") as handle:
    handle.write("ID,SYEAR,EYEAR,LON,LAT,DIR\n")
    handle.write(f"{station},{start},{end},{lon},{lat},{os.path.abspath(out_nc)}\n")
print(f"{station}: {len(files)} files, {merged.sizes['time']} steps, {len(merged.data_vars)} variables")
"#;

/// 一个 history 目录的概况（[`INSPECT_SCRIPT`]）。比较页加模拟时调，填前缀、时间分辨率、年份与种类。
#[tauri::command]
pub async fn bench_inspect(dir: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let root = root()?;
        let python = env_bin(&root, "python");
        if !python.is_file() {
            return Err("先装专用 Python 和 OpenBench".into());
        }
        let mut command = Command::new(python);
        command.args(["-c", INSPECT_SCRIPT]).arg(&dir);
        colm_kernel::run::no_console(&mut command);
        let output = command.output().map_err(|e| e.to_string())?;
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
        // 站点 ID 写进 CSV：不能有逗号、引号、换行。
        if site.station.is_empty()
            || site.station.len() > 80
            || site
                .station
                .chars()
                .any(|c| matches!(c, ',' | '"' | '\n' | '\r'))
        {
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
            let mut command = Command::new(env_bin(&root, "python"));
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
            let mut command = Command::new(&openbench);
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
        let mut command = Command::new(openbench);
        command
            .args(["ref", "register", &name, "--root-dir", &root_dir])
            .args(["--data-type", &data_type, "--tim-res", &tim_res])
            .args(["--years", &years.0.to_string(), &years.1.to_string()])
            .env("NO_COLOR", "1");
        if let Some(category) = category.filter(|c| !c.trim().is_empty()) {
            command.args(["--category", &category]);
        }
        if let Some(list) = fulllist.filter(|f| !f.trim().is_empty()) {
            command.args(["--fulllist", &list]);
        }
        for variable in &variables {
            command.args(["-v", variable]);
        }
        colm_kernel::run::no_console(&mut command);
        let output = command.output().map_err(|e| e.to_string())?;
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

/// 预览一个参考数据目录里能自动认出哪些数据集（`openbench ref scan --dry-run`，不写登记表）。
#[tauri::command]
pub async fn bench_scan_preview(dir: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let root = root()?;
        let openbench = env_bin(&root, "openbench");
        if !openbench.is_file() {
            return Err("先装 OpenBench".into());
        }
        let mut command = Command::new(openbench);
        command
            .args(["ref", "scan", &dir, "--dry-run", "--allow-skip"])
            .env("NO_COLOR", "1");
        colm_kernel::run::no_console(&mut command);
        let output = command.output().map_err(|e| e.to_string())?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读一份 OpenBench 报告在软件里显示。报告可以是这里跑出来的，也可以是用户选的任何一份
/// （"载入报告"）；只读 `.html`，过大的请用浏览器打开。
#[tauri::command]
pub async fn bench_report(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let is_html = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"));
        if !is_html {
            return Err("报告应是 .html 文件".into());
        }
        let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if size > INLINE_REPORT_LIMIT {
            return Err(format!(
                "报告有 {} MB，太大不在软件里显示；请用浏览器打开",
                size / 1024 / 1024
            ));
        }
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "bench_tests.rs"]
mod bench_tests;
