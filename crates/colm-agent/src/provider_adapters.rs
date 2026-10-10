use super::*;

fn service_host(c: &ProviderConfig) -> &str {
    c.base_url
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default()
}
fn is_qwen_host(host: &str) -> bool {
    matches!(
        host,
        "dashscope.aliyuncs.com" | "dashscope-intl.aliyuncs.com" | "dashscope-us.aliyuncs.com"
    ) || host.ends_with(".dashscope.aliyuncs.com")
        || host.ends_with(".maas.aliyuncs.com")
}
pub(super) fn provider_id(c: &ProviderConfig) -> &str {
    if !c.provider_id.is_empty() {
        return &c.provider_id;
    }
    match service_host(c) {
        "api.deepseek.com" => "deepseek",
        host if is_qwen_host(host) => "qwen",
        _ => "custom",
    }
}
pub(super) fn model_list_url(c: &ProviderConfig) -> (String, bool) {
    let host = service_host(c);
    if is_qwen_host(host) {
        let scheme = c
            .base_url
            .split_once("://")
            .map(|(scheme, _)| scheme)
            .unwrap_or("https");
        (
            format!("{scheme}://{host}/api/v1/models?page_no=1&page_size=100"),
            true,
        )
    } else {
        (
            format!("{}/models", c.base_url.trim_end_matches('/')),
            false,
        )
    }
}
pub(super) fn model_ids(value: &Value, qwen: bool) -> Result<Vec<String>> {
    let (models, key) = if qwen {
        (&value["output"]["models"], "model")
    } else {
        (&value["data"], "id")
    };
    let mut ids: Vec<String> = models
        .as_array()
        .context("model list is missing its models array")?
        .iter()
        .filter_map(|m| m[key].as_str())
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .take(500)
        .map(str::to_owned)
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub(super) fn merge_options(body: &mut Value, options: &Value) {
    if let Some(options) = options.as_object() {
        for (key, value) in options {
            if !matches!(
                key.as_str(),
                "model"
                    | "messages"
                    | "input"
                    | "instructions"
                    | "system"
                    | "tools"
                    | "stream"
                    | "api_key"
                    | "authorization"
                    | "store"
                    | "include"
                    | "previous_response_id"
            ) {
                body[key] = value.clone();
            }
        }
    }
}

fn state<'a>(c: &ProviderConfig, state: &'a Option<Value>) -> Option<&'a Value> {
    state
        .as_ref()
        .filter(|s| {
            s["format"] == json!(c.api_format)
                && s["base_url"] == c.base_url
                && s["model"] == c.model
                && s["provider_id"] == provider_id(c)
        })
        .map(|s| &s["items"])
}
pub(super) fn chat_message(c: &ProviderConfig, message: &Message) -> Value {
    let mut value = message.to_api();
    if let Message::Assistant { provider_state, .. } = message {
        let saved = state(c, provider_state);
        let compatible_reasoning = matches!(provider_id(c), "deepseek" | "kimi" | "glm")
            && ((provider_state.is_none() && provider_id(c) == "deepseek") || saved.is_some());
        if !compatible_reasoning {
            value.as_object_mut().unwrap().remove("reasoning_content");
        }
        if let Some(extras) = saved.and_then(|s| s["tool_call_extra"].as_object()) {
            for (i, extra) in extras {
                if let Some(call) = i.parse::<usize>().ok().and_then(|i| {
                    value["tool_calls"]
                        .as_array_mut()
                        .and_then(|calls| calls.get_mut(i))
                }) {
                    call["extra_content"] = extra.clone();
                }
            }
        }
    }
    value
}
fn save_state(c: &ProviderConfig, items: Value) -> Option<Value> {
    Some(
        json!({"format":c.api_format,"base_url":c.base_url,"model":c.model,"provider_id":provider_id(c),"items":items}),
    )
}

