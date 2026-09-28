//! `MOD_BGC_Soil_BiogeochemLittVertTransp.F90`：土壤有机质（含凋落物，不含粗木质残体）的
//! 垂直扩散/对流输运，Patankar 差分 + 三对角隐式求解。
//!
//! 局部数组按 Fortran 下标（`0..=nl_soil+1`）开，下标与上游逐行对应。
//!
//! **上游越界读，按参考内核的内存布局复现。** driver 传进来的全局 `z_soi`/`zi_soi` 只有
//! `1:nl_soil`，本过程却把 `zi_soi` 声明成 `0:nl_soil_full`（按序列关联，形参 `zi_soi(k)` 是全局
//! `zi_soi(k+1)`），并在 `nl_soil+1` 层上读 `z_soi(nl_soil+1)`：
//!
//! - `nm` 看 `MOD_Vars_Global` 的布局是 `zi_soi`、`z_soi`、`N_URB`（4 字节 + 4 字节零填充）、
//!   `dz_soi` 依次相接，所以形参 `zi_soi(nl_soil)`、`zi_soi(nl_soil+1)` 读到的是 `z_soi(1)`、
//!   `z_soi(2)`；
//! - `z_soi(nl_soil+1)` 读到 `N_URB`（3 或 10）的位模式，是个次正规数。它只出现在
//!   `z_soi(nl_soil+1) − x`（`x` 为正规数）里，被完全吸收，结果与取 `+0.0` 逐位相同。
//!
//! 这是 macOS/gfortran 下参考内核的行为；换一个链接布局的平台，Fortran 自己的结果也会变。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use anyhow::{anyhow, Result};

