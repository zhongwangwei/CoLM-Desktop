use crate::validation::{self, parameter_key};
use crate::*;
use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a registry writer waits for another one to finish.
const REGISTRY_LOCK_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct Registry {
    pub(crate) root: PathBuf,
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
impl Registry {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        fs::create_dir_all(root.join("versions"))?;
        fs::create_dir_all(root.join("drafts"))?;
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn lock(&self) -> Result<Lock> {
        self.lock_within(REGISTRY_LOCK_WAIT)
    }
    /// Writers wait their turn instead of failing at once: the desktop finishes server cases
    /// several at a time, and each `model-migrate-case` takes this lock briefly (2026-10-18:
    /// four of five sites in a batch failed with "model registry is locked").
    fn lock_within(&self, wait: Duration) -> Result<Lock> {
        let path = self.root.join("write.lock");
        let deadline = Instant::now() + wait;
        let mut pause = Duration::from_millis(10);
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())?;
                    return Ok(Lock(path));
                }
                // Windows reports a lock file that is being deleted as PermissionDenied.
                Err(error)
                    if (error.kind() == std::io::ErrorKind::AlreadyExists
                        || (cfg!(windows)
                            && error.kind() == std::io::ErrorKind::PermissionDenied))
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(200));
                }
                Err(error) => {
                    return Err(error).context(
                        "model registry is locked; recover a stale write.lock only after confirming its writer exited",
                    )
                }
            }
        }
    }
    pub(crate) fn revision_dir(&self, version: &VersionRef) -> Result<PathBuf> {
        validation::identifier(&version.id)?;
        ensure!(version.revision > 0, "revision must be positive");
        Ok(self
            .root
            .join("versions")
            .join(&version.id)
            .join(version.revision.to_string()))
    }
    pub fn install_official(&self, code: CodeRef) -> Result<FrozenRevision> {
        validation::code(&code)?;
        ensure!(
            code.kind == CodeKind::Official,
            "official version needs official code"
        );
        let _lock = self.lock()?;
        let versions: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|r| r.version.id == "official")
            .collect();
        if let Some(existing) = versions.iter().find(|r| r.code == code) {
            return Ok(existing.clone());
        }
        let version = VersionRef {
            id: "official".into(),
            revision: versions
                .iter()
                .map(|r| r.version.revision)
                .max()
                .unwrap_or(0)
                + 1,
        };
        let overlay = Overlay::default();
        let mut frozen = FrozenRevision {
            schema: SCHEMA,
            version,
            name: "Official CoLM".into(),
            note: "Bundled immutable baseline".into(),
            parent: None,
            code,
            overlay_sha256: digest_json(&overlay)?,
            overlay,
            fingerprint: String::new(),
            created_unix: now()?,
            applicability: Applicability::default(),
            derived_from: None,
        };
        frozen.fingerprint = revision_fingerprint(&frozen)?;
        self.publish(&frozen, None, &[])?;
        Ok(frozen)
    }
    pub fn get(&self, version: &VersionRef) -> Result<FrozenRevision> {
        let dir = self.revision_dir(version)?;
        let frozen: FrozenRevision = read_json(&dir.join("revision.json"))?;
        ensure!(
            frozen.schema == SCHEMA && frozen.version == *version,
            "revision identity mismatch"
        );
        validation::code(&frozen.code)?;
        validation::applicability(&frozen.applicability)?;
        validate_overlay(&model_overlay(&frozen.overlay))?;
        ensure!(
            digest_json(&frozen.overlay)? == frozen.overlay_sha256
                && revision_fingerprint(&frozen)? == frozen.fingerprint,
            "revision content fingerprint mismatch"
        );
        verify_assets(&frozen.overlay, &dir)?;
        Ok(frozen)
    }
    /// Flat, deterministically ordered source tree; `parent` is the pinned edge.
    pub fn list(&self) -> Result<Vec<FrozenRevision>> {
        let mut result = Vec::new();
        for item in fs::read_dir(self.root.join("versions"))? {
            let item = item?;
            ensure!(item.file_type()?.is_dir(), "unexpected version entry");
            let id = item
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid version name"))?;
            validation::identifier(&id)?;
            for revision in fs::read_dir(item.path())? {
                let revision = revision?;
                let name = revision.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                result.push(self.get(&VersionRef {
                    id: id.clone(),
                    revision: name.parse().context("invalid revision directory")?,
                })?);
            }
        }
        result.sort_by(|a, b| {
            a.version
                .id
                .cmp(&b.version.id)
                .then(a.version.revision.cmp(&b.version.revision))
        });
        Ok(result)
    }
    pub fn tree(&self) -> Result<Vec<FrozenRevision>> {
        self.list()
    }
    /// Delete a frozen revision nothing in the registry still builds on: not the latest
    /// official baseline, no child revision and no draft. Source snapshots that no remaining
    /// revision uses are removed too; the returned paths are what was deleted. Callers check
    /// external references (history, running work) before calling this.
    /// What stops `version` from being removed (the latest official revision, a revision
    /// derived from it, a draft based on it); `None` when the registry itself allows it.
    pub fn removal_blocker(&self, version: &VersionRef) -> Result<Option<String>> {
        Ok(removal_blocker(&self.list()?, &self.drafts()?, version))
    }
    pub fn remove(&self, version: &VersionRef) -> Result<Vec<PathBuf>> {
        let _lock = self.lock()?;
        let dir = self.revision_dir(version)?;
        let frozen = self.get(version)?;
        let all = self.list()?;
        if let Some(reason) = removal_blocker(&all, &self.drafts()?, version) {
            anyhow::bail!(reason);
        }
        self.discard_revision(&dir)?;
        let mut removed = vec![dir];
        if !all
            .iter()
            .any(|r| r.version != *version && r.code == frozen.code)
        {
            let source = self.source_dir(&frozen.code)?;
            if source.is_dir() {
                fs::remove_dir_all(&source)?;
                removed.push(source);
            }
        }
        Ok(removed)
    }
    /// Tidy away old official baselines nothing uses any more. Every desktop start freezes the
    /// bundled source as a new official revision with a ~126 MB source snapshot, and nothing
    /// removed the old ones (2026-10-18: 71 revisions, 8.57 GB). `keep` names the revisions the
    /// caller knows are still in use (case bindings, history, the official revision of the
    /// running program); on top of it the rules of [`Registry::remove`] hold: the latest
    /// official revision and anything a revision or draft builds on stay, and a source
    /// snapshot goes only when no remaining revision or draft shares it. `dry_run` reports
    /// what would go without touching anything.
    pub fn prune_official(&self, keep: &[VersionRef], dry_run: bool) -> Result<PruneReport> {
        let _lock = self.lock()?;
        let all = self.list()?;
        let drafts = self.drafts()?;
        let mut report = PruneReport {
            dry_run,
            ..PruneReport::default()
        };
        for revision in all.iter().filter(|r| r.version.id == "official") {
            let reason = removal_blocker(&all, &drafts, &revision.version).or_else(|| {
                keep.contains(&revision.version)
                    .then(|| "a case or history record still uses it".to_owned())
            });
            match reason {
                Some(reason) => report.kept.push(KeptRevision {
                    version: revision.version.clone(),
                    reason,
                }),
                None => report.removed.push(revision.version.clone()),
            }
        }
        let remaining = |code: &CodeRef| {
            all.iter()
                .any(|r| !report.removed.contains(&r.version) && r.code == *code)
                || drafts.iter().any(|d| d.code == *code)
        };
        let mut sources = Vec::new();
        for revision in all.iter().filter(|r| report.removed.contains(&r.version)) {
            let source = self.source_dir(&revision.code)?;
            if !remaining(&revision.code) && source.is_dir() && !sources.contains(&source) {
                sources.push(source);
            }
        }
        for version in &report.removed {
            let dir = self.revision_dir(version)?;
            report.bytes += tree_size(&dir)?;
            if !dry_run {
                self.discard_revision(&dir)?;
            }
            report.paths.push(dir);
        }
        for source in sources {
            report.bytes += tree_size(&source)?;
            if !dry_run {
                // Rename first: a half-deleted snapshot without code.json counts as missing.
                let name = source.file_name().context("source name")?.to_string_lossy();
                let trash = source.with_file_name(format!(".remove-{name}-{}", std::process::id()));
                fs::rename(&source, &trash)?;
                fs::remove_dir_all(&trash)?;
            }
            report.paths.push(source);
        }
        Ok(report)
    }
    fn source_dir(&self, code: &CodeRef) -> Result<PathBuf> {
        Ok(self.root.join("sources").join(source_cache_name(code)?))
    }
    /// Remove one revision directory atomically (and its version directory once empty).
    fn discard_revision(&self, dir: &Path) -> Result<()> {
        let parent = dir.parent().context("revision parent")?.to_path_buf();
        let name = dir.file_name().context("revision name")?.to_string_lossy();
        let trash = parent.join(format!(".remove-{name}-{}", std::process::id()));
        fs::rename(dir, &trash)?;
        fs::remove_dir_all(&trash)?;
        if fs::read_dir(&parent)?.next().is_none() {
            fs::remove_dir(&parent)?;
        }
        Ok(())
    }
    pub fn drafts(&self) -> Result<Vec<ModelDraft>> {
        let mut drafts = Vec::new();
        for entry in fs::read_dir(self.root.join("drafts"))? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .context("invalid draft filename")?;
                drafts.push(self.draft(id)?);
            }
        }
        drafts.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(drafts)
    }
    pub fn draft(&self, id: &str) -> Result<ModelDraft> {
        validation::identifier(id)?;
        let mut draft: ModelDraft =
            read_json(&self.root.join("drafts").join(format!("{id}.json")))?;
        draft.overlay = model_overlay(&draft.overlay);
        ensure!(draft.id == id, "draft identity mismatch");
        self.validate_draft(&draft)?;
        Ok(draft)
    }
    pub fn create_draft(&self, mut draft: ModelDraft) -> Result<ModelDraft> {
        let _lock = self.lock()?;
        validation::identifier(&draft.id)?;
        ensure!(draft.id != "official", "official model is read-only");
        let file = self.root.join("drafts").join(format!("{}.json", draft.id));
        ensure!(
            !file.exists() && !self.root.join("versions").join(&draft.id).exists(),
            "model ID already exists"
        );
        let parent = self.get(&draft.parent)?;
        let changes = draft.overlay;
        check_expected_values(&changes, &parent.overlay)?;
        draft.overlay = model_overlay(&parent.overlay);
        for record in changes.parameters {
            if record.scope_instance.kind == "case-scalar" {
                draft
                    .overlay
                    .physics
                    .retain(|key, _| !key.eq_ignore_ascii_case(&record.raw_key));
            }
            let key = parameter_key(&record);
            draft.overlay.parameters.retain(|r| parameter_key(r) != key);
            draft.overlay.parameters.push(record);
        }
        draft.overlay.physics.extend(changes.physics);
        if changes.hybrid.is_some() {
            draft.overlay.hybrid = changes.hybrid;
        }
        for asset in changes.process_files {
            draft
                .overlay
                .process_files
                .retain(|a| a.relative_path != asset.relative_path);
            draft.overlay.process_files.push(asset);
        }
        draft.generation = 1;
        draft.created_unix = now()?;
        self.validate_draft(&draft)?;
        write_json(&file, &draft)?;
        Ok(draft)
    }
    pub fn save_draft(
        &self,
        mut draft: ModelDraft,
        expected_generation: u64,
    ) -> Result<ModelDraft> {
        let _lock = self.lock()?;
        let old = self.draft(&draft.id)?;
        ensure!(
            old.generation == expected_generation && draft.generation == expected_generation,
            "draft changed since it was loaded"
        );
        ensure!(
            draft.parent == old.parent,
            "draft parent is immutable; create a new derived model"
        );
        let changed = Overlay {
            parameters: draft
                .overlay
                .parameters
                .iter()
                .filter(|record| !old.overlay.parameters.iter().any(|prior| prior == *record))
                .cloned()
                .collect(),
            ..Overlay::default()
        };
        check_expected_values(&changed, &old.overlay)?;
        draft.created_unix = old.created_unix;
        draft.generation = old
            .generation
            .checked_add(1)
            .context("draft generation overflow")?;
        normalize_overlay(&mut draft.overlay);
        self.validate_draft(&draft)?;
        write_json(
            &self.root.join("drafts").join(format!("{}.json", draft.id)),
            &draft,
        )?;
        Ok(draft)
    }
    fn validate_draft(&self, draft: &ModelDraft) -> Result<()> {
        validation::identifier(&draft.id)?;
        ensure!(draft.id != "official", "official model is read-only");
        ensure!(
            !draft.name.trim().is_empty()
                && draft.name.len() <= 200
                && !draft.name.chars().any(char::is_control),
            "invalid model name"
        );
        ensure!(
            !draft.note.trim().is_empty() && draft.note.len() <= 10000,
            "model purpose/note is required"
        );
        ensure!(
            draft.parent.id != draft.id,
            "model cannot derive from itself"
        );
        self.get(&draft.parent)?;
        validation::code(&draft.code)?;
        validation::applicability(&draft.applicability)?;
        validate_overlay(&draft.overlay)
    }
    pub fn freeze(&self, id: &str, asset_root: &Path) -> Result<FrozenRevision> {
        let _lock = self.lock()?;
        let mut draft = self.draft(id)?;
        normalize_overlay(&mut draft.overlay);
        let versions: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|r| r.version.id == id)
            .collect();
        let mut frozen = FrozenRevision {
            schema: SCHEMA,
            version: VersionRef {
                id: id.into(),
                revision: versions
                    .iter()
                    .map(|r| r.version.revision)
                    .max()
                    .unwrap_or(0)
                    + 1,
            },
            name: draft.name,
            note: draft.note,
            parent: Some(draft.parent.clone()),
            code: draft.code,
            overlay_sha256: digest_json(&draft.overlay)?,
            overlay: draft.overlay,
            fingerprint: String::new(),
            created_unix: now()?,
            applicability: draft.applicability,
            derived_from: draft.derived_from,
        };
        frozen.fingerprint = revision_fingerprint(&frozen)?;
        if let Some(existing) = versions
            .iter()
            .find(|v| v.fingerprint == frozen.fingerprint)
        {
            return Ok(existing.clone());
        }
        // A configuration that already exists (in effect) is that version, whatever its draft
        // was called: re-choosing a scheme must not mint another revision. The unused draft goes.
        if versions.is_empty() {
            if let Some(existing) = self.list()?.into_iter().find(|v| {
                crate::equivalence::same_model(
                    &frozen.code,
                    &frozen.overlay,
                    &frozen.applicability,
                    &v.code,
                    &v.overlay,
                    &v.applicability,
                )
            }) {
                let _ = fs::remove_file(self.root.join("drafts").join(format!("{id}.json")));
                return Ok(existing);
            }
        }
        // Existing immutable revisions are the fallback for inherited assets.
        let mut inherited = versions
            .iter()
            .rev()
            .map(|v| self.revision_dir(&v.version))
            .collect::<Result<Vec<_>>>()?;
        inherited.push(self.revision_dir(&draft.parent)?);
        self.publish(&frozen, Some(asset_root), &inherited)?;
        Ok(frozen)
    }
    fn publish(
        &self,
        frozen: &FrozenRevision,
        assets: Option<&Path>,
        inherited: &[PathBuf],
    ) -> Result<()> {
        let dest = self.revision_dir(&frozen.version)?;
        let parent = dest.parent().context("revision parent")?;
        fs::create_dir_all(parent)?;
        let stage = parent.join(format!(
            ".stage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir(&stage)
            .context("unfinished revision staging directory; inspect before recovering")?;
        let result = (|| {
            for asset in all_assets(&frozen.overlay) {
                let primary = assets.map(|root| root.join(&asset.relative_path));
                let source = match primary {
                    Some(path) if path.exists() => {
                        let root = assets.context("asset root")?;
                        safe_file(root, &asset.relative_path)?
                    }
                    _ => inherited
                        .iter()
                        .filter_map(|root| safe_file(root, &asset.relative_path).ok())
                        .find(|path| {
                            fs::read(path).is_ok_and(|bytes| digest(&bytes) == asset.sha256)
                        })
                        .context("missing frozen asset source")?,
                };
                let bytes = fs::read(&source)?;
                ensure!(
                    digest(&bytes) == asset.sha256,
                    "asset changed: {}",
                    asset.relative_path
                );
                let target = stage.join(&asset.relative_path);
                fs::create_dir_all(target.parent().context("asset parent")?)?;
                fs::write(target, bytes)?;
            }
            if let Some(h) = &frozen.overlay.hybrid {
                fs::write(stage.join("hybrid.toml"), &h.config)?;
            }
            verify_assets(&frozen.overlay, &stage)?;
            write_json(&stage.join("revision.json"), frozen)?;
            ensure!(!dest.exists(), "immutable revision already exists");
            fs::rename(&stage, &dest)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(stage);
        }
        result
    }
}
/// Why the registry itself still needs `version`: the latest official baseline, a revision
/// derived from it, or a draft based on it. `None` means only outside references can keep it.
fn removal_blocker(
    all: &[FrozenRevision],
    drafts: &[ModelDraft],
    version: &VersionRef,
) -> Option<String> {
    if version.id == "official"
        && all
            .iter()
            .filter(|r| r.version.id == "official")
            .map(|r| r.version.revision)
            .max()
            == Some(version.revision)
    {
        return Some(
            "the latest official revision is the bundled baseline and cannot be deleted".into(),
        );
    }
    if let Some(child) = all.iter().find(|r| r.parent.as_ref() == Some(version)) {
        return Some(format!(
            "revision {}#{} is derived from it; delete that revision first",
            child.version.id, child.version.revision
        ));
    }
    drafts
        .iter()
        .find(|d| d.parent == *version)
        .map(|draft| format!("draft {} is based on it; delete that draft first", draft.id))
}
/// Bytes under `path`, not following links.
fn tree_size(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        total += tree_size(&entry?.path())?;
    }
    Ok(total)
}
pub(crate) fn normalize_overlay(overlay: &mut Overlay) {
    overlay.parameters.sort_by_key(parameter_key);
    overlay
        .process_files
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    if let Some(h) = &mut overlay.hybrid {
        h.assets
            .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    }
}
pub(crate) fn all_assets(overlay: &Overlay) -> impl Iterator<Item = &Asset> {
    overlay
        .process_files
        .iter()
        .chain(overlay.hybrid.iter().flat_map(|h| &h.assets))
}
pub(crate) fn verify_assets(overlay: &Overlay, root: &Path) -> Result<()> {
    for asset in all_assets(overlay) {
        ensure!(
            digest(&fs::read(safe_file(root, &asset.relative_path)?)?) == asset.sha256,
            "frozen asset hash mismatch: {}",
            asset.relative_path
        );
    }
    for asset in &overlay.process_files {
        colm_namelist::parse(&fs::read_to_string(safe_file(root, &asset.relative_path)?)?)
            .context("invalid process parameter namelist")?;
    }
    if let Some(h) = &overlay.hybrid {
        ensure!(
            fs::read_to_string(root.join("hybrid.toml"))? == h.config,
            "hybrid config changed"
        );
        for slot in colm_hybrid::HybridConfig::declared(&root.join("hybrid.toml"))? {
            for path in [slot.model, slot.normalize].into_iter().flatten() {
                let rel = path
                    .strip_prefix(root)
                    .context("hybrid asset outside bundle")?
                    .to_str()
                    .context("asset path is not UTF-8")?;
                validation::relative(rel)?;
                ensure!(
                    h.assets.iter().any(|a| a.relative_path == rel),
                    "hybrid references undeclared asset: {rel}"
                );
            }
        }
        colm_hybrid::HybridConfig::load(&root.join("hybrid.toml"))?;
    }
    Ok(())
}
pub(crate) fn safe_file(root: &Path, relative: &str) -> Result<PathBuf> {
    validation::relative(relative)?;
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    ensure!(
        path.starts_with(&root) && path.is_file(),
        "asset escapes its root or is not a file: {relative}"
    );
    Ok(path)
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn digest_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(digest(&serde_json::to_vec(value)?))
}
pub(crate) fn revision_fingerprint(r: &FrozenRevision) -> Result<String> {
    digest_json(&(
        r.schema,
        &r.version.id,
        &r.name,
        &r.note,
        &r.parent,
        &r.code,
        &r.overlay,
        &r.applicability,
        &r.derived_from,
    ))
}
pub(crate) fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

// Expected values are optimistic edit preconditions, never execution-time assumptions.
fn check_expected_values(changes: &Overlay, baseline: &Overlay) -> Result<()> {
    for record in &changes.parameters {
        let Some(expected) = &record.expected_old_value else {
            continue;
        };
        let prior = baseline
            .parameters
            .iter()
            .find(|p| parameter_key(p) == parameter_key(record))
            .map(|p| &p.value)
            .or_else(|| {
                baseline
                    .physics
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(&record.raw_key))
                    .map(|(_, value)| value)
            });
        let prior = prior.context(
            "cannot verify expected_old_value without an explicit frozen baseline value",
        )?;
        let old = validation::literal(prior)?;
        let expected = validation::literal(expected)?;
        ensure!(
            old == expected
                || old
                    .as_f64()
                    .zip(expected.as_f64())
                    .is_some_and(|(a, b)| a == b),
            "parameter changed since it was read: {}",
            record.parameter_id
        );
    }
    Ok(())
}

/// Directory name of the frozen source snapshot for `code` under `<registry>/sources`.
pub fn source_cache_name(code: &CodeRef) -> Result<String> {
    let digest = Sha256::digest(serde_json::to_vec(code)?);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{}-{}", code.source_sha256, &hex[..16]))
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn a_second_writer_waits_for_the_lock_instead_of_failing() {
        let root = std::env::temp_dir().join(format!("colm-registry-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let registry = Registry::open(&root).unwrap();
        let held = registry.lock().unwrap();
        // Still held after the wait: the original error, not a hang.
        let refused = registry
            .lock_within(Duration::from_millis(100))
            .err()
            .unwrap();
        assert!(format!("{refused:#}").contains("model registry is locked"));
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        let started = Instant::now();
        let second = registry.lock_within(Duration::from_secs(10)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));
        releaser.join().unwrap();
        drop(second);
        assert!(!root.join("write.lock").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
