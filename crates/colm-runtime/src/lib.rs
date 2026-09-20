//! Rust runtime orchestration for CoLM.
//!
//! This crate owns executable-stage coordination only.  Numerical kernels stay
//! in `colm-core`, NetCDF POINT forcing stays in `colm-forcing`, and restart
//! serialization stays in `colm-init`; that keeps `colm-init` from becoming a
//! second copy of `colm.x`.

pub mod assembly;
pub mod history;
pub mod physics;

use std::path::{Path, PathBuf};

use crate::assembly::{StandardLctRestartTemplate, StandardLctStepBinding, SurfaceOpticsStep};
use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    apply_downscaled_runtime_forcing, downscale_forcings, grid_forcing_from_runtime,
    month_day_to_julian, orbital_calendar_day, orbital_cosine_azimuth, orbital_cosine_zenith,
    standard_lct_soil_step, CalendarTime, Co2Scenario, DownscalingSolarGeometry,
    DownscalingTerrain, ForcingDownscalingConfig, ForcingDownscalingInput, LaiUpdateSchedule,
    RestartFrequency, RuntimeClock, RuntimeForcing, RuntimeStep, StandardLctSnowSoilOutput,
    StandardLctSnowSoilState, StandardLctSoilInput, StandardLctSoilOutput, StandardLctSoilState,
};
use colm_forcing::{load_point_forcing, PointForcingSeries};
use colm_namelist::{parse, Document, Value};

/// The POINT subset of the `CoLM.F90` runtime configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PointRuntimeConfig {
    pub start: CalendarTime,
    pub end: CalendarTime,
    pub spinup_until: CalendarTime,
    pub timestep_seconds: f64,
    pub spinup_repeats: usize,
    pub lai_update_schedule: LaiUpdateSchedule,
    pub restart_frequency: RestartFrequency,
    /// `DEF_SSP`：未来 CO2 情景。它只影响 2022 年之后的年份。
    pub co2_scenario: Co2Scenario,
    /// `DEF_HIST_FREQ`。缺省与上游一致是 `none`（不写 history）。
    pub history_frequency: colm_hist::schedule::HistoryFrequency,
    /// `DEF_HIST_groupby`。缺省与上游一致是 `MONTH`。
    pub history_grouping: colm_hist::schedule::HistoryGrouping,
    pub greenwich: bool,
    pub longitude_degrees: f64,
    pub latitude_degrees: f64,
    pub forcing_file: PathBuf,
    /// `forc_hgt_u/t/q`：风、温、湿的参考高度。
    ///
    /// 优先级与上游一致（`MOD_Forcing.F90:297-311`）：**强迫文件里的
    /// `reference_height_v/t/q` 优先**，文件里没有才用 `DEF_forcing%HEIGHT_*`
    /// （`nl_forcing_type`，在 **forcing** namelist 里，不是 case namelist）。
    /// 实测 CN-Cng 的文件写着 6/6/6，而 schema 默认是 100/50/50 ——
    /// 用错一套会让 `zol` 差几十倍。
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
}

/// One fully prepared POINT forcing record for a `CoLM.F90` loop pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointRuntimeStep {
    pub clock: RuntimeStep,
    /// Grid-level forcing prepared by the common reader path.
    pub forcing: RuntimeForcing,
    /// `MOD_OrbCosazi` evaluated from this step's local orbital calendar.
    pub cosine_azimuth: f64,
    /// `CoLMMAIN.F90:2076` 的 `coszen`：`orb_coszen(calendarday(idate))`，
    /// 而 `idate` 已被 `CoLM.F90:480` 的 `TICKTIME` 推到**步末**。
    ///
    /// **不要**用 [`Self::forcing`] 里那个 `cosine_zenith` 顶替：那一个来自
    /// `MOD_Forcing`，是在 `TICKTIME` **之前**按步首时刻算的，用于短波直散拆分与
    /// 地形降尺度。上游确实同时存在这两个值，混用会让 `albland` 的太阳天顶角
    /// 差半个步长 —— 实测 CN-Cng 第 1 天正午 `0.379821` 与 `0.374192`。
    pub surface_cosine_zenith: f64,
}

/// Static terrain data needed to downscale one POINT forcing series.
///
/// The lifetime belongs to restart/surface data owned by the caller; the
/// runtime neither copies it nor lets it leak into the numerical core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointDownscalingTemplate<'a> {
    pub grid_surface_elevation_m: f64,
    pub grid_maximum_elevation_m: f64,
    pub reference_height_m: f64,
    pub column_surface_elevation_m: f64,
    pub glacier: bool,
    pub terrain: DownscalingTerrain<'a>,
    pub config: ForcingDownscalingConfig,
}

/// history 里的 `lat`/`lon` 取**单精度**的站点坐标。
///
/// 上游把站点经纬度存成 `real(r4)`（`mksrfdata` 会为此报
/// "Latitude mismatch: 44.593299865722656 in data file and 44.593299999999999 in
/// namelist"），写进 history 的 `lat`/`lon` 因此是 f32 量化过的值。
/// 这两个量是 tier0 的**坐标**、按逐位比，所以本仓库也必须量化 ——
/// 直接写 namelist 里的 f64 会在 `golden-compare` 里报成两个变量超差。
fn site_coordinate_degrees(value: f64) -> f64 {
    f64::from(value as f32)
}

impl PointRuntimeConfig {
    /// 按本算例的窗口、频率与分组开一个 history 会话。
    ///
    /// 窗口、站点与步长都来自同一份配置，所以调用方不必自己拼 `SimulationWindow` ——
    /// 拼错一个字段（例如把结束时刻写成时长）只会让记录数悄悄不对。
    pub fn history_session(
        &self,
        directory: impl AsRef<Path>,
        stem: impl Into<String>,
    ) -> Result<crate::history::HistorySession> {
        ensure!(
            self.timestep_seconds.fract() == 0.0,
            "the history schedule needs whole-second timesteps"
        );
        let timestep_seconds = i32::try_from(self.timestep_seconds as i64)
            .context("the timestep does not fit the history schedule")?;
        let field = |time: CalendarTime| -> Result<(i32, i32, i32)> {
            Ok((
                time.year,
                i32::from(time.julian_day),
                i32::try_from(time.seconds).context("seconds do not fit an i32")?,
            ))
        };
        let (start_year, start_julian_day, start_seconds) = field(self.start)?;
        let (end_year, end_julian_day, end_seconds) = field(self.end)?;
        crate::history::HistorySession::new(
            crate::history::point_dimensions(),
            colm_hist::history::HistorySite {
                latitude_degrees: site_coordinate_degrees(self.latitude_degrees),
                longitude_degrees: site_coordinate_degrees(self.longitude_degrees),
            },
            colm_hist::schedule::SimulationWindow {
                start_year,
                start_julian_day,
                start_seconds,
                end_year,
                end_julian_day,
                end_seconds,
                timestep_seconds,
            },
            self.history_frequency,
            self.history_grouping,
            directory,
            stem,
        )
    }
}

/// 一次带 history 的运行的产出：走了多少步、写了哪些文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRunOutcome {
    pub steps: usize,
    pub files: Vec<PathBuf>,
}

/// One committed `read_forcing → downscale_forcings` runtime hand-off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownscaledPointRuntimeStep {
    pub clock: RuntimeStep,
    pub grid_forcing: RuntimeForcing,
    pub forcing: RuntimeForcing,
}

/// The Rust `CoLM.F90 → read_forcing` orchestration for one POINT location.
///
/// Physical patch dispatch is intentionally a separate next layer: this type
/// is the single owner of the clock and of calendar-to-NetCDF forcing lookup,
/// so every later LCT/PFT/PC/urban driver starts from the same prepared state.
#[derive(Debug, Clone)]
pub struct PointRuntime {
    clock: RuntimeClock,
    forcing: PointForcingSeries,
    greenwich: bool,
    longitude_degrees: f64,
    latitude_degrees: f64,
    co2_scenario: Co2Scenario,
}

