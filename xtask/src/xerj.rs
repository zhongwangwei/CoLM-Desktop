//! 把固定的 Xerj release 取回并暂存到 Tauri 打包要找的位置。
//!
//! 为什么固定 release：应用托管的 Xerj 节点（`gui/src-tauri/src/xerj_manager.rs`）
//! 与检索侧（`crates/colm-agent/src/retrieval.rs`）的每一个行为假设都是按
//! `v1.0.0-rc.100` 实测的；打包时换一个版本，会让“安装包里的二进制”和“检索端
//! 假设的版本”各自正确、拼起来错。所以 4 个平台资产的 URL 与 sha256 钉在入库的
//! `packaging/xerj/xerj-release.json`，每次使用都重新校验，缓存只加速不降级。
//!
//! 为什么不入库二进制：约 122 MB/平台（压缩后 ~30 MB）。`stage-sidecar` 在打包前
//! 调用这里，把校验过的二进制、LICENSE 和一份不含时间戳的 `xerj-release.json`
//! 放到 `gui/src-tauri/binaries/`，Tauri 的 `externalBin`/`resources` 再打进安装包。
//! 目录被 `.gitignore` 忽略，仓库里没有大文件。
//!
//! 失败策略：网络、sha256、解压、归档内路径、`--version` 任何一步失败都让打包失败，
//! 并给出去查什么。只有明确设置 `COLM_SKIP_XERJ_SIDECAR=1`（本地开发、不打包 Xerj）
//! 才跳过，且跳过会打印在 stdout 上 —— 免得“以为打进了，其实没有”。
//!
//! 归档成员路径校验是纵深防御：sha256 已经钉死归档字节，但下载清单/URL 将来若被
//! 改动，先验路径安全再解压，比任何一次解压后补救都便宜。解压后仍会对 binary/license
//! 做 canonicalize + 目录包含检查，防符号链接把文件指到解压目录之外。

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::sidecar::host_triple;

/// 固定 release 清单（入库）。被钉住的 release 串、4 个平台资产以及各自的 sha256
/// 都只在这里写一次；`pin_*` 测试逐字段守住它不被顺手改掉。
pub(crate) const RELEASE_MANIFEST: &str = "packaging/xerj/xerj-release.json";
/// Tauri `bundle.externalBin` 与 `bundle.resources` 要找的目录。
const BINARIES_DIR: &str = "gui/src-tauri/binaries";
/// 覆盖下载缓存目录（CI 用 actions/cache 预热点就是它）。
pub(crate) const CACHE_ENV: &str = "COLM_XERJ_CACHE";
/// 只供本地开发构建跳过 Xerj；打包流水线绝不应设置它。
pub(crate) const SKIP_ENV: &str = "COLM_SKIP_XERJ_SIDECAR";
/// 固定 release 的官方下载前缀。清单里的每个 URL 都必须落在它下面。
const DOWNLOAD_PREFIX: &str = "https://github.com/xerj-org/xerj/releases/download/";
/// 暂存到 binaries 目录的 LICENSE 文件名（Tauri resources 里同名的键）。
const STAGED_LICENSE: &str = "xerj-LICENSE";
/// 暂存的文件清单：release/target/sha256/url/archive/license 的副本，安装后用于核对。
const STAGED_METADATA: &str = "xerj-release.json";
/// release 矩阵只构建这 4 个 target；清单必须把它们写全，host 不在其中时直接报错。
pub(crate) const SUPPORTED_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// `cargo run -p xtask -- stage-xerj [--verify-only] [--offline] [--force]`。
pub fn command(root: &Path) -> Result<()> {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    let options = parse_options(&arguments)?;
    let manifest = load_manifest(root)?;
    // 用 rustc 的 host triple 选资产：与 `sidecar.rs` 暂存 Colm sidecar 的规则一致，
    // 交叉编译时也不会把别的平台的二进制打进安装包。
    let target = host_triple()?;
    let cache = cache_dir()?;
    stage_with(root, &manifest, &target, &cache, &options)
}

