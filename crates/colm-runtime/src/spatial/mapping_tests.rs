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
