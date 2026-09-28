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
