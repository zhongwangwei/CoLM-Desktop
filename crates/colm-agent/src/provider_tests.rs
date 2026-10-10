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
    c.thinking = Some(false);
    c.reasoning_effort = Some("max".into());
    let body = request_body(&c, &[], &[]);
    assert!(body.get("thinking").is_none());
    assert_eq!(body["reasoning_effort"], "max");
    c.model = "glm-4.7".into();
    let body = request_body(&c, &[], &[]);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert!(body.get("reasoning_effort").is_none());
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
