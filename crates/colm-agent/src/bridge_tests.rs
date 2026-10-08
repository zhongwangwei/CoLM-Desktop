use super::*;

struct Echo;

impl BridgeHandler for Echo {
    fn list(&self) -> Result<Vec<Value>> {
        Ok(vec![json!({ "name": "metrics" })])
    }
    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)> {
        if name == "fail" {
            bail!("no turn is in progress");
        }
        Ok((false, format!("{name}:{arguments}")))
    }
}

#[test]
fn the_bridge_forwards_with_the_token_and_refuses_without_it() {
    let server = serve(Arc::new(Echo)).unwrap();
    assert!(server.addr.ip().is_loopback());
    assert_eq!(server.token.len(), 32);
    let client = BridgeClient::connect(&server.addr.to_string(), &server.token).unwrap();
    assert_eq!(client.list().unwrap()[0]["name"], "metrics");
    assert_eq!(
        client.call("run_status", json!({ "case": "/c" })).unwrap(),
        (false, r#"run_status:{"case":"/c"}"#.to_owned())
    );
    // agent 那边的错误原样带回。
    assert!(client
        .call("fail", json!({}))
        .unwrap_err()
        .to_string()
        .contains("no turn"));
    // 令牌不对：拒绝并断开。
    let intruder = BridgeClient::connect(&server.addr.to_string(), "wrong").unwrap();
    assert!(intruder
        .list()
        .unwrap_err()
        .to_string()
        .contains("bad token"));
    assert!(intruder.list().is_err());
    // 每次生成的令牌不同。
    assert_ne!(serve(Arc::new(Echo)).unwrap().token, server.token);
}
