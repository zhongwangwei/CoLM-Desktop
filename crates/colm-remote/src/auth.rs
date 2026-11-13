//! Process-local SSH credentials, received over stdin and served only to askpass.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

static AUTH: OnceLock<Auth> = OnceLock::new();
static GUI_RELAY: OnceLock<(String, String)> = OnceLock::new();
static GUI_TARGET_PASSWORD: Mutex<Option<(String, String)>> = Mutex::new(None);
static ANSWERED_SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());
const GUI_BROKER: &str = "COLM_SSH_GUI_BROKER";
const GUI_TOKEN: &str = "COLM_SSH_GUI_TOKEN";
const LIMIT: u64 = 64 * 1024;
const HUMAN_REPLY_TIMEOUT_SECS: u64 = 600;
const NETWORK_CONNECT_TIMEOUT_SECS: u64 = 60;
const BROKER: &str = "COLM_SSH_ASKPASS_BROKER";
const TOKEN: &str = "COLM_SSH_ASKPASS_TOKEN";

/// Nonsecret connection binding saved with a submitted job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub host: String,
    pub username: String,
    pub port: u16,
    pub auth: String,
    pub identity_file: String,
}

pub fn current_connection() -> Option<Connection> {
    AUTH.get().map(|a| Connection {
        host: a.host.clone(),
        username: a.username.clone(),
        port: a.port,
        auth: a.auth.clone(),
        identity_file: a.identity_file.clone(),
    })
}

/// Refuse to operate on a job using credentials for another connection.
pub fn check_connection(recorded: Option<&Connection>) -> Result<()> {
    if let Some(recorded) = recorded {
        if AUTH.get().is_none() {
            ensure!(!matches!(recorded.auth.as_str(), "password" | "key-password") || GUI_RELAY.get().is_some(), "this job requires an SSH password; enter it in the server connection settings again");
            let auth = Auth {
                host: recorded.host.clone(),
                username: recorded.username.clone(),
                port: recorded.port,
                auth: recorded.auth.clone(),
                identity_file: recorded.identity_file.clone(),
                password: None,
                broker: None,
            };
            auth.validate()?;
            AUTH.set(auth)
                .map_err(|_| anyhow::anyhow!("SSH authentication already initialized"))?;
        }
        ensure!(current_connection().as_ref() == Some(recorded), "saved job SSH settings differ; restore the job's connection settings before continuing");
    }
    Ok(())
}

// Deliberately neither Debug nor Serialize: credentials must not enter diagnostics.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Auth {
    host: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    port: u16,
    auth: String,
    #[serde(default)]
    identity_file: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(skip)]
    broker: Option<(String, String)>,
}

impl Auth {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.host.is_empty()
                && !self.host.starts_with('-')
                && !self
                    .host
                    .contains(|c: char| c.is_whitespace() || c.is_control()),
            "invalid SSH host"
        );
        ensure!(
            self.username.is_empty()
                || (!self.username.starts_with('-')
                    && !self
                        .username
                        .contains(|c: char| c.is_whitespace() || c.is_control() || c == '@')),
            "invalid SSH username"
        );
        ensure!(
            ["config", "key", "password", "key-password"].contains(&self.auth.as_str()),
            "invalid SSH authentication method"
        );
        ensure!(
            self.identity_file.is_empty()
                || (matches!(self.auth.as_str(), "key" | "key-password")
                    && Path::new(&self.identity_file).is_absolute()
                    && !self.identity_file.contains(char::is_control)),
            "SSH key must be an absolute local path in key mode"
        );
        if matches!(self.auth.as_str(), "password" | "key-password") {
            let password = self.password.as_deref().unwrap_or("");
            ensure!(
                (!password.is_empty() || GUI_RELAY.get().is_some())
                    && !password.contains(['\r', '\n', '\0']),
                "enter a nonempty single-line SSH password"
            );
        } else {
            ensure!(
                self.password.as_deref().unwrap_or("").is_empty(),
                "password supplied for non-password authentication"
            );
        }
        Ok(())
    }

    fn apply(&self, command: &mut Command) -> Result<()> {
        if self.auth != "config" || GUI_RELAY.get().is_some() {
            command.args(["-o", "StrictHostKeyChecking=yes"]);
        }
        if !self.username.is_empty() {
            command.args(["-l", &self.username]);
        }
        if self.port != 0 {
            command.args(["-p", &self.port.to_string()]);
        }
        let broker = GUI_RELAY.get().or(self.broker.as_ref());
        if let Some(broker) = broker {
            apply_broker(command, broker)?;
        } else {
            ensure!(
                !matches!(self.auth.as_str(), "password" | "key-password"),
                "SSH password broker is unavailable"
            );
            command.args(["-o", "BatchMode=yes"]);
        }
        if self.auth == "password" {
            command.args([
                "-o",
                "PreferredAuthentications=password",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "KbdInteractiveAuthentication=no",
            ]);
            if GUI_RELAY.get().is_none() {
                command.args(["-o", "NumberOfPasswordPrompts=1"]);
            }
        } else if matches!(self.auth.as_str(), "key" | "key-password") {
            let preferred = if self.auth == "key-password" {
                "PreferredAuthentications=publickey,password,keyboard-interactive"
            } else {
                "PreferredAuthentications=publickey"
            };
            command.args(["-o", preferred]);
            if !self.identity_file.is_empty() {
                command.args(["-i", &self.identity_file, "-o", "IdentitiesOnly=yes"]);
            }
        }
        // Explicit credentials must not reuse an already authenticated connection.
        if self.auth != "config" || !self.username.is_empty() || self.port != 0 {
            command.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
        }
        Ok(())
    }
}

