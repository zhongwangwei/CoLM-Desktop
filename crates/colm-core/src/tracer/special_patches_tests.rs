//! `MOD_Tracer_SpecialPatches` 与 `sync_tracer_patch_ratio` 的测试。
//!
//! `BOX_CASES`/`SYNC_CASES` 来自 `harness.F90`：`tracer_waterbody_patch` 的混合箱算式
//! （调用真实的 `MOD_Tracer_EvapLimit`）与 `sync_tracer_patch_ratio` 的同位素含水层式、
//! `tracer_hist_out` 的叶水伪质量，原样抄写后用内核编译选项编译。其 GIMPLE 的
//! `trc_input` 为 `FMA(R_pool, deficit, FMA(R_frost, dep_ice, FMA(atm, R_precip, R_dew*dep_liq)))`，
//! 含水层为 `FMS(wa + ref_water, R, ref_mass)`，与内核 dump 相同。

use super::*;
use crate::tracer::hist::{history_variable, patch_term, PatchTerm, TracerHistId};
use crate::tracer::{
    PatchTracerState, ReactionMode, StateOwner, TracerDescriptor, TracerFamily, TracerSet,
};

fn descriptor(family: TracerFamily, ref_ratio: f64, init_conc: f64) -> TracerDescriptor {
    let isotope = family == TracerFamily::Isotope;
    TracerDescriptor {
        name: if isotope { "HDO" } else { "Cl" }.to_owned(),
        category: if isotope { "isotope" } else { "solute" }.to_owned(),
        family,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: if isotope { "ratio" } else { "tracer_per_water" }.to_owned(),
        mol_weight: 18.0,
        ref_ratio,
        init_delta: 0.0,
        init_conc,
        precip_default_conc: init_conc,
        vapor_default_conc: init_conc,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        charge: 0,
    }
}

