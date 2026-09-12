//! Point readers for CoLM's gridded initialization datasets.
//!
//! The upstream initializer builds an areal mapping from these coordinate
//! centers to a patch.  A single-point surface has one geographic sample, so
//! this adapter reads the nearest source-cell center without materializing a
//! global field.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

/// Soil, water-table, and snow state selected for one calendar month.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSoilProfile {
    pub depth_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    pub wetness: Vec<f64>,
    pub water_table_m: f64,
    pub snow_depth_m: Option<f64>,
    /// False when CoLM's `zwt:missing_value` marks this source cell invalid.
    pub valid: bool,
}

/// Read `soilstate.nc` at a single surface coordinate and one-based month.
pub fn read_single_point_soil_profile(
    path: impl AsRef<Path>,
    latitude_degrees: f64,
    longitude_degrees: f64,
    month: u8,
) -> Result<RuntimeSoilProfile> {
    ensure!((1..=12).contains(&month), "runtime month must be in 1..=12");
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open soil initial state {}", path.display()))?;
    let (latitude, longitude) = cell_indices(&file, latitude_degrees, longitude_degrees)?;
    let depth_m = values_1d(&file, "soildepth")?;
    ensure!(!depth_m.is_empty(), "soildepth must not be empty");
    let month = usize::from(month - 1);
    let temperature_k = profile_4d(&file, "soiltemp", month, latitude, longitude, depth_m.len())?;
    let wetness = profile_4d(&file, "soilwat", month, latitude, longitude, depth_m.len())?;
    let water_table_m = scalar_3d(&file, "zwt", month, latitude, longitude)?;
    let snow_depth_m = optional_scalar_3d(&file, "snowdepth", month, latitude, longitude)?;
    let valid = valid_value(&file, "zwt", water_table_m);
    if valid {
        validate_values("soildepth", &depth_m)?;
        validate_variable_values(&file, "soiltemp", &temperature_k)?;
        validate_variable_values(&file, "soilwat", &wetness)?;
    }
    Ok(RuntimeSoilProfile {
        depth_m,
        temperature_k,
        wetness,
        water_table_m,
        snow_depth_m,
        valid,
    })
}

/// Read the standalone monthly `wtd.nc` source at a single surface coordinate.
pub fn read_single_point_water_table(
    path: impl AsRef<Path>,
    latitude_degrees: f64,
    longitude_degrees: f64,
    month: u8,
) -> Result<Option<f64>> {
    ensure!((1..=12).contains(&month), "runtime month must be in 1..=12");
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open water-table initial state {}", path.display()))?;
    let (latitude, longitude) = cell_indices(&file, latitude_degrees, longitude_degrees)?;
    let value = scalar_3d(&file, "wtd", usize::from(month - 1), latitude, longitude)?;
    Ok(valid_value(&file, "wtd", value).then_some(value))
}

/// Read an optional standalone monthly `snowdepth` source.
pub fn read_single_point_snow_depth(
    path: impl AsRef<Path>,
    latitude_degrees: f64,
    longitude_degrees: f64,
    month: u8,
) -> Result<Option<f64>> {
    ensure!((1..=12).contains(&month), "runtime month must be in 1..=12");
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open snow initial state {}", path.display()))?;
    let (latitude, longitude) = cell_indices(&file, latitude_degrees, longitude_degrees)?;
    optional_scalar_3d(
        &file,
        "snowdepth",
        usize::from(month - 1),
        latitude,
        longitude,
    )
}

fn cell_indices(file: &netcdf::File, latitude: f64, longitude: f64) -> Result<(usize, usize)> {
    ensure!(
        latitude.is_finite() && longitude.is_finite(),
        "surface coordinate must be finite"
    );
    let latitudes = values_1d(file, "lat")?;
    let longitudes = values_1d(file, "lon")?;
    Ok((
        nearest_index(&latitudes, latitude, false)?,
        nearest_index(&longitudes, longitude, true)?,
    ))
}

fn nearest_index(values: &[f64], target: f64, longitude: bool) -> Result<usize> {
    ensure!(!values.is_empty(), "runtime coordinate must not be empty");
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            ensure!(
                value.is_finite(),
                "runtime coordinate contains a non-finite value"
            );
            let distance = if longitude {
                let raw = (target - value).rem_euclid(360.0);
                raw.min(360.0 - raw)
            } else {
                (target - value).abs()
            };
            Ok((index, distance))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| index)
        .context("runtime coordinate must not be empty")
}

