//! 远程运行（docs/design-ai-assistant.md 第 5.3 节，R1）：把一个算例放到服务器上用 Rust 引擎跑，再把结果取回来。
//!
//! ```text
//! colm-cli remote-probe  --host H --root R
//! colm-cli remote-run    <case> --host H --root R --kernel <本机内核目录> [--map 本机前缀=服务器前缀]…
//!                        [--stage S] [--force 1] [--threads N] [--upload-unmapped 1]
//!                        [--scheduler auto|bare|slurm|pbs|lsf] [--partition P] [--account A] [--walltime T]
//!                        [--cpus N] [--mem-gb N] [--env-script TEXT] [--directive '-x…']… [--dry-run 1]
//!                        [--engine rust|fortran] [--remote-kernel auto|名字|路径] [--preprocessors rust|fortran]
//!                        [--ranks N] [--nodes N] [--launcher auto|srun|mpiexec]
//! colm-cli remote-kernel --host H --root R --preset P [--env-script TEXT] [--profile production|debug] [--force 1]
//!                        [调度选项同 remote-run]    在服务器上编 Fortran 内核（R4），登记在 <根>/kernels/<预设>-<标识>
//! colm-cli remote-kernels --host H --root R         列出服务器上的内核
//! colm-cli remote-dist  --host H --root R [--targets x86_64,aarch64] [--out 目录]
//! colm-cli remote-fetch  <case> [--vars a,b] [--from YYYY-MM] [--to YYYY-MM] [--compress N]
//!                        只取所选变量与月份（先在服务器上裁剪压缩，R5）；不给就取全部
//! colm-cli remote-status <case> [--lines N]
//! colm-cli remote-cancel <case>
//! ```
//!
//! `--dry-run 1` 只生成作业脚本全文（含调度指令）打印出来，不上传、不提交；`--scheduler auto` 先探测服务器有什么
//! 调度系统。
//!
//! 服务器上的布局（都在工作根目录 R 下）：`engine/<快照>/`（源码与编好的四个程序）、`target/`（cargo 编译目录）、
//! `kernels/<清单哈希>/manifest.json`（只放清单，Rust 引擎只用它的宏）、`cases/<名字>-<本机路径哈希>/`
//! （算例副本，输出在其 `out/`）、`jobs/<作业号>/`（作业脚本、日志、阶段、退出码）。
//!
//! 算例副本里所有路径字段都改写成服务器路径：算例目录内的换成副本目录，其余按 `--map` 的前缀对应；对应不上
//! 的默认报错列出来（大数据不悄悄上传），`--upload-unmapped 1` 时把其中的文件随算例一起传。提交记录写在本机
//! 算例的 `.colm-remote.json`，关掉应用也能接着查状态与取结果。取回时只取 `out/<名字>/history` 与日志，不取
//! `stages.json`：它记着服务器上的路径与程序哈希，本机那份保持不变（本机再跑时自己判断要不要重跑）。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::Value;
use colm_remote::engine::{self, Prebuilt, Snapshot, Source};
use colm_remote::job::{self, Spec};
use colm_remote::sched::{self, Resources, Scheduler};
use colm_remote::ssh::{quote, Ssh};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Opts;

/// 本机路径前缀与服务器路径前缀的一条对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mapping {
    local: String,
    remote: String,
}

fn trim_slash(path: &str) -> &str {
    if path.len() > 1 {
        path.trim_end_matches(['/', '\\'])
    } else {
        path
    }
}

pub(crate) fn parse_mapping(text: &str) -> Result<Mapping> {
    let (local, remote) = text
        .split_once('=')
        .with_context(|| format!("--map needs LOCAL=REMOTE, got {text}"))?;
    ensure!(
        Path::new(local).is_absolute() && remote.starts_with('/'),
        "--map needs two absolute paths, got {text}"
    );
    Ok(Mapping {
        local: trim_slash(local).to_owned(),
        remote: trim_slash(remote).to_owned(),
    })
}

/// 一个路径字段在服务器上的去向。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Placed {
    /// 改写成这个服务器路径。
    Remote(String),
    /// 对应不上的本机路径（存在）。
    Unmapped(PathBuf),
    /// 本机也不存在、又对应不上：保持原样（例如可选输入的占位）。
    Keep,
}

/// `path` 在 `prefix` 之下时返回剩下的部分（按路径分量比，`/a/bc` 不算在 `/a/b` 之下）。
fn under<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with(['/', '\\'])).then_some(rest)
}

/// 决定一个路径字段的值放到服务器的哪里。目录值的结尾分隔符保留（CoLM 直接拼接，少了就错）。
pub(crate) fn place(raw: &str, case: &Path, remote_case: &str, maps: &[Mapping]) -> Placed {
    let trailing = raw.ends_with(['/', '\\']);
    let absolute = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        case.join(raw)
    };
    let local = absolute.to_string_lossy().replace('\\', "/");
    let local = trim_slash(&local).to_owned();
    let case_text = case.to_string_lossy().replace('\\', "/");
    let finish = |remote: String| {
        let remote = remote.replace('\\', "/");
        Placed::Remote(if trailing && !remote.ends_with('/') {
            format!("{remote}/")
        } else {
            remote
        })
    };
    if let Some(rest) = under(&local, trim_slash(&case_text)) {
        return finish(format!("{}{rest}", trim_slash(remote_case)));
    }
    // 最长的前缀优先（嵌套的对应里更具体的那条）。
    let mut maps: Vec<&Mapping> = maps.iter().collect();
    maps.sort_by_key(|m| std::cmp::Reverse(m.local.len()));
    for map in maps {
        if let Some(rest) = under(&local, &map.local) {
            return finish(format!("{}{rest}", map.remote));
        }
    }
    if absolute.exists() {
        Placed::Unmapped(absolute)
    } else {
        Placed::Keep
    }
}

