//! MEC：一个单元、两个旧 patch（类型 1、5）并成一个新 patch（类型 1），无雪；
//! 份额不变的 patch 保留 SAT 结果。自建最小重启，不依赖写出器夹具。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-lulcc-mec-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

const SOIL: usize = 10;

/// 时间重启：`values(name, patch)` 给每个 patch 的标量；分层量整列同值。
fn time_restart(path: &Path, patches: usize, value: impl Fn(&str, usize) -> f64) -> RestartFile {
    let mut file = netcdf::create(path).unwrap();
    for (name, len) in [
        ("patch", patches),
        ("soilsnow", SNOW_LAYERS + SOIL),
        ("snow", SNOW_LAYERS),
        ("soil", SOIL),
        ("snowp1", SNOW_LAYERS + 1),
        ("rtyp", 2),
        ("band", 2),
        ("vegnodes", 4),
    ] {
        file.add_dimension(name, len).unwrap();
    }
    let layered = |name: &str, per: usize, snow_zero: bool| {
        (0..patches)
            .flat_map(|p| (0..per).map(move |i| (p, i)))
            .map(|(p, i)| {
                if snow_zero && i < SNOW_LAYERS {
                    0.0
                } else {
                    value(name, p)
                }
            })
            .collect::<Vec<_>>()
    };
    let mut put = |name: &str, dims: &[&str], values: Vec<f64>| {
        file.add_variable::<f64>(name, dims)
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    };
    for name in ["wliq_soisno", "wice_soisno", "t_soisno"] {
        put(
            name,
            &["patch", "soilsnow"],
            layered(name, SNOW_LAYERS + SOIL, true),
        );
    }
    for name in [
        "z_sno",
        "dz_sno",
        "snw_rds",
        "mss_bcpho",
        "mss_bcphi",
        "mss_ocpho",
        "mss_ocphi",
        "mss_dst1",
        "mss_dst2",
        "mss_dst3",
        "mss_dst4",
    ] {
        put(name, &["patch", "snow"], vec![0.0; patches * SNOW_LAYERS]);
    }
    for name in ["smp", "hk"] {
        put(name, &["patch", "soil"], layered(name, SOIL, false));
    }
    put("vegwp", &["patch", "vegnodes"], layered("vegwp", 4, false));
    put(
        "ssno_lyr",
        &["patch", "snowp1", "rtyp", "band"],
        vec![0.0; patches * (SNOW_LAYERS + 1) * 4],
    );
    for name in [
        "tleaf",
        "ldew",
        "ldew_rain",
        "ldew_snow",
        "sag",
        "wa",
        "wdsrf",
        "trad",
        "tref",
        "qref",
        "rst",
        "emis",
        "z0m",
        "zol",
        "rib",
        "ustar",
        "qstar",
        "tstar",
        "fm",
        "fh",
        "fq",
        "t_grnd",
        "scv",
        "snowdp",
        "fsno",
        "sigf",
        "zwt",
        "lai",
        "sai",
        "tlai",
        "tsai",
        "gs0sun",
        "gs0sha",
    ] {
        put(
            name,
            &["patch"],
            (0..patches).map(|p| value(name, p)).collect(),
        );
    }
    file.close().unwrap();
    RestartFile::open(path).unwrap()
}

fn const_restart(path: &Path, class: &[i32], csol: f64, porsl: f64) -> RestartFile {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", class.len()).unwrap();
    file.add_dimension("soil", SOIL).unwrap();
    file.add_variable::<i32>("patchclass", &["patch"])
        .unwrap()
        .put_values(class, ..)
        .unwrap();
    file.add_variable::<i32>("patchtype", &["patch"])
        .unwrap()
        .put_values(&vec![0; class.len()], ..)
        .unwrap();
    for (name, value) in [("csol", csol), ("porsl", porsl)] {
        file.add_variable::<f64>(name, &["patch", "soil"])
            .unwrap()
            .put_values(&vec![value; class.len() * SOIL], ..)
            .unwrap();
    }
    file.close().unwrap();
    RestartFile::open(path).unwrap()
}

fn options() -> MecOptions {
    MecOptions {
        plant_hydraulics: true,
        ozone_stress: false,
        variably_saturated_flow: true,
        vegetation_snow: true,
        snow_cover_exponent: 1.0,
        campbell_soil: true,
    }
}

