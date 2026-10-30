//! 天河 LLM 底线：用户明确要求“不能在天河上直接运行 DeepSeek 等 LLM”。
//!
//! 范围只针对**模型服务端点**（API base URL 与外部后端会读的环境变量），不涉及普通远程模拟、
//! 训练或结果传输。判定必须保守：
//! - 已知官方云服务的 host → allowed；
//! - host 或原始 URL 里出现天河明确标记 → blocked（宁可误拦，不能让天河 LLM 请求发出去）；
//! - 其余（自定义域名、内网 IP、localhost、未知域名）→ needs_confirmation，要求用户显式确认
//!   该端点不在天河上运行 LLM。
//!
//! 这个模块不读设置、不碰窗口状态，只做纯分类；保存设置与启动助手都会复用同一份规则，
//! 因此不存在只在界面里做、后端可绕过的“检查”。

use serde::Serialize;
use tauri::Url;

/// 已知官方云服务商的模型服务 host。精确匹配（大小写不敏感、忽略端口），不做通配子域：
/// 猜“看起来像官方”的子域会把自定义代理误判成 allowed，而底线宁可多确认一次。
pub const KNOWN_PUBLIC_HOSTS: [&str; 8] = [
    "api.deepseek.com",
    "api.openai.com",
    "api.anthropic.com",
    "api.x.ai",
    "open.bigmodel.cn",
    "generativelanguage.googleapis.com",
    "api.moonshot.cn",
    "dashscope.aliyuncs.com",
];

/// 天河相关的保守标记。`th0`/`th1` 故意不收进列表：它们会误中 `auth0`、`health1` 这类
/// 无关域名；用户实际常用的入口是 `th2`/`th-` 前缀与 `tianhe`/`天河` 字样。
pub const TIANHE_MARKERS: [&str; 4] = ["tianhe", "天河", "th2", "th-"];

/// 外部后端（Codex / Claude Code / OpenCode 及其工具）可能读取的模型服务端点环境变量。
pub const EXTERNAL_ENDPOINT_ENV: [&str; 4] = [
    "ANTHROPIC_BASE_URL",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE",
    "OLLAMA_HOST",
];

/// 端点判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 已知官方云服务，允许保存/启动。
    Allowed,
    /// 无法确认不在天河：必须由用户显式确认（设置里的确认项）后才能保存/启动。
    NeedsConfirmation,
    /// 检测到天河标记：硬拒绝，不允许保存/启动，也不允许用确认项绕过。
    Blocked,
}

/// 一次端点分类的结果，原样返回给前端（`assistant_llm_endpoint_guard`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Guard {
    pub verdict: Verdict,
    /// 规范化后的 host（小写，不含端口）；解析失败时为空。
    pub host: String,
    /// host 是否在官方云服务预设里。
    pub known_public: bool,
    /// host 或原始 URL 是否命中天河标记。
    pub matched_tianhe: bool,
    /// 面向用户的中文说明（前端原样显示）。
    pub message: String,
}

/// 外部后端的端点提示：来自环境变量，错误信息只暴露变量名与 host，不回显完整取值
/// （取值可能含代理凭据或查询串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalHint {
    pub variable: String,
    pub host: Option<String>,
    pub verdict: Verdict,
    pub message: String,
}

fn parse_endpoint(base_url: &str) -> Result<Url, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("模型服务地址是空的".into());
    }
    let url = Url::parse(trimmed).map_err(|_| "模型服务地址不是有效的网址".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("模型服务端点必须是 http:// 或 https://".into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("模型服务地址缺少主机名".into());
    }
    Ok(url)
}

/// 端点规范形式，用于和用户确认值做 exact match：scheme/host 小写、忽略默认端口、去掉尾部 `/`。
pub fn normalize_endpoint(base_url: &str) -> Result<String, String> {
    let url = parse_endpoint(base_url)?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let mut normalized = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        normalized.push_str(&format!(":{port}"));
    }
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        normalized.push_str(path);
    }
    Ok(normalized)
}

/// 分类一个模型服务端点。URL 不合法时返回 Err（调用方据此直接拒绝）。
pub fn classify_llm_endpoint(base_url: &str) -> Result<Guard, String> {
    let url = parse_endpoint(base_url)?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let raw_lower = base_url.to_ascii_lowercase();
    let matched_tianhe = TIANHE_MARKERS
        .iter()
        .any(|marker| host.contains(marker) || raw_lower.contains(marker));
    if matched_tianhe {
        return Ok(Guard {
            verdict: Verdict::Blocked,
            host: host.clone(),
            known_public: false,
            matched_tianhe: true,
            message: format!(
                "检测到模型服务端点疑似位于天河（{host}）。软件底线：禁止在天河上直接运行 DeepSeek 等 LLM；请改用官方云服务或其他机器。"
            ),
        });
    }
    let known_public = KNOWN_PUBLIC_HOSTS.contains(&host.as_str());
    if known_public {
        return Ok(Guard {
            verdict: Verdict::Allowed,
            host,
            known_public: true,
            matched_tianhe: false,
            message: "这是已知的官方云模型服务端点，可以使用。".into(),
        });
    }
    Ok(Guard {
        verdict: Verdict::NeedsConfirmation,
        host: host.clone(),
        known_public: false,
        matched_tianhe: false,
        message: format!(
            "模型服务端点 {host} 不是已知的官方云服务。请确认它不在天河上运行 DeepSeek 等 LLM（本软件禁止在天河上直接运行）；确认后才能保存或启动助手。"
        ),
    })
}

