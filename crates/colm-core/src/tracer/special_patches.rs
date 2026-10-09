//! 冰川与水体 patch 的示踪物（`MOD_Tracer_SpecialPatches`），以及两者共用的
//! `sync_tracer_patch_ratio`（`MOD_Tracer_Vars`）。
//!
//! 这两类 patch 不走完整的示踪物水文链：把转入时残留在不存在的载体（冠层、湿地、
//! 灌溉库、冰川的分层雪）里的示踪物先归并，然后在一个"混合箱"里记账——输入（降水、
//! 凝结、水体的湖亏补水）、蒸发/升华、径流，最后按箱内比值重建各存储。固定签名的
//! 同位素（不分馏、无运行时驱动）直接用 `R_init`。各 patch 自己调保存快照、衰减、守恒
//! 检查与 history 累加。
//!
//! 冰川溢出（CoLMMAIN 冰川分支 1592-1646 行）：顶层液/冰超过 `dz*den` 的部分作为
//! `wextra` 加进 `pg_rain`/`pg_snow` 并从 `totwb` 扣掉；示踪物侧把这部分水量记在
//! `glacier_overflow_mass_trc`，从水量收支 `water_dS` 里扣除（它已在箱内，不是外部输入）。
//! 由宿主计算后经 [`GlacierInput::glacier_overflow_mass`] 传入，[`glacier_overflow_mass`]
//! 给出与上游相同的算式。
//!
//! GIMPLE（`MOD_Tracer_SpecialPatches.F90`、`MOD_Tracer_Vars.F90`、`CoLMMAIN.F90`）：
//! * 冰川 `trc_input = FMA(R_frost, dep_ice, FMA(precip, R_precip, R_dew*dep_liq))`；
//! * 水体 `trc_input = FMA(R_pool, deficit, FMA(R_frost, dep_ice, FMA(atm, R_precip, R_dew*dep_liq)))`；
//! * 同位素 `trc_wa = FMS(wa + ref_water, R_mix, ref_mass)`（`sync_tracer_patch_ratio`）；
//! * 溢出 `overflow += ((w1 - dz*den)/dt)*dt`，乘加均独立舍入；
//! * 其余（含 `sum(...)` 从 0 起逐项）为源码顺序的独立舍入。

use anyhow::{bail, Result};
use colm_numeric::Contract;

use super::conservation::{
    tracer_apply_reactive_processes, tracer_balance_check, tracer_save_storage, BalanceCheckInput,
    BalanceSnapshot, BalanceTracker, SaveStorageInput,
};
use super::evap_limit::atmospheric_tracer_loss;
use super::hist::{tracer_hist_accumulate, HistAccumulateInput};
use super::{
    soisno_slot, EvapKind, PatchTracerState, TracerDescriptor, TracerPhysics, TracerPools,
    TracerSet, MAX_SNOW_LAYERS, SOIL_LAYERS, SOISNO_LAYERS, TRC_TINY, TRC_WATER_MIN_FOR_RATIO,
};

/// `denh2o`。
const DENH2O: f64 = 1000.0;
/// `denice`。
const DENICE: f64 = 917.0;

/// CoLMMAIN 冰川分支的 `glacier_overflow_mass_trc`：顶层液水、冰超出 `dz*den` 的部分
/// （先液后冰），每段 `wextra = (w - dz*den)/deltim`，累加 `wextra*deltim`。
/// 参数是该分支开头（溢出处理前）的 `wliq_soisno(1)`、`wice_soisno(1)`、`dz_soisno(1)`。
pub fn glacier_overflow_mass(wliq1: f64, wice1: f64, dz1: f64, deltim: f64) -> f64 {
    let mut overflow = 0.0;
    let cap_liq = dz1 * DENH2O;
    if wliq1 > cap_liq {
        let wextra = (wliq1 - cap_liq) / deltim;
        overflow += deltim * wextra;
    }
    let cap_ice = dz1 * DENICE;
    if wice1 > cap_ice {
        let wextra = (wice1 - cap_ice) / deltim;
        overflow += deltim * wextra;
    }
    overflow
}

