//! 远程作业：每个作业一个目录 `<根>/jobs/<作业号>/`，里面有 `script.sh`、`log`、`phase`（当前阶段，脚本自己写）与
//! `exit_code`；没有调度系统时还有 `pid`，有调度系统时是 `scheduler` 与 `sched_id`。作业号由本地生成；关掉
//! 应用也能凭它接着查状态、看日志、取消。
//!
//! 没有调度系统（`Scheduler::Bare`）时用 `setsid nohup` 直接后台运行；Slurm、PBS、LSF 由 [`crate::sched`] 生成
//! 指令头与提交、查询、取消的命令。

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::sched::{self, Resources, Scheduler};
use crate::ssh::{quote, Ssh};

/// 作业目录。
pub fn job_dir(root: &str, id: &str) -> String {
    format!("{}/jobs/{id}", root.trim_end_matches('/'))
}

/// 作业号只许字母、数字、`-`、`_`（拼进路径前挡住 `..` 之类）。
pub fn check_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("not a job id: {id}");
    }
    Ok(())
}

/// 怎么提交：哪种调度系统、要什么资源、作业叫什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub scheduler: Scheduler,
    pub resources: Resources,
    pub name: String,
}

impl Spec {
    /// 没有调度系统：直接后台运行。
    pub fn bare() -> Self {
        Self {
            scheduler: Scheduler::Bare,
            resources: Resources {
                cpus: 1,
                ..Resources::default()
            },
            name: "colm".into(),
        }
    }
}

/// 作业脚本（`script.sh` 的全文）：调度指令头、环境准备、作业体、退出码。
/// 提交前可以把它给用户看。
pub fn job_script(root: &str, id: &str, body: &str, spec: &Spec) -> Result<String> {
    check_id(id)?;
    let dir = job_dir(root, id);
    // LSF 的 `-oo` 和 PBS 的 `-o` 要等作业结束才把输出写回去（摆渡船实测 2026-10-20：作业 08:55 开始，
    // `log` 直到 08:58 结束才出现），运行期间读不到阶段、步数和日志。这两种由脚本自己把输出实时追加到
    // `log`，调度系统的输出另写到 `scheduler.log`，结束时就不会把 `log` 盖掉。Slurm 的 `--output` 本来就是实时写的。
    let spooled = matches!(spec.scheduler, Scheduler::Lsf | Scheduler::Pbs);
    let header = sched::directives(
        spec.scheduler,
        &spec.name,
        &format!("{dir}/{}", if spooled { "scheduler.log" } else { "log" }),
        &spec.resources,
    )?;
    let mut script = String::from("#!/bin/bash\n");
    for line in header {
        script.push_str(&line);
        script.push('\n');
    }
    if spooled {
        script.push_str(&format!("exec >> {} 2>&1\n", quote(&format!("{dir}/log"))));
    }
    script.push_str(&format!("cd {}\n", quote(&dir)));
    if let Some(env) = &spec.resources.env_script {
        script.push_str(env.trim_end());
        script.push('\n');
    }
    // `set -e`：作业体里任何一步失败就停，退出码如实反映（否则最后一条命令的状态会把前面的失败盖住）。
    script.push_str(&format!("(\nset -e\n{body}\n)\necho $? > exit_code\n"));
    Ok(script)
}

/// 提交作业的远程脚本：写下作业脚本，交给调度系统（或脱离 ssh 会话在后台跑），最后一行回报作业号。
/// 作业脚本里可以 `echo 阶段 > phase` 报告进度；结束时写下退出码。
pub fn submit_script(root: &str, id: &str, body: &str, spec: &Spec) -> Result<String> {
    submit_script_with(root, id, body, spec, sched::launch(spec.scheduler))
}

/// 和 [`submit_script`] 一样，但放进这一批的节点队列（[`crate::pack`]），由载体按节点打包运行。
pub fn packed_submit_script(
    root: &str,
    id: &str,
    body: &str,
    spec: &Spec,
    pack: &crate::pack::Pack,
) -> Result<String> {
    let launch = crate::pack::launch(root, id, pack, spec.scheduler, &spec.resources)?;
    submit_script_with(root, id, body, spec, &launch)
}

