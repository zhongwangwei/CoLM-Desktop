//! Durable Study/task state. The latest valid checkpoint is authoritative.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyStatus {
    Ready,
    Running,
    Paused,
    Completed,
    CompletedWithFailures,
    Cancelled,
    NeedsReview,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Materialized,
    Queued,
    Running,
    Evaluating,
    Succeeded,
    Failed,
    Interrupted,
    NeedsReview,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub executable: String,
    pub argv_sha256: String,
    pub started_unix: i64,
    pub heartbeat_unix: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TaskState {
    pub member: String,
    pub site: String,
    pub case_dir: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub objective: Option<f64>,
    #[serde(default)]
    pub validation_objective: Option<f64>,
    #[serde(default)]
    pub process: Option<ProcessIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CandidateState {
    pub generation: usize,
    pub feasible: bool,
    #[serde(default)]
    pub calibration: Option<f64>,
    #[serde(default)]
    pub validation: Option<f64>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StudyState {
    pub schema_version: u32,
    pub study_id: String,
    pub status: StudyStatus,
    pub generation: usize,
    pub tasks: BTreeMap<String, TaskState>,
    #[serde(default)]
    pub best_member: Option<String>,
    #[serde(default)]
    pub best_objective: Option<f64>,
    #[serde(default)]
    pub completed_candidates: usize,
    /// Current DE parents. Empty for OAT/LHS and before generation zero is scored.
    #[serde(default)]
    pub population: Vec<String>,
    #[serde(default)]
    pub candidates: BTreeMap<String, CandidateState>,
    #[serde(default)]
    pub no_improvement_generations: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl StudyState {
    pub fn new(study_id: String, tasks: impl IntoIterator<Item = TaskState>) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        for task in tasks {
            let id = task_id(&task.member, &task.site);
            if by_id.insert(id.clone(), task).is_some() {
                bail!("duplicate Study task {id}");
            }
        }
        Ok(Self {
            schema_version: 1,
            study_id,
            status: StudyStatus::Ready,
            generation: 0,
            tasks: by_id,
            best_member: None,
            best_objective: None,
            completed_candidates: 0,
            population: Vec::new(),
            candidates: BTreeMap::new(),
            no_improvement_generations: 0,
            warnings: Vec::new(),
        })
    }

    /// 所有站点任务都已结束（成功、失败或取消）的候选数，不含基线 `m000000`。
    /// 和 `refresh_candidates` 写入 `candidates` 的判据相同，但不用打分：每个任务结束时就能重算。
    /// 原来只在一整批成员跑完后才更新，差分进化跑一代的几十分钟里界面一直显示 0（第 734 轮）。
    pub fn finished_candidates(&self, sites: &[String]) -> usize {
        let members: BTreeSet<&str> = self
            .tasks
            .values()
            .map(|task| task.member.as_str())
            .filter(|member| *member != "m000000")
            .collect();
        members
            .into_iter()
            .filter(|member| {
                sites.iter().all(|site| {
                    self.tasks.get(&task_id(member, site)).is_some_and(|task| {
                        matches!(
                            task.status,
                            TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
                        )
                    })
                })
            })
            .count()
    }

    pub fn insert_task(&mut self, task: TaskState) -> Result<()> {
        let id = task_id(&task.member, &task.site);
        if self.tasks.insert(id.clone(), task).is_some() {
            bail!("duplicate Study task {id}");
        }
        Ok(())
    }

    pub fn set_task(&mut self, member: &str, site: &str, status: TaskStatus) -> Result<()> {
        let id = task_id(member, site);
        let task = self
            .tasks
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown Study task {id}"))?;
        if !valid_transition(task.status, status) {
            bail!("invalid task transition {:?} -> {status:?}", task.status);
        }
        task.status = status;
        Ok(())
    }

    pub fn finish_status(&mut self) {
        let statuses = self.tasks.values().map(|task| task.status);
        let mut failed = false;
        let mut pending = false;
        let mut cancelled = false;
        let mut review = false;
        for status in statuses {
            failed |= matches!(status, TaskStatus::Failed | TaskStatus::Interrupted);
            pending |= matches!(
                status,
                TaskStatus::Pending
                    | TaskStatus::Materialized
                    | TaskStatus::Queued
                    | TaskStatus::Running
                    | TaskStatus::Evaluating
            );
            cancelled |= status == TaskStatus::Cancelled;
            review |= status == TaskStatus::NeedsReview;
        }
        self.status = if review {
            StudyStatus::NeedsReview
        } else if pending {
            StudyStatus::Paused
        } else if cancelled {
            StudyStatus::Cancelled
        } else if failed {
            StudyStatus::CompletedWithFailures
        } else {
            StudyStatus::Completed
        };
    }

    /// A previous supervisor disappeared while a model task was marked running.
    /// Without a verified child identity it is unsafe to double-run, so recovery
    /// deliberately requires review rather than guessing that the child exited.
    /// 一批成员跑完、研究还要接着跑（率定先单独跑基准，再跑种群、下一代）时的状态。原来每批跑完都用
    /// `finish_status` 落盘：基准跑完、种群还排着就记成 Paused，一代跑完、下一代还没建就记成
    /// Completed，一直留到下一批跑完。研究页实跑（2026-10-22）看到运行中先显示"已暂停"、给出"恢复"，
    /// 后显示"已完成"、放开"写入新版本"。批次之间研究仍在运行；只有请求了暂停或取消、或有成员待复核
    /// 才停在那里，最终状态由调用方在全部跑完后用 `finish_status` 再定。
    pub fn finish_batch_status(&mut self, stopping: bool) {
        self.finish_status();
        if !stopping
            && matches!(
                self.status,
                StudyStatus::Paused | StudyStatus::Completed | StudyStatus::CompletedWithFailures
            )
        {
            self.status = StudyStatus::Running;
        }
    }

    #[allow(dead_code)]
    pub fn reconcile_unverified_running(&mut self) {
        for task in self.tasks.values_mut() {
            if matches!(task.status, TaskStatus::Running | TaskStatus::Evaluating) {
                task.status = TaskStatus::NeedsReview;
                task.reason = Some("previous process identity could not be verified".into());
            }
        }
        self.finish_status();
    }
}

pub fn task_id(member: &str, site: &str) -> String {
    format!("{member}/{site}")
}

pub fn pause_requested(study_dir: &Path) -> bool {
    study_dir.join("pause.request").is_file()
}

pub fn cancel_requested(study_dir: &Path) -> bool {
    study_dir.join("cancel.request").is_file()
}

pub fn request_pause(study_dir: &Path) -> Result<()> {
    std::fs::write(study_dir.join("pause.request"), b"pause\n")?;
    Ok(())
}

pub fn resume(study_dir: &Path) -> Result<()> {
    let manifest = super::engine::status(study_dir)?;
    super::engine::ensure_supported_study_manifest(&manifest, None)?;
    remove_if_present(&study_dir.join("pause.request"))
}

pub fn clear_cancel(study_dir: &Path) -> Result<()> {
    remove_if_present(&study_dir.join("cancel.request"))
}

pub fn request_cancel(study_dir: &Path) -> Result<()> {
    std::fs::write(study_dir.join("cancel.request"), b"cancel pending tasks\n")?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn valid_transition(from: TaskStatus, to: TaskStatus) -> bool {
    use TaskStatus::*;
    from == to
        || matches!(
            (from, to),
            (Pending, Materialized | Queued | Failed | Cancelled)
                | (Materialized, Queued | Failed | Cancelled)
                | (Queued, Running | Cancelled)
                | (
                    Running,
                    Evaluating | Succeeded | Failed | Interrupted | NeedsReview
                )
                | (Evaluating, Succeeded | Failed | Interrupted | NeedsReview)
                | (Failed | Interrupted, Queued | Cancelled)
                | (NeedsReview, Queued | Cancelled)
                | (Succeeded, Queued)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: TaskStatus) -> TaskState {
        TaskState {
            member: "m000001".into(),
            site: "AT-Neu".into(),
            case_dir: "/case".into(),
            status,
            stage: None,
            reason: None,
            objective: None,
            validation_objective: None,
            process: None,
        }
    }

    #[test]
    fn finished_candidates_count_members_whose_every_site_ended() {
        let at = |member: &str, site: &str, status| TaskState {
            member: member.into(),
            site: site.into(),
            ..task(status)
        };
        let state = StudyState::new(
            "s-test".into(),
            [
                at("m000000", "A", TaskStatus::Succeeded),
                at("m000000", "B", TaskStatus::Succeeded),
                at("m000001", "A", TaskStatus::Succeeded),
                at("m000001", "B", TaskStatus::Failed),
                at("m000002", "A", TaskStatus::Succeeded),
                at("m000002", "B", TaskStatus::Running),
                at("m000003", "A", TaskStatus::Cancelled),
                at("m000003", "B", TaskStatus::Succeeded),
            ],
        )
        .unwrap();
        // 基线不算；m000002 还有站点在跑。
        assert_eq!(state.finished_candidates(&["A".into(), "B".into()]), 2);
    }

    #[test]
    fn rejects_impossible_task_transitions() {
        let mut state = StudyState::new("s-test".into(), [task(TaskStatus::Pending)]).unwrap();
        assert!(state
            .set_task("m000001", "AT-Neu", TaskStatus::Succeeded)
            .is_err());
        state
            .set_task("m000001", "AT-Neu", TaskStatus::Queued)
            .unwrap();
        state
            .set_task("m000001", "AT-Neu", TaskStatus::Running)
            .unwrap();
        state
            .set_task("m000001", "AT-Neu", TaskStatus::Succeeded)
            .unwrap();
        state.finish_status();
        assert_eq!(state.status, StudyStatus::Completed);
    }

    #[test]
    fn the_study_stays_running_between_batches() {
        let mut state = StudyState::new(
            "s-test".into(),
            [
                task(TaskStatus::Pending),
                TaskState {
                    member: "m000000".into(),
                    ..task(TaskStatus::Pending)
                },
            ],
        )
        .unwrap();
        for status in [
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Succeeded,
        ] {
            state.set_task("m000000", "AT-Neu", status).unwrap();
        }
        state.finish_batch_status(false);
        assert_eq!(state.status, StudyStatus::Running);
        state.finish_batch_status(true);
        assert_eq!(state.status, StudyStatus::Paused);
        state
            .set_task("m000001", "AT-Neu", TaskStatus::Queued)
            .unwrap();
        state
            .set_task("m000001", "AT-Neu", TaskStatus::Running)
            .unwrap();
        state
            .set_task("m000001", "AT-Neu", TaskStatus::Succeeded)
            .unwrap();
        // 这一代全跑完了，下一代还没建：研究仍在运行，不是已完成。
        state.finish_batch_status(false);
        assert_eq!(state.status, StudyStatus::Running);
        state.finish_status();
        assert_eq!(state.status, StudyStatus::Completed);
    }

    #[test]
    fn unverified_running_task_is_never_silently_requeued() {
        let mut state = StudyState::new("s-test".into(), [task(TaskStatus::Running)]).unwrap();
        state.reconcile_unverified_running();
        assert_eq!(state.status, StudyStatus::NeedsReview);
        assert_eq!(
            state.tasks["m000001/AT-Neu"].status,
            TaskStatus::NeedsReview
        );
    }
}
