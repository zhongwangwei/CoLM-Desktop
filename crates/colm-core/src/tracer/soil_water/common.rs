//! `tracer_soil_water` 与 `tracer_wetland` 共用的部分：含水层参考水的记账、`check_isotope_aquifer`、
//! `exhaust_surface_phase`、`release_leaf_iso_storage`、内部函数 `atmospheric_loss_tracer`/
//! `deposition_ratio_for`（不分馏形态）、雪顶层外部通量与雪层渗流（两处镜像）、溶质的
//! 气相扩散（`DEF_TRACER_SOIL_VAPOR_DIFFUSION`）。

use anyhow::{bail, Result};

use super::super::evap_limit::atmospheric_tracer_loss;
use super::super::{
    soisno_slot, EvapKind, PatchTracerState, TracerDescriptor, TracerPhysics, TracerPools, MAX_SNOW_LAYERS,
    SOIL_LAYERS, SOISNO_LAYERS, TRC_TINY, TRC_WATER_MIN_FOR_RATIO,
};
use super::super::frac::{
    equilibration_exchange, snow_vapor_equivalent_diffusivity, soil_diffusive_transfer,
    soil_effective_diffusivity, soil_vapor_equivalent_diffusivity, surface_relhum,
};

/// 内部函数 `layer_temp` 在没有温度实参时的取值。
pub(super) const DEFAULT_LAYER_TEMP_K: f64 = 273.15;

/// 土壤层 `j`（`1..=nl_soil`）在 `[f64; SOIL_LAYERS]` 里的位置。
pub(super) const fn soil_slot(j: i32) -> usize {
    (j - 1) as usize
}

/// `tracer_aquifer_actual_water`：相对含水层水量 + 参考水量。
pub fn aquifer_actual_water(wa: f64, reference_water: f64) -> f64 {
    wa + reference_water
}

/// `tracer_aquifer_actual_mass`：相对同位素质量 + 参考质量。
pub fn aquifer_actual_mass(relative_mass: f64, reference_mass: f64) -> f64 {
    relative_mass + reference_mass
}

/// `tracer_aquifer_isotope_ratio`：实际质量 / 实际水量；水量不足可解析下限时取 `fallback`。
pub fn aquifer_isotope_ratio(
    wa: f64,
    relative_mass: f64,
    reference_water: f64,
    reference_mass: f64,
    fallback: f64,
) -> f64 {
    let actual_water = aquifer_actual_water(wa, reference_water);
    if actual_water.abs() > TRC_WATER_MIN_FOR_RATIO {
        aquifer_actual_mass(relative_mass, reference_mass) / actual_water
    } else {
        fallback
    }
}

/// `tracer_aquifer_isotope_state_valid`（参考水量/质量两个可选实参都给出时的形态）。
pub fn aquifer_isotope_state_valid(
    water_mass: f64,
    isotope_mass: f64,
    ref_ratio: f64,
    reference_water: f64,
    reference_mass: f64,
) -> bool {
    if !water_mass.is_finite() || !isotope_mass.is_finite() {
        return false;
    }
    if !ref_ratio.is_finite() || ref_ratio <= 0.0 {
        return false;
    }
    if !reference_water.is_finite() || !reference_mass.is_finite() {
        return false;
    }
    if reference_water < 0.0 || reference_mass < 0.0 {
        return false;
    }
    if reference_water > f64::MAX - water_mass.abs() {
        return false;
    }
    if reference_mass > f64::MAX - isotope_mass.abs() {
        return false;
    }
    let actual_water = aquifer_actual_water(water_mass, reference_water);
    let actual_mass = aquifer_actual_mass(isotope_mass, reference_mass);
    // `16*epsilon(1._r8)` 折成常数 2^-48。
    let dust_mass = (TRC_WATER_MIN_FOR_RATIO * ref_ratio)
        .max(16.0 * f64::EPSILON * isotope_mass.abs().max(reference_mass));
    if actual_water.abs() <= TRC_WATER_MIN_FOR_RATIO {
        return actual_mass.abs() <= dust_mass;
    }
    if reference_water > 0.0 && actual_water < 0.0 {
        return false;
    }
    if actual_water * 1.0_f64.copysign(actual_mass) < 0.0 && actual_mass != 0.0 {
        return false;
    }
    if actual_water.abs() < 1.0 && actual_mass.abs() > f64::MAX * actual_water.abs() {
        return false;
    }
    let ratio = actual_mass / actual_water;
    ratio.is_finite() && ratio >= 0.0
}

