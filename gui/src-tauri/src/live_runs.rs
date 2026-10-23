//! 本机运行的落盘登记，供崩溃/强退之后的下一次启动清理孤儿进程（GUI 审查 B-12）。
//!
//! `top_level_sidecar` 让每个 colm-cli 自成进程组，窗口进程死了它们照跑：重开以后 `RunProcesses`
//! 是空的，界面既不当它在跑、也停不了它，再点一次开始就有两个进程写同一个 `runs/`。正常关窗由
//! `stop_everything` 收拾；这里只管"没来得及收拾"的那种。
//!
//! 每个运行一个 `<pid>.json`，开始时写、结束时删，不做读改写，并发运行之间不用加锁。只在真正的
//! 应用里打开（[`enable`]），单元测试里的 `RunProcesses` 不会碰用户目录。
//!
//! 启动时只停"仍活着、自成进程组、命令行里是 colm-cli"的进程：PID 会被复用，三条都对上才认。
//! 被否决：启动时把它们接回界面继续显示进度 —— stdout 管道已随旧窗口断了，进度和日志都接不回来，
//! 半截的输出也不能当结果用。
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();
static SWEPT: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(serde::Serialize, serde::Deserialize)]
struct Entry {
    key: String,
    pid: u32,
    /// 写登记时 colm-cli 的文件名，用来核对 PID 没被别的程序复用。
    program: String,
}

/// 登记目录放在历史库旁边：跟着 `COLM_HISTORY_DB`/隔离 HOME 走，e2e 不会碰到真实用户的登记。
fn default_dir() -> Option<PathBuf> {
    colm_history::default_db_path()
        .ok()?
        .parent()
        .map(|parent| parent.join("live-runs"))
}

/// 应用启动时调用：打开登记，并先清掉上次没收拾的运行。
pub(crate) fn enable() {
    let Some(dir) = default_dir() else {
        return;
    };
    let stopped = sweep(&dir, &program_name(), alive_colm_cli);
    if let Ok(mut swept) = SWEPT.lock() {
        *swept = stopped;
    }
    let _ = DIR.set(dir);
}

fn program_name() -> String {
    crate::sidecar::resolve_cli()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "colm-cli".into())
}

pub(crate) fn record(key: &str, pid: u32) {
    let Some(dir) = DIR.get() else {
        return;
    };
    let entry = Entry {
        key: key.to_owned(),
        pid,
        program: program_name(),
    };
    // 写不下只是少了崩溃后的清理，不能因此拒绝运行。
    let _ = std::fs::create_dir_all(dir).and_then(|()| {
        std::fs::write(
            dir.join(format!("{pid}.json")),
            serde_json::to_vec(&entry).unwrap_or_default(),
        )
    });
}

pub(crate) fn clear(pid: u32) {
    if let Some(dir) = DIR.get() {
        let _ = std::fs::remove_file(dir.join(format!("{pid}.json")));
    }
}

/// 读出每条登记；还活着的就停掉（研究再补一次取消收尾），登记一律删。返回被停掉的运行键。
fn sweep(dir: &Path, program: &str, alive: impl Fn(u32) -> Option<String>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut stopped = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let entry = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Entry>(&bytes).ok());
        if let Some(entry) = entry {
            let ours = alive(entry.pid).is_some_and(|command| {
                command.contains(&entry.program) || command.contains(program)
            });
            if ours && crate::sidecar::terminate_process_tree(entry.pid).is_ok() {
                if let Some(study) = entry.key.strip_prefix("study:") {
                    let _ = crate::sidecar::capture(&[
                        "study-finalize-cancel".into(),
                        study.into(),
                        "--pid".into(),
                        entry.pid.to_string(),
                    ]);
                }
                stopped.push(entry.key);
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    stopped
}

/// 这个 PID 仍是一个自成进程组的活进程时返回它的命令行。
#[cfg(unix)]
fn alive_colm_cli(pid: u32) -> Option<String> {
    let output = crate::sidecar::sidecar_command("ps")
        .args(["-p", &pid.to_string(), "-o", "pgid=,stat=,command="])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    let mut fields = line.splitn(3, char::is_whitespace);
    let group = fields.next()?;
    let stat = fields.next()?.trim();
    let command = fields.next().unwrap_or("").trim();
    (group == pid.to_string() && !stat.starts_with('Z')).then(|| command.to_owned())
}

#[cfg(windows)]
fn alive_colm_cli(pid: u32) -> Option<String> {
    let output = crate::sidecar::sidecar_command("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.contains(&format!("\"{pid}\"")).then_some(text)
}

#[cfg(not(any(unix, windows)))]
fn alive_colm_cli(_pid: u32) -> Option<String> {
    None
}

/// 这次启动时停掉的上次遗留运行；只交出一次，前端提示过就清空。
#[tauri::command]
pub fn live_runs_swept() -> Vec<String> {
    SWEPT
        .lock()
        .map(|mut swept| std::mem::take(&mut *swept))
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "live_runs_tests.rs"]
mod live_runs_tests;
