//! 在工作区里编译 Rust 引擎与 Fortran 内核（docs/design-ai-assistant.md 第 5.2 节）。
//!
//! 命令都预先定义，参数受校验；在沙箱里跑（断网，只能写工作区与临时目录）。所以 cargo 用 `--offline --locked`：
//! 依赖要事先在本机的 cargo 缓存里（在正式仓库里编译过一次就有了）。真缺依赖时报错，由用户决定是否放开联网。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

use crate::gates::GateRun;
use crate::layout::{now, Workspace};
use crate::sandbox::{self, Policy, SandboxInfo};

/// Rust 引擎的四个程序：（包，可执行文件）。
pub const ENGINE_BINARIES: [(&str, &str); 4] = [
    ("colm-cli", "colm-cli"),
    ("colm-srfdata", "mksrfdata-rs"),
    ("colm-init", "mkinidata-rs"),
    ("colm-runtime", "colm-rs"),
];

/// `build_kernel.sh` 认的预设。
pub const KERNEL_PRESETS: [&str; 17] = [
    "default",
    "usgs",
    "bgc",
    "urban",
    "crop",
    "latlon",
    "latlon-usgs",
    "latlon-crop",
    "latlon-hyper",
    "unstructured",
    "unstructured-usgs",
    "unstructured-crop",
    "unstructured-hyper",
    "catchment",
    "catchment-usgs",
    "catchment-crop",
    "catchment-hyper",
];

pub fn check_preset(preset: &str) -> Result<()> {
    ensure!(
        KERNEL_PRESETS.contains(&preset),
        "unknown kernel preset {preset:?}; use one of {}",
        KERNEL_PRESETS.join(", ")
    );
    Ok(())
}

/// 一条命令跑完的结果。
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub ok: bool,
    pub command: String,
    pub log: PathBuf,
    /// 输出的最后几十行（出错时先看这里）。
    pub tail: String,
    pub seconds: f64,
    pub sandbox: SandboxInfo,
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// 在沙箱里跑一条命令，输出全部写进 `reports/<label>-<时间>.log`，返回结果。
pub fn run_logged(
    workspace: &Workspace,
    label: &str,
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(&str, String)],
    allow_network: bool,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Outcome> {
    std::fs::create_dir_all(workspace.reports())?;
    let log = workspace
        .reports()
        .join(format!("{label}-{}.log", now()));
    let policy = Policy {
        writable: vec![workspace.dir.clone()],
        allow_network,
    };
    let (mut command, info) = sandbox::wrap(program, args, &policy);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot start {}", program.display()))?;
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst)) {
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", "--", &format!("-{}", child.id())])
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled; the process was stopped");
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let combined = format!("$ {} {}\n{stdout}\n--- stderr ---\n{stderr}", program.display(), args.join(" "));
    std::fs::write(&log, &combined)?;
    Ok(Outcome {
        ok: status.success(),
        command: format!("{} {}", program.display(), args.join(" ")),
        log,
        tail: tail(&format!("{stdout}\n{stderr}"), 40),
        seconds: started.elapsed().as_secs_f64(),
        sandbox: info,
    })
}

fn gate(workspace: &Workspace, outcome: &Outcome) -> Result<GateRun> {
    Ok(GateRun {
        ok: outcome.ok,
        at: now(),
        commit: workspace.head()?,
        detail: if outcome.ok {
            format!("{:.0} s, log {}", outcome.seconds, outcome.log.display())
        } else {
            outcome.tail.lines().last().unwrap_or("failed").to_owned()
        },
    })
}

fn program(name: &str) -> PathBuf {
    PathBuf::from(name)
}

/// 编译 Rust 引擎：四个程序，放进 `bin/`。
pub fn build_engine(
    workspace: &mut Workspace,
    allow_network: bool,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Outcome> {
    let mut args: Vec<String> = vec!["build".into(), "--release".into(), "--locked".into()];
    if !allow_network {
        args.push("--offline".into());
    }
    for (package, binary) in ENGINE_BINARIES {
        args.extend(["-p".into(), package.into(), "--bin".into(), binary.into()]);
    }
    let target = workspace.target();
    let src = workspace.src();
    let outcome = run_logged(
        workspace,
        "build-engine",
        &program("cargo"),
        &args,
        &src,
        &[("CARGO_TARGET_DIR", target.display().to_string())],
        allow_network,
        cancel,
    )?;
    if outcome.ok {
        std::fs::create_dir_all(workspace.bin())?;
        for (_, binary) in ENGINE_BINARIES {
            let from = target.join("release").join(binary);
            ensure!(from.is_file(), "built, but {} is missing", from.display());
            let to = workspace.bin().join(binary);
            let _ = std::fs::remove_file(&to);
            std::fs::copy(&from, &to)
                .with_context(|| format!("cannot copy {} into bin/", binary))?;
        }
    }
    workspace.info.gates.engine = Some(gate(workspace, &outcome)?);
    workspace.save()?;
    Ok(outcome)
}

/// 编译一个 Fortran 内核到 `kernels/<预设>/`（要 gfortran、netCDF-Fortran；空间预设还要 MPI）。
pub fn build_kernel(
    workspace: &mut Workspace,
    preset: &str,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Outcome> {
    check_preset(preset)?;
    let src = workspace.src();
    let script = src.join("oracle/scripts/build_kernel.sh");
    ensure!(
        script.is_file(),
        "{} is missing; this source tree has no build_kernel.sh",
        script.display()
    );
    let outdir = workspace.kernels();
    let args = vec![
        script.display().to_string(),
        preset.to_owned(),
        outdir.display().to_string(),
    ];
    let outcome = run_logged(
        workspace,
        &format!("build-kernel-{preset}"),
        &program("bash"),
        &args,
        &src,
        &[],
        false,
        cancel,
    )?;
    if outcome.ok {
        let manifest = outdir.join(preset).join("manifest.json");
        ensure!(manifest.is_file(), "built, but {} is missing", manifest.display());
    }
    workspace
        .info
        .gates
        .kernels
        .insert(preset.to_owned(), gate(workspace, &outcome)?);
    workspace.save()?;
    Ok(outcome)
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod build_tests;
