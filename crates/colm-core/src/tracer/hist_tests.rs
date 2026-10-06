//! `MOD_Tracer_Hist` 的测试：累加、变量表与单点/网格取值规则。

use super::*;
use crate::tracer::{ReactionMode, StateOwner, TracerFamily};

fn descriptor(family: TracerFamily, name: &str, unit_kind: &str) -> TracerDescriptor {
    let isotope = family == TracerFamily::Isotope;
    TracerDescriptor {
        name: name.to_owned(),
        category: if isotope { "isotope" } else { "solute" }.to_owned(),
        family,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: unit_kind.to_owned(),
        mol_weight: 18.0,
        ref_ratio: if isotope { 2.0052e-3 } else { 1.0 },
        init_delta: 0.0,
        init_conc: 0.0,
        precip_default_conc: 0.0,
        vapor_default_conc: 0.0,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        charge: 0,
    }
}

fn two_tracers() -> TracerSet {
    TracerSet {
        tracers: vec![
            descriptor(TracerFamily::Isotope, "HDO", "ratio"),
            descriptor(TracerFamily::Solute, "Cl", "tracer_per_water"),
        ],
    }
}

#[test]
fn accumulate_routes_layers_aquifer_and_thin_snow() {
    let set = two_tracers();
    let mut state = PatchTracerState::allocated(&set);
    let mut wliq = [0.0; SOISNO_LAYERS];
    let mut wice = [0.0; SOISNO_LAYERS];
    wliq[soisno_slot(0)] = 1.0;
    wice[soisno_slot(0)] = 2.0;
    wliq[soisno_slot(1)] = 10.0;
    // 第 2 层干：非挥发溶质计入 layer_dry，同位素仍计入层。
    wliq[soisno_slot(3)] = 4.0;
    for itrc in 0..2 {
        let pools = &mut state.pools[itrc];
        pools.wliq_soisno[soisno_slot(0)] = 0.1;
        pools.wice_soisno[soisno_slot(0)] = 0.2;
        pools.wliq_soisno[soisno_slot(1)] = 0.5;
        pools.wliq_soisno[soisno_slot(2)] = 0.25;
        pools.wa = -3.0;
        pools.solid_soisno[soisno_slot(-3)] = 9.0; // snl = -1：不计
        pools.solid_soisno[soisno_slot(0)] = 1.0;
        pools.canopy_solid = 0.5;
        pools.scv = 7.0;
    }
    let input = HistAccumulateInput {
        snl: -1,
        ldew_rain: 0.3,
        ldew_snow: 0.4,
        wliq_soisno: &wliq,
        wice_soisno: &wice,
        wa: -30.0,
        wdsrf: 0.0,
        wetwat: 0.0,
        scv: 5.0,
    };
    tracer_hist_accumulate(&set, &mut state, &input);
    assert_eq!(state.water_acc.ldew, 0.3 + 0.4);
    assert_eq!(state.water_acc.snow[soisno_slot(0)], (0.0 + 1.0) + 2.0);
    assert_eq!(state.water_acc.soil[0], 10.0);
    assert_eq!(state.water_acc.wa, 0.0);
    assert_eq!(state.water_acc.wa_debt, 30.0);
    // 有雪层时 scv 不与薄雪示踪物配对。
    assert_eq!(state.water_acc.scv, 0.0);
    for itrc in 0..2 {
        let acc = &state.acc[itrc];
        assert_eq!(acc.snow_mass[soisno_slot(0)], 0.1 + 0.2);
        assert_eq!(acc.soil_mass[0], 0.5);
        assert_eq!(acc.wa_debt_mass, 3.0);
        assert_eq!(acc.wa_mass, 0.0);
        assert_eq!(acc.scv_mass, 0.0);
        assert_eq!(acc.solid_mass, (((0.0 + 0.5) + 0.0) + 0.0) + 0.0 + 1.0);
    }
    assert_eq!(state.acc[0].soil_mass[1], 0.25);
    assert_eq!(state.acc[0].layer_dry_mass, 0.0);
    assert_eq!(state.acc[1].soil_mass[1], 0.0);
    assert_eq!(state.acc[1].layer_dry_mass, 0.25);
}

