//! 对全部 90 个真实 PLUMBER2 站点文件跑一遍补齐。
//!
//! 合成用例能证明每一步的算术，只有真实文件能证明**它对所有站点都成立**。
//! 先前的实现在 CN-Cng 上看起来完全正确，而它对另外 89 个站点是错的 ——
//! 因为它把一个恰好在 CN-Cng 成立的常数写死了。

use std::path::PathBuf;

use colm_srfdata::site::{fill, missing_fields};

fn plumber2() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("PLUMBER2_ROOT")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s".to_string()),
    );
    assert!(
        p.join("Sitedata").is_dir(),
        "PLUMBER2 not found at {}; set PLUMBER2_ROOT",
        p.display()
    );
    p
}

fn rawdata() -> PathBuf {
    PathBuf::from(
        std::env::var("COLM_RAWDATA")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/rawdata".to_string()),
    )
}

fn site_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(plumber2().join("Sitedata"))
        .expect("readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "nc"))
        .collect();
    out.sort();
    assert!(
        out.len() >= 85,
        "expected ~90 site files, found {}",
        out.len()
    );
    out
}

#[test]
fn every_site_is_missing_exactly_the_same_twelve_fields() {
    for f in site_files() {
        let m = missing_fields(&f).expect("readable");
        assert_eq!(m.len(), 12, "{}: missing {:?}", f.display(), m);
    }
}

#[test]
fn every_site_fills_and_lands_inside_the_usda_triangle() {
    let dir = std::env::temp_dir().join("colm-srfdata-real-sites");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workdir");
    let raw = rawdata();
    let raw = raw.join("soil_brightness.nc").exists().then_some(raw);

    let mut failures = Vec::new();
    let mut classes = std::collections::BTreeMap::new();
    for f in site_files() {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let out = dir.join(format!("{name}.nc"));
        match fill(&f, &out, raw.as_deref()) {
            Ok(r) => {
                *classes.entry(r.texture).or_insert(0usize) += 1;
                let (s, si, c) = r.fine_earth;
                if (s + si + c - 100.0).abs() > 1e-6 {
                    failures.push(format!("{name}: fractions sum to {}", s + si + c));
                }
            }
            Err(e) => failures.push(format!("{name}: {e:#}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} site(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // 全部站点判成同一类几乎必然是分类器坏了，而不是世界如此
    assert!(
        classes.len() >= 3,
        "only {} distinct texture classes across all sites: {classes:?}",
        classes.len()
    );
    println!("texture classes across sites: {classes:?}");
}

#[test]
fn the_raster_and_the_classifier_disagree_about_as_often_as_measured() {
    // 实测：90 个站点里 25 个一致。这条不是要求它们一致 —— 它们出自不同的
    // 土壤产品，本来就不该一致 —— 而是钉住分歧的量级。若某天一致率跳到
    // 90% 或掉到 5%，说明有一侧变了，那值得有人看一眼。
    let raw = rawdata();
    if !raw.join("soil/soiltexture_0cm-60cm_mean.nc").exists() {
        panic!(
            "texture raster missing at {}; set COLM_RAWDATA",
            raw.display()
        );
    }
    let dir = std::env::temp_dir().join("colm-srfdata-texture-agreement");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workdir");
    let mut agree = 0usize;
    let mut total = 0usize;
    for f in site_files() {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let out = dir.join(format!("{name}.nc"));
        let r = fill(&f, &out, Some(&raw)).expect("fills");
        total += 1;
        if r.texture == r.classified_texture {
            agree += 1;
        }
    }
    println!("raster and classifier agree on {agree} of {total} sites");
    assert!(
        (15..=40).contains(&agree),
        "agreement was {agree}/{total}; measured was 25/90, so something changed"
    );
}
