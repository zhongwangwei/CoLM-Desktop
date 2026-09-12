use super::*;

fn soil(value: f64) -> SoilLayerInput {
    SoilLayerInput {
        vf_quartz: value,
        vf_gravels: value + 1.0,
        vf_om: value + 2.0,
        vf_sand: value + 3.0,
        vf_clay: value + 4.0,
        wf_gravels: value + 5.0,
        wf_sand: value + 6.0,
        wf_clay: value + 7.0,
        wf_om: value + 8.0,
        om_density: value + 9.0,
        bulk_density: value + 10.0,
        theta_s: 0.4,
        psi_s_cm: -10.0,
        lambda: 0.2,
        theta_r: 0.05,
        alpha_vgm: 0.02,
        l_vgm: 0.5,
        n_vgm: 1.5,
        k_s_cm_day: 86.4,
        csol: value + 11.0,
        k_solids: value + 12.0,
        tksatu: value + 13.0,
        tksatf: value + 14.0,
        tkdry: value + 15.0,
        ba_alpha: value + 16.0,
        ba_beta: value + 17.0,
    }
}

#[test]
fn lake_layers_match_fortrans_clamps_scales_and_default_branch() {
    let state = derive_lake_layers(&[0.05, 25.0, 2000.0], 10).unwrap();
    assert_eq!(state.depth_m, vec![0.1, 25.0, 50.0]);
    assert!((state.thickness_m[0] - 0.01).abs() < 1e-14);
    assert!((state.thickness_m[3 + 1] - 0.5).abs() < 1e-14);
    assert!((state.thickness_m[9 * 3 + 1] - 5.175).abs() < 1e-14);
    assert_eq!(state.thickness_m[2], 0.1);
    assert_eq!(state.thickness_m[9 * 3 + 2], 10.45);
}

#[test]
fn lake_initializer_rejects_the_unimplemented_comment_only_25_layer_variant() {
    assert!(derive_lake_layers(&[2.0], 25).is_err());
}

#[test]
fn bedrock_preserves_fortran_ocean_exception_and_last_true_interface() {
    let state = derive_bedrock(
        &[200.0, 50.0, 0.1],
        &[0, 1, 1],
        &[0.02],
        &[0.02, 0.08, 0.2, 0.5, 1.0],
    )
    .unwrap();
    assert_eq!(state.depth, vec![200.0, 0.5, 0.02]);
    assert_eq!(state.layer_index, vec![0, 4, 1]);
}

#[test]
fn soil_texture_clamps_fortrans_out_of_range_classes_to_zero() {
    let mut texture = [-1, 0, 8, 12, 13];
    normalize_soil_texture(&mut texture);
    assert_eq!(texture, [0, 0, 8, 12, 0]);
}

#[test]
fn soil_conversion_expands_every_patch_including_natural_soil_type_zero() {
    let source = (1..=8)
        .flat_map(|layer| [soil(layer as f64), soil(100.0 + layer as f64)])
        .collect::<Vec<_>>();
    let state = derive_soil_parameters(&source, &[1, 0], 11, HydraulicModel::Campbell).unwrap();
    assert_eq!(state.layers, 11);
    assert_eq!(state.patches, 2);
    assert_eq!(
        (0..11)
            .map(|layer| state.get(SoilField::VfQuartz, layer, 0))
            .collect::<Vec<_>>(),
        vec![1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 8.0, 8.0]
    );
    assert_eq!(state.get(SoilField::VfQuartz, 4, 1), 104.0);
    assert_eq!(state.get(SoilField::Psi0, 0, 0), -100.0);
    assert_eq!(state.get(SoilField::ThetaR, 0, 0), 0.0);
    assert_eq!(state.get(SoilField::AlphaVgm, 0, 0), MISSING);
    assert_eq!(state.get(SoilField::ScVgm, 0, 0), MISSING);
    assert!((state.get(SoilField::HydraulicConductivity, 0, 0) - 0.01).abs() < 1e-14);
    let expected = (-339.9_f64 / -10.0).powf(-0.2) * 0.4;
    assert!((state.get(SoilField::FieldCapacity, 0, 0) - expected).abs() < 1e-14);
}

#[test]
fn van_genuchten_uses_its_own_field_capacity_and_psi0() {
    let source = vec![soil(1.0); 8];
    let state = derive_soil_parameters(&source, &[1], 10, HydraulicModel::VanGenuchten).unwrap();
    let expected = 0.05 + (0.4 - 0.05) * (1.0 + (0.02_f64 * 339.9).powf(1.5)).powf(1.0 / 1.5 - 1.0);
    assert_eq!(state.get(SoilField::Psi0, 0, 0), -10.0);
    assert_eq!(state.get(SoilField::ThetaR, 0, 0), 0.05);
    assert!((state.get(SoilField::FieldCapacity, 0, 0) - expected).abs() < 1e-14);
    let m = 1.0 - 1.0 / 1.5;
    let sc = (1.0 + (0.02_f64 * 10.0).powf(1.5)).powf(-m);
    let fc = 1.0 - (1.0 - sc.powf(1.0 / m)).powf(m);
    assert!((state.get(SoilField::ScVgm, 0, 0) - sc).abs() < 1e-14);
    assert!((state.get(SoilField::FcVgm, 0, 0) - fc).abs() < 1e-14);
}

#[test]
fn soil_conversion_rejects_a_non_fortran_source_shape() {
    assert!(derive_soil_parameters(&[], &[1], 10, HydraulicModel::Campbell).is_err());
    assert!(
        derive_soil_parameters(&vec![soil(1.0); 8], &[1], 8, HydraulicModel::Campbell).is_err()
    );
}

#[test]
fn spatial_soil_marks_only_ocean_classes_missing() {
    let source = (1..=8)
        .flat_map(|layer| [soil(layer as f64), soil(100.0 + layer as f64)])
        .collect::<Vec<_>>();
    let state =
        derive_spatial_soil_parameters(&source, &[0, 1], &[0, 0], 10, HydraulicModel::Campbell)
            .unwrap();
    assert_eq!(state.get(SoilField::VfQuartz, 0, 0), MISSING);
    assert_eq!(state.get(SoilField::VfQuartz, 0, 1), 101.0);
    assert_eq!(state.get(SoilField::FieldCapacity, 0, 0), MISSING);
    assert_ne!(state.get(SoilField::FieldCapacity, 0, 1), MISSING);
}
