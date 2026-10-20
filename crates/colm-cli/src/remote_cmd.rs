//! 远程运行（docs/design-ai-assistant.md 第 5.3 节，R1）：把一个算例放到服务器上用 Rust 引擎跑，再把结果取回来。
//!
//! ```text
//! colm-cli remote-probe  --host H [--root R]
//! colm-cli remote-run    <case> --host H --root R --kernel <本机内核目录> [--map 本机前缀=服务器前缀]…
//!                        [--stage S] [--force 1] [--threads N] [--upload-unmapped 1]
//!                        [--scheduler auto|bare|slurm|pbs|lsf] [--partition P] [--account A] [--walltime T]
//!                        [--cpus N] [--mem-gb N] [--env-script TEXT] [--directive '-x…']… [--dry-run 1]
//!                        [--engine rust|fortran] [--remote-kernel auto|名字|路径] [--preprocessors rust|fortran]
//!                        [--ranks N] [--nodes N] [--cores-per-node N] [--launcher auto|srun|mpiexec]
//!                        [--model-version official|<修改版>] [--workspace-root DIR]
//! colm-cli remote-kernel --host H --root R --preset P [--env-script TEXT] [--profile production|debug] [--force 1]
//!                        [--model-version <修改版>]   # 用修改版的源码编，内核身份记为 ws-<名>-<提交>
//!                        [调度选项同 remote-run]    在服务器上编 Fortran 内核（R4），登记在 <根>/kernels/<预设>-<标识>
//! colm-cli remote-kernels --host H --root R         列出服务器上的内核
//! colm-cli remote-engine-recover --host H --root R [--older-than 秒]   清掉中断的提交记录与清理锁（只挡清理）
//! colm-cli remote-dist  --host H --root R [--targets x86_64,aarch64] [--out 目录]
//! colm-cli remote-fetch  <case> [--vars a,b] [--from YYYY-MM] [--to YYYY-MM] [--compress N]
//!                         [--overwrite-newer 1]   # 本机结果比这次远程运行新时也覆盖（默认拒绝）
//!                        只取所选变量与月份（先在服务器上裁剪压缩，R5）；不给就取全部
//! colm-cli remote-status <case> [--lines N]
//! colm-cli remote-cancel <case>
//! ```
//!
//! `--dry-run 1` 只生成作业脚本全文（含调度指令）打印出来，不上传、不提交；`--scheduler auto` 先探测服务器有什么
//! 调度系统。
//!
//! 服务器上的布局（都在工作根目录 R 下）：`engine/<快照>/`（源码与编好的四个程序）、`target/`（cargo 编译目录）、
//! `kernels/<清单哈希>/manifest.json`（只放清单，Rust 引擎只用它的宏）、`cases/<名字>-<本机路径哈希>/`
//! （算例副本，输出在其 `out/`）、`jobs/<作业号>/`（作业脚本、日志、阶段、退出码）。
//!
//! 算例副本里所有路径字段都改写成服务器路径：算例目录内的换成副本目录，其余按 `--map` 的前缀对应；对应不上
//! 的默认报错列出来（大数据不悄悄上传），`--upload-unmapped 1` 时把其中的文件随算例一起传。提交记录写在本机
//! 算例的 `.colm-remote.json`，关掉应用也能接着查状态与取结果。取回时只取 `out/<名字>/history` 与日志，不取
//! `stages.json`：它记着服务器上的路径与程序哈希，本机那份保持不变（本机再跑时自己判断要不要重跑）。
//!
//! **修改版**（`--model-version`）：引擎源码换成那个工作区的 `src/`（要求没有未提交的改动，快照就是它的 HEAD），
//! 服务器从源码编出它自己的四个程序（快照号不同，和官方引擎互不覆盖；预编包是官方构建，不能用）。改了 Fortran
//! 又用 Fortran 引擎时，服务器上的内核也要用它的源码编（`remote-kernel --model-version`，身份 `ws-<名>-<提交>`），
//! 官方运行的 `--remote-kernel auto` 不会挑到这种内核。提交记录里带着出处，取回结果时写成本机的
//! `out/<名字>/produced-by.json`（位置记为这台服务器）。

pub(crate) mod remote_cases;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::Value;
use colm_remote::engine::{self, Prebuilt, Snapshot, Source};
use colm_remote::job::{self, Spec};
use colm_remote::sched::{self, Resources, Scheduler};
use colm_remote::ssh::{quote, Ssh};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Opts;

/// 本机路径前缀与服务器路径前缀的一条对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mapping {
    local: String,
    remote: String,
}

fn trim_slash(path: &str) -> &str {
    if path.len() > 1 {
        path.trim_end_matches(['/', '\\'])
    } else {
        path
    }
}

pub(crate) fn parse_mapping(text: &str) -> Result<Mapping> {
    let (local, remote) = text
        .split_once('=')
        .with_context(|| format!("--map needs LOCAL=REMOTE, got {text}"))?;
    ensure!(
        Path::new(local).is_absolute() && remote.starts_with('/'),
        "--map needs two absolute paths, got {text}"
    );
    Ok(Mapping {
        local: trim_slash(local).to_owned(),
        remote: trim_slash(remote).to_owned(),
    })
}

/// 一个路径字段在服务器上的去向。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Placed {
    /// 改写成这个服务器路径。
    Remote(String),
    /// 对应不上的本机路径（存在）。
    Unmapped(PathBuf),
    /// 本机也不存在、又对应不上：保持原样（例如可选输入的占位）。
    Keep,
}

/// `path` 在 `prefix` 之下时返回剩下的部分（按路径分量比，`/a/bc` 不算在 `/a/b` 之下）。
fn under<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with(['/', '\\'])).then_some(rest)
}

/// 决定一个路径字段的值放到服务器的哪里。目录值的结尾分隔符保留（CoLM 直接拼接，少了就错）。
pub(crate) fn place(raw: &str, case: &Path, remote_case: &str, maps: &[Mapping]) -> Placed {
    let trailing = raw.ends_with(['/', '\\']);
    let absolute = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        case.join(raw)
    };
    let local = absolute.to_string_lossy().replace('\\', "/");
    let local = trim_slash(&local).to_owned();
    let case_text = case.to_string_lossy().replace('\\', "/");
    let finish = |remote: String| {
        let remote = remote.replace('\\', "/");
        Placed::Remote(if trailing && !remote.ends_with('/') {
            format!("{remote}/")
        } else {
            remote
        })
    };
    if let Some(rest) = under(&local, trim_slash(&case_text)) {
        return finish(format!("{}{rest}", trim_slash(remote_case)));
    }
    // 最长的前缀优先（嵌套的对应里更具体的那条）。
    let mut maps: Vec<&Mapping> = maps.iter().collect();
    maps.sort_by_key(|m| std::cmp::Reverse(m.local.len()));
    for map in maps {
        if let Some(rest) = under(&local, &map.local) {
            return finish(format!("{}{rest}", map.remote));
        }
    }
    if absolute.exists() {
        Placed::Unmapped(absolute)
    } else {
        Placed::Keep
    }
}

/// Server-native datasets are never interpreted as paths on this computer.
fn server_place(raw: &str, case: &Path, remote_case: &str, maps: &[Mapping]) -> Result<Placed> {
    // Configuration mirrors use native Windows paths, including canonical verbatim prefixes.
    let native_path = |value: &str| {
        let value = value.replace('\\', "/");
        if let Some(rest) = value.strip_prefix("//?/UNC/") {
            format!("//{rest}")
        } else {
            value.strip_prefix("//?/").unwrap_or(&value).to_owned()
        }
    };
    let normalized = native_path(raw);
    let local_case = native_path(&case.to_string_lossy());
    let local = std::iter::once((local_case.as_str(), remote_case))
        .chain(maps.iter().map(|m| (m.local.as_str(), m.remote.as_str())));
    for (prefix, destination) in local {
        if let Some(rest) = under(&normalized, trim_slash(&native_path(prefix))) {
            let remote = format!("{}{rest}", trim_slash(destination));
            validate_server_path(&remote)?;
            return Ok(Placed::Remote(remote));
        }
    }
    if raw.starts_with('/') {
        validate_server_path(raw)?;
        return Ok(Placed::Remote(raw.to_owned()));
    }
    ensure!(
        !raw.contains('\\') && !raw.contains(':') && !raw.split('/').any(|c| c == ".."),
        "invalid relative server input path: {raw}"
    );
    Ok(place(raw, case, remote_case, maps))
}

fn validate_server_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && !path.trim_end_matches('/').is_empty()
            && !path.contains(|c: char| c.is_control() || c == '\\')
            && !path.split('/').any(|c| matches!(c, "." | "..")),
        "invalid absolute server path: {path}"
    );
    Ok(())
}

const CLAIM_SERVER_RUN: &str = r#"
import json, os, sys
base, target, claim = sys.argv[1:]
with open(os.path.join(base, '.colm-desktop-server.json')) as f:
    owner = json.load(f)
if owner.get('schema') != 1 or owner.get('remote_directory') != base:
    raise SystemExit('server project ownership does not match')
if target != base:
    runs = os.path.join(base, 'runs')
    if os.path.islink(runs) or os.path.realpath(os.path.dirname(target)) != os.path.join(os.path.realpath(base), 'runs'):
        raise SystemExit('server run directory escapes project')
    if claim == '1':
        os.makedirs(runs, exist_ok=True)
        os.mkdir(target)
    elif os.path.lexists(target):
        raise SystemExit('server run destination already exists')
"#;

fn check_server_destination(ssh: &Ssh, base: &str, target: &str, claim: bool) -> Result<()> {
    ssh.run_ok(&format!(
        "python3 -c {} {} {} {}",
        quote(CLAIM_SERVER_RUN),
        quote(trim_slash(base)),
        quote(target),
        if claim { "1" } else { "0" }
    ))?;
    Ok(())
}

fn server_destination(base: &str, operation: Option<&str>) -> Result<String> {
    validate_server_path(base)?;
    let base = trim_slash(base);
    match operation {
        Some(id) => {
            job::check_id(id)?;
            Ok(format!("{base}/runs/{id}"))
        }
        None => Ok(base.to_owned()),
    }
}

/// 不随算例上传的东西：输出、日志、阶段指纹、结果标记、远程记录。
fn skipped(name: &str) -> bool {
    name == "out"
        || name.ends_with(".log")
        || name == "stages.json"
        || name == ".colm-results-stale"
        || name == RECORD
        || name.starts_with(".DS_Store")
}

fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    let mut bytes = 0;
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            bytes += copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        bytes +=
            std::fs::copy(from, to).with_context(|| format!("cannot copy {}", from.display()))?;
    }
    Ok(bytes)
}

/// 准备好的算例副本（本机暂存目录）与改写时发现的问题。
#[derive(Debug, Default)]
pub(crate) struct Staged {
    pub bytes: u64,
    /// 改写后指向服务器数据的路径（提交前核对它们在服务器上存在）。
    pub remote_inputs: Vec<String>,
    pub unmapped: Vec<PathBuf>,
    pub uploaded_extra: Vec<PathBuf>,
}

fn looks_like_path(field: &str, raw: &str) -> bool {
    let lower = field.to_ascii_lowercase();
    lower.contains("file")
        || lower.contains("namelist")
        || lower.starts_with("def_dir")
        || lower.contains("path")
        || lower.ends_with("_data")
        || lower == "site_fsitedata"
        || raw.starts_with('/')
}

/// 改写一个 namelist 文件里的全部路径字段。
#[allow(clippy::too_many_arguments)]
fn rewrite_namelist(
    file: &Path,
    case: &Path,
    remote_case: &str,
    maps: &[Mapping],
    upload_unmapped: bool,
    staging: &Path,
    staged: &mut Staged,
    server_inputs: bool,
) -> Result<()> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut document = colm_namelist::parse(&text)?;
    let place_input = |raw: &str| {
        if server_inputs {
            server_place(raw, case, remote_case, maps)
        } else {
            Ok(place(raw, case, remote_case, maps))
        }
    };
    let paths: Vec<String> = document.paths();
    for field in paths {
        let Some(Value::Str(raw)) = document.get(&field).cloned() else {
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("null")
            || !looks_like_path(&field, trimmed)
        {
            continue;
        }
        if field.eq_ignore_ascii_case("DEF_dir_output") {
            document.set(
                &field,
                Value::Str(format!("{}/out/", trim_slash(remote_case))),
            )?;
            continue;
        }
        if field.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES") {
            let rewritten =
                colm_namelist::tracer_files::rewrite_param_files(
                    trimmed,
                    |path| match place_input(path)? {
                        Placed::Remote(remote) => {
                            if under(&remote, trim_slash(remote_case)).is_none() {
                                staged.remote_inputs.push(remote.clone());
                            }
                            Ok(remote)
                        }
                        _ => bail!(
                            "process parameter file {path} is outside the case and not mapped"
                        ),
                    },
                )?;
            document.set(&field, Value::Str(rewritten))?;
            continue;
        }
        // 只改看得出是路径的值（绝对路径或带分隔符），其余（例如文件名前缀）原样。
        if !trimmed.contains(['/', '\\']) {
            continue;
        }
        match place_input(trimmed)? {
            Placed::Remote(remote) => {
                if under(&remote, trim_slash(remote_case)).is_none() {
                    staged.remote_inputs.push(remote.clone());
                }
                document.set(&field, Value::Str(remote))?;
            }
            Placed::Unmapped(local) if upload_unmapped && local.is_file() => {
                let name = local
                    .file_name()
                    .context("input file has no name")?
                    .to_string_lossy()
                    .into_owned();
                let extra = staging.join("inputs");
                std::fs::create_dir_all(&extra)?;
                staged.bytes += std::fs::copy(&local, extra.join(&name))
                    .with_context(|| format!("cannot copy {}", local.display()))?;
                document.set(
                    &field,
                    Value::Str(format!("{}/inputs/{name}", trim_slash(remote_case))),
                )?;
                staged.uploaded_extra.push(local);
            }
            Placed::Unmapped(local) => staged.unmapped.push(local),
            Placed::Keep => {}
        }
    }
    std::fs::write(file, document.to_string())
        .with_context(|| format!("cannot write {}", file.display()))
}

/// 在本机暂存目录里准备服务器用的算例副本。
pub(crate) fn stage_case(
    case: &Path,
    staging: &Path,
    remote_case: &str,
    maps: &[Mapping],
    upload_unmapped: bool,
) -> Result<Staged> {
    stage_case_inputs(case, staging, remote_case, maps, upload_unmapped, false)
}

