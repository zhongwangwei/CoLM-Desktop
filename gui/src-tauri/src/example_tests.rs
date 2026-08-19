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
    ] {
        let p = dest.join(d).join(f);
        assert!(p.is_file(), "{} 没复制过去", p.display());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), d);
    }
}
