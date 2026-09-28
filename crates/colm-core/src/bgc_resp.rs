//! 植被呼吸：`MOD_BGC_Veg_CNMResp.F90`（维持呼吸）与 `MOD_BGC_Veg_CNGResp.F90`（生长呼吸）。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, NPCROPMIN};
use crate::bgc_state::BgcState;
use crate::LibmPow;

/// `CNMResp`。温度订正是 `Q10**(((T − 273.15) − 20)/10)`（两次减法，不合并成 293.15）；
/// 细根逐层累加被收缩成 `FMA((br_root·frootn)·tcsoi(j), rootfr(j), acc)`。
pub fn cn_m_resp(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let nl = s.dims.nl_soil;
    let q10 = s.constants.Q10;
    let br = s.constants.br;
    let br_root = s.constants.br_root;
    let correction = |t: f64| q10.lpow(((t - 273.15) - 20.0) / 10.0);
    let tcsoi: Vec<f64> = p.t_soisno[..nl].iter().map(|t| correction(*t)).collect();
    let tc = correction(p.tref[0]);
    let v = &s.pft;
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        f.leaf_mr_p[m] = if p.sigf_p[m] == 1.0 {
            p.respc_p[m] * 12.011
        } else {
            0.0
        };
        if c.woody[class] == 1.0 {
            f.livestem_mr_p[m] = v.livestemn_p[m] * br * tc;
            f.livecroot_mr_p[m] = v.livecrootn_p[m] * br_root * tc;
        } else if ivt >= NPCROPMIN {
            f.livestem_mr_p[m] = v.livestemn_p[m] * br * tc;
            f.grain_mr_p[m] = v.grainn_p[m] * br * tc;
        }
        for j in 0..nl {
            f.froot_mr_p[m] = (br_root * v.frootn_p[m] * tcsoi[j])
                .mul_add(p.rootfr_p[j + nl * m], f.froot_mr_p[m]);
        }
    }
}

/// `CNGResp`：生长呼吸 = 分配量 × `grperc`；储存部分再按 `grpnow` 拆成当场呼出（`*_storage_gr`）
/// 与转移时呼出（`transfer_*_gr`）。上游的 `respfact_*` 是恒为 1 的占位（`x·1.0` 逐位不变），
/// 这里不再保留。GIMPLE 全是源码顺序的乘法，没有收缩。
pub fn cn_g_resp(s: &mut BgcState, p: &BgcPhysics, c: &BgcPftConstants) {
    let f = &mut s.pft_flux;
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let class = ivt as usize;
        let grperc = c.grperc[class];
        let grpnow = c.grpnow[class];
        let crop = ivt >= NPCROPMIN;
        let woody = c.woody[class] == 1.0;
        if crop {
            f.cpool_grain_gr_p[m] = f.cpool_to_grainc_p[m] * grperc;
            f.cpool_grain_storage_gr_p[m] = f.cpool_to_grainc_storage_p[m] * grperc * grpnow;
            f.transfer_grain_gr_p[m] = f.grainc_xfer_to_grainc_p[m] * grperc * (1.0 - grpnow);
        }
        f.cpool_leaf_gr_p[m] = f.cpool_to_leafc_p[m] * grperc;
        f.cpool_leaf_storage_gr_p[m] = f.cpool_to_leafc_storage_p[m] * grperc * grpnow;
        f.transfer_leaf_gr_p[m] = f.leafc_xfer_to_leafc_p[m] * grperc * (1.0 - grpnow);
        f.cpool_froot_gr_p[m] = f.cpool_to_frootc_p[m] * grperc;
        f.cpool_froot_storage_gr_p[m] = f.cpool_to_frootc_storage_p[m] * grperc * grpnow;
        f.transfer_froot_gr_p[m] = f.frootc_xfer_to_frootc_p[m] * grperc * (1.0 - grpnow);
        if crop || woody {
            f.cpool_livestem_gr_p[m] = f.cpool_to_livestemc_p[m] * grperc;
            f.cpool_livestem_storage_gr_p[m] = f.cpool_to_livestemc_storage_p[m] * grperc * grpnow;
            f.transfer_livestem_gr_p[m] =
                f.livestemc_xfer_to_livestemc_p[m] * grperc * (1.0 - grpnow);
        }
        if woody {
            f.cpool_deadstem_gr_p[m] = f.cpool_to_deadstemc_p[m] * grperc;
            f.cpool_deadstem_storage_gr_p[m] = f.cpool_to_deadstemc_storage_p[m] * grperc * grpnow;
            f.transfer_deadstem_gr_p[m] =
                f.deadstemc_xfer_to_deadstemc_p[m] * grperc * (1.0 - grpnow);
            f.cpool_livecroot_gr_p[m] = f.cpool_to_livecrootc_p[m] * grperc;
            f.cpool_livecroot_storage_gr_p[m] =
                f.cpool_to_livecrootc_storage_p[m] * grperc * grpnow;
            f.transfer_livecroot_gr_p[m] =
                f.livecrootc_xfer_to_livecrootc_p[m] * grperc * (1.0 - grpnow);
            f.cpool_deadcroot_gr_p[m] = f.cpool_to_deadcrootc_p[m] * grperc;
            f.cpool_deadcroot_storage_gr_p[m] =
                f.cpool_to_deadcrootc_storage_p[m] * grperc * grpnow;
            f.transfer_deadcroot_gr_p[m] =
                f.deadcrootc_xfer_to_deadcrootc_p[m] * grperc * (1.0 - grpnow);
        }
    }
}
