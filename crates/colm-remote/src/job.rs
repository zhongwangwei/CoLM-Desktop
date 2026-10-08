//! 远程后台作业（无调度系统时直接 `setsid nohup`）：每个作业一个目录
//! `<根>/jobs/<作业号>/`，里面有 `script.sh`、`pid`、`log`、`phase`（当前阶段，脚本自己写）与 `exit_code`。
//! 作业号由本地生成；关掉应用也能凭它接着查状态、看日志、取消。

use anyhow::{bail, Context, Result};
use serde::Serialize;

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

/// 提交作业的远程脚本：写下作业脚本，脱离 ssh 会话在后台跑，回报 pid。
/// 作业脚本里可以 `echo 阶段 > phase` 报告进度；结束时写下退出码。
pub fn submit_script(root: &str, id: &str, body: &str) -> String {
    let dir = quote(&job_dir(root, id));
    format!(
        r#"set -e
mkdir -p {dir}
cd {dir}
cat > script.sh <<'COLM_JOB_EOF'
#!/bin/bash
cd "$(dirname "$0")"
(
{body}
)
echo $? > exit_code
COLM_JOB_EOF
chmod +x script.sh
setsid nohup ./script.sh > log 2>&1 < /dev/null &
echo $! > pid
cat pid
"#
    )
}

/// 作业状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    Running,
    Finished {
        exit_code: i32,
    },
    /// 进程不在了却没留下退出码（机器重启、被杀）。
    Lost,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub state: State,
    pub phase: Option<String>,
    pub log_tail: String,
}

pub fn status_script(root: &str, id: &str, lines: usize) -> String {
    let dir = quote(&job_dir(root, id));
    format!(
        r#"cd {dir} 2>/dev/null || {{ echo "state=unknown"; exit 0; }}
if [ -f exit_code ]; then echo "state=finished $(cat exit_code)"
elif [ -f pid ] && kill -0 "$(cat pid)" 2>/dev/null; then echo "state=running"
else echo "state=lost"; fi
echo "phase=$(tail -n 1 phase 2>/dev/null)"
echo "---log---"
tail -n {lines} log 2>/dev/null
"#
    )
}

pub fn parse_status(text: &str) -> Status {
    let (head, log) = text.split_once("---log---\n").unwrap_or((text, ""));
    let mut state = State::Unknown;
    let mut phase = None;
    for line in head.lines() {
        if let Some(value) = line.strip_prefix("state=") {
            state = match value.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["running"] => State::Running,
                ["lost"] => State::Lost,
                ["finished", code] => State::Finished {
                    exit_code: code.parse().unwrap_or(-1),
                },
                _ => State::Unknown,
            };
        } else if let Some(value) = line.strip_prefix("phase=") {
            phase = Some(value.trim().to_owned()).filter(|p| !p.is_empty());
        }
    }
    Status {
        state,
        phase,
        log_tail: log.to_owned(),
    }
}

/// 结束整个进程组（`setsid` 让作业自成一组）。
pub fn cancel_script(root: &str, id: &str) -> String {
    let dir = quote(&job_dir(root, id));
    format!(
        r#"cd {dir} || exit 1
P=$(cat pid)
kill -TERM -- -"$P" 2>/dev/null || kill -TERM "$P" 2>/dev/null
sleep 2
kill -0 "$P" 2>/dev/null && kill -KILL -- -"$P" 2>/dev/null
[ -f exit_code ] || echo 143 > exit_code
echo cancelled
"#
    )
}

pub fn submit(ssh: &Ssh, root: &str, id: &str, body: &str) -> Result<u32> {
    check_id(id)?;
    let out = ssh.run_ok(&submit_script(root, id, body))?;
    out.trim()
        .lines()
        .last()
        .and_then(|l| l.trim().parse().ok())
        .context("the job did not report a process id")
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
