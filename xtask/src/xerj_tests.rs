//! `xerj` 的单元测试。
//!
//! 默认**不联网**：下载链路用“预置缓存 + 系统 `tar` 造的假归档”覆盖，网络失败路径由
//! `--offline` 测试覆盖；真实 rc.100 的下载/解压/版本验收由
//! `cargo run -p xtask -- stage-xerj --force` 单独执行（见任务记录）。
//!
//! 假归档里放一个 `#!/bin/sh` 脚本冒充 `xerj`，所以“离线暂存”和“`--verify-only`”
//! 两组测试只在 unix 上跑；Windows 上仍会跑清单/选择/路径/sha/缓存/skip 这些纯逻辑测试。

use super::*;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::process::Command;

/// 与固定清单手动同步的 release；改 release 时这里、清单和 GUI 常量必须一起改。
const FIXTURE_RELEASE: &str = "v1.0.0-rc.100";
const FIXTURE_VERSION: &str = "1.0.0-rc.100";
/// 断言固定清单时用的真 target；假归档也用它（unix 上 shell 脚本可执行，与 host 无关）。
const FIXTURE_TARGET: &str = "aarch64-apple-darwin";
const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// 4 个官方 `.sha256`（与任务书一致）。这是“固定 release”的根：任何一次顺手改清单
/// 都会在这里被逐字符打回。
const PINNED_SHA256: [(&str, &str); 4] = [
    (
        "aarch64-apple-darwin",
        "c88fdcb99c4155d2f7260138c3a5898d0f3cf6b07cd208895137ff03c52b9d1a",
    ),
    (
        "x86_64-apple-darwin",
        "30f6148c68c584ab990eeaa9bc9a05209c2ba2374a5ae90651be17f588edac91",
    ),
    (
        "x86_64-unknown-linux-gnu",
        "25aec3e84b904c995bd2d58abe4f83badb8a6f9d1d56dbe2f22180bada9b00cc",
    ),
    (
        "x86_64-pc-windows-msvc",
        "a58f293c1002c4ce5ce1793afd6fa4d3e7a509916a34044c91a38d4a53937812",
    ),
];

fn temp(tag: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("colm-xerj-xtask-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn repo_manifest() -> Manifest {
    let path = Path::new(REPO_ROOT).join(RELEASE_MANIFEST);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    parse_manifest(&text)
        .unwrap_or_else(|error| panic!("pinned Xerj release manifest must parse: {error:#}"))
}

fn manifest_from_value(value: &serde_json::Value) -> Manifest {
    parse_manifest(&value.to_string()).unwrap()
}

fn manifest_error(value: &serde_json::Value) -> String {
    format!(
        "{:#}",
        parse_manifest(&value.to_string()).expect_err("manifest must be rejected")
    )
}

/// 测试用清单：4 个 target 齐全，sha256 是占位值（校验只看格式）；需要真实 sha 的地方
/// 单独覆盖（假归档测试用真实计算值）。
fn fixture_manifest_value() -> serde_json::Value {
    let mut assets = serde_json::Map::new();
    for target in SUPPORTED_TARGETS {
        let root = format!("xerj-{FIXTURE_VERSION}-{target}");
        let (archive, extension, binary) = if target.contains("windows") {
            ("zip", "zip", "xerj.exe")
        } else {
            ("tar.gz", "tar.gz", "xerj")
        };
        assets.insert(
            target.to_string(),
            serde_json::json!({
                "url": format!("{DOWNLOAD_PREFIX}{FIXTURE_RELEASE}/{root}.{extension}"),
                "sha256": "a".repeat(64),
                "archive": archive,
                "binary": format!("{root}/{binary}"),
                "license": format!("{root}/LICENSE"),
            }),
        );
    }
    serde_json::json!({
        "schema": 1,
        "project": "xerj",
        "release": FIXTURE_RELEASE,
        "license": "Apache-2.0",
        "assets": assets,
    })
}

// ---------------------------------------------------------------------------
// 固定清单
// ---------------------------------------------------------------------------

#[test]
fn fixture_manifest_value_parses_with_all_four_targets() {
    let manifest = manifest_from_value(&fixture_manifest_value());
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.release, FIXTURE_RELEASE);
    assert_eq!(manifest.assets.len(), SUPPORTED_TARGETS.len());
    for target in SUPPORTED_TARGETS {
        assert_eq!(manifest.assets[target].target, target);
    }
}

