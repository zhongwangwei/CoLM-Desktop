//! `lulcc_host` 的测试：上游 `mt_lulcc_host_sync` 的逐仓室规则。

use super::*;
use colm_core::tracer::{
    soisno_slot, PatchTracerState, ReactionMode, StateOwner, TracerDescriptor, TracerFamily,
    TracerSet, MAX_SNOW_LAYERS,
};

fn solute(init_conc: f64) -> TracerDescriptor {
    TracerDescriptor {
        name: "NO3".to_owned(),
        category: "solute".to_owned(),
        family: TracerFamily::Solute,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: "material_per_water".to_owned(),
        mol_weight: 62.0,
        ref_ratio: 1.0,
        init_delta: 0.0,
        init_conc,
        precip_default_conc: init_conc,
        vapor_default_conc: 0.0,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        n2o_ef5: 0.0,
        charge: -1,
    }
}

fn set(tracers: Vec<TracerDescriptor>) -> TracerSet {
    TracerSet { tracers }
}

fn state(set: &TracerSet) -> PatchTracerState {
    PatchTracerState::allocated(set)
}

fn options() -> HostFollowOptions {
    HostFollowOptions {
        vegetation_snow: false,
        cold_start: TracerColdStart {
            variably_saturated_flow: false,
            aquifer_mixing_water_mm: 0.0,
        },
    }
}

fn host(patch_type: i64) -> HostWater {
    let mut water = HostWater::zeroed();
    water.patch_type = patch_type;
    water
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-14, "{a} != {b}");
}

#[test]
fn patch_without_a_source_cold_starts_from_the_new_host_water() {
    let set = set(vec![solute(0.5)]);
    let mut states = vec![state(&set)];
    let mut new = host(0);
    new.ldew_rain = 2.0;
    new.wliq_soisno[soisno_slot(1)] = 3.0;
    let outcome = follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        &[host(0)],
        &[false],
        &[],
        options(),
    )
    .unwrap();
    assert_eq!(outcome.cold_started, 1);
    assert_eq!(outcome.followed, 0);
    close(states[0].pools[0].ldew_rain, 2.0 * 0.5);
    close(states[0].pools[0].wliq_soisno[soisno_slot(1)], 3.0 * 0.5);
}

#[test]
fn canopy_follows_the_total_ldew_when_vegetation_snow_is_off() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].ldew_rain = 1.0;
    states[0].pools[0].ldew_snow = 1.0;
    let mut src = host(0);
    src.ldew = 10.0;
    src.ldew_rain = 7.0;
    src.ldew_snow = 3.0;
    let mut new = host(0);
    new.ldew = 20.0;
    new.ldew_rain = 14.0;
    new.ldew_snow = 6.0;
    let outcome = follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    assert_eq!(outcome.followed, 1);
    close(states[0].pools[0].ldew_rain, 2.0);
    close(states[0].pools[0].ldew_snow, 2.0);
}

#[test]
fn canopy_follows_rain_and_snow_separately_with_vegetation_snow() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].ldew_rain = 1.0;
    states[0].pools[0].ldew_snow = 1.0;
    let mut src = host(0);
    src.ldew_rain = 4.0;
    src.ldew_snow = 2.0;
    let mut new = host(0);
    new.ldew_rain = 8.0;
    new.ldew_snow = 1.0;
    let outcome = follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        HostFollowOptions {
            vegetation_snow: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(outcome.followed, 1);
    close(states[0].pools[0].ldew_rain, 2.0);
    close(states[0].pools[0].ldew_snow, 0.5);
}

#[test]
fn snow_column_rebins_and_moves_solid_to_the_top_soil_when_none_is_left() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].wliq_soisno[0] = 2.5;
    states[0].pools[0].wice_soisno[0] = 0.5;
    states[0].pools[0].solid_soisno[0] = 0.4;
    states[0].pools[0].solid_soisno[soisno_slot(1)] = 0.6;
    let mut src = host(0);
    src.wliq_soisno[0] = 5.0;
    src.wice_soisno[0] = 1.0;
    let mut new = host(0);
    // 主机把雪并进前置雪：层 0 没有水，只剩 scv。
    new = new.with_thin_snow(3.0);
    assert_eq!(new.thin_snow, 3.0);
    let outcome = follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    assert_eq!(outcome.followed, 1);
    close(states[0].pools[0].scv, 1.5);
    for layer in 0..MAX_SNOW_LAYERS {
        close(states[0].pools[0].wliq_soisno[layer], 0.0);
        close(states[0].pools[0].wice_soisno[layer], 0.0);
    }
    close(states[0].pools[0].solid_soisno[0], 0.0);
    close(states[0].pools[0].solid_soisno[soisno_slot(1)], 1.0);
}

