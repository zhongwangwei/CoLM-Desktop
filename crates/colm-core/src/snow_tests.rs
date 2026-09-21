use super::*;

#[test]
fn snow_age_matches_mod_albedo_and_resets_for_no_or_antarctic_snow() {
    // Standalone gfortran reference from MOD_Albedo:snowage.
    assert!(
        (update_snow_age(1800.0, 270.0, 25.0, 25.0, 0.0).unwrap() - 0.002204192232638983).abs()
            < 1.0e-15
    );
    assert!(
        (update_snow_age(3600.0, 268.0, 12.0, 10.0, 0.4).unwrap() - 0.3229734684006023).abs()
            < 1.0e-14
    );
    assert_eq!(update_snow_age(1800.0, 270.0, 0.0, 10.0, 0.9).unwrap(), 0.0);
    assert_eq!(
        update_snow_age(1800.0, 270.0, 801.0, 800.0, 0.9).unwrap(),
        0.0
    );
    assert!(update_snow_age(0.0, 270.0, 1.0, 1.0, 0.0).is_err());
}

#[test]
fn snow_fraction_matches_mod_snowfraction() {
    // 独立 gfortran 程序（`-fdefault-real-8`）逐字复制 MOD_SnowFraction:snowfraction，
    // 指数取默认 1.0。第一组用的正是 CN-Cng 对齐算例 2008-01-12 00:00 重启里的
    // `tlai/tsai/z0m/zlnd/scv/snowdp`，Fortran 自己算出 `fsno = 0.01756794`。
    for (lai, sai, z0m, zlnd, scv, snowdp, wt, sigf, fsno) in [
        (
            0.2,
            0.45,
            0.120_559_31,
            0.01,
            0.047_187_64,
            0.000_455_27,
            0.000377489005685568,
            0.9996225109943144,
            0.01756811294015333,
        ),
        // `snowdp == 0`：`fsno` 恒为 0，`sigf` 为 1。
        (0.2, 0.45, 0.120_572_65, 0.01, 0.0, 0.0, 0.0, 1.0, 0.0),
        // 无冠层：`wt`/`sigf` 走 ELSE 支，而 `fsno` 照算。
        (
            0.0,
            0.0,
            0.1,
            0.01,
            5.0,
            0.02,
            0.0,
            1.0,
            0.30950692121263845,
        ),
        (
            1.0,
            0.5,
            0.05,
            0.01,
            20.0,
            0.1,
            0.16666666666666666,
            0.8333333333333334,
            0.9640275800758169,
        ),
        (
            0.5,
            0.1,
            0.2,
            0.02,
            100.0,
            0.5,
            0.2,
            0.8,
            0.9999092042625951,
        ),
        // 深雪饱和：`tanh` 到 1。
        (
            1.0,
            0.0,
            0.3,
            0.01,
            0.0,
            0.005,
            0.0016638935108153079,
            0.9983361064891847,
            1.0,
        ),
    ] {
        let fraction = snow_fraction(lai, sai, z0m, zlnd, scv, snowdp, 1.0).unwrap();
        assert!((fraction.vegetation_snow_fraction - wt).abs() < 1.0e-15);
        assert!((fraction.vegetation_free_fraction - sigf).abs() < 1.0e-15);
        assert!((fraction.ground_snow_fraction - fsno).abs() < 1.0e-15);
    }
    // 上游的 `snowdp > 0` 判据是**严格**大于：0 深度不给 `fmelt` 一个 `0/0`。
    assert_eq!(
        snow_fraction(0.2, 0.45, 0.1, 0.01, 0.0, 0.0, 1.0)
            .unwrap()
            .ground_snow_fraction,
        0.0
    );
    // `z0m <= 0` 会让 `wt` 变成 `-0.1*snowdp/0`；上游不检查，这里拒绝。
    assert!(snow_fraction(0.2, 0.45, 0.0, 0.01, 1.0, 0.01, 1.0).is_err());
    assert!(snow_fraction(0.2, 0.45, 0.1, 0.0, 1.0, 0.01, 1.0).is_err());
}