impl PointRuntime {
    /// Opens the configured POINT forcing series once, before stepping.
    pub fn open(config: PointRuntimeConfig) -> Result<Self> {
        ensure!(
            config.longitude_degrees.is_finite() && config.latitude_degrees.is_finite(),
            "POINT runtime location must be finite"
        );
        Ok(Self {
            clock: RuntimeClock::with_lai_update_schedule(
                config.start,
                config.end,
                config.spinup_until,
                config.timestep_seconds,
                config.spinup_repeats,
                config.lai_update_schedule,
            )?
            .with_restart_frequency(config.restart_frequency),
            forcing: load_point_forcing(&config.forcing_file)?,
            greenwich: config.greenwich,
            longitude_degrees: config.longitude_degrees,
            latitude_degrees: config.latitude_degrees,
            co2_scenario: config.co2_scenario,
        })
    }

    /// Advances one CoLM clock pass and prepares its forcing.
    ///
    /// The clock advances only after the forcing lookup succeeds, so a bad
    /// forcing boundary cannot silently skip a model step.
    pub fn next_step(&mut self) -> Result<Option<PointRuntimeStep>> {
        let Some((next_clock, step)) = self.prepared_next_step()? else {
            return Ok(None);
        };
        self.clock = next_clock;
        Ok(Some(step))
    }

    /// Runs the shared POINT loop and commits each clock pass only after its
    /// physics/history callback succeeds.
    ///
    /// LCT, PFT, PC, and urban dispatchers use this one loop rather than each
    /// reproducing `CoLM.F90`'s forcing-to-driver progression.  If a callback
    /// fails, the uncommitted clock remains at that same model step.
    pub fn run<F>(&mut self, mut on_step: F) -> Result<usize>
    where
        F: FnMut(PointRuntimeStep) -> Result<()>,
    {
        let mut completed = 0;
        while let Some((next_clock, step)) = self.prepared_next_step()? {
            on_step(step)?;
            self.clock = next_clock;
            completed += 1;
        }
        Ok(completed)
    }

    /// Runs one persistent Rust model state through the shared POINT loop.
    ///
    /// The next state is committed only after its callback succeeds, alongside
    /// the clock commit in [`Self::run`].  This is the common boundary for
    /// every Rust `CoLMDRIVER` branch: callers do not need to duplicate clock,
    /// forcing, or failure-retry behavior around their own state updates.
    pub fn run_with_state<S, F>(&mut self, state: &mut S, mut on_step: F) -> Result<usize>
    where
        S: Clone,
        F: FnMut(PointRuntimeStep, &mut S) -> Result<()>,
    {
        self.run(|step| {
            let mut next = state.clone();
            on_step(step, &mut next)?;
            *state = next;
            Ok(())
        })
    }

    /// Runs the already ported regular-soil LCT chain from the shared POINT loop.
    ///
    /// `template` supplies static land parameters; its forcing is overwritten
    /// on each pass with the record owned by this runtime.  `state` is only
    /// committed after the physics and output callback both succeed.  This is
    /// intentionally the no-snow regular-soil branch that
    /// [`colm_core::standard_lct_soil_step`] supports today; other CoLM patch
    /// branches must add their own exact core drivers rather than approximating
    /// them here.
    pub fn run_standard_lct<F>(
        &mut self,
        template: StandardLctSoilInput<'_>,
        state: &mut StandardLctSoilState,
        mut on_step: F,
    ) -> Result<usize>
    where
        F: FnMut(PointRuntimeStep, &StandardLctSoilOutput) -> Result<()>,
    {
        self.run_with_state(state, |step, next| {
            let mut input = template;
            input.energy.forcing = step.forcing;
            let output = standard_lct_soil_step(input, next)?;
            on_step(step, &output)
        })
    }

    /// 上游只有一条入口：`CoLMMAIN` 每步无条件 `newsnow`，雪是状态。所以真正跑算例的
    /// 那一支是 [`Self::run_restart_standard_lct_snow`] —— 它带末尾的
    /// 「Preparation for the next time step」。这一支（无雪状态）现在只服务合成算例与
    /// 单元测试，**不做**那一节：它的 `scv`/`snowdp` 恒为 0，`fsno`/`sag` 没有落点，
    /// 而 `lai`/`sai` 的差别在这里已经被装配期的断言钉死。
    ///
    /// Runs an assembled restart template through the whole POINT forcing window.
    ///
    /// This is the layer that was missing: [`StandardLctRestartTemplate`] knows the static
    /// state the restart files own, the POINT loop knows the clock and forcing, and the
    /// per-step [`StandardLctStepBinding`] is what joins them.  The binding is rebuilt on
    /// every pass on purpose: wind, seconds-of-day and longitude are passed straight
    /// through by the kernels rather than recomputed, so a template that carried them
    /// would be one step stale from the second pass on.
    ///
    /// `state` is the caller's, so the same state can be written back out as a restart.
    /// It is committed together with the clock and forcing, which means a failed physics
    /// or output callback leaves all three at the same retryable step.
    pub fn run_restart_standard_lct<F>(
        &mut self,
        template: &StandardLctRestartTemplate,
        state: &mut StandardLctSoilState,
        mut on_step: F,
    ) -> Result<usize>
    where
        F: FnMut(PointRuntimeStep, &StandardLctSoilOutput) -> Result<()>,
    {
        let (greenwich_time, longitude_degrees, co2_scenario) =
            (self.greenwich, self.longitude_degrees, self.co2_scenario);
        self.run_with_state(state, |step, next| {
            let binding = lct_binding(step, greenwich_time, longitude_degrees, co2_scenario)?;
            let output = standard_lct_soil_step(template.input(&binding), next)?;
            on_step(step, &output)
        })
    }

    /// Runs the no-snow branch **and** writes history through the schedule.
    ///
    /// Same loop, same binding, same transaction as [`Self::run_restart_standard_lct`];
    /// the only addition is that each step's post-step state and diagnostics go into
    /// `session`. When the loop ends the session must have consumed its whole schedule —
    /// a run that stops short would otherwise flush a file full of zero-valued records,
    /// which reads exactly like real data.
    pub fn run_restart_standard_lct_with_history<F>(
        &mut self,
        template: &StandardLctRestartTemplate,
        state: &mut StandardLctSoilState,
        session: &mut crate::history::HistorySession,
        mut on_step: F,
    ) -> Result<HistoryRunOutcome>
    where
        F: FnMut(PointRuntimeStep, &StandardLctSoilOutput) -> Result<()>,
    {
        let (greenwich_time, longitude_degrees, co2_scenario) =
            (self.greenwich, self.longitude_degrees, self.co2_scenario);
        // 中途换分组时 `push` 会把上一个文件落盘并返回它的路径 —— 丢掉它会让
        // `outcome.files` 少列文件（实测过：跨月运行只报了二月那一个）。
        let mut files = Vec::new();
        let steps = self.run_with_state(state, |step, next| {
            let binding = lct_binding(step, greenwich_time, longitude_degrees, co2_scenario)?;
            let output = standard_lct_soil_step(template.input(&binding), next)?;
            if let Some(path) = session.push_lct(
                step.clock.end_time,
                template,
                next,
                &output,
                crate::history::HistoryReferenceState::from_forcing(&step.forcing),
            )? {
                files.push(path);
            }
            on_step(step, &output)
        })?;
        files.extend(session.finish()?);
        ensure!(
            session.remaining() == 0,
            "the run ended with {} history record(s) still unwritten; the window and the \
             schedule disagree",
            session.remaining()
        );
        Ok(HistoryRunOutcome { steps, files })
    }

    /// Runs the snow branch **and** writes history through the schedule.
    ///
    /// The snow sibling of [`Self::run_restart_standard_lct_with_history`].
    pub fn run_restart_standard_lct_snow_with_history<F>(
        &mut self,
        template: &StandardLctRestartTemplate,
        state: &mut StandardLctSnowSoilState,
        session: &mut crate::history::HistorySession,
        mut on_step: F,
    ) -> Result<HistoryRunOutcome>
    where
        F: FnMut(PointRuntimeStep, &StandardLctSnowSoilOutput) -> Result<()>,
    {
        let (greenwich_time, longitude_degrees, co2_scenario) =
            (self.greenwich, self.longitude_degrees, self.co2_scenario);
        let mut files = Vec::new();
        let steps = self.run_with_state(state, |step, next| {
            let binding = lct_binding(step, greenwich_time, longitude_degrees, co2_scenario)?;
            // `scvold`：上游在 `newsnow` **之前**把 `scv` 抄一份（`CoLMMAIN.F90:814`），
            // 所以要在内核动手之前读。
            let previous_snow_water_equivalent_mm = next.snow.water_equivalent_kg_m2;
            let output =
                colm_core::standard_lct_snow_soil_step(template.snow_input(&binding), next)?;
            // 顺序不能反：上游 `hist_out`（`CoLM.F90:537`）在 `CoLMDRIVER`（`:512`）
            // **之后**跑，而末尾那一节在 `CoLMDRIVER` 里面。所以 history 记下的
            // `fsno`/`lai`/`sai` 是**下一步**的值，不是这一步用掉的那一组。
            template.prepare_surface_optics(
                next,
                surface_optics_step(step, previous_snow_water_equivalent_mm, &output),
            )?;
            if let Some(path) = session.push_lct_snow(
                step.clock.end_time,
                template,
                next,
                &output,
                crate::history::HistoryReferenceState::from_forcing(&step.forcing),
            )? {
                files.push(path);
            }
            on_step(step, &output)
        })?;
        files.extend(session.finish()?);
        ensure!(
            session.remaining() == 0,
            "the run ended with {} history record(s) still unwritten; the window and the \
             schedule disagree",
            session.remaining()
        );
        Ok(HistoryRunOutcome { steps, files })
    }