#[test]
fn accumulate_isotope_aquifer_reference() {
    let set = two_tracers();
    let mut state = PatchTracerState::allocated(&set);
    state.aquifer_ref_water = 500.0;
    state.pools[0].wa = -0.2;
    state.pools[0].aquifer_ref_mass = 1.0;
    state.pools[1].wa = 3.0;
    let zeros = [0.0; SOISNO_LAYERS];
    let input = HistAccumulateInput {
        snl: 0,
        ldew_rain: 0.0,
        ldew_snow: 0.0,
        wliq_soisno: &zeros,
        wice_soisno: &zeros,
        wa: -100.0,
        wdsrf: 0.0,
        wetwat: 0.0,
        scv: 1.0e-31,
    };
    tracer_hist_accumulate(&set, &mut state, &input);
    let actual_water = -100.0 + 500.0;
    let actual_mass = -0.2 + 1.0;
    assert_eq!(state.water_acc.aquifer_actual, actual_water);
    assert_eq!(state.water_acc.wa_debt, 100.0);
    assert_eq!(state.acc[0].aquifer_actual_mass, actual_mass);
    assert_eq!(
        state.acc[0].wa_debt_mass,
        0.0 - -100.0 * actual_mass / actual_water
    );
    // 溶质走普通分支：wa < -1 记亏欠 max(-trc_wa, 0)。
    assert_eq!(state.acc[1].wa_debt_mass, 0.0);
    assert_eq!(state.acc[1].aquifer_actual_mass, 0.0);
    // scv <= tiny：不累加。
    assert_eq!(state.water_acc.scv, 0.0);
}

#[test]
fn variable_table_names_units_and_scopes() {
    assert_eq!(TRACER_HISTORY_VARIABLES.len(), 28);
    let set = two_tracers();
    let (hdo, cl) = (&set.tracers[0], &set.tracers[1]);
    let names = |tracer: &TracerDescriptor| -> Vec<String> {
        TRACER_HISTORY_VARIABLES
            .iter()
            .filter(|variable| variable.applies_to(tracer))
            .map(|variable| variable.variable_name(tracer))
            .collect()
    };
    assert_eq!(
        names(hdo),
        [
            "f_trc_delta_precip_HDO",
            "f_trc_delta_runoff_HDO",
            "f_trc_delta_evap_HDO",
            "f_trc_evap_mass_HDO",
            "f_trc_delta_soilevap_HDO",
            "f_trc_delta_canopyevap_HDO",
            "f_trc_delta_subl_HDO",
            "f_trc_delta_wetland_evap_HDO",
            "f_trc_delta_transp_HDO",
            "f_trc_delta_transp_src_HDO",
            "f_trc_leaf_delta_e_HDO",
            "f_trc_leaf_delta_b_HDO",
            "f_trc_vapor_exchange_HDO",
            "f_trc_conc_ldew_HDO",
            "f_trc_conc_soisno_HDO",
            "f_trc_conc_snowpack_HDO",
            "f_trc_delta_snowpack_HDO",
            "f_trc_conc_wa_HDO",
            "f_trc_conc_wa_debt_HDO",
            "f_trc_conc_wdsrf_HDO",
            "f_trc_conc_wetwat_HDO",
            "f_trc_conc_scv_HDO",
        ]
    );
    assert_eq!(
        names(cl),
        [
            "f_trc_conc_precip_Cl",
            "f_trc_conc_runoff_Cl",
            "f_trc_surface_residue_Cl",
            "f_trc_subsurface_residue_Cl",
            "f_trc_layer_dry_inventory_Cl",
            "f_trc_solid_inventory_Cl",
            "f_trc_conc_ldew_Cl",
            "f_trc_conc_soisno_Cl",
            "f_trc_conc_snowpack_Cl",
            "f_trc_conc_wa_Cl",
            "f_trc_conc_wa_debt_Cl",
            "f_trc_conc_wdsrf_Cl",
            "f_trc_conc_wetwat_Cl",
            "f_trc_conc_scv_Cl",
        ]
    );
    let ldew = history_variable(TracerHistId::ConcLdew);
    assert_eq!(ldew.long_name(hdo), "canopy tracer heavy/total ratio (HDO)");
    assert_eq!(ldew.long_name(cl), "canopy tracer concentration (Cl)");
    assert_eq!(ldew.units(hdo), "R");
    assert_eq!(ldew.units(cl), "tracer/water");
    let precip = history_variable(TracerHistId::DeltaPrecip);
    assert!(precip.enabled(|key| key == "xy_prl"));
    assert!(!precip.enabled(|key| key == "rnof"));
    assert_eq!(
        precip.long_name(hdo),
        "precipitation plus dew/frost deposition tracer delta (HDO)"
    );
    // 露霜不带不挥发溶质：Cl 的名字里写明。
    let conc = history_variable(TracerHistId::ConcPrecip);
    assert_eq!(
        conc.long_name(cl),
        "precipitation plus dew/frost deposition tracer concentration, dew/frost adding water only (Cl)"
    );
    assert_eq!(
        history_variable(TracerHistId::ConcSoisno).dims,
        TracerHistDims::SoilSnow
    );
    assert_eq!(
        history_variable(TracerHistId::ConcWetwat).patch_filter,
        PatchFilter::Wetland
    );
    let mut mass_fraction = cl.clone();
    mass_fraction.unit_kind = "mass_fraction".to_owned();
    assert_eq!(ldew.units(&mass_fraction), "kg/kg water");
}

