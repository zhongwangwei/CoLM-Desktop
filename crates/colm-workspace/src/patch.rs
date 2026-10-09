//! 应用补丁与撤回（docs/design-ai-assistant.md 第 4 节 C 级）：每个补丁自动成为一次 git 提交。
//!
//! 补丁在落地前先逐个检查它碰到的路径：只能改工作区 `src/` 里的普通文件，不能碰 `.git`，
//! 不能碰黄金参考（`oracle/golden/`——改了它等于改答案），不许二进制补丁，大小有上限。

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;

use crate::git;
use crate::layout::Workspace;

/// 补丁文本大小上限（字节）。
pub const MAX_PATCH_BYTES: usize = 512 * 1024;

/// 不许补丁碰的前缀（相对 `src/`）。
const FORBIDDEN_PREFIXES: [&str; 2] = [".git/", "oracle/golden/"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub commit: String,
    pub short: String,
    pub files: Vec<String>,
}

/// `diff --git a/X b/Y` 头里的两个路径。只改权限（`old mode`/`new mode`）或二进制标记的那一段没有
/// `---`/`+++` 行，路径只在这个头里——不读它，那一段就绕过了禁区检查（第 655 轮实测：
/// 一段只改 `oracle/golden/` 文件权限、一段正常改文档，前者照样被应用）。
/// 路径可以含空格：两边相同时按长度精确切开，不同（改名）时在第一个 ` b/` 处切开、两边都查。
fn header_paths(header: &str) -> Result<Vec<String>> {
    let rest = header
        .strip_prefix("a/")
        .ok_or_else(|| anyhow::anyhow!("diff header must name a/… and b/… paths: {header:?}"))?;
    let same = (rest.len() >= 3 && (rest.len() - 3) % 2 == 0)
        .then(|| {
            let half = (rest.len() - 3) / 2;
            let (a, b) = (rest.get(..half)?, rest.get(half..)?);
            (b.strip_prefix(" b/") == Some(a)).then(|| a.to_owned())
        })
        .flatten();
    if let Some(path) = same {
        return Ok(vec![path]);
    }
    let (a, b) = rest
        .split_once(" b/")
        .ok_or_else(|| anyhow::anyhow!("diff header must name a/… and b/… paths: {header:?}"))?;
    Ok(vec![a.to_owned(), b.to_owned()])
}

/// 补丁里碰到的路径（`diff --git` 头、`--- a/…`、`+++ b/…`、`rename`/`copy` 行）。`/dev/null` 不算。
pub fn touched_paths(diff: &str) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for line in diff.lines() {
        // ponytail: reject Git C-quoted paths until a complete decoder is needed.
        if let Some(header) = line.strip_prefix("diff --git ") {
            ensure!(
                !header.contains('"'),
                "quoted patch paths are not supported"
            );
            for path in header_paths(header)? {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
            continue;
        }
        let candidate = if let Some(rest) = line.strip_prefix("+++ ") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("--- ") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("rename from ") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("copy to ") {
            Some(rest)
        } else {
            line.strip_prefix("copy from ")
        };
        let Some(raw) = candidate else { continue };
        ensure!(
            !raw.starts_with('"'),
            "quoted patch paths are not supported"
        );
        // 路径后面可能跟制表符与时间戳。
        let raw = raw.split('\t').next().unwrap_or(raw).trim();
        if raw == "/dev/null" {
            continue;
        }
        let path = raw
            .strip_prefix("a/")
            .or_else(|| raw.strip_prefix("b/"))
            .unwrap_or(raw);
        if !paths.iter().any(|p| p == path) {
            paths.push(path.to_owned());
        }
    }
    ensure!(
        !paths.is_empty(),
        "the patch touches no file; send a unified diff (diff --git a/… b/…)"
    );
    Ok(paths)
}

