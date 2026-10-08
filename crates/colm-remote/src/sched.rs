//! 调度系统适配（Slurm、PBS、LSF；R2）：只管“提交、查队列、取消”三件事。
//!
//! 作业目录的布局不变（`script.sh`、`log`、`phase`、`exit_code`）：作业脚本自己在结束时写 `exit_code`，所以
//! “是否完成”对所有调度系统一视同仁，调度器只补充“还在排队吗、有没有被它杀掉”。

use anyhow::{bail, ensure, Result};

/// 调度系统。`Bare` 是没有调度系统的机器：`setsid nohup` 直接后台运行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheduler {
    Bare,
    Slurm,
    Pbs,
    Lsf,
}

impl Scheduler {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text.trim().to_ascii_lowercase().as_str() {
            "" | "bare" | "none" => Self::Bare,
            "slurm" => Self::Slurm,
            "pbs" => Self::Pbs,
            "lsf" => Self::Lsf,
            other => bail!("unknown scheduler {other:?}; use bare, slurm, pbs or lsf"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bare => "bare",
            Self::Slurm => "slurm",
            Self::Pbs => "pbs",
            Self::Lsf => "lsf",
        }
    }
}

/// 向调度系统申请的资源。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resources {
    /// 一个节点上的核数（至少 1）。
    pub cpus: u32,
    pub memory_gb: Option<u32>,
    /// `HH:MM:SS`、`D-HH:MM:SS` 或 `HH:MM`。
    pub walltime: Option<String>,
    /// 分区（Slurm）或队列（PBS、LSF）。
    pub partition: Option<String>,
    pub account: Option<String>,
    /// 作业开头先执行的环境准备（`module load …`、`conda activate …`），原样写进脚本。
    pub env_script: Option<String>,
    /// 原样追加的指令选项（例如 `--constraint=cpu`），每条一行，不带 `#SBATCH` 之类的前缀。
    pub directives: Vec<String>,
}

/// 把时限解析成秒。
pub fn parse_walltime(text: &str) -> Result<u64> {
    let text = text.trim();
    let (days, rest) = match text.split_once('-') {
        Some((d, rest)) => (d.parse::<u64>().ok(), rest),
        None => (Some(0), text),
    };
    let days = days.ok_or_else(|| anyhow::anyhow!("bad walltime {text:?}"))?;
    let parts: Vec<Option<u64>> = rest.split(':').map(|p| p.parse().ok()).collect();
    let numbers: Option<Vec<u64>> = parts.into_iter().collect();
    let numbers = numbers.ok_or_else(|| anyhow::anyhow!("bad walltime {text:?}"))?;
    let seconds = match numbers.as_slice() {
        [h, m] => h * 3600 + m * 60,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => bail!("walltime must look like HH:MM:SS, D-HH:MM:SS or HH:MM, got {text:?}"),
    };
    let total = days * 86_400 + seconds;
    ensure!(total > 0, "walltime must be longer than zero");
    Ok(total)
}

fn clock(seconds: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// 名字、分区、账户之类：不许空白与前导 `-`，免得拼成指令后改变含义。
fn word(what: &str, text: &str) -> Result<()> {
    ensure!(
        !text.is_empty()
            && !text.starts_with('-')
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-@:+/".contains(c)),
        "{what} {text:?} may only contain letters, digits and _ . - @ : + /"
    );
    Ok(())
}

impl Resources {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.cpus >= 1, "cpus must be at least 1");
        if let Some(seconds) = &self.walltime {
            parse_walltime(seconds)?;
        }
        if let Some(partition) = &self.partition {
            word("partition", partition)?;
        }
        if let Some(account) = &self.account {
            word("account", account)?;
        }
        for line in &self.directives {
            ensure!(
                line.starts_with('-') && !line.contains(['\n', '\r']),
                "a directive must be one line starting with '-', got {line:?}"
            );
        }
        if let Some(env) = &self.env_script {
            ensure!(
                !env.contains("COLM_JOB_EOF"),
                "the environment script may not contain the job terminator"
            );
        }
        Ok(())
    }
}

/// 作业名：字母数字与 `_.-`，最多 15 个字符（PBS 的老限制）。
pub fn job_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = format!("colm_{cleaned}");
    name.chars().take(15).collect()
}

