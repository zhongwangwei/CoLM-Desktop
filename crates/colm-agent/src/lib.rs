//! CoLM-Desktop 的 AI 助手（docs/design-ai-assistant.md）。
//!
//! 内置后端是 OpenAI 兼容的流式对话循环（默认 DeepSeek 官方 API）；工具注册表分级审批，
//! 同一份注册表以后也经 MCP 服务提供给外部编码工具。GUI 经 stdio JSONL（[`protocol`]）驱动它。

pub mod agent;
pub mod message;
pub mod protocol;
pub mod provider;
pub mod secrets;
pub mod session;
pub mod tools;

/// 内置后端的系统提示。
pub const SYSTEM_PROMPT: &str = include_str!("prompt.md");
