use super::*;

/// 与旧版程序写出的研究规格同形：没有 `parent_study`。
fn legacy_spec() -> serde_json::Value {
    serde_json::json!({
        "kind": "tuning",
        "method": "differential-evolution",
        "seed": 1,
        "kernel_dir": null,
        "base_cases": ["site"],
        "observations": {"site": "/obs/site.nc"},
        "site_mode": "shared",
        "parameters": [{"name": "DEF_TUNING_CNFAC", "sample_min": 0.4, "sample_max": 0.6}],
        "outputs": [],
        "analysis_from": null,
        "analysis_to": null,
        "targets": [{
            "key": "Qle@site",
            "site": "site",
            "variable": "Qle",
            "from": 0,
            "to": 86_400,
            "aggregate": "daily"
        }],
        "budget": {"population": 4, "generations": 1, "jobs": 1}
    })
}

/// 旧研究的 `manifest.provenance.spec_sha256` 是对不含 `parent_study` 的 JSON 算的。
/// 新字段必须真的不出现（不能是 `null`），否则所有旧研究运行前的 spec 哈希核对都会拒绝。
#[test]
fn a_legacy_spec_serialises_without_the_parent_study_key() {
    let spec: StudySpec = serde_json::from_value(legacy_spec()).unwrap();
    assert!(spec.parent_study.is_none());
    let text = serde_json::to_string(&spec).unwrap();
    assert!(!text.contains("parent_study"), "{text}");
    // 再序列化一次逐字节相同：哈希核对拿的是这份字节。
    let again = serde_json::to_string(&serde_json::from_str::<StudySpec>(&text).unwrap()).unwrap();
    assert_eq!(text, again);
}

/// 派生研究的溯源：父研究目录、父研究冻结规格的哈希、父研究实际用的模型修订。
#[test]
fn parent_study_round_trips_with_the_model_revision() {
    let mut value = legacy_spec();
    value["parent_study"] = serde_json::json!({
        "study": "/p/.colm/studies/s-uncertainty",
        "spec_sha256": "a".repeat(64),
        "model_revision": {"id": "trace", "revision": 7},
    });
    let spec: StudySpec = serde_json::from_value(value.clone()).unwrap();
    let parent = spec.parent_study.as_ref().expect("parent study recorded");
    assert_eq!(parent.study, "/p/.colm/studies/s-uncertainty");
    assert_eq!(parent.spec_sha256, "a".repeat(64));
    let revision = parent.model_revision.as_ref().expect("revision recorded");
    assert_eq!(revision.id, "trace");
    assert_eq!(revision.revision, 7);
    // 逐字段断言而不是和手写 JSON 整体比：反序列化会补上缺省值（weight、min_pairs、budget…），
    // 整体相等测的是缺省值清单，不是这个字段。
    let text = serde_json::to_string(&spec).unwrap();
    assert!(text.contains("parent_study"), "{text}");
    assert!(text.contains("\"trace\""), "{text}");
    let again: StudySpec = serde_json::from_str(&text).unwrap();
    assert_eq!(
        serde_json::to_value(&spec).unwrap(),
        serde_json::to_value(&again).unwrap()
    );
}

/// 老研究没有记录模型修订：派生时仍能记下父研究与设计哈希，只是没有修订。
#[test]
fn parent_study_without_a_model_revision_round_trips() {
    let mut value = legacy_spec();
    value["parent_study"] = serde_json::json!({
        "study": "/p/.colm/studies/legacy",
        "spec_sha256": "b".repeat(64),
    });
    let spec: StudySpec = serde_json::from_value(value.clone()).unwrap();
    assert!(spec.parent_study.as_ref().unwrap().model_revision.is_none());
    let text = serde_json::to_string(&spec).unwrap();
    let again: StudySpec = serde_json::from_str(&text).unwrap();
    assert!(again
        .parent_study
        .as_ref()
        .unwrap()
        .model_revision
        .is_none());
}

/// 派生来源是审计证据：多写的字段要拒绝，而不是悄悄忽略。
#[test]
fn parent_study_rejects_unknown_fields() {
    let mut value = legacy_spec();
    value["parent_study"] = serde_json::json!({
        "study": "/p/.colm/studies/s",
        "spec_sha256": "c".repeat(64),
        "note": "not part of the contract",
    });
    let error = serde_json::from_value::<StudySpec>(value)
        .unwrap_err()
        .to_string();
    assert!(error.contains("note"), "{error}");
}
