use super::*;

fn temp_case(label: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-agent-act-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("case.nml"), format!("&nl_colm\n{body}/\n")).unwrap();
    dir
}

#[test]
fn values_are_typed_by_the_field_definitions() {
    let (value, group) = typed_value("DEF_USE_PC", "true").unwrap();
    assert_eq!(value, colm_namelist::Value::Bool(true));
    assert_eq!(group, Some("nl_colm"));
    assert_eq!(
        typed_value("DEF_USE_PC", ".FALSE.").unwrap().0,
        colm_namelist::Value::Bool(false)
    );
    assert!(typed_value("DEF_USE_PC", "maybe").is_err());
    assert!(typed_value("NOT_A_FIELD", "1").is_err());
    let (pft, _) = typed_value("DEF_PFT_VMAX25(3)", "41.5").unwrap();
    assert_eq!(pft.as_f64(), Some(41.5));
    assert!(typed_value("DEF_PFT_VMAX25(3)", "abc").is_err());
}

#[test]
fn fields_are_set_in_place_or_inserted_and_bad_ones_change_nothing() {
    let dir = temp_case("fields", " DEF_CASE_NAME = 'x'\n DEF_USE_PC = .false.\n");
    let ctx = ToolContext {
        project_root: dir.clone(),
        ..ToolContext::default()
    };
    let result = SetCaseFields
        .call(
            &json!({ "case": dir.display().to_string(), "fields": [
                { "name": "DEF_USE_PC", "value": "true" },
                { "name": "DEF_PFT_VMAX25(3)", "value": "40" },
            ]}),
            &ctx,
        )
        .unwrap();
    assert_eq!(result["changed"][0]["old"], ".false.");
    let text = std::fs::read_to_string(dir.join("case.nml")).unwrap();
    let document = colm_namelist::parse(&text).unwrap();
    assert_eq!(
        document.get("DEF_USE_PC"),
        Some(&colm_namelist::Value::Bool(true))
    );
    assert_eq!(
        document.get("DEF_PFT_VMAX25(3)").and_then(|v| v.as_f64()),
        Some(40.0)
    );
    // 第二个字段不合法时整份不写（先全部校验、最后才写盘）。
    let before = std::fs::read_to_string(dir.join("case.nml")).unwrap();
    assert!(SetCaseFields
        .call(
            &json!({ "case": dir.display().to_string(), "fields": [
                { "name": "DEF_USE_PC", "value": "false" },
                { "name": "DEF_USE_PC_TYPO", "value": "1" },
            ]}),
            &ctx,
        )
        .is_err());
    assert_eq!(
        std::fs::read_to_string(dir.join("case.nml")).unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn acting_tools_need_approval_and_describe_the_real_action() {
    for tool in tools() {
        if tool.name() == "scan_sites" {
            assert_eq!(tool.tier(), Tier::Read);
        } else {
            assert_eq!(tool.tier(), Tier::Act, "{}", tool.name());
        }
    }
    let summary = CreateCase.summary(&json!({
        "site": "/d/CA-Qfo_site.nc", "out": "/p/CA-Qfo", "name": null, "start": "2004-01-01",
        "end": "2005-12-31", "met": null, "mode": "pc", "spinup_years": 1, "spinup_repeat": 1,
    }));
    assert!(
        summary.contains("/p/CA-Qfo") && summary.contains("pc") && summary.contains("2004-01-01"),
        "{summary}"
    );
    assert!(RunCase
        .summary(&json!({ "case": "/p/x", "stage": null, "engine": null, "force": true }))
        .contains("强制重跑"));
}

#[test]
fn running_needs_a_kernel_and_a_real_case() {
    let ctx = ToolContext::default();
    let dir = temp_case("run", "");
    let error = RunCase
        .call(&json!({ "case": dir.display().to_string(), "stage": null, "engine": null, "force": false }), &ctx)
        .unwrap_err();
    assert!(error.to_string().contains("no kernel is selected"));
    assert!(RunCase
        .call(
            &json!({ "case": "/nowhere", "stage": null, "engine": null, "force": false }),
            &ctx
        )
        .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bound_models_cannot_use_legacy_original_case_writes_or_runs() {
    let dir = temp_case("frozen-guard", " DEF_USE_PC = .false.\n");
    let before = std::fs::read(dir.join("case.nml")).unwrap();
    let ctx = ToolContext {
        model_context: Some(super::super::ModelContext {
            model: json!({"version":{"id":"old","revision":1},"code":{"source_sha256":"a".repeat(64)}}),
            model_root: dir.join("models"),
            source_dir: dir.clone(),
            docs_dir: None,
            source_case: Some(dir.clone()),
        }),
        ..ToolContext::default()
    };
    let error = SetCaseFields
        .call(
            &json!({"case":dir,"fields":[{"name":"DEF_USE_PC","value":"true"}]}),
            &ctx,
        )
        .unwrap_err();
    assert!(error.to_string().contains("frozen model parameters"));
    assert_eq!(std::fs::read(dir.join("case.nml")).unwrap(), before);
    let error = RunCase
        .call(
            &json!({"case":dir,"stage":null,"engine":null,"force":false}),
            &ctx,
        )
        .unwrap_err();
    assert!(error.to_string().contains("complete revision"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn cancellable_cli_keeps_full_output_and_stops_term_resistant_descendants() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_case("cancellable-cli", "");
    let cli = dir.join("fake-cli");
    std::fs::write(
        &cli,
        "#!/bin/sh\ni=0; while [ $i -lt 90 ]; do echo line-$i; i=$((i+1)); done\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctx = ToolContext {
        cli: cli.clone(),
        cancel: Some(signal.clone()),
        ..ToolContext::default()
    };
    assert_eq!(ctx.cli_cancellable(&[]).unwrap().lines().count(), 90);
    std::fs::write(&cli, format!("#!/bin/sh\ncd '{}' || exit 1\ntrap '' TERM\n(sleep 1; echo escaped > escaped) &\necho ready > ready\nwait\n", dir.display())).unwrap();
    let worker = std::thread::spawn(move || ctx.cli_cancellable(&[]));
    let began = std::time::Instant::now();
    while !dir.join("ready").exists() && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(dir.join("ready").exists());
    signal.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert!(!dir.join("escaped").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_cancelled_cli_request_does_not_spawn_the_command() {
    let ctx = ToolContext {
        cli: std::path::PathBuf::from("nonexistent-cancelled-cli"),
        cancel: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            true,
        ))),
        ..ToolContext::default()
    };
    assert!(ctx
        .cli_cancellable(&[])
        .unwrap_err()
        .to_string()
        .contains("not started"));
    assert!(ctx
        .cli_long(&[])
        .unwrap_err()
        .to_string()
        .contains("not started"));
}

#[cfg(unix)]
#[test]
fn cancellation_remains_live_after_a_cli_leader_exits_with_inherited_pipes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_case("exited-cli-leader", "");
    let cli = dir.join("fake-cli");
    std::fs::write(&cli, format!("#!/bin/sh\ncd '{}' || exit 1\ntrap '' TERM\n(sleep 2; echo escaped > escaped) &\necho ready > ready\nexit 0\n", dir.display())).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ctx = ToolContext {
        cli,
        cancel: Some(signal.clone()),
        ..ToolContext::default()
    };
    let worker = std::thread::spawn(move || ctx.cli_cancellable(&[]));
    let started = std::time::Instant::now();
    while !dir.join("ready").exists() && started.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(dir.join("ready").exists());
    std::thread::sleep(std::time::Duration::from_millis(250));
    let cancelled = std::time::Instant::now();
    signal.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    assert!(cancelled.elapsed() < std::time::Duration::from_millis(1500));
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(!dir.join("escaped").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
