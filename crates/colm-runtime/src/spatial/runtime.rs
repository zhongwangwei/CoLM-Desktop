//! 空间主循环（`CoLM.F90` 的时间步 + `CoLMDRIVER` 的 patch 循环）。
//!
//! 与单点共用同一个时钟（[`RuntimeClock`]）与同一个 patch 入口（`advance_patch`）；不同的只有
//! 每步的强迫：先在强迫格上算好、逐量映射到每个 patch（[`super::forcing`]），再按 patch 自己的
//! `patchlonr`/`patchlatr` 算天顶角与方位角。空间构建强制格林尼治时间（`initimetype`）。

use anyhow::{ensure, Context, Result};
use colm_core::{
    orbital_calendar_day, orbital_cosine_azimuth, orbital_cosine_zenith, Co2Scenario, RuntimeClock,
    RuntimeForcing, ShortwaveForcing, StandardLctSnowSoilState,
};
use rayon::prelude::*;

use super::forcing::{map_to_patches, GriddedForcing, PatchForcing};
use super::mapping::AreaWeightedMapping;
use crate::assembly::{StandardLctRestartTemplate, StandardLctStepBinding};
use crate::{PatchOutput, PatchStepOutput, PointRuntimeStep};

/// 空间主循环的运行态。
pub struct SpatialRuntime {
    clock: RuntimeClock,
    forcing: GriddedForcing,
    mapping: AreaWeightedMapping,
    /// 每个 patch 的 `(patchlonr, patchlatr)`（弧度，常数重启）。
    coordinates: Vec<(f64, f64)>,
    co2_scenario: Co2Scenario,
    baseflow_optimizer: Option<crate::baseflow_optimizer::BaseflowOptimizer>,
    /// 网格河湖汇流与每个 patch 是否进入 `filter_rnof`（`patchtype < 99 .and. patchmask`）。
    river: Option<(crate::river::RiverModel, Vec<bool>)>,
    /// LULCC 年末那一步的终点：这一步不重读 LAI（见 [`SpatialRuntime::defer_lai_refresh_at`]）。
    deferred_lai_refresh: Option<colm_core::CalendarTime>,
    /// 网格示踪物强迫（`read_tracer_forcing`）。
    tracer_forcing: Option<super::tracer_forcing::GriddedTracerForcing>,
    /// `forcmask_pch`：足迹不全落在缺测格上的 patch 为真。为假的 patch 整步跳过
    /// （`CoLMDRIVER.F90:78` 的 `CYCLE`），状态不动、不出通量。
    forcing_mask: Vec<bool>,
    /// `DEF_USE_Forcing_Downscaling(_Simple)`：强迫按地形降到 patch（[`super::downscaling`]）。
    downscaling: Option<super::downscaling::SpatialDownscaling>,
    /// `CatchLateralFlow`（CATCHMENT 内核）：每步陆面之后的流域侧向流。
    catchment: Option<crate::catchment::runtime::CatchmentRuntime>,
}

impl SpatialRuntime {
    pub fn new(
        clock: RuntimeClock,
        forcing: GriddedForcing,
        mapping: AreaWeightedMapping,
        coordinates: Vec<(f64, f64)>,
        co2_scenario: Co2Scenario,
        forcing_mask: Vec<bool>,
    ) -> Result<Self> {
        ensure!(
            mapping.parts.len() == coordinates.len() && forcing_mask.len() == coordinates.len(),
            "the forcing mapping covers {} patches and the forcing mask {}, but {} coordinates \
             were given",
            mapping.parts.len(),
            forcing_mask.len(),
            coordinates.len()
        );
        // 被遮蔽的 patch（足迹全是缺测格）面积清零是预期的；其余必须与强迫网格有重叠。
        ensure!(
            mapping
                .area
                .iter()
                .zip(&forcing_mask)
                .all(|(&area, &active)| area > 0.0 || !active),
            "a patch has no overlap with the forcing grid"
        );
        Ok(Self {
            clock,
            forcing,
            mapping,
            coordinates,
            co2_scenario,
            baseflow_optimizer: None,
            river: None,
            deferred_lai_refresh: None,
            tracer_forcing: None,
            forcing_mask,
            downscaling: None,
            catchment: None,
        })
    }

    /// 接上流域侧向流（`CatchLateralFlow`）。与网格河湖汇流互斥，且不支持强迫缺测遮蔽。
    pub fn with_catchment(
        mut self,
        catchment: crate::catchment::runtime::CatchmentRuntime,
    ) -> Result<Self> {
        ensure!(
            self.river.is_none(),
            "catchment lateral flow and grid river-lake flow are exclusive builds"
        );
        ensure!(
            self.forcing_mask.iter().all(|&active| active),
            "catchment lateral flow with patches masked by missing forcing is not ported"
        );
        self.catchment = Some(catchment);
        Ok(self)
    }

