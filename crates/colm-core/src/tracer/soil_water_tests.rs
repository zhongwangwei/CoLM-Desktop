//! 不变量：均一比值保持、示踪物总量与记账通量闭合、非挥发溶质不随蒸发离开。

use super::*;
use crate::tracer::TracerDescriptor;
use crate::tracer::{ReactionMode, StateOwner, TracerFamily, TracerPools};

const DT: f64 = 1800.0;
const NL: i32 = SOIL_LAYERS as i32;
const R_ISO: f64 = 2.0052e-3;

fn descriptor(name: &str, family: TracerFamily) -> TracerDescriptor {
    TracerDescriptor {
        name: name.to_owned(),
        category: if family == TracerFamily::Isotope {
            "isotope".to_owned()
        } else {
            "solute".to_owned()
        },
        family,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: String::new(),
        mol_weight: 18.0,
        ref_ratio: R_ISO,
        init_delta: 0.0,
        init_conc: 0.0,
        precip_default_conc: 0.0,
        vapor_default_conc: 0.0,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        charge: 0,
    }
}

fn no_diffusion() -> SoilWaterOptions {
    SoilWaterOptions {
        soil_diffusion: false,
        soil_vapor_diffusion: false,
        ..SoilWaterOptions::default()
    }
}

/// 一个 patch 一步的宿主水量；WATER 之后的层水量按示踪物眼中的同一套路径推出来，
/// 使 `water_shadow` 与宿主一致。
struct Scenario {
    snl: i32,
    qlayer: [f64; SOIL_LAYERS + 1],
    wliq: [f64; SOISNO_LAYERS],
    wice: [f64; SOISNO_LAYERS],
    wliq_bef: [f64; SOISNO_LAYERS],
    wice_bef: [f64; SOISNO_LAYERS],
    wblc_ice_sink: [f64; SOIL_LAYERS],
    etroot_actual: [f64; SOIL_LAYERS],
    snow_qout: [f64; MAX_SNOW_LAYERS],
    etroot_aquifer: f64,
    qinfl: f64,
    rsur: f64,
    rsub: f64,
    qseva: f64,
    qsdew: f64,
    qsubl: f64,
    qfros: f64,
    pg_rain: f64,
    wa: f64,
    wa_bef: f64,
    wdsrf: f64,
    wdsrf_bef: f64,
    irrig: f64,
    waterstorage: f64,
}

impl Scenario {
    /// 无雪：蒸发从地表池扣，露/霜进第 1 层，第 3 层内部冻结一点水。
    fn bare_soil() -> Self {
        let s = soisno_slot;
        let mut wliq_bef = [0.0; SOISNO_LAYERS];
        let mut wice_bef = [0.0; SOISNO_LAYERS];
        for j in 1..=NL {
            wliq_bef[s(j)] = 20.0 + 3.0 * f64::from(j);
        }
        wice_bef[s(1)] = 2.0;
        wice_bef[s(2)] = 1.0;
        let mut qlayer = [0.0; SOIL_LAYERS + 1];
        for (j, q) in qlayer.iter_mut().enumerate().skip(1).take(SOIL_LAYERS - 1) {
            *q = if j % 3 == 0 { -2.0e-5 } else { 3.0e-5 };
        }
        let mut etroot_actual = [0.0; SOIL_LAYERS];
        etroot_actual[0] = 0.05;
        etroot_actual[1] = 0.08;
        etroot_actual[3] = 0.02;
        let qseva = 2.0e-4;
        let qsdew = 1.0e-5;
        let qfros = 3.0e-6;
        let qsubl = 1.0e-6;
        let pg_rain = 1.0e-3;
        let rsur = 2.0e-4;
        let wdsrf_bef = 2.0;
        let wdsrf = 1.0;
        let irrig = 5.0e-5;
        // 地表池闭合：wdsrf_bef + (pg_rain + irrig - qseva)*dt = wdsrf + (rsur + qinfl)*dt。
        let qinfl = (wdsrf_bef + (pg_rain + irrig - qseva) * DT - wdsrf - rsur * DT) / DT;
        let rsub = 1.0e-5;
        let qcharge = 4.0e-6;
        let etroot_aquifer = 0.1;
        let wa_bef = 5000.0;
        let wa = wa_bef - etroot_aquifer + qcharge * DT;

        let mut scen = Self {
            snl: 0,
            qlayer,
            wliq: [0.0; SOISNO_LAYERS],
            wice: wice_bef,
            wliq_bef,
            wice_bef,
            wblc_ice_sink: [0.0; SOIL_LAYERS],
            etroot_actual,
            snow_qout: [0.0; MAX_SNOW_LAYERS],
            etroot_aquifer,
            qinfl,
            rsur,
            rsub,
            qseva,
            qsdew,
            qsubl,
            qfros,
            pg_rain,
            wa,
            wa_bef,
            wdsrf,
            wdsrf_bef,
            irrig,
            waterstorage: 40.0,
        };
        // 第 1 层冰：外部霜/升华；第 3 层：冻结 0.5 mm。
        scen.wice[s(1)] = wice_bef[s(1)] + (qfros - qsubl) * DT;
        scen.wice[s(3)] = wice_bef[s(3)] + 0.5;
        scen.wliq = scen.shadow(qcharge);
        scen.wliq[s(3)] -= 0.5;
        scen
    }

