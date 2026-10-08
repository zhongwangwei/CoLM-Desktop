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