    /// 当前的流域侧向流（写续跑用）。
    pub fn catchment(&self) -> Option<&crate::catchment::runtime::CatchmentRuntime> {
        self.catchment.as_ref()
    }

    /// 接上强迫降尺度（`patches` 与映射的 set 一一对应）。
    pub fn with_downscaling(
        mut self,
        settings: super::downscaling::DownscalingSettings,
        patches: Vec<super::downscaling::DownscalingPatch>,
    ) -> Result<Self> {
        self.downscaling = Some(super::downscaling::SpatialDownscaling::new(
            settings,
            patches,
            &self.mapping,
        )?);
        Ok(self)
    }

    /// 接上网格示踪物强迫（只在配置了示踪物强迫变量时）。
    pub fn with_tracer_forcing(
        mut self,
        forcing: super::tracer_forcing::GriddedTracerForcing,
    ) -> Self {
        self.tracer_forcing = Some(forcing);
        self
    }

    /// 网格示踪物强迫的运行态（写续跑用）。
    pub fn tracer_forcing(&self) -> Option<&super::tracer_forcing::GriddedTracerForcing> {
        self.tracer_forcing.as_ref()
    }

    /// 主强迫配置（示踪物强迫的重启指纹要用）。
    pub fn forcing_config(&self) -> &super::forcing::GriddedForcingConfig {
        self.forcing.config()
    }

    /// 当前的河道模型（写续跑用）。
    pub fn river(&self) -> Option<&crate::river::RiverModel> {
        self.river.as_ref().map(|(river, _)| river)
    }

    /// 接上网格河湖汇流（`GridRiverLakeFlow`）。`included` 是 `filter_rnof`。
    pub fn with_river(
        mut self,
        river: crate::river::RiverModel,
        included: Vec<bool>,
    ) -> Result<Self> {
        ensure!(
            included.len() == self.coordinates.len(),
            "the river runoff filter needs one entry per patch"
        );
        self.river = Some((river, included));
        Ok(self)
    }

    #[must_use]
    pub fn with_baseflow_optimizer(
        mut self,
        optimizer: crate::baseflow_optimizer::BaseflowOptimizer,
    ) -> Self {
        self.baseflow_optimizer = Some(optimizer);
        self
    }

    /// 段末取出优化器（LULCC 换年时交给下一段）。
    pub fn take_baseflow_optimizer(
        &mut self,
    ) -> Option<crate::baseflow_optimizer::BaseflowOptimizer> {
        self.baseflow_optimizer.take()
    }

    /// LULCC 年末（`CoLM.F90`）：`LAI_readin` 排在 `LulccDriver` 之后，读的是新一年的 patch。
    /// 旧 patch 在这一步不重读 LAI；新 patch 的 LAI 由新年份的冷启动读进来。
    pub fn defer_lai_refresh_at(mut self, end_time: colm_core::CalendarTime) -> Self {
        self.deferred_lai_refresh = Some(end_time);
        self
    }

    /// 观测高度 `forc_hgt_u/t/q` 也要过 `grid2pset`：映射后的值与 namelist 常数差若干 ULP，
    /// 所以每个 patch 的模板用自己映射得到的那个值。
    pub fn apply_mapped_heights(&self, templates: &mut [StandardLctRestartTemplate]) -> Result<()> {
        let heights = self.forcing.mapped_heights(&self.mapping);
        ensure!(
            heights.len() == templates.len(),
            "one template per mapped patch is needed"
        );
        for (template, (wind, temperature, humidity)) in templates.iter_mut().zip(heights) {
            template.physics.wind_height_m = wind;
            template.physics.temperature_height_m = temperature;
            template.physics.humidity_height_m = humidity;
        }
        Ok(())
    }

