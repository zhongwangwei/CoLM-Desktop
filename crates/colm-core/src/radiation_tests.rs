use super::*;

#[test]
fn canopy_thermal_gap_matches_original_pearl_river_patches() {
    assert_eq!(two_stream_zmu(1.0e-6, 0.5), 1.0 / 0.877);
    assert_eq!(two_stream_zmu(0.5, 1.0e-6), 1.0);
    // Linked original MOD_Albedo.o, -O2 -fdefault-real-8; classes 13 and 9.
    let soil_grid = crate::colm_soil_grid(10).unwrap();
    for (patch_type, class, lai, sai, coszen, water, expected, soil_bits) in [
        (
            1,
            13,
            0.016731545999066972,
            0.053416176787594076,
            0.13061497421041415,
            8.838313932745438,
            0x3fed_d3e7_0416_235f,
            [
                0x3fe728c1913468da,
                0x3feb93cf56fdd804,
                0x3fe63aa459ca40d0,
                0x3fe9780a7f2f49e0,
            ],
        ),
        (
            0,
            9,
            0.8492320693433216,
            0.7621269547891374,
            0.17590374025111977,
            8.955592710048453,
            0x3fc9_76a3_ae9c_1116,
            [
                0x3fa24758892d48ea,
                0x3fca469e4b58594c,
                0x3fc2e60085eaaeb5,
                0x3fd29e032612c954,
            ],
        ),
    ] {
        let output = cold_start_broadband_radiation_with_snow(
            patch_type,
            SoilReflectance {
                saturated_visible: 0.08,
                dry_visible: 0.19,
                saturated_near_infrared: 0.16,
                dry_near_infrared: 0.27,
            },
            water,
            soil_grid.thickness_m[0],
            leaf_optics_from_land_cover(LandCoverScheme::Igbp, class).unwrap(),
            lai,
            sai,
            0.0,
            coszen,
            true,
            false,
            false,
            0.0,
            0.0,
            283.0,
        )
        .unwrap();
        assert_eq!(
            output.thermal_gap_fraction.to_bits(),
            expected,
            "class {class}"
        );
        for (actual, expected) in output.soil_absorption.iter().flatten().zip(soil_bits) {
            assert_eq!(actual.to_bits(), expected, "class {class} soil absorption");
        }
    }
}