fn parse(reader: impl Read) -> Result<Auth> {
    let mut line = String::new();
    BufReader::new(reader.take(LIMIT + 1))
        .read_line(&mut line)
        .context("cannot read SSH authentication")?;
    ensure!(
        line.len() as u64 <= LIMIT,
        "SSH authentication input is too large"
    );
    // JSON errors can include credential fragments, so discard parser diagnostics.
    let auth: Auth = serde_json::from_str(&line)
        .map_err(|_| anyhow::anyhow!("invalid SSH authentication input"))?;
    auth.validate()?;
    Ok(auth)
}

/// Called once by the CLI before dispatch; no credential is written to disk.
pub fn initialize_from_stdin() -> Result<()> {
    match (std::env::var_os(GUI_BROKER), std::env::var_os(GUI_TOKEN)) {
        (None, None) => {}
        (Some(address), Some(token)) => {
            let address = address
                .to_str()
                .context("invalid SSH credential endpoint")?;
            let token = token.to_str().context("invalid SSH credential token")?;
            validate_endpoint(address, token)?;
            GUI_RELAY
                .set(start_gui_relay(address.to_owned(), token.to_owned())?)
                .map_err(|_| anyhow::anyhow!("SSH GUI relay already initialized"))?;
        }
        _ => anyhow::bail!("incomplete SSH GUI credential endpoint"),
    }
    if std::env::var("COLM_SSH_AUTH_STDIN").as_deref() != Ok("1") {
        return Ok(());
    }
    let mut auth = parse(std::io::stdin().lock())?;
    if GUI_RELAY.get().is_some() {
        if let Some(password) = auth
            .password
            .as_ref()
            .filter(|password| !password.is_empty())
        {
            *GUI_TARGET_PASSWORD
                .lock()
                .unwrap_or_else(|e| e.into_inner()) =
                direct_password_prompt(&resolved_config(&auth)?)?
                    .map(|prompt| (prompt, password.clone()));
        }
    } else if matches!(auth.auth.as_str(), "password" | "key-password") {
        let prompt = password_prompt(&auth)?;
        auth.broker = Some(start_broker(auth.password.clone().unwrap(), prompt)?);
    }
    AUTH.set(auth)
        .map_err(|_| anyhow::anyhow!("SSH authentication already initialized"))
}

pub(crate) fn check_host(host: &str) -> Result<()> {
    if let Some(auth) = AUTH.get() {
        ensure!(auth.host == host, "saved job server differs from the selected SSH credentials; select the job's server first");
    }
    Ok(())
}