    /// 按 `tracer_soil_water` 的顺序推 WATER 之后的液态水。
    fn shadow(&self, qcharge: f64) -> [f64; SOISNO_LAYERS] {
        let s = soisno_slot;
        let mut w = self.wliq_bef;
        for j in 1..=NL {
            let k = soil_slot(j);
            if self.etroot_actual[k] > TRC_TINY && self.wliq_bef[s(j)] > TRC_TINY {
                w[s(j)] -= self.etroot_actual[k];
            }
        }
        if self.qinfl > 0.0 {
            w[s(1)] = DT.mul_add(self.qinfl, w[s(1)]);
        }
        for j in 1..NL {
            let q = self.qlayer[j as usize];
            w[s(j)] = (-DT).mul_add(q, w[s(j)]);
            w[s(j + 1)] = DT.mul_add(q, w[s(j + 1)]);
        }
        w[s(NL)] = (-DT).mul_add(qcharge, w[s(NL)]);
        w[s(NL)] -= DT * self.rsub;
        if self.snl == 0 {
            w[s(1)] = DT.mul_add(self.qsdew, w[s(1)]);
        }
        w
    }

    fn input<'a>(&'a self, ratios: &'a [f64]) -> SoilWaterInput<'a> {
        let qcharge = qcharge_trc(self.wa, self.wa_bef, self.etroot_aquifer, 0.0, DT);
        let snow = self.snl < 0;
        SoilWaterInput {
            ipatch: 1,
            deltim: DT,
            snl: self.snl,
            qlayer: &self.qlayer,
            qinfl: self.qinfl,
            qcharge,
            rsur: self.rsur,
            rsub: self.rsub,
            qseva_in: self.qseva,
            qsdew_in: self.qsdew,
            qsubl_in: self.qsubl,
            qfros_in: self.qfros,
            qseva_soil: 0.0,
            qsdew_soil: 0.0,
            qsubl_soil: 0.0,
            qfros_soil: 0.0,
            qseva_snow: 0.0,
            qsdew_snow: 0.0,
            qsubl_snow: 0.0,
            qfros_snow: 0.0,
            sm: 0.0,
            fsno: if snow { 1.0 } else { 0.0 },
            split_soilsnow: false,
            wliq_soisno: &self.wliq,
            wice_soisno: &self.wice,
            wliq_soisno_bef: &self.wliq_bef,
            wice_soisno_bef: &self.wice_bef,
            wa: self.wa,
            wa_bef: self.wa_bef,
            wdsrf: self.wdsrf,
            wdsrf_bef: self.wdsrf_bef,
            wetwat: 0.0,
            wetwat_bef: 0.0,
            pg_rain: self.pg_rain,
            pg_snow: 0.0,
            wblc_ice_sink: &self.wblc_ice_sink,
            etroot_actual: &self.etroot_actual,
            etroot_aquifer: self.etroot_aquifer,
            qflx_irrig_ground: self.irrig,
            waterstorage_patch: Some(self.waterstorage),
            imperv_evap_wdsrf: Some(0.0),
            imperv_evap_soil: Some(0.0),
            imperv_subl_soil: Some(0.0),
            snow_qout_layer: snow.then_some(&self.snow_qout),
            qgtop_solver: None,
            tleaf: None,
            t_soisno: None,
            forc_q: None,
            forc_psrf: None,
            lai: Some(1.0),
            rst: None,
            ra: None,
            rss: None,
            dz_soi: None,
            porsl: None,
            dz_sno: None,
            flood_tracer_input: None,
            flood_infil_water: None,
            etroot_surface: Some(0.0),
            dew_overflow: Some(0.0),
            frost_displaced: Some(0.0),
            late_surface_runoff: Some(0.0),
            rsub_source_layer: None,
            rsub_source_surface: None,
            rsub_source_aquifer: None,
            permeable_soil: None,
            precip_ratio: ratios,
            vapor_ratio: ratios,
            has_vapor: None,
        }
    }