fn input() -> NewSnowInput {
    NewSnowInput {
        patch_type: 0,
        time_step_seconds: 1800.0,
        ground_temperature_k: 270.0,
        ground_snowfall_kg_m2_s: 0.002,
        new_snow_bulk_density_kg_m3: 100.0,
        precipitation_temperature_k: 269.0,
        variably_saturated_flow: true,
    }
}

#[test]
fn fresh_and_existing_snow_match_current_fortran_newsnow() {
    let mut state = RuntimeSnowColumn::empty();
    add_new_snow(input(), &mut state).unwrap();
    assert_eq!(state.layer_count, -1);
    close(state.water_equivalent_kg_m2, 3.6);
    close(state.depth_m, 0.036);
    close(state.ground_snow_fraction, 0.34521403413552093);
    close(state.thickness_m[layer_slot(0)], 0.036);
    close(state.node_depth_m[layer_slot(0)], -0.018);
    close(state.interface_depth_m[interface_slot(-1)], -0.036);
    assert_eq!(state.temperature_k[layer_slot(0)], 269.0);
    close(state.ice_water_kg_m2[layer_slot(0)], 3.6);

    let mut later = input();
    later.ground_snowfall_kg_m2_s = 0.001;
    add_new_snow(later, &mut state).unwrap();
    close(state.water_equivalent_kg_m2, 5.4);
    close(state.depth_m, 0.054);
    close(state.ground_snow_fraction, 0.46181888736771204);
    close(state.thickness_m[layer_slot(0)], 0.054);
    close(state.node_depth_m[layer_slot(0)], -0.027);
    close(state.ice_water_kg_m2[layer_slot(0)], 5.4);
}

#[test]
fn warm_wetland_transfers_fresh_snow_to_its_external_store() {
    let mut state = RuntimeSnowColumn::empty();
    let mut wetland = input();
    wetland.patch_type = 2;
    wetland.ground_temperature_k = 274.0;
    let outcome = add_new_snow(wetland, &mut state).unwrap();
    close(outcome.wetland_water_added_mm, 3.6);
    assert_eq!(state.layer_count, 0);
    assert_eq!(state.water_equivalent_kg_m2, 0.0);
    assert_eq!(state.depth_m, 0.0);
    assert_eq!(state.ground_snow_fraction, 0.0);
}

#[test]
fn invalid_snow_inputs_and_shapes_are_rejected() {
    let mut state = RuntimeSnowColumn::empty();
    let mut invalid = input();
    invalid.new_snow_bulk_density_kg_m3 = 0.0;
    assert!(add_new_snow(invalid, &mut state).is_err());
    state.thickness_m.pop();
    assert!(add_new_snow(input(), &mut state).is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn compaction_matches_current_fortran_destructive_melt_and_wind_terms() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -3;
    for (fortran_layer, temperature, liquid, ice, thickness) in [
        (-2, 267.0, 1.0, 20.0, 0.12),
        (-1, 270.0, 3.0, 30.0, 0.08),
        (0, 273.0, 0.5, 15.0, 0.04),
    ] {
        let slot = layer_slot(fortran_layer);
        state.temperature_k[slot] = temperature;
        state.liquid_water_kg_m2[slot] = liquid;
        state.ice_water_kg_m2[slot] = ice;
        state.thickness_m[slot] = thickness;
        state.previous_ice_fraction[slot] = 0.95;
    }

    compact_snow_layers(&mut state, 1800.0, 8.0, 2.0, &[true, false, true]).unwrap();

    close(state.thickness_m[layer_slot(-2)], 0.11726473275210518);
    close(state.thickness_m[layer_slot(-1)], 0.07999990731579197);
    close(state.thickness_m[layer_slot(0)], 0.03999994461272519);
}

#[test]
fn compaction_rejects_mismatched_flags_and_zero_previous_melt_fraction() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -1;
    let slot = layer_slot(0);
    state.thickness_m[slot] = 0.04;
    state.temperature_k[slot] = 270.0;
    state.ice_water_kg_m2[slot] = 4.0;
    assert!(compact_snow_layers(&mut state, 1800.0, 0.0, 0.0, &[]).is_err());
    assert!(compact_snow_layers(&mut state, 1800.0, 0.0, 0.0, &[true]).is_err());
}