fn values_1d(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("runtime file has no {name}"))?;
    ensure!(
        variable.dimensions().len() == 1,
        "runtime {name} must be one-dimensional"
    );
    Ok(variable.get_values::<f64, _>(..)?)
}

fn profile_4d(
    file: &netcdf::File,
    name: &str,
    month: usize,
    latitude: usize,
    longitude: usize,
    layers: usize,
) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("runtime file has no {name}"))?;
    require_dimensions(&variable, name, &["month", "lat", "lon", "layer"])?;
    ensure!(
        variable.dimensions()[3].len() == layers,
        "runtime {name} layer count differs from soildepth"
    );
    Ok(variable.get_values::<f64, _>((
        month..month + 1,
        latitude..latitude + 1,
        longitude..longitude + 1,
        0..layers,
    ))?)
}

fn scalar_3d(
    file: &netcdf::File,
    name: &str,
    month: usize,
    latitude: usize,
    longitude: usize,
) -> Result<f64> {
    let variable = file
        .variable(name)
        .with_context(|| format!("runtime file has no {name}"))?;
    let first = variable
        .dimensions()
        .first()
        .map(|dimension| dimension.name());
    ensure!(
        matches!(first.as_deref(), Some("month") | Some("time")),
        "runtime {name} must start with month or time"
    );
    require_dimensions_tail(&variable, name, &["lat", "lon"])?;
    let values = variable.get_values::<f64, _>((
        month..month + 1,
        latitude..latitude + 1,
        longitude..longitude + 1,
    ))?;
    values
        .into_iter()
        .next()
        .context("runtime scalar selection returned no value")
}

fn optional_scalar_3d(
    file: &netcdf::File,
    name: &str,
    month: usize,
    latitude: usize,
    longitude: usize,
) -> Result<Option<f64>> {
    let Some(variable) = file.variable(name) else {
        return Ok(None);
    };
    let first = variable
        .dimensions()
        .first()
        .map(|dimension| dimension.name());
    ensure!(
        matches!(first.as_deref(), Some("month") | Some("time")),
        "runtime {name} must start with month or time"
    );
    require_dimensions_tail(&variable, name, &["lat", "lon"])?;
    let value = variable
        .get_values::<f64, _>((
            month..month + 1,
            latitude..latitude + 1,
            longitude..longitude + 1,
        ))?
        .into_iter()
        .next()
        .context("runtime scalar selection returned no value")?;
    Ok(valid_value(file, name, value).then_some(value))
}

fn require_dimensions(
    variable: &netcdf::Variable<'_>,
    name: &str,
    expected: &[&str],
) -> Result<()> {
    let actual = variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    ensure!(
        actual == expected,
        "runtime {name} dimensions are {actual:?}, expected {expected:?}"
    );
    Ok(())
}

fn require_dimensions_tail(
    variable: &netcdf::Variable<'_>,
    name: &str,
    tail: &[&str],
) -> Result<()> {
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == tail.len() + 1
            && dimensions[1..]
                .iter()
                .map(|dimension| dimension.name())
                .eq(tail.iter().map(|name| (*name).to_owned())),
        "runtime {name} must have time, lat, lon dimensions"
    );
    Ok(())
}

fn valid_value(file: &netcdf::File, name: &str, value: f64) -> bool {
    value.is_finite()
        && !["missing_value", "_FillValue"]
            .into_iter()
            .any(|attribute| {
                file.variable(name)
                    .and_then(|variable| variable.attribute_value(attribute))
                    .and_then(Result::ok)
                    .and_then(numeric_attribute)
                    .is_some_and(|missing| value == missing)
            })
}

fn numeric_attribute(value: netcdf::AttributeValue) -> Option<f64> {
    match value {
        netcdf::AttributeValue::Double(value) => Some(value),
        netcdf::AttributeValue::Float(value) => Some(f64::from(value)),
        netcdf::AttributeValue::Int(value) => Some(f64::from(value)),
        netcdf::AttributeValue::Short(value) => Some(f64::from(value)),
        _ => None,
    }
}

fn validate_values(name: &str, values: &[f64]) -> Result<()> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        bail!("runtime {name} contains a non-finite value for a valid water-table cell")
    }
}

fn validate_variable_values(file: &netcdf::File, name: &str, values: &[f64]) -> Result<()> {
    if values.iter().all(|&value| valid_value(file, name, value)) {
        Ok(())
    } else {
        bail!("runtime {name} contains a missing value for a valid water-table cell")
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