#[test]
fn lct_two_stream_matches_all_original_pearl_river_outputs() {
    // Original ebe6de9 MOD_Albedo twostream, -O2 -fdefault-real-8.
    // Stored LCT patches e110_n25:826, e110_n20:1075/1087, e105_n20:1504.
    // Output order: alb(2x2), tran(2x3), ssun(2x2), ssha(2x2), thermk/extkb/extkd.
    for (class, lai, sai, coszen, visible, nir, exact, expected) in [
        (
            8,
            0.21468035864032103,
            0.2457523580306496,
            0.17552658878122934,
            0.08,
            0.16,
            true,
            [
                0x3fafb6b1b44f80bc,
                0x3fb00d38f6a616db,
                0x3fcf40049a90b3f7,
                0x3fc96ee3bffb36c9,
                0x3fa9ceaa6639ca67,
                0x3fe4d980b7ef8e88,
                0x3fd1710ed5535ff1,
                0x3fcb4d5c3d1d86de,
                0x3fe751a8cc35d79f,
                0x3fd1710ed5535ff1,
                0x3fe4190c4d053806,
                0x3fc942a7b013cc6b,
                0x3fd4b66d0d3dfa7d,
                0x3fbc19b5883ac6c9,
                0x3f8a6e5015ea0a45,
                0x3fc1fcb92fb6f2af,
                0x3f98b6290eee9f9d,
                0x3fb4546691464167,
                0x3fe42c2d84bcad5c,
                0x4006967d3db067c9,
                0x3fe7020c49ba5e35,
            ],
        ),
        (
            13,
            0.016731545999066972,
            0.053416176787594076,
            0.13061497421041415,
            0.08,
            0.16,
            true,
            [
                0x3fb34a7963897e4b,
                0x3fb39857a7f22aed,
                0x3fc633328593f6b6,
                0x3fc4ae6715ebc2f3,
                0x3f949eef9937bd59,
                0x3fedf9b4d8fda804,
                0x3fe8875558b201f7,
                0x3faef3d8c1a32d07,
                0x3fee51f41d817054,
                0x3fe8875558b201f7,
                0x3fc9a4b06a708fed,
                0x3fabc2d2e015df49,
                0x3fc0c61de4fcf5a2,
                0x3fa3243e5958b593,
                0x3f430c9ef90d84dd,
                0x3f7e7c43014254b8,
                0x3f4c1e2e461065be,
                0x3f750beae01d04f8,
                0x3fedd3e70416235f,
                0x400e52ec6d0c9440,
                0x3fe7020c49ba5e35,
            ],
        ),
        (
            9,
            0.8492320693433216,
            0.7621269547891374,
            0.17590374025111977,
            0.08,
            0.16,
            true,
            [
                0x3fad22eaaa1fd6e2,
                0x3faa19fc96a9fa2e,
                0x3fd2d28469b30028,
                0x3fcf9f65202d9abc,
                0x3f9cce501b58fcd8,
                0x3fcc8f8aaaf0ba1b,
                0x3f85dc5b28db4354,
                0x3fc521c10540897e,
                0x3fd629d2fc904503,
                0x3f85dc5b28db4354,
                0x3feb44df8b3dfec6,
                0x3fce4039b61de2bd,
                0x3fdac898706053c0,
                0x3fc1433d2213ee75,
                0x3fac47c418d2f3d0,
                0x3fe03caa3637d15b,
                0x3fc1e3c5c5eea97c,
                0x3fd4f0abb8cc7214,
                0x3fc976a3ae9c1116,
                0x40068a23dec29950,
                0x3fe7020c49ba5e35,
            ],
        ),
        (
            16,
            0.04352080352942207,
            0.024613835967001594,
            0.12618927879439434,
            0.1,
            0.2,
            true,
            [
                0x3fb70906fd913d1b,
                0x3fb808c05516365c,
                0x3fcc7521c7f0e341,
                0x3fc9fae74bcbae10,
                0x3f94776d02fbe073,
                0x3fee0ae2fff0a6c1,
                0x3fe87e8b86066e2f,
                0x3fb3e39479613314,
                0x3fee8f9fff9ab9f2,
                0x3fe87e8b86066e2f,
                0x3fc9e9f12772c11c,
                0x3fab849c30b5f786,
                0x3fba3301f42ee069,
                0x3f9db0578a350ce1,
                0x3f46259a24573ab4,
                0x3f7e68f2c96ea078,
                0x3f4afcfa229deca4,
                0x3f706e8553054314,
                0x3fede3528647a71c,
                0x400f6261d9d7f2e7,
                0x3fe7020c49ba5e35,
            ],
        ),
        (
            13,
            0.016731545999066972,
            0.053416176787594076,
            0.5347439652952921,
            0.08,
            0.16,
            false,
            [
                0x3fb3368aaf5c16b9,
                0x3fb39857a7f22aed,
                0x3fc42894ecf964db,
                0x3fc4ae6715ebc2f3,
                0x3f77f47b853008fd,
                0x3fedf9b4d8fda804,
                0x3fedf853847379b8,
                0x3f9312a1f4b977d5,
                0x3fee51f41d817054,
                0x3fedf853847379b8,
                0x3fad95cad85f6f87,
                0x3fae91da4d5b84d7,
                0x3fa4748053bca2b8,
                0x3fa514a426fcf604,
                0x3f233f1582e76a50,
                0x3f60080f2e2a5090,
                0x3f2a66c14794e8e5,
                0x3f5622f1cbec05c0,
                0x3fedd3e70416235f,
                0x3fede5b0ebd02112,
                0x3fe7020c49ba5e35,
            ],
        ),
        (
            13,
            0.016731545999066972,
            0.053416176787594076,
            0.5347438652952922,
            0.08,
            0.16,
            false,
            [
                0x3fb33580f1229e6b,
                0x3fb39857a7f22aed,
                0x3fc42894ef233750,
                0x3fc4ae6715ebc2f3,
                0x3f7803143a4cd47f,
                0x3fedf9b4d8fda804,
                0x3fedf8537e5cb9a8,
                0x3f9312a225cdc664,
                0x3fee51f41d817054,
                0x3fedf8537e5cb9a8,
                0x3fad95c514fd522d,
                0x3fae91da4a55f2e6,
                0x3fa4748087fa3c21,
                0x3fa514a424e3cabd,
                0x3f23aaf64dc54baa,
                0x3f60080f5e836fa0,
                0x3f2a66c19b49d619,
                0x3f5622f20f116ea0,
                0x3fedd3e70416235f,
                0x3fede5b1487e3375,
                0x3fe7020c49ba5e35,
            ],
        ),
    ] {
        let state = two_stream(
            leaf_optics_from_land_cover(LandCoverScheme::Igbp, class).unwrap(),
            lai,
            sai,
            0.0,
            coszen,
            [[visible; 2], [nir; 2]],
            false,
            false,
        )
        .unwrap();
        let actual = state
            .albedo
            .iter()
            .flatten()
            .chain(state.transmission.iter().flatten())
            .chain(state.sunlit_absorption.iter().flatten())
            .chain(state.shaded_absorption.iter().flatten())
            .copied()
            .chain([
                state.thermal_gap_fraction,
                state.direct_extinction,
                state.diffuse_extinction,
            ]);
        for (index, (actual, expected)) in actual.zip(expected).enumerate() {
            if exact {
                assert_eq!(actual.to_bits(), expected, "class {class}, output {index}");
            } else {
                let expected = f64::from_bits(expected);
                assert!((actual - expected).abs() <= 1.0e-12 + 1.0e-12 * expected.abs(),
                    "singular-boundary coszen {coszen}, output {index}: {actual:.17e} != {expected:.17e}");
            }
        }
    }
}

