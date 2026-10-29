//! 在服务器上跑研究（界面重组第 5 阶段）。
//!
//! ```text
//! colm-cli remote-study-run    <本机研究> --host H --root R [--kernel 本机内核] [--map 本机=服务器]… [--upload-unmapped 1]
//!                              [--engine rust|fortran] [--preprocessors rust|fortran] [--jobs N] [--threads N]
//!                              [--model-version official|<修改版>] [--workspace-root DIR] [调度选项同 remote-run] [--dry-run 1]
//! colm-cli remote-study-status <本机研究>
//! colm-cli remote-study-fetch  <本机研究>
//! colm-cli remote-study-close  <本机研究>   # 确认服务器作业已丢失/无法核对后关闭本机远程记录
//! ```
//!
//! 研究里到处是本机的绝对路径（`manifest.root`、任务的 `case_dir`、观测、成员算例的 namelist），不能原样搬走。这里在
//! 服务器上**按同一份设定重建同一个研究**：基础算例像 `remote-run` 一样改写路径后上传，观测文件随研究上传，设定里的
//! 内核与观测换成服务器路径，然后 `study-create` + `study-run`。种子相同，成员与参数就相同——取回时逐个核对，
//! 一致才把服务器上的状态与 `results/` 移植进本机这个研究（`case_dir` 换回本机路径）。之后本机的研究状态、比较、
//! 另存最优方案照常可用；成员的模拟输出留在服务器上。
//!
//! 引擎同 `remote-run`：官方版或修改版（`--model-version`，服务器用它的源码编）。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_remote::job::{self, Spec};
use colm_remote::sched::{self, Resources};
use colm_remote::ssh::{quote, Ssh};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    choose_engine, engine, now, parse_mapping, parse_scheduler, remote_version,
    resolve_remote_kernel, resources_from, root_of, short_hash, stage_case, RunPlan,
};
use crate::study;
use crate::Opts;

/// 本机研究目录里的提交记录。
pub(crate) const RECORD: &str = ".colm-remote-study.json";
pub(crate) const PENDING: &str = ".colm-remote-study-pending.json";
/// 取回核验证据。研究目录里单独一份：记录（`RECORD`）每次状态轮询都要经 SSH 搬回，把每个成员的结果树
/// 哈希塞进记录会把它撑大；证据是审计材料，放旁边文件，旧记录没有它也不影响任何读取路径。
pub(crate) const EVIDENCE: &str = ".colm-remote-study-evidence.json";

/// Submission intent survives an interrupted SSH response. Only status reads may use it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingStudy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_auth: Option<colm_remote::auth::Connection>,
    host: String,
    root: String,
    job: String,
    remote_project: String,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    provenance: Value,
}

fn check_study_location(host: &str, root: &str, job: &str, project: &str) -> Result<()> {
    ensure!(
        !host.is_empty()
            && !host.starts_with('-')
            && !host.contains(|c: char| c.is_whitespace() || c.is_control()),
        "invalid recorded SSH host"
    );
    job::check_id(job)?;
    ensure!(
        root.starts_with('/')
            && root != "/"
            && !root.contains(|c: char| c.is_whitespace() || c.is_control())
            && Path::new(root).components().all(|part| matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )),
        "invalid recorded remote root"
    );
    ensure!(
        project.starts_with(&format!("{root}/s/"))
            && !project.contains(char::is_control)
            && Path::new(project).components().all(|part| matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )),
        "Study project is outside its recorded remote root"
    );
    Ok(())
}

fn read_pending(study_dir: &Path) -> Result<PendingStudy> {
    let pending: PendingStudy = serde_json::from_slice(&std::fs::read(study_dir.join(PENDING))?)
        .context("cannot parse pending remote Study submission")?;
    check_study_location(
        &pending.host,
        &pending.root,
        &pending.job,
        &pending.remote_project,
    )?;
    if let Some(engine) = &pending.engine {
        job::check_id(engine)?;
    }
    Ok(pending)
}

pub(crate) fn pending_status(study_dir: &Path) -> Result<Option<Value>> {
    if study_dir.join(RECORD).exists() || !study_dir.join(PENDING).exists() {
        return Ok(None);
    }
    let pending = read_pending(study_dir)?;
    Ok(Some(json!({
        "host": pending.host, "root": pending.root, "job": pending.job,
        "remote_project": pending.remote_project, "needs_reconciliation": true,
    })))
}

fn pending_identity_script(pending: &PendingStudy) -> Result<String> {
    check_study_location(
        &pending.host,
        &pending.root,
        &pending.job,
        &pending.remote_project,
    )?;
    let marker = format!(
        "# colm-study-root: {}",
        serde_json::to_string(&pending.remote_project)?
    );
    let engine = pending.engine.as_ref().map_or(String::new(), |engine| {
        format!("grep -Fqx -- {} script.sh || {{ echo 'engine snapshot identity changed' >&2; exit 1; }}\n", quote(&format!("# colm-engine-snapshot: {engine}")))
    });
    Ok(format!(
        "set -e\ncd {dir}\n[ -f script.sh ] && [ ! -L script.sh ] || {{ echo 'pending Study job has no regular script' >&2; exit 1; }}\ngrep -Fqx -- {marker} script.sh || {{ echo 'Study project identity changed' >&2; exit 1; }}\n{engine}",
        dir = quote(&job::job_dir(&pending.root, &pending.job)), marker = quote(&marker),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StudyRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_auth: Option<colm_remote::auth::Connection>,
    pub host: String,
    pub root: String,
    pub job: String,
    /// 服务器上的项目目录（基础算例、观测、设定都在这里，研究建在它的 `.colm/studies/` 下）。
    pub remote_project: String,
    pub engine: String,
    pub submitted_at: u64,
    pub scheduler: String,
    #[serde(default)]
    pub scheduler_id: Option<String>,
    /// 版本、引擎、服务器（和 `produced-by.json` 同形）。
    pub provenance: Value,
    #[serde(default)]
    pub fetched_at: Option<u64>,
}

/// 远程 Study 记录里写下的模型版本身份。
///
/// `fetch` 只搬回状态与结果，不保证带回 `model-version.json`；修改版 Study 从服务器跑完取回后，
/// 本机记录里唯一的版本身份就是提交时写下的 `provenance.version`。本地证据（`study-status`）
/// 必须能从它还原真实身份，否则修改版结果会被错误归因成官方版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedModelVersion {
    /// 记录明确声明用官方版。只有 `kind == "official"` 才算，缺记录不算。
    Official,
    /// 修改版工作区：记录里有名字与提交，但没有冻结修订 id/source hash（那些不在远程记录里）。
    Workspace {
        name: String,
        head: String,
        base_commit: Option<String>,
    },
}

impl RecordedModelVersion {
    /// 解析 `provenance.version`（与 `produced-by.json` 同形）。缺字段、未知 kind、
    /// 名字/提交为空都报错，由调用方转成明确的 unknown，绝不静默当成官方版。
    fn from_provenance(provenance: &Value) -> Result<Self> {
        let version = provenance
            .get("version")
            .filter(|version| version.is_object())
            .context("captured remote Study record has no provenance.version")?;
        match version["kind"].as_str() {
            Some("official") => Ok(Self::Official),
            Some("workspace") => {
                let name = version["name"]
                    .as_str()
                    .filter(|name| !name.trim().is_empty())
                    .context("captured remote workspace version has no name")?;
                let head = version["head"]
                    .as_str()
                    .filter(|head| !head.trim().is_empty())
                    .context("captured remote workspace version has no commit")?;
                Ok(Self::Workspace {
                    name: name.into(),
                    head: head.into(),
                    base_commit: version["base_commit"].as_str().map(str::to_owned),
                })
            }
            _ => bail!("captured remote Study version has no supported kind"),
        }
    }
}

/// 从本机研究目录的 remote record 还原版本身份；没有记录时 `Ok(None)`。
///
/// 与 `study-compare` 的版本身份解析同一套逻辑：只读本地文件，不校验/初始化 SSH 凭据，
/// 不连服务器。坏 JSON、缺字段、不支持的 kind 都返回错误，让调用方明确报 unknown。
pub(crate) fn recorded_model_version(study_dir: &Path) -> Result<Option<RecordedModelVersion>> {
    let path = study_dir.join(RECORD);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("cannot inspect captured remote Study version"),
    }
    // 记录里可能记着 password 认证设置；读身份不是 SSH 操作，不能因此要求凭据。
    let record: StudyRecord = serde_json::from_str(
        &std::fs::read_to_string(&path).context("cannot read captured remote Study version")?,
    )
    .context("cannot parse captured remote Study version")?;
    RecordedModelVersion::from_provenance(&record.provenance).map(Some)
}

pub(crate) fn read_record(study_dir: &Path) -> Result<StudyRecord> {
    let path = study_dir.join(RECORD);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "{} has not been run on a server (no {RECORD})",
            study_dir.display()
        )
    })?;
    let record: StudyRecord = serde_json::from_str(&text)?;
    check_study_location(
        &record.host,
        &record.root,
        &record.job,
        &record.remote_project,
    )?;
    job::check_id(&record.engine)?;
    colm_remote::auth::check_connection(record.ssh_auth.as_ref())?;
    Ok(record)
}

/// 研究所在的项目目录：`<项目>/.colm/studies/<编号>/`。
fn project_of(study_dir: &Path) -> Result<PathBuf> {
    let parent = study_dir
        .parent()
        .context("a Study has a parent directory")?;
    ensure!(
        parent.ends_with(Path::new(".colm").join("studies")),
        "{} is not under <project>/.colm/studies",
        study_dir.display()
    );
    Ok(parent
        .parent()
        .and_then(Path::parent)
        .context("a Study is under <project>/.colm/studies")?
        .to_path_buf())
}

/// 本机这个研究还没开始跑（服务器上要从头跑同一个研究）。
fn not_started(study_dir: &Path) -> Result<()> {
    use study::state::TaskStatus;
    let started = study::runner::status_state(study_dir)?.is_some_and(|state| {
        state
            .tasks
            .values()
            .any(|task| !matches!(task.status, TaskStatus::Pending | TaskStatus::Materialized))
    });
    ensure!(
        !started,
        "this Study has already started on this computer; create a new Study to run it on a server"
    );
    Ok(())
}

/// 观测文件在服务器上的名字：站点名加原文件名（不同站点的同名文件不会撞）。
pub(crate) fn remote_observation_name(site: &str, path: &str) -> String {
    let file = file_name_of(path);
    format!("{site}-{file}")
}

/// 目标观测在服务器上的名字。目标键可以由用户随便写（可能带空格或路径分隔符），不能直接拼进文件名；
/// 用序号做唯一前缀，再带上消过毒的目标键方便在服务器上认出来源。两个目标都叫 `Pool.nc` 时不会相撞。
pub(crate) fn remote_target_source_name(index: usize, key: &str, path: &str) -> String {
    let label: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let label = label.trim_matches('_');
    let label = if label.is_empty() { "target" } else { label };
    format!("target-{}-{label}-{}", index + 1, file_name_of(path))
}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "obs.nc".into())
}

/// `--map` 命中的服务器路径；与 `remote_cmd::place` 同一套语义：按路径分量比前缀（`/a/bc` 不在 `/a/b`
/// 之下），嵌套映射里最具体的那条优先。映射是目录时保留剩下的相对路径。
fn mapped_server_path(raw: &str, maps: &[super::Mapping]) -> Option<String> {
    let local = PathBuf::from(raw).to_string_lossy().replace('\\', "/");
    let local = local.trim_end_matches('/');
    let mut maps: Vec<&super::Mapping> = maps.iter().collect();
    maps.sort_by_key(|map| std::cmp::Reverse(map.local.len()));
    maps.into_iter().find_map(|map| {
        let rest = super::under(local, &map.local)?;
        Some(format!("{}{rest}", map.remote))
    })
}

/// 一次提交在服务器 `obs/` 里放好的观测文件。
struct StagedObservations {
    bytes: u64,
    /// 命中 `--map`、留给服务器上既有文件的路径：提交前要确认它们真的在服务器上。
    remote_inputs: Vec<String>,
}

/// 把所有观测源搬成服务器路径：站点观测（`spec.observations`）和每个目标自己的观测文件
/// （格点产品、区域通量站、水文站）走同一条规则。
///
/// 为什么必须上传而不是按项目前缀推导服务器路径：观测文件在这台电脑上的绝对路径（`/Users/.../GLEAM.nc`）
/// 服务器根本没有；`study-create`/预检会读观测算哈希，读不到就整个研究建不起来。观测文件都不大，复制进
/// 项目的 `obs/` 比要求用户为每个目标配映射可靠。已经有 `--map` 对应的路径保留服务器路径、不复制
/// （共享观测目录在服务器上已有一份时不该再传一遍）。
///
/// `spec.observations` 原来的行为是无论有没有映射都上传；现在按同一规则处理：只有映射命中才不复制，
/// 没命中照样上传——它们不受 `--upload-unmapped` 约束（那是基础算例里大文件的开关）。
fn stage_observation_sources(
    spec: &mut study::spec::StudySpec,
    staging: &Path,
    remote_project: &str,
    maps: &[super::Mapping],
) -> Result<StagedObservations> {
    std::fs::create_dir_all(staging.join("obs"))?;
    let mut staged = StagedObservations {
        bytes: 0,
        remote_inputs: Vec::new(),
    };
    let mut used = std::collections::BTreeSet::new();
    for (site, path) in spec.observations.iter_mut() {
        let name = remote_observation_name(site, path.as_str());
        stage_one_observation(
            &mut staged,
            path,
            name,
            &format!("site {site}"),
            staging,
            remote_project,
            maps,
            &mut used,
        )?;
    }
    for (index, target) in spec.targets.iter_mut().enumerate() {
        if let Some(source) = &mut target.source {
            let path = source.path().to_owned();
            let name = remote_target_source_name(index, &target.key, &path);
            let mut rewritten = path;
            stage_one_observation(
                &mut staged,
                &mut rewritten,
                name,
                &format!("target {}", target.key),
                staging,
                remote_project,
                maps,
                &mut used,
            )?;
            source.set_path(rewritten);
        }
    }
    staged.remote_inputs.sort();
    staged.remote_inputs.dedup();
    Ok(staged)
}

