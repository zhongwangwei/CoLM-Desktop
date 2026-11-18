//! Keep Windows source-link copies consistent with their targets when committing edits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

// Include tracked native links without following directory links or scanning ignored trees.
fn all_links(root: &Path) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let mut links = source_links(root)?;
    for file in git::run(root, &["ls-files", "-z"])?
        .split('\0')
        .filter(|file| !file.is_empty())
    {
        if std::fs::symlink_metadata(root.join(file))
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            links.insert(PathBuf::from(file), std::fs::read_link(root.join(file))?);
        }
    }
    Ok(links)
}

// Keep the directories traversed before `..`, which lexical normalization would erase.
fn link_paths(alias: &Path, target: &Path) -> Result<(PathBuf, Vec<PathBuf>)> {
    ensure!(
        !target.is_absolute()
            && !target.components().any(|part| matches!(
                part,
                std::path::Component::Prefix(_) | std::path::Component::RootDir
            )),
        "source link must be relative"
    );
    let mut selected = alias
        .parent()
        .context("source link has no parent")?
        .to_path_buf();
    let parts = target
        .to_str()
        .context("source link target must be UTF-8")?
        .split(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .collect::<Vec<_>>();
    let mut directories = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        match *part {
            "" | "." => (),
            ".." => ensure!(selected.pop(), "source link escapes root"),
            part => selected.push(part),
        }
        if index + 1 < parts.len() && !selected.as_os_str().is_empty() {
            directories.push(selected.clone());
        }
    }
    Ok((selected, directories))
}

pub(crate) fn preserve_traversal_directories(from: &Path, to: &Path) -> Result<()> {
    // A sparse source can omit link targets that exist in the full destination checkout.
    let links = all_links(to)?;
    for (alias, target) in &links {
        for directory in link_paths(alias, target)?.1 {
            if !to.join(&directory).exists() {
                ensure!(
                    std::fs::symlink_metadata(from.join(&directory))?.is_dir(),
                    "source link traverses a missing directory or directory link"
                );
                std::fs::create_dir_all(to.join(directory))?;
            }
        }
    }
    for alias in links.keys() {
        resolve_source_file_link(to, alias, &links)?;
    }
    Ok(())
}

// Canonicalize parent spelling, but keep a native alias distinct from its target.
fn entry_identity(root: &Path, path: &Path) -> Result<PathBuf> {
    if !std::fs::symlink_metadata(root.join(path))
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return relative_identity(root, path);
    }
    let parent = path.parent().context("source path has no parent")?;
    Ok(relative_identity(root, parent)?.join(path.file_name().context("source path has no name")?))
}

/// Check destructive replacements before deleting anything, including links outside the directory.
/// Return required empty directories that a replacement must preserve.
pub(crate) fn check_removal(
    root: &Path,
    path: &str,
    replacement: Option<&Path>,
) -> Result<BTreeSet<PathBuf>> {
    let copies = source_links(root)?;
    let links = all_links(root)?;
    if links.is_empty() {
        return Ok(BTreeSet::new());
    }
    let removed = entry_identity(root, Path::new(path))?;
    let projected = |file: &Path| -> Result<PathBuf> {
        let identity = entry_identity(root, file)?;
        Ok(match identity.strip_prefix(&removed) {
            Ok(suffix) => replacement.map(|dir| dir.join(suffix)).unwrap_or_default(),
            Err(_) => root.join(file),
        })
    };
    let mut directories = BTreeSet::new();
    for alias in links.keys() {
        let inside = entry_identity(root, alias)?.starts_with(&removed);
        // Native links in a replaced tree disappear; only materialized aliases retain metadata.
        if inside && (!copies.contains_key(alias) || !projected(alias)?.is_file()) {
            continue;
        }
        if let Err(error) = resolve_source_file_link(root, alias, &links) {
            if copies.contains_key(alias) {
                return Err(error);
            }
            continue;
        }
        let mut selected = alias.clone();
        while let Some(target) = links.get(&selected) {
            if selected != *alias
                && entry_identity(root, &selected)?.starts_with(&removed)
                && !copies.contains_key(&selected)
            {
                // A native intermediate link becomes the replacement's regular file.
                break;
            }
            let (next, traversed) = link_paths(&selected, target)?;
            for directory in traversed {
                ensure!(
                    projected(&directory)?.is_dir(),
                    "cannot remove {} while source link {} traverses it",
                    directory.display(),
                    alias.display()
                );
                if let Ok(suffix) = entry_identity(root, &directory)?.strip_prefix(&removed) {
                    directories.insert(suffix.to_path_buf());
                }
            }
            selected = next;
            ensure!(
                projected(&selected)?.is_file(),
                "cannot remove {} while source link {} still uses it",
                selected.display(),
                alias.display()
            );
        }
        if replacement.is_some() && copies.contains_key(alias) {
            let copy = std::fs::read(projected(alias)?)?;
            ensure!(
                copy == std::fs::read(projected(&selected)?)?
                    || copy == std::fs::read(regular_path(root, alias)?)?,
                "source-link conflict: replacement of {} differs from its target",
                alias.display()
            );
        }
    }
    Ok(directories)
}

// Preserve legacy workspaces with unrelated unsupported links while protecting valid ones.
pub(crate) fn valid_native_links(root: &Path) -> Result<Vec<PathBuf>> {
    let links = all_links(root)?;
    Ok(links
        .keys()
        .filter(|alias| {
            std::fs::symlink_metadata(root.join(alias))
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
                && resolve_source_file_link(root, alias, &links).is_ok()
        })
        .cloned()
        .collect())
}

pub(crate) fn check_native_links(root: &Path, aliases: &[PathBuf]) -> Result<()> {
    let links = source_links(root)?;
    for alias in aliases {
        if std::fs::symlink_metadata(root.join(alias))
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            resolve_source_file_link(root, alias, &links)?;
        }
    }
    Ok(())
}

/// Validate every copy before writing any, so divergent external edits are never overwritten.
pub(crate) fn synchronize(root: &Path, edit: &mut git::Edit) -> Result<Vec<String>> {
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
        let relative = spelling(path.strip_prefix(root)?)?;
        edit.change(&[relative], || Ok(std::fs::write(path, bytes)?))?;
    }
    if let Some(bytes) = metadata {
        edit.change(&[SOURCE_LINKS_FILE.to_owned()], || {
            Ok(std::fs::write(root.join(SOURCE_LINKS_FILE), bytes)?)
        })?;
    }
    Ok(changed)
}
