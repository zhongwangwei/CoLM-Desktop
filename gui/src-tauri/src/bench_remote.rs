//! 比较页选"服务器"时：环境、登记表、history、评估都在那台服务器上（`colm-cli remote-bench-*`），只把报告、
//! 指标与配置取回本机显示。服务器怎么连、工作目录在哪、用什么调度系统都取自"计算资源"里的服务器设置。
//!
//! 取回的结果放在本机 `openbench/projects/<名字>@<服务器>/`：和本机跑的比较列在一起，`bench_report` 也只认
//! `projects/` 下的报告。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::remote::{resource_dir, scheduler_args, server_for, ssh_cli_json, Server};

/// 拼一条 `remote-bench-*` 命令：带上这台服务器和它的工作目录。
fn args(server: &Server, command: &str, extra: &[(&str, String)]) -> Vec<String> {
    let mut args = vec![
        format!("remote-bench-{command}"),
        "--host".into(),
        server.host.clone(),
        "--root".into(),
        server.root.clone(),
    ];
    for (flag, value) in extra {
        args.push((*flag).to_owned());
        args.push(value.clone());
    }
    args
}

async fn call(
    app: tauri::AppHandle,
    host: &str,
    command: &str,
    extra: &[(&str, String)],
) -> Result<Value, String> {
    let server = server_for(app.clone(), host)?;
    ssh_cli_json(
        args(&server, command, extra),
        resource_dir(&app),
        Some(&server),
    )
    .await
}

/// 取回结果在本机的目录名：比较名加服务器名（只留字母、数字、`-`、`_`）。
///
/// C10：服务器名清洗会把 `user@host.example` 与本来就写成 `user_host_example` 的别名压成同一个
/// 镜像名；两台不同的机器就共用一个目录，后取回的会顶掉先取回的。清洗没有改动 host（普通 ASCII
/// 别名）时保持旧名字不变，兼容已有镜像；清洗改动过 host 时追加原始 host 的稳定 FNV-1a 短哈希
/// （自己实现，不用 `DefaultHasher`：它的算法与进程随机种子都不能当持久目录名的一部分）。
///
/// 分隔符用 `.`：清洗后的 host 只可能含字母/数字/`-`/`_`（旧格式的 host 因此不可能带 `.`），
/// 所以"旧名字"与"带哈希的新名字"永远不会撞在一起；旧名字继续用 `@`，已有镜像不用搬家。
pub(crate) fn mirror_name(name: &str, host: &str) -> Result<String, String> {
    crate::bench::valid_project(name)?;
    let sanitized = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        return Err("服务器名不能为空".into());
    }
    if sanitized == host {
        return Ok(format!("{name}@{host}"));
    }
    // 清洗后的 host 全是 ASCII（其余字符都换成了 `_`），截断不会切坏 UTF-8；限长是为了目录名
    // 不被一个超长主机名撑爆（valid_project 的 60 字节限制只管比较名那一半）。
    let host_part = &sanitized[..sanitized.len().min(MIRROR_HOST_LIMIT)];
    Ok(format!("{name}@{host_part}.{}", fnv1a_8(host)))
}

/// 清洗过的 host 在镜像名里最多留这么多字符；再加 `@`、`.`、8 位哈希与比较名（≤60）。
const MIRROR_HOST_LIMIT: usize = 48;

