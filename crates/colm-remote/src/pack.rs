//! 按节点打包站点（Slurm、LSF、PBS，以及没有调度系统的机器）。
//!
//! 天河的 deimos 分区是 `OverSubscribe=EXCLUSIVE`：不管要几个核，一个作业就占一整个节点、按整节点计费。
//! 每个站点一个作业时，90 个站点占 90 个节点；sacct 实测 AU-How 跑 5 分 27 秒，记 5 小时 48 分核时
//! （2026-10-19）。所以站点不再各交一个作业，而是放进本批的队列
//! `<根>/pools/<批次>/queue/`，由"载体"作业去领：一个载体占一个节点，同时跑到这个节点的核数，
//! 跑完一个领下一个，队列空了再等一会儿就退出。90 个站点、64 核的节点，就是两个载体。
//!
//! 每个站点仍有自己的作业目录（`script.sh`、`log`、`phase`、`exit_code` 都不变），所以查状态、看日志、
//! 取回结果和单站点时一样；作业目录里 `scheduler` 是 `slurm-pack`，`pool` 是队列目录，`sched_id` 是批次。
//! 领取用 `mkdir claimed/<站点>`（原子操作），所以一个站点只会被一个载体跑一次；取消一个还没领的站点
//! 也是抢先建这个目录，载体就不会再跑它。
//!
//! 共享节点的分区也打包：land02 的 faculty 账户每人同时只能跑 3 个作业（`MaxJobs=3`，2026-10-19），
//! 一个站点一个作业时 90 个站点 3 个 3 个地挪。那里载体要明确申请整个节点的逻辑 CPU（超线程的节点
//! 96 个 CPU = 48 核），站点按物理核数放；内存由 `colm_sbatch` 按核补上。
//!
//! 用户要求不管什么调度系统都打包（2026-10-19）：LSF（摆渡船）用 `bsub -n 核数 -R span[hosts=1]`、PBS 用
//! `qsub -l select=1:ncpus=核数` 交载体；没有调度系统的机器直接在后台起一个载体进程，按机器的核数同时跑。
//! 不同调度系统只差交载体、判断载体还在不在、载体的作业号从哪个环境变量来这几处；排队、领取、收尾、
//! 取消都一样。作业目录里的 `scheduler` 是 `<调度系统>-pack`。
use anyhow::{ensure, Result};

use crate::sched::{self, Resources, Scheduler};
use crate::ssh::quote;

/// 一批站点共用的队列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// 批次号（应用里一次批量运行的编号），也是队列目录名。
    pub pool: String,
    /// 每个载体同时跑几个站点；0 = 用节点的核数（Slurm 按 `sinfo` 估，其它按机器的核数）。服务器设置里填了
    /// 每节点核数时由调用方传进来（摆渡船 48）。
    pub slots: u32,
}

pub fn pool_dir(root: &str, pool: &str) -> String {
    format!("{}/pools/{pool}", root.trim_end_matches('/'))
}

