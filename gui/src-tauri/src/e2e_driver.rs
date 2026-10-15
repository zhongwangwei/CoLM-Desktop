//! 端到端测试驱动：只在 `--features e2e-driver` 时编进来，发布包里没有。
//!
//! 为什么要它：macOS 上 Tauri 用 WKWebView，没有 tauri-driver/WebDriver；
//! 借系统辅助功能点窗口又要用户去改安全设置。这里换个方向——测试脚本把
//! 一段 JS 发到本机端口，后端用 `webview.eval` 在**真实窗口**里执行，结果经
//! Tauri 事件 `e2e://result` 回来。点击、填表、读 DOM 都发生在用户看到的同一个
//! 界面上，走的也是真实 IPC、真实 sidecar 与内核。
//!
//! 被否决的方案：用 `tauri::test` 的 MockRuntime 把命令表挂到 HTTP 上、前端放进
//! 普通浏览器。69 处命令写死了 Wry 的 `tauri::AppHandle`，要全改成泛型才挂得上，
//! 而且最后测的不是真窗口。
//!
//! 只在设置了 `COLM_E2E_PORT` 时启动，只绑 127.0.0.1。
//!
//! 原生文件对话框（rfd）同样点不到，所以另有两个旁路：`POST /pick` 预置下一次
//! `pick_file`/`pick_folder` 的结果；设了 `COLM_E2E_SAVE_DIR` 时导出直接写进那个目录。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Listener, Manager};

type Pending = Arc<Mutex<HashMap<u64, SyncSender<serde_json::Value>>>>;

static NEXT_PICK: Mutex<Option<String>> = Mutex::new(None);

/// 测试预置的下一次文件/目录选择结果（取一次就清空）。
pub fn take_pick() -> Option<String> {
    NEXT_PICK.lock().unwrap().take()
}

/// 导出不弹保存面板时写入的目录。
pub fn save_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("COLM_E2E_SAVE_DIR").map(std::path::PathBuf::from)
}

pub fn start(app: &tauri::AppHandle) {
    let Ok(port) = std::env::var("COLM_E2E_PORT") else {
        return;
    };
    let listener = match TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap_or(0))) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("colm-desktop e2e: cannot bind 127.0.0.1:{port}: {error}");
            return;
        }
    };
    eprintln!(
        "colm-desktop e2e: driver listening on {}",
        listener.local_addr().map(|a| a.to_string()).unwrap_or(port)
    );
    let pending: Pending = Arc::default();
    let results = pending.clone();
    app.listen_any("e2e://result", move |event| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(event.payload()) else {
            return;
        };
        let Some(id) = value.get("id").and_then(|v| v.as_u64()) else {
            return;
        };
        if let Some(tx) = results.lock().unwrap().remove(&id) {
            let _ = tx.send(value);
        }
    });
    let app = app.clone();
    std::thread::spawn(move || {
        let next = Arc::new(AtomicU64::new(1));
        for stream in listener.incoming().flatten() {
            let (app, pending, next) = (app.clone(), pending.clone(), next.clone());
            // 每个连接一条线程：长时间的运行（等待 run 结束）不能挡住并发的查询。
            std::thread::spawn(move || {
                if let Err(error) = serve(stream, &app, &pending, &next) {
                    eprintln!("colm-desktop e2e: {error}");
                }
            });
        }
    });
}

fn serve(
    mut stream: TcpStream,
    app: &tauri::AppHandle,
    pending: &Pending,
    next: &AtomicU64,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut length = 0usize;
    let mut timeout = 60u64;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
        if let Some(v) = lower.strip_prefix("x-e2e-timeout:") {
            timeout = v.trim().parse().unwrap_or(timeout);
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let reply = match path {
        "/ping" => serde_json::json!({"ok": true, "value": "pong"}),
        "/pick" => {
            let path = String::from_utf8_lossy(&body).trim().to_string();
            *NEXT_PICK.lock().unwrap() = (!path.is_empty()).then_some(path);
            serde_json::json!({"ok": true, "value": "armed"})
        }
        "/eval" => eval(app, pending, next, &String::from_utf8_lossy(&body), timeout),
        _ => serde_json::json!({"ok": false, "value": format!("unknown path {path}")}),
    };
    let text = reply.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        text.len(),
        text
    )?;
    stream.flush()
}

/// `body` 是一个 async 函数体：可以 `await`，`return` 的值要能 JSON 序列化。
fn eval(
    app: &tauri::AppHandle,
    pending: &Pending,
    next: &AtomicU64,
    body: &str,
    timeout: u64,
) -> serde_json::Value {
    let Some(window) = app.get_webview_window("main") else {
        return serde_json::json!({"ok": false, "value": "no main window"});
    };
    let id = next.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = sync_channel(1);
    pending.lock().unwrap().insert(id, tx);
    let script = format!(
        "(async () => {{ let r; try {{ const v = await (async () => {{ {body}\n }})(); \
         r = {{ id: {id}, ok: true, value: v === undefined ? null : v }}; }} \
         catch (e) {{ r = {{ id: {id}, ok: false, value: String((e && e.stack) || e) }}; }} \
         await window.__TAURI__.event.emit('e2e://result', r); }})();"
    );
    if let Err(error) = window.eval(&script) {
        pending.lock().unwrap().remove(&id);
        return serde_json::json!({"ok": false, "value": format!("eval failed: {error}")});
    }
    match rx.recv_timeout(Duration::from_secs(timeout)) {
        Ok(value) => value,
        Err(_) => {
            pending.lock().unwrap().remove(&id);
            serde_json::json!({"ok": false, "value": format!("timed out after {timeout} s")})
        }
    }
}
