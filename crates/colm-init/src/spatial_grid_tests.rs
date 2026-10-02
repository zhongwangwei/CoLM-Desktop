use super::*;

/// 由格心推格边：内部是相邻格心的中点，外缘取 ±90 或区域边界；经度跨日界线时加 360 取中点。
#[test]
fn edges_are_midpoints_of_centres() {
    let grid = LatLonGrid::define_by_center(&[25.0, 24.0, 23.0], &[113.0, 114.0], None).unwrap();
    assert_eq!(grid.yinc, -1);
    assert_eq!(grid.lat_n, vec![90.0, 24.5, 23.5]);
    assert_eq!(grid.lat_s, vec![24.5, 23.5, -90.0]);
    // 两个格心 113、114：113 的西边是 (114 + 113 + 360) / 2 = 293.5 → 规整化成 -66.5。
    assert_eq!(grid.lon_e, vec![113.5, -66.5]);
    assert_eq!(grid.lon_w, vec![-66.5, 113.5]);
    let bounded = LatLonGrid::define_by_center(
        &[23.0, 24.0],
        &[113.0, 114.0],
        Some(GridBounds {
            south: 22.5,
            north: 24.5,
            west: 112.5,
            east: 114.5,
        }),
    )
    .unwrap();
    assert_eq!(bounded.yinc, 1);
    assert_eq!(bounded.lat_s, vec![22.5, 23.5]);
    assert_eq!(bounded.lon_w, vec![112.5, 113.5]);
    assert_eq!(bounded.lon_e, vec![113.5, 114.5]);
    assert_eq!(bounded.rlat[0], 23.0 / 180.0 * std::f64::consts::PI);
}

#[test]
fn longitude_normalization_matches_modulo() {
    assert_eq!(normalize_longitude(190.0).unwrap(), -170.0);
    assert_eq!(normalize_longitude(-190.0).unwrap(), 170.0);
    assert_eq!(normalize_longitude(180.0).unwrap(), -180.0);
    assert_eq!(normalize_longitude(179.5).unwrap(), 179.5);
    assert!(normalize_longitude(-1.0e36).is_err());
}

#[test]
fn nearest_searches_follow_the_grid_direction() {
    let ascending = [0.0, 1.0, 2.0, 3.0];
    assert_eq!(find_nearest_south(1.5, &ascending), 1);
    assert_eq!(find_nearest_north(1.5, &ascending), 2);
    assert_eq!(find_nearest_south(1.0, &ascending), 1);
    assert_eq!(find_nearest_north(1.0, &ascending), 1);
    let descending = [3.0, 2.0, 1.0, 0.0];
    assert_eq!(find_nearest_south(1.5, &descending), 2);
    assert_eq!(find_nearest_north(1.5, &descending), 1);
    let west = [0.0, 1.0, 2.0, 3.0];
    assert_eq!(find_nearest_west(1.5, &west), 1);
    assert_eq!(find_nearest_east(1.5, &[1.0, 2.0, 3.0, 4.0]), 1);
}

#[test]
fn areaquad_is_positive_and_wraps_the_dateline() {
    let plain = areaquad(0.0, 1.0, 10.0, 11.0);
    let wrapped = areaquad(0.0, 1.0, 179.5, -179.5);
    assert!(plain > 0.0);
    assert_eq!(plain, wrapped);
}
