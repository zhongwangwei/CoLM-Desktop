//! 网格河湖汇流（`GridRiverLakeFlow`，`MOD_Grid_RiverLakeFlow.F90` 的 `grid_riverlake_flow`）。
//!
//! 默认路径：单向耦合（`DEF_GridRiverLake_FloodFeedback = .false.`），不开堤防、分汊、水库与示踪物。
//! 每个陆面步把 patch 径流按面积汇进单元流域（`acc_rnof_uc`）；累计满 `DEF_GRIDBASED_ROUTING_MAX_DT`
//! 后，在全球网络上按河系自适应子步（不超过 60 s）用 HLL 通量推进水深与动量。
//!
//! 收缩形状取自 latlon 内核的 GIMPLE（`gimpleL/MOD_Grid_RiverLakeFlow.F90`），逐句注明。

pub mod flood;
pub mod history;
pub mod levee;
pub mod network;
pub mod remap;
pub mod restart;

use anyhow::{ensure, Result};
use colm_core::LibmPow;
use rayon::prelude::*;

use network::{RiverNetwork, RunoffRouting, INLAND_DEPRESSION, RIVER_MOUTH};

/// `RIVERMIN`：低于它的水深视为干河。
const RIVERMIN: f64 = 1.0e-5;
/// `MOD_Const_Physical` 的 `grav`。
const GRAV: f64 = 9.80616;

/// `rebuild_volwater_ucat`（`volwater_ucat_valid` 为真时的那一支）：只补「没有蓄量却有水深」的单元。
///
/// 读回续跑与 LULCC（`grid_riverlake_flow_lulcc`）都走这一步；漫滩曲线的深度与体积不互逆，
/// 所以已有的蓄量不能从水深重算。
/// 有堤的单元流域按 `levee_visible_volume_from_stage` 补（只算堤外可见的那份）。
pub fn rebuild_volwater(
    network: &RiverNetwork,
    state: &mut RiverState,
    levee: Option<&levee::Levee>,
) {
    for i in 0..network.len() {
        if state.volwater[i] <= 0.0 && state.wdsrf[i] > RIVERMIN {
            state.volwater[i] = match (levee, state.levsto.as_ref()) {
                (Some(levee), Some(levsto)) if levee.has[i] => {
                    levee.visible_volume_from_stage(network, i, state.wdsrf[i], levsto[i])
                }
                _ => network.curves[i].volume(state.wdsrf[i]),
            };
        }
    }
}

/// 河道状态（`MOD_Grid_RiverLakeTimeVars` 的基本量）。
#[derive(Debug, Clone, PartialEq)]
pub struct RiverState {
    pub wdsrf: Vec<f64>,
    pub veloc: Vec<f64>,
    pub volwater: Vec<f64>,
    pub acc_rnof: Vec<f64>,
    pub acctime_rnof: f64,
    /// `DEF_USE_LEVEE`：堤内受保护的蓄量（m³，续跑里的 `levsto`）；没开堤防时是 `None`。
    pub levsto: Option<Vec<f64>>,
    /// 堤内水深（m）：不进续跑，汇流开头的重新分区会重算它。
    pub levdph: Option<Vec<f64>>,
}

/// 河道 history 累加量（`MOD_Grid_RiverLakeHist` 的 `a_*`，按 `dt` 加权）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RiverHistory {
    pub acctime: Vec<f64>,
    pub wdsrf: Vec<f64>,
    pub veloc: Vec<f64>,
    pub discharge: Vec<f64>,
    pub floodarea: Vec<f64>,
    pub rivsto: Vec<f64>,
    pub fldsto: Vec<f64>,
    pub flddph: Vec<f64>,
    pub storge: Vec<f64>,
    pub sfcelv: Vec<f64>,
    /// `a_levsto`、`a_levdph`：只在 `DEF_USE_LEVEE` 时有。
    pub levsto: Option<Vec<f64>>,
    pub levdph: Option<Vec<f64>>,
}

