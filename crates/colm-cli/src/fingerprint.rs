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
    // mksrfdata 按 CH4 参数文件里的两个开关决定要不要生成湖泊土壤碳与空间 pH，mkinidata 再读
    // 湖泊土壤碳（`MOD_Initialize.F90` 的 `lake_soilc_patches.nc`）。参数文件的其它内容不影响这两段，
    // 所以只记这两个开关的生效值，调 Q10 之类的系数不会让前处理重跑。两个都关（包括没有甲烷）时
    // 不记：与加这一项之前的指纹相同，现有算例不必为此重做前处理。
    if stage == "mksrfdata" || stage == "mkinidata" {
        let value = match colm_srfdata::methane_preprocessing::requirements(&doc, case_nml) {
            Ok(methane) if !methane.lake_soil_carbon && !methane.spatial_ph => None,
            Ok(methane) => Some(format!(
                "allowlakeprod={} use_spatial_ph={}",
                methane.lake_soil_carbon, methane.spatial_ph
            )),
            Err(error) => Some(format!("error: {error:#}")),
        };
        if let Some(value) = value {
            inputs.insert("CH4 preprocessing (DEF_METHANE)".into(), value);
        }
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
            // 与 Fortran 同一套拆法（`,`/`;`、盘符、`null` 占位）；写错了就让指纹带上错误，
            // 改对之后自然对不上、会重跑。
            match colm_namelist::tracer_files::param_file_paths(raw) {
                Ok(paths) => {
                    for file in paths {
                        record_file(
                            &mut files,
                            resolve(case_dir, file.trim_matches(['\'', '"'])),
                            FileKind::SmallConfig,
                        );
                    }
                }
                Err(error) => {
                    inputs.insert(
                        "DEF_TRACER_PARAM_FILES (invalid)".into(),
                        format!("{error:#}"),
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
    // 混合模型（hybrid.toml）：配置文本与它引用的模型、归一化文件的内容合成一个指纹。换了模型或
    // 配置就重跑 colm 段；前处理两段不受影响。没有这个文件时不记（现有算例的指纹不变）。
    if stage == "colm" {
        let hybrid = case_dir.join("hybrid.toml");
        if hybrid.is_file() {
            let value = match colm_hybrid::HybridConfig::load_spec(&hybrid) {
                Ok(config) => config.fingerprint,
                Err(error) => format!("error: {error:#}"),
            };
            inputs.insert("hybrid (hybrid.toml)".into(), value);
            // 气候特征文件（`clim_*` 特征）重算过也要重跑 colm 段。
            for file in crate::hybrid_cmd::climate_files(case_dir) {
                let value = std::fs::read(&file).map_or_else(
                    |error| format!("error: {error}"),
                    |bytes| colm_hybrid::sha256_hex(&bytes),
                );
                inputs.insert(
                    format!(
                        "hybrid climate ({})",
                        file.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    value,
                );
            }
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
    lower == "site_fsitedata"
        || lower.contains("file")
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

/// Stable-enough identity for an explicit Rust-preprocessor source.
///
/// This shares the normal stage-fingerprint policy: small files are hashed in full,
/// while large rasters and source trees use the bounded forms used for rawdata.
pub fn input_identity(path: &Path) -> Result<String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let metadata = std::fs::metadata(&canonical)
        .with_context(|| format!("cannot inspect preprocessing input {}", path.display()))?;
    if metadata.is_dir() {
        return directory_fingerprint(&canonical)
            .map(|hash| format!("dir-sha256:{hash}"))
            .with_context(|| format!("cannot fingerprint preprocessing input {}", path.display()));
    }
    if !metadata.is_file() {
        return Ok(format!("not-a-file:{}", metadata_signature(&metadata)));
    }
    if is_large_data(&canonical) {
        return Ok(format!(
            "sample-sha256:{}:{}",
            sample_file(&canonical).with_context(|| format!(
                "cannot fingerprint preprocessing input {}",
                path.display()
            ))?,
            metadata_signature(&metadata)
        ));
    }
    Ok(format!(
        "sha256:{}",
        sha256_file(&canonical).with_context(|| format!(
            "cannot fingerprint preprocessing input {}",
            path.display()
        ))?
    ))
}

/// 目录树超过这么多文件时不再逐个 stat（见 [`directory_fingerprint`]）。
const PER_FILE_STAT_LIMIT: usize = 20_000;

fn directory_fingerprint(path: &Path) -> std::io::Result<String> {
    tree_fingerprint(path, PER_FILE_STAT_LIMIT)
}

fn tree_fingerprint(path: &Path, per_file_stat_limit: usize) -> std::io::Result<String> {
    // ponytail: directory trees use path/size/mtime only; reading samples from
    // tens of thousands of rawdata files delayed every run before MPI started.
    // Add a persisted content manifest only if silent same-size/mtime rewrites
    // become a real input workflow.
    //
    // 文件很多的树（rawdata 约三十万个文件）连逐个 stat 也太贵：exFAT 外接盘上冷遍历一次要
    // 七分钟，开跑前一直停在「等待 CPU」，并行 stat 也不快（瓶颈在盘）。这种树只记文件名与
    // 各级目录的 len/mtime —— 目录 mtime 随增删、改名而变，同名原地改写不算变更。
    let mut entries = Vec::new();
    list_tree(path, &mut entries)?;
    let mut hash = Sha256::new();
    if entries.iter().filter(|(_, dir)| !dir).count() <= per_file_stat_limit {
        hash_tree(path, path, &mut hash)?;
    } else {
        hash.update(b"names\0");
        hash.update(metadata_signature(&std::fs::metadata(path)?).as_bytes());
        hash.update(b"\n");
        for (p, dir) in &entries {
            let rel = p.strip_prefix(path).unwrap_or(p).to_string_lossy();
            hash.update(rel.as_bytes());
            hash.update(b"\0");
            if *dir {
                hash.update(metadata_signature(&std::fs::metadata(p)?).as_bytes());
                hash.update(b"\0");
            }
            hash.update(b"\n");
        }
    }
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

/// 按名字排序的先序遍历；类型取 readdir 的 d_type，不额外 stat。
fn list_tree(path: &Path, out: &mut Vec<(PathBuf, bool)>) -> std::io::Result<()> {
    let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let dir = entry.file_type()?.is_dir();
        out.push((entry.path(), dir));
        if dir {
            list_tree(&entry.path(), out)?;
        }
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

/// Full-content identity for revision runs. This is deliberately separate from
/// the sampled fingerprints used to decide whether legacy stages can be skipped.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FullInputIdentity {
    pub schema: u32,
    pub algorithm: String,
    pub complete: bool,
    pub data_sha256: Option<String>,
    pub data_settings: BTreeMap<String, String>,
    pub files: BTreeMap<String, FullInputFile>,
    pub unresolved: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FullInputFile {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    pub category: String,
}

pub(crate) fn stable_file_identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    }
    #[cfg(not(unix))]
    {
        format!(
            "{}:{:?}:{:?}",
            metadata.len(),
            metadata.modified(),
            metadata.created()
        )
    }
}

pub(crate) fn full_file(path: &Path, category: &str) -> Result<FullInputFile> {
    let mut file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file(),
        "input is not a regular file: {}",
        path.display()
    );
    let identity = stable_file_identity(&before);
    anyhow::ensure!(
        identity == stable_file_identity(&std::fs::metadata(path)?),
        "input replaced before hashing"
    );
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        hash.update(&buffer[..count]);
    }
    anyhow::ensure!(
        bytes == before.len()
            && identity == stable_file_identity(&file.metadata()?)
            && identity == stable_file_identity(&std::fs::metadata(path)?),
        "input changed during hashing: {}",
        path.display()
    );
    Ok(FullInputFile {
        path: path.to_path_buf(),
        sha256: format!("{:x}", hash.finalize()),
        bytes,
        category: category.into(),
    })
}

impl FullInputIdentity {
    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            schema: 1,
            algorithm: "sha256-full-v1".into(),
            complete: false,
            data_sha256: None,
            data_settings: BTreeMap::new(),
            files: BTreeMap::new(),
            unresolved: vec![reason.into()],
        }
    }
    fn add(&mut self, role: String, path: &Path, category: &str) {
        match full_file(path, category) {
            Ok(value) => {
                self.files.insert(role, value);
            }
            Err(error) => self.unresolved.push(format!("{role}: {error:#}")),
        }
    }
    pub(crate) fn invalidate(&mut self, reason: impl Into<String>) {
        self.complete = false;
        self.data_sha256 = None;
        self.unresolved.push(reason.into());
    }
}

// Comparison identity describes data and evaluation conditions, independently of whether
// experimental physics choices are stored in the case or in a model revision.
fn is_comparison_physics(key: &str) -> bool {
    if colm_model::is_model_field(key) {
        return true;
    }
    let Some(field) = colm_schema::find(key).filter(|field| field.group.is_some()) else {
        return false;
    };
    if matches!(field.name, "DEF_LC_YEAR" | "DEF_LAI_START_YEAR") {
        return false;
    }
    matches!(
        field.name,
        "DEF_USE_LCT"
            | "DEF_USE_PFT"
            | "DEF_USE_PC"
            | "DEF_SUBGRID_SCHEME"
            | "DEF_SOLO_PFT"
            | "DEF_FAST_PC"
            | "DEF_PC_CROP_SPLIT"
            | "DEF_SOILPAR_UPS_FIT"
            | "DEF_SOIL_REFL_SCHEME"
            | "DEF_LULCC_SCHEME"
            | "DEF_USE_LULCC"
            | "DEF_HIGHRESURBAN_ALBEDO"
    ) || matches!(
        colm_case::parameters::field_section(field.name, field.group),
        Some("水热过程" | "生态与生地化" | "示踪剂" | "城市" | "河道与水库" | "数据同化")
    )
}

/// Resolve explicit payloads, forcing-prefix matches and declared data roots.
/// Directory scans are conservative: every regular file is hashed, with stable
/// membership checked before and after; no metadata-only or sampled shortcuts.
pub(crate) fn full_inputs(case: &Path) -> Result<FullInputIdentity> {
    let nml = case.join("case.nml");
    let doc = colm_namelist::parse(&std::fs::read_to_string(&nml)?)?;
    let mut out = FullInputIdentity {
        schema: 1,
        algorithm: "sha256-full-v1".into(),
        complete: false,
        data_sha256: None,
        data_settings: BTreeMap::new(),
        files: BTreeMap::new(),
        unresolved: Vec::new(),
    };
    out.add("configuration:case".into(), &nml, "configuration");
    let forcing = doc
        .get("DEF_forcing_namelist")
        .and_then(string_value)
        .map(|raw| resolve(case, raw))
        .unwrap_or_else(|| case.join("forcing.nml"));
    out.add("configuration:forcing".into(), &forcing, "configuration");
    let force = colm_namelist::parse(&std::fs::read_to_string(&forcing)?)?;
    let force_base = force
        .get("DEF_dir_forcing")
        .and_then(string_value)
        .map(|raw| resolve(forcing.parent().unwrap_or(case), raw));
    for key in force.paths() {
        let Some(value) = force.get(&key) else {
            continue;
        };
        let lower = key.to_ascii_lowercase();
        if lower.contains("fprefix") {
            let Some(raw) = string_value(value)
                .filter(|raw| !raw.is_empty() && !raw.eq_ignore_ascii_case("null"))
            else {
                continue;
            };
            let Some(base) = &force_base else {
                out.unresolved.push("forcing directory is missing".into());
                continue;
            };
            let exact = resolve(base, raw);
            if exact.is_file() {
                out.add(format!("forcing:{lower}"), &exact, "data");
            } else {
                let parent = exact.parent().unwrap_or(base);
                let prefix = exact.file_name().unwrap_or_default().to_string_lossy();
                let mut matched = Vec::new();
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries {
                        let entry = entry?;
                        if entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(prefix.as_ref())
                            && entry.path().is_file()
                        {
                            matched.push(entry.path());
                        }
                    }
                }
                matched.sort();
                if matched.is_empty() {
                    out.unresolved
                        .push(format!("forcing:{lower}: no matching files"));
                }
                for path in matched {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    out.add(format!("forcing:{lower}:{name}"), &path, "data");
                }
            }
        } else if lower != "def_dir_forcing" {
            out.data_settings
                .insert(format!("forcing:{lower}"), value.to_string());
        }
    }
    if !out.files.keys().any(|role| role.starts_with("forcing:")) {
        out.unresolved
            .push("forcing payload inventory is empty".into());
    }
    for key in doc.paths() {
        let Some(value) = doc.get(&key) else {
            continue;
        };
        let lower = key.to_ascii_lowercase();
        if lower == "def_dir_existing_srfdata"
            && !matches!(
                doc.get("USE_srfdata_from_larger_region"),
                Some(colm_namelist::Value::Bool(true))
            )
        {
            continue;
        }
        if lower == "def_hist_vars_namelist" {
            let raw =
                string_value(value).context("DEF_HIST_vars_namelist must be a path string")?;
            if raw.trim().is_empty() || raw.trim().eq_ignore_ascii_case("null") {
                out.data_settings.insert(lower, "default".into());
            } else {
                out.add("case:history_variables".into(), &resolve(case, raw), "data");
            }
            continue;
        }
        if lower.starts_with("def_hist") || lower.starts_with("def_wrst") {
            out.data_settings.insert(lower, value.to_string());
            continue;
        }
        if matches!(
            lower.as_str(),
            "def_dir_output"
                | "def_dir_landdata"
                | "def_dir_restart"
                | "def_dir_history"
                | "def_case_name"
                | "def_forcing_namelist"
        ) {
            continue;
        }
        if lower == "def_tracer_param_files" {
            if let Some(raw) = string_value(value) {
                for (index, path) in colm_namelist::tracer_files::param_file_paths(raw)?
                    .into_iter()
                    .enumerate()
                {
                    out.add(
                        format!("model:tracer:{index}"),
                        &resolve(case, path.trim_matches(['\'', '"'])),
                        "model",
                    );
                }
            }
        } else if looks_like_config_path(&key) {
            if let Some(raw) = string_value(value)
                .filter(|raw| !raw.is_empty() && !raw.eq_ignore_ascii_case("null"))
            {
                let path = resolve(case, raw);
                if !path.exists() && declared_root_unused(&nml, &doc, &lower)? {
                    out.data_settings
                        .insert(format!("unused:{lower}"), "true".into());
                    continue;
                }
                if path.is_dir() {
                    full_directory(&mut out, &format!("case:{lower}"), &path)?;
                } else {
                    out.add(
                        format!("case:{lower}"),
                        &path,
                        if colm_model::is_model_field(&key) {
                            "model"
                        } else {
                            "data"
                        },
                    );
                }
            }
        } else if !is_comparison_physics(&key) {
            out.data_settings.insert(lower, value.to_string());
        }
    }
    for file in crate::hybrid_cmd::climate_files(case) {
        out.add(
            format!(
                "climate:{}",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
            &file,
            "data",
        );
    }
    if case.join(colm_model::RUN_MANIFEST).is_file() {
        let manifest: colm_model::MaterializedRun =
            serde_json::from_slice(&std::fs::read(case.join(colm_model::RUN_MANIFEST))?)?;
        for asset in &manifest.artifact_paths {
            out.add(
                format!(
                    "model:{}",
                    asset
                        .strip_prefix(case.canonicalize()?.as_path())
                        .unwrap_or(asset)
                        .display()
                ),
                &case.join(asset),
                if asset.file_name().is_some_and(|name| name == "case.nml") {
                    "configuration"
                } else {
                    "model"
                },
            );
        }
        for flag in ["--highres-params", "--soil-hyper-albedo-dir"] {
            if let Some(path) = manifest.inputs.get(&format!("execution-option:{flag}")) {
                full_directory(&mut out, &format!("option:{flag}"), Path::new(path))?;
            }
        }
    }
    out.unresolved.sort();
    out.unresolved.dedup();
    anyhow::ensure!(
        out.unresolved.is_empty(),
        "cannot establish full input identity: {}",
        out.unresolved.join("; ")
    );
    out.complete = true;
    if out.complete {
        out.data_sha256 = Some(full_data_digest(&out)?);
    }
    Ok(out)
}

/// Full content hashes establish identity; these guards detect transient changes during execution.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FullInputGuard {
    files: BTreeMap<PathBuf, String>,
    directories: BTreeMap<PathBuf, BTreeMap<PathBuf, String>>,
    forcing_prefixes: BTreeMap<PathBuf, BTreeMap<PathBuf, String>>,
}

pub(crate) fn full_input_guard(
    case: &Path,
    identity: &FullInputIdentity,
) -> Result<FullInputGuard> {
    anyhow::ensure!(
        identity.complete
            && identity.schema == 1
            && identity.algorithm == "sha256-full-v1"
            && identity.data_sha256.is_some(),
        "execution guard requires a complete full-content input identity"
    );
    let files = identity
        .files
        .values()
        .map(|file| {
            let metadata = std::fs::metadata(&file.path)?;
            anyhow::ensure!(
                metadata.is_file(),
                "scientific input is no longer a regular file"
            );
            Ok((file.path.clone(), stable_file_identity(&metadata)))
        })
        .collect::<Result<_>>()?;
    let nml = case.join("case.nml");
    let doc = colm_namelist::parse(&std::fs::read_to_string(&nml)?)?;
    let mut directories = BTreeMap::new();
    for key in doc.paths() {
        let lower = key.to_ascii_lowercase();
        if !looks_like_config_path(&key)
            || lower.starts_with("def_hist")
            || lower.starts_with("def_wrst")
            || matches!(
                lower.as_str(),
                "def_dir_output" | "def_dir_landdata" | "def_dir_restart" | "def_dir_history"
            )
            || (lower == "def_dir_existing_srfdata"
                && !matches!(
                    doc.get("USE_srfdata_from_larger_region"),
                    Some(colm_namelist::Value::Bool(true))
                ))
        {
            continue;
        }
        if let Some(raw) = doc.get(&key).and_then(string_value) {
            let path = resolve(case, raw);
            if path.is_dir() {
                directories.insert(path.clone(), directory_inventory(&path)?);
            }
        }
    }
    let manifest = case.join(colm_model::RUN_MANIFEST);
    if manifest.is_file() {
        let run: colm_model::MaterializedRun = serde_json::from_slice(&std::fs::read(manifest)?)?;
        for flag in ["--highres-params", "--soil-hyper-albedo-dir"] {
            if let Some(path) = run.inputs.get(&format!("execution-option:{flag}")) {
                let path = PathBuf::from(path);
                directories.insert(path.clone(), directory_inventory(&path)?);
            }
        }
    }
    let forcing = doc
        .get("DEF_forcing_namelist")
        .and_then(string_value)
        .map(|raw| resolve(case, raw))
        .unwrap_or_else(|| case.join("forcing.nml"));
    let force = colm_namelist::parse(&std::fs::read_to_string(&forcing)?)?;
    let base = force
        .get("DEF_dir_forcing")
        .and_then(string_value)
        .map(|raw| resolve(forcing.parent().unwrap_or(case), raw));
    let mut forcing_prefixes = BTreeMap::new();
    if let Some(base) = base {
        for key in force
            .paths()
            .into_iter()
            .filter(|key| key.to_ascii_lowercase().contains("fprefix"))
        {
            let Some(raw) = force
                .get(&key)
                .and_then(string_value)
                .filter(|raw| !raw.is_empty() && !raw.eq_ignore_ascii_case("null"))
            else {
                continue;
            };
            let prefix = resolve(&base, raw);
            if prefix.is_file() {
                continue;
            }
            let parent = prefix
                .parent()
                .context("forcing prefix has no parent directory")?;
            let name = prefix
                .file_name()
                .context("forcing prefix has no name")?
                .to_string_lossy()
                .into_owned();
            // A forcing directory may also be the case directory; output/audit writes are not inputs.
            let mut matches = BTreeMap::new();
            for entry in std::fs::read_dir(parent)? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(name.as_str())
                    && entry.path().is_file()
                {
                    matches.insert(entry.path(), stable_file_identity(&entry.metadata()?));
                }
            }
            forcing_prefixes.insert(prefix, matches);
        }
    }
    Ok(FullInputGuard {
        files,
        directories,
        forcing_prefixes,
    })
}

