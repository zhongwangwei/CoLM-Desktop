//! 示踪物守恒检查（`MOD_Tracer_Conservation`）。
//!
//! 每步开头 [`tracer_save_storage`] 记下各存储分量与累加器快照，步末
//! [`tracer_apply_reactive_processes`] 做一级衰减，[`tracer_balance_check`] 比较
//! `Δ存储 = 输入 − 输出 + 源汇`，把超差的 (patch, 示踪物) 记进 [`BalanceTracker`]；
//! 整步跑完后 [`BalanceTracker::report`] 生成与上游相同的日志行并给出是否中止。
//!
//! 上游把快照放在模块级 `snap_*(ntracers, numpatch)` 数组里；这里是每个 patch 一份
//! [`BalanceSnapshot`]，由调用方与 [`PatchTracerState`] 一起保存（`state.rs` 不动）。
//! 最差项追踪器是模块级 `save` 变量，这里是显式的 [`BalanceTracker`]。单进程，
//! 不做 MPI 归约（`owner` 恒为 0）。
//!
//! 收缩形状取自 `MOD_Tracer_Conservation.F90` 的 GIMPLE（default 内核，非
//! `CatchLateralFlow`）：
//! * 存储分量与 `sum(storage_comp)` 都从 0 起逐项相加；
//! * 衰减 `pool = pool*(1-f)`、`source_sink = (source_sink + pool) - before`；
//! * `err = ((storage_end - storage_beg) - step_input_check) + step_output_check`；
//! * `balance_tol = FMA(scale, 1e-10, 1e-12)`，`signature_tol` 同形；
//!   `resid_tol = max(scale*1e-6, 5e-5)` 独立舍入；
//! * `dS_minus_water_R = FNMA(water_dS, R_init, dS)`、
//!   `out_minus_water_R = FNMA(water_output, R_init, step_output)`；
//!   `in/evap/rnof_minus_water_R` 的乘积另有他用，独立舍入后相减。

use super::{soisno_slot, PatchTracerState, TracerPhysics, TracerSet, SOIL_LAYERS, TRC_TINY};

/// `trc_balance_abs_tol`。
const BALANCE_ABS_TOL: f64 = 1.0e-12;
/// `trc_balance_rel_tol`。
const BALANCE_REL_TOL: f64 = 1.0e-10;
/// `trc_resid_warn_frac`。
const RESID_WARN_FRAC: f64 = 1.0e-6;
/// `trc_resid_abs_tol`。
const RESID_ABS_TOL: f64 = 5.0e-5;
/// `trc_resid_rel_tol`。
const RESID_REL_TOL: f64 = 1.0e-6;
/// `n_storage_diag`。
pub const N_STORAGE_DIAG: usize = 12;
/// `n_flux_diag`。
pub const N_FLUX_DIAG: usize = 7;
/// `balance_worst_diag` 的长度。
pub const N_BALANCE_DIAG: usize = 19;

/// 一个示踪物在步首的快照（上游 `snap_*(itrc, ipatch)` 与 `snap_storage_comp(:, itrc, ipatch)`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TracerSnapshot {
    pub precip: f64,
    pub vapor_exchange: f64,
    pub evap: f64,
    pub rnof: f64,
    pub rsur: f64,
    pub rsub: f64,
    pub qinfl: f64,
    pub qcharge: f64,
    /// 1 ldew、2 雪/土液、3 雪/土冰、4 wa、5 wdsrf、6 wetwat、7 scv、8 waterstorage、
    /// 9 叶 NSS、10 地表残留、11 地下残留、12 固相。
    pub storage_comp: [f64; N_STORAGE_DIAG],
}

/// 一个 patch 的步首快照（逐示踪物，与 [`TracerSet::tracers`] 同序）。
///
/// 上游的 `snap_*` 首次调用时分配并清零、跨步保留；不走通用输运的示踪物永远是 0。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BalanceSnapshot {
    pub tracers: Vec<TracerSnapshot>,
}

impl BalanceSnapshot {
    pub fn new(set: &TracerSet) -> Self {
        Self {
            tracers: vec![TracerSnapshot::default(); set.len()],
        }
    }
}

