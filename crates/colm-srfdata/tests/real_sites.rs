//! 对全部 90 个真实 PLUMBER2 站点文件跑一遍补齐。
//!
//! 合成用例能证明每一步的算术，只有真实文件能证明**它对所有站点都成立**。
//! 先前的实现在 CN-Cng 上看起来完全正确，而它对另外 89 个站点是错的 ——
//! 因为它把一个恰好在 CN-Cng 成立的常数写死了。

use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use colm_srfdata::site::{fill, missing_fields};

static NEXT_TMP: AtomicUsize = AtomicUsize::new(0);

fn unique_workdir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).expect("workdir");
    dir
}

fn plumber2() -> Option<PathBuf> {
    let Ok(root) = std::env::var("PLUMBER2_ROOT") else {
        eprintln!("skipping PLUMBER2 real-site test; set PLUMBER2_ROOT to run it");
        return None;
    };
    let p = PathBuf::from(root);
    assert!(
        p.join("Sitedata").is_dir(),
        "PLUMBER2 not found at {}; set PLUMBER2_ROOT",
        p.display()
    );
    Some(p)
}

fn rawdata() -> PathBuf {
    PathBuf::from(
        std::env::var("COLM_RAWDATA")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/rawdata".to_string()),
    )
}

fn site_files() -> Option<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(plumber2()?.join("Sitedata"))
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
    Some(out)
}

#[test]
fn real_site_workdirs_are_unique_per_test_call() {
    let a = unique_workdir("colm-srfdata-real-sites-proof");
    let b = unique_workdir("colm-srfdata-real-sites-proof");
    assert_ne!(a, b);
}

#[test]
fn every_site_is_missing_exactly_the_same_twelve_fields() {
    let Some(files) = site_files() else { return };
    for f in files {
        let m = missing_fields(&f).expect("readable");
        assert_eq!(m.len(), 12, "{}: missing {:?}", f.display(), m);
    }
}

#[test]
fn every_site_fills_and_lands_inside_the_usda_triangle() {
    let dir = unique_workdir("colm-srfdata-real-sites");
    let raw = rawdata();
    let raw = raw.join("soil_brightness.nc").exists().then_some(raw);

    let mut failures = Vec::new();
    let mut classes = std::collections::BTreeMap::new();
    let Some(files) = site_files() else { return };
    for f in files {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let out = dir.join(format!("{name}.nc"));
        match fill(&f, &out, raw.as_deref(), None) {
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
    // 实测：90 个站点里 26 个一致。这条不是要求它们一致 —— 它们出自不同的
    // 土壤产品，本来就不该一致 —— 而是钉住分歧的量级。若某天一致率跳到
    // 90% 或掉到 5%，说明有一侧变了，那值得有人看一眼。
    //
    // 26 里有 1 个是「一致」得来的另一种方式：DK-Lva（12.083E 55.683N）在
    // 质地栅格上读到 _FillValue(-1)，被 (1..=12) 过滤挡下、回落到分类器，
    // 于是两者当然相同。那不是两个产品达成了一致，而是只有一个产品有话说 ——
    // 正好也证明填充值兜底在真实数据上是通的。
    let raw = rawdata();
    if !raw.join("soil/soiltexture_0cm-60cm_mean.nc").exists() {
        panic!(
            "texture raster missing at {}; set COLM_RAWDATA",
            raw.display()
        );
    }
    let dir = unique_workdir("colm-srfdata-texture-agreement");
    let mut agree = 0usize;
    let mut total = 0usize;
    let Some(files) = site_files() else { return };
    for f in files {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        let out = dir.join(format!("{name}.nc"));
        let r = fill(&f, &out, Some(&raw), None).expect("fills");
        total += 1;
        if r.raster_texture == Some(r.texture) {
            agree += 1;
        }
    }
    println!("raster and classifier agree on {agree} of {total} sites");
    assert!(
        (15..=40).contains(&agree),
        "agreement was {agree}/{total}; measured was 25/90, so something changed"
    );
}

#[test]
fn every_site_file_carries_its_own_location_and_landtype() {
    // 新建算例时这三项不该问用户。实测 90 个站点文件全都自带。
    let Some(root) = plumber2() else { return };
    let dir = root.join("Sitedata");
    let mut n = 0;
    let mut classes = std::collections::BTreeSet::new();
    for e in std::fs::read_dir(&dir).expect("Sitedata") {
        let p = e.expect("entry").path();
        if p.extension().and_then(|x| x.to_str()) != Some("nc") {
            continue;
        }
        let l = colm_srfdata::site::location(&p).expect("location");
        assert!((-180.0..=180.0).contains(&l.lon), "{p:?} lon {}", l.lon);
        assert!((-90.0..=90.0).contains(&l.lat), "{p:?} lat {}", l.lat);
        // PLUMBER2 的 90 个站全都带 IGBP_classification；城市站点文件不带，
        // 那时是 None（见 Urban-PLUMBER）。这里断言这个语料里一个都不缺。
        let lt = l
            .landtype
            .unwrap_or_else(|| panic!("{p:?} has no IGBP class"));
        assert!((1..=17).contains(&lt), "{p:?} IGBP {lt}");
        classes.insert(lt);
        n += 1;
    }
    assert_eq!(n, 90);
    // 90 个站点不该全是同一种地类 —— 那说明读错了字段。
    assert!(
        classes.len() > 3,
        "only {} distinct IGBP classes",
        classes.len()
    );
}

#[test]
fn the_golden_site_matches_the_hand_written_case() {
    // 手写的 oracle/cases/CN-Cng/case.nml 里那三个数就是从这里来的。
    let Some(root) = plumber2() else { return };
    let p = root.join("Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc");
    let l = colm_srfdata::site::location(&p).expect("location");
    assert!((l.lon - 123.5092).abs() < 1e-4, "{}", l.lon);
    assert!((l.lat - 44.5933).abs() < 1e-4, "{}", l.lat);
    assert_eq!(l.landtype, Some(10));
}
