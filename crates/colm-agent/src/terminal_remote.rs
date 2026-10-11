//! Headless P3 bridge. Server settings come only from the user's explicit file.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub struct TerminalRemote {
    servers: Vec<Value>,
    cli: PathBuf,
    root: Option<PathBuf>,
}

impl TerminalRemote {
    pub fn new(config_path: PathBuf, cli: PathBuf) -> Result<Self> {
        let value: Value = serde_json::from_slice(&std::fs::read(config_path)?)
            .context("invalid server configuration JSON")?;
        reject_secrets(&value)?;
        let servers = value["servers"]
            .as_array()
            .context("server configuration needs a servers array")?
            .clone();
        let mut hosts = std::collections::HashSet::new();
        for server in &servers {
            validate_server(server)?;
            ensure!(
                hosts.insert(field(server, "host")?),
                "duplicate server host"
            );
        }
        Ok(Self {
            servers,
            cli,
            root: None,
        })
    }

    pub fn with_workspace_root(mut self, root: PathBuf) -> Self {
        self.root = Some(root);
        self
    }

    pub fn handle(
        &self,
        action: &str,
        args: Value,
        password: impl FnMut(&str) -> Result<String>,
    ) -> Result<Value> {
        self.handle_with_cancel(action, args, password, &AtomicBool::new(false))
    }

    pub fn handle_with_cancel(
        &self,
        action: &str,
        args: Value,
        mut password: impl FnMut(&str) -> Result<String>,
        cancel: &AtomicBool,
    ) -> Result<Value> {
        check_cancel(cancel)?;
        if action == "workspace_remote_servers" {
            return Ok(json!({"servers":self.servers.iter().map(|s| json!({
                "host":s["host"], "root":s["root"],
                "scheduler":s.get("scheduler").cloned().unwrap_or(json!("auto")),
                "cpus":s.get("cpus").cloned().unwrap_or(json!(0)),
                "memory_gb":s.get("memory_gb").cloned().unwrap_or(json!(0))
            })).collect::<Vec<_>>()}));
        }
        ensure!(
            action == "workspace_remote",
            "unsupported terminal bridge action"
        );
        let name = field(&args, "name")?;
        let operation = field(&args, "operation")?;
        if operation != "knowledge" || !name.is_empty() {
            identifier(name)?;
        }
        ensure!(
            ["list", "submit", "status", "cancel", "fetch", "knowledge"].contains(&operation),
            "unsupported remote operation"
        );
        let mut argv = self.args(name, operation);
        if operation == "list" {
            return self.run(argv, None, cancel);
        }
        let server = if operation == "submit" || operation == "knowledge" {
            let host = field(&args, "host")?;
            let server = self
                .servers
                .iter()
                .find(|s| s["host"] == host)
                .context("server is not configured")?
                .clone();
            let request = &args["request"];
            ensure!(
                (operation == "knowledge"
                    && request.is_object()
                    && request.to_string().len() <= 16384)
                    || matches!(
                        request["action"].as_str(),
                        Some(
                            "build-engine"
                                | "build-kernel"
                                | "test"
                                | "run"
                                | "parity"
                                | "regress"
                                | "verify"
                        )
                    ),
                "unsupported remote task"
            );
            argv.extend([
                "--request".into(),
                request.to_string(),
                "--host".into(),
                host.into(),
                "--remote-root".into(),
                field(&server, "root")?.into(),
            ]);
            argv.extend([
                "--scheduler".into(),
                server["scheduler"].as_str().unwrap_or("auto").into(),
            ]);
            for (key, flag) in [
                ("partition", "--partition"),
                ("account", "--account"),
                ("walltime", "--walltime"),
                ("env_script", "--env-script"),
            ] {
                if let Some(value) = server[key].as_str().filter(|s| !s.trim().is_empty()) {
                    argv.extend([flag.into(), value.into()]);
                }
            }
            for (key, flag) in [("cpus", "--cpus"), ("memory_gb", "--mem-gb")] {
                if let Some(value) = server[key].as_u64().filter(|n| *n > 0) {
                    argv.extend([flag.into(), value.to_string()]);
                }
            }
            if let Some(directives) = server["directives"].as_array() {
                for directive in directives {
                    argv.extend([
                        "--directive".into(),
                        directive.as_str().context("directive must be text")?.into(),
                    ]);
                }
            }
            server
        } else {
            let job = field(&args, "job")?;
            identifier(job)?;
            let records = self.run(self.args(name, "list"), None, cancel)?;
            let record = records["jobs"]
                .as_array()
                .and_then(|jobs| jobs.iter().find(|r| r["id"] == job))
                .context("saved job not found")?;
            // Preserve the submitted connection even if settings have subsequently changed.
            let mut server = if record["connection"].is_object() {
                record["connection"].clone()
            } else {
                json!({"host":record["host"],"auth":"config"})
            };
            ensure!(
                server["host"] == record["host"],
                "job connection host mismatch"
            );
            server["root"] = record["root"].clone();
            validate_server(&server)?;
            argv.extend(["--job".into(), job.into()]);
            server
        };
        let auth = server["auth"].as_str().unwrap_or("config");
        let secret = if auth == "password" {
            let label = format!(
                "{} @ {} (port {})",
                server["username"].as_str().unwrap_or("SSH user"),
                field(&server, "host")?,
                server["port"].as_u64().unwrap_or(0)
            );
            check_cancel(cancel)?;
            let secret = password(&label)?;
            check_cancel(cancel)?;
            ensure!(
                !secret.is_empty() && secret.len() <= 4096 && !secret.contains(['\n', '\r', '\0']),
                "invalid SSH password"
            );
            Some(secret)
        } else {
            None
        };
        self.run(argv,Some(json!({"host":server["host"],"username":server["username"].as_str().unwrap_or(""),"port":server["port"].as_u64().unwrap_or(0),"auth":auth,"identity_file":server["identity_file"].as_str().unwrap_or(""),"password":secret})),cancel)
    }