/// 载体作业的脚本。当前目录是它自己的目录（sbatch 在那里提交）。
///
/// 只用 bash 3 也有的写法：关联数组、`wait -n` 都没用，测试在 macOS 自带的 bash 3.2 上跑。
/// 每个站点用 `setsid` 起在自己的进程组里，取消时只杀这一组；站点脚本照常自己写 `exit_code`，
/// 被杀掉没写成时，由载体把它实际的退出状态补上（这是观察到的结果，不是编造的）。
fn carrier_script(pool: &str, jobs: &str, slots: u32, header: &[String]) -> String {
    let mut script = String::from("#!/bin/bash\n");
    for line in header {
        script.push_str(line);
        script.push('\n');
    }
    script.push_str(&format!(
        r#"cd "${{SLURM_SUBMIT_DIR:-${{PBS_O_WORKDIR:-${{LS_SUBCWD:-.}}}}}}"
P={pool}; JOBS={jobs}
SLOTS={slots}
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=$(cat "$P/slots" 2>/dev/null)
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=${{SLURM_CPUS_ON_NODE:-$(nproc 2>/dev/null || echo 1)}}
LINGER=${{COLM_PACK_LINGER:-180}}
# 自己的作业号：各调度系统的环境变量；没有调度系统时是自己的进程号（启动它的一方记下的正是这个）。
ME=${{SLURM_JOB_ID:-${{LSB_JOBID:-${{PBS_JOBID:-$$}}}}}}
mkdir -p running done pg term
idle=0
while :; do
  n=0
  for r in running/*; do
    [ -e "$r" ] || continue
    j=${{r##*/}}
    if [ -f "done/$j" ]; then
      # 是应请求停下的（term/ 里有它，而且它自己没来得及写退出码）：记一笔，状态才说"已停止"而不是
      # 退出码 143 的"失败"。TERM 发出时恰好自己跑完、写了退出码的，照实按它的退出码算。
      [ -e "term/$j" ] && [ ! -f "$JOBS/$j/exit_code" ] && : > "$JOBS/$j/cancelled"
      [ -f "$JOBS/$j/exit_code" ] || cp "done/$j" "$JOBS/$j/exit_code"
      wait "$(cat "$r")" 2>/dev/null
      rm -f "$r"
      continue
    fi
    n=$((n + 1))
    g=$(cat "pg/$j" 2>/dev/null)
    [ -n "$g" ] || continue
    if [ -e "$P/claimed/$j/cancel" ] && [ ! -e "term/$j" ]; then
      kill -TERM -- "-$g" 2>/dev/null || kill -TERM "$g" 2>/dev/null
      date +%s > "term/$j"
    elif [ -e "term/$j" ] && [ $(($(date +%s) - $(cat "term/$j"))) -ge 10 ]; then
      kill -KILL -- "-$g" 2>/dev/null || kill -KILL "$g" 2>/dev/null
    fi
  done
  while [ "$n" -lt "$SLOTS" ]; do
    j=
    for f in "$P"/queue/*; do
      [ -e "$f" ] || continue
      c=${{f##*/}}
      if mkdir "$P/claimed/$c" 2>/dev/null; then j=$c; break; fi
    done
    [ -n "$j" ] || break
    echo "$ME" > "$P/claimed/$j/carrier"
    ( setsid "$JOBS/$j/script.sh" > "$JOBS/$j/log" 2>&1 < /dev/null &
      echo $! > "pg/$j"
      wait $!
      echo $? > "done/$j" ) &
    echo $! > "running/$j"
    n=$((n + 1))
    idle=0
  done
  if [ "$n" -eq 0 ]; then
    idle=$((idle + 2))
    if [ "$idle" -ge "$LINGER" ]; then
      # 和入队用同一把锁：要么入队的那个站点看见本载体已在收尾、另交一个载体，要么本载体看见它、接着跑。
      exec 8>"$P/.lock"
      flock 8
      left=0
      for f in "$P"/queue/*; do
        [ -e "$f" ] && [ ! -d "$P/claimed/${{f##*/}}" ] && left=1
      done
      if [ "$left" = 0 ]; then
        # 并行文件系统上 flock 未必跨节点生效，所以写下 closing 后再看一次：入队的一方先入队、
        # 后看 closing，两边总有一方看见对方。
        : > closing
        sleep 2
        for f in "$P"/queue/*; do
          [ -e "$f" ] && [ ! -d "$P/claimed/${{f##*/}}" ] && left=1
        done
        [ "$left" = 0 ] && exit 0
        rm -f closing
      fi
      exec 8>&-
      idle=0
    fi
  fi
  sleep 2
done
"#,
        pool = quote(pool),
        jobs = quote(jobs),
    ));
    script
}

/// 判断载体还在不在（排队或运行中）的 shell 函数 `colm_alive 作业号`：还在就输出它的状态词。按
/// `$COLM_BASE`（slurm、lsf、pbs、bare）选问法；打包提交与状态查询共用这一份。
pub(crate) const ALIVE: &str = r#"colm_alive() {
  case "$COLM_BASE" in
  slurm) squeue -h -j "$1" -o %T 2>/dev/null | head -n 1 ;;
  lsf) bjobs -noheader -o stat "$1" 2>/dev/null | awk 'NR == 1 && $1 != "DONE" && $1 != "EXIT" { print $1 }' ;;
  pbs) qstat -f "$1" 2>/dev/null | sed -n 's/^ *job_state = *//p' | head -n 1 | awk '$1 != "F" && $1 != "X" { print $1 }' ;;
  bare) kill -0 "$1" 2>/dev/null && echo RUNNING ;;
  esac
}
# 载体确实已经结束（调度系统明确这么说）时输出 yes。查不到、调度系统没回答都不算：那只是"不知道"，
# 不能据此把站点判成失败。
colm_gone() {
  case "$COLM_BASE" in
  lsf) colm_s=$(bjobs -noheader -o stat "$1" 2>&1); case "$colm_s" in DONE*|EXIT*|*"is not found"*) echo yes ;; esac ;;
  pbs) colm_s=$(qstat -f "$1" 2>&1); case "$colm_s" in *"job_state = F"*|*"job_state = X"*|*"Unknown Job Id"*|*"Job has finished"*) echo yes ;; esac ;;
  bare) kill -0 "$1" 2>/dev/null || echo yes ;;
  esac
}
"#;

