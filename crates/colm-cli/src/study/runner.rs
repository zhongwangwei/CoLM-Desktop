//! Authoritative local Study scheduler and scientific result writer.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use colm_namelist::Value;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::science::{ObjectiveMetric, ObjectiveTerm};
use super::spec::{
    Manifest, MemberPlan, ScaleSpec, StudyKind, StudyMethod, StudySpec, TargetPeriod, TargetSource,
    TargetSpec,
};
use super::state::{CandidateState, ProcessIdentity, StudyState, StudyStatus, TaskStatus};
use crate::{Layout, MetricsRequest, RunNotice, VarMetrics};

pub struct RunOptions<'a> {
    pub kernel_dir: &'a Path,
    pub jobs: usize,
    pub stream: bool,
    pub retry_failed: bool,
    pub preprocessors: crate::PreprocessorMode,
    pub engine: crate::ModelEngine,
}

pub(crate) struct StudyRunLock {
    path: PathBuf,
    identity: ProcessIdentity,
    stop: mpsc::Sender<()>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
}

const RUN_LOCK_HEARTBEAT_SECONDS: u64 = 10;
const RUN_LOCK_STALE_SECONDS: i64 = 60;
const TASK_LOG_EMIT_INTERVAL: Duration = Duration::from_millis(100);
const NON_SCALAR_UQ_OUTPUTS: &[&str] = &["f_t_soisno", "f_wliq_soisno", "f_wice_soisno"];

fn should_emit_task_log(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_none_or(|previous| now.saturating_duration_since(previous) >= TASK_LOG_EMIT_INTERVAL)
    {
        *last = Some(now);
        true
    } else {
        false
    }
}

impl StudyRunLock {
    pub(crate) fn acquire(study_dir: &Path) -> Result<Self> {
        let path = study_dir.join("run.lock");
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                bail!("Study is already running or has a stale run.lock; confirm the old process exited, then retry with --include-review")
            }
            Err(error) => {
                return Err(error).with_context(|| format!("cannot create {}", path.display()))
            }
        };
        let identity = supervisor_identity();
        if let Err(error) = write_run_lock_identity(&path, &mut file, &identity) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let heartbeat_path = path.clone();
        let owner = identity.clone();
        let (stop, stopped) = mpsc::channel();
        // ponytail: MSRV 1.85 has no portable std file lock; this short lease
        // prevents confirmed crash recovery from deleting a live scheduler lock.
        let heartbeat = std::thread::spawn(move || {
            let mut identity = identity;
            while matches!(
                stopped.recv_timeout(std::time::Duration::from_secs(RUN_LOCK_HEARTBEAT_SECONDS)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                // A lock that now names another scheduler (ours was judged stale and
                // cleared) is not ours to refresh.
                if !owns_run_lock(&heartbeat_path, &identity) {
                    break;
                }
                identity.heartbeat_unix = unix_now();
                if replace_run_lock(&heartbeat_path, &identity).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            path,
            identity: owner,
            stop,
            heartbeat: Some(heartbeat),
        })
    }
}

impl Drop for StudyRunLock {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(heartbeat) = self.heartbeat.take() {
            let _ = heartbeat.join();
        }
        // Remove only our own lock, never one a replacement scheduler created after ours.
        if owns_run_lock(&self.path, &self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn owns_run_lock(path: &Path, identity: &ProcessIdentity) -> bool {
    run_lock_owner(path).ok().flatten().is_some_and(|owner| {
        owner.pid == identity.pid
            && owner.started_unix == identity.started_unix
            && owner.argv_sha256 == identity.argv_sha256
    })
}

/// Rewrite the lease through a sibling file and a rename, so readers never see half a record.
fn replace_run_lock(path: &Path, identity: &ProcessIdentity) -> Result<()> {
    let staged = path.with_extension(format!("lock.{}", std::process::id()));
    let mut file = fs::File::create(&staged)?;
    write_run_lock_identity(&staged, &mut file, identity)?;
    drop(file);
    fs::rename(&staged, path).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })?;
    Ok(())
}

fn write_run_lock_identity(
    path: &Path,
    file: &mut fs::File,
    identity: &ProcessIdentity,
) -> Result<()> {
    file.write_all(&serde_json::to_vec(identity)?)?;
    file.sync_all()
        .with_context(|| format!("cannot sync {}", path.display()))
}

fn clear_stale_run_lock(study_dir: &Path) -> Result<()> {
    let path = study_dir.join("run.lock");
    let Some(owner) = run_lock_owner(&path)? else {
        return Ok(());
    };
    if unix_now().saturating_sub(owner.heartbeat_unix) <= RUN_LOCK_STALE_SECONDS
        || scheduler_process_alive(owner.pid)?
    {
        bail!("Study scheduler PID {} is still running", owner.pid);
    }
    fs::remove_file(&path).with_context(|| format!("cannot remove stale {}", path.display()))
}

fn run_lock_owner(path: &Path) -> Result<Option<ProcessIdentity>> {
    let mut last = Vec::new();
    for attempt in 0..2 {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if let Ok(owner) = serde_json::from_slice(&bytes) {
            return Ok(Some(owner));
        }
        last = bytes;
        if attempt == 0 {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    serde_json::from_slice(&last)
        .with_context(|| format!("cannot verify the owner of {}; remove it manually only after confirming the scheduler exited", path.display()))
        .map(Some)
}

#[derive(Clone, Debug, Serialize)]
pub struct ResultFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplyPreviewRow {
    pub site: String,
    pub file: String,
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TargetResult {
    key: String,
    site: String,
    variable: String,
    period: String,
    metric: ObjectiveMetric,
    weight: f64,
    min_pairs: usize,
    pairs: usize,
    value: f64,
    observation_sd: Option<f64>,
    loss: f64,
    model_mean: f64,
    observation_mean: f64,
    #[serde(default)]
    support_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OutputSummary {
    variable: String,
    units: Option<String>,
    count: usize,
    mean: f64,
    min: f64,
    max: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TaskResult {
    member: String,
    site: String,
    #[serde(default)]
    outputs: Vec<OutputSummary>,
    #[serde(default)]
    calibration: Vec<TargetResult>,
    #[serde(default)]
    validation: Vec<TargetResult>,
    #[serde(default)]
    validation_errors: Vec<TargetError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TargetError {
    key: String,
    site: String,
    variable: String,
    period: String,
    reason: String,
}

#[derive(Clone, Debug, Serialize)]
struct ObjectiveMetricRow {
    member: String,
    site: String,
    period: String,
    target: String,
    variable: String,
    metric: ObjectiveMetric,
    weight: f64,
    min_pairs: usize,
    pairs: usize,
    value: f64,
    observation_sd: Option<f64>,
    loss: f64,
    model_mean: f64,
    observation_mean: f64,
    #[serde(default)]
    support_hash: String,
}

enum WorkerEvent {
    Started {
        member: String,
        site: String,
    },
    Notice {
        member: String,
        site: String,
        kind: String,
        stage: String,
        line: Option<String>,
        ok: Option<bool>,
    },
    Finished {
        member: String,
        site: String,
        case_dir: PathBuf,
        error: Option<String>,
    },
    /// A stage child (Fortran kernel or Rust engine) started for this task.
    Process {
        member: String,
        site: String,
        pid: u32,
        program: PathBuf,
    },
}

pub fn preflight_create(case_root: &Path, spec_file: &Path) -> Result<()> {
    let spec = super::spec::read_spec(spec_file)?;
    super::spec::validate_spec(&spec)?;
    let requested_kernel = spec
        .kernel_dir
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .context(
            "Study creation requires kernel_dir so parameter activity and provenance can be frozen",
        )?;
    let case_root = colm_kernel::manifest::absolute(case_root)?;
    let sites = resolved_base_cases(&case_root, &spec)?;
    let base_case_paths = sites
        .iter()
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    let site_names = sites
        .iter()
        .map(|(site, _)| site.clone())
        .collect::<Vec<_>>();
    super::spec::validate_target_site_coverage(&spec, &site_names)?;
    let path = PathBuf::from(requested_kernel);
    let resolved = if path.is_absolute() {
        path
    } else {
        case_root.join(path)
    };
    let kernel_macros = colm_kernel::Kernel::open_manifest(&resolved)?
        .manifest
        .macros;
    super::engine::ensure_supported_study_inputs(&base_case_paths, &kernel_macros, &spec)?;
    super::engine::baseline(&base_case_paths, &spec, &kernel_macros)?;
    if spec.kind == StudyKind::Uncertainty {
        for output in &spec.outputs {
            if NON_SCALAR_UQ_OUTPUTS.contains(&output.as_str()) {
                bail!(
                    "UQ output {output} is a vertical profile; Study outputs must be scalar time series"
                );
            }
        }
        for (site, case) in &sites {
            let planned = crate::planned_history_variables(&case.join("case.nml"), &resolved)?;
            for output in &spec.outputs {
                if !planned.contains(output) {
                    bail!(
                        "baseline output {output} is not enabled by the case and frozen kernel for {site}"
                    );
                }
            }
        }
        // 带目标的不确定性分析：目标照率定的规矩预检（变量可比、观测时段里配对够数）。
        if spec.targets.is_empty() {
            return Ok(());
        }
    }
    for (site, case) in &sites {
        let planned = crate::planned_history_variables(&case.join("case.nml"), &resolved)?;
        for target in spec
            .targets
            .iter()
            .filter(|target| target.site.as_deref().is_none_or(|wanted| wanted == site))
        {
            let observation = match &target.source {
                Some(TargetSource::Grid { .. } | TargetSource::Gauge { .. }) => {
                    super::targets::preflight_observation(&case_root, target, &planned)?;
                    continue;
                }
                Some(TargetSource::Station { path, .. }) => resolve_observation(&case_root, path)?,
                None => observation_for(&spec, &case_root, site)?,
            };
            let catalog =
                crate::evaluation_plan_rows(&case.join("case.nml"), &observation, &resolved)?;
            let row = catalog
                .iter()
                .find(|row| row.name.eq_ignore_ascii_case(&target.variable))
                .with_context(|| {
                    format!(
                        "required target {} ({}) is not supported for {site}",
                        target.key, target.variable
                    )
                })?;
            if !row.available {
                let missing = row
                    .missing_model
                    .iter()
                    .map(|name| format!("model {name}"))
                    .chain(
                        row.missing_observation
                            .iter()
                            .map(|name| format!("observation {name}")),
                    )
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "required target {} ({}) is unavailable for {site}: missing {missing}",
                    target.key,
                    target.variable
                );
            }
            // 订正目标要求观测里真有订正版：缺了运行时才发现，基准已经白跑一轮。
            if let Some(wanted) = target
                .corrected
                .then(|| colm_hist::obs::corrected(&row.name))
                .flatten()
            {
                let file = netcdf::open(&observation)
                    .with_context(|| format!("cannot open {}", observation.display()))?;
                if file.variable(wanted).is_none() {
                    bail!(
                        "target {} asks for energy-balance-corrected {wanted}, but {} has none",
                        target.key,
                        observation.display()
                    );
                }
            }
            preflight_observation_window(&observation, site, target, false)?;
            if target.validation_from.is_some() {
                preflight_observation_window(&observation, site, target, true)?;
            }
        }
    }
    Ok(())
}

fn resolve_observation(case_root: &Path, raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        case_root.join(path)
    };
    if !path.is_file() {
        bail!("observation file does not exist: {}", path.display());
    }
    Ok(path)
}

pub fn result_files(study_dir: &Path) -> Result<Vec<ResultFile>> {
    let root = study_dir.join("results");
    let mut files = Vec::new();
    collect_result_files(&root, &root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn read_result(study_dir: &Path, relative: &Path) -> Result<String> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("Study result path must stay under results/");
    }
    let root = study_dir
        .join("results")
        .canonicalize()
        .with_context(|| format!("cannot resolve Study results under {}", study_dir.display()))?;
    let requested = root.join(relative);
    let path = requested
        .canonicalize()
        .with_context(|| format!("Study result does not exist: {}", relative.display()))?;
    if !path.starts_with(&root) || !path.is_file() {
        bail!("Study result does not exist: {}", relative.display());
    }
    fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))
}

/// 证据里每个列表最多保留多少条；远程研究状态会把这个 JSON 原样经 SSH 搬回本机。
const EVIDENCE_LIST_LIMIT: usize = 8;
/// 证据里每条原因 / 警告最多保留多少字符。
const EVIDENCE_TEXT_LIMIT: usize = 200;
/// `study-status.evidence` 的序列化上限：够放下身份、两期分数、选择规则与缺失原因，
/// 又不至于让远程状态输出膨胀（不能把整个 state / events / results 塞进来）。
const EVIDENCE_BUDGET_BYTES: usize = 8 * 1024;
fn compact_evidence_text(text: &str, limit: usize) -> String {
    let mut out = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        out.push('…');
    }
    out
}

/// `study-status` 的结构化证据（P1-A）：只读 checkpoint、样本文件与已记录的任务结果。
///
/// 不重跑模拟、不连服务器、不重算科学评分 —— 聚合分数复用 runner 写入候选时同一个 [`fixed_score`]，
/// 逐目标 loss 直接来自 `results/tasks/<member>/<site>.json`。`state` 由调用方传入：`status_state`
/// 可能写 checkpoint，`study-status` 只应读一次，这条路径必须无副作用。
pub(crate) fn study_evidence(
    study_dir: &Path,
    manifest: &Manifest,
    state: Option<&StudyState>,
) -> Result<serde_json::Value> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let (members, members_error) = match super::generation::load_members(manifest) {
        Ok(members) => (Some(members), None),
        Err(error) => (
            None,
            Some(compact_evidence_text(
                &format!("{error:#}"),
                EVIDENCE_TEXT_LIMIT,
            )),
        ),
    };
    let best_id = state.and_then(|state| state.best_member.clone());
    // 基线成员用样本里的 `baseline` 标记定位，不按编号猜：`m000000` 只是 `sample::design`
    // 的默认编号，样本文件才是权威来源。找不到标记时明确 unavailable，不退回任何编号。
    let baseline_id = members
        .as_deref()
        .and_then(|members| members.iter().find(|member| member.baseline))
        .map(|member| member.id.clone());
    let baseline_unavailable = if members.is_none() {
        Some(
            "the recorded sample files could not be loaded, so the baseline member cannot be located"
                .to_string(),
        )
    } else if baseline_id.is_none() {
        Some("no baseline member is recorded in the Study samples".to_string())
    } else {
        None
    };
    // 研究还在跑时，某个成员缺结果可能只是还没轮到；已完成的研究则说明结果缺失/损坏。
    let pending = state.is_none_or(|state| {
        !matches!(
            state.status,
            StudyStatus::Completed
                | StudyStatus::CompletedWithFailures
                | StudyStatus::Cancelled
                | StudyStatus::NeedsReview
        )
    });
    let (baseline_results, baseline_missing) = match baseline_id.as_deref() {
        Some(member) => read_recorded_results(manifest, member),
        None => (Vec::new(), Vec::new()),
    };
    let comparison_baseline = match baseline_id.as_deref() {
        Some(member) => evidence_comparison_side(
            manifest,
            member,
            &baseline_results,
            &baseline_missing,
            pending,
            state,
            true,
        ),
        None => {
            let reason = baseline_unavailable.clone().unwrap_or_else(|| {
                "no baseline member is recorded in the Study samples".to_string()
            });
            json!({
                "member": null,
                "status": "unavailable",
                "calibration": {"status": "unavailable", "score": null, "reason": reason},
                "validation": {"status": "unavailable", "score": null, "reason": reason},
                "warnings": [],
                "reason": reason,
            })
        }
    };
    let comparison_best = match best_id.as_deref() {
        Some(member) => {
            let (results, missing) = read_recorded_results(manifest, member);
            evidence_comparison_side(
                manifest,
                member,
                &results,
                &missing,
                pending,
                state,
                baseline_id.as_deref() == Some(member),
            )
        }
        None => json!({
            "member": null, "status": "unavailable",
            "calibration": null, "validation": null, "warnings": [],
            "reason": "no best member is recorded",
        }),
    };
    let validation = evidence_validation(manifest, &comparison_baseline, &comparison_best, state);
    let parameters = evidence_parameters(
        manifest,
        members.as_deref(),
        members_error.as_deref(),
        best_id.as_deref(),
    );
    let model_identity = evidence_model_identity(&study_dir);
    let selection = evidence_selection(manifest, state, baseline_id.as_deref());
    let missing = {
        let mut missing = Vec::new();
        let mut push = |text: String| {
            if missing.len() < EVIDENCE_LIST_LIMIT && !missing.contains(&text) {
                missing.push(text);
            }
        };
        if let Some(reason) = baseline_unavailable.as_ref() {
            push(reason.clone());
        }
        if selection["best_member"].is_null() {
            if let (Some(code), Some(detail)) = (
                selection["no_best_reason"]["code"].as_str(),
                selection["no_best_reason"]["detail"].as_str(),
            ) {
                push(format!("No best member ({code}): {detail}"));
            }
        }
        for side in [&comparison_baseline, &comparison_best] {
            for period in ["calibration", "validation"] {
                let period = &side[period];
                if period["status"] != "recorded" {
                    if let Some(reason) = period["reason"].as_str() {
                        push(compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT));
                    }
                }
            }
        }
        for reason in validation["reasons"].as_array().into_iter().flatten() {
            if let Some(reason) = reason.as_str() {
                push(compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT));
            }
        }
        if let Some(reason) = parameters["reason"].as_str() {
            push(compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT));
        }
        if let Some(reason) = model_identity["study_record"]["reason"].as_str() {
            push(compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT));
        }
        missing
    };
    let warnings = state
        .map(|state| {
            state
                .warnings
                .iter()
                .take(EVIDENCE_LIST_LIMIT)
                .map(|warning| compact_evidence_text(warning, EVIDENCE_TEXT_LIMIT))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let evidence = json!({
        "schema": 1,
        "study": {
            "id": manifest.id,
            "kind": manifest.spec.kind,
            "method": manifest.spec.method,
            "status": state.map(|state| state.status),
            "generation": state.map(|state| state.generation),
        },
        "execution": evidence_execution(state),
        "selection": selection,
        "objective": evidence_objective(&manifest.spec),
        "comparison": {
            "basis": "best is compared with the baseline only within the same period; calibration scores are never compared with validation scores",
            "baseline": comparison_baseline,
            "best": comparison_best,
            "relation": {
                "calibration": score_relation(&comparison_baseline, &comparison_best, "calibration"),
                "validation": score_relation(&comparison_baseline, &comparison_best, "validation"),
            },
        },
        "validation": validation,
        "parameters": parameters,
        "model_identity": model_identity,
        "verification": evidence_verification(manifest, state),
        "remote": evidence_remote(&study_dir),
        "source_paths": {
            "root": manifest.root,
            "manifest": "manifest.json",
            "samples": "samples",
            "state": "checkpoints/state",
            "results": "results/tasks",
        },
        "warnings": warnings,
        "warnings_count": state.map_or(0, |state| state.warnings.len()),
        "missing": missing,
    });
    Ok(bound_evidence(evidence))
}

fn execution_state(state: Option<&StudyState>) -> &'static str {
    match state.map(|state| state.status) {
        None | Some(StudyStatus::Ready) => "not_started",
        Some(StudyStatus::Running) => "running",
        Some(StudyStatus::Paused) => "paused",
        Some(StudyStatus::Completed) => "completed",
        Some(StudyStatus::CompletedWithFailures) => "completed_with_failures",
        Some(StudyStatus::Cancelled) => "cancelled",
        Some(StudyStatus::NeedsReview) => "needs_review",
    }
}

fn evidence_execution(state: Option<&StudyState>) -> serde_json::Value {
    let Some(state) = state else {
        return json!({
            "state": "not_started",
            "tasks": {"total": 0},
            "completed_candidates": 0,
            "partial_failures": false,
            "failed_tasks": [],
        });
    };
    let count = |wanted: &[TaskStatus]| {
        state
            .tasks
            .values()
            .filter(|task| wanted.contains(&task.status))
            .count()
    };
    let failed = count(&[TaskStatus::Failed, TaskStatus::Interrupted]);
    let needs_review = count(&[TaskStatus::NeedsReview]);
    let active = count(&[
        TaskStatus::Pending,
        TaskStatus::Materialized,
        TaskStatus::Queued,
        TaskStatus::Running,
        TaskStatus::Evaluating,
    ]);
    let failed_tasks = state
        .tasks
        .values()
        .filter(|task| {
            matches!(
                task.status,
                TaskStatus::Failed | TaskStatus::Interrupted | TaskStatus::NeedsReview
            )
        })
        .take(5)
        .map(|task| {
            json!({
                "member": task.member,
                "site": task.site,
                "status": task.status,
                "reason": task.reason.as_deref().map(|reason| compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT)),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "state": execution_state(Some(state)),
        "tasks": {
            "total": state.tasks.len(),
            "succeeded": count(&[TaskStatus::Succeeded]),
            "failed": failed,
            "needs_review": needs_review,
            "active": active,
            "cancelled": count(&[TaskStatus::Cancelled]),
        },
        "completed_candidates": state.completed_candidates,
        "partial_failures": failed > 0 || needs_review > 0,
        "failed_tasks": failed_tasks,
    })
}

fn evidence_selection(
    manifest: &Manifest,
    state: Option<&StudyState>,
    baseline_id: Option<&str>,
) -> serde_json::Value {
    let best_member = state.and_then(|state| state.best_member.clone());
    let best_objective = state.and_then(|state| state.best_objective);
    let feasible_non_baseline = state.map_or(0, |state| {
        state
            .candidates
            .iter()
            .filter(|(member, candidate)| {
                Some(member.as_str()) != baseline_id && candidate.feasible
            })
            .count()
    });
    let no_best_reason = if best_member.is_some() {
        serde_json::Value::Null
    } else {
        no_best_reason(manifest, state, feasible_non_baseline)
    };
    json!({
        "rule": "feasible tuning member (baseline excluded) with the lowest recorded calibration loss; the validation score never selects the best member",
        "best_member": best_member,
        "best_objective": best_objective,
        "provisional": matches!(
            state.map(|state| state.status),
            Some(StudyStatus::Running | StudyStatus::Paused)
        ),
        "candidates": {
            "recorded": state.map_or(0, |state| state.candidates.len()),
            "feasible_non_baseline": feasible_non_baseline,
        },
        "no_best_reason": no_best_reason,
    })
}

fn no_best_reason(
    manifest: &Manifest,
    state: Option<&StudyState>,
    feasible_non_baseline: usize,
) -> serde_json::Value {
    if manifest.spec.kind != StudyKind::Tuning {
        return json!({
            "code": "uncertainty",
            "detail": "an uncertainty analysis ranks parameter effects; it has no best member",
        });
    }
    let Some(state) = state else {
        return json!({
            "code": "not_started",
            "detail": "no checkpoint records a scored candidate",
        });
    };
    match state.status {
        StudyStatus::Ready => json!({
            "code": "not_started",
            "detail": "the Study has no scored candidate yet",
        }),
        StudyStatus::Running => json!({
            "code": "running",
            "detail": "the Study is still running; no best member has been recorded",
        }),
        StudyStatus::Paused => json!({
            "code": "paused",
            "detail": "the Study is paused; no best member has been recorded",
        }),
        StudyStatus::Cancelled => json!({
            "code": "cancelled",
            "detail": "the Study was cancelled before a best member was recorded",
        }),
        StudyStatus::NeedsReview => json!({
            "code": "needs_review",
            "detail": "at least one task needs review; no best member is recorded",
        }),
        StudyStatus::Completed | StudyStatus::CompletedWithFailures => {
            let failures = state
                .tasks
                .values()
                .filter(|task| {
                    matches!(
                        task.status,
                        TaskStatus::Failed | TaskStatus::Interrupted | TaskStatus::NeedsReview
                    )
                })
                .count();
            if state.candidates.is_empty() {
                json!({
                    "code": "no_scored_candidate",
                    "detail": "no candidate has a complete scored task set",
                })
            } else if feasible_non_baseline == 0 {
                json!({
                    "code": if failures > 0 { "no_feasible_member_after_failures" } else { "no_feasible_member" },
                    "detail": format!(
                        "{} candidate result(s) recorded, none feasible{}",
                        state.candidates.len(),
                        if failures > 0 { format!("; {failures} task(s) failed or need review") } else { String::new() },
                    ),
                })
            } else {
                json!({
                    "code": "not_recorded",
                    "detail": "feasible candidates exist but no best member was recorded",
                })
            }
        }
    }
}

fn target_source_name(target: &TargetSpec) -> &'static str {
    match &target.source {
        None => "site_observations",
        Some(TargetSource::Grid { .. }) => "grid_observations",
        Some(TargetSource::Station { .. }) => "station_observations",
        Some(TargetSource::Gauge { .. }) => "gauge_observations",
    }
}

fn target_aggregate(target: &TargetSpec) -> Option<TargetPeriod> {
    target.aggregate.or(match &target.source {
        Some(TargetSource::Grid { period, .. } | TargetSource::Gauge { period, .. }) => *period,
        Some(TargetSource::Station { .. }) | None => None,
    })
}

fn scale_name(scale: ScaleSpec) -> &'static str {
    match scale {
        ScaleSpec::Linear => "linear",
        ScaleSpec::Log => "log",
    }
}