/// `tracer_save_storage` 的实参与它读的模块量。
#[derive(Debug, Clone, Copy)]
pub struct SaveStorageInput<'a> {
    /// `snl`：累计下界取 `max(maxsnl+1, snl+1)`。
    pub snl: i32,
    /// 可选实参 `waterstorage`（CROP 且 `DEF_USE_IRRIGATION` 时为 `waterstorage_trc_beg`）。
    pub waterstorage: Option<f64>,
    /// `trc_runtime_forced(itrc)`（`MOD_Tracer_Forcing` 设定），逐示踪物。
    pub runtime_forced: &'a [bool],
}

/// `max(lbound(trc_wliq_soisno, 2), snl + 1)`。
fn active_lower_bound(snl: i32) -> i32 {
    (-(super::MAX_SNOW_LAYERS as i32) + 1).max(snl + 1)
}

/// 按 `tracer_save_storage`/`tracer_balance_check` 共用的顺序拆存储分量。
fn storage_components(state: &PatchTracerState, itrc: usize, lb: i32) -> [f64; N_STORAGE_DIAG] {
    let pools = &state.pools[itrc];
    let mut comp = [0.0; N_STORAGE_DIAG];
    comp[0] = pools.ldew_rain + pools.ldew_snow;
    for j in lb..=SOIL_LAYERS as i32 {
        comp[1] += pools.wliq_soisno[soisno_slot(j)];
        comp[2] += pools.wice_soisno[soisno_slot(j)];
    }
    comp[3] = pools.wa;
    comp[4] = pools.wdsrf;
    comp[5] = pools.wetwat;
    comp[6] = pools.scv;
    comp[7] = pools.waterstorage;
    comp[8] = pools.leaf_iso_storage;
    comp[9] = pools.surface_residue;
    comp[10] = pools.subsurface_residue;
    for j in lb..=SOIL_LAYERS as i32 {
        comp[11] += pools.solid_soisno[soisno_slot(j)];
    }
    comp[11] += pools.canopy_solid;
    comp[11] += pools.surface_solid;
    comp[11] += pools.subsurface_solid;
    comp[11] += pools.waterstorage_solid;
    comp
}

/// `sum(storage_comp)`：从 0 起逐项相加。
fn sum_components(comp: &[f64; N_STORAGE_DIAG]) -> f64 {
    comp.iter().fold(0.0, |acc, value| acc + value)
}

/// `tracer_save_storage`：清本步的径流/源汇临时量、（可选）按 `R_init` 重置灌溉库、
/// 记下存储分量与累加器快照，`trc_storage_beg = sum(storage_comp)`。
pub fn tracer_save_storage(
    set: &TracerSet,
    physics: TracerPhysics,
    state: &mut PatchTracerState,
    snapshot: &mut BalanceSnapshot,
    input: &SaveStorageInput<'_>,
) {
    if set.is_empty() {
        return;
    }
    let lb = active_lower_bound(input.snl);
    if snapshot.tracers.len() != set.len() {
        snapshot.tracers = vec![TracerSnapshot::default(); set.len()];
    }
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let step = &mut state.step[itrc];
        step.rnof_step = 0.0;
        step.reactive_source_step = 0.0;
        step.numerical_residual_step = 0.0;
        step.numerical_water_step = 0.0;

        // 不分馏、无运行时驱动、无溶解度上限的同位素：灌溉库同步到 waterstorage*R_init。
        let fixed_signature_storage = tracer.can_use_fixed_signature()
            && !physics.fractionation_active(tracer)
            && !tracer.has_dissolved_limit()
            && !input.runtime_forced[itrc];
        if let Some(waterstorage) = input.waterstorage {
            if fixed_signature_storage {
                state.pools[itrc].waterstorage = waterstorage.max(0.0) * tracer.init_water_ratio();
            }
        }

        let comp = storage_components(state, itrc, lb);
        state.step[itrc].storage_beg = sum_components(&comp);
        let acc = &state.acc[itrc];
        snapshot.tracers[itrc] = TracerSnapshot {
            precip: acc.precip,
            vapor_exchange: acc.vapor_exchange,
            evap: acc.evap,
            rnof: acc.rnof,
            rsur: acc.rsur,
            rsub: acc.rsub,
            qinfl: acc.qinfl,
            qcharge: acc.qcharge,
            storage_comp: comp,
        };
    }
}