/// `sync_tracer_patch_ratio`：按混合比值 `r_mix` 重建一个示踪物的存储。
///
/// `snl` 是记账下界：`j < snl+1` 的层清零（液、冰；固相不动），以免藏在已不存在的
/// 雪层里的示踪物漏出守恒账。`aquifer_ref_water` 是 `trc_aquifer_ref_water(ipatch)`。
#[allow(clippy::too_many_arguments)]
pub fn sync_tracer_patch_ratio(
    tracer: &TracerDescriptor,
    pools: &mut TracerPools,
    aquifer_ref_water: f64,
    snl: i32,
    wliq_soisno: &[f64; SOISNO_LAYERS],
    wice_soisno: &[f64; SOISNO_LAYERS],
    wa: f64,
    wdsrf: f64,
    scv: f64,
    r_mix: f64,
) {
    pools.ldew_rain = 0.0;
    pools.ldew_snow = 0.0;
    for j in -(MAX_SNOW_LAYERS as i32) + 1..=SOIL_LAYERS as i32 {
        let slot = soisno_slot(j);
        if j > snl {
            pools.wliq_soisno[slot] = wliq_soisno[slot].max(0.0) * r_mix;
            pools.wice_soisno[slot] = wice_soisno[slot].max(0.0) * r_mix;
            tracer.equilibrate_dissolved(
                wliq_soisno[slot].max(0.0),
                &mut pools.wliq_soisno[slot],
                &mut pools.solid_soisno[slot],
            );
        } else {
            pools.wliq_soisno[slot] = 0.0;
            pools.wice_soisno[slot] = 0.0;
        }
    }
    pools.wa = if tracer.is_isotope() {
        (wa + aquifer_ref_water).contract(r_mix, -pools.aquifer_ref_mass)
    } else {
        wa * r_mix
    };
    pools.wdsrf = wdsrf.max(0.0) * r_mix;
    tracer.equilibrate_dissolved(wa, &mut pools.wa, &mut pools.subsurface_solid);
    tracer.equilibrate_dissolved(wdsrf.max(0.0), &mut pools.wdsrf, &mut pools.surface_solid);
    pools.scv = if snl < 0 { 0.0 } else { scv.max(0.0) * r_mix };
}

/// `move_special_subsurface_residue_to_surface`。
fn move_subsurface_residue_to_surface(tracer: &TracerDescriptor, pools: &mut TracerPools) {
    if pools.subsurface_residue <= TRC_TINY {
        return;
    }
    if tracer.has_dissolved_limit() {
        pools.surface_solid += pools.subsurface_residue;
    } else {
        pools.surface_residue += pools.subsurface_residue;
    }
    pools.subsurface_residue = 0.0;
}

/// `solid_inventory(itrc, ipatch, lb, nl_soil)`。
fn solid_inventory(pools: &TracerPools, lb: i32) -> f64 {
    let mut inventory = ((pools.canopy_solid + pools.surface_solid) + pools.subsurface_solid)
        + pools.waterstorage_solid;
    if lb <= SOIL_LAYERS as i32 {
        let mut sum = 0.0;
        for j in lb..=SOIL_LAYERS as i32 {
            sum += pools.solid_soisno[soisno_slot(j)];
        }
        inventory += sum;
    }
    inventory
}

/// 两类 patch 开头相同的归并：非挥发溶质把不存在的载体里的示踪物移进地表隔离库
/// （有溶解度上限的进地表固相），再把这些载体清零。
fn quarantine_carrierless_pools(tracer: &TracerDescriptor, pools: &mut TracerPools) {
    if tracer.is_nonvolatile_solute() {
        move_subsurface_residue_to_surface(tracer, pools);
        if tracer.has_dissolved_limit() {
            pools.surface_solid = ((((pools.surface_solid + pools.surface_residue)
                + pools.ldew_rain)
                + pools.ldew_snow)
                + pools.wetwat)
                + pools.canopy_solid;
            pools.surface_residue = 0.0;
            let mut snow_solid = 0.0;
            for j in -(MAX_SNOW_LAYERS as i32) + 1..=0 {
                snow_solid += pools.solid_soisno[soisno_slot(j)];
            }
            pools.surface_solid += snow_solid;
            for j in -(MAX_SNOW_LAYERS as i32) + 1..=0 {
                pools.solid_soisno[soisno_slot(j)] = 0.0;
            }
            pools.surface_solid =
                (pools.surface_solid + pools.waterstorage) + pools.waterstorage_solid;
        } else {
            pools.surface_residue =
                ((pools.surface_residue + pools.ldew_rain) + pools.ldew_snow) + pools.wetwat;
            pools.surface_residue += pools.waterstorage;
        }
    }
    pools.ldew_rain = 0.0;
    pools.ldew_snow = 0.0;
    pools.wetwat = 0.0;
    pools.canopy_solid = 0.0;
    pools.waterstorage = 0.0;
    pools.waterstorage_solid = 0.0;
}

