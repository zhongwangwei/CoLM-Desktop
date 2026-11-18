//! 在工作区里直接改文件：浏览、读、写文本、用本机的文件或文件夹替换、删除。
//!
//! 原来改代码只能导入补丁或让助手改：自己的 `MOD_*.F90` 要整个换掉、要放一张查找表（二进制），
//! 都得先在外面做成补丁，二进制根本放不进去（2026-10-20 梳理"新建修改版"）。这里每次改动都是一次提交，
//! 路径规则和补丁完全一样（[`crate::patch::check_path`]：不出 `src/`、不碰 `.git` 与黄金参考），
//! 提交前要求工作区干净；写入路径上不许有符号链接，免得经链接写到工作区外面。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::git;
use crate::layout::Workspace;
use crate::patch::{check_path, Applied};

/// 在界面里打开编辑的文本文件上限。
pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
/// 用本机文件替换时单个文件的上限（可以是二进制，比如查找表）。
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// 用本机文件夹替换时整棵目录的上限。
pub const MAX_TREE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TREE_FILES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    /// file、dir 或 link。
    pub kind: &'static str,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileText {
    pub path: String,
    pub size: u64,
    /// 不是 UTF-8 文本（或含 NUL）时为空：界面只提供替换，不打开编辑。
    pub text: Option<String>,
}

/// 工作区里可编辑文件的内容。比只读版本浏览用的 [`FileText`] 多一个内容哈希：冻结版本是只读的，
/// 那时不需要基线；能在界面里保存的文件才需要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditableFile {
    pub path: String,
    pub size: u64,
    pub text: Option<String>,
    /// 内容 sha256（十六进制）。只有真的读出内容时才有：超过 `MAX_TEXT_BYTES` 的文件是为了省内存
    /// 才不读的，浏览目录不该为了拿哈希把几百 MB 的二进制读一遍。界面用它当保存基线。
    pub hash: Option<String>,
}

/// 写入成功的结果：这次提交 + 新内容哈希。
///
/// 带上新哈希是给界面更新基线的：保存成功后如果还要再读一次文件，中间任何外部写入都会让界面
/// 拿着旧基线继续，下一次保存就会被自己刚写的内容误判成冲突。写入端算出的哈希与落盘内容同源，
/// 没有这个窗口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Written {
    #[serde(flatten)]
    pub applied: Applied,
    pub hash: String,
}

/// 内容身份：sha256 十六进制。用哈希而不是修改时间/大小——外部编辑器保存一次时间戳必变、大小却
/// 常常不变；同一内容重新保存时哈希不变，界面可以据此判断"其实没变"，不弹无意义的冲突。
fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    // 分块读：文件最大可以到 MAX_FILE_BYTES（64 MB），一次性读进来只为算哈希不值得。
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 写入前的文本限制：与界面里能编辑的上限一致，更大的文件走"用本机文件替换"。
fn check_text(text: &str) -> Result<()> {
    ensure!(
        text.len() as u64 <= MAX_TEXT_BYTES,
        "the text is larger than {MAX_TEXT_BYTES} bytes; replace the file from this computer instead"
    );
    ensure!(!text.contains('\0'), "text files cannot contain NUL bytes");
    Ok(())
}

/// 保存前在**写入端**核对基线：磁盘现在的哈希必须等于界面打开时拿到的哈希。
///
/// 不能只靠界面比较：工作区快照会把外部改动记成提交，界面里看到的列表因此更新了，但文本框里
/// 还是旧内容；只有写之前再看一眼磁盘，才能保证不用旧缓冲区覆盖别人的修改。这里拒绝的是整次
/// 写入——文件一个字节都不动，也不提交。`content-conflict:` 是稳定标记，界面按它换成双语提示。
fn check_expected(file: &Path, path: &str, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    match std::fs::symlink_metadata(file) {
        Ok(meta) => ensure!(!meta.is_dir(), "{path} is a directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "content-conflict: {path} was deleted or replaced outside the app since it was opened; reload it or save it again explicitly"
        ),
        Err(error) => return Err(error.into()),
    }
    let actual = sha256_file(file)?;
    ensure!(
        actual == expected,
        "content-conflict: {path} changed on disk since it was opened; reload it or overwrite explicitly"
    );
    Ok(())
}

fn check_message(message: &str) -> Result<String> {
    let message = message.trim();
    ensure!(
        !message.is_empty() && message.chars().count() <= 200 && !message.contains('\n'),
        "the change message must be one short line"
    );
    Ok(message.to_owned())
}

