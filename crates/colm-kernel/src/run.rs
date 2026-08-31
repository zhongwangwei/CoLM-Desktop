//! 三段编排：mksrfdata → mkinidata → colm。
//!
//! 每一段都是「跑 → 收日志 → 判成败 → 抽覆盖」。判成败在 `outcome`，
//! 抽覆盖在 `overrides`，本模块只负责把它们串起来并落一份日志。
//!
//! stdout 与 stderr 都要收。gfortran 运行时的错误只走 stderr，所以
//! `FAILURE_MARKERS` 里的 `Fortran runtime error` 与 `Error termination`
//! 在只读 stdout 时**永远不可能命中**；实测 namelist 文件缺失时 stdout 是
//! 0 字节而 stderr 有 302 字节，日志会空得看不出原因。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::manifest::Kernel;
use crate::outcome::{adjudicate, Outcome, Stage};
use crate::overrides::{extract, Override};

/// 一段跑完之后知道的一切。
#[derive(Debug)]
pub struct StageReport {
    pub stage: Stage,
    pub outcome: Outcome,
    /// 日志落盘的位置。失败时报给用户看的就是它。
    pub log: PathBuf,
    /// CoLM 在这一段里声明的静默覆盖。
    pub overrides: Vec<Override>,
}

impl StageReport {
    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, Outcome::Succeeded)
    }
}

/// 跑一段，只在结束时拿到全部输出。
///
/// `artifacts` 是这一段必须产出的文件，交给 `adjudicate` 做硬校验 ——
/// 必须列到**文件**，不能只列目录：目录在程序写任何东西之前就已存在，
/// 于是「跑完了但什么都没写」恰好抓不到。
pub fn run_stage(
    kernel: &Kernel,
    stage: Stage,
    namelist: &Path,
    work: &Path,
    artifacts: &[PathBuf],
) -> Result<StageReport> {
    run_stage_streaming(kernel, stage, namelist, work, artifacts, &mut |_| {})
}

