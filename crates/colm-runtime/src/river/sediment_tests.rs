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
    let curve = FloodplainCurve::new(rivhgt, rivstomax, area, &fldhgt);
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

/// 高换水算例的子步数 = `ceil(毛出流·dt / 供体水量)`，且供体水量不高于
/// `SED_MIN_SUBSTEP_CARRIER` 的单元不参与计数。
#[test]
fn material_substeps_follow_the_donor_outflow_cfl() {
    let (network, _sediment) = two_cell_sediment("substep-forward");
    // 0→1，单元 0 正向毛出流 3 m³/s、供体水量 1 m³、区间 1 s → 3 个子步。
    let count = |signed: &[f64], abs: &[f64], donor: &[f64]| {
        Sediment::material_substep_count(&network, 1.0, signed, abs, donor).unwrap()
    };
    assert_eq!(count(&[3.0, 0.0], &[3.0, 0.0], &[1.0, 1.0]), 3);
    assert_eq!(count(&[3.0, 0.0], &[3.0, 0.0], &[1.5, 1.5]), 2);
    // 毛出流 ≤ 供体水量（低换水）不细分。
    assert_eq!(count(&[0.5, 0.0], &[0.5, 0.0], &[1.0, 1.0]), 1);
    // 近干单元的水量除以它没有物理意义，不能把整个河系拖到子步上限。
    assert_eq!(
        count(
            &[1000.0, 0.0],
            &[1000.0, 0.0],
            &[SED_MIN_SUBSTEP_CARRIER, 1000.0]
        ),
        1
    );
}

/// 子步分母要含反向面：反向面排的是它下游单元流域的水，所以毛出流记在下游供体上。
#[test]
fn material_substeps_count_reverse_faces_at_their_donor() {
    let (network, _sediment) = two_cell_sediment("substep-reverse");
    // 单元 0 反向流 100（排的是下游单元流域 1 的水），单元 1 无流：
    // 供体 1 的毛出流 = 100 → ceil(100·1/1) = 100 个子步；只看 rivout_abs 会算成 0。
    let reverse = Sediment::material_substep_count(
        &network,
        1.0,
        &[-100.0, 0.0],
        &[100.0, 0.0],
        &[1.0e6, 1.0],
    )
    .unwrap();
    assert_eq!(reverse, 100);
    // 同一个供体上正向出流与上游反向面叠加：单元 1 正向 40 + 单元 0 反向 100 → 140。
    let both = Sediment::material_substep_count(
        &network,
        1.0,
        &[-100.0, 40.0],
        &[100.0, 40.0],
        &[1.0e6, 1.0],
    )
    .unwrap();
    assert_eq!(both, 140);
}

/// 240 是硬上限：正好 240 允许，超过（哪怕上限溢出）都报错而不是静默截断。
#[test]
fn material_substep_limit_is_a_hard_error() {
    let (network, _sediment) = two_cell_sediment("substep-limit");
    let count = |signed: &[f64]| {
        Sediment::material_substep_count(&network, 1.0, signed, signed, &[1.0, 1.0])
    };
    assert_eq!(
        count(&[SED_MAX_SUBSTEPS as f64, 0.0]).unwrap(),
        SED_MAX_SUBSTEPS
    );
    // 超一步就报错（上游的 `min(cfl, 241)` + `ceiling` 把 240.x 也算成 241）。
    let over = SED_MAX_SUBSTEPS as f64 + 1.0;
    let error = count(&[over, 0.0]).unwrap_err().to_string();
    assert!(error.contains(&SED_MAX_SUBSTEPS.to_string()), "{error}");
    // `CEILING` 之前的 `min(…, 241)` 挡住整数溢出：天文数字也只报超限，不会回绕成小步数。
    assert!(count(&[1.0e300, 0.0]).is_err());
}

/// 低换水（`nsub == 1`）必须保持移植前的单步结果：这里钉死手算的数值。
#[test]
fn low_turnover_advection_keeps_the_single_donor_exchange() {
    let (network, mut sediment) = two_cell_sediment("single-step");
    // 关掉推移质，只留悬沙对流：毛出流 0.5 m³/s ≤ 供体水量 1 m³、区间 1 s → nsub = 1。
    sediment.shearvel = vec![0.0; 2];
    sediment.critshearvel = vec![1.0e20; 6];
    let per_grain = 0.001;
    for s in 0..3 {
        sediment.sedsto[s] = per_grain;
    }
    let signed = [0.5, 0.0];
    let abs = [0.5, 0.0];
    let rivsto = [1.0, 1.0];
    assert_eq!(
        Sediment::material_substep_count(&network, 1.0, &signed, &abs, &rivsto).unwrap(),
        1
    );
    sediment
        .advection(&network, 1.0, &signed, &abs, &rivsto, &rivsto, None)
        .unwrap();
    // 单步：浓度 = 悬沙/供体水量，通量 = 浓度·流量，一次散度更新。
    let concentration = per_grain / rivsto[0];
    let exported = concentration * signed[0] * 1.0;
    for s in 0..3 {
        assert_eq!(sediment.sedout[s], concentration * signed[0]);
        assert_eq!(sediment.sedsto[s], per_grain - exported);
        assert_eq!(sediment.sedsto[3 + s], exported);
        assert_eq!(sediment.sedcon[s], exported / rivsto[0]);
        assert_eq!(sediment.sedcon[3 + s], exported / rivsto[0]);
    }
}