fn stage_case_inputs(
    case: &Path,
    staging: &Path,
    remote_case: &str,
    maps: &[Mapping],
    upload_unmapped: bool,
    server_inputs: bool,
) -> Result<Staged> {
    let mut staged = Staged::default();
    let mut effective_maps = if server_inputs {
        Vec::new()
    } else {
        maps.to_vec()
    };
    let alias = crate::model_version::execution_path(case)?;
    if alias != case {
        effective_maps.insert(
            0,
            Mapping {
                local: alias.to_string_lossy().into_owned(),
                remote: remote_case.into(),
            },
        );
    }
    let maps = effective_maps.as_slice();
    std::fs::create_dir_all(staging)?;
    for entry in
        std::fs::read_dir(case).with_context(|| format!("cannot read {}", case.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if skipped(&name)
            || (server_inputs
                && matches!(
                    name.as_str(),
                    "runs" | ".colm-desktop.json" | ".colm-desktop-server.json"
                ))
        {
            continue;
        }
        staged.bytes += copy_tree(&entry.path(), &staging.join(&name))?;
    }
    ensure!(
        staging.join("case.nml").is_file(),
        "{} has no case.nml",
        case.display()
    );
    let mut namelists: Vec<PathBuf> = std::fs::read_dir(staging)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nml")))
        .collect();
    namelists.sort();
    for file in namelists {
        rewrite_namelist(
            &file,
            case,
            remote_case,
            maps,
            upload_unmapped,
            staging,
            &mut staged,
            server_inputs,
        )?;
    }
    staged.remote_inputs.sort();
    staged.remote_inputs.dedup();
    Ok(staged)
}

/// 本机算例的提交记录。
pub(crate) const RECORD: &str = ".colm-remote.json";
pub(crate) const PENDING: &str = ".colm-remote-pending.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_auth: Option<colm_remote::auth::Connection>,
    pub host: String,
    pub root: String,
    pub job: String,
    pub remote_case: String,
    pub case_name: String,
    pub engine: String,
    pub submitted_at: u64,
    /// bare、slurm、pbs 或 lsf；老记录没有这一项，当作 bare。
    #[serde(default = "bare")]
    pub scheduler: String,
    /// 调度系统给的作业号（bare 没有）。
    #[serde(default)]
    pub scheduler_id: Option<String>,
    /// 这次运行的出处（版本、引擎、服务器）；取回结果时写成本机的 `produced-by.json`。老记录没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<serde_json::Value>,
}

fn bare() -> String {
    "bare".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingRun {
    schema: u32,
    source_case: PathBuf,
    run_case: PathBuf,
    #[serde(flatten)]
    record: Record,
}

fn check_record(record: &Record) -> Result<()> {
    ensure!(
        !record.host.is_empty()
            && !record.host.starts_with('-')
            && !record
                .host
                .contains(|c: char| c.is_whitespace() || c.is_control()),
        "invalid recorded SSH host"
    );
    job::check_id(&record.job)?;
    job::check_id(&record.engine)?;
    colm_case::validate_case_name(&record.case_name)?;
    ensure!(
        record.root.starts_with('/')
            && record.root != "/"
            && !record.root.contains(char::is_whitespace)
            && Path::new(&record.root).components().all(|c| matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )),
        "invalid recorded remote root"
    );
    ensure!(
        (record
            .remote_case
            .starts_with(&format!("{}/cases/", trim_slash(&record.root)))
            || record
                .provenance
                .as_ref()
                .and_then(|p| p["server_case"].as_str())
                .is_some_and(|base| {
                    validate_server_path(base).is_ok()
                        && (record.remote_case == trim_slash(base)
                            || record
                                .remote_case
                                .strip_prefix(&format!("{}/runs/", trim_slash(base)))
                                .is_some_and(|id| job::check_id(id).is_ok()))
                }))
            && !record.remote_case.contains(char::is_control)
            && Path::new(&record.remote_case)
                .components()
                .all(|c| matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )),
        "remote case is outside its recorded root"
    );
    Ok(())
}

fn source_case(case: &Path) -> Result<PathBuf> {
    let case = case.canonicalize()?;
    if case.join(colm_model::RUN_MANIFEST).is_file() {
        let run: colm_model::MaterializedRun =
            serde_json::from_slice(&std::fs::read(case.join(colm_model::RUN_MANIFEST))?)?;
        ensure!(
            run.run_case.canonicalize()? == case,
            "model run path changed"
        );
        let source = run.source_case.canonicalize()?;
        ensure!(
            source.join("case.nml").is_file(),
            "model source case is missing"
        );
        Ok(source)
    } else {
        Ok(case)
    }
}

fn read_pending(case: &Path) -> Result<Option<PendingRun>> {
    let source = source_case(case)?;
    let bytes = match std::fs::read(source.join(PENDING)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("cannot read remote submission intent"),
    };
    let pending: PendingRun = serde_json::from_slice(&bytes)
        .context("remote submission intent is incomplete; inspect it before any new submission")?;
    ensure!(
        pending.schema == 1 && pending.source_case == source,
        "remote submission intent belongs to another source case"
    );
    ensure!(
        pending.run_case.is_absolute(),
        "pending run path must be absolute"
    );
    check_record(&pending.record)?;
    Ok(Some(pending))
}

pub(crate) fn pending_record(case: &Path) -> Result<Option<serde_json::Value>> {
    Ok(read_pending(case)?.map(serde_json::to_value).transpose()?)
}

fn sync_directory(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    let _ = directory;
    Ok(())
}

