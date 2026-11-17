//! Keep Windows source-link copies consistent with their targets when committing edits.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_remote::engine::{resolve_source_file_link, source_links, SOURCE_LINKS_FILE};

use crate::{git, patch::check_path};

pub(crate) fn check_edit(root: &Path, path: &str) -> Result<()> {
    let links = source_links(root)?;
    if links.is_empty() {
        return Ok(());
    }
    ensure!(
        !links.contains_key(Path::new(path))
            && !links.contains_key(&relative_identity(root, Path::new(path))?),
        "{path} is a materialized symbolic link; change the file it points to instead"
    );
    Ok(())
}

pub(crate) fn check_restore(root: &Path, path: &str, revision: &str) -> Result<()> {
    check_edit(root, path)?;
    if git::blob(root, revision, SOURCE_LINKS_FILE)?.is_some() {
        let text = git::run(root, &["show", &format!("{revision}:{SOURCE_LINKS_FILE}")])?;
        let links: BTreeMap<String, String> = serde_json::from_str(&text)?;
        ensure!(
            !links.contains_key(path),
            "{path} was a materialized symbolic link; restore its target or the whole version instead"
        );
    }
    if git::blob(root, revision, path)?.is_none() {
        check_removal(root, path, None)?;
    }
    Ok(())
}

// Resolve stored spelling on case-insensitive filesystems without folding distinct Linux names.
fn relative_identity(root: &Path, path: &Path) -> Result<PathBuf> {
    match root.join(path).canonicalize() {
        Ok(resolved) => Ok(resolved.strip_prefix(root.canonicalize()?)?.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(error.into()),
    }
}

fn spelling(path: &Path) -> Result<String> {
    Ok(path
        .to_str()
        .context("source path must be UTF-8")?
        .replace('\\', "/"))
}

fn regular_path(root: &Path, path: &Path) -> Result<PathBuf> {
    check_path(&spelling(path)?)?;
    let mut current = root.to_path_buf();
    for part in path.components() {
        current.push(part);
        ensure!(
            !std::fs::symlink_metadata(&current)?
                .file_type()
                .is_symlink(),
            "source-link copies must not traverse symbolic links"
        );
    }
    ensure!(
        current.is_file(),
        "source-link target must be a regular file"
    );
    Ok(current)
}

fn next_path(path: &Path, target: &Path) -> Result<PathBuf> {
    let mut result = path
        .parent()
        .context("source link has no parent")?
        .to_path_buf();
    for part in target.components() {
        match part {
            Component::Normal(part) => result.push(part),
            Component::CurDir => (),
            Component::ParentDir => ensure!(result.pop(), "source link escapes root"),
            _ => anyhow::bail!("source link must be relative"),
        }
    }
    Ok(result)
}

/// Check destructive replacements before deleting anything, including links outside the directory.
pub(crate) fn check_removal(root: &Path, path: &str, replacement: Option<&Path>) -> Result<()> {
    let links = source_links(root)?;
    if links.is_empty() {
        return Ok(());
    }
    let removed = relative_identity(root, Path::new(path))?;
    let survives = |file: &Path| -> Result<bool> {
        Ok(relative_identity(root, file)?
            .strip_prefix(&removed)
            .map_or(true, |suffix| {
                replacement.is_some_and(|dir| dir.join(suffix).is_file())
            }))
    };
    for alias in links.keys() {
        if !survives(alias)? {
            continue;
        }
        // Validate the original chain before walking its logical metadata.
        resolve_source_file_link(root, alias, &links)?;
        let mut selected = alias.clone();
        while let Some(target) = links.get(&selected) {
            selected = next_path(&selected, target)?;
            ensure!(
                survives(&selected)?,
                "cannot remove {} while source link {} still uses it",
                selected.display(),
                alias.display()
            );
        }
        if let Some(replacement) = replacement {
            let projected = |file: &Path| -> Result<PathBuf> {
                Ok(relative_identity(root, file)?
                    .strip_prefix(&removed)
                    .map_or_else(|_| root.join(file), |suffix| replacement.join(suffix)))
            };
            let copy = std::fs::read(projected(alias)?)?;
            ensure!(
                copy == std::fs::read(projected(&selected)?)?
                    || copy == std::fs::read(regular_path(root, alias)?)?,
                "source-link conflict: replacement of {} differs from its target",
                alias.display()
            );
        }
    }
    Ok(())
}

/// Validate every copy before writing any, so divergent external edits are never overwritten.
pub(crate) fn synchronize(root: &Path) -> Result<Vec<String>> {
    let original = source_links(root)?;
    if original.is_empty() {
        return Ok(Vec::new());
    }
    let mut links = original.clone();
    let mut changed = Vec::new();
    for alias in original.keys() {
        check_path(&spelling(alias)?)?;
        match std::fs::symlink_metadata(root.join(alias)) {
            Ok(metadata) => ensure!(
                metadata.is_file(),
                "source-link copy must be a regular file"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                links.remove(alias);
            }
            Err(error) => return Err(error.into()),
        }
    }
    let canonical_root = root.canonicalize()?;
    let mut copies = Vec::new();
    for alias in links.keys() {
        let link = resolve_source_file_link(root, alias, &links)?;
        let target = link.resolved_file.strip_prefix(&canonical_root)?;
        let target = regular_path(root, target)?;
        let output = regular_path(root, alias)?;
        let bytes = std::fs::read(&output)?;
        let expected = std::fs::read(target)?;
        if bytes == expected {
            continue;
        }
        let name = spelling(alias)?;
        let actual = git::run_with_input(
            root,
            &["hash-object", &format!("--path={name}"), "--stdin"],
            &bytes,
        )?;
        ensure!(
            git::blob(root, "HEAD", &name)?.as_deref() == Some(actual.as_str()),
            "source-link conflict: {name} was edited separately from its target; preserve the desired content in the target and make its copies agree before committing"
        );
        copies.push((output, expected));
        changed.push(name);
    }
    // Validate metadata serialization before changing copies.
    let metadata = if links != original {
        let portable = links
            .iter()
            .map(|(path, target)| Ok((spelling(path)?, spelling(target)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        changed.push(SOURCE_LINKS_FILE.into());
        Some(serde_json::to_vec(&portable)?)
    } else {
        None
    };
    for (path, bytes) in copies {
        std::fs::write(path, bytes)?;
    }
    if let Some(bytes) = metadata {
        std::fs::write(root.join(SOURCE_LINKS_FILE), bytes)?;
    }
    Ok(changed)
}
