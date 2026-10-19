//! Servers that cannot reach the internet (星逸/天河 compute centres, many campus clusters).
//!
//! The desktop downloads what a source build needs, checks every file against a pinned or
//! lock-file checksum, pushes it over the existing ssh pipe, and the server builds with no network:
//!
//! - **Rust** — the three official `1.97.1` component archives (`rustc`, `cargo`, `rust-std`), their
//!   checksums pinned below; installed with their own `install.sh` under
//!   `~/.local/share/colm/toolchain/rust`.
//! - **CMake** — the same pinned archive the online installer uses, under `…/toolchain/cmake`.
//! - **crates** — every registry package in the snapshot's `Cargo.lock`, downloaded from
//!   `static.crates.io` and verified against the lock file's own checksum, repacked into a cargo
//!   "directory source" under `<root>/offline/vendor-<key>`. This needs no cargo on the desktop.
//!   `engine::ensure_script` finds it by hashing the snapshot's `Cargo.lock` on the server and then
//!   builds with `--offline` and a `--config` source replacement (a config file is not needed;
//!   environment variables cannot define `[source.*]`).
//!
//! Nothing is installed outside the user's home and the work root, and nothing is fetched from the
//! server's side. Downloads are cached on the desktop, so a second server costs only the upload.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use sha2::{Digest, Sha256};

use crate::ssh::{quote, Ssh};

pub const RUST_VERSION: &str = "1.97.1";
pub const CMAKE_VERSION: &str = "4.4.4";

/// One file to fetch: where from, what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub file: String,
    pub url: String,
    pub sha256: &'static str,
}

fn triple(arch: &str) -> Result<&'static str> {
    match arch {
        "x86_64" => Ok("x86_64-unknown-linux-gnu"),
        "aarch64" => Ok("aarch64-unknown-linux-gnu"),
        other => bail!("offline installation supports Linux x86_64 and aarch64, not {other}"),
    }
}

/// The three Rust component archives for a server architecture (`uname -m`).
pub fn rust_downloads(arch: &str) -> Result<Vec<Download>> {
    let triple = triple(arch)?;
    let sums: [&'static str; 3] = if arch == "x86_64" {
        [
            "1c441e430c1cca49dff54a8d59c41038bf6f79f7b8756596cb2f36511a015eba",
            "90cbeb8adfe8ca0fcbe01a18bd9b99d3e5e98fd29c003032828ebf3ffba0e4ed",
            "45b927ecf776b9645ca2ed5b287fc6814824a073c30c7c5d5c0ad4425295ecee",
        ]
    } else {
        [
            "8d9f3095a3108e38232f123645e2a0368bca67c975f8b91088004f8ffd9a6d39",
            "cafde243518b016f825d13a5f2ca4ffd7d80f5966dfbec3539cd092c7c4f9871",
            "ceffa428fe573507418858d59add342b351730725f36f7679b251e8b68103dec",
        ]
    };
    Ok(["rustc", "cargo", "rust-std"]
        .iter()
        .zip(sums)
        .map(|(component, sha256)| {
            let file = format!("{component}-{RUST_VERSION}-{triple}.tar.gz");
            Download {
                url: format!("https://static.rust-lang.org/dist/{file}"),
                file,
                sha256,
            }
        })
        .collect())
}

/// The CMake archive (also pinned in [`crate::toolchain::install_script`]).
pub fn cmake_download(arch: &str) -> Result<Download> {
    triple(arch)?;
    let sha256 = if arch == "x86_64" {
        "e5bb807f7728cb60cd8b27ebc97a2edb469b68655f21e844a600c3575b76f5bb"
    } else {
        "a1b6cc63636a0e55c63257cf3315a8a5f129e42fade25db1afea4ff8ab06f25e"
    };
    let file = format!("cmake-{CMAKE_VERSION}-linux-{arch}.tar.gz");
    Ok(Download {
        url: format!("https://github.com/Kitware/CMake/releases/download/v{CMAKE_VERSION}/{file}"),
        file,
        sha256,
    })
}