    /// 每个池子都按比值 `ratio` 填满；叶片水量非零以免触发叶片释放。
    fn state(&self, set: &TracerSet, ratio: &[f64]) -> PatchTracerState {
        let mut state = PatchTracerState::allocated(set);
        for (itrc, pools) in state.pools.iter_mut().enumerate() {
            let r = ratio[itrc];
            for j in (self.snl + 1)..=NL {
                pools.wliq_soisno[soisno_slot(j)] = r * self.wliq_bef[soisno_slot(j)];
                pools.wice_soisno[soisno_slot(j)] = r * self.wice_bef[soisno_slot(j)];
            }
            pools.wa = r * self.wa_bef;
            pools.wdsrf = r * self.wdsrf_bef;
            pools.waterstorage = r * self.waterstorage;
            pools.leaf_water_moles = 1.0;
            state.step[itrc].pg_rain_ground = r * self.pg_rain * DT;
        }
        state
    }
}

fn pool_total(p: &TracerPools) -> f64 {
    let layers: f64 = (0..SOISNO_LAYERS)
        .map(|k| p.wliq_soisno[k] + p.wice_soisno[k] + p.solid_soisno[k])
        .sum();
    layers
        + p.wa
        + p.aquifer_ref_mass
        + p.wdsrf
        + p.wetwat
        + p.surface_residue
        + p.subsurface_residue
        + p.surface_solid
        + p.subsurface_solid
        + p.waterstorage
        + p.waterstorage_solid
        + p.leaf_iso_storage
}

/// `Δstorage = pg_rain_ground (+ sm_carry) + Δprecip - Δevap - Δrnof + Δnumerical_residual`。
fn assert_closed(before: &PatchTracerState, after: &PatchTracerState, itrc: usize, sm: bool) {
    let input =
        before.step[itrc].pg_rain_ground + if sm { before.step[itrc].sm_carry } else { 0.0 };
    let (a0, a1) = (&before.acc[itrc], &after.acc[itrc]);
    let booked = input + (a1.precip - a0.precip) - (a1.evap - a0.evap) - (a1.rnof - a0.rnof)
        + (after.step[itrc].numerical_residual_step - before.step[itrc].numerical_residual_step);
    let change = pool_total(&after.pools[itrc]) - pool_total(&before.pools[itrc]);
    let scale = pool_total(&before.pools[itrc]).abs().max(1.0);
    assert!(
        (change - booked).abs() <= 1.0e-12 * scale,
        "tracer {itrc}: storage change {change:e} != booked {booked:e}"
    );
}

