//! 工作区里的 git：只调系统的 `git`，从不交互（没有凭据提示）。

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

/// 在 `dir` 里运行 git，返回 stdout（去掉结尾换行）；失败时带上 stderr。
pub fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
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
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\n', '\r'])
        .to_owned())
}

/// 同上，但把 stdin 交给 git（`git apply` 读补丁）。
pub fn run_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
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
    let text = run(dir, &["diff", "--numstat", base, "HEAD"])?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}
