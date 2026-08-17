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
            let exe = dir.join(format!("{prog}.x"));
            if !exe.exists() {
                bail!(
                    "{prog}.x is missing from the kernel at {}; the preset has not been built",
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
                    "{prog}.x does not match its manifest sha256\n  expected {want}\n  actual   {got}\n\
                     the binary has been replaced since the manifest was written; rebuild the preset"
                );
            }
        }

        Ok(Kernel {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    /// 某一段的可执行文件路径。
    pub fn program(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.x"))
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
mod manifest_tests;
