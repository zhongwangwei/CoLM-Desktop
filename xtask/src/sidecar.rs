//! 把 `colm-cli`、Rust 预处理器、Rust 主循环（`colm-rs`）与 AI 助手（`colm-agent` 及外部后端用的 `colm-mcp`）暂存到 Tauri 打包要找的位置。
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
    println!("building desktop sidecars --release for {triple}");
    // `cargo build -p A -p B --bin x` selects only the named binaries, not
    // every default binary from A and B.  Build each required sidecar directly.
    for (package, binary) in [
        ("colm-cli", "colm-cli"),
        ("colm-srfdata", "mksrfdata-rs"),
        ("colm-init", "mkinidata-rs"),
        ("colm-runtime", "colm-rs"),
        ("colm-agent", "colm-agent"),
        ("colm-agent", "colm-mcp"),
    ] {
        let status = Command::new("cargo")
            .args(["build", "--release", "-p", package, "--bin", binary])
            .current_dir(root)
            .status()
            .context("cannot run cargo")?;
        if !status.success() {
            bail!("cargo build -p {package} --bin {binary} failed");
        }
    }

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let dir = root.join("gui/src-tauri/binaries");
    std::fs::create_dir_all(&dir)?;
    for name in [
        "colm-cli",
        "mksrfdata-rs",
        "mkinidata-rs",
        "colm-rs",
        "colm-agent",
        "colm-mcp",
    ] {
        let src = root.join("target/release").join(format!("{name}{ext}"));
        if !src.is_file() {
            bail!("built but {} is missing", src.display());
        }
        let dst = dir.join(format!("{name}-{triple}{ext}"));
        std::fs::copy(&src, &dst).with_context(|| format!("cannot copy to {}", dst.display()))?;
        let size = std::fs::metadata(&dst)?.len();
        println!("staged {} ({:.1} MB)", dst.display(), size as f64 / 1e6);
    }
    pack_engine_source(root, ext)
}

/// 远程运行要把引擎源码传到服务器上编：安装包里没有仓库，所以把源码快照打成 `colm-src.tar.gz` 随包附带
///（`colm-cli` 在资源目录的 `engine/` 里找它）。预编的 Linux 引擎由发布流水线另放进同一个目录。
fn pack_engine_source(root: &Path, ext: &str) -> Result<()> {
    let out = root.join("gui/src-tauri/engine/colm-src.tar.gz");
    let status = Command::new(root.join("target/release").join(format!("colm-cli{ext}")))
        .arg("engine-pack")
        .arg("--out")
        .arg(&out)
        .status()
        .context("cannot run colm-cli engine-pack")?;
    if !status.success() {
        bail!("colm-cli engine-pack failed");
    }
    stage_snapshot_docs(&out)?;
    let size = std::fs::metadata(&out)?.len();
    println!("packed {} ({:.1} MB)", out.display(), size as f64 / 1e6);
    Ok(())
}

/// Read docs from the completed archive so later checkout edits cannot mix source and docs.
fn stage_snapshot_docs(archive: &Path) -> Result<()> {
    let listing = Command::new("tar").arg("-tzf").arg(archive).output()?;
    if !listing.status.success() {
        bail!("cannot list source snapshot");
    }
    let parent = archive.parent().context("source snapshot has no parent")?;
    let stage = parent.join(format!(".docs-stage-{}", std::process::id()));
    std::fs::create_dir(&stage).context("cannot reserve docs staging directory")?;
    let result = (|| -> Result<()> {
        for name in std::str::from_utf8(&listing.stdout)?.lines() {
            if !name.starts_with("docs/") || name.ends_with('/') {
                continue;
            }
            if name.contains('\\')
                || Path::new(name)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                bail!("invalid snapshot document path: {name}");
            }
            // Extract to stdout, never let archive links or paths write into the filesystem.
            let file = Command::new("tar")
                .arg("-xOzf")
                .arg(archive)
                .arg(name)
                .output()?;
            if !file.status.success() {
                bail!("cannot read snapshot document: {name}");
            }
            let destination = stage.join(name);
            std::fs::create_dir_all(destination.parent().context("document has no parent")?)?;
            std::fs::write(destination, file.stdout)?;
        }
        for required in [
            "design-ai-assistant.md",
            "colm-process-knowledge.md",
            "knowledge-sources.json",
        ] {
            if !stage.join("docs").join(required).is_file() {
                bail!("source snapshot is missing docs/{required}");
            }
        }
        let destination = parent.join("docs");
        let previous = stage.join("previous");
        let had_previous = destination.exists();
        if had_previous {
            std::fs::rename(&destination, &previous)?;
        }
        if let Err(error) = std::fs::rename(stage.join("docs"), &destination) {
            if had_previous {
                std::fs::rename(&previous, &destination)
                    .context("cannot restore previous staged docs")?;
            }
            return Err(error.into());
        }
        Ok(())
    })();
    // Keep the previous snapshot if rollback failed; it remains recoverable in stage/previous.
    if !stage.join("previous").exists() || result.is_ok() {
        std::fs::remove_dir_all(&stage)?;
    }
    result
}

/// `rustc -vV` 报的 host 三元组。
fn host_triple() -> Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("cannot run rustc")?;
    if !out.status.success() {
        bail!(
            "rustc -vV failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8(out.stdout)?;
    text.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|s| s.trim().to_string())
        .context("rustc -vV did not report a host triple")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_docs_use_snapshot_bytes_and_missing_docs_preserve_previous_stage() {
        let root = std::env::temp_dir().join(format!("colm-bundle-docs-{}", std::process::id()));
        std::fs::create_dir_all(root.join("checkout/docs")).unwrap();
        std::fs::create_dir_all(root.join("engine")).unwrap();
        let checkout = root.join("checkout");
        for name in [
            "design-ai-assistant.md",
            "colm-process-knowledge.md",
            "knowledge-sources.json",
        ] {
            std::fs::write(checkout.join("docs").join(name), "snapshot").unwrap();
        }
        let archive = root.join("engine/colm-src.tar.gz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&checkout)
            .arg("docs")
            .status()
            .unwrap()
            .success());
        std::fs::write(
            checkout.join("docs/colm-process-knowledge.md"),
            "new checkout",
        )
        .unwrap();
        stage_snapshot_docs(&archive).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("engine/docs/colm-process-knowledge.md")).unwrap(),
            "snapshot"
        );
        std::fs::remove_file(checkout.join("docs/knowledge-sources.json")).unwrap();
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&checkout)
            .arg("docs")
            .status()
            .unwrap()
            .success());
        assert!(stage_snapshot_docs(&archive).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("engine/docs/colm-process-knowledge.md")).unwrap(),
            "snapshot"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
