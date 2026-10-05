//! `MOD_Catch_RiverLakeFlow:river_lake_flow`：河道/湖泊的浅水方程，按河系推进子步。
//!
//! 界面通量与它们的上游求和在上游是 `real(r16)`：通量本身按 r8 算完再无损转成 r16，求和、
//! 以及用到这些和的子步长、蓄量与动量更新都在 r16 里算，赋给 r8 时才舍入（双重舍入），这里用
//! [`colm_core::binary128::Quad`] 逐位复现。其余舍入形状取自 GIMPLE（行号见注释）。
//! `grav*nmanning_riv**2` 折成 `0.0088255440`。

// 循环逐行对照上游的下标写法（同一下标同时索引多个流域数组）。
#![allow(clippy::needless_range_loop)]

use colm_core::binary128::Quad;
use colm_core::LibmPow;
use colm_init::catch_network::{basin_surface, CatchState, RiverLakeNetwork};
use colm_init::catch_reservoir::CatchReservoirs;

const GRAV: f64 = 9.80616;
const INV_GRAV: f64 = 0.101_976_716_676_048_52;
const HALF_GRAV: f64 = 4.90308;
const FRICTION_COEF: f64 = 0.008_825_544;
const TWO_GRAV: f64 = 19.61232;
const RIVERMIN: f64 = 1.0e-5;
const VOLUMEMIN: f64 = 1.0e-5;

/// `river_lake_flow` 的时间平均累加量（流域顺序）。
#[derive(Debug, Clone, Default)]
pub struct RiverAccum {
    pub wdsrf_bsn_ta: Vec<f64>,
    pub momen_riv_ta: Vec<f64>,
    pub discharge_ta: Vec<f64>,
    pub wdsrf_bsnhru_ta: Vec<f64>,
    pub momen_bsnhru_ta: Vec<f64>,
    /// 每个流域推进过的子步数（`ntacc_bsn`）。
    pub ntacc_bsn: Vec<f64>,
}

/// 水库的运行量（`volresv`、`qresv_in/out`，跨步保留）与本步的时间累加（`*_ta`），水库序。
#[derive(Debug, Clone, Default)]
pub struct ReservoirFlow {
    pub volresv: Vec<f64>,
    pub qresv_in: Vec<f64>,
    pub qresv_out: Vec<f64>,
    pub volresv_ta: Vec<f64>,
    pub qresv_in_ta: Vec<f64>,
    pub qresv_out_ta: Vec<f64>,
}

impl ReservoirFlow {
    pub fn new(numresv: usize) -> Self {
        let zeros = vec![0.0; numresv];
        Self {
            volresv: zeros.clone(),
            qresv_in: zeros.clone(),
            qresv_out: zeros.clone(),
            volresv_ta: zeros.clone(),
            qresv_in_ta: zeros.clone(),
            qresv_out_ta: zeros,
        }
    }
}

/// `DEF_Reservoir_Method > 0` 时的水库参数与运行量。
pub struct Reservoirs<'a> {
    pub table: &'a CatchReservoirs,
    pub flow: &'a mut ReservoirFlow,
}

impl Reservoirs<'_> {
    /// 流域 `i` 是已建成（`year >= dam_build_year`）的水库或受控湖时返回水库序号。
    fn built(&self, network: &RiverLakeNetwork, i: usize, year: i32) -> Option<usize> {
        self.table.bsn2resv[i]
            .filter(|&r| network.lake_type[i] >= 2 && year >= self.table.dam_build_year[r])
    }
}

fn q(x: f64) -> Quad {
    Quad::from_f64(x)
}

