use super::*;

/// 一个均匀剖面，8 层。三种基准各自独立，所以刻意取互不相干的数：
/// 若某条公式误用了别的基准的量，结果会立刻偏离。
fn uniform() -> SoilColumn {
    SoilColumn {
        vf_sand: vec![0.30; 8],
        vf_gravels: vec![0.10; 8],
        vf_om: vec![0.02; 8],
        wf_sand: vec![0.40; 8],
        om_density: vec![26.0; 8],
        bd_all: vec![1300.0; 8],
    }
}

#[test]
fn the_soil_layer_thicknesses_are_colms() {
    // CoLM 标准 10 层，srfdata 只用前 8 层，累计到第 8 层是 1.3829 m
    assert_eq!(DZ_SOIL.len(), 8);
    let total: f64 = DZ_SOIL.iter().sum();
    assert!((total - 1.3829).abs() < 1e-9, "got {total}");
}

#[test]
fn only_the_top_sixty_centimetres_carry_weight() {
    // 0-60cm 深度加权：第 8 层的顶已在 60cm 以下，权重必须是 0
    let w = depth_weights(0.60);
    assert_eq!(w.len(), 8);
    assert!(w[0] > 0.0);
    assert_eq!(w[7], 0.0, "layer 8 starts below 60 cm");
    let total: f64 = w.iter().sum();
    assert!(
        (total - 0.60).abs() < 1e-12,
        "weights should sum to 0.60, got {total}"
    );
}

#[test]
fn wf_om_is_colms_own_identity_not_a_product_of_three_things() {
    // OM_density = BD_ave * wf_om_s * 1000 且 BD_all = BD_ave * 1000
    // （rawdata_soil_solids_fractions.F90），所以 wf_om = OM_density / BD_all。
    // 一个看起来同样合理的写法 vf_om * OM_density / BD_all 会小两个数量级。
    let c = uniform();
    let d = derive(&c);
    assert!((d.wf_om[0] - 26.0 / 1300.0).abs() < 1e-15, "{}", d.wf_om[0]);
}

#[test]
fn a_zero_bulk_density_does_not_produce_infinity() {
    // 除以 0 会得到 inf，写进 netcdf 之后 CoLM 会拿它去算能量平衡。
    let mut c = uniform();
    c.bd_all[3] = 0.0;
    let d = derive(&c);
    assert!(d.wf_om[3].is_finite(), "got {}", d.wf_om[3]);
    assert_eq!(d.wf_om[3], 0.0);
}

#[test]
fn wf_clay_shares_wf_sands_basis_and_vf_clay_shares_the_volume_one() {
    // 这两条基准不同，是本 Task 的全部要点。混用会在有机质丰富或多砾石的
    // 站点上算出负的剩余量 —— 实测 17/90 个站点会因此失败。
    let c = uniform();
    let d = derive(&c);
    assert!(
        (d.wf_clay[0] - 0.25 * (1.0 - 0.40)).abs() < 1e-15,
        "{}",
        d.wf_clay[0]
    );
    assert!(
        (d.vf_clay[0] - 0.25 * (1.0 - 0.30 - 0.10 - 0.02)).abs() < 1e-15,
        "{}",
        d.vf_clay[0]
    );
}

#[test]
fn a_gravelly_organic_soil_still_produces_usable_fractions() {
    // US-NR1 的实测形态：wf_sand 0.82 与 wf_gravels 0.5488 并存，
    // 因为两者基准不同。按同基准去减会得到负数。
    let mut c = uniform();
    c.wf_sand = vec![0.82; 8];
    c.om_density = vec![1200.0; 8];
    c.bd_all = vec![1300.0; 8];
    let d = derive(&c);
    let f = fine_earth_fractions(&c);
    for v in d
        .vf_clay
        .iter()
        .chain(d.wf_clay.iter())
        .chain(d.wf_om.iter())
    {
        assert!((0.0..=1.0).contains(v), "fraction out of range: {v}");
    }
    assert!(f.sand >= 0.0 && f.silt >= 0.0 && f.clay >= 0.0, "{f:?}");
    assert!((f.sand + f.silt + f.clay - 100.0).abs() < 1e-9);
}

#[test]
fn the_triangle_gets_fine_earth_with_no_gravel_or_organics_subtracted() {
    // 质地三角描述的是细土。wf_sand 已经是细土分数（rd_soil_properties.F90:504），
    // 再去减砾石与有机质就是把两套基准混在一起。
    let c = uniform();
    let f = fine_earth_fractions(&c);
    assert!((f.sand - 40.0).abs() < 1e-9, "sand {}", f.sand);
    assert!((f.clay - 15.0).abs() < 1e-9, "clay {}", f.clay);
    assert!((f.silt - 45.0).abs() < 1e-9, "silt {}", f.silt);
}

#[test]
fn a_short_profile_does_not_run_off_the_end() {
    // 实测的站点文件是 10 层，深度权重是 8 个。层数更少的文件不该以
    // 数组越界 panic 收场 —— 那种报错指向的地方与真正的原因毫无关系。
    let mut c = uniform();
    for v in [
        &mut c.vf_sand,
        &mut c.vf_gravels,
        &mut c.vf_om,
        &mut c.wf_sand,
        &mut c.om_density,
        &mut c.bd_all,
    ] {
        v.truncate(3);
    }
    let d = derive(&c);
    assert_eq!(d.vf_clay.len(), 3);
    let f = fine_earth_fractions(&c);
    assert!((f.sand + f.silt + f.clay - 100.0).abs() < 1e-9);
}
