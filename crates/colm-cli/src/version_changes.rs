//! 一个模型版本改了什么：代码相对官方源码改了哪些文件，参数、物理方案、过程文件与 AI 网络相对父版本
//! （或任选的另一个版本）有什么不同，以及它由哪些研究得来。
//!
//! 原来版本树只列名字和修订号：要知道一个修改版动了哪些代码，得自己去它的工作区里翻 git，而冻结以后
//! 工作区可能已经删了，只剩注册表里缓存的那份克隆（2026-10-20 梳理"新建修改版"）。这里只读注册表。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use colm_model::{CodeKind, FrozenRevision, Overlay, ParameterOverrideRecord, VersionRef};
use colm_workspace::git::{self, FileChange};
use serde::Serialize;

use crate::Opts;

#[derive(Debug, Serialize)]
pub struct Changes {
    version: VersionRef,
    /// 和谁比：没指定时是父版本；没有父版本（官方版）时为空。
    against: Option<VersionRef>,
    against_is_parent: bool,
    code: CodeChanges,
    parameters: Vec<Change>,
    physics: Vec<Change>,
    process_files: Vec<Change>,
    network: Option<Change>,
    studies: Vec<Study>,
    /// 不是研究声明的出处原文：从算例迁移来的 `case:…`、研究页另存的 `study:…/member:…`。
    origin: Option<String>,
}

