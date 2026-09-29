//! `CNFireArea` 里转写器处理不了的调用（其余由 `oracle/scripts/bgc_port/regen.py` 生成进 `bgc_fire.rs`）。
//!
//! `CALL eroot(nl_soil, 0., porsl, bsw, <VG 参数>, psi0, rootfr_p(:, pftclass(m)), dz_soi, t_soisno,
//! wliq_soisno, tmp1d, tmp0d, btran2_p(m))`：`eroot` 在 `MOD_Eroot` 里单独编译、这里是一次真调用，
//! 与叶温里那次是同一段目标代码，所以直接复用已逐位验证的 [`crate::root_uptake`]。

use anyhow::{bail, Result};

use crate::bgc_state::BgcState;

use crate::bgc_driver::{BgcPhysics, BgcSwitches};
use crate::{root_uptake, RootUptakeInput, SoilHydraulicModel};

/// 第 `m` 个 PFT（0 起）的 `rstfac`（`CNFireArea` 的 `btran2_p(m)`）。`trsmx0 = 0`，根分布取该 PFT
/// 类别的 `rootfr_p`。
pub fn eroot_rstfac(p: &BgcPhysics, sw: BgcSwitches, m: usize) -> Result<f64> {
    let nl = p.dz_soi.len();
    let hydraulic_model: Vec<SoilHydraulicModel> = (0..nl)
        .map(|j| {
            if sw.campbell {
                SoilHydraulicModel::Campbell { bsw: p.bsw[j] }
            } else {
                SoilHydraulicModel::VanGenuchten {
                    alpha_vgm: p.alpha_vgm[j],
                    n_vgm: p.n_vgm[j],
                    l_vgm: p.L_vgm[j],
                    sc_vgm: p.sc_vgm[j],
                    fc_vgm: p.fc_vgm[j],
                }
            }
        })
        .collect();
    let state = root_uptake(RootUptakeInput {
        maximum_transpiration_mm_s: 0.0,
        porosity: &p.porsl[..nl],
        residual_water: &p.theta_r[..nl],
        saturated_soil_suction_mm: &p.psi0[..nl],
        hydraulic_model: &hydraulic_model,
        root_fraction: &p.rootfr_p[nl * m..nl * (m + 1)],
        layer_thickness_m: &p.dz_soi[..nl],
        temperature_k: &p.t_soisno[..nl],
        liquid_water_kg_m2: &p.wliq_soisno[..nl],
        stress_scheme: sw.rstfac,
    })?;
    Ok(state.soil_water_stress)
}

/// `CNFireArea` 写的 patch 量（`MOD_BGC_Veg_CNFireLi2016.F90`）。
const FIRE_AREA_OUTPUTS: [&str; 17] = [
    "cropf",
    "lfwt",
    "fuelc",
    "fuelc_crop",
    "fsr",
    "fd",
    "rootc",
    "lgdp",
    "lgdp1",
    "lpop",
    "wtlf",
    "trotr1",
    "trotr2",
    "baf_crop",
    "baf_peatf",
    "farea_burned",
    "nfire",
];

/// 内核带 `-ffpe-trap=invalid,zero,overflow` 编译：上游在这里除零或无效运算会当场终止（SIGILL），
/// Rust 则静默得到 inf/NaN 往下跑。`ivt` 恒为 0 让纯作物 patch（`cropf = 1`）也走自然植被那一支，
/// `1/(1-cropf)` 与 `fd_pft(0)*3600/(1-cropf)` 必然除零——上游 CROP 内核打开 FIRE 第一步就崩
/// （upstream-bugs 第 24 条）。这里按输出是否有限判定，同样失败；只落在中间量、没有传到输出的
/// 溢出查不到。
pub fn ensure_no_fp_trap(s: &BgcState) -> Result<()> {
    for name in FIRE_AREA_OUTPUTS {
        if let Some(values) = s.f64_field(name) {
            if values.iter().any(|value| !value.is_finite()) {
                bail!(
                    "CNFireArea: {name} is not finite; the upstream kernel (-ffpe-trap=invalid,zero,overflow) \
                     stops here with a floating-point trap"
                );
            }
        }
    }
    Ok(())
}