/// `check_isotope_aquifer`：同位素含水层的亏欠必须有物理的水载体，否则 `CoLM_stop`。
///
/// 上游读的是模块数组 `trc_aquifer_ref_water(ipatch)` 与 `trc_aquifer_ref_mass(itrc, ipatch)`，
/// 本模块各过程里都不改它们，所以就是 [`PatchTracerState::aquifer_ref_water`] 与
/// [`TracerPools::aquifer_ref_mass`]。
#[allow(clippy::too_many_arguments)]
pub fn check_isotope_aquifer(
    tracer: &TracerDescriptor,
    itrc: usize,
    ipatch: usize,
    patch_aquifer_ref_water: f64,
    aquifer_ref_mass: f64,
    water_mass: f64,
    isotope_mass: f64,
    stage: &str,
) -> Result<()> {
    if !tracer.is_isotope() {
        return Ok(());
    }
    if aquifer_isotope_state_valid(
        water_mass,
        isotope_mass,
        tracer.ref_ratio,
        patch_aquifer_ref_water,
        aquifer_ref_mass,
    ) {
        return Ok(());
    }
    bail!(
        "Unresolved isotope aquifer debt at {} tracer={} patch={} water={:E} isotope={:E}: \
         isotope aquifer debt has no physical water carrier",
        stage.trim_end(),
        itrc + 1,
        ipatch,
        water_mass,
        isotope_mass
    )
}

/// `exhaust_surface_phase`：把一个相的示踪物整体作为蒸发/升华送走；非挥发溶质留在原层。
pub fn exhaust_surface_phase(
    tracer: &TracerDescriptor,
    state: &mut PatchTracerState,
    itrc: usize,
    phase_tracer: &mut f64,
    water_loss: f64,
    kind: EvapKind,
) {
    let tracer_loss = phase_tracer.max(0.0);
    if tracer_loss <= TRC_TINY {
        return;
    }
    if tracer.is_nonvolatile_solute() {
        return;
    }
    *phase_tracer = 0.0;
    state.book_evap_loss(itrc, tracer_loss, water_loss, kind);
}

/// `release_leaf_iso_storage`：叶片 NSS 只存同位素异常（不存水）；落叶时把它还给根区液态水。
///
/// `wliq_soil` 是 `wliq_soil(1:nl_soil)`（长度 `nl_soil`），`wa_liq` 是相对含水层水量。
/// 正异常按水量比例混入各层与含水层；负异常按示踪物量同比例扣，扣不完的留到下次。
///
/// GIMPLE：两个求和都是从 0 顺序累加；`trc_wa - aquifer_mass*release_fraction` 是
/// `.FNMA`，`anomaly + release_fraction*pool_total` 是 `.FMA`；`anomaly*w/pool_total`
/// 先乘后除、不融合。
pub fn release_leaf_iso_storage(
    tracer: &TracerDescriptor,
    pools: &mut TracerPools,
    patch_aquifer_ref_water: f64,
    wliq_soil: &[f64],
    wa_liq: f64,
) {
    let (aquifer_ref_water, aquifer_ref_mass) = if tracer.is_isotope() {
        (patch_aquifer_ref_water, pools.aquifer_ref_mass)
    } else {
        (0.0, 0.0)
    };
    let aquifer_water = aquifer_actual_water(wa_liq, aquifer_ref_water);
    let aquifer_mass = aquifer_actual_mass(pools.wa, aquifer_ref_mass);
    let anomaly = pools.leaf_iso_storage;

    pools.leaf_delta_e = 0.0;
    pools.leaf_delta_b = 0.0;
    pools.leaf_peclet = 1.0;
    pools.leaf_water_moles = 0.0;
    if anomaly.abs() <= TRC_TINY {
        pools.leaf_iso_storage = 0.0;
        return;
    }

    if anomaly > 0.0 {
        let mut pool_total = 0.0;
        for &w in wliq_soil {
            pool_total = w.max(0.0) + pool_total;
        }
        pool_total += aquifer_water.max(0.0);
        if pool_total <= TRC_TINY {
            return;
        }
        for (j, &w) in (1..).zip(wliq_soil) {
            if w > 0.0 {
                let slot = soisno_slot(j);
                pools.wliq_soisno[slot] += w * anomaly / pool_total;
            }
        }
        if aquifer_water > 0.0 {
            pools.wa += aquifer_water * anomaly / pool_total;
        }
        pools.leaf_iso_storage = 0.0;
    } else {
        let mut pool_total = 0.0;
        for j in 1..=wliq_soil.len() as i32 {
            pool_total = pools.wliq_soisno[soisno_slot(j)].max(0.0) + pool_total;
        }
        if aquifer_water > 0.0 {
            pool_total += aquifer_mass.max(0.0);
        }
        if pool_total <= TRC_TINY {
            return;
        }
        let release_fraction = (-(anomaly / pool_total)).min(1.0);
        for j in 1..=wliq_soil.len() as i32 {
            let slot = soisno_slot(j);
            if pools.wliq_soisno[slot] > 0.0 {
                pools.wliq_soisno[slot] *= 1.0 - release_fraction;
            }
        }
        if aquifer_water > 0.0 && aquifer_mass > 0.0 {
            pools.wa = (-aquifer_mass).mul_add(release_fraction, pools.wa);
        }
        pools.leaf_iso_storage = release_fraction.mul_add(pool_total, anomaly);
        if pools.leaf_iso_storage.abs() <= TRC_TINY {
            pools.leaf_iso_storage = 0.0;
        }
    }
}

