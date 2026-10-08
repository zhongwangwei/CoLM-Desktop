//! `colm-agent`：GUI 经 stdio JSONL 驱动的 AI 助手进程（docs/design-ai-assistant.md 第 2 节）。
//!
//! ```text
//! colm-agent --data-dir <目录> --cli <colm-cli 路径> --key-file <文件>   # 服务模式：stdin 收 Inbound，stdout 发 Outbound
//! colm-agent --key-file <文件> --set-key <服务地址>       # 从 stdin 读一行 Key，存进本地 Key 文件
//! colm-agent --key-file <文件> --has-key <服务地址>       # 打印 true / false
//! colm-agent --key-file <文件> --delete-key <服务地址>
//! colm-agent --data-dir <目录> --list-sessions            # 历史会话列表（JSON）
//! colm-agent --data-dir <目录> --transcript <会话号>      # 一个会话的对话记录（JSON）
//! colm-agent --data-dir <目录> --delete-session <会话号>
//! colm-agent --backend-status                              # 本机 Codex / Claude Code 的安装与登录状态（JSON）
//! ```
//!
//! stdout 只输出协议消息；诊断写 stderr。stdin 关闭即退出。

use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use colm_agent::agent::{execute_tool, Agent, Approver, Decision, Limits};
use colm_agent::backend::{self, claude, codex, BackendKind, ExternalSession, Launch, TurnSink};
use colm_agent::bridge::{random_hex, BridgeHandler, BridgeServer};
use colm_agent::mcp::tool_entry;
use colm_agent::message::Message;
use colm_agent::message::ToolCall;
use colm_agent::protocol::{ApprovalPolicy, Inbound, Outbound};
use colm_agent::provider::{OpenAiCompatible, ProviderConfig, DEEPSEEK_BASE_URL};
use colm_agent::session::{self, Session};
use colm_agent::tools::ui::{UiBridge, UiHandle};
use colm_agent::tools::{web, Registry, Tier, ToolContext};
use serde_json::Value;

