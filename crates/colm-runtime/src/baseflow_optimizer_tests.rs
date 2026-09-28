use super::*;

fn scratch_directory(tag: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "colm-baseflow-optimizer-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn read_scalar(path: &Path, name: &str) -> f64 {
    let file = netcdf::open(path).unwrap();
    let values: Vec<f64> = file
        .variable(name)
        .unwrap()
        .get_values(netcdf::Extents::All)
        .unwrap();
    assert_eq!(values.len(), 1, "{name} should be a one-patch vector");
    values[0]
}

fn step(recharge_mm_s: f64, subsurface_mm_s: f64) -> BaseflowStep {
    BaseflowStep {
        convective_precipitation_kg_m2_s: recharge_mm_s,
        large_scale_precipitation_kg_m2_s: 0.0,
        total_evaporation_kg_m2_s: 0.0,
        surface_runoff_mm_s: 0.0,
        subsurface_runoff_mm_s: subsurface_mm_s,
        time_step_seconds: 1800.0,
    }
}

/// 两个条件各自成立时按 `rchg/rsub` 缩放；方向不对（水位变深却补给多于基流）不动。
#[test]
fn scale_moves_only_when_the_water_table_trend_and_the_budget_agree() {
    // 水位变深（3.5 > 3.0）且补给 < 基流：按比例减小。
    assert_eq!(
        adjusted_scale(1.0, true, 3.5, 3.0, 50.0, 200.0),
        50.0 / 200.0
    );
    // 水位变浅且补给 > 基流：按比例增大。
    assert_eq!(
        adjusted_scale(0.5, true, 2.5, 3.0, 300.0, 100.0),
        300.0 / 100.0 * 0.5
    );
    // 水位变深但补给 > 基流：两个条件都不成立。
    assert_eq!(adjusted_scale(0.5, true, 3.5, 3.0, 300.0, 100.0), 0.5);
    // 补给为负（年蒸散大于降水）：`rchg > 0` 不成立，只剩下限。
    assert_eq!(adjusted_scale(0.7, true, 3.5, 3.0, -10.0, 100.0), 0.7);
    // 下限 1e-8。
    assert_eq!(adjusted_scale(1.0e-9, true, 3.0, 3.0, -1.0, -1.0), 1.0e-8);
    // `patchtype > 1`（湿地、冰川、湖）整段跳过，连下限都不套。
    assert_eq!(adjusted_scale(1.0e-9, false, 3.5, 3.0, 50.0, 200.0), 1.0e-9);
}

/// 累加器从 `spval` 起步：第一笔直接取 `var*dt`，之后 `s + var*dt`；
/// 跨年写两份文件并复位，`iter_bf_opt` 递增。
#[test]
fn a_year_writes_the_cycle_record_then_the_updated_scale() {
    let directory = scratch_directory("year");
    let mut optimizer = BaseflowOptimizer::new(1.0, 3.0, 0, &directory, "site");
    optimizer.accumulate(step(2.0e-5, 4.0e-5));
    optimizer.accumulate(step(1.0e-5, 4.0e-5));
    optimizer.close_year(3.5).unwrap();

    let recharge = 2.0e-5 * 1800.0 + 1.0e-5 * 1800.0;
    let subsurface = 4.0e-5 * 1800.0 + 4.0e-5 * 1800.0;
    let cycle = directory.join("c0001").join("site_baseflow_w180_s90.nc");
    assert_eq!(read_scalar(&cycle, "zwt"), 3.5);
    assert_eq!(read_scalar(&cycle, "zwt_init"), 3.0);
    assert_eq!(read_scalar(&cycle, "scale_baseflow"), 1.0);
    assert_eq!(read_scalar(&cycle, "total_recharge"), recharge);
    assert_eq!(read_scalar(&cycle, "total_subsurface_runoff"), subsurface);

    // 水位变深、补给不及基流 ⇒ 缩放。
    let expected = recharge / subsurface * 1.0;
    assert_eq!(optimizer.scale(), expected);
    let main = directory.join("site_baseflow_w180_s90.nc");
    assert_eq!(read_scalar(&main, "scale_baseflow"), expected);
    assert_eq!(optimizer.iteration(), 1);

    // 复位后的一年没有累加任何一步：两个总量写成 `spval`，比例不动。
    optimizer.close_year(3.5).unwrap();
    let second = directory.join("c0002").join("site_baseflow_w180_s90.nc");
    assert_eq!(read_scalar(&second, "total_recharge"), SPVAL);
    assert_eq!(read_scalar(&second, "total_subsurface_runoff"), SPVAL);
    assert_eq!(read_scalar(&second, "scale_baseflow"), expected);
    assert_eq!(optimizer.scale(), expected);
    std::fs::remove_dir_all(&directory).unwrap();
}