    /// Runs an assembled snow-bearing restart template through the POINT window.
    ///
    /// The snow sibling of [`Self::run_restart_standard_lct`]: same clock, same binding,
    /// same transaction — only the kernel and the state type differ. Both share
    /// [`Self::lct_binding`] so the per-step fields cannot drift between the branches.
    pub fn run_restart_standard_lct_snow<F>(
        &mut self,
        template: &StandardLctRestartTemplate,
        state: &mut StandardLctSnowSoilState,
        mut on_step: F,
    ) -> Result<usize>
    where
        F: FnMut(PointRuntimeStep, &StandardLctSnowSoilOutput) -> Result<()>,
    {
        let (greenwich_time, longitude_degrees, co2_scenario) =
            (self.greenwich, self.longitude_degrees, self.co2_scenario);
        self.run_with_state(state, |step, next| {
            let binding = lct_binding(step, greenwich_time, longitude_degrees, co2_scenario)?;
            // 同带 history 的那一支：`scvold` 必须在 `newsnow` 之前读。
            let previous_snow_water_equivalent_mm = next.snow.water_equivalent_kg_m2;
            let output =
                colm_core::standard_lct_snow_soil_step(template.snow_input(&binding), next)?;
            template.prepare_surface_optics(
                next,
                surface_optics_step(step, previous_snow_water_equivalent_mm, &output),
            )?;
            on_step(step, &output)
        })
    }

    /// Runs POINT forcing through the shared CoLM terrain-downscaling kernel.
    ///
    /// This is deliberately a forcing hand-off, rather than another physics
    /// driver: all later LCT/PFT/PC/urban branches receive one adjusted
    /// [`RuntimeForcing`] without duplicating `MOD_Forcing` mathematics.
    pub fn run_downscaled<F>(
        &mut self,
        template: PointDownscalingTemplate<'_>,
        mut on_step: F,
    ) -> Result<usize>
    where
        F: FnMut(DownscaledPointRuntimeStep) -> Result<()>,
    {
        let greenwich = self.greenwich;
        let longitude_degrees = self.longitude_degrees;
        self.run(|step| {
            let grid = grid_forcing_from_runtime(
                step.forcing,
                template.grid_surface_elevation_m,
                template.grid_maximum_elevation_m,
                template.reference_height_m,
            )?;
            let forcing = apply_downscaled_runtime_forcing(
                step.forcing,
                downscale_forcings(
                    ForcingDownscalingInput {
                        glacier: template.glacier,
                        grid,
                        column_surface_elevation_m: template.column_surface_elevation_m,
                        solar: DownscalingSolarGeometry {
                            calendar_day: orbital_calendar_day(
                                step.clock.forcing_time,
                                greenwich,
                                longitude_degrees,
                            )?,
                            cosine_zenith: step.forcing.cosine_zenith,
                            cosine_azimuth: step.cosine_azimuth,
                        },
                        terrain: template.terrain,
                    },
                    template.config,
                )?,
            );
            on_step(DownscaledPointRuntimeStep {
                clock: step.clock,
                grid_forcing: step.forcing,
                forcing,
            })
        })
    }

    /// Runs a persistent Rust state through the shared downscaled POINT loop.
    ///
    /// State, forcing and clock are committed as one transaction.  A failed
    /// driver/history callback leaves all three at the same retryable step.
    pub fn run_downscaled_with_state<S, F>(
        &mut self,
        template: PointDownscalingTemplate<'_>,
        state: &mut S,
        mut on_step: F,
    ) -> Result<usize>
    where
        S: Clone,
        F: FnMut(DownscaledPointRuntimeStep, &mut S) -> Result<()>,
    {
        self.run_downscaled(template, |step| {
            let mut next = state.clone();
            on_step(step, &mut next)?;
            *state = next;
            Ok(())
        })
    }

    /// Runs the existing regular-soil LCT chain after shared terrain downscaling.
    ///
    /// This retains [`Self::run_standard_lct`]'s intentionally limited
    /// no-snow LCT contract.  Its only new responsibility is selecting the
    /// column forcing before the same `standard_lct_soil_step` call.
    pub fn run_downscaled_standard_lct<F>(
        &mut self,
        downscaling: PointDownscalingTemplate<'_>,
        input_template: StandardLctSoilInput<'_>,
        state: &mut StandardLctSoilState,
        mut on_step: F,
    ) -> Result<usize>
    where
        F: FnMut(DownscaledPointRuntimeStep, &StandardLctSoilOutput) -> Result<()>,
    {
        self.run_downscaled_with_state(downscaling, state, |step, next| {
            let mut input = input_template;
            input.energy.forcing = step.forcing;
            let output = standard_lct_soil_step(input, next)?;
            on_step(step, &output)
        })
    }

    fn prepared_next_step(&self) -> Result<Option<(RuntimeClock, PointRuntimeStep)>> {
        let mut next_clock = self.clock.clone();
        let Some(clock) = next_clock.next_step() else {
            return Ok(None);
        };
        let forcing = self.forcing.runtime_at_calendar_time(
            clock.forcing_time,
            self.greenwich,
            self.longitude_degrees,
            self.latitude_degrees,
        )?;
        let calendar_day =
            orbital_calendar_day(clock.forcing_time, self.greenwich, self.longitude_degrees)?;
        // 步末的太阳天顶角：`CoLMMAIN` 用的是被 `TICKTIME` 推过一步的 `idate`。
        let surface_calendar_day =
            orbital_calendar_day(clock.end_time, self.greenwich, self.longitude_degrees)?;
        Ok(Some((
            next_clock,
            PointRuntimeStep {
                clock,
                cosine_azimuth: orbital_cosine_azimuth(
                    calendar_day,
                    self.longitude_degrees.to_radians(),
                    self.latitude_degrees.to_radians(),
                    forcing.cosine_zenith,
                ),
                surface_cosine_zenith: orbital_cosine_zenith(
                    surface_calendar_day,
                    self.longitude_degrees.to_radians(),
                    self.latitude_degrees.to_radians(),
                ),
                forcing,
            },
        )))
    }
}

/// 当步的绑定：forcing、当日秒数、greenwich 标志、经度与 CO2 体积分数。
///
/// 两支（无雪与积雪）共用它，免得各自拼一份而在某个字段上漂开。
fn lct_binding(
    step: PointRuntimeStep,
    greenwich_time: bool,
    longitude_degrees: f64,
    co2_scenario: Co2Scenario,
) -> Result<StandardLctStepBinding> {
    let (month, _) = colm_core::month_day(step.clock.forcing_time)?;
    Ok(StandardLctStepBinding {
        forcing: step.forcing,
        // `MOD_NetSolar.F90:292` 的 `local_secs = idate(3)`，`idate` 同样是步末。
        seconds_of_day: seconds_of_day(step.clock.end_time)?,
        greenwich_time,
        longitude_radians: longitude_degrees.to_radians(),
        // `MOD_Forcing` 每步按年月查 Mauna Loa 月表，再乘 1e-6 转成体积分数。
        co2_volume_fraction: colm_core::monthly_co2_ppm(
            co2_scenario,
            step.clock.forcing_time.year,
            month,
        )? * 1.0e-6,
    })
}