/// 两个旧 patch：类型 1 与 5；新单元只有类型 1 与类型 17。
fn case(dir: &Path) -> (RestartFile, RestartFile, RestartFile, RestartFile) {
    let old_time = time_restart(&dir.join("old.nc"), 2, |name, p| match name {
        "wliq_soisno" => [100.0, 300.0][p],
        "wice_soisno" => 0.0,
        "t_soisno" => [280.0, 290.0][p],
        "tleaf" => [285.0, 295.0][p],
        "wa" => 4800.0,
        "z0m" => [0.1, 0.5][p],
        _ => 1.0,
    });
    let old_const = const_restart(&dir.join("old_const.nc"), &[1, 5], 2.0e6, 0.45);
    let new_time = time_restart(&dir.join("new.nc"), 2, |name, _| match name {
        "tlai" => 3.0,
        "tsai" => 0.5,
        "z0m" => 0.3,
        _ => 0.0,
    });
    let new_const = const_restart(&dir.join("new_const.nc"), &[1, 17], 2.0e6, 0.45);
    (old_time, old_const, new_time, new_const)
}

#[test]
fn changed_patches_mix_sources_by_transfer_fraction() {
    let dir = temp_dir("mix");
    let (old_time, old_const, new_time, new_const) = case(&dir);
    // 新 patch 0（类型 1）：来自旧类型 1 的 0.25、旧类型 5 的 0.75；新 patch 1（类型 17）不变。
    let mut lcc0 = vec![0.0; 18];
    lcc0[1] = 0.25;
    lcc0[5] = 0.75;
    let mut lcc1 = vec![0.0; 18];
    lcc1[17] = 1.0;
    let lccpct = [lcc0, lcc1];
    let inputs = MecInputs {
        new_time: &new_time,
        new_const: &new_const,
        new_element: &[7, 7],
        old_time: &old_time,
        old_const: &old_const,
        old_element: &[7, 7],
        lccpct: &lccpct,
        pft: None,
        urban: None,
    };
    let overrides = mass_energy_conserve(&inputs, Vec::new(), options()).unwrap().patch;
    let get = |name: &str| {
        overrides
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .values
            .clone()
    };
    let row = SNOW_LAYERS + SOIL;
    let wliq = get("wliq_soisno");
    // 水按份额平均：100*0.25 + 300*0.75，先乘后除再加，与上游同一次序。
    let expected = 0.0 + 100.0 * 0.25 / 1.0 + 300.0 * 0.75 / 1.0;
    assert_eq!(wliq[SNOW_LAYERS], expected);
    // 温度按热容加权：两个来源的热容不同，所以不是简单的份额平均。
    let t = get("t_soisno")[SNOW_LAYERS];
    assert!(t > 280.0 && t < 290.0 && (t - 287.5).abs() > 1.0e-9, "{t}");
    assert_eq!(
        get("tleaf")[0],
        0.0 + 285.0 * 0.25 / 1.0 + 295.0 * 0.75 / 1.0
    );
    // 无雪：雪层保持 0，`t_grnd` 取第一层土温，`sigf` 由 snowfraction 给 1。
    assert!(wliq[..SNOW_LAYERS].iter().all(|&v| v == 0.0));
    assert_eq!(get("t_grnd")[0], t);
    assert_eq!(get("sigf")[0], 1.0);
    assert_eq!(get("sai")[0], 0.5);
    assert_eq!(get("lai")[0], 3.0);
    // 份额不变的 patch 1 保留冷启动值（这里没有 SAT 替换）。
    assert_eq!(get("tleaf")[1], 0.0);
    assert!(get("wliq_soisno")[row..].iter().all(|&v| v == 0.0));
}

#[test]
fn a_missing_source_class_is_refused() {
    let dir = temp_dir("missing");
    let (old_time, old_const, new_time, new_const) = case(&dir);
    let mut lcc0 = vec![0.0; 18];
    lcc0[1] = 0.5;
    lcc0[8] = 0.5; // 旧单元里没有类型 8。
    let lccpct = [lcc0, vec![0.0; 18]];
    let inputs = MecInputs {
        new_time: &new_time,
        new_const: &new_const,
        new_element: &[7, 7],
        old_time: &old_time,
        old_const: &old_const,
        old_element: &[7, 7],
        lccpct: &lccpct,
        pft: None,
        urban: None,
    };
    let error = mass_energy_conserve(&inputs, Vec::new(), options()).unwrap_err();
    assert!(format!("{error:#}").contains("class 8"), "{error:#}");
}

