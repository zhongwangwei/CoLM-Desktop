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
pub const SNAPSHOT_PATHS: [&str; 7] = [
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "oracle",
    "xtask",
    "vendor",
    "scripts",
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

// ---- 预编的引擎（R3）：没有 cargo 或不能联网的服务器直接用，不必在那里编译 ---------------------------------------

/// 预编包的文件名：`colm-engine-linux-<arch>.tar.gz`（`arch` 是 `uname -m`：x86_64 或 aarch64）。
pub fn prebuilt_name(arch: &str) -> String {
    format!("colm-engine-linux-{arch}.tar.gz")
}

/// 本机放预编包的缓存目录（`remote-dist` 把服务器上编好的取回到这里）。
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COLM_ENGINE_CACHE") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Caches")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache"))
    };
    base.join("edu.sysu.colm.desktop").join("engines")
}

/// 缓存里与某份源码快照对应的预编包文件名。带快照标识，免得源码改了还在用旧程序。
pub fn cached_name(arch: &str, snapshot_id: &str) -> String {
    format!("colm-engine-linux-{arch}-{snapshot_id}.tar.gz")
}

/// 一个预编包：标识按包内容算，与源码无关。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prebuilt {
    pub id: String,
    pub tarball: PathBuf,
}

impl Prebuilt {
    pub fn open(tarball: &Path) -> Result<Self> {
        let bytes =
            std::fs::read(tarball).with_context(|| format!("cannot read {}", tarball.display()))?;
        Ok(Self {
            id: format!("pre-{}", &format!("{:x}", Sha256::digest(&bytes))[..16]),
            tarball: tarball.to_path_buf(),
        })
    }
}

/// 安装包里放引擎材料（源码包、预编包）的目录，按优先级：GUI 告诉的资源目录（环境变量
/// `COLM_RESOURCE_DIR`）、`colm-cli` 旁边、macOS 的 `../Resources`；每处都找 `engine/` 子目录与目录本身。
pub fn resource_dirs(exe_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("COLM_RESOURCE_DIR") {
        roots.push(PathBuf::from(dir));
    }
    roots.push(exe_dir.to_path_buf());
    roots.push(exe_dir.join("../Resources"));
    roots
        .into_iter()
        .flat_map(|root| [root.join("engine"), root])
        .collect()
}

/// 安装包随附的源码包 `colm-src.tar.gz`（`engine-pack` 在打包时生成）。
pub fn bundled_source(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join("colm-src.tar.gz"))
        .find(|p| p.is_file())
}

/// 找一个适合 `arch` 的预编包。顺序：
/// 1. 应用安装包里随附的（与随附的源码同一版本，不核对快照）；
/// 2. 缓存里与当前源码快照一致的那份（开发时：源码一改，旧包就不再匹配）。
pub fn find_prebuilt(arch: &str, snapshot_id: Option<&str>, dirs: &[PathBuf]) -> Option<Prebuilt> {
    let mut candidates: Vec<PathBuf> = dirs.iter().map(|d| d.join(prebuilt_name(arch))).collect();
    if let Some(id) = snapshot_id {
        candidates.push(cache_dir().join(cached_name(arch, id)));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| Prebuilt::open(&p).ok())
}

/// 把源码快照打成 `colm-src.tar.gz`（根目录就是仓库根），随安装包附带：没有仓库的机器也能把源码传到服务器上编。
pub fn pack_source(snapshot: &Snapshot, out: &Path) -> Result<()> {
    let Source::Checkout(repo) = &snapshot.source else {
        bail!("only a checkout can be packed");
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let list = std::env::temp_dir().join(format!("colm-pack-{}.txt", std::process::id()));
    std::fs::write(&list, snapshot.files.join("\n") + "\n")?;
    let status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-czf")
        .arg(out)
        .arg("-C")
        .arg(repo)
        .arg("-T")
        .arg(&list)
        .status()
        .context("cannot run tar")?;
    let _ = std::fs::remove_file(&list);
    if !status.success() {
        bail!("tar failed while packing the engine sources");
    }
    Ok(())
}

/// 服务器上这个预编包已经在不在。
pub fn prebuilt_present(ssh: &Ssh, root: &str, id: &str) -> Result<bool> {
    let exe = quote(&format!("{}/bin/colm-rs", engine_dir(root, id)));
    Ok(ssh
        .run_ok(&format!("test -x {exe} && echo yes || echo no"))?
        .trim()
        == "yes")
}

/// 把预编包传上去解开（已经在就跳过）。
pub fn upload_prebuilt(ssh: &Ssh, root: &str, prebuilt: &Prebuilt) -> Result<bool> {
    if prebuilt_present(ssh, root, &prebuilt.id)? {
        return Ok(false);
    }
    ssh.upload_tarball(&prebuilt.tarball, &engine_dir(root, &prebuilt.id))?;
    if !prebuilt_present(ssh, root, &prebuilt.id)? {
        bail!(
            "the prebuilt engine was uploaded but bin/colm-rs is not there; the package is damaged"
        );
    }
    Ok(true)
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod engine_tests;