/// 读取当前进程环境里可能决定外部后端模型端点的变量并分类。只返回实际存在的变量。
pub fn external_endpoint_hints() -> Vec<ExternalHint> {
    external_endpoint_hints_with(|name| std::env::var(name).ok())
}

/// 供测试注入的版本（不依赖进程环境）。
pub fn external_endpoint_hints_with(read: impl Fn(&str) -> Option<String>) -> Vec<ExternalHint> {
    EXTERNAL_ENDPOINT_ENV
        .iter()
        .filter_map(|variable| {
            read(variable)
                .filter(|value| !value.trim().is_empty())
                .map(|value| (variable, value))
        })
        .map(|(variable, value)| classify_hint(variable, &value))
        .collect()
}

fn classify_hint(variable: &str, value: &str) -> ExternalHint {
    // 先按完整 URL 分类；OLLAMA_HOST 这类变量常写成 host:port，再补一次 http:// 兜底。
    let classified = classify_llm_endpoint(value)
        .or_else(|_| classify_llm_endpoint(&format!("http://{}", value.trim())))
        .or_else(|_| classify_llm_endpoint(&format!("http://{}/", value.trim())));
    match classified {
        Ok(guard) => ExternalHint {
            variable: variable.to_owned(),
            host: Some(guard.host),
            verdict: guard.verdict,
            message: guard.message,
        },
        Err(_) => {
            // 解析不了的取值不能当成“没问题”：原始串里出现天河标记就按 blocked 处理；
            // 否则要求用户确认（外部后端启动前还需要 external_llm_ack）。
            let lower = value.to_ascii_lowercase();
            let matched_tianhe = TIANHE_MARKERS.iter().any(|marker| lower.contains(marker));
            ExternalHint {
                variable: variable.to_owned(),
                host: None,
                verdict: if matched_tianhe {
                    Verdict::Blocked
                } else {
                    Verdict::NeedsConfirmation
                },
                message: if matched_tianhe {
                    format!(
                        "环境变量 {variable} 的模型服务端点无法解析，但地址中出现天河标记；软件底线禁止在天河上直接运行 DeepSeek 等 LLM。"
                    )
                } else {
                    format!(
                        "环境变量 {variable} 的模型服务端点无法解析，无法确认是否在天河上；请清除它或改成官方云服务地址。"
                    )
                },
            }
        }
    }
}

#[cfg(test)]
// 仓库惯例是 `<模块>_tests.rs`；本任务 write_scopes 只给了 llm_guard.rs，测试因此内联在本文件。
mod tests {
    use super::*;

    fn blocked(url: &str) -> Guard {
        let guard = classify_llm_endpoint(url).unwrap();
        assert_eq!(guard.verdict, Verdict::Blocked, "{url}: {guard:?}");
        assert!(guard.matched_tianhe, "{url}");
        assert!(
            guard.message.contains("禁止在天河"),
            "{url}: {}",
            guard.message
        );
        guard
    }