#[allow(clippy::too_many_arguments)]
fn stage_one_observation(
    staged: &mut StagedObservations,
    path: &mut String,
    name: String,
    label: &str,
    staging: &Path,
    remote_project: &str,
    maps: &[super::Mapping],
    used: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    if let Some(remote) = mapped_server_path(path, maps) {
        staged.remote_inputs.push(remote.clone());
        *path = remote;
        return Ok(());
    }
    let local = Path::new(path.as_str());
    ensure!(
        local.is_file(),
        "observation file for {label} does not exist: {path}; put a copy on the server and add a --map \
         mapping for it, or restore the file here so it can be uploaded with the Study"
    );
    // 同名文件的唯一化：正常情况下序号/站点前缀已经唯一，这里只是兜底，保证改写后的路径不会互相覆盖。
    let mut unique = name.clone();
    let mut suffix = 2;
    while !used.insert(unique.clone()) {
        unique = format!("{name}-{suffix}");
        suffix += 1;
    }
    let destination = staging.join("obs").join(&unique);
    staged.bytes += std::fs::copy(local, &destination)
        .with_context(|| format!("cannot copy the observation file for {label} from {path}"))?;
    *path = format!("{remote_project}/obs/{unique}");
    Ok(())
}

/// 作业体：确保引擎编好，在服务器上建研究，然后跑。研究目录写进作业目录的 `study_root`，查状态与取回时用。
#[allow(clippy::too_many_arguments)]
pub(crate) fn study_job_body(
    root: &str,
    engine_id: &str,
    remote_project: &str,
    remote_kernel: &str,
    threads: u32,
    jobs: usize,
    plan_engine: &str,
    preprocessors: &str,
) -> String {
    let bin = quote(&format!("{}/bin", engine::engine_dir(root, engine_id)));
    format!(
        "# colm-study-root: {study_marker}\necho \"preparing the engine\" > phase\n{ensure}{policy}\
         echo \"creating the Study\" >> phase\n\
         {bin}/colm-cli study-create {project} --spec {spec} > study_create.out\n\
         tail -n 1 study_create.out > study_root\n\
         STUDY=$(cat study_root)\n\
         echo \"running the Study\" >> phase\n\
         RAYON_NUM_THREADS={threads} {bin}/colm-cli study-run \"$STUDY\" --kernel {kernel} --stream 1 --jobs {jobs} --engine {plan_engine} --preprocessors {preprocessors}\n",
        study_marker = serde_json::to_string(remote_project).expect("strings serialize to JSON"),
        ensure = engine::ensure_script(root, engine_id, threads.max(8).clamp(1, 64)),
        policy = super::forcing_policy_guard(&format!("{}/bin/colm-cli", engine::engine_dir(root, engine_id))),
        project = quote(remote_project),
        spec = quote(&format!("{remote_project}/spec.json")),
        kernel = quote(remote_kernel),
    )
}

/// 提交前在本机暂存的整份研究资产（基础算例、观测、设定）。
#[derive(Debug)]
pub(crate) struct StagedStudy {
    pub(crate) spec: Value,
    pub(crate) remote_inputs: Vec<String>,
    pub(crate) bytes: u64,
}

/// 暂存目录名。`dry-run` 与任何失败路径都要把它整个删掉：里面是本机观测文件的副本，
/// 服务器没收到、本机也没人要，留在 `/tmp` 只是泄漏数据。
pub(crate) fn staging_dir(study_id: &str, pid: u32) -> PathBuf {
    std::env::temp_dir().join(format!("colm-remote-study-{study_id}-{pid}"))
}

pub(crate) fn discard_staging(staging: &Path) {
    let _ = std::fs::remove_dir_all(staging);
}

/// 把研究搬到服务器上的路径空间：基础算例按 `remote-run` 的规则改写，观测源统一上传/映射，
/// 最后产出一份服务器能直接 `study-create` 的设定。任何失败都自己清掉暂存目录：里面是本机
/// 观测文件的副本，服务器没收到、本机也不要，留在 `/tmp` 只是泄漏。
fn stage_study_assets(
    manifest: &study::spec::Manifest,
    staging: &Path,
    remote_project: &str,
    maps: &[super::Mapping],
    upload_unmapped: bool,
) -> Result<StagedStudy> {
    match stage_study_assets_inner(manifest, staging, remote_project, maps, upload_unmapped) {
        Ok(staged) => Ok(staged),
        Err(error) => {
            discard_staging(staging);
            Err(error)
        }
    }
}

fn stage_study_assets_inner(
    manifest: &study::spec::Manifest,
    staging: &Path,
    remote_project: &str,
    maps: &[super::Mapping],
    upload_unmapped: bool,
) -> Result<StagedStudy> {
    let mut unmapped = Vec::new();
    let mut remote_inputs = Vec::new();
    let mut bytes = 0;
    for base in &manifest.spec.base_cases {
        let staged = stage_case(
            &study::model::base_case(manifest, base)?,
            &staging.join(base),
            &format!("{remote_project}/{base}"),
            maps,
            upload_unmapped,
        )?;
        let run_file = staging.join(base).join(colm_model::RUN_MANIFEST);
        if run_file.is_file() {
            let mut run: colm_model::MaterializedRun =
                serde_json::from_slice(&std::fs::read(&run_file)?)?;
            run.run_case = PathBuf::from(format!("{remote_project}/{base}"));
            std::fs::write(run_file, serde_json::to_vec_pretty(&run)?)?;
        }
        unmapped.extend(staged.unmapped);
        remote_inputs.extend(staged.remote_inputs);
        bytes += staged.bytes;
    }
    if !unmapped.is_empty() {
        let list: Vec<String> = unmapped.iter().map(|p| p.display().to_string()).collect();
        bail!(
            "these inputs are on this computer but have no server path; add a path mapping for them \
             (or allow uploading the files): {}",
            list.join(", ")
        );
    }
    let mut spec = manifest.spec.clone();
    // 服务器上的引擎可能来自较旧快照，读不了新加的审计字段 parent_study；谱系只在本地研究里保留，
    // 服务器重建的研究不需要它。
    spec.parent_study = None;
    let observations = stage_observation_sources(&mut spec, staging, remote_project, maps)?;
    remote_inputs.extend(observations.remote_inputs);
    remote_inputs.sort();
    remote_inputs.dedup();
    bytes += observations.bytes;
    Ok(StagedStudy {
        spec: serde_json::to_value(spec)?,
        remote_inputs,
        bytes,
    })
}

/// 远程研究一次申请的资源与它的三个量：总核数、每个候选的线程数、同时跑几个候选。
///
/// `resources_from` 只负责调度器字段；总核数必须由研究这边算：每个成员各有 `threads` 个线程，
/// 同时最多跑 `jobs` 个成员，所以默认要 `threads × jobs` 核。沿用 `max(threads, jobs)` 会在
/// jobs > 1 时只申请一个成员的核数，其余成员超配、在集群上被 OOM/限核打掉（审计 P2-10）。
#[derive(Debug)]
struct StudyResources {
    resources: Resources,
    threads: u32,
    jobs: usize,
}

fn study_resources(opts: &Opts, threads: u32, jobs: usize) -> Result<StudyResources> {
    ensure!(
        threads >= 1,
        "--threads must be at least 1 (every Study member runs at least one thread)"
    );
    ensure!(
        jobs >= 1,
        "--jobs must be at least 1 (a Study runs at least one candidate at a time)"
    );
    let jobs_u32 =
        u32::try_from(jobs).context("--jobs is too large for a scheduler resource request")?;
    let required = threads
        .checked_mul(jobs_u32)
        .context("remote Study resource request overflows the scheduler core count")?;
    let resources = resources_from(opts, required, 1, 0)?;
    // 申请的总核数必须覆盖实际并行：作业脚本里 `RAYON_NUM_THREADS=threads`、`study-run
    // --jobs jobs`，同时最多 jobs 个成员各跑 threads 线程。显式 --cpus 小于 threads × jobs
    // 时作业仍会开这么多线程，超出申请的部分会被集群限核或 OOM 打掉——这正是 P2-10 里
    // max(threads, jobs) 少申请的老问题，只是换成了使用者显式给的数字。
    //
    // 不选“按 cpus/threads 自动降并发”：那会让作业脚本里的 --jobs、报告里的并发与用户
    // 设定的预算三者分叉，界面也看不出并发被悄悄改了；申请不足是配置错误，明确拒绝并
    // 告诉使用者该申请多少更有用。多申请的核（cpus > threads × jobs）仍按用户意愿保留。
    ensure!(
        resources.cpus >= required,
        "--cpus ({}) must be at least --threads × --jobs ({} × {jobs} = {required}): {jobs} Study members run concurrently, each with {threads} threads; raise --cpus or lower --jobs/--threads",
        resources.cpus, threads
    );
    Ok(StudyResources {
        resources,
        threads,
        jobs,
    })
}

/// 提交/预演报告里的 `resources` 段：写清总核、每候选线程与并发，以及调度器要的其他字段。
fn resources_report(resources: &Resources, threads: u32, jobs: usize) -> Value {
    json!({
        "cpus": resources.cpus,
        "cores_per_candidate": threads,
        "concurrent_candidates": jobs,
        "ranks": resources.ranks,
        "nodes": resources.nodes,
        "cores_per_node": resources.cores_per_node,
        "memory_gb": resources.memory_gb,
        "walltime": resources.walltime,
        "partition": resources.partition,
        "account": resources.account,
    })
}

/// `--dry-run 1` 的报告；抽出来让测试能核对资源/线程/并发确实写进了给人看的输出。
fn dry_run_report(
    job_script: String,
    remote_project: &str,
    engine_id: &str,
    spec: &Value,
    resources: &StudyResources,
) -> Value {
    json!({
        "dry_run": true,
        "job_script": job_script,
        "remote_project": remote_project,
        "engine": engine_id,
        "spec": spec,
        "threads": resources.threads,
        "jobs": resources.jobs,
        "resources": resources_report(&resources.resources, resources.threads, resources.jobs),
    })
}

/// `remote-study-run`：在服务器上重建并运行这个研究。
pub(crate) fn cmd_run(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let manifest = study::engine::status(&study_dir)?;
    let _submission_lock = study::runner::StudyRunLock::acquire(&study_dir)?;
    not_started(&study_dir)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let execution = study::model::execution_options(&study_dir, opts)?;
    let opts = &execution;
    let _project = project_of(&study_dir)?;
    if opts.get("--dry-run").as_deref() != Some("1") {
        ensure!(!study_dir.join(RECORD).exists(), "this Study already has a remote submission; inspect its status or fetch it instead of submitting again");
        ensure!(!study_dir.join(PENDING).exists(), "remote Study submission outcome is uncertain; inspect the recorded job before submitting again");
    }
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let plan = RunPlan::from_opts(opts)?;
    ensure!(
        plan.ranks == 1,
        "Study members run one process each; --ranks does not apply"
    );
    let version = remote_version(opts)?;
    let kernel_dir = opts
        .get("--kernel")
        .or_else(|| manifest.spec.kernel_dir.clone())
        .context("remote-study-run needs --kernel or spec.kernel_dir")?;
    let local_kernel = colm_kernel::Kernel::open_manifest(Path::new(&kernel_dir))?;
    let preset = local_kernel.manifest.preset.clone();
    let maps = opts
        .get_all("--map")
        .iter()
        .map(|m| parse_mapping(m))
        .collect::<Result<Vec<_>>>()?;
    let upload_unmapped = opts.get("--upload-unmapped").is_some_and(|v| v == "1");
    let dry_run = opts.get("--dry-run").is_some_and(|v| v == "1");
    let threads: u32 = opts
        .get("--threads")
        .map(|t| t.parse())
        .transpose()?
        .unwrap_or(8);
    let jobs: usize = opts
        .get("--jobs")
        .map(|j| j.parse())
        .transpose()?
        .unwrap_or(manifest.spec.budget.jobs.max(1));
    let scheduler = parse_scheduler(opts, &ssh, &root)?;
    // 总核数默认 = 每候选线程 × 并发；显式 --cpus 由用户决定，但不能少于一个候选的线程数。
    let resources = study_resources(opts, threads, jobs)?;
    // 路径尽量短：成员的重启文件路径要在 CoLM 的 256 字节以内（`<项目>/.colm/studies/<编号>/members/…`）。
    let remote_project = format!(
        "{root}/s/{}",
        short_hash(&format!("{}|{}", manifest.id, study_dir.display()))
    );

    // 1. 基础算例：改写路径后暂存；所有观测源（站点观测与目标自己的观测文件）随后统一上传或按 --map 保留。
    let staging = staging_dir(&manifest.id, std::process::id());
    discard_staging(&staging);
    let staged =
        match stage_study_assets(&manifest, &staging, &remote_project, &maps, upload_unmapped) {
            Ok(staged) => staged,
            Err(error) => {
                // 失败时也不能把本机观测副本留在临时目录（helper 已经写了一半）。
                discard_staging(&staging);
                return Err(error);
            }
        };
    let StagedStudy {
        mut spec,
        remote_inputs,
        bytes,
    } = staged;

    // 2. 引擎与内核。
    let engine = choose_engine(opts, &ssh, &root, version.as_ref())?;
    let fortran = plan.engine == "fortran";
    let manifest_text = std::fs::read_to_string(Path::new(&kernel_dir).join("manifest.json"))?;
    let remote_kernel = if fortran {
        resolve_remote_kernel(opts, &ssh, &root, &preset, version.as_ref())?
    } else {
        format!("{root}/kernels/{preset}-{}", short_hash(&manifest_text))
    };
    if let Some(revision) =
        crate::study_version::pinned(&study_dir)?.and_then(|pin| pin.model_revision)
    {
        if fortran {
            crate::remote_cmd::check_selected_remote_kernel(&ssh, &remote_kernel, &revision)?;
        } else {
            crate::model_version::check_kernel(&revision, &local_kernel)?;
        }
    }
    spec["kernel_dir"] = json!(remote_kernel);
    std::fs::write(
        staging.join("spec.json"),
        serde_json::to_string_pretty(&spec)?,
    )?;
    let revision_path = study::model::stage_revision(&study_dir, &staging)?;
    let mut body = study_job_body(
        &root,
        engine.id(),
        &remote_project,
        &remote_kernel,
        threads,
        jobs,
        &plan.engine,
        &plan.preprocessors,
    );
    if let Some(path) = revision_path {
        let argument = quote(&format!("{remote_project}/{}", path.display()));
        body = body.replace(
            " > study_create.out",
            &format!(" --frozen-study-model {argument} > study_create.out"),
        );
        body = body.replace(
            "RAYON_NUM_THREADS=",
            "COLM_STUDY_BY_PARENT=1 RAYON_NUM_THREADS=",
        );
    }
    let job_spec = Spec {
        scheduler,
        resources: resources.resources.clone(),
        name: sched::job_name(&format!("study-{}", manifest.id)),
    };
    if dry_run {
        discard_staging(&staging);
        let report = dry_run_report(
            job::job_script(&root, "JOBID", &body, &job_spec)?,
            &remote_project,
            engine.id(),
            &spec,
            &resources,
        );
        println!("{}", report);
        return Ok(());
    }

    // 3. 核对服务器上的数据，上传，提交。
    if !remote_inputs.is_empty() {
        let checks: Vec<String> = remote_inputs
            .iter()
            .map(|p| format!("[ -e {q} ] || echo {q}", q = quote(p)))
            .collect();
        let missing = ssh.run_ok(&checks.join("\n"))?;
        if !missing.trim().is_empty() {
            discard_staging(&staging);
            bail!(
                "these inputs are missing on {}: {}",
                ssh.host,
                missing.trim().lines().collect::<Vec<_>>().join(", ")
            );
        }
    }
    let id = format!("study-{}-{}", now(), short_hash(&manifest.id));
    // Keep an intent after an uncertain transport result; a retry must never submit a second job.
    let pending = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(study_dir.join(PENDING))?;
    let operation_id = opts.get("--operation-id");
    serde_json::to_writer(
        &pending,
        &PendingStudy {
            ssh_auth: colm_remote::auth::current_connection(),
            host: ssh.host.clone(),
            root: root.clone(),
            job: id.clone(),
            remote_project: remote_project.clone(),
            engine: Some(engine.id().to_owned()),
            provenance: json!({"operation_id":operation_id}),
        },
    )?;
    pending.sync_all()?;
    let mut engine_lock = colm_remote::engine_inventory::LifecycleLock::acquire(&ssh, &root)?;
    let (engine_uploaded, id, submitted) = engine_lock.run(|| {
        let engine_uploaded = engine.upload(&ssh, &root)?;
        if !fortran {
            ssh.run_ok(&super::write_manifest_script(
                &remote_kernel,
                &manifest_text,
            ))?;
        }
        // 服务器上已有的旧项目目录先清掉：研究要从头按这份设定建。
        ssh.run_ok(&format!("rm -rf {}", quote(&remote_project)))?;
        let entries: Vec<String> = std::fs::read_dir(&staging)?
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        ssh.upload(&staging, &entries, &remote_project)?;
        discard_staging(&staging);
        let submitted = job::submit(&ssh, &root, &id, &body, &job_spec, engine.id())?;
        Ok((engine_uploaded, id.clone(), submitted))
    })?;
    let record = StudyRecord {
        ssh_auth: colm_remote::auth::current_connection(),
        host: ssh.host.clone(),
        root,
        job: id.clone(),
        remote_project: remote_project.clone(),
        engine: engine.id().to_owned(),
        submitted_at: now(),
        scheduler: submitted.scheduler.to_owned(),
        scheduler_id: submitted.scheduler_id.clone(),
        provenance: json!({
            "schema": 1,
            "operation_id": operation_id,
            "model_revision": crate::study_version::pinned(&study_dir)?.and_then(|pin| pin.model_revision),
            "started": now(),
            "version": match &version {
                Some(version) => version.value.clone(),
                None => crate::provenance::official_version_value(),
            },
            "engine": plan.engine,
            "preprocessors": plan.preprocessors,
            "location": { "kind": "server", "host": ssh.host },
            "kernel": { "dir": remote_kernel, "preset": preset },
            "engine_snapshot": engine.id(),
        }),
        fetched_at: None,
    };
    let record_warning = std::fs::write(
        study_dir.join(RECORD),
        serde_json::to_string_pretty(&record)?,
    )
    .err()
    .map(|e| format!("Job {id} submitted, but local record could not be saved: {e}"));
    if record_warning.is_none() {
        let _ = std::fs::remove_file(study_dir.join(PENDING));
    }
    let lock_warning = engine_lock.finish_submitted(&id);
    // Submission has already succeeded: ledger failure must not invite a duplicate submission.
    let ledger_warning = record_version_use(
        version.as_ref(),
        &study_dir,
        &manifest.id,
        &ssh.host,
        record.submitted_at,
    )
    .err()
    .map(|error| {
        format!("job submitted, but workspace study ledger could not be updated: {error:#}")
    });
    println!(
        "{}",
        json!({
            "ledger_warning": ledger_warning,
            "record_warning": record_warning,
            "lock_warning": lock_warning,
            "job": id,
            "host": ssh.host,
            "remote_project": remote_project,
            "engine": engine.id(),
            "engine_uploaded": engine_uploaded,
            "bytes": bytes,
            "remote_inputs": remote_inputs,
            "threads": resources.threads,
            "jobs": resources.jobs,
            "resources": resources_report(&resources.resources, resources.threads, resources.jobs),
        })
    );
    Ok(())
}

