//! “测试连接”：探测远程主机的系统、资源、工具链、调度系统与工作根目录的空间。

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::ssh::{quote, Ssh};

// Non-interactive SSH does not reliably load the user's toolchain PATH.
pub(crate) const BUILD_PATH_PRELUDE: &str =
    "export PATH=\"$HOME/.local/share/colm/toolchain/cmake/bin:$HOME/.local/share/colm/toolchain/rust/bin:$PATH:$HOME/.local/bin:${CARGO_HOME:-$HOME/.cargo}/bin\"\n";

/// 探测结果。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Probe {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cpus: Option<u32>,
    pub memory_gb: Option<u64>,
    /// `cargo --version`；空表示没有（第一版的远程引擎要在服务器上编译）。
    pub cargo: Option<String>,
    pub gfortran: Option<String>,
    pub mpi: Option<String>,
    pub cmake: bool,
    pub cc: bool,
    /// 找到的调度系统：slurm、pbs、lsf；空且 qsub_unverified 为 false 时可直接后台运行。
    pub schedulers: Vec<String>,
    /// qsub is present but read-only checks could not establish PBS identity.
    pub qsub_unverified: bool,
    /// 调度系统的分区或队列（`sinfo`、`qstat -Q`、`bqueues`），给“选分区”用；拿不到就空。
    pub queues: Vec<String>,
    /// Slurm 分区的详情，给"选分区"显示节点规模，并在选定后自动填"每节点核数"；拿不到就空。
    pub partitions: Vec<Partition>,
    /// 工作根目录：存在吗、可写吗、所在盘的剩余空间。
    pub root: String,
    pub root_exists: bool,
    pub root_writable: bool,
    pub root_free_gb: Option<u64>,
    /// 这台机器跑不了的原因（连不上的系统、工作目录不可写）。
    pub problems: Vec<String>,
    /// 在这台机器上从源码编引擎缺什么（cargo、cmake、C 编译器）。空表示能编；非空时要有预编包才能用。
    pub build_problems: Vec<String>,
    /// Missing Rust/CMake can be installed for this user when a source build starts.
    pub toolchain_installable: bool,
    /// 这次要用的引擎：source（在服务器上编）、prebuilt（传预编包）；调用方判断后填，探测本身不填。
    pub engine: Option<String>,
}

/// 一个 Slurm 分区。land02（2026-10-19）有 first、second、largemem 三个分区，节点 48–80 核、
/// 320 GB–1.5 TB 不等，单作业能占的节点数也不同；只给分区名时，用户得自己去查这些。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Partition {
    pub name: String,
    pub default: bool,
    /// 作业总是占整个节点（`OverSubscribe=EXCLUSIVE`）：站点批量会按节点打包（`crate::pack`）。
    pub exclusive: bool,
    pub nodes: u32,
    /// 物理核数（插槽 × 每插槽核数，不算超线程），分区里最小、最大的节点。
    pub cores_min: u32,
    pub cores_max: u32,
    pub memory_gb_min: u64,
    pub memory_gb_max: u64,
    /// 单个作业最多几个节点；`UNLIMITED` 时为空。
    pub max_nodes: Option<u32>,
    pub max_time: Option<String>,
    /// 不写内存的作业会拿走整个节点的内存（`DefMemPerNode=UNLIMITED`）；提交时会补上按核分摊的内存。
    pub whole_node_memory: bool,
}

/// `partition=名|默认|独占|单作业节点数|时限|整节点内存` 每个分区一行，`partnode=分区|插槽|每插槽核|内存MB`
/// 每个节点一行（一个节点在几个分区就出现几次）。有 timeout 才问，免得卡死的 slurmctld 拖住整个探测。
const PARTITIONS: &str = r#"if command -v scontrol >/dev/null 2>&1 && command -v timeout >/dev/null 2>&1; then
  timeout -k 1 5 scontrol -o show partition 2>/dev/null | awk '{ n=""; d=0; x=0; mn=""; mt=""; u=0
    for (i = 1; i <= NF; i++) { k = $i; sub(/=.*/, "", k); v = substr($i, length(k) + 2)
      if (k == "PartitionName") n = v
      else if (k == "Default" && v == "YES") d = 1
      else if ((k == "OverSubscribe" || k == "Shared") && v ~ /^EXCLUSIVE/) x = 1
      else if (k == "MaxNodes") mn = v
      else if (k == "MaxTime") mt = v
      else if (k == "DefMemPerNode" && v == "UNLIMITED") u = 1 }
    if (n != "") print "partition=" n "|" d "|" x "|" mn "|" mt "|" u }'
  timeout -k 1 5 sinfo -h -N -o '%R|%X|%Y|%m' 2>/dev/null | sed 's/^/partnode=/'
