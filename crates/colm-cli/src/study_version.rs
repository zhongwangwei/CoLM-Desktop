//! 研究用哪一版模型（界面“运行方式”里的模型版本）。
//!
//! 研究第一次以 `study-run --model-version NAME` 运行时，把那个修改版**拍成快照**：它的 `colm-cli`、`colm-rs` 与两个
//! Rust 前处理（改过 Fortran 时连同它的内核）复制到研究目录的 `version/` 下，记进 `model-version.json`。之后续跑、
//! 重试都用这份快照——工作区里接着改代码、重编，不会让一个研究的成员混着两版程序跑完。
//!
//! 版本记在单独的文件里而不是研究设定里：设定是 `deny_unknown_fields`，交给修改版自己的 `colm-cli` 去跑时它未必
//! 认得新字段。交出去的那个进程带着 [`DELEGATED_ENV`]，不会再按这个文件转交一次。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::study;

pub(crate) const FILE: &str = "model-version.json";
pub(crate) const DELEGATED_ENV: &str = "COLM_STUDY_BY_PARENT";
/// 快照里的程序：编排、主循环、两个前处理。
const PROGRAMS: [&str; 4] = ["colm-cli", "colm-rs", "mksrfdata-rs", "mkinidata-rs"];

/// 研究固定下来的模型版本。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct Pinned {
    pub name: String,
    pub head: String,
    pub base_commit: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub based_on: Option<String>,
    pub touches: colm_workspace::version::Touches,
    /// 快照里的 `colm-cli`。
    pub cli: PathBuf,
    /// 用哪个内核：改过 Fortran 时是快照里的那份，否则是正式内核。
    pub kernel: PathBuf,
    pub pinned_unix: i64,
    #[serde(default)]
    pub model_revision: Option<colm_model::FrozenRevision>,
    #[serde(default)]
    pub model_root: Option<PathBuf>,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

pub(crate) fn pinned(study_dir: &Path) -> Result<Option<Pinned>> {
    let path = study_dir.join(FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text)?))
}

/// 研究有没有任何任务开始过（开始过就不能再换版本）。建研究时成员已经落成算例（`Materialized`），那还不算开始。
fn started(study_dir: &Path) -> Result<bool> {
    use study::state::TaskStatus;
    let Some(state) = study::runner::status_state(study_dir)? else {
        return Ok(false);
    };
    Ok(state
        .tasks
        .values()
        .any(|task| !matches!(task.status, TaskStatus::Pending | TaskStatus::Materialized)))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 把修改版拍成快照，写 `model-version.json`。
fn pin(
    study_dir: &Path,
    readiness: &colm_workspace::version::Readiness,
    official_kernel: &Path,
) -> Result<Pinned> {
    let dest = study_dir.join("version");
    let bin = dest.join("bin");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&bin)?;
    let source_bin = readiness
        .cli
        .parent()
        .context("the version's colm-cli has no directory")?;
    for program in PROGRAMS {
        let from = source_bin.join(exe(program));
        std::fs::copy(&from, bin.join(exe(program)))
            .with_context(|| format!("cannot snapshot {}", from.display()))?;
    }
    let kernel = if readiness.kernel != official_kernel {
        let to = dest.join("kernel");
        copy_dir(&readiness.kernel, &to)?;
        to
    } else {
        official_kernel.to_path_buf()
    };
    let pinned = Pinned {
        name: readiness.name.clone(),
        head: readiness.head.clone(),
        base_commit: readiness.base_commit.clone(),
        note: readiness.note.clone(),
        based_on: readiness.based_on.clone(),
        touches: readiness.touches,
        cli: bin.join(exe("colm-cli")),
        kernel,
        pinned_unix: crate::provenance::now() as i64,
        model_revision: None,
        model_root: None,
    };
    std::fs::write(study_dir.join(FILE), serde_json::to_string_pretty(&pinned)?)?;
    Ok(pinned)
}

/// 这次 `study-run` 用哪一版：已经固定过就用固定的（再给别的版本名就报错）；没固定过、给了修改版名、研究还没开始
/// 跑，就检查就绪并拍快照；其余情况是官方版（`None`）。
pub(crate) fn ensure(
    study_dir: &Path,
    requested: Option<&str>,
    workspace_root: &Path,
    engine: &str,
    preprocessors: &str,
    official_kernel: &Path,
) -> Result<Option<Pinned>> {
    let requested = requested.filter(|name| !name.is_empty());
    let result = match (pinned(study_dir)?, requested) {
        (Some(pinned), Some(name)) if name != pinned.name => bail!(
            "this Study was started with model version {}; it cannot switch to {name}. Create a new Study for another version",
            pinned.name
        ),
        (Some(pinned), _) => Ok(Some(pinned)),
        (None, None | Some("official")) => Ok(None),
        (None, Some(name)) => {
            if started(study_dir)? {
                bail!(
                    "this Study was started with the official version; it cannot switch to {name}. Create a new Study for another version"
                );
            }
            let readiness = crate::provenance::resolve(
                workspace_root,
                name,
                engine,
                preprocessors,
                official_kernel,
            )?;
            pin(study_dir, &readiness, official_kernel).map(Some)
        }
    }?;
    if let Some(pinned) = result.as_ref().filter(|p| p.model_revision.is_none()) {
        // A successful pin remains usable even if its history needs repair on the next run.
        if let Err(error) = record_pin(study_dir, workspace_root, pinned) {
            eprintln!("warning: study snapshot is saved, but its workspace ledger could not be updated: {error:#}");
        }
    }
    Ok(result)
}

fn record_pin(study_dir: &Path, workspace_root: &Path, pinned: &Pinned) -> Result<()> {
    let workspace = colm_workspace::Workspace::open(workspace_root, &pinned.name)?;
    let id = study_dir
        .file_name()
        .context("study directory has no name")?
        .to_string_lossy();
    colm_workspace::studies::append(
        &workspace,
        study_dir,
        &id,
        &pinned.head,
        "local",
        pinned.pinned_unix.max(0) as u64,
    )
}