#[test]
fn single_point_thresholds_and_missing_values() {
    let set = two_tracers();
    let (hdo, cl) = (&set.tracers[0], &set.tracers[1]);
    let mut state = PatchTracerState::allocated(&set);
    let nac = 48.0;
    // 通量 δ 要求累计水量 > 0.1。
    state.acc[0].precip = 0.05 * 2.0052e-3;
    state.acc[0].water_precip = 0.05;
    let precip = history_variable(TracerHistId::DeltaPrecip);
    assert_eq!(single_point_value(precip, hdo, 0, &state, nac, true), SPVAL);
    state.acc[0].precip = 2.0 * 2.0052e-3 * 0.9;
    state.acc[0].water_precip = 2.0;
    let expected = ((state.acc[0].precip / 2.0) / 2.0052e-3 - 1.0) * 1000.0;
    assert_eq!(
        single_point_value(precip, hdo, 0, &state, nac, true),
        expected
    );
    assert_eq!(
        single_point_value(precip, hdo, 0, &state, nac, false),
        SPVAL
    );
    // |δ| > 2000 缺测。
    state.acc[0].precip = 2.0 * 2.0052e-3 * 4.0;
    assert_eq!(single_point_value(precip, hdo, 0, &state, nac, true), SPVAL);

    // 比值门槛只是 |water| > 1e-30：0 水量即缺测。
    let wdsrf = history_variable(TracerHistId::ConcWdsrf);
    assert_eq!(single_point_value(wdsrf, cl, 1, &state, nac, true), SPVAL);
    state.acc[1].wdsrf_mass = 3.0;
    state.water_acc.wdsrf = 12.0;
    assert_eq!(single_point_value(wdsrf, cl, 1, &state, nac, true), 0.25);

    // 质量变量除以 nac。
    state.acc[1].solid_mass = 96.0;
    let solid = history_variable(TracerHistId::SolidInventory);
    assert_eq!(single_point_value(solid, cl, 1, &state, nac, true), 2.0);

    // 叶片 e 位点：状态量本身，越界缺测。
    state.pools[0].leaf_delta_e = -12.5;
    let leaf_e = history_variable(TracerHistId::LeafDeltaE);
    assert_eq!(single_point_value(leaf_e, hdo, 0, &state, nac, true), -12.5);
    state.pools[0].leaf_delta_e = 2500.0;
    assert_eq!(single_point_value(leaf_e, hdo, 0, &state, nac, true), SPVAL);
    // 叶水为 0 时 δ_b 的伪水量为 0：缺测。
    let leaf_b = history_variable(TracerHistId::LeafDeltaB);
    assert_eq!(single_point_value(leaf_b, hdo, 0, &state, nac, true), SPVAL);

    // 湿地池只在 patchtype == 2 时有值。
    assert!(!PatchFilter::Wetland.admits(0, true, true));
    assert!(PatchFilter::Wetland.admits(2, true, true));
    assert!(PatchFilter::Land.admits(4, true, true));
    assert!(!PatchFilter::Land.admits(99, true, true));
}

#[test]
fn single_point_soisno_prescales_by_nac() {
    let set = two_tracers();
    let mut state = PatchTracerState::allocated(&set);
    let nac = 3.0;
    state.water_acc.soil[0] = 30.0;
    state.acc[1].soil_mass[0] = 0.7;
    state.water_acc.snow[soisno_slot(0)] = 1.0e-31;
    state.acc[1].snow_mass[soisno_slot(0)] = 0.1;
    let values = single_point_soisno(1, &state, nac, true);
    assert_eq!(values[soisno_slot(1)], 0.7 / 30.0 * nac / nac);
    assert_eq!(values[soisno_slot(0)], SPVAL);
    assert_eq!(values[soisno_slot(2)], SPVAL);
    assert_eq!(
        single_point_soisno(1, &state, nac, false),
        [SPVAL; SOISNO_LAYERS]
    );
}

