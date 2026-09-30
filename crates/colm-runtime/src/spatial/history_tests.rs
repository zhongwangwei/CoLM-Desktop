use super::*;

#[test]
fn half_degree_grid_runs_north_to_south_from_the_dateline() {
    let grid = LatLonGrid::define_by_res(0.5, 0.5).unwrap();
    assert_eq!(grid.nlat(), 360);
    assert_eq!(grid.nlon(), 720);
    assert_eq!(grid.yinc, -1);
    assert_eq!((grid.lat_n[0], grid.lat_s[0]), (90.0, 89.5));
    assert_eq!((grid.lon_w[0], grid.lon_e[0]), (-180.0, -179.5));
    assert_eq!(grid.lon_e[719], -180.0);
}

#[test]
fn domain_window_covers_exactly_the_region_cells() {
    let grid = LatLonGrid::define_by_res(0.5, 0.5).unwrap();
    let (rows, columns) = grid
        .domain_window(GridBounds {
            south: 23.0,
            north: 25.0,
            west: 113.0,
            east: 115.0,
        })
        .unwrap();
    let lats = rows.iter().map(|&i| grid.lat_n[i]).collect::<Vec<_>>();
    let lons = columns.iter().map(|&i| grid.lon_w[i]).collect::<Vec<_>>();
    assert_eq!(lats, vec![25.0, 24.5, 24.0, 23.5]);
    assert_eq!(lons, vec![113.0, 113.5, 114.0, 114.5]);
}

#[test]
fn global_domain_window_wraps_once() {
    let grid = LatLonGrid::define_by_res(30.0, 30.0).unwrap();
    let (rows, columns) = grid
        .domain_window(GridBounds {
            south: -90.0,
            north: 90.0,
            west: -180.0,
            east: 180.0,
        })
        .unwrap();
    assert_eq!(rows.len(), 6);
    assert_eq!(columns, (0..12).collect::<Vec<_>>());
}
