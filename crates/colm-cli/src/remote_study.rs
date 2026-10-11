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
        "# colm-study-root: {study_marker}\necho \"preparing the engine\" > phase\n{ensure}\
         echo \"creating the Study\" >> phase\n\
         {bin}/colm-cli study-create {project} --spec {spec} > study_create.out\n\
         tail -n 1 study_create.out > study_root\n\
         STUDY=$(cat study_root)\n\
         echo \"running the Study\" >> phase\n\
         RAYON_NUM_THREADS={threads} {bin}/colm-cli study-run \"$STUDY\" --kernel {kernel} --stream 1 --jobs {jobs} --engine {plan_engine} --preprocessors {preprocessors}\n",
        study_marker = serde_json::to_string(remote_project).expect("strings serialize to JSON"),
        ensure = engine::ensure_script(root, engine_id, threads.max(8).clamp(1, 64)),
        project = quote(remote_project),
        spec = quote(&format!("{remote_project}/spec.json")),
        kernel = quote(remote_kernel),
    )
}

/// `remote-study-run`：在服务器上重建并运行这个研究。
pub(crate) fn cmd_run(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let manifest = study::engine::status(&study_dir)?;
    not_started(&study_dir)?;
    let project = project_of(&study_dir)?;
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
                &project.join(base),
                &staging.join(base),
                &format!("{remote_project}/{base}"),
                &maps,
                upload_unmapped,
            )?;
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
    spec["kernel_dir"] = json!(remote_kernel);
    std::fs::write(
        staging.join("spec.json"),
        serde_json::to_string_pretty(&spec)?,
    )?;
    let body = study_job_body(
        &root,
        engine.id(),
        &remote_project,
        &remote_kernel,
        threads,
        jobs,
        &plan.engine,
        &plan.preprocessors,
    );
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
    let mut engine_lock = colm_remote::engine_inventory::LifecycleLock::acquire(&ssh, &root)?;
    let (engine_uploaded, id, submitted) = engine_lock.run(|| {
    let engine_uploaded = engine.upload(&ssh, &root)?;
    if !fortran {
        ssh.run_ok(&format!(
            "mkdir -p {dir} && cat > {dir}/manifest.json <<'COLM_MANIFEST_EOF'\n{manifest_text}\nCOLM_MANIFEST_EOF\n",
            dir = quote(&remote_kernel)
        ))?;
    }
    // 服务器上已有的旧项目目录先清掉：研究要从头按这份设定建。
    ssh.run_ok(&format!("rm -rf {}", quote(&remote_project)))?;
    let entries: Vec<String> = std::fs::read_dir(&staging)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    ssh.upload(&staging, &entries, &remote_project)?;
    let _ = std::fs::remove_dir_all(&staging);
    let id = format!("study-{}-{}", now(), short_hash(&manifest.id));
    let submitted = job::submit(&ssh, &root, &id, &body, &job_spec, engine.id())?;
        Ok((engine_uploaded, id, submitted))
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
    Ok((!root.is_empty() && root.starts_with('/')).then(|| root.to_owned()))
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

/// `remote-study-status`：作业状态，加上服务器上研究的进度（建好之后）。
pub(crate) fn cmd_status(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let record = read_record(&study_dir)?;
    let ssh = Ssh::new(&record.host)?;
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
    println!(
        "{}",
        json!({ "record": record, "status": status, "study": progress })
    );
    Ok(())
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
    let local = local_root.to_string_lossy();
    if let Some(tasks) = state["tasks"].as_object_mut() {
        for task in tasks.values_mut() {
            if let Some(dir) = task["case_dir"].as_str() {
                if let Some(rest) = dir.strip_prefix(remote_root) {
                    task["case_dir"] = json!(format!("{local}{rest}"));
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
pub(crate) fn cmd_fetch(opts: &Opts) -> Result<()> {
    let study_dir = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let manifest = study::engine::status(&study_dir)?;
    let mut record = read_record(&study_dir)?;
    let ssh = Ssh::new(&record.host)?;
    let status = job::status(&ssh, &record.root, &record.job, 20)?;
    let exit_code = match status.state {
        job::State::Finished { exit_code } => exit_code,
        _ => bail!("the server job has not finished yet; fetch after it ends"),
    };
    let remote_root = remote_study_root(&ssh, &record)?
        .context("the server job did not create the Study; see its log")?;
    let remote = remote_study_status(&ssh, &record, &remote_root)?;
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
    // `results/` 整个换成服务器上的。
    let tmp = study_dir.join(".fetch-results");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    ssh.download(&remote_root, &["results".to_owned()], &tmp)?;
    let results = study_dir.join("results");
    let _ = std::fs::remove_dir_all(&results);
    std::fs::rename(tmp.join("results"), &results)?;
    let _ = std::fs::remove_dir_all(&tmp);
    study::checkpoint::write_next(&study_dir.join("checkpoints/state"), &typed)?;
    record.fetched_at = Some(now());
    record.provenance["ok"] = json!(exit_code == 0);
    record.provenance["finished"] = json!(now());
    std::fs::write(
        study_dir.join(RECORD),
        serde_json::to_string_pretty(&record)?,
    )?;
    println!(
        "{}",
        json!({
            "fetched": true,
            "exit_code": exit_code,
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