fi
"#;

/// 探测脚本：每行 `key=value`，缺的工具输出空值。
pub fn script(root: &str) -> String {
    format!(
        r#"set +e
{prelude}{build_path}{toolchain_requirements}R={root}
echo "toolchain_installable=$(colm_toolchain_installable 2>/dev/null && echo 1)"
echo "hostname=$(hostname)"
echo "os=$(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME" || uname -s)"
echo "arch=$(uname -m)"
echo "cpus=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN)"
echo "memory_kb=$(awk '/MemTotal/{{print $2}}' /proc/meminfo 2>/dev/null)"
C=$(command -v cargo || ls "$HOME/.cargo/bin/cargo" 2>/dev/null)
echo "cargo=$([ -n "$C" ] && rustc --version >/dev/null 2>&1 && "$C" --version 2>/dev/null)"
echo "gfortran=$(gfortran --version 2>/dev/null | head -1)"
echo "mpi=$( (command -v mpif90 || command -v mpiifort || command -v mpiifx) 2>/dev/null | head -1)"
echo "cmake=$(colm_cmake_ok && command -v cmake)"
echo "cc=$( (command -v cc || command -v gcc) 2>/dev/null | head -1)"
echo "slurm=$(command -v sbatch)"
PBS=""; QSUB=$(command -v qsub); QSUB_UNVERIFIED=""
if [ -n "$QSUB" ]; then
  QSUB_UNVERIFIED=1
  # Slurm and LSF also install qsub wrappers; command names alone do not identify PBS.
  if command -v qstat >/dev/null 2>&1 && command -v timeout >/dev/null 2>&1; then
    if timeout -k 1 3 qstat --version 2>&1 | grep -Eiq \
      -e '^[[:space:]]*pbs_version[[:space:]]*=[[:space:]]*[^[:space:]]' \
      -e '^[[:space:]]*(OpenPBS|PBS Professional|TORQUE)[[:space:]]+([[:alpha:]]+[[:space:]:]+)?[0-9]' \
      -e '^[[:space:]]*Version:[[:space:]]*[0-9][[:alnum:]._-]*[[:space:]]+TORQUE([[:space:]]|$)'; then
      PBS="$QSUB"
    elif timeout -k 1 3 qstat -Bf 2>/dev/null | grep -Eq '^[[:space:]]*pbs_version[[:space:]]*=[[:space:]]*[^[:space:]]'; then
      PBS="$QSUB"
    fi
  fi
  [ -n "$PBS" ] && QSUB_UNVERIFIED=""
fi
echo "pbs=$PBS"
echo "qsub_unverified=$QSUB_UNVERIFIED"
echo "lsf=$(command -v bsub)"
{partitions}echo "queues=$( (
  if command -v timeout >/dev/null 2>&1; then
    timeout -k 1 3 sinfo -h -o '%P' 2>/dev/null
    [ -n "$PBS" ] && timeout -k 1 3 qstat -Q 2>/dev/null | awk 'NR>2{{print $1}}'
    timeout -k 1 3 bqueues -w 2>/dev/null | awk 'NR>1{{print $1}}'
  fi
) | tr -d '*' | tr '\n' ' ')"
if [ -n "$R" ]; then
echo "root_exists=$([ -d "$R" ] && echo 1)"
P="$R"; while [ ! -d "$P" ] && [ "$P" != "/" ]; do P=$(dirname "$P"); done
echo "root_writable=$([ -w "$P" ] && echo 1)"
echo "root_free_kb=$(df -Pk "$P" 2>/dev/null | awk 'NR==2{{print $4}}')"
fi
"#,
        root = quote(root),
        prelude = crate::sched::PATH_PRELUDE,
        build_path = BUILD_PATH_PRELUDE,
        toolchain_requirements = crate::toolchain::requirements_script(),
        partitions = PARTITIONS,
    )
}

