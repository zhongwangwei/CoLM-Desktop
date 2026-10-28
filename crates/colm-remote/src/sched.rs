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
    /// 每个 MPI 进程用的核数（线程数；至少 1）。
    pub cpus: u32,
    /// MPI 进程数；0 或 1 表示不用 MPI。
    pub ranks: u32,
    /// 占几个节点；0 表示不指定（Slurm 自己分配；PBS、LSF 按一个节点算）。进程数要能被它整除。
    pub nodes: u32,
    /// 每个节点的核数（0 = 不知道）。节点数没指定、总核数又超过它时，由 `planned_nodes` 算出要占几个节点。
    pub cores_per_node: u32,
    /// 每个节点的内存（GB）。
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
    /// 作业实际占几个节点；0 表示不指定。指定了节点数就用它；没指定时，总核数（每进程核数 × 进程数）
    /// 不超过一个节点的核数就是 0，超过了就取能让进程数整除、且每节点不超过 `cores_per_node` 的最少节点数。
    pub fn planned_nodes(&self) -> Result<u32> {
        if self.nodes > 0 {
            return Ok(self.nodes);
        }
        let ranks = self.ranks.max(1);
        let total = self.cpus.max(1) * ranks;
        let per_node = self.cores_per_node;
        if per_node == 0 || total <= per_node {
            return Ok(0);
        }
        ensure!(
            ranks > 1,
            "a single process asks for {total} cores but a node has {per_node}; lower the threads or use MPI ranks"
        );
        ensure!(
            self.cpus <= per_node,
            "each MPI process asks for {} cores but a node has {per_node}",
            self.cpus
        );
        // Never more than twice the minimum: a prime rank count would otherwise end up one per node.
        let first = total.div_ceil(per_node);
        (first..=ranks.min(first * 2))
            .find(|nodes| ranks % nodes == 0)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the {ranks} MPI ranks cannot be spread evenly over nodes of {per_node} cores"
                )
            })
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.cpus >= 1, "cpus must be at least 1");
        self.planned_nodes()?;
        ensure!(
            self.nodes == 0 || self.ranks.max(1) % self.nodes == 0,
            "the {} MPI ranks cannot be spread evenly over {} nodes",
            self.ranks.max(1),
            self.nodes
        );
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
    let nodes = resources.planned_nodes()?;
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
            if nodes > 0 {
                lines.push(format!("#SBATCH --nodes={nodes}"));
            } else if resources.ranks <= 1 {
                lines.push("#SBATCH --nodes=1".into());
            }
            lines.push(format!("#SBATCH --ntasks={}", resources.ranks.max(1)));
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
            let nodes = nodes.max(1);
            let per_node = resources.ranks.max(1) / nodes;
            let mut select = format!(
                "select={nodes}:ncpus={}:mpiprocs={per_node}",
                resources.cpus * per_node
            );
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
            lines.push(format!(
                "#BSUB -n {}",
                resources.cpus * resources.ranks.max(1)
            ));
            if nodes > 1 {
                let per_node = resources.cpus * resources.ranks.max(1) / nodes;
                lines.push(format!("#BSUB -R \"span[ptile={per_node}]\""));
            } else {
                lines.push("#BSUB -R \"span[hosts=1]\"".into());
            }
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
///
/// 有些集群（摆渡船）的 `/etc/profile` 会把 PATH 整个重设；服务器配置的环境准备（`export PATH=…`）
/// 已经在前面跑过，所以读完 profile 后把原来的 PATH 放回最前。
pub const PATH_PRELUDE: &str = r#"if ! { command -v sbatch >/dev/null 2>&1 || command -v qsub >/dev/null 2>&1 || command -v bsub >/dev/null 2>&1; }; then
  COLM_FLAGS=$-; COLM_PATH_BEFORE=$PATH; set +e; [ -f /etc/profile ] && . /etc/profile >/dev/null 2>&1
  PATH="$COLM_PATH_BEFORE:$PATH"; export PATH
  case "$COLM_FLAGS" in *e*) set -e;; esac
fi
"#;