fn evidence_objective(spec: &StudySpec) -> serde_json::Value {
    let targets = spec
        .targets
        .iter()
        .take(EVIDENCE_LIST_LIMIT)
        .map(|target| {
            json!({
                "key": target.key,
                "variable": target.variable,
                "metric": target.metric,
                "weight": target.weight,
                "calibration": [target.from, target.to],
                "validation": target.validation_from.zip(target.validation_to),
                "source": target_source_name(target),
                "aggregate": target_aggregate(target),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "direction": "minimize",
        "loss": "weighted mean of per-target losses; each loss is the metric mapped to lower-is-better (nrmse: rmse/observation_sd, mae: mae, abs_bias: |bias|, nse/kge: max(0, 1-value), r/r2: 1-value)",
        "target_count": spec.targets.len(),
        "targets": targets,
    })
}

fn read_recorded_results(manifest: &Manifest, member: &str) -> (Vec<TaskResult>, Vec<String>) {
    let mut results = Vec::new();
    let mut missing = Vec::new();
    for site in &manifest.spec.base_cases {
        match read_task_result(manifest, member, site) {
            Ok(result) => results.push(result),
            Err(error) => missing.push(compact_evidence_text(
                &format!("{member}/{site}: {error:#}"),
                EVIDENCE_TEXT_LIMIT,
            )),
        }
    }
    (results, missing)
}

fn evidence_comparison_side(
    manifest: &Manifest,
    member: &str,
    results: &[TaskResult],
    missing: &[String],
    pending: bool,
    state: Option<&StudyState>,
    is_baseline: bool,
) -> serde_json::Value {
    let candidate = state.and_then(|state| state.candidates.get(member));
    let calibration = comparison_period(
        manifest,
        member,
        results,
        missing,
        pending,
        state,
        "calibration",
        candidate,
    );
    let validation = comparison_period(
        manifest,
        member,
        results,
        missing,
        pending,
        state,
        "validation",
        candidate,
    );
    let warnings = state.map_or_else(Vec::new, |state| {
        state
            .warnings
            .iter()
            .filter(|warning| {
                warning.contains(&format!("{member} / "))
                    || (!is_baseline
                        && (warning.starts_with("overfitting:") || warning.starts_with("bound:")))
            })
            .take(5)
            .map(|warning| compact_evidence_text(warning, EVIDENCE_TEXT_LIMIT))
            .collect()
    });
    json!({
        "member": member,
        "calibration": calibration,
        "validation": validation,
        "warnings": warnings,
    })
}

/// 该成员相关的引擎警告（validation unavailable / validation target … unavailable / support mismatch）。
fn member_warnings(state: Option<&StudyState>, member: &str) -> Vec<String> {
    state.map_or_else(Vec::new, |state| {
        state
            .warnings
            .iter()
            .filter(|warning| warning.contains(&format!("{member} / ")))
            .take(5)
            .map(|warning| compact_evidence_text(warning, EVIDENCE_TEXT_LIMIT))
            .collect()
    })
}

/// 引擎记录里该成员的候选状态比结果文件更权威：`refresh_candidates` 只在结果可比且完整时写分数，
/// 并把 validation 置 None 的原因写进 warnings。证据必须与这个判定一致，不能自己对文件重打分。
fn comparison_period_from_candidate(
    member: &str,
    results: &[TaskResult],
    state: Option<&StudyState>,
    period: &str,
    candidate: &CandidateState,
) -> serde_json::Value {
    let rows = if period == "calibration" {
        results
            .iter()
            .flat_map(|result| result.calibration.iter().cloned())
            .collect::<Vec<_>>()
    } else {
        results
            .iter()
            .flat_map(|result| result.validation.iter().cloned())
            .collect::<Vec<_>>()
    };
    let targets = target_loss_rows(&rows);
    if period == "calibration" {
        return match candidate.calibration {
            Some(score) => json!({
                "status": "recorded",
                "score": score,
                "targets": targets,
            }),
            None => json!({
                "status": "unavailable",
                "score": null,
                "reason": candidate.reason.as_deref().map_or_else(
                    || "the Study recorded no calibration score for this member".to_string(),
                    |reason| compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT),
                ),
                "targets": targets,
            }),
        };
    }
    match candidate.validation {
        Some(score) => json!({
            "status": "recorded",
            "score": score,
            "targets": targets,
        }),
        None => {
            let reasons = member_warnings(state, member);
            let errors = results
                .iter()
                .flat_map(|result| result.validation_errors.iter())
                .take(EVIDENCE_LIST_LIMIT)
                .map(|error| {
                    json!({
                        "key": error.key,
                        "site": error.site,
                        "reason": compact_evidence_text(&error.reason, EVIDENCE_TEXT_LIMIT),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "status": if rows.is_empty() { "unavailable" } else { "partial" },
                "score": null,
                "reason": reasons.first().cloned().unwrap_or_else(|| {
                    "the Study recorded no validation score for this member (validation not comparable to the baseline or target errors)".to_string()
                }),
                "reasons": reasons,
                "errors": errors,
                "targets": targets,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn comparison_period(
    manifest: &Manifest,
    member: &str,
    results: &[TaskResult],
    missing: &[String],
    pending: bool,
    state: Option<&StudyState>,
    period: &str,
    candidate: Option<&CandidateState>,
) -> serde_json::Value {
    let site_count = manifest.spec.base_cases.len();
    if manifest.spec.targets.is_empty() {
        return json!({
            "status": "not_configured",
            "reason": "the Study scores no targets",
        });
    }
    if period == "validation"
        && !manifest
            .spec
            .targets
            .iter()
            .any(|target| target.validation_from.is_some())
    {
        return json!({
            "status": "not_configured",
            "reason": "validation windows are not configured in the Study spec",
        });
    }
    // 已有候选记录：以引擎记录为准（见 comparison_period_from_candidate）。
    if let Some(candidate) = candidate {
        return comparison_period_from_candidate(member, results, state, period, candidate);
    }
    let incomplete_reason = if missing.is_empty() {
        format!(
            "recorded task results are incomplete ({}/{site_count} sites); some results are missing",
            results.len()
        )
    } else {
        format!(
            "recorded task results are incomplete ({}/{site_count} sites): {}",
            results.len(),
            missing.first().map_or("unreadable result", String::as_str)
        )
    };
    if results.len() != site_count {
        return json!({
            "status": if pending { "pending" } else { "unavailable" },
            "reason": compact_evidence_text(&incomplete_reason, EVIDENCE_TEXT_LIMIT),
        });
    }
    if period == "calibration" {
        let rows = results
            .iter()
            .flat_map(|result| result.calibration.iter().cloned())
            .collect::<Vec<_>>();
        let lost_targets = target_loss_rows(&rows);
        return match fixed_score(&rows) {
            Some(score) => json!({
                "status": "recorded",
                "score": score,
                "targets": lost_targets,
            }),
            None => json!({
                "status": "unavailable",
                "reason": "recorded calibration rows could not be aggregated into a score",
                "targets": lost_targets,
            }),
        };
    }
    let errors = results
        .iter()
        .flat_map(|result| result.validation_errors.iter())
        .collect::<Vec<_>>();
    let rows = results
        .iter()
        .flat_map(|result| result.validation.iter().cloned())
        .collect::<Vec<_>>();
    let lost_targets = target_loss_rows(&rows);
    if errors.is_empty() {
        return match fixed_score(&rows) {
            Some(score) => json!({
                "status": "recorded",
                "score": score,
                "targets": lost_targets,
            }),
            None => json!({
                "status": "unavailable",
                "reason": "validation targets are configured but no complete validation rows were recorded",
                "targets": lost_targets,
            }),
        };
    }
    let error_rows = errors
        .iter()
        .take(EVIDENCE_LIST_LIMIT)
        .map(|error| {
            json!({
                "key": error.key,
                "site": error.site,
                "reason": compact_evidence_text(&error.reason, EVIDENCE_TEXT_LIMIT),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "status": if rows.is_empty() { "unavailable" } else { "partial" },
        "reason": "some validation targets could not be scored; the aggregate validation score is not comparable",
        "errors": error_rows,
        "targets": lost_targets,
    })
}

/// 逐目标 loss：同一目标在多个站点上的记录按同一套权重合成一个 loss（与 [`fixed_score`] 同义），
/// 不让同一目标重复出现多行。
fn target_loss_rows(rows: &[TargetResult]) -> Vec<serde_json::Value> {
    let mut by_key: BTreeMap<&str, Vec<TargetResult>> = BTreeMap::new();
    for row in rows {
        by_key
            .entry(row.key.as_str())
            .or_default()
            .push(row.clone());
    }
    by_key
        .into_iter()
        .filter_map(|(key, rows)| {
            fixed_score(&rows).map(|loss| {
                json!({
                    "key": key,
                    "loss": loss,
                })
            })
        })
        .take(EVIDENCE_LIST_LIMIT)
        .collect()
}

fn score_relation(
    baseline: &serde_json::Value,
    best: &serde_json::Value,
    period: &str,
) -> &'static str {
    let baseline_score = baseline[period]["score"].as_f64();
    let best_score = best[period]["score"].as_f64();
    match (baseline_score, best_score) {
        (Some(baseline), Some(best)) if best < baseline => "lower_loss",
        (Some(baseline), Some(best)) if best > baseline => "higher_loss",
        (Some(_), Some(_)) => "equal",
        _ => "unknown",
    }
}

fn evidence_validation(
    manifest: &Manifest,
    baseline: &serde_json::Value,
    best: &serde_json::Value,
    state: Option<&StudyState>,
) -> serde_json::Value {
    if !manifest
        .spec
        .targets
        .iter()
        .any(|target| target.validation_from.is_some())
    {
        return json!({
            "configured": false,
            "windows": [],
            "status": "not_configured",
            "reasons": ["The Study spec has no validation windows; only in-sample calibration fit is recorded."],
        });
    }
    let windows = manifest
        .spec
        .targets
        .iter()
        .filter(|target| target.validation_from.is_some())
        .take(EVIDENCE_LIST_LIMIT)
        .map(|target| {
            json!({
                "target": target.key,
                "from": target.validation_from,
                "to": target.validation_to,
            })
        })
        .collect::<Vec<_>>();
    let baseline_status = baseline["validation"]["status"]
        .as_str()
        .unwrap_or("unknown");
    let best_status = best["validation"]["status"].as_str().unwrap_or("unknown");
    let running = state.is_none_or(|state| {
        matches!(
            state.status,
            StudyStatus::Ready | StudyStatus::Running | StudyStatus::Paused
        )
    });
    let status = if baseline_status == "recorded" && best_status == "recorded" {
        "available"
    } else if running && (baseline_status == "pending" || best_status == "pending") {
        "pending"
    } else if baseline_status == "recorded"
        || best_status == "recorded"
        || baseline_status == "partial"
        || best_status == "partial"
    {
        "partial"
    } else if baseline_status == "pending" && best_status == "pending" {
        "pending"
    } else {
        "unavailable"
    };
    let mut reasons = Vec::new();
    for side in [baseline, best] {
        let validation = &side["validation"];
        if validation["status"] == "recorded" {
            continue;
        }
        if let Some(reason) = validation["reason"].as_str() {
            reasons.push(compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT));
        }
        for reason in validation["reasons"].as_array().into_iter().flatten() {
            if let Some(reason) = reason.as_str() {
                reasons.push(compact_evidence_text(reason, EVIDENCE_TEXT_LIMIT));
            }
        }
        for error in validation["errors"].as_array().into_iter().flatten() {
            let text = format!(
                "{} / {}: {}",
                error["site"].as_str().unwrap_or("?"),
                error["key"].as_str().unwrap_or("?"),
                error["reason"].as_str().unwrap_or("unavailable"),
            );
            reasons.push(compact_evidence_text(&text, EVIDENCE_TEXT_LIMIT));
        }
    }
    if best["member"].is_null() {
        reasons.push("no best member is recorded to compare validation against".into());
    }
    reasons.sort();
    reasons.dedup();
    reasons.truncate(EVIDENCE_LIST_LIMIT);
    json!({
        "configured": true,
        "windows": windows,
        "status": status,
        "reasons": reasons,
    })
}

fn evidence_parameters(
    manifest: &Manifest,
    members: Option<&[MemberPlan]>,
    members_error: Option<&str>,
    best_id: Option<&str>,
) -> serde_json::Value {
    let Some(members) = members else {
        return json!({
            "status": "unavailable",
            "reason": members_error.unwrap_or("the recorded sample files could not be loaded"),
        });
    };
    let dimensions = super::sample::dimensions(&manifest.spec);
    let sampled = dimensions
        .iter()
        .filter(|dimension| !super::hybrid::is_weight_key(&dimension.key))
        .collect::<Vec<_>>();
    let hybrid = dimensions
        .iter()
        .filter(|dimension| super::hybrid::is_weight_key(&dimension.key))
        .collect::<Vec<_>>();
    let baseline = members.iter().find(|member| member.baseline);
    let best = best_id.and_then(|id| members.iter().find(|member| member.id == id));
    let shown = &sampled[..sampled.len().min(EVIDENCE_LIST_LIMIT * 2)];
    let mut value = json!({
        "status": match (best, best_id) {
            (Some(_), _) => "recorded",
            (None, Some(_)) => "missing_best",
            (None, None) => "no_best",
        },
        "dimension_count": sampled.len(),
        "hybrid_weight_count": hybrid.len(),
        "dims": shown
            .iter()
            .map(|dimension| json!({
                "key": dimension.key,
                "range": [dimension.min, dimension.max],
                "scale": scale_name(dimension.scale),
            }))
            .collect::<Vec<_>>(),
        "baseline": parameter_side(
            baseline,
            shown,
            &hybrid,
            "no baseline member is recorded in the Study samples",
        ),
        "best": parameter_side(
            best,
            shown,
            &hybrid,
            if best_id.is_some() {
                "the recorded sample files do not contain the best member"
            } else {
                "no best member is recorded"
            },
        ),
    });
    if best_id.is_some() && best.is_none() {
        value["reason"] = json!("the recorded sample files do not contain the best member");
    } else if best_id.is_none() {
        value["reason"] = json!("no best member is recorded");
    }
    if let Some(reason) = members_error {
        value["samples_error"] = json!(reason);
    }
    if sampled.len() > shown.len() {
        value["truncated"] = json!(true);
        value["omitted_dimensions"] = json!(sampled.len() - shown.len());
    }
    value
}

fn parameter_side(
    member: Option<&MemberPlan>,
    dims: &[&super::sample::Dimension],
    hybrid: &[&super::sample::Dimension],
    missing_reason: &str,
) -> serde_json::Value {
    let Some(member) = member else {
        return json!({
            "status": "unavailable",
            "reason": missing_reason,
        });
    };
    let values = dims
        .iter()
        .map(|dimension| {
            let Some(&value) = member.parameters.get(&dimension.key) else {
                return json!({
                    "key": dimension.key,
                    "status": "unavailable",
                    "reason": "the member has no value for this dimension",
                });
            };
            let position = super::generation::unit_position(value, dimension);
            let bound_side = match position {
                Some(position) if position <= BOUND_MARGIN => Some("lower"),
                Some(position) if position >= 1.0 - BOUND_MARGIN => Some("upper"),
                _ => None,
            };
            json!({
                "key": dimension.key,
                "value": value,
                "position": position,
                "position_scale": scale_name(dimension.scale),
                "at_bound": bound_side.is_some(),
                "bound_side": bound_side,
            })
        })
        .collect::<Vec<_>>();
    let hybrid_value = if hybrid.is_empty() {
        json!({
            "status": "not_configured",
            "reason": "this Study trains no hybrid network",
        })
    } else {
        let recorded = member
            .parameters
            .keys()
            .filter(|key| super::hybrid::is_weight_key(key))
            .count();
        if recorded != hybrid.len() {
            json!({
                "status": "unavailable",
                "reason": if recorded == 0 {
                    "the member has no hybrid network weights (physics-only reference)"
                } else {
                    "the member has an incomplete hybrid weight vector"
                },
            })
        } else {
            let at_bound = hybrid
                .iter()
                .filter(|dimension| {
                    member
                        .parameters
                        .get(&dimension.key)
                        .and_then(|&value| super::generation::unit_position(value, dimension))
                        .is_some_and(|position| {
                            position <= BOUND_MARGIN || position >= 1.0 - BOUND_MARGIN
                        })
                })
                .count();
            json!({
                "status": "recorded",
                "count": hybrid.len(),
                "at_bound_count": at_bound,
            })
        }
    };
    json!({
        "status": "recorded",
        "values": values,
        "hybrid": hybrid_value,
    })
}

fn evidence_model_identity(study_dir: &Path) -> serde_json::Value {
    use crate::remote_cmd::remote_study::{recorded_model_version, RecordedModelVersion};

    let study_record = match crate::study_version::pinned(study_dir) {
        Ok(Some(pinned)) => {
            let revision = pinned.model_revision.as_ref();
            json!({
                "status": "pinned",
                "source": "model_version_file",
                "name": pinned.name,
                "head": pinned.head,
                "base_commit": pinned.base_commit,
                "version": revision.map(|revision| json!({
                    "id": revision.version.id,
                    "revision": revision.version.revision,
                })),
                "code_source_sha256": revision.map(|revision| revision.code.source_sha256.clone()),
                "revision_recorded": revision.is_some(),
            })
        }
        // 没有本地 pin 不等于官方版：远程 Study 取回后 pin 可能没跟着回来，但提交记录里
        // 留下了真实身份。先读 remote record.provenance.version；只有它明确写 official
        // 才判官方版，缺失/损坏一律 unknown，不能把修改版历史结果归因给官方模型。
        Ok(None) => match recorded_model_version(study_dir) {
            Ok(Some(RecordedModelVersion::Official)) => json!({
                "status": "official",
                "source": "remote_record",
                "reason": "the local model-version.json pin is absent; the captured remote Study record declares the official version",
            }),
            Ok(Some(RecordedModelVersion::Workspace {
                name,
                head,
                base_commit,
            })) => json!({
                "status": "workspace",
                "source": "remote_record",
                "name": name,
                "head": head,
                "base_commit": base_commit,
                "version": null,
                "code_source_sha256": null,
                "reason": "the local model-version.json pin is absent; identity comes from the captured remote Study record, which records the workspace name and commit but no frozen revision id or source hash",
            }),
            Ok(None) => json!({
                "status": "unknown",
                "source": "none",
                "reason": "no model-version.json pin and no captured remote Study record; official versus workspace cannot be established (legacy or local-only Study)",
            }),
            Err(error) => json!({
                "status": "unknown",
                "source": "remote_record",
                "reason": compact_evidence_text(
                    &format!(
                        "the captured remote Study record cannot establish the model version: {error:#}"
                    ),
                    EVIDENCE_TEXT_LIMIT,
                ),
            }),
        },
        Err(error) => json!({
            "status": "unknown",
            "source": "model_version_file",
            "reason": compact_evidence_text(&format!("{error:#}"), EVIDENCE_TEXT_LIMIT),
        }),
    };
    json!({
        "study_record": study_record,
        "current_selection": null,
        "comparison": {
            "status": "unknown",
            "matches": null,
            "basis": "the CLI has no application selection to compare; the assistant compares the current selection with this recorded Study identity",
        },
    })
}

fn evidence_remote(study_dir: &Path) -> serde_json::Value {
    let record = study_dir.join(crate::remote_cmd::remote_study::RECORD);
    if record.is_file() {
        return match crate::remote_cmd::remote_study::read_record(study_dir) {
            Ok(record) => json!({
                "status": "recorded",
                "host": record.host,
                "job": record.job,
                "submitted_at": record.submitted_at,
                "fetched": record.fetched_at.is_some(),
                "note": "recorded submission metadata only; this status read made no SSH connection",
            }),
            Err(error) => json!({
                "status": "unknown",
                "reason": compact_evidence_text(&format!("{error:#}"), EVIDENCE_TEXT_LIMIT),
                "note": "the recorded submission file could not be validated; no SSH connection was made",
            }),
        };
    }
    match crate::remote_cmd::remote_study::pending_status(study_dir) {
        Ok(Some(pending)) => json!({
            "status": "pending_reconciliation",
            "job": pending["job"],
            "note": "a submission intent was recorded but no result record exists; no SSH connection was made",
        }),
        _ => json!({
            "status": "none",
            "note": "no remote submission was recorded; all evidence is local",
        }),
    }
}

/// `results/tasks/**/*.json` 的文件数：记录的证据来自哪些结果文件，而不是当场重算的结论。
fn recorded_result_files(manifest: &Manifest) -> usize {
    let root = Path::new(&manifest.root).join("results/tasks");
    let mut files = Vec::new();
    let _ = collect_result_files(&root, &root, &mut files);
    files
        .iter()
        .filter(|file| file.path.ends_with(".json"))
        .count()
}

fn evidence_verification(manifest: &Manifest, state: Option<&StudyState>) -> serde_json::Value {
    let succeeded_tasks = state.map_or(0, |state| {
        state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Succeeded)
            .count()
    });
    json!({
        "recorded_result_files": recorded_result_files(manifest),
        "succeeded_tasks": succeeded_tasks,
        "reverified": false,
        "note": "scores and losses are read from recorded task-result files; they were not re-verified against history output, observations or the frozen kernel by this status read",
    })
}

fn evidence_bytes(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

fn truncate_evidence_array(value: &mut serde_json::Value, path: &[&str], keep: usize) -> bool {
    let mut current = value;
    for key in path {
        let Some(next) = current.get_mut(*key) else {
            return false;
        };
        current = next;
    }
    let Some(array) = current.as_array_mut() else {
        return false;
    };
    if array.len() <= keep {
        return false;
    }
    array.truncate(keep);
    true
}

/// 把证据压到 [`EVIDENCE_BUDGET_BYTES`] 以内：先砍列表，再收缩到只保留身份、选择、两期分数与缺失原因。
/// 正常研究（少量目标、少量参数）不会触发裁剪。
fn bound_evidence(mut evidence: serde_json::Value) -> serde_json::Value {
    if evidence_bytes(&evidence) <= EVIDENCE_BUDGET_BYTES {
        return evidence;
    }
    evidence["truncated"] = json!(true);
    let trims: &[(&[&str], usize)] = &[
        (&["warnings"], 3),
        (&["missing"], 3),
        (&["execution", "failed_tasks"], 2),
        (&["validation", "windows"], 3),
        (&["objective", "targets"], 4),
        (&["comparison", "baseline", "calibration", "targets"], 2),
        (&["comparison", "baseline", "validation", "targets"], 2),
        (&["comparison", "best", "calibration", "targets"], 2),
        (&["comparison", "best", "validation", "targets"], 2),
        (&["parameters", "dims"], 8),
        (&["parameters", "baseline", "values"], 8),
        (&["parameters", "best", "values"], 8),
        (&["parameters", "dims"], 4),
        (&["parameters", "baseline", "values"], 4),
        (&["parameters", "best", "values"], 4),
        (&["objective", "targets"], 2),
        (&["validation", "windows"], 2),
        (&["comparison", "baseline", "validation", "errors"], 2),
        (&["comparison", "best", "validation", "errors"], 2),
    ];
    for (path, keep) in trims {
        if evidence_bytes(&evidence) <= EVIDENCE_BUDGET_BYTES {
            break;
        }
        truncate_evidence_array(&mut evidence, path, *keep);
    }
    if evidence_bytes(&evidence) <= EVIDENCE_BUDGET_BYTES {
        return evidence;
    }
    // 极端情况（目标或参数特别多）：只留判断所需的最小结构，仍然保持合法 JSON 与显式缺失。
    let baseline = &evidence["comparison"]["baseline"];
    let best = &evidence["comparison"]["best"];
    json!({
        "schema": 1,
        "truncated": true,
        "study": evidence["study"].clone(),
        "execution": {
            "state": evidence["execution"]["state"],
            "tasks": evidence["execution"]["tasks"],
            "partial_failures": evidence["execution"]["partial_failures"],
            "note": "execution details omitted to fit the evidence budget",
        },
        "selection": {
            "rule": evidence["selection"]["rule"],
            "best_member": evidence["selection"]["best_member"],
            "best_objective": evidence["selection"]["best_objective"],
            "provisional": evidence["selection"]["provisional"],
            "no_best_reason": evidence["selection"]["no_best_reason"],
        },
        "objective": {
            "direction": "minimize",
            "loss": evidence["objective"]["loss"],
            "target_count": evidence["objective"]["target_count"],
        },
        "comparison": {
            "basis": evidence["comparison"]["basis"],
            "baseline": period_scores(baseline),
            "best": period_scores(best),
            "relation": evidence["comparison"]["relation"],
        },
        "validation": {
            "configured": evidence["validation"]["configured"],
            "status": evidence["validation"]["status"],
            "reasons": evidence["validation"]["reasons"],
            "note": "windows omitted to fit the evidence budget",
        },
        "parameters": {
            "status": evidence["parameters"]["status"],
            "dimension_count": evidence["parameters"]["dimension_count"],
            "hybrid_weight_count": evidence["parameters"]["hybrid_weight_count"],
            "note": "parameter values omitted to fit the evidence budget",
        },
        "model_identity": evidence["model_identity"].clone(),
        "verification": evidence["verification"].clone(),
        "remote": evidence["remote"].clone(),
        "source_paths": evidence["source_paths"].clone(),
        "warnings": [],
        "missing": evidence["missing"],
    })
}

fn period_scores(side: &serde_json::Value) -> serde_json::Value {
    json!({
        "member": side["member"],
        "calibration": {"status": side["calibration"]["status"], "score": side["calibration"]["score"]},
        "validation": {"status": side["validation"]["status"], "score": side["validation"]["score"]},
    })
}

/// 运行用的内核目录必须是建研究时冻结的那一份。
///
/// 例外：固定了模型修订的研究（`study_version::pin_revision`）会把那份内核复制到
/// `version/kernel`，再带着这份副本委托给固定的 `colm-cli`。只比路径的话，这类研究
/// 一启动就报 "Study was created for kernel …"（实测：GUI 里新建的参数率定全部如此）。
/// 内容由上面的 manifest 身份检查与 `Kernel::open` 的 sha256 校验把关，这里只放行
/// 研究自己记录的那份副本，别的目录照旧拒绝。
fn ensure_frozen_kernel_dir(study_dir: &Path, frozen: &Path, kernel_dir: &Path) -> Result<()> {
    let frozen = colm_kernel::manifest::absolute(frozen)?;
    let kernel_dir = &colm_kernel::manifest::absolute(kernel_dir)?;
    if frozen == *kernel_dir {
        return Ok(());
    }
    let pinned_copy = crate::study_version::pinned(study_dir)?
        .map(|pin| colm_kernel::manifest::absolute(&pin.kernel))
        .transpose()?;
    if pinned_copy.as_deref() == Some(kernel_dir)
        && kernel_dir.starts_with(colm_kernel::manifest::absolute(study_dir)?)
    {
        return Ok(());
    }
    bail!(
        "Study was created for kernel {}, not {}",
        frozen.display(),
        kernel_dir.display()
    );
}

pub fn run(study_dir: &Path, options: RunOptions<'_>) -> Result<StudyState> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let _run_lock = StudyRunLock::acquire(&study_dir)?;
    let kernel_dir = colm_kernel::manifest::absolute(options.kernel_dir)?;
    let manifest = super::engine::status(&study_dir)?;
    // As in run_case, only an entirely Rust pipeline can run with a manifest alone.
    let kernel = if options.engine == crate::ModelEngine::Rust
        && options.preprocessors == crate::PreprocessorMode::Rust
    {
        colm_kernel::Kernel::open_manifest(&kernel_dir)?
    } else {
        colm_kernel::Kernel::open(&kernel_dir)?
    };
    let kernel_identity = format!(
        "{} ({})",
        kernel.manifest.identity(),
        kernel.manifest.platform
    );
    // Match the backend identity written by run_case before reusing completed tasks.
    let kernel_id = format!(
        "{};preprocessors={}",
        kernel.manifest.stage_fingerprint_identity(),
        options.preprocessors.as_str()
    );
    if options.preprocessors == crate::PreprocessorMode::Rust
        && kernel
            .manifest
            .macros
            .iter()
            .any(|name| name == "HYPERSPECTRAL")
    {
        bail!("Study Rust preprocessing does not yet support HYPERSPECTRAL input fingerprints; use --preprocessors fortran");
    }
    super::engine::ensure_supported_study_manifest(&manifest, Some(&kernel.manifest.macros))?;
    if manifest.spec.hybrid.is_some() && options.engine != crate::ModelEngine::Rust {
        bail!("a Study that trains a hybrid network needs --engine rust");
    }
    let mut rust_identities = if options.preprocessors == crate::PreprocessorMode::Rust {
        crate::RustPreprocessorIdentities {
            surface: Some(crate::rust_preprocessor_stage_identity(
                Path::new(&manifest.root),
                crate::Stage::MkSrfData,
                &[],
            )?),
            initial: Some(crate::rust_preprocessor_stage_identity(
                Path::new(&manifest.root),
                crate::Stage::MkIniData,
                &[],
            )?),
            model: None,
        }
    } else {
        crate::RustPreprocessorIdentities::default()
    };
    // 与 `run_case` 同一条身份规则：否则复用判断会把 Fortran 跑出来的成员当成
    // Rust 引擎的结果（或反之）而跳过。
    if options.engine == crate::ModelEngine::Rust {
        rust_identities.model = Some(crate::rust_model_identity()?);
    }
    if manifest.spec.kernel_dir.is_none() || manifest.provenance.kernel_id.is_empty() {
        bail!("Study has no frozen kernel identity; create a new Study");
    }
    if !manifest.provenance.kernel_id.is_empty() && manifest.provenance.kernel_id != kernel_identity
    {
        bail!(
            "Study was created for kernel {}, not {}; create a new Study",
            manifest.provenance.kernel_id,
            kernel_identity
        );
    }
    if let Some(frozen) = manifest.spec.kernel_dir.as_deref() {
        ensure_frozen_kernel_dir(&study_dir, Path::new(frozen), &kernel_dir)?;
    }
    super::engine::verify_frozen_inputs(&manifest)?;
    super::model::verify_if_captured(&manifest)?;
    let checkpoint_dir = study_dir.join("checkpoints/state");
    let mut state = super::checkpoint::load_latest::<StudyState>(&checkpoint_dir)?
        .map(|loaded| loaded.payload)
        .with_context(|| format!("{} has no Study state checkpoint", study_dir.display()))?;
    if state.study_id != manifest.id {
        bail!(
            "Study state belongs to {}, not {}",
            state.study_id,
            manifest.id
        );
    }
    if state
        .tasks
        .values()
        .any(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Evaluating))
    {
        state.reconcile_unverified_running();
        super::checkpoint::write_next(&checkpoint_dir, &state)?;
        bail!("Study has tasks requiring review before it can be resumed");
    }

    let mut members = super::generation::reconcile_tasks(&manifest, &mut state)?;
    state.status = StudyStatus::Running;
    super::checkpoint::write_next(&checkpoint_dir, &state)?;
    let available = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let jobs = bounded_jobs(options.jobs, available);
    // `colm-rs` 默认开满全部逻辑核；`jobs` 个成员同时跑时让它们平分，否则线程数是核数的
    // `jobs` 倍。成员的子进程继承这个环境变量。用户自己设了就不动。此时工作线程还没起，
    // 进程里没有别的线程在读环境。
    if jobs > 1 && std::env::var_os("RAYON_NUM_THREADS").is_none() {
        std::env::set_var(
            "RAYON_NUM_THREADS",
            threads_per_job(available, jobs).to_string(),
        );
    }
    emit(
        &study_dir,
        options.stream,
        serde_json::json!({"type":"study_started","kind":"study_started","study":manifest.id,"jobs":jobs}),
    )?;

    if manifest.spec.kind == StudyKind::Tuning {
        if !baseline_tasks_succeeded(&manifest, &state) {
            run_members(
                &study_dir,
                &manifest,
                &mut state,
                &checkpoint_dir,
                &kernel_dir,
                &kernel_id,
                options.preprocessors,
                options.engine,
                &rust_identities,
                jobs,
                options.stream,
                options.retry_failed,
                None,
            )?;
            refresh_candidates(&manifest, &members, &mut state)?;
        }
        if !baseline_tasks_succeeded(&manifest, &state) {
            let cancelled = super::state::cancel_requested(&study_dir);
            let paused = super::state::pause_requested(&study_dir);
            if cancelled {
                cancel_unstarted(&mut state);
                state.status = StudyStatus::Cancelled;
            } else if paused {
                state.status = StudyStatus::Paused;
            } else {
                state.status = StudyStatus::CompletedWithFailures;
                push_warning_once(
                    &mut state,
                    "tuning baseline failed; candidates were not run and can be retried after fixing the baseline".into(),
                );
            }
            let _ = write_objective_tables(&manifest, &state);
            super::checkpoint::write_next(&checkpoint_dir, &state)?;
            emit(
                &study_dir,
                options.stream,
                serde_json::json!({"type":"study_done","kind":"study_done","study":manifest.id,"status":state.status}),
            )?;
            return Ok(state);
        }
        ensure_tuning_baseline_ready(&manifest, &state)?;
    }
    run_members(
        &study_dir,
        &manifest,
        &mut state,
        &checkpoint_dir,
        &kernel_dir,
        &kernel_id,
        options.preprocessors,
        options.engine,
        &rust_identities,
        jobs,
        options.stream,
        options.retry_failed,
        None,
    )?;
    refresh_candidates(&manifest, &members, &mut state)?;

    if manifest.spec.method == StudyMethod::DifferentialEvolution
        && !super::state::pause_requested(&study_dir)
        && !super::state::cancel_requested(&study_dir)
        && state.status != StudyStatus::NeedsReview
    {
        run_de_generations(
            &study_dir,
            &manifest,
            &mut state,
            &checkpoint_dir,
            &kernel_dir,
            &kernel_id,
            options.preprocessors,
            options.engine,
            &rust_identities,
            jobs,
            options.stream,
            &mut members,
        )?;
    }

    refresh_candidates(&manifest, &members, &mut state)?;
    let cancelled = super::state::cancel_requested(&study_dir);
    let paused = super::state::pause_requested(&study_dir);
    if cancelled {
        cancel_unstarted(&mut state);
    }
    state.finish_status();
    if cancelled {
        state.status = StudyStatus::Cancelled;
    } else if paused {
        state.status = StudyStatus::Paused;
    }

    if manifest.spec.kind == StudyKind::Uncertainty {
        update_uncertainty_outputs(&manifest, &members, &mut state);
    }
    if let Err(error) = write_objective_tables(&manifest, &state) {
        push_warning_once(
            &mut state,
            format!("Study summary tables could not be written: {error}"),
        );
        if state.status == StudyStatus::Completed {
            state.status = StudyStatus::CompletedWithFailures;
        }
    }
    super::checkpoint::write_next(&checkpoint_dir, &state)?;
    emit(
        &study_dir,
        options.stream,
        serde_json::json!({"type":"study_done","kind":"study_done","study":manifest.id,"status":state.status}),
    )?;
    Ok(state)
}

fn bounded_jobs(requested: usize, available: usize) -> usize {
    requested.clamp(1, available.max(1))
}

/// 同时跑 `jobs` 个成员时，每个成员的 Rust 引擎线程数（`RAYON_NUM_THREADS`）：平分 CPU 核。
fn threads_per_job(available: usize, jobs: usize) -> usize {
    (available / jobs.max(1)).max(1)
}

pub(super) fn ensure_scheduler_idle(study_dir: &Path) -> Result<()> {
    clear_stale_run_lock(study_dir)
}

pub fn status_state(study_dir: &Path) -> Result<Option<StudyState>> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let checkpoint_dir = study_dir.join("checkpoints/state");
    let Some(mut state) =
        super::checkpoint::load_latest::<StudyState>(&checkpoint_dir)?.map(|loaded| loaded.payload)
    else {
        return Ok(None);
    };
    let manifest = super::engine::status(&study_dir)?;
    if state.study_id != manifest.id {
        bail!(
            "Study checkpoint id {} does not match manifest {}",
            state.study_id,
            manifest.id
        );
    }
    if scheduler_is_live(&study_dir)? {
        return Ok(Some(state));
    }
    if mark_exited_active_tasks_for_review(&mut state)? {
        super::checkpoint::write_next(&checkpoint_dir, &state)?;
    }
    Ok(Some(state))
}

fn scheduler_is_live(study_dir: &Path) -> Result<bool> {
    let Some(owner) = run_lock_owner(&study_dir.join("run.lock"))? else {
        return Ok(false);
    };
    scheduler_owner_live(&owner)
}

/// The lock owner may still write checkpoints: its heartbeat is fresh, or its PID is alive and
/// still runs the recorded scheduler executable. A stale heartbeat alone (sleep, clock jump,
/// one failed heartbeat write) must not let `study-status` become a second checkpoint writer
/// next to a running scheduler; a PID reused by another program is not that scheduler.
fn scheduler_owner_live(owner: &ProcessIdentity) -> Result<bool> {
    if unix_now().saturating_sub(owner.heartbeat_unix) <= RUN_LOCK_STALE_SECONDS {
        return Ok(true);
    }
    if !scheduler_process_alive(owner.pid)? {
        return Ok(false);
    }
    let recorded = Path::new(&owner.executable).file_name();
    Ok(match process_image_name(owner.pid) {
        // When the platform cannot say what the PID runs, stay on the safe side: no second writer.
        None => true,
        Some(name) => recorded.is_some_and(|recorded| recorded == name.as_os_str()),
    })
}

/// File name of the executable a live PID runs, when the platform can tell.
#[cfg(target_os = "linux")]
fn process_image_name(pid: u32) -> Option<std::path::PathBuf> {
    let target = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let text = target.to_string_lossy();
    // A scheduler binary replaced on disk while running reads back as "... (deleted)".
    let text = text.strip_suffix(" (deleted)").unwrap_or(&text);
    Path::new(text).file_name().map(std::path::PathBuf::from)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_image_name(pid: u32) -> Option<std::path::PathBuf> {
    // BSD `ps -o comm=` prints the executable path.
    let output = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (output.status.success() && !line.is_empty())
        .then(|| Path::new(line).file_name().map(std::path::PathBuf::from))
        .flatten()
}

#[cfg(windows)]
fn process_image_name(pid: u32) -> Option<std::path::PathBuf> {
    let mut command = std::process::Command::new("tasklist");
    colm_kernel::run::no_console(&mut command);
    let output = command
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()?
        .split(',')
        .next()
        .map(|image| std::path::PathBuf::from(image.trim().trim_matches('"')))
        .filter(|image| !image.as_os_str().is_empty())
}

#[cfg(not(any(unix, windows)))]
fn process_image_name(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

fn mark_exited_active_tasks_for_review(state: &mut StudyState) -> Result<bool> {
    let mut changed = false;
    for task in state.tasks.values_mut() {
        if matches!(task.status, TaskStatus::Running | TaskStatus::Evaluating) {
            let process_alive = match &task.process {
                Some(process) => scheduler_process_alive(process.pid)?,
                None => false,
            };
            task.status = TaskStatus::NeedsReview;
            task.stage = None;
            task.reason = Some(
                if process_alive {
                    "previous Study scheduler ownership could not be verified"
                } else if task.process.is_some() {
                    "previous Study worker exited before reporting a final status"
                } else {
                    "previous Study worker has no verifiable process identity"
                }
                .into(),
            );
            if !process_alive {
                task.process = None;
            }
            changed = true;
        }
    }
    if changed {
        state.finish_status();
        state.status = StudyStatus::NeedsReview;
    }
    Ok(changed)
}

/// GUI 已确认对应进程树退出后再落盘；PID 必须对上，避免迟到的取消请求
/// 误关掉后来启动的调度器。
pub fn finalize_cancel(study_dir: &Path, expected_pid: u32) -> Result<StudyState> {
    finalize_cancel_impl(study_dir, Some(expected_pid))
}

pub fn finalize_idle_cancel(study_dir: &Path) -> Result<StudyState> {
    finalize_cancel_impl(study_dir, None)
}

fn finalize_cancel_impl(study_dir: &Path, expected_pid: Option<u32>) -> Result<StudyState> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let lock_path = study_dir.join("run.lock");
    let checkpoint_dir = study_dir.join("checkpoints/state");
    let mut state = super::checkpoint::load_latest::<StudyState>(&checkpoint_dir)?
        .map(|loaded| loaded.payload)
        .with_context(|| format!("{} has no Study state checkpoint", study_dir.display()))?;
    let had_lock = match run_lock_owner(&lock_path)? {
        Some(owner) => {
            if let Some(expected_pid) = expected_pid {
                if owner.pid != expected_pid {
                    bail!(
                        "Study scheduler changed while cancelling: expected PID {expected_pid}, found {}",
                        owner.pid
                    );
                }
            }
            if scheduler_process_alive(owner.pid)? {
                bail!("Study scheduler PID {} is still running", owner.pid);
            }
            true
        }
        None => false,
    };
    let mut changed = false;
    for task in state.tasks.values_mut() {
        if matches!(task.status, TaskStatus::Running | TaskStatus::Evaluating) {
            let Some(process) = &task.process else {
                bail!("Study has active tasks without a verified process identity");
            };
            if scheduler_process_alive(process.pid)? {
                bail!("Study worker PID {} is still running", process.pid);
            }
        } else if task.status == TaskStatus::NeedsReview
            && task
                .process
                .as_ref()
                .is_some_and(|process| scheduler_process_alive(process.pid).unwrap_or(true))
        {
            bail!("Study has a review task with a live unverified process");
        }
        if matches!(
            task.status,
            TaskStatus::Pending
                | TaskStatus::Materialized
                | TaskStatus::Queued
                | TaskStatus::Running
                | TaskStatus::Evaluating
                | TaskStatus::NeedsReview
        ) {
            task.status = TaskStatus::Cancelled;
            task.stage = None;
            task.process = None;
            task.reason = Some("cancelled by user".into());
            changed = true;
        }
    }
    state.finish_status();
    if changed {
        state.status = StudyStatus::Cancelled;
        super::checkpoint::write_next(&checkpoint_dir, &state)?;
        emit(
            &study_dir,
            false,
            serde_json::json!({"type":"study_cancelled","kind":"study_cancelled","study":state.study_id,"status":state.status}),
        )?;
    }
    if had_lock {
        fs::remove_file(&lock_path).with_context(|| {
            format!("cannot remove cancelled Study lock {}", lock_path.display())
        })?;
    }
    Ok(state)
}

#[cfg(unix)]
fn scheduler_process_alive(pid: u32) -> Result<bool> {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .context("cannot inspect Study scheduler process")?;
    Ok(output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|status| !status.trim_start().starts_with('Z')))
}

#[cfg(windows)]
fn scheduler_process_alive(pid: u32) -> Result<bool> {
    let mut command = std::process::Command::new("tasklist");
    colm_kernel::run::no_console(&mut command);
    let output = command
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .context("cannot inspect Study scheduler process")?;
    if !output.status.success() {
        bail!("tasklist failed while checking Study scheduler PID {pid}");
    }
    let pid = pid.to_string();
    Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        line.split(',')
            .nth(1)
            .is_some_and(|field| field.trim().trim_matches('"') == pid)
    }))
}

#[cfg(not(any(unix, windows)))]
fn scheduler_process_alive(_pid: u32) -> Result<bool> {
    bail!("cannot verify a Study scheduler process on this platform")
}

fn baseline_tasks_succeeded(manifest: &Manifest, state: &StudyState) -> bool {
    manifest.spec.base_cases.iter().all(|site| {
        state
            .tasks
            .get(&super::state::task_id("m000000", site))
            .is_some_and(|task| task.status == TaskStatus::Succeeded)
    })
}

fn ensure_tuning_baseline_ready(manifest: &Manifest, state: &StudyState) -> Result<()> {
    if manifest.spec.kind != StudyKind::Tuning {
        return Ok(());
    }
    for site in &manifest.spec.base_cases {
        let task = state
            .tasks
            .get(&super::state::task_id("m000000", site))
            .with_context(|| format!("missing tuning baseline task for {site}"))?;
        if task.status != TaskStatus::Succeeded {
            bail!("tuning baseline for {site} did not finish successfully; fix it before running candidates");
        }
        let result = read_task_result(manifest, "m000000", site)?;
        for row in result.calibration.iter().chain(&result.validation) {
            if row.pairs < row.min_pairs {
                bail!(
                    "baseline target {} ({}) has only {} real model-observation pairs in {} for {site}; need at least {}",
                    row.key, row.variable, row.pairs, row.period, row.min_pairs
                );
            }
        }
    }
    Ok(())
}

fn preflight_observation_window(
    observation: &Path,
    site: &str,
    target: &TargetSpec,
    validation: bool,
) -> Result<()> {
    let (from, to, period) = if validation {
        (
            target.validation_from.context("missing validation start")?,
            target.validation_to.context("missing validation end")?,
            "validation",
        )
    } else {
        (target.from, target.to, "calibration")
    };
    let times = crate::observation_usable_times(observation, &target.variable, from, to)?;
    // 按日、按月比时，运行时数的是凑够观测的时段个数，不是逐时观测点数。原来预检按点数判 `min_pairs`：
    // 14 天的逐时观测有几百个点，预检通过，运行时只有 14 个日均，跑完基准才失败（复验 A2-03，25 次模拟白跑）。
    // 模型每个时刻都有值，所以按观测自己能凑够的时段数判，是运行时配对数的上界，与 `aggregate_pairs` 同一口径。
    let Some(aggregate) = target.aggregate else {
        if times.len() < target.min_pairs {
            bail!(
                "target {} ({}) has only {} usable observation points in the {period} window for {site}; need at least {}",
                target.key,
                target.variable,
                times.len(),
                target.min_pairs
            );
        }
        return Ok(());
    };
    let zeros = vec![0.0; times.len()];
    let count = super::targets::aggregate_pairs(&times, &zeros, &zeros, aggregate)?
        .support
        .len();
    if count < target.min_pairs {
        let unit = match aggregate {
            TargetPeriod::Daily => "daily",
            TargetPeriod::Monthly => "monthly",
        };
        bail!(
            "target {} ({}) has only {count} {unit} means with enough observations in the {period} window for {site}; need at least {}",
            target.key, target.variable, target.min_pairs
        );
    }
    Ok(())
}

fn push_warning_once(state: &mut StudyState, warning: String) {
    if !state.warnings.contains(&warning) {
        state.warnings.push(warning);
    }
}

fn update_uncertainty_outputs(manifest: &Manifest, members: &[MemberPlan], state: &mut StudyState) {
    clear_uncertainty_aggregation_warnings(state);
    if let Err(error) = clear_uncertainty_summary_outputs(manifest) {
        push_warning_once(
            state,
            format!("uncertainty result aggregation failed: {error}"),
        );
        if state.status == StudyStatus::Completed {
            state.status = StudyStatus::CompletedWithFailures;
        }
        return;
    }
    if baseline_tasks_succeeded(manifest, state) {
        if let Err(error) = write_uncertainty_results(manifest, members, state) {
            push_warning_once(
                state,
                format!("uncertainty result aggregation failed: {error}"),
            );
            if state.status == StudyStatus::Completed {
                state.status = StudyStatus::CompletedWithFailures;
            }
        }
    } else {
        push_warning_once(
            state,
            "uncertainty results are unavailable until every baseline task succeeds".into(),
        );
    }
}

fn clear_uncertainty_aggregation_warnings(state: &mut StudyState) {
    state.warnings.retain(|warning| {
        warning != "uncertainty results are unavailable until every baseline task succeeds"
            && !warning.starts_with("uncertainty result aggregation failed:")
            && !warning.contains(" timestamps have insufficient ensemble support")
            && !warning.contains(" has a different time axis and was excluded")
            && !warning.contains(" was excluded:")
    });
}

fn clear_uncertainty_summary_outputs(manifest: &Manifest) -> Result<()> {
    let results = Path::new(&manifest.root).join("results");
    remove_path_if_present(&results.join("envelopes"))?;
    remove_path_if_present(&results.join("importance.json"))?;
    remove_path_if_present(&results.join("members.csv"))?;
    Ok(())
}

fn remove_path_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn study_task_is_current(
    manifest: &Manifest,
    task: &super::state::TaskState,
    kernel_id: &str,
    rust_identities: &crate::RustPreprocessorIdentities,
) -> bool {
    crate::case_is_current(Path::new(&task.case_dir), kernel_id, rust_identities).unwrap_or(false)
        && (!Path::new(&manifest.root)
            .join(super::model::IDENTITY_FILE)
            .is_file()
            || verify_task_evidence(
                manifest,
                &task.member,
                &task.site,
                Path::new(&task.case_dir),
            )
            .is_ok())
}

#[allow(clippy::too_many_arguments)]
fn run_members(
    study_dir: &Path,
    manifest: &Manifest,
    state: &mut StudyState,
    checkpoint_dir: &Path,
    kernel_dir: &Path,
    kernel_id: &str,
    preprocessors: crate::PreprocessorMode,
    engine: crate::ModelEngine,
    rust_identities: &crate::RustPreprocessorIdentities,
    jobs: usize,
    stream: bool,
    retry_failed: bool,
    generation: Option<usize>,
) -> Result<()> {
    let members = super::generation::load_members(manifest)?;
    let members_by_id = if manifest.spec.kind == StudyKind::Tuning
        && manifest.spec.method == StudyMethod::DifferentialEvolution
    {
        members
            .iter()
            .cloned()
            .map(|member| (member.id.clone(), member))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let generation_members = generation.map(|generation| {
        members
            .iter()
            .filter(|member| member.generation == generation && !member.baseline)
            .map(|member| member.id.clone())
            .collect::<BTreeSet<_>>()
    });
    let closed_generation = state.generation;
    let population = state.population.clone();
    if manifest.spec.kind == StudyKind::Tuning
        && manifest.spec.method == StudyMethod::DifferentialEvolution
    {
        for task in state.tasks.values_mut() {
            let stale_success = task.status == TaskStatus::Succeeded
                && !study_task_is_current(manifest, task, kernel_id, rust_identities);
            if stale_success
                && !retryable_task(
                    manifest,
                    closed_generation,
                    &population,
                    &members_by_id,
                    &task.member,
                )
            {
                task.status = TaskStatus::Failed;
                task.objective = None;
                task.validation_objective = None;
                task.reason = Some(format!(
                    "cannot rerun closed DE generation for {}; create a new Study",
                    task.member
                ));
            }
        }
    }
    let mut runnable = VecDeque::new();
    for task in state.tasks.values() {
        if generation_members
            .as_ref()
            .is_some_and(|ids| !ids.contains(&task.member))
        {
            continue;
        }
        let stale_success = task.status == TaskStatus::Succeeded
            && !study_task_is_current(manifest, task, kernel_id, rust_identities);
        let waiting_for_tuning_baseline = manifest.spec.kind == StudyKind::Tuning
            && !baseline_tasks_succeeded(manifest, state)
            && task.member != "m000000";
        let retry_allowed = retry_failed
            && matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted)
            && retryable_task(
                manifest,
                state.generation,
                &state.population,
                &members_by_id,
                &task.member,
            );
        let allowed = !waiting_for_tuning_baseline
            && (matches!(
                task.status,
                TaskStatus::Pending | TaskStatus::Materialized | TaskStatus::Queued
            ) || stale_success
                || retry_allowed);
        if allowed {
            runnable.push_back((
                task.member.clone(),
                task.site.clone(),
                PathBuf::from(&task.case_dir),
            ));
        }
    }
    if runnable.is_empty() {
        return Ok(());
    }

    for (member, site, _) in &runnable {
        state.set_task(member, site, TaskStatus::Queued)?;
        if let Some(task) = state.tasks.get_mut(&super::state::task_id(member, site)) {
            task.reason = None;
            task.objective = None;
            task.validation_objective = None;
        }
    }
    super::checkpoint::write_next(checkpoint_dir, state)?;

    let queue = Arc::new(Mutex::new(runnable));
    let (tx, rx) = mpsc::channel();
    let worker_count = jobs.max(1).min(queue.lock().unwrap().len());
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let kernel_dir = kernel_dir.to_path_buf();
            let study_dir = study_dir.to_path_buf();
            scope.spawn(move || loop {
                if super::state::pause_requested(&study_dir)
                    || super::state::cancel_requested(&study_dir)
                {
                    break;
                }
                let Some((member, site, case_dir)) = queue.lock().unwrap().pop_front() else {
                    break;
                };
                if tx
                    .send(WorkerEvent::Started {
                        member: member.clone(),
                        site: site.clone(),
                    })
                    .is_err()
                {
                    break;
                }
                let event_tx = tx.clone();
                let event_member = member.clone();
                let event_site = site.clone();
                let mut last_task_log_emit = None;
                let execution = begin_execution(manifest, &site, &case_dir);
                let spawn_tx = tx.clone();
                let (spawn_member, spawn_site) = (member.clone(), site.clone());
                let observer = move |pid: u32, program: &Path| {
                    let _ = spawn_tx.send(WorkerEvent::Process {
                        member: spawn_member.clone(),
                        site: spawn_site.clone(),
                        pid,
                        program: program.to_path_buf(),
                    });
                };
                let result = colm_kernel::observe_spawns(observer, || {
                    execution.and_then(|identity| {
                        crate::run_case(
                            &case_dir,
                            &kernel_dir,
                            stream,
                            false,
                            None,
                            1,
                            preprocessors,
                            engine,
                            None,
                            None,
                            true,
                            true,
                            &mut |notice| {
                                let event = match notice {
                                    RunNotice::StageBegin(stage) => WorkerEvent::Notice {
                                        member: event_member.clone(),
                                        site: event_site.clone(),
                                        kind: "stage_started".into(),
                                        stage: stage.into(),
                                        line: None,
                                        ok: None,
                                    },
                                    RunNotice::StageSkipped(stage) => WorkerEvent::Notice {
                                        member: event_member.clone(),
                                        site: event_site.clone(),
                                        kind: "stage_skipped".into(),
                                        stage: stage.into(),
                                        line: None,
                                        ok: Some(true),
                                    },
                                    RunNotice::Log { stage, line } => {
                                        // The complete member log is already on disk. Sampling here,
                                        // before the unbounded scheduler channel, prevents parallel
                                        // CoLM chatter from exhausting memory while preserving live logs.
                                        if line.trim().is_empty()
                                            || !should_emit_task_log(
                                                &mut last_task_log_emit,
                                                Instant::now(),
                                            )
                                        {
                                            return;
                                        }
                                        WorkerEvent::Notice {
                                            member: event_member.clone(),
                                            site: event_site.clone(),
                                            kind: "task_log".into(),
                                            stage: stage.into(),
                                            line: Some(line.into()),
                                            ok: None,
                                        }
                                    }
                                    RunNotice::StageDone { stage, ok } => WorkerEvent::Notice {
                                        member: event_member.clone(),
                                        site: event_site.clone(),
                                        kind: "stage_done".into(),
                                        stage: stage.into(),
                                        line: None,
                                        ok: Some(ok),
                                    },
                                };
                                let _ = event_tx.send(event);
                            },
                        )?;
                        finish_execution(&case_dir, identity)
                    })
                });
                let _ = tx.send(WorkerEvent::Finished {
                    member,
                    site,
                    case_dir,
                    error: result.err().map(|error| error.to_string()),
                });
            });
        }
        drop(tx);
        for event in rx {
            match event {
                WorkerEvent::Started { member, site } => {
                    state.set_task(&member, &site, TaskStatus::Running)?;
                    if let Some(task) = state.tasks.get_mut(&super::state::task_id(&member, &site))
                    {
                        task.process = Some(supervisor_identity());
                    }
                    emit(
                        study_dir,
                        stream,
                        serde_json::json!({"type":"task_started","kind":"task_started","member":member,"site":site}),
                    )?;
                    super::checkpoint::write_next(checkpoint_dir, state)?;
                }
                WorkerEvent::Process {
                    member,
                    site,
                    pid,
                    program,
                } => {
                    // Track the process that actually computes, so status and cancellation see a
                    // model that outlives a killed scheduler instead of re-checking our own PID.
                    if let Some(task) = state.tasks.get_mut(&super::state::task_id(&member, &site))
                    {
                        if matches!(task.status, TaskStatus::Running | TaskStatus::Evaluating) {
                            let now = unix_now();
                            task.process = Some(ProcessIdentity {
                                pid,
                                executable: program.to_string_lossy().into_owned(),
                                argv_sha256: String::new(),
                                started_unix: now,
                                heartbeat_unix: now,
                            });
                            super::checkpoint::write_next(checkpoint_dir, state)?;
                        }
                    }
                }
                WorkerEvent::Notice {
                    member,
                    site,
                    kind,
                    stage,
                    line,
                    ok,
                } => {
                    if let Some(task) = state.tasks.get_mut(&super::state::task_id(&member, &site))
                    {
                        task.stage = Some(stage.clone());
                    }
                    if stream && (kind != "task_log" || line.is_some()) {
                        emit(
                            study_dir,
                            true,
                            serde_json::json!({"type":kind,"kind":kind,"member":member,"site":site,"stage":stage,"line":line,"ok":ok}),
                        )?;
                    }
                }
                WorkerEvent::Finished {
                    member,
                    site,
                    case_dir,
                    error,
                } => {
                    finish_task(
                        manifest,
                        state,
                        checkpoint_dir,
                        &member,
                        &site,
                        &case_dir,
                        error,
                        stream,
                    )?;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;

    let cancelled = super::state::cancel_requested(study_dir);
    if cancelled {
        cancel_unstarted(state);
    }
    state.finish_batch_status(cancelled || super::state::pause_requested(study_dir));
    super::checkpoint::write_next(checkpoint_dir, state)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_task(
    manifest: &Manifest,
    state: &mut StudyState,
    checkpoint_dir: &Path,
    member: &str,
    site: &str,
    case_dir: &Path,
    error: Option<String>,
    stream: bool,
) -> Result<()> {
    let id = super::state::task_id(member, site);
    if let Some(reason) = error {
        state.set_task(member, site, TaskStatus::Failed)?;
        if let Some(task) = state.tasks.get_mut(&id) {
            task.reason = Some(reason.clone());
            task.process = None;
        }
        emit(
            Path::new(&manifest.root),
            stream,
            serde_json::json!({"type":"task_failed","kind":"task_failed","member":member,"site":site,"reason":reason}),
        )?;
    } else {
        state.set_task(member, site, TaskStatus::Evaluating)?;
        super::checkpoint::write_next(checkpoint_dir, state)?;
        match evaluate_task(manifest, member, site, case_dir) {
            Ok(result) => {
                let calibration = fixed_score(&result.calibration);
                let validation = fixed_score(&result.validation);
                state.set_task(member, site, TaskStatus::Succeeded)?;
                if let Some(task) = state.tasks.get_mut(&id) {
                    task.objective = calibration;
                    task.validation_objective = validation;
                    task.reason = None;
                    task.stage = None;
                    task.process = None;
                }
                emit(
                    Path::new(&manifest.root),
                    stream,
                    serde_json::json!({"type":"task_done","kind":"task_done","member":member,"site":site,"objective":calibration,"validation_objective":validation}),
                )?;
            }
            Err(error) => {
                state.set_task(member, site, TaskStatus::Failed)?;
                if let Some(task) = state.tasks.get_mut(&id) {
                    task.reason = Some(format!("evaluation failed: {error}"));
                    task.process = None;
                }
                emit(
                    Path::new(&manifest.root),
                    stream,
                    serde_json::json!({"type":"task_failed","kind":"task_failed","member":member,"site":site,"reason":format!("evaluation failed: {error}")}),
                )?;
            }
        }
    }
    state.completed_candidates = state.finished_candidates(&manifest.spec.base_cases);
    super::checkpoint::write_next(checkpoint_dir, state)?;
    Ok(())
}

const EXECUTION_IDENTITY_FILE: &str = "study-execution-identity.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ExecutionIdentity {
    schema: u32,
    inputs: crate::fingerprint::FullInputIdentity,
    observation: Option<crate::fingerprint::FullInputFile>,
    #[serde(default)]
    file_stats: BTreeMap<PathBuf, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationIdentity {
    schema: u32,
    execution: ExecutionIdentity,
    outputs: BTreeMap<String, crate::fingerprint::FullInputFile>,
    result: crate::fingerprint::FullInputFile,
}

fn input_file_stats<'a>(
    files: impl IntoIterator<Item = &'a crate::fingerprint::FullInputFile>,
) -> Result<BTreeMap<PathBuf, String>> {
    files
        .into_iter()
        .map(|file| {
            Ok((
                file.path.clone(),
                crate::fingerprint::stable_file_identity(&fs::metadata(&file.path)?),
            ))
        })
        .collect()
}

fn actual_execution(manifest: &Manifest, site: &str, case: &Path) -> Result<ExecutionIdentity> {
    let inputs = crate::fingerprint::full_inputs(case)?;
    let observation = manifest
        .spec
        .observations
        .get(site)
        .map(|path| crate::fingerprint::full_file(Path::new(path), "observation"))
        .transpose()?;
    let file_stats = input_file_stats(inputs.files.values().chain(observation.iter()))?;
    Ok(ExecutionIdentity {
        schema: 1,
        inputs,
        observation,
        file_stats,
    })
}

fn begin_execution(
    manifest: &Manifest,
    site: &str,
    case: &Path,
) -> Result<Option<ExecutionIdentity>> {
    if !Path::new(&manifest.root)
        .join(super::model::IDENTITY_FILE)
        .is_file()
    {
        return Ok(None);
    }
    let path = case.join(EXECUTION_IDENTITY_FILE);
    match fs::remove_file(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    // Sampled legacy stage marks cannot certify a new full-content execution.
    crate::fingerprint::save(case, &BTreeMap::new())?;
    let identity = actual_execution(manifest, site, case)?;
    let creation: super::model::StudyInputIdentity = serde_json::from_slice(&fs::read(
        Path::new(&manifest.root).join(super::model::IDENTITY_FILE),
    )?)?;
    let baseline = creation
        .base_cases
        .get(site)
        .context("Study site has no creation input identity")?;
    anyhow::ensure!(
        baseline.complete
            && identity.inputs.complete
            && baseline.algorithm == identity.inputs.algorithm
            && baseline.data_settings.get("forcing_content_policy")
                == identity.inputs.data_settings.get("forcing_content_policy")
            && baseline.data_sha256.is_some()
            && baseline.data_sha256 == identity.inputs.data_sha256,
        "Study member non-forcing data or coverage policy differs from its frozen baseline"
    );
    anyhow::ensure!(
        identity.observation == creation.observations.get(site).cloned(),
        "Study observations changed since creation"
    );
    Ok(Some(identity))
}

fn finish_execution(case: &Path, expected: Option<ExecutionIdentity>) -> Result<()> {
    if let Some(expected) = expected {
        anyhow::ensure!(
            expected.inputs == crate::fingerprint::full_inputs(case)?,
            "Study scientific inputs changed during execution"
        );
        if let Some(observation) = &expected.observation {
            anyhow::ensure!(
                *observation == crate::fingerprint::full_file(&observation.path, "observation")?,
                "Study observations changed during execution"
            );
        }
        anyhow::ensure!(
            expected.file_stats
                == input_file_stats(
                    expected
                        .inputs
                        .files
                        .values()
                        .chain(expected.observation.iter())
                )?,
            "Study input files changed during execution, including a restored payload"
        );
        write_json(&case.join(EXECUTION_IDENTITY_FILE), &expected)?;
    }
    Ok(())
}

fn history_identity(case: &Path) -> Result<BTreeMap<String, crate::fingerprint::FullInputFile>> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let files = crate::history_files(&layout.out().join(&name))?;
    files
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("invalid history filename")?
                .to_owned();
            Ok((name, crate::fingerprint::full_file(&path, "output")?))
        })
        .collect()
}

type EvaluationEvidence = (
    ExecutionIdentity,
    BTreeMap<String, crate::fingerprint::FullInputFile>,
    BTreeMap<PathBuf, String>,
);

fn before_evaluation(
    manifest: &Manifest,
    site: &str,
    case: &Path,
) -> Result<Option<EvaluationEvidence>> {
    if !Path::new(&manifest.root)
        .join(super::model::IDENTITY_FILE)
        .is_file()
    {
        return Ok(None);
    }
    let expected: ExecutionIdentity = serde_json::from_slice(
        &fs::read(case.join(EXECUTION_IDENTITY_FILE))
            .context("Study task has no completed full-input execution evidence")?,
    )?;
    anyhow::ensure!(
        expected.schema == 1 && expected == actual_execution(manifest, site, case)?,
        "Study task inputs or observations changed before evaluation"
    );
    let outputs = history_identity(case)?;
    let output_stats = input_file_stats(outputs.values())?;
    Ok(Some((expected, outputs, output_stats)))
}

fn evaluation_identity_path(manifest: &Manifest, member: &str, site: &str) -> PathBuf {
    Path::new(&manifest.root)
        .join("results/provenance")
        .join(member)
        .join(format!("{site}.json"))
}

/// A cached objective is usable only while its full inputs, observations and raw outputs still match.
pub(crate) fn verify_member_evidence(
    manifest: &Manifest,
    state: &StudyState,
    member: &str,
) -> Result<()> {
    for site in &manifest.spec.base_cases {
        let task = state
            .tasks
            .get(&super::state::task_id(member, site))
            .context("Study member has no task")?;
        anyhow::ensure!(
            task.status == TaskStatus::Succeeded,
            "Study member task did not succeed"
        );
        let expected = Path::new(&manifest.root)
            .join("members")
            .join(member)
            .join(site);
        anyhow::ensure!(
            expected.canonicalize()? == Path::new(&task.case_dir).canonicalize()?,
            "Study task directory differs from its frozen member location"
        );
        verify_task_evidence(manifest, member, site, Path::new(&task.case_dir))?;
    }
    let mut recalculated = state.clone();
    refresh_candidates(
        manifest,
        &super::generation::load_members(manifest)?,
        &mut recalculated,
    )?;
    let recorded = state
        .candidates
        .get(member)
        .context("Study member has no recorded candidate objective")?;
    // 记下的分数来自检查点，经过一次 JSON 往返（末位可能变）；重算的也走同一遍再比。
    let actual = super::checkpoint::as_checkpointed(
        recalculated
            .candidates
            .get(member)
            .context("Study member objective cannot be recalculated")?,
    )?;
    anyhow::ensure!(
        recorded.feasible
            && actual.feasible
            && recorded.calibration == actual.calibration
            && recorded.validation == actual.validation,
        "Study candidate score differs from its verified evaluation results \
         ({member}: recorded {:?}/{:?}, recalculated {:?}/{:?}, feasible {}/{})",
        recorded.calibration,
        recorded.validation,
        actual.calibration,
        actual.validation,
        recorded.feasible,
        actual.feasible
    );
    Ok(())
}

/// Stat guards span the whole comparison; every payload is still fully hashed by the verifier.
pub(crate) fn evidence_file_snapshot(
    manifest: &Manifest,
    state: &StudyState,
    members: &[&str],
) -> Result<BTreeMap<PathBuf, String>> {
    let root = Path::new(&manifest.root);
    let identity_path = root.join(super::model::IDENTITY_FILE);
    let inputs: super::model::StudyInputIdentity =
        serde_json::from_slice(&fs::read(&identity_path)?)?;
    let mut paths = BTreeSet::from([identity_path, root.join("manifest.json")]);
    if root.join(crate::study_version::FILE).is_file() {
        paths.insert(root.join(crate::study_version::FILE));
    }
    for file in inputs
        .base_cases
        .values()
        .flat_map(|input| input.files.values())
        .chain(inputs.observations.values())
    {
        paths.insert(file.path.clone());
    }
    for member in members {
        for site in &manifest.spec.base_cases {
            let task = state
                .tasks
                .get(&super::state::task_id(member, site))
                .context("Study member has no task")?;
            let proof_path = evaluation_identity_path(manifest, member, site);
            let proof: EvaluationIdentity = serde_json::from_slice(&fs::read(&proof_path)?)?;
            paths.insert(proof_path);
            paths.insert(task_result_path(manifest, member, site));
            for file in proof
                .execution
                .inputs
                .files
                .values()
                .chain(proof.execution.observation.iter())
            {
                paths.insert(file.path.clone());
            }
            let layout = Layout::new(Path::new(&task.case_dir));
            let name = colm_case::case_name(&layout.case_nml())?;
            paths.extend(crate::history_files(&layout.out().join(&name))?);
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let metadata = fs::metadata(&path)?;
            anyhow::ensure!(
                metadata.is_file(),
                "Study proof dependency is not a regular file"
            );
            Ok((path, crate::fingerprint::stable_file_identity(&metadata)))
        })
        .collect()
}

fn verify_task_evidence(manifest: &Manifest, member: &str, site: &str, case: &Path) -> Result<()> {
    let evidence: EvaluationIdentity = serde_json::from_slice(
        &fs::read(evaluation_identity_path(manifest, member, site))
            .context("Study member lacks full-content evaluation evidence")?,
    )?;
    anyhow::ensure!(
        evidence.schema == 1,
        "unsupported Study evaluation identity schema"
    );
    let actual = actual_execution(manifest, site, case)?;
    anyhow::ensure!(
        crate::fingerprint::same_payloads(&evidence.execution.inputs, &actual.inputs),
        "Study member scientific inputs changed after evaluation"
    );
    let same_observation = match (&evidence.execution.observation, &actual.observation) {
        (None, None) => true,
        (Some(a), Some(b)) => a.sha256 == b.sha256 && a.bytes == b.bytes,
        _ => false,
    };
    anyhow::ensure!(
        same_observation,
        "Study member observations changed after evaluation"
    );
    let actual_outputs = history_identity(case)?;
    anyhow::ensure!(
        same_file_contents(&evidence.outputs, &actual_outputs),
        "Study raw history output changed after evaluation"
    );
    let result =
        crate::fingerprint::full_file(&task_result_path(manifest, member, site), "evaluation")?;
    anyhow::ensure!(
        result.sha256 == evidence.result.sha256 && result.bytes == evidence.result.bytes,
        "Study evaluation result changed after it was computed"
    );
    read_task_result(manifest, member, site)?;
    Ok(())
}

fn same_file_contents(
    a: &BTreeMap<String, crate::fingerprint::FullInputFile>,
    b: &BTreeMap<String, crate::fingerprint::FullInputFile>,
) -> bool {
    !a.is_empty()
        && a.len() == b.len()
        && a.iter().all(|(name, left)| {
            b.get(name)
                .is_some_and(|right| left.sha256 == right.sha256 && left.bytes == right.bytes)
        })
}

/// 这个站点的目标逐个配对：率定期必算；有验证期的再算验证期，验证期失败只记原因、不让任务失败。
fn evaluate_targets(
    manifest: &Manifest,
    site: &str,
    case: &Path,
    result: &mut TaskResult,
) -> Result<()> {
    for target in manifest
        .spec
        .targets
        .iter()
        .filter(|target| target.site.as_deref().is_none_or(|wanted| wanted == site))
    {
        // 单点比 `observations` 里该站的文件；区域里的通量站比它自己的文件；格点产品与水文站自己配对。
        let observation = match &target.source {
            Some(source) => PathBuf::from(source.path()),
            None => PathBuf::from(
                manifest
                    .spec
                    .observations
                    .get(site)
                    .with_context(|| format!("missing frozen observation for {site}"))?,
            ),
        };
        let evaluate = |validation: bool| {
            if matches!(
                target.source,
                Some(TargetSource::Grid { .. } | TargetSource::Gauge { .. })
            ) {
                paired_target_result(
                    case,
                    site,
                    target,
                    validation,
                    manifest.spec.base_cases.len(),
                )
            } else {
                target_result(
                    case,
                    &observation,
                    site,
                    target,
                    validation,
                    manifest.spec.base_cases.len(),
                )
            }
        };
        result.calibration.push(evaluate(false)?);
        if target.validation_from.is_some() {
            match evaluate(true) {
                Ok(row) => result.validation.push(row),
                Err(error) => result.validation_errors.push(TargetError {
                    key: target.key.clone(),
                    site: site.into(),
                    variable: target.variable.clone(),
                    period: "validation".into(),
                    reason: error.to_string(),
                }),
            }
        }
    }
    Ok(())
}

fn evaluate_task(manifest: &Manifest, member: &str, site: &str, case: &Path) -> Result<TaskResult> {
    let evidence = before_evaluation(manifest, site, case)?;
    let mut result = TaskResult {
        member: member.into(),
        site: site.into(),
        outputs: Vec::new(),
        calibration: Vec::new(),
        validation: Vec::new(),
        validation_errors: Vec::new(),
    };
    match manifest.spec.kind {
        StudyKind::Uncertainty => {
            for variable in &manifest.spec.outputs {
                let (_, values, units) = model_series(
                    case,
                    variable,
                    manifest.spec.analysis_from,
                    manifest.spec.analysis_to,
                )?;
                let finite = values
                    .into_iter()
                    .filter(|value| value.is_finite())
                    .collect::<Vec<_>>();
                if finite.is_empty() {
                    bail!("{variable} has no finite values in the analysis window");
                }
                let mean = super::science::finite_mean(&finite)
                    .with_context(|| format!("{variable} mean is not finite"))?;
                result.outputs.push(OutputSummary {
                    variable: variable.clone(),
                    units,
                    count: finite.len(),
                    mean,
                    min: finite.iter().copied().fold(f64::INFINITY, f64::min),
                    max: finite.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                });
            }
            // 带了目标的不确定性分析：每个成员也算目标函数，敏感性才能对着"拟合好不好"排。
            evaluate_targets(manifest, site, case, &mut result)?;
        }
        StudyKind::Tuning => {
            evaluate_targets(manifest, site, case, &mut result)?;
            if result.calibration.is_empty() {
                bail!("parameter tuning has no target covering site {site}");
            }
        }
    }
    let result_path = task_result_path(manifest, member, site);
    write_json(&result_path, &result)?;
    if let Some((execution, outputs, output_stats)) = evidence {
        anyhow::ensure!(
            execution == actual_execution(manifest, site, case)?,
            "Study inputs or observations changed during evaluation"
        );
        anyhow::ensure!(
            outputs == history_identity(case)?,
            "Study history outputs changed during evaluation"
        );
        anyhow::ensure!(
            output_stats == input_file_stats(outputs.values())?,
            "Study history output files changed during evaluation, including a restored payload"
        );
        let evidence = EvaluationIdentity {
            schema: 1,
            execution,
            outputs,
            result: crate::fingerprint::full_file(&result_path, "evaluation")?,
        };
        write_json(&evaluation_identity_path(manifest, member, site), &evidence)?;
    }
    Ok(result)
}

fn target_result(
    case: &Path,
    observation: &Path,
    site: &str,
    target: &TargetSpec,
    validation: bool,
    site_count: usize,
) -> Result<TargetResult> {
    let (from, to, period) = if validation {
        (
            target.validation_from.context("missing validation start")?,
            target.validation_to.context("missing validation end")?,
            "validation",
        )
    } else {
        (target.from, target.to, "calibration")
    };
    // 格点算例里的通量站：比站点所在那一格；单点算例取整条站点序列。
    let point = if let Some(super::spec::TargetSource::Station {
        lon,
        lat,
        utc_offset,
        ..
    }) = &target.source
    {
        Some(crate::StationPoint {
            lon: *lon,
            lat: *lat,
            utc_offset_hours: *utc_offset,
        })
    } else {
        None
    };
    let rows = crate::compute_metric_rows(MetricsRequest {
        case,
        obs_path: observation,
        spinup: 0,
        json: true,
        corrected: target.corrected,
        summary_only: false,
        pair_vars: vec![target.variable.clone()],
        pair_max_points: None,
        from: Some(from),
        to: Some(to),
        point,
    })?;
    let row = rows
        .into_iter()
        .find(|row| row.name.eq_ignore_ascii_case(&target.variable))
        .with_context(|| {
            format!(
                "required target {} ({}) is unavailable for {site}",
                target.key, target.variable
            )
        })?;
    // 观测文件没有订正版时 `compute_metric_rows` 会退回未订正的那条；研究里那等于悄悄换了目标，
    // 所以要求实际读到的就是订正版。
    if let Some(wanted) = target
        .corrected
        .then(|| colm_hist::obs::corrected(&row.name))
        .flatten()
    {
        if !row.obs_var.eq_ignore_ascii_case(wanted) {
            bail!(
                "target {} asks for energy-balance-corrected {wanted}, but the observation file for {site} has none",
                target.key
            );
        }
    }
    let time = row.time.as_ref().with_context(|| {
        format!(
            "target {} ({}) did not return full pair support",
            target.key, target.variable
        )
    })?;
    let obs = row.obs.as_ref().with_context(|| {
        format!(
            "target {} ({}) did not return full paired observations",
            target.key, target.variable
        )
    })?;
    if time.len() != row.n || obs.len() != row.n {
        bail!(
            "target {} ({}) returned incomplete support for {} pairs",
            target.key,
            target.variable,
            row.n
        );
    }
    // 要按日或按月比时，把逐时配对平均成时段再算指标（支持集也换成时段）。
    let (support_hash, pairs, value, observation_sd, model_mean, observation_mean) = if let Some(
        aggregate,
    ) =
        target.aggregate
    {
        let model = row.model.as_ref().with_context(|| {
            format!(
                "target {} ({}) did not return full paired model values",
                target.key, target.variable
            )
        })?;
        let paired = super::targets::aggregate_pairs(time, model, obs, aggregate)?;
        let metrics = colm_hist::metric::compute(&paired.pairs()).with_context(|| {
                format!(
                    "target {} ({}) has fewer than two {aggregate:?} means with enough data for {site} in the {period} window",
                    target.key, target.variable
                )
            })?;
        let value = match target.metric {
            ObjectiveMetric::Nrmse => metrics.rmse,
            ObjectiveMetric::Mae => metrics.mae,
            ObjectiveMetric::AbsBias => metrics.bias,
            ObjectiveMetric::Nse => metrics.nse,
            ObjectiveMetric::Kge => metrics.kge,
            ObjectiveMetric::R2 => metrics.r2,
            ObjectiveMetric::R => metrics.correlation,
        };
        (
            pair_support_hash(&paired.support, &paired.obs),
            metrics.n,
            value,
            metrics.obs_sd,
            metrics.model_mean,
            metrics.obs_mean,
        )
    } else {
        (
            pair_support_hash(time, obs),
            row.n,
            metric_value(&row, target.metric),
            row.obs_sd,
            row.model_mean,
            row.obs_mean,
        )
    };
    let weight = if target.site.is_none() {
        target.weight / site_count.max(1) as f64
    } else {
        target.weight
    };
    let term = ObjectiveTerm {
        metric: target.metric,
        value,
        observation_sd: Some(observation_sd),
        weight,
        pairs,
    };
    let loss = super::science::objective_loss(&term, target.min_pairs)
        .map_err(|reason| anyhow::anyhow!("target {} {reason}", target.key))?;
    Ok(TargetResult {
        key: target.key.clone(),
        site: site.into(),
        variable: target.variable.clone(),
        period: period.into(),
        metric: target.metric,
        weight,
        min_pairs: target.min_pairs,
        pairs,
        value,
        observation_sd: Some(observation_sd),
        loss,
        model_mean,
        observation_mean,
        support_hash,
    })
}

/// 格点观测产品与水文站：先按时段平均、配好对（[`super::targets`]），再与站点目标同一套指标和损失。
fn paired_target_result(
    case: &Path,
    site: &str,
    target: &TargetSpec,
    validation: bool,
    site_count: usize,
) -> Result<TargetResult> {
    let (from, to, period) = if validation {
        (
            target.validation_from.context("missing validation start")?,
            target.validation_to.context("missing validation end")?,
            "validation",
        )
    } else {
        (target.from, target.to, "calibration")
    };
    let paired = match &target.source {
        Some(TargetSource::Grid { .. }) => super::targets::grid_pairs(case, target, (from, to))?,
        Some(TargetSource::Gauge { .. }) => super::targets::gauge_pairs(case, target, (from, to))?,
        Some(TargetSource::Station { .. }) | None => {
            bail!("target {} is compared through the site metrics", target.key)
        }
    };
    let metrics = colm_hist::metric::compute(&paired.pairs()).with_context(|| {
        format!(
            "target {} ({}) has fewer than two paired values for {site} in the {period} window",
            target.key, target.variable
        )
    })?;
    let value = match target.metric {
        ObjectiveMetric::Nrmse => metrics.rmse,
        ObjectiveMetric::Mae => metrics.mae,
        ObjectiveMetric::AbsBias => metrics.bias,
        ObjectiveMetric::Nse => metrics.nse,
        ObjectiveMetric::Kge => metrics.kge,
        ObjectiveMetric::R2 => metrics.r2,
        ObjectiveMetric::R => metrics.correlation,
    };
    let weight = if target.site.is_none() {
        target.weight / site_count.max(1) as f64
    } else {
        target.weight
    };
    let term = ObjectiveTerm {
        metric: target.metric,
        value,
        observation_sd: Some(metrics.obs_sd),
        weight,
        pairs: metrics.n,
    };
    let loss = super::science::objective_loss(&term, target.min_pairs)
        .map_err(|reason| anyhow::anyhow!("target {} {reason}", target.key))?;
    Ok(TargetResult {
        key: target.key.clone(),
        site: site.into(),
        variable: target.variable.clone(),
        period: period.into(),
        metric: target.metric,
        weight,
        min_pairs: target.min_pairs,
        pairs: metrics.n,
        value,
        observation_sd: Some(metrics.obs_sd),
        loss,
        model_mean: metrics.model_mean,
        observation_mean: metrics.obs_mean,
        support_hash: pair_support_hash(&paired.support, &paired.obs),
    })
}

fn metric_value(row: &VarMetrics, metric: ObjectiveMetric) -> f64 {
    match metric {
        ObjectiveMetric::Nrmse => row.rmse,
        ObjectiveMetric::Mae => row.mae,
        ObjectiveMetric::AbsBias => row.bias,
        ObjectiveMetric::Nse => row.nse,
        ObjectiveMetric::Kge => row.kge,
        ObjectiveMetric::R2 => row.r2,
        ObjectiveMetric::R => row.correlation,
    }
}

fn pair_support_hash(time: &[i64], obs: &[f64]) -> String {
    let mut hash = Sha256::new();
    for (time, obs) in time.iter().zip(obs) {
        hash.update(time.to_le_bytes());
        hash.update(obs.to_bits().to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn fixed_score(results: &[TargetResult]) -> Option<f64> {
    if results.is_empty() {
        return None;
    }
    let scale = results
        .iter()
        .map(|result| result.weight.abs())
        .fold(0.0, f64::max);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let mut weight = 0.0;
    for result in results {
        let normalized = result.weight / scale;
        if !normalized.is_finite()
            || normalized < 0.0
            || !result.loss.is_finite()
            || result.loss < 0.0
        {
            return None;
        }
        weight += normalized;
    }
    if !weight.is_finite() || weight <= 0.0 {
        return None;
    }
    let mut score = 0.0;
    for result in results {
        score += (result.weight / scale / weight) * result.loss;
    }
    score.is_finite().then_some(score)
}

fn expected_target_results<'a>(
    manifest: &'a Manifest,
    site: &'a str,
) -> impl Iterator<Item = (&'a TargetSpec, &'static str)> + 'a {
    manifest
        .spec
        .targets
        .iter()
        .filter(move |target| target.site.as_deref().is_none_or(|wanted| wanted == site))
        .flat_map(|target| {
            std::iter::once((target, "calibration")).chain(
                target
                    .validation_from
                    .is_some()
                    .then_some((target, "validation")),
            )
        })
}

fn validate_task_result(
    manifest: &Manifest,
    member: &str,
    site: &str,
    result: &TaskResult,
) -> Result<()> {
    if result.member != member || result.site != site {
        bail!(
            "task result identity mismatch: expected {member}/{site}, found {}/{}",
            result.member,
            result.site
        );
    }
    if manifest.spec.targets.is_empty() {
        return Ok(());
    }
    let mut rows = BTreeMap::new();
    let mut validation_errors = BTreeMap::new();
    for error in &result.validation_errors {
        if error.site != site
            || error.period != "validation"
            || error.reason.trim().is_empty()
            || !manifest.spec.targets.iter().any(|target| {
                target.key == error.key
                    && target.variable == error.variable
                    && target.validation_from.is_some()
                    && target.site.as_deref().is_none_or(|wanted| wanted == site)
            })
        {
            bail!("invalid validation error for target {}", error.key);
        }
        if validation_errors.insert(error.key.clone(), error).is_some() {
            bail!("duplicate validation error for target {}", error.key);
        }
    }
    for (stored_period, row) in result
        .calibration
        .iter()
        .map(|row| ("calibration", row))
        .chain(result.validation.iter().map(|row| ("validation", row)))
    {
        if row.period != stored_period {
            bail!(
                "target {} is stored in {stored_period} but declares {}",
                row.key,
                row.period
            );
        }
        if row.site != site {
            bail!("target {} belongs to {}, not {site}", row.key, row.site);
        }
        if !row.weight.is_finite()
            || !row.value.is_finite()
            || !row.loss.is_finite()
            || row.loss < 0.0
            || !row.model_mean.is_finite()
            || !row.observation_mean.is_finite()
            || row.observation_sd.is_some_and(|value| !value.is_finite())
        {
            bail!("target {} has a non-finite score field", row.key);
        }
        if row.support_hash.is_empty() || row.pairs == 0 {
            bail!("target {} has missing pair support", row.key);
        }
        let expected_loss = super::science::objective_loss(
            &ObjectiveTerm {
                metric: row.metric,
                value: row.value,
                observation_sd: row.observation_sd,
                weight: row.weight,
                pairs: row.pairs,
            },
            row.min_pairs,
        )
        .map_err(|reason| anyhow::anyhow!("target {} {reason}", row.key))?;
        if (row.loss - expected_loss).abs() > 1e-12 {
            bail!("target {} loss does not match its metric fields", row.key);
        }
        let id = (row.period.clone(), row.key.clone());
        if rows.insert(id, row).is_some() {
            bail!("duplicate target result {} / {}", row.period, row.key);
        }
    }
    for (target, period) in expected_target_results(manifest, site) {
        let Some(row) = rows.remove(&(period.into(), target.key.clone())) else {
            if period == "validation" && validation_errors.remove(&target.key).is_some() {
                continue;
            }
            bail!(
                "missing required target {} in {period} for {site}",
                target.key
            );
        };
        if period == "validation" && validation_errors.contains_key(&target.key) {
            bail!("target {} has both validation result and error", target.key);
        }
        if row.variable != target.variable
            || row.metric != target.metric
            || row.min_pairs != target.min_pairs
        {
            bail!(
                "target {} metadata does not match the frozen Study spec",
                target.key
            );
        }
        let expected_weight = if target.site.is_none() {
            target.weight / manifest.spec.base_cases.len().max(1) as f64
        } else {
            target.weight
        };
        if row.weight != expected_weight {
            bail!(
                "target {} weight does not match the frozen Study spec",
                target.key
            );
        }
    }
    if !rows.is_empty() {
        bail!("task result contains target rows outside the frozen Study spec");
    }
    if !validation_errors.is_empty() {
        bail!("task result contains validation errors outside the frozen Study spec");
    }
    Ok(())
}

fn comparable_to_baseline(candidate: &TaskResult, baseline: &TaskResult) -> Result<()> {
    let mut baseline_rows = BTreeMap::new();
    for row in &baseline.calibration {
        baseline_rows.insert((row.period.as_str(), row.key.as_str()), row);
    }
    for row in &candidate.calibration {
        let base = baseline_rows
            .get(&(row.period.as_str(), row.key.as_str()))
            .with_context(|| format!("baseline missing target {} / {}", row.period, row.key))?;
        if row.pairs != base.pairs || row.support_hash != base.support_hash {
            bail!(
                "target {} / {} uses different model-observation support than baseline",
                row.period,
                row.key
            );
        }
    }
    Ok(())
}

fn validation_comparable_to_baseline(candidate: &TaskResult, baseline: &TaskResult) -> Result<()> {
    let mut baseline_rows = BTreeMap::new();
    for row in &baseline.validation {
        baseline_rows.insert(row.key.as_str(), row);
    }
    for row in &candidate.validation {
        let base = baseline_rows
            .get(row.key.as_str())
            .with_context(|| format!("baseline missing validation target {}", row.key))?;
        if row.pairs != base.pairs || row.support_hash != base.support_hash {
            bail!(
                "validation target {} uses different model-observation support than baseline",
                row.key
            );
        }
    }
    Ok(())
}

fn refresh_candidates(
    manifest: &Manifest,
    members: &[MemberPlan],
    state: &mut StudyState,
) -> Result<()> {
    state.candidates.clear();
    for member in members {
        let tasks = manifest
            .spec
            .base_cases
            .iter()
            .filter_map(|site| state.tasks.get(&super::state::task_id(&member.id, site)))
            .collect::<Vec<_>>();
        if tasks.len() != manifest.spec.base_cases.len()
            || tasks.iter().any(|task| {
                !matches!(
                    task.status,
                    TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
                )
            })
        {
            continue;
        }
        let mut calibration = Vec::new();
        let mut validation = Vec::new();
        let mut validation_failed = false;
        let mut reason = tasks
            .iter()
            .find(|task| task.status != TaskStatus::Succeeded)
            .and_then(|task| task.reason.clone())
            .or_else(|| {
                tasks
                    .iter()
                    .find(|task| task.status != TaskStatus::Succeeded)
                    .map(|task| format!("{} was {:?}", task.site, task.status))
            });
        if reason.is_none() {
            for site in &manifest.spec.base_cases {
                match read_task_result(manifest, &member.id, site) {
                    Ok(result) => {
                        // 目标函数只有和基准同一组配对时刻、同一份观测才可比；带目标的不确定性分析也一样。
                        let scored = !manifest.spec.targets.is_empty() && member.id != "m000000";
                        let baseline = scored
                            .then(|| read_task_result(manifest, "m000000", site))
                            .transpose()?;
                        if scored {
                            let baseline = baseline.as_ref().expect("baseline loaded");
                            if let Err(error) = comparable_to_baseline(&result, baseline) {
                                reason = Some(error.to_string());
                                break;
                            }
                            if let Err(error) = validation_comparable_to_baseline(&result, baseline)
                            {
                                validation_failed = true;
                                push_warning_once(
                                    state,
                                    format!(
                                        "{} / {} validation unavailable: {}",
                                        member.id, site, error
                                    ),
                                );
                            }
                        }
                        for error in &result.validation_errors {
                            validation_failed = true;
                            push_warning_once(
                                state,
                                format!(
                                    "{} / {} validation target {} unavailable: {}",
                                    member.id, site, error.key, error.reason
                                ),
                            );
                        }
                        calibration.extend(result.calibration);
                        validation.extend(result.validation);
                    }
                    Err(error) => {
                        reason = Some(error.to_string());
                        break;
                    }
                }
            }
        }
        let calibration_score = fixed_score(&calibration);
        let validation_score = if validation_failed {
            None
        } else {
            fixed_score(&validation)
        };
        let feasible =
            reason.is_none() && (manifest.spec.targets.is_empty() || calibration_score.is_some());
        if !feasible && reason.is_none() {
            reason = Some("required target set is incomplete".into());
        }
        state.candidates.insert(
            member.id.clone(),
            CandidateState {
                generation: member.generation,
                feasible,
                calibration: calibration_score,
                validation: validation_score,
                reason,
            },
        );
    }
    state.completed_candidates = state
        .candidates
        .keys()
        .filter(|member| member.as_str() != "m000000")
        .count();
    let best = members
        .iter()
        .filter(|member| !member.baseline)
        .filter_map(|member| {
            state
                .candidates
                .get(&member.id)
                .filter(|candidate| candidate.feasible)
                .and_then(|candidate| candidate.calibration)
                .map(|score| (member.id.clone(), score))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    // 不确定性分析的样本也有目标函数，但它不是在找"最优"：不选最优，也就没有最优可写回、贴边界可报。
    let best = best.filter(|_| manifest.spec.kind == StudyKind::Tuning);
    state.best_member = best.as_ref().map(|best| best.0.clone());
    state.best_objective = best.map(|best| best.1);
    update_overfit_warning(state);
    update_bound_warnings(manifest, members, state)?;
    Ok(())
}

pub(super) fn refresh_tuning_state(manifest: &Manifest, state: &mut StudyState) -> Result<()> {
    if manifest.spec.kind != StudyKind::Tuning {
        return Ok(());
    }
    let members = super::generation::load_members(manifest)?;
    for task in state.tasks.values_mut() {
        if task.status != TaskStatus::Succeeded {
            continue;
        }
        let result = read_task_result(manifest, &task.member, &task.site).with_context(|| {
            format!(
                "cannot validate succeeded Study task result for {} / {}",
                task.member, task.site
            )
        })?;
        task.objective = fixed_score(&result.calibration);
        task.validation_objective = result
            .validation_errors
            .is_empty()
            .then(|| fixed_score(&result.validation))
            .flatten();
    }
    refresh_candidates(manifest, &members, state)
}

#[allow(clippy::too_many_arguments)]
fn run_de_generations(
    study_dir: &Path,
    manifest: &Manifest,
    state: &mut StudyState,
    checkpoint_dir: &Path,
    kernel_dir: &Path,
    kernel_id: &str,
    preprocessors: crate::PreprocessorMode,
    engine: crate::ModelEngine,
    rust_identities: &crate::RustPreprocessorIdentities,
    jobs: usize,
    stream: bool,
    members: &mut Vec<MemberPlan>,
) -> Result<()> {
    let population_size = manifest
        .spec
        .budget
        .population
        .context("missing DE population")?;
    if state.population.is_empty() {
        state.population = members
            .iter()
            .filter(|member| member.generation == 0 && !member.baseline)
            .take(population_size)
            .map(|member| member.id.clone())
            .collect();
        if manifest.spec.budget.include_baseline {
            seed_population_with_baseline(manifest, members, state)?;
        }
    }
    if state.population.len() != population_size {
        bail!("DE checkpoint population size does not match the manifest");
    }
    super::checkpoint::write_next(checkpoint_dir, state)?;
    let max_generation = manifest
        .spec
        .budget
        .generations
        .context("missing DE generations")?;
    for generation in state.generation.saturating_add(1)..=max_generation {
        if super::state::pause_requested(study_dir)
            || super::state::cancel_requested(study_dir)
            || (manifest.spec.budget.patience > 0
                && state.no_improvement_generations >= manifest.spec.budget.patience)
        {
            break;
        }
        let before = population_score(state);
        let parents = state
            .population
            .iter()
            .map(|id| {
                members
                    .iter()
                    .find(|member| &member.id == id)
                    .with_context(|| format!("missing DE parent {id}"))
            })
            .collect::<Result<Vec<_>>>()?;
        let parent_vectors = parents
            .iter()
            .map(|member| super::generation::normalized(member, manifest))
            .collect::<Result<Vec<_>>>()?;
        let trials = super::de::trial_generation(
            manifest.spec.seed,
            generation,
            &parent_vectors,
            manifest.spec.budget.mutation,
            manifest.spec.budget.crossover,
        )?;
        let physical = trials
            .iter()
            .enumerate()
            .map(|(index, vector)| {
                let values = super::generation::physical(manifest, vector)?;
                let names = super::sample::sorted_parameter_names(&manifest.spec);
                let named = names
                    .into_iter()
                    .zip(values.iter().copied())
                    .collect::<BTreeMap<_, _>>();
                // Cross-field-invalid mutants fall back to their valid parent. This
                // is deterministic and keeps the generation barrier recoverable.
                if super::sample::validate_vector(&manifest.spec, &named).is_ok() {
                    Ok(values)
                } else {
                    super::generation::physical(manifest, &parent_vectors[index])
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let trials = super::generation::write_generation(manifest, generation, &physical)?;
        *members = super::generation::reconcile_tasks(manifest, state)?;
        run_members(
            study_dir,
            manifest,
            state,
            checkpoint_dir,
            kernel_dir,
            kernel_id,
            preprocessors,
            engine,
            rust_identities,
            jobs,
            stream,
            false,
            Some(generation),
        )?;
        if super::state::pause_requested(study_dir)
            || super::state::cancel_requested(study_dir)
            || !generation_finished(&manifest.spec.base_cases, state, &trials)
        {
            // Keep the generation barrier open. On resume, queued trials finish
            // first and this same immutable generation is selected exactly once.
            super::checkpoint::write_next(checkpoint_dir, state)?;
            break;
        }
        refresh_candidates(manifest, members, state)?;
        let mut selected = Vec::with_capacity(population_size);
        for (parent, trial) in state.population.iter().zip(&trials) {
            selected.push(if trial_wins(state, parent, &trial.id) {
                trial.id.clone()
            } else {
                parent.clone()
            });
        }
        state.population = selected;
        state.generation = generation;
        refresh_candidates(manifest, members, state)?;
        let improved = generation_improved(
            before,
            population_score(state),
            manifest.spec.budget.min_improvement,
        );
        state.no_improvement_generations = if improved {
            0
        } else {
            state.no_improvement_generations + 1
        };
        write_json(
            &Path::new(&manifest.root)
                .join("results/de")
                .join(format!("g{generation:06}.json")),
            &serde_json::json!({
                "generation": generation,
                "parents": state.population,
                "best_member": state.best_member,
                "best_objective": state.best_objective,
                "no_improvement_generations": state.no_improvement_generations,
            }),
        )?;
        super::checkpoint::write_next(checkpoint_dir, state)?;
        emit(
            study_dir,
            stream,
            serde_json::json!({"type":"generation_done","kind":"generation_done","generation":generation,"best_member":state.best_member,"best_objective":state.best_objective}),
        )?;
    }
    Ok(())
}

/// 基准顶替初代里最差的成员（不可行的算最差，同分取靠后的，保证确定）。只在基准可行、严格更好、
/// 且每个参数都落在抽样范围内时顶替：范围外的基准归一化时会被夹到边上，进种群的就不再是基准本身。
/// 被顶替的初代成员不再是父代，之后也不会重试。
fn seed_population_with_baseline(
    manifest: &Manifest,
    members: &[MemberPlan],
    state: &mut StudyState,
) -> Result<()> {
    let score = |member: &str| {
        state
            .candidates
            .get(member)
            .filter(|candidate| candidate.feasible)
            .and_then(|candidate| candidate.calibration)
    };
    let Some(baseline_score) = score("m000000") else {
        return Ok(());
    };
    let Some(baseline) = members.iter().find(|member| member.baseline) else {
        return Ok(());
    };
    if !super::generation::inside_sample_box(baseline, manifest) {
        return Ok(());
    }
    let worst = state
        .population
        .iter()
        .enumerate()
        .map(|(index, id)| (index, score(id).unwrap_or(f64::INFINITY)))
        .reduce(|worst, next| if next.1 >= worst.1 { next } else { worst });
    if let Some((index, worst_score)) = worst {
        if baseline_score < worst_score {
            state.population[index] = baseline.id.clone();
        }
    }
    Ok(())
}

fn generation_finished(sites: &[String], state: &StudyState, trials: &[MemberPlan]) -> bool {
    trials.iter().all(|trial| {
        sites.iter().all(|site| {
            state
                .tasks
                .get(&super::state::task_id(&trial.id, site))
                .is_some_and(|task| {
                    matches!(
                        task.status,
                        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
                    )
                })
        })
    })
}

fn trial_wins(state: &StudyState, parent: &str, trial: &str) -> bool {
    let score = |member: &str| {
        state
            .candidates
            .get(member)
            .filter(|candidate| candidate.feasible)
            .and_then(|candidate| candidate.calibration)
    };
    match (score(parent), score(trial)) {
        (Some(parent), Some(trial)) => trial <= parent,
        (None, Some(_)) => true,
        _ => false,
    }
}

fn population_score(state: &StudyState) -> Option<f64> {
    state
        .population
        .iter()
        .filter_map(|member| {
            state
                .candidates
                .get(member)
                .filter(|candidate| candidate.feasible)
                .and_then(|candidate| candidate.calibration)
        })
        .min_by(f64::total_cmp)
}

fn generation_improved(before: Option<f64>, after: Option<f64>, min_improvement: f64) -> bool {
    match (before, after) {
        (None, Some(_)) => true,
        (Some(before), Some(after)) => {
            let gain = before - after;
            if min_improvement == 0.0 {
                gain > 0.0
            } else {
                gain >= min_improvement
            }
        }
        _ => false,
    }
}

fn write_uncertainty_results(
    manifest: &Manifest,
    members: &[MemberPlan],
    state: &mut StudyState,
) -> Result<()> {
    let root = Path::new(&manifest.root).join("results");
    let baseline = members
        .iter()
        .find(|member| member.baseline)
        .context("uncertainty Study has no baseline member")?;
    let mut means: BTreeMap<(String, String, String), f64> = BTreeMap::new();
    for member in members {
        for site in &manifest.spec.base_cases {
            if !state
                .tasks
                .get(&super::state::task_id(&member.id, site))
                .is_some_and(|task| task.status == TaskStatus::Succeeded)
            {
                continue;
            }
            let result = read_task_result(manifest, &member.id, site).with_context(|| {
                format!(
                    "cannot read succeeded Study task result for {} / {site}",
                    member.id
                )
            })?;
            for output in result.outputs {
                means.insert(
                    (member.id.clone(), site.clone(), output.variable),
                    output.mean,
                );
            }
        }
    }
    for site in &manifest.spec.base_cases {
        for variable in &manifest.spec.outputs {
            let baseline_case = Path::new(&manifest.root)
                .join("members")
                .join(&baseline.id)
                .join(site);
            let (baseline_time, baseline_values, units) = model_series(
                &baseline_case,
                variable,
                manifest.spec.analysis_from,
                manifest.spec.analysis_to,
            )?;
            let mut ensemble = Vec::new();
            for member in members.iter().filter(|member| !member.baseline) {
                let task = state.tasks.get(&super::state::task_id(&member.id, site));
                if !task.is_some_and(|task| task.status == TaskStatus::Succeeded) {
                    continue;
                }
                let case = Path::new(&manifest.root)
                    .join("members")
                    .join(&member.id)
                    .join(site);
                match model_series(
                    &case,
                    variable,
                    manifest.spec.analysis_from,
                    manifest.spec.analysis_to,
                ) {
                    Ok((time, values, _)) if time == baseline_time => ensemble.push(values),
                    Ok(_) => push_warning_once(
                        state,
                        format!(
                            "{} / {} / {} has a different time axis and was excluded",
                            member.id, site, variable
                        ),
                    ),
                    Err(error) => push_warning_once(
                        state,
                        format!(
                            "{} / {} / {} was excluded: {error}",
                            member.id, site, variable
                        ),
                    ),
                }
            }
            let mut p05 = Vec::with_capacity(baseline_time.len());
            let mut p50 = Vec::with_capacity(baseline_time.len());
            let mut p95 = Vec::with_capacity(baseline_time.len());
            let mut n_eff = Vec::with_capacity(baseline_time.len());
            let mut stable = Vec::with_capacity(baseline_time.len());
            let required = required_ensemble_support(ensemble.len());
            for index in 0..baseline_time.len() {
                let values = ensemble
                    .iter()
                    .filter_map(|series| series.get(index).copied())
                    .filter(|value| value.is_finite())
                    .collect::<Vec<_>>();
                n_eff.push(values.len());
                let enough = values.len() >= required;
                stable.push(enough);
                p05.push(if enough {
                    super::science::type7_quantile(values.clone(), 0.05)?
                } else {
                    None
                });
                p50.push(if enough {
                    super::science::type7_quantile(values.clone(), 0.50)?
                } else {
                    None
                });
                p95.push(if enough {
                    super::science::type7_quantile(values, 0.95)?
                } else {
                    None
                });
            }
            let unstable = stable.iter().filter(|&&value| !value).count();
            if unstable > 0 {
                push_warning_once(
                    state,
                    format!(
                        "{site} / {variable}: {unstable} timestamps have insufficient ensemble support (need at least {required})"
                    ),
                );
            }
            // ponytail: exact reduction keeps all timestamps; downsample the small
            // aggregate only if real Studies show JSON transfer is the bottleneck.
            write_json(
                &root
                    .join("envelopes")
                    .join(site)
                    .join(format!("{variable}.json")),
                &serde_json::json!({
                    "site":site,
                    "variable":variable,
                    "units":units,
                    "time":baseline_time,
                    "baseline":finite_options(&baseline_values),
                    "p05":p05,
                    "p50":p50,
                    "p95":p95,
                    "n_eff":n_eff,
                    "stable":stable,
                    "members":ensemble.len(),
                    "interpretation":"finite-sample scenario quantiles, not a confidence interval"
                }),
            )?;
        }
    }
    write_importance(manifest, members, &means, state)?;
    write_member_table(manifest, members, state, &means)?;
    Ok(())
}

fn write_importance(
    manifest: &Manifest,
    members: &[MemberPlan],
    means: &BTreeMap<(String, String, String), f64>,
    state: &StudyState,
) -> Result<()> {
    let mut rows = Vec::new();
    for site in &manifest.spec.base_cases {
        for variable in &manifest.spec.outputs {
            let y = |member: &str| {
                means
                    .get(&(member.to_string(), site.clone(), variable.clone()))
                    .copied()
            };
            importance_rows(
                manifest,
                members,
                &y,
                serde_json::json!({"site":site,"variable":variable}),
                &mut rows,
            )?;
        }
    }
    // 对目标函数的敏感性：一个成员一个值（各站、各目标已按冻结权重合并），所以不分站点。
    // 输出均值的敏感性只说明参数会不会让输出变；要挑率定参数，看的是它能不能让拟合变好。
    if !manifest.spec.targets.is_empty() {
        let y = |member: &str| {
            state
                .candidates
                .get(member)
                .filter(|candidate| candidate.feasible)
                .and_then(|candidate| candidate.calibration)
        };
        importance_rows(
            manifest,
            members,
            &y,
            serde_json::json!({"site":null,"variable":"objective","target":"objective"}),
            &mut rows,
        )?;
    }
    write_json(
        &Path::new(&manifest.root).join("results/importance.json"),
        &rows,
    )
}

/// 一个响应量（某站某输出的均值，或目标函数）对每个参数的敏感性，按研究方法选算法。
fn importance_rows(
    manifest: &Manifest,
    members: &[MemberPlan],
    y: &dyn Fn(&str) -> Option<f64>,
    label: serde_json::Value,
    rows: &mut Vec<serde_json::Value>,
) -> Result<()> {
    let mut push = |extra: serde_json::Value| {
        let mut row = label.clone();
        if let (Some(row), Some(extra)) = (row.as_object_mut(), extra.as_object()) {
            row.extend(extra.clone());
        }
        rows.push(row);
    };
    let morris =
        (manifest.spec.method == StudyMethod::Morris).then(|| morris_effects(manifest, members, y));
    for parameter in &manifest.spec.parameters {
        let parameter_key = parameter.member_key();
        match manifest.spec.method {
            StudyMethod::Lhs => {
                let (x, values): (Vec<_>, Vec<_>) = members
                    .iter()
                    .filter(|member| !member.baseline)
                    .filter_map(|member| {
                        Some((*member.parameters.get(&parameter_key)?, y(&member.id)?))
                    })
                    .unzip();
                push(serde_json::json!({
                    "parameter":parameter_key,"method":"spearman",
                    "value":super::science::spearman(&x,&values),"n":x.len()
                }));
            }
            StudyMethod::Oat => {
                let baseline = members
                    .iter()
                    .find(|member| member.baseline)
                    .context("OAT Study has no baseline member")?;
                let parameter_index = super::sample::sorted_parameter_names(&manifest.spec)
                    .iter()
                    .position(|name| name == &parameter_key)
                    .context("OAT parameter is missing from the frozen design")?;
                let candidate_indices = [2 * parameter_index + 1, 2 * parameter_index + 2];
                let mut changed = members
                    .iter()
                    .filter(|member| candidate_indices.contains(&member.candidate_index))
                    .collect::<Vec<_>>();
                if changed.len() == 2 {
                    changed.sort_by(|a, b| {
                        a.parameters[&parameter_key].total_cmp(&b.parameters[&parameter_key])
                    });
                    let (low, high) = (changed[0], changed[1]);
                    if let (Some(y_low), Some(y_high), Some(y0)) =
                        (y(&low.id), y(&high.id), y(&baseline.id))
                    {
                        let dx = high.parameters[&parameter_key] - low.parameters[&parameter_key];
                        push(serde_json::json!({
                            "parameter":parameter_key,"method":"oat_finite_difference_slope",
                            "value":(y_high-y_low)/dx,"asymmetry":(y_high-y0)-(y0-y_low),"n":2
                        }));
                    }
                }
            }
            StudyMethod::Morris => {
                let effects = morris
                    .as_ref()
                    .and_then(|effects| effects.get(&parameter_key))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let (mu_star, mu, sigma) = super::science::morris_summary(effects);
                push(serde_json::json!({
                    "parameter":parameter_key,"method":"morris",
                    "value":mu_star,"mu":mu,"sigma":sigma,"n":effects.len()
                }));
            }
            StudyMethod::DifferentialEvolution => {}
        }
    }
    Ok(())
}

/// 每个参数在各条 Morris 轨迹上的基本效应 EE = Δy / Δu（u 是归一化位置，对数参数取对数尺度）。
/// 相邻两点恰好只有一个参数不同；对不上（不该发生）或任一端没有响应值的那一步跳过。
fn morris_effects(
    manifest: &Manifest,
    members: &[MemberPlan],
    y: &dyn Fn(&str) -> Option<f64>,
) -> BTreeMap<String, Vec<f64>> {
    let dims = super::sample::dimensions(&manifest.spec);
    let mut ordered = members
        .iter()
        .filter(|member| !member.baseline)
        .collect::<Vec<_>>();
    ordered.sort_by_key(|member| member.candidate_index);
    let mut effects: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for trajectory in ordered.chunks(dims.len() + 1) {
        if trajectory.len() != dims.len() + 1 {
            continue;
        }
        for step in trajectory.windows(2) {
            let (a, b) = (step[0], step[1]);
            let changed = dims
                .iter()
                .filter(|dim| a.parameters.get(&dim.key) != b.parameters.get(&dim.key))
                .collect::<Vec<_>>();
            let [dim] = changed.as_slice() else {
                continue;
            };
            let position = |member: &MemberPlan| {
                member
                    .parameters
                    .get(&dim.key)
                    .and_then(|&value| super::generation::unit_position(value, dim))
            };
            if let (Some(ua), Some(ub), Some(ya), Some(yb)) =
                (position(a), position(b), y(&a.id), y(&b.id))
            {
                let effect = (yb - ya) / (ub - ua);
                if effect.is_finite() {
                    effects.entry(dim.key.clone()).or_default().push(effect);
                }
            }
        }
    }
    effects
}

fn write_member_table(
    manifest: &Manifest,
    members: &[MemberPlan],
    state: &StudyState,
    means: &BTreeMap<(String, String, String), f64>,
) -> Result<()> {
    let mut csv = String::from("member,generation,baseline,site,status,reason");
    let names = super::sample::sorted_parameter_names(&manifest.spec);
    for name in &names {
        csv.push(',');
        csv.push_str(name);
    }
    for variable in &manifest.spec.outputs {
        csv.push(',');
        csv.push_str(variable);
        csv.push_str("_mean");
    }
    let scored = !manifest.spec.targets.is_empty();
    if scored {
        csv.push_str(",objective");
    }
    csv.push('\n');
    for member in members {
        for site in &manifest.spec.base_cases {
            let task = state.tasks.get(&super::state::task_id(&member.id, site));
            csv.push_str(&format!(
                "{},{},{},{},{},{}",
                member.id,
                member.generation,
                member.baseline,
                csv_cell(site),
                task.map(|task| format!("{:?}", task.status).to_ascii_lowercase())
                    .unwrap_or_else(|| "missing".into()),
                csv_cell(task.and_then(|task| task.reason.as_deref()).unwrap_or(""))
            ));
            for name in &names {
                csv.push(',');
                if let Some(value) = member.parameters.get(name) {
                    csv.push_str(&value.to_string());
                }
            }
            for variable in &manifest.spec.outputs {
                csv.push(',');
                if let Some(value) = means.get(&(member.id.clone(), site.clone(), variable.clone()))
                {
                    csv.push_str(&value.to_string());
                }
            }
            if scored {
                csv.push(',');
                if let Some(value) = state
                    .candidates
                    .get(&member.id)
                    .and_then(|candidate| candidate.calibration)
                {
                    csv.push_str(&value.to_string());
                }
            }
            csv.push('\n');
        }
    }
    write_bytes(
        &Path::new(&manifest.root).join("results/members.csv"),
        csv.as_bytes(),
    )
}

fn write_objective_tables(manifest: &Manifest, state: &StudyState) -> Result<()> {
    write_objective_tables_to(manifest, state, &Path::new(&manifest.root).join("results"))
}

/// 近优集合的门槛：目标函数不超过最优值的 (1 + NEAR_OPTIMAL_MARGIN) 倍。
const NEAR_OPTIMAL_MARGIN: f64 = 0.05;

/// 目标函数在最优值 5% 以内的候选，各参数取值占抽样范围的多宽。只看最优一组会以为参数被定住了；
/// 近优的那些散满整个范围，说明这份观测定不住它（异参同效）。网络权重不是物理参数，不列。
fn near_optimal(
    manifest: &Manifest,
    members: &[MemberPlan],
    state: &StudyState,
) -> Option<serde_json::Value> {
    let score = |member: &MemberPlan| {
        state
            .candidates
            .get(&member.id)
            .filter(|candidate| candidate.feasible && !member.baseline)
            .and_then(|candidate| candidate.calibration)
    };
    let best = members.iter().filter_map(score).min_by(f64::total_cmp)?;
    let threshold = best + best.abs() * NEAR_OPTIMAL_MARGIN;
    let near = members
        .iter()
        .filter(|member| score(member).is_some_and(|value| value <= threshold))
        .collect::<Vec<_>>();
    let parameters = super::sample::dimensions(&manifest.spec)
        .into_iter()
        .filter(|dim| !dim.key.starts_with(super::hybrid::WEIGHT_PREFIX))
        .filter_map(|dim| {
            let values = near
                .iter()
                .filter_map(|member| member.parameters.get(&dim.key).copied())
                .collect::<Vec<_>>();
            let positions = values
                .iter()
                .filter_map(|&value| super::generation::unit_position(value, &dim))
                .collect::<Vec<_>>();
            let min = values.iter().copied().reduce(f64::min)?;
            let max = values.iter().copied().reduce(f64::max)?;
            let spread = positions.iter().copied().reduce(f64::max)?
                - positions.iter().copied().reduce(f64::min)?;
            Some(serde_json::json!({
                "parameter": dim.key,
                "min": min,
                "max": max,
                "sample_min": dim.min,
                "sample_max": dim.max,
                "spread": spread,
            }))
        })
        .collect::<Vec<_>>();
    Some(serde_json::json!({
        "best": best,
        "threshold": threshold,
        "margin": NEAR_OPTIMAL_MARGIN,
        "members": near.iter().map(|member| member.id.clone()).collect::<Vec<_>>(),
        "parameters": parameters,
    }))
}

/// 多站点共享参数的留一站交叉验证：每次拿掉一个站，只按其余站的成绩挑最优成员，再看它在拿掉的那个站上
/// 是否仍比基准好。全站一起挑出的最优可能只是在拟合个别站；这一步回答"换到没参与率定的站还灵不灵"。
/// 用的是已有的成员结果，不额外运行模型。
fn leave_one_site_out(manifest: &Manifest, state: &StudyState) -> Option<serde_json::Value> {
    let sites = &manifest.spec.base_cases;
    if sites.len() < 2 {
        return None;
    }
    let rows = |member: &str, site: &str| {
        read_task_result(manifest, member, site)
            .ok()
            .map(|result| result.calibration)
    };
    let members = state
        .candidates
        .iter()
        .filter(|(id, candidate)| id.as_str() != "m000000" && candidate.feasible)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let mut folds = Vec::new();
    for held in sites {
        let chosen = members
            .iter()
            .filter_map(|member| {
                let mut kept = Vec::new();
                for site in sites.iter().filter(|site| *site != held) {
                    kept.extend(rows(member, site)?);
                }
                fixed_score(&kept).map(|score| (member, score))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((chosen, trained)) = chosen else {
            continue;
        };
        let held_out = rows(chosen, held).and_then(|rows| fixed_score(&rows));
        let baseline = rows("m000000", held).and_then(|rows| fixed_score(&rows));
        folds.push(serde_json::json!({
            "site": held,
            "chosen": chosen,
            "trained": trained,
            "held_out": held_out,
            "baseline": baseline,
        }));
    }
    (!folds.is_empty()).then(|| serde_json::json!({ "folds": folds }))
}

pub(super) fn write_objective_tables_to(
    manifest: &Manifest,
    state: &StudyState,
    results: &Path,
) -> Result<()> {
    write_json(&results.join("objectives.json"), &state.candidates)?;
    // 近优集合是派生的辅助结果：读不到样本文件就不写它，不连累成绩表本身。
    if manifest.spec.kind == StudyKind::Tuning {
        if let Some(near) = super::generation::load_members(manifest)
            .ok()
            .and_then(|members| near_optimal(manifest, &members, state))
        {
            write_json(&results.join("near_optimal.json"), &near)?;
        }
        if let Some(cross) = leave_one_site_out(manifest, state) {
            write_json(&results.join("cross_validation.json"), &cross)?;
        }
    }
    let mut objectives = String::from("member,generation,feasible,calibration,validation,reason\n");
    for (member, candidate) in &state.candidates {
        objectives.push_str(&format!(
            "{},{},{},{},{},{}\n",
            member,
            candidate.generation,
            candidate.feasible,
            candidate
                .calibration
                .map(|value| value.to_string())
                .unwrap_or_default(),
            candidate
                .validation
                .map(|value| value.to_string())
                .unwrap_or_default(),
            csv_cell(candidate.reason.as_deref().unwrap_or(""))
        ));
    }
    write_bytes(&results.join("objectives.csv"), objectives.as_bytes())?;
    let mut metrics = String::from("member,site,period,target,variable,metric,weight,min_pairs,pairs,value,observation_sd,loss,model_mean,observation_mean\n");
    let mut metric_rows = Vec::new();
    for task in state
        .tasks
        .values()
        .filter(|task| task.status == TaskStatus::Succeeded)
    {
        let result = read_task_result(manifest, &task.member, &task.site).with_context(|| {
            format!(
                "cannot read succeeded Study task result for {} / {}",
                task.member, task.site
            )
        })?;
        for row in result.calibration.iter().chain(&result.validation) {
            metrics.push_str(&format!(
                "{},{},{},{},{},{:?},{},{},{},{},{},{},{},{}\n",
                csv_cell(&task.member),
                csv_cell(&row.site),
                csv_cell(&row.period),
                csv_cell(&row.key),
                csv_cell(&row.variable),
                row.metric,
                row.weight,
                row.min_pairs,
                row.pairs,
                row.value,
                row.observation_sd
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                row.loss,
                row.model_mean,
                row.observation_mean
            ));
            metric_rows.push(ObjectiveMetricRow {
                member: task.member.clone(),
                site: row.site.clone(),
                period: row.period.clone(),
                target: row.key.clone(),
                variable: row.variable.clone(),
                metric: row.metric,
                weight: row.weight,
                min_pairs: row.min_pairs,
                pairs: row.pairs,
                value: row.value,
                observation_sd: row.observation_sd,
                loss: row.loss,
                model_mean: row.model_mean,
                observation_mean: row.observation_mean,
                support_hash: row.support_hash.clone(),
            });
        }
    }
    write_bytes(&results.join("metrics.csv"), metrics.as_bytes())?;
    write_json(&results.join("metrics.json"), &metric_rows)
}

pub fn retry(study_dir: &Path, include_review: bool) -> Result<StudyState> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    if include_review {
        clear_stale_run_lock(&study_dir)?;
    }
    let _retry_lock = StudyRunLock::acquire(&study_dir)?;
    let manifest = super::engine::status(&study_dir)?;
    super::engine::ensure_supported_study_manifest(&manifest, None)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let checkpoint_dir = study_dir.join("checkpoints/state");
    let mut state = super::checkpoint::load_latest::<StudyState>(&checkpoint_dir)?
        .map(|loaded| loaded.payload)
        .with_context(|| format!("{} has no Study state checkpoint", study_dir.display()))?;
    if state.study_id != manifest.id {
        bail!(
            "Study checkpoint id {} does not match manifest {}",
            state.study_id,
            manifest.id
        );
    }
    let members = if manifest.spec.kind == StudyKind::Tuning
        && manifest.spec.method == StudyMethod::DifferentialEvolution
    {
        super::generation::load_members(&manifest)?
            .into_iter()
            .map(|member| (member.id.clone(), member))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let closed_generation = state.generation;
    let population = state.population.clone();
    let mut reopened = false;
    for task in state.tasks.values_mut() {
        if matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted)
            || (include_review
                && matches!(
                    task.status,
                    TaskStatus::NeedsReview | TaskStatus::Running | TaskStatus::Evaluating
                ))
        {
            if !retryable_task(
                &manifest,
                closed_generation,
                &population,
                &members,
                &task.member,
            ) {
                task.reason = Some(format!(
                    "cannot retry closed DE generation for {}; create a new Study to rerun it",
                    task.member
                ));
                continue;
            }
            task.status = TaskStatus::Queued;
            task.reason = None;
            task.stage = None;
            task.objective = None;
            task.validation_objective = None;
            task.process = None;
            reopened = true;
        }
    }
    if !reopened {
        bail!("Study has no retryable failed tasks; closed DE generations are immutable");
    }
    state.status = StudyStatus::Ready;
    super::state::resume(&study_dir)?;
    super::state::clear_cancel(&study_dir)?;
    super::checkpoint::write_next(&checkpoint_dir, &state)?;
    Ok(state)
}

fn retryable_task(
    manifest: &Manifest,
    closed_generation: usize,
    population: &[String],
    members: &BTreeMap<String, MemberPlan>,
    member: &str,
) -> bool {
    if manifest.spec.kind != StudyKind::Tuning
        || manifest.spec.method != StudyMethod::DifferentialEvolution
        || member == "m000000"
    {
        return true;
    }
    let Some(plan) = members.get(member) else {
        return false;
    };
    plan.generation > closed_generation
        || (plan.generation == 0
            && closed_generation == 0
            && (population.is_empty() || population.iter().any(|id| id == member)))
}

pub fn apply(
    study_dir: &Path,
    member_id: &str,
    output: &Path,
    name: Option<&str>,
) -> Result<Vec<PathBuf>> {
    if output.to_string_lossy().contains(' ') {
        bail!("case path cannot contain spaces: {}", output.display());
    }
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    clear_stale_run_lock(&study_dir)?;
    let _apply_lock = StudyRunLock::acquire(&study_dir)?;
    let manifest = super::engine::status(&study_dir)?;
    super::engine::ensure_supported_study_manifest(&manifest, None)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let member = resolve_apply_member(&study_dir, &manifest, member_id)?;
    let values = manifest.spec.member_namelist_values(&member.parameters);
    let output_name = output
        .file_name()
        .context("Study apply output must name a case or directory")?;
    let output_parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent)?;
    let output = colm_kernel::manifest::absolute(output_parent)?.join(output_name);
    if output.to_string_lossy().contains(' ') {
        bail!("case path cannot contain spaces: {}", output.display());
    }
    if output.exists() {
        if !output.is_dir() {
            bail!(
                "Study apply output is not a directory: {}",
                output.display()
            );
        }
        if output.read_dir()?.next().is_some() {
            bail!("refusing to overwrite non-empty {}", output.display());
        }
    }
    let destinations = manifest
        .spec
        .base_cases
        .iter()
        .map(|site| {
            if manifest.spec.base_cases.len() == 1 {
                output.clone()
            } else {
                output.join(site)
            }
        })
        .collect::<Vec<_>>();
    let stage = output_parent.join(format!(
        ".colm-study-apply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&stage)?;
    let stage = colm_kernel::manifest::absolute(&stage)?;
    let staged = manifest
        .spec
        .base_cases
        .iter()
        .map(|site| {
            if manifest.spec.base_cases.len() == 1 {
                stage.clone()
            } else {
                stage.join(site)
            }
        })
        .collect::<Vec<_>>();
    let prepared = (|| -> Result<()> {
        for ((site, destination), staged) in manifest
            .spec
            .base_cases
            .iter()
            .zip(&destinations)
            .zip(&staged)
        {
            let baseline = super::model::base_case(&manifest, site)?;
            let before = sha256(&fs::read(baseline.join("case.nml"))?);
            let case_name = if manifest.spec.base_cases.len() == 1 {
                name.map(str::to_string)
                    .unwrap_or_else(|| format!("{site}-{member_id}"))
            } else {
                format!("{site}-tuned")
            };
            colm_case::validate_case_name(&case_name)?;
            super::materialize::member_case(
                &baseline,
                staged,
                &case_name,
                if manifest.spec.base_cases.len() == 1 {
                    "calibrated"
                } else {
                    site
                },
                &values,
            )?;
            super::hybrid::write_member_files(staged, &manifest.spec, &member.parameters)?;
            let case_nml = staged.join("case.nml");
            let mut document = colm_namelist::parse(&fs::read_to_string(&case_nml)?)?;
            for field in [
                "DEF_forcing_namelist",
                "DEF_HIST_vars_namelist",
                "DEF_TRACER_PARAM_FILES",
            ] {
                let Some(Value::Str(raw)) = document.get(field).cloned() else {
                    continue;
                };
                let relocated = raw.replace(
                    staged.to_string_lossy().as_ref(),
                    destination.to_string_lossy().as_ref(),
                );
                if relocated != raw {
                    document.set(field, Value::Str(relocated))?;
                }
            }
            document.set("DEF_CASE_NAME", Value::Str(case_name))?;
            document.set(
                "DEF_dir_output",
                Value::Str(destination.join("out").to_string_lossy().into_owned()),
            )?;
            fs::write(&case_nml, document.to_string())?;
            // 这个算例从哪来：结果页的“这个算例的运行”据此写“率定参数 · 研究 … 成员 …”。
            fs::write(
                staged.join(crate::case_runs::DERIVED_FILE),
                serde_json::to_string_pretty(&serde_json::json!({
                    "study": manifest.id,
                    "study_dir": study_dir,
                    "kind": manifest.spec.kind,
                    "member": member.id,
                    "site": site,
                    "version": crate::study_version::pinned(&study_dir)?.map(|v| v.name),
                    "applied_unix": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or_default(),
                }))?,
            )?;
            let after = sha256(&fs::read(baseline.join("case.nml"))?);
            if before != after {
                bail!("baseline case changed while applying {member_id}");
            }
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    publish_applied_cases(&stage, &output)?;
    Ok(destinations)
}

fn publish_applied_cases(stage: &Path, output: &Path) -> Result<()> {
    let existed = output.exists();
    if existed {
        // Another study may have populated the destination during staging.
        if let Err(error) = fs::remove_dir(output) {
            let _ = fs::remove_dir_all(stage);
            return Err(error).with_context(|| format!("cannot publish {}", output.display()));
        }
    }
    if let Err(error) = fs::rename(stage, output) {
        if existed {
            let _ = fs::create_dir(output);
        }
        let _ = fs::remove_dir_all(stage);
        return Err(error).with_context(|| format!("cannot publish {}", output.display()));
    }
    Ok(())
}

pub fn apply_preview(study_dir: &Path, member_id: &str) -> Result<Vec<ApplyPreviewRow>> {
    ensure_scheduler_idle(study_dir)?;
    let manifest = super::engine::status(study_dir)?;
    super::engine::ensure_supported_study_manifest(&manifest, None)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let member = resolve_apply_member(study_dir, &manifest, member_id)?;
    let mut rows = Vec::new();
    for site in &manifest.spec.base_cases {
        let file = super::model::base_case(&manifest, site)?.join("case.nml");
        let document = colm_namelist::parse(&fs::read_to_string(&file)?)?;
        let weights = member
            .parameters
            .keys()
            .filter(|key| super::hybrid::is_weight_key(key))
            .count();
        if let (Some(hybrid), true) = (&manifest.spec.hybrid, weights > 0) {
            rows.push(ApplyPreviewRow {
                site: site.clone(),
                file: super::model::base_case(&manifest, site)?
                    .join("hybrid.toml")
                    .to_string_lossy()
                    .into_owned(),
                field: format!("hybrid slot {}", hybrid.slot),
                old: "<none>".into(),
                new: format!("trained network ({weights} weights)"),
            });
        }
        for (field, value) in member
            .parameters
            .iter()
            .filter(|(field, _)| !super::hybrid::is_weight_key(field))
        {
            rows.push(ApplyPreviewRow {
                site: site.clone(),
                file: file.to_string_lossy().into_owned(),
                field: field.clone(),
                old: document
                    .get(field)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<unset>".into()),
                new: format!("{value:.17e}"),
            });
        }
    }
    Ok(rows)
}

pub(crate) fn resolve_apply_member(
    study_dir: &Path,
    manifest: &Manifest,
    member_id: &str,
) -> Result<MemberPlan> {
    let mut state =
        super::checkpoint::load_latest::<StudyState>(&study_dir.join("checkpoints/state"))?
            .map(|loaded| loaded.payload);
    if let Some(state) = &state {
        if state.study_id != manifest.id {
            bail!(
                "Study checkpoint id {} does not match manifest {}",
                state.study_id,
                manifest.id
            );
        }
    }
    let members = super::generation::load_members(manifest)?;
    if let Some(state) = state.as_mut() {
        refresh_tuning_state(manifest, state)?;
    }
    let member_id = if member_id == "best" {
        state
            .as_ref()
            .and_then(|state| state.best_member.clone())
            .context("Study has no best member yet")?
    } else {
        member_id.to_string()
    };
    if manifest.spec.kind == StudyKind::Tuning {
        let state = state.context("Study has no state checkpoint")?;
        if !matches!(
            state.status,
            StudyStatus::Completed | StudyStatus::CompletedWithFailures
        ) {
            bail!("parameter tuning results can only be applied after the Study finishes");
        }
        let candidate = state
            .candidates
            .get(&member_id)
            .with_context(|| format!("Study member {member_id} has no objective result"))?;
        if !candidate.feasible {
            bail!(
                "Study member {member_id} is not feasible and cannot be applied: {}",
                candidate.reason.as_deref().unwrap_or("no objective result")
            );
        }
        for site in &manifest.spec.base_cases {
            let task = state
                .tasks
                .get(&super::state::task_id(&member_id, site))
                .with_context(|| format!("Study member {member_id} has no task for {site}"))?;
            if task.status != TaskStatus::Succeeded {
                bail!("Study member {member_id} did not succeed for {site}");
            }
        }
    }
    members
        .into_iter()
        .find(|member| member.id == member_id)
        .with_context(|| format!("unknown Study member {member_id}"))
}

pub(crate) fn study_case_root(manifest: &Manifest) -> Result<PathBuf> {
    Ok(Path::new(&manifest.root)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("Study directory is not under <case-root>/.colm/studies")?
        .to_path_buf())
}

fn resolved_base_cases(case_root: &Path, spec: &StudySpec) -> Result<Vec<(String, PathBuf)>> {
    super::engine::base_cases(case_root, spec)?
        .into_iter()
        .map(|candidate| {
            let site = candidate
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            Ok((site, candidate))
        })
        .collect()
}

fn observation_for(spec: &StudySpec, case_root: &Path, site: &str) -> Result<PathBuf> {
    let raw = spec
        .observations
        .get(site)
        .or_else(|| spec.observations.get("*"))
        .with_context(|| format!("missing observation path for {site}"))?;
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        case_root.join(path)
    };
    if !path.is_file() {
        bail!(
            "observation file for {site} does not exist: {}",
            path.display()
        );
    }
    Ok(path)
}

fn task_result_path(manifest: &Manifest, member: &str, site: &str) -> PathBuf {
    Path::new(&manifest.root)
        .join("results/tasks")
        .join(member)
        .join(format!("{site}.json"))
}

fn read_task_result(manifest: &Manifest, member: &str, site: &str) -> Result<TaskResult> {
    let path = task_result_path(manifest, member, site);
    let result: TaskResult = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("cannot parse {}", path.display()))?;
    validate_task_result(manifest, member, site, &result)
        .with_context(|| format!("invalid {}", path.display()))?;
    Ok(result)
}

fn model_series(
    case: &Path,
    variable: &str,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<(Vec<i64>, Vec<f64>, Option<String>)> {
    // 格点算例：每个时刻全区按陆地面积加权的平均。
    if let Some((time, values, units)) = super::targets::domain_mean_series(case, variable)? {
        let (time, values) = time
            .into_iter()
            .zip(values)
            .filter(|(time, _)| from.is_none_or(|from| *time >= from))
            .filter(|(time, _)| to.is_none_or(|to| *time < to))
            .unzip();
        return Ok((time, values, units));
    }
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let files = crate::history_files(&layout.out().join(&name))?;
    let mut data = crate::read_history_many(&files, &["time", variable])?;
    let minutes = data.remove("time").unwrap_or_default();
    let values = data.remove(variable).unwrap_or_default();
    if minutes.len() != values.len() {
        bail!("{variable} is not a scalar time series");
    }
    let unix = colm_hist::time::unix_seconds(&minutes);
    let selected = unix
        .iter()
        .enumerate()
        .filter(|(_, time)| from.is_none_or(|from| **time >= from))
        .filter(|(_, time)| to.is_none_or(|to| **time < to))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let units = files.iter().find_map(|path| {
        let file = netcdf::open(path).ok()?;
        file.variable(variable)?;
        crate::variable_units(&file, variable)
    });
    Ok((
        selected.iter().map(|&index| unix[index]).collect(),
        selected.iter().map(|&index| values[index]).collect(),
        units,
    ))
}

fn cancel_unstarted(state: &mut StudyState) {
    for task in state.tasks.values_mut() {
        if matches!(
            task.status,
            TaskStatus::Pending | TaskStatus::Materialized | TaskStatus::Queued
        ) {
            task.status = TaskStatus::Cancelled;
            task.reason = Some("cancelled before dispatch".into());
        }
    }
}

/// 最优候选的参数贴着抽样范围的边（归一化后离边不到 2%）：真正的最优很可能在范围外，
/// 该放宽范围重跑。网络权重不是物理参数，不报。
fn update_bound_warnings(
    manifest: &Manifest,
    members: &[MemberPlan],
    state: &mut StudyState,
) -> Result<()> {
    state
        .warnings
        .retain(|warning| !warning.starts_with("bound:"));
    let Some(best) = state
        .best_member
        .as_ref()
        .and_then(|id| members.iter().find(|member| &member.id == id))
    else {
        return Ok(());
    };
    for dim in super::sample::dimensions(&manifest.spec) {
        if dim.key.starts_with(super::hybrid::WEIGHT_PREFIX) {
            continue;
        }
        let Some(&value) = best.parameters.get(&dim.key) else {
            continue;
        };
        let side = match super::generation::unit_position(value, &dim) {
            Some(u) if u <= BOUND_MARGIN => "lower",
            Some(u) if u >= 1.0 - BOUND_MARGIN => "upper",
            _ => continue,
        };
        state
            .warnings
            .push(format!("bound: {} {side} {}", dim.key, value));
    }
    Ok(())
}

const BOUND_MARGIN: f64 = 0.02;

fn update_overfit_warning(state: &mut StudyState) {
    state
        .warnings
        .retain(|warning| !warning.starts_with("overfitting:"));
    let Some(best) = state
        .best_member
        .as_ref()
        .and_then(|member| state.candidates.get(member))
    else {
        return;
    };
    let Some(baseline) = state.candidates.get("m000000") else {
        return;
    };
    if best
        .calibration
        .zip(baseline.calibration)
        .is_some_and(|(best, base)| best < base)
        && best
            .validation
            .zip(baseline.validation)
            .is_some_and(|(best, base)| best > base)
    {
        state
            .warnings
            .push("overfitting: calibration improved while validation became worse".into());
    }
}

fn finite_options(values: &[f64]) -> Vec<Option<f64>> {
    values
        .iter()
        .map(|value| value.is_finite().then_some(*value))
        .collect()
}

fn required_ensemble_support(successful_members: usize) -> usize {
    20usize.max(successful_members.saturating_mul(4).div_ceil(5))
}

fn collect_result_files(root: &Path, current: &Path, out: &mut Vec<ResultFile>) -> Result<()> {
    if !current.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_result_files(root, &path, out)?;
        } else if kind.is_file() {
            out.push(ResultFile {
                path: path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                bytes: entry.metadata()?.len(),
            });
        }
    }
    Ok(())
}

pub fn event_log_tail(study_dir: &Path, limit: usize) -> Result<Vec<serde_json::Value>> {
    let path = study_dir.join("study.log");
    if !path.is_file() || limit == 0 {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)?;
    let mut rows = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .take(limit)
        .collect::<Vec<_>>();
    rows.reverse();
    Ok(rows)
}

fn emit(study_dir: &Path, enabled: bool, value: serde_json::Value) -> Result<()> {
    let line = serde_json::to_string(&value)?;
    // Raw CoLM output already lives in each member's stage log. Keeping only
    // scheduler events here makes restart/status reads bounded in practice.
    if value.get("kind").and_then(serde_json::Value::as_str) != Some("task_log") {
        if let Ok(mut log) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(study_dir.join("study.log"))
        {
            let _ = writeln!(log, "{line}");
        }
    }
    if !enabled {
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}")?;
    out.flush()?;
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_bytes(path, &serde_json::to_vec_pretty(value)?)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes).map_err(Into::into)
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into()
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn supervisor_identity() -> ProcessIdentity {
    let now = unix_now();
    let executable = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let argv = std::env::args().collect::<Vec<_>>().join("\0");
    ProcessIdentity {
        pid: std::process::id(),
        executable,
        argv_sha256: sha256(argv.as_bytes()),
        started_unix: now,
        heartbeat_unix: now,
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Export an evaluated member as a model revision without editing its source cases.
/// 要把一个成员的参数带出研究（存成新版本，或并进一个修改版草稿）之前的同一组校验；锁要拿到用完为止。
pub(crate) fn verified_member(
    study_dir: &Path,
    member_id: &str,
) -> Result<(StudyRunLock, Manifest, MemberPlan)> {
    clear_stale_run_lock(study_dir)?;
    let lock = StudyRunLock::acquire(study_dir)?;
    let manifest = super::engine::status(study_dir)?;
    super::engine::ensure_supported_study_manifest(&manifest, None)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let member = resolve_apply_member(study_dir, &manifest, member_id)?;
    let evidence_state = status_state(study_dir)?.context("Study has no evaluated member state")?;
    verify_member_evidence(&manifest, &evidence_state, &member.id)?;
    Ok((lock, manifest, member))
}

pub(crate) fn save_version(
    study_dir: &Path,
    member_id: &str,
    opts: &crate::Opts,
) -> Result<colm_model::FrozenRevision> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let (_lock, manifest, member) = verified_member(&study_dir, member_id)?;
    let input_identity = super::model::verified_inputs(&manifest)?;
    let store = crate::model_version::registry(opts)?;
    let pin = crate::study_version::pinned(&study_dir)?;
    if pin.as_ref().is_some_and(|p| p.model_revision.is_none()) {
        bail!("legacy workspace Study has no frozen model revision; create a Study with a model revision before saving a model version");
    }
    let pinned = pin.and_then(|p| p.model_revision);
    let requested = crate::model_version::selected(opts)?;
    if let (Some(pinned), Some(requested)) = (&pinned, &requested) {
        if pinned.version != *requested {
            bail!("cannot export a Study against another model revision");
        }
    }
    let parent = match pinned {
        Some(revision) => revision,
        None => match requested {
            Some(version) => store.get(&version)?,
            None => store.install_official(crate::model_version::code(opts, None)?)?,
        },
    };
    let first = manifest
        .spec
        .base_cases
        .first()
        .context("Study has no base cases")?;
    let staging = store.root().join(format!(
        ".study-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let values = manifest.spec.member_namelist_values(&member.parameters);
    let result = (|| -> Result<colm_model::FrozenRevision> {
        super::materialize::member_case(
            &super::model::base_case(&manifest, first)?,
            &staging,
            "model-export",
            "calibrated",
            &values,
        )?;
        super::hybrid::write_member_files(&staging, &manifest.spec, &member.parameters)?;
        let binding = store.migrate_case(&staging, parent.code.clone())?;
        let migrated = store.get(&binding.version)?;
        let id = format!(
            "study-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let metrics =
            status_state(&study_dir)?.and_then(|state| state.candidates.get(&member.id).cloned());
        let note = serde_json::json!({
            "note": opts.get("--note").unwrap_or_default(),
            "study": study_dir, "member": member.id,
            "sites": manifest.spec.base_cases, "targets": manifest.spec.targets,
            "metrics": metrics,
            "input_provenance": {
                "algorithm": input_identity.base_cases.iter().map(|(site, identity)| (site, &identity.algorithm)).collect::<BTreeMap<_, _>>(),
                "coverage": "non-forcing-inputs-and-observations",
                "forcing_contents_verified": false,
                "manifest": study_dir.join(super::model::IDENTITY_FILE),
                "manifest_sha256": crate::fingerprint::full_file(&study_dir.join(super::model::IDENTITY_FILE), "provenance")?.sha256,
                "data_sha256": input_identity.base_cases.iter().map(|(site, identity)| (site, &identity.data_sha256)).collect::<BTreeMap<_, _>>(),
                "observation_sha256": input_identity.observations.iter().map(|(site, file)| (site, &file.sha256)).collect::<BTreeMap<_, _>>(),
            },
        })
        .to_string();
        let calibration = manifest
            .spec
            .targets
            .iter()
            .map(|t| (t.from, t.to))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        let validation = manifest
            .spec
            .targets
            .iter()
            .filter_map(|t| Some((t.validation_from?, t.validation_to?)))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        let period = |p: (i64, i64)| colm_model::Period {
            start: crate::unix_date(p.0),
            end: crate::unix_date(p.1),
        };
        store.create_draft(colm_model::ModelDraft {
            id: id.clone(),
            name: opts.need_str("--name")?,
            note,
            parent: parent.version,
            code: parent.code,
            overlay: migrated.overlay,
            applicability: colm_model::Applicability {
                sites: manifest.spec.base_cases.clone(),
                calibration: calibration.map(period),
                validation: validation.map(period),
                ..Default::default()
            },
            generation: 0,
            created_unix: 0,
            derived_from: Some(format!(
                "study:{}/member:{}",
                study_dir.display(),
                member.id
            )),
        })?;
        store.freeze(
            &id,
            &store
                .root()
                .join("versions")
                .join(&binding.version.id)
                .join(binding.version.revision.to_string()),
        )
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinned_studys_own_kernel_copy_is_accepted_but_other_dirs_are_not() {
        let root = std::env::temp_dir().join(format!("colm-frozen-kernel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let study = root.join("study");
        let frozen = root.join("kernels/default");
        let copy = study.join("version/kernel");
        let other = root.join("elsewhere/default");
        for dir in [&frozen, &copy, &other] {
            fs::create_dir_all(dir).unwrap();
        }
        ensure_frozen_kernel_dir(&study, &frozen, &frozen).unwrap();
        // 没有固定版本时，副本路径同样不算。
        assert!(ensure_frozen_kernel_dir(&study, &frozen, &copy).is_err());
        fs::write(
            study.join(crate::study_version::FILE),
            serde_json::json!({
                "name": "official", "head": "h", "base_commit": "",
                "touches": {"engine": false, "kernel": false},
                "cli": study.join("version/bin/colm-cli"),
                "kernel": copy, "pinned_unix": 0
            })
            .to_string(),
        )
        .unwrap();
        ensure_frozen_kernel_dir(&study, &frozen, &copy).unwrap();
        let error = ensure_frozen_kernel_dir(&study, &frozen, &other).unwrap_err();
        assert!(format!("{error:#}").contains("Study was created for kernel"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn task_logs_are_sampled_before_entering_the_scheduler_channel() {
        let start = Instant::now();
        let mut last = None;
        assert!(should_emit_task_log(&mut last, start));
        assert!(!should_emit_task_log(
            &mut last,
            start + TASK_LOG_EMIT_INTERVAL / 2
        ));
        assert!(should_emit_task_log(
            &mut last,
            start + TASK_LOG_EMIT_INTERVAL
        ));
    }

    #[test]
    fn trial_selection_never_prefers_failed_candidate() {
        let mut state = StudyState::new("s".into(), []).unwrap();
        state.candidates.insert(
            "parent".into(),
            CandidateState {
                generation: 0,
                feasible: true,
                calibration: Some(1.0),
                validation: Some(1.0),
                reason: None,
            },
        );
        state.candidates.insert(
            "failed".into(),
            CandidateState {
                generation: 1,
                feasible: false,
                calibration: None,
                validation: None,
                reason: Some("failed".into()),
            },
        );
        state.candidates.insert(
            "better".into(),
            CandidateState {
                generation: 1,
                feasible: true,
                calibration: Some(0.5),
                validation: Some(0.7),
                reason: None,
            },
        );
        assert!(!trial_wins(&state, "parent", "failed"));
        assert!(trial_wins(&state, "parent", "better"));
    }

    #[test]
    fn trial_selection_tie_wins_but_patience_tie_is_not_improvement() {
        let mut state = StudyState::new("s".into(), []).unwrap();
        for member in ["parent", "trial"] {
            state.candidates.insert(
                member.into(),
                CandidateState {
                    generation: 0,
                    feasible: true,
                    calibration: Some(1.0),
                    validation: None,
                    reason: None,
                },
            );
        }
        assert!(trial_wins(&state, "parent", "trial"));
        assert!(!generation_improved(Some(1.0), Some(1.0), 0.0));
        assert!(generation_improved(Some(1.0), Some(0.99), 0.0));
        assert!(!generation_improved(Some(1.0), Some(0.99), 0.02));
        assert!(generation_improved(Some(1.0), Some(0.98), 0.02));
    }

    fn candidate(score: Option<f64>) -> CandidateState {
        CandidateState {
            generation: 0,
            feasible: score.is_some(),
            calibration: score,
            validation: None,
            reason: score.is_none().then(|| "failed".into()),
        }
    }

    fn member(id: &str, value: f64) -> MemberPlan {
        MemberPlan {
            id: id.into(),
            generation: 0,
            candidate_index: id[1..].parse().unwrap(),
            baseline: id == "m000000",
            parameters: BTreeMap::from([("DEF_TUNING_CNFAC".to_string(), value)]),
        }
    }

    fn manifest_with_range(min: f64, max: f64) -> Manifest {
        let mut manifest = tuning_manifest(Path::new("/unused"));
        manifest.spec.parameters = vec![super::super::spec::ParameterSpec {
            name: "DEF_TUNING_CNFAC".into(),
            parameter_id: None,
            scope_instance: None,
            sample_min: min,
            sample_max: max,
            scale: Some(super::super::spec::ScaleSpec::Linear),
        }];
        manifest
    }

    #[test]
    fn morris_effects_recover_linear_slopes_in_normalized_units() {
        let mut manifest = manifest_with_range(0.1, 0.9);
        manifest.spec.kind = StudyKind::Uncertainty;
        manifest.spec.method = StudyMethod::Morris;
        manifest.spec.targets.clear();
        let mut second = manifest.spec.parameters[0].clone();
        second.name = "DEF_TUNING_ZLND".into();
        second.sample_min = 0.001;
        second.sample_max = 0.1;
        second.scale = Some(super::super::spec::ScaleSpec::Log);
        manifest.spec.parameters.push(second);
        manifest.spec.budget.candidate_count = Some(4);
        let baseline = BTreeMap::from([
            ("DEF_TUNING_CNFAC".to_string(), 0.5),
            ("DEF_TUNING_ZLND".to_string(), 0.01),
        ]);
        let members = super::super::sample::design(&manifest.spec, &baseline).unwrap();
        // y = 2·CNFAC + 0·ZLND：在归一化尺度上 CNFAC 的基本效应恒为 2 × 0.8 = 1.6，ZLND 恒为 0。
        let y = |id: &str| {
            members
                .iter()
                .find(|member| member.id == id)
                .map(|member| 2.0 * member.parameters["DEF_TUNING_CNFAC"])
        };
        let effects = morris_effects(&manifest, &members, &y);
        assert_eq!(effects["DEF_TUNING_CNFAC"].len(), 4);
        assert!(effects["DEF_TUNING_CNFAC"]
            .iter()
            .all(|effect| (effect - 1.6).abs() < 1e-9));
        assert!(effects["DEF_TUNING_ZLND"]
            .iter()
            .all(|effect| effect.abs() < 1e-9));
        let mut rows = Vec::new();
        importance_rows(
            &manifest,
            &members,
            &y,
            serde_json::json!({"site":null,"variable":"objective","target":"objective"}),
            &mut rows,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["variable"], "objective");
        assert_eq!(rows[0]["method"], "morris");
        assert_eq!(rows[0]["n"], 4);
        // 缺了一个成员的响应值：只少那一步，不让整个参数没结果。
        let missing = members[2].id.clone();
        let partial = |id: &str| if id == missing { None } else { y(id) };
        let effects = morris_effects(&manifest, &members, &partial);
        assert!(effects.values().map(Vec::len).sum::<usize>() < 8);
    }

    #[test]
    fn baseline_replaces_the_worst_initial_member_only_when_better_and_inside_the_box() {
        let members = vec![
            member("m000000", 0.5),
            member("m000001", 0.2),
            member("m000002", 0.8),
            member("m000003", 0.3),
        ];
        let seeded = |baseline: Option<f64>, scores: [Option<f64>; 3], range: (f64, f64)| {
            let manifest = manifest_with_range(range.0, range.1);
            let mut state = StudyState::new("s".into(), []).unwrap();
            state.population = vec!["m000001".into(), "m000002".into(), "m000003".into()];
            state
                .candidates
                .insert("m000000".into(), candidate(baseline));
            for (id, score) in ["m000001", "m000002", "m000003"].iter().zip(scores) {
                state.candidates.insert((*id).into(), candidate(score));
            }
            seed_population_with_baseline(&manifest, &members, &mut state).unwrap();
            state.population
        };
        // 最差的是 m000002，基准更好就顶替它。
        assert_eq!(
            seeded(Some(0.5), [Some(0.4), Some(0.9), Some(0.6)], (0.1, 0.9)),
            ["m000001", "m000000", "m000003"]
        );
        // 不可行的初代成员算最差；同为不可行时取靠后的。
        assert_eq!(
            seeded(Some(0.5), [None, Some(0.4), None], (0.1, 0.9)),
            ["m000001", "m000002", "m000000"]
        );
        // 基准不比最差的好、基准不可行、基准在抽样范围外：都不动。
        let untouched = ["m000001", "m000002", "m000003"];
        assert_eq!(
            seeded(Some(0.9), [Some(0.4), Some(0.9), Some(0.6)], (0.1, 0.9)),
            untouched
        );
        assert_eq!(
            seeded(None, [Some(0.4), Some(0.9), Some(0.6)], (0.1, 0.9)),
            untouched
        );
        assert_eq!(
            seeded(Some(0.1), [Some(0.4), Some(0.9), Some(0.6)], (0.6, 0.9)),
            untouched
        );
    }

    #[test]
    fn leave_one_site_out_picks_on_the_other_sites_and_scores_the_held_out_one() {
        let root = std::env::temp_dir().join(format!("colm-loso-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut manifest = tuning_manifest(&root);
        manifest.spec.base_cases = vec!["a".into(), "b".into()];
        manifest.spec.targets[0].validation_from = None;
        manifest.spec.targets[0].validation_to = None;
        // value / obs_sd(2.0) 就是损失。m000001 只在站 a 好、在站 b 差；m000002 两站都比基准好一点。
        let losses = [
            ("m000000", "a", 0.8),
            ("m000000", "b", 0.8),
            ("m000001", "a", 0.2),
            ("m000001", "b", 1.6),
            ("m000002", "a", 0.6),
            ("m000002", "b", 0.6),
        ];
        let mut state = StudyState::new("s".into(), []).unwrap();
        for (member, site, loss) in losses {
            let mut row = target_row("calibration", vec![1, 2]);
            row.site = site.into();
            row.weight = 0.5;
            row.value = loss * 2.0;
            row.loss = loss;
            let result = TaskResult {
                member: member.into(),
                site: site.into(),
                outputs: Vec::new(),
                calibration: vec![row],
                validation: Vec::new(),
                validation_errors: Vec::new(),
            };
            let path = task_result_path(&manifest, member, site);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
            state
                .candidates
                .insert(member.into(), candidate(Some(loss)));
        }
        let cross = leave_one_site_out(&manifest, &state).unwrap();
        let folds = cross["folds"].as_array().unwrap();
        // 拿掉 b：只看 a 会挑 m000001，可它在 b 上 1.6，比基准 0.8 差 —— 这就是交叉验证要揭出来的。
        let held_b = folds.iter().find(|f| f["site"] == "b").unwrap();
        assert_eq!(held_b["chosen"], "m000001");
        assert_eq!(held_b["held_out"], 1.6);
        assert_eq!(held_b["baseline"], 0.8);
        let held_a = folds.iter().find(|f| f["site"] == "a").unwrap();
        assert_eq!(held_a["chosen"], "m000002");
        assert!(held_a["held_out"].as_f64().unwrap() < held_a["baseline"].as_f64().unwrap());
        manifest.spec.base_cases.truncate(1);
        assert!(leave_one_site_out(&manifest, &state).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn near_optimal_members_show_how_wide_each_parameter_stays_unpinned() {
        let manifest = manifest_with_range(0.1, 0.9);
        let mut state = StudyState::new("s".into(), []).unwrap();
        let members = vec![
            member("m000000", 0.5),
            member("m000001", 0.2),
            member("m000002", 0.8),
            member("m000003", 0.5),
            member("m000004", 0.9),
        ];
        // 基准再好也不算；0.104 在最优 0.1 的 5% 以内，0.2 不在；不可行的不算。
        state
            .candidates
            .insert("m000000".into(), candidate(Some(0.01)));
        state
            .candidates
            .insert("m000001".into(), candidate(Some(0.1)));
        state
            .candidates
            .insert("m000002".into(), candidate(Some(0.104)));
        state
            .candidates
            .insert("m000003".into(), candidate(Some(0.2)));
        state.candidates.insert("m000004".into(), candidate(None));
        let near = near_optimal(&manifest, &members, &state).unwrap();
        assert_eq!(near["best"], 0.1);
        assert_eq!(near["members"], serde_json::json!(["m000001", "m000002"]));
        let spread = near["parameters"][0]["spread"].as_f64().unwrap();
        assert!((spread - 0.75).abs() < 1e-12, "{spread}");
        assert!(near_optimal(&manifest, &members[..1], &state).is_none());
    }

    #[test]
    fn best_member_on_a_sampling_bound_is_warned_and_cleared_when_it_moves_inside() {
        let manifest = manifest_with_range(0.1, 0.9);
        let mut state = StudyState::new("s".into(), []).unwrap();
        state.best_member = Some("m000001".into());
        let members = vec![member("m000001", 0.895)];
        update_bound_warnings(&manifest, &members, &mut state).unwrap();
        assert_eq!(state.warnings, ["bound: DEF_TUNING_CNFAC upper 0.895"]);
        let members = vec![member("m000001", 0.5)];
        update_bound_warnings(&manifest, &members, &mut state).unwrap();
        assert!(state.warnings.is_empty());
        let members = vec![member("m000001", 0.1)];
        update_bound_warnings(&manifest, &members, &mut state).unwrap();
        assert_eq!(state.warnings, ["bound: DEF_TUNING_CNFAC lower 0.1"]);
    }

    fn tuning_manifest(root: &Path) -> Manifest {
        Manifest {
            schema_version: 1,
            id: "s".into(),
            root: root.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Tuning,
                method: StudyMethod::DifferentialEvolution,
                seed: 0,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: BTreeMap::new(),
                site_mode: super::super::spec::SiteMode::Shared,
                parameters: Vec::new(),
                outputs: Vec::new(),
                analysis_from: None,
                analysis_to: None,
                targets: vec![TargetSpec {
                    key: "Qle".into(),
                    site: None,
                    variable: "Qle".into(),
                    metric: ObjectiveMetric::Nrmse,
                    weight: 1.0,
                    from: 0,
                    to: 10,
                    validation_from: Some(10),
                    validation_to: Some(20),
                    min_pairs: 2,
                    source: None,
                    corrected: false,
                    aggregate: None,
                }],
                budget: Default::default(),
                hybrid: None,
            },
            members: Vec::new(),
            provenance: Default::default(),
        }
    }

    fn target_row(period: &str, support: Vec<i64>) -> TargetResult {
        target_row_with_obs(
            period,
            support.clone(),
            support.iter().map(|v| *v as f64).collect(),
        )
    }

    fn target_row_with_obs(period: &str, support: Vec<i64>, obs: Vec<f64>) -> TargetResult {
        TargetResult {
            key: "Qle".into(),
            site: "site".into(),
            variable: "Qle".into(),
            period: period.into(),
            metric: ObjectiveMetric::Nrmse,
            weight: 1.0,
            min_pairs: 2,
            pairs: support.len(),
            value: 0.4,
            observation_sd: Some(2.0),
            loss: 0.2,
            model_mean: 5.0,
            observation_mean: 4.5,
            support_hash: pair_support_hash(&support, &obs),
        }
    }

    #[test]
    fn fixed_score_is_invariant_to_large_common_weight_scale() {
        let mut a = target_row("calibration", vec![1, 2]);
        a.weight = 1e308;
        a.loss = 2.0;
        let mut b = target_row("validation", vec![3, 4]);
        b.weight = 1e308;
        b.loss = 4.0;
        assert_eq!(fixed_score(&[a, b]), Some(3.0));

        let mut a = target_row("calibration", vec![1, 2]);
        a.loss = 1e308;
        let mut b = target_row("validation", vec![3, 4]);
        b.loss = 1e308;
        assert_eq!(fixed_score(&[a, b]), Some(1e308));
    }

    #[test]
    fn cached_tuning_results_must_match_frozen_target_set_and_identity() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-result-validate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manifest = tuning_manifest(&root);
        let mut ok = TaskResult {
            member: "m000001".into(),
            site: "site".into(),
            outputs: Vec::new(),
            calibration: vec![target_row("calibration", vec![1, 2])],
            validation: vec![target_row("validation", vec![3, 4])],
            validation_errors: Vec::new(),
        };
        validate_task_result(&manifest, "m000001", "site", &ok).unwrap();

        ok.site = "other".into();
        assert!(validate_task_result(&manifest, "m000001", "site", &ok)
            .unwrap_err()
            .to_string()
            .contains("identity"));
        ok.site = "site".into();
        ok.validation.clear();
        assert!(validate_task_result(&manifest, "m000001", "site", &ok)
            .unwrap_err()
            .to_string()
            .contains("missing required target"));
        ok.validation = vec![target_row("validation", vec![])];
        assert!(validate_task_result(&manifest, "m000001", "site", &ok)
            .unwrap_err()
            .to_string()
            .contains("missing pair support"));
        ok.validation = vec![target_row("calibration", vec![3, 4])];
        assert!(validate_task_result(&manifest, "m000001", "site", &ok)
            .unwrap_err()
            .to_string()
            .contains("stored in validation"));
        ok.validation = vec![target_row("validation", vec![3, 4])];
        ok.validation[0].loss = 123.0;
        assert!(validate_task_result(&manifest, "m000001", "site", &ok)
            .unwrap_err()
            .to_string()
            .contains("loss does not match"));
        ok.validation.clear();
        ok.validation_errors = vec![TargetError {
            key: "Qle".into(),
            site: "site".into(),
            variable: "Qle".into(),
            period: "validation".into(),
            reason: "no validation pairs".into(),
        }];
        validate_task_result(&manifest, "m000001", "site", &ok).unwrap();
    }

    #[test]
    fn candidate_support_must_match_baseline_support() {
        let baseline = TaskResult {
            member: "m000000".into(),
            site: "site".into(),
            outputs: Vec::new(),
            calibration: vec![target_row("calibration", vec![1, 2, 3])],
            validation: Vec::new(),
            validation_errors: Vec::new(),
        };
        let same_len_wrong_time = TaskResult {
            member: "m000001".into(),
            site: "site".into(),
            outputs: Vec::new(),
            calibration: vec![target_row("calibration", vec![1, 2, 4])],
            validation: Vec::new(),
            validation_errors: Vec::new(),
        };
        assert!(comparable_to_baseline(&same_len_wrong_time, &baseline)
            .unwrap_err()
            .to_string()
            .contains("different model-observation support"));
        let wrong_observation = TaskResult {
            member: "m000001".into(),
            site: "site".into(),
            outputs: Vec::new(),
            calibration: vec![target_row_with_obs(
                "calibration",
                vec![1, 2, 3],
                vec![1.0, 99.0, 3.0],
            )],
            validation: Vec::new(),
            validation_errors: Vec::new(),
        };
        assert!(comparable_to_baseline(&wrong_observation, &baseline)
            .unwrap_err()
            .to_string()
            .contains("different model-observation support"));
        let mut validation_mismatch = wrong_observation.clone();
        validation_mismatch.calibration = baseline.calibration.clone();
        validation_mismatch.validation = vec![target_row_with_obs(
            "validation",
            vec![1, 2, 3],
            vec![1.0, 99.0, 3.0],
        )];
        assert!(comparable_to_baseline(&validation_mismatch, &baseline).is_ok());
    }

    #[test]
    fn retry_leaves_closed_de_generation_failed() {
        let manifest = tuning_manifest(Path::new("/unused"));
        let mut members = BTreeMap::new();
        members.insert(
            "m000005".into(),
            MemberPlan {
                id: "m000005".into(),
                generation: 1,
                candidate_index: 5,
                baseline: false,
                parameters: BTreeMap::new(),
            },
        );
        assert!(!retryable_task(&manifest, 1, &[], &members, "m000005"));
        assert!(retryable_task(&manifest, 0, &[], &members, "m000005"));
        assert!(retryable_task(&manifest, 1, &[], &members, "m000000"));
        members.insert(
            "m000001".into(),
            MemberPlan {
                id: "m000001".into(),
                generation: 0,
                candidate_index: 1,
                baseline: false,
                parameters: BTreeMap::new(),
            },
        );
        assert!(!retryable_task(
            &manifest,
            1,
            &["m000001".into()],
            &members,
            "m000001"
        ));
    }

    #[test]
    fn unfinished_de_generation_never_crosses_the_selection_barrier() {
        let sites = vec!["caseA".to_string()];
        let trials = vec![MemberPlan {
            id: "m000005".into(),
            generation: 1,
            candidate_index: 5,
            baseline: false,
            parameters: BTreeMap::new(),
        }];
        let mut state = StudyState::new(
            "s".into(),
            [super::super::state::TaskState {
                member: "m000005".into(),
                site: "caseA".into(),
                case_dir: "/unused".into(),
                status: TaskStatus::Queued,
                stage: None,
                reason: None,
                objective: None,
                validation_objective: None,
                process: None,
            }],
        )
        .unwrap();
        assert!(!generation_finished(&sites, &state, &trials));
        state.tasks.get_mut("m000005/caseA").unwrap().status = TaskStatus::Failed;
        assert!(generation_finished(&sites, &state, &trials));
    }

    #[test]
    fn read_result_refuses_paths_outside_results() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-result-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("results")).unwrap();
        std::fs::write(dir.join("results/ok.json"), "{}").unwrap();
        std::fs::write(dir.join("secret.txt"), "secret").unwrap();

        assert_eq!(read_result(&dir, Path::new("ok.json")).unwrap(), "{}");
        assert!(read_result(&dir, Path::new("../secret.txt")).is_err());
        assert!(read_result(&dir, Path::new("/secret.txt")).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn read_result_refuses_a_symlink_escape() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "colm-study-result-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("results")).unwrap();
        std::fs::write(dir.join("secret.txt"), "secret").unwrap();
        symlink(dir.join("secret.txt"), dir.join("results/escape.txt")).unwrap();
        assert!(read_result(&dir, Path::new("escape.txt")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_succeeded_task_with_missing_outputs_is_not_current() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-current-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'case'\n   DEF_dir_output = 'out'\n/\n",
        )
        .unwrap();
        assert!(!crate::case_is_current(
            &dir,
            "test@kernel",
            &crate::RustPreprocessorIdentities::default()
        )
        .unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn write_history_nc(path: &Path, vars: &[(&str, &[f64])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = netcdf::create(path).unwrap();
        let n = vars
            .iter()
            .find(|(name, _)| *name == "time")
            .map(|(_, values)| values.len())
            .unwrap_or_else(|| vars.first().map(|(_, values)| values.len()).unwrap_or(0));
        file.add_dimension("time", n).unwrap();
        for (name, values) in vars {
            file.add_variable::<f64>(name, &["time"])
                .unwrap()
                .put_values(values, ..)
                .unwrap();
        }
    }

    fn minimal_uncertainty_manifest(root: &Path, outputs: Vec<String>) -> Manifest {
        Manifest {
            schema_version: 1,
            id: "s".into(),
            root: root.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Uncertainty,
                method: StudyMethod::Lhs,
                seed: 0,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: BTreeMap::new(),
                site_mode: super::super::spec::SiteMode::Shared,
                parameters: Vec::new(),
                outputs,
                analysis_from: None,
                analysis_to: None,
                targets: Vec::new(),
                budget: Default::default(),
                hybrid: None,
            },
            members: Vec::new(),
            provenance: Default::default(),
        }
    }

    fn write_status_manifest(dir: &Path, id: &str) {
        let mut manifest = minimal_uncertainty_manifest(dir, Vec::new());
        manifest.id = id.into();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn uncertainty_output_mean_does_not_overflow_after_history_read() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let root = std::env::temp_dir().join(format!(
            "colm-study-mean-overflow-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let case = root.join("case");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n/\n",
        )
        .unwrap();
        write_history_nc(
            &case.join("out/site/history/site_hist_2000-01.nc"),
            &[("time", &[0.0, 1800.0]), ("f_qle", &[1.0e308, 1.0e308])],
        );
        let manifest = minimal_uncertainty_manifest(&root, vec!["f_qle".into()]);

        let result = evaluate_task(&manifest, "m000001", "site", &case).unwrap();
        assert_eq!(result.outputs[0].count, 2);
        assert!(result.outputs[0].mean.is_finite());
        assert!((result.outputs[0].mean - 1.0e308).abs() < 1.0e292);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uncertainty_envelope_excludes_baseline_mismatched_time_and_low_support() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let root = std::env::temp_dir().join(format!(
            "colm-study-envelope-support-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study");
        let mut manifest = minimal_uncertainty_manifest(&study, vec!["f_qle".into()]);
        let mut members = vec![MemberPlan {
            id: "m000000".into(),
            generation: 0,
            candidate_index: 0,
            baseline: true,
            parameters: BTreeMap::new(),
        }];
        members.extend((1..=21).map(|index| MemberPlan {
            id: format!("m{index:06}"),
            generation: 0,
            candidate_index: index,
            baseline: false,
            parameters: BTreeMap::new(),
        }));
        manifest.members = members.clone();
        let tasks = members.iter().map(|member| crate::study::state::TaskState {
            member: member.id.clone(),
            site: "site".into(),
            case_dir: study
                .join("members")
                .join(&member.id)
                .join("site")
                .to_string_lossy()
                .into_owned(),
            status: TaskStatus::Succeeded,
            stage: None,
            reason: None,
            objective: None,
            validation_objective: None,
            process: None,
        });
        let mut state = StudyState::new("s".into(), tasks).unwrap();
        state.warnings.push("keep unrelated warning".into());
        state
            .warnings
            .push("uncertainty results are unavailable until every baseline task succeeds".into());

        for member in &members {
            let case = study.join("members").join(&member.id).join("site");
            std::fs::create_dir_all(&case).unwrap();
            std::fs::write(
                case.join("case.nml"),
                "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n/\n",
            )
            .unwrap();
            let index = member.candidate_index as f64;
            let (time, values): (&[f64], &[f64]) = if member.baseline {
                (&[0.0, 1800.0], &[999.0, 999.0])
            } else if member.id == "m000021" {
                (&[0.0, 3600.0], &[21.0, 21.0])
            } else if member.id == "m000020" {
                (&[0.0, 1800.0], &[20.0, f64::NAN])
            } else {
                (&[0.0, 1800.0], &[index, index])
            };
            write_history_nc(
                &case.join("out/site/history/site_hist_2000-01.nc"),
                &[("time", time), ("f_qle", values)],
            );
            write_json(
                &task_result_path(&manifest, &member.id, "site"),
                &TaskResult {
                    member: member.id.clone(),
                    site: "site".into(),
                    outputs: vec![OutputSummary {
                        variable: "f_qle".into(),
                        units: None,
                        count: values.iter().filter(|value| value.is_finite()).count(),
                        mean: super::super::science::finite_mean(values).unwrap(),
                        min: values.iter().copied().fold(f64::INFINITY, f64::min),
                        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    }],
                    calibration: Vec::new(),
                    validation: Vec::new(),
                    validation_errors: Vec::new(),
                },
            )
            .unwrap();
        }

        update_uncertainty_outputs(&manifest, &members, &mut state);

        let envelope: serde_json::Value = serde_json::from_slice(
            &std::fs::read(study.join("results/envelopes/site/f_qle.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(envelope["members"], 20);
        assert_eq!(envelope["n_eff"], serde_json::json!([20, 19]));
        assert_eq!(envelope["stable"], serde_json::json!([true, false]));
        assert_eq!(envelope["baseline"], serde_json::json!([999.0, 999.0]));
        assert_eq!(envelope["p50"], serde_json::json!([10.5, null]));
        assert!(!state
            .warnings
            .iter()
            .any(|warning| warning.contains("unavailable until every baseline task succeeds")));
        assert!(state.warnings.contains(&"keep unrelated warning".into()));
        assert!(state
            .warnings
            .iter()
            .any(|warning| warning.contains("different time axis")));
        assert!(state
            .warnings
            .iter()
            .any(|warning| warning.contains("insufficient ensemble support")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uncertainty_aggregation_rejects_missing_succeeded_task_results() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let root = std::env::temp_dir().join(format!(
            "colm-study-missing-result-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let study = root.join("study");
        let mut manifest = minimal_uncertainty_manifest(&study, vec!["f_qle".into()]);
        let members = vec![
            MemberPlan {
                id: "m000000".into(),
                generation: 0,
                candidate_index: 0,
                baseline: true,
                parameters: BTreeMap::new(),
            },
            MemberPlan {
                id: "m000001".into(),
                generation: 0,
                candidate_index: 1,
                baseline: false,
                parameters: BTreeMap::new(),
            },
        ];
        manifest.members = members.clone();
        let tasks = members.iter().map(|member| crate::study::state::TaskState {
            member: member.id.clone(),
            site: "site".into(),
            case_dir: study
                .join("members")
                .join(&member.id)
                .join("site")
                .to_string_lossy()
                .into_owned(),
            status: TaskStatus::Succeeded,
            stage: None,
            reason: None,
            objective: None,
            validation_objective: None,
            process: None,
        });
        let mut state = StudyState::new("s".into(), tasks).unwrap();
        for member in &members {
            let case = study.join("members").join(&member.id).join("site");
            std::fs::create_dir_all(&case).unwrap();
            std::fs::write(
                case.join("case.nml"),
                "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n/\n",
            )
            .unwrap();
            write_history_nc(
                &case.join("out/site/history/site_hist_2000-01.nc"),
                &[("time", &[0.0]), ("f_qle", &[1.0])],
            );
        }
        write_json(
            &task_result_path(&manifest, "m000000", "site"),
            &TaskResult {
                member: "m000000".into(),
                site: "site".into(),
                outputs: vec![OutputSummary {
                    variable: "f_qle".into(),
                    units: None,
                    count: 1,
                    mean: 1.0,
                    min: 1.0,
                    max: 1.0,
                }],
                calibration: Vec::new(),
                validation: Vec::new(),
                validation_errors: Vec::new(),
            },
        )
        .unwrap();

        write_json(
            &task_result_path(&manifest, "m000001", "site"),
            &TaskResult {
                member: "m000001".into(),
                site: "site".into(),
                outputs: vec![OutputSummary {
                    variable: "f_qle".into(),
                    units: None,
                    count: 1,
                    mean: 1.0,
                    min: 1.0,
                    max: 1.0,
                }],
                calibration: Vec::new(),
                validation: Vec::new(),
                validation_errors: Vec::new(),
            },
        )
        .unwrap();
        update_uncertainty_outputs(&manifest, &members, &mut state);
        assert!(study.join("results/envelopes/site/f_qle.json").is_file());
        assert!(study.join("results/importance.json").is_file());
        assert!(study.join("results/members.csv").is_file());

        std::fs::remove_file(task_result_path(&manifest, "m000001", "site")).unwrap();
        update_uncertainty_outputs(&manifest, &members, &mut state);
        assert!(!study.join("results/envelopes").exists());
        assert!(!study.join("results/importance.json").exists());
        assert!(!study.join("results/members.csv").exists());
        assert!(state.warnings.iter().any(|warning| {
            warning.contains("uncertainty result aggregation failed")
                && warning.contains("cannot read succeeded Study task result for m000001 / site")
        }));
        assert!(task_result_path(&manifest, "m000000", "site").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_marks_active_tasks_for_review_without_a_verified_scheduler_lock() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-status-unowned-live-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        let id = dir.file_name().unwrap().to_string_lossy().into_owned();
        write_status_manifest(&dir, &id);
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut process = supervisor_identity();
        process.executable = "not-the-recorded-scheduler".into();
        process.argv_sha256 = "wrong".into();
        let state = StudyState::new(
            id,
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: Some(process),
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();

        let recovered = status_state(&dir).unwrap().unwrap();
        let task = &recovered.tasks["m000001/site"];
        assert_eq!(recovered.status, StudyStatus::NeedsReview);
        assert_eq!(task.status, TaskStatus::NeedsReview);
        assert!(task.process.is_some());
        assert!(task
            .reason
            .as_deref()
            .unwrap()
            .contains("ownership could not be verified"));
        let error = finalize_idle_cancel(&dir).unwrap_err().to_string();
        assert!(error.contains("live unverified process"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quantile_bands_require_twenty_and_eighty_percent_support() {
        assert_eq!(required_ensemble_support(10), 20);
        assert_eq!(required_ensemble_support(40), 32);
        assert_eq!(required_ensemble_support(41), 33);
    }

    #[test]
    fn study_jobs_never_exceed_available_cpus() {
        assert_eq!(bounded_jobs(0, 8), 1);
        assert_eq!(bounded_jobs(4, 8), 4);
        assert_eq!(bounded_jobs(99, 8), 8);
        assert_eq!(bounded_jobs(4, 0), 1);
        assert_eq!(threads_per_job(16, 4), 4);
        assert_eq!(threads_per_job(4, 8), 1);
    }

    fn exited_child_pid() -> u32 {
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "exit", "0"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());
        pid
    }

    #[test]
    fn one_study_has_only_one_scheduler_writer() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-run-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let first = StudyRunLock::acquire(&dir).unwrap();
        assert!(StudyRunLock::acquire(&dir).is_err());
        drop(first);
        StudyRunLock::acquire(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn confirmed_retry_refuses_a_live_lock_and_clears_a_stale_one() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-retry-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join(".colm/studies/s");
        let checkpoint = dir.join("checkpoints/state");
        std::fs::create_dir_all(root.join("site")).unwrap();
        std::fs::write(
            root.join("site/case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n/\n",
        )
        .unwrap();
        std::fs::write(root.join("site/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        std::fs::create_dir_all(dir.join("samples")).unwrap();
        std::fs::write(
            dir.join("samples/design.csv"),
            "member,baseline,generation,candidate,DEF_TUNING_CNFAC\nm000001,false,0,1,0.5\n",
        )
        .unwrap();
        let manifest = Manifest {
            schema_version: 1,
            id: "s".into(),
            root: dir.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Uncertainty,
                method: StudyMethod::Lhs,
                seed: 0,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: BTreeMap::new(),
                site_mode: super::super::spec::SiteMode::Shared,
                parameters: vec![super::super::spec::ParameterSpec {
                    name: "DEF_TUNING_CNFAC".into(),
                    parameter_id: None,
                    scope_instance: None,
                    sample_min: 0.1,
                    sample_max: 0.9,
                    scale: None,
                }],
                outputs: vec!["f_lfevpa".into()],
                analysis_from: None,
                analysis_to: None,
                targets: Vec::new(),
                budget: Default::default(),
                hybrid: None,
            },
            members: Vec::new(),
            provenance: Default::default(),
        };
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let state = StudyState::new(
            "s".into(),
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: Some("old".into()),
                objective: Some(1.0),
                validation_objective: Some(2.0),
                process: Some(supervisor_identity()),
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let live = StudyRunLock::acquire(&dir).unwrap();
        assert!(retry(&dir, false).is_err());
        let error = retry(&dir, true).unwrap_err().to_string();
        assert!(error.contains("still running"), "{error}");
        drop(live);
        let mut stale = supervisor_identity();
        stale.pid = exited_child_pid();
        stale.heartbeat_unix = 0;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&stale).unwrap()).unwrap();
        let retried = retry(&dir, true).unwrap();
        let task = &retried.tasks["m000001/site"];
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.process.is_none());
        assert!(!dir.join("run.lock").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_validates_frozen_manifest_before_changing_requests_or_checkpoint() {
        let root = std::env::temp_dir().join(format!(
            "cs-rf-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = write_fake_kernel(&root);
        let case = root.join("site");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        std::fs::write(case.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let spec = root.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
                "outputs": ["Qle"]
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = super::super::engine::create(&root, &spec).unwrap();
        let study = PathBuf::from(&manifest.root);
        let checkpoint = study.join("checkpoints/state");
        let mut state = super::super::checkpoint::load_latest::<StudyState>(&checkpoint)
            .unwrap()
            .unwrap()
            .payload;
        state.tasks.values_mut().next().unwrap().status = TaskStatus::Failed;
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        std::fs::write(study.join("pause.request"), "pause").unwrap();
        std::fs::write(study.join("cancel.request"), "cancel").unwrap();
        std::fs::write(study.join("samples/design.csv"), "tampered\n").unwrap();

        let error = retry(&study, false).unwrap_err().to_string();
        assert!(error.contains("initial sample design changed"), "{error}");
        assert!(study.join("pause.request").is_file());
        assert!(study.join("cancel.request").is_file());
        let latest = super::super::checkpoint::load_latest::<StudyState>(&checkpoint)
            .unwrap()
            .unwrap()
            .payload;
        assert_eq!(
            latest.tasks.values().next().unwrap().status,
            TaskStatus::Failed
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_rejects_a_checkpoint_for_another_study_id() {
        let root = std::env::temp_dir().join(format!(
            "cs-ri-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = write_fake_kernel(&root);
        let case = root.join("site");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        std::fs::write(case.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let spec = root.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
                "outputs": ["Qle"]
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = super::super::engine::create(&root, &spec).unwrap();
        let study = PathBuf::from(&manifest.root);
        let checkpoint = study.join("checkpoints/state");
        let mut state = super::super::checkpoint::load_latest::<StudyState>(&checkpoint)
            .unwrap()
            .unwrap()
            .payload;
        state.study_id = "other".into();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();

        let error = retry(&study, false).unwrap_err().to_string();
        assert!(
            error.contains("checkpoint id other does not match manifest"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_gui_cancel_closes_active_tasks_and_rejects_a_new_scheduler() {
        assert!(scheduler_process_alive(std::process::id()).unwrap());
        let exited_pid = exited_child_pid();
        assert!(!scheduler_process_alive(exited_pid).unwrap());
        let dir = std::env::temp_dir().join(format!(
            "colm-study-finalize-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut running = crate::study::state::TaskState {
            member: "m000001".into(),
            site: "site".into(),
            case_dir: "/case".into(),
            status: TaskStatus::Running,
            stage: Some("colm".into()),
            reason: None,
            objective: None,
            validation_objective: None,
            process: Some(supervisor_identity()),
        };
        let mut queued = running.clone();
        queued.member = "m000002".into();
        queued.status = TaskStatus::Queued;
        running.process.as_mut().unwrap().pid = exited_pid;
        let state = StudyState::new("s".into(), [running, queued]).unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let mut owner = supervisor_identity();
        owner.pid = exited_pid;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&owner).unwrap()).unwrap();

        assert!(finalize_cancel(&dir, 9999).is_err());
        assert!(dir.join("run.lock").is_file());
        let cancelled = finalize_cancel(&dir, exited_pid).unwrap();
        assert_eq!(cancelled.status, StudyStatus::Cancelled);
        assert!(cancelled.tasks.values().all(|task| {
            task.status == TaskStatus::Cancelled && task.stage.is_none() && task.process.is_none()
        }));
        assert!(!dir.join("run.lock").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_marks_exited_running_tasks_for_review_when_no_scheduler_is_live() {
        let exited_pid = exited_child_pid();
        let dir = std::env::temp_dir().join(format!(
            "colm-study-status-review-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        let id = dir.file_name().unwrap().to_string_lossy().into_owned();
        write_status_manifest(&dir, &id);
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut process = supervisor_identity();
        process.pid = exited_pid;
        let state = StudyState::new(
            id,
            [
                crate::study::state::TaskState {
                    member: "m000001".into(),
                    site: "site".into(),
                    case_dir: "/case".into(),
                    status: TaskStatus::Evaluating,
                    stage: Some("metrics".into()),
                    reason: None,
                    objective: None,
                    validation_objective: None,
                    process: Some(process),
                },
                crate::study::state::TaskState {
                    member: "m000002".into(),
                    site: "site".into(),
                    case_dir: "/case".into(),
                    status: TaskStatus::Running,
                    stage: Some("colm".into()),
                    reason: None,
                    objective: None,
                    validation_objective: None,
                    process: None,
                },
            ],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();

        let recovered = status_state(&dir).unwrap().unwrap();
        let task = &recovered.tasks["m000001/site"];
        assert_eq!(recovered.status, StudyStatus::NeedsReview);
        assert_eq!(task.status, TaskStatus::NeedsReview);
        assert!(task.process.is_none());
        assert_eq!(
            recovered.tasks["m000002/site"].status,
            TaskStatus::NeedsReview
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_leaves_checkpoints_to_a_live_scheduler_with_a_stale_heartbeat() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-status-live-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        let id = dir.file_name().unwrap().to_string_lossy().into_owned();
        write_status_manifest(&dir, &id);
        std::fs::create_dir_all(&checkpoint).unwrap();
        // This test process plays the scheduler: alive, but its heartbeat is long expired.
        let mut owner = supervisor_identity();
        owner.pid = std::process::id();
        owner.heartbeat_unix = 0;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&owner).unwrap()).unwrap();
        let state = StudyState::new(
            id,
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: Some(owner.clone()),
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let before = std::fs::read_dir(&checkpoint).unwrap().count();

        let observed = status_state(&dir).unwrap().unwrap();
        assert_eq!(observed.tasks["m000001/site"].status, TaskStatus::Running);
        assert_eq!(
            std::fs::read_dir(&checkpoint).unwrap().count(),
            before,
            "status must not write a checkpoint while the scheduler process lives"
        );
        assert!(clear_stale_run_lock(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_replaced_run_lock_is_neither_refreshed_nor_removed_by_its_old_owner() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-lock-owner-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = StudyRunLock::acquire(&dir).unwrap();
        let path = dir.join("run.lock");
        assert!(owns_run_lock(&path, &lock.identity));
        replace_run_lock(&path, &lock.identity).unwrap();
        assert!(
            owns_run_lock(&path, &lock.identity),
            "atomic rewrite keeps a valid record"
        );
        // A replacement scheduler takes over after ours was judged stale.
        let mut successor = lock.identity.clone();
        successor.pid = lock.identity.pid.wrapping_add(1);
        successor.started_unix += 1;
        std::fs::write(&path, serde_json::to_vec(&successor).unwrap()).unwrap();
        drop(lock);
        assert!(
            path.is_file(),
            "the old owner left the successor's lock alone"
        );
        let owner = run_lock_owner(&path).unwrap().unwrap();
        assert_eq!(owner.pid, successor.pid);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_rejects_wrong_study_id_before_recovering_active_tasks() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-status-wrong-id-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join(".colm/studies/s");
        let checkpoint = dir.join("checkpoints/state");
        std::fs::create_dir_all(dir.join("samples")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&minimal_uncertainty_manifest(&dir, Vec::new())).unwrap(),
        )
        .unwrap();
        let state = StudyState::new(
            "other".into(),
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: None,
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();

        let error = status_state(&dir).unwrap_err().to_string();
        assert!(
            error.contains("checkpoint id other does not match manifest s"),
            "{error}"
        );
        let latest = super::super::checkpoint::load_latest::<StudyState>(&checkpoint)
            .unwrap()
            .unwrap()
            .payload;
        assert_eq!(latest.tasks["m000001/site"].status, TaskStatus::Running);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_treats_stale_lock_with_reused_pid_as_unverified_review() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-status-stale-reused-pid-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        let id = dir.file_name().unwrap().to_string_lossy().into_owned();
        write_status_manifest(&dir, &id);
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut process = supervisor_identity();
        process.executable = "not-the-recorded-scheduler".into();
        process.argv_sha256 = "wrong".into();
        let state = StudyState::new(
            id,
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: Some(process),
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let mut owner = supervisor_identity();
        owner.executable = "not-the-recorded-scheduler".into();
        owner.argv_sha256 = "wrong".into();
        owner.heartbeat_unix = 0;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&owner).unwrap()).unwrap();

        let recovered = status_state(&dir).unwrap().unwrap();
        let task = &recovered.tasks["m000001/site"];
        assert_eq!(recovered.status, StudyStatus::NeedsReview);
        assert_eq!(task.status, TaskStatus::NeedsReview);
        assert!(task.process.is_some());
        assert!(task
            .reason
            .as_deref()
            .unwrap()
            .contains("ownership could not be verified"));
        assert!(retry(&dir, true).is_err());
        let error = finalize_idle_cancel(&dir).unwrap_err().to_string();
        assert!(error.contains("still running"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_does_not_recover_while_scheduler_lock_is_live() {
        let exited_pid = exited_child_pid();
        let dir = std::env::temp_dir().join(format!(
            "colm-study-status-live-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        let id = dir.file_name().unwrap().to_string_lossy().into_owned();
        write_status_manifest(&dir, &id);
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut process = supervisor_identity();
        process.pid = exited_pid;
        let state = StudyState::new(
            id,
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: Some(process),
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let lock = StudyRunLock::acquire(&dir).unwrap();

        let recovered = status_state(&dir).unwrap().unwrap();
        assert_eq!(recovered.tasks["m000001/site"].status, TaskStatus::Running);
        drop(lock);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn idle_gui_cancel_can_finalize_an_abandoned_scheduler() {
        let exited_pid = exited_child_pid();
        let dir = std::env::temp_dir().join(format!(
            "colm-study-idle-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut process = supervisor_identity();
        process.pid = exited_pid;
        let state = StudyState::new(
            "s".into(),
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: Some(process),
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let mut owner = supervisor_identity();
        owner.pid = exited_pid;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&owner).unwrap()).unwrap();

        let cancelled = finalize_idle_cancel(&dir).unwrap();
        assert_eq!(cancelled.status, StudyStatus::Cancelled);
        assert!(!dir.join("run.lock").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn idle_gui_cancel_rejects_unverified_active_tasks() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-idle-cancel-unverified-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        std::fs::create_dir_all(&checkpoint).unwrap();
        let state = StudyState::new(
            "s".into(),
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::Running,
                stage: Some("colm".into()),
                reason: None,
                objective: None,
                validation_objective: None,
                process: None,
            }],
        )
        .unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();

        let error = finalize_idle_cancel(&dir).unwrap_err().to_string();
        assert!(error.contains("verified process identity"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn idle_gui_cancel_finalizes_a_recovered_review_task() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-idle-cancel-review-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let checkpoint = dir.join("checkpoints/state");
        std::fs::create_dir_all(&checkpoint).unwrap();
        let mut state = StudyState::new(
            "s".into(),
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: "/case".into(),
                status: TaskStatus::NeedsReview,
                stage: None,
                reason: Some("previous worker exited".into()),
                objective: None,
                validation_objective: None,
                process: None,
            }],
        )
        .unwrap();
        state.status = StudyStatus::NeedsReview;
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();

        let cancelled = finalize_idle_cancel(&dir).unwrap();
        assert_eq!(cancelled.status, StudyStatus::Cancelled);
        assert_eq!(
            cancelled.tasks["m000001/site"].status,
            TaskStatus::Cancelled
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_run_lock_is_not_treated_as_stale() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-corrupt-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run.lock"), b"{").unwrap();
        assert!(ensure_scheduler_idle(&dir).is_err());
        assert!(dir.join("run.lock").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_only_study_actions_ignore_only_a_confirmed_stale_lock() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-read-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let live = StudyRunLock::acquire(&dir).unwrap();
        assert!(ensure_scheduler_idle(&dir).is_err());
        drop(live);

        let mut stale_but_alive = supervisor_identity();
        stale_but_alive.heartbeat_unix = 0;
        std::fs::write(
            dir.join("run.lock"),
            serde_json::to_vec(&stale_but_alive).unwrap(),
        )
        .unwrap();
        assert!(ensure_scheduler_idle(&dir).is_err());
        assert!(dir.join("run.lock").is_file());

        let mut stale = supervisor_identity();
        stale.pid = exited_child_pid();
        stale.heartbeat_unix = 0;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&stale).unwrap()).unwrap();
        ensure_scheduler_idle(&dir).unwrap();
        assert!(!dir.join("run.lock").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn write_fake_kernel(root: &Path) -> PathBuf {
        let kernel = root.join("kernel");
        std::fs::create_dir_all(&kernel).unwrap();
        let body = b"#!/bin/sh\nexit 0\n";
        let mut hashes = serde_json::Map::new();
        for program in colm_kernel::PROGRAMS {
            let path = kernel.join(colm_kernel::program_file(program));
            std::fs::write(&path, body).unwrap();
            hashes.insert(program.into(), serde_json::Value::String(sha256(body)));
        }
        std::fs::write(
            kernel.join("manifest.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": 1,
                "preset": "study-test",
                "platform": "test",
                "colm_git_sha": "deadbeef",
                "generator_args": "SinglePoint LULC_IGBP",
                "macros": ["SinglePoint", "LULC_IGBP"],
                "built_with": "test",
                "netcdf_c": "test",
                "netcdf_fortran": "test",
                "hdf5": "test",
                "sha256": hashes,
            }))
            .unwrap(),
        )
        .unwrap();
        kernel
    }

    fn write_fake_spatial_kernel(root: &Path) -> PathBuf {
        let kernel = root.join("spatial-kernel");
        std::fs::create_dir_all(&kernel).unwrap();
        let body = b"#!/bin/sh\nexit 0\n";
        let mut hashes = serde_json::Map::new();
        for program in colm_kernel::PROGRAMS {
            let path = kernel.join(colm_kernel::program_file(program));
            std::fs::write(&path, body).unwrap();
            hashes.insert(program.into(), serde_json::Value::String(sha256(body)));
        }
        std::fs::write(
            kernel.join("manifest.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": 1,
                "preset": "study-spatial-test",
                "platform": "test",
                "colm_git_sha": "deadbeef",
                "generator_args": "GRIDBASED LULC_IGBP",
                "macros": ["GRIDBASED", "LULC_IGBP"],
                "built_with": "test",
                "netcdf_c": "test",
                "netcdf_fortran": "test",
                "hdf5": "test",
                "sha256": hashes,
            }))
            .unwrap(),
        )
        .unwrap();
        kernel
    }

    fn write_study_spec(root: &Path, kernel: &Path) -> PathBuf {
        let spec = root.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
                "outputs": ["Qle"]
            }))
            .unwrap(),
        )
        .unwrap();
        spec
    }

    fn manifest_only_study_fixture(hyperspectral: bool) -> (PathBuf, PathBuf, PathBuf) {
        let (root, _) = preview_fixture();
        let kernel = write_fake_kernel(&root);
        for program in colm_kernel::PROGRAMS {
            std::fs::remove_file(kernel.join(colm_kernel::program_file(program))).unwrap();
        }
        if hyperspectral {
            let path = kernel.join("manifest.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            manifest["macros"]
                .as_array_mut()
                .unwrap()
                .push("HYPERSPECTRAL".into());
            write_json(&path, &manifest).unwrap();
        }
        let spec_path = root.join("spec.json");
        let mut spec: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&spec_path).unwrap()).unwrap();
        spec["kernel_dir"] = serde_json::json!(kernel);
        spec["outputs"] = serde_json::json!(["f_lfevpa"]);
        write_json(&spec_path, &spec).unwrap();
        preflight_create(&root, &spec_path).unwrap();
        let manifest = super::super::engine::create(&root, &spec_path).unwrap();
        (root, kernel, PathBuf::from(manifest.root))
    }

    #[test]
    fn manifest_only_study_metadata_and_preview_do_not_require_fortran_programs() {
        let (root, kernel, study) = manifest_only_study_fixture(false);
        assert!(colm_kernel::Kernel::open(&kernel).is_err());
        let manifest = super::super::engine::status(&study).unwrap();
        assert!(!manifest.provenance.kernel_id.is_empty());
        super::super::engine::verify_frozen_inputs(&manifest).unwrap();
        super::super::engine::ensure_supported_study_manifest(&manifest, None).unwrap();
        assert!(!apply_preview(&study, "m000001").unwrap().is_empty());
        let retry_error = retry(&study, false).unwrap_err().to_string();
        assert!(
            retry_error.contains("no retryable failed tasks"),
            "{retry_error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_only_study_run_requires_fortran_programs_unless_both_backends_are_rust() {
        // The existing HYPERSPECTRAL guard is reached before Rust executable
        // discovery. This verifies kernel selection without fake scientific runs.
        let (root, kernel, study) = manifest_only_study_fixture(true);
        let checkpoints = checkpoint_count(&study);
        for (engine, preprocessors) in [
            (crate::ModelEngine::Rust, crate::PreprocessorMode::Rust),
            (crate::ModelEngine::Rust, crate::PreprocessorMode::Fortran),
            (crate::ModelEngine::Fortran, crate::PreprocessorMode::Rust),
            (
                crate::ModelEngine::Fortran,
                crate::PreprocessorMode::Fortran,
            ),
        ] {
            let error = run(
                &study,
                RunOptions {
                    kernel_dir: &kernel,
                    jobs: 1,
                    stream: false,
                    retry_failed: false,
                    preprocessors,
                    engine,
                },
            )
            .unwrap_err()
            .to_string();
            if engine == crate::ModelEngine::Rust && preprocessors == crate::PreprocessorMode::Rust
            {
                assert!(
                    error.contains("Rust preprocessing does not yet support HYPERSPECTRAL"),
                    "{error}"
                );
            } else {
                assert!(
                    error.contains("mksrfdata") && error.contains("missing"),
                    "{error}"
                );
            }
            assert_eq!(checkpoint_count(&study), checkpoints);
            assert!(!study.join("run.lock").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn study_create_rejects_a_gridded_case_with_a_single_point_kernel() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-spatial-case-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = write_fake_kernel(&root);
        std::fs::create_dir_all(root.join("site")).unwrap();
        std::fs::write(
            root.join("site/case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_file_mesh = 'mesh.nc'\n DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        std::fs::write(root.join("site/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let spec = write_study_spec(&root, &kernel);
        let error = super::super::engine::create(&root, &spec)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not match its base cases"), "{error}");
        assert!(!root.join(".colm/studies").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn study_preflight_rejects_a_spatial_kernel_for_single_point_cases() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-spatial-kernel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = write_fake_spatial_kernel(&root);
        std::fs::create_dir_all(root.join("site")).unwrap();
        std::fs::write(
            root.join("site/case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        std::fs::write(root.join("site/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let spec = write_study_spec(&root, &kernel);
        let error = preflight_create(&root, &spec).unwrap_err().to_string();
        assert!(error.contains("does not match its base cases"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_study_without_kernel_path_can_still_preview_singlepoint_case() {
        let (root, study) = preview_fixture();
        let rows = apply_preview(&study, "m000001").unwrap();
        assert!(!rows.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn checkpoint_count(study: &Path) -> usize {
        std::fs::read_dir(study.join("checkpoints/state"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count()
    }

    fn mark_baseline_case_spatial(root: &Path) {
        std::fs::write(
            root.join("caseA/case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   DEF_forcing_namelist = 'forcing.nml'\n   DEF_CatchmentMesh_data = 'catchment.nc'\n   DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
    }

    fn set_stored_kernel(study: &Path, kernel: &Path) {
        let mut manifest = super::super::engine::status(study).unwrap();
        manifest.spec.kernel_dir = Some(kernel.to_string_lossy().into_owned());
        write_json(&study.join("manifest.json"), &manifest).unwrap();
    }

    #[test]
    /// 研究建好后基础算例改成了格点算例：带内核的运行报内核与算例不配，其余操作在冻结的基础算例指纹上停下；
    /// 都在改动任何状态之前。
    fn study_actions_reject_a_kernel_that_no_longer_matches_the_cases_before_mutation() {
        let (root, study) = preview_fixture();
        let kernel = write_fake_kernel(&root);
        mark_baseline_case_spatial(&root);
        std::fs::write(study.join("pause.request"), "pause\n").unwrap();
        std::fs::write(study.join("cancel.request"), "cancel\n").unwrap();
        let checkpoints = checkpoint_count(&study);
        let output = root.join("applied-spatial");

        let run_error = run(
            &study,
            RunOptions {
                kernel_dir: &kernel,
                jobs: 1,
                stream: false,
                retry_failed: false,
                preprocessors: crate::PreprocessorMode::Fortran,
                engine: crate::ModelEngine::Fortran,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            run_error.contains("does not match its base cases"),
            "{run_error}"
        );
        assert_eq!(checkpoint_count(&study), checkpoints);

        let retry_error = retry(&study, false).unwrap_err().to_string();
        assert!(
            retry_error.contains("does not match its base cases"),
            "{retry_error}"
        );
        assert_eq!(checkpoint_count(&study), checkpoints);
        assert!(study.join("pause.request").is_file());
        assert!(study.join("cancel.request").is_file());

        let apply_error = apply(&study, "m000001", &output, None)
            .unwrap_err()
            .to_string();
        assert!(
            apply_error.contains("does not match its base cases"),
            "{apply_error}"
        );
        assert!(!output.exists());

        let preview_error = apply_preview(&study, "m000001").unwrap_err().to_string();
        assert!(
            preview_error.contains("does not match its base cases"),
            "{preview_error}"
        );

        let resume_error = super::super::state::resume(&study).unwrap_err().to_string();
        assert!(
            resume_error.contains("does not match its base cases"),
            "{resume_error}"
        );
        assert!(study.join("pause.request").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stored_spatial_kernel_study_actions_reject_single_point_cases() {
        let (root, study) = preview_fixture();
        let spatial_kernel = write_fake_spatial_kernel(&root);
        set_stored_kernel(&study, &spatial_kernel);
        std::fs::write(study.join("pause.request"), "pause\n").unwrap();
        std::fs::write(study.join("cancel.request"), "cancel\n").unwrap();
        let checkpoints = checkpoint_count(&study);
        let output = root.join("applied-spatial-kernel");

        let run_error = run(
            &study,
            RunOptions {
                kernel_dir: &spatial_kernel,
                jobs: 1,
                stream: false,
                retry_failed: false,
                preprocessors: crate::PreprocessorMode::Fortran,
                engine: crate::ModelEngine::Fortran,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            run_error.contains("does not match its base cases"),
            "{run_error}"
        );
        assert_eq!(checkpoint_count(&study), checkpoints);

        let retry_error = retry(&study, false).unwrap_err().to_string();
        assert!(
            retry_error.contains("does not match its base cases"),
            "{retry_error}"
        );
        assert_eq!(checkpoint_count(&study), checkpoints);
        assert!(study.join("pause.request").is_file());
        assert!(study.join("cancel.request").is_file());

        let apply_error = apply(&study, "m000001", &output, None)
            .unwrap_err()
            .to_string();
        assert!(
            apply_error.contains("does not match its base cases"),
            "{apply_error}"
        );
        assert!(!output.exists());

        let preview_error = apply_preview(&study, "m000001").unwrap_err().to_string();
        assert!(
            preview_error.contains("does not match its base cases"),
            "{preview_error}"
        );

        let resume_error = super::super::state::resume(&study).unwrap_err().to_string();
        assert!(
            resume_error.contains("does not match its base cases"),
            "{resume_error}"
        );
        assert!(study.join("pause.request").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn study_creation_requires_a_frozen_kernel() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-kernel-required-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = dir.join("spec.json");
        std::fs::write(
            &spec,
            r#"{"kind":"uncertainty","method":"lhs","base_cases":["case"],"parameters":[{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],"outputs":["Qle"]}"#,
        )
        .unwrap();
        let error = preflight_create(&dir, &spec).unwrap_err().to_string();
        assert!(error.contains("kernel_dir"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preflight_rejects_duplicate_base_cases_like_create() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-duplicate-base-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("caseA")).unwrap();
        std::fs::write(
            dir.join("caseA/case.nml"),
            "&nl_colm\n DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        let spec = dir.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "kernel_dir": "missing-kernel",
                "base_cases": ["caseA", "./caseA"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
                "outputs": ["Qle"]
            }))
            .unwrap(),
        )
        .unwrap();
        let error = preflight_create(&dir, &spec).unwrap_err().to_string();
        assert!(error.contains("duplicate base case"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preflight_rejects_shared_baseline_mismatch_like_create() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-baseline-mismatch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = write_fake_kernel(&dir);
        for (site, value) in [("caseA", "0.5"), ("caseB", "0.7")] {
            std::fs::create_dir_all(dir.join(site)).unwrap();
            std::fs::write(
                dir.join(site).join("case.nml"),
                format!("&nl_colm\n DEF_TUNING_CNFAC = {value}\n/\n"),
            )
            .unwrap();
        }
        let spec = dir.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "kernel_dir": kernel,
                "base_cases": ["caseA", "caseB"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
                "outputs": ["Qle"]
            }))
            .unwrap(),
        )
        .unwrap();
        let error = preflight_create(&dir, &spec).unwrap_err().to_string();
        assert!(
            error.contains("shared study requires the same baseline value"),
            "{error}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn objective_tables_write_parseable_metrics_json_result_file() {
        let root = std::env::temp_dir().join(format!(
            "colm-study-metrics-json-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target_key = "Qle,\r\nquoted\"key";
        let manifest = Manifest {
            schema_version: 1,
            id: "s".into(),
            root: root.to_string_lossy().into_owned(),
            created_unix: 0,
            spec: StudySpec {
                kind: StudyKind::Tuning,
                method: StudyMethod::DifferentialEvolution,
                seed: 0,
                kernel_dir: None,
                base_cases: vec!["site".into()],
                observations: BTreeMap::new(),
                site_mode: super::super::spec::SiteMode::Shared,
                parameters: Vec::new(),
                outputs: Vec::new(),
                analysis_from: None,
                analysis_to: None,
                targets: vec![TargetSpec {
                    key: target_key.into(),
                    site: None,
                    variable: "Qle".into(),
                    metric: ObjectiveMetric::Nrmse,
                    weight: 1.0,
                    from: 0,
                    to: 10,
                    validation_from: None,
                    validation_to: None,
                    min_pairs: 2,
                    source: None,
                    corrected: false,
                    aggregate: None,
                }],
                budget: Default::default(),
                hybrid: None,
            },
            members: Vec::new(),
            provenance: Default::default(),
        };
        let mut state = StudyState::new(
            "s".into(),
            [crate::study::state::TaskState {
                member: "m000001".into(),
                site: "site".into(),
                case_dir: String::new(),
                status: TaskStatus::Succeeded,
                stage: None,
                reason: None,
                objective: Some(0.25),
                validation_objective: Some(0.3),
                process: None,
            }],
        )
        .unwrap();
        state.candidates.insert(
            "m000001".into(),
            CandidateState {
                generation: 0,
                feasible: true,
                calibration: Some(0.25),
                validation: Some(0.3),
                reason: None,
            },
        );
        write_json(
            &task_result_path(&manifest, "m000001", "site"),
            &TaskResult {
                member: "m000001".into(),
                site: "site".into(),
                outputs: Vec::new(),
                calibration: vec![TargetResult {
                    key: target_key.into(),
                    site: "site".into(),
                    variable: "Qle".into(),
                    period: "calibration".into(),
                    metric: ObjectiveMetric::Nrmse,
                    weight: 1.0,
                    min_pairs: 2,
                    pairs: 12,
                    value: 0.4,
                    observation_sd: Some(2.0),
                    loss: 0.2,
                    model_mean: 5.0,
                    observation_mean: 4.5,
                    support_hash: pair_support_hash(
                        &(0..12).collect::<Vec<_>>(),
                        &(0..12).map(|v| v as f64).collect::<Vec<_>>(),
                    ),
                }],
                validation: Vec::new(),
                validation_errors: Vec::new(),
            },
        )
        .unwrap();

        write_objective_tables(&manifest, &state).unwrap();
        let files = result_files(&root).unwrap();
        assert!(files.iter().any(|file| file.path == "metrics.json"));
        let rows: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("results/metrics.json")).unwrap())
                .unwrap();
        assert_eq!(rows[0]["member"], "m000001");
        assert_eq!(rows[0]["target"], target_key);
        let csv = std::fs::read_to_string(root.join("results/metrics.csv")).unwrap();
        assert!(csv.contains("\"Qle,\r\nquoted\"\"key\""), "{csv:?}");
        assert_eq!(rows[0]["min_pairs"], 2);
        assert_eq!(rows[0]["observation_sd"], 2.0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scheduler_log_survives_restart_without_copying_raw_model_chatter() {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-log-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        emit(
            &dir,
            false,
            serde_json::json!({"kind":"task_started","member":"m1"}),
        )
        .unwrap();
        emit(
            &dir,
            false,
            serde_json::json!({"kind":"task_log","line":"large raw line"}),
        )
        .unwrap();
        let events = event_log_tail(&dir, 20).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["kind"], "task_started");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preflight_rejects_tuning_windows_without_enough_observations() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let root = std::env::temp_dir().join(format!(
            "colm-study-preflight-pairs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = write_fake_kernel(&root);
        let case = root.join("site");
        std::fs::create_dir_all(&case).unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm
 DEF_CASE_NAME = 'site'
 DEF_dir_output = 'out'
 DEF_TUNING_CNFAC = 0.5
/
",
        )
        .unwrap();
        let observation = root.join("obs.nc");
        {
            let mut file = netcdf::create(&observation).unwrap();
            file.add_dimension("time", 2).unwrap();
            let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
            time.put_attribute("units", "seconds since 1970-01-01 00:00:00")
                .unwrap();
            time.put_values(&[0.0, 1800.0], ..).unwrap();
            file.add_variable::<f64>("Qle", &["time"])
                .unwrap()
                .put_values(&[1.0, 2.0], ..)
                .unwrap();
            file.add_variable::<f64>("Qle_qc", &["time"])
                .unwrap()
                .put_values(&[0.0, 0.0], ..)
                .unwrap();
        }
        let spec = root.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "tuning",
                "method": "differential-evolution",
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "observations": {"site": observation},
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
                "targets": [{"key":"Qle","variable":"Qle","from":0,"to":10,"min_pairs":2}],
                "budget": {"population":4,"generations":1}
            }))
            .unwrap(),
        )
        .unwrap();
        let error = preflight_create(&root, &spec).unwrap_err().to_string();
        assert!(error.contains("usable observation"), "{error}");
        // 订正目标而观测里没有 Qle_cor：预检就拒，不等基准跑完才在评价阶段失败。
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "tuning",
                "method": "differential-evolution",
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "observations": {"site": observation},
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
                "targets": [{"key":"Qle","variable":"Qle","from":0,"to":10,"min_pairs":2,"corrected":true}],
                "budget": {"population":4,"generations":1}
            }))
            .unwrap(),
        )
        .unwrap();
        let error = preflight_create(&root, &spec).unwrap_err().to_string();
        assert!(
            error.contains("energy-balance-corrected Qle_cor"),
            "{error}"
        );
        // Windows netCDF may release its final test-file handle after this assertion returns.
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn lhs_study_runs_end_to_end_with_a_fake_kernel() {
        let _netcdf_guard = crate::netcdf_test_guard();
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "colm-study-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let kernel = root.join("kernel");
        let case = root.join("site");
        std::fs::create_dir_all(&kernel).unwrap();
        std::fs::create_dir_all(&case).unwrap();
        let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../oracle/golden/CN-Cng_hist_2008-01.nc")
            .canonicalize()
            .unwrap();
        let script = format!(
            r#"#!/bin/sh
set -eu
nml="$1"
case_name=$(sed -n "s/^[[:space:]]*DEF_CASE_NAME[[:space:]]*=[[:space:]]*'\([^']*\)'.*/\1/p" "$nml")
output_root=$(sed -n "s/^[[:space:]]*DEF_dir_output[[:space:]]*=[[:space:]]*'\([^']*\)'.*/\1/p" "$nml")
lc_year=$(sed -n "s/^[[:space:]]*DEF_LC_YEAR[[:space:]]*=[[:space:]]*\([0-9][0-9]*\).*/\1/p" "$nml")
lc_year=${{lc_year:-2005}}
lc=$(printf 'lc%04d' "$lc_year")
out="$output_root/$case_name"
program=$(basename "$0")
case "$program" in
  mksrfdata*) mkdir -p "$out/landdata"; : > "$out/landdata/srfdata.nc"; echo 'Successful in surface data making.' ;;
  mkinidata*) mkdir -p "$out/restart/const"; : > "$out/restart/const/${{case_name}}_restart_const_${{lc}}_w180_s90.nc"; : > "$out/restart/const/${{case_name}}_restart_const_${{lc}}.nc"; echo 'CoLM Initialization Execution Completed' ;;
  colm*) mkdir -p "$out/history"; cp '{}' "$out/history/${{case_name}}_hist_2008-01.nc"; echo 'CoLM Execution Completed.' ;;
esac
"#,
            golden.display()
        );
        let mut hashes = serde_json::Map::new();
        for program in colm_kernel::PROGRAMS {
            let path = kernel.join(colm_kernel::program_file(program));
            std::fs::write(&path, &script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            hashes.insert(
                program.into(),
                serde_json::Value::String(sha256(script.as_bytes())),
            );
        }
        std::fs::write(
            kernel.join("manifest.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema": 1,
                "preset": "study-test",
                "platform": "test",
                "colm_git_sha": "deadbeef",
                "generator_args": "SinglePoint LULC_IGBP",
                "macros": ["SinglePoint", "LULC_IGBP"],
                "built_with": "test",
                "netcdf_c": "test",
                "netcdf_fortran": "test",
                "hdf5": "test",
                "sha256": hashes,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'site'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_LC_YEAR = 2010\n DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        std::fs::write(case.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        let spec = root.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "seed": 7,
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
                "outputs": ["f_lfevpa"],
                "budget": {"candidate_count": 2}
            }))
            .unwrap(),
        )
        .unwrap();
        // Preflight must validate the frozen plan, not require history from the
        // untouched base case; the Study runs its own baseline member below.
        preflight_create(&root, &spec).unwrap();
        let profile_spec = root.join("profile-spec.json");
        let mut profile_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&spec).unwrap()).unwrap();
        profile_json["outputs"] = serde_json::json!(["f_t_soisno"]);
        std::fs::write(
            &profile_spec,
            serde_json::to_vec_pretty(&profile_json).unwrap(),
        )
        .unwrap();
        let error = preflight_create(&root, &profile_spec)
            .unwrap_err()
            .to_string();
        assert!(error.contains("vertical profile"), "{error}");
        let observation = root.join("obs.nc");
        {
            let mut file = netcdf::create(&observation).unwrap();
            file.add_dimension("time", 2).unwrap();
            let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
            time.put_attribute("units", "seconds since 1970-01-01 00:00:00")
                .unwrap();
            time.put_values(&[0.0, 1800.0], ..).unwrap();
            file.add_variable::<f64>("Qle", &["time"])
                .unwrap()
                .put_values(&[1.0, 2.0], ..)
                .unwrap();
            file.add_variable::<f64>("Qle_qc", &["time"])
                .unwrap()
                .put_values(&[0.0, 0.0], ..)
                .unwrap();
        }
        let tuning_spec = root.join("tuning-spec.json");
        std::fs::write(
            &tuning_spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "tuning",
                "method": "differential-evolution",
                "kernel_dir": kernel,
                "base_cases": ["site"],
                "observations": {"site": observation},
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
                "targets": [{"key":"Qle","variable":"Qle","from":0,"to":3600,"min_pairs":2}],
                "budget": {"population":4,"generations":1}
            }))
            .unwrap(),
        )
        .unwrap();
        preflight_create(&root, &tuning_spec).unwrap();
        let manifest = super::super::engine::create(&root, &spec).unwrap();
        let study = PathBuf::from(&manifest.root);
        let state = run(
            &study,
            RunOptions {
                kernel_dir: &kernel,
                jobs: 2,
                stream: false,
                retry_failed: false,
                preprocessors: crate::PreprocessorMode::Fortran,
                engine: crate::ModelEngine::Fortran,
            },
        )
        .unwrap();
        assert_eq!(state.status, StudyStatus::Completed);
        assert_eq!(state.tasks.len(), 3);
        assert!(state
            .tasks
            .values()
            .all(|task| task.status == TaskStatus::Succeeded));
        let files = result_files(&study).unwrap();
        assert!(files
            .iter()
            .any(|file| file.path == "envelopes/site/f_lfevpa.json"));
        assert!(files.iter().any(|file| file.path == "members.csv"));
        std::fs::remove_file(task_result_path(&manifest, "m000001", "site")).unwrap();
        let members = super::super::generation::load_members(&manifest).unwrap();
        let mut refreshed = state.clone();
        refresh_candidates(&manifest, &members, &mut refreshed).unwrap();
        let candidate = &refreshed.candidates["m000001"];
        assert!(!candidate.feasible);
        assert!(candidate.reason.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn preview_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "cs-pv-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("caseA")).unwrap();
        std::fs::write(
            root.join("caseA/case.nml"),
            "&nl_colm\n   DEF_CASE_NAME = 'base'\n   DEF_dir_output = 'out'\n   DEF_forcing_namelist = 'forcing.nml'\n   DEF_TUNING_CNFAC = 0.5\n/\n",
        )
        .unwrap();
        std::fs::write(root.join("caseA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        std::fs::write(root.join("caseA/site.nc"), b"site").unwrap();
        let spec_path = root.join("spec.json");
        let spec = super::super::spec::StudySpec {
            kind: StudyKind::Uncertainty,
            method: StudyMethod::Lhs,
            seed: 1,
            kernel_dir: None,
            base_cases: vec!["caseA".into()],
            observations: BTreeMap::new(),
            site_mode: super::super::spec::SiteMode::Shared,
            parameters: vec![super::super::spec::ParameterSpec {
                name: "DEF_TUNING_CNFAC".into(),
                parameter_id: None,
                scope_instance: None,
                sample_min: 0.1,
                sample_max: 0.9,
                scale: Some(super::super::spec::ScaleSpec::Linear),
            }],
            outputs: vec!["f_qle".into()],
            analysis_from: None,
            analysis_to: None,
            targets: vec![],
            budget: super::super::spec::StudyBudget {
                candidate_count: Some(2),
                ..Default::default()
            },
            hybrid: None,
        };
        std::fs::write(&spec_path, serde_json::to_string(&spec).unwrap()).unwrap();
        let manifest = super::super::engine::create(&root, &spec_path).unwrap();
        (root, PathBuf::from(manifest.root))
    }

    fn multi_site_apply_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "cs-am-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for site in ["caseA", "caseB"] {
            std::fs::create_dir_all(root.join(site)).unwrap();
            std::fs::write(
                root.join(site).join("case.nml"),
                format!(
                    "&nl_colm\n DEF_CASE_NAME = '{site}'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_TUNING_CNFAC = 0.5\n/\n"
                ),
            )
            .unwrap();
            std::fs::write(root.join(site).join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
        }
        // Give the blocker thread enough time to inject a failure after staging
        // exists but before the second site starts materializing.
        std::fs::write(
            root.join("caseA/large_parameter.nml"),
            vec![b'x'; 8 * 1024 * 1024],
        )
        .unwrap();
        let spec = root.join("spec.json");
        std::fs::write(
            &spec,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "uncertainty",
                "method": "lhs",
                "seed": 1,
                "base_cases": ["caseA", "caseB"],
                "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.1,"sample_max":0.9}],
                "outputs": ["f_qle"],
                "budget": {"candidate_count": 2}
            }))
            .unwrap(),
        )
        .unwrap();
        let manifest = super::super::engine::create(&root, &spec).unwrap();
        (root, PathBuf::from(manifest.root))
    }

    #[test]
    fn apply_preview_is_readonly_and_resolves_best_member() {
        let (root, study) = preview_fixture();
        let baseline = root.join("caseA/case.nml");
        let before = std::fs::read_to_string(&baseline).unwrap();
        let explicit = apply_preview(&study, "m000001").unwrap();
        assert_eq!(std::fs::read_to_string(&baseline).unwrap(), before);
        let row = explicit
            .iter()
            .find(|row| row.field == "DEF_TUNING_CNFAC")
            .expect("preview must include changed field");
        assert_eq!(row.site, "caseA");
        assert!(
            Path::new(&row.file).ends_with(Path::new("caseA").join("case.nml")),
            "{}",
            row.file
        );
        assert_eq!(row.old, "0.5");
        assert!(!row.new.is_empty());

        let checkpoint = study.join("checkpoints/state");
        let mut state = super::super::checkpoint::load_latest::<StudyState>(&checkpoint)
            .unwrap()
            .unwrap()
            .payload;
        state.best_member = Some("m000001".into());
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        assert_eq!(
            serde_json::to_value(apply_preview(&study, "best").unwrap()).unwrap(),
            serde_json::to_value(&explicit).unwrap()
        );

        let out = root.join("applied");
        let published = apply(&study, "best", &out, None).unwrap();
        assert!(out.join("case.nml").is_file());
        let applied = std::fs::read_to_string(out.join("case.nml")).unwrap();
        assert!(!applied.contains(".colm-study-apply-"), "{applied}");
        assert!(applied.contains(published[0].join("forcing.nml").to_string_lossy().as_ref()));
        assert_eq!(std::fs::read_to_string(&baseline).unwrap(), before);

        let precreated = root.join("precreated");
        std::fs::create_dir(&precreated).unwrap();
        apply(&study, "best", &precreated, None).unwrap();
        assert!(precreated.join("case.nml").is_file());

        let spaced = root.join("has space");
        let error = apply(&study, "best", &spaced, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot contain spaces"), "{error}");
        assert!(!spaced.exists());

        let occupied = root.join("occupied");
        std::fs::write(&occupied, b"user data").unwrap();
        let error = apply(&study, "best", &occupied, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a directory"), "{error}");
        assert_eq!(std::fs::read(&occupied).unwrap(), b"user data");

        let invalid = root.join("invalid");
        assert!(apply(&study, "best", &invalid, Some("bad/name")).is_err());
        assert!(!invalid.exists());
        assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".colm-study-apply-")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evaluation_evidence_rejects_changed_raw_history_and_observations() {
        let root = std::env::temp_dir().join(format!(
            "study-evidence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let case = root.join("caseA");
        let study = root.join(".colm/studies/identity");
        fs::create_dir_all(case.join("out/caseA/history")).unwrap();
        fs::create_dir_all(&study).unwrap();
        fs::write(case.join("case.nml"), "&nl_colm\nDEF_CASE_NAME='caseA'\nSITE_fsitedata='site.nc'\nDEF_forcing_namelist='forcing.nml'\n/\n").unwrap();
        fs::write(
            case.join("forcing.nml"),
            "&nl_colm_forcing\nDEF_dir_forcing='.'\nDEF_forcing_fprefix='forcing.nc'\n/\n",
        )
        .unwrap();
        fs::write(case.join("site.nc"), b"site").unwrap();
        fs::write(case.join("forcing.nc"), b"forcing").unwrap();
        fs::write(root.join("obs.nc"), b"observations").unwrap();
        let history = case.join("out/caseA/history/caseA_hist_2000-01.nc");
        let mut bytes = vec![0_u8; 256 * 1024];
        fs::write(&history, &bytes).unwrap();
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "schema_version":1,"id":"identity","root":study,"created_unix":0,
            "spec":{"kind":"uncertainty","method":"oat","base_cases":["caseA"],"parameters":[],"outputs":["f_tref"],"observations":{"caseA":root.join("obs.nc")}},
            "members":[],
        })).unwrap();
        let result = TaskResult {
            member: "m1".into(),
            site: "caseA".into(),
            outputs: Vec::new(),
            calibration: Vec::new(),
            validation: Vec::new(),
            validation_errors: Vec::new(),
        };
        let result_path = task_result_path(&manifest, "m1", "caseA");
        write_json(&result_path, &result).unwrap();
        let proof = EvaluationIdentity {
            schema: 1,
            execution: actual_execution(&manifest, "caseA", &case).unwrap(),
            outputs: history_identity(&case).unwrap(),
            result: crate::fingerprint::full_file(&result_path, "evaluation").unwrap(),
        };
        write_json(&evaluation_identity_path(&manifest, "m1", "caseA"), &proof).unwrap();
        verify_task_evidence(&manifest, "m1", "caseA", &case).unwrap();
        let execution = actual_execution(&manifest, "caseA", &case).unwrap();
        fs::write(case.join("forcing.nc"), b"temporary forcing change").unwrap();
        fs::write(case.join("forcing.nc"), b"forcing").unwrap();
        finish_execution(&case, Some(execution)).unwrap();
        let execution = actual_execution(&manifest, "caseA", &case).unwrap();
        fs::write(case.join("site.nc"), b"temporary site change").unwrap();
        fs::write(case.join("site.nc"), b"site").unwrap();
        assert!(finish_execution(&case, Some(execution))
            .unwrap_err()
            .to_string()
            .contains("restored payload"));
        bytes[128 * 1024] = 1;
        fs::write(&history, &bytes).unwrap();
        assert!(verify_task_evidence(&manifest, "m1", "caseA", &case)
            .unwrap_err()
            .to_string()
            .contains("history output changed"));
        bytes[128 * 1024] = 0;
        fs::write(&history, &bytes).unwrap();
        fs::write(root.join("obs.nc"), b"other values").unwrap();
        assert!(verify_task_evidence(&manifest, "m1", "caseA", &case)
            .unwrap_err()
            .to_string()
            .contains("observations changed"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_publish_preserves_destination_populated_during_staging() {
        let (root, study) = preview_fixture();
        let output = root.join("applied");
        std::fs::create_dir(&output).unwrap();
        assert!(output.read_dir().unwrap().next().is_none());

        let stage = root.join("staged");
        apply(&study, "m000001", &stage, Some("a")).unwrap();
        let published = output.join("case.nml");
        std::fs::write(&published, b"first published case").unwrap();

        let error = publish_applied_cases(&stage, &output).unwrap_err();
        assert!(error.to_string().contains("cannot publish"), "{error}");
        assert_eq!(std::fs::read(&published).unwrap(), b"first published case");
        assert_eq!(output.read_dir().unwrap().count(), 1);
        assert!(!stage.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multi_site_apply_failure_publishes_nothing() {
        let (root, study) = multi_site_apply_fixture();
        std::fs::remove_file(root.join("caseB/forcing.nml")).unwrap();
        let out = root.join("applied-multi");
        assert!(apply(&study, "m000001", &out, None).is_err());
        assert!(!out.exists());
        assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".colm-study-apply-")));
        std::fs::remove_dir_all(root).unwrap();
    }

    // ---- P1-A：结构化 Study 证据（study-status.evidence） ----

    fn evidence_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "colm-study-evidence-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("samples")).unwrap();
        root
    }

    fn evidence_manifest(root: &Path) -> Manifest {
        let mut manifest = tuning_manifest(root);
        manifest.spec.parameters = vec![super::super::spec::ParameterSpec {
            name: "DEF_TUNING_CNFAC".into(),
            parameter_id: None,
            scope_instance: None,
            sample_min: 0.1,
            sample_max: 0.9,
            scale: Some(ScaleSpec::Linear),
        }];
        manifest
    }

    /// 样本 CSV 的一行：成员、是否基线、代、候选序号、各决策维的值（`None` 写空格）。
    type EvidenceSampleRow<'a> = (&'a str, bool, usize, usize, &'a [Option<f64>]);

    fn write_evidence_sample_file(manifest: &Manifest, file: &str, rows: &[EvidenceSampleRow<'_>]) {
        let keys = manifest
            .spec
            .parameters
            .iter()
            .map(super::super::spec::ParameterSpec::member_key)
            .collect::<Vec<_>>();
        let mut text = String::from("member,baseline,generation,candidate");
        for key in &keys {
            text.push(',');
            text.push_str(key);
        }
        text.push('\n');
        for (id, baseline, generation, candidate, values) in rows {
            text.push_str(&format!("{id},{baseline},{generation},{candidate}"));
            for value in *values {
                text.push(',');
                if let Some(value) = value {
                    text.push_str(&value.to_string());
                }
            }
            text.push('\n');
        }
        let path = Path::new(&manifest.root).join("samples").join(file);
        fs::write(path, text).unwrap();
    }

    fn evidence_result_row(period: &str, key: &str, loss: f64) -> TargetResult {
        let mut row = target_row(period, vec![1, 2]);
        row.key = key.into();
        row.variable = key.into();
        row.loss = loss;
        // Nrmse 的 loss = rmse / observation_sd，observation_sd 取 2.0。
        row.value = loss * 2.0;
        row
    }

    fn write_evidence_result(
        manifest: &Manifest,
        member: &str,
        calibration: &[(&str, f64)],
        validation: &[(&str, f64)],
        validation_errors: &[(&str, &str)],
    ) {
        let site = manifest.spec.base_cases[0].clone();
        let result = TaskResult {
            member: member.into(),
            site: site.clone(),
            outputs: Vec::new(),
            calibration: calibration
                .iter()
                .map(|(key, loss)| evidence_result_row("calibration", key, *loss))
                .collect(),
            validation: validation
                .iter()
                .map(|(key, loss)| evidence_result_row("validation", key, *loss))
                .collect(),
            validation_errors: validation_errors
                .iter()
                .map(|(key, reason)| TargetError {
                    key: (*key).into(),
                    site: site.clone(),
                    variable: (*key).into(),
                    period: "validation".into(),
                    reason: (*reason).into(),
                })
                .collect(),
        };
        let path = task_result_path(manifest, member, &site);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
    }

    fn evidence_state(
        manifest: &Manifest,
        status: StudyStatus,
        members: &[&str],
        candidates: &[(&str, Option<f64>, Option<f64>)],
        best: Option<(&str, f64)>,
    ) -> StudyState {
        let tasks = manifest
            .spec
            .base_cases
            .iter()
            .flat_map(|site| {
                members
                    .iter()
                    .map(move |member| crate::study::state::TaskState {
                        member: (*member).into(),
                        site: site.clone(),
                        case_dir: "case".into(),
                        status: TaskStatus::Succeeded,
                        stage: None,
                        reason: None,
                        objective: None,
                        validation_objective: None,
                        process: None,
                    })
            })
            .collect::<Vec<_>>();
        let mut state = StudyState::new(manifest.id.clone(), tasks).unwrap();
        state.status = status;
        for (member, calibration, validation) in candidates {
            state.candidates.insert(
                (*member).to_string(),
                CandidateState {
                    generation: 0,
                    feasible: calibration.is_some(),
                    calibration: *calibration,
                    validation: *validation,
                    reason: calibration.is_none().then(|| "failed".into()),
                },
            );
        }
        if let Some((member, score)) = best {
            state.best_member = Some(member.into());
            state.best_objective = Some(score);
        }
        state
    }

    fn set_evidence_task_status(
        state: &mut StudyState,
        member: &str,
        site: &str,
        status: TaskStatus,
        reason: &str,
    ) {
        let task = state
            .tasks
            .get_mut(&crate::study::state::task_id(member, site))
            .unwrap();
        task.status = status;
        task.reason = Some(reason.into());
    }

    /// 率定期改善、验证期变差：两期各自比较，`relation` 必须分开报。
    #[test]
    fn evidence_compares_calibration_and_validation_within_the_same_period() {
        let root = evidence_root("dual-period");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000001", false, 1, 1, &[Some(0.2)])],
        );
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.8)], &[("Qle", 0.6)], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.4)], &[("Qle", 0.9)], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[
                ("m000000", Some(0.8), Some(0.6)),
                ("m000001", Some(0.4), Some(0.9)),
            ],
            Some(("m000001", 0.4)),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(evidence["selection"]["best_member"], "m000001");
        assert_eq!(evidence["study"]["status"], "completed");
        assert_eq!(evidence["execution"]["state"], "completed");
        assert_eq!(
            evidence["comparison"]["baseline"]["calibration"]["score"].as_f64(),
            Some(0.8)
        );
        assert_eq!(
            evidence["comparison"]["best"]["calibration"]["score"].as_f64(),
            Some(0.4)
        );
        assert_eq!(
            evidence["comparison"]["relation"]["calibration"],
            "lower_loss"
        );
        assert_eq!(
            evidence["comparison"]["best"]["validation"]["targets"][0]["loss"].as_f64(),
            Some(0.9)
        );
        assert_eq!(
            evidence["comparison"]["relation"]["validation"],
            "higher_loss"
        );
        assert_eq!(evidence["validation"]["status"], "available");
        assert_eq!(evidence["verification"]["reverified"], false);
        assert_eq!(evidence["objective"]["direction"], "minimize");
        assert!(
            evidence_bytes(&evidence) <= EVIDENCE_BUDGET_BYTES,
            "{}",
            evidence_bytes(&evidence)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 验证更好的另一个候选不改变记录里的 best_member：选择只用率定期。
    #[test]
    fn evidence_selection_never_uses_validation_to_replace_best_member() {
        let root = evidence_root("selection");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[
                ("m000001", false, 0, 1, &[Some(0.4)]),
                ("m000002", false, 0, 2, &[Some(0.45)]),
            ],
        );
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.8)], &[("Qle", 0.6)], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.4)], &[("Qle", 0.9)], &[]);
        write_evidence_result(&manifest, "m000002", &[("Qle", 0.45)], &[("Qle", 0.2)], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001", "m000002"],
            &[
                ("m000000", Some(0.8), Some(0.6)),
                ("m000001", Some(0.4), Some(0.9)),
                ("m000002", Some(0.45), Some(0.2)),
            ],
            Some(("m000001", 0.4)),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(evidence["selection"]["best_member"], "m000001");
        assert_eq!(evidence["selection"]["best_objective"].as_f64(), Some(0.4));
        assert_eq!(
            evidence["selection"]["candidates"]["feasible_non_baseline"],
            2
        );
        // 只报基线 vs 记录的最优；验证更好的 m000002 不会出现在证据里。
        assert!(!evidence.to_string().contains("m000002"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// best 比 baseline 差也是合法记录：`relation` 报 higher_loss，不能改写成"改善"。
    #[test]
    fn evidence_reports_best_member_worse_than_baseline() {
        let root = evidence_root("worse");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000001", false, 1, 1, &[Some(0.7)])],
        );
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.3)], &[("Qle", 0.6)], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.5)], &[("Qle", 0.2)], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[
                ("m000000", Some(0.3), Some(0.6)),
                ("m000001", Some(0.5), Some(0.2)),
            ],
            Some(("m000001", 0.5)),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(
            evidence["comparison"]["relation"]["calibration"],
            "higher_loss"
        );
        assert_eq!(
            evidence["comparison"]["relation"]["validation"],
            "lower_loss"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// baseline 分数为 0 是有效分数；结果文件缺失才是 unavailable。
    #[test]
    fn evidence_keeps_zero_baseline_and_reports_missing_baseline() {
        let root = evidence_root("zero-baseline");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000001", false, 1, 1, &[Some(0.2)])],
        );
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.0)], &[("Qle", 0.5)], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.4)], &[("Qle", 0.2)], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[("m000000", Some(0.0), None), ("m000001", Some(0.4), None)],
            Some(("m000001", 0.4)),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(
            evidence["comparison"]["baseline"]["calibration"]["status"],
            "recorded"
        );
        assert_eq!(
            evidence["comparison"]["baseline"]["calibration"]["score"].as_f64(),
            Some(0.0)
        );
        assert_eq!(
            evidence["comparison"]["relation"]["calibration"],
            "higher_loss"
        );

        fs::remove_file(task_result_path(&manifest, "m000000", "site")).unwrap();
        // 引擎记录里没有候选时，缺失结果文件才是 unavailable；有候选记录时以引擎记录为准。
        let without_candidates = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[],
            Some(("m000001", 0.4)),
        );
        let missing = study_evidence(&root, &manifest, Some(&without_candidates)).unwrap();
        assert_eq!(
            missing["comparison"]["baseline"]["calibration"]["status"],
            "unavailable"
        );
        assert_eq!(missing["comparison"]["relation"]["calibration"], "unknown");
        assert!(!missing["missing"].as_array().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 验证未配置与验证已配置但不可用是两种证据，不能都报成"没有验证"。
    #[test]
    fn evidence_distinguishes_unconfigured_and_unavailable_validation() {
        let root = evidence_root("validation");
        let mut manifest = evidence_manifest(&root);
        manifest.spec.targets[0].validation_from = None;
        manifest.spec.targets[0].validation_to = None;
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000001", false, 1, 1, &[Some(0.2)])],
        );
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.8)], &[], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.4)], &[], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[("m000000", Some(0.8), None), ("m000001", Some(0.4), None)],
            Some(("m000001", 0.4)),
        );
        let unconfigured = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(unconfigured["validation"]["status"], "not_configured");
        assert_eq!(unconfigured["validation"]["configured"], false);
        assert!(unconfigured["validation"]["reasons"]
            .to_string()
            .contains("no validation windows"));
        assert_eq!(
            unconfigured["comparison"]["relation"]["validation"],
            "unknown"
        );

        let mut configured = evidence_manifest(&root);
        configured.spec.targets[0].validation_from = Some(10);
        configured.spec.targets[0].validation_to = Some(20);
        write_evidence_result(&configured, "m000000", &[("Qle", 0.8)], &[], &[]);
        write_evidence_result(
            &configured,
            "m000001",
            &[("Qle", 0.4)],
            &[],
            &[("Qle", "validation window has no paired values")],
        );
        let unavailable = study_evidence(&root, &configured, Some(&state)).unwrap();
        assert_eq!(unavailable["validation"]["configured"], true);
        assert!(
            ["unavailable", "partial"].contains(
                &unavailable["validation"]["status"]
                    .as_str()
                    .unwrap_or_default()
            ),
            "{unavailable}"
        );
        assert!(unavailable["validation"]["reasons"]
            .to_string()
            .contains("validation window has no paired values"));
        assert_eq!(
            unavailable["comparison"]["best"]["validation"]["status"],
            "unavailable"
        );
        assert!(!unavailable["missing"].as_array().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 运行中与不确定性分析都没有 best；原因要分开报，不能编造一个最优。
    #[test]
    fn evidence_reports_running_uncertainty_and_partial_failure_without_best() {
        let root = evidence_root("no-best");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        let running = evidence_state(&manifest, StudyStatus::Running, &["m000000"], &[], None);
        let evidence = study_evidence(&root, &manifest, Some(&running)).unwrap();
        assert!(evidence["selection"]["best_member"].is_null());
        assert_eq!(evidence["selection"]["no_best_reason"]["code"], "running");
        assert_eq!(evidence["selection"]["provisional"], true);
        assert_eq!(evidence["execution"]["state"], "running");

        let mut uncertainty_manifest = manifest.clone();
        uncertainty_manifest.spec.kind = StudyKind::Uncertainty;
        uncertainty_manifest.spec.method = StudyMethod::Lhs;
        uncertainty_manifest.spec.targets[0].validation_from = None;
        uncertainty_manifest.spec.targets[0].validation_to = None;
        let complete = evidence_state(
            &uncertainty_manifest,
            StudyStatus::Completed,
            &["m000000"],
            &[("m000000", Some(0.8), None)],
            None,
        );
        let uncertainty = study_evidence(&root, &uncertainty_manifest, Some(&complete)).unwrap();
        assert!(uncertainty["selection"]["best_member"].is_null());
        assert_eq!(
            uncertainty["selection"]["no_best_reason"]["code"],
            "uncertainty"
        );
        assert!(!uncertainty["selection"]["provisional"].as_bool().unwrap());

        let mut failed = evidence_state(
            &manifest,
            StudyStatus::CompletedWithFailures,
            &["m000000", "m000001"],
            &[("m000001", None, None)],
            None,
        );
        set_evidence_task_status(
            &mut failed,
            "m000001",
            "site",
            TaskStatus::Failed,
            "engine exit 1",
        );
        let partial = study_evidence(&root, &manifest, Some(&failed)).unwrap();
        assert!(partial["selection"]["best_member"].is_null());
        assert_eq!(
            partial["selection"]["no_best_reason"]["code"],
            "no_feasible_member_after_failures"
        );
        assert_eq!(partial["execution"]["partial_failures"], true);
        assert_eq!(partial["execution"]["tasks"]["failed"], 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 参数取全部已记录的代（samples/g*.csv）；Log 尺度、带索引的键与基线缺失的维度都要正确。
    #[test]
    fn evidence_parameters_use_all_generations_log_scale_and_indexed_keys() {
        let root = evidence_root("parameters");
        let mut manifest = tuning_manifest(&root);
        manifest.spec.parameters = vec![
            super::super::spec::ParameterSpec {
                name: "DEF_TUNING_CNFAC".into(),
                parameter_id: None,
                scope_instance: None,
                sample_min: 0.1,
                sample_max: 0.9,
                scale: Some(ScaleSpec::Linear),
            },
            super::super::spec::ParameterSpec {
                name: "DEF_TUNING_ZLND".into(),
                parameter_id: None,
                scope_instance: None,
                sample_min: 0.001,
                sample_max: 0.1,
                scale: Some(ScaleSpec::Log),
            },
            super::super::spec::ParameterSpec {
                name: "DEF_PFT_VMAX25".into(),
                parameter_id: Some("pft:DEF_PFT_VMAX25".into()),
                scope_instance: Some(super::super::spec::ParameterScopeInstance {
                    kind: super::super::spec::ParameterScopeKind::PftType,
                    scheme: None,
                    index: Some(1),
                    type_name: None,
                }),
                sample_min: 10.0,
                sample_max: 100.0,
                scale: Some(ScaleSpec::Linear),
            },
        ];
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5), Some(0.01), None])],
        );
        // 后续代：samples/g000001.csv 里的成员必须能被找到。
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[(
                "m000005",
                false,
                1,
                5,
                &[Some(0.895), Some(0.001), Some(55.0)],
            )],
        );
        write_evidence_result(&manifest, "m000005", &[("Qle", 0.3)], &[("Qle", 0.4)], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000005"],
            &[("m000005", Some(0.3), None)],
            Some(("m000005", 0.3)),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        let parameters = &evidence["parameters"];
        assert_eq!(parameters["status"], "recorded");
        let dims = parameters["dims"].as_array().unwrap();
        let scale = |key: &str| {
            dims.iter()
                .find(|dim| dim["key"] == key)
                .map(|dim| dim["scale"].as_str().unwrap().to_owned())
                .unwrap()
        };
        assert_eq!(scale("DEF_TUNING_CNFAC"), "linear");
        assert_eq!(scale("DEF_TUNING_ZLND"), "log");
        let value = |side: &str, key: &str| {
            parameters[side]["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["key"] == key)
                .cloned()
                .unwrap()
        };
        assert_eq!(value("best", "DEF_TUNING_CNFAC")["at_bound"], true);
        assert_eq!(value("best", "DEF_TUNING_CNFAC")["bound_side"], "upper");
        assert_eq!(value("best", "DEF_TUNING_ZLND")["position_scale"], "log");
        assert_eq!(
            value("best", "DEF_TUNING_ZLND")["position"].as_f64(),
            Some(0.0)
        );
        assert_eq!(value("best", "DEF_TUNING_ZLND")["at_bound"], true);
        let indexed = value("best", "DEF_PFT_VMAX25(2)");
        assert!((indexed["position"].as_f64().unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(indexed["at_bound"], false);
        assert_eq!(
            value("baseline", "DEF_PFT_VMAX25(2)")["status"],
            "unavailable"
        );
        assert!(value("baseline", "DEF_PFT_VMAX25(2)")["reason"]
            .as_str()
            .unwrap()
            .contains("no value"));
        assert_eq!(parameters["best"]["hybrid"]["status"], "not_configured");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn evidence_identity_fixture(label: &str) -> (PathBuf, Manifest, StudyState) {
        let root = evidence_root(label);
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000"],
            &[("m000000", Some(0.8), None)],
            None,
        );
        (root, manifest, state)
    }

    fn write_evidence_remote_record(study: &Path, provenance: serde_json::Value) {
        fs::write(
            study.join(".colm-remote-study.json"),
            json!({
                "host": "server",
                "root": "/data/colm",
                "job": "job-1",
                "remote_project": "/data/colm/s/proj",
                "engine": "engine-1",
                "submitted_at": 5,
                "scheduler": "bare",
                "provenance": provenance,
            })
            .to_string(),
        )
        .unwrap();
    }

    fn write_evidence_pinned(study: &Path) {
        let pinned = json!({
            "name": "modified",
            "head": "abc123",
            "base_commit": "base",
            "note": "",
            "based_on": null,
            "touches": {"engine": true, "kernel": false},
            "cli": study.join("version/bin/colm-cli"),
            "kernel": study.join("version/kernel"),
            "pinned_unix": 0,
            "model_revision": {
                "schema": 1,
                "version": {"id": "workspace-x", "revision": 3},
                "name": "workspace-x#3",
                "note": "",
                "parent": null,
                "code": {
                    "kind": "workspace",
                    "workspace": "workspace-x",
                    "commit": "abc123",
                    "source_sha256": "a".repeat(64),
                },
                "overlay": {},
                "overlay_sha256": "b".repeat(64),
                "fingerprint": "c".repeat(12),
                "created_unix": 0,
                "applicability": {},
                "derived_from": null,
            },
        });
        fs::write(study.join("model-version.json"), pinned.to_string()).unwrap();
    }

    /// R05：没有 model-version.json 不能直接判官方版。
    /// 无记录 → unknown；远程记录明确 official → official；远程记录写明 workspace → 用其 name/head。
    #[test]
    fn evidence_identity_without_a_pin_uses_the_remote_record_not_official() {
        let (root, manifest, state) = evidence_identity_fixture("identity-remote");

        // 1) 既没有 pin 也没有远程记录：无法区分官方版与修改版，明确 unknown。
        let unknown = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(
            unknown["model_identity"]["study_record"]["status"],
            "unknown"
        );
        assert_eq!(unknown["model_identity"]["study_record"]["source"], "none");
        assert!(unknown["model_identity"]["study_record"]["reason"]
            .as_str()
            .unwrap()
            .contains("cannot be established"));
        assert_eq!(unknown["remote"]["status"], "none");
        assert!(!unknown.to_string().contains("\"official\""));

        // 2) 远程记录明确写 official 才算官方版。
        write_evidence_remote_record(
            &root,
            json!({"version": {"kind": "official", "label": "官方版"}}),
        );
        let official = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(
            official["model_identity"]["study_record"]["status"],
            "official"
        );
        assert_eq!(
            official["model_identity"]["study_record"]["source"],
            "remote_record"
        );
        assert!(official["model_identity"]["study_record"]["reason"]
            .as_str()
            .unwrap()
            .contains("declares the official version"));
        assert_eq!(official["remote"]["status"], "recorded");

        // 3) 远程记录写明 workspace：回退出真实 name/head，不能归因官方模型。
        //    记录里带 password SSH 设置：读身份是本地元数据，不要求凭据、不初始化全局认证。
        fs::write(
            root.join(".colm-remote-study.json"),
            json!({
                "host": "server",
                "root": "/data/colm",
                "job": "job-1",
                "remote_project": "/data/colm/s/proj",
                "engine": "engine-1",
                "submitted_at": 5,
                "scheduler": "bare",
                "ssh_auth": {
                    "host": "server", "username": "researcher", "port": 22,
                    "auth": "password", "identity_file": ""
                },
                "provenance": {"version": {
                    "kind": "workspace", "name": "snow", "label": "snow",
                    "head": "0123456789abcdef", "base_commit": "base-1",
                }},
            })
            .to_string(),
        )
        .unwrap();
        let workspace = study_evidence(&root, &manifest, Some(&state)).unwrap();
        let record = &workspace["model_identity"]["study_record"];
        assert_eq!(record["status"], "workspace");
        assert_eq!(record["source"], "remote_record");
        assert_eq!(record["name"], "snow");
        assert_eq!(record["head"], "0123456789abcdef");
        assert_eq!(record["base_commit"], "base-1");
        assert!(record["reason"]
            .as_str()
            .unwrap()
            .contains("no frozen revision id"));
        assert!(
            !workspace.to_string().contains("\"status\":\"official\""),
            "{workspace}"
        );
        // 凭据设置不影响本地身份解析；远程记录校验失败只影响 remote 段。
        assert_ne!(workspace["remote"]["status"], "recorded");

        std::fs::remove_dir_all(root).unwrap();
    }

    /// R05：坏/不完整 remote record 必须明确 unknown，不能静默官方化。
    #[test]
    fn evidence_identity_bad_remote_record_is_unknown_not_official() {
        let (root, manifest, state) = evidence_identity_fixture("identity-broken");

        fs::write(root.join(".colm-remote-study.json"), "broken json").unwrap();
        let broken = study_evidence(&root, &manifest, Some(&state)).unwrap();
        let record = &broken["model_identity"]["study_record"];
        assert_eq!(record["status"], "unknown");
        assert_eq!(record["source"], "remote_record");
        assert!(record["reason"]
            .as_str()
            .unwrap()
            .contains("cannot parse captured remote Study version"));
        assert_ne!(record["status"], "official");

        for (provenance, expected) in [
            (json!({}), "no provenance.version"),
            (
                json!({"version": {"kind": "workspace", "name": "snow"}}),
                "no commit",
            ),
            (json!({"version": {"kind": "unknown"}}), "no supported kind"),
        ] {
            write_evidence_remote_record(&root, provenance);
            let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
            let record = &evidence["model_identity"]["study_record"];
            assert_eq!(record["status"], "unknown", "{evidence}");
            assert_eq!(record["source"], "remote_record");
            assert!(
                record["reason"].as_str().unwrap().contains(expected),
                "{evidence}"
            );
            assert_ne!(record["status"], "official");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 有 model-version.json 时仍用 pinned 身份，且优先于 remote record。
    #[test]
    fn evidence_identity_prefers_the_pinned_file_over_the_remote_record() {
        let (root, manifest, state) = evidence_identity_fixture("identity-pinned");
        write_evidence_remote_record(
            &root,
            json!({"version": {"kind": "workspace", "name": "snow", "head": "0123456789abcdef"}}),
        );
        write_evidence_pinned(&root);
        let frozen = study_evidence(&root, &manifest, Some(&state)).unwrap();
        let record = &frozen["model_identity"]["study_record"];
        assert_eq!(record["status"], "pinned");
        assert_eq!(record["source"], "model_version_file");
        assert_eq!(record["name"], "modified");
        assert_eq!(record["version"]["id"], "workspace-x");
        assert_eq!(record["version"]["revision"], 3);
        assert_eq!(record["code_source_sha256"], "a".repeat(64));
        // 当前选择留给助手补；CLI 不自称知道界面选了什么。
        assert!(frozen["model_identity"]["current_selection"].is_null());
        assert_eq!(frozen["model_identity"]["comparison"]["status"], "unknown");

        // 坏的 pin：明确 unknown，不退回 remote record 猜身份。
        fs::write(root.join("model-version.json"), "not json").unwrap();
        let unknown = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(
            unknown["model_identity"]["study_record"]["status"],
            "unknown"
        );
        assert_eq!(
            unknown["model_identity"]["study_record"]["source"],
            "model_version_file"
        );
        assert!(!unknown["model_identity"]["study_record"]["reason"]
            .as_str()
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 目标/参数/警告很多时证据仍须是合法、有界的 JSON，并显式标出被裁掉。
    #[test]
    fn evidence_stays_within_the_status_budget() {
        let root = evidence_root("budget");
        let mut manifest = tuning_manifest(&root);
        manifest.spec.parameters = (0..40)
            .map(|index| super::super::spec::ParameterSpec {
                name: format!("DEF_TUNING_LONG_PARAMETER_NAME_{index:02}"),
                parameter_id: None,
                scope_instance: None,
                sample_min: 0.0,
                sample_max: 1.0,
                scale: Some(ScaleSpec::Linear),
            })
            .collect();
        let base_target = manifest.spec.targets[0].clone();
        manifest.spec.targets = (0..12)
            .map(|index| {
                let mut target = base_target.clone();
                target.key = format!("diagnostic_variable_number_{index:02}");
                target.variable = target.key.clone();
                target
            })
            .collect();
        let mut state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000"],
            &[("m000000", Some(0.8), None)],
            None,
        );
        state.warnings = (0..20)
            .map(|index| format!("warning {index}: {}", "x".repeat(400)))
            .collect();
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        let bytes = evidence_bytes(&evidence);
        assert!(bytes <= EVIDENCE_BUDGET_BYTES, "{bytes}");
        assert_eq!(evidence["truncated"], true, "bytes {bytes}");
        for key in [
            "study",
            "execution",
            "selection",
            "objective",
            "comparison",
            "validation",
            "parameters",
            "model_identity",
            "verification",
            "remote",
            "source_paths",
            "warnings",
            "missing",
        ] {
            assert!(evidence.get(key).is_some(), "missing evidence key {key}");
        }
        assert!(serde_json::from_str::<serde_json::Value>(&evidence.to_string()).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 基线按样本里的 baseline 标记定位，不按 m000000 编号猜。
    #[test]
    fn evidence_locates_baseline_by_flag_not_member_number() {
        let root = evidence_root("baseline-flag");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000009", true, 0, 9, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000010", false, 1, 10, &[Some(0.2)])],
        );
        write_evidence_result(&manifest, "m000009", &[("Qle", 0.8)], &[("Qle", 0.6)], &[]);
        write_evidence_result(&manifest, "m000010", &[("Qle", 0.4)], &[("Qle", 0.9)], &[]);
        let mut state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000009", "m000010"],
            &[
                ("m000009", Some(0.8), Some(0.6)),
                ("m000010", Some(0.4), Some(0.9)),
            ],
            Some(("m000010", 0.4)),
        );
        state
            .warnings
            .push("overfitting: calibration improved while validation became worse".into());
        state
            .warnings
            .push("bound: DEF_TUNING_CNFAC upper 0.895".into());
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(evidence["comparison"]["baseline"]["member"], "m000009");
        assert_eq!(
            evidence["comparison"]["baseline"]["calibration"]["score"].as_f64(),
            Some(0.8)
        );
        assert_eq!(
            evidence["comparison"]["baseline"]["validation"]["score"].as_f64(),
            Some(0.6)
        );
        assert_eq!(
            evidence["comparison"]["relation"]["calibration"],
            "lower_loss"
        );
        assert_eq!(
            evidence["comparison"]["relation"]["validation"],
            "higher_loss"
        );
        // 基线侧不挂 overfitting/bound 警告；最优侧挂。
        assert!(evidence["comparison"]["baseline"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(evidence["comparison"]["best"]["warnings"]
            .to_string()
            .contains("overfitting"));
        // feasible 统计排除的是标记出来的基线，不是编号 m000000。
        assert_eq!(
            evidence["selection"]["candidates"]["feasible_non_baseline"],
            1
        );
        assert_eq!(evidence["parameters"]["baseline"]["status"], "recorded");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 样本里没有任何 baseline 标记：基线明确 unavailable，不退回 m000000、不编造分数。
    #[test]
    fn evidence_reports_unavailable_baseline_without_a_baseline_marker() {
        let root = evidence_root("no-baseline");
        let manifest = evidence_manifest(&root);
        // 编号仍是 m000000，但样本没有 baseline=true：它不能冒充基线。
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", false, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000001", false, 1, 1, &[Some(0.2)])],
        );
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.8)], &[("Qle", 0.6)], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.4)], &[("Qle", 0.9)], &[]);
        let state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[
                ("m000000", Some(0.8), Some(0.6)),
                ("m000001", Some(0.4), Some(0.9)),
            ],
            Some(("m000001", 0.4)),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert!(evidence["comparison"]["baseline"]["member"].is_null());
        assert_eq!(evidence["comparison"]["baseline"]["status"], "unavailable");
        assert_eq!(
            evidence["comparison"]["baseline"]["calibration"]["status"],
            "unavailable"
        );
        assert!(
            evidence["comparison"]["baseline"]["calibration"]["score"].is_null(),
            "no fabricated baseline score: {evidence}"
        );
        assert!(evidence["comparison"]["baseline"]["calibration"]["reason"]
            .as_str()
            .unwrap()
            .contains("no baseline member"));
        // 配置了验证窗口，但没有基线可比：unavailable，不是 not_configured。
        assert_eq!(
            evidence["comparison"]["baseline"]["validation"]["status"],
            "unavailable"
        );
        assert_eq!(evidence["validation"]["configured"], true);
        assert_ne!(evidence["validation"]["status"], "not_configured");
        assert_eq!(evidence["comparison"]["relation"]["calibration"], "unknown");
        assert_eq!(evidence["comparison"]["relation"]["validation"], "unknown");
        assert!(evidence["missing"]
            .to_string()
            .contains("no baseline member is recorded"));
        // 参数侧也不按编号退回：baseline 明确 unavailable。
        assert_eq!(evidence["parameters"]["baseline"]["status"], "unavailable");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn evidence_engine_candidate(
        calibration: Option<f64>,
        validation: Option<f64>,
        feasible: bool,
        reason: Option<&str>,
    ) -> CandidateState {
        CandidateState {
            generation: 0,
            feasible,
            calibration,
            validation,
            reason: reason.map(str::to_owned),
        }
    }

    /// 验证期可用性以 state.candidates 为准：runner 判成不可用时证据不能对结果文件重算出 recorded。
    #[test]
    fn evidence_prefers_engine_candidate_availability_over_result_files() {
        let root = evidence_root("candidate-authority");
        let manifest = evidence_manifest(&root);
        write_evidence_sample_file(
            &manifest,
            "g000000.csv",
            &[("m000000", true, 0, 0, &[Some(0.5)])],
        );
        write_evidence_sample_file(
            &manifest,
            "g000001.csv",
            &[("m000001", false, 1, 1, &[Some(0.2)])],
        );
        // 结果文件里两期都有完整行；引擎记录却判定 calibration 不可行、validation 不可用。
        write_evidence_result(&manifest, "m000000", &[("Qle", 0.8)], &[("Qle", 0.6)], &[]);
        write_evidence_result(&manifest, "m000001", &[("Qle", 0.4)], &[("Qle", 0.2)], &[]);
        let mut state = evidence_state(
            &manifest,
            StudyStatus::Completed,
            &["m000000", "m000001"],
            &[],
            Some(("m000001", 0.4)),
        );
        state.candidates.insert(
            "m000000".into(),
            evidence_engine_candidate(
                None,
                None,
                false,
                Some("baseline required target set is incomplete"),
            ),
        );
        state.candidates.insert(
            "m000001".into(),
            evidence_engine_candidate(Some(0.35), None, true, None),
        );
        state.warnings.push(
            "m000001 / site validation unavailable: model-observation support differs from baseline"
                .into(),
        );
        let evidence = study_evidence(&root, &manifest, Some(&state)).unwrap();
        // calibration：引擎记录没有分数 → unavailable 且用 candidate.reason，不从文件硬算 0.4。
        assert_eq!(
            evidence["comparison"]["baseline"]["calibration"]["status"],
            "unavailable"
        );
        assert!(evidence["comparison"]["baseline"]["calibration"]["score"].is_null());
        assert!(evidence["comparison"]["baseline"]["calibration"]["reason"]
            .as_str()
            .unwrap()
            .contains("required target set is incomplete"));
        // 引擎记录的分数优先于文件聚合值（文件是 0.4）。
        assert_eq!(
            evidence["comparison"]["best"]["calibration"]["score"].as_f64(),
            Some(0.35)
        );
        // validation：candidate.validation=None → 不得报 recorded；reason 取 member 相关 warning。
        assert!(
            ["unavailable", "partial"].contains(
                &evidence["comparison"]["best"]["validation"]["status"]
                    .as_str()
                    .unwrap_or_default()
            ),
            "{evidence}"
        );
        assert!(evidence["comparison"]["best"]["validation"]["score"].is_null());
        assert!(evidence["comparison"]["best"]["validation"]["reason"]
            .as_str()
            .unwrap()
            .contains("support differs from baseline"));
        assert_ne!(evidence["validation"]["status"], "available");
        assert!(evidence["validation"]["reasons"]
            .to_string()
            .contains("support differs from baseline"));
        assert!(!evidence["missing"].as_array().unwrap().is_empty());

        // 反向：candidate.validation 有值且与文件聚合值不同 → 用 candidate 的值，target 明细仍来自文件。
        state.candidates.insert(
            "m000000".into(),
            evidence_engine_candidate(Some(0.8), Some(0.65), true, None),
        );
        state.candidates.insert(
            "m000001".into(),
            evidence_engine_candidate(Some(0.35), Some(0.55), true, None),
        );
        state.warnings.clear();
        let recorded = study_evidence(&root, &manifest, Some(&state)).unwrap();
        assert_eq!(recorded["validation"]["status"], "available");
        assert_eq!(
            recorded["comparison"]["baseline"]["validation"]["score"].as_f64(),
            Some(0.65)
        );
        assert_eq!(
            recorded["comparison"]["best"]["validation"]["score"].as_f64(),
            Some(0.55)
        );
        assert_eq!(
            recorded["comparison"]["best"]["validation"]["targets"][0]["loss"].as_f64(),
            Some(0.2)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