/// `decay_pool`：只衰减正的库（负值是 wa 亏欠记账，不算化学源）。
fn decay_pool(pool: &mut f64, fraction: f64, source_sink: &mut f64) {
    if *pool <= TRC_TINY {
        return;
    }
    let before = *pool;
    *pool *= 1.0 - fraction;
    *source_sink = (*source_sink + *pool) - before;
}

/// `tracer_apply_reactive_processes`：通用水输运示踪物的一级衰减，衰减量记进
/// `trc_reactive_source_step`。
pub fn tracer_apply_reactive_processes(
    set: &TracerSet,
    state: &mut PatchTracerState,
    snl: i32,
    deltim: f64,
) {
    if set.is_empty() {
        return;
    }
    let lb = active_lower_bound(snl);
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let fraction = tracer.reactive_decay_fraction(deltim);
        if fraction <= 0.0 {
            continue;
        }
        let mut source_sink = 0.0;
        let pools = &mut state.pools[itrc];
        decay_pool(&mut pools.ldew_rain, fraction, &mut source_sink);
        decay_pool(&mut pools.ldew_snow, fraction, &mut source_sink);
        for j in lb..=SOIL_LAYERS as i32 {
            let slot = soisno_slot(j);
            decay_pool(&mut pools.wliq_soisno[slot], fraction, &mut source_sink);
            decay_pool(&mut pools.wice_soisno[slot], fraction, &mut source_sink);
        }
        decay_pool(&mut pools.wa, fraction, &mut source_sink);
        decay_pool(&mut pools.wdsrf, fraction, &mut source_sink);
        decay_pool(&mut pools.wetwat, fraction, &mut source_sink);
        decay_pool(&mut pools.scv, fraction, &mut source_sink);
        decay_pool(&mut pools.waterstorage, fraction, &mut source_sink);
        decay_pool(&mut pools.leaf_iso_storage, fraction, &mut source_sink);
        decay_pool(&mut pools.surface_residue, fraction, &mut source_sink);
        decay_pool(&mut pools.subsurface_residue, fraction, &mut source_sink);
        for j in lb..=SOIL_LAYERS as i32 {
            decay_pool(
                &mut pools.solid_soisno[soisno_slot(j)],
                fraction,
                &mut source_sink,
            );
        }
        decay_pool(&mut pools.canopy_solid, fraction, &mut source_sink);
        decay_pool(&mut pools.surface_solid, fraction, &mut source_sink);
        decay_pool(&mut pools.subsurface_solid, fraction, &mut source_sink);
        decay_pool(&mut pools.waterstorage_solid, fraction, &mut source_sink);
        let step = &mut state.step[itrc];
        step.reactive_source_step += source_sink;
    }
}

/// `tracer_balance_check` 的实参与它读的模块量/宏。
///
/// `Option` 字段对应上游的 `optional` 实参：`None` 即"未传"（`present` 为假）。
/// CoLMMAIN（土壤/城市/湿地）传全部，冰川/水体由 [`super::special_patches`] 传。
#[derive(Debug, Clone, Copy)]
pub struct BalanceCheckInput<'a> {
    /// `ipatch`（1 起的 Fortran 下标，只用于报告）。
    pub ipatch: i32,
    pub snl: i32,
    pub deltim: f64,
    /// `patchtype_in`。
    pub patchtype: Option<i32>,
    /// `water_err_in`（CoLMMAIN 传 `errorw`）。
    pub water_err: Option<f64>,
    /// `water_dS_in`（`endwb - totwb`）。
    pub water_ds: Option<f64>,
    /// `water_input_in`（`(forc_prc + forc_prl + flood_input_wb) * deltim`）。
    pub water_input: Option<f64>,
    /// `water_output_in`（非 CatchLateralFlow：`(fevpa_wb + rnof) * deltim`）。
    pub water_output: Option<f64>,
    /// `water_evap_in`（`fevpa_wb * deltim`）。
    pub water_evap: Option<f64>,
    /// `water_rnof_in`（非 CatchLateralFlow：`rnof * deltim`；否则 0）。
    pub water_rnof: Option<f64>,
    /// `flood_heterogeneous_in`（只在 GridRiverLakeFlow 构建里传：`LWINFILT .and. patchtype == 0`）。
    pub flood_heterogeneous: Option<bool>,
    /// 编译宏 `CatchLateralFlow`：打开时径流不进本检查。
    pub catch_lateral_flow: bool,
    /// `trc_runtime_forced(itrc)`。
    pub runtime_forced: &'a [bool],
}

