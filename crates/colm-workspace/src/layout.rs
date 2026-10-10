//! 工作区的布局与生命周期（docs/design-ai-assistant.md 第 5.1 节）：
//!
//! ```text
//! <根>/<名字>/        默认根 ~/CoLM-Workspaces/
//!   src/              git 仓库，分支 ws/<名字>
//!   target/           cargo 的编译目录（放在 src 外，免得弄脏仓库）
//!   kernels/<预设>/   本工作区编出的 Fortran 内核（含 manifest.json）
//!   bin/              本工作区编出的 colm-cli、mksrfdata-rs、mkinidata-rs、colm-rs
//!   runs/<编号>/      run_case_with 与对照运行的算例副本和输出
//!   reports/          编译日志、compare_outputs、parity_check、测试的报告
//!   workspace.json    来源、基线提交、各道门的状态、采纳记录
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::gates::Gates;
use crate::git;

pub const INFO_FILE: &str = "workspace.json";

/// 默认根目录：环境变量 `COLM_WORKSPACES`，否则 `~/CoLM-Workspaces`。
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("COLM_WORKSPACES") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("CoLM-Workspaces")
}

/// 名字：字母数字开头，字母数字与 `_-`，最长 40 个字符（拼进路径与分支名前先挡住 `..` 之类）。
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ensure!(
        ok,
        "workspace names are 1-40 characters: letters, digits, _ and -, starting with a letter or digit (got {name:?})"
    );
    Ok(())
}

/// 一次采纳类操作的记录（D 级：只有人能做）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adoption {
    pub at: u64,
    pub action: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub name: String,
    pub created_at: u64,
    /// 从哪来：本地仓库路径、仓库地址或源码包。
    pub origin: String,
    /// 建立工作区时的提交（完整哈希）；改动都是它之后的提交。
    pub base_commit: String,
    pub branch: String,
    #[serde(default)]
    pub gates: Gates,
    #[serde(default)]
    pub adopted: Vec<Adoption>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub dir: PathBuf,
    pub info: Info,
}

/// 列表里的一行。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub name: String,
    pub dir: PathBuf,
    pub origin: String,
    pub head: String,
    pub commits: usize,
    pub dirty: bool,
    pub lights: crate::gates::Lights,
    pub created_at: u64,
}

impl Workspace {
    pub fn src(&self) -> PathBuf {
        self.dir.join("src")
    }
    pub fn target(&self) -> PathBuf {
        self.dir.join("target")
    }
    pub fn bin(&self) -> PathBuf {
        self.dir.join("bin")
    }
    pub fn kernels(&self) -> PathBuf {
        self.dir.join("kernels")
    }
    pub fn runs(&self) -> PathBuf {
        self.dir.join("runs")
    }
    pub fn reports(&self) -> PathBuf {
        self.dir.join("reports")
    }

    pub fn head(&self) -> Result<String> {
        git::head(&self.src())
    }

    pub fn save(&self) -> Result<()> {
        write_info(&self.dir, &self.info)
    }

    /// 改 `workspace.json`：持锁、**重新读盘**、应用改动、原子写回。几条命令同时结束（编引擎与编内核常常
    /// 同时跑）时，各自只改自己那一项，谁也不会把别人刚写的记录覆盖掉。
    pub fn update(&mut self, change: impl FnOnce(&mut Info)) -> Result<()> {
        let _lock = InfoLock::acquire(&self.dir)?;
        let text = std::fs::read_to_string(self.dir.join(INFO_FILE))
            .with_context(|| format!("cannot read {INFO_FILE} of {}", self.dir.display()))?;
        let mut info: Info = serde_json::from_str(&text)?;
        change(&mut info);
        write_info(&self.dir, &info)?;
        self.info = info;
        Ok(())
    }

    pub fn open(root: &Path, name: &str) -> Result<Self> {
        validate_name(name)?;
        let dir = root.join(name);
        let text = std::fs::read_to_string(dir.join(INFO_FILE))
            .with_context(|| format!("{} is not a workspace (no {INFO_FILE})", dir.display()))?;
        Ok(Self {
            dir,
            info: serde_json::from_str(&text)?,
        })
    }

    /// 从 `from` 建一个工作区。`from` 可以是：
    /// - 本地 git 仓库目录（`git clone --local`，几乎不占额外空间）；
    /// - 仓库地址（`https://`、`ssh://`、`git@`）；
    /// - 源码包（`.tar.gz`，应用随附的 `colm-src.tar.gz`；没有历史，建一个基线提交）。
    ///
    /// `rev` 是标签、分支或提交；不给就用来源的当前提交。
    pub fn create(root: &Path, name: &str, from: &str, rev: Option<&str>) -> Result<Self> {
        validate_name(name)?;
        let dir = root.join(name);
        ensure!(
            !dir.exists(),
            "workspace {name} already exists at {}",
            dir.display()
        );
        std::fs::create_dir_all(root)
            .with_context(|| format!("cannot create {}", root.display()))?;
        // Only the caller that reserves this directory may clean up a failed creation.
        std::fs::create_dir(&dir).with_context(|| format!("cannot reserve {}", dir.display()))?;
        match Self::populate(&dir, name, from, rev) {
            Ok(workspace) => Ok(workspace),
            Err(error) => {
                // 半成品不留下。
                let _ = std::fs::remove_dir_all(&dir);
                Err(error)
            }
        }
    }