#[test]
fn pinned_manifest_pins_exactly_the_four_release_assets() {
    let manifest = repo_manifest();
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.project, "xerj");
    assert_eq!(manifest.release, FIXTURE_RELEASE);
    assert_eq!(manifest.license, "Apache-2.0");
    assert_eq!(manifest.assets.len(), SUPPORTED_TARGETS.len());
    for (target, sha256) in PINNED_SHA256 {
        let root = format!("xerj-{FIXTURE_VERSION}-{target}");
        let asset = manifest
            .assets
            .get(target)
            .unwrap_or_else(|| panic!("manifest is missing {target}"));
        assert_eq!(asset.target, target);
        assert_eq!(asset.sha256, sha256, "sha256 drifted for {target}");
        if target.contains("windows") {
            assert_eq!(asset.archive, Archive::Zip);
            assert_eq!(
                asset.url,
                format!("{DOWNLOAD_PREFIX}{FIXTURE_RELEASE}/{root}.zip")
            );
            assert_eq!(asset.binary, format!("{root}/xerj.exe"));
        } else {
            assert_eq!(asset.archive, Archive::TarGz);
            assert_eq!(
                asset.url,
                format!("{DOWNLOAD_PREFIX}{FIXTURE_RELEASE}/{root}.tar.gz")
            );
            assert_eq!(asset.binary, format!("{root}/xerj"));
        }
        assert_eq!(asset.license, format!("{root}/LICENSE"));
        assert!(asset.url.starts_with(DOWNLOAD_PREFIX));
    }
}

#[test]
fn manifest_rejects_unsupported_archive_formats() {
    let mut value = fixture_manifest_value();
    value["assets"][FIXTURE_TARGET]["archive"] = "tar.xz".into();
    let error = manifest_error(&value);
    assert!(error.contains("archive format"), "{error}");
    assert!(error.contains("tar.xz"), "{error}");
}

#[test]
fn manifest_rejects_short_or_non_hex_sha256() {
    let mut value = fixture_manifest_value();
    value["assets"][FIXTURE_TARGET]["sha256"] = "a".repeat(63).into();
    assert!(manifest_error(&value).contains("64 lowercase hexadecimal"));
    value["assets"][FIXTURE_TARGET]["sha256"] = "z".repeat(64).into();
    assert!(manifest_error(&value).contains("64 lowercase hexadecimal"));
    value["assets"][FIXTURE_TARGET]["sha256"] = "A".repeat(64).into();
    assert!(manifest_error(&value).contains("64 lowercase hexadecimal"));
}

#[test]
fn manifest_rejects_urls_outside_the_pinned_release_repository() {
    let mut value = fixture_manifest_value();
    value["assets"][FIXTURE_TARGET]["url"] =
        "http://github.com/xerj-org/xerj/releases/download/v1.0.0-rc.100/x.tar.gz".into();
    assert!(manifest_error(&value).contains(DOWNLOAD_PREFIX));
    value["assets"][FIXTURE_TARGET]["url"] = "https://example.com/xerj-1.0.0-rc.100.tar.gz".into();
    assert!(manifest_error(&value).contains(DOWNLOAD_PREFIX));
}