/// 不随算例上传的东西：输出、日志、阶段指纹、结果标记、远程记录。
fn skipped(name: &str) -> bool {
    name == "out"
        || name.ends_with(".log")
        || name == "stages.json"
        || name == ".colm-results-stale"
        || name == RECORD
        || name.starts_with(".DS_Store")
}

fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    let mut bytes = 0;
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            bytes += copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        bytes +=
            std::fs::copy(from, to).with_context(|| format!("cannot copy {}", from.display()))?;
    }
    Ok(bytes)
}

/// 准备好的算例副本（本机暂存目录）与改写时发现的问题。
#[derive(Debug, Default)]
pub(crate) struct Staged {
    pub bytes: u64,
    /// 改写后指向服务器数据的路径（提交前核对它们在服务器上存在）。
    pub remote_inputs: Vec<String>,
    pub unmapped: Vec<PathBuf>,
    pub uploaded_extra: Vec<PathBuf>,
}

fn looks_like_path(field: &str, raw: &str) -> bool {
    let lower = field.to_ascii_lowercase();
    lower.contains("file")
        || lower.contains("namelist")
        || lower.starts_with("def_dir")
        || lower.contains("path")
        || lower.ends_with("_data")
        || lower == "site_fsitedata"
        || raw.starts_with('/')
}

/// 改写一个 namelist 文件里的全部路径字段。
fn rewrite_namelist(
    file: &Path,
    case: &Path,
    remote_case: &str,
    maps: &[Mapping],
    upload_unmapped: bool,
    staging: &Path,
    staged: &mut Staged,
) -> Result<()> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut document = colm_namelist::parse(&text)?;
    let paths: Vec<String> = document.paths();
    for field in paths {
        let Some(Value::Str(raw)) = document.get(&field).cloned() else {
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("null")
            || !looks_like_path(&field, trimmed)
        {
            continue;
        }
        if field.eq_ignore_ascii_case("DEF_dir_output") {
            document.set(
                &field,
                Value::Str(format!("{}/out/", trim_slash(remote_case))),
            )?;
            continue;
        }
        if field.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES") {
            let rewritten = colm_namelist::tracer_files::rewrite_param_files(trimmed, |path| {
                match place(path, case, remote_case, maps) {
                    Placed::Remote(remote) => Ok(remote),
                    _ => bail!("process parameter file {path} is outside the case and not mapped"),
                }
            })?;
            document.set(&field, Value::Str(rewritten))?;
            continue;
        }
        // 只改看得出是路径的值（绝对路径或带分隔符），其余（例如文件名前缀）原样。
        if !trimmed.contains(['/', '\\']) {
            continue;
        }
        match place(trimmed, case, remote_case, maps) {
            Placed::Remote(remote) => {
                if !remote.starts_with(trim_slash(remote_case)) {
                    staged.remote_inputs.push(remote.clone());
                }
                document.set(&field, Value::Str(remote))?;
            }
            Placed::Unmapped(local) if upload_unmapped && local.is_file() => {
                let name = local
                    .file_name()
                    .context("input file has no name")?
                    .to_string_lossy()
                    .into_owned();
                let extra = staging.join("inputs");
                std::fs::create_dir_all(&extra)?;
                staged.bytes += std::fs::copy(&local, extra.join(&name))
                    .with_context(|| format!("cannot copy {}", local.display()))?;
                document.set(
                    &field,
                    Value::Str(format!("{}/inputs/{name}", trim_slash(remote_case))),
                )?;
                staged.uploaded_extra.push(local);
            }
            Placed::Unmapped(local) => staged.unmapped.push(local),
            Placed::Keep => {}
        }
    }
    std::fs::write(file, document.to_string())
        .with_context(|| format!("cannot write {}", file.display()))
}

/// 在本机暂存目录里准备服务器用的算例副本。
pub(crate) fn stage_case(
    case: &Path,
    staging: &Path,
    remote_case: &str,
    maps: &[Mapping],
    upload_unmapped: bool,
) -> Result<Staged> {
    let mut staged = Staged::default();
    std::fs::create_dir_all(staging)?;
    for entry in
        std::fs::read_dir(case).with_context(|| format!("cannot read {}", case.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if skipped(&name) {
            continue;
        }
        staged.bytes += copy_tree(&entry.path(), &staging.join(&name))?;
    }
    ensure!(
        staging.join("case.nml").is_file(),
        "{} has no case.nml",
        case.display()
    );
    let mut namelists: Vec<PathBuf> = std::fs::read_dir(staging)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nml")))
        .collect();
    namelists.sort();
    for file in namelists {
        rewrite_namelist(
            &file,
            case,
            remote_case,
            maps,
            upload_unmapped,
            staging,
            &mut staged,
        )?;
    }
    staged.remote_inputs.sort();
    staged.remote_inputs.dedup();
    Ok(staged)
}

/// 本机算例的提交记录。
pub(crate) const RECORD: &str = ".colm-remote.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub host: String,
    pub root: String,
    pub job: String,
    pub remote_case: String,
    pub case_name: String,
    pub engine: String,
    pub submitted_at: u64,
    /// bare、slurm、pbs 或 lsf；老记录没有这一项，当作 bare。
    #[serde(default = "bare")]
    pub scheduler: String,
    /// 调度系统给的作业号（bare 没有）。
    #[serde(default)]
    pub scheduler_id: Option<String>,
}

fn bare() -> String {
    "bare".into()
}

fn read_record(case: &Path) -> Result<Record> {
    let path = case.join(RECORD);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} has not been run remotely (no {RECORD})", case.display()))?;
    Ok(serde_json::from_str(&text)?)
}

fn short_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..8].to_owned()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn root_of(opts: &Opts) -> Result<String> {
    let root = opts.need_str("--root")?;
    ensure!(
        root.starts_with('/') && !root.contains(char::is_whitespace) && root != "/",
        "--root must be an absolute directory on the server without spaces, got {root}"
    );
    Ok(trim_slash(&root).to_owned())
}

