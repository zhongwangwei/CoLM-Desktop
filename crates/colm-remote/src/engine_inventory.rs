//! Read-only inventory and conservatively locked cleanup of remote engine snapshots.
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

/// Explicit finish only: an interrupted SSH operation must leave cleanup blocked.
pub struct LifecycleLock {
    ssh: Ssh,
    root: String,
    token: String,
    active: bool,
}
impl LifecycleLock {
    pub fn acquire(ssh: &Ssh, root: &str) -> Result<Self> {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = format!(
            "{}-{:?}-{}",
            std::process::id(),
            std::time::SystemTime::now(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let token = format!("{:x}", Sha256::digest(nonce.as_bytes()));
        ssh.run_ok(&lock_script(root, &token, false)?)?;
        Ok(Self {
            ssh: ssh.clone(),
            root: root.to_owned(),
            token,
            active: true,
        })
    }
    fn release(&mut self) -> Result<()> {
        self.ssh
            .run_ok(&lock_script(&self.root, &self.token, true)?)?;
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
            .run_ok(&lock_script(&self.root, &self.token, true)?)?;
        self.active = false;
        Ok(())
    }
    pub fn finish_submitted(self, job: &str) -> Option<String> {
        self.finish().err().map(|error| {
            let warning =
                format!("Job {job} was submitted; engine cleanup remains locked: {error:#}");
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
            eprintln!("Engine lifecycle lock retained on {}: {}/.engine-lifecycle-lock (owner {}). The SSH operation may still be running; verify it has ended and inspect the owner before removing this lock. No automatic expiry is safe.", self.ssh.host, self.root, self.token);
        }
    }
}

fn lock_script(root: &str, token: &str, release: bool) -> Result<String> {
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
    let operation = if release {
        "test ! -L \"$L/owner\" && test -f \"$L/owner\"\n[ \"$(cat \"$L/owner\")\" = \"$TOKEN\" ] || { echo 'engine lock ownership changed' >&2; exit 1; }\nrm -- \"$L/owner\"\nrmdir -- \"$L\""
    } else {
        "mkdir -p -- \"$R\"\nmkdir -- \"$L\" || { echo \"engine lifecycle is busy or needs recovery: $L\" >&2; exit 1; }\nprintf '%s' \"$TOKEN\" > \"$L/owner\""
    };
    Ok(format!("set -e\nR={}\nTOKEN={}\nL=\"$R/.engine-lifecycle-lock\"\nP=\"$L\"\nwhile [ \"$P\" != / ]; do test ! -L \"$P\" || {{ echo 'symbolic links are not allowed in engine resource paths' >&2; exit 1; }}; P=$(dirname -- \"$P\"); done\n{operation}\n", quote(root), quote(token)))
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
    #[test]
    fn posix_lock_needs_no_python_and_checks_ownership() {
        let root = fixture();
        let call = |token, release| {
            Command::new("sh")
                .args([
                    "-c",
                    &lock_script(root.to_str().unwrap(), token, release).unwrap(),
                ])
                .output()
                .unwrap()
        };
        assert!(call("first", false).status.success());
        assert!(!call("second", false).status.success());
        assert!(!call("second", true).status.success());
        assert!(call("first", true).status.success());
        assert!(call("second", false).status.success());
        assert!(call("second", true).status.success());
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
        assert!(!root.join(".engine-lifecycle-lock").exists());
        let mut guard = LifecycleLock::acquire(&ssh, root.to_str().unwrap()).unwrap();
        assert!(guard
            .run::<()>(|| ssh.run_ok("exit 255").map(|_| ()))
            .is_err());
        assert!(root.join(".engine-lifecycle-lock").is_dir());
        guard.finish().unwrap();
        let guard = LifecycleLock::acquire(&ssh, root.to_str().unwrap()).unwrap();
        std::fs::write(root.join(".engine-lifecycle-lock/owner"), "changed").unwrap();
        assert!(guard
            .finish_submitted("submitted-job-1")
            .unwrap()
            .contains("submitted-job-1"));
        assert!(root.join(".engine-lifecycle-lock").is_dir());
    }
}
