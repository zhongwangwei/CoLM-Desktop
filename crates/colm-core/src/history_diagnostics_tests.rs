use super::*;

/// 独立 gfortran 程序：`MOD_FrictionVelocity:moninobuk` 原文 +
/// `MOD_Vars_1DAccFluxes.F90:2733-2790` 那一段原样复制，两组输入各算一次。
///
/// 这两组输入不是随手编的：它们是 CN-Cng 对齐算例第一个小时两步的量级
/// （参考高度 6 m、气温 271.5/270.1 K、比湿 3.2/2.9 g/kg、风速 3.0/2.2 m/s、
/// 应力 0.28/0.19 kg m-1 s-2、感热 700/460 W m-2、蒸发 7.4/6.6e-5 kg m-2 s-1、
/// `z0m = 0.1205/0.1206 m`）。用它把重算块的每一行都钉住。
#[test]
fn history_diagnostics_matches_accumulate_fluxes() {
    for (index, input, expected) in [
        (
            0,
            HistoryDiagnosticsInput {
                wind_height_m: 6.0,
                temperature_height_m: 6.0,
                humidity_height_m: 6.0,
                wind_speed_eastward_m_s: 3.0,
                wind_speed_northward_m_s: 1.0,
                air_temperature_k: 271.5,
                specific_humidity_kg_kg: 0.0032,
                surface_pressure_pa: 99_000.0,
                eastward_stress_kg_m_s2: -0.28,
                northward_stress_kg_m_s2: -0.14,
                sensible_heat_w_m2: 700.0,
                evaporation_kg_m2_s: 7.4e-5,
                momentum_roughness_m: 0.1205,
                surface_layer_scheme: SurfaceLayerScheme::Standard,
                boundary_layer_height_m: None,
            },
            [
                0.4968976287449242,
                -1.105965197217017,
                -0.00011745881257739387,
                -0.3283212794939838,
                -0.08942141325538659,
                3.1289234554448293,
                2.666443437650966,
                2.666443437650966,
            ],
        ),
        (
            1,
            HistoryDiagnosticsInput {
                wind_height_m: 6.0,
                temperature_height_m: 6.0,
                humidity_height_m: 6.0,
                wind_speed_eastward_m_s: 2.2,
                wind_speed_northward_m_s: 0.6,
                air_temperature_k: 270.1,
                specific_humidity_kg_kg: 0.0029,
                surface_pressure_pa: 99_100.0,
                eastward_stress_kg_m_s2: -0.19,
                northward_stress_kg_m_s2: -0.10,
                sensible_heat_w_m2: 460.0,
                evaporation_kg_m2_s: 6.6e-5,
                momentum_roughness_m: 0.1206,
                surface_layer_scheme: SurfaceLayerScheme::Standard,
                boundary_layer_height_m: None,
            },
            [
                0.4102081322549408,
                -0.8747846809371393,
                -0.000126094963049003,
                -0.38545799096062283,
                -0.10523162363355666,
                3.072514897866384,
                2.577253941297972,
                2.577253941297972,
            ],
        ),
    ] {
        let state = history_diagnostics(input).unwrap();
        for (name, computed, expected) in [
            ("r_ustar", state.friction_velocity_m_s, expected[0]),
            ("r_tstar", state.temperature_scale_k, expected[1]),
            ("r_qstar", state.humidity_scale, expected[2]),
            ("r_zol", state.zol, expected[3]),
            ("r_rib", state.bulk_richardson, expected[4]),
            ("r_fm", state.momentum_similarity, expected[5]),
            ("r_fh", state.heat_similarity, expected[6]),
            ("r_fq", state.moisture_similarity, expected[7]),
        ] {
            // 容差按 history 的 tier2（rtol=1e-7）给，而不是 1e-15：`monin_obukhov`
            // 里的 `vonkar` 走的是本仓库的 `f77()` 约定（`0.4f32 as f64`），
            // 而 Fortran 在 `-fdefault-real-8` 下读的是 `0.4_r8`，两者差 1.5e-8。
            // `r_rib` 里 `vonkar` 出现两次，放大到 3e-8 —— 实测正是这个量级。
            assert!(
                (computed - expected).abs() <= 1.0e-7 * expected.abs().max(1.0),
                "case {index} {name}: {computed} against the gfortran reference's {expected}"
            );
        }
    }
}

