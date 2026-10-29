//! Human terminal frontend; the existing JSONL service remains the agent runtime.
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::backend::{BackendKind, ExternalChoice};
use crate::protocol::{Inbound, Outbound};
use crate::provider::ProviderConfig;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const HELP: &str = "CoLM terminal assistant\n\n  colm-agent chat [OPTIONS]\n  colm-agent config [OPTIONS]\n  colm-agent --data-dir PATH --cli PATH --key-file PATH  (JSONL service)\n\nOptions (saved without API keys):\n  --data-dir PATH  --config PATH  --cli PATH  --key-file PATH\n  --project-root PATH  --docs-root PATH  --kernel-dir PATH  --servers PATH\n  --backend builtin|codex|claude_code|opencode\n  --base-url URL  --provider-id ID  --model MODEL\n  --api-format chat_completions|responses|anthropic\n  --context-window TOKENS  (model context window; required for models without a documented preset)\n  --thinking true|false  --reasoning-effort VALUE\nChat only: --message TEXT (one turn; approvals denied), --resume SESSION\n\nStore API keys using stdin:\n  colm-agent --key-file ~/.colm-agent/keys.json --set-key https://api.deepseek.com\n\nChat commands: /help /new /sessions /resume ID /cancel /exit\nWrites and runs require explicit approval. Type y or n at the approval prompt.\n";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    provider: ProviderConfig,
    #[serde(default)]
    backend: BackendKind,
    #[serde(default)]
    external: ExternalChoice,
    project_root: PathBuf,
    #[serde(default)]
    docs_root: Option<PathBuf>,
    #[serde(default)]
    kernel_dir: Option<PathBuf>,
    #[serde(default)]
    servers: Option<PathBuf>,
    #[serde(default)]
    cli: Option<PathBuf>,
    #[serde(default)]
    key_file: Option<PathBuf>,
}

impl Config {
    fn new() -> Result<Self> {
        Ok(Self {
            // deepseek-chat 已停用，而且新逻辑只对文档确认的 deepseek-flash / deepseek-v4-pro
            // 用预设窗口；默认选前者，免得开箱即报“未知窗口”。
            provider: ProviderConfig::deepseek("deepseek-flash", String::new()),
            backend: BackendKind::Builtin,
            external: ExternalChoice::default(),
            project_root: std::env::current_dir()?,
            docs_root: None,
            kernel_dir: None,
            servers: None,
            cli: None,
            key_file: None,
        })
    }
    fn configure(&self) -> Value {
        json!({"type":"configure", "provider":self.provider,"backend":self.backend,"external":self.external,
            "project_root":self.project_root,"docs_root":self.docs_root,"kernel_dir":self.kernel_dir,
            "approval":"ask","ui":false,"remote":self.servers.is_some(),"web_search":false})
    }
}

