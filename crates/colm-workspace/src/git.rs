//! 工作区里的 git：只调系统的 `git`，从不交互（没有凭据提示）。

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

/// 交给 git 的本地路径（`-C` 的目录、`hash-object` 的文件）：模型库缓存下的路径在 Windows 上带 `\\?\` 前缀，git 不认，交给它之前去掉。
pub fn spelled(dir: &Path) -> std::ffi::OsString {
    match dir.to_str() {
        Some(text) => crate::layout::git_spelling(text).into(),
        None => dir.as_os_str().to_owned(),
    }
}

/// 在 `dir` 里运行 git，返回 stdout（去掉结尾换行）；失败时带上 stderr。
pub fn run(dir: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&run_bytes(dir, args)?)
        .trim_end_matches(['\n', '\r'])
        .to_owned())
}

fn run_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(spelled(dir))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null())
        .output()
        .context("cannot run git (is it installed?)")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// 同上，但把 stdin 交给 git（`git apply` 读补丁）。
pub fn run_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(spelled(dir))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot run git (is it installed?)")?;
    child
        .stdin
        .take()
        .context("no stdin")?
        .write_all(input)
        .context("cannot write to git")?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// 这个目录的仓库有没有提交（`git init` 之后只 add 没 commit 的没有）。
pub fn has_head(dir: &Path) -> bool {
    run(dir, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok()
}

pub fn head(dir: &Path) -> Result<String> {
    run(dir, &["rev-parse", "HEAD"])
}

pub fn short(dir: &Path, rev: &str) -> Result<String> {
    run(dir, &["rev-parse", "--short=8", rev])
}

/// 工作区有没有未提交的改动（被忽略的文件不算）。
pub fn is_dirty(dir: &Path) -> Result<bool> {
    Ok(!run(dir, &["status", "--porcelain"])?.is_empty())
}

/// 配置仓库本地的提交身份：工作区里的提交不依赖用户的全局 git 配置。
pub fn set_identity(dir: &Path) -> Result<()> {
    run(dir, &["config", "user.name", "CoLM Workspace"])?;
    run(dir, &["config", "user.email", "workspace@colm.local"])?;
    run(dir, &["config", "commit.gpgsign", "false"])?;
    Ok(())
}

/// 一条提交。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Commit {
    pub id: String,
    pub short: String,
    pub subject: String,
    pub at: u64,
}

/// `base..HEAD` 的提交，新的在前。
pub fn commits_since(dir: &Path, base: &str) -> Result<Vec<Commit>> {
    let text = run(
        dir,
        &[
            "log",
            "--format=%H%x1f%h%x1f%ct%x1f%s",
            &format!("{base}..HEAD"),
        ],
    )?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            Some(Commit {
                id: parts.next()?.to_owned(),
                short: parts.next()?.to_owned(),
                at: parts.next()?.parse().ok()?,
                subject: parts.next()?.to_owned(),
            })
        })
        .collect())
}

/// `base..HEAD` 改动的文件与增删行数。
pub fn changed_files(dir: &Path, base: &str) -> Result<Vec<FileChange>> {
    changed_between(dir, base, "HEAD")
}

/// `from..to` 改动的文件与增删行数（二进制文件两个数都是 0）。冻结版本的代码缓存里 HEAD
/// 不一定是冻结的那次提交，所以要能指定终点。
pub fn changed_between(dir: &Path, from: &str, to: &str) -> Result<Vec<FileChange>> {
    let bytes = run_bytes(
        dir,
        &["diff", "--no-renames", "--numstat", "-z", from, to, "--"],
    )?;
    let text = String::from_utf8(bytes).context("changed file paths must be UTF-8")?;
    Ok(text
        .split('\0')
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?;
            let removed = parts.next()?;
            Some(FileChange {
                path: parts.next()?.to_owned(),
                added: added.parse().unwrap_or(0),
                removed: removed.parse().unwrap_or(0),
            })
        })
        .collect())
}

/// 某次提交里一个路径的对象 id（内容相同的文件在不同克隆里 id 也相同）；路径不存在时为空。
pub fn blob(dir: &Path, rev: &str, path: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(spelled(dir))
        .args(["rev-parse", "--verify", "--quiet", &format!("{rev}:{path}")])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .context("cannot run git (is it installed?)")?;
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}
