//! THERMAL 后蒸散示踪物：守恒、均匀比值不变、非挥发溶质留下、分馏拒绝运行、融雪携带。

use super::*;
use crate::tracer::{TracerNamelist, TracerParameterOverrides, SOISNO_LAYERS};

const R: f64 = 2.0052e-3;

fn set_of(types: &str, names: &str, num: i64) -> TracerSet {
    TracerSet::build(
        &TracerNamelist {
            num,
            names: names.to_owned(),
            types: types.to_owned(),
            ..TracerNamelist::default()
        },
        |_| Ok(None::<TracerParameterOverrides>),
    )
    .unwrap()
}

/// 一个 O18 同位素（init_delta=0 → 比值 = ref_ratio）。
fn isotope_set() -> TracerSet {
    let mut set = set_of("isotope", "O18", 1);
    set.tracers[0].ref_ratio = R;
    set
}

/// 水量场景：冠层蒸发+融化+升华，第 1 层蒸发+霜，第 2 层残差损失，雪层 -1 融化，
/// 第 3 层冻结，第 4 层残差增益。
struct Scene {
    snl: i32,
    wliq_bef: [f64; SOISNO_LAYERS],
    wice_bef: [f64; SOISNO_LAYERS],
    wliq: [f64; SOISNO_LAYERS],
    wice: [f64; SOISNO_LAYERS],
    thaw: [f64; SOISNO_LAYERS],
    frzc: [f64; SOISNO_LAYERS],
    t: [f64; SOISNO_LAYERS],
}

fn scene() -> Scene {
    let mut s = Scene {
        snl: -2,
        wliq_bef: [0.0; SOISNO_LAYERS],
        wice_bef: [0.0; SOISNO_LAYERS],
        wliq: [0.0; SOISNO_LAYERS],
        wice: [0.0; SOISNO_LAYERS],
        thaw: [0.0; SOISNO_LAYERS],
        frzc: [0.0; SOISNO_LAYERS],
        t: [272.0; SOISNO_LAYERS],
    };
    for j in -1..=10 {
        let k = soisno_slot(j);
        s.wliq_bef[k] = 10.0 + f64::from(j);
        s.wice_bef[k] = 3.0;
    }
    s.wliq = s.wliq_bef;
    s.wice = s.wice_bef;
    // 雪层 -1：融化 0.4。
    let k = soisno_slot(-1);
    s.thaw[k] = 0.4;
    s.wliq[k] += 0.4;
    s.wice[k] -= 0.4;
    // 第 1 层：蒸发 0.3，霜 0.05。
    let k = soisno_slot(1);
    s.wliq[k] -= 0.3;
    s.wice[k] += 0.05;
    // 第 2 层：液态残差损失 0.2、冰残差损失 0.1。
    let k = soisno_slot(2);
    s.wliq[k] -= 0.2;
    s.wice[k] -= 0.1;
    // 第 3 层：冻结 0.5。
    let k = soisno_slot(3);
    s.frzc[k] = 0.5;
    s.wliq[k] -= 0.5;
    s.wice[k] += 0.5;
    // 第 4 层：液态残差增益 0.25。
    let k = soisno_slot(4);
    s.wliq[k] += 0.25;
    s
}

fn input<'a>(s: &'a Scene) -> EvapoInput<'a> {
    EvapoInput {
        deltim: 1800.0,
        snl: s.snl,
        // 冠层：雨 1.0 → 0.8（蒸发 0.3、融化 0.1），雪 0.5 → 0.35（融化 0.1、升华 0.05）。
        ldew_rain: 0.8,
        ldew_snow: 0.35,
        ldew_rain_bef: 1.0,
        ldew_snow_bef: 0.5,
        wliq_soisno: &s.wliq,
        wice_soisno: &s.wice,
        wliq_soisno_bef: &s.wliq_bef,
        wice_soisno_bef: &s.wice_bef,
        canopy_smelt_mass: Some(0.1),
        canopy_frzc_mass: Some(0.0),
        soil_thaw_mass: Some(&s.thaw),
        soil_frzc_mass: Some(&s.frzc),
        tleaf: Some(271.0),
        t_soisno: Some(&s.t),
        forc_q: Some(0.004),
        forc_psrf: Some(1.0e5),
        subl_skin_mm: 1.0,
        canopy_equilibration: 0.0,
    }
}

