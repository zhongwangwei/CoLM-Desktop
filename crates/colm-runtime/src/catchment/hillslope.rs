//! `MOD_Catch_HillslopeFlow:hillslope_flow`：坡面上的浅水方程（HLL 通量、静水重构），各流域独立。
//!
//! 舍入形状取自 GIMPLE（`MOD_Catch_HillslopeFlow.F90` 行号）。`0.5*grav` 折成 `4.90308`，
//! `1/grav` 折成 `0.10197671667604852`，`grav*nmanning**2` 折成 `0.8825544`。

use colm_core::LibmPow;
use colm_init::catch_network::{CatchState, Hillslope, RiverLakeNetwork};
use colm_numeric::Contract;
use rayon::prelude::*;

const GRAV: f64 = 9.80616;
const PONDMIN: f64 = 1.0e-4;
/// `1/grav`
const INV_GRAV: f64 = 0.101_976_716_676_048_52;
/// `0.5*grav`
const HALF_GRAV: f64 = 4.90308;
/// `grav * nmanning_hslp**2`（0.3²）
const FRICTION_COEF: f64 = 0.882_554_4;

/// 一个流域的坡面推进（`:84-289`）。`wdsrf/veloc/momen/prev` 是本流域的 HRU 段；`ta` 是 HRU 段的
/// `wdsrf_bsnhru_ta`/`momen_bsnhru_ta`。
#[allow(clippy::too_many_arguments)]
fn basin_flow(
    hillslope: &Hillslope,
    dt: f64,
    wdsrf_bsnhru: &mut [f64],
    veloc_bsnhru: &mut [f64],
    momen_bsnhru: &mut [f64],
    wdsrf_prev: &[f64],
    wdsrf_ta: &mut [f64],
    momen_ta: &mut [f64],
) {
    // `:84-87`：`momen = veloc * MIN_EXPR(prev, wdsrf)`
    for i in 0..wdsrf_bsnhru.len() {
        momen_bsnhru[i] = veloc_bsnhru[i] * wdsrf_prev[i].min(wdsrf_bsnhru[i]);
    }
    let nhru = hillslope.nhru;
    let local = |h: usize| hillslope.ihru[h] - hillslope.ihru[0];
    let mut wdsrf: Vec<f64> = (0..nhru).map(|i| wdsrf_bsnhru[local(i)]).collect();
    let mut momen: Vec<f64> = (0..nhru).map(|i| momen_bsnhru[local(i)]).collect();
    let mut veloc: Vec<f64> = (0..nhru)
        .map(|i| {
            if wdsrf[i] > 0.0 {
                momen[i] / wdsrf[i]
            } else {
                0.0
            }
        })
        .collect();
    let mut sum_h = vec![0.0; nhru];
    let mut sum_m = vec![0.0; nhru];
    let mut sum_z = vec![0.0; nhru];
    let mut xsurf = vec![0.0; nhru];
    let is_target: Vec<bool> = (0..nhru)
        .map(|i| hillslope.inext.contains(&Some(i)))
        .collect();
    let mut dt_res = dt;
    while dt_res > 0.0 {
        sum_h.fill(0.0);
        sum_m.fill(0.0);
        sum_z.fill(0.0);
        let mut dt_this = dt_res;
        for i in 0..nhru {
            let Some(j) = hillslope.inext[i] else {
                continue;
            };
            if wdsrf[i] < PONDMIN && wdsrf[j] < PONDMIN {
                continue;
            }
            let (hand_i, hand_j) = (hillslope.hand[i], hillslope.hand[j]);
            // `:137-139`
            let hand_fc = hand_i.max(hand_j);
            let wup = ((wdsrf[i] + hand_i) - hand_fc).max(0.0);
            let wdn = ((wdsrf[j] + hand_j) - hand_fc).max(0.0);
            let (vi, vj) = (veloc[i], veloc[j]);
            let sqrt_up = (wup * GRAV).sqrt();
            let sqrt_dn = (wdn * GRAV).sqrt();
            // `:143` `FMA(vi + vj, 0.5, sqrt_up) - sqrt_dn`
            let veloc_fc = (vi + vj).contract(0.5, sqrt_up) - sqrt_dn;
            // `:147` `t = FMA(sqrt_up + sqrt_dn, 0.5, (vi - vj) * 0.25)`，`(t*t) * (1/grav)`
            let t = (sqrt_up + sqrt_dn).contract(0.5, (vi - vj) * 0.25);
            let wdsrf_fc = (t * t) * INV_GRAV;
            let vwave_up = if wup > 0.0 {
                // `MIN_EXPR (vi - sqrt_up, veloc_fc - sqrt(wfc*g))`
                (vi - sqrt_up).min(veloc_fc - (wdsrf_fc * GRAV).sqrt())
            } else {
                // `.FNMA (sqrt_dn, 2, vj)`
                (-sqrt_dn).contract(2.0, vj)
            };
            let vwave_dn = if wdn > 0.0 {
                // `MAX_EXPR (vj + sqrt_dn, sqrt(wfc*g) + veloc_fc)`
                (vj + sqrt_dn).max((wdsrf_fc * GRAV).sqrt() + veloc_fc)
            } else {
                // `.FMA (sqrt_up, 2, vi)`
                sqrt_up.contract(2.0, vi)
            };
            let hflux_up = vi * wup;
            let hflux_dn = vj * wdn;
            // `:163-164` `FMA(w, v*v, (w*w)*4.90308)`
            let mflux_up = wup.contract(vi * vi, (wup * wup) * HALF_GRAV);
            let mflux_dn = wdn.contract(vj * vj, (wdn * wdn) * HALF_GRAV);
            let flen = hillslope.flen[i];
            let (hflux_fc, mflux_fc) = if vwave_up >= 0.0 {
                (hflux_up * flen, mflux_up * flen)
            } else if vwave_dn <= 0.0 {
                (hflux_dn * flen, mflux_dn * flen)
            } else {
                // `:174` `(FMA(vdn*vup, wdn - wup, FMS(vdn, hup, vup*hdn)) * flen) / (vdn - vup)`
                let vv = vwave_dn * vwave_up;
                let denom = vwave_dn - vwave_up;
                let h = vv.contract(
                    wdn - wup,
                    vwave_dn.contract(hflux_up, -(vwave_up * hflux_dn)),
                );
                let m = vv.contract(
                    hflux_dn - hflux_up,
                    vwave_dn.contract(mflux_up, -(vwave_up * mflux_dn)),
                );
                ((h * flen) / denom, (m * flen) / denom)
            };
            sum_h[i] += hflux_fc;
            sum_h[j] -= hflux_fc;
            sum_m[i] += mflux_fc;
            sum_m[j] -= mflux_fc;
            // `:185-186` `a = (flen*0.5)*grav`；`FMA(a, wup*wup, z_i)`、`FNMA(a, wdn*wdn, z_j)`
            let a = (flen * 0.5) * GRAV;
            sum_z[i] = a.contract(wup * wup, sum_z[i]);
            sum_z[j] = (-a).contract(wdn * wdn, sum_z[j]);
        }
        for i in 0..nhru {
            // `:192-196`
            if hillslope.inext[i].is_some() && (veloc[i] != 0.0 || wdsrf[i] > 0.0) {
                let limit = (hillslope.plen[i] / ((wdsrf[i] * GRAV).sqrt() + veloc[i].abs())) * 0.8;
                dt_this = limit.min(dt_this);
            }
            // `:199-202`
            xsurf[i] = sum_h[i] / hillslope.area[i];
            if xsurf[i] > 0.0 {
                dt_this = dt_this.min(wdsrf[i] / xsurf[i]);
            }
            // `:205-209`
            let net = sum_m[i] - sum_z[i];
            if veloc[i].abs() > 0.1 && net * veloc[i] > 0.0 {
                dt_this = dt_this.min(((hillslope.area[i] * momen[i]) / net).abs());
            }
        }
        for i in 0..nhru {
            // `:214` `MAX_EXPR (FNMA (xsurf, dt, w), 0)`
            wdsrf[i] = (-xsurf[i]).contract(dt_this, wdsrf[i]).max(0.0);
            if wdsrf[i] < PONDMIN {
                momen[i] = 0.0;
            } else {
                // `:219` `(|momen| * 0.8825544) / pow(w, 7/3)`
                let friction = (momen[i].abs() * FRICTION_COEF) / wdsrf[i].lpow(7.0 / 3.0);
                // `:222` `FNMA ((Σm - Σz)/area, dt, momen) / FMA (dt, friction, 1)`
                momen[i] = (-((sum_m[i] - sum_z[i]) / hillslope.area[i]))
                    .contract(dt_this, momen[i])
                    / dt_this.contract(friction, 1.0);
                if hillslope.inext[i].is_none() {
                    momen[i] = momen[i].min(0.0);
                }
                if !is_target[i] {
                    momen[i] = momen[i].max(0.0);
                }
            }
        }
        if hillslope.indx[0] == 0 {
            // `:235-268`：河道 HRU 水位高过周边时把多出的水摊到低处。
            let mut srfbsn = hillslope
                .hand
                .iter()
                .zip(&wdsrf)
                .map(|(h, w)| h + w)
                .fold(f64::MAX, f64::min);
            if srfbsn < wdsrf[0] {
                let mut dvol = (wdsrf[0] - srfbsn) * hillslope.area[0];
                momen[0] *= srfbsn / wdsrf[0];
                wdsrf[0] = srfbsn;
                while dvol > 0.0 {
                    let mask: Vec<bool> = (0..nhru)
                        .map(|k| hillslope.hand[k] + wdsrf[k] > srfbsn)
                        .collect();
                    let nexta = (0..nhru)
                        .filter(|&k| !mask[k])
                        .fold(0.0, |acc, k| acc + hillslope.area[k]);
                    let ddep;
                    if mask.iter().any(|&m| m) {
                        let nextl = (0..nhru)
                            .filter(|&k| mask[k])
                            .map(|k| hillslope.hand[k] + wdsrf[k])
                            .fold(f64::MAX, f64::min);
                        let nextv = nexta * (nextl - srfbsn);
                        if dvol > nextv {
                            ddep = nextl - srfbsn;
                            dvol -= nextv;
                        } else {
                            ddep = dvol / nexta;
                            dvol = 0.0;
                        }
                    } else {
                        ddep = dvol / nexta;
                        dvol = 0.0;
                    }
                    srfbsn += ddep;
                    for k in 0..nhru {
                        if !mask[k] {
                            wdsrf[k] += ddep;
                        }
                    }
                }
            }
        }
        for i in 0..nhru {
            veloc[i] = if wdsrf[i] < PONDMIN {
                0.0
            } else {
                momen[i] / wdsrf[i]
            };
            let h = local(i);
            // `:277-278` `FMA(w, dt, ta)`、`FMA(dt, momen, ta)`
            wdsrf_ta[h] = wdsrf[i].contract(dt_this, wdsrf_ta[h]);
            momen_ta[h] = dt_this.contract(momen[i], momen_ta[h]);
        }
        dt_res -= dt_this;
    }
    for i in 0..nhru {
        wdsrf_bsnhru[local(i)] = wdsrf[i];
        veloc_bsnhru[local(i)] = veloc[i];
    }
}