/// `nsub > 1` 时四个速率按 `Σ rate·dt_sub` 积分成区间平均，不能读最后一次子步。
#[test]
fn material_substeps_integrate_the_interval_rates() {
    let (network, mut sediment) = two_cell_sediment("substep-integral");
    let (ns, dt) = (sediment.p.nsed, 1.0);
    let signed = [3.0, 0.0];
    let abs = [3.0, 0.0];
    let rivsto_donor = [1.0, 1.0];
    // 落水段：载体水量在两个端点之间线性插值，子区间里的浓度随之变化。
    let rivsto_end = [0.5, 1.0];
    let nsub =
        Sediment::material_substep_count(&network, dt, &signed, &abs, &rivsto_donor).unwrap();
    assert_eq!(nsub, 3);
    // 供体悬沙超过浓度上限：第 1 个子步先按上限沉积（netflw/exch 非零）再被冲走，
    // 后面的子步已无沙可冲 —— 直接读最后一次子步会把整段速率记成 0。
    let each = 0.02 / ns as f64;
    for s in 0..ns {
        sediment.sedsto[s] = each;
    }
    let interp = |i: usize, frac: f64| {
        (rivsto_end[i] - rivsto_donor[i])
            .contract(frac, rivsto_donor[i])
            .max(0.0)
    };
    // 用与 `advection_substeps` 相同的插值与累加表达式显式复算。
    let mut expected = sediment.clone();
    let dt_sub = dt / nsub as f64;
    let mut netflw_int = vec![0.0; 2 * ns];
    let mut exch_int = vec![0.0; 2 * ns];
    let mut sedout_int = vec![0.0; 2 * ns];
    let mut bedout_int = vec![0.0; 2 * ns];
    for isub in 1..=nsub {
        let frac_lo = (isub - 1) as f64 / nsub as f64;
        let frac_hi = isub as f64 / nsub as f64;
        let w_don = (0..2).map(|i| interp(i, frac_lo)).collect::<Vec<f64>>();
        let w_end = (0..2).map(|i| interp(i, frac_hi)).collect::<Vec<f64>>();
        expected
            .advection(&network, dt_sub, &signed, &abs, &w_don, &w_end, None)
            .unwrap();
        for k in 0..2 * ns {
            netflw_int[k] = expected.netflw_adv_step[k].contract(dt_sub, netflw_int[k]);
            exch_int[k] = expected.exch_d_adv_step[k].contract(dt_sub, exch_int[k]);
            sedout_int[k] = expected.sedout[k].contract(dt_sub, sedout_int[k]);
            bedout_int[k] = expected.bedout[k].contract(dt_sub, bedout_int[k]);
        }
    }
    let last_sedout = expected.sedout.clone();
    sediment
        .advection(
            &network,
            dt,
            &signed,
            &abs,
            &rivsto_donor,
            &rivsto_end,
            None,
        )
        .unwrap();
    for k in 0..2 * ns {
        assert_eq!(sediment.netflw_adv_step[k], netflw_int[k] / dt);
        assert_eq!(sediment.exch_d_adv_step[k], exch_int[k] / dt);
        assert_eq!(sediment.sedout[k], sedout_int[k] / dt);
        assert_eq!(sediment.bedout[k], bedout_int[k] / dt);
        // 末态与显式循环一致（子步顺序、插值、限流都相同）。
        assert_eq!(sediment.sedsto[k], expected.sedsto[k]);
        assert_eq!(sediment.layer[k], expected.layer[k]);
        assert_eq!(sediment.sedcon[k], expected.sedcon[k]);
    }
    // 积分值必须明显不同于最后一次子步的速率。
    assert!(sediment.sedout[0] > 1.0e-3, "{:e}", sediment.sedout[0]);
    assert!(last_sedout[0] < 1.0e-12, "{:e}", last_sedout[0]);
    assert!(sediment.netflw_adv_step[0] < -1.0e-3);
    assert!(sediment.exch_d_adv_step[0] > 1.0e-3);
}

/// 续跑只允许停在已完成的汇流窗口：`sed_acc_time > 0` 的检查点与上游一样报错。
#[test]
fn checkpoint_requires_a_closed_routing_window() {
    let (_network, mut sediment) = two_cell_sediment("checkpoint-window");
    // 冷启动的空累加器就是合法检查点。
    sediment.validate_checkpoint("write").unwrap();
    sediment.acc[0].time = 1.0;
    let error = sediment
        .validate_checkpoint("write")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unfinished routing window"), "{error}");
    // 时间清零后又合法（端点与累加量本来就在窗口闭合时一并清零）。
    sediment.acc[0].time = 0.0;
    sediment.validate_checkpoint("write").unwrap();
}
