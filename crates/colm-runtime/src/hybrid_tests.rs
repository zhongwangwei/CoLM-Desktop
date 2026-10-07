use super::*;

#[test]
fn features_parse_scalars_and_one_based_layers() {
    assert_eq!(parse_feature("patchclass").unwrap(), ("patchclass", None));
    assert_eq!(parse_feature("BD_all[1]").unwrap(), ("BD_all", Some(0)));
    assert!(parse_feature("BD_all[0]").is_err());
    assert!(parse_feature("BD_all[x]").is_err());
}

#[test]
fn only_land_class_param_slots_with_def_lc_outputs_are_accepted() {
    let config = |name: &str, kind: SlotKind, output: &str| SlotConfig {
        name: name.into(),
        kind,
        model: None,
        sha256: None,
        features: vec!["patchclass".into()],
        normalize: None,
        outputs: vec![colm_hybrid::OutputSpec {
            name: output.into(),
            range: None,
            transform: colm_hybrid::Transform::Identity,
        }],
    };
    assert!(known_slot(&config(LAND_CLASS_SLOT, SlotKind::Param, "DEF_LC_VMAX25")).is_ok());
    assert!(known_slot(&config(LAND_CLASS_SLOT, SlotKind::Process, "DEF_LC_VMAX25")).is_err());
    assert!(known_slot(&config(LAND_CLASS_SLOT, SlotKind::Param, "DEF_LC_C3C4")).is_err());
    assert!(known_slot(&config("stomata", SlotKind::Param, "DEF_LC_VMAX25")).is_err());
}

#[test]
fn restart_markers_follow_the_hybrid_fingerprint() {
    let dir = std::env::temp_dir().join(format!("colm-hybrid-marker-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let restart = dir.join("x_restart_2008-001-00000_lc2008.nc");
    std::fs::write(&restart, b"nc").unwrap();
    // 没有标记：什么配置都能接着跑。
    check_restart(&restart, None).unwrap();
    check_restart(&restart, Some("aaa")).unwrap();
    // 纯物理写出不留文件。
    mark_restart(&restart, None).unwrap();
    assert!(!restart_marker(&restart).exists());
    // 混合写出留标记；接着跑必须同一指纹。
    mark_restart(&restart, Some("aaa")).unwrap();
    check_restart(&restart, Some("aaa")).unwrap();
    assert!(check_restart(&restart, Some("bbb")).is_err());
    assert!(check_restart(&restart, None).is_err());
    // 被纯物理运行覆盖时删掉旧标记。
    mark_restart(&restart, None).unwrap();
    assert!(!restart_marker(&restart).exists());
    check_restart(&restart, Some("bbb")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
