use super::*;

fn settings() -> IrrigationSettings {
    IrrigationSettings {
        start_seconds: 21_600.0,
        duration_seconds: 14_400.0,
        max_depth_m: 1.0,
        threshold_fraction: 1.0,
        supply_fraction: 1.0,
        min_crop_phase: 1.0,
        max_crop_phase: 4.0,
        paddy_ponding_limit_mm: 100.0,
        allocation: 1,
        variably_saturated_flow: true,
        campbell: true,
        greenwich: false,
    }
}

#[test]
fn application_consumes_storage_and_preserves_source_method_routing() {
    let mut state = IrrigationState {
        methods: vec![IRRIGATION_FLOOD],
        rate_mm_s: 2.0,
        water_storage_mm: 3.0,
        steps_left: 2,
        ..IrrigationState::default()
    };
    let first = state.application_fluxes(1.0);
    assert_eq!(first.flood_mm_s, 2.0);
    assert_eq!(state.steps_left, 1);
    assert_eq!(state.water_storage_mm, 1.0);

    state.methods = vec![IRRIGATION_PADDY];
    let second = state.application_fluxes(1.0);
    assert_eq!(second.paddy_mm_s, 1.0);
    assert_eq!(state.steps_left, 0);
    assert_eq!(state.water_storage_mm, 0.0);

    state.methods = vec![IRRIGATION_DRIP];
    let third = state.application_fluxes(1.0);
    assert_eq!(third.drip_mm_s, 0.0);
    assert_eq!(state.rate_mm_s, 0.0);
}

/// 一个 3 层、全在冰点之上、干到阈值以下的土柱：`idate(3) = 18000`（`0 <= idate(3) - 21600 + 3600 < 3600`）触发灌溉。
fn run(
    methods: Vec<i32>,
    classes: &[i32],
    settings: IrrigationSettings,
    wliq: &mut [f64],
    zwt: &mut f64,
    wa: &mut f64,
) -> IrrigationState {
    let mut state = IrrigationState {
        methods,
        zwt_stand_m: 3.0,
        groundwater_allocation: 0.5,
        ..IrrigationState::default()
    };
    let three = |value: f64| [value; 3];
    let (dz, z, zi) = ([0.1, 0.2, 0.3], [0.05, 0.2, 0.45], [0.1, 0.3, 0.6]);
    let (t, porosity, residual) = (three(290.0), three(0.45), three(0.05));
    let (psi0, bsw) = (three(-200.0), three(5.0));
    let (alpha, n, l, sc, fc) = (three(0.01), three(1.5), three(0.5), three(1.0), three(1.0));
    irrigation_needed(
        &mut state,
        settings,
        IrrigationColumn {
            idate: [2002, 150, 18_000],
            time_step_seconds: 3600.0,
            longitude_deg: -96.0,
            pft_class: classes,
            crop_phase: &vec![2.0; classes.len()],
            node_depth_m: &z,
            layer_thickness_m: &dz,
            interface_depth_m: &zi,
            temperature_k: &t,
            porosity: &porosity,
            residual_water: &residual,
            saturated_potential_mm: &psi0,
            clapp_hornberger_b: &bsw,
            alpha_vgm: &alpha,
            n_vgm: &n,
            l_vgm: &l,
            sc_vgm: &sc,
            fc_vgm: &fc,
            liquid_water_kg_m2: wliq,
            water_table_depth_m: zwt,
            aquifer_water_mm: wa,
        },
    )
    .unwrap();
    state
}

#[test]
fn a_dry_column_schedules_the_field_capacity_deficit_over_the_duration() {
    let (mut wliq, mut zwt, mut wa) = ([5.0, 10.0, 15.0], 2.0, 4000.0);
    let state = run(vec![IRRIGATION_DRIP], &[18], settings(), &mut wliq, &mut zwt, &mut wa);
    assert!(state.deficit_mm > 0.0);
    assert_eq!(state.actual_mm, state.deficit_mm);
    assert_eq!(state.water_storage_mm, state.deficit_mm);
    assert_eq!(state.steps_left, 4);
    assert_eq!(state.rate_mm_s, state.actual_mm / 3600.0 / 4.0);
    assert_eq!((state.sum_mm, state.sum_count), (state.actual_mm, 1.0));
}

#[test]
fn rainfed_classes_and_other_hours_are_not_irrigated_but_rice_flood_becomes_paddy() {
    let (mut wliq, mut zwt, mut wa) = ([5.0, 10.0, 15.0], 2.0, 4000.0);
    let rainfed = run(vec![IRRIGATION_DRIP], &[17], settings(), &mut wliq, &mut zwt, &mut wa);
    assert_eq!((rainfed.deficit_mm, rainfed.steps_left), (0.0, 0));
    let mut later = settings();
    later.start_seconds = 25_200.0;
    let off_hour = run(vec![IRRIGATION_DRIP], &[18], later, &mut wliq, &mut zwt, &mut wa);
    assert_eq!(off_hour.steps_left, 0);
    let rice = run(vec![IRRIGATION_FLOOD], &[62], settings(), &mut wliq, &mut zwt, &mut wa);
    assert_eq!(rice.methods, vec![IRRIGATION_PADDY]);
}

#[test]
fn campbell_groundwater_withdrawal_keeps_the_column_water_balance() {
    let mut allocation = settings();
    allocation.allocation = 2;
    allocation.variably_saturated_flow = false;
    let (mut wliq, mut zwt, mut wa) = ([5.0, 10.0, 15.0], 0.4, 4000.0);
    let before = wliq.iter().sum::<f64>() + wa;
    let state = run(vec![IRRIGATION_DRIP], &[18], allocation, &mut wliq, &mut zwt, &mut wa);
    assert!(state.groundwater_supply_mm > 0.0);
    assert_eq!(state.actual_mm, state.groundwater_supply_mm);
    let after = wliq.iter().sum::<f64>() + wa;
    assert!((before - after - state.groundwater_supply_mm).abs() < 1.0e-9);
    assert!(zwt > 0.4);
}