pub(crate) fn verify_input_guard(
    case: &Path,
    identity: &FullInputIdentity,
    expected: &FullInputGuard,
) -> Result<()> {
    anyhow::ensure!(*expected == full_input_guard(case, identity)?,
        "scientific input files or directory membership changed during execution, including a restored payload");
    Ok(())
}

fn full_data_digest(identity: &FullInputIdentity) -> Result<String> {
    let data = identity
        .files
        .iter()
        .filter(|(_, f)| f.category == "data")
        .map(|(role, f)| (role, (&f.sha256, f.bytes)))
        .collect::<BTreeMap<_, _>>();
    Ok(colm_kernel::sha256_hex(&serde_json::to_vec(&(
        &identity.data_settings,
        data,
    ))?))
}

// All declared roots are read-only inputs. Reject cycles, special files and
// changes to membership or file identities while producing the full inventory.
fn directory_inventory(root: &Path) -> Result<BTreeMap<PathBuf, String>> {
    fn visit(
        root: &Path,
        relative: &Path,
        active: &mut Vec<PathBuf>,
        out: &mut BTreeMap<PathBuf, String>,
    ) -> Result<()> {
        let path = root.join(relative);
        let canonical = path.canonicalize()?;
        let metadata = std::fs::metadata(&path)?;
        out.insert(relative.to_path_buf(), stable_file_identity(&metadata));
        if metadata.is_dir() {
            anyhow::ensure!(
                !active.contains(&canonical),
                "input directory symlink cycle: {}",
                path.display()
            );
            active.push(canonical);
            for entry in std::fs::read_dir(&path)? {
                visit(root, &relative.join(entry?.file_name()), active, out)?;
            }
            active.pop();
        } else {
            anyhow::ensure!(
                metadata.is_file(),
                "unsupported scientific input: {}",
                path.display()
            );
        }
        Ok(())
    }
    let mut inventory = BTreeMap::new();
    visit(root, Path::new(""), &mut Vec::new(), &mut inventory)?;
    Ok(inventory)
}
fn full_directory(out: &mut FullInputIdentity, role: &str, root: &Path) -> Result<()> {
    let before = directory_inventory(root)?;
    for relative in before.keys() {
        let path = root.join(relative);
        if path.is_file() {
            let name = relative
                .to_str()
                .context("input filename is not UTF-8")?
                .replace('\\', "/");
            out.files
                .insert(format!("{role}/{name}"), full_file(&path, "data")?);
        }
    }
    anyhow::ensure!(
        before == directory_inventory(root)?,
        "input directory changed during hashing: {}",
        root.display()
    );
    Ok(())
}

