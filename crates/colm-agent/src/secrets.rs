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
    let value = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    if let Ok(uri) = value.parse::<ureq::http::Uri>() {
        if let (Some(scheme), Some(authority), Some(port)) =
            (uri.scheme_str(), uri.authority(), uri.port_u16())
        {
            if (scheme == "https" && port == 443) || (scheme == "http" && port == 80) {
                let host = authority.as_str().trim_end_matches(&format!(":{port}"));
                return format!(
                    "{scheme}://{host}{}",
                    uri.path_and_query().map_or("", |p| p.as_str())
                )
                .trim_end_matches('/')
                .to_owned();
            }
        }
    }
    value
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
    // 先建 0600 的临时文件再写内容、换名，任何时刻都不会出现所有人可读的 Key（B-09）。
    let temp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .with_context(|| format!("cannot create {}", temp.display()))?;
    {
        use std::io::Write;
        file.write_all(serde_json::to_string_pretty(keys)?.as_bytes())
            .with_context(|| format!("cannot write {}", temp.display()))?;
        file.sync_all()?;
    }
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path).with_context(|| format!("cannot write {}", path.display()))
}

fn stored_key(keys: &BTreeMap<String, String>, base_url: &str) -> Option<String> {
    let endpoint = normalized(base_url);
    keys.get(&endpoint)
        .or_else(|| {
            keys.iter()
                .find(|(url, _)| normalized(url) == endpoint)
                .map(|(_, key)| key)
        })
        .cloned()
}

/// 读 Key：环境变量优先，其次本地文件。
pub fn get(path: &Path, base_url: &str) -> Result<Option<String>> {
    if let Ok(key) = std::env::var("COLM_AGENT_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(Some(key.trim().to_owned()));
        }
    }
    Ok(stored_key(&load(path)?, base_url).filter(|key| !key.is_empty()))
}

/// 这个服务存过 Key 吗（不读出 Key 本身）。
pub fn has(path: &Path, base_url: &str) -> Result<bool> {
    Ok(get_stored(path, base_url)?.is_some())
}

/// 只读本地 Key 文件里的这个服务，不套用 `COLM_AGENT_API_KEY` 环境变量。
/// 本机 Xerj 的 key 与模型服务是两回事：环境变量是给某次会话覆盖模型 Key 用的，
/// 拿它去连文档检索节点会把模型 Key 发给另一个本机服务。
pub fn get_stored(path: &Path, base_url: &str) -> Result<Option<String>> {
    Ok(stored_key(&load(path)?, base_url).filter(|key| !key.is_empty()))
}

// Lock a stable sibling inode: the key file itself is replaced atomically.
fn lock_keys(path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .context("key file has no filename")?
        .to_string_lossy();
    colm_case::file_lock::exclusive(&path.with_file_name(format!("{name}.lock")))
}

pub fn set(path: &Path, base_url: &str, key: &str) -> Result<()> {
    let _lock = lock_keys(path)?;
    let mut keys = load(path)?;
    let endpoint = normalized(base_url);
    keys.retain(|url, _| normalized(url) != endpoint);
    keys.insert(endpoint, key.trim().to_owned());
    save(path, &keys)
}

pub fn delete(path: &Path, base_url: &str) -> Result<()> {
    let _lock = lock_keys(path)?;
    let mut keys = load(path)?;
    let endpoint = normalized(base_url);
    let before = keys.len();
    keys.retain(|url, _| normalized(url) != endpoint);
    if keys.len() != before {
        save(path, &keys)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod secrets_tests;
