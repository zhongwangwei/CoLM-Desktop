//! 「这一段能不能跳过」的判据。
//!
//! **只看产物在不在是不够的。** 改了 `SITE_fsitedata` 或 `DEF_dir_rawdata`，
//! `srfdata.nc` 就失效了，而文件还好好躺在那儿 —— 跳过它等于拿旧地表数据
//! 算新算例，而且没有任何迹象。所以每段跑完记一份**输入指纹**，
//! 下次比对不上就必须重跑，并说出是哪一项变了。

use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 一段完成时，它的输入长什么样。
#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    /// 相关 namelist 字段及其值（原文）
    pub inputs: BTreeMap<String, String>,
    /// 站点文件的 sha256。**必须算内容而不是记路径** ——
    /// 同一个路径下换一份站点文件是最容易发生、也最容易漏掉的一种变更。
    pub site_sha256: String,
    /// 小型外部配置文件内容。旧 stages.json 没有这个字段，读入时默认空。
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// 内核身份：预设 + 上游 commit。换个预设就是换了一套编译期宏，
    /// 地表数据也跟着不同。
    pub kernel: String,
}

/// 一段**不**依赖的字段前缀。
///
/// 反过来列是有理由的：正着列「这一段依赖哪些字段」要枚举两百个名字里的
/// 大部分，漏一个就是静默算错；反着列只需说清楚哪些时间字段与输出设置
/// 影响不到当前阶段，短得多，也容易辩护。
///
/// `DEF_dir_output` 也在里面：它变了，产物就落到别处，
/// 而「产物在不在」那一关本来就会发现。
fn ignored(stage: &str, path: &str, greenwich: bool) -> bool {
    let output_side = path.starts_with("DEF_HIST")
        || path.starts_with("DEF_hist")
        || path.starts_with("DEF_WRST")
        || path == "DEF_dir_output";
    match stage {
        // LAI/城市地表聚合总会读起止年份；本地时间还会用月、日、秒换算 UTC。
        "mksrfdata" => {
            output_side
                || (path.starts_with("DEF_simulation_time") && !surface_time_input(path, greenwich))
        }
        // 初始场取决于起始时刻和 greenwich，但与结束时刻、spin-up 轮数无关。
        "mkinidata" => {
            output_side || (path.starts_with("DEF_simulation_time") && !initial_time_input(path))
        }
        // 主程序什么都依赖。
        _ => false,
    }
}

fn surface_time_input(path: &str, greenwich: bool) -> bool {
    path == "DEF_simulation_time%greenwich"
        || path == "DEF_simulation_time%start_year"
        || path == "DEF_simulation_time%end_year"
        || (!greenwich
            && (path.starts_with("DEF_simulation_time%start")
                || path.starts_with("DEF_simulation_time%end")))
}

fn initial_time_input(path: &str) -> bool {
    path == "DEF_simulation_time%greenwich" || path.starts_with("DEF_simulation_time%start")
}