/// 跨 patch 的最差项追踪器（上游模块级 `balance_worst_*`、`signature_worst_*`、
/// `resid_*` 的 `save` 变量）。每步由 [`BalanceTracker::report`] 清空。
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceTracker {
    pub balance_worst_err: f64,
    pub balance_worst_diag: [f64; N_BALANCE_DIAG],
    pub balance_worst_sbeg: [f64; N_STORAGE_DIAG],
    pub balance_worst_send: [f64; N_STORAGE_DIAG],
    pub balance_worst_sds: [f64; N_STORAGE_DIAG],
    pub balance_worst_fcomp: [f64; N_FLUX_DIAG],
    pub balance_worst_ipatch: i32,
    pub balance_worst_itrc: i32,
    pub balance_worst_ptype: i32,
    pub balance_nbad: i32,
    pub signature_worst_abs: f64,
    pub signature_worst_tol: f64,
    /// input、evap、runoff 三项 `*_minus_water_R`。
    pub signature_worst_terms: [f64; 3],
    pub signature_worst_ipatch: i32,
    pub signature_worst_itrc: i32,
    pub signature_worst_ptype: i32,
    pub signature_nbad: i32,
    pub resid_worst_abs: f64,
    pub resid_hard_worst_abs: f64,
    pub resid_hard_worst_tol: f64,
    pub resid_hard_worst_ipatch: i32,
    pub resid_hard_worst_itrc: i32,
    pub resid_hard_worst_ptype: i32,
    pub resid_nbad: i32,
    pub resid_hard_nbad: i32,
}

impl Default for BalanceTracker {
    fn default() -> Self {
        Self {
            balance_worst_err: 0.0,
            balance_worst_diag: [0.0; N_BALANCE_DIAG],
            balance_worst_sbeg: [0.0; N_STORAGE_DIAG],
            balance_worst_send: [0.0; N_STORAGE_DIAG],
            balance_worst_sds: [0.0; N_STORAGE_DIAG],
            balance_worst_fcomp: [0.0; N_FLUX_DIAG],
            balance_worst_ipatch: 0,
            balance_worst_itrc: 0,
            balance_worst_ptype: -1,
            balance_nbad: 0,
            signature_worst_abs: 0.0,
            signature_worst_tol: 0.0,
            signature_worst_terms: [0.0; 3],
            signature_worst_ipatch: 0,
            signature_worst_itrc: 0,
            signature_worst_ptype: -1,
            signature_nbad: 0,
            resid_worst_abs: 0.0,
            resid_hard_worst_abs: 0.0,
            resid_hard_worst_tol: 0.0,
            resid_hard_worst_ipatch: 0,
            resid_hard_worst_itrc: 0,
            resid_hard_worst_ptype: -1,
            resid_nbad: 0,
            resid_hard_nbad: 0,
        }
    }
}

