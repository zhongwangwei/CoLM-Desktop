use super::*;
static TEST_LOCK: Mutex<()> = Mutex::new(());
fn ask(session: &Session, prompt: &str) -> std::thread::JoinHandle<Vec<u8>> {
    let address = session.address.clone();
    let token = session.token.clone();
    let prompt = prompt.to_owned();
    std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(format!("{token}{prompt}\n").as_bytes())
            .unwrap();
        let mut result = Vec::new();
        stream.read_to_end(&mut result).unwrap();
        result
    })
}
fn pending() -> AuthSnapshot {
    let start = Instant::now();
    loop {
        let snapshot = remote_auth_pending().unwrap();
        if !snapshot.requests.is_empty() {
            return snapshot;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn an_accepted_nonblocking_socket_waits_for_the_full_prompt() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("delayed-prompt".into()).unwrap();
    session.operation.pid.store(u32::MAX, Ordering::SeqCst);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let operation = session.operation.clone();
    let token = session.token.clone();
    let (finished, observed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        serve(stream, &token, &operation, Instant::now() + DEADLINE);
        let _ = finished.send(());
    });
    let (first, rest) = session.token.as_bytes().split_at(16);
    client.write_all(first).unwrap();
    assert_eq!(
        observed.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    client.write_all(rest).unwrap();
    client.write_all(b"Password: \n").unwrap();
    let request = pending().requests.remove(0);
    respond(request.request_id, Some("delayed-secret".into())).unwrap();
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).unwrap();
    assert_eq!(reply, b"delayed-secret");
    worker.join().unwrap();
}

#[test]
fn socket_framing_repeated_prompts_redaction_and_stale_response() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("server-one".into()).unwrap();
    session.operation.pid.store(u32::MAX, Ordering::SeqCst);
    let reply = ask(&session, "Enter passphrase for key '/tmp/key': ");
    let snapshot = pending();
    let request = &snapshot.requests[0];
    assert_eq!(request.operation_host, "server-one");
    assert_eq!(request.kind, "passphrase");
    let id = request.request_id.clone();
    respond(id.clone(), Some("secret-value".into())).unwrap();
    assert_eq!(reply.join().unwrap(), b"secret-value");
    assert_eq!(session.redact("error secret-value"), "error [redacted]");
    assert!(remote_auth_pending().unwrap().revision > snapshot.revision);
    assert!(respond(id, Some("stale".into())).is_err());
    let retry = ask(&session, &request.prompt);
    let retry_request = pending().requests.remove(0);
    assert_ne!(retry_request.request_id, request.request_id);
    assert_eq!(retry_request.prompt, request.prompt);
    respond(retry_request.request_id, Some("corrected-secret".into())).unwrap();
    assert_eq!(retry.join().unwrap(), b"corrected-secret");
    let repeated = ask(&session, "user@server-one's password: ");
    let request = pending().requests.remove(0);
    respond(request.request_id, None).unwrap();
    assert!(repeated.join().unwrap().is_empty());
    assert!(session.operation.closed.load(Ordering::SeqCst));
}
#[test]
fn drop_and_stop_clear_pending_and_wake_socket() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("server-two".into()).unwrap();
    session.operation.pid.store(u32::MAX - 1, Ordering::SeqCst);
    let reply = ask(&session, "user@server-two's password: ");
    let id = pending().requests.remove(0).request_id;
    drop(session);
    assert!(reply.join().unwrap().is_empty());
    assert!(remote_auth_pending().unwrap().requests.is_empty());
    assert!(respond(id, Some("late".into())).is_err());
    let session = Session::new("server-three".into()).unwrap();
    session.operation.pid.store(u32::MAX - 2, Ordering::SeqCst);
    let reply = ask(&session, "Password: ");
    pending();
    cancel_pid(u32::MAX - 2);
    assert!(reply.join().unwrap().is_empty());
    assert!(remote_auth_pending().unwrap().requests.is_empty());
}
#[test]
fn token_is_not_a_request_id_and_invalid_token_cannot_enqueue() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("server".into()).unwrap();
    assert_eq!(session.token.len(), 64);
    assert_ne!(session.token, session.operation.id);
    session.operation.pid.store(u32::MAX, Ordering::SeqCst);
    let mut stream = TcpStream::connect(&session.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(&[b'0'; 64]).unwrap();
    let mut result = Vec::new();
    if let Err(error) = stream.read_to_end(&mut result) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    assert!(result.is_empty());
    assert!(remote_auth_pending().unwrap().requests.is_empty());
}

