//! 雪层示踪物：与宿主水侧例程对拍拓扑（比值取 2 的幂时示踪物 = 水 × R 逐位成立）、
//! 一般比值下的守恒，以及几个手算算例。

use super::*;
use crate::snow::{
    combine_snow_layers, divide_snow_layers, relocate_soil_frost_ice, RuntimeSnowColumn,
    SnowToSoilTransfer, SoilFrostTop,
};
use crate::tracer::{ReactionMode, StateOwner, TracerDescriptor, TracerFamily};

fn descriptor(name: &str, owner: StateOwner, max_dissolved_conc: f64) -> TracerDescriptor {
    TracerDescriptor {
        name: name.to_owned(),
        category: "solute".to_owned(),
        family: TracerFamily::Solute,
        state_owner: owner,
        reaction_mode: ReactionMode::None,
        unit_kind: "tracer_per_water".to_owned(),
        mol_weight: 18.0,
        ref_ratio: 1.0,
        init_delta: 0.0,
        init_conc: 1.0,
        precip_default_conc: 1.0,
        vapor_default_conc: 1.0,
        max_dissolved_conc,
        reactive_decay_rate: 0.0,
        charge: 0,
    }
}

fn one_tracer() -> TracerSet {
    TracerSet {
        tracers: vec![descriptor("sol", StateOwner::GenericWater, f64::MAX)],
    }
}

/// 宿主雪列：`layers` 自上而下 `(dz, wliq, wice)`。
fn column(layers: &[(f64, f64, f64)]) -> RuntimeSnowColumn {
    let mut state = RuntimeSnowColumn::empty();
    let snl = -(layers.len() as i32);
    state.layer_count = snl;
    for (k, &(dz, wliq, wice)) in layers.iter().enumerate() {
        let slot = soisno_slot(snl + 1 + k as i32);
        state.thickness_m[slot] = dz;
        state.liquid_water_kg_m2[slot] = wliq;
        state.ice_water_kg_m2[slot] = wice;
        state.temperature_k[slot] = 268.0;
    }
    state.water_equivalent_kg_m2 = layers.iter().map(|l| l.1 + l.2).sum();
    state.depth_m = layers.iter().map(|l| l.0).sum();
    state
}

fn snapshot(state: &RuntimeSnowColumn) -> ([f64; MAX_SNOW_LAYERS], [f64; MAX_SNOW_LAYERS]) {
    let mut wice = [0.0; MAX_SNOW_LAYERS];
    let mut dz = [0.0; MAX_SNOW_LAYERS];
    wice.copy_from_slice(&state.ice_water_kg_m2);
    dz.copy_from_slice(&state.thickness_m);
    (wice, dz)
}

/// 示踪物 = 水 × R（雪层来自宿主列，土壤第 1 层给定），固相 = 冰示踪物 × 1/8。
fn tracer_from_water(
    set: &TracerSet,
    state: &RuntimeSnowColumn,
    soil: (f64, f64),
    ratio: f64,
) -> PatchTracerState {
    let mut trc = PatchTracerState::allocated(set);
    for p in &mut trc.pools {
        for j in -4..=0 {
            let slot = soisno_slot(j);
            p.wliq_soisno[slot] = state.liquid_water_kg_m2[slot] * ratio;
            p.wice_soisno[slot] = state.ice_water_kg_m2[slot] * ratio;
            p.solid_soisno[slot] = p.wice_soisno[slot] * 0.125;
        }
        p.wliq_soisno[soisno_slot(1)] = soil.0 * ratio;
        p.wice_soisno[soisno_slot(1)] = soil.1 * ratio;
    }
    trc
}

fn snow_and_soil_total(p: &TracerPools) -> f64 {
    let mut total = p.scv;
    for j in -4..=1 {
        let slot = soisno_slot(j);
        total += p.wliq_soisno[slot] + p.wice_soisno[slot] + p.solid_soisno[slot];
    }
    total
}

/// 总量守恒：各层逐个相加的顺序与搬运顺序不同，允许舍入级差别。
fn assert_close(after: f64, before: f64) {
    assert!(
        (after - before).abs() <= 1e-14 * before.abs(),
        "{before} -> {after}"
    );
}

