//! Project-local, rename-only recycling. Records remain after restore; nothing is permanently deleted.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::*;

const TRASH_DIR: &str = ".colm-trash";
const RECORD: &str = "record.json";
const PAYLOAD: &str = "payload";
const TREE_LIMIT: usize = 10_000;
const DEPTH_LIMIT: usize = 128;

// The absence checks are advisory: the OS must reject destinations created concurrently.
#[allow(unsafe_code)]
fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::ffi::{c_char, c_int, c_uint, CString};
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(source.as_os_str().as_bytes())?;
        let destination = CString::new(destination.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        unsafe extern "C" {
            fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
        }
        #[cfg(target_os = "linux")]
        unsafe extern "C" {
            fn renameat2(
                from_fd: c_int,
                from: *const c_char,
                to_fd: c_int,
                to: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }
        // SAFETY: both NUL-terminated strings remain alive for the call; these APIs retain no pointers.
        let status = unsafe {
            #[cfg(target_os = "macos")]
            {
                renamex_np(
                    source.as_ptr(),
                    destination.as_ptr(),
                    4, /* RENAME_EXCL */
                )
            }
            #[cfg(target_os = "linux")]
            {
                renameat2(
                    -100, /* AT_FDCWD */
                    source.as_ptr(),
                    -100,
                    destination.as_ptr(),
                    1, /* RENAME_NOREPLACE */
                )
            }
        };
        if status == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
            let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
            if value.contains(&0) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "path contains NUL",
                ));
            }
            value.push(0);
            Ok(value)
        };
        let source = wide(source)?;
        let destination = wide(destination)?;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        // SAFETY: both NUL-terminated buffers remain alive for the call; zero flags forbid copying/replacement.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (source, destination);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "atomic no-replace rename is unavailable on this platform",
        ))
    }
}

#[derive(Clone, Copy)]
enum TrashTool {
    Trash,
    List,
    Restore,
}

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(TrashTool::Trash),
        Box::new(TrashTool::List),
        Box::new(TrashTool::Restore),
    ]
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    id: String,
    root: PathBuf,
    original: PathBuf,
    trashed_at_unix_ms: u64,
    kind: String,
    recoverable: bool,
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            bail!("symlinks are forbidden in trash operations")
        }
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn plain_path(scope: &Scope, input: &str) -> Result<PathBuf> {
    let input = Path::new(input);
    validate_path(input)?;
    let relative = if input.is_absolute() {
        input
            .strip_prefix(&scope.selected)
            .or_else(|_| input.strip_prefix(&scope.root))
            .context("path is outside the authorized root")?
    } else {
        input
    };
    let mut path = scope.root.clone();
    for part in relative.components() {
        if let Component::Normal(name) = part {
            path.push(name);
            metadata(&path)?;
        }
    }
    let resolved = scope.resolve(input.to_str().context("path must be UTF-8")?, true)?;
    if path != resolved {
        bail!("trash operations require a path without symlink traversal");
    }
    if path == scope.root {
        bail!("the authorized root cannot be trashed or restored");
    }
    Ok(path)
}

fn area(scope: &Scope, create: bool) -> Result<Option<PathBuf>> {
    let path = scope.root.join(TRASH_DIR);
    if let Some(meta) = metadata(&path)? {
        if !meta.is_dir() {
            bail!("project trash area is not a directory");
        }
        return Ok(Some(path));
    }
    if !create {
        return Ok(None);
    }
    fs::create_dir(&path).context("cannot create project trash area; selected root must exist")?;
    Ok(Some(path))
}

