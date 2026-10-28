use super::*;
use std::path::PathBuf;

/// 每个测试自己的临时算例目录，避免并行测试互相踩。
fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "colm-time-basis-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn case_with(name: &str, body: &str) -> PathBuf {
    let root = scratch(name);
    std::fs::write(root.join("case.nml"), body).unwrap();
    root
}

#[test]
fn greenwich_true_resolves_to_utc() {
    // build 给区域/空间算例统一写 true；单点算例在强迫场是 UTC 时也写 true。
    let root = case_with(
        "true",
        "&nl_colm\n DEF_simulation_time%greenwich = .TRUE.\n/\n",
    );
    let (basis, source) = resolve(&root).unwrap();
    assert_eq!(basis, TimeBasis::Utc);
    assert_eq!(source, "case_namelist_greenwich_true");
    assert_eq!(basis.utc_offset_seconds(), Some(0));
    let payload = json(basis, source);
    assert_eq!(payload.kind, "utc");
    assert_eq!(payload.utc_offset_seconds, Some(0));
    assert_eq!(payload.calendar, "standard");
    assert_eq!(payload.source, "case_namelist_greenwich_true");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn greenwich_false_resolves_to_the_model_clock() {
    // 地方时推进但偏移没落盘：只能叫模型钟，不能伪装成 UTC 或猜 +8。
    let root = case_with(
        "false",
        "&nl_colm\n DEF_simulation_time%greenwich = .FALSE.\n/\n",
    );
    let (basis, source) = resolve(&root).unwrap();
    assert_eq!(basis, TimeBasis::ModelClock);
    assert_eq!(source, "case_namelist_greenwich_false");
    assert_eq!(basis.utc_offset_seconds(), None);
    let payload = json(basis, source);
    assert_eq!(payload.kind, "model_clock");
    assert_eq!(payload.utc_offset_seconds, None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_or_unreadable_configuration_falls_back_conservatively() {
    let missing_field = case_with("missing-field", "&nl_colm\n DEF_CASE_NAME = 'demo'\n/\n");
    assert_eq!(
        resolve(&missing_field).unwrap(),
        (TimeBasis::ModelClock, "case_namelist_missing_greenwich")
    );
    let _ = std::fs::remove_dir_all(&missing_field);

    // 解析失败也不该让结果页打不开；只把来源标成 missing。
    let broken = case_with("broken", "this is not a namelist\n");
    assert_eq!(
        resolve(&broken).unwrap(),
        (TimeBasis::ModelClock, "case_namelist_missing_greenwich")
    );
    let _ = std::fs::remove_dir_all(&broken);

    let absent = scratch("absent");
    assert_eq!(
        resolve(&absent).unwrap(),
        (TimeBasis::ModelClock, "case_namelist_missing_greenwich")
    );
    let _ = std::fs::remove_dir_all(&absent);
}

#[test]
fn an_explicit_local_offset_serialises_its_seconds() {
    // resolve 目前不会产出 LocalStandardTime（偏移还没有输入入口），
    // 但 JSON 契约必须先固定下来，第三批接偏移输入时不用改前端。
    let payload = json(
        TimeBasis::LocalStandardTime {
            utc_offset_seconds: 8 * 3_600,
        },
        "user",
    );
    assert_eq!(payload.kind, "local_standard_time");
    assert_eq!(payload.utc_offset_seconds, Some(28_800));
    let value = serde_json::to_value(payload).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "kind": "local_standard_time",
            "utc_offset_seconds": 28_800,
            "calendar": "standard",
            "source": "user",
        })
    );
    // 模型钟必须序列化成 null，而不是被前端当成 0。
    let value = serde_json::to_value(json(
        TimeBasis::ModelClock,
        "case_namelist_missing_greenwich",
    ))
    .unwrap();
    assert_eq!(value["kind"], "model_clock");
    assert!(value["utc_offset_seconds"].is_null());
}
