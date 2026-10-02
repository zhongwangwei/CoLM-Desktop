use super::*;

fn one_degree_grid() -> LatLonGrid {
    LatLonGrid::define_by_center(&[23.5, 24.5], &[113.5, 114.5], None).unwrap()
}

fn half_degree_pixels() -> PixelAxes {
    PixelAxes {
        lon_w: vec![113.0, 113.5, 114.0, 114.5],
        lon_e: vec![113.5, 114.0, 114.5, 115.0],
        lat_s: vec![23.0, 23.5, 24.0, 24.5],
        lat_n: vec![23.5, 24.0, 24.5, 25.0],
    }
}

/// 一个像元落在一个格子里：一份，面积就是像元面积；跨格子的 set 按 `(ilat, ilon)` 排序。
#[test]
fn parts_are_sorted_by_latitude_then_longitude() {
    let grid = one_degree_grid();
    let pixel = half_degree_pixels();
    let mapping = AreaWeightedMapping::build(
        &grid,
        &pixel,
        &[vec![(1, 1)], vec![(4, 4), (1, 1), (3, 1)]],
        &[1.0, 1.0],
    )
    .unwrap();
    assert_eq!(mapping.parts[0].len(), 1);
    assert_eq!((mapping.parts[0][0].ilon, mapping.parts[0][0].ilat), (0, 0));
    assert_eq!(mapping.parts[0][0].area, areaquad(23.0, 23.5, 113.0, 113.5));
    let order = mapping.parts[1]
        .iter()
        .map(|part| (part.ilat, part.ilon))
        .collect::<Vec<_>>();
    assert_eq!(order, vec![(0, 0), (0, 1), (1, 1)]);
    let total = mapping.parts[1].iter().fold(0.0, |sum, p| sum + p.area);
    assert_eq!(mapping.area[1], total);
    // 常数场映射后仍是那个常数。
    let value = mapping.grid_to_set(1, |_, _| 280.0);
    assert!((value - 280.0).abs() < 1.0e-12);
}


/// `set_missing_value`：缺测格的份面积清零、面积和重算；全落在缺测格上的 set 掩掉。
#[test]
fn missing_cells_drop_out_of_the_mapping() {
    let grid = one_degree_grid();
    let pixel = half_degree_pixels();
    let mut mapping = AreaWeightedMapping::build(
        &grid,
        &pixel,
        &[vec![(1, 1), (3, 1)], vec![(1, 1)]],
        &[1.0, 1.0],
    )
    .unwrap();
    // 格子 (ilon 0, ilat 0) 缺测。
    let mask = mapping.set_missing_value(|ilon, ilat| if (ilon, ilat) == (0, 0) { -9999.0 } else { 1.0 }, -9999.0);
    assert_eq!(mask, vec![true, false]);
    assert_eq!(mapping.area[1], 0.0);
    assert_eq!(mapping.area[0], areaquad(23.0, 23.5, 114.0, 114.5));
    // `round(7·a)/a`：FMA 链下常数场映射回来差 1 ulp 以内。
    assert!((mapping.grid_to_set(0, |_, _| 7.0) - 7.0).abs() < 1.0e-14);
}

/// `build_bilinear`：四份依次是西北、东北、西南、东南，份面积 = set 面积 × 两向权重；
/// 中心正好在四个格心正中时四份平分；区域网格上落在格心带之外时取最近的一列、份不合并。
#[test]
fn bilinear_parts_split_the_set_area_by_great_circle_weights() {
    let pixel = half_degree_pixels();
    let rad = |deg: f64| deg.to_radians();
    let global = LatLonGrid::define_by_res(1.0, 1.0).unwrap();
    let mapping = AreaWeightedMapping::build_bilinear(
        &global,
        &pixel,
        &[vec![(2, 2), (3, 2), (2, 3), (3, 3)]],
        &[1.0],
        &[(rad(114.0), rad(24.0))],
    )
    .unwrap();
    let parts = &mapping.parts[0];
    assert_eq!(parts.len(), 4);
    // 北两份同行、西两份同列；北行的纬度更高。
    assert_eq!((parts[0].ilat, parts[2].ilon), (parts[1].ilat, parts[0].ilon));
    assert!(global.rlat[parts[0].ilat] > global.rlat[parts[2].ilat]);
    for part in parts {
        assert!((part.area / mapping.area[0] - 0.25).abs() < 1.0e-9, "{part:?}");
    }
    // 区域网格西边界外的 set：两侧都取最近的那一列，东份面积为 0。
    let regional = LatLonGrid {
        lat_s: vec![23.0, 24.0],
        lat_n: vec![24.0, 25.0],
        lon_w: vec![113.0, 114.0],
        lon_e: vec![114.0, 115.0],
        yinc: 1,
        rlon: vec![rad(113.5), rad(114.5)],
        rlat: vec![rad(23.5), rad(24.5)],
    };
    let edge = AreaWeightedMapping::build_bilinear(
        &regional,
        &pixel,
        &[vec![(1, 2)]],
        &[1.0],
        &[(rad(113.25), rad(24.0))],
    )
    .unwrap();
    let parts = &edge.parts[0];
    assert_eq!(parts[0].ilon, parts[1].ilon);
    assert_eq!(parts[1].area, 0.0);
    assert!((edge.grid_to_set(0, |_, _| 280.0) - 280.0).abs() < 1.0e-12);
}