/// 宿主合并之后：雪层示踪物 = 水 × R 逐位相同，空槽为 0。
fn assert_layers_track_water(state: &RuntimeSnowColumn, p: &TracerPools, ratio: f64) {
    for j in -4..=0 {
        let slot = soisno_slot(j);
        if j > state.layer_count {
            assert_eq!(
                p.wliq_soisno[slot],
                state.liquid_water_kg_m2[slot] * ratio,
                "wliq {j}"
            );
            assert_eq!(
                p.wice_soisno[slot],
                state.ice_water_kg_m2[slot] * ratio,
                "wice {j}"
            );
            assert_eq!(
                p.solid_soisno[slot],
                p.wice_soisno[slot] * 0.125,
                "solid {j}"
            );
        } else {
            assert_eq!(p.wliq_soisno[slot], 0.0, "empty wliq {j}");
            assert_eq!(p.wice_soisno[slot], 0.0, "empty wice {j}");
            assert_eq!(p.solid_soisno[slot], 0.0, "empty solid {j}");
        }
    }
}

fn run_combine(
    layers: &[(f64, f64, f64)],
    soil: (f64, f64),
    ratio: f64,
) -> (
    RuntimeSnowColumn,
    SnowToSoilTransfer,
    PatchTracerState,
    SnowCombineOutcome,
    f64,
) {
    let set = one_tracer();
    let mut water = column(layers);
    let mut trc = tracer_from_water(&set, &water, soil, ratio);
    let before = snow_and_soil_total(&trc.pools[0]);
    let (wice, dz) = snapshot(&water);
    let input = SnowCombineInput {
        snl: water.layer_count,
        wice,
        dz,
    };
    let mut soil_surface = SnowToSoilTransfer {
        liquid_water_kg_m2: soil.0,
        ice_water_kg_m2: soil.1,
    };
    combine_snow_layers(&mut water, &mut soil_surface).unwrap();
    let outcome = tracer_snow_layers_combine(&set, &mut trc, &input);
    (water, soil_surface, trc, outcome, before)
}

#[test]
fn combine_thin_ice_layers_follow_the_host_topology() {
    // 第 2、4 层冰 <= 0.1 并入下层；第 5 层（j=0）厚度足够。
    let layers = [
        (0.03, 0.5, 6.0),
        (0.02, 0.25, 0.05),
        (0.04, 1.0, 8.0),
        (0.01, 0.0, 0.1),
        (0.05, 0.75, 9.0),
    ];
    let (water, soil, trc, outcome, before) = run_combine(&layers, (3.0, 1.0), 0.5);
    assert_eq!(outcome.snl, water.layer_count);
    assert!(!outcome.all_snow_gone);
    let p = &trc.pools[0];
    assert_layers_track_water(&water, p, 0.5);
    assert_eq!(p.wliq_soisno[soisno_slot(1)], soil.liquid_water_kg_m2 * 0.5);
    assert_close(snow_and_soil_total(p), before);
}

#[test]
fn combine_thin_layers_with_neighbours_follow_the_host_topology() {
    // 冰量都够，靠 `dz < dzmin` 合并：顶层 0.005 < 0.010 并入下层，再有一层 0.02 < 0.025。
    let layers = [
        (0.005, 0.1, 1.0),
        (0.10, 0.5, 20.0),
        (0.02, 0.3, 4.0),
        (0.20, 2.0, 40.0),
        (0.30, 1.0, 60.0),
    ];
    let (water, _soil, trc, outcome, before) = run_combine(&layers, (3.0, 1.0), 0.5);
    assert!(outcome.snl > -5);
    assert_eq!(outcome.snl, water.layer_count);
    let p = &trc.pools[0];
    assert_layers_track_water(&water, p, 0.5);
    assert_close(snow_and_soil_total(p), before);
}