/// 混合箱一步的水量（两类 patch 共用）。
struct BoxWater {
    /// 计入示踪物输入的大气降水（水体为 `atm_precip_mass`，冰川为 `precip_mass`）。
    atm_precip_mass: f64,
    /// 水体的湖亏补水（按箱内比值补），冰川为 `None`。
    deficit_mass: Option<f64>,
    rnof_mass: f64,
    evap_liq_mass: f64,
    evap_ice_mass: f64,
    dep_liq_mass: f64,
    dep_ice_mass: f64,
    water_input: f64,
    water_beg: f64,
    water_end: f64,
    /// 示踪物残留回到液相的顶层 `wliq_soisno(snl+1)`（冰川为层 1）。
    surface_wliq: f64,
    wdsrf: f64,
    held_lb: i32,
    subl_skin_mm: f64,
    t_grnd: f64,
}

/// 箱内记账的结果。
struct BoxFluxes {
    trc_input: f64,
    trc_evap_liq: f64,
    trc_evap_ice: f64,
    trc_rnof: f64,
    r_final: f64,
}

/// 混合箱（`mixed_signature`）分支：只在不分馏时调用（分馏需要的平衡/蒸发比值尚未移植）。
/// 冰川/水体箱式记账的分馏环境（`glacier_evap_ratio_for`/`waterbody_evap_ratio_for`）。
#[derive(Clone, Copy)]
struct SpecialFrac {
    physics: TracerPhysics,
    forc_q: f64,
    forc_psrf: f64,
    /// 液面动力分馏：冰川 `craig_gordon(.false.)`，水体开阔水面 `open_water(|u|)`。
    open_water_wind: Option<f64>,
}

impl SpecialFrac {
    fn evap_ratio(
        &self,
        tracer: &TracerDescriptor,
        r_vapor: f64,
        source_ratio: f64,
        temp_k: f64,
        from_ice: bool,
    ) -> f64 {
        let physics = self.physics;
        if !physics.fractionation_active(tracer) {
            return source_ratio;
        }
        let alpha_k = if from_ice {
            physics.alpha_kinetic_craig_gordon(tracer, true)
        } else {
            match self.open_water_wind {
                Some(wind) => physics.alpha_kinetic_open_water(tracer, wind),
                None => physics.alpha_kinetic_craig_gordon(tracer, false),
            }
        };
        let relhum = super::frac::surface_relhum(self.forc_q, self.forc_psrf, temp_k, from_ice);
        physics.craig_gordon_evap_ratio(
            tracer,
            source_ratio,
            r_vapor,
            temp_k,
            relhum,
            alpha_k,
            from_ice,
        )
    }
}