#[test]
fn simultaneous_operations_are_queued_and_cancellation_is_scoped() {
    let _lock = TEST_LOCK.lock().unwrap();
    let first = Session::new("first".into()).unwrap();
    let second = Session::new("second".into()).unwrap();
    first.operation.pid.store(u32::MAX - 3, Ordering::SeqCst);
    second.operation.pid.store(u32::MAX - 4, Ordering::SeqCst);
    let first_reply = ask(&first, "Password: ");
    pending();
    let second_reply = ask(&second, "Password: ");
    let started = Instant::now();
    while remote_auth_pending().unwrap().requests.len() != 2 {
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    cancel_pid(u32::MAX - 3);
    assert!(first_reply.join().unwrap().is_empty());
    let snapshot = remote_auth_pending().unwrap();
    assert_eq!(snapshot.requests.len(), 1);
    assert_eq!(snapshot.requests[0].operation_host, "second");
    respond(
        snapshot.requests[0].request_id.clone(),
        Some("second-secret".into()),
    )
    .unwrap();
    assert_eq!(second_reply.join().unwrap(), b"second-secret");
    assert_eq!(first.redact("second-secret"), "second-secret");
}

#[test]
fn same_session_requests_queue_before_first_reply_and_cancel_together() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("shared-server".into()).unwrap();
    session.operation.pid.store(u32::MAX - 8, Ordering::SeqCst);
    let first = ask(&session, "first@server's password: ");
    let first_id = pending().requests[0].request_id.clone();
    let second = ask(&session, "second@server's password: ");
    let started = Instant::now();
    let snapshot = loop {
        let snapshot = remote_auth_pending().unwrap();
        if snapshot.requests.len() == 2 {
            break snapshot;
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(snapshot.requests[0].request_id, first_id);
    respond(first_id, Some("first-secret".into())).unwrap();
    assert_eq!(first.join().unwrap(), b"first-secret");
    assert_eq!(remote_auth_pending().unwrap().requests.len(), 1);
    respond(
        snapshot.requests[1].request_id.clone(),
        Some("second-secret".into()),
    )
    .unwrap();
    assert_eq!(second.join().unwrap(), b"second-secret");
    assert_eq!(
        session.redact("first-secret second-secret"),
        "[redacted] [redacted]"
    );

    let first = ask(&session, "first@server's password: ");
    pending();
    let second = ask(&session, "second@server's password: ");
    let started = Instant::now();
    while remote_auth_pending().unwrap().requests.len() != 2 {
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot = remote_auth_pending().unwrap();
    respond(snapshot.requests[0].request_id.clone(), None).unwrap();
    assert!(first.join().unwrap().is_empty());
    assert!(second.join().unwrap().is_empty());
    assert!(remote_auth_pending().unwrap().requests.is_empty());
    assert!(respond(snapshot.requests[1].request_id.clone(), Some("late".into())).is_err());
}

#[test]
#[cfg(unix)]
fn cancelling_prompt_marks_and_stops_only_registered_child() {
    let _lock = TEST_LOCK.lock().unwrap();
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 30"]);
    let mut child = colm_kernel::run::top_level_sidecar(&mut command)
        .spawn()
        .unwrap();
    let guard = crate::sidecar::register_child(child.id(), crate::sidecar::ChildOwner::Background);
    let session = Session::new("server".into()).unwrap();
    session.bind(&guard);
    let reply = ask(&session, "Password: ");
    let snapshot = pending();
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert!(serialized["revision"].is_u64());
    assert_eq!(serialized["requests"][0]["kind"], "password");
    assert!(serialized["requests"][0].get("pid").is_none());
    assert!(serialized["requests"][0].get("token").is_none());
    respond(snapshot.requests[0].request_id.clone(), None).unwrap();
    assert!(guard.stopped());
    assert!(!child.wait().unwrap().success());
    assert!(reply.join().unwrap().is_empty());
    let cancellation = guard.cancellation();
    drop(guard);
    cancellation.stop().unwrap();
}

#[test]
fn generic_password_prompts_are_never_reused_for_a_different_connection() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("server".into()).unwrap();
    session.operation.pid.store(u32::MAX - 5, Ordering::SeqCst);
    let first = ask(&session, "Password: ");
    let request = pending().requests.remove(0);
    respond(request.request_id, Some("jump-secret".into())).unwrap();
    assert_eq!(first.join().unwrap(), b"jump-secret");
    let second = ask(&session, "Password: ");
    let request = pending().requests.remove(0);
    respond(request.request_id, Some("target-secret".into())).unwrap();
    assert_eq!(second.join().unwrap(), b"target-secret");
}

#[test]
fn invalid_prompt_shapes_do_not_reach_gui() {
    let _lock = TEST_LOCK.lock().unwrap();
    let session = Session::new("server".into()).unwrap();
    session.operation.pid.store(u32::MAX - 6, Ordering::SeqCst);
    for prompt in [
        "Confirm password login (yes/no)?",
        "password:\t",
        "passphrase please",
        "Are you sure?",
    ] {
        assert!(ask(&session, prompt).join().unwrap().is_empty());
        assert!(remote_auth_pending().unwrap().requests.is_empty());
    }
}

#[test]
fn stale_child_capability_cannot_cancel_new_registration_with_same_pid() {
    let _lock = TEST_LOCK.lock().unwrap();
    let first =
        crate::sidecar::register_child(u32::MAX - 7, crate::sidecar::ChildOwner::Background);
    let stale = first.cancellation();
    drop(first);
    let second =
        crate::sidecar::register_child(u32::MAX - 7, crate::sidecar::ChildOwner::Background);
    stale.stop().unwrap();
    assert!(!second.stopped());
}