impl RiverHistory {
    /// 全零的累加量；`levee` 决定有没有堤防那两项。
    pub fn zeros_with_levee(n: usize, levee: bool) -> Self {
        Self {
            levsto: levee.then(|| vec![0.0; n]),
            levdph: levee.then(|| vec![0.0; n]),
            ..Self::zeros(n)
        }
    }

    pub fn zeros(n: usize) -> Self {
        let z = vec![0.0; n];
        Self {
            acctime: z.clone(),
            wdsrf: z.clone(),
            veloc: z.clone(),
            discharge: z.clone(),
            floodarea: z.clone(),
            rivsto: z.clone(),
            fldsto: z.clone(),
            flddph: z.clone(),
            storge: z.clone(),
            sfcelv: z,
            levsto: None,
            levdph: None,
        }
    }
}

impl RiverModel {
    /// 写续跑时的河道 history 累加：运行终点有原始快照就用它。
    pub fn history_for_restart(&self) -> &RiverHistory {
        self.raw_history_at_end.as_ref().unwrap_or(&self.history)
    }
}

/// 汇流模型：网络、区域径流映射、状态与 history 累加。
pub struct RiverModel {
    pub network: RiverNetwork,
    pub routing: RunoffRouting,
    pub state: RiverState,
    pub history: RiverHistory,
    /// `DEF_GRIDBASED_ROUTING_MAX_DT`。
    pub max_dt: f64,
    /// 运行终点那条不满周期的记录写出之前的原始累加（`MOD_Hist.F90:265-274` 先存旁车再写历史）。
    pub raw_history_at_end: Option<RiverHistory>,
    /// `DEF_GridRiverLake_FloodFeedback`：漫滩回馈的 patch 与单元流域状态。
    pub flood: Option<flood::FloodFeedback>,
    /// `DEF_USE_LEVEE`：每个单元流域的堤防几何。
    pub levee: Option<levee::Levee>,
    momen: Vec<f64>,
}

impl RiverModel {
    /// `grid_riverlake_flow_init` 的默认路径：读回状态后 `rebuild_volwater_ucat`。
    pub fn new(
        network: RiverNetwork,
        routing: RunoffRouting,
        mut state: RiverState,
        max_dt: f64,
        levee: Option<levee::Levee>,
    ) -> Result<Self> {
        let n = network.len();
        ensure!(
            state.wdsrf.len() == n
                && state.veloc.len() == n
                && state.volwater.len() == n
                && state.acc_rnof.len() == n,
            "the river restart does not match the unit-catchment network"
        );
        ensure!(
            max_dt.is_finite() && max_dt > 0.0,
            "DEF_GRIDBASED_ROUTING_MAX_DT must be finite and positive"
        );
        // `read_levee_restart`：续跑没带 `levsto` 时堤内蓄量从 0 起；当前配置在某单元流域没有堤
        // （或整体关了堤防）时，堤内蓄量并回堤外可见蓄量。
        let levsto = state.levsto.take();
        if let Some(levsto) = &levsto {
            ensure!(
                levsto.len() == n,
                "levsto does not match the unit-catchment network"
            );
        }
        match (levee.as_ref(), levsto) {
            (Some(levee), levsto) => {
                let mut levsto = levsto.unwrap_or_else(|| vec![0.0; n]);
                for ((protected, visible), &has) in
                    levsto.iter_mut().zip(&mut state.volwater).zip(&levee.has)
                {
                    if !has && *protected > 0.0 {
                        *visible += *protected;
                        *protected = 0.0;
                    }
                }
                state.levsto = Some(levsto);
                state.levdph = Some(vec![0.0; n]);
            }
            (None, Some(levsto)) => {
                for (visible, &protected) in state.volwater.iter_mut().zip(&levsto) {
                    if protected > 0.0 {
                        *visible += protected;
                    }
                }
                // 关了堤防却读到带堤防的续跑：水深按并回后的可见蓄量重算。
                for ((w, &visible), curve) in state
                    .wdsrf
                    .iter_mut()
                    .zip(&state.volwater)
                    .zip(&network.curves)
                {
                    *w = curve.depth(visible);
                }
                state.levdph = None;
            }
            (None, None) => state.levdph = None,
        }
        rebuild_volwater(&network, &mut state, levee.as_ref());
        Ok(Self {
            momen: vec![0.0; n],
            history: RiverHistory::zeros_with_levee(n, levee.is_some()),
            levee,
            network,
            routing,
            state,
            max_dt,
            raw_history_at_end: None,
            flood: None,
        })
    }

