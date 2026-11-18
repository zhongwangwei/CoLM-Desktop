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

/// Serialize complete source edits across CLI processes, not just individual Git commands.
pub(crate) struct WriteLock(std::path::PathBuf);

impl WriteLock {
    pub(crate) fn acquire(workspace: &crate::layout::Workspace) -> Result<Self> {
        let path = workspace.dir.join("source-write.lock");
        let started = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(error) = writeln!(file, "{}", std::process::id()) {
                        let _ = std::fs::remove_file(&path);
                        return Err(error.into());
                    }
                    return Ok(Self(path));
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        || (cfg!(windows)
                            && error.kind() == std::io::ErrorKind::PermissionDenied) =>
                {
                    anyhow::ensure!(started.elapsed().as_secs() < 15,
                        "workspace is busy; if its writer crashed, remove {} after confirming no write is running", path.display());
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => return Err(error).context("cannot lock workspace source"),
            }
        }
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Debug, Clone, PartialEq)]
enum DiskEntry {
    Directory(std::fs::Permissions),
    Link(std::path::PathBuf),
    File {
        digest: Vec<u8>,
        permissions: std::fs::Permissions,
    },
}

type DiskState = std::collections::BTreeMap<std::path::PathBuf, DiskEntry>;

fn capture(root: &Path, paths: &[String], backup: Option<&Path>) -> Result<DiskState> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut entries = DiskState::new();
    let mut pending: Vec<_> = paths.iter().map(std::path::PathBuf::from).collect();
    while let Some(path) = pending.pop() {
        if entries.contains_key(&path) {
            continue;
        }
        let full = root.join(&path);
        let meta = match std::fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let entry = if meta.file_type().is_symlink() {
            DiskEntry::Link(std::fs::read_link(&full)?)
        } else if meta.is_dir() {
            for child in std::fs::read_dir(&full)? {
                pending.push(path.join(child?.file_name()));
            }
            DiskEntry::Directory(meta.permissions())
        } else {
            anyhow::ensure!(
                meta.is_file(),
                "cannot back up special file {}",
                full.display()
            );
            let mut file = std::fs::File::open(&full)?;
            let mut saved = if let Some(backup) = backup {
                let dest = backup.join(&path);
                std::fs::create_dir_all(dest.parent().context("missing backup parent")?)?;
                Some(std::fs::File::create(dest)?)
            } else {
                None
            };
            let mut hasher = Sha256::new();
            let mut buffer = [0; 64 * 1024];
            loop {
                let size = file.read(&mut buffer)?;
                if size == 0 {
                    break;
                }
                hasher.update(&buffer[..size]);
                if let Some(saved) = &mut saved {
                    use std::io::Write;
                    saved.write_all(&buffer[..size])?;
                }
            }
            DiskEntry::File {
                digest: hasher.finalize().to_vec(),
                permissions: meta.permissions(),
            }
        };
        entries.insert(path, entry);
    }
    Ok(entries)
}

/// Back up only owned paths, including ignored contents of a replaced directory.
pub(crate) struct Edit {
    root: std::path::PathBuf,
    backup: std::path::PathBuf,
    paths: Vec<String>,
    before: DiskState,
    expected: DiskState,
    index: Vec<u8>,
    expected_index: Vec<u8>,
    head: String,
}