fn reserve_pending(pending: &PendingRun) -> Result<()> {
    use std::io::Write;
    check_record(&pending.record)?;
    ensure!(
        pending.source_case == source_case(&pending.run_case)?,
        "pending source case changed"
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(pending.source_case.join(PENDING))
        .context("an earlier remote submission must be reconciled before retrying")?;
    file.write_all(&serde_json::to_vec_pretty(pending)?)?;
    file.sync_all()?;
    sync_directory(&pending.source_case)
}

fn submit_with_intent<T>(pending: &PendingRun, submit: impl FnOnce() -> Result<T>) -> Result<T> {
    reserve_pending(pending)?;
    // Every failure keeps this reservation, including loss of the SSH success response.
    submit()
}

fn atomic_record(case: &Path, record: &Record) -> Result<()> {
    use std::io::Write;
    let temporary = case.join(format!(
        "{RECORD}.{}-{}.tmp",
        record.job,
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(record)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, case.join(RECORD))?;
        sync_directory(case)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn finish_pending(pending: &PendingRun, record: &Record) -> Result<()> {
    ensure!(
        record.job == pending.record.job
            && record.remote_case == pending.record.remote_case
            && record.host == pending.record.host
            && record.root == pending.record.root
            && record.engine == pending.record.engine,
        "submission result differs from saved intent"
    );
    atomic_record(&pending.run_case, record)?;
    if pending.source_case != pending.run_case {
        atomic_record(&pending.source_case, record)?;
    }
    std::fs::remove_file(pending.source_case.join(PENDING))?;
    sync_directory(&pending.source_case)
}

fn refuse_pending(opts: &Opts) -> Result<()> {
    if opts.get("--dry-run").as_deref() == Some("1") {
        return Ok(());
    }
    let source = source_case(&opts.positional_case()?)?;
    if let Some(pending) = read_pending(&source)? {
        bail!("remote submission {} on {} has an unresolved outcome; use remote-status {} before any new run", pending.record.job, pending.record.host, source.display());
    }
    crate::history_cmd::refuse_unresolved_remote_submission(opts, &source)?;
    if let Ok(record) = read_record(&source) {
        if record
            .provenance
            .as_ref()
            .and_then(|value| value["operation_id"].as_str())
            == opts.get("--operation-id").as_deref()
            && opts.get("--operation-id").is_some()
        {
            bail!("this operation already submitted job {}; inspect its status instead of submitting again", record.job);
        }
    }
    Ok(())
}

fn read_record(case: &Path) -> Result<Record> {
    ensure!(
        read_pending(case)?.is_none(),
        "remote submission is pending; inspect remote-status before fetching or cancelling"
    );
    let path = case.join(RECORD);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} has not been run remotely (no {RECORD})", case.display()))?;
    let record: Record = serde_json::from_str(&text)?;
    check_record(&record)?;
    colm_remote::auth::check_connection(record.ssh_auth.as_ref())?;
    Ok(record)
}

fn short_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..8].to_owned()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn root_of(opts: &Opts) -> Result<String> {
    let root = opts.need_str("--root")?;
    ensure!(
        root.starts_with('/') && !root.contains(char::is_whitespace) && root != "/",
        "--root must be an absolute directory on the server without spaces, got {root}"
    );
    Ok(trim_slash(&root).to_owned())
}

/// 引擎源码：开发环境里是这个仓库，安装包里是随应用附带的 `colm-src.tar.gz`。
pub(super) fn engine_source() -> Result<Source> {
    let dir = exe_dir()?;
    engine_source_at(&dir, &engine::resource_dirs(&dir))
}

fn engine_source_at(dir: &Path, resources: &[PathBuf]) -> Result<Source> {
    // A packaged app must use its own source, even when launched inside a checkout.
    if let Some(tarball) = engine::bundled_source(resources) {
        return Ok(Source::Tarball(tarball));
    }
    if let Some(source) = Source::find_checkout(dir) {
        return Ok(source);
    }
    bail!("the engine sources were not found next to colm-cli; this build cannot run remotely")
}

/// Workspace creation needs the original package, not the extracted read-only source cache.
pub(crate) fn workspace_source() -> Result<PathBuf> {
    Ok(match engine_source()? {
        Source::Checkout(path) | Source::Tarball(path) => path,
    })
}

/// Explicit source identity, either a frozen official source or a committed workspace.
pub(crate) struct RemoteVersion {
    pub kind: colm_model::CodeKind,
    pub name: String,
    pub head: String,
    pub src: PathBuf,
    pub touches: colm_workspace::version::Touches,
    /// 出处里 `version` 那一段（和本机运行的 `produced-by.json` 同形）。
    pub value: serde_json::Value,
}

impl RemoteVersion {
    /// 用这个版本的源码在服务器上编的 Fortran 内核，清单里的 `colm_git_sha` 记成它。
    pub fn kernel_identity(&self) -> String {
        match self.kind {
            colm_model::CodeKind::Official => "bundled".into(),
            colm_model::CodeKind::Workspace => {
                format!("ws-{}-{}", self.name, &self.head[..self.head.len().min(8)])
            }
        }
    }
}

pub(crate) fn remote_version(opts: &Opts) -> Result<Option<RemoteVersion>> {
    if let Some(source) = opts.get("--frozen-model-source") {
        ensure!(
            opts.get("--model-version")
                .is_none_or(|name| name == "official"),
            "an official frozen source cannot be combined with a workspace model version"
        );
        return Ok(Some(RemoteVersion {
            kind: colm_model::CodeKind::Official,
            name: "official".into(),
            head: String::new(),
            src: PathBuf::from(source),
            touches: colm_workspace::version::Touches {
                engine: false,
                kernel: false,
            },
            value: super::provenance::official_version_value(),
        }));
    }
    let name = match opts.get("--model-version") {
        None => return Ok(None),
        Some(name) if name == "official" => return Ok(None),
        Some(name) => name,
    };
    let root = super::provenance::workspace_root(opts.get("--workspace-root"));
    let workspace = colm_workspace::Workspace::open(&root, &name)?;
    let src = workspace.src();
    ensure!(
        !colm_workspace::git::is_dirty(&src)?,
        "model version {name} has uncommitted source changes; commit or revert them first, so the server runs a definite commit"
    );
    let head = workspace.head()?;
    let changed = workspace.changes_from_official()?;
    let touches = colm_workspace::version::touches(changed.iter().map(|c| c.path.as_str()));
    let value = json!({
        "kind": "workspace",
        "name": workspace.info.name,
        "label": workspace.info.name,
        "head": head,
        "base_commit": workspace.info.base_commit,
        "origin": workspace.info.origin,
        "note": workspace.info.note,
        "based_on": workspace.info.based_on,
        "touches": touches,
    });
    Ok(Some(RemoteVersion {
        kind: colm_model::CodeKind::Workspace,
        name,
        head,
        src,
        touches,
        value,
    }))
}

/// 这次的引擎源码：修改版用它自己的 `src/`，官方版用应用的源码。
fn source_for(version: Option<&RemoteVersion>) -> Result<Source> {
    match version {
        Some(version) if version.src.is_file() => Ok(Source::Tarball(version.src.clone())),
        Some(version) => Ok(Source::Checkout(version.src.clone())),
        None => engine_source(),
    }
}

/// Source identity excludes Git metadata, bulk datasets and generated build/cache directories.
/// The same relative-path/byte stream is used for checkouts and extracted packages.
pub(crate) fn source_content_sha256(root: &Path) -> Result<String> {
    colm_remote::engine::source_content_sha256(root)
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn unpack_app_source(tarball: &Path, cache: &Path) -> Result<PathBuf> {
    const MARKER: &str = ".colm-source-sha256";
    let digest = file_sha256(tarball)?;
    let dest = cache.join(&digest);
    let complete = |path: &Path| {
        path.join("Cargo.lock").is_file()
            && path.join("Cargo.toml").is_file()
            && std::fs::read_to_string(path.join(MARKER)).is_ok_and(|value| value == digest)
    };
    if complete(&dest) {
        return Ok(dest);
    }
    if dest.exists() {
        ensure!(
            complete(&dest),
            "incomplete source cache {}; preserve it for inspection",
            dest.display()
        );
        return Ok(dest);
    }
    std::fs::create_dir_all(cache)?;
    // macOS 的 `SystemTime` 只到微秒：同一进程的几个线程会拿到同一个时间戳，
    // 所以再加一个进程内计数，暂存目录名才不撞（撞了 `create_dir` 报 EEXIST）。
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = cache.join(format!(
        ".{digest}.{}.{nonce}.{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&staging)?;
    let result = (|| {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(tarball)
            .arg("-C")
            .arg(&staging)
            .status()
            .context("cannot run tar")?;
        ensure!(status.success(), "cannot unpack {}", tarball.display());
        ensure!(
            staging.join("Cargo.lock").is_file() && staging.join("Cargo.toml").is_file(),
            "source package is incomplete"
        );
        ensure!(
            file_sha256(tarball)? == digest,
            "source package changed during extraction"
        );
        std::fs::write(staging.join(MARKER), &digest)?;
        match std::fs::rename(&staging, &dest) {
            Ok(()) => Ok(dest.clone()),
            Err(_) if complete(&dest) => Ok(dest.clone()),
            Err(error) => Err(error.into()),
        }
    })();
    // Only this invocation's private staging directory may be removed.
    if staging.exists() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

/// Installed sources are cached by the full package digest, published only after extraction.
pub(crate) fn app_source_dir() -> Result<PathBuf> {
    match engine_source()? {
        Source::Checkout(repo) => Ok(repo),
        Source::Tarball(tarball) => {
            unpack_app_source(&tarball, &engine::cache_dir().join("source"))
        }
    }
}

/// 这次用哪个引擎：服务器上从源码编，或者传一个预编包。
enum Engine {
    Source(Snapshot),
    Prebuilt(Prebuilt),
}

impl Engine {
    fn id(&self) -> &str {
        match self {
            Self::Source(snapshot) => &snapshot.id,
            Self::Prebuilt(prebuilt) => &prebuilt.id,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Source(_) => "source",
            Self::Prebuilt(_) => "prebuilt",
        }
    }

    /// 传上去（已经在就跳过）；返回这次是不是真的传了。
    fn upload(&self, ssh: &Ssh, root: &str) -> Result<bool> {
        match self {
            Self::Source(snapshot) => {
                let uploaded = engine::upload(ssh, root, snapshot)?;
                prepare_offline(ssh, root, snapshot)?;
                Ok(uploaded)
            }
            Self::Prebuilt(prebuilt) => engine::upload_prebuilt(ssh, root, prebuilt),
        }
    }
}

/// A server without internet gets Rust, CMake and the crates from this computer before anything
/// builds the snapshot there (see `colm_remote::offline`); a connected one is left as it was.
pub(crate) fn prepare_offline(ssh: &Ssh, root: &str, snapshot: &Snapshot) -> Result<()> {
    let lock = engine::lock_text(snapshot)?;
    colm_remote::offline::prepare(ssh, root, &snapshot.id, &lock, &|line| eprintln!("{line}"))?;
    Ok(())
}

fn exe_dir() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .context("colm-cli has no directory")?
        .to_path_buf())
}

/// 选引擎。`--engine-mode`：
/// - `source`：把源码传上去，在服务器上编（要 cargo、cmake 与 C 编译器）；
/// - `prebuilt`：传预编包（`--prebuilt 路径`，或应用随附的、缓存里与当前源码一致的那份）；
/// - `auto`（默认）：已有工具时从源码编，否则优先用预编包；没有预编包时自动安装缺少的 Rust/CMake。
fn choose_engine(
    opts: &Opts,
    ssh: &Ssh,
    root: &str,
    version: Option<&RemoteVersion>,
) -> Result<Engine> {
    let mode = opts.get("--engine-mode").unwrap_or_else(|| "auto".into());
    ensure!(
        ["auto", "source", "prebuilt"].contains(&mode.as_str()),
        "--engine-mode must be auto, source or prebuilt"
    );
    let source = source_for(version).and_then(|s| engine::snapshot(&s));
    if mode == "source" {
        return Ok(Engine::Source(source?));
    }
    let probe = colm_remote::probe::probe(ssh, root)?;
    engine_for_probe(opts, &mode, &ssh.host, &probe, version, source)
}

fn engine_for_probe(
    opts: &Opts,
    mode: &str,
    host: &str,
    probe: &colm_remote::probe::Probe,
    version: Option<&RemoteVersion>,
    source: Result<engine::Snapshot>,
) -> Result<Engine> {
    let can_build = probe.cargo.is_some() && probe.cmake && probe.cc;
    // 预编包是官方构建：修改版只能在服务器上从它自己的源码编。
    if let Some(version) = version.filter(|v| v.kind == colm_model::CodeKind::Workspace) {
        ensure!(
            mode != "prebuilt" && (can_build || probe.toolchain_installable),
            "model version {} has to be built on {} from its own source, which needs cargo, cmake and a C compiler there; \
             a prebuilt engine is the official build and cannot run a modified version",
            version.name,
            host
        );
        return Ok(Engine::Source(source?));
    }
    if mode == "auto" && can_build {
        return Ok(Engine::Source(source?));
    }
    // A failed snapshot must not turn an explicit source into an unpinned official engine.
    let snapshot = source?;
    let prebuilt = match opts.get("--prebuilt") {
        Some(path) => {
            let prebuilt = Prebuilt::open(Path::new(&path))?;
            ensure!(
                prebuilt.matches_source(&probe.arch, &snapshot.id)?,
                "prebuilt engine does not match architecture {} and source snapshot {}",
                probe.arch,
                snapshot.id
            );
            Some(prebuilt)
        }
        None => engine::find_prebuilt(
            &probe.arch,
            Some(&snapshot.id),
            &engine::resource_dirs(&exe_dir()?),
        ),
    };
    match prebuilt {
        Some(prebuilt) => Ok(Engine::Prebuilt(prebuilt)),
        None if mode == "auto" && probe.toolchain_installable => Ok(Engine::Source(snapshot)),
        None => bail!(
            "{} cannot build the engine (cargo, cmake or a C compiler is missing) and no prebuilt engine for {} was found; \
             build one with `colm-cli remote-dist --host <a machine with internet> --root <dir>` or pass --prebuilt <colm-engine-linux-{}.tar.gz>",
            host,
            probe.arch,
            probe.arch
        ),
    }
}

/// 这次运行的引擎与 MPI 设置。
struct RunPlan {
    /// rust 或 fortran。
    engine: String,
    /// 前处理用谁：rust 或 fortran（真正的 Fortran 对照要选 fortran）。
    preprocessors: String,
    ranks: u32,
    nodes: u32,
    /// auto、srun 或 mpiexec。
    launcher: String,
}

impl RunPlan {
    fn from_opts(opts: &Opts) -> Result<Self> {
        let engine = opts.get("--engine").unwrap_or_else(|| "rust".into());
        ensure!(
            ["rust", "fortran"].contains(&engine.as_str()),
            "--engine must be rust or fortran"
        );
        let preprocessors = opts.get("--preprocessors").unwrap_or_else(|| "rust".into());
        ensure!(
            ["rust", "fortran"].contains(&preprocessors.as_str()),
            "--preprocessors must be rust or fortran"
        );
        let ranks: u32 = opts
            .get("--ranks")
            .map(|v| v.parse())
            .transpose()
            .context("--ranks must be a whole number")?
            .unwrap_or(1);
        ensure!(ranks >= 1, "--ranks must be at least 1");
        ensure!(
            engine == "fortran" || ranks == 1,
            "--ranks only applies to --engine fortran (the Rust engine uses threads)"
        );
        let nodes: u32 = opts
            .get("--nodes")
            .map(|v| v.parse())
            .transpose()
            .context("--nodes must be a whole number")?
            .unwrap_or(0);
        let launcher = opts.get("--launcher").unwrap_or_else(|| "auto".into());
        ensure!(
            ["auto", "srun", "mpiexec"].contains(&launcher.as_str()),
            "--launcher must be auto, srun or mpiexec"
        );
        Ok(Self {
            engine,
            preprocessors,
            ranks,
            nodes,
            launcher,
        })
    }
}

pub(super) fn parse_scheduler(opts: &Opts, ssh: &Ssh, root: &str) -> Result<Scheduler> {
    Ok(match opts.get("--scheduler").as_deref() {
        Some("auto") => detected_scheduler(&colm_remote::probe::probe(ssh, root)?)?,
        Some(name) => Scheduler::parse(name)?,
        None => Scheduler::Bare,
    })
}

fn detected_scheduler(probe: &colm_remote::probe::Probe) -> Result<Scheduler> {
    if let Some(name) = probe.schedulers.first() {
        return Scheduler::parse(name);
    }
    // An unidentified qsub may belong to a cluster; never fall back to its login node.
    ensure!(
        !probe.qsub_unverified,
        "qsub was found but its scheduler could not be verified; choose the scheduler explicitly instead of falling back to direct execution"
    );
    Ok(Scheduler::Bare)
}

pub(super) fn resources_from(
    opts: &Opts,
    threads: u32,
    ranks: u32,
    nodes: u32,
) -> Result<Resources> {
    let resources = Resources {
        cpus: opts
            .get("--cpus")
            .map(|v| v.parse())
            .transpose()
            .context("--cpus must be a whole number")?
            .unwrap_or(threads),
        ranks,
        nodes,
        cores_per_node: opts
            .get("--cores-per-node")
            .map(|v| v.parse())
            .transpose()
            .context("--cores-per-node must be a whole number")?
            .unwrap_or(0),
        memory_gb: opts
            .get("--mem-gb")
            .map(|v| v.parse())
            .transpose()
            .context("--mem-gb must be a whole number")?,
        walltime: opts.get("--walltime"),
        partition: opts.get("--partition"),
        account: opts.get("--account"),
        env_script: opts.get("--env-script").filter(|t| !t.trim().is_empty()),
        directives: opts.get_all("--directive"),
    };
    resources.validate()?;
    Ok(resources)
}

/// 等一个作业结束，进度打到 stderr；失败时带上日志末尾。
fn wait_job(ssh: &Ssh, root: &str, id: &str, what: &str, timeout_min: u64) -> Result<()> {
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(20));
        let status = job::status(ssh, root, id, 3)?;
        let last = status.log_tail.lines().last().unwrap_or("").trim();
        eprintln!("  {:?} {}", status.state, last);
        match status.state {
            job::State::Finished { exit_code: 0 } => return Ok(()),
            job::State::Finished { exit_code } => {
                let tail = job::status(ssh, root, id, 40)?.log_tail;
                bail!(
                    "{what} failed (exit code {exit_code}); job {id} on {}:\n{tail}",
                    ssh.host
                )
            }
            job::State::Lost | job::State::Unknown => {
                bail!("the {what} job {id} disappeared on {}", ssh.host)
            }
            _ => {}
        }
        ensure!(
            started.elapsed().as_secs() < timeout_min * 60,
            "{what} is still running after {timeout_min} minutes; job {id} on {} keeps going — check it with ssh",
            ssh.host
        );
    }
}

/// `colm-cli remote-kernels`：服务器上有哪些内核。
pub(super) fn cmd_kernels(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    println!(
        "{}",
        serde_json::to_string(&colm_remote::kernel::list(&ssh, &root)?)?
    );
    Ok(())
}

/// Fortran 源的提交号（本机算，带到服务器上，因为服务器上的快照没有 .git）。
fn fortran_source_sha() -> String {
    Source::find_checkout(&exe_dir().unwrap_or_default())
        .and_then(|source| match source {
            Source::Checkout(repo) => Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["log", "-1", "--format=%h", "--", "vendor/CoLM202X"])
                .output()
                .ok(),
            Source::Tarball(_) => None,
        })
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "bundled".into())
}

/// `colm-cli remote-kernel`：在服务器上从 `vendor/CoLM202X` 编一个 Fortran 内核（要 gfortran、`mpif90`、netCDF-Fortran，
/// 用 `--env-script` 载入，例如 `module load …`）。目录名带源码、环境和预设的哈希，编过的不再重编（`--force 1` 重编）。
pub(super) fn cmd_kernel(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let preset = opts.need_str("--preset")?;
    ensure!(
        !preset.is_empty()
            && preset
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "--preset may only contain letters, digits, - and _"
    );
    let profile = opts.get("--profile").unwrap_or_else(|| "production".into());
    ensure!(
        ["production", "debug"].contains(&profile.as_str()),
        "--profile must be production or debug"
    );
    let force = opts.get("--force").is_some_and(|v| v == "1");
    let threads: u32 = opts
        .get("--threads")
        .map(|t| t.parse())
        .transpose()?
        .unwrap_or(16);
    let scheduler = parse_scheduler(opts, &ssh, &root)?;
    let resources = resources_from(opts, threads, 1, 0)?;
    let version = remote_version(opts)?;
    let snapshot = engine::snapshot(&source_for(version.as_ref())?)?;
    // 修改版的内核在清单里记成 `ws-<名>-<提交>`：它的运行按这个身份认领，官方运行的 auto 不会挑到它。
    let source_sha = match &version {
        Some(version) => version.kernel_identity(),
        None => fortran_source_sha(),
    };
    let name = format!(
        "{preset}-{}",
        short_hash(&format!(
            "{}|{}|{profile}|{}",
            snapshot.id,
            resources.env_script.as_deref().unwrap_or(""),
            source_sha
        ))
    );
    let kdir = format!("{root}/kernels/{name}");
    let have = colm_remote::kernel::list(&ssh, &root)?
        .into_iter()
        .any(|k| k.name == name && k.full);
    if have && !force {
        println!(
            "{}",
            json!({ "kernel": kdir, "name": name, "built": false })
        );
        return Ok(());
    }
    let mut engine_lock = colm_remote::engine_inventory::LifecycleLock::acquire(&ssh, &root)?;
    let (uploaded, id) = engine_lock.run(|| {
        let uploaded = engine::upload(&ssh, &root, &snapshot)?;
        let src = quote(&format!("{}/src", engine::engine_dir(&root, &snapshot.id)));
        let tmp = quote(&format!("{root}/kernels/.build-{name}"));
        let body = format!(
            "echo \"building the Fortran kernel {preset}\" > phase\n\
         export COLM_GIT_SHA={sha} COLM_KERNEL_PROFILE={profile}\n\
         rm -rf {tmp}\n\
         bash {src}/oracle/scripts/build_kernel.sh {preset} {tmp}\n\
         rm -rf {kdir_q}\n\
         mv {tmp}/{preset} {kdir_q}\n\
         rm -rf {tmp}\n\
         echo \"kernel ready\" >> phase\n",
            sha = quote(&source_sha),
            kdir_q = quote(&kdir),
        );
        let id = format!("kernel-{}", now());
        let spec = Spec {
            scheduler,
            resources,
            name: sched::job_name(&format!("k{preset}")),
        };
        job::submit(&ssh, &root, &id, &body, &spec, &snapshot.id)?;
        Ok((uploaded, id))
    })?;
    let lock_warning = engine_lock.finish_submitted(&id);
    eprintln!(
        "building the Fortran kernel {preset} on {} as job {id}{}",
        ssh.host,
        if uploaded { " (sources uploaded)" } else { "" }
    );
    wait_job(&ssh, &root, &id, "the kernel build", 180)?;
    let built = colm_remote::kernel::list(&ssh, &root)?
        .into_iter()
        .find(|k| k.name == name)
        .context("the build finished but the kernel is not on the server")?;
    ensure!(
        built.full,
        "the kernel was registered without all three Fortran programs"
    );
    println!(
        "{}",
        json!({ "kernel": kdir, "name": name, "built": true, "manifest": built, "job": id, "lock_warning": lock_warning })
    );
    Ok(())
}

