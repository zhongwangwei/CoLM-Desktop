//! OpenCode's authenticated loopback server. Only the runtime OpenAPI contract is trusted.
use super::{
    child_path, find_cli, ExternalChoice, ExternalOutcome, ExternalSession, Launch, TurnSink,
};
use crate::{protocol::Outbound, tools::Tier};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const LIMIT: u64 = 8 * 1024 * 1024;
fn identifier(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty()
            && value.len() < 256
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "invalid OpenCode identifier"
    );
    Ok(value)
}
fn permission(_web: bool) -> Value {
    // V2 evaluates the last match, and configured deny cannot be overridden by saved allows.
    json!([{"action":"*","resource":"*","effect":"deny"},{"action":"colm_*","resource":"*","effect":"allow"}])
}

/// One persistent profile shared by discovery, login and conversations, never the user's OpenCode DB.
pub fn profile_dir(data_dir: Option<&Path>) -> Result<PathBuf> {
    let base = data_dir.context("OpenCode requires --data-dir for its isolated profile")?;
    let base = if base.is_absolute() {
        base.to_owned()
    } else {
        std::env::current_dir()?.join(base)
    };
    Ok(base.join("opencode-profile"))
}
fn profile_environment(profile: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("HOME", profile.join("home")),
        ("USERPROFILE", profile.join("home")),
        ("XDG_DATA_HOME", profile.join("data")),
        ("XDG_CONFIG_HOME", profile.join("config")),
        ("XDG_STATE_HOME", profile.join("state")),
        ("XDG_CACHE_HOME", profile.join("cache")),
        ("OPENCODE_DB", profile.join("opencode.db")),
    ]
}
fn create_private_profile(profile: &Path) -> Result<()> {
    let mut dirs = std::fs::DirBuilder::new();
    dirs.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        dirs.mode(0o700);
    }
    dirs.create(profile)?;
    for (key, path) in profile_environment(profile) {
        if key != "OPENCODE_DB" {
            dirs.create(path)?;
        }
    }
    dirs.create(profile.join("settings"))?;
    Ok(())
}
// Only inspect presence metadata; install guards solely in our dedicated profile DB.
// Never select credential records or stored values. Guards also block concurrent /connect writes.
fn check_profile_sources(profile: &Path) -> Result<()> {
    ensure!(
        !profile.join("data/opencode/auth.json").exists(),
        "OpenCode profile contains legacy authentication data; connect a provider in a fresh dedicated profile (organization configuration is unsupported)"
    );
    let database = profile.join("opencode.db");
    if !database.exists() {
        return Ok(());
    }
    const INSERT: &str = "CREATE TRIGGER colm_block_wellknown_insert BEFORE INSERT ON kv WHEN NEW.key = 'wellknown:sources' BEGIN SELECT RAISE(ABORT, 'organization configuration disabled by CoLM'); END";
    const UPDATE: &str = "CREATE TRIGGER colm_block_wellknown_update BEFORE UPDATE ON kv WHEN NEW.key = 'wellknown:sources' BEGIN SELECT RAISE(ABORT, 'organization configuration disabled by CoLM'); END";
    let quote = |s: &str| s.replace('\'', "''");
    let sql = format!(
        "BEGIN IMMEDIATE; SELECT EXISTS(SELECT 1 FROM kv WHERE key = 'wellknown:sources'); {}; {}; SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND sql IN ('{}','{}'); COMMIT;",
        INSERT.replace("CREATE TRIGGER ", "CREATE TRIGGER IF NOT EXISTS "),
        UPDATE.replace("CREATE TRIGGER ", "CREATE TRIGGER IF NOT EXISTS "),
        quote(INSERT), quote(UPDATE)
    );
    let output = Command::new("sqlite3")
        .args(["-batch", "-bail", "-noheader", "-cmd", ".timeout 2000"])
        .arg(&database)
        .arg(sql)
        .stdin(Stdio::null())
        .output()
        .context("OpenCode requires sqlite3 to verify its dedicated profile before startup")?;
    ensure!(
        output.status.success() && output.stdout == b"0\n2\n",
        "OpenCode profile cannot be verified or contains organization configuration; use a dedicated provider-only profile"
    );
    Ok(())
}
fn spawn_cli(command: &mut Command) -> std::io::Result<Child> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match command.spawn() {
            // Concurrent Linux execs can briefly inherit a writer before close-on-exec.
            Err(error)
                if cfg!(target_os = "linux")
                    && error.raw_os_error() == Some(26)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}
