//! 关窗与退出（⌘Q）的确认：先问前端有没有未保存的编辑、在跑的本机任务，用户点"仍然关闭"才真的停掉一切、
//! 关窗。原来 `CloseRequested` 只调 `stop_everything` 就放行：未保存的算例设置、版本草稿、服务器设置直接丢，
//! 正在跑的模拟也被无提示终止（GUI 审查 G-01）。
//!
//! 前端没响应（卡死、还没加载完）时不能把窗口关不掉：上一次请求 [`UNANSWERED`] 之后还没回音，再点关闭就直接放行。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

/// 前端这么久还没答复关窗请求，就当它答不了，再请求一次时直接关。
const UNANSWERED: Duration = Duration::from_secs(8);

#[derive(Default)]
pub struct CloseGate {
    /// 用户已经确认（或前端说没什么要保存的）：之后的关窗、退出一律放行。
    confirmed: AtomicBool,
    /// 上一次把关窗请求交给前端的时刻；前端答复（不论关不关）后清掉。
    asked: Mutex<Option<Instant>>,
}

impl CloseGate {
    /// 这次关窗/退出要不要先拦下来问前端。返回 `true` 表示已经发出询问、应当阻止这次关闭。
    pub fn intercept(&self, app: &tauri::AppHandle) -> bool {
        if self.confirmed.load(Ordering::SeqCst) {
            return false;
        }
        let mut asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
        if asked.is_some_and(|at| at.elapsed() > UNANSWERED) {
            // 前端没有在限期内答复：不能让窗口永远关不掉。
            self.confirmed.store(true, Ordering::SeqCst);
            return false;
        }
        if asked.is_none() {
            *asked = Some(Instant::now());
        }
        let _ = app.emit("app://close-requested", ());
        true
    }

    fn answered(&self) {
        *self.asked.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// 前端对关窗请求的答复：`close` 为真时停掉一切再关窗，为假时留着窗口（用户点了"取消"）。
#[tauri::command]
pub fn app_close(app: tauri::AppHandle, close: bool) -> Result<(), String> {
    let gate = app.state::<CloseGate>();
    gate.answered();
    if !close {
        return Ok(());
    }
    gate.confirmed.store(true, Ordering::SeqCst);
    crate::stop_everything(&app, "closing");
    app.exit(0);
    Ok(())
}

#[cfg(test)]
#[path = "close_gate_tests.rs"]
mod close_gate_tests;