/// 原始 host 的稳定 32 位 FNV-1a 哈希，8 位十六进制。选 FNV-1a 是因为实现只有两行、跨平台逐位
/// 一致，足够把两个被清洗成同名的 host 分开；不是加密用途。
fn fnv1a_8(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

/// 服务器上的环境装到了哪一步、有哪些比较。
#[tauri::command]
pub async fn bench_remote_status(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    call(app, &host, "status", &[]).await
}

/// 在服务器上装环境的一步（作为服务器上的作业提交），返回作业号；进度用 [`bench_remote_job`] 看。
#[tauri::command]
pub async fn bench_remote_install(
    app: tauri::AppHandle,
    host: String,
    step: String,
    source: Option<String>,
    index: Option<String>,
) -> Result<Value, String> {
    if !["uv", "python", "openbench"].contains(&step.as_str()) {
        return Err(format!("没有这一步：{step}"));
    }
    let mut extra = vec![("--step", step)];
    if let Some(source) = source.filter(|s| !s.trim().is_empty()) {
        extra.push(("--source", source));
    }
    if let Some(index) = index.filter(|s| !s.trim().is_empty()) {
        if !colm_bench::valid_index_url(&index) {
            return Err(format!("镜像地址不合适：{index}"));
        }
        extra.push(("--index-url", index));
    }
    call(app, &host, "install", &extra).await
}

fn check_job(job: &str) -> Result<(), String> {
    let ok = job.starts_with("bench-")
        && job
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some(())
        .ok_or_else(|| format!("不是比较页的作业：{job}"))
}

/// 远程 run 的身份就是提交时本地生成的作业号（`<服务器比较目录>/runs/<作业号>/`）。
fn check_run(run: &str) -> Result<(), String> {
    check_job(run).map_err(|_| format!("不是一次评估运行：{run}"))
}

/// `remote-bench-fetch` 的参数：比较名、本机临时目录，以及可选的运行号（不传 = latest-success）。
fn fetch_extra(name: &str, out: &str, run_id: Option<&str>) -> Vec<(&'static str, String)> {
    let mut extra = vec![("--name", name.to_owned()), ("--out", out.to_owned())];
    if let Some(run_id) = run_id {
        extra.push(("--run", run_id.to_owned()));
    }
    extra
}

/// 删掉服务器上的环境：`env` 只删虚拟环境，`all` 连 uv、专用 Python 和下载缓存；登记表与比较留着。
#[tauri::command]
pub async fn bench_remote_uninstall(
    app: tauri::AppHandle,
    host: String,
    what: String,
) -> Result<Value, String> {
    if !["env", "all"].contains(&what.as_str()) {
        return Err(format!("没有这一项：{what}"));
    }
    call(app, &host, "uninstall", &[("--what", what)]).await
}

/// 服务器上一个作业的状态与日志末尾。
#[tauri::command]
pub async fn bench_remote_job(
    app: tauri::AppHandle,
    host: String,
    job: String,
    lines: Option<u32>,
) -> Result<Value, String> {
    check_job(&job)?;
    let lines = lines.unwrap_or(80).min(400).to_string();
    call(app, &host, "job", &[("--job", job), ("--lines", lines)]).await
}

/// 结束服务器上的作业。
#[tauri::command]
pub async fn bench_remote_cancel(
    app: tauri::AppHandle,
    host: String,
    job: String,
) -> Result<Value, String> {
    check_job(&job)?;
    call(app, &host, "cancel", &[("--job", job)]).await
}

fn server_dir(dir: &str) -> Result<(), String> {
    if dir.starts_with('/') && !dir.chars().any(char::is_control) {
        Ok(())
    } else {
        Err("服务器路径须为绝对路径".into())
    }
}

/// 服务器上一个 history（或算例）目录的概况。
#[tauri::command]
pub async fn bench_remote_inspect(
    app: tauri::AppHandle,
    host: String,
    dir: String,
) -> Result<Value, String> {
    server_dir(&dir)?;
    call(app, &host, "inspect", &[("--dir", dir)]).await
}

/// 服务器上已安装的 OpenBench 有哪些可选指标/评分。
#[tauri::command]
pub async fn bench_remote_metric_options(
    app: tauri::AppHandle,
    host: String,
) -> Result<Value, String> {
    call(app, &host, "metric-options", &[]).await
}

/// 服务器上的登记表。
#[tauri::command]
pub async fn bench_remote_references(app: tauri::AppHandle, host: String) -> Result<Value, String> {
    call(app, &host, "references", &[]).await
}

/// 在服务器的登记表里登记一个参考数据集；字段同本机的 `bench_register`，路径是服务器上的。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn bench_remote_register(
    app: tauri::AppHandle,
    host: String,
    name: String,
    root_dir: String,
    data_type: String,
    tim_res: String,
    years: (i32, i32),
    category: Option<String>,
    fulllist: Option<String>,
    variables: Vec<String>,
) -> Result<Value, String> {
    crate::bench::valid_project(&name)?;
    server_dir(&root_dir)?;
    let request = serde_json::json!({
        "name": name, "root_dir": root_dir, "data_type": data_type, "tim_res": tim_res,
        "years": years, "category": category, "fulllist": fulllist, "variables": variables,
    });
    call(app, &host, "register", &[("--json", request.to_string())]).await
}