fn verify_cli_version(exe: &Path, profile: &Path) -> Result<()> {
    let mut command = Command::new(exe);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("OPENCODE_") {
            command.env_remove(key);
        }
    }
    command
        .arg("--version")
        .envs(profile_environment(profile))
        .env("PATH", child_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = spawn_cli(&mut command).context("Could not probe OpenCode V2 version")?;
    // GUI discovery and conversation startup may probe concurrently; cold CLI runtimes
    // need more than three seconds under load. Poll frequently so cleanup stays bounded.
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("OpenCode version probe timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut version = String::new();
    if let Some(stdout) = child.stdout.take() {
        stdout.take(256).read_to_string(&mut version)?;
    }
    ensure!(status.success() && version.trim() == "2.0.6", "OpenCode integration requires verified version 2.0.6; installed CLI failed its version probe or is unsupported");
    Ok(())
}
fn setup_command(exe: &Path, profile: &Path) -> String {
    let mut environment = profile_environment(profile);
    environment.push(("OPENCODE_CONFIG_DIR", profile.join("settings")));
    if cfg!(windows) {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let mut commands=vec!["Remove-Item Env:OPENCODE_CONFIG,Env:OPENCODE_CONFIG_CONTENT,Env:OPENCODE_PASSWORD,Env:OPENCODE_SERVER_PASSWORD -ErrorAction SilentlyContinue".to_owned()];
        commands.extend(
            environment
                .iter()
                .map(|(key, path)| format!("$env:{key} = {}", quote(&path.to_string_lossy()))),
        );
        commands.push("$env:OPENCODE_CONFIG_PROJECT_DISABLE = 'true'".to_owned());
        commands.push(format!("& {} --standalone", quote(&exe.to_string_lossy())));
        commands.join("; ")
    } else {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\"'\"'"));
        let mut args=vec!["env -u OPENCODE_CONFIG -u OPENCODE_CONFIG_CONTENT -u OPENCODE_PASSWORD -u OPENCODE_SERVER_PASSWORD -u OPENCODE_BIN_PATH".to_owned()];
        args.extend(
            environment
                .iter()
                .map(|(key, path)| format!("{key}={}", quote(&path.to_string_lossy()))),
        );
        args.push("OPENCODE_CONFIG_PROJECT_DISABLE=true".to_owned());
        args.push(quote(&exe.to_string_lossy()));
        args.push("--standalone".to_owned());
        args.join(" ")
    }
}
fn configuration(launch: Option<&Launch>) -> Value {
    let rules = permission(launch.is_some_and(|l| l.web));
    let mut config = json!({"update":"disable","share":"disabled","snapshots":false,"formatter":false,"lsp":false,"warming":false,"permissions":rules,"agents":{"colm":{"mode":"primary","system":launch.map(|l|format!("{}\nUse only colm MCP tools in this integration. Native shell, file, web and subagent tools are disabled.",l.instructions)).unwrap_or_else(||"CoLM Desktop model discovery".to_owned()),"permissions":rules}}});
    if let Some(l) = launch {
        config["mcp"] = json!({"servers":{"colm":{"type":"local","command":[l.mcp_exe],"cwd":l.cwd,"environment":{crate::bridge::ENV_ADDR:l.bridge_addr,crate::bridge::ENV_TOKEN:l.bridge_token},"codemode":false,"protocol":"legacy"}}});
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_default();
        for (key, fallback) in [
            ("HOME", home.clone()),
            ("USERPROFILE", home.clone()),
            ("XDG_DATA_HOME", home.join(".local/share")),
            ("XDG_CONFIG_HOME", home.join(".config")),
            ("XDG_STATE_HOME", home.join(".local/state")),
            ("XDG_CACHE_HOME", home.join(".cache")),
        ] {
            let value = std::env::var_os(key).map(PathBuf::from).unwrap_or(fallback);
            config["mcp"]["servers"]["colm"]["environment"][key] = json!(value);
        }
    }
    config
}

