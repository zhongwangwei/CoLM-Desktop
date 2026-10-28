//! 比较页每次运行的身份：`projects/<名字>/runs/<run_id>/` 快照、`meta.json` 状态流转与
//! `latest-success.json` 上次成功指针。
//!
//! 旧实现把 `openbench.yaml` 覆盖写进比较目录、在比较目录里跑，报告再从 `output/` 里找第一份 HTML：
//! A 跑成功留下报告、B 改了设置再跑失败时，目录里就是"B 的配置 + A 的报告"，界面把 A 的报告当成
//! B 的结果（比较页审查报告第 5 条）。现在一次运行一个不可变目录 —— 配置文本、站点清单、输入清单、
//! 报告都在这次运行的目录里；报告只从本次 `output/` 发现；失败只写自己的 meta，上次成功指针
//! 一个字节都不动。
//!
//! 这里只做纯文件系统逻辑，不起进程、不依赖 `AppHandle`：临时目录里就能把整条身份链测完。
//! 单点模拟的合并文件（`sim/<键>.nc`、`sim/<键>.csv`）例外 —— 配置文本里的路径是前端按项目目录
//! 算好的绝对路径（OpenBench 的 check 与 run 对相对路径解析不一致，2026-10-23 窗口实测），
//! 所以它们仍落在 `<项目>/sim/`，由每次 prepare 重写；快照里存站点清单与输入清单，不复制大的 NetCDF。
//!
//! 文件布局：
//! ```text
//! projects/<名字>/
//!   runs/<run_id>/
//!     openbench.yaml        本次实际提交的配置文本（逐字节冻结）
//!     sites.json            提交的站点（id/station/lon/lat）
//!     inputs.json           history 目录、前缀、参考根（输入清单）
//!     sim/<键>.csv           PREPARE 实际生成的站点清单副本
//!     meta.json             run_id/created_at/finished_at/state/config_sha/机器/失败原因
//!     output/               OpenBench 的输出（报告在这里）
//!   latest-success.json     只有成功才更新：run_id、config_sha、report、finished_at
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 每次运行一个目录：`<项目>/runs/<run_id>/`。
pub(crate) const RUNS_DIR: &str = "runs";
/// 本次实际提交的配置文本，冻结在运行目录里。
pub(crate) const CONFIG_FILE: &str = "openbench.yaml";
/// 运行状态记录：先 `running`，收尾时补 `finished_at` 与报告。
pub(crate) const META_FILE: &str = "meta.json";
/// 上次成功的指针，只有成功才原子更新。
pub(crate) const LATEST_SUCCESS_FILE: &str = "latest-success.json";
/// 提交的站点清单副本（id、station、经纬度）。
pub(crate) const SITES_FILE: &str = "sites.json";
/// 输入清单（history 目录、前缀、参考根）。
pub(crate) const INPUTS_FILE: &str = "inputs.json";

/// 失败原因写进 meta 的上限（按字符）：OpenBench 的输出可能很长，meta 只用来诊断，
/// 不能让一份日志把状态文件撑大。
const ERROR_LIMIT: usize = 4_000;

/// 一次运行的状态。取消也是 `failed`（meta 里记"已停止"）：`bench_run` 的返回契约里
/// 只有 running/succeeded/failed 三种，读取的一方不必再记一个新状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RunState {
    Running,
    Succeeded,
    Failed,
}

impl RunState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// 运行在哪台机器、哪个版本上：复盘"报告是不是同一环境生成的"用。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Machine {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub pid: u32,
}

impl Machine {
    fn detect() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            pid: std::process::id(),
        }
    }
}

/// 一次运行提交的站点，与 `bench_run` 的 `sites` 参数一一对应。
/// 经纬度是可选覆盖：PREPARE 给了就用，缺省回退到 history 文件里自带的坐标。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunSite {
    pub id: String,
    pub station: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    pub history_dir: String,
    pub prefix: String,
}