    fn populate(dir: &Path, name: &str, from: &str, rev: Option<&str>) -> Result<Self> {
        let src = dir.join("src");
        let is_tarball = from.ends_with(".tar.gz") || from.ends_with(".tgz");
        let is_url =
            from.starts_with("https://") || from.starts_with("ssh://") || from.starts_with("git@");
        if is_tarball {
            ensure!(Path::new(from).is_file(), "{from} is not a file");
            std::fs::create_dir_all(&src)?;
            let status = Command::new("tar")
                .arg("-xzf")
                .arg(from)
                .arg("-C")
                .arg(&src)
                .status()
                .context("cannot run tar")?;
            ensure!(status.success(), "cannot unpack {from}");
            git::run(&src, &["init", "-q"])?;
            git::set_identity(&src)?;
            git::run(&src, &["add", "-A"])?;
            git::run(&src, &["commit", "-q", "-m", &format!("base: {from}")])?;
        } else if is_url {
            clone(from, &src, false)?;
        } else {
            ensure!(
                Path::new(from).join(".git").exists(),
                "{from} is not a git repository, a repository URL or a .tar.gz source package"
            );
            clone(from, &src, true)?;
        }
        git::set_identity(&src)?;
        if let Some(rev) = rev {
            ensure!(
                !rev.starts_with('-') && !rev.contains(char::is_whitespace),
                "bad revision {rev:?}"
            );
            git::run(&src, &["checkout", "-q", "--detach", rev])
                .with_context(|| format!("cannot check out {rev}"))?;
        }
        let branch = format!("ws/{name}");
        git::run(&src, &["checkout", "-q", "-B", &branch])?;
        let base = git::head(&src)?;
        for sub in ["kernels", "bin", "runs", "reports"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        let workspace = Self {
            dir: dir.to_path_buf(),
            info: Info {
                name: name.to_owned(),
                created_at: now(),
                origin: from.to_owned(),
                base_commit: base,
                branch,
                gates: Gates::default(),
                adopted: Vec::new(),
            },
        };
        workspace.save()?;
        Ok(workspace)
    }

    /// 相对基线的改动里，有没有会影响计算结果的代码（决定“两版一致”要不要做）。
    pub fn parity_needed(&self) -> Result<bool> {
        let changed = git::changed_files(&self.src(), &self.info.base_commit)?;
        Ok(crate::gates::parity_needed(
            changed.iter().map(|f| f.path.as_str()),
        ))
    }

    pub fn summary(&self) -> Result<Summary> {
        let src = self.src();
        let head = git::head(&src)?;
        Ok(Summary {
            name: self.info.name.clone(),
            dir: self.dir.clone(),
            origin: self.info.origin.clone(),
            commits: git::commits_since(&src, &self.info.base_commit)?.len(),
            dirty: git::is_dirty(&src)?,
            lights: self.info.gates.lights_for(&head, self.parity_needed()?),
            head,
            created_at: self.info.created_at,
        })
    }

    pub fn list(root: &Path) -> Result<Vec<Summary>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(root) else {
            return Ok(out);
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_name(&name).is_err() || !entry.path().join(INFO_FILE).is_file() {
                continue;
            }
            if let Ok(summary) = Self::open(root, &name).and_then(|w| w.summary()) {
                out.push(summary);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// 删除工作区（D 级：只在界面上由人触发）。只删根目录下带 `workspace.json` 的那个目录。
    pub fn delete(root: &Path, name: &str) -> Result<()> {
        validate_name(name)?;
        let dir = root.join(name);
        ensure!(
            dir.join(INFO_FILE).is_file(),
            "{} is not a workspace; nothing was deleted",
            dir.display()
        );
        let canonical = dir.canonicalize()?;
        let root = root.canonicalize()?;
        ensure!(
            canonical.parent() == Some(root.as_path()),
            "{} is not directly under the workspace root",
            dir.display()
        );
        std::fs::remove_dir_all(&canonical)
            .with_context(|| format!("cannot delete {}", canonical.display()))
    }

    /// 记一次采纳类操作。
    pub fn record_adoption(&mut self, action: &str, detail: &str) -> Result<()> {
        let adoption = Adoption {
            at: now(),
            action: action.to_owned(),
            detail: detail.to_owned(),
        };
        self.update(|info| info.adopted.push(adoption))
    }
}

/// 先写临时文件再改名：别的进程读到的要么是旧的、要么是新的完整文件。
fn write_info(dir: &Path, info: &Info) -> Result<()> {
    let path = dir.join(INFO_FILE);
    let tmp = dir.join(format!("{INFO_FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(info)?)
        .with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot replace {}", path.display()))
}

/// `workspace.json` 的锁文件：独占创建，用完删除；超过一分钟的当作崩溃留下的残余清掉。
struct InfoLock(PathBuf);

impl InfoLock {
    fn acquire(dir: &Path) -> Result<Self> {
        let path = dir.join("workspace.lock");
        let started = std::time::Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                // Windows 上锁文件刚被另一个持有者删掉、还处在“等待删除”时，`create_new`
                // 报的是拒绝访问（os error 5）而不是已存在：同样是有人占着，等一下再试。
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        || (cfg!(windows)
                            && error.kind() == std::io::ErrorKind::PermissionDenied) =>
                {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > std::time::Duration::from_secs(60));
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    ensure!(
                        started.elapsed() < std::time::Duration::from_secs(15),
                        "{} is locked by another command",
                        dir.display()
                    );
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => return Err(error).context("cannot lock the workspace"),
            }
        }
    }
}

impl Drop for InfoLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn clone(from: &str, src: &Path, local: bool) -> Result<()> {
    let mut command = Command::new("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("clone")
        .arg("-q");
    if local {
        command.arg("--local");
    }
    let output = command
        .arg("--")
        .arg(from)
        .arg(src)
        .output()
        .context("cannot run git (is it installed?)")?;
    if !output.status.success() {
        bail!(
            "git clone {from} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "layout_tests.rs"]
pub(crate) mod layout_tests;