/// 工作区里的一个路径：规则同补丁；已存在的每一段都不能是符号链接。
fn target(workspace: &Workspace, path: &str) -> Result<PathBuf> {
    crate::source_links::check_edit(&workspace.src(), path)?;
    physical_target(workspace, path)
}

fn physical_target(workspace: &Workspace, path: &str) -> Result<PathBuf> {
    check_path(path)?;
    let mut current = workspace.src();
    for part in path.split('/') {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "{path} goes through a symbolic link; change the file it points to instead"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

fn clean(workspace: &Workspace) -> Result<()> {
    ensure!(
        !git::is_dirty(&workspace.src())?,
        "the workspace has uncommitted changes; commit or revert them first"
    );
    Ok(())
}

// Canonicalize existing ancestors too: a new destination can still be inside the source.
fn physical_identity(path: &Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("path has no existing ancestor")?;
            Ok(physical_identity(parent)?.join(path.file_name().context("path has no name")?))
        }
        Err(error) => Err(error.into()),
    }
}

fn check_old_subtree(workspace: &Workspace, path: &str) -> Result<()> {
    check_path(path)?;
    let item = workspace.src().join(path);
    if std::fs::symlink_metadata(&item).is_ok_and(|meta| meta.is_dir()) {
        for entry in std::fs::read_dir(item)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("file names must be UTF-8")?;
            check_old_subtree(workspace, &format!("{path}/{name}"))?;
        }
    }
    Ok(())
}

fn commit(
    workspace: &Workspace,
    path: &str,
    message: &str,
    edit: &mut git::Edit,
) -> Result<Applied> {
    let src = workspace.src();
    let mut files = vec![path.to_owned()];
    files.extend(crate::source_links::synchronize(&src, edit)?);
    edit.checkpoint()?;
    let mut add = vec!["--literal-pathspecs", "add", "-A", "-f", "--"];
    add.extend(files.iter().map(String::as_str));
    edit.stage(&add)?;
    let message = format!("ws: {message}");
    let mut commit = vec![
        "--literal-pathspecs",
        "commit",
        "-q",
        "--only",
        "-m",
        &message,
        "--",
    ];
    commit.extend(files.iter().map(String::as_str));
    edit.checkpoint()?;
    git::run(&src, &commit).context("nothing changed")?;
    let commit = git::head(&src)?;
    Ok(Applied {
        short: git::short(&src, &commit)?,
        commit,
        files,
    })
}

/// 编辑器自己留下的东西：不是对模型的修改，记进快照会被当成"相对原版改了"。写进仓库自己的
/// `.git/info/exclude`（只对本地生效，不进版本库）。
const EDITOR_LEFTOVERS: [&str; 9] = [
    ".vscode/",
    ".idea/",
    ".DS_Store",
    "Thumbs.db",
    "*.swp",
    "*.swo",
    "*~",
    "*.orig",
    "*.bak",
];

fn exclude_editor_leftovers(src: &Path) -> Result<()> {
    let path = src.join(".git/info/exclude");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .context("cannot read local Git ignore rules; original file preserved")
        }
    };
    let missing = EDITOR_LEFTOVERS
        .iter()
        .filter(|pattern| !existing.lines().any(|line| line.trim() == **pattern))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# CoLM Desktop：外部编辑器留下的文件不算对模型的修改\n");
    for pattern in missing {
        text.push_str(pattern);
        text.push('\n');
    }
    std::fs::write(&path, text)?;
    Ok(())
}

