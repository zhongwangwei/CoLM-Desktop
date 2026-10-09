//! Bounded file access shared by the built-in assistant and MCP.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::{object, req_str, Tier, Tool, ToolContext, MAX_RESULT_CHARS};

const TEXT_LIMIT: u64 = 64 * 1024;
const COPY_LIMIT: u64 = 64 * 1024 * 1024;
const LIST_LIMIT: usize = 200;
const SCAN_LIMIT: usize = 10_000;

#[path = "trash.rs"]
mod trash;

#[derive(Clone, Copy)]
enum FileTool {
    Info,
    List,
    Read,
    Mkdir,
    Copy,
    Write,
}

pub(crate) fn tools() -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = [
        FileTool::Info,
        FileTool::List,
        FileTool::Read,
        FileTool::Mkdir,
        FileTool::Copy,
        FileTool::Write,
    ]
    .into_iter()
    .map(|tool| Box::new(tool) as Box<dyn Tool>)
    .collect();
    tools.extend(trash::tools());
    tools
}

fn validate_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.to_string_lossy().contains('\0') {
        bail!("an explicit nonempty path without NUL is required");
    }
    for part in path.components() {
        if part == Component::ParentDir {
            bail!("parent traversal (..) is forbidden");
        }
        if let Component::Normal(name) = part {
            let name = name.to_string_lossy().to_ascii_lowercase();
            if name.starts_with(".env")
                || name == "assistant-keys.json"
                || name.starts_with("assistant-keys.json.")
                || matches!(
                    Path::new(&name).extension().and_then(|ext| ext.to_str()),
                    Some("pem" | "key" | "p12" | "pfx")
                )
                || matches!(
                    name.as_str(),
                    ".colm-trash"
                        | ".git"
                        | ".git-credentials"
                        | ".ssh"
                        | ".aws"
                        | ".codex"
                        | ".claude"
                        | ".config"
                        | ".gnupg"
                        | ".azure"
                        | ".kube"
                        | ".docker"
                        | ".npmrc"
                        | ".netrc"
                        | ".pypirc"
                        | "credentials"
                        | "credentials.json"
                        | "id_rsa"
                        | "id_dsa"
                        | "id_ecdsa"
                        | "id_ed25519"
                )
            {
                bail!("sensitive path component is forbidden: {name}");
            }
        }
    }
    Ok(())
}

