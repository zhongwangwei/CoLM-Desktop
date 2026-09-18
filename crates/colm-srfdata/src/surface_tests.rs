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
fn methane_ph_averages_hydrogen_activity_and_keeps_nonsoil_fallbacks() {
    let layout = patches(vec![1, 17, 11], vec![0, 2, 3, 5], vec![0, 1, 2, 3, 4]);
    let result = layout
        .aggregate_methane_ph(
            &[4.0, 6.0, 2.0, 4.0, f64::NAN],
            &[1.0, 3.0, 1.0, 2.0, 0.0],
            &[1.0, 1.0, 1.0, 2.0, 1.0],
            |land_cover| !matches!(land_cover, 13 | 15 | 17),
        )
        .unwrap();
    assert!((result[0] + ((10_f64.powi(-4) + 3.0 * 10_f64.powi(-6)) / 4.0).log10()).abs() < 1e-12);
    assert_eq!(result[1], 6.2);
    assert_eq!(result[2], 4.0);
}

#[test]
fn lake_soil_carbon_masks_missing_values_and_keeps_non_lake_patches_zero() {
    let layout = patches(vec![17, 1], vec![0, 3, 5], vec![0, 1, 2, 3, 4]);
    let carbon = layout
        .aggregate_lake_soil_carbon(
            &[
                1.0,
                -1.0,
                5.0,
                10.0,
                SURFACE_MISSING,
                30.0,
                40.0,
                50.0,
                60.0,
                70.0,
            ],
            2,
            &[1.0, 3.0, 2.0, 1.0, 1.0],
            17,
        )
        .unwrap();
    assert_eq!(carbon, [11.0 / 3.0, 0.0, 250.0 / 6.0, 0.0]);
}

#[test]
fn canopy_structure_masks_invalid_cells_and_copies_wmo_source() {
    let layout = FlatPatches::new(
        vec![1, 1, 1],
        vec![0, 3, 4, 5],
        vec![0, 1, 2, 3, 4],
        vec![None, None, Some(0)],
    )
    .unwrap();
    let structure = layout
        .aggregate_canopy_structure(
            &[2.0, 4.0, f64::NAN, 1000.0, 99.0],
            &[3.0, 9.0, -1.0, 1001.0, 99.0],
            &[5.0, 11.0, f64::MIN_POSITIVE, 0.0, 99.0],
            &[1.0, 3.0, 20.0, 1.0, 1.0],
        )
        .unwrap();
    assert_eq!(
        structure.needleleaf_crown_depth_m,
        [3.5, SURFACE_MISSING, 3.5]
    );
    assert_eq!(
        structure.needleleaf_crown_width_m,
        [7.5, SURFACE_MISSING, 7.5]
    );
    assert_eq!(
        structure.broadleaf_crown_width_m,
        [
            (5.0 + 33.0 + 20.0 * f64::MIN_POSITIVE) / 24.0,
            SURFACE_MISSING,
            (5.0 + 33.0 + 20.0 * f64::MIN_POSITIVE) / 24.0
        ]
    );
}

#[test]
fn lulcc_source_fractions_are_area_weighted_and_keep_wmo_consumers_zero() {
    let layout = FlatPatches::new(
        vec![1, 1],
        vec![0, 3, 4],
        vec![0, 1, 2, 3],
        vec![None, Some(0)],
    )
    .unwrap();
    assert_eq!(
        layout
            .aggregate_lulcc_source_fractions(&[0, 1, 2, 1], &[1.0, 3.0, 2.0, 4.0], 2)
            .unwrap(),
        [1.0 / 6.0, 0.0, 3.0 / 6.0, 0.0, 2.0 / 6.0, 0.0]
    );
}

#[test]
fn lulcc_diagnostic_fractions_are_normalized_by_element_area() {
    let layout = FlatPatches::new(
        vec![1, 2, 1],
        vec![0, 1, 2, 3],
        vec![0, 1, 2],
        vec![None, None, Some(0)],
    )
    .unwrap();
    assert_eq!(
        layout
            .aggregate_lulcc_element_source_fractions(
                &[1, 1, 1],
                &[1, 1, 2],
                &[1.0, 3.0, 100.0],
                2,
            )
            .unwrap(),
        [0.0, 0.0, 0.0, 0.25, 0.75, 0.0, 0.0, 0.0, 0.0]
    );
}

