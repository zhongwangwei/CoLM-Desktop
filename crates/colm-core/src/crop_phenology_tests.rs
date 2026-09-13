use super::*;

fn close(actual: f64, expected: f64) {
    let tolerance = 1.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

fn input<'a>(
    time: CalendarTime,
    temperature: &'a [f64],
    live: &'a [bool],
) -> CropPhenologyClimateInput<'a> {
    CropPhenologyClimateInput {
        time,
        time_step_seconds: 1_800,
        latitude_degrees: 40.0,
        pft_class: &[WINTER_WHEAT_CLASS, 17],
        reference_temperature_k: temperature,
        crop_live: live,
        crop_phase: &[2.0, 1.0],
        vernalization_factor: &[0.5, 1.0],
        base_temperature_c: &[5.0, 10.0],
    }
}

#[test]
fn crop_climate_tracks_daily_extrema_seasonal_gdd_and_winter_wheat_vernalization() {
    let mut state = CropPhenologyClimateState::new(2);
    // A live crop is either resumed from a restart or was reset to zero during
    // the preceding non-live climate step before `CropPhenology` plants it.
    state.growing_degree_days_since_planting_c[0] = 0.0;
    crop_phenology_climate_step(
        input(
            CalendarTime {
                year: 2007,
                julian_day: 100,
                seconds: 1_800,
            },
            &[280.0, 270.0],
            &[true, false],
        ),
        &mut state,
    )
    .unwrap();
    crop_phenology_climate_step(
        input(
            CalendarTime {
                year: 2007,
                julian_day: 100,
                seconds: 84_600,
            },
            &[285.0, 275.0],
            &[true, false],
        ),
        &mut state,
    )
    .unwrap();

    close(state.minimum_reference_temperature_k[0], 280.0);
    close(state.maximum_reference_temperature_k[0], 285.0);
    close(state.minimum_reference_temperature_k[1], 270.0);
    close(state.maximum_reference_temperature_k[1], 275.0);
    close(state.growing_degree_days_zero_c[0], (6.85 + 11.85) / 48.0);
    close(state.growing_degree_days_eight_c[0], 3.85 / 48.0);
    close(state.growing_degree_days_ten_c[0], 1.85 / 48.0);
    close(
        state.growing_degree_days_since_planting_c[0],
        0.5 * ((1.85 + 6.85) / 48.0),
    );
    assert_eq!(state.growing_degree_days_since_planting_c[1], 0.0);
    close(
        state.average_reference_temperature_k[0],
        (280.0 + 285.0) / 48.0 / 365.0,
    );
}

#[test]
fn crop_climate_uses_the_source_hemisphere_season_and_new_year_average_order() {
    let mut state = CropPhenologyClimateState::new(2);
    state.growing_degree_days_zero_c = vec![10.0, 20.0];
    state.growing_degree_days_eight_c = vec![8.0, 18.0];
    state.growing_degree_days_ten_c = vec![6.0, 16.0];
    state.active_crop_years = vec![1, 2];
    state.growing_degree_days_zero_twenty_year_c = vec![0.0, 100.0];
    state.growing_degree_days_eight_twenty_year_c = vec![0.0, 80.0];
    state.growing_degree_days_ten_twenty_year_c = vec![0.0, 60.0];
    crop_phenology_climate_step(
        input(
            CalendarTime {
                year: 2007,
                julian_day: 1,
                seconds: 1_800,
            },
            &[280.0, 280.0],
            &[false, false],
        ),
        &mut state,
    )
    .unwrap();

    // Northern January is out of season, so the first value remains 10;
    // upstream samples that value before it clears annual GDD at year start.
    close(state.growing_degree_days_zero_twenty_year_c[0], 10.0);
    close(state.growing_degree_days_eight_twenty_year_c[0], 8.0);
    close(state.growing_degree_days_ten_twenty_year_c[0], 6.0);
    close(state.growing_degree_days_zero_twenty_year_c[1], 96.0);
    close(state.growing_degree_days_eight_twenty_year_c[1], 76.9);
    close(state.growing_degree_days_ten_twenty_year_c[1], 57.8);
    assert_eq!(state.growing_degree_days_zero_c, [0.0, 0.0]);
    assert_eq!(state.growing_degree_days_since_planting_c, [0.0, 0.0]);

    let mut southern = CropPhenologyClimateState::new(1);
    crop_phenology_climate_step(
        CropPhenologyClimateInput {
            time: CalendarTime {
                year: 2007,
                julian_day: 2,
                seconds: 1_800,
            },
            time_step_seconds: 1_800,
            latitude_degrees: -40.0,
            pft_class: &[17],
            reference_temperature_k: &[280.0],
            crop_live: &[false],
            crop_phase: &[1.0],
            vernalization_factor: &[1.0],
            base_temperature_c: &[0.0],
        },
        &mut southern,
    )
    .unwrap();
    close(southern.growing_degree_days_zero_c[0], 6.85 / 48.0);
}

