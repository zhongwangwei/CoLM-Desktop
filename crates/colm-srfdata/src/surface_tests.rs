use super::*;

fn patches(types: Vec<i32>, offsets: Vec<usize>, cells: Vec<usize>) -> FlatPatches {
    let count = types.len();
    FlatPatches::new(types, offsets, cells, vec![None; count]).unwrap()
}

#[test]
fn flat_layout_rejects_invalid_offsets_and_forward_wmo_references() {
    assert!(FlatPatches::new(vec![1], vec![0, 2], vec![0], vec![None]).is_err());
    assert!(FlatPatches::new(vec![1], vec![0, 1], vec![0], vec![Some(0)]).is_err());
}

#[test]
fn soil_texture_uses_fortrans_smallest_value_on_a_frequency_tie() {
    let layout = patches(vec![0, 0], vec![0, 4, 6], vec![3, 2, 3, 2, 9, 8]);
    assert_eq!(
        layout
            .aggregate_soil_texture(&[0, 0, 2, 3, 0, 0, 0, 0, 8, 9])
            .unwrap(),
        vec![2, 8]
    );
}

#[test]
fn lake_depth_matches_fortrans_decimeter_scale_median_and_missing_marker() {
    let layout = patches(vec![17, 3, 17], vec![0, 4, 6, 7], vec![0, 1, 2, 3, 4, 5, 6]);
    let result = layout
        .aggregate_lake_depth(&[10.0, 20.0, 30.0, 40.0, 999.0, 999.0, 75.0], 17)
        .unwrap();
    assert_eq!(result, vec![2.5, SURFACE_MISSING, 7.5]);
}

#[test]
fn bedrock_is_area_weighted_and_wmo_patches_reuse_the_prior_value() {
    let layout = FlatPatches::new(
        vec![1, 1],
        vec![0, 2, 3],
        vec![0, 1, 2],
        vec![None, Some(0)],
    )
    .unwrap();
    assert_eq!(
        layout
            .aggregate_bedrock(&[10.0, 40.0, 999.0], &[3.0, 1.0, 99.0])
            .unwrap(),
        vec![17.5, 17.5]
    );
}

#[test]
fn forest_height_preserves_the_usgs_median_and_igbp_weighted_branches() {
    let layout = FlatPatches::new(
        vec![0, 1, 2, 17, 15],
        vec![0, 1, 4, 6, 7, 8],
        vec![0, 1, 2, 3, 4, 5, 6, 7],
        vec![None; 5],
    )
    .unwrap();
    let height = [99.0, 1.0, 11.0, 100.0, 4.0, 10.0, 99.0, 99.0];
    assert_eq!(
        layout
            .aggregate_usgs_forest_height(&height, 1, 17, 15)
            .unwrap(),
        vec![
            SURFACE_MISSING,
            SURFACE_MISSING,
            7.0,
            SURFACE_MISSING,
            SURFACE_MISSING
        ]
    );
    assert_eq!(
        layout
            .aggregate_igbp_forest_height(&height, &[1.0, 1.0, 3.0, 2.0, 1.0, 3.0, 1.0, 1.0])
            .unwrap(),
        vec![SURFACE_MISSING, 39.0, 8.5, 99.0, 99.0]
    );
}

#[test]
fn soil_brightness_reuses_colms_tables_skips_missing_classes_and_marks_water() {
    let layout = FlatPatches::new(
        vec![1, 17, 1],
        vec![0, 3, 4, 5],
        vec![0, 1, 2, 3, 4],
        vec![None, None, Some(0)],
    )
    .unwrap();
    let value = layout
        .aggregate_soil_brightness(&[1, 20, 0, 7, 12], 17, 15)
        .unwrap();
    assert_eq!(value.saturated_visible, vec![0.15, SURFACE_MISSING, 0.15]);
    assert_eq!(value.dry_visible, vec![0.26, SURFACE_MISSING, 0.26]);
    assert_eq!(
        value.saturated_near_infrared,
        vec![0.30, SURFACE_MISSING, 0.30]
    );
    for (got, want) in value
        .dry_near_infrared
        .iter()
        .copied()
        .zip([0.41, SURFACE_MISSING, 0.41])
    {
        assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
    }
}

#[test]
fn topography_is_area_weighted_and_wmo_patches_copy_an_earlier_result() {
    let layout = FlatPatches::new(
        vec![1, 1, 1],
        vec![0, 2, 3, 4],
        vec![0, 1, 2, 3],
        vec![None, Some(0), None],
    )
    .unwrap();
    let topography = layout
        .aggregate_topography(
            &[2.0, 1.0, 99.0, 1.0],
            &[10.0, 20.0, 9999.0, -9999.0],
            &[3.0, 4.0, 99.0, 99.0],
            &[1.0, 2.0, 99.0, 99.0],
        )
        .unwrap();
    let mean: f64 = 40.0 / 3.0;
    let expected_std =
        ((((10.0 - mean).powi(2) + 9.0) * 2.0 + ((20.0 - mean).powi(2) + 16.0)) / 3.0).sqrt();
    assert!((topography.elevation[0] - mean).abs() < 1e-12);
    assert!((topography.elevation_std[0] - expected_std).abs() < 1e-12);
    assert!((topography.slope_ratio[0] - 4.0 / 3.0).abs() < 1e-12);
    assert_eq!(topography.elevation[1], topography.elevation[0]);
    assert_eq!(topography.elevation_std[1], topography.elevation_std[0]);
    assert_eq!(topography.slope_ratio[1], topography.slope_ratio[0]);
    assert_eq!(topography.elevation[2], 0.0);
    assert_eq!(topography.elevation_std[2], 0.0);
    assert_eq!(topography.slope_ratio[2], 0.0);
}

#[test]
fn aggregators_reject_a_raw_cell_outside_the_source() {
    let layout = patches(vec![17], vec![0, 1], vec![4]);
    let err = layout.aggregate_lake_depth(&[1.0], 17).unwrap_err();
    assert!(err.to_string().contains("raw cell 4"), "{err:#}");
}
