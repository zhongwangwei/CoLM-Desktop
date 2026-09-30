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
    };
    let new = SatSide {
        time: &file,
        patch_class: &new_class,
        element: &new_element,
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
    };
    let new = SatSide {
        time: &new_file,
        patch_class: &[1, 2, 17],
        element: &[3, 3, 3],
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