#[test]
fn crop_climate_increments_active_years_only_when_the_native_timestamp_crosses_year_end() {
    let mut state = CropPhenologyClimateState::new(2);
    crop_phenology_climate_step(
        input(
            CalendarTime {
                year: 2007,
                julian_day: 365,
                seconds: 84_600,
            },
            &[280.0, 280.0],
            &[false, false],
        ),
        &mut state,
    )
    .unwrap();
    assert_eq!(state.active_crop_years, [0, 0]);

    crop_phenology_climate_step(
        input(
            CalendarTime {
                year: 2007,
                julian_day: 365,
                seconds: 86_400,
            },
            &[280.0, 280.0],
            &[false, false],
        ),
        &mut state,
    )
    .unwrap();
    assert_eq!(state.active_crop_years, [1, 1]);
}

#[test]
fn crop_climate_rejects_misaligned_state() {
    let mut state = CropPhenologyClimateState::new(1);
    assert!(crop_phenology_climate_step(
        input(
            CalendarTime {
                year: 2007,
                julian_day: 100,
                seconds: 1_800,
            },
            &[280.0, 280.0],
            &[false, false],
        ),
        &mut state,
    )
    .is_err());
}

fn lifecycle_input(time: CalendarTime, class: &[i32]) -> CropPhenologyInput<'_> {
    CropPhenologyInput {
        time,
        time_step_seconds: 1_800,
        pft_class: class,
        reference_temperature_k: &[280.0],
        leaf_carbon_to_nitrogen: &[30.0],
        leaf_longevity_years: &[1.0],
        leaf_emergence_heat_unit_index: &[0.2],
        grain_fill_heat_unit_index: &[0.8],
        maximum_maturity_days: &[180],
        total_leaf_area_index: &[0.0],
        use_fertilizer: false,
    }
}

#[test]
fn crop_lifecycle_plants_with_the_source_seed_transfer_and_corn_maturity_rule() {
    let mut climate = CropPhenologyClimateState::new(1);
    climate.growing_degree_days_eight_twenty_year_c[0] = 1_000.0;
    climate.growing_degree_days_since_planting_c[0] = 0.0;
    let mut state = CropPhenologyState::new(1);
    state.planting_day[0] = 120.0;
    crop_phenology_step(
        lifecycle_input(
            CalendarTime {
                year: 2007,
                julian_day: 120,
                seconds: 1_800,
            },
            &[17],
        ),
        &mut climate,
        &mut state,
    )
    .unwrap();

    assert!(state.crop_live[0]);
    assert!(state.crop_planted[0]);
    assert_eq!(state.day_of_planting[0], 120);
    assert_eq!(state.harvest_day[0], 999.0);
    close(state.leaf_carbon_transfer_g_m2[0], 3.0);
    close(state.leaf_nitrogen_transfer_g_m2[0], 0.1);
    close(state.crop_seed_carbon_to_leaf_g_m2_s[0], 3.0 / 1_800.0);
    close(state.crop_seed_nitrogen_to_leaf_g_m2_s[0], 0.1 / 1_800.0);
    close(state.growing_degree_days_at_maturity_c[0], 1_116.0);
    assert_eq!(state.heat_unit_index[0], 0.0);
    assert_eq!(state.crop_phase[0], 1.0);
}