#[test]
fn manifest_rejects_unsafe_binary_and_license_paths() {
    for unsafe_binary in [
        "../xerj",
        "/usr/local/bin/xerj",
        "xerj-root\\xerj",
        "C:/xerj",
    ] {
        let mut value = fixture_manifest_value();
        value["assets"][FIXTURE_TARGET]["binary"] = unsafe_binary.into();
        assert!(
            manifest_error(&value).contains("unsafe binary path"),
            "{unsafe_binary:?} must be rejected"
        );
    }
    let mut value = fixture_manifest_value();
    value["assets"][FIXTURE_TARGET]["binary"] = "xerj-root/not-xerj".into();
    assert!(manifest_error(&value).contains("xerj or xerj.exe"));
    let mut value = fixture_manifest_value();
    value["assets"][FIXTURE_TARGET]["license"] = "xerj-root/../LICENSE".into();
    assert!(manifest_error(&value).contains("unsafe license path"));
}

#[test]
fn manifest_requires_all_four_targets() {
    let mut value = fixture_manifest_value();
    value["assets"]
        .as_object_mut()
        .unwrap()
        .remove("x86_64-pc-windows-msvc");
    let error = manifest_error(&value);
    assert!(error.contains("missing targets"), "{error}");
    assert!(error.contains("x86_64-pc-windows-msvc"), "{error}");
    for target in SUPPORTED_TARGETS {
        assert!(error.contains(target), "{target} must be listed in {error}");
    }
}

#[test]
fn manifest_rejects_wrong_schema_project_or_release() {
    let mut value = fixture_manifest_value();
    value["schema"] = 2.into();
    assert!(manifest_error(&value).contains("schema 2"));
    let mut value = fixture_manifest_value();
    value["project"] = "something-else".into();
    assert!(manifest_error(&value).contains("project"));
    let mut value = fixture_manifest_value();
    value["release"] = "1.0.0-rc.100".into();
    assert!(manifest_error(&value).contains("starting with"));
    let mut value = fixture_manifest_value();
    value["license"] = "MIT".into();
    assert!(manifest_error(&value).contains("Apache-2.0"));
}

// ---------------------------------------------------------------------------
// target 选择与路径安全
// ---------------------------------------------------------------------------

#[test]
fn asset_selection_is_by_host_triple_and_lists_supported_targets() {
    let manifest = repo_manifest();
    for target in SUPPORTED_TARGETS {
        assert_eq!(select_asset(&manifest, target).unwrap().target, target);
    }
    let error = format!(
        "{:#}",
        select_asset(&manifest, "riscv64gc-unknown-linux-gnu").unwrap_err()
    );
    assert!(error.contains("riscv64gc-unknown-linux-gnu"), "{error}");
    for target in SUPPORTED_TARGETS {
        assert!(error.contains(target), "error must list {target}: {error}");
    }
    // host triple 必须真的能走到选择逻辑；release 矩阵只覆盖这 4 个 target。
    let host = host_triple().expect("rustc -vV must report a host triple");
    assert!(!host.is_empty());
    if SUPPORTED_TARGETS.contains(&host.as_str()) {
        assert_eq!(select_asset(&manifest, &host).unwrap().target, host);
    }
}

#[test]
fn safe_relative_paths_reject_absolute_parent_and_backslash() {
    for safe in [
        "xerj",
        "xerj-root/xerj",
        "./xerj-root/LICENSE",
        "with space/xerj",
    ] {
        assert!(safe_relative_path(safe), "{safe:?} should be safe");
    }
    for unsafe_path in [
        "",
        "/usr/local/bin/xerj",
        "../xerj",
        "xerj-root/../xerj",
        "xerj-root\\xerj",
        "C:/xerj",
        "C:\\xerj",
        "xerj-root/\0xerj",
        "./",
    ] {
        assert!(
            !safe_relative_path(unsafe_path),
            "{unsafe_path:?} must be rejected"
        );
    }
}