/// Frozen local configuration is checked separately from server-side datasets.
pub(crate) fn configuration_identity(case: &Path) -> Result<BTreeMap<String, String>> {
    let case = case.canonicalize()?;
    let doc = colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml"))?)?;
    let forcing = doc
        .get("DEF_forcing_namelist")
        .and_then(string_value)
        .map(|raw| resolve(&case, raw))
        .unwrap_or_else(|| case.join("forcing.nml"));
    let mut files = BTreeMap::from([
        (
            "configuration:case".into(),
            full_file(&case.join("case.nml"), "configuration")?.sha256,
        ),
        (
            "configuration:forcing".into(),
            full_file(&forcing, "configuration")?.sha256,
        ),
    ]);
    let manifest = case.join(colm_model::RUN_MANIFEST);
    if manifest.is_file() {
        let run: colm_model::MaterializedRun = serde_json::from_slice(&std::fs::read(manifest)?)?;
        for asset in run.artifact_paths {
            let path = case.join(asset).canonicalize()?;
            let relative = path
                .strip_prefix(&case)
                .context("model asset escapes frozen case")?;
            files.insert(
                format!("model:{}", relative.display()),
                full_file(&path, "model")?.sha256,
            );
        }
    }
    Ok(files)
}

/// Host paths and rewritten configuration bytes are deliberately not payload
/// identities. The frozen model and effective configuration are checked separately.
pub(crate) fn same_payloads(a: &FullInputIdentity, b: &FullInputIdentity) -> bool {
    let payloads = |v: &FullInputIdentity| {
        v.files
            .iter()
            .filter(|(_, f)| f.category != "configuration")
            .map(|(k, f)| (k.clone(), (f.sha256.clone(), f.bytes, f.category.clone())))
            .collect::<BTreeMap<_, _>>()
    };
    a.complete
        && b.complete
        && a.algorithm == b.algorithm
        && a.data_settings == b.data_settings
        && a.data_sha256 == b.data_sha256
        && payloads(a) == payloads(b)
}