#[test]
fn crop_lifecycle_uses_shared_gdd_to_emerge_fertilize_and_vernalize_winter_wheat() {
    let mut climate = CropPhenologyClimateState::new(1);
    climate.growing_degree_days_ten_twenty_year_c[0] = 1_000.0;
    climate.growing_degree_days_since_planting_c[0] = 200.0;
    let mut state = CropPhenologyState::new(1);
    state.crop_live[0] = true;
    state.crop_planted[0] = true;
    state.day_of_planting[0] = 100;
    state.harvest_day[0] = 999.0;
    state.cumulative_vernalization_days[0] = 0.0;
    state.vernalization_factor[0] = 0.0;
    state.manure_nitrogen_g_m2[0] = 1.0;
    state.fertilizer_nitrogen_g_m2[0] = 2.0;
    let mut input = lifecycle_input(
        CalendarTime {
            year: 2007,
            julian_day: 110,
            seconds: 1_800,
        },
        &[WINTER_WHEAT_CLASS],
    );
    input.reference_temperature_k = &[278.05];
    input.use_fertilizer = true;
    crop_phenology_step(input, &mut climate, &mut state).unwrap();

    close(state.growing_degree_days_at_maturity_c[0], 860.0);
    close(state.heat_unit_index[0], 200.0 / 860.0);
    assert_eq!(state.crop_phase[0], 2.0);
    assert_eq!(state.onset_flag[0], 1.0);
    assert_eq!(state.onset_counter_seconds[0], 1_800.0);
    assert!(state.cumulative_vernalization_days[0] > 0.0);
    assert!(state.vernalization_factor[0] > 0.0 && state.vernalization_factor[0] < 1.0);
    close(state.fertilizer_rate_g_m2_s[0], 3.0 / (20.0 * 86_400.0));
    close(
        state.fertilizer_counter_seconds[0],
        20.0 * 86_400.0 - 1_800.0,
    );
}

#[test]
fn crop_lifecycle_harvests_before_phase_three_and_reverses_unemerged_seed_flux() {
    let mut climate = CropPhenologyClimateState::new(1);
    climate.growing_degree_days_since_planting_c[0] = 816.0;
    let mut state = CropPhenologyState::new(1);
    state.crop_live[0] = true;
    state.crop_planted[0] = true;
    state.day_of_planting[0] = 100;
    state.harvest_day[0] = 99_999_999.0;
    state.leaf_carbon_transfer_g_m2[0] = 3.0;
    state.leaf_nitrogen_transfer_g_m2[0] = 0.1;
    state.crop_seed_carbon_to_leaf_g_m2_s[0] = 0.01;
    state.crop_seed_nitrogen_to_leaf_g_m2_s[0] = 0.001;
    crop_phenology_step(
        lifecycle_input(
            CalendarTime {
                year: 2007,
                julian_day: 121,
                seconds: 1_800,
            },
            &[17],
        ),
        &mut climate,
        &mut state,
    )
    .unwrap();

    assert!(!state.crop_live[0]);
    assert!(!state.crop_planted[0]);
    assert_eq!(state.harvest_day[0], 121.0);
    assert_eq!(state.crop_phase[0], 4.0);
    assert_eq!(state.heat_unit_index[0], 0.0);
    assert_eq!(state.offset_flag[0], 0.0);
    assert_eq!(state.leaf_carbon_transfer_g_m2[0], 0.0);
    assert_eq!(state.leaf_nitrogen_transfer_g_m2[0], 0.0);
    close(
        state.crop_seed_carbon_to_leaf_g_m2_s[0],
        0.01 - 3.0 / 1_800.0,
    );
    close(
        state.crop_seed_nitrogen_to_leaf_g_m2_s[0],
        0.001 - 0.1 / 1_800.0,
    );
}
