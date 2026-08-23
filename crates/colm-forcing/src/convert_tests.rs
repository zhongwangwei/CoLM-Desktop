//! 转换管道的测试。
//!
//! **这一组测试的地位与别处不同**：转换器改的是数值，而数值错了
//! 模型照样跑得完 —— 所以每一条都要断言具体的数，不能只断言「没报错」。

use std::path::PathBuf;

/// 造一个最小的强迫场文件：一个变量、三个时刻。
///
/// 不用真实的 PLUMBER2 文件（15 MB，且测试要能离线跑），但**维度与属性
/// 的形状照抄它** —— 转换器读的正是这些。
fn tiny_met(dir: &std::path::Path, var: &str, values: &[f64]) -> PathBuf {
    let p = dir.join("tiny_Met.nc");
    let mut f = netcdf::create(&p).expect("create");
    f.add_dimension("time", values.len()).expect("dim");
    let mut t = f.add_variable::<f64>("time", &["time"]).expect("time var");
    t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
        .expect("units");
    let secs: Vec<f64> = (0..values.len()).map(|i| (i as f64) * 1800.0).collect();
    t.put_values(&secs, netcdf::Extents::All).expect("put time");
    let mut v = f.add_variable::<f64>(var, &["time"]).expect("var");
    v.put_values(values, netcdf::Extents::All)
        .expect("put values");
    p
}

#[test]
fn an_identity_conversion_reproduces_every_value_bit_for_bit() {
    let dir = std::env::temp_dir().join("colm-convert-identity");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 刻意用一组**不能被二进制精确表示**的值 —— 若管道中途做了
    // 十进制往返（比如经过字符串），这里会差最后一位。
    let vals = [1.8337343205163141, 273.15, 0.1 + 0.2];
    let src = tiny_met(&dir, "Tair", &vals);
    let dst = dir.join("out_Met.nc");

    super::identity(&src, &dst).expect("identity conversion");

    let f = netcdf::open(&dst).unwrap();
    let got: Vec<f64> = f
        .variable("Tair")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(got, vals, "恒等转换必须逐位复现，差一个 ULP 都算失败");
}

#[test]
fn a_renamed_and_rescaled_variable_lands_in_the_slot_with_the_canonical_name() {
    let dir = std::env::temp_dir().join("colm-convert-rename");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 用户的文件：变量叫 TA_F，单位是摄氏度
    let p = dir.join("user_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("TA_F", &["time"]).unwrap();
        v.put_attribute("units", "degC").unwrap();
        v.put_values(&[0.0, 25.0], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "TA_F".into(),
            source_units: "degC".into(),
            also_add: Vec::new(),
        }],
        heights: None,
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    // 落地时用的是**规范名**（槽位的第一个候选名），不是用户的名字
    let got: Vec<f64> = f
        .variable("Tair")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(got, vec![273.15, 298.15]);

    // **换算过的要标出来** —— 否则读文件的人以为那就是源数据里的值
    let note = f
        .variable("Tair")
        .unwrap()
        .attribute("source")
        .and_then(|a| a.value().ok());
    let note = match note {
        Some(netcdf::AttributeValue::Str(s)) => s,
        other => panic!("Tair 应当带一条 source 属性，得到 {other:?}"),
    };
    assert!(note.contains("TA_F"), "要说出原变量名：{note}");
    assert!(note.contains("degC"), "要说出原单位：{note}");
}