/// 官方运行能挑的内核：去掉用修改版源码编的（清单身份 `ws-…`）。
pub(crate) fn official_kernels(
    kernels: &[colm_remote::kernel::RemoteKernel],
) -> Vec<colm_remote::kernel::RemoteKernel> {
    kernels
        .iter()
        .filter(|k| !k.colm_git_sha.starts_with("ws-"))
        .cloned()
        .collect()
}

/// 改了 Fortran 的修改版要用自己源码编的内核（清单身份 `ws-<名>-<提交>`）；没有就说明怎么编。
/// 没改 Fortran 的修改版与官方版一样用正式内核（返回 `None`）。
pub(crate) fn version_kernel(
    kernels: &[colm_remote::kernel::RemoteKernel],
    preset: &str,
    version: Option<&RemoteVersion>,
) -> Result<Option<String>> {
    let Some(version) = version.filter(|v| v.touches.kernel) else {
        return Ok(None);
    };
    let identity = version.kernel_identity();
    let own: Vec<_> = kernels
        .iter()
        .filter(|k| k.colm_git_sha == identity)
        .cloned()
        .collect();
    colm_remote::kernel::newest_full(&own, preset)
        .map(|k| Some(k.dir.clone()))
        .with_context(|| {
            format!(
                "model version {} changes Fortran, and the server has no {preset} kernel built from its commit {}; \
                 build it first (colm-cli remote-kernel --preset {preset} --model-version {})",
                version.name,
                &version.head[..version.head.len().min(8)],
                version.name
            )
        })
}

/// `--engine fortran` 要的服务器上的内核：`--remote-kernel auto`（默认）取与本机内核同一预设、最新的完整内核；
/// 也可以给 `<根>/kernels/` 下的名字或绝对路径。
fn resolve_remote_kernel(
    opts: &Opts,
    ssh: &Ssh,
    root: &str,
    preset: &str,
    version: Option<&RemoteVersion>,
) -> Result<String> {
    let kernels = colm_remote::kernel::list(ssh, root)?;
    let wanted = opts.get("--remote-kernel").unwrap_or_else(|| "auto".into());
    if wanted == "auto" {
        if let Some(dir) = version_kernel(&kernels, preset, version)? {
            return Ok(dir);
        }
        let official = official_kernels(&kernels);
        return colm_remote::kernel::newest_full(&official, preset)
            .map(|k| k.dir.clone())
            .with_context(|| {
                format!(
                    "{} has no Fortran kernel for the {preset} preset; build one with \
                     `colm-cli remote-kernel --host {} --root {root} --preset {preset} --env-script '…'`",
                    ssh.host, ssh.host
                )
            });
    }
    let dir = if wanted.contains('/') {
        wanted.clone()
    } else {
        format!("{root}/kernels/{wanted}")
    };
    let found = kernels
        .iter()
        .find(|k| k.dir == dir)
        .with_context(|| format!("{dir} is not a kernel on {}", ssh.host))?;
    ensure!(
        found.full,
        "{dir} has only a manifest; the Fortran engine needs the three programs (colm.x, mkinidata.x, mksrfdata.x)"
    );
    Ok(dir)
}

/// Validate the execution host's selected manifest, never the local preset proxy.
pub(super) fn check_selected_remote_kernel(
    ssh: &Ssh,
    remote_kernel: &str,
    revision: &colm_model::FrozenRevision,
) -> Result<()> {
    let text = ssh.run_ok(&format!(
        "cat -- {}",
        quote(&format!("{remote_kernel}/manifest.json"))
    ))?;
    check_remote_kernel_manifest(&text, remote_kernel, revision)
}

pub(crate) fn check_remote_kernel_manifest(
    text: &str,
    dir: &str,
    revision: &colm_model::FrozenRevision,
) -> Result<()> {
    let manifest: colm_kernel::manifest::Manifest =
        serde_json::from_str(text).context("cannot parse actual remote kernel manifest")?;
    ensure!(
        manifest.schema == colm_kernel::manifest::SCHEMA,
        "unsupported remote kernel manifest schema"
    );
    ensure!(
        manifest
            .macros
            .iter()
            .filter(|name| matches!(name.as_str(), "LULC_IGBP" | "LULC_USGS"))
            .count()
            == 1,
        "remote kernel must declare exactly one land-cover classification"
    );
    crate::model_version::check_kernel(
        revision,
        &colm_kernel::Kernel {
            dir: PathBuf::from(dir),
            manifest,
        },
    )
}

fn materialized_revision(
    case: &Path,
    registry: Option<&Path>,
) -> Result<Option<colm_model::FrozenRevision>> {
    if !case.join(colm_model::RUN_MANIFEST).is_file() {
        return Ok(None);
    }
    let case = case.canonicalize()?;
    let run: colm_model::MaterializedRun =
        serde_json::from_slice(&std::fs::read(case.join(colm_model::RUN_MANIFEST))?)?;
    ensure!(
        run.run_case.canonicalize()? == case,
        "model run belongs to another case"
    );
    let registry = registry
        .or_else(|| case.parent().and_then(Path::parent))
        .context("model run has no registry")?;
    ensure!(
        registry.join("versions").is_dir(),
        "model run registry missing"
    );
    let revision = colm_model::Registry::open(registry)?.get(&run.version)?;
    ensure!(
        revision.code == run.code
            && revision.fingerprint == run.revision_fingerprint
            && revision.overlay_sha256 == run.overlay_sha256,
        "materialized model revision identity changed"
    );
    Ok(Some(revision))
}

fn forcing_policy_guard(cli: &str) -> String {
    format!("colm_forcing_policy=$({cli} fingerprint-policy) || {{ echo 'Update model code: fingerprint-policy is unavailable' >&2; exit 1; }}\n[ \"$colm_forcing_policy\" = '{policy}' ] || {{ echo 'Update model code: forcing content fingerprints must be excluded' >&2; exit 1; }}\n", cli=quote(cli), policy=r#"{"forcing_content_policy":"excluded-v1"}"#)
}

/// 作业体：先确保引擎编好，再跑三段。
#[allow(clippy::too_many_arguments)]
fn job_body(
    root: &str,
    engine_id: &str,
    remote_case: &str,
    remote_kernel: &str,
    threads: u32,
    plan: &RunPlan,
    scheduler: Scheduler,
    opts: &Opts,
) -> Result<String> {
    let mut run = format!(
        "COLM_AUDIT_BY_PARENT=1 COLM_EXECUTION_SNAPSHOT={snapshot} RAYON_NUM_THREADS={threads} {bin}/colm-cli run {case} --kernel {kernel} --stream 1 --engine {engine} --preprocessors {pre}",
        snapshot = quote(engine_id),
        bin = quote(&format!("{}/bin", engine::engine_dir(root, engine_id))),
        case = quote(remote_case),
        kernel = quote(remote_kernel),
        engine = plan.engine,
        pre = plan.preprocessors,
    );
    if plan.ranks > 1 {
        run.push_str(&format!(" --ranks {}", plan.ranks));
    }
    // 取回本机的 `produced-by.json` 要说清结果是在服务器上算的（旧引擎不认这一项，照样忽略）。
    run.push_str(" --location server");
    // MPI 进程怎么起：Slurm 里用 srun（跨节点），其余用 PATH 里的 mpiexec。`colm-cli` 认 COLM_MPIEXEC。
    let srun = match plan.launcher.as_str() {
        "srun" => true,
        "mpiexec" => false,
        _ => scheduler == Scheduler::Slurm,
    };
    let launcher = if plan.ranks > 1 && srun {
        "export COLM_MPIEXEC=srun\n"
    } else {
        ""
    };
    if let Some(stage) = opts.get("--stage") {
        ensure!(
            ["mksrfdata", "mkinidata", "colm"].contains(&stage.as_str()),
            "--stage must be mksrfdata, mkinidata or colm"
        );
        run.push_str(&format!(" --stage {stage}"));
    }
    if opts.get("--force").is_some_and(|v| v == "1") {
        run.push_str(" --force 1");
    }
    if opts
        .get("--record-input-fingerprints")
        .is_some_and(|v| v == "1")
    {
        run.push_str(" --record-input-fingerprints 1");
    }
    Ok(format!(
        "echo \"preparing the engine\" > phase\n{ensure}{policy}{launcher}echo \"running\" >> phase\n{run}\n",
        policy = forcing_policy_guard(&format!("{}/bin/colm-cli", engine::engine_dir(root, engine_id))),
        // 编译的并行度与运行线程数无关：至少 8 个任务。
        ensure = engine::ensure_script(root, engine_id, threads.max(8).clamp(1, 64)),
    ))
}

/// 在一台联网的 Linux 机器上为 `--targets` 预编引擎（glibc 2.17，见 `scripts/build-engine-linux.sh`），取回到本机的
/// 缓存目录（或 `--out`）。之后没有 cargo 或不能联网的服务器用 `remote-run` 的预编包路径就不必再编。
pub(super) fn cmd_dist(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let targets: Vec<String> = opts
        .get("--targets")
        .unwrap_or_else(|| "x86_64".into())
        .split(',')
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    ensure!(!targets.is_empty(), "--targets needs x86_64 and/or aarch64");
    for target in &targets {
        ensure!(
            ["x86_64", "aarch64"].contains(&target.as_str()),
            "unknown target {target}; use x86_64 or aarch64"
        );
    }
    let out = opts
        .get("--out")
        .map(PathBuf::from)
        .unwrap_or_else(engine::cache_dir);
    let snapshot = engine::snapshot(&engine_source()?)?;
    let mut engine_lock = colm_remote::engine_inventory::LifecycleLock::acquire(&ssh, &root)?;
    let (uploaded, id, dist_dir) = engine_lock.run(|| {
    let uploaded = engine::upload(&ssh, &root, &snapshot)?;
    let src = quote(&format!("{}/src", engine::engine_dir(&root, &snapshot.id)));
    let dist_dir = format!("{}/dist", engine::engine_dir(&root, &snapshot.id));
    let id = format!("dist-{}", now());
    let body = format!(
        "echo {sid} > {src}/.colm-snapshot-id\necho \"building prebuilt engines\" > phase\nbash {src}/scripts/build-engine-linux.sh {src} {dist} {tools} {targets}\n",
        sid = snapshot.id,
        dist = quote(&dist_dir),
        tools = quote(&format!("{root}/tools")),
        targets = targets.join(" "),
    );
    job::submit(&ssh, &root, &id, &body, &Spec::bare(), &snapshot.id)?;
        Ok((uploaded, id, dist_dir))
    })?;
    let lock_warning = engine_lock.finish_submitted(&id);
    eprintln!(
        "building prebuilt engines ({}) on {} as job {id}; source snapshot {}{}",
        targets.join(", "),
        ssh.host,
        snapshot.id,
        if uploaded { " (uploaded)" } else { "" }
    );
    wait_job(&ssh, &root, &id, "the engine build", 240)?;
    let mut files = Vec::new();
    for target in &targets {
        let name = engine::prebuilt_name(target);
        ssh.download(&dist_dir, std::slice::from_ref(&name), &out)?;
        let cached = out.join(engine::cached_name(target, &snapshot.id));
        std::fs::rename(out.join(&name), &cached)
            .with_context(|| format!("cannot move {name} into {}", out.display()))?;
        files.push(cached.display().to_string());
    }
    println!(
        "{}",
        json!({ "snapshot": snapshot.id, "files": files, "job": id, "lock_warning": lock_warning })
    );
    Ok(())
}

/// `colm-cli engine-pack --source CHECKOUT --out FILE`：打包指定源码，支持仓库外的 Cargo target。
pub(super) fn cmd_pack(opts: &Opts) -> Result<()> {
    let out = opts.need("--out")?;
    let source = match opts.get("--source") {
        Some(path) => {
            let root = std::fs::canonicalize(&path)
                .with_context(|| format!("cannot read engine source checkout {path}"))?;
            ensure!(
                root.join("Cargo.toml").is_file()
                    && root.join("Cargo.lock").is_file()
                    && root.join("crates/colm-cli").is_dir(),
                "--source must name the engine checkout root"
            );
            Source::Checkout(root)
        }
        None => Source::find_checkout(&exe_dir()?).context(
            "engine-pack needs --source CHECKOUT when its binary is outside the repository",
        )?,
    };
    let snapshot = engine::snapshot(&source)?;
    engine::pack_source(&snapshot, &out)?;
    println!(
        "{}",
        json!({ "out": out, "files": snapshot.files.len(), "snapshot": snapshot.id })
    );
    Ok(())
}

pub(super) fn cmd_probe(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let maps = opts
        .get_all("--map")
        .iter()
        .map(|m| parse_mapping(m))
        .collect::<Result<Vec<_>>>()?;
    let paths: Vec<String> = maps.iter().map(|m| m.remote.clone()).collect();
    let root = if opts.get("--root").is_some() {
        root_of(opts)?
    } else {
        String::new()
    };
    let (mut probe, present) = colm_remote::probe::probe_paths(&ssh, &root, &paths)?;
    // 服务器编不了引擎时，有预编包也能用；两样都没有才算问题。
    if probe.build_problems.is_empty() {
        probe.engine = Some("source".into());
    } else {
        let source = engine_source().and_then(|s| engine::snapshot(&s)).ok();
        let prebuilt = engine::find_prebuilt(
            &probe.arch,
            source.as_ref().map(|s| s.id.as_str()),
            &engine::resource_dirs(&exe_dir()?),
        );
        if prebuilt.is_some() {
            probe.engine = Some("prebuilt".into());
        } else if probe.toolchain_installable {
            probe.engine = Some("source".into());
        } else {
            let hint = format!(
                "no prebuilt engine for {} was found; build one with `colm-cli remote-dist` on a machine with internet",
                probe.arch
            );
            let reasons = probe.build_problems.clone();
            probe.problems.extend(reasons);
            probe.problems.push(hint);
        }
    }
    let mut result = serde_json::to_value(probe)?;
    result["maps"] = json!(maps
        .iter()
        .zip(present)
        .map(|(map, remote_exists)| json!({
            "local": map.local, "remote": map.remote,
            "local_exists": Path::new(&map.local).exists(), "remote_exists": remote_exists
        }))
        .collect::<Vec<_>>());
    println!("{result}");
    Ok(())
}

/// Content identities of existing source versions protect their remote caches.
fn inventory_versions(opts: &Opts) -> (Vec<serde_json::Value>, Vec<String>) {
    let mut versions = Vec::new();
    let mut blockers = Vec::new();
    match engine_source().and_then(|source| engine::snapshot(&source)) {
        Ok(snapshot) => {
            versions.push(json!({"name":"official", "snapshot":snapshot.id, "dirty":false}))
        }
        Err(error) => blockers.push(format!("Cannot verify official source snapshot: {error}")),
    }
    let root = super::provenance::workspace_root(opts.get("--workspace-root"));
    let result = inventory_workspace_versions(&root).map(|entries| versions.extend(entries));
    if let Err(error) = result {
        blockers.push(format!("Cannot verify workspace source snapshots: {error}"));
    }
    if opts.get("--model-root").is_some() {
        match crate::model_version::protected_snapshots(opts) {
            Ok(entries) => versions.extend(entries),
            Err(error) => blockers.push(format!("Cannot verify frozen model references: {error}")),
        }
    }
    (versions, blockers)
}

fn inventory_workspace_versions(root: &Path) -> Result<Vec<serde_json::Value>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry?;
        ensure!(
            !entry.file_type()?.is_symlink(),
            "workspace entry is a symbolic link: {}",
            entry.path().display()
        );
        if !entry.file_type()?.is_dir() {
            continue;
        }
        match std::fs::symlink_metadata(entry.path().join("workspace.json")) {
            Ok(metadata) => ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "invalid workspace metadata"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("workspace name is not UTF-8"))?;
        let workspace = colm_workspace::Workspace::open(root, &name)?;
        workspace.summary()?;
        let snapshot = engine::snapshot(&Source::Checkout(workspace.src()))?;
        versions.push(json!({"name":name, "snapshot":snapshot.id, "dirty":colm_workspace::git::is_dirty(&workspace.src())?}));
    }
    Ok(versions)
}

