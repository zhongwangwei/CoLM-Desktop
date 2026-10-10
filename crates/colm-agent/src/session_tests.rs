use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn assistant(content: &str, calls: &[(&str, &str)]) -> Message {
    Message::Assistant {
        provider_state: None,
        content: content.into(),
        reasoning_content: Some("thinking".into()),
        tool_calls: calls
            .iter()
            .map(|(id, name)| crate::message::ToolCall {
                id: (*id).into(),
                name: (*name).into(),
                arguments: "{}".into(),
            })
            .collect(),
    }
}

#[test]
fn sessions_are_written_on_the_first_message_and_can_be_resumed() {
    let root = temp_dir("resume");
    let mut session = Session::new(Some(&root), "prompt v1").unwrap();
    // 没说话的会话不落盘，也不出现在列表里。
    assert!(!root.join("sessions").join(&session.id).exists());
    assert!(list(&root).unwrap().is_empty());

    session
        .push(Message::User {
            content: "建一个 PC 算例\n\n[Current view in the application]\nkernel: /k".into(),
        })
        .unwrap();
    let from = session.history.len();
    session
        .history
        .push(assistant("", &[("c1", "create_case")]));
    session.history.push(Message::Tool {
        tool_call_id: "c1".into(),
        content: r#"{"created":true}"#.into(),
    });
    session.history.push(assistant("", &[("c2", "run_case")]));
    session.history.push(Message::Tool {
        tool_call_id: "c2".into(),
        content: "the user declined this action".into(),
    });
    session.history.push(assistant("建好了。", &[]));
    session.persist_from(from).unwrap();

    let listed = list(&root).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session.id);
    assert_eq!(listed[0].title, "建一个 PC 算例");
    assert_eq!(listed[0].turns, 1);

    let items = transcript(&root, &session.id).unwrap();
    assert_eq!(
        items[0],
        TranscriptItem::User {
            text: "建一个 PC 算例".into()
        }
    );
    assert!(
        matches!(&items[1], TranscriptItem::Tool { name, ok: true, result: Some(_), .. } if name == "create_case")
    );
    assert!(
        matches!(&items[2], TranscriptItem::Tool { name, ok: false, .. } if name == "run_case")
    );
    assert_eq!(
        items[3],
        TranscriptItem::Assistant {
            text: "建好了。".into()
        }
    );

    // 续接：历史原样读回（含 reasoning_content），系统提示换成当前版本；接着写不重复。
    let mut resumed = Session::open(&root, &session.id, "prompt v2").unwrap();
    assert_eq!(resumed.history.len(), session.history.len());
    assert!(matches!(&resumed.history[0], Message::System { content }
        if content.starts_with("prompt v2") && content.contains("Recovered task checkpoint")));
    assert_eq!(resumed.history[2], session.history[2]);
    resumed
        .push(Message::User {
            content: "再跑一次".into(),
        })
        .unwrap();
    let again = Session::open(&root, &session.id, "prompt v2").unwrap();
    assert_eq!(again.history.len(), session.history.len() + 1);
    assert_eq!(list(&root).unwrap()[0].turns, 2);

    // 外部后端的会话号随会话保存，续接时读回。
    assert!(backend_of(&root, &session.id).is_none());
    again
        .save_backend(crate::backend::BackendKind::ClaudeCode, "4844562a-80d3")
        .unwrap();
    assert_eq!(
        backend_of(&root, &session.id),
        Some((
            crate::backend::BackendKind::ClaudeCode,
            "4844562a-80d3".to_owned()
        ))
    );

    delete(&root, &session.id).unwrap();
    assert!(list(&root).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn session_ids_cannot_escape_the_sessions_directory() {
    let root = temp_dir("escape");
    for id in ["../x", "", "a/b", "..", "123/../../etc"] {
        assert!(transcript(&root, id).is_err(), "{id}");
        assert!(delete(&root, id).is_err(), "{id}");
        assert!(Session::open(&root, id, "p").is_err(), "{id}");
    }
}

#[test]
fn a_crash_recovers_audited_actions_even_without_model_history() {
    let root = temp_dir("interrupted-actions");
    let mut session = Session::new(Some(&root), "current rules").unwrap();
    session
        .push(Message::User {
            content: "run current case".into(),
        })
        .unwrap();
    session
        .audit(&Outbound::ToolCall {
            id: "pending-run".into(),
            name: "run_case".into(),
            arguments: r#"{"case":"site"}"#.into(),
            tier: crate::tools::Tier::Act,
            summary: "run case site".into(),
            preapproved: false,
        })
        .unwrap();
    let opened = Session::open(&root, &session.id, "updated rules").unwrap();
    let task = opened.task_snapshot(true).unwrap().unwrap();
    assert!(task.requires_reconciliation);
    assert_eq!(task.actions[0].state, "unconfirmed");
    assert!(
        matches!(&opened.history[0], Message::System { content } if content.contains("pending-run") && content.contains("Do not automatically repeat"))
    );
    let items = transcript(&root, &session.id).unwrap();
    assert!(items.iter().any(
        |item| matches!(item, TranscriptItem::Tool { id, result: None, .. } if id == "pending-run")
    ));
    assert!(items
        .iter()
        .any(|item| matches!(item, TranscriptItem::Task { task } if task.state == "interrupted")));
    session
        .audit(&Outbound::ToolResult {
            id: "pending-run".into(),
            name: "run_case".into(),
            ok: true,
            result: "existing output verified".into(),
            elapsed_ms: 1,
        })
        .unwrap();
    let task = Session::open(&root, &session.id, "updated rules")
        .unwrap()
        .task_snapshot(true)
        .unwrap()
        .unwrap();
    assert!(!task.requires_reconciliation);
    assert_eq!(
        task.actions[0].result.as_deref(),
        Some("existing output verified")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn torn_journal_tails_do_not_swallow_the_next_message_or_checkpoint() {
    let root = temp_dir("torn-tail");
    let mut session = Session::new(Some(&root), "rules").unwrap();
    session
        .push(Message::User {
            content: "first".into(),
        })
        .unwrap();
    let dir = root.join("sessions").join(&session.id);
    for file in ["messages.jsonl", "audit.jsonl"] {
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(file))
            .unwrap();
        handle.write_all(b"{partial").unwrap();
    }
    session
        .push(Message::User {
            content: "second".into(),
        })
        .unwrap();
    let reopened = Session::open(&root, &session.id, "rules").unwrap();
    assert!(
        matches!(reopened.history.last(), Some(Message::User { content }) if content == "second")
    );
    assert_eq!(
        reopened.task_snapshot(true).unwrap().unwrap().goal,
        "second"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn history_fixture(root: &Path, id: &str, updated: u64, messages: &[Message]) {
    let dir = root.join("sessions").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("messages.jsonl");
    let text = messages
        .iter()
        .map(|message| serde_json::to_string(message).unwrap() + "\n")
        .collect::<String>();
    std::fs::write(&path, text).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(updated)),
        )
        .unwrap();
}

#[test]
fn input_history_contains_only_sent_prompts_and_follows_session_updates() {
    let root = temp_dir("input-history");
    assert!(input_history(&root).unwrap().is_empty());
    let user = |content: &str| Message::User {
        content: content.into(),
    };
    history_fixture(
        &root,
        "200-1",
        1,
        &[
            Message::System {
                content: "private system".into(),
            },
            user(" first \n\n[Current view in the application]\nprivate context"),
            assistant("not input", &[]),
            Message::Tool {
                tool_call_id: "1".into(),
                content: "not input either".into(),
            },
            user("same"),
        ],
    );
    // An older conversation resumed later is more recent than a newer conversation.
    history_fixture(
        &root,
        "100-1",
        2,
        &[user("same"), user(" \n "), user("last")],
    );
    assert_eq!(input_history(&root).unwrap(), ["first", "same", "last"]);
    delete(&root, "100-1").unwrap();
    assert_eq!(input_history(&root).unwrap(), ["first", "same"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_history_limits_sessions_inputs_and_oversized_prompts() {
    let root = temp_dir("input-history-limits");
    for index in 0..21 {
        history_fixture(
            &root,
            &format!("{index}-1"),
            index,
            &[Message::User {
                content: format!("session {index}"),
            }],
        );
    }
    let history = input_history(&root).unwrap();
    assert_eq!(history.len(), 20);
    assert_eq!(history[0], "session 1");
    assert_eq!(history[19], "session 20");
    let mut messages = (0..110)
        .map(|index| Message::User {
            content: format!("input {index}"),
        })
        .collect::<Vec<_>>();
    messages.push(Message::User {
        content: "x".repeat(32_001),
    });
    history_fixture(&root, "21-1", 21, &messages);
    let history = input_history(&root).unwrap();
    assert_eq!(history.len(), 100);
    assert_eq!(history[0], "input 10");
    assert_eq!(history[99], "input 109");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_history_recovers_recent_input_from_a_large_or_torn_transcript() {
    let root = temp_dir("input-history-tail");
    history_fixture(
        &root,
        "1-1",
        1,
        &[
            assistant(&"大".repeat(3 * 1024 * 1024), &[]),
            Message::User {
                content: "尾部输入".into(),
            },
        ],
    );
    let path = root.join("sessions/1-1/messages.jsonl");
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(b"{\"role\":\"user\",\"content\":\"torn")
        .unwrap();
    assert_eq!(input_history(&root).unwrap(), ["尾部输入"]);
    std::fs::remove_dir_all(root).unwrap();
}