    /// 打开漫滩回馈：`grid_riverlake_flow_init` 末尾立刻发布一次（非 spinup）。
    pub fn with_flood_feedback(mut self, infiltration_max_mm_day: f64) -> Self {
        let mut flood =
            flood::FloodFeedback::new(&self.network, &self.routing, infiltration_max_mm_day);
        flood.publish(&self.network, &self.routing, &self.state);
        self.flood = Some(flood);
        self
    }

    /// 一个陆面步：把 patch 径流（mm/s，`rnof`）汇进 `acc_rnof_uc`，满时间就汇流一次。
    ///
    /// `included[p]` 是 `filter_rnof = patchtype < 99 .and. patchmask`。
    pub fn step(&mut self, runoff_mm_s: &[f64], included: &[bool], deltime: f64) -> Result<bool> {
        let routing = &self.routing;
        ensure!(
            runoff_mm_s.len() == routing.patch_parts.len() && included.len() == runoff_mm_s.len(),
            "one runoff value per patch is needed"
        );
        // `worker_remap_data_pset2grid`：填充值 0，值为 0 的 patch 不参与；首份直接赋值。
        let mut grid = vec![0.0; routing.grids.len()];
        for ((parts, &value), &keep) in routing.patch_parts.iter().zip(runoff_mm_s).zip(included) {
            if !keep || value == 0.0 {
                continue;
            }
            for &(k, area) in parts {
                let term = value * area;
                grid[k] = if grid[k] == 0.0 { term } else { grid[k] + term };
            }
        }
        for (value, &area) in grid.iter_mut().zip(&routing.grid_area) {
            if area > 0.0 {
                *value /= area;
            }
        }
        // `push_inpm2ucat`（sum）：同样跳过 0；随后 `acc = FMA(rnof_uc*1e-3, deltime, acc)`。
        for (i, entries) in &routing.catchments {
            let mut sum = 0.0;
            for &(k, area) in entries {
                let Some(k) = k else { continue };
                let value = grid[k];
                if value == 0.0 {
                    continue;
                }
                sum = if sum == 0.0 {
                    value * area
                } else {
                    sum + value * area
                };
            }
            self.state.acc_rnof[*i] = (sum * 1.0e-3).mul_add(deltime, self.state.acc_rnof[*i]);
        }
        // 漫滩回馈：每个陆面步都扣账、再发布（`grid_riverlake_flow` 里汇流判定之前）。
        if let Some(flood) = self.flood.as_mut() {
            flood.accumulate(deltime);
            flood.debit(&self.network, &self.routing, &mut self.state)?;
            flood.publish(&self.network, &self.routing, &self.state);
        }
        self.state.acctime_rnof += deltime;
        if self.state.acctime_rnof + 0.01 < self.max_dt {
            return Ok(false);
        }
        self.route();
        if let Some(flood) = self.flood.as_mut() {
            flood.publish(&self.network, &self.routing, &self.state);
        }
        Ok(true)
    }