/// 在服务器的 OpenBench 模型 profile 里增加一个变量映射。
#[tauri::command]
pub async fn bench_remote_register_model_variable(
    app: tauri::AppHandle,
    host: String,
    model: String,
    variable: String,
    varname: String,
    unit: String,
) -> Result<Value, String> {
    let request = serde_json::json!({
        "model": model, "variable": variable, "varname": varname, "unit": unit,
    });
    call(
        app,
        &host,
        "register-model-variable",
        &[("--json", request.to_string())],
    )
    .await
}

/// 预览服务器上一个参考数据目录能认出哪些。
#[tauri::command]
pub async fn bench_remote_scan(
    app: tauri::AppHandle,
    host: String,
    dir: String,
) -> Result<Value, String> {
    server_dir(&dir)?;
    call(app, &host, "scan", &[("--dir", dir)]).await
}

/// 站点清单发到服务器时的 JSON：字段与 `remote-bench-run --json` 的解析一一对应。
/// 经纬度没有就是 null（远程侧缺省时用 history 文件里的坐标）。
pub(crate) fn sites_payload(sites: &[crate::bench::BenchSite]) -> Vec<Value> {
    sites
        .iter()
        .map(|site| {
            json!({
                "id": site.id.as_str(),
                "station": site.station.as_str(),
                "history_dir": site.history_dir.as_str(),
                "prefix": site.prefix.as_str(),
                "lon": site.lon,
                "lat": site.lat,
            })
        })
        .collect()
}

/// 在服务器上评估：按服务器设置的调度系统提交作业，返回作业号。
#[tauri::command]
pub async fn bench_remote_run(
    app: tauri::AppHandle,
    host: String,
    name: String,
    config: String,
    sites: Option<Vec<crate::bench::BenchSite>>,
    ref_root: Option<String>,
) -> Result<Value, String> {
    crate::bench::valid_project(&name)?;
    if let Some(root) = ref_root.as_deref().filter(|r| !r.trim().is_empty()) {
        server_dir(root)?;
    }
    let sites = sites.unwrap_or_default();
    crate::bench::validate_sites(&sites)?;
    let sites = sites_payload(&sites);
    let server = server_for(app.clone(), &host)?;
    let request = serde_json::json!({
        "name": name, "config": config, "sites": sites,
        "ref_root": ref_root.filter(|r| !r.trim().is_empty()),
    });
    let mut command = args(&server, "run", &[("--json", request.to_string())]);
    command.extend(scheduler_args(&server));
    ssh_cli_json(command, resource_dir(&app), Some(&server)).await
}

/// 取回用的临时目录放在本机镜像的祖父目录（`openbench/`）下：与 `projects/` 同一文件系统，
/// 目录 rename 才不会退化成跨设备复制；不放在 `projects/` 里，是因为那里的每个目录都会被
/// 本机比较列表当成一个项目显示出来，取回中的残缺内容不该出现在列表里。
const FETCH_WORK_DIR: &str = ".fetch";

/// 同一份本机镜像（比较名 + 服务器）同时只允许一次取回。键是最终镜像路径，所以不同
/// host/name 互不影响；这里选择"明确拒绝"而不是等锁 —— 一次取回动辄几十秒（SSH + 打包下载），
/// 在异步运行时线程上阻塞等锁比让用户再点一次更糟。
static FETCHING: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn fetching() -> std::sync::MutexGuard<'static, Vec<String>> {
    // 登记表里只有几次字符串比较；某次取回 panic 毒化锁之后继续用旧表，比让此后所有取回都失败合理。
    FETCHING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 取回期间在登记表里占住这份镜像；Drop 时释放，失败分支不必逐个手写。
#[derive(Debug)]
struct FetchLock {
    key: String,
}

impl Drop for FetchLock {
    fn drop(&mut self) {
        let key = &self.key;
        fetching().retain(|held| held != key);
    }
}