fn mixed_box(
    tracer: &TracerDescriptor,
    frac: &SpecialFrac,
    pools: &mut TracerPools,
    storage_beg: f64,
    r_precip: f64,
    r_vapor: f64,
    box_water: &BoxWater,
) -> BoxFluxes {
    let nonvolatile = tracer.is_nonvolatile_solute();
    let mut surface_residue_beg = pools.surface_residue;
    let trc_held_storage = (pools.subsurface_residue + surface_residue_beg)
        + solid_inventory(pools, box_water.held_lb);
    let water_beg = box_water.water_beg;
    let active = frac.physics.fractionation_active(tracer);
    let (r_dew, r_frost) = if nonvolatile {
        (0.0, 0.0)
    } else if active {
        (
            frac.physics
                .equilibrium_deposition_ratio(tracer, r_vapor, box_water.t_grnd, false),
            frac.physics
                .equilibrium_deposition_ratio(tracer, r_vapor, box_water.t_grnd, true),
        )
    } else {
        (r_vapor, r_vapor)
    };
    // `FMA(R_frost, dep_ice, FMA(precip, R_precip, R_dew*dep_liq))`。
    let mut trc_input = r_frost.contract(
        box_water.dep_ice_mass,
        box_water
            .atm_precip_mass
            .contract(r_precip, r_dew * box_water.dep_liq_mass),
    );
    // 水体的湖亏补水按箱内比值：`FMA(R_pool, deficit, ...)`；冰川没有这一项。
    if let Some(deficit_mass) = box_water.deficit_mass {
        let r_pool = if water_beg > TRC_WATER_MIN_FOR_RATIO {
            (storage_beg - trc_held_storage).max(0.0) / water_beg
        } else {
            r_precip
        };
        trc_input = r_pool.contract(deficit_mass, trc_input);
    }
    let trc_available = ((storage_beg - trc_held_storage) + trc_input).max(0.0);
    let water_before_output = water_beg + box_water.water_input;
    let (trc_evap_liq, trc_evap_ice) = if nonvolatile {
        (0.0, 0.0)
    } else {
        let evap_ratio = |source_ratio: f64, temp_k: f64, from_ice: bool| {
            frac.evap_ratio(tracer, r_vapor, source_ratio, temp_k, from_ice)
        };
        // `tracer_ratio_cap`：分馏时 `ref_ratio*(1+2000/1000)`，否则 0。
        let ratio_cap = if active { tracer.ref_ratio * 3.0 } else { 0.0 };
        let liq = atmospheric_tracer_loss(
            trc_available,
            water_before_output,
            box_water.evap_liq_mass,
            box_water.t_grnd,
            false,
            &evap_ratio,
            TRC_TINY,
            ratio_cap,
            nonvolatile,
            None,
        );
        let trc_after_liq = trc_available - liq;
        let water_after_liq = water_before_output - box_water.evap_liq_mass;
        let ice = atmospheric_tracer_loss(
            trc_after_liq,
            water_after_liq,
            box_water.evap_ice_mass,
            box_water.t_grnd,
            true,
            &evap_ratio,
            TRC_TINY,
            ratio_cap,
            nonvolatile,
            Some(box_water.subl_skin_mm),
        );
        (liq, ice)
    };
    let trc_evap = trc_evap_liq + trc_evap_ice;
    let evap_mass = box_water.evap_liq_mass + box_water.evap_ice_mass;
    let water_after_evap = water_before_output - evap_mass;
    let mut trc_final = (trc_available - trc_evap).max(0.0);
    tracer.equilibrate_dissolved(water_after_evap, &mut trc_final, &mut pools.surface_solid);
    let r_runoff = if water_after_evap > TRC_WATER_MIN_FOR_RATIO {
        trc_final / water_after_evap
    } else {
        0.0
    };
    let rnof_mass = box_water.rnof_mass;
    let mut trc_rnof = (rnof_mass * r_runoff).min(trc_final);
    let surface_liquid_end = box_water.wdsrf.max(0.0) + box_water.surface_wliq.max(0.0);
    let surface_carrier = surface_liquid_end + rnof_mass;
    let mut surface_residue_export = 0.0;
    if nonvolatile
        && !tracer.has_dissolved_limit()
        && surface_residue_beg > TRC_TINY
        && rnof_mass > TRC_TINY
        && surface_carrier > TRC_WATER_MIN_FOR_RATIO
    {
        surface_residue_export =
            (surface_residue_beg * rnof_mass / surface_carrier).min(surface_residue_beg);
        surface_residue_beg -= surface_residue_export;
        trc_rnof += surface_residue_export;
    }
    // 残留输出来自隔离库，不是 trc_available。
    trc_final = ((trc_final - trc_rnof) + surface_residue_export).max(0.0);
    tracer.equilibrate_dissolved(
        box_water.water_end,
        &mut trc_final,
        &mut pools.surface_solid,
    );
    let r_final = if box_water.water_end > TRC_WATER_MIN_FOR_RATIO {
        if nonvolatile {
            pools.surface_residue = surface_residue_beg;
        }
        trc_final / box_water.water_end
    } else {
        if nonvolatile {
            pools.surface_residue = surface_residue_beg + trc_final;
        }
        0.0
    };
    BoxFluxes {
        trc_input,
        trc_evap_liq,
        trc_evap_ice,
        trc_rnof,
        r_final,
    }
}

/// 固定签名分支：所有通量 = 水量 × `R_init`。
fn fixed_box(r_init: f64, box_water: &BoxWater) -> BoxFluxes {
    BoxFluxes {
        trc_input: box_water.water_input * r_init,
        trc_evap_liq: box_water.evap_liq_mass * r_init,
        trc_evap_ice: box_water.evap_ice_mass * r_init,
        trc_rnof: box_water.rnof_mass * r_init,
        r_final: r_init,
    }
}