/// 脚本头部的调度指令（每行一条，已含前缀）。`Bare` 没有。
pub fn directives(
    scheduler: Scheduler,
    name: &str,
    log: &str,
    resources: &Resources,
) -> Result<Vec<String>> {
    resources.validate()?;
    let seconds = resources
        .walltime
        .as_deref()
        .map(parse_walltime)
        .transpose()?;
    let mut lines = Vec::new();
    match scheduler {
        Scheduler::Bare => {}
        Scheduler::Slurm => {
            lines.push(format!("#SBATCH --job-name={name}"));
            lines.push(format!("#SBATCH --output={log}"));
            lines.push(format!("#SBATCH --error={log}"));
            lines.push("#SBATCH --nodes=1".into());
            lines.push("#SBATCH --ntasks=1".into());
            lines.push(format!("#SBATCH --cpus-per-task={}", resources.cpus));
            if let Some(gb) = resources.memory_gb {
                lines.push(format!("#SBATCH --mem={gb}G"));
            }
            if let Some(s) = seconds {
                let time = if s >= 86_400 {
                    format!("{}-{}", s / 86_400, clock(s % 86_400))
                } else {
                    clock(s)
                };
                lines.push(format!("#SBATCH --time={time}"));
            }
            if let Some(p) = &resources.partition {
                lines.push(format!("#SBATCH --partition={p}"));
            }
            if let Some(a) = &resources.account {
                lines.push(format!("#SBATCH --account={a}"));
            }
            lines.extend(resources.directives.iter().map(|d| format!("#SBATCH {d}")));
        }
        Scheduler::Pbs => {
            lines.push(format!("#PBS -N {name}"));
            lines.push("#PBS -j oe".into());
            lines.push(format!("#PBS -o {log}"));
            let mut select = format!("select=1:ncpus={}", resources.cpus);
            if let Some(gb) = resources.memory_gb {
                select.push_str(&format!(":mem={gb}gb"));
            }
            lines.push(format!("#PBS -l {select}"));
            if let Some(s) = seconds {
                lines.push(format!("#PBS -l walltime={}", clock(s)));
            }
            if let Some(p) = &resources.partition {
                lines.push(format!("#PBS -q {p}"));
            }
            if let Some(a) = &resources.account {
                lines.push(format!("#PBS -A {a}"));
            }
            lines.extend(resources.directives.iter().map(|d| format!("#PBS {d}")));
        }
        Scheduler::Lsf => {
            lines.push(format!("#BSUB -J {name}"));
            lines.push(format!("#BSUB -oo {log}"));
            lines.push(format!("#BSUB -n {}", resources.cpus));
            lines.push("#BSUB -R \"span[hosts=1]\"".into());
            if let Some(gb) = resources.memory_gb {
                lines.push(format!("#BSUB -R \"rusage[mem={}]\"", u64::from(gb) * 1024));
            }
            if let Some(s) = seconds {
                lines.push(format!(
                    "#BSUB -W {:02}:{:02}",
                    s.div_ceil(60) / 60,
                    s.div_ceil(60) % 60
                ));
            }
            if let Some(p) = &resources.partition {
                lines.push(format!("#BSUB -q {p}"));
            }
            if let Some(a) = &resources.account {
                lines.push(format!("#BSUB -P {a}"));
            }
            lines.extend(resources.directives.iter().map(|d| format!("#BSUB {d}")));
        }
    }
    Ok(lines)
}

/// 找得到调度命令的前置：非交互的 ssh 会话常常没有 `/etc/profile` 里设的 PATH。
pub const PATH_PRELUDE: &str = r#"if ! { command -v sbatch >/dev/null 2>&1 || command -v qsub >/dev/null 2>&1 || command -v bsub >/dev/null 2>&1; }; then
  COLM_FLAGS=$-; set +e; [ -f /etc/profile ] && . /etc/profile >/dev/null 2>&1
  case "$COLM_FLAGS" in *e*) set -e;; esac
fi
"#;

