//! 研究用哪一版模型（界面“运行方式”里的模型版本）。
//!
//! 研究第一次以 `study-run --model-version NAME` 运行时，把那个修改版**拍成快照**：它的 `colm-cli`、`colm-rs` 与两个
//! Rust 前处理（改过 Fortran 时连同它的内核）复制到研究目录的 `version/` 下，记进 `model-version.json`。之后续跑、
//! 重试都用这份快照——工作区里接着改代码、重编，不会让一个研究的成员混着两版程序跑完。
//!
//! 版本记在单独的文件里而不是研究设定里：设定是 `deny_unknown_fields`，交给修改版自己的 `colm-cli` 去跑时它未必
//! 认得新字段。交出去的那个进程带着 [`DELEGATED_ENV`]，不会再按这个文件转交一次。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::study;

pub(crate) const FILE: &str = "model-version.json";
pub(crate) const DELEGATED_ENV: &str = "COLM_STUDY_BY_PARENT";
/// 快照里的程序：编排、主循环、两个前处理。
const PROGRAMS: [&str; 4] = ["colm-cli", "colm-rs", "mksrfdata-rs", "mkinidata-rs"];

/// 研究固定下来的模型版本。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct Pinned {
    pub name: String,
    pub head: String,
    pub base_commit: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub based_on: Option<String>,
    pub touches: colm_workspace::version::Touches,
    /// 快照里的 `colm-cli`。
    pub cli: PathBuf,
    /// 用哪个内核：改过 Fortran 时是快照里的那份，否则是正式内核。
    pub kernel: PathBuf,
    pub pinned_unix: i64,
    #[serde(default)]
    pub model_revision: Option<colm_model::FrozenRevision>,
    #[serde(default)]
    pub model_root: Option<PathBuf>,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

pub(crate) fn pinned(study_dir: &Path) -> Result<Option<Pinned>> {
    let path = study_dir.join(FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text)?))
}

/// 研究有没有任何任务开始过（开始过就不能再换版本）。建研究时成员已经落成算例（`Materialized`），那还不算开始。
fn started(study_dir: &Path) -> Result<bool> {
    use study::state::TaskStatus;
    let Some(state) = study::runner::status_state(study_dir)? else {
        return Ok(false);
    };
    Ok(state
        .tasks
        .values()
        .any(|task| !matches!(task.status, TaskStatus::Pending | TaskStatus::Materialized)))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 把修改版拍成快照，写 `model-version.json`。
fn pin(
    study_dir: &Path,
    readiness: &colm_workspace::version::Readiness,
    official_kernel: &Path,
) -> Result<Pinned> {
    let dest = study_dir.join("version");
    let bin = dest.join("bin");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&bin)?;
    let source_bin = readiness
        .cli
        .parent()
        .context("the version's colm-cli has no directory")?;
    for program in PROGRAMS {
        let from = source_bin.join(exe(program));
        std::fs::copy(&from, bin.join(exe(program)))
            .with_context(|| format!("cannot snapshot {}", from.display()))?;
    }
    let kernel = if readiness.kernel != official_kernel {
        let to = dest.join("kernel");
        copy_dir(&readiness.kernel, &to)?;
        to
    } else {
        official_kernel.to_path_buf()
    };
    let pinned = Pinned {
        name: readiness.name.clone(),
        head: readiness.head.clone(),
        base_commit: readiness.base_commit.clone(),
        note: readiness.note.clone(),
        based_on: readiness.based_on.clone(),
        touches: readiness.touches,
        cli: bin.join(exe("colm-cli")),
        kernel,
        pinned_unix: crate::provenance::now() as i64,
        model_revision: None,
        model_root: None,
    };
    std::fs::write(study_dir.join(FILE), serde_json::to_string_pretty(&pinned)?)?;
    Ok(pinned)
}