/// 同上，但每读到 stdout 的一行就交给 `on_line` 一次。
///
/// **为什么需要它。** `colm.x` 在一次 528 步的运行里打出 5330 行，其中
/// 528 行是 `TIMESTEP = n | DATE = ...`；GUI 的进度条与日志窗全靠它们。
/// 用 `Command::output()` 的话这些行要等整段跑完才一起到达，进度条从
/// 0 直接跳到 100，日志窗在运行期间一片空白 —— 界面那边的限流、批量发送、
/// `TIMESTEP` 解析全都建在一个永远不会按时到达的输入上。
///
/// 传给 `on_line` 的是**去掉行尾换行的一行**；写进日志的仍是原始字节，
/// 逐字节与 `Command::output()` 那条路相同（下面按 `read_until` 收，
/// 保留行尾符，不重新拼接）。
///
/// stderr 由一个单独的线程读到底。**必须是单独的线程**：两个管道都由本进程
/// 读，如果先把 stdout 读完再读 stderr，子进程在 stderr 管道写满时就会阻塞，
/// 而本进程正等着一个再也不会来的 stdout —— 双方各等各的。
/// 代价是 stderr 不参与逐行回调，它整块在末尾追加。这是可以接受的：
/// gfortran 的运行时错误意味着这一段已经结束了，没有「实时」可言。
/// Windows 上不让子进程弹出自己的控制台窗口。
///
/// **一次运行会起四个进程**：界面起 `colm-cli`，它再依次起 `mksrfdata.x`、
/// `mkinidata.x`、`colm.x`。四个都是控制台程序，而 Windows 默认给控制台
/// 程序开一个新窗口 —— 于是点一次「运行」，屏幕上会闪出四个黑框，
/// 其中跑模型那个还会停留几分钟，关掉它就把模型杀了。
///
/// `CREATE_NO_WINDOW` = 0x0800_0000。不用 `DETACHED_PROCESS`：
/// 那个会让子进程脱离作业对象，界面退出时模型还在后台跑。
///
/// 非 Windows 平台是恒等函数 —— **判断放在这里而不是每个调用点**，
/// 否则四处各写一遍 `#[cfg(windows)]`，漏掉一处就是一个只在 Windows 上
/// 看得见的窗口，而开发机上永远复现不出来。
pub fn no_console(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// GUI 启动的顶层 `colm-cli` 放进自己的进程组。
///
/// 用户点取消时，界面可以按这个进程组杀掉 `colm-cli` 与它正在跑的
/// `mksrfdata.x`/`mkinidata.x`/`colm.x`，而不是只杀父进程留下 Fortran 孤儿。
/// Windows 走 `taskkill /T`，这里只保留不弹黑框的标志。
pub fn top_level_sidecar(cmd: &mut std::process::Command) -> &mut std::process::Command {
    no_console(cmd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

/// RangeCheck 的逐变量播报里**不带异常标记**的那些行。
///
/// 实测一次 11 年的 AT-Neu：2208 万行，占 `colm.log` 2.0 GB 的绝大部分。
/// 90 个站点就是 180 GB —— 而且这些字节先在内存里攒成一个 `String` 才落盘。
///
/// **丢掉它们是无损的。** `MOD_RangeCheck.F90:262-270` 在越界或出现 NAN 时
/// 往同一行尾部追加 ` with NAN` / ` Out of Range!`，而带标记的行**不满足**
/// 这里的判据、会原样留下；`case.nml` 里打开 `DEF_USE_CoLMDEBUG` 的话，
/// `MOD_RangeCheck.F90:295` 还会直接 `CoLM_stop` 把运行终止掉 ——
/// 所以一次异常既进日志也让运行失败，两条路都不依赖这些播报行。
///
/// `RangeCheck`/`CoLMDEBUG` 本身也是运行时开关了（`DEF_USE_RangeCheck` /
/// `DEF_USE_CoLMDEBUG`，`MOD_Namelist.F90`，默认 `.false.`），不是编译期宏——
/// 默认设置下这个函数基本不会遇到匹配的行，因为内核压根不打印它们；
/// 用户在 `case.nml` 里打开调试之后，丢弃这段逻辑才会真正派上用场。
///
/// 判据取「以 `)` 收尾」而不是只看前缀：范围那一对括号是格式串的最后一项，
/// 带标记的行一定在它后面还有字符。往**留下**的方向偏 ——
/// 少删一行只是日志大一点，多删一行可能删掉唯一的线索。
pub fn is_benign_rangecheck(line: &str) -> bool {
    let t = line.trim_end();
    let t = t.trim_start();
    // 两种前缀：`MOD_RangeCheck.F90:148` 是 block（栅格），其余是 vector。
    // block 那个中间是**两个空格**，照抄不改。
    (t.starts_with("Check vector data:") || t.starts_with("Check block  data:"))
        && t.ends_with(')')
        && !t.contains("Out of Range")
        && !t.contains("NAN")
}

pub fn run_stage_streaming(
    kernel: &Kernel,
    stage: Stage,
    namelist: &Path,
    work: &Path,
    artifacts: &[PathBuf],
    on_line: &mut dyn FnMut(&str),
) -> Result<StageReport> {
    run_stage_streaming_ranks(kernel, stage, namelist, work, artifacts, 1, on_line)
}

/// 普通 MPI/SPMD 启动：每个 rank 执行同一个程序和同一份 namelist，不引入
/// master/io/worker 角色。MPI 内核即使只有一个 rank 也必须经 launcher 启动。
pub fn run_stage_streaming_ranks(
    kernel: &Kernel,
    stage: Stage,
    namelist: &Path,
    work: &Path,
    artifacts: &[PathBuf],
    ranks: usize,
    on_line: &mut dyn FnMut(&str),
) -> Result<StageReport> {
    let exe = kernel.program(stage.program());
    let uses_mpi = kernel.manifest.macros.iter().any(|item| item == "USEMPI");
    let (program, args) = launch_command(&exe, namelist, ranks, uses_mpi)?;
    let mut cmd = Command::new(&program);
    if uses_mpi {
        configure_mpi_runtime(&mut cmd, &exe)?;
    }
    let mut child = no_console(&mut cmd)
        .args(args)
        .current_dir(work)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", program.display()))?;

    let mut err_pipe = child.stderr.take().context("no stderr pipe")?;
    let errs = std::thread::spawn(move || {
        let mut raw = Vec::new();
        let _ = err_pipe.read_to_end(&mut raw);
        raw
    });

    let mut text = String::new();
    let mut muted = 0usize;
    let mut terminated_after_success = false;
    {
        let out = child.stdout.take().context("no stdout pipe")?;
        let mut reader = BufReader::new(out);
        let mut raw = Vec::new();
        while reader.read_until(b'\n', &mut raw).unwrap_or(0) > 0 {
            let chunk = String::from_utf8_lossy(&raw);
            let line = chunk.trim_end_matches(['\n', '\r']);
            // 无异常标记的 RangeCheck 播报既不进日志也不进回调。
            // **两处一起挡**：日志是 2 GB 的那一半，回调是把这 2 GB
            // 逐行推过进程边界给界面、再由界面丢掉的那一半。
            if is_benign_rangecheck(line) {
                muted += 1;
                raw.clear();
                continue;
            }
            on_line(line);
            text.push_str(&chunk);

            // MSYS2 的 netCDF DLL 会在 AWS SDK 的进程退出清理里永久等待。
            // 成功标记是每个 CoLM 程序的最后一行，且文件都已关闭；此时终止
            // 进程只跳过坏掉的 DLL 析构，后面的错误标记与产物检查仍会执行。
            // ponytail: remove when MSYS2's netCDF no longer links the hanging AWS cleanup.
            if kernel.manifest.platform.starts_with("MINGW")
                && line.contains(stage.success_marker())
                && child
                    .try_wait()
                    .context("cannot inspect the child")?
                    .is_none()
            {
                child
                    .kill()
                    .context("cannot terminate the completed child")?;
                terminated_after_success = true;
                break;
            }
            raw.clear();
        }
    }

    let status = child.wait().context("cannot wait for the child")?;
    let raw_err = errs
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))?;
    let stderr = String::from_utf8_lossy(&raw_err);
    if !stderr.is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&stderr);
    }

    // 说出丢了多少。**静默地少掉 2200 万行，与「这一段没做范围检查」
    // 在日志上长得一样** —— 而那两件事完全不同。
    if muted > 0 {
        text.push_str(&format!(
            "\n--- {muted} 行无异常的 RangeCheck 播报未记入本日志。\n带 NAN / Out of Range 标记的行一律保留；\ncase.nml 里打开了 DEF_USE_CoLMDEBUG 的话，那种行还会直接终止运行。---\n"
        ));
    }

    let log = work.join(format!("{}.log", stage.program()));
    std::fs::write(&log, text.as_bytes())
        .with_context(|| format!("cannot write {}", log.display()))?;

    Ok(StageReport {
        stage,
        outcome: adjudicate(
            stage,
            if terminated_after_success {
                Some(0)
            } else {
                status.code()
            },
            &text,
            artifacts,
        ),
        log,
        overrides: extract(&text),
    })
}