/// `river_lake_flow (year, dt)`。返回子步数 `ntimestep_riverlake`。
pub fn river_lake_flow(
    network: &RiverLakeNetwork,
    state: &mut CatchState,
    acc: &mut RiverAccum,
    mut reservoirs: Option<Reservoirs<'_>>,
    year: i32,
    dt: f64,
) -> usize {
    let n = network.lake_id.len();
    let ranges = &network.basin_hru.basin_hru;
    // `:83-114`
    for i in 0..n {
        let range = ranges[i].clone();
        state.wdsrf_bsn[i] = basin_surface(network, i, &state.wdsrf_bsnhru[range]);
        if network.lake_id[i] == 0 {
            if state.wdsrf_bsn_prev[i] < state.wdsrf_bsn[i] {
                state.momen_riv[i] = state.wdsrf_bsn_prev[i] * state.veloc_riv[i];
                state.veloc_riv[i] = state.momen_riv[i] / state.wdsrf_bsn[i];
            } else {
                state.momen_riv[i] = state.wdsrf_bsn[i] * state.veloc_riv[i];
            }
        } else {
            state.momen_riv[i] = 0.0;
            state.veloc_riv[i] = 0.0;
        }
    }
    let mut wdsrf_ds = vec![f64::NAN; n];
    let mut veloc_ds = vec![f64::NAN; n];
    let mut hflux_fc = vec![Quad::ZERO; n];
    let mut mflux_fc = vec![Quad::ZERO; n];
    let mut zgrad_dn = vec![Quad::ZERO; n];
    let mut sum_h = vec![Quad::ZERO; n];
    let mut sum_m = vec![Quad::ZERO; n];
    let mut sum_z = vec![Quad::ZERO; n];
    let mut filter = vec![false; n];
    let mut dt_res = vec![dt; network.numrivsys];
    let mut dt_all = vec![0.0; network.numrivsys];
    // `height_up/height_dn` 在上游是跨流域保留的局部变量（`:298-303` 可能读到上一个流域的值）。
    let (mut height_up, mut height_dn) = (0.0_f64, 0.0_f64);
    let mut ntimestep = 0;
    while dt_res.iter().any(|&r| r > 0.0) {
        ntimestep += 1;
        for i in 0..n {
            filter[i] = dt_res[network.irivsys[i]] > 0.0;
            if filter[i] {
                sum_h[i] = Quad::ZERO;
                sum_m[i] = Quad::ZERO;
                sum_z[i] = Quad::ZERO;
                acc.ntacc_bsn[i] += 1.0;
            }
        }
        // `pull_from_downstream`：只在本河系还有剩余时间、且有下游时更新。
        for i in 0..n {
            if filter[i] {
                if let Some(d) = network.ilocdown[i] {
                    wdsrf_ds[i] = state.wdsrf_bsn[d];
                    veloc_ds[i] = state.veloc_riv[d];
                }
            }
            if network.riverdown[i] <= 0 {
                veloc_ds[i] = 0.0;
            }
        }
        dt_all.copy_from_slice(&dt_res);
        for i in 0..n {
            if !filter[i] {
                continue;
            }
            let down = network.riverdown[i];
            let w = state.wdsrf_bsn[i];
            let v = state.veloc_riv[i];
            let width = network.outletwth[i];
            if down >= 0 {
                if down > 0 && w < RIVERMIN && wdsrf_ds[i] < RIVERMIN {
                    hflux_fc[i] = Quad::ZERO;
                    mflux_fc[i] = Quad::ZERO;
                    zgrad_dn[i] = Quad::ZERO;
                    continue;
                }
                let bedelv_fc;
                if down > 0 {
                    // `:189` `MAX_EXPR (bedelv, bedelv_ds)`；已建成的水库/受控湖抬到坝顶（`:190-196`）。
                    let mut fc = network.bedelv[i].max(network.bedelv_ds[i]);
                    if let Some(r) = reservoirs
                        .as_ref()
                        .and_then(|r| r.built(network, i, year).map(|k| r.table.dam_elv[k]))
                    {
                        fc = fc.max(r);
                    }
                    bedelv_fc = fc;
                    // `:197-198` `MAX_EXPR ((w + bedelv) - bedelv_fc, 0)`
                    height_up = ((w + network.bedelv[i]) - bedelv_fc).max(0.0);
                    height_dn = ((wdsrf_ds[i] + network.bedelv_ds[i]) - bedelv_fc).max(0.0);
                } else {
                    bedelv_fc = network.bedelv[i];
                    height_up = w;
                    height_dn = (-bedelv_fc).max(0.0);
                }
                let vd = veloc_ds[i];
                let sqrt_up = (height_up * GRAV).sqrt();
                let sqrt_dn = (height_dn * GRAV).sqrt();
                // `:208` `FMA(v + vd, 0.5, sqrt_up) - sqrt_dn`
                let veloct_fc = (v + vd).mul_add(0.5, sqrt_up) - sqrt_dn;
                // `:212` `t = FMA(sqrt_up + sqrt_dn, 0.5, (v - vd)*0.25)`；`(t*t) * (1/grav)`
                let t = (sqrt_up + sqrt_dn).mul_add(0.5, (v - vd) * 0.25);
                let height_fc = (t * t) * INV_GRAV;
                let vwave_up = if height_up > 0.0 {
                    (v - sqrt_up).min(veloct_fc - (height_fc * GRAV).sqrt())
                } else {
                    (-sqrt_dn).mul_add(2.0, vd)
                };
                let vwave_dn = if height_dn > 0.0 {
                    (vd + sqrt_dn).max((height_fc * GRAV).sqrt() + veloct_fc)
                } else {
                    sqrt_up.mul_add(2.0, v)
                };
                let hh_up = height_up * height_up;
                let hh_dn = height_dn * height_dn;
                let hflux_up = v * height_up;
                let hflux_dn = vd * height_dn;
                // `:228-229` `FMA(h, v*v, (h*h)*4.90308)`
                let mflux_up = height_up.mul_add(v * v, hh_up * HALF_GRAV);
                let mflux_dn = height_dn.mul_add(vd * vd, hh_dn * HALF_GRAV);
                let (h, m) = hll(
                    width, vwave_up, vwave_dn, height_up, height_dn, hflux_up, hflux_dn, mflux_up,
                    mflux_dn,
                );
                hflux_fc[i] = q(h);
                mflux_fc[i] = q(m);
                // `:244-246` `((width*0.5)*grav) * h*h`，r8 算完转 r16。
                let a = (width * 0.5) * GRAV;
                sum_z[i] = sum_z[i] + q(a * hh_up);
                zgrad_dn[i] = q(a * hh_dn);
            } else if down == -3 {
                // `:254` 下游不在区域内。
                let v = v.max(0.0);
                state.veloc_riv[i] = v;
                if w > network.riverdpth[i] {
                    height_up = w;
                    height_dn = network.riverdpth[i];
                    let sqrt_up = (height_up * GRAV).sqrt();
                    let sqrt_dn = (height_dn * GRAV).sqrt();
                    // `:262` `(sqrt_up + v) - sqrt_dn`
                    let veloct_fc = (sqrt_up + v) - sqrt_dn;
                    // `:263` `((sqrt_up + sqrt_dn) * 0.5)^2 * (1/grav)`
                    let t = (sqrt_up + sqrt_dn) * 0.5;
                    let height_fc = (t * t) * INV_GRAV;
                    let vwave_up = (v - sqrt_up).min(veloct_fc - (height_fc * GRAV).sqrt());
                    let vwave_dn = (sqrt_dn + v).max((height_fc * GRAV).sqrt() + veloct_fc);
                    let hh_up = height_up * height_up;
                    let hh_dn = height_dn * height_dn;
                    let vv = v * v;
                    let hflux_up = v * height_up;
                    let hflux_dn = v * height_dn;
                    let mflux_up = height_up.mul_add(vv, hh_up * HALF_GRAV);
                    let mflux_dn = height_dn.mul_add(vv, hh_dn * HALF_GRAV);
                    let (h, m) = hll(
                        width, vwave_up, vwave_dn, height_up, height_dn, hflux_up, hflux_dn,
                        mflux_up, mflux_dn,
                    );
                    hflux_fc[i] = q(h);
                    mflux_fc[i] = q(m);
                    let a = (width * 0.5) * GRAV;
                    sum_z[i] = sum_z[i] + q(a * hh_up);
                } else {
                    hflux_fc[i] = Quad::ZERO;
                    mflux_fc[i] = Quad::ZERO;
                }
            } else if down == -1 {
                hflux_fc[i] = Quad::ZERO;
                mflux_fc[i] = Quad::ZERO;
            }
            if network.lake_id[i] < 0 && hflux_fc[i] < Quad::ZERO {
                // `:298-303`：湖泊汇流区倒灌不超过水面以下的体积。
                let dt_this = dt_all[network.irivsys[i]];
                let net = &network.hillslope_basin[i];
                let level = state.wdsrf_bsn[i] + network.handmin[i];
                let area = net
                    .hand
                    .iter()
                    .zip(&net.area)
                    .filter(|(hand, _)| **hand <= level)
                    .fold(0.0, |acc, (_, a)| acc + a);
                let limit = ((height_up - height_dn) / dt_this) * area;
                hflux_fc[i] = hflux_fc[i].max(q(limit));
            }
            // `:306-314`：已建成的水库不走界面通量，由调度放流（见下）。
            if network.lake_type[i] == 2
                && reservoirs
                    .as_ref()
                    .is_some_and(|r| r.built(network, i, year).is_some())
            {
                hflux_fc[i] = Quad::ZERO;
                mflux_fc[i] = Quad::ZERO;
                zgrad_dn[i] = Quad::ZERO;
            }
            sum_h[i] = sum_h[i] + hflux_fc[i];
            sum_m[i] = sum_m[i] + mflux_fc[i];
        }
        // `:321-331`：通量取负推给下游再取回（取负是精确的）。
        for i in 0..n {
            if filter[i] {
                if let Some(d) = network.ilocdown[i] {
                    sum_h[d] = sum_h[d] - hflux_fc[i];
                    sum_m[d] = sum_m[d] - mflux_fc[i];
                    sum_z[d] = sum_z[d] - zgrad_dn[i];
                }
            }
        }
        // `:334-372` 水库调度：入流是推完上游之后的 `-Σh`，放流按 `reservoir_operation`。
        if let Some(r) = reservoirs.as_mut() {
            let mut hflux_resv = vec![Quad::ZERO; n];
            let mut mflux_resv = vec![Quad::ZERO; n];
            for i in 0..n {
                if !filter[i] || network.lake_type[i] != 2 || network.riverdown[i] == -1 {
                    continue;
                }
                let Some(k) = r.built(network, i, year) else {
                    continue;
                };
                let w = state.wdsrf_bsn[i];
                let qin = (-sum_h[i]).to_f64();
                let vol = network.lakeinfo[i].volume(w);
                // `:349` `vol > total * 1e-4`
                let qout = if vol > r.table.volresv_total[k] * 1.0e-4 {
                    r.table.operation(k, qin, vol)
                } else {
                    0.0
                };
                r.flow.qresv_in[k] = qin;
                r.flow.volresv[k] = vol;
                r.flow.qresv_out[k] = qout;
                hflux_fc[i] = q(qout);
                // `:357` `(r16) (sqrt (w * 19.61232) * qout)`
                mflux_fc[i] = q((w * TWO_GRAV).sqrt() * qout);
                sum_h[i] = sum_h[i] + hflux_fc[i];
                sum_m[i] = sum_m[i] + mflux_fc[i];
                hflux_resv[i] = -hflux_fc[i];
                mflux_resv[i] = -mflux_fc[i];
            }
            for i in 0..n {
                if filter[i] {
                    if let Some(d) = network.ilocdown[i] {
                        sum_h[d] = sum_h[d] + hflux_resv[i];
                        sum_m[d] = sum_m[d] + mflux_resv[i];
                    }
                }
            }
        }
        // 约束（`:375-415`）。
        for i in 0..n {
            if !filter[i] {
                continue;
            }
            let sys = network.irivsys[i];
            let mut dt_this = dt_all[sys];
            let w = state.wdsrf_bsn[i];
            let v = state.veloc_riv[i];
            if network.lake_id[i] == 0 && network.riverdown[i] != -1 && (v != 0.0 || w > 0.0) {
                // `:384` `MIN_EXPR (dt, (riverlen / (|v| + sqrt(g w))) * 0.8)`
                let limit = (network.riverlen[i] / (v.abs() + (w * GRAV).sqrt())) * 0.8;
                dt_this = dt_this.min(limit);
            }
            if sum_h[i] > Quad::ZERO {
                let totalvolume = if network.lake_id[i] <= 0 {
                    volume_below(network, i, w)
                } else {
                    network.lakeinfo[i].volume(w)
                };
                // `:400` `(r8) MIN_EXPR ((r16) dt, (r16) vol / Σh)`
                dt_this = q(dt_this).min(q(totalvolume) / sum_h[i]).to_f64();
            }
            if network.lake_id[i] == 0 {
                let net = sum_m[i] - sum_z[i];
                // `:407` `(r16) v * (Σm - Σz) > 0`
                if v.abs() > 0.1 && q(v) * net > Quad::ZERO {
                    // `:409` `(r8) MIN_EXPR ((r16) dt, ABS ((r16)(momen*area) / (Σm - Σz)))`
                    let ratio = q(state.momen_riv[i] * network.riverarea[i]) / net;
                    let ratio = if ratio < Quad::ZERO { -ratio } else { ratio };
                    dt_this = q(dt_this).min(ratio).to_f64();
                }
            }
            // `:413` `MIN_EXPR (dt_this, dt_all)`
            dt_all[sys] = dt_this.min(dt_all[sys]);
        }
        // 更新（`:423-535`）。
        for i in 0..n {
            if !filter[i] {
                continue;
            }
            let sys = network.irivsys[i];
            let dt_sys = dt_all[sys];
            let range = ranges[i].clone();
            if network.lake_id[i] <= 0 {
                let net = &network.hillslope_basin[i];
                let mut totalvolume = volume_below(network, i, state.wdsrf_bsn[i]);
                // `:437` `(r8) ((r16) vol - Σh * (r16) dt)`；`dvol` 复用同一个 r16 乘积（`:449`）。
                let product = sum_h[i] * q(dt_sys);
                totalvolume = (q(totalvolume) - product).to_f64();
                if totalvolume < VOLUMEMIN {
                    // `:440-446`
                    let level = state.wdsrf_bsn[i] + network.handmin[i];
                    for (j, hand) in net.hand.iter().enumerate() {
                        if *hand <= level {
                            let h = range.start + j;
                            state.wdsrf_bsnhru[h] -= level - hand;
                        }
                    }
                    state.wdsrf_bsn[i] = 0.0;
                } else {
                    let mut dvol = product.to_f64();
                    if dvol > VOLUMEMIN {
                        while dvol > VOLUMEMIN {
                            let level = net_level(state, network, i);
                            let mask: Vec<bool> = net.hand.iter().map(|h| *h < level).collect();
                            // `maxval (hand, mask)`、`sum (area, mask)`（从 0 起）
                            let nextl = net
                                .hand
                                .iter()
                                .zip(&mask)
                                .filter(|(_, m)| **m)
                                .map(|(h, _)| *h)
                                .fold(-f64::MAX, f64::max);
                            let nexta = net
                                .area
                                .iter()
                                .zip(&mask)
                                .filter(|(_, m)| **m)
                                .fold(0.0, |acc, (a, _)| acc + a);
                            // `:455` `(level - nextl) * nexta`
                            let nextv = (level - nextl) * nexta;
                            let ddep;
                            if nextv > dvol {
                                ddep = dvol / nexta;
                                dvol = 0.0;
                            } else {
                                ddep = level - nextl;
                                dvol -= nextv;
                            }
                            state.wdsrf_bsn[i] -= ddep;
                            for (j, m) in mask.iter().enumerate() {
                                if *m {
                                    state.wdsrf_bsnhru[range.start + j] -= ddep;
                                }
                            }
                        }
                    } else if dvol < -VOLUMEMIN {
                        let nh = net.hand.len();
                        let mut mask = vec![true; nh];
                        let mut nexta = 0.0;
                        while dvol < -VOLUMEMIN {
                            let surface = |state: &CatchState, j: usize| {
                                net.hand[j] + state.wdsrf_bsnhru[range.start + j]
                            };
                            if mask.iter().any(|&m| m) {
                                // `minloc (hand + wdsrf, mask)`：第一个最小值。
                                let mut best: Option<(usize, f64)> = None;
                                for j in 0..nh {
                                    if mask[j] {
                                        let s = surface(state, j);
                                        if best.is_none_or(|(_, b)| s < b) {
                                            best = Some((j, s));
                                        }
                                    }
                                }
                                let j = best.expect("mask has a true entry").0;
                                nexta += net.area[j];
                                mask[j] = false;
                            }
                            let level = net_level(state, network, i);
                            let ddep;
                            if mask.iter().any(|&m| m) {
                                let nextl = (0..nh)
                                    .filter(|&j| mask[j])
                                    .map(|j| surface(state, j))
                                    .fold(f64::MAX, f64::min);
                                // `:483` `(nextl - level) * nexta`
                                let nextv = (nextl - level) * nexta;
                                if -dvol > nextv {
                                    ddep = nextl - level;
                                    dvol += nextv;
                                } else {
                                    ddep = (-dvol) / nexta;
                                    dvol = 0.0;
                                }
                            } else {
                                ddep = (-dvol) / nexta;
                                dvol = 0.0;
                            }
                            state.wdsrf_bsn[i] += ddep;
                            for (j, m) in mask.iter().enumerate() {
                                if !*m {
                                    state.wdsrf_bsnhru[range.start + j] += ddep;
                                }
                            }
                        }
                    }
                }
            } else {
                // `:510-512`
                let info = &network.lakeinfo[i];
                let volume = info.volume(state.wdsrf_bsn[i]);
                let totalvolume = (q(volume) - sum_h[i] * q(dt_sys)).to_f64();
                state.wdsrf_bsn[i] = info.surface(totalvolume);
            }
            let w = state.wdsrf_bsn[i];
            if network.lake_id[i] != 0 || w < RIVERMIN {
                state.momen_riv[i] = 0.0;
                state.veloc_riv[i] = 0.0;
            } else {
                // `:519` `(0.008825544 / pow(w, 7/3)) * |momen|`
                let momen = state.momen_riv[i];
                let friction = (FRICTION_COEF / w.lpow(7.0 / 3.0)) * momen.abs();
                // `:522`：全部在 r16 里。
                let numer =
                    q(momen) - ((sum_m[i] - sum_z[i]) / q(network.riverarea[i])) * q(dt_sys);
                let denom = q(dt_sys.mul_add(friction, 1.0));
                state.momen_riv[i] = (numer / denom).to_f64();
                state.veloc_riv[i] = state.momen_riv[i] / w;
            }
            if network.lake_id[i] == 0 && network.riverdown[i] == -1 {
                state.momen_riv[i] = state.momen_riv[i].min(0.0);
                state.veloc_riv[i] = state.veloc_riv[i].min(0.0);
            }
            state.veloc_riv[i] = state.veloc_riv[i].min(20.0);
            state.veloc_riv[i] = state.veloc_riv[i].max(-20.0);
        }
        for i in 0..n {
            if !filter[i] {
                continue;
            }
            let dt_sys = dt_all[network.irivsys[i]];
            // `:540-542`
            acc.wdsrf_bsn_ta[i] = state.wdsrf_bsn[i].mul_add(dt_sys, acc.wdsrf_bsn_ta[i]);
            acc.momen_riv_ta[i] = dt_sys.mul_add(state.momen_riv[i], acc.momen_riv_ta[i]);
            acc.discharge_ta[i] = (q(acc.discharge_ta[i]) + hflux_fc[i] * q(dt_sys)).to_f64();
            if network.lake_id[i] > 0 {
                let info = &network.lakeinfo[i];
                for (k, h) in ranges[i].clone().enumerate() {
                    // `:549-550`
                    let value = (state.wdsrf_bsn[i] - (info.depth[0] - info.depth0[k])).max(0.0);
                    state.wdsrf_bsnhru[h] = value;
                    acc.wdsrf_bsnhru_ta[h] = dt_sys.mul_add(value, acc.wdsrf_bsnhru_ta[h]);
                }
            }
            // `:554-562` `FMA (x, dt, x_ta)`
            if network.lake_type[i] == 2 {
                if let Some(r) = reservoirs.as_mut() {
                    if let Some(k) = r.built(network, i, year) {
                        let f = &mut *r.flow;
                        f.volresv_ta[k] = f.volresv[k].mul_add(dt_sys, f.volresv_ta[k]);
                        f.qresv_in_ta[k] = f.qresv_in[k].mul_add(dt_sys, f.qresv_in_ta[k]);
                        f.qresv_out_ta[k] = f.qresv_out[k].mul_add(dt_sys, f.qresv_out_ta[k]);
                    }
                }
            }
        }
        for (res, all) in dt_res.iter_mut().zip(&dt_all) {
            *res -= all;
        }
    }
    state.wdsrf_bsn_prev.copy_from_slice(&state.wdsrf_bsn);
    ntimestep
}

