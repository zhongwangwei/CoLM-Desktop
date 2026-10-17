//! Optional, bounded CF-time coverage suggestions; layout discovery survives every failure here.
use anyhow::{ensure, Context, Result};
use colm_case::forcing_templates::{forcing_directory_periods, ForcingPeriod, FORCING_TEMPLATES};
use colm_forcing::{civil_from_days, days_from_civil};
use colm_namelist::{Document, Value};
use std::{collections::BTreeSet, path::Path};

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq)]
pub(super) struct Coverage {
    pub start: String,
    pub end: String,
    pub dataset: String,
}

fn string(doc: &Document, key: &str) -> Result<String> {
    match doc.get(key) {
        Some(Value::Str(v)) => Ok(v.clone()),
        _ => anyhow::bail!("missing template string {key}"),
    }
}
fn integers(doc: &Document, key: &str) -> Result<Vec<i64>> {
    match doc.get(key) {
        Some(Value::List(values)) => values
            .iter()
            .map(|v| match v {
                Value::Int(v) => Ok(*v),
                _ => anyhow::bail!("invalid template integer {key}"),
            })
            .collect(),
        _ => anyhow::bail!("missing template array {key}"),
    }
}
fn period_bounds(period: &ForcingPeriod, yearly: bool) -> (i64, i64) {
    let start = days_from_civil(period.year, period.month, 1) * 86400;
    let (year, month) = if yearly || period.month == 12 {
        (period.year + 1, 1)
    } else {
        (period.year, period.month + 1)
    };
    (start, days_from_civil(year, month, 1) * 86400)
}

fn longest_contiguous(periods: &[ForcingPeriod], yearly: bool) -> &[ForcingPeriod] {
    let mut best = (0, 0);
    let mut begin = 0;
    for end in 1..=periods.len() {
        if end == periods.len()
            || period_bounds(&periods[end - 1], yearly).1 != period_bounds(&periods[end], yearly).0
        {
            if end - begin > best.1 - best.0 {
                best = (begin, end);
            }
            begin = end;
        }
    }
    &periods[best.0..best.1]
}

pub(super) fn probe(path: &Path, dataset: &str, files: &BTreeSet<String>) -> Result<Coverage> {
    let (_, text) = FORCING_TEMPLATES
        .iter()
        .find(|(name, _)| *name == dataset)
        .context("unknown forcing template")?;
    let doc = colm_namelist::parse(text)?;
    ensure!(
        doc.get("DEF_forcing%leapyear") == Some(&Value::Bool(true)),
        "non-Gregorian template requires manual date selection"
    );
    let yearly = match string(&doc, "DEF_forcing%groupby")?.as_str() {
        "year" => true,
        "month" => false,
        _ => anyhow::bail!("unsupported template file grouping"),
    };
    let steps = integers(&doc, "DEF_forcing%dtime")?;
    let offsets = integers(&doc, "DEF_forcing%offset")?;
    let Some(Value::List(names)) = doc.get("DEF_forcing%vname") else {
        anyhow::bail!("missing template variable names")
    };
    let periods = forcing_directory_periods(dataset, files)?
        .into_iter()
        .filter(|p| !yearly || p.month == 1)
        .collect::<Vec<_>>();
    let periods = longest_contiguous(&periods, yearly);
    ensure!(!periods.is_empty(), "no complete common forcing period");
    ensure!(
        periods.iter().map(|p| p.files.len()).sum::<usize>() <= 2048,
        "file inspection limit reached; enter dates manually"
    );
    let mut remaining = 2_000_000;
    let mut first = i64::MIN;
    let mut last = i64::MAX;
    for (position, period) in periods.iter().enumerate() {
        ensure!(
            (1..=9999).contains(&period.year),
            "unsupported forcing year"
        );
        let (start, end) = period_bounds(period, yearly);
        for (index, filename) in &period.files {
            let Value::Str(variable) = names.get(*index).context("missing forcing variable")?
            else {
                anyhow::bail!("invalid forcing variable")
            };
            let file = netcdf::open(path.join(filename))
                .with_context(|| format!("cannot inspect {filename}"))?;
            let (lower, upper) = colm_forcing::time_coverage::aligned_time_bounds(
                &file,
                variable,
                start,
                end,
                *steps.get(*index).context("missing time step")?,
                *offsets.get(*index).context("missing time offset")?,
                &mut remaining,
            )
            .with_context(|| format!("{filename}: time coverage cannot be confirmed"))?;
            if position == 0 {
                first = first.max(lower);
            }
            if position + 1 == periods.len() {
                last = last.min(upper);
            }
        }
    }
    inclusive_days(first, last, dataset)
}

