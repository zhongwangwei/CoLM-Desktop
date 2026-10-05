use super::*;

fn one(total: f64, qmean: f64, qflood: f64) -> CatchReservoirs {
    let normal = total * 0.7;
    let qnormal = qmean.mul_add(0.25, (normal * 0.7) / 1.5552e7);
    CatchReservoirs {
        bsn2resv: vec![Some(0)],
        dam_elv: vec![10.0],
        volresv_total: vec![total],
        volresv_emerg: vec![total * 0.94],
        volresv_adjust: vec![total * 0.77],
        volresv_normal: vec![normal],
        qresv_mean: vec![qmean],
        qresv_flood: vec![qflood],
        qresv_adjust: vec![(qnormal + qflood) * 0.5],
        qresv_normal: vec![qnormal],
        dam_build_year: vec![1990],
        resv_hylak_id: vec![7],
        resv_loc2glb: vec![0],
    }
}

#[test]
fn operation_follows_the_four_storage_zones() {
    let r = one(1.0e8, 50.0, 400.0);
    let (normal, adjust, emerg) = (r.volresv_normal[0], r.volresv_adjust[0], r.volresv_emerg[0]);
    // 应急区：至少放洪水流量，入流更大时放入流。
    assert_eq!(r.operation(0, 10.0, emerg + 1.0), 400.0);
    assert_eq!(r.operation(0, 900.0, emerg + 1.0), 900.0);
    // 正常区以下按 sqrt(vol/normal) 缩减。
    let low = r.operation(0, 10.0, normal * 0.25);
    assert!((low - 0.5 * r.qresv_normal[0]).abs() < 1e-12);
    // 正常—调节区与调节—应急区在区间端点连续。
    let at_adjust = r.operation(0, 10.0, adjust);
    assert!((at_adjust - r.qresv_adjust[0]).abs() < 1e-9);
    let mid = r.operation(0, 10.0, 0.5 * (adjust + emerg));
    assert!(mid > r.qresv_adjust[0] && mid < r.qresv_flood[0]);
}

#[test]
fn gather_adds_reservoirs_of_one_lake_and_skips_missing() {
    let mut r = one(1.0, 1.0, 1.0);
    r.resv_hylak_id = vec![3, 7];
    r.resv_loc2glb = vec![1, 1, 0];
    assert_eq!(r.gather(&[1.0, 2.0, SPVAL]), vec![SPVAL, 3.0]);
}