/// 引擎源码：开发环境里是这个仓库，安装包里是随应用附带的 `colm-src.tar.gz`。
fn engine_source() -> Result<Source> {
    let dir = exe_dir()?;
    if let Some(source) = Source::find_checkout(&dir) {
        return Ok(source);
    }
    if let Some(tarball) = engine::bundled_source(&engine::resource_dirs(&dir)) {
        return Ok(Source::Tarball(tarball));
    }
    bail!("the engine sources were not found next to colm-cli; this build cannot run remotely")
}

/// Workspace creation needs the original package, not the extracted read-only source cache.
pub(crate) fn workspace_source() -> Result<PathBuf> {
    Ok(match engine_source()? {
        Source::Checkout(path) | Source::Tarball(path) => path,
    })
}

/// Source identity excludes Git metadata, bulk datasets and generated build/cache directories.
/// The same relative-path/byte stream is used for checkouts and extracted packages.
pub(crate) fn source_content_sha256(root: &Path) -> Result<String> {
    fn collect(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        let path = root.join(relative);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            !meta.file_type().is_symlink(),
            "source symlink is not supported: {}",
            path.display()
        );
        if meta.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                let name = entry.file_name();
                if matches!(
                    name.to_str(),
                    Some("target" | ".git" | "node_modules" | "__pycache__" | "cache" | ".cache")
                ) {
                    continue;
                }
                collect(root, &relative.join(name), files)?;
            }
        } else if meta.is_file() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(name, "Cargo.lock" | "Makefile" | "CMakeLists.txt")
                || matches!(
                    ext.as_str(),
                    "rs" | "f"
                        | "f90"
                        | "f95"
                        | "for"
                        | "c"
                        | "h"
                        | "cpp"
                        | "hpp"
                        | "py"
                        | "sh"
                        | "ps1"
                        | "js"
                        | "mjs"
                        | "ts"
                        | "json"
                        | "toml"
                        | "mk"
                        | "cmake"
                )
            {
                files.push(relative.to_owned());
            }
        }
        Ok(())
    }
    ensure!(
        !std::fs::symlink_metadata(root)?.file_type().is_symlink(),
        "source root cannot be a symlink"
    );
    ensure!(
        root.join("Cargo.toml").is_file(),
        "source has no Cargo.toml"
    );
    let mut files = Vec::new();
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "crates",
        "vendor",
        "xtask",
        "scripts",
    ] {
        collect(root, Path::new(path), &mut files)?;
    }
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let bytes = std::fs::read(root.join(&file))?;
        let normalized = file
            .components()
            .map(|part| {
                part.as_os_str()
                    .to_str()
                    .context("source path is not UTF-8")
            })
            .collect::<Result<Vec<_>>>()?
            .join("/");
        hasher.update(normalized.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn unpack_app_source(tarball: &Path, cache: &Path) -> Result<PathBuf> {
    const MARKER: &str = ".colm-source-sha256";
    let digest = file_sha256(tarball)?;
    let dest = cache.join(&digest);
    let complete = |path: &Path| {
        path.join("Cargo.lock").is_file()
            && path.join("Cargo.toml").is_file()
            && std::fs::read_to_string(path.join(MARKER)).is_ok_and(|value| value == digest)
    };
    if complete(&dest) {
        return Ok(dest);
    }
    if dest.exists() {
        ensure!(
            complete(&dest),
            "incomplete source cache {}; preserve it for inspection",
            dest.display()
        );
        return Ok(dest);
    }
    std::fs::create_dir_all(cache)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let staging = cache.join(format!(".{digest}.{}.{nonce}", std::process::id()));
    std::fs::create_dir(&staging)?;
    let result = (|| {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(tarball)
            .arg("-C")
            .arg(&staging)
            .status()
            .context("cannot run tar")?;
        ensure!(status.success(), "cannot unpack {}", tarball.display());
        ensure!(
            staging.join("Cargo.lock").is_file() && staging.join("Cargo.toml").is_file(),
            "source package is incomplete"
        );
        ensure!(
            file_sha256(tarball)? == digest,
            "source package changed during extraction"
        );
        std::fs::write(staging.join(MARKER), &digest)?;
        match std::fs::rename(&staging, &dest) {
            Ok(()) => Ok(dest.clone()),
            Err(_) if complete(&dest) => Ok(dest.clone()),
            Err(error) => Err(error.into()),
        }
    })();
    // Only this invocation's private staging directory may be removed.
    if staging.exists() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

/// Installed sources are cached by the full package digest, published only after extraction.
pub(crate) fn app_source_dir() -> Result<PathBuf> {
    match engine_source()? {
        Source::Checkout(repo) => Ok(repo),
        Source::Tarball(tarball) => {
            unpack_app_source(&tarball, &engine::cache_dir().join("source"))
        }
    }
}

/// 这次用哪个引擎：服务器上从源码编，或者传一个预编包。
enum Engine {
    Source(Snapshot),
    Prebuilt(Prebuilt),
}

impl Engine {
    fn id(&self) -> &str {
        match self {
            Self::Source(snapshot) => &snapshot.id,
            Self::Prebuilt(prebuilt) => &prebuilt.id,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Source(_) => "source",
            Self::Prebuilt(_) => "prebuilt",
        }
    }

    /// 传上去（已经在就跳过）；返回这次是不是真的传了。
    fn upload(&self, ssh: &Ssh, root: &str) -> Result<bool> {
        match self {
            Self::Source(snapshot) => engine::upload(ssh, root, snapshot),
            Self::Prebuilt(prebuilt) => engine::upload_prebuilt(ssh, root, prebuilt),
        }
    }
}

fn exe_dir() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .context("colm-cli has no directory")?
        .to_path_buf())
}

