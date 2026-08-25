#[test]
fn development_prefers_repository_examples_over_stale_staged_resources() {
    let resource = std::path::PathBuf::from("/staged");
    let roots = super::example_roots(Some(resource.clone()));
    let staged = resource.join("examples");
    let staged_position = roots.iter().position(|root| root == &staged).unwrap();
    let repository_position = roots
        .iter()
        .position(|root| root.ends_with("../../examples"))
        .unwrap();
    if cfg!(debug_assertions) {
        assert!(repository_position < staged_position);
    } else {
        assert!(staged_position < repository_position);
    }
}

#[test]
fn bundled_crop_example_contains_the_files_needed_to_create_a_case() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for file in [
        "Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc",
        "Forcing/US-Ne3_2002-2003_FLUXNET2015_CROP_Met.nc",
        "Forcingnml/US-Ne3.nml",
        "Runtime/ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc",
    ] {
        assert!(root.join(file).is_file(), "missing bundled CROP file: {file}");
    }
}

#[test]
fn copying_the_tree_keeps_the_layout_that_scanning_depends_on() {
    // `colm-cli scan` 顺着命名约定从 Sitedata 找到 ../Forcing 与
    // ../Observation。复制时把三个目录压平，扫描仍然能列出站点，
    // 但强迫场找不到 —— 而那时界面上显示的是"这个站点没有强迫场，跑不了"。
    let root = std::env::temp_dir().join("colm-example-copy");
    let _ = std::fs::remove_dir_all(&root);
    let src = root.join("src");
    for (d, f) in [
        ("Sitedata", "X_site.nc"),
        ("Forcing", "X_Met.nc"),
        ("Observation", "X_Flux.nc"),
        ("Runtime/ndep", "X_runtime.nc"),
    ] {
        std::fs::create_dir_all(src.join(d)).unwrap();
        std::fs::write(src.join(d).join(f), d).unwrap();
    }
    let dest = root.join("dest");
    super::copy_tree(&src, &dest).unwrap();
    for (d, f) in [
        ("Sitedata", "X_site.nc"),
        ("Forcing", "X_Met.nc"),
        ("Observation", "X_Flux.nc"),
        ("Runtime/ndep", "X_runtime.nc"),
    ] {
        let p = dest.join(d).join(f);
        assert!(p.is_file(), "{} 没复制过去", p.display());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), d);
    }

    // 升级时补新站点，但用户改过的旧示例不能被覆盖。
    std::fs::write(dest.join("Sitedata/X_site.nc"), "user edit").unwrap();
    std::fs::write(src.join("Sitedata/Y_site.nc"), "new example").unwrap();
    super::copy_tree(&src, &dest).unwrap();
    assert_eq!(
        std::fs::read_to_string(dest.join("Sitedata/X_site.nc")).unwrap(),
        "user edit"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("Sitedata/Y_site.nc")).unwrap(),
        "new example"
    );
}