#[test]
fn uniform_ratio_is_preserved_by_every_pool() {
    let set = TracerSet {
        tracers: vec![descriptor("HDO", TracerFamily::Isotope)],
    };
    // 灌溉水在地表蒸发时已算进分母、示踪物却晚到（上游如此），所以这里不开灌溉。
    let mut scen = Scenario::bare_soil();
    let irrig = scen.irrig;
    scen.irrig = 0.0;
    scen.wdsrf_bef += irrig * DT;
    let ratios = [R_ISO];
    let mut state = scen.state(&set, &ratios);
    tracer_soil_water(
        &set,
        &mut state,
        TracerPhysics::default(),
        &no_diffusion(),
        &scen.input(&ratios),
    )
    .unwrap();
    let p = &state.pools[0];
    let close = |trc: f64, water: f64, what: &str| {
        assert!(
            (trc - R_ISO * water).abs() <= 1.0e-12 * R_ISO * water.abs().max(1.0),
            "{what}: ratio {} != {R_ISO}",
            trc / water
        );
    };
    for j in 1..=NL {
        close(
            p.wliq_soisno[soisno_slot(j)],
            scen.wliq[soisno_slot(j)],
            "wliq",
        );
        close(
            p.wice_soisno[soisno_slot(j)],
            scen.wice[soisno_slot(j)],
            "wice",
        );
    }
    close(p.wa, scen.wa, "wa");
    close(p.wdsrf, scen.wdsrf, "wdsrf");
    close(
        p.waterstorage,
        scen.waterstorage - scen.irrig * DT,
        "waterstorage",
    );
    // 输出通量也带同一比值。
    let acc = &state.acc[0];
    close(acc.evap, acc.water_evap_gross, "evap");
    close(acc.rnof, (scen.rsur + scen.rsub) * DT, "runoff");
    assert!(state.step[0].numerical_residual_step.abs() <= 1.0e-12);
}

#[test]
fn total_tracer_closes_against_booked_fluxes() {
    let set = TracerSet {
        tracers: vec![
            descriptor("HDO", TracerFamily::Isotope),
            descriptor("Cl", TracerFamily::Solute),
        ],
    };
    // 比值故意不一致，让各条路径都搬出非比例的质量。
    let mut scen = Scenario::bare_soil();
    scen.wliq[soisno_slot(5)] += 0.3;
    scen.wliq[soisno_slot(2)] -= 0.2;
    let ratios = [R_ISO * 1.01, 0.4];
    let mut before = scen.state(&set, &[R_ISO, 2.0]);
    before.pools[1].wliq_soisno[soisno_slot(1)] *= 3.0;
    before.pools[1].wa *= 0.5;
    let mut after = before.clone();
    tracer_soil_water(
        &set,
        &mut after,
        TracerPhysics::default(),
        &no_diffusion(),
        &scen.input(&ratios),
    )
    .unwrap();
    for itrc in 0..set.len() {
        assert_closed(&before, &after, itrc, false);
    }
}