#[test]
fn soil_layers_follow_liquid_and_ice_separately() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    let slot = soisno_slot(2);
    states[0].pools[0].wliq_soisno[slot] = 2.0;
    states[0].pools[0].wice_soisno[slot] = 0.5;
    let mut src = host(0);
    src.wliq_soisno[slot] = 4.0;
    src.wice_soisno[slot] = 1.0;
    let mut new = host(0);
    new.wliq_soisno[slot] = 2.0;
    new.wice_soisno[slot] = 2.0;
    follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    close(states[0].pools[0].wliq_soisno[slot], 1.0);
    close(states[0].pools[0].wice_soisno[slot], 1.0);
}

#[test]
fn surface_water_follows_the_host() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].wdsrf = 3.0;
    let mut src = host(0);
    src.wdsrf = 6.0;
    let mut new = host(0);
    new.wdsrf = 2.0;
    follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    close(states[0].pools[0].wdsrf, 1.0);
}

#[test]
fn wetland_water_restarts_at_the_cold_ratio_even_with_a_source_patch() {
    let set = set(vec![solute(0.2)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].wetwat = 99.0;
    let mut src = host(2);
    src.wetwat = 10.0;
    let mut new = host(2);
    new.wetwat = 10.0;
    follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    close(states[0].pools[0].wetwat, 2.0);
}

#[test]
fn dissolved_limit_rebalances_canopy_and_the_shared_surface_store() {
    let mut tracer = solute(1.0);
    tracer.max_dissolved_conc = 0.5;
    let set = set(vec![tracer]);
    let mut states = vec![state(&set)];
    // 冠层：跟随之后材料比水量多，超出的进冠层固相。
    states[0].pools[0].ldew_rain = 2.0;
    states[0].pools[0].wdsrf = 1.0;
    states[0].pools[0].wetwat = 3.0;
    let mut src = host(2);
    src.ldew = 1.0;
    src.ldew_rain = 1.0;
    src.wdsrf = 1.0;
    src.wetwat = 3.0;
    let mut new = host(2);
    new.ldew = 1.0;
    new.ldew_rain = 1.0;
    new.wdsrf = 1.0;
    new.wetwat = 3.0;
    follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    // 冠层：cap = 0.5 * 1，材料 2 → 固相 1.5、液相 0.5。
    close(states[0].pools[0].ldew_rain, 0.5);
    close(states[0].pools[0].canopy_solid, 1.5);
    // 地表：湿地水 3 + ponded 1 = 4 的水共用一份容量 0.5*4 = 2；
    // 总材料 3（湿地）+ 1（ponded）= 4 → 固相 2，按水量比拆回 0.5 / 1.5。
    close(states[0].pools[0].surface_solid, 2.0);
    close(states[0].pools[0].wdsrf, 0.5);
    close(states[0].pools[0].wetwat, 1.5);
}

#[test]
fn change_is_the_patch_mass_difference_times_area() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].ldew_rain = 1.0;
    let mut src = host(0);
    src.ldew = 1.0;
    let mut new = host(0);
    new.ldew = 3.0;
    let outcome = follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[2.0],
        options(),
    )
    .unwrap();
    close(outcome.change[0], (3.0 - 1.0) * 2.0);
}

#[test]
fn a_difference_below_the_ratio_threshold_is_left_alone() {
    let set = set(vec![solute(0.1)]);
    let mut states = vec![state(&set)];
    states[0].pools[0].ldew_rain = 1.0;
    let mut src = host(0);
    src.ldew = 1.0;
    let mut new = host(0);
    new.ldew = 1.0 + 5.0e-13;
    let outcome = follow_host_water(
        &set,
        &mut states,
        std::slice::from_ref(&new),
        std::slice::from_ref(&src),
        &[true],
        &[],
        options(),
    )
    .unwrap();
    assert_eq!(outcome.followed, 0);
    close(states[0].pools[0].ldew_rain, 1.0);
}