/// 内部函数 `evap_ratio_for` 的动力分馏形态：土壤支与湿地支不同。
#[derive(Clone, Copy)]
pub(super) enum EvapKinetics {
    /// `tracer_soil_water`：裸土液面蒸发（`kinetic_on_soil_surface`）且
    /// `DEF_TRACER_SOIL_KINETIC = 'RESISTANCE'` 且 `ra`/`rss` 都给了时按阻力加权，
    /// 否则 `tracer_alpha_kinetic_craig_gordon`。
    Soil {
        resistance: bool,
        ra: Option<f64>,
        rss: Option<f64>,
    },
    /// `tracer_wetland`：冰面 `craig_gordon(.true.)`，液面开阔水面 `open_water(|u|)`，
    /// `|u| = sqrt(FMA(us, us, vs*vs))`（`max(·,0)` 被优化掉）。
    OpenWater { wind: f64 },
}

/// 一个示踪物在本步里的常量（上游内部函数经 `CHAIN` 读的那些）。
pub(super) struct TracerCtx<'a> {
    pub tracer: &'a TracerDescriptor,
    pub itrc: usize,
    pub nonvolatile: bool,
    /// `DEF_TRACER_SUBL_SKIN_MM`（上游总以 `skin_mass=` 传入）。
    pub subl_skin_mm: f64,
    pub physics: TracerPhysics,
    /// `tracer_fractionation_active(itrc)`。
    pub active: bool,
    /// `R_atm = tracer_forcing_vapor_value(itrc, ipatch)`。
    pub r_atm: f64,
    pub forc_q: Option<f64>,
    pub forc_psrf: Option<f64>,
    pub kinetics: EvapKinetics,
}

impl<'a> TracerCtx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tracer: &'a TracerDescriptor,
        itrc: usize,
        physics: TracerPhysics,
        vapor_ratio: f64,
        subl_skin_mm: f64,
        forc_q: Option<f64>,
        forc_psrf: Option<f64>,
        kinetics: EvapKinetics,
    ) -> Self {
        Self {
            tracer,
            itrc,
            nonvolatile: tracer.is_nonvolatile_solute(),
            subl_skin_mm,
            physics,
            active: physics.fractionation_active(tracer),
            r_atm: vapor_ratio,
            forc_q,
            forc_psrf,
            kinetics,
        }
    }

    /// 内部函数 `deposition_ratio_for`：非挥发溶质 0；不分馏时 `R_atm`；否则平衡（冰面
    /// Jouzel-Merlivat 有效）分馏后的凝结比值。
    pub fn deposition_ratio_for(&self, temp_k: f64, from_ice: bool) -> f64 {
        if self.nonvolatile {
            return 0.0;
        }
        if !self.active {
            return self.r_atm;
        }
        self.physics
            .equilibrium_deposition_ratio(self.tracer, self.r_atm, temp_k, from_ice)
    }

    /// 内部函数 `evap_ratio_for`（Craig-Gordon）；`soil_surface` 即 `kinetic_on_soil_surface`。
    pub fn evap_ratio_for(&self, source_ratio: f64, temp_k: f64, from_ice: bool, soil_surface: bool) -> f64 {
        if !self.active {
            return source_ratio;
        }
        let (Some(forc_q), Some(forc_psrf)) = (self.forc_q, self.forc_psrf) else {
            return source_ratio;
        };
        let relhum = surface_relhum(forc_q, forc_psrf, temp_k, from_ice);
        let alpha_k = match self.kinetics {
            EvapKinetics::Soil { resistance, ra: Some(ra), rss: Some(rss) }
                if soil_surface && !from_ice && resistance =>
            {
                self.physics.alpha_kinetic_soil(self.tracer, ra, rss)
            }
            EvapKinetics::Soil { .. } => self.physics.alpha_kinetic_craig_gordon(self.tracer, from_ice),
            EvapKinetics::OpenWater { .. } if from_ice => {
                self.physics.alpha_kinetic_craig_gordon(self.tracer, true)
            }
            EvapKinetics::OpenWater { wind } => self.physics.alpha_kinetic_open_water(self.tracer, wind),
        };
        self.physics.craig_gordon_evap_ratio(
            self.tracer,
            source_ratio,
            self.r_atm,
            temp_k,
            relhum,
            alpha_k,
            from_ice,
        )
    }

    fn loss(&self, pool_trc: f64, pool_water: f64, water_loss: f64, temp_k: f64, from_ice: bool, soil_surface: bool) -> f64 {
        // `merge(ref_ratio*(1+trc_delta_sanity_max/1000), 0, active)`：常数折叠成 `ref*3`。
        let r_max = if self.active {
            self.tracer.ref_ratio * (1.0 + TRC_DELTA_SANITY_MAX / 1000.0)
        } else {
            0.0
        };
        atmospheric_tracer_loss(
            pool_trc,
            pool_water,
            water_loss,
            temp_k,
            from_ice,
            &|source_ratio, t, ice| self.evap_ratio_for(source_ratio, t, ice, soil_surface),
            TRC_TINY,
            r_max,
            self.nonvolatile,
            Some(self.subl_skin_mm),
        )
    }

    /// 内部函数 `atmospheric_loss_tracer`（`kinetic_on_soil_surface = .false.`）。
    pub fn atmospheric_loss(&self, pool_trc: f64, pool_water: f64, water_loss: f64, temp_k: f64, from_ice: bool) -> f64 {
        self.loss(pool_trc, pool_water, water_loss, temp_k, from_ice, false)
    }

    /// 同上，裸土液面蒸发的两处调用（上游临时置 `kinetic_on_soil_surface = .true.`）。
    pub fn atmospheric_loss_soil_surface(&self, pool_trc: f64, pool_water: f64, water_loss: f64, temp_k: f64) -> f64 {
        self.loss(pool_trc, pool_water, water_loss, temp_k, false, true)
    }
}