/// 提交的一段（`script.sh` 已写好，当前目录是作业目录）：成功时最后一行输出调度器给的作业号。
pub fn launch(scheduler: Scheduler) -> &'static str {
    match scheduler {
        Scheduler::Bare => {
            "setsid nohup ./script.sh > log 2>&1 < /dev/null &\necho $! > pid\ncat pid\n"
        }
        Scheduler::Slurm => {
            r#"echo slurm > scheduler
OUT=$(sbatch --parsable script.sh 2>&1) || { echo "sbatch failed: $OUT" >&2; exit 1; }
ID=${OUT%%;*}
case "$ID" in ''|*[!A-Za-z0-9._-]*) echo "sbatch gave no job id: $OUT" >&2; exit 1;; esac
echo "$ID" > sched_id
echo "$ID"
"#
        }
        Scheduler::Pbs => {
            r#"echo pbs > scheduler
OUT=$(qsub script.sh 2>&1) || { echo "qsub failed: $OUT" >&2; exit 1; }
ID=$(echo "$OUT" | tail -n 1)
case "$ID" in ''|*[!A-Za-z0-9._-]*) echo "qsub gave no job id: $OUT" >&2; exit 1;; esac
echo "$ID" > sched_id
echo "$ID"
"#
        }
        Scheduler::Lsf => {
            r#"echo lsf > scheduler
OUT=$(bsub < script.sh 2>&1) || { echo "bsub failed: $OUT" >&2; exit 1; }
ID=$(echo "$OUT" | sed -n 's/^Job <\([0-9][0-9]*\)>.*/\1/p' | head -n 1)
[ -n "$ID" ] || { echo "bsub gave no job id: $OUT" >&2; exit 1; }
echo "$ID" > sched_id
echo "$ID"
"#
        }
    }
}

/// 状态脚本里“没有 `exit_code` 时问调度器”的一段，输出 `state=…` 与（可选）`detail=…`。
/// 调度器给的状态词只放进 `detail`，`state` 只有 queued、running、lost 三种。
pub const QUERY: &str = r#"S=$(cat scheduler 2>/dev/null || echo bare)
ID=$(cat sched_id 2>/dev/null)
case "$S" in
bare)
  if [ -f pid ] && kill -0 "$(cat pid)" 2>/dev/null; then echo "state=running"; else echo "state=lost"; fi ;;
slurm)
  Q=$(squeue -h -j "$ID" -o %T 2>/dev/null | head -n 1)
  if [ -z "$Q" ]; then
    echo "state=lost"; echo "detail=$(sacct -n -X -P -j "$ID" -o State,ExitCode 2>/dev/null | head -n 1)"
  else
    case "$Q" in PENDING|CONFIGURING|REQUEUED|REQUEUE_HOLD|RESV_DEL_HOLD|SPECIAL_EXIT) echo "state=queued";; *) echo "state=running";; esac
    echo "detail=$Q"
  fi ;;
pbs)
  Q=$(qstat -f "$ID" 2>/dev/null | sed -n 's/^ *job_state = *//p' | head -n 1)
  if [ -z "$Q" ]; then
    echo "state=lost"; echo "detail=$(qstat -xf "$ID" 2>/dev/null | sed -n 's/^ *job_state = *//p;s/^ *Exit_status = */exit /p' | tr '\n' ' ')"
  else
    case "$Q" in Q|H|W|T|S) echo "state=queued";; *) echo "state=running";; esac
    echo "detail=$Q"
  fi ;;
lsf)
  Q=$(bjobs -noheader -o stat "$ID" 2>/dev/null | head -n 1 | tr -d ' ')
  case "$Q" in
  PEND|PSUSP) echo "state=queued"; echo "detail=$Q";;
  RUN|USUSP|SSUSP|WAIT|PROV) echo "state=running"; echo "detail=$Q";;
  *) echo "state=lost"; echo "detail=$Q";;
  esac ;;
*) echo "state=unknown" ;;
esac
"#;

/// 取消脚本里调度器的那一段：先让调度器去结束，再留一个退出码，免得状态一直停在“丢失”。
pub const CANCEL: &str = r#"S=$(cat scheduler 2>/dev/null || echo bare)
ID=$(cat sched_id 2>/dev/null)
case "$S" in
slurm) scancel "$ID" ;;
pbs) qdel "$ID" ;;
lsf) bkill "$ID" ;;
*)
  P=$(cat pid)
  kill -TERM -- -"$P" 2>/dev/null || kill -TERM "$P" 2>/dev/null
  sleep 2
  kill -0 "$P" 2>/dev/null && kill -KILL -- -"$P" 2>/dev/null ;;
esac
[ -f exit_code ] || echo 143 > exit_code
echo cancelled
"#;

#[cfg(test)]
#[path = "sched_tests.rs"]
mod sched_tests;
