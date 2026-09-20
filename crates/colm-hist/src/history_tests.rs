//! history 写出器的验收：与仓库里的黄金文件逐项比结构，再把每个变量的值
//! 原样灌进去、落盘后逐值比回来。
//!
//! 黄金文件（2.7 MB）入库，所以这条测试在任何平台上都能跑，不需要 PLUMBER2，
//! 也不需要 gfortran。结构比较覆盖：维度（含无限维落盘后的长度）、变量集合、
//! 逐变量维度顺序、数据类型、`long_name`/`units`/`missing_value`。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::schedule::{schedule, HistoryFrequency, HistoryGrouping, SimulationWindow};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/golden/CN-Cng_hist_2008-01.nc")
}

fn scratch(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("colm-hist-{label}-{}.nc", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn text_attribute(variable: &netcdf::Variable<'_>, name: &str) -> Option<String> {
    variable
        .attribute(name)
        .and_then(|attribute| attribute.value().ok())
        .and_then(|value| match value {
            netcdf::AttributeValue::Str(value) => Some(value),
            _ => None,
        })
}

#[test]
fn a_written_history_file_matches_the_golden_schema_and_values() {
    let golden_file = netcdf::open(golden()).expect("golden history opens");
    let golden_dims: BTreeMap<String, usize> = golden_file
        .dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect();
    let soil = golden_dims["soil"];
    let dims = HistoryDimensions {
        patch: golden_dims["patch"],
        soil,
        lake: golden_dims["lake"],
        snow_layers: golden_dims["soilsnow"] - soil,
        vegnodes: golden_dims["vegnodes"],
        band: golden_dims["band"],
        radiation_types: golden_dims["rtyp"],
        sensor: golden_dims["sensor"],
    };
    let scalar = |name: &str| -> f64 {
        golden_file
            .variable(name)
            .expect("golden scalar exists")
            .get_values::<f64, _>(..)
            .expect("readable")[0]
    };
    let site = HistorySite {
        latitude_degrees: scalar("lat"),
        longitude_degrees: scalar("lon"),
    };
    let time: Vec<i32> = golden_file
        .variable("time")
        .unwrap()
        .get_values(..)
        .unwrap();
    let names: Vec<String> = golden_file
        .variables()
        .filter_map(|variable| variable.name().strip_prefix("f_").map(str::to_string))
        .collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

    let mut buffers = HistoryBuffers::new(dims, site, time.len());
    buffers.declare(&name_refs).unwrap();
    for (record, minutes) in time.iter().enumerate() {
        buffers.set_time(record, *minutes).unwrap();
    }
    // 黄金文件里每个历史变量的值原样灌进去。
    let mut expected_values: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for variable in golden_file.variables() {
        // `name()` 返回 String，先绑定再切前缀，免得对临时值取借用。
        let raw = variable.name();
        let Some(name) = raw.strip_prefix("f_") else {
            continue;
        };
        let values: Vec<f64> = variable.get_values(..).unwrap();
        let per_record = values.len() / time.len();
        for record in 0..time.len() {
            let start = record * per_record;
            buffers
                .set_layered(name, record, &values[start..start + per_record])
                .unwrap();
        }
        expected_values.insert(name.to_string(), values);
    }
    let produced_path = scratch("golden-schema");
    buffers.write(&produced_path).unwrap();

    let produced = netcdf::open(&produced_path).expect("written history opens");
    let produced_dims: BTreeMap<String, usize> = produced
        .dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect();
    assert_eq!(produced_dims, golden_dims, "dimensions");
    let golden_names: BTreeSet<String> = golden_file.variables().map(|v| v.name()).collect();
    let produced_names: BTreeSet<String> = produced.variables().map(|v| v.name()).collect();
    assert_eq!(produced_names, golden_names, "variable set");

    for variable in golden_file.variables() {
        let name = variable.name();
        let ours = produced.variable(&name).expect("same variable set");
        let expected_dims: Vec<String> = variable
            .dimensions()
            .iter()
            .map(|dimension| dimension.name())
            .collect();
        let actual_dims: Vec<String> = ours
            .dimensions()
            .iter()
            .map(|dimension| dimension.name())
            .collect();
        assert_eq!(actual_dims, expected_dims, "{name}: dimension order");
        assert_eq!(ours.vartype(), variable.vartype(), "{name}: data type");
        assert_eq!(
            text_attribute(&ours, "long_name"),
            text_attribute(&variable, "long_name"),
            "{name}: long_name"
        );
        assert_eq!(
            text_attribute(&ours, "units"),
            text_attribute(&variable, "units"),
            "{name}: units"
        );
        assert_eq!(
            format!("{:?}", ours.attribute_value("missing_value")),
            format!("{:?}", variable.attribute_value("missing_value")),
            "{name}: missing_value"
        );
    }

    let written_time: Vec<i32> = produced.variable("time").unwrap().get_values(..).unwrap();
    assert_eq!(written_time, time, "time values");
    for (name, expected) in &expected_values {
        let actual: Vec<f64> = produced
            .variable(&file_variable_name(name))
            .unwrap_or_else(|| panic!("{name} missing from the written file"))
            .get_values(..)
            .unwrap();
        assert_eq!(actual.len(), expected.len(), "{name}: value count");
        for (index, (ours, theirs)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(ours, theirs, "{name}[{index}]");
        }
    }
    // 索引坐标与站点标量也要按值对上。
    for name in [
        "soil",
        "soilinterface",
        "soilsnow",
        "lake",
        "vegnodes",
        "band",
        "rtyp",
    ] {
        let ours: Vec<i32> = produced.variable(name).unwrap().get_values(..).unwrap();
        let theirs: Vec<i32> = golden_file.variable(name).unwrap().get_values(..).unwrap();
        assert_eq!(ours, theirs, "{name}: index values");
    }
    for name in ["lat", "lon"] {
        let ours: Vec<f64> = produced.variable(name).unwrap().get_values(..).unwrap();
        let theirs: Vec<f64> = golden_file.variable(name).unwrap().get_values(..).unwrap();
        assert_eq!(ours, theirs, "{name}: site coordinate");
    }
    std::fs::remove_file(&produced_path).unwrap();
}

/// 调度器必须逐值复现黄金文件的 `time` 轴，并按 `MONTH` 分组得到与文件名
/// 一致的后缀 —— 这是「哪些时刻落一条记录、标签是多少」唯一的经验证据。
#[test]
fn the_scheduled_labels_reproduce_the_golden_time_axis() {
    let golden_file = netcdf::open(golden()).expect("golden history opens");
    let expected: Vec<i32> = golden_file
        .variable("time")
        .unwrap()
        .get_values(..)
        .unwrap();
    // `oracle/cases/CN-Cng/case.nml`：2008-01-01 00:00 → 01-11 24:00，步长 1800 s，
    // DEF_HIST_FREQ='HOURLY'，DEF_HIST_groupby='MONTH'。
    let groups = schedule(
        SimulationWindow {
            start_year: 2008,
            start_julian_day: 1,
            start_seconds: 0,
            end_year: 2008,
            end_julian_day: 11,
            end_seconds: 86_400,
            timestep_seconds: 1_800,
        },
        HistoryFrequency::Hourly,
        HistoryGrouping::Month,
    )
    .unwrap();
    assert_eq!(groups.len(), 1, "the window lies inside one month");
    assert_eq!(groups[0].suffix, "2008-01");
    assert_eq!(groups[0].labels_minutes.len(), expected.len());
    for (index, (ours, theirs)) in groups[0].labels_minutes.iter().zip(&expected).enumerate() {
        assert_eq!(*ours, i64::from(*theirs), "record {index}");
    }
    let golden_name = golden().file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        golden_name.contains(&format!("hist_{}.nc", groups[0].suffix)),
        "{golden_name} should carry the scheduled suffix"
    );
}

#[test]
fn a_name_outside_the_gate_table_is_refused() {
    let dims = HistoryDimensions {
        patch: 1,
        soil: 10,
        lake: 10,
        snow_layers: 5,
        vegnodes: 4,
        band: 2,
        radiation_types: 2,
        sensor: 1,
    };
    let site = HistorySite {
        latitude_degrees: 0.0,
        longitude_degrees: 0.0,
    };
    let mut buffers = HistoryBuffers::new(dims, site, 1);
    let error = buffers
        .declare(&["definitely_not_a_history_variable"])
        .unwrap_err();
    assert!(error.to_string().contains("gate table"), "{error}");
}

/// DA 的 `ens` 维度没有尺寸来源，必须显式拒绝，不能静默按标量写出。
#[test]
fn a_dimension_without_a_declared_size_is_refused() {
    let Some(unsupported) = VARS.iter().find(|entry| entry.dims.contains(&"ens")) else {
        panic!("the gate table should still contain an ensemble-dimension variable");
    };
    let dims = HistoryDimensions {
        patch: 1,
        soil: 10,
        lake: 10,
        snow_layers: 5,
        vegnodes: 4,
        band: 2,
        radiation_types: 2,
        sensor: 1,
    };
    let site = HistorySite {
        latitude_degrees: 0.0,
        longitude_degrees: 0.0,
    };
    let mut buffers = HistoryBuffers::new(dims, site, 1);
    let error = buffers.declare(&[unsupported.name]).unwrap_err();
    assert!(error.to_string().contains("ens"), "{error}");
}