/// `trc_delta_sanity_max`（`MOD_Tracer_Defs.F90:121`）。
pub(super) const TRC_DELTA_SANITY_MAX: f64 = 2.0e3;

/// 雪顶层与渗流需要的宿主量（两个过程的同名实参）。
pub(super) struct SnowColumn<'a> {
    pub snl: i32,
    pub deltim: f64,
    pub split_soilsnow: bool,
    pub fsno: f64,
    pub pg_rain: f64,
    pub qseva_in: f64,
    pub qsdew_in: f64,
    pub qsubl_in: f64,
    pub qfros_in: f64,
    pub qseva_snow: f64,
    pub qsdew_snow: f64,
    pub qsubl_snow: f64,
    pub qfros_snow: f64,
    pub wliq_soisno: &'a [f64; SOISNO_LAYERS],
    /// WATER 之后的冰（融水与层冰的同位素交换用）。
    pub wice_soisno: &'a [f64; SOISNO_LAYERS],
    pub wliq_soisno_bef: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno_bef: &'a [f64; SOISNO_LAYERS],
    pub snow_qout_layer: Option<&'a [f64; MAX_SNOW_LAYERS]>,
    pub layer_temp: &'a dyn Fn(i32) -> f64,
    /// `DEF_TRACER_SNOWMELT_EQUILIBRATION`。
    pub snowmelt_equilibration: f64,
}

/// 两处镜像在 GIMPLE 上的唯一差别：霜的水量。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FrostShape {
    /// `tracer_soil_water`：`max(q,0)*dt` 先舍入一次（`_817`），`a_water_precip` 与
    /// `water_ice_pool` 都直接加它。
    Shared,
    /// `tracer_wetland`：两处各自是 `.FMA (dt, max(q,0), ·)`。
    Fused,
}

/// 雪顶层外部通量与渗流的结果。
pub(super) struct SnowOutcome {
    /// `d_wice_ext_snow`（只有 `tracer_soil_water` 用）。
    pub d_wice_ext_snow: f64,
    /// `trc_gwat_snow`：雪底流出的示踪物。
    pub trc_gwat_snow: f64,
    /// `gwat_snow_local`：雪底流出的水 [mm]。
    pub gwat_snow: f64,
}

