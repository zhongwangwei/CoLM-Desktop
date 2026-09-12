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