// 一份文本两处用（单站点提交、打包的载体作业），`concat!` 只认字面量，所以写成宏。
macro_rules! slurm_submit {
    () => {
        r#"colm_sbatch() {
  COLM_MEM=
  if ! grep -Eq '^#SBATCH --mem(-per-cpu|-per-gpu)?[= ]' "$1"; then
    COLM_PART=$(sed -n 's/^#SBATCH --partition=//p' "$1" | head -n 1)
    if [ -n "$COLM_PART" ]; then
      COLM_LINE=$(scontrol -o show partition "$COLM_PART" 2>/dev/null | head -n 1)
    else
      COLM_LINE=$(scontrol -o show partition 2>/dev/null | grep Default=YES | head -n 1)
    fi
    case "$COLM_LINE" in
    *OverSubscribe=EXCLUSIVE*|*Shared=EXCLUSIVE*) ;;
    *DefMemPerNode=UNLIMITED*)
      COLM_MB=$(sinfo -h ${COLM_PART:+-p "$COLM_PART"} -o '%m %c' 2>/dev/null | tr -d '+' | awk '$2 > 0 { v = int($1 / $2); if (min == "" || v < min) min = v } END { if (min > 0) print min }')
      [ -n "$COLM_MB" ] && COLM_MEM="--mem-per-cpu=${COLM_MB}M" ;;
    esac
  fi
  OUT=$(sbatch --parsable $COLM_MEM ${COLM_SBATCH_EXTRA:-} "$1" 2>&1) || {
    W=$(pwd -P); ID=
    for colm_try in 1 2 3; do
      ID=$( { squeue -h -u "$(id -un)" -o '%i|%Z' 2>/dev/null; sacct -n -X -P -u "$(id -un)" -S now-1hours -o JobID,WorkDir 2>/dev/null; } | awk -F'|' -v w="$W" '$2 == w { print $1 }' | sort -u)
      [ -n "$ID" ] && break
      [ "$colm_try" = 3 ] || sleep 2
    done
    case "$ID" in ''|*[!A-Za-z0-9._-]*) echo "sbatch failed: $OUT" >&2; return 1;; esac
    echo "sbatch reported an error but the job was accepted ($ID): $OUT" >&2
    OUT=$ID
  }
  ID=${OUT%%;*}
  case "$ID" in ''|*[!A-Za-z0-9._-]*) echo "sbatch gave no job id: $OUT" >&2; return 1;; esac
  return 0
}
"#
    };
}
/// `colm_sbatch 脚本`：在脚本所在的当前目录提交，成功时把作业号放进 `$ID`。
///
/// 脚本没写内存时，分区的默认内存若是"整个节点"（`DefMemPerNode=UNLIMITED`，没有 `DefMemPerCPU`），
/// 就按节点内存 ÷ CPU 数（取分区里最小的节点）给 `--mem-per-cpu`。land02 实测（Slurm 19.05，
/// `CR_CORE_MEMORY`，2026-10-19）：不写内存的作业拿走整个节点的内存（4 核的作业占 1031000 MB），
/// 一个只要 1 核的站点就把整个节点挡住，90 个站点最多同时跑十几个，还挡住别人的作业。独占分区不加：
/// 那里整节点本来就归这个作业。
///
/// 天河实测（Slurm 23.11，2026-10-19）：同时提交 5 个，slurmctld 都收下并开始运行了，sbatch 却报
/// "Unexpected message received" 退出非零，`sched_id` 没写，应用只能标"待核对"、整批停下。作业目录
/// 只属于这一次提交，所以 sbatch 报错时先按工作目录找：squeue 找排队和运行中的，sacct 找已经跑完的
/// （实测 47 s 就结束的也有）。恰好找到一个才算提交成功；找到多个或一个都没有，仍按失败报告，不猜。
pub(crate) const SLURM_SUBMIT: &str = slurm_submit!();

const SLURM_LAUNCH: &str = concat!(
    "echo slurm > scheduler\n",
    slurm_submit!(),
    "colm_sbatch script.sh || exit 1\necho \"$ID\" > sched_id\necho \"$ID\"\n"
);