/// 选引擎。`--engine-mode`：
/// - `source`：把源码传上去，在服务器上编（要 cargo、cmake 与 C 编译器）；
/// - `prebuilt`：传预编包（`--prebuilt 路径`，或应用随附的、缓存里与当前源码一致的那份）；
/// - `auto`（默认）：服务器能编就从源码编，不能（没有 cargo 等）就用预编包，两样都没有就说明怎么办。
fn choose_engine(opts: &Opts, ssh: &Ssh, root: &str) -> Result<Engine> {
    let mode = opts.get("--engine-mode").unwrap_or_else(|| "auto".into());
    ensure!(
        ["auto", "source", "prebuilt"].contains(&mode.as_str()),
        "--engine-mode must be auto, source or prebuilt"
    );
    let source = engine_source().and_then(|s| engine::snapshot(&s));
    if mode == "source" {
        return Ok(Engine::Source(source?));
    }
    let probe = colm_remote::probe::probe(ssh, root)?;
    let can_build = probe.cargo.is_some() && probe.cmake && probe.cc;
    if mode == "auto" && can_build {
        return Ok(Engine::Source(source?));
    }
    let prebuilt = match opts.get("--prebuilt") {
        Some(path) => Some(Prebuilt::open(Path::new(&path))?),
        None => engine::find_prebuilt(
            &probe.arch,
            source.as_ref().ok().map(|s| s.id.as_str()),
            &engine::resource_dirs(&exe_dir()?),
        ),
    };
    match prebuilt {
        Some(prebuilt) => Ok(Engine::Prebuilt(prebuilt)),
        None => bail!(
            "{} cannot build the engine (cargo, cmake or a C compiler is missing) and no prebuilt engine for {} was found; \
             build one with `colm-cli remote-dist --host <a machine with internet> --root <dir>` or pass --prebuilt <colm-engine-linux-{}.tar.gz>",
            ssh.host,
            probe.arch,
            probe.arch
        ),
    }
}

/// 这次运行的引擎与 MPI 设置。
struct RunPlan {
    /// rust 或 fortran。
    engine: String,
    /// 前处理用谁：rust 或 fortran（真正的 Fortran 对照要选 fortran）。
    preprocessors: String,
    ranks: u32,
    nodes: u32,
    /// auto、srun 或 mpiexec。
    launcher: String,
}

impl RunPlan {
    fn from_opts(opts: &Opts) -> Result<Self> {
        let engine = opts.get("--engine").unwrap_or_else(|| "rust".into());
        ensure!(
            ["rust", "fortran"].contains(&engine.as_str()),
            "--engine must be rust or fortran"
        );
        let preprocessors = opts.get("--preprocessors").unwrap_or_else(|| "rust".into());
        ensure!(
            ["rust", "fortran"].contains(&preprocessors.as_str()),
            "--preprocessors must be rust or fortran"
        );
        let ranks: u32 = opts
            .get("--ranks")
            .map(|v| v.parse())
            .transpose()
            .context("--ranks must be a whole number")?
            .unwrap_or(1);
        ensure!(ranks >= 1, "--ranks must be at least 1");
        ensure!(
            engine == "fortran" || ranks == 1,
            "--ranks only applies to --engine fortran (the Rust engine uses threads)"
        );
        let nodes: u32 = opts
            .get("--nodes")
            .map(|v| v.parse())
            .transpose()
            .context("--nodes must be a whole number")?
            .unwrap_or(0);
        let launcher = opts.get("--launcher").unwrap_or_else(|| "auto".into());
        ensure!(
            ["auto", "srun", "mpiexec"].contains(&launcher.as_str()),
            "--launcher must be auto, srun or mpiexec"
        );
        Ok(Self {
            engine,
            preprocessors,
            ranks,
            nodes,
            launcher,
        })
    }
}

fn parse_scheduler(opts: &Opts, ssh: &Ssh, root: &str) -> Result<Scheduler> {
    Ok(match opts.get("--scheduler").as_deref() {
        // auto：问服务器有什么调度系统，取第一个；没有就直接后台运行。
        Some("auto") => colm_remote::probe::probe(ssh, root)?
            .schedulers
            .first()
            .map(|s| Scheduler::parse(s))
            .transpose()?
            .unwrap_or(Scheduler::Bare),
        Some(name) => Scheduler::parse(name)?,
        None => Scheduler::Bare,
    })
}

fn resources_from(opts: &Opts, threads: u32, ranks: u32, nodes: u32) -> Result<Resources> {
    let resources = Resources {
        cpus: opts
            .get("--cpus")
            .map(|v| v.parse())
            .transpose()
            .context("--cpus must be a whole number")?
            .unwrap_or(threads),
        ranks,
        nodes,
        memory_gb: opts
            .get("--mem-gb")
            .map(|v| v.parse())
            .transpose()
            .context("--mem-gb must be a whole number")?,
        walltime: opts.get("--walltime"),
        partition: opts.get("--partition"),
        account: opts.get("--account"),
        env_script: opts.get("--env-script").filter(|t| !t.trim().is_empty()),
        directives: opts.get_all("--directive"),
    };
    resources.validate()?;
    Ok(resources)
}

/// 等一个作业结束，进度打到 stderr；失败时带上日志末尾。
fn wait_job(ssh: &Ssh, root: &str, id: &str, what: &str, timeout_min: u64) -> Result<()> {
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(20));
        let status = job::status(ssh, root, id, 3)?;
        let last = status.log_tail.lines().last().unwrap_or("").trim();
        eprintln!("  {:?} {}", status.state, last);
        match status.state {
            job::State::Finished { exit_code: 0 } => return Ok(()),
            job::State::Finished { exit_code } => {
                let tail = job::status(ssh, root, id, 40)?.log_tail;
                bail!(
                    "{what} failed (exit code {exit_code}); job {id} on {}:\n{tail}",
                    ssh.host
                )
            }
            job::State::Lost | job::State::Unknown => {
                bail!("the {what} job {id} disappeared on {}", ssh.host)
            }
            _ => {}
        }
        ensure!(
            started.elapsed().as_secs() < timeout_min * 60,
            "{what} is still running after {timeout_min} minutes; job {id} on {} keeps going — check it with ssh",
            ssh.host
        );
    }
}