/// `hillslope_flow (dt)`：湖泊流域速度、动量清零；其余流域各自推进。流域之间互不依赖，按流域并行。
pub fn hillslope_flow(
    network: &RiverLakeNetwork,
    state: &mut CatchState,
    wdsrf_bsnhru_ta: &mut [f64],
    momen_bsnhru_ta: &mut [f64],
    dt: f64,
) {
    let ranges = &network.basin_hru.basin_hru;
    let CatchState {
        wdsrf_bsnhru,
        veloc_bsnhru,
        momen_bsnhru,
        wdsrf_bsnhru_prev,
        ..
    } = state;
    let wdsrf = split_ranges(wdsrf_bsnhru, ranges);
    let veloc = split_ranges(veloc_bsnhru, ranges);
    let momen = split_ranges(momen_bsnhru, ranges);
    let wta = split_ranges(wdsrf_bsnhru_ta, ranges);
    let mta = split_ranges(momen_bsnhru_ta, ranges);
    let prev = wdsrf_bsnhru_prev.as_slice();
    wdsrf
        .into_par_iter()
        .zip(veloc)
        .zip(momen)
        .zip(wta)
        .zip(mta)
        .enumerate()
        .for_each(|(b, ((((w, v), m), wt), mt))| {
            if network.lake_id[b] > 0 {
                v.fill(0.0);
                m.fill(0.0);
                return;
            }
            if w.is_empty() {
                return;
            }
            let range = ranges[b].clone();
            basin_flow(
                &network.hillslope_basin[b],
                dt,
                w,
                v,
                m,
                &prev[range],
                wt,
                mt,
            );
        });
    // `:303`
    wdsrf_bsnhru_prev.copy_from_slice(wdsrf_bsnhru);
}

/// 把连续排列的流域 HRU 段拆成互不重叠的可变切片。
pub(crate) fn split_ranges<'a>(
    data: &'a mut [f64],
    ranges: &[std::ops::Range<usize>],
) -> Vec<&'a mut [f64]> {
    let mut out = Vec::with_capacity(ranges.len());
    let mut rest = data;
    let mut offset = 0;
    for range in ranges {
        let (_, tail) = rest.split_at_mut(range.start - offset);
        let (head, tail) = tail.split_at_mut(range.len());
        out.push(head);
        rest = tail;
        offset = range.end;
    }
    out
}
