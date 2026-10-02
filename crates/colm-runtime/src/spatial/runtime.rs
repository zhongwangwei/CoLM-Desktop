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
}

impl SpatialRuntime {
    pub fn new(
        clock: RuntimeClock,
        forcing: GriddedForcing,
        mapping: AreaWeightedMapping,
        coordinates: Vec<(f64, f64)>,
        co2_scenario: Co2Scenario,
    ) -> Result<Self> {
        ensure!(
            mapping.parts.len() == coordinates.len(),
            "the forcing mapping covers {} patches but {} coordinates were given",
            mapping.parts.len(),
            coordinates.len()
        );
        ensure!(
            mapping.area.iter().all(|&area| area > 0.0),
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
        })
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
            &[PatchStepOutput<'_>],
            Option<&crate::river::RiverModel>,
            Option<&crate::tracer::ForcingCache<'_>>,
        ) -> Result<()>,
    {
        ensure!(
            templates.len() == states.len() && templates.len() == self.coordinates.len(),
            "every patch needs one template, one state and one coordinate"
        );
        let time_step_seconds = self.clock.timestep_seconds();
        let mut optimizer = self.baseflow_optimizer.take();
        let mut completed = 0;
        let result = (|| -> Result<()> {
            loop {
                let mut next_clock = self.clock.clone();
                let Some(clock) = next_clock.next_step() else {
                    return Ok(());
                };
                let (month, _) = colm_core::month_day(clock.forcing_time)?;
                let co2 =
                    colm_core::monthly_co2_ppm(self.co2_scenario, clock.forcing_time.year, month)?
                        * 1.0e-6;
                let cells = self.forcing.step(clock.forcing_time, co2)?;
                let patch_forcing = map_to_patches(&self.mapping, &self.forcing, &cells);
                // `read_tracer_forcing (jdate, dir_forcing)`：紧跟 `read_forcing`。
                if let Some(tracer_forcing) = self.tracer_forcing.as_mut() {
                    tracer_forcing.step(clock.forcing_time, &self.forcing, &self.mapping)?;
                }
                let calendar_day = orbital_calendar_day(clock.forcing_time, true, 0.0)?;
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
                let mut next_states = states.clone();
                let mut outputs = Vec::with_capacity(templates.len());
                let initial_totals = templates
                    .iter()
                    .zip(states.iter())
                    .map(|(template, state)| crate::initial_total_water_mm(template, state))
                    .collect::<Vec<_>>();
                for (index, ((template, state), step)) in templates
                    .iter()
                    .zip(next_states.iter_mut())
                    .zip(&steps)
                    .enumerate()
                {
                    let binding = StandardLctStepBinding {
                        forcing: step.forcing,
                        seconds_of_day,
                        greenwich_time: true,
                        longitude_radians: self.coordinates[index].0,
                        co2_volume_fraction: co2,
                        partial_pressures_pa: Some((
                            patch_forcing[index].pco2m,
                            patch_forcing[index].po2m,
                        )),
                        // 漫滩回馈只作用在土壤 patch 上（`patchtype == 0`）。
                        flood: self
                            .river
                            .as_ref()
                            .and_then(|(river, _)| river.flood.as_ref())
                            .filter(|_| template.patch_type == 0)
                            .map(|flood| colm_core::flood_evaporation::FloodPatchInput {
                                depth_mm: flood.depth_mm[index],
                                fraction: flood.fraction[index],
                                infiltration_max_mm_day: flood.infiltration_max_mm_day,
                            }),
                        tracer_ratios: self
                            .tracer_forcing
                            .as_ref()
                            .map(|forcing| forcing.ratios(index)),
                    };
                    let scale = optimizer.as_ref().map(|optimizer| optimizer.scale(index));
                    outputs.push(
                        crate::advance_patch(*step, template, &binding, state, scale)
                            .with_context(|| format!("patch {index}"))?,
                    );
                }
                // `CNFireArea` 的 `tsoi17 = forc_t(i)` 是整列赋值：一步之后所有 patch 都是最后一个
                // 跑火灾的 patch 的值（续跑里写的就是它）。
                crate::broadcast_fire_tsoi17(templates, &mut next_states);
                // `tracer_report`：一步里所有 patch 推进完之后（`CoLMDRIVER.F90:392-393`）。
                crate::tracer::report_after_patches(templates)?;
                // `CoLM.F90:559-563`：陆面步之后、`hist_out` 之前汇流；预热期不汇流。
                if !clock.is_spinup {
                    if let Some((river, included)) = self.river.as_mut() {
                        let runoff = outputs
                            .iter()
                            .map(|output| output.view().total_runoff_mm_s())
                            .collect::<Vec<_>>();
                        // `CoLMDRIVER` 把 `fevpg_fld`/`qinfl_fld` 写进 `flood_evap/infil_patch`。
                        if let Some(flood) = river.flood.as_mut() {
                            for (index, output) in outputs.iter().enumerate() {
                                let (evaporation, infiltration) =
                                    output.view().flood_exchange_mm_s();
                                flood.evap_mm_s[index] = evaporation;
                                flood.infil_mm_s[index] = infiltration;
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
                if let Some(history) = history.as_deref_mut() {
                    if !clock.is_spinup {
                        push_history(
                            history,
                            templates,
                            &next_states,
                            &outputs,
                            &steps,
                            &initial_totals,
                            time_step_seconds,
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
                if self.deferred_lai_refresh != Some(steps[0].clock.end_time) {
                    for ((template, state), step) in
                        templates.iter().zip(next_states.iter_mut()).zip(&steps)
                    {
                        crate::refresh_lai(*step, template, state)?;
                    }
                }
                let forcings = steps.iter().map(|step| step.forcing).collect::<Vec<_>>();
                crate::optimize_baseflow(
                    optimizer.as_mut(),
                    steps[0],
                    &forcings,
                    &next_states,
                    &outputs,
                    time_step_seconds,
                )?;
                let views = outputs.iter().map(PatchOutput::view).collect::<Vec<_>>();
                let cache = self.tracer_forcing.as_ref().map(|forcing| forcing.cache());
                on_step(
                    &steps,
                    &next_states,
                    &views,
                    self.river.as_ref().map(|(river, _)| river),
                    cache.as_ref(),
                )?;
                *states = next_states;
                self.clock = next_clock;
                completed += 1;
            }
        })();
        self.baseflow_optimizer = optimizer;
        result.map(|()| completed)
    }
}

/// `hist_out` 的累加：每个网格元先聚合一次近地面诊断，再逐 patch 交给会话。
fn push_history(
    history: &mut super::history::SpatialHistory,
    templates: &[StandardLctRestartTemplate],
    states: &[StandardLctSnowSoilState],
    outputs: &[crate::PatchOutput],
    steps: &[PointRuntimeStep],
    initial_totals: &[f64],
    time_step_seconds: f64,
) -> Result<()> {
    let reference = |index: usize| {
        crate::history::HistoryReferenceState::from_forcing(
            &steps[index].forcing,
            steps[index].surface_cosine_zenith,
            time_step_seconds,
            initial_totals[index],
        )
    };
    for range in history.elements.ranges.clone() {
        let inputs = range
            .clone()
            .map(|index| {
                (
                    crate::history::patch_surface_input(
                        &templates[index],
                        &outputs[index],
                        reference(index),
                    ),
                    history.elements.fractions[index],
                )
            })
            .collect::<Vec<_>>();
        let element =
            colm_core::history_diagnostics(crate::history::element_surface_input(&inputs)?)
                .context("cannot recompute the element near-surface diagnostics")?;
        history.session.set_element_surface(Some(element));
        for index in range {
            if let Some(path) = crate::push_patch_history(
                &mut history.session,
                steps[index].clock.end_time,
                &templates[index],
                &states[index],
                &outputs[index],
                reference(index),
            )? {
                history.files.push(path);
            }
        }
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
        boundary_layer_height_m: None,
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
}

impl SpatialRuntimeConfig {
    pub fn read(case_namelist: &std::path::Path) -> Result<Self> {
        let case = crate::read_document(case_namelist, "case")?;
        let forcing_namelist =
            std::path::PathBuf::from(crate::required_string(&case, "DEF_forcing_namelist")?);
        let forcing = crate::read_document(&forcing_namelist, "forcing")?;
        // 空间强迫里还没移植的分支：一次列全，默认拒绝（原来它们会被悄悄忽略）。
        let interpolation = match case.get("DEF_Forcing_Interp_Method") {
            Some(colm_namelist::Value::Str(text)) => text.trim().to_owned(),
            Some(other) => anyhow::bail!("DEF_Forcing_Interp_Method must be a string, got {other}"),
            None => "arealweight".to_owned(),
        };
        anyhow::ensure!(
            interpolation == "arealweight",
            "DEF_Forcing_Interp_Method = {interpolation:?} is not ported to the Rust spatial \
             runtime (only 'arealweight'); run this case with --engine fortran"
        );
        for (key, what) in [
            (
                "DEF_USE_ClimForcing_for_Spinup",
                "climatological spin-up forcing",
            ),
            (
                "DEF_USE_CBL_HEIGHT",
                "the boundary-layer-height forcing (variable 9)",
            ),
            ("DEF_USE_Forcing_Downscaling", "forcing downscaling"),
            (
                "DEF_USE_Forcing_Downscaling_Simple",
                "simple forcing downscaling",
            ),
        ] {
            anyhow::ensure!(
                !crate::optional_bool_or(&case, key, false)?,
                "{key} ({what}) is not ported to the Rust spatial runtime; run this case with \
                 --engine fortran"
            );
        }
        let start = crate::simulation_date(&case, "start")?;
        let spinup_until =
            if crate::required_integer(&case, "DEF_simulation_time%spinup_year")? == 0 {
                start
            } else {
                crate::simulation_date(&case, "spinup")?
            };
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
            forcing: super::forcing::GriddedForcingConfig::from_document(&forcing)?,
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