pub fn compute(stage: &str, case_nml: &Path, kernel: &str) -> Result<Fingerprint> {
    let text = std::fs::read_to_string(case_nml)
        .with_context(|| format!("cannot read {}", case_nml.display()))?;
    let doc = colm_namelist::parse(&text)?;
    // CoLM 的缺省值是 .TRUE.；只有显式设为 false 才需跟踪本地日期的月、日、秒。
    let greenwich = !matches!(
        doc.get("DEF_simulation_time%greenwich"),
        Some(colm_namelist::Value::Bool(false))
    );
    let mut inputs = BTreeMap::new();
    let mut files = BTreeMap::new();
    for p in doc.paths() {
        if ignored(stage, &p, greenwich) {
            continue;
        }
        if let Some(v) = doc.get(&p) {
            inputs.insert(p, v.to_string());
        }
    }
    // 站点文件的内容。读不到就用空串 —— 那时下一关（产物是否存在）会接手，
    // 而在这里报错会让「站点文件还没生成」的正常情形变成硬失败。
    let case_dir = case_nml.parent().unwrap_or_else(|| Path::new("."));
    let site = doc
        .get("SITE_fsitedata")
        .and_then(|v| match v {
            colm_namelist::Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .map(|p| resolve(case_dir, &p))
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| colm_kernel::sha256_hex(&b))
        .unwrap_or_default();
    let forcing = doc
        .get("DEF_forcing_namelist")
        .and_then(string_value)
        .map(|raw| resolve(case_dir, raw))
        .unwrap_or_else(|| case_dir.join("forcing.nml"));
    if stage == "colm" {
        record_file(&mut files, forcing.clone(), FileKind::SmallConfig);
        record_forcing_inventory(&mut files, &forcing);
    }
    for p in doc.paths() {
        if ignored(stage, &p, greenwich) {
            continue;
        }
        let Some(colm_namelist::Value::Str(raw)) = doc.get(&p) else {
            continue;
        };
        if !looks_like_config_path(&p) {
            continue;
        }
        if stage != "colm"
            && (p.eq_ignore_ascii_case("DEF_forcing_namelist")
                || p.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES"))
        {
            continue;
        }
        if raw.eq_ignore_ascii_case("null")
            || raw.trim().is_empty()
            || p == "SITE_fsitedata"
            || p == "DEF_dir_output"
        {
            continue;
        }
        if p.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES") {
            for entry in raw
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
            {
                let file = entry
                    .rsplit_once(':')
                    .map_or(entry, |(_, file)| file)
                    .trim();
                if !file.eq_ignore_ascii_case("null") {
                    record_file(
                        &mut files,
                        resolve(case_dir, file.trim_matches(['\'', '"'])),
                        FileKind::SmallConfig,
                    );
                }
            }
            continue;
        }
        let path = resolve(case_dir, raw);
        if path != forcing {
            record_file(
                &mut files,
                path.clone(),
                if is_large_data(&path) {
                    FileKind::LargeData
                } else {
                    FileKind::SmallConfig
                },
            );
        }
    }
    // Process parameter files are case-local by contract. Include manually
    // added files even when an older case.nml does not list them yet.
    if stage == "colm" {
        record_file(
            &mut files,
            case_dir.join(".colm-study-sample.sha256"),
            FileKind::SmallConfig,
        );
        let entries = std::fs::read_dir(case_dir);
        if let Ok(entries) = entries {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.ends_with(".nml") && name.contains("parameter") {
                    record_file(&mut files, entry.path(), FileKind::SmallConfig);
                }
            }
        }
    }

    Ok(Fingerprint {
        inputs,
        site_sha256: site,
        files,
        kernel: kernel.to_string(),
    })
}

fn looks_like_config_path(field: &str) -> bool {
    let lower = field.to_ascii_lowercase();
    lower.contains("file")
        || lower.contains("namelist")
        || lower.contains("rawdata")
        || lower.contains("dir")
        || lower.ends_with("_data")
        || lower.ends_with("_files")
}

fn is_large_data(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase),
        Some(ext) if matches!(ext.as_str(), "nc" | "nc4" | "h5" | "hdf5")
    )
}

#[derive(Clone, Copy)]
enum FileKind {
    SmallConfig,
    LargeData,
}

fn record_file(out: &mut BTreeMap<String, String>, path: PathBuf, kind: FileKind) {
    let canonical = path.canonicalize().unwrap_or(path);
    let key = canonical.to_string_lossy().into_owned();
    if out.contains_key(&key) {
        return;
    }
    let marker = match (kind, std::fs::metadata(&canonical)) {
        (_, Err(error)) => format!("missing:{:?}", error.kind()),
        (_, Ok(metadata)) if metadata.is_dir() => directory_fingerprint(&canonical)
            .map(|hash| format!("dir-sha256:{hash}"))
            .unwrap_or_else(|error| format!("unreadable-dir:{:?}", error.kind())),
        (_, Ok(metadata)) if !metadata.is_file() => {
            format!("not-a-file:{}", metadata_signature(&metadata))
        }
        (FileKind::SmallConfig, Ok(_)) => std::fs::read(&canonical)
            .map(|bytes| format!("sha256:{}", colm_kernel::sha256_hex(&bytes)))
            .unwrap_or_else(|error| format!("unreadable:{:?}", error.kind())),
        (FileKind::LargeData, Ok(metadata)) => sample_file(&canonical)
            .map(|hash| format!("sample-sha256:{hash}:{}", metadata_signature(&metadata)))
            .unwrap_or_else(|error| format!("unreadable:{:?}", error.kind())),
    };
    out.insert(key, marker);
}