#[test]
fn combining_thin_snow_matches_current_fortran_enthalpy_and_geometry() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -4;
    for (fortran_layer, thickness, ice, liquid, temperature) in [
        (-3, 0.003, 0.2, 0.02, 260.0),
        (-2, 0.013, 2.0, 0.5, 266.0),
        (-1, 0.06, 12.0, 1.0, 270.0),
        (0, 0.12, 30.0, 2.0, 272.0),
    ] {
        let slot = layer_slot(fortran_layer);
        state.thickness_m[slot] = thickness;
        state.ice_water_kg_m2[slot] = ice;
        state.liquid_water_kg_m2[slot] = liquid;
        state.temperature_k[slot] = temperature;
    }
    let mut soil_surface = SnowToSoilTransfer {
        liquid_water_kg_m2: 7.0,
        ice_water_kg_m2: 2.0,
    };

    combine_snow_layers(&mut state, &mut soil_surface).unwrap();

    assert_eq!(state.layer_count, -3);
    close(state.water_equivalent_kg_m2, 47.72);
    close(state.depth_m, 0.196);
    close(state.thickness_m[layer_slot(-2)], 0.016);
    close(state.ice_water_kg_m2[layer_slot(-2)], 2.2);
    close(state.liquid_water_kg_m2[layer_slot(-2)], 0.52);
    close(state.temperature_k[layer_slot(-2)], 273.16);
    close(state.node_depth_m[layer_slot(-2)], -0.188);
    close(state.interface_depth_m[interface_slot(-3)], -0.196);
    close(soil_surface.liquid_water_kg_m2, 7.0);
    close(soil_surface.ice_water_kg_m2, 2.0);
}

#[test]
fn combining_subcentimeter_snow_preserves_ice_and_moves_liquid_to_soil() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -1;
    let slot = layer_slot(0);
    state.thickness_m[slot] = 0.005;
    state.ice_water_kg_m2[slot] = 0.2;
    state.liquid_water_kg_m2[slot] = 0.03;
    state.temperature_k[slot] = 270.0;
    let mut soil_surface = SnowToSoilTransfer::default();

    combine_snow_layers(&mut state, &mut soil_surface).unwrap();

    assert_eq!(state.layer_count, 0);
    close(state.water_equivalent_kg_m2, 0.2);
    close(state.depth_m, 0.005);
    close(soil_surface.liquid_water_kg_m2, 0.03);
}

#[test]
fn dividing_thick_snow_matches_current_fortran_enthalpy_and_geometry() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -2;
    for (fortran_layer, thickness, ice, liquid, temperature) in
        [(-1, 0.05, 4.0, 2.0, 270.0), (0, 0.08, 15.0, 3.0, 275.0)]
    {
        let slot = layer_slot(fortran_layer);
        state.thickness_m[slot] = thickness;
        state.ice_water_kg_m2[slot] = ice;
        state.liquid_water_kg_m2[slot] = liquid;
        state.temperature_k[slot] = temperature;
    }
    state.water_equivalent_kg_m2 = 24.0;
    state.depth_m = 0.13;

    divide_snow_layers(&mut state).unwrap();

    assert_eq!(state.layer_count, -3);
    close(state.water_equivalent_kg_m2, 24.0);
    close(state.depth_m, 0.13);
    close(state.thickness_m[layer_slot(-2)], 0.02);
    close(state.liquid_water_kg_m2[layer_slot(-2)], 0.7999999999999999);
    close(state.ice_water_kg_m2[layer_slot(-2)], 1.5999999999999999);
    close(state.temperature_k[layer_slot(-2)], 270.0);
    close(state.thickness_m[layer_slot(-1)], 0.05);
    close(state.liquid_water_kg_m2[layer_slot(-1)], 1.9090909090909094);
    close(state.ice_water_kg_m2[layer_slot(-1)], 7.909090909090909);
    close(state.temperature_k[layer_slot(-1)], 274.0715570638877);
    close(state.thickness_m[layer_slot(0)], 0.06);
    close(state.liquid_water_kg_m2[layer_slot(0)], 2.290909090909091);
    close(state.ice_water_kg_m2[layer_slot(0)], 9.49090909090909);
    close(state.temperature_k[layer_slot(0)], 274.0715570638877);
    close(state.node_depth_m[layer_slot(-2)], -0.12);
    close(state.interface_depth_m[interface_slot(-3)], -0.13);
}