fn validate_tree(actual: &Path, original: &Path, scope: &Scope) -> Result<usize> {
    let mut stack = vec![(actual.to_path_buf(), original.to_path_buf(), 0)];
    let mut count = 1;
    while let Some((actual, original, depth)) = stack.pop() {
        validate_path(&original)?;
        scope.check_mutation(&original)?;
        let meta = metadata(&actual)?.context("trash candidate disappeared")?;
        if meta.is_file() {
            destination(&original, false)?;
        } else if meta.is_dir() {
            if depth >= DEPTH_LIMIT {
                bail!("directory tree exceeds maximum depth {DEPTH_LIMIT}");
            }
            for entry in fs::read_dir(&actual)? {
                let entry = entry?;
                count += 1;
                if count > TREE_LIMIT {
                    bail!("directory tree exceeds {TREE_LIMIT} entries; select a smaller subtree");
                }
                stack.push((entry.path(), original.join(entry.file_name()), depth + 1));
            }
        } else {
            bail!("only regular data/report files and directories can be trashed or restored");
        }
    }
    Ok(count)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

fn load_record(scope: &Scope, area: &Path, id: &str) -> Result<(Record, PathBuf, PathBuf)> {
    if !valid_id(id) {
        bail!("invalid trash id");
    }
    let entry = area.join(id);
    if !metadata(&entry)?.is_some_and(|meta| meta.is_dir()) {
        bail!("trash entry must be a directory");
    }
    let record_path = entry.join(RECORD);
    if !metadata(&record_path)?.is_some_and(|meta| meta.is_file() && meta.len() <= TEXT_LIMIT) {
        bail!("trash record is missing or invalid");
    }
    let mut bytes = Vec::new();
    File::open(record_path)?
        .take(TEXT_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > TEXT_LIMIT {
        bail!("trash record is too large");
    }
    let record: Record = serde_json::from_slice(&bytes).context("invalid trash record")?;
    if record.version != 1
        || record.id != id
        || record.root != scope.root
        || !record.recoverable
        || !matches!(record.kind.as_str(), "file" | "directory")
        // Windows 上 `/tmp/x` 有根但没有盘符，`is_absolute()` 为假：记录里的路径只能是纯相对路径。
        || record.original.is_absolute()
        || record.original.has_root()
        || record.original.as_os_str().is_empty()
    {
        bail!("trash record does not match this root and entry");
    }
    let original = plain_path(
        scope,
        record
            .original
            .to_str()
            .context("original path must be UTF-8")?,
    )?;
    let payload = entry.join(PAYLOAD);
    if let Some(meta) = metadata(&payload)? {
        if (record.kind == "file" && !meta.is_file())
            || (record.kind == "directory" && !meta.is_dir())
        {
            bail!("trash payload type does not match record");
        }
    }
    Ok((record, original, payload))
}

fn trash(scope: &Scope, input: &str) -> Result<Value> {
    let original = plain_path(scope, input)?;
    let count = validate_tree(&original, &original, scope)?;
    let kind = if fs::symlink_metadata(&original)?.is_dir() {
        "directory"
    } else {
        "file"
    };
    let area = area(scope, true)?.context("trash area is unavailable")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("clock precedes Unix epoch")?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = format!(
        "{:x}-{:x}-{:x}",
        now.as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let entry = area.join(&id);
    fs::create_dir(&entry).context("cannot reserve a unique trash entry; original is unchanged")?;
    let record = Record {
        version: 1,
        id: id.clone(),
        root: scope.root.clone(),
        original: original.strip_prefix(&scope.root)?.to_path_buf(),
        trashed_at_unix_ms: u64::try_from(now.as_millis())
            .context("timestamp exceeds supported range")?,
        kind: kind.into(),
        recoverable: true,
    };
    let bytes = serde_json::to_vec(&record)?;
    create_file(&entry.join(RECORD), |file| {
        file.write_all(&bytes)?;
        Ok(bytes.len() as u64)
    })
    .with_context(|| format!("cannot persist trash record {id}; original is unchanged"))?;
    let payload = entry.join(PAYLOAD);
    if fs::symlink_metadata(&payload).is_ok() {
        bail!("trash payload already exists; original is unchanged (id {id})");
    }
    rename_no_replace(&original, &payload).with_context(|| format!("cannot move to project trash (id {id}); no cross-filesystem copy fallback; original or payload remains recoverable"))?;
    Ok(
        json!({"root": scope.root, "id": id, "original": record.original, "type": kind, "trashed_at_unix_ms": record.trashed_at_unix_ms, "recoverable": true, "entries_moved": count}),
    )
}

fn restore(scope: &Scope, id: &str) -> Result<Value> {
    let area = area(scope, false)?.context("project trash is empty")?;
    let (record, original, payload) = load_record(scope, &area, id)?;
    if metadata(&original)?.is_some() {
        bail!("restore destination already exists; no overwrite; trash payload is unchanged");
    }
    let parent = original.parent().context("original has no parent")?;
    if !metadata(parent)?.is_some_and(|meta| meta.is_dir()) {
        bail!("original parent directory must exist before restore; trash payload is unchanged");
    }
    validate_tree(&payload, &original, scope)?;
    rename_no_replace(&payload, &original).context("cannot restore with rename; trash payload remains recoverable; no cross-filesystem fallback")?;
    Ok(
        json!({"root": scope.root, "id": id, "path": original, "original": record.original, "restored": true}),
    )
}

fn list(scope: &Scope) -> Result<Value> {
    let mut result = json!({"root": scope.root, "entries": [], "truncated": false});
    let Some(area) = area(scope, false)? else {
        return Ok(result);
    };
    let mut ids = Vec::new();
    for (index, entry) in fs::read_dir(&area)?.enumerate() {
        if index == SCAN_LIMIT {
            result["truncated"] = json!(true);
            break;
        }
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if valid_id(&id) {
            ids.push(id);
        }
    }
    ids.sort();
    if ids.len() > LIST_LIMIT {
        result["truncated"] = json!(true);
        ids.truncate(LIST_LIMIT);
    }
    let mut entries = Vec::new();
    for id in ids {
        let item = match load_record(scope, &area, &id) {
            Ok((record, original, payload)) => {
                let recoverable = metadata(&payload)?.is_some();
                json!({"id": id, "original": record.original, "type": record.kind, "trashed_at_unix_ms": record.trashed_at_unix_ms, "recoverable": recoverable, "destination_exists": metadata(&original)?.is_some(), "status": if recoverable { "in_trash" } else { "restored_or_incomplete" }})
            }
            Err(_) => {
                json!({"id": id, "recoverable": false, "status": "invalid_record_or_path", "preserved": true})
            }
        };
        entries.push(item);
    }
    result["entries"] = json!(entries);
    while serde_json::to_string(&result)?.chars().count() > MAX_RESULT_CHARS {
        result["entries"]
            .as_array_mut()
            .context("entries must be an array")?
            .pop()
            .context("root exceeds response limit")?;
        result["truncated"] = json!(true);
    }
    Ok(result)
}

impl Tool for TrashTool {
    fn name(&self) -> &'static str {
        match self {
            Self::Trash => "trash_path",
            Self::List => "list_trash",
            Self::Restore => "restore_trash",
        }
    }
    fn description(&self) -> &'static str {
        match self {
            Self::Trash => "Move a data/report file or directory into this project's .colm-trash using rename only. Never deletes permanently, never moves the root. Rejects symlinks, sensitive/source paths and trees over 10000 entries or depth 128. Requires explicit approval every time.",
            Self::List => "List up to 200 project trash records with original relative path, time, type and recoverability. Does not create directories. Restored/incomplete and invalid records remain visible; no automatic cleanup.",
            Self::Restore => "Restore a project trash id to its original location using rename only. Never overwrites; original parent must exist. Validates metadata, scope and payload tree; requires explicit approval every time.",
        }
    }
    fn parameters(&self) -> Value {
        match self {
            Self::Trash => object(
                json!({"path": {"type": "string", "description": "data/report file or directory inside selected project root"}}),
            ),
            Self::List => object(json!({})),
            Self::Restore => object(
                json!({"id": {"type": "string", "description": "id returned by trash_path or list_trash"}}),
            ),
        }
    }
    fn tier(&self) -> Tier {
        if matches!(self, Self::List) {
            Tier::Read
        } else {
            Tier::Act
        }
    }
    fn session_allowance(&self) -> bool {
        false
    }
    fn requires_explicit_approval(&self) -> bool {
        !matches!(self, Self::List)
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let scope = Scope::new(ctx).with_context(|| {
            format!(
                "authorized root: {} (select an explicit project directory)",
                ctx.project_root.display()
            )
        })?;
        let run = || match self {
            Self::Trash => trash(&scope, req_str(args, "path")?),
            Self::List => list(&scope),
            Self::Restore => restore(&scope, req_str(args, "id")?),
        };
        run().with_context(|| format!("authorized root: {}", scope.root.display()))
    }
}

#[cfg(test)]
#[path = "trash_tests.rs"]
mod trash_tests;