/// 提交的一段（`script.sh` 已写好，当前目录是作业目录）：成功时最后一行输出调度器给的作业号。
pub fn launch(scheduler: Scheduler) -> &'static str {
    match scheduler {
        Scheduler::Bare => {
            "setsid nohup ./script.sh > log 2>&1 < /dev/null &\necho $! > pid\ncat pid\n"
        }
        Scheduler::Slurm => SLURM_LAUNCH,
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

/// 状态脚本里“没有 `exit_code` 时问调度器”的一段。
///
/// 输出契约（[`crate::job::parse_report`] 消费）：
/// - `state=queued|running`：作业还在队列里或在运行，`detail` 是调度系统的原始状态词；
/// - `state=lost`：调度器回答了“没有这个作业”或给出了结束状态，`detail` 是它的说法；
/// - `state=unknown` + `query=failed <原因>`：查询命令本身没能回答（调度器不可用、命令不存在、
///   需要的那条命令失败）。这不是“作业没了”，解析层会把它变成错误，比较页保留作业身份；
/// - `terminal=confirmed`：调度系统确认作业已经结束（CANCELLED/FAILED/COMPLETED…），只给占位判断用。
///
/// 为什么把失败和“查无记录”分得这么细：以前 squeue/sacct 失败时两者都输出空，脚本照样 exit 0，
/// 状态就落成 lost；比较页连续几次之后把作业判失败、删掉身份，服务器侧占位也被当成“已结束”清掉
/// （审查报告第 6 条）。命令非零退出有时只是“这个作业号已经不在”，所以先看它输出的说法，只有明确说
/// 没有这个作业才算回答（`job_not_found`），其余都按查询失败上报。
pub const QUERY: &str = r#"S=$(cat scheduler 2>/dev/null || echo bare)
ID=$(cat sched_id 2>/dev/null)
one_line() {
  printf '%s' "$1" | tr '\n\r' '  '
}
query_failed() {
  echo "state=unknown"
  echo "query=failed $*"
}
# 调度系统明确给出的终态词：确认作业已经结束，但不编造退出码（COMPLETED 的退出码可能还没经 NFS
# 传回来，前端等 exit_code 出现后再判成功/失败）。
lost_confirmed() {
  echo "state=lost"
  echo "detail=$1"
  echo "terminal=confirmed"
}
# 调度器回答了“没有这个作业”，但没给结束方式：确实查不到，不是查询失败，也不能据此判终态。
lost_unconfirmed() {
  echo "state=lost"
  [ -n "$1" ] && echo "detail=$1"
  true
}
# 非零退出时用来判断“是不是只是没有这个作业号”：各家把这句话放在输出里（常是 stderr）。
job_not_found() {
  case "$1" in
  *"Invalid job id"*|*"Unknown Job Id"*|*"Job has finished"*|*"is not found"*) return 0 ;;
  esac
  return 1
}
# 调度系统的作业号还没写下来（提交正写到一半、并发查询撞上这个窗口）：身份不完整，不是“没有这个
# 作业”。bare/pack 不用 $ID，不适用。
case "$S" in
bare|*-pack) ;;
*) case "$ID" in ''|*[!A-Za-z0-9._-]*) echo "state=unknown"; S=incomplete ;; esac ;;
esac
case "$S" in
bare)
  # pid 文件还没出现时作业可能正在提交（占位已建、launch 还没落盘）：这时既不是 running 也不是 lost。
  P=$(cat pid 2>/dev/null)
  case "$P" in
  ''|*[!0-9]*) echo "state=unknown" ;;
  *) if kill -0 "$P" 2>/dev/null; then echo "state=running"; else lost_confirmed "the job process is gone"; fi ;;
  esac ;;