fn record_version_use(
    version: Option<&super::RemoteVersion>,
    study_dir: &Path,
    study_id: &str,
    host: &str,
    submitted_at: u64,
) -> Result<()> {
    let Some(version) = version else {
        return Ok(());
    };
    if version.name == "official" {
        return Ok(());
    }
    let workspace_root = version
        .src
        .parent()
        .and_then(Path::parent)
        .context("workspace source has no workspace root")?;
    let workspace = colm_workspace::Workspace::open(workspace_root, &version.name)?;
    colm_workspace::studies::append(
        &workspace,
        study_dir,
        study_id,
        &version.head,
        host,
        submitted_at,
    )
}

fn job_dir(record: &StudyRecord) -> String {
    format!("{}/jobs/{}", record.root, record.job)
}

/// 服务器上那个研究的目录（作业建好研究之后才有）。
fn remote_study_root(ssh: &Ssh, record: &StudyRecord) -> Result<Option<String>> {
    let text = ssh.run_ok(&format!(
        "cat {} 2>/dev/null || true",
        quote(&format!("{}/study_root", job_dir(record)))
    ))?;
    let root = text.trim();
    if root.is_empty() {
        return Ok(None);
    }
    ensure!(
        study_under_project(root, &project_spellings(ssh, record)?),
        "remote Study root differs from its submitted project"
    );
    Ok(Some(root.to_owned()))
}

/// The submitted project and its real path on the server. `study-create` prints the real path, so a
/// symlinked server root (a linked data disk, or macOS `/var` → `/private/var`) names the same Study
/// in two spellings; both are accepted, and nothing outside the project is.
fn project_spellings(ssh: &Ssh, record: &StudyRecord) -> Result<Vec<String>> {
    let real = ssh.run_ok(&format!(
        "cd -- {} && pwd -P",
        quote(&record.remote_project)
    ))?;
    let real = real.trim().lines().last().unwrap_or_default().to_owned();
    let mut spellings = vec![record.remote_project.clone()];
    if real.starts_with('/') && !real.contains(char::is_control) && real != record.remote_project {
        spellings.push(real);
    }
    Ok(spellings)
}

fn study_under_project(root: &str, projects: &[String]) -> bool {
    projects
        .iter()
        .any(|project| root.starts_with(&format!("{project}/.colm/studies/")))
        && !root.contains(char::is_control)
        && Path::new(root).components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}

fn remote_study_status(ssh: &Ssh, record: &StudyRecord, study_root: &str) -> Result<Value> {
    let bin = format!("{}/bin", engine::engine_dir(&record.root, &record.engine));
    let text = ssh.run_ok(&format!(
        "{}/colm-cli study-status {}",
        quote(&bin),
        quote(study_root)
    ))?;
    serde_json::from_str(text.trim().lines().last().unwrap_or("null"))
        .context("cannot read the server's study-status output")
}

fn comparison_command(
    record: &StudyRecord,
    study_root: &str,
    projects: &[String],
) -> Result<String> {
    check_study_location(
        &record.host,
        &record.root,
        &record.job,
        &record.remote_project,
    )?;
    job::check_id(&record.engine)?;
    ensure!(
        study_under_project(study_root, projects),
        "comparison Study root differs from its submitted project"
    );
    Ok(format!(
        "{identity}\nCOLM_AUDIT_BY_PARENT=1 {cli} study-verify-comparison {study}\n",
        identity = cancel_identity_script(record)?,
        cli = quote(&format!(
            "{}/bin/colm-cli",
            engine::engine_dir(&record.root, &record.engine)
        )),
        study = quote(study_root),
    ))
}

fn scoped_comparison_evidence(
    cli: &str,
    command: &str,
    mut run: impl FnMut(&str) -> Result<String>,
) -> Result<Value> {
    let policy = run(&format!(
        "COLM_AUDIT_BY_PARENT=1 {} fingerprint-policy",
        quote(cli)
    ))
    .context("update the model code: server CLI cannot prove forcing-content exclusion")?;
    ensure!(
        serde_json::from_str::<Value>(policy.trim()).ok()
            == Some(json!({"forcing_content_policy":"excluded-v1"})),
        "update the model code: server CLI does not exclude forcing content fingerprints"
    );
    let output = run(command)?;
    serde_json::from_str(
        output
            .trim()
            .lines()
            .last()
            .context("server returned no comparison proof")?,
    )
    .context("server CLI does not provide readable scoped Study evidence")
}

fn server_comparison_evidence(ssh: &Ssh, record: &StudyRecord, study_root: &str) -> Result<Value> {
    let projects = project_spellings(ssh, record)?;
    let command = comparison_command(record, study_root, &projects)?;
    let cli = format!(
        "{}/bin/colm-cli",
        engine::engine_dir(&record.root, &record.engine)
    );
    scoped_comparison_evidence(&cli, &command, |command| ssh.run_ok(command))
}

/// 服务器比较证据（schema 1）里与本机检查点绑定的部分：状态本身、状态哈希、研究编号与最优成员。
///
/// 为什么单独成函数：率定取回/复核要求 `best_member`，不确定性研究按设计没有最优成员，不能走这里。
fn evidence_state(evidence: &Value, remote_root: &str) -> Result<study::state::StudyState> {
    ensure!(
        evidence["schema"] == 1,
        "unsupported remote comparison evidence schema"
    );
    ensure!(
        evidence["root"].as_str() == Some(remote_root),
        "remote comparison evidence came from another Study"
    );
    let remote_state: study::state::StudyState = serde_json::from_value(evidence["state"].clone())?;
    let state_sha256 = colm_kernel::sha256_hex(&serde_json::to_vec(&remote_state)?);
    ensure!(
        evidence["state_sha256"].as_str() == Some(state_sha256.as_str()),
        "remote comparison state hash does not match its evidence"
    );
    ensure!(
        evidence["study_id"].as_str() == Some(remote_state.study_id.as_str())
            && evidence["best_member"] == serde_json::to_value(&remote_state.best_member)?,
        "remote comparison member identity differs from its checkpoint"
    );
    Ok(remote_state)
}

/// 服务器比较证据的 schema 1 是率定专用的：里面只有最优成员与基线的比较，没有 `kind`。
/// 分流后旧证据（缺 `kind`）仍按率定接收——schema 1 唯一可能的含义就是率定比较；明确带了
/// `kind` 却与本机研究类型不符的拒绝，免得以后类型化的证据被放进错的分支。
fn ensure_evidence_kind(evidence: &Value, kind: study::spec::StudyKind) -> Result<()> {
    match evidence.get("kind").and_then(Value::as_str) {
        None => Ok(()),
        Some(declared) => {
            let expected = match kind {
                study::spec::StudyKind::Tuning => "tuning",
                study::spec::StudyKind::Uncertainty => "uncertainty",
            };
            ensure!(
                declared == expected,
                "server comparison evidence is for a {declared} Study; this is a {expected} Study"
            );
            Ok(())
        }
    }
}

/// 目标观测的绝对路径在各机器上都不同：比较设计前把服务器清单里的目标源路径换成本机清单里同键目标的
/// 路径，观测内容另外按服务器 manifest 记录的 SHA 核对。键集合或结构不同仍由 `same_design` 报出来。
fn relocate_target_source_paths(
    remote: &mut study::spec::StudySpec,
    local: &study::spec::StudySpec,
) {
    for target in &mut remote.targets {
        let Some(local_target) = local.targets.iter().find(|t| t.key == target.key) else {
            continue;
        };
        if let (Some(source), Some(local_source)) = (&mut target.source, &local_target.source) {
            source.set_path(local_source.path().to_owned());
        }
    }
}

/// 设计是否一致：服务器设定里的观测路径（站点观测与目标观测）都是服务器端的绝对路径，不能直接比。
fn same_study_design(
    local: &study::spec::StudySpec,
    remote: &study::spec::StudySpec,
) -> Result<bool> {
    let mut remote = remote.clone();
    relocate_target_source_paths(&mut remote, local);
    crate::study_version::same_design(local, &remote)
}

/// 服务器建研究时冻结的观测内容与本机一致。`study-create` 在服务器上当场算这些 SHA；两边都从同一份
/// 上传字节算出，所以内容相同、路径不同也应当相等。这是 P1-7a 上传目标观测之后能核对的部分。
///
/// 不比较基础算例指纹：`fingerprint::compute` 把 namelist 里的路径字符串也算进去，服务器上的算例
/// 路径已改写，指纹必然不同；输入身份在另一条路径上核对。
fn ensure_remote_frozen_inputs(
    local: &study::spec::Manifest,
    remote: &study::spec::Manifest,
) -> Result<()> {
    let local_provenance = &local.provenance;
    let remote_provenance = &remote.provenance;
    ensure!(
        !local_provenance.samples_sha256.is_empty(),
        "this Study did not freeze its sample design identity; create a new Study"
    );
    ensure!(
        !remote_provenance.samples_sha256.is_empty(),
        "the server Study did not record its sample design identity; \
         its server-side CLI is too old to verify an uncertainty Study"
    );
    ensure!(
        local_provenance.samples_sha256 == remote_provenance.samples_sha256,
        "the server Study froze a different sample design from this Study"
    );
    ensure!(
        !remote_provenance.outputs_sha256.is_empty(),
        "the server Study did not record its history outputs identity; \
         its server-side CLI is too old to verify an uncertainty Study"
    );
    ensure!(
        local_provenance.outputs_sha256 == remote_provenance.outputs_sha256,
        "the server Study recorded different history outputs from this Study"
    );
    for (site, expected) in &local_provenance.observation_sha256 {
        ensure!(
            remote_provenance.observation_sha256.get(site) == Some(expected),
            "observation {site} differs between this Study and the one on the server"
        );
    }
    // 目标源列表：本机有、服务器没记录（旧版 CLI 建的服务器研究）时不能悄悄跳过——那正是上传目标观测
    // 要保证的东西。缺这个新字段就明确拒绝，让用户知道这份服务器证据不能核对。
    for (key, expected) in &local_provenance.target_source_sha256 {
        let recorded = remote_provenance.target_source_sha256.get(key);
        ensure!(
            recorded.is_some(),
            "the server Study did not record the identity of target observation {key}; \
             its server-side CLI predates target observation uploads, so the fetched results cannot \
             be verified against this Study"
        );
        ensure!(
            recorded == Some(expected),
            "target observation {key} differs between this Study and the one on the server"
        );
    }
    Ok(())
}

