//! 把 `colm-cli` 暂存到 Tauri 打包要找的位置。
//!
//! Tauri 的 `bundle.externalBin` 要求文件名带**目标三元组**后缀
//! （`colm-cli-aarch64-apple-darwin`），打包时它按当前目标去找。
//!
//! 这一步用 xtask 做而不是 Node 脚本：EarthMesh 用 Node 是因为它本来就有
//! 前端工具链，本项目一处都没有，不该为一个拷贝动作引入第二套工具链。
//!
//! **不做「先拷成临时副本再跑」那个变通。** EarthMesh 需要它是因为它的静态
//! netcdf 二进制在源码树里运行会被 SIGKILL；本项目实测没有这个问题 ——
//! `target/debug/colm-cli` 直接跑正常，动态依赖只剩 `libiconv` 与 `libSystem`
//! 两个系统库。复现不出来的问题不该先写变通。

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn stage(root: &Path) -> Result<()> {
    let triple = host_triple()?;
    println!("building colm-cli --release for {triple}");
    let st = Command::new("cargo")
        .args(["build", "--release", "-p", "colm-cli"])
        .current_dir(root)
        .status()
        .context("cannot run cargo")?;
    if !st.success() {
        bail!("cargo build -p colm-cli failed");
    }

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let src = root.join("target/release").join(format!("colm-cli{ext}"));
    if !src.is_file() {
        bail!("built but {} is missing", src.display());
    }
    let dir = root.join("gui/src-tauri/binaries");
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(format!("colm-cli-{triple}{ext}"));
    std::fs::copy(&src, &dst).with_context(|| format!("cannot copy to {}", dst.display()))?;
    let size = std::fs::metadata(&dst)?.len();
    println!("staged {} ({:.1} MB)", dst.display(), size as f64 / 1e6);
    Ok(())
}

/// `rustc -vV` 报的 host 三元组。
fn host_triple() -> Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("cannot run rustc")?;
    let text = String::from_utf8(out.stdout)?;
    text.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|s| s.trim().to_string())
        .context("rustc -vV did not report a host triple")
}