#[test]
fn snowpack_and_solute_vapor_diffusion_close_the_budget() {
    let set = TracerSet {
        tracers: vec![
            descriptor("HDO", TracerFamily::Isotope),
            descriptor("Cl", TracerFamily::Solute),
        ],
    };
    let s = soisno_slot;
    let mut scen = Scenario::bare_soil();
    scen.snl = -2;
    scen.wliq_bef[s(-1)] = 3.0;
    scen.wliq_bef[s(0)] = 4.0;
    scen.wice_bef[s(-1)] = 30.0;
    scen.wice_bef[s(0)] = 40.0;
    scen.wliq[s(-1)] = 2.5;
    scen.wliq[s(0)] = 3.5;
    scen.wice[s(-1)] = 30.2;
    scen.wice[s(0)] = 39.0;
    scen.snow_qout[s(-1)] = 1.2;
    scen.snow_qout[s(0)] = 1.6;
    let ratios = [R_ISO, 0.0];
    let mut before = scen.state(&set, &[R_ISO * 0.98, 1.5]);
    before.pools[1].wice_soisno[s(-1)] *= 4.0;
    let mut after = before.clone();
    // 溶质走气相扩散；同位素会被拒绝，所以只放溶质。
    let solute_only = TracerSet {
        tracers: vec![set.tracers[1].clone()],
    };
    let mut solute_before = PatchTracerState::allocated(&solute_only);
    solute_before.pools[0] = before.pools[1].clone();
    solute_before.step[0] = before.step[1].clone();
    let mut solute_after = solute_before.clone();
    let dz_soi = [0.1; SOIL_LAYERS];
    let porsl = [0.45; SOIL_LAYERS];
    let dz_sno = [0.2; MAX_SNOW_LAYERS];
    let mut temps = [270.0; SOISNO_LAYERS];
    temps[s(-1)] = 260.0;
    let mut input = scen.input(&ratios);
    tracer_soil_water(
        &set,
        &mut after,
        TracerPhysics::default(),
        &no_diffusion(),
        &input,
    )
    .unwrap();
    for itrc in 0..set.len() {
        assert_closed(&before, &after, itrc, false);
    }

    input.dz_soi = Some(&dz_soi);
    input.porsl = Some(&porsl);
    input.dz_sno = Some(&dz_sno);
    input.t_soisno = Some(&temps);
    input.forc_psrf = Some(9.0e4);
    let solute_ratios = [0.0];
    input.precip_ratio = &solute_ratios;
    input.vapor_ratio = &solute_ratios;
    tracer_soil_water(
        &solute_only,
        &mut solute_after,
        TracerPhysics::default(),
        &SoilWaterOptions::default(),
        &input,
    )
    .unwrap();
    assert_closed(&solute_before, &solute_after, 0, false);
    // 气相扩散确实搬了东西（与不扩散的那次相比）。
    let (diffused, plain) = (&solute_after.pools[0], &after.pools[1]);
    assert_ne!(diffused.wice_soisno[s(-1)], plain.wice_soisno[s(-1)]);
    assert_ne!(diffused.wliq_soisno[s(2)], plain.wliq_soisno[s(2)]);
    // 同位素在扩散打开时被拒绝，且状态不变。
    let mut rejected = before.clone();
    assert!(tracer_soil_water(
        &set,
        &mut rejected,
        TracerPhysics::default(),
        &SoilWaterOptions::default(),
        &input,
    )
    .is_err());
    assert_eq!(rejected, before);
}

#[test]
fn nonvolatile_solute_stays_behind_when_water_evaporates() {
    let set = TracerSet {
        tracers: vec![descriptor("Cl", TracerFamily::Solute)],
    };
    let s = soisno_slot;
    let mut scen = Scenario::bare_soil();
    scen.qlayer = [0.0; SOIL_LAYERS + 1];
    scen.etroot_actual = [0.0; SOIL_LAYERS];
    scen.etroot_aquifer = 0.0;
    scen.rsub = 0.0;
    scen.irrig = 0.0;
    scen.pg_rain = 0.0;
    scen.rsur = 0.0;
    scen.qsdew = 0.0;
    scen.qfros = 0.0;
    scen.qsubl = 1.5e-3; // 超过第 1 层的冰：整相升华光
    scen.wdsrf_bef = 0.0;
    scen.wdsrf = 0.0;
    // 蒸发亏缺直接从第 1 层抽（负入渗，qgtop < 0）。
    scen.qinfl = -1.0e-4;
    scen.wa = scen.wa_bef;
    scen.wice = scen.wice_bef;
    scen.wice[s(1)] = 0.0;
    scen.wliq = scen.shadow(0.0);
    scen.wliq[s(1)] += scen.qinfl * DT;
    let ratios = [0.7];
    let before = scen.state(&set, &[2.0]);
    let mut after = before.clone();
    let mut input = scen.input(&ratios);
    input.qgtop_solver = Some(-1.0e-4);
    tracer_soil_water(
        &set,
        &mut after,
        TracerPhysics::default(),
        &no_diffusion(),
        &input,
    )
    .unwrap();
    let (p0, p1) = (&before.pools[0], &after.pools[0]);
    assert_eq!(
        after.acc[0].evap, 0.0,
        "a nonvolatile solute was evaporated"
    );
    assert!(after.acc[0].water_evap_gross > 0.0);
    // 第 1 层的溶质原地浓缩；冰相整体升华时溶质留在冰里。
    assert_eq!(p1.wliq_soisno[s(1)], p0.wliq_soisno[s(1)]);
    assert!(p1.wliq_soisno[s(1)] / scen.wliq[s(1)] > 2.0);
    assert_eq!(p1.wice_soisno[s(1)], p0.wice_soisno[s(1)]);
    assert_closed(&before, &after, 0, false);
}

