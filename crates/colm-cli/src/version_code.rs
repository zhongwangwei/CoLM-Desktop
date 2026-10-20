//! 只读浏览一个冻结版本的代码：版本页"代码"页签里冻结的版本（以及还在用父版本代码的草稿）
//! 也能像修改版那样逐个文件看，只是不能改（2026-10-20 用户要求"代码在草稿里直接改，冻结后只读"）。
//!
//! 只读注册表里的缓存：修改版是冻结时的 git 克隆，按冻结的那次提交读（克隆的 HEAD 不一定是它）；
//! 官方版是缓存里的普通源码目录。返回的形式和工作区的文件浏览一样（`colm_workspace::files`）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, ensure, Context, Result};
use colm_model::{CodeKind, FrozenRevision, VersionRef};
use colm_workspace::files::{Entry, FileText, MAX_TEXT_BYTES};
use colm_workspace::git;

use crate::Opts;

/// 一个冻结版本的代码在哪里。
enum Source {
    Git { dir: PathBuf, commit: String },
    Plain(PathBuf),
}

fn source(opts: &Opts) -> Result<Source> {
    let store = crate::model_version::registry(opts)?;
    let revision: FrozenRevision = store.get(&VersionRef {
        id: opts.need_str("--version")?,
        revision: opts
            .need_str("--revision")?
            .parse()
            .context("revision must be a number")?,
    })?;
    let cache = crate::model_version::cache_dir(opts, &revision.code)?;
    match (&revision.code.kind, &revision.code.workspace) {
        (CodeKind::Workspace, Some(name)) => Ok(Source::Git {
            dir: cache.join("workspaces").join(name).join("src"),
            commit: revision
                .code
                .commit
                .clone()
                .context("a workspace version records its commit")?,
        }),
        _ => {
            let plain = cache.join("source");
            ensure!(
                plain.is_dir(),
                "this official version only keeps a source archive; its files cannot be browsed"
            );
            Ok(Source::Plain(plain))
        }
    }
}

/// 只认仓库里的相对路径：不许空段、`.`、`..`、绝对路径。
fn check(path: &str) -> Result<()> {
    ensure!(
        !path.starts_with('/') && !path.contains('\\') && !path.contains('\0'),
        "{path} is not a relative path inside the source"
    );
    for part in path.split('/') {
        ensure!(
            !part.is_empty() && part != "." && part != "..",
            "{path} is not a relative path inside the source"
        );
    }
    Ok(())
}

fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(git::spelled(dir))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
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

fn text_of(bytes: Vec<u8>) -> Option<String> {
    (!bytes.contains(&0))
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

pub fn list(opts: &Opts) -> Result<Vec<Entry>> {
    list_in(&source(opts)?, &opts.get("--dir").unwrap_or_default())
}

fn list_in(source: &Source, dir: &str) -> Result<Vec<Entry>> {
    if !dir.is_empty() {
        check(dir)?;
    }
    let join = |name: &str| {
        if dir.is_empty() {
            name.to_owned()
        } else {
            format!("{dir}/{name}")
        }
    };
    let mut entries = Vec::new();
    match source {
        Source::Git { dir: repo, commit } => {
            let tree = if dir.is_empty() {
                commit.clone()
            } else {
                format!("{commit}:{dir}")
            };
            let listing = git_bytes(repo, &["ls-tree", "-z", "-l", &tree])?;
            for line in listing.split(|b| *b == 0).filter(|l| !l.is_empty()) {
                let line = String::from_utf8_lossy(line);
                let Some((meta, name)) = line.split_once('\t') else {
                    continue;
                };
                let mut fields = meta.split_whitespace();
                let mode = fields.next().unwrap_or_default();
                let kind = fields.next().unwrap_or_default();
                let size = fields.nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let kind = match (mode, kind) {
                    ("120000", _) => "link",
                    (_, "tree") => "dir",
                    (_, "blob") => "file",
                    // 子模块（commit 对象）：只列出来，打不开。
                    _ => "link",
                };
                entries.push(Entry {
                    name: name.to_owned(),
                    path: join(name),
                    kind,
                    size: if kind == "file" { size } else { 0 },
                });
            }
        }
        Source::Plain(root) => {
            let base = within(root, dir)?;
            ensure!(base.is_dir(), "{dir} is not a directory");
            for entry in std::fs::read_dir(&base)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let meta = std::fs::symlink_metadata(entry.path())?;
                let kind = if meta.file_type().is_symlink() {
                    "link"
                } else if meta.is_dir() {
                    "dir"
                } else {
                    "file"
                };
                entries.push(Entry {
                    path: join(&name),
                    name,
                    kind,
                    size: if kind == "file" { meta.len() } else { 0 },
                });
            }
        }
    }
    entries.retain(|e| e.name != ".git");
    entries.sort_by(|a, b| (a.kind != "dir", &a.name).cmp(&(b.kind != "dir", &b.name)));
    Ok(entries)
}

/// 普通源码目录里的一个路径；每一段都不许是符号链接，免得读到缓存外面。
fn within(root: &Path, path: &str) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    if path.is_empty() {
        return Ok(current);
    }
    for part in path.split('/') {
        current.push(part);
        let meta = std::fs::symlink_metadata(&current)
            .with_context(|| format!("{path} does not exist"))?;
        ensure!(
            !meta.file_type().is_symlink(),
            "{path} goes through a symbolic link"
        );
    }
    Ok(current)
}

pub fn read(opts: &Opts) -> Result<FileText> {
    read_in(&source(opts)?, &opts.need_str("--path")?)
}

fn read_in(source: &Source, path: &str) -> Result<FileText> {
    check(path)?;
    let (size, text) = match source {
        Source::Git { dir, commit } => {
            let object = format!("{commit}:{path}");
            let size: u64 = String::from_utf8_lossy(&git_bytes(dir, &["cat-file", "-s", &object])?)
                .trim()
                .parse()
                .context("cannot read the file size")?;
            let text = if size <= MAX_TEXT_BYTES {
                text_of(git_bytes(dir, &["cat-file", "blob", &object])?)
            } else {
                None
            };
            (size, text)
        }
        Source::Plain(root) => {
            let file = within(root, path)?;
            let meta = std::fs::metadata(&file)?;
            ensure!(meta.is_file(), "{path} is not a file");
            let text = if meta.len() <= MAX_TEXT_BYTES {
                text_of(std::fs::read(&file)?)
            } else {
                None
            };
            (meta.len(), text)
        }
    };
    Ok(FileText {
        path: path.to_owned(),
        size,
        text,
    })
}

#[cfg(test)]
#[path = "version_code_tests.rs"]
mod version_code_tests;