/// `colm-cli remote-kernels`：服务器上有哪些内核。
pub(super) fn cmd_kernels(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    println!(
        "{}",
        serde_json::to_string(&colm_remote::kernel::list(&ssh, &root)?)?
    );
    Ok(())
}

/// Fortran 源的提交号（本机算，带到服务器上，因为服务器上的快照没有 .git）。
fn fortran_source_sha() -> String {
    Source::find_checkout(&exe_dir().unwrap_or_default())
        .and_then(|source| match source {
            Source::Checkout(repo) => Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["log", "-1", "--format=%h", "--", "vendor/CoLM202X"])
                .output()
                .ok(),
            Source::Tarball(_) => None,
        })
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "bundled".into())
}

/// `colm-cli remote-kernel`：在服务器上从 `vendor/CoLM202X` 编一个 Fortran 内核（要 gfortran、`mpif90`、netCDF-Fortran，
/// 用 `--env-script` 载入，例如 `module load …`）。目录名带源码、环境和预设的哈希，编过的不再重编（`--force 1` 重编）。
pub(super) fn cmd_kernel(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let preset = opts.need_str("--preset")?;
    ensure!(
        !preset.is_empty()
            && preset
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "--preset may only contain letters, digits, - and _"
    );
    let profile = opts.get("--profile").unwrap_or_else(|| "production".into());
    ensure!(
        ["production", "debug"].contains(&profile.as_str()),
        "--profile must be production or debug"
    );
    let force = opts.get("--force").is_some_and(|v| v == "1");
    let threads: u32 = opts
        .get("--threads")
        .map(|t| t.parse())
        .transpose()?
        .unwrap_or(16);
    let scheduler = parse_scheduler(opts, &ssh, &root)?;
    let resources = resources_from(opts, threads, 1, 0)?;
    let snapshot = engine::snapshot(&engine_source()?)?;
    let name = format!(
        "{preset}-{}",
        short_hash(&format!(
            "{}|{}|{profile}|{}",
            snapshot.id,
            resources.env_script.as_deref().unwrap_or(""),
            fortran_source_sha()
        ))
    );
    let kdir = format!("{root}/kernels/{name}");
    let have = colm_remote::kernel::list(&ssh, &root)?
        .into_iter()
        .any(|k| k.name == name && k.full);
    if have && !force {
        println!(
            "{}",
            json!({ "kernel": kdir, "name": name, "built": false })
        );
        return Ok(());
    }
    let uploaded = engine::upload(&ssh, &root, &snapshot)?;
    let src = quote(&format!("{}/src", engine::engine_dir(&root, &snapshot.id)));
    let tmp = quote(&format!("{root}/kernels/.build-{name}"));
    let body = format!(
        "echo \"building the Fortran kernel {preset}\" > phase\n\
         export COLM_GIT_SHA={sha} COLM_KERNEL_PROFILE={profile}\n\
         rm -rf {tmp}\n\
         bash {src}/oracle/scripts/build_kernel.sh {preset} {tmp}\n\
         rm -rf {kdir_q}\n\
         mv {tmp}/{preset} {kdir_q}\n\
         rm -rf {tmp}\n\
         echo \"kernel ready\" >> phase\n",
        sha = quote(&fortran_source_sha()),
        kdir_q = quote(&kdir),
    );
    let id = format!("kernel-{}", now());
    let spec = Spec {
        scheduler,
        resources,
        name: sched::job_name(&format!("k{preset}")),
    };
    job::submit(&ssh, &root, &id, &body, &spec)?;
    eprintln!(
        "building the Fortran kernel {preset} on {} as job {id}{}",
        ssh.host,
        if uploaded { " (sources uploaded)" } else { "" }
    );
    wait_job(&ssh, &root, &id, "the kernel build", 180)?;
    let built = colm_remote::kernel::list(&ssh, &root)?
        .into_iter()
        .find(|k| k.name == name)
        .context("the build finished but the kernel is not on the server")?;
    ensure!(
        built.full,
        "the kernel was registered without all three Fortran programs"
    );
    println!(
        "{}",
        json!({ "kernel": kdir, "name": name, "built": true, "manifest": built })
    );
    Ok(())
}

/// `--engine fortran` 要的服务器上的内核：`--remote-kernel auto`（默认）取与本机内核同一预设、最新的完整内核；
/// 也可以给 `<根>/kernels/` 下的名字或绝对路径。
fn resolve_remote_kernel(opts: &Opts, ssh: &Ssh, root: &str, preset: &str) -> Result<String> {
    let kernels = colm_remote::kernel::list(ssh, root)?;
    let wanted = opts.get("--remote-kernel").unwrap_or_else(|| "auto".into());
    if wanted == "auto" {
        return colm_remote::kernel::newest_full(&kernels, preset)
            .map(|k| k.dir.clone())
            .with_context(|| {
                format!(
                    "{} has no Fortran kernel for the {preset} preset; build one with \
                     `colm-cli remote-kernel --host {} --root {root} --preset {preset} --env-script '…'`",
                    ssh.host, ssh.host
                )
            });
    }
    let dir = if wanted.contains('/') {
        wanted.clone()
    } else {
        format!("{root}/kernels/{wanted}")
    };
    let found = kernels
        .iter()
        .find(|k| k.dir == dir)
        .with_context(|| format!("{dir} is not a kernel on {}", ssh.host))?;
    ensure!(
        found.full,
        "{dir} has only a manifest; the Fortran engine needs the three programs (colm.x, mkinidata.x, mksrfdata.x)"
    );
    Ok(dir)
}

