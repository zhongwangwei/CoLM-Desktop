//! Streaming Chat Completions, OpenAI Responses and Anthropic Messages transports.
//!
//! 只依赖通用格式：服务地址、模型名与 Key 都可配置，所以 Qwen、本地 Ollama/vLLM 等兼容服务也能用。
//! DeepSeek 专有的部分（`thinking` 参数、回传 `reasoning_content`）按配置开关，见
//! docs/design-ai-assistant.md 第 3.1 节。

use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::message::{Message, ToolCall, Usage};

/// DeepSeek 官方服务地址（OpenAI 兼容）。
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

/// 模型服务的配置。Key 不在这里持久化，由调用方从本地 Key 文件取出后填入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub api_format: ApiFormat,
    #[serde(default = "empty_options")]
    pub api_options: Value,
    #[serde(default = "default_max_tokens")]
    pub max_output_tokens: u32,
    pub base_url: String,
    pub model: String,
    #[serde(skip)]
    pub api_key: String,
    /// Provider-specific thinking switch; None leaves the service default.
    #[serde(default)]
    pub thinking: Option<bool>,
    /// Provider-specific effort; native adapters map it to their own request fields.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 工具 schema 带 `strict: true`（DeepSeek 要配 `/beta` 地址）。
    #[serde(default)]
    pub strict: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    #[default]
    ChatCompletions,
    Responses,
    Anthropic,
}
fn empty_options() -> Value {
    json!({})
}
fn default_max_tokens() -> u32 {
    16384
}

fn default_timeout() -> u64 {
    600
}

impl ProviderConfig {
    /// DeepSeek 官方 API 的缺省配置。
    pub fn deepseek(model: &str, api_key: String) -> Self {
        Self {
            provider_id: "deepseek".into(),
            api_format: ApiFormat::ChatCompletions,
            api_options: empty_options(),
            max_output_tokens: default_max_tokens(),
            base_url: DEEPSEEK_BASE_URL.into(),
            model: model.into(),
            api_key,
            thinking: None,
            reasoning_effort: None,
            strict: false,
            timeout_seconds: default_timeout(),
        }
    }
}

/// 生成过程中的增量。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Content(String),
    Reasoning(String),
}

/// 一轮生成的结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    pub content: String,
    pub provider_state: Option<Value>,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: Option<String>,
}

impl Turn {
    /// 写回对话历史的助手消息（含 `reasoning_content`，下一轮要原样回传）。
    pub fn message(&self) -> Message {
        Message::Assistant {
            content: self.content.clone(),
            provider_state: self.provider_state.clone(),
            reasoning_content: self.reasoning.clone(),
            tool_calls: self.tool_calls.clone(),
        }
    }
}

/// 模型服务。测试用脚本化的实现代替真实网络。
pub trait Provider: Send + Sync {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[Value],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Turn>;
}

