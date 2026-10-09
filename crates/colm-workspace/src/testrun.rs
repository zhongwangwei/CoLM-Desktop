//! 测试白名单（docs/design-ai-assistant.md 第 4 节 `run_tests`）：指定 crate 的 `cargo test`、漂移检查、
//! oracle 分层检查、`check-gui`。没有别的：参数是枚举加校验过的包名，不是任意命令。

use std::path::PathBuf;

use anyhow::{bail, ensure, Result};

use crate::build::{run_logged, Outcome};
use crate::gates::GateRun;
use crate::layout::{now, Workspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `cargo test -p <包> --lib --bins`（串行：colm-init 并行跑会撞 HDF5）。
    Cargo(String),
    /// 漂移检查：`crates/*/tests/drift*.rs` 全部跑一遍。它们把从 Fortran 源码生成的 Rust 表（地类常量、CO₂、
    /// history 变量、配置字段）重新生成并逐字节比对——改了一边忘了另一边，这里就会失败。
    /// 是集成测试，`Cargo` 那一类（`--lib --bins`）跑不到它们。
    Drift,
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
            Self::Drift => "drift".into(),
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
            "drift" => Self::Drift,
            "oracle" => Self::Oracle,
            "check-gui" => Self::CheckGui,
            other => bail!("unknown test kind {other:?}; use cargo, drift, oracle or check-gui"),
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

/// 工作区里的漂移测试：`(包, 测试名)`，按名字排序。包目录名就是包名（与 [`check_package`] 同一约定）。
pub fn drift_tests(workspace: &Workspace) -> Result<Vec<(String, String)>> {
    let mut found = Vec::new();
    let crates = workspace.src().join("crates");
    for entry in std::fs::read_dir(&crates)?.flatten() {
        let package = entry.file_name().to_string_lossy().into_owned();
        let Ok(tests) = std::fs::read_dir(entry.path().join("tests")) else {
            continue;
        };
        for test in tests.flatten() {
            let file = test.file_name().to_string_lossy().into_owned();
            if let Some(stem) = file.strip_suffix(".rs").filter(|s| s.starts_with("drift")) {
                found.push((package.clone(), stem.to_owned()));
            }
        }
    }
    found.sort();
    Ok(found)
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
        Kind::Drift => {
            let tests = drift_tests(workspace)?;
            ensure!(
                !tests.is_empty(),
                "no drift tests (crates/*/tests/drift*.rs) in this workspace"
            );
            tests
                .iter()
                .map(|(package, test)| cargo(&["test", "-p", package, "--test", test]))
                .collect()
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
    // 开始时就固定提交：跑测试期间又打了补丁的话，结果不能记在新提交名下。
    let commit = crate::build::pin_commit(workspace)?;
    let src = workspace.src();
    let target = workspace.target();
    let mut last: Option<Outcome> = None;
    // 一类测试可能有几条命令（漂移检查每个测试一条）：全过时报告里列出全部命令、用时累加；
    // 有一条失败就停在那里，报告的就是失败的那一条。
    let mut commands_run = Vec::new();
    let mut seconds = 0.0;
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
        commands_run.push(outcome.command.clone());
        seconds += outcome.seconds;
        last = Some(outcome);
        if failed {
            break;
        }
    }
    let mut outcome = last.expect("at least one test command");
    if outcome.ok {
        outcome.command = commands_run.join("; ");
        outcome.seconds = seconds;
    }
    let unchanged = crate::build::still_at(workspace, &commit)?;
    let run = GateRun {
        ok: outcome.ok && unchanged,
        at: now(),
        commit,
        detail: if !unchanged {
            "source changed while the tests ran; run them again on the current commit".into()
        } else if outcome.ok {
            format!("{:.0} s", outcome.seconds)
        } else {
            outcome.tail.lines().last().unwrap_or("failed").to_owned()
        },
    };
    crate::build::disown_if_failed(&mut outcome, &run);
    let label = kind.label();
    workspace.update(|info| {
        info.gates.tests.insert(label, run);
    })?;
    Ok(outcome)
}

#[cfg(test)]
#[path = "testrun_tests.rs"]
mod testrun_tests;
