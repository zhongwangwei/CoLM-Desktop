//! Immutable model revisions, case migration and isolated run materialization.
//! No engine, NetCDF, GUI, or remote-execution dependency is required.
mod equivalence;
mod materialize;
mod registry;
mod validation;
pub use equivalence::{is_noop, minimize_physics, same_effect};
pub use materialize::{case_land_cover_scheme, case_process_references};
pub use registry::{source_cache_name, Registry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
pub use validation::{is_model_field, model_overlay, validate_overlay};

pub const SCHEMA: u32 = 1;
pub const BINDING_FILE: &str = "model-binding.json";
pub const CASE_DATA_FILE: &str = "case-data.json";
pub const RUN_MANIFEST: &str = "model-run.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionRef {
    pub id: String,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeKind {
    Official,
    Workspace,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeRef {
    pub kind: CodeKind,
    pub workspace: Option<String>,
    pub commit: Option<String>,
    pub source_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterScopeInstance {
    pub kind: String,
    pub scheme: Option<String>,
    pub index: Option<u8>,
    pub type_name: Option<String>,
    pub process_file: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterOverrideRecord {
    pub parameter_id: String,
    pub raw_key: String,
    pub scope_instance: ParameterScopeInstance,
    pub value: String,
    pub expected_old_value: Option<String>,
    pub unit: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub relative_path: String,
    pub sha256: String,
}
pub type ProcessFile = Asset;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HybridBundle {
    pub config: String,
    pub assets: Vec<Asset>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overlay {
    pub parameters: Vec<ParameterOverrideRecord>,
    pub physics: BTreeMap<String, String>,
    pub hybrid: Option<HybridBundle>,
    pub process_files: Vec<ProcessFile>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Period {
    pub start: String,
    pub end: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Applicability {
    pub sites: Vec<String>,
    pub land_classes: Vec<String>,
    pub calibration: Option<Period>,
    pub validation: Option<Period>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDraft {
    pub id: String,
    pub name: String,
    pub note: String,
    pub parent: VersionRef,
    pub code: CodeRef,
    pub overlay: Overlay,
    #[serde(default)]
    pub applicability: Applicability,
    pub generation: u64,
    #[serde(default)]
    pub derived_from: Option<String>,
    #[serde(default)]
    pub created_unix: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenRevision {
    pub schema: u32,
    pub version: VersionRef,
    pub name: String,
    pub note: String,
    pub parent: Option<VersionRef>,
    pub code: CodeRef,
    pub overlay: Overlay,
    pub overlay_sha256: String,
    pub fingerprint: String,
    pub created_unix: u64,
    pub applicability: Applicability,
    pub derived_from: Option<String>,
}
/// The error a case bound to a revision that is gone reports. The desktop matches
/// "model revision <id>#<n> is no longer in the model registry" to offer a rebind.
pub fn missing_revision_message(version: &VersionRef) -> String {
    format!(
        "model revision {}#{} is no longer in the model registry (unused old official revisions are tidied away); rebind the case to an available revision",
        version.id, version.revision
    )
}
/// What [`Registry::prune_official`] removed (or, for a dry run, would remove).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruneReport {
    pub dry_run: bool,
    pub removed: Vec<VersionRef>,
    /// Official revisions that stay, each with why.
    pub kept: Vec<KeptRevision>,
    /// Revision directories and source snapshots removed.
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptRevision {
    pub version: VersionRef,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseBinding {
    pub schema: u32,
    pub version: VersionRef,
    pub original_case_sha256: String,
    pub migrated_unix: u64,
    pub model_fields: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseData {
    pub schema: u32,
    pub source_case: PathBuf,
    pub original_case_sha256: String,
    pub fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub process_file_bindings: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedRun {
    pub schema: u32,
    pub operation_id: String,
    pub source_case: PathBuf,
    pub run_case: PathBuf,
    pub version: VersionRef,
    pub code: CodeRef,
    pub case_sha256: String,
    pub effective_nml_sha256: String,
    pub overlay_sha256: String,
    pub revision_fingerprint: String,
    pub artifact_paths: Vec<PathBuf>,
    pub inputs: BTreeMap<String, String>,
}
#[cfg(test)]
mod tests;
