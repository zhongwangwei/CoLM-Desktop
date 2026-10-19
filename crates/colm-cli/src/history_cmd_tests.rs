use super::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "colm-history-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn dispatch_unknown_does_not_open_database() {
    let flags = Flags::from([(
        "--history-db".into(),
        "/does-not-exist/history.sqlite".into(),
    )]);
    assert!(!dispatch("version", &[], &flags).unwrap());
}
#[test]
fn query_filter_parse_and_default_root_share_database_parent() {
    let flags = Flags::from([
        ("--history-db".into(), "/tmp/custom/history.sqlite".into()),
        ("--json".into(), r#"{"kind":"run","limit":7}"#.into()),
    ]);
    assert_eq!(
        model_root(&flags).unwrap(),
        PathBuf::from("/tmp/custom/models")
    );
    assert_eq!(request(&flags).unwrap().limit, 7);
}
#[test]
fn audit_classification_does_not_log_read_only_or_auth_commands() {
    assert_eq!(action_kind("run"), Some("run"));
    assert_eq!(action_kind("history-list"), None);
    // Status polls reconcile the run they observe instead of becoming operations.
    assert_eq!(action_kind("remote-status"), None);
    assert_eq!(action_kind("remote-study-status"), None);
    assert_eq!(action_kind("remote-fetch"), Some("fetch"));
    assert_eq!(action_kind("series"), None);
    let flags = Flags::from([
        ("--password".into(), "credential".into()),
        ("--json".into(), "private payload".into()),
        ("--engine".into(), "rust".into()),
    ]);
    assert_eq!(
        safe_flags(&flags),
        Flags::from([("--engine".into(), "rust".into())])
    );
    assert!(validate_id("../escape").is_err());
}

#[test]
fn durable_remote_intent_binds_the_original_source_case_and_blocks_new_copies() {
    let temp = Temp::new();
    std::fs::write(
        temp.0.join("case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'fixture'\n/\n",
    )
    .unwrap();
    let source = temp.0.canonicalize().unwrap();
    let database = temp.0.join("history.sqlite");
    let history = History::open(&database).unwrap();
    let mut initial = Event::start("run");
    initial.operation_id = "original-operation".into();
    initial.case_path = Some(source.to_string_lossy().into_owned());
    initial.detail = json!({"action":"remote-run"});
    history.append(&initial).unwrap();
    let opts = crate::Opts {
        positional: vec![source.to_string_lossy().into_owned()],
        flags: vec![
            (
                "--history-db".into(),
                database.to_string_lossy().into_owned(),
            ),
            ("--operation-id".into(), initial.operation_id.clone()),
        ],
    };
    let record = json!({"host":"server","root":"/data/colm","job":"original-job","engine":"0123456789abcdef","remote_case":"/data/colm/cases/original","provenance":{"operation_id":initial.operation_id}});
    record_remote_intent(&opts, &record, &source).unwrap();
    let saved = history
        .detail(&initial.operation_id)
        .unwrap()
        .unwrap()
        .operation
        .latest;
    assert_eq!(saved.status, "needs_reconciliation");
    assert_eq!(saved.case_path, initial.case_path);
    assert_eq!(saved.detail["remote_record"]["job"], "original-job");
    let mut retry = opts;
    retry.flags.retain(|(key, _)| key != "--operation-id");
    retry
        .flags
        .push(("--operation-id".into(), "different-copy-operation".into()));
    assert!(refuse_unresolved_remote_submission(&retry, &source).is_err());
    let outcome =
        json!({"job":"original-job","terminal":true,"status":{"state":"finished","exit_code":0}});
    reconcile_remote_history(&history, &record, &outcome, Some(&source)).unwrap();
    assert!(refuse_unresolved_remote_submission(&retry, &source).is_ok());

    // Importing the case's own run record of that job adds no placeholder (天河 2026-10-19).
    std::fs::write(temp.0.join(".colm-remote.json"), record.to_string()).unwrap();
    let report = history.import_case(&source).unwrap();
    assert_eq!((report.imported, report.duplicates), (0, 1));
    // Placeholders imported before this check existed no longer refuse a new run either.
    let mut placeholder =
        Event::start("remote_run").next(EventPhase::Imported, "needs_reconciliation");
    placeholder.operation_id = "legacy-old".into();
    placeholder.case_path = Some(source.to_string_lossy().into_owned());
    placeholder.detail = json!({"legacy": record});
    history.append(&placeholder).unwrap();
    assert!(refuse_unresolved_remote_submission(&retry, &source).is_ok());
    // One for a job the app never recorded still does: nothing else will settle it.
    let mut stranger =
        Event::start("remote_run").next(EventPhase::Imported, "needs_reconciliation");
    stranger.operation_id = "legacy-stranger".into();
    stranger.case_path = Some(source.to_string_lossy().into_owned());
    stranger.detail = json!({"legacy": {"host":"server","job":"unknown-job"}});
    history.append(&stranger).unwrap();
    assert!(refuse_unresolved_remote_submission(&retry, &source)
        .unwrap_err()
        .to_string()
        .contains("unknown-job"));
}

#[test]
fn submitting_cli_completion_does_not_overwrite_an_observed_terminal_state() {
    let temp = Temp::new();
    let database = temp.0.join("history.sqlite");
    let history = History::open(&database).unwrap();
    let initial = Event::start("run");
    history.append(&initial).unwrap();
    let terminal = initial.next(EventPhase::Finished, "succeeded");
    history.append(&terminal).unwrap();
    observe_remote_outcome(
        &json!({"job":"job","outcome":"submitted","terminal":false,"remote_record":{"host":"server","root":"/data/colm","job":"job","engine":"0123456789abcdef","remote_case":"/data/colm/cases/run"}}),
    );
    finish(
        AuditToken {
            event: initial.clone(),
            database,
            command: "remote-run".into(),
            positional: vec![],
            flags: Flags::new(),
        },
        &Ok(()),
    )
    .unwrap();
    assert_eq!(
        history
            .detail(&initial.operation_id)
            .unwrap()
            .unwrap()
            .operation
            .latest
            .event_id,
        terminal.event_id
    );
}
#[test]
fn imported_or_missing_frozen_records_cannot_authorize_replay() {
    let h = History::open(":memory:").unwrap();
    let e = Event::start("run");
    h.append(&e).unwrap();
    let d = h.detail(&e.operation_id).unwrap().unwrap();
    assert!(frozen_run(&d, Path::new("/tmp")).is_err());
    let mut imported = e.next(EventPhase::Imported, "succeeded");
    imported.operation_id = "imported".into();
    h.append(&imported).unwrap();
    assert!(frozen_run(&h.detail("imported").unwrap().unwrap(), Path::new("/tmp")).is_err());
}
#[test]
fn compare_without_evidence_never_ranks_runs() {
    let h = History::open(":memory:").unwrap();
    for id in ["a", "b"] {
        let mut e = Event::start("run");
        e.operation_id = id.into();
        h.append(&e).unwrap();
    }
    let value = compare(
        &h,
        &Flags::from([
            ("--a".into(), "a".into()),
            ("--b".into(), "b".into()),
            ("--model-root".into(), "/tmp/models".into()),
        ]),
    )
    .unwrap();
    assert!(value["better"].is_null());
    assert_eq!(value["same_design"], false);
}
#[test]
fn sampled_scientific_identity_is_not_full_content_evidence() {
    let value = json!({"input_fingerprint":{"files":{"/data/forcing.nc":"sample-sha256:abc:1024"},"site_sha256":"abc"}});
    assert!(scientific_inputs(&value, Path::new("/missing-frozen-run"))
        .unwrap_err()
        .to_string()
        .contains("sampled"));
}
#[test]
fn verified_study_import_uses_reader_and_rejects_corruption() {
    let dir = Temp::new();
    let state = crate::study::state::StudyState::new("old-study".into(), []).unwrap();
    let checkpoint = dir.0.join("checkpoints/state");
    crate::study::checkpoint::write_next(&checkpoint, &state).unwrap();
    let h = History::open(":memory:").unwrap();
    let positional = vec![dir.0.to_string_lossy().into_owned()];
    let flags = Flags::from([("--kind".into(), "study".into())]);
    assert_eq!(import(&h, &positional, &flags).unwrap().imported, 1);
    assert_eq!(import(&h, &positional, &flags).unwrap().duplicates, 1);
    std::fs::write(checkpoint.join("000000000001.json"), "{broken").unwrap();
    assert!(import(&h, &positional, &flags).is_err());
}

#[test]
fn remote_submit_audit_is_uncertain_and_failure_is_terminal() {
    let dir = Temp::new();
    let database = dir.0.join("history.sqlite");
    let h = History::open(&database).unwrap();
    for (command, success) in [("remote-run", true), ("remote-run", false), ("run", false)] {
        let event = Event::start("run");
        let id = event.operation_id.clone();
        h.append(&event).unwrap();
        let token = AuditToken {
            event,
            database: database.clone(),
            command: command.into(),
            positional: vec![],
            flags: Flags::from([(
                "--model-root".into(),
                dir.0.join("models").to_string_lossy().into_owned(),
            )]),
        };
        let outcome = if success {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "failure containing PASSWORD=must-not-persist"
            ))
        };
        finish(token, &outcome).unwrap();
        let detail = h.detail(&id).unwrap().unwrap();
        assert_eq!(
            detail.operation.latest.status,
            if success { "submitted" } else { "failed" }
        );
        assert_eq!(
            detail.operation.needs_reconciliation,
            command == "remote-run" && success
        );
        assert!(!serde_json::to_string(&detail)
            .unwrap()
            .contains("must-not-persist"));
    }
}

