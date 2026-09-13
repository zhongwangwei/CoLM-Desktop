use super::*;

#[test]
fn scheduling_converts_irrigated_rice_flooding_to_paddy_at_local_start_time() {
    let mut crop = CropPhenologyState::new(1);
    crop.crop_phase[0] = 2.0;
    let mut method = [IRRIGATION_FLOOD];
    let scheduled = irrigation_is_scheduled(
        IrrigationScheduleInput {
            time: CalendarTime {
                year: 2007,
                julian_day: 120,
                seconds: 0,
            },
            time_step_seconds: 1_800,
            longitude_degrees: 15.0,
            greenwich_time: true,
            start_seconds: 5_400,
            minimum_crop_phase: 2.0,
            maximum_crop_phase: 4.0,
            pft_class: &[62],
        },
        &crop,
        &mut method,
    )
    .unwrap();
    assert!(scheduled);
    assert_eq!(method, [IRRIGATION_PADDY]);
}

#[test]
fn scheduling_keeps_the_source_last_pft_result() {
    let mut crop = CropPhenologyState::new(2);
    crop.crop_phase = vec![2.0, 4.0];
    let mut method = [IRRIGATION_SPRINKLER, IRRIGATION_SPRINKLER];
    assert!(!irrigation_is_scheduled(
        IrrigationScheduleInput {
            time: CalendarTime {
                year: 2007,
                julian_day: 120,
                seconds: 5_400,
            },
            time_step_seconds: 1_800,
            longitude_degrees: 0.0,
            greenwich_time: false,
            start_seconds: 7_200,
            minimum_crop_phase: 2.0,
            maximum_crop_phase: 4.0,
            pft_class: &[18, 20],
        },
        &crop,
        &mut method,
    )
    .unwrap());
}

#[test]
fn application_consumes_storage_and_preserves_source_method_routing() {
    let mut state = IrrigationApplicationState {
        rate_mm_s: 2.0,
        water_storage_mm: 3.0,
        steps_left: 2,
    };
    let first = irrigation_application_fluxes(1, &[IRRIGATION_FLOOD], &mut state).unwrap();
    assert_eq!(first.flood_mm_s, 2.0);
    assert_eq!(state.steps_left, 1);
    assert_eq!(state.water_storage_mm, 1.0);

    let second = irrigation_application_fluxes(1, &[IRRIGATION_PADDY], &mut state).unwrap();
    assert_eq!(second.paddy_mm_s, 1.0);
    assert_eq!(state.steps_left, 0);
    assert_eq!(state.water_storage_mm, 0.0);

    let third = irrigation_application_fluxes(1, &[IRRIGATION_DRIP], &mut state).unwrap();
    assert_eq!(third.drip_mm_s, 0.0);
    assert_eq!(state.rate_mm_s, 0.0);
}