pub(super) fn native_body(
    c: &ProviderConfig,
    messages: &[Message],
    tools: &[Value],
) -> Result<Value> {
    let mut input = Vec::new();
    let mut system = Vec::new();
    for message in messages {
        match message {
            Message::System { content } => system.push(content.clone()),
            Message::User { content } => input.push(json!({"role":"user","content":content})),
            Message::Tool { tool_call_id, content } => input.push(if c.api_format == ApiFormat::Responses {
                json!({"type":"function_call_output","call_id":tool_call_id,"output":content})
            } else { json!({"role":"user","content":[{"type":"tool_result","tool_use_id":tool_call_id,"content":content}]}) }),
            Message::Assistant { content, tool_calls, provider_state, .. } => {
                if let Some(items) = state(c, provider_state).and_then(Value::as_array) {
                    if c.api_format == ApiFormat::Responses { input.extend(items.iter().cloned()); }
                    else { input.push(json!({"role":"assistant","content":items})); }
                    continue;
                }
                if c.api_format == ApiFormat::Responses {
                    if !content.is_empty() { input.push(json!({"role":"assistant","content":content})); }
                    for call in tool_calls { input.push(json!({"type":"function_call","call_id":call.id,"name":call.name,"arguments":call.arguments})); }
                } else {
                    let mut blocks = Vec::new();
                    if !content.is_empty() { blocks.push(json!({"type":"text","text":content})); }
                    for call in tool_calls { blocks.push(json!({"type":"tool_use","id":call.id,"name":call.name,"input":serde_json::from_str::<Value>(&call.arguments)?})); }
                    if !blocks.is_empty() { input.push(json!({"role":"assistant","content":blocks})); }
                }
            }
        }
    }
    let mut body = if c.api_format == ApiFormat::Responses {
        json!({"model":c.model,"input":input,"instructions":system.join("\n\n"),"stream":true,"store":false,"include":["reasoning.encrypted_content"],"max_output_tokens":c.max_output_tokens})
    } else {
        json!({"model":c.model,"messages":input,"system":system.join("\n\n"),"stream":true,"max_tokens":c.max_output_tokens})
    };
    if !tools.is_empty() {
        body["tools"] = tools.iter().map(|t| {
            let f=&t["function"];
            if c.api_format == ApiFormat::Responses { json!({"type":"function","name":f["name"],"description":f["description"],"parameters":f["parameters"],"strict":c.strict}) }
            else { json!({"name":f["name"],"description":f["description"],"input_schema":f["parameters"]}) }
        }).collect();
    }
    if c.api_format == ApiFormat::Responses {
        if let Some(effort) = &c.reasoning_effort {
            body["reasoning"] = json!({"effort":effort});
        }
        if c.thinking == Some(false) {
            body["reasoning"] = json!({"effort":"none"});
        }
    } else if let Some(thinking) = c.thinking.or(c.reasoning_effort.as_ref().map(|_| true)) {
        body["thinking"] = if thinking {
            json!({"type":"adaptive"})
        } else {
            json!({"type":"disabled"})
        };
        if thinking {
            if let Some(effort) = &c.reasoning_effort {
                body["output_config"] = json!({"effort":effort});
            }
        }
    }
    merge_options(&mut body, &c.api_options);
    Ok(body)
}

fn validate(c: &ProviderConfig) -> Result<()> {
    if c.api_key.trim().is_empty() {
        bail!("no API key is configured");
    }
    let authority = c
        .base_url
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    if !(c.base_url.starts_with("https://") || c.base_url.starts_with("http://"))
        || authority.is_empty()
        || authority.contains('@')
        || c.base_url.contains(['?', '#', '\n', '\r'])
    {
        bail!("invalid model service URL (use an HTTP(S) base URL without credentials or query)");
    }
    if c.max_output_tokens == 0 || c.timeout_seconds == 0 {
        bail!("output token limit and timeout must be set");
    }
    if !c.api_options.is_object() {
        bail!("API options must be a JSON object");
    }
    Ok(())
}

