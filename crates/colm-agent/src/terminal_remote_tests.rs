use super::*;

#[test]
fn headless_remote_capability_does_not_enable_window_tools() {
    let registry = crate::tools::Registry::standard_for(false, false, true);
    assert!(registry.find("remote_workspace_submit").is_some());
    assert!(registry.find("create_case").is_some());
    assert!(!registry.tools().any(|t| t.name().starts_with("ui_")));
    assert!(crate::tools::Registry::standard_with(false, false)
        .find("remote_workspace_submit")
        .is_none());
}

#[test]
fn rejects_secrets_and_invalid_connection_fields() {
    assert!(reject_secrets(&json!({"servers":[{"password":"secret"}]})).is_err());
    for server in [
        json!({"host":"-oProxyCommand=x","root":"/safe"}),
        json!({"host":"host","root":"/"}),
        json!({"host":"host","root":"/safe","port":70000}),
        json!({"host":"host","root":"/safe","username":"bad user"}),
    ] {
        assert!(validate_server(&server).is_err());
    }
}

#[test]
fn redacts_escaped_passwords_in_json_values_and_keys() {
    let secret = "quote\"\\秘密";
    let mut value = json!({secret:[format!("prefix {secret} suffix")]});
    redact_value(&mut value, secret);
    assert_eq!(value, json!({"[redacted]":["prefix [redacted] suffix"]}));
}

#[cfg(unix)]
#[test]
fn fake_cli_receives_password_only_on_stdin_and_saved_identity_wins() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!(
        "colm-terminal-remote-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("cli");
    std::fs::write(&cli,r#"#!/usr/bin/env python3
import sys,json,os
args=sys.argv[1:]
if args[args.index('--operation')+1]=='list':
 print(json.dumps({'jobs':[{'id':'job1','host':'saved','root':'/saved','connection':{'host':'saved','username':'olduser','port':2222,'auth':'password','identity_file':''}}]}))
else:
 payload=json.loads(sys.stdin.readline())
 assert os.environ['COLM_SSH_AUTH_STDIN']=='1'
 assert payload['password']=='unique_secret'
 assert 'unique_secret' not in str(args)
 if args[args.index('--operation')+1]=='cancel':
  print('unique_secret',file=sys.stderr);sys.exit(1)
 print(json.dumps({'args':args,'host':payload['host'],'username':payload['username'],'password_echo':payload['password']}))
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = dir.join("servers.json");
    std::fs::write(
        &config,
        r#"{"servers":[{"host":"newhost","root":"/new","auth":"password","username":"newuser"}]}"#,
    )
    .unwrap();
    let bridge = TerminalRemote::new(config, cli).unwrap();
    let resources = bridge
        .handle("workspace_remote_servers", json!({}), |_| {
            panic!("no password for listing")
        })
        .unwrap();
    assert_eq!(resources["servers"][0]["host"], "newhost");
    assert!(resources["servers"][0].get("username").is_none());
    let submission = bridge.handle("workspace_remote",json!({"name":"work","operation":"submit","host":"newhost","root":"/model-override","request":{"action":"test","package":"colm-core"}}), |_| Ok("unique_secret".into())).unwrap();
    let knowledge=bridge.handle("workspace_remote",json!({"name":"","operation":"knowledge","host":"newhost","request":{"source":"/server/src","query":"","paths":["crates/a.rs"]}}), |_| Ok("unique_secret".into())).unwrap();
    assert_eq!(knowledge["host"], "newhost");
    assert!(knowledge["args"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "knowledge"));
    assert!(bridge
        .handle(
            "workspace_remote",
            json!({"name":"","operation":"knowledge","host":"unknown","request":{}}),
            |_| panic!("unconfigured server must fail before credentials")
        )
        .is_err());
    let argv = submission["args"].as_array().unwrap();
    let root_at = argv.iter().position(|v| v == "--remote-root").unwrap();
    assert_eq!(argv[root_at + 1], "/new");
    assert!(!argv.iter().any(|v| v == "/model-override"));
    let result = bridge
        .handle(
            "workspace_remote",
            json!({"name":"work","operation":"status","job":"job1"}),
            |_| Ok("unique_secret".into()),
        )
        .unwrap();
    assert_eq!(result["host"], "saved");
    assert_eq!(result["username"], "olduser");
    assert_eq!(result["password_echo"], "[redacted]");
    let error = bridge
        .handle(
            "workspace_remote",
            json!({"name":"work","operation":"cancel","job":"job1"}),
            |_| Ok("unique_secret".into()),
        )
        .unwrap_err()
        .to_string();
    assert!(!error.contains("unique_secret"));
    assert!(bridge
        .handle(
            "workspace_remote",
            json!({"name":"../escape","operation":"list"}),
            |_| panic!()
        )
        .is_err());
    assert!(bridge
        .handle(
            "workspace_remote",
            json!({"name":"work","operation":"execute"}),
            |_| panic!()
        )
        .is_err());
    assert!(bridge.handle("workspace_remote",json!({"name":"work","operation":"submit","host":"unconfigured","request":{"action":"test"}}), |_| panic!()).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn cancellation_stops_cli_and_descendants_even_with_full_pipes() {
    use std::{
        os::unix::fs::PermissionsExt,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let dir = std::env::temp_dir().join(format!(
        "colm-remote-cancel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let cli = dir.join("cli");
    std::fs::write(&cli,r#"#!/usr/bin/env python3
import sys,subprocess,time,pathlib
root=pathlib.Path(__file__).parent
subprocess.Popen([sys.executable,'-c','import time,pathlib;time.sleep(1.5);pathlib.Path('+repr(str(root/'escaped'))+').write_text("alive")'])
sys.stdout.write('o'*100000);sys.stdout.flush()
sys.stderr.write('e'*100000);sys.stderr.flush()
(root/'ready').write_text('ready')
time.sleep(30)
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = dir.join("servers.json");
    std::fs::write(&config, "{\"servers\":[]}").unwrap();
    let bridge = TerminalRemote::new(config, cli).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let token = cancel.clone();
    let worker = std::thread::spawn(move || {
        bridge.handle_with_cancel(
            "workspace_remote",
            json!({"name":"work","operation":"list"}),
            |_| panic!("no prompt"),
            &token,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.join("ready").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let ready = dir.join("ready").exists();
    cancel.store(true, Ordering::SeqCst);
    let result = worker.join().unwrap();
    assert!(ready, "stdout/stderr drain failed");
    assert!(result.unwrap_err().to_string().contains("interrupted"));
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !dir.join("escaped").exists(),
        "CLI descendant survived cancellation"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cancelled_operation_never_prompts_or_spawns() {
    let bridge = TerminalRemote {
        servers: vec![json!({"host":"test","root":"/safe","auth":"password"})],
        cli: PathBuf::from("must-not-exist"),
        root: None,
    };
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let error = bridge
        .handle_with_cancel(
            "workspace_remote",
            json!({"name":"work","operation":"submit","host":"test","request":{"action":"test"}}),
            |_| panic!("cancelled password prompt"),
            &cancel,
        )
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
}
