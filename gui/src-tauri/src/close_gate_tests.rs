use super::*;

#[test]
fn an_unanswered_request_lets_the_next_close_through() {
    let gate = CloseGate::default();
    // 前端答复过（取消关闭）就重新计时；确认之后一律放行。
    *gate.asked.lock().unwrap() = Some(Instant::now() - UNANSWERED - Duration::from_secs(1));
    assert!(!gate.confirmed.load(Ordering::SeqCst));
    gate.answered();
    assert!(gate.asked.lock().unwrap().is_none());
    gate.confirmed.store(true, Ordering::SeqCst);
    assert!(gate.confirmed.load(Ordering::SeqCst));
}

#[test]
fn acknowledged_saving_cannot_be_bypassed_by_the_unanswered_timeout() {
    let gate = CloseGate::default();
    let old = Some(Instant::now() - UNANSWERED - Duration::from_secs(1));
    assert!(gate.unanswered_expired(old));
    gate.preparing.store(true, Ordering::SeqCst);
    assert!(!gate.unanswered_expired(old));
    assert!(
        gate.saving_expired(old),
        "a repeat close must offer explicit discard"
    );
    assert!(!gate.saving_expired(Some(Instant::now())));
    assert!(
        !gate.confirmed.load(Ordering::SeqCst),
        "elapsed time never confirms discard"
    );
    gate.answered();
    assert!(!gate.preparing.load(Ordering::SeqCst));
    assert!(!gate.unanswered_expired(None));
}