/// 一次运行的 `meta.json`：身份（run_id/时间/机器/配置哈希）+ 输入清单 + 结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunMeta {
    pub run_id: String,
    pub project: String,
    /// ISO-8601 UTC，如 `2026-10-28T13:23:45Z`。
    pub created_at: String,
    /// 同一时刻的无时区秒数：列表排序用，不必解析时间字符串。
    pub created_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_unix: Option<u64>,
    pub state: RunState,
    /// 本次实际提交配置文本的 sha256（十六进制）。前端用它确认"这份报告对应这份配置"。
    pub config_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_root: Option<String>,
    #[serde(default)]
    pub sites: Vec<RunSite>,
    #[serde(default)]
    pub machine: Machine,
    /// 报告相对**运行目录**的路径；只有成功才有。存相对路径，整个 openbench 目录搬家后还能打开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// 失败原因（截断）；成功与进行中没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `latest-success.json`：上次成功运行的身份与报告。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LatestSuccess {
    pub run_id: String,
    pub config_sha: String,
    /// 报告相对**运行目录**（`runs/<run_id>/`）的路径。
    #[serde(default)]
    pub report: Option<String>,
    pub finished_at: String,
    #[serde(default)]
    pub finished_unix: u64,
}

impl LatestSuccess {
    /// 报告的实际路径；文件被人手工删掉就返回 `None`（界面显示"没有报告"而不是一个打不开的路径）。
    pub(crate) fn report_path(&self, project_dir: &Path) -> Option<PathBuf> {
        let relative = self.report.as_deref()?;
        let path = project_dir.join(RUNS_DIR).join(&self.run_id).join(relative);
        path.is_file().then_some(path)
    }

    pub(crate) fn to_json(&self, project_dir: &Path) -> Value {
        json!({
            "run_id": self.run_id,
            "config_sha": self.config_sha,
            "report": self.report_path(project_dir),
            "finished_at": self.finished_at,
        })
    }
}

/// `runs/` 里最新的一次运行（不管成功失败）：状态页显示"最近一次运行"。
pub(crate) struct RunRecord {
    dir: PathBuf,
    meta: RunMeta,
}

impl RunRecord {
    pub(crate) fn meta(&self) -> &RunMeta {
        &self.meta
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// 本次运行自己 `output/` 下记录的报告；失败/进行中的运行没有。
    /// 只认 meta 里记下的相对路径 —— 报告只在运行成功时写进 meta，绝不会指向别的运行。
    pub(crate) fn report_path(&self) -> Option<PathBuf> {
        let relative = self.meta.report.as_deref()?;
        let path = self.dir.join(relative);
        path.is_file().then_some(path)
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "run_id": self.meta.run_id,
            "state": self.meta.state.as_str(),
            "report": self.report_path(),
            "finished_at": self.meta.finished_at,
            "created_at": self.meta.created_at,
            "config_sha": self.meta.config_sha,
            // 失败原因给状态页诊断用（例如"B 的配置 + B 失败了"），成功时是 null。
            "error": self.meta.error,
        })
    }
}

/// 项目是否已经用上快照目录。旧目录（以及服务器取回的本机镜像）没有 `runs/`，
/// 报告继续按项目根 `output/` 兼容发现，不迁移也不破坏。
pub(crate) fn has_runs(project_dir: &Path) -> bool {
    project_dir.join(RUNS_DIR).is_dir()
}