/// 示踪物池按水量 × 比值填满。
fn fill(state: &mut PatchTracerState, itrc: usize, s: &Scene, ratio: f64) {
    let p = &mut state.pools[itrc];
    p.ldew_rain = 1.0 * ratio;
    p.ldew_snow = 0.5 * ratio;
    for k in 0..SOISNO_LAYERS {
        p.wliq_soisno[k] = s.wliq_bef[k] * ratio;
        p.wice_soisno[k] = s.wice_bef[k] * ratio;
    }
}

fn total(state: &PatchTracerState, itrc: usize) -> f64 {
    let p = &state.pools[itrc];
    let mut sum = p.ldew_rain + p.ldew_snow + p.canopy_solid;
    for k in 0..SOISNO_LAYERS {
        sum += p.wliq_soisno[k] + p.wice_soisno[k] + p.solid_soisno[k];
    }
    sum
}

#[test]
fn tracer_change_equals_deposition_minus_booked_losses() {
    for (set, ratio) in [(isotope_set(), R), (set_of("solute", "cl", 1), 0.7)] {
        let s = scene();
        let mut state = PatchTracerState::allocated(&set);
        fill(&mut state, 0, &s, ratio);
        let before = total(&state, 0);
        tracer_evapo(
            &set,
            &mut state,
            TracerPhysics::default(),
            &[1.3 * ratio],
            &input(&s),
        )
        .unwrap();
        let after = total(&state, 0);
        let acc = &state.acc[0];
        let budget = acc.precip - acc.evap + state.step[0].numerical_residual_step;
        assert!(
            ((after - before) - budget).abs() <= 1.0e-13 * before,
            "{}: change {} vs budget {}",
            set.tracers[0].name,
            after - before,
            budget
        );
        // 分类累加器加起来就是总蒸发。
        assert_eq!(acc.evap, acc.canopyevap + acc.subl + acc.soilevap);
    }
}

#[test]
fn uniform_ratio_is_preserved_without_fractionation() {
    let set = isotope_set();
    let s = scene();
    let mut state = PatchTracerState::allocated(&set);
    fill(&mut state, 0, &s, R);
    let inp = input(&s);
    tracer_evapo(&set, &mut state, TracerPhysics::default(), &[R], &inp).unwrap();
    let close = |trc: f64, water: f64| (trc - water * R).abs() <= 1.0e-14 * water * R;
    let p = &state.pools[0];
    assert!(close(p.ldew_rain, inp.ldew_rain));
    assert!(close(p.ldew_snow, inp.ldew_snow));
    for j in inp.snl + 1..=10 {
        let k = soisno_slot(j);
        assert!(close(p.wliq_soisno[k], s.wliq[k]), "wliq layer {j}");
        assert!(close(p.wice_soisno[k], s.wice[k]), "wice layer {j}");
    }
    let acc = &state.acc[0];
    // 冠层蒸发 0.3、冠层升华 0.05、第 1 层蒸发 0.3；霜 0.05。
    assert!((acc.water_evap_gross - 0.65).abs() < 1.0e-12);
    assert!((acc.water_precip - 0.05).abs() < 1.0e-12);
    assert!((acc.evap - 0.65 * R).abs() < 1.0e-14);
}

#[test]
fn nonvolatile_solute_stays_behind() {
    let set = set_of("solute", "cl", 1);
    let s = scene();
    let mut state = PatchTracerState::allocated(&set);
    fill(&mut state, 0, &s, 0.7);
    let inp = input(&s);
    tracer_evapo(&set, &mut state, TracerPhysics::default(), &[5.0], &inp).unwrap();
    let p = &state.pools[0];
    let acc = &state.acc[0];
    // 蒸发不带走溶质，露/霜也不带来。
    assert_eq!(acc.evap, 0.0);
    assert_eq!(acc.precip, 0.0);
    assert!((acc.water_evap_gross - 0.65).abs() < 1.0e-12);
    // 冠层总量不变（融化只是雪 → 雨）；第 1 层液态全部留下。
    assert!((p.ldew_rain + p.ldew_snow - 1.5 * 0.7).abs() < 1.0e-14);
    let k = soisno_slot(1);
    assert_eq!(p.wliq_soisno[k], s.wliq_bef[k] * 0.7);
    assert_eq!(p.wice_soisno[k], s.wice_bef[k] * 0.7);
}