/// 把一步的输出打包成「准备下一步表面光学」的输入。
///
/// 三样来自步输出（`t_grnd`、`z0m`、`fwet_snow`），`coszen` 取**步末**那个
/// （[`PointRuntimeStep::surface_cosine_zenith`]），`scvold` 由调用方在**内核动手之前**
/// 读出来。上游也是这么取的：`CoLMMAIN.F90` 的末尾一节用的正是这一步 `THERMAL`
/// 刚写下的全局量，`coszen` 由 `:2076` 按步末的 `idate` 现算。
fn surface_optics_step(
    step: PointRuntimeStep,
    previous_snow_water_equivalent_mm: f64,
    output: &colm_core::StandardLctSnowSoilOutput,
) -> SurfaceOpticsStep {
    SurfaceOpticsStep {
        cosine_zenith: step.surface_cosine_zenith,
        ground_temperature_k: output.energy.ground.temperature_k[0],
        momentum_roughness_m: output.energy.leaf.momentum_roughness_m,
        wet_snow_fraction: output.energy.leaf.wet_snow_fraction,
        previous_snow_water_equivalent_mm,
    }
}

/// `MOD_NetSolar.F90:292` 的 `local_secs = idate(3)`：当日秒数，要求落在 `[0, 86400)`。
///
/// **步末的那一天末尾要进位。** 上游的 `idate` 用 `adj2end` 约定（`CoLM.F90:301`），
/// 一天的末尾写成"第二天 00:00"；而时钟交出来的 `end_time` 保留 `86400` 这个
/// "当日末尾"的写法。两者是同一时刻，但只有前者落在 `NetSolar` 的定义域里 ——
/// 直接断言会让每个跨日步都失败（实测跑一次跨月窗口就会撞上）。
fn seconds_of_day(time: CalendarTime) -> Result<i32> {
    ensure!(
        time.seconds <= 86_400,
        "the runtime clock produced {} seconds into a day; NetSolar requires [0, 86400)",
        time.seconds
    );
    Ok(if time.seconds == 86_400 {
        0
    } else {
        time.seconds as i32
    })
}

/// Reads the POINT runtime inputs from the same case and forcing namelists that
/// `read_namelist` opens in `CoLM.F90`.
pub fn read_point_runtime_config(case_namelist: impl AsRef<Path>) -> Result<PointRuntimeConfig> {
    let case_namelist = case_namelist.as_ref();
    let case = read_document(case_namelist, "case")?;
    let forcing_namelist = PathBuf::from(required_string(&case, "DEF_forcing_namelist")?);
    let forcing = read_document(&forcing_namelist, "forcing")?;
    let dataset = required_string(&forcing, "DEF_forcing%dataset")?;
    ensure!(
        dataset.eq_ignore_ascii_case("POINT"),
        "Rust PointRuntime requires DEF_forcing%dataset='POINT', got {dataset:?}"
    );
    let forcing_directory = required_string(&forcing, "DEF_dir_forcing")?;
    let forcing_name = required_string(&forcing, "DEF_forcing%fprefix(1)")?;
    // `MOD_UserSpecifiedForcing` concatenates these strings directly.
    let forcing_file = PathBuf::from(format!("{forcing_directory}{forcing_name}"));
    let (wind_height_m, temperature_height_m, humidity_height_m) =
        observation_heights(&forcing, &forcing_file)?;
    let start = simulation_date(&case, "start")?;
    // `spinup_year = 0` 在上游就是"不预热"的写法：`CoLM.F90:315` 判的是
    // `is_spinup = ststamp < ptstamp`，年份 0 永远早于真实起报时刻。
    //
    // 这类算例里 `spinup_month/day/sec` **不参与任何计算**，而它们常常留着上一个
    // 算例的值 —— 实测 `oracle/work/CN-Cng/case.nml` 是 `spinup_day = 365`，按"月内
    // 第几天"根本放不进 `u8`。所以年份为 0 时直接取 `start`，不去解析那三个字段：
    // 否则一个关掉预热的算例会因为死字段而跑不起来。
    let spinup_until = if required_integer(&case, "DEF_simulation_time%spinup_year")? == 0 {
        start
    } else {
        simulation_date(&case, "spinup")?
    };
    Ok(PointRuntimeConfig {
        start,
        end: simulation_date(&case, "end")?,
        spinup_until,
        timestep_seconds: required_real(&case, "DEF_simulation_time%timestep")?,
        spinup_repeats: usize::try_from(required_integer(
            &case,
            "DEF_simulation_time%spinup_repeat",
        )?)
        .context("DEF_simulation_time%spinup_repeat must be nonnegative")?,
        lai_update_schedule: if optional_bool_or(&case, "DEF_LAI_MONTHLY", true)? {
            LaiUpdateSchedule::Monthly
        } else {
            LaiUpdateSchedule::EightDay
        },
        restart_frequency: restart_frequency(&case)?,
        co2_scenario: co2_scenario(&case)?,
        history_frequency: history_frequency(&case)?,
        history_grouping: history_grouping(&case)?,
        greenwich: required_bool(&case, "DEF_simulation_time%greenwich")?,
        longitude_degrees: required_real(&case, "SITE_lon_location")?,
        latitude_degrees: required_real(&case, "SITE_lat_location")?,
        forcing_file,
        wind_height_m,
        temperature_height_m,
        humidity_height_m,
    })
}

/// 观测高度：文件里的优先，其次 forcing namelist，最后 schema 声明默认值。
///
/// 三步都与上游一致，见 [`PointRuntimeConfig::wind_height_m`] 的说明。
fn observation_heights(forcing: &Document, forcing_file: &Path) -> Result<(f64, f64, f64)> {
    let namelist = |field: &str| -> Result<f64> {
        match forcing.get(field) {
            Some(Value::Real { text }) => text
                .replace(['d', 'D'], "e")
                .trim_end_matches("_r8")
                .parse()
                .with_context(|| format!("{field} is not a readable real")),
            Some(Value::Int(value)) => Ok(*value as f64),
            Some(other) => bail!("{field} must be a real, got {other:?}"),
            None => match colm_schema::find(field).map(|field| &field.default) {
                Some(colm_schema::Default::Real(text)) => text
                    .replace(['d', 'D'], "e")
                    .trim_end_matches("_r8")
                    .parse()
                    .with_context(|| format!("the declared default for {field} is unreadable")),
                _ => bail!("{field} is missing and has no declared real default"),
            },
        }
    };
    // 文件**不存在**时回落到 namelist：上游总是有那个文件，但本仓库的配置解析在
    // 真实算例之外（测试、界面预览）也要能用。文件存在却读不出来仍然是错误 ——
    // 那说明路径或格式有问题，静默回落会变成"用了另一套观测高度"。
    let file = if forcing_file.is_file() {
        colm_forcing::observation_heights(forcing_file).with_context(|| {
            format!(
                "cannot read the observation heights from {}",
                forcing_file.display()
            )
        })?
    } else {
        colm_forcing::ObservationHeights::default()
    };
    Ok((
        file.wind_m.unwrap_or(namelist("DEF_forcing%HEIGHT_V")?),
        file.temperature_m
            .unwrap_or(namelist("DEF_forcing%HEIGHT_T")?),
        file.humidity_m.unwrap_or(namelist("DEF_forcing%HEIGHT_Q")?),
    ))
}

fn restart_frequency(document: &Document) -> Result<RestartFrequency> {
    match document.get("DEF_WRST_FREQ") {
        None => Ok(RestartFrequency::Never),
        Some(Value::Str(value)) => match value.trim().to_ascii_uppercase().as_str() {
            "NONE" => Ok(RestartFrequency::Never),
            "TIMESTEP" => Ok(RestartFrequency::Timestep),
            "HOURLY" => Ok(RestartFrequency::Hourly),
            "DAILY" => Ok(RestartFrequency::Daily),
            "MONTHLY" => Ok(RestartFrequency::Monthly),
            "YEARLY" => Ok(RestartFrequency::Yearly),
            other => bail!("DEF_WRST_FREQ has unsupported value {other:?}"),
        },
        Some(_) => bail!("DEF_WRST_FREQ must be a string"),
    }
}

/// `DEF_SSP` 缺省时与上游一致地取 `off`。
fn co2_scenario(document: &Document) -> Result<Co2Scenario> {
    match document.get("DEF_SSP") {
        None => Ok(Co2Scenario::Off),
        Some(Value::Str(value)) => Co2Scenario::parse(value),
        Some(_) => bail!("DEF_SSP must be a string"),
    }
}