struct Server {
    child: Child,
    root: PathBuf,
    url: String,
    client: ureq::Agent,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.child.stdin.take();
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Server {
    fn start(exe: &Path, profile: &Path, launch: Option<&Launch>) -> Result<Self> {
        create_private_profile(profile)?;
        verify_cli_version(exe, profile)?;
        check_profile_sources(profile)?;
        let root =
            std::env::temp_dir().join(format!("colm-opencode-{}", crate::bridge::random_hex(16)));
        // 只有 Unix 要设权限位；Windows 上它不必可变。
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut dir = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dir.mode(0o700);
        }
        dir.create(&root)?;
        let result = Self::spawn(exe, launch, &root, profile);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&root);
        }
        result
    }
    fn spawn(exe: &Path, launch: Option<&Launch>, root: &Path, profile: &Path) -> Result<Self> {
        let password = crate::bridge::random_hex(32);
        let config = root.join("opencode.json");
        std::fs::write(&config, configuration(launch).to_string())?;
        let mut command = Command::new(exe);
        // Prevent inherited flags from selecting a shared service, extra config, or simulated output.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("OPENCODE_") {
                command.env_remove(key);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command
            .current_dir(root)
            .envs(profile_environment(profile))
            .env("PATH", child_path())
            .env("OPENCODE_CONFIG_DIR", root)
            .env("OPENCODE_CONFIG_PROJECT_DISABLE", "true")
            .env("OPENCODE_DISABLE_MODELS_FETCH", "true")
            .env("OPENCODE_FILEWATCHER_DISABLE", "true")
            .env("OPENCODE_PASSWORD", &password)
            .args(["serve", "--stdio", "--hostname", "127.0.0.1", "--port", "0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = spawn_cli(&mut command).context("cannot start OpenCode V2")?;
        let client = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(2)))
            .timeout_recv_response(Some(Duration::from_secs(5)))
            .timeout_recv_body(Some(Duration::from_secs(15)))
            .build()
            .into();
        let mut server = Self {
            child,
            root: root.to_owned(),
            url: String::new(),
            client,
        };
        let stdout = server
            .child
            .stdout
            .take()
            .context("OpenCode did not expose stdio")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let read = BufReader::new(stdout).take(4097).read_line(&mut line);
            let _ = tx.send(if read.is_ok() && line.len() <= 4096 {
                serde_json::from_str::<Value>(&line).ok()
            } else {
                None
            });
        });
        let address = rx
            .recv_timeout(Duration::from_secs(15))
            .ok()
            .flatten()
            .context(
            "OpenCode V2 server did not publish a stdio address; check the installed executable",
        )?;
        let url = address["url"]
            .as_str()
            .context("invalid OpenCode V2 stdio address")?;
        let port = url
            .strip_prefix("http://127.0.0.1:")
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
            .filter(|p| *p > 0)
            .context("OpenCode must listen on IPv4 loopback")?;
        server.url = format!("http://opencode:{password}@127.0.0.1:{port}");
        let info = server.get("/api/info")?;
        let version = info["version"]
            .as_str()
            .context("OpenCode V2 version is missing")?;
        ensure!(
            version == "2.0.6",
            "OpenCode server version must match verified version 2.0.6"
        );
        ensure!(profile.join("opencode.db").exists(), "OpenCode profile database is not initialized; first connect a provider with the setup command");
        check_profile_sources(profile)?;
        let response = server
            .client
            .get(format!("http://127.0.0.1:{port}/api/model"))
            .call()
            .map_err(|_| anyhow::anyhow!("OpenCode authentication check failed"))?;
        ensure!(
            response.status().as_u16() == 401,
            "OpenCode server did not enforce authentication"
        );
        validate_contract(&server.get("/openapi.json")?)?;
        Ok(server)
    }
    fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .client
            .get(format!("{}{path}", self.url))
            .call()
            .map_err(|_| anyhow::anyhow!("OpenCode local request failed"))?;
        decode(response)
    }
    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("{}{path}", self.url))
            .send_json(body)
            .map_err(|_| anyhow::anyhow!("OpenCode local request failed"))?;
        decode(response)
    }
    fn events(&self) -> Result<mpsc::Receiver<Result<Value>>> {
        let response = self
            .client
            .get(format!("{}/api/event", self.url))
            .config()
            .timeout_recv_body(None)
            .build()
            .call()
            .map_err(|_| anyhow::anyhow!("OpenCode event connection failed"))?;
        ensure!(
            response.status().is_success(),
            "OpenCode event connection rejected"
        );
        let (tx, rx) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(response.into_body().into_reader());
            loop {
                let mut line = String::new();
                match reader.by_ref().take(LIMIT + 1).read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) if line.len() as u64 > LIMIT => {
                        let _ = tx.send(Err(anyhow::anyhow!("OpenCode event exceeds size limit")));
                        break;
                    }
                    Ok(_) => {
                        if let Some(data) = line.strip_prefix("data:") {
                            let event =
                                serde_json::from_str(data.trim()).context("invalid OpenCode event");
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(Err(anyhow::anyhow!("OpenCode event stream disconnected")));
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }
}
fn decode(response: ureq::http::Response<ureq::Body>) -> Result<Value> {
    ensure!(
        response.status().is_success(),
        "OpenCode local request rejected (HTTP {})",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("OpenCode response read failed"))?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "OpenCode response exceeds size limit"
    );
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).context("invalid OpenCode response")
}
fn validate_contract(doc: &Value) -> Result<()> {
    for (path, method) in [
        ("/api/session", "post"),
        ("/api/session/{sessionID}", "patch"),
        ("/api/session/{sessionID}/prompt", "post"),
        ("/api/session/{sessionID}/interrupt", "post"),
        ("/api/session/{sessionID}/message", "get"),
        ("/api/session/{sessionID}/permission", "get"),
        (
            "/api/session/{sessionID}/permission/{requestID}/reply",
            "post",
        ),
        ("/api/session/{sessionID}/model", "post"),
        ("/api/session/{sessionID}/agent", "post"),
        ("/api/event", "get"),
        ("/api/model", "get"),
    ] {
        ensure!(
            doc["paths"][path][method].is_object(),
            "unsupported OpenCode V2 server contract: {method} {path}"
        );
    }
    let schema = &doc["components"]["schemas"]["Permission.Reply"]["enum"];
    ensure!(
        schema
            .as_array()
            .is_some_and(|v| ["once", "reject"].iter().all(|s| v.contains(&json!(s)))),
        "unsupported OpenCode V2 permission schema"
    );
    let body = &doc["paths"]["/api/session/{sessionID}/permission/{requestID}/reply"]["post"]
        ["requestBody"]["content"]["application/json"]["schema"]["properties"];
    ensure!(
        body.get("decision").is_some(),
        "unsupported OpenCode V2 permission decision contract"
    );
    Ok(())
}
fn model_list(value: &Value) -> Value {
    let mut list = Vec::new();
    for m in value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["enabled"] == true && m["status"] != "deprecated")
    {
        let (Some(provider), Some(id)) = (m["providerID"].as_str(), m["id"].as_str()) else {
            continue;
        };
        let efforts: Vec<_> = m["variants"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["id"].as_str())
            .collect();
        list.push(json!({"id":format!("{provider}/{id}"),"name":m["name"].as_str().unwrap_or(id),"efforts":efforts}));
    }
    list.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    json!(list)
}
pub fn list_models(profile: &Path) -> Result<Value> {
    let exe = find_cli("opencode").context("OpenCode V2 is not installed")?;
    let server = Server::start(&exe, profile, None)?;
    Ok(model_list(&server.get("/api/model")?))
}
pub fn status(profile: &Path) -> Value {
    let exe = find_cli("opencode");
    let mut result = if let Some(exe) = &exe {
        match Server::start(exe, profile, None) {
            Ok(server) => match server.get("/api/model") {
                Ok(models) => {
                    json!({"installed":true,"path":exe,"version":server.get("/api/info").ok().map(|v|v["version"].clone()),"configured":model_list(&models).as_array().is_some_and(|m|!m.is_empty())})
                }
                Err(_) => {
                    json!({"installed":true,"configured":false,"error":"OpenCode V2 model discovery failed"})
                }
            },
            Err(error) => json!({"installed":true,"configured":false,"error":error.to_string()}),
        }
    } else {
        json!({"installed":false,"configured":false})
    };
    result["profile_path"] = json!(profile);
    result["setup_command"] = json!(setup_command(
        exe.as_deref().unwrap_or(Path::new("opencode")),
        profile
    ));
    result
}

