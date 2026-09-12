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
