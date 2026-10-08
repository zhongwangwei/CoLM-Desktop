//! 远程的 Rust 引擎：把源码快照传到服务器，在上面编译 `colm-cli`、`mksrfdata-rs`、`mkinidata-rs`、`colm-rs`。
//!
//! 快照按文件内容（路径加字节）算标识，同一份源码永远是同一个标识，编好一次就一直用。服务器上的布局：
//! `<根>/engine/<标识>/src`（源码）、`<根>/engine/<标识>/bin`（四个程序），`<根>/target`（各快照共用的
//! cargo 编译目录，增量编译）。编译期会用 `include_str!` 读入 `vendor/CoLM202X` 里的文件（强迫场模板、
//! 常数表），所以快照要带上 `vendor`（第 637 轮在 T7920 上实测：不带就编译失败）。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::ssh::{quote, Ssh};

/// 远程要编的四个程序：（包，可执行文件）。
pub const BINARIES: [(&str, &str); 4] = [
    ("colm-cli", "colm-cli"),
    ("colm-srfdata", "mksrfdata-rs"),
    ("colm-init", "mkinidata-rs"),
    ("colm-runtime", "colm-rs"),
];

/// 快照包含的仓库路径（整个 workspace 加编译期读入的 `vendor`）。
pub const SNAPSHOT_PATHS: [&str; 6] = [
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "oracle",
    "xtask",
    "vendor",
];

/// 引擎源码从哪来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// 开发环境：一个 git 工作区（取受管理的文件与未被忽略的新文件，含未提交的改动）。
    Checkout(PathBuf),
    /// 安装包：随应用附带的源码包（`.tar.gz`）。
    Tarball(PathBuf),
}

impl Source {
    /// 从某个目录往上找仓库根（有 `Cargo.lock` 与 `crates/colm-cli`）。
    pub fn find_checkout(start: &Path) -> Option<Self> {
        start
            .ancestors()
            .find(|dir| dir.join("Cargo.lock").is_file() && dir.join("crates/colm-cli").is_dir())
            .map(|dir| Self::Checkout(dir.to_path_buf()))
    }
}

/// 快照：标识与要上传的文件。
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: String,
    pub source: Source,
    /// `Checkout` 时是仓库根下的相对路径。
    pub files: Vec<String>,
}

fn git_files(repo: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        // 已纳入管理的加上还没 `git add`、也没被忽略的：快照就是工作区里的样子（第 637 轮：新 crate
        // 还没纳入管理时，只取已管理的文件会让 workspace 缺成员而编译失败）。
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(SNAPSHOT_PATHS)
        .output()
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut files: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        // 已删除但还没提交的文件不在工作区里。
        .filter(|p| repo.join(p).is_file())
        .collect();
    files.sort();
    Ok(files)
}

/// 按内容算快照标识：每个文件的路径与字节依次进哈希。
pub fn content_id(base: &Path, files: &[String]) -> Result<String> {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.as_bytes());
        hasher.update([0]);
        let mut bytes = Vec::new();
        std::fs::File::open(base.join(file))
            .with_context(|| format!("cannot read {file}"))?
            .read_to_end(&mut bytes)?;
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{:x}", hasher.finalize())[..16].to_owned())
}

pub fn snapshot(source: &Source) -> Result<Snapshot> {
    match source {
        Source::Checkout(repo) => {
            let files = git_files(repo)?;
            if files.is_empty() {
                bail!("{} has no engine sources", repo.display());
            }
            Ok(Snapshot {
                id: content_id(repo, &files)?,
                source: source.clone(),
                files,
            })
        }
        Source::Tarball(path) => {
            let bytes =
                std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
            Ok(Snapshot {
                id: format!("{:x}", Sha256::digest(&bytes))[..16].to_owned(),
                source: source.clone(),
                files: Vec::new(),
            })
        }
    }
}

pub fn engine_dir(root: &str, id: &str) -> String {
    format!("{}/engine/{id}", root.trim_end_matches('/'))
}

/// 服务器上这份快照的源码在不在（在就不必再传）。
pub fn uploaded(ssh: &Ssh, root: &str, id: &str) -> Result<bool> {
    let dir = quote(&format!("{}/src/Cargo.lock", engine_dir(root, id)));
    Ok(ssh
        .run_ok(&format!("test -f {dir} && echo yes || echo no"))?
        .trim()
        == "yes")
}

/// 把快照传上去（已经在就跳过）。
pub fn upload(ssh: &Ssh, root: &str, snapshot: &Snapshot) -> Result<bool> {
    if uploaded(ssh, root, &snapshot.id)? {
        return Ok(false);
    }
    let dest = format!("{}/src", engine_dir(root, &snapshot.id));
    match &snapshot.source {
        Source::Checkout(repo) => ssh.upload_list(repo, &snapshot.files, &dest)?,
        Source::Tarball(path) => ssh.upload_tarball(path, &dest)?,
    }
    Ok(true)
}

/// 作业脚本里“确保引擎编好”的一段：没编过就加锁编译，编好把四个程序放进 `bin`。
pub fn ensure_script(root: &str, id: &str, jobs: u32) -> String {
    let engine = quote(&engine_dir(root, id));
    let target = quote(&format!("{}/target", root.trim_end_matches('/')));
    let packages: Vec<String> = BINARIES
        .iter()
        .map(|(package, binary)| format!("-p {package} --bin {binary}"))
        .collect();
    let copies: Vec<String> = BINARIES.iter().map(|(_, b)| b.to_string()).collect();
    format!(
        r#"E={engine}
if [ ! -x "$E/bin/colm-rs" ]; then
  echo "building the Rust engine" >> phase
  exec 9>"$E/.build.lock"
  flock 9
  if [ ! -x "$E/bin/colm-rs" ]; then
    C=$(command -v cargo || echo "$HOME/.cargo/bin/cargo")
    (cd "$E/src" && CARGO_TARGET_DIR={target} "$C" build --release --locked -j {jobs} {packages}) || exit 1
    mkdir -p "$E/bin"
    for b in {copies}; do cp {target}/release/$b "$E/bin/$b" || exit 1; done
  fi
  exec 9>&-
fi
"#,
        packages = packages.join(" "),
        copies = copies.join(" "),
    )
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod engine_tests;
