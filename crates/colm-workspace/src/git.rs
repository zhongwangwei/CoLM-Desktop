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
    Ok(String::from_utf8(run_bytes(dir, args)?)
        .context("Git output contains non-UTF-8 text or file paths; rename unsupported paths before editing")?
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
            args.iter()
                .find(|arg| !arg.starts_with('-'))
                .copied()
                .unwrap_or_default(),
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
            args.iter()
                .find(|arg| !arg.starts_with('-'))
                .copied()
                .unwrap_or_default(),
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

/// The stable OS lock survives as a file; its ownership ends when the process exits.
pub(crate) struct WriteLock {
    _file: std::fs::File,
    legacy: std::path::PathBuf,
    owner: String,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        // Old versions coordinate through this marker, so keep it until our write ends.
        if std::fs::read_to_string(&self.legacy).ok().as_ref() == Some(&self.owner) {
            let _ = std::fs::remove_file(&self.legacy);
        }
    }
}

impl WriteLock {
    pub(crate) fn acquire(workspace: &crate::layout::Workspace) -> Result<Self> {
        let file = colm_case::file_lock::exclusive_timeout(
            &workspace.dir.join(".source-write.guard"),
            std::time::Duration::from_secs(15),
        )
        .map_err(|error| {
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|cause| cause.kind() == std::io::ErrorKind::TimedOut)
            {
                error.context("workspace is busy; timed out waiting for its writer")
            } else {
                error.context("cannot acquire workspace writer lock")
            }
        })?;
        let legacy = workspace.dir.join("source-write.lock");
        match std::fs::read_to_string(&legacy) {
            Ok(owner) => {
                let pid = owner.trim().parse::<u32>().context(
                    "workspace has an unreadable legacy write lock; inspect source-write.lock before removing it",
                )?;
                let absent = !colm_case::file_lock::process_is_alive(pid)
                    .context("cannot inspect legacy workspace writer")?;
                anyhow::ensure!(
                    absent,
                    "workspace is busy or its legacy writer cannot be checked; inspect {}",
                    legacy.display()
                );
                anyhow::ensure!(
                    std::fs::read_to_string(&legacy)? == owner,
                    "workspace is busy; legacy writer changed while checking its owner"
                );
                std::fs::remove_file(&legacy)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect legacy workspace writer"),
        }
        let owner = std::process::id().to_string();
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&legacy)
            .context("workspace is busy; cannot claim legacy writer marker")?;
        let lock = Self {
            _file: file,
            legacy,
            owner,
        };
        use std::io::Write;
        marker
            .write_all(lock.owner.as_bytes())
            .context("cannot write workspace writer marker")?;
        Ok(lock)
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
    created_dirs: Vec<std::path::PathBuf>,
}