/// 交载体（当前目录是载体目录，`script.sh` 已写好），成功时作业号放进 `$ID`。`$SLOTS` 是它要同时跑的
/// 站点数，`$COLM_CPUS` 是一个节点的逻辑 CPU 数，`$COLM_WHOLE` 非空表示分区本来就整节点独占（Slurm）。
fn carrier_submit(scheduler: Scheduler) -> &'static str {
    match scheduler {
        Scheduler::Slurm => {
            "colm_carrier() { COLM_SBATCH_EXTRA=; [ -n \"$COLM_WHOLE\" ] || COLM_SBATCH_EXTRA=\"--cpus-per-task=$COLM_CPUS\"; colm_sbatch script.sh; }\n"
        }
        // 命令行的选项盖过脚本里 `#BSUB` / `#PBS` 写的 1 核。
        Scheduler::Lsf => {
            r#"colm_carrier() {
  OUT=$(bsub -n "$SLOTS" -R "span[hosts=1]" < script.sh 2>&1) || { echo "bsub failed: $OUT" >&2; return 1; }
  ID=$(echo "$OUT" | sed -n 's/^Job <\([0-9][0-9]*\)>.*/\1/p' | head -n 1)
  [ -n "$ID" ] || { echo "bsub gave no job id: $OUT" >&2; return 1; }
}
"#
        }
        Scheduler::Pbs => {
            r#"colm_carrier() {
  OUT=$(qsub -l "select=1:ncpus=$SLOTS" script.sh 2>&1) || { echo "qsub failed: $OUT" >&2; return 1; }
  ID=$(echo "$OUT" | tail -n 1)
  case "$ID" in ''|*[!A-Za-z0-9._-]*) echo "qsub gave no job id: $OUT" >&2; return 1;; esac
}
"#
        }
        Scheduler::Bare => {
            "colm_carrier() { setsid nohup ./script.sh > log 2>&1 < /dev/null & ID=$!; }\n"
        }
    }
}

