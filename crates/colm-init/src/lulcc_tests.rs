//! SAT 的配对与赋值：自建最小时间重启（两个单元），不依赖写出器夹具。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-lulcc-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// 一份时间重启：每个变量的第 p 个 patch 取 `base + p`；`t_soisno` 两层，`ssno_lyr` 两层雪。
fn restart(path: &Path, patches: usize, base: f64, sigf: &[f64], lai: &[f64]) -> RestartFile {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    file.add_dimension("soilsnow", 2).unwrap();
    file.add_dimension("snowp1", 2).unwrap();
    file.add_dimension("rtyp", 2).unwrap();
    file.add_dimension("band", 2).unwrap();
    let scalars = ALWAYS
        .iter()
        .chain(&PLANT_HYDRAULICS)
        .chain(&SNOW_AEROSOL)
        .chain(&SURFACE_DIAGNOSTICS)
        .filter(|name| **name != "t_soisno");
    for name in scalars {
        let values = (0..patches).map(|p| base + p as f64).collect::<Vec<_>>();
        file.add_variable::<f64>(name, &["patch"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    let layered = (0..2 * patches)
        .map(|i| base + (i / 2) as f64 + 0.1 * (i % 2) as f64)
        .collect::<Vec<_>>();
    file.add_variable::<f64>("t_soisno", &["patch", "soilsnow"])
        .unwrap()
        .put_values(&layered, ..)
        .unwrap();
    let ssno = (0..8 * patches)
        .map(|i| base + (i / 8) as f64 + 0.01 * (i % 8) as f64)
        .collect::<Vec<_>>();
    file.add_variable::<f64>("ssno_lyr", &["patch", "snowp1", "rtyp", "band"])
        .unwrap()
        .put_values(&ssno, ..)
        .unwrap();
    for (name, values) in [
        ("sigf", sigf),
        ("lai", lai),
        ("sai", &vec![0.0; patches][..]),
    ] {
        file.add_variable::<f64>(name, &["patch"])
            .unwrap()
            .put_values(values, ..)
            .unwrap();
    }
    file.close().unwrap();
    RestartFile::open(path).unwrap()
}

fn options() -> SatOptions {
    SatOptions {
        plant_hydraulics: true,
        ozone_stress: false,
        irrigation: false,
    }
}

#[test]
fn pairs_walk_each_element_by_patch_class() {
    // 单元 7：旧 [1, 5, 17]，新 [1, 2, 17] —— 5 消失、2 新增；单元 9 只在新的一侧。
    let old_class = [1, 5, 17];
    let old_element = [7, 7, 7];
    let new_class = [1, 2, 17, 4];
    let new_element = [7, 7, 7, 9];
    let dir = temp_dir("pairs");
    let file = restart(&dir.join("r.nc"), 1, 0.0, &[0.0], &[0.0]);
    let old = SatSide {
        time: &file,
        patch_class: &old_class,
        element: &old_element,
        urban_class: None,
    };
    let new = SatSide {
        time: &file,
        patch_class: &new_class,
        element: &new_element,
        urban_class: None,
    };
    assert_eq!(match_patches(&new, &old).unwrap(), vec![(0, 0), (2, 2)]);
}

#[test]
fn matched_patches_take_the_old_state_and_new_ones_stay_cold() {
    let dir = temp_dir("assign");
    // 旧：2 个 patch（类型 1、17）；新：3 个（1、2、17）。sigf 旧值 0，新 patch 有叶。
    let old_file = restart(&dir.join("old.nc"), 2, 100.0, &[0.0, 0.5], &[0.0, 0.0]);
    let new_file = restart(
        &dir.join("new.nc"),
        3,
        200.0,
        &[0.3, 0.3, 0.3],
        &[2.0, 1.0, 0.0],
    );
    let old = SatSide {
        time: &old_file,
        patch_class: &[1, 17],
        element: &[3, 3],
        urban_class: None,
    };
    let new = SatSide {
        time: &new_file,
        patch_class: &[1, 2, 17],
        element: &[3, 3, 3],
        urban_class: None,
    };
    let overrides = same_type_assignment(&new, &old, options()).unwrap();
    let get = |name: &str| {
        overrides
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} not overridden"))
            .values
            .clone()
    };
    assert_eq!(get("t_grnd"), vec![100.0, 201.0, 101.0]);
    assert_eq!(get("vegwp"), vec![100.0, 201.0, 101.0]);
    assert_eq!(
        get("t_soisno"),
        vec![100.0, 100.1, 201.0, 201.1, 101.0, 101.1]
    );
    // 旧 sigf 0 但新 patch 的 lai + sai > 0：置 1；第二对照抄 0.5；新增的保持冷启动值。
    assert_eq!(get("sigf"), vec![1.0, 0.3, 0.5]);
    // ssno_lyr 每个 patch 8 个值 (snowp1, rtyp, band)：只有 (s, 1, 1) 即偏移 3、7 被抄。
    let ssno = get("ssno_lyr");
    let cold = new_file.floats("ssno_lyr").unwrap();
    for (i, value) in ssno.iter().enumerate() {
        let (patch, offset) = (i / 8, i % 8);
        let expected = match (patch, offset) {
            (0, 3 | 7) => 100.0 + 0.01 * offset as f64,
            (2, 3 | 7) => 101.0 + 0.01 * offset as f64,
            _ => cold[i],
        };
        assert_eq!(*value, expected, "ssno_lyr[{i}]");
    }
    // 没开臭氧：不碰 `lai_old`。
    assert!(overrides.iter().all(|o| o.name != "lai_old"));
}

#[test]
fn a_missing_variable_is_named() {
    let dir = temp_dir("missing");
    let old_file = restart(&dir.join("old.nc"), 1, 0.0, &[0.0], &[0.0]);
    let new_file = restart(&dir.join("new.nc"), 1, 0.0, &[0.0], &[0.0]);
    let side = |file| SatSide {
        time: file,
        patch_class: &[1],
        element: &[1],
        urban_class: None,
    };
    let error = same_type_assignment(
        &side(&new_file),
        &side(&old_file),
        SatOptions {
            ozone_stress: true,
            ..options()
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("lai_old"), "{error:#}");
}

/// PFT 时间重启：`pfts` 个 PFT，每个变量第 ip 个取 `base + ip`。
fn pft_restart(path: &Path, pfts: usize, base: f64) -> RestartFile {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", pfts).unwrap();
    for name in PFT_ALWAYS {
        let values = (0..pfts).map(|ip| base + ip as f64).collect::<Vec<_>>();
        file.add_variable::<f64>(name, &["pft"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.close().unwrap();
    RestartFile::open(path).unwrap()
}

/// 逐 PFT 配对：同一土壤 patch 里按 `pftclass` 对齐，消失的旧 PFT 被跳过、新增的保持冷启动值；
/// `ldew` 按新的 `pftfrac` 从 0 起顺序 FMA 重算。非土壤配对不碰 PFT。
#[test]
fn pfts_pair_by_class_and_ldew_is_reweighted() {
    let dir = temp_dir("pft");
    let new_time = restart(&dir.join("new.nc"), 2, 100.0, &[1.0, 1.0], &[1.0, 1.0]);
    let old_time = restart(&dir.join("old.nc"), 2, 0.0, &[1.0, 1.0], &[1.0, 1.0]);
    // 一个单元两个 patch：土壤（类 1）与湿地（类 11）。
    let (class, element) = ([1i64, 11], [7i64, 7]);
    let new_side = SatSide {
        time: &new_time,
        patch_class: &class,
        element: &element,
        urban_class: None,
    };
    let old_side = SatSide {
        time: &old_time,
        patch_class: &class,
        element: &element,
        urban_class: None,
    };
    // 新：PFT 类 1、3、4；旧：类 1、2、4（类 2 消失、类 3 新增）。
    let new_pft = pft_restart(&dir.join("new_pft.nc"), 3, 100.0);
    let old_pft = pft_restart(&dir.join("old_pft.nc"), 3, 0.0);
    let ranges = [0..3, 3..3];
    let patch_type = [0i64, 2];
    let (overrides, ldew) = pft_same_type_assignment(
        &new_side,
        &old_side,
        &PftSatSide {
            time: &new_pft,
            pft_class: &[1, 3, 4],
            ranges: &ranges,
            patch_type: &patch_type,
        },
        &PftSatSide {
            time: &old_pft,
            pft_class: &[1, 2, 4],
            ranges: &ranges,
            patch_type: &patch_type,
        },
        &[0.5, 0.3, 0.2],
        SatOptions {
            plant_hydraulics: false,
            ..options()
        },
    )
    .unwrap();
    let tleaf = &overrides
        .iter()
        .find(|o| o.name == "tleaf_p")
        .unwrap()
        .values;
    // 类 1 ← 旧第 0 个（0），类 3 冷启动（101），类 4 ← 旧第 2 个（2）。
    assert_eq!(tleaf, &vec![0.0, 101.0, 2.0]);
    let expected = 2.0f64.mul_add(0.2, 101.0f64.mul_add(0.3, 0.0f64.mul_add(0.5, 0.0)));
    assert_eq!(ldew, vec![(0, expected)]);
}

/// 城市 patch 还要按城市类型对齐：旧 [城市 2, 城市 3]，新 [城市 1, 城市 3] —— 2 消失、1 新增。
#[test]
fn urban_patches_pair_by_urban_class() {
    let dir = temp_dir("urban-pairs");
    let time = restart(&dir.join("t.nc"), 3, 0.0, &[0.0; 3], &[0.0; 3]);
    let class = [1, URBAN, URBAN];
    let element = [7, 7, 7];
    let side = |urban: &'static [i64]| SatSide {
        time: &time,
        patch_class: &class,
        element: &element,
        urban_class: Some(urban),
    };
    let pairs = match_patches(&side(&[0, 1, 3]), &side(&[0, 2, 3])).unwrap();
    assert_eq!(pairs, vec![(0, 0), (2, 2)]);
    // 不带城市类型时按 patchclass 一一配对。
    let plain = |time| SatSide {
        time,
        patch_class: &class,
        element: &element,
        urban_class: None,
    };
    assert_eq!(
        match_patches(&plain(&time), &plain(&time)).unwrap(),
        vec![(0, 0), (1, 1), (2, 2)]
    );
}

/// 城市 patch 的水量重组：屋顶只占前 6 层，透水地面整列，不透水地面前 6 层；`scv` 三项加权。
#[test]
fn urban_patch_water_is_recomposed_from_the_urban_columns() {
    let dir = temp_dir("urban-water");
    let mut file = netcdf::create(dir.join("u.nc")).unwrap();
    file.add_dimension("urban", 1).unwrap();
    file.add_dimension("roofsnow", 8).unwrap();
    file.add_dimension("soilsnow", 8).unwrap();
    for name in ["wliq_roofsno", "wice_roofsno"] {
        file.add_variable::<f64>(name, &["urban", "roofsnow"])
            .unwrap()
            .put_values(&[1.0; 8], ..)
            .unwrap();
    }
    for name in [
        "wliq_gpersno",
        "wice_gpersno",
        "wliq_gimpsno",
        "wice_gimpsno",
    ] {
        let value = if name.contains("gper") { 2.0 } else { 4.0 };
        file.add_variable::<f64>(name, &["urban", "soilsnow"])
            .unwrap()
            .put_values(&[value; 8], ..)
            .unwrap();
    }
    for (name, value) in [("scv_roof", 1.0), ("scv_gper", 2.0), ("scv_gimp", 4.0)] {
        file.add_variable::<f64>(name, &["urban"])
            .unwrap()
            .put_values(&[value], ..)
            .unwrap();
    }
    file.close().unwrap();
    let urban_time = RestartFile::open(dir.join("u.nc")).unwrap();
    let urban: Vec<RestartOverride> = [
        "wliq_roofsno",
        "wice_roofsno",
        "wliq_gpersno",
        "wice_gpersno",
        "wliq_gimpsno",
        "wice_gimpsno",
        "scv_roof",
        "scv_gper",
        "scv_gimp",
    ]
    .iter()
    .map(|&name| RestartOverride::new(name, values(&urban_time, name).unwrap()))
    .collect();
    let mut patch = vec![
        RestartOverride::new("wliq_soisno", vec![9.0; 16]),
        RestartOverride::new("wice_soisno", vec![9.0; 16]),
        RestartOverride::new("scv", vec![9.0, 9.0]),
    ];
    let (froof, fgper) = (0.25, 0.5);
    recompose_urban_patch_water(
        &[(1, 0)],
        &urban,
        &urban_time,
        &[froof],
        &[fgper],
        &mut patch,
        8,
    )
    .unwrap();
    let open = 1.0 - froof;
    let top = (4.0 * open).mul_add(1.0 - fgper, (2.0 * open).mul_add(fgper, 1.0 * froof));
    let deep = (2.0 * open).mul_add(fgper, 0.0);
    let column = &patch[0].values[8..];
    assert_eq!(column[..6], [top; 6]);
    assert_eq!(column[6..], [deep; 2]);
    // 第 0 个 patch 不在目标里，原样保留。
    assert_eq!(patch[0].values[..8], [9.0; 8]);
    let scv = (4.0 * open).mul_add(1.0 - fgper, 1.0f64.mul_add(froof, (2.0 * open) * fgper));
    assert_eq!(patch[2].values, vec![9.0, scv]);
}