/// Returns true only when this invocation belongs to the terminal frontend.
pub fn dispatch(args: &[String]) -> Result<bool> {
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(true);
    }
    if !args.first().is_some_and(|a| a == "chat" || a == "config") {
        return Ok(false);
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(true);
    }
    let opts = options(&args[1..])?;
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let data = opts
        .get("--data-dir")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| PathBuf::from(h).join(".colm-agent")))
        .context("set --data-dir: home directory is unavailable")?;
    let path = opts
        .get("--config")
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("config.json"));
    let mut config: Config = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?).context("invalid terminal config")?
    } else {
        Config::new()?
    };
    apply(&mut config, &opts)?;
    if config.docs_root.is_none() {
        let docs = std::env::current_exe()?.with_file_name("docs");
        if docs.is_dir() {
            config.docs_root = Some(docs);
        }
    }
    if !config.project_root.is_dir() {
        bail!(
            "project root is not a directory: {}",
            config.project_root.display()
        );
    }
    std::fs::create_dir_all(&data)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // A config never contains keys; ProviderConfig skips api_key during serialization.
    let serialized = serde_json::to_vec_pretty(&config)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, &serialized)?;
    std::fs::rename(&temporary, &path)?;
    if args[0] == "config" {
        println!("{}\n{}", path.display(), String::from_utf8(serialized)?);
        return Ok(true);
    }
    let cli = config.cli.clone().unwrap_or(
        std::env::current_exe()?
            .with_file_name(format!("colm-cli{}", std::env::consts::EXE_SUFFIX)),
    );
    let cli = if cli.is_file() {
        cli
    } else if config.cli.is_some() {
        bail!("colm-cli not found: {}", cli.display())
    } else {
        PathBuf::from(format!("colm-cli{}", std::env::consts::EXE_SUFFIX))
    };
    let key = config
        .key_file
        .clone()
        .unwrap_or_else(|| data.join("keys.json"));
    let remote = config
        .servers
        .clone()
        .map(|p| crate::terminal_remote::TerminalRemote::new(p, cli.clone()).map(Arc::new))
        .transpose()?;
    if let Some(id) = opts.get("--resume") {
        validate_resume(&data, id)?;
    }
    let _signals = crate::terminal_signals::Guard::install()?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .env("COLM_TERMINAL_PARENT", "1")
        .args(["--data-dir"])
        .arg(&data)
        .arg("--cli")
        .arg(cli)
        .arg("--key-file")
        .arg(key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().context("start agent service")?;
    let result = chat(
        &mut child,
        &config,
        &data,
        opts.get("--message").cloned(),
        opts.get("--resume").cloned(),
        remote,
    );
    shutdown(&mut child);
    result?;
    Ok(true)
}

fn options(args: &[String]) -> Result<std::collections::BTreeMap<String, String>> {
    let mut out = std::collections::BTreeMap::new();
    let valid = [
        "--data-dir",
        "--config",
        "--cli",
        "--key-file",
        "--project-root",
        "--docs-root",
        "--kernel-dir",
        "--servers",
        "--backend",
        "--base-url",
        "--provider-id",
        "--model",
        "--api-format",
        "--context-window",
        "--thinking",
        "--reasoning-effort",
        "--message",
        "--resume",
    ];
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if !valid.contains(&flag.as_str()) {
            bail!("unknown option {flag}; use --help");
        }
        let value = args
            .next()
            .with_context(|| format!("missing value for {flag}"))?;
        if value.starts_with("--") {
            bail!("missing value for {flag}");
        }
        out.insert(flag.clone(), value.clone());
    }
    Ok(out)
}
fn apply(c: &mut Config, o: &std::collections::BTreeMap<String, String>) -> Result<()> {
    for (flag, v) in o {
        match flag.as_str() {
            "--project-root" => c.project_root = absolute(v)?,
            "--docs-root" => c.docs_root = Some(absolute(v)?),
            "--kernel-dir" => c.kernel_dir = Some(absolute(v)?),
            "--servers" => c.servers = Some(absolute(v)?),
            "--cli" => c.cli = Some(absolute(v)?),
            "--key-file" => c.key_file = Some(absolute(v)?),
            "--backend" => c.backend = serde_json::from_value(json!(v))?,
            "--base-url" => {
                c.provider.base_url = v.clone();
                c.provider.provider_id.clear();
            }
            "--model" => {
                c.provider.model = v.clone();
                c.external.model = Some(v.clone());
            }
            "--api-format" => c.provider.api_format = serde_json::from_value(json!(v))?,
            "--context-window" => c.provider.context_window = Some(parse_context_window(v)?),
            "--thinking" => {
                c.provider.thinking = Some(v.parse().context("thinking must be true or false")?)
            }
            "--reasoning-effort" => {
                c.provider.reasoning_effort = Some(v.clone());
                c.external.effort = Some(v.clone());
            }
            _ => {}
        }
    }
    if let Some(v) = o.get("--provider-id") {
        c.provider.provider_id = v.clone();
    }
    if c.provider.model.trim().is_empty() {
        bail!("model must not be empty");
    }
    let url: ureq::http::Uri = c.provider.base_url.parse().context("invalid base URL")?;
    if !matches!(url.scheme_str(), Some("http" | "https"))
        || url.host().is_none()
        || url.authority().is_some_and(|v| v.as_str().contains('@'))
        || url.query().is_some()
        || c.provider.base_url.contains('#')
    {
        bail!("base URL must be HTTP(S) without credentials, query or fragment");
    }
    Ok(())
}
/// `--context-window`：只接受合理范围的整数 token；非法值在写配置前报错。
fn parse_context_window(value: &str) -> Result<u64> {
    let window = value
        .trim()
        .parse::<u64>()
        .context("context window must be a whole number of tokens")?;
    if !(crate::provider::MIN_CONTEXT_WINDOW_TOKENS..=crate::provider::MAX_CONTEXT_WINDOW_TOKENS)
        .contains(&window)
    {
        bail!(
            "context window must be between {} and {} tokens",
            crate::provider::MIN_CONTEXT_WINDOW_TOKENS,
            crate::provider::MAX_CONTEXT_WINDOW_TOKENS
        );
    }
    Ok(window)
}
fn absolute(value: &str) -> Result<PathBuf> {
    let p = PathBuf::from(value);
    Ok(if p.is_absolute() {
        p
    } else {
        std::env::current_dir()?.join(p)
    })
}
fn send(out: &mut impl Write, message: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *out, message)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