/// 交给快照里的 `colm-cli` 跑这个研究。`args` 是去掉了版本与内核参数的 `study-run` 原参数（不含子命令）。
pub(crate) fn delegate(pinned: &Pinned, args: &[String]) -> Result<i32> {
    crate::require_forcing_exclusion(&pinned.cli)?;
    let status = std::process::Command::new(&pinned.cli)
        .arg("study-run")
        .args(args)
        .arg("--kernel")
        .arg(&pinned.kernel)
        .env(DELEGATED_ENV, "1")
        .env("COLM_AUDIT_BY_PARENT", "1")
        .status()
        .with_context(|| format!("cannot start {}", pinned.cli.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// 会按当前程序重算 provenance 的身份敏感命令：写回模型版本、提交/取回远程研究，以及
/// 给出“已核对输入/成绩”的单侧检查（`study-verified-side`、`study-verify-comparison`）。
/// 它们都读研究 pin 并调用 `verify_frozen_inputs`：参数目录与来源哈希编在程序里，pin 到旧修订后
/// 由当前程序重算会与 pin 时不同，输入被误报成变了（#6b，P1-8 同类）。后两个命令虽只读，
/// 但它们的“已核对”结论同样来自这次重算，所以也必须由 pin 的程序来做。
///
/// 为什么 `study-compare` 不在这里：它有两个研究目录，两侧可能各自 pin 到不同的 CLI；整条命令
/// 交给其中一份程序，就会用那份程序校验另一侧。按侧分别起进程是另一个改动，本批明确不静默用
/// 一份 CLI 校验另一侧。
const PINNED_CLI_COMMANDS: [&str; 6] = [
    "study-save-version",
    "model-import-study",
    "remote-study-run",
    "remote-study-fetch",
    "study-verified-side",
    "study-verify-comparison",
];

/// 这些命令从哪取研究目录。各入口的实际读法不同，不能统一按位置参数取。
fn pinned_command_study_dir(command: &str, opts: &crate::Opts) -> Option<PathBuf> {
    if !PINNED_CLI_COMMANDS.contains(&command) {
        return None;
    }
    if command == "model-import-study" {
        opts.get("--study-dir").map(PathBuf::from)
    } else {
        opts.positional.first().map(PathBuf::from)
    }
}

/// 转交整条命令：子命令、原位置参数、原 flags（含重复 flag 与原顺序），不重解释路径与值。
fn pinned_command_args(command: &str, opts: &crate::Opts) -> Vec<String> {
    let mut args = vec![command.to_owned()];
    args.extend(opts.positional.iter().cloned());
    for (flag, value) in &opts.flags {
        args.extend([flag.clone(), value.clone()]);
    }
    args
}

/// 研究 pin 的 CLI 与当前程序不同时，返回整条命令该交给的那份程序；`None` 表示按当前程序继续。
///
/// 为什么整条转交：`verify_frozen_inputs` 用程序内嵌的参数目录与来源文件重算 provenance 哈希。
/// 研究 pin 到旧修订后，当前程序算出的值与 pin 时由旧程序算出的不同，写回/提交会被误报成
/// “输入变了”（#6b，P1-8 同类）。协议与 `study-run` 的 [`delegate`] 一致：参数原值透传，
/// 子进程带 [`DELEGATED_ENV`] 防止再转交，审计由父进程记（`COLM_AUDIT_BY_PARENT`），退出码
/// 原样交回调用方。
///
/// 单独拆出“决定交给谁”不是为了复用，而是因为远程命令的 SSH 凭据由调用方写在 stdin 上：
/// 一旦当前进程先读了凭据，被转交的程序就读不到了（见 main 里的调用顺序）。
pub(crate) fn pinned_command_program(command: &str, opts: &crate::Opts) -> Result<Option<PathBuf>> {
    // 已经由父进程交出来的程序不再转交：它的 current_exe 可能不是 pin 里记的那条路径。
    if std::env::var_os(DELEGATED_ENV).is_some() {
        return Ok(None);
    }
    let Some(study_dir) = pinned_command_study_dir(command, opts) else {
        return Ok(None);
    };
    let Some(pin) = pinned(&study_dir)? else {
        return Ok(None);
    };
    let current =
        std::env::current_exe().context("cannot locate the running colm-cli executable")?;
    if same_program(&pin.cli, &current) {
        return Ok(None);
    }
    anyhow::ensure!(
        pin.cli.is_file(),
        "this Study was created with model version {} whose colm-cli is missing at {}; restore that version's program or create the Study again",
        pin.name,
        pin.cli.display()
    );
    Ok(Some(pin.cli))
}

/// 用 [`pinned_command_program`] 指定的程序执行整条命令，返回它的退出码。stdout/stderr 继承，
/// 输出直接交给调用方（GUI sidecar）；`auth_line` 是调用方从 stdin 读到的 SSH 凭据原文，按原样
/// 从子进程的 stdin 送进去——远程命令的凭据只能读一次，父进程读了就必须转给真正执行的程序。
pub(crate) fn run_pinned_command(
    program: &Path,
    command: &str,
    opts: &crate::Opts,
    auth_line: Option<&str>,
) -> Result<i32> {
    let mut child = std::process::Command::new(program);
    child
        .args(pinned_command_args(command, opts))
        .env(DELEGATED_ENV, "1")
        .env("COLM_AUDIT_BY_PARENT", "1");
    if auth_line.is_some() {
        child.stdin(std::process::Stdio::piped());
    }
    let mut child = child.spawn().with_context(|| {
        format!(
            "cannot start the Study's model version program {}",
            program.display()
        )
    })?;
    if let Some(line) = auth_line {
        use std::io::Write;
        let mut stdin = child
            .stdin
            .take()
            .context("the delegated colm-cli has no stdin")?;
        let mut payload = line.to_owned();
        if !payload.ends_with('\n') {
            payload.push('\n');
        }
        if let Err(error) = stdin.write_all(payload.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error)
                .context("cannot forward SSH authentication to the Study's model version program");
        }
    }
    let status = child.wait().with_context(|| {
        format!(
            "cannot wait for the Study's model version program {}",
            program.display()
        )
    })?;
    Ok(status.code().unwrap_or(1))
}

/// 创建/预检研究时该由哪份程序写 manifest 与 provenance。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CreationExecutor {
    Local,
    Delegate(PathBuf),
}

fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

fn same_program(left: &Path, right: &Path) -> bool {
    resolved(left) == resolved(right)
}

/// 运行研究时用的是该修订的 `revision_cli`（[`delegate`]），创建/预检必须由同一份程序做：
/// `verify_frozen_inputs` 用程序内嵌的参数目录与来源文件重算 provenance 哈希，两份不同代码算出的
/// 值对不上，研究就会建得成却跑不起来（P1-8）。
///
/// 只看 `revision_cli` 是不是当前可执行文件，不再对官方/workspace 特判：官方修订也可能是另一份
/// 构建（缓存里那份），按官方就本地处理会让创建与运行用两份程序算哈希。CLI 还不存在时也返回
/// `Delegate`，由 [`delegate_creation`] 按运行侧同一条路径先补齐，补不齐再明确拒绝。
pub(crate) fn creation_executor(current_exe: &Path, revision_cli: &Path) -> CreationExecutor {
    if same_program(current_exe, revision_cli) {
        CreationExecutor::Local
    } else {
        CreationExecutor::Delegate(revision_cli.to_owned())
    }
}

/// 按运行侧同一条路径准备好修订自己的 `colm-cli`。
///
/// workspace 修订缓存不在时，[`revision_cli`] 会退到工作区现在编的那份（可能已经改过），所以即使
/// 文件存在也必须先把代码冻结进缓存；官方修订的程序就在按内容命名的缓存里，文件在就说明已经
/// 准备过。两边用到时才做，避免每次创建都重算源码哈希。
fn prepare_revision_cli(opts: &crate::Opts, revision: &colm_model::FrozenRevision) -> Result<()> {
    let ready = if revision.code.workspace.is_some() {
        crate::model_version::cached_options(opts, &revision.code)?.is_some()
    } else {
        crate::model_version::cached_official_cli(opts, &revision.code)?.is_file()
    };
    if ready {
        return Ok(());
    }
    crate::model_version::verify_code(opts, &revision.code)?;
    crate::model_version::cache_code(opts, &revision.code)?;
    Ok(())
}

/// 转交给修订 CLI 的参数：子命令、**原始**算例目录（不是 prepare_create 的临时冻结输入目录）、原样
/// 透传的 flags，以及研究设定。设定原本是位置参数时补成 `--spec`；已经是 flag 时不要写两遍。
pub(crate) fn creation_delegate_args(
    command: &str,
    case_root: &Path,
    spec_file: &Path,
    flags: &[(String, String)],
) -> Vec<String> {
    let mut args = vec![command.to_owned(), case_root.to_string_lossy().into_owned()];
    for (flag, value) in flags {
        args.extend([flag.clone(), value.clone()]);
    }
    if !flags.iter().any(|(flag, _)| flag == "--spec") {
        args.extend([
            "--spec".to_owned(),
            spec_file.to_string_lossy().into_owned(),
        ]);
    }
    args
}

/// 把创建/预检整条命令交给将来运行这份研究的那份 `colm-cli`。成功返回它的 stdout 交给调用方原样
/// 打印（`study-create` 打印的是研究目录）；`None` 表示没有选冻结修订、或当前程序就是那份程序，
/// 按现有路径继续。快照 CLI 自己的 `study-create` 会做它版本的 prepare/pin/capture，当前程序
/// 只负责转交、核对输出与退出码。
pub(crate) fn delegate_creation(
    command: &str,
    opts: &crate::Opts,
    case_root: &Path,
    spec_file: &Path,
) -> Result<Option<String>> {
    // 已经由父进程交出来的程序不再转交：它自己就是那份修订的 colm-cli。
    if std::env::var_os(DELEGATED_ENV).is_some() {
        return Ok(None);
    }
    let Some(version) = crate::model_version::selected(opts)? else {
        return Ok(None);
    };
    let store = crate::model_version::registry(opts)?;
    let revision = store.get(&version)?;
    // 官方修订也要走同一条准备路径：`revision_cli` 指向缓存里的程序，缓存缺失时运行侧同样起不来。
    // 准备与定位放在同一个上下文里：两者任一失败，用户都需要知道是哪个修订、下一步能做什么。
    let (cli, _) = (|| -> Result<_> {
        prepare_revision_cli(opts, &revision)?;
        revision_cli(opts, &revision)
    })()
    .with_context(|| {
        format!(
            "model version {} r{} must create or preflight this Study itself, but its colm-cli could not be prepared; rebuild that version or restore its source, then retry (or create the Study without freezing that model version)",
            revision.version.id, revision.version.revision
        )
    })?;
    let current =
        std::env::current_exe().context("cannot locate the running colm-cli executable")?;
    match creation_executor(&current, &cli) {
        CreationExecutor::Local => Ok(None),
        CreationExecutor::Delegate(program) => Ok(Some(create_with_revision_cli(
            command,
            &revision,
            &program,
            case_root,
            spec_file,
            &opts.flags,
        )?)),
    }
}

struct CreationRun {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_creation_cli(cli: &Path, args: &[String]) -> Result<CreationRun> {
    let output = std::process::Command::new(cli)
        .args(args)
        .env(DELEGATED_ENV, "1")
        .env("COLM_AUDIT_BY_PARENT", "1")
        .output()
        .with_context(|| {
            format!(
                "cannot start the model version's colm-cli at {}; rebuild this version, or create the Study with the official version",
                cli.display()
            )
        })?;
    Ok(CreationRun {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// 用修订自己的 `colm-cli` 建/预检研究。只有一种失败可以在本地兼容：旧修订不认 GUI 自动加的
/// `include_baseline`（与 [`ensure_revision_reads_spec`] 同一处置），去掉这个增强重试一次——改的是
/// 临时副本，绝不重写用户给的设定文件。其余失败原样带回并补上修订身份与可行动的方向。
fn create_with_revision_cli(
    command: &str,
    revision: &colm_model::FrozenRevision,
    cli: &Path,
    case_root: &Path,
    spec_file: &Path,
    flags: &[(String, String)],
) -> Result<String> {
    // 第一次尝试就用剥掉 parent_study 的副本：旧修订读不了这个新元数据，而新程序不介意少一个
    // 审计字段；创建成功后由这里把谱系写回研究目录。
    let sanitized = spec_for_older_program(spec_file, false)?;
    let mut args = creation_delegate_args(command, case_root, &sanitized, flags);
    // GUI 的 study-create/study-preflight 一直把 `--spec 原文件` 放在 flags 里；
    // `creation_delegate_args` 只在没有 --spec 时才补副本，于是剥离 parent_study 的副本根本
    // 没送出去（5a）。这里统一把已有的 --spec 重指到副本，第一次尝试与重试走同一条路径。
    set_spec_argument(&mut args, &sanitized);
    let (run, retry_spec) = (|| -> Result<(CreationRun, Option<PathBuf>)> {
        let run = run_creation_cli(cli, &args)?;
        if !run.success && run.stderr.contains("unknown field `include_baseline`") {
            let copy = spec_for_older_program(spec_file, true)?;
            let mut retry = args.clone();
            set_spec_argument(&mut retry, &copy);
            return match run_creation_cli(cli, &retry) {
                Ok(run) => Ok((run, Some(copy))),
                Err(error) => {
                    let _ = std::fs::remove_file(&copy);
                    Err(error)
                }
            };
        }
        Ok((run, None))
    })()?;
    let result = finish_creation(command, revision, case_root, run);
    if let Some(copy) = retry_spec {
        let _ = std::fs::remove_file(&copy);
    }
    let _ = std::fs::remove_file(&sanitized);
    let root = result?;
    if command == "study-create" {
        study::spec::write_parent_study_metadata(Path::new(root.trim()), spec_file)?;
    }
    Ok(root)
}

fn finish_creation(
    command: &str,
    revision: &colm_model::FrozenRevision,
    case_root: &Path,
    run: CreationRun,
) -> Result<String> {
    if !run.success {
        return Err(creation_failure(command, revision, &run));
    }
    // 修订程序的警告照常出现在父进程的 stderr 上，不吞掉。
    if !run.stderr.is_empty() {
        eprint!("{}", run.stderr);
    }
    if command == "study-create" {
        let Some(root) = study_root_from_stdout(&run.stdout) else {
            return Err(anyhow::anyhow!(
                "model version {} r{} reported a created Study but printed no Study directory; create the Study with the official version, or rebuild this version on the current code",
                revision.version.id,
                revision.version.revision
            ));
        };
        if let Err(error) = bound_revision(Path::new(root), revision) {
            discard_unusable_study(Path::new(root), case_root);
            return Err(anyhow::anyhow!(
                "model version {} r{} was built from older code that did not bind the Study it created at {root} to this revision ({error:#}); create the Study with the official version, or rebuild this version on the current code",
                revision.version.id,
                revision.version.revision
            ));
        }
    }
    Ok(run.stdout)
}

fn creation_failure(
    command: &str,
    revision: &colm_model::FrozenRevision,
    run: &CreationRun,
) -> anyhow::Error {
    let verb = if command == "study-create" {
        "create"
    } else {
        "preflight"
    };
    let detail = stderr_tail(&run.stderr);
    if outdated_code(&run.stderr) {
        return anyhow::anyhow!(
            "model version {} r{} was built from older code that cannot {verb} this Study ({detail}); create the Study with the official version, or rebuild this version on the current code",
            revision.version.id,
            revision.version.revision
        );
    }
    let status = run
        .code
        .map_or_else(String::new, |code| format!(" (exit status {code})"));
    anyhow::anyhow!(
        "model version {} r{} could not {verb} this Study{status}: {detail}",
        revision.version.id,
        revision.version.revision
    )
}

/// 修订程序报的错是不是"这份代码比这个功能老"。`unknown command` 说明它连 `study-create` 都还没有。
fn outdated_code(stderr: &str) -> bool {
    [
        "unknown field",
        "unknown variant",
        "unexpected argument",
        "unknown command",
    ]
    .iter()
    .any(|marker| stderr.contains(marker))
}

fn stderr_tail(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return "no error output".to_owned();
    }
    let lines: Vec<&str> = trimmed.lines().rev().take(12).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn study_root_from_stdout(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

/// 研究目录里有没有这份修订的 `model-version.json`。还不会在创建时绑定修订的旧程序会跳过这一步，
/// 于是研究建得成、第一次运行才知道没有它——在这里就拦下并说清怎么办。
fn bound_revision(study_root: &Path, revision: &colm_model::FrozenRevision) -> Result<()> {
    let path = study_root.join(FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let pin: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    let version = &pin["model_revision"]["version"];
    anyhow::ensure!(
        version["id"].as_str() == Some(revision.version.id.as_str())
            && version["revision"].as_u64() == Some(revision.version.revision),
        "the Study does not record this revision"
    );
    Ok(())
}

/// 只删刚被那个子进程建出来、却绑定不了修订的研究：位置必须正好在 `<原始算例>/.colm/studies` 下
/// 且有 manifest，子进程输出被误读时绝不能删到用户别的目录。
fn discard_unusable_study(study_root: &Path, case_root: &Path) {
    let (Ok(root), Ok(case)) = (
        colm_kernel::manifest::absolute(study_root),
        colm_kernel::manifest::absolute(case_root),
    ) else {
        return;
    };
    let studies = case.join(".colm/studies");
    if root.parent() == Some(studies.as_path()) && root.join("manifest.json").is_file() {
        let _ = std::fs::remove_dir_all(root);
    }
}

/// 交给旧修订 CLI 的设定临时副本。
///
/// 旧修订的 `StudySpec` 是 `deny_unknown_fields`：这一轮新增的 `parent_study`（审计元数据）会被
/// 判未知字段；`include_baseline` 是 GUI 自动加的增强，旧修订也可能不认。两者都不属于执行语义，
/// 按同一份设定剥掉后写临时副本，用户给的设定文件保持原样。谱系由调用方在创建成功后写进研究目录的
/// [`study::spec::PARENT_STUDY_FILE`]。
fn spec_for_older_program(spec_file: &Path, drop_include_baseline: bool) -> Result<PathBuf> {
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(spec_file)?)
        .with_context(|| format!("invalid study spec {}", spec_file.display()))?;
    if let Some(map) = value.as_object_mut() {
        map.remove("parent_study");
        if drop_include_baseline {
            if let Some(budget) = map.get_mut("budget").and_then(|b| b.as_object_mut()) {
                // 旧 StudyBudget 是 deny_unknown_fields：它只看键在不在，写 `false` 一样被拒，
                // 所以整键删掉而不是置 false（5b）。
                budget.remove("include_baseline");
            }
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "colm-study-delegated-{}-{nanos}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
    Ok(path)
}

fn set_spec_argument(args: &mut [String], spec_file: &Path) {
    if let Some(index) = args.iter().position(|arg| arg == "--spec") {
        if let Some(value) = args.get_mut(index + 1) {
            *value = spec_file.to_string_lossy().into_owned();
        }
    }
}

/// 两个研究里最优成员的成绩（`study-compare`，“比较两个版本”）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Side {
    pub study: String,
    pub version: String,
    pub head: Option<String>,
    pub status: String,
    pub best_member: Option<String>,
    pub calibration: Option<f64>,
    pub validation: Option<f64>,
    pub baseline_calibration: Option<f64>,
    pub baseline_validation: Option<f64>,
}

fn comparison_version(study_dir: &Path) -> Result<(String, Option<String>)> {
    use crate::remote_cmd::remote_study::{recorded_model_version, RecordedModelVersion};

    if let Some(version) = pinned(study_dir)? {
        return Ok((version.name, Some(version.head)));
    }
    match recorded_model_version(study_dir)? {
        Some(RecordedModelVersion::Official) => Ok(("official".into(), None)),
        Some(RecordedModelVersion::Workspace { name, head, .. }) => Ok((name, Some(head))),
        // 没有 pin 也没有远程记录：本地 Study 只在 `--model-version` 时才写 pin，所以
        // 比较页仍按官方版处理（旧语义）。证据页面对同一情形报 unknown：见
        // `study::runner::evidence_model_identity`，不能把"没有记录"当成官方记录。
        None => Ok(("official".into(), None)),
    }
}

fn side(study_dir: &Path) -> Result<(Side, study::spec::StudySpec)> {
    let manifest = study::engine::status(study_dir)?;
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .map(|loaded| loaded.payload);
    let (version, head) = comparison_version(study_dir)?;
    let best = state.as_ref().and_then(|s| s.best_member.clone());
    let candidate = |member: &str| {
        state
            .as_ref()
            .and_then(|s| s.candidates.get(member))
            .cloned()
    };
    let baseline = manifest
        .members
        .iter()
        .find(|m| m.baseline)
        .and_then(|m| candidate(&m.id));
    let best_candidate = best.as_deref().and_then(candidate);
    Ok((
        Side {
            study: study_dir.display().to_string(),
            version,
            head,
            status: state.as_ref().map_or_else(
                || "not started".to_owned(),
                |s| format!("{:?}", s.status).to_lowercase(),
            ),
            best_member: best,
            calibration: best_candidate
                .as_ref()
                .and_then(|c| c.calibration)
                .or(state.as_ref().and_then(|s| s.best_objective)),
            validation: best_candidate.as_ref().and_then(|c| c.validation),
            baseline_calibration: baseline.as_ref().and_then(|c| c.calibration),
            baseline_validation: baseline.as_ref().and_then(|c| c.validation),
        },
        manifest.spec,
    ))
}

/// 两份设定除了内核目录之外是否一致（公平比较的前提：同样的参数、预算、种子、目标与时段）。
pub(crate) fn same_design(a: &study::spec::StudySpec, b: &study::spec::StudySpec) -> Result<bool> {
    let strip = |spec: &study::spec::StudySpec| -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(spec)?;
        if let Some(map) = value.as_object_mut() {
            map.remove("kernel_dir");
            // 谱系是审计元数据，不属于执行设计：远程 staging 会故意把 parent_study 清成 None
            // （服务器引擎可能不认识它），留在这里会让本机 Some / 远端 None 判成不同设计，
            // 派生研究取回时被误拒（7a）。
            map.remove("parent_study");
            // Paths relocate across hosts; observation contents are checked separately.
            map.insert(
                "observations".into(),
                serde_json::to_value(spec.observations.keys().collect::<Vec<_>>())?,
            );
        }
        Ok(value)
    };
    Ok(strip(a)? == strip(b)?)
}

/// 比较两个版本：两个研究各自率定的最优成员，看验证期（没有验证期就看率定期）上谁的目标值更低。
pub(crate) fn compare(a: &Path, b: &Path) -> Result<serde_json::Value> {
    let (left, left_spec) = side(a)?;
    let (right, right_spec) = side(b)?;
    let same = same_design(&left_spec, &right_spec)?;
    let inputs = comparison_inputs(a).and_then(|left_inputs| {
        comparison_inputs(b).map(|right_inputs| {
            let full = !left_inputs.base_cases.is_empty()
                && !right_inputs.base_cases.is_empty()
                && left_inputs
                    .base_cases
                    .values()
                    .chain(right_inputs.base_cases.values())
                    .all(crate::fingerprint::has_full_forcing_content);
            (
                study::model::same_conditions(&left_inputs, &right_inputs),
                full,
            )
        })
    });
    let inputs = inputs.and_then(|same_inputs| {
        anyhow::ensure!(
            left == side(a)?.0 && right == side(b)?.0,
            "Study comparison scores or model identity changed during verification"
        );
        Ok(same_inputs)
    });
    let verified = inputs.as_ref().is_ok_and(|(_, full)| *full);
    let same_inputs = inputs.as_ref().is_ok_and(|(same, _)| *same);
    let mut result = verdict(left, right, same);
    if !same_inputs {
        result["better"] = serde_json::Value::Null;
    }
    result["same_design"] = serde_json::json!(same);
    result["same_inputs"] = serde_json::json!(same_inputs);
    result["inputs_verified"] = serde_json::json!(verified);
    result["forcing_contents_verified"] = serde_json::json!(verified);
    let notes = result["notes"]
        .as_array_mut()
        .expect("verdict notes is an array");
    match inputs {
        Err(error) => notes.push(serde_json::json!(format!("full-content inputs, observations and outputs could not be verified: {error:#}; no version winner is reported"))),
        Ok((false, true)) => notes.push(serde_json::json!("the Studies use different scientific input or observation contents; no version winner is reported")),
        Ok((_, false)) => notes.push(serde_json::json!("forcing content fingerprints are excluded; only the recorded non-forcing scope was verified; no version winner is reported")),
        Ok((true, true)) => (),
    }
    Ok(result)
}

/// Each side is verified in its own process so SSH credentials cannot cross host boundaries.
pub(crate) fn verified_side(study_dir: &Path) -> Result<serde_json::Value> {
    let (side, spec) = side(study_dir)?;
    let verification = comparison_inputs(study_dir).and_then(|identity| {
        anyhow::ensure!(
            side == self::side(study_dir)?.0,
            "Study score or model identity changed during side verification"
        );
        Ok(identity)
    });
    let (inputs, verified, notes) = match verification {
        Ok(identity) => {
            let full = !identity.base_cases.is_empty()
                && identity
                    .base_cases
                    .values()
                    .all(crate::fingerprint::has_full_forcing_content);
            (
                Some(identity),
                full,
                if full {
                    Vec::new()
                } else {
                    vec!["forcing content fingerprints are excluded; non-forcing scope only".into()]
                },
            )
        }
        Err(error) => (
            None,
            false,
            vec![format!(
                "full-content scientific evidence could not be verified: {error:#}"
            )],
        ),
    };
    Ok(serde_json::json!({
        "schema":1,
        "side":side,
        "spec":spec,
        "input_identity":inputs,
        "verified":verified,
        "notes":notes,
    }))
}

fn comparison_inputs(study_dir: &Path) -> Result<study::model::StudyInputIdentity> {
    let record = study_dir.join(crate::remote_cmd::remote_study::RECORD);
    match std::fs::symlink_metadata(&record) {
        Ok(_) => return crate::remote_cmd::remote_study::verified_comparison(study_dir),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error).context("cannot inspect Study remote submission"),
    }
    local_comparison_inputs(study_dir)
}

fn local_comparison_inputs(study_dir: &Path) -> Result<study::model::StudyInputIdentity> {
    let manifest = study::engine::status(study_dir)?;
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study has no state")?
    .payload;
    anyhow::ensure!(
        state.schema_version == 1 && state.study_id == manifest.id,
        "Study state identity changed"
    );
    anyhow::ensure!(
        !matches!(
            state.status,
            study::state::StudyStatus::Running
                | study::state::StudyStatus::Ready
                | study::state::StudyStatus::NeedsReview
        ),
        "Study is not in a verified finished state"
    );
    let best = state
        .best_member
        .as_deref()
        .context("Study has no best member")?;
    let baseline = manifest
        .members
        .iter()
        .find(|member| member.baseline)
        .context("Study has no baseline member")?;
    let members = if baseline.id == best {
        vec![best]
    } else {
        vec![best, baseline.id.as_str()]
    };
    let before = study::runner::evidence_file_snapshot(&manifest, &state, &members)?;
    study::engine::verify_frozen_inputs(&manifest)?;
    let inputs = study::model::verified_inputs(&manifest)?;
    for member in &members {
        study::runner::verify_member_evidence(&manifest, &state, member)?;
    }
    anyhow::ensure!(
        before == study::runner::evidence_file_snapshot(&manifest, &state, &members)?,
        "Study comparison inputs or outputs changed during verification"
    );
    Ok(inputs)
}

/// Executed by the bound remote job's CLI, never accepted as a frontend assertion.
pub(crate) fn local_comparison_evidence(study_dir: &Path) -> Result<serde_json::Value> {
    let state = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study has no verified state")?
    .payload;
    let state_sha256 = colm_kernel::sha256_hex(&serde_json::to_vec(&state)?);
    let manifest = study::engine::status(study_dir)?;
    let best = state
        .best_member
        .as_deref()
        .context("Study has no best member")?;
    let baseline = manifest
        .members
        .iter()
        .find(|member| member.baseline)
        .context("Study has no baseline")?;
    let members = vec![best, baseline.id.as_str()];
    let files_before = study::runner::evidence_file_snapshot(&manifest, &state, &members)?;
    let inputs = local_comparison_inputs(study_dir)?;
    let results_sha256 = comparison_results_sha256(study_dir)?;
    let after = study::checkpoint::load_latest::<study::state::StudyState>(
        &study_dir.join("checkpoints/state"),
    )?
    .context("Study state disappeared during comparison")?
    .payload;
    anyhow::ensure!(
        state_sha256 == colm_kernel::sha256_hex(&serde_json::to_vec(&after)?),
        "Study state changed during comparison verification"
    );
    anyhow::ensure!(
        inputs == study::model::verified_inputs(&manifest)?,
        "Study scientific inputs changed during comparison verification"
    );
    anyhow::ensure!(
        files_before == study::runner::evidence_file_snapshot(&manifest, &after, &members)?,
        "Study raw outputs or evaluation evidence changed during comparison verification"
    );
    let pin = pinned(study_dir)?;
    let revision = pin.as_ref().and_then(|pin| pin.model_revision.as_ref());
    Ok(serde_json::json!({
        "schema":1,
        "study_id":manifest.id,
        "root":manifest.root,
        "spec":manifest.spec,
        "model_revision":revision,
        "pinned":pin,
        "best_member":state.best_member,
        "state_sha256":state_sha256,
        "state":state,
        "input_identity":inputs,
        "input_scope":"sha256-nonforcing-v1",
        "forcing_content_policy":"excluded-v1",
        "forcing_contents_verified":false,
        "results_sha256":results_sha256,
    }))
}

pub(crate) fn comparison_results_sha256(
    study_dir: &Path,
) -> Result<std::collections::BTreeMap<String, String>> {
    fn paths(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                paths(root, &path, files)?;
            } else {
                anyhow::ensure!(kind.is_file(), "Study results contain a non-regular file");
                files.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
        Ok(())
    }
    let root = study_dir.join("results");
    let mut before = Vec::new();
    paths(&root, &root, &mut before)?;
    before.sort();
    let before_stats = before
        .iter()
        .map(|relative| {
            Ok((
                relative.clone(),
                crate::fingerprint::stable_file_identity(&std::fs::metadata(root.join(relative))?),
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let hashes = before
        .iter()
        .map(|relative| {
            let key = relative
                .to_str()
                .context("Study result path is not UTF-8")?
                .replace('\\', "/");
            let file = crate::fingerprint::full_file(&root.join(relative), "result")?;
            Ok((key, file.sha256))
        })
        .collect::<Result<_>>()?;
    let mut after = Vec::new();
    paths(&root, &root, &mut after)?;
    after.sort();
    let after_stats = after
        .iter()
        .map(|relative| {
            Ok((
                relative.clone(),
                crate::fingerprint::stable_file_identity(&std::fs::metadata(root.join(relative))?),
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    anyhow::ensure!(
        before == after && before_stats == after_stats,
        "Study results changed during comparison verification"
    );
    Ok(hashes)
}

/// 两边成绩的判定（目标值越小越好，与率定的目标函数一致）。
pub(crate) fn verdict(left: Side, right: Side, same: bool) -> serde_json::Value {
    let mut notes = Vec::new();
    if !same {
        notes.push("the two Studies differ in parameters, budget, seed, targets or periods; this is not a fair comparison".to_owned());
    }
    if left.version == right.version {
        notes.push("both Studies use the same model version".to_owned());
    }
    let has_validation = left.validation.is_some() && right.validation.is_some();
    if !has_validation {
        notes.push("no validation score on both sides; compared on the calibration period, which favours whichever fits the training data harder".to_owned());
    }
    let (l, r) = if has_validation {
        (left.validation, right.validation)
    } else {
        (left.calibration, right.calibration)
    };
    let better = match (l, r) {
        _ if !same => None,
        (Some(l), Some(r)) if !l.is_finite() || !r.is_finite() => None,
        (Some(l), Some(r)) if (l - r).abs() <= 1e-12 * l.abs().max(r.abs()).max(1.0) => Some("tie"),
        (Some(l), Some(r)) if l < r => Some("a"),
        (Some(_), Some(_)) => Some("b"),
        _ => None,
    };
    if same && better.is_none() {
        notes.push("at least one Study has no best member yet".to_owned());
    }
    serde_json::json!({
        "a": left,
        "b": right,
        "same_design": same,
        "period": if has_validation { "validation" } else { "calibration" },
        "better": better,
        "notes": notes,
    })
}

/// `study-clone <研究>`：按同一份设定（参数、预算、种子、目标、时段，以及建研究时冻结的标准化）在同一个项目里
/// 再建一个研究，给“比较两个版本”的另一边用。返回新研究的目录。
pub(crate) fn clone_study(study_dir: &Path) -> Result<PathBuf> {
    let study_dir = colm_kernel::manifest::absolute(study_dir)?;
    let manifest = study::engine::status(&study_dir)?;
    // 研究在 `<项目>/.colm/studies/<编号>/`。
    let case_root = study_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|_| {
            study_dir
                .parent()
                .is_some_and(|p| p.ends_with(Path::new(".colm").join("studies")))
        })
        .with_context(|| {
            format!(
                "{} is not under <project>/.colm/studies",
                study_dir.display()
            )
        })?
        .to_path_buf();
    let spec_file = std::env::temp_dir().join(format!(
        "colm-study-clone-{}-{}.json",
        std::process::id(),
        crate::provenance::now()
    ));
    std::fs::write(&spec_file, serde_json::to_string_pretty(&manifest.spec)?)?;
    let model_pin = pinned(&study_dir)?.filter(|p| p.model_revision.is_some());
    let created = (|| {
        if model_pin.is_some() {
            study::engine::verify_frozen_inputs(&manifest)?;
            study::model::verified_inputs(&manifest)?;
            let first = manifest
                .spec
                .base_cases
                .first()
                .context("Study has no base cases")?;
            let baseline = study::model::base_case(&manifest, first)?;
            study::engine::create_from_baselines(
                &case_root,
                &spec_file,
                baseline.parent().context("baseline directory")?,
            )
        } else {
            study::engine::create(&case_root, &spec_file)
        }
    })();
    let _ = std::fs::remove_file(&spec_file);
    let created = PathBuf::from(created?.root);
    let finalization = (|| -> Result<()> {
        if let Some(mut pin) = model_pin {
            let source = study_dir.join("version");
            let target = created.join("version");
            if source.is_dir() {
                copy_dir(&source, &target)?;
            }
            if let Ok(relative) = pin.kernel.strip_prefix(&source) {
                pin.kernel = target.join(relative);
            }
            if let Ok(relative) = pin.cli.strip_prefix(&source) {
                pin.cli = target.join(relative);
            }
            std::fs::write(created.join(FILE), serde_json::to_vec_pretty(&pin)?)?;
        }
        study::model::capture_inputs(&study::engine::status(&created)?)?;
        Ok(())
    })();
    if let Err(error) = finalization {
        let _ = std::fs::remove_dir_all(&created);
        return Err(error);
    }
    Ok(created)
}

#[cfg(test)]
#[path = "study_version_tests.rs"]
mod study_version_tests;

/// 冻结的模型修订用哪个 `colm-cli` 跑研究（修改版用它工作区里编出的，否则用该修订源码缓存的官方程序），
/// 以及它改动了哪些部分。
pub(crate) fn revision_cli(
    opts: &crate::Opts,
    revision: &colm_model::FrozenRevision,
) -> Result<(PathBuf, colm_workspace::version::Touches)> {
    let cached = crate::model_version::cached_options(opts, &revision.code)?;
    if let Some(name) = revision.code.workspace.as_deref() {
        let flags = cached.as_ref().unwrap_or(opts);
        let workspace = colm_workspace::Workspace::open(
            &crate::provenance::workspace_root(flags.get("--workspace-root")),
            name,
        )?;
        let changed = workspace.changes_from_official()?;
        Ok((
            workspace.bin().join(exe("colm-cli")),
            colm_workspace::version::touches(changed.iter().map(|c| c.path.as_str())),
        ))
    } else {
        Ok((
            crate::model_version::cached_official_cli(opts, &revision.code)?,
            colm_workspace::version::Touches {
                engine: false,
                kernel: false,
            },
        ))
    }
}

/// 研究交给冻结修订自己的 `colm-cli` 跑，而研究设定是 `deny_unknown_fields`：代码早于某个新设定的修订
/// 读不了它，研究要到运行时才报 "unknown field"（2026-10-23 实测：10-22 的程序拒绝 `include_baseline`、
/// `corrected`、`morris`）。建研究时先让那个程序预检一遍准备好的设定：
/// - 只是不认得 `include_baseline`（GUI 自动加的增强，不改研究的含义）：去掉它再试；
/// - 不认得别的设定（Morris、订正观测、不确定性分析的目标……）：拒绝建研究并说明原因；
/// - 旧程序读得懂、却按旧规则拒绝（如旧程序不许不确定性分析带对照观测），而当前程序的预检通过：
///   同样拒绝。原来只认 unknown field，这种拒绝被放过，旧程序运行时静默丢掉目标（复验 B2-04）；
/// - 两边都拒绝的是普通预检错误，不在这里判，照旧交给当前程序的预检报。
pub(crate) fn ensure_revision_reads_spec(
    cli: &Path,
    case_root: &Path,
    spec_file: &Path,
    revision: &colm_model::FrozenRevision,
) -> Result<()> {
    ensure_revision_reads_spec_with(cli, case_root, spec_file, revision, || {
        study::runner::preflight_create(case_root, spec_file).is_ok()
    })
}

fn ensure_revision_reads_spec_with(
    cli: &Path,
    case_root: &Path,
    spec_file: &Path,
    revision: &colm_model::FrozenRevision,
    current_accepts: impl Fn() -> bool,
) -> Result<()> {
    let probe = || -> Result<Option<String>> {
        let output = std::process::Command::new(cli)
            .arg("study-preflight")
            .arg(case_root)
            .arg("--spec")
            .arg(spec_file)
            .env(DELEGATED_ENV, "1")
            .output()
            .with_context(|| format!("cannot start {}", cli.display()))?;
        if output.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&output.stderr).into_owned();
        let unknown = text.contains("unknown field") || text.contains("unknown variant");
        Ok((unknown || current_accepts()).then_some(text))
    };
    let Some(error) = probe()? else {
        return Ok(());
    };
    if error.contains("unknown field `include_baseline`") {
        let mut spec = study::spec::read_spec(spec_file)?;
        spec.budget.include_baseline = false;
        std::fs::write(spec_file, serde_json::to_vec_pretty(&spec)?)?;
        let Some(error) = probe()? else {
            return Ok(());
        };
        return Err(unreadable(revision, &error));
    }
    Err(unreadable(revision, &error))
}

fn unreadable(revision: &colm_model::FrozenRevision, error: &str) -> anyhow::Error {
    let detail = error
        .lines()
        .find(|line| line.contains("unknown field") || line.contains("unknown variant"))
        .or_else(|| error.lines().rev().find(|line| !line.trim().is_empty()))
        .unwrap_or(error)
        .trim();
    anyhow::anyhow!(
        "model version {} r{} was built from older code that cannot read this study design ({detail}); create the study with the official version, or rebuild this version on the current code",
        revision.version.id,
        revision.version.revision
    )
}

/// 研究第一次运行用的引擎与前处理。和模型版本一样记在单独的文件里而不是研究设定里：设定交给冻结修订的
/// 旧 `colm-cli` 去读，新字段会被 `deny_unknown_fields` 拒绝（见 [`ensure_revision_reads_spec`]）。
pub(crate) const RUN_OPTIONS_FILE: &str = "run-options.json";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct RunOptions {
    pub engine: String,
    pub preprocessors: String,
}

pub(crate) fn run_options(study_dir: &Path) -> Result<Option<RunOptions>> {
    let path = study_dir.join(RUN_OPTIONS_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text)?))
}

/// 这次运行该用的引擎与前处理。原来续跑用的是界面当时的选择（缺省 Rust）：用 Fortran 跑了一半的研究，
/// 打开后点"恢复"会把已完成的成员判成过期、换 Rust 整批重跑，一个研究里混着两种引擎的结果（GUI 审查 R8）。
/// 没记过就按这次的请求（不给就是 Rust）记下；记过的沿用，明确要另一种就拒绝。
pub(crate) fn frozen_run_options(
    study_dir: &Path,
    engine: Option<&str>,
    preprocessors: Option<&str>,
) -> Result<RunOptions> {
    let recorded = run_options(study_dir)?;
    let pick = |label: &str,
                requested: Option<&str>,
                recorded: Option<&String>|
     -> Result<String> {
        match (requested, recorded) {
            (Some(requested), Some(recorded)) if requested != recorded => bail!(
                "this Study runs with the {recorded} {label}; it cannot switch to {requested}. Create a new Study for another {label}"
            ),
            (Some(requested), _) => Ok(requested.to_string()),
            (None, Some(recorded)) => Ok(recorded.clone()),
            (None, None) => Ok("rust".into()),
        }
    };
    let options = RunOptions {
        engine: pick("engine", engine, recorded.as_ref().map(|r| &r.engine))?,
        preprocessors: pick(
            "preprocessors",
            preprocessors,
            recorded.as_ref().map(|r| &r.preprocessors),
        )?,
    };
    if recorded.is_none() {
        std::fs::write(
            study_dir.join(RUN_OPTIONS_FILE),
            serde_json::to_vec_pretty(&options)?,
        )?;
    }
    Ok(options)
}

pub(crate) fn pin_revision(
    study_dir: &Path,
    revision: &colm_model::FrozenRevision,
    model_root: &Path,
    opts: &crate::Opts,
) -> Result<Pinned> {
    crate::model_version::verify_code(opts, &revision.code)?;
    crate::model_version::cache_code(opts, &revision.code)?;
    let manifest = study::engine::status(study_dir)?;
    let kernel = opts
        .get("--kernel")
        .or(manifest.spec.kernel_dir)
        .map(PathBuf::from)
        .unwrap_or_default();
    let kernel = if kernel.is_dir() {
        let target = study_dir.join("version/kernel");
        copy_dir(&kernel, &target)?;
        target
    } else {
        kernel
    };
    let (cli, touches) = revision_cli(opts, revision)?;
    let pinned = Pinned {
        name: revision.version.id.clone(),
        head: revision
            .code
            .commit
            .clone()
            .unwrap_or_else(|| revision.code.source_sha256.clone()),
        base_commit: revision.code.commit.clone().unwrap_or_default(),
        note: revision.note.clone(),
        based_on: revision.parent.as_ref().map(|p| p.id.clone()),
        touches,
        cli,
        kernel,
        pinned_unix: crate::provenance::now() as i64,
        model_revision: Some(revision.clone()),
        model_root: Some(model_root.to_owned()),
    };
    std::fs::write(study_dir.join(FILE), serde_json::to_vec_pretty(&pinned)?)?;
    Ok(pinned)
}

pub(crate) fn validate_revision_request(study: &Path, opts: &crate::Opts) -> Result<()> {
    if let Some(requested) = crate::model_version::selected(opts)? {
        let pinned = pinned(study)?.and_then(|p| p.model_revision).context(
            "this Study has no frozen model revision; create a Study with the selected revision",
        )?;
        anyhow::ensure!(
            pinned.version == requested,
            "Study is locked to another model revision"
        );
    }
    Ok(())
}
fn revision_options(pinned: &Pinned, opts: &crate::Opts) -> Result<crate::Opts> {
    let mut flags = opts
        .flags
        .iter()
        .filter(|(k, _)| k != "--model-root")
        .cloned()
        .collect::<Vec<_>>();
    flags.push((
        "--model-root".into(),
        pinned
            .model_root
            .as_ref()
            .context("pinned revision has no registry")?
            .to_string_lossy()
            .into_owned(),
    ));
    Ok(crate::Opts {
        flags,
        positional: opts.positional.clone(),
    })
}
pub(crate) fn validate_pinned(
    pinned: &mut Pinned,
    opts: &crate::Opts,
    kernel: &Path,
    engine: &str,
    preprocessors: &str,
) -> Result<()> {
    let Some(revision) = &pinned.model_revision else {
        return Ok(());
    };
    let options = revision_options(pinned, opts)?;
    crate::model_version::verify_code(&options, &revision.code)?;
    anyhow::ensure!(
        revision.overlay.hybrid.is_none() || engine == "rust",
        "hybrid model revisions require Rust"
    );
    let kernel = if pinned.kernel.as_os_str().is_empty() {
        kernel
    } else {
        &pinned.kernel
    };
    crate::model_version::check_kernel(revision, &colm_kernel::Kernel::open_manifest(kernel)?)?;
    let used = if let Some(name) = &revision.code.workspace {
        let cached = crate::model_version::cached_options(&options, &revision.code)?
            .context("missing frozen workspace")?;
        crate::provenance::resolve(
            &crate::provenance::workspace_root(cached.get("--workspace-root")),
            name,
            engine,
            preprocessors,
            kernel,
        )?
        .kernel
    } else {
        kernel.to_owned()
    };
    pinned.kernel = used;
    Ok(())
}