#[test]
fn legacy_heuristic_matches_explicit_phase_masses_when_unambiguous() {
    let set = isotope_set();
    let s = scene();
    let mut explicit = PatchTracerState::allocated(&set);
    fill(&mut explicit, 0, &s, R);
    let mut legacy = explicit.clone();
    // 冠层只有融化（雨 +0.1、雪 -0.1），各层只有相变。
    let mut s2 = scene();
    s2.wliq = s2.wliq_bef;
    s2.wice = s2.wice_bef;
    for k in 0..SOISNO_LAYERS {
        s2.wliq[k] += s2.thaw[k] - s2.frzc[k];
        s2.wice[k] += s2.frzc[k] - s2.thaw[k];
    }
    let mut a = input(&s2);
    a.ldew_rain = 1.1;
    a.ldew_snow = 0.4;
    tracer_evapo(&set, &mut explicit, TracerPhysics::default(), &[R], &a).unwrap();
    let mut b = a;
    b.canopy_smelt_mass = None;
    b.canopy_frzc_mass = None;
    b.soil_thaw_mass = None;
    b.soil_frzc_mass = None;
    tracer_evapo(&set, &mut legacy, TracerPhysics::default(), &[R], &b).unwrap();
    let _ = s;
    // 启发式从水量差推相变量，与显式质量只差舍入。
    let (e, l) = (&explicit.pools[0], &legacy.pools[0]);
    let near = |x: f64, y: f64| (x - y).abs() <= 1.0e-14 * x.abs().max(1.0e-3);
    assert!(near(e.ldew_rain, l.ldew_rain) && near(e.ldew_snow, l.ldew_snow));
    for k in 0..SOISNO_LAYERS {
        assert!(near(e.wliq_soisno[k], l.wliq_soisno[k]), "wliq slot {k}");
        assert!(near(e.wice_soisno[k], l.wice_soisno[k]), "wice slot {k}");
    }
    assert_eq!(explicit.acc[0].evap, 0.0);
    assert_eq!(legacy.acc[0].evap, 0.0);
}

#[test]
fn fractionating_isotope_is_refused() {
    let set = isotope_set();
    let s = scene();
    let mut state = PatchTracerState::allocated(&set);
    fill(&mut state, 0, &s, R);
    let physics = TracerPhysics {
        fractionation: true,
    };
    let err = tracer_evapo(&set, &mut state, physics, &[R], &input(&s)).unwrap_err();
    assert!(err.to_string().contains("not ported yet"), "{err}");
    // 溶质不受分馏开关影响。
    let solute = set_of("solute", "cl", 1);
    let mut state = PatchTracerState::allocated(&solute);
    fill(&mut state, 0, &s, 0.7);
    tracer_evapo(&solute, &mut state, physics, &[0.7], &input(&s)).unwrap();
}

#[test]
fn flood_loss_is_proportional_and_skips_solutes() {
    let set = set_of("isotope,solute", "O18,cl", 2);
    let flood = FloodEvapInput {
        water_credit: 50.0,
        water_evap: 2.0,
        temp_k: 290.0,
        forc_q: 0.01,
        forc_psrf: 1.0e5,
        forc_us: 2.0,
        forc_vs: 1.0,
    };
    let credit = [50.0 * R, 50.0 * 0.7];
    let loss =
        tracer_flood_evap_loss(&set, TracerPhysics::default(), &[R, 0.7], &credit, &flood).unwrap();
    assert_eq!(loss[0], (2.0 * (credit[0] / 50.0)).min(credit[0]));
    assert_eq!(loss[1], 0.0);
    let none = FloodEvapInput {
        water_evap: 0.0,
        ..flood
    };
    let physics = TracerPhysics {
        fractionation: true,
    };
    assert_eq!(
        tracer_flood_evap_loss(&set, physics, &[R, 0.7], &credit, &none).unwrap(),
        vec![0.0, 0.0]
    );
    assert!(tracer_flood_evap_loss(&set, physics, &[R, 0.7], &credit, &flood).is_err());
}

#[test]
fn snow_melt_carry_moves_the_melted_fraction() {
    let set = isotope_set();
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].scv = 4.0 * R;
    tracer_snow_melt_carry(&set, &mut state, 0, 3.0, 4.0);
    let ratio: f64 = 3.0 / 4.0;
    assert_eq!(state.step[0].sm_carry, 4.0 * R * (1.0 - ratio));
    assert_eq!(state.pools[0].scv, 4.0 * R * ratio);

    tracer_snow_melt_carry(&set, &mut state, 0, 0.0, 3.0);
    assert_eq!(state.step[0].sm_carry, 4.0 * R * ratio);
    assert_eq!(state.pools[0].scv, 0.0);

    tracer_snow_melt_carry(&set, &mut state, -1, 0.0, 3.0);
    assert_eq!(state.step[0].sm_carry, 0.0);
}
