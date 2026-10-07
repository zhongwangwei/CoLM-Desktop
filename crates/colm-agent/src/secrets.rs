//! API Key 存在本地文件里（应用配置目录的 `assistant-keys.json`，按服务地址分开），三个平台一样。
//! 不用系统钥匙串：macOS 上每换一个（或重新编译的）程序读钥匙串都会弹授权框，用户要的是简单可移植。
//!
//! Key 以明文保存；Unix 上文件权限设为 0600（只有当前用户可读写），Windows 上用户配置目录本身
//! 只对该用户开放。Key 只在 agent 进程里读出，不进日志、会话或工作区。环境变量
//! `COLM_AGENT_API_KEY` 优先（脚本与 CI 用）。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

/// 文件里的键：去掉首尾空白与结尾斜杠、转小写的服务地址。
pub fn normalized(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn load(path: &Path) -> Result<BTreeMap<String, String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not a valid key file", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn save(path: &Path, keys: &BTreeMap<String, String>) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    // 先写临时文件、设好权限再换名，任何时刻都不会有一个所有人可读的 Key 文件。
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_string_pretty(keys)?)
        .with_context(|| format!("cannot write {}", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path).with_context(|| format!("cannot write {}", path.display()))
}

/// 读 Key：环境变量优先，其次本地文件。
pub fn get(path: &Path, base_url: &str) -> Result<Option<String>> {
    if let Ok(key) = std::env::var("COLM_AGENT_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(Some(key.trim().to_owned()));
        }
    }
    Ok(load(path)?
        .remove(&normalized(base_url))
        .filter(|key| !key.is_empty()))
}

/// 这个服务存过 Key 吗（不读出 Key 本身）。
pub fn has(path: &Path, base_url: &str) -> Result<bool> {
    Ok(load(path)?.contains_key(&normalized(base_url)))
}

pub fn set(path: &Path, base_url: &str, key: &str) -> Result<()> {
    let mut keys = load(path)?;
    keys.insert(normalized(base_url), key.trim().to_owned());
    save(path, &keys)
}

pub fn delete(path: &Path, base_url: &str) -> Result<()> {
    let mut keys = load(path)?;
    if keys.remove(&normalized(base_url)).is_some() {
        save(path, &keys)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod secrets_tests;
