use std::path::{Path, PathBuf};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("colm-tabular-{name}-{}", std::process::id()))
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn plan() -> super::TabularPlan {
    super::TabularPlan {
        time_column: "time".into(),
        site_column: Some("site".into()),
        latitude_column: Some("lat".into()),
        longitude_column: Some("lon".into()),
        landtype_column: Some("landtype".into()),
        utc_offset_column: None,
        manual_utc_offset: None,
        latitude: None,
        longitude: None,
        step_seconds: Some(3600),
        land_cover_scheme: Some(super::LandCoverScheme::Igbp),
        heights: Some(crate::convert::Heights {
            v: 10.0,
            t: 2.0,
            q: 2.0,
        }),
        slots: vec![
            super::TabularSlot::new(1, "Tair", "K"),
            super::TabularSlot::new(2, "Qair", "kg/kg"),
            super::TabularSlot::new(3, "Psurf", "Pa"),
            super::TabularSlot::new(4, "Precip", "mm/hr"),
            super::TabularSlot::new(6, "Wind", "m/s"),
            super::TabularSlot::new(7, "SWdown", "W/m2"),
            super::TabularSlot::new(8, "LWdown", "W/m2"),
        ],
    }
}

#[test]
fn one_second_timestamp_jitter_is_rejected_instead_of_inferred_as_one_second_data() {
    let root = temp("jitter");
    let src = root.join("jitter.csv");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         A,2020-01-01 00:00:00,50,10,10,280,.005,100000,0,2,0,300\n\
         A,2020-01-01 00:30:00,50,10,10,281,.006,100010,0,2,20,301\n\
         A,2020-01-01 01:00:01,50,10,10,282,.007,100020,0,2,30,302\n",
    );
    let probe = super::probe_table(&src).unwrap();
    assert_eq!(probe.sites[0].step_seconds, Some(1800));
    assert_eq!(probe.sites[0].inserted_steps, 0);

    let mut p = plan();
    p.step_seconds = None;
    let error = super::import_table(&src, &root.join("Forcing"), &p).unwrap_err();
    assert!(format!("{error:#}").contains("off the inferred 1800s cadence"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn land_cover_class_is_validated_for_the_selected_scheme() {
    let root = temp("land-cover-scheme");
    let src = root.join("site.csv");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         A,2020-01-01 00:00,50,10,20,280,.005,100000,0,2,0,300\n\
         A,2020-01-01 01:00,50,10,20,281,.006,100010,0,2,20,301\n",
    );
    let error = super::import_table(&src, &root.join("Forcing"), &plan()).unwrap_err();
    assert!(format!("{error:#}").contains("IGBP"));

    let mut usgs = plan();
    usgs.land_cover_scheme = Some(super::LandCoverScheme::Usgs);
    assert!(super::import_table(&src, &root.join("Forcing-usgs"), &usgs).is_ok());

    let mut urban = plan();
    urban.land_cover_scheme = Some(super::LandCoverScheme::Urban);
    assert!(super::import_table(&src, &root.join("Forcing-lcz"), &urban).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn split_precipitation_columns_are_converted_before_summing() {
    let root = temp("mixed-extra-units");
    let src = root.join("site.csv");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair,Qair,Psurf,Rainf[kg/m2/s],Snowf[mm/hr],Wind,SWdown,LWdown\n\
         A,2020-01-01 00:00Z,50,10,10,280,.005,100000,1,3600,2,0,300\n\
         A,2020-01-01 01:00Z,50,10,10,281,.006,100010,1,1800,2,20,301\n",
    );
    let mut import = plan();
    let precipitation = import
        .slots
        .iter_mut()
        .find(|slot| slot.index == 4)
        .unwrap();
    precipitation.column = "Rainf".into();
    precipitation.source_units = "kg/m2/s".into();
    precipitation.also_add = vec!["Snowf".into()];
    let sites = super::import_table(&src, &root.join("Forcing"), &import).unwrap();
    let file = netcdf::open(&sites[0].staged_path).unwrap();
    let values: Vec<f64> = file.variable("Precip").unwrap().get_values(..).unwrap();
    assert_eq!(values, vec![2.0, 1.5]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn relative_humidity_column_can_supply_specific_humidity() {
    let root = temp("relative-humidity");
    let src = root.join("site.csv");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair[K],RH[%],Psurf[Pa],Precip[mm/hr],Wind[m/s],SWdown[W/m2],LWdown[W/m2]\n\
         A,2020-01-01 00:00Z,50,10,10,293.15,50,100000,0,2,0,300\n\
         A,2020-01-01 01:00Z,50,10,10,293.15,60,100000,0,2,20,301\n",
    );
    let mut import = plan();
    let humidity = import
        .slots
        .iter_mut()
        .find(|slot| slot.index == 2)
        .unwrap();
    humidity.column = "RH".into();
    humidity.source_units = "%".into();
    let sites = super::import_table(&src, &root.join("Forcing"), &import).unwrap();
    let file = netcdf::open(&sites[0].staged_path).unwrap();
    let values: Vec<f64> = file.variable("Qair").unwrap().get_values(..).unwrap();
    assert!(values[0] > 0.0 && values[0] < values[1]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_per_row_offset_column_can_represent_daylight_saving_changes() {
    let root = temp("varying-offsets");
    let src = root.join("site.csv");
    write(
        &src,
        "site,time,utc_offset,lat,lon,landtype,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         A,2020-03-29 01:00,1,50,10,10,280,.005,100000,0,2,0,300\n\
         A,2020-03-29 03:00,2,50,10,10,281,.006,100010,0,2,20,301\n",
    );
    let mut p = plan();
    p.utc_offset_column = Some("utc_offset".into());
    let imported = super::import_table(&src, &root.join("Forcing"), &p).unwrap();
    assert_eq!(imported[0].timezone_offset_hours, None);
    assert_eq!(imported[0].timezone_source, "table_offset_column");
    assert_eq!(imported[0].end_utc - imported[0].start_utc, 3600);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn table_expansion_has_a_hard_safety_limit() {
    let error = super::checked_step_count(0, 20_000_000, 1, 2).unwrap_err();
    assert!(format!("{error:#}").contains("would create"));
}

#[test]
fn csv_probe_finds_two_sites_columns_units_and_slot_candidates() {
    let root = temp("probe");
    let src = root.join("many.csv");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair[K],Qair[kg/kg],Psurf[Pa],Precip[mm/hr],Wind[m/s],SWdown[W/m2],LWdown[W/m2]\n\
         A,2020-01-01 00:00,50,10,10,280,.005,100000,0,2,0,300\n\
         B,2020-01-01 00:00,40,120,12,290,.010,99000,1,3,10,310\n\
         A,2020-01-01 01:00,50,10,10,281,.006,100010,0,2,20,301\n\
         B,2020-01-01 01:00,40,120,12,291,.011,99010,0,3,30,311\n",
    );

    let probe = super::probe_table(&src).unwrap();
    assert_eq!(probe.delimiter, "comma");
    assert_eq!(probe.rows, 4);
    assert_eq!(probe.site_column.as_deref(), Some("site"));
    assert_eq!(probe.time_column.as_deref(), Some("time"));
    assert_eq!(probe.sites.len(), 2);
    assert_eq!(
        probe
            .columns
            .iter()
            .find(|c| c.name == "Tair")
            .unwrap()
            .units
            .as_deref(),
        Some("K")
    );
    assert_eq!(
        probe
            .slots
            .iter()
            .find(|s| s.index == 1)
            .unwrap()
            .column
            .as_deref(),
        Some("Tair")
    );
    assert_eq!(
        probe.slots.iter().find(|s| s.index == 5).unwrap().column,
        None
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quoted_csv_site_names_keep_commas_and_escaped_quotes() {
    let root = temp("quoted-csv");
    let src = root.join("quoted.csv");
    write(
        &src,
        "site,time,lat,lon,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         \"Alpha, \"\"Forest\"\"\",202001010000,50,10,280,.005,100000,0,2,0,300\n\
         \"Alpha, \"\"Forest\"\"\",202001010100,50,10,281,.006,100010,0,2,20,301\n",
    );
    let probe = super::probe_table(&src).unwrap();
    assert_eq!(probe.sites[0].id, "Alpha, \"Forest\"");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tab_and_semicolon_delimiters_are_detected() {
    let root = temp("delimiters");
    for (name, separator, expected) in [("sites.tsv", '\t', "tab"), ("sites.txt", ';', "semicolon")]
    {
        let src = root.join(name);
        let header = [
            "time", "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
        ]
        .join(&separator.to_string());
        let first = [
            "202001010000",
            "280",
            ".005",
            "100000",
            "0",
            "2",
            "0",
            "300",
        ]
        .join(&separator.to_string());
        let second = [
            "202001010100",
            "281",
            ".006",
            "100010",
            "0",
            "2",
            "20",
            "301",
        ]
        .join(&separator.to_string());
        write(&src, &format!("{header}\n{first}\n{second}\n"));
        assert_eq!(super::probe_table(&src).unwrap().delimiter, expected);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn multisite_import_normalizes_utc_inserts_missing_rows_and_keeps_sites_separate() {
    let root = temp("import");
    let src = root.join("many.csv");
    let out = root.join("Forcing");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         A,2020-01-01 00:00,50,10,10,280,.005,100000,0,2,0,300\n\
         B,2020-01-01T00:00:00Z,40,120,12,290,.010,99000,1,3,10,310\n\
         A,2020-01-01 01:00,50,10,10,281,.006,100010,0,2,20,301\n\
         B,2020-01-01T01:00:00Z,40,120,12,291,.011,99010,0,3,30,311\n\
         A,2020-01-01 03:00,50,10,10,283,.008,100030,3.6,2,40,303\n\
         B,2020-01-01T02:00:00Z,40,120,12,292,.012,99020,0,3,50,312\n",
    );

    let imported = super::import_table(&src, &out, &plan()).unwrap();
    assert_eq!(imported.len(), 2);
    let a = imported.iter().find(|site| site.site == "A").unwrap();
    let b = imported.iter().find(|site| site.site == "B").unwrap();
    assert_eq!(a.rows, 3);
    assert_eq!(a.inserted_steps, 1);
    assert_eq!(a.timezone_offset_hours, Some(1.0));
    assert_eq!(a.timezone_source, "longitude_inferred_offset");
    assert_eq!(b.timezone_offset_hours, Some(0.0));
    assert_eq!(b.timezone_source, "timestamp_offset");
    assert!(a.staged_path.ends_with(".colm-tabular/A_Met.nc"));
    assert!(a.final_path.ends_with("Forcing/A_Met.nc"));

    let f = netcdf::open(&a.staged_path).unwrap();
    let time: Vec<f64> = f.variable("time").unwrap().get_values(..).unwrap();
    assert_eq!(time.len(), 4);
    assert_eq!(time[1] - time[0], 3600.0);
    let tair: Vec<f64> = f.variable("Tair").unwrap().get_values(..).unwrap();
    let fill = f
        .variable("Tair")
        .unwrap()
        .fill_value::<f64>()
        .unwrap()
        .unwrap();
    assert_eq!(tair[2], fill, "the absent 02:00 row must remain a real gap");
    let precip: Vec<f64> = f.variable("Precip").unwrap().get_values(..).unwrap();
    assert!(
        (precip[3] - 0.001).abs() < 1e-12,
        "mm/hr must become kg/m2/s"
    );
    assert_eq!(
        f.attribute("time_shown_in").unwrap().value().unwrap(),
        netcdf::AttributeValue::Str("UTC".into())
    );
    let lat: Vec<f64> = f.variable("latitude").unwrap().get_values(..).unwrap();
    assert_eq!(lat, vec![50.0]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn whitespace_txt_and_compact_timestamps_are_supported() {
    let root = temp("txt");
    let src = root.join("AT-Neu.txt");
    write(
        &src,
        "time Tair Qair Psurf Precip Wind SWdown LWdown\n\
         202001010000 280 .005 100000 0 2 0 300\n\
         202001010100 281 .006 100010 0 2 20 301\n",
    );
    let probe = super::probe_table(&src).unwrap();
    assert_eq!(probe.delimiter, "whitespace");
    assert_eq!(probe.sites[0].id, "AT-Neu");
    let mut p = plan();
    p.site_column = None;
    p.latitude_column = None;
    p.longitude_column = None;
    p.landtype_column = None;
    p.latitude = Some(47.1167);
    p.longitude = Some(11.3175);
    let imported = super::import_table(&src, &root.join("Forcing"), &p).unwrap();
    assert_eq!(imported[0].site, "AT-Neu");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tabular_local_time_uses_shortwave_solar_noon_before_longitude_fallback() {
    let root = temp("solar-timezone");
    let src = root.join("beijing.csv");
    let mut csv =
        String::from("site,time,lat,lon,landtype,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n");
    for index in 0..96 {
        let day = index / 24 + 1;
        let hour = index % 24;
        let cosine = ((hour as f64 - 12.0) / 12.0 * std::f64::consts::PI).cos();
        let swdown = if cosine > 0.0 {
            900.0 * cosine.powi(2)
        } else {
            0.0
        };
        csv.push_str(&format!(
            "CN-Bej,2020-01-{day:02} {hour:02}:00,39.9,116.4,10,280,.005,100000,0,2,{swdown},300\n"
        ));
    }
    write(&src, &csv);

    let imported = super::import_table(&src, &root.join("Forcing"), &plan()).unwrap();
    assert_eq!(imported[0].timezone_source, "solar_noon_inferred_offset");
    assert_eq!(imported[0].timezone_offset_hours, Some(8.0));
    let file = netcdf::open(&imported[0].staged_path).unwrap();
    assert_eq!(
        file.attribute("tabular_timezone_confidence")
            .unwrap()
            .value()
            .unwrap(),
        netcdf::AttributeValue::Str("medium".into())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn duplicate_station_timestamps_are_rejected() {
    let root = temp("duplicate");
    let src = root.join("bad.csv");
    write(
        &src,
        "site,time,lat,lon,landtype,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         A,2020-01-01T00:00Z,50,10,10,280,.005,100000,0,2,0,300\n\
         A,2020-01-01T00:00Z,50,10,10,281,.006,100010,0,2,20,301\n",
    );
    let err = super::import_table(&src, &root.join("Forcing"), &plan()).unwrap_err();
    assert!(format!("{err:#}").contains("duplicate"), "{err:#}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn multisite_table_requires_per_site_coordinates_instead_of_reusing_one_fallback() {
    let root = temp("multisite-coordinates");
    let src = root.join("bad.csv");
    write(
        &src,
        "site,time,Tair,Qair,Psurf,Precip,Wind,SWdown,LWdown\n\
         A,2020-01-01T00:00Z,280,.005,100000,0,2,0,300\n\
         A,2020-01-01T01:00Z,281,.006,100010,0,2,20,301\n\
         B,2020-01-01T00:00Z,290,.010,99000,1,3,10,310\n\
         B,2020-01-01T01:00Z,291,.011,99010,0,3,30,311\n",
    );
    let mut p = plan();
    p.latitude_column = None;
    p.longitude_column = None;
    p.landtype_column = None;
    p.latitude = Some(50.0);
    p.longitude = Some(10.0);
    let err = super::import_table(&src, &root.join("Forcing"), &p).unwrap_err();
    assert!(format!("{err:#}").contains("multiple sites"), "{err:#}");
    let _ = std::fs::remove_dir_all(root);
}
