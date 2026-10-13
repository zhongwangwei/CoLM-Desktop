//! 把 `colm-cli`、Rust 预处理器、Rust 主循环（`colm-rs`）与 AI 助手（`colm-agent` 及外部后端用的 `colm-mcp`）暂存到 Tauri 打包要找的位置。
//!
//! Tauri 的 `bundle.externalBin` 要求文件名带**目标三元组**后缀
//! （`colm-cli-aarch64-apple-darwin`），打包时它按当前目标去找。
//!
//! 源码与程序摘要一起暂存，拒绝把尚未编译的编辑声明为已运行版本。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

pub fn stage(root: &Path) -> Result<()> {
    let triple = host_triple()?;
    let source_before = colm_remote::engine::source_content_sha256(root)?;
    println!("building desktop sidecars --release for {triple}");
    // `cargo build -p A -p B --bin x` selects only the named binaries, not
    // every default binary from A and B.  Build each required sidecar directly.
    let mut built = Vec::new();
    for (package, binary) in [
        ("colm-cli", "colm-cli"),
        ("colm-srfdata", "mksrfdata-rs"),
        ("colm-init", "mkinidata-rs"),
        ("colm-runtime", "colm-rs"),
        ("colm-agent", "colm-agent"),
        ("colm-agent", "colm-mcp"),
    ] {
        built.push((binary, build_sidecar(root, package, binary)?));
    }

    if colm_remote::engine::source_content_sha256(root)? != source_before {
        bail!("engine source changed during compilation; rebuild before staging");
    }
    let bin_dir = built[0].1.parent().context("built CLI has no parent")?;
    if built.iter().any(|(_, path)| path.parent() != Some(bin_dir)) {
        bail!("desktop sidecars must be built into the same program directory");
    }
    write_build_stamp(bin_dir, &source_before)?;

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let dir = root.join("gui/src-tauri/binaries");
    std::fs::create_dir_all(&dir)?;
    for (name, src) in &built {
        if !src.is_file() {
            bail!("built but {} is missing", src.display());
        }
        let dst = dir.join(format!("{name}-{triple}{ext}"));
        std::fs::copy(src, &dst).with_context(|| format!("cannot copy to {}", dst.display()))?;
        let size = std::fs::metadata(&dst)?.len();
        println!("staged {} ({:.1} MB)", dst.display(), size as f64 / 1e6);
    }
    std::fs::copy(
        bin_dir.join("colm-build-source.json"),
        dir.join("colm-build-source.json"),
    )?;
    pack_engine_source(root, &built[0].1)?;
    if colm_remote::engine::source_content_sha256(root)? != source_before {
        bail!("engine source changed during staging; rebuild before packaging");
    }
    Ok(())
}

fn built_executable(message: &serde_json::Value, binary: &str) -> Option<PathBuf> {
    (message["reason"] == "compiler-artifact"
        && message["target"]["name"] == binary
        && message["target"]["kind"]
            .as_array()?
            .iter()
            .any(|kind| kind == "bin"))
    .then(|| message["executable"].as_str().map(PathBuf::from))
    .flatten()
}

fn build_sidecar(root: &Path, package: &str, binary: &str) -> Result<PathBuf> {
    let mut child = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            package,
            "--bin",
            binary,
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .spawn()
        .context("cannot run cargo")?;
    let stdout = child.stdout.take().context("cargo stdout is missing")?;
    let mut executable = None;
    for line in BufReader::new(stdout).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        if let Ok(message) = serde_json::from_str(&line) {
            if let Some(path) = built_executable(&message, binary) {
                executable = Some(path);
            }
        }
    }
    if !child.wait()?.success() {
        bail!("cargo build -p {package} --bin {binary} failed");
    }
    executable.context("Cargo did not report the executable it built")
}

fn write_build_stamp(dir: &Path, source_sha256: &str) -> Result<()> {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let mut programs = serde_json::Map::new();
    for (_, name) in colm_remote::engine::BINARIES {
        let mut file = std::fs::File::open(dir.join(format!("{name}{ext}")))?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        programs.insert(name.into(), format!("{:x}", hasher.finalize()).into());
    }
    let stamp =
        serde_json::json!({"schema":1,"source_sha256":source_sha256,"program_sha256":programs});
    let temp = dir.join(format!(".colm-build-source-{}.json", std::process::id()));
    std::fs::write(&temp, serde_json::to_vec_pretty(&stamp)?)?;
    let destination = dir.join("colm-build-source.json");
    // Windows rename cannot replace an existing destination; an absent stamp rejects registration safely.
    if cfg!(windows) && destination.exists() {
        std::fs::remove_file(&destination)?;
    }
    std::fs::rename(temp, destination)?;
    Ok(())
}

/// 远程运行要把引擎源码传到服务器上编：安装包里没有仓库，所以把源码快照打成 `colm-src.tar.gz` 随包附带
///（`colm-cli` 在资源目录的 `engine/` 里找它）。预编的 Linux 引擎由发布流水线另放进同一个目录。
fn pack_engine_source(root: &Path, cli: &Path) -> Result<()> {
    let out = root.join("gui/src-tauri/engine/colm-src.tar.gz");
    let status = Command::new(cli)
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
    fn cargo_artifact_path_wins_over_an_old_default_target_binary() {
        let message = serde_json::json!({"reason":"compiler-artifact","target":{"name":"colm-cli","kind":["bin"]},"executable":"/custom-target/release/colm-cli"});
        assert_eq!(
            built_executable(&message, "colm-cli"),
            Some(PathBuf::from("/custom-target/release/colm-cli"))
        );
        assert_eq!(built_executable(&message, "colm-rs"), None);
        assert_eq!(
            built_executable(
                &serde_json::json!({"reason":"compiler-message"}),
                "colm-cli"
            ),
            None
        );
    }

    #[test]
    fn stamp_binds_every_model_program_and_requires_all_four() {
        let root = std::env::temp_dir().join(format!("colm-build-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let ext = if cfg!(windows) { ".exe" } else { "" };
        assert!(write_build_stamp(&root, &"a".repeat(64)).is_err());
        for (_, name) in colm_remote::engine::BINARIES {
            std::fs::write(root.join(format!("{name}{ext}")), name).unwrap();
        }
        write_build_stamp(&root, &"a".repeat(64)).unwrap();
        let stamp: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("colm-build-source.json")).unwrap())
                .unwrap();
        assert_eq!(stamp["source_sha256"], "a".repeat(64));
        assert_eq!(stamp["program_sha256"].as_object().unwrap().len(), 4);
        assert_eq!(
            stamp["program_sha256"]["colm-cli"],
            format!("{:x}", Sha256::digest(b"colm-cli"))
        );
        std::fs::write(root.join(format!("colm-rs{ext}")), "changed program").unwrap();
        write_build_stamp(&root, &"b".repeat(64)).unwrap();
        let changed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("colm-build-source.json")).unwrap())
                .unwrap();
        assert_ne!(
            stamp["program_sha256"]["colm-rs"],
            changed["program_sha256"]["colm-rs"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

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