#[test]
fn archive_listing_rejects_parent_absolute_and_backslash_members() {
    verify_archive_listing(
        "xerj-1.0.0-rc.100-aarch64-apple-darwin/\n\
         xerj-1.0.0-rc.100-aarch64-apple-darwin/xerj\n\
         xerj-1.0.0-rc.100-aarch64-apple-darwin/LICENSE\n",
    )
    .expect("a normal archive listing must pass");
    verify_archive_listing("./xerj-root/LICENSE\n").expect("a ./ prefix is harmless");
    for unsafe_member in ["../evil\n", "/etc/passwd\n", "a\\b\n", "a/../inject\n"] {
        let error = format!("{:#}", verify_archive_listing(unsafe_member).unwrap_err());
        assert!(
            error.contains("unsafe member path"),
            "{unsafe_member:?}: {error}"
        );
    }
}

// ---------------------------------------------------------------------------
// sha256 / 缓存
// ---------------------------------------------------------------------------

#[test]
fn sha256_verification_reports_expected_and_actual() {
    let root = temp("sha256");
    let file = root.join("payload.bin");
    fs::write(&file, b"pinned bytes").unwrap();
    let actual = sha256_file(&file).unwrap();
    assert_eq!(actual, format!("{:x}", Sha256::digest(b"pinned bytes")));
    verify_sha256_file(&file, &actual).expect("matching sha256 must pass");

    let expected = "b".repeat(64);
    let error = format!("{:#}", verify_sha256_file(&file, &expected).unwrap_err());
    assert!(error.contains("sha256 mismatch"), "{error}");
    assert!(
        error.contains(&expected),
        "error must include expected: {error}"
    );
    assert!(
        error.contains(&actual),
        "error must include actual: {error}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn cache_file_names_follow_the_pinned_asset_names() {
    let manifest = repo_manifest();
    assert_eq!(
        cache_file_name(&manifest.assets["aarch64-apple-darwin"]).unwrap(),
        format!("xerj-{FIXTURE_VERSION}-aarch64-apple-darwin.tar.gz")
    );
    assert_eq!(
        cache_file_name(&manifest.assets["x86_64-pc-windows-msvc"]).unwrap(),
        format!("xerj-{FIXTURE_VERSION}-x86_64-pc-windows-msvc.zip")
    );

    let mut asset = manifest.assets["aarch64-apple-darwin"].clone();
    asset.url = format!("{}?download=1#fragment", asset.url);
    assert_eq!(
        cache_file_name(&asset).unwrap(),
        format!("xerj-{FIXTURE_VERSION}-aarch64-apple-darwin.tar.gz")
    );
    asset.url = "https://github.com/xerj-org/xerj/releases/download/".into();
    assert!(cache_file_name(&asset).is_err());
}

#[test]
fn default_cache_directory_follows_platform_conventions() {
    let home = PathBuf::from("/home/example");
    let xdg_cache_home = PathBuf::from("/home/example/cache-root");
    let local_app_data = PathBuf::from("/home/example/AppData/Local");
    let override_dir = PathBuf::from("/tmp/colm-xerj-cache");
    let with =
        |override_dir: Option<PathBuf>,
         xdg: Option<PathBuf>,
         home: Option<PathBuf>,
         local: Option<PathBuf>,
         windows: bool| default_cache_dir_with(override_dir, xdg, home, local, windows);

    assert_eq!(
        with(
            Some(override_dir.clone()),
            Some(xdg_cache_home.clone()),
            Some(home.clone()),
            Some(local_app_data.clone()),
            false,
        )
        .unwrap(),
        override_dir
    );
    assert_eq!(
        with(
            None,
            Some(xdg_cache_home.clone()),
            Some(home.clone()),
            Some(local_app_data.clone()),
            false,
        )
        .unwrap(),
        xdg_cache_home.join("colm-xerj")
    );
    assert_eq!(
        with(
            None,
            None,
            Some(home.clone()),
            Some(local_app_data.clone()),
            false,
        )
        .unwrap(),
        home.join(".cache").join("colm-xerj")
    );
    // Windows 只看 LOCALAPPDATA，不拿 HOME/XDG 猜测。
    assert_eq!(
        with(
            None,
            Some(xdg_cache_home.clone()),
            Some(home.clone()),
            Some(local_app_data.clone()),
            true,
        )
        .unwrap(),
        local_app_data.join("colm-xerj")
    );
    let error = format!(
        "{:#}",
        with(None, Some(xdg_cache_home), Some(home), None, true).unwrap_err()
    );
    assert!(error.contains(CACHE_ENV), "{error}");
    let error = format!("{:#}", with(None, None, None, None, false).unwrap_err());
    assert!(error.contains(CACHE_ENV), "{error}");
}

#[test]
fn unique_temporary_names_do_not_use_timestamps() {
    let first = unique_suffix();
    let second = unique_suffix();
    assert_ne!(first, second);
    assert!(first.starts_with(&format!("{}-", std::process::id())));
}

// ---------------------------------------------------------------------------
// 命令行开关与 skip 逻辑
// ---------------------------------------------------------------------------

#[test]
fn stage_xerj_options_are_parsed_strictly() {
    assert_eq!(parse_options(&[]).unwrap(), Options::default());
    assert_eq!(
        parse_options(&[
            "--verify-only".to_string(),
            "--offline".to_string(),
            "--force".to_string(),
        ])
        .unwrap(),
        Options {
            verify_only: true,
            offline: true,
            force: true,
        }
    );
    let error = format!("{:#}", parse_options(&["--nope".to_string()]).unwrap_err());
    assert!(error.contains("--nope"), "{error}");
    assert!(error.contains("--verify-only"), "{error}");
    assert!(error.contains("--offline"), "{error}");
    assert!(error.contains("--force"), "{error}");
}

#[test]
fn skip_flag_only_accepts_exactly_one() {
    assert!(skip_requested(Some(OsStr::new("1"))));
    assert!(!skip_requested(None));
    for value in ["0", "true", "TRUE", "yes", "", " 1", "1 "] {
        assert!(
            !skip_requested(Some(OsStr::new(value))),
            "{value:?} must not skip Xerj staging"
        );
    }
}

#[test]
fn sidecar_stage_skips_with_the_flag_and_otherwise_requires_the_manifest() {
    let root = temp("sidecar-skip");
    stage_sidecar_with(&root, true)
        .expect("the development skip must succeed without a manifest or network");
    let error = format!("{:#}", stage_sidecar_with(&root, false).unwrap_err());
    assert!(
        error.contains(SKIP_ENV),
        "error must point at {SKIP_ENV}: {error}"
    );
    assert!(
        error.contains("cannot stage the pinned Xerj sidecar"),
        "{error}"
    );
    fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// 离线暂存：预置缓存 + 系统 tar 造的假归档
// ---------------------------------------------------------------------------

#[cfg(unix)]
struct StagedFixture {
    root: PathBuf,
    cache: PathBuf,
    manifest: Manifest,
}

#[cfg(unix)]
impl Drop for StagedFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
impl StagedFixture {
    fn asset(&self) -> &Asset {
        &self.manifest.assets[FIXTURE_TARGET]
    }

    fn archive_path(&self) -> PathBuf {
        self.cache.join(cache_file_name(self.asset()).unwrap())
    }

    fn binary_path(&self) -> PathBuf {
        staged_binary_path(&self.root, self.asset())
    }

    fn license_path(&self) -> PathBuf {
        staged_license_path(&self.root)
    }

    fn metadata_path(&self) -> PathBuf {
        staged_metadata_path(&self.root)
    }

    fn rewrite_metadata(&self) {
        fs::write(
            self.metadata_path(),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&release_metadata(&self.manifest, self.asset()))
                    .unwrap()
            ),
        )
        .unwrap();
    }
}