fn bits(values: &[u64]) -> Vec<f64> {
    values.iter().map(|b| f64::from_bits(*b)).collect()
}
#[rustfmt::skip]
const BOX_CASES: [([u64; 16], [u64; 6]); 6] = [
    (
        [0x40140D6C50681AD8, 0x3F30F3E08646C4F1, 0x3F5A09D11AB30D7D, 0x3F5F40A8E4DA2735, 0x3F5F40A8E4DA2735, 0x3FF17D880402FB10, 0x3FED9B238B3B3647, 0x3FB64E0A80C635AF, 0x3FB21B3C4AF10346, 0x4097BFCC8FDAFF9A, 0x4012FD8E7CCDFB1D, 0x409364A39D628947, 0x3FF5962B326B2C56, 0x40069B27DE6D3650, 0x40B5E8A932BF3153, 0x3FE2265289A44CA5],
        [0x3F74D8F12F2D316A, 0x401056D284FF26D1, 0x3F722FEC86656AB5, 0x3F830BCFBBD1DCAB, 0x3F257C7BC314E82F, 0x4014125EBD31CD09],
    ),
    (
        [0x403D2EBF55165D7E, 0x3F49CD554864C195, 0x3F5717781FAB1C81, 0x3F5609A0FB4EE427, 0x3F5609A0FB4EE427, 0x4010A3D9CF6947B4, 0x3FF25E604724BCC1, 0x3FABE00AC037C015, 0x3FAA0B2A50341655, 0x40BCC55E401BAABC, 0x3FFD7E6AC23AFCD6, 0x4082C501D07B8A03, 0x3FE1535DA622A6BB, 0x40032BA89FE65751, 0x40997E75CEAE7CEC, 0x3FEF01F6B33E03ED],
        [0x3F859CFFFCC5D058, 0x40030A2A539D21AF, 0x3F6193342EA927D7, 0x3F83724B5DD7C219, 0x3F90D1F4704056DD, 0x403D313F5A6B656E],
    ),
    (
        [0x4000385BA54070B7, 0x3F369AFA931EDFF1, 0x3F52EE34F00517CE, 0x3F597D25AF8F2342, 0x3F597D25AF8F2342, 0x4013A7DB662F4FB7, 0x3FFFD0F484BFA1E9, 0x3FB43B424E5BA9B8, 0x3F93FB212427F643, 0x40C2C589FFCB5B14, 0x4009685054F2D0A1, 0x4073FB444AB6F689, 0x40B2C589FFCB5B14, 0x4008C51766B18A2F, 0x40B788C3C6BA5187, 0x3FC22AC340245587],
        [0x3F799A038CC6713B, 0x3FB14F65730CA974, 0x3FF04313A3603159, 0x3F45755682708117, 0x3F24A42037537D4F, 0x40004473CF323AF9],
    ),
    (
        [0x4017EAB4DD4FD56A, 0x3F484883AFCE4334, 0x3F508AF308F613DA, 0x3F5D164A6E4EA777, 0x3F5D164A6E4EA777, 0x3FE80F22C6701E46, 0x3FEF327612BE64EC, 0x3FB7F57CEA965160, 0x3FB04373D42086E8, 0x409F5F818D01BF03, 0x3FF1A867F5E350D0, 0x4098447CD8F688FA, 0x3FDFED14763FDA29, 0x400C1D513C783AA2, 0x40C11C5CB95EB8B9, 0x3FEC91B25A392365],
        [0x3F70244DB998F056, 0x40127FEF46975225, 0x3F58568B0BFF8994, 0x3F856EB1E6CF0ACA, 0x3F242177D8A463E1, 0x4017EDFBACA0BD34],
    ),
    (
        [0x4035B54A7AD76A95, 0x3F25B04747D97509, 0x3F579A5423C4B5AE, 0x3F5667B7206E58A6, 0x3F5667B7206E58A6, 0x4009D37FBA93A700, 0x3FE7E895D82FD12C, 0x3FB06295F353F85F, 0x3FAA60E92C9B2838, 0x40B445AC1B514B58, 0x3FD57A7674AAF4EC, 0x40A625F59F6E0BEB, 0x3FF89AABDB713558, 0x40049B26B769364D, 0x40B7506840AE60D0, 0x3FB93C1D1832783A],
        [0x3F803FA2BE17F0C3, 0x4027B961494DE12D, 0x3F7A5AD336AAD6FA, 0x3F86127B21EE21C8, 0x3F5B00B7DD40D747, 0x4035B747970B89A6],
    ),
    (
        [0x4033DC600847B8C0, 0x3F48B61B5B543D1C, 0x3F5AEC4583EC1E2D, 0x3F52266E68F94AD1, 0x3F52266E68F94AD1, 0x3FFFE7539F9FCEA7, 0x0000000000000000, 0x3FB1890810231210, 0x3F9C43D179D2213D, 0x3D06849B86A12B9B, 0x3FF15A9A4CA2B534, 0x0000000000000000, 0x3FDD7E5E5C3AFCBD, 0x4001B89C35A37138, 0x40BE52059FE1440B, 0x3FACCD80DC399B02],
        [0x3F6BB6C5E47DD052, 0x0000000000000000, 0x4020E11D3DB929D8, 0x4026D8FB66C72236, 0x0000000000000000, 0x4033DD0C52402607],
    ),
];

#[rustfmt::skip]
const SYNC_CASES: [([u64; 5], [u64; 2]); 4] = [
    ([0xC05A26EE72604DDD, 0x4083BB80CEF1F701, 0x3F60681E3354FF19, 0x3FD02E49EC205C94, 0xC024EBF1F519D7E4], [0x3FE9AC6578CA9587, 0x3F40703EB8F6236D]),
    ([0xC065D1AA3591A354, 0x408F2D9D33D05B3A, 0x3F6064E3556AC30F, 0x3FFBFAC4FC77F58A, 0xC03174882ADAE910], [0xBFB9F5F5CA9C5C1A, 0x3F6C39628D98BDC6]),
    ([0xC0538FFE7E1B1FFE, 0x407F0BA54619174A, 0x3F60795FF0B9A5AA, 0x3FED80CEBABB019D, 0xC01F4CCA635E9996], [0xBFB49603541A26D0, 0x3F5E0D81D129700E]),
    ([0xC054ACCCD04D599A, 0x407D6A63465ED4C7, 0x3F60800E1914CB5A, 0x3FF3F60F5567EC1F, 0xC0208A3D7371147B], [0xBFDDD5BF754B5F4C, 0x3F6452EB5E0E255C]),
];
#[test]
fn mixed_box_matches_fortran_kernel() {
    // 运行时驱动的同位素：走混合箱，无溶解度上限、可挥发、不分馏。
    let tracer = descriptor(TracerFamily::Isotope, 2.0e-3, 0.0);
    for (inputs, outputs) in BOX_CASES {
        let a = bits(&inputs);
        let mut pools = TracerPools::allocated(0.0);
        pools.subsurface_residue = a[1];
        let box_water = BoxWater {
            atm_precip_mass: a[5],
            deficit_mass: Some(a[6]),
            rnof_mass: a[13],
            evap_liq_mass: a[11],
            evap_ice_mass: a[12],
            dep_liq_mass: a[7],
            dep_ice_mass: a[8],
            water_input: a[10],
            water_beg: a[9],
            water_end: a[14],
            surface_wliq: 0.0,
            wdsrf: 0.0,
            held_lb: -4,
            subl_skin_mm: a[15],
            t_grnd: 273.0,
        };
        assert_eq!(a[3], a[4], "harness ties R_frost to R_vapor");
        let fluxes = mixed_box(&tracer, &SpecialFrac { physics: TracerPhysics::default(), forc_q: 0.0, forc_psrf: 1.0e5, open_water_wind: None }, &mut pools, a[0], a[2], a[3], &box_water);
        assert_eq!(fluxes.trc_input.to_bits(), outputs[0]);
        assert_eq!(fluxes.trc_evap_liq.to_bits(), outputs[1]);
        assert_eq!(fluxes.trc_evap_ice.to_bits(), outputs[2]);
        assert_eq!(fluxes.trc_rnof.to_bits(), outputs[3]);
        assert_eq!(fluxes.r_final.to_bits(), outputs[4]);
    }
}