fn verified_evidence(
    manifest: &study::spec::Manifest,
    record: &StudyRecord,
    remote_root: &str,
    local_state: &study::state::StudyState,
    evidence: &Value,
) -> Result<study::model::StudyInputIdentity> {
    ensure_evidence_kind(evidence, manifest.spec.kind.clone())?;
    evidence_state(evidence, remote_root)?;
    // Compare as typed states: the transplant drops server-only fields (`process`), which the
    // local checkpoint serializes as null.
    let fetched: study::state::StudyState = serde_json::from_value(transplant_state(
        evidence["state"].clone(),
        &manifest.id,
        remote_root,
        Path::new(&manifest.root),
    ))?;
    ensure!(
        serde_json::to_value(local_state)? == serde_json::to_value(&fetched)?,
        "remote Study state changed since fetch; fetch the verified results before comparing"
    );
    let local_model = serde_json::to_value(
        crate::study_version::pinned(Path::new(&manifest.root))?.and_then(|pin| pin.model_revision),
    )?;
    ensure_model_identity(
        &local_model,
        &record.provenance["model_revision"],
        &evidence["model_revision"],
    )?;
    let remote_spec: study::spec::StudySpec = serde_json::from_value(evidence["spec"].clone())?;
    ensure!(
        same_study_design(&manifest.spec, &remote_spec)?,
        "server comparison design differs from the frozen local Study"
    );
    let local = study::model::verified_inputs(manifest)?;
    let remote: study::model::StudyInputIdentity =
        serde_json::from_value(evidence["input_identity"].clone())?;
    if remote
        .base_cases
        .values()
        .any(|identity| identity.algorithm == "sha256-nonforcing-v1")
    {
        ensure!(
            evidence["forcing_content_policy"] == "excluded-v1"
                && evidence["forcing_contents_verified"] == false,
            "remote scoped evidence must disclose unverified forcing contents"
        );
    }
    ensure!(
        study::model::same_scope_conditions(&local, &remote),
        "server scientific input or observation contents differ from the frozen local Study"
    );
    Ok(local)
}

/// 服务器结果树清单（`study-status` 的 `results`）：相对 `results/` 的路径与字节数。
/// 服务器只报清单不报内容哈希，内容身份在下面逐个成员按评估身份核对。
fn remote_result_tree_inventory(remote: &Value) -> Result<std::collections::BTreeMap<String, u64>> {
    let entries = remote["results"].as_array().with_context(|| {
        "the server Study status does not report its result tree; \
         its server-side CLI is too old to verify an uncertainty Study"
    })?;
    let mut files = std::collections::BTreeMap::new();
    for entry in entries {
        let path = entry["path"]
            .as_str()
            .context("the server result tree has an entry without a path")?;
        let bytes = entry["bytes"]
            .as_u64()
            .context("the server result tree has an entry without a size")?;
        ensure!(
            !path.is_empty()
                && Path::new(path)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "the server result tree contains an unsafe path: {path}"
        );
        files.insert(path.to_owned(), bytes);
    }
    Ok(files)
}

/// 下载/本机结果树里普通文件的相对路径与大小。拒绝链接与特殊文件：服务器控制的树在安装前先查一遍。
fn result_tree_inventory(root: &Path) -> Result<std::collections::BTreeMap<String, u64>> {
    fn visit(
        root: &Path,
        dir: &Path,
        files: &mut std::collections::BTreeMap<String, u64>,
    ) -> Result<()> {
        ensure!(
            std::fs::symlink_metadata(dir)?.file_type().is_dir(),
            "not a regular directory: {}",
            dir.display()
        );
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), files)?;
            } else {
                ensure!(
                    kind.is_file(),
                    "non-regular fetched result: {}",
                    entry.path().display()
                );
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    entry.metadata()?.len(),
                );
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

/// 有序的「相对路径 + 内容哈希」摘要；每个成员的结果树和整棵结果树都用它。
fn tree_digest(files: &std::collections::BTreeMap<String, String>) -> String {
    let mut text = String::new();
    for (path, sha256) in files {
        text.push_str(path);
        text.push('\0');
        text.push_str(sha256);
        text.push('\n');
    }
    colm_kernel::sha256_hex(text.as_bytes())
}