#[test]
fn conversion_preserves_global_timezone_and_gapfill_provenance() {
    let dir = std::env::temp_dir().join("colm-convert-global-attrs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("source.nc");
    {
        let mut file = netcdf::create(&src).unwrap();
        file.add_attribute("time_shown_in", "UTC").unwrap();
        file.add_attribute("colm_gapfill_timezone_source", "manual_override")
            .unwrap();
        file.add_dimension("time", 2).unwrap();
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        time.put_values(&[0.0, 3600.0], netcdf::Extents::All)
            .unwrap();
        let mut tair = file.add_variable::<f64>("Tair", &["time"]).unwrap();
        tair.put_attribute("units", "K").unwrap();
        tair.put_values(&[280.0, 281.0], netcdf::Extents::All)
            .unwrap();
        let mut qc = file
            .add_variable::<u8>("Tair_gapfill_qc", &["time"])
            .unwrap();
        qc.put_attribute("flag_meanings", "observed interpolated")
            .unwrap();
        qc.put_values(&[0, 1], netcdf::Extents::All).unwrap();
    }
    let dst = dir.join("converted.nc");
    super::convert(
        &src,
        &dst,
        &super::Plan {
            slots: vec![super::SlotPlan {
                index: 1,
                source_name: "Tair".into(),
                source_units: "K".into(),
                also_add: Vec::new(),
            }],
            heights: None,
        },
    )
    .unwrap();
    let output = netcdf::open(dst).unwrap();
    for name in ["time_shown_in", "colm_gapfill_timezone_source"] {
        assert!(
            output.attribute(name).is_some(),
            "global attribute {name} was lost"
        );
    }
    assert_eq!(
        format!(
            "{:?}",
            output.variable("Tair_gapfill_qc").unwrap().vartype()
        ),
        "Int(U8)",
        "gap-fill provenance must remain an integer flag variable"
    );
}

#[test]
fn two_sources_sum_into_one_slot_and_both_survive_in_the_output() {
    let dir = std::env::temp_dir().join("colm-convert-sum");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("split_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 3).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0, 3600.0], netcdf::Extents::All)
            .unwrap();
        for (n, vals) in [("Rainf", [1.0, 0.0, 2.0]), ("Snowf", [0.0, 3.0, 0.5])] {
            let mut v = f.add_variable::<f64>(n, &["time"]).unwrap();
            v.put_attribute("units", "kg/m2/s").unwrap();
            v.put_values(&vals, netcdf::Extents::All).unwrap();
        }
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 4,
            source_name: "Rainf".into(),
            source_units: "kg/m2/s".into(),
            also_add: vec!["Snowf".into()],
        }],
        heights: None,
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();

    // 合成的总降水进第 4 槽的规范名。1+0、0+3、2+0.5 都是二进制精确的，
    // 所以这里比得起字面量。
    let precip: Vec<f64> = f
        .variable("Precip")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(precip, vec![1.0, 3.0, 2.5], "总降水应当是两者之和");

    // **两个源变量都要还在** —— 转换可以增加信息，不能减少信息
    let rain: Vec<f64> = f
        .variable("Rainf")
        .expect("Rainf 必须保留在产物里")
        .get_values(netcdf::Extents::All)
        .unwrap();
    let snow: Vec<f64> = f
        .variable("Snowf")
        .expect("Snowf 必须保留在产物里")
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(rain, vec![1.0, 0.0, 2.0]);
    assert_eq!(snow, vec![0.0, 3.0, 0.5]);

    // source 属性要说出它是合成的，以及 CoLM 会重新判相态
    let note = match f
        .variable("Precip")
        .unwrap()
        .attribute("source")
        .and_then(|a| a.value().ok())
    {
        Some(netcdf::AttributeValue::Str(s)) => s,
        other => panic!("Precip 应当带 source 属性，得到 {other:?}"),
    };
    assert!(note.contains("Rainf"), "要说出来源：{note}");
    assert!(note.contains("Snowf"), "要说出来源：{note}");
}

#[test]
fn a_kept_source_variable_does_not_lose_its_fill_value() {
    // **「转换可以增加信息，不能减少信息」也管属性。** `_FillValue` 是
    // `Float` 不是 `Str`，只搬字符串属性会把它丢在源文件里 —— 产物的
    // 读者就不知道哪个数代表缺测了。
    let dir = std::env::temp_dir().join("colm-convert-fill");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("fill_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        for (n, vals) in [("Rainf", [1.0, 2.0]), ("Snowf", [0.5, 0.25])] {
            let mut v = f.add_variable::<f64>(n, &["time"]).unwrap();
            v.put_attribute("units", "kg/m2/s").unwrap();
            v.put_attribute("_FillValue", -999.0_f64).unwrap();
            v.put_attribute("long_name", "rate").unwrap();
            v.put_values(&vals, netcdf::Extents::All).unwrap();
        }
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 4,
            source_name: "Rainf".into(),
            source_units: "kg/m2/s".into(),
            also_add: vec!["Snowf".into()],
        }],
        heights: None,
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    for n in ["Rainf", "Snowf"] {
        let v = f.variable(n).expect("源变量该保留");
        let fill = v
            .attribute_value("_FillValue")
            .and_then(|r| r.ok())
            .unwrap_or_else(|| panic!("{n} 丢了 _FillValue"));
        assert!(
            matches!(fill, netcdf::AttributeValue::Double(x) if x == -999.0),
            "{n} 的 _FillValue 应当原样保留，得到 {fill:?}"
        );
        // 字符串属性本来就搬得过去，一并守住别回退。
        assert!(v.attribute("long_name").is_some(), "{n} 丢了 long_name");
    }
}

