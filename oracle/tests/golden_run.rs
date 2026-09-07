#[test]
fn invalid_case_argument_is_rejected_before_external_data_or_workdir_access() {
    for name in ["../outside", "/tmp/outside", ".", ".."] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_golden-run"))
            .arg(name)
            .env_remove("PLUMBER2_ROOT")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("DEF_CASE_NAME"), "{name}: {error}");
        assert!(!error.contains("PLUMBER2_ROOT is not set"), "{error}");
    }
}