fn main() {
    if let Err(error) = run() {
        eprintln!("colm-agent: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if args.iter().any(|a| a == "--backend-status") {
        println!("{}", backend::status());
        return Ok(());
    }
    // 历史会话的查看与删除不碰 Key。
    if let Some(data_dir) = value("--data-dir").map(PathBuf::from) {
        if args.iter().any(|a| a == "--list-sessions") {
            println!("{}", serde_json::to_string(&session::list(&data_dir)?)?);
            return Ok(());
        }
        if let Some(id) = value("--transcript") {
            println!(
                "{}",
                serde_json::to_string(&session::transcript(&data_dir, &id)?)?
            );
            return Ok(());
        }
        if let Some(id) = value("--delete-session") {
            return session::delete(&data_dir, &id);
        }
    }
    let key_file = value("--key-file")
        .map(PathBuf::from)
        .context("--key-file <path> is required")?;
    if let Some(base) = value("--set-key") {
        let mut key = String::new();
        std::io::stdin().read_line(&mut key)?;
        if key.trim().is_empty() {
            bail!("no key on stdin");
        }
        return colm_agent::secrets::set(&key_file, &base, &key);
    }
    if let Some(base) = value("--has-key") {
        println!("{}", colm_agent::secrets::has(&key_file, &base)?);
        return Ok(());
    }
    if let Some(base) = value("--delete-key") {
        return colm_agent::secrets::delete(&key_file, &base);
    }
    let data_dir = value("--data-dir").map(PathBuf::from);
    let cli = value("--cli")
        .map(PathBuf::from)
        .context("--cli <colm-cli path> is required")?;
    serve(data_dir, cli, key_file)
}

/// 发给 GUI 的一行（并写审计）。
#[derive(Clone)]
struct Emitter {
    session: Arc<Mutex<Session>>,
}

impl Emitter {
    fn emit(&self, event: Outbound) {
        if let Ok(session) = self.session.lock() {
            if let Err(error) = session.audit(&event) {
                eprintln!("colm-agent: audit: {error:#}");
            }
        }
        let line = serde_json::to_string(&event).unwrap_or_default();
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// 这个服务的 Key：本进程第一次用时从 Key 文件读，之后用内存里的（设置里换了 Key 会重启进程）。
fn cached_key(
    keys: &Mutex<std::collections::BTreeMap<String, String>>,
    key_file: &std::path::Path,
    base_url: &str,
) -> Result<String> {
    let mut keys = keys
        .lock()
        .map_err(|_| anyhow::anyhow!("key cache poisoned"))?;
    if let Some(key) = keys.get(base_url) {
        return Ok(key.clone());
    }
    let key = colm_agent::secrets::get(key_file, base_url)?
        .context("no API key is stored for this service; set it in the assistant settings")?;
    keys.insert(base_url.to_owned(), key.clone());
    Ok(key)
}

/// 等 GUI 回复审批（取消时也会收到拒绝）。
struct ChannelApprover {
    decisions: Arc<Mutex<Receiver<(String, Decision)>>>,
    policy: ApprovalPolicy,
    /// 本会话选过“不再询问”的操作；开新会话时清空。
    allowed: Arc<Mutex<BTreeSet<String>>>,
}

impl Approver for ChannelApprover {
    /// 运行操作按审批设置放行；代码操作（外部后端的命令、改文件）只在本会话点过“都允许”后放行，
    /// 不受“自动执行”影响。
    fn preapproved(&self, name: &str, tier: Tier) -> bool {
        let remembered = self.allowed.lock().unwrap().contains(name);
        match tier {
            Tier::Read => true,
            Tier::Act => self.policy == ApprovalPolicy::Auto || remembered,
            Tier::Code => remembered,
        }
    }

    fn remember(&mut self, name: &str) {
        self.allowed.lock().unwrap().insert(name.to_owned());
    }

    fn decide(&mut self, request: &Outbound) -> Decision {
        let Outbound::ApprovalRequest { id, .. } = request else {
            return Decision::Deny(Some("not an approval request".into()));
        };
        let Ok(decisions) = self.decisions.lock() else {
            return Decision::Deny(None);
        };
        loop {
            match decisions.recv() {
                Ok((answered, decision)) if &answered == id || answered == "*" => return decision,
                Ok(_) => continue,
                Err(_) => return Decision::Deny(Some("the application closed".into())),
            }
        }
    }
}

struct Settings {
    provider: ProviderConfig,
    context: ToolContext,
    approval: ApprovalPolicy,
    web_search: bool,
    ui: bool,
    backend: BackendKind,
}

impl Settings {
    /// 面板头部显示的模型名：外部后端显示后端名（模型由它自己选）。
    fn display_model(&self) -> String {
        match self.backend {
            BackendKind::Builtin => self.provider.model.clone(),
            BackendKind::Codex => "Codex".into(),
            BackendKind::ClaudeCode => "Claude Code".into(),
        }
    }
}

/// 外部后端一轮里转发层要用的东西（`colm-mcp` 经转发口调工具时取用）。
#[derive(Clone)]
struct ActiveTurn {
    registry: Arc<Registry>,
    context: ToolContext,
    approver: Arc<Mutex<ChannelApprover>>,
    emitter: Emitter,
    cancel: Arc<AtomicBool>,
}

/// 转发口那一头：执行 `colm-mcp` 交回来的工具调用。
#[derive(Default)]
struct AgentBridge {
    active: Mutex<Option<ActiveTurn>>,
    /// 最近一轮的工具清单与是否提供 `approve`（Claude Code 的审批工具）；轮与轮之间也能列出。
    listing: Mutex<Option<(Arc<Registry>, bool)>>,
}

const APPROVE_DESCRIPTION: &str = "Internal: CoLM-Desktop asks the user to approve a Claude Code action. Do not call this yourself.";

impl BridgeHandler for AgentBridge {
    fn list(&self) -> Result<Vec<Value>> {
        let (registry, approve) = self
            .listing
            .lock()
            .unwrap()
            .clone()
            .context("the assistant has not started an answer yet")?;
        let mut tools: Vec<Value> = registry
            .tools()
            .map(|tool| tool_entry(tool.name(), tool.description(), tool.parameters()))
            .collect();
        if approve {
            tools.push(tool_entry(
                "approve",
                APPROVE_DESCRIPTION,
                serde_json::json!({ "type": "object", "properties": {
                    "tool_name": { "type": "string" }, "input": { "type": "object" },
                    "tool_use_id": { "type": "string" } } }),
            ));
        }
        Ok(tools)
    }

    fn call(&self, name: &str, arguments: Value) -> Result<(bool, String)> {
        let turn = self
            .active
            .lock()
            .unwrap()
            .clone()
            .context("no answer is in progress in CoLM-Desktop")?;
        if name == "approve" {
            return Ok((false, claude_permission(&arguments, &turn)));
        }
        let call = ToolCall {
            id: format!("colm-{}", random_hex(12)),
            name: name.to_owned(),
            arguments: arguments.to_string(),
        };
        let mut approver = turn.approver.lock().unwrap();
        let text = execute_tool(
            &turn.registry,
            &turn.context,
            &call,
            &mut |event| turn.emitter.emit(event),
            &mut *approver,
            &turn.cancel,
        );
        let is_error = text.starts_with("error:") || text.starts_with("the user declined");
        Ok((is_error, text))
    }
}

/// Claude Code 的权限请求（`--permission-prompt-tool`）：转成面板审批卡片，挂在同一个工具卡片上。
fn claude_permission(arguments: &Value, turn: &ActiveTurn) -> String {
    let tool = arguments["tool_name"].as_str().unwrap_or("?");
    let input = &arguments["input"];
    // CoLM 自己的工具在转发层按我们的审批规则把关，这里直接放行，免得同一件事问两次。
    if tool.starts_with("mcp__colm__") {
        return claude::permission_answer(true, input, None);
    }
    let request = Outbound::ApprovalRequest {
        id: arguments["tool_use_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("perm-{}", random_hex(12))),
        name: tool.to_owned(),
        tier: Tier::Code,
        summary: claude::summary(tool, input),
        arguments: input.to_string(),
    };
    match PanelSink::decide(&turn.emitter, &turn.approver, request) {
        Decision::Deny(note) => claude::permission_answer(false, input, note.as_deref()),
        _ => claude::permission_answer(true, input, None),
    }
}

/// 外部后端一轮的回调：事件发给界面，审批走面板（本会话点过“都允许”的同名动作直接放行）。
struct PanelSink {
    emitter: Emitter,
    approver: Arc<Mutex<ChannelApprover>>,
}

impl PanelSink {
    fn decide(emitter: &Emitter, approver: &Mutex<ChannelApprover>, request: Outbound) -> Decision {
        let Outbound::ApprovalRequest { name, tier, .. } = &request else {
            return Decision::Deny(None);
        };
        let (name, tier) = (name.clone(), *tier);
        let mut approver = approver.lock().unwrap();
        if approver.preapproved(&name, tier) {
            return Decision::Approve;
        }
        emitter.emit(request.clone());
        let decision = approver.decide(&request);
        if decision == Decision::ApproveForSession {
            approver.remember(&name);
        }
        decision
    }
}

impl TurnSink for PanelSink {
    fn emit(&mut self, event: Outbound) {
        self.emitter.emit(event);
    }
    fn approve(&mut self, request: Outbound) -> Decision {
        Self::decide(&self.emitter, &self.approver, request)
    }
}

/// 引导模式的界面桥：发 `ui_request`，等 GUI 的 `ui_result`（取消或两分钟没回话时报错）。
struct ChannelUi {
    emitter: Emitter,
    results: Arc<Mutex<Receiver<(String, bool, Value)>>>,
    cancel: Arc<AtomicBool>,
}

static UI_REQUESTS: AtomicU64 = AtomicU64::new(0);

impl UiBridge for ChannelUi {
    fn request(&self, action: &str, args: Value) -> Result<Value> {
        let id = format!("ui-{}", UI_REQUESTS.fetch_add(1, Ordering::SeqCst));
        let results = self
            .results
            .lock()
            .map_err(|_| anyhow::anyhow!("the window channel is poisoned"))?;
        // 先清掉上一次超时后才到的回话。
        while results.try_recv().is_ok() {}
        self.emitter.emit(Outbound::UiRequest {
            id: id.clone(),
            action: action.to_owned(),
            args,
        });
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            match results.recv_timeout(Duration::from_millis(200)) {
                Ok((answered, ok, result)) if answered == id => {
                    if ok {
                        return Ok(result);
                    }
                    let message = result
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| result.to_string());
                    bail!("{message}");
                }
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => continue,
                Err(RecvTimeoutError::Timeout) => bail!("the application window did not answer"),
                Err(RecvTimeoutError::Disconnected) => bail!("the application closed"),
            }
        }
    }
}

fn serve(data_dir: Option<PathBuf>, cli: PathBuf, key_file: PathBuf) -> Result<()> {
    let session = Arc::new(Mutex::new(Session::new(
        data_dir.as_deref(),
        colm_agent::SYSTEM_PROMPT,
    )?));
    let emitter = Emitter {
        session: Arc::clone(&session),
    };
    let settings: Arc<Mutex<Option<Settings>>> = Arc::new(Mutex::new(None));
    let busy = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let (decision_tx, decision_rx): (Sender<(String, Decision)>, _) = mpsc::channel();
    let decision_rx = Arc::new(Mutex::new(decision_rx));
    let (ui_tx, ui_rx): (Sender<(String, bool, Value)>, _) = mpsc::channel();
    // 外部后端：当前会话对应的外部会话（Codex 常驻进程 / Claude Code 的 session id），与转发口。
    let external: Arc<Mutex<Option<External>>> = Arc::default();
    let bridge: SharedBridge = Arc::default();
    let ui_rx = Arc::new(Mutex::new(ui_rx));
    // 本会话允许 `fetch_url` 打开的网站（搜索结果与用户消息里的）；换会话时清空。
    let allowed_hosts: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let allowed: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    // 读过的 Key 留在内存里，不必每轮都读文件。
    let keys: Arc<Mutex<std::collections::BTreeMap<String, String>>> = Arc::default();
    let mut worker: Option<std::thread::JoinHandle<()>> = None;

    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Inbound = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(error) => {
                emitter.emit(Outbound::Error {
                    message: format!("unreadable request: {error}"),
                });
                continue;
            }
        };
        match message {
            Inbound::Configure {
                provider,
                project_root,
                kernel_dir,
                docs_root,
                approval,
                web_search,
                ui,
                backend,
            } => {
                let model = match backend {
                    BackendKind::Builtin => provider.model.clone(),
                    BackendKind::Codex => "Codex".into(),
                    BackendKind::ClaudeCode => "Claude Code".into(),
                };
                *settings.lock().unwrap() = Some(Settings {
                    provider,
                    context: ToolContext {
                        project_root: PathBuf::from(project_root),
                        cli: cli.clone(),
                        kernel_dir: kernel_dir.filter(|k| !k.is_empty()).map(PathBuf::from),
                        docs_root: docs_root.filter(|d| !d.is_empty()).map(PathBuf::from),
                        cancel: None,
                        web: None,
                        ui: None,
                    },
                    approval,
                    web_search,
                    ui,
                    backend,
                });
                // 换了后端就丢掉旧的外部会话（Codex 的进程随之结束）。
                if external
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|e| e.kind != backend)
                {
                    *external.lock().unwrap() = None;
                }
                let id = session.lock().unwrap().id.clone();
                emitter.emit(Outbound::Ready { session: id, model });
            }
            Inbound::ResumeSession { id } => {
                if busy.load(Ordering::SeqCst) {
                    emitter.emit(Outbound::Error {
                        message: "stop the current answer before switching conversations".into(),
                    });
                    continue;
                }
                let current = session.lock().unwrap().id.clone();
                // 没存过的（开了没说话，进程就重启了）不算错：留在当前会话。
                let saved = data_dir
                    .as_deref()
                    .is_some_and(|root| session::saved(root, &id));
                if current != id && saved {
                    let Some(root) = data_dir.as_deref() else {
                        continue;
                    };
                    match Session::open(root, &id, colm_agent::SYSTEM_PROMPT) {
                        Ok(opened) => {
                            *session.lock().unwrap() = opened;
                            allowed.lock().unwrap().clear();
                            allowed_hosts.lock().unwrap().clear();
                            *external.lock().unwrap() = None;
                        }
                        Err(error) => {
                            emitter.emit(Outbound::Error {
                                message: format!("{error:#}"),
                            });
                            continue;
                        }
                    }
                }
                let model = settings
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(Settings::display_model)
                    .unwrap_or_default();
                let id = session.lock().unwrap().id.clone();
                emitter.emit(Outbound::Ready { session: id, model });
            }
            Inbound::NewSession => {
                if busy.load(Ordering::SeqCst) {
                    emitter.emit(Outbound::Error {
                        message: "stop the current answer before starting a new conversation"
                            .into(),
                    });
                    continue;
                }
                *session.lock().unwrap() =
                    Session::new(data_dir.as_deref(), colm_agent::SYSTEM_PROMPT)?;
                allowed.lock().unwrap().clear();
                allowed_hosts.lock().unwrap().clear();
                *external.lock().unwrap() = None;
                let model = settings
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(Settings::display_model)
                    .unwrap_or_default();
                let id = session.lock().unwrap().id.clone();
                emitter.emit(Outbound::Ready { session: id, model });
            }
            Inbound::ApprovalDecision {
                id,
                approve,
                note,
                remember,
            } => {
                let decision = if approve && remember {
                    Decision::ApproveForSession
                } else if approve {
                    Decision::Approve
                } else {
                    Decision::Deny(note)
                };
                let _ = decision_tx.send((id, decision));
            }
            Inbound::UiResult { id, ok, result } => {
                let _ = ui_tx.send((id, ok, result));
            }
            Inbound::Cancel => {
                cancel.store(true, Ordering::SeqCst);
                let _ = decision_tx.send(("*".into(), Decision::Deny(Some("cancelled".into()))));
            }
            Inbound::UserMessage { text, context } => {
                if busy.swap(true, Ordering::SeqCst) {
                    emitter.emit(Outbound::Error {
                        message: "still answering the previous message".into(),
                    });
                    continue;
                }
                let Some((mut provider, mut tool_context, policy, web_search, ui, backend)) =
                    settings.lock().unwrap().as_ref().map(|s| {
                        (
                            s.provider.clone(),
                            s.context.clone(),
                            s.approval,
                            s.web_search,
                            s.ui,
                            s.backend,
                        )
                    })
                else {
                    busy.store(false, Ordering::SeqCst);
                    emitter.emit(Outbound::Error {
                        message: "the assistant is not configured yet".into(),
                    });
                    continue;
                };
                cancel.store(false, Ordering::SeqCst);
                tool_context.cancel = Some(Arc::clone(&cancel));
                let session = Arc::clone(&session);
                let emitter = emitter.clone();
                let busy = Arc::clone(&busy);
                let cancel = Arc::clone(&cancel);
                let decisions = Arc::clone(&decision_rx);
                let allowed = Arc::clone(&allowed);
                // 外部后端用自带的联网搜索，不提供 DeepSeek 的 web_search / fetch_url。
                let registry =
                    Registry::standard_with(web_search && backend == BackendKind::Builtin, ui);
                if ui {
                    tool_context.ui = Some(UiHandle(Arc::new(ChannelUi {
                        emitter: emitter.clone(),
                        results: Arc::clone(&ui_rx),
                        cancel: Arc::clone(&cancel),
                    })));
                }
                if web_search {
                    let mut urls = allowed_hosts.lock().unwrap();
                    urls.extend(web::hosts_in(&text));
                }
                let allowed_hosts = Arc::clone(&allowed_hosts);
                let keys = Arc::clone(&keys);
                let key_file = key_file.clone();
                if backend != BackendKind::Builtin {
                    let turn = ExternalTurn {
                        backend,
                        text,
                        context,
                        registry,
                        tool_context,
                        approver: ChannelApprover {
                            decisions,
                            policy,
                            allowed,
                        },
                        web_search,
                        data_dir: data_dir.clone(),
                        session,
                        emitter,
                        cancel,
                        external: Arc::clone(&external),
                        bridge: Arc::clone(&bridge),
                    };
                    worker = Some(std::thread::spawn(move || {
                        let emitter = turn.emitter.clone();
                        if let Err(error) = turn.run() {
                            emitter.emit(Outbound::Error {
                                message: format!("{error:#}"),
                            });
                        }
                        busy.store(false, Ordering::SeqCst);
                    }));
                    continue;
                }
                worker = Some(std::thread::spawn(move || {
                    let result = (|| -> Result<()> {
                        provider.api_key = cached_key(&keys, &key_file, &provider.base_url)?;
                        if web_search {
                            // 搜索总用 DeepSeek 的 Key（会话模型可以是别家）；没存时搜索工具自己报错。
                            let search_key =
                                cached_key(&keys, &key_file, DEEPSEEK_BASE_URL).unwrap_or_default();
                            let mut access = web::WebAccess::deepseek(search_key, allowed_hosts);
                            if let Ok(base) = std::env::var("COLM_AGENT_SEARCH_URL") {
                                access.search_base = base;
                            }
                            tool_context.web = Some(access);
                        }
                        let strict = provider.strict;
                        let client = OpenAiCompatible::new(provider);
                        let agent = Agent {
                            provider: &client,
                            registry: &registry,
                            context: tool_context,
                            limits: Limits::default(),
                            strict,
                        };
                        let content = match context.filter(|c| !c.trim().is_empty()) {
                            Some(context) => {
                                format!("{text}\n\n[Current view in the application]\n{context}")
                            }
                            None => text,
                        };
                        // 历史在会话锁外跑（运行时间长），结束后再写回。
                        let (mut history, from) = {
                            let mut session = session.lock().unwrap();
                            session.push(Message::User { content })?;
                            (session.history.clone(), session.history.len())
                        };
                        let mut approver = ChannelApprover {
                            decisions,
                            policy,
                            allowed,
                        };
                        let outcome = agent.run_turn(
                            &mut history,
                            &mut |event| emitter.emit(event),
                            &mut approver,
                            &cancel,
                        );
                        let totals = {
                            let mut session = session.lock().unwrap();
                            session.history = history;
                            session.persist_from(from)?;
                            if let Ok(outcome) = &outcome {
                                session.usage.prompt_tokens += outcome.usage.prompt_tokens;
                                session.usage.completion_tokens += outcome.usage.completion_tokens;
                            }
                            session.usage
                        };
                        let outcome = outcome?;
                        emitter.emit(Outbound::Usage {
                            prompt_tokens: outcome.usage.prompt_tokens,
                            completion_tokens: outcome.usage.completion_tokens,
                            session_prompt_tokens: totals.prompt_tokens,
                            session_completion_tokens: totals.completion_tokens,
                        });
                        emitter.emit(Outbound::TurnDone {
                            content: outcome.content,
                            steps: outcome.steps,
                        });
                        Ok(())
                    })();
                    if let Err(error) = result {
                        emitter.emit(Outbound::Error {
                            message: format!("{error:#}"),
                        });
                    }
                    busy.store(false, Ordering::SeqCst);
                }));
            }
        }
    }
    // stdin 关了（GUI 退出或脚本输完）：先让正在进行的这一轮做完、写完日志再退出。
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    Ok(())
}

