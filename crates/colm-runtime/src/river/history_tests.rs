use super::domain_window;

fn axes() -> (Vec<f64>, Vec<f64>) {
    // 全球 15′ 格心：经度自西向东，纬度自北向南。
    let lon = (0..1440).map(|i| -179.875 + 0.25 * i as f64).collect();
    let lat = (0..720).map(|j| 89.875 - 0.25 * j as f64).collect();
    (lon, lat)
}

#[test]
fn the_unitcat_window_is_the_model_domain() {
    let (lon, lat) = axes();
    let (x0, y0, nx, ny) = domain_window(&lon, &lat, (96.25, 104.25, 37.75, 43.25));
    assert_eq!((nx, ny), (32, 22));
    assert_eq!((lon[x0], lon[x0 + nx - 1]), (96.375, 104.125));
    assert_eq!((lat[y0], lat[y0 + ny - 1]), (43.125, 37.875));
}

#[test]
fn a_dateline_or_global_domain_keeps_the_whole_grid() {
    let (lon, lat) = axes();
    assert_eq!(domain_window(&lon, &lat, (170.0, -170.0, 0.0, 10.0)), (0, 0, 1440, 720));
    assert_eq!(domain_window(&lon, &lat, (-180.0, 180.0, -90.0, 90.0)), (0, 0, 1440, 720));
}
