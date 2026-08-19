//! 守住 `urban_soil.rs` 这张生成表：条目齐、层数对、数在物理范围里。
//!
//! 这些断言防的是**重新生成时静默变形**——换一份 rawdata、改一次抽取顺序，
//! 表还是能编过，但值可能已经不是那个像元的了。

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

#[test]
fn au_preston_is_found_by_its_coordinates() {
    let s = lookup(145.01449584960938, -37.73059844970703).expect("AU-Preston 查不到");
    assert_eq!(s.site, "AU-Preston");
    // 抽取当时的实测值，逐位钉住。改栅格才该改这两个数。
    assert_eq!(s.vf_sand[0], 0.5782577741851876);
    assert_eq!(s.bd_all[0], 1588.0);
}

#[test]
fn lookup_misses_when_no_site_is_near() {
    // 大西洋中间，离任何城市站都远。
    assert!(lookup(-30.0, 0.0).is_none());
}

/// 三个「固体内体积分数」加上两个质量分数都必须落在 0–1。
///
/// 越界说明抽到的是 `_FillValue` 而不是数据 —— 实测 21 个站点在 24 个剖面
/// 栅格上一个缺测都没有，所以这里可以断言得很紧。
#[test]
fn fractions_are_between_zero_and_one() {
    for s in SITES {
        for (name, xs) in [
            ("vf_quartz_mineral", &s.vf_quartz_mineral),
            ("vf_gravels", &s.vf_gravels),
            ("vf_sand", &s.vf_sand),
            ("vf_clay", &s.vf_clay),
            ("vf_om", &s.vf_om),
            ("wf_gravels", &s.wf_gravels),
            ("wf_sand", &s.wf_sand),
            ("wf_clay", &s.wf_clay),
            ("wf_om", &s.wf_om),
            ("theta_s", &s.theta_s),
            ("theta_r", &s.theta_r),
        ] {
            assert_eq!(xs.len(), 8, "{} 的 {name} 不是 8 层", s.site);
            for (i, x) in xs.iter().enumerate() {
                assert!(
                    (0.0..=1.0).contains(x),
                    "{} 的 {name}[{i}] = {x}，不在 0–1 里",
                    s.site
                );
            }
        }
    }
}

/// 正定量：干密度、热学量、导水率都不该是零或负数。
#[test]
fn bulk_and_thermal_properties_are_positive() {
    for s in SITES {
        for (name, xs) in [
            ("bd_all", &s.bd_all),
            ("csol", &s.csol),
            ("tksatu", &s.tksatu),
            ("tksatf", &s.tksatf),
            ("tkdry", &s.tkdry),
            ("k_solids", &s.k_solids),
            ("k_s", &s.k_s),
            ("lambda", &s.lambda),
            ("alpha_vgm", &s.alpha_vgm),
            ("n_vgm", &s.n_vgm),
            ("l_vgm", &s.l_vgm),
            ("om_density", &s.om_density),
        ] {
            for (i, x) in xs.iter().enumerate() {
                assert!(x.is_finite(), "{} 的 {name}[{i}] 不是有限数", s.site);
                assert!(*x >= 0.0, "{} 的 {name}[{i}] = {x} 是负的", s.site);
            }
        }
        // 饱和导水势按 CoLM 的约定是负的（单位 mm）。
        for (i, x) in s.psi_s.iter().enumerate() {
            assert!(*x < 0.0, "{} 的 psi_s[{i}] = {x} 不是负的", s.site);
        }
        // 残余含水量不能超过饱和含水量。
        for i in 0..8 {
            assert!(
                s.theta_r[i] < s.theta_s[i],
                "{} 第 {i} 层 theta_r >= theta_s",
                s.site
            );
        }
    }
}

/// 质地要么是 USDA 的 1–12，要么是 `-1`（栅格在这个像元上没数据）。
///
/// `-1` 是**实测结果不是失败**：21 个站里有 16 个落在质地产品的空洞上，
/// 建成区没有土壤调查。CoLM 把负值夹到 0 再取 `BVIC_USDA(0)`，照抄即可。
#[test]
fn texture_is_a_usda_class_or_the_documented_gap() {
    let gaps = SITES.iter().filter(|s| s.texture == -1).count();
    for s in SITES {
        assert!(
            s.texture == -1 || (1..=12).contains(&s.texture),
            "{} 的 texture = {}",
            s.site,
            s.texture
        );
    }
    assert_eq!(gaps, 16, "质地空洞的站点数变了，栅格或坐标可能换了");
}

/// 生成表里每个剖面字段都要在 `SITE_VARS` 里有对应的 site.nc 变量名。
#[test]
fn site_vars_covers_every_field() {
    assert_eq!(SITE_VARS.len(), 25);
    for (field, site_var) in SITE_VARS {
        assert!(!field.is_empty() && site_var.starts_with("soil_"));
    }
    let mut vars: Vec<&str> = SITE_VARS.iter().map(|(_, v)| *v).collect();
    vars.sort_unstable();
    vars.dedup();
    assert_eq!(vars.len(), 25, "site.nc 变量名有重复");
}

/// 同坐标的两个站点必须抽到同一组值 —— 它们本来就是同一个像元。
#[test]
fn the_two_minneapolis_sites_share_one_pixel() {
    let a = SITES.iter().find(|s| s.site == "US-Minneapolis1").unwrap();
    let b = SITES.iter().find(|s| s.site == "US-Minneapolis2").unwrap();
    assert_eq!(a.lon, b.lon);
    assert_eq!(a.lat, b.lat);
    assert_eq!(a.vf_sand, b.vf_sand);
    assert_eq!(a.texture, b.texture);
}