fn ensure_same_result_tree(
    server: &std::collections::BTreeMap<String, u64>,
    fetched: &std::collections::BTreeMap<String, u64>,
) -> Result<()> {
    let missing = server
        .keys()
        .filter(|path| !fetched.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    let extra = fetched
        .keys()
        .filter(|path| !server.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    ensure!(
        missing.is_empty() && extra.is_empty(),
        "the fetched result tree differs from the one the server reported \
         (missing: [{}], unexpected: [{}])",
        missing.join(", "),
        extra.join(", ")
    );
    let changed = server
        .iter()
        .filter(|(path, bytes)| fetched.get(*path) != Some(*bytes))
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    ensure!(
        changed.is_empty(),
        "fetched result files differ in size from the server's recorded tree: {}",
        changed.join(", ")
    );
    Ok(())
}

/// 逐个成员核对结果树：成功的任务必须有评估结果与评估身份，结果文件的字节身份必须与评估时记录的一致
/// （内容被替换、长度不变也能发现）。返回证据里的逐成员树哈希与整棵树的哈希。
fn verify_uncertainty_result_trees(
    manifest: &study::spec::Manifest,
    state: &study::state::StudyState,
    results_root: &Path,
    inventory: &std::collections::BTreeMap<String, u64>,
) -> Result<(std::collections::BTreeMap<String, Value>, String)> {
    let mut member_results = std::collections::BTreeMap::new();
    for member in &manifest.members {
        let mut files = std::collections::BTreeMap::new();
        for site in &manifest.spec.base_cases {
            let task = state
                .tasks
                .get(&study::state::task_id(&member.id, site))
                .with_context(|| {
                    format!(
                        "the server Study state has no task for {} / {site}",
                        member.id
                    )
                })?;
            let task_rel = format!("tasks/{}/{site}.json", member.id);
            let proof_rel = format!("provenance/{}/{site}.json", member.id);
            let present = inventory.contains_key(&task_rel);
            if task.status != study::state::TaskStatus::Succeeded {
                // 没成功的任务不该有评估结果；有就说明下载树与服务端状态对不上。
                ensure!(
                    !present,
                    "the fetched result tree contains an evaluation for {} / {site}, \
                     but its server task did not succeed",
                    member.id
                );
                continue;
            }
            ensure!(
                present && inventory.contains_key(&proof_rel),
                "the succeeded Study task {} / {site} has no evaluation result and identity \
                 in the fetched tree",
                member.id
            );
            let task_path = results_root.join(&task_rel);
            let bytes = std::fs::read(&task_path)?;
            let parsed: Value = serde_json::from_slice(&bytes).with_context(|| {
                format!("cannot parse the fetched evaluation result {task_rel}")
            })?;
            ensure!(
                parsed["member"].as_str() == Some(member.id.as_str())
                    && parsed["site"].as_str() == Some(site.as_str()),
                "the fetched evaluation result {task_rel} belongs to another member or site"
            );
            let sha256 = colm_kernel::sha256_hex(&bytes);
            let proof_path = results_root.join(&proof_rel);
            let proof: Value =
                serde_json::from_slice(&std::fs::read(&proof_path)?).with_context(|| {
                    format!("cannot parse the fetched evaluation identity {proof_rel}")
                })?;
            ensure!(
                proof["schema"] == 1,
                "unsupported fetched evaluation identity schema"
            );
            ensure!(
                proof["result"]["sha256"].as_str() == Some(sha256.as_str())
                    && proof["result"]["bytes"].as_u64() == Some(bytes.len() as u64),
                "the fetched evaluation result {task_rel} differs from the identity recorded \
                 when it was computed"
            );
            files.insert(task_rel, sha256);
            files.insert(proof_rel, crate::fingerprint::sha256_file(&proof_path)?);
        }
        member_results.insert(
            member.id.clone(),
            json!({
                "generation": member.generation,
                "tree_sha256": tree_digest(&files),
                "files": files,
            }),
        );
    }
    // 整棵树的内容哈希：服务器只给文件清单，内容由上面逐个文件算出来。审计时能拿它与下载树对照。
    let mut all = std::collections::BTreeMap::new();
    for relative in inventory.keys() {
        all.insert(
            relative.clone(),
            crate::fingerprint::sha256_file(&results_root.join(relative))?,
        );
    }
    Ok((member_results, tree_digest(&all)))
}

/// 状态候选表：任务覆盖全部成员/站点、候选的成员与代数对得上、有分数的候选必须每个站点都成功。
/// 不确定性研究按设计不写最优成员，这里不读、也不要求 `best_member`。
fn verify_uncertainty_candidates(
    manifest: &study::spec::Manifest,
    state: &study::state::StudyState,
) -> Result<()> {
    ensure!(
        matches!(
            state.status,
            study::state::StudyStatus::Completed | study::state::StudyStatus::CompletedWithFailures
        ),
        "the server Study is not in a finished state"
    );
    ensure!(
        state.tasks.len() == manifest.members.len() * manifest.spec.base_cases.len(),
        "the server Study state tasks do not cover every sampled member and site"
    );
    let members = manifest
        .members
        .iter()
        .map(|member| (member.id.as_str(), member))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (id, task) in &state.tasks {
        let member = members.get(task.member.as_str()).with_context(|| {
            format!(
                "the server Study state references unknown member {}",
                task.member
            )
        })?;
        ensure!(
            manifest.spec.base_cases.contains(&task.site),
            "the server Study state references unknown site {}",
            task.site
        );
        ensure!(
            *id == study::state::task_id(&member.id, &task.site),
            "the server Study state task identity differs from its content"
        );
    }
    for (id, candidate) in &state.candidates {
        let member = members.get(id.as_str()).with_context(|| {
            format!("the server Study state has a candidate for unknown member {id}")
        })?;
        ensure!(
            candidate.generation == member.generation,
            "the server Study candidate {id} has a different generation from its sample"
        );
        if candidate.feasible && !manifest.spec.targets.is_empty() {
            ensure!(
                candidate.calibration.is_some(),
                "the server Study candidate {id} is feasible without a recorded objective"
            );
        }
        if candidate.calibration.is_some() || candidate.validation.is_some() {
            for site in &manifest.spec.base_cases {
                let task = state
                    .tasks
                    .get(&study::state::task_id(id, site))
                    .with_context(|| format!("the server Study has no task for {id} / {site}"))?;
                ensure!(
                    task.status == study::state::TaskStatus::Succeeded,
                    "the server Study candidate {id} recorded an objective but its {site} task \
                     did not succeed"
                );
            }
        }
    }
    Ok(())
}

/// 不确定性研究的取回证据：样本 SHA、成员集合/顺序、每个成员的结果树、状态候选表。
///
/// 为什么不能走率定的比较核验：不确定性研究按设计没有最优成员，`study-verify-comparison` 要求
/// `best_member` 与基线比较，绑定模型修订的取回会被它整体拒绝。这里按同一份服务器状态与下载字节
/// 核对集合证据，不读 `best_member`，证据里也不声称最优。原始 history 留在服务器上，所以不重算
/// 分数、不假装复核了原始输出——只核对下载内容与服务器记录的评估身份、结果树清单一致。
fn uncertainty_fetch_evidence(
    manifest: &study::spec::Manifest,
    remote_root: &str,
    remote: &Value,
    study_root: &Path,
    installed_state: &study::state::StudyState,
) -> Result<Value> {
    ensure!(
        manifest.spec.kind == study::spec::StudyKind::Uncertainty,
        "uncertainty fetch evidence is only for uncertainty Studies"
    );
    let remote_manifest_value = remote
        .get("manifest")
        .filter(|value| value.is_object())
        .context(
            "the server Study status does not report its manifest; its server-side CLI is too old \
             to verify an uncertainty Study",
        )?;
    let remote_manifest: study::spec::Manifest =
        serde_json::from_value(remote_manifest_value.clone())
            .context("the server Study manifest is not readable here")?;
    ensure!(
        remote_manifest.spec.kind == manifest.spec.kind,
        "the server Study is a different kind of Study"
    );
    ensure_remote_frozen_inputs(manifest, &remote_manifest)?;
    ensure!(
        same_study_design(&manifest.spec, &remote_manifest.spec)?,
        "server Study design differs from the frozen local Study"
    );
    let remote_state_value = remote
        .get("state")
        .filter(|state| !state.is_null())
        .cloned()
        .context("the server Study has no state yet")?;
    let remote_state: study::state::StudyState = serde_json::from_value(remote_state_value.clone())
        .context("the server Study state is not readable here")?;
    ensure!(
        remote_state.schema_version == 1 && remote_state.study_id == remote_manifest.id,
        "the server Study state identity differs from its manifest"
    );
    let state_sha256 = colm_kernel::sha256_hex(&serde_json::to_vec(&remote_state)?);
    let transplanted: study::state::StudyState = serde_json::from_value(transplant_state(
        remote_state_value,
        &manifest.id,
        remote_root,
        Path::new(&manifest.root),
    ))?;
    ensure!(
        serde_json::to_value(installed_state)? == serde_json::to_value(&transplanted)?,
        "remote Study state changed since fetch; fetch the verified results before comparing"
    );
    // 样本设计：初始样本文件的 SHA 与成员顺序。成员集合/参数已由 `validate_fetched_samples` 建研究时核对，
    // 这里再用服务器证据核对一次，取回与复核两条路径都走同一段。
    let initial = if manifest.spec.method == study::spec::StudyMethod::DifferentialEvolution {
        "g000000.csv"
    } else {
        "design.csv"
    };
    let sample_path = study_root.join("samples").join(initial);
    let sample_bytes = std::fs::read(&sample_path).with_context(|| {
        format!(
            "the fetched Study has no initial samples {}",
            sample_path.display()
        )
    })?;
    let samples_sha256 = colm_kernel::sha256_hex(&sample_bytes);
    ensure!(
        !manifest.provenance.samples_sha256.is_empty()
            && samples_sha256 == manifest.provenance.samples_sha256,
        "fetched initial samples differ from the frozen local samples"
    );
    let sample_members = study::generation::read_sample_file(&sample_path)?;
    ensure!(
        serde_json::to_value(&sample_members)? == serde_json::to_value(&manifest.members)?,
        "the fetched sample members differ from the frozen Study (members or their order changed)"
    );
    let results_root = study_root.join("results");
    let server_inventory = remote_result_tree_inventory(remote)?;
    let fetched_inventory = result_tree_inventory(&results_root)?;
    ensure_same_result_tree(&server_inventory, &fetched_inventory)?;
    let (member_results, results_tree_sha256) = verify_uncertainty_result_trees(
        manifest,
        installed_state,
        &results_root,
        &fetched_inventory,
    )?;
    verify_uncertainty_candidates(manifest, installed_state)?;
    Ok(json!({
        "schema": 2,
        "kind": "uncertainty",
        "method": manifest.spec.method,
        "root": remote_root,
        "study_id": manifest.id,
        "state_sha256": state_sha256,
        "samples": {"initial": initial, "sha256": samples_sha256},
        "members": manifest.members.iter().map(|member| &member.id).collect::<Vec<_>>(),
        "members_sha256": colm_kernel::sha256_hex(&serde_json::to_vec(&manifest.members)?),
        "member_results": member_results,
        "results_tree_sha256": results_tree_sha256,
        "candidates": {
            "recorded": installed_state.candidates.len(),
            "succeeded_tasks": installed_state
                .tasks
                .values()
                .filter(|task| task.status == study::state::TaskStatus::Succeeded)
                .count(),
        },
        // 明确写清证据的边界：不确定性分析排的是参数影响，不产生最优成员，也不重算原始输出的分数。
        "selection": {
            "basis": "an uncertainty analysis ranks parameter effects; it does not select a best member",
        },
    }))
}

fn verify_result_tree(root: &Path, expected: &Value) -> Result<()> {
    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_value(expected.clone())?;
    ensure!(
        !expected.is_empty(),
        "server comparison evidence contains no evaluation results"
    );
    let actual = crate::study_version::comparison_results_sha256(
        root.parent().context("results have no Study parent")?,
    )?;
    ensure!(
        actual == expected,
        "fetched result bytes differ from the freshly verified server results"
    );
    Ok(())
}

/// 服务器侧核验用到的 SSH 操作。
///
/// 抽成 trait 是为了让"远程取回的研究写回版本"这条路径能用假实现单测（现有测试没有 SSH mock）；
/// 生产实现 [`Ssh`] 只做转发，行为与原先 `verified_comparison` 里直接调用完全一致。
pub(crate) trait StudyRemote {
    /// 核对作业目录里的研究/引擎身份。
    fn verify_identity(&self, record: &StudyRecord) -> Result<()>;
    fn job_status(&self, record: &StudyRecord) -> Result<job::Status>;
    /// 服务器上研究目录（作业建好研究之后才有）。
    fn study_root(&self, record: &StudyRecord) -> Result<Option<String>>;
    fn study_status(&self, record: &StudyRecord, study_root: &str) -> Result<Value>;
    /// 率定的服务器现场比较证据；不确定性研究没有这条命令。
    fn comparison_evidence(&self, record: &StudyRecord, study_root: &str) -> Result<Value>;
    /// 读服务器上的小文本文件（只用于核对样本/成员参数，不拉 history）。
    fn read_text_file(&self, path: &str) -> Result<String>;
}

impl StudyRemote for Ssh {
    fn verify_identity(&self, record: &StudyRecord) -> Result<()> {
        self.run_ok(&cancel_identity_script(record)?).map(|_| ())
    }

    fn job_status(&self, record: &StudyRecord) -> Result<job::Status> {
        job::status(self, &record.root, &record.job, 20)
    }

    fn study_root(&self, record: &StudyRecord) -> Result<Option<String>> {
        remote_study_root(self, record)
    }

    fn study_status(&self, record: &StudyRecord, study_root: &str) -> Result<Value> {
        remote_study_status(self, record, study_root)
    }

    fn comparison_evidence(&self, record: &StudyRecord, study_root: &str) -> Result<Value> {
        server_comparison_evidence(self, record, study_root)
    }

    fn read_text_file(&self, path: &str) -> Result<String> {
        self.run_ok(&format!("cat -- {}", quote(path)))
            .with_context(|| format!("cannot read {path} on {}", self.host))
    }
}

/// 复核一个远程研究需要先读的本机材料。
fn verified_comparison_inputs(
    study_dir: &Path,
) -> Result<(study::spec::Manifest, StudyRecord, study::state::StudyState)> {
    let manifest = study::engine::status(study_dir)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let record = read_record(study_dir)?;
    ensure!(
        record.fetched_at.is_some(),
        "remote Study has not been fetched"
    );
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study has no fetched checkpoint")?
    .payload;
    Ok((manifest, record, state))
}

/// 一次服务器侧核验的结论：服务器证据、服务器状态（已移植成本机路径）与可用的输入身份。
struct RemoteVerification {
    /// 率定比较时顺带算出的完整输入身份；不确定性走本机 `verified_inputs`，这里为空。
    identity: Option<study::model::StudyInputIdentity>,
    /// 服务器给出的证据（率定是 `study-verify-comparison` 的 schema 1，不确定性是取回同款 schema 2）。
    evidence: Value,
    /// 服务器状态；率定来自证据里的 `state`，不确定性来自本机检查点（已由证据核对与服务器一致）。
    state: study::state::StudyState,
    remote_root: String,
}

impl RemoteVerification {
    /// 调用方要比较用的输入身份：率定已经算过，不确定性按本机冻结输入算。
    fn input_identity(
        &self,
        manifest: &study::spec::Manifest,
    ) -> Result<study::model::StudyInputIdentity> {
        match &self.identity {
            Some(identity) => Ok(identity.clone()),
            None => study::model::verified_inputs(manifest),
        }
    }
}

/// 在原始输出所在的服务器上核验研究：作业成功、研究身份、成员集合/冻结输入、证据与结果树哈希。
/// 不下载 history；失败按"服务器不可达 / 作业没成功 / 研究没建出来 / 证据不符"给可行动错误。
fn remote_study_verification(
    manifest: &study::spec::Manifest,
    record: &StudyRecord,
    state: &study::state::StudyState,
    remote: &dyn StudyRemote,
) -> Result<RemoteVerification> {
    remote.verify_identity(record).with_context(|| {
        format!(
            "server {} is unreachable or its recorded job identity cannot be verified; \
             check the connection and the remote Study record",
            record.host
        )
    })?;
    let status = remote.job_status(record).with_context(|| {
        format!(
            "cannot read the recorded job status on {}; check the connection before writing \
             the Study member into a model version",
            record.host
        )
    })?;
    match status.state {
        job::State::Finished { exit_code: 0 } => {}
        job::State::Finished { exit_code } => bail!(
            "the server job failed with exit code {exit_code}; its outputs cannot be written into \
             a model version"
        ),
        _ => bail!(
            "the server job has not finished; write its result into a model version only after it ends"
        ),
    }
    let remote_root = remote.study_root(record)?.context(
        "the server job did not create the Study (its log says why); there is nothing to write \
         into a model version",
    )?;
    ensure!(
        study_under_project(&remote_root, std::slice::from_ref(&record.remote_project)),
        "remote Study root differs from its submitted project"
    );
    match manifest.spec.kind {
        study::spec::StudyKind::Tuning => {
            let evidence = remote
                .comparison_evidence(record, &remote_root)
                .context("cannot verify the original member outputs on the server")?;
            let identity = verified_evidence(manifest, record, &remote_root, state, &evidence)?;
            verify_result_tree(
                &Path::new(&manifest.root).join("results"),
                &evidence["results_sha256"],
            )?;
            let server_state: study::state::StudyState = serde_json::from_value(transplant_state(
                evidence["state"].clone(),
                &manifest.id,
                &remote_root,
                Path::new(&manifest.root),
            ))
            .context("the server Study state is not readable here")?;
            Ok(RemoteVerification {
                identity: Some(identity),
                evidence,
                state: server_state,
                remote_root,
            })
        }
        study::spec::StudyKind::Uncertainty => {
            let server = remote
                .study_status(record, &remote_root)
                .context("cannot read the server Study status to verify the original outputs")?;
            let local_model = serde_json::to_value(
                crate::study_version::pinned(Path::new(&manifest.root))?
                    .and_then(|pin| pin.model_revision),
            )?;
            ensure_model_identity(
                &local_model,
                &record.provenance["model_revision"],
                &server["model_version"]["model_revision"],
            )?;
            let evidence = uncertainty_fetch_evidence(
                manifest,
                &remote_root,
                &server,
                Path::new(&manifest.root),
                state,
            )?;
            Ok(RemoteVerification {
                identity: None,
                evidence,
                state: state.clone(),
                remote_root,
            })
        }
    }
}

/// Reverify raw server outputs for every comparison, using the recorded job's fixed CLI.
///
/// 按研究类型分流：率定仍要服务器现场比较最优成员与基线（`study-verify-comparison`）；不确定性研究
/// 没有最优成员，那条命令按设计会拒绝，改用服务器 `study-status` 报告的清单与本机下载树核对集合证据。
pub(crate) fn verified_comparison(study_dir: &Path) -> Result<study::model::StudyInputIdentity> {
    let (manifest, record, state) = verified_comparison_inputs(study_dir)?;
    let ssh = Ssh::new(&record.host)?;
    let verified = remote_study_verification(&manifest, &record, &state, &ssh)?;
    verified.input_identity(&manifest)
}

/// 写回版本前的成员核验结果。
///
/// `lock` 是研究锁：本地分支持到写回结束（与原先 `verified_member` 的语义一致），远程分支也持有，
/// 防止核验期间本机又启动一次研究运行。`evidence` 是可追溯证据（成员、站点、结果文件哈希、
/// 结果树哈希、状态哈希、job id 与服务器）。
pub(crate) struct VerifiedStudyMember {
    pub(crate) manifest: study::spec::Manifest,
    pub(crate) member: study::spec::MemberPlan,
    pub(crate) objective: Option<f64>,
    pub(crate) evidence: Value,
    pub(crate) lock: Option<study::runner::StudyRunLock>,
}

// 研究锁不实现 Debug（里面有心跳线程句柄）；错误路径的 unwrap_err/assert 需要这个类型可打印。
impl std::fmt::Debug for VerifiedStudyMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedStudyMember")
            .field("manifest", &self.manifest)
            .field("member", &self.member)
            .field("objective", &self.objective)
            .field("evidence", &self.evidence)
            .finish_non_exhaustive()
    }
}

/// 统一的本机/远端成员核验入口：研究目录里有 `.colm-remote-study.json` 就走服务器核验，
/// 否则走现有本地 `verified_member`（要求本机成员目录里有原始输出与评估证据）。
///
/// 远程分支绝不"本地目录不存在就跳过核验"：本机没有原始输出时只能以服务器记录为准，记录缺失或
/// 不可读都明确报错。
pub(crate) fn verified_study_member(
    study_dir: &Path,
    member_id: &str,
) -> Result<VerifiedStudyMember> {
    verified_study_member_with(study_dir, member_id, None)
}

pub(crate) fn verified_study_member_with(
    study_dir: &Path,
    member_id: &str,
    injected: Option<&dyn StudyRemote>,
) -> Result<VerifiedStudyMember> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    if !study_dir.join(RECORD).is_file() {
        return verified_study_member_local(&study_dir, member_id);
    }
    let record = read_record(&study_dir)
        .context("cannot read this Study's remote submission record; fetch the Study again before writing its member into a model version")?;
    match injected {
        Some(remote) => verified_study_member_remote(&study_dir, member_id, &record, remote),
        None => {
            let ssh = Ssh::new(&record.host)
                .with_context(|| format!("cannot connect to the server {}", record.host))?;
            verified_study_member_remote(&study_dir, member_id, &record, &ssh)
        }
    }
}

fn verified_study_member_local(study_dir: &Path, member_id: &str) -> Result<VerifiedStudyMember> {
    match verified_study_member_local_inner(study_dir, member_id) {
        Ok(verified) => Ok(verified),
        Err(error) if study_dir.join(EVIDENCE).is_file() => Err(error.context(
            "this Study's original outputs are on a server, but its remote submission record \
             (.colm-remote-study.json) is missing; fetch the Study again (or restore the record) \
             before writing its member into a model version",
        )),
        Err(error) => Err(error),
    }
}

fn verified_study_member_local_inner(
    study_dir: &Path,
    member_id: &str,
) -> Result<VerifiedStudyMember> {
    // 本地分支原样复用 runner::verified_member：研究锁、冻结输入、成员资格与原始输出证据都由它核对。
    let (lock, manifest, member) = study::runner::verified_member(study_dir, member_id)?;
    let state =
        study::runner::status_state(study_dir)?.context("Study has no evaluated member state")?;
    let objective = state
        .candidates
        .get(&member.id)
        .and_then(|candidate| candidate.calibration);
    let results_sha256 = crate::study_version::comparison_results_sha256(study_dir)?;
    let (tasks, results) =
        member_result_files(&manifest, &member, study_dir, &results_sha256, false)?;
    let evidence = json!({
        "schema": 1,
        "location": {"kind": "local", "root": manifest.root},
        "study_id": manifest.id,
        "study_kind": manifest.spec.kind,
        "member": {"id": member.id, "generation": member.generation, "baseline": member.baseline},
        "sites": manifest.spec.base_cases,
        "tasks": tasks,
        "results": results,
        "results_tree_sha256": tree_digest(&results_sha256),
        "state_sha256": colm_kernel::sha256_hex(&serde_json::to_vec(&state)?),
        "model_revision": serde_json::to_value(
            crate::study_version::pinned(study_dir)?.and_then(|pin| pin.model_revision),
        )?,
    });
    Ok(VerifiedStudyMember {
        manifest,
        member,
        objective,
        evidence,
        lock: Some(lock),
    })
}