pub(crate) fn configure(command: &mut Command) -> Result<()> {
    // The outer SSH banner wait includes time spent answering a jump-host prompt.
    let timeout = NETWORK_CONNECT_TIMEOUT_SECS
        + if GUI_RELAY.get().is_some() {
            HUMAN_REPLY_TIMEOUT_SECS
        } else {
            0
        };
    command.args(["-o", &format!("ConnectTimeout={timeout}")]);
    command
        .env_remove("COLM_SSH_AUTH_STDIN")
        .env_remove(GUI_BROKER)
        .env_remove(GUI_TOKEN)
        .env_remove(BROKER)
        .env_remove(TOKEN)
        .env_remove("SSH_ASKPASS")
        .env("SSH_ASKPASS_REQUIRE", "never");
    if let Some(auth) = AUTH.get() {
        auth.apply(command)?;
    } else if let Some(broker) = GUI_RELAY.get() {
        command.args(["-o", "StrictHostKeyChecking=yes"]);
        apply_broker(command, broker)?;
    } else {
        command.args(["-o", "StrictHostKeyChecking=yes", "-o", "BatchMode=yes"]);
    }
    Ok(())
}

pub(crate) fn redact(text: &str) -> String {
    let mut secrets = ANSWERED_SECRETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if let Some(password) = AUTH.get().and_then(|auth| auth.password.as_ref()) {
        secrets.push(password.clone());
    }
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(text.to_owned(), |text, secret| {
            text.replace(secret, "[redacted]")
        })
}

fn apply_broker(command: &mut Command, (address, token): &(String, String)) -> Result<()> {
    command
        .args(["-o", "BatchMode=no"])
        .env("SSH_ASKPASS", std::env::current_exe()?)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(BROKER, address)
        .env(TOKEN, token)
        .env("LC_ALL", "C");
    Ok(())
}

fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    #[cfg(unix)]
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    #[cfg(windows)]
    {
        let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "$b = New-Object byte[] 32; $r = [Security.Cryptography.RandomNumberGenerator]::Create(); $r.GetBytes($b); [Console]::OpenStandardOutput().Write($b, 0, $b.Length); $r.Dispose()"]).output()?;
        ensure!(
            output.status.success() && output.stdout.len() == bytes.len(),
            "system random generator failed"
        );
        bytes.copy_from_slice(&output.stdout);
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("SSH password authentication is unsupported on this platform");
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn resolved_config(auth: &Auth) -> Result<String> {
    let mut command = Command::new("ssh");
    command
        .arg("-G")
        .env_remove(BROKER)
        .env_remove(TOKEN)
        .env_remove("COLM_SSH_AUTH_STDIN");
    if !auth.username.is_empty() {
        command.args(["-l", &auth.username]);
    }
    if auth.port != 0 {
        command.args(["-p", &auth.port.to_string()]);
    }
    let output = command
        .arg(&auth.host)
        .output()
        .context("cannot resolve SSH connection settings")?;
    ensure!(
        output.status.success(),
        "cannot resolve SSH connection settings"
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn password_prompt(auth: &Auth) -> Result<String> {
    prompt_from_config(&resolved_config(auth)?)
}

fn direct_password_prompt(config: &str) -> Result<Option<String>> {
    // A jump host can emit target-shaped prompts; never prefill across a proxy.
    if config.lines().any(|line| {
        ["proxyjump ", "proxycommand "].iter().any(|field| {
            line.strip_prefix(field).is_some_and(|value| {
                !value.trim().is_empty() && !value.trim().eq_ignore_ascii_case("none")
            })
        })
    }) {
        return Ok(None);
    }
    prompt_from_config(config).map(Some)
}

fn prompt_from_config(text: &str) -> Result<String> {
    let field = |name: &str| text.lines().find_map(|line| line.strip_prefix(name));
    let user = field("user ").context("SSH did not resolve a username")?;
    let host = field("hostkeyalias ")
        .filter(|alias| !alias.is_empty() && *alias != "none")
        .or_else(|| field("hostname "))
        .context("SSH did not resolve a hostname")?;
    Ok(format!("{user}@{host}'s password: "))
}

fn serve(mut stream: TcpStream, token: &str, password: &str, prompt: &str) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut supplied = [0u8; 64];
    stream.read_exact(&mut supplied)?;
    ensure!(
        supplied
            .iter()
            .zip(token.as_bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0,
        "invalid SSH credential request"
    );
    let mut received = String::new();
    BufReader::new((&mut stream).take(4096)).read_line(&mut received)?;
    ensure!(
        received.strip_suffix('\n') == Some(prompt),
        "unexpected SSH credential prompt"
    );
    stream.write_all(password.as_bytes())?;
    Ok(())
}

fn start_broker(password: String, prompt: String) -> Result<(String, String)> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?.to_string();
    let token = random_token()?;
    let expected = token.clone();
    std::thread::Builder::new()
        .name("ssh-askpass".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = serve(stream, &expected, &password, &prompt);
            }
        })?;
    Ok((address, token))
}

