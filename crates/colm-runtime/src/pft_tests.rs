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