/// `stage-sidecar` 在 6 个 Colm sidecar 之后调用它。
///
/// 跳过逻辑只在这里读环境变量：直接执行 `stage-xerj` 永远照常下载/校验，
/// 避免“开发构建跳过”悄悄污染发布构建。
pub(crate) fn stage_sidecar(root: &Path) -> Result<()> {
    let skip = skip_requested(std::env::var_os(SKIP_ENV).as_deref());
    stage_sidecar_with(root, skip)
}

fn stage_sidecar_with(root: &Path, skip: bool) -> Result<()> {
    if skip {
        // 明确打印，让 CI 日志能看出这个安装包**没有**带 Xerj。
        println!("staged xerj: skipped (development build)");
        return Ok(());
    }
    // 整个 Xerj 阶段（清单、下载、校验、版本）的失败都要带上“本地开发可以跳过”的提示，
    // 否则 CI 上只看到一句 curl 报错，不知道该改网络还是该设置跳过。
    let result = (|| -> Result<()> {
        let manifest = load_manifest(root)?;
        let target = host_triple()?;
        let cache = cache_dir()?;
        stage_with(root, &manifest, &target, &cache, &Options::default())
    })();
    result.with_context(|| {
        format!(
            "cannot stage the pinned Xerj sidecar (release manifest: {}); fix the cause above and retry. \
             {SKIP_ENV}=1 skips Xerj entirely and is only for local development builds that must not ship it",
            RELEASE_MANIFEST
        )
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Options {
    pub verify_only: bool,
    pub offline: bool,
    pub force: bool,
}

fn parse_options(arguments: &[String]) -> Result<Options> {
    let mut options = Options::default();
    for argument in arguments {
        match argument.as_str() {
            "--verify-only" => options.verify_only = true,
            "--offline" => options.offline = true,
            "--force" => options.force = true,
            other => bail!(
                "unknown stage-xerj option {other:?}; supported options are --verify-only, --offline and --force"
            ),
        }
    }
    Ok(options)
}

/// `COLM_SKIP_XERJ_SIDECAR` 只认精确的 `1`：其它值（包括 `true`/`0`/空串）都不跳过，
/// 避免 CI 里一个设置失误变成“发布包悄悄少了 Xerj”。
fn skip_requested(value: Option<&OsStr>) -> bool {
    matches!(value.and_then(OsStr::to_str), Some("1"))
}

// ---------------------------------------------------------------------------
// 固定 release 清单
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub schema: u32,
    pub project: String,
    pub release: String,
    pub license: String,
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asset {
    pub target: String,
    pub url: String,
    pub sha256: String,
    pub archive: Archive,
    pub binary: String,
    pub license: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Archive {
    TarGz,
    Zip,
}

impl Archive {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "tar.gz" => Ok(Self::TarGz),
            "zip" => Ok(Self::Zip),
            other => bail!(
                "unsupported Xerj archive format {other:?}: the supported formats are \"tar.gz\" and \"zip\""
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::Zip => "zip",
        }
    }
}

fn load_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join(RELEASE_MANIFEST);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "cannot read the pinned Xerj release manifest {}",
            path.display()
        )
    })?;
    parse_manifest(&text)
        .with_context(|| format!("invalid pinned Xerj release manifest {}", path.display()))
}

