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
    if let Some(pinned) = &result {
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
        .status()
        .with_context(|| format!("cannot start {}", pinned.cli.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// 两个研究里最优成员的成绩（`study-compare`，“比较两个版本”）。
#[derive(Debug, Clone, Serialize)]
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

fn side(study_dir: &Path) -> Result<(Side, study::spec::StudySpec)> {
    let manifest = study::engine::status(study_dir)?;
    let state = study::runner::status_state(study_dir)?;
    let version = pinned(study_dir)?;
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
            version: version
                .as_ref()
                .map_or_else(|| "official".to_owned(), |v| v.name.clone()),
            head: version.as_ref().map(|v| v.head.clone()),
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
fn same_design(a: &study::spec::StudySpec, b: &study::spec::StudySpec) -> Result<bool> {
    let strip = |spec: &study::spec::StudySpec| -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(spec)?;
        if let Some(map) = value.as_object_mut() {
            map.remove("kernel_dir");
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
    Ok(verdict(left, right, same))
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
        (Some(l), Some(r)) if (l - r).abs() <= 1e-12 * l.abs().max(r.abs()).max(1.0) => Some("tie"),
        (Some(l), Some(r)) if l < r => Some("a"),
        (Some(_), Some(_)) => Some("b"),
        _ => None,
    };
    if better.is_none() {
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
    let created = study::engine::create(&case_root, &spec_file);
    let _ = std::fs::remove_file(&spec_file);
    Ok(PathBuf::from(created?.root))
}

#[cfg(test)]
#[path = "study_version_tests.rs"]
mod study_version_tests;