fn transport_error(error: ureq::Error) -> anyhow::Error {
    let category = match error {
        ureq::Error::Timeout(_) => "timeout",
        ureq::Error::HostNotFound => "DNS lookup failed",
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "TLS connection failed",
        ureq::Error::Io(_) | ureq::Error::ConnectionFailed => "connection failed",
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "redirect refused",
        _ => "invalid request or response",
    };
    anyhow::anyhow!("cannot reach model service: {category}")
}
fn redacted_detail(c: &ProviderConfig, message: &str) -> String {
    message
        .replace(&c.api_key, "[redacted]")
        .chars()
        .filter(|c| !c.is_control())
        .take(400)
        .collect()
}
fn stream_error_detail(c: &ProviderConfig, value: &Value) -> String {
    let message = value["error"]["message"]
        .as_str()
        .or(value["message"].as_str())
        .or(value["incomplete_details"]["reason"].as_str())
        .or(value["error"].as_str())
        .or(value["code"].as_str())
        .unwrap_or("no details provided");
    redacted_detail(c, message)
}
fn service_error(c: &ProviderConfig, status: u16, mut reader: impl Read) -> anyhow::Error {
    let mut bytes = Vec::new();
    let detail =
        if reader.by_ref().take(8193).read_to_end(&mut bytes).is_err() || bytes.len() > 8192 {
            "error details unavailable".into()
        } else {
            let raw = String::from_utf8_lossy(&bytes);
            let json: Option<Value> = serde_json::from_slice(&bytes).ok();
            let message = json
                .as_ref()
                .and_then(|v| v["error"]["message"].as_str().or(v["message"].as_str()))
                .unwrap_or(&raw);
            redacted_detail(c, message)
        };
    anyhow::anyhow!("model service answered HTTP {status}: {detail}")
}

pub(super) fn complete_http(
    c: &ProviderConfig,
    agent: &ureq::Agent,
    messages: &[Message],
    tools: &[Value],
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<Turn> {
    validate(c)?;
    if c.model.trim().is_empty() {
        bail!("model must be set");
    }
    let endpoint = match c.api_format {
        ApiFormat::ChatCompletions => "chat/completions",
        ApiFormat::Responses => "responses",
        ApiFormat::Anthropic => "messages",
    };
    let url = format!("{}/{endpoint}", c.base_url.trim_end_matches('/'));
    let mut request = agent.post(&url).header("Accept", "text/event-stream");
    if c.api_format == ApiFormat::Anthropic {
        request = request
            .header("x-api-key", &c.api_key)
            .header("anthropic-version", "2023-06-01");
    } else {
        request = request.header("Authorization", &format!("Bearer {}", c.api_key));
    }
    let body = if c.api_format == ApiFormat::ChatCompletions {
        request_body(c, messages, tools)
    } else {
        native_body(c, messages, tools)?
    };
    let response = request.send_json(body).map_err(transport_error)?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(service_error(c, status, response.into_body().as_reader()));
    }
    parse_stream(c, response.into_body().as_reader(), on_event)
}

fn validate_turn(turn: &Turn) -> Result<()> {
    if turn.tool_calls.len() > 128 {
        bail!("model returned too many tool calls");
    }
    let mut ids = std::collections::HashSet::new();
    for call in &turn.tool_calls {
        if call.id.is_empty()
            || call.name.is_empty()
            || !ids.insert(&call.id)
            || !serde_json::from_str::<Value>(&call.arguments).is_ok_and(|v| v.is_object())
        {
            bail!("model returned an invalid or incomplete tool call");
        }
    }
    Ok(())
}

