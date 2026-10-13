use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

fn secret_key(key: &str) -> bool {
    let key = key
        .trim_start_matches('-')
        .replace('-', "_")
        .to_ascii_lowercase();
    matches!(
        key.as_str(),
        "password"
            | "passwd"
            | "passphrase"
            | "pwd"
            | "secret"
            | "token"
            | "api_key"
            | "apikey"
            | "authorization"
            | "cookie"
            | "credentials"
            | "credential"
            | "private_key"
            | "private_key_pem"
            | "access_key"
            | "secret_key"
            | "ssh_auth"
    ) || key.ends_with("_password")
        || key.ends_with("_token")
        || key.ends_with("_secret")
        || key.ends_with("_api_key")
        || key.ends_with("_access_key")
        || key.ends_with("_access_key_id")
}

/// Classify structured keys, never apply text replacement to scientific values.
/// Free-form prose must already be non-sensitive; do not pass process output here.
pub fn sanitize_detail(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter_map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    if secret_key(key)
                        || matches!(
                            normalized.as_str(),
                            "env"
                                | "environment"
                                | "environ"
                                | "stdout"
                                | "stderr"
                                | "log"
                                | "logs"
                                | "log_tail"
                                | "command"
                                | "argv"
                                | "shell_command"
                                | "error"
                        )
                    {
                        None
                    } else {
                        Some((key.clone(), sanitize_detail(value)))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(sanitize_detail).collect()),
        _ => value.clone(),
    }
}
impl Command {
    /// Redacted commands require the caller to restore credentials through its secure flow.
    pub fn is_redacted(&self) -> bool {
        self.argv.iter().any(|a| a.contains("[REDACTED"))
    }
    pub(crate) fn sanitize(&mut self) -> Result<()> {
        ensure!(
            !self.program.is_empty() && !self.program.contains('\0'),
            "command requires a valid program"
        );
        let shell = std::path::Path::new(&self.program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let curl = shell == "curl";
        let shell = matches!(
            shell,
            "sh" | "bash" | "zsh" | "fish" | "cmd" | "cmd.exe" | "powershell" | "pwsh"
        );
        let mut redact_next = false;
        for arg in &mut self.argv {
            ensure!(!arg.contains('\0'), "command argument contains NUL");
            if redact_next {
                *arg = "[REDACTED]".into();
                redact_next = false;
                continue;
            }
            if shell
                && matches!(
                    arg.as_str(),
                    "-c" | "-lc" | "-ic" | "/C" | "/c" | "-Command" | "-EncodedCommand"
                )
            {
                redact_next = true;
                continue;
            }
            if curl
                && matches!(
                    arg.as_str(),
                    "-u" | "--user" | "-U" | "--proxy-user" | "-H" | "--header" | "-b" | "--cookie"
                )
            {
                redact_next = true;
                continue;
            }
            if let Some((key, _)) = arg.split_once('=') {
                if secret_key(key) {
                    *arg = format!("{key}=[REDACTED]");
                    continue;
                }
            }
            if secret_key(arg) {
                redact_next = true;
                continue;
            }
            if arg.contains("://") {
                let tail = arg.split_once("://").map(|(_, v)| v).unwrap_or("");
                let authority = tail.split('/').next().unwrap_or("");
                let query_secret = tail.split_once('?').is_some_and(|(_, q)| {
                    q.split('&')
                        .any(|part| secret_key(part.split('=').next().unwrap_or("")))
                });
                if authority.contains('@') || query_secret {
                    *arg = "[REDACTED URL]".into();
                }
            }
        }
        Ok(())
    }
}