#[test]
fn snow_water_matches_current_fortran_percolation_and_surface_fluxes() {
    // Standalone gfortran reference from MOD_SoilSnowHydrology:snowwater.
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -2;
    state.water_equivalent_kg_m2 = 95.0;
    state.depth_m = 0.15;
    for (fortran_layer, thickness, ice, liquid) in [(-1, 0.05, 20.0, 5.0), (0, 0.1, 60.0, 10.0)] {
        let slot = layer_slot(fortran_layer);
        state.thickness_m[slot] = thickness;
        state.temperature_k[slot] = 270.0;
        state.ice_water_kg_m2[slot] = ice;
        state.liquid_water_kg_m2[slot] = liquid;
    }

    let outcome = snow_water(
        SnowWaterInput {
            time_step_seconds: 1800.0,
            irreducible_saturation: 0.033,
            impermeable_porosity: 0.05,
            rainfall_kg_m2_s: 0.001,
            evaporation_kg_m2_s: 0.0001,
            dew_kg_m2_s: 0.0002,
            sublimation_kg_m2_s: 0.0001,
            frost_kg_m2_s: 0.0003,
        },
        &mut state,
    )
    .unwrap();

    close(state.ice_water_kg_m2[layer_slot(-1)], 20.36);
    close(state.liquid_water_kg_m2[layer_slot(-1)], 0.9173064340239909);
    close(state.ice_water_kg_m2[layer_slot(0)], 60.0);
    close(state.liquid_water_kg_m2[layer_slot(0)], 7.2034787350054525);
    close(outcome.layer_drainage_kg_m2[0], 6.0626935659760095);
    close(outcome.layer_drainage_kg_m2[1], 8.859_214_830_970_556);
    close(outcome.bottom_drainage_kg_m2_s, 0.004921786017205864);
}

#[test]
fn combining_exhausted_snow_resets_aggregate_state_like_fortran() {
    let mut state = RuntimeSnowColumn::empty();
    state.layer_count = -1;
    state.water_equivalent_kg_m2 = 0.4;
    state.depth_m = 0.02;
    let top = layer_slot(0);
    state.thickness_m[top] = 0.02;
    state.temperature_k[top] = 270.0;
    state.ice_water_kg_m2[top] = 0.1;
    state.liquid_water_kg_m2[top] = 0.3;
    let mut lake_surface = SnowToSoilTransfer::default();

    combine_snow_layers(&mut state, &mut lake_surface).unwrap();

    assert_eq!(state.layer_count, 0);
    close(state.water_equivalent_kg_m2, 0.0);
    close(state.depth_m, 0.0);
    close(lake_surface.ice_water_kg_m2, 0.1);
    close(lake_surface.liquid_water_kg_m2, 0.3);
}

/// 重启里的雪段：层数按水量数，界面深度按上游递推。
///
/// 期望值手算自 `CoLMMAIN.F90:816-826`，不是在测试里重算一遍实现。
#[test]
fn a_restart_snow_column_counts_layers_from_water_content() {
    // 只有最下面那一层有水：Fortran 层 `0`，槽位 4。
    let mut liquid = [0.0; 5];
    let mut ice = [0.0; 5];
    let mut thickness = [0.0; 5];
    thickness[4] = 0.05;
    ice[4] = 20.0;
    liquid[4] = 5.0;
    let column = RuntimeSnowColumn::from_restart(
        0,
        RestartSnowSlots {
            node_depth_m: &[-0.025, 0.0, 0.0, 0.0, 0.0],
            thickness_m: &thickness,
            temperature_k: &[0.0, 0.0, 0.0, 0.0, 268.0],
            liquid_water_kg_m2: &liquid,
            ice_water_kg_m2: &ice,
            water_equivalent_kg_m2: 25.0,
            depth_m: 0.05,
            ground_snow_fraction: 0.9,
            age: 0.5,
        },
    )
    .unwrap();
    assert_eq!(column.layer_count, -1);
    assert_eq!(column.water_equivalent_kg_m2, 25.0);
    assert_eq!(column.depth_m, 0.05);
    assert_eq!(column.ground_snow_fraction, 0.9);
    assert_eq!(column.age, 0.5);
    // `snl = -1` 时唯一算出来的界面是 `zi(-1) = zi(0) - dz(0) = -0.05`。
    assert_eq!(column.interface_depth_m[snow_interface_slot(0)], 0.0);
    assert_eq!(column.interface_depth_m[snow_interface_slot(-1)], -0.05);
    // `fiold = wice/(wliq+wice)`。
    assert_eq!(column.previous_ice_fraction[4], 20.0 / 25.0);
    // 没被数进去的槽位保持 0，不会把上一轮的残值带进来。
    assert!(column.thickness_m[..4].iter().all(|value| *value == 0.0));
}