#[test]
fn unchanged_fractions_keep_the_sat_result() {
    let dir = temp_dir("unchanged");
    let (old_time, old_const, new_time, new_const) = case(&dir);
    let mut lcc0 = vec![0.0; 18];
    lcc0[1] = 1.0;
    let mut lcc1 = vec![0.0; 18];
    lcc1[17] = 1.0;
    let lccpct = [lcc0, lcc1];
    let inputs = MecInputs {
        new_time: &new_time,
        new_const: &new_const,
        new_element: &[7, 7],
        old_time: &old_time,
        old_const: &old_const,
        old_element: &[7, 7],
        lccpct: &lccpct,
        pft: None,
        urban: None,
    };
    let sat = vec![RestartOverride::new("tleaf", vec![285.0, 0.0])];
    let overrides = mass_energy_conserve(&inputs, sat.clone(), options()).unwrap().patch;
    assert_eq!(overrides, sat);
}

/// PFT 时间重启：两个 PFT 都属于新 patch 0。
fn pft_restart(path: &Path) -> RestartFile {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("pft", 2).unwrap();
    for (name, values) in [
        ("ldew_p", [2.0, 0.0]),
        ("sigf_p", [0.0, 0.0]),
        ("sai_p", [0.0, 0.0]),
        ("lai_p", [0.0, 0.0]),
        ("z0m_p", [0.5, 0.0]),
        ("tlai_p", [3.0, 0.0]),
        ("tsai_p", [0.5, 0.0]),
    ] {
        file.add_variable::<f64>(name, &["pft"])
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    file.close().unwrap();
    RestartFile::open(path).unwrap()
}

#[test]
fn pft_merges_soil_classes_and_rescales_ldew_p() {
    let dir = temp_dir("pft");
    let (old_time, _, new_time, new_const) = case(&dir);
    // 旧单元：PFT 土壤 patch（类 1）与水体（类 17）。
    let old_const = const_restart(&dir.join("old_const_pft.nc"), &[1, 17], 2.0e6, 0.45);
    let pft_time = pft_restart(&dir.join("pft.nc"));
    // 新 patch 0：类 2 与类 5 都是土壤类，归并后是类 1 的 0.75；类 17 的 0.25。
    let mut lcc0 = vec![0.0; 18];
    lcc0[2] = 0.25;
    lcc0[5] = 0.5;
    lcc0[17] = 0.25;
    let mut lcc1 = vec![0.0; 18];
    lcc1[17] = 1.0;
    let lccpct = [lcc0, lcc1];
    let ranges = [0..2, 2..2];
    let inputs = |merge: bool| MecInputs {
        new_time: &new_time,
        new_const: &new_const,
        new_element: &[7, 7],
        old_time: &old_time,
        old_const: &old_const,
        old_element: &[7, 7],
        lccpct: &lccpct,
        urban: None,
        pft: Some(MecPft {
            time: &pft_time,
            sat: Vec::new(),
            pft_class: &[1, 0],
            pftfrac: &[0.25, 0.75],
            htop: &[10.0, 0.0],
            hbot: &[1.0, 0.0],
            ranges: &ranges,
            merge_soil_classes: merge,
            fast_pc: false,
        }),
    };
    // 不归并时类 2 的来源在旧单元里找不到。
    let error = mass_energy_conserve(&inputs(false), Vec::new(), options()).unwrap_err();
    assert!(format!("{error:#}").contains("class 2"), "{error:#}");

    let result = mass_energy_conserve(&inputs(true), Vec::new(), options()).unwrap();
    let get = |set: &[RestartOverride], name: &str| {
        set.iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} is overridden"))
            .values
            .clone()
    };
    // 两个来源的 ldew 都是 1，混合后 ldew = 1；ldew_tmp = 2*0.25 = 0.5，ldew_p 放大 2 倍。
    assert_eq!(get(&result.patch, "ldew")[0], 1.0);
    assert_eq!(get(&result.pft, "ldew_p"), vec![4.0, 0.0]);
    // 无雪：有冠层的 PFT sigf_p = 1，裸土 PFT 也是 1；sai_p = tsai_p，lai_p = tlai_p。
    assert_eq!(get(&result.pft, "sigf_p"), vec![1.0, 1.0]);
    assert_eq!(get(&result.patch, "sigf")[0], 1.0);
    assert_eq!(get(&result.pft, "sai_p"), vec![0.5, 0.0]);
    assert_eq!(get(&result.patch, "sai")[0], 0.125);
    assert_eq!(get(&result.patch, "lai")[0], 0.75);
}
