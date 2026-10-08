use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn assistant(content: &str, calls: &[(&str, &str)]) -> Message {
    Message::Assistant {
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
    assert_eq!(
        resumed.history[0],
        Message::System {
            content: "prompt v2".into()
        }
    );
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
