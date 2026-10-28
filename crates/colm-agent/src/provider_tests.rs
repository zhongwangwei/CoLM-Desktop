use super::*;

#[test]
fn streamed_tool_call_fragments_are_joined_by_index() {
    let stream = concat!(
        ": keep-alive\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"check \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"metrics\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Looking\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"metrics\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"case\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"type\":\"function\",\"function\":{\"name\":\"run_status\",\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"/p/x\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":30}}\n\n",
        "data: [DONE]\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ignored after DONE\"}}]}\n\n",
    );
    let mut assembler = StreamAssembler::default();
    let mut events = Vec::new();
    read_sse(stream.as_bytes(), |data| {
        events.extend(assembler.push(&serde_json::from_str(data)?));
        Ok(())
    })
    .unwrap();
    let turn = assembler.finish();
    assert_eq!(turn.content, "Looking");
    assert_eq!(turn.reasoning.as_deref(), Some("check metrics"));
    assert_eq!(turn.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(
        turn.usage,
        Usage {
            prompt_tokens: 120,
            completion_tokens: 30
        }
    );
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[0].id, "call_a");
    assert_eq!(turn.tool_calls[0].name, "metrics");
    assert_eq!(turn.tool_calls[0].arguments, r#"{"case":"/p/x"}"#);
    assert_eq!(turn.tool_calls[1].name, "run_status");
    assert_eq!(
        events,
        vec![
            StreamEvent::Reasoning("check ".into()),
            StreamEvent::Reasoning("metrics".into()),
            StreamEvent::Content("Looking".into()),
        ]
    );
    // 下一轮回传的助手消息带着推理内容。
    let api = turn.message().to_api();
    assert_eq!(api["reasoning_content"], "check metrics");
}

#[test]
fn request_bodies_stream_with_usage_and_only_send_thinking_when_set() {
    let mut config = ProviderConfig::deepseek("deepseek-flash", "k".into());
    let messages = [Message::User {
        content: "hi".into(),
    }];
    let body = request_body(&config, &messages, &[]);
    assert_eq!(body["model"], "deepseek-flash");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert!(body.get("tools").is_none());
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
    config.reasoning_effort = Some("max".into());
    assert_eq!(
        request_body(&config, &messages, &[])["reasoning_effort"],
        "max"
    );
    config.thinking = Some(false);
    let tools = [json!({"type": "function", "function": {"name": "x"}})];
    let body = request_body(&config, &messages, &tools);
    assert_eq!(body["thinking"]["type"], "disabled");
    // 关了思考就不发强度。
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["tools"][0]["function"]["name"], "x");
    // Key 不进序列化的配置。
    assert!(!serde_json::to_string(&config).unwrap().contains("\"k\""));
}

#[test]
fn a_call_without_arguments_gets_an_empty_object() {
    let mut assembler = StreamAssembler::default();
    assembler.push(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"list_cases"}}]}}]}));
    let turn = assembler.finish();
    assert_eq!(turn.tool_calls[0].arguments, "{}");
}

#[test]
fn tool_free_summary_requests_override_forced_tool_options_for_all_api_formats() {
    let tools = [
        json!({"type":"function","function":{"name":"status","description":"status","parameters":{"type":"object","properties":{}}}}),
    ];
    for format in [
        ApiFormat::ChatCompletions,
        ApiFormat::Responses,
        ApiFormat::Anthropic,
    ] {
        let mut config = ProviderConfig::deepseek("test", "k".into());
        config.api_format = format;
        config.api_options =
            json!({"tool_choice":"required","parallel_tool_calls":true,"temperature":0.2});
        let body = |tools: &[Value]| match format {
            ApiFormat::ChatCompletions => request_body(&config, &[], tools),
            _ => adapters::native_body(&config, &[], tools).unwrap(),
        };
        let summary = body(&[]);
        assert!(summary.get("tools").is_none());
        assert!(summary.get("tool_choice").is_none());
        assert!(summary.get("parallel_tool_calls").is_none());
        assert_eq!(summary["temperature"], 0.2);
        assert_eq!(body(&tools)["tool_choice"], "required");
    }
}

fn native_config(format: ApiFormat) -> ProviderConfig {
    let mut c = ProviderConfig::deepseek("model", "secret".into());
    c.api_format = format;
    c.base_url = "https://example.test/v1".into();
    c.provider_id = "custom".into();
    c
}
fn stream(events: &[Value]) -> String {
    events.iter().map(|v| format!("data: {v}\n\n")).collect()
}