/// 替代 `sched::launch` 的一段：`script.sh` 已写好，当前目录是站点的作业目录。站点进本批的队列，必要时
/// 再交一个载体，最后一行输出 `pack <批次>`。
pub fn launch(
    root: &str,
    job: &str,
    pack: &Pack,
    scheduler: Scheduler,
    resources: &Resources,
) -> Result<String> {
    crate::job::check_id(job)?;
    crate::job::check_id(&pack.pool)?;
    let pool = pool_dir(root, &pack.pool);
    let jobs = format!("{}/jobs", root.trim_end_matches('/'));
    // 载体只要一个任务一个核：分区独占，Slurm 照样给整个节点，节点的核数在载体里从 SLURM_CPUS_ON_NODE 读。
    let carrier = Resources {
        cpus: 1,
        ranks: 1,
        nodes: 0,
        cores_per_node: 0,
        memory_gb: None,
        walltime: resources.walltime.clone(),
        partition: resources.partition.clone(),
        account: resources.account.clone(),
        env_script: None,
        directives: resources.directives.clone(),
    };
    // 日志写在每个载体自己的目录里（`carriers/cN/carrier.log`），用相对路径：载体都在自己的目录里提交，
    // 三种调度系统都按提交目录解析相对路径。原来写成 `<队列>/carrier-%j.log`：只有 Slurm 认 `%j`，
    // LSF 认 `%J`、PBS 不替换，于是摆渡船上所有载体写进同一个文件、互相覆盖（2026-10-20 核查）。
    // 也不能直接放进 carriers/：那里按 `c*` 数载体（天河实测 `carrier-<号>.log` 也被数了进去）。
    // 没有调度系统时载体自己的输出写在它目录下的 `log`。
    let header = sched::directives(scheduler, &sched::job_name("pack"), "carrier.log", &carrier)?;
    let script = carrier_script(&pool, &jobs, pack.slots, &header);
    ensure!(
        !script.contains("COLM_CARRIER_EOF"),
        "the carrier script may not contain its terminator"
    );
    let slurm = scheduler == Scheduler::Slurm;
    let forced = slurm
        && resources
            .directives
            .iter()
            .any(|d| d == "--exclusive" || d.starts_with("--exclusive="));
    let partition = resources.partition.as_deref().map(quote);
    // 分区要不要整节点：`scontrol -o` 每个分区一行，没指定分区时看默认分区；老版本的 Slurm 叫 Shared。
    let exclusive = if forced {
        "true".to_owned()
    } else if !slurm {
        "false".to_owned()
    } else {
        format!(
            r#"{{ L=$(scontrol -o show partition {part} 2>/dev/null | {pick} | head -n 1); case "$L" in *OverSubscribe=EXCLUSIVE*|*Shared=EXCLUSIVE*) true;; *) false;; esac; }}"#,
            part = partition.clone().unwrap_or_default(),
            pick = if partition.is_some() {
                "cat"
            } else {
                "grep Default=YES"
            },
        )
    };
    // 每个节点：插槽 × 每插槽核数（物理核）与逻辑 CPU 数，取分区里最小的节点。land02 的节点有超线程
    // （96 个逻辑 CPU = 48 核）：站点按物理核放，载体按逻辑 CPU 申请才能拿到这些核。
    // 其它调度系统没有统一的问法：用服务器设置里的每节点核数（`pack.slots`），没填时按这台机器的核数。
    let nodes = match (&partition, slurm) {
        (Some(part), true) => format!("sinfo -h -p {part} -o '%X %Y %c'"),
        (None, true) => "sinfo -h -o '%X %Y %c'".to_owned(),
        (_, false) => {
            "echo \"1 $(nproc 2>/dev/null || echo 1) $(nproc 2>/dev/null || echo 1)\"".to_owned()
        }
    };
    Ok(format!(
        r#"{submit}{alive}{carrier_submit}COLM_BASE={kind}
# 站点批量一律按节点打包：天河整节点独占计费，land02 每人同时只能跑 3 个作业（faculty 账户
# MaxJobs=3，2026-10-19），一个站点一个作业两边都慢。独占分区上载体照样只要 1 核（整节点自然归它），
# 共享分区上要明确申请整个节点的 CPU，并由 colm_sbatch 按核补内存。
COLM_WHOLE=
if {exclusive}; then COLM_WHOLE=1; fi
P={pool}; J={job}
mkdir -p "$P/queue" "$P/claimed" "$P/carriers"
echo {kind}-pack > scheduler
echo "$P" > pool
echo {batch} > sched_id
exec 8>"$P/.lock"
flock 8
SLOTS={slots}
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=$(cat "$P/slots" 2>/dev/null || true)
COLM_SIZES=$({nodes} 2>/dev/null | tr -d '+' | awk '$1 > 0 && $2 > 0 {{ c = $1 * $2; if (cores == "" || c < cores) cores = c; if (cpus == "" || $3 < cpus) cpus = $3 }} END {{ if (cores > 0) print cores, cpus }}')
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=${{COLM_SIZES%% *}}
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=1
COLM_CPUS=${{COLM_SIZES##* }}
[ "$COLM_CPUS" -ge "$SLOTS" ] 2>/dev/null || COLM_CPUS=$SLOTS
echo "$SLOTS" > "$P/slots"
: > "$P/queue/$J"
# 还要几个载体：没领走的站点比活着的载体剩下的位置多，就再交一个。站点一个一个到，载体也一个一个加。
W=0; R=0
for f in "$P"/queue/*; do
  [ -e "$f" ] || continue
  j=${{f##*/}}
  if [ ! -d "$P/claimed/$j" ]; then W=$((W + 1))
  elif [ -f "$P/claimed/$j/carrier" ] && [ ! -f {jobs}/"$j/exit_code" ]; then R=$((R + 1)); fi
done
L=0
for c in "$P"/carriers/c*; do
  [ -f "$c/sched_id" ] && [ ! -f "$c/closing" ] || continue
  if [ -n "$(colm_alive "$(cat "$c/sched_id")")" ]; then L=$((L + 1)); fi
done
if [ "$W" -gt $((L * SLOTS - R)) ]; then
  N=0
  while [ -e "$P/carriers/c$N" ]; do N=$((N + 1)); done
  C="$P/carriers/c$N"
  mkdir "$C"
  cat > "$C/script.sh" <<'COLM_CARRIER_EOF'
{script}COLM_CARRIER_EOF
  chmod +x "$C/script.sh"
  if ! ( cd "$C" && colm_carrier && echo "$ID" > sched_id ); then
    # 没有载体来跑它：撤出队列，这次提交按失败报告。
    rm -f "$P/queue/$J" sched_id
    exit 1
  fi
fi
exec 8>&-
echo pack {batch}
"#,
        submit = if slurm { sched::SLURM_SUBMIT } else { "" },
        alive = ALIVE,
        carrier_submit = carrier_submit(scheduler),
        kind = scheduler.name(),
        pool = quote(&pool),
        job = quote(job),
        batch = quote(&pack.pool),
        slots = pack.slots,
        jobs = quote(&jobs),
    ))
}
