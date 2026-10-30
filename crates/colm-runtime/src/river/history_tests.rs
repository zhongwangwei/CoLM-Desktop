use super::{domain_window, flooded_fraction, mean_or_missing, SPVAL};

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
    assert_eq!(
        domain_window(&lon, &lat, (170.0, -170.0, 0.0, 10.0)),
        (0, 0, 1440, 720)
    );
    assert_eq!(
        domain_window(&lon, &lat, (-180.0, 180.0, -90.0, 90.0)),
        (0, 0, 1440, 720)
    );
}

#[test]
fn zero_accumulation_is_spval_not_the_raw_accumulator() {
    // 上游 `MOD_Grid_RiverLakeHist.F90:460-475`：`WHERE (acctime_ucat > 0) a/acctime
    // ELSEWHERE spval`。表驱动三种窗口：只累加了一部分、完全没累加、以及累加结果本身是
    // 0（0 是物理值，不是缺测）。
    let acctime = [0.0, 2.0, 86400.0];
    for (values, expected) in [
        // 没累加的元素是 spval；累加过但累加值恰为 0 的元素保留 0。
        (vec![0.0, 0.0, 0.0], vec![SPVAL, 0.0, 0.0]),
        // 部分累加：只有累加过的除时长。
        (vec![1.0, 10.0, 86400.0], vec![SPVAL, 5.0, 1.0]),
        (vec![SPVAL, 0.0, 0.0], vec![SPVAL, 0.0, 0.0]),
    ] {
        let result = mean_or_missing(&values, &acctime);
        for (k, (&got, &want)) in result.iter().zip(&expected).enumerate() {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "element {k}: {got} != {want}"
            );
        }
    }
    // 整个窗口一次都没累加（例如第一步就写 history）：全部 spval，绝不写 0。
    assert_eq!(
        mean_or_missing(&[7.0, 8.0], &[0.0, 0.0]),
        vec![SPVAL, SPVAL]
    );
}

#[test]
fn flood_fraction_is_spval_when_its_flood_area_is_missing() {
    // 上游 `:423-430`：`WHERE (topo_area > 0 .and. a_floodarea /= spval)`。关键是先判缺测
    // 再除 —— 直接除会得到 -1e36/area 这种有限巨大负值，history 里认不出是缺测。
    let floodarea = [SPVAL, 100.0, 100.0, 50.0];
    let area = [1.0e6, 1.0e6, 0.0, SPVAL];
    let fraction = flooded_fraction(&floodarea, &area);
    assert_eq!(fraction[0], SPVAL);
    assert_eq!(fraction[1], 1.0e-4);
    assert_eq!(fraction[2], SPVAL);
    assert_eq!(fraction[3], SPVAL);
    assert!(
        fraction.iter().all(|&value| value >= 0.0 || value == SPVAL),
        "a missing flood area was divided instead of masked: {fraction:?}"
    );
}

#[test]
fn the_river_history_writer_creates_a_missing_restart_date_directory() {
    let root = std::env::temp_dir().join(format!("colm-river-hist-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let path = root
        .join("2026-10-31-00000")
        .join("case_restart_hist_2026-10-31-00000.nc.river");
    let history = crate::river::RiverHistory::zeros_for(3, false, None, None);
    crate::river::restart::write_river_history(&path, &history).unwrap();
    assert!(path.is_file());
    let file = netcdf::open(&path).unwrap();
    assert_eq!(file.dimension("ucatch").unwrap().len(), 3);
    std::fs::remove_dir_all(&root).unwrap();
}