/// 作业体：先确保引擎编好，再跑三段。
#[allow(clippy::too_many_arguments)]
fn job_body(
    root: &str,
    engine_id: &str,
    remote_case: &str,
    remote_kernel: &str,
    threads: u32,
    plan: &RunPlan,
    scheduler: Scheduler,
    opts: &Opts,
) -> Result<String> {
    let mut run = format!(
        "RAYON_NUM_THREADS={threads} {bin}/colm-cli run {case} --kernel {kernel} --stream 1 --engine {engine} --preprocessors {pre}",
        bin = quote(&format!("{}/bin", engine::engine_dir(root, engine_id))),
        case = quote(remote_case),
        kernel = quote(remote_kernel),
        engine = plan.engine,
        pre = plan.preprocessors,
    );
    if plan.ranks > 1 {
        run.push_str(&format!(" --ranks {}", plan.ranks));
    }
    // MPI 进程怎么起：Slurm 里用 srun（跨节点），其余用 PATH 里的 mpiexec。`colm-cli` 认 COLM_MPIEXEC。
    let srun = match plan.launcher.as_str() {
        "srun" => true,
        "mpiexec" => false,
        _ => scheduler == Scheduler::Slurm,
    };
    let launcher = if plan.ranks > 1 && srun {
        "export COLM_MPIEXEC=srun\n"
    } else {
        ""
    };
    if let Some(stage) = opts.get("--stage") {
        ensure!(
            ["mksrfdata", "mkinidata", "colm"].contains(&stage.as_str()),
            "--stage must be mksrfdata, mkinidata or colm"
        );
        run.push_str(&format!(" --stage {stage}"));
    }
    if opts.get("--force").is_some_and(|v| v == "1") {
        run.push_str(" --force 1");
    }
    Ok(format!(
        "echo \"preparing the engine\" > phase\n{ensure}{launcher}echo \"running\" >> phase\n{run}\n",
        // 编译的并行度与运行线程数无关：至少 8 个任务。
        ensure = engine::ensure_script(root, engine_id, threads.max(8).clamp(1, 64)),
    ))
}

/// 在一台联网的 Linux 机器上为 `--targets` 预编引擎（glibc 2.17，见 `scripts/build-engine-linux.sh`），取回到本机的
/// 缓存目录（或 `--out`）。之后没有 cargo 或不能联网的服务器用 `remote-run` 的预编包路径就不必再编。
pub(super) fn cmd_dist(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let targets: Vec<String> = opts
        .get("--targets")
        .unwrap_or_else(|| "x86_64".into())
        .split(',')
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    ensure!(!targets.is_empty(), "--targets needs x86_64 and/or aarch64");
    for target in &targets {
        ensure!(
            ["x86_64", "aarch64"].contains(&target.as_str()),
            "unknown target {target}; use x86_64 or aarch64"
        );
    }
    let out = opts
        .get("--out")
        .map(PathBuf::from)
        .unwrap_or_else(engine::cache_dir);
    let snapshot = engine::snapshot(&engine_source()?)?;
    let uploaded = engine::upload(&ssh, &root, &snapshot)?;
    let src = quote(&format!("{}/src", engine::engine_dir(&root, &snapshot.id)));
    let dist_dir = format!("{}/dist", engine::engine_dir(&root, &snapshot.id));
    let id = format!("dist-{}", now());
    let body = format!(
        "echo {sid} > {src}/.colm-snapshot-id\necho \"building prebuilt engines\" > phase\nbash {src}/scripts/build-engine-linux.sh {src} {dist} {tools} {targets}\n",
        sid = snapshot.id,
        dist = quote(&dist_dir),
        tools = quote(&format!("{root}/tools")),
        targets = targets.join(" "),
    );
    job::submit(&ssh, &root, &id, &body, &Spec::bare())?;
    eprintln!(
        "building prebuilt engines ({}) on {} as job {id}; source snapshot {}{}",
        targets.join(", "),
        ssh.host,
        snapshot.id,
        if uploaded { " (uploaded)" } else { "" }
    );
    wait_job(&ssh, &root, &id, "the engine build", 240)?;
    let mut files = Vec::new();
    for target in &targets {
        let name = engine::prebuilt_name(target);
        ssh.download(&dist_dir, std::slice::from_ref(&name), &out)?;
        let cached = out.join(engine::cached_name(target, &snapshot.id));
        std::fs::rename(out.join(&name), &cached)
            .with_context(|| format!("cannot move {name} into {}", out.display()))?;
        files.push(cached.display().to_string());
    }
    println!("{}", json!({ "snapshot": snapshot.id, "files": files }));
    Ok(())
}

/// `colm-cli engine-pack --out FILE`：把引擎源码快照打成 `colm-src.tar.gz`，打包安装包时调用。
pub(super) fn cmd_pack(opts: &Opts) -> Result<()> {
    let out = opts.need("--out")?;
    let Some(source) = Source::find_checkout(&exe_dir()?) else {
        bail!("engine-pack must run from a build inside the repository");
    };
    let snapshot = engine::snapshot(&source)?;
    engine::pack_source(&snapshot, &out)?;
    println!(
        "{}",
        json!({ "out": out, "files": snapshot.files.len(), "snapshot": snapshot.id })
    );
    Ok(())
}

pub(super) fn cmd_probe(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let mut probe = colm_remote::probe::probe(&ssh, &root_of(opts)?)?;
    // 服务器编不了引擎时，有预编包也能用；两样都没有才算问题。
    if probe.build_problems.is_empty() {
        probe.engine = Some("source".into());
    } else {
        let source = engine_source().and_then(|s| engine::snapshot(&s)).ok();
        let prebuilt = engine::find_prebuilt(
            &probe.arch,
            source.as_ref().map(|s| s.id.as_str()),
            &engine::resource_dirs(&exe_dir()?),
        );
        if prebuilt.is_some() {
            probe.engine = Some("prebuilt".into());
        } else {
            let hint = format!(
                "no prebuilt engine for {} was found; build one with `colm-cli remote-dist` on a machine with internet",
                probe.arch
            );
            let reasons = probe.build_problems.clone();
            probe.problems.extend(reasons);
            probe.problems.push(hint);
        }
    }
    println!("{}", serde_json::to_string(&probe)?);
    Ok(())
}

