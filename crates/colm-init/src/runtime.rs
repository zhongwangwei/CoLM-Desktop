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

/// Runtime BGC state is core-model data; this module only reads it from NetCDF.
pub use colm_core::{
    BgcEquilibriumState as RuntimeCnState, BgcVegetationCarbon as RuntimeCnVegetationCarbon,
};

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

/// Read one spatial sample from `cnsteadystate.nc` for the BGC cold-start path.
///
/// CoLM initializes seven carbon and seven nitrogen decomposition pools from this
/// source, then derives total mineral nitrogen from the ammonium and nitrate
/// profiles.  Missing values are rejected rather than turned into a plausible but
/// scientifically false BGC restart.
pub fn read_single_point_cn_state(
    path: impl AsRef<Path>,
    latitude_degrees: f64,
    longitude_degrees: f64,
) -> Result<RuntimeCnState> {
    const CARBON: [&str; 7] = [
        "litr1c_vr",
        "litr2c_vr",
        "litr3c_vr",
        "cwdc_vr",
        "soil1c_vr",
        "soil2c_vr",
        "soil3c_vr",
    ];
    const NITROGEN: [&str; 7] = [
        "litr1n_vr",
        "litr2n_vr",
        "litr3n_vr",
        "cwdn_vr",
        "soil1n_vr",
        "soil2n_vr",
        "soil3n_vr",
    ];

    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open BGC initial state {}", path.display()))?;
    let (latitude, longitude) = cell_indices_f32(&file, latitude_degrees, longitude_degrees)?;
    let layers = file
        .dimension("soil")
        .context("BGC runtime file has no soil dimension")?
        .len();
    ensure!(
        layers == 10,
        "BGC runtime must contain CoLM's ten soil layers"
    );
    let profiles = |names: &[&str]| -> Result<Vec<f64>> {
        let mut values = Vec::with_capacity(names.len() * layers);
        for &name in names {
            let profile = profile_3d_f32(&file, name, latitude, longitude, layers)?;
            validate_variable_values(&file, name, &profile)?;
            values.extend(profile);
        }
        Ok(values)
    };
    let scalar = |name| {
        let value = scalar_2d_f32(&file, name, latitude, longitude)?;
        ensure!(
            valid_value(&file, name, value),
            "BGC runtime {name} contains a missing value"
        );
        Ok(value)
    };
    let ammonium_g_m3 = profile_3d_f32(&file, "smin_nh4_vr", latitude, longitude, layers)?;
    validate_variable_values(&file, "smin_nh4_vr", &ammonium_g_m3)?;
    let nitrate_g_m3 = profile_3d_f32(&file, "smin_no3_vr", latitude, longitude, layers)?;
    validate_variable_values(&file, "smin_no3_vr", &nitrate_g_m3)?;
    Ok(RuntimeCnState {
        decomposition_carbon_g_m3: profiles(&CARBON)?,
        decomposition_nitrogen_g_m3: profiles(&NITROGEN)?,
        ammonium_g_m3,
        nitrate_g_m3,
        vegetation_carbon: RuntimeCnVegetationCarbon {
            leaf_g_m2: scalar("leafc")?,
            leaf_storage_g_m2: scalar("leafc_storage")?,
            fine_root_g_m2: scalar("frootc")?,
            fine_root_storage_g_m2: scalar("frootc_storage")?,
            live_stem_g_m2: scalar("livestemc")?,
            dead_stem_g_m2: scalar("deadstemc")?,
            live_coarse_root_g_m2: scalar("livecrootc")?,
            dead_coarse_root_g_m2: scalar("deadcrootc")?,
        },
    })
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

fn cell_indices_f32(file: &netcdf::File, latitude: f64, longitude: f64) -> Result<(usize, usize)> {
    ensure!(
        latitude.is_finite() && longitude.is_finite(),
        "surface coordinate must be finite"
    );
    let latitudes = values_1d_f32(file, "lat")?;
    let longitudes = values_1d_f32(file, "lon")?;
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

fn values_1d_f32(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("runtime file has no {name}"))?;
    ensure!(
        variable.dimensions().len() == 1,
        "runtime {name} must be one-dimensional"
    );
    Ok(variable
        .get_values::<f32, _>(..)?
        .into_iter()
        .map(f64::from)
        .collect())
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

fn profile_3d_f32(
    file: &netcdf::File,
    name: &str,
    latitude: usize,
    longitude: usize,
    layers: usize,
) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("runtime file has no {name}"))?;
    require_dimensions(&variable, name, &["lat", "lon", "soil"])?;
    ensure!(
        variable.dimensions()[2].len() == layers,
        "runtime {name} layer count differs from soil"
    );
    Ok(variable
        .get_values::<f32, _>((latitude..latitude + 1, longitude..longitude + 1, 0..layers))?
        .into_iter()
        .map(f64::from)
        .collect())
}

fn scalar_2d_f32(
    file: &netcdf::File,
    name: &str,
    latitude: usize,
    longitude: usize,
) -> Result<f64> {
    let variable = file
        .variable(name)
        .with_context(|| format!("runtime file has no {name}"))?;
    require_dimensions(&variable, name, &["lat", "lon"])?;
    variable
        .get_values::<f32, _>((latitude..latitude + 1, longitude..longitude + 1))?
        .into_iter()
        .next()
        .map(f64::from)
        .context("BGC runtime scalar selection returned no value")
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
