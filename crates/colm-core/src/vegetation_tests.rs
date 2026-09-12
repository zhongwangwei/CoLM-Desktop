use super::*;

#[test]
fn usgs_canopy_uses_class_defaults_without_an_igbp_height_override() {
    let state = derive_usgs_canopy(&[0, 2], &[1.0, 3.0, 5.0], &[0.1, 0.3, 0.5]).unwrap();
    assert_eq!(state.patch_top_m, [1.0, 5.0]);
    assert_eq!(state.patch_bottom_m, [0.1, 0.5]);
    assert!(state.pft_top_m.is_empty());
    assert!(derive_usgs_canopy(&[3], &[1.0], &[0.1]).is_err());
}

#[test]
fn igbp_canopy_uses_observed_height_only_for_trees_and_woody_savanna() {
    let state = derive_igbp_canopy(
        &[1, 6, 8],
        &[0, 0, 0],
        &[4.0, 99.0, 3.0],
        &[0.0, 17.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 1.0],
        &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        None,
    )
    .unwrap();
    assert_eq!(state.patch_top_m, vec![4.0, 0.5, 3.0]);
    assert_eq!(state.patch_bottom_m, vec![1.0, 0.0, 1.0]);
}

#[test]
fn pft_canopy_replaces_only_natural_patch_height_with_fraction_weighted_values() {
    let state = derive_igbp_canopy(
        &[1, 13],
        &[0, 1],
        &[17.0, 1.0],
        &[
            0.0, 17.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5,
        ],
        &[
            0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
        Some(PftCanopyInput {
            offsets: &[0, 2, 2],
            class: &[1, 9],
            fraction: &[0.3, 0.7],
            observed_top_m: &[5.0, 100.0],
            default_top_m: &[0.5, 17.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5],
            default_bottom_m: &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        }),
    )
    .unwrap();
    assert_eq!(state.pft_top_m, vec![5.0, 0.5]);
    assert_eq!(state.patch_top_m, vec![1.85, 0.5]);
    assert_eq!(state.patch_bottom_m, vec![0.3, 0.0]);
}

#[test]
fn canopy_rejects_invalid_land_or_pft_topology() {
    assert!(derive_igbp_canopy(&[1], &[0], &[1.0], &[0.0], &[0.0], None).is_err());
    assert!(derive_igbp_canopy(
        &[1],
        &[0],
        &[1.0],
        &[0.0, 1.0],
        &[0.0, 1.0],
        Some(PftCanopyInput {
            offsets: &[0, 2],
            class: &[1],
            fraction: &[1.0],
            observed_top_m: &[1.0],
            default_top_m: &[0.0, 1.0],
            default_bottom_m: &[0.0, 1.0],
        }),
    )
    .is_err());
}

#[test]
fn empirical_lai_matches_the_igbp_temperature_and_root_depth_rule() {
    let vegetation = empirical_lai(
        EmpiricalLandCover::Igbp,
        12,
        &[0.5, 0.41, 0.09],
        &[300.0, 288.0, 270.0],
    )
    .unwrap();
    assert!((vegetation.leaf_area_index - 4.18).abs() < 1.0e-12);
    assert_eq!(vegetation.stem_area_index, 0.4);
    assert_eq!(vegetation.vegetation_fraction, 1.0);
    assert_eq!(vegetation.greenness, 1.0);
}

#[test]
fn empirical_lai_keeps_all_upstream_land_cover_tables_and_rejects_bad_inputs() {
    let water = empirical_lai(EmpiricalLandCover::Usgs, 16, &[1.0], &[280.0]).unwrap();
    assert_eq!(
        water,
        EmpiricalVegetation {
            leaf_area_index: 0.0,
            stem_area_index: 0.0,
            vegetation_fraction: 0.0,
            greenness: 0.0,
        }
    );
    assert!(empirical_lai(EmpiricalLandCover::Igbp, 18, &[1.0], &[280.0]).is_err());
    assert!(empirical_lai(EmpiricalLandCover::Igbp, 1, &[1.0], &[]).is_err());
}