fn lock_fetch(key: &str) -> Result<FetchLock, String> {
    let mut held = fetching();
    if held.iter().any(|current| current == key) {
        return Err("这个比较的报告正在取回中，等这次取回结束后再试。".into());
    }
    held.push(key.to_owned());
    Ok(FetchLock {
        key: key.to_owned(),
    })
}

/// 临时目录名必须唯一：跨进程用 pid + 时间戳，同进程多次取回用递增序号。不能靠
/// "先删同名目录再建"来保证 —— 那会把别的取回或上次崩溃留下的现场一起卷进来。
fn unique_name(role: &str, tag: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("{role}-{tag}-{}-{seq}-{nanos}", std::process::id())
}

fn fetch_work_dir(local: &Path) -> Result<PathBuf, String> {
    let projects = local.parent().ok_or("本机报告路径不完整")?;
    let root = projects.parent().ok_or("本机报告路径不完整")?;
    Ok(root.join(FETCH_WORK_DIR))
}

/// 删文件或目录。镜像理应是目录，但被用户手工动过时也可能是文件或链接；统一处理，
/// 免得清理临时内容时再抛出一个盖住原始错误的二次错误。
fn remove_path(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Windows 上杀毒/索引短暂占用刚下载的文件时，目录 rename 会失败（ERROR_ACCESS_DENIED），
/// 这类失败几百毫秒内自愈；Linux/macOS 的 rename 不受目录内容影响。只重试 rename 本身，
/// 不重试"删旧再写"那类不可回滚的动作。
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                attempt += 1;
                if attempt >= 3 {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(100 * attempt));
            }
        }
    }
}

/// 用 `staged` 整体替换 `target`：先把旧目录 rename 成同文件系统上的备份，再把新目录
/// rename 到位。不用"先删旧目录再 rename"：那会在删除和安装之间留一个失败就丢数据的
/// 窗口；现在任何一步失败都尽量回到原状（新内容不要、旧目录仍在原路径），并且换目录
/// 而不是逐个文件覆盖，旧版本里这次不存在的文件不会残留成混合报告。
fn replace_mirror(staged: &Path, target: &Path) -> Result<(), String> {
    let work = staged.parent().ok_or("取回临时目录不完整")?;
    let tag = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report".into());
    let backup = work.join(unique_name("backup", &tag));
    let had_old = match rename_with_retry(target, &backup) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("无法把本机旧报告移开（{error}）")),
    };
    match rename_with_retry(staged, target) {
        Ok(()) => {
            if had_old {
                let _ = remove_path(&backup);
            }
            Ok(())
        }
        Err(error) => {
            if !had_old {
                return Err(format!("新报告安装失败（{error}）"));
            }
            match rename_with_retry(&backup, target) {
                Ok(()) => Err(format!("新报告替换失败（{error}），本机旧报告已恢复原处")),
                // 同一文件系统上恢复几乎不可能再失败（第一次 rename 已经证明没有占用），
                // 但真失败时宁可把备份留在原地并报出路径，也不能删掉它让旧报告消失。
                Err(restore) => Err(format!(
                    "新报告替换失败（{error}），旧报告恢复也失败（{restore}）；旧报告暂存在 {}，没有删除",
                    backup.display()
                )),
            }
        }
    }
}

/// 一次取回的临时目录：下载写在这里，成功校验后才整体换成本机镜像，失败直接删掉。
/// 从开始到替换之前，本机旧镜像一个字节都不动（旧实现在下载前就 `remove_dir_all`，
/// 连接或认证失败时本机旧报告也一起没了）。
#[derive(Debug)]
struct FetchStaging {
    /// 本机镜像的最终位置：`projects/<比较名>@<服务器>`。
    local: PathBuf,
    /// 本次下载目录：`openbench/.fetch/staging-…`。
    dir: PathBuf,
    /// 取回开始时本机有没有旧镜像：失败提示要说清"旧报告仍在"还是"本来就没有"。
    had_previous: bool,
    /// 登记表占位，最后释放（Drop 先删临时目录，再释放它）。
    _lock: FetchLock,
}