/// 读上次成功指针。指针坏了当作没有：状态页少一行，好过整页读不出来。
pub(crate) fn latest_success(project_dir: &Path) -> Option<LatestSuccess> {
    let text = std::fs::read_to_string(project_dir.join(LATEST_SUCCESS_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 在 `runs/` 里找最新的一次运行。按 `created_unix` 排，同秒再按 run_id（时间戳在前，可读序）。
/// 读不懂的 meta（写了一半、被人改坏）直接跳过，不让一个坏目录把整页状态打挂。
pub(crate) fn latest_run(project_dir: &Path) -> Option<RunRecord> {
    let entries = std::fs::read_dir(project_dir.join(RUNS_DIR)).ok()?;
    let mut latest: Option<RunRecord> = None;
    for entry in entries.filter_map(Result::ok) {
        if !entry.path().is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path().join(META_FILE)) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<RunMeta>(&text) else {
            continue;
        };
        let record = RunRecord {
            dir: entry.path(),
            meta,
        };
        let newer = latest.as_ref().is_none_or(|current| {
            (record.meta.created_unix, &record.meta.run_id)
                > (current.meta.created_unix, &current.meta.run_id)
        });
        if newer {
            latest = Some(record);
        }
    }
    latest
}

/// 一次正在进行的运行。`create` 时就把配置、站点、输入与 `running` 状态的 meta 一起落盘：
/// 进程之后崩溃，留下的也是一个可诊断的快照，而不是"目录里半份别人的东西"。
pub(crate) struct RunSnapshot {
    project_dir: PathBuf,
    dir: PathBuf,
    meta: RunMeta,
}

impl RunSnapshot {
    /// 建立快照目录并冻结输入。`config` 是本次实际要提交的文本，原样写入（不重新格式化）。
    pub(crate) fn create(
        project_dir: &Path,
        project: &str,
        config: &str,
        sites: &[RunSite],
        ref_root: Option<&str>,
    ) -> Result<Self, String> {
        let runs = project_dir.join(RUNS_DIR);
        std::fs::create_dir_all(&runs)
            .map_err(|error| format!("建不了运行目录 {}：{error}", runs.display()))?;
        let created_unix = unix_now();
        let mut run_id = new_run_id(created_unix);
        let mut dir = runs.join(&run_id);
        // 同秒同进程、或进程号被复用时换个序号重试；绝不删已有目录 —— 那是别人的运行。
        let mut attempt = 0;
        loop {
            match std::fs::create_dir(&dir) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                    if attempt >= 64 {
                        return Err(format!("运行目录名一直撞名：{}", dir.display()));
                    }
                    run_id = new_run_id(created_unix);
                    dir = runs.join(&run_id);
                }
                Err(error) => {
                    return Err(format!("建不了运行目录 {}：{error}", dir.display()));
                }
            }
        }
        let config_sha = colm_kernel::sha256_hex(config.as_bytes());
        // 冻结的配置必须逐字节等于提交文本：后面 check/run 用的就是这份文件。
        std::fs::write(dir.join(CONFIG_FILE), config)
            .map_err(|error| format!("写不了冻结的配置：{error}"))?;
        let sites_snapshot = sites
            .iter()
            .map(|site| {
                json!({
                    "id": site.id,
                    "station": site.station,
                    "lon": site.lon,
                    "lat": site.lat,
                })
            })
            .collect::<Vec<_>>();
        write_json(&dir.join(SITES_FILE), &Value::Array(sites_snapshot))?;
        write_json(
            &dir.join(INPUTS_FILE),
            &json!({
                "config_sha": config_sha,
                "ref_root": ref_root,
                "sites": sites
                    .iter()
                    .map(|site| json!({
                        "id": site.id,
                        "history_dir": site.history_dir,
                        "prefix": site.prefix,
                    }))
                    .collect::<Vec<_>>(),
            }),
        )?;
        let meta = RunMeta {
            run_id,
            project: project.to_owned(),
            created_at: iso8601_utc(created_unix),
            created_unix,
            finished_at: None,
            finished_unix: None,
            state: RunState::Running,
            config_sha,
            ref_root: ref_root.map(str::to_owned),
            sites: sites.to_vec(),
            machine: Machine::detect(),
            report: None,
            error: None,
        };
        let snapshot = Self {
            project_dir: project_dir.to_path_buf(),
            dir,
            meta,
        };
        snapshot.write_meta()?;
        Ok(snapshot)
    }

    pub(crate) fn id(&self) -> &str {
        &self.meta.run_id
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn config_sha(&self) -> &str {
        &self.meta.config_sha
    }

    pub(crate) fn config_file(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }

    /// 本次运行自己 `output/` 下的报告。只看这一个运行目录，绝不回退到别的运行。
    pub(crate) fn discover_report(&self) -> Option<PathBuf> {
        crate::bench::find_report(&self.dir.join("output"))
    }

    /// PREPARE 实际生成的站点清单副本（小文件）：它是 OpenBench 真正读到的站点行，
    /// 与 `sites.json`（提交的参数）区分开，复盘时能看出经纬度回退成了什么值。
    pub(crate) fn stash_station_manifest(&self, site_id: &str, csv: &Path) -> Result<(), String> {
        let target = self.dir.join("sim").join(format!("{site_id}.csv"));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("建不了站点清单快照目录：{error}"))?;
        }
        std::fs::copy(csv, &target)
            .map(|_| ())
            .map_err(|error| format!("复制站点清单快照失败（{}）：{error}", csv.display()))
    }

    /// 成功收尾：写自己的 meta，再原子更新 `latest-success.json`。
    /// 指针写不进去时返回错误 —— 报告已经生成了，但界面必须知道"这次没被记成上次成功"。
    pub(crate) fn succeed(&mut self, report: Option<&Path>) -> Result<LatestSuccess, String> {
        let finished_unix = unix_now();
        let finished_at = iso8601_utc(finished_unix);
        let report = report.map(|path| relative_to(&self.dir, path));
        self.meta.state = RunState::Succeeded;
        self.meta.finished_at = Some(finished_at.clone());
        self.meta.finished_unix = Some(finished_unix);
        self.meta.report = report.clone();
        self.meta.error = None;
        self.write_meta()?;
        let latest = LatestSuccess {
            run_id: self.meta.run_id.clone(),
            config_sha: self.meta.config_sha.clone(),
            report,
            finished_at,
            finished_unix,
        };
        write_json(
            &self.project_dir.join(LATEST_SUCCESS_FILE),
            &to_json(&latest)?,
        )?;
        Ok(latest)
    }

    /// 失败/取消收尾：只写自己的 meta；`latest-success.json` 属于上次成功的那次，不动。
    pub(crate) fn fail(&mut self, detail: &str) -> Result<(), String> {
        let finished_unix = unix_now();
        self.meta.state = RunState::Failed;
        self.meta.finished_at = Some(iso8601_utc(finished_unix));
        self.meta.finished_unix = Some(finished_unix);
        self.meta.report = None;
        self.meta.error = Some(truncate(detail, ERROR_LIMIT));
        self.write_meta()
    }

    fn write_meta(&self) -> Result<(), String> {
        write_json(&self.dir.join(META_FILE), &to_json(&self.meta)?)
    }
}

