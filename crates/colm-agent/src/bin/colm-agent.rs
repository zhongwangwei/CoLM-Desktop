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
//! ```
//!
//! stdout 只输出协议消息；诊断写 stderr。stdin 关闭即退出。

use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use colm_agent::agent::{Agent, Approver, Decision, Limits};
use colm_agent::message::Message;
use colm_agent::protocol::{ApprovalPolicy, Inbound, Outbound};
use colm_agent::provider::{OpenAiCompatible, ProviderConfig};
use colm_agent::session::{self, Session};
use colm_agent::tools::{Registry, Tier, ToolContext};

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
    /// 只放行运行操作；代码操作始终逐次审批。
    fn preapproved(&self, name: &str, tier: Tier) -> bool {
        tier == Tier::Act
            && (self.policy == ApprovalPolicy::Auto || self.allowed.lock().unwrap().contains(name))
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
    let registry = Arc::new(Registry::standard());
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
            } => {
                let model = provider.model.clone();
                *settings.lock().unwrap() = Some(Settings {
                    provider,
                    context: ToolContext {
                        project_root: PathBuf::from(project_root),
                        cli: cli.clone(),
                        kernel_dir: kernel_dir.filter(|k| !k.is_empty()).map(PathBuf::from),
                        docs_root: docs_root.filter(|d| !d.is_empty()).map(PathBuf::from),
                        cancel: None,
                    },
                    approval,
                });
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
                    .map(|s| s.provider.model.clone())
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
                let model = settings
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|s| s.provider.model.clone())
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
                let Some((mut provider, mut tool_context, policy)) = settings
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|s| (s.provider.clone(), s.context.clone(), s.approval))
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
                let registry = Arc::clone(&registry);
                let keys = Arc::clone(&keys);
                let key_file = key_file.clone();
                worker = Some(std::thread::spawn(move || {
                    let result = (|| -> Result<()> {
                        provider.api_key = cached_key(&keys, &key_file, &provider.base_url)?;
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