slurm)
  # squeue 管“还在不在队列”，sacct 管“怎么结束的”。两条都正常回答才算查无记录；需要的那条命令
  # 答不上来时只能报查询失败（尤其是 squeue 失败、sacct 无记录：作业可能还在跑，sacct 里没有）。
  Q=$(squeue -h -j "$ID" -o '%T|%r' 2>/dev/null); SQ=$?
  SQERR=
  if [ "$SQ" -ne 0 ]; then
    SQERR=$(one_line "$(squeue -h -j "$ID" -o '%T|%r' 2>&1)")
    if job_not_found "$SQERR"; then SQ=0; Q=; fi
  fi
  if [ "$SQ" -eq 0 ] && [ -n "$Q" ]; then
    # 状态带上原因（`PENDING|Priority`）：排队时界面要说清楚是在等优先级更高的作业还是在等资源。
    Q=$(printf '%s\n' "$Q" | head -n 1)
    case "${Q%%|*}" in
    PENDING|CONFIGURING|REQUEUED|REQUEUE_HOLD|RESV_DEL_HOLD|SPECIAL_EXIT) echo "state=queued";;
    # squeue 偶尔也会直接给出终态词（取消在途）：那是调度器确认结束，不是"在运行"。
    CANCELLED|FAILED|COMPLETED|TIMEOUT|OUT_OF_MEMORY|NODE_FAIL|PREEMPTED|BOOT_FAIL|DEADLINE) lost_confirmed "$Q"; Q= ;;
    *) echo "state=running";;
    esac
    [ -n "$Q" ] && echo "detail=$Q"
  else
    # sacct 不写 -S 时默认只看"今天"，跨夜结束的作业会查成"没有记录"；比页面身份更久的作业
    # 本来也不该再被当成还在跑，所以固定回看 7 天（slurmdbd 的默认查询范围足够大）。
    H=$(sacct -n -X -P -j "$ID" -S now-7days -o State,ExitCode 2>/dev/null); SH=$?
    HERR=
    if [ "$SH" -ne 0 ]; then
      HERR=$(one_line "$(sacct -n -X -P -j "$ID" -S now-7days -o State,ExitCode 2>&1)")
      if job_not_found "$HERR"; then SH=0; H=; fi
    fi
    if [ "$SH" -ne 0 ]; then
      query_failed "squeue: ${SQERR:-no answer}; sacct: $HERR"
    elif [ -n "$H" ]; then
      H=$(printf '%s\n' "$H" | head -n 1)
      case "$H" in
      *CANCELLED*|*FAILED*|*COMPLETED*|*TIMEOUT*|*OUT_OF_MEMORY*|*NODE_FAIL*|*PREEMPTED*|*BOOT_FAIL*|*DEADLINE*) lost_confirmed "$H" ;;
      *) lost_unconfirmed "$H" ;;
      esac
    elif [ "$SQ" -eq 0 ]; then
      # 两个命令都正常回答且都没有记录：确实查不到（不是查询失败），但也没确认怎么结束的。
      lost_unconfirmed ""
    else
      query_failed "squeue did not answer and sacct has no record: $SQERR"
    fi
  fi ;;
pbs)
  QF=$(qstat -f "$ID" 2>/dev/null); SQ=$?
  QERR=
  if [ "$SQ" -ne 0 ]; then
    QERR=$(one_line "$(qstat -f "$ID" 2>&1)")
    if job_not_found "$QERR"; then SQ=0; QF=; fi
  fi
  QSTATE=$(printf '%s\n' "$QF" | sed -n 's/^ *job_state = *//p' | head -n 1)
  case "$QSTATE" in
  Q|H|W|T|S) echo "state=queued"; echo "detail=$QSTATE" ;;
  R|E|B) echo "state=running"; echo "detail=$QSTATE" ;;
  *)
    # 活动查询没有答案，或者它给的是 C/F/X 这类已经结束的状态：再去 qstat -x 找结束方式。
    HF=$(qstat -xf "$ID" 2>/dev/null); SH=$?
    HERR=
    if [ "$SH" -ne 0 ]; then
      HERR=$(one_line "$(qstat -xf "$ID" 2>&1)")
      if job_not_found "$HERR"; then SH=0; HF=; fi
    fi
    HSTATE=$(printf '%s\n' "$HF" | sed -n 's/^ *job_state = *//p' | head -n 1)
    HEXIT=$(printf '%s\n' "$HF" | sed -n 's/^ *Exit_status = */exit /p' | head -n 1)
    if [ "$SH" -ne 0 ]; then
      if [ "$SQ" -eq 0 ] && [ -n "$QSTATE" ]; then
        # 活动查询自己说了 C/F/X：调度器已经确认结束，历史查询失败不改变这一点。
        case "$QSTATE" in C|F|X) lost_confirmed "$QSTATE" ;; *) lost_unconfirmed "$QSTATE" ;; esac
      else
        query_failed "qstat: ${QERR:-no answer}; qstat -xf: $HERR"
      fi
    elif [ -n "$HSTATE" ]; then
      case "$HSTATE" in
      C|F|X)
        case "$HEXIT" in
        "exit 0"|"") lost_confirmed "$HSTATE${HEXIT:+ $HEXIT}" ;;
        *)
          # 取消请求被收下之后出现的非零退出就是它在确认取消；没发过取消那是作业自己失败。
          if [ -f cancel_requested ]; then lost_confirmed "CANCELLED by request (PBS $HSTATE $HEXIT)"
          else lost_confirmed "FAILED (PBS $HSTATE $HEXIT)"; fi ;;
        esac ;;
      *) lost_unconfirmed "$HSTATE${HEXIT:+ $HEXIT}" ;;
      esac
    elif [ "$SQ" -eq 0 ] && [ -z "$QSTATE" ]; then
      lost_unconfirmed ""
    else
      query_failed "qstat did not answer and qstat -xf has no record: $QERR"
    fi ;;
  esac ;;