#[test]
fn leaf_optics_are_the_native_land_cover_constants() {
    assert_eq!(
        leaf_optics_from_land_cover(LandCoverScheme::Igbp, 10).unwrap(),
        LeafOptics {
            chil: -0.3,
            reflectance: [[0.105, 0.360], [0.580, 0.580]],
            transmittance: [[0.070, 0.220], [0.250, 0.380]],
        }
    );
    assert_eq!(
        leaf_optics_from_land_cover(LandCoverScheme::Usgs, 12).unwrap(),
        LeafOptics {
            chil: 0.01,
            reflectance: [[0.070, 0.160], [0.350, 0.390]],
            transmittance: [[0.050, 0.001], [0.100, 0.001]],
        }
    );
    assert!(leaf_optics_from_land_cover(LandCoverScheme::Igbp, 0).is_err());
    assert!(leaf_optics_from_land_cover(LandCoverScheme::Usgs, 25).is_err());
}

#[test]
fn cold_start_radiation_matches_the_upstream_cn_cng_broadband_restart() {
    let output = cold_start_broadband_radiation(
        0,
        SoilReflectance {
            saturated_visible: 0.14,
            dry_visible: 0.25,
            saturated_near_infrared: 0.28,
            dry_near_infrared: 0.39,
        },
        12.55,
        0.025,
        LeafOptics {
            chil: -0.3,
            reflectance: [[0.105, 0.360], [0.580, 0.580]],
            transmittance: [[0.070, 0.220], [0.250, 0.380]],
        },
        0.200_000_002_980_232,
        0.449_999_988_079_071,
        0.0,
        0.001,
        true,
        false,
        true,
    )
    .unwrap();
    assert_close(output.thermal_gap_fraction, 0.546_974_995_631_841);
    assert_close(output.direct_extinction, 659.919_009_2);
    assert_close(output.diffuse_extinction, 0.719);
    assert_matrix_close(
        output.albedo,
        [
            [0.260_189_838_882_736, 0.148_271_866_271_85],
            [0.631_103_931_100_346, 0.376_045_493_682_729],
        ],
    );
    assert_matrix_close(
        output.sunlit_absorption,
        [
            [0.544_815_630_561_079, 0.000_878_704_147_059_4],
            [0.080_071_308_935_473_3, 0.000_154_740_689_721],
        ],
    );
    assert_matrix_close(
        output.shaded_absorption,
        [
            [0.069_258_780_211_443_9, 0.302_210_617_164_036],
            [0.026_493_376_605_311_3, 0.057_220_696_582_536_3],
        ],
    );
    assert_matrix_close(
        output.soil_absorption,
        [
            [0.125_735_750_344_742, 0.548_638_812_417_054],
            [0.262_331_383_358_87, 0.566_579_069_045_013],
        ],
    );
    assert_eq!(output.snow_absorption, [[0.0; RADIATION_TYPES]; BANDS]);
}

