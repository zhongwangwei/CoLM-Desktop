use super::*;

#[test]
fn gap_runs_are_classified_without_touching_observations() {
    let values = [
        1.0,
        f64::NAN,
        f64::NAN,
        4.0,
        f64::NAN,
        f64::NAN,
        f64::NAN,
        f64::NAN,
        9.0,
    ];
    let report = analyze_gaps(&values, 2);
    assert_eq!(report.missing, 6);
    assert_eq!(report.short_missing, 2);
    assert_eq!(report.long_missing, 4);
    assert_eq!(report.longest_gap, 4);
    assert_eq!(report.runs.len(), 2);
}

#[test]
fn repaired_long_gaps_do_not_request_era5_again() {
    let report = RepairSummary {
        timezone: TimezoneDecision {
            offset_hours: 0.0,
            source: TimezoneSource::FileMetadata,
            confidence: TimezoneConfidence::High,
            conflict: false,
            solar_noon_hour: None,
            solar_noon_std_hours: None,
        },
        latitude: 0.0,
        longitude: 0.0,
        start_utc: 0,
        end_utc: 0,
        variables: vec![VariableRepairSummary {
            slot: 1,
            variable: "Tair".into(),
            missing: 2,
            quality_rejected: 0,
            short_missing: 0,
            long_missing: 2,
            longest_gap: 2,
            interpolated: 0,
            era5_corrected: 2,
            unresolved: 0,
        }],
    };
    assert!(!report.needs_era5());
}

#[test]
fn continuous_short_gap_is_linear_and_long_gap_stays_missing() {
    let mut values = vec![
        0.0,
        f64::NAN,
        f64::NAN,
        3.0,
        f64::NAN,
        f64::NAN,
        f64::NAN,
        7.0,
    ];
    let qc = fill_short_gaps(&mut values, 2, VariableKind::Continuous);
    assert_eq!(&values[..4], &[0.0, 1.0, 2.0, 3.0]);
    assert!(values[4..7].iter().all(|v| v.is_nan()));
    assert_eq!(
        qc,
        vec![
            QC_OBSERVED,
            QC_INTERPOLATED,
            QC_INTERPOLATED,
            QC_OBSERVED,
            QC_UNRESOLVED,
            QC_UNRESOLVED,
            QC_UNRESOLVED,
            QC_OBSERVED
        ]
    );
}

#[test]
fn precipitation_is_not_invented_between_wet_bounds() {
    let mut dry = vec![0.0, f64::NAN, 0.0];
    let dry_qc = fill_short_gaps(&mut dry, 2, VariableKind::Precipitation);
    assert_eq!(dry, vec![0.0, 0.0, 0.0]);
    assert_eq!(dry_qc[1], QC_INTERPOLATED);

    let mut wet = vec![0.0, f64::NAN, 1.0];
    let wet_qc = fill_short_gaps(&mut wet, 2, VariableKind::Precipitation);
    assert!(wet[1].is_nan());
    assert_eq!(wet_qc[1], QC_UNRESOLVED);
}

#[test]
fn basic_qc_turns_only_physically_implausible_observations_into_gaps() {
    let mut temperature = [280.0, 999.0, 180.0, 350.0];
    assert_eq!(apply_basic_qc(&mut temperature, 1, false).unwrap(), 1);
    assert!(temperature[1].is_nan());
    assert_eq!(temperature[0], 280.0);

    let mut scalar_wind = [2.0, -1.0, 101.0];
    assert_eq!(apply_basic_qc(&mut scalar_wind, 6, true).unwrap(), 2);
    assert!(scalar_wind[1..].iter().all(|value| value.is_nan()));

    let mut wind_component = [-50.0, 50.0];
    assert_eq!(apply_basic_qc(&mut wind_component, 6, false).unwrap(), 0);
}