/// 第 0b 节（雪顶层的霜/升华/雨/露/蒸发）与第 0c 节（逐层渗流）；只在 `snl < 0` 时调用。
///
/// GIMPLE 形状（两处相同，除霜量见 [`FrostShape`]）：
/// * 霜的示踪物 `trc_wice = FMA(dt, max(q,0)*R, trc_wice)`，`a_trc_precip` 同形；
/// * 露的示踪物 `dt*(max(q,0)*R)` 先舍入（同时加到 `trc_wliq` 与 `a_trc_precip`），
///   `a_water_precip = FMA(dt, max(q,0), a)`；
/// * 液池 `water_liq_pool = FMA(dt, FMA(fsno, pg_rain, max(qsdew,0)), wliq_bef)`（分开时），
///   否则 `FMA(dt, max(qsdew,0)+pg_rain, wliq_bef)`；
/// * 渗流 `trc_qout = min(qout*(trc/water), trc)`，不融合。
pub(super) fn snow_column(
    ctx: &TracerCtx<'_>,
    state: &mut PatchTracerState,
    p: &mut TracerPools,
    col: &SnowColumn<'_>,
    frost_shape: FrostShape,
) -> SnowOutcome {
    let dt = col.deltim;
    let itrc = ctx.itrc;
    let lb_snow = col.snl + 1;
    let top = soisno_slot(lb_snow);
    let (eff_qseva, eff_qsdew, eff_qsubl, eff_qfros) = if col.split_soilsnow {
        (
            col.qseva_snow,
            col.qsdew_snow,
            col.qsubl_snow,
            col.qfros_snow,
        )
    } else {
        (col.qseva_in, col.qsdew_in, col.qsubl_in, col.qfros_in)
    };
    let temp_top = (col.layer_temp)(lb_snow);

    // Step 1：冰相（霜进、升华出），升华超过冰量时余下的从液相扣。
    let frost_rate = eff_qfros.max(0.0);
    let frost_tracer_rate = frost_rate * ctx.deposition_ratio_for(temp_top, true);
    p.wice_soisno[top] = dt.mul_add(frost_tracer_rate, p.wice_soisno[top]);
    let acc = &mut state.acc[itrc];
    acc.precip = dt.mul_add(frost_tracer_rate, acc.precip);
    let frost_water = dt * frost_rate;
    let mut water_ice_pool = match frost_shape {
        FrostShape::Shared => {
            acc.water_precip += frost_water;
            frost_water + col.wice_soisno_bef[top]
        }
        FrostShape::Fused => {
            acc.water_precip = dt.mul_add(frost_rate, acc.water_precip);
            dt.mul_add(frost_rate, col.wice_soisno_bef[top])
        }
    };
    let water_ice_pool_prefrost = water_ice_pool;
    let subl_water = eff_qsubl.max(0.0) * dt;
    if subl_water > TRC_TINY {
        if water_ice_pool - subl_water > TRC_WATER_MIN_FOR_RATIO {
            let flux = ctx.atmospheric_loss(
                p.wice_soisno[top],
                water_ice_pool,
                subl_water,
                temp_top,
                true,
            );
            p.wice_soisno[top] -= flux;
            state.book_evap_loss(itrc, flux, subl_water, EvapKind::Sublimation);
        } else {
            exhaust_surface_phase(
                ctx.tracer,
                state,
                itrc,
                &mut p.wice_soisno[top],
                subl_water.min(water_ice_pool.max(0.0)),
                EvapKind::Sublimation,
            );
            let deficit_water = subl_water - water_ice_pool.max(0.0);
            if deficit_water > TRC_TINY && col.wliq_soisno_bef[top] > TRC_TINY {
                let flux = ctx.atmospheric_loss(
                    p.wliq_soisno[top],
                    col.wliq_soisno_bef[top],
                    deficit_water,
                    temp_top,
                    false,
                );
                p.wliq_soisno[top] -= flux;
                state.book_evap_loss(itrc, flux, deficit_water, EvapKind::Sublimation);
            }
        }
    }
    water_ice_pool = (water_ice_pool - subl_water).max(0.0);
    let mut d_wice_ext_snow = frost_water - subl_water.min(water_ice_pool_prefrost);

    // Step 2：液相（雨与露进、蒸发出），蒸发超过液量时余下的从冰相扣。
    let pg_rain_ground = state.step[itrc].pg_rain_ground;
    let rain_tracer = if col.split_soilsnow {
        col.fsno * pg_rain_ground
    } else {
        pg_rain_ground
    };
    p.wliq_soisno[top] += rain_tracer;
    let dew_rate = eff_qsdew.max(0.0);
    let dew_tracer = dt * (dew_rate * ctx.deposition_ratio_for(temp_top, false));
    p.wliq_soisno[top] = dew_tracer + p.wliq_soisno[top];
    let acc = &mut state.acc[itrc];
    acc.precip = dew_tracer + acc.precip;
    acc.water_precip = dt.mul_add(dew_rate, acc.water_precip);
    if eff_qseva > TRC_TINY {
        let mut water_liq_pool = if col.split_soilsnow {
            dt.mul_add(
                col.fsno.mul_add(col.pg_rain, dew_rate),
                col.wliq_soisno_bef[top],
            )
        } else {
            dt.mul_add(dew_rate + col.pg_rain, col.wliq_soisno_bef[top])
        };
        if water_ice_pool_prefrost < subl_water {
            water_liq_pool -= subl_water - water_ice_pool_prefrost;
        }
        water_liq_pool = water_liq_pool.max(0.0);
        let evap_water = dt * eff_qseva;
        if water_liq_pool - evap_water > TRC_WATER_MIN_FOR_RATIO {
            let flux = ctx.atmospheric_loss(
                p.wliq_soisno[top],
                water_liq_pool,
                evap_water,
                temp_top,
                false,
            );
            p.wliq_soisno[top] -= flux;
            state.book_evap_loss(itrc, flux, evap_water, EvapKind::SoilEvaporation);
        } else {
            exhaust_surface_phase(
                ctx.tracer,
                state,
                itrc,
                &mut p.wliq_soisno[top],
                evap_water.min(water_liq_pool.max(0.0)),
                EvapKind::SoilEvaporation,
            );
            let mut deficit_water = evap_water - water_liq_pool.max(0.0);
            if deficit_water > TRC_TINY && water_ice_pool > TRC_TINY {
                deficit_water = deficit_water.min(water_ice_pool);
                d_wice_ext_snow -= deficit_water;
                let flux = ctx.atmospheric_loss(
                    p.wice_soisno[top],
                    water_ice_pool,
                    deficit_water,
                    temp_top,
                    true,
                );
                p.wice_soisno[top] -= flux;
                state.book_evap_loss(itrc, flux, deficit_water, EvapKind::SoilEvaporation);
            }
        }
    }

    // 0c：自顶向下重演 snowwater 的 qin/qout。
    let (rain_in, dew_in, evap_out, frost_in, subl_out) = if col.split_soilsnow {
        (
            (col.pg_rain * col.fsno).max(0.0) * dt,
            col.qsdew_snow.max(0.0) * dt,
            col.qseva_snow.max(0.0) * dt,
            col.qfros_snow.max(0.0) * dt,
            col.qsubl_snow.max(0.0) * dt,
        )
    } else {
        (
            col.pg_rain.max(0.0) * dt,
            col.qsdew_in.max(0.0) * dt,
            col.qseva_in.max(0.0) * dt,
            col.qfros_in.max(0.0) * dt,
            col.qsubl_in.max(0.0) * dt,
        )
    };
    let mut snow_wliq_before_flow = col.wliq_soisno_bef[top];
    let ice_after = (col.wice_soisno_bef[top] + frost_in) - subl_out;
    if ice_after < 0.0 {
        snow_wliq_before_flow += ice_after;
    }
    snow_wliq_before_flow = ((snow_wliq_before_flow + rain_in) + dew_in) - evap_out;
    // 上游：soil 支写 `IF (x < 0) x = 0`，wetland 支写 `max(x, 0)`；随后 `max(x,0)` 被优化掉。
    if snow_wliq_before_flow < 0.0 {
        snow_wliq_before_flow = 0.0;
    }

    let mut qin_snow = 0.0;
    let mut trc_qin_snow = 0.0;
    for j in lb_snow..=0 {
        let slot = soisno_slot(j);
        let water_before_flow = if j == lb_snow {
            snow_wliq_before_flow
        } else {
            col.wliq_soisno_bef[slot].max(0.0) + qin_snow
        };
        let mut trc_before_flow = p.wliq_soisno[slot].max(0.0) + trc_qin_snow;
        let mut qout_snow = match col.snow_qout_layer {
            Some(qout) => qout[slot].max(0.0),
            None => (water_before_flow - col.wliq_soisno[slot].max(0.0)).max(0.0),
        };
        qout_snow = qout_snow.min(water_before_flow);
        // 渗流的融水与所经层冰的同位素交换，向 `R_liq = R_ice/α_ice_liq` 弛豫（`:879-896`）。
        if col.snowmelt_equilibration > 0.0
            && ctx.active
            && !ctx.nonvolatile
            && water_before_flow > TRC_WATER_MIN_FOR_RATIO
            && col.wice_soisno[slot] > TRC_WATER_MIN_FOR_RATIO
        {
            let alpha = 1.0 / ctx.physics.alpha_ice_liq(ctx.tracer, (col.layer_temp)(j)).max(TRC_TINY);
            let mut melt_exchange = equilibration_exchange(
                trc_before_flow,
                water_before_flow,
                p.wice_soisno[slot].max(0.0) / col.wice_soisno[slot],
                alpha,
                col.snowmelt_equilibration,
            );
            melt_exchange = if melt_exchange > 0.0 {
                melt_exchange.min(p.wice_soisno[slot].max(0.0))
            } else {
                -(-melt_exchange).min(trc_before_flow.max(0.0))
            };
            trc_before_flow = (trc_before_flow + melt_exchange).max(0.0);
            p.wice_soisno[slot] = (p.wice_soisno[slot] - melt_exchange).max(0.0);
        }
        let trc_qout_snow = if qout_snow > TRC_TINY
            && water_before_flow > TRC_WATER_MIN_FOR_RATIO
            && trc_before_flow > TRC_TINY
        {
            let ratio_src = trc_before_flow / water_before_flow;
            (qout_snow * ratio_src).min(trc_before_flow)
        } else {
            0.0
        };
        p.wliq_soisno[slot] = (trc_before_flow - trc_qout_snow).max(0.0);
        qin_snow = qout_snow;
        trc_qin_snow = trc_qout_snow;
    }

    SnowOutcome {
        d_wice_ext_snow,
        trc_gwat_snow: trc_qin_snow,
        gwat_snow: qin_snow,
    }
}

