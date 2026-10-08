//! 测试白名单（docs/design-ai-assistant.md 第 4 节 `run_tests`）：指定 crate 的 `cargo test`、oracle 分层检查、
//! `check-gui`。没有别的：参数是枚举加校验过的包名，不是任意命令。

use std::path::PathBuf;

use anyhow::{bail, ensure, Result};

use crate::build::{run_logged, Outcome};
use crate::gates::GateRun;
use crate::layout::{now, Workspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `cargo test -p <包> --lib --bins`（串行：colm-init 并行跑会撞 HDF5）。
    Cargo(String),
    /// oracle 的 history 闸门测试与分层检查。
    Oracle,
    /// GUI 命令与前端的一致性检查。
    CheckGui,
}

impl Kind {
    /// 门槛里记的名字。
    pub fn label(&self) -> String {
        match self {
            Self::Cargo(package) => format!("cargo:{package}"),
            Self::Oracle => "oracle".into(),
            Self::CheckGui => "check-gui".into(),
        }
    }

    pub fn parse(kind: &str, package: Option<&str>) -> Result<Self> {
        Ok(match kind {
            "cargo" => Self::Cargo(
                package
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("a cargo test needs a package name"))?
                    .to_owned(),
            ),
            "oracle" => Self::Oracle,
            "check-gui" => Self::CheckGui,
            other => bail!("unknown test kind {other:?}; use cargo, oracle or check-gui"),
        })
    }
}

/// 包名必须是工作区里真有的 crate，且只含字母数字与 `-_`。
fn check_package(workspace: &Workspace, package: &str) -> Result<()> {
    ensure!(
        !package.is_empty()
            && package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "bad package name {package:?}"
    );
    let known = workspace
        .src()
        .join("crates")
        .join(package)
        .join("Cargo.toml")
        .is_file()
        || ["oracle", "xtask"].contains(&package);
    ensure!(known, "{package} is not a crate of this workspace");
    Ok(())
}

fn cargo(args: &[&str]) -> Vec<String> {
    let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    all.extend(["--offline".into(), "--locked".into()]);
    all
}

/// 要跑的命令（每个 Kind 一到几条）。
pub fn commands(workspace: &Workspace, kind: &Kind) -> Result<Vec<Vec<String>>> {
    Ok(match kind {
        Kind::Cargo(package) => {
            check_package(workspace, package)?;
            let mut args = cargo(&["test", "-p", package, "--lib", "--bins"]);
            args.extend(["--".into(), "--test-threads=1".into()]);
            vec![args]
        }
        Kind::Oracle => {
            let mut tier = cargo(&["run", "-q", "-p", "oracle", "--bin", "tier-check"]);
            tier.push("--".into());
            let golden = workspace.src().join("oracle/golden");
            let mut files: Vec<PathBuf> = std::fs::read_dir(&golden)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "nc"))
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            ensure!(!files.is_empty(), "no golden files in {}", golden.display());
            tier.extend(files.iter().map(|p| p.display().to_string()));
            vec![cargo(&["test", "-p", "oracle", "--test", "histmap"]), tier]
        }
        Kind::CheckGui => vec![cargo(&["run", "-q", "-p", "xtask"])
            .into_iter()
            .chain(["--".into(), "check-gui".into()])
            .collect::<Vec<_>>()],
    })
}

/// 跑一类测试，把结果记成门槛。
pub fn run(
    workspace: &mut Workspace,
    kind: &Kind,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Outcome> {
    let list = commands(workspace, kind)?;
    let src = workspace.src();
    let target = workspace.target();
    let mut last: Option<Outcome> = None;
    for args in list {
        let outcome = run_logged(
            workspace,
            &format!("test-{}", kind.label().replace(':', "-")),
            std::path::Path::new("cargo"),
            &args,
            &src,
            &[("CARGO_TARGET_DIR", target.display().to_string())],
            false,
            cancel,
        )?;
        let failed = !outcome.ok;
        last = Some(outcome);
        if failed {
            break;
        }
    }
    let outcome = last.expect("at least one test command");
    let run = GateRun {
        ok: outcome.ok,
        at: now(),
        commit: workspace.head()?,
        detail: if outcome.ok {
            format!("{:.0} s", outcome.seconds)
        } else {
            outcome.tail.lines().last().unwrap_or("failed").to_owned()
        },
    };
    let label = kind.label();
    workspace.update(|info| {
        info.gates.tests.insert(label, run);
    })?;
    Ok(outcome)
}

#[cfg(test)]
#[path = "testrun_tests.rs"]
mod testrun_tests;
