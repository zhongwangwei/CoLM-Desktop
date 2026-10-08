//! 远程运行（docs/design-ai-assistant.md 第 5.3 节，R1）：把一个算例放到服务器上用 Rust 引擎跑，再把结果取回来。
//!
//! ```text
//! colm-cli remote-probe  --host H --root R
//! colm-cli remote-run    <case> --host H --root R --kernel <本机内核目录> [--map 本机前缀=服务器前缀]…
//!                        [--stage S] [--force 1] [--threads N] [--upload-unmapped 1]
//! colm-cli remote-status <case> [--lines N]
//! colm-cli remote-cancel <case>
//! colm-cli remote-fetch  <case>
//! ```
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

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::Value;
use colm_remote::engine::{self, Source};
use colm_remote::job;
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
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("colm-cli has no directory")?;
    if let Some(source) = Source::find_checkout(dir) {
        return Ok(source);
    }
    for candidate in [
        dir.join("colm-src.tar.gz"),
        dir.join("../Resources/colm-src.tar.gz"),
    ] {
        if candidate.is_file() {
            return Ok(Source::Tarball(candidate));
        }
    }
    bail!("the engine sources were not found next to colm-cli; this build cannot run remotely")
}

pub(super) fn cmd_probe(opts: &Opts) -> Result<()> {
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let probe = colm_remote::probe::probe(&ssh, &root_of(opts)?)?;
    println!("{}", serde_json::to_string(&probe)?);
    Ok(())
}

pub(super) fn cmd_run(opts: &Opts) -> Result<()> {
    let case = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let ssh = Ssh::new(&opts.need_str("--host")?)?;
    let root = root_of(opts)?;
    let kernel_dir = opts.need("--kernel")?;
    let kernel = colm_kernel::Kernel::open_manifest(&kernel_dir)?;
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
    let snapshot = engine::snapshot(&engine_source()?)?;
    let engine_uploaded = engine::upload(&ssh, &root, &snapshot)?;
    let manifest = std::fs::read_to_string(kernel_dir.join("manifest.json"))?;
    let remote_kernel = format!(
        "{root}/kernels/{}-{}",
        kernel.manifest.preset,
        short_hash(&manifest)
    );
    ssh.run_ok(&format!(
        "mkdir -p {dir} && cat > {dir}/manifest.json <<'COLM_MANIFEST_EOF'\n{manifest}\nCOLM_MANIFEST_EOF\n",
        dir = quote(&remote_kernel)
    ))?;
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
    let mut run = format!(
        "RAYON_NUM_THREADS={threads} {bin}/colm-cli run {case} --kernel {kernel} --stream 1 --engine rust --preprocessors rust",
        bin = quote(&format!("{}/bin", engine::engine_dir(&root, &snapshot.id))),
        case = quote(&remote_case),
        kernel = quote(&remote_kernel),
    );
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
    let body = format!(
        "echo \"preparing the engine\" > phase\n{ensure}echo \"running\" >> phase\n{run}\n",
        ensure = engine::ensure_script(&root, &snapshot.id, threads.clamp(1, 64)),
    );
    let pid = job::submit(&ssh, &root, &id, &body)?;
    let record = Record {
        host: ssh.host.clone(),
        root,
        job: id.clone(),
        remote_case: remote_case.clone(),
        case_name,
        engine: snapshot.id.clone(),
        submitted_at: now(),
    };
    std::fs::write(case.join(RECORD), serde_json::to_string_pretty(&record)?)?;
    println!(
        "{}",
        json!({
            "job": id,
            "pid": pid,
            "host": ssh.host,
            "remote_case": remote_case,
            "engine": snapshot.id,
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

/// 取回结果：`out/<名字>/history` 与三段的日志，放到本机算例的同一位置；清掉本机的“结果已过期”标记。
pub(super) fn cmd_fetch(opts: &Opts) -> Result<()> {
    let case = colm_kernel::manifest::absolute(&opts.positional_case()?)?;
    let record = read_record(&case)?;
    let ssh = Ssh::new(&record.host)?;
    let history = format!("out/{}/history", record.case_name);
    let listing = ssh.run_ok(&format!(
        "cd {} && for e in {} mksrfdata.log mkinidata.log colm.log; do [ -e \"$e\" ] && echo \"$e\"; done",
        quote(&record.remote_case),
        quote(&history)
    ))?;
    let entries: Vec<String> = listing
        .lines()
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
        .collect();
    ensure!(
        entries.iter().any(|e| e == &history),
        "the server has no history for {} yet (the run may not have finished)",
        record.case_name
    );
    ssh.download(&record.remote_case, &entries, &case)?;
    colm_case::clear_results_stale(&case)?;
    let files = std::fs::read_dir(case.join(&history))?.count();
    println!("{}", json!({ "fetched": entries, "history_files": files }));
    Ok(())
}

#[cfg(test)]
#[path = "remote_cmd_tests.rs"]
mod remote_cmd_tests;