fn verified_study_member_remote(
    study_dir: &Path,
    member_id: &str,
    record: &StudyRecord,
    remote: &dyn StudyRemote,
) -> Result<VerifiedStudyMember> {
    let lock = study::runner::StudyRunLock::acquire(study_dir)
        .context("cannot lock the Study while verifying its member for writing back")?;
    let manifest = study::engine::status(study_dir)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let member = study::runner::resolve_apply_member(study_dir, &manifest, member_id)?;
    let state =
        study::runner::status_state(study_dir)?.context("Study has no evaluated member state")?;
    let server = remote_study_verification(&manifest, record, &state, remote)?;
    // 服务器状态（已核对与本机检查点一致）里这个成员的任务必须全部成功；否则没有可写回的原始输出。
    for site in &manifest.spec.base_cases {
        let task = server
            .state
            .tasks
            .get(&study::state::task_id(&member.id, site))
            .with_context(|| {
                format!(
                    "the server Study has no task for member {} / {site}; its output is missing \
                     on the server",
                    member.id
                )
            })?;
        ensure!(
            task.status == study::state::TaskStatus::Succeeded,
            "Study member {} did not succeed for {site} on the server ({}); its output cannot be \
             written into a model version",
            member.id,
            task.reason.as_deref().unwrap_or("no reason recorded")
        );
    }
    let objective = server
        .state
        .candidates
        .get(&member.id)
        .and_then(|candidate| candidate.calibration);
    let hashes = member_result_hashes(&server.evidence, &manifest.spec.kind, &member.id)?;
    let (tasks, results) = member_result_files(&manifest, &member, study_dir, &hashes, true)?;
    verify_remote_member_parameters(&manifest, &member, &server.remote_root, remote)?;
    let stored_evidence = stored_evidence_cross_check(study_dir, &server.evidence)?;
    let local_model = serde_json::to_value(
        crate::study_version::pinned(study_dir)?.and_then(|pin| pin.model_revision),
    )?;
    let results_tree_sha256 = match server.evidence["schema"].as_u64() {
        Some(1) => {
            let hashes: std::collections::BTreeMap<String, String> =
                serde_json::from_value(server.evidence["results_sha256"].clone())?;
            json!(tree_digest(&hashes))
        }
        _ => server.evidence["results_tree_sha256"].clone(),
    };
    let basis = if manifest.spec.kind == study::spec::StudyKind::Tuning {
        "server-study-verify-comparison"
    } else {
        "server-study-status+fetch-evidence"
    };
    let evidence = json!({
        "schema": 1,
        "location": {
            "kind": "server",
            "host": record.host,
            "root": record.root,
            "remote_project": record.remote_project,
            "job": record.job,
            "scheduler": record.scheduler,
        },
        "study_id": manifest.id,
        "study_kind": manifest.spec.kind,
        "member": {"id": member.id, "generation": member.generation, "baseline": member.baseline},
        "sites": manifest.spec.base_cases,
        "tasks": tasks,
        "results": results,
        "results_tree_sha256": results_tree_sha256,
        "state_sha256": server.evidence["state_sha256"],
        "model_revision": local_model,
        "stored_evidence": stored_evidence,
        "verification": {"basis": basis},
    });
    Ok(VerifiedStudyMember {
        manifest,
        member,
        objective,
        evidence,
        lock: Some(lock),
    })
}

/// 服务器证据里这个成员的结果文件内容哈希。率定证据覆盖整棵结果树；不确定性证据只按成员记。
fn member_result_hashes(
    evidence: &Value,
    kind: &study::spec::StudyKind,
    member_id: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    let value = match kind {
        study::spec::StudyKind::Tuning => evidence.get("results_sha256").cloned(),
        study::spec::StudyKind::Uncertainty => evidence
            .get("member_results")
            .and_then(|members| members.get(member_id))
            .and_then(|entry| entry.get("files"))
            .cloned(),
    };
    let value = value.with_context(|| {
        format!(
            "the server evidence contains no verified result files for Study member {member_id}"
        )
    })?;
    let hashes: std::collections::BTreeMap<String, String> =
        serde_json::from_value(value).context("the server result file hashes are not readable")?;
    ensure!(
        !hashes.is_empty(),
        "the server evidence contains no verified result files for Study member {member_id}; \
         the member output is missing on the server"
    );
    Ok(hashes)
}

/// 核对成员在本机下载的结果树：任务结果与评估身份都在证据里，且本机字节与证据哈希一致。
/// 返回证据里可追溯的 `tasks`/`results` 片段。
fn member_result_files(
    manifest: &study::spec::Manifest,
    member: &study::spec::MemberPlan,
    study_dir: &Path,
    hashes: &std::collections::BTreeMap<String, String>,
    remote: bool,
) -> Result<(Value, Value)> {
    let mut tasks = serde_json::Map::new();
    let mut results = serde_json::Map::new();
    let results_root = study_dir.join("results");
    for site in &manifest.spec.base_cases {
        let task_rel = format!("tasks/{}/{site}.json", member.id);
        let proof_rel = format!("provenance/{}/{site}.json", member.id);
        let missing = || {
            if remote {
                format!(
                    "the server evidence has no evaluation result for Study member {} / {site}; \
                     the member output is missing on the server (fetch the Study again)",
                    member.id
                )
            } else {
                format!(
                    "the Study has no evaluation result for member {} / {site}",
                    member.id
                )
            }
        };
        let (task_sha, proof_sha) = match (hashes.get(&task_rel), hashes.get(&proof_rel)) {
            (Some(task_sha), Some(proof_sha)) => (task_sha, proof_sha),
            _ => bail!("{}", missing()),
        };
        let task_path = results_root.join(&task_rel);
        let local_task_sha = crate::fingerprint::sha256_file(&task_path).with_context(|| {
            format!(
                "cannot read the fetched evaluation result {}; fetch the Study again",
                task_path.display()
            )
        })?;
        ensure!(
            local_task_sha == *task_sha,
            "the fetched evaluation result for member {} / {site} differs from the verified \
             server bytes; fetch the Study again before writing the member into a model version",
            member.id
        );
        let proof_path = results_root.join(&proof_rel);
        let local_proof_sha = crate::fingerprint::sha256_file(&proof_path).with_context(|| {
            format!(
                "cannot read the fetched evaluation identity {}; fetch the Study again",
                proof_path.display()
            )
        })?;
        ensure!(
            local_proof_sha == *proof_sha,
            "the fetched evaluation identity for member {} / {site} differs from the verified \
             server bytes; fetch the Study again before writing the member into a model version",
            member.id
        );
        let proof: Value = serde_json::from_slice(&std::fs::read(&proof_path)?)
            .with_context(|| format!("cannot parse the fetched evaluation identity {proof_rel}"))?;
        ensure!(
            proof["schema"] == 1,
            "unsupported fetched evaluation identity schema"
        );
        ensure!(
            proof["result"]["sha256"].as_str() == Some(task_sha.as_str()),
            "the evaluation identity of member {} / {site} does not match its result file",
            member.id
        );
        tasks.insert(site.clone(), json!({"status": "succeeded"}));
        results.insert(
            site.clone(),
            json!({
                "task": {"path": task_rel, "sha256": task_sha},
                "proof": {"path": proof_rel, "sha256": proof_sha},
            }),
        );
    }
    Ok((Value::Object(tasks), Value::Object(results)))
}

/// 成员的参数必须与服务器评估时用的样本文件逐字节一致（只读一个小 CSV，不碰 history）。
fn verify_remote_member_parameters(
    manifest: &study::spec::Manifest,
    member: &study::spec::MemberPlan,
    remote_root: &str,
    remote: &dyn StudyRemote,
) -> Result<()> {
    let file = member_sample_file(&manifest.spec, member);
    let local_path = Path::new(&manifest.root).join("samples").join(&file);
    let local = std::fs::read(&local_path).with_context(|| {
        format!(
            "cannot read the frozen sample file {}; fetch the Study again",
            local_path.display()
        )
    })?;
    let remote_path = format!("{remote_root}/samples/{file}");
    let remote_text = remote.read_text_file(&remote_path).with_context(|| {
        format!("cannot read the server's sample file {file} to compare the member parameters")
    })?;
    ensure!(
        remote_text.as_bytes() == local.as_slice(),
        "the member parameters in this Study's samples differ from the sample file the server \
         evaluated; fetch the Study again before writing the member into a model version"
    );
    Ok(())
}

/// 成员参数所在的样本文件。DE 的初代在 `g000000.csv`，后续代在 `g%06d.csv`；其它方法是 `design.csv`。
fn member_sample_file(spec: &study::spec::StudySpec, member: &study::spec::MemberPlan) -> String {
    if member.generation > 0 {
        format!("g{:06}.csv", member.generation)
    } else if spec.method == study::spec::StudyMethod::DifferentialEvolution {
        "g000000.csv".into()
    } else {
        "design.csv".into()
    }
}

/// 与取回时写下的证据交叉核对：服务器研究在上次取回后不能变过，否则本机结果不是它现在的输出。
fn stored_evidence_cross_check(study_dir: &Path, fresh: &Value) -> Result<&'static str> {
    let path = study_dir.join(EVIDENCE);
    if !path.is_file() {
        return Ok("absent");
    }
    let stored: Value = serde_json::from_slice(&std::fs::read(&path)?)
        .context("cannot parse the recorded remote Study evidence; fetch the Study again")?;
    ensure!(
        stored["schema"] == fresh["schema"],
        "the recorded fetch evidence has a different format from the server's current evidence; \
         fetch the Study again before writing its member into a model version"
    );
    for key in ["root", "state_sha256"] {
        ensure!(
            stored[key] == fresh[key],
            "the recorded fetch evidence differs from the server's current evidence ({key}); \
             the Study changed since it was fetched — fetch it again before writing its member \
             into a model version"
        );
    }
    match fresh["schema"].as_u64() {
        Some(1) => ensure!(
            stored["results_sha256"] == fresh["results_sha256"],
            "the recorded fetch evidence's result tree differs from the server's current verified \
             results; fetch the Study again before writing its member into a model version"
        ),
        Some(2) => ensure!(
            stored["results_tree_sha256"] == fresh["results_tree_sha256"],
            "the recorded fetch evidence's result tree differs from the server's current verified \
             results; fetch the Study again before writing its member into a model version"
        ),
        _ => bail!("unsupported remote Study evidence schema"),
    }
    Ok("matched")
}

/// `remote-study-status`：作业状态，加上服务器上研究的进度（建好之后）。
/// 只有提交意图（`PENDING`）、没有正式记录时怎么办（GUI 审查 G-05）。原来这种研究只能看状态：作业已经到了服务器、
/// 本机记录没写成时，运行中取消不了、结束了取不回，服务器白耗机时。
enum PendingOutcome {
    /// 没有待核对的提交。
    None,
    /// 另一个进程正在提交（它持有研究锁）：别动。
    Busy,
    /// 服务器上确有这个作业，而且身份对得上：补写正式记录，之后取消、取回照常。
    Adopted,
    /// 服务器上没有这个作业目录：提交没到服务器。意图改名留档，可以重新提交。
    NeverSubmitted(PathBuf),
}

/// 服务器探测的输出：`colm-job-probe=present|absent`，作业目录在时再给调度系统与作业号（没有调度系统时两者都没有，
/// 按直接后台运行）。用带前缀的标记行、在整段输出里找：集群的登录脚本可能先打印横幅。认不出时返回 `None`，不猜。
fn parse_job_probe(text: &str) -> Option<(bool, String, Option<String>)> {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    match field("colm-job-probe=")?.as_str() {
        "absent" => Some((false, String::new(), None)),
        "present" => Some((
            true,
            field("colm-job-scheduler=").unwrap_or_else(|| "bare".into()),
            field("colm-job-sched-id="),
        )),
        _ => None,
    }
}

fn adopt_pending(study_dir: &Path) -> Result<PendingOutcome> {
    if study_dir.join(RECORD).exists() || !study_dir.join(PENDING).exists() {
        return Ok(PendingOutcome::None);
    }
    // 提交全程持有研究锁：拿不到锁就是还在提交，这时没有作业目录也不能当成"没到服务器"，否则会重复提交。
    let Ok(_lock) = study::runner::StudyRunLock::acquire(study_dir) else {
        return Ok(PendingOutcome::Busy);
    };
    if study_dir.join(RECORD).exists() {
        return Ok(PendingOutcome::None);
    }
    let pending = read_pending(study_dir)?;
    colm_remote::auth::check_connection(pending.ssh_auth.as_ref())?;
    let ssh = Ssh::new(&pending.host)?;
    let dir = quote(&job::job_dir(&pending.root, &pending.job));
    let probe = ssh.run_ok(&format!(
        "if [ -d {dir} ]; then echo colm-job-probe=present; cat {dir}/scheduler 2>/dev/null | head -n 1 | sed 's/^/colm-job-scheduler=/'; cat {dir}/sched_id 2>/dev/null | head -n 1 | sed 's/^/colm-job-sched-id=/'; else echo colm-job-probe=absent; fi"
    ))?;
    let Some((present, scheduler, sched_id)) = parse_job_probe(&probe) else {
        bail!(
            "cannot tell whether the pending Study job reached {}",
            pending.host
        );
    };
    if !present {
        let target = study_dir.join(format!(
            ".colm-remote-study-pending-never-submitted-{}.json",
            now()
        ));
        std::fs::rename(study_dir.join(PENDING), &target)
            .with_context(|| format!("cannot archive {}", PENDING))?;
        return Ok(PendingOutcome::NeverSubmitted(target));
    }
    ssh.run_ok(&pending_identity_script(&pending)?)?;
    ensure!(
        matches!(scheduler.as_str(), "bare" | "slurm" | "pbs" | "lsf"),
        "the pending Study job has an unknown scheduler {scheduler:?}"
    );
    let engine = pending
        .engine
        .clone()
        .context("the pending Study submission did not record its engine snapshot")?;
    let mut provenance = pending.provenance.clone();
    if !provenance.is_object() {
        provenance = json!({});
    }
    provenance["schema"] = json!(1);
    provenance["adopted_from_pending"] = json!(true);
    provenance["model_revision"] = serde_json::to_value(
        crate::study_version::pinned(study_dir)?.and_then(|pin| pin.model_revision),
    )?;
    provenance["location"] = json!({ "kind": "server", "host": pending.host });
    provenance["engine_snapshot"] = json!(engine);
    let record = StudyRecord {
        ssh_auth: pending.ssh_auth.clone(),
        host: pending.host.clone(),
        root: pending.root.clone(),
        job: pending.job.clone(),
        remote_project: pending.remote_project.clone(),
        engine,
        submitted_at: now(),
        scheduler,
        scheduler_id: sched_id,
        provenance,
        fetched_at: None,
    };
    std::fs::write(
        study_dir.join(RECORD),
        serde_json::to_string_pretty(&record)?,
    )?;
    std::fs::remove_file(study_dir.join(PENDING))?;
    Ok(PendingOutcome::Adopted)
}

