use super::*;

/// CoLM `find_nearest_south` 的忠实移植，只用来验证闭式解。
///
/// 它慢且笨，但它是**独立的第二实现**：闭式解与二分查找同时错成一样的
/// 概率远低于各自出错。里程碑 2 的教训是「一条只会说相同的测试比没有更糟」。
fn binary_search_south(y: f64) -> usize {
    let n = COLM_500M.nlat;
    let lat = |j: usize| 90.0 - COLM_500M.dlat() * (j as f64);
    if y >= lat(1) {
        return 1;
    }
    if y <= lat(n) {
        return n;
    }
    let (mut l, mut r) = (1usize, n);
    while r - l > 1 {
        let i = (r + l) / 2;
        if y >= lat(i) {
            r = i;
        } else {
            l = i;
        }
    }
    r
}

#[test]
fn the_grid_is_the_one_colm_defines() {
    // share/MOD_Grid.F90 的 grid_define_by_ndims(86400, 43200)
    assert_eq!(COLM_500M.nlon, 86400);
    assert_eq!(COLM_500M.nlat, 43200);
    assert!((COLM_500M.dlon() - 1.0 / 240.0).abs() < 1e-15);
    assert!((COLM_500M.dlat() - 1.0 / 240.0).abs() < 1e-15);
}

#[test]
fn cn_cng_lands_on_the_pixel_the_extraction_used() {
    // 实测：该像元的 elevation=144.1444549560547、soil_brightness=10
    let (ilon, ilat) = COLM_500M.index_of(123.509_201_049_804_69, 44.593_299_865_722_656);
    assert_eq!((ilon, ilat), (72843, 10898));
}

#[test]
fn a_latitude_exactly_on_a_cell_edge_matches_colm_not_the_naive_formula() {
    // 赤道正好落在格边界上。floor(...)+1 给 21601，CoLM 给 21600。
    // 90 个 PLUMBER2 站点都没踩到这个，但用户自己的站点会。
    assert_eq!(COLM_500M.index_of(0.0, 0.0).1, 21600);
    assert_eq!(binary_search_south(0.0), 21600);
}

#[test]
fn the_poles_clamp_instead_of_running_off_the_end() {
    assert_eq!(COLM_500M.index_of(0.0, 90.0).1, 1);
    assert_eq!(COLM_500M.index_of(0.0, -90.0).1, 43200);
}

#[test]
fn every_single_cell_edge_agrees_with_the_binary_search() {
    // 全部 43200 个精确格边界逐个比对。这里不抽样：格边界正是浮点抵消
    // 会让两者分道扬镳的地方，抽样只会碰巧躲开它。跑完约 30 ms。
    let mut bad = Vec::new();
    for j in 1..=COLM_500M.nlat {
        let y = 90.0 - COLM_500M.dlat() * (j as f64);
        let (got, want) = (COLM_500M.index_of(0.0, y).1, binary_search_south(y));
        if got != want {
            bad.push(format!(
                "edge {j}: lat {y} -> {got}, binary search says {want}"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} exact edges disagree; first few: {:?}",
        bad.len(),
        COLM_500M.nlat,
        &bad[..bad.len().min(5)]
    );
}

#[test]
fn a_dense_sweep_of_ordinary_latitudes_also_agrees() {
    // 格边界之外的普通纬度。两条一起才说明「既没漏边界，也没在中间跑偏」。
    let n = 50_000;
    for k in 0..=n {
        let y = -90.0 + 180.0 * (k as f64) / (n as f64);
        assert_eq!(
            COLM_500M.index_of(0.0, y).1,
            binary_search_south(y),
            "lat {y}"
        );
    }
}

#[test]
fn longitude_picks_the_cell_whose_west_edge_is_at_or_west_of_the_point() {
    for k in 0..1000 {
        let x = -180.0 + 360.0 * (k as f64) / 999.0;
        let (ilon, _) = COLM_500M.index_of(x, 0.0);
        let west = -180.0 + COLM_500M.dlon() * ((ilon - 1) as f64);
        assert!(west <= x + 1e-9, "lon {x}: west edge {west} is east of it");
        if ilon < COLM_500M.nlon {
            let next = -180.0 + COLM_500M.dlon() * (ilon as f64);
            assert!(x < next + 1e-9, "lon {x}: cell {ilon} does not contain it");
        }
    }
}

#[test]
fn indices_are_one_based_like_fortran() {
    // 与 Fortran 一致是有意的：抽取代码要和 MOD_NetCDFPoint 的
    // nf90_get_var(..., (/ilon,ilat/), ...) 对得上，换成 0-based
    // 只会在两套约定的交界处埋一个 off-by-one。
    let (ilon, ilat) = COLM_500M.index_of(-180.0, 90.0);
    assert_eq!((ilon, ilat), (1, 1));
}

/// 5x5 瓦片的文件名词干，钉住 `get_5x5_filename` 的四个数字与它们的次序。
///
/// **次序是北、西、南、东。** 照几何直觉写成「南在前」会得到一个
/// **存在的**、但差 5 度的文件名 —— 那种错误既不报错也不越界，只会安静地
/// 取到另一块地方的数据。所以这里挑的七个站横跨南北半球与东西半球：
/// 期望值出自另一条独立算出来的路径（`north=ceil(lat/5)*5, west=floor(lon/5)*5`）。
// 一行一个站点：三列对齐着看，才能一眼核出哪个站落在哪块上。
// 拆成每项六行就核不动了 —— 而这张表的全部意义就是被人核对。
#[rustfmt::skip]
#[test]
fn five_by_five_tile_names_match_colms_naming() {
    for (site, lon, lat, want) in [
        ("AU-Preston", 145.014_495_849_609_38, -37.730_598_449_707_03, "RG_-35_145_-40_150"),
        ("FI-Kumpula", 24.961_099_624_633_79, 60.202_800_750_732_42, "RG_65_20_60_25"),
        ("CA-Sunset", -123.078_399_658_203_13, 49.226_100_921_630_86, "RG_50_-125_45_-120"),
        ("SG-TelokKurau06", 103.911_201_477_050_78, 1.314_299_941_062_927_2, "RG_5_100_0_105"),
        ("MX-Escandon", -99.176_101_684_570_31, 19.404_199_600_219_727, "RG_20_-100_15_-95"),
        ("US-Minneapolis1", -93.188_362_121_582_03, 44.998_401_641_845_7, "RG_45_-95_40_-90"),
        ("UK-KingsCollege", -0.116_700_001_060_962_68, 51.511_798_858_642_58, "RG_55_-5_50_0"),
    ] {
        let t = COLM_500M.tile_5x5(lon, lat);
        assert_eq!(t.stem, want, "{site} 落在了 {} 上", t.stem);
        // 瓦片是 1200x1200，下标 1-based。
        assert!((1..=1200).contains(&t.ilon), "{site}: ilon {}", t.ilon);
        assert!((1..=1200).contains(&t.ilat), "{site}: ilat {}", t.ilat);
    }
}

/// 瓦片内下标与全球下标必须自洽：`(ibox-1)*1200 + ilon` 要还原成全球下标。
#[test]
fn the_tile_offset_reconstructs_the_global_index() {
    for (lon, lat) in [(145.0145, -37.7306), (-0.1167, 51.5118), (0.0, 0.0)] {
        let (ilon, ilat) = COLM_500M.index_of(lon, lat);
        let t = COLM_500M.tile_5x5(lon, lat);
        assert_eq!((ilon - 1) % 1200 + 1, t.ilon);
        assert_eq!((ilat - 1) % 1200 + 1, t.ilat);
    }
}