/// 用本地软件（VS Code、记事本……）在工作区里改过的文件，记成一次提交，于是和原版的差异、编译与
/// 冻结都看得到它们。没改什么就返回 `None`。规则与界面里改同：不能碰黄金参考与 `.git`；
/// 被 `.gitignore` 忽略的编译产物不记。
pub fn snapshot_external(workspace: &Workspace) -> Result<Option<Applied>> {
    let _guard = git::WriteLock::acquire(workspace)?;
    let src = workspace.src();
    exclude_editor_leftovers(&src)?;
    let status = git::run(
        &src,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    )?;
    let mut files = Vec::new();
    let mut entries = status.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        let Some(path) = entry.get(3..) else {
            continue;
        };
        // Porcelain -z puts the source after the destination for renames and copies.
        if entry.as_bytes()[..2]
            .iter()
            .any(|status| matches!(status, b'R' | b'C'))
        {
            let source = entries
                .next()
                .context("missing rename source in git status")?;
            check_path(source).with_context(|| format!("{source} was changed outside the app"))?;
            files.push(source.to_owned());
        }
        check_path(path).with_context(|| format!("{path} was changed outside the app"))?;
        files.push(path.to_owned());
    }
    if files.is_empty() {
        return Ok(None);
    }
    let mut edit = git::Edit::begin(workspace, files.clone())?;
    let result = (|| {
        files.extend(crate::source_links::synchronize(&src, &mut edit)?);
        edit.checkpoint()?;
        // Already staged deletions/rename sources have no index entry left for `git add`.
        let tracked = git::run(&src, &["ls-files", "-z"])?;
        let tracked: std::collections::HashSet<_> = tracked.split('\0').collect();
        let to_add: Vec<_> = files
            .iter()
            .filter(|path| {
                std::fs::symlink_metadata(src.join(path)).is_ok() || tracked.contains(path.as_str())
            })
            .map(String::as_str)
            .collect();
        if !to_add.is_empty() {
            let mut add = vec!["--literal-pathspecs", "add", "-A", "--"];
            add.extend(to_add);
            edit.stage(&add)?;
        }
        // Recheck the actual index too; an external Git client can stage additional paths.
        let staged = git::run(
            &src,
            &["diff", "--cached", "--no-renames", "--name-only", "-z"],
        )?;
        let selected = files;
        let mut files = Vec::new();
        for path in staged.split('\0').filter(|path| !path.is_empty()) {
            ensure!(
                selected.iter().any(|selected| selected == path),
                "the Git index changed during the snapshot; retry after reviewing external changes"
            );
            if let Err(error) = if path == colm_remote::engine::SOURCE_LINKS_FILE {
                Ok(())
            } else {
                check_path(path)
            } {
                return Err(error.context(format!("{path} was changed outside the app")));
            }
            files.push(path.to_owned());
        }
        if files.is_empty() {
            return Ok(None);
        }
        let shown = files.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        let more = files.len().saturating_sub(5);
        let message = if more > 0 {
            format!("ws: external edit of {shown} and {more} more")
        } else {
            format!("ws: external edit of {shown}")
        };
        let mut commit = vec![
            "--literal-pathspecs",
            "commit",
            "-q",
            "--only",
            "-m",
            &message,
            "--",
        ];
        commit.extend(files.iter().map(String::as_str));
        edit.checkpoint()?;
        git::run(&src, &commit)?;
        let commit = git::head(&src)?;
        Ok(Some(Applied {
            short: git::short(&src, &commit)?,
            commit,
            files,
        }))
    })();
    edit.finish(result)
}

/// 列出一个目录（空串是源码根目录）；不列 `.git`。
pub fn list(workspace: &Workspace, dir: &str) -> Result<Vec<Entry>> {
    let base = if dir.is_empty() {
        workspace.src()
    } else {
        target(workspace, dir)?
    };
    ensure!(base.is_dir(), "{dir} is not a directory");
    let links = colm_remote::engine::source_links(&workspace.src())?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&base)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if dir.is_empty() && (name == ".git" || name == colm_remote::engine::SOURCE_LINKS_FILE) {
            continue;
        }
        let meta = std::fs::symlink_metadata(entry.path())?;
        let kind =
            if meta.file_type().is_symlink() || links.contains_key(&Path::new(dir).join(&name)) {
                "link"
            } else if meta.is_dir() {
                "dir"
            } else {
                "file"
            };
        entries.push(Entry {
            path: if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            },
            name,
            kind,
            size: if kind == "file" { meta.len() } else { 0 },
        });
    }
    entries.sort_by(|a, b| (a.kind != "dir", &a.name).cmp(&(b.kind != "dir", &b.name)));
    Ok(entries)
}

/// 读一个文件，给界面编辑。
pub fn read(workspace: &Workspace, path: &str) -> Result<EditableFile> {
    let file = target(workspace, path)?;
    let meta = std::fs::metadata(&file).with_context(|| format!("{path} does not exist"))?;
    ensure!(meta.is_file(), "{path} is not a file");
    let (text, hash) = if meta.len() <= MAX_TEXT_BYTES {
        let bytes = std::fs::read(&file)?;
        let hash = sha256_bytes(&bytes);
        let text = (!bytes.contains(&0))
            .then(|| String::from_utf8(bytes).ok())
            .flatten();
        (text, Some(hash))
    } else {
        (None, None)
    };
    Ok(EditableFile {
        path: path.to_owned(),
        size: meta.len(),
        text,
        hash,
    })
}

