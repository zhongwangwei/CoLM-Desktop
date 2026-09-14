use super::*;

fn patches() -> FlatPatches {
    FlatPatches::new(
        vec![1, 1, 12],
        vec![0, 2, 3, 5],
        vec![0, 1, 2, 3, 4],
        vec![None, Some(0), None],
    )
    .unwrap()
}

#[test]
fn pft_fractions_match_weighting_wmo_crop_and_bare_soil_branches() {
    let output = aggregate_pft_fractions(
        &patches(),
        PftFractionInput {
            pft_offsets: &[0, 2, 3, 5],
            pft_classes: &[0, 1, 1, 0, 1],
            patch_kind: &[
                PftPatchKind::Natural,
                PftPatchKind::Natural,
                PftPatchKind::Crop,
            ],
            raw_class_count: 2,
            raw_percent: &[30.0, 50.0, 0.0, -1.0, -1.0, 70.0, 50.0, 100.0, -2.0, -3.0],
            land_area: &[1.0, 3.0, 1.0, 1.0, 1.0],
            crop_excluded_class: None,
        },
    )
    .unwrap();
    assert_eq!(output, [0.45, 0.55, 1.0, 1.0, 1.0]);

    let bare = aggregate_pft_fractions(
        &patches(),
        PftFractionInput {
            pft_offsets: &[0, 2, 3, 5],
            pft_classes: &[0, 1, 1, 0, 1],
            patch_kind: &[
                PftPatchKind::Natural,
                PftPatchKind::Other,
                PftPatchKind::Other,
            ],
            raw_class_count: 2,
            raw_percent: &[-1.0; 10],
            land_area: &[1.0; 5],
            crop_excluded_class: None,
        },
    )
    .unwrap();
    assert_eq!(bare, [1.0, 0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn pft_fractions_reject_an_out_of_range_class() {
    assert!(aggregate_pft_fractions(
        &patches(),
        PftFractionInput {
            pft_offsets: &[0, 2, 3, 5],
            pft_classes: &[0, 2, 1, 0, 1],
            patch_kind: &[PftPatchKind::Other; 3],
            raw_class_count: 2,
            raw_percent: &[0.0; 10],
            land_area: &[1.0; 5],
            crop_excluded_class: None,
        },
    )
    .is_err());
}

#[test]
fn landpft_keeps_positive_natural_classes_and_skips_non_soil_patches() {
    let layout =
        FlatPatches::new(vec![1, 17], vec![0, 2, 3], vec![0, 1, 2], vec![None; 2]).unwrap();
    let land_patches = crate::topology::FlatLandPatches {
        element_ids: vec![8, 9],
        pixel_start: vec![1, 1],
        pixel_end: vec![2, 1],
        set_type: vec![1, 17],
        element_index: vec![1, 2],
    };
    let topology = build_pft_topology(
        &land_patches,
        &layout,
        3,
        2,
        &[20.0, 0.0, 100.0, 80.0, 0.0, 0.0, 50.0, 0.0, 0.0],
        &[1.0, 3.0, 2.0],
    )
    .unwrap();

    assert_eq!(topology.patch_offsets, [0, 2, 2]);
    assert_eq!(topology.pft_classes, [0, 1]);
    assert_eq!(
        topology.land_pfts,
        crate::topology::FlatLandPatches {
            element_ids: vec![8, 8],
            pixel_start: vec![1, 1],
            pixel_end: vec![2, 2],
            set_type: vec![0, 1],
            element_index: vec![1, 1],
        }
    );
    assert_eq!(
        topology.patch_kind,
        [PftPatchKind::Natural, PftPatchKind::Other]
    );
}

#[test]
fn non_crop_pft_topology_retains_modis_class_sixteen() {
    let layout = FlatPatches::new(vec![1], vec![0, 1], vec![0], vec![None]).unwrap();
    let land_patches = crate::topology::FlatLandPatches {
        element_ids: vec![8],
        pixel_start: vec![1],
        pixel_end: vec![1],
        set_type: vec![1],
        element_index: vec![1],
    };
    let mut raw = vec![0.0; 16];
    raw[15] = 100.0;
    let topology = build_pft_topology(&land_patches, &layout, 16, 16, &raw, &[1.0]).unwrap();
    assert_eq!(topology.pft_classes, vec![15]);
    assert_eq!(topology.land_pfts.set_type, vec![15]);
}

#[test]
fn landpft_treats_all_igbp_soil_ground_classes_as_natural() {
    let natural = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 16];
    for land_type in 1..=17 {
        let layout = FlatPatches::new(vec![land_type], vec![0, 1], vec![0], vec![None]).unwrap();
        let land_patches = crate::topology::FlatLandPatches {
            element_ids: vec![8],
            pixel_start: vec![1],
            pixel_end: vec![1],
            set_type: vec![land_type],
            element_index: vec![1],
        };
        let topology =
            build_pft_topology(&land_patches, &layout, 3, 2, &[20.0, 80.0, 0.0], &[1.0]).unwrap();

        if natural.contains(&land_type) {
            assert_eq!(topology.patch_offsets, [0, 2], "IGBP {land_type}");
            assert_eq!(topology.pft_classes, [0, 1], "IGBP {land_type}");
            assert_eq!(
                topology.patch_kind,
                [PftPatchKind::Natural],
                "IGBP {land_type}"
            );
        } else {
            assert_eq!(topology.patch_offsets, [0, 0], "IGBP {land_type}");
            assert_eq!(topology.pft_classes, [], "IGBP {land_type}");
            assert_eq!(
                topology.patch_kind,
                [PftPatchKind::Other],
                "IGBP {land_type}"
            );
        }
    }
}

#[test]
fn crop_land_patches_split_only_shared_filter_class_one() {
    let layout = FlatPatches::new(
        vec![1, 12, 14],
        vec![0, 1, 2, 3],
        vec![0, 1, 2],
        vec![None; 3],
    )
    .unwrap();
    let land_patches = crate::topology::FlatLandPatches {
        element_ids: vec![8, 8, 8],
        pixel_start: vec![1, 2, 3],
        pixel_end: vec![1, 2, 3],
        set_type: vec![1, 12, 14],
        element_index: vec![1, 1, 1],
    };
    let crop = build_crop_land_patches(
        &land_patches,
        &layout,
        &[50.0, 50.0, 50.0],
        1,
        &[100.0, 100.0, 100.0],
        &[1.0, 1.0, 1.0],
    )
    .unwrap();

    assert_eq!(crop.land_patches.set_type, vec![1, 12, 12, 14]);
    assert_eq!(crop.crop_class, vec![None, Some(1), Some(1), None]);
    assert_eq!(crop.pctshared, vec![0.5, 0.5, 1.0, 1.0]);

    let mut raw = vec![0.0; 16 * 3];
    raw[0] = 100.0;
    raw[2] = 100.0;
    let pfts = build_crop_pft_topology(
        &crop.land_patches,
        &crop.layout,
        &crop.crop_class,
        16,
        16,
        &raw,
        &[1.0, 1.0, 1.0],
    )
    .unwrap();
    assert_eq!(
        pfts.patch_kind,
        [
            PftPatchKind::Natural,
            PftPatchKind::Crop,
            PftPatchKind::Crop,
            PftPatchKind::Natural
        ]
    );
}

#[test]
fn crop_topology_splits_shared_patches_and_preserves_cft_ownership() {
    let layout =
        FlatPatches::new(vec![1, 17], vec![0, 2, 3], vec![0, 1, 2], vec![None; 2]).unwrap();
    let land_patches = crate::topology::FlatLandPatches {
        element_ids: vec![8, 9],
        pixel_start: vec![1, 1],
        pixel_end: vec![2, 1],
        set_type: vec![1, 17],
        element_index: vec![1, 2],
    };
    let crop = build_crop_land_patches(
        &land_patches,
        &layout,
        &[20.0, 60.0, 0.0],
        2,
        &[25.0, 75.0, 0.0, 75.0, 25.0, 0.0],
        &[1.0, 3.0, 2.0],
    )
    .unwrap();

    assert_eq!(
        crop.land_patches.set_type,
        [1, IGBP_CROPLAND, IGBP_CROPLAND, 17]
    );
    for (actual, expected) in crop.pctshared.iter().zip([0.5, 0.3125, 0.1875, 1.0]) {
        assert!((actual - expected).abs() < 1.0e-12);
    }
    assert_eq!(crop.crop_class, [None, Some(1), Some(2), None]);
    assert_eq!(
        crop.layout,
        FlatPatches::new(
            vec![1, IGBP_CROPLAND, IGBP_CROPLAND, 17],
            vec![0, 2, 4, 6, 7],
            vec![0, 1, 0, 1, 0, 1, 2],
            vec![None; 4],
        )
        .unwrap()
    );

    let mut raw_pft = vec![0.0; 16 * 3];
    raw_pft[..3].copy_from_slice(&[50.0, 50.0, 0.0]);
    let pfts = build_crop_pft_topology(
        &crop.land_patches,
        &crop.layout,
        &crop.crop_class,
        16,
        15,
        &raw_pft,
        &[1.0, 3.0, 2.0],
    )
    .unwrap();
    assert_eq!(pfts.patch_offsets, [0, 1, 2, 3, 3]);
    assert_eq!(pfts.pft_classes, [0, 15, 16]);
    assert_eq!(
        pfts.patch_kind,
        [
            PftPatchKind::Natural,
            PftPatchKind::Crop,
            PftPatchKind::Crop,
            PftPatchKind::Other,
        ]
    );
    let fraction = aggregate_pft_fractions(
        &crop.layout,
        PftFractionInput {
            pft_offsets: &pfts.patch_offsets,
            pft_classes: &pfts.pft_classes,
            patch_kind: &pfts.patch_kind,
            raw_class_count: 16,
            raw_percent: &raw_pft,
            land_area: &[1.0, 3.0, 2.0],
            crop_excluded_class: Some(15),
        },
    )
    .unwrap();
    assert_eq!(fraction, [1.0, 1.0, 1.0]);
    for (actual, expected) in crop_pft_pctshared(&pfts, &fraction, &crop.pctshared)
        .unwrap()
        .iter()
        .zip([1.0, 0.3125, 0.1875])
    {
        assert!((actual - expected).abs() < 1.0e-12);
    }
}

#[test]
fn pft_index_matches_lai_weighting_crop_and_wmo_paths() {
    let layout = FlatPatches::new(
        vec![1, 1, 12],
        vec![0, 2, 3, 5],
        vec![0, 1, 2, 3, 4],
        vec![None, Some(0), None],
    )
    .unwrap();
    let mut raw_percent = vec![0.0; 13 * 5];
    let mut raw_index = vec![0.0; 13 * 5];
    for (class, values, indices) in [
        (12, [25.0, 50.0, 80.0, 0.0, 0.0], [2.0, 4.0, 6.0, 0.0, 0.0]),
        (1, [75.0, 50.0, 20.0, 0.0, 0.0], [8.0, 12.0, 16.0, 0.0, 0.0]),
        (0, [0.0, 0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 0.0, 6.0, 6.0]),
    ] {
        for cell in 0..5 {
            raw_percent[class * 5 + cell] = values[cell];
            raw_index[class * 5 + cell] = indices[cell];
        }
    }
    let state = aggregate_pft_index(
        &layout,
        PftIndexInput {
            pft_offsets: &[0, 2, 3, 4],
            pft_classes: &[12, 1, 12, 0],
            patch_kind: &[
                PftPatchKind::Natural,
                PftPatchKind::Natural,
                PftPatchKind::Crop,
            ],
            raw_class_count: 13,
            raw_percent: &raw_percent,
            raw_index: &raw_index,
            land_area: &[1.0, 3.0, 1.0, 1.0, 1.0],
        },
    )
    .unwrap();
    assert_eq!(state.patch_index, [7.625, 26.0 / 7.0, 6.0]);
    assert_eq!(state.pft_index, [26.0 / 7.0, 32.0 / 3.0, 26.0 / 7.0, 6.0]);
}

#[test]
fn pft_index_keeps_non_pft_land_patches_at_zero() {
    let layout = FlatPatches::new(vec![1, 17], vec![0, 1, 2], vec![0, 1], vec![None; 2]).unwrap();
    let state = aggregate_pft_index(
        &layout,
        PftIndexInput {
            pft_offsets: &[0, 1, 1],
            pft_classes: &[0],
            patch_kind: &[PftPatchKind::Natural, PftPatchKind::Other],
            raw_class_count: 1,
            raw_percent: &[100.0, 0.0],
            raw_index: &[3.0, 7.0],
            land_area: &[1.0, 1.0],
        },
    )
    .unwrap();
    assert_eq!(state.patch_index, [3.0, 0.0]);
    assert_eq!(state.pft_index, [3.0]);
}

#[test]
fn pft_height_uses_pft_weights_and_patch_mean_fallback() {
    let layout = FlatPatches::new(vec![1], vec![0, 2], vec![0, 1], vec![None]).unwrap();
    let height = aggregate_pft_height(
        &layout,
        PftFractionInput {
            pft_offsets: &[0, 3],
            pft_classes: &[0, 1, 2],
            patch_kind: &[PftPatchKind::Natural],
            raw_class_count: 3,
            raw_percent: &[25.0, 50.0, 75.0, 50.0, 0.0, 0.0],
            land_area: &[1.0, 3.0],
            crop_excluded_class: None,
        },
        &[10.0, 20.0],
    )
    .unwrap();
    assert_eq!(height, [130.0 / 7.0, 50.0 / 3.0, 17.5]);
}