    /// 积雪入口：每步推进全部 patch；`history` 给了就先逐 patch 累加 history（`hist_out`），
    /// 再刷新 LAI、跑基流优化，最后交给回调。
    pub fn run<F>(
        &mut self,
        templates: &[StandardLctRestartTemplate],
        states: &mut Vec<StandardLctSnowSoilState>,
        mut history: Option<&mut super::history::SpatialHistory>,
        mut on_step: F,
    ) -> Result<usize>
    where
        F: FnMut(
            &[PointRuntimeStep],
            &[StandardLctSnowSoilState],
            &[Option<PatchStepOutput<'_>>],
            Option<&crate::river::RiverModel>,
            Option<&crate::tracer::ForcingCache<'_>>,
            Option<&crate::catchment::runtime::CatchmentRuntime>,
        ) -> Result<()>,
    {
        ensure!(
            templates.len() == states.len() && templates.len() == self.coordinates.len(),
            "every patch needs one template, one state and one coordinate"
        );
        let time_step_seconds = self.clock.timestep_seconds();
        let masked = self.forcing_mask.iter().any(|&active| !active);
        // 上游 `BaseFlow_Optimize` 不看 `forcmask_pch`，会用被遮蔽 patch 从未赋值的通量。
        ensure!(
            !(masked && self.baseflow_optimizer.is_some()),
            "baseflow optimization with patches masked by missing forcing has no defined upstream \
             behaviour (BaseFlow_Optimize reads their unassigned fluxes)"
        );
        let mut optimizer = self.baseflow_optimizer.take();
        let mut completed = 0;
        let mut spinup_cycle = self
            .clock
            .clone()
            .next_step()
            .map_or(1, |step| step.spinup_cycle);
        let mut timer = PhaseTimer::from_env();
        let result = (|| -> Result<()> {
            loop {
                let mut next_clock = self.clock.clone();
                let Some(clock) = next_clock.next_step() else {
                    return Ok(());
                };
                timer.mark("other");
                let (month, _) = colm_core::month_day(clock.forcing_time)?;
                let co2 =
                    colm_core::monthly_co2_ppm(self.co2_scenario, clock.forcing_time.year, month)?
                        * 1.0e-6;
                // 上一步把时钟回卷到了起点（`CoLM.F90:711-724`）：`forcing_reset` 与 `tracer_forcing_reset`。
                if clock.spinup_cycle > spinup_cycle {
                    self.forcing.reset();
                    if let Some(tracer_forcing) = self.tracer_forcing.as_mut() {
                        tracer_forcing.reset();
                    }
                }
                spinup_cycle = clock.spinup_cycle;
                self.forcing.set_spinup(clock.is_spinup);
                let cells = self.forcing.step(clock.forcing_time, co2)?;
                let calendar_day = orbital_calendar_day(clock.forcing_time, true, 0.0)?;
                let patch_forcing = match &self.downscaling {
                    // 降尺度用的是步首（上一步末）的 `alb`。
                    Some(downscaling) => downscaling.map_to_patches(
                        &self.mapping,
                        &self.forcing,
                        &cells,
                        states,
                        &self.coordinates,
                        calendar_day,
                    )?,
                    None => map_to_patches(&self.mapping, &self.forcing, &cells),
                };
                // `read_tracer_forcing (jdate, dir_forcing)`：紧跟 `read_forcing`。
                if let Some(tracer_forcing) = self.tracer_forcing.as_mut() {
                    tracer_forcing.step(clock.forcing_time, &self.forcing, &self.mapping)?;
                }
                let surface_calendar_day = orbital_calendar_day(clock.end_time, true, 0.0)?;
                let seconds_of_day = crate::seconds_of_day(clock.end_time)?;
                let steps = patch_forcing
                    .iter()
                    .zip(&self.coordinates)
                    .map(|(forcing, &(lon, lat))| {
                        patch_step(
                            clock,
                            *forcing,
                            lon,
                            lat,
                            calendar_day,
                            surface_calendar_day,
                        )
                    })
                    .collect::<Vec<_>>();
                timer.mark("forcing");
                let mut next_states = states.clone();
                timer.mark("state clone");
                // patch 之间在一步之内互不依赖：每个只读本步强迫与共享的只读数据、只改自己的状态，
                // 所以用 rayon 并行推进（`RAYON_NUM_THREADS` 控制线程数，相当于 OpenMP）。`collect`
                // 保持 patch 次序，之后的汇流、漫滩交换与历史累加仍按原次序串行，结果逐位不变。
                // 示踪物收支追踪器只记计数与最坏值；最坏值恰好相等时报告里写哪个 patch 可能随调度
                // 变化，只影响诊断文字。
                let forcing_mask = &self.forcing_mask;
                let coordinates = &self.coordinates;
                let river = self.river.as_ref().map(|(river, _)| river);
                let tracer_forcing = self.tracer_forcing.as_ref();
                let scales = optimizer.as_ref();
                let initial_totals = templates
                    .iter()
                    .zip(states.iter())
                    .map(|(template, state)| crate::initial_total_water_mm(template, state))
                    .collect::<Vec<_>>();
                // 被遮蔽的 patch 没有输出（`None`）。
                let outputs: Vec<Option<PatchOutput>> = templates
                    .par_iter()
                    .zip(next_states.par_iter_mut())
                    .zip(steps.par_iter())
                    .enumerate()
                    .map(
                        |(index, ((template, state), step))| -> Result<Option<PatchOutput>> {
                            if !forcing_mask[index] {
                                // `update_ozone_data` 的 `grid2pset` 同样对整列 patch 做。
                                template.update_ozone(step.clock.forcing_time, state)?;
                                // `CoLM.F90:495-541` 的 BGC 数据更新（硝化 O2、闪电、氮沉降、人口密度）对整列
                                // patch 做，被遮蔽的也一样：`update_lightning_data` 的 `grid2pset` 会把
                                // `lnfm` 从分配时的 `spval` 换成数据值（第 541 轮）。
                                if let Some(bgc) = &template.bgc {
                                    let end = step.clock.end_time;
                                    let idate = [
                                        end.year,
                                        i32::from(end.julian_day),
                                        i32::try_from(end.seconds)?,
                                    ];
                                    bgc.update_non_soil(step.clock.forcing_time, idate, state)?;
                                }
                                return Ok(None);
                            }
                            let binding = StandardLctStepBinding {
                                forcing: step.forcing,
                                seconds_of_day,
                                greenwich_time: true,
                                longitude_radians: coordinates[index].0,
                                co2_volume_fraction: co2,
                                partial_pressures_pa: Some((
                                    patch_forcing[index].pco2m,
                                    patch_forcing[index].po2m,
                                )),
                                // 漫滩回馈只作用在土壤 patch 上（`patchtype == 0`）。
                                flood: river
                                    .and_then(|river| river.flood.as_ref())
                                    .filter(|_| template.patch_type == 0)
                                    .map(|flood| colm_core::flood_evaporation::FloodPatchInput {
                                        depth_mm: flood.depth_mm[index],
                                        fraction: flood.fraction[index],
                                        infiltration_max_mm_day: flood.infiltration_max_mm_day,
                                    }),
                                tracer_ratios: tracer_forcing.map(|forcing| forcing.ratios(index)),
                                flood_tracer: river
                                    .and_then(|river| river.flood.as_ref())
                                    .and_then(|flood| {
                                        flood.tracer.as_ref().map(|tracer| {
                                            (
                                                flood.credit[index] * 1000.0,
                                                tracer.credit_patch[index].as_slice(),
                                            )
                                        })
                                    }),
                            };
                            let scale = scales.map(|optimizer| optimizer.scale(index));
                            Ok(Some(
                                crate::advance_patch(*step, template, &binding, state, scale)
                                    .with_context(|| format!("patch {index}"))?,
                            ))
                        },
                    )
                    .collect::<Result<Vec<_>>>()?;
                // （`CNFireArea` 原来对 `tsoi17` 整列赋值、需在此广播；upstream-bugs 第 61 条已修，各 patch 只写自己的。）
                // `tracer_report`：一步里所有 patch 推进完之后（`CoLMDRIVER.F90:392-393`）。
                crate::tracer::report_after_patches(templates)?;
                timer.mark("patches");
                // `CoLM.F90:544-546`：`lateral_flow (idate(1), deltim)`，预热期也做。
                if let Some(catchment) = self.catchment.as_mut() {
                    catchment.step(&mut next_states)?;
                }
                timer.mark("catchment lateral flow");
                // `CoLM.F90:559-563`：陆面步之后、`hist_out` 之前汇流；预热期不汇流。
                if !clock.is_spinup {
                    if let Some((river, included)) = self.river.as_mut() {
                        // 被遮蔽的 patch 不在 `filter_rnof` 里（`included` 已与上掩膜），值不用。
                        let runoff = outputs
                            .iter()
                            .map(|output| {
                                output
                                    .as_ref()
                                    .map_or(0.0, |output| output.view().total_runoff_mm_s())
                            })
                            .collect::<Vec<_>>();
                        // `CoLMDRIVER` 把 `fevpg_fld`/`qinfl_fld` 写进 `flood_evap/infil_patch`。
                        if let Some(flood) = river.flood.as_mut() {
                            for (index, output) in outputs.iter().enumerate() {
                                let Some(output) = output else { continue };
                                let (evaporation, infiltration) =
                                    output.view().flood_exchange_mm_s();
                                flood.evap_mm_s[index] = evaporation;
                                flood.infil_mm_s[index] = infiltration;
                            }
                            // `flood_tracer_evap/land_patch`：走 CoLMMAIN 的 patch 本步写过，其余停在
                            // 发布时清的 0。
                            if let Some(tracer) = flood.tracer.as_mut() {
                                for (index, state) in next_states.iter().enumerate() {
                                    if let Some(exchange) = state
                                        .tracer
                                        .as_deref()
                                        .and_then(|track| track.flood_exchange.as_ref())
                                    {
                                        tracer.evap_patch[index].clone_from(&exchange.evap);
                                        tracer.land_patch[index].clone_from(&exchange.land);
                                    }
                                }
                            }
                        }
                        // `grid_riverlake_flow(idate(1), …)`：`TICKTIME` 之后的年份，即本步末。
                        // `trc_rnof_step(itrc, ipatch)`：本步每个 patch 的径流示踪物 [R*mm]。
                        let tracer_runoff = river.tracers.as_ref().map(|tracers| {
                            (0..tracers.set.len())
                                .map(|itrc| {
                                    next_states
                                        .iter()
                                        .map(|state| {
                                            state.tracer.as_deref().map_or(0.0, |track| {
                                                track.state.step[itrc].rnof_step
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .collect::<Vec<_>>()
                        });
                        // 泥沙的降水：`forc_prc`、`forc_prl`（读入并映射到 patch 的强迫）。
                        let precip = river.sediment.as_ref().map(|_| {
                            patch_forcing
                                .iter()
                                .map(|forcing| (forcing.prc, forcing.prl))
                                .collect::<Vec<_>>()
                        });
                        river.step(
                            &runoff,
                            tracer_runoff.as_deref(),
                            precip.as_deref(),
                            included,
                            time_step_seconds,
                            clock.end_time.year,
                        )?;
                        // `ch4_reactive_publish_flood`/`_levee_flood`：汇流过就覆盖甲烷状态里的三个比例。
                        if let Some(flood) = river.methane_flood.as_mut() {
                            if std::mem::take(&mut flood.published) {
                                for (p, state) in next_states.iter_mut().enumerate() {
                                    if let Some(methane) = state
                                        .bgc
                                        .as_deref_mut()
                                        .and_then(|bgc| bgc.methane.as_deref_mut())
                                    {
                                        methane.flood =
                                            [flood.levee[p], flood.fraction[p], flood.depth[p]];
                                    }
                                }
                            }
                        }
                    }
                }
                // `hist_out` 在写记录的那一步先写河道部分（`hist_grid_riverlake_out`），它把河道量
                // 交给会话，随本步的记录一起落进网格文件。
                if let (Some(history), Some((river, _))) =
                    (history.as_deref_mut(), self.river.as_mut())
                {
                    if !clock.is_spinup {
                        if let (Some(writer), Some(record)) = (
                            history.river.as_ref(),
                            history.session.pending_record(clock.end_time)?,
                        ) {
                            if !record.natural_boundary {
                                river.raw_history_at_end = Some(river.history.clone());
                            }
                            writer.write_record(
                                &river.network,
                                &river.routing,
                                &mut river.history,
                                river.tracers.as_mut(),
                                river.sediment.as_mut(),
                                &record,
                                clock.end_time,
                                &mut history.session,
                            )?;
                        }
                    }
                }
                timer.mark("grid river");
                // `hist_out`：`accumulate_fluxes` 末尾 `accumulate_fluxes_basin`，写记录时
                // `hist_basin_out`（`MOD_Hist.F90:5280`）。
                if let (Some(history), Some(catchment)) =
                    (history.as_deref_mut(), self.catchment.as_mut())
                {
                    if !clock.is_spinup {
                        catchment.accumulate_history();
                        let target = history
                            .basin
                            .clone()
                            .context("a catchment history needs its basin history target")?;
                        if let Some(record) = history.session.pending_record(clock.end_time)? {
                            let path = catchment.write_history(
                                &target.directory,
                                &target.stem,
                                &record,
                                target.compress_level,
                            )?;
                            if !history.files.contains(&path) {
                                history.files.push(path);
                            }
                        }
                    }
                }
                if let Some(history) = history.as_deref_mut() {
                    // 续跑旁车的窗口快照只在要写续跑的步上更新：周期续跑，或这一段的最后一步。
                    let last_step = next_clock.clone().next_step().is_none();
                    history
                        .session
                        .set_snapshot(clock.write_restart || last_step);
                    if !clock.is_spinup {
                        push_history(
                            history,
                            templates,
                            &next_states,
                            &outputs,
                            &steps,
                            &initial_totals,
                            time_step_seconds,
                            self.catchment.as_ref(),
                        )?;
                    }
                    // `tracer_hist_out`：主 history 之后；预热期不计步也不清零（会话里处理）。
                    if let Some(path) = history.session.push_tracer(
                        clock.end_time,
                        clock.is_spinup,
                        &mut next_states,
                    )? {
                        history.files.push(path);
                    }
                }
                timer.mark("history");
                if self.deferred_lai_refresh != Some(steps[0].clock.end_time) {
                    for ((template, state), step) in
                        templates.iter().zip(next_states.iter_mut()).zip(&steps)
                    {
                        crate::refresh_lai(*step, template, state)?;
                    }
                }
                let forcings = steps.iter().map(|step| step.forcing).collect::<Vec<_>>();
                if optimizer.is_some() {
                    // 入口已拒绝"优化 + 遮蔽"，这里每个 patch 都有输出。
                    let present = outputs.iter().flatten().collect::<Vec<_>>();
                    crate::optimize_baseflow(
                        optimizer.as_mut(),
                        steps[0],
                        &forcings,
                        &next_states,
                        &present,
                        time_step_seconds,
                    )?;
                }
                let views = outputs
                    .iter()
                    .map(|output| output.as_ref().map(PatchOutput::view))
                    .collect::<Vec<_>>();
                let cache = self.tracer_forcing.as_ref().map(|forcing| forcing.cache());
                on_step(
                    &steps,
                    &next_states,
                    &views,
                    self.river.as_ref().map(|(river, _)| river),
                    cache.as_ref(),
                    self.catchment.as_ref(),
                )?;
                timer.mark("lai, optimizer, restarts");
                *states = next_states;
                self.clock = next_clock;
                completed += 1;
            }
        })();
        timer.report(completed);
        self.baseflow_optimizer = optimizer;
        result.map(|()| completed)
    }
}

/// `COLM_RS_TIMING=1`：主循环各段的墙钟累计，运行结束时打到标准错误（只为性能剖析，不影响结果）。
struct PhaseTimer {
    last: Option<std::time::Instant>,
    totals: Vec<(&'static str, std::time::Duration)>,
}

impl PhaseTimer {
    fn from_env() -> Self {
        let on = std::env::var_os("COLM_RS_TIMING").is_some_and(|v| v != "0");
        Self {
            last: on.then(std::time::Instant::now),
            totals: Vec::new(),
        }
    }

    fn mark(&mut self, phase: &'static str) {
        let Some(last) = self.last else { return };
        let now = std::time::Instant::now();
        let elapsed = now - last;
        match self.totals.iter_mut().find(|(name, _)| *name == phase) {
            Some((_, total)) => *total += elapsed,
            None => self.totals.push((phase, elapsed)),
        }
        self.last = Some(now);
    }

    fn report(&self, steps: usize) {
        if self.last.is_none() {
            return;
        }
        let all: f64 = self.totals.iter().map(|(_, d)| d.as_secs_f64()).sum();
        eprintln!("colm-rs timing over {steps} step(s), {all:.1} s in the main loop:");
        for (name, total) in &self.totals {
            let secs = total.as_secs_f64();
            eprintln!(
                "  {name:<26} {secs:8.2} s  {:5.1}%",
                100.0 * secs / all.max(1e-9)
            );
        }
    }
}

/// `hist_out` 的累加：每个网格元先聚合一次近地面诊断，再逐 patch 交给会话。
#[allow(clippy::too_many_arguments)]
fn push_history(
    history: &mut super::history::SpatialHistory,
    templates: &[StandardLctRestartTemplate],
    states: &[StandardLctSnowSoilState],
    // 被强迫缺测遮蔽的 patch 没有输出（`None`）。
    outputs: &[Option<crate::PatchOutput>],
    steps: &[PointRuntimeStep],
    initial_totals: &[f64],
    time_step_seconds: f64,
    catchment: Option<&crate::catchment::runtime::CatchmentRuntime>,
) -> Result<()> {
    let reference = |index: usize| {
        let mut reference = crate::history::HistoryReferenceState::from_forcing(
            &steps[index].forcing,
            steps[index].surface_cosine_zenith,
            time_step_seconds,
            initial_totals[index],
        );
        reference.catch_lateral = catchment.is_some();
        reference
    };
    let mut jobs: Vec<
        Option<(
            crate::history::HistoryJob<'_>,
            Option<crate::history::HistoryOverrides>,
        )>,
    > = (0..templates.len()).map(|_| None).collect();
    for range in history.elements.ranges.clone() {
        // 网格元诊断只用未遮蔽的 patch（`filter = patchmask .and. forcmask_pch`）；
        // 全元都被遮蔽时上游 `CYCLE`，诊断留 `spval`。
        let inputs = range
            .clone()
            .filter_map(|index| {
                outputs[index].as_ref().map(|output| {
                    (
                        crate::history::patch_surface_input(
                            &templates[index],
                            output,
                            reference(index),
                        ),
                        history.elements.fractions[index],
                    )
                })
            })
            .collect::<Vec<_>>();
        let element = if inputs.is_empty() {
            None
        } else {
            Some(
                colm_core::history_diagnostics(crate::history::element_surface_input(&inputs)?)
                    .context("cannot recompute the element near-surface diagnostics")?,
            )
        };
        history.session.set_element_surface(element);
        for index in range {
            // `lateral_flow` 改写过的 patch 量（`rsur`/`rsub`/`rnof`/`wat`/`h2osoi` 与流域独有的三项）。
            let overrides = catchment.and_then(|c| c.history_overrides(index));
            let job = match &outputs[index] {
                Some(output) => crate::patch_history_job(
                    &mut history.session,
                    &templates[index],
                    &states[index],
                    output,
                    reference(index),
                )?,
                None => history
                    .session
                    .push_masked_job(&templates[index], &states[index])?,
            };
            ensure!(
                jobs[index].is_none(),
                "patch {index} belongs to two history elements"
            );
            jobs[index] = Some((job, overrides));
        }
    }
    // 生成任务（设会话标志、取网格元诊断）按 patch 串行；累加按 patch 并行（`push_jobs`）。
    let jobs = jobs
        .into_iter()
        .enumerate()
        .map(|(index, job)| job.with_context(|| format!("patch {index} has no history element")))
        .collect::<Result<Vec<_>>>()?;
    let end = steps
        .first()
        .context("a history step needs at least one patch")?
        .clock
        .end_time;
    if let Some(path) = history.session.push_jobs(end, jobs)? {
        history.files.push(path);
    }
    history.session.set_element_surface(None);
    Ok(())
}

/// 一个 patch 这一步的 `PointRuntimeStep`：强迫来自映射，天顶角/方位角按 patch 坐标。
fn patch_step(
    clock: colm_core::RuntimeStep,
    forcing: PatchForcing,
    longitude_radians: f64,
    latitude_radians: f64,
    calendar_day: f64,
    surface_calendar_day: f64,
) -> PointRuntimeStep {
    let cosine_zenith = orbital_cosine_zenith(calendar_day, longitude_radians, latitude_radians);
    let runtime = RuntimeForcing {
        air_temperature_k: forcing.t,
        specific_humidity: forcing.q,
        surface_pressure_pa: forcing.psrf,
        bottom_pressure_pa: forcing.pbot,
        convective_precipitation_kg_m2_s: forcing.prc,
        large_scale_precipitation_kg_m2_s: forcing.prl,
        eastward_wind_m_s: forcing.us,
        northward_wind_m_s: forcing.vs,
        downward_longwave_w_m2: forcing.frl,
        shortwave: ShortwaveForcing {
            direct_visible_w_m2: forcing.sols,
            direct_near_infrared_w_m2: forcing.soll,
            diffuse_visible_w_m2: forcing.solsd,
            diffuse_near_infrared_w_m2: forcing.solld,
        },
        solar_in_w_m2: forcing.solarin,
        cosine_zenith,
        air_density_kg_m3: forcing.rhoair,
        boundary_layer_height_m: forcing.hpbl,
    };
    PointRuntimeStep {
        clock,
        forcing: runtime,
        cosine_azimuth: orbital_cosine_azimuth(
            calendar_day,
            longitude_radians,
            latitude_radians,
            cosine_zenith,
        ),
        surface_cosine_zenith: orbital_cosine_zenith(
            surface_calendar_day,
            longitude_radians,
            latitude_radians,
        ),
    }
}

/// 空间算例的运行配置：时间窗口与频率沿用单点的解析，强迫换成网格配置。
#[derive(Debug, Clone)]
pub struct SpatialRuntimeConfig {
    pub start: colm_core::CalendarTime,
    pub end: colm_core::CalendarTime,
    pub spinup_until: colm_core::CalendarTime,
    pub timestep_seconds: f64,
    pub spinup_repeats: usize,
    pub lai_update_schedule: colm_core::LaiUpdateSchedule,
    pub restart_frequency: colm_core::RestartFrequency,
    pub co2_scenario: Co2Scenario,
    pub history_frequency: colm_hist::schedule::HistoryFrequency,
    pub history_grouping: colm_hist::schedule::HistoryGrouping,
    pub forcing: super::forcing::GriddedForcingConfig,
    /// `DEF_Forcing_Interp_Method = 'bilinear'`：强迫到 patch 的映射用 `build_bilinear`，带上
    /// `DEF_domain`（决定哪些强迫行列在块覆盖里）。
    pub bilinear: Option<colm_init::spatial_grid::GridBounds>,
    /// `DEF_USE_Forcing_Downscaling(_Simple)` 与 `DEF_DS_*`。
    pub downscaling: Option<super::downscaling::DownscalingSettings>,
}

impl SpatialRuntimeConfig {
    pub fn read(case_namelist: &std::path::Path) -> Result<Self> {
        let case = crate::read_document(case_namelist, "case")?;
        let forcing_namelist =
            std::path::PathBuf::from(crate::required_string(&case, "DEF_forcing_namelist")?);
        let forcing_document = crate::read_document(&forcing_namelist, "forcing")?;
        // 空间强迫里还没移植的分支：一次列全，默认拒绝（原来它们会被悄悄忽略）。
        let interpolation = match case.get("DEF_Forcing_Interp_Method") {
            Some(colm_namelist::Value::Str(text)) => text.trim().to_owned(),
            Some(other) => anyhow::bail!("DEF_Forcing_Interp_Method must be a string, got {other}"),
            None => "arealweight".to_owned(),
        };
        anyhow::ensure!(
            matches!(interpolation.as_str(), "arealweight" | "bilinear"),
            "unknown DEF_Forcing_Interp_Method = {interpolation:?}; upstream accepts 'arealweight' \
             and 'bilinear'"
        );
        // 双线性的邻格限在 `DEF_domain` 的块覆盖内（upstream-bugs 第 42 条，两侧都已修）。
        let downscaling = super::downscaling::DownscalingSettings::from_case(&case)?;
        let start = crate::simulation_date(&case, "start")?;
        let spinup_until =
            if crate::required_integer(&case, "DEF_simulation_time%spinup_year")? == 0 {
                start
            } else {
                crate::simulation_date(&case, "spinup")?
            };
        let mut forcing = super::forcing::GriddedForcingConfig::from_document(&forcing_document)?;
        forcing.clim_spinup =
            crate::optional_bool_or(&case, "DEF_USE_ClimForcing_for_Spinup", false)?;
        if crate::optional_bool_or(&case, "DEF_USE_CBL_HEIGHT", false)? {
            forcing.enable_cbl(&forcing_document)?;
        }
        Ok(Self {
            start,
            end: crate::simulation_date(&case, "end")?,
            spinup_until,
            timestep_seconds: crate::required_real(&case, "DEF_simulation_time%timestep")?,
            spinup_repeats: usize::try_from(crate::required_integer(
                &case,
                "DEF_simulation_time%spinup_repeat",
            )?)
            .context("DEF_simulation_time%spinup_repeat must be nonnegative")?,
            lai_update_schedule: if crate::optional_bool_or(&case, "DEF_LAI_MONTHLY", true)? {
                colm_core::LaiUpdateSchedule::Monthly
            } else {
                colm_core::LaiUpdateSchedule::EightDay
            },
            restart_frequency: crate::restart_frequency(&case)?,
            co2_scenario: crate::co2_scenario(&case)?,
            history_frequency: crate::history_frequency(&case)?,
            history_grouping: crate::history_grouping(&case)?,
            forcing,
            bilinear: if interpolation == "bilinear" {
                Some(colm_init::spatial_grid::GridBounds {
                    south: crate::required_real(&case, "DEF_domain%edges")?,
                    north: crate::required_real(&case, "DEF_domain%edgen")?,
                    west: crate::required_real(&case, "DEF_domain%edgew")?,
                    east: crate::required_real(&case, "DEF_domain%edgee")?,
                })
            } else {
                None
            },
            downscaling,
        })
    }

    pub fn history_window(&self) -> crate::HistoryWindowSpec {
        crate::HistoryWindowSpec {
            start: self.start,
            spinup_until: self.spinup_until,
            end: self.end,
            timestep_seconds: self.timestep_seconds,
            frequency: self.history_frequency,
            grouping: self.history_grouping,
        }
    }

    pub fn clock(&self) -> Result<RuntimeClock> {
        Ok(RuntimeClock::with_lai_update_schedule(
            self.start,
            self.end,
            self.spinup_until,
            self.timestep_seconds,
            self.spinup_repeats,
            self.lai_update_schedule,
        )?
        .with_restart_frequency(self.restart_frequency))
    }
}
