use super::*;
use serde_json::json;

fn add(history: &History, run: Value, site: &str, status: &str, time: i64) -> String {
    let mut event = Event::start("run");
    event.time_ms = Some(time);
    event.case_id = Some(site.into());
    event.case_path = Some(format!("/cases/{site}"));
    event.detail = json!({"run_id":run,"model_run_dir":format!("/cases/{site}/runs/one")});
    history.append(&event).unwrap();
    if status != "running" {
        let mut end = event.next(EventPhase::Finished, status);
        end.time_ms = Some(time + 100);
        history.append(&end).unwrap();
    }
    event.operation_id
}
fn query() -> Query {
    Query {
        group_runs: true,
        ..Query::default()
    }
}

#[test]
fn groups_before_pagination_and_matches_complete_members() {
    let history = History::open(":memory:").unwrap();
    for i in 0..90 {
        add(
            &history,
            json!("batch"),
            &format!("site-{i:02}"),
            "succeeded",
            1000 + i,
        );
    }
    add(&history, json!("rerun"), "site-00", "succeeded", 2000);
    let q = Query {
        limit: 1,
        ascending: true,
        ..query()
    };
    let page = history.query(&q).unwrap();
    assert_eq!(page.total, 2);
    let group = page.items[0].run_group.as_ref().unwrap();
    assert_eq!(group.members.len(), 90);
    assert!(group.members.iter().all(|op| op.run_group.is_none()));
    assert_eq!(page.items[0].latest.duration_ms, Some(189));
    assert!(page.items[0].latest.case_path.is_none());
    assert!(page.items[0].latest.detail.get("model_run_dir").is_none());
    let search = history
        .query(&Query {
            search: Some("site-89".into()),
            ..query()
        })
        .unwrap();
    assert_eq!(search.total, 1);
    assert_eq!(
        search.items[0].run_group.as_ref().unwrap().members.len(),
        90
    );
    let scoped = history
        .query(&Query {
            case_path: Some("/cases/site-00".into()),
            ..query()
        })
        .unwrap();
    assert_eq!(scoped.total, 2);
    assert!(scoped.items.iter().all(|op| op.run_group.is_none()));
    let exported: Page =
        serde_json::from_str(&history.export_json(&Query { offset: 1, ..q }).unwrap()).unwrap();
    assert_eq!(exported.total, 2);
    assert_eq!(exported.items.len(), 2);
    assert!(history
        .export_csv(&query())
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .ends_with("run_id,member_count"));
    assert!(!history
        .export_csv(&Query::default())
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .contains("member_count"));
}

#[test]
fn only_valid_run_kinds_and_text_ids_group() {
    let history = History::open(":memory:").unwrap();
    for value in [
        Value::Null,
        json!(""),
        json!("  "),
        json!(42),
        json!({}),
        json!(["x"]),
    ] {
        add(&history, value.clone(), "a", "succeeded", 1000);
        add(&history, value, "b", "succeeded", 1000);
    }
    for _ in 0..2 {
        let mut e = Event::start("case-save");
        e.detail = json!({"run_id":"batch"});
        history.append(&e).unwrap();
    }
    add(&history, json!("batch"), "a", "succeeded", 1000);
    let page = history.query(&query()).unwrap();
    assert_eq!(page.total, 15);
    assert!(page.items.iter().all(|op| op.run_group.is_none()));
}