lsf)
  Q=$(bjobs -noheader -o stat "$ID" 2>/dev/null); SQ=$?
  if [ "$SQ" -ne 0 ]; then
    QERR=$(one_line "$(bjobs -noheader -o stat "$ID" 2>&1)")
    if job_not_found "$QERR"; then
      lost_unconfirmed "$QERR"
    else
      query_failed "bjobs did not answer (exit $SQ): $QERR"
    fi
  else
    Q=$(printf '%s\n' "$Q" | head -n 1 | tr -d ' ')
    case "$Q" in
    PEND|PSUSP) echo "state=queued"; echo "detail=$Q" ;;
    RUN|USUSP|SSUSP|WAIT|PROV) echo "state=running"; echo "detail=$Q" ;;
    # LSF 的终态词：DONE=正常结束，EXIT=非零退出。取消请求被收下之后出现的 EXIT 是它确认取消的方式。
    DONE*) lost_confirmed "COMPLETED (LSF $Q)" ;;
    EXIT*)
      if [ -f cancel_requested ]; then lost_confirmed "CANCELLED by request (LSF $Q)"
      else lost_confirmed "FAILED (LSF $Q)"; fi ;;
    '') lost_unconfirmed "" ;;
    *) lost_unconfirmed "$Q" ;;
    esac
  fi ;;
*-pack)
  # 打包运行（`crate::pack`）：看站点被哪个载体领走、那个载体还在不在。
  COLM_BASE=${S%-pack}
  P=$(cat pool 2>/dev/null); J=${PWD##*/}
  if [ -z "$P" ]; then echo "state=unknown"
  elif [ -f "$P/claimed/$J/carrier" ]; then
    C=$(cat "$P/claimed/$J/carrier"); Q=$(colm_alive "$C")
    if [ -n "$Q" ]; then echo "state=running"; echo "detail=node pack job $C $Q"
    elif [ "$COLM_BASE" = slurm ]; then echo "state=lost"; echo "detail=node pack job $C $(sacct -n -X -P -j "$C" -o State,ExitCode 2>/dev/null | head -n 1)"
    # 载体确实结束了，站点却没写下退出码：它被连带停掉了（超时、节点故障），这个站点不会再跑完。
    elif [ -n "$(colm_gone "$C")" ]; then echo "state=lost"; echo "detail=node pack job $C ended before this station finished: FAILED"
    else echo "state=lost"; echo "detail=node pack job $C is not answering"; fi
  elif [ -f "$P/claimed/$J/cancelled" ]; then echo "state=lost"; echo "detail=cancelled before it started"
  elif [ -d "$P/claimed/$J" ]; then echo "state=queued"; echo "detail=starting in node pack"
  elif [ -f "$P/queue/$J" ]; then
    L=
    for c in "$P"/carriers/c*; do
      [ -f "$c/sched_id" ] && [ ! -f "$c/closing" ] || continue
      Q=$(colm_alive "$(cat "$c/sched_id")")
      if [ -n "$Q" ]; then L="$L $(cat "$c/sched_id") $Q"; fi
    done
    if [ -n "$L" ]; then echo "state=queued"; echo "detail=waiting in node pack:$L"
    else echo "state=lost"; echo "detail=node pack has no job left to run it"; fi
  else echo "state=lost"; echo "detail=not in the node pack queue"; fi ;;
*) echo "state=unknown" ;;
esac
"#;

