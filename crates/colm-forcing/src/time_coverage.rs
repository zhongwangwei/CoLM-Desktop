//! Bounded coordinate-only checks for suggesting gridded forcing dates.
use anyhow::{ensure, Context, Result};

/// Check the entire small time coordinate, never the gridded meteorological arrays.
/// Missing/interior/duplicate records must not be inferred away from matching endpoints.
pub fn aligned_time_bounds(
    file: &netcdf::File,
    variable_name: &str,
    period_start: i64,
    period_end: i64,
    step: i64,
    offset: i64,
    remaining: &mut usize,
) -> Result<(i64, i64)> {
    ensure!(
        step > 0 && (0..step).contains(&offset),
        "unsupported forcing step/offset"
    );
    let data = file
        .variable(variable_name)
        .with_context(|| format!("missing forcing variable {variable_name}"))?;
    // Runtime uses the final Fortran dimension as time (first in netCDF's C ordering).
    let dimension = data
        .dimensions()
        .first()
        .context("forcing variable has no dimensions")?;
    let time = file
        .variable(&dimension.name())
        .context("forcing variable has no CF time coordinate")?;
    ensure!(
        time.dimensions().len() == 1
            && time.dimensions()[0].name() == dimension.name()
            && time.len() == dimension.len(),
        "CF time coordinate does not index the forcing time dimension"
    );
    crate::gapfill::validate_cf_calendar(&time)?;
    let units = match time
        .attribute_value("units")
        .context("CF time units are missing")??
    {
        netcdf::AttributeValue::Str(value) => value,
        netcdf::AttributeValue::Strs(values) if values.len() == 1 => values[0].clone(),
        _ => anyhow::bail!("CF time units are not a string"),
    };
    let (factor, origin) = crate::gapfill::parse_cf_time_units(&units)?;
    let len = time.len();
    ensure!(
        (2..=100_000).contains(&len) && len <= *remaining,
        "time-coordinate inspection limit reached; enter dates manually"
    );
    ensure!(
        (period_end - period_start) % step == 0
            && i64::try_from(len)? == (period_end - period_start) / step,
        "time record count does not match the template period"
    );
    *remaining -= len;
    let values = time.get_values::<f64, _>(..)?;
    for (index, value) in values.iter().enumerate() {
        let actual = origin as f64 + value * factor;
        let expected = (period_start + offset + index as i64 * step) as f64;
        ensure!(
            actual.is_finite() && (actual - expected).abs() <= 0.01,
            "CF time coordinate does not match template period/step/offset at record {}",
            index + 1
        );
    }
    Ok((
        period_start + offset,
        period_start + offset + (len as i64 - 1) * step,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(tag: &str, units: &str, calendar: &str, values: &[f64]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("colm-cf-time-{tag}-{}.nc", std::process::id()));
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", values.len()).unwrap();
        file.add_dimension("lat", 1).unwrap();
        file.add_dimension("lon", 1).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", units).unwrap();
        time.put_attribute("calendar", calendar).unwrap();
        time.put_values(values, ..).unwrap();
        file.add_variable::<f32>("temperature", &["time", "lat", "lon"])
            .unwrap();
        file.close().unwrap();
        path
    }
    #[test]
    fn cf_hour_axis_checks_template_alignment_and_budget() {
        let values = (0..24)
            .map(|hour| f64::from(hour) + 0.5)
            .collect::<Vec<_>>();
        let path = fixture(
            "hours",
            "hours since 2003-01-01 00:00:00",
            "standard",
            &values,
        );
        let file = netcdf::open(&path).unwrap();
        let origin = crate::days_from_civil(2003, 1, 1) * 86400;
        let mut remaining = 24;
        assert_eq!(
            aligned_time_bounds(
                &file,
                "temperature",
                origin,
                origin + 86400,
                3600,
                1800,
                &mut remaining
            )
            .unwrap(),
            (origin + 1800, origin + 84600)
        );
        assert_eq!(remaining, 0);
        assert!(aligned_time_bounds(
            &file,
            "temperature",
            origin,
            origin + 86400,
            3600,
            1800,
            &mut 23
        )
        .is_err());
        assert!(aligned_time_bounds(
            &file,
            "temperature",
            origin,
            origin + 86400,
            3600,
            0,
            &mut 24
        )
        .is_err());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn unrelated_coordinate_dimension_cannot_overstate_meteorology_coverage() {
        let path = std::env::temp_dir().join(format!(
            "colm-cf-time-wrong-dimension-{}.nc",
            std::process::id()
        ));
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 2).unwrap();
        file.add_dimension("other", 24).unwrap();
        let mut time = file.add_variable::<f64>("time", &["other"]).unwrap();
        time.put_attribute("units", "hours since 2003-01-01")
            .unwrap();
        time.put_values(&(0..24).map(f64::from).collect::<Vec<_>>(), ..)
            .unwrap();
        file.add_variable::<f32>("temperature", &["time"]).unwrap();
        file.close().unwrap();
        let file = netcdf::open(&path).unwrap();
        let origin = crate::days_from_civil(2003, 1, 1) * 86400;
        assert!(aligned_time_bounds(
            &file,
            "temperature",
            origin,
            origin + 86400,
            3600,
            0,
            &mut 24
        )
        .unwrap_err()
        .to_string()
        .contains("dimension"));
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn duplicate_missing_nonfinite_or_non_gregorian_time_never_produces_dates() {
        let origin = crate::days_from_civil(2003, 1, 1) * 86400;
        for kind in ["duplicate", "missing", "nan", "calendar", "units"] {
            let mut values = (0..24).map(f64::from).collect::<Vec<_>>();
            match kind {
                "duplicate" => values[12] = 11.,
                "missing" => {
                    values.remove(12);
                }
                "nan" => values[12] = f64::NAN,
                _ => (),
            }
            let path = fixture(
                kind,
                if kind == "units" {
                    "fortnights since 2003-01-01"
                } else {
                    "hours since 2003-01-01"
                },
                if kind == "calendar" {
                    "360_day"
                } else {
                    "gregorian"
                },
                &values,
            );
            let file = netcdf::open(&path).unwrap();
            assert!(
                aligned_time_bounds(
                    &file,
                    "temperature",
                    origin,
                    origin + 86400,
                    3600,
                    0,
                    &mut 100
                )
                .is_err(),
                "{kind}"
            );
            drop(file);
            std::fs::remove_file(path).unwrap();
        }
    }
}