#[test]
fn aggregate_status_filters_and_uncertainty_preserve_running_members() {
    let history = History::open(":memory:").unwrap();
    add(&history, json!("batch"), "a", "succeeded", 1000);
    let running_id = add(&history, json!("batch"), "b", "running", 2000);
    let page = history
        .query(&Query {
            status: Some("running".into()),
            needs_reconciliation: Some(true),
            ..query()
        })
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].latest.status, "running");
    assert_eq!(page.items[0].latest.phase, EventPhase::Status);
    assert_eq!(page.items[0].finished_ms, None);
    assert_eq!(page.items[0].latest.duration_ms, None);
    assert_eq!(
        history
            .query(&Query {
                status: Some("succeeded".into()),
                ..query()
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        history
            .query(&Query {
                from_ms: Some(1500),
                ..query()
            })
            .unwrap()
            .total,
        0
    );
    let event = history
        .detail(&running_id)
        .unwrap()
        .unwrap()
        .operation
        .latest;
    history
        .append(&event.next(EventPhase::Finished, "failed"))
        .unwrap();
    assert_eq!(
        history
            .query(&Query {
                status: Some("completed_with_failures".into()),
                needs_reconciliation: Some(false),
                ..query()
            })
            .unwrap()
            .total,
        1
    );
}

#[test]
fn run_removal_is_atomic_and_retains_audit_and_other_runs() {
    let history = History::open(":memory:").unwrap();
    let first = add(&history, json!("batch"), "a", "succeeded", 1000);
    let active = add(&history, json!("batch"), "b", "running", 2000);
    add(&history, json!("other"), "a", "succeeded", 3000);
    assert!(history.remove_run(&first).is_err());
    assert_eq!(history.query(&Query::default()).unwrap().total, 3);
    let event = history.detail(&active).unwrap().unwrap().operation.latest;
    history
        .append(&event.next(EventPhase::Finished, "cancelled"))
        .unwrap();
    assert!(history.remove_run(&first).unwrap());
    assert!(!history.remove_run(&first).unwrap());
    assert_eq!(history.query(&query()).unwrap().total, 1);
    assert_eq!(history.query_audit(&Query::default()).unwrap().total, 3);
    assert_eq!(history.query_audit(&query()).unwrap().total, 2);
    assert!(history.detail(&active).unwrap().is_none());
    assert_eq!(
        history.detail_audit(&active).unwrap().unwrap().events.len(),
        2
    );
}

#[test]
fn terminal_statuses_are_aggregated_without_losing_failures() {
    for (statuses, expected) in [
        (vec!["succeeded", "succeeded"], "succeeded"),
        (vec!["failed", "failed"], "failed"),
        (vec!["succeeded", "failed"], "completed_with_failures"),
        (vec!["cancelled", "cancelled"], "cancelled"),
        (vec!["cancelled", "succeeded"], "cancelled"),
        (vec!["cancelled", "failed"], "completed_with_failures"),
        (vec!["succeeded", "unknown"], "unknown"),
    ] {
        let history = History::open(":memory:").unwrap();
        for (i, status) in statuses.into_iter().enumerate() {
            add(&history, json!("batch"), &i.to_string(), status, 1000);
        }
        let page = history
            .query(&Query {
                status: Some(expected.into()),
                ..query()
            })
            .unwrap();
        assert_eq!(page.total, 1, "{expected}");
        assert_eq!(page.items[0].latest.status, expected);
    }
}

#[test]
fn singleton_status_and_raw_serialization_remain_unchanged() {
    let history = History::open(":memory:").unwrap();
    let id = add(&history, json!("one"), "a", "completed", 1000);
    let page = history
        .query(&Query {
            status: Some("completed".into()),
            ..query()
        })
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].latest.status, "completed");
    assert!(page.items[0].run_group.is_none());
    assert!(!serde_json::to_value(&page.items[0])
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("run_group"));
    assert!(history.remove_run(&id).unwrap());
    let plain = add(&history, Value::Null, "a", "succeeded", 2000);
    assert!(history.remove_run(&plain).unwrap());
}

#[test]
fn member_filters_keep_complete_group_but_metadata_is_only_common() {
    let history = History::open(":memory:").unwrap();
    for (i, kind) in ["run", "remote-run"].into_iter().enumerate() {
        let mut event = Event::start(kind);
        event.case_path = Some(format!("/site/{i}"));
        event.version_id = Some(format!("v{i}"));
        event.host = Some(format!("host{i}"));
        event.location = Some("local".into());
        event.detail = json!({"run_id":"batch"});
        history.append(&event).unwrap();
        history
            .append(&event.next(EventPhase::Finished, "succeeded"))
            .unwrap();
    }
    let page = history
        .query(&Query {
            kind: Some("remote-run".into()),
            host: Some("host1".into()),
            version_id: Some("v1".into()),
            ..query()
        })
        .unwrap();
    assert_eq!(page.total, 1);
    let op = &page.items[0];
    assert_eq!(op.run_group.as_ref().unwrap().members.len(), 2);
    assert!(op.latest.host.is_none());
    assert!(op.latest.version_id.is_none());
    assert_eq!(op.latest.location.as_deref(), Some("local"));
}