    #[test]
    fn official_cloud_hosts_are_allowed_case_insensitively_and_ignore_ports() {
        for (url, host) in [
            ("https://api.deepseek.com/v1", "api.deepseek.com"),
            ("https://api.openai.com/v1/", "api.openai.com"),
            ("https://API.Anthropic.com/v1", "api.anthropic.com"),
            ("https://api.x.ai/v1", "api.x.ai"),
            ("https://open.bigmodel.cn/api/paas/v4", "open.bigmodel.cn"),
            (
                "https://generativelanguage.googleapis.com/v1beta",
                "generativelanguage.googleapis.com",
            ),
            ("https://api.moonshot.cn/v1", "api.moonshot.cn"),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "dashscope.aliyuncs.com",
            ),
            ("https://api.deepseek.com:8443/v1", "api.deepseek.com"),
        ] {
            let guard = classify_llm_endpoint(url).unwrap();
            assert_eq!(guard.verdict, Verdict::Allowed, "{url}: {guard:?}");
            assert_eq!(guard.host, host, "{url}");
            assert!(guard.known_public, "{url}");
            assert!(!guard.matched_tianhe, "{url}");
        }
    }

    #[test]
    fn tianhe_markers_are_blocked_even_in_subdomains_paths_or_case() {
        for url in [
            "https://tianhe.example.com/v1",
            "https://TH2.example.com/v1",
            "https://api.th-ai.cn/v1",
            "https://llm.th2.internal/v1",
            "https://天河.example.com/v1",
            "https://proxy.example.com/tianhe/v1",
        ] {
            blocked(url);
        }
        // 官方云域名出现在原始 URL 里也不能盖过天河标记。
        blocked("https://api.deepseek.com/tianhe-proxy/v1");
    }

    #[test]
    fn custom_loopback_and_unknown_hosts_need_confirmation() {
        for (url, host) in [
            ("https://api.example.com/v1", "api.example.com"),
            ("http://127.0.0.1:11434/v1", "127.0.0.1"),
            ("http://localhost:8000", "localhost"),
            ("https://notdeepseek.com/v1", "notdeepseek.com"),
            ("http://192.168.1.9:8080/v1", "192.168.1.9"),
        ] {
            let guard = classify_llm_endpoint(url).unwrap();
            assert_eq!(
                guard.verdict,
                Verdict::NeedsConfirmation,
                "{url}: {guard:?}"
            );
            assert_eq!(guard.host, host, "{url}");
            assert!(!guard.known_public, "{url}");
            assert!(!guard.matched_tianhe, "{url}");
            assert!(guard.message.contains("确认"), "{url}: {}", guard.message);
        }
        // 精确匹配：官方 host 的子域不自动放行。
        let guard = classify_llm_endpoint("https://evil.api.deepseek.com/v1").unwrap();
        assert_eq!(guard.verdict, Verdict::NeedsConfirmation);
    }

    #[test]
    fn malformed_endpoints_are_errors() {
        for url in [
            "",
            "   ",
            "ftp://api.deepseek.com/v1",
            "api.deepseek.com/v1",
            "https://",
            "https://exa mple.com/v1",
            "http://:11434/v1",
        ] {
            assert!(classify_llm_endpoint(url).is_err(), "{url:?}");
            assert!(normalize_endpoint(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn normalized_endpoints_are_stable_for_ack_comparison() {
        assert_eq!(
            normalize_endpoint(" HTTPS://API.DeepSeek.COM/v1/ ").unwrap(),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com").unwrap(),
            "https://api.deepseek.com"
        );
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com/").unwrap(),
            "https://api.deepseek.com"
        );
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com:8443/v1").unwrap(),
            "https://api.deepseek.com:8443/v1"
        );
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:11434/v1/").unwrap(),
            "http://127.0.0.1:11434/v1"
        );
        // IPv6 host 与前端 URL.host 的写法一致（带方括号）。
        assert_eq!(
            normalize_endpoint("http://[::1]:11434/v1/").unwrap(),
            "http://[::1]:11434/v1"
        );
    }

    #[test]
    fn external_endpoint_hints_are_classified_without_echoing_values() {
        let hints = external_endpoint_hints_with(|name| match name {
            "ANTHROPIC_BASE_URL" => Some("https://api.anthropic.com".into()),
            "OPENAI_BASE_URL" => Some("https://tianhe.example.com/v1?key=secret".into()),
            "OLLAMA_HOST" => Some("127.0.0.1:11434".into()),
            "OPENAI_API_BASE" => Some("   ".into()),
            _ => None,
        });
        assert_eq!(hints.len(), 3, "{hints:?}");
        let anthropic = &hints[0];
        assert_eq!(anthropic.variable, "ANTHROPIC_BASE_URL");
        assert_eq!(anthropic.verdict, Verdict::Allowed);
        assert_eq!(anthropic.host.as_deref(), Some("api.anthropic.com"));
        let openai = &hints[1];
        assert_eq!(openai.verdict, Verdict::Blocked);
        assert_eq!(openai.host.as_deref(), Some("tianhe.example.com"));
        // 错误/提示信息不回显可能含凭据的完整取值。
        assert!(!openai.message.contains("secret"), "{openai:?}");
        // host:port 形式（OLLAMA_HOST）按 http 补全后分类。
        let ollama = &hints[2];
        assert_eq!(ollama.variable, "OLLAMA_HOST");
        assert_eq!(ollama.verdict, Verdict::NeedsConfirmation);
        assert_eq!(ollama.host.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn unparseable_external_hints_with_tianhe_markers_are_still_blocked() {
        let hints = external_endpoint_hints_with(|name| match name {
            "OPENAI_BASE_URL" => Some("not a url tianhe ???".into()),
            "OPENAI_API_BASE" => Some("::bad::".into()),
            _ => None,
        });
        assert_eq!(hints.len(), 2, "{hints:?}");
        assert_eq!(hints[0].verdict, Verdict::Blocked);
        assert!(hints[0].message.contains("天河"), "{hints:?}");
        assert_eq!(hints[1].verdict, Verdict::NeedsConfirmation);
    }
}