#[test]
fn a_full_restart_snow_column_chains_the_interfaces() {
    let thickness = [0.02, 0.05, 0.11, 0.10, 0.10];
    let water = [1.0; 5];
    let column = RuntimeSnowColumn::from_restart(
        0,
        RestartSnowSlots {
            node_depth_m: &[-0.01, -0.045, -0.125, -0.175, -0.225],
            thickness_m: &thickness,
            temperature_k: &[268.0; 5],
            liquid_water_kg_m2: &water,
            ice_water_kg_m2: &water,
            water_equivalent_kg_m2: 10.0,
            depth_m: 0.38,
            ground_snow_fraction: 1.0,
            age: 1.0,
        },
    )
    .unwrap();
    assert_eq!(column.layer_count, -5);
    // `zi(0) = 0`，然后 `zi(-1) = -dz(0)`、`zi(-2) = zi(-1) - dz(-1)`…
    // 按 `zi(0), zi(-1), …, zi(-5)` 排列。
    let expected = [0.0, -0.10, -0.20, -0.31, -0.36, -0.38];
    for interface in -5..=0 {
        let slot = snow_interface_slot(interface);
        let expected = expected[(-interface) as usize];
        assert!(
            (column.interface_depth_m[slot] - expected).abs() < 1.0e-15,
            "zi({interface}): {} != {expected}",
            column.interface_depth_m[slot]
        );
    }
}

#[test]
fn a_restart_snow_column_with_a_gap_is_refused() {
    // 中间空一层：`snl` 数出来会比实际占用的层数多，而内核按「最后 |snl| 个槽位」
    // 组织数据 —— 这种列必须先报错，不能带着跑。
    let mut ice = [0.0; 5];
    ice[2] = 5.0;
    ice[4] = 5.0;
    let error = RuntimeSnowColumn::from_restart(
        0,
        RestartSnowSlots {
            node_depth_m: &[0.0; 5],
            thickness_m: &[0.0, 0.0, 0.1, 0.0, 0.1],
            temperature_k: &[0.0, 0.0, 268.0, 0.0, 268.0],
            liquid_water_kg_m2: &[0.0; 5],
            ice_water_kg_m2: &ice,
            water_equivalent_kg_m2: 10.0,
            depth_m: 0.2,
            ground_snow_fraction: 1.0,
            age: 1.0,
        },
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("breaks the column"),
        "{error:#}"
    );
}

#[test]
fn a_restart_snow_column_rejects_impossible_values() {
    let snow = |liquid: [f64; 5], ice: [f64; 5], patch_type: i32| {
        RuntimeSnowColumn::from_restart(
            patch_type,
            RestartSnowSlots {
                node_depth_m: &[0.0; 5],
                thickness_m: &[0.0, 0.0, 0.0, 0.0, 0.1],
                temperature_k: &[0.0, 0.0, 0.0, 0.0, 268.0],
                liquid_water_kg_m2: &liquid,
                ice_water_kg_m2: &ice,
                water_equivalent_kg_m2: 10.0,
                depth_m: 0.1,
                ground_snow_fraction: 1.0,
                age: 1.0,
            },
        )
    };
    let mut negative = [0.0; 5];
    negative[4] = -1.0;
    assert!(snow([0.0; 5], negative, 0).is_err());
    // 水体不可能带雪列。
    let mut water = [0.0; 5];
    water[4] = 5.0;
    assert!(snow([0.0; 5], water, 4).is_err());
    // 全空的雪列是合法的：`snl = 0`。
    let empty = snow([0.0; 5], [0.0; 5], 0).unwrap();
    assert_eq!(empty.layer_count, 0);
    assert_eq!(empty.interface_depth_m[snow_interface_slot(0)], 0.0);
}
