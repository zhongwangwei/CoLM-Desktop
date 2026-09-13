use super::*;

fn time(year: i32, julian_day: u16, seconds: u32) -> CalendarTime {
    CalendarTime {
        year,
        julian_day,
        seconds,
    }
}

#[test]
fn clock_preserves_colms_beginning_forcing_and_end_driver_boundaries() {
    let steps: Vec<_> = std::iter::from_fn({
        let mut clock = RuntimeClock::new(
            time(2008, 1, 82_800),
            time(2008, 2, 3_600),
            time(2008, 1, 82_800),
            3_600.0,
            1,
        )
        .unwrap();
        move || clock.next_step()
    })
    .collect();

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].forcing_time, time(2008, 1, 82_800));
    // `TICKTIME` retains 86400; `adj2begin` is only applied for next forcing.
    assert_eq!(steps[0].end_time, time(2008, 1, 86_400));
    assert_eq!(steps[1].forcing_time, time(2008, 2, 0));
    assert_eq!(steps[1].end_time, time(2008, 2, 3_600));
}

#[test]
fn spinup_restarts_only_until_its_last_cycle_then_continues() {
    let steps: Vec<_> = std::iter::from_fn({
        let mut clock = RuntimeClock::new(
            time(2008, 1, 0),
            time(2008, 1, 14_400),
            time(2008, 1, 7_200),
            3_600.0,
            2,
        )
        .unwrap();
        move || clock.next_step()
    })
    .collect();

    assert_eq!(steps.len(), 6);
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.forcing_time.seconds, step.is_spinup, step.spinup_cycle))
            .collect::<Vec<_>>(),
        vec![
            (0, true, 1),
            (3_600, true, 1),
            (0, true, 2),
            (3_600, true, 2),
            (7_200, false, 2),
            (10_800, false, 2),
        ]
    );
}

#[test]
fn clock_matches_fortran_nint_and_rejects_an_invalid_window() {
    let mut clock = RuntimeClock::new(
        time(2007, 365, 86_399),
        time(2008, 1, 2),
        time(2007, 365, 86_399),
        1.5,
        0,
    )
    .unwrap();
    assert_eq!(clock.next_step().unwrap().end_time, time(2008, 1, 1));
    assert!(
        RuntimeClock::new(time(2008, 2, 0), time(2008, 1, 0), time(2008, 1, 0), 1.0, 1,).is_err()
    );
    assert!(RuntimeClock::new(
        time(2008, 1, 0),
        time(2008, 1, 3_600),
        time(2008, 1, 0),
        3_601.0,
        1,
    )
    .is_err());
    assert!(RuntimeClock::new(
        time(2008, 1, 0),
        time(2008, 1, 3_600),
        time(2008, 1, 0),
        0.5,
        1,
    )
    .is_err());
}

#[test]
fn clock_uses_fortran_int_for_loop_progress_and_nint_for_driver_time() {
    let steps: Vec<_> = std::iter::from_fn({
        let mut clock =
            RuntimeClock::new(time(2008, 1, 0), time(2008, 1, 3), time(2008, 1, 0), 1.5, 1)
                .unwrap();
        move || clock.next_step()
    })
    .collect();
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.forcing_time.seconds, step.end_time.seconds))
            .collect::<Vec<_>>(),
        vec![(0, 2), (2, 4), (4, 6)]
    );
}

#[test]
fn clock_carries_colms_lai_update_flags_into_each_driver_step() {
    let monthly: Vec<_> = std::iter::from_fn({
        let mut clock = RuntimeClock::new(
            time(2008, 1, 0),
            time(2008, 1, 7_200),
            time(2008, 1, 0),
            3_600.0,
            1,
        )
        .unwrap();
        move || clock.next_step()
    })
    .collect();
    assert_eq!(
        monthly
            .iter()
            .map(|step| (step.update_lai, step.update_albedo, step.update_sst))
            .collect::<Vec<_>>(),
        vec![(true, true, false), (false, true, false)]
    );

    let eight_day: Vec<_> = std::iter::from_fn({
        let mut clock = RuntimeClock::with_lai_update_schedule(
            time(2008, 8, 82_800),
            time(2008, 9, 7_200),
            time(2008, 8, 82_800),
            3_600.0,
            1,
            LaiUpdateSchedule::EightDay,
        )
        .unwrap();
        move || clock.next_step()
    })
    .collect();
    assert_eq!(
        eight_day
            .iter()
            .map(|step| step.update_lai)
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
}

#[test]
fn clock_matches_colms_restart_cadence_spinup_gate_and_final_write() {
    let scheduled: Vec<_> = std::iter::from_fn({
        let mut clock = RuntimeClock::new(
            time(2008, 1, 0),
            time(2008, 1, 7_200),
            time(2008, 1, 7_200),
            3_600.0,
            1,
        )
        .unwrap()
        .with_restart_frequency(RestartFrequency::Hourly);
        move || clock.next_step()
    })
    .collect();
    assert_eq!(
        scheduled
            .iter()
            .map(|step| step.write_restart)
            .collect::<Vec<_>>(),
        vec![false, true]
    );

    let final_only: Vec<_> = std::iter::from_fn({
        let mut clock = RuntimeClock::new(
            time(2008, 1, 0),
            time(2008, 1, 3_600),
            time(2008, 1, 0),
            3_600.0,
            1,
        )
        .unwrap()
        .with_restart_frequency(RestartFrequency::Never);
        move || clock.next_step()
    })
    .collect();
    assert_eq!(final_only.len(), 1);
    assert!(final_only[0].write_restart);
}