fn launch_command(
    exe: &Path,
    namelist: &Path,
    ranks: usize,
    uses_mpi: bool,
) -> Result<(PathBuf, Vec<String>)> {
    anyhow::ensure!(ranks > 0, "MPI rank count must be at least 1");
    if !uses_mpi {
        anyhow::ensure!(ranks == 1, "non-MPI kernels only support one rank");
        return Ok((
            exe.to_path_buf(),
            vec![namelist.to_string_lossy().into_owned()],
        ));
    }
    let program = std::env::var_os("COLM_MPIEXEC")
        .map(PathBuf::from)
        .or_else(|| bundled_mpi_root(exe).and_then(|root| bundled_mpiexec(&root)))
        .unwrap_or_else(|| PathBuf::from("mpiexec"));
    let mut args: Vec<String> = std::env::var("COLM_MPIEXEC_ARGS")
        .ok()
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    args.extend([
        "-n".into(),
        ranks.to_string(),
        exe.to_string_lossy().into_owned(),
        namelist.to_string_lossy().into_owned(),
    ]);
    Ok((program, args))
}

fn bundled_mpi_root(exe: &Path) -> Option<PathBuf> {
    let root = exe.parent()?.parent()?.join("_runtime");
    root.is_dir().then_some(root)
}

fn bundled_mpiexec(root: &Path) -> Option<PathBuf> {
    ["mpiexec", "mpiexec.exe"]
        .into_iter()
        .map(|name| root.join("bin").join(name))
        .find(|path| path.is_file())
}

fn configure_mpi_runtime(cmd: &mut Command, exe: &Path) -> Result<()> {
    let Some(root) = bundled_mpi_root(exe) else {
        return Ok(());
    };
    let bin = root.join("bin");
    let lib = root.join("lib");
    prepend_env_path(cmd, "PATH", &bin)?;
    #[cfg(target_os = "linux")]
    prepend_env_path(cmd, "LD_LIBRARY_PATH", &lib)?;
    #[cfg(target_os = "macos")]
    {
        prepend_env_path(cmd, "DYLD_LIBRARY_PATH", &lib)?;
        prepend_env_path(cmd, "DYLD_FALLBACK_LIBRARY_PATH", &lib)?;
    }
    cmd.env("OPAL_PREFIX", &root)
        .env("PRTE_PREFIX", &root)
        .env("PMIX_PREFIX", &root)
        .env("OMPI_MCA_component_path", lib.join("openmpi"));
    Ok(())
}

fn prepend_env_path(cmd: &mut Command, name: &str, first: &Path) -> Result<()> {
    let mut paths = vec![first.to_path_buf()];
    if let Some(current) = std::env::var_os(name) {
        paths.extend(std::env::split_paths(&current));
    }
    cmd.env(
        name,
        std::env::join_paths(paths).with_context(|| format!("cannot construct {name}"))?,
    );
    Ok(())
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