#[test]
fn responses_roundtrip_encrypted_reasoning_and_tools_without_foreign_state() {
    let c = native_config(ApiFormat::Responses);
    let output = json!([
        {"type":"reasoning","id":"rs_1","encrypted_content":"opaque","summary":[]},
        {"type":"function_call","call_id":"call_1","name":"inspect","arguments":"{}"}
    ]);
    let wire = stream(&[
        json!({"type":"response.output_text.delta","delta":"hello"}),
        json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":9,"output_tokens":3}}}),
    ]);
    let turn = adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {}).unwrap();
    assert_eq!(turn.tool_calls[0].name, "inspect");
    assert_eq!(turn.usage.prompt_tokens, 9);
    let saved = serde_json::to_string(&turn.message()).unwrap();
    let message: Message = serde_json::from_str(&saved).unwrap();
    let body = adapters::native_body(
        &c,
        &[
            message.clone(),
            Message::Tool {
                tool_call_id: "call_1".into(),
                content: "result".into(),
            },
        ],
        &[],
    )
    .unwrap();
    assert_eq!(body["input"][0], output[0]);
    assert_eq!(body["input"][2]["type"], "function_call_output");
    assert_eq!(body["store"], false);
    let mut switched = c.clone();
    switched.base_url = "https://other.test/v1".into();
    assert!(!adapters::native_body(&switched, &[message], &[])
        .unwrap()
        .to_string()
        .contains("opaque"));
    for status in ["incomplete", "failed"] {
        let wire =
            stream(&[json!({"type":format!("response.{status}"),"response":{"status":status}})]);
        assert!(adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {}).is_err());
    }
    let partial = stream(&[json!({"type":"response.function_call_arguments.delta","delta":"{}"})]);
    assert!(adapters::parse_stream(&c, partial.as_bytes(), &mut |_| {}).is_err());
}

#[test]
fn anthropic_signed_blocks_survive_tools_and_restarts() {
    let c = native_config(ApiFormat::Anthropic);
    let events = vec![
        json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reason"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"opaque"}}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"inspect","input":{}}}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ];
    let wire = stream(&events);
    let turn = adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {}).unwrap();
    assert_eq!(turn.tool_calls[0].arguments, "{\"x\":1}");
    let message: Message =
        serde_json::from_str(&serde_json::to_string(&turn.message()).unwrap()).unwrap();
    let body = adapters::native_body(
        &c,
        &[
            message,
            Message::Tool {
                tool_call_id: "t1".into(),
                content: "ok".into(),
            },
        ],
        &[],
    )
    .unwrap();
    assert_eq!(body["messages"][0]["content"][0]["signature"], "signed");
    assert_eq!(
        body["messages"][0]["content"][1]["type"],
        "redacted_thinking"
    );
    assert_eq!(body["messages"][1]["content"][0]["tool_use_id"], "t1");
    let truncated = stream(&events[..events.len() - 1]);
    assert!(adapters::parse_stream(&c, truncated.as_bytes(), &mut |_| {}).is_err());
    let mut limited = events.clone();
    limited[7]["delta"]["stop_reason"] = json!("max_tokens");
    assert!(adapters::parse_stream(&c, stream(&limited).as_bytes(), &mut |_| {}).is_err());
}

#[test]
fn chat_signatures_are_scoped_and_partial_tools_fail_closed() {
    let c = native_config(ApiFormat::ChatCompletions);
    let wire = stream(&[
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"t1","function":{"name":"inspect","arguments":"{}"},"extra_content":{"google":{"thought_signature":"opaque"}}}]},"finish_reason":"tool_calls"}]}),
    ]) + "data: [DONE]\n\n";
    let turn = adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {}).unwrap();
    let body = request_body(&c, &[turn.message()], &[]);
    assert_eq!(
        body["messages"][0]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
        "opaque"
    );
    let mut other = c.clone();
    other.model = "another-model".into();
    assert!(!request_body(&other, &[turn.message()], &[])
        .to_string()
        .contains("opaque"));
    assert!(adapters::parse_stream(
        &c,
        wire.replace("data: [DONE]\n\n", "").as_bytes(),
        &mut |_| {}
    )
    .is_err());
    assert!(adapters::parse_stream(
        &c,
        wire.replace("\"index\":0", "\"index\":99999999").as_bytes(),
        &mut |_| {}
    )
    .is_err());
}

#[test]
fn options_cannot_replace_history_or_credentials_and_thinking_is_provider_specific() {
    let mut c = native_config(ApiFormat::ChatCompletions);
    c.thinking = Some(true);
    c.api_options = json!({"model":"evil","messages":[],"tools":[],"stream":false,"api_key":"leak","temperature":0.2});
    let body = request_body(
        &c,
        &[Message::User {
            content: "keep".into(),
        }],
        &[],
    );
    assert_eq!(body["model"], "model");
    assert_eq!(body["messages"][0]["content"], "keep");
    assert_eq!(body["stream"], true);
    assert!(body.get("api_key").is_none());
    assert!(body.get("thinking").is_none());
    assert_eq!(body["temperature"], 0.2);
    c.provider_id = "qwen".into();
    assert_eq!(request_body(&c, &[], &[])["enable_thinking"], true);
    c.provider_id = "kimi".into();
    c.model = "kimi-k3".into();
    assert!(request_body(&c, &[], &[]).get("thinking").is_none());
}

