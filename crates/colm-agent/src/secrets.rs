//! API Key 存系统钥匙串（macOS 钥匙串、Windows 凭据管理器）。Key 只在 agent 进程里读出，
//! 不写进配置、日志、会话或工作区。Linux 第一版没有钥匙串，只读环境变量 `COLM_AGENT_API_KEY`。

use anyhow::Result;

const SERVICE: &str = "CoLM-Desktop AI assistant";

/// 钥匙串里的账户名：按服务地址区分（换服务商不会串 Key）。
fn account(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_ascii_lowercase()
}

/// 读 Key：环境变量优先（便于脚本与 CI），其次钥匙串。
pub fn get(base_url: &str) -> Result<Option<String>> {
    if let Ok(key) = std::env::var("COLM_AGENT_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(Some(key.trim().to_owned()));
        }
    }
    platform::get(&account(base_url))
}

pub fn set(base_url: &str, key: &str) -> Result<()> {
    platform::set(&account(base_url), key.trim())
}

pub fn delete(base_url: &str) -> Result<()> {
    platform::delete(&account(base_url))
}

#[cfg(any(target_os = "macos", windows))]
mod platform {
    use anyhow::{Context, Result};

    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(super::SERVICE, account).context("cannot open the system keychain")
    }

    pub fn get(account: &str) -> Result<Option<String>> {
        match entry(account)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("cannot read the API key from the system keychain"),
        }
    }

    pub fn set(account: &str, key: &str) -> Result<()> {
        entry(account)?
            .set_password(key)
            .context("cannot store the API key in the system keychain")
    }

    pub fn delete(account: &str) -> Result<()> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context("cannot delete the API key from the system keychain"),
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use anyhow::{bail, Result};

    pub fn get(_account: &str) -> Result<Option<String>> {
        Ok(None)
    }

    pub fn set(_account: &str, _key: &str) -> Result<()> {
        bail!("no system keychain on this platform; set COLM_AGENT_API_KEY instead")
    }

    pub fn delete(_account: &str) -> Result<()> {
        Ok(())
    }
}
