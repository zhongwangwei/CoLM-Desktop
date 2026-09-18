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
fn pft_canopy_structure_uses_pft_weights_and_patch_fallbacks() {
    let layout = patches();
    let input = PftFractionInput {
        pft_offsets: &[0, 2, 3, 4],
        pft_classes: &[0, 1, 0, 16],
        patch_kind: &[
            PftPatchKind::Natural,
            PftPatchKind::Natural,
            PftPatchKind::Crop,
        ],
        raw_class_count: 2,
        raw_percent: &[100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0],
        land_area: &[1.0; 5],
        crop_excluded_class: None,
    };
    let patch = CanopyStructure {
        needleleaf_crown_depth_m: vec![10.0, 10.0, 30.0],
        needleleaf_crown_width_m: vec![11.0, 11.0, 31.0],
        broadleaf_crown_width_m: vec![12.0, 12.0, 32.0],
    };
    let raw = CanopyStructure {
        needleleaf_crown_depth_m: vec![2.0, 6.0, 9.0, 20.0, 40.0],
        needleleaf_crown_width_m: vec![3.0, 7.0, 9.0, 21.0, 41.0],
        broadleaf_crown_width_m: vec![4.0, 8.0, 9.0, 22.0, 42.0],
    };
    let result = aggregate_pft_canopy_structure(&layout, input, &patch, &raw).unwrap();
    assert_eq!(result.needleleaf_crown_depth_m, [2.0, 6.0, 10.0, 30.0]);
    assert_eq!(result.needleleaf_crown_width_m, [3.0, 7.0, 11.0, 31.0]);
    assert_eq!(result.broadleaf_crown_width_m, [4.0, 8.0, 12.0, 32.0]);
}

