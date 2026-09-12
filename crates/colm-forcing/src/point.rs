//! In-memory POINT forcing for the native Rust runtime.
//!
//! CoLM's Fortran POINT reader opens the same NetCDF variables while stepping.
//! The Rust path reads each validated point series once, which both gives the
//! numerical driver typed values and avoids concurrent HDF5 reads across
//! patches or worker threads.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

use crate::{canonical_units, check, resolve, summarize, MetSummary};

/// One canonical forcing record consumed by a Rust surface step.
///
/// Units are K, kg kg⁻¹, Pa, kg m⁻² s⁻¹, m s⁻¹, and W m⁻² respectively.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointForcingFrame {
    pub time_seconds: f64,
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub precipitation_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    /// For scalar-wind files this is the speed, matching CoLM slot six.
    pub northward_or_scalar_wind_m_s: f64,
    pub downward_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
}

/// Fully preloaded one-point forcing data.
#[derive(Debug, Clone)]
pub struct PointForcingSeries {
    summary: MetSummary,
    frames: Vec<PointForcingFrame>,
    wind_is_vector: bool,
}

impl PointForcingSeries {
    pub fn summary(&self) -> &MetSummary {
        &self.summary
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn wind_is_vector(&self) -> bool {
        self.wind_is_vector
    }

    /// Returns the exact record at `index`; interpolation belongs to the
    /// simulation clock, where the requested model instant is known.
    pub fn frame(&self, index: usize) -> Result<PointForcingFrame> {
        self.frames
            .get(index)
            .copied()
            .with_context(|| format!("forcing record {index} is outside 0..{}", self.len()))
    }

    /// Samples the Point record at an elapsed forcing second.
    ///
    /// This preserves `MOD_Forcing:read_forcing`: all continuous slots use
    /// linear lower/upper interpolation; precipitation is nearest-neighbour
    /// and a midpoint tie selects the lower record.
    pub fn sample_at_seconds(&self, time_seconds: f64) -> Result<PointForcingFrame> {
        ensure!(
            time_seconds.is_finite(),
            "forcing sample time must be finite"
        );
        let first = self.frames.first().context("forcing series is empty")?;
        let last = self.frames.last().context("forcing series is empty")?;
        ensure!(
            time_seconds >= first.time_seconds && time_seconds <= last.time_seconds,
            "forcing sample time {time_seconds} is outside {}..={}",
            first.time_seconds,
            last.time_seconds
        );
        let upper = self
            .frames
            .partition_point(|frame| frame.time_seconds < time_seconds);
        if upper == 0 {
            return Ok(*first);
        }
        if upper == self.frames.len() {
            return Ok(*last);
        }
        let lower = self.frames[upper - 1];
        let upper = self.frames[upper];
        let lower_weight =
            (upper.time_seconds - time_seconds) / (upper.time_seconds - lower.time_seconds);
        let upper_weight = 1.0 - lower_weight;
        Ok(PointForcingFrame {
            time_seconds,
            air_temperature_k: linear(
                lower.air_temperature_k,
                upper.air_temperature_k,
                lower_weight,
                upper_weight,
            ),
            specific_humidity: linear(
                lower.specific_humidity,
                upper.specific_humidity,
                lower_weight,
                upper_weight,
            ),
            surface_pressure_pa: linear(
                lower.surface_pressure_pa,
                upper.surface_pressure_pa,
                lower_weight,
                upper_weight,
            ),
            precipitation_kg_m2_s: if lower_weight >= upper_weight {
                lower.precipitation_kg_m2_s
            } else {
                upper.precipitation_kg_m2_s
            },
            eastward_wind_m_s: linear(
                lower.eastward_wind_m_s,
                upper.eastward_wind_m_s,
                lower_weight,
                upper_weight,
            ),
            northward_or_scalar_wind_m_s: linear(
                lower.northward_or_scalar_wind_m_s,
                upper.northward_or_scalar_wind_m_s,
                lower_weight,
                upper_weight,
            ),
            downward_shortwave_w_m2: linear(
                lower.downward_shortwave_w_m2,
                upper.downward_shortwave_w_m2,
                lower_weight,
                upper_weight,
            ),
            downward_longwave_w_m2: linear(
                lower.downward_longwave_w_m2,
                upper.downward_longwave_w_m2,
                lower_weight,
                upper_weight,
            ),
        })
    }
}

/// Loads and canonicalizes a validated NetCDF POINT forcing file.
///
/// This deliberately rejects spatial fields: a grid reader must select an
/// explicit patch/block rather than silently taking its first cell.
pub fn load_point_forcing(path: impl AsRef<Path>) -> Result<PointForcingSeries> {
    let path = path.as_ref();
    let summary = summarize(path)?;
    let problems = check(&summary, None);
    ensure!(
        problems.is_empty(),
        "{} is not a usable CoLM POINT forcing file: {}",
        path.display(),
        problems.join("; ")
    );
    let (resolved, missing) = resolve(&summary.variables);
    ensure!(
        missing.is_empty(),
        "{} has unresolved forcing slots: {}",
        path.display(),
        missing.join("; ")
    );
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let time = values(&file, path, "time", summary.steps)?;
    let temperature = slot_values(
        &file,
        path,
        resolved.vname[0],
        1,
        summary.steps,
        summary.step_seconds,
    )?;
    let pressure = slot_values(
        &file,
        path,
        resolved.vname[2],
        3,
        summary.steps,
        summary.step_seconds,
    )?;
    let humidity = humidity_values(
        &file,
        path,
        resolved.vname[1].expect("resolved required humidity slot"),
        &temperature,
        &pressure,
        summary.step_seconds,
    )?;
    let precipitation = slot_values(
        &file,
        path,
        resolved.vname[3],
        4,
        summary.steps,
        summary.step_seconds,
    )?;
    let eastward_wind = match resolved.vname[4] {
        Some(name) => slot_values(
            &file,
            path,
            Some(name),
            5,
            summary.steps,
            summary.step_seconds,
        )?,
        None => vec![0.0; summary.steps],
    };
    let northward_or_scalar_wind = slot_values(
        &file,
        path,
        resolved.vname[5],
        6,
        summary.steps,
        summary.step_seconds,
    )?;
    let shortwave = slot_values(
        &file,
        path,
        resolved.vname[6],
        7,
        summary.steps,
        summary.step_seconds,
    )?;
    let longwave = slot_values(
        &file,
        path,
        resolved.vname[7],
        8,
        summary.steps,
        summary.step_seconds,
    )?;
    let mut frames = Vec::with_capacity(summary.steps);
    for index in 0..summary.steps {
        let frame = PointForcingFrame {
            time_seconds: time[index],
            air_temperature_k: temperature[index],
            specific_humidity: humidity[index],
            surface_pressure_pa: pressure[index],
            precipitation_kg_m2_s: precipitation[index],
            eastward_wind_m_s: eastward_wind[index],
            northward_or_scalar_wind_m_s: northward_or_scalar_wind[index],
            downward_shortwave_w_m2: shortwave[index],
            downward_longwave_w_m2: longwave[index],
        };
        ensure!(
            frame_values(frame).iter().all(|value| value.is_finite()),
            "{} has a missing or non-finite forcing value at record {index}",
            path.display()
        );
        frames.push(frame);
    }
    Ok(PointForcingSeries {
        summary,
        frames,
        wind_is_vector: resolved.wind_is_vector(),
    })
}

fn slot_values(
    file: &netcdf::File,
    path: &Path,
    name: Option<&str>,
    index: usize,
    steps: usize,
    step_seconds: f64,
) -> Result<Vec<f64>> {
    let name = name.with_context(|| format!("required forcing slot {index} is NULL"))?;
    let raw = values(file, path, name, steps)?;
    let units = units(file, name)?;
    crate::units::convert_units_with_step(&units, canonical_units(index), &raw, Some(step_seconds))
}

fn humidity_values(
    file: &netcdf::File,
    path: &Path,
    name: &str,
    temperature: &[f64],
    pressure: &[f64],
    step_seconds: f64,
) -> Result<Vec<f64>> {
    let raw = values(file, path, name, temperature.len())?;
    let units = units(file, name)?;
    match crate::units::humidity_to_specific(name, &units, &raw, temperature, pressure)? {
        Some(values) => Ok(values),
        None => crate::units::convert_units_with_step(
            &units,
            canonical_units(2),
            &raw,
            Some(step_seconds),
        ),
    }
}

fn values(file: &netcdf::File, path: &Path, name: &str, steps: usize) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions
            .first()
            .is_some_and(|dimension| dimension.name() == "time")
            && dimensions
                .iter()
                .skip(1)
                .all(|dimension| dimension.len() == 1),
        "{name} in {} is not a one-point time series",
        path.display()
    );
    let mut output: Vec<f64> = variable
        .get_values(netcdf::Extents::All)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    ensure!(
        output.len() == steps,
        "{name} in {} has {} values, expected {steps}",
        path.display(),
        output.len()
    );
    crate::gapfill::normalize_declared_missing(file, name, &mut output);
    Ok(output)
}

fn units(file: &netcdf::File, name: &str) -> Result<String> {
    let variable = file
        .variable(name)
        .with_context(|| format!("no variable {name}"))?;
    match variable
        .attribute("units")
        .with_context(|| format!("{name} has no units attribute"))?
        .value()?
    {
        netcdf::AttributeValue::Str(value) => Ok(value),
        netcdf::AttributeValue::Strs(mut values) if values.len() == 1 => Ok(values.remove(0)),
        value => bail!("{name} has a non-string units attribute {value:?}"),
    }
}

fn frame_values(frame: PointForcingFrame) -> [f64; 9] {
    [
        frame.time_seconds,
        frame.air_temperature_k,
        frame.specific_humidity,
        frame.surface_pressure_pa,
        frame.precipitation_kg_m2_s,
        frame.eastward_wind_m_s,
        frame.northward_or_scalar_wind_m_s,
        frame.downward_shortwave_w_m2,
        frame.downward_longwave_w_m2,
    ]
}

fn linear(lower: f64, upper: f64, lower_weight: f64, upper_weight: f64) -> f64 {
    lower * lower_weight + upper * upper_weight
}

#[cfg(test)]
#[path = "point_tests.rs"]
mod point_tests;