impl FetchStaging {
    fn begin(local: &Path) -> Result<Self, String> {
        let lock = lock_fetch(&local.to_string_lossy())?;
        let projects = local.parent().ok_or("本机报告路径不完整")?;
        let work = fetch_work_dir(local)?;
        std::fs::create_dir_all(projects)
            .map_err(|error| format!("建不了本机报告目录：{error}"))?;
        std::fs::create_dir_all(&work).map_err(|error| format!("建不了取回临时目录：{error}"))?;
        let tag = local
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or("本机报告路径不完整")?;
        let dir = work.join(unique_name("staging", &tag));
        std::fs::create_dir(&dir).map_err(|error| format!("建不了本次取回目录：{error}"))?;
        Ok(Self {
            local: local.to_path_buf(),
            dir,
            had_previous: local.exists(),
            _lock: lock,
        })
    }

    fn dir(&self) -> &Path {
        &self.dir
    }

    /// 下载结束后的统一收尾。失败只清理本次临时内容并返回可诊断错误；成功要再确认服务器
    /// 确实给了一份能显示的报告，然后整体替换 —— 只凭命令退出码就替换正是"半新半旧"的来源。
    fn finish(self, fetched: Result<Value, String>) -> Result<Value, String> {
        let fetched = match fetched {
            Ok(fetched) => fetched,
            Err(error) => return Err(self.failed(&error)),
        };
        if crate::bench::find_report(&self.dir.join("output")).is_none() {
            return Err(self.failed(
                "服务器上这次比较没有找到可显示的报告（output/<项目>/reports/ 下没有 HTML）",
            ));
        }
        replace_mirror(&self.dir, &self.local).map_err(|error| self.failed(&error))?;
        Ok(json!({
            "dir": self.local,
            "report": crate::bench::find_report(&self.local.join("output")),
            "entries": fetched["entries"],
            // C07：取回的是哪一次运行。旧式项目没有 run 身份，三个字段是 null/true。
            "run_id": fetched.get("run_id").cloned().unwrap_or(Value::Null),
            "config_sha": fetched.get("config_sha").cloned().unwrap_or(Value::Null),
            "state": fetched.get("state").cloned().unwrap_or(Value::Null),
            "legacy": fetched.get("legacy").cloned().unwrap_or(Value::Bool(true)),
        }))
    }

    fn failed(&self, detail: &str) -> String {
        // 内层错误可能已经自带句号（例如服务器侧活动作业拒绝的固定句子），这里去掉再拼，
        // 免得中文界面出现「。。」、英文映射多一个句点（GUI-07 服务器侧错误接入时实测）。
        let detail = detail.trim_end_matches(['。', '.']);
        if self.had_previous {
            format!("取回失败：{detail}。本机旧报告未改动，仍是上次成功版本。")
        } else {
            format!("取回失败：{detail}。本机原先没有这个比较的报告。")
        }
    }
}

impl Drop for FetchStaging {
    fn drop(&mut self) {
        // 成功时 dir 已经被 rename 走，这里是 NotFound；失败时删掉本次下载的残缺内容。
        let _ = remove_path(&self.dir);
    }
}

/// 把服务器上一次比较的报告、指标与配置取回本机，返回本机的报告路径。
///
/// 下载到同文件系统的临时目录，确认有报告后才整体替换本机旧镜像：连接、认证、下载或校验
/// 任何一步失败，本机旧报告都原样保留（错误里会说明它仍是上次成功版本），服务器上的原始
/// 报告不受影响。
///
/// 不传 `run_id` 时取回服务器 `latest-success.json` 指向的那次成功运行；传了则取回指定运行
/// （C07：每次运行一个冻结目录，报告只从这次运行的 output 里发现）。
#[tauri::command]
pub async fn bench_remote_fetch(
    app: tauri::AppHandle,
    host: String,
    name: String,
    run_id: Option<String>,
) -> Result<Value, String> {
    let run_id = run_id.filter(|run| !run.trim().is_empty());
    if let Some(run) = &run_id {
        check_run(run)?;
    }
    let local = crate::bench::root()?
        .join("projects")
        .join(mirror_name(&name, &host)?);
    let staging = FetchStaging::begin(&local)?;
    let out = staging.dir().to_string_lossy().into_owned();
    let extra = fetch_extra(&name, &out, run_id.as_deref());
    let fetched = call(app, &host, "fetch", &extra).await;
    staging.finish(fetched)
}

#[cfg(test)]
#[path = "bench_remote_tests.rs"]
mod bench_remote_tests;