/// 手写 JSON 解析（不新增 serde derive 依赖）：每个字段都是固定契约，逐项校验比
/// 反序列化后再补检查更能给出“哪个 target 的哪个字段错了”。
fn parse_manifest(text: &str) -> Result<Manifest> {
    let value: serde_json::Value =
        serde_json::from_str(text).context("the pinned Xerj release manifest is not valid JSON")?;
    let object = value
        .as_object()
        .context("the pinned Xerj release manifest must be a JSON object")?;

    let schema = object
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .context("the pinned Xerj release manifest field \"schema\" must be an integer")?;
    if schema != 1 {
        bail!("unsupported Xerj release manifest schema {schema}: this xtask understands schema 1 only");
    }
    let project = manifest_string(object, "project")?;
    if project != "xerj" {
        bail!("Xerj release manifest project is {project:?}: expected \"xerj\"");
    }
    let release = manifest_string(object, "release")?;
    if !release.starts_with('v') || release.len() < 2 {
        bail!("Xerj release {release:?} must be a tag starting with \"v\"");
    }
    let license = manifest_string(object, "license")?;
    if license != "Apache-2.0" {
        bail!("Xerj release manifest license is {license:?}: expected \"Apache-2.0\"");
    }

    let assets_value = object
        .get("assets")
        .and_then(serde_json::Value::as_object)
        .context("the pinned Xerj release manifest field \"assets\" must be an object")?;
    let mut assets = BTreeMap::new();
    for (target, raw) in assets_value {
        assets.insert(target.clone(), parse_asset(target, raw)?);
    }
    let missing = SUPPORTED_TARGETS
        .iter()
        .copied()
        .filter(|target| !assets.contains_key(*target))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "pinned Xerj release manifest is missing targets {missing:?}; supported targets are {SUPPORTED_TARGETS:?}"
        );
    }

    Ok(Manifest {
        schema: schema as u32,
        project,
        release,
        license,
        assets,
    })
}

fn manifest_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("the pinned Xerj release manifest field {key:?} must be a string"))
}

fn parse_asset(target: &str, value: &serde_json::Value) -> Result<Asset> {
    let object = value
        .as_object()
        .with_context(|| format!("Xerj release asset {target:?} must be a JSON object"))?;
    let url = manifest_string(object, "url")?;
    if !url.starts_with(DOWNLOAD_PREFIX) {
        bail!("Xerj release asset {target:?} URL must start with {DOWNLOAD_PREFIX}: {url}");
    }
    let sha256 = manifest_string(object, "sha256")?;
    validate_sha256(&sha256).with_context(|| format!("Xerj release asset {target:?}"))?;
    let archive = Archive::parse(&manifest_string(object, "archive")?)
        .with_context(|| format!("Xerj release asset {target:?}"))?;
    let binary = manifest_string(object, "binary")?;
    if !safe_relative_path(&binary) {
        bail!("Xerj release asset {target:?} has an unsafe binary path: {binary:?}");
    }
    if !matches!(binary.rsplit('/').next(), Some("xerj") | Some("xerj.exe")) {
        bail!("Xerj release asset {target:?} binary must be named xerj or xerj.exe: {binary:?}");
    }
    let license = manifest_string(object, "license")?;
    if !safe_relative_path(&license) {
        bail!("Xerj release asset {target:?} has an unsafe license path: {license:?}");
    }
    if license.rsplit('/').next() != Some("LICENSE") {
        bail!("Xerj release asset {target:?} license must be named LICENSE: {license:?}");
    }
    Ok(Asset {
        target: target.to_string(),
        url,
        sha256,
        archive,
        binary,
        license,
    })
}

fn validate_sha256(sha256: &str) -> Result<()> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("sha256 must be 64 lowercase hexadecimal characters: {sha256:?}");
    }
    Ok(())
}

/// 归档内路径与清单内路径共用的安全判据：相对、无 `..`、无反斜杠、无驱动器前缀。
///
/// 允许前缀 `./`（tar 用 `-C dir .` 打包时会这样列），但 `..` 永远拒绝 —— 解压破坏
/// 工作区只需要一个父目录分量。
fn safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') || path.contains('\0') {
        return false;
    }
    let mut path = path;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    if path.is_empty() {
        return false;
    }
    if Path::new(path).is_absolute() {
        return false;
    }
    Path::new(path)
        .components()
        .all(|component| match component {
            std::path::Component::Normal(name) => {
                let name = name.to_string_lossy();
                !name.is_empty() && !name.ends_with(':')
            }
            std::path::Component::CurDir
            | std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => false,
        })
}