fn directory_fingerprint(path: &Path) -> std::io::Result<String> {
    // ponytail: directory trees use path/size/mtime only; reading samples from
    // tens of thousands of rawdata files delayed every run before MPI started.
    // Add a persisted content manifest only if silent same-size/mtime rewrites
    // become a real input workflow.
    let mut hash = Sha256::new();
    hash_tree(path, path, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn hash_tree(root: &Path, path: &Path, hash: &mut Sha256) -> std::io::Result<()> {
    let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let p = entry.path();
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy();
        let metadata = entry.metadata()?;
        hash.update(rel.as_bytes());
        hash.update(b"\0");
        hash.update(metadata_signature(&metadata).as_bytes());
        hash.update(b"\0");
        if metadata.is_dir() {
            hash_tree(root, &p, hash)?;
        }
        hash.update(b"\n");
    }
    Ok(())
}

fn metadata_signature(metadata: &Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("len:{}:mtime:{modified}", metadata.len())
}

fn sample_file(path: &Path) -> std::io::Result<String> {
    // ponytail: bounded head/tail sampling keeps multi-GB rawdata checks cheap;
    // switch to chunk manifests only if unchanged size+mtime interior rewrites occur.
    const SAMPLE: usize = 64 * 1024;
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut hash = Sha256::new();
    let mut head = vec![0_u8; SAMPLE.min(len as usize)];
    let read = file.read(&mut head)?;
    hash.update(&head[..read]);
    if len > SAMPLE as u64 {
        file.seek(SeekFrom::End(-(SAMPLE as i64)))?;
        let mut tail = vec![0_u8; SAMPLE];
        let read = file.read(&mut tail)?;
        hash.update(&tail[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn record_forcing_inventory(out: &mut BTreeMap<String, String>, forcing_nml: &Path) {
    let Ok(text) = std::fs::read_to_string(forcing_nml) else {
        return;
    };
    let Ok(doc) = colm_namelist::parse(&text) else {
        return;
    };
    let base = doc
        .get("DEF_dir_forcing")
        .and_then(string_value)
        .map(|raw| resolve(forcing_nml.parent().unwrap_or_else(|| Path::new(".")), raw));
    let Some(base) = base else {
        return;
    };
    for field in doc.paths() {
        if !field
            .to_ascii_lowercase()
            .starts_with("def_forcing%fprefix")
        {
            continue;
        }
        let Some(raw) = doc.get(&field).and_then(string_value) else {
            continue;
        };
        if raw.eq_ignore_ascii_case("null") || raw.trim().is_empty() {
            continue;
        }
        let exact = resolve(&base, raw);
        if exact.is_file() {
            record_file(out, exact, FileKind::LargeData);
            continue;
        }
        // Gridded forcing may use a prefix rather than one exact file. Hash the
        // matched payloads too; metadata-only fingerprints missed rewritten
        // forcing with unchanged size/mtime.
        let raw_path = Path::new(raw.trim().trim_matches(['\'', '"']));
        let prefix_dir = raw_path.parent().unwrap_or_else(|| Path::new(""));
        let prefix = raw_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(raw);
        if let Ok(entries) = std::fs::read_dir(base.join(prefix_dir)) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with(prefix) {
                    continue;
                }
                record_file(out, entry.path(), FileKind::LargeData);
            }
        }
    }
}

fn string_value(value: &colm_namelist::Value) -> Option<&str> {
    match value {
        colm_namelist::Value::Str(value) => Some(value),
        _ => None,
    }
}

fn resolve(base: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw.trim().trim_matches(['\'', '"']));
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// 算例目录里那份记录。整体读写，不做增量 —— 三段而已。
pub fn load(case: &Path) -> BTreeMap<String, Fingerprint> {
    std::fs::read_to_string(case.join("stages.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(case: &Path, all: &BTreeMap<String, Fingerprint>) -> Result<()> {
    let p = case.join("stages.json");
    std::fs::write(&p, serde_json::to_string_pretty(all)?)
        .with_context(|| format!("cannot write {}", p.display()))
}

/// 两份指纹的第一处差异，用人话说。
///
/// 只报**第一处**：界面上要的是「为什么要重跑」，一个具体原因比一张
/// 差异表更有用，而全列出来在字段多时会淹没重点。
pub fn first_difference(old: &Fingerprint, new: &Fingerprint) -> Option<String> {
    if old.kernel != new.kernel {
        return Some(format!("内核换了：{} -> {}", old.kernel, new.kernel));
    }
    if old.site_sha256 != new.site_sha256 {
        return Some("站点文件的内容变了".to_string());
    }
    for (k, v) in &new.files {
        match old.files.get(k) {
            None => return Some(format!("新外部输入 {k}")),
            Some(o) if o != v => return Some(format!("外部输入 {k} 变了")),
            _ => {}
        }
    }
    for k in old.files.keys() {
        if !new.files.contains_key(k) {
            return Some(format!("外部输入 {k} 被删掉了"));
        }
    }
    for (k, v) in &new.inputs {
        match old.inputs.get(k) {
            None => return Some(format!("新设了 {k}")),
            Some(o) if o != v => return Some(format!("{k}：{o} -> {v}")),
            _ => {}
        }
    }
    for k in old.inputs.keys() {
        if !new.inputs.contains_key(k) {
            return Some(format!("{k} 被删掉了"));
        }
    }
    None
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod fingerprint_tests;
