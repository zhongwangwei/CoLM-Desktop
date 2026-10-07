//! 对话消息（OpenAI Chat Completions 格式）。
//!
//! DeepSeek 的思考模式要求：有工具调用时，之后每一轮都要把助手消息的 `reasoning_content` 原样回传，
//! 否则返回 400（docs/design-ai-assistant.md 第 3.1 节）。所以助手消息保留它，序列化时一并写出。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 一次工具调用：`arguments` 是模型给出的 JSON 文本（原样保留，回传时不改写）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl Message {
    /// 请求体里的形状（OpenAI 兼容）。
    pub fn to_api(&self) -> Value {
        match self {
            Self::System { content } => json!({ "role": "system", "content": content }),
            Self::User { content } => json!({ "role": "user", "content": content }),
            Self::Assistant {
                content,
                reasoning_content,
                tool_calls,
            } => {
                let mut message = json!({ "role": "assistant", "content": content });
                if let Some(reasoning) = reasoning_content {
                    message["reasoning_content"] = json!(reasoning);
                }
                if !tool_calls.is_empty() {
                    message["tool_calls"] = tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": { "name": call.name, "arguments": call.arguments },
                            })
                        })
                        .collect();
                }
                message
            }
            Self::Tool {
                tool_call_id,
                content,
            } => json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content }),
        }
    }
}

/// 一轮生成的用量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[cfg(test)]
#[path = "message_tests.rs"]
mod message_tests;