/// `DEF_HIST_FREQ` 缺省时与上游一致取 `none`。
///
/// 取值由 `colm-hist` 解析：上游遇到不认识的取值只打一句 warning 然后静默不写，
/// 那会让用户拿到一个空目录而不知道原因；这里报错。
fn history_frequency(document: &Document) -> Result<colm_hist::schedule::HistoryFrequency> {
    match document.get("DEF_HIST_FREQ") {
        None => Ok(colm_hist::schedule::HistoryFrequency::None),
        Some(Value::Str(value)) => colm_hist::schedule::HistoryFrequency::parse(value),
        Some(_) => bail!("DEF_HIST_FREQ must be a string"),
    }
}

/// `DEF_HIST_groupby` 缺省时与上游一致取 `MONTH`。
fn history_grouping(document: &Document) -> Result<colm_hist::schedule::HistoryGrouping> {
    match document.get("DEF_HIST_groupby") {
        None => Ok(colm_hist::schedule::HistoryGrouping::Month),
        Some(Value::Str(value)) => colm_hist::schedule::HistoryGrouping::parse(value),
        Some(_) => bail!("DEF_HIST_groupby must be a string"),
    }
}

fn read_document(path: &Path, kind: &str) -> Result<Document> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {kind} namelist {}", path.display()))?;
    parse(&text).with_context(|| format!("cannot parse {kind} namelist {}", path.display()))
}

fn simulation_date(document: &Document, prefix: &str) -> Result<CalendarTime> {
    let year = i32::try_from(required_integer(
        document,
        &format!("DEF_simulation_time%{prefix}_year"),
    )?)
    .with_context(|| format!("{prefix} year is outside i32"))?;
    let month = u8::try_from(required_integer(
        document,
        &format!("DEF_simulation_time%{prefix}_month"),
    )?)
    .with_context(|| format!("{prefix} month is outside u8"))?;
    let day = u8::try_from(required_integer(
        document,
        &format!("DEF_simulation_time%{prefix}_day"),
    )?)
    .with_context(|| format!("{prefix} day is outside u8"))?;
    let seconds = u32::try_from(required_integer(
        document,
        &format!("DEF_simulation_time%{prefix}_sec"),
    )?)
    .with_context(|| format!("{prefix} second is outside u32"))?;
    Ok(CalendarTime {
        year,
        julian_day: month_day_to_julian(year, month, day)?,
        seconds,
    })
}

fn required_string(document: &Document, field: &str) -> Result<String> {
    match document.get(field) {
        Some(Value::Str(value)) => Ok(value.clone()),
        Some(_) => bail!("{field} must be a string"),
        None => bail!("namelist is missing required field {field}"),
    }
}

fn required_bool(document: &Document, field: &str) -> Result<bool> {
    match document.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
        None => bail!("namelist is missing required field {field}"),
    }
}

fn optional_bool_or(document: &Document, field: &str, default: bool) -> Result<bool> {
    match document.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
        None => Ok(default),
    }
}

fn required_integer(document: &Document, field: &str) -> Result<i64> {
    let value = required_real(document, field)?;
    ensure!(
        value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64,
        "{field} must be a finite integer"
    );
    Ok(value as i64)
}