/// 请求体。`tools` 已是 OpenAI 的 `{type: function, function: {...}}` 形状。
pub fn request_body(config: &ProviderConfig, messages: &[Message], tools: &[Value]) -> Value {
    let mut body = json!({
        "model": config.model,
        "messages": messages.iter().map(|m| chat_message(config, m)).collect::<Vec<_>>(),
        "stream": true,
        "max_tokens": config.max_output_tokens,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    let glm_53 = provider_id(config) == "glm" && config.model.contains("glm-5.3");
    if let Some(thinking) = config
        .thinking
        .filter(|_| matches!(provider_id(config), "deepseek" | "glm") && !glm_53)
    {
        body["thinking"] = json!({ "type": if thinking { "enabled" } else { "disabled" } });
    }
    if provider_id(config) == "qwen" {
        if let Some(thinking) = config.thinking {
            body["enable_thinking"] = json!(thinking);
        }
    }
    if provider_id(config) == "kimi" && config.model.contains("k2.6") {
        if let Some(thinking) = config.thinking {
            body["thinking"] = json!({"type":if thinking {"enabled"} else {"disabled"}});
        }
    }
    if (config.thinking != Some(false) || glm_53)
        && (!matches!(provider_id(config), "glm" | "qwen") || glm_53)
        && !(provider_id(config) == "kimi" && !config.model.contains("k3"))
    {
        if let Some(effort) = config.reasoning_effort.as_deref().filter(|e| !e.is_empty()) {
            body["reasoning_effort"] = json!(effort);
        }
    }
    merge_options(&mut body, &config.api_options);
    if provider_id(config) == "gemini"
        && (body["extra_body"]["google"]
            .get("thinking_config")
            .is_some()
            || body["google"].get("thinking_config").is_some())
    {
        body.as_object_mut().unwrap().remove("reasoning_effort");
    }
    body
}

/// 把流式块拼成一轮结果。工具调用按 `index` 拼：第一块带 `id`、`type`、`function.name`，
/// 之后的块只带参数片段。
#[derive(Debug, Default)]
pub struct StreamAssembler {
    turn: Turn,
    reasoning: String,
    calls: Vec<(String, String, String)>,
    extras: std::collections::BTreeMap<usize, Value>,
}

impl StreamAssembler {
    /// 吃一块 `data:` 负载（已解析的 JSON），返回其中的文本增量。
    pub fn push(&mut self, chunk: &Value) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.turn.usage = Usage {
                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
            };
        }
        let Some(choice) = chunk["choices"].as_array().and_then(|c| c.first()) else {
            return events;
        };
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.turn.finish_reason = Some(reason.to_owned());
        }
        let delta = &choice["delta"];
        if let Some(text) = delta["reasoning_content"]
            .as_str()
            .filter(|t| !t.is_empty())
        {
            self.reasoning.push_str(text);
            events.push(StreamEvent::Reasoning(text.to_owned()));
        }
        if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            self.turn.content.push_str(text);
            events.push(StreamEvent::Content(text.to_owned()));
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            if self.calls.len() <= index {
                self.calls
                    .resize(index + 1, (String::new(), String::new(), String::new()));
            }
            if let Some(extra) = call.get("extra_content") {
                self.extras.insert(index, extra.clone());
            }
            let slot = &mut self.calls[index];
            if let Some(id) = call["id"].as_str() {
                slot.0.push_str(id);
            }
            if let Some(name) = call["function"]["name"].as_str() {
                slot.1.push_str(name);
            }
            if let Some(arguments) = call["function"]["arguments"].as_str() {
                slot.2.push_str(arguments);
            }
        }
        events
    }

    pub fn finish(mut self) -> Turn {
        if !self.reasoning.is_empty() {
            self.turn.reasoning = Some(self.reasoning);
        }
        if !self.extras.is_empty() {
            self.turn.provider_state = Some(json!({"tool_call_extra":self.extras}));
        }
        self.turn.tool_calls = self
            .calls
            .into_iter()
            .map(|(id, name, arguments)| ToolCall {
                id,
                name,
                arguments: if arguments.trim().is_empty() {
                    "{}".into()
                } else {
                    arguments
                },
            })
            .collect();
        self.turn
    }
}

/// 逐行读 SSE：`data: {...}` 交给 `on_data`，遇到 `data: [DONE]` 结束。注释行与空行忽略。
pub fn read_sse(reader: impl Read, mut on_data: impl FnMut(&str) -> Result<()>) -> Result<bool> {
    for line in BufReader::new(reader.take(16 * 1024 * 1024 + 1)).lines() {
        let line = line.context("the model stream was interrupted")?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(true);
        }
        if !data.is_empty() {
            on_data(data)?;
        }
    }
    Ok(false)
}

/// 真实的 HTTP 客户端。
pub struct OpenAiCompatible {
    config: ProviderConfig,
    agent: ureq::Agent,
}

impl OpenAiCompatible {
    pub fn new(config: ProviderConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(config.timeout_seconds)))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .build()
            .into();
        Self { config, agent }
    }
}

impl Provider for OpenAiCompatible {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[Value],
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Turn> {
        complete_http(&self.config, &self.agent, messages, tools, on_event)
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod provider_tests;

#[path = "provider_adapters.rs"]
mod adapters;
pub use adapters::list_models;
use adapters::*;
