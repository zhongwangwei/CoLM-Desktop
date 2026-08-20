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