#[test]
fn wetland_pool_closes_and_keeps_uniform_ratio() {
    let set = TracerSet {
        tracers: vec![
            descriptor("HDO", TracerFamily::Isotope),
            descriptor("Cl", TracerFamily::Solute),
        ],
    };
    let s = soisno_slot;
    let scen = Scenario::bare_soil();
    let mut t_soisno = [280.0; SOISNO_LAYERS];
    t_soisno[s(1)] = 285.0;
    let porsl = [0.4; SOIL_LAYERS];
    let mut dz = [0.0; SOISNO_LAYERS];
    for j in 1..=NL {
        dz[s(j)] = 0.05;
    }
    let (wdsrf_bef, wetwat_bef, wa_bef) = (5.0, 300.0, 0.0);
    let (qseva, qsdew, etr, rsur) = (1.0e-4, 2.0e-6, 3.0e-5, 1.0e-4);
    let mut wresi = 0.0;
    for j in 1..=NL {
        wresi += (scen.wliq_bef[s(j)] - porsl[soil_slot(j)] * dz[s(j)] * 1000.0).max(0.0);
    }
    let pool_water = wdsrf_bef
        + wa_bef
        + wetwat_bef
        + wresi
        + (scen.pg_rain + qsdew + scen.qfros - qseva - scen.qsubl - etr) * DT;
    let wdsrf = 10.0;
    let wa = 0.0;
    let wetwat = pool_water - wdsrf - wa - rsur * DT;
    let ratios = [R_ISO, 0.3];
    let input = WetlandInput {
        ipatch: 7,
        deltim: DT,
        snl: 0,
        rsur,
        qseva_in: qseva,
        qsdew_in: qsdew,
        qsubl_in: scen.qsubl,
        qfros_in: scen.qfros,
        qseva_soil: 0.0,
        qsdew_soil: 0.0,
        qsubl_soil: 0.0,
        qfros_soil: 0.0,
        qseva_snow: 0.0,
        qsdew_snow: 0.0,
        qsubl_snow: 0.0,
        qfros_snow: 0.0,
        etr,
        sm: 0.0,
        fsno: 0.0,
        split_soilsnow: false,
        wliq_soisno: &scen.wliq_bef,
        wice_soisno: &scen.wice_bef,
        wliq_soisno_bef: &scen.wliq_bef,
        wice_soisno_bef: &scen.wice_bef,
        wa,
        wa_bef,
        wdsrf,
        wdsrf_bef,
        wetwat,
        wetwat_bef,
        pg_rain: scen.pg_rain,
        pg_snow: 0.0,
        t_soisno: &t_soisno,
        porsl: &porsl,
        dz_soisno: &dz,
        qflx_irrig_ground: 0.0,
        forc_us: 1.0,
        forc_vs: 1.0,
        waterstorage_patch: Some(10.0),
        snow_qout_layer: None,
        forc_q: None,
        forc_psrf: None,
        tleaf: None,
        lai: None,
        rst: None,
        ra: None,
        dz_sno: None,
        vapor_ratio: &ratios,
        has_vapor: None,
    };
    let mut before = scen.state(&set, &[R_ISO, 0.3]);
    for (itrc, pools) in before.pools.iter_mut().enumerate() {
        pools.wa = ratios[itrc] * wa_bef;
        pools.wdsrf = ratios[itrc] * wdsrf_bef;
        pools.wetwat = ratios[itrc] * wetwat_bef;
    }
    let mut after = before.clone();
    tracer_wetland(
        &set,
        &mut after,
        TracerPhysics::default(),
        &no_diffusion(),
        &input,
    )
    .unwrap();
    for itrc in 0..set.len() {
        assert_closed(&before, &after, itrc, false);
    }
    let p = &after.pools[0];
    assert!((p.wetwat / wetwat - R_ISO).abs() <= 1.0e-12 * R_ISO);
    assert!((p.wdsrf / wdsrf - R_ISO).abs() <= 1.0e-12 * R_ISO);
    // 溶质不随湿地蒸发离开，池子变浓。
    assert_eq!(after.acc[1].evap, 0.0);
    assert!(after.pools[1].wetwat / wetwat > 0.3);
}