#[cfg(unix)]
const FIXTURE_BINARY: &str = "#!/bin/sh\necho \"xerj v1.0.0-rc.100\"\n";
#[cfg(unix)]
const FIXTURE_LICENSE: &str = "Apache License 2.0 — fixture\n";

/// 用“临时文件 + chmod + rename”替换一个可执行测试文件。
///
/// 直接 `fs::write` 后立刻 exec 在 Linux 上偶发 `ETXTBSY`（Text file busy），
/// 并行测试时更明显；rename 换的是 inode，exec 不会看到正在写的文件。
#[cfg(unix)]
fn replace_executable(path: &Path, bytes: &[u8]) {
    let staging = path.with_extension(format!("new-{}", std::process::id()));
    fs::write(&staging, bytes).unwrap();
    let mut permissions = fs::metadata(&staging).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&staging, permissions).unwrap();
    fs::rename(&staging, path).unwrap();
}

/// 用系统 `tar` 造一个与真实资产同布局的假归档：根目录 `<release>-<target>/`，
/// 里面是可执行的 shell 脚本 `xerj` 与 `LICENSE`；再按真实 sha 生成清单并预置缓存。
#[cfg(unix)]
fn staged_fixture(tag: &str) -> StagedFixture {
    let root = temp(tag);
    let cache = root.join("cache");
    fs::create_dir_all(&cache).unwrap();

    let package_root = root.join("package");
    let root_name = format!("xerj-{FIXTURE_VERSION}-{FIXTURE_TARGET}");
    let package = package_root.join(&root_name);
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("xerj"), FIXTURE_BINARY).unwrap();
    let mut permissions = fs::metadata(package.join("xerj")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(package.join("xerj"), permissions).unwrap();
    fs::write(package.join("LICENSE"), FIXTURE_LICENSE).unwrap();

    let archive = cache.join(format!("xerj-{FIXTURE_VERSION}-{FIXTURE_TARGET}.tar.gz"));
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&package_root)
        .arg(&root_name)
        .status()
        .expect("system tar is required by the xtask tests");
    assert!(status.success(), "tar must create the fixture archive");

    let sha256 = sha256_file(&archive).unwrap();
    let mut value = fixture_manifest_value();
    value["assets"][FIXTURE_TARGET]["sha256"] = serde_json::Value::from(sha256);
    let manifest = manifest_from_value(&value);

    let options = Options {
        verify_only: false,
        offline: true,
        force: true,
    };
    stage_with(&root, &manifest, FIXTURE_TARGET, &cache, &options)
        .expect("offline staging from a preseeded cache must succeed");
    StagedFixture {
        root,
        cache,
        manifest,
    }
}

