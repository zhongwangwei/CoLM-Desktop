use super::*;

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
