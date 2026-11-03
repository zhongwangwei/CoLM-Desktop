use super::*;

#[test]
fn lucy_selects_weekday_and_fixed_holiday_profiles() {
    let mut input = input();
    input.fixed_holiday = &[0.0; 365];
    input.weekday_traffic_profile = &[0.1; 24];
    input.weekend_traffic_profile = &[0.2; 24];
    let weekday = urban_lucy_flux(input).unwrap();
    close(weekday.metabolic_heat, 0.002);
    close(weekday.vehicle_heat, 0.033125);
    close(weekday.anthropogenic_heat, 0.035125);

    input.fixed_holiday = &[1.0; 365];
    input.week_holiday = &[1.0; 7];
    let weekend = urban_lucy_flux(input).unwrap();
    close(weekend.vehicle_heat, 0.06625);
    close(weekend.anthropogenic_heat, 0.06825);
}

#[test]
fn lucy_converts_greenwich_time_before_selecting_the_hour() {
    let mut input = input();
    input.time.seconds = 43_200;
    input.greenwich = true;
    input.longitude_radians = std::f64::consts::PI;
    input.fixed_holiday = &[0.0; 365];
    let mut metabolism = [0.0; 24];
    metabolism[0] = 4.0;
    input.human_metabolic_profile = &metabolism;
    let fluxes = urban_lucy_flux(input).unwrap();
    close(fluxes.metabolic_heat, 0.004);
}

#[test]
fn lucy_rejects_bad_restart_shapes_and_negative_vehicle_counts() {
    let mut invalid = input();
    invalid.week_holiday = &[0.0; 6];
    assert!(urban_lucy_flux(invalid).is_err());
    let mut invalid = input();
    invalid.vehicles_per_thousand = &[1.0, -1.0, 1.0];
    assert!(urban_lucy_flux(invalid).is_err());
}

fn input() -> UrbanLucyFluxInput<'static> {
    UrbanLucyFluxInput {
        time: CalendarTime {
            year: 2024,
            julian_day: 1,
            seconds: 3600,
        },
        greenwich: false,
        longitude_radians: 0.0,
        fixed_holiday: &[0.0; 365],
        week_holiday: &[0.0; 7],
        human_metabolic_profile: &[2.0; 24],
        weekday_traffic_profile: &[0.1; 24],
        weekend_traffic_profile: &[0.2; 24],
        population_density_per_km2: 1000.0,
        vehicles_per_thousand: &[1.0, 2.0, 3.0],
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-14,
        "{actual} != {expected}"
    );
}

#[test]
fn local_clock_rounds_time_zones_and_rolls_midnight_into_the_next_year() {
    let time = CalendarTime {
        year: 2024,
        julian_day: 366,
        seconds: 57_600,
    };
    let midnight = local_time(
        CalendarTime {
            seconds: 86_400,
            ..time
        },
        false,
        0.0,
    )
    .unwrap();
    assert_eq!(
        (midnight.year, midnight.julian_day, midnight.seconds),
        (2025, 1, 0)
    );
    let local = local_time(time, true, 113.5_f64.to_radians()).unwrap();
    assert_eq!((local.year, local.julian_day, local.seconds), (2025, 1, 0));
    for (longitude, seconds) in [
        (113.5_f64, 28_800),
        (-113.5, 57_600),
        (7.6, 3600),
        (-7.6, 82_800),
    ] {
        let local = local_time(
            CalendarTime {
                year: 2024,
                julian_day: 1,
                seconds: 0,
            },
            true,
            longitude.to_radians(),
        )
        .unwrap();
        assert_eq!(local.seconds, seconds);
    }
}

#[test]
fn lucy_selects_the_fortran_floor_hour_at_the_step_start() {
    let mut input = input();
    let profile = std::array::from_fn::<_, 24, _>(|index| (index + 1) as f64);
    input.human_metabolic_profile = &profile;
    for (seconds, index) in [(0, 0), (3599, 0), (3600, 1), (86_399, 23), (86_400, 0)] {
        input.time.seconds = seconds;
        close(
            urban_lucy_flux(input).unwrap().metabolic_heat,
            (index + 1) as f64 / 1000.0,
        );
    }
}

#[test]
fn anthropogenic_heat_step_start_normalizes_local_and_utc_year_boundaries() {
    let end = CalendarTime {
        year: 2025,
        julian_day: 1,
        seconds: 0,
    };
    assert_eq!(
        urban_local_step_start(end, 1800.0, false, 113.5_f64.to_radians()).unwrap(),
        CalendarTime {
            year: 2024,
            julian_day: 366,
            seconds: 84_600
        }
    );
    assert_eq!(
        urban_local_step_start(end, 1800.0, true, 113.5_f64.to_radians()).unwrap(),
        CalendarTime {
            year: 2025,
            julian_day: 1,
            seconds: 27_000
        }
    );
    for step in [f64::NAN, 0.0, -1800.0, 86_401.0] {
        assert!(urban_local_step_start(end, step, false, 0.0).is_err());
    }
}

#[test]
fn lucy_clock_integration_matches_the_executable_fortran_reference() {
    // Checked by oracle/scripts/test_urban_bem.py against the current LUCY routine.
    let expected: [[f64; 3]; 6] = [
        [0.1035, 0.0795, 0.024],
        [0.0345, 0.0265, 0.008],
        [0.0043125, 0.0033125, 0.001],
        [0.0733125, 0.0563125, 0.017],
        [0.1035, 0.0795, 0.024],
        [0.1035, 0.0795, 0.024],
    ];
    let human = std::array::from_fn::<_, 24, _>(|index| (index + 1) as f64);
    let weekday = human.map(|value| value / 100.0);
    for (index, expected) in expected.into_iter().enumerate() {
        let case = index + 1;
        let seconds = match case {
            3 | 4 => 3600,
            5 => 86_400,
            6 => 43_200,
            _ => 0,
        };
        let longitude = match case {
            2 => 113.5_f64.to_radians(),
            4 => -113.5_f64.to_radians(),
            6 => std::f64::consts::PI,
            _ => 0.0,
        };
        let time = urban_local_step_start(
            CalendarTime {
                year: 2025,
                julian_day: 1,
                seconds,
            },
            1800.0,
            matches!(case, 2 | 4 | 6),
            longitude,
        )
        .unwrap();
        let mut sample = input();
        sample.time = time;
        sample.fixed_holiday = &[0.0; 365];
        sample.week_holiday = &[0.0; 7];
        sample.human_metabolic_profile = &human;
        sample.weekday_traffic_profile = &weekday;
        sample.weekend_traffic_profile = &weekday;
        let actual = urban_lucy_flux(sample).unwrap();
        for (actual, expected) in [
            actual.anthropogenic_heat,
            actual.vehicle_heat,
            actual.metabolic_heat,
        ]
        .into_iter()
        .zip(expected)
        {
            close(actual, expected);
        }
    }
}