use crate::bgc_driver::{BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;
use crate::linear::solve_tridiagonal;

/// `MOD_Vars_Global`：基岩起始层。
const NBEDROCK: usize = 10;

/// Patankar 的 A 函数 `max(0, (1 − 0.1|Pe|)^5)`：底数收缩成 `FNMA(|Pe|, 0.1, 1)`，
/// 五次方展开成 `t² · (t²·t)`。
fn aaa(pe: f64) -> f64 {
    let t = (-pe.abs()).mul_add(0.1, 1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    (t2 * t3).max(0.0)
}

/// `SoilBiogeochemLittVertTransp`。
pub fn soil_biogeochem_litt_vert_transp(
    s: &mut BgcState,
    p: &BgcPhysics,
    sw: BgcSwitches,
) -> Result<()> {
    let nl = s.dims.nl_soil;
    let full = s.dims.nl_soil_full;
    let npools = s.dims.ndecomp_pools;
    let deltim = p.deltim;
    let c = &s.constants;
    let (som_adv_flux, som_diffus) = (c.som_adv_flux, c.som_diffus);
    let cryoturb_k = c.cryoturb_diffusion_k;
    let max_altdepth = c.max_altdepth_cryoturbation;
    let max_depth_cryoturb = c.max_depth_cryoturb;
    const EPSILON: f64 = 1.0e-30;
    let spinup_term = 1.0;

    // 见模块文档：越界读按参考内核的布局取值。
    let z = |k: usize| if k <= nl { p.z_soi[k - 1] } else { 0.0 };
    let zi = |k: usize| if k < nl { p.zi_soi[k] } else { p.z_soi[k - nl] };
    let dz = |k: usize| p.dz_soi[k - 1];

    let altmax = s.patch.altmax[0].max(s.patch.altmax_lastyear[0]);
    let v = &mut s.patch;
    if altmax <= max_altdepth && altmax > 0.0 {
        for j in 1..=nl + 1 {
            if j <= NBEDROCK + 1 {
                if zi(j) < altmax {
                    v.som_diffus_coef[j - 1] = cryoturb_k;
                } else {
                    v.som_diffus_coef[j - 1] = (cryoturb_k
                        * (1.0
                            - (zi(j) - altmax)
                                / (max_depth_cryoturb.min(zi(NBEDROCK + 1)) - altmax)))
                        .max(0.0);
                }
                v.som_adv_coef[j - 1] = 0.0;
            } else {
                v.som_adv_coef[j - 1] = 0.0;
                v.som_diffus_coef[j - 1] = 0.0;
            }
        }
    } else if altmax > 0.0 {
        for j in 1..=nl + 1 {
            if j <= NBEDROCK + 1 {
                v.som_adv_coef[j - 1] = som_adv_flux;
                v.som_diffus_coef[j - 1] = som_diffus;
            } else {
                v.som_adv_coef[j - 1] = 0.0;
                v.som_diffus_coef[j - 1] = 0.0;
            }
        }
    } else {
        for j in 1..=nl + 1 {
            v.som_adv_coef[j - 1] = 0.0;
            v.som_diffus_coef[j - 1] = 0.0;
        }
    }

    let n = nl + 2;
    let mut dz_node = vec![0.0; n];
    dz_node[1] = z(1);
    for j in 2..=nl + 1 {
        dz_node[j] = z(j) - z(j - 1);
    }

    let mut adv_flux = vec![0.0; n];
    let mut diffus = vec![0.0; n];
    let mut d_p1_zp1 = vec![0.0; n];
    let mut d_m1_zm1 = vec![0.0; n];
    let mut f_p1 = vec![0.0; n];
    let mut f_m1 = vec![0.0; n];
    let mut pe_p1 = vec![0.0; n];
    let mut pe_m1 = vec![0.0; n];
    let mut a_tri = vec![0.0; n];
    let mut b_tri = vec![0.0; n];
    let mut c_tri = vec![0.0; n];
    let mut r_tri_c = vec![0.0; n];
    let mut r_tri_n = vec![0.0; n];
    let mut conc_c = vec![0.0; n];
    let mut conc_n = vec![0.0; n];
    let f = &mut s.patch_flux;
    let is_cwd = &s.invariants.is_cwd;

    for l in 0..npools {
        let at = |j: usize| j - 1 + full * l;
        let at_acc = |j: usize| j - 1 + nl * l;
        if !is_cwd[l] {
            for j in 1..=nl + 1 {
                adv_flux[j] = if v.som_adv_coef[j - 1].abs() * spinup_term < EPSILON {
                    EPSILON
                } else {
                    v.som_adv_coef[j - 1] * spinup_term
                };
                diffus[j] = if v.som_diffus_coef[j - 1].abs() * spinup_term < EPSILON {
                    EPSILON
                } else {
                    v.som_diffus_coef[j - 1] * spinup_term
                };
            }
            conc_c[0] = 0.0;
            conc_n[0] = 0.0;
            for j in NBEDROCK + 1..=nl + 1 {
                conc_c[j] = 0.0;
                conc_n[j] = 0.0;
            }
            for j in 1..=nl + 1 {
                conc_c[j] = v.decomp_cpools_vr[at(j)];
                conc_n[j] = v.decomp_npools_vr[at(j)];
                if j == 1 {
                    d_m1_zm1[j] = 0.0;
                    let w_p1 = (z(j + 1) - zi(j)) / dz_node[j + 1];
                    let d_p1 = if diffus[j + 1] > 0.0 && diffus[j] > 0.0 {
                        1.0 / ((1.0 - w_p1) / diffus[j] + w_p1 / diffus[j + 1])
                    } else {
                        0.0
                    };
                    d_p1_zp1[j] = d_p1 / dz_node[j + 1];
                    f_m1[j] = adv_flux[j];
                    f_p1[j] = adv_flux[j + 1];
                    pe_m1[j] = 0.0;
                    pe_p1[j] = f_p1[j] / d_p1_zp1[j];
                } else if j > NBEDROCK {
                    let w_m1 = (zi(j - 1) - z(j - 1)) / dz_node[j];
                    let d_m1 = if diffus[j] > 0.0 && diffus[j - 1] > 0.0 {
                        1.0 / ((1.0 - w_m1) / diffus[j] + w_m1 / diffus[j - 1])
                    } else {
                        0.0
                    };
                    d_m1_zm1[j] = d_m1 / dz_node[j];
                    d_p1_zp1[j] = d_m1_zm1[j];
                    f_m1[j] = adv_flux[j];
                    f_p1[j] = 0.0;
                    pe_m1[j] = f_m1[j] / d_m1_zm1[j];
                    pe_p1[j] = f_p1[j] / d_p1_zp1[j];
                } else {
                    let w_m1 = (zi(j - 1) - z(j - 1)) / dz_node[j];
                    let d_m1 = if diffus[j - 1] > 0.0 && diffus[j] > 0.0 {
                        1.0 / ((1.0 - w_m1) / diffus[j] + w_m1 / diffus[j - 1])
                    } else {
                        0.0
                    };
                    let w_p1 = (z(j + 1) - zi(j)) / dz_node[j + 1];
                    let d_p1 = if diffus[j + 1] > 0.0 && diffus[j] > 0.0 {
                        1.0 / ((1.0 - w_p1) / diffus[j] + w_p1 / diffus[j + 1])
                    } else {
                        // 算术平均：左边的乘积被收缩
                        (1.0 - w_p1).mul_add(diffus[j], w_p1 * diffus[j + 1])
                    };
                    d_m1_zm1[j] = d_m1 / dz_node[j];
                    d_p1_zp1[j] = d_p1 / dz_node[j + 1];
                    f_m1[j] = adv_flux[j];
                    f_p1[j] = adv_flux[j + 1];
                    pe_m1[j] = f_m1[j] / d_m1_zm1[j];
                    pe_p1[j] = f_p1[j] / d_p1_zp1[j];
                }
            }

            let mut a_p_0 = 0.0;
            for j in 0..=nl + 1 {
                if j > 0 && j < nl + 1 {
                    a_p_0 = dz(j) / deltim;
                }
                if j == 0 {
                    a_tri[j] = 0.0;
                    b_tri[j] = 1.0;
                    c_tri[j] = -1.0;
                    r_tri_c[j] = 0.0;
                    r_tri_n[j] = 0.0;
                } else if j < nl + 1 {
                    a_tri[j] = -d_m1_zm1[j].mul_add(aaa(pe_m1[j]), f_m1[j].max(0.0));
                    c_tri[j] = -d_p1_zp1[j].mul_add(aaa(pe_p1[j]), (-f_p1[j]).max(0.0));
                    b_tri[j] = -a_tri[j] - c_tri[j] + a_p_0;
                    let source_c = f.decomp_cpools_sourcesink[at(j)] * dz(j) / deltim;
                    let source_n = f.decomp_npools_sourcesink[at(j)] * dz(j) / deltim;
                    if j == 1 {
                        r_tri_c[j] = (a_p_0 - adv_flux[j]).mul_add(conc_c[j], source_c);
                        r_tri_n[j] = (a_p_0 - adv_flux[j]).mul_add(conc_n[j], source_n);
                        if sw.sasu || sw.diag_matrix {
                            let k = c_tri[j] / dz(j) * deltim;
                            v.upperVX_c_vr_acc[at_acc(j)] =
                                (-k).mul_add(conc_c[j + 1], v.upperVX_c_vr_acc[at_acc(j)]);
                            v.diagVX_c_vr_acc[at_acc(j)] =
                                (-k).mul_add(conc_c[j], v.diagVX_c_vr_acc[at_acc(j)]);
                            v.upperVX_n_vr_acc[at_acc(j)] =
                                (-k).mul_add(conc_n[j + 1], v.upperVX_n_vr_acc[at_acc(j)]);
                            v.diagVX_n_vr_acc[at_acc(j)] =
                                (-k).mul_add(conc_n[j], v.diagVX_n_vr_acc[at_acc(j)]);
                        }
                    } else {
                        r_tri_c[j] = a_p_0.mul_add(conc_c[j], source_c);
                        r_tri_n[j] = a_p_0.mul_add(conc_n[j], source_n);
                        if sw.sasu || sw.diag_matrix {
                            if j <= NBEDROCK {
                                let ka = a_tri[j] / dz(j) * deltim;
                                v.lowerVX_c_vr_acc[at_acc(j)] =
                                    (-ka).mul_add(conc_c[j - 1], v.lowerVX_c_vr_acc[at_acc(j)]);
                                v.lowerVX_n_vr_acc[at_acc(j)] =
                                    (-ka).mul_add(conc_n[j - 1], v.lowerVX_n_vr_acc[at_acc(j)]);
                                if j != nl {
                                    let kc = c_tri[j] / dz(j) * deltim;
                                    v.upperVX_c_vr_acc[at_acc(j)] =
                                        (-kc).mul_add(conc_c[j + 1], v.upperVX_c_vr_acc[at_acc(j)]);
                                    v.upperVX_n_vr_acc[at_acc(j)] =
                                        (-kc).mul_add(conc_n[j + 1], v.upperVX_n_vr_acc[at_acc(j)]);
                                    let kb = (b_tri[j] - a_p_0) / dz(j) * deltim;
                                    v.diagVX_c_vr_acc[at_acc(j)] =
                                        kb.mul_add(conc_c[j], v.diagVX_c_vr_acc[at_acc(j)]);
                                    v.diagVX_n_vr_acc[at_acc(j)] =
                                        kb.mul_add(conc_n[j], v.diagVX_n_vr_acc[at_acc(j)]);
                                } else {
                                    v.diagVX_c_vr_acc[at_acc(j)] =
                                        (-ka).mul_add(conc_c[j], v.diagVX_c_vr_acc[at_acc(j)]);
                                    v.diagVX_n_vr_acc[at_acc(j)] =
                                        (-ka).mul_add(conc_n[j], v.diagVX_n_vr_acc[at_acc(j)]);
                                }
                            } else if j == NBEDROCK + 1 && j != nl && j > 1 {
                                let ka = a_tri[j] / dz(j - 1) * deltim;
                                v.diagVX_c_vr_acc[at_acc(j - 1)] =
                                    ka.mul_add(conc_c[j - 1], v.diagVX_c_vr_acc[at_acc(j - 1)]);
                                v.diagVX_n_vr_acc[at_acc(j - 1)] =
                                    ka.mul_add(conc_n[j - 1], v.diagVX_n_vr_acc[at_acc(j - 1)]);
                            }
                        }
                    }
                } else {
                    a_tri[j] = -1.0;
                    b_tri[j] = 1.0;
                    c_tri[j] = 0.0;
                    r_tri_c[j] = 0.0;
                    r_tri_n[j] = 0.0;
                }
            }

            for j in 1..=nl {
                f.decomp_cpools_transport_tendency[at(j)] =
                    0.0 - (conc_c[j] + f.decomp_cpools_sourcesink[at(j)]);
                f.decomp_npools_transport_tendency[at(j)] =
                    0.0 - (conc_n[j] + f.decomp_npools_sourcesink[at(j)]);
            }
            conc_c = solve_tridiagonal(&a_tri, &b_tri, &c_tri, &r_tri_c).map_err(|e| anyhow!(e))?;
            conc_n = solve_tridiagonal(&a_tri, &b_tri, &c_tri, &r_tri_n).map_err(|e| anyhow!(e))?;
            for j in 1..=nl {
                f.decomp_cpools_transport_tendency[at(j)] =
                    (f.decomp_cpools_transport_tendency[at(j)] + conc_c[j]) / deltim;
                f.decomp_npools_transport_tendency[at(j)] =
                    (f.decomp_npools_transport_tendency[at(j)] + conc_n[j]) / deltim;
            }
        } else {
            // 粗木质残体不输运，只加上本步源汇。
            for j in 1..=nl {
                conc_c[j] = v.decomp_cpools_vr[at(j)] + f.decomp_cpools_sourcesink[at(j)];
                conc_n[j] = v.decomp_npools_vr[at(j)] + f.decomp_npools_sourcesink[at(j)];
            }
        }
        for j in 1..=nl {
            v.decomp_cpools_vr[at(j)] = conc_c[j];
            v.decomp_npools_vr[at(j)] = conc_n[j];
            if j > NBEDROCK {
                let ratio = dz(j) / dz(NBEDROCK);
                v.decomp_cpools_vr[at(NBEDROCK)] =
                    conc_c[j].mul_add(ratio, v.decomp_cpools_vr[at(NBEDROCK)]);
                v.decomp_cpools_vr[at(j)] = 0.0;
                v.decomp_npools_vr[at(NBEDROCK)] =
                    conc_n[j].mul_add(ratio, v.decomp_npools_vr[at(NBEDROCK)]);
                v.decomp_npools_vr[at(j)] = 0.0;
            }
        }
    }
    Ok(())
}
