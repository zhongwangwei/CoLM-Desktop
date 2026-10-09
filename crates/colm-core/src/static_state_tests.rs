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
fn soil_grid_matches_fortran_global_initialization() {
    let grid = colm_soil_grid(10).unwrap();
    assert_eq!(grid.interface_depth_m[0], 0.0);
    // Pristine MOD_Vars_Global::Init_GlobalVars, gfortran -O2 -fdefault-real-8.
    // Keep 0.025 * (exp(...) - 1), not the algebraically expanded expression.
    assert_eq!(grid.node_depth_m[0].to_bits(), 0x3f7d_158e_4e5c_d68d);
    assert_eq!(grid.thickness_m[0].to_bits(), 0x3f91_eee1_50d8_2f92);
    assert_eq!(grid.interface_depth_m[10].to_bits(), 0x400b_76f9_788b_6317);
    assert!((grid.interface_depth_m[1] - grid.thickness_m[0]).abs() < 1.0e-14);
    assert!((grid.interface_depth_m[10] - grid.thickness_m.iter().sum::<f64>()).abs() < 1.0e-14);
    assert!(colm_soil_grid(1).is_err());
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
    // 存进状态的 `alpha` 换成 1/mm（CoLM-SYSU/CoLM#507），`wfc` 仍用原值配 339.9 cm。
    assert_eq!(state.get(SoilField::AlphaVgm, 0, 0), 0.02 * 0.1);
    let sc = (1.0 + (0.02_f64 * 0.1 * 10.0).powf(1.5)).powf(-m);
    let fc = 1.0 - (1.0 - sc.powf(1.0 / m)).powf(m);
    assert!((state.get(SoilField::ScVgm, 0, 0) - sc).abs() < 1e-14);
    assert!((state.get(SoilField::FcVgm, 0, 0) - fc).abs() < 1e-14);
}

#[test]
fn van_genuchten_field_capacity_matches_original_single_rounding() {
    colm_numeric::skip_unless_fused!();
    // Original Pearl River constant restart e100_n20, patch 1, layer 7;
    // MOD_SoilParametersReadin built with -O2 -fdefault-real-8.
    let input = SoilLayerInput {
        theta_s: f64::from_bits(0x3fdc_4df2_9182_e905),
        theta_r: f64::from_bits(0x3fc3_45e8_36d2_d9a2),
        alpha_vgm: f64::from_bits(0x3f9e_ef2a_c0fa_3908),
        n_vgm: f64::from_bits(0x3ff2_3271_dfb7_528f),
        ..soil(1.0)
    };
    let state =
        derive_soil_parameters(&[input; 8], &[0], 10, HydraulicModel::VanGenuchten).unwrap();
    assert_eq!(
        state.get(SoilField::FieldCapacity, 0, 0).to_bits(),
        0x3fd7_1556_aa77_a6c1
    );
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

/// 两个 patch、CoLM 的十层，每个 `(field, layer, patch)` 都取一个互不相同的值。
///
/// 直接用 `from_fields` 而不是 `derive_soil_parameters`：这里要考的正是
/// 「装配层给出的层主序缓冲被按 `(layer, patch)` 正确取出」，派生公式会掩盖这一点。
fn two_patch_soil() -> SoilState {
    const LAYERS: usize = 10;
    const PATCHES: usize = 2;
    let mut values: [Vec<f64>; SoilField::COUNT] = std::array::from_fn(|_| vec![0.0; 20]);
    for field in SoilField::ALL {
        for layer in 0..LAYERS {
            for patch in 0..PATCHES {
                values[field as usize][layer * PATCHES + patch] =
                    field as usize as f64 * 1000.0 + layer as f64 * 10.0 + patch as f64;
            }
        }
    }
    SoilState::from_fields(LAYERS, PATCHES, values).unwrap()
}

#[test]
fn from_fields_and_get_agree_on_the_layer_major_index() {
    let soil = two_patch_soil();
    for field in SoilField::ALL {
        for layer in 0..soil.layers {
            for patch in 0..soil.patches {
                assert_eq!(
                    soil.get(field, layer, patch),
                    field as usize as f64 * 1000.0 + layer as f64 * 10.0 + patch as f64,
                    "{field:?} layer {layer} patch {patch}"
                );
            }
        }
        // 同一层里两个 patch 的值必须不同，否则 patch 索引写反也看不出来。
        assert_ne!(soil.get(field, 0, 0), soil.get(field, 0, 1));
    }
}

#[test]
fn from_fields_rejects_a_buffer_of_the_wrong_length() {
    let mut values: [Vec<f64>; SoilField::COUNT] = std::array::from_fn(|_| vec![0.0; 4]);
    assert!(SoilState::from_fields(2, 2, values.clone()).is_ok());
    values[SoilField::VfQuartz as usize] = vec![0.0; 3];
    let error = SoilState::from_fields(2, 2, values).unwrap_err();
    assert!(format!("{error:#}").contains("VfQuartz"), "{error:#}");
}

#[test]
fn from_fields_rejects_an_empty_column() {
    let values: [Vec<f64>; SoilField::COUNT] = std::array::from_fn(|_| Vec::new());
    assert!(SoilState::from_fields(0, 1, values).is_err());
}

#[test]
fn hydraulic_models_follow_the_selected_relation_and_the_patch() {
    let soil = two_patch_soil();
    let campbell = soil_hydraulic_models(&soil, 0, HydraulicModel::Campbell).unwrap();
    assert_eq!(campbell.len(), soil.layers);
    for (layer, model) in campbell.iter().enumerate() {
        assert_eq!(
            *model,
            crate::SoilHydraulicModel::Campbell {
                bsw: soil.get(SoilField::Bsw, layer, 0),
            }
        );
    }
    // patch 参数不能用常量 0：另一个 patch 必须读出它自己的那一列。
    let other = soil_hydraulic_models(&soil, 1, HydraulicModel::Campbell).unwrap();
    match (campbell[0], other[0]) {
        (
            crate::SoilHydraulicModel::Campbell { bsw: first },
            crate::SoilHydraulicModel::Campbell { bsw: second },
        ) => assert_ne!(first, second),
        other => panic!("expected two Campbell models, got {other:?}"),
    }
}

#[test]
fn hydraulic_models_read_the_van_genuchten_arrays() {
    let soil = two_patch_soil();
    let models = soil_hydraulic_models(&soil, 1, HydraulicModel::VanGenuchten).unwrap();
    for (layer, model) in models.iter().enumerate() {
        assert_eq!(
            *model,
            crate::SoilHydraulicModel::VanGenuchten {
                alpha_vgm: soil.get(SoilField::AlphaVgm, layer, 1),
                n_vgm: soil.get(SoilField::NVgm, layer, 1),
                l_vgm: soil.get(SoilField::LVgm, layer, 1),
                sc_vgm: soil.get(SoilField::ScVgm, layer, 1),
                fc_vgm: soil.get(SoilField::FcVgm, layer, 1),
            }
        );
    }
}

#[test]
fn hydraulic_models_reject_a_patch_outside_the_state() {
    let soil = two_patch_soil();
    assert!(soil_hydraulic_models(&soil, 2, HydraulicModel::Campbell).is_err());
}