impl Edit {
    pub(crate) fn begin(
        workspace: &crate::layout::Workspace,
        mut paths: Vec<String>,
    ) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = workspace.src();
        paths.extend(
            colm_remote::engine::source_links(&root)?
                .keys()
                .map(|p| p.to_string_lossy().replace('\\', "/")),
        );
        paths.push(colm_remote::engine::SOURCE_LINKS_FILE.into());
        paths.sort();
        paths.dedup();
        let mut args = vec!["--literal-pathspecs", "ls-files", "--stage", "-z", "--"];
        args.extend(paths.iter().map(String::as_str));
        let index = run_bytes(&root, &args)?;
        let head = head(&root)?;
        let backup = workspace.dir.join(format!(
            ".edit-backup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&backup)?;
        let before = match capture(&root, &paths, Some(&backup.join("files"))) {
            Ok(before) => before,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&backup);
                return Err(error);
            }
        };
        if capture(&root, &paths, None)? != before {
            let _ = std::fs::remove_dir_all(&backup);
            bail!("files changed while preparing the edit; retry after reviewing external changes");
        }
        std::fs::write(backup.join("index"), &index)?;
        std::fs::write(backup.join("entries.txt"), format!("{before:#?}"))?;
        Ok(Self {
            root,
            backup,
            paths,
            expected: before.clone(),
            before,
            expected_index: index.clone(),
            index,
            head,
        })
    }

    /// Update only paths this operation actually wrote, never arbitrary post-error state.
    pub(crate) fn record(&mut self, paths: &[String]) -> Result<()> {
        let changed = capture(&self.root, paths, None)?;
        self.expected
            .retain(|path, _| !paths.iter().any(|scope| path.starts_with(scope)));
        self.expected.extend(changed);
        Ok(())
    }

    pub(crate) fn change<T>(
        &mut self,
        paths: &[String],
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.checkpoint()?;
        let result = operation();
        self.record(paths)?;
        result
    }

    /// External changes are conflicts, not something the rollback owns.
    pub(crate) fn checkpoint(&self) -> Result<()> {
        anyhow::ensure!(
            self.index_state()? == self.expected_index,
            "the Git index changed outside this operation; preserve and review its staging"
        );
        anyhow::ensure!(capture(&self.root, &self.paths, None)? == self.expected,
            "files changed outside this operation; preserve and review those changes before retrying");
        Ok(())
    }

    fn index_state(&self) -> Result<Vec<u8>> {
        let mut args = vec!["--literal-pathspecs", "ls-files", "--stage", "-z", "--"];
        args.extend(self.paths.iter().map(String::as_str));
        run_bytes(&self.root, &args)
    }

    pub(crate) fn record_index(&mut self) -> Result<()> {
        self.expected_index = self.index_state()?;
        Ok(())
    }

    pub(crate) fn stage(&mut self, args: &[&str]) -> Result<()> {
        anyhow::ensure!(
            self.index_state()? == self.expected_index,
            "the Git index changed outside this operation; preserve and review its staging"
        );
        run(&self.root, args)?;
        self.record_index()
    }

    pub(crate) fn finish<T>(self, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => {
                if let Err(error) = std::fs::remove_dir_all(&self.backup) {
                    eprintln!(
                        "workspace edit succeeded; could not remove backup {}: {error}",
                        self.backup.display()
                    );
                }
                Ok(value)
            }
            Err(error) => {
                let rollback = (|| {
                    // Never undo a successful commit or an external branch change.
                    anyhow::ensure!(
                        head(&self.root)? == self.head,
                        "workspace HEAD changed; files were preserved"
                    );
                    self.rollback()
                })();
                match rollback {
                    Ok(()) => {
                        let _ = std::fs::remove_dir_all(&self.backup);
                        Err(error)
                    }
                    Err(rollback) => Err(error.context(format!(
                        "rollback incomplete: {rollback:#}; originals retained at {}",
                        self.backup.display()
                    ))),
                }
            }
        }
    }

    fn rollback(&self) -> Result<()> {
        let current = capture(&self.root, &self.paths, None)?;
        anyhow::ensure!(
            current == self.expected,
            "files changed again during the operation; preserved them"
        );
        anyhow::ensure!(
            self.index_state()? == self.expected_index,
            "the Git index changed again during the operation; preserved its staging"
        );
        if self.expected_index != self.index {
            // Restore the index only for the paths owned by this operation.
            let mut args = vec!["--literal-pathspecs", "ls-files", "-z", "--"];
            args.extend(self.paths.iter().map(String::as_str));
            let indexed = run_bytes(&self.root, &args)?;
            run_with_input(
                &self.root,
                &["update-index", "--force-remove", "-z", "--stdin"],
                &indexed,
            )?;
            run_with_input(
                &self.root,
                &["update-index", "-z", "--index-info"],
                &self.index,
            )?;
        }
        for (path, entry) in current.iter().rev() {
            if self.before.get(path) == Some(entry) {
                continue;
            }
            let full = self.root.join(path);
            if matches!(entry, DiskEntry::Directory(_)) {
                // Never recursively remove a directory: a newly arrived user file must survive.
                if matches!(self.before.get(path), Some(DiskEntry::Directory(_))) {
                    continue;
                }
                std::fs::remove_dir(&full)?;
            } else {
                std::fs::remove_file(&full)?;
            }
        }
        for (path, entry) in &self.before {
            if current.get(path) == Some(entry) {
                continue;
            }
            let full = self.root.join(path);
            std::fs::create_dir_all(full.parent().context("missing restore parent")?)?;
            match entry {
                DiskEntry::Directory(_) => std::fs::create_dir_all(full)?,
                DiskEntry::File { permissions, .. } => {
                    std::fs::copy(self.backup.join("files").join(path), &full)?;
                    std::fs::set_permissions(full, permissions.clone())?;
                }
                DiskEntry::Link(target) => {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(target, full)?;
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_file(target, full)?;
                }
            }
        }
        // Apply directory modes last, after restoring children of read-only directories.
        for (path, entry) in self.before.iter().rev() {
            if let DiskEntry::Directory(permissions) = entry {
                std::fs::set_permissions(self.root.join(path), permissions.clone())?;
            }
        }
        Ok(())
    }
}
