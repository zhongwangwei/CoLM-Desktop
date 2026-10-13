//! Check the current React source against generated Rust command signatures.
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn source_root(start: &Path) -> Result<PathBuf> {
    start
        .ancestors()
        .find(|dir| {
            dir.join(".git").exists()
                && dir.join("gui/package.json").is_file()
                && dir.join("gui/src-tauri/src/lib.rs").is_file()
        })
        .map(Path::to_path_buf)
        .context("check-gui must run inside the checkout being checked")
}

pub fn check(root: &Path) -> Result<()> {
    // Reuse the installed TypeScript parser so generic calls and nested objects
    // cannot be misread by the former JavaScript substring scanner.
    let status = Command::new("node")
        .arg(root.join("scripts/check-gui.mjs"))
        .current_dir(root)
        .status()
        .context("check-gui needs Node.js and npm ci in gui/")?;
    if !status.success() {
        bail!("GUI command/event contract validation failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_root_finds_the_current_react_checkout() {
        let dir = std::env::temp_dir().join(format!("gui-source-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("gui/src-tauri/src")).unwrap();
        std::fs::write(dir.join("gui/package.json"), "{}").unwrap();
        std::fs::write(dir.join("gui/src-tauri/src/lib.rs"), "").unwrap();
        assert!(source_root(&dir).is_err());
        std::fs::write(dir.join(".git"), "gitdir: /unused").unwrap();
        assert_eq!(source_root(&dir.join("gui/src-tauri/src")).unwrap(), dir);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