/// 这次 `study-run` 用哪一版：已经固定过就用固定的（再给别的版本名就报错）；没固定过、给了修改版名、研究还没开始
/// 跑，就检查就绪并拍快照；其余情况是官方版（`None`）。
pub(crate) fn ensure(
    study_dir: &Path,
    requested: Option<&str>,
    workspace_root: &Path,
    engine: &str,
    preprocessors: &str,
    official_kernel: &Path,
) -> Result<Option<Pinned>> {
    let requested = requested.filter(|name| !name.is_empty());
    let result = match (pinned(study_dir)?, requested) {
        (Some(pinned), Some(name)) if name != pinned.name => bail!(
            "this Study was started with model version {}; it cannot switch to {name}. Create a new Study for another version",
            pinned.name
        ),
        (Some(pinned), _) => Ok(Some(pinned)),
        (None, None | Some("official")) => Ok(None),
        (None, Some(name)) => {
            if started(study_dir)? {
                bail!(
                    "this Study was started with the official version; it cannot switch to {name}. Create a new Study for another version"
                );
            }
            let readiness = crate::provenance::resolve(
                workspace_root,
                name,
                engine,
                preprocessors,
                official_kernel,
            )?;
            pin(study_dir, &readiness, official_kernel).map(Some)
        }
    }?;
    if let Some(pinned) = result.as_ref().filter(|p| p.model_revision.is_none()) {
        // A successful pin remains usable even if its history needs repair on the next run.
        if let Err(error) = record_pin(study_dir, workspace_root, pinned) {
            eprintln!("warning: study snapshot is saved, but its workspace ledger could not be updated: {error:#}");
        }
    }
    Ok(result)
}

fn record_pin(study_dir: &Path, workspace_root: &Path, pinned: &Pinned) -> Result<()> {
    let workspace = colm_workspace::Workspace::open(workspace_root, &pinned.name)?;
    let id = study_dir
        .file_name()
        .context("study directory has no name")?
        .to_string_lossy();
    colm_workspace::studies::append(
        &workspace,
        study_dir,
        &id,
        &pinned.head,
        "local",
        pinned.pinned_unix.max(0) as u64,
    )
}

