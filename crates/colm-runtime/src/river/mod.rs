//! 网格河湖汇流（`GridRiverLakeFlow`，`MOD_Grid_RiverLakeFlow.F90` 的 `grid_riverlake_flow`）。
//!
//! 默认路径：单向耦合（`DEF_GridRiverLake_FloodFeedback = .false.`），不开堤防、分汊、水库与示踪物。
//! 每个陆面步把 patch 径流按面积汇进单元流域（`acc_rnof_uc`）；累计满 `DEF_GRIDBASED_ROUTING_MAX_DT`
//! 后，在全球网络上按河系自适应子步（不超过 60 s）用 HLL 通量推进水深与动量。
//!
//! 收缩形状取自 latlon 内核的 GIMPLE（`gimpleL/MOD_Grid_RiverLakeFlow.F90`），逐句注明。

pub mod bifurcation;
pub mod flood;
pub mod history;
pub mod levee;
pub mod network;
pub mod remap;
pub mod reservoir;
pub mod restart;
pub mod sediment;
pub mod tracer;

use anyhow::{ensure, Context, Result};
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
    /// `DEF_USE_BIFURCATION`：上一子步水深与路径状态；没开分汊时是 `None`。
    pub bifurcation: Option<BifurcationState>,
    /// `DEF_Reservoir_Method > 0`：逐水库库容（m³，续跑里的 `volresv`，按参数表行序）；
    /// 尚未建成的是 `spval`。
    pub volresv: Option<Vec<f64>>,
}

/// 分汊的续跑状态（`wdsrf_ucat_prev`、`pth_veloc`、`pth_momen`、`bif_path_signature`）。
#[derive(Debug, Clone, PartialEq)]
pub struct BifurcationState {
    /// `npthlev_bif`。
    pub levels: usize,
    /// `wdsrf_ucat_prev`：上一个分汊子步开头的水深。
    pub wdsrf_prev: Vec<f64>,
    /// `pth_veloc`、`pth_momen`：逐路径逐层，`[p*levels + l]`。
    pub veloc: Vec<f64>,
    pub momen: Vec<f64>,
    /// 路径签名：读回时是续跑里的，构造模型时与当前网络核对。
    pub signature: Vec<f64>,
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
    /// `a_bifout`（逐单元流域）、`a_bifflw_lev`（逐路径逐层）、`a_bifflw_acctime`（逐路径）：
    /// 只在 `DEF_USE_BIFURCATION` 时有。
    pub bifout: Option<Vec<f64>>,
    pub bifflw_lev: Option<Vec<f64>>,
    pub bifflw_acctime: Option<Vec<f64>>,
    /// `acctime_resv`、`a_volresv`、`a_qresv_in`、`a_qresv_out`（逐水库）：只在开水库时有。
    pub acctime_resv: Option<Vec<f64>>,
    pub volresv: Option<Vec<f64>>,
    pub qresv_in: Option<Vec<f64>>,
    pub qresv_out: Option<Vec<f64>>,
}

impl RiverHistory {
    /// 全零的累加量；`levee` 决定有没有堤防那两项，`bifurcation = (路径数, 层数)` 决定分汊那三项。
    pub fn zeros_for(
        n: usize,
        levee: bool,
        bifurcation: Option<(usize, usize)>,
        reservoirs: Option<usize>,
    ) -> Self {
        Self {
            acctime_resv: reservoirs.map(|m| vec![0.0; m]),
            volresv: reservoirs.map(|m| vec![0.0; m]),
            qresv_in: reservoirs.map(|m| vec![0.0; m]),
            qresv_out: reservoirs.map(|m| vec![0.0; m]),
            levsto: levee.then(|| vec![0.0; n]),
            levdph: levee.then(|| vec![0.0; n]),
            bifout: bifurcation.map(|_| vec![0.0; n]),
            bifflw_lev: bifurcation.map(|(paths, levels)| vec![0.0; paths * levels]),
            bifflw_acctime: bifurcation.map(|(paths, _)| vec![0.0; paths]),
            ..Self::zeros(n)
        }
    }