    /// 一次汇流：把 `acctime_rnof` 秒按河系子步推进完，然后清零累计径流。
    ///
    /// 河系之间没有耦合（上下游都在同一河系，子步长在河系内取最小），所以逐河系独立推进、
    /// 河系之间并行；河系内部的顺序与上游逐单元流域的循环相同。
    fn route(&mut self) {
        let net = &self.network;
        let state = &self.state;
        let history = &self.history;
        let levee = self.levee.as_ref();
        let acctime = state.acctime_rnof;
        let results = net
            .systems
            .par_iter()
            .map(|system| route_system(net, system, state, history, levee, acctime))
            .collect::<Vec<_>>();
        for (system, result) in net.systems.iter().zip(results) {
            for (k, &i) in system.cells.iter().enumerate() {
                self.state.wdsrf[i] = result.wdsrf[k];
                self.state.veloc[i] = result.veloc[k];
                self.state.volwater[i] = result.volwater[k];
                self.momen[i] = result.momen[k];
                let hist = &mut self.history;
                hist.acctime[i] = result.history[k][0];
                hist.wdsrf[i] = result.history[k][1];
                hist.veloc[i] = result.history[k][2];
                hist.discharge[i] = result.history[k][3];
                hist.floodarea[i] = result.history[k][4];
                hist.rivsto[i] = result.history[k][5];
                hist.fldsto[i] = result.history[k][6];
                hist.flddph[i] = result.history[k][7];
                hist.storge[i] = result.history[k][8];
                hist.sfcelv[i] = result.history[k][9];
                if let Some(lev) = result.levee.as_ref() {
                    self.state.levsto.as_mut().expect("levee state")[i] = lev[k].0;
                    self.state.levdph.as_mut().expect("levee state")[i] = lev[k].1;
                    hist.levsto.as_mut().expect("levee history")[i] = lev[k].2;
                    hist.levdph.as_mut().expect("levee history")[i] = lev[k].3;
                }
            }
        }
        self.state.acctime_rnof = 0.0;
        self.state.acc_rnof.fill(0.0);
    }
}

/// 一个河系汇流后的局部结果（按 `RiverSystem::cells` 次序）。
struct SystemResult {
    wdsrf: Vec<f64>,
    veloc: Vec<f64>,
    volwater: Vec<f64>,
    momen: Vec<f64>,
    history: Vec<[f64; 10]>,
    /// 开堤防时每个单元流域的 `(levsto, levdph, a_levsto, a_levdph)`。
    levee: Option<Vec<(f64, f64, f64, f64)>>,
}

