//! 守住 `urban_extra.rs` 这张生成表：条目齐、年份对齐、值在 CoLM 认得的
//! 取值域里。
//!
//! 这些断言防的是**重新生成时静默变形**。这张表尤其怕这一类：`LCZ_DOM`
//! 越出 1..=17 会让 `emroof_lcz(utyp)` 越界，而 `LUCY_ID` 越出 1..=231 会让
//! `MOD_Urban_LUCY` 取到别的区 —— 两样都不会在编译期或抽取期被发现。

use super::*;

/// Urban-PLUMBER 的站点数。少一个就是抽取时漏了一个文件。
const N_SITES: usize = 21;

#[test]
fn all_urban_plumber_sites_are_present() {
    assert_eq!(SITES.len(), N_SITES);
    let mut names: Vec<&str> = SITES.iter().map(|s| s.site).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), N_SITES, "站点名有重复");
}

/// 两张表必须对同一批站点、同一批坐标命中。
///
/// 一张命中而另一张没命中是个说不清的状态：`prepare_urban` 会写下半份
/// 站点文件，而 CoLM 只对没写的那半份回落栅格 —— 于是「省了多少」这件事
/// 变得不可预测。
#[test]
fn the_two_tables_cover_the_same_sites() {
    for s in SITES {
        let soil = crate::urban_soil::lookup(s.lon, s.lat)
            .unwrap_or_else(|| panic!("{} 在 urban_soil 表里查不到", s.site));
        assert_eq!(soil.lon, s.lon, "{} 的经度两张表不一致", s.site);
        assert_eq!(soil.lat, s.lat, "{} 的纬度两张表不一致", s.site);
    }
    for s in crate::urban_soil::SITES {
        assert!(
            lookup(s.lon, s.lat).is_some(),
            "{} 在 urban_extra 表里查不到",
            s.site
        );
    }
}

#[test]
fn au_preston_is_found_by_its_coordinates() {
    let s = lookup(145.01449584960938, -37.73059844970703).expect("AU-Preston 查不到");
    assert_eq!(s.site, "AU-Preston");
    // 这五个数是拿真 rawdata 跑一遍城市算例、从 srfdata.nc 里读出来的：
    // URBAN_TYPE = 6、LUCY_id = 12、lakedepth = 0、elvstd = 5.19530534744263、
    // sloperatio = 0.0399660468101501，而四个反照率对应颜色档 16。
    // **换栅格才该改这几个数。**
    assert_eq!(s.lcz_dom, 6);
    assert_eq!(s.ncar_region, 2);
    assert_eq!(s.ncar_density, 3);
    assert_eq!(s.lucy_id, 12.0);
    assert_eq!(s.soil_colour, 16);
    assert_eq!(s.lakedepth, 0.0);
    assert_eq!(s.elvstd, 5.195305347442627);
    assert_eq!(s.sloperatio, 0.039966046810150146);
    // 2000 年 1 月的树 LAI，同样出自那次参照运行的 srfdata.nc。
    assert_eq!(s.tree_lai[0][0], 1.8337343205163141);
}

#[test]
fn lookup_misses_when_no_site_is_near() {
    // 大西洋中间，离任何城市站都远。
    assert!(lookup(-30.0, 0.0).is_none());
}

/// 来源栅格包含完整 LCZ 1..=17；写入前另行限制为 CoLM 支持的城市类 1..=10。
#[test]
fn source_lcz_classes_are_in_product_range() {
    for s in SITES {
        assert!(
            (1..=17).contains(&s.lcz_dom),
            "{} 的 LCZ_DOM = {}",
            s.site,
            s.lcz_dom
        );
    }
    let mut classes: Vec<i32> = SITES.iter().map(|s| s.lcz_dom).collect();
    classes.sort_unstable();
    classes.dedup();
    // 实测七个类别。**这一条正是「不许编默认值」的证据** ——
    // 挑任何一个当默认值，另外六个类别上的站点全会被换掉。
    assert_eq!(classes, vec![1, 2, 3, 5, 6, 8, 12]);
}

#[test]
fn ncar_classes_index_the_bundled_property_table() {
    for s in SITES {
        assert!((1..=33).contains(&s.ncar_region), "{} region", s.site);
        assert!((0..=3).contains(&s.ncar_density), "{} density", s.site);
    }
    let unsupported = SITES
        .iter()
        .filter(|site| site.ncar_density == 0)
        .map(|site| site.site)
        .collect::<Vec<_>>();
    assert_eq!(
        unsupported,
        ["KR-Ochang", "US-Minneapolis1", "US-Minneapolis2"]
    );
}

/// `LUCY_ID` 要落在 `LUCY_rawdata.nc` 的 `region = 231` 里。
///
/// 栅格的 `_FillValue` 是 0，而 0 会让 `lvehicle(:, 0)` 越界。
#[test]
fn lucy_ids_index_a_real_region() {
    for s in SITES {
        assert!(
            s.lucy_id >= 1.0 && s.lucy_id <= 231.0,
            "{} 的 LUCY_ID = {}",
            s.site,
            s.lucy_id
        );
        assert_eq!(s.lucy_id.fract(), 0.0, "{} 的 LUCY_ID 不是整数", s.site);
    }
    let mut ids: Vec<i64> = SITES.iter().map(|s| s.lucy_id as i64).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 13, "LUCY 区号的取值个数变了");
}

