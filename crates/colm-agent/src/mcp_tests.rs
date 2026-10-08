use super::*;

struct Fake;

impl ToolHost for Fake {
    fn list(&self) -> Result<Vec<Value>> {
        Ok(vec![tool_entry(
            "list_cases",
            "list",
            json!({ "type": "object" }),
        )])
    }
    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)> {
        match name {
            "list_cases" => Ok((false, format!("cases under {}", arguments["root"]))),
            "broken" => anyhow::bail!("tool exploded"),
            _ => Ok((true, format!("error: there is no tool named {name}"))),
        }
    }
}

#[test]
fn initialize_negotiates_a_supported_version() {
    let answer = handle(
        &Fake,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }),
    )
    .unwrap();
    assert_eq!(answer["id"], 1);
    assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(answer["result"]["serverInfo"]["name"], "colm");
    assert!(answer["result"]["capabilities"]["tools"].is_object());
    // 不认识的版本回最新的。
    let newer = handle(
        &Fake,
        &json!({ "id": "a", "method": "initialize", "params": { "protocolVersion": "2099-01-01" } }),
    )
    .unwrap();
    assert_eq!(newer["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
    // 旧一代的结果不带新一代字段。
    assert!(answer["result"].get("resultType").is_none());
    // 通知不回话。
    assert!(handle(&Fake, &json!({ "method": "notifications/initialized" })).is_none());
}

#[test]
fn tools_are_listed_and_called_and_failures_become_results() {
    let list = handle(&Fake, &json!({ "id": 2, "method": "tools/list" })).unwrap();
    assert_eq!(list["result"]["tools"][0]["name"], "list_cases");
    let call = handle(
        &Fake,
        &json!({ "id": 3, "method": "tools/call", "params": { "name": "list_cases", "arguments": { "root": "/p" } } }),
    )
    .unwrap();
    assert_eq!(call["result"]["isError"], false);
    assert_eq!(call["result"]["content"][0]["text"], "cases under \"/p\"");
    let broken = handle(
        &Fake,
        &json!({ "id": 4, "method": "tools/call", "params": { "name": "broken" } }),
    )
    .unwrap();
    assert_eq!(broken["result"]["isError"], true);
    assert!(broken["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("tool exploded"));
    let unknown = handle(&Fake, &json!({ "id": 5, "method": "resources/list" })).unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
    assert_eq!(
        handle(&Fake, &json!({ "id": 6, "method": "ping" })).unwrap()["result"],
        json!({})
    );
}

fn modern(id: i64, method: &str, extra: Value) -> Value {
    let mut params = json!({ "_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "claude-code", "version": "2.1.293" },
    } });
    if let Value::Object(more) = extra {
        for (k, v) in more {
            params[k] = v;
        }
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[test]
fn modern_requests_get_stateless_results_with_the_required_fields() {
    let discover = handle(
        &Fake,
        &json!({ "id": "p", "method": "server/discover", "params": { "_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } }),
    )
    .unwrap();
    let result = &discover["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"][0], "2026-07-28");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "colm"
    );
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["ttlMs"].is_u64() && result["cacheScope"] == "private");

    let list = handle(&Fake, &modern(1, "tools/list", json!({}))).unwrap();
    assert_eq!(list["result"]["resultType"], "complete");
    assert_eq!(list["result"]["cacheScope"], "private");
    assert!(list["result"]["ttlMs"].is_u64());
    assert_eq!(list["result"]["tools"][0]["name"], "list_cases");

    let call = handle(
        &Fake,
        &modern(
            2,
            "tools/call",
            json!({ "name": "list_cases", "arguments": { "root": "/p" } }),
        ),
    )
    .unwrap();
    assert_eq!(call["result"]["resultType"], "complete");
    assert_eq!(call["result"]["isError"], false);

    // 不支持的新版本：规范规定的错误，列出支持的版本。
    let mut future = modern(3, "tools/list", json!({}));
    future["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
    let error = handle(&Fake, &future).unwrap();
    assert_eq!(error["error"]["code"], -32022);
    assert_eq!(error["error"]["data"]["supported"][0], "2026-07-28");
    assert_eq!(error["error"]["data"]["requested"], "2099-01-01");
}