pub(crate) fn cmd_status(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    match adopt_pending(&study_dir)? {
        PendingOutcome::NeverSubmitted(archived) => {
            println!(
                "{}",
                json!({"record":Value::Null,"pending":Value::Null,"outcome":"never_submitted","archived":archived,"status":{"state":"unknown"},"study":Value::Null})
            );
            return Ok(());
        }
        PendingOutcome::None | PendingOutcome::Busy | PendingOutcome::Adopted => {}
    }
    if !study_dir.join(RECORD).exists() {
        let pending = read_pending(&study_dir)?;
        colm_remote::auth::check_connection(pending.ssh_auth.as_ref())?;
        let ssh = Ssh::new(&pending.host)?;
        // A saved intent may precede actual submission. Never run or submit it here.
        ssh.run_ok(&pending_identity_script(&pending)?)?;
        let status = job::status(&ssh, &pending.root, &pending.job, 40)?;
        let outcome = json!({"job":pending.job,"outcome":"observed","terminal":matches!(status.state, job::State::Finished{..}),"status":status});
        crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&pending)?, &outcome)?;
        println!(
            "{}",
            json!({"record":Value::Null,"pending":pending_status(&study_dir)?,"status":status,"study":Value::Null})
        );
        return Ok(());
    }
    let record = read_record(&study_dir)?;
    let ssh = Ssh::new(&record.host)?;
    ssh.run_ok(&cancel_identity_script(&record)?)?;
    let status = job::status(&ssh, &record.root, &record.job, 40)?;
    let progress = match remote_study_root(&ssh, &record)? {
        Some(root) => remote_study_status(&ssh, &record, &root).ok().map(|value| {
            json!({
                "status": value["state"]["status"],
                "generation": value["state"]["generation"],
                "completed_candidates": value["state"]["completed_candidates"],
                "best_member": value["state"]["best_member"],
                "best_objective": value["state"]["best_objective"],
            })
        }),
        None => None,
    };
    let outcome = json!({"job":record.job,"outcome":"observed","terminal":matches!(status.state, job::State::Finished{..}),"status":status});
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &outcome)?;
    let mut public_record = serde_json::to_value(&record)?;
    public_record
        .as_object_mut()
        .context("record is not an object")?
        .remove("ssh_auth");
    println!(
        "{}",
        json!({ "record": public_record, "status": status, "study": progress })
    );
    Ok(())
}

/// Cancel only the job named by this Study's captured connection and submission record.
pub(crate) fn cmd_cancel(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    // 只有提交意图时先核对补记，否则作业到了服务器也取消不了（G-05）。要在拿研究锁之前做。
    if let PendingOutcome::NeverSubmitted(archived) = adopt_pending(&study_dir)? {
        println!(
            "{}",
            json!({"outcome":"never_submitted","archived":archived,"request_accepted":false,"terminal":true})
        );
        return Ok(());
    }
    let _lock = study::runner::StudyRunLock::acquire(&study_dir)?;
    study::engine::status(&study_dir)?;
    let record = read_record(&study_dir)?;
    let identity = cancel_identity_script(&record)?;
    let ssh = Ssh::new(&record.host)?;
    let result = cancel_job(&ssh, &record.root, &record.job, &identity)?;
    crate::history_cmd::observe_remote_outcome(&result);
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &result)?;
    println!("{result}");
    Ok(())
}

/// `remote-study-close`：服务器作业已经丢失、状态无法再核对时，由用户确认后关闭本机远程记录。
///
/// 只允许 Lost/Unknown：排队中/运行中的作业必须先取消；已经正常结束的应取回结果，不能当作丢失丢掉。
/// 记录改名留档（`.colm-remote-study.closed-*.json`），本机研究之后可重新提交或在本机运行。
pub(crate) fn cmd_close(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let _lock = study::runner::StudyRunLock::acquire(&study_dir)?;
    study::engine::status(&study_dir)?;
    let record_path = study_dir.join(RECORD);
    let pending_path = study_dir.join(PENDING);
    ensure!(
        record_path.exists() || pending_path.exists(),
        "this Study has no server submission to close"
    );
    let (job, status) = if record_path.exists() {
        let record = read_record(&study_dir)?;
        let ssh = Ssh::new(&record.host)?;
        ssh.run_ok(&cancel_identity_script(&record)?)?;
        let status = job::status(&ssh, &record.root, &record.job, 20)?;
        (record.job, status)
    } else {
        let pending = read_pending(&study_dir)?;
        colm_remote::auth::check_connection(pending.ssh_auth.as_ref())?;
        let ssh = Ssh::new(&pending.host)?;
        ssh.run_ok(&pending_identity_script(&pending)?)?;
        let status = job::status(&ssh, &pending.root, &pending.job, 20)?;
        (pending.job, status)
    };
    closable(&status)?;
    let stamp = now();
    let mut archived = Vec::new();
    for (path, suffix) in [(record_path, ""), (pending_path, "-pending")] {
        if path.exists() {
            let target =
                study_dir.join(format!(".colm-remote-study{}-closed-{stamp}.json", suffix));
            std::fs::rename(&path, &target)
                .with_context(|| format!("cannot archive {}", path.display()))?;
            archived.push(target);
        }
    }
    println!(
        "{}",
        json!({
            "outcome": "closed",
            "job": job,
            "previous_state": status,
            "archived": archived,
        })
    );
    Ok(())
}

/// 远程研究记录能不能当"丢失"关掉：排队或运行中的要先取消，正常结束的要取回结果；调度系统说已经结束、
/// 退出码还没出现的（NFS 延迟）要等一会儿再查。只有真正查不到的 Lost/Unknown 才能关。
fn closable(status: &job::Status) -> Result<()> {
    match status.state {
        job::State::Queued | job::State::Running => {
            bail!("the server job is still queued or running; cancel it before closing the record")
        }
        job::State::Finished { .. } => {
            bail!("the server job finished; fetch its results instead of closing the record")
        }
        // 调度系统说作业已经结束（Slurm 的 COMPLETED）却还没看到退出码：多半是计算节点写下的 exit_code
        // 还没经 NFS 传到登录节点（2026-10-23 land02 实测，过一会儿就出现）。这时当成丢失关掉，跑完的
        // 结果就再也取不回来；要求等一会儿重新查看状态。
        job::State::Lost
            if status
                .detail
                .as_deref()
                .is_some_and(|detail| detail.to_ascii_uppercase().contains("COMPLETED")) =>
        {
            bail!("the scheduler reports the job as completed but its exit code is not visible yet (shared file systems can lag); check the status again in a minute and fetch the results instead of closing the record")
        }
        job::State::Lost | job::State::Unknown => {}
    }
    Ok(())
}

pub(super) fn cancel_job(ssh: &Ssh, root: &str, id: &str, identity: &str) -> Result<Value> {
    let mut lease = colm_remote::engine_inventory::LifecycleLock::acquire(ssh, root)?;
    let before = lease.run(|| {
        ssh.run_ok(identity)?;
        job::status(ssh, root, id, 20)
    })?;
    if matches!(before.state, job::State::Finished { .. }) {
        lease.finish()?;
        return Ok(cancellation_result(id, false, Some(&before), None, None));
    }
    if !matches!(before.state, job::State::Queued | job::State::Running) {
        lease.finish()?;
        bail!("remote Study job identity is not live; inspect its status before cancelling");
    }
    let requested = lease.run(|| {
        ssh.run_ok(&format!(
            "{identity}\n{cancel}",
            cancel = job::cancel_script(root, id),
        ))?;
        Ok(())
    });
    let accepted = requested.is_ok();
    let cancel_error = requested.err().map(|error| format!("{error:#}"));
    let observed = job::status(ssh, root, id, 20);
    let status_error = observed.as_ref().err().map(|error| format!("{error:#}"));
    let mut result = cancellation_result(
        id,
        accepted,
        observed.as_ref().ok(),
        cancel_error,
        status_error,
    );
    // Unknown SSH outcomes leave the existing lease for explicit recovery.
    if accepted {
        if let Err(error) = lease.finish() {
            result["lock_warning"] = json!(format!("{error:#}"));
        }
    }
    Ok(result)
}

fn cancellation_result(
    job: &str,
    accepted: bool,
    status: Option<&job::Status>,
    cancel_error: Option<String>,
    status_error: Option<String>,
) -> Value {
    let outcome = if cancel_error.is_some() || status_error.is_some() {
        "uncertain"
    } else if accepted {
        "cancel_requested"
    } else {
        "already_finished"
    };
    json!({
        "job": job, "outcome": outcome, "request_accepted": accepted,
        "terminal": status.is_some_and(|status| matches!(status.state, job::State::Finished { .. })),
        "status": status, "cancel_error": cancel_error, "status_error": status_error,
    })
}

fn cancel_identity_script(record: &StudyRecord) -> Result<String> {
    job::check_id(&record.job)?;
    ensure!(
        record.root.starts_with('/')
            && record.root != "/"
            && !record.root.contains(char::is_whitespace),
        "invalid recorded remote root"
    );
    ensure!(
        record
            .remote_project
            .starts_with(&format!("{}/s/", record.root)),
        "Study project is outside its recorded remote root"
    );
    ensure!(
        matches!(record.scheduler.as_str(), "bare" | "slurm" | "pbs" | "lsf"),
        "invalid recorded scheduler"
    );
    let engine_marker = format!("# colm-engine-snapshot: {}", record.engine);
    let study_marker = format!(
        "# colm-study-root: {}",
        serde_json::to_string(&record.remote_project)?
    );
    let scheduler_id = if record.scheduler == "bare" {
        String::new()
    } else {
        let expected = record
            .scheduler_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .context("recorded scheduler job ID is missing")?;
        format!("[ \"$(cat sched_id)\" = {} ] || {{ echo 'scheduler job identity changed' >&2; exit 1; }}\n", quote(expected))
    };
    Ok(format!(
        "set -e\ncd {dir}\ngrep -Fqx -- {engine} script.sh || {{ echo 'engine snapshot identity changed' >&2; exit 1; }}\ngrep -Fqx -- {study} script.sh || {{ echo 'Study project identity changed' >&2; exit 1; }}\n[ \"$(cat scheduler 2>/dev/null || echo bare)\" = {scheduler} ] || {{ echo 'scheduler identity changed' >&2; exit 1; }}\n{scheduler_id}",
        dir = quote(&job::job_dir(&record.root, &record.job)), engine = quote(&engine_marker), study = quote(&study_marker), scheduler = quote(&record.scheduler),
    ))
}

/// 服务器上的成员与本机的一一对应吗（编号、是否基线、参数值）。
pub(crate) fn same_members(local: &[study::spec::MemberPlan], remote: &Value) -> Result<bool> {
    let remote: Vec<study::spec::MemberPlan> = serde_json::from_value(remote.clone())?;
    Ok(local.len() == remote.len()
        && local.iter().zip(&remote).all(|(a, b)| {
            a.id == b.id
                && a.baseline == b.baseline
                && a.parameters.len() == b.parameters.len()
                && a.parameters.iter().all(|(k, v)| {
                    b.parameters
                        .get(k)
                        .is_some_and(|w| (v - w).abs() <= 1e-12 * v.abs().max(1.0))
                })
        }))
}

/// 服务器上的研究状态改成本机这个研究的：编号换成本机的，任务的 `case_dir` 从服务器研究目录换成本机研究目录，
/// 进程身份去掉（那是服务器上的进程）。
pub(crate) fn transplant_state(
    mut state: Value,
    local_id: &str,
    remote_root: &str,
    local_root: &Path,
) -> Value {
    state["study_id"] = json!(local_id);
    if let Some(tasks) = state["tasks"].as_object_mut() {
        for task in tasks.values_mut() {
            if let Some(dir) = task["case_dir"].as_str() {
                if let Ok(rest) = Path::new(dir).strip_prefix(remote_root) {
                    task["case_dir"] = json!(local_root.join(rest));
                }
            }
            if let Some(task) = task.as_object_mut() {
                task.remove("process");
            }
        }
    }
    state
}

/// `remote-study-fetch`：作业跑完后，核对成员一致，把状态与 `results/` 移植进本机研究。
// Reject links and special files before importing server-controlled trees or
// moving existing local data into the transaction backup.
fn regular_tree_files(
    root: &Path,
    read_contents: bool,
) -> Result<std::collections::BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(
        root: &Path,
        dir: &Path,
        read_contents: bool,
        files: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<()> {
        ensure!(
            std::fs::symlink_metadata(dir)?.file_type().is_dir(),
            "not a regular directory: {}",
            dir.display()
        );
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), read_contents, files)?;
            } else {
                ensure!(
                    kind.is_file(),
                    "non-regular fetched file: {}",
                    entry.path().display()
                );
                files.insert(
                    entry.path().strip_prefix(root)?.to_owned(),
                    if read_contents {
                        std::fs::read(entry.path())?
                    } else {
                        Vec::new()
                    },
                );
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    visit(root, root, read_contents, &mut files)?;
    Ok(files)
}