#[test]
fn variables_the_slots_do_not_consume_are_carried_over() {
    // **CoLM 读的不止那八个槽位。** `reference_height_v/t/q` 是标量，
    // 不属于任何槽位，但 `met::summarize` 要读它们来填 forcing.nml 的
    // `DEF_forcing%HEIGHT_*`。转换时丢掉，它们就回落成 NaN 写进 namelist，
    // 而 CoLMDEBUG 内核的 RangeCheck 会直接 SIGILL —— 报出来的是
    // 「内核编进了 CoLMDEBUG」，看不出问题在强迫场少了三个标量。
    //
    // 规矩还是那条：**转换可以增加信息，不能减少信息。**
    let dir = std::env::temp_dir().join("colm-convert-carry");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("carry_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        f.add_dimension("x", 1).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        // 用户的名字，会被槽位消费掉
        let mut v = f.add_variable::<f64>("TA_F", &["time"]).unwrap();
        v.put_attribute("units", "K").unwrap();
        v.put_values(&[273.15, 274.15], netcdf::Extents::All)
            .unwrap();
        // 标量：不属于任何槽位，必须原样过去
        let mut h = f.add_variable::<f64>("reference_height_t", &[]).unwrap();
        h.put_attribute("units", "m").unwrap();
        h.put_values(&[6.0], netcdf::Extents::All).unwrap();
        // 一维辅助变量，同样不该丢
        let mut lat = f.add_variable::<f64>("latitude", &["x"]).unwrap();
        lat.put_values(&[44.5933], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "TA_F".into(),
            source_units: "K".into(),
            also_add: Vec::new(),
        }],
        heights: None,
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    let h: Vec<f64> = f
        .variable("reference_height_t")
        .expect("标量 reference_height_t 必须搬过去")
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(h, vec![6.0]);
    let lat: Vec<f64> = f
        .variable("latitude")
        .expect("latitude 必须搬过去")
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(lat, vec![44.5933]);

    // **被槽位消费掉的不重复搬。** `TA_F` 已经变成规范名 `Tair` 了，
    // 再留一份原名只会让人分不清哪个是准的。
    assert!(f.variable("Tair").is_some(), "槽位落地用规范名");
    assert!(
        f.variable("TA_F").is_none(),
        "TA_F 已经被槽位消费，不该再留一份"
    );
}

