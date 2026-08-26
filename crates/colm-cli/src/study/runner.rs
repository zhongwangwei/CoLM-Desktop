//! Authoritative local Study scheduler and scientific result writer.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

use anyhow::{bail, Context, Result};
use colm_namelist::Value;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::science::{ObjectiveMetric, ObjectiveScore, ObjectiveTerm};
use super::spec::{Manifest, MemberPlan, StudyKind, StudyMethod, StudySpec, TargetSpec};
use super::state::{CandidateState, ProcessIdentity, StudyState, StudyStatus, TaskStatus};
use crate::{Layout, MetricsRequest, RunNotice, VarMetrics};

pub struct RunOptions<'a> {
    pub kernel_dir: &'a Path,
    pub jobs: usize,
    pub stream: bool,
    pub retry_failed: bool,
}

struct StudyRunLock {
    path: PathBuf,
    stop: mpsc::Sender<()>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
}

const RUN_LOCK_HEARTBEAT_SECONDS: u64 = 10;
const RUN_LOCK_STALE_SECONDS: i64 = 60;

impl StudyRunLock {
    fn acquire(study_dir: &Path) -> Result<Self> {
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
        if let Err(error) = (|| -> Result<()> {
            file.write_all(&serde_json::to_vec(&identity)?)?;
            file.sync_all()?;
            Ok(())
        })() {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let heartbeat_path = path.clone();
        let (stop, stopped) = mpsc::channel();
        // ponytail: MSRV 1.85 has no portable std file lock; this short lease
        // prevents confirmed crash recovery from deleting a live scheduler lock.
        let heartbeat = std::thread::spawn(move || {
            let mut identity = identity;
            while matches!(
                stopped.recv_timeout(std::time::Duration::from_secs(RUN_LOCK_HEARTBEAT_SECONDS)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                identity.heartbeat_unix = unix_now();
                let Ok(bytes) = serde_json::to_vec(&identity) else {
                    break;
                };
                if fs::write(&heartbeat_path, bytes).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            path,
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
        let _ = fs::remove_file(&self.path);
    }
}

fn clear_stale_run_lock(study_dir: &Path) -> Result<()> {
    let path = study_dir.join("run.lock");
    let Some(owner) = run_lock_owner(&path)? else {
        return Ok(());
    };
    if unix_now().saturating_sub(owner.heartbeat_unix) <= RUN_LOCK_STALE_SECONDS {
        bail!("Study scheduler PID {} is still running", owner.pid);
    }
    fs::remove_file(&path).with_context(|| format!("cannot remove stale {}", path.display()))
}

fn run_lock_owner(path: &Path) -> Result<Option<ProcessIdentity>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    serde_json::from_slice(&bytes)
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
    let kernel_macros = colm_kernel::Kernel::open(&resolved)?.manifest.macros;
    let parameter_names = spec
        .parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect::<Vec<_>>();
    for (_, case) in &sites {
        colm_case::tuning::validate_case_parameter_activity(
            &case.join("case.nml"),
            &parameter_names,
            &kernel_macros,
        )?;
    }
    if spec.kind == StudyKind::Uncertainty {
        for (site, case) in &sites {
            for output in &spec.outputs {
                let (_, values, _) =
                    model_series(case, output, spec.analysis_from, spec.analysis_to)?;
                if !values.iter().any(|value| value.is_finite()) {
                    bail!("baseline output {output} has no finite values for {site}");
                }
            }
        }
        return Ok(());
    }
    let mut terms = Vec::new();
    for (site, case) in &sites {
        let observation = observation_for(&spec, &case_root, site)?;
        for target in spec
            .targets
            .iter()
            .filter(|target| target.site.as_deref().is_none_or(|wanted| wanted == site))
        {
            let result = target_result(case, &observation, site, target, false, sites.len())?;
            terms.push(ObjectiveTerm {
                metric: result.metric,
                value: result.value,
                observation_sd: result.observation_sd,
                weight: result.weight,
                pairs: result.pairs,
            });
            if target.validation_from.is_some() {
                let result = target_result(case, &observation, site, target, true, sites.len())?;
                terms.push(ObjectiveTerm {
                    metric: result.metric,
                    value: result.value,
                    observation_sd: result.observation_sd,
                    weight: result.weight,
                    pairs: result.pairs,
                });
            }
        }
    }
    match super::science::score_required(&terms, 0) {
        ObjectiveScore::Feasible(_) => Ok(()),
        ObjectiveScore::Infeasible(reason) => {
            bail!("baseline does not satisfy the frozen tuning targets: {reason}")
        }
    }
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

pub fn run(study_dir: &Path, options: RunOptions<'_>) -> Result<StudyState> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let _run_lock = StudyRunLock::acquire(&study_dir)?;
    let kernel_dir = colm_kernel::manifest::absolute(options.kernel_dir)?;
    let manifest = super::engine::status(&study_dir)?;
    let kernel = colm_kernel::Kernel::open(&kernel_dir)?;
    let kernel_identity = format!(
        "{} ({})",
        kernel.manifest.identity(),
        kernel.manifest.platform
    );
    let kernel_id = kernel.manifest.stage_fingerprint_identity();
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
        let frozen = colm_kernel::manifest::absolute(Path::new(frozen))?;
        if frozen != kernel_dir {
            bail!(
                "Study was created for kernel {}, not {}",
                frozen.display(),
                kernel_dir.display()
            );
        }
    }
    super::engine::verify_frozen_inputs(&manifest)?;
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
    let jobs = bounded_jobs(
        options.jobs,
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1),
    );
    emit(
        &study_dir,
        options.stream,
        serde_json::json!({"type":"study_started","kind":"study_started","study":manifest.id,"jobs":jobs}),
    )?;

    run_members(
        &study_dir,
        &manifest,
        &mut state,
        &checkpoint_dir,
        &kernel_dir,
        &kernel_id,
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
        if baseline_tasks_succeeded(&manifest, &state) {
            if let Err(error) = write_uncertainty_results(&manifest, &members, &mut state) {
                push_warning_once(
                    &mut state,
                    format!("uncertainty result aggregation failed: {error}"),
                );
                if state.status == StudyStatus::Completed {
                    state.status = StudyStatus::CompletedWithFailures;
                }
            }
        } else {
            push_warning_once(
                &mut state,
                "uncertainty results are unavailable until every baseline task succeeds".into(),
            );
        }
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

pub(super) fn ensure_scheduler_idle(study_dir: &Path) -> Result<()> {
    clear_stale_run_lock(study_dir)
}

/// GUI 已确认对应进程树退出后再落盘；PID 必须对上，避免迟到的取消请求
/// 误关掉后来启动的调度器。
pub fn finalize_cancel(study_dir: &Path, expected_pid: u32) -> Result<StudyState> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let lock_path = study_dir.join("run.lock");
    let checkpoint_dir = study_dir.join("checkpoints/state");
    let mut state = super::checkpoint::load_latest::<StudyState>(&checkpoint_dir)?
        .map(|loaded| loaded.payload)
        .with_context(|| format!("{} has no Study state checkpoint", study_dir.display()))?;
    let had_lock = match run_lock_owner(&lock_path)? {
        Some(owner) => {
            if owner.pid != expected_pid {
                bail!(
                    "Study scheduler changed while cancelling: expected PID {expected_pid}, found {}",
                    owner.pid
                );
            }
            if scheduler_process_alive(owner.pid)? {
                bail!("Study scheduler PID {} is still running", owner.pid);
            }
            true
        }
        None => {
            if state
                .tasks
                .values()
                .any(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Evaluating))
            {
                bail!("Study scheduler lock is missing while active tasks remain");
            }
            false
        }
    };
    let mut changed = false;
    for task in state.tasks.values_mut() {
        if matches!(
            task.status,
            TaskStatus::Pending
                | TaskStatus::Materialized
                | TaskStatus::Queued
                | TaskStatus::Running
                | TaskStatus::Evaluating
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

fn push_warning_once(state: &mut StudyState, warning: String) {
    if !state.warnings.contains(&warning) {
        state.warnings.push(warning);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_members(
    study_dir: &Path,
    manifest: &Manifest,
    state: &mut StudyState,
    checkpoint_dir: &Path,
    kernel_dir: &Path,
    kernel_id: &str,
    jobs: usize,
    stream: bool,
    retry_failed: bool,
    generation: Option<usize>,
) -> Result<()> {
    let members = super::generation::load_members(manifest)?;
    let generation_members = generation.map(|generation| {
        members
            .iter()
            .filter(|member| member.generation == generation && !member.baseline)
            .map(|member| member.id.clone())
            .collect::<BTreeSet<_>>()
    });
    let mut runnable = VecDeque::new();
    for task in state.tasks.values() {
        if generation_members
            .as_ref()
            .is_some_and(|ids| !ids.contains(&task.member))
        {
            continue;
        }
        let stale_success = task.status == TaskStatus::Succeeded
            && !crate::case_is_current(Path::new(&task.case_dir), kernel_id).unwrap_or(false);
        let allowed = matches!(
            task.status,
            TaskStatus::Pending | TaskStatus::Materialized | TaskStatus::Queued
        ) || stale_success
            || (retry_failed
                && matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted));
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
                let result = crate::run_case(
                    &case_dir,
                    &kernel_dir,
                    stream,
                    false,
                    None,
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
                            RunNotice::Log { stage, line } => WorkerEvent::Notice {
                                member: event_member.clone(),
                                site: event_site.clone(),
                                kind: "task_log".into(),
                                stage: stage.into(),
                                line: Some(line.into()),
                                ok: None,
                            },
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
                );
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

    if super::state::cancel_requested(study_dir) {
        cancel_unstarted(state);
    }
    state.finish_status();
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
    super::checkpoint::write_next(checkpoint_dir, state)?;
    Ok(())
}

fn evaluate_task(manifest: &Manifest, member: &str, site: &str, case: &Path) -> Result<TaskResult> {
    let mut result = TaskResult {
        member: member.into(),
        site: site.into(),
        outputs: Vec::new(),
        calibration: Vec::new(),
        validation: Vec::new(),
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
                result.outputs.push(OutputSummary {
                    variable: variable.clone(),
                    units,
                    count: finite.len(),
                    mean: finite.iter().sum::<f64>() / finite.len() as f64,
                    min: finite.iter().copied().fold(f64::INFINITY, f64::min),
                    max: finite.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                });
            }
        }
        StudyKind::Tuning => {
            let observation = PathBuf::from(
                manifest
                    .spec
                    .observations
                    .get(site)
                    .with_context(|| format!("missing frozen observation for {site}"))?,
            );
            for target in manifest
                .spec
                .targets
                .iter()
                .filter(|target| target.site.as_deref().is_none_or(|wanted| wanted == site))
            {
                result.calibration.push(target_result(
                    case,
                    &observation,
                    site,
                    target,
                    false,
                    manifest.spec.base_cases.len(),
                )?);
                if target.validation_from.is_some() {
                    result.validation.push(target_result(
                        case,
                        &observation,
                        site,
                        target,
                        true,
                        manifest.spec.base_cases.len(),
                    )?);
                }
            }
            if result.calibration.is_empty() {
                bail!("parameter tuning has no target covering site {site}");
            }
        }
    }
    write_json(&task_result_path(manifest, member, site), &result)?;
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
    let rows = crate::compute_metric_rows(MetricsRequest {
        case,
        obs_path: observation,
        spinup: 0,
        json: true,
        corrected: false,
        summary_only: true,
        pair_vars: vec![target.variable.clone()],
        pair_max_points: None,
        from: Some(from),
        to: Some(to),
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
    let value = metric_value(&row, target.metric);
    let weight = if target.site.is_none() {
        target.weight / site_count.max(1) as f64
    } else {
        target.weight
    };
    let term = ObjectiveTerm {
        metric: target.metric,
        value,
        observation_sd: Some(row.obs_sd),
        weight,
        pairs: row.n,
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
        pairs: row.n,
        value,
        observation_sd: Some(row.obs_sd),
        loss,
        model_mean: row.model_mean,
        observation_mean: row.obs_mean,
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

fn fixed_score(results: &[TargetResult]) -> Option<f64> {
    if results.is_empty() {
        return None;
    }
    let weight = results.iter().map(|result| result.weight).sum::<f64>();
    (weight.is_finite() && weight > 0.0)
        .then(|| {
            results
                .iter()
                .map(|result| result.weight * result.loss)
                .sum::<f64>()
                / weight
        })
        .filter(|score| score.is_finite())
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
                        calibration.extend(result.calibration);
                        validation.extend(result.validation);
                    }
                    Err(error) if manifest.spec.kind == StudyKind::Tuning => {
                        reason = Some(error.to_string());
                        break;
                    }
                    Err(_) => {}
                }
            }
        }
        let calibration_score = fixed_score(&calibration);
        let validation_score = fixed_score(&validation);
        let feasible = reason.is_none()
            && (manifest.spec.kind == StudyKind::Uncertainty || calibration_score.is_some());
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
    state.best_member = best.as_ref().map(|best| best.0.clone());
    state.best_objective = best.map(|best| best.1);
    update_overfit_warning(state);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_de_generations(
    study_dir: &Path,
    manifest: &Manifest,
    state: &mut StudyState,
    checkpoint_dir: &Path,
    kernel_dir: &Path,
    kernel_id: &str,
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
        let before = state.best_objective;
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
                    .collect::<Vec<_>>();
                // Cross-field-invalid mutants fall back to their valid parent. This
                // is deterministic and keeps the generation barrier recoverable.
                if colm_case::tuning::validate_values(&named).is_ok() {
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
        let improved = match (before, state.best_objective) {
            (None, Some(_)) => true,
            (Some(before), Some(after)) => before - after >= manifest.spec.budget.min_improvement,
            _ => false,
        };
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
            if let Ok(result) = read_task_result(manifest, &member.id, site) {
                for output in result.outputs {
                    means.insert(
                        (member.id.clone(), site.clone(), output.variable),
                        output.mean,
                    );
                }
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
                    Ok(_) => state.warnings.push(format!(
                        "{} / {} / {} has a different time axis and was excluded",
                        member.id, site, variable
                    )),
                    Err(error) => state.warnings.push(format!(
                        "{} / {} / {} was excluded: {error}",
                        member.id, site, variable
                    )),
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
    write_importance(manifest, members, &means)?;
    write_member_table(manifest, members, state, &means)?;
    Ok(())
}

fn write_importance(
    manifest: &Manifest,
    members: &[MemberPlan],
    means: &BTreeMap<(String, String, String), f64>,
) -> Result<()> {
    let mut rows = Vec::new();
    for site in &manifest.spec.base_cases {
        for variable in &manifest.spec.outputs {
            for parameter in &manifest.spec.parameters {
                match manifest.spec.method {
                    StudyMethod::Lhs => {
                        let pairs = members
                            .iter()
                            .filter(|member| !member.baseline)
                            .filter_map(|member| {
                                means
                                    .get(&(member.id.clone(), site.clone(), variable.clone()))
                                    .map(|mean| (member.parameters[&parameter.name], *mean))
                            })
                            .collect::<Vec<_>>();
                        let (x, y): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
                        rows.push(serde_json::json!({
                            "site":site,"variable":variable,"parameter":parameter.name,
                            "method":"spearman","value":super::science::spearman(&x,&y),"n":x.len()
                        }));
                    }
                    StudyMethod::Oat => {
                        let baseline = members
                            .iter()
                            .find(|member| member.baseline)
                            .context("OAT Study has no baseline member")?;
                        let parameter_index = super::sample::sorted_parameter_names(&manifest.spec)
                            .iter()
                            .position(|name| name == &parameter.name)
                            .context("OAT parameter is missing from the frozen design")?;
                        let candidate_indices = [2 * parameter_index + 1, 2 * parameter_index + 2];
                        let changed = members
                            .iter()
                            .filter(|member| candidate_indices.contains(&member.candidate_index))
                            .collect::<Vec<_>>();
                        if changed.len() == 2 {
                            let mut changed = changed;
                            changed.sort_by(|a, b| {
                                a.parameters[&parameter.name]
                                    .total_cmp(&b.parameters[&parameter.name])
                            });
                            let low = changed[0];
                            let high = changed[1];
                            if let (Some(y_low), Some(y_high), Some(y0)) = (
                                means.get(&(low.id.clone(), site.clone(), variable.clone())),
                                means.get(&(high.id.clone(), site.clone(), variable.clone())),
                                means.get(&(baseline.id.clone(), site.clone(), variable.clone())),
                            ) {
                                let dx = high.parameters[&parameter.name]
                                    - low.parameters[&parameter.name];
                                rows.push(serde_json::json!({
                                    "site":site,"variable":variable,"parameter":parameter.name,
                                    "method":"oat_finite_difference_slope","value":(*y_high-*y_low)/dx,
                                    "asymmetry":(*y_high-*y0)-(*y0-*y_low),"n":2
                                }));
                            }
                        }
                    }
                    StudyMethod::DifferentialEvolution => {}
                }
            }
        }
    }
    write_json(
        &Path::new(&manifest.root).join("results/importance.json"),
        &rows,
    )
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
                csv.push_str(&member.parameters[name].to_string());
            }
            for variable in &manifest.spec.outputs {
                csv.push(',');
                if let Some(value) = means.get(&(member.id.clone(), site.clone(), variable.clone()))
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
    let results = Path::new(&manifest.root).join("results");
    write_json(&results.join("objectives.json"), &state.candidates)?;
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
    let mut metrics = String::from("member,site,period,target,variable,metric,weight,pairs,value,loss,model_mean,observation_mean\n");
    for task in state
        .tasks
        .values()
        .filter(|task| task.status == TaskStatus::Succeeded)
    {
        let Ok(result) = read_task_result(manifest, &task.member, &task.site) else {
            continue;
        };
        for row in result.calibration.iter().chain(&result.validation) {
            metrics.push_str(&format!(
                "{},{},{},{},{},{:?},{},{},{},{},{},{}\n",
                task.member,
                row.site,
                row.period,
                row.key,
                row.variable,
                row.metric,
                row.weight,
                row.pairs,
                row.value,
                row.loss,
                row.model_mean,
                row.observation_mean
            ));
        }
    }
    write_bytes(&results.join("metrics.csv"), metrics.as_bytes())
}

pub fn retry(study_dir: &Path, include_review: bool) -> Result<StudyState> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    if include_review {
        clear_stale_run_lock(&study_dir)?;
    }
    let _retry_lock = StudyRunLock::acquire(&study_dir)?;
    let checkpoint_dir = study_dir.join("checkpoints/state");
    let mut state = super::checkpoint::load_latest::<StudyState>(&checkpoint_dir)?
        .map(|loaded| loaded.payload)
        .with_context(|| format!("{} has no Study state checkpoint", study_dir.display()))?;
    for task in state.tasks.values_mut() {
        if matches!(task.status, TaskStatus::Failed | TaskStatus::Interrupted)
            || (include_review
                && matches!(
                    task.status,
                    TaskStatus::NeedsReview | TaskStatus::Running | TaskStatus::Evaluating
                ))
        {
            task.status = TaskStatus::Queued;
            task.reason = None;
            task.stage = None;
            task.objective = None;
            task.validation_objective = None;
            task.process = None;
        }
    }
    state.status = StudyStatus::Ready;
    super::state::resume(&study_dir)?;
    super::state::clear_cancel(&study_dir)?;
    super::checkpoint::write_next(&checkpoint_dir, &state)?;
    Ok(state)
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
    super::engine::verify_frozen_inputs(&manifest)?;
    let member = resolve_apply_member(&study_dir, &manifest, member_id)?;
    let case_root = study_case_root(&manifest)?;
    let values = member
        .parameters
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .collect::<Vec<_>>();
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
            let baseline = case_root.join(site);
            let before = sha256(&fs::read(baseline.join("case.nml"))?);
            let case_name = if manifest.spec.base_cases.len() == 1 {
                name.map(str::to_string)
                    .unwrap_or_else(|| format!("{site}-{member_id}"))
            } else {
                format!("{site}-tuned")
            };
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
    let existed = output.exists();
    if existed {
        fs::remove_dir(&output)?;
    }
    if let Err(error) = fs::rename(&stage, &output) {
        if existed {
            let _ = fs::create_dir(&output);
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(error).with_context(|| format!("cannot publish {}", output.display()));
    }
    Ok(destinations)
}

pub fn apply_preview(study_dir: &Path, member_id: &str) -> Result<Vec<ApplyPreviewRow>> {
    ensure_scheduler_idle(study_dir)?;
    let manifest = super::engine::status(study_dir)?;
    super::engine::verify_frozen_inputs(&manifest)?;
    let member = resolve_apply_member(study_dir, &manifest, member_id)?;
    let case_root = study_case_root(&manifest)?;
    let mut rows = Vec::new();
    for site in &manifest.spec.base_cases {
        let file = case_root.join(site).join("case.nml");
        let document = colm_namelist::parse(&fs::read_to_string(&file)?)?;
        for (field, value) in &member.parameters {
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

fn resolve_apply_member(
    study_dir: &Path,
    manifest: &Manifest,
    member_id: &str,
) -> Result<MemberPlan> {
    let member_id = if member_id == "best" {
        super::checkpoint::load_latest::<StudyState>(&study_dir.join("checkpoints/state"))?
            .and_then(|loaded| loaded.payload.best_member)
            .context("Study has no best member yet")?
    } else {
        member_id.to_string()
    };
    super::generation::load_members(manifest)?
        .into_iter()
        .find(|member| member.id == member_id)
        .with_context(|| format!("unknown Study member {member_id}"))
}

fn study_case_root(manifest: &Manifest) -> Result<PathBuf> {
    Ok(Path::new(&manifest.root)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .context("Study directory is not under <case-root>/.colm/studies")?
        .to_path_buf())
}

fn resolved_base_cases(case_root: &Path, spec: &StudySpec) -> Result<Vec<(String, PathBuf)>> {
    spec.base_cases
        .iter()
        .map(|raw| {
            let candidate = PathBuf::from(raw);
            let candidate = if candidate.join("case.nml").is_file() {
                candidate
            } else {
                case_root.join(raw)
            };
            let candidate = colm_kernel::manifest::absolute(&candidate)?;
            if candidate.parent() != Some(case_root) || !candidate.join("case.nml").is_file() {
                bail!(
                    "base case {} must be a direct child of {}",
                    candidate.display(),
                    case_root.display()
                );
            }
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
    serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("cannot parse {}", path.display()))
}

fn model_series(
    case: &Path,
    variable: &str,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<(Vec<i64>, Vec<f64>, Option<String>)> {
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
    if value.contains([',', '"', '\n']) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!crate::case_is_current(&dir, "test@kernel").unwrap());
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
        let dir = std::env::temp_dir().join(format!(
            "colm-study-retry-lock-{}-{}",
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
        stale.pid = u32::MAX;
        stale.heartbeat_unix = 0;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&stale).unwrap()).unwrap();
        let retried = retry(&dir, true).unwrap();
        let task = &retried.tasks["m000001/site"];
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.process.is_none());
        assert!(!dir.join("run.lock").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verified_gui_cancel_closes_active_tasks_and_rejects_a_new_scheduler() {
        assert!(scheduler_process_alive(std::process::id()).unwrap());
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
        running.process.as_mut().unwrap().pid = u32::MAX;
        let state = StudyState::new("s".into(), [running, queued]).unwrap();
        super::super::checkpoint::write_next(&checkpoint, &state).unwrap();
        let mut owner = supervisor_identity();
        owner.pid = u32::MAX;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&owner).unwrap()).unwrap();

        assert!(finalize_cancel(&dir, 9999).is_err());
        assert!(dir.join("run.lock").is_file());
        let cancelled = finalize_cancel(&dir, u32::MAX).unwrap();
        assert_eq!(cancelled.status, StudyStatus::Cancelled);
        assert!(cancelled.tasks.values().all(|task| {
            task.status == TaskStatus::Cancelled && task.stage.is_none() && task.process.is_none()
        }));
        assert!(!dir.join("run.lock").exists());
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

        let mut stale = supervisor_identity();
        stale.pid = u32::MAX;
        stale.heartbeat_unix = 0;
        std::fs::write(dir.join("run.lock"), serde_json::to_vec(&stale).unwrap()).unwrap();
        ensure_scheduler_idle(&dir).unwrap();
        assert!(!dir.join("run.lock").exists());
        std::fs::remove_dir_all(dir).unwrap();
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
        let manifest = super::super::engine::create(&root, &spec).unwrap();
        let study = PathBuf::from(&manifest.root);
        let state = run(
            &study,
            RunOptions {
                kernel_dir: &kernel,
                jobs: 2,
                stream: false,
                retry_failed: false,
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
        std::fs::remove_dir_all(root).unwrap();
    }

    fn preview_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "colm-study-preview-{}-{}",
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
        };
        std::fs::write(&spec_path, serde_json::to_string(&spec).unwrap()).unwrap();
        let manifest = super::super::engine::create(&root, &spec_path).unwrap();
        (root, PathBuf::from(manifest.root))
    }

    fn multi_site_apply_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "colm-study-apply-multi-{}-{}",
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
}
