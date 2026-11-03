use super::*;

#[test]
fn pft_restart_path_inserts_the_pft_tag_once() {
    let path = Path::new("/r/2010-001-00000/at_restart_2010-001-00000_lc2005_w180_s90.nc");
    assert_eq!(
        pft_restart_path(path).unwrap(),
        Path::new("/r/2010-001-00000/at_restart_pft_2010-001-00000_lc2005_w180_s90.nc")
    );
    let constant = Path::new("/r/const/at_restart_const_lc2005_w180_s90.nc");
    assert_eq!(
        pft_restart_path(constant).unwrap(),
        Path::new("/r/const/at_restart_pft_const_lc2005_w180_s90.nc")
    );
}

#[test]
fn absorption_layout_round_trips_the_netcdf_order() {
    let matrix = [[1.0, 2.0], [3.0, 4.0]];
    // (rtyp, band) 行主序：rtyp=0 的两个波段在前。
    assert_eq!(flatten_absorption(matrix), [1.0, 3.0, 2.0, 4.0]);
}

/// 单 patch 拥有全部 PFT；多作物单点每个 patch 恰好一个 PFT；其它组合报错。
#[test]
fn patch_pft_ranges_follow_the_single_point_crop_layout() {
    assert_eq!(patch_pft_range(1, 3, 0).unwrap(), 0..3);
    assert_eq!(patch_pft_range(2, 2, 1).unwrap(), 1..2);
    assert!(patch_pft_range(2, 3, 0).is_err());
    assert!(patch_pft_range(2, 2, 2).is_err());
}

/// 混合模型 `pft` 插槽：把每个参数的有效值原样作为这个 PFT 的覆盖传进去，组出的参数与不覆盖逐位相同
/// （覆盖走的正是查表那条路，派生量照算）。
#[test]
fn echoing_every_pft_parameter_as_a_hybrid_override_changes_nothing() {
    let document = colm_namelist::parse("&nl_colm\n/\n").unwrap();
    let interfaces = colm_core::colm_soil_grid(10).unwrap().interface_depth_m;
    for campbell in [false, true] {
        for pc in [false, true] {
            for class in 0..16 {
                let plain =
                    pft_parameters_uncached(&document, class, campbell, pc, &interfaces).unwrap();
                let mut echo = std::collections::BTreeMap::new();
                for meta in colm_case::pft::all_parameters() {
                    if let Ok(value) =
                        colm_init::pft_parameter(&document, meta.name, class, campbell, pc)
                    {
                        echo.insert(meta.name.to_owned(), value);
                    }
                }
                assert!(echo.contains_key("DEF_PFT_VMAX25"));
                let echoed =
                    pft_parameters_with(&document, class, campbell, pc, &interfaces, Some(&echo))
                        .unwrap();
                assert_eq!(
                    format!("{plain:?}"),
                    format!("{echoed:?}"),
                    "class {class} campbell {campbell} pc {pc}"
                );
            }
        }
    }
}

/// 覆盖确实生效：Vcmax 换成别的值，组出的参数跟着变（与 namelist 的 `DEF_PFT_VMAX25` 同样换算）。
#[test]
fn a_hybrid_vmax25_override_reaches_the_pft_parameters() {
    let document = colm_namelist::parse("&nl_colm\n/\n").unwrap();
    let interfaces = colm_core::colm_soil_grid(10).unwrap().interface_depth_m;
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert("DEF_PFT_VMAX25".to_owned(), 40.0);
    let changed =
        pft_parameters_with(&document, 2, false, true, &interfaces, Some(&overrides)).unwrap();
    let namelist = colm_namelist::parse("&nl_colm\n DEF_PFT_VMAX25(3) = 40.0\n/\n").unwrap();
    let via_namelist = pft_parameters_uncached(&namelist, 2, false, true, &interfaces).unwrap();
    assert_eq!(format!("{changed:?}"), format!("{via_namelist:?}"));
    let plain = pft_parameters_uncached(&document, 2, false, true, &interfaces).unwrap();
    assert_ne!(format!("{changed:?}"), format!("{plain:?}"));
}

#[test]
fn pc_restart_crown_ratio_reaches_runtime_parameters_and_rs_never_silently_defaults() {
    let root = std::env::temp_dir().join(format!("colm-pc-runtime-crown-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let time = root.join("time.nc");
    let mut file = netcdf::create(&time).unwrap();
    for (name, count) in [("pft", 1), ("band", 2), ("rtyp", 2)] {
        file.add_dimension(name, count).unwrap();
    }
    for name in PFT_VECTORS {
        file.add_variable::<f64>(name, &["pft"])
            .unwrap()
            .put_values(&[1.0], ..)
            .unwrap();
    }
    for name in ["ssun_p", "ssha_p"] {
        file.add_variable::<f64>(name, &["pft", "band", "rtyp"])
            .unwrap()
            .put_values(&[0.1; 4], (.., .., ..))
            .unwrap();
    }
    file.close().unwrap();
    let constant = root.join("constant.nc");
    let missing = root.join("missing.nc");
    for (path, ratio) in [(&constant, Some(0.5)), (&missing, None)] {
        let mut file = netcdf::create(path).unwrap();
        file.add_dimension("pft", 1).unwrap();
        file.add_variable::<i32>("pftclass", &["pft"])
            .unwrap()
            .put_values(&[1], ..)
            .unwrap();
        for (name, value) in [("pftfrac", 1.0), ("htop_p", 20.0), ("hbot_p", 6.0)] {
            file.add_variable::<f64>(name, &["pft"])
                .unwrap()
                .put_values(&[value], ..)
                .unwrap();
        }
        if let Some(ratio) = ratio {
            file.add_variable::<f64>("cratio_p", &["pft"])
                .unwrap()
                .put_values(&[ratio], ..)
                .unwrap();
        }
        file.close().unwrap();
    }
    let source = "&nl_colm\n DEF_USE_PC=.true.\n DEF_USE_PFT=.false.\n DEF_USE_LCT=.false.\n DEF_USE_Campbell_SOIL_MODEL=.true.\n DEF_USE_PLANTHYDRAULICS=.false.\n DEF_Interception_scheme=1\n/\n";
    let document = colm_namelist::parse(source).unwrap();
    let physics = crate::physics::land_physics_parameters(
        &document,
        colm_core::LandCoverScheme::Igbp,
        crate::physics::ObservationHeights {
            wind_m: 10.0,
            temperature_m: 2.0,
            humidity_m: 2.0,
            mode: colm_core::ObservationHeightMode::Absolute,
        },
    )
    .unwrap();
    let interfaces = colm_core::colm_soil_grid(10).unwrap().interface_depth_m;
    let read = |path: &Path, doc: &Document| {
        PftTemplate::read(path, &time, doc, &physics, &interfaces, 0, None)
    };
    assert_eq!(
        read(&constant, &document).unwrap().initial.parameters[0].crown_aspect_ratio,
        0.5
    );
    assert_eq!(
        read(&missing, &document).unwrap().initial.parameters[0].crown_aspect_ratio,
        1.0
    );
    let observed =
        colm_namelist::parse(&source.replace("/\n", " DEF_RS_CROWN_STRUCTURE=.true.\n/\n"))
            .unwrap();
    assert!(read(&missing, &observed)
        .unwrap_err()
        .to_string()
        .contains("cratio_p"));
    assert_eq!(
        read(&constant, &observed).unwrap().initial.parameters[0].crown_aspect_ratio,
        0.5
    );
    std::fs::remove_dir_all(root).unwrap();
}