#[test]
fn conversion_refuses_a_stale_unit_from_the_plan() {
    let dir = std::env::temp_dir().join("colm-convert-unit-contract");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = tiny_met(&dir, "TA_F", &[0.0, 1.0]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.variable_mut("TA_F")
            .unwrap()
            .put_attribute("units", "degC")
            .unwrap();
    }
    let dst = dir.join("out.nc");
    let error = super::convert(
        &src,
        &dst,
        &super::Plan {
            slots: vec![super::SlotPlan {
                index: 1,
                source_name: "TA_F".into(),
                source_units: "K".into(),
                also_add: Vec::new(),
            }],
            heights: None,
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("re-probe"));
}

#[test]
fn split_precipitation_is_converted_per_source_before_summing() {
    let dir = std::env::temp_dir().join("colm-convert-mixed-units");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = tiny_met(&dir, "Rainf", &[1.0, 1.0]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.variable_mut("Rainf")
            .unwrap()
            .put_attribute("units", "kg/m2/s")
            .unwrap();
        let mut snow = file.add_variable::<f64>("Snowf", &["time"]).unwrap();
        snow.put_attribute("units", "mm/hr").unwrap();
        snow.put_values(&[3600.0, 1800.0], netcdf::Extents::All)
            .unwrap();
    }
    let dst = dir.join("out.nc");
    super::convert(
        &src,
        &dst,
        &super::Plan {
            slots: vec![super::SlotPlan {
                index: 4,
                source_name: "Rainf".into(),
                source_units: "kg/m2/s".into(),
                also_add: vec!["Snowf".into()],
            }],
            heights: None,
        },
    )
    .unwrap();
    let output = netcdf::open(dst).unwrap();
    let values: Vec<f64> = output
        .variable("Precip")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(values, vec![2.0, 1.5]);
}

#[test]
fn interval_precipitation_uses_the_file_cadence() {
    let dir = std::env::temp_dir().join("colm-convert-interval-units");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = tiny_met(&dir, "Rain", &[1.8, 3.6]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.variable_mut("Rain")
            .unwrap()
            .put_attribute("units", "mm")
            .unwrap();
    }
    let dst = dir.join("out.nc");
    super::convert(
        &src,
        &dst,
        &super::Plan {
            slots: vec![super::SlotPlan {
                index: 4,
                source_name: "Rain".into(),
                source_units: "mm".into(),
                also_add: Vec::new(),
            }],
            heights: None,
        },
    )
    .unwrap();
    let output = netcdf::open(dst).unwrap();
    let values: Vec<f64> = output
        .variable("Precip")
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert!((values[0] - 0.001).abs() < 1e-12);
    assert!((values[1] - 0.002).abs() < 1e-12);
}

#[test]
fn ancillary_integer_variables_keep_their_netcdf_type() {
    let dir = std::env::temp_dir().join("colm-convert-ancillary-type");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = tiny_met(&dir, "TA_F", &[280.0, 281.0]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.variable_mut("TA_F")
            .unwrap()
            .put_attribute("units", "K")
            .unwrap();
        let mut qc = file.add_variable::<i16>("station_qc", &["time"]).unwrap();
        qc.put_values(&[0, 2], netcdf::Extents::All).unwrap();
    }
    let dst = dir.join("out.nc");
    super::convert(
        &src,
        &dst,
        &super::Plan {
            slots: vec![super::SlotPlan {
                index: 1,
                source_name: "TA_F".into(),
                source_units: "K".into(),
                also_add: Vec::new(),
            }],
            heights: None,
        },
    )
    .unwrap();
    let output = netcdf::open(dst).unwrap();
    assert_eq!(
        output.variable("station_qc").unwrap().vartype(),
        netcdf::types::NcVariableType::Int(netcdf::types::IntType::I16)
    );
}

#[test]
fn measurement_heights_must_be_positive() {
    assert!(super::parse_heights("10,2,0").is_err());
    assert!(super::parse_heights("10,-2,2").is_err());
    assert!(super::parse_heights("10,2,NaN").is_err());
}

#[test]
fn relative_humidity_can_supply_the_specific_humidity_slot() {
    let dir = std::env::temp_dir().join("colm-convert-relative-humidity");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = tiny_met(&dir, "Tair", &[293.15, 293.15]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.variable_mut("Tair")
            .unwrap()
            .put_attribute("units", "K")
            .unwrap();
        let mut rh = file.add_variable::<f64>("RH", &["time"]).unwrap();
        rh.put_attribute("units", "%").unwrap();
        rh.put_values(&[50.0, 60.0], netcdf::Extents::All).unwrap();
        let mut pressure = file.add_variable::<f64>("Psurf", &["time"]).unwrap();
        pressure.put_attribute("units", "Pa").unwrap();
        pressure
            .put_values(&[100_000.0, 100_000.0], netcdf::Extents::All)
            .unwrap();
    }
    let slots = [(1, "Tair", "K"), (2, "RH", "%"), (3, "Psurf", "Pa")]
        .into_iter()
        .map(|(index, name, units)| super::SlotPlan {
            index,
            source_name: name.into(),
            source_units: units.into(),
            also_add: Vec::new(),
        })
        .collect();
    let dst = dir.join("out.nc");
    super::convert(
        &src,
        &dst,
        &super::Plan {
            slots,
            heights: None,
        },
    )
    .unwrap();
    let output = netcdf::open(dst).unwrap();
    let humidity: Vec<f64> = output.variable("Qair").unwrap().get_values(..).unwrap();
    assert!(humidity[0] > 0.0 && humidity[0] < humidity[1]);
}

#[test]
fn invalid_derived_humidity_is_not_written_as_nan() {
    let dir = std::env::temp_dir().join("colm-convert-invalid-relative-humidity");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = tiny_met(&dir, "Tair", &[293.15, 293.15]);
    {
        let mut file = netcdf::append(&src).unwrap();
        file.variable_mut("Tair")
            .unwrap()
            .put_attribute("units", "K")
            .unwrap();
        let mut rh = file.add_variable::<f64>("RH", &["time"]).unwrap();
        rh.put_attribute("units", "%").unwrap();
        rh.put_values(&[150.0, 50.0], ..).unwrap();
        let mut pressure = file.add_variable::<f64>("Psurf", &["time"]).unwrap();
        pressure.put_attribute("units", "Pa").unwrap();
        pressure.put_values(&[100_000.0, 100_000.0], ..).unwrap();
    }
    let slots = [(1, "Tair", "K"), (2, "RH", "%"), (3, "Psurf", "Pa")]
        .into_iter()
        .map(|(index, name, units)| super::SlotPlan {
            index,
            source_name: name.into(),
            source_units: units.into(),
            also_add: Vec::new(),
        })
        .collect();
    let dst = dir.join("out.nc");
    let error = super::convert(
        &src,
        &dst,
        &super::Plan {
            slots,
            heights: None,
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("non-finite"));
    assert!(
        !dst.exists(),
        "failed conversion must not leave a partial product"
    );
}

#[test]
fn heights_given_by_hand_land_in_the_product() {
    // Urban-PLUMBER 的 21 个站都没有 reference_height_*，而 CoLM 要它们。
    // 界面上让人填，填了就要写进产物 —— **产物必须自包含**，不能只写
    // 进这一次的 forcing.nml，否则下次拿这份文件重建算例又是 NaN，
    // 而 NaN 的下场是 SIGILL。
    let dir = std::env::temp_dir().join("colm-convert-heights");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("noheight_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("Tair", &["time"]).unwrap();
        v.put_attribute("units", "K").unwrap();
        v.put_values(&[273.15, 274.15], netcdf::Extents::All)
            .unwrap();
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "Tair".into(),
            source_units: "K".into(),
            also_add: Vec::new(),
        }],
        heights: Some(super::Heights {
            v: 48.05,
            t: 48.05,
            q: 48.05,
        }),
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    for (name, want) in [
        ("reference_height_v", 48.05),
        ("reference_height_t", 48.05),
        ("reference_height_q", 48.05),
    ] {
        let got: Vec<f64> = f
            .variable(name)
            .unwrap_or_else(|| panic!("{name} 该被写进产物"))
            .get_values(netcdf::Extents::All)
            .unwrap();
        assert_eq!(got, vec![want]);
    }
}

#[test]
fn heights_already_in_the_source_are_not_overwritten() {
    // **源文件说了的，界面不该覆盖。** PLUMBER2 的 90 个站都带着这三个
    // 标量，转换时原样搬（`191fea7` 那条规则），手填只在源文件没有时用。
    // 反过来会让「量出来的」被「填进去的」悄悄换掉。
    let dir = std::env::temp_dir().join("colm-convert-heights-keep");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("hasheight_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("Tair", &["time"]).unwrap();
        v.put_attribute("units", "K").unwrap();
        v.put_values(&[273.15, 274.15], netcdf::Extents::All)
            .unwrap();
        // 源文件量出来的高度：6.0
        let mut h = f.add_variable::<f64>("reference_height_t", &[]).unwrap();
        h.put_attribute("units", "m").unwrap();
        h.put_values(&[6.0], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "Tair".into(),
            source_units: "K".into(),
            also_add: Vec::new(),
        }],
        // 界面填了 99，但源文件已经有 reference_height_t —— 不该被覆盖。
        heights: Some(super::Heights {
            v: 99.0,
            t: 99.0,
            q: 99.0,
        }),
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    let got: Vec<f64> = f
        .variable("reference_height_t")
        .expect("源文件带着的要保留")
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(got, vec![6.0], "源文件量出来的高度不该被界面填的覆盖");
}

// parse_slot_spec / parse_heights 本来在 forcing-convert.rs 里各写一份。
// colm-cli 的 forcing-convert 子命令要同一套解析，所以抽出来共用——
// 放在这里而不是 slots_tests.rs：`SlotPlan`/`Heights` 是 convert.rs 里
// 的类型，让 slots.rs 反过来 `use crate::convert::SlotPlan` 会让本来
// 单向的依赖（convert -> slots）变成互相依赖，所以解析函数留在
// convert.rs，测试跟着放这里。

#[test]
fn a_slot_spec_parses_into_a_plan_entry() {
    let p = super::parse_slot_spec("4=Rainf:kg/m2/s+Snowf").expect("解析");
    assert_eq!(p.index, 4);
    assert_eq!(p.source_name, "Rainf");
    assert_eq!(p.source_units, "kg/m2/s");
    assert_eq!(p.also_add, vec!["Snowf".to_string()]);
}

#[test]
fn a_slot_spec_without_a_plus_has_nothing_to_add() {
    let p = super::parse_slot_spec("1=TA_F:degC").expect("解析");
    assert_eq!(p.index, 1);
    assert!(p.also_add.is_empty());
}

#[test]
fn a_malformed_slot_spec_says_what_it_wanted() {
    // **报错要说出正确的形状**，不能只说「格式错误」——
    // 用户下一步要用的正是那个形状。
    for bad in ["4", "4=Rainf", "x=Rainf:mm/s"] {
        let e = super::parse_slot_spec(bad).unwrap_err().to_string();
        assert!(
            e.contains("N=name:units"),
            "{bad:?} 的报错要给出形状，得到：{e}"
        );
    }
}

#[test]
fn heights_parse_in_order() {
    let h = super::parse_heights("48.05,48.05,48.05").expect("解析");
    assert_eq!((h.v, h.t, h.q), (48.05, 48.05, 48.05));
}

#[test]
fn a_malformed_height_spec_says_what_it_wanted() {
    let e = super::parse_heights("1,2").unwrap_err().to_string();
    assert!(
        e.contains("three"),
        "数量不对的报错要说清楚要三个数，得到：{e}"
    );
}

#[test]
fn a_product_directory_that_does_not_exist_yet_is_created() {
    // **产物目录不存在时建出来，而不是报错。**
    //
    // 界面给的默认产物目录是 `~/CoLM-forcing`，第一次用必然不存在 ——
    // 而 `netcdf::create` 只建文件不建目录，于是「不用打字直接点转换」
    // 这条路径直接撞 `No such file or directory`。
    //
    // 真机验收才发现的：单测一直往 `std::env::temp_dir()` 写，那个目录
    // 永远存在。**测试用的路径太顺，就测不到路径本身的问题。**
    let root = std::env::temp_dir().join("colm-convert-mkdir");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let p = root.join("src_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("Tair", &["time"]).unwrap();
        v.put_attribute("units", "K").unwrap();
        v.put_values(&[273.15, 274.15], netcdf::Extents::All)
            .unwrap();
    }

    // 两层都不存在 —— 用户可能填一个从没建过的路径。
    let dst = root.join("never/made/out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "Tair".into(),
            source_units: "K".into(),
            also_add: Vec::new(),
        }],
        heights: None,
    };
    super::convert(&p, &dst, &plan).expect("产物目录该被建出来");
    assert!(dst.exists(), "产物没写出来");
}