    fn args(&self, name: &str, operation: &str) -> Vec<String> {
        let mut args = vec![
            "remote-workspace".into(),
            "--name".into(),
            name.into(),
            "--operation".into(),
            operation.into(),
        ];
        if let Some(root) = &self.root {
            args.extend(["--root".into(), root.to_string_lossy().into_owned()]);
        }
        args
    }

    fn run(&self, args: Vec<String>, auth: Option<Value>, cancel: &AtomicBool) -> Result<Value> {
        check_cancel(cancel)?;
        let mut command = Command::new(&self.cli);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .env_remove("COLM_SSH_AUTH_STDIN");
        if auth.is_some() {
            command
                .stdin(Stdio::piped())
                .env("COLM_SSH_AUTH_STDIN", "1");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        check_cancel(cancel)?;
        let mut child = command.spawn().context("cannot start colm-cli")?;
        let (tx, rx) = mpsc::channel();
        drain(child.stdout.take().expect("piped stdout"), 0, tx.clone());
        drain(child.stderr.take().expect("piped stderr"), 1, tx.clone());
        if let Some(auth) = &auth {
            let mut stdin = child.stdin.take().expect("piped stdin");
            let payload = auth.to_string();
            std::thread::spawn(move || {
                let result = writeln!(stdin, "{payload}").map(|_| Vec::new());
                let _ = tx.send((2, result));
            });
        }
        let mut streams = [None, None];
        let mut written = auth.is_none();
        let deadline = Instant::now() + Duration::from_secs(1800);
        let status = loop {
            if cancel.load(Ordering::SeqCst) || Instant::now() >= deadline {
                stop(&mut child);
                bail!("remote operation interrupted; inspect saved jobs before retrying; an already submitted server job may still be running");
            }
            while let Ok((stream, result)) = rx.try_recv() {
                match result {
                    Ok(bytes) if stream < 2 => streams[stream] = Some(bytes),
                    Ok(_) => written = true,
                    Err(_) => {
                        stop(&mut child);
                        bail!("cannot exchange data with colm-cli");
                    }
                }
            }
            // Keep the child unreaped until pipes close, preserving group ownership on cancellation.
            if written && streams.iter().all(Option::is_some) {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => {}
                    Err(error) => {
                        stop(&mut child);
                        return Err(error.into());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let stdout = streams[0].take().unwrap();
        let stderr = streams[1].take().unwrap();
        let secret = auth.as_ref().and_then(|a| a["password"].as_str());
        if !status.success() {
            let mut error = String::from_utf8_lossy(&stderr).into_owned();
            if let Some(secret) = secret {
                let escaped = serde_json::to_string(secret)?;
                error = error
                    .replace(&escaped[1..escaped.len() - 1], "[redacted]")
                    .replace(secret, "[redacted]");
            }
            bail!("colm-cli failed: {error}");
        }
        let stdout = String::from_utf8_lossy(&stdout);
        let mut result: Value = serde_json::from_str(
            stdout
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .context("empty CLI response")?,
        )
        .context("invalid CLI JSON response")?;
        if let Some(secret) = secret {
            redact_value(&mut result, secret);
        }
        Ok(result)
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "remote operation cancelled");
    Ok(())
}

fn drain(
    reader: impl Read + Send + 'static,
    stream: usize,
    tx: mpsc::Sender<(usize, std::io::Result<Vec<u8>>)>,
) {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .and_then(|_| {
                if bytes.len() > 64 * 1024 * 1024 {
                    Err(std::io::Error::other("CLI output exceeds limit"))
                } else {
                    Ok(bytes)
                }
            });
        let _ = tx.send((stream, result));
    });
}

fn stop(child: &mut Child) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let _ = Command::new("kill")
            .args(["-KILL", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn redact_value(value: &mut Value, secret: &str) {
    match value {
        Value::String(text) => *text = text.replace(secret, "[redacted]"),
        Value::Array(values) => {
            for value in values {
                redact_value(value, secret);
            }
        }
        Value::Object(values) => {
            let old = std::mem::take(values);
            for (key, mut value) in old {
                redact_value(&mut value, secret);
                values.insert(key.replace(secret, "[redacted]"), value);
            }
        }
        _ => {}
    }
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("missing {key}"))
}
fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "invalid workspace name or job ID"
    );
    Ok(())
}
fn validate_server(server: &Value) -> Result<()> {
    let host = field(server, "host")?;
    ensure!(
        !host.is_empty()
            && !host.starts_with('-')
            && !host.contains(|c: char| c.is_whitespace() || c.is_control()),
        "invalid server host"
    );
    let root = field(server, "root")?;
    ensure!(
        root.starts_with('/')
            && root != "/"
            && !root.contains(|c: char| c.is_whitespace() || c.is_control()),
        "invalid server root"
    );
    ensure!(
        ["config", "key", "password"].contains(&server["auth"].as_str().unwrap_or("config")),
        "invalid SSH authentication mode"
    );
    if let Some(port) = server.get("port") {
        ensure!(
            port.as_u64().is_some_and(|p| p <= 65535),
            "invalid SSH port"
        );
    }
    if let Some(user) = server.get("username") {
        let user = user.as_str().context("invalid SSH username")?;
        ensure!(
            !user.starts_with('-')
                && user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
            "invalid SSH username"
        );
    }
    if let Some(key) = server.get("identity_file") {
        let key = key.as_str().context("invalid SSH key path")?;
        ensure!(
            key.is_empty() || (PathBuf::from(key).is_absolute() && !key.contains(char::is_control)),
            "invalid SSH key path"
        );
    }
    Ok(())
}
fn reject_secrets(value: &Value) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                ensure!(
                    !matches!(
                        key.to_lowercase().as_str(),
                        "password" | "api_key" | "token" | "private_key"
                    ),
                    "server configuration must not contain secrets"
                );
                reject_secrets(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_secrets(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
#[path = "terminal_remote_tests.rs"]
mod tests;