impl Edit {
    pub(crate) fn begin(
        workspace: &crate::layout::Workspace,
        mut paths: Vec<String>,
    ) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // Git resolves index and worktree arguments after -C, so pin them to this caller's CWD.
        let root = std::path::absolute(workspace.src())?;
        let workspace_dir = std::path::absolute(&workspace.dir)?;
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
        let backup = workspace_dir.join(format!(
            ".edit-backup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&backup)?;
        let prepare =
            (|| -> Result<DiskState> {
                let before = capture(&root, &paths, Some(&backup.join("files")))?;
                anyhow::ensure!(capture(&root, &paths, None)? == before,
                "files changed while preparing the edit; retry after reviewing external changes");
                std::fs::write(backup.join("index"), &index)?;
                std::fs::write(backup.join("entries.txt"), format!("{before:#?}"))?;
                Ok(before)
            })();
        let before = match prepare {
            Ok(before) => before,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&backup);
                return Err(error);
            }
        };
        Ok(Self {
            root,
            backup,
            paths,
            expected: before.clone(),
            before,
            expected_index: index.clone(),
            index,
            head,
            created_dirs: Vec::new(),
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

    /// Git may partly update an index before failing; publish only a successful private index.
    pub(crate) fn stage(&mut self, args: &[&str]) -> Result<()> {
        self.change_index(&[(args, &[])])
    }

    fn change_index(&mut self, commands: &[(&[&str], &[u8])]) -> Result<()> {
        use std::fs::{File, FileTimes};
        use std::io::Write;
        self.checkpoint()?;
        let index = self.root.join(".git/index");
        let modified = std::fs::metadata(&index)?.modified()?;
        let original = std::fs::read(&index)?;
        let staged = self.backup.join("staged-index");
        // Git uses the index timestamp to decide whether to recheck a file's contents.
        {
            let mut copy = File::create(&staged)?;
            copy.write_all(&original)?;
            copy.set_times(FileTimes::new().set_modified(modified))?;
        }
        for (args, input) in commands {
            let mut child = Command::new("git")
                .arg("-C")
                .arg(spelled(&self.root))
                .args(*args)
                .env("GIT_INDEX_FILE", spelled(&staged))
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?;
            let written = child
                .stdin
                .take()
                .context("missing Git stdin")?
                .write_all(input);
            let output = child.wait_with_output()?;
            anyhow::ensure!(
                output.status.success(),
                "git {} failed: {}",
                args.iter()
                    .find(|arg| !arg.starts_with('-'))
                    .copied()
                    .unwrap_or_default(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            written.context("cannot write index transaction to Git")?;
        }
        let mut args = vec!["--literal-pathspecs", "ls-files", "--stage", "-z", "--"];
        args.extend(self.paths.iter().map(String::as_str));
        let staged_entries = Command::new("git")
            .arg("-C")
            .arg(spelled(&self.root))
            .args(args)
            .env("GIT_INDEX_FILE", spelled(&staged))
            .output()?;
        anyhow::ensure!(
            staged_entries.status.success(),
            "cannot read prepared Git index"
        );
        self.checkpoint()?;
        let lock_path = self.root.join(".git/index.lock");
        let mut lock = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        let result = (|| -> Result<()> {
            anyhow::ensure!(
                std::fs::read(&index)? == original,
                "the Git index changed outside this operation; preserve and review its staging"
            );
            let mut prepared = File::open(&staged)?;
            let modified = prepared.metadata()?.modified()?;
            std::io::copy(&mut prepared, &mut lock)?;
            lock.set_times(FileTimes::new().set_modified(modified))?;
            lock.sync_all()?;
            drop(lock);
            std::fs::rename(&lock_path, &index)?;
            self.expected_index = staged_entries.stdout;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&lock_path);
        }
        result
    }

    pub(crate) fn create_parents(&mut self, path: &str) -> Result<()> {
        let mut missing = Vec::new();
        let mut parent = self.root.join(path);
        while let Some(dir) = parent.parent() {
            if dir == self.root || dir.exists() {
                break;
            }
            missing.push(dir.to_path_buf());
            parent = dir.to_path_buf();
        }
        for dir in missing.into_iter().rev() {
            match std::fs::create_dir(&dir) {
                Ok(()) => self.created_dirs.push(dir),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Renaming a staged regular file prevents writes through hard-linked destinations.
    pub(crate) fn replace_file(
        &mut self,
        path: &str,
        source: &mut impl std::io::Read,
        permissions: Option<std::fs::Permissions>,
    ) -> Result<()> {
        self.checkpoint()?;
        self.create_parents(path)?;
        let target = self.root.join(path);
        let temporary = self.backup.join("replacement");
        let result = (|| -> Result<()> {
            let mut file = std::fs::File::create(&temporary)?;
            if let Ok(meta) = std::fs::metadata(&target) {
                anyhow::ensure!(!meta.permissions().readonly(), "{path} is read-only");
                file.set_permissions(meta.permissions())?;
            }
            if let Some(permissions) = permissions {
                file.set_permissions(permissions)?;
            }
            std::io::copy(source, &mut file)?;
            file.sync_all()?;
            drop(file);
            let staged = capture(&self.backup, &["replacement".into()], None)?;
            let expected = staged
                .get(Path::new("replacement"))
                .context("missing prepared replacement")?
                .clone();
            self.checkpoint()?;
            std::fs::rename(&temporary, target)?;
            self.expected.insert(path.into(), expected);
            Ok(())
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }

    /// Compute Git's intended contents separately so a partial failure cannot own an external edit.
    pub(crate) fn git_change(
        &mut self,
        paths: &[String],
        args: &[&str],
        input: &[u8],
    ) -> Result<()> {
        // Both preview and execution share the real Git directory: admit only worktree operations.
        let worktree_only = match args {
            ["apply", "--whitespace=nowarn", "-"] => true,
            ["--literal-pathspecs", "restore", "--source", source, "--worktree", "--", path] => {
                !source.starts_with('-') && paths == [*path]
            }
            _ => false,
        };
        anyhow::ensure!(
            worktree_only,
            "git_change only accepts worktree-only apply or restore"
        );
        self.checkpoint()?;
        let staged = self.backup.join("worktree");
        std::fs::create_dir_all(&staged)?;
        let before = capture(&self.root, paths, Some(&staged))?;
        for (path, entry) in &before {
            let full = staged.join(path);
            match entry {
                DiskEntry::Directory(_) => std::fs::create_dir_all(&full)?,
                DiskEntry::File { permissions, .. } => {
                    std::fs::set_permissions(full, permissions.clone())?
                }
                DiskEntry::Link(target) => {
                    std::fs::create_dir_all(full.parent().context("missing staged parent")?)?;
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(target, full)?;
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_file(target, full)?;
                }
            }
        }
        let git_dir = self.root.join(".git");
        let git_dir =
            crate::layout::git_spelling(git_dir.to_str().context("Git directory must be UTF-8")?);
        let staged_arg =
            crate::layout::git_spelling(staged.to_str().context("staged directory must be UTF-8")?);
        let mut staged_args = vec![
            "--git-dir",
            git_dir.as_str(),
            "--work-tree",
            staged_arg.as_str(),
        ];
        staged_args.extend_from_slice(args);
        run_with_input(&staged, &staged_args, input)?;
        let predicted = capture(&staged, paths, None)?;
        self.checkpoint()?;
        for path in paths {
            self.create_parents(path)?;
        }
        let result = run_with_input(&self.root, args, input);
        let actual = capture(&self.root, paths, None)?;
        for path in before.keys().chain(predicted.keys()).chain(actual.keys()) {
            anyhow::ensure!(
                actual.get(path) == before.get(path) || actual.get(path) == predicted.get(path),
                "files changed outside this operation; preserved them and the original backup"
            );
        }
        self.expected
            .retain(|path, _| !paths.iter().any(|scope| path.starts_with(scope)));
        self.expected.extend(actual);
        result.map(|_| ())
    }

    pub(crate) fn finish<T>(mut self, result: Result<T>) -> Result<T> {
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

    fn rollback(&mut self) -> Result<()> {
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
            let original = self.index.clone();
            self.change_index(&[
                (
                    &["update-index", "--force-remove", "-z", "--stdin"],
                    &indexed,
                ),
                (&["update-index", "-z", "--index-info"], &original),
            ])?;
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
        for dir in self.created_dirs.iter().rev() {
            match std::fs::remove_dir(dir) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_error)
                    if dir
                        .read_dir()
                        .is_ok_and(|mut entries| entries.next().is_some()) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