/// A registry package named by `Cargo.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crate {
    pub name: String,
    pub version: String,
    pub checksum: String,
}

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The crates.io packages a lock file pins. Workspace members have no `source` and are skipped;
/// anything else (git, another registry) cannot be fetched this way and is refused.
pub fn crates_in_lock(lock: &str) -> Result<Vec<Crate>> {
    let mut crates = Vec::new();
    let mut block: BTreeMap<&str, &str> = BTreeMap::new();
    let mut finish = |block: &mut BTreeMap<&str, &str>| -> Result<()> {
        let name = block.get("name").copied();
        let version = block.get("version").copied();
        let source = block.get("source").copied();
        let checksum = block.get("checksum").copied();
        block.clear();
        let (Some(name), Some(version)) = (name, version) else {
            return Ok(());
        };
        match source {
            None => Ok(()),
            Some(CRATES_IO) => {
                let checksum = checksum
                    .with_context(|| format!("Cargo.lock has no checksum for {name} {version}"))?;
                ensure!(
                    checksum.len() == 64 && checksum.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Cargo.lock has a malformed checksum for {name} {version}"
                );
                ensure!(
                    [name, version].iter().all(|part| !part.is_empty()
                        && part
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))),
                    "Cargo.lock has an unusual package name {name} {version}"
                );
                crates.push(Crate {
                    name: name.to_owned(),
                    version: version.to_owned(),
                    checksum: checksum.to_ascii_lowercase(),
                });
                Ok(())
            }
            Some(other) => bail!(
                "Cargo.lock uses a package source that cannot be vendored for an offline server: {name} {version} from {other}"
            ),
        }
    };
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            finish(&mut block)?;
        } else if let Some((key, value)) = line.split_once(" = ") {
            if let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                if matches!(key, "name" | "version" | "source" | "checksum") {
                    block.insert(key, value);
                }
            }
        }
    }
    finish(&mut block)?;
    Ok(crates)
}

/// A short stable name for one lock file; the server keeps one vendor directory per key.
pub fn lock_key(lock: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(lock.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_owned()
}

/// Where downloads are kept on the desktop (`COLM_OFFLINE_CACHE` overrides it).
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COLM_OFFLINE_CACHE") {
        return PathBuf::from(dir);
    }
    crate::engine::cache_dir()
        .parent()
        .map(|parent| parent.join("offline"))
        .unwrap_or_else(|| PathBuf::from("offline"))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Download `url` to `dest` unless a file with the right checksum is already there. The file only