fn select_asset<'a>(manifest: &'a Manifest, target: &str) -> Result<&'a Asset> {
    manifest.assets.get(target).with_context(|| {
        format!(
            "pinned Xerj release {} has no asset for host target {target:?}; supported targets are {SUPPORTED_TARGETS:?}",
            manifest.release
        )
    })
}

// ---------------------------------------------------------------------------
// 缓存与下载
// ---------------------------------------------------------------------------

fn cache_dir() -> Result<PathBuf> {
    default_cache_dir_with(
        std::env::var_os(CACHE_ENV).map(PathBuf::from),
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        cfg!(windows),
    )
}

/// 默认缓存目录：unix 走 `$XDG_CACHE_HOME` → `~/.cache`，windows 走 `%LOCALAPPDATA%`；
/// 环境变量全部作为参数传入，测试不需要改进程环境（并行测试互不干扰）。
fn default_cache_dir_with(
    override_dir: Option<PathBuf>,
    xdg_cache_home: Option<PathBuf>,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    windows: bool,
) -> Result<PathBuf> {
    if let Some(override_dir) = override_dir {
        if override_dir.as_os_str().is_empty() {
            bail!("{CACHE_ENV} is set but empty; point it at a writable directory or unset it");
        }
        return Ok(override_dir);
    }
    if windows {
        let local_app_data = local_app_data
            .filter(|directory| !directory.as_os_str().is_empty())
            .with_context(|| {
                format!(
                    "cannot determine the Xerj cache directory on Windows: LOCALAPPDATA is not set; set {CACHE_ENV}"
                )
            })?;
        return Ok(local_app_data.join("colm-xerj"));
    }
    if let Some(xdg_cache_home) =
        xdg_cache_home.filter(|directory| !directory.as_os_str().is_empty())
    {
        return Ok(xdg_cache_home.join("colm-xerj"));
    }
    let home = home
        .filter(|directory| !directory.as_os_str().is_empty())
        .with_context(|| {
            format!("cannot determine the Xerj cache directory: neither XDG_CACHE_HOME nor HOME is set; set {CACHE_ENV}")
        })?;
    Ok(home.join(".cache").join("colm-xerj"))
}

/// 缓存文件名直接取资产 URL 的最后一段：与 release 页上的文件名一致，人肉核对时
/// 不用查映射表；同时也保证不会把 URL 的 query/fragment 带进文件名。
fn cache_file_name(asset: &Asset) -> Result<String> {
    let name = asset.url.rsplit('/').next().unwrap_or_default();
    let name = name.split(['?', '#']).next().unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." || name.contains('\\') || name.contains('\0')
    {
        bail!(
            "cannot derive a cache file name from the pinned Xerj asset URL: {}",
            asset.url
        );
    }
    Ok(name.to_string())
}

/// 进程内唯一的临时后缀（pid + 计数器）：同一进程里多个 stage 并发/连发不撞名，
/// 也不用时间戳 —— 时间戳会让同一份缓存目录在不同次运行里长得不一样。
fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("cannot open {} for sha256", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_sha256_file(path: &Path, expected: &str) -> Result<String> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        bail!(
            "sha256 mismatch for {}: expected {expected}, actual {actual}",
            path.display()
        );
    }
    Ok(actual)
}

