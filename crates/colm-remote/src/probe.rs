//! “测试连接”：探测远程主机的系统、资源、工具链、调度系统与工作根目录的空间。

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::ssh::{quote, Ssh};

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
    /// 找到的调度系统：slurm、pbs、lsf；空表示直接后台运行。
    pub schedulers: Vec<String>,
    /// 工作根目录：存在吗、可写吗、所在盘的剩余空间。
    pub root: String,
    pub root_exists: bool,
    pub root_writable: bool,
    pub root_free_gb: Option<u64>,
    /// 第一版能不能在这台机器上跑（缺什么）。
    pub problems: Vec<String>,
}

/// 探测脚本：每行 `key=value`，缺的工具输出空值。
pub fn script(root: &str) -> String {
    format!(
        r#"set +e
R={root}
echo "hostname=$(hostname)"
echo "os=$(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME" || uname -s)"
echo "arch=$(uname -m)"
echo "cpus=$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN)"
echo "memory_kb=$(awk '/MemTotal/{{print $2}}' /proc/meminfo 2>/dev/null)"
C=$(command -v cargo || ls "$HOME/.cargo/bin/cargo" 2>/dev/null)
echo "cargo=$([ -n "$C" ] && "$C" --version 2>/dev/null)"
echo "gfortran=$(gfortran --version 2>/dev/null | head -1)"
echo "mpi=$( (command -v mpif90 || command -v mpiifort || command -v mpiifx) 2>/dev/null | head -1)"
echo "cmake=$(command -v cmake)"
echo "cc=$( (command -v cc || command -v gcc) 2>/dev/null | head -1)"
echo "slurm=$(command -v sbatch)"
echo "pbs=$(command -v qsub)"
echo "lsf=$(command -v bsub)"
echo "root_exists=$([ -d "$R" ] && echo 1)"
P="$R"; while [ ! -d "$P" ] && [ "$P" != "/" ]; do P=$(dirname "$P"); done
echo "root_writable=$([ -w "$P" ] && echo 1)"
echo "root_free_kb=$(df -Pk "$P" 2>/dev/null | awk 'NR==2{{print $4}}')"
"#,
        root = quote(root)
    )
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
        root: root.to_owned(),
        root_exists: get("root_exists").is_some(),
        root_writable: get("root_writable").is_some(),
        root_free_gb: get("root_free_kb")
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kb| kb / 1024 / 1024),
        problems: Vec::new(),
    };
    if probe.os.is_empty()
        || !probe.os.to_ascii_lowercase().contains("linux") && probe.arch.is_empty()
    {
        probe
            .problems
            .push("the host did not answer like a Linux system".into());
    }
    if probe.cargo.is_none() {
        probe.problems.push("cargo (Rust) is not installed; the first version builds the Rust engine on the server — install it with rustup".into());
    }
    if !probe.cmake || !probe.cc {
        probe
            .problems
            .push("cmake and a C compiler are needed to build netCDF/HDF5 into the engine".into());
    }
    if !probe.root_writable {
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

#[cfg(test)]
#[path = "probe_tests.rs"]
mod probe_tests;