enum Event {
    Input(String),
    Eof,
    Service(Result<Outbound>),
    Closed,
    Remote(String, Result<Value>),
    Password(String, String, mpsc::Sender<Result<String>>),
}

fn chat(
    child: &mut Child,
    config: &Config,
    data: &Path,
    mut message: Option<String>,
    mut resume: Option<String>,
    remote: Option<Arc<crate::terminal_remote::TerminalRemote>>,
) -> Result<()> {
    let one_shot = message.is_some();
    let interactive = !one_shot && std::io::stdin().is_terminal();
    let mut input = ServiceInput(child.stdin.take().context("service stdin unavailable")?);
    let output = child.stdout.take().context("service stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    let events = tx.clone();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(output).lines() {
            let event = line
                .map_err(anyhow::Error::from)
                .and_then(|s| serde_json::from_str(&s).context("invalid service event"));
            if events.send(Event::Service(event)).is_err() {
                return;
            }
        }
        let _ = events.send(Event::Closed);
    });
    if !one_shot {
        let events = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                match line {
                    Ok(line) => {
                        if events.send(Event::Input(line)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = events.send(Event::Eof);
        });
    }
    send(&mut input, &config.configure())?;
    let mut busy = true;
    let mut remote_job: Option<RemoteJob> = None;
    let mut expected_resume: Option<String> = None;
    let mut approval: Option<String> = None;
    let mut password: Option<mpsc::Sender<Result<String>>> = None;
    let mut echo = EchoGuard(false);
    let mut streamed = false;
    let mut cancelled = false;
    let mut denied = false;
    let mut discard_password_line = false;
    loop {
        if crate::terminal_signals::interrupted() {
            bail!("terminal interrupted; active work cancelled");
        }
        let event = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("terminal event channel closed"),
        };
        match event {
            Event::Closed => bail!("agent service exited unexpectedly"),
            Event::Eof => {
                send(&mut input, &Inbound::Cancel)?;
                break;
            }
            Event::Input(line) => {
                if discard_password_line {
                    discard_password_line = false;
                    echo.restore();
                    continue;
                }
                if let Some(reply) = password.take() {
                    echo.restore();
                    println!();
                    if line == "/cancel" || line == "/exit" {
                        let _ = reply.send(Err(anyhow::anyhow!("password cancelled")));
                        cancelled = true;
                        cancel_remote(&remote_job);
                        send(&mut input, &Inbound::Cancel)?;
                        if line == "/exit" {
                            break;
                        }
                    } else {
                        let _ = reply.send(Ok(line));
                    }
                    continue;
                }
                let line = line.trim();
                if line == "/exit" {
                    send(&mut input, &Inbound::Cancel)?;
                    break;
                }
                if line == "/cancel" {
                    cancelled = true;
                    cancel_remote(&remote_job);
                    approval = None;
                    send(&mut input, &Inbound::Cancel)?;
                    continue;
                }
                if let Some(id) = approval.as_ref() {
                    if let Some(approve) = approval_answer(line) {
                        send(
                            &mut input,
                            &Inbound::ApprovalDecision {
                                id: id.clone(),
                                approve,
                                note: None,
                                remember: false,
                            },
                        )?;
                        approval = None;
                    } else {
                        println!("Type y to approve, n to deny, or /cancel.");
                    }
                    continue;
                }
                if line == "/help" {
                    print!("{HELP}");
                    continue;
                }
                if line == "/sessions" {
                    for s in crate::session::list(data)? {
                        println!("{}  {}", s.id, s.title);
                    }
                    continue;
                }
                if busy || remote_job.is_some() {
                    println!("Still working or cancelling; wait for completion, or use /cancel.");
                    continue;
                }
                if line == "/new" {
                    send(&mut input, &Inbound::NewSession)?;
                    busy = true;
                } else if let Some(id) = line.strip_prefix("/resume ") {
                    let id = id.trim();
                    if let Err(error) = validate_resume(data, id) {
                        eprintln!("{error:#}");
                        continue;
                    }
                    expected_resume = Some(id.to_owned());
                    send(
                        &mut input,
                        &Inbound::ResumeSession {
                            id: id.trim().into(),
                        },
                    )?;
                    busy = true;
                } else if line.starts_with('/') {
                    println!("Unknown command; use /help.");
                } else if !line.is_empty() {
                    send(
                        &mut input,
                        &Inbound::UserMessage {
                            text: line.into(),
                            context: None,
                            request_id: None,
                        },
                    )?;
                    busy = true;
                    streamed = false;
                    cancelled = false;
                }
            }
            Event::Password(id, label, reply) => {
                if !interactive
                    || discard_password_line
                    || password.is_some()
                    || !busy
                    || cancelled
                    || remote_job
                        .as_ref()
                        .is_none_or(|job| job.id != id || job.cancel.load(Ordering::SeqCst))
                {
                    let _ = reply.send(Err(anyhow::anyhow!(
                        "password requires an interactive terminal"
                    )));
                } else if let Err(error) = echo.hide() {
                    let _ = reply.send(Err(error));
                } else {
                    print!("Password for {label}: ");
                    std::io::stdout().flush()?;
                    password = Some(reply);
                }
            }
            Event::Remote(id, result) => {
                if remote_job.as_ref().is_none_or(|job| job.id != id) {
                    continue;
                }
                let was_cancelled = remote_job.as_ref().unwrap().cancel.load(Ordering::SeqCst);
                remote_job.take();
                if was_cancelled || cancelled {
                    continue;
                }
                let (ok, result) = match result {
                    Ok(v) => (true, v),
                    Err(e) => (false, json!({"error":format!("{e:#}")})),
                };
                send(&mut input, &Inbound::UiResult { id, ok, result })?;
            }
            Event::Service(event) => match event? {
                Outbound::Ready { session, model } => {
                    if let Some(id) = resume.take() {
                        validate_resume(data, &id)?;
                        expected_resume = Some(id.clone());
                        send(&mut input, &Inbound::ResumeSession { id })?;
                        continue;
                    }
                    if let Some(expected) = expected_resume.take() {
                        if session != expected {
                            bail!("service did not resume requested session {expected}");
                        }
                    }
                    busy = false;
                    if !one_shot {
                        println!("Session {session} · {model}\n/help for commands");
                    }
                    if let Some(text) = message.take() {
                        send(
                            &mut input,
                            &Inbound::UserMessage {
                                text,
                                context: None,
                                request_id: None,
                            },
                        )?;
                        busy = true;
                    }
                }
                Outbound::AssistantDelta { text } => {
                    streamed = true;
                    print!("{text}");
                    std::io::stdout().flush()?;
                }
                Outbound::ToolCall { name, summary, .. } => println!("\n[{name}] {summary}"),
                Outbound::ToolResult {
                    name, ok, result, ..
                } => println!("\n[{name}: {}] {result}", if ok { "ok" } else { "failed" }),
                Outbound::ApprovalRequest {
                    id,
                    name,
                    summary,
                    arguments,
                    ..
                } => {
                    if interactive {
                        println!("\nApprove {name}?\n{summary}\n{arguments}\n[y/n]");
                        approval = Some(id);
                    } else {
                        denied = true;
                        send(
                            &mut input,
                            &Inbound::ApprovalDecision {
                                id,
                                approve: false,
                                note: Some("noninteractive terminal: approval denied".into()),
                                remember: false,
                            },
                        )?;
                    }
                }
                Outbound::UiRequest { id, action, args } => {
                    if cancelled || remote_job.is_some() {
                        send(
                            &mut input,
                            &Inbound::UiResult {
                                id,
                                ok: false,
                                result: json!({"error":"operation cancelled or previous remote operation is still active"}),
                            },
                        )?;
                    } else if let Some(remote) = remote.clone() {
                        let events = tx.clone();
                        let cancel = Arc::new(AtomicBool::new(false));
                        let token = cancel.clone();
                        let request_id = id.clone();
                        let worker = std::thread::spawn(move || {
                            let result = remote.handle_with_cancel(
                                &action,
                                args,
                                |label| {
                                    let (reply, answer) = mpsc::channel();
                                    events.send(Event::Password(
                                        request_id.clone(),
                                        label.into(),
                                        reply,
                                    ))?;
                                    loop {
                                        if token.load(Ordering::SeqCst) {
                                            bail!("remote operation cancelled");
                                        }
                                        match answer.recv_timeout(Duration::from_millis(100)) {
                                            Ok(answer) => return answer,
                                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                                            Err(_) => bail!("password prompt closed"),
                                        }
                                    }
                                },
                                &token,
                            );
                            let _ = events.send(Event::Remote(request_id, result));
                        });
                        remote_job = Some(RemoteJob {
                            id,
                            cancel,
                            worker: Some(worker),
                        });
                    } else {
                        send(
                            &mut input,
                            &Inbound::UiResult {
                                id,
                                ok: false,
                                result: json!({"error":"terminal remote servers are not configured"}),
                            },
                        )?;
                    }
                }
                Outbound::TurnDone { content, .. } => {
                    cancel_remote(&remote_job);
                    if let Some(reply) = password.take() {
                        discard_password_line = true;
                        println!("Password request ended; press Enter to clear pending input.");
                        let _ = reply.send(Err(anyhow::anyhow!("turn ended")));
                    }
                    if !streamed {
                        print!("{content}");
                    }
                    println!();
                    busy = false;
                    approval = None;
                    streamed = false;
                    if one_shot {
                        if denied {
                            bail!("turn required approval; rerun interactively to authorize the operation");
                        }
                        break;
                    }
                }
                Outbound::Error { message } => {
                    cancel_remote(&remote_job);
                    if let Some(reply) = password.take() {
                        discard_password_line = true;
                        println!("Password request ended; press Enter to clear pending input.");
                        let _ = reply.send(Err(anyhow::anyhow!("turn failed")));
                    }
                    busy = false;
                    approval = None;
                    if one_shot {
                        bail!("{message}");
                    }
                    eprintln!("Error: {message}");
                }
                Outbound::ReasoningDelta { .. }
                | Outbound::TurnStarted { .. }
                | Outbound::Usage { .. }
                | Outbound::Context { .. }
                // 副作用结局是给审计与恢复用的结构信息；终端里已有 ok/failed 卡片，不重复显示。
                | Outbound::ToolOutcome { .. }
                | Outbound::DocsIndexStatus { .. }
                | Outbound::TaskState { .. } => {}
            },
        }
    }
    Ok(())
}
fn validate_resume(data: &Path, id: &str) -> Result<()> {
    if !crate::session::list(data)?
        .iter()
        .any(|session| session.id == id)
    {
        bail!("unknown saved session {id}; use /sessions to list available sessions");
    }
    Ok(())
}
struct RemoteJob {
    id: String,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
fn cancel_remote(job: &Option<RemoteJob>) {
    if let Some(job) = job {
        job.cancel.store(true, Ordering::SeqCst);
    }
}
impl Drop for RemoteJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn approval_answer(value: &str) -> Option<bool> {
    match value {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

struct EchoGuard(bool);
impl EchoGuard {
    fn hide(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            if Command::new("stty")
                .arg("-echo")
                .stdin(Stdio::inherit())
                .status()?
                .success()
            {
                self.0 = true;
                return Ok(());
            }
        }
        bail!("cannot hide password input; use SSH key authentication")
    }
    fn restore(&mut self) {
        if self.0 {
            let _ = Command::new("stty")
                .arg("echo")
                .stdin(Stdio::inherit())
                .status();
            self.0 = false;
        }
    }
}
impl Drop for EchoGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
struct ServiceInput(std::process::ChildStdin);
impl Write for ServiceInput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl Drop for ServiceInput {
    fn drop(&mut self) {
        let _ = send(&mut self.0, &Inbound::Cancel);
    }
}
fn shutdown(child: &mut Child) {
    child.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
        std::thread::sleep(Duration::from_millis(200));
        // Reap only after signaling the owned group, so its identifier cannot be reused.
        let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_excludes_secrets_and_requires_approval() {
        let mut c = Config::new().unwrap();
        c.provider.api_key = "never-persist-me".into();
        assert!(!serde_json::to_string(&c)
            .unwrap()
            .contains("never-persist-me"));
        assert_eq!(c.configure()["approval"], "ask");
        assert_eq!(c.configure()["ui"], false);
    }
    #[test]
    fn configuration_updates_both_backends_and_validates_flags() {
        let mut c = Config::new().unwrap();
        let opts = options(
            &[
                "--model",
                "test-model",
                "--backend",
                "codex",
                "--thinking",
                "false",
            ]
            .map(String::from),
        )
        .unwrap();
        apply(&mut c, &opts).unwrap();
        assert_eq!(c.external.model.as_deref(), Some("test-model"));
        assert_eq!(c.provider.thinking, Some(false));
        assert_eq!(c.backend, BackendKind::Codex);
        assert!(options(&["--api-key", "secret"].map(String::from)).is_err());
    }
    #[test]
    fn approval_requires_explicit_answer() {
        assert_eq!(approval_answer("y"), Some(true));
        assert_eq!(approval_answer("n"), Some(false));
        for input in ["", "ok", "run this", "yes please"] {
            assert_eq!(approval_answer(input), None);
        }
    }
    #[cfg(unix)]
    #[test]
    fn one_shot_streams_and_denies_approval_through_service_protocol() {
        let script = r#"
import json,sys
read=lambda:json.loads(sys.stdin.readline())
assert read()['type']=='configure'
print(json.dumps(dict(type='ready',session='test',model='fake')),flush=True)
assert read()['text']=='hello'
print(json.dumps(dict(type='approval_request',id='a',name='run',tier='act',summary='run',arguments='{}')),flush=True)
answer=read()
assert answer['type']=='approval_decision' and not answer['approve'] and not answer['remember']
print(json.dumps(dict(type='assistant_delta',text='safe answer')),flush=True)
print(json.dumps(dict(type='turn_done',content='safe answer',steps=1)),flush=True)
assert read()['type']=='cancel'
"#;
        let mut child = Command::new("python3")
            .args(["-u", "-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let c = Config::new().unwrap();
        let result = chat(
            &mut child,
            &c,
            Path::new("."),
            Some("hello".into()),
            None,
            None,
        );
        shutdown(&mut child);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("required approval"));
        assert!(child.wait().unwrap().success());
    }
    #[cfg(unix)]
    #[test]
    fn one_shot_error_is_failure_and_cancels_service() {
        let script = r#"
import json,sys
json.loads(sys.stdin.readline())
print(json.dumps(dict(type='error',message='provider rejected request')),flush=True)
assert json.loads(sys.stdin.readline())['type']=='cancel'
"#;
        let mut child = Command::new("python3")
            .args(["-u", "-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let c = Config::new().unwrap();
        let result = chat(
            &mut child,
            &c,
            Path::new("."),
            Some("hello".into()),
            None,
            None,
        );
        shutdown(&mut child);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("provider rejected"));
        assert!(child.wait().unwrap().success());
    }
    #[test]
    fn rejects_empty_models_and_credential_urls() {
        for (flag, value) in [
            ("--model", ""),
            ("--base-url", "file:///tmp/service"),
            ("--base-url", "https://secret@host/v1"),
            ("--base-url", "https://host/v1?key=secret"),
        ] {
            let mut c = Config::new().unwrap();
            let opts = options(&[flag, value].map(String::from)).unwrap();
            assert!(apply(&mut c, &opts).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn one_shot_success_waits_for_turn_done() {
        let script = r#"
import json,sys
assert json.loads(sys.stdin.readline())['type']=='configure'
print(json.dumps(dict(type='ready',session='s',model='fake')),flush=True)
assert json.loads(sys.stdin.readline())['text']=='hello'
print(json.dumps(dict(type='turn_done',content='answer',steps=0)),flush=True)
assert json.loads(sys.stdin.readline())['type']=='cancel'
"#;
        let mut child = Command::new("python3")
            .args(["-u", "-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let result = chat(
            &mut child,
            &Config::new().unwrap(),
            Path::new("."),
            Some("hello".into()),
            None,
            None,
        );
        shutdown(&mut child);
        result.unwrap();
        assert!(child.wait().unwrap().success());
    }
    #[cfg(unix)]
    #[test]
    fn shutdown_terminates_owned_unresponsive_service_group() {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new("sh")
            .args(["-c", "sleep 60 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        shutdown(&mut child);
        assert!(!child.wait().unwrap().success());
    }
    #[test]
    fn unknown_resume_is_rejected_before_model_message() {
        let dir = std::env::temp_dir().join(format!("colm-unknown-session-{}", std::process::id()));
        assert!(validate_resume(&dir, "typo-session")
            .unwrap_err()
            .to_string()
            .contains("unknown saved session"));
    }
    #[test]
    fn dropping_remote_job_cancels_and_joins_before_next_operation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let worker = std::thread::spawn(move || {
            while !token.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            done.store(true, Ordering::SeqCst);
        });
        drop(RemoteJob {
            id: "old".into(),
            cancel,
            worker: Some(worker),
        });
        assert!(finished.load(Ordering::SeqCst));
    }
    #[cfg(unix)]
    #[test]
    fn aborted_password_input_is_hidden_and_never_sent_as_chat() {
        const CHILD: &str = "COLM_PASSWORD_TEST_DIR";
        if let Some(dir) = std::env::var_os(CHILD) {
            let dir = PathBuf::from(dir);
            let server = dir.join("servers.json");
            std::fs::write(&server,r#"{"servers":[{"host":"test-host","root":"/test","auth":"password","username":"test"}]}"#).unwrap();
            let remote = Arc::new(
                crate::terminal_remote::TerminalRemote::new(server, PathBuf::from("/bin/false"))
                    .unwrap(),
            );
            let script = r#"
import sys,json,pathlib,time
root=pathlib.Path(sys.argv[1])
for line in sys.stdin:
 with (root/'requests').open('a') as log: log.write(line)
 m=json.loads(line)
 if m['type']=='configure':
  print(json.dumps(dict(type='ready',session='s',model='fake')),flush=True)
 elif m['type']=='user_message':
  print(json.dumps(dict(type='ui_request',id='remote',action='workspace_remote',args=dict(name='work',operation='submit',host='test-host',request=dict(action='test')))),flush=True)
  deadline=time.monotonic()+10
  while not (root/'abort').exists() and time.monotonic()<deadline: time.sleep(.01)
  print(json.dumps(dict(type='error',message='prompt aborted')),flush=True)
"#;
            let mut child = Command::new("python3")
                .args(["-u", "-c", script])
                .arg(&dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let result = chat(
                &mut child,
                &Config::new().unwrap(),
                &dir,
                None,
                None,
                Some(remote),
            );
            shutdown(&mut child);
            result.unwrap();
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "colm-password-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"
import os,sys,pty,subprocess,select,time,termios,pathlib
root=pathlib.Path(sys.argv[2]); master,slave=pty.openpty()
env=os.environ.copy();env['COLM_PASSWORD_TEST_DIR']=str(root)
p=subprocess.Popen([sys.argv[1],'--exact','terminal::tests::aborted_password_input_is_hidden_and_never_sent_as_chat','--nocapture'],stdin=slave,stdout=slave,stderr=slave,env=env)
output=b''
def until(marker):
 global output
 deadline=time.monotonic()+12
 while marker not in output:
  assert time.monotonic()<deadline, repr(output)
  ready,_,_=select.select([master],[],[],.1)
  if ready: output+=os.read(master,65536)
try:
 until(b'Session s');os.write(master,b'hello\n')
 until(b'Password for');assert not termios.tcgetattr(slave)[3]&termios.ECHO
 os.write(master,b'private-');(root/'abort').write_text('now')
 until(b'press Enter to clear');assert not termios.tcgetattr(slave)[3]&termios.ECHO
 os.write(master,b'password\n')
 deadline=time.monotonic()+3
 while not termios.tcgetattr(slave)[3]&termios.ECHO:
  assert time.monotonic()<deadline
  time.sleep(.02)
 os.write(master,b'/exit\n');assert p.wait(timeout=8)==0
 requests=(root/'requests').read_text()
 assert 'private-' not in requests and 'password' not in requests,requests
 assert b'private-' not in output and b'password\r\n' not in output,output
finally:
 if p.poll() is None: p.kill();p.wait()
 os.close(master);os.close(slave)
"#;
        let status = Command::new("python3")
            .args(["-c", script])
            .arg(std::env::current_exe().unwrap())
            .arg(&dir)
            .status()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(status.success());
    }
    #[test]
    fn default_model_uses_the_documented_deepseek_preset() {
        let config = Config::new().unwrap();
        assert_eq!(config.provider.model, "deepseek-flash");
        assert_eq!(config.provider.context_window, None);
        assert!(HELP.contains("--context-window"));
    }
    #[test]
    fn context_window_flag_is_bounded_and_persisted() {
        let mut config = Config::new().unwrap();
        let opts = options(&["--context-window", "200000"].map(String::from)).unwrap();
        apply(&mut config, &opts).unwrap();
        assert_eq!(config.provider.context_window, Some(200_000));
        // configure 消息经 JSON 交给 Agent 进程：窗口必须原样到达 ProviderConfig。
        let provider: crate::provider::ProviderConfig =
            serde_json::from_value(config.configure()["provider"].clone()).unwrap();
        assert_eq!(provider.context_window, Some(200_000));
        assert!(options(&["--context-window", "1024"].map(String::from)).is_ok());
        for invalid in ["0", "1023", "10000001", "many", "-1"] {
            let mut config = Config::new().unwrap();
            let opts = options(&["--context-window", invalid].map(String::from)).unwrap();
            let error = apply(&mut config, &opts).unwrap_err().to_string();
            assert!(
                error.contains("context window") || error.contains("context"),
                "{invalid}: {error}"
            );
            assert_eq!(
                config.provider.context_window, None,
                "an invalid flag must not be persisted: {invalid}"
            );
        }
        // 未知模型可以靠显式窗口使用：预算解析走用户值。
        let mut custom = Config::new().unwrap();
        let opts = options(
            &[
                "--base-url",
                "https://custom.example/v1",
                "--model",
                "local",
            ]
            .map(String::from),
        )
        .unwrap();
        apply(&mut custom, &opts).unwrap();
        assert!(custom.provider.effective_context_window().is_err());
        let opts = options(&["--context-window", "32768"].map(String::from)).unwrap();
        apply(&mut custom, &opts).unwrap();
        assert_eq!(
            custom.provider.effective_context_window().unwrap(),
            (32_768, crate::context::WindowSource::User)
        );
    }
    #[test]
    fn old_config_files_without_a_context_window_still_load() {
        let legacy = r#"{
            "provider": {"base_url": "https://api.deepseek.com", "model": "deepseek-flash"},
            "project_root": "."
        }"#;
        let config: Config = serde_json::from_str(legacy).unwrap();
        assert_eq!(config.provider.context_window, None);
        assert_eq!(
            config.provider.effective_context_window().unwrap(),
            (
                crate::context::CONTEXT_WINDOW_TOKENS,
                crate::context::WindowSource::Preset
            )
        );
    }
}