fn cleanup_result(
    mut result: serde_json::Value,
    unlocked: Result<()>,
) -> Result<serde_json::Value> {
    if result.get("removed").is_some() {
        if let Err(error) = unlocked {
            result["warning"] = json!(format!(
                "Engine removed, but cleanup lock remains: {error:#}"
            ));
        }
        Ok(result)
    } else {
        let unlock_warning = unlocked
            .err()
            .map(|error| format!("; cleanup lock remains: {error:#}"))
            .unwrap_or_default();
        bail!(
            "engine cleanup refused: {}{unlock_warning}",
            result["refused"]
        )
    }
}

pub(super) fn cmd_engines(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let (versions, blockers) = inventory_versions(opts);
    let mut result = colm_remote::engine_inventory::list(&ssh, &root)?;
    for item in result["engines"]
        .as_array_mut()
        .context("invalid engine inventory")?
    {
        let names: Vec<_> = versions
            .iter()
            .filter(|v| v["snapshot"] == item["id"])
            .map(|v| v["name"].clone())
            .collect();
        let reasons = item["protected_reasons"]
            .as_array_mut()
            .context("invalid protection reasons")?;
        reasons.extend(blockers.iter().map(|s| json!(s)));
        for name in &names {
            reasons.push(json!(format!(
                "Matches current source version {}",
                name.as_str().unwrap_or_default()
            )));
        }
        item["can_clean"] = json!(reasons.is_empty());
        item["versions"] = json!(names);
    }
    result["cleanup_blockers"]
        .as_array_mut()
        .context("invalid cleanup blockers")?
        .extend(blockers.into_iter().map(|s| json!(s)));
    result["versions"] = json!(versions);
    println!("{result}");
    Ok(())
}

/// `remote-engine-recover`：清掉中断留下的提交记录与清理锁（超过 `--older-than` 秒，默认一小时，至少 10 分钟）。
/// 只由人在计算资源页确认后调用；它们只挡引擎清理，不挡提交。
pub(super) fn cmd_engine_recover(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let older_than: u64 = opts
        .get("--older-than")
        .map(|v| v.parse())
        .transpose()
        .context("--older-than must be a whole number of seconds")?
        .unwrap_or(3600);
    let result = colm_remote::engine_inventory::recover(&ssh, &root, older_than)?;
    println!("{result}");
    Ok(())
}

pub(super) fn cmd_engine_clean(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let snapshot = opts.need_str("--snapshot")?;
    ensure!(
        snapshot.len() == 16
            && snapshot
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid engine snapshot"
    );
    let (versions, blockers) = inventory_versions(opts);
    ensure!(
        blockers.is_empty(),
        "engine cleanup refused: {}",
        blockers.join("; ")
    );
    ensure!(
        !versions
            .iter()
            .any(|v| v["snapshot"].as_str() == Some(&snapshot)),
        "engine cleanup refused: snapshot matches an existing source version"
    );
    let mut lock = colm_remote::engine_inventory::LifecycleLock::acquire_exclusive(&ssh, &root)?;
    let result = lock.clean(&snapshot);
    // A failed removal may have been interrupted; keep the lock in that case.
    let result = result?;
    let result = cleanup_result(result, lock.finish())?;
    println!("{result}");
    Ok(())
}

pub(super) fn cmd_run(opts: &Opts) -> Result<()> {
    refuse_pending(opts)?;
    let source = source_case(&opts.positional_case()?)?;
    let profile = source.join(".colm-desktop.json");
    if profile.is_file() {
        let profile: serde_json::Value = serde_json::from_slice(&std::fs::read(profile)?)?;
        if let Some(directory) = profile["remote_directory"].as_str() {
            ensure!(
                profile["run_host"].as_str() == opts.get("--host").as_deref(),
                "server-native case belongs to another host"
            );
            ensure!(
                Some(trim_slash(directory)) == opts.get("--server-case").as_deref().map(trim_slash),
                "server-native case requires its saved server project directory"
            );
        }
    }
    if let Some((prepared, run)) = crate::model_version::prepare(opts)? {
        ensure!(
            opts.get("--upload-unmapped").as_deref() != Some("1"),
            "model revision runs require mapped server datasets; bulk upload is not allowed"
        );
        let _ = run;
        return cmd_run(&prepared);
    }
    let case = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let plan = RunPlan::from_opts(opts)?;
    // Rust 引擎只用本机内核的清单（宏），所以必须给 --kernel；Fortran 引擎用服务器上的完整内核，
    // 本机只需要知道预设名（--preset，或从 --kernel 的清单读）。
    let local_kernel = match opts.get("--kernel") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            let kernel = colm_kernel::Kernel::open_manifest(&dir)?;
            Some((dir, kernel))
        }
        None => None,
    };
    ensure!(
        local_kernel.is_some() || plan.engine == "fortran",
        "--kernel is required\n{}",
        "(only --engine fortran can do without it, using --preset and a kernel built by remote-kernel)"
    );
    let preset = match (&local_kernel, opts.get("--preset")) {
        (_, Some(preset)) => preset,
        (Some((_, kernel)), None) => kernel.manifest.preset.clone(),
        (None, None) => bail!("--preset is required when there is no --kernel"),
    };
    let maps = opts
        .get_all("--map")
        .iter()
        .map(|m| parse_mapping(m))
        .collect::<Result<Vec<_>>>()?;
    let threads: u32 = opts
        .get("--threads")
        .map(|t| t.parse())
        .transpose()?
        .unwrap_or(8);
    let upload_unmapped = opts.get("--upload-unmapped").is_some_and(|v| v == "1");
    let dry_run = opts.get("--dry-run").is_some_and(|v| v == "1");
    let scheduler = parse_scheduler(opts, &ssh, &root)?;
    let resources = resources_from(opts, threads, plan.ranks, plan.nodes)?;

    let case_name = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?
        .get("DEF_CASE_NAME")
        .and_then(|v| match v {
            Value::Str(s) => Some(s.trim().to_owned()),
            _ => None,
        })
        .context("case.nml has no DEF_CASE_NAME")?;
    colm_case::validate_case_name(&case_name)?;
    let server_case = opts.get("--server-case");
    let remote_case = if let Some(base) = &server_case {
        let operation = if case.join(colm_model::RUN_MANIFEST).is_file() {
            Some(
                case.file_name()
                    .and_then(|n| n.to_str())
                    .context("invalid run folder")?,
            )
        } else {
            None
        };
        server_destination(base, operation)?
    } else {
        format!(
            "{root}/cases/{case_name}-{}",
            short_hash(&case.to_string_lossy())
        )
    };

    // 1. 算例副本：在本机暂存、改写路径，对应不上的先报出来。
    let local_configuration = crate::fingerprint::configuration_identity(&case)?;
    let staging =
        std::env::temp_dir().join(format!("colm-remote-{}-{}", case_name, std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let staged = stage_case_inputs(
        &case,
        &staging,
        &remote_case,
        &maps,
        upload_unmapped,
        server_case.is_some(),
    )?;
    if !staged.unmapped.is_empty() {
        let _ = std::fs::remove_dir_all(&staging);
        let list: Vec<String> = staged
            .unmapped
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        bail!(
            "these inputs are on this computer but have no server path; add a path mapping for them \
             (or allow uploading the files): {}",
            list.join(", ")
        );
    }

    let remote_model_run = if case.join(colm_model::RUN_MANIFEST).is_file() {
        let mut run: colm_model::MaterializedRun =
            serde_json::from_slice(&std::fs::read(case.join(colm_model::RUN_MANIFEST))?)?;
        run.run_case = PathBuf::from(&remote_case);
        for path in &mut run.artifact_paths {
            *path = PathBuf::from(&remote_case).join(
                path.strip_prefix(&case)
                    .context("model artifact escapes run case")?,
            );
        }
        run.effective_nml_sha256 =
            colm_kernel::sha256_hex(&std::fs::read(staging.join("case.nml"))?);
        for flag in ["--highres-params", "--soil-hyper-albedo-dir"] {
            if run.inputs.contains_key(&format!("execution-option:{flag}")) {
                bail!("remote spectral data options require an explicit server input mapping");
            }
        }
        std::fs::write(
            staging.join(colm_model::RUN_MANIFEST),
            serde_json::to_vec_pretty(&run)?,
        )?;
        let _ = std::fs::remove_file(staging.join("full-input-identity.json"));
        Some(serde_json::to_value(run)?)
    } else {
        None
    };

    if let Some(base) = &server_case {
        check_server_destination(&ssh, base, &remote_case, false)?;
    }

    // 2. 核对服务器上的数据在场。
    if !staged.remote_inputs.is_empty() {
        let checks: Vec<String> = staged
            .remote_inputs
            .iter()
            .map(|p| format!("[ -e {q} ] || echo {q}", q = quote(p)))
            .collect();
        let missing = ssh.run_ok(&checks.join("\n"))?;
        if !missing.trim().is_empty() {
            let _ = std::fs::remove_dir_all(&staging);
            bail!(
                "these inputs are missing on {}: {}",
                ssh.host,
                missing.trim().lines().collect::<Vec<_>>().join(", ")
            );
        }
    }

    // 3. 引擎源码、内核清单、算例副本上传。
    let version = remote_version(opts)?;
    let engine = choose_engine(opts, &ssh, &root, version.as_ref())?;
    if let Some(expected) = opts.get("--expected-engine") {
        ensure!(engine.id() == expected, "replay source snapshot changed");
    }
    let fortran = plan.engine == "fortran";
    let manifest = match &local_kernel {
        Some((dir, _)) => std::fs::read_to_string(dir.join("manifest.json"))?,
        None => String::new(),
    };
    let remote_kernel = if fortran {
        resolve_remote_kernel(opts, &ssh, &root, &preset, version.as_ref())?
    } else {
        format!("{root}/kernels/{preset}-{}", short_hash(&manifest))
    };
    if let Some(revision) =
        materialized_revision(&case, opts.get("--model-root").as_deref().map(Path::new))?
    {
        if fortran {
            check_selected_remote_kernel(&ssh, &remote_kernel, &revision)?;
        } else {
            check_remote_kernel_manifest(&manifest, &remote_kernel, &revision)?;
        }
    }
    if dry_run {
        let _ = std::fs::remove_dir_all(&staging);
        let body = job_body(
            &root,
            engine.id(),
            &remote_case,
            &remote_kernel,
            threads,
            &plan,
            scheduler,
            opts,
        )?;
        let spec = Spec {
            scheduler,
            resources,
            name: sched::job_name(&case_name),
        };
        let script = job::job_script(&root, "JOBID", &body, &spec)?;
        println!(
            "{}",
            json!({
                "dry_run": true,
                "scheduler": scheduler.name(),
                "job_script": script,
                "remote_case": remote_case,
                "engine": engine.id(),
                "engine_kind": engine.kind(),
            })
        );
        return Ok(());
    }
    let id = format!("run-{}", opts.need_str("--operation-id")?);
    // 多站点批量运行带上批次号：分区独占时同一批的站点放进同一个节点队列（`colm_remote::pack`），
    // 不再一个站点占一个节点。分区是否独占由服务器上的提交脚本判断。
    let pack = opts
        .get("--pack-batch")
        .map(|batch| colm_remote::pack::Pack {
            pool: batch.to_owned(),
            // 服务器设置里填了每节点核数就按它放站点（摆渡船 48）；没填时由服务器上的提交脚本自己估。
            slots: opts
                .get("--cores-per-node")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        });
    let raw_body = job_body(
        &root,
        engine.id(),
        &remote_case,
        &remote_kernel,
        threads,
        &plan,
        scheduler,
        opts,
    )?;
    let spec = Spec {
        scheduler,
        resources,
        name: sched::job_name(&case_name),
    };
    let body = format!(
        "# colm-run-case: {}\n# colm-operation-id: {}\n{raw_body}",
        serde_json::to_string(&remote_case)?,
        opts.need_str("--operation-id")?
    );
    let mut provenance = json!({
        "schema": 1,
        "operation_id": opts.get("--operation-id"),
        "started": now(),
        "version": match &version {
            Some(version) => version.value.clone(),
            None => super::provenance::official_version_value(),
        },
        "engine": plan.engine,
        "preprocessors": plan.preprocessors,
        "ranks": plan.ranks,
        "stage": opts.get("--stage"),
        "force": opts.get("--force").is_some_and(|v| v == "1"),
        "record_input_fingerprints": opts.get("--record-input-fingerprints").is_some_and(|v| v == "1"),
        "location": { "kind": "server", "host": ssh.host },
        "kernel": match &local_kernel {
            Some((_, kernel)) if !fortran => json!({ "dir": remote_kernel, "identity": kernel.manifest.identity(), "preset": preset }),
            _ => json!({ "dir": remote_kernel, "preset": preset }),
        },
        "engine_snapshot": engine.id(),
    });
    crate::model_version::attach(&case, &mut provenance)?;
    provenance["server_case"] = serde_json::to_value(&server_case)?;
    provenance["local_configuration_sha256"] = serde_json::to_value(&local_configuration)?;
    provenance["remote_model_run"] = serde_json::to_value(remote_model_run)?;
    provenance["remote_options"] = serde_json::to_value(
        opts.flags
            .iter()
            .filter(|(key, _)| replay_remote_flag(key))
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    provenance["replay_kernel"] =
        serde_json::to_value(local_kernel.as_ref().map(|(path, _)| path))?;
    let record = Record {
        ssh_auth: colm_remote::auth::current_connection(),
        host: ssh.host.clone(),
        root: root.clone(),
        job: id.clone(),
        remote_case: remote_case.clone(),
        case_name,
        engine: engine.id().to_owned(),
        submitted_at: now(),
        scheduler: scheduler.name().to_owned(),
        scheduler_id: None,
        provenance: Some(provenance),
    };
    let pending = PendingRun {
        schema: 1,
        source_case: source_case(&case)?,
        run_case: case.clone(),
        record: record.clone(),
    };
    // 提交前先在登录节点把引擎编好：天河的计算节点没有 cc，作业里编引擎会在链接时失败（2026-10-19，
    // 软件更新后的一批站点全部这样失败）。放在记下提交意图之前：编不成时这次运行直接报错，不会留下待核对。
    if matches!(engine, Engine::Source(_)) {
        let mut lock = colm_remote::engine_inventory::LifecycleLock::acquire(&ssh, &root)?;
        let built = lock.run(|| {
            engine.upload(&ssh, &root)?;
            remote_cases::build_on_login_node(&ssh, &root, &engine)
        });
        let unlocked = lock.finish();
        built?;
        unlocked?;
    }
    let (engine_lock, engine_uploaded, submitted) = submit_with_intent(&pending, || {
        crate::history_cmd::observe_remote_outcome(
            &json!({"job":id,"outcome":"uncertain","terminal":false,"remote_record":pending}),
        );
        crate::history_cmd::record_remote_intent(
            opts,
            &serde_json::to_value(&pending)?,
            &pending.source_case,
        )?;
        let mut engine_lock = colm_remote::engine_inventory::LifecycleLock::acquire(&ssh, &root)?;
        let (engine_uploaded, submitted) = engine_lock.run(|| {
    let engine_uploaded = engine.upload(&ssh, &root)?;
    if !fortran {
        ssh.run_ok(&format!(
            "mkdir -p {dir} && cat > {dir}/manifest.json <<'COLM_MANIFEST_EOF'\n{manifest}\nCOLM_MANIFEST_EOF\n",
            dir = quote(&remote_kernel)
        ))?;
    }
    let entries: Vec<String> = std::fs::read_dir(&staging)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    if let Some(base) = &server_case {
        check_server_destination(&ssh, base, &remote_case, true)?;
    }
    ssh.upload(&staging, &entries, &remote_case)?;
    let _ = std::fs::remove_dir_all(&staging);

    // 4. 提交作业：先确保引擎编好，再跑。
    ensure!(crate::fingerprint::configuration_identity(&case)? == local_configuration, "local frozen configuration changed during remote staging");
    let submitted = job::submit_with(&ssh, &root, &id, &body, &spec, engine.id(), pack.as_ref())?;
        Ok((engine_uploaded, submitted))
    })?;
        Ok((engine_lock, engine_uploaded, submitted))
    })?;
    let mut record = record;
    record.scheduler = submitted.scheduler.to_owned();
    record.scheduler_id = submitted.scheduler_id.clone();
    let record_warning = finish_pending(&pending, &record)
        .err()
        .map(|e| format!("Job {id} submitted, but local record could not be saved: {e}"));
    let lock_warning = engine_lock.finish_submitted(&id);
    crate::history_cmd::observe_remote_outcome(
        &json!({"job":id,"outcome":"submitted","terminal":false,"remote_record":record}),
    );
    if opts.get("--quiet-submit-json").is_none() {
        println!(
            "{}",
            json!({
                "job": id,
                "pid": submitted.pid,
                "scheduler": submitted.scheduler,
                "scheduler_id": submitted.scheduler_id,
                "host": ssh.host,
                "remote_case": remote_case,
                "engine": engine.id(),
                "engine_kind": engine.kind(),
                "engine_uploaded": engine_uploaded,
                "case_bytes": staged.bytes,
                "record_warning": record_warning,
                "lock_warning": lock_warning,
                "remote_inputs": staged.remote_inputs,
                "uploaded_extra": staged.uploaded_extra,
            })
        );
    }
    Ok(())
}

fn pending_identity_script(pending: &PendingRun) -> Result<String> {
    check_record(&pending.record)?;
    let operation = pending
        .record
        .provenance
        .as_ref()
        .and_then(|value| value["operation_id"].as_str())
        .context("pending submission has no operation identity")?;
    job::check_id(operation)?;
    let markers = [
        format!("# colm-engine-snapshot: {}", pending.record.engine),
        format!(
            "# colm-run-case: {}",
            serde_json::to_string(&pending.record.remote_case)?
        ),
        format!("# colm-operation-id: {operation}"),
    ];
    let checks = markers.iter().map(|marker| format!("grep -Fqx -- {} script.sh || {{ echo 'pending job identity changed' >&2; exit 1; }}", quote(marker))).collect::<Vec<_>>().join("\n");
    Ok(format!("set -e\n{prelude}cd {dir} 2>/dev/null || {{ echo 'identity=missing'; exit 0; }}\n[ -f script.sh ] && [ ! -L script.sh ] || {{ echo 'identity=incomplete'; exit 0; }}\n{checks}\nS=$(cat scheduler 2>/dev/null || echo bare)\ncase \"$S\" in {scheduler}|{scheduler}-pack) ;; *) echo 'pending scheduler identity changed' >&2; exit 1;; esac\nif [ \"$S\" = bare ]; then\n  ID=$(cat pid 2>/dev/null || true)\n  case \"$ID\" in ''|*[!0-9]*) echo 'identity=incomplete'; exit 0;; esac\nelse\n  ID=$(cat sched_id 2>/dev/null || true)\n{adopt}  case \"$ID\" in ''|*[!A-Za-z0-9._-]*) echo 'identity=incomplete'; exit 0;; esac\nfi\necho 'identity=verified'\necho \"$ID\"\necho \"$S\"\n", dir=quote(&job::job_dir(&pending.record.root,&pending.record.job)),scheduler=quote(&pending.record.scheduler),adopt=ADOPT_SLURM_JOB,prelude=colm_remote::sched::PATH_PRELUDE))
}

/// 待核对的 Slurm 提交没有 `sched_id` 时，按作业目录去 Slurm 里找一次，恰好一个就记下。天河 2026-10-19：
/// sbatch 报 "Unexpected message received" 退出非零，作业其实已被收下并跑完了；当时的提交脚本没有按目录
/// 找，这 5 个站点就一直"待核对"，下一次批量又把它们当成身份变了。作业目录只属于这一次提交，所以按目录
/// 找到的就是它；找到多个或一个都没有，仍然不认。
const ADOPT_SLURM_JOB: &str = r#"  if [ -z "$ID" ] && [ "$S" = slurm ]; then
    ID=$( { squeue -h -u "$(id -un)" -o '%i|%Z' 2>/dev/null; sacct -n -X -P -u "$(id -un)" -S now-14days -o JobID,WorkDir 2>/dev/null; } | awk -F'|' -v w="$(pwd -P)" '$2 == w { print $1 }' | sort -u)
    case "$ID" in ''|*[!A-Za-z0-9._-]*) ID= ;; *) echo "$ID" > sched_id ;; esac
  fi
