//! 土壤分解：`MOD_BGC_Soil_BiogeochemDecompCascadeBGC.F90`（速率常数）、
//! `MOD_BGC_Soil_BiogeochemPotential.F90`（潜在分解）与 `MOD_BGC_Soil_BiogeochemDecomp.F90`。
//!
//! 数组下标：`(nl_soil_full, ndecomp_pools)` 的池是 `j + nl_soil_full·l`，`(nl_soil, ·)` 的是
//! `j + nl_soil·l`，池号 `l` 由上游的 1 起下标减一得到。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use crate::bgc_driver::BgcPhysics;
use crate::bgc_state::BgcState;
use crate::LibmPow;

/// `catanf(15)/catanf(30)`：两个都是常数，gfortran 在编译期折成这个双精度数。
const CATANF_RATIO: f64 = 0.547_063_348_006_363_359_132_819_823_571_480_810_642_242_431_640_625;

fn pool(index: i32) -> usize {
    usize::try_from(index - 1).expect("BGC pool indices start at 1")
}

/// `decomp_rate_constants_bgc`。
///
/// - `86400·365` 折成 `3.1536e7`；
/// - 暖区 `T − (273.15 + 25)` 折成 `T − 298.15`；
/// - 归一化 `(catanf(15)/catanf(30)) / Q10**(-1)` 里 `Q10**(-1)` 被改写成 `1/Q10`；
/// - `decomp_k = (((k·t)·w)·d)·o`。
pub fn decomp_rate_constants_bgc(s: &mut BgcState, p: &BgcPhysics) {
    let nl = s.dims.nl_soil;
    let full = s.dims.nl_soil_full;
    let c = &s.constants;
    let rate = |tau: f64| 1.0 / (tau * 3.1536e7);
    let k_l1 = rate(c.tau_l1);
    let k_l2_l3 = rate(c.tau_l2_l3);
    let k_s1 = rate(c.tau_s1);
    let k_s2 = rate(c.tau_s2);
    let k_s3 = rate(c.tau_s3);
    let k_frag = rate(c.tau_cwd);
    let q10 = c.Q10;
    let froz_q10 = c.froz_q10;
    let v = &mut s.patch;
    for j in 0..nl {
        let t = p.t_soisno[j];
        v.t_scalar[j] = if t >= 273.15 {
            q10.lpow((t - 298.15) / 10.0)
        } else {
            q10.lpow(-2.5) * froz_q10.lpow((t - 273.15) / 10.0)
        };
    }
    for j in 0..nl {
        let psi = p.smp[j].min(p.smpmax_hr);
        v.w_scalar[j] = if psi > p.smpmin_hr {
            (p.smpmin_hr / psi).ln() / (p.smpmin_hr / p.smpmax_hr).ln()
        } else {
            0.001
        };
    }
    v.o_scalar[..nl].fill(1.0);
    let normalization = CATANF_RATIO / (1.0 / q10);
    for j in 0..nl {
        v.t_scalar[j] *= normalization;
    }
    for j in 0..nl {
        v.depth_scalar[j] = (-(p.z_soi[j] / 10.0)).exp();
    }
    let rates = [
        (c.i_met_lit, k_l1),
        (c.i_cel_lit, k_l2_l3),
        (c.i_lig_lit, k_l2_l3),
        (c.i_soil1, k_s1),
        (c.i_soil2, k_s2),
        (c.i_soil3, k_s3),
        (c.i_cwd, k_frag),
    ];
    for (index, k) in rates {
        let l = pool(index);
        for j in 0..nl {
            v.decomp_k[j + full * l] =
                k * v.t_scalar[j] * v.w_scalar[j] * v.depth_scalar[j] * v.o_scalar[j];
        }
    }
}

/// `SoilBiogeochemPotential`：潜在分解通量、潜在矿化/固持与潜在异养呼吸。
///
/// `p_decomp_cpool_loss = (C·k)·pathfrac`；`pmnf = (loss·((1 − rf) − ratio))/cn_receiver`；
/// `phr_vr` 的累加收缩成 `FMA(rf, loss, phr)`。
pub fn soil_biogeochem_potential(s: &mut BgcState) {
    let nl = s.dims.nl_soil;
    let full = s.dims.nl_soil_full;
    let npools = s.dims.ndecomp_pools;
    let ntrans = s.dims.ndecomp_transitions;
    let i_atm = s.constants.i_atm;
    let inv = &s.invariants;
    let v = &mut s.patch;
    let f = &mut s.patch_flux;
    f.p_decomp_cpool_loss[..nl * ntrans].fill(0.0);
    f.pmnf_decomp[..nl * ntrans].fill(0.0);
    for l in 0..npools {
        if inv.floating_cn_ratio[l] {
            for j in 0..nl {
                if v.decomp_npools_vr[j + full * l] > 0.0 {
                    v.cn_decomp_pools[j + nl * l] =
                        v.decomp_cpools_vr[j + full * l] / v.decomp_npools_vr[j + full * l];
                }
            }
        } else {
            for j in 0..nl {
                v.cn_decomp_pools[j + nl * l] = inv.initial_cn_ratio[l];
            }
        }
    }
    for k in 0..ntrans {
        let donor = pool(inv.donor_pool[k]);
        let receiver_index = inv.receiver_pool[k];
        for j in 0..nl {
            let cpool = v.decomp_cpools_vr[j + full * donor];
            let rate = v.decomp_k[j + full * donor];
            if !(cpool > 0.0 && rate > 0.0) {
                continue;
            }
            let t = j + nl * k;
            let loss = cpool * rate * inv.pathfrac_decomp[t];
            f.p_decomp_cpool_loss[t] = loss;
            // 上游用 `floating_cn_ratio(receiver_pool(k))` 判断；接收方是大气（i_atm）时那是越界读，
            // 现行级联里没有这种转化，这里把它归到"100% 呼吸"分支。
            let receiver_floats =
                receiver_index != i_atm && inv.floating_cn_ratio[pool(receiver_index)];
            f.pmnf_decomp[t] = if receiver_floats {
                0.0
            } else if receiver_index != i_atm {
                let receiver = pool(receiver_index);
                let ratio = if v.decomp_npools_vr[j + full * donor] > 0.0 {
                    v.cn_decomp_pools[j + nl * receiver] / v.cn_decomp_pools[j + nl * donor]
                } else {
                    0.0
                };
                (loss * ((1.0 - inv.rf_decomp[t]) - ratio)) / v.cn_decomp_pools[j + nl * receiver]
            } else {
                -(loss / v.cn_decomp_pools[j + nl * donor])
            };
        }
    }
    let mut immob = vec![0.0; nl];
    for k in 0..ntrans {
        for j in 0..nl {
            let pmnf = f.pmnf_decomp[j + nl * k];
            if pmnf > 0.0 {
                immob[j] += pmnf;
            } else {
                f.gross_nmin_vr[j] -= pmnf;
            }
        }
    }
    f.potential_immob_vr[..nl].copy_from_slice(&immob);
    f.phr_vr[..nl].fill(0.0);
    for k in 0..ntrans {
        for j in 0..nl {
            let t = j + nl * k;
            f.phr_vr[j] = inv.rf_decomp[t].mul_add(f.p_decomp_cpool_loss[t], f.phr_vr[j]);
        }
    }
}
