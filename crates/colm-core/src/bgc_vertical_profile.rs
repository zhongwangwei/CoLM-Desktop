//! `MOD_BGC_Soil_BiogeochemVerticalProfile.F90`：凋落物输入、固氮与氮沉降在土层间的分配廓线。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]

use anyhow::{bail, Result};

use crate::bgc_driver::BgcPhysics;
use crate::bgc_state::BgcState;

/// 地表组分廓线的陡度（1/e 折减深度的倒数，1/m）。
const SURFPROF_EXP: f64 = 10.0;
/// `MOD_Vars_Global`：基岩起始深度（m）。
const ZMIN_BEDROCK: f64 = 0.4;
/// 廓线积分偏离 1 的容差；超出时上游 `abort`。
const DELTA: f64 = 1.0e-10;

/// `sum(x(1:n)*dz(1:n))`：gfortran 把 `sum` 展开成从 0 起的 FMA 链。
fn integral(values: &[f64], dz: &[f64]) -> f64 {
    values
        .iter()
        .zip(dz)
        .fold(0.0, |acc, (x, d)| x.mul_add(*d, acc))
}

/// `SoilBiogeochemVerticalProfile`。所有逐层累加都被收缩成 FMA。
pub fn soil_biogeochem_vertical_profile(s: &mut BgcState, p: &BgcPhysics) -> Result<()> {
    let nl = s.dims.nl_soil;
    let dz = &p.dz_soi[..nl];
    let active = usize::try_from(s.patch.altmax_lastyear_indx[0].max(1))
        .expect("positive")
        .min(nl);
    let altmax_positive = s.patch.altmax_lastyear_indx[0] > 0;

    let mut surface_prof = vec![0.0; nl];
    for j in 0..nl {
        surface_prof[j] = (-(SURFPROF_EXP * p.z_soi[j])).exp() / dz[j];
        if p.z_soi[j] > ZMIN_BEDROCK {
            surface_prof[j] = 0.0;
        }
    }
    let mut col_cinput_rootfr = vec![0.0; nl];
    s.patch.nfixation_prof[..nl].fill(0.0);
    s.patch.ndep_prof[..nl].fill(0.0);

    let v = &mut s.pft;
    let npft = p.pftclass.len();
    for (m, &ivt) in p.pftclass.iter().enumerate() {
        let column = m * nl..(m + 1) * nl;
        v.leaf_prof_p[column.clone()].fill(0.0);
        v.froot_prof_p[column.clone()].fill(0.0);
        v.croot_prof_p[column.clone()].fill(0.0);
        v.stem_prof_p[column.clone()].fill(0.0);
        v.cinput_rootfr_p[column].fill(0.0);
        if ivt != 0 {
            for j in 0..nl {
                v.cinput_rootfr_p[j + nl * m] = p.rootfr_p[j + nl * m] / dz[j];
            }
        }
    }

    for m in 0..npft {
        let at = |j: usize| j + nl * m;
        let mut rootfr_tot = 0.0;
        let mut surface_prof_tot = 0.0;
        for j in 0..active {
            rootfr_tot = v.cinput_rootfr_p[at(j)].mul_add(dz[j], rootfr_tot);
            surface_prof_tot = surface_prof[j].mul_add(dz[j], surface_prof_tot);
        }
        if altmax_positive && rootfr_tot > 0.0 && surface_prof_tot > 0.0 {
            for j in 0..active {
                v.froot_prof_p[at(j)] = v.cinput_rootfr_p[at(j)] / rootfr_tot;
                v.croot_prof_p[at(j)] = v.cinput_rootfr_p[at(j)] / rootfr_tot;
                v.leaf_prof_p[at(j)] = surface_prof[j] / surface_prof_tot;
                v.stem_prof_p[at(j)] = surface_prof[j] / surface_prof_tot;
            }
        } else {
            let top = 1.0 / dz[0];
            v.froot_prof_p[at(0)] = top;
            v.croot_prof_p[at(0)] = top;
            v.leaf_prof_p[at(0)] = top;
            v.stem_prof_p[at(0)] = top;
        }
        for j in 0..nl {
            if s.patch.w_scalar[j] == 0.0 && p.t_soisno[j] < 273.15 {
                v.froot_prof_p[at(j)] = 0.0;
                v.croot_prof_p[at(j)] = 0.0;
                v.stem_prof_p[at(j)] = 0.0;
                v.leaf_prof_p[at(j)] = 0.0;
            }
        }
        for profile in [
            &mut v.froot_prof_p,
            &mut v.croot_prof_p,
            &mut v.stem_prof_p,
            &mut v.leaf_prof_p,
        ] {
            let column = &mut profile[at(0)..at(nl)];
            let sumprof = integral(column, dz);
            if sumprof != 0.0 {
                for value in column.iter_mut() {
                    *value /= sumprof;
                }
            } else {
                column[0] = 1.0 / dz[0];
            }
        }
    }

    for m in 0..npft {
        for j in 0..nl {
            col_cinput_rootfr[j] =
                v.cinput_rootfr_p[j + nl * m].mul_add(p.pftfrac[m], col_cinput_rootfr[j]);
        }
    }
    let mut rootfr_tot = 0.0;
    let mut surface_prof_tot = 0.0;
    for j in 0..active {
        rootfr_tot = col_cinput_rootfr[j].mul_add(dz[j], rootfr_tot);
        surface_prof_tot = surface_prof[j].mul_add(dz[j], surface_prof_tot);
    }
    let patch = &mut s.patch;
    if altmax_positive && rootfr_tot > 0.0 && surface_prof_tot > 0.0 {
        for j in 0..active {
            patch.nfixation_prof[j] = col_cinput_rootfr[j] / rootfr_tot;
            patch.ndep_prof[j] = surface_prof[j] / surface_prof_tot;
        }
    } else {
        patch.nfixation_prof[0] = 1.0 / dz[0];
        patch.ndep_prof[0] = 1.0 / dz[0];
    }

    let ndep_sum = integral(&patch.ndep_prof[..nl], dz);
    let nfix_sum = integral(&patch.nfixation_prof[..nl], dz);
    if (ndep_sum - 1.0).abs() > DELTA || (nfix_sum - 1.0).abs() > DELTA {
        bail!("SoilBiogeochemVerticalProfile: ndep/nfixation profile sums {ndep_sum}/{nfix_sum} differ from 1");
    }
    for m in 0..npft {
        let column = m * nl..(m + 1) * nl;
        let sums = [
            integral(&v.froot_prof_p[column.clone()], dz),
            integral(&v.croot_prof_p[column.clone()], dz),
            integral(&v.leaf_prof_p[column.clone()], dz),
            integral(&v.stem_prof_p[column], dz),
        ];
        if sums.iter().any(|sum| (sum - 1.0).abs() > DELTA) {
            bail!("SoilBiogeochemVerticalProfile: PFT {m} profile sums {sums:?} differ from 1");
        }
    }
    Ok(())
}
