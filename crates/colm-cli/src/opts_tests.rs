use super::Opts;

/// `--note`/`--directive` 承载自由文本，必须允许以 `--` 开头；其它 flag 仍拒绝，避免漏值吞掉下一个选项。
#[test]
fn free_text_flags_accept_values_starting_with_dashes() {
    let opts = Opts::parse(&["--note".into(), "--example".into()]).unwrap();
    assert_eq!(opts.get("--note").as_deref(), Some("--example"));
    let opts = Opts::parse(&["--directive".into(), "--keep-going".into()]).unwrap();
    assert_eq!(opts.get("--directive").as_deref(), Some("--keep-going"));
    // 后面还有参数时用 `--note=TEXT`，避免把下一个 flag 吞成说明。
    let opts = Opts::parse(&["--note=--example".into(), "--name".into(), "x".into()]).unwrap();
    assert_eq!(opts.get("--note").as_deref(), Some("--example"));
    let error = Opts::parse(&["--note".into(), "--rev".into(), "main".into()])
        .err()
        .expect("a note followed by another flag must not silently swallow it");
    assert!(
        error.to_string().contains("--note needs a value"),
        "{error}"
    );
    assert!(error.to_string().contains("--note=TEXT"), "{error}");
    let error = Opts::parse(&["--member".into(), "--example".into()])
        .err()
        .expect("non-free-text flags must reject -- values");
    assert!(
        error.to_string().contains("--member needs a value"),
        "{error}"
    );
}