"#;

fn cmd_pending_status(opts: &Opts, pending: PendingRun, lines: usize) -> Result<()> {
    colm_remote::auth::check_connection(pending.record.ssh_auth.as_ref())?;
    let ssh = Ssh::new(&pending.record.host)?;
    let identity = ssh.run_ok(&pending_identity_script(&pending)?)?;
    let verified = identity.lines().next() == Some("identity=verified");
    let status = if verified {
        job::status(&ssh, &pending.record.root, &pending.record.job, lines)?
    } else {
        job::Status {
            state: job::State::Unknown,
            phase: None,
            detail: Some("submission has no complete server identity; do not resubmit".into()),
            log_tail: String::new(),
        }
    };
    let ended = job::ended(&status);
    let outcome = json!({"job":pending.record.job,"outcome":"observed","terminal":matches!(status.state,job::State::Finished{..}) || ended.is_some(),"ended":ended,"status":status});
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&pending)?, &outcome)?;
    let mut record = pending.record.clone();
    if verified {
        // 打包提交在记下意图时记的是基础调度系统；以服务器上写下的为准（`bare-pack` 也有作业号）。
        let packed = identity
            .lines()
            .nth(2)
            .filter(|s| *s == format!("{}-pack", record.scheduler))
            .map(str::to_owned);
        if let Some(actual) = packed {
            record.scheduler = actual;
            record.scheduler_id = identity.lines().nth(1).map(str::to_owned);
        } else if record.scheduler != "bare" {
            record.scheduler_id = identity.lines().nth(1).map(str::to_owned);
        }
    }
    let record_warning = if verified {
        finish_pending(&pending, &record)
            .err()
            .map(|error| error.to_string())
    } else {
        None
    };
    let unresolved = !verified || record_warning.is_some();
    println!(
        "{}",
        json!({"record":if unresolved {serde_json::Value::Null}else{serde_json::to_value(&record)?},"pending":if unresolved {json!({"host":record.host,"root":record.root,"job":record.job,"remote_case":record.remote_case,"source_case":pending.source_case,"run_case":pending.run_case,"needs_reconciliation":true})}else{serde_json::Value::Null},"status":status,"ended":ended,"record_warning":record_warning})
    );
    Ok(())
}

pub(super) fn cmd_status(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    let lines: usize = opts
        .get("--lines")
        .map(|l| l.parse())
        .transpose()?
        .unwrap_or(60);
    if let Some(pending) = read_pending(&case)? {
        return cmd_pending_status(opts, pending, lines);
    }
    let record = read_record(&case)?;
    let status = job::status(&Ssh::new(&record.host)?, &record.root, &record.job, lines)?;
    let ended = job::ended(&status);
    let outcome = json!({"job":record.job,"outcome":"observed","terminal":matches!(status.state, job::State::Finished {..}) || ended.is_some(),"ended":ended,"status":status});
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &outcome)?;
    println!(
        "{}",
        json!({ "record": record, "status": status, "ended": ended })
    );
    Ok(())
}

/// 结果页看服务器上的运行：在服务器上用这次运行的引擎读它的 history，只把结果（变量表约 15 KB、
/// 一条曲线约 70 KB）传回来，不搬 NetCDF。天河实测 AU-Cpr（84 个月文件，457 MB）：变量表第一次 9 s、
/// 之后约 1 s，一条曲线 3–7 s（2026-10-19）。只放行只读的结果命令。
pub(super) fn cmd_read(opts: &Opts) -> Result<()> {
    let command = opts.need_str("--command")?;
    ensure!(
        matches!(
            command.as_str(),
            "history-catalog" | "series" | "energy-summary" | "history-map"
        ),
        "remote-read only reads results (history-catalog, series, energy-summary, history-map), not {command}"
    );
    let case = opts.positional_case()?;
    let record = read_record(&case)?;
    let mut args = vec![command, record.remote_case.clone()];
    for flag in [
        "--vars",
        "--from",
        "--to",
        "--max-points",
        "--var",
        "--index",
    ] {
        if let Some(value) = opts.get(flag) {
            args.extend([flag.to_owned(), value]);
        }
    }
    print!(
        "{}",
        Ssh::new(&record.host)?.run_ok(&read_script(&record.root, &record.engine, &args))?
    );
    Ok(())
}