/// 交给快照里的 `colm-cli` 跑这个研究。`args` 是去掉了版本与内核参数的 `study-run` 原参数（不含子命令）。
pub(crate) fn delegate(pinned: &Pinned, args: &[String]) -> Result<i32> {
    let status = std::process::Command::new(&pinned.cli)
        .arg("study-run")
        .args(args)
        .arg("--kernel")
        .arg(&pinned.kernel)
        .env(DELEGATED_ENV, "1")
        .env("COLM_AUDIT_BY_PARENT", "1")
        .status()
        .with_context(|| format!("cannot start {}", pinned.cli.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// 两个研究里最优成员的成绩（`study-compare`，“比较两个版本”）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Side {
    pub study: String,
    pub version: String,
    pub head: Option<String>,
    pub status: String,
    pub best_member: Option<String>,
    pub calibration: Option<f64>,
    pub validation: Option<f64>,
    pub baseline_calibration: Option<f64>,
    pub baseline_validation: Option<f64>,
}

fn comparison_version(study_dir: &Path) -> Result<(String, Option<String>)> {
    if let Some(version) = pinned(study_dir)? {
        return Ok((version.name, Some(version.head)));
    }
    let record_path = study_dir.join(crate::remote_cmd::remote_study::RECORD);
    match std::fs::symlink_metadata(&record_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(("official".into(), None));
        }
        Err(error) => return Err(error).context("cannot inspect captured remote Study version"),
    }
    // Comparison is a local metadata read, not an SSH operation: recorded
    // password settings must not require credentials or initialize global auth.
    let record: crate::remote_cmd::remote_study::StudyRecord = serde_json::from_str(
        &std::fs::read_to_string(&record_path)
            .context("cannot read captured remote Study version")?,
    )
    .context("cannot parse captured remote Study version")?;
    let version = &record.provenance["version"];
    match version["kind"].as_str() {
        Some("official") => Ok(("official".into(), None)),
        Some("workspace") => {
            let name = version["name"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("captured remote workspace version has no name")?;
            let head = version["head"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("captured remote workspace version has no commit")?;
            Ok((name.into(), Some(head.into())))
        }
        _ => bail!("captured remote Study version has no supported kind"),
    }
}

fn side(study_dir: &Path) -> Result<(Side, study::spec::StudySpec)> {
    let manifest = study::engine::status(study_dir)?;
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .map(|loaded| loaded.payload);
    let (version, head) = comparison_version(study_dir)?;
    let best = state.as_ref().and_then(|s| s.best_member.clone());
    let candidate = |member: &str| {
        state
            .as_ref()
            .and_then(|s| s.candidates.get(member))
            .cloned()
    };
    let baseline = manifest
        .members
        .iter()
        .find(|m| m.baseline)
        .and_then(|m| candidate(&m.id));
    let best_candidate = best.as_deref().and_then(candidate);
    Ok((
        Side {
            study: study_dir.display().to_string(),
            version,
            head,
            status: state.as_ref().map_or_else(
                || "not started".to_owned(),
                |s| format!("{:?}", s.status).to_lowercase(),
            ),
            best_member: best,
            calibration: best_candidate
                .as_ref()
                .and_then(|c| c.calibration)
                .or(state.as_ref().and_then(|s| s.best_objective)),
            validation: best_candidate.as_ref().and_then(|c| c.validation),
            baseline_calibration: baseline.as_ref().and_then(|c| c.calibration),
            baseline_validation: baseline.as_ref().and_then(|c| c.validation),
        },
        manifest.spec,
    ))
}

/// 两份设定除了内核目录之外是否一致（公平比较的前提：同样的参数、预算、种子、目标与时段）。
pub(crate) fn same_design(a: &study::spec::StudySpec, b: &study::spec::StudySpec) -> Result<bool> {
    let strip = |spec: &study::spec::StudySpec| -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(spec)?;
        if let Some(map) = value.as_object_mut() {
            map.remove("kernel_dir");
            // Paths relocate across hosts; observation contents are checked separately.
            map.insert(
                "observations".into(),
                serde_json::to_value(spec.observations.keys().collect::<Vec<_>>())?,
            );
        }
        Ok(value)
    };
    Ok(strip(a)? == strip(b)?)
}

/// 比较两个版本：两个研究各自率定的最优成员，看验证期（没有验证期就看率定期）上谁的目标值更低。
pub(crate) fn compare(a: &Path, b: &Path) -> Result<serde_json::Value> {
    let (left, left_spec) = side(a)?;
    let (right, right_spec) = side(b)?;
    let same = same_design(&left_spec, &right_spec)?;
    let inputs = comparison_inputs(a).and_then(|left_inputs| {
        comparison_inputs(b)
            .map(|right_inputs| study::model::same_conditions(&left_inputs, &right_inputs))
    });
    let inputs = inputs.and_then(|same_inputs| {
        anyhow::ensure!(
            left == side(a)?.0 && right == side(b)?.0,
            "Study comparison scores or model identity changed during verification"
        );
        Ok(same_inputs)
    });
    let verified = inputs.is_ok();
    let same_inputs = inputs.as_ref().is_ok_and(|same| *same);
    let mut result = verdict(left, right, same);
    if !same_inputs {
        result["better"] = serde_json::Value::Null;
    }
    result["same_design"] = serde_json::json!(same);
    result["same_inputs"] = serde_json::json!(same_inputs);
    result["inputs_verified"] = serde_json::json!(verified);
    let notes = result["notes"]
        .as_array_mut()
        .expect("verdict notes is an array");
    match inputs {
        Err(error) => notes.push(serde_json::json!(format!("full-content inputs, observations and outputs could not be verified: {error:#}; no version winner is reported"))),
        Ok(false) => notes.push(serde_json::json!("the Studies use different scientific input or observation contents; no version winner is reported")),
        Ok(true) => (),
    }
    Ok(result)
}

/// Each side is verified in its own process so SSH credentials cannot cross host boundaries.
pub(crate) fn verified_side(study_dir: &Path) -> Result<serde_json::Value> {
    let (side, spec) = side(study_dir)?;
    let verification = comparison_inputs(study_dir).and_then(|identity| {
        anyhow::ensure!(
            side == self::side(study_dir)?.0,
            "Study score or model identity changed during side verification"
        );
        Ok(identity)
    });
    let (inputs, verified, notes) = match verification {
        Ok(identity) => (Some(identity), true, Vec::new()),
        Err(error) => (
            None,
            false,
            vec![format!(
                "full-content scientific evidence could not be verified: {error:#}"
            )],
        ),
    };
    Ok(serde_json::json!({
        "schema":1,
        "side":side,
        "spec":spec,
        "input_identity":inputs,
        "verified":verified,
        "notes":notes,
    }))
}

fn comparison_inputs(study_dir: &Path) -> Result<study::model::StudyInputIdentity> {
    let record = study_dir.join(crate::remote_cmd::remote_study::RECORD);
    match std::fs::symlink_metadata(&record) {
        Ok(_) => return crate::remote_cmd::remote_study::verified_comparison(study_dir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error).context("cannot inspect Study remote submission"),
    }
    local_comparison_inputs(study_dir)
}

fn local_comparison_inputs(study_dir: &Path) -> Result<study::model::StudyInputIdentity> {
    let manifest = study::engine::status(study_dir)?;
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study has no state")?
    .payload;
    anyhow::ensure!(
        state.schema_version == 1 && state.study_id == manifest.id,
        "Study state identity changed"
    );
    anyhow::ensure!(
        !matches!(
            state.status,
            study::state::StudyStatus::Running
                | study::state::StudyStatus::Ready
                | study::state::StudyStatus::NeedsReview
        ),
        "Study is not in a verified finished state"
    );
    let best = state
        .best_member
        .as_deref()
        .context("Study has no best member")?;
    let baseline = manifest
        .members
        .iter()
        .find(|member| member.baseline)
        .context("Study has no baseline member")?;
    let members = if baseline.id == best {
        vec![best]
    } else {
        vec![best, baseline.id.as_str()]
    };
    let before = study::runner::evidence_file_snapshot(&manifest, &state, &members)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let inputs = study::model::verified_inputs(&manifest)?;
    for member in &members {
        study::runner::verify_member_evidence(&manifest, &state, member)?;
    }
    anyhow::ensure!(
        before == study::runner::evidence_file_snapshot(&manifest, &state, &members)?,
        "Study comparison inputs or outputs changed during verification"
    );
    Ok(inputs)
}

/// Executed by the bound remote job's CLI, never accepted as a frontend assertion.
pub(crate) fn local_comparison_evidence(study_dir: &Path) -> Result<serde_json::Value> {
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study has no verified state")?
    .payload;
    let state_sha256 = colm_kernel::sha256_hex(&serde_json::to_vec(&state)?);
    let manifest = study::engine::status(study_dir)?;
    let best = state
        .best_member
        .as_deref()
        .context("Study has no best member")?;
    let baseline = manifest
        .members
        .iter()
        .find(|member| member.baseline)
        .context("Study has no baseline")?;
    let members = vec![best, baseline.id.as_str()];
    let files_before = study::runner::evidence_file_snapshot(&manifest, &state, &members)?;
    let inputs = local_comparison_inputs(study_dir)?;
    let results_sha256 = comparison_results_sha256(study_dir)?;
    let after = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study state disappeared during comparison")?
    .payload;
    anyhow::ensure!(
        state_sha256 == colm_kernel::sha256_hex(&serde_json::to_vec(&after)?),
        "Study state changed during comparison verification"
    );
    anyhow::ensure!(
        inputs == study::model::verified_inputs(&manifest)?,
        "Study scientific inputs changed during comparison verification"
    );
    anyhow::ensure!(
        files_before == study::runner::evidence_file_snapshot(&manifest, &after, &members)?,
        "Study raw outputs or evaluation evidence changed during comparison verification"
    );
    let pin = pinned(study_dir)?;
    let revision = pin.as_ref().and_then(|pin| pin.model_revision.as_ref());
    Ok(serde_json::json!({
        "schema":1,
        "study_id":manifest.id,
        "root":manifest.root,
        "spec":manifest.spec,
        "model_revision":revision,
        "pinned":pin,
        "best_member":state.best_member,
        "state_sha256":state_sha256,
        "state":state,
        "input_identity":inputs,
        "results_sha256":results_sha256,
    }))
}

pub(crate) fn comparison_results_sha256(
    study_dir: &Path,
) -> Result<std::collections::BTreeMap<String, String>> {
    fn paths(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                paths(root, &path, files)?;
            } else {
                anyhow::ensure!(kind.is_file(), "Study results contain a non-regular file");
                files.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
        Ok(())
    }
    let root = study_dir.join("results");
    let mut before = Vec::new();
    paths(&root, &root, &mut before)?;
    before.sort();
    let before_stats = before
        .iter()
        .map(|relative| {
            Ok((
                relative.clone(),
                crate::fingerprint::stable_file_identity(&std::fs::metadata(root.join(relative))?),
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let hashes = before
        .iter()
        .map(|relative| {
            let key = relative
                .to_str()
                .context("Study result path is not UTF-8")?
                .replace('\\', "/");
            let file = crate::fingerprint::full_file(&root.join(relative), "result")?;
            Ok((key, file.sha256))
        })
        .collect::<Result<_>>()?;
    let mut after = Vec::new();
    paths(&root, &root, &mut after)?;
    after.sort();
    let after_stats = after
        .iter()
        .map(|relative| {
            Ok((
                relative.clone(),
                crate::fingerprint::stable_file_identity(&std::fs::metadata(root.join(relative))?),
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    anyhow::ensure!(
        before == after && before_stats == after_stats,
        "Study results changed during comparison verification"
    );
    Ok(hashes)
}

/// 两边成绩的判定（目标值越小越好，与率定的目标函数一致）。
pub(crate) fn verdict(left: Side, right: Side, same: bool) -> serde_json::Value {
    let mut notes = Vec::new();
    if !same {
        notes.push("the two Studies differ in parameters, budget, seed, targets or periods; this is not a fair comparison".to_owned());
    }
    if left.version == right.version {
        notes.push("both Studies use the same model version".to_owned());
    }
    let has_validation = left.validation.is_some() && right.validation.is_some();
    if !has_validation {
        notes.push("no validation score on both sides; compared on the calibration period, which favours whichever fits the training data harder".to_owned());
    }
    let (l, r) = if has_validation {
        (left.validation, right.validation)
    } else {
        (left.calibration, right.calibration)
    };
    let better = match (l, r) {
        _ if !same => None,
        (Some(l), Some(r)) if !l.is_finite() || !r.is_finite() => None,
        (Some(l), Some(r)) if (l - r).abs() <= 1e-12 * l.abs().max(r.abs()).max(1.0) => Some("tie"),
        (Some(l), Some(r)) if l < r => Some("a"),
        (Some(_), Some(_)) => Some("b"),
        _ => None,
    };
    if same && better.is_none() {
        notes.push("at least one Study has no best member yet".to_owned());
    }
    serde_json::json!({
        "a": left,
        "b": right,
        "same_design": same,
        "period": if has_validation { "validation" } else { "calibration" },
        "better": better,
        "notes": notes,
    })
}

/// `study-clone <研究>`：按同一份设定（参数、预算、种子、目标、时段，以及建研究时冻结的标准化）在同一个项目里
/// 再建一个研究，给“比较两个版本”的另一边用。返回新研究的目录。
pub(crate) fn clone_study(study_dir: &Path) -> Result<PathBuf> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let manifest = study::engine::status(&study_dir)?;
    // 研究在 `<项目>/.colm/studies/<编号>/`。
    let case_root = study_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|_| {
            study_dir
                .parent()
                .is_some_and(|p| p.ends_with(Path::new(".colm").join("studies")))
        })
        .with_context(|| {
            format!(
                "{} is not under <project>/.colm/studies",
                study_dir.display()
            )
        })?
        .to_path_buf();
    let spec_file = std::env::temp_dir().join(format!(
        "colm-study-clone-{}-{}.json",
        std::process::id(),
        crate::provenance::now()
    ));
    std::fs::write(&spec_file, serde_json::to_string_pretty(&manifest.spec)?)?;
    let model_pin = pinned(&study_dir)?.filter(|p| p.model_revision.is_some());
    let created = (|| {
        if model_pin.is_some() {
            study::engine::verify_frozen_inputs(&manifest)?;
            study::model::verified_inputs(&manifest)?;
            let first = manifest
                .spec
                .base_cases
                .first()
                .context("Study has no base cases")?;
            let baseline = study::model::base_case(&manifest, first)?;
            study::engine::create_from_baselines(
                &case_root,
                &spec_file,
                baseline.parent().context("baseline directory")?,
            )
        } else {
            study::engine::create(&case_root, &spec_file)
        }
    })();
    let _ = std::fs::remove_file(&spec_file);
    let created = PathBuf::from(created?.root);
    let finalization = (|| -> Result<()> {
        if let Some(mut pin) = model_pin {
            let source = study_dir.join("version");
            let target = created.join("version");
            if source.is_dir() {
                copy_dir(&source, &target)?;
            }
            if let Ok(relative) = pin.kernel.strip_prefix(&source) {
                pin.kernel = target.join(relative);
            }
            if let Ok(relative) = pin.cli.strip_prefix(&source) {
                pin.cli = target.join(relative);
            }
            std::fs::write(created.join(FILE), serde_json::to_vec_pretty(&pin)?)?;
        }
        study::model::capture_inputs(&study::engine::status(&created)?)?;
        Ok(())
    })();
    if let Err(error) = finalization {
        let _ = std::fs::remove_dir_all(&created);
        return Err(error);
    }
    Ok(created)
}

#[cfg(test)]
#[path = "study_version_tests.rs"]
mod study_version_tests;

pub(crate) fn pin_revision(
    study_dir: &Path,
    revision: &colm_model::FrozenRevision,
    model_root: &Path,
    opts: &crate::Opts,
) -> Result<Pinned> {
    crate::model_version::verify_code(opts, &revision.code)?;
    crate::model_version::cache_code(opts, &revision.code)?;
    let manifest = study::engine::status(study_dir)?;
    let kernel = opts
        .get("--kernel")
        .or(manifest.spec.kernel_dir)
        .map(PathBuf::from)
        .unwrap_or_default();
    let kernel = if kernel.is_dir() {
        let target = study_dir.join("version/kernel");
        copy_dir(&kernel, &target)?;
        target
    } else {
        kernel
    };
    let cached = crate::model_version::cached_options(opts, &revision.code)?;
    let (cli, touches) = if let Some(name) = revision.code.workspace.as_deref() {
        let flags = cached.as_ref().unwrap_or(opts);
        let workspace = colm_workspace::Workspace::open(
            &crate::provenance::workspace_root(flags.get("--workspace-root")),
            name,
        )?;
        let changed = workspace.changes_from_official()?;
        (
            workspace.bin().join(exe("colm-cli")),
            colm_workspace::version::touches(changed.iter().map(|c| c.path.as_str())),
        )
    } else {
        (
            crate::model_version::cached_official_cli(opts, &revision.code)?,
            colm_workspace::version::Touches {
                engine: false,
                kernel: false,
            },
        )
    };
    let pinned = Pinned {
        name: revision.version.id.clone(),
        head: revision
            .code
            .commit
            .clone()
            .unwrap_or_else(|| revision.code.source_sha256.clone()),
        base_commit: revision.code.commit.clone().unwrap_or_default(),
        note: revision.note.clone(),
        based_on: revision.parent.as_ref().map(|p| p.id.clone()),
        touches,
        cli,
        kernel,
        pinned_unix: crate::provenance::now() as i64,
        model_revision: Some(revision.clone()),
        model_root: Some(model_root.to_owned()),
    };
    std::fs::write(study_dir.join(FILE), serde_json::to_vec_pretty(&pinned)?)?;
    Ok(pinned)
}

pub(crate) fn validate_revision_request(study: &Path, opts: &crate::Opts) -> Result<()> {
    if let Some(requested) = crate::model_version::selected(opts)? {
        let pinned = pinned(study)?.and_then(|p| p.model_revision).context(
            "this Study has no frozen model revision; create a Study with the selected revision",
        )?;
        anyhow::ensure!(
            pinned.version == requested,
            "Study is locked to another model revision"
        );
    }
    Ok(())
}
fn revision_options(pinned: &Pinned, opts: &crate::Opts) -> Result<crate::Opts> {
    let mut flags = opts
        .flags
        .iter()
        .filter(|(k, _)| k != "--model-root")
        .cloned()
        .collect::<Vec<_>>();
    flags.push((
        "--model-root".into(),
        pinned
            .model_root
            .as_ref()
            .context("pinned revision has no registry")?
            .to_string_lossy()
            .into_owned(),
    ));
    Ok(crate::Opts {
        flags,
        positional: opts.positional.clone(),
    })
}
pub(crate) fn validate_pinned(
    pinned: &mut Pinned,
    opts: &crate::Opts,
    kernel: &Path,
    engine: &str,
    preprocessors: &str,
) -> Result<()> {
    let Some(revision) = &pinned.model_revision else {
        return Ok(());
    };
    let options = revision_options(pinned, opts)?;
    crate::model_version::verify_code(&options, &revision.code)?;
    anyhow::ensure!(
        revision.overlay.hybrid.is_none() || engine == "rust",
        "hybrid model revisions require Rust"
    );
    let kernel = if pinned.kernel.as_os_str().is_empty() {
        kernel
    } else {
        &pinned.kernel
    };
    crate::model_version::check_kernel(revision, &colm_kernel::Kernel::open_manifest(kernel)?)?;
    let used = if let Some(name) = &revision.code.workspace {
        let cached = crate::model_version::cached_options(&options, &revision.code)?
            .context("missing frozen workspace")?;
        crate::provenance::resolve(
            &crate::provenance::workspace_root(cached.get("--workspace-root")),
            name,
            engine,
            preprocessors,
            kernel,
        )?
        .kernel
    } else {
        kernel.to_owned()
    };
    pinned.kernel = used;
    Ok(())
}