// 阶段循环按单元流域下标写多组数组（与上游逐单元流域的 DO 循环一一对应），用下标更清楚。
#[allow(clippy::needless_range_loop, clippy::manual_clamp)]
/// 一个河系把 `acctime` 秒推进完（`grid_riverlake_flow` 的 `DO WHILE` 循环限于这一河系）。
fn route_system(
    net: &RiverNetwork,
    system: &network::RiverSystem,
    state: &RiverState,
    history: &RiverHistory,
    levee: Option<&levee::Levee>,
    acctime: f64,
) -> SystemResult {
    let cells = &system.cells;
    let n = cells.len();
    let mut wdsrf = Vec::with_capacity(n);
    let mut veloc = Vec::with_capacity(n);
    let mut volwater_ucat = Vec::with_capacity(n);
    let mut momen = Vec::with_capacity(n);
    // 堤防：`levsto/levdph`、累加 `a_levsto/a_levdph` 与本次汇流的 `levee_floodarea`（每次汇流清零）。
    let mut lev = levee.map(|_| {
        let levsto = state.levsto.as_ref().expect("levee state");
        let levdph = state.levdph.as_ref().expect("levee state");
        let a_levsto = history.levsto.as_ref().expect("levee history");
        let a_levdph = history.levdph.as_ref().expect("levee history");
        cells
            .iter()
            .map(|&i| (levsto[i], levdph[i], a_levsto[i], a_levdph[i]))
            .collect::<Vec<_>>()
    });
    let mut levee_floodarea = vec![0.0; n];
    // `levee_repartition_storage`：可见 + 堤内重新分区，返回新的可见蓄量与水深。
    let repartition = |lev: &mut (f64, f64, f64, f64), i: usize, visible: f64, area: &mut f64| {
        let levee = levee.expect("levee");
        let vol_total = visible + lev.0;
        let stage = levee.fldstg(net, i, vol_total);
        lev.0 = stage.levsto;
        lev.1 = stage.levdph;
        *area = stage.fldfrc * net.area[i];
        (vol_total - stage.levsto, stage.wdsrf)
    };
    for (k, &i) in cells.iter().enumerate() {
        let mom = state.wdsrf[i] * state.veloc[i];
        // 开堤防时所有单元流域都以 `volwater_ucat` 为状态，而不是由水深反算。
        let mut volwater = if levee.is_some() {
            state.volwater[i]
        } else {
            net.curves[i].volume(state.wdsrf[i])
        } + state.acc_rnof[i];
        let w = match (levee, lev.as_mut()) {
            (Some(levee), Some(lev)) if levee.has[i] => {
                let (visible, w) = repartition(&mut lev[k], i, volwater, &mut levee_floodarea[k]);
                volwater = visible;
                w
            }
            _ => net.curves[i].depth(volwater),
        };
        momen.push(mom);
        volwater_ucat.push(volwater);
        wdsrf.push(w);
        veloc.push(if w > RIVERMIN { mom / w } else { 0.0 });
    }
    let mut hist = cells
        .iter()
        .map(|&i| {
            [
                history.acctime[i],
                history.wdsrf[i],
                history.veloc[i],
                history.discharge[i],
                history.floodarea[i],
                history.rivsto[i],
                history.fldsto[i],
                history.flddph[i],
                history.storge[i],
                history.sfcelv[i],
            ]
        })
        .collect::<Vec<_>>();
    let mut faces = vec![Face::default(); n];
    let mut sums = vec![(0.0, 0.0, 0.0); n];
    let mut dt_res = acctime;
    while dt_res > 0.0 {
        let mut dt_all = dt_res.min(60.0);
        for k in 0..n {
            faces[k] = face_of(net, system, &wdsrf, &veloc, k, faces[k].zgrad_dn);
        }
        // `push_ups2ucat`（sum，权重 1）：跳过 0 值，首项直接赋值。
        for k in 0..n {
            let push = |value: fn(&Face) -> f64| {
                let mut total = 0.0;
                for &u in &system.upstream[k] {
                    let value = value(&faces[u]);
                    if value == 0.0 {
                        continue;
                    }
                    total = if total == 0.0 {
                        value * 1.0
                    } else {
                        total + value * 1.0
                    };
                }
                total
            };
            let face = &faces[k];
            sums[k] = (
                face.sum_hflux - push(|face| face.hflux),
                face.sum_mflux - push(|face| face.mflux),
                face.sum_zgrad - push(|face| face.zgrad_dn),
            );
        }
        // 子步长：CFL 与蓄量两道限制，逐单元流域取最小。
        for k in 0..n {
            let i = cells[k];
            let mut dt_this = dt_all;
            let (w, v) = (wdsrf[k], veloc[k]);
            if v != 0.0 || w > 0.0 {
                let wave = v.abs() + (w * GRAV).sqrt();
                dt_this = dt_this.min(net.rivlen[i] / wave * 0.8);
            }
            if sums[k].0 > 0.0 {
                let volwater = if levee.is_some() {
                    volwater_ucat[k]
                } else {
                    net.curves[i].volume(w)
                };
                dt_this = dt_this.min(volwater / sums[k].0);
            }
            dt_all = dt_this.min(dt_all);
        }
        let dt = dt_all;
        // 蓄量、水深与动量。
        for k in 0..n {
            let i = cells[k];
            let curve = &net.curves[i];
            let (sum_h, sum_m, sum_z) = sums[k];
            // `volwater = FNMA(visible_hflux, dt, volwater)`。
            let start = if levee.is_some() {
                volwater_ucat[k]
            } else {
                curve.volume(wdsrf[k])
            };
            let mut volwater = (-sum_h).mul_add(dt, start);
            // 无分汊时 `levee_apply_protected_flux` 的受保护通量为 0：堤内蓄量不变，
            // 它重算的 `levdph` 随即被下面的重新分区覆盖。
            volwater = volwater.max(0.0);
            if system.next[k] == INLAND_DEPRESSION && volwater > net.rivstomax[i] {
                faces[k].hflux = (volwater - net.rivstomax[i]) / dt;
                volwater = net.rivstomax[i];
            }
            let w = match (levee, lev.as_mut()) {
                (Some(levee), Some(lev)) if levee.has[i] => {
                    let (visible, w) =
                        repartition(&mut lev[k], i, volwater, &mut levee_floodarea[k]);
                    volwater = visible;
                    w
                }
                _ => curve.depth(volwater),
            };
            wdsrf[k] = w;
            volwater_ucat[k] = volwater;
            if w >= RIVERMIN {
                let manning = net.rivman[i];
                let friction = (manning * manning * GRAV / w.lpow(7.0 / 3.0)) * momen[k].abs();
                let gradient = (sum_m - sum_z) / net.rivare[i];
                momen[k] = (-gradient).mul_add(dt, momen[k]) / dt.mul_add(friction, 1.0);
                veloc[k] = momen[k] / w;
            } else {
                momen[k] = 0.0;
                veloc[k] = 0.0;
            }
            if system.next[k] == INLAND_DEPRESSION {
                momen[k] = momen[k].min(0.0);
                veloc[k] = veloc[k].min(0.0);
            }
            veloc[k] = veloc[k].min(20.0).max(-20.0);
        }
        // history 累加：`a_x = FMA(x, dt, a_x)`，`acctime` 平铺相加。
        for k in 0..n {
            let i = cells[k];
            let curve = &net.curves[i];
            let w = wdsrf[k];
            let volwater = if levee.is_some() {
                volwater_ucat[k]
            } else {
                curve.volume(w)
            };
            let rivsto = volwater.min(curve.rivstomax);
            let floodarea = if levee_floodarea[k] > 0.0 {
                levee_floodarea[k]
            } else {
                curve.floodarea(w)
            };
            let a = &mut hist[k];
            a[0] += dt;
            a[1] = w.mul_add(dt, a[1]);
            a[2] = veloc[k].mul_add(dt, a[2]);
            // `a_discharge + hflux_fc*dt` 不融合：乘积与调试用的 `totaldis` 共用（`_7820`）。
            a[3] += faces[k].hflux * dt;
            a[4] = floodarea.mul_add(dt, a[4]);
            a[5] = rivsto.mul_add(dt, a[5]);
            a[6] = (volwater - rivsto).mul_add(dt, a[6]);
            a[7] = (w - curve.rivhgt).max(0.0).mul_add(dt, a[7]);
            match (levee, lev.as_mut()) {
                (Some(levee), Some(lev)) if levee.has[i] => {
                    let l = &mut lev[k];
                    a[8] = (volwater + l.0).mul_add(dt, a[8]);
                    l.2 = l.0.mul_add(dt, l.2);
                    l.3 = l.1.mul_add(dt, l.3);
                }
                _ => a[8] = volwater.mul_add(dt, a[8]),
            }
            a[9] = (net.rivelv[i] + w).mul_add(dt, a[9]);
        }
        dt_res -= dt;
    }
    SystemResult {
        wdsrf,
        veloc,
        volwater: volwater_ucat,
        momen,
        history: hist,
        levee: lev,
    }
}