fn validate_prompt(prompt: &str) -> Result<()> {
    let lower = prompt.trim_end().to_ascii_lowercase();
    ensure!(
        !prompt.is_empty()
            && prompt.len() < 4096
            && !prompt.contains(char::is_control)
            && (lower.ends_with("password:")
                || (lower.starts_with("enter passphrase for key '") && lower.ends_with("':"))),
        "unsupported SSH prompt"
    );
    Ok(())
}

fn validate_endpoint(address: &str, token: &str) -> Result<std::net::SocketAddr> {
    let address: std::net::SocketAddr = address.parse()?;
    ensure!(
        address.ip().is_loopback(),
        "invalid SSH credential endpoint"
    );
    ensure!(
        token.len() == 64 && token.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid SSH credential token"
    );
    Ok(address)
}

fn request_secret(address: &str, token: &str, prompt: &str) -> Result<Vec<u8>> {
    validate_prompt(prompt)?;
    let address = validate_endpoint(address, token)?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(HUMAN_REPLY_TIMEOUT_SECS)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(token.as_bytes())?;
    writeln!(stream, "{prompt}")?;
    let mut secret = Vec::new();
    stream.take(LIMIT + 1).read_to_end(&mut secret)?;
    ensure!(
        !secret.is_empty()
            && secret.len() as u64 <= LIMIT
            && !secret.iter().any(|byte| matches!(byte, b'\r' | b'\n' | 0)),
        "SSH credential unavailable"
    );
    Ok(secret)
}

fn relay(mut stream: TcpStream, token: &str, gui_address: &str, gui_token: &str) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut supplied = [0u8; 64];
    stream.read_exact(&mut supplied)?;
    ensure!(
        supplied
            .iter()
            .zip(token.as_bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0,
        "invalid SSH credential request"
    );
    let mut received = String::new();
    BufReader::new((&mut stream).take(4096)).read_line(&mut received)?;
    let prompt = received
        .strip_suffix('\n')
        .context("invalid SSH credential prompt")?;
    validate_prompt(prompt)?;
    let supplied_password = {
        let mut password = GUI_TARGET_PASSWORD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if password
            .as_ref()
            .is_some_and(|(expected, _)| expected == prompt)
        {
            password.take().map(|(_, secret)| secret.into_bytes())
        } else {
            None
        }
    };
    let secret = match supplied_password {
        Some(secret) => secret,
        None => request_secret(gui_address, gui_token, prompt)?,
    };
    let secret_text = std::str::from_utf8(&secret).context("invalid SSH credential reply")?;
    ANSWERED_SECRETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(secret_text.to_owned());
    stream.write_all(&secret)?;
    Ok(())
}

fn start_gui_relay(gui_address: String, gui_token: String) -> Result<(String, String)> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?.to_string();
    let token = random_token()?;
    let expected = token.clone();
    std::thread::Builder::new()
        .name("ssh-gui-relay".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (token, address, gui_token) =
                    (expected.clone(), gui_address.clone(), gui_token.clone());
                let _ = std::thread::Builder::new()
                    .name("ssh-gui-request".into())
                    .spawn(move || {
                        let _ = relay(stream, &token, &address, &gui_token);
                    });
            }
        })?;
    Ok((address, token))
}

fn askpass() -> Result<()> {
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let secret = request_secret(&std::env::var(BROKER)?, &std::env::var(TOKEN)?, &prompt)?;
    std::io::stdout().write_all(&secret)?;
    Ok(())
}

/// Dispatch before argument parsing: ssh invokes the same packaged CLI as askpass.
pub fn run_askpass_if_requested() -> Option<i32> {
    std::env::var_os(BROKER).map(|_| if askpass().is_ok() { 0 } else { 1 })
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
