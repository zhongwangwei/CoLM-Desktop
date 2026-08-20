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