fn download_to_cache(asset: &Asset, archive_path: &Path) -> Result<()> {
    let temporary = archive_path.with_file_name(format!(".xerj-download-{}.part", unique_suffix()));
    println!(
        "downloading pinned Xerj {} from {}",
        asset.target, asset.url
    );
    let result = (|| -> Result<()> {
        // 系统 curl：不引入 HTTP 依赖。--retry 3 只重试瞬时故障（连接重置/5xx 前段），
        // 404/403 这类“清单写错了”会立刻失败，不浪费时间。
        let status = Command::new("curl")
            .args(["-fsSL", "--retry", "3", "--connect-timeout", "20", "-o"])
            .arg(&temporary)
            .arg(&asset.url)
            .status()
            .context(
                "cannot run curl: system curl is required to download the pinned Xerj release",
            )?;
        if !status.success() {
            bail!(
                "curl failed to download {} (status {status}); check network/proxy access to github.com, \
                 or set {CACHE_ENV} to a directory that already contains the pinned archive",
                asset.url
            );
        }
        verify_sha256_file(&temporary, &asset.sha256).with_context(|| {
            format!(
                "downloaded Xerj archive for {} does not match the pinned sha256; the download was discarded",
                asset.target
            )
        })?;
        replace_file(&temporary, archive_path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    println!(
        "cached pinned Xerj archive at {} (sha256 {})",
        archive_path.display(),
        asset.sha256
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// 解压与暂存
// ---------------------------------------------------------------------------

fn verify_archive_listing(listing: &str) -> Result<()> {
    for member in listing.lines() {
        let member = member.trim_end_matches('\r');
        if member.is_empty() {
            continue;
        }
        if !safe_relative_path(member) {
            bail!("pinned Xerj archive contains an unsafe member path: {member:?}");
        }
    }
    Ok(())
}

fn verify_archive_members(archive_path: &Path, archive: Archive) -> Result<()> {
    // `tar -tf` 在 GNU tar 与 bsdtar 上都会自动识别 gzip；zip 则只有 bsdtar 认。
    let output = Command::new("tar").arg("-tf").arg(archive_path).output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) if archive == Archive::Zip => {
            // Windows 上的系统 tar 可能是 GNU tar（不认 zip）。归档字节已被 sha256
            // 钉死，这里降级为“解压后对目标文件做目录包含检查”，不阻断打包；
            // 真正决定信任边界的是清单里的 sha256。
            println!(
                "cannot list zip entries with tar ({}); relying on the pinned sha256 and post-extraction path checks",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return Ok(());
        }
        Ok(output) => bail!(
            "cannot list the pinned Xerj archive {} with tar: {}",
            archive_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => bail!("cannot run tar to list the pinned Xerj archive: {error}"),
    };
    verify_archive_listing(&String::from_utf8_lossy(&output.stdout))
}

fn extract_archive(archive_path: &Path, archive: Archive, destination: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    match archive {
        Archive::TarGz => {
            command.arg("-xzf");
        }
        Archive::Zip => {
            command.arg("-xf");
        }
    }
    let status = command
        .arg(archive_path)
        .arg("-C")
        .arg(destination)
        .status()
        .with_context(|| format!("cannot run tar to extract {}", archive_path.display()))?;
    if status.success() {
        return Ok(());
    }
    if archive == Archive::Zip && cfg!(windows) {
        // Windows 上的 GNU tar 不认 zip；PowerShell 5.1 自带 Expand-Archive。
        println!(
            "tar cannot extract the pinned Xerj zip; falling back to PowerShell Expand-Archive"
        );
        return extract_zip_with_powershell(archive_path, destination);
    }
    bail!(
        "tar failed to extract the pinned Xerj archive {} (status {status}); the cached archive may be corrupt — delete it and rerun with --force",
        archive_path.display()
    )
}

/// 只用于 Windows 的 zip 兜底。路径用单引号包住，内部单引号成对转义。
fn extract_zip_with_powershell(archive_path: &Path, destination: &Path) -> Result<()> {
    let quote = |path: &Path| path.display().to_string().replace('\'', "''");
    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        quote(archive_path),
        quote(destination)
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .context("cannot run PowerShell to extract the pinned Xerj zip")?;
    if !status.success() {
        bail!(
            "PowerShell Expand-Archive failed to extract {} (status {status})",
            archive_path.display()
        );
    }
    Ok(())
}

/// 解压后取目标文件：canonicalize 后必须仍在解压目录内，且是普通文件。
/// 这样即使归档里有指向目录外的符号链接，也不会把外部文件拷进安装包。
fn extracted_file(extraction: &Path, relative: &str) -> Result<PathBuf> {
    let root = extraction.canonicalize().with_context(|| {
        format!(
            "cannot resolve the Xerj extraction directory {}",
            extraction.display()
        )
    })?;
    let candidate = extraction.join(relative);
    let resolved = candidate.canonicalize().with_context(|| {
        format!("the pinned Xerj archive does not contain {relative:?} (archive layout changed?)")
    })?;
    if !resolved.starts_with(&root) {
        bail!("pinned Xerj archive member {relative:?} resolves outside the extraction directory");
    }
    if !resolved.is_file() {
        bail!("pinned Xerj archive member {relative:?} is not a regular file");
    }
    Ok(resolved)
}

fn verify_version(binary: &Path, release: &str) -> Result<String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .with_context(|| {
            format!(
                "cannot run {} --version; the pinned Xerj binary is not runnable on this host",
                binary.display()
            )
        })?;
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    }
    if !output.status.success() {
        bail!(
            "{} --version failed with {}: {text}",
            binary.display(),
            output.status
        );
    }
    let first_line = text.lines().next().unwrap_or_default().trim().to_string();
    if !text.contains(release) {
        bail!(
            "{} --version does not report pinned release {release}: {first_line:?}",
            binary.display()
        );
    }
    Ok(first_line)
}

fn staged_binary_path(root: &Path, asset: &Asset) -> PathBuf {
    let extension = if asset.binary.ends_with(".exe") {
        ".exe"
    } else {
        ""
    };
    root.join(BINARIES_DIR)
        .join(format!("xerj-{}{extension}", asset.target))
}

fn staged_license_path(root: &Path) -> PathBuf {
    root.join(BINARIES_DIR).join(STAGED_LICENSE)
}

fn staged_metadata_path(root: &Path) -> PathBuf {
    root.join(BINARIES_DIR).join(STAGED_METADATA)
}

/// 暂存的 release 清单：只放可复现的事实，**不放时间戳**——同一 release 在同一平台
/// 上重跑必须得到逐字节相同的文件，否则安装包 diff 与缓存都会无意义地变化。
fn release_metadata(manifest: &Manifest, asset: &Asset) -> serde_json::Value {
    serde_json::json!({
        "schema": manifest.schema,
        "release": manifest.release,
        "target": asset.target,
        "sha256": asset.sha256,
        "url": asset.url,
        "archive": asset.archive.as_str(),
        "license": manifest.license,
        "licensed_file": STAGED_LICENSE,
    })
}

/// 校验已暂存的 sidecar（`--verify-only` 与幂等跳过共用）：清单字段逐项对、binary
/// 与 LICENSE 存在、binary 可执行、`--version` 含固定 release。
fn check_staged(root: &Path, manifest: &Manifest, asset: &Asset) -> Result<String> {
    let metadata_path = staged_metadata_path(root);
    let text = std::fs::read_to_string(&metadata_path).with_context(|| {
        format!(
            "the Xerj sidecar is not staged: {} is missing; run `cargo run -p xtask -- stage-xerj`",
            metadata_path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).with_context(|| {
        format!(
            "staged Xerj metadata {} is not valid JSON",
            metadata_path.display()
        )
    })?;
    let object = value.as_object().with_context(|| {
        format!(
            "staged Xerj metadata {} must be a JSON object",
            metadata_path.display()
        )
    })?;
    let expected = release_metadata(manifest, asset);
    let expected = expected
        .as_object()
        .context("internal error: Xerj release metadata is not an object")?;
    for (key, expected_value) in expected {
        let actual = object.get(key);
        if actual != Some(expected_value) {
            bail!(
                "staged Xerj metadata {} disagrees with the pinned manifest at {key:?}: expected {expected_value}, actual {}",
                metadata_path.display(),
                actual
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "<missing>".to_string())
            );
        }
    }

    let binary = staged_binary_path(root, asset);
    if !binary.is_file() {
        bail!(
            "the staged Xerj binary {} is missing; run `cargo run -p xtask -- stage-xerj --force`",
            binary.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&binary)
            .with_context(|| format!("cannot read permissions of {}", binary.display()))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            bail!(
                "the staged Xerj binary {} is not executable (mode {mode:o}); run `cargo run -p xtask -- stage-xerj --force`",
                binary.display()
            );
        }
    }
    let license = staged_license_path(root);
    if !license.is_file() {
        bail!(
            "the staged Xerj LICENSE {} is missing; run `cargo run -p xtask -- stage-xerj --force`",
            license.display()
        );
    }

    verify_version(&binary, &manifest.release)
}

fn stage_with(
    root: &Path,
    manifest: &Manifest,
    target: &str,
    cache: &Path,
    options: &Options,
) -> Result<()> {
    let asset = select_asset(manifest, target)?;

    if options.verify_only {
        let version = check_staged(root, manifest, asset)?;
        println!(
            "verified staged xerj {} for {target}: {} ({version})",
            manifest.release,
            staged_binary_path(root, asset).display()
        );
        return Ok(());
    }

    // 幂等：暂存文件已经和固定清单一致就不重复下载（CI 用缓存目录时尤其重要）。
    // `--force` 用于故意重建，或排障时重新校验下载链路。
    if !options.force && check_staged(root, manifest, asset).is_ok() {
        println!(
            "staged xerj {} for {target} is already current at {}; pass --force to re-download",
            manifest.release,
            staged_binary_path(root, asset).display()
        );
        return Ok(());
    }

    std::fs::create_dir_all(cache).with_context(|| {
        format!(
            "cannot create the Xerj download cache directory {}",
            cache.display()
        )
    })?;
    let archive_path = cache.join(cache_file_name(asset)?);

    let mut cache_hit = false;
    if archive_path.is_file() {
        match verify_sha256_file(&archive_path, &asset.sha256) {
            Ok(_) => {
                cache_hit = true;
                println!(
                    "using cached pinned Xerj archive {} (sha256 verified)",
                    archive_path.display()
                );
            }
            Err(error) if options.offline => {
                return Err(error).with_context(|| {
                    format!(
                        "--offline is set and the cached Xerj archive {} is unusable; delete it and rerun without --offline",
                        archive_path.display()
                    )
                });
            }
            Err(error) => {
                println!(
                    "discarding cached Xerj archive that does not match the pinned sha256: {error:#}"
                );
                std::fs::remove_file(&archive_path).with_context(|| {
                    format!(
                        "cannot remove corrupt cached Xerj archive {}",
                        archive_path.display()
                    )
                })?;
            }
        }
    }

    if !cache_hit {
        if options.offline {
            bail!(
                "--offline is set but the pinned Xerj archive is not cached at {}; run `cargo run -p xtask -- stage-xerj` once with network access, or set {CACHE_ENV} to a populated cache directory",
                archive_path.display()
            );
        }
        download_to_cache(asset, &archive_path)?;
    }

    stage_from_archive(root, manifest, asset, cache, &archive_path, cache_hit)
}

fn stage_from_archive(
    root: &Path,
    manifest: &Manifest,
    asset: &Asset,
    cache: &Path,
    archive_path: &Path,
    cache_hit: bool,
) -> Result<()> {
    let extraction = cache.join(format!(".xerj-extract-{}", unique_suffix()));
    let _ = std::fs::remove_dir_all(&extraction);
    std::fs::create_dir_all(&extraction).with_context(|| {
        format!(
            "cannot create the Xerj extraction directory {}",
            extraction.display()
        )
    })?;

    let result = (|| -> Result<()> {
        verify_archive_members(archive_path, asset.archive)?;
        extract_archive(archive_path, asset.archive, &extraction)?;
        let binary = extracted_file(&extraction, &asset.binary)?;
        let license = extracted_file(&extraction, &asset.license)?;
        // 解压目录里的那份先跑一次：`--version` 失败时不用写任何暂存文件。
        verify_version(&binary, &manifest.release)?;

        // 所有校验都通过后才往 binaries/ 写：失败时那里不会有半成品。
        let staged_binary = staged_binary_path(root, asset);
        let staged_license = staged_license_path(root);
        let staged_metadata = staged_metadata_path(root);
        std::fs::create_dir_all(
            staged_binary
                .parent()
                .context("staged Xerj binary has no parent")?,
        )?;
        install_file(&binary, &staged_binary, true)?;
        install_file(&license, &staged_license, false)?;
        install_bytes(
            format!(
                "{}\n",
                serde_json::to_string_pretty(&release_metadata(manifest, asset))?
            )
            .as_bytes(),
            &staged_metadata,
        )?;

        // 对暂存副本再验一遍：copy/权限/版本任何一步在这里出错，比装进安装包后才发现便宜。
        let staged_version = check_staged(root, manifest, asset)?;
        let binary_size = std::fs::metadata(&staged_binary)?.len();
        let license_size = std::fs::metadata(&staged_license)?.len();
        println!("staged xerj {} for {}", manifest.release, asset.target);
        println!(
            "  archive: {} ({})",
            archive_path.display(),
            if cache_hit {
                "cache hit, sha256 verified"
            } else {
                "downloaded, sha256 verified"
            }
        );
        println!(
            "  binary: {} ({binary_size} bytes, --version reports {staged_version:?})",
            staged_binary.display()
        );
        println!(
            "  license: {} ({license_size} bytes)",
            staged_license.display()
        );
        println!("  metadata: {}", staged_metadata.display());
        Ok(())
    })();

    // 解压目录用完即清；清理失败只警告，不让它盖住真正的打包结果。
    match std::fs::remove_dir_all(&extraction) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "warning: cannot remove Xerj extraction directory {}: {error}",
            extraction.display()
        ),
    }
    result
}

fn install_file(source: &Path, destination: &Path, executable: bool) -> Result<()> {
    let extension = destination
        .file_name()
        .context("staged Xerj destination has no file name")?
        .to_string_lossy()
        .to_string();
    let temporary = destination.with_file_name(format!(".{extension}.{}.tmp", unique_suffix()));
    let result = (|| -> Result<()> {
        std::fs::copy(source, &temporary).with_context(|| {
            format!(
                "cannot copy {} to {}",
                source.display(),
                temporary.display()
            )
        })?;
        if executable {
            make_executable(&temporary)?;
        }
        replace_file(&temporary, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn install_bytes(bytes: &[u8], destination: &Path) -> Result<()> {
    let extension = destination
        .file_name()
        .context("staged Xerj destination has no file name")?
        .to_string_lossy()
        .to_string();
    let temporary = destination.with_file_name(format!(".{extension}.{}.tmp", unique_suffix()));
    let result = (|| -> Result<()> {
        std::fs::write(&temporary, bytes)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        replace_file(&temporary, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<()> {
    // Windows 的 rename 不能覆盖已存在文件；先删再换。
    if cfg!(windows) && destination.exists() {
        std::fs::remove_file(destination)
            .with_context(|| format!("cannot replace {}", destination.display()))?;
    }
    std::fs::rename(temporary, destination).with_context(|| {
        format!(
            "cannot move {} into place as {}",
            temporary.display(),
            destination.display()
        )
    })
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .with_context(|| format!("cannot read permissions of {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("cannot make {} executable", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
#[path = "xerj_tests.rs"]
mod xerj_tests;