    /// 写完一条记录后清零（`MOD_Grid_RiverLakeHist` 末尾的 `a_* = 0`），保留各项是否存在。
    pub fn reset(&mut self) {
        for v in [
            &mut self.acctime,
            &mut self.wdsrf,
            &mut self.veloc,
            &mut self.discharge,
            &mut self.floodarea,
            &mut self.rivsto,
            &mut self.fldsto,
            &mut self.flddph,
            &mut self.storge,
            &mut self.sfcelv,
        ] {
            v.fill(0.0);
        }
        for v in [
            &mut self.levsto,
            &mut self.levdph,
            &mut self.bifout,
            &mut self.bifflw_lev,
            &mut self.bifflw_acctime,
            &mut self.acctime_resv,
            &mut self.volresv,
            &mut self.qresv_in,
            &mut self.qresv_out,
        ]
        .into_iter()
        .flatten()
        {
            v.fill(0.0);
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
            bifout: None,
            bifflw_lev: None,
            bifflw_acctime: None,
            acctime_resv: None,
            volresv: None,
            qresv_in: None,
            qresv_out: None,
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
    /// `DEF_USE_BIFURCATION`：分汊路径，以及把全网当成一个河系的拓扑（分汊跨河系，
    /// 上游这时把所有河系同步到同一子步长）。
    pub bifurcation: Option<(bifurcation::Bifurcation, network::RiverSystem)>,
    /// `DEF_Reservoir_Method = 1`：网络里的水库参数。
    pub reservoir: Option<reservoir::Reservoir>,
    /// `DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT`：子步长再加一道「不让流向反转」的限制。
    pub momentum_dt_limit: bool,
    /// `DEF_USE_TRACER`（有走通用输运的示踪物）：河湖示踪物（[`tracer`]）。
    pub tracers: Option<tracer::RiverTracers>,
    /// `SEDIMENT` provider 示踪物：河道泥沙（[`sediment`]）。
    pub sediment: Option<sediment::Sediment>,
    /// `DEF_USE_TRACER` 但没有输运示踪物：续跑里仍提交空的河道示踪物事务。
    pub empty_tracer_transaction: bool,
    /// 注册了 CH4 provider：每次汇流末推到 patch 的淹没比例（见 [`MethaneFlood`]）。
    pub methane_flood: Option<MethaneFlood>,
    /// 本次汇流每个单元流域的 `[total_floodarea, total_flooddepth, levee_floodarea]`；
    /// 不在任何河系里的单元流域保持 0（上游每次汇流重新分配、置 0）。
    flood_uc: Vec<[f64; 3]>,
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
        bifurcation: Option<bifurcation::Bifurcation>,
        reservoir: Option<reservoir::Reservoir>,
    ) -> Result<Self> {
        let n = network.len();
        // 水库库容由续跑读回（`read_river_state` 已核对水库标识）；没开水库时不带。
        match (&reservoir, &state.volresv) {
            (Some(reservoir), Some(volresv)) => ensure!(
                volresv.len() == reservoir.len(),
                "volresv does not match the reservoir table"
            ),
            (Some(reservoir), None) => ensure!(
                reservoir.is_empty(),
                "the river restart has no volresv for the reservoirs in the network"
            ),
            (None, _) => state.volresv = None,
        }
        if let (Some(_), None) = (&reservoir, &state.volresv) {
            state.volresv = Some(Vec::new());
        }
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
        let restart_levee = state.levsto.is_some();
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
                // 水库单元流域的堤内蓄量无法并进可见蓄量（它的水在 `volresv` 里）：上游报错。
                if let Some(reservoir) = reservoir.as_ref() {
                    ensure!(
                        reservoir
                            .of_catchment
                            .iter()
                            .zip(&levsto)
                            .all(|(r, &protected)| r.is_none() || protected <= 0.0),
                        "protected storage exists on a reservoir cell; levee-to-reservoir restart \
                         mapping is ambiguous"
                    );
                }
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
                // 关了堤防却读到带堤防的续跑：水深按并回后的可见蓄量重算（开分汊时状态本就是蓄量，
                // 上游不重算）。
                if bifurcation.is_none() {
                    for ((w, &visible), curve) in state
                        .wdsrf
                        .iter_mut()
                        .zip(&state.volwater)
                        .zip(&network.curves)
                    {
                        *w = curve.depth(visible);
                    }
                }
                state.levdph = None;
            }
            (None, None) => state.levdph = None,
        }
        // `read_bifurcation_restart`：续跑带分汊状态时核对路径签名；状态越界或堤防开关变了就冷启动
        // 路径状态，这时上一子步水深取当前水深。没开分汊时丢掉续跑里的分汊状态。
        let restart_bifurcation = state.bifurcation.take();
        if let Some(bif) = bifurcation.as_ref() {
            let cells = bif.paths() * bif.levels;
            let signature = bif.signature();
            let cold = || BifurcationState {
                levels: bif.levels,
                wdsrf_prev: state.wdsrf.clone(),
                veloc: vec![0.0; cells],
                momen: vec![0.0; cells],
                signature: signature.clone(),
            };
            let resumed = match restart_bifurcation {
                Some(restart) => {
                    ensure!(
                        restart.signature == signature,
                        "Refusing to load bifurcation momentum for a different pathway network"
                    );
                    ensure!(
                        restart.wdsrf_prev.len() == n
                            && restart.veloc.len() == cells
                            && restart.momen.len() == cells,
                        "the bifurcation restart does not match the pathway network"
                    );
                    let valid = restart.veloc.iter().zip(&restart.momen).all(|(v, m)| {
                        v.is_finite() && v.abs() <= 50.0 && m.is_finite() && m.abs() <= 1.0e4
                    });
                    ensure!(
                        valid,
                        "GridRiverLake restart declares bifurcation enabled but pathway state is invalid"
                    );
                    // 续跑与当前的堤防开关不同：路径状态冷启动。
                    if restart_levee != levee.is_some() {
                        cold()
                    } else {
                        BifurcationState {
                            levels: bif.levels,
                            signature: signature.clone(),
                            ..restart
                        }
                    }
                }
                None => cold(),
            };
            state.bifurcation = Some(resumed);
        }
        rebuild_volwater(&network, &mut state, levee.as_ref());
        let history = RiverHistory::zeros_for(
            n,
            levee.is_some(),
            bifurcation.as_ref().map(|bif| (bif.paths(), bif.levels)),
            reservoir.as_ref().map(reservoir::Reservoir::len),
        );
        let bifurcation = bifurcation.map(|bif| {
            // 全网当成一个河系：单元流域按全局序号，上下游关系不变。
            let system = network::RiverSystem {
                cells: (0..n).collect(),
                next: network.next.clone(),
                upstream: network.upstream.clone(),
            };
            (bif, system)
        });
        Ok(Self {
            momen: vec![0.0; n],
            history,
            levee,
            bifurcation,
            reservoir,
            momentum_dt_limit: false,
            network,
            routing,
            state,
            max_dt,
            raw_history_at_end: None,
            flood: None,
            tracers: None,
            sediment: None,
            empty_tracer_transaction: false,
            methane_flood: None,
            flood_uc: Vec::new(),
        })
    }

    /// 注册了 CH4 provider（`tracer_lifecycle_has_flood_publisher`）：每次汇流末把淹没比例推到 patch。
    pub fn with_methane_flood(mut self) -> Self {
        let patches = self.routing.patch_parts.len();
        self.methane_flood = Some(MethaneFlood {
            levee: vec![0.0; patches],
            fraction: vec![0.0; patches],
            depth: vec![0.0; patches],
            published: false,
        });
        self
    }

    /// 挂上河湖示踪物（`river_lake_tracer_init`）。
    pub fn with_tracers(mut self, tracers: tracer::RiverTracers) -> Result<Self> {
        self.tracers = Some(tracers);
        Ok(self)
    }

    /// 挂上河道泥沙（`grid_sediment_init`）：按汇流的堤防与分汊配置打开堤内泥沙池与路径毛水量。
    /// 续跑随后由 [`sediment::Sediment::read_restart`] 读回（schema 依赖这两个开关）。
    pub fn with_sediment(mut self, mut sediment: sediment::Sediment) -> Result<Self> {
        ensure!(
            sediment.shearvel.len() == self.network.len(),
            "sediment state does not match the unit-catchment network"
        );
        sediment.levee = self.levee.as_ref().map(|levee| levee.has.clone());
        sediment.bif_acc = self
            .bifurcation
            .as_ref()
            .map(|(bif, _)| sediment::BifWaterAcc::new(bif.paths(), bif.levels));
        self.sediment = Some(sediment);
        Ok(self)
    }

    /// 打开漫滩回馈：`grid_riverlake_flow_init` 末尾立刻发布一次（非 spinup）。
    ///
    /// `start_year` 是 `publish_flood_feedback(start_year)` 判断水库是否已建成用的年份。
    pub fn with_flood_feedback(
        mut self,
        infiltration_max_mm_day: f64,
        start_year: i32,
    ) -> Result<Self> {
        let mut flood =
            flood::FloodFeedback::new(&self.network, &self.routing, infiltration_max_mm_day);
        // 有输运示踪物时另开示踪物账（上游在河湖示踪物初始化之后分配并发布，所以示踪物要先挂上）。
        if let Some(tracers) = self.tracers.as_ref() {
            flood.tracer = Some(flood::FloodTracers::new(
                tracers.set.len(),
                self.network.len(),
                self.routing.patch_parts.len(),
            ));
        }
        let context = flood::FloodContext {
            levee: self.levee.as_ref(),
            reservoir: self.reservoir.as_ref().map(|r| (r, start_year)),
        };
        flood.publish(
            &self.network,
            &self.routing,
            &mut self.state,
            context,
            self.tracers.as_mut(),
        )?;
        self.flood = Some(flood);
        Ok(self)
    }

    /// 一个陆面步：把 patch 径流（mm/s，`rnof`）汇进 `acc_rnof_uc`，满时间就汇流一次。
    ///
    /// `included[p]` 是 `filter_rnof = patchtype < 99 .and. patchmask`。
    ///
    /// `year` 是 `grid_riverlake_flow(idate(1), …)` 的年份：上游先 `TICKTIME` 再调陆面与汇流，
    /// 所以是本步末的年份；水库按它判断是否已建成。
    pub fn step(
        &mut self,
        runoff_mm_s: &[f64],
        tracer_runoff: Option<&[Vec<f64>]>,
        precip: Option<&[(f64, f64)]>,
        included: &[bool],
        deltime: f64,
        year: i32,
    ) -> Result<bool> {
        let routing = &self.routing;
        ensure!(
            runoff_mm_s.len() == routing.patch_parts.len() && included.len() == runoff_mm_s.len(),
            "one runoff value per patch is needed"
        );
        let rnof_uc = remap_to_catchments(routing, runoff_mm_s, included, self.network.len());
        // `acc = FMA(rnof_uc*1e-3, deltime, acc)`。
        for (i, _) in &routing.catchments {
            self.state.acc_rnof[*i] =
                (rnof_uc[*i] * 1.0e-3).mul_add(deltime, self.state.acc_rnof[*i]);
        }
        // 示踪物径流同一套映射（`trc_rnof_step` → `trc_rnof_uc`），再
        // `tracer_input_from_runoff(rnof_uc*1e-3*deltime, numucat, trc_rnof_uc*1e-3)`。
        if let Some(tracers) = self.tracers.as_mut() {
            let tracer_runoff = tracer_runoff
                .context("river tracers need the patch runoff tracer (trc_rnof_step)")?;
            let mut trc = vec![vec![0.0; self.network.len()]; tracers.set.len()];
            for itrc in tracers.set.transport_indices().collect::<Vec<_>>() {
                let uc = remap_to_catchments(
                    routing,
                    &tracer_runoff[itrc],
                    included,
                    self.network.len(),
                );
                for (i, _) in &routing.catchments {
                    trc[itrc][*i] = uc[*i] * 1.0e-3;
                }
            }
            let mut depth = vec![0.0; self.network.len()];
            for (i, _) in &routing.catchments {
                depth[*i] = (rnof_uc[*i] * 1.0e-3) * deltime;
            }
            tracers.input_from_runoff(&depth, &trc);
        }
        // `tracer_lifecycle_route_forcing_put`：泥沙的降水 `forc_prc + forc_prl`（mm/s），只取
        // 有效、非负的 patch；单元流域上的值是有效面积上的平均，另给有效面积占单元流域的份额。
        if let Some(sediment) = self.sediment.as_mut() {
            let precip = precip.context("river sediment needs the patch precipitation")?;
            ensure!(
                precip.len() == included.len(),
                "one precipitation pair is needed per patch"
            );
            let n = self.network.len();
            let filter = included
                .iter()
                .zip(precip)
                .map(|(&keep, &(prc, prl))| {
                    keep && prc.is_finite()
                        && prl.is_finite()
                        && prc != colm_core::MISSING
                        && prl != colm_core::MISSING
                        && prc >= 0.0
                        && prl >= 0.0
                })
                .collect::<Vec<_>>();
            let total = precip
                .iter()
                .zip(&filter)
                .map(|(&(prc, prl), &keep)| if keep { prc + prl } else { 0.0 })
                .collect::<Vec<_>>();
            let mut prcp_uc = remap_to_catchments(routing, &total, &filter, n);
            let mut area_uc = remap_to_catchments(routing, &vec![1.0; total.len()], &filter, n);
            for i in 0..n {
                prcp_uc[i] = if area_uc[i] > 0.0 {
                    prcp_uc[i] / area_uc[i]
                } else {
                    0.0
                };
                area_uc[i] = if self.network.area[i] > 0.0 {
                    area_uc[i] / self.network.area[i]
                } else {
                    0.0
                };
            }
            sediment.forcing_put(&prcp_uc, deltime, &area_uc)?;
        }
        // 漫滩回馈：每个陆面步都扣账、再发布（`grid_riverlake_flow` 里汇流判定之前）。
        if let Some(flood) = self.flood.as_mut() {
            flood.accumulate(deltime);
            let context = flood::FloodContext {
                levee: self.levee.as_ref(),
                reservoir: self.reservoir.as_ref().map(|r| (r, year)),
            };
            let mut repartitions = Vec::new();
            flood.debit(
                &self.network,
                &self.routing,
                &mut self.state,
                context,
                &mut repartitions,
                self.tracers.as_mut(),
            )?;
            // 扣账后的堤防重新分区也要告诉泥沙（`tracer_lifecycle_route_sediment_levee_repartition`）。
            if let Some(sediment) = self.sediment.as_mut() {
                for (j, water) in repartitions {
                    sediment.acc[j].levee_repartition(water)?;
                }
            }
            flood.publish(
                &self.network,
                &self.routing,
                &mut self.state,
                context,
                self.tracers.as_mut(),
            )?;
        }
        self.state.acctime_rnof += deltime;
        if self.state.acctime_rnof + 0.01 < self.max_dt {
            return Ok(false);
        }
        self.route(year)?;
        if let Some(methane) = self.methane_flood.as_mut() {
            methane.publish(&self.network, &self.routing, &self.flood_uc);
        }
        if let Some(flood) = self.flood.as_mut() {
            let context = flood::FloodContext {
                levee: self.levee.as_ref(),
                reservoir: self.reservoir.as_ref().map(|r| (r, year)),
            };
            flood.publish(
                &self.network,
                &self.routing,
                &mut self.state,
                context,
                self.tracers.as_mut(),
            )?;
        }
        Ok(true)
    }

    /// 一次汇流：把 `acctime_rnof` 秒按河系子步推进完，然后清零累计径流。
    ///
    /// 河系之间没有耦合（上下游都在同一河系，子步长在河系内取最小），所以逐河系独立推进、
    /// 河系之间并行；河系内部的顺序与上游逐单元流域的循环相同。
    fn route(&mut self, year: i32) -> Result<()> {
        // 分汊：把路径状态与累加量搬进这次汇流，全网一个河系推进，结束后放回。
        let bif_run = self.bifurcation.as_ref().map(|(bif, _)| BifurcationRun {
            bif,
            state: self.state.bifurcation.take().expect("bifurcation state"),
            bifout: self.history.bifout.take().expect("bifurcation history"),
            bifflw_lev: self.history.bifflw_lev.take().expect("bifurcation history"),
            bifflw_acctime: self
                .history
                .bifflw_acctime
                .take()
                .expect("bifurcation history"),
            layer_flux: Vec::new(),
            layer_active: Vec::new(),
        });
        let net = &self.network;
        let state = &self.state;
        let history = &self.history;
        let levee = self.levee.as_ref();
        let resv = self.reservoir.as_ref().map(|reservoir| (reservoir, year));
        let momentum_limit = self.momentum_dt_limit;
        let acctime = state.acctime_rnof;
        let record = self.tracers.is_some();
        let sed_acc = self.sediment.as_ref().map(|sediment| SedimentIn {
            acc: sediment.acc.as_slice(),
            bif: sediment.bif_acc.as_ref(),
            levee: levee.filter(|_| sediment.levee.is_some()),
        });
        let (systems, results): (Vec<&network::RiverSystem>, Vec<SystemResult<'_>>) =
            match (bif_run, self.bifurcation.as_ref()) {
                (Some(run), Some((_, global))) => (
                    vec![global],
                    vec![route_system(
                        net,
                        global,
                        state,
                        history,
                        levee,
                        resv,
                        Some(run),
                        momentum_limit,
                        acctime,
                        record,
                        sed_acc,
                    )],
                ),
                _ => (
                    net.systems.iter().collect(),
                    net.systems
                        .par_iter()
                        .map(|system| {
                            route_system(
                                net,
                                system,
                                state,
                                history,
                                levee,
                                resv,
                                None,
                                momentum_limit,
                                acctime,
                                record,
                                sed_acc,
                            )
                        })
                        .collect(),
                ),
            };
        let mut bif_back = None;
        let mut tapes = Vec::new();
        ensure!(
            results.iter().all(|result| !result.protected_failed),
            "BIF protected-side limiter failed"
        );
        self.flood_uc = vec![[0.0; 3]; self.network.len()];
        for (system, mut result) in systems.into_iter().zip(results) {
            for &(r, volresv, a) in &result.reservoirs {
                self.state.volresv.as_mut().expect("reservoir state")[r] = volresv;
                let hist = &mut self.history;
                hist.acctime_resv.as_mut().expect("reservoir history")[r] = a[0];
                hist.volresv.as_mut().expect("reservoir history")[r] = a[1];
                hist.qresv_in.as_mut().expect("reservoir history")[r] = a[2];
                hist.qresv_out.as_mut().expect("reservoir history")[r] = a[3];
            }
            if let Some(run) = result.bifurcation.take() {
                bif_back = Some((run.state, run.bifout, run.bifflw_lev, run.bifflw_acctime));
            }
            if let Some(tape) = result.tape.take() {
                tapes.push(tape);
            }
            if let (Some(sediment), Some(out)) = (self.sediment.as_mut(), result.sediment.take()) {
                if let Some(error) = out.error {
                    return Err(error);
                }
                for (k, &i) in system.cells.iter().enumerate() {
                    sediment.acc[i] = out.acc[k];
                }
                if out.bif.is_some() {
                    sediment.bif_acc = out.bif;
                }
            }
            for (k, &i) in system.cells.iter().enumerate() {
                self.flood_uc[i] = result.flood[k];
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
        if let Some((state, bifout, bifflw_lev, bifflw_acctime)) = bif_back {
            self.state.bifurcation = Some(state);
            self.history.bifout = Some(bifout);
            self.history.bifflw_lev = Some(bifflw_lev);
            self.history.bifflw_acctime = Some(bifflw_acctime);
        }
        // 示踪物按全局子步重放这次汇流（见 [`tracer`]）。
        if let Some(tracers) = self.tracers.as_mut() {
            tracers.replay(
                &self.network,
                self.levee.as_ref(),
                self.bifurcation.as_ref().map(|(bif, _)| bif),
                &tapes,
            )?;
        }
        // `tracer_lifecycle_route_calc(acctime_rnof)`：一次汇流之后的泥沙形态步。
        if let Some(sediment) = self.sediment.as_mut() {
            sediment.calc(
                &self.network,
                self.bifurcation.as_ref().map(|(bif, _)| bif),
                self.state.acctime_rnof,
            )?;
        }
        self.state.acctime_rnof = 0.0;
        self.state.acc_rnof.fill(0.0);
        Ok(())
    }
}

/// CH4 provider 的淹没比例（`publish_fldfrc_to_patches`、`publish_levee_fldfrc_to_patches`）：
/// 单元流域 → 输入网格 → patch，填充值 `RIVERLAKE_FLOOD_MISSING_VALUE`，都取 `average`，缺测记 0。
#[derive(Debug, Clone)]
pub struct MethaneFlood {
    pub levee: Vec<f64>,
    pub fraction: Vec<f64>,
    pub depth: Vec<f64>,
    /// 上次取走之后又汇流过一次（只在那时覆盖 patch 上的甲烷状态）。
    pub published: bool,
}

/// `RIVERLAKE_FLOOD_MISSING_VALUE`（`MOD_Grid_RiverLakeFlow.F90:39`）。
const FLOOD_MISSING: f64 = -1.0e30;

impl MethaneFlood {
    // `max(0).min(1)` 照搬上游 `min(max(x,0),1)`：NaN 落到 0，`clamp` 会把 NaN 原样放过。
    #[allow(clippy::manual_clamp)]
    fn publish(&mut self, network: &RiverNetwork, routing: &RunoffRouting, flood_uc: &[[f64; 3]]) {
        let n = network.len();
        let fraction_of = |area: f64, i: usize| {
            if network.area[i] > 0.0 {
                (area / network.area[i]).max(0.0).min(1.0)
            } else {
                0.0
            }
        };
        let to_patches = |uc: &[f64]| -> Vec<f64> {
            let grid = remap::catchments_to_inpm(routing, uc, FLOOD_MISSING, true);
            remap::grid_to_patches(routing, &grid, FLOOD_MISSING, true)
                .into_iter()
                .map(|v| if v == FLOOD_MISSING { 0.0 } else { v })
                .collect()
        };
        let levee_uc: Vec<f64> = (0..n).map(|i| fraction_of(flood_uc[i][2], i)).collect();
        self.levee = to_patches(&levee_uc);
        let fraction_uc: Vec<f64> = (0..n).map(|i| fraction_of(flood_uc[i][0], i)).collect();
        // `fldwat_uc = fldfrc_uc * max(0, total_flooddepth)`（面积为 0 的单元流域是 0）。
        let water_uc: Vec<f64> = (0..n)
            .map(|i| fraction_uc[i] * flood_uc[i][1].max(0.0))
            .collect();
        self.fraction = to_patches(&fraction_uc);
        let water = to_patches(&water_uc);
        self.depth = self
            .fraction
            .iter()
            .zip(&water)
            .map(|(&f, &w)| if f > 0.0 { w.max(0.0) / f } else { 0.0 })
            .collect();
        self.published = true;
    }
}

/// patch → 输入网格 → 单元流域（`worker_remap_data_pset2grid` + `push_inpm2ucat`，填充 0，
/// 值为 0 的 patch/格子不参与，首份直接赋值）；返回每个单元流域的值（不在区域里的为 0）。
fn remap_to_catchments(
    routing: &RunoffRouting,
    values: &[f64],
    included: &[bool],
    n: usize,
) -> Vec<f64> {
    let mut grid = vec![0.0; routing.grids.len()];
    for ((parts, &value), &keep) in routing.patch_parts.iter().zip(values).zip(included) {
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
    let mut out = vec![0.0; n];
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
        out[*i] = sum;
    }
    out
}

/// 磁带里一个单元流域的水（水库取 `volresv`，有堤带堤内蓄量）。
fn cell_water(
    k: usize,
    wdsrf: &[f64],
    volwater_ucat: &[f64],
    lev: &Option<Vec<(f64, f64, f64, f64)>>,
    built: &[Option<usize>],
    reservoirs: &[(usize, f64, [f64; 4])],
) -> tracer::CellWater {
    tracer::CellWater {
        wdsrf: wdsrf[k],
        volwater_ucat: volwater_ucat[k],
        volresv: built[k].map(|j| reservoirs[j].1),
        levsto: lev.as_ref().map(|lev| lev[k].0),
    }
}

/// 一个河系汇流后的局部结果（按 `RiverSystem::cells` 次序）。
struct SystemResult<'a> {
    wdsrf: Vec<f64>,
    veloc: Vec<f64>,
    volwater: Vec<f64>,
    momen: Vec<f64>,
    history: Vec<[f64; 10]>,
    /// 开堤防时每个单元流域的 `(levsto, levdph, a_levsto, a_levdph)`。
    levee: Option<Vec<(f64, f64, f64, f64)>>,
    bifurcation: Option<BifurcationRun<'a>>,
    /// 本河系里已建成的水库：`(水库号, volresv, [acctime, a_volresv, a_qresv_in, a_qresv_out])`。
    reservoirs: Vec<(usize, f64, [f64; 4])>,
    /// 堤内一侧的分汊出流扣穿了堤内蓄量（上游 `BIF protected-side limiter failed`）。
    protected_failed: bool,
    /// 开示踪物时这一河系的磁带。
    tape: Option<tracer::SystemTape>,
    /// 开泥沙时逐单元流域的水量累加（`sediment_diag_accumulate`）。
    sediment: Option<SedimentOut>,
    /// 每个单元流域最后一个子步的 `(total_floodarea, total_flooddepth)` 与汇流末的
    /// `levee_floodarea`（`publish_fldfrc_to_patches` 与 `publish_levee_fldfrc_to_patches` 的输入）。
    flood: Vec<[f64; 3]>,
}

/// 汇流开始时交给一个河系的泥沙累加。
#[derive(Clone, Copy)]
struct SedimentIn<'a> {
    acc: &'a [sediment::WaterAcc],
    bif: Option<&'a sediment::BifWaterAcc>,
    /// 开堤防时的堤防几何（算堤内淹没面积）。
    levee: Option<&'a levee::Levee>,
}