#[test]
fn pft_ordered_reductions_match_original_compiled_sums() {
    // Original MOD_LandPFT, Aggregation_LAI and Aggregation_ForestHeight expressions.
    let rows = include_str!("../tests/fixtures/pft_ordered_reductions.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            line.split_whitespace()
                .map(|s| s.parse::<f64>().unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 9);
    assert!(rows.iter().all(|row| row.len() == 50));
    let cells = (0..rows.len()).collect::<Vec<_>>();
    let areas = rows.iter().map(|row| row[0]).collect::<Vec<_>>();
    let heights = rows.iter().map(|row| row[1]).collect::<Vec<_>>();
    let field = |start: usize| {
        (0..16)
            .flat_map(|class| rows.iter().map(move |row| row[start + class]))
            .collect::<Vec<_>>()
    };
    let percent = field(2);
    let lai = field(18);
    let sai = field(34);
    let classes = [0, 1, 5, 7, 9, 10, 13];
    let layout = FlatPatches::new(vec![1], vec![0, 9], cells.clone(), vec![None]).unwrap();
    let share = normalized_patch_pft_fractions(&cells, 0, 16, 16, &percent, &areas).unwrap();
    let fraction_input = PftFractionInput {
        pft_offsets: &[0, 7],
        pft_classes: &classes,
        patch_kind: &[PftPatchKind::Natural],
        raw_class_count: 16,
        raw_percent: &percent,
        land_area: &areas,
        crop_excluded_class: None,
    };
    let fractions = aggregate_pft_fractions(&layout, fraction_input).unwrap();
    let htop = aggregate_pft_height(&layout, fraction_input, &heights).unwrap();
    let index_input = PftIndexInput {
        pft_offsets: &[0, 7],
        pft_classes: &classes,
        patch_kind: &[PftPatchKind::Natural],
        raw_class_count: 16,
        raw_percent: &percent,
        raw_index: &lai,
        land_area: &areas,
    };
    let lai = aggregate_pft_index(&layout, index_input).unwrap();
    let sai = aggregate_pft_index(
        &layout,
        PftIndexInput {
            raw_index: &sai,
            ..index_input
        },
    )
    .unwrap();
    let expected = [
        [
            0x3c523464cea15b10,
            0x403c000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x3c523464ce633bca,
        ],
        [
            0x3fdc0783951fc4a4,
            0x403be2103953b650,
            0x3ffada5bc95668bd,
            0x3ff0228b17eb92f4,
            0x3fdc078395d40200,
        ],
        [
            0x3fb110f630731fe0,
            0x403d000000000000,
            0x3ffbc140e0000000,
            0x3fef97a100000000,
            0x3fb110f633a2adf0,
        ],
        [
            0x3fbfc5c72853b847,
            0x403bb1120e7f1bb9,
            0x3fea5992c2593da7,
            0x3ff053bc3b2697c5,
            0x3fbfc5c728789685,
        ],
        [
            0x3fc40b9fe6c171ab,
            0x403ba0c997f44aac,
            0x3ff5f72461264670,
            0x3ff0ac59d220b3a5,
            0x3fc40b9fe4b0001c,
        ],
        [
            0x3fc05472b8d45b55,
            0x403b6ed74f34e782,
            0x3fe635ac0060aaaf,
            0x3ff164296c5bf1f9,
            0x3fc05472b63ad0a5,
        ],
        [
            0x3fb64b0f138e7b49,
            0x403c7c4daeb7de64,
            0x3fe4e4526a573d67,
            0x3ff0ab3127ef9c25,
            0x3fb64b0f16bf120c,
        ],
    ];
    for (pft, (&class, expected)) in classes.iter().zip(expected).enumerate() {
        for (field, (actual, expected)) in [
            share[class],
            htop[pft],
            lai.pft_index[pft],
            sai.pft_index[pft],
            fractions[pft],
        ]
        .into_iter()
        .zip(expected)
        .enumerate()
        {
            assert_eq!(actual.to_bits(), expected, "class {class}, field {field}");
        }
    }
    for (actual, expected) in [
        (lai.patch_index[0], 0x3ff5031fb4acd60f),
        (sai.patch_index[0], 0x3ff06d610ac7fb78),
        (
            patch_area_weighted_height(&cells, 0, &areas, &heights).unwrap(),
            0x403be38ddf4b6920,
        ),
    ] {
        assert_eq!(actual.to_bits(), expected);
    }
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
fn pft_topology_shares_differ_from_mean_per_cell_surface_fractions() {
    let patches = FlatPatches::new(vec![1], vec![0, 2], vec![0, 1], vec![None]).unwrap();
    let land = crate::topology::FlatLandPatches {
        element_ids: vec![1],
        pixel_start: vec![1],
        pixel_end: vec![2],
        set_type: vec![1],
        element_index: vec![1],
    };
    let raw = [20.0, 0.0, 0.0, 80.0];
    let area = [1.0, 1.0];
    let pft = build_pft_topology(&land, &patches, 2, 2, &raw, &area).unwrap();
    assert_eq!(pft.pctshared, [0.2, 0.8]);
    let surface = aggregate_pft_fractions(
        &patches,
        PftFractionInput {
            pft_offsets: &pft.patch_offsets,
            pft_classes: &pft.pft_classes,
            patch_kind: &pft.patch_kind,
            raw_class_count: 2,
            raw_percent: &raw,
            land_area: &area,
            crop_excluded_class: None,
        },
    )
    .unwrap();
    assert_eq!(surface, [0.5, 0.5]);
    // MOD_LandPFT falls back before selecting classes when total raw cover <= 0.
    let missing =
        build_pft_topology(&land, &patches, 2, 2, &[20.0, 0.0, -80.0, 0.0], &area).unwrap();
    assert_eq!(missing.pft_classes, [0]);
    assert_eq!(missing.pctshared, [1.0]);
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
fn landpft_bare_fallback_uses_output_pft_denominator_not_extra_raw_classes() {
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

    let topology = build_pft_topology(&land_patches, &layout, 16, 15, &raw, &[1.0]).unwrap();

    assert_eq!(topology.pft_classes, vec![0]);
    assert_eq!(topology.land_pfts.set_type, vec![0]);
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

    let mesh =
        crate::topology::FlatMesh::new(vec![8, 9], vec![0, 2, 3], vec![1, 2, 3], vec![1, 1, 1])
            .unwrap();
    let mut elements = mesh.land_elements();
    let crop_wmo = crop.clone().with_wmo_patches(&mesh, &mut elements).unwrap();
    assert_eq!(crop_wmo.land_patches.set_type, [1, 12, 12, 1, 17]);
    for (actual, expected) in crop_wmo
        .pctshared
        .iter()
        .zip([0.5, 0.3125, 0.1875, 0.5, 1.0])
    {
        assert!((actual - expected).abs() < 1.0e-12);
    }
    assert_eq!(crop_wmo.crop_class, [None, Some(1), Some(2), None, None]);
    assert_eq!(crop_wmo.layout.wmo_source_for(3), Some(0));

    let mut raw_pft = vec![0.0; 16 * 3];
    raw_pft[..3].copy_from_slice(&[50.0, 50.0, 0.0]);
    let wmo_pfts = build_crop_pft_topology(
        &crop_wmo.land_patches,
        &crop_wmo.layout,
        &crop_wmo.crop_class,
        16,
        15,
        &raw_pft,
        &[1.0, 3.0, 2.0],
    )
    .unwrap();
    assert_eq!(wmo_pfts.patch_offsets, [0, 1, 2, 3, 4, 4]);
    assert_eq!(wmo_pfts.pft_classes, [0, 15, 16, 0]);
    assert_eq!(
        wmo_pfts.patch_kind,
        [
            PftPatchKind::Natural,
            PftPatchKind::Crop,
            PftPatchKind::Crop,
            PftPatchKind::Natural,
            PftPatchKind::Other,
        ]
    );
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
    for (actual, expected) in crop_pft_pctshared(&pfts, &crop.pctshared)
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

fn wmo_land_patches(
    set_type: Vec<i32>,
    pixel_start: Vec<usize>,
    pixel_end: Vec<usize>,
) -> crate::topology::FlatLandPatches {
    let len = set_type.len();
    crate::topology::FlatLandPatches {
        element_ids: vec![8; len],
        pixel_start,
        pixel_end,
        set_type,
        element_index: vec![1; len],
    }
}

#[test]
fn wmo_pft_topology_chooses_largest_source_grass_with_first_tie() {
    let layout =
        FlatPatches::new(vec![1, 1], vec![0, 2, 2], vec![0, 1], vec![None, Some(0)]).unwrap();
    let land_patches = wmo_land_patches(vec![1, 1], vec![1, 0], vec![2, 0]);
    let mut raw = vec![0.0; 15 * 2];
    raw[12 * 2] = 50.0;
    raw[12 * 2 + 1] = 50.0;
    raw[13 * 2] = 50.0;
    raw[13 * 2 + 1] = 50.0;

    let topology = build_pft_topology(&land_patches, &layout, 15, 15, &raw, &[1.0, 1.0]).unwrap();

    assert_eq!(topology.patch_offsets, [0, 2, 3]);
    assert_eq!(topology.pft_classes, [12, 13, 12]);
    assert_eq!(topology.land_pfts.pixel_start, [1, 1, 0]);
    assert_eq!(topology.land_pfts.pixel_end, [2, 2, 0]);
}

#[test]
fn wmo_pft_topology_falls_back_to_bare_when_source_has_no_grass() {
    let layout = FlatPatches::new(vec![1, 1], vec![0, 1, 1], vec![0], vec![None, Some(0)]).unwrap();
    let land_patches = wmo_land_patches(vec![1, 1], vec![1, 0], vec![1, 0]);
    let mut raw = vec![0.0; 15];
    raw[1] = 100.0;

    let topology = build_pft_topology(&land_patches, &layout, 15, 15, &raw, &[1.0]).unwrap();

    assert_eq!(topology.patch_offsets, [0, 1, 2]);
    assert_eq!(topology.pft_classes, [1, 0]);
    assert_eq!(topology.land_pfts.pixel_start, [1, 0]);
    assert_eq!(topology.land_pfts.pixel_end, [1, 0]);
}

#[test]
fn wmo_pft_height_uses_source_patch_height_not_source_pft_height() {
    let layout =
        FlatPatches::new(vec![1, 1], vec![0, 2, 2], vec![0, 1], vec![None, Some(0)]).unwrap();
    let land_patches = wmo_land_patches(vec![1, 1], vec![1, 0], vec![2, 0]);
    let mut raw = vec![0.0; 15 * 2];
    raw[12 * 2] = 100.0;
    raw[13 * 2 + 1] = 100.0;
    let topology = build_pft_topology(&land_patches, &layout, 15, 15, &raw, &[1.0, 3.0]).unwrap();

    let height = aggregate_pft_height(
        &layout,
        PftFractionInput {
            pft_offsets: &topology.patch_offsets,
            pft_classes: &topology.pft_classes,
            patch_kind: &topology.patch_kind,
            raw_class_count: 15,
            raw_percent: &raw,
            land_area: &[1.0, 3.0],
            crop_excluded_class: None,
        },
        &[10.0, 30.0],
    )
    .unwrap();

    assert_eq!(topology.pft_classes, [12, 13, 13]);
    assert_eq!(height, [10.0, 30.0, 25.0]);
}

#[test]
fn wmo_pft_lai_sai_copies_matching_source_grass_or_zero_for_bare() {
    let layout =
        FlatPatches::new(vec![1, 1], vec![0, 2, 2], vec![0, 1], vec![None, Some(0)]).unwrap();
    let land_patches = wmo_land_patches(vec![1, 1], vec![1, 0], vec![2, 0]);
    let mut raw = vec![0.0; 15 * 2];
    let mut index = vec![0.0; 15 * 2];
    raw[12 * 2] = 100.0;
    raw[13 * 2 + 1] = 100.0;
    index[12 * 2] = 4.0;
    index[13 * 2 + 1] = 9.0;
    let topology = build_pft_topology(&land_patches, &layout, 15, 15, &raw, &[1.0, 3.0]).unwrap();

    let state = aggregate_pft_index(
        &layout,
        PftIndexInput {
            pft_offsets: &topology.patch_offsets,
            pft_classes: &topology.pft_classes,
            patch_kind: &topology.patch_kind,
            raw_class_count: 15,
            raw_percent: &raw,
            raw_index: &index,
            land_area: &[1.0, 3.0],
        },
    )
    .unwrap();
    assert_eq!(state.pft_index, [4.0, 9.0, 9.0]);
    assert_eq!(state.patch_index, [31.0 / 4.0, 9.0]);

    let mut bare_raw = vec![0.0; 15];
    let mut bare_index = vec![0.0; 15];
    bare_raw[1] = 100.0;
    bare_index[1] = 7.0;
    let bare_layout =
        FlatPatches::new(vec![1, 1], vec![0, 1, 1], vec![0], vec![None, Some(0)]).unwrap();
    let bare_land_patches = wmo_land_patches(vec![1, 1], vec![1, 0], vec![1, 0]);
    let bare_topology =
        build_pft_topology(&bare_land_patches, &bare_layout, 15, 15, &bare_raw, &[1.0]).unwrap();
    let bare_state = aggregate_pft_index(
        &bare_layout,
        PftIndexInput {
            pft_offsets: &bare_topology.patch_offsets,
            pft_classes: &bare_topology.pft_classes,
            patch_kind: &bare_topology.patch_kind,
            raw_class_count: 15,
            raw_percent: &bare_raw,
            raw_index: &bare_index,
            land_area: &[1.0],
        },
    )
    .unwrap();
    assert_eq!(bare_topology.pft_classes, [1, 0]);
    assert_eq!(bare_state.pft_index, [7.0, 0.0]);
    assert_eq!(bare_state.patch_index, [7.0, 0.0]);
}