/// 雪层之间的冰相气相扩散（`DEF_TRACER_SOIL_VAPOR_DIFFUSION`，`:1424-1470`）。未登记分馏物理的
/// 示踪物 `tracer_diffusivity_ratio_air = tracer_alpha_ice_vap = 1`。
#[allow(clippy::too_many_arguments)]
pub(super) fn snow_vapor_diffusion(
    ctx: &TracerCtx<'_>,
    p: &mut TracerPools,
    snl: i32,
    wliq: &[f64; SOISNO_LAYERS],
    wice: &[f64; SOISNO_LAYERS],
    dz_sno: &[f64; MAX_SNOW_LAYERS],
    layer_temp: &dyn Fn(i32) -> f64,
    forc_psrf: f64,
    deltim: f64,
) {
    let (physics, tracer) = (ctx.physics, ctx.tracer);
    let lb = snl + 1;
    let mut diff_face = [0.0; SOISNO_LAYERS];
    let mut diff_out = [0.0; SOISNO_LAYERS];
    let mut diff_scale = [1.0; SOISNO_LAYERS];
    for j in lb..=-1 {
        let (up, dn) = (soisno_slot(j), soisno_slot(j + 1));
        if dz_sno[up] <= 0.0 || dz_sno[dn] <= 0.0 {
            continue;
        }
        if wice[up] <= TRC_WATER_MIN_FOR_RATIO || wice[dn] <= TRC_WATER_MIN_FOR_RATIO {
            continue;
        }
        let d_eff_up = snow_vapor_equivalent_diffusivity(
            wliq[up].max(0.0),
            wice[up].max(0.0),
            dz_sno[up],
            layer_temp(j),
            forc_psrf,
            physics.diffusivity_ratio_air(tracer),
            physics.alpha_ice_vap(tracer, layer_temp(j)),
        );
        let d_eff_dn = snow_vapor_equivalent_diffusivity(
            wliq[dn].max(0.0),
            wice[dn].max(0.0),
            dz_sno[dn],
            layer_temp(j + 1),
            forc_psrf,
            physics.diffusivity_ratio_air(tracer),
            physics.alpha_ice_vap(tracer, layer_temp(j + 1)),
        );
        if d_eff_up <= 0.0 || d_eff_dn <= 0.0 {
            continue;
        }
        let ratio_up = p.wice_soisno[up] / wice[up];
        let ratio_dn = p.wice_soisno[dn] / wice[dn];
        diff_face[up] = soil_diffusive_transfer(
            ratio_up, ratio_dn, wice[up], wice[dn], d_eff_up, d_eff_dn, dz_sno[up], dz_sno[dn],
            deltim,
        );
        if diff_face[up] > 0.0 {
            diff_out[up] += diff_face[up];
        } else {
            diff_out[dn] -= diff_face[up];
        }
    }
    for j in lb..=0 {
        let slot = soisno_slot(j);
        let available = p.wice_soisno[slot].max(0.0);
        if diff_out[slot] > available && diff_out[slot] > TRC_TINY {
            diff_scale[slot] = available / diff_out[slot];
        }
    }
    for j in lb..=-1 {
        let (up, dn) = (soisno_slot(j), soisno_slot(j + 1));
        let transfer = if diff_face[up] > 0.0 {
            diff_face[up] * diff_scale[up]
        } else {
            diff_face[up] * diff_scale[dn]
        };
        p.wice_soisno[up] -= transfer;
        p.wice_soisno[dn] += transfer;
    }
}

