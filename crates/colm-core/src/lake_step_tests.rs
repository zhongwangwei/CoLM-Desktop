use super::*;

fn lake(thickness_m: f64) -> RuntimeLakeState {
    RuntimeLakeState {
        column: LakeColumn {
            thickness_m: vec![thickness_m; 10],
            temperature_k: vec![280.0; 10],
            ice_fraction: vec![0.0; 10],
        },
        saved_tke: 0.1,
        ground_temperature_k: 280.0,
    }
}

#[test]
fn dry_lake_refill_splits_shallow_ponding_evenly() {
    // `CoLMMAIN.F90:1457-1469`：积水不足 100 mm 时只等分，不重排。
    let mut state = lake(0.1);
    refill_lake_column(&mut state, 50.0, 271.0, 268.5).unwrap();
    assert_eq!(state.ground_temperature_k, 268.5);
    assert_eq!(state.column.thickness_m, vec![50.0 * 1.0e-3 / 10.0; 10]);
    assert_eq!(state.column.temperature_k, vec![271.0; 10]);
    // 土温低于冰点：整列冰。
    assert_eq!(state.column.ice_fraction, vec![1.0; 10]);

    let mut state = lake(0.1);
    refill_lake_column(&mut state, 0.0, crate::FREEZING_K, crate::FREEZING_K).unwrap();
    assert_eq!(state.column.thickness_m, vec![0.0; 10]);
    // 恰在冰点算作未冻（`t_soisno(1) >= tfrz`）。
    assert_eq!(state.column.ice_fraction, vec![0.0; 10]);
}

#[test]
fn dry_lake_refill_remaps_deep_ponding_to_the_standard_layers() {
    // 积水够 100 mm 时再走一遍 `adjust_lake_layer`：1 m 以内仍是等分，但要保持总深。
    let mut state = lake(0.1);
    refill_lake_column(&mut state, 150.0, 285.0, 285.0).unwrap();
    let total: f64 = state.column.thickness_m.iter().sum();
    assert!((total - 0.15).abs() < 1.0e-15, "total depth {total}");
    // 重排是按重叠厚度加权的温度和再除回去，只保证到舍入。
    for temperature in &state.column.temperature_k {
        assert!(
            (temperature - 285.0).abs() < 1.0e-12,
            "temperature {temperature}"
        );
    }
    assert_eq!(state.column.ice_fraction, vec![0.0; 10]);
}
