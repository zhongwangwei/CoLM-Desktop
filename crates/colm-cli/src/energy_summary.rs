//! Full-record surface-flux statistics; plot sampling never enters the reduction.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

const FLUXES: [&str; 4] = ["f_rnet", "f_fsena", "f_lfevpa", "f_fgrnd"];
const PLOT_POINTS: usize = 2400;
const MAX_RECORDS: usize = 1_000_000;

fn file_stats(files: &[PathBuf]) -> Result<BTreeMap<PathBuf, String>> {
    files
        .iter()
        .map(|path| {
            Ok((
                path.clone(),
                super::fingerprint::stable_file_identity(&std::fs::metadata(path)?),
            ))
        })
        .collect()
}

fn check_variable(file: &netcdf::File, name: &str, records: usize) -> Result<()> {
    let variable = file
        .variable(name)
        .with_context(|| format!("missing energy flux {name}"))?;
    ensure!(
        variable
            .dimensions()
            .first()
            .is_some_and(|dim| dim.name() == "time")
            && variable
                .dimensions()
                .iter()
                .skip(1)
                .all(|dim| dim.len() == 1)
            && variable.len() == records,
        "{name}: energy statistics require a single-point scalar time series"
    );
    for (attribute, identity) in [("scale_factor", 1.0), ("add_offset", 0.0)] {
        if let Some(value) = variable.attribute_value(attribute) {
            let values = super::numeric_attribute_values(value?);
            ensure!(
                values.len() == 1 && values[0] == identity,
                "{name}: packed values must be unpacked before energy statistics"
            );
        }
    }
    if name == "time" {
        let units = super::variable_units(file, name).context("history time units are missing")?;
        ensure!(
            matches!(
                units.trim(),
                "minutes since 1900-1-1 0:0:0" | "minutes since 1900-01-01 00:00:00"
            ),
            "unsupported history time units: {units}"
        );
        if let Some(calendar) = variable.attribute_value("calendar") {
            ensure!(
                matches!(calendar?, netcdf::AttributeValue::Str(ref text) if ["standard", "gregorian", "proleptic_gregorian"].contains(&text.as_str())),
                "unsupported history calendar"
            );
        }
    } else {
        let units = super::variable_units(file, name)
            .with_context(|| format!("{name}: energy flux units are missing"))?;
        ensure!(
            matches!(
                units.trim(),
                "W/m2" | "W/m²" | "W m-2" | "W m^-2" | "W m**-2"
            ),
            "{name}: energy statistics require W/m²; actual units are {units:?}"
        );
    }
    Ok(())
}