/// `tracer_balance_check`：返回 `xerr_tracer`（各示踪物 `|check_err|/deltim` 的最大值）。
///
/// 改写的状态：`trc_balance_err`（`step.balance_err`）、`a_water_precip`/`a_water_rnof`
/// （逐示踪物累加 `max(water_input,0)`、`max(water_rnof,0)`）与 `tracker`。
pub fn tracer_balance_check(
    set: &TracerSet,
    physics: TracerPhysics,
    state: &mut PatchTracerState,
    snapshot: &BalanceSnapshot,
    tracker: &mut BalanceTracker,
    input: &BalanceCheckInput<'_>,
) -> f64 {
    let mut xerr_tracer = 0.0_f64;
    if set.is_empty() {
        return xerr_tracer;
    }
    let lb = active_lower_bound(input.snl);
    let ptype = input.patchtype.unwrap_or(-1);
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let comp_end = storage_components(state, itrc, lb);
        let storage_end = sum_components(&comp_end);
        let snap = snapshot.tracers.get(itrc).cloned().unwrap_or_default();
        let comp_beg = snap.storage_comp;
        let mut comp_ds = [0.0; N_STORAGE_DIAG];
        for k in 0..N_STORAGE_DIAG {
            comp_ds[k] = comp_end[k] - comp_beg[k];
        }

        let r_init = tracer.init_water_ratio();
        let mut water_corrected_check = tracer.can_use_fixed_signature();
        let mut fixed_signature_step =
            water_corrected_check && !physics.fractionation_active(tracer);
        fixed_signature_step = fixed_signature_step && !input.runtime_forced[itrc];
        if input.flood_heterogeneous == Some(true) {
            fixed_signature_step = false;
            water_corrected_check = false;
        }

        let acc = &state.acc[itrc];
        let step_input = acc.precip - snap.precip;
        let step_evap = acc.evap - snap.evap;
        let step_rsur = acc.rsur - snap.rsur;
        let step_rsub = acc.rsub - snap.rsub;
        let step_qinfl = acc.qinfl - snap.qinfl;
        let step_qcharge = acc.qcharge - snap.qcharge;
        let mut step_rnof = 0.0;
        let mut step_output = step_evap;
        if !input.catch_lateral_flow {
            step_rnof = acc.rnof - snap.rnof;
            step_output += step_rnof;
        }
        let step_output_check = step_output;
        let step_vapor_exchange = acc.vapor_exchange - snap.vapor_exchange;
        let step_input_check = step_input + step_vapor_exchange;

        let reactive_source_sink = state.step[itrc].reactive_source_step;
        let numerical_source_sink = state.step[itrc].numerical_residual_step;
        let storage_beg = state.step[itrc].storage_beg;
        let ds = storage_end - storage_beg;
        let err = (ds - step_input_check) + step_output_check;
        state.step[itrc].balance_err = (err - reactive_source_sink) - numerical_source_sink;

        let water_err = input.water_err.unwrap_or(0.0);
        let numerical_water = state.step[itrc].numerical_water_step;
        let booked_host_water = if water_err * numerical_water > 0.0 {
            water_err
                .abs()
                .min(numerical_water.abs())
                .copysign(water_err)
        } else {
            0.0
        };
        let water_err_r = (water_err - booked_host_water) * r_init;
        let err_minus_water = err - water_err_r;
        let check_err = if input.water_err.is_some() && water_corrected_check {
            err_minus_water
        } else {
            err
        };
        let check_err = (check_err - reactive_source_sink) - numerical_source_sink;

        let water_ds = input.water_ds.unwrap_or(0.0);
        let water_input = input.water_input.unwrap_or(0.0);
        let water_output = input.water_output.unwrap_or(0.0);
        let water_evap = input.water_evap.unwrap_or(0.0);
        let water_rnof = input.water_rnof.unwrap_or(0.0);
        let acc = &mut state.acc[itrc];
        acc.water_precip += water_input.max(0.0);
        acc.water_rnof += water_rnof.max(0.0);

        let ds_minus_water_r = (-water_ds).mul_add(r_init, ds);
        let water_input_r = water_input * r_init;
        let in_minus_water_r = step_input - water_input_r;
        let out_minus_water_r = (-water_output).mul_add(r_init, step_output);
        let water_evap_r = water_evap * r_init;
        let evap_minus_water_r = step_evap - water_evap_r;
        let water_rnof_r = water_rnof * r_init;
        let rnof_minus_water_r = step_rnof - water_rnof_r;

        let balance_scale = storage_end
            .abs()
            .max(storage_beg.abs())
            .max(step_input_check.abs())
            .max(step_output_check.abs())
            .max(reactive_source_sink.abs())
            .max(numerical_source_sink.abs());
        let balance_tol = balance_scale.mul_add(BALANCE_REL_TOL, BALANCE_ABS_TOL);
        let resid_scale = 1.0_f64
            .max(storage_end.abs())
            .max(storage_beg.abs())
            .max(step_input_check.abs())
            .max(step_output_check.abs())
            .max(reactive_source_sink.abs());
        let resid_tol = RESID_ABS_TOL.max(RESID_REL_TOL * resid_scale);

        let mut signature_error = 0.0_f64;
        let mut signature_scale = 0.0_f64;
        if fixed_signature_step {
            if input.water_input.is_some() || input.water_evap.is_some() {
                signature_error =
                    signature_error.max((in_minus_water_r - evap_minus_water_r).abs());
                signature_scale = signature_scale
                    .max(step_input.abs())
                    .max(water_input_r.abs())
                    .max(step_evap.abs())
                    .max(water_evap_r.abs());
            }
            if !input.catch_lateral_flow && input.water_rnof.is_some() {
                signature_error = signature_error.max(rnof_minus_water_r.abs());
                signature_scale = signature_scale.max(step_rnof.abs()).max(water_rnof_r.abs());
            }
        }
        let signature_tol = signature_scale.mul_add(BALANCE_REL_TOL, BALANCE_ABS_TOL);
        let itrc_fortran = itrc as i32 + 1;
        if fixed_signature_step && signature_error > signature_tol {
            tracker.signature_nbad += 1;
            if signature_error > tracker.signature_worst_abs {
                tracker.signature_worst_abs = signature_error;
                tracker.signature_worst_tol = signature_tol;
                tracker.signature_worst_terms =
                    [in_minus_water_r, evap_minus_water_r, rnof_minus_water_r];
                tracker.signature_worst_ipatch = input.ipatch;
                tracker.signature_worst_itrc = itrc_fortran;
                tracker.signature_worst_ptype = ptype;
            }
        }

        if check_err.abs() > balance_tol {
            if check_err.abs() > tracker.balance_worst_err.abs() {
                tracker.balance_worst_err = check_err;
                tracker.balance_worst_diag = [
                    err,
                    ds,
                    step_input,
                    step_output,
                    water_err,
                    water_err_r,
                    err_minus_water,
                    water_ds,
                    water_input,
                    water_output,
                    ds_minus_water_r,
                    in_minus_water_r,
                    out_minus_water_r,
                    step_evap,
                    step_rnof,
                    water_evap,
                    water_rnof,
                    evap_minus_water_r,
                    rnof_minus_water_r,
                ];
                tracker.balance_worst_sbeg = comp_beg;
                tracker.balance_worst_send = comp_end;
                tracker.balance_worst_sds = comp_ds;
                tracker.balance_worst_fcomp = [
                    step_input,
                    step_evap,
                    step_rsur,
                    step_rsub,
                    step_rnof,
                    step_qinfl,
                    step_qcharge,
                ];
                tracker.balance_worst_ipatch = input.ipatch;
                tracker.balance_worst_itrc = itrc_fortran;
                tracker.balance_worst_ptype = ptype;
            }
            tracker.balance_nbad += 1;
        }

        if numerical_source_sink.abs() > RESID_WARN_FRAC * storage_end.abs().max(storage_beg.abs())
        {
            tracker.resid_nbad += 1;
            tracker.resid_worst_abs = tracker.resid_worst_abs.max(numerical_source_sink.abs());
        }
        if numerical_source_sink.abs() > resid_tol {
            tracker.resid_hard_nbad += 1;
            if numerical_source_sink.abs() > tracker.resid_hard_worst_abs {
                tracker.resid_hard_worst_abs = numerical_source_sink.abs();
                tracker.resid_hard_worst_tol = resid_tol;
                tracker.resid_hard_worst_ipatch = input.ipatch;
                tracker.resid_hard_worst_itrc = itrc_fortran;
                tracker.resid_hard_worst_ptype = ptype;
            }
        }

        xerr_tracer = if input.deltim > 0.0 {
            xerr_tracer.max(check_err.abs() / input.deltim)
        } else {
            xerr_tracer.max(check_err.abs())
        };
    }
    xerr_tracer
}