/// 一个单元流域出口面本子步的通量与它自己的收支初值。
#[derive(Debug, Clone, Copy, Default)]
struct Face {
    hflux: f64,
    mflux: f64,
    zgrad_dn: f64,
    sum_hflux: f64,
    sum_mflux: f64,
    sum_zgrad: f64,
}

/// 河系内第 `k` 个单元流域出口面本子步的通量（`bb 553-588`）。`stale_zgrad` 是洼地保留的旧
/// `zgrad_dn`（上游不在洼地之后，从不被读）。
fn face_of(
    net: &RiverNetwork,
    system: &network::RiverSystem,
    wdsrf: &[f64],
    veloc: &[f64],
    k: usize,
    stale_zgrad: f64,
) -> Face {
    let next = system.next[k];
    if next == INLAND_DEPRESSION {
        return Face {
            zgrad_dn: stale_zgrad,
            ..Face::default()
        };
    }
    let i = system.cells[k];
    let (w, v) = (wdsrf[k], veloc[k]);
    let (w_next, v_next) = if next >= 0 {
        (wdsrf[next as usize], veloc[next as usize])
    } else {
        (colm_core::MISSING, 0.0)
    };
    if next >= 0 && w < RIVERMIN && w_next < RIVERMIN {
        return Face::default();
    }
    let (height_up, height_dn) = if next >= 0 {
        let bedelv = net.rivelv[i].max(net.bedelv_next[i]);
        (
            ((w + net.rivelv[i]) - bedelv).max(0.0),
            ((net.bedelv_next[i] + w_next) - bedelv).max(0.0),
        )
    } else {
        debug_assert_eq!(next, RIVER_MOUTH);
        (w, (-net.rivelv[i]).max(0.0))
    };
    let (h, m) = hll_flux(v, v_next, height_up, height_dn, net.outletwth[i]);
    let coefficient = net.outletwth[i] * 0.5 * GRAV;
    Face {
        hflux: h,
        mflux: m,
        zgrad_dn: coefficient * (height_dn * height_dn),
        sum_hflux: h + 0.0,
        sum_mflux: m + 0.0,
        sum_zgrad: coefficient.mul_add(height_up * height_up, 0.0),
    }
}