#[test]
fn combine_shallow_pack_collapses_into_scv() {
    let layers = [(0.004, 0.2, 0.5), (0.004, 0.1, 0.7)];
    let (water, soil, trc, outcome, before) = run_combine(&layers, (3.0, 1.0), 0.5);
    assert_eq!(water.layer_count, 0);
    assert_eq!(
        outcome,
        SnowCombineOutcome {
            snl: 0,
            all_snow_gone: true
        }
    );
    let p = &trc.pools[0];
    // trc_scv = 冰示踪物之和；液相加到土壤第 1 层；固相同样下到土壤。
    assert_eq!(p.scv, water.water_equivalent_kg_m2 * 0.5);
    assert_eq!(p.scv, (0.5 + 0.7) * 0.5);
    assert_eq!(p.wliq_soisno[soisno_slot(1)], soil.liquid_water_kg_m2 * 0.5);
    assert_eq!(p.solid_soisno[soisno_slot(1)], (0.25 + 0.35) * 0.125);
    assert_layers_track_water(&water, p, 0.5);
    assert_close(snow_and_soil_total(p), before);
}

#[test]
fn combine_all_thin_ice_moves_everything_to_soil() {
    // 冰 0.03 并入下层后下层 0.08 仍 <= 0.1，再并入土壤（若下层累加后超过 0.1 则保留一层）。
    let layers = [(0.03, 0.2, 0.03), (0.03, 0.1, 0.05)];
    let (water, soil, trc, outcome, before) = run_combine(&layers, (3.0, 1.0), 0.5);
    assert_eq!(outcome.snl, 0);
    assert!(!outcome.all_snow_gone);
    assert_eq!(water.layer_count, 0);
    let p = &trc.pools[0];
    assert_eq!(p.wliq_soisno[soisno_slot(1)], soil.liquid_water_kg_m2 * 0.5);
    assert_eq!(p.wice_soisno[soisno_slot(1)], soil.ice_water_kg_m2 * 0.5);
    assert_eq!(p.scv, 0.0);
    assert_close(snow_and_soil_total(p), before);
}

#[test]
fn combine_conserves_tracer_with_a_general_ratio() {
    let layers = [
        (0.005, 0.1, 1.3),
        (0.10, 0.5, 0.07),
        (0.02, 0.3, 4.1),
        (0.20, 2.0, 40.0),
        (0.012, 1.0, 6.0),
    ];
    let ratio = 0.3141592653589793;
    let (water, soil, trc, outcome, before) = run_combine(&layers, (3.0, 1.0), ratio);
    assert_eq!(outcome.snl, water.layer_count);
    let p = &trc.pools[0];
    let after = snow_and_soil_total(p);
    assert!((after - before).abs() <= 1e-14 * before, "{before} {after}");
    for j in water.layer_count + 1..=0 {
        let slot = soisno_slot(j);
        let r_ice = p.wice_soisno[slot] / water.ice_water_kg_m2[slot];
        assert!((r_ice - ratio).abs() <= 1e-15, "layer {j}: {r_ice}");
    }
    let r_soil = p.wliq_soisno[soisno_slot(1)] / soil.liquid_water_kg_m2;
    assert!((r_soil - ratio).abs() <= 1e-15);
}

#[test]
fn combine_moves_every_tracer_including_non_transport_ones() {
    let set = TracerSet {
        tracers: vec![
            descriptor("sol", StateOwner::GenericWater, f64::MAX),
            descriptor("prov", StateOwner::Provider, f64::MAX),
        ],
    };
    let water = column(&[(0.03, 0.5, 6.0), (0.02, 0.25, 0.05)]);
    let mut trc = tracer_from_water(&set, &water, (0.0, 0.0), 0.5);
    let (wice, dz) = snapshot(&water);
    let outcome = tracer_snow_layers_combine(
        &set,
        &mut trc,
        &SnowCombineInput {
            snl: water.layer_count,
            wice,
            dz,
        },
    );
    assert_eq!(outcome.snl, -1);
    for p in &trc.pools {
        // 第 -1 层（冰 6.0）下移到第 0 层；第 0 层（冰 0.05）并入土壤。
        assert_eq!(p.wice_soisno[soisno_slot(0)], 3.0);
        assert_eq!(p.wice_soisno[soisno_slot(-1)], 0.0);
        assert_eq!(p.wice_soisno[soisno_slot(1)], 0.025);
        assert_eq!(p.wliq_soisno[soisno_slot(1)], 0.125);
    }
}

