//! 构建期清单：认定配置身份与二进制完整性。
//!
//! `colm.x` / `mkinidata.x` / `mksrfdata.x` 都以 `getarg(1)` 取 namelist 路径，
//! **不接受其他参数，没有 `--version`**（`main/CoLM.F90:185` 等）。所以版本握手
//! 靠构建期写出的 `manifest.json`，而不是问二进制。
//!
//! 清单里两组字段职责不同。`macros` / `colm_git_sha` / `generator_args` 可复现，
//! 认定**配置身份** —— 单点模式最容易搞错的正是编译期宏集合。`sha256` 每次构建
//! 都变（实测：同一路径连跑两次，三个二进制的 sha256 全不同），只认定**完整性**：
//! 二进制自其清单写出以来未被替换。后者要求清单与二进制同生同存，不能分开分发，
//! 也不能拿一份入库的清单去校验重新构建的二进制。
//!
//! 「不存在」与「存在但对不上」是两种不同的失败，分开报 —— 用户对这两种的处置
//! 完全不同：前者是没构建，后者是构建过但被换了。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// CoLM 单点的三个可执行文件，按运行顺序。
pub const PROGRAMS: [&str; 3] = ["mksrfdata", "mkinidata", "colm"];

/// 可执行文件的后缀。**Windows 上是 `.exe`，其余是 CoLM 自己的 `.x`。**
///
/// CoLM 的 Makefile 在所有平台上都写 `.x`，`build_kernel.sh` 在 Windows 上
/// 把拷进内核目录的那份改名。为什么值得改：Windows 的 `PATHEXT` 不含 `.x`，
/// 于是这个文件在系统眼里不是「可执行文件」而是「文档」—— PowerShell 会拒绝
/// `& .\colm.x | ...`（实测报 `Cannot run a document in the middle of a
/// pipeline`），双击也不会执行，而安全软件对「带 PE 头却顶着陌生后缀」的文件
/// 往往更不客气。
///
/// 严格说程序本身不依赖这个改动：`run_stage` 用 `Command::new(绝对路径)`，
/// 走 `CreateProcessW`，对显式路径不查 `PATHEXT`。但那是推断，
/// 而按平台惯例命名之后这个问题根本不用问 —— 并且
/// `run_tests::a_real_kernel_can_actually_be_spawned` 现在把它验了。
pub const EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { ".x" };

/// 某个程序在内核目录里的文件名。**取名只有这一处**，
/// 免得校验的时候找一个名字、启动的时候找另一个。
pub fn program_file(name: &str) -> String {
    format!("{name}{EXE_SUFFIX}")
}

/// 本 crate 认识的清单版本。
pub const SCHEMA: u32 = 1;

/// `kernels/<preset>/manifest.json` 的内容。
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub preset: String,
    pub platform: String,
    pub colm_git_sha: String,
    pub generator_args: String,
    pub macros: Vec<String>,
    pub built_with: String,
    pub netcdf_c: String,
    pub netcdf_fortran: String,
    pub hdf5: String,
    /// 程序名 -> sha256。用 `BTreeMap` 让报错里的顺序稳定。
    pub sha256: BTreeMap<String, String>,
}

/// 一个已校验的内核目录。
#[derive(Debug, Clone)]
pub struct Kernel {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Kernel {
    /// 读清单并校验三个二进制。任一不符即失败。
    pub fn open(dir: &Path) -> Result<Kernel> {
        let mpath = dir.join("manifest.json");
        let text = std::fs::read_to_string(&mpath)
            .with_context(|| format!("cannot read the kernel manifest at {}", mpath.display()))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse the kernel manifest at {}", mpath.display()))?;

        if manifest.schema != SCHEMA {
            bail!(
                "the manifest at {} has schema {}, and this build understands {SCHEMA}; \
                 refusing to read it with the wrong field meanings",
                mpath.display(),
                manifest.schema
            );
        }

        for prog in PROGRAMS {
            let exe = dir.join(program_file(prog));
            if !exe.exists() {
                bail!(
                    "{} is missing from the kernel at {}; the preset has not been built",
                    program_file(prog),
                    dir.display()
                );
            }
            let want = manifest.sha256.get(prog).with_context(|| {
                format!(
                    "the manifest at {} records no sha256 for {prog}",
                    mpath.display()
                )
            })?;
            let got = sha256_file(&exe)?;
            if &got != want {
                bail!(
                    "{} does not match its manifest sha256\n  expected {want}\n  actual   {got}\n\
                     the binary has been replaced since the manifest was written; rebuild the preset",
                    program_file(prog)
                );
            }
        }

        // **绝对化**。`run_stage` 用 `current_dir(work)` 启动子进程，于是一个
        // 相对的可执行文件路径会被相对 `work` 解析，而不是相对调用方的当前
        // 目录 —— `Kernel::open("kernels/default")` 成功，随后 spawn 报
        // 「No such file or directory」。一个已校验的内核本就该持有绝对路径。
        // `absolute` 而不是 `canonicalize`：Windows 上后者返回 `\\?\C:\...`，
        // 而 `CreateProcessW` 的当前目录不接受那种路径。见 `plain`。
        let dir = absolute(dir).with_context(|| format!("cannot resolve {}", dir.display()))?;

        Ok(Kernel { dir, manifest })
    }

    /// 某一段的可执行文件路径。
    pub fn program(&self, name: &str) -> PathBuf {
        self.dir.join(program_file(name))
    }
}

/// 一段字节的 sha256，小写十六进制。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn sha256_file(p: &Path) -> Result<String> {
    let bytes =
        std::fs::read(p).with_context(|| format!("cannot read {} to hash it", p.display()))?;
    Ok(sha256_hex(&bytes))
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
pub mod manifest_tests;

/// 去掉 Windows 的 `\\?\` 扩展长度前缀。
///
/// `std::fs::canonicalize` 在 Windows 上返回的**一律**是
/// `\\?\C:\Users\...` 这种形式，而那种路径有两处用不了：
///
/// 1. **`CreateProcessW` 的当前目录不接受它。** `Command::current_dir`
///    走的正是这个参数 —— 而 `run_stage` 要把工作目录设成算例目录。
/// 2. gfortran 运行时的 `OPEN` 也未必认，而 namelist 路径是当参数传进去的。
///
/// 实测的表现：`mksrfdata` 一个字符都没打就以 `0xC0E90002` 结束，
/// 日志里 `last_line: ""`。看上去像内核坏了，其实是路径形式。
///
/// UNC 形式是 `\\?\UNC\server\share`，去掉前缀之后要还原成
/// `\\server\share` —— 直接砍掉四个字符会得到 `UNC\server\share`，
/// 那是一个相对路径，比原来更糟。
///
/// 非 Windows 平台是恒等函数：那里的 `canonicalize` 不加前缀。
pub fn plain(p: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        use std::path::PathBuf;
        if let Some(s) = p.to_str() {
            if let Some(rest) = s.strip_prefix(r"\\?\") {
                return match rest.strip_prefix("UNC\\") {
                    Some(unc) => PathBuf::from(format!(r"\\{unc}")),
                    None => PathBuf::from(rest),
                };
            }
        }
    }
    p
}

/// `canonicalize` 之后再去掉 `\\?\`。**要绝对路径的地方一律用这个**，
/// 不要直接 `canonicalize` —— 见 [`plain`]。
pub fn absolute(p: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    p.canonicalize().map(plain)
}
