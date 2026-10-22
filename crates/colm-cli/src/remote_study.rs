//! 在服务器上跑研究（界面重组第 5 阶段）。
//!
//! ```text
//! colm-cli remote-study-run    <本机研究> --host H --root R [--kernel 本机内核] [--map 本机=服务器]… [--upload-unmapped 1]
//!                              [--engine rust|fortran] [--preprocessors rust|fortran] [--jobs N] [--threads N]
//!                              [--model-version official|<修改版>] [--workspace-root DIR] [调度选项同 remote-run] [--dry-run 1]
//! colm-cli remote-study-status <本机研究>
//! colm-cli remote-study-fetch  <本机研究>
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
use colm_remote::sched;
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
    let file = Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "obs.nc".into());
    format!("{site}-{file}")
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
    let resources = resources_from(opts, threads.max(jobs as u32), 1, 0)?;
    // 路径尽量短：成员的重启文件路径要在 CoLM 的 256 字节以内（`<项目>/.colm/studies/<编号>/members/…`）。
    let remote_project = format!(
        "{root}/s/{}",
        short_hash(&format!("{}|{}", manifest.id, study_dir.display()))
    );

    // 1. 基础算例：改写路径后暂存；观测文件随研究上传。
    let staging = std::env::temp_dir().join(format!(
        "colm-remote-study-{}-{}",
        manifest.id,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&staging);
    let result = (|| -> Result<(Value, Vec<String>, u64)> {
        let mut unmapped = Vec::new();
        let mut remote_inputs = Vec::new();
        let mut bytes = 0;
        for base in &manifest.spec.base_cases {
            let staged = stage_case(
                &study::model::base_case(&manifest, base)?,
                &staging.join(base),
                &format!("{remote_project}/{base}"),
                &maps,
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
        std::fs::create_dir_all(staging.join("obs"))?;
        for (site, path) in spec.observations.iter_mut() {
            let name = remote_observation_name(site, path);
            bytes += std::fs::copy(&*path, staging.join("obs").join(&name))
                .with_context(|| format!("cannot copy the observations {path}"))?;
            *path = format!("{remote_project}/obs/{name}");
        }
        Ok((serde_json::to_value(spec)?, remote_inputs, bytes))
    })();
    let (mut spec, remote_inputs, bytes) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };

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
        resources,
        name: sched::job_name(&format!("study-{}", manifest.id)),
    };
    if dry_run {
        let _ = std::fs::remove_dir_all(&staging);
        println!(
            "{}",
            json!({
                "dry_run": true,
                "job_script": job::job_script(&root, "JOBID", &body, &job_spec)?,
                "remote_project": remote_project,
                "engine": engine.id(),
                "spec": spec,
            })
        );
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
            let _ = std::fs::remove_dir_all(&staging);
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
        let _ = std::fs::remove_dir_all(&staging);
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

fn verified_evidence(
    manifest: &study::spec::Manifest,
    record: &StudyRecord,
    remote_root: &str,
    local_state: &study::state::StudyState,
    evidence: &Value,
) -> Result<study::model::StudyInputIdentity> {
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
        crate::study_version::same_design(&manifest.spec, &remote_spec)?,
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

/// Reverify raw server outputs for every comparison, using the recorded job's fixed CLI.
pub(crate) fn verified_comparison(study_dir: &Path) -> Result<study::model::StudyInputIdentity> {
    let manifest = study::engine::status(study_dir)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let record = read_record(study_dir)?;
    ensure!(
        record.fetched_at.is_some(),
        "remote Study has not been fetched"
    );
    let ssh = Ssh::new(&record.host)?;
    ssh.run_ok(&cancel_identity_script(&record)?)?;
    let status = job::status(&ssh, &record.root, &record.job, 1)?;
    ensure!(
        matches!(status.state, job::State::Finished { exit_code: 0 }),
        "remote Study job did not finish successfully"
    );
    let remote_root = remote_study_root(&ssh, &record)?.context("remote job has no Study root")?;
    let evidence = server_comparison_evidence(&ssh, &record, &remote_root)?;
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study has no fetched checkpoint")?
    .payload;
    let identity = verified_evidence(&manifest, &record, &remote_root, &state, &evidence)?;
    verify_result_tree(&study_dir.join("results"), &evidence["results_sha256"])?;
    Ok(identity)
}

/// `remote-study-status`：作业状态，加上服务器上研究的进度（建好之后）。
pub(crate) fn cmd_status(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
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
    let entries = ["results", "samples", "checkpoints/state", RECORD];
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
    let comparison = if exit_code == 0 {
        server_comparison_evidence(&ssh, &record, &remote_root).and_then(|evidence| {
            verified_evidence(&manifest, &record, &remote_root, &typed, &evidence)?;
            verify_result_tree(&tmp.join("results"), &evidence["results_sha256"])?;
            Ok(evidence)
        })
    } else {
        Err(anyhow::anyhow!(
            "server job failed with exit code {exit_code}; only diagnostic results were fetched"
        ))
    };
    if exit_code == 0 && !local_model.is_null() {
        comparison.as_ref().map_err(|error| {
            anyhow::anyhow!(
                "remote scientific outputs were not verified; nothing was installed: {error:#}"
            )
        })?;
    }
    let scope_verified = comparison.is_ok();
    let scientific_verified = comparison
        .as_ref()
        .is_ok_and(|evidence| evidence["forcing_contents_verified"] == true);
    let verification_error = comparison.as_ref().err().map(|error| format!("{error:#}"));
    for (name, bytes) in regular_tree_files(&study_dir.join("checkpoints/state"), true)? {
        let path = tmp.join("checkpoints/state").join(name);
        std::fs::create_dir_all(path.parent().context("checkpoint has no parent")?)?;
        std::fs::write(path, bytes)?;
    }
    study::checkpoint::write_next(&tmp.join("checkpoints/state"), &typed)?;
    record.fetched_at = Some(now());
    record.provenance["ok"] = json!(exit_code == 0 && scope_verified);
    record.provenance["exit_code"] = json!(exit_code);
    record.provenance["scientific_verified"] = json!(scientific_verified);
    record.provenance["nonforcing_inputs_verified"] = json!(scope_verified);
    record.provenance["forcing_contents_verified"] = json!(scientific_verified);
    record.provenance["input_scope"] = json!("sha256-nonforcing-v1");
    record.provenance["verification_error"] = json!(verification_error);
    record.provenance["finished"] = json!(now());
    std::fs::write(tmp.join(RECORD), serde_json::to_string_pretty(&record)?)?;
    install_fetched_study(&study_dir, &tmp)?;
    let _ = std::fs::remove_dir_all(&tmp);
    let outcome = json!({"job":record.job,"outcome":"observed","terminal":true,"status":status});
    crate::history_cmd::reconcile_remote(opts, &serde_json::to_value(&record)?, &outcome)?;
    println!(
        "{}",
        json!({
            "fetched": true,
            "exit_code": exit_code,
            "scientific_verified": scientific_verified,
            "nonforcing_inputs_verified": scope_verified,
            "forcing_contents_verified": scientific_verified,
            "input_scope": "sha256-nonforcing-v1",
            "verification_error": verification_error,
            "status": typed.status,
            "best_member": typed.best_member,
            "remote_study": remote_root,
        })
    );
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