#[test]
fn lct_lai_and_sai_are_area_weighted_without_wmo_sharing() {
    let layout = FlatPatches::new(
        vec![1, 1],
        vec![0, 2, 3],
        vec![0, 1, 2],
        vec![None, Some(0)],
    )
    .unwrap();
    assert_eq!(
        layout
            .aggregate_patch_vegetation_index(&[1.0, 4.0, 100.0], &[3.0, 1.0, 1.0])
            .unwrap(),
        [1.75, 100.0]
    );
}

#[test]
fn weighted_surface_means_preserve_original_sum_rounding() {
    // One real Pearl River LAI patch, reused for the identical weighted SUM
    // in forest height, elevation and slope. Original gfortran -O2
    // -fdefault-real-8 gives 3fee690826d1f3ea; separate multiply/add gives ...ec.
    let values = [
        1.0548386573791504,
        0.9548386931419373,
        1.2612903118133545,
        0.8322580456733704,
        1.2612903118133545,
        1.5387096405029297,
        1.229032278060913,
        1.1806451082229614,
        3.070967674255371,
    ];
    let area = [
        0.1997263501383564,
        0.1997269605092288,
        0.00017124049444929067,
        0.1997269605092288,
        0.00017124049444929067,
        0.0001712404944498747,
        0.00017124049444929067,
        0.00017124049444929064,
        0.0007115080808939559,
    ];
    let layout = patches(vec![1], vec![0, values.len()], (0..values.len()).collect());
    let vegetation = layout
        .aggregate_patch_vegetation_index(&values, &area)
        .unwrap();
    let height = layout.aggregate_igbp_forest_height(&values, &area).unwrap();
    let topography = layout
        .aggregate_topography(&area, &values, &[0.0; 9], &values)
        .unwrap();
    assert_eq!(
        [
            vegetation[0].to_bits(),
            height[0].to_bits(),
            topography.elevation[0].to_bits(),
            topography.slope_ratio[0].to_bits(),
        ],
        [0x3fee_6908_26d1_f3ea; 4]
    );
}

#[test]
fn topography_variance_preserves_original_outer_sum_rounding() {
    // Two cells from the real Pearl River topography request. Expected bits
    // come from unchanged Aggregation_Topography expressions compiled with
    // gfortran -O2 -fdefault-real-8. Without outer FMA, std ends in ...3ea7.
    let layout = patches(vec![1], vec![0, 2], vec![0, 1]);
    let output = layout
        .aggregate_topography(
            &[177445.296875, 177429.4375],
            &[99.48611450195313, 80.60832977294922],
            &[27.390289306640625, 15.267455101013184],
            &[0.17822501063346863, 0.11853952705860138],
        )
        .unwrap();
    assert_eq!(output.elevation[0].to_bits(), 0x4056_830c_9942_d6af);
    assert_eq!(output.elevation_std[0].to_bits(), 0x4038_195d_843a_3ea8);
    assert_eq!(output.slope_ratio[0].to_bits(), 0x3fc2_fe3b_e00b_1392);
}

#[test]
fn hyper_albedo_scales_before_median_and_marks_water_and_ice_missing() {
    let layout = FlatPatches::new(
        vec![1, 1, 17, 15],
        vec![0, 4, 4, 5, 6],
        vec![0, 1, 2, 3, 4, 5],
        vec![None, Some(0), None, None],
    )
    .unwrap();
    assert_eq!(
        layout
            .aggregate_soil_hyper_albedo(&[1000.0, 3000.0, 7000.0, 9000.0, 9999.0, 9999.0], 17, 15,)
            .unwrap(),
        [0.5, 0.5, SURFACE_MISSING, SURFACE_MISSING]
    );
}

