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
}
