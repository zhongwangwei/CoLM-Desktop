use super::*;

#[test]
fn lucy_uses_weekday_or_weekend_profiles_in_fortran_calendar_order() {
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
    metabolism[23] = 4.0;
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