/// 运行目录名：`20261028T132345Z-<pid>-<六位序号>`。UTC 时间戳在前，人一眼能看出先后；
/// 序号补零，名字的字典序才等于创建先后（不补零时 `-10` 会排在 `-9` 前面）。
/// 进程号与序号保证同秒、多进程同时开跑也不会撞名。
fn new_run_id(unix: u64) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{seq:06}", compact_utc(unix), std::process::id())
}

/// 报告在运行目录里记相对路径：整个 `openbench/` 目录搬家后，指针仍然指向报告。
fn relative_to(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out = text.chars().take(limit).collect::<String>();
    out.push('…');
    out
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// UNIX 秒 → UTC 日历分量。不引 chrono：这里只要两种固定格式，民用历公式一次算完。
fn utc_parts(unix: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (unix / 86_400) as i64;
    let seconds = unix % 86_400;
    // Howard Hinnant 的 days → civil 算法：把 400 年当一个纪元处理闰年。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        month as u32,
        day as u32,
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60,
    )
}

pub(crate) fn iso8601_utc(unix: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(unix);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn compact_utc(unix: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(unix);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

fn to_json<T: Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("运行记录序列化失败：{error}"))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("运行记录序列化失败：{error}"))?;
    write_atomic(path, &format!("{text}\n"))
}

/// 临时文件 + rename：读的一方要么看到旧内容、要么看到新内容，不会读到写了一半的 JSON。
/// 临时文件与目标同目录，rename 因此不跨文件系统；写失败时删掉临时文件，目标（旧内容）还在。
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} 没有文件名", path.display()))?;
    let staged = path.with_file_name(format!(
        ".{}.tmp-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|error| format!("写不了 {}：{error}", path.display()))?;
    if let Err(error) = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(&staged);
        return Err(format!("写不了 {}：{error}", path.display()));
    }
    drop(file);
    if let Err(error) = std::fs::rename(&staged, path) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("写不了 {}：{error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "bench_runs_tests.rs"]
mod bench_runs_tests;