fn validate_fetched_samples(
    manifest: &study::spec::Manifest,
    state: &study::state::StudyState,
    staged: &Path,
) -> Result<()> {
    use study::spec::StudyMethod;
    let local = regular_tree_files(&Path::new(&manifest.root).join("samples"), true)?;
    let remote = regular_tree_files(&staged.join("samples"), true)?;
    // Immutable local generations must never be replaced, even on a repeated fetch.
    for (name, bytes) in &local {
        ensure!(
            remote.get(name) == Some(bytes),
            "local sample conflicts with fetched sample: {}",
            name.display()
        );
    }
    let de = manifest.spec.method == StudyMethod::DifferentialEvolution;
    let initial = if de { "g000000.csv" } else { "design.csv" };
    let bytes = remote
        .get(Path::new(initial))
        .context("fetched Study has no initial samples")?;
    ensure!(
        colm_kernel::sha256_hex(bytes) == manifest.provenance.samples_sha256,
        "fetched initial samples differ from frozen local samples"
    );
    let mut generations = std::collections::BTreeSet::new();
    for name in remote.keys() {
        let text = name.to_str().context("invalid sample filename")?;
        ensure!(
            name.components().count() == 1,
            "nested sample path is not supported"
        );
        if text == initial {
            continue;
        }
        let csv = text.strip_suffix(".sha256").unwrap_or(text);
        let digits = csv
            .strip_prefix('g')
            .and_then(|s| s.strip_suffix(".csv"))
            .context("unexpected sample file")?;
        ensure!(
            de && digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()),
            "unexpected generation filename"
        );
        let generation: usize = digits.parse()?;
        ensure!(
            generation > 0 && generation <= manifest.spec.budget.generations.unwrap_or(0),
            "sample generation exceeds Study budget"
        );
        ensure!(
            remote.contains_key(Path::new(csv))
                && remote.contains_key(Path::new(&format!("{csv}.sha256"))),
            "incomplete immutable generation"
        );
        let path = staged.join("samples").join(csv);
        study::generation::verify_generation_hash(&path)?;
        if generations.insert(generation) {
            let rows = study::generation::read_sample_file(&path)?;
            ensure!(
                rows.len() == manifest.spec.budget.population.unwrap_or(0),
                "incomplete generation population"
            );
            ensure!(
                rows.iter().all(|row| row.generation == generation),
                "sample row generation does not match its filename"
            );
        }
    }
    ensure!(
        generations.iter().copied().eq(1..=generations.len()),
        "non-contiguous fetched generations"
    );
    let mut imported = manifest.clone();
    imported.root = staged.to_string_lossy().into_owned();
    let members = study::generation::load_members(&imported)?;
    let names = study::sample::sorted_parameter_names(&manifest.spec)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut by_id = std::collections::BTreeMap::new();
    for member in &members {
        ensure!(
            member.id == format!("m{:06}", member.candidate_index),
            "sample member identity mismatch"
        );
        if member.generation == 0 {
            ensure!(
                manifest
                    .members
                    .iter()
                    .any(|m| serde_json::to_value(m).ok() == serde_json::to_value(member).ok()),
                "initial member differs from frozen Study"
            );
        } else {
            ensure!(
                !member.baseline && generations.contains(&member.generation),
                "invalid generated member"
            );
            ensure!(
                member
                    .parameters
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
                    == names,
                "generation parameter columns do not match Study"
            );
            study::sample::validate_vector(&manifest.spec, &member.parameters)?;
            ensure!(
                member.parameters.values().all(|v| v.is_finite()),
                "non-finite generation parameter"
            );
            for parameter in &manifest.spec.parameters {
                let value = member.parameters[&parameter.member_key()];
                ensure!(
                    value >= parameter.sample_min && value <= parameter.sample_max,
                    "generation parameter is outside frozen sampling bounds"
                );
            }
            if let Some(hybrid) = &manifest.spec.hybrid {
                for (name, value) in &member.parameters {
                    if !manifest
                        .spec
                        .parameters
                        .iter()
                        .any(|p| p.member_key() == *name)
                    {
                        ensure!(
                            value.abs() <= hybrid.weight_range,
                            "hybrid weight exceeds frozen bounds"
                        );
                    }
                }
            }
        }
        by_id.insert(member.id.as_str(), member);
    }
    ensure!(
        members.len()
            == manifest.members.len()
                + generations.len() * manifest.spec.budget.population.unwrap_or(0),
        "incomplete generation population"
    );
    ensure!(
        state.schema_version == 1 && state.study_id == manifest.id,
        "fetched state identity mismatch"
    );
    ensure!(
        state.tasks.len() == members.len() * manifest.spec.base_cases.len(),
        "state tasks do not cover all sampled members"
    );
    for (id, task) in &state.tasks {
        ensure!(
            by_id.contains_key(task.member.as_str())
                && manifest.spec.base_cases.contains(&task.site),
            "state references unknown sampled member/site"
        );
        ensure!(
            *id == study::state::task_id(&task.member, &task.site),
            "state task identity mismatch"
        );
        ensure!(
            Path::new(&task.case_dir)
                == Path::new(&manifest.root)
                    .join("members")
                    .join(&task.member)
                    .join(&task.site),
            "state task path does not match local member"
        );
    }
    for (id, candidate) in &state.candidates {
        ensure!(
            by_id
                .get(id.as_str())
                .is_some_and(|m| m.generation == candidate.generation),
            "candidate has no matching sample generation"
        );
    }
    for id in state.population.iter().chain(state.best_member.iter()) {
        ensure!(
            by_id.contains_key(id.as_str()),
            "state refers to an unknown member"
        );
    }
    Ok(())
}

fn ensure_fetch_boundary(study_dir: &Path) -> Result<()> {
    for path in [study_dir.to_path_buf(), study_dir.join("checkpoints")] {
        ensure!(
            std::fs::symlink_metadata(&path)?.file_type().is_dir(),
            "fetch requires a real directory inside the Study: {}",
            path.display()
        );
    }
    Ok(())
}

fn install_fetched_study(study_dir: &Path, staged: &Path) -> Result<()> {
    ensure_fetch_boundary(study_dir)?;
    let mut entries = vec!["results", "samples", "checkpoints/state", RECORD];
    // 证据文件是核验通过后新写的审计材料：旧暂存目录没有它时安装行为与以前完全一样。
    if staged.join(EVIDENCE).is_file() {
        entries.push(EVIDENCE);
    }
    let mut changed = Vec::new();
    let result = (|| {
        for (index, entry) in entries.iter().enumerate() {
            let destination = study_dir.join(entry);
            let backup = staged.join(format!("previous-{index}"));
            let had_previous = destination.exists();
            if had_previous {
                std::fs::rename(&destination, &backup)?;
            }
            changed.push((index, had_previous, false));
            std::fs::rename(staged.join(entry), &destination)?;
            changed.last_mut().unwrap().2 = true;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for (index, had_previous, installed) in changed.into_iter().rev() {
            let destination = study_dir.join(entries[index]);
            if installed {
                std::fs::rename(&destination, staged.join(entries[index])).with_context(|| {
                    format!(
                        "fetch rollback failed; preserved backup in {}",
                        staged.display()
                    )
                })?;
            }
            if had_previous {
                std::fs::rename(staged.join(format!("previous-{index}")), &destination)
                    .with_context(|| {
                        format!(
                            "fetch rollback failed; preserved backup in {}",
                            staged.display()
                        )
                    })?;
            }
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn cmd_fetch(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    // 同 `cmd_cancel`：只有提交意图、作业其实跑完了的，先补记再取回（G-05）。
    if let PendingOutcome::NeverSubmitted(_) = adopt_pending(&study_dir)? {
        bail!("this Study's submission never reached the server; there is nothing to fetch, and it can be submitted again");
    }
    ensure_fetch_boundary(&study_dir)?;
    let _fetch_lock = study::runner::StudyRunLock::acquire(&study_dir)?;
    let manifest = study::engine::status(&study_dir)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let mut record = read_record(&study_dir)?;
    let ssh = Ssh::new(&record.host)?;
    ssh.run_ok(&cancel_identity_script(&record)?)?;
    let status = job::status(&ssh, &record.root, &record.job, 20)?;
    let exit_code = match status.state {
        job::State::Finished { exit_code } => exit_code,
        _ => bail!("the server job has not finished yet; fetch after it ends"),
    };
    let remote_root = remote_study_root(&ssh, &record)?
        .context("the server job did not create the Study; see its log")?;
    let remote = remote_study_status(&ssh, &record, &remote_root)?;
    let local_model = serde_json::to_value(
        crate::study_version::pinned(&study_dir)?.and_then(|pin| pin.model_revision),
    )?;
    ensure_model_identity(
        &local_model,
        &record.provenance["model_revision"],
        &remote["model_version"]["model_revision"],
    )?;
    ensure!(
        same_members(&manifest.members, &remote["manifest"]["members"])?,
        "the Study built on the server has different members from this one (spec or base cases changed); nothing was fetched"
    );
    let state = remote
        .get("state")
        .filter(|s| !s.is_null())
        .cloned()
        .context("the server Study has no state yet")?;
    let state = transplant_state(state, &manifest.id, &remote_root, &study_dir);
    let typed: study::state::StudyState =
        serde_json::from_value(state).context("the server Study state is not readable here")?;
    // Stage all data and the next checkpoint before replacing anything local.
    let tmp = study_dir.join(format!(
        ".fetch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir(&tmp)?;
    ssh.download(
        &remote_root,
        &["results".to_owned(), "samples".to_owned()],
        &tmp,
    )?;
    validate_fetched_samples(&manifest, &typed, &tmp)?;
    regular_tree_files(&tmp.join("results"), false)?;
    if study_dir.join("results").exists() {
        regular_tree_files(&study_dir.join("results"), false)?;
    }
    // Verification runs on the recorded server against raw outputs, never on downloaded metrics alone.
    // 按研究类型分流：率定让服务器 CLI 现场比较最优成员与基线；不确定性研究按设计没有最优成员，
    // 那条命令会整体拒绝，改按服务器 `study-status` 报告与本机下载树核对集合证据。
    let verification = if exit_code == 0 {
        match manifest.spec.kind {
            study::spec::StudyKind::Tuning => {
                server_comparison_evidence(&ssh, &record, &remote_root).and_then(|evidence| {
                    verified_evidence(&manifest, &record, &remote_root, &typed, &evidence)?;
                    verify_result_tree(&tmp.join("results"), &evidence["results_sha256"])?;
                    Ok(evidence)
                })
            }
            study::spec::StudyKind::Uncertainty => {
                uncertainty_fetch_evidence(&manifest, &remote_root, &remote, &tmp, &typed)
            }
        }
    } else {
        Err(anyhow::anyhow!(
            "server job failed with exit code {exit_code}; only diagnostic results were fetched"
        ))
    };
    if exit_code == 0 && !local_model.is_null() {
        verification.as_ref().map_err(|error| {
            anyhow::anyhow!(
                "remote scientific outputs were not verified; nothing was installed: {error:#}"
            )
        })?;
    }
    let scope_verified = verification.is_ok();
    // 不确定性证据没有"强迫内容已核对"的概念（原始输出留在服务器上）：与率定的 scoped 证据一样，
    // 只声称非强迫输入/观测与结果身份这一层被核对过，不冒充全内容核对。
    let scientific_verified = verification
        .as_ref()
        .is_ok_and(|evidence| evidence["forcing_contents_verified"] == true);
    let verification_error = verification
        .as_ref()
        .err()
        .map(|error| format!("{error:#}"));
    let verification_kind = match manifest.spec.kind {
        study::spec::StudyKind::Tuning => "tuning",
        study::spec::StudyKind::Uncertainty => "uncertainty",
    };
    for (name, bytes) in regular_tree_files(&study_dir.join("checkpoints/state"), true)? {
        let path = tmp.join("checkpoints/state").join(name);
        std::fs::create_dir_all(path.parent().context("checkpoint has no parent")?)?;
        std::fs::write(path, bytes)?;
    }
    study::checkpoint::write_next(&tmp.join("checkpoints/state"), &typed)?;
    record.fetched_at = Some(now());
    record.provenance["ok"] = json!(exit_code == 0 && scope_verified);
    record.provenance["exit_code"] = json!(exit_code);
    record.provenance["verification_kind"] = json!(verification_kind);
    record.provenance["scientific_verified"] = json!(scientific_verified);
    record.provenance["nonforcing_inputs_verified"] = json!(scope_verified);
    record.provenance["forcing_contents_verified"] = json!(scientific_verified);
    record.provenance["input_scope"] = json!("sha256-nonforcing-v1");
    record.provenance["verification_error"] = json!(verification_error);
    record.provenance["finished"] = json!(now());
    // 核验通过时把证据本身也记下来（每个成员的结果树哈希等），取回后的比较/审计还能拿它对照。
    // 核验失败时没有可记录的证据，`verification_error` 已经把原因写进记录。
    if let Ok(evidence) = &verification {
        std::fs::write(tmp.join(EVIDENCE), serde_json::to_vec_pretty(evidence)?)?;
        record.provenance["evidence"] = json!(EVIDENCE);
    }
    std::fs::write(tmp.join(RECORD), serde_json::to_string_pretty(&record)?)?;
    install_fetched_study(&study_dir, &tmp)?;
    let _ = std::fs::remove_dir_all(&tmp);
    let outcome = json!({"job":record.job,"outcome":"observed","terminal":true,"status":status});
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &outcome)?;
    let mut output = json!({
        "fetched": true,
        "exit_code": exit_code,
        "verification_kind": verification_kind,
        "scientific_verified": scientific_verified,
        "nonforcing_inputs_verified": scope_verified,
        "forcing_contents_verified": scientific_verified,
        "input_scope": "sha256-nonforcing-v1",
        "verification_error": verification_error,
        "status": typed.status,
        "remote_study": remote_root,
    });
    // 只有率定有最优成员；不确定性结果不能带一个会被读成"最优"的字段。
    if verification_kind == "tuning" {
        output["best_member"] = json!(typed.best_member);
    }
    println!("{}", output);
    Ok(())
}

#[cfg(test)]
#[path = "remote_study_tests.rs"]
mod remote_study_tests;

fn ensure_model_identity(local: &Value, submitted: &Value, remote: &Value) -> Result<()> {
    ensure!(
        local == submitted && local == remote,
        "remote Study model revision differs from its frozen submission; nothing was fetched"
    );
    Ok(())
}

#[cfg(test)]
mod model_revision_tests {
    use super::*;

    #[test]
    fn cancellation_acceptance_and_observed_termination_are_distinct() {
        let running = job::parse_status("state=running\n---log---\n");
        let accepted = cancellation_result("study-one", true, Some(&running), None, None);
        assert_eq!(accepted["request_accepted"], true);
        assert_eq!(accepted["terminal"], false);
        assert_eq!(accepted["outcome"], "cancel_requested");
        let uncertain = cancellation_result(
            "study-one",
            false,
            Some(&running),
            Some("SSH disconnected".into()),
            None,
        );
        assert_eq!(uncertain["terminal"], false);
        assert_eq!(uncertain["outcome"], "uncertain");
        let finished = job::parse_status("state=finished 143\n---log---\n");
        assert_eq!(
            cancellation_result("study-one", false, Some(&finished), None, None)["terminal"],
            true
        );
    }

    #[test]
    fn cancellation_script_binds_the_study_engine_and_scheduler_identity() {
        let mut record: StudyRecord = serde_json::from_value(json!({
            "host":"server", "root":"/data/colm", "job":"study-1",
            "remote_project":"/data/colm/s/abc", "engine":"0123456789abcdef",
            "submitted_at":1, "scheduler":"slurm", "scheduler_id":"4242", "provenance":{}
        }))
        .unwrap();
        let script = cancel_identity_script(&record).unwrap();
        assert!(script.contains("# colm-engine-snapshot: 0123456789abcdef"));
        assert!(script.contains("# colm-study-root: \"/data/colm/s/abc\""));
        assert!(script.contains("$(cat sched_id)\" = '4242'"));
        record.scheduler_id = None;
        assert!(cancel_identity_script(&record).is_err());
        record.job = "../other".into();
        assert!(cancel_identity_script(&record).is_err());
    }

    #[test]
    fn fetch_rejects_changed_revision_even_when_members_match() {
        let selected = json!({"version":{"id":"snow","revision":1},"fingerprint":"old"});
        let changed = json!({"version":{"id":"snow","revision":1},"fingerprint":"new"});
        ensure_model_identity(&selected, &selected, &selected).unwrap();
        assert!(ensure_model_identity(&selected, &selected, &changed).is_err());
        assert!(ensure_model_identity(&selected, &Value::Null, &selected).is_err());
        ensure_model_identity(&Value::Null, &Value::Null, &Value::Null).unwrap();
    }
}