fn submit_script_with(
    root: &str,
    id: &str,
    body: &str,
    spec: &Spec,
    launch: &str,
) -> Result<String> {
    let dir = quote(&job_dir(root, id));
    let script = job_script(root, id, body, spec)?;
    let prelude = if spec.scheduler == Scheduler::Bare {
        ""
    } else {
        sched::PATH_PRELUDE
    };
    Ok(format!(
        "set -e\n{prelude}mkdir -p {jobs}\nmkdir {dir} || {{ echo 'job identity already exists; inspect its status instead of resubmitting' >&2; exit 1; }}\ncd {dir}\ncat > script.sh <<'COLM_JOB_EOF'\n{script}COLM_JOB_EOF\nchmod +x script.sh\n{launch}",
        jobs = quote(&format!("{}/jobs",root.trim_end_matches('/'))),
    ))
}

/// 作业状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    /// 调度系统里排队，还没开始。
    Queued,
    Running,
    Finished {
        exit_code: i32,
    },
    /// 进程（或队列里的作业）不在了却没留下退出码（机器重启、被杀、超时）。
    Lost,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub state: State,
    pub phase: Option<String>,
    /// 调度系统给的原始状态词（`PENDING`、`TIMEOUT 0:0`…），没有调度系统时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub log_tail: String,
}

/// 作业不在队列里、也没有退出码（`Lost`），但调度系统明确记下了它怎么结束的：`cancelled` 或 `failed`。
/// land02 实测（2026-10-19）：取消 40 个还在排队的作业后，sacct 是 `CANCELLED by 1026|0:0`；它们从没开始，
/// 所以没有 `exit_code`。只认 `Finished` 时这些站点永远"待核对"，界面一直显示运行中，也不能重新提交。
/// 不编造退出码；调度系统说法含糊（`COMPLETED` 却没有退出码、查不到记录）的仍是 `Lost`。
pub fn ended(status: &Status) -> Option<&'static str> {
    if status.state != State::Lost {
        return None;
    }
    let detail = status.detail.as_deref()?.to_ascii_uppercase();
    if detail.contains("CANCELLED") {
        return Some("cancelled");
    }
    [
        "FAILED",
        "TIMEOUT",
        "OUT_OF_MEMORY",
        "NODE_FAIL",
        "PREEMPTED",
        "BOOT_FAIL",
        "DEADLINE",
    ]
    .iter()
    .any(|word| detail.contains(word))
    .then_some("failed")
}

pub fn status_script(root: &str, id: &str, lines: usize) -> String {
    let dir = quote(&job_dir(root, id));
    format!(
        r#"{prelude}{alive}cd {dir} 2>/dev/null || {{ echo "state=unknown"; exit 0; }}
if [ -f exit_code ] && [ -f cancelled ]; then echo "state=lost"; echo "detail=CANCELLED by request (exit $(cat exit_code))"
elif [ -f exit_code ]; then echo "state=finished $(cat exit_code)"
else
{query}fi
echo "phase=$(tail -n 1 phase 2>/dev/null)"
echo "---log---"
tail -n {lines} log 2>/dev/null
true
"#,
        prelude = sched::PATH_PRELUDE,
        alive = crate::pack::ALIVE,
        query = sched::QUERY,
    )
}

pub fn parse_status(text: &str) -> Status {
    let (head, log) = text.split_once("---log---\n").unwrap_or((text, ""));
    let mut state = State::Unknown;
    let mut phase = None;
    let mut detail = None;
    for line in head.lines() {
        if let Some(value) = line.strip_prefix("state=") {
            state = match value.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["queued"] => State::Queued,
                ["running"] => State::Running,
                ["lost"] => State::Lost,
                ["finished", code] => State::Finished {
                    exit_code: code.parse().unwrap_or(-1),
                },
                _ => State::Unknown,
            };
        } else if let Some(value) = line.strip_prefix("phase=") {
            phase = Some(value.trim().to_owned()).filter(|p| !p.is_empty());
        } else if let Some(value) = line.strip_prefix("detail=") {
            detail = Some(value.trim().to_owned()).filter(|d| !d.is_empty());
        }
    }
    Status {
        state,
        phase,
        detail,
        log_tail: crate::auth::redact(log),
    }
}