/// 土壤颜色档要落在 1..=20，否则 `albedo` 给不出四个反照率。
#[test]
fn soil_colour_classes_have_an_albedo() {
    for s in SITES {
        assert!(
            crate::albedo::albedo(s.soil_colour, crate::albedo::IGBP_URBAN).is_some(),
            "{} 的颜色档 {} 查不到反照率",
            s.site,
            s.soil_colour
        );
    }
}

/// 湖深、地形起伏、坡度都是非负的有限数。
///
/// 湖深实测 21 个站全是 0.0 —— **而模块默认值是 1.0**。这一条钉住的正是
/// 「不许编默认值」：写 1.0 会改掉 `f_t_lake`。
#[test]
fn topography_and_lake_depth_are_non_negative() {
    for s in SITES {
        for (name, x) in [
            ("lakedepth", s.lakedepth),
            ("elvstd", s.elvstd),
            ("sloperatio", s.sloperatio),
        ] {
            assert!(x.is_finite(), "{} 的 {name} 不是有限数", s.site);
            assert!(x >= 0.0, "{} 的 {name} = {x} 是负的", s.site);
        }
    }
    assert!(
        SITES.iter().all(|s| s.lakedepth == 0.0),
        "湖深不再全是 0，模块默认值 1.0 的代价要重新评估"
    );
}

/// 树 LAI/SAI：年份连续、每年 12 个月、值非负且有限。
///
/// **不断言「有季节循环」**：FI-Torni 全年 0.00（市中心塔楼站，周边无树），
/// 而南半球的两个澳洲站抽出来的相位看着像北半球物候。两样都照抄栅格。
#[test]
fn tree_lai_covers_every_year_and_month() {
    assert_eq!(LAI_YEARS.len(), 23);
    assert_eq!(LAI_YEARS[0], 2000);
    assert_eq!(LAI_YEARS[LAI_YEARS.len() - 1], 2022);
    for w in LAI_YEARS.windows(2) {
        assert_eq!(w[1], w[0] + 1, "年份不连续");
    }
    for s in SITES {
        assert_eq!(s.tree_lai.len(), LAI_YEARS.len());
        assert_eq!(s.tree_sai.len(), LAI_YEARS.len());
        for (y, (lai, sai)) in LAI_YEARS.iter().zip(s.tree_lai.iter().zip(&s.tree_sai)) {
            for m in 0..12 {
                for (name, x) in [("TREE_LAI", lai[m]), ("TREE_SAI", sai[m])] {
                    assert!(
                        x.is_finite(),
                        "{} 的 {name} {y}-{} 不是有限数",
                        s.site,
                        m + 1
                    );
                    // 瓦片的 `_FillValue` 是 -999，抽取时会报错而不是写进来；
                    // 这一条守的是「万一它绕过去了」。
                    assert!(x >= 0.0, "{} 的 {name} {y}-{} = {x}", s.site, m + 1);
                    assert!(
                        x < 20.0,
                        "{} 的 {name} {y}-{} = {x} 大得不像话",
                        s.site,
                        m + 1
                    );
                }
            }
        }
    }
}

/// CoLM 默认的 LAI 年份区间必须被这张表盖住。
///
/// 运行时的年份是 `min(DEF_LAI_END_YEAR, max(DEF_LAI_START_YEAR, year))`
/// （`MOD_Urban_LAIReadin.F90:58`），落到表外就是 `findloc_ud` 返回 0，
/// 再拿 0 去索引 —— 越界。
#[test]
fn the_table_covers_colms_default_lai_window() {
    for y in 2000..=2020 {
        assert!(LAI_YEARS.contains(&y), "{y} 不在表里");
    }
}

/// 同坐标的两个站点必须抽到同一组值 —— 它们本来就是同一个像元。
#[test]
fn the_two_minneapolis_sites_share_one_pixel() {
    let a = SITES.iter().find(|s| s.site == "US-Minneapolis1").unwrap();
    let b = SITES.iter().find(|s| s.site == "US-Minneapolis2").unwrap();
    assert_eq!(a.lon, b.lon);
    assert_eq!(a.lat, b.lat);
    assert_eq!(a.lcz_dom, b.lcz_dom);
    assert_eq!(a.lucy_id, b.lucy_id);
    assert_eq!(a.soil_colour, b.soil_colour);
    assert_eq!(a.elvstd, b.elvstd);
    assert_eq!(a.tree_lai, b.tree_lai);
    // **栅格给 12，而这两个站的站点文件自己写着 6。**
    // `prepare_urban` 只补站点文件没有的变量，所以这两个站用的是 6 ——
    // 表里的 12 写下来是为了让「站点自己说的话优先」这条规则看得见。
    assert_eq!(a.lcz_dom, 12);
}