#[test]
fn remote_preflight_failure_is_terminal_but_a_reserved_submission_stays_uncertain() {
    for command in ["remote-run", "remote-study-run"] {
        assert_eq!(
            completion(command, false, None),
            (EventPhase::Finished, "failed")
        );
        let uncertain = json!({"outcome":"uncertain","remote_record":{"job":"reserved-job"}});
        assert_eq!(
            completion(command, false, Some(&uncertain)),
            (EventPhase::Status, "needs_reconciliation")
        );
    }
    let dir = Temp::new();
    let database = dir.0.join("history.sqlite");
    let history = History::open(&database).unwrap();
    let mut event = Event::start("run");
    event.detail = json!({"model_run":{"operation_id":event.operation_id,"run_case":"materialized-but-not-submitted"}});
    history.append(&event).unwrap();
    REMOTE_OUTCOME.with(|slot| slot.borrow_mut().take());
    finish(
        AuditToken {
            event: event.clone(),
            database,
            command: "remote-run".into(),
            positional: vec![],
            flags: Flags::from([(
                "--model-root".into(),
                dir.0.join("models").to_string_lossy().into_owned(),
            )]),
        },
        &Err(anyhow::anyhow!("input has no server mapping")),
    )
    .unwrap();
    let recorded = history
        .detail(&event.operation_id)
        .unwrap()
        .unwrap()
        .operation;
    assert_eq!(recorded.latest.phase, EventPhase::Finished);
    assert_eq!(recorded.latest.status, "failed");
    assert!(!recorded.needs_reconciliation);
}