#[cfg(unix)]
#[test]
fn offline_staging_from_a_preseeded_cache_writes_the_sidecar_deterministically() {
    let fixture = staged_fixture("offline-stage");
    let asset = fixture.asset();

    assert_eq!(
        fs::read_to_string(fixture.binary_path()).unwrap(),
        FIXTURE_BINARY
    );
    assert_ne!(
        fs::metadata(fixture.binary_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0,
        "the staged binary must be executable"
    );
    assert_eq!(
        fs::read_to_string(fixture.license_path()).unwrap(),
        FIXTURE_LICENSE
    );

    // 暂存清单：字段固定、值来自清单、没有时间戳。
    let text = fs::read_to_string(fixture.metadata_path()).unwrap();
    assert!(text.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let object = value.as_object().unwrap();
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "archive",
            "license",
            "licensed_file",
            "release",
            "schema",
            "sha256",
            "target",
            "url"
        ]
    );
    assert_eq!(value["schema"], 1);
    assert_eq!(value["release"], FIXTURE_RELEASE);
    assert_eq!(value["target"], FIXTURE_TARGET);
    assert_eq!(value["sha256"], asset.sha256);
    assert_eq!(value["url"], asset.url);
    assert_eq!(value["archive"], "tar.gz");
    assert_eq!(value["license"], "Apache-2.0");
    assert_eq!(value["licensed_file"], STAGED_LICENSE);

    // 解压目录用完即清，缓存里只剩归档本身。
    let cache_entries = fs::read_dir(&fixture.cache)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        cache_entries,
        vec![format!("xerj-{FIXTURE_VERSION}-{FIXTURE_TARGET}.tar.gz")]
    );

    // 可复现：同一个 release 再暂存一次，清单逐字节相同。
    let first = fs::read(fixture.metadata_path()).unwrap();
    let options = Options {
        verify_only: false,
        offline: true,
        force: true,
    };
    stage_with(
        &fixture.root,
        &fixture.manifest,
        FIXTURE_TARGET,
        &fixture.cache,
        &options,
    )
    .expect("restaging the same pinned release must succeed from the cache");
    assert_eq!(fs::read(fixture.metadata_path()).unwrap(), first);
}