/// 一个河系汇流后的泥沙累加（单元流域按 `RiverSystem::cells` 次序）。
struct SedimentOut {
    acc: Vec<sediment::WaterAcc>,
    bif: Option<sediment::BifWaterAcc>,
    /// 第一个越界（上游 `CoLM_stop`）。
    error: Option<anyhow::Error>,
}

/// 分汊子步里看到的水库：哪些单元流域是已建成的水库、它们的库容，以及要改写的出入流。
struct ReservoirView<'r> {
    built: &'r [Option<usize>],
    reservoirs: &'r [(usize, f64, [f64; 4])],
    q: &'r mut [(f64, f64)],
}

/// `DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT` 在分汊子步里要看的动量与流速。
struct MomentumLimit<'m> {
    on: bool,
    momen: &'m [f64],
    veloc: &'m [f64],
}

/// 一次汇流里分汊要带进带出的东西（全网一个河系，单元流域下标即全局序号）。
struct BifurcationRun<'a> {
    bif: &'a bifurcation::Bifurcation,
    state: BifurcationState,
    bifout: Vec<f64>,
    bifflw_lev: Vec<f64>,
    bifflw_acctime: Vec<f64>,
    /// 最近一个子步的 `bif_hflux_lev`（示踪物磁带要记它，`[p*levels + l]`）。
    layer_flux: Vec<f64>,
    /// 最近一个子步的 `bif_path_active`（泥沙的路径毛水量只累加活动路径）。
    layer_active: Vec<bool>,
}

