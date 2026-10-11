//! Read-only inventory and conservatively locked cleanup of remote engine snapshots.
//!
//! Two kinds of hold on a server root: every submission writes its own lease in `.engine-leases/` (many at
//! once; a submission never waits for another), and cleanup takes the exclusive `.engine-lifecycle-lock` and
//! refuses while any lease exists. An interrupted submission therefore only blocks cleanup, never later
//! submissions; `recover` (human-confirmed) removes records left behind for longer than an hour.
use crate::ssh::{quote, Ssh};
use anyhow::{ensure, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn script(root: &str, action: &str, snapshot: &str, token: &str) -> String {
    format!(
        "command -v python3 >/dev/null || {{ echo 'python3 is required for safe engine inventory and cleanup' >&2; exit 1; }}\npython3 - {} {} {} {} <<'COLM_ENGINE_INVENTORY_PY'\n{}\nCOLM_ENGINE_INVENTORY_PY\n",
        quote(root),
        quote(action),
        quote(snapshot),
        quote(token),
        include_str!("engine_inventory.py")
    )
}

pub fn list(ssh: &Ssh, root: &str) -> Result<Value> {
    Ok(serde_json::from_str(
        &ssh.run_ok(&script(root, "list", "", ""))?,
    )?)
}

/// Removes submission leases and a cleanup lock older than `older_than_s` (at least 10 minutes). Human-only.
pub fn recover(ssh: &Ssh, root: &str, older_than_s: u64) -> Result<Value> {
    ensure!(
        older_than_s >= 600,
        "only records older than 10 minutes can be recovered"
    );
    Ok(serde_json::from_str(&ssh.run_ok(&script(
        root,
        "recover",
        "",
        &older_than_s.to_string(),
    ))?)?)
}

/// Explicit finish only: an interrupted SSH operation must leave cleanup blocked.
/// `exclusive` is the cleanup lock; otherwise this is a submission lease.
pub struct LifecycleLock {
    ssh: Ssh,
    root: String,
    token: String,
    active: bool,
    exclusive: bool,
}
impl LifecycleLock {
    /// A submission lease: any number may be held at once; refused only while cleanup holds the lock.
    pub fn acquire(ssh: &Ssh, root: &str) -> Result<Self> {
        Self::take(ssh, root, false)
    }

    /// The exclusive cleanup lock: refused while any submission lease exists.
    pub fn acquire_exclusive(ssh: &Ssh, root: &str) -> Result<Self> {
        Self::take(ssh, root, true)
    }

    fn take(ssh: &Ssh, root: &str, exclusive: bool) -> Result<Self> {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = format!(
            "{}-{:?}-{}",
            std::process::id(),
            std::time::SystemTime::now(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let token = format!("{:x}", Sha256::digest(nonce.as_bytes()));
        ssh.run_ok(&lock_script(root, &token, false, exclusive)?)?;
        Ok(Self {
            ssh: ssh.clone(),
            root: root.to_owned(),
            token,
            active: true,
            exclusive,
        })
    }
    fn release(&mut self) -> Result<()> {
        self.ssh
            .run_ok(&lock_script(&self.root, &self.token, true, self.exclusive)?)?;
        self.active = false;
        Ok(())
    }
    pub fn run<T>(&mut self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        match operation() {
            Ok(value) => Ok(value),
            Err(error) => {
                if confirmed_end(&error) {
                    if let Err(unlock) = self.release() {
                        eprintln!("Engine lock could not be released after failure: {unlock:#}");
                    }
                }
                Err(error)
            }
        }
    }
    pub fn finish(mut self) -> Result<()> {
        self.ssh
            .run_ok(&lock_script(&self.root, &self.token, true, self.exclusive)?)?;
        self.active = false;
        Ok(())
    }
    pub fn finish_submitted(self, job: &str) -> Option<String> {
        self.finish().err().map(|error| {
            let warning =
                format!("Job {job} was submitted; its submission record remains and blocks engine cleanup until recovered: {error:#}");
            eprintln!("{warning}");
            warning
        })
    }
    pub fn clean(&mut self, snapshot: &str) -> Result<Value> {
        ensure!(
            snapshot.len() == 16
                && snapshot
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "invalid engine snapshot"
        );
        let ssh = self.ssh.clone();
        let command = script(&self.root, "clean", snapshot, &self.token);
        self.run(|| Ok(serde_json::from_str(&ssh.run_ok(&command)?)?))
    }
}

fn confirmed_end(error: &anyhow::Error) -> bool {
    if let Some(remote) = error.downcast_ref::<crate::ssh::CommandFailure>() {
        return remote.code.is_some_and(|code| code != 255);
    }
    !error.chain().any(|cause| cause.is::<std::io::Error>())
}

impl Drop for LifecycleLock {
    fn drop(&mut self) {
        if self.active {
            let held = if self.exclusive {
                ".engine-lifecycle-lock"
            } else {
                ".engine-leases"
            };
            eprintln!("Engine {} retained on {}: {}/{held} (token {}). It only blocks engine cleanup; once the operation has ended, remove it from Compute resources (interrupted submission records).", if self.exclusive { "cleanup lock" } else { "submission record" }, self.ssh.host, self.root, self.token);
        }
    }
}

fn lock_script(root: &str, token: &str, release: bool, exclusive: bool) -> Result<String> {
    ensure!(
        root.starts_with('/')
            && root != "/"
            && !root.contains('\0')
            && root
                .split('/')
                .skip(1)
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "unsafe engine resource root"
    );
    ensure!(
        !token.is_empty() && token.bytes().all(|c| c.is_ascii_alphanumeric()),
        "unsafe engine lock token"
    );
    let busy = "echo \"engine cleanup is in progress on this server; try again in a minute (if it was interrupted, recover it on the Compute resources page): $L\" >&2; exit 1";
    let operation = match (exclusive, release) {
        (true, false) => format!(
            "mkdir -p -- \"$D\"\nmkdir -- \"$L\" || {{ {busy}; }}\nprintf '%s' \"$TOKEN\" > \"$L/owner\"\nif [ -n \"$(ls -A -- \"$D\")\" ]; then\n  rm -- \"$L/owner\"; rmdir -- \"$L\"\n  echo \"submissions are in progress or were interrupted ($(ls -- \"$D\" | tr '\\n' ' ')); engine cleanup must wait (interrupted submission records can be recovered on the Compute resources page)\" >&2; exit 1\nfi"
        ),
        (true, true) => "test ! -L \"$L/owner\" && test -f \"$L/owner\"\n[ \"$(cat \"$L/owner\")\" = \"$TOKEN\" ] || { echo 'engine lock ownership changed' >&2; exit 1; }\nrm -- \"$L/owner\"\nrmdir -- \"$L\"".to_owned(),
        (false, false) => format!(
            "mkdir -p -- \"$D\"\n[ ! -e \"$L\" ] || {{ {busy}; }}\nprintf '%s' \"$TOKEN\" > \"$D/$TOKEN\"\n[ ! -e \"$L\" ] || {{ rm -f -- \"$D/$TOKEN\"; {busy}; }}"
        ),
        (false, true) => "test ! -L \"$D/$TOKEN\" && test -f \"$D/$TOKEN\" || { echo 'submission record is missing' >&2; exit 1; }\nrm -- \"$D/$TOKEN\"".to_owned(),
    };
    // Symlinks above the root are normal on servers ($HOME, /scratch); resolve them once and refuse symlinks
    // only for the lock and lease entries inside the root.
    Ok(format!(
        "set -e\nR={}\nTOKEN={}\nmkdir -p -- \"$R\"\nR=$(cd -- \"$R\" && pwd -P)\nL=\"$R/.engine-lifecycle-lock\"\nD=\"$R/.engine-leases\"\nfor P in \"$L\" \"$D\"; do test ! -L \"$P\" || {{ echo 'symbolic links are not allowed inside the engine resource root' >&2; exit 1; }}; done\n{operation}\n",
        quote(root),
        quote(token)
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    fn fixture() -> PathBuf {
        // SystemTime can repeat across parallel tests on macOS.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "colm-engine-inventory-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root)
            .unwrap_or_else(|error| panic!("unique fixture {}: {error}", root.display()));
        root
    }
    fn run(root: &Path, action: &str, snapshot: &str, token: &str) -> std::process::Output {
        Command::new("python3")
            .args([
                "-c",
                include_str!("engine_inventory.py"),
                root.to_str().unwrap(),
                action,
                snapshot,
                token,
            ])
            .output()
            .unwrap()
    }
    fn engine(root: &Path, id: &str) {
        std::fs::create_dir_all(root.join("engine").join(id).join("bin")).unwrap();
    }
    #[test]
    fn cleanup_is_serialized_and_job_references_are_retained() {
        let root = fixture();
        let id = "0123456789abcdef";
        engine(&root, id);
        assert!(run(&root, "acquire", "", "first").status.success());
        assert!(!run(&root, "acquire", "", "second").status.success());
        assert!(!run(&root, "release", "", "second").status.success());
        let job = root.join("jobs/j1");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(
            job.join("script.sh"),
            format!("# colm-engine-snapshot: {id}\n"),
        )
        .unwrap();
        std::fs::write(job.join("exit_code"), "0").unwrap();
        let result = run(&root, "clean", id, "first");
        assert!(result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("refused"));
        assert!(root.join("engine").join(id).exists());
        assert!(run(&root, "release", "", "first").status.success());
        assert!(run(&root, "acquire", "", "second").status.success());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn cleanup_only_removes_the_selected_unreferenced_snapshot() {
        let root = fixture();
        let id = "0123456789abcdef";
        engine(&root, id);
        engine(&root, "1111111111111111");
        assert!(run(&root, "acquire", "", "owner").status.success());
        for bad in ["../engine", "", "0123456789abcdeF"] {
            assert!(!run(&root, "clean", bad, "owner").status.success());
        }
        let output = run(&root, "clean", id, "owner");
        assert!(
            output.status.success(),
            "cleanup failed at {}: status={} stdout={} stderr={}",
            root.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.join("engine").join(id).exists());
        assert!(root.join("engine/1111111111111111").is_dir());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn unknown_jobs_and_studies_block_cleanup() {
        let root = fixture();
        engine(&root, "0123456789abcdef");
        std::fs::create_dir_all(root.join("jobs/old")).unwrap();
        std::fs::write(root.join("jobs/old/script.sh"), "old job\n").unwrap();
        std::fs::create_dir_all(root.join("s/study")).unwrap();
        let result: Value = serde_json::from_slice(&run(&root, "list", "", "").stdout).unwrap();
        assert_eq!(result["engines"][0]["can_clean"], false);
        assert_eq!(result["cleanup_blockers"].as_array().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }
    #[cfg(unix)]
    #[test]
    fn symbolic_links_never_expand_cleanup_scope() {
        let root = fixture();
        let outside = fixture();
        engine(&root, "0123456789abcdef");
        std::fs::write(outside.join("keep"), "keep").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("engine/0123456789abcdef/link")).unwrap();
        assert!(run(&root, "acquire", "", "owner").status.success());
        assert!(!run(&root, "clean", "0123456789abcdef", "owner")
            .status
            .success());
        assert!(outside.join("keep").is_file());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }
    fn sh(root: &Path, token: &str, release: bool, exclusive: bool) -> std::process::Output {
        Command::new("sh")
            .args([
                "-c",
                &lock_script(root.to_str().unwrap(), token, release, exclusive).unwrap(),
            ])
            .output()
            .unwrap()
    }
    #[test]
    fn posix_lock_needs_no_python_and_checks_ownership() {
        let root = fixture();
        assert!(sh(&root, "first", false, true).status.success());
        assert!(!sh(&root, "second", false, true).status.success());
        assert!(!sh(&root, "second", true, true).status.success());
        assert!(sh(&root, "first", true, true).status.success());
        assert!(sh(&root, "second", false, true).status.success());
        assert!(sh(&root, "second", true, true).status.success());
        let _ = std::fs::remove_dir_all(root);
    }
    /// Submissions never wait for each other; cleanup waits for all of them, and they wait for cleanup.
    #[test]
    fn submissions_share_leases_and_cleanup_excludes_them() {
        let root = fixture();
        assert!(sh(&root, "a1", false, false).status.success());
        assert!(
            sh(&root, "b2", false, false).status.success(),
            "two submissions at once"
        );
        let refused = sh(&root, "clean", false, true);
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("submissions are in progress"));
        assert!(
            !root.join(".engine-lifecycle-lock").exists(),
            "a refused cleanup leaves no lock"
        );
        assert!(sh(&root, "a1", true, false).status.success());
        assert!(
            !sh(&root, "a1", true, false).status.success(),
            "a lease is released once"
        );
        assert!(sh(&root, "b2", true, false).status.success());
        assert!(sh(&root, "clean", false, true).status.success());
        let blocked = sh(&root, "c3", false, false);
        assert!(
            !blocked.status.success(),
            "no submission while cleanup holds the lock"
        );
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("engine cleanup is in progress"));
        assert!(!root.join(".engine-leases/c3").exists());
        assert!(sh(&root, "clean", true, true).status.success());
        assert!(sh(&root, "c3", false, false).status.success());
        let _ = std::fs::remove_dir_all(root);
    }
    /// A symlinked directory above the root (a symlinked $HOME, /scratch, or macOS /tmp) is fine.
    #[test]
    fn symlinks_above_the_root_are_resolved_not_refused() {
        let real = fixture();
        let parent = fixture();
        let link = parent.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let root = link.join("colm");
        assert!(sh(&root, "lease1", false, false).status.success());
        assert!(real.join("colm/.engine-leases/lease1").is_file());
        assert!(sh(&root, "lease1", true, false).status.success());
        engine(&real.join("colm"), "0123456789abcdef");
        let listed: Value = serde_json::from_slice(&run(&root, "list", "", "").stdout).unwrap();
        assert_eq!(listed["engines"][0]["can_clean"], true, "{listed}");
        assert!(run(&root, "acquire", "", "owner").status.success());
        let cleaned = run(&root, "clean", "0123456789abcdef", "owner");
        assert!(
            String::from_utf8_lossy(&cleaned.stdout).contains("removed"),
            "{cleaned:?}"
        );
        assert!(run(&root, "release", "", "owner").status.success());
        // A symlink inside the root is still refused.
        std::os::unix::fs::symlink(&parent, real.join("colm/.engine-leases-moved")).unwrap();
        std::fs::remove_dir_all(real.join("colm/.engine-leases")).unwrap();
        std::os::unix::fs::symlink(&parent, real.join("colm/.engine-leases")).unwrap();
        assert!(!sh(&root, "lease2", false, false).status.success());
        let _ = std::fs::remove_dir_all(real);
        let _ = std::fs::remove_dir_all(parent);
    }
    /// Interrupted submissions block only cleanup; recovery removes old records and keeps recent ones.
    #[test]
    fn interrupted_submissions_block_cleanup_until_recovered() {
        let root = fixture();
        engine(&root, "0123456789abcdef");
        assert!(sh(&root, "stale", false, false).status.success());
        assert!(sh(&root, "fresh", false, false).status.success());
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::File::options()
            .write(true)
            .open(root.join(".engine-leases/stale"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let listed: Value = serde_json::from_slice(&run(&root, "list", "", "").stdout).unwrap();
        assert_eq!(listed["engines"][0]["can_clean"], false);
        assert_eq!(listed["leases"].as_array().unwrap().len(), 2);
        assert!(
            !run(&root, "recover", "", "60").status.success(),
            "younger than 10 minutes is refused"
        );
        let recovered: Value =
            serde_json::from_slice(&run(&root, "recover", "", "3600").stdout).unwrap();
        assert_eq!(recovered["removed_leases"], serde_json::json!(["stale"]));
        assert_eq!(recovered["kept_leases"], serde_json::json!(["fresh"]));
        assert!(root.join(".engine-leases/fresh").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn prebuilt_job_is_known_but_its_cache_is_preserved() {
        let root = fixture();
        engine(&root, "pre-0123456789abcdef");
        engine(&root, "1111111111111111");
        std::fs::create_dir_all(root.join("jobs/prebuilt")).unwrap();
        std::fs::write(
            root.join("jobs/prebuilt/script.sh"),
            "# colm-engine-snapshot: pre-0123456789abcdef\n",
        )
        .unwrap();
        let result: Value = serde_json::from_slice(&run(&root, "list", "", "").stdout).unwrap();
        assert!(result["cleanup_blockers"].as_array().unwrap().is_empty());
        let items = result["engines"].as_array().unwrap();
        assert_eq!(
            items
                .iter()
                .find(|e| e["id"] == "pre-0123456789abcdef")
                .unwrap()["can_clean"],
            false
        );
        assert_eq!(
            items
                .iter()
                .find(|e| e["id"] == "1111111111111111")
                .unwrap()["can_clean"],
            true
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn known_studies_only_protect_their_referenced_engine() {
        let root = fixture();
        engine(&root, "0123456789abcdef");
        engine(&root, "1111111111111111");
        std::fs::create_dir_all(root.join("jobs/study")).unwrap();
        std::fs::create_dir_all(root.join("s/current")).unwrap();
        std::fs::write(
            root.join("jobs/study/script.sh"),
            format!(
                "# colm-engine-snapshot: 0123456789abcdef\n# colm-study-root: {}\n",
                serde_json::to_string(&root.join("s/current")).unwrap()
            ),
        )
        .unwrap();
        let result: Value = serde_json::from_slice(&run(&root, "list", "", "").stdout).unwrap();
        assert!(result["cleanup_blockers"].as_array().unwrap().is_empty());
        assert_eq!(result["engines"][0]["can_clean"], false);
        assert_eq!(result["engines"][1]["can_clean"], true);
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn only_confirmed_failures_allow_automatic_unlock() {
        for code in [None, Some(255), Some(1), Some(2)] {
            let error = anyhow::Error::new(crate::ssh::CommandFailure {
                code,
                message: "redacted failure".into(),
            })
            .context("upload");
            assert_eq!(confirmed_end(&error), matches!(code, Some(1 | 2)));
        }
        assert!(!confirmed_end(
            &std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "connection lost").into()
        ));
        assert!(confirmed_end(&anyhow::anyhow!("invalid local input")));
    }
    #[cfg(unix)]
    #[test]
    fn lock_releases_confirmed_failure_but_preserves_transport_uncertainty() {
        use std::os::unix::fs::PermissionsExt;
        const CHILD: &str = "COLM_ENGINE_LOCK_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let root = fixture();
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let ssh = bin.join("ssh");
            std::fs::write(&ssh, "#!/bin/bash\nexec /bin/bash -c \"${!#}\"\n").unwrap();
            std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "engine_inventory::tests::lock_releases_confirmed_failure_but_preserves_transport_uncertainty", "--nocapture"])
                .env(CHILD, &root).env("PATH", format!("{}:/usr/bin:/bin", bin.display())).output().unwrap();
            let _ = std::fs::remove_dir_all(root);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let root = PathBuf::from(std::env::var_os(CHILD).unwrap()).join("remote");
        let ssh = Ssh::new("fixture").unwrap();
        let mut guard = LifecycleLock::acquire(&ssh, root.to_str().unwrap()).unwrap();
        let error = guard
            .run::<()>(|| ssh.run_ok("exit 12").map(|_| ()))
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::ssh::CommandFailure>()
                .unwrap()
                .code,
            Some(12)
        );
        let leases = || {
            std::fs::read_dir(root.join(".engine-leases"))
                .unwrap()
                .count()
        };
        assert_eq!(leases(), 0);
        let mut guard = LifecycleLock::acquire(&ssh, root.to_str().unwrap()).unwrap();
        assert!(guard
            .run::<()>(|| ssh.run_ok("exit 255").map(|_| ()))
            .is_err());
        assert_eq!(
            leases(),
            1,
            "transport uncertainty keeps the submission record"
        );
        // ...but it does not block the next submission.
        let other = LifecycleLock::acquire(&ssh, root.to_str().unwrap()).unwrap();
        other.finish().unwrap();
        guard.finish().unwrap();
        assert_eq!(leases(), 0);
        let guard = LifecycleLock::acquire(&ssh, root.to_str().unwrap()).unwrap();
        for entry in std::fs::read_dir(root.join(".engine-leases")).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        assert!(guard
            .finish_submitted("submitted-job-1")
            .unwrap()
            .contains("submitted-job-1"));
        assert!(!root.join(".engine-lifecycle-lock").exists());
    }
}