#[test]
fn topographic_wetness_matches_threshold_fit_and_short_sample_fallback() {
    assert_eq!(derive_topographic_wetness(&[1.0; 24]).unwrap(), None);
    let values = (0..25).map(f64::from).collect::<Vec<_>>();
    let output = derive_topographic_wetness(&values).unwrap().unwrap();
    assert_eq!(output.mean_twi, 12.0);
    assert_eq!(output.fsatmax, 0.52);
    assert_eq!(output.alp_twi, 0.1);
    assert_eq!(output.chi_twi, 0.01);
    assert_eq!(output.mu_twi, 0.0);
    assert_eq!(output.fsatdcf, 0.2);

    let skewed = (0..24)
        .map(f64::from)
        .chain(std::iter::once(100.0))
        .collect::<Vec<_>>();
    let skewed = derive_topographic_wetness(&skewed).unwrap().unwrap();
    assert!(skewed.alp_twi > 0.1);
    assert!(skewed.chi_twi > 0.01);
    assert!(skewed.mu_twi > 0.0);
}

#[test]
fn topographic_wetness_is_layer_major_and_reuses_wmo_sources() {
    let layout =
        FlatPatches::new(vec![1, 1], vec![0, 1, 2], vec![0, 1], vec![None, Some(0)]).unwrap();
    let mut raw = Vec::new();
    for value in 0..25 {
        raw.extend([f64::from(value), 99.0]);
    }
    let output = layout.aggregate_topographic_wetness(&raw, 25).unwrap();
    assert_eq!(output[0].unwrap().mean_twi, 12.0);
    assert_eq!(output[1], output[0]);
    assert!(layout.aggregate_topographic_wetness(&raw, 24).is_err());
}

#[test]
fn simple_topography_factors_mask_missing_values_and_share_wmo_results() {
    let layout = FlatPatches::new(
        vec![1, 1],
        vec![0, 2, 3],
        vec![0, 1, 2],
        vec![None, Some(0)],
    )
    .unwrap();
    let output = layout
        .aggregate_simple_topography_factors(
            &[1.0, -9999.0, 99.0],
            &[10.0, 30.0, 99.0, -9999.0, 40.0, 99.0],
            &[100.0, 300.0, 99.0, -9999.0, -9999.0, 99.0],
            2,
            &[1.0, 3.0, 1.0],
        )
        .unwrap();
    assert_eq!(output.curvature, [1.0, 1.0]);
    assert_eq!(output.slope_by_aspect, [25.0, 25.0, 40.0, 40.0]);
    assert_eq!(
        output.aspect_by_aspect,
        [250.0, 250.0, SURFACE_MISSING, SURFACE_MISSING]
    );
}

#[test]
fn regular_topography_factors_preserve_four_slope_types_and_shadow_curve() {
    let layout = patches(vec![1], vec![0, 2], vec![0, 1]);
    let factors = layout
        .aggregate_regular_topography_factors(
            &[0.1, 0.4],
            &[0.0, std::f64::consts::PI],
            &[0.5, -9999.0],
            &[2.0, 4.0],
            &[0.0; 32],
            &[0.0; 32],
            &[1.0, 3.0],
        )
        .unwrap();
    assert_eq!(factors.sky_view_factor, [0.5]);
    assert_eq!(factors.curvature, [3.5]);
    assert_eq!(factors.area_type, [0.0, 0.25, 0.75, 0.0]);
    assert_eq!(
        factors.aspect_type,
        [0.0, 0.0, 3.0 * std::f64::consts::PI / 4.0, 0.0]
    );
    assert_eq!(factors.slope_type[0], 0.0);
    assert_eq!(factors.slope_type[1], 0.025);
    assert!((factors.slope_type[2] - 0.3).abs() < 1.0e-12);
    assert_eq!(factors.slope_type[3], 0.0);
    assert_eq!(factors.shadow_curve.len(), 16 * 3);
    assert_eq!(factors.shadow_curve[0], 0.0);
    assert!(factors.shadow_curve[1].abs() < 1.0e-12);
    assert!((factors.shadow_curve[2] - (-0.999_f64.ln()).ln()).abs() < 1.0e-12);
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
    let wmo = FlatPatches::new(vec![1, 1], vec![0, 2, 2], vec![0, 1], vec![None, Some(0)]).unwrap();
    assert_eq!(
        wmo.aggregate_igbp_forest_height(&[4.0, 12.0], &[1.0, 3.0])
            .unwrap(),
        vec![10.0, 10.0]
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
