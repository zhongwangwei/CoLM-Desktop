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
    // Windows 上**打开的 NetCDF 文件不能删**（`Os code 32`：被另一个进程占用）；
    // unix 允许删掉打开着的文件，所以本机看不出来。先放掉句柄再删。
    drop(produced);
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

/// 网格写出（`flux_map_and_write_2d`）：计入但缺测的 patch 只进分母；未计入的两边都不进；
/// 分母不超过 `1e-5` 的格子写 `spval`；静态场原样写出。
#[test]
fn gridded_history_aggregates_by_area_and_filter() {
    let dims = HistoryDimensions {
        patch: 3,
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
    // 两格：格子 0 由 patch 0、1 共享，格子 1 只有 patch 2。
    let grid = std::sync::Arc::new(HistoryGrid {
        lat: vec![0.25],
        lon: vec![0.25, 0.75],
        lat_s: vec![0.0],
        lat_n: vec![0.5],
        lon_w: vec![0.0, 0.5],
        lon_e: vec![0.5, 1.0],
        parts: vec![vec![(0, 3.0)], vec![(0, 1.0)], vec![(1, 2.0)]],
        patch_area: vec![3.0, 1.0, 2.0],
        statics: vec![(
            "landarea".to_owned(),
            "land area".to_owned(),
            "km2".to_owned(),
            vec![4.0, 2.0],
        )],
        first_record_statics: vec![(
            "croparea".to_owned(),
            "crop area".to_owned(),
            "km2".to_owned(),
            vec![3.0, 0.0],
        )],
    });
    let mut buffers = HistoryBuffers::new(dims, site, 1).with_grid(grid).unwrap();
    buffers.declare(&["t_grnd", "fsena"]).unwrap();
    buffers.set_time(0, 60).unwrap();
    // t_grnd：patch 0 = 10、patch 1 计入但缺测、patch 2 未计入。
    buffers.select_patch(Some(0)).unwrap();
    buffers.set_patch_scalar("t_grnd", 0, 10.0).unwrap();
    buffers.include("t_grnd", 0).unwrap();
    buffers.include("fsena", 0).unwrap();
    buffers.set_patch_scalar("fsena", 0, 1.0).unwrap();
    buffers.select_patch(Some(1)).unwrap();
    buffers.include("t_grnd", 0).unwrap();
    buffers.include("fsena", 0).unwrap();
    buffers.set_patch_scalar("fsena", 0, 5.0).unwrap();
    buffers.select_patch(Some(2)).unwrap();
    buffers.set_patch_scalar("t_grnd", 0, 99.0).unwrap();
    buffers.select_patch(None).unwrap();
    let path = scratch("gridded");
    buffers.write(&path).unwrap();
    let file = netcdf::open(&path).unwrap();
    let values = |name: &str| {
        file.variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    };
    // 格子 0：(10*3)/(3+1)；格子 1 没有计入的 patch → spval。
    assert_eq!(values("f_t_grnd"), vec![30.0 / 4.0, MISSING_VALUE]);
    assert_eq!(
        values("f_fsena"),
        vec![(1.0 * 3.0 + 5.0 * 1.0) / 4.0, MISSING_VALUE]
    );
    assert_eq!(values("landarea"), vec![4.0, 2.0]);
    // `croparea` 定义带 time 维（上游 itime = 1），只写第 1 个时间槽。
    assert_eq!(values("croparea"), vec![3.0, 0.0]);
    let croparea = file.variable("croparea").unwrap();
    let names = croparea
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["time", "lat", "lon"]);
    let t_grnd = file.variable("f_t_grnd").unwrap();
    let dimensions = t_grnd
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    assert_eq!(dimensions, vec!["time", "lat", "lon"]);
    drop(file);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn vector_aggregation_weights_by_subfrc_and_skips_missing() {
    let vector = HistoryVector {
        elmindex: vec![10, 20],
        elements: vec![0..2, 2..3],
        subfrc: vec![0.25, 0.75, 1.0],
    };
    let values = [4.0, 8.0, MISSING_VALUE];
    let out = vector.aggregate(|p| values[p], |_| true, false);
    // `Σ FMA(frac, v, acc) / Σ frac`；全缺测的单元写 spval。
    assert_eq!(out[0], 0.75f64.mul_add(8.0, 0.25 * 4.0) / (0.25 + 0.75));
    assert_eq!(out[1], MISSING_VALUE);
    let total = vector.aggregate(|p| values[p], |p| p != 0, true);
    assert_eq!(total, vec![8.0, MISSING_VALUE]);
}
