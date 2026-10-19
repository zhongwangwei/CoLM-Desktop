//! "核对服务器状态"时一并看服务器忙不忙：登录节点的负载与内存，调度系统各分区（队列）的节点与核占用，
//! 全体用户的排队与运行作业数，以及自己的作业在跑还是在排、排的原因。只读，几条查询各限时 8 s。
//!
//! 实测（2026-10-20）：land02 是 Slurm 19.05，`sinfo -o '%R|%F|%C'` 给出各分区节点与 CPU 的
//! 已分配/空闲/其他/总数；天河 Slurm 23.11 同样可用；摆渡船 LSF 10.1 在非登录 shell 里没有 `bqueues`，
//! 要先加载 `/etc/profile`（`sched::PATH_PRELUDE`），`bhosts -w` 的 MAX/NJOBS 两列给出作业槽占用。

use anyhow::Result;
use serde::Serialize;

use crate::ssh::Ssh;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Load {
    pub hostname: String,
    /// 登录节点（ssh 登上去的那台）的逻辑 CPU 数、1/5/15 分钟负载、内存总量与可用量。
    pub cpus: Option<u32>,
    pub load: Option<[f64; 3]>,
    pub memory_total_gb: Option<f64>,
    pub memory_available_gb: Option<f64>,
    /// slurm、lsf、pbs；都没有时为空（作业直接跑在这台机器上，看登录节点的负载就够了）。
    pub scheduler: Option<String>,
    /// Slurm 各分区：节点与 CPU 的已分配、空闲、其他（下线、维护）、总数。
    pub partitions: Vec<PartitionLoad>,
    /// LSF 全部主机：可用、已满、关闭的主机数，作业槽已用与总数。
    pub hosts: Option<HostsLoad>,
    /// 各分区（队列）全体用户的排队与运行作业数。
    pub queues: Vec<QueueLoad>,
    /// 自己的作业，按分区、状态、等待原因归并。
    pub mine: Vec<MyJobs>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PartitionLoad {
    pub name: String,
    /// 已分配、空闲、其他、总数。
    pub nodes: [u32; 4],
    pub cpus: [u32; 4],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HostsLoad {
    pub ok: u32,
    pub full: u32,
    pub closed: u32,
    pub total: u32,
    pub slots_used: u32,
    pub slots_total: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct QueueLoad {
    pub name: String,
    pub pending: u32,
    pub running: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MyJobs {
    pub partition: String,
    /// RUNNING、PENDING 等，LSF 与 PBS 的状态词换成 Slurm 的说法。
    pub state: String,
    /// Slurm 的等待原因（`squeue %r`）；其他调度系统为空。
    pub reason: String,
    pub count: u32,
}

/// 每行 `key=value`；`uniq -c` 的计数放在行尾。
pub fn script() -> String {
    format!(
        r#"set +e
{prelude}colm_t() {{ if command -v timeout >/dev/null 2>&1; then timeout -k 1 8 "$@"; else "$@"; fi; }}
echo "hostname=$(hostname)"
echo "cpus=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN)"
echo "loadavg=$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null)"
echo "mem_kb=$(awk '/^MemTotal:/{{t=$2}} /^MemAvailable:/{{a=$2}} END{{print t " " a}}' /proc/meminfo 2>/dev/null)"
U=${{USER:-$(id -un)}}
if command -v sinfo >/dev/null 2>&1 && command -v squeue >/dev/null 2>&1; then
  echo "scheduler=slurm"
  colm_t sinfo -h -o '%R|%F|%C' 2>/dev/null | sed 's/^/part=/'
  colm_t squeue -h -o '%P|%T' 2>/dev/null | sort | uniq -c | sed -E 's/^ *([0-9]+) (.*)$/queue=\2|\1/'
  colm_t squeue -h -u "$U" -o '%P|%T|%r' 2>/dev/null | sort | uniq -c | sed -E 's/^ *([0-9]+) (.*)$/mine=\2|\1/'
elif command -v bjobs >/dev/null 2>&1; then
  echo "scheduler=lsf"
  colm_t bqueues -w 2>/dev/null | awk 'NR>1 && NF>=10 {{print "queue=" $1 "|PENDING|" $9; print "queue=" $1 "|RUNNING|" $10}}'
  colm_t bhosts -w 2>/dev/null | awk 'NR>1 {{n++; if ($2=="ok") ok++; else if ($2=="closed_Full") full++; else closed++; if ($4 ~ /^[0-9]+$/) {{max+=$4; used+=$5}}}} END {{if (n) print "hosts=" ok+0 "|" full+0 "|" closed+0 "|" n "|" used+0 "|" max+0}}'
  colm_t bjobs -u "$U" -noheader -o 'queue stat' 2>/dev/null | awk 'NF==2 {{print $1 "|" $2 "|"}}' | sort | uniq -c | sed -E 's/^ *([0-9]+) (.*)$/mine=\2|\1/'
elif command -v qstat >/dev/null 2>&1; then
  echo "scheduler=pbs"
  colm_t qstat -u "$U" 2>/dev/null | awk '$1 ~ /^[0-9]/ && NF>=10 {{print $3 "|" $10 "|"}}' | sort | uniq -c | sed -E 's/^ *([0-9]+) (.*)$/mine=\2|\1/'
fi
"#,
        prelude = crate::sched::PATH_PRELUDE,
    )
}

/// LSF、PBS 的状态词换成 Slurm 的说法，界面只认一套；不认识的原样保留。
fn slurm_state(state: &str) -> String {
    match state {
        "RUN" | "R" => "RUNNING".into(),
        "PEND" | "Q" => "PENDING".into(),
        "H" => "HELD".into(),
        "PSUSP" | "USUSP" | "SSUSP" | "S" => "SUSPENDED".into(),
        other => other.into(),
    }
}

/// `A/I/O/T` -> 四个数；格式不对就不要这一行。
fn four(text: &str) -> Option<[u32; 4]> {
    let parts = text
        .split('/')
        .map(|n| n.trim().parse().ok())
        .collect::<Option<Vec<u32>>>()?;
    parts.try_into().ok()
}

pub fn parse(text: &str) -> Load {
    let mut load = Load::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let fields: Vec<&str> = value.split('|').collect();
        match key {
            "hostname" => load.hostname = value.into(),
            "cpus" => load.cpus = value.parse().ok(),
            "loadavg" => {
                let numbers = value
                    .split_whitespace()
                    .map(|n| n.parse().ok())
                    .collect::<Option<Vec<f64>>>();
                load.load = numbers.and_then(|n| n.try_into().ok());
            }
            "mem_kb" => {
                let mut numbers = value.split_whitespace().map(|n| n.parse::<f64>().ok());
                let gb = |kb: f64| (kb / 1024.0 / 1024.0 * 10.0).round() / 10.0;
                load.memory_total_gb = numbers.next().flatten().map(gb);
                load.memory_available_gb = numbers.next().flatten().map(gb);
            }
            "scheduler" => load.scheduler = Some(value.into()).filter(|s: &String| !s.is_empty()),
            "part" => {
                if let [name, nodes, cpus] = fields[..] {
                    if let (Some(nodes), Some(cpus)) = (four(nodes), four(cpus)) {
                        load.partitions.push(PartitionLoad {
                            name: name.into(),
                            nodes,
                            cpus,
                        });
                    }
                }
            }
            "queue" => {
                if let [name, state, count] = fields[..] {
                    let Ok(count) = count.parse::<u32>() else {
                        continue;
                    };
                    let index = match load.queues.iter().position(|q| q.name == name) {
                        Some(index) => index,
                        None => {
                            load.queues.push(QueueLoad {
                                name: name.into(),
                                ..QueueLoad::default()
                            });
                            load.queues.len() - 1
                        }
                    };
                    match slurm_state(state).as_str() {
                        "PENDING" => load.queues[index].pending += count,
                        "RUNNING" => load.queues[index].running += count,
                        // 正在结束、已完成等不算排队也不算在跑。
                        _ => {}
                    }
                }
            }
            "hosts" => {
                let numbers = fields
                    .iter()
                    .map(|n| n.parse().ok())
                    .collect::<Option<Vec<u32>>>();
                if let Some([ok, full, closed, total, slots_used, slots_total]) = numbers.as_deref()
                {
                    load.hosts = Some(HostsLoad {
                        ok: *ok,
                        full: *full,
                        closed: *closed,
                        total: *total,
                        slots_used: *slots_used,
                        slots_total: *slots_total,
                    });
                }
            }
            "mine" => {
                if let [partition, state, reason, count] = fields[..] {
                    if let Ok(count) = count.parse() {
                        load.mine.push(MyJobs {
                            partition: partition.into(),
                            state: slurm_state(state),
                            reason: reason.into(),
                            count,
                        });
                    }
                }
            }
            // 别的行来自登录脚本（`/etc/profile` 的欢迎词等），不是查询结果；每个 key 都有测试守着拼写。
            _ => {}
        }
    }
    load
}

pub fn load(ssh: &Ssh) -> Result<Load> {
    Ok(parse(&ssh.run_ok(&script())?))
}

#[cfg(test)]
#[path = "load_tests.rs"]
mod load_tests;
