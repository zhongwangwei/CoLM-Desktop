//! 一次性现场证据：验证第 4 槽多源合成在真实 Urban-PLUMBER 文件上是对的。
//!
//! 这不是自造的测试数据 —— `FI-Kumpula_metforcing_v1.nc` 是真实站点文件，
//! 245469 个时刻、`float32`、`(time, y, x)` 形状。这条 example 用它验两件事：
//!
//! 1. `also_add` 把 `Rainf` + `Snowf` 合成的总量，与源文件里两者的总量
//!    是否相等（应当相等，因为求和是逐时刻做的，总量自然守恒）。
//! 2. 雪占比是否落在已实测的 24.7% 附近 —— 这是 bug 存在时被丢掉的那部分。
//!
//! 跑法：`cargo run -p colm-forcing --example sum_urban_precip`

use std::path::PathBuf;

use colm_forcing::convert::{convert, Plan, SlotPlan};

fn main() -> anyhow::Result<()> {
    let home = std::env::var("HOME").expect("HOME not set");
    let src = PathBuf::from(home)
        .join("Desktop/colm-rust/Urban-PLUMBER/Forcing/FI-Kumpula_metforcing_v1.nc");
    let dst = std::env::temp_dir().join("fi-kumpula-sum.nc");

    println!("source: {}", src.display());
    println!("dest:   {}", dst.display());

    // 源文件里的 Rainf / Snowf 总量，转换前先自己算一遍，
    // 这样才能独立核对 convert() 算出来的 Precip 是不是真的等于两者之和。
    let fin = netcdf::open(&src)?;
    let rainf: Vec<f64> = fin
        .variable("Rainf")
        .expect("no Rainf")
        .get_values(netcdf::Extents::All)?;
    let snowf: Vec<f64> = fin
        .variable("Snowf")
        .expect("no Snowf")
        .get_values(netcdf::Extents::All)?;
    drop(fin);

    let rainf_total: f64 = rainf.iter().sum();
    let snowf_total: f64 = snowf.iter().sum();
    let src_total = rainf_total + snowf_total;
    let snow_fraction = snowf_total / src_total * 100.0;

    println!("Rainf steps: {}", rainf.len());
    println!("Snowf steps: {}", snowf.len());
    println!("源 Rainf 总量: {rainf_total}");
    println!("源 Snowf 总量: {snowf_total}");
    println!("源 Rainf+Snowf 总量: {src_total}");
    println!("雪占比: {snow_fraction:.4}%");

    let plan = Plan {
        slots: vec![SlotPlan {
            index: 4,
            source_name: "Rainf".into(),
            source_units: "kg/m2/s".into(),
            also_add: vec!["Snowf".into()],
        }],
        heights: None,
    };
    convert(&src, &dst, &plan)?;

    let fout = netcdf::open(&dst)?;
    let precip: Vec<f64> = fout
        .variable("Precip")
        .expect("产物没有 Precip")
        .get_values(netcdf::Extents::All)?;
    let precip_total: f64 = precip.iter().sum();

    println!("产物 Precip 步数: {}", precip.len());
    println!("产物 Precip 总量: {precip_total}");
    println!("差值 (产物 - 源Rainf+Snowf): {}", precip_total - src_total);

    // **两个源变量必须还在** —— 转换可以增加信息，不能减少信息
    let rainf_kept = fout.variable("Rainf").is_some();
    let snowf_kept = fout.variable("Snowf").is_some();
    println!("产物里保留了 Rainf: {rainf_kept}");
    println!("产物里保留了 Snowf: {snowf_kept}");

    assert!(rainf_kept, "Rainf 必须保留在产物里");
    assert!(snowf_kept, "Snowf 必须保留在产物里");
    assert_eq!(precip.len(), rainf.len(), "步数应当一致");

    let diff = (precip_total - src_total).abs();
    assert!(
        diff < 1e-3,
        "产物 Precip 总量应当约等于源 Rainf+Snowf 总量，实际差 {diff}"
    );

    println!("OK: Precip = Rainf + Snowf，雪占比约 {snow_fraction:.1}%，两个源变量都保留了");
    Ok(())
}