fn run_divide(
    layers: &[(f64, f64, f64)],
    ratio: f64,
) -> (RuntimeSnowColumn, PatchTracerState, i32, f64) {
    let set = one_tracer();
    let mut water = column(layers);
    let mut trc = tracer_from_water(&set, &water, (0.0, 0.0), ratio);
    let before = snow_and_soil_total(&trc.pools[0]);
    let (_, dz) = snapshot(&water);
    let input = SnowDivideInput {
        snl: water.layer_count,
        dz,
    };
    divide_snow_layers(&mut water).unwrap();
    let snl = tracer_snow_layers_divide(&set, &mut trc, &input);
    (water, trc, snl, before)
}

#[test]
fn divide_single_thick_layer_cascades_like_the_host() {
    // 一层 1.2 m：对半后逐级拆出，最后五层。
    let (water, trc, snl, before) = run_divide(&[(1.2, 3.0, 300.0)], 0.5);
    assert_eq!(snl, water.layer_count);
    assert_eq!(snl, -5);
    let p = &trc.pools[0];
    for j in -4..=0 {
        let slot = soisno_slot(j);
        assert_eq!(
            p.wliq_soisno[slot],
            water.liquid_water_kg_m2[slot] * 0.5,
            "wliq {j}"
        );
        assert_eq!(
            p.wice_soisno[slot],
            water.ice_water_kg_m2[slot] * 0.5,
            "wice {j}"
        );
        assert_eq!(
            p.solid_soisno[slot],
            p.wice_soisno[slot] * 0.125,
            "solid {j}"
        );
    }
    let after = snow_and_soil_total(p);
    assert!((after - before).abs() <= 1e-14 * before);
}

#[test]
fn divide_two_layers_matches_the_host_and_conserves() {
    for layers in [
        vec![(0.05, 0.4, 8.0), (0.12, 1.0, 20.0)],
        vec![(0.015, 0.4, 3.0), (0.03, 1.0, 6.0)],
        vec![(0.025, 0.1, 5.0), (0.3, 1.0, 60.0), (0.5, 0.0, 100.0)],
    ] {
        let (water, trc, snl, before) = run_divide(&layers, 0.5);
        assert_eq!(snl, water.layer_count);
        let p = &trc.pools[0];
        for j in snl + 1..=0 {
            let slot = soisno_slot(j);
            assert_eq!(p.wliq_soisno[slot], water.liquid_water_kg_m2[slot] * 0.5);
            assert_eq!(p.wice_soisno[slot], water.ice_water_kg_m2[slot] * 0.5);
        }
        let after = snow_and_soil_total(p);
        assert!((after - before).abs() <= 1e-14 * before);
    }
}

#[test]
fn divide_hand_checked_split() {
    // 两层：顶层 0.05 > 0.02，拆出 drr = 0.03（propor = 0.03/0.05）并入第二层。
    let set = one_tracer();
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].wice_soisno[soisno_slot(-1)] = 10.0;
    trc.pools[0].wice_soisno[soisno_slot(0)] = 4.0;
    let mut dz = [0.0; MAX_SNOW_LAYERS];
    dz[soisno_slot(-1)] = 0.05;
    dz[soisno_slot(0)] = 0.04;
    let snl = tracer_snow_layers_divide(&set, &mut trc, &SnowDivideInput { snl: -2, dz });
    assert_eq!(snl, -2);
    let p = &trc.pools[0];
    let drr = 0.05 - 0.02;
    assert_eq!(p.wice_soisno[soisno_slot(-1)], (0.02 / 0.05) * 10.0);
    assert_eq!(p.wice_soisno[soisno_slot(0)], 4.0 + (drr / 0.05) * 10.0);
}

#[test]
fn newsnow_first_layer_partitions_accumulated_scv() {
    let set = one_tracer();
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].scv = 6.0;
    trc.step[0].pg_snow_ground = 2.0;
    let warnings = tracer_newsnow(
        &set,
        &mut trc,
        &NewSnowInput {
            patchtype: 0,
            snl: -1,
            snl_old: 0,
            pg_snow: 0.001,
            deltim: 1800.0,
            scv: 16.0,
            top: Some(NewSnowTop {
                wliq: 4.0,
                wice: 12.0,
                wice_before: 0.0,
            }),
            debug: true,
        },
    );
    assert!(warnings.is_empty());
    let p = &trc.pools[0];
    // R_layer = 8/16：冰 6、液 2，trc_scv 清零。
    assert_eq!(p.wice_soisno[soisno_slot(0)], 6.0);
    assert_eq!(p.wliq_soisno[soisno_slot(0)], 2.0);
    assert_eq!(p.scv, 0.0);
}