#[test]
fn native_wire_endpoints_headers_and_model_listing_use_mock_http() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    for format in [ApiFormat::Responses, ApiFormat::Anthropic] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = native_config(format);
        c.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for listing in [false, true] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse::<usize>().unwrap();
                    }
                    head.push_str(&line);
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let lower = head.to_lowercase();
                if format == ApiFormat::Anthropic {
                    assert!(lower.contains("x-api-key: secret"));
                    assert!(lower.contains("anthropic-version: 2023-06-01"));
                    assert!(!lower.contains("authorization:"));
                } else {
                    assert!(lower.contains("authorization: bearer secret"));
                }
                let body = if listing {
                    assert!(head.starts_with("GET /v1/models "));
                    json!({"data":[{"id":"b"},{"id":"a"},{"id":"a"},{"id":"bad\n"}]}).to_string()
                } else {
                    let payload: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(payload["model"], "model");
                    assert_eq!(payload["stream"], true);
                    if format == ApiFormat::Responses {
                        assert!(head.starts_with("POST /v1/responses "));
                        stream(&[
                            json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}}),
                        ])
                    } else {
                        assert!(head.starts_with("POST /v1/messages "));
                        stream(&[
                            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"ok"}}),
                            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
                            json!({"type":"message_stop"}),
                        ])
                    }
                };
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let client = OpenAiCompatible::new(c.clone());
        let turn = client
            .complete(
                &[Message::User {
                    content: "hi".into(),
                }],
                &[],
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(turn.content, "ok");
        assert_eq!(list_models(&c).unwrap(), vec!["a", "b"]);
        server.join().unwrap();
    }
}

#[test]
fn legacy_config_defaults_and_native_effort_mapping_are_compatible() {
    let legacy: ProviderConfig = serde_json::from_value(
        json!({"base_url":"https://api.deepseek.com","model":"deepseek-chat"}),
    )
    .unwrap();
    assert_eq!(legacy.api_format, ApiFormat::ChatCompletions);
    assert_eq!(legacy.max_output_tokens, 16384);
    assert_eq!(adapters::provider_id(&legacy), "deepseek");
    let mut c = native_config(ApiFormat::Anthropic);
    c.reasoning_effort = Some("high".into());
    let body = adapters::native_body(&c, &[], &[]).unwrap();
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
    assert!(body.get("reasoning_effort").is_none());
    c.api_format = ApiFormat::Responses;
    let body = adapters::native_body(&c, &[], &[]).unwrap();
    assert_eq!(body["reasoning"]["effort"], "high");
    c.api_format = ApiFormat::ChatCompletions;
    c.provider_id = "gemini".into();
    c.api_options = json!({"extra_body":{"google":{"thinking_config":{"thinking_budget":1000}}}});
    assert!(request_body(&c, &[], &[]).get("reasoning_effort").is_none());
}