#[test]
fn timezone_prefers_manual_then_metadata_then_longitude() {
    let manual = decide_timezone(Some(9.5), Some("UTC"), Some(120.0)).unwrap();
    assert_eq!(manual.offset_hours, 9.5);
    assert_eq!(manual.source, TimezoneSource::ManualOverride);

    let explicit = decide_timezone(None, Some("UTC+08:00"), Some(0.0)).unwrap();
    assert_eq!(explicit.offset_hours, 8.0);
    assert_eq!(explicit.source, TimezoneSource::FileMetadata);

    let inferred = decide_timezone(None, None, Some(145.0)).unwrap();
    assert_eq!(inferred.offset_hours, 10.0);
    assert_eq!(inferred.source, TimezoneSource::LongitudeInferred);
    assert_eq!(inferred.confidence, TimezoneConfidence::Low);
}

fn synthetic_shortwave(days: usize, peak_hour: f64) -> (Vec<i64>, Vec<f64>) {
    let times = (0..days * 24)
        .map(|index| index as i64 * 3600)
        .collect::<Vec<_>>();
    let shortwave = times
        .iter()
        .map(|time| {
            let hour = time.rem_euclid(86400) as f64 / 3600.0;
            let cosine = ((hour - peak_hour) / 12.0 * std::f64::consts::PI).cos();
            if cosine > 0.0 {
                900.0 * cosine.powi(2)
            } else {
                0.0
            }
        })
        .collect();
    (times, shortwave)
}

#[test]
fn solar_noon_distinguishes_local_clock_from_utc() {
    let (times, local_shortwave) = synthetic_shortwave(12, 12.0);
    let local =
        decide_timezone_with_solar(None, None, Some(116.4), &times, &local_shortwave).unwrap();
    assert_eq!(local.source, TimezoneSource::SolarNoonInferred);
    assert_eq!(local.offset_hours, 8.0);
    assert_eq!(local.confidence, TimezoneConfidence::Medium);
    assert!(!local.conflict);

    let (_, utc_shortwave) = synthetic_shortwave(12, 4.25);
    let utc = decide_timezone_with_solar(None, None, Some(116.4), &times, &utc_shortwave).unwrap();
    assert_eq!(utc.source, TimezoneSource::SolarNoonConfirmedUtc);
    assert_eq!(utc.offset_hours, 0.0);
}

#[test]
fn solar_noon_confirms_at_neu_utc_instead_of_longitude_fallback() {
    let (times, shortwave) = synthetic_shortwave(12, 11.70);
    let decision =
        decide_timezone_with_solar(None, None, Some(11.3175), &times, &shortwave).unwrap();

    assert_eq!(decision.source, TimezoneSource::SolarNoonConfirmedUtc);
    assert_eq!(decision.offset_hours, 0.0);
    assert_eq!(decision.confidence, TimezoneConfidence::Medium);
    assert!((decision.solar_noon_hour.unwrap() - 11.70).abs() < 0.1);
}

#[test]
fn declared_timezone_wins_but_disagreement_is_reported() {
    let (times, local_shortwave) = synthetic_shortwave(12, 12.0);
    let decision =
        decide_timezone_with_solar(None, Some("UTC"), Some(116.4), &times, &local_shortwave)
            .unwrap();
    assert_eq!(decision.source, TimezoneSource::FileMetadata);
    assert_eq!(decision.offset_hours, 0.0);
    assert!(decision.conflict);
    assert_eq!(decision.confidence, TimezoneConfidence::Low);
}

#[test]
fn netcdf_diagnosis_uses_the_selected_shortwave_slot_as_timezone_evidence() {
    let dir = std::env::temp_dir().join("colm-gapfill-solar-file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    let (times, shortwave) = synthetic_shortwave(12, 12.0);
    let mut file = netcdf::create(&src).unwrap();
    file.add_dimension("time", times.len()).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "seconds since 2020-01-01 00:00:00")
        .unwrap();
    time.put_values(
        &times.iter().map(|value| *value as f64).collect::<Vec<_>>(),
        ..,
    )
    .unwrap();
    let mut lat = file.add_variable::<f64>("latitude", &[]).unwrap();
    lat.put_value(39.9, ()).unwrap();
    let mut lon = file.add_variable::<f64>("longitude", &[]).unwrap();
    lon.put_value(116.4, ()).unwrap();
    let mut tair = file.add_variable::<f64>("Tair", &["time"]).unwrap();
    tair.put_attribute("units", "K").unwrap();
    tair.put_values(&vec![280.0; times.len()], ..).unwrap();
    let mut sw = file.add_variable::<f64>("SWdown", &["time"]).unwrap();
    sw.put_attribute("units", "W/m2").unwrap();
    sw.put_values(&shortwave, ..).unwrap();
    drop(file);

    let mut plan = repair_plan(None, 1);
    plan.slots.push(RepairSlot {
        index: 7,
        source_name: "SWdown".into(),
        source_units: "W/m2".into(),
        also_add: Vec::new(),
    });
    let diagnosis = diagnose_file(&src, &plan).unwrap();
    assert_eq!(diagnosis.timezone.source, TimezoneSource::SolarNoonInferred);
    assert_eq!(diagnosis.timezone.offset_hours, 8.0);
}