/// 土壤层之间的扩散（液膜 + 孔隙气相，`:1337-1420`）。液相自扩散系数对未登记分馏物理的
/// 示踪物是 0：只开 `DEF_TRACER_SOIL_DIFFUSION` 时第一个合格界面就 `EXIT`。
#[allow(clippy::too_many_arguments)]
pub(super) fn soil_diffusion(
    ctx: &TracerCtx<'_>,
    p: &mut TracerPools,
    water_shadow: &[f64; SOIL_LAYERS],
    wice: &[f64; SOISNO_LAYERS],
    dz_soi: &[f64; SOIL_LAYERS],
    porsl: &[f64; SOIL_LAYERS],
    layer_temp: &dyn Fn(i32) -> f64,
    liquid_diffusion: bool,
    vapor_diffusion: bool,
    forc_psrf: Option<f64>,
    deltim: f64,
) {
    let (physics, tracer) = (ctx.physics, ctx.tracer);
    let nl = SOIL_LAYERS as i32;
    let mut diff_face = [0.0; SOIL_LAYERS];
    let mut diff_out = [0.0; SOIL_LAYERS];
    let mut diff_scale = [1.0; SOIL_LAYERS];
    for j in 1..nl {
        let (up, dn) = (soil_slot(j), soil_slot(j + 1));
        if water_shadow[up] <= TRC_WATER_MIN_FOR_RATIO {
            continue;
        }
        if water_shadow[dn] <= TRC_WATER_MIN_FOR_RATIO {
            continue;
        }
        let diff_liquid = physics.leaf_liquid_diffusivity(tracer, layer_temp(j));
        if diff_liquid <= 0.0 && !vapor_diffusion {
            break;
        }
        let mut d_eff_up = 0.0;
        let mut d_eff_dn = 0.0;
        if liquid_diffusion && diff_liquid > 0.0 {
            d_eff_up = soil_effective_diffusivity(water_shadow[up], dz_soi[up], porsl[up], diff_liquid);
            d_eff_dn = soil_effective_diffusivity(
                water_shadow[dn],
                dz_soi[dn],
                porsl[dn],
                physics.leaf_liquid_diffusivity(tracer, layer_temp(j + 1)),
            );
        }
        if vapor_diffusion {
            if let Some(psrf) = forc_psrf {
                d_eff_up += soil_vapor_equivalent_diffusivity(
                    water_shadow[up],
                    wice[soisno_slot(j)].max(0.0),
                    dz_soi[up],
                    porsl[up],
                    layer_temp(j),
                    psrf,
                    physics.diffusivity_ratio_air(tracer),
                    physics.alpha_liq_vap(tracer, layer_temp(j)),
                );
                d_eff_dn += soil_vapor_equivalent_diffusivity(
                    water_shadow[dn],
                    wice[soisno_slot(j + 1)].max(0.0),
                    dz_soi[dn],
                    porsl[dn],
                    layer_temp(j + 1),
                    psrf,
                    physics.diffusivity_ratio_air(tracer),
                    physics.alpha_liq_vap(tracer, layer_temp(j + 1)),
                );
            }
        }
        if d_eff_up <= 0.0 || d_eff_dn <= 0.0 {
            continue;
        }
        let ratio_up = p.wliq_soisno[soisno_slot(j)] / water_shadow[up];
        let ratio_dn = p.wliq_soisno[soisno_slot(j + 1)] / water_shadow[dn];
        diff_face[up] = soil_diffusive_transfer(
            ratio_up,
            ratio_dn,
            water_shadow[up],
            water_shadow[dn],
            d_eff_up,
            d_eff_dn,
            dz_soi[up],
            dz_soi[dn],
            deltim,
        );
        if diff_face[up] > 0.0 {
            diff_out[up] += diff_face[up];
        } else {
            diff_out[dn] -= diff_face[up];
        }
    }
    for j in 1..=nl {
        let available = p.wliq_soisno[soisno_slot(j)].max(0.0);
        let k = soil_slot(j);
        if diff_out[k] > available && diff_out[k] > TRC_TINY {
            diff_scale[k] = available / diff_out[k];
        }
    }
    for j in 1..nl {
        let (up, dn) = (soil_slot(j), soil_slot(j + 1));
        let transfer = if diff_face[up] > 0.0 {
            diff_face[up] * diff_scale[up]
        } else {
            diff_face[up] * diff_scale[dn]
        };
        p.wliq_soisno[soisno_slot(j)] -= transfer;
        p.wliq_soisno[soisno_slot(j + 1)] += transfer;
    }
}
