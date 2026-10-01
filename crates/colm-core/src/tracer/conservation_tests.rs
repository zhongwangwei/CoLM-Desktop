//! `MOD_Tracer_Conservation` 的测试。
//!
//! 逐位参照来自 `harness.F90`：把 `tracer_balance_check` 循环体的算式原样抄成独立
//! 子程序，用内核同样的 `gfortran -O2 -fdefault-real-8` 编译（其 GIMPLE 的收缩位置与
//! `MOD_Tracer_Conservation.F90.273t.optimized` 一致：`balance_tol`/`signature_tol`
//! 为 FMA，`dS/out_minus_water_R` 为 FNMA，其余独立舍入），输出十六进制位型。
//! `E12.5` 的期望串是同一程序 `WRITE(*,'(E12.5)')` 的输出。

use super::*;
use crate::tracer::{
    soisno_slot, PatchTracerState, ReactionMode, StateOwner, TracerDescriptor, TracerFamily,
    TracerSet,
};

pub(crate) fn descriptor(family: TracerFamily, ref_ratio: f64, init_conc: f64) -> TracerDescriptor {
    let isotope = family == TracerFamily::Isotope;
    TracerDescriptor {
        name: "T".to_owned(),
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

fn set_of(tracers: Vec<TracerDescriptor>) -> TracerSet {
    TracerSet { tracers }
}
const FORMAT_CASES: [(u64, &str); 9] = [
    (0x0000000000000000, " 0.00000E+00"),
    (0x8000000000000000, "-0.00000E+00"),
    (0x3F543A23F7B2D2C6, " 0.12346E-02"),
    (0xC12E847F33333333, "-0.10000E+07"),
    (0x270EFCABEFA5CC93, " 0.15000-119"),
    (0xE98A20DF0DCD3AF1, "-0.25000+201"),
    (0x419D6F3454000000, " 0.12346E+09"),
    (0x3FEFFFFEEE32223C, " 0.10000E+01"),
    (0x3F0A36E2EB1C432D, " 0.50000E-04"),
];

#[rustfmt::skip]
const BALANCE_CASES: [([u64; 16], [u64; 13]); 6] = [
    (
        [0x402BBFBF402F7F7F, 0x404223BDEF0A477C, 0x4000BFF9AC117FF4, 0x3F3CEA5E1EB09D71, 0x3FEA70E62B34E1CC, 0x3FE70C267C2E184D, 0xBE8091BB6C588436, 0xBEAD01CFF85CBF8D, 0x3DD50F1FD1336E53, 0x3DE62837C8225171, 0x3F6080DB4BE52A19, 0x3FA4FD50DC29FAA2, 0x3FFE070ECE7C0E1E, 0x3FD343A1C6A68744, 0x3FDE63802C3CC700, 0x3FB76A2AB02ED455],
        [0xC036F3CFE80EE6AC, 0xC036F3CFE80EE6AC, 0xC03667A1A79393B0, 0x4000B83B735C8183, 0x3FF8BC0A7AD5BEAD, 0x3FEA69101C08F61F, 0x3FE70AA4105EF9B3, 0x3E2F2C163FAE9904, 0x3F0A36E2EB1C432D, 0x3DECE9DEAC0ABE0C, 0x3FF43BEED8B487F6, 0xC036F3CFF8A20616, 0x0000000000000000],
    ),
    (
        [0x402C454C8EB454D6, 0x402FDFD79FAFBFAF, 0x3FD019B1CA203363, 0x3F34CB01E5484E56, 0x3FF74CD8C5EE99B2, 0x3FE31E71C9263CE4, 0x0000000000000000, 0x0000000000000000, 0x0000000000000000, 0xBDF9AF620A473C7B, 0x3F607D3CD753D2DC, 0x40047D2369B8FA47, 0x4007E86BE61FD0D8, 0x40021144E924228A, 0x3FFDAB9BB0BB5737, 0x3FE931C14CB26383],
        [0xBCC0000000000000, 0xBCC0000000000000, 0xBFFCE9760AD2BB67, 0x3FCF6E471F6D112E, 0x400064B97CFD26BC, 0x3FF73D8ED954A709, 0x3FE311764E3DFD60, 0x3E1B65A6318AD22E, 0x3F0A36E2EB1C432D, 0x3DE426F2E02781E1, 0x3FF34FC5F56704E3, 0xBCC0000000000000, 0x0000000000000000],
    ),
    (
        [0x404675740950EAE8, 0x4031A551416B4AA2, 0x3FFBDB4B3397B696, 0x3F398FE660B6323C, 0x3FECD5518FB9AAA3, 0x3FA870DD1030E1BA, 0xBE8F17E7813BF130, 0x3ECE4E1756D381B9, 0xBDFC55AAE31D33F9, 0xBDF19A66C167262E, 0x3F607F8E3E6A9956, 0x3FFE33F392DC67E8, 0x3FF5E4E8E56BC9D2, 0x4002F349A7B5E693, 0x3FF551BDAFEAA37B, 0x3FDFEC4B283FD896],
        [0x403A7AAB50636316, 0x403A7AAB5063636F, 0x403B449DAB52F23B, 0x3FFBD00179563441, 0x3FEE354AA649D12C, 0x3FECBF55DBAF7D67, 0x3FA7ED31E607170D, 0x3E334BD79E7CF8B8, 0x3F0A36E2EB1C432D, 0x3DE810EB47C93DD1, 0x3FEAE0AD16FCEB1B, 0x403A7AAB891C94D4, 0xBD5621A3ECB6EB0D],
    ),
    (
        [0x403605241A000A48, 0x403F62428D92C485, 0x3FFB9C9E4517393C, 0x3F27CC84D7BCE8EA, 0x3FCFBFAED43F7F5E, 0x3FE00ED0E6201DA2, 0x3E9450485D9CEFED, 0x3ED2FC53A04D4AFC, 0x3DB89097D2F62A83, 0x3DFEEBAD6BBDCF87, 0x3F60773394635C0B, 0x3FA0D372F421A6E6, 0x4002BA21FC557444, 0x3FFDDC956A3BB92A, 0x3FDFD945DE3FB28C, 0x3FE66B4F7B2CD69F],
        [0xC024ADFD54A4F4B7, 0xC024ADFD54A4F4B7, 0xC022BA458F8C610C, 0x3FFB89589E5F24B2, 0x3FE7E0018CEA6D8B, 0x3FCF9EE847BDAC6F, 0x3FE00347BB2FF890, 0x3E2AF79366EE4871, 0x3F0A36E2EB1C432D, 0x3DE7DB14CA00D8D9, 0x3FF7957B95676F24, 0xC024ADFCB29A3386, 0x0000000000000000],
    ),
    (
        [0x403FBCC5C537798B, 0x4046D7A6F39FAF4E, 0x3F807B8A8020F715, 0xBEC7979B43C4B021, 0x3FF7BF60C1AF7EC2, 0x3FDDEA92243BD524, 0x3E9A95834F31E85B, 0xBEC630094D6C84C6, 0x3DFEF85154188E3E, 0x3DFCEFFA30D34115, 0x3F607425FF550FCE, 0x3FF5ADE7AECB5BCF, 0x40012463FD8248C8, 0x4004C2064939840C, 0x3FDAA6DB6B354DB7, 0x3FEB15D301F62BA6],
        [0xC02801EDD3843342, 0xC02801EDD3843363, 0xC02BE674F88F73F6, 0x3F6EAC80F036BD76, 0x3FFF24AC947BB67D, 0x3FF7BBF3B721D964, 0x3FDDCEB7B2AD5CF9, 0x3E33A031C86AAFCE, 0x3F0A36E2EB1C432D, 0x3DE489548CB11F53, 0x3FF7AC9D76A9BE05, 0xC02801EE1EF996D0, 0x3D30B8C375F201DE],
    ),
    (
        [0x402977B67DC2EF6E, 0x4037F1A5B3DBE34C, 0x4006E1BFD0ADC380, 0xBF3E210DDC23AEA7, 0x3FF5FDB9226BFB72, 0x3FECAFB186F95F63, 0xBE7D409E181DCCD1, 0xBEC6B7BF2A33D346, 0x3DFB9AD9DCB6FDEC, 0xBDF7E1CDAAAE430E, 0x3F60933B03690FD1, 0x3FDC08CD30B8119B, 0x3FDF9A6563BF34CB, 0x3FF26CF8C8A4D9F1, 0x3FF6618B6D6CC317, 0x3FB709EFF82E13E0],
        [0xC027991600C069F2, 0xC027991600C06BBC, 0xC0266C09150AA882, 0x4006DFB3FD664E20, 0x4002260351794ECD, 0x3FF5F22163568AB3, 0x3FECAE33A7F5781F, 0x3E249386A405782F, 0x3F0A36E2EB1C432D, 0x3DF3B95C49D24399, 0x3FF7CD469776118D, 0xC02799165F477A5E, 0x3D6C98DE5F387965],
    ),
];
#[test]
fn e12_5_matches_gfortran() {
    for (bits, expected) in FORMAT_CASES {
        assert_eq!(
            fortran_e12_5(f64::from_bits(bits)),
            expected,
            "bits {bits:016X}"
        );
    }
    assert_eq!(fortran_e12_5(f64::NAN), "         NaN");
    assert_eq!(fortran_e12_5(f64::NEG_INFINITY), "   -Infinity");
    assert_eq!(fortran_i(42, 8), "      42");
    assert_eq!(fortran_i(-1, 3), " -1");
    assert_eq!(fortran_i(12345, 4), "****");
}

#[test]
fn balance_check_matches_fortran_kernel() {
    for (inputs, outputs) in BALANCE_CASES {
        let a: Vec<f64> = inputs.iter().map(|b| f64::from_bits(*b)).collect();
        let o: Vec<f64> = outputs.iter().map(|b| f64::from_bits(*b)).collect();
        // 同位素、init_delta = 0：R_init = ref_ratio*(1 + 0/1000) = a11。
        let set = set_of(vec![descriptor(TracerFamily::Isotope, a[10], 0.0)]);
        let mut state = PatchTracerState::allocated(&set);
        state.pools[0].wa = a[0];
        state.step[0].storage_beg = a[1];
        state.acc[0].precip = a[2];
        state.acc[0].vapor_exchange = a[3];
        state.acc[0].evap = a[4];
        state.acc[0].rnof = a[5];
        state.step[0].reactive_source_step = a[6];
        state.step[0].numerical_residual_step = a[7];
        state.step[0].numerical_water_step = a[9];
        let snapshot = BalanceSnapshot::new(&set);
        let mut tracker = BalanceTracker::default();
        let xerr = tracer_balance_check(
            &set,
            TracerPhysics::default(),
            &mut state,
            &snapshot,
            &mut tracker,
            &BalanceCheckInput {
                ipatch: 7,
                snl: 0,
                deltim: 1.0,
                patchtype: Some(0),
                water_err: Some(a[8]),
                water_ds: Some(a[11]),
                water_input: Some(a[12]),
                water_output: Some(a[13]),
                water_evap: Some(a[14]),
                water_rnof: Some(a[15]),
                flood_heterogeneous: None,
                catch_lateral_flow: false,
                runtime_forced: &[false],
            },
        );
        assert_eq!(state.step[0].balance_err.to_bits(), outputs[0]);
        assert_eq!(xerr.to_bits(), o[1].abs().to_bits());
        let bad = o[1].abs() > o[7];
        assert_eq!(tracker.balance_nbad, i32::from(bad));
        if bad {
            let d = &tracker.balance_worst_diag;
            assert_eq!(tracker.balance_worst_err.to_bits(), outputs[1]);
            assert_eq!(d[0].to_bits(), outputs[11]);
            assert_eq!(d[5].to_bits(), outputs[12]);
            assert_eq!(d[10].to_bits(), outputs[2]);
            assert_eq!(d[11].to_bits(), outputs[3]);
            assert_eq!(d[12].to_bits(), outputs[4]);
            assert_eq!(d[17].to_bits(), outputs[5]);
            assert_eq!(d[18].to_bits(), outputs[6]);
            assert_eq!(tracker.balance_worst_ipatch, 7);
            assert_eq!(tracker.balance_worst_itrc, 1);
            assert_eq!(tracker.balance_worst_ptype, 0);
        }
        let sig_bad = o[10] > o[9];
        assert_eq!(tracker.signature_nbad, i32::from(sig_bad));
        if sig_bad {
            assert_eq!(tracker.signature_worst_tol.to_bits(), outputs[9]);
            assert_eq!(tracker.signature_worst_abs.to_bits(), outputs[10]);
        }
        assert_eq!(state.acc[0].water_precip, a[12].max(0.0));
        assert_eq!(state.acc[0].water_rnof, a[15].max(0.0));
    }
}

#[test]
fn harness_covers_both_balance_outcomes() {
    let bad: Vec<bool> = BALANCE_CASES
        .iter()
        .map(|(_, o)| f64::from_bits(o[1]).abs() > f64::from_bits(o[7]))
        .collect();
    assert!(bad.contains(&true) && bad.contains(&false), "{bad:?}");
}

#[test]
fn save_storage_then_check_closes_a_simple_budget() {
    let set = set_of(vec![descriptor(TracerFamily::Solute, 1.0, 0.5)]);
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].wliq_soisno[soisno_slot(1)] = 2.0;
    // 不活动的雪层槽位（snl = 0）不计入。
    state.pools[0].wliq_soisno[soisno_slot(-2)] = 99.0;
    state.pools[0].wa = 3.0;
    state.step[0].rnof_step = 5.0;
    let mut snapshot = BalanceSnapshot::new(&set);
    tracer_save_storage(
        &set,
        TracerPhysics::default(),
        &mut state,
        &mut snapshot,
        &SaveStorageInput {
            snl: 0,
            waterstorage: None,
            runtime_forced: &[false],
        },
    );
    assert_eq!(state.step[0].storage_beg, 5.0);
    assert_eq!(state.step[0].rnof_step, 0.0);
    assert_eq!(snapshot.tracers[0].storage_comp[1], 2.0);
    // 一步：降水 1 进土壤，蒸发 0.25 离开 wa，径流 0.5 离开土壤。
    state.acc[0].precip += 1.0;
    state.acc[0].evap += 0.25;
    state.acc[0].rnof += 0.5;
    state.pools[0].wliq_soisno[soisno_slot(1)] += 0.5;
    state.pools[0].wa -= 0.25;
    let mut tracker = BalanceTracker::default();
    let xerr = tracer_balance_check(
        &set,
        TracerPhysics::default(),
        &mut state,
        &snapshot,
        &mut tracker,
        &BalanceCheckInput {
            ipatch: 1,
            snl: 0,
            deltim: 1800.0,
            patchtype: Some(0),
            water_err: Some(0.0),
            water_ds: None,
            water_input: None,
            water_output: None,
            water_evap: None,
            water_rnof: None,
            flood_heterogeneous: None,
            catch_lateral_flow: false,
            runtime_forced: &[false],
        },
    );
    assert_eq!(xerr, 0.0);
    assert_eq!(state.step[0].balance_err, 0.0);
    assert_eq!(tracker, BalanceTracker::default());
    let report = tracker.report(1, 0, 0);
    assert!(report.lines.is_empty());
    assert!(report.abort.is_none());
}

#[test]
fn fixed_signature_resyncs_waterstorage_only_for_isotopes() {
    let set = set_of(vec![
        descriptor(TracerFamily::Isotope, 2.0e-3, 0.0),
        descriptor(TracerFamily::Solute, 1.0, 0.5),
    ]);
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].waterstorage = 7.0;
    state.pools[1].waterstorage = 7.0;
    let mut snapshot = BalanceSnapshot::new(&set);
    tracer_save_storage(
        &set,
        TracerPhysics::default(),
        &mut state,
        &mut snapshot,
        &SaveStorageInput {
            snl: 0,
            waterstorage: Some(10.0),
            runtime_forced: &[false, false],
        },
    );
    assert_eq!(state.pools[0].waterstorage, 10.0 * 2.0e-3);
    assert_eq!(state.pools[1].waterstorage, 7.0);
    // 运行时驱动的同位素不重置。
    state.pools[0].waterstorage = 7.0;
    tracer_save_storage(
        &set,
        TracerPhysics::default(),
        &mut state,
        &mut snapshot,
        &SaveStorageInput {
            snl: 0,
            waterstorage: Some(10.0),
            runtime_forced: &[true, false],
        },
    );
    assert_eq!(state.pools[0].waterstorage, 7.0);
}