pub(super) fn cmd_run(opts: &Opts) -> Result<()> {
    let case = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let plan = RunPlan::from_opts(opts)?;
    // Rust 引擎只用本机内核的清单（宏），所以必须给 --kernel；Fortran 引擎用服务器上的完整内核，
    // 本机只需要知道预设名（--preset，或从 --kernel 的清单读）。
    let local_kernel = match opts.get("--kernel") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            let kernel = colm_kernel::Kernel::open_manifest(&dir)?;
            Some((dir, kernel))
        }
        None => None,
    };
    ensure!(
        local_kernel.is_some() || plan.engine == "fortran",
        "--kernel is required\n{}",
        "(only --engine fortran can do without it, using --preset and a kernel built by remote-kernel)"
    );
    let preset = match (&local_kernel, opts.get("--preset")) {
        (_, Some(preset)) => preset,
        (Some((_, kernel)), None) => kernel.manifest.preset.clone(),
        (None, None) => bail!("--preset is required when there is no --kernel"),
    };
    let maps = opts
        .get_all("--map")
        .iter()
        .map(|m| parse_mapping(m))
        .collect::<Result<Vec<_>>>()?;
    let threads: u32 = opts
        .get("--threads")
        .map(|t| t.parse())
        .transpose()?
        .unwrap_or(8);
    let upload_unmapped = opts.get("--upload-unmapped").is_some_and(|v| v == "1");
    let dry_run = opts.get("--dry-run").is_some_and(|v| v == "1");
    let scheduler = parse_scheduler(opts, &ssh, &root)?;
    let resources = resources_from(opts, threads, plan.ranks, plan.nodes)?;

    let case_name = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?
        .get("DEF_CASE_NAME")
        .and_then(|v| match v {
            Value::Str(s) => Some(s.trim().to_owned()),
            _ => None,
        })
        .context("case.nml has no DEF_CASE_NAME")?;
    colm_case::validate_case_name(&case_name)?;
    let remote_case = format!(
        "{root}/cases/{case_name}-{}",
        short_hash(&case.to_string_lossy())
    );

    // 1. 算例副本：在本机暂存、改写路径，对应不上的先报出来。
    let staging =
        std::env::temp_dir().join(format!("colm-remote-{}-{}", case_name, std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let staged = stage_case(&case, &staging, &remote_case, &maps, upload_unmapped)?;
    if !staged.unmapped.is_empty() {
        let _ = std::fs::remove_dir_all(&staging);
        let list: Vec<String> = staged
            .unmapped
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        bail!(
            "these inputs are on this computer but have no server path; add a path mapping for them \
             (or allow uploading the files): {}",
            list.join(", ")
        );
    }

    // 2. 核对服务器上的数据在场。
    if !staged.remote_inputs.is_empty() {
        let checks: Vec<String> = staged
            .remote_inputs
            .iter()
            .map(|p| format!("[ -e {q} ] || echo {q}", q = quote(p)))
            .collect();
        let missing = ssh.run_ok(&checks.join("\n"))?;
        if !missing.trim().is_empty() {
            let _ = std::fs::remove_dir_all(&staging);
            bail!(
                "these inputs are missing on {}: {}",
                ssh.host,
                missing.trim().lines().collect::<Vec<_>>().join(", ")
            );
        }
    }

    // 3. 引擎源码、内核清单、算例副本上传。
    let engine = choose_engine(opts, &ssh, &root)?;
    let fortran = plan.engine == "fortran";
    let manifest = match &local_kernel {
        Some((dir, _)) => std::fs::read_to_string(dir.join("manifest.json"))?,
        None => String::new(),
    };
    let remote_kernel = if fortran {
        resolve_remote_kernel(opts, &ssh, &root, &preset)?
    } else {
        format!("{root}/kernels/{preset}-{}", short_hash(&manifest))
    };
    if dry_run {
        let _ = std::fs::remove_dir_all(&staging);
        let body = job_body(
            &root,
            engine.id(),
            &remote_case,
            &remote_kernel,
            threads,
            &plan,
            scheduler,
            opts,
        )?;
        let spec = Spec {
            scheduler,
            resources,
            name: sched::job_name(&case_name),
        };
        let script = job::job_script(&root, "JOBID", &body, &spec)?;
        println!(
            "{}",
            json!({
                "dry_run": true,
                "scheduler": scheduler.name(),
                "job_script": script,
                "remote_case": remote_case,
                "engine": engine.id(),
                "engine_kind": engine.kind(),
            })
        );
        return Ok(());
    }
    let engine_uploaded = engine.upload(&ssh, &root)?;
    if !fortran {
        ssh.run_ok(&format!(
            "mkdir -p {dir} && cat > {dir}/manifest.json <<'COLM_MANIFEST_EOF'\n{manifest}\nCOLM_MANIFEST_EOF\n",
            dir = quote(&remote_kernel)
        ))?;
    }
    let entries: Vec<String> = std::fs::read_dir(&staging)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    ssh.upload(&staging, &entries, &remote_case)?;
    let _ = std::fs::remove_dir_all(&staging);

    // 4. 提交作业：先确保引擎编好，再跑。
    let id = format!(
        "{}-{}",
        now(),
        short_hash(&format!("{}{}", case.display(), now()))
    );
    let body = job_body(
        &root,
        engine.id(),
        &remote_case,
        &remote_kernel,
        threads,
        &plan,
        scheduler,
        opts,
    )?;
    let spec = Spec {
        scheduler,
        resources,
        name: sched::job_name(&case_name),
    };
    let submitted = job::submit(&ssh, &root, &id, &body, &spec)?;
    let record = Record {
        host: ssh.host.clone(),
        root,
        job: id.clone(),
        remote_case: remote_case.clone(),
        case_name,
        engine: engine.id().to_owned(),
        submitted_at: now(),
        scheduler: submitted.scheduler.to_owned(),
        scheduler_id: submitted.scheduler_id.clone(),
    };
    std::fs::write(case.join(RECORD), serde_json::to_string_pretty(&record)?)?;
    println!(
        "{}",
        json!({
            "job": id,
            "pid": submitted.pid,
            "scheduler": submitted.scheduler,
            "scheduler_id": submitted.scheduler_id,
            "host": ssh.host,
            "remote_case": remote_case,
            "engine": engine.id(),
            "engine_kind": engine.kind(),
            "engine_uploaded": engine_uploaded,
            "case_bytes": staged.bytes,
            "remote_inputs": staged.remote_inputs,
            "uploaded_extra": staged.uploaded_extra,
        })
    );
    Ok(())
}

pub(super) fn cmd_status(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    let record = read_record(&case)?;
    let lines: usize = opts
        .get("--lines")
        .map(|l| l.parse())
        .transpose()?
        .unwrap_or(60);
    let status = job::status(&Ssh::new(&record.host)?, &record.root, &record.job, lines)?;
    println!("{}", json!({ "record": record, "status": status }));
    Ok(())
}

pub(super) fn cmd_cancel(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    let record = read_record(&case)?;
    job::cancel(&Ssh::new(&record.host)?, &record.root, &record.job)?;
    println!("{}", json!({ "cancelled": record.job }));
    Ok(())
}

/// 本机算例里记着“这份结果只取回了一部分”的标记；全部取回后删掉。
pub(crate) const FETCH_MARK: &str = ".colm-fetch.json";

/// 取回结果：`out/<名字>/history` 与三段的日志，放到本机算例的同一位置；清掉本机的“结果已过期”标记。
///
/// 不给 `--vars`、`--from`、`--to` 就取全部 history。给了就先在服务器上用 `colm-cli history-subset`
/// 裁成只含所选变量与月份的压缩小文件，再只把这些取回来（R5）：经慢链路时取回量与耗时随变量数线性下降。
/// 部分取回会在算例里留一个 `.colm-fetch.json`，说明取回了什么；同名文件会被覆盖，不在所选月份里的旧文件不动。
pub(super) fn cmd_fetch(opts: &Opts) -> Result<()> {
    let case = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let record = read_record(&case)?;
    let ssh = Ssh::new(&record.host)?;
    let history = format!("out/{}/history", record.case_name);
    let vars = opts.get("--vars");
    let from = opts.get("--from");
    let to = opts.get("--to");
    let partial = vars.is_some() || from.is_some() || to.is_some();
    let listing = ssh.run_ok(&format!(
        "cd {} && for e in {} mksrfdata.log mkinidata.log colm.log; do [ -e \"$e\" ] && echo \"$e\"; done",
        quote(&record.remote_case),
        quote(&history)
    ))?;
    let mut entries: Vec<String> = listing
        .lines()
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
        .collect();
    ensure!(
        entries.iter().any(|e| e == &history),
        "the server has no history for {} yet (the run may not have finished)",
        record.case_name
    );

    let mut subset = serde_json::Value::Null;
    if partial {
        // 先在服务器上裁剪，之后 history 目录不整个取，只取裁出来的文件。
        let tmp = format!("{}/.fetch-subset", record.remote_case);
        let bin = format!(
            "{}/bin/colm-cli",
            engine::engine_dir(&record.root, &record.engine)
        );
        let mut command = format!(
            "rm -rf {tmp} && {bin} history-subset {case} --out {tmp}",
            tmp = quote(&tmp),
            bin = quote(&bin),
            case = quote(&record.remote_case)
        );
        for (flag, value) in [("--vars", &vars), ("--from", &from), ("--to", &to)] {
            if let Some(value) = value {
                ensure!(
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_,-".contains(c)),
                    "{flag} may only contain letters, digits, _ , and -"
                );
                command.push_str(&format!(" {flag} {}", quote(value)));
            }
        }
        if let Some(level) = opts.get("--compress") {
            ensure!(
                level.len() == 1 && level.chars().all(|c| c.is_ascii_digit()),
                "--compress must be 0-9"
            );
            command.push_str(&format!(" --compress {level}"));
        }
        let out = ssh.run_ok(&command).map_err(|e| {
            anyhow::anyhow!(
                "{e}\n(the engine that ran this case may be too old to subset history; run the case again, or fetch everything by leaving out --vars/--from/--to)"
            )
        })?;
        subset = serde_json::from_str(out.trim().lines().last().unwrap_or("{}"))
            .unwrap_or(serde_json::Value::Null);
        let files = ssh.run_ok(&format!("ls -1 {}", quote(&tmp)))?;
        let files: Vec<String> = files.lines().map(str::to_owned).collect();
        ensure!(!files.is_empty(), "the server produced no subset files");
        entries.retain(|e| e != &history);
        ssh.download(&record.remote_case, &entries, &case)?;
        ssh.download(&tmp, &files, &case.join(&history))?;
        let _ = ssh.run_ok(&format!("rm -rf {}", quote(&tmp)));
        std::fs::write(
            case.join(FETCH_MARK),
            serde_json::to_string_pretty(&json!({
                "vars": vars,
                "from": from,
                "to": to,
                "files": files.len(),
                "fetched_at": now(),
            }))?,
        )?;
    } else {
        ssh.download(&record.remote_case, &entries, &case)?;
        let _ = std::fs::remove_file(case.join(FETCH_MARK));
    }
    colm_case::clear_results_stale(&case)?;
    let files = std::fs::read_dir(case.join(&history))?.count();
    println!(
        "{}",
        json!({ "fetched": entries, "history_files": files, "partial": partial, "subset": subset })
    );
    Ok(())
}

#[cfg(test)]
#[path = "remote_cmd_tests.rs"]
mod remote_cmd_tests;