fn inclusive_days(first: i64, last: i64, dataset: &str) -> Result<Coverage> {
    let start = first.div_euclid(86400) + i64::from(first.rem_euclid(86400) != 0);
    // The final run step needs an upper record. Never extend coverage by the forcing step.
    let end = last.div_euclid(86400) - 1;
    ensure!(
        end >= start,
        "no complete common day with an upper forcing record"
    );
    let date = |day| {
        let (y, m, d) = civil_from_days(day);
        format!("{y:04}-{m:02}-{d:02}")
    };
    Ok(Coverage {
        start: date(start),
        end: date(end),
        dataset: dataset.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inclusive_dates_trim_partial_days_and_reserve_upper_record() {
        let origin = days_from_civil(2003, 1, 1) * 86400;
        assert_eq!(
            inclusive_days(origin + 1800, origin + 4 * 86400 - 3600, "ERA5").unwrap(),
            Coverage {
                start: "2003-01-02".into(),
                end: "2003-01-03".into(),
                dataset: "ERA5".into()
            }
        );
        assert_eq!(
            inclusive_days(origin, origin + 86400, "X").unwrap().end,
            "2003-01-01"
        );
        assert!(inclusive_days(origin + 3600, origin + 86400, "X").is_err());
    }
    #[test]
    fn missing_months_never_bridge_a_coverage_gap() {
        let periods = [1, 2, 4, 5, 6].map(|month| ForcingPeriod {
            year: 2003,
            month,
            files: vec![],
        });
        let selected = longest_contiguous(&periods, false);
        assert_eq!(
            selected.iter().map(|p| p.month).collect::<Vec<_>>(),
            [4, 5, 6]
        );
        assert!(longest_contiguous(&[], false).is_empty());
    }
    #[test]
    fn real_monthly_netcdf_times_produce_common_safe_days_and_reject_bad_axis() {
        let root = std::env::temp_dir().join(format!("colm-coverage-mswx-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut files = BTreeSet::new();
        for (prefix, variable, offset) in [
            ("Temp", "air_temperature", 1.5),
            ("SpecHum", "specific_humidity", 1.5),
            ("Pres", "surface_pressure", 1.5),
            ("P", "precipitation", 1.5),
            ("Wind", "wind_speed", 1.5),
            ("SWd", "downward_shortwave_radiation", 0.0),
            ("LWd", "downward_longwave_radiation", 1.5),
        ] {
            let name = format!("{prefix}_2003_01.nc");
            let mut file = netcdf::create(root.join(&name)).unwrap();
            file.add_dimension("time", 248).unwrap();
            file.add_dimension("lat", 1).unwrap();
            file.add_dimension("lon", 1).unwrap();
            let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
            time.put_attribute("units", "hours since 2003-01-01 00:00:00")
                .unwrap();
            let values = (0..248)
                .map(|index| f64::from(index) * 3. + offset)
                .collect::<Vec<_>>();
            time.put_values(&values, ..).unwrap();
            file.add_variable::<f32>(variable, &["time", "lat", "lon"])
                .unwrap();
            file.close().unwrap();
            files.insert(name);
        }
        assert_eq!(
            probe(&root, "MSWX", &files).unwrap(),
            Coverage {
                start: "2003-01-02".into(),
                end: "2003-01-30".into(),
                dataset: "MSWX".into()
            }
        );
        let mut file = netcdf::append(root.join("Temp_2003_01.nc")).unwrap();
        file.variable_mut("time")
            .unwrap()
            .put_value(0., 12)
            .unwrap();
        file.close().unwrap();
        assert!(probe(&root, "MSWX", &files).is_err());
        assert!(probe(&root, "CRUJRA", &files).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
