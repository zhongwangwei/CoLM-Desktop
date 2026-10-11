//! Signal handlers only flip an atomic; terminal cleanup runs on the normal thread.
#![allow(unsafe_code)]
use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
pub(crate) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

#[cfg(unix)]
unsafe extern "C" {
    fn signal(number: i32, handler: usize) -> usize;
}
#[cfg(unix)]
extern "C" fn interrupt(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
}
#[cfg(windows)]
extern "system" fn console_interrupt(event: u32) -> i32 {
    if event <= 1 {
        INTERRUPTED.store(true, Ordering::SeqCst);
        1
    } else {
        0
    }
}

pub(crate) struct Guard {
    #[cfg(unix)]
    previous: [usize; 2],
}
impl Guard {
    pub(crate) fn install() -> anyhow::Result<Self> {
        INTERRUPTED.store(false, Ordering::SeqCst);
        #[cfg(unix)]
        {
            // POSIX signal numbers and function-pointer ABI are shared by supported Unix targets.
            let int = unsafe { signal(2, interrupt as *const () as usize) };
            anyhow::ensure!(
                int != usize::MAX,
                "cannot install terminal interrupt handler"
            );
            let term = unsafe { signal(15, interrupt as *const () as usize) };
            if term == usize::MAX {
                unsafe {
                    signal(2, int);
                }
                anyhow::bail!("cannot install terminal termination handler");
            }
            Ok(Self {
                previous: [int, term],
            })
        }
        #[cfg(windows)]
        {
            anyhow::ensure!(
                unsafe { SetConsoleCtrlHandler(Some(console_interrupt), 1) } != 0,
                "cannot install terminal console handler"
            );
            Ok(Self {})
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self {})
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            signal(2, self.previous[0]);
            signal(15, self.previous[1]);
        }
        #[cfg(windows)]
        unsafe {
            SetConsoleCtrlHandler(Some(console_interrupt), 0);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn signal_cleanup_probe() {
        if std::env::var_os("COLM_SIGNAL_TEST_CHILD").is_some() {
            let _guard = Guard::install().unwrap();
            assert!(std::process::Command::new("kill")
                .args(["-TERM", &std::process::id().to_string()])
                .status()
                .unwrap()
                .success());
            for _ in 0..50 {
                if interrupted() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("SIGTERM did not request graceful cleanup");
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "terminal_signals::tests::signal_cleanup_probe"])
            .env("COLM_SIGNAL_TEST_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
    }
}