#[test]
fn reactive_decay_books_the_source_sink() {
    let mut solute = descriptor(TracerFamily::Solute, 1.0, 0.0);
    solute.reaction_mode = ReactionMode::FirstOrder;
    solute.reactive_decay_rate = 1.0e-4;
    let set = set_of(vec![solute]);
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].wa = -4.0; // 亏欠记账：不衰减。
    state.pools[0].wdsrf = 2.0;
    state.pools[0].solid_soisno[soisno_slot(3)] = 1.0;
    let fraction = set.tracers[0].reactive_decay_fraction(1800.0);
    tracer_apply_reactive_processes(&set, &mut state, 0, 1800.0);
    let wdsrf = 2.0 * (1.0 - fraction);
    let solid = 1.0 * (1.0 - fraction);
    assert_eq!(state.pools[0].wa, -4.0);
    assert_eq!(state.pools[0].wdsrf, wdsrf);
    assert_eq!(state.pools[0].solid_soisno[soisno_slot(3)], solid);
    let expected = ((((0.0 + wdsrf) - 2.0) + solid) - 1.0) + 0.0;
    assert_eq!(state.step[0].reactive_source_step, expected);
}

#[test]
fn report_lines_and_abort_thresholds() {
    let mut tracker = BalanceTracker {
        balance_worst_err: -2.5e-6,
        balance_nbad: 3,
        balance_worst_ipatch: 12,
        balance_worst_itrc: 2,
        balance_worst_ptype: 3,
        resid_nbad: 1,
        resid_worst_abs: 1.0e-9,
        ..BalanceTracker::default()
    };
    let report = tracker.report(2, 5, 0);
    assert!(report.abort.is_none());
    assert_eq!(report.lines.len(), 5);
    assert!(report.lines[0].starts_with(
        "TRC_BAL step report: nbad_entries=       3 worst_abs_err= 0.25000E-05 @ipatch=      12 \
         itrc=   2 ptype=  3 owner=   0 err= 0.00000E+00"
    ));
    assert!(report.lines[1]
        .starts_with("TRC_BAL_DCOMP @ipatch=      12 itrc=   2 ptype=  3 d_ldew= 0.00000E+00"));
    assert!(report.lines[2].ends_with(" qcharge= 0.00000E+00"));
    assert!(report.lines[3].ends_with(" solid= 0.00000E+00"));
    assert_eq!(
        report.lines[4],
        "TRC_BAL residual note (excluded from tol check): n_entries=       1 worst_abs= 0.10000E-08"
    );
    // 报告后清零。
    assert_eq!(tracker, BalanceTracker::default());

    let mut tracker = BalanceTracker {
        balance_nbad: 6,
        ..BalanceTracker::default()
    };
    let report = tracker.report(1, 5, 0);
    assert_eq!(
        report.abort.as_deref(),
        Some("TRC_BAL hard failure: tracer balance error exceeds aggregate threshold")
    );
    let mut tracker = BalanceTracker {
        resid_hard_nbad: 1,
        signature_nbad: 9,
        ..BalanceTracker::default()
    };
    let report = tracker.report(1, 5, 0);
    assert_eq!(
        report.abort.as_deref(),
        Some("TRC_BAL hard failure: numerical residual exceeds independent threshold")
    );
    assert!(report.lines[0].starts_with("TRC_BAL residual hard failure: n_entries=       1"));
    let mut tracker = BalanceTracker::default();
    assert_eq!(tracker.report(0, 0, 0), BalanceReport::default());
}