#[test]
fn fractionation_changes_only_the_registered_isotope() {
    let set = TracerSet {
        tracers: vec![
            descriptor("Cl", TracerFamily::Solute),
            descriptor("HDO", TracerFamily::Isotope),
        ],
    };
    let scen = Scenario::bare_soil();
    let ratios = [0.5, R_ISO];
    let before = scen.state(&set, &ratios);
    let mut after = before.clone();
    let physics = TracerPhysics {
        fractionation: true,
        ..Default::default()
    };
    let mut plain = before.clone();
    tracer_soil_water(
        &set,
        &mut after,
        physics,
        &no_diffusion(),
        &scen.input(&ratios),
    )
    .unwrap();
    tracer_soil_water(
        &set,
        &mut plain,
        TracerPhysics::default(),
        &no_diffusion(),
        &scen.input(&ratios),
    )
    .unwrap();
    // 溶质（Cl）不受分馏开关影响；HDO 走分馏路径。
    assert_eq!(after.pools[0], plain.pools[0]);
    assert_ne!(after.pools[1], plain.pools[1]);
}

#[test]
fn leaf_anomaly_is_returned_to_the_root_zone() {
    let tracer = descriptor("HDO", TracerFamily::Isotope);
    let wliq = [10.0, 0.0, 30.0];
    let mut pools = TracerPools::allocated(0.0);
    pools.wliq_soisno[soisno_slot(1)] = 10.0 * R_ISO;
    pools.wliq_soisno[soisno_slot(3)] = 30.0 * R_ISO;
    pools.wa = 60.0 * R_ISO;
    pools.leaf_iso_storage = 1.0e-3;
    pools.leaf_water_moles = 5.0;
    let total = pool_total(&pools);
    release_leaf_iso_storage(&tracer, &mut pools, 0.0, &wliq, 60.0);
    assert_eq!(pools.leaf_iso_storage, 0.0);
    assert_eq!(pools.leaf_water_moles, 0.0);
    assert_eq!(pools.leaf_peclet, 1.0);
    assert!((pool_total(&pools) - total).abs() <= 1.0e-15);
    // 正异常按水量比例分：第 1 层拿 10/100。
    assert!((pools.wliq_soisno[soisno_slot(1)] - (10.0 * R_ISO + 1.0e-4)).abs() <= 1.0e-15);

    // 负异常超过现存量时只扣到零，余下留着。
    pools.leaf_iso_storage = -1.0;
    let total = pool_total(&pools);
    release_leaf_iso_storage(&tracer, &mut pools, 0.0, &wliq, 60.0);
    assert_eq!(pools.wliq_soisno[soisno_slot(1)], 0.0);
    assert!(pools.wa.abs() <= 1.0e-18);
    assert!((pool_total(&pools) - total).abs() <= 1.0e-12);
    assert!(pools.leaf_iso_storage < 0.0);
}