#[test]
fn removed_representative_still_resolves_remaining_run_members() {
    let history = History::open(":memory:").unwrap();
    let first = add(&history, json!("batch"), "a", "succeeded", 1000);
    let second = add(&history, json!("batch"), "b", "succeeded", 1000);
    assert!(history.remove_operation(&first).unwrap());
    assert!(history.remove_run(&first).unwrap());
    assert!(history.detail(&second).unwrap().is_none());
    assert_eq!(history.query_audit(&Query::default()).unwrap().total, 2);
}

#[test]
fn homogeneous_remote_states_keep_their_status_without_claiming_completion() {
    for status in ["submitted", "queued", "cancel_requested"] {
        let history = History::open(":memory:").unwrap();
        for site in ["a", "b"] {
            let mut event = Event::start("remote-run");
            event.status = status.into();
            event.case_id = Some(site.into());
            event.detail = json!({"run_id": "batch"});
            history.append(&event).unwrap();
        }
        let page = history
            .query(&Query {
                status: Some(status.into()),
                needs_reconciliation: Some(true),
                ..query()
            })
            .unwrap();
        assert_eq!(page.total, 1, "{status}");
        let operation = &page.items[0];
        assert_eq!(operation.latest.status, status);
        assert_eq!(operation.latest.phase, EventPhase::Status);
        assert!(operation.needs_reconciliation);
        assert_eq!(operation.finished_ms, None);
        assert_eq!(operation.latest.duration_ms, None);
        assert_eq!(operation.run_group.as_ref().unwrap().members.len(), 2);
    }
}

#[test]
fn batched_hydration_preserves_pagination_order_and_complete_exports() {
    let history = History::open(":memory:").unwrap();
    let events = (0..1005)
        .map(|i| {
            let mut event = Event::start("run");
            event.time_ms = Some(i);
            event.detail = json!({"run_id": format!("batch-{i}")});
            event
        })
        .collect::<Vec<_>>();
    history.append_batch(&events).unwrap();
    let started = std::time::Instant::now();
    let page = history
        .query(&Query {
            limit: 1005,
            ascending: true,
            ..query()
        })
        .unwrap();
    eprintln!(
        "grouped hydration of 1005 singleton rows: {:?}",
        started.elapsed()
    );
    assert_eq!(page.total, 1005);
    assert_eq!(page.items.len(), 1005);
    for (i, item) in page.items.iter().enumerate() {
        assert_eq!(item.latest.operation_id, events[i].operation_id);
        assert!(item.run_group.is_none());
    }
    let page = history
        .query(&Query {
            offset: 499,
            limit: 501,
            ascending: true,
            ..query()
        })
        .unwrap();
    assert_eq!(
        page.items.first().unwrap().latest.operation_id,
        events[499].operation_id
    );
    assert_eq!(
        page.items.last().unwrap().latest.operation_id,
        events[999].operation_id
    );
    let started = std::time::Instant::now();
    let exported: Page = serde_json::from_str(
        &history
            .export_json(&Query {
                offset: 900,
                limit: 1,
                ascending: true,
                ..query()
            })
            .unwrap(),
    )
    .unwrap();
    eprintln!(
        "grouped export of 1005 singleton rows: {:?}",
        started.elapsed()
    );
    assert_eq!(exported.total, 1005);
    assert_eq!(exported.items.len(), 1005);
    assert_eq!(
        exported.items.last().unwrap().latest.operation_id,
        events[1004].operation_id
    );
}
