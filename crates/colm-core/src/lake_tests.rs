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