#[test]
fn aquifer_validity_matches_the_reference_carrier_rules() {
    // 参考载体在：实际水量为正、实际质量非负。
    assert!(aquifer_isotope_state_valid(-50.0, -0.05, R_ISO, 100.0, 0.2));
    // 实际水量为负（亏欠超过参考载体）不合法。
    assert!(!aquifer_isotope_state_valid(
        -150.0, -0.1, R_ISO, 100.0, 0.2
    ));
    // 没有载体时只允许舍入量级的质量。
    assert!(aquifer_isotope_state_valid(0.0, 1.0e-20, R_ISO, 0.0, 0.0));
    assert!(!aquifer_isotope_state_valid(0.0, 1.0e-6, R_ISO, 0.0, 0.0));
    let tracer = descriptor("HDO", TracerFamily::Isotope);
    assert!(check_isotope_aquifer(&tracer, 0, 3, 0.0, 0.0, 0.0, 1.0, "test").is_err());
    let solute = descriptor("Cl", TracerFamily::Solute);
    assert!(check_isotope_aquifer(&solute, 0, 3, 0.0, 0.0, 0.0, 1.0, "test").is_ok());
}

#[test]
fn qcharge_follows_colmmain_formula() {
    let q = qcharge_trc(10.0, 9.0, 0.5, -3.0, 1800.0);
    assert_eq!(q, ((10.0_f64 - 9.0) + 0.5) / 1800.0);
    let q = qcharge_trc(10.0, 9.0, 0.5, 0.25, 0.0);
    assert_eq!(q, (((10.0_f64 - 9.0) + 0.5) + 0.25) / TRC_TINY);
}

/// upstream-bugs #78：饱和土柱向上渗出（`qinfl < 0`）远大于土壤蒸发时，只有 `qseva·dt`
/// 记为蒸发，其余作为渗出进地表池；不分馏时各池与通量都保持同一比值。
#[test]
fn exfiltration_beyond_soil_evaporation_is_not_booked_as_evaporation() {
    let set = TracerSet {
        tracers: vec![descriptor("HDO", TracerFamily::Isotope)],
    };
    let mut scen = Scenario::bare_soil();
    scen.pg_rain = 0.0;
    scen.irrig = 0.0;
    scen.rsur = 0.0;
    scen.qseva = 1.0e-5;
    scen.qinfl = -1.0e-3;
    // 地表池闭合：wdsrf_bef - qseva*dt = wdsrf + qinfl*dt。
    scen.wdsrf = scen.wdsrf_bef - (scen.qseva + scen.qinfl) * DT;
    scen.wliq = scen.shadow(4.0e-6);
    scen.wliq[soisno_slot(3)] -= 0.5;
    assert!(-scen.qinfl * DT > 10.0 * scen.qseva * DT);
    let ratios = [R_ISO];
    let mut state = scen.state(&set, &ratios);
    tracer_soil_water(
        &set,
        &mut state,
        TracerPhysics::default(),
        &no_diffusion(),
        &scen.input(&ratios),
    )
    .unwrap();
    let acc = &state.acc[0];
    assert!(
        (acc.water_soilevap - scen.qseva * DT).abs() <= 1.0e-12,
        "soil evaporation booked {} mm, qseva*dt = {} mm",
        acc.water_soilevap,
        scen.qseva * DT
    );
    let close = |trc: f64, water: f64, what: &str| {
        assert!(
            (trc - R_ISO * water).abs() <= 1.0e-12 * R_ISO * water.abs().max(1.0),
            "{what}: ratio {} != {R_ISO}",
            trc / water
        );
    };
    close(acc.soilevap, acc.water_soilevap, "soil evaporation");
    close(state.pools[0].wdsrf, scen.wdsrf, "wdsrf");
    close(
        state.pools[0].wliq_soisno[soisno_slot(1)],
        scen.wliq[soisno_slot(1)],
        "wliq1",
    );
}