/// HLL 通量（`:231-242`、`:273-284`）：`(FMA(vdn*vup, Δh, FMS(vdn, hup, vup*hdn)) * w) / (vdn - vup)`。
#[allow(clippy::too_many_arguments)]
fn hll(
    width: f64,
    vwave_up: f64,
    vwave_dn: f64,
    height_up: f64,
    height_dn: f64,
    hflux_up: f64,
    hflux_dn: f64,
    mflux_up: f64,
    mflux_dn: f64,
) -> (f64, f64) {
    if vwave_up >= 0.0 {
        (hflux_up * width, mflux_up * width)
    } else if vwave_dn <= 0.0 {
        (hflux_dn * width, mflux_dn * width)
    } else {
        let vv = vwave_dn * vwave_up;
        let denom = vwave_dn - vwave_up;
        let h = vv.mul_add(
            height_dn - height_up,
            vwave_dn.mul_add(hflux_up, -(vwave_up * hflux_dn)),
        );
        let m = vv.mul_add(
            hflux_dn - hflux_up,
            vwave_dn.mul_add(mflux_up, -(vwave_up * mflux_dn)),
        );
        ((h * width) / denom, (m * width) / denom)
    }
}

/// `sum ((w + handmin - hand) * area, mask = w + handmin >= hand)`：`.FMA (level - hand, area, acc)`
/// （`:392-394`、`:433-435`）。
fn volume_below(network: &RiverLakeNetwork, i: usize, w: f64) -> f64 {
    let net = &network.hillslope_basin[i];
    let level = w + network.handmin[i];
    net.hand
        .iter()
        .zip(&net.area)
        .filter(|(hand, _)| level >= **hand)
        .fold(0.0, |acc, (hand, area)| (level - hand).mul_add(*area, acc))
}

fn net_level(state: &CatchState, network: &RiverLakeNetwork, i: usize) -> f64 {
    state.wdsrf_bsn[i] + network.handmin[i]
}
