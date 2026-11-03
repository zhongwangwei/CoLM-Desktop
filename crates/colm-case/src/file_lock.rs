//! Cross-process mutation lock for a stable sibling file. Never unlink the lock file.
use anyhow::{Context, Result};
use std::path::Path;

/// Blocks until exclusively locked; closing the returned file releases the lock, including on exit.
#[allow(unsafe_code)]
pub fn exclusive(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).context("cannot open file lock")?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        extern "C" {
            fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
        }
        loop {
            if unsafe { flock(file.as_raw_fd(), 2) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error).context("cannot lock file");
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct Overlapped {
            internal: usize,
            internal_high: usize,
            offset: u32,
            offset_high: u32,
            event: *mut std::ffi::c_void,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn LockFileEx(
                handle: *mut std::ffi::c_void,
                flags: u32,
                reserved: u32,
                low: u32,
                high: u32,
                overlapped: *mut Overlapped,
            ) -> i32;
        }
        let mut overlapped = Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event: std::ptr::null_mut(),
        };
        if unsafe {
            LockFileEx(
                file.as_raw_handle(),
                2,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("cannot lock file");
        }
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("file locking is unsupported on this platform");
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_child() {
        let Some(path) = std::env::var_os("COLM_TEST_LOCK_PATH") else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        std::fs::write(path.with_extension("ready"), "ready").unwrap();
        let _lock = exclusive(&path).unwrap();
        std::fs::write(path.with_extension("done"), "done").unwrap();
    }

    #[test]
    fn independent_process_waits_until_owner_releases_the_file() {
        let path = std::env::temp_dir().join(format!(
            "colm-mutation-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let lock = exclusive(&path).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "file_lock::tests::lock_child", "--nocapture"])
            .env("COLM_TEST_LOCK_PATH", &path)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let began = std::time::Instant::now();
        while !path.with_extension("ready").exists()
            && began.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(path.with_extension("ready").exists());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!path.with_extension("done").exists());
        drop(lock);
        while child.try_wait().unwrap().is_none()
            && began.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if child.try_wait().unwrap().is_none() {
            let _ = child.kill();
        }
        assert!(child.wait().unwrap().success());
        assert!(path.with_extension("done").exists());
        for file in [
            &path,
            &path.with_extension("ready"),
            &path.with_extension("done"),
        ] {
            std::fs::remove_file(file).unwrap();
        }
    }
}