/// appears under its final name once it has been verified.
pub fn fetch(url: &str, sha256: &str, dest: &Path) -> Result<()> {
    if dest.is_file() && sha256_file(dest).is_ok_and(|actual| actual == sha256) {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_extension(format!("part{}", std::process::id()));
    let result = (|| -> Result<()> {
        // Proxy settings come from the usual environment variables (HTTPS_PROXY, ALL_PROXY).
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(1800)))
            .timeout_connect(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut response = agent
            .get(url)
            .call()
            .with_context(|| format!("cannot download {url}"))?;
        let mut reader = response.body_mut().as_reader();
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let read = reader
                .read(&mut buffer)
                .with_context(|| format!("download of {url} was interrupted"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])?;
        }
        file.flush()?;
        let actual = format!("{:x}", hasher.finalize());
        ensure!(
            actual == sha256,
            "checksum mismatch for {url}: expected {sha256}, got {actual}; refusing to use it"
        );
        std::fs::rename(&part, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn fetch_many(
    items: Vec<(String, String, PathBuf)>,
    say: &(dyn Fn(&str) + Sync),
    label: &str,
) -> Result<()> {
    let total = items.len();
    let queue = std::sync::Mutex::new(items.into_iter());
    let done = std::sync::atomic::AtomicUsize::new(0);
    let failure = std::sync::Mutex::new(None::<anyhow::Error>);
    std::thread::scope(|scope| {
        for _ in 0..8.min(total.max(1)) {
            scope.spawn(|| loop {
                if failure.lock().unwrap().is_some() {
                    return;
                }
                let Some((url, sha, dest)) = queue.lock().unwrap().next() else {
                    return;
                };
                match fetch(&url, &sha, &dest) {
                    Ok(()) => {
                        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if n % 25 == 0 || n == total {
                            say(&format!("{label}: {n}/{total}"));
                        }
                    }
                    Err(error) => {
                        failure.lock().unwrap().get_or_insert(error);
                        return;
                    }
                }
            });
        }
    });
    match failure.into_inner().unwrap() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Unpack one `.crate` into the vendor tar being written, plus the checksum file cargo expects.
fn append_crate(
    builder: &mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>,
    package: &Crate,
    crate_file: &Path,
) -> Result<()> {
    let prefix = format!("{}-{}", package.name, package.version);
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(
        std::fs::File::open(crate_file)
            .with_context(|| format!("cannot read {}", crate_file.display()))?,
    ));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut parts = path.components();
        ensure!(
            parts.next().is_some_and(
                |first| matches!(first, std::path::Component::Normal(name) if name == prefix.as_str())
            ) && parts.all(|part| matches!(part, std::path::Component::Normal(_))),
            "{prefix}: unexpected path {} in the package",
            path.display()
        );
        let kind = entry.header().entry_type();
        // AppleDouble files only exist when something re-packed the crate on macOS.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("._"))
        {
            continue;
        }
        if kind.is_dir() {
            continue;
        }
        ensure!(
            kind.is_file(),
            "{prefix}: {} is neither a file nor a directory",
            path.display()
        );
        let mut header = entry.header().clone();
        builder.append_data(&mut header, &path, &mut entry)?;
    }
    let checksum = format!("{{\"files\":{{}},\"package\":\"{}\"}}", package.checksum);
    let mut header = tar::Header::new_gnu();
    header.set_size(checksum.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    builder.append_data(
        &mut header,
        format!("{prefix}/.cargo-checksum.json"),
        checksum.as_bytes(),
    )?;
    Ok(())
}

/// Build (or reuse) the vendor tarball of a lock file in the desktop cache and return its path.
pub fn vendor_tarball(lock: &str, say: &(dyn Fn(&str) + Sync)) -> Result<PathBuf> {
    vendor_tarball_in(&cache_dir(), lock, say)
}

fn vendor_tarball_in(cache: &Path, lock: &str, say: &(dyn Fn(&str) + Sync)) -> Result<PathBuf> {
    let key = lock_key(lock);
    let tarball = cache.join(format!("vendor-{key}.tar.gz"));
    if tarball.is_file() {
        return Ok(tarball);
    }
    let crates = crates_in_lock(lock)?;
    ensure!(!crates.is_empty(), "Cargo.lock names no registry packages");
    let crate_dir = cache.join("crates");
    say(&format!("downloading {} Rust packages", crates.len()));
    fetch_many(
        crates
            .iter()
            .map(|package| {
                (
                    format!(
                        "https://static.crates.io/crates/{0}/{0}-{1}.crate",
                        package.name, package.version
                    ),
                    package.checksum.clone(),
                    crate_dir.join(format!("{}-{}.crate", package.name, package.version)),
                )
            })
            .collect(),
        say,
        "Rust packages",
    )?;
    let part = tarball.with_extension(format!("part{}", std::process::id()));
    let result = (|| -> Result<()> {
        let encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&part)?,
            flate2::Compression::fast(),
        );
        let mut builder = tar::Builder::new(encoder);
        for package in &crates {
            append_crate(
                &mut builder,
                package,
                &crate_dir.join(format!("{}-{}.crate", package.name, package.version)),
            )?;
        }
        builder.into_inner()?.finish()?;
        std::fs::rename(&part, &tarball)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result?;
    Ok(tarball)
}

/// Can the server fetch from crates.io and rust-lang.org itself? A server without `curl` counts as
/// offline: the online installer needs it too.
pub fn online_script() -> &'static str {
    "command -v curl >/dev/null 2>&1 && curl -fsS -o /dev/null --max-time 8 https://index.crates.io/config.json >/dev/null 2>&1 && curl -fsS -o /dev/null --max-time 8 https://static.rust-lang.org/dist/channel-rust-stable.toml.sha256 >/dev/null 2>&1 && echo yes || echo no"
}

pub fn is_online(ssh: &Ssh) -> Result<bool> {
    Ok(ssh.run_ok(online_script())?.trim() == "yes")
}

/// True when a failed build looks like "the server could not reach the internet" rather than a
/// compile error — the cue to switch to the offline route.
pub fn looks_like_network_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "failed to download",
        "unable to update registry",
        "failed to get `",
        "could not resolve host",
        "couldn't resolve host",
        "spurious network error",
        "network failure seems to have happened",
        "failed to connect",
        "connection timed out",
        "connection refused",
        "toolchain download failed",
        "failed to fetch",
        "ssl connect error",
        "timeout was reached",
        "operation timed out",
        "check server https access",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn toolchain_base(home: &str) -> String {
    format!("{}/.local/share/colm/toolchain", home.trim_end_matches('/'))
}

/// Where the vendor directory of a lock file lives under the work root.
pub fn vendor_dir(root: &str, key: &str) -> String {
    format!("{}/offline/vendor-{key}", root.trim_end_matches('/'))
}

/// What the server already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Present {
    pub rust: bool,
    pub cmake: bool,
    pub vendor: bool,
}

/// The server as [`state_script`] saw it. `online` is only asked while the vendor directory is
/// missing: once it is there the build uses it whatever the network does.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    home: String,
    arch: String,
    present: Present,
    online: bool,
}

