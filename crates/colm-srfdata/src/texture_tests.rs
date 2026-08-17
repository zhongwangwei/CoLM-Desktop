use super::*;

#[test]
fn the_class_numbering_is_colms_not_the_other_convention() {
    // preprocess/rawdata_soil_solids_fractions.F90:253-264。
    // 这个顺序不是可以自选的：MOD_Initialize.F90:420 用它直接索引 BVIC_USDA。
    assert_eq!(CLASS_NAMES[0], "clay");
    assert_eq!(CLASS_NAMES[7], "silty loam");
    assert_eq!(CLASS_NAMES[11], "sand");
    assert_eq!(CLASS_NAMES.len(), 12);
}

#[test]
fn bvic_matches_the_table_colm_indexes() {
    // MOD_Initialize.F90:271 的 BVIC_USDA(0:12)
    assert_eq!(BVIC_USDA[0], 1.0);
    assert_eq!(BVIC_USDA[1], 0.300);
    assert_eq!(BVIC_USDA[8], 0.100);
    assert_eq!(BVIC_USDA[12], 0.050);
    assert_eq!(BVIC_USDA.len(), 13);
}

#[test]
fn cn_cng_is_a_silty_loam_not_a_clay_loam() {
    // 这条是本 crate 存在的直接原因之一。实测 0-60cm 深度加权分数：
    // 砂 14.2760 / 粉 64.2930 / 黏 21.4310。
    // 先前的 Python 脚本用相反的编号给出 4（clay loam, BVIC 0.230），
    // 大了 2.3 倍；CoLM 自己的分类器给 8（silty loam, BVIC 0.100）。
    let c = classify(64.2930, 21.4310).expect("inside the triangle");
    assert_eq!(c, 8);
    assert_eq!(CLASS_NAMES[(c - 1) as usize], "silty loam");
    assert_eq!(BVIC_USDA[c as usize], 0.100);
}

#[test]
fn the_three_corners_classify_as_the_pure_textures() {
    // 三角形的三个角：纯黏、纯粉、纯砂
    assert_eq!(classify(0.0, 100.0), Some(1)); // clay
    assert_eq!(classify(100.0, 0.0), Some(10)); // silt
    assert_eq!(classify(0.0, 0.0), Some(12)); // sand
}

#[test]
fn a_point_outside_the_triangle_is_rejected_rather_than_guessed() {
    // silt + clay > 100 在物理上不存在。返回 None 而不是硬凑一个类 ——
    // MOD_SoilTextureReadin.F90:47 把越界值静默置 0，而 BVIC_USDA(0)=1.0，
    // 比任何正常类别都大三倍以上。宁可在这里停下。
    assert_eq!(classify(80.0, 80.0), None);
    assert_eq!(classify(-1.0, 50.0), None);
}

#[test]
fn a_point_on_a_shared_boundary_takes_the_higher_numbered_class() {
    // pointinpolygon 把顶点与边上都算作「在内」，所以公共边与公共顶点会
    // 同时命中多类。CoLM 的调用方是连续的 IF(c(k)) 赋值，后匹配覆盖先匹配
    // （rawdata_soil_solids_fractions.F90:192-201），即最大编号胜出。
    //
    // 下面三点的命中集是实测出来的，不是推的：
    //   (0, 55)    -> {1 clay, 3 sandy clay}                 -> 3
    //   (40, 60)   -> {1 clay, 2 silty clay}                 -> 2
    //   (50, 27.5) -> {4 clay loam, 7 loam, 8 silty loam}    -> 8
    assert_eq!(classify(0.0, 55.0), Some(3));
    assert_eq!(classify(40.0, 60.0), Some(2));
    assert_eq!(classify(50.0, 27.5), Some(8));
}

#[test]
fn every_class_is_reachable() {
    // 一个只会返回少数几类的分类器会让上面所有测试都过，却在真实语料上
    // 把大半站点判错。这里在三角形上密集撒点，要求 12 类都出现过。
    let mut seen = [false; 13];
    let n = 400;
    for i in 0..=n {
        for j in 0..=n {
            let silt = 100.0 * (i as f64) / (n as f64);
            let clay = 100.0 * (j as f64) / (n as f64);
            if silt + clay > 100.0 {
                continue;
            }
            if let Some(c) = classify(silt, clay) {
                seen[c as usize] = true;
            }
        }
    }
    let missing: Vec<usize> = (1..=12).filter(|k| !seen[*k]).collect();
    assert!(
        missing.is_empty(),
        "these classes were never produced: {missing:?}"
    );
}