#[test]
fn newsnow_existing_layer_adds_ground_snow_and_residual() {
    let set = one_tracer();
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].scv = 0.25;
    trc.pools[0].wice_soisno[soisno_slot(-1)] = 10.0;
    trc.step[0].pg_snow_ground = 1.5;
    let warnings = tracer_newsnow(
        &set,
        &mut trc,
        &NewSnowInput {
            patchtype: 0,
            snl: -2,
            snl_old: -2,
            pg_snow: 0.001,
            deltim: 1800.0,
            scv: 30.0,
            top: Some(NewSnowTop {
                wliq: 0.0,
                wice: 21.8,
                wice_before: 20.0,
            }),
            debug: true,
        },
    );
    // trc_scv 残留触发诊断；d_wice = 1.8 = pg_snow*dt，不报 Case C。
    assert_eq!(
        warnings,
        vec![NewSnowWarning::LayeredScvResidual {
            itrc: 0,
            trc_scv: 0.25
        }]
    );
    let p = &trc.pools[0];
    assert_eq!(p.wice_soisno[soisno_slot(-1)], (10.0 + 1.5) + 0.25);
    assert_eq!(p.scv, 0.0);
}

#[test]
fn newsnow_new_layer_on_existing_pack_uses_step_ratio() {
    let set = one_tracer();
    let mut trc = PatchTracerState::allocated(&set);
    trc.step[0].pg_snow_ground = 0.9;
    tracer_newsnow(
        &set,
        &mut trc,
        &NewSnowInput {
            patchtype: 0,
            snl: -2,
            snl_old: -1,
            pg_snow: 0.001,
            deltim: 1800.0,
            scv: 30.0,
            top: Some(NewSnowTop {
                wliq: 0.5,
                wice: 1.8,
                wice_before: 0.0,
            }),
            debug: false,
        },
    );
    let p = &trc.pools[0];
    let r = 0.9 / (0.001 * 1800.0);
    assert_eq!(p.wice_soisno[soisno_slot(-1)], 1.8 * r);
    assert_eq!(p.wliq_soisno[soisno_slot(-1)], 0.5 * r);
}

#[test]
fn newsnow_without_layers_accumulates_and_warm_wetland_drains() {
    let set = TracerSet {
        tracers: vec![
            descriptor("sol", StateOwner::GenericWater, f64::MAX),
            descriptor("prov", StateOwner::Provider, f64::MAX),
        ],
    };
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].scv = 1.0;
    trc.step[0].pg_snow_ground = 0.5;
    trc.step[1].pg_snow_ground = 0.5;
    let mut input = NewSnowInput {
        patchtype: 0,
        snl: 0,
        snl_old: 0,
        pg_snow: 0.001,
        deltim: 1800.0,
        scv: 3.0,
        top: None,
        debug: false,
    };
    tracer_newsnow(&set, &mut trc, &input);
    assert_eq!(trc.pools[0].scv, 1.5);
    // 不走通用输运的示踪物不动。
    assert_eq!(trc.pools[1].scv, 0.0);

    input.patchtype = 2;
    input.scv = 0.0;
    trc.pools[0].wetwat = 2.0;
    tracer_newsnow(&set, &mut trc, &input);
    assert_eq!(trc.pools[0].scv, 0.0);
    assert_eq!(trc.pools[0].wetwat, 2.0 + (1.5 + 0.5));
}

#[test]
fn newsnow_equilibrates_the_top_liquid() {
    let set = TracerSet {
        tracers: vec![descriptor("sol", StateOwner::GenericWater, 0.1)],
    };
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].scv = 4.0;
    tracer_newsnow(
        &set,
        &mut trc,
        &NewSnowInput {
            patchtype: 0,
            snl: -1,
            snl_old: 0,
            pg_snow: 0.0,
            deltim: 1800.0,
            scv: 16.0,
            top: Some(NewSnowTop {
                wliq: 4.0,
                wice: 12.0,
                wice_before: 0.0,
            }),
            debug: false,
        },
    );
    let p = &trc.pools[0];
    // 液相分得 1.0，溶解上限 0.1*4 = 0.4，其余进固相。
    assert_eq!(p.wice_soisno[soisno_slot(0)], 3.0);
    assert_eq!(p.wliq_soisno[soisno_slot(0)], 0.4);
    assert_eq!(p.solid_soisno[soisno_slot(0)], 1.0 - 0.4);
}