/// 读结果用服务器上最新编好的引擎，没有才用这次运行自己的引擎：读 history 的代码只增不改（格点的
/// 区域平均、`history-map` 是 2026-10-20 才有的），旧运行的引擎读不出格点图。
fn read_script(root: &str, run_engine: &str, args: &[String]) -> String {
    let own = format!("{}/bin/colm-cli", engine::engine_dir(root, run_engine));
    let engines = format!("{}/engine", root.trim_end_matches('/'));
    format!(
        "B=$(ls -td {engines}/*/bin/colm-cli 2>/dev/null | head -n 1)
[ -n \"$B\" ] && [ -x \"$B\" ] || B={own}
exec \"$B\" {args}\n",
        engines = quote(&engines),
        own = quote(&own),
        args = args
            .iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// 服务器上正在编的引擎编到哪一步（给界面的进度条）；只读，几十字节。
pub(super) fn cmd_engine_progress(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = opts.need_str("--root")?;
    let progress = engine::parse_progress(&ssh.run_ok(&engine::progress_script(&root))?);
    println!("{}", serde_json::to_string(&progress)?);
    Ok(())
}

/// 服务器忙不忙：登录节点负载、各分区占用、排队情况和自己的作业（`colm_remote::load`）。只读。
pub(super) fn cmd_load(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    println!(
        "{}",
        serde_json::to_string(&colm_remote::load::load(&ssh)?)?
    );
    Ok(())
}

pub(super) fn cmd_cancel(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    let record = read_record(&case)?;
    job::check_id(&record.job)?;
    let ssh = Ssh::new(&record.host)?;
    let marker = quote(&format!("# colm-engine-snapshot: {}", record.engine));
    let mut identity = format!("set -e\ncd {}\ngrep -Fqx -- {marker} script.sh || {{ echo 'engine identity changed' >&2; exit 1; }}\n[ \"$(cat scheduler 2>/dev/null || echo bare)\" = {} ] || {{ echo 'scheduler identity changed' >&2; exit 1; }}\n", quote(&job::job_dir(&record.root, &record.job)), quote(&record.scheduler));
    if record.scheduler != "bare" {
        let id = record
            .scheduler_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .context("recorded scheduler job ID is missing")?;
        identity.push_str(&format!("[ \"$(cat sched_id)\" = {} ] || {{ echo 'scheduler job identity changed' >&2; exit 1; }}\n", quote(id)));
    }
    let result = remote_study::cancel_job(&ssh, &record.root, &record.job, &identity)?;
    crate::history_cmd::observe_remote_outcome(&result);
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &result)?;
    println!("{result}");
    Ok(())
}

/// 本机算例里记着“这份结果只取回了一部分”的标记；全部取回后删掉。
pub(crate) const FETCH_MARK: &str = ".colm-fetch.json";

/// 取回结果：`out/<名字>/history` 与三段的日志，放到本机算例的同一位置；清掉本机的“结果已过期”标记。
///
/// 不给 `--vars`、`--from`、`--to` 就取全部 history。给了就先在服务器上用 `colm-cli history-subset`
/// 裁成只含所选变量与月份的压缩小文件，再只把这些取回来（R5）：经慢链路时取回量与耗时随变量数线性下降。
/// 部分取回会在算例里留一个 `.colm-fetch.json`，说明取回了什么；同名文件会被覆盖，不在所选月份里的旧文件不动。
/// 本机 history 里最新的文件修改时间（Unix 秒）；没有 history 时为 `None`。
fn newest_local_history(case: &Path, case_name: &str) -> Option<u64> {
    let dir = super::Layout::new(case)
        .out()
        .join(case_name)
        .join("history");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "nc"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .max()
}

/// 取回的目标是不是这次作业自己的运行目录（`runs/<作业号>`，记录里有 `model_run`）。是的话里面的历史
/// 只可能是这个作业上一次取回写进去的，再取回一次就是用服务器的同一份结果覆盖同一份，不需要拦。
/// 只有取回到别的、可能被本机另外跑过的目录（直接写进源算例的旧做法）时才要保护本机更新的结果。
pub(crate) fn fetches_into_its_own_run_directory(provenance: Option<&serde_json::Value>) -> bool {
    provenance
        .and_then(|p| p.get("model_run"))
        .is_some_and(|run| run.is_object())
}

/// 本机结果比这次远程运行还新（提交之后本机又跑过），取回会覆盖它：除非明确要求，否则拒绝。
/// 界面的自动取回不带 `--overwrite-newer`，手动点“取回”才带（审查第 2 条的复查：恢复任务时本机还没有结果，
/// 之后本机跑完了，原来记下的“可以自动取回”不能再算数）。
pub(crate) fn refuse_to_overwrite_newer(
    newest_local: Option<u64>,
    submitted_at: u64,
    overwrite: bool,
) -> Result<()> {
    if let Some(local) = newest_local.filter(|_| !overwrite) {
        ensure!(
            local <= submitted_at,
            "local results are newer than this remote job (local history written after the job was submitted); \
             fetch with --overwrite-newer 1 to replace them with the server's"
        );
    }
    Ok(())
}

type OutputHashes = std::collections::BTreeMap<String, String>;

fn recorded_history_hashes(provenance: &serde_json::Value) -> Result<OutputHashes> {
    let outputs = provenance["output_sha256"]
        .as_object()
        .context("completion has no original output hashes")?;
    let mut history = OutputHashes::new();
    for (name, hash) in outputs {
        let name = name.replace('\\', "/");
        ensure!(
            Path::new(&name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "invalid recorded output path: {name}"
        );
        let hash = hash.as_str().context("invalid recorded output hash")?;
        ensure!(
            hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()),
            "invalid recorded output hash: {name}"
        );
        if name.starts_with("history/") && Path::new(&name).extension().is_some_and(|v| v == "nc") {
            ensure!(
                history.insert(name.clone(), hash.to_owned()).is_none(),
                "duplicate recorded history path: {name}"
            );
        }
    }
    ensure!(
        !history.is_empty(),
        "completion has no original history hashes"
    );
    Ok(history)
}

fn history_inventory(root: &Path) -> Result<std::collections::BTreeMap<PathBuf, String>> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut std::collections::BTreeMap<PathBuf, String>,
    ) -> Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
            "history contains a symlink or special file: {}",
            path.display()
        );
        out.insert(
            path.strip_prefix(root)?.to_path_buf(),
            crate::fingerprint::stable_file_identity(&metadata),
        );
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path)? {
                visit(root, &entry?.path(), out)?;
            }
        }
        Ok(())
    }
    let mut inventory = std::collections::BTreeMap::new();
    visit(root, root, &mut inventory)?;
    Ok(inventory)
}

fn verify_history_hashes(root: &Path, expected: &OutputHashes) -> Result<()> {
    let before = history_inventory(root)?;
    let actual: OutputHashes = crate::provenance::output_identities(root)?
        .into_iter()
        .map(|(name, hash)| (format!("history/{}", name.replace('\\', "/")), hash))
        .collect();
    ensure!(
        before == history_inventory(root)?,
        "history changed during output verification"
    );
    ensure!(
        &actual == expected,
        "history differs from original completed output hashes"
    );
    Ok(())
}

// Only the explicit new policy can skip content checks; missing legacy metadata cannot.
fn content_fingerprints_disabled(provenance: &serde_json::Value) -> bool {
    provenance["verification"]["status"] == "disabled"
        && provenance["verification"]["input_fingerprints"] == false
        && provenance["verification"]["content_fingerprints"] == false
}

fn unhashed_history_files(root: &Path) -> Result<std::collections::BTreeSet<PathBuf>> {
    let inventory = history_inventory(root)?;
    let files: std::collections::BTreeSet<_> = inventory
        .keys()
        .filter(|p| p.extension().is_some_and(|ext| ext == "nc") && root.join(p).is_file())
        .cloned()
        .collect();
    ensure!(!files.is_empty(), "completion has no history files");
    Ok(files)
}

fn unhashed_proof_files(proof: &serde_json::Value) -> Result<std::collections::BTreeSet<PathBuf>> {
    let files: std::collections::BTreeSet<PathBuf> =
        serde_json::from_value(proof["history_files"].clone())?;
    ensure!(
        !files.is_empty()
            && files.iter().all(|p| p
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
                && p.extension().is_some_and(|ext| ext == "nc")),
        "invalid unverified history file paths"
    );
    Ok(files)
}

fn verify_fetched_history(root: &Path, proof: &serde_json::Value) -> Result<()> {
    if content_fingerprints_disabled(&proof["provenance"]) {
        ensure!(
            unhashed_history_files(root)? == unhashed_proof_files(proof)?,
            "downloaded history paths differ from server completion"
        );
        Ok(())
    } else {
        verify_history_hashes(root, &recorded_history_hashes(&proof["provenance"])?)
    }
}

/// Read-only proof from the CLI bound to the job's source snapshot.
pub(crate) fn verify_completed_outputs(case: &Path) -> Result<serde_json::Value> {
    let layout = colm_case::Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let out = layout.out().join(&name);
    let path = out.join(crate::provenance::FILE);
    let before = std::fs::symlink_metadata(&path)?;
    ensure!(
        before.is_file() && !before.file_type().is_symlink(),
        "completion provenance is not a regular file"
    );
    let identity = crate::fingerprint::stable_file_identity(&before);
    let bytes = std::fs::read(&path)?;
    let provenance: serde_json::Value = serde_json::from_slice(&bytes)?;
    if content_fingerprints_disabled(&provenance) {
        let history_files = unhashed_history_files(&out.join("history"))?;
        let inventory = history_inventory(&out.join("history"))?;
        ensure!(
            identity
                == crate::fingerprint::stable_file_identity(&std::fs::symlink_metadata(&path)?)
                && bytes == std::fs::read(&path)?,
            "completion provenance changed during output inspection"
        );
        return Ok(
            json!({"schema":1,"case_name":name,"provenance":provenance,"history_files":history_files,"history_inventory":inventory,"provenance_identity":identity}),
        );
    }
    let digest = crate::fingerprint::full_file(&path, "provenance")?.sha256;
    let hashes = recorded_history_hashes(&provenance)?;
    let inventory = history_inventory(&out.join("history"))?;
    verify_history_hashes(&out.join("history"), &hashes)?;
    ensure!(
        identity == crate::fingerprint::stable_file_identity(&std::fs::symlink_metadata(&path)?)
            && digest == crate::fingerprint::full_file(&path, "provenance")?.sha256,
        "completion provenance changed during output verification"
    );
    ensure!(
        inventory == history_inventory(&out.join("history"))?,
        "history changed during completion verification"
    );
    Ok(
        json!({"schema":1,"case_name":name,"provenance":provenance,"history_sha256":hashes,"history_inventory":inventory,"provenance_identity":identity}),
    )
}

fn parse_output_verification(
    output: colm_remote::ssh::Output,
) -> Result<Option<serde_json::Value>> {
    if !output.success {
        // An old CLI can provide diagnostic downloads, never scientific certification.
        if output.code == Some(1)
            && output.stderr.lines().any(|line| {
                line.trim().strip_prefix("Error: ").unwrap_or(line.trim())
                    == "unknown command: verify-completed-outputs"
            })
        {
            return Ok(None);
        }
        bail!(
            "server completed-output verification failed: {}",
            output.stderr.trim()
        );
    }
    let proof: serde_json::Value = serde_json::from_str(output.stdout.trim())
        .context("invalid server completed-output proof")?;
    ensure!(proof["schema"] == 1, "unsupported completed-output proof");
    ensure!(
        proof["history_inventory"].is_object() && proof["provenance_identity"].is_string(),
        "server proof has no stable output identities"
    );
    if content_fingerprints_disabled(&proof["provenance"]) {
        unhashed_proof_files(&proof)?;
        return Ok(Some(proof));
    }
    let hashes: OutputHashes = serde_json::from_value(proof["history_sha256"].clone())?;
    ensure!(
        hashes == recorded_history_hashes(&proof["provenance"])?,
        "server proof differs from original completion hashes"
    );
    Ok(Some(proof))
}

fn remote_output_verification(ssh: &Ssh, record: &Record) -> Result<Option<serde_json::Value>> {
    let cli = format!(
        "{}/bin/colm-cli",
        engine::engine_dir(&record.root, &record.engine)
    );
    let proof = parse_output_verification(ssh.run(&format!(
        "COLM_AUDIT_BY_PARENT=1 {} verify-completed-outputs {}",
        quote(&cli),
        quote(&record.remote_case)
    ))?)?;
    if let Some(proof) = &proof {
        verify_submitted_output_identity(proof, record)?;
    }
    Ok(proof)
}

fn verify_submitted_output_identity(proof: &serde_json::Value, record: &Record) -> Result<()> {
    ensure!(
        proof["case_name"].as_str() == Some(record.case_name.as_str()),
        "server proof belongs to another case"
    );
    let submitted = record
        .provenance
        .as_ref()
        .context("remote submission provenance is missing")?;
    let remote = &proof["provenance"];
    ensure!(
        remote["execution_snapshot"].as_str() == Some(record.engine.as_str()),
        "server source snapshot differs from submitted engine"
    );
    if content_fingerprints_disabled(remote) {
        ensure!(
            submitted["record_input_fingerprints"] == false,
            "server disabled fingerprints without an explicit disabled submission policy"
        );
        ensure!(
            submitted["remote_model_run"].is_object() && remote["model_run"].is_object(),
            "disabled fingerprint fetch requires a bound model identity"
        );
    }
    // Legacy runs retain output integrity checks, but cannot certify scientific identity.
    if submitted["remote_model_run"].is_null() && remote["model_run"].is_null() {
        return Ok(());
    }
    ensure!(
        submitted["remote_model_run"].is_object()
            && remote["model_run"] == submitted["remote_model_run"],
        "server model identity differs from submitted revision"
    );
    if content_fingerprints_disabled(remote) {
        return Ok(());
    }
    let full: crate::fingerprint::FullInputIdentity =
        serde_json::from_value(remote["full_input_identity"].clone())?;
    ensure!(
        full.complete
            && (full.algorithm == "sha256-full-v1"
                || (full.algorithm == "sha256-nonforcing-v1"
                    && full
                        .data_settings
                        .get("forcing_content_policy")
                        .map(String::as_str)
                        == Some("excluded-v1")
                    && full.unresolved.is_empty())),
        "server input content identity is incomplete or has an unsupported scope"
    );
    Ok(())
}