#[test]
fn a_variable_listed_both_as_source_and_in_also_add_is_refused() {
    // **后端要独立拦住这个，不能只靠界面。**
    //
    // 界面上修过一次（`5d42291`：主变量换成原来的额外变量时清掉它），
    // 但那是唯一的防线 —— 而它防的正是界面自己的 bug。真机验收实测：
    // 拿修复前会产生的那份 payload（`4=Snowf:kg/m2/s+Snowf`）跑后端，
    // **Precip = 2× Snowf 精确，退出码 0，一句警告都没有**。
    //
    // 降水翻倍，模型跑得完，曲线照样是曲线。
    let dir = std::env::temp_dir().join("colm-convert-dup");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("dup_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00")
            .unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("Snowf", &["time"]).unwrap();
        v.put_attribute("units", "kg/m2/s").unwrap();
        v.put_values(&[1.0, 2.0], netcdf::Extents::All).unwrap();
    }

    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 4,
            source_name: "Snowf".into(),
            source_units: "kg/m2/s".into(),
            also_add: vec!["Snowf".into()],
        }],
        heights: None,
    };
    let e = super::convert(&p, &dir.join("out.nc"), &plan).unwrap_err();
    let m = e.to_string();
    assert!(m.contains("Snowf"), "报错要点名那个变量：{m}");
    assert!(m.contains("twice"), "要说清楚后果是加了两次：{m}");
}