#[test]
fn glm_53_forces_thinking_and_maps_effort_while_older_glm_keeps_switch() {
    let mut c = native_config(ApiFormat::ChatCompletions);
    c.provider_id = "glm".into();
    c.model = "glm-5.3".into();
    c.reasoning_effort = Some("max".into());
    let body = request_body(&c, &[], &[]);
    assert!(body.get("thinking").is_none());
    assert_eq!(body["reasoning_effort"], "max");
    c.thinking = Some(false);
    assert!(OpenAiCompatible::new(c.clone())
        .complete(&[], &[], &mut |_| {})
        .unwrap_err()
        .to_string()
        .contains("cannot disable thinking"));
    c.model = "glm-4.7".into();
    let body = request_body(&c, &[], &[]);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn qwen_38_sends_effort_and_an_explicit_off_switch() {
    let mut c = native_config(ApiFormat::ChatCompletions);
    c.provider_id = "qwen".into();
    c.model = "qwen3.8-flash".into();
    for effort in ["low", "medium", "xhigh"] {
        c.reasoning_effort = Some(effort.into());
        let body = request_body(&c, &[], &[]);
        assert_eq!(body["reasoning_effort"], effort);
        assert!(body.get("enable_thinking").is_none());
    }
    c.thinking = Some(false);
    c.reasoning_effort = None;
    assert_eq!(request_body(&c, &[], &[])["reasoning_effort"], "none");
    c.model = "qwen-plus".into();
    let body = request_body(&c, &[], &[]);
    assert_eq!(body["enable_thinking"], false);
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn qwen_preserved_reasoning_is_bound_to_the_actual_provider_model_and_url() {
    let mut c = native_config(ApiFormat::ChatCompletions);
    c.provider_id = "qwen".into();
    c.model = "qwen3.8-flash".into();
    let wire = stream(&[
        json!({"choices":[{"delta":{"reasoning_content":"check the source"}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"t1","function":{"name":"inspect","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}),
    ]) + "data: [DONE]\n\n";
    let turn = adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {}).unwrap();
    let message = turn.message();
    let body = request_body(&c, std::slice::from_ref(&message), &[]);
    assert_eq!(body["messages"][0]["reasoning_content"], "check the source");
    assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "t1");
    let mut foreign = c.clone();
    foreign.model = "other-model".into();
    assert!(
        request_body(&foreign, std::slice::from_ref(&message), &[])["messages"][0]
            .get("reasoning_content")
            .is_none()
    );
    foreign = c.clone();
    foreign.base_url = "https://other.example/v1".into();
    assert!(
        request_body(&foreign, std::slice::from_ref(&message), &[])["messages"][0]
            .get("reasoning_content")
            .is_none()
    );
    foreign = c.clone();
    foreign.provider_id = "custom".into();
    assert!(request_body(&foreign, &[message], &[])["messages"][0]
        .get("reasoning_content")
        .is_none());
}

#[test]
fn known_invalid_thinking_settings_fail_before_any_http_request() {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    for (provider, model, format, thinking, effort, options, expected) in [
        (
            "gemini",
            "gemini-3.8-flash",
            ApiFormat::ChatCompletions,
            None,
            Some("minimal"),
            json!({}),
            "minimal",
        ),
        (
            "gemini",
            "gemini-3.8-flash",
            ApiFormat::ChatCompletions,
            None,
            None,
            json!({"reasoning_effort":"minimal"}),
            "minimal",
        ),
        (
            "qwen",
            "qwen3.8-flash",
            ApiFormat::ChatCompletions,
            None,
            Some("low"),
            json!({"thinking_budget":4096}),
            "cannot be set together",
        ),
        (
            "qwen",
            "qwen3.8-flash",
            ApiFormat::ChatCompletions,
            Some(false),
            None,
            json!({"reasoning_effort":"high"}),
            "conflicts",
        ),
        (
            "openai",
            "gpt-6.1-sol",
            ApiFormat::Responses,
            Some(false),
            None,
            json!({}),
            "cannot disable thinking",
        ),
        (
            "openai",
            "gpt-6.1-sol",
            ApiFormat::Responses,
            None,
            None,
            json!({"reasoning":{"effort":"none"}}),
            "none",
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            ApiFormat::Anthropic,
            Some(false),
            None,
            json!({}),
            "cannot disable thinking",
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            ApiFormat::Anthropic,
            None,
            None,
            json!({"thinking":{"type":"disabled"}}),
            "disabled",
        ),
        (
            "anthropic",
            "claude-sonnet-5-5",
            ApiFormat::Anthropic,
            None,
            Some("max"),
            json!({"thinking":{"type":"between_tools"}}),
            "between_tools",
        ),
        (
            "glm",
            "glm-5.3",
            ApiFormat::ChatCompletions,
            Some(false),
            None,
            json!({}),
            "cannot disable thinking",
        ),
        (
            "kimi",
            "kimi-k3",
            ApiFormat::ChatCompletions,
            Some(false),
            None,
            json!({}),
            "cannot disable thinking",
        ),
    ] {
        let mut c = native_config(format);
        c.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        c.provider_id = provider.into();
        c.model = model.into();
        c.thinking = thinking;
        c.reasoning_effort = effort.map(str::to_owned);
        c.api_options = options;
        let error = OpenAiCompatible::new(c)
            .complete(&[], &[], &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{model}: {error}");
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[test]
fn sonnet_between_tools_is_an_explicit_expert_option_and_unknown_models_keep_their_settings() {
    let mut c = native_config(ApiFormat::Anthropic);
    c.provider_id = "anthropic".into();
    c.model = "claude-sonnet-5-5".into();
    c.reasoning_effort = Some("low".into());
    c.api_options = json!({"thinking":{"type":"between_tools"}});
    let body = adapters::native_body(&c, &[], &[]).unwrap();
    assert_eq!(body["thinking"]["type"], "between_tools");
    assert_eq!(body["output_config"]["effort"], "low");
    c.api_options = json!({"temperature":0.2});
    assert!(adapters::native_body(&c, &[], &[])
        .unwrap_err()
        .to_string()
        .contains("temperature"));
    c.api_options = json!({"temperature":1});
    assert!(adapters::native_body(&c, &[], &[]).is_ok());
    c.model = "future-model".into();
    c.reasoning_effort = Some("future_effort".into());
    c.api_options = json!({});
    assert_eq!(
        adapters::native_body(&c, &[], &[]).unwrap()["output_config"]["effort"],
        "future_effort"
    );
    c.provider_id = "openai".into();
    c.model = "gpt-6.1-sol".into();
    c.api_format = ApiFormat::Responses;
    c.reasoning_effort = None;
    c.api_options = json!({"temperature":1});
    assert!(adapters::native_body(&c, &[], &[])
        .unwrap_err()
        .to_string()
        .contains("sampling parameters"));
}

#[test]
fn anthropic_tools_keep_the_requested_strict_flag_and_input_schema() {
    let mut c = native_config(ApiFormat::Anthropic);
    for (strict, configured, expected) in [
        (Some(true), false, Some(true)),
        (Some(false), true, Some(false)),
        (None, true, Some(true)),
        (None, false, None),
    ] {
        c.strict = configured;
        let mut tool = json!({"type":"function","function":{"name":"inspect","description":"Inspect a case","parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false}}});
        if let Some(strict) = strict {
            tool["function"]["strict"] = json!(strict);
        }
        let body = adapters::native_body(&c, &[], &[tool.clone()]).unwrap();
        assert_eq!(body["tools"][0]["strict"].as_bool(), expected);
        assert_eq!(
            body["tools"][0]["input_schema"],
            tool["function"]["parameters"]
        );
        assert_eq!(body["tools"][0]["name"], "inspect");
    }
}

#[test]
fn sonnet_forced_tool_selection_is_rejected_before_http_but_custom_models_are_unchanged() {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let tools = [
        json!({"type":"function","function":{"name":"inspect","description":"Inspect","parameters":{"type":"object","properties":{}}}}),
    ];
    let mut c = native_config(ApiFormat::Anthropic);
    c.provider_id = "anthropic".into();
    c.model = "claude-sonnet-5-5".into();
    c.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    for choice in [
        json!({"type":"any"}),
        json!({"type":"tool","name":"inspect"}),
        json!("required"),
    ] {
        c.api_options = json!({"tool_choice":choice});
        let error = OpenAiCompatible::new(c.clone())
            .complete(&[], &tools, &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(error.contains("tool_choice auto or none"), "{error}");
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let summary = adapters::native_body(&c, &[], &[]).unwrap();
        assert!(summary.get("tool_choice").is_none());
        let mut unknown = c.clone();
        unknown.model = "custom-model".into();
        assert_eq!(
            adapters::native_body(&unknown, &[], &tools).unwrap()["tool_choice"],
            choice
        );
    }
    for choice in ["auto", "none"] {
        c.api_options = json!({"tool_choice":{"type":choice}});
        assert_eq!(
            adapters::native_body(&c, &[], &tools).unwrap()["tool_choice"]["type"],
            choice
        );
    }
}

#[test]
fn qwen_discovery_uses_official_service_root_and_output_model_field() {
    let mut c = native_config(ApiFormat::ChatCompletions);
    c.provider_id.clear();
    for host in [
        "dashscope.aliyuncs.com",
        "dashscope-intl.aliyuncs.com",
        "workspace.cn-beijing.maas.aliyuncs.com",
    ] {
        c.base_url = format!("https://{host}/compatible-mode/v1");
        assert_eq!(adapters::provider_id(&c), "qwen");
        assert_eq!(
            adapters::model_list_url(&c),
            (
                format!("https://{host}/api/v1/models?page_no=1&page_size=100"),
                true
            )
        );
    }
    c.provider_id = "qwen".into();
    c.base_url = "https://proxy.example/v1".into();
    assert_eq!(
        adapters::model_list_url(&c),
        ("https://proxy.example/v1/models".into(), false)
    );
    c.provider_id.clear();
    c.base_url = "https://evil-dashscope.aliyuncs.com/compatible-mode/v1".into();
    assert_eq!(adapters::provider_id(&c), "custom");
    let ids=adapters::model_ids(&json!({"output":{"models":[{"model":"qwen3-max"},{"model":"qwen3-flash"},{"model":"qwen3-max"},{"model":"bad\n"}]}}),true).unwrap();
    assert_eq!(ids, vec!["qwen3-flash", "qwen3-max"]);
    assert!(adapters::model_ids(&json!({"data":[]}), true).is_err());
}

#[test]
fn anthropic_redirects_never_forward_keys_and_errors_are_redacted() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut c = native_config(ApiFormat::Anthropic);
    c.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let redirect = format!("http://{}/stolen", destination.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for index in 0..4 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            if index < 2 {
                write!(socket,"HTTP/1.1 302 Found\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else {
                let body=json!({"error":{"message":"unsupported reasoning effort; supplied key secret"}}).to_string();
                write!(socket,"HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        }
    });
    let client = OpenAiCompatible::new(c.clone());
    for error in [
        client.complete(&[], &[], &mut |_| {}).unwrap_err(),
        list_models(&c).unwrap_err(),
    ] {
        assert!(error.to_string().contains("HTTP 302"));
    }
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    for error in [
        client.complete(&[], &[], &mut |_| {}).unwrap_err(),
        list_models(&c).unwrap_err(),
    ] {
        let text = error.to_string();
        assert!(text.contains("HTTP 400"));
        assert!(text.contains("unsupported reasoning effort"));
        assert!(!text.contains("secret"));
        assert!(text.contains("[redacted]"));
    }
    server.join().unwrap();
}

#[test]
fn response_refusal_is_visible_and_malformed_tool_rejects_entire_turn() {
    let c = native_config(ApiFormat::Responses);
    let wire = stream(&[
        json!({"type":"response.refusal.delta","delta":"Cannot help"}),
        json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"Cannot help"}]}]}}),
    ]);
    let mut events = Vec::new();
    let turn =
        adapters::parse_stream(&c, wire.as_bytes(), &mut |event| events.push(event)).unwrap();
    assert_eq!(turn.content, "Cannot help");
    assert_eq!(events, vec![StreamEvent::Content("Cannot help".into())]);
    let c = native_config(ApiFormat::ChatCompletions);
    let wire = stream(&[
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"good","function":{"name":"inspect","arguments":"{}"}},{"index":1,"id":"bad","function":{"arguments":"{}"}}]},"finish_reason":"tool_calls"}]}),
    ]) + "data: [DONE]\n\n";
    assert!(adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {})
        .unwrap_err()
        .to_string()
        .contains("invalid or incomplete tool"));
}