#[test]
fn snowpack_combines_thin_snow_and_layers() {
    let set = two_tracers();
    let cl = &set.tracers[1];
    let mut state = PatchTracerState::allocated(&set);
    state.acc[1].scv_mass = 0.5;
    state.water_acc.scv = 2.0;
    state.acc[1].snow_mass[soisno_slot(-1)] = 0.25;
    state.water_acc.snow[soisno_slot(-1)] = 3.0;
    let snowpack = history_variable(TracerHistId::ConcSnowpack);
    assert_eq!(
        patch_term(snowpack, cl, 1, &state, 1.0),
        PatchTerm::Pair {
            mass: 0.5 + 0.25,
            water: 5.0
        }
    );
    assert_eq!(
        single_point_value(snowpack, cl, 1, &state, 1.0, true),
        0.75 / 5.0
    );
}

#[test]
fn gridded_aggregation_sums_mass_and_water_before_dividing() {
    let set = two_tracers();
    let cl = &set.tracers[1];
    let ldew = history_variable(TracerHistId::ConcLdew);
    let mut cell = GridCell::default();
    cell.add(
        ldew,
        PatchTerm::Pair {
            mass: 1.0,
            water: 10.0,
        },
        0.25,
        true,
    );
    // 水量 0 的 patch 以 (0, 0) 入图，不改变结果。
    cell.add(
        ldew,
        PatchTerm::Pair {
            mass: 5.0,
            water: 0.0,
        },
        0.5,
        true,
    );
    cell.add(
        ldew,
        PatchTerm::Pair {
            mass: 3.0,
            water: 1.0,
        },
        0.25,
        true,
    );
    // 未过滤的 patch 不入图。
    cell.add(
        ldew,
        PatchTerm::Pair {
            mass: 100.0,
            water: 1.0,
        },
        0.5,
        false,
    );
    let mass = (1.0 * 0.25 + 0.0 * 0.5) + 3.0 * 0.25;
    let water = (10.0 * 0.25 + 0.0 * 0.5) + 1.0 * 0.25;
    assert_eq!(cell.finish(ldew, cl.ref_ratio), mass / water);
    assert_eq!(GridCell::default().finish(ldew, cl.ref_ratio), SPVAL);

    // 质量变量：Σ area*v / sumarea，sumarea 太小时缺测。
    let solid = history_variable(TracerHistId::SolidInventory);
    let mut cell = GridCell::default();
    cell.add_area(0.6);
    cell.add(solid, PatchTerm::Scalar(2.0), 0.6, true);
    cell.add_area(0.4);
    cell.add(solid, PatchTerm::Scalar(SPVAL), 0.4, true);
    assert_eq!(cell.finish(solid, 1.0), (2.0 * 0.6) / (0.6 + 0.4));
    let mut tiny = GridCell::default();
    tiny.add_area(5.0e-6);
    tiny.add(solid, PatchTerm::Scalar(2.0), 5.0e-6, true);
    assert_eq!(tiny.finish(solid, 1.0), SPVAL);
    let mut empty = GridCell::default();
    empty.add_area(1.0);
    assert_eq!(empty.finish(solid, 1.0), SPVAL);

    // 三维：不合格层不入图。
    let soisno = history_variable(TracerHistId::ConcSoisno);
    let mut layer = GridCell::default();
    layer.add_layer(SPVAL, SPVAL, 0.5, true);
    assert_eq!(layer.finish(soisno, 1.0), SPVAL);
    layer.add_layer(2.0, 4.0, 0.5, true);
    assert_eq!(layer.finish(soisno, 1.0), (2.0 * 0.5) / (4.0 * 0.5));
}

#[test]
fn mass_to_delta_branches() {
    assert_eq!(mass_to_delta(1.0, 0.0, 1.0), 0.0);
    assert_eq!(mass_to_delta(-1.0, 1.0, 1.0), SPVAL);
    assert_eq!(
        mass_to_delta(1.1, 1.0, 1.0),
        ((1.1 / 1.0) / 1.0 - 1.0) * 1000.0
    );
    // 同号负值（亏欠）给出正比值。
    assert_eq!(
        mass_to_delta(-1.0, -2.0, 0.5),
        ((-1.0 / -2.0) / 0.5 - 1.0) * 1000.0
    );
}
