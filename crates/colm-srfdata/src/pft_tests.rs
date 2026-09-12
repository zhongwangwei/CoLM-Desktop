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