fn partitions(text: &str) -> Vec<Partition> {
    let mut found: Vec<Partition> = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("partition=") else {
            continue;
        };
        let fields: Vec<&str> = rest.split('|').collect();
        let [name, default, exclusive, max_nodes, max_time, whole] = fields[..] else {
            continue;
        };
        if name.is_empty() || found.iter().any(|p| p.name == name) {
            continue;
        }
        found.push(Partition {
            name: name.to_owned(),
            default: default == "1",
            exclusive: exclusive == "1",
            max_nodes: max_nodes.parse().ok(),
            max_time: Some(max_time.to_owned()).filter(|t| !t.is_empty() && t != "UNLIMITED"),
            whole_node_memory: whole == "1",
            ..Partition::default()
        });
    }
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("partnode=") else {
            continue;
        };
        let fields: Vec<&str> = rest.split('|').collect();
        let [name, sockets, cores, memory] = fields[..] else {
            continue;
        };
        let (Ok(sockets), Ok(cores), Ok(memory)) = (
            sockets.parse::<u32>(),
            cores.parse::<u32>(),
            memory.trim_end_matches('+').parse::<u64>(),
        ) else {
            continue;
        };
        let Some(partition) = found.iter_mut().find(|p| p.name == name) else {
            continue;
        };
        let (cores, memory) = (sockets * cores, memory / 1024);
        if partition.nodes == 0 {
            (partition.cores_min, partition.cores_max) = (cores, cores);
            (partition.memory_gb_min, partition.memory_gb_max) = (memory, memory);
        } else {
            partition.cores_min = partition.cores_min.min(cores);
            partition.cores_max = partition.cores_max.max(cores);
            partition.memory_gb_min = partition.memory_gb_min.min(memory);
            partition.memory_gb_max = partition.memory_gb_max.max(memory);
        }
        partition.nodes += 1;
    }
    found
}

/// 解析探测脚本的输出。
pub fn parse(root: &str, text: &str) -> Probe {
    let values: BTreeMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect();
    let get = |k: &str| values.get(k).copied().filter(|v| !v.is_empty());
    let mut probe = Probe {
        hostname: get("hostname").unwrap_or_default().to_owned(),
        os: get("os").unwrap_or_default().to_owned(),
        arch: get("arch").unwrap_or_default().to_owned(),
        cpus: get("cpus").and_then(|v| v.parse().ok()),
        memory_gb: get("memory_kb")
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kb| kb / 1024 / 1024),
        cargo: get("cargo").map(str::to_owned),
        gfortran: get("gfortran").map(str::to_owned),
        mpi: get("mpi").map(str::to_owned),
        cmake: get("cmake").is_some(),
        cc: get("cc").is_some(),
        schedulers: ["slurm", "pbs", "lsf"]
            .into_iter()
            .filter(|s| get(s).is_some())
            .map(str::to_owned)
            .collect(),
        qsub_unverified: get("qsub_unverified") == Some("1"),
        queues: get("queues")
            .map(|v| v.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        partitions: partitions(text),
        root: root.to_owned(),
        root_exists: get("root_exists").is_some(),
        root_writable: get("root_writable").is_some(),
        root_free_gb: get("root_free_kb")
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kb| kb / 1024 / 1024),
        problems: Vec::new(),
        build_problems: Vec::new(),
        toolchain_installable: get("toolchain_installable") == Some("1"),
        engine: None,
    };
    if probe.os.is_empty()
        || !probe.os.to_ascii_lowercase().contains("linux") && probe.arch.is_empty()
    {
        probe
            .problems
            .push("the host did not answer like a Linux system".into());
    }
    if probe.cargo.is_none() {
        probe
            .build_problems
            .push("cargo (Rust) is not installed; source builds need Rust/Cargo".into());
    }
    if !probe.cmake || !probe.cc {
        probe.build_problems.push(
            "CMake 3.26 or newer and a C compiler are needed to build netCDF/HDF5 into the engine"
                .into(),
        );
    }
    if !root.is_empty() && !probe.root_writable {
        probe.problems.push(format!(
            "the work directory {root} cannot be created or written"
        ));
    }
    probe
}

/// 探测一台主机。
pub fn probe(ssh: &Ssh, root: &str) -> Result<Probe> {
    let text = ssh.run_ok(&script(root))?;
    Ok(parse(root, &text))
}

/// Probe mappings in the same SSH request as the connection check.
fn paths_script(root: &str, paths: &[String]) -> String {
    let mut command = script(root);
    for (index, path) in paths.iter().enumerate() {
        command.push_str(&format!(
            "if test -e {}; then echo map_{index}=1; else echo map_{index}=0; fi\n",
            quote(path)
        ));
    }
    command
}

pub fn probe_paths(ssh: &Ssh, root: &str, paths: &[String]) -> Result<(Probe, Vec<bool>)> {
    let text = ssh.run_ok(&paths_script(root, paths))?;
    let present = paths
        .iter()
        .enumerate()
        .map(|(index, _)| text.lines().any(|line| line == format!("map_{index}=1")))
        .collect();
    Ok((parse(root, &text), present))
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod probe_tests;