/// 写一个文本文件（新建或覆盖）并提交。`expected_hash` 是界面打开文件时拿到的内容哈希：给了就必须
/// 与磁盘现在的哈希一致，否则整次拒绝（见 [`check_expected`]）。
pub fn write_text(
    workspace: &Workspace,
    path: &str,
    text: &str,
    message: &str,
    expected_hash: Option<&str>,
) -> Result<Written> {
    let _guard = git::WriteLock::acquire(workspace)?;
    let message = check_message(message)?;
    check_text(text)?;
    let file = target(workspace, path)?;
    ensure!(!file.is_dir(), "{path} is a directory");
    // 先比基线再要求工作区干净：外部编辑还没被快照记成提交时，工作区本来就是脏的；这时报"先提交"
    // 会让用户莫名其妙，而"文件在软件外被改了"才是他需要处理的事（GUI-03）。
    check_expected(&file, path, expected_hash)?;
    clean(workspace)?;
    let mut edit = git::Edit::begin(workspace, vec![path.to_owned()])?;
    let result = (|| {
        check_expected(&file, path, expected_hash)?;
        edit.create_parents(path)?;
        edit.replace_file(path, &mut text.as_bytes(), None)?;
        let applied = commit(workspace, path, &message, &mut edit)?;
        Ok(Written {
            applied,
            hash: sha256_bytes(text.as_bytes()),
        })
    })();
    edit.finish(result)
}

/// 原子新建一个文本文件并提交：目标已存在就拒绝，原文件一个字节都不动。
///
/// 不能用"先查 exists 再写"：两次操作之间别的进程/另一个界面实例可能刚好创建同名文件，先查再写
/// 就会把它覆盖。`OpenOptions::create_new` 把"存在性检查 + 创建"合成一次系统调用，并发下至多
/// 一个成功——这是 GUI-01 要求在后端保证的部分，前端列表只是提前提示。
pub fn create_text(
    workspace: &Workspace,
    path: &str,
    text: &str,
    message: &str,
) -> Result<Written> {
    let _guard = git::WriteLock::acquire(workspace)?;
    let message = check_message(message)?;
    check_text(text)?;
    let file = target(workspace, path)?;
    clean(workspace)?;
    let mut edit = git::Edit::begin(workspace, vec![path.to_owned()])?;
    let result = (|| {
        edit.create_parents(path)?;
        let mut handle = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
    {
        Ok(handle) => handle,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => bail!(
            "already-exists: {path} already exists in the workspace; open it instead, or replace it explicitly"
        ),
        Err(error) => return Err(error.into()),
    };
        edit.record(&[path.to_owned()])?;
        edit.change(
            &[path.to_owned()],
            || Ok(handle.write_all(text.as_bytes())?),
        )?;
        drop(handle);
        let applied = commit(workspace, path, &message, &mut edit)?;
        Ok(Written {
            applied,
            hash: sha256_bytes(text.as_bytes()),
        })
    })();
    edit.finish(result)
}

/// 用本机的一个文件替换（或新增）工作区里的文件；可以是二进制。`create_only` 为真时只允许新建，
/// 同名文件已存在就拒绝——"从本机加入文件"不能以新增的名义覆盖别人的文件。
pub fn put_file(
    workspace: &Workspace,
    path: &str,
    from: &Path,
    message: &str,
    create_only: bool,
) -> Result<Applied> {
    let _guard = git::WriteLock::acquire(workspace)?;
    let message = check_message(message)?;
    let meta = std::fs::symlink_metadata(from)
        .with_context(|| format!("cannot read {}", from.display()))?;
    ensure!(meta.is_file(), "{} is not a regular file", from.display());
    ensure!(
        meta.len() <= MAX_FILE_BYTES,
        "{} is larger than {MAX_FILE_BYTES} bytes",
        from.display()
    );
    let file = target(workspace, path)?;
    ensure!(
        !file.is_dir(),
        "{path} is a directory; replace it with a folder instead"
    );
    ensure!(
        physical_identity(from)? != physical_identity(&file)?,
        "source and destination are the same file"
    );
    #[cfg(unix)]
    if file.exists() {
        use std::os::unix::fs::MetadataExt;
        let destination = std::fs::metadata(&file)?;
        ensure!(
            (meta.dev(), meta.ino()) != (destination.dev(), destination.ino()),
            "source and destination are the same file"
        );
    }
    // Windows stable std has no file ID API. Deny writes to the source through any hardlink.
    #[cfg(windows)]
    let source_guard = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(from)?
    };
    clean(workspace)?;
    let mut edit = git::Edit::begin(workspace, vec![path.to_owned()])?;
    let result = (|| {
        edit.create_parents(path)?;
        if create_only {
            let mut source = std::fs::File::open(from)
                .with_context(|| format!("cannot read {}", from.display()))?;
            let mut destination = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
        {
            Ok(destination) => destination,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => bail!(
                "already-exists: {path} already exists in the workspace; add it under another name, open it instead, or replace it explicitly"
            ),
            Err(error) => return Err(error.into()),
        };
            edit.record(&[path.to_owned()])?;
            edit.change(&[path.to_owned()], || {
                std::io::copy(&mut source, &mut destination)
                    .with_context(|| format!("cannot copy {}", from.display()))
            })?;
            drop(destination);
            #[cfg(windows)]
            drop(source_guard);
            return commit(workspace, path, &message, &mut edit);
        }
        edit.replace_file(
            path,
            &mut std::fs::File::open(from)?,
            Some(meta.permissions()),
        )?;
        #[cfg(windows)]
        drop(source_guard);
        commit(workspace, path, &message, &mut edit)
    })();
    edit.finish(result)
}