/// 一个出口面的 HLL 通量（`bb 564-575`）：返回 `(hflux, mflux)`，已乘出口宽度。
fn hll_flux(v: f64, v_next: f64, height_up: f64, height_dn: f64, width: f64) -> (f64, f64) {
    let c_up = (height_up * GRAV).sqrt();
    let c_dn = (height_dn * GRAV).sqrt();
    // `veloct = FMA(v+vn, 0.5, c_up) - c_dn`；`height_fc = FMA(c_up+c_dn, 0.5, (v-vn)*0.25)^2 * (1/g)`。
    let veloct = (v + v_next).mul_add(0.5, c_up) - c_dn;
    let root = (c_up + c_dn).mul_add(0.5, (v - v_next) * 0.25);
    let height_fc = root * root * (1.0 / GRAV);
    let c_fc = (height_fc * GRAV).sqrt();
    let wave_up = if height_up > 0.0 {
        (v - c_up).min(veloct - c_fc)
    } else {
        (-c_dn).mul_add(2.0, v_next)
    };
    let wave_dn = if height_dn > 0.0 {
        (v_next + c_dn).max(c_fc + veloct)
    } else {
        c_up.mul_add(2.0, v)
    };
    let half_g = 0.5 * GRAV;
    let hflux_up = v * height_up;
    let mflux_up = height_up.mul_add(v * v, height_up * height_up * half_g);
    if wave_up >= 0.0 {
        return (hflux_up * width, mflux_up * width);
    }
    let hflux_dn = v_next * height_dn;
    let mflux_dn = height_dn.mul_add(v_next * v_next, height_dn * height_dn * half_g);
    if wave_dn <= 0.0 {
        return (hflux_dn * width, mflux_dn * width);
    }
    let product = wave_dn * wave_up;
    let spread = wave_dn - wave_up;
    let h = product.mul_add(
        height_dn - height_up,
        wave_dn.mul_add(hflux_up, -(wave_up * hflux_dn)),
    );
    let m = product.mul_add(
        hflux_dn - hflux_up,
        wave_dn.mul_add(mflux_up, -(wave_up * mflux_dn)),
    );
    (h * width / spread, m * width / spread)
}

#[cfg(test)]
#[path = "river_tests.rs"]
mod river_tests;