pub(super) fn run(case: &Path, from: Option<i64>, to: Option<i64>) -> Result<Value> {
    ensure!(
        from.zip(to).is_none_or(|(from, to)| from < to),
        "energy window start must precede its end"
    );
    let name = colm_case::case_name(&case.join("case.nml"))?;
    let history_root = case.join("out").join(name);
    let all = super::history_files(&history_root)?;
    ensure!(
        all.len() <= 1200,
        "energy statistics limited to 1200 history files; subset the inputs first"
    );
    let before = file_stats(&all)?;
    let mut wanted = vec!["time"];
    wanted.extend(FLUXES);
    let available = super::files_with_variables(&all, &wanted)?;
    let mut files = Vec::new();
    let mut total = 0_usize;
    for path in available {
        let file = netcdf::open(&path)?;
        let records = file
            .variable("time")
            .context("history time is missing")?
            .len();
        for variable in &wanted {
            check_variable(&file, variable, records)?;
        }
        if from.is_some() || to.is_some() {
            let time = super::read_file_1d(&file, &path, "time")?;
            let unix = colm_hist::time::unix_seconds(&time);
            if !unix.iter().any(|time| {
                from.is_none_or(|from| *time >= from) && to.is_none_or(|to| *time <= to)
            }) {
                continue;
            }
        }
        total = total
            .checked_add(records)
            .context("history record count overflow")?;
        ensure!(total <= MAX_RECORDS, "energy statistics limited to 1000000 history records; select a smaller window or subset the inputs");
        files.push(path);
    }
    ensure!(
        !files.is_empty(),
        "no history records in the selected energy window"
    );
    let mut raw = super::read_history_many(&files, &wanted)?;
    let time = raw.remove("time").context("history time is missing")?;
    let unix = colm_hist::time::unix_seconds(&time);
    ensure!(
        FLUXES.iter().all(|name| raw[*name].len() == time.len()),
        "energy fluxes do not share one scalar time axis"
    );
    let residual = (0..time.len())
        .map(|index| {
            let values = FLUXES.map(|name| raw[name][index]);
            if values.iter().all(|value| value.is_finite()) {
                let value = values[0] - values[1] - values[2] - values[3];
                ensure!(
                    value.is_finite(),
                    "energy flux subtraction overflowed; verify input values and units"
                );
                Ok(value)
            } else {
                Ok(f64::NAN)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let indexes = super::series_indices_multi(&unix, &[residual.as_slice()], from, to, None);
    let selected = indexes
        .iter()
        .map(|index| residual[*index])
        .collect::<Vec<_>>();
    let n = selected.iter().filter(|value| value.is_finite()).count();
    let mean = super::study::science::finite_mean(&selected);
    ensure!(
        n == 0 || mean.is_some(),
        "energy mean could not be represented; verify input values and units"
    );
    let plot =
        super::series_indices_multi(&unix, &[residual.as_slice()], from, to, Some(PLOT_POINTS));
    ensure!(
        before == file_stats(&all)? && all == super::history_files(&history_root)?,
        "history changed during energy statistics; refresh and try again"
    );
    Ok(json!({
        "schema":1,
        "expression":"f_rnet - f_fsena - f_lfevpa - f_fgrnd",
        "closure_assessed":false,
        "full_stats":{
            "basis":"all_records_in_selected_window",
            "mean_kind":"arithmetic_mean_of_common_finite_records",
            "status":if mean.is_some() {"computed"} else {"unavailable"},
            "source_n":indexes.len(), "n":n, "missing":indexes.len()-n,
            "mean":mean, "units":"W/m²",
        },
        "series":{
            "time":plot.iter().map(|index| unix[*index]).collect::<Vec<_>>(),
            "vars":{"residual":plot.iter().map(|index| residual[*index].is_finite().then_some(residual[*index])).collect::<Vec<_>>()},
            "source_n":indexes.len(),"n":plot.len(),"downsampled":plot.len()<indexes.len(),
        },
        "window":{"from":from,"to":to,"end_inclusive":true,"time_basis":"model_history_clock"},
        "limitation":"Surface flux residual excludes storage and other energy terms; it does not establish model budget closure.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(values: &[f64], unit: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "energy-summary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("out/Case/history")).unwrap();
        std::fs::write(root.join("case.nml"), "&nl_colm\nDEF_CASE_NAME='Case'\n/\n").unwrap();
        let mut file = netcdf::create(root.join("out/Case/history/Case_hist_2000-01.nc")).unwrap();
        file.add_dimension("time", values.len()).unwrap();
        file.add_dimension("patch", 1).unwrap();
        let time = (0..values.len())
            .map(|index| colm_hist::time::minutes_from_1900(2000) as f64 + index as f64 * 60.)
            .collect::<Vec<_>>();
        {
            let mut variable = file.add_variable::<f64>("time", &["time"]).unwrap();
            variable
                .put_attribute("units", "minutes since 1900-1-1 0:0:0")
                .unwrap();
            variable.put_values(&time, ..).unwrap();
        }
        let zeros = vec![0.; values.len()];
        for name in FLUXES {
            let mut variable = file.add_variable::<f64>(name, &["time", "patch"]).unwrap();
            variable.put_attribute("units", unit).unwrap();
            variable
                .put_values(if name == "f_rnet" { values } else { &zeros }, ..)
                .unwrap();
        }
        drop(file);
        root
    }
    #[test]
    fn full_mean_is_not_the_extrema_sample_mean() {
        let _guard = super::super::netcdf_test_guard();
        let mut values = vec![1.; 10001];
        values[5000] = 10000.;
        let root = fixture(&values, "W/m2");
        let result = run(&root, None, None).unwrap();
        let full = result["full_stats"]["mean"].as_f64().unwrap();
        let sampled = result["series"]["vars"]["residual"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect::<Vec<_>>();
        let sample_mean = super::super::study::science::finite_mean(&sampled).unwrap();
        assert!((full - super::super::study::science::finite_mean(&values).unwrap()).abs() < 1e-12);
        assert!((sample_mean - full).abs() > 1.);
        assert_eq!(result["full_stats"]["source_n"], 10001);
        assert_eq!(result["series"]["downsampled"], true);
        assert_eq!(result["closure_assessed"], false);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn complete_flux_tuples_and_window_define_the_full_statistic() {
        let _guard = super::super::netcdf_test_guard();
        let root = fixture(&[1., 5., f64::NAN, 9.], "W m-2");
        let from = colm_hist::time::unix_seconds(&[colm_hist::time::minutes_from_1900(2000)
            as f64
            + 60.])[0];
        let result = run(&root, Some(from), None).unwrap();
        assert_eq!(result["full_stats"]["source_n"], 3);
        assert_eq!(result["full_stats"]["n"], 2);
        assert_eq!(result["full_stats"]["missing"], 1);
        assert_eq!(result["full_stats"]["mean"], 7.);
        assert!(result["series"]["vars"]["residual"][1].is_null());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_flux_units_do_not_become_watts_per_square_metre() {
        let _guard = super::super::netcdf_test_guard();
        let root = fixture(&[1., 5.], "J/m2");
        assert!(run(&root, None, None)
            .unwrap_err()
            .to_string()
            .contains("actual units"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
