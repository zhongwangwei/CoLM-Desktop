use super::*;

#[test]
fn forcing_summary_rejects_a_non_finite_time_axis() {
    let dir = std::env::temp_dir().join(format!(
        "colm-forcing-met-time-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("met.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("time", 2).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        time.put_values(&[0.0, f64::INFINITY], ..).unwrap();
    }
    let error = summarize(&path).unwrap_err().to_string();
    assert!(error.contains("non-finite"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}