/// 转发口与它的处理方：第一次用外部后端时才打开。
type SharedBridge = Arc<Mutex<Option<(BridgeServer, Arc<AgentBridge>)>>>;

/// 当前会话对应的外部会话。
struct External {
    kind: BackendKind,
    /// 属于哪个 CoLM 会话（换会话就换外部会话）。
    session: String,
    inner: Box<dyn ExternalSession>,
}

/// 外部后端的一轮（在工作线程里跑）。
struct ExternalTurn {
    backend: BackendKind,
    text: String,
    context: Option<String>,
    registry: Registry,
    tool_context: ToolContext,
    approver: ChannelApprover,
    web_search: bool,
    data_dir: Option<PathBuf>,
    session: Arc<Mutex<Session>>,
    emitter: Emitter,
    cancel: Arc<AtomicBool>,
    external: Arc<Mutex<Option<External>>>,
    bridge: SharedBridge,
}

impl ExternalTurn {
    fn run(self) -> Result<()> {
        // 联网交给外部后端自带的搜索（计入它的订阅），这里不挂 DeepSeek 的 web_search / fetch_url。
        let mut tool_context = self.tool_context;
        if !tool_context.project_root.is_dir() {
            tool_context.project_root = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_default();
        }
        let content = match self.context.filter(|c| !c.trim().is_empty()) {
            Some(context) => format!(
                "{}\n\n[Current view in the application]\n{context}",
                self.text
            ),
            None => self.text,
        };
        let session_id = {
            let mut session = self.session.lock().unwrap();
            session.push(Message::User {
                content: content.clone(),
            })?;
            session.id.clone()
        };

        // 转发口：第一次用外部后端时打开，之后一直用同一个。
        let (addr, token, handler) = {
            let mut bridge = self.bridge.lock().unwrap();
            if bridge.is_none() {
                let handler = Arc::new(AgentBridge::default());
                let server = colm_agent::bridge::serve(handler.clone())?;
                *bridge = Some((server, handler));
            }
            let (server, handler) = bridge.as_ref().unwrap();
            (
                server.addr.to_string(),
                server.token.clone(),
                Arc::clone(handler),
            )
        };
        let registry = Arc::new(self.registry);
        let approver = Arc::new(Mutex::new(self.approver));
        *handler.listing.lock().unwrap() = Some((
            Arc::clone(&registry),
            self.backend == BackendKind::ClaudeCode,
        ));
        *handler.active.lock().unwrap() = Some(ActiveTurn {
            registry,
            context: tool_context.clone(),
            approver: Arc::clone(&approver),
            emitter: self.emitter.clone(),
            cancel: Arc::clone(&self.cancel),
        });

        let mut external = self.external.lock().unwrap();
        if external
            .as_ref()
            .is_none_or(|e| e.kind != self.backend || e.session != session_id)
        {
            let launch = Launch {
                cwd: tool_context.project_root.clone(),
                mcp_exe: std::env::current_exe()?
                    .with_file_name(format!("colm-mcp{}", std::env::consts::EXE_SUFFIX)),
                bridge_addr: addr,
                bridge_token: token,
                instructions: backend::instructions(colm_agent::SYSTEM_PROMPT, self.web_search),
                web: self.web_search,
            };
            // 续接历史对话时接着用当时的外部会话号。
            let resume = self
                .data_dir
                .as_deref()
                .and_then(|root| session::backend_of(root, &session_id))
                .filter(|(kind, _)| *kind == self.backend)
                .map(|(_, id)| id);
            let inner: Box<dyn ExternalSession> = match self.backend {
                BackendKind::Codex => Box::new(codex::CodexSession::new(launch, resume)),
                _ => Box::new(claude::ClaudeSession::new(launch, resume)),
            };
            *external = Some(External {
                kind: self.backend,
                session: session_id,
                inner,
            });
        }
        let current = external.as_mut().unwrap();
        let mut sink = PanelSink {
            emitter: self.emitter.clone(),
            approver,
        };
        let outcome = current.inner.turn(&content, &mut sink, &self.cancel);
        *handler.active.lock().unwrap() = None;
        let resume = current.inner.resume_id();
        drop(external);

        let outcome = outcome?;
        let totals = {
            let mut session = self.session.lock().unwrap();
            session.push(Message::Assistant {
                content: outcome.content.clone(),
                reasoning_content: None,
                tool_calls: Vec::new(),
            })?;
            if let Some(id) = &resume {
                session.save_backend(self.backend, id)?;
            }
            session.usage.prompt_tokens += outcome.usage.prompt_tokens;
            session.usage.completion_tokens += outcome.usage.completion_tokens;
            session.usage
        };
        self.emitter.emit(Outbound::Usage {
            prompt_tokens: outcome.usage.prompt_tokens,
            completion_tokens: outcome.usage.completion_tokens,
            session_prompt_tokens: totals.prompt_tokens,
            session_completion_tokens: totals.completion_tokens,
        });
        self.emitter.emit(Outbound::TurnDone {
            content: outcome.content,
            steps: 1,
        });
        Ok(())
    }
}