#[test]
fn stream_errors_preserve_bounded_redacted_service_reasons() {
    for format in [
        ApiFormat::ChatCompletions,
        ApiFormat::Anthropic,
        ApiFormat::Responses,
    ] {
        let c = native_config(format);
        let wire = stream(&[
            json!({"type":"error","error":{"message":"unsupported reasoning_effort; key secret"}}),
        ]);
        let error = adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported reasoning_effort"));
        assert!(error.contains("[redacted]"));
        assert!(!error.contains("secret"));
    }
    let c = native_config(ApiFormat::Responses);
    for event in [
        json!({"type":"response.failed","response":{"error":{"message":"unknown model; secret"}}}),
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens; secret"}}}),
    ] {
        let error = adapters::parse_stream(
            &c,
            stream(std::slice::from_ref(&event)).as_bytes(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(if event["type"] == "response.failed" {
            "unknown model"
        } else {
            "max_output_tokens"
        }));
        assert!(error.contains("[redacted]"));
        assert!(!error.contains("secret"));
    }
    let wire = stream(&[json!({"type":"error","message":format!("{}secret", "x".repeat(1000))})]);
    let error = adapters::parse_stream(&c, wire.as_bytes(), &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(error.len() < 500);
    assert!(!error.contains("secret"));
}

#[test]
fn provider_debug_redacts_the_api_key() {
    let config = ProviderConfig::deepseek("deepseek-flash", "sk-secret-value".into());
    let text = format!("{config:?}");
    assert!(!text.contains("sk-secret-value"), "{text}");
    assert!(text.contains("<set>"), "{text}");
}

#[test]
fn a_cancelled_stream_stops_at_the_next_line() {
    // 点"停止"之后不再读模型流（G-02）：第一段之后置位，第二段不再交出去。
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let wire = "data: {\"a\":1}\n\ndata: {\"a\":2}\n\ndata: [DONE]\n";
    let mut seen = Vec::new();
    let error = crate::provider::read_sse_cancellable(wire.as_bytes(), &cancel, |data| {
        seen.push(data.to_owned());
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "cancelled");
    assert_eq!(seen, ["{\"a\":1}"]);
}

#[test]
fn effective_context_window_prefers_user_value_and_only_trusts_documented_presets() {
    for model in ["deepseek-flash", "deepseek-v4-pro"] {
        let mut c = ProviderConfig::deepseek(model, String::new());
        c.base_url = "https://api.deepseek.com/v1".into();
        assert_eq!(
            c.effective_context_window().unwrap(),
            (CONTEXT_WINDOW_TOKENS, WindowSource::Preset),
            "{model}"
        );
    }
    // 历史模型名（deepseek-chat 已停用）没有文档确认的窗口：不猜，要求填写。
    let mut unknown = ProviderConfig::deepseek("deepseek-chat", String::new());
    let text = unknown.effective_context_window().unwrap_err().to_string();
    assert!(text.contains("context window"), "{text}");
    assert!(text.contains("settings"), "{text}");
    // 同一模型走代理/非官方地址、或换协议都不算预设。
    let mut proxied = ProviderConfig::deepseek("deepseek-flash", String::new());
    proxied.base_url = "https://proxy.example/v1".into();
    assert!(proxied.effective_context_window().is_err());
    let mut responses = ProviderConfig::deepseek("deepseek-flash", String::new());
    responses.api_format = ApiFormat::Responses;
    assert!(responses.effective_context_window().is_err());
    // 用户显式值优先，不受模型/地址限制，但必须在合理范围内。
    unknown.context_window = Some(200_000);
    assert_eq!(
        unknown.effective_context_window().unwrap(),
        (200_000, WindowSource::User)
    );
    for invalid in [0, 1_023, 10_000_001] {
        unknown.context_window = Some(invalid);
        let text = unknown.effective_context_window().unwrap_err().to_string();
        assert!(text.contains("between"), "{invalid}: {text}");
    }
    for valid in [MIN_CONTEXT_WINDOW_TOKENS, MAX_CONTEXT_WINDOW_TOKENS] {
        unknown.context_window = Some(valid);
        assert_eq!(
            unknown.effective_context_window().unwrap(),
            (valid, WindowSource::User)
        );
    }
}

#[test]
fn old_configs_without_a_context_window_still_load_and_new_ones_roundtrip() {
    let legacy: ProviderConfig = serde_json::from_value(
        json!({"base_url":"https://api.deepseek.com","model":"deepseek-flash"}),
    )
    .unwrap();
    assert_eq!(legacy.context_window, None);
    let explicit_null: ProviderConfig = serde_json::from_value(
        json!({"base_url":"https://api.deepseek.com","model":"deepseek-flash","context_window":null}),
    )
    .unwrap();
    assert_eq!(explicit_null.context_window, None);
    let configured: ProviderConfig = serde_json::from_value(
        json!({"base_url":"https://custom.example/v1","model":"local","context_window":32768}),
    )
    .unwrap();
    assert_eq!(configured.context_window, Some(32_768));
    let round: ProviderConfig =
        serde_json::from_str(&serde_json::to_string(&configured).unwrap()).unwrap();
    assert_eq!(round.context_window, Some(32_768));
    assert!(serde_json::to_string(&legacy)
        .unwrap()
        .contains("\"context_window\":null"));
}

#[test]
fn build_request_is_the_single_constructor_used_by_budget_and_transport() {
    let messages = [Message::User {
        content: "查一下这个算例".into(),
    }];
    let tools = [
        json!({"type":"function","function":{"name":"inspect","description":"Inspect","parameters":{"type":"object","properties":{}}}}),
    ];
    let mut chat = ProviderConfig::deepseek("deepseek-flash", "secret".into());
    chat.base_url = "https://api.deepseek.com".into();
    assert_eq!(
        build_request(&chat, &messages, &tools).unwrap(),
        request_body(&chat, &messages, &tools)
    );
    let mut responses = native_config(ApiFormat::Responses);
    responses.model = "future-model".into();
    assert_eq!(
        build_request(&responses, &messages, &tools).unwrap(),
        adapters::native_body(&responses, &messages, &tools).unwrap()
    );
    let mut anthropic = native_config(ApiFormat::Anthropic);
    anthropic.model = "future-model".into();
    assert_eq!(
        build_request(&anthropic, &messages, &tools).unwrap(),
        adapters::native_body(&anthropic, &messages, &tools).unwrap()
    );
}

#[test]
fn chat_transport_sends_exactly_the_budget_estimated_request_body() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut c = ProviderConfig::deepseek("deepseek-flash", "secret".into());
    c.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let messages = vec![Message::User {
        content: "查一下这个算例的产流".into(),
    }];
    let tools = vec![
        json!({"type":"function","function":{"name":"inspect","description":"Inspect a case","parameters":{"type":"object","properties":{"case":{"type":"string"}}}}}),
    ];
    // 预算估算走的就是这个构造：请求体字符数（含工具 schema 与中文）是估算口径。
    let expected = build_request(&c, &messages, &tools).unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                length = n.trim().parse::<usize>().unwrap();
            }
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).unwrap();
        let payload: Value = serde_json::from_slice(&bytes).unwrap();
        let body = stream(&[
            json!({"choices":[{"delta":{"content":"ok"}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        ]) + "data: [DONE]\n\n";
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        payload
    });
    let turn = OpenAiCompatible::new(c)
        .complete(&messages, &tools, &mut |_| {})
        .unwrap();
    assert_eq!(turn.content, "ok");
    assert_eq!(server.join().unwrap(), expected);
    // 工具 schema 与中文都进估算：不含工具时明显更短。
    let without_tools = build_request(
        &ProviderConfig::deepseek("deepseek-flash", "secret".into()),
        &messages,
        &[],
    )
    .unwrap();
    assert!(
        serde_json::to_string(&expected).unwrap().chars().count()
            > serde_json::to_string(&without_tools)
                .unwrap()
                .chars()
                .count()
    );
}

#[test]
fn effective_max_output_reserves_exactly_the_output_limit_in_the_body() {
    let chat = ProviderConfig::deepseek("deepseek-flash", "secret".into());
    let body = build_request(&chat, &[], &[]).unwrap();
    assert_eq!(
        effective_max_output(&body, ApiFormat::ChatCompletions).unwrap(),
        u64::from(chat.max_output_tokens)
    );
    // 单键（max_completion_tokens）也能作为有效输出上限。
    let body = json!({"max_completion_tokens": 4096});
    assert_eq!(
        effective_max_output(&body, ApiFormat::ChatCompletions).unwrap(),
        4_096
    );
    // 通过 api_options 覆盖 max_tokens：预留跟着最终 body 走。
    let mut overridden = chat.clone();
    overridden.api_options = json!({"max_tokens": 8192});
    let body = build_request(&overridden, &[], &[]).unwrap();
    assert_eq!(
        effective_max_output(&body, ApiFormat::ChatCompletions).unwrap(),
        8_192
    );
    // 两键并存但值相同是允许的：仍然只有一个有效预留值。
    let mut same = chat.clone();
    same.api_options = json!({"max_tokens": 8192, "max_completion_tokens": 8192});
    assert_eq!(
        effective_max_output(
            &build_request(&same, &[], &[]).unwrap(),
            ApiFormat::ChatCompletions
        )
        .unwrap(),
        8_192
    );
    // 两键并存且值不同：预算无法确定预留哪个，发送前报错。
    let mut conflict = chat.clone();
    conflict.api_options = json!({"max_completion_tokens": 4096});
    let error = effective_max_output(
        &build_request(&conflict, &[], &[]).unwrap(),
        ApiFormat::ChatCompletions,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("max_tokens"), "{error}");
    assert!(error.contains("max_completion_tokens"), "{error}");
    // 协议不认的输出键、非数字、0 都报错，而不是静默取一个值。
    let mut responses = native_config(ApiFormat::Responses);
    responses.model = "future-model".into();
    for forbidden in ["max_tokens", "max_completion_tokens"] {
        let mut forbidden_config = responses.clone();
        forbidden_config.api_options = Value::Object(serde_json::Map::from_iter([(
            forbidden.to_owned(),
            json!(100),
        )]));
        let body = build_request(&forbidden_config, &[], &[]).unwrap();
        let error = effective_max_output(&body, ApiFormat::Responses)
            .unwrap_err()
            .to_string();
        assert!(error.contains(forbidden), "{error}");
        assert!(error.contains("output token limit"), "{error}");
    }
    responses.api_options = json!({"max_output_tokens": 2048});
    assert_eq!(
        effective_max_output(
            &build_request(&responses, &[], &[]).unwrap(),
            ApiFormat::Responses
        )
        .unwrap(),
        2_048
    );
    let mut anthropic = native_config(ApiFormat::Anthropic);
    anthropic.model = "future-model".into();
    for forbidden in ["max_output_tokens", "max_completion_tokens"] {
        let mut forbidden_config = anthropic.clone();
        forbidden_config.api_options = Value::Object(serde_json::Map::from_iter([(
            forbidden.to_owned(),
            json!(100),
        )]));
        let body = build_request(&forbidden_config, &[], &[]).unwrap();
        let error = effective_max_output(&body, ApiFormat::Anthropic)
            .unwrap_err()
            .to_string();
        assert!(error.contains(forbidden), "{error}");
        assert!(error.contains("output token limit"), "{error}");
    }
    anthropic.api_options = json!({"max_tokens": 512});
    assert_eq!(
        effective_max_output(
            &build_request(&anthropic, &[], &[]).unwrap(),
            ApiFormat::Anthropic
        )
        .unwrap(),
        512
    );
    let mut invalid = chat.clone();
    invalid.api_options = json!({"max_tokens": "8192"});
    let body = build_request(&invalid, &[], &[]).unwrap();
    assert!(effective_max_output(&body, ApiFormat::ChatCompletions)
        .unwrap_err()
        .to_string()
        .contains("positive integer"));
    let mut zero = chat.clone();
    zero.api_options = json!({"max_tokens": 0});
    let body = build_request(&zero, &[], &[]).unwrap();
    assert!(effective_max_output(&body, ApiFormat::ChatCompletions)
        .unwrap_err()
        .to_string()
        .contains("positive integer"));
    let mut no_limit = chat;
    no_limit.max_output_tokens = 0;
    let body = build_request(&no_limit, &[], &[]).unwrap();
    assert!(effective_max_output(&body, ApiFormat::ChatCompletions)
        .unwrap_err()
        .to_string()
        .contains("positive integer"));
}

#[test]
fn provider_budget_limits_match_the_body_that_will_be_sent() {
    let c = ProviderConfig::deepseek("deepseek-flash", "secret".into());
    let provider = OpenAiCompatible::new(c.clone());
    let limits = provider.budget_limits().unwrap().unwrap();
    assert_eq!(limits.window_tokens, CONTEXT_WINDOW_TOKENS);
    assert_eq!(limits.source, WindowSource::Preset);
    assert_eq!(limits.safety_tokens, safety_margin(CONTEXT_WINDOW_TOKENS));
    let messages = [Message::User {
        content: "hi".into(),
    }];
    let tools = [
        json!({"type":"function","function":{"name":"inspect","description":"Inspect","parameters":{"type":"object","properties":{}}}}),
    ];
    let body = provider.request_body(&messages, &tools).unwrap().unwrap();
    assert_eq!(
        effective_max_output(&body, c.api_format).unwrap(),
        limits.output_tokens,
        "reserved output tokens must equal the effective body limit"
    );
    assert_eq!(body, build_request(&c, &messages, &tools).unwrap());
    // 未知窗口在预算解析阶段就报错（任何 HTTP 请求之前）。
    let mut unknown = c;
    unknown.model = "mystery-model".into();
    unknown.base_url = "https://custom.example/v1".into();
    unknown.provider_id = "custom".into();
    assert!(OpenAiCompatible::new(unknown).budget_limits().is_err());
    // 输出键冲突同样在预算解析阶段报错。
    let mut conflict = ProviderConfig::deepseek("deepseek-flash", "secret".into());
    conflict.api_options = json!({"max_completion_tokens": 4096});
    assert!(OpenAiCompatible::new(conflict)
        .budget_limits()
        .unwrap_err()
        .to_string()
        .contains("max_completion_tokens"));
}

#[test]
fn providers_without_budget_keep_the_old_default_methods() {
    struct Bare;
    impl Provider for Bare {
        fn complete(
            &self,
            _messages: &[Message],
            _tools: &[Value],
            _on_event: &mut dyn FnMut(StreamEvent),
        ) -> Result<Turn> {
            Ok(Turn::default())
        }
    }
    assert!(Bare.budget_limits().unwrap().is_none());
    assert!(Bare.request_body(&[], &[]).unwrap().is_none());
}
