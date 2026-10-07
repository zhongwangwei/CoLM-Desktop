//! OpenAI 兼容的 Chat Completions 客户端（流式），默认对接 DeepSeek。
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

/// 模型服务的配置。Key 不在这里持久化，由调用方从钥匙串取出后填入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    #[serde(skip)]
    pub api_key: String,
    /// DeepSeek 的思考模式：`None` 不发这个参数（服务端默认开启）。
    #[serde(default)]
    pub thinking: Option<bool>,
    /// 工具 schema 带 `strict: true`（DeepSeek 要配 `/beta` 地址）。
    #[serde(default)]
    pub strict: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_timeout() -> u64 {
    600
}

impl ProviderConfig {
    /// DeepSeek 官方 API 的缺省配置。
    pub fn deepseek(model: &str, api_key: String) -> Self {
        Self {
            base_url: DEEPSEEK_BASE_URL.into(),
            model: model.into(),
            api_key,
            thinking: None,
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
        "messages": messages.iter().map(Message::to_api).collect::<Vec<_>>(),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(thinking) = config.thinking {
        body["thinking"] = json!({ "type": if thinking { "enabled" } else { "disabled" } });
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
        self.turn.tool_calls = self
            .calls
            .into_iter()
            .filter(|(_, name, _)| !name.is_empty())
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
pub fn read_sse(reader: impl Read, mut on_data: impl FnMut(&str) -> Result<()>) -> Result<()> {
    for line in BufReader::new(reader).lines() {
        let line = line.context("the model stream was interrupted")?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        if !data.is_empty() {
            on_data(data)?;
        }
    }
    Ok(())
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
        if self.config.api_key.trim().is_empty() {
            bail!("no API key is configured for {}", self.config.base_url);
        }
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .header("Accept", "text/event-stream")
            .send_json(request_body(&self.config, messages, tools))
            .with_context(|| format!("cannot reach {url}"))?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        if !(200..300).contains(&status) {
            let text = body.read_to_string().unwrap_or_default();
            bail!("{url} answered HTTP {status}: {}", text.trim());
        }
        let mut assembler = StreamAssembler::default();
        read_sse(body.as_reader(), |data| {
            let chunk: Value = serde_json::from_str(data)
                .with_context(|| format!("unreadable stream chunk: {data}"))?;
            if let Some(error) = chunk.get("error") {
                bail!("the model service reported an error: {error}");
            }
            for event in assembler.push(&chunk) {
                on_event(event);
            }
            Ok(())
        })?;
        Ok(assembler.finish())
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod provider_tests;
