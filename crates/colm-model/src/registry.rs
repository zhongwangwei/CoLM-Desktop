use crate::validation::{self, parameter_key};
use crate::*;
use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
        let path = self.root.join("write.lock");
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).context("model registry is locked; recover a stale write.lock only after confirming its writer exited")?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Lock(path))
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
        validate_overlay(&frozen.overlay)?;
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
        let draft: ModelDraft = read_json(&self.root.join("drafts").join(format!("{id}.json")))?;
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
        draft.overlay = parent.overlay;
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
        let stage = parent.join(format!(".stage-{}", std::process::id()));
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
fn revision_fingerprint(r: &FrozenRevision) -> Result<String> {
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
