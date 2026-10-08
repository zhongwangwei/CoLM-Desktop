//! 编译与运行的沙箱：断网，只能写工作区与临时目录。
//!
//! - macOS：`sandbox-exec` 的 Seatbelt 配置。
//! - Linux：bubblewrap（`bwrap`），装了才用。
//! - 其他（Windows，或没装 bwrap）：不套沙箱，**明确告诉调用方**（报告里写 `kind = "none"`）。
//!
//! **这只能防误写与误联网，不是运行恶意代码的完整安全边界**：编译和运行本质上是执行任意代码。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SandboxInfo {
    /// seatbelt、bubblewrap 或 none。
    pub kind: String,
    pub network_blocked: bool,
    pub note: String,
}

/// 沙箱策略：除了工作区，还有哪些目录可写；要不要联网。
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// 可写的目录（工作区、临时目录之外的）；通常是工作区自己。
    pub writable: Vec<PathBuf>,
    pub allow_network: bool,
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// cargo 即使 `--offline` 也要往 `CARGO_HOME` 里写锁文件，所以那里可写。
fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".cargo")))
        .filter(|p| p.is_dir())
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|p| p.is_file())
    })
}

pub fn detect() -> SandboxInfo {
    if cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file() {
        return SandboxInfo {
            kind: "seatbelt".into(),
            network_blocked: true,
            note:
                "macOS sandbox-exec: no network, writes only to the workspace and temporary folders"
                    .into(),
        };
    }
    if cfg!(target_os = "linux") && find_in_path("bwrap").is_some() {
        return SandboxInfo {
            kind: "bubblewrap".into(),
            network_blocked: true,
            note: "bubblewrap: read-only system, no network, writes only to the workspace and /tmp"
                .into(),
        };
    }
    SandboxInfo {
        kind: "none".into(),
        network_blocked: false,
        note: if cfg!(windows) {
            "Windows has no sandbox here: commands run with your normal permissions".into()
        } else {
            "no sandbox tool found (install bubblewrap): commands run with your normal permissions"
                .into()
        },
    }
}

/// Seatbelt 配置里的字符串字面量。
fn sb_string(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn seatbelt_profile(policy: &Policy) -> String {
    let mut writable: Vec<PathBuf> = policy.writable.iter().map(|p| canonical(p)).collect();
    writable.extend(
        ["/private/tmp", "/private/var/folders", "/dev"]
            .iter()
            .map(PathBuf::from),
    );
    if let Some(dir) = std::env::var_os("TMPDIR") {
        writable.push(canonical(Path::new(&dir)));
    }
    if let Some(dir) = cargo_home() {
        writable.push(canonical(&dir));
    }
    let allowed: Vec<String> = writable
        .iter()
        .map(|p| format!("(subpath {})", sb_string(p)))
        .collect();
    let mut profile = String::from("(version 1)\n(allow default)\n");
    if !policy.allow_network {
        profile.push_str("(deny network*)\n");
    }
    profile.push_str("(deny file-write*)\n");
    profile.push_str(&format!("(allow file-write* {})\n", allowed.join(" ")));
    profile
}

/// 把 `program args…` 包进沙箱，返回可以直接 `spawn` 的命令与沙箱说明。
pub fn wrap(program: &Path, args: &[String], policy: &Policy) -> (Command, SandboxInfo) {
    let info = detect();
    let mut command = match info.kind.as_str() {
        "seatbelt" => {
            let mut c = Command::new("/usr/bin/sandbox-exec");
            c.arg("-p").arg(seatbelt_profile(policy)).arg(program);
            c
        }
        "bubblewrap" => {
            let mut c = Command::new("bwrap");
            c.args([
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--die-with-parent",
            ]);
            if !policy.allow_network {
                c.arg("--unshare-net");
            }
            c.args(["--bind", "/tmp", "/tmp"]);
            let mut binds: Vec<PathBuf> = policy.writable.iter().map(|p| canonical(p)).collect();
            if let Some(dir) = cargo_home() {
                binds.push(dir);
            }
            for dir in binds {
                c.arg("--bind").arg(&dir).arg(&dir);
            }
            c.arg("--").arg(program);
            c
        }
        _ => Command::new(program),
    };
    command.args(args);
    (command, info)
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod sandbox_tests;
