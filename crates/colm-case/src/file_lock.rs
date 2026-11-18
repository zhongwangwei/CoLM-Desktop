//! Cross-process mutation lock for a stable sibling file. Never unlink the lock file.
use anyhow::{Context, Result};
use std::path::Path;

/// Blocks until exclusively locked; closing the returned file releases the lock, including on exit.
pub fn exclusive(path: &Path) -> Result<std::fs::File> {
    acquire(path, None)
}

/// Waits at most `timeout` for lock ownership; the file remains locked until closed.
pub fn exclusive_timeout(path: &Path, timeout: std::time::Duration) -> Result<std::fs::File> {
    acquire(path, Some(timeout))
}

#[allow(unsafe_code)]
fn acquire(path: &Path, timeout: Option<std::time::Duration>) -> Result<std::fs::File> {
    let began = std::time::Instant::now();
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
            if unsafe { flock(file.as_raw_fd(), if timeout.is_some() { 2 | 4 } else { 2 }) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            match (error.kind(), timeout) {
                (
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted,
                    Some(timeout),
                ) => wait_for_retry(began, timeout)?,
                (std::io::ErrorKind::Interrupted, None) => {}
                _ => return Err(error).context("cannot lock file"),
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
        loop {
            if unsafe {
                LockFileEx(
                    file.as_raw_handle(),
                    if timeout.is_some() { 2 | 1 } else { 2 },
                    0,
                    u32::MAX,
                    u32::MAX,
                    &mut overlapped,
                )
            } != 0
            {
                break;
            }
            let error = std::io::Error::last_os_error();
            match (error.raw_os_error(), timeout) {
                (Some(33), Some(timeout)) => wait_for_retry(began, timeout)?,
                _ => return Err(error).context("cannot lock file"),
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("file locking is unsupported on this platform");
    Ok(file)
}

fn wait_for_retry(began: std::time::Instant, timeout: std::time::Duration) -> Result<()> {
    let remaining = timeout.saturating_sub(began.elapsed());
    if remaining.is_zero() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out waiting for file lock",
        )
        .into());
    }
    std::thread::sleep(remaining.min(std::time::Duration::from_millis(25)));
    Ok(())
}

/// Only reports a process absent when the OS proves it; inaccessible owners stay protected.
#[allow(unsafe_code)]
pub fn process_is_alive(pid: u32) -> Result<bool> {
    anyhow::ensure!(pid > 0, "invalid process PID");
    #[cfg(unix)]
    {
        let pid = i32::try_from(pid).context("invalid process PID")?;
        extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        if unsafe { kill(pid, 0) } == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(3) => Ok(false), // ESRCH
            Some(1) => Ok(true),  // EPERM
            _ => Err(error).context("cannot check process owner"),
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        let handle = unsafe { OpenProcess(0x00100000, 0, pid) }; // SYNCHRONIZE
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(87) => Ok(false), // ERROR_INVALID_PARAMETER: PID does not exist
                Some(5) => Ok(true),   // ERROR_ACCESS_DENIED
                _ => Err(error).context("cannot check process owner"),
            };
        }
        let status = unsafe { WaitForSingleObject(handle, 0) };
        let error = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(handle);
        }
        match status {
            0 => Ok(false),
            258 => Ok(true),
            _ => Err(error).context("cannot check process owner"),
        }
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("process checks are unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_lock_times_out_and_can_be_reacquired() {
        let path = std::env::temp_dir().join(format!("colm-bounded-lock-{}", std::process::id()));
        let owner = exclusive(&path).unwrap();
        let began = std::time::Instant::now();
        let error = exclusive_timeout(&path, std::time::Duration::from_millis(50)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::TimedOut
        );
        assert!(began.elapsed() < std::time::Duration::from_secs(2));
        drop(owner);
        drop(exclusive_timeout(&path, std::time::Duration::ZERO).unwrap());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn process_probe_preserves_live_owners_and_rejects_invalid_pids() {
        assert!(process_is_alive(std::process::id()).unwrap());
        assert!(process_is_alive(0).is_err());
        #[cfg(unix)]
        assert!(process_is_alive(u32::MAX).is_err());
    }

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