#[test]
fn export_writes_complete_json_and_refuses_overwrite() {
    let dir = Temp::new();
    let database = dir.0.join("history.sqlite");
    let output = dir.0.join("export.json");
    let h = History::open(&database).unwrap();
    for _ in 0..3 {
        h.append(&Event::start("run")).unwrap();
    }
    let flags = Flags::from([
        (
            "--history-db".into(),
            database.to_string_lossy().into_owned(),
        ),
        ("--out".into(), output.to_string_lossy().into_owned()),
        ("--json".into(), r#"{"limit":1,"offset":2}"#.into()),
    ]);
    assert!(dispatch("history-export", &[], &flags).unwrap());
    let value: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 3);
    assert!(dispatch("history-export", &[], &flags).is_err());
}

#[test]
fn cancellation_request_never_implies_remote_termination() {
    for command in ["remote-cancel", "remote-study-cancel"] {
        assert!(action_kind(command).is_some());
        assert_eq!(
            completion(command, true, None),
            (EventPhase::Status, "needs_reconciliation")
        );
        assert_eq!(
            completion(command, false, None),
            (EventPhase::Status, "needs_reconciliation")
        );
        let requested = json!({"outcome":"cancel_requested","request_accepted":true,"terminal":false,"state":"running"});
        assert_eq!(
            completion(command, true, Some(&requested)),
            (EventPhase::Status, "cancel_requested")
        );
        let uncertain = json!({"outcome":"uncertain","request_accepted":true,"terminal":false,"state":"unknown"});
        assert_eq!(
            completion(command, true, Some(&uncertain)),
            (EventPhase::Status, "needs_reconciliation")
        );
        let ended =
            json!({"outcome":"already_finished","terminal":true,"state":"finished","exit_code":0});
        assert_eq!(
            completion(command, true, Some(&ended)),
            (EventPhase::Finished, "succeeded")
        );
        let missing_code = json!({"outcome":"already_finished","terminal":true,"state":"finished"});
        assert_eq!(
            completion(command, true, Some(&missing_code)),
            (EventPhase::Status, "needs_reconciliation")
        );
    }
}

