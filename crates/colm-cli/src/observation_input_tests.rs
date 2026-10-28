//! 观测解码层的回归测试。
//!
//! 这里测的是**语义**，不是格式：同一个物理量从 packed NetCDF、kW/m² CSV、
//! 无单位 CSV 或带订正候选的文件进来，解码后的数必须表达同一个事实；
//! 不能安全解读时必须是 Err 而不是「看起来正常」的数。

use std::path::{Path, PathBuf};

use super::*;
use colm_hist::obs::EVALUATION_VARIABLES;

fn eval(name: &str) -> &'static EvaluationVariable {
    EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == name)
        .expect("known evaluation variable")
}

fn request(corrected: bool) -> ObsDecodeRequest {
    ObsDecodeRequest {
        corrected,
        label_position: ObsLabelPosition::Start,
    }
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "colm-obs-input-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn create(dir: &Path, length: usize) -> (PathBuf, netcdf::FileMut) {
    let path = dir.join("obs.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("time", length).unwrap();
    (path, file)
}

fn write_f64(file: &mut netcdf::FileMut, name: &str, units: Option<&str>, values: &[f64]) {
    let mut variable = file.add_variable::<f64>(name, &["time"]).unwrap();
    if let Some(units) = units {
        variable.put_attribute("units", units).unwrap();
    }
    variable.put_values(values, ..).unwrap();
}

fn decode_path(path: &Path, variable: &str, corrected: bool) -> Result<Option<DecodedObs>> {
    let file = netcdf::open(path).unwrap();
    decode(&file, path, eval(variable), &request(corrected))
}

#[test]
fn packed_values_are_decoded_and_fill_is_masked_before_scaling() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("packed");
    let (path, mut file) = create(&dir, 3);
    {
        let mut variable = file.add_variable::<i16>("Qle", &["time"]).unwrap();
        variable.put_attribute("units", "W/m2").unwrap();
        variable.put_attribute("_FillValue", -32767_i16).unwrap();
        variable.put_attribute("scale_factor", 0.1_f64).unwrap();
        variable.put_attribute("add_offset", 1.0_f64).unwrap();
        variable.put_values(&[1000_i16, -32767, 500], ..).unwrap();
    }
    write_f64(&mut file, "Qle_qc", None, &[0.0, 0.0, 1.0]);
    drop(file);

    let decoded = decode_path(&path, "Qle", false).unwrap().unwrap();
    // 1000*0.1+1 = 101；500*0.1+1 = 51。填充值必须先于 packing 识别，
    // 否则 -32767 会变成 -3275.7 这样的「观测值」。
    assert!(
        (decoded.values[0] - 101.0).abs() < 1e-9,
        "{:?}",
        decoded.values
    );
    assert!(decoded.values[1].is_nan(), "fill 必须先于 scale 识别");
    assert!(
        (decoded.values[2] - 51.0).abs() < 1e-9,
        "{:?}",
        decoded.values
    );
    assert_eq!(decoded.qc, vec![0.0, 0.0, 1.0]);
    assert_eq!(decoded.units, "W/m2");
    assert_eq!(decoded.source_units.as_deref(), Some("W/m2"));
    assert_eq!(
        decoded.qc_provenance,
        QcProvenance::Source {
            variable: "Qle_qc".into()
        }
    );
    assert_eq!(decoded.correction, CorrectionStatus::NotRequested);
    assert_eq!(
        qc_label(&decoded.qc_provenance),
        "measured_only",
        "源文件 QC 仍是 measured_only"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn packed_qc_is_decoded_with_the_same_rules() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("packed-qc");
    let (path, mut file) = create(&dir, 2);
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0, 20.0]);
    {
        let mut variable = file.add_variable::<i16>("Qle_qc", &["time"]).unwrap();
        variable.put_attribute("scale_factor", 0.1_f64).unwrap();
        variable.put_values(&[0_i16, 10], ..).unwrap();
    }
    drop(file);

    let decoded = decode_path(&path, "Qle", false).unwrap().unwrap();
    assert_eq!(decoded.qc, vec![0.0, 1.0]);
    assert_eq!(decoded.values, vec![10.0, 20.0]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kilowatt_flux_units_are_converted_to_watts() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("kilowatt");
    let (path, mut file) = create(&dir, 2);
    write_f64(&mut file, "Qle", Some("kW/m2"), &[0.1, -0.0]);
    write_f64(&mut file, "Qle_qc", None, &[0.0, 0.0]);
    drop(file);

    let decoded = decode_path(&path, "Qle", false).unwrap().unwrap();
    assert!(
        (decoded.values[0] - 100.0).abs() < 1e-9,
        "{:?}",
        decoded.values
    );
    assert_eq!(decoded.values[1], 0.0);
    assert_eq!(decoded.source_units.as_deref(), Some("kW/m2"));
    assert_eq!(decoded.units, "W/m2");
    assert!(!decoded.units_assumed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_source_units_are_refused_instead_of_relabelled() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("bad-units");
    let (path, mut file) = create(&dir, 1);
    write_f64(&mut file, "Qle", Some("furlongs"), &[1.0]);
    write_f64(&mut file, "Qle_qc", None, &[0.0]);
    drop(file);

    let error = decode_path(&path, "Qle", false).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("furlongs"), "{message}");
    assert!(message.contains("Qle"), "报错要点名变量：{message}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_units_attribute_warns_and_assumes_canonical_units() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("no-units");
    let (path, mut file) = create(&dir, 2);
    write_f64(&mut file, "Qle", None, &[1.5, 2.5]);
    write_f64(&mut file, "Qle_qc", None, &[0.0, 0.0]);
    drop(file);

    let decoded = decode_path(&path, "Qle", false).unwrap().unwrap();
    assert!(decoded.units_assumed);
    assert_eq!(decoded.source_units, None);
    assert_eq!(decoded.units, "W/m2");
    assert_eq!(decoded.values, vec![1.5, 2.5]);
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.contains("no units attribute")),
        "{:?}",
        decoded.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_corrected_variable_is_reported_not_silently_substituted() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("no-corrected");
    let (path, mut file) = create(&dir, 2);
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0, 20.0]);
    write_f64(&mut file, "Qle_qc", None, &[0.0, 0.0]);
    drop(file);

    let decoded = decode_path(&path, "Qle", true).unwrap().unwrap();
    assert_eq!(decoded.label, "Qle", "缺订正变量时仍读未订正版本");
    match &decoded.correction {
        CorrectionStatus::NotApplied { requested, reason } => {
            assert_eq!(requested, "Qle_cor");
            assert!(reason.contains("Qle_cor"), "{reason}");
        }
        other => panic!("缺订正变量时应是 NotApplied，实际 {other:?}"),
    }
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.contains("Qle_cor")),
        "{:?}",
        decoded.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrected_variable_is_used_and_its_actual_qc_is_recorded() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("corrected");
    let (path, mut file) = create(&dir, 2);
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0, 20.0]);
    write_f64(&mut file, "Qle_cor", Some("W/m2"), &[30.0, 40.0]);
    write_f64(&mut file, "Qle_qc", None, &[0.0, 1.0]);
    drop(file);

    let decoded = decode_path(&path, "Qle", true).unwrap().unwrap();
    assert_eq!(decoded.label, "Qle_cor");
    assert_eq!(decoded.values, vec![30.0, 40.0]);
    assert_eq!(
        decoded.correction,
        CorrectionStatus::Applied {
            variable: "Qle_cor".into()
        }
    );
    // 文件没有 Qle_cor_qc，实际用的是 Qle_qc：必须记录，不能只留布尔。
    assert_eq!(decoded.qc_variable.as_deref(), Some("Qle_qc"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn generated_qc_is_not_reported_as_measured_quality() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("generated-qc");
    let (path, mut file) = create(&dir, 2);
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0, f64::NAN]);
    {
        let mut variable = file.add_variable::<f64>("Qle_qc", &["time"]).unwrap();
        variable
            .put_attribute("source", "generated: finite value=0, missing=1")
            .unwrap();
        variable
            .put_attribute("qc_kind", "generated_validity")
            .unwrap();
        variable.put_values(&[0.0, 1.0], ..).unwrap();
    }
    drop(file);

    let decoded = decode_path(&path, "Qle", false).unwrap().unwrap();
    match &decoded.qc_provenance {
        QcProvenance::ImportedGenerated { variable, rule } => {
            assert_eq!(variable, "Qle_qc");
            assert!(rule.contains("generated"), "{rule}");
        }
        other => panic!("生成掩码被标成了 {other:?}"),
    }
    assert_eq!(qc_label(&decoded.qc_provenance), "generated_validity");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_qc_contract_means_finite_only() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("no-qc");
    let (path, mut file) = create(&dir, 2);
    // GPP 没有 QC 契约；单位用 µ 开头的 CF 写法，顺带验证别名换算。
    write_f64(&mut file, "GPP", Some("µmol/m²/s"), &[1.0, 2.0]);
    drop(file);

    let decoded = decode_path(&path, "GPP", false).unwrap().unwrap();
    assert_eq!(decoded.qc, vec![0.0, 0.0]);
    assert_eq!(decoded.qc_provenance, QcProvenance::Absent);
    assert_eq!(decoded.qc_variable, None);
    assert_eq!(decoded.units, "umol/m2/s");
    assert_eq!(decoded.values, vec![1.0, 2.0], "改名不动数");
    assert_eq!(qc_label(&decoded.qc_provenance), "finite_only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn declared_qc_without_a_qc_variable_is_refused() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("missing-qc");
    let (path, mut file) = create(&dir, 1);
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0]);
    drop(file);

    let error = decode_path(&path, "Qle", false).unwrap_err();
    assert!(format!("{error:#}").contains("QC"), "{error:#}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_packing_is_refused() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("bad-packing");
    let (path, mut file) = create(&dir, 1);
    {
        let mut variable = file.add_variable::<f64>("Qle", &["time"]).unwrap();
        variable.put_attribute("units", "W/m2").unwrap();
        variable.put_attribute("scale_factor", 0.0_f64).unwrap();
        variable.put_values(&[10.0], ..).unwrap();
    }
    write_f64(&mut file, "Qle_qc", None, &[0.0]);
    drop(file);

    let error = decode_path(&path, "Qle", false).unwrap_err();
    assert!(format!("{error:#}").contains("scale_factor"), "{error:#}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn label_position_mismatch_is_surfaced_as_a_warning() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("label-position");
    let (path, mut file) = create(&dir, 1);
    file.add_attribute("obs_time_label_position", "end")
        .unwrap();
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0]);
    write_f64(&mut file, "Qle_qc", None, &[0.0]);
    drop(file);

    // 请求按区间起点（默认），文件声明区间终点：要告警而不是当作没看见。
    let decoded = decode_path(&path, "Qle", false).unwrap().unwrap();
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.contains("obs_time_label_position") || warning.contains("label")),
        "{:?}",
        decoded.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn urban_rnet_is_derived_from_converted_components() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("derived-rnet");
    let (path, mut file) = create(&dir, 3);
    write_f64(&mut file, "SWdown", Some("W/m2"), &[500.0, 500.0, -9999.0]);
    write_f64(&mut file, "LWdown", Some("W/m2"), &[350.0, 350.0, 350.0]);
    write_f64(&mut file, "SWup", Some("W/m2"), &[100.0, 100.0, 100.0]);
    write_f64(&mut file, "LWup", Some("W/m2"), &[420.0, 420.0, 420.0]);
    write_f64(&mut file, "SWdown_qc", None, &[0.0, 0.0, 0.0]);
    write_f64(&mut file, "LWdown_qc", None, &[0.0, 0.0, 0.0]);
    write_f64(&mut file, "SWup_qc", None, &[0.0, 1.0, 0.0]);
    write_f64(&mut file, "LWup_qc", None, &[0.0, 0.0, 0.0]);
    drop(file);

    let decoded = decode_path(&path, "Rnet", false).unwrap().unwrap();
    assert_eq!(decoded.label, "SWdown+LWdown-SWup-LWup");
    assert!(
        (decoded.values[0] - 330.0).abs() < 1e-9,
        "{:?}",
        decoded.values
    );
    assert!(decoded.values[1].is_nan(), "分量 QC 非实测时整点不可用");
    assert!(decoded.values[2].is_nan(), "分量缺测时整点不可用");
    assert_eq!(decoded.qc, vec![0.0, 1.0, 1.0]);
    assert_eq!(decoded.units, "W/m2");
    assert_eq!(decoded.source_units.as_deref(), Some("W/m2"));
    assert_eq!(qc_label(&decoded.qc_provenance), "measured_only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn absent_variable_is_reported_as_none_not_as_an_error() {
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("absent");
    let (path, mut file) = create(&dir, 1);
    write_f64(&mut file, "Qle", Some("W/m2"), &[10.0]);
    write_f64(&mut file, "Qle_qc", None, &[0.0]);
    drop(file);

    assert!(decode_path(&path, "Qh", false).unwrap().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imported_csv_is_decoded_with_converted_units_and_generated_qc() {
    // 端到端复现审计里的缺陷链：CSV 列头写 `Qle [kW/m2]`、没有 QC 列。
    // 导入必须真的换算（0.1 → 100 W/m²），解码必须把生成掩码标成
    // generated_validity，而不是 measured_only。
    let _guard = crate::netcdf_test_guard();
    let dir = tmp_dir("csv-roundtrip");
    let src = dir.join("obs.csv");
    std::fs::write(
        &src,
        "time,site,Qle [kW/m2]\n\
         2020-01-01 00:00:00,A,0.1\n\
         2020-01-01 00:30:00,A,0.2\n",
    )
    .unwrap();
    let converted = crate::observation_table::convert(
        &src,
        &dir.join("Observation"),
        &crate::observation_table::ConvertOptions {
            time_column: "time".into(),
            site_column: Some("site".into()),
            site_name: None,
            variables: vec![],
        },
    )
    .unwrap();
    let decoded = decode_path(&converted[0].path, "Qle", false)
        .unwrap()
        .unwrap();
    assert!(
        (decoded.values[0] - 100.0).abs() < 1e-9,
        "{:?}",
        decoded.values
    );
    assert!(
        (decoded.values[1] - 200.0).abs() < 1e-9,
        "{:?}",
        decoded.values
    );
    assert_eq!(decoded.source_units.as_deref(), Some("kW/m2"));
    assert_eq!(decoded.units, "W/m2");
    assert_eq!(qc_label(&decoded.qc_provenance), "generated_validity");
    // 文件里记的标签口径与请求一致，不该产生误导性警告。
    assert!(
        !decoded
            .warnings
            .iter()
            .any(|warning| warning.contains("obs_time_label_position")),
        "{:?}",
        decoded.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}