/// 记账、重建存储之后，非挥发溶质的地表残留若有液相载体则并回（地表水优先，其次顶层液水）。
fn release_surface_residue(
    tracer: &TracerDescriptor,
    pools: &mut TracerPools,
    wdsrf: f64,
    top_slot: usize,
    top_wliq: f64,
) {
    if tracer.is_nonvolatile_solute()
        && !tracer.has_dissolved_limit()
        && pools.surface_residue > TRC_TINY
    {
        if wdsrf > TRC_WATER_MIN_FOR_RATIO {
            pools.wdsrf += pools.surface_residue;
            pools.surface_residue = 0.0;
        } else if top_wliq > TRC_WATER_MIN_FOR_RATIO {
            pools.wliq_soisno[top_slot] += pools.surface_residue;
            pools.surface_residue = 0.0;
        }
    }
}

/// 一个示踪物的箱内记账与存储重建（两类 patch 共用的循环体）。
#[allow(clippy::too_many_arguments)]
fn account_tracer(
    tracer: &TracerDescriptor,
    frac: &SpecialFrac,
    itrc: usize,
    state: &mut PatchTracerState,
    runtime_forced: bool,
    r_precip: f64,
    r_vapor: f64,
    box_water: &BoxWater,
    sync_snl: i32,
    wliq_soisno: &[f64; SOISNO_LAYERS],
    wice_soisno: &[f64; SOISNO_LAYERS],
    wa: f64,
    scv: f64,
    top_slot: usize,
) {
    state.step[itrc].rnof_step = 0.0;
    let r_init = tracer.init_water_ratio();
    let fixed_signature = tracer.can_use_fixed_signature()
        && !frac.physics.fractionation_active(tracer)
        && !runtime_forced;
    let storage_beg = state.step[itrc].storage_beg;
    let fluxes = if fixed_signature {
        fixed_box(r_init, box_water)
    } else {
        mixed_box(
            tracer,
            frac,
            &mut state.pools[itrc],
            storage_beg,
            r_precip,
            r_vapor,
            box_water,
        )
    };
    if fluxes.trc_input > 0.0 {
        state.acc[itrc].precip += fluxes.trc_input;
    }
    state.book_evap_loss(
        itrc,
        fluxes.trc_evap_liq,
        box_water.evap_liq_mass,
        EvapKind::SoilEvaporation,
    );
    state.book_evap_loss(
        itrc,
        fluxes.trc_evap_ice,
        box_water.evap_ice_mass,
        EvapKind::Sublimation,
    );
    if fluxes.trc_rnof > 0.0 {
        state.step[itrc].rnof_step = fluxes.trc_rnof;
        state.acc[itrc].rsur += fluxes.trc_rnof;
        state.acc[itrc].rnof += fluxes.trc_rnof;
    }
    let aquifer_ref_water = state.aquifer_ref_water;
    let pools = &mut state.pools[itrc];
    sync_tracer_patch_ratio(
        tracer,
        pools,
        aquifer_ref_water,
        sync_snl,
        wliq_soisno,
        wice_soisno,
        wa,
        box_water.wdsrf,
        scv,
        fluxes.r_final,
    );
    release_surface_residue(
        tracer,
        pools,
        box_water.wdsrf,
        top_slot,
        wliq_soisno[top_slot],
    );
}

/// `tracer_glacier_patch` 的实参与它读的模块量。
#[derive(Debug, Clone, Copy)]
pub struct GlacierInput<'a> {
    /// `ipatch`（1 起，只用于守恒报告）。
    pub ipatch: i32,
    pub deltim: f64,
    pub prc_rain: f64,
    pub prl_rain: f64,
    pub prc_snow: f64,
    pub prl_snow: f64,
    pub rnof: f64,
    pub qseva: f64,
    pub qsubl: f64,
    pub qsdew: f64,
    pub qfros: f64,
    pub endwb: f64,
    pub totwb: f64,
    /// CoLMMAIN 的 `glacier_overflow_mass_trc`（[`glacier_overflow_mass`]）。
    pub glacier_overflow_mass: f64,
    pub errorw: f64,
    pub wdsrf: f64,
    pub scv: f64,
    pub t_grnd: f64,
    pub forc_q: f64,
    pub forc_psrf: f64,
    /// 步末 `wliq_soisno(maxsnl+1:nl_soil)`。
    pub wliq_soisno: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno: &'a [f64; SOISNO_LAYERS],
    /// `DEF_TRACER_SUBL_SKIN_MM`。
    pub subl_skin_mm: f64,
    /// `tracer_forcing_precip_value(itrc, ipatch)`，逐示踪物。
    pub precip_ratio: &'a [f64],
    /// `tracer_forcing_vapor_value(itrc, ipatch)`，逐示踪物。
    pub vapor_ratio: &'a [f64],
    /// `trc_runtime_forced(itrc)`。
    pub runtime_forced: &'a [bool],
    /// 编译宏 `CatchLateralFlow`（传给守恒检查）。
    pub catch_lateral_flow: bool,
}