// 阶段循环按单元流域下标写多组数组（与上游逐单元流域的 DO 循环一一对应），用下标更清楚。
#[allow(
    clippy::needless_range_loop,
    clippy::manual_clamp,
    clippy::too_many_arguments
)]
/// 一个河系把 `acctime` 秒推进完（`grid_riverlake_flow` 的 `DO WHILE` 循环限于这一河系）。
///
/// 开分汊时（`bif` 非空）`system` 是全网：每个子步先按河系求各自的子步长，再做普通出流限制
/// （`normal_outgoing_rate`/`ordinary_scale`），然后同步到全局最小子步长
/// （`sync_global_routing_dt`），算分汊通量并加进 `sum_hflux_riv`。
fn route_system<'a>(
    net: &RiverNetwork,
    system: &network::RiverSystem,
    state: &RiverState,
    history: &RiverHistory,
    levee: Option<&levee::Levee>,
    resv: Option<(&reservoir::Reservoir, i32)>,
    mut bif: Option<BifurcationRun<'a>>,
    momentum_limit: bool,
    acctime: f64,
    record: bool,
    sed_in: Option<SedimentIn<'_>>,
) -> SystemResult<'a> {
    let cells = &system.cells;
    let n = cells.len();
    let mut sed = sed_in.map(|sed_in| SedimentOut {
        acc: cells.iter().map(|&i| sed_in.acc[i]).collect(),
        bif: sed_in.bif.cloned(),
        error: None,
    });
    let sed_levee = sed_in.and_then(|sed_in| sed_in.levee);
    // 泥沙：每个子步首的总蓄量与堤内蓄量（`particle_water_storage_start`、`particle_protected_start`）。
    let mut sed_start = vec![(0.0, 0.0); if sed.is_some() { n } else { 0 }];
    let sed_note = |sed: &mut Option<SedimentOut>, result: Result<()>| {
        if let (Some(sed), Err(error)) = (sed.as_mut(), result) {
            sed.error.get_or_insert(error);
        }
    };
    // 开堤防或分汊时所有单元流域都以 `volwater_ucat` 为状态，而不是由水深反算
    // （上游 `IF (DEF_USE_BIFURCATION .or. DEF_USE_LEVEE)`）。
    let volume_state = levee.is_some() || bif.is_some();
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
    let mut flood_uc = vec![[0.0; 2]; n];
    let mut tape = record.then(|| tracer::SystemTape {
        cells: cells.clone(),
        pre_levee: vec![None; n],
        initial: Vec::with_capacity(n),
        substeps: Vec::new(),
        bif_hflux_lev: Vec::new(),
    });
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
    // 水库（`is_built_resv`）：本河系里这一年已建成的水库，与它们的库容、出入流与累加。
    // `built[k] = Some(j)` 指向 `reservoirs[j]`。
    let mut built = vec![None; n];
    let mut reservoirs: Vec<(usize, f64, [f64; 4])> = Vec::new();
    let mut qresv = Vec::<(f64, f64)>::new();
    if let Some((table, year)) = resv {
        let volresv = state.volresv.as_ref().expect("reservoir state");
        for (k, &i) in cells.iter().enumerate() {
            if let Some(r) = table.of_catchment[i] {
                if table.is_built(r, year) {
                    built[k] = Some(reservoirs.len());
                    let hist = |v: &Option<Vec<f64>>| v.as_ref().expect("reservoir history")[r];
                    reservoirs.push((
                        r,
                        volresv[r],
                        [
                            hist(&history.acctime_resv),
                            hist(&history.volresv),
                            hist(&history.qresv_in),
                            hist(&history.qresv_out),
                        ],
                    ));
                    qresv.push((0.0, 0.0));
                }
            }
        }
    }
    for (k, &i) in cells.iter().enumerate() {
        if let Some(j) = built[k] {
            // 水库里的水视为静止：`spval` 时由水深补库容，否则水深由库容反算；径流直接进库，
            // 水深不随之更新；`volwater_ucat` 不动。
            let (_, volresv, _) = &mut reservoirs[j];
            let w = if *volresv == colm_core::MISSING {
                *volresv = net.curves[i].volume(state.wdsrf[i]);
                state.wdsrf[i]
            } else {
                net.curves[i].depth(*volresv)
            };
            *volresv += state.acc_rnof[i];
            momen.push(0.0);
            volwater_ucat.push(state.volwater[i]);
            wdsrf.push(w);
            veloc.push(0.0);
            if let Some(tape) = tape.as_mut() {
                tape.initial.push(tracer::CellWater {
                    wdsrf: w,
                    volwater_ucat: state.volwater[i],
                    volresv: Some(*volresv),
                    levsto: lev.as_ref().map(|lev| lev[k].0),
                });
            }
            continue;
        }
        let mom = state.wdsrf[i] * state.veloc[i];
        let mut volwater = if volume_state {
            state.volwater[i]
        } else {
            net.curves[i].volume(state.wdsrf[i])
        } + state.acc_rnof[i];
        let w = match (levee, lev.as_mut()) {
            (Some(levee), Some(lev)) if levee.has[i] => {
                let (vis_bef, lev_bef) = (volwater, lev[k].0);
                let (visible, w) = repartition(&mut lev[k], i, volwater, &mut levee_floodarea[k]);
                volwater = visible;
                if let Some(tape) = tape.as_mut() {
                    tape.pre_levee[k] = Some([vis_bef, lev_bef, visible, lev[k].0]);
                }
                if let Some(out) = sed.as_mut() {
                    let result =
                        out.acc[k].levee_repartition([vis_bef, lev_bef, visible, lev[k].0]);
                    sed_note(&mut sed, result);
                }
                w
            }
            _ => net.curves[i].depth(volwater),
        };
        momen.push(mom);
        volwater_ucat.push(volwater);
        wdsrf.push(w);
        veloc.push(if w > RIVERMIN { mom / w } else { 0.0 });
        if let Some(tape) = tape.as_mut() {
            tape.initial.push(tracer::CellWater {
                wdsrf: w,
                volwater_ucat: volwater,
                volresv: None,
                levsto: lev.as_ref().map(|lev| lev[k].0),
            });
        }
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
    // 分汊：逐河系的子步长与这一子步的净分汊出流。
    let mut bif_sum = vec![0.0; if bif.is_some() { n } else { 0 }];
    // 堤防 + 分汊：第 2 层及以上（堤内一侧）的净分汊出流，以及堤内蓄量被扣穿时的报错。
    let mut bif_lev_sum = Vec::new();
    let mut protected_failed = false;
    // 普通情形（无已建成水库、堤防、分汊、泥沙、示踪物记录带）且单元数多：子步内各单元按单元并行。
    let plain = n >= PARALLEL_CELLS
        && built.iter().all(Option::is_none)
        && levee.is_none()
        && bif.is_none()
        && sed.is_none()
        && tape.is_none();
    while dt_res > 0.0 {
        if plain {
            dt_res -= plain_substep(
                net,
                system,
                dt_res,
                momentum_limit,
                PlainCells {
                    wdsrf: &mut wdsrf,
                    veloc: &mut veloc,
                    volwater: &mut volwater_ucat,
                    momen: &mut momen,
                    faces: &mut faces,
                    sums: &mut sums,
                    hist: &mut hist,
                    flood: &mut flood_uc,
                },
            );
            continue;
        }
        let mut dt_all = dt_res.min(60.0);
        // 所有河系的剩余时间相同（每个子步都同步成同一 `dt`），起始子步长也相同。
        let mut dt_sys = bif.is_some().then(|| vec![dt_all; net.river_systems]);
        for k in 0..n {
            // 已建成的水库没有出口面通量（`zgrad_dn` 也清零）。
            faces[k] = if built[k].is_some() {
                Face::default()
            } else {
                face_of(net, system, &wdsrf, &veloc, k, faces[k].zgrad_dn)
            };
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
        // 水库调度（`DEF_Reservoir_Method > 0`）：入流是净汇入，出流按调度方案；出流作为出口通量
        // 推给下游。洼地上的水库不放水。
        if let Some((table, _)) = resv {
            let mut resv_faces = vec![Face::default(); n];
            for k in 0..n {
                let Some(j) = built[k] else { continue };
                let (r, volresv, _) = reservoirs[j];
                let qin = -sums[k].0;
                if system.next[k] == INLAND_DEPRESSION {
                    qresv[j] = (qin, 0.0);
                    continue;
                }
                let qout = if volresv > 1.0e-4 * table.volume_total[r] {
                    table.operation(r, qin, volresv)
                } else {
                    0.0
                };
                qresv[j] = (qin, qout);
                faces[k].hflux = qout;
                faces[k].mflux = qout * (2.0 * GRAV * wdsrf[k]).sqrt();
                sums[k].0 += faces[k].hflux;
                sums[k].1 += faces[k].mflux;
                resv_faces[k].hflux = faces[k].hflux;
                resv_faces[k].mflux = faces[k].mflux;
            }
            for k in 0..n {
                let push = |value: fn(&Face) -> f64| {
                    let mut total = 0.0;
                    for &u in &system.upstream[k] {
                        let value = value(&resv_faces[u]);
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
                sums[k].0 -= push(|face| face.hflux);
                sums[k].1 -= push(|face| face.mflux);
            }
        }
        // 子步长：CFL 与蓄量两道限制，逐单元流域取最小。
        for k in 0..n {
            let i = cells[k];
            // 开分汊时每个河系从自己的子步长起算（`dt_this = dt_all(irivsys(i))`）。
            let mut dt_this = match dt_sys.as_ref() {
                Some(dt_sys) => dt_sys[net.river_system[i]],
                None => dt_all,
            };
            let (w, v) = (wdsrf[k], veloc[k]);
            if built[k].is_none() && (v != 0.0 || w > 0.0) {
                let wave = v.abs() + (w * GRAV).sqrt();
                dt_this = dt_this.min(net.rivlen[i] / wave * 0.8);
            }
            if sums[k].0 > 0.0 {
                let volwater = if let Some(j) = built[k] {
                    reservoirs[j].1
                } else if volume_state {
                    volwater_ucat[k]
                } else {
                    net.curves[i].volume(w)
                };
                dt_this = dt_this.min(volwater / sums[k].0);
            }
            // 动量限制：流速超过 0.1 m/s 且动量收支要把它推向反向时，子步长不超过让动量归零的时间。
            if momentum_limit && built[k].is_none() {
                let gradient = sums[k].1 - sums[k].2;
                if v.abs() > 0.1 && v * gradient > 0.0 {
                    dt_this = dt_this.min(((momen[k] * net.rivare[i]) / gradient).abs());
                }
            }
            dt_all = dt_this.min(dt_all);
            if let Some(dt_sys) = dt_sys.as_mut() {
                let s = net.river_system[i];
                dt_sys[s] = dt_this.min(dt_sys[s]);
            }
        }
        let dt = match (bif.as_mut(), dt_sys.as_mut()) {
            (Some(run), Some(dt_sys)) => bifurcation_substep(
                net,
                system,
                run,
                dt_sys,
                dt_res,
                &wdsrf,
                &volwater_ucat,
                &mut faces,
                &mut sums,
                &mut bif_sum,
                ReservoirView {
                    built: &built,
                    reservoirs: &reservoirs,
                    q: &mut qresv,
                },
                MomentumLimit {
                    on: momentum_limit,
                    momen: &momen,
                    veloc: &veloc,
                },
                lev.as_ref().map(|lev| {
                    (
                        levee.expect("levee"),
                        lev.iter().map(|l| l.0).collect::<Vec<_>>(),
                        lev.iter().map(|l| l.1).collect::<Vec<_>>(),
                    )
                }),
                &mut bif_lev_sum,
            ),
            _ => dt_all,
        };
        // `tracer_lifecycle_route_sediment_bif`：每条活动路径逐层限流后的毛水量。
        if let (Some(out), Some(run)) = (sed.as_mut(), bif.as_ref()) {
            if let Some(acc) = out.bif.as_mut() {
                let result = acc.add(dt, &run.layer_flux, &run.layer_active);
                sed_note(&mut sed, result);
            }
        }
        let mut cells_tape = tape.as_ref().map(|_| {
            (0..n)
                .map(|k| tracer::CellSubstep {
                    start: cell_water(k, &wdsrf, &volwater_ucat, &lev, &built, &reservoirs),
                    hflux: faces[k].hflux,
                    sum_hflux: sums[k].0,
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        });
        // 蓄量、水深与动量。
        for k in 0..n {
            let i = cells[k];
            let curve = &net.curves[i];
            let (sum_h, sum_m, sum_z) = sums[k];
            // `volwater = FNMA(visible_hflux, dt, volwater)`。
            let start = if let Some(j) = built[k] {
                reservoirs[j].1
            } else if volume_state {
                volwater_ucat[k]
            } else {
                curve.volume(wdsrf[k])
            };
            if !sed_start.is_empty() {
                let protected = match (sed_levee, lev.as_ref()) {
                    (Some(levee), Some(lev)) if levee.has[i] && built[k].is_none() => {
                        lev[k].0.max(0.0)
                    }
                    _ => 0.0,
                };
                sed_start[k] = (start.max(0.0) + protected, protected);
            }
            // 堤防 + 分汊：有堤单元流域的第 2 层及以上分汊通量走堤内一侧。
            let leveed_bif = !bif_lev_sum.is_empty()
                && levee.is_some_and(|levee| levee.has[i])
                && built[k].is_none();
            let (visible_hflux, protected_hflux) = if leveed_bif {
                (sum_h - bif_lev_sum[k], bif_lev_sum[k])
            } else {
                (sum_h, 0.0)
            };
            let mut volwater = (-visible_hflux).mul_add(dt, start);
            // `levee_apply_protected_flux`：堤内蓄量扣掉堤内一侧的分汊出流（不收缩，乘积与报错
            // 判断共用），扣穿超过容差就报错。无分汊时通量为 0，蓄量不变；它重算的 `levdph`
            // 随即被下面的重新分区覆盖。
            if leveed_bif {
                if let Some(lev) = lev.as_mut() {
                    let levsto = lev[k].0;
                    let taken = dt * protected_hflux;
                    let mut raw = levsto - taken;
                    if raw < 0.0 {
                        let tol = levsto.abs().max(taken.abs().max(1.0)) * 1.0e-10;
                        if -raw <= tol {
                            raw = 0.0;
                        } else {
                            protected_failed = true;
                        }
                    }
                    lev[k].0 = raw.max(0.0);
                }
            }
            volwater = volwater.max(0.0);
            if system.next[k] == INLAND_DEPRESSION && volwater > net.rivstomax[i] {
                if let Some(cells_tape) = cells_tape.as_mut() {
                    cells_tape[k].overflow = Some((volwater, net.rivstomax[i]));
                }
                faces[k].hflux = (volwater - net.rivstomax[i]) / dt;
                if let Some(j) = built[k] {
                    qresv[j].1 = faces[k].hflux;
                }
                volwater = net.rivstomax[i];
            }
            if let Some(j) = built[k] {
                // 水库：水深由库容反算，库容写回 `volresv`，动量与流速为 0。
                wdsrf[k] = curve.depth(volwater);
                reservoirs[j].1 = volwater;
                momen[k] = 0.0;
                veloc[k] = 0.0;
                continue;
            }
            let w = match (levee, lev.as_mut()) {
                (Some(levee), Some(lev)) if levee.has[i] => {
                    let (vis_bef, lev_bef) = (volwater, lev[k].0);
                    let (visible, w) =
                        repartition(&mut lev[k], i, volwater, &mut levee_floodarea[k]);
                    volwater = visible;
                    if let Some(cells_tape) = cells_tape.as_mut() {
                        cells_tape[k].levee = Some([vis_bef, lev_bef, visible, lev[k].0]);
                    }
                    if let Some(out) = sed.as_mut() {
                        let result =
                            out.acc[k].levee_repartition([vis_bef, lev_bef, visible, lev[k].0]);
                        sed_note(&mut sed, result);
                    }
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
            // 截断后的流速与动量重新对齐（原来只在开示踪物时做，vendor 已解耦成无条件）。
            if w >= RIVERMIN {
                momen[k] = veloc[k] * w;
            }
        }
        if let (Some(tape), Some(mut cells_tape)) = (tape.as_mut(), cells_tape.take()) {
            for k in 0..n {
                cells_tape[k].end =
                    cell_water(k, &wdsrf, &volwater_ucat, &lev, &built, &reservoirs);
            }
            tape.substeps.push((dt, cells_tape));
            if let Some(run) = bif.as_ref() {
                tape.bif_hflux_lev.push(run.layer_flux.clone());
            }
        }
        // history 累加：`a_x = FMA(x, dt, a_x)`，`acctime` 平铺相加。
        for k in 0..n {
            let i = cells[k];
            let curve = &net.curves[i];
            let w = wdsrf[k];
            let volwater = if let Some(j) = built[k] {
                reservoirs[j].1
            } else if volume_state {
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
            // `DEF_USE_TRACER`：`total_floodarea/total_flooddepth`（`:1200-1207`），留最后一个子步的值。
            let above_bank = (w - curve.rivhgt).max(0.0);
            flood_uc[k] = [
                floodarea,
                match lev.as_ref() {
                    Some(lev) if levee_floodarea[k] > 0.0 => lev[k].1.max(above_bank),
                    _ => above_bank,
                },
            ];
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
            if let Some(run) = bif.as_mut() {
                run.bifout[k] = bif_sum[k].mul_add(dt, run.bifout[k]);
            }
            if let Some(j) = built[k] {
                let (_, volresv, a) = &mut reservoirs[j];
                let (qin, qout) = qresv[j];
                a[0] += dt;
                a[1] = volresv.mul_add(dt, a[1]);
                a[2] = qin.mul_add(dt, a[2]);
                a[3] = qout.mul_add(dt, a[3]);
            }
            // `tracer_lifecycle_route_diag_accumulate`：子步末的蓄量（水库取 `volresv`）、
            // 洼地溢流之后的出口通量与本子步的淹没面积。
            if let Some(out) = sed.as_mut() {
                let leveed = sed_levee.filter(|levee| levee.has[i] && built[k].is_none());
                let (end, protected_end) = match (built[k], leveed, lev.as_ref()) {
                    (Some(j), _, _) if reservoirs[j].1 != colm_core::MISSING => {
                        (reservoirs[j].1.max(0.0), 0.0)
                    }
                    (Some(_), _, _) => (volwater_ucat[k].max(0.0), 0.0),
                    (None, Some(_), Some(lev)) => {
                        let protected = lev[k].0.max(0.0);
                        (protected + volwater_ucat[k].max(0.0), protected)
                    }
                    _ => (volwater_ucat[k].max(0.0), 0.0),
                };
                // `min(max(FNMA(levee_frc, area, levee_floodarea), 0), area*(1-levee_frc))`。
                let protected_area = sed_levee.map(|levee| match leveed {
                    Some(_) => (-levee.frc[i])
                        .mul_add(net.area[i], levee_floodarea[k])
                        .max(0.0)
                        .min(net.area[i] * (1.0 - levee.frc[i])),
                    None => 0.0,
                });
                let water = sediment::SubstepWater {
                    start: sed_start[k].0,
                    end,
                    protected_start: sed_start[k].1,
                    protected_end,
                    protected_area,
                };
                let result = out.acc[k].add(dt, veloc[k], w, water, faces[k].hflux, floodarea);
                sed_note(&mut sed, result);
            }
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
        bifurcation: bif,
        reservoirs,
        protected_failed,
        tape,
        sediment: sed,
        flood: flood_uc
            .into_iter()
            .zip(&levee_floodarea)
            .map(|([area, depth], &levee)| [area, depth, levee])
            .collect(),
    }
}

/// 开分汊时一个子步在求出逐河系子步长之后的那一段（`grid_riverlake_flow` 的
/// `IF (DEF_USE_BIFURCATION)` 分支）：普通出流限制、全局子步长同步、分汊通量。返回同步后的子步长，
/// 并把分汊净出流加进 `sums`、记进 `bif_sum`。单元流域下标即全局序号。
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn bifurcation_substep(
    net: &RiverNetwork,
    system: &network::RiverSystem,
    run: &mut BifurcationRun<'_>,
    dt_sys: &mut [f64],
    dt_res: f64,
    wdsrf: &[f64],
    volwater: &[f64],
    faces: &mut [Face],
    sums: &mut [(f64, f64, f64)],
    bif_sum: &mut [f64],
    resv: ReservoirView<'_>,
    momentum: MomentumLimit<'_>,
    levee: Option<(&levee::Levee, Vec<f64>, Vec<f64>)>,
    lev_sum: &mut Vec<f64>,
) -> f64 {
    let n = faces.len();
    // 已建成的水库用库容代替河道蓄量。
    let volume = |k: usize| match resv.built[k] {
        Some(j) => resv.reservoirs[j].1,
        None => volwater[k],
    };
    let push = |faces: &[Face], k: usize, value: fn(&Face) -> f64| {
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
    // 普通出流：顺流时是自己的出口通量，逆流时由下游计（推到下游求和）。
    let mut normal = vec![0.0; n];
    let mut reverse = vec![Face::default(); n];
    for k in 0..n {
        let h = faces[k].hflux;
        if h >= 0.0 {
            normal[k] = h;
        } else {
            reverse[k].hflux = -h;
        }
    }
    for k in 0..n {
        normal[k] += push(&reverse, k, |face| face.hflux);
    }
    // `ordinary_scale`：本河系子步长内出流不超过蓄量。
    let mut scale = vec![1.0; n];
    for k in 0..n {
        if normal[k] <= 0.0 {
            continue;
        }
        let dt = dt_sys[net.river_system[k]];
        scale[k] = if !dt.is_finite() || dt <= 0.0 {
            0.0
        } else {
            (volume(k).max(0.0) / (normal[k] * dt)).min(1.0)
        };
        normal[k] *= scale[k];
    }
    for k in 0..n {
        let factor = if faces[k].hflux >= 0.0 {
            scale[k]
        } else {
            let next = system.next[k];
            if next >= 0 {
                scale[next as usize]
            } else {
                1.0
            }
        };
        faces[k].hflux *= factor;
        faces[k].mflux *= factor;
        if let Some(j) = resv.built[k] {
            resv.q[j].1 = faces[k].hflux;
        }
    }
    for k in 0..n {
        let inflow = push(faces, k, |face| face.hflux);
        sums[k].0 = faces[k].hflux - inflow;
        sums[k].1 = faces[k].mflux - push(faces, k, |face| face.mflux);
        // 水库入流取缩放后上游推来的出口通量（`qresv_in = hflux_sumups`）。
        if let Some(j) = resv.built[k] {
            resv.q[j].0 = inflow;
        }
    }
    // 缩放后再做一次动量限制（同步全局子步长之前）。
    if momentum.on {
        for k in 0..n {
            if resv.built[k].is_some() {
                continue;
            }
            let v = momentum.veloc[k];
            let gradient = sums[k].1 - sums[k].2;
            if v.abs() > 0.1 && v * gradient > 0.0 {
                let limit = ((momentum.momen[k] * net.rivare[k]) / gradient).abs();
                let s = net.river_system[k];
                dt_sys[s] = dt_sys[s].min(limit);
            }
        }
    }
    // `sync_global_routing_dt`：无效的河系子步长退回 `min(10, dt_res)`，再取全局最小。
    for dt in dt_sys.iter_mut() {
        if !dt.is_finite() || *dt <= 0.0 {
            *dt = 10.0f64.min(dt_res);
        }
    }
    let dt = dt_sys.iter().fold(f64::INFINITY, |a, &b| a.min(b));
    let reservoir_volume: Vec<Option<f64>> = resv
        .built
        .iter()
        .map(|b| b.map(|j| resv.reservoirs[j].1))
        .collect();
    let flux = run.bif.calc(
        net,
        wdsrf,
        &run.state.wdsrf_prev,
        volwater,
        dt,
        &normal,
        &reservoir_volume,
        levee
            .as_ref()
            .map(|(levee, levsto, levdph)| bifurcation::BifurcationLevee {
                levee,
                levsto,
                levdph,
            }),
        &mut run.state.veloc,
        &mut run.state.momen,
    );
    run.state.wdsrf_prev.copy_from_slice(wdsrf);
    let levels = run.bif.levels;
    for (p, &active) in flux.active.iter().enumerate() {
        if !active {
            continue;
        }
        for k in p * levels..(p + 1) * levels {
            run.bifflw_lev[k] = flux.hflux_lev[k].mul_add(dt, run.bifflw_lev[k]);
        }
        run.bifflw_acctime[p] += dt;
    }
    for k in 0..n {
        sums[k].0 += flux.hflux_sum[k];
    }
    bif_sum.copy_from_slice(&flux.hflux_sum);
    *lev_sum = flux.lev_hflux_sum;
    run.layer_flux = flux.hflux_lev;
    run.layer_active = flux.active;
    dt
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

/// 单元数不少于这个数的河系在子步内按单元并行（再小线程开销就超过收益；结果与串行逐位相同）。
const PARALLEL_CELLS: usize = 4096;

/// [`plain_substep`] 改写的逐单元数组（与 `route_system` 里同名的量一一对应）。
struct PlainCells<'a> {
    wdsrf: &'a mut [f64],
    veloc: &'a mut [f64],
    volwater: &'a mut [f64],
    momen: &'a mut [f64],
    faces: &'a mut [Face],
    sums: &'a mut [(f64, f64, f64)],
    hist: &'a mut [[f64; 10]],
    flood: &'a mut [[f64; 2]],
}

/// `route_system` 一个子步的普通情形（无已建成水库、堤防、分汊、泥沙、示踪物记录带），按单元并行。
///
/// 每一段都与串行代码里同一情形的表达式逐式相同：出口通量只读本单元与下游的旧状态；上游求和按
/// `system.upstream[k]` 的原次序；子步长先逐单元算出限制，再按单元次序串行取最小（取最小与次序
/// 无关）；蓄量、水深、动量与历史累加都只写本单元。所以结果与串行逐位相同。返回本子步长。
// `min(20).max(-20)` 照抄串行代码：`clamp` 遇 NaN 的行为不同。
#[allow(clippy::manual_clamp)]
fn plain_substep(
    net: &RiverNetwork,
    system: &network::RiverSystem,
    dt_res: f64,
    momentum_limit: bool,
    cells: PlainCells<'_>,
) -> f64 {
    let PlainCells {
        wdsrf,
        veloc,
        volwater,
        momen,
        faces,
        sums,
        hist,
        flood,
    } = cells;
    let ids = &system.cells;
    let new_faces: Vec<Face> = (0..ids.len())
        .into_par_iter()
        .map(|k| face_of(net, system, wdsrf, veloc, k, faces[k].zgrad_dn))
        .collect();
    faces.copy_from_slice(&new_faces);
    // `push_ups2ucat`（sum，权重 1）：跳过 0 值，首项直接赋值。
    sums.par_iter_mut().enumerate().for_each(|(k, sum)| {
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
        *sum = (
            face.sum_hflux - push(|face| face.hflux),
            face.sum_mflux - push(|face| face.mflux),
            face.sum_zgrad - push(|face| face.zgrad_dn),
        );
    });
    // 子步长：CFL 与蓄量两道限制，逐单元流域取最小。
    let dt_start = dt_res.min(60.0);
    let limits: Vec<f64> = (0..ids.len())
        .into_par_iter()
        .map(|k| {
            let i = ids[k];
            let mut dt_this = dt_start;
            let (w, v) = (wdsrf[k], veloc[k]);
            if v != 0.0 || w > 0.0 {
                let wave = v.abs() + (w * GRAV).sqrt();
                dt_this = dt_this.min(net.rivlen[i] / wave * 0.8);
            }
            if sums[k].0 > 0.0 {
                dt_this = dt_this.min(net.curves[i].volume(w) / sums[k].0);
            }
            if momentum_limit {
                let gradient = sums[k].1 - sums[k].2;
                if v.abs() > 0.1 && v * gradient > 0.0 {
                    dt_this = dt_this.min(((momen[k] * net.rivare[i]) / gradient).abs());
                }
            }
            dt_this
        })
        .collect();
    let mut dt = dt_start;
    for dt_this in limits {
        dt = dt_this.min(dt);
    }
    // 蓄量、水深与动量。
    wdsrf
        .par_iter_mut()
        .zip(volwater.par_iter_mut())
        .zip(momen.par_iter_mut())
        .zip(veloc.par_iter_mut())
        .zip(faces.par_iter_mut())
        .enumerate()
        .for_each(|(k, ((((wdsrf, volwater_k), momen), veloc), face))| {
            let i = ids[k];
            let curve = &net.curves[i];
            let (sum_h, sum_m, sum_z) = sums[k];
            let start = curve.volume(*wdsrf);
            let mut volume = (-sum_h).mul_add(dt, start);
            volume = volume.max(0.0);
            if system.next[k] == INLAND_DEPRESSION && volume > net.rivstomax[i] {
                face.hflux = (volume - net.rivstomax[i]) / dt;
                volume = net.rivstomax[i];
            }
            let w = curve.depth(volume);
            *wdsrf = w;
            *volwater_k = volume;
            if w >= RIVERMIN {
                let manning = net.rivman[i];
                let friction = (manning * manning * GRAV / w.lpow(7.0 / 3.0)) * momen.abs();
                let gradient = (sum_m - sum_z) / net.rivare[i];
                *momen = (-gradient).mul_add(dt, *momen) / dt.mul_add(friction, 1.0);
                *veloc = *momen / w;
            } else {
                *momen = 0.0;
                *veloc = 0.0;
            }
            if system.next[k] == INLAND_DEPRESSION {
                *momen = momen.min(0.0);
                *veloc = veloc.min(0.0);
            }
            *veloc = veloc.min(20.0).max(-20.0);
            if w >= RIVERMIN {
                *momen = *veloc * w;
            }
        });
    // history 累加：`a_x = FMA(x, dt, a_x)`，`acctime` 平铺相加。
    hist.par_iter_mut()
        .zip(flood.par_iter_mut())
        .enumerate()
        .for_each(|(k, (a, flood))| {
            let i = ids[k];
            let curve = &net.curves[i];
            let w = wdsrf[k];
            let volume = curve.volume(w);
            let rivsto = volume.min(curve.rivstomax);
            let floodarea = curve.floodarea(w);
            let above_bank = (w - curve.rivhgt).max(0.0);
            *flood = [floodarea, above_bank];
            a[0] += dt;
            a[1] = w.mul_add(dt, a[1]);
            a[2] = veloc[k].mul_add(dt, a[2]);
            // `a_discharge + hflux_fc*dt` 不融合：乘积与调试用的 `totaldis` 共用（`_7820`）。
            a[3] += faces[k].hflux * dt;
            a[4] = floodarea.mul_add(dt, a[4]);
            a[5] = rivsto.mul_add(dt, a[5]);
            a[6] = (volume - rivsto).mul_add(dt, a[6]);
            a[7] = (w - curve.rivhgt).max(0.0).mul_add(dt, a[7]);
            a[8] = volume.mul_add(dt, a[8]);
            a[9] = (net.rivelv[i] + w).mul_add(dt, a[9]);
        });
    dt
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