#[cfg(unix)]
#[test]
fn already_current_staged_files_skip_the_download_even_without_a_cache() {
    let fixture = staged_fixture("idempotent");
    // 删掉缓存归档：幂等跳过若想联网/解压，--offline 会立刻失败。
    fs::remove_file(fixture.archive_path()).unwrap();
    let options = Options {
        verify_only: false,
        offline: true,
        force: false,
    };
    stage_with(
        &fixture.root,
        &fixture.manifest,
        FIXTURE_TARGET,
        &fixture.cache,
        &options,
    )
    .expect("already-current staged files must skip the download");

    let verify = Options {
        verify_only: true,
        offline: true,
        force: false,
    };
    stage_with(
        &fixture.root,
        &fixture.manifest,
        FIXTURE_TARGET,
        &fixture.cache,
        &verify,
    )
    .expect("--verify-only must accept consistent staged files");
}

#[cfg(unix)]
#[test]
fn offline_staging_without_a_cached_archive_fails_before_networking() {
    let root = temp("offline-missing");
    let cache = root.join("cache");
    let manifest = manifest_from_value(&fixture_manifest_value());
    let options = Options {
        verify_only: false,
        offline: true,
        force: true,
    };
    let error = format!(
        "{:#}",
        stage_with(&root, &manifest, FIXTURE_TARGET, &cache, &options).unwrap_err()
    );
    assert!(error.contains("--offline"), "{error}");
    assert!(error.contains("not cached"), "{error}");
    assert!(
        error.contains(&format!("xerj-{FIXTURE_VERSION}-{FIXTURE_TARGET}.tar.gz")),
        "error must name the expected cache file: {error}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn verify_only_rejects_missing_or_inconsistent_staged_files() {
    let fixture = staged_fixture("verify-only-errors");
    let options = Options {
        verify_only: true,
        offline: true,
        force: false,
    };
    let verify = |fixture: &StagedFixture| {
        format!(
            "{:#}",
            stage_with(
                &fixture.root,
                &fixture.manifest,
                FIXTURE_TARGET,
                &fixture.cache,
                &options,
            )
            .expect_err("inconsistent staged files must be rejected")
        )
    };

    // 1. 删掉 binary。
    let saved_binary = fs::read(fixture.binary_path()).unwrap();
    fs::remove_file(fixture.binary_path()).unwrap();
    let error = verify(&fixture);
    assert!(
        error.contains("binary") && error.contains("missing"),
        "{error}"
    );
    replace_executable(&fixture.binary_path(), &saved_binary);

    // 2. 清单里的 sha256 与固定清单不一致。
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.metadata_path()).unwrap()).unwrap();
    value["sha256"] = serde_json::Value::from("b".repeat(64));
    fs::write(fixture.metadata_path(), format!("{value}\n")).unwrap();
    let error = verify(&fixture);
    assert!(error.contains("sha256"), "{error}");
    assert!(error.contains(&"b".repeat(64)), "{error}");
    fixture.rewrite_metadata();

    // 3. 清单文件缺失。
    fs::remove_file(fixture.metadata_path()).unwrap();
    let error = verify(&fixture);
    assert!(error.contains("not staged"), "{error}");

    // 4. 版本不符：binary 换成打印别的版本的脚本。
    replace_executable(&fixture.binary_path(), b"#!/bin/sh\necho \"xerj v0.0.1\"\n");
    fixture.rewrite_metadata();
    let error = verify(&fixture);
    assert!(error.contains(FIXTURE_RELEASE), "{error}");
    assert!(error.contains("does not report"), "{error}");
}