/// `tracer_glacier_patch`：返回守恒检查的 `xerr_tracer`（上游丢弃）。
pub fn tracer_glacier_patch(
    set: &TracerSet,
    physics: TracerPhysics,
    state: &mut PatchTracerState,
    snapshot: &mut BalanceSnapshot,
    tracker: &mut BalanceTracker,
    input: &GlacierInput<'_>,
) -> Result<f64> {
    if set.is_empty() {
        return Ok(0.0);
    }
    if state.aquifer_ref_water > 0.0 {
        bail!("glacier transition with isotope aquifer reference needs explicit water transfer");
    }
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        // 冰川只有一个整体雪载体 scv：先把分层雪的示踪物并进来。
        let pools = &mut state.pools[itrc];
        let mut liq = 0.0;
        let mut ice = 0.0;
        for j in -(MAX_SNOW_LAYERS as i32) + 1..=0 {
            liq += pools.wliq_soisno[soisno_slot(j)];
        }
        for j in -(MAX_SNOW_LAYERS as i32) + 1..=0 {
            ice += pools.wice_soisno[soisno_slot(j)];
        }
        pools.scv = (pools.scv + liq) + ice;
        for j in -(MAX_SNOW_LAYERS as i32) + 1..=0 {
            pools.wliq_soisno[soisno_slot(j)] = 0.0;
            pools.wice_soisno[soisno_slot(j)] = 0.0;
        }
        quarantine_carrierless_pools(tracer, pools);
    }

    let snl_trc = 0;
    tracer_save_storage(
        set,
        physics,
        state,
        snapshot,
        &SaveStorageInput {
            snl: snl_trc,
            waterstorage: None,
            runtime_forced: input.runtime_forced,
        },
    );
    let dt = input.deltim;
    let precip_mass = (((input.prc_rain + input.prl_rain) + input.prc_snow) + input.prl_snow) * dt;
    let rnof_mass = input.rnof.max(0.0) * dt;
    let evap_liq_mass = input.qseva.max(0.0) * dt;
    let evap_ice_mass = input.qsubl.max(0.0) * dt;
    let dep_liq_mass = input.qsdew.max(0.0) * dt;
    let dep_ice_mass = input.qfros.max(0.0) * dt;
    let evap_mass = evap_liq_mass + evap_ice_mass;
    let dep_mass = dep_liq_mass + dep_ice_mass;
    let water_input = precip_mass + dep_mass;
    let water_ds = (input.endwb - input.totwb) - input.glacier_overflow_mass;
    let mut water_end = input.wdsrf.max(0.0) + input.scv.max(0.0);
    for j in 1..=SOIL_LAYERS as i32 {
        let slot = soisno_slot(j);
        water_end =
            (water_end + input.wliq_soisno[slot].max(0.0)) + input.wice_soisno[slot].max(0.0);
    }
    let water_beg = water_end - water_ds;
    let top_slot = soisno_slot(1);
    let glacier_frac = SpecialFrac {
        physics,
        forc_q: input.forc_q,
        forc_psrf: input.forc_psrf,
        open_water_wind: None,
    };
    let box_water = BoxWater {
        atm_precip_mass: precip_mass,
        deficit_mass: None,
        rnof_mass,
        evap_liq_mass,
        evap_ice_mass,
        dep_liq_mass,
        dep_ice_mass,
        water_input,
        water_beg,
        water_end,
        surface_wliq: input.wliq_soisno[top_slot],
        wdsrf: input.wdsrf,
        held_lb: 1,
        subl_skin_mm: input.subl_skin_mm,
        t_grnd: input.t_grnd,
    };
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        account_tracer(
            tracer,
            &glacier_frac,
            itrc,
            state,
            input.runtime_forced[itrc],
            input.precip_ratio[itrc],
            input.vapor_ratio[itrc],
            &box_water,
            snl_trc,
            input.wliq_soisno,
            input.wice_soisno,
            0.0,
            input.scv,
            top_slot,
        );
    }

    tracer_apply_reactive_processes(set, state, snl_trc, dt);
    let xerr = tracer_balance_check(
        set,
        physics,
        state,
        snapshot,
        tracker,
        &BalanceCheckInput {
            ipatch: input.ipatch,
            snl: snl_trc,
            deltim: dt,
            patchtype: Some(3),
            water_err: Some(input.errorw),
            water_ds: Some(water_ds),
            water_input: Some(precip_mass + dep_mass),
            water_output: Some(evap_mass + rnof_mass),
            water_evap: Some(evap_mass),
            water_rnof: Some(rnof_mass),
            flood_heterogeneous: None,
            catch_lateral_flow: input.catch_lateral_flow,
            runtime_forced: input.runtime_forced,
        },
    );
    tracer_hist_accumulate(
        set,
        state,
        &HistAccumulateInput {
            snl: snl_trc,
            ldew_rain: 0.0,
            ldew_snow: 0.0,
            wliq_soisno: input.wliq_soisno,
            wice_soisno: input.wice_soisno,
            wa: 0.0,
            wdsrf: input.wdsrf,
            wetwat: 0.0,
            scv: input.scv,
        },
    );
    Ok(xerr)
}