pub(super) fn parse_stream(
    c: &ProviderConfig,
    reader: impl Read,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<Turn> {
    let mut chat = StreamAssembler::default();
    let mut turn = Turn::default();
    let mut items = std::collections::BTreeMap::<u64, Value>::new();
    let mut complete = false;
    let mut bytes = 0usize;
    let done = read_sse(reader, |data| {
        bytes = bytes.saturating_add(data.len());
        if bytes > 16 * 1024 * 1024 {
            bail!("model stream exceeded size limit");
        }
        let chunk: Value = serde_json::from_str(data).context("unreadable model stream chunk")?;
        if chunk.get("error").is_some() || chunk["type"] == "error" {
            bail!(
                "model service reported a stream error: {}",
                stream_error_detail(c, &chunk)
            );
        }
        match c.api_format {
            ApiFormat::ChatCompletions => {
                for call in chunk["choices"][0]["delta"]["tool_calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if call["index"].as_u64().is_none_or(|i| i >= 128) {
                        bail!("invalid streamed tool index");
                    }
                }
                for event in chat.push(&chunk) {
                    on_event(event);
                }
            }
            ApiFormat::Responses => match chunk["type"].as_str().unwrap_or_default() {
                "response.output_text.delta" | "response.refusal.delta" => {
                    if let Some(t) = chunk["delta"].as_str() {
                        on_event(StreamEvent::Content(t.into()));
                    }
                }
                "response.reasoning_summary_text.delta" => {
                    if let Some(t) = chunk["delta"].as_str() {
                        on_event(StreamEvent::Reasoning(t.into()));
                    }
                }
                "response.failed" | "response.incomplete" => {
                    bail!(
                        "model response did not complete: {}",
                        stream_error_detail(c, &chunk["response"])
                    )
                }
                "response.completed" => {
                    let response = &chunk["response"];
                    if response["status"] != "completed" {
                        bail!(
                            "model response did not complete: {}",
                            stream_error_detail(c, &chunk["response"])
                        );
                    }
                    let output = response["output"]
                        .as_array()
                        .context("completed response is missing output")?;
                    for item in output {
                        if item
                            .get("status")
                            .is_some_and(|status| status != "completed")
                        {
                            bail!("model returned an incomplete output item");
                        }
                        match item["type"].as_str().unwrap_or_default() {
                            "function_call" => turn.tool_calls.push(ToolCall {
                                id: item["call_id"].as_str().unwrap_or_default().into(),
                                name: item["name"].as_str().unwrap_or_default().into(),
                                arguments: item["arguments"].as_str().unwrap_or_default().into(),
                            }),
                            "message" => {
                                for part in item["content"].as_array().into_iter().flatten() {
                                    if part["type"] == "output_text" {
                                        turn.content
                                            .push_str(part["text"].as_str().unwrap_or_default());
                                    } else if part["type"] == "refusal" {
                                        turn.content
                                            .push_str(part["refusal"].as_str().unwrap_or_default());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    turn.usage = Usage {
                        prompt_tokens: response["usage"]["input_tokens"].as_u64().unwrap_or(0),
                        completion_tokens: response["usage"]["output_tokens"].as_u64().unwrap_or(0),
                    };
                    turn.provider_state = save_state(c, json!(output));
                    turn.finish_reason = Some("completed".into());
                    complete = true;
                }
                _ => {}
            },
            ApiFormat::Anthropic => match chunk["type"].as_str().unwrap_or_default() {
                "message_start" => {
                    turn.usage.prompt_tokens = chunk["message"]["usage"]["input_tokens"]
                        .as_u64()
                        .unwrap_or(0)
                }
                "content_block_start" => {
                    let i = chunk["index"]
                        .as_u64()
                        .filter(|i| *i < 128)
                        .context("invalid content block index")?;
                    items.insert(i, chunk["content_block"].clone());
                }
                "content_block_delta" => {
                    let block = items
                        .get_mut(
                            &chunk["index"]
                                .as_u64()
                                .context("missing content block index")?,
                        )
                        .context("delta without content block")?;
                    let delta = &chunk["delta"];
                    let (field, text) = match delta["type"].as_str().unwrap_or_default() {
                        "text_delta" => ("text", delta["text"].as_str()),
                        "thinking_delta" => ("thinking", delta["thinking"].as_str()),
                        "signature_delta" => ("signature", delta["signature"].as_str()),
                        "input_json_delta" => ("partial_json", delta["partial_json"].as_str()),
                        _ => return Ok(()),
                    };
                    if let Some(text) = text {
                        let previous = block[field].as_str().unwrap_or_default();
                        block[field] = json!(format!("{previous}{text}"));
                        if field == "text" {
                            on_event(StreamEvent::Content(text.into()));
                        }
                        if field == "thinking" {
                            on_event(StreamEvent::Reasoning(text.into()));
                        }
                    }
                }
                "message_delta" => {
                    turn.finish_reason = chunk["delta"]["stop_reason"].as_str().map(str::to_owned);
                    turn.usage.completion_tokens =
                        chunk["usage"]["output_tokens"].as_u64().unwrap_or(0);
                }
                "message_stop" => complete = true,
                _ => {}
            },
        }
        Ok(())
    })?;
    if c.api_format == ApiFormat::ChatCompletions {
        turn = chat.finish();
        turn.provider_state =
            save_state(c, turn.provider_state.take().unwrap_or_else(|| json!({})));
        complete = done && matches!(turn.finish_reason.as_deref(), Some("stop" | "tool_calls"));
    } else if c.api_format == ApiFormat::Anthropic {
        complete &= matches!(
            turn.finish_reason.as_deref(),
            Some("end_turn" | "tool_use" | "stop_sequence")
        );
        for block in items.values_mut() {
            match block["type"].as_str().unwrap_or_default() {
                "tool_use" => {
                    if let Some(partial) =
                        block.as_object_mut().and_then(|b| b.remove("partial_json"))
                    {
                        block["input"] = serde_json::from_str(partial.as_str().unwrap_or_default())
                            .context("incomplete tool JSON")?;
                    }
                    turn.tool_calls.push(ToolCall {
                        id: block["id"].as_str().unwrap_or_default().into(),
                        name: block["name"].as_str().unwrap_or_default().into(),
                        arguments: block["input"].to_string(),
                    });
                }
                "text" => turn
                    .content
                    .push_str(block["text"].as_str().unwrap_or_default()),
                "thinking" => {
                    if block["signature"].as_str().is_none_or(str::is_empty) {
                        bail!("thinking block is missing its signature");
                    }
                    turn.reasoning
                        .get_or_insert_default()
                        .push_str(block["thinking"].as_str().unwrap_or_default());
                }
                _ => {}
            }
        }
        turn.provider_state = save_state(c, items.into_values().collect());
    }
    if !complete {
        bail!("model stream ended without a successful completion; no tools were executed");
    }
    validate_turn(&turn)?;
    Ok(turn)
}

pub fn list_models(c: &ProviderConfig) -> Result<Vec<String>> {
    validate(c)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(c.timeout_seconds.min(60))))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .into();
    let (url, qwen) = model_list_url(c);
    let mut ids = Vec::new();
    let mut seen = 0;
    // Bound discovery even if a service returns inconsistent pagination metadata.
    for page in 1..=5 {
        let page_url = if qwen {
            url.replace("page_no=1", &format!("page_no={page}"))
        } else {
            url.clone()
        };
        let mut request = agent.get(&page_url);
        if c.api_format == ApiFormat::Anthropic {
            request = request
                .header("x-api-key", &c.api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            request = request.header("Authorization", &format!("Bearer {}", c.api_key));
        }
        let mut response = request.call().map_err(transport_error)?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(service_error(c, status, response.into_body().as_reader()));
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .context("cannot read model list")?;
        if bytes.len() > 1024 * 1024 {
            bail!("model list exceeded size limit");
        }
        let value: Value = serde_json::from_slice(&bytes).context("invalid model list response")?;
        let page_ids = model_ids(&value, qwen)?;
        let count = value["output"]["models"].as_array().map_or(0, Vec::len);
        seen += count;
        ids.extend(page_ids);
        if !qwen
            || count == 0
            || value["output"]["total"]
                .as_u64()
                .is_none_or(|total| seen as u64 >= total)
        {
            break;
        }
    }
    ids.sort();
    ids.dedup();
    ids.truncate(500);
    Ok(ids)
}