fn frost_soil() -> SoilFrostTop {
    SoilFrostTop {
        porosity: 0.4,
        thickness_m: 0.02,
        temperature_k: 265.0,
        ice_water_kg_m2: 9.0,
    }
}

#[test]
fn relocate_frost_into_scv_or_a_new_layer_matches_the_host() {
    let set = one_tracer();
    for snowdp in [0.0, 0.009] {
        let mut water = RuntimeSnowColumn::empty();
        water.depth_m = snowdp;
        water.water_equivalent_kg_m2 = snowdp * 100.0;
        let mut soil = frost_soil();
        let input = FrostRelocationInput {
            snl: 0,
            porsl1: soil.porosity,
            dz1: soil.thickness_m,
            wice1: soil.ice_water_kg_m2,
            snowdp,
        };
        let mut trc = PatchTracerState::allocated(&set);
        trc.pools[0].scv = water.water_equivalent_kg_m2 * 0.5;
        trc.pools[0].wice_soisno[soisno_slot(1)] = soil.ice_water_kg_m2 * 0.5;
        let before = snow_and_soil_total(&trc.pools[0]);
        relocate_soil_frost_ice(&mut water, &mut soil, None);
        let moved = tracer_relocate_soil_frost_ice(&set, &mut trc, &input).unwrap();
        let p = &trc.pools[0];
        // excess = 9 - 917*0.4*0.02 = 1.664。
        assert_eq!(moved.excess, (-(917.0 * 0.4_f64)).mul_add(0.02, 9.0));
        assert_eq!(moved.created_layer, water.layer_count == -1);
        if moved.created_layer {
            assert_eq!(p.scv, 0.0);
            let r = p.wice_soisno[soisno_slot(0)] / water.ice_water_kg_m2[soisno_slot(0)];
            assert!((r - 0.5).abs() <= 1e-15);
        } else {
            assert_eq!(p.wice_soisno[soisno_slot(0)], 0.0);
            assert!((p.scv / water.water_equivalent_kg_m2 - 0.5).abs() <= 1e-15);
        }
        let r_soil = p.wice_soisno[soisno_slot(1)] / soil.ice_water_kg_m2;
        assert!((r_soil - 0.5).abs() <= 1e-15);
        let after = snow_and_soil_total(p);
        assert!((after - before).abs() <= 1e-15 * before);
    }
}

#[test]
fn relocate_frost_into_the_top_layer() {
    let set = one_tracer();
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].wice_soisno[soisno_slot(-1)] = 7.0;
    trc.pools[0].wice_soisno[soisno_slot(1)] = 18.0;
    let input = FrostRelocationInput {
        snl: -2,
        porsl1: 0.4,
        dz1: 0.02,
        wice1: 9.0,
        snowdp: 0.3,
    };
    let moved = tracer_relocate_soil_frost_ice(&set, &mut trc, &input).unwrap();
    assert!(!moved.created_layer);
    let p = &trc.pools[0];
    assert_eq!(
        p.wice_soisno[soisno_slot(-1)],
        18.0_f64.mul_add(moved.fraction, 7.0)
    );
    assert_eq!(p.wice_soisno[soisno_slot(1)], (1.0 - moved.fraction) * 18.0);
    assert_eq!(p.wice_soisno[soisno_slot(0)], 0.0);
}

#[test]
fn relocate_without_excess_is_a_no_op() {
    let set = one_tracer();
    let mut trc = PatchTracerState::allocated(&set);
    trc.pools[0].wice_soisno[soisno_slot(1)] = 1.0;
    let before = trc.clone();
    let input = FrostRelocationInput {
        snl: 0,
        porsl1: 0.4,
        dz1: 0.02,
        wice1: 2.0,
        snowdp: 0.0,
    };
    assert!(tracer_relocate_soil_frost_ice(&set, &mut trc, &input).is_none());
    assert_eq!(trc, before);
}