/// [`BalanceTracker::report`] 的结果：要打印的行（与上游 `WRITE(*,...)` 逐字相同）与
/// 是否 `CoLM_stop`。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BalanceReport {
    pub lines: Vec<String>,
    /// `Some(msg)`：上游此处 `CALL CoLM_stop(msg)`。
    pub abort: Option<String>,
}

impl BalanceTracker {
    /// `tracer_balance_report`（单进程，`print_me = .true.`，`owner = 0`）。
    ///
    /// `balance_abort_nbad`/`resid_abort_nbad` 是 `DEF_TRACER_BALANCE_ABORT_NBAD`、
    /// `DEF_TRACER_RESID_ABORT_NBAD`。上游在第一处 `CoLM_stop` 就退出，后面的行不再打印、
    /// 计数器也不清零；这里遇到中止即返回（不清零）。`ntracers <= 0` 时什么都不做。
    pub fn report(
        &mut self,
        ntracers: usize,
        balance_abort_nbad: i32,
        resid_abort_nbad: i32,
    ) -> BalanceReport {
        let mut out = BalanceReport::default();
        if ntracers == 0 {
            return out;
        }
        let owner = 0;
        if self.balance_nbad > 0 {
            let d = &self.balance_worst_diag;
            let mut line = format!(
                "TRC_BAL step report: nbad_entries={} worst_abs_err={} @ipatch={} itrc={} ptype={} owner={}",
                fortran_i(self.balance_nbad, 8),
                fortran_e12_5(self.balance_worst_err.abs()),
                fortran_i(self.balance_worst_ipatch, 8),
                fortran_i(self.balance_worst_itrc, 4),
                fortran_i(self.balance_worst_ptype, 3),
                fortran_i(owner, 4),
            );
            let labels = [
                " err=",
                " dS=",
                " in=",
                " out=",
                " water_err=",
                " water_err_R=",
                " err_minus_water=",
                " water_dS=",
                " water_input=",
                " water_output=",
                " dS_minus_water_R=",
                " in_minus_water_R=",
                " out_minus_water_R=",
                " step_evap=",
                " step_rnof=",
                " water_evap=",
                " water_rnof=",
                " evap_minus_water_R=",
                " rnof_minus_water_R=",
            ];
            for (label, value) in labels.iter().zip(d.iter()) {
                line.push_str(label);
                line.push_str(&fortran_e12_5(*value));
            }
            out.lines.push(line);
            let head = |name: &str| {
                format!(
                    "{name} @ipatch={} itrc={} ptype={}",
                    fortran_i(self.balance_worst_ipatch, 8),
                    fortran_i(self.balance_worst_itrc, 4),
                    fortran_i(self.balance_worst_ptype, 3),
                )
            };
            let storage_labels = [
                "ldew",
                "soil_liq",
                "soil_ice",
                "wa",
                "wdsrf",
                "wetwat",
                "scv",
                "waterstorage",
                "leaf_iso",
                "surface_residue",
                "subsurface_residue",
                "solid",
            ];
            let mut line = head("TRC_BAL_DCOMP");
            for (label, value) in storage_labels.iter().zip(self.balance_worst_sds.iter()) {
                line.push_str(&format!(" d_{label}={}", fortran_e12_5(*value)));
            }
            out.lines.push(line);
            let mut line = head("TRC_BAL_FCOMP");
            for (label, value) in ["precip", "evap", "rsur", "rsub", "rnof", "qinfl", "qcharge"]
                .iter()
                .zip(self.balance_worst_fcomp.iter())
            {
                line.push_str(&format!(" {label}={}", fortran_e12_5(*value)));
            }
            out.lines.push(line);
            let mut line = head("TRC_BAL_SEND");
            for (label, value) in storage_labels.iter().zip(self.balance_worst_send.iter()) {
                line.push_str(&format!(" {label}={}", fortran_e12_5(*value)));
            }
            out.lines.push(line);
        }
        if self.resid_nbad > 0 {
            out.lines.push(format!(
                "TRC_BAL residual note (excluded from tol check): n_entries={} worst_abs={}",
                fortran_i(self.resid_nbad, 8),
                fortran_e12_5(self.resid_worst_abs),
            ));
        }
        if self.resid_hard_nbad > resid_abort_nbad {
            out.lines.push(format!(
                "TRC_BAL residual hard failure: n_entries={} worst_abs={} tol={} @ipatch={} itrc={} ptype={} owner={}",
                fortran_i(self.resid_hard_nbad, 8),
                fortran_e12_5(self.resid_hard_worst_abs),
                fortran_e12_5(self.resid_hard_worst_tol),
                fortran_i(self.resid_hard_worst_ipatch, 8),
                fortran_i(self.resid_hard_worst_itrc, 4),
                fortran_i(self.resid_hard_worst_ptype, 3),
                fortran_i(owner, 4),
            ));
            out.abort = Some(
                "TRC_BAL hard failure: numerical residual exceeds independent threshold".to_owned(),
            );
            return out;
        }
        if self.signature_nbad > 0 {
            out.lines.push(format!(
                "TRC_SIG step report: nbad_entries={} worst_abs={} tol={} @ipatch={} itrc={} ptype={} owner={} input_minus_water_R={} evap_minus_water_R={} rnof_minus_water_R={}",
                fortran_i(self.signature_nbad, 8),
                fortran_e12_5(self.signature_worst_abs),
                fortran_e12_5(self.signature_worst_tol),
                fortran_i(self.signature_worst_ipatch, 8),
                fortran_i(self.signature_worst_itrc, 4),
                fortran_i(self.signature_worst_ptype, 3),
                fortran_i(owner, 4),
                fortran_e12_5(self.signature_worst_terms[0]),
                fortran_e12_5(self.signature_worst_terms[1]),
                fortran_e12_5(self.signature_worst_terms[2]),
            ));
        }
        if self.signature_nbad > balance_abort_nbad {
            out.abort = Some(
                "TRC_SIG hard failure: fixed-signature flux differs from water flux times R_init"
                    .to_owned(),
            );
            return out;
        }
        if self.balance_nbad > balance_abort_nbad {
            out.abort = Some(
                "TRC_BAL hard failure: tracer balance error exceeds aggregate threshold".to_owned(),
            );
            return out;
        }
        *self = Self::default();
        out
    }
}