#[test]
fn nearest_grid_handles_longitude_wrap() {
    let lats = [-37.8, -37.7];
    let lons = [359.8, 0.0, 0.2];
    assert_eq!(
        nearest_grid_point(&lats, &lons, -37.73, -0.1).unwrap(),
        (1, 0)
    );
}

#[test]
fn stale_manual_coordinates_cannot_override_the_forcing_file() {
    let root = std::env::temp_dir().join(format!(
        "colm-gapfill-coordinate-contract-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("point.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        let mut latitude = file.add_variable::<f64>("latitude", &[]).unwrap();
        latitude.put_value(47.1, ()).unwrap();
        let mut longitude = file.add_variable::<f64>("longitude", &[]).unwrap();
        longitude.put_value(11.3, ()).unwrap();
    }
    let file = netcdf::open(path).unwrap();
    let error = super::site_coordinates(&file, Some(-37.7), Some(145.0)).unwrap_err();
    assert!(format!("{error:#}").contains("conflicts"));
    assert_eq!(
        super::site_coordinates(&file, Some(47.1), Some(11.3)).unwrap(),
        (47.1, 11.3)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn additive_and_multiplicative_bias_use_only_overlap() {
    let obs = [Some(12.0), Some(14.0), None, Some(18.0)];
    let donor = [10.0, 12.0, 5000.0, 16.0];
    let add = correction(&obs, &donor, CorrectionKind::Additive, 3).unwrap();
    assert!((add.apply(20.0) - 22.0).abs() < 1e-12);

    let obs = [Some(2.0), Some(4.0), None, Some(6.0)];
    let donor = [1.0, 2.0, 9000.0, 3.0];
    let mul = correction(&obs, &donor, CorrectionKind::Multiplicative, 3).unwrap();
    assert!((mul.apply(5.0) - 10.0).abs() < 1e-12);
}

#[test]
fn correction_rejects_too_little_overlap() {
    let obs = [Some(2.0), None];
    let donor = [1.0, 2.0];
    assert!(correction(&obs, &donor, CorrectionKind::Additive, 2).is_err());
}

fn repair_plan(era5: Option<std::path::PathBuf>, short_gap_max: usize) -> RepairPlan {
    RepairPlan {
        slots: vec![RepairSlot {
            index: 1,
            source_name: "Tair".into(),
            source_units: "K".into(),
            also_add: Vec::new(),
        }],
        short_gap_max,
        manual_utc_offset: None,
        latitude: None,
        longitude: None,
        era5,
        min_overlap: 2,
    }
}

fn write_source(path: &std::path::Path, values: &[f64]) {
    let mut file = netcdf::create(path).unwrap();
    file.add_attribute("time_shown_in", "UTC").unwrap();
    file.add_dimension("time", values.len()).unwrap();
    let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
    time.put_attribute("units", "seconds since 2008-01-01 00:00:00")
        .unwrap();
    time.put_values(
        &(0..values.len())
            .map(|index| index as f64 * 3600.0)
            .collect::<Vec<_>>(),
        netcdf::Extents::All,
    )
    .unwrap();
    let mut lat = file.add_variable::<f64>("latitude", &[]).unwrap();
    lat.put_values(&[-37.73], netcdf::Extents::All).unwrap();
    let mut lon = file.add_variable::<f64>("longitude", &[]).unwrap();
    lon.put_values(&[145.01], netcdf::Extents::All).unwrap();
    let mut tair = file.add_variable::<f64>("Tair", &["time"]).unwrap();
    tair.set_fill_value(-9999.0).unwrap();
    tair.put_attribute("units", "K").unwrap();
    tair.put_values(values, netcdf::Extents::All).unwrap();
    let mut station_code = file.add_variable::<i32>("station_code", &[]).unwrap();
    station_code
        .put_values(&[42], netcdf::Extents::All)
        .unwrap();
}

#[test]
fn short_gap_repair_writes_a_new_file_with_qc_and_keeps_source_unchanged() {
    let dir = std::env::temp_dir().join("colm-gapfill-short-file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    let dst = dir.join("repaired.nc");
    write_source(&src, &[280.0, -9999.0, 282.0]);

    let summary = repair_file(&src, &dst, &repair_plan(None, 1)).unwrap();
    assert_eq!(summary.variables[0].interpolated, 1);
    assert_eq!(summary.unresolved(), 0);

    let source: Vec<f64> = netcdf::open(&src)
        .unwrap()
        .variable("Tair")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(source, vec![280.0, -9999.0, 282.0]);

    let output = netcdf::open(&dst).unwrap();
    let repaired: Vec<f64> = output
        .variable("Tair")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(repaired, vec![280.0, 281.0, 282.0]);
    let qc: Vec<u8> = output
        .variable("Tair_gapfill_qc")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(qc, vec![QC_OBSERVED, QC_INTERPOLATED, QC_OBSERVED]);
    assert!(output.attribute("colm_gapfill_timezone_source").is_some());
    assert_eq!(
        format!("{:?}", output.variable("station_code").unwrap().vartype()),
        "Int(I32)",
        "untouched ancillary variables must retain their netCDF dtype"
    );
}

#[test]
fn failed_observation_qc_is_reported_and_repaired_like_a_gap() {
    let dir = std::env::temp_dir().join("colm-gapfill-qc-file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    let dst = dir.join("repaired.nc");
    write_source(&src, &[280.0, 999.0, 282.0]);

    let diagnosis = diagnose_file(&src, &repair_plan(None, 1)).unwrap();
    assert_eq!(diagnosis.variables[0].quality_rejected, 1);
    assert_eq!(diagnosis.missing(), 1);

    let summary = repair_file(&src, &dst, &repair_plan(None, 1)).unwrap();
    assert_eq!(summary.variables[0].quality_rejected, 1);
    assert_eq!(summary.variables[0].interpolated, 1);
    let repaired: Vec<f64> = netcdf::open(&dst)
        .unwrap()
        .variable("Tair")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(repaired, vec![280.0, 281.0, 282.0]);
}

#[test]
fn declared_missing_value_is_not_reported_as_a_qc_rejection() {
    let dir = std::env::temp_dir().join("colm-gapfill-missing-value");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    write_source(&src, &[280.0, -8888.0, 282.0]);
    netcdf::append(&src)
        .unwrap()
        .variable_mut("Tair")
        .unwrap()
        .put_attribute("missing_value", -8888.0_f64)
        .unwrap();

    let diagnosis = diagnose_file(&src, &repair_plan(None, 1)).unwrap();
    assert_eq!(diagnosis.variables[0].missing, 1);
    assert_eq!(diagnosis.variables[0].quality_rejected, 0);
}

#[test]
fn local_timezone_metadata_uses_a_numeric_offset_and_survives_rediagnosis() {
    let dir = std::env::temp_dir().join("colm-gapfill-local-timezone");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    write_source(&src, &[280.0, 281.0, 282.0]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.add_attribute("time_shown_in", "local standard time")
            .unwrap();
        file.add_attribute("time_zone", "Australia/Brisbane")
            .unwrap();
        file.add_attribute("local_utc_offset_hours", 10.0_f64)
            .unwrap();
    }
    let report = diagnose_file(&src, &repair_plan(None, 1)).unwrap();
    assert_eq!(report.timezone.offset_hours, 10.0);
    assert_eq!(report.timezone.source, TimezoneSource::FileMetadata);

    let dst = dir.join("repaired.nc");
    let mut plan = repair_plan(None, 1);
    plan.manual_utc_offset = Some(9.5);
    repair_file(&src, &dst, &plan).unwrap();
    let report = diagnose_file(&dst, &repair_plan(None, 1)).unwrap();
    assert_eq!(report.timezone.offset_hours, 9.5);
    assert_eq!(report.timezone.source, TimezoneSource::FileMetadata);
}

#[test]
fn accumulated_era5_water_and_energy_are_deaccumulated_across_midnight() {
    let times = [23 * 3600, 24 * 3600, 25 * 3600, 26 * 3600];
    let mut water = [0.023, 0.024, 0.001, 0.003];
    apply_donor_transform(
        &times,
        &mut water,
        "m",
        DonorTransform::AccumulatedWater,
        false,
    )
    .unwrap();
    assert!(water[0].is_nan());
    for (got, amount_m) in water[1..].iter().zip([0.001, 0.001, 0.002]) {
        let expected = amount_m * 1000.0 / 3600.0;
        assert!((got - expected).abs() < 1e-12, "{got} != {expected}");
    }

    let mut energy = [82_800.0, 86_400.0, 3_600.0, 10_800.0];
    apply_donor_transform(
        &times,
        &mut energy,
        "J m-2",
        DonorTransform::AccumulatedEnergy,
        false,
    )
    .unwrap();
    assert!(energy[0].is_nan());
    assert_eq!(&energy[1..], &[1.0, 1.0, 2.0]);
}

#[test]
fn timeseries_interval_amounts_are_converted_without_second_deaccumulation() {
    let times = [0, 3600, 7200];
    let mut water = [0.001, 0.002, 0.0];
    apply_donor_transform(
        &times,
        &mut water,
        "m",
        DonorTransform::AccumulatedWater,
        true,
    )
    .unwrap();
    assert_eq!(water, [1.0 / 3600.0, 2.0 / 3600.0, 0.0]);

    let interpolated = sample_donor(&[0, 3600], &[280.0, 282.0], &[1800], false).unwrap();
    assert_eq!(interpolated, [281.0]);

    let held = sample_donor(&[0, 3600], &[10.0, 20.0], &[0, 1800, 3600], true).unwrap();
    assert_eq!(held, [10.0, 20.0, 20.0]);
}

#[test]
fn unsupported_cf_calendar_is_rejected() {
    let dir = std::env::temp_dir().join("colm-gapfill-calendar");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    write_source(&src, &[280.0, 281.0, 282.0]);
    netcdf::append(&src)
        .unwrap()
        .variable_mut("time")
        .unwrap()
        .put_attribute("calendar", "noleap")
        .unwrap();

    let error = diagnose_file(&src, &repair_plan(None, 1))
        .unwrap_err()
        .to_string();
    assert!(error.contains("unsupported CF calendar"), "{error}");
}

#[test]
fn non_finite_time_coordinates_are_rejected() {
    let dir = std::env::temp_dir().join("colm-gapfill-invalid-time");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    write_source(&src, &[280.0, 281.0, 282.0]);
    netcdf::append(&src)
        .unwrap()
        .variable_mut("time")
        .unwrap()
        .put_value(f64::NAN, 1)
        .unwrap();
    let error = diagnose_file(&src, &repair_plan(None, 1))
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-finite"), "{error}");
}

#[test]
fn donor_fill_rejects_invalid_donors_before_and_after_correction() {
    let mut values = [280.0, f64::NAN];
    let original = [280.0, f64::NAN];
    let donor = [280.0, 9999.0];
    let months = [1, 1];
    let mut qc = [QC_OBSERVED, QC_UNRESOLVED];
    fill_from_donor(&mut values, &original, &donor, &months, &mut qc, 1, 1).unwrap();
    assert!(values[1].is_nan());
    assert_eq!(qc[1], QC_UNRESOLVED);

    let mut values = [280.0, f64::NAN];
    let original = [280.0, f64::NAN];
    let donor = [9999.0, 281.0];
    let mut qc = [QC_OBSERVED, QC_UNRESOLVED];
    let error = fill_from_donor(&mut values, &original, &donor, &months, &mut qc, 1, 1)
        .unwrap_err()
        .to_string();
    assert!(error.contains("overlapping samples"), "{error}");
}

fn write_era5_temperature(
    path: &std::path::Path,
    time_values: &[f64],
    latitude: f64,
    longitude: f64,
    values: &[f64],
) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("valid_time", time_values.len()).unwrap();
    file.add_dimension("latitude", 1).unwrap();
    file.add_dimension("longitude", 1).unwrap();
    let mut time = file
        .add_variable::<f64>("valid_time", &["valid_time"])
        .unwrap();
    time.put_attribute("units", "hours since 2008-01-01 00:00:00")
        .unwrap();
    time.put_values(time_values, netcdf::Extents::All).unwrap();
    let mut lat = file.add_variable::<f64>("latitude", &["latitude"]).unwrap();
    lat.put_values(&[latitude], netcdf::Extents::All).unwrap();
    let mut lon = file
        .add_variable::<f64>("longitude", &["longitude"])
        .unwrap();
    lon.put_values(&[longitude], netcdf::Extents::All).unwrap();
    let mut t2m = file
        .add_variable::<f64>("t2m", &["valid_time", "latitude", "longitude"])
        .unwrap();
    t2m.put_attribute("units", "K").unwrap();
    t2m.put_values(values, netcdf::Extents::All).unwrap();
}

fn write_era5_precip(path: &std::path::Path, file_name_is_timeseries: bool) {
    let mut file = netcdf::create(path).unwrap();
    if !file_name_is_timeseries {
        file.add_attribute("dataset", "reanalysis-era5-land-timeseries")
            .unwrap();
    }
    file.add_dimension("valid_time", 2).unwrap();
    file.add_dimension("latitude", 1).unwrap();
    file.add_dimension("longitude", 1).unwrap();
    let mut time = file
        .add_variable::<f64>("valid_time", &["valid_time"])
        .unwrap();
    time.put_attribute("units", "hours since 2008-01-01 00:00:00")
        .unwrap();
    time.put_values(&[0.0, 1.0], netcdf::Extents::All).unwrap();
    let mut lat = file.add_variable::<f64>("latitude", &["latitude"]).unwrap();
    lat.put_values(&[10.0], netcdf::Extents::All).unwrap();
    let mut lon = file
        .add_variable::<f64>("longitude", &["longitude"])
        .unwrap();
    lon.put_values(&[20.0], netcdf::Extents::All).unwrap();
    let mut tp = file
        .add_variable::<f64>("tp", &["valid_time", "latitude", "longitude"])
        .unwrap();
    tp.put_attribute("units", "m").unwrap();
    tp.put_values(&[0.001, 0.002], netcdf::Extents::All)
        .unwrap();
}

#[test]
fn era5_timeseries_interval_detection_uses_metadata_before_filename() {
    let dir = std::env::temp_dir().join("colm-gapfill-era5-metadata");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_era5_precip(&dir.join("renamed-cache.nc"), false);
    let origin = crate::civil::days_from_civil(2008, 1, 1) * 86400;
    let values = Era5Catalog::open(&dir, 10.0, 20.0)
        .unwrap()
        .series(4, false, &[origin, origin + 3600], 10.0, 20.0)
        .unwrap();
    assert_eq!(
        values,
        vec![0.001 * 1000.0 / 3600.0, 0.002 * 1000.0 / 3600.0]
    );
}

#[test]
fn era5_catalog_merges_multiple_month_files_in_time_order() {
    let dir = std::env::temp_dir().join("colm-gapfill-era5-months");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_era5_temperature(
        &dir.join("era5land_2008_01.nc"),
        &[0.0, 1.0],
        10.0,
        20.0,
        &[1.0, 2.0],
    );
    write_era5_temperature(
        &dir.join("era5land_2008_02.nc"),
        &[2.0, 3.0],
        10.0,
        20.0,
        &[3.0, 4.0],
    );
    let origin = crate::civil::days_from_civil(2008, 1, 1) * 86400;
    let values = Era5Catalog::open(&dir, 10.0, 20.0)
        .unwrap()
        .series(
            1,
            false,
            &[origin, origin + 3600, origin + 7200, origin + 10800],
            10.0,
            20.0,
        )
        .unwrap();
    assert_eq!(values, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn era5_cache_root_resolves_the_downloaded_point_directory() {
    let root = std::env::temp_dir().join("colm-gapfill-era5-point-root");
    let _ = std::fs::remove_dir_all(&root);
    let point = root.join(era5_point_cache_name(-37.73, 145.01));
    assert!(point.ends_with("era5land_lat_m37p7_lon_p145p0"));
    std::fs::create_dir_all(&point).unwrap();
    write_era5_temperature(
        &root.join("legacy_other_station.nc"),
        &[0.0, 1.0],
        0.0,
        0.0,
        &[999.0, 999.0],
    );
    write_era5_temperature(
        &point.join("era5land_timeseries_2008.nc"),
        &[0.0, 1.0],
        -37.7,
        145.0,
        &[280.0, 281.0],
    );
    let origin = crate::civil::days_from_civil(2008, 1, 1) * 86400;
    let values = Era5Catalog::open(&root, -37.73, 145.01)
        .unwrap()
        .series(1, false, &[origin, origin + 3600], -37.73, 145.01)
        .unwrap();
    assert_eq!(values, vec![280.0, 281.0]);
}

#[test]
fn era5_cache_for_a_different_grid_point_is_rejected() {
    let dir = std::env::temp_dir().join("colm-gapfill-era5-wrong-grid");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    write_source(&src, &[281.0, 282.0, -9999.0, -9999.0, 285.0, 286.0]);
    let donor = dir.join("era5.nc");
    write_era5_temperature(
        &donor,
        &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        0.0,
        0.0,
        &[279.0, 280.0, 281.0, 282.0, 283.0, 284.0],
    );
    let error = repair_file(&src, &dir.join("repaired.nc"), &repair_plan(Some(donor), 1))
        .unwrap_err()
        .to_string();
    assert!(error.contains("too far"), "{error}");
}

#[test]
fn a_long_gap_uses_the_nearest_era5_grid_and_overlap_bias() {
    let dir = std::env::temp_dir().join("colm-gapfill-era5-file");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    let dst = dir.join("repaired.nc");
    let donor = dir.join("era5.nc");
    write_source(&src, &[281.0, 282.0, -9999.0, -9999.0, 285.0, 286.0]);
    {
        let mut file = netcdf::create(&donor).unwrap();
        file.add_dimension("valid_time", 6).unwrap();
        file.add_dimension("latitude", 2).unwrap();
        file.add_dimension("longitude", 2).unwrap();
        let mut time = file
            .add_variable::<f64>("valid_time", &["valid_time"])
            .unwrap();
        time.put_attribute("units", "hours since 2008-01-01 00:00:00")
            .unwrap();
        time.put_values(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0], netcdf::Extents::All)
            .unwrap();
        let mut lat = file.add_variable::<f64>("latitude", &["latitude"]).unwrap();
        lat.put_values(&[-38.0, -37.7], netcdf::Extents::All)
            .unwrap();
        let mut lon = file
            .add_variable::<f64>("longitude", &["longitude"])
            .unwrap();
        lon.put_values(&[144.9, 145.0], netcdf::Extents::All)
            .unwrap();
        let mut t2m = file
            .add_variable::<f64>("t2m", &["valid_time", "latitude", "longitude"])
            .unwrap();
        t2m.put_attribute("units", "K").unwrap();
        let mut values = Vec::new();
        for time in 0..6 {
            for lat in 0..2 {
                for lon in 0..2 {
                    values.push(279.0 + time as f64 + lat as f64 * 20.0 + lon as f64 * 40.0);
                }
            }
        }
        t2m.put_values(&values, netcdf::Extents::All).unwrap();
    }

    // 最近格点 (lat=1, lon=1) 的 donor 是 339..344；观测是 donor -58。
    let summary = repair_file(&src, &dst, &repair_plan(Some(donor), 1)).unwrap();
    assert_eq!(summary.variables[0].era5_corrected, 2);
    let repaired: Vec<f64> = netcdf::open(&dst)
        .unwrap()
        .variable("Tair")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(repaired, vec![281.0, 282.0, 283.0, 284.0, 285.0, 286.0]);
}
