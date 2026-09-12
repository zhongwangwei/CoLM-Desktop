use super::*;

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2.0e-12 * expected.abs().max(1.0),
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn lake_adjustment_matches_mod_lake_overlap_and_phase_balance() {
    // Standalone gfortran reference from MOD_Lake:adjust_lake_layer.
    let mut column = LakeColumn {
        thickness_m: vec![0.2, 2.0, 4.0, 6.0, 8.0, 10.0, 14.0, 14.0, 20.9, 20.9],
        temperature_k: vec![
            274.0, 273.0, 272.0, 271.0, 270.0, 269.0, 268.0, 267.0, 266.0, 265.0,
        ],
        ice_fraction: vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9],
    };
    adjust_lake_layers(&mut column).unwrap();
    for (actual, expected) in column
        .thickness_m
        .iter()
        .zip([0.1, 2.0, 4.0, 6.0, 8.0, 10.0, 14.0, 14.0, 20.9, 21.0])
    {
        close(*actual, expected);
    }
    close(column.temperature_k[0], 274.0);
    for temperature in &column.temperature_k[1..] {
        close(*temperature, 273.16);
    }
    for (actual, expected) in column.ice_fraction.iter().zip([
        0.0,
        0.096_286_585_443_645_7,
        0.210_342_183_998_800_92,
        0.321_261_402_941_647,
        0.430_465_443_348_321_43,
        0.538_238_545_434_052_8,
        0.644_801_940_957_52,
        0.749_816_780_602_946_4,
        0.853_834_869_607_472_5,
        0.956_366_227_938_792_1,
    ]) {
        close(*actual, expected);
    }
}

#[test]
fn lake_adjustment_keeps_a_zero_depth_column_and_rejects_bad_shape() {
    let mut empty = LakeColumn {
        thickness_m: vec![0.0; 10],
        temperature_k: vec![260.0; 10],
        ice_fraction: vec![1.0; 10],
    };
    adjust_lake_layers(&mut empty).unwrap();
    assert_eq!(empty.thickness_m, vec![0.0; 10]);
    let mut bad = LakeColumn {
        thickness_m: vec![1.0; 9],
        temperature_k: vec![273.0; 9],
        ice_fraction: vec![0.0; 9],
    };
    assert!(adjust_lake_layers(&mut bad).is_err());
}

#[test]
fn lake_roughness_matches_the_open_water_ice_and_snow_branches() {
    // Standalone gfortran references from MOD_Lake:roughness_lake.
    let open = lake_roughness(LakeRoughnessInput {
        snow_layer_count: 0,
        ground_temperature_k: 280.0,
        lake_surface_temperature_k: 279.0,
        surface_pressure_pa: 100_000.0,
        charnock_parameter: 0.015,
        friction_velocity_m_s: 0.3,
    })
    .unwrap();
    close(open.momentum_m, 1.376_685_675_126_655e-4);
    close(open.sensible_heat_m, 1.823_950_908_080_136_8e-5);
    close(open.latent_heat_m, 2.842_694_462_596_562e-5);
    let ice = lake_roughness(LakeRoughnessInput {
        ground_temperature_k: 270.0,
        lake_surface_temperature_k: 270.0,
        ..LakeRoughnessInput {
            snow_layer_count: 0,
            ground_temperature_k: 280.0,
            lake_surface_temperature_k: 279.0,
            surface_pressure_pa: 100_000.0,
            charnock_parameter: 0.015,
            friction_velocity_m_s: 0.3,
        }
    })
    .unwrap();
    close(ice.momentum_m, 0.001);
    close(ice.sensible_heat_m, 6.062_255_359_617_881e-4);
    let snow = lake_roughness(LakeRoughnessInput {
        snow_layer_count: -1,
        ..LakeRoughnessInput {
            snow_layer_count: 0,
            ground_temperature_k: 280.0,
            lake_surface_temperature_k: 279.0,
            surface_pressure_pa: 100_000.0,
            charnock_parameter: 0.015,
            friction_velocity_m_s: 0.3,
        }
    })
    .unwrap();
    close(snow.momentum_m, 0.0024);
    close(snow.sensible_heat_m, 0.001_142_594_183_954_192_2);
}

#[test]
#[allow(clippy::excessive_precision)]
fn lake_conductivity_matches_mod_lake_for_open_and_frozen_water() {
    let node_depth_m = [0.05, 0.6, 2.1, 4.6, 8.1, 12.6, 18.6, 25.6, 34.325, 44.775];
    let mut column = LakeColumn {
        thickness_m: vec![0.1, 1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 7.0, 10.45, 10.45],
        temperature_k: vec![
            280.0, 279.0, 278.0, 277.0, 276.0, 275.0, 274.0, 273.0, 272.0, 271.0,
        ],
        ice_fraction: vec![0.0; 10],
    };
    let open = lake_thermal_conductivity(
        LakeConductivityInput {
            snow_layer_count: 0,
            ground_temperature_k: 280.0,
            node_depth_m: &node_depth_m,
            latitude_radians: 0.7,
            friction_velocity_m_s: 0.3,
            momentum_roughness_m: 1.376_685_675_126_655e-4,
            lake_depth_m: 100.0,
            deep_lake_threshold_m: 25.0,
        },
        &column,
    )
    .unwrap();
    for (actual, expected) in open.thermal_conductivity_w_m_k.iter().zip([
        3_589.914_707_850_793,
        26_262.089_104_480_408,
        24_104.997_482_628_87,
        4_056.489_040_394_442,
        520.917_853_399_902_4,
        63.711_119_845_733_364,
        18.480_770_527_158_864,
        16.018_484_106_736_334,
        15.923_410_019_232_465,
        15.923_410_019_232_465,
    ]) {
        close(*actual, expected);
    }
    close(open.top_eddy_conductivity_w_m_k, 3_589.914_707_850_793);

    column.temperature_k.fill(270.0);
    column.ice_fraction = vec![0.5, 0.4, 0.3, 0.2, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0];
    let frozen = lake_thermal_conductivity(
        LakeConductivityInput {
            snow_layer_count: 0,
            ground_temperature_k: 270.0,
            node_depth_m: &node_depth_m,
            latitude_radians: 0.7,
            friction_velocity_m_s: 0.3,
            momentum_roughness_m: 0.001,
            lake_depth_m: 10.0,
            deep_lake_threshold_m: 25.0,
        },
        &column,
    )
    .unwrap();
    for (actual, expected) in frozen.thermal_conductivity_w_m_k.iter().zip([
        1.047_080_137_179_393_6,
        1.010_172_020_659_320_8,
        0.969_945_832_810_362_8,
        0.932_033_643_188_937_3,
        0.897_736_028_935_456_7,
        3.184_317_552_596_075,
        3.184_317_552_596_075,
        3.184_317_552_596_075,
        3.184_317_552_596_075,
        3.184_317_552_596_075,
    ]) {
        close(*actual, expected);
    }
    close(frozen.top_eddy_conductivity_w_m_k, 0.697_414_690_570_876_9);
}
