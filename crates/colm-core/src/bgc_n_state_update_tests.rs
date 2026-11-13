use super::*;
use crate::bgc_state_generated::BgcDims;

/// 上游 `MOD_BGC_CNNStateUpdate3.F90:114`：`max(smin_nh4_vr - smin_nh4_leached_vr*deltim, 0)`。
/// gfortran `-O2`（默认 `-ffp-contract=fast`）在 arm64 上收缩成 `fmsub`；x86-64 无 FMA 时不收缩。
/// 参考位由 gfortran 16.1 对同一组输入生成（`-fdefault-real-8`，fused/unfused 两份）。
#[test]
fn nitrif_nh4_leaching_loss_uses_the_upstream_fused_or_unfused_rounding() {
    let dims = BgcDims::default();
    let mut s = BgcState::new(1, dims);
    let mut p = BgcPhysics::default();
    let c = BgcPftConstants::default();
    let sw = BgcSwitches {
        nitrif: true,
        nh4_leaching: true,
        ..BgcSwitches::default()
    };
    for j in 0..dims.nl_soil {
        s.patch.smin_nh4_vr[j] = 0.0;
        s.patch.smin_no3_vr[j] = 0.0;
        s.patch.sminn_vr[j] = 0.0;
        s.patch_flux.smin_nh4_leached_vr[j] = 0.0;
        s.patch_flux.smin_no3_leached_vr[j] = 0.0;
        s.patch_flux.smin_no3_runoff_vr[j] = 0.0;
    }
    // 十进制 5.96223445202170767e-3 与 7.82683886724980191e-7 的 f64 位模式（clippy 不允许多余精度）。
    s.patch.smin_nh4_vr[0] = f64::from_bits(0x3f786bdb1fb8647f);
    s.patch_flux.smin_nh4_leached_vr[0] = f64::from_bits(0x3eaa433411934ff9);
    p.deltim = 1800.0;

    n_state_update3(&mut s, &p, &c, sw);

    let expected = if colm_numeric::FUSED {
        f64::from_bits(0x3F72A696EEDBC62D)
    } else {
        f64::from_bits(0x3F72A696EEDBC62C)
    };
    assert_eq!(
        s.patch.smin_nh4_vr[0].to_bits(),
        expected.to_bits(),
        "{}",
        s.patch.smin_nh4_vr[0]
    );
}