/// Upstream stage fingerprints check relevant settings; full data bytes exclude
/// model-only network assets that do not change surface or initial data.
pub(crate) fn same_data_payloads(a: &FullInputIdentity, b: &FullInputIdentity) -> bool {
    let data = |identity: &FullInputIdentity| {
        identity
            .files
            .iter()
            .filter(|(_, file)| file.category == "data")
            .map(|(key, file)| (key.clone(), (file.sha256.clone(), file.bytes)))
            .collect::<BTreeMap<_, _>>()
    };
    a.complete
        && b.complete
        && a.schema == 1
        && b.schema == 1
        && a.algorithm == "sha256-full-v1"
        && b.algorithm == a.algorithm
        && data(a) == data(b)
}

fn declared_root_unused(nml: &Path, doc: &colm_namelist::Document, field: &str) -> Result<bool> {
    let enabled = |name: &str| match doc.get(name) {
        Some(colm_namelist::Value::Bool(v)) => *v,
        _ => matches!(
            colm_schema::find(name).map(|f| f.default),
            Some(colm_schema::Default::Logical(true))
        ),
    };
    if field == "def_dir_runtime" {
        return Ok(![
            "DEF_USE_BGC",
            "DEF_USE_SNICAR",
            "DEF_USE_OZONEDATA",
            "DEF_URBAN_RUN",
            "DEF_USE_CROP",
            "DEF_METHANE",
        ]
        .iter()
        .any(|key| enabled(key)));
    }
    if field != "def_dir_rawdata" {
        return Ok(false);
    }
    use colm_srfdata::site::{single_point_surface_run_from_namelist, SiteMode};
    let crop = enabled("DEF_USE_CROP");
    let runs = match single_point_surface_run_from_namelist(nml, None, crop) {
        Ok(run) => vec![run],
        Err(_) => {
            let Some(source) = doc.get("SITE_fsitedata").and_then(string_value) else {
                return Ok(false);
            };
            let Ok(site) = netcdf::open(resolve(nml.parent().unwrap_or(Path::new(".")), source))
            else {
                return Ok(false);
            };
            if site.variable("IGBP_classification").is_none()
                || site.variable("USGS_classification").is_none()
            {
                return Ok(false);
            }
            drop(site);
            // A dual-classification site is unused only if both possible kernels need no rawdata.
            let (Ok(igbp), Ok(usgs)) = (
                single_point_surface_run_from_namelist(nml, Some(SiteMode::Igbp), crop),
                single_point_surface_run_from_namelist(nml, Some(SiteMode::Usgs), crop),
            ) else {
                return Ok(false);
            };
            vec![igbp, usgs]
        }
    };
    for s in runs {
        let supplied = s.use_site_lai
            && s.use_site_htop
            && (s.use_site_landtype || s.site_landtype.is_some())
            && s.use_site_soilparameters
            && s.use_site_lakedepth
            && s.use_site_soilreflectance
            && s.use_site_topography
            && (!s.use_bedrock || s.use_site_dbedrock)
            && !matches!(s.mode, SiteMode::Urban)
            && (!matches!(s.mode, SiteMode::Pft | SiteMode::Pc) || s.use_site_pctpfts)
            && (!s.crop_enabled || s.use_site_pctcrop);
        if !supplied
            || !colm_srfdata::site::audit_with_lai_frequency(
                &s.source,
                s.mode,
                None,
                s.crop_enabled,
                s.lai_frequency,
                s.runoff_scheme == 3,
                s.site_landtype,
            )?
            .self_contained()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod full_identity_tests {
    use super::*;
    fn self_contained_site(path: &Path, texture: Option<f64>) {
        use colm_srfdata::site::{REQUIRED_FIELDS, SOIL_RUN_FIELDS};
        let mut file = netcdf::create(path).unwrap();
        file.add_dimension("soil", 8).unwrap();
        file.add_dimension("month", 12).unwrap();
        let names = REQUIRED_FIELDS
            .into_iter()
            .chain(SOIL_RUN_FIELDS)
            .chain([
                "longitude",
                "latitude",
                "IGBP_classification",
                "USGS_classification",
                "canopy_height",
                "LAI_year",
                "LAI_monthly",
                "SAI_monthly",
            ])
            .collect::<std::collections::BTreeSet<_>>();
        for name in names {
            if name == "soil_texture" && texture.is_none() {
                continue;
            }
            let (dimensions, count): (&[&str], usize) = if SOIL_RUN_FIELDS.contains(&name) {
                (&["soil"], 8)
            } else if matches!(name, "LAI_monthly" | "SAI_monthly") {
                (&["month"], 12)
            } else {
                (&[], 1)
            };
            let value = match name {
                "soil_texture" => texture.unwrap(),
                "IGBP_classification" | "USGS_classification" => 10.0,
                "LAI_year" => 2008.0,
                _ => 0.2,
            };
            file.add_variable::<f64>(name, dimensions)
                .unwrap()
                .put_values(&vec![value; count], ..)
                .unwrap();
        }
        file.close().unwrap();
    }

    #[test]
    fn default_vic_site_identity_requires_complete_site_or_real_rawdata() {
        let _netcdf_guard = crate::netcdf_test_guard();
        let root = std::env::temp_dir().join(format!(
            "colm-vic-identity-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let site = root.join("site.nc");
        self_contained_site(&site, Some(8.0));
        std::fs::write(root.join("forcing.nc"), b"driver").unwrap();
        std::fs::write(
            root.join("forcing.nml"),
            "&nl_colm_forcing\nDEF_dir_forcing='.'\nforcing%fprefix(1)='forcing.nc'\n/\n",
        )
        .unwrap();
        let case = format!(
            "&nl_colm\nDEF_CASE_NAME='vic'\nSITE_fsitedata='{}'\nUSE_SITE_landtype=.TRUE.\nDEF_dir_output='{}'\nDEF_dir_rawdata='{}'\n/\n",
            site.display(), root.join("out").display(), root.join("rawdata_unused").display()
        );
        std::fs::write(root.join("case.nml"), &case).unwrap();
        let first = full_inputs(&root).unwrap();
        assert!(first.complete);
        assert_eq!(
            first.files["case:site_fsitedata"].sha256,
            sha256_file(&site).unwrap()
        );
        assert_eq!(first.data_settings["unused:def_dir_rawdata"], "true");
        assert!(!root.join("rawdata_unused").exists());

        self_contained_site(&site, Some(7.0));
        let changed_site = full_inputs(&root).unwrap();
        assert_ne!(first.data_sha256, changed_site.data_sha256);
        assert_ne!(
            first.files["case:site_fsitedata"].sha256,
            changed_site.files["case:site_fsitedata"].sha256
        );

        for texture in [None, Some(99.0)] {
            self_contained_site(&site, texture);
            let audit =
                colm_srfdata::site::audit(&site, colm_srfdata::site::SiteMode::Igbp, None, false)
                    .unwrap();
            assert_eq!(audit.needs_external.len(), 1);
            assert!(audit.needs_external[0].starts_with("soil_texture"));
            let error = full_inputs(&root).unwrap_err().to_string();
            assert!(error.contains("case:def_dir_rawdata"), "{error}");
        }
        self_contained_site(&site, Some(8.0));
        {
            let mut file = netcdf::append(&site).unwrap();
            file.variable_mut("USGS_classification")
                .unwrap()
                .put_values(&[99.0_f64], ..)
                .unwrap();
            file.close().unwrap();
        }
        assert!(full_inputs(&root)
            .unwrap_err()
            .to_string()
            .contains("case:def_dir_rawdata"));
        self_contained_site(&site, Some(8.0));
        std::fs::write(
            root.join("case.nml"),
            case.replace(
                "USE_SITE_landtype",
                "USE_SITE_soilparameters=.FALSE.\nUSE_SITE_landtype",
            ),
        )
        .unwrap();
        assert!(full_inputs(&root)
            .unwrap_err()
            .to_string()
            .contains("case:def_dir_rawdata"));

        std::fs::write(
            root.join("case.nml"),
            case.replace(
                "USE_SITE_landtype",
                "DEF_LAI_MONTHLY=.FALSE.\nUSE_SITE_landtype",
            ),
        )
        .unwrap();
        assert!(
            full_inputs(&root)
                .unwrap_err()
                .to_string()
                .contains("case:def_dir_rawdata"),
            "monthly-only site cannot supply eight-day LAI"
        );

        std::fs::write(root.join("case.nml"), &case).unwrap();
        std::fs::create_dir(root.join("rawdata_unused")).unwrap();
        let raster = root.join("rawdata_unused/soil.nc");
        std::fs::write(&raster, b"soil A").unwrap();
        let before = full_inputs(&root).unwrap();
        assert!(!before.data_settings.contains_key("unused:def_dir_rawdata"));
        std::fs::write(&raster, b"soil B").unwrap();
        let after = full_inputs(&root).unwrap();
        assert_ne!(before.data_sha256, after.data_sha256);
        assert_ne!(
            before.files["case:def_dir_rawdata/soil.nc"].sha256,
            after.files["case:def_dir_rawdata/soil.nc"].sha256
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_hash_detects_same_size_middle_mutation_missed_by_sampling() {
        let dir = std::env::temp_dir().join(format!(
            "colm-full-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("forcing.nc");
        let mut bytes = vec![0_u8; 256 * 1024];
        std::fs::write(&path, &bytes).unwrap();
        let before = full_file(&path, "data").unwrap();
        bytes[128 * 1024] = 1;
        std::fs::write(&path, &bytes).unwrap();
        let after = full_file(&path, "data").unwrap();
        assert_eq!(before.bytes, after.bytes);
        assert_ne!(before.sha256, after.sha256);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn declared_directory_identity_is_full_and_location_independent() {
        let base = std::env::temp_dir().join(format!(
            "colm-tree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let left = base.join("local data");
        let right = base.join("server");
        for root in [&left, &right] {
            std::fs::create_dir_all(root.join("nested")).unwrap();
            std::fs::write(root.join("nested/driver.nc"), vec![7_u8; 200_000]).unwrap();
        }
        let mut a = FullInputIdentity::unavailable("test");
        a.unresolved.clear();
        a.complete = true;
        let mut b = a.clone();
        full_directory(&mut a, "case:def_dir_runtime", &left).unwrap();
        full_directory(&mut b, "case:def_dir_runtime", &right).unwrap();
        a.data_sha256 = Some(full_data_digest(&a).unwrap());
        b.data_sha256 = Some(full_data_digest(&b).unwrap());
        assert!(same_payloads(&a, &b));
        std::fs::write(right.join("nested/new.nc"), b"new input").unwrap();
        full_directory(&mut b, "case:def_dir_runtime", &right).unwrap();
        assert!(!same_payloads(&a, &b));
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn study_input_identity_needs_no_run_manifest_and_tracks_output_selection() {
        let root = std::env::temp_dir().join(format!(
            "colm-full-study-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("forcing.nc"), b"same driver").unwrap();
        std::fs::write(
            root.join("forcing.nml"),
            "&nl_colm_forcing
 DEF_dir_forcing='.'
 forcing%fprefix(1)='forcing.nc'
 forcing%timestep=1800
/
",
        )
        .unwrap();
        let case = "&nl_colm
 DEF_CASE_NAME='demo'
 DEF_forcing_namelist='forcing.nml'
 DEF_USE_PFT=.FALSE.
 DEF_HIST_FREQ='DAY'
/
";
        std::fs::write(root.join("case.nml"), case).unwrap();
        let first = full_inputs(&root).unwrap();
        assert!(first.complete);
        assert!(!root.join(colm_model::RUN_MANIFEST).exists());
        std::fs::write(root.join("case.nml"), case.replace(".FALSE.", ".TRUE.")).unwrap();
        let parameters = full_inputs(&root).unwrap();
        assert_eq!(
            first.data_sha256, parameters.data_sha256,
            "model structure may be compared under the same data conditions"
        );
        for (field, value) in [
            ("DEF_USE_PC", ".true."),
            ("DEF_SUBGRID_SCHEME", "'PC'"),
            ("DEF_USE_BGC", ".true."),
            ("DEF_USE_MEDLYNST", ".true."),
            ("DEF_USE_TRACER", ".true."),
            ("DEF_TUNING_CSOILC", "0.008"),
        ] {
            let modified = case.replace("/\n", &format!(" {field}={value}\n/\n"));
            std::fs::write(root.join("case.nml"), modified).unwrap();
            assert_eq!(
                first.data_sha256,
                full_inputs(&root).unwrap().data_sha256,
                "{field} changes the experiment, not its data conditions"
            );
        }
        for (field, value) in [
            ("DEF_LC_YEAR", "2000"),
            ("DEF_LAI_START_YEAR", "2000"),
            ("DEF_simulation_time%timestep", "900"),
        ] {
            let modified = case.replace("/\n", &format!(" {field}={value}\n/\n"));
            std::fs::write(root.join("case.nml"), modified).unwrap();
            assert_ne!(
                first.data_sha256,
                full_inputs(&root).unwrap().data_sha256,
                "{field} changes data selection or evaluation conditions"
            );
        }
        std::fs::write(root.join("case.nml"), case.replace("'DAY'", "'HOUR'")).unwrap();
        assert_ne!(
            first.data_sha256,
            full_inputs(&root).unwrap().data_sha256,
            "output time sampling is an evaluation condition"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    fn small_case(root: &Path, history: &Path) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("drivers-a.nc"), b"A").unwrap();
        std::fs::write(root.join("forcing.nml"), "&nl_colm_forcing\nDEF_dir_forcing='.'\nforcing%fprefix(1)='drivers-'\nforcing%timestep=1800\n/\n").unwrap();
        std::fs::write(root.join("case.nml"), format!("&nl_colm\nDEF_CASE_NAME='demo'\nDEF_forcing_namelist='forcing.nml'\nDEF_HIST_vars_namelist='{}'\nDEF_HIST_FREQ='HOUR'\n/\n", history.display())).unwrap();
    }

    #[test]
    fn history_selection_identity_follows_bytes_across_clone_and_host_paths() {
        let root = std::env::temp_dir().join(format!(
            "history-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let local = root.join("original");
        let remote = root.join("mapped-server");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&remote).unwrap();
        let a = local.join("selected-source-history.nml");
        let b = remote.join("history.nml");
        let history = "&nl_history\nf_rnet=.TRUE.\nf_lfevpa=.TRUE.\n/\n";
        std::fs::write(&a, history).unwrap();
        std::fs::write(&b, history).unwrap();
        small_case(&local, &a);
        small_case(&remote, &b);
        let left = full_inputs(&local).unwrap();
        let right = full_inputs(&remote).unwrap();
        assert_eq!(left.data_sha256, right.data_sha256);
        assert!(same_payloads(&left, &right));
        assert!(!left.data_settings.contains_key("def_hist_vars_namelist"));
        assert!(left.files.contains_key("case:history_variables"));
        std::fs::write(&b, history.replace("f_lfevpa=.TRUE.", "f_lfevpa=.FALSE.")).unwrap();
        assert_ne!(left.data_sha256, full_inputs(&remote).unwrap().data_sha256);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_guard_allows_case_output_writes_but_rejects_new_forcing_members() {
        let root = std::env::temp_dir().join(format!(
            "guard-members-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let history = root.join("history.nml");
        std::fs::write(&history, "&nl_history\n/\n").unwrap();
        small_case(&root, &history);
        let inputs = full_inputs(&root).unwrap();
        let guard = full_input_guard(&root, &inputs).unwrap();
        std::fs::create_dir_all(root.join("out/history")).unwrap();
        std::fs::write(root.join("stages.json"), "{}").unwrap();
        verify_input_guard(&root, &inputs, &guard).unwrap();
        std::fs::write(root.join("drivers-b.nc"), b"new driver").unwrap();
        assert!(verify_input_guard(&root, &inputs, &guard).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_guard_rejects_restored_readonly_directory_membership() {
        let root = std::env::temp_dir().join(format!(
            "guard-root-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("raw")).unwrap();
        std::fs::write(root.join("raw/driver.nc"), b"data").unwrap();
        let history = root.join("history.nml");
        std::fs::write(&history, "&nl_history\n/\n").unwrap();
        small_case(&root, &history);
        let case = root.join("case.nml");
        std::fs::write(
            &case,
            std::fs::read_to_string(&case)
                .unwrap()
                .replace("DEF_CASE_NAME", "DEF_dir_rawdata='raw'\nDEF_CASE_NAME"),
        )
        .unwrap();
        let inputs = full_inputs(&root).unwrap();
        let guard = full_input_guard(&root, &inputs).unwrap();
        let temporary = root.join("raw/transient.nc");
        std::fs::write(&temporary, b"briefly present").unwrap();
        std::fs::remove_file(temporary).unwrap();
        assert_eq!(
            inputs,
            full_inputs(&root).unwrap(),
            "bytes and final membership alone miss a transient change"
        );
        assert!(verify_input_guard(&root, &inputs, &guard).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn successful_cached_child_cannot_certify_a_restored_input() {
        let root = std::env::temp_dir().join(format!(
            "guard-child-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let history = root.join("history.nml");
        std::fs::write(&history, "&nl_history\n/\n").unwrap();
        small_case(&root, &history);
        let inputs = full_inputs(&root).unwrap();
        let guard = full_input_guard(&root, &inputs).unwrap();
        let child = std::process::Command::new("sh")
            .args([
                "-c",
                "printf B > \"$1\"; printf A > \"$1\"; exit 0",
                "cached-model",
            ])
            .arg(root.join("drivers-a.nc"))
            .status()
            .unwrap();
        assert!(
            child.success(),
            "the execution itself completed successfully"
        );
        assert_eq!(inputs, full_inputs(&root).unwrap());
        assert!(
            verify_input_guard(&root, &inputs, &guard).is_err(),
            "its output cannot be certified as reproducible"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn data_digest_excludes_locations_and_model_but_includes_driver_contents() {
        let mut identity = FullInputIdentity {
            schema: 1,
            algorithm: "sha256-full-v1".into(),
            complete: true,
            data_sha256: None,
            data_settings: BTreeMap::new(),
            files: BTreeMap::new(),
            unresolved: vec![],
        };
        identity.files.insert(
            "forcing:1".into(),
            FullInputFile {
                path: "/old/forcing.nc".into(),
                sha256: "a".repeat(64),
                bytes: 10,
                category: "data".into(),
            },
        );
        let original = full_data_digest(&identity).unwrap();
        identity.files.get_mut("forcing:1").unwrap().path = "/new/forcing.nc".into();
        identity.files.insert(
            "model:parameters".into(),
            FullInputFile {
                path: "params.nml".into(),
                sha256: "b".repeat(64),
                bytes: 30,
                category: "model".into(),
            },
        );
        assert_eq!(original, full_data_digest(&identity).unwrap());
        identity.files.get_mut("forcing:1").unwrap().sha256 = "c".repeat(64);
        assert_ne!(original, full_data_digest(&identity).unwrap());
        identity.invalidate("unresolved external data");
        assert!(!identity.complete);
        assert!(identity.data_sha256.is_none());
    }
}