/// With `engine` (an engine directory) the script stops at `built=1` when that engine is already
/// compiled: nothing needs staging then, however offline the server is.
fn state_script(root: &str, key: &str, engine: Option<&str>) -> String {
    let built = engine.map_or(String::new(), |dir| {
        format!(
            "[ -x {} ] && {{ echo built=1; exit 0; }}\n",
            quote(&format!("{dir}/bin/colm-rs"))
        )
    });
    format!(
        "set +e\n{built}{prelude}{build_path}{requirements}echo \"home=$HOME\"\necho \"arch=$(uname -m)\"\n\
         echo \"rust=$(cargo --version >/dev/null 2>&1 && rustc --version >/dev/null 2>&1 && echo 1)\"\n\
         echo \"cmake=$(colm_cmake_ok && echo 1)\"\n\
         echo \"vendor=$([ -f {vendor}/.complete ] && echo 1)\"\n\
         [ -f {vendor}/.complete ] || echo \"online=$({online})\"\n",
        online = online_script(),
        prelude = crate::sched::PATH_PRELUDE,
        build_path = crate::probe::BUILD_PATH_PRELUDE,
        requirements = crate::toolchain::requirements_script(),
        vendor = quote(&vendor_dir(root, key)),
    )
}

fn parse_state(text: &str) -> Result<State> {
    let values: BTreeMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect();
    let home = values
        .get("home")
        .filter(|home| home.starts_with('/'))
        .context("the server did not report its home directory")?
        .to_string();
    let arch = values.get("arch").copied().unwrap_or_default().to_owned();
    let flag = |key: &str| values.get(key).is_some_and(|v| *v == "1");
    Ok(State {
        home,
        arch,
        present: Present {
            rust: flag("rust"),
            cmake: flag("cmake"),
            vendor: flag("vendor"),
        },
        online: values.get("online").is_some_and(|v| *v == "yes"),
    })
}

/// Remote step that installs the uploaded archives. Runs under a lock so two builds cannot race.
fn install_script(base: &str, need_rust: bool, need_cmake: bool) -> String {
    format!(
        "set -e\n{prelude}{build_path}{requirements}\
         base={base}\nstage=\"$base/.offline-stage\"\nexec 8>\"$base/install.lock\"\nflock 8\n\
         if [ {need_rust} = 1 ] && ! {{ cargo --version >/dev/null 2>&1 && rustc --version >/dev/null 2>&1; }}; then\n\
           for d in \"$stage\"/rustc-* \"$stage\"/cargo-* \"$stage\"/rust-std-*; do\n\
             sh \"$d/install.sh\" --prefix=\"$base/rust\" --disable-ldconfig >/dev/null\n\
           done\n\
         fi\n\
         if [ {need_cmake} = 1 ] && ! colm_cmake_ok; then\n\
           if [ -e \"$base/cmake\" ] || [ -L \"$base/cmake\" ]; then\n\
             echo \"Existing CoLM CMake installation is unusable: $base/cmake. Repair it manually; it will not be overwritten.\" >&2\n\
             exit 1\n\
           fi\n\
           mv -T -n -- \"$stage\"/cmake-* \"$base/cmake\"\n\
         fi\n\
         rm -rf -- \"$stage\"\n\
         cargo --version >/dev/null 2>&1 && rustc --version >/dev/null 2>&1 || {{ echo 'The uploaded Rust toolchain does not run on this server (system libraries?)' >&2; exit 1; }}\n\
         colm_cmake_ok || {{ echo 'The uploaded CMake does not run on this server' >&2; exit 1; }}\n",
        prelude = crate::sched::PATH_PRELUDE,
        build_path = crate::probe::BUILD_PATH_PRELUDE,
        requirements = crate::toolchain::requirements_script(),
        base = quote(base),
        need_rust = u8::from(need_rust),
        need_cmake = u8::from(need_cmake),
    )
}