/// `tracer_waterbody_patch` 的实参与它读的模块量。
#[derive(Debug, Clone, Copy)]
pub struct WaterbodyInput<'a> {
    /// `ipatch`（1 起，只用于守恒报告）。
    pub ipatch: i32,
    pub snl: i32,
    pub deltim: f64,
    pub forc_rain: f64,
    pub forc_snow: f64,
    pub lake_deficit: f64,
    pub rnof: f64,
    pub qseva: f64,
    pub qsubl: f64,
    pub qsdew: f64,
    pub qfros: f64,
    pub endwb: f64,
    pub totwb: f64,
    pub errorw: f64,
    pub wa: f64,
    pub wdsrf: f64,
    pub scv: f64,
    pub t_grnd: f64,
    pub forc_q: f64,
    pub forc_psrf: f64,
    pub forc_us: f64,
    pub forc_vs: f64,
    /// 步末 `wliq_soisno(maxsnl+1:nl_soil)`。
    pub wliq_soisno: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno: &'a [f64; SOISNO_LAYERS],
    /// `DEF_USE_Dynamic_Lake`。
    pub use_dynamic_lake: bool,
    /// `DEF_TRACER_SUBL_SKIN_MM`。
    pub subl_skin_mm: f64,
    /// `waterbody_hist_sample`（CoLMDRIVER 在子步循环里只让最后一个子步累加 history）。
    pub hist_sample: bool,
    /// `tracer_forcing_precip_value(itrc, ipatch)`，逐示踪物。
    pub precip_ratio: &'a [f64],
    /// `tracer_forcing_vapor_value(itrc, ipatch)`，逐示踪物。
    pub vapor_ratio: &'a [f64],
    /// `trc_runtime_forced(itrc)`。
    pub runtime_forced: &'a [bool],
    /// 编译宏 `CatchLateralFlow`（传给守恒检查）。
    pub catch_lateral_flow: bool,
}

