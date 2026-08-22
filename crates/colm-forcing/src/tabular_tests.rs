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