/// gfortran 的 `Iw`：右对齐，放不下时全是 `*`。
pub fn fortran_i(value: i32, width: usize) -> String {
    let text = value.to_string();
    if text.len() > width {
        "*".repeat(width)
    } else {
        format!("{text:>width$}")
    }
}

/// gfortran 的 `E12.5`：`0.ddddd` 尾数、`E±dd` 指数（|指数| > 99 时去掉 `E` 写三位），
/// 右对齐到 12 列；NaN/无穷右对齐写 `NaN`/`Infinity`。
pub fn fortran_e12_5(value: f64) -> String {
    const WIDTH: usize = 12;
    if value.is_nan() {
        return format!("{:>WIDTH$}", "NaN");
    }
    if value.is_infinite() {
        let text = if value < 0.0 { "-Infinity" } else { "Infinity" };
        return format!("{text:>WIDTH$}");
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let (digits, exponent) = if value == 0.0 {
        ("00000".to_owned(), 0)
    } else {
        let formatted = format!("{:.4e}", value.abs());
        let (mantissa, exp) = formatted
            .split_once('e')
            .expect("Rust scientific formatting always has an exponent");
        let exp: i32 = exp.parse().expect("integer exponent");
        (mantissa.replace('.', ""), exp + 1)
    };
    let exp_text = if exponent.abs() > 99 {
        format!(
            "{}{:03}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        )
    } else {
        format!(
            "E{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        )
    };
    let body = format!("{sign}0.{digits}{exp_text}");
    if body.len() > WIDTH {
        "*".repeat(WIDTH)
    } else {
        format!("{body:>WIDTH$}")
    }
}

#[cfg(test)]
#[path = "conservation_tests.rs"]
mod tests;
