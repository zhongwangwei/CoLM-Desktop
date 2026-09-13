//! Rust runtime orchestration for CoLM.
//!
//! This crate owns executable-stage coordination only.  Numerical kernels stay
//! in `colm-core`, NetCDF POINT forcing stays in `colm-forcing`, and restart
//! serialization stays in `colm-init`; that keeps `colm-init` from becoming a
//! second copy of `colm.x`.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    month_day_to_julian, standard_lct_soil_step, CalendarTime, LaiUpdateSchedule, RestartFrequency,
    RuntimeClock, RuntimeForcing, RuntimeStep, StandardLctSoilInput, StandardLctSoilOutput,
    StandardLctSoilState,
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
    pub greenwich: bool,
    pub longitude_degrees: f64,
    pub latitude_degrees: f64,
    pub forcing_file: PathBuf,
}

/// One fully prepared POINT forcing record for a `CoLM.F90` loop pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointRuntimeStep {
    pub clock: RuntimeStep,
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
        Ok(Some((next_clock, PointRuntimeStep { clock, forcing })))
    }
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
    Ok(PointRuntimeConfig {
        start: simulation_date(&case, "start")?,
        end: simulation_date(&case, "end")?,
        spinup_until: simulation_date(&case, "spinup")?,
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
        greenwich: required_bool(&case, "DEF_simulation_time%greenwich")?,
        longitude_degrees: required_real(&case, "SITE_lon_location")?,
        latitude_degrees: required_real(&case, "SITE_lat_location")?,
        // `MOD_UserSpecifiedForcing` concatenates these strings directly.
        forcing_file: PathBuf::from(format!("{forcing_directory}{forcing_name}")),
    })
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
        std::fs::write(
            path,
            format!(
                "&nl_colm\n DEF_forcing_namelist='{}'\n DEF_simulation_time%start_year=2008\n DEF_simulation_time%start_month=1\n DEF_simulation_time%start_day=1\n DEF_simulation_time%start_sec=0\n DEF_simulation_time%end_year=2008\n DEF_simulation_time%end_month=1\n DEF_simulation_time%end_day=1\n DEF_simulation_time%end_sec=1800\n DEF_simulation_time%spinup_year=0\n DEF_simulation_time%spinup_month=1\n DEF_simulation_time%spinup_day=1\n DEF_simulation_time%spinup_sec=0\n DEF_simulation_time%spinup_repeat=0\n DEF_simulation_time%timestep=1800.\n DEF_simulation_time%greenwich=.false.\n DEF_LAI_MONTHLY=.true.\n DEF_WRST_FREQ='none'\n SITE_lon_location=113.0\n SITE_lat_location=23.0\n /\n",
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

    #[test]
    fn runtime_rejects_non_point_forcing_before_opening_a_file() {
        let root = directory("dataset");
        let case = root.join("case.nml");
        let forcing = root.join("forcing.nml");
        write_case(&case, &forcing, "/data/", "CRUNCEP");
        assert!(read_point_runtime_config(&case).is_err());
    }
}