#[test]
fn remote_evidence_excludes_log_and_transport_secrets() {
    observe_remote_outcome(
        &json!({"outcome":"uncertain","terminal":false,"cancel_error":"secret-password","status":{"state":"unknown","log_tail":"secret-token"}}),
    );
    let value = REMOTE_OUTCOME
        .with(|slot| slot.borrow_mut().take())
        .unwrap();
    let text = serde_json::to_string(&value).unwrap();
    assert!(!text.contains("secret"));
    assert_eq!(value["state"], "unknown");
}

#[test]
fn comparison_requires_complete_versioned_data_digest() {
    let dir = Temp::new();
    std::fs::write(dir.0.join("case.nml"), "&nl_colm\n/\n").unwrap();
    let mut identity = crate::fingerprint::FullInputIdentity::unavailable("dynamic dependency");
    assert!(complete_data_identity(&identity, &dir.0).is_err());
    identity.complete = true;
    identity.unresolved.clear();
    identity.data_sha256 = Some("a".repeat(64));
    assert_eq!(
        complete_data_identity(&identity, &dir.0).unwrap(),
        "a".repeat(64)
    );
    identity.algorithm = "sample-sha256".into();
    assert!(complete_data_identity(&identity, &dir.0).is_err());
}

#[test]
fn comparison_rejects_legacy_identity_without_valid_declared_site_hash() {
    let dir = Temp::new();
    std::fs::write(
        dir.0.join("case.nml"),
        "&nl_colm\nSITE_fsitedata='/server/site.nc'\n/\n",
    )
    .unwrap();
    let mut identity = crate::fingerprint::FullInputIdentity::unavailable("legacy");
    identity.complete = true;
    identity.unresolved.clear();
    identity.data_sha256 = Some("a".repeat(64));
    for location in ["server", "local"] {
        let provenance = json!({"location":{"kind":location},"full_input_identity":identity});
        let error = scientific_inputs(&provenance, &dir.0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SITE_fsitedata"), "{error}");
    }
    let site = crate::fingerprint::FullInputFile {
        path: "/server/site.nc".into(),
        sha256: "b".repeat(64),
        bytes: 123,
        category: "data".into(),
    };
    identity
        .files
        .insert("case:site_fsitedata".into(), site.clone());
    assert_eq!(
        complete_data_identity(&identity, &dir.0).unwrap(),
        "a".repeat(64)
    );
    for invalid in [
        crate::fingerprint::FullInputFile {
            sha256: "a".repeat(63),
            ..site.clone()
        },
        crate::fingerprint::FullInputFile {
            sha256: "g".repeat(64),
            ..site.clone()
        },
        crate::fingerprint::FullInputFile {
            bytes: 0,
            ..site.clone()
        },
        crate::fingerprint::FullInputFile {
            category: "configuration".into(),
            ..site
        },
    ] {
        identity.files.insert("case:site_fsitedata".into(), invalid);
        for location in ["server", "local"] {
            let provenance = json!({"location":{"kind":location},"full_input_identity":identity});
            let error = scientific_inputs(&provenance, &dir.0)
                .unwrap_err()
                .to_string();
            assert!(error.contains("SITE_fsitedata"), "{error}");
        }
    }
}

#[test]
fn dry_run_never_creates_a_submission_audit() {
    let dir = Temp::new();
    let database = dir.0.join("not-created.sqlite");
    let flags = Flags::from([
        ("--dry-run".into(), "1".into()),
        (
            "--history-db".into(),
            database.to_string_lossy().into_owned(),
        ),
    ]);
    assert!(begin("remote-run", &[], &flags).unwrap().is_none());
    assert!(!database.exists());
    for command in [
        "model-migrate-case",
        "model-import-process",
        "study-save-version",
        "ws-build-engine",
        "remote-study-cancel",
    ] {
        assert!(action_kind(command).is_some(), "{command}");
    }
}

#[test]
fn assistant_initiator_is_explicit_and_unknown_values_are_rejected() {
    assert_eq!(validated_initiator(None).unwrap(), "cli");
    assert_eq!(validated_initiator(Some("assistant")).unwrap(), "assistant");
    assert!(validated_initiator(Some("arbitrary-user")).is_err());
}

#[test]
fn remote_replay_submission_is_pending_and_reconciliation_checks_job_binding() {
    assert_eq!(
        completion(
            "history-replay",
            true,
            Some(&json!({"outcome":"submitted"}))
        ),
        (EventPhase::Status, "submitted")
    );
    let history = History::open(":memory:").unwrap();
    let mut start = Event::start("remote-run");
    start.operation_id = "remote-operation".into();
    start.case_path = Some("/cases/source".into());
    start.detail = json!({"action":"remote-run","remote_record":{"host":"7920l","root":"/data/jobs","job":"job-a","engine":"engine-a","remote_case":"/data/jobs/cases/a"}});
    history.append(&start).unwrap();
    let record = json!({"host":"7920l","root":"/data/jobs","job":"job-a","engine":"engine-a","remote_case":"/data/jobs/cases/a","provenance":{"operation_id":"remote-operation","model_run":{"source_case":"/cases/source"}}});
    let mut legacy = record.clone();
    legacy["provenance"]
        .as_object_mut()
        .unwrap()
        .remove("operation_id");
    assert_eq!(
        remote_operation_id(&history, &legacy).unwrap().as_deref(),
        Some("remote-operation")
    );
    let wrong = json!({"job":"job-b","terminal":true,"status":{"state":"finished","exit_code":0}});
    assert!(reconcile_remote_history(&history, &record, &wrong, None).is_err());
    assert_eq!(
        history
            .detail("remote-operation")
            .unwrap()
            .unwrap()
            .operation
            .event_count,
        1
    );
    let uncertain =
        json!({"job":"job-a","outcome":"uncertain","terminal":false,"status":{"state":"lost"}});
    reconcile_remote_history(&history, &record, &uncertain, None).unwrap();
    assert!(
        history
            .detail("remote-operation")
            .unwrap()
            .unwrap()
            .operation
            .needs_reconciliation
    );
    let queued =
        json!({"job":"job-a","outcome":"observed","terminal":false,"status":{"state":"queued"}});
    reconcile_remote_history(&history, &record, &queued, None).unwrap();
    assert_eq!(
        history
            .detail("remote-operation")
            .unwrap()
            .unwrap()
            .operation
            .latest
            .status,
        "queued"
    );
    let done = json!({"job":"job-a","outcome":"observed","terminal":true,"status":{"state":"finished","exit_code":0}});
    reconcile_remote_history(&history, &record, &done, None).unwrap();
    let detail = history.detail("remote-operation").unwrap().unwrap();
    assert_eq!(detail.operation.latest.status, "succeeded");
    assert!(!detail.operation.needs_reconciliation);
    let count = detail.operation.event_count;
    reconcile_remote_history(&history, &record, &done, None).unwrap();
    assert_eq!(
        history
            .detail("remote-operation")
            .unwrap()
            .unwrap()
            .operation
            .event_count,
        count
    );
    let mut wrong_host = record.clone();
    wrong_host["host"] = json!("other-host");
    assert!(reconcile_remote_history(&history, &wrong_host, &done, None).is_err());
}

#[test]
fn a_finished_remote_run_keeps_its_last_stage_and_step_but_no_log_text() {
    let history = History::open(":memory:").unwrap();
    let mut start = Event::start("remote-run");
    start.operation_id = "remote-operation".into();
    start.case_path = Some("/cases/source".into());
    start.detail = json!({"action":"remote-run","remote_record":{"host":"land02","root":"/data/jobs","job":"job-a","engine":"engine-a","remote_case":"/data/jobs/cases/a"}});
    history.append(&start).unwrap();
    let record = json!({"host":"land02","root":"/data/jobs","job":"job-a","engine":"engine-a","remote_case":"/data/jobs/cases/a","provenance":{"operation_id":"remote-operation","model_run":{"source_case":"/cases/source"}}});
    let log = "token=secret\n=== colm-stage colm begin ===\nTIMESTEP = 87645 | DATE = 2006-12-31-79200\nTIMESTEP = 87646 | DATE = 2006-12-31-81000\n=== colm-stage colm ok ===\n";
    let running = json!({"job":"job-a","outcome":"observed","terminal":false,"status":{"state":"running","log_tail":log}});
    reconcile_remote_history(&history, &record, &running, None).unwrap();
    let latest = || {
        history
            .detail("remote-operation")
            .unwrap()
            .unwrap()
            .operation
            .latest
    };
    assert!(
        latest().detail.get("final_progress").is_none(),
        "only a finished run records it"
    );
    let done = json!({"job":"job-a","outcome":"observed","terminal":true,"status":{"state":"finished","exit_code":0,"log_tail":log}});
    reconcile_remote_history(&history, &record, &done, None).unwrap();
    assert_eq!(
        latest().detail["final_progress"],
        json!({"stages":[["colm","ok"]],"step":87646,"date":"2006-12-31-81000"})
    );
    assert!(!latest().detail.to_string().contains("secret"));
    // 取回时不读日志（`lines` 为 0）：已记下的最终进度不被一次空观察抹掉。
    let fetched = json!({"job":"job-a","outcome":"observed","terminal":true,"status":{"state":"finished","exit_code":0,"log_tail":""}});
    reconcile_remote_history(&history, &record, &fetched, None).unwrap();
    assert_eq!(latest().detail["final_progress"]["step"], 87646);
}

#[test]
fn history_finishes_and_authorizes_registered_case_local_and_legacy_runs() {
    for legacy in [false, true] {
        let temp = Temp::new();
        let root = temp.0.join("models");
        let registry = colm_model::Registry::open(&root).unwrap();
        let official = registry
            .install_official(colm_model::CodeRef {
                kind: colm_model::CodeKind::Official,
                workspace: None,
                commit: None,
                source_sha256: "a".repeat(64),
            })
            .unwrap();
        let case = temp.0.join("case");
        std::fs::create_dir(&case).unwrap();
        std::fs::write(
            case.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'fixture'\n DEF_dir_output = 'out'\n/\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("runs")).unwrap();
        let index = root.join("runs/operation");
        let run = if legacy {
            registry
                .materialize_run(&official.version, &case, &index, "operation")
                .unwrap()
        } else {
            let run = registry
                .materialize_case_run(&official.version, &case, &case, "operation")
                .unwrap();
            std::fs::create_dir(&index).unwrap();
            std::fs::write(
                index.join(colm_model::RUN_MANIFEST),
                serde_json::to_vec(&run).unwrap(),
            )
            .unwrap();
            run
        };
        let database = temp.0.join("history.sqlite");
        let history = History::open(&database).unwrap();
        let mut initial = Event::start("run");
        initial.operation_id = "operation".into();
        history.append(&initial).unwrap();
        finish(
            AuditToken {
                event: initial,
                database,
                command: "run".into(),
                positional: vec![case.to_string_lossy().into_owned()],
                flags: Flags::from([("--model-root".into(), root.to_string_lossy().into_owned())]),
            },
            &Ok(()),
        )
        .unwrap();
        let detail = history.detail("operation").unwrap().unwrap();
        assert_eq!(
            event_value(&detail, "model_run").unwrap()["run_case"],
            json!(run.run_case)
        );
        assert_eq!(frozen_run(&detail, &root).unwrap().run_case, run.run_case);
        if !legacy {
            let mut changed = run.clone();
            changed.source_case = temp.0.clone();
            std::fs::write(
                index.join(colm_model::RUN_MANIFEST),
                serde_json::to_vec(&changed).unwrap(),
            )
            .unwrap();
            assert!(frozen_run(&detail, &root).is_err());
        }
    }
}

#[test]
fn nonforcing_digest_never_authorizes_complete_input_comparison() {
    let mut identity = crate::fingerprint::FullInputIdentity::unavailable("fixture");
    identity.algorithm = "sha256-nonforcing-v1".into();
    identity.complete = true;
    identity.unresolved.clear();
    identity.data_sha256 = Some("a".repeat(64));
    identity
        .data_settings
        .insert("forcing_content_policy".into(), "excluded-v1".into());
    // Rejection precedes opening the case or attempting any actual input verification.
    assert!(
        complete_data_identity(&identity, Path::new("/missing-case"))
            .unwrap_err()
            .to_string()
            .contains("unsupported full-content")
    );
}