/// `tracer_waterbody_patch`：返回守恒检查的 `xerr_tracer`（上游丢弃）。
pub fn tracer_waterbody_patch(
    set: &TracerSet,
    physics: TracerPhysics,
    state: &mut PatchTracerState,
    snapshot: &mut BalanceSnapshot,
    tracker: &mut BalanceTracker,
    input: &WaterbodyInput<'_>,
) -> Result<f64> {
    if set.is_empty() {
        return Ok(0.0);
    }
    if state.aquifer_ref_water > 0.0 {
        bail!("waterbody transition with isotope aquifer reference needs explicit water transfer");
    }
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        quarantine_carrierless_pools(tracer, &mut state.pools[itrc]);
    }

    let maxsnl = -(MAX_SNOW_LAYERS as i32);
    tracer_save_storage(
        set,
        physics,
        state,
        snapshot,
        &SaveStorageInput {
            snl: maxsnl,
            waterstorage: None,
            runtime_forced: input.runtime_forced,
        },
    );
    let dt = input.deltim;
    let atm_precip_mass = (input.forc_rain + input.forc_snow) * dt;
    let precip_mass = if input.use_dynamic_lake {
        atm_precip_mass
    } else {
        ((input.forc_rain + input.forc_snow) + input.lake_deficit) * dt
    };
    let deficit_mass = precip_mass - atm_precip_mass;
    let rnof_mass = input.rnof.max(0.0) * dt;
    let evap_liq_mass = input.qseva.max(0.0) * dt;
    let evap_ice_mass = input.qsubl.max(0.0) * dt;
    let dep_liq_mass = input.qsdew.max(0.0) * dt;
    let dep_ice_mass = input.qfros.max(0.0) * dt;
    let evap_mass = evap_liq_mass + evap_ice_mass;
    let dep_mass = dep_liq_mass + dep_ice_mass;
    let water_input = precip_mass + dep_mass;
    let water_ds = input.endwb - input.totwb;
    let mut water_end = input.wa + input.wdsrf.max(0.0);
    for j in maxsnl + 1..=SOIL_LAYERS as i32 {
        if j > input.snl {
            let slot = soisno_slot(j);
            water_end =
                (water_end + input.wliq_soisno[slot].max(0.0)) + input.wice_soisno[slot].max(0.0);
        }
    }
    if input.snl >= 0 {
        water_end += input.scv.max(0.0);
    }
    let water_beg = water_end - water_ds;
    let top_slot = soisno_slot(input.snl + 1);
    let waterbody_frac = SpecialFrac {
        physics,
        forc_q: input.forc_q,
        forc_psrf: input.forc_psrf,
        // `sqrt(FMA(us, us, vs*vs))`（`max(·,0)` 被优化掉）。
        open_water_wind: Some(
            input
                .forc_us
                .contract(input.forc_us, input.forc_vs * input.forc_vs)
                .sqrt(),
        ),
    };
    let box_water = BoxWater {
        atm_precip_mass,
        deficit_mass: Some(deficit_mass),
        rnof_mass,
        evap_liq_mass,
        evap_ice_mass,
        dep_liq_mass,
        dep_ice_mass,
        water_input,
        water_beg,
        water_end,
        surface_wliq: input.wliq_soisno[top_slot],
        wdsrf: input.wdsrf,
        held_lb: maxsnl + 1,
        subl_skin_mm: input.subl_skin_mm,
        t_grnd: input.t_grnd,
    };
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        account_tracer(
            tracer,
            &waterbody_frac,
            itrc,
            state,
            input.runtime_forced[itrc],
            input.precip_ratio[itrc],
            input.vapor_ratio[itrc],
            &box_water,
            input.snl,
            input.wliq_soisno,
            input.wice_soisno,
            input.wa,
            input.scv,
            top_slot,
        );
    }

    tracer_apply_reactive_processes(set, state, maxsnl, dt);
    let xerr = tracer_balance_check(
        set,
        physics,
        state,
        snapshot,
        tracker,
        &BalanceCheckInput {
            ipatch: input.ipatch,
            snl: maxsnl,
            deltim: dt,
            patchtype: Some(4),
            water_err: Some(input.errorw),
            water_ds: Some(water_ds),
            water_input: Some(precip_mass + dep_mass),
            water_output: Some(evap_mass + rnof_mass),
            water_evap: Some(evap_mass),
            water_rnof: Some(rnof_mass),
            flood_heterogeneous: None,
            catch_lateral_flow: input.catch_lateral_flow,
            runtime_forced: input.runtime_forced,
        },
    );
    if input.hist_sample {
        tracer_hist_accumulate(
            set,
            state,
            &HistAccumulateInput {
                snl: input.snl,
                ldew_rain: 0.0,
                ldew_snow: 0.0,
                wliq_soisno: input.wliq_soisno,
                wice_soisno: input.wice_soisno,
                wa: input.wa,
                wdsrf: input.wdsrf,
                wetwat: 0.0,
                scv: input.scv,
            },
        );
    }
    Ok(xerr)
}

#[cfg(test)]
#[path = "special_patches_tests.rs"]
mod tests;