#[derive(Debug, Serialize)]
struct CodeChanges {
    kind: CodeKind,
    workspace: Option<String>,
    /// 相对官方源码改了的文件（含祖先修改版的改动）与增删行数；二进制文件行数是 0。
    from_official: Vec<FileChange>,
    touches: colm_workspace::version::Touches,
    /// 与 `against` 内容不同的文件。
    differs: Vec<String>,
    /// 比较不完整时为什么（界面按它给中英文说明）：`official_revisions_differ` 是两个官方修订，
    /// 源码不同但不逐个列出；`different_official_source` 是两个版本基于不同的官方源码，只比了两边
    /// 各自改过的文件。
    note: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Change {
    key: String,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
struct Study {
    study: String,
    /// `network`（训练 AI 网络）或 `parameters`（导入率定参数）。
    purpose: String,
    sites: Vec<String>,
    best_member: Option<String>,
    best_objective: Option<f64>,
    /// "变量 指标"。
    targets: Vec<String>,
}

/// 一个版本的代码在哪里。
enum Side {
    /// 官方源码：缓存里的普通目录（只有源码包时为空）。`root` 是它自己。
    Official {
        plain: Option<PathBuf>,
        root: VersionRef,
    },
    /// 修改版：注册表里缓存的克隆、冻结的提交、它所基于的官方源码那次提交，以及沿父版本找到的官方版。
    Workspace {
        dir: PathBuf,
        commit: String,
        base: String,
        root: Option<VersionRef>,
    },
}

impl Side {
    fn changed(&self) -> Result<Vec<FileChange>> {
        match self {
            Side::Official { .. } => Ok(Vec::new()),
            Side::Workspace {
                dir, commit, base, ..
            } => git::changed_between(dir, base, commit),
        }
    }

    fn root(&self) -> Option<&VersionRef> {
        match self {
            Side::Official { root, .. } => Some(root),
            Side::Workspace { root, .. } => root.as_ref(),
        }
    }
}

pub fn changes(opts: &Opts) -> Result<Changes> {
    let store = crate::model_version::registry(opts)?;
    let version = |id: String, revision: String| -> Result<FrozenRevision> {
        store.get(&VersionRef {
            id,
            revision: revision.parse().context("revision must be a number")?,
        })
    };
    let this = version(opts.need_str("--version")?, opts.need_str("--revision")?)?;
    let (against, against_is_parent) = match opts.get("--against") {
        Some(id) => (
            Some(version(id, opts.need_str("--against-revision")?)?),
            false,
        ),
        // 父版本可能已被整理掉（旧官方修订）：那就只列相对官方源码的改动。
        None => (this.parent.as_ref().and_then(|p| store.get(p).ok()), true),
    };
    let root = |rev: &FrozenRevision| -> Option<VersionRef> {
        let mut current = rev.clone();
        for _ in 0..64 {
            if current.code.kind == CodeKind::Official {
                return Some(current.version);
            }
            current = store.get(current.parent.as_ref()?).ok()?;
        }
        None
    };
    let side = |rev: &FrozenRevision| -> Result<Side> {
        let cache = crate::model_version::cache_dir(opts, &rev.code)?;
        Ok(match (&rev.code.kind, &rev.code.workspace) {
            (CodeKind::Workspace, Some(name)) => {
                let dir = cache.join("workspaces").join(name);
                let info: colm_workspace::Info = serde_json::from_slice(
                    &std::fs::read(dir.join("workspace.json")).with_context(|| {
                        format!(
                            "the code frozen with {}#{} is no longer in the model registry",
                            rev.version.id, rev.version.revision
                        )
                    })?,
                )?;
                Side::Workspace {
                    dir: dir.join("src"),
                    commit: rev
                        .code
                        .commit
                        .clone()
                        .context("a workspace version records its commit")?,
                    base: info.root_commit.unwrap_or(info.base_commit),
                    root: root(rev),
                }
            }
            _ => Side::Official {
                plain: Some(cache.join("source")).filter(|dir| dir.is_dir()),
                root: rev.version.clone(),
            },
        })
    };
    let mine = side(&this)?;
    let from_official = mine.changed()?;
    let touches = colm_workspace::version::touches(from_official.iter().map(|c| c.path.as_str()));
    let (differs, note) = match &against {
        Some(other) => code_differs(&mine, &side(other)?, &this, other)?,
        None => (Vec::new(), None),
    };
    let empty = Overlay::default();
    let before = against.as_ref().map_or(&empty, |other| &other.overlay);
    let (studies, origin) = studies(this.derived_from.as_deref());
    Ok(Changes {
        version: this.version.clone(),
        against: against.as_ref().map(|other| other.version.clone()),
        against_is_parent,
        code: CodeChanges {
            kind: this.code.kind.clone(),
            workspace: this.code.workspace.clone(),
            from_official,
            touches,
            differs,
            note,
        },
        parameters: diff(&parameter_map(before), &parameter_map(&this.overlay)),
        physics: diff(&before.physics, &this.overlay.physics),
        process_files: diff(&asset_map(before), &asset_map(&this.overlay)),
        network: network(before, &this.overlay),
        studies,
        origin,
    })
}

/// 两个版本的代码哪些文件不同：只看两边相对官方源码改过的文件，按内容（git 对象 id）比。
fn code_differs(
    mine: &Side,
    other: &Side,
    this: &FrozenRevision,
    against: &FrozenRevision,
) -> Result<(Vec<String>, Option<&'static str>)> {
    if let (Side::Official { .. }, Side::Official { .. }) = (mine, other) {
        let same = this.code.source_sha256 == against.code.source_sha256;
        return Ok((Vec::new(), (!same).then_some("official_revisions_differ")));
    }
    let same_root = mine.root().is_some() && mine.root() == other.root();
    let paths: BTreeSet<String> = mine
        .changed()?
        .into_iter()
        .chain(other.changed()?)
        .map(|change| change.path)
        .collect();
    let mut differs = Vec::new();
    for path in paths {
        if blob(mine, other, same_root, &path)? != blob(other, mine, same_root, &path)? {
            differs.push(path);
        }
    }
    let note = (!same_root).then_some("different_official_source");
    Ok((differs, note))
}

/// 一个版本里某个文件的内容 id；官方版从另一边的官方基线取（同一份官方源码时内容一样），
/// 否则从官方源码缓存里的普通目录算。都取不到时返回一个不会与任何 id 相等的记号，按"不同"算。
fn blob(side: &Side, other: &Side, same_root: bool, path: &str) -> Result<Option<String>> {
    match side {
        Side::Workspace { dir, commit, .. } => git::blob(dir, commit, path),
        Side::Official { plain, .. } => match (other, plain) {
            (Side::Workspace { dir, base, .. }, _) if same_root => git::blob(dir, base, path),
            (_, Some(plain)) => hash_file(&plain.join(path)),
            (_, None) => Ok(Some(format!("unknown:{path}"))),
        },
    }
}

fn hash_file(file: &Path) -> Result<Option<String>> {
    if !file.is_file() {
        return Ok(None);
    }
    let output = Command::new("git")
        .args(["hash-object", "--no-filters"])
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .context("cannot run git (is it installed?)")?;
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

/// 参数改值按"变量 + 作用范围"认同一条。
fn parameter_key(record: &ParameterOverrideRecord) -> String {
    let scope = &record.scope_instance;
    let mut parts = Vec::new();
    match scope.kind.as_str() {
        "case-scalar" => {}
        kind => parts.push(kind.to_owned()),
    }
    parts.extend(scope.scheme.clone());
    parts.extend(scope.index.map(|index| index.to_string()));
    parts.extend(scope.type_name.clone());
    parts.extend(scope.process_file.clone());
    if parts.is_empty() {
        record.raw_key.clone()
    } else {
        format!("{} [{}]", record.raw_key, parts.join(" "))
    }
}

fn parameter_map(overlay: &Overlay) -> BTreeMap<String, String> {
    overlay
        .parameters
        .iter()
        .map(|record| {
            let unit = record
                .unit
                .as_deref()
                .map(|unit| format!(" {unit}"))
                .unwrap_or_default();
            (parameter_key(record), format!("{}{unit}", record.value))
        })
        .collect()
}

fn asset_map(overlay: &Overlay) -> BTreeMap<String, String> {
    overlay
        .process_files
        .iter()
        .map(|asset| {
            (
                asset.relative_path.clone(),
                asset.sha256.chars().take(12).collect(),
            )
        })
        .collect()
}

fn diff(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<Change> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .map(|key| Change {
            key: key.clone(),
            before: before.get(key).cloned(),
            after: after.get(key).cloned(),
        })
        .collect()
}

/// AI 网络：按配置与各文件的校验和认同一个。
fn network(before: &Overlay, after: &Overlay) -> Option<Change> {
    let id = |overlay: &Overlay| {
        overlay.hybrid.as_ref().map(|bundle| {
            let mut text = bundle.config.clone();
            for asset in &bundle.assets {
                text.push_str(&asset.sha256);
            }
            use sha2::Digest;
            format!("{:x}", sha2::Sha256::digest(text.as_bytes()))
                .chars()
                .take(12)
                .collect::<String>()
        })
    };
    let (before, after) = (id(before), id(after));
    (before != after).then(|| Change {
        key: "network".into(),
        before,
        after,
    })
}

/// 研究声明的出处（见 `model_version::declare_studies`）；别的写法原样交回。
fn studies(derived_from: Option<&str>) -> (Vec<Study>, Option<String>) {
    let Some(text) = derived_from else {
        return (Vec::new(), None);
    };
    let entries = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("declared_training_studies").cloned());
    let Some(serde_json::Value::Array(entries)) = entries else {
        return (Vec::new(), Some(text.to_owned()));
    };
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    let studies = entries
        .iter()
        .map(|entry| Study {
            study: entry["study"].as_str().unwrap_or_default().to_owned(),
            purpose: entry["use"].as_str().unwrap_or("network").to_owned(),
            sites: strings(&entry["sites"]),
            best_member: entry["state"]["best_member"].as_str().map(str::to_owned),
            best_objective: entry["state"]["best_objective"].as_f64(),
            targets: entry["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|target| {
                    format!(
                        "{} {}",
                        target["variable"].as_str().unwrap_or("?"),
                        target["metric"].as_str().unwrap_or("")
                    )
                    .trim_end()
                    .to_owned()
                })
                .collect(),
        })
        .collect();
    (studies, None)
}

#[cfg(test)]
#[path = "version_changes_tests.rs"]
mod version_changes_tests;
