//! In-memory, operation-scoped bridge from OpenSSH askpass to the desktop.
use serde::Serialize;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc, Mutex, OnceLock, Weak,
};
use std::time::{Duration, Instant};
use tauri::Emitter;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static QUEUE: OnceLock<Mutex<Queue>> = OnceLock::new();
const DEADLINE: Duration = Duration::from_secs(600);
#[derive(Default)]
struct Queue {
    revision: u64,
    pending: Vec<Pending>,
    operations: Vec<Weak<Operation>>,
}
struct Pending {
    view: AuthRequest,
    operation: Arc<Operation>,
    reply: mpsc::Sender<Option<String>>,
}
#[derive(Clone, Serialize)]
pub struct AuthRequest {
    request_id: String,
    operation_host: String,
    prompt: String,
    kind: String,
}
#[derive(Clone, Serialize)]
pub struct AuthSnapshot {
    revision: u64,
    requests: Vec<AuthRequest>,
}
struct Operation {
    id: String,
    host: String,
    pid: AtomicU32,
    child: Mutex<Option<crate::sidecar::ChildCancellation>>,
    closed: AtomicBool,
    secrets: Mutex<Vec<String>>,
}
impl Operation {
    fn stop(&self) -> Result<(), String> {
        let child = self
            .child
            .lock()
            .map_err(|_| "SSH child lifetime unavailable")?
            .clone();
        if let Some(child) = child {
            child.stop()?;
        }
        Ok(())
    }
}
pub(crate) struct Session {
    operation: Arc<Operation>,
    address: String,
    token: String,
}
fn queue() -> &'static Mutex<Queue> {
    QUEUE.get_or_init(|| Mutex::new(Queue::default()))
}
pub(crate) fn initialize(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
}
pub(crate) fn available() -> bool {
    APP.get().is_some()
}
fn changed() {
    if let Some(app) = APP.get() {
        if let Ok(snapshot) = remote_auth_pending() {
            let _ = app.emit("ssh-auth://changed", snapshot);
        }
    }
}
fn random_id() -> Result<String, String> {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|_| "SSH credential entropy unavailable")?;
        Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "$b=New-Object byte[] 32; $r=[System.Security.Cryptography.RandomNumberGenerator]::Create(); $r.GetBytes($b); [BitConverter]::ToString($b).Replace('-','').ToLowerInvariant()"] ).output().map_err(|_| "SSH credential entropy unavailable")?;
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !output.status.success()
            || token.len() != 64
            || !token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("SSH credential entropy unavailable".into());
        }
        Ok(token)
    }
}
#[tauri::command]
pub fn remote_auth_pending() -> Result<AuthSnapshot, String> {
    let state = queue()
        .lock()
        .map_err(|_| "SSH credential queue unavailable")?;
    Ok(AuthSnapshot {
        revision: state.revision,
        requests: state.pending.iter().map(|p| p.view.clone()).collect(),
    })
}
#[tauri::command]
pub async fn remote_auth_respond(request_id: String, value: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || respond(request_id, value))
        .await
        .map_err(|_| "SSH credential response worker failed".to_string())?
}
fn respond(request_id: String, value: Option<String>) -> Result<(), String> {
    if value.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > 16384 || value.contains(['\0', '\r', '\n'])
    }) {
        return Err("Invalid SSH credential".into());
    }
    let pending = {
        let mut state = queue()
            .lock()
            .map_err(|_| "SSH credential queue unavailable")?;
        let index = state
            .pending
            .iter()
            .position(|p| p.view.request_id == request_id)
            .ok_or("SSH request is no longer active")?;
        state.revision += 1;
        state.pending.remove(index)
    };
    changed();
    if pending.operation.closed.load(Ordering::SeqCst) {
        return Err("SSH request is no longer active".into());
    }
    if let Some(secret) = &value {
        pending
            .operation
            .secrets
            .lock()
            .map_err(|_| "SSH credential session unavailable")?
            .push(secret.clone());
    }
    if value.is_none() {
        close(&pending.operation);
        pending.operation.stop()?;
        return Ok(());
    }
    pending
        .reply
        .send(value)
        .map_err(|_| "SSH request is no longer active".into())
}
fn close(operation: &Operation) {
    {
        let _delivery = operation.child.lock().unwrap_or_else(|e| e.into_inner());
        operation.closed.store(true, Ordering::SeqCst);
    }
    let mut state = queue().lock().unwrap_or_else(|e| e.into_inner());
    let before = state.pending.len();
    state.pending.retain(|p| {
        if p.operation.id == operation.id {
            let _ = p.reply.send(None);
            false
        } else {
            true
        }
    });
    let removed = before != state.pending.len();
    if removed {
        state.revision += 1;
    }
    drop(state);
    if removed {
        changed();
    }
}
pub(crate) fn cancel_pid(pid: u32) {
    let operations: Vec<_> = queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .operations
        .iter()
        .filter_map(Weak::upgrade)
        .filter(|operation| operation.pid.load(Ordering::SeqCst) == pid)
        .collect();
    for operation in operations {
        close(&operation);
    }
}
pub(crate) fn cancel_all() {
    let operations: Vec<_> = queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .operations
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
    for operation in operations {
        close(&operation);
    }
}
impl Session {
    pub(crate) fn begin(host: String) -> Result<Option<Self>, String> {
        if !available() {
            return Ok(None);
        }
        Self::new(host).map(Some)
    }
    fn new(host: String) -> Result<Self, String> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|_| "Cannot start SSH credential broker")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot start SSH credential broker")?;
        let address = listener
            .local_addr()
            .map_err(|_| "Cannot start SSH credential broker")?
            .to_string();
        let token = random_id()?;
        let operation = Arc::new(Operation {
            id: random_id()?,
            host,
            pid: AtomicU32::new(0),
            child: Mutex::new(None),
            closed: AtomicBool::new(false),
            secrets: Mutex::new(Vec::new()),
        });
        {
            let mut state = queue().lock().unwrap_or_else(|e| e.into_inner());
            state
                .operations
                .retain(|operation| operation.strong_count() > 0);
            state.operations.push(Arc::downgrade(&operation));
        }
        let worker = operation.clone();
        let credential = token.clone();
        std::thread::spawn(move || {
            while !worker.closed.load(Ordering::SeqCst) {
                if worker.pid.load(Ordering::SeqCst) == 0 {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve(stream, &credential, &worker, Instant::now() + DEADLINE)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
            if !worker.closed.load(Ordering::SeqCst) {
                close(&worker);
                let _ = worker.stop();
            }
        });
        Ok(Self {
            operation,
            address,
            token,
        })
    }
    pub(crate) fn attach(&self, command: &mut std::process::Command) {
        command
            .env("COLM_SSH_GUI_BROKER", &self.address)
            .env("COLM_SSH_GUI_TOKEN", &self.token);
    }
    pub(crate) fn bind(&self, child: &crate::sidecar::DetachedChildGuard) {
        let cancellation = child.cancellation();
        let pid = cancellation.pid();
        *self
            .operation
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(cancellation);
        self.operation.pid.store(pid, Ordering::SeqCst);
        if self.operation.closed.load(Ordering::SeqCst) {
            let _ = self.operation.stop();
        }
    }
    pub(crate) fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in self
            .operation
            .secrets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            text = text.replace(secret, "[redacted]");
        }
        text
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        close(&self.operation);
    }
}
fn serve(mut stream: TcpStream, token: &str, operation: &Arc<Operation>, deadline: Instant) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let handshake_deadline = Instant::now() + Duration::from_secs(3);
    let mut supplied = [0; 64];
    if stream.read_exact(&mut supplied).is_err() || supplied != token.as_bytes() {
        return;
    }
    let mut bytes = Vec::new();
    for _ in 0..4096 {
        let remaining = handshake_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        let _ = stream.set_read_timeout(Some(remaining));
        let mut byte = [0];
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }
    if bytes.len() == 4096 {
        return;
    }
    let Ok(prompt) = String::from_utf8(bytes) else {
        return;
    };
    if operation.closed.load(Ordering::SeqCst) {
        return;
    }
    if prompt.chars().any(char::is_control) {
        return;
    }
    let lower = prompt.trim().to_ascii_lowercase();
    let kind = if lower.starts_with("enter passphrase for key '") && lower.ends_with("':") {
        "passphrase"
    } else if lower.ends_with("password:") {
        "password"
    } else {
        return;
    };
    let Ok(id) = random_id() else {
        return;
    };
    let (reply, receiver) = mpsc::channel();
    {
        let mut state = queue().lock().unwrap_or_else(|e| e.into_inner());
        if operation.closed.load(Ordering::SeqCst) {
            return;
        }
        state.pending.push(Pending {
            view: AuthRequest {
                request_id: id,
                operation_host: operation.host.clone(),
                prompt: prompt.clone(),
                kind: kind.into(),
            },
            operation: operation.clone(),
            reply,
        });
        state.revision += 1;
    }
    changed();
    loop {
        if operation.closed.load(Ordering::SeqCst) {
            return;
        }
        if Instant::now() >= deadline {
            close(operation);
            let _ = operation.stop();
            return;
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(secret)) => {
                // Serialize delivery against close, never against the user's wait.
                let _delivery = operation.child.lock().unwrap_or_else(|e| e.into_inner());
                if operation.closed.load(Ordering::SeqCst) {
                    return;
                }
                let _ = stream.write_all(secret.as_bytes());
                return;
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}
#[cfg(test)]
#[path = "ssh_credentials_tests.rs"]
mod tests;