fn publish_vendor_script(stage: &str, dest: &str) -> String {
    format!(
        "set -e\nmkdir -p {parent}\nexec 9>{lock}\nflock 9\n\
         if [ -f {dest}/.complete ]; then rm -rf -- {stage}; else\n\
           touch {stage}/.complete\n  rm -rf -- {dest}\n  mv -- {stage} {dest}\nfi\n",
        parent = quote(dest.rsplit_once('/').map_or(".", |(parent, _)| parent)),
        lock = quote(&format!("{dest}.lock")),
        stage = quote(stage),
        dest = quote(dest),
    )
}

/// Run after a source snapshot reaches the server and before anything builds it. A server that
/// can fetch from crates.io and rust-lang.org is left alone (the online route installs what it
/// lacks); otherwise it gets Rust, CMake and the lock file's crates from the desktop. Returns
/// whether anything was staged. Once the engine is built or the vendor directory is there this
/// costs one ssh round trip and no network check.
pub fn prepare(
    ssh: &Ssh,
    root: &str,
    engine_id: &str,
    lock: &str,
    say: &(dyn Fn(&str) + Sync),
) -> Result<bool> {
    let key = lock_key(lock);
    let engine = crate::engine::engine_dir(root, engine_id);
    let text = ssh.run_ok(&state_script(root, &key, Some(&engine)))?;
    if text.lines().any(|line| line.trim() == "built=1") {
        return Ok(false);
    }
    let state = parse_state(&text)?;
    let present = state.present;
    if (present.vendor && present.rust && present.cmake) || (!present.vendor && state.online) {
        return Ok(false);
    }
    say("the server cannot download build dependencies; sending them from this computer");
    stage_state(ssh, root, lock, &key, &state, say)?;
    Ok(true)
}

/// Make the server able to build offline whatever its network looks like: install Rust and CMake
/// if it lacks them and put the lock file's crates under the work root. For a server whose
/// connectivity check passed but whose build still failed to download (a proxy slow enough to
/// time out, 天河 2026-10-19). Every piece the server already has is skipped.
pub fn stage(ssh: &Ssh, root: &str, lock: &str, say: &(dyn Fn(&str) + Sync)) -> Result<()> {
    let key = lock_key(lock);
    let state = parse_state(&ssh.run_ok(&state_script(root, &key, None))?)?;
    stage_state(ssh, root, lock, &key, &state, say)
}

fn stage_state(
    ssh: &Ssh,
    root: &str,
    lock: &str,
    key: &str,
    state: &State,
    say: &(dyn Fn(&str) + Sync),
) -> Result<()> {
    let present = state.present;
    let base = toolchain_base(&state.home);
    if !present.rust || !present.cmake {
        let mut downloads = Vec::new();
        if !present.rust {
            downloads.extend(rust_downloads(&state.arch)?);
        }
        if !present.cmake {
            downloads.push(cmake_download(&state.arch)?);
        }
        let stage_dir = format!("{base}/.offline-stage");
        ssh.run_ok(&format!(
            "rm -rf -- {stage} && mkdir -p {stage}",
            stage = quote(&stage_dir)
        ))?;
        let cache = cache_dir();
        for download in &downloads {
            say(&format!("downloading {}", download.file));
            let local = cache.join(&download.file);
            fetch(&download.url, download.sha256, &local)?;
            say(&format!("uploading {}", download.file));
            ssh.upload_tarball(&local, &stage_dir)?;
        }
        say("installing Rust and CMake on the server");
        ssh.run_ok(&install_script(&base, !present.rust, !present.cmake))?;
    }
    if !present.vendor {
        let tarball = vendor_tarball(lock, say)?;
        say("uploading the Rust packages");
        let dest = vendor_dir(root, key);
        let stage_dir = format!("{dest}.upload.{}", std::process::id());
        ssh.run_ok(&format!(
            "rm -rf -- {stage} && mkdir -p {stage}",
            stage = quote(&stage_dir)
        ))?;
        let result = ssh.upload_tarball(&tarball, &stage_dir).and_then(|()| {
            ssh.run_ok(&publish_vendor_script(&stage_dir, &dest))
                .map(|_| ())
        });
        if result.is_err() {
            let _ = ssh.run_ok(&format!("rm -rf -- {}", quote(&stage_dir)));
        }
        result?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "offline_tests.rs"]
mod tests;
