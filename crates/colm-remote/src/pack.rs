//! 按节点打包站点（Slurm）。
//!
//! 天河的 deimos 分区是 `OverSubscribe=EXCLUSIVE`：不管要几个核，一个作业就占一整个节点、按整节点计费。
//! 每个站点一个作业时，90 个站点占 90 个节点；sacct 实测 AU-How 跑 5 分 27 秒，记 5 小时 48 分核时
//! （2026-10-19）。所以在独占分区上，站点不再各交一个作业，而是放进本批的队列
//! `<根>/pools/<批次>/queue/`，由"载体"作业去领：一个载体占一个节点，同时跑到这个节点的核数，
//! 跑完一个领下一个，队列空了再等一会儿就退出。90 个站点、64 核的节点，就是两个载体。
//!
//! 每个站点仍有自己的作业目录（`script.sh`、`log`、`phase`、`exit_code` 都不变），所以查状态、看日志、
//! 取回结果和单站点时一样；作业目录里 `scheduler` 是 `slurm-pack`，`pool` 是队列目录，`sched_id` 是批次。
//! 领取用 `mkdir claimed/<站点>`（原子操作），所以一个站点只会被一个载体跑一次；取消一个还没领的站点
//! 也是抢先建这个目录，载体就不会再跑它。
//!
//! 共享节点的分区不打包：在那里每个站点一个作业本来就只占它要的核。
use anyhow::{ensure, Result};

use crate::sched::{self, Resources, Scheduler};
use crate::ssh::quote;

/// 一批站点共用的队列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// 批次号（应用里一次批量运行的编号），也是队列目录名。
    pub pool: String,
    /// 每个载体同时跑几个站点；0 = 用节点的核数（登录节点上按 `sinfo` 估，载体里按 `SLURM_CPUS_ON_NODE`）。
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
        r#"cd "${{SLURM_SUBMIT_DIR:-.}}"
P={pool}; JOBS={jobs}
SLOTS={slots}
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=${{SLURM_CPUS_ON_NODE:-$(nproc 2>/dev/null || echo 1)}}
LINGER=${{COLM_PACK_LINGER:-60}}
ME=${{SLURM_JOB_ID:-$(cat sched_id 2>/dev/null)}}
mkdir -p running done pg term
idle=0
while :; do
  n=0
  for r in running/*; do
    [ -e "$r" ] || continue
    j=${{r##*/}}
    if [ -f "done/$j" ]; then
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

/// 替代 `sched::launch` 的一段：`script.sh` 已写好，当前目录是站点的作业目录。分区独占时入队，
/// 必要时再交一个载体，最后一行输出 `pack <批次>`；分区不独占时照常单独提交，最后一行是作业号。
pub fn launch(root: &str, job: &str, pack: &Pack, resources: &Resources) -> Result<String> {
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
    let header = sched::directives(
        Scheduler::Slurm,
        &sched::job_name("pack"),
        // 不放进 carriers/：那里按 `c*` 数载体（天河实测 `carrier-<号>.log` 也被数了进去，好在没有 sched_id 被跳过）。
        &format!("{pool}/carrier-%j.log"),
        &carrier,
    )?;
    let script = carrier_script(&pool, &jobs, pack.slots, &header);
    ensure!(
        !script.contains("COLM_CARRIER_EOF"),
        "the carrier script may not contain its terminator"
    );
    let forced = resources
        .directives
        .iter()
        .any(|d| d == "--exclusive" || d.starts_with("--exclusive="));
    let partition = resources.partition.as_deref().map(quote);
    // 分区要不要整节点：`scontrol -o` 每个分区一行，没指定分区时看默认分区；老版本的 Slurm 叫 Shared。
    let exclusive = if forced {
        "true".to_owned()
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
    let estimate = match &partition {
        Some(part) => format!("sinfo -h -p {part} -o %c"),
        None => "sinfo -h -o %c".to_owned(),
    };
    Ok(format!(
        r#"{submit}if {exclusive}; then
P={pool}; J={job}
mkdir -p "$P/queue" "$P/claimed" "$P/carriers"
echo slurm-pack > scheduler
echo "$P" > pool
echo {batch} > sched_id
exec 8>"$P/.lock"
flock 8
SLOTS={slots}
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=$(cat "$P/slots" 2>/dev/null || true)
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=$({estimate} 2>/dev/null | tr -d '+ ' | sort -n | head -n 1)
[ "$SLOTS" -gt 0 ] 2>/dev/null || SLOTS=1
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
  if [ -n "$(squeue -h -j "$(cat "$c/sched_id")" -o %i 2>/dev/null)" ]; then L=$((L + 1)); fi
done
if [ "$W" -gt $((L * SLOTS - R)) ]; then
  N=0
  while [ -e "$P/carriers/c$N" ]; do N=$((N + 1)); done
  C="$P/carriers/c$N"
  mkdir "$C"
  cat > "$C/script.sh" <<'COLM_CARRIER_EOF'
{script}COLM_CARRIER_EOF
  chmod +x "$C/script.sh"
  if ! ( cd "$C" && colm_sbatch script.sh && echo "$ID" > sched_id ); then
    # 没有载体来跑它：撤出队列，这次提交按失败报告。
    rm -f "$P/queue/$J" sched_id
    exit 1
  fi
fi
exec 8>&-
echo pack {batch}
else
{single}fi
"#,
        submit = sched::SLURM_SUBMIT,
        pool = quote(&pool),
        job = quote(job),
        batch = quote(&pack.pool),
        slots = pack.slots,
        jobs = quote(&jobs),
        single = sched::launch(Scheduler::Slurm),
    ))
}