#[test]
fn pft_radiation_matches_the_upstream_cn_cng_pft_restart() {
    let output = cold_start_pft_broadband_radiation_with_snow(
        0,
        SoilReflectance {
            saturated_visible: 0.14,
            dry_visible: 0.25,
            saturated_near_infrared: 0.28,
            dry_near_infrared: 0.39,
        },
        0.0,
        0.017_524_084_877_847_7,
        LeafOptics {
            chil: -0.3,
            reflectance: [[0.11, 0.31], [0.35, 0.53]],
            transmittance: [[0.05, 0.12], [0.34, 0.25]],
        },
        0.200_000_002_980_232,
        0.449_999_988_079_071,
        0.0,
        0.001,
        true,
        0.0,
        0.0,
        273.16,
    )
    .unwrap();
    assert_close(output.thermal_gap_fraction, 0.546_974_995_631_841);
    assert_close(output.direct_extinction, 659.919_009_2);
    assert_close(output.diffuse_extinction, 0.719);
    assert_matrix_close(
        output.sunlit_absorption,
        [
            [0.653_260_962_413_402, 0.000_920_882_050_068_061],
            [0.247_869_169_434_298, 0.000_414_032_765_122_806],
        ],
    );
    assert_matrix_close(
        output.shaded_absorption,
        [
            [0.063_710_598_137_456_6, 0.332_772_574_677_887],
            [0.065_079_552_237_013_9, 0.158_616_791_744_303],
        ],
    );
}

#[test]
fn natural_non_lct_path_retains_ground_albedo() {
    let output = cold_start_broadband_radiation(
        0,
        SoilReflectance {
            saturated_visible: 0.2,
            dry_visible: 0.3,
            saturated_near_infrared: 0.4,
            dry_near_infrared: 0.5,
        },
        10.0,
        0.1,
        LeafOptics {
            chil: 0.0,
            reflectance: [[0.1, 0.1], [0.1, 0.1]],
            transmittance: [[0.1, 0.1], [0.1, 0.1]],
        },
        1.0,
        0.0,
        0.0,
        0.5,
        false,
        false,
        true,
    )
    .unwrap();
    assert_matrix_close(output.albedo, [[0.27, 0.27], [0.47, 0.47]]);
    assert_eq!(output.thermal_gap_fraction, 0.0);
    assert_eq!(output.direct_extinction, 1.0);
}

#[test]
fn shared_ground_albedo_retains_the_soil_and_snow_components() {
    let ground = cold_start_ground_albedo(
        0,
        SoilReflectance {
            saturated_visible: 0.2,
            dry_visible: 0.3,
            saturated_near_infrared: 0.4,
            dry_near_infrared: 0.5,
        },
        10.0,
        0.1,
        0.5,
        0.0,
        0.0,
        273.16,
    )
    .unwrap();
    assert_matrix_close(ground.soil, [[0.27, 0.27], [0.47, 0.47]]);
    assert_matrix_close(ground.ground, ground.soil);
    assert_eq!(ground.snow_age, 0.0);
}

#[test]
fn frozen_inland_water_uses_the_upstream_ice_albedo() {
    let ground = cold_start_ground_albedo(
        4,
        SoilReflectance {
            saturated_visible: 0.14,
            dry_visible: 0.25,
            saturated_near_infrared: 0.28,
            dry_near_infrared: 0.39,
        },
        0.0,
        0.1,
        0.5,
        0.0,
        0.0,
        272.0,
    )
    .unwrap();
    assert_eq!(ground.soil, [[0.6; 2], [0.4; 2]]);
    assert_eq!(ground.ground, ground.soil);
}

#[test]
fn initialized_snow_uses_the_non_snicar_source_albedo_and_age() {
    let output = cold_start_broadband_radiation_with_snow(
        0,
        SoilReflectance {
            saturated_visible: 0.14,
            dry_visible: 0.25,
            saturated_near_infrared: 0.28,
            dry_near_infrared: 0.39,
        },
        10.0,
        0.1,
        LeafOptics {
            chil: 0.0,
            reflectance: [[0.1, 0.1], [0.1, 0.1]],
            transmittance: [[0.1, 0.1], [0.1, 0.1]],
        },
        0.0,
        0.0,
        0.0,
        0.5,
        true,
        false,
        true,
        0.1,
        1.0,
        270.0,
    )
    .unwrap();
    assert_close(output.snow_age, 0.002_204_192_232_638_98);
    assert_matrix_close(
        output.albedo,
        [
            [0.849_626_111_442_705, 0.849_626_111_442_705],
            [0.649_285_213_052_231, 0.649_285_213_052_231],
        ],
    );
    assert_matrix_close(
        output.snow_absorption,
        [
            [0.150_373_888_557_295, 0.150_373_888_557_295],
            [0.350_714_786_947_769, 0.350_714_786_947_769],
        ],
    );
}

fn assert_matrix_close(
    actual: [[f64; RADIATION_TYPES]; BANDS],
    expected: [[f64; RADIATION_TYPES]; BANDS],
) {
    for band in 0..BANDS {
        for radiation_type in 0..RADIATION_TYPES {
            assert_close(actual[band][radiation_type], expected[band][radiation_type]);
        }
    }
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = 1.0e-10 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}