/// 一个相对路径能不能被补丁碰：不是绝对路径、不含 `..`、不在禁区里。
pub fn check_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty(), "empty path in the patch");
    ensure!(
        !path.starts_with('/') && !path.contains('\\') && !path.contains('\0'),
        "the patch must use relative paths inside the repository: {path:?}"
    );
    for part in path.split('/') {
        // 空段（`oracle//golden/x`）与 `.` 段会被 git 规整掉、落到同一个文件上，却对不上下面的前缀比较。
        ensure!(
            !part.is_empty() && part != ".",
            "the patch path must not contain empty or '.' segments: {path:?}"
        );
        ensure!(
            part != "..",
            "the patch may not leave the repository: {path:?}"
        );
        // macOS 与 Windows 的默认文件系统不分大小写：`.GIT`、`Oracle/Golden` 写到的就是受保护的那个。
        ensure!(
            !part.eq_ignore_ascii_case(".git"),
            "the patch may not touch .git: {path:?}"
        );
    }
    let lower = path.to_ascii_lowercase();
    for prefix in FORBIDDEN_PREFIXES {
        ensure!(
            !lower.starts_with(prefix),
            "{path} is protected (reference answers and git internals cannot be patched)"
        );
    }
    Ok(())
}

/// 应用补丁并提交。补丁先整体检查（`git apply --check`），不能应用就一个字节都不改。
pub fn apply(workspace: &Workspace, diff: &str, message: &str) -> Result<Applied> {
    ensure!(
        diff.len() <= MAX_PATCH_BYTES,
        "the patch is {} bytes; the limit is {MAX_PATCH_BYTES}",
        diff.len()
    );
    ensure!(
        !diff.contains("GIT binary patch") && !diff.lines().any(|l| l.starts_with("Binary files ")),
        "binary patches are not allowed"
    );
    let message = message.trim();
    ensure!(
        !message.is_empty() && message.len() <= 200 && !message.contains('\n'),
        "the commit message must be one short line"
    );
    let files = touched_paths(diff)?;
    for file in &files {
        check_path(file)?;
    }
    let src = workspace.src();
    ensure!(
        !git::is_dirty(&src)?,
        "the workspace has uncommitted changes; commit or revert them first"
    );
    let bytes = if diff.ends_with('\n') {
        diff.as_bytes().to_vec()
    } else {
        format!("{diff}\n").into_bytes()
    };
    git::run_with_input(
        &src,
        &["apply", "--check", "--whitespace=nowarn", "-"],
        &bytes,
    )
    .context("the patch does not apply to the current files")?;
    git::run_with_input(&src, &["apply", "--whitespace=nowarn", "-"], &bytes)?;
    // 只加补丁碰过的路径：不顺手带进别的东西。
    let mut add = vec!["add", "--"];
    add.extend(files.iter().map(String::as_str));
    git::run(&src, &add)?;
    git::run(&src, &["commit", "-q", "-m", &format!("ws: {message}")])
        .context("the patch applied but nothing changed")?;
    let commit = git::head(&src)?;
    Ok(Applied {
        short: git::short(&src, &commit)?,
        commit,
        files,
    })
}

/// 撤回到某个提交（只能撤回到工作区自己的提交里：基线提交或之后的）。之后的提交被丢弃。
pub fn revert_to(workspace: &Workspace, commit: &str) -> Result<String> {
    ensure!(
        !commit.is_empty() && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "give a commit id (hex), got {commit:?}"
    );
    let src = workspace.src();
    let full = git::run(
        &src,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .with_context(|| format!("no such commit {commit}"))?;
    let base = &workspace.info.base_commit;
    // 目标要在 基线..HEAD 之间（含基线）：基线是它的祖先或它就是基线，并且它是 HEAD 的祖先。
    let on_branch =
        full == *base || (is_ancestor(&src, base, &full)? && is_ancestor(&src, &full, "HEAD")?);
    ensure!(
        on_branch,
        "{commit} is not one of this workspace's own commits"
    );
    git::run(&src, &["reset", "-q", "--hard", &full])?;
    git::run(&src, &["clean", "-q", "-fd"])?;
    Ok(full)
}

fn is_ancestor(dir: &Path, ancestor: &str, of: &str) -> Result<bool> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["merge-base", "--is-ancestor", ancestor, of])
        .status()
        .context("cannot run git")?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        other => bail!("git merge-base failed ({other:?})"),
    }
}

/// 把工作区相对基线的全部改动导出成一个补丁（D 级：界面上的“导出”按钮）。
pub fn export(workspace: &Workspace) -> Result<String> {
    git::run(
        &workspace.src(),
        &["diff", "--no-color", &workspace.info.base_commit, "HEAD"],
    )
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod patch_tests;