/// 本机文件夹里要复制的文件；超过上限或含符号链接就拒绝，一个字节都不写。
fn tree_files(from: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![PathBuf::new()];
    while let Some(relative) = stack.pop() {
        for entry in std::fs::read_dir(from.join(&relative))? {
            let entry = entry?;
            let meta = std::fs::symlink_metadata(entry.path())?;
            let name = relative.join(entry.file_name());
            ensure!(
                !meta.file_type().is_symlink(),
                "{} is a symbolic link; replace folders without links",
                entry.path().display()
            );
            ensure!(
                entry.file_name() != ".git",
                "{} contains a .git folder",
                from.display()
            );
            if meta.is_dir() {
                stack.push(name);
            } else {
                total += meta.len();
                files.push(name);
                ensure!(
                    files.len() <= MAX_TREE_FILES && total <= MAX_TREE_BYTES,
                    "{} has more than {MAX_TREE_FILES} files or {MAX_TREE_BYTES} bytes",
                    from.display()
                );
            }
        }
    }
    Ok(files)
}

/// 用本机的一个文件夹整个替换工作区里的目录（比如换掉一个物理模块）；原目录里的东西全部换掉。
pub fn put_dir(workspace: &Workspace, path: &str, from: &Path, message: &str) -> Result<Applied> {
    let _guard = git::WriteLock::acquire(workspace)?;
    let message = check_message(message)?;
    ensure!(
        std::fs::symlink_metadata(from).is_ok_and(|meta| meta.is_dir()),
        "{} is not a folder",
        from.display()
    );
    let files = tree_files(from)?;
    let dir = target(workspace, path)?;
    ensure!(
        !dir.is_file(),
        "{path} is a file; replace it with a file instead"
    );
    let source = physical_identity(from)?;
    let destination = physical_identity(&dir)?;
    ensure!(
        !source.starts_with(&destination) && !destination.starts_with(&source),
        "source and destination folders overlap"
    );
    check_old_subtree(workspace, path)?;
    for file in &files {
        let relative = file.to_str().context("file names must be UTF-8")?;
        check_path(&format!("{path}/{}", relative.replace('\\', "/")))?;
    }
    clean(workspace)?;
    let mut edit = git::Edit::begin(workspace, vec![path.to_owned()])?;
    let result = (|| {
        let directories = crate::source_links::check_removal(&workspace.src(), path, Some(from))?;
        edit.create_parents(path)?;
        edit.change(&[path.to_owned()], || {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            for directory in directories {
                std::fs::create_dir_all(dir.join(directory))?;
            }
            for file in &files {
                let destination = dir.join(file);
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(from.join(file), &destination)?;
            }
            std::fs::create_dir_all(&dir)?;
            Ok(())
        })?;
        commit(workspace, path, &message, &mut edit)
    })();
    edit.finish(result)
}

/// 删除工作区里的一个文件或目录并提交。
pub fn remove(workspace: &Workspace, path: &str, message: &str) -> Result<Applied> {
    let _guard = git::WriteLock::acquire(workspace)?;
    let message = check_message(message)?;
    let item = physical_target(workspace, path)?;
    check_old_subtree(workspace, path)?;
    clean(workspace)?;
    let mut edit = git::Edit::begin(workspace, vec![path.to_owned()])?;
    let result = (|| {
        crate::source_links::check_removal(&workspace.src(), path, None)?;
        edit.change(&[path.to_owned()], || {
            match std::fs::symlink_metadata(&item) {
                Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&item)?,
                Ok(_) => std::fs::remove_file(&item)?,
                Err(_) => bail!("{path} does not exist"),
            }
            Ok(())
        })?;
        commit(workspace, path, &message, &mut edit)
    })();
    edit.finish(result)
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod files_tests;