pub struct OpenCodeSession {
    launch: Launch,
    profile: PathBuf,
    session_id: Option<String>,
    choice: ExternalChoice,
    server: Option<Server>,
    events: Option<mpsc::Receiver<Result<Value>>>,
}
impl OpenCodeSession {
    pub fn new(launch: Launch, resume: Option<String>, profile: PathBuf) -> Self {
        Self {
            launch,
            profile,
            session_id: resume,
            choice: ExternalChoice::default(),
            server: None,
            events: None,
        }
    }
    fn connect(&mut self) -> Result<()> {
        if self.server.is_some() {
            return Ok(());
        }
        let exe = find_cli("opencode").context("OpenCode V2 is not installed")?;
        self.connect_with(&exe)
    }
    fn connect_with(&mut self, exe: &Path) -> Result<()> {
        let server = Server::start(exe, &self.profile, Some(&self.launch))?;
        let session = if let Some(id) = &self.session_id {
            identifier(id)?;
            server.get(&format!("/api/session/{id}"))?["data"].clone()
        } else {
            server.post("/api/session",&json!({"title":"CoLM-Desktop","agent":"colm","location":{"directory":self.launch.cwd},"permissions":permission(self.launch.web)}))?["data"].clone()
        };
        let id = identifier(
            session["id"]
                .as_str()
                .context("OpenCode V2 did not return a session id")?,
        )?
        .to_owned();
        let expected_cwd =
            std::fs::canonicalize(&self.launch.cwd).context("OpenCode workspace is unavailable")?;
        ensure!(
            session["location"]["directory"]
                .as_str()
                .and_then(|p| std::fs::canonicalize(p).ok())
                == Some(expected_cwd),
            "OpenCode session belongs to a different workspace; start a new conversation"
        );
        let rules = permission(self.launch.web);
        let response = server
            .client
            .patch(format!("{}/api/session/{id}", server.url))
            .send_json(json!({"permissions":rules}))
            .map_err(|_| anyhow::anyhow!("OpenCode session permission reset failed"))?;
        decode(response)?;
        let actual = server.get(&format!("/api/session/{id}"))?;
        ensure!(
            actual["data"]["permissions"] == rules,
            "OpenCode did not apply the session permission rules"
        );
        server.post(
            &format!("/api/session/{id}/agent"),
            &json!({"agent":"colm"}),
        )?;
        self.session_id = Some(id);
        self.events = Some(server.events()?);
        self.server = Some(server);
        Ok(())
    }
    fn run(
        &mut self,
        text: &str,
        sink: &mut dyn TurnSink,
        cancel: &AtomicBool,
    ) -> Result<ExternalOutcome> {
        ensure!(!cancel.load(Ordering::SeqCst), "cancelled");
        let selected_id = self
            .choice
            .model
            .as_deref()
            .context("select an OpenCode provider/model first")?;
        let (provider, id) = selected_id
            .split_once('/')
            .filter(|(p, m)| !p.is_empty() && !m.is_empty())
            .context("OpenCode model must be provider/model")?;
        let mut model = json!({"providerID":provider,"id":id});
        if let Some(variant) = &self.choice.effort {
            model["variant"] = json!(variant);
        }
        self.connect()?;
        ensure!(!cancel.load(Ordering::SeqCst), "cancelled");
        let server = self.server.as_ref().unwrap();
        let id = self.session_id.as_deref().unwrap();
        let events = self.events.as_ref().unwrap();
        loop {
            match events.try_recv() {
                Ok(event) => {
                    event?;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(_) => bail!("OpenCode event stream disconnected before prompt"),
            }
        }
        let models = model_list(&server.get("/api/model")?);
        let selected = models
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == self.choice.model.as_deref().unwrap_or(""))
            .context("selected OpenCode model is not enabled")?;
        if let Some(effort) = &self.choice.effort {
            ensure!(
                selected["efforts"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(effort))),
                "selected OpenCode thinking variant is unavailable"
            );
        }
        server.post(&format!("/api/session/{id}/model"), &json!({"model":model}))?;
        let message_id = format!("msg_{}", crate::bridge::random_hex(24));
        ensure!(!cancel.load(Ordering::SeqCst), "cancelled");
        let queued = server.post(
            &format!("/api/session/{id}/prompt"),
            &json!({"id":message_id,"text":text,"delivery":"queue"}),
        )?;
        ensure!(
            queued["data"]["id"] == message_id,
            "OpenCode did not acknowledge the requested message"
        );
        let start = Instant::now();
        let mut state = TurnState::default();
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = server.post(&format!("/api/session/{id}/interrupt"), &json!({}));
                bail!("cancelled");
            }
            ensure!(
                start.elapsed() < Duration::from_secs(1800),
                "OpenCode turn timed out"
            );
            // SSE is volatile in V2: reconcile permissions and projected messages, not only deltas.
            match events.recv_timeout(Duration::from_millis(250)) {
                Ok(event) => {
                    event?;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => bail!("OpenCode event stream disconnected"),
            }
            let requests = server.get(&format!("/api/session/{id}/permission"))?;
            for p in requests["data"]
                .as_array()
                .context("invalid OpenCode permission response")?
            {
                ensure!(p["sessionID"] == id, "OpenCode permission session mismatch");
                let request =
                    identifier(p["id"].as_str().context("OpenCode permission has no id")?)?;
                let name = p["action"]
                    .as_str()
                    .context("OpenCode permission has no action")?;
                // Native tools stay denied; CoLM requests still pass through the shared MCP approval bridge.
                let reply = if !cancel.load(Ordering::SeqCst) && name.starts_with("colm_") {
                    "once"
                } else {
                    "reject"
                };
                server.post(
                    &format!("/api/session/{id}/permission/{request}/reply"),
                    &json!({"decision":reply}),
                )?;
            }
            let messages = current_messages(server, id, &message_id)?;
            for message in &messages {
                state.message(message, sink);
            }
            if let Some(out) = final_answer(&messages)? {
                return Ok(out);
            }
        }
    }
}
impl ExternalSession for OpenCodeSession {
    fn turn(
        &mut self,
        text: &str,
        sink: &mut dyn TurnSink,
        cancel: &AtomicBool,
    ) -> Result<ExternalOutcome> {
        let result = self.run(text, sink, cancel);
        if result.is_err() {
            if let (Some(s), Some(id)) = (&self.server, &self.session_id) {
                let _ = s.post(&format!("/api/session/{id}/interrupt"), &json!({}));
            }
            self.events = None;
            self.server = None;
        }
        result
    }
    fn set_choice(&mut self, choice: ExternalChoice) {
        let choice = choice.cleaned();
        self.choice = choice;
    }
    fn resume_id(&self) -> Option<String> {
        self.session_id.clone()
    }
}
fn current_messages(server: &Server, id: &str, user_id: &str) -> Result<Vec<Value>> {
    let mut path = format!("/api/session/{id}/message?limit=100&order=desc");
    let mut out = Vec::new();
    for _ in 0..10 {
        let response = server.get(&path)?;
        for message in response["data"]
            .as_array()
            .context("invalid OpenCode messages")?
        {
            if message["id"] == user_id {
                ensure!(
                    !out.iter().any(|m: &Value| m["type"] == "user"),
                    "OpenCode session received another concurrent user message"
                );
                out.reverse();
                return Ok(out);
            }
            out.push(message.clone());
        }
        let Some(cursor) = response["cursor"]["next"].as_str() else {
            return Ok(Vec::new());
        };
        let encoded: String = cursor.bytes().map(|b| format!("%{b:02X}")).collect();
        path = format!("/api/session/{id}/message?limit=100&cursor={encoded}");
    }
    bail!("OpenCode turn exceeds the message reconciliation limit")
}
#[derive(Default)]
struct TurnState {
    texts: HashMap<String, String>,
    tools: HashMap<String, bool>,
}
impl TurnState {
    fn message(&mut self, message: &Value, sink: &mut dyn TurnSink) {
        if message["type"] != "assistant" {
            return;
        }
        for (index, part) in message["content"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let kind = part["type"].as_str().unwrap_or("");
            if matches!(kind, "text" | "reasoning") {
                let key = format!("{}:{index}", message["id"]);
                let entry = self.texts.entry(key).or_default();
                let text = part["text"].as_str().unwrap_or("");
                let delta = text.strip_prefix(entry.as_str()).unwrap_or("").to_owned();
                *entry = text.to_owned();
                if !delta.is_empty() {
                    sink.emit(if kind == "reasoning" {
                        Outbound::ReasoningDelta { text: delta }
                    } else {
                        Outbound::AssistantDelta { text: delta }
                    });
                }
            } else if kind == "tool" {
                let name = part["name"].as_str().unwrap_or("unknown");
                if name.starts_with("colm_") {
                    continue;
                }
                let id = part["id"].as_str().unwrap_or("").to_owned();
                let state = &part["state"];
                if state["status"] == "streaming" {
                    continue;
                }
                let done = self.tools.entry(id.clone()).or_insert_with(|| {
                    sink.emit(Outbound::ToolCall {
                        id: id.clone(),
                        name: name.to_owned(),
                        arguments: state["input"].to_string(),
                        tier: Tier::Code,
                        summary: format!("OpenCode {name}"),
                        preapproved: false,
                    });
                    false
                });
                if !*done && matches!(state["status"].as_str(), Some("completed" | "error")) {
                    *done = true;
                    sink.emit(Outbound::ToolResult {
                        id,
                        name: name.to_owned(),
                        ok: state["status"] == "completed",
                        result: if state["status"] == "error" {
                            state["error"].to_string()
                        } else {
                            state["content"].to_string()
                        },
                        elapsed_ms: 0,
                    });
                }
            }
        }
    }
}
fn final_answer(messages: &[Value]) -> Result<Option<ExternalOutcome>> {
    let Some(idle) = messages.iter().rev().find(|m| m["type"] == "idle") else {
        return Ok(None);
    };
    if !idle["error"].is_null() {
        bail!("{}", provider_error(&idle["error"]));
    }
    ensure!(
        idle["outcome"] == "succeeded",
        "OpenCode turn failed or was interrupted"
    );
    let answers: Vec<_> = messages
        .iter()
        .filter(|m| m["type"] == "assistant")
        .collect();
    let last = answers
        .last()
        .context("OpenCode stopped without an answer")?;
    if let Some(failed) = answers.iter().find(|m| !m["error"].is_null()) {
        bail!("{}", provider_error(&failed["error"]));
    }
    ensure!(last["finish"]=="stop", "OpenCode response did not finish successfully (output limit, filtering, or unfinished tool call)");
    let mut out = ExternalOutcome::default();
    for m in &answers {
        let t = &m["tokens"];
        out.usage.prompt_tokens += t["input"].as_u64().unwrap_or(0)
            + t["cache"]["read"].as_u64().unwrap_or(0)
            + t["cache"]["write"].as_u64().unwrap_or(0);
        out.usage.completion_tokens +=
            t["output"].as_u64().unwrap_or(0) + t["reasoning"].as_u64().unwrap_or(0);
    }
    out.content = last["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(out))
}
fn provider_error(error: &Value) -> String {
    // Provider messages/bodies can echo keys and URLs. Retain only a bounded numeric status.
    let status = error["status"].as_u64().filter(|s| (100..=599).contains(s));
    let hint = match status {
        Some(401 | 403) => "check the provider login and model access in OpenCode",
        Some(429) => "provider rate limit or quota exceeded; check the provider account",
        Some(500..=599) => "provider service unavailable; retry later",
        _ => "inspect the provider error in the official OpenCode client",
    };
    match status {
        Some(code) => format!("OpenCode provider error (HTTP {code}): {hint}"),
        None => format!("OpenCode provider error: {hint}"),
    }
}

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod tests;