/// 取消：调度系统的作业交给它结束；后台进程则结束整个进程组。
///
/// 调度器接受取消请求不等于作业已经终止：它可能还在队列里、还在运行。所以 slurm/pbs/lsf 只在请求
/// 被收下后写下 `cancel_requested` 标记（状态查询据此显示"正在取消"），绝不伪造退出码或终态；
/// 只有后面的状态查询看到调度器确认 CANCELLED/FAILED/COMPLETED 或退出码，才算作业结束。没有调度
/// 系统时 `bare` 是亲手确认进程组不在了才写 `exit_code`+`cancelled`，那是确认过的终态。
pub const CANCEL: &str = r#"S=$(cat scheduler 2>/dev/null || echo bare)
[ -f exit_code ] && { echo finished; exit 0; }
ID=$(cat sched_id 2>/dev/null || true)
case "$S" in
slurm) scancel "$ID" || exit $?; : > cancel_requested ;;
*-pack)
  # 不能取消整个载体：它上面还跑着别的站点。没领走的抢先占住，领走了的请载体只杀这一个。
  P=$(cat pool) || exit 1; J=${PWD##*/}
  if mkdir "$P/claimed/$J" 2>/dev/null; then : > "$P/claimed/$J/cancelled"; else : > "$P/claimed/$J/cancel"; fi ;;
pbs) qdel "$ID" || exit $?; : > cancel_requested ;;
lsf) bkill "$ID" || exit $?; : > cancel_requested ;;
bare)
  P=$(cat pid) || exit 1
  case "$P" in ''|*[!0-9]*|0|1) echo "invalid job pid" >&2; exit 1;; esac
  START=$(sed 's/.*) //' "/proc/$P/stat" | awk '{print $20}')
  [ -n "$START" ] || { echo "job process start identity missing" >&2; exit 1; }
  [ "$(ps -o uid= -p "$P" | tr -d ' ')" = "$(id -u)" ] || { echo "job process owner changed" >&2; exit 1; }
  [ "$(readlink "/proc/$P/cwd")" = "$(pwd -P)" ] || { echo "job process directory changed" >&2; exit 1; }
  [ "$(ps -o sid= -p "$P" | tr -d ' ')" = "$P" ] || { echo "job process session changed" >&2; exit 1; }
  tr '\000' '\n' < "/proc/$P/cmdline" | grep -Fxq -- './script.sh' || { echo "job process command changed" >&2; exit 1; }
  [ "$(sed 's/.*) //' "/proc/$P/stat" | awk '{print $20}')" = "$START" ] || { echo "job process was replaced" >&2; exit 1; }
  kill -TERM -- -"$P" 2>/dev/null || exit 1
  sleep 2
  if kill -0 -- -"$P" 2>/dev/null; then
    # A vanished group leader cannot be re-authenticated for escalation.
    [ "$(sed 's/.*) //' "/proc/$P/stat" | awk '{print $20}')" = "$START" ] &&
      [ "$(readlink "/proc/$P/cwd")" = "$(pwd -P)" ] &&
      [ "$(ps -o sid= -p "$P" | tr -d ' ')" = "$P" ] || { echo "job identity changed during cancellation" >&2; exit 1; }
    kill -KILL -- -"$P" 2>/dev/null || exit 1
    sleep 1
    if kill -0 -- -"$P" 2>/dev/null; then echo "job group still exists" >&2; exit 1; fi
  fi
  # 是我们停下的：退出码 143 之外再记一笔，状态说"已停止"而不是"失败"。
  [ -f exit_code ] || { echo 143 > exit_code; : > cancelled; } ;;
*) echo "unknown scheduler: $S" >&2; exit 1 ;;

esac
echo cancellation_requested
"#;

#[cfg(test)]
#[path = "sched_tests.rs"]
mod sched_tests;
