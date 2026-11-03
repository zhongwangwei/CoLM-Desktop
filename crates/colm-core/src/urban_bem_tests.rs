use super::*;

#[test]
fn bem_matches_upstream_unconditioned_solution() {
    let state = urban_bem(input()).unwrap();
    // Standalone gfortran/LAPACK run of MOD_Urban_BEM.F90 with input().
    close(state.room_temperature_k, 292.0697118632375);
    close(state.roof_inner_temperature_k, 290.7731693221621);
    close(state.sunlit_wall_inner_temperature_k, 288.8627148534697);
    close(state.shaded_wall_inner_temperature_k, 287.2735086490582);
    close(state.cooling_energy_w_m2, 0.0);
    close(state.waste_heat_w_m2, 0.0);
    close(state.air_exchange_w_m2, -1.1775538694868306);
    close(state.heating_energy_w_m2, 0.0);
}

#[test]
fn bem_matches_current_fortran_effective_ac_heating_cooling_and_ventilation() {
    // `python3 oracle/scripts/test_urban_bem.py`: current vendor SimpleBEM + its LAPACK inverse.
    let expected: [[f64; 8]; 8] = [
        [
            292.0697118632375,
            290.7731693221621,
            288.8627148534697,
            287.2735086490582,
            0.0,
            0.0,
            -1.1775538694868306,
            0.0,
        ],
        [
            289.99999999999994,
            290.2518703241895,
            288.51051117503874,
            286.90067931599907,
            5.530049415529866,
            1.6590148246589598,
            -2.0092800000000226,
            0.0,
        ],
        [
            288.7116847012589,
            289.92738193223727,
            288.2912780560452,
            286.6686075275868,
            4.2873819404558295,
            1.2862145821367488,
            -2.5269972326909076,
            0.0,
        ],
        [
            289.99999999999994,
            290.2518703241895,
            288.51051117503874,
            286.90067931599907,
            5.530049415529866,
            1.6590148246589598,
            -2.0092800000000226,
            0.0,
        ],
        [
            294.99999999999994,
            291.5112219451372,
            289.3613631334366,
            287.80135863199814,
            0.0,
            2.0705776999193843,
            -2.2842868929728862e-14,
            10.352888499596922,
        ],
        [
            291.2116847012589,
            290.55705774271115,
            288.7167040352441,
            287.1189471855863,
            0.0,
            1.0352888499596922,
            -1.5223572326909076,
            5.176444249798461,
        ],
        [
            288.7116847012589,
            289.92738193223727,
            288.2912780560452,
            286.6686075275868,
            1.7603847077649215,
            0.5281154123294763,
            3.5008427673090914,
            0.0,
        ],
        [
            291.2116847012589,
            290.55705774271115,
            288.7167040352441,
            287.1189471855863,
            0.0,
            1.0352888499596922,
            -7.5501972326909055,
            5.176444249798461,
        ],
    ];
    for (index, expected) in expected.into_iter().enumerate() {
        let case = index + 1;
        let mut sample = input();
        sample.room_max_temperature_k = if matches!(case, 1 | 5 | 6 | 8) {
            298.0
        } else {
            290.0
        };
        if matches!(case, 5 | 6 | 8) {
            sample.room_min_temperature_k = 295.0;
        }
        if matches!(case, 3 | 6 | 7 | 8) {
            sample.ac_usage_fraction = 0.5;
        }
        if case == 4 {
            sample.ac_usage_fraction = ac_usage_fraction(41_400, &[0.0; 24]).unwrap();
        }
        if case == 7 {
            sample.urban_air_temperature_k = 280.0;
        }
        if case == 8 {
            sample.urban_air_temperature_k = 310.0;
        }
        let state = urban_bem(sample).unwrap();
        let actual = [
            state.room_temperature_k,
            state.roof_inner_temperature_k,
            state.sunlit_wall_inner_temperature_k,
            state.shaded_wall_inner_temperature_k,
            state.cooling_energy_w_m2,
            state.waste_heat_w_m2,
            state.air_exchange_w_m2,
            state.heating_energy_w_m2,
        ];
        for (actual, expected) in actual.into_iter().zip(expected) {
            close(actual, expected);
        }
    }
}

#[test]
fn effective_ac_profiles_use_local_step_start_and_disabled_lucy_keeps_constant_ac() {
    let mut profile = [1.0; 24];
    profile[0] = 0.5;
    profile[1] = 0.25;
    profile[23] = 0.75;
    assert_eq!(ac_usage_fraction(3599, &profile).unwrap(), 0.5);
    assert_eq!(ac_usage_fraction(3600, &profile).unwrap(), 0.25);
    assert_eq!(ac_usage_fraction(86_399, &profile).unwrap(), 0.75);
    assert_eq!(ac_usage_fraction(86_400, &profile).unwrap(), 0.5);
    assert_eq!(ac_usage_fraction(0, &[0.0; 24]).unwrap(), 1.0);
    assert!(ac_usage_fraction(0, &[1.0; 23]).is_err());
    profile[0] = f64::NAN;
    assert!(ac_usage_fraction(0, &profile).is_err());
    for usage in [f64::NAN, -0.1, 1.1] {
        assert!(urban_bem(UrbanBemInput {
            ac_usage_fraction: usage,
            ..input()
        })
        .is_err());
    }
}

fn input() -> UrbanBemInput {
    UrbanBemInput {
        time_step_seconds: 1800.0,
        ac_usage_fraction: 1.0,
        air_density_kg_m3: 1.2,
        cover_fraction: [0.4, 0.35, 0.25],
        building_height_m: 10.0,
        room_max_temperature_k: 298.0,
        room_min_temperature_k: 288.0,
        roof_outer_temperature_previous_k: 291.0,
        sunlit_wall_outer_temperature_previous_k: 290.0,
        shaded_wall_outer_temperature_previous_k: 289.0,
        roof_outer_temperature_k: 292.0,
        sunlit_wall_outer_temperature_k: 291.0,
        shaded_wall_outer_temperature_k: 290.0,
        roof_inner_conductance_w_m2_k: 12.0,
        sunlit_wall_inner_conductance_w_m2_k: 15.0,
        shaded_wall_inner_conductance_w_m2_k: 14.0,
        urban_air_temperature_k: 295.0,
        room_temperature_k: 294.0,
        roof_inner_temperature_k: 293.0,
        sunlit_wall_inner_temperature_k: 293.0,
        shaded_wall_inner_temperature_k: 293.0,
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 5.0e-12,
        "{actual} != {expected}"
    );
}
