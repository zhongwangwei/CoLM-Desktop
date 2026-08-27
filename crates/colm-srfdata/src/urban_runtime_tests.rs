//! 守住随仓库发的两张城市参数表：内容有效、维度正确、能铺到 CoLM 的固定路径。

use super::*;

/// 嵌进来的字节确实是一个 netCDF 文件，而且没被 LFS 之类的东西换成指针。
#[test]
fn the_embedded_table_is_a_netcdf_file() {
    // 实测 37 KB。数量级掉了就说明入库的不是那张表。
    assert_eq!(LUCY_RAWDATA.len(), 38197);
    // netCDF-4 是 HDF5 容器，魔数 \x89HDF；netCDF-3 是 "CDF"。两种都认。
    let head = &LUCY_RAWDATA[..4];
    assert!(
        head == b"\x89HDF" || &head[..3] == b"CDF",
        "开头是 {head:?}，不像 netCDF"
    );
}

/// 铺出来之后路径与内容都对，且重铺一次是幂等的。
#[test]
fn staging_puts_it_where_colm_looks_for_it() {
    let dir = std::env::temp_dir().join(format!("colm-lucy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = stage(&dir).expect("铺不出来");
    assert_eq!(file, dir.join("urban/LUCY_rawdata.nc"));
    assert_eq!(std::fs::read(&file).unwrap(), LUCY_RAWDATA);

    // 第二次铺不该改动它 —— 内容一样就不重写。
    let before = std::fs::metadata(&file).unwrap().modified().unwrap();
    let again = stage(&dir).expect("重铺失败");
    assert_eq!(again, file);
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        before
    );

    // 内容不同就覆盖：表换了要跟着换。
    std::fs::write(&file, b"stale").unwrap();
    stage(&dir).expect("覆盖失败");
    assert_eq!(std::fs::read(&file).unwrap(), LUCY_RAWDATA);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn staging_ncar_puts_the_three_class_table_in_rawdata() {
    let dir = std::env::temp_dir().join(format!("colm-ncar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = stage_ncar(&dir).expect("铺不出来");
    assert_eq!(file, dir.join(NCAR_RELATIVE));
    let f = netcdf::open(&file).expect("打不开");
    assert_eq!(f.dimension("region").expect("region").len(), 33);
    assert_eq!(
        f.dimension("density_class").expect("density_class").len(),
        3
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 这张表是**按区号**索引的全局表，与站点无关 —— 所以随仓库发是对的。
///
/// 判据不是「看着像」：`region` 维长 231，而 21 个城市站点的 `LUCY_ID`
/// 取到 13 个不同的区号，全落在 1..=231 里。一张只覆盖某几个区的表
/// 不会有这个维度。
#[test]
fn it_is_a_global_region_table_not_a_per_site_one() {
    let dir = std::env::temp_dir().join(format!("colm-lucy-dims-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = stage(&dir).expect("铺不出来");
    let f = netcdf::open(&file).expect("打不开");
    let region = f.dimension("region").expect("没有 region 维");
    assert_eq!(region.len(), 231);
    for name in [
        "NUMS_VEHC",
        "WEEKEND_DAY",
        "TraffProf_24hr_holiday",
        "TraffProf_24hr_work",
        "HumMetabolic_24hr",
        "FIXED_HOLIDAY",
    ] {
        let v = f
            .variable(name)
            .unwrap_or_else(|| panic!("{name} 不在表里"));
        // MOD_UrbanReadin.F90 把六个变量整个读进来，所以每一个都必须
        // 带着 region 维 —— 少一个维度就意味着这不是那张表。
        assert!(
            v.dimensions().iter().any(|d| d.name() == "region"),
            "{name} 没有 region 维"
        );
    }
    // 21 个站点的区号必须都能在这张表里索引到。
    for s in crate::urban_extra::SITES {
        let id = s.lucy_id as usize;
        assert!(
            (1..=region.len()).contains(&id),
            "{} 的 LUCY_ID = {} 落在 1..={} 之外",
            s.site,
            s.lucy_id,
            region.len()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