/// Canonicalize an existing ancestor without treating a dangling symlink as a missing directory.
fn canonical_future(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            fs::canonicalize(path).with_context(|| format!("cannot resolve {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("path has no existing ancestor")?;
            let name = path.file_name().context("path has no final component")?;
            Ok(canonical_future(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

// ponytail: path checks assume no concurrent hostile directory replacement; use descriptor-relative I/O if that enters the threat model.
struct Scope {
    selected: PathBuf,
    root: PathBuf,
}

// shortcut: path checks and opens can race with local directory replacement, use OS-level isolation for hostile concurrent writers.
impl Scope {
    fn new(ctx: &ToolContext) -> Result<Self> {
        validate_path(&ctx.project_root)?;
        let selected = std::path::absolute(&ctx.project_root)?;
        let root = canonical_future(&selected)?;
        validate_path(&root)?;
        if let Ok(meta) = fs::metadata(&root) {
            if !meta.is_dir() {
                bail!("authorized root is not a directory");
            }
        }
        Ok(Self { selected, root })
    }

    fn resolve(&self, input: &str, mutation: bool) -> Result<PathBuf> {
        let input = Path::new(input);
        validate_path(input)?;
        let relative = if input.is_absolute() {
            input
                .strip_prefix(&self.selected)
                .or_else(|_| input.strip_prefix(&self.root))
                .context("path is outside the authorized root")?
        } else {
            input
        };
        if mutation {
            self.check_mutation(&self.root.join(relative))?;
        }
        let mut path = self.root.clone();
        for part in relative.components() {
            if let Component::Normal(name) = part {
                path.push(name);
                path = canonical_future(&path)?;
                if !path.starts_with(&self.root) {
                    bail!("symlink escapes the authorized root");
                }
                validate_path(&path)?;
            }
        }
        if mutation {
            self.check_mutation(&path)?;
        }
        Ok(path)
    }

    fn check_mutation(&self, path: &Path) -> Result<()> {
        for part in self.selected.components().chain(path.components()) {
            let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "oracle"
                    | "golden"
                    | "crates"
                    | "vendor"
                    | "gui"
                    | "bin"
                    | "kernels"
                    | "kernel"
                    | "target"
                    | "source"
                    | "src"
                    | "engine"
                    | "workspace.json"
                    | "manifest.json"
                    | "stages.json"
            ) || name.starts_with("cargo.")
            {
                bail!("protected source, oracle/golden or metadata path: {name}");
            }
        }
        Ok(())
    }
}

fn destination(path: &Path, text: bool) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "txt" | "md" | "csv" | "tsv" | "json")
        && (text || !matches!(ext.as_str(), "nc" | "dat"))
    {
        bail!("destination must have an allowed data/report suffix (.txt, .md, .csv, .tsv, .json{}); source/configuration files are protected", if text { "" } else { ", .nc, .dat" });
    }
    Ok(())
}

fn regular_file(path: &Path, limit: u64) -> Result<File> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() || meta.len() > limit {
        bail!("expected a regular file of at most {limit} bytes");
    }
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > limit {
        bail!("expected a regular file of at most {limit} bytes");
    }
    Ok(file)
}

fn create_file(path: &Path, write: impl FnOnce(&mut File) -> Result<u64>) -> Result<u64> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context(
            "cannot create destination (overwriting is forbidden; parent directory must exist)",
        )?;
    let result = write(&mut file).and_then(|bytes| {
        file.sync_all()?;
        Ok(bytes)
    });
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

impl Tool for FileTool {
    fn name(&self) -> &'static str {
        match self {
            Self::Info => "path_info",
            Self::List => "list_directory",
            Self::Read => "read_text_file",
            Self::Mkdir => "create_directory",
            Self::Copy => "copy_file",
            Self::Write => "write_text_file",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Info => "Inspect a path inside the explicitly selected project root; report existence, type and size. Sensitive paths are denied.",
            Self::List => "List up to 200 sorted visible entries without recursion in the selected project root; scans at most 10000 entries and reports truncation. Sensitive and escaping symlink entries are hidden.",
            Self::Read => "Read a UTF-8 text file of at most 64 KiB inside the selected project root; return a bounded excerpt with explicit truncation. Sensitive paths, binary/NUL content and larger files are rejected.",
            Self::Mkdir => "Create a directory and missing parents inside the selected project root, including a not-yet-existing root. Existing directories succeed. Sensitive, source and oracle/golden paths are protected.",
            Self::Copy => "Copy a regular file (at most 64 MiB) within the selected project root to a new .txt/.md/.csv/.tsv/.json/.nc/.dat destination. No overwrite; parent must exist. Sensitive, source, metadata and oracle/golden destinations are protected.",
            Self::Write => "Create a new UTF-8 report/data file (at most 64 KiB) within the selected project root, with .txt/.md/.csv/.tsv/.json suffix. No overwrite; parent must exist. Sensitive, source, metadata and oracle/golden destinations are protected.",
        }
    }

    fn parameters(&self) -> Value {
        let path = json!({"type": "string", "description": "relative to selected project root, or an absolute path inside it; use . for the root"});
        match self {
            Self::Copy => object(json!({"source": path, "destination": path})),
            Self::Write => object(
                json!({"path": path, "content": {"type": "string", "description": "UTF-8 text, at most 64 KiB; may be empty"}}),
            ),
            _ => object(json!({"path": path})),
        }
    }

    fn tier(&self) -> Tier {
        match self {
            Self::Info | Self::List | Self::Read => Tier::Read,
            _ => Tier::Act,
        }
    }

    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let scope = Scope::new(ctx).with_context(|| {
            format!(
                "authorized root: {} (select an explicit project directory)",
                ctx.project_root.display()
            )
        })?;
        let run = || -> Result<Value> {
            let key = if matches!(self, Self::Copy) {
                "destination"
            } else {
                "path"
            };
            let path = scope.resolve(req_str(args, key)?, self.tier() == Tier::Act)?;
            let mut result = json!({"root": scope.root, "path": path});
            match self {
                Self::Info => match fs::metadata(&path) {
                    Ok(meta) => {
                        result["exists"] = json!(true);
                        result["type"] = json!(if meta.is_dir() {
                            "directory"
                        } else if meta.is_file() {
                            "file"
                        } else {
                            "other"
                        });
                        result["size_bytes"] = json!(meta.len());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        result["exists"] = json!(false)
                    }
                    Err(error) => return Err(error.into()),
                },
                Self::List => {
                    let mut entries = Vec::new();
                    let mut scan_truncated = false;
                    for (index, entry) in fs::read_dir(&path)?.enumerate() {
                        if index == SCAN_LIMIT {
                            scan_truncated = true;
                            break;
                        }
                        let entry = entry?;
                        let entry_path = entry.path();
                        if scope.resolve(&entry_path.to_string_lossy(), false).is_err() {
                            continue;
                        }
                        let meta = fs::symlink_metadata(&entry_path)?;
                        entries.push(json!({"name": entry.file_name().to_string_lossy(), "type": if meta.file_type().is_symlink() { "symlink" } else if meta.is_dir() { "directory" } else if meta.is_file() { "file" } else { "other" }, "size_bytes": meta.len()}));
                    }
                    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                    result["truncated"] = json!(scan_truncated || entries.len() > LIST_LIMIT);
                    result["scan_truncated"] = json!(scan_truncated);
                    entries.truncate(LIST_LIMIT);
                    result["entries"] = json!(entries);
                    while serde_json::to_string(&result)?.chars().count() > MAX_RESULT_CHARS {
                        result["entries"]
                            .as_array_mut()
                            .context("entries must be an array")?
                            .pop()
                            .context("directory path exceeds result limit")?;
                        result["truncated"] = json!(true);
                    }
                }
                Self::Read => {
                    let mut bytes = Vec::new();
                    regular_file(&path, TEXT_LIMIT)?
                        .take(TEXT_LIMIT + 1)
                        .read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > TEXT_LIMIT || bytes.contains(&0) {
                        bail!("file exceeds 64 KiB or contains binary/NUL content");
                    }
                    let content = String::from_utf8(bytes).context("file is not UTF-8 text")?;
                    let budget = MAX_RESULT_CHARS
                        .saturating_sub(serde_json::to_string(&result)?.chars().count() + 128)
                        .min(16_000);
                    let mut length = 0;
                    let mut end = content.len();
                    for (index, character) in content.char_indices() {
                        length += match character {
                            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                            c if c < '\u{20}' => 6,
                            _ => 1,
                        };
                        if length > budget {
                            end = index;
                            break;
                        }
                    }
                    result["content"] = json!(&content[..end]);
                    result["size_bytes"] = json!(content.len());
                    result["truncated"] = json!(end < content.len());
                }
                Self::Mkdir => {
                    let exists = path.try_exists()?;
                    fs::create_dir_all(&path)?;
                    result["created"] = json!(!exists);
                }
                Self::Write => {
                    destination(&path, true)?;
                    let content = args["content"]
                        .as_str()
                        .context("argument content must be a string")?;
                    if content.len() as u64 > TEXT_LIMIT || content.contains('\0') {
                        bail!("content must be text without NUL, at most 64 KiB");
                    }
                    result["bytes_written"] = json!(create_file(&path, |file| {
                        file.write_all(content.as_bytes())?;
                        Ok(content.len() as u64)
                    })?);
                }
                Self::Copy => {
                    destination(&path, false)?;
                    let source = scope.resolve(req_str(args, "source")?, false)?;
                    let input = regular_file(&source, COPY_LIMIT)?;
                    let expected = input.metadata()?.len();
                    result["bytes_copied"] = json!(create_file(&path, |file| {
                        let copied = std::io::copy(&mut input.take(COPY_LIMIT + 1), file)?;
                        if copied != expected || copied > COPY_LIMIT {
                            bail!("source size changed while copying");
                        }
                        Ok(copied)
                    })?);
                    result["source"] = json!(source);
                }
            }
            Ok(result)
        };
        run().with_context(|| format!("authorized root: {}", scope.root.display()))
    }
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod fs_tests;
