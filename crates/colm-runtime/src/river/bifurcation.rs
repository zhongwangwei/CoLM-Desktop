//! 网格河湖分汊（`DEF_USE_BIFURCATION`，`MOD_Grid_RiverLakeBifurcation`）。
//!
//! 分汊路径把两个单元流域（可以跨河系）连起来，按 CaMa 的局部惯性方程逐层更新路径动量，
//! 再经过三道限流：路径 5% 端点蓄量、逐层供水方蓄量、单元流域总出流。得到的净通量
//! `hflux_sum` 加进汇流的 `sum_hflux_riv`。
//!
//! 只接单进程、无堤防、无水库的路径（调用方先拒绝其余组合）：这时所有下游单元流域都在本地，
//! 上游跨进程的 `neg_pth`/`protected_*` 各项恒为 0。

// 夹紧保留上游 `MAX(MIN(·))` 的次序；逐路径、逐单元流域的下标循环与上游 `DO` 循环对应。
#![allow(clippy::manual_clamp, clippy::needless_range_loop)]

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::LibmPow;

use super::levee::Levee;
use super::network::RiverNetwork;
use super::GRAV;

/// `BIFMIN = RIVERLAKE_DRY_DEPTH`。
const BIFMIN: f64 = 1.0e-5;
/// `available_storage_ucat` 的 `stage_restart_tol`。
const STAGE_RESTART_TOL: f64 = 1.0e-5;
/// `BIF_RESTART_SIGNATURE_VERSION`。
pub const SIGNATURE_VERSION: f64 = 1.0;

/// 分汊路径的静态参数（`read_and_distribute_bifurcation` 的单进程分支）。
#[derive(Debug, Clone)]
pub struct Bifurcation {
    /// `npthlev_bif`。
    pub levels: usize,
    /// 上游单元流域（0 起）。
    pub upst: Vec<usize>,
    /// 下游单元流域（0 起）；下游不在网络里时为 `None`。
    pub down: Vec<Option<usize>>,
    /// 文件里的原始下游号（1 起，`<= 0` 表示没有），写续跑签名用。
    pub down_ucid: Vec<i32>,
    pub dst: Vec<f64>,
    /// 逐路径逐层，`[p*levels + l]`。
    pub elv: Vec<f64>,
    pub wth: Vec<f64>,
    pub man: Vec<f64>,
    /// `bif_incoming_pths`：每个单元流域的入流路径，按路径号递增。
    pub incoming: Vec<Vec<usize>>,
}

/// 一个子步的分汊通量。
#[derive(Debug, Clone)]
pub struct BifurcationFlux {
    /// `bif_hflux_lev`，`[p*levels + l]`。
    pub hflux_lev: Vec<f64>,
    /// `bif_hflux_sum`：每个单元流域的净分汊出流（m³/s）。
    pub hflux_sum: Vec<f64>,
    /// `bif_path_active`。
    pub active: Vec<bool>,
    /// `bif_lev_hflux_sum`：第 2 层及以上（漫过河岸、堤内一侧）的净出流，只在开堤防时算。
    pub lev_hflux_sum: Vec<f64>,
}

/// 开堤防时分汊要看的堤防状态。
#[derive(Clone, Copy)]
pub struct BifurcationLevee<'a> {
    pub levee: &'a Levee,
    pub levsto: &'a [f64],
    pub levdph: &'a [f64],
}

fn read_values<T: netcdf::NcTypeDescriptor + Copy>(
    file: &netcdf::File,
    name: &str,
    path: &Path,
) -> Result<Vec<T>> {
    file.variable(name)
        .with_context(|| {
            format!(
                "{} has no {name}; DEF_USE_BIFURCATION needs it",
                path.display()
            )
        })?
        .get_values::<T, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))
}

/// `bif_limiter_fraction`。
fn limiter_fraction(available: f64, transfer: f64) -> f64 {
    if available <= 0.0 {
        0.0
    } else if transfer > available {
        available / transfer
    } else {
        1.0
    }
}

/// `push` 的 `sum` 模式：值为 0 的项不参与，第一项直接赋值。
fn accumulate(slot: &mut f64, term: f64) {
    if term == 0.0 {
        return;
    }
    *slot = if *slot == 0.0 {
        term * 1.0
    } else {
        *slot + term * 1.0
    };
}