/// 取消：调度系统的作业交给它结束；后台进程则结束整个进程组（`setsid` 让作业自成一组）。
pub fn cancel_script(root: &str, id: &str) -> String {
    let dir = quote(&job_dir(root, id));
    format!(
        "{prelude}cd {dir} || exit 1\n{cancel}",
        prelude = sched::PATH_PRELUDE,
        cancel = sched::CANCEL,
    )
}

/// 提交的结果：没有调度系统时是进程号，有调度系统时是它给的作业号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Submitted {
    pub scheduler: &'static str,
    pub pid: Option<u32>,
    pub scheduler_id: Option<String>,
}

fn engine_body(engine_id: &str, body: &str) -> Result<String> {
    let digest = engine_id.strip_prefix("pre-").unwrap_or(engine_id);
    anyhow::ensure!(
        digest.len() == 16
            && digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid engine snapshot"
    );
    Ok(format!("# colm-engine-snapshot: {engine_id}\n{body}"))
}

pub fn submit(
    ssh: &Ssh,
    root: &str,
    id: &str,
    body: &str,
    spec: &Spec,
    engine_id: &str,
) -> Result<Submitted> {
    submit_with(ssh, root, id, body, spec, engine_id, None)
}

/// 带上批次时站点放进节点队列（任何调度系统都打包）；不带时和 [`submit`] 一样。
pub fn submit_with(
    ssh: &Ssh,
    root: &str,
    id: &str,
    body: &str,
    spec: &Spec,
    engine_id: &str,
    pack: Option<&crate::pack::Pack>,
) -> Result<Submitted> {
    check_id(id)?;
    let body = engine_body(engine_id, body)?;
    let script = match pack {
        Some(pack) => packed_submit_script(root, id, &body, spec, pack)?,
        None => submit_script(root, id, &body, spec)?,
    };
    let out = ssh.run_ok(&script)?;
    let last = out
        .trim()
        .lines()
        .last()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .context("the job did not report an id")?;
    if let Some(batch) = last.strip_prefix("pack ") {
        check_id(batch)?;
        return Ok(Submitted {
            scheduler: match spec.scheduler {
                Scheduler::Slurm => "slurm-pack",
                Scheduler::Lsf => "lsf-pack",
                Scheduler::Pbs => "pbs-pack",
                Scheduler::Bare => "bare-pack",
            },
            pid: None,
            scheduler_id: Some(batch.to_owned()),
        });
    }
    Ok(match spec.scheduler {
        Scheduler::Bare => Submitted {
            scheduler: "bare",
            pid: Some(
                last.parse()
                    .context("the job did not report a process id")?,
            ),
            scheduler_id: None,
        },
        other => Submitted {
            scheduler: other.name(),
            pid: None,
            scheduler_id: Some(last.to_owned()),
        },
    })
}

/// 不带引擎快照的作业（比较页在服务器上装 OpenBench、跑评估）：作业目录、日志、阶段、退出码与取消都和
/// 模拟作业一样，只是作业体不用服务器上的 CoLM 引擎，所以不写快照标记。不支持打包。
pub fn submit_plain(ssh: &Ssh, root: &str, id: &str, body: &str, spec: &Spec) -> Result<Submitted> {
    check_id(id)?;
    let out = ssh.run_ok(&submit_script(root, id, body, spec)?)?;
    let last = out
        .trim()
        .lines()
        .last()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .context("the job did not report an id")?;
    Ok(match spec.scheduler {
        Scheduler::Bare => Submitted {
            scheduler: "bare",
            pid: Some(
                last.parse()
                    .context("the job did not report a process id")?,
            ),
            scheduler_id: None,
        },
        other => Submitted {
            scheduler: other.name(),
            pid: None,
            scheduler_id: Some(last.to_owned()),
        },
    })
}

pub fn status(ssh: &Ssh, root: &str, id: &str, lines: usize) -> Result<Status> {
    check_id(id)?;
    Ok(parse_status(&ssh.run_ok(&status_script(root, id, lines))?))
}

pub fn cancel(ssh: &Ssh, root: &str, id: &str) -> Result<()> {
    check_id(id)?;
    ssh.run_ok(&cancel_script(root, id)).map(|_| ())
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod job_tests;
