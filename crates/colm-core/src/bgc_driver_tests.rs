use super::*;

/// 阶段序列与 Fortran 插桩追踪（bm 算例：NITRIF/FIRE/SASU 关、无 CROP）的记录名一致，
/// 长名按 32 字节截断。
#[test]
fn default_sequence_matches_the_fortran_trace_tags() {
    let stages = stage_sequence(BgcSwitches::default());
    assert_eq!(stages.first(), Some(&"BeginCNBalance"));
    assert_eq!(stages.len(), 27);
    assert!(stages.iter().all(|stage| stage.len() <= 32));
    let nitrif = stage_sequence(BgcSwitches {
        nitrif: true,
        ..BgcSwitches::default()
    });
    assert_eq!(nitrif[7], "SoilBiogeochemNitrifDenitrif");
}
