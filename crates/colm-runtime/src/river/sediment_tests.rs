use super::*;

#[test]
fn critical_shear_squares_follow_the_size_bands() {
    // 粗砂一段是一次幂，只乘系数（编译期把 `**1` 去掉）。
    assert_eq!(critical_shear_vel_sq(0.004), 80.9 * (0.004 * 100.0));
    assert_eq!(
        critical_shear_vel_sq(0.0002),
        8.41 * (0.0002f64 * 100.0).lpow(11.0 / 32.0)
    );
    assert_eq!(critical_shear_vel_sq(0.00001), 226.0 * (0.00001 * 100.0));
}

#[test]
fn water_accumulator_keeps_the_first_start_and_last_end() {
    let mut acc = WaterAcc::default();
    let water = |start, end| SubstepWater {
        start,
        end,
        ..Default::default()
    };
    acc.add(10.0, 2.0, 1.0, water(5.0, 6.0), -3.0, 7.0).unwrap();
    acc.add(10.0, 1.0, 1.0, water(6.0, 8.0), 1.0, 7.0).unwrap();
    assert_eq!(acc.rivsto_start, 5.0);
    assert_eq!(acc.rivsto_end, 8.0);
    assert_eq!(acc.time, 20.0);
    assert_eq!(acc.v2, 50.0);
    assert_eq!(acc.rivout, -20.0);
    assert_eq!(acc.abs_rivout, 40.0);
}

#[test]
fn log10_19_is_the_folded_constant() {
    assert_eq!(LOG10_19, 19f64.log10());
}

/// 两个单元流域：0 流向 1，1 是出口。床沙充足、剪切相同。
fn two_cell_sediment(tag: &str) -> (RiverNetwork, Sediment) {
    use crate::river::network::FloodplainCurve;
    let rivhgt = 2.0;
    let (rivwth, rivlen, area) = (50.0, 10_000.0, 2.0e7);
    let rivstomax = rivhgt * rivwth * rivlen;
    let fldhgt: Vec<f64> = (1..=10).map(|j| 0.5 * j as f64).collect();
    let curve = FloodplainCurve::new(rivhgt, rivstomax, area, &fldhgt, true);
    let network = RiverNetwork {
        x: vec![1, 2],
        y: vec![1, 1],
        next: vec![1, -9],
        upstream: vec![Vec::new(), vec![0]],
        river_system: vec![0, 0],
        river_systems: 1,
        systems: Vec::new(),
        rivelv: vec![0.0; 2],
        rivhgt: vec![rivhgt; 2],
        rivlen: vec![rivlen; 2],
        rivman: vec![0.03; 2],
        rivwth: vec![rivwth; 2],
        rivare: vec![curve.rivare; 2],
        rivstomax: vec![rivstomax; 2],
        area: vec![area; 2],
        curves: vec![curve.clone(), curve],
        bedelv_next: vec![0.0; 2],
        outletwth: vec![rivwth; 2],
        nlon: 2,
        nlat: 1,
        inpmat: vec![Vec::new(), Vec::new()],
    };
    let dir = std::env::temp_dir().join(format!("colm-sed-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let unitcat = dir.join("unitcat.nc");
    {
        let mut file = netcdf::create(&unitcat).unwrap();
        file.add_dimension("nseqmax", 2).unwrap();
        file.add_dimension("sed_n", 3).unwrap();
        file.add_dimension("slope_layers", 10).unwrap();
        file.add_variable::<f64>("sed_frc", &["nseqmax", "sed_n"])
            .unwrap()
            .put_values(&[1.0 / 3.0; 6], ..)
            .unwrap();
        file.add_variable::<f64>("sed_slope", &["nseqmax", "slope_layers"])
            .unwrap()
            .put_values(&[0.01; 20], ..)
            .unwrap();
    }
    let param = dir.join("sediment.nml");
    std::fs::write(
        &param,
        include_str!("../../../../vendor/CoLM202X/run/standard_sediment_parameter.nml"),
    )
    .unwrap();
    let mut sediment = Sediment::init(&network, &unitcat, param.to_str().unwrap()).unwrap();
    // 两侧剪切速度相同，且高于所有粒径的临界值。
    sediment.shearvel = vec![0.5; 2];
    sediment.critshearvel = vec![0.01; 6];
    (network, sediment)
}

/// upstream-bugs #80：正反向输运的推移质按方向份额加权。反向份额从 0 增到 1e-6，
/// 毛推移质只多出约 1e-6，而不是再加一份满强度的推移质；单向流不受影响。
#[test]
fn bedload_scales_with_the_share_of_each_flow_direction() {
    let (network, mut sediment) = two_cell_sediment("bedload");
    let bed = vec![1.0; 6];
    let conc = vec![0.0; 6];
    let total = |demand: &[f64]| demand.iter().sum::<f64>();
    let one_way =
        sediment.donor_demand(&network, 1800.0, &[100.0, 0.0], &[100.0, 0.0], &conc, &bed);
    let base = total(&one_way.1);
    assert!(base > 0.0);
    // 反向流量 1e-4、正向 100-1e-4：反向份额 1e-6。
    let mixed = sediment.donor_demand(
        &network,
        1800.0,
        &[100.0 - 2.0e-4, 0.0],
        &[100.0, 0.0],
        &conc,
        &bed,
    );
    let gross = total(&mixed.1);
    assert!(
        (gross / base - 1.0).abs() < 1.0e-5,
        "gross bedload {gross} vs one-way {base}"
    );
    // 下游单元流域（反向面的供体）只承担约 1e-6 的份额。
    let reverse_donor: f64 = mixed.1[3..6].iter().sum();
    assert!(
        reverse_donor > 0.0 && reverse_donor < 1.0e-5 * base,
        "{reverse_donor}"
    );
}