struct FetchStage(PathBuf);
impl FetchStage {
    fn new(case: &Path) -> Result<Self> {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = case.join(format!(".fetch-{}-{suffix}", std::process::id()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for FetchStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fetch_certification(
    fetched: &mut serde_json::Value,
    proof: Option<&serde_json::Value>,
    partial: bool,
) {
    fetched["execution_ok"] = fetched["ok"].clone();
    fetched["execution_error"] = fetched["error"].clone();
    if let Some(proof) = proof {
        let remote = &proof["provenance"];
        for field in [
            "full_input_identity",
            "verification",
            "input_sources",
            "input_fingerprint",
            "kernel",
            "execution_program_sha256",
            "output_sha256",
            "execution_snapshot",
        ] {
            if let Some(value) = remote.get(field) {
                fetched[field] = value.clone();
            } else if let Some(fields) = fetched.as_object_mut() {
                fields.remove(field);
            }
        }
        fetched["remote_execution_provenance"] = remote.clone();
    }
    if !partial
        && proof.is_some_and(|p| {
            content_fingerprints_disabled(&p["provenance"])
                && p["provenance"]["model_run"].is_object()
        })
    {
        let remote = &proof.unwrap()["provenance"];
        fetched["verification"] = remote["verification"].clone();
        fetched["output_verification"] = json!({"status":"disabled","partial":false,"reason":"content fingerprint recording was disabled; output paths and presence were checked"});
        if remote["ok"] != true {
            fetched["ok"] = json!(false);
            fetched["error"] = json!("server execution provenance is not successful");
        }
        return;
    }
    let reason = if partial {
        Some("partial history is derived data, not the original completed output")
    } else if proof.is_none() {
        Some("the bound server CLI cannot verify original completed outputs")
    } else if proof.is_some_and(|p| !p["provenance"]["model_run"].is_object()) {
        Some("legacy execution has no frozen model identity for scientific verification")
    } else {
        None
    };
    let verified = reason.is_none();
    fetched["output_verification"] = json!({"status":if verified {"passed"} else {"unavailable"},"partial":partial,"reason":reason});
    if proof.is_some_and(|p| {
        p["provenance"]["full_input_identity"]["algorithm"] == "sha256-nonforcing-v1"
    }) {
        fetched["verification"]["status"] = json!("partial");
        fetched["verification"]["forcing_contents_verified"] = json!(false);
        fetched["output_verification"]["forcing_contents_verified"] = json!(false);
    }
    if let Some(reason) = reason {
        fetched["ok"] = json!(false);
        fetched["error"] = json!(reason);
    } else if proof.is_some_and(|p| p["provenance"]["ok"] != true) {
        fetched["ok"] = json!(false);
        fetched["error"] = json!("server execution provenance is not successful");
    }
}

pub(super) fn cmd_fetch(opts: &Opts) -> Result<()> {
    let case = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let record = read_record(&case)?;
    let case = crate::model_version::recorded_run_case(&case, record.provenance.as_ref())?;
    let overwrite = opts.get("--overwrite-newer").is_some_and(|v| v == "1")
        || fetches_into_its_own_run_directory(record.provenance.as_ref());
    refuse_to_overwrite_newer(
        newest_local_history(&case, &record.case_name),
        record.submitted_at,
        overwrite,
    )?;
    let ssh = Ssh::new(&record.host)?;
    let history = format!("out/{}/history", record.case_name);
    let vars = opts.get("--vars");
    let from = opts.get("--from");
    let to = opts.get("--to");
    let partial = vars.is_some() || from.is_some() || to.is_some();
    let listing = ssh.run_ok(&format!(
        "cd {} && for e in {} mksrfdata.log mkinidata.log colm.log; do [ -e \"$e\" ] && echo \"$e\"; done",
        quote(&record.remote_case),
        quote(&history)
    ))?;
    let mut entries: Vec<String> = listing
        .lines()
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
        .collect();
    ensure!(
        entries.iter().any(|e| e == &history),
        "the server has no history for {} yet (the run may not have finished)",
        record.case_name
    );

    let proof = remote_output_verification(&ssh, &record)?;
    let stage = FetchStage::new(&case)?;
    let staged_history = stage.0.join(&history);
    let mut subset = serde_json::Value::Null;
    let mut subset_files = 0;
    if partial {
        // 先在服务器上裁剪，之后 history 目录不整个取，只取裁出来的文件。
        let tmp = format!(
            "{}/{}-subset",
            record.remote_case,
            stage.0.file_name().unwrap().to_string_lossy()
        );
        let bin = format!(
            "{}/bin/colm-cli",
            engine::engine_dir(&record.root, &record.engine)
        );
        let mut command = format!(
            "rm -rf {tmp} && {bin} history-subset {case} --out {tmp}",
            tmp = quote(&tmp),
            bin = quote(&bin),
            case = quote(&record.remote_case)
        );
        for (flag, value) in [("--vars", &vars), ("--from", &from), ("--to", &to)] {
            if let Some(value) = value {
                ensure!(
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_,-".contains(c)),
                    "{flag} may only contain letters, digits, _ , and -"
                );
                command.push_str(&format!(" {flag} {}", quote(value)));
            }
        }
        if let Some(level) = opts.get("--compress") {
            ensure!(
                level.len() == 1 && level.chars().all(|c| c.is_ascii_digit()),
                "--compress must be 0-9"
            );
            command.push_str(&format!(" --compress {level}"));
        }
        let out = ssh.run_ok(&command).map_err(|e| {
            anyhow::anyhow!(
                "{e}\n(the engine that ran this case may be too old to subset history; run the case again, or fetch everything by leaving out --vars/--from/--to)"
            )
        })?;
        subset = serde_json::from_str(out.trim().lines().last().unwrap_or("{}"))?;
        let files = ssh.run_ok(&format!("ls -1 {}", quote(&tmp)))?;
        let files: Vec<String> = files.lines().map(str::to_owned).collect();
        ensure!(!files.is_empty(), "the server produced no subset files");
        ensure!(
            files.iter().all(|name| !name.starts_with('-')
                && Path::new(name).components().count() == 1
                && Path::new(name)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                && Path::new(name).extension().is_some_and(|v| v == "nc")),
            "invalid subset filename"
        );
        subset_files = files.len();
        entries.retain(|e| e != &history);
        ssh.download(&record.remote_case, &entries, &stage.0)?;
        ssh.download(&tmp, &files, &staged_history)?;
        let _ = ssh.run_ok(&format!("rm -rf {}", quote(&tmp)));
        history_inventory(&staged_history)?;
    } else {
        ssh.download(&record.remote_case, &entries, &stage.0)?;
        if let Some(proof) = &proof {
            verify_fetched_history(&staged_history, proof)?;
        } else {
            history_inventory(&staged_history)?;
        }
    }
    if let Some(proof) = &proof {
        ensure!(
            remote_output_verification(&ssh, &record)?.as_ref() == Some(proof),
            "server completion changed during fetch"
        );
    }
    refuse_to_overwrite_newer(
        newest_local_history(&case, &record.case_name),
        record.submitted_at,
        overwrite,
    )?;
    if partial && case.join(&history).exists() {
        history_inventory(&case.join(&history))?;
    }
    // Invalidate existing certification before publishing any replacement bytes.
    let mut fetched = fetched_provenance(
        record
            .provenance
            .as_ref()
            .unwrap_or(&json!({"schema":1,"location":{"kind":"server","host":record.host}})),
        job::status(&ssh, &record.root, &record.job, 0)
            .ok()
            .map(|s| s.state),
    );
    fetch_certification(&mut fetched, proof.as_ref(), partial);
    let mut publishing = fetched.clone();
    publishing["ok"] = json!(false);
    publishing["error"] = json!("fetch publication is not complete");
    publishing["output_verification"] =
        json!({"status":"unavailable","reason":"fetch publication is not complete"});
    super::provenance::write_value(&case, &publishing);
    ensure!(
        super::provenance::read(&case)?.is_some_and(|p| p["ok"] == false),
        "cannot invalidate previous output certification"
    );
    if partial {
        if let Err(error) = publish_partial_history(
            &staged_history,
            &case.join(&history),
            &stage.0.join("replaced-history"),
        ) {
            if error.to_string().contains(PARTIAL_ROLLBACK_FAILED) {
                // Keep the only intact copies of the replaced files.
                std::mem::forget(stage);
            }
            return Err(error);
        }
        std::fs::write(
            case.join(FETCH_MARK),
            serde_json::to_string_pretty(&json!({
                "vars": vars,
                "from": from,
                "to": to,
                "files": subset_files,
                "fetched_at": now(),
                "scientific_verified": false,
                "original_history_sha256": proof.as_ref().map(|p| &p["history_sha256"]),
            }))?,
        )?;
    } else {
        let destination = case.join(&history);
        std::fs::create_dir_all(destination.parent().context("history parent is missing")?)?;
        let backup = stage.0.join("previous-history");
        let had_previous = destination.exists();
        if had_previous {
            std::fs::rename(&destination, &backup)?;
        }
        if let Err(error) = std::fs::rename(&staged_history, &destination) {
            if had_previous {
                if let Err(restore) = std::fs::rename(&backup, &destination) {
                    // Keep the only intact copy if the filesystem refuses rollback.
                    std::mem::forget(stage);
                    bail!("cannot restore previous history after fetch failure ({error}; {restore}); original data remain at {}", backup.display());
                }
            }
            return Err(error.into());
        }
        let _ = std::fs::remove_file(case.join(FETCH_MARK));
    }
    for name in ["mksrfdata.log", "mkinidata.log", "colm.log"] {
        let path = stage.0.join(name);
        if path.is_file() {
            std::fs::rename(path, case.join(name))?;
        }
    }
    let publication_inventory = history_inventory(&case.join(&history))?;
    if !partial {
        if let Some(proof) = &proof {
            verify_fetched_history(&case.join(&history), proof)?;
        }
    }
    super::provenance::write_value(&case, &fetched);
    let publication = (|| -> Result<()> {
        ensure!(
            publication_inventory == history_inventory(&case.join(&history))?,
            "history changed during fetch publication"
        );
        let written =
            super::provenance::read(&case)?.context("fetched provenance was not recorded")?;
        ensure!(
            written["ok"] == fetched["ok"]
                && written["output_verification"] == fetched["output_verification"]
                && written["remote_execution_provenance"] == fetched["remote_execution_provenance"],
            "fetched output certification was not recorded"
        );
        if !partial {
            if let Some(proof) = proof
                .as_ref()
                .filter(|p| !content_fingerprints_disabled(&p["provenance"]))
            {
                let expected = recorded_history_hashes(&proof["provenance"])?;
                ensure!(
                    recorded_history_hashes(&written)? == expected,
                    "published output hashes differ from original completed outputs"
                );
            }
        }
        Ok(())
    })();
    if let Err(error) = publication {
        publishing["error"] = json!(error.to_string());
        super::provenance::write_value(&case, &publishing);
        return Err(error);
    }
    if fetched["ok"] == true {
        colm_case::clear_results_stale(&case)?;
    }
    let status = job::status(&ssh, &record.root, &record.job, 0)?;
    let outcome = json!({"job":record.job,"outcome":"observed","terminal":matches!(status.state, job::State::Finished {..}),"status":status});
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &outcome)?;
    let files = std::fs::read_dir(case.join(&history))?.count();
    println!(
        "{}",
        json!({ "fetched": entries, "history_files": files, "partial": partial, "subset": subset, "run_case": case, "output_verification":fetched["output_verification"] })
    );
    Ok(())
}

/// Re-hash inputs where the run executed; local/server path mappings are not content differences.
pub(crate) fn verify_recorded_inputs(
    case: &Path,
    provenance: &serde_json::Value,
    recorded: &crate::fingerprint::FullInputIdentity,
) -> Result<()> {
    ensure!(
        provenance["output_verification"]["status"] == "passed"
            && provenance["output_verification"]["partial"] == false
            && !case.join(FETCH_MARK).exists(),
        "remote output is partial or lacks original-output verification"
    );
    let _ = recorded;
    bail!("forcing content fingerprints are excluded by policy; full-input verification cannot invoke a remote CLI")
}

/// 取回时的出处：提交时记下的那份，加上作业的结局。
pub(crate) fn fetched_provenance(
    submitted: &serde_json::Value,
    state: Option<job::State>,
) -> serde_json::Value {
    let mut value = submitted.clone();
    let (ok, error) = match state {
        Some(job::State::Finished { exit_code: 0 }) => (json!(true), json!(null)),
        Some(job::State::Finished { exit_code }) => (
            json!(false),
            json!(format!("the server job exited with status {exit_code}")),
        ),
        _ => (json!(null), json!("the server job state is unknown")),
    };
    value["ok"] = ok;
    value["error"] = error;
    value["finished"] = json!(now());
    value
}

/// 在服务器上跑研究（第 5 阶段）；用这里的路径改写、引擎与内核的选择和作业提交。
#[path = "remote_study.rs"]
pub(crate) mod remote_study;

#[cfg(test)]
#[path = "remote_cmd_tests.rs"]
mod remote_cmd_tests;

fn replay_remote_flag(flag: &str) -> bool {
    matches!(
        flag,
        "--host"
            | "--root"
            | "--server-case"
            | "--servers"
            | "--server"
            | "--map"
            | "--scheduler"
            | "--threads"
            | "--cpus"
            | "--mem-gb"
            | "--nodes"
            | "--cores-per-node"
            | "--walltime"
            | "--queue"
            | "--partition"
            | "--account"
            | "--launcher"
            | "--preset"
            | "--remote-kernel"
    )
}
const PARTIAL_ROLLBACK_FAILED: &str = "cannot restore the previous history files";

/// Move fetched subset files into the case history, keeping each replaced file aside, and undo
/// every step if one fails so the history never mixes old and new files.
pub(crate) fn publish_partial_history(
    staged: &Path,
    history: &Path,
    replaced: &Path,
) -> Result<()> {
    std::fs::create_dir_all(history)?;
    std::fs::create_dir_all(replaced)?;
    let mut names = std::fs::read_dir(staged)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for name in names {
        let destination = history.join(&name);
        let step = (|| -> std::io::Result<Option<PathBuf>> {
            let backup = if destination.exists() {
                let backup = replaced.join(&name);
                std::fs::rename(&destination, &backup)?;
                Some(backup)
            } else {
                None
            };
            if let Err(error) = std::fs::rename(staged.join(&name), &destination) {
                if let Some(backup) = &backup {
                    let _ = std::fs::rename(backup, &destination);
                }
                return Err(error);
            }
            Ok(backup)
        })();
        match step {
            Ok(backup) => done.push((destination, backup)),
            Err(error) => {
                let mut failures = Vec::new();
                for (destination, backup) in done.iter().rev() {
                    let undone = std::fs::remove_file(destination).and_then(|_| match backup {
                        Some(backup) => std::fs::rename(backup, destination),
                        None => Ok(()),
                    });
                    if let Err(failure) = undone {
                        failures.push(format!("{}: {failure}", destination.display()));
                    }
                }
                if failures.is_empty() {
                    bail!("partial fetch publication failed and was rolled back: {error}");
                }
                bail!(
                    "{PARTIAL_ROLLBACK_FAILED} after a fetch failure ({error}; {}); they remain in {}",
                    failures.join("; "),
                    replaced.display()
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn replay_remote(
    old_case: &Path,
    prepared: &Opts,
    provenance: &serde_json::Value,
) -> Result<serde_json::Value> {
    let previous = read_record(old_case)?;
    ensure!(
        previous
            .provenance
            .as_ref()
            .and_then(|v| v["model_run"].get("operation_id"))
            == provenance["model_run"].get("operation_id"),
        "remote replay record does not match captured operation"
    );
    let supplied: Vec<(String, String)> =
        serde_json::from_value(provenance["remote_options"].clone())
            .context("remote replay options were not captured")?;
    ensure!(
        supplied.iter().all(|(key, _)| replay_remote_flag(key)),
        "invalid captured remote options"
    );
    let mut opts = Opts {
        positional: prepared.positional.clone(),
        flags: prepared.flags.clone(),
    };
    opts.flags.extend(supplied);
    opts.flags
        .push(("--expected-engine".into(), previous.engine.clone()));
    opts.flags.push(("--quiet-submit-json".into(), "1".into()));
    let case = prepared.positional_case()?;
    std::fs::write(
        case.join("replay-input-identity.json"),
        serde_json::to_vec(&json!({
            "full_input_identity":provenance["full_input_identity"],
            "kernel_program_sha256":provenance["kernel"]["program_sha256"],
            "execution_program_sha256":provenance["execution_program_sha256"]
        }))?,
    )?;
    cmd_run(&opts)?;
    let submitted = read_record(&case)?;
    crate::history_cmd::observe_remote_outcome(
        &json!({"job":submitted.job,"outcome":"submitted","terminal":false,"remote_record":submitted}),
    );
    Ok(json!({"job":submitted.job,"host":submitted.host,"status":"submitted"}))
}