fn required_real(document: &Document, field: &str) -> Result<f64> {
    let value = document
        .get(field)
        .and_then(Value::as_f64)
        .with_context(|| format!("namelist is missing numeric field {field}"))?;
    ensure!(value.is_finite(), "{field} must be finite");
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("colm-runtime-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_case(path: &Path, forcing: &Path, forcing_dir: &str, dataset: &str) {
        write_case_window(path, forcing, forcing_dir, dataset, 1800);
    }

    /// 与 `write_case` 相同，但窗口末秒可调 —— 多步驱动测试要跑不止一个时间步。
    fn write_case_window(
        path: &Path,
        forcing: &Path,
        forcing_dir: &str,
        dataset: &str,
        end_sec: u32,
    ) {
        write_case_span(path, forcing, forcing_dir, dataset, 1, 0, 1, 1, end_sec);
    }

    /// 起止都可跨月跨日 —— 跨分组边界的用例需要它。
    #[allow(clippy::too_many_arguments)]
    fn write_case_span(
        path: &Path,
        forcing: &Path,
        forcing_dir: &str,
        dataset: &str,
        start_day: u32,
        start_sec: u32,
        end_month: u32,
        end_day: u32,
        end_sec: u32,
    ) {
        std::fs::write(
            path,
            format!(
                "&nl_colm\n DEF_forcing_namelist='{}'\n DEF_simulation_time%start_year=2008\n DEF_simulation_time%start_month=1\n DEF_simulation_time%start_day={start_day}\n DEF_simulation_time%start_sec={start_sec}\n DEF_simulation_time%end_year=2008\n DEF_simulation_time%end_month={end_month}\n DEF_simulation_time%end_day={end_day}\n DEF_simulation_time%end_sec={end_sec}\n DEF_simulation_time%spinup_year=0\n DEF_simulation_time%spinup_month=1\n DEF_simulation_time%spinup_day=1\n DEF_simulation_time%spinup_sec=0\n DEF_simulation_time%spinup_repeat=0\n DEF_simulation_time%timestep=1800.\n DEF_simulation_time%greenwich=.false.\n DEF_LAI_MONTHLY=.true.\n DEF_WRST_FREQ='none'\n DEF_HIST_FREQ='none'\n DEF_HIST_groupby='MONTH'\n SITE_lon_location=113.0\n SITE_lat_location=23.0\n /\n",
                forcing.display()
            ),
        )
        .unwrap();
        std::fs::write(
            forcing,
            format!(
                "&nl_colm_forcing\n DEF_dir_forcing='{}'\n DEF_forcing%dataset='{}'\n DEF_forcing%fprefix(1)='CN-Cng_2008-2009_FLUXNET2015_Met.nc'\n /\n",
                forcing_dir, dataset
            ),
        )
        .unwrap();
    }

    #[test]
    fn config_uses_the_native_case_and_forcing_namelist_split() {
        let root = directory("config");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "POINT");
        let config = read_point_runtime_config(&case).unwrap();
        assert_eq!(
            config.start,
            CalendarTime {
                year: 2008,
                julian_day: 1,
                seconds: 0
            }
        );
        assert_eq!(
            config.forcing_file,
            PathBuf::from("/data/CN-Cng_2008-2009_FLUXNET2015_Met.nc")
        );
        assert_eq!(config.lai_update_schedule, LaiUpdateSchedule::Monthly);
        assert_eq!(config.restart_frequency, RestartFrequency::Never);
        assert!(!config.greenwich);
    }

    #[test]
    fn config_passes_the_eight_day_lai_cadence_to_the_shared_clock() {
        let root = directory("lai-cadence");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "POINT");
        let contents = std::fs::read_to_string(&case).unwrap();
        std::fs::write(
            &case,
            contents.replace("DEF_LAI_MONTHLY=.true.", "DEF_LAI_MONTHLY=.false."),
        )
        .unwrap();
        assert_eq!(
            read_point_runtime_config(&case)
                .unwrap()
                .lai_update_schedule,
            LaiUpdateSchedule::EightDay
        );
    }

    #[test]
    fn config_parses_the_restart_cadence_shared_with_the_clock() {
        let root = directory("restart-cadence");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "POINT");
        let contents = std::fs::read_to_string(&case).unwrap();
        std::fs::write(
            &case,
            contents.replace("DEF_WRST_FREQ='none'", "DEF_WRST_FREQ='monthly'"),
        )
        .unwrap();
        assert_eq!(
            read_point_runtime_config(&case).unwrap().restart_frequency,
            RestartFrequency::Monthly
        );
    }

    #[test]
    fn config_uses_colms_monthly_lai_default_when_the_field_is_absent() {
        let root = directory("lai-default");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "POINT");
        let contents = std::fs::read_to_string(&case).unwrap();
        std::fs::write(&case, contents.replace(" DEF_LAI_MONTHLY=.true.\n", "")).unwrap();
        assert_eq!(
            read_point_runtime_config(&case)
                .unwrap()
                .lai_update_schedule,
            LaiUpdateSchedule::Monthly
        );
    }

    #[test]
    fn point_runtime_reads_one_shared_clock_and_forcing_step() {
        let root = directory("step");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case(&case, &forcing, &source_dir, "POINT");
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let mut steps = Vec::new();
        assert_eq!(
            runtime
                .run(|step| {
                    steps.push(step);
                    Ok(())
                })
                .unwrap(),
            1
        );
        assert_eq!(steps[0].clock.index, 1);
        assert_eq!(steps[0].clock.forcing_time.seconds, 0);
        assert!(steps[0].forcing.air_temperature_k.is_finite());
        assert!(runtime.next_step().unwrap().is_none());
    }

    #[test]
    fn shared_run_loop_does_not_commit_a_failed_physics_step() {
        let root = directory("transaction");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case(&case, &forcing, &source_dir, "POINT");
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        assert!(runtime.run(|_| bail!("physics failed")).is_err());
        assert_eq!(runtime.next_step().unwrap().unwrap().clock.index, 1);
    }

    #[test]
    fn shared_state_and_clock_commit_together_after_a_successful_step() {
        let root = directory("state-transaction");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case(&case, &forcing, &source_dir, "POINT");
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let mut state = 0_u8;

        assert!(runtime
            .run_with_state(&mut state, |_, next| {
                *next += 1;
                bail!("history write failed")
            })
            .is_err());
        assert_eq!(state, 0);

        assert_eq!(
            runtime
                .run_with_state(&mut state, |_, next| {
                    *next += 1;
                    Ok(())
                })
                .unwrap(),
            1
        );
        assert_eq!(state, 1);
        assert!(runtime.next_step().unwrap().is_none());
    }

    /// 装配层测试用的那套物理参数；这里只需一份，避免第二个测试抄一遍三十多个字段。
    fn land_physics() -> crate::assembly::LandPhysicsParameters {
        crate::assembly::LandPhysicsParameters {
            hydraulic_model: colm_core::HydraulicModel::VanGenuchten,
            variably_saturated_flow: false,
            plant_hydraulics: false,
            vegetation_snow: false,
            land_cover_scheme: colm_core::LandCoverScheme::Igbp,
            root_fraction_scheme: colm_core::RootFractionScheme::SchenkJackson,
            timestep_seconds: 1800.0,
            precipitation_scheme: colm_core::PrecipitationPhaseScheme::AirTemperature,
            surface_resistance_scheme: 1,
            stress_scheme: 1,
            surface_layer_scheme: colm_core::SurfaceLayerScheme::Standard,
            thermal_conductivity_scheme: colm_core::ThermalConductivityScheme::Johansen,
            observation_height_mode: colm_core::ObservationHeightMode::Absolute,
            stomata: colm_core::StomataOptions {
                use_medlyn: true,
                use_wue: false,
                medlyn_g1_override: None,
                medlyn_g0_override: None,
                wue_lambda_override: None,
                ball_berry_slope_override: None,
                ball_berry_intercept_override: None,
            },
            soil_ice_impedance: 6.0,
            snow_irreducible_saturation: 0.033,
            impermeable_porosity: 0.05,
            ponding_limit_mm: 5.0,
            minimum_soil_potential_mm: -1.0e8,
            maximum_dew_mm: 0.1,
            maximum_transpiration_mm_s: 0.001,
            surface_temperature_factor: 0.5,
            crank_nicolson_factor: 0.5,
            soil_roughness_m: 0.01,
            snow_cover_exponent: 1.0,
            supercool_water: true,
            snow_roughness_m: 0.0024,
            wind_height_m: 30.0,
            temperature_height_m: 30.0,
            humidity_height_m: 30.0,
            vaporization_heat_j_kg: 2.5104e6,
            sprinkler_irrigation_kg_m2_s: 0.0,
            runoff_scheme: crate::assembly::StandardLctRunoffScheme::Topmodel,
            topmodel_decay_tuning: 0.1,
        }
    }

    fn assembled_template(
        fixture: &colm_init::fixtures::SyntheticRestart,
    ) -> crate::assembly::StandardLctRestartTemplate {
        crate::assembly::assemble_standard_lct_template(
            &crate::assembly::RestartStateFiles {
                constant: fixture.constant.block.clone(),
                time: fixture.time.block.clone(),
            },
            1,
            land_physics(),
        )
        .unwrap()
    }

    /// 装配好的模板 + 真实 POINT 强迫 + 三步窗口：这是整条链第一次真的跑起来。
    ///
    /// 用真实强迫而不是合成常量场，是为了让「每步的绑定真的刷新了」这句话有内容：
    /// 合成常量强迫下秒偏移与气温都不变，装配层漏刷也看不出来。
    #[test]
    fn an_assembled_restart_template_runs_several_point_steps() {
        let root = directory("restart-driver");
        let case = root.join("case.nml");
        let forcing_namelist = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        // 00:00 起、1800 s 一步、走到 01:30，即三步。
        write_case_window(&case, &forcing_namelist, &source_dir, "POINT", 5400);

        let fixture = colm_init::fixtures::SyntheticRestart::write(root.join("restart")).unwrap();
        let template = assembled_template(&fixture);
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let mut state = template.state();
        let initial = state.temperature_k.clone();
        let mut observed = Vec::new();
        let steps = runtime
            .run_restart_standard_lct(&template, &mut state, |step, output| {
                // 从 output 取，而不是从 state：state 正被这次调用可变借用。
                observed.push((
                    step.clock.index,
                    step.clock.forcing_time.seconds,
                    step.forcing.air_temperature_k,
                    output.water.total_runoff_mm_s,
                    output.energy.ground.temperature_k[0],
                ));
                Ok(())
            })
            .unwrap();

        assert_eq!(steps, 3, "the 01:30 window is three half-hour steps");
        assert_eq!(
            observed.iter().map(|entry| entry.1).collect::<Vec<_>>(),
            vec![0, 1800, 3600],
            "the per-step binding must refresh seconds-of-day"
        );
        assert_eq!(
            observed.iter().map(|entry| entry.0).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        // 真实强迫的气温逐步变化，绑定的 forcing 也就必须跟着换。
        assert!(
            observed.iter().any(|entry| entry.2 != observed[0].2),
            "the forcing never changed across the window"
        );
        assert!(observed
            .iter()
            .all(|entry| entry.3.is_finite() && entry.3 >= 0.0));
        // 三十分钟确实推动了土壤柱，且没有跑飞。
        assert!(
            state
                .temperature_k
                .iter()
                .zip(&initial)
                .all(|(now, before)| now.is_finite() && (now - before).abs() > 0.0),
            "the soil column did not move"
        );
        assert!(runtime.next_step().unwrap().is_none());
    }

    /// 输出回调失败时，状态与时钟必须一起回滚 —— 否则重试会从一个已经推进的柱子里开始。
    #[test]
    fn a_failed_output_callback_rolls_the_restart_state_back() {
        let root = directory("restart-rollback");
        let case = root.join("case.nml");
        let forcing_namelist = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case_window(&case, &forcing_namelist, &source_dir, "POINT", 5400);
        let fixture = colm_init::fixtures::SyntheticRestart::write(root.join("restart")).unwrap();
        let template = assembled_template(&fixture);
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let mut state = template.state();
        let before = state.temperature_k.clone();
        assert!(runtime
            .run_restart_standard_lct(&template, &mut state, |_, _| bail!("history failed"))
            .is_err());
        assert_eq!(
            state.temperature_k, before,
            "a failed step must not advance the soil column"
        );
        assert_eq!(runtime.next_step().unwrap().unwrap().clock.index, 1);
    }

    /// 积雪分支跑多步：与无雪那条走同一个 POINT 循环与同一份绑定。
    #[test]
    fn an_assembled_snow_template_runs_several_point_steps() {
        let root = directory("snow-driver");
        let case = root.join("case.nml");
        let forcing_namelist = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case_window(&case, &forcing_namelist, &source_dir, "POINT", 5400);
        let fixture = colm_init::fixtures::SyntheticRestart::write_with_snow(
            root.join("restart"),
            colm_init::fixtures::SyntheticSnow {
                depth_m: 0.15,
                water_equivalent_kg_m2: 45.0,
                ground_snow_fraction: 1.0,
                temperature_k: 268.0,
            },
        )
        .unwrap();
        let template = crate::assembly::assemble_standard_lct_snow_template(
            &crate::assembly::RestartStateFiles {
                constant: fixture.constant.block.clone(),
                time: fixture.time.block.clone(),
            },
            1,
            land_physics(),
        )
        .unwrap();
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let mut state = template.snow_state();
        let initial = state.snow.water_equivalent_kg_m2;
        let mut layers = Vec::new();
        // `state` 正被这次调用可变借用，所以雪层数从 output 里取。
        let steps = runtime
            .run_restart_standard_lct_snow(&template, &mut state, |step, output| {
                layers.push(step.clock.index);
                // 积雪分支的出水在 `soil` 那一半里；雪那一半给的是底部排水。
                assert!(output.water.soil.total_runoff_mm_s.is_finite());
                assert!(output.water.snow.bottom_drainage_kg_m2_s.is_finite());
                Ok(())
            })
            .unwrap();
        assert_eq!(steps, 3);
        assert_eq!(layers, vec![1, 2, 3]);
        // 雪列在三步之后仍然存在，且水量推进过。
        assert!(state.snow.layer_count <= -1);
        assert_ne!(state.snow.water_equivalent_kg_m2, initial);
        assert!(state
            .soil_temperature_k
            .iter()
            .all(|value| value.is_finite()));
        assert!(runtime.next_step().unwrap().is_none());
        // 末尾那一节（`CoLMMAIN` 的「Preparation for the next time step」）真的跑了。
        // 两条判据在装配期都**不成立**，所以漏调这一节会被抓住：
        //   * `sai = tsai*sigf` —— 夹具里 `sai = tsai = 0.5` 而 `sigf = 0.8`，对不上；
        //   * `fsno` 是 `snowfraction` 按 0.15 m 雪深现算的（≈0.96），夹具里写的是 1.0。
        let canopy = state.energy.canopy;
        assert!(
            (canopy.stem_area_index
                - template.temporal_stem_area_index * canopy.vegetation_free_fraction)
                .abs()
                < 1.0e-12,
            "the canopy geometry was not recomputed from tsai and sigf"
        );
        assert!(canopy.vegetation_free_fraction < 1.0);
        assert!((0.5..1.0).contains(&state.snow.ground_snow_fraction));
        // 雪龄也要跟着走：三步里至少有一个白天步（`coszen > -0.3`）。
        assert!(state.snow.age > 0.0);
    }

    /// 配置里的窗口，供需要"比运行更长"的调度时改写。
    fn window_of(config: &PointRuntimeConfig) -> colm_hist::schedule::SimulationWindow {
        colm_hist::schedule::SimulationWindow {
            start_year: config.start.year,
            start_julian_day: i32::from(config.start.julian_day),
            start_seconds: config.start.seconds as i32,
            end_year: config.end.year,
            end_julian_day: i32::from(config.end.julian_day),
            end_seconds: config.end.seconds as i32,
            timestep_seconds: config.timestep_seconds as i32,
        }
    }

    /// 把算例的 history 频率/分组改掉 —— 默认是 `none`，不写 history。
    fn with_history(case: &Path, frequency: &str, grouping: &str) {
        let contents = std::fs::read_to_string(case).unwrap();
        std::fs::write(
            case,
            contents
                .replace(
                    "DEF_HIST_FREQ='none'",
                    &format!("DEF_HIST_FREQ='{frequency}'"),
                )
                .replace(
                    "DEF_HIST_groupby='MONTH'",
                    &format!("DEF_HIST_groupby='{grouping}'"),
                ),
        )
        .unwrap();
    }

    /// 带 history 的运行：一次真实的三小时窗口，产出按调度分组的一个文件。
    #[test]
    fn a_run_with_history_writes_the_scheduled_records() {
        let root = directory("history-run");
        let case = root.join("case.nml");
        let forcing_namelist = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        // 00:00 → 03:00，1800 s 一步：六步，HOURLY 下三条记录。
        write_case_window(&case, &forcing_namelist, &source_dir, "POINT", 3 * 3600);
        with_history(&case, "hourly", "MONTH");
        let fixture = colm_init::fixtures::SyntheticRestart::write(root.join("restart")).unwrap();
        let template = assembled_template(&fixture);
        let config = read_point_runtime_config(&case).unwrap();
        assert_eq!(
            config.history_frequency,
            colm_hist::schedule::HistoryFrequency::Hourly
        );
        assert_eq!(
            config.history_grouping,
            colm_hist::schedule::HistoryGrouping::Month
        );
        let mut session = config.history_session(root.join("out"), "CN-Cng").unwrap();

        let mut runtime = PointRuntime::open(config).unwrap();
        let mut state = template.state();
        let outcome = runtime
            .run_restart_standard_lct_with_history(&template, &mut state, &mut session, |_, _| {
                Ok(())
            })
            .unwrap();
        assert_eq!(outcome.steps, 6);
        assert_eq!(outcome.files.len(), 1);
        assert_eq!(
            outcome.files[0].file_name().unwrap(),
            "CN-Cng_hist_2008-01.nc"
        );
        let file = netcdf::open(&outcome.files[0]).unwrap();
        let times = file
            .variable("time")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap();
        // 三条记录，标签与黄金文件的头三个值逐位相同。
        assert_eq!(times, vec![56_802_270, 56_802_330, 56_802_390]);
        assert_eq!(session.remaining(), 0);
    }

    /// 跨分组边界：换月时把上一个文件落盘，再开新文件。
    ///
    /// 这是多文件路径唯一的证据 —— 单月窗口只走"最后一个分组收尾"，不走"中途换分组"。
    #[test]
    fn a_run_crossing_a_month_boundary_writes_two_history_files() {
        let root = directory("history-months");
        let case = root.join("case.nml");
        let forcing_namelist = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        // 1 月 31 日 22:00 → 2 月 1 日 02:00（四小时、八步），跨过月份边界。
        write_case_span(
            &case,
            &forcing_namelist,
            &source_dir,
            "POINT",
            31,
            22 * 3600,
            2,
            1,
            2 * 3600,
        );
        with_history(&case, "hourly", "MONTH");
        let fixture = colm_init::fixtures::SyntheticRestart::write(root.join("restart")).unwrap();
        let template = assembled_template(&fixture);
        let config = read_point_runtime_config(&case).unwrap();
        assert_eq!(
            config.start.julian_day, 31,
            "22:00 on Jan 31 is julian day 31"
        );
        let mut session = config.history_session(root.join("out"), "CN-Cng").unwrap();
        let mut runtime = PointRuntime::open(config).unwrap();
        let mut state = template.state();
        let outcome = runtime
            .run_restart_standard_lct_with_history(&template, &mut state, &mut session, |_, _| {
                Ok(())
            })
            .unwrap();
        assert_eq!(outcome.steps, 8);
        // 分组由**写入时刻**的日期决定，不是它覆盖的那个区间：写于 1 月 31 日 23:00 的
        // 那条落在一月，写于 2 月 1 日 00:00 的那条已经落在二月。所以一月只有一条、
        // 二月有三条 —— 边界正好在午夜。
        let mut names = outcome
            .files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            vec!["CN-Cng_hist_2008-01.nc", "CN-Cng_hist_2008-02.nc"]
        );
        let january = netcdf::open(
            outcome
                .files
                .iter()
                .find(|path| path.to_string_lossy().contains("2008-01"))
                .unwrap(),
        )
        .unwrap();
        let times = january
            .variable("time")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap();
        assert_eq!(times.len(), 1, "January keeps only the 23:00 write");
        // 写于 23:00 的记录标签是 22:30。
        let february_after_one = times[0];
        assert_eq!(february_after_one % 60, 30);
        let february = netcdf::open(
            outcome
                .files
                .iter()
                .find(|path| path.to_string_lossy().contains("2008-02"))
                .unwrap(),
        )
        .unwrap();
        let times = february
            .variable("time")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap();
        assert_eq!(times.len(), 3, "February takes 00:00, 01:00 and 02:00");
        assert_eq!(times[1] - times[0], 60);
        assert_eq!(times[2] - times[1], 60);
        // 相邻两条跨月但连续：一月的最后一条比二月的第一条早一小时。
        assert_eq!(times[0] - february_after_one, 60);
        assert_eq!(session.remaining(), 0);
    }

    /// 运行比窗口短时必须在落盘前报错 —— 否则会写出一个满是零的记录文件，而它读起来
    /// 与真实数据没有区别。
    #[test]
    fn a_run_that_stops_short_of_the_schedule_is_refused() {
        let root = directory("history-short");
        let case = root.join("case.nml");
        let forcing_namelist = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case_window(&case, &forcing_namelist, &source_dir, "POINT", 1800);
        with_history(&case, "hourly", "MONTH");
        let fixture = colm_init::fixtures::SyntheticRestart::write(root.join("restart")).unwrap();
        let template = assembled_template(&fixture);
        let config = read_point_runtime_config(&case).unwrap();
        // 算例只走一步，但调度按三小时开 —— 运行结束时有未写的记录，必须报错。
        let window = colm_hist::schedule::SimulationWindow {
            end_seconds: 3 * 3600,
            ..window_of(&config)
        };
        let mut session = crate::history::HistorySession::new(
            crate::history::point_dimensions(),
            colm_hist::history::HistorySite {
                latitude_degrees: config.latitude_degrees,
                longitude_degrees: config.longitude_degrees,
            },
            window,
            config.history_frequency,
            config.history_grouping,
            root.join("out"),
            "CN-Cng",
        )
        .unwrap();
        let mut runtime = PointRuntime::open(config).unwrap();
        let mut state = template.state();
        let error = runtime
            .run_restart_standard_lct_with_history(&template, &mut state, &mut session, |_, _| {
                Ok(())
            })
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("still unwritten"),
            "{error:#}"
        );
    }

    /// `DEF_SSP` 决定 2022 年之后用哪张 CO2 表；缺省与上游一致是 `off`。
    #[test]
    fn the_case_namelist_selects_the_co2_scenario() {
        let root = directory("co2-scenario");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "POINT");
        assert_eq!(
            read_point_runtime_config(&case).unwrap().co2_scenario,
            Co2Scenario::Off
        );
        let contents = std::fs::read_to_string(&case).unwrap();
        std::fs::write(
            &case,
            contents.replace(
                "DEF_WRST_FREQ='none'",
                "DEF_SSP='585'\n DEF_WRST_FREQ='none'",
            ),
        )
        .unwrap();
        assert_eq!(
            read_point_runtime_config(&case).unwrap().co2_scenario,
            Co2Scenario::Ssp585
        );
        // 上游在 `CASE DEFAULT` 里直接停；静默退回 off 会让一份 SSP 算例拿到观测段的 CO2。
        std::fs::write(
            &case,
            contents.replace(
                "DEF_WRST_FREQ='none'",
                "DEF_SSP='999'\n DEF_WRST_FREQ='none'",
            ),
        )
        .unwrap();
        assert!(read_point_runtime_config(&case).is_err());
    }

    #[test]
    fn runtime_rejects_non_point_forcing_before_opening_a_file() {
        let root = directory("dataset");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "CRUNCEP");
        assert!(read_point_runtime_config(&case).is_err());
    }

    #[test]
    fn downscaled_loop_commits_one_shared_column_forcing_record() {
        let root = directory("downscaled-step");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case(&case, &forcing, &source_dir, "POINT");
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let slope = [0.0; colm_core::ASPECT_TYPES];
        let area = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let mut steps = Vec::new();
        assert_eq!(
            runtime
                .run_downscaled(
                    PointDownscalingTemplate {
                        grid_surface_elevation_m: 500.0,
                        grid_maximum_elevation_m: 2_500.0,
                        reference_height_m: 30.0,
                        column_surface_elevation_m: 800.0,
                        glacier: false,
                        terrain: DownscalingTerrain::Simple(colm_core::SimpleTerrain {
                            slope_tangent: &slope,
                            area_fraction: &area,
                        }),
                        config: ForcingDownscalingConfig::default(),
                    },
                    |step| {
                        steps.push(step);
                        Ok(())
                    },
                )
                .unwrap(),
            1
        );
        assert_eq!(steps[0].clock.index, 1);
        assert!(steps[0].grid_forcing.cosine_zenith.is_finite());
        assert!(steps[0].forcing.air_temperature_k.is_finite());
        assert!(steps[0].forcing.bottom_pressure_pa < steps[0].grid_forcing.bottom_pressure_pa);
        assert!(steps[0].forcing.air_temperature_k < steps[0].grid_forcing.air_temperature_k);
        assert!(runtime.next_step().unwrap().is_none());
    }

    /// 一步里有**两个**太阳天顶角，不能混：
    ///
    /// * `MOD_Forcing` 在 `TICKTIME` 之前按**步首**算的（`MOD_Forcing.F90:752`），
    ///   用于短波直散拆分与地形降尺度 —— 就是 `forcing.cosine_zenith`；
    /// * `CoLMMAIN.F90:2076` 在 `TICKTIME` 之后按**步末** `idate` 现算的那个，
    ///   `albland` 与重启的 `coszen` 用它 —— 就是 `surface_cosine_zenith`。
    ///
    /// 本仓库原先只有一个，两处都用步首那一份。实测 CN-Cng 第 1 天正午两个值
    /// 是 0.379821 与 0.374192，差半个步长的太阳时角；用它算出的 `extkb`
    /// 也跟着差 0.026。
    #[test]
    fn the_surface_cosine_zenith_uses_the_end_of_the_step() {
        let root = directory("surface-zenith");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case(&case, &forcing, &source_dir, "POINT");
        let config = read_point_runtime_config(&case).unwrap();
        let (longitude_degrees, latitude_degrees, greenwich) = (
            config.longitude_degrees,
            config.latitude_degrees,
            config.greenwich,
        );
        let mut runtime = PointRuntime::open(config).unwrap();
        let step = runtime.next_step().unwrap().unwrap();
        let expected = orbital_cosine_zenith(
            orbital_calendar_day(step.clock.end_time, greenwich, longitude_degrees).unwrap(),
            longitude_degrees.to_radians(),
            latitude_degrees.to_radians(),
        );
        assert!((step.surface_cosine_zenith - expected).abs() < 1.0e-15);
        // 两个时刻确实不同：一个 1800 秒的窗口里两者就不会相等。
        assert!((step.surface_cosine_zenith - step.forcing.cosine_zenith).abs() > 1.0e-6);
    }

    /// 当日末尾（`seconds == 86400`）要进位成第二天 00:00 再交给 `NetSolar`。
    ///
    /// 上游的 `idate` 用 `adj2end` 约定，而时钟交出来的 `end_time` 保留 86400
    /// 这个"当日末尾"的写法 —— 直接断言会让每个跨日步都失败。
    #[test]
    fn the_seconds_of_day_roll_over_at_the_end_of_a_day() {
        assert_eq!(
            seconds_of_day(CalendarTime {
                year: 2008,
                julian_day: 1,
                seconds: 86_400,
            })
            .unwrap(),
            0
        );
        assert_eq!(
            seconds_of_day(CalendarTime {
                year: 2008,
                julian_day: 1,
                seconds: 43_200,
            })
            .unwrap(),
            43_200
        );
        assert!(seconds_of_day(CalendarTime {
            year: 2008,
            julian_day: 1,
            seconds: 86_401,
        })
        .is_err());
    }

    #[test]
    fn downscaled_state_and_clock_rollback_together() {
        let root = directory("downscaled-transaction");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/Forcing");
        let source_dir = format!("{}/", source.display());
        write_case(&case, &forcing, &source_dir, "POINT");
        let mut runtime = PointRuntime::open(read_point_runtime_config(&case).unwrap()).unwrap();
        let slope = [0.0; colm_core::ASPECT_TYPES];
        let area = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let template = PointDownscalingTemplate {
            grid_surface_elevation_m: 500.0,
            grid_maximum_elevation_m: 2_500.0,
            reference_height_m: 30.0,
            column_surface_elevation_m: 800.0,
            glacier: false,
            terrain: DownscalingTerrain::Simple(colm_core::SimpleTerrain {
                slope_tangent: &slope,
                area_fraction: &area,
            }),
            config: ForcingDownscalingConfig::default(),
        };
        let mut state = 0_u8;
        assert!(runtime
            .run_downscaled_with_state(template, &mut state, |_, next| {
                *next += 1;
                bail!("history write failed")
            })
            .is_err());
        assert_eq!(state, 0);
        assert_eq!(
            runtime
                .run_downscaled_with_state(template, &mut state, |_, next| {
                    *next += 1;
                    Ok(())
                })
                .unwrap(),
            1
        );
        assert_eq!(state, 1);
        assert!(runtime.next_step().unwrap().is_none());
    }
}