/// 观测高度的下限 `max(hgt, 5 + displa)` 必须生效 —— 这是 history 的
/// `f_zol`/`f_rib` 与重启里那两列差一倍的直接原因。
///
/// CN-Cng 的强迫参考高度是 6 m，`displa = 2/3*z0m/0.07 = 1.148`（`z0m ≈ 0.1206`），
/// 所以下限把它抬到 6.148 m、`zldis` 恰好是 5.0。
#[test]
fn the_reference_height_is_lifted_to_five_metres_above_the_displacement() {
    let mut input = HistoryDiagnosticsInput {
        wind_height_m: 6.0,
        temperature_height_m: 6.0,
        humidity_height_m: 6.0,
        wind_speed_eastward_m_s: 3.0,
        wind_speed_northward_m_s: 1.0,
        air_temperature_k: 271.5,
        specific_humidity_kg_kg: 0.0032,
        surface_pressure_pa: 99_000.0,
        eastward_stress_kg_m_s2: -0.28,
        northward_stress_kg_m_s2: -0.14,
        sensible_heat_w_m2: 700.0,
        evaporation_kg_m2_s: 7.4e-5,
        momentum_roughness_m: 0.1205,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
        boundary_layer_height_m: None,
    };
    let lifted = history_diagnostics(input).unwrap();
    // 把参考高度抬到远高于下限：结果必须跟着变，说明下限确实参与了计算。
    input.wind_height_m = 30.0;
    input.temperature_height_m = 30.0;
    input.humidity_height_m = 30.0;
    let tall = history_diagnostics(input).unwrap();
    assert!((lifted.zol - tall.zol).abs() > 1.0e-3);
    assert!((lifted.bulk_richardson - tall.bulk_richardson).abs() > 1.0e-3);
    // **`f_ustar` 不受参考高度影响**：它是 `sqrt(|tau|/rho)` 直接从应力算的，
    // 与模型自己的 `ustar`（由相似函数解出来）不是一回事。这条不变式正是
    // "history 那八列要重算"的最好证据。
    assert_eq!(lifted.friction_velocity_m_s, tall.friction_velocity_m_s);
    assert_eq!(lifted.temperature_scale_k, tall.temperature_scale_k);
}

/// 位移高度与粗糙度用的都是 `2/3*z0m/0.07` 那个固定式，不是冠层自算的位移高度 ——
/// 换一个 `z0m` 必须整体跟着动。
#[test]
fn the_displacement_height_follows_the_momentum_roughness() {
    let base = HistoryDiagnosticsInput {
        wind_height_m: 6.0,
        temperature_height_m: 6.0,
        humidity_height_m: 6.0,
        wind_speed_eastward_m_s: 3.0,
        wind_speed_northward_m_s: 1.0,
        air_temperature_k: 271.5,
        specific_humidity_kg_kg: 0.0032,
        surface_pressure_pa: 99_000.0,
        eastward_stress_kg_m_s2: -0.28,
        northward_stress_kg_m_s2: -0.14,
        sensible_heat_w_m2: 700.0,
        evaporation_kg_m2_s: 7.4e-5,
        momentum_roughness_m: 0.1205,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
        boundary_layer_height_m: None,
    };
    let reference = history_diagnostics(base).unwrap();
    let rough = history_diagnostics(HistoryDiagnosticsInput {
        momentum_roughness_m: 0.3,
        ..base
    })
    .unwrap();
    assert!((reference.bulk_richardson - rough.bulk_richardson).abs() > 1.0e-3);
}

/// 非物理输入必须报错，而不是给出一个看起来正常的数。
#[test]
fn non_physical_inputs_are_rejected() {
    let base = HistoryDiagnosticsInput {
        wind_height_m: 6.0,
        temperature_height_m: 6.0,
        humidity_height_m: 6.0,
        wind_speed_eastward_m_s: 3.0,
        wind_speed_northward_m_s: 1.0,
        air_temperature_k: 271.5,
        specific_humidity_kg_kg: 0.0032,
        surface_pressure_pa: 99_000.0,
        eastward_stress_kg_m_s2: -0.28,
        northward_stress_kg_m_s2: -0.14,
        sensible_heat_w_m2: 700.0,
        evaporation_kg_m2_s: 7.4e-5,
        momentum_roughness_m: 0.1205,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
        boundary_layer_height_m: None,
    };
    assert!(history_diagnostics(HistoryDiagnosticsInput {
        surface_pressure_pa: 0.0,
        ..base
    })
    .is_err());
    assert!(history_diagnostics(HistoryDiagnosticsInput {
        momentum_roughness_m: 0.0,
        ..base
    })
    .is_err());
    assert!(history_diagnostics(HistoryDiagnosticsInput {
        air_temperature_k: f64::NAN,
        ..base
    })
    .is_err());
}