#[test]
fn sync_aquifer_and_leaf_pseudo_mass_match_fortran() {
    let isotope = descriptor(TracerFamily::Isotope, 2.0052e-3, 0.0);
    let set = TracerSet {
        tracers: vec![isotope.clone()],
    };
    for (inputs, outputs) in SYNC_CASES {
        let a = bits(&inputs);
        let mut pools = TracerPools::allocated(0.0);
        pools.aquifer_ref_mass = a[3];
        let zeros = [0.0; SOISNO_LAYERS];
        sync_tracer_patch_ratio(
            &isotope, &mut pools, a[1], 0, &zeros, &zeros, a[0], 0.0, 0.0, a[2],
        );
        assert_eq!(pools.wa.to_bits(), outputs[0]);

        let mut state = PatchTracerState::allocated(&set);
        state.pools[0].leaf_delta_b = a[4];
        state.pools[0].leaf_water_moles = a[3];
        let term = patch_term(
            history_variable(TracerHistId::LeafDeltaB),
            &isotope,
            0,
            &state,
            1.0,
        );
        match term {
            PatchTerm::Pair { mass, water } => {
                assert_eq!(mass.to_bits(), outputs[1]);
                assert_eq!(water, a[3]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn sync_zeroes_inactive_layers_and_layered_scv() {
    let solute = descriptor(TracerFamily::Solute, 1.0, 0.0);
    let mut pools = TracerPools::allocated(0.0);
    pools.ldew_rain = 3.0;
    pools.wliq_soisno = [9.0; SOISNO_LAYERS];
    pools.solid_soisno = [1.0; SOISNO_LAYERS];
    let mut wliq = [2.0; SOISNO_LAYERS];
    wliq[soisno_slot(3)] = -1.0;
    let wice = [0.5; SOISNO_LAYERS];
    sync_tracer_patch_ratio(
        &solute, &mut pools, 0.0, -2, &wliq, &wice, 4.0, 1.0, 6.0, 0.1,
    );
    assert_eq!(pools.ldew_rain, 0.0);
    assert_eq!(pools.wliq_soisno[soisno_slot(-4)], 0.0);
    assert_eq!(pools.wliq_soisno[soisno_slot(-2)], 0.0);
    assert_eq!(pools.wliq_soisno[soisno_slot(-1)], 2.0 * 0.1);
    assert_eq!(pools.wliq_soisno[soisno_slot(3)], 0.0);
    // 不活动层只清液/冰，固相保留。
    assert_eq!(pools.solid_soisno[soisno_slot(-4)], 1.0);
    assert_eq!(pools.wa, 4.0 * 0.1);
    assert_eq!(pools.wdsrf, 0.1);
    assert_eq!(pools.scv, 0.0);
}

#[test]
fn glacier_overflow_mass_matches_colmmain() {
    let dt = 1800.0;
    let wliq1 = 0.1 * 1000.0 + 3.3;
    let wice1 = 0.1 * 917.0 + 1.7;
    let liq = dt * ((wliq1 - 0.1 * 1000.0) / dt);
    let ice = dt * ((wice1 - 0.1 * 917.0) / dt);
    assert_eq!(
        glacier_overflow_mass(wliq1, wice1, 0.1, dt),
        (0.0 + liq) + ice
    );
    assert_eq!(glacier_overflow_mass(10.0, 10.0, 0.1, dt), 0.0);
}

struct Water {
    wliq: [f64; SOISNO_LAYERS],
    wice: [f64; SOISNO_LAYERS],
}

fn lake_water() -> Water {
    let mut wliq = [0.0; SOISNO_LAYERS];
    let mut wice = [0.0; SOISNO_LAYERS];
    for j in 1..=SOIL_LAYERS as i32 {
        wliq[soisno_slot(j)] = 100.0 + f64::from(j);
    }
    wice[soisno_slot(1)] = 5.0;
    Water { wliq, wice }
}

#[allow(clippy::too_many_arguments)]
fn waterbody_input<'a>(
    water: &'a Water,
    totwb: f64,
    endwb: f64,
    ratios: &'a [f64],
    forced: &'a [bool],
    hist_sample: bool,
) -> WaterbodyInput<'a> {
    WaterbodyInput {
        ipatch: 3,
        snl: 0,
        deltim: 1800.0,
        forc_rain: 2.0e-4,
        forc_snow: 0.0,
        lake_deficit: 1.0e-5,
        rnof: 1.5e-4,
        qseva: 3.0e-5,
        qsubl: 0.0,
        qsdew: 1.0e-6,
        qfros: 0.0,
        endwb,
        totwb,
        errorw: 0.0,
        wa: 0.0,
        wdsrf: 0.0,
        scv: 0.0,
        t_grnd: 290.0,
        forc_q: 0.01,
        forc_psrf: 1.0e5,
        forc_us: 1.0,
        forc_vs: 1.0,
        wliq_soisno: &water.wliq,
        wice_soisno: &water.wice,
        use_dynamic_lake: false,
        subl_skin_mm: 1.0,
        hist_sample,
        precip_ratio: ratios,
        vapor_ratio: ratios,
        runtime_forced: forced,
        catch_lateral_flow: false,
    }
}

/// 宿主步末水量与 (输入 − 输出) 自洽时，混合箱应精确闭合（守恒检查不报）。
#[test]
fn waterbody_mixed_box_conserves_solute() {
    let set = TracerSet {
        tracers: vec![descriptor(TracerFamily::Solute, 1.0, 0.3)],
    };
    let water = lake_water();
    let dt = 1800.0;
    let endwb: f64 = (1..=SOIL_LAYERS as i32)
        .map(|j| water.wliq[soisno_slot(j)] + water.wice[soisno_slot(j)])
        .sum();
    let net = ((2.0e-4 + 1.0e-5) * dt + 1.0e-6 * dt) - (3.0e-5 * dt + 1.5e-4 * dt);
    let totwb = endwb - net;
    let mut state = PatchTracerState::allocated(&set);
    for j in 1..=SOIL_LAYERS as i32 {
        let slot = soisno_slot(j);
        state.pools[0].wliq_soisno[slot] = (water.wliq[slot] - 0.01) * 0.3;
    }
    // 转入前残留在冠层的溶质进地表隔离库，随后有液相载体就并回顶层。
    state.pools[0].ldew_rain = 0.2;
    let mut snapshot = BalanceSnapshot::new(&set);
    let mut tracker = BalanceTracker::default();
    let ratios = [0.3];
    let forced = [false];
    let xerr = tracer_waterbody_patch(
        &set,
        TracerPhysics::default(),
        &mut state,
        &mut snapshot,
        &mut tracker,
        &waterbody_input(&water, totwb, endwb, &ratios, &forced, true),
    )
    .unwrap();
    assert!(xerr < 1.0e-12, "xerr {xerr}");
    assert_eq!(tracker.balance_nbad, 0);
    let pools = &state.pools[0];
    assert_eq!(pools.ldew_rain, 0.0);
    assert_eq!(pools.surface_residue, 0.0);
    // 非挥发溶质不随蒸发离开。
    assert_eq!(state.acc[0].evap, 0.0);
    assert!(state.acc[0].rnof > 0.0);
    // history 已累加。
    assert_eq!(state.water_acc.soil[0], water.wliq[soisno_slot(1)] + 5.0);
}

#[test]
fn waterbody_fixed_signature_isotope_uses_r_init() {
    let set = TracerSet {
        tracers: vec![descriptor(TracerFamily::Isotope, 2.0e-3, 0.0)],
    };
    let water = lake_water();
    let mut state = PatchTracerState::allocated(&set);
    let mut snapshot = BalanceSnapshot::new(&set);
    let mut tracker = BalanceTracker::default();
    let ratios = [1.0];
    let forced = [false];
    tracer_waterbody_patch(
        &set,
        TracerPhysics::default(),
        &mut state,
        &mut snapshot,
        &mut tracker,
        &waterbody_input(&water, 0.0, 0.0, &ratios, &forced, false),
    )
    .unwrap();
    let dt = 1800.0;
    let r = 2.0e-3;
    let water_input = ((2.0e-4 + 0.0) + 1.0e-5) * dt + (1.0e-6 * dt + 0.0 * dt);
    assert_eq!(state.acc[0].precip, water_input * r);
    assert_eq!(state.acc[0].rnof, (1.5e-4 * dt) * r);
    assert_eq!(state.acc[0].soilevap, (3.0e-5 * dt) * r);
    assert_eq!(
        state.pools[0].wliq_soisno[soisno_slot(2)],
        water.wliq[soisno_slot(2)] * r
    );
    // hist_sample = false：不累加 history。
    assert_eq!(state.water_acc.soil, [0.0; SOIL_LAYERS]);
}

#[test]
fn glacier_folds_layered_snow_and_rejects_aquifer_reference() {
    let set = TracerSet {
        tracers: vec![descriptor(TracerFamily::Isotope, 2.0e-3, 0.0)],
    };
    let water = lake_water();
    let ratios = [1.0];
    let forced = [false];
    let input = GlacierInput {
        ipatch: 1,
        deltim: 1800.0,
        prc_rain: 0.0,
        prl_rain: 0.0,
        prc_snow: 1.0e-4,
        prl_snow: 0.0,
        rnof: 0.0,
        qseva: 0.0,
        qsubl: 1.0e-6,
        qsdew: 0.0,
        qfros: 0.0,
        endwb: 0.0,
        totwb: 0.0,
        glacier_overflow_mass: 0.0,
        errorw: 0.0,
        wdsrf: 0.0,
        scv: 12.0,
        t_grnd: 260.0,
        forc_q: 0.002,
        forc_psrf: 7.0e4,
        wliq_soisno: &water.wliq,
        wice_soisno: &water.wice,
        subl_skin_mm: 1.0,
        precip_ratio: &ratios,
        vapor_ratio: &ratios,
        runtime_forced: &forced,
        catch_lateral_flow: false,
    };
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].wliq_soisno[soisno_slot(-1)] = 0.25;
    state.pools[0].wice_soisno[soisno_slot(0)] = 0.5;
    let mut snapshot = BalanceSnapshot::new(&set);
    let mut tracker = BalanceTracker::default();
    tracer_glacier_patch(
        &set,
        TracerPhysics::default(),
        &mut state,
        &mut snapshot,
        &mut tracker,
        &input,
    )
    .unwrap();
    // 折叠进 scv 的部分在快照里，之后按 R_init 重建。
    assert_eq!(snapshot.tracers[0].storage_comp[6], (0.0 + 0.25) + 0.5);
    assert_eq!(state.pools[0].wliq_soisno[soisno_slot(-1)], 0.0);
    assert_eq!(state.pools[0].scv, 12.0 * 2.0e-3);
    assert_eq!(state.water_acc.scv, 12.0);

    let mut frac_state = PatchTracerState::allocated(&set);
    tracer_glacier_patch(
        &set,
        TracerPhysics {
            fractionation: true,
            ..Default::default()
        },
        &mut frac_state,
        &mut snapshot,
        &mut tracker,
        &input,
    )
    .unwrap();

    let mut ref_state = PatchTracerState::allocated(&set);
    ref_state.aquifer_ref_water = 100.0;
    assert!(tracer_glacier_patch(
        &set,
        TracerPhysics::default(),
        &mut ref_state,
        &mut snapshot,
        &mut tracker,
        &input,
    )
    .is_err());
}
