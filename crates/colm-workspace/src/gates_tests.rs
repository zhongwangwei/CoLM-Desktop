#![allow(clippy::field_reassign_with_default)]
use super::*;

fn run(ok: bool, commit: &str) -> GateRun {
    GateRun {
        ok,
        at: 1,
        commit: commit.into(),
        detail: String::new(),
    }
}

#[test]
fn a_gate_measured_on_an_older_commit_is_stale() {
    let mut gates = Gates::default();
    assert_eq!(gates.lights("c2").compile, Light::Unknown);
    gates.engine = Some(run(true, "c1"));
    assert_eq!(gates.lights("c1").compile, Light::Pass);
    assert_eq!(gates.lights("c2").compile, Light::Stale);
    gates.kernels.insert("default".into(), run(false, "c2"));
    // 一个内核编不过就是不通过；其余是旧的也不改变这一点。
    assert_eq!(gates.lights("c2").compile, Light::Fail);
}

#[test]
fn registration_needs_compile_and_tests_on_the_current_commit() {
    let mut gates = Gates::default();
    gates.engine = Some(run(true, "c1"));
    assert!(!gates.may_register("c1"), "no tests yet");
    gates
        .tests
        .insert("cargo:colm-core".into(), run(true, "c1"));
    assert!(gates.may_register("c1"));
    assert!(!gates.may_register("c2"), "a new commit invalidates both");
    gates.regression = Some(Regression {
        kind: ChangeKind::Refactor,
        ok: false,
        at: 2,
        commit: "c1".into(),
        case: "ref".into(),
        identical: 3,
        changed: 4,
        verdict: "not bitwise".into(),
    });
    assert!(!gates.may_register("c1"), "a failed regression blocks it");
    // 回归是旧提交上测的：作废，不再拦。
    assert!(gates.may_register("c1") == (gates.lights("c1").regression != Light::Fail));
}

#[test]
fn gates_round_trip_through_json() {
    let mut gates = Gates::default();
    gates.engine = Some(run(true, "c1"));
    gates.parity = Some(ParityRecord {
        ok: true,
        at: 5,
        commit: "c1".into(),
        preset: "default".into(),
        case: "ref".into(),
        first_difference: None,
        rtol: 0.0,
        atol: 0.0,
        first_records: None,
        ignored: Vec::new(),
    });
    let text = serde_json::to_string(&gates).unwrap();
    assert_eq!(serde_json::from_str::<Gates>(&text).unwrap(), gates);
    let old: Gates = serde_json::from_str("{}").unwrap();
    assert!(old.engine.is_none() && old.kernels.is_empty() && old.tests.is_empty());
}

#[test]
fn every_recorded_test_kind_must_pass_on_the_current_commit() {
    let mut gates = Gates::default();
    gates.engine = Some(run(true, "c1"));
    gates
        .tests
        .insert("cargo:colm-core".into(), run(true, "c1"));
    gates.tests.insert("oracle".into(), run(false, "c1"));
    assert_eq!(gates.lights("c1").tests, Light::Fail);
    assert!(!gates.may_register("c1"));
    gates.tests.insert("oracle".into(), run(true, "c0"));
    assert_eq!(
        gates.lights("c1").tests,
        Light::Stale,
        "an old oracle run does not count"
    );
    gates.tests.insert("oracle".into(), run(true, "c1"));
    assert!(gates.may_register("c1"));
}

/// “两版一致”只在改了会影响计算结果的代码时需要：Fortran 上游、内核编译脚本、Rust 引擎的计算部分、依赖版本。
#[test]
fn parity_is_needed_only_for_changes_that_can_alter_results() {
    for path in [
        "vendor/CoLM202X/main/MOD_Const_LC.F90",
        "oracle/scripts/build_kernel.sh",
        "crates/colm-core/src/land_cover_generated.rs",
        "crates/colm-runtime/src/lib.rs",
        "crates/colm-numeric/src/lib.rs",
        "Cargo.lock",
    ] {
        assert!(parity_needed([path]), "{path}");
    }
    for path in [
        "crates/colm-hybrid/src/lib.rs",
        "crates/colm-agent/src/prompt.md",
        "crates/colm-workspace/src/gates.rs",
        "crates/colm-cli/src/ws_cmd.rs",
        "gui/dist/app/research.js",
        "docs/implementation-verification.md",
        "oracle/scripts/regress.sh",
        "crates/colm-core-extra/README.md",
    ] {
        assert!(!parity_needed([path]), "{path}");
    }
    // 一个文件碰了就需要；没有改动就不需要。
    assert!(parity_needed([
        "gui/dist/index.html",
        "crates/colm-core/src/lib.rs"
    ]));
    assert!(!parity_needed(std::iter::empty::<&str>()));
}

#[test]
fn parity_shows_not_needed_unless_it_was_actually_run_on_this_commit() {
    let mut gates = Gates::default();
    assert_eq!(gates.lights_for("h1", false).parity, Light::NotNeeded);
    assert_eq!(gates.lights_for("h1", true).parity, Light::Unknown);
    assert_eq!(
        gates.lights("h1").parity,
        Light::Unknown,
        "lights() keeps treating parity as needed"
    );
    let record = |ok: bool, commit: &str| ParityRecord {
        ok,
        at: 0,
        commit: commit.into(),
        preset: "default".into(),
        case: "/c".into(),
        first_difference: None,
        rtol: 0.0,
        atol: 0.0,
        first_records: None,
        ignored: Vec::new(),
    };
    // 在旧提交上做过：需要时是“需要重测”，不需要时是“不需要”。
    gates.parity = Some(record(true, "h0"));
    assert_eq!(gates.lights_for("h1", true).parity, Light::Stale);
    assert_eq!(gates.lights_for("h1", false).parity, Light::NotNeeded);
    // 在当前提交上真做过：照实显示，失败也照实。
    gates.parity = Some(record(false, "h1"));
    assert_eq!(gates.lights_for("h1", false).parity, Light::Fail);
    gates.parity = Some(record(true, "h1"));
    assert_eq!(gates.lights_for("h1", false).parity, Light::Pass);
    assert_eq!(
        serde_json::to_string(&Light::NotNeeded).unwrap(),
        "\"not_needed\""
    );
}
