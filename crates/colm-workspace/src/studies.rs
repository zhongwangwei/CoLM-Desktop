//! Research snapshots that used this workspace, stored outside its source checkout.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{git, layout::InfoLock, Workspace};

const FILE: &str = "studies.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyUse {
    pub dir: PathBuf,
    pub id: String,
    pub head: String,
    pub location: String,
    pub pinned_unix: u64,
    #[serde(default, skip_deserializing)]
    pub current: bool,
}

fn read(workspace: &Workspace) -> Result<Vec<StudyUse>> {
    let text = match std::fs::read_to_string(workspace.dir.join(FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("cannot read workspace study ledger"),
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("invalid study ledger row {}", index + 1))
        })
        .collect()
}

/// Append once per study, captured commit and execution location. Never discard damaged history.
pub fn append(
    workspace: &Workspace,
    study_dir: &Path,
    study_id: &str,
    head: &str,
    location: &str,
    pinned_unix: u64,
) -> Result<()> {
    let dir = study_dir
        .canonicalize()
        .context("study directory is missing")?;
    anyhow::ensure!(dir.is_dir(), "study path is not a directory");
    let _lock = InfoLock::acquire(&workspace.dir)?;
    let mut rows = read(workspace)?;
    if rows
        .iter()
        .any(|row| row.dir == dir && row.head == head && row.location == location)
    {
        return Ok(());
    }
    rows.push(StudyUse {
        dir,
        id: study_id.to_owned(),
        head: head.to_owned(),
        location: location.to_owned(),
        pinned_unix,
        current: false,
    });
    let mut text = String::new();
    for row in rows {
        text.push_str(&serde_json::to_string(&row)?);
        text.push('\n');
    }
    let tmp = workspace
        .dir
        .join(format!("{FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, workspace.dir.join(FILE))?;
    Ok(())
}

/// Remote runs also refer to their real local study directory, never a guessed server path.
pub fn list(workspace: &Workspace) -> Result<Vec<StudyUse>> {
    let mut rows = read(workspace)?;
    rows.retain(|row| row.dir.is_dir());
    let head = workspace.head()?;
    let clean = !git::is_dirty(&workspace.src())?;
    for row in &mut rows {
        row.current = clean && row.head == head;
    }
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::layout_tests::{source_repo, temp};

    #[test]
    fn snapshots_are_unique_current_only_when_clean_and_hide_removed_studies() {
        let root = temp("study-ledger");
        let source = root.join("source");
        source_repo(&source);
        let ws =
            Workspace::create(&root.join("ws"), "snow", source.to_str().unwrap(), None).unwrap();
        let study = root.join("study");
        std::fs::create_dir(&study).unwrap();
        let head = ws.head().unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| append(&ws, &study, "s1", &head, "local", 12).unwrap());
            }
        });
        let rows = list(&ws).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].current);
        assert_eq!(rows[0].pinned_unix, 12);
        std::fs::write(ws.src().join("new.txt"), "changed").unwrap();
        assert!(!list(&ws).unwrap()[0].current);
        git::run(&ws.src(), &["add", "new.txt"]).unwrap();
        git::run(&ws.src(), &["commit", "-qm", "change"]).unwrap();
        assert!(!list(&ws).unwrap()[0].current);
        append(&ws, &study, "s1", &head, "7920ts", 13).unwrap();
        assert_eq!(list(&ws).unwrap().len(), 2);
        std::fs::remove_dir(&study).unwrap();
        assert!(list(&ws).unwrap().is_empty());
        assert_eq!(read(&ws).unwrap().len(), 2, "history remains on disk");
        std::fs::write(ws.dir.join(FILE), "broken\n").unwrap();
        assert!(list(&ws).unwrap_err().to_string().contains("row 1"));
        std::fs::create_dir(&study).unwrap();
        assert!(append(&ws, &study, "s1", &head, "local", 14).is_err());
        assert_eq!(
            std::fs::read_to_string(ws.dir.join(FILE)).unwrap(),
            "broken\n"
        );
    }
}