impl Bifurcation {
    /// `read_bifurcation_global_arrays` 加单进程的路径与入流表。
    pub fn read(path: &Path, network: &RiverNetwork) -> Result<Self> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let upst_raw = read_values::<i32>(&file, "bifurcation_upst", path)?;
        let down_ucid = read_values::<i32>(&file, "bifurcation_down", path)?;
        let dst = read_values::<f64>(&file, "bifurcation_distance", path)?;
        let elv = read_values::<f64>(&file, "bifurcation_elevation", path)?;
        let wth = read_values::<f64>(&file, "bifurcation_width", path)?;
        let man = read_values::<f64>(&file, "bifurcation_manning", path)?;
        let paths = upst_raw.len();
        let levels = man.len();
        ensure!(
            down_ucid.len() == paths
                && dst.len() == paths
                && elv.len() == paths * levels
                && wth.len() == paths * levels,
            "bifurcation parameter dimensions are inconsistent"
        );
        ensure!(
            dst.iter().all(|d| d.is_finite() && *d > 0.0),
            "bifurcation distance must be finite and positive"
        );
        ensure!(
            elv.iter().all(|e| e.is_finite()),
            "bifurcation elevation contains a non-finite value"
        );
        ensure!(
            wth.iter().all(|w| w.is_finite() && *w >= 0.0),
            "bifurcation width must be finite and non-negative"
        );
        ensure!(
            man.iter().all(|m| m.is_finite()),
            "bifurcation Manning coefficient contains a non-finite value"
        );
        for l in 0..levels {
            if (0..paths).any(|p| wth[p * levels + l] > 0.0) && man[l] <= 0.0 {
                bail!("active bifurcation layer requires a positive Manning coefficient");
            }
        }
        let n = network.len() as i32;
        let mut upst = Vec::with_capacity(paths);
        let mut down = Vec::with_capacity(paths);
        let mut incoming = vec![Vec::new(); network.len()];
        for p in 0..paths {
            ensure!(
                (1..=n).contains(&upst_raw[p]),
                "bifurcation upstream index out of range"
            );
            ensure!(
                down_ucid[p] <= n,
                "bifurcation downstream index out of range"
            );
            ensure!(
                down_ucid[p] != upst_raw[p],
                "bifurcation self-loop pathway is invalid"
            );
            let row = &wth[p * levels..(p + 1) * levels];
            ensure!(
                row.iter().any(|w| *w > 0.0),
                "bifurcation pathway has no active positive-width layer"
            );
            let mut previous: Option<usize> = None;
            for l in 0..levels {
                if row[l] <= 0.0 {
                    continue;
                }
                if let Some(q) = previous {
                    ensure!(
                        elv[p * levels + l] >= elv[p * levels + q],
                        "active bifurcation layer elevation must be non-decreasing"
                    );
                }
                previous = Some(l);
            }
            upst.push(upst_raw[p] as usize - 1);
            if down_ucid[p] > 0 {
                let j = down_ucid[p] as usize - 1;
                down.push(Some(j));
                incoming[j].push(p);
            } else {
                down.push(None);
            }
        }
        Ok(Self {
            levels,
            upst,
            down,
            down_ucid,
            dst,
            elv,
            wth,
            man,
            incoming,
        })
    }

    pub fn paths(&self) -> usize {
        self.upst.len()
    }

    /// `build_bifurcation_path_signature`，逐路径 `4 + 3*levels` 个数。
    pub fn signature(&self) -> Vec<f64> {
        let levels = self.levels;
        let mut signature = Vec::with_capacity(self.paths() * (4 + 3 * levels));
        for p in 0..self.paths() {
            signature.extend([
                SIGNATURE_VERSION,
                (self.upst[p] + 1) as f64,
                f64::from(self.down_ucid[p]),
                self.dst[p],
            ]);
            signature.extend_from_slice(&self.elv[p * levels..(p + 1) * levels]);
            signature.extend_from_slice(&self.wth[p * levels..(p + 1) * levels]);
            signature.extend_from_slice(&self.man);
        }
        signature
    }

    /// `available_storage_ucat`（无堤防、无水库，`volwater_ucat_valid` 恒真）。
    fn available_storage(net: &RiverNetwork, i: usize, wdsrf: f64, volwater: f64) -> f64 {
        if volwater > 0.0 || wdsrf <= STAGE_RESTART_TOL {
            volwater + 0.0
        } else {
            // `storage_with_river_prism`：漫过河槽的部分按河槽面积补一块棱柱。
            let curve = &net.curves[i];
            let mut storage = curve.volume(wdsrf);
            if wdsrf > curve.rivhgt {
                storage = curve.rivare.mul_add(wdsrf - curve.rivhgt, storage);
            }
            storage.max(0.0)
        }
    }

    /// `bifurcation_calc` 的一个子步（单进程，所有单元流域同一子步长 `dt`）。
    ///
    /// 开堤防时（`levee` 非空）第 2 层及以上在有堤一侧用堤内水面与堤内蓄量，堤内一侧另有
    /// 供水方限流；第 1 层的界面水深不再有 0.01 m 下限。
    ///
    /// `normal_outgoing` 是普通汇流缩放后的出流（`normal_outgoing_rate`）；`veloc`/`momen`
    /// 是路径状态 `pth_veloc`/`pth_momen`，就地更新。
    #[allow(clippy::too_many_arguments)]
    pub fn calc(
        &self,
        net: &RiverNetwork,
        wdsrf: &[f64],
        wdsrf_prev: &[f64],
        volwater: &[f64],
        dt: f64,
        normal_outgoing: &[f64],
        reservoir: &[Option<f64>],
        levee: Option<BifurcationLevee<'_>>,
        veloc: &mut [f64],
        momen: &mut [f64],
    ) -> BifurcationFlux {
        let n = net.len();
        let levels = self.levels;
        let paths = self.paths();
        let mut hflux_lev = vec![0.0; paths * levels];
        let mut hflux_sum = vec![0.0; n];
        let mut active = vec![false; paths];
        let is_reservoir = |i: usize| matches!(reservoir.get(i), Some(Some(_)));
        let has_levee = |i: usize| levee.is_some_and(|l| l.levee.has[i]);
        // 有堤单元流域：可见蓄量为零却有水深时（老续跑的占位），由水位按堤外可见那份补算。
        let stage_visible = |i: usize| {
            let l = levee.expect("levee");
            l.levee
                .visible_volume_from_stage(net, i, wdsrf[i], l.levsto[i])
        };
        let wet_volume = |i: usize| volwater[i] > 0.0 || wdsrf[i] <= STAGE_RESTART_TOL;
        // `available_storage_ucat`：已建成的水库是库容；有堤单元流域再加上堤内蓄量。
        let storage: Vec<f64> = (0..n)
            .map(|i| match reservoir.get(i).copied().flatten() {
                Some(volresv) => volresv,
                None if has_levee(i) => {
                    let protected = levee.expect("levee").levsto[i].max(0.0);
                    if wet_volume(i) {
                        volwater[i] + protected
                    } else {
                        stage_visible(i) + protected
                    }
                }
                None => Self::available_storage(net, i, wdsrf[i], volwater[i]),
            })
            .collect();
        // `available_visible_storage_ucat`：不开堤防时就是可用蓄量。
        let visible: Vec<f64> = (0..n)
            .map(|i| match reservoir.get(i).copied().flatten() {
                Some(volresv) => volresv.max(0.0),
                None if levee.is_none() => storage[i],
                None if has_levee(i) && !wet_volume(i) => stage_visible(i).max(0.0),
                None if wet_volume(i) => volwater[i].max(0.0),
                None => Self::available_storage(net, i, wdsrf[i], volwater[i]).max(0.0),
            })
            .collect();
        // `available_protected_storage_ucat` 与堤内水面 `rivhgt + levdph`。
        let protected: Vec<f64> = (0..n)
            .map(|i| {
                if has_levee(i) && !is_reservoir(i) {
                    levee.expect("levee").levsto[i].max(0.0)
                } else {
                    0.0
                }
            })
            .collect();
        let protected_wdsrf: Vec<f64> = (0..n)
            .map(|i| {
                if has_levee(i) {
                    net.curves[i].rivhgt + levee.expect("levee").levdph[i]
                } else {
                    wdsrf[i]
                }
            })
            .collect();
        let use_protected = levee.is_some() && levels >= 2;
        let mut protected_outgoing = vec![0.0; n];
        let mut protected_rate = vec![1.0f64; n];
        let mut total = vec![0.0; paths];
        let mut pth_rate = vec![1.0f64; paths];
        let mut layer_rate = vec![1.0f64; paths * levels];
        let mut outgoing = vec![0.0; n];
        let mut out_rate = vec![1.0f64; n];
        let with_lev = levee.is_some() && levels >= 2;
        let mut lev_total = vec![0.0; if with_lev { paths } else { 0 }];
        let mut lev_hflux_sum = vec![0.0; if with_lev { n } else { 0 }];

        // Step 3：逐路径逐层的局部惯性更新与限流量。
        for p in 0..paths {
            let i_up = self.upst[p];
            // 水库里的水静止：两端任一是已建成的水库都不走分汊。
            if is_reservoir(i_up) {
                continue;
            }
            // 下游不在网络里：推到路径上的是填充值，跳过。
            let Some(i_dn) = self.down[p] else { continue };
            if is_reservoir(i_dn) {
                continue;
            }
            if dt <= 0.0 {
                continue;
            }
            let row = p * levels..(p + 1) * levels;
            if self.dst[p] <= 0.0 {
                momen[row.clone()].fill(0.0);
                veloc[row].fill(0.0);
                continue;
            }
            active[p] = true;
            let (rivelv_up, rivelv_dn) = (net.rivelv[i_up], net.rivelv[i_dn]);
            let (w_up, w_dn) = (wdsrf[i_up], wdsrf[i_dn]);
            let (w_up_prev, w_dn_prev) = (wdsrf_prev[i_up], wdsrf_prev[i_dn]);
            let dst = self.dst[p];
            let (up_levee, dn_levee) = (has_levee(i_up), has_levee(i_dn));
            for l in 0..levels {
                let k = p * levels + l;
                let width = self.wth[k];
                if width <= 0.0 {
                    momen[k] = 0.0;
                    veloc[k] = 0.0;
                    continue;
                }
                let elv = self.elv[k];
                // 漫过河岸的层在有堤一侧用堤内水面。
                let w_up = if l > 0 && up_levee {
                    protected_wdsrf[i_up]
                } else {
                    w_up
                };
                let w_dn = if l > 0 && dn_levee {
                    protected_wdsrf[i_dn]
                } else {
                    w_dn
                };
                let height_up = ((w_up + rivelv_up) - elv).max(0.0);
                let height_dn = ((w_dn + rivelv_dn) - elv).max(0.0);
                if height_up < BIFMIN && height_dn < BIFMIN {
                    momen[k] = 0.0;
                    veloc[k] = 0.0;
                    continue;
                }
                let zsurf_up = w_up + rivelv_up;
                let zsurf_dn = w_dn + rivelv_dn;
                let slope = ((zsurf_dn - zsurf_up) / dst).min(0.005).max(-0.005);
                let current = height_up.max(height_dn);
                let up_prev = ((w_up_prev + rivelv_up) - elv).max(0.0);
                let dn_prev = ((w_dn_prev + rivelv_dn) - elv).max(0.0);
                let previous = up_prev.max(dn_prev);
                let h_face = if levee.is_some() {
                    // CaMa 堤防方案：河槽层用几何平均（一侧干时退回当前深度），漫过河岸的层显式。
                    if l == 0 {
                        let h = (current * previous).sqrt();
                        if h <= 0.0 {
                            current
                        } else {
                            h
                        }
                    } else {
                        current
                    }
                } else {
                    // 标准 CaMa 分汊：几何平均，0.01 m 的下限保证由干转湿时有导水。
                    (current * previous).sqrt().max((current * 0.01).sqrt())
                };
                if h_face <= BIFMIN {
                    momen[k] = 0.0;
                    veloc[k] = 0.0;
                    continue;
                }
                let area = dst * width;
                let manning = self.man[l];
                let friction = (manning * manning * GRAV / h_face.lpow(7.0 / 3.0)) * momen[k].abs();
                let mflux = dst * (((width * GRAV) * h_face) * slope);
                let trial = (-(mflux / area)).mul_add(dt, momen[k]) / dt.mul_add(friction, 1.0);
                let v = (trial / h_face).min(20.0).max(-20.0);
                momen[k] = h_face * v;
                veloc[k] = v;
                let flux = (width * v) * h_face;
                total[p] += flux;
                hflux_lev[k] = flux;
            }

            // Step 4：路径 5% 端点蓄量限制。
            if total[p].abs() > 0.0 && dt > 0.0 {
                let reference = storage[i_up].min(storage[i_dn]).max(0.0);
                pth_rate[p] = limiter_fraction(0.05 * reference, total[p].abs() * dt);
            }
            // 逐层按供水方蓄量限制，并记下各供水方的总出流。
            if dt > 0.0 {
                for l in 0..levels {
                    let k = p * levels + l;
                    let transfer = hflux_lev[k].abs();
                    if transfer <= 0.0 {
                        continue;
                    }
                    let (donor, donor_levee) = if hflux_lev[k] >= 0.0 {
                        (i_up, up_levee)
                    } else {
                        (i_dn, dn_levee)
                    };
                    // 漫过河岸的层从有堤一侧的堤内蓄量出水。
                    let available = if l > 0 && donor_levee {
                        protected_outgoing[donor] += transfer;
                        protected[donor]
                    } else {
                        outgoing[donor] += transfer;
                        visible[donor]
                    };
                    layer_rate[k] =
                        layer_rate[k].min(limiter_fraction(available.max(0.0), transfer * dt));
                }
            }
        }

        // 堤内一侧的总出流限制。
        if use_protected {
            for i in 0..n {
                if protected_outgoing[i] > 0.0 && dt > 0.0 {
                    protected_rate[i] =
                        limiter_fraction(protected[i].max(0.0), protected_outgoing[i] * dt);
                }
            }
        }
        // 单元流域总出流限制：普通汇流已占用的出流先扣掉。
        for i in 0..n {
            let reference = visible[i].max(0.0);
            let normal = normal_outgoing[i].max(0.0);
            let bif_outflow = outgoing[i];
            if bif_outflow > 0.0 && dt > 0.0 {
                let remaining = (reference / dt - normal).max(0.0);
                if remaining < bif_outflow {
                    out_rate[i] = (remaining / bif_outflow).min(1.0);
                }
            }
        }

        // 施加限流，累加到两端单元流域。
        for p in 0..paths {
            if !active[p] {
                continue;
            }
            let i_up = self.upst[p];
            let Some(i_dn) = self.down[p] else { continue };
            total[p] = 0.0;
            let (up_levee, dn_levee) = (has_levee(i_up), has_levee(i_dn));
            for l in 0..levels {
                let k = p * levels + l;
                let mut rate = pth_rate[p].min(layer_rate[k]);
                let h = hflux_lev[k];
                if levee.is_some() && l > 0 && h > 0.0 && up_levee {
                    rate = rate.min(protected_rate[i_up]);
                } else if levee.is_some() && l > 0 && h < 0.0 && dn_levee {
                    rate = rate.min(protected_rate[i_dn]);
                } else if h > 0.0 {
                    rate = rate.min(out_rate[i_up]);
                } else if h < 0.0 {
                    rate = rate.min(out_rate[i_dn]);
                }
                if rate < 1.0 {
                    hflux_lev[k] *= rate;
                    momen[k] *= rate;
                    veloc[k] *= rate;
                }
                total[p] += hflux_lev[k];
            }
            // 供水方限流可能拆掉层间抵消，对净通量再做一次路径限制。
            if total[p].abs() > 0.0 {
                let reference = storage[i_up].min(storage[i_dn]).max(0.0);
                let rate = limiter_fraction(0.05 * reference, total[p].abs() * dt);
                if rate < 1.0 {
                    for k in p * levels..(p + 1) * levels {
                        hflux_lev[k] *= rate;
                        momen[k] *= rate;
                        veloc[k] *= rate;
                    }
                    total[p] = hflux_lev[p * levels..(p + 1) * levels]
                        .iter()
                        .fold(0.0, |sum, &h| h + sum);
                }
            }
            hflux_sum[i_up] += total[p];
            if levee.is_some() && levels >= 2 {
                lev_total[p] = hflux_lev[p * levels + 1..(p + 1) * levels]
                    .iter()
                    .fold(0.0, |sum, &h| h + sum);
                lev_hflux_sum[i_up] += lev_total[p];
            }
            hflux_sum[i_dn] -= total[p];
            if levee.is_some() && levels >= 2 {
                lev_hflux_sum[i_dn] -= lev_total[p];
            }
        }

        // Step 6/7：`push_bif_influx` 把路径净通量推到下游，再减掉本地已算过的部分。
        // 单进程下全部下游都在本地，两者相消只留下舍入残差，但上游照样做，这里也照做。
        let mut influx = vec![0.0; n];
        for (slot, paths_in) in influx.iter_mut().zip(&self.incoming) {
            for &p in paths_in {
                accumulate(slot, total[p]);
            }
        }
        for p in 0..paths {
            if let Some(j) = self.down[p] {
                influx[j] -= total[p];
            }
        }
        for (sum, &inflow) in hflux_sum.iter_mut().zip(&influx) {
            *sum -= inflow;
        }
        // 同样推送第 2 层及以上的净通量（`bif_lev_influx`）。
        if with_lev {
            let mut lev_influx = vec![0.0; n];
            for (slot, paths_in) in lev_influx.iter_mut().zip(&self.incoming) {
                for &p in paths_in {
                    accumulate(slot, lev_total[p]);
                }
            }
            for p in 0..paths {
                if let Some(j) = self.down[p] {
                    lev_influx[j] -= lev_total[p];
                }
            }
            for (sum, &inflow) in lev_hflux_sum.iter_mut().zip(&lev_influx) {
                *sum -= inflow;
            }
        }
        BifurcationFlux {
            hflux_lev,
            hflux_sum,
            active,
            lev_hflux_sum,
        }
    }
}

#[cfg(test)]
#[path = "bifurcation_tests.rs"]
mod bifurcation_tests;
