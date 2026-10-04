//! WATER 之后的土壤/雪/地表/含水层示踪物（`MOD_Tracer_SoilWater`）。
//!
//! 示踪物只记账：读宿主本步 WATER 前后的水量与 WATER_VSF 的诊断量，按同样的路径搬运。
//! * 地表水：混合池（穿透雨 + 旧积水 + 雪底流出 + 灌溉），蒸发、产流、入渗按池比值分；
//! * 土壤层：按 `qlayer` 逐界面搬运，`water_shadow` 追踪示踪物眼中的水量；
//! * 含水层：按 `qcharge`；同位素带固定的参考水/参考质量（相对量可为负，表示亏欠）；
//! * 根系吸水、基流的来源按 VSF 的级联/交换诊断（`etroot_actual`、`rsub_source_*`）；
//! * WATER 内部的冻融按冰量变化重建；剩下的水量失配按层当前浓度记入数值残差。
//!
//! 静态湿地走 [`tracer_wetland`]（`soil_water/wetland.rs`）。
//!
//! # 尚未移植
//! 分馏（`tracer_fractionation_active`）与同位素的土壤/雪层扩散属于下一阶段：只要某个
//! 输运示踪物会走到这些分支，入口就返回错误、什么都不改。非同位素示踪物（溶质）在
//! `DEF_TRACER_SOIL_VAPOR_DIFFUSION` 打开时的气相扩散**已移植**——上游对未登记分馏物理
//! 的示踪物取 `diff_ratio = alpha = 1`、液相自扩散系数 0。
//!
//! # 层下标
//! 雪+土的数组是 `[f64; SOISNO_LAYERS]`（Fortran `-4..=10`，用 [`soisno_slot`] 取，只读
//! `snl+1..=nl_soil`），对应上游的 `x(snl+1:nl_soil)` 实参；纯土壤层的数组是
//! `[f64; SOIL_LAYERS]`（下标 `j-1`），对应 `x(1:nl_soil)`；`qlayer(0:nl_soil)` 是
//! `[f64; SOIL_LAYERS + 1]`（下标 `j`）；雪层的 `x(snl+1:0)` 是 `[f64; MAX_SNOW_LAYERS]`，
//! 也用 [`soisno_slot`] 取。
//!
//! # GIMPLE 收缩形状（`MOD_Tracer_SoilWater.F90.273t.optimized`）
//! 其余全是逐条舍入（Fortran 自左向右，`sum` 从 0 顺序累加）。
//! * 根系：`xylem_tracer_total` 在逐层循环里 `FMA(etroot_actual, ratio_layer, x)`，含水层与积水
//!   两项（`etroot_aquifer*aquifer_ratio`、`surface_et_water*surface_et_ratio`）**不**融合；
//!   回流比值 `(FMA(excess, R_dep, rgt*(rrw-excess)/rgw))/rrw`；回流 `trc += (-e)*R` 是
//!   `.FNMA`、积水回流 `.FMA`，`root_return_tracer_total` 同形；上限检查
//!   `FMA(excess, R_dep, rgt) + max(1e-9|R|, 1e-12)`。
//! * 雪顶层：霜 `trc_wice = FMA(dt, max(q,0)*R, trc_wice)`，`a_trc_precip` 同形，
//!   `a_water_precip += max(q,0)*dt` 与 `water_ice_pool` 共用这次舍入；露的示踪物
//!   `dt*(max(q,0)*R)` 先舍入，`a_water_precip = FMA(dt, max(q,0), a)`；液池
//!   `FMA(dt, FMA(fsno, pg_rain, max(qsdew,0)), wliq_bef)`。
//! * 地表：`surface_base_balance = FMA(dt, qinfl, max(wdsrf,0)+max(rsur,0)*dt) - flood`；
//!   `gwat_evap = max(FMS(dt, max(qseva,0), top_soil_evap) - …, 0)`；分开时
//!   `trc_pool = FMA(trc_pg_rain, 1-fsno, trc_wdsrf+trc_gwat_snow)`；
//!   `pending = max(FNMA(dt, late_ratio*max(qinfl,0), trc_pool), 0)`；负入渗的
//!   `a_trc_qinfl = FMA(dt, ratio_layer(1)*qinfl, a)`，正入渗 `dt*(late_ratio*qinfl)` 先舍入
//!   （同时进 `a_trc_qinfl` 与第 1 层）；`water_shadow(1) = FMA(dt, qinfl, ·)`。
//! * 界面：`water_shadow(j) = FNMA(dt, qlayer, ·)`、`water_shadow(j+1) = FMA(dt, qlayer, ·)`，
//!   示踪物通量 `dt*(q*R)` 先舍入；`qcharge` 两支的 `water_shadow(nl)` 都是 `FNMA(dt, q, ·)`。
//! * 晚到积水的露 `pending = FMA(dew, R, pending)`，`a_trc_precip` 同形；第 1 层露/霜的
//!   `a_water_precip` 与 `water_shadow(1)` 是 `FMA(dt, q, ·)`。
//! * 孤儿质量 `FNMA(actual_water, aquifer_ratio, actual_mass)`；灌溉储水的最后平衡用
//!   `max(FNMA(dt, max(irrig,0), waterstorage), 0)`。
//! * `reconcile_internal_soil_flow`/`move_dissolved_face` 见 `soil_water/reconcile.rs`；
//!   `release_leaf_iso_storage` 见 [`release_leaf_iso_storage`]。

// 夹紧保留上游 `MIN(MAX(·))` 的次序；`x = y + x` 照抄 GIMPLE 的操作数次序（IEEE 加法可交换，
// 与 `x += y` 逐位相同）。
#![allow(clippy::manual_clamp, clippy::assign_op_pattern)]

mod common;
mod reconcile;
mod wetland;

use anyhow::{bail, ensure, Result};

use super::{
    soisno_slot, EvapKind, PatchTracerState, TracerPhysics, TracerSet, MAX_SNOW_LAYERS,
    SOIL_LAYERS, SOISNO_LAYERS, TRC_TINY, TRC_WATER_MIN_FOR_RATIO,
};
pub use common::{
    aquifer_actual_mass, aquifer_actual_water, aquifer_isotope_ratio, aquifer_isotope_state_valid,
    check_isotope_aquifer, exhaust_surface_phase, release_leaf_iso_storage,
};
use common::{
    snow_column, snow_vapor_diffusion, soil_diffusion, soil_slot, EvapKinetics, FrostShape,
    SnowColumn, TracerCtx, DEFAULT_LAYER_TEMP_K,
};
pub use wetland::{tracer_wetland, WetlandInput};

/// `spval`。
const SPVAL: f64 = -1.0e36;

/// 本模块读的 namelist 开关（`MOD_Namelist`）。
///
/// `DEF_TRACER_SOIL_KINETIC` 与 `DEF_TRACER_SNOWMELT_EQUILIBRATION` 只在分馏生效时起作用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilWaterOptions {
    /// `DEF_TRACER_SUBL_SKIN_MM`（默认 5）：升华的表层交换厚度，总以 `skin_mass=` 传入
    /// `tracer_atmospheric_tracer_loss`，不分馏时也改变舍入路径。
    pub subl_skin_mm: f64,
    /// `DEF_TRACER_SOIL_DIFFUSION`（默认开）。
    pub soil_diffusion: bool,
    /// `DEF_TRACER_SOIL_VAPOR_DIFFUSION`（默认开）。
    pub soil_vapor_diffusion: bool,
    /// `DEF_USE_CoLMDEBUG`：标准土壤支里 `wetwat` 变化时打印警告。
    pub colm_debug: bool,
    /// `DEF_TRACER_SOIL_KINETIC == 'RESISTANCE'`（默认）；`'EXPONENT'` 时为假。
    pub soil_kinetic_resistance: bool,
    /// `DEF_TRACER_SNOWMELT_EQUILIBRATION`（默认 0）。
    pub snowmelt_equilibration: f64,
}

impl Default for SoilWaterOptions {
    fn default() -> Self {
        Self {
            subl_skin_mm: 5.0,
            soil_diffusion: true,
            soil_vapor_diffusion: true,
            colm_debug: false,
            soil_kinetic_resistance: true,
            snowmelt_equilibration: 0.0,
        }
    }
}

/// `tracer_soil_water` 的实参（CoLMMAIN.F90:1299-1338）。可选实参用 `Option`，`None` 即
/// 上游的 `.not. present(·)`。
#[derive(Debug, Clone, Copy)]
pub struct SoilWaterInput<'a> {
    /// `ipatch`（只用于报错信息）。
    pub ipatch: usize,
    pub deltim: f64,
    pub snl: i32,
    /// `qlayer(0:nl_soil)` [mm/s]，下标 `j`。
    pub qlayer: &'a [f64; SOIL_LAYERS + 1],
    /// `qinfl` [mm/s]。
    pub qinfl: f64,
    /// `qcharge_trc`（CoLMMAIN.F90:1272，见 [`qcharge_trc`]）。
    pub qcharge: f64,
    pub rsur: f64,
    pub rsub: f64,
    /// `qseva`/`qsdew`/`qsubl`/`qfros`（不带后缀的总量）。
    pub qseva_in: f64,
    pub qsdew_in: f64,
    pub qsubl_in: f64,
    pub qfros_in: f64,
    pub qseva_soil: f64,
    pub qsdew_soil: f64,
    pub qsubl_soil: f64,
    pub qfros_soil: f64,
    pub qseva_snow: f64,
    pub qsdew_snow: f64,
    pub qsubl_snow: f64,
    pub qfros_snow: f64,
    pub sm: f64,
    pub fsno: f64,
    /// `DEF_SPLIT_SOILSNOW`。
    pub split_soilsnow: bool,
    /// WATER 之后的 `wliq_soisno(snl+1:nl_soil)`/`wice_soisno`。
    pub wliq_soisno: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno: &'a [f64; SOISNO_LAYERS],
    /// WATER 之前（THERMAL 之后）的 `wliq_soisno_old_trc`/`wice_soisno_old_trc`。
    pub wliq_soisno_bef: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno_bef: &'a [f64; SOISNO_LAYERS],
    pub wa: f64,
    pub wa_bef: f64,
    pub wdsrf: f64,
    pub wdsrf_bef: f64,
    pub wetwat: f64,
    pub wetwat_bef: f64,
    pub pg_rain: f64,
    pub pg_snow: f64,
    /// `wblc_ice_sink_trc(1:nl_soil)` [kg/m²]（MOD_SoilSnowHydrology.F90:1283-1294）。
    pub wblc_ice_sink: &'a [f64; SOIL_LAYERS],
    /// `etroot_actual_trc(1:nl_soil)` [mm]（MOD_Hydro_SoilWater.F90:305-425）。
    pub etroot_actual: &'a [f64; SOIL_LAYERS],
    /// `etroot_aquifer_trc` [mm]。
    pub etroot_aquifer: f64,
    /// `qflx_irrig_drip + qflx_irrig_flood + qflx_irrig_paddy` [mm/s]。
    pub qflx_irrig_ground: f64,
    /// `waterstorage_trc_ground`（CoLMMAIN 总是给出）。
    pub waterstorage_patch: Option<f64>,
    /// `imperv_evap_wdsrf_trc`/`imperv_evap_soil_trc`/`imperv_subl_soil_trc` [mm]。
    pub imperv_evap_wdsrf: Option<f64>,
    pub imperv_evap_soil: Option<f64>,
    pub imperv_subl_soil: Option<f64>,
    /// `snow_qout_layer_trc(snl+1:0)` [mm]。
    pub snow_qout_layer: Option<&'a [f64; MAX_SNOW_LAYERS]>,
    /// `qgtop_trc` [mm/s]。
    pub qgtop_solver: Option<f64>,
    /// `tleaf`：植物回流的凝结比值与叶片 NSS。
    pub tleaf: Option<f64>,
    /// `t_soisno(snl+1:nl_soil)`：`layer_temp`（溶质气相扩散用）。
    pub t_soisno: Option<&'a [f64; SOISNO_LAYERS]>,
    /// `forc_q`：分馏的近地面相对湿度。
    pub forc_q: Option<f64>,
    /// `forc_psrf`：气相扩散与分馏。
    pub forc_psrf: Option<f64>,
    /// `lai`：叶面积消失时释放叶片同位素异常。
    pub lai: Option<f64>,
    /// `rst`、`raw_trc`、`rss`：叶片 NSS 与裸土动力分馏的阻力。
    pub rst: Option<f64>,
    pub ra: Option<f64>,
    pub rss: Option<f64>,
    /// `dz_soisno(1:nl_soil)` [m]、`porsl(1:nl_soil)`：土壤扩散。
    pub dz_soi: Option<&'a [f64; SOIL_LAYERS]>,
    pub porsl: Option<&'a [f64; SOIL_LAYERS]>,
    /// `dz_soisno(snl+1:0)` [m]：雪层气相扩散。
    pub dz_sno: Option<&'a [f64; MAX_SNOW_LAYERS]>,
    /// `flood_input_tracer`（`GridRiverLakeFlow`，逐示踪物）与 `qinfl_fld*deltim`。
    pub flood_tracer_input: Option<&'a [f64]>,
    pub flood_infil_water: Option<f64>,
    /// `etroot_surface_trc`、`dew_overflow_trc`、`frost_displaced_trc`、`late_runoff_trc` [mm]。
    pub etroot_surface: Option<f64>,
    pub dew_overflow: Option<f64>,
    pub frost_displaced: Option<f64>,
    pub late_surface_runoff: Option<f64>,
    /// `rsub_source_layer_trc(1:nl_soil)`、`rsub_source_surface_trc`、`rsub_source_aquifer_trc` [mm]。
    pub rsub_source_layer: Option<&'a [f64; SOIL_LAYERS]>,
    pub rsub_source_surface: Option<f64>,
    pub rsub_source_aquifer: Option<f64>,
    /// `permeable_soil_trc(1:nl_soil)`（`WATER_VSF` 的 `permeable_soil_out`）。
    pub permeable_soil: Option<&'a [bool; SOIL_LAYERS]>,
    /// 逐示踪物的 `tracer_forcing_precip_value(itrc, ipatch)`。
    pub precip_ratio: &'a [f64],
    /// 逐示踪物的 `tracer_forcing_vapor_value(itrc, ipatch)`。
    pub vapor_ratio: &'a [f64],
    /// 逐示踪物的 `tracer_forcing_has_vapor(itrc, ipatch)`（`None` 即都没有）。
    pub has_vapor: Option<&'a [bool]>,
}

/// CoLMMAIN.F90:1272 的 `qcharge_trc`：由含水层水量变化反推的补给通量 [mm/s]。
///
/// GIMPLE：`(((wa - wa_old) + etroot_aquifer) + max(rsub_source_aquifer,0)) / max(dt, trc_tiny)`。
pub fn qcharge_trc(
    wa: f64,
    wa_old: f64,
    etroot_aquifer: f64,
    rsub_source_aquifer: f64,
    deltim: f64,
) -> f64 {
    (((wa - wa_old) + etroot_aquifer) + rsub_source_aquifer.max(0.0)) / deltim.max(TRC_TINY)
}

/// 内部函数 `current_liq_ratio`：按示踪物眼中的水量求当前液相比值。
fn current_liq_ratio(
    water_shadow: &[f64; SOIL_LAYERS],
    trc_wliq: &[f64; SOISNO_LAYERS],
    ratio_layer: &[f64; SOIL_LAYERS],
    nonvolatile: bool,
    j: i32,
) -> f64 {
    let k = soil_slot(j);
    if water_shadow[k] > TRC_WATER_MIN_FOR_RATIO {
        trc_wliq[soisno_slot(j)] / water_shadow[k]
    } else if nonvolatile {
        0.0
    } else {
        ratio_layer[k]
    }
}

/// `tracer_soil_water`（标准土壤支：非湿地或动态湿地）。
pub fn tracer_soil_water(
    set: &TracerSet,
    state: &mut PatchTracerState,
    physics: TracerPhysics,
    options: &SoilWaterOptions,
    input: &SoilWaterInput<'_>,
) -> Result<()> {
    if set.is_empty() {
        return Ok(());
    }
    ensure!(
        (-(MAX_SNOW_LAYERS as i32)..=0).contains(&input.snl),
        "tracer_soil_water: snl={} outside -{}..=0",
        input.snl,
        MAX_SNOW_LAYERS
    );
    ensure!(
        input.precip_ratio.len() >= set.len() && input.vapor_ratio.len() >= set.len(),
        "tracer_soil_water: forcing ratios do not cover {} tracers",
        set.len()
    );
    if let Some(flood) = input.flood_tracer_input {
        ensure!(
            flood.len() >= set.len(),
            "tracer_soil_water: flood tracer input does not cover {} tracers",
            set.len()
        );
    }
    let soil_diffusion_block = (options.soil_diffusion || options.soil_vapor_diffusion)
        && input.dz_soi.is_some()
        && input.porsl.is_some();
    let snow_vapor_block = options.soil_vapor_diffusion
        && input.snl < 0
        && input.dz_sno.is_some()
        && input.forc_psrf.is_some();

    let dt = input.deltim;
    let nl = SOIL_LAYERS as i32;
    let lb = input.snl + 1;
    let s = soisno_slot;
    let layer_temp = |j: i32| {
        input
            .t_soisno
            .map_or(DEFAULT_LAYER_TEMP_K, |t| t[soisno_slot(j)])
    };
    let wliq_bef = input.wliq_soisno_bef;
    let wice_bef = input.wice_soisno_bef;
    let wliq_bef_soil = &wliq_bef[s(1)..];

    let flood_water = input.flood_infil_water.map_or(0.0, |w| w.max(0.0));
    let (surface_et_water, surface_root_return) = match input.etroot_surface {
        Some(e) => (e.max(0.0), (-e).max(0.0)),
        None => (0.0, 0.0),
    };
    let dew_surface_water = input.dew_overflow.map_or(0.0, |w| w.max(0.0));
    let frost_surface_water = input.frost_displaced.map_or(0.0, |w| w.max(0.0));
    let late_surface_water = dew_surface_water + frost_surface_water;
    let late_runoff_water = input.late_surface_runoff.map_or(0.0, |w| w.max(0.0));
    let rsur_water = dt * input.rsur.max(0.0);
    let early_runoff_water = rsur_water - late_runoff_water;
    ensure!(
        early_runoff_water >= -1.0e-9,
        "late runoff exceeds total surface runoff"
    );
    let early_runoff_water = early_runoff_water.max(0.0);
    let wdsrf_pos = input.wdsrf.max(0.0);
    let top_infil_water = dt * input.qinfl.max(0.0);
    let flood_destination_water =
        (((late_runoff_water + wdsrf_pos) - late_surface_water) + top_infil_water).max(0.0);
    let resolved_rsub = match (
        input.rsub_source_layer,
        input.rsub_source_surface,
        input.rsub_source_aquifer,
    ) {
        (Some(layer), Some(surface), Some(aquifer)) => {
            let mut sum = 0.0;
            for &x in layer {
                sum = x + sum;
            }
            (surface + sum) + aquifer > TRC_TINY
        }
        _ => false,
    };
    if flood_water > 0.0 && input.flood_tracer_input.is_none() {
        bail!("grid flood tracer: water input has no tracer composition");
    }
    let qcharge_eff = if !input.qcharge.is_finite() || input.qcharge.abs() > 0.5 * SPVAL.abs() {
        0.0
    } else {
        input.qcharge
    };
    let etroot_aquifer = input.etroot_aquifer;
    let wa_after_root = input.wa_bef - etroot_aquifer;
    let dt_floor = dt.max(TRC_TINY);

    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let ctx = TracerCtx::new(
            tracer,
            itrc,
            physics,
            input.vapor_ratio[itrc],
            options.subl_skin_mm,
            input.forc_q,
            input.forc_psrf,
            EvapKinetics::Soil {
                resistance: options.soil_kinetic_resistance,
                ra: input.ra,
                rss: input.rss,
            },
        );
        let nonvolatile = ctx.nonvolatile;
        let is_isotope = tracer.is_isotope();
        let mut p = state.pools[itrc].clone();
        let patch_ref_water = state.aquifer_ref_water;
        let (aquifer_ref_water, aquifer_ref_mass) = if is_isotope {
            (patch_ref_water, p.aquifer_ref_mass)
        } else {
            (0.0, 0.0)
        };
        let check = |isotope_mass: f64, water: f64, stage: &str| {
            check_isotope_aquifer(
                tracer,
                itrc,
                input.ipatch,
                patch_ref_water,
                aquifer_ref_mass,
                water,
                isotope_mass,
                stage,
            )
        };
        let r_precip = input.precip_ratio[itrc];
        let mut soil_resid_trc = 0.0;
        let mut soil_resid_water = 0.0;
        let mut water_shadow = [0.0; SOIL_LAYERS];
        for j in 1..=nl {
            water_shadow[soil_slot(j)] = wliq_bef[s(j)];
        }

        if nonvolatile && input.wa_bef > TRC_WATER_MIN_FOR_RATIO && p.subsurface_residue > TRC_TINY
        {
            p.wa = p.subsurface_residue + p.wa;
            p.subsurface_residue = 0.0;
        }
        tracer.equilibrate_dissolved(input.wa_bef, &mut p.wa, &mut p.subsurface_solid);
        for j in lb..=nl {
            tracer.equilibrate_dissolved(
                wliq_bef[s(j)].max(0.0),
                &mut p.wliq_soisno[s(j)],
                &mut p.solid_soisno[s(j)],
            );
        }
        if let Some(lai) = input.lai {
            if lai <= TRC_TINY || p.leaf_water_moles <= TRC_TINY {
                release_leaf_iso_storage(
                    tracer,
                    &mut p,
                    patch_ref_water,
                    wliq_bef_soil,
                    input.wa_bef,
                );
            }
        }
        check(p.wa, input.wa_bef, "soil start")?;

        // 0. WATER 之前各层的比值；干层的回退比值取最深的有水层，不借降水。
        let mut source_fallback_ratio = tracer.init_water_ratio();
        for j in (1..=nl).rev() {
            if wliq_bef[s(j)] > TRC_WATER_MIN_FOR_RATIO {
                source_fallback_ratio = p.wliq_soisno[s(j)] / wliq_bef[s(j)];
                break;
            }
        }
        let mut ratio_layer = [0.0; SOIL_LAYERS];
        for j in 1..=nl {
            ratio_layer[soil_slot(j)] = if wliq_bef[s(j)] > TRC_WATER_MIN_FOR_RATIO {
                p.wliq_soisno[s(j)] / wliq_bef[s(j)]
            } else if nonvolatile {
                0.0
            } else {
                source_fallback_ratio
            };
        }

        // 0a. 根系吸水（级联后的实际量）与植物水力回流。
        let mut aquifer_ratio = source_fallback_ratio;
        if aquifer_actual_water(input.wa_bef, aquifer_ref_water).abs() > TRC_WATER_MIN_FOR_RATIO {
            check(p.wa, input.wa_bef, "root uptake")?;
            aquifer_ratio = aquifer_isotope_ratio(
                input.wa_bef,
                p.wa,
                aquifer_ref_water,
                aquifer_ref_mass,
                source_fallback_ratio,
            );
        } else if nonvolatile {
            aquifer_ratio = 0.0;
        }
        let mut transp_water_total = etroot_aquifer.max(0.0);
        let mut xylem_tracer_total = transp_water_total * aquifer_ratio;
        let mut surface_et_ratio = source_fallback_ratio;
        if input.wdsrf_bef > TRC_WATER_MIN_FOR_RATIO {
            surface_et_ratio = p.wdsrf / input.wdsrf_bef;
        } else if nonvolatile {
            surface_et_ratio = 0.0;
        }
        let surface_et_tracer = surface_et_water * surface_et_ratio;
        transp_water_total += surface_et_water;
        xylem_tracer_total += surface_et_tracer;
        for j in 1..=nl {
            let k = soil_slot(j);
            let e = input.etroot_actual[k];
            if e > TRC_TINY {
                transp_water_total = e + transp_water_total;
                xylem_tracer_total = e.mul_add(ratio_layer[k], xylem_tracer_total);
            }
        }
        let mut xylem_ratio = source_fallback_ratio;
        if transp_water_total > TRC_TINY {
            xylem_ratio = xylem_tracer_total / transp_water_total;
        }
        let root_gross_water = transp_water_total;
        let mut negative_uptake = 0.0;
        for &e in input.etroot_actual {
            negative_uptake = e.min(0.0) + negative_uptake;
        }
        let root_return_water =
            ((-etroot_aquifer).max(0.0) - negative_uptake) + surface_root_return;
        let root_return_excess = (root_return_water - root_gross_water).max(0.0);
        // `:452`：净蒸腾水量（总吸水减植物水力回流）。
        let transp_water_total = (root_gross_water - root_return_water).max(0.0);
        let mut root_gross_tracer = 0.0;
        let mut transp_ratio = xylem_ratio;
        // 分馏时根系取出的示踪物先攒在 `transp_source_tracer_total`，叶片 NSS 定出蒸腾比值后
        // 再记蒸发损失，差额进叶片同位素储量（`:457`、`:658-670`）。
        let mut transp_source_tracer_total = 0.0;
        let transp_frac_active = transp_water_total > TRC_TINY && ctx.active;

        for j in 1..=nl {
            let k = soil_slot(j);
            let e = input.etroot_actual[k];
            if e > TRC_TINY && wliq_bef[s(j)] > TRC_TINY {
                if !nonvolatile {
                    let flux = (e * ratio_layer[k]).min(p.wliq_soisno[s(j)].max(0.0));
                    p.wliq_soisno[s(j)] -= flux;
                    root_gross_tracer += flux;
                    if transp_frac_active {
                        transp_source_tracer_total += flux;
                    } else {
                        state.book_evap_loss(itrc, flux, e, EvapKind::Transpiration);
                        state.acc[itrc].transp_src += flux;
                    }
                }
                water_shadow[k] -= e;
            }
        }
        if etroot_aquifer > TRC_TINY && !nonvolatile {
            let mut flux = etroot_aquifer * aquifer_ratio;
            if aquifer_actual_water(input.wa_bef, aquifer_ref_water) > TRC_TINY {
                let actual_mass = aquifer_actual_mass(p.wa, aquifer_ref_mass);
                flux = flux.min(actual_mass.max(0.0));
            }
            p.wa -= flux;
            root_gross_tracer += flux;
            if transp_frac_active {
                transp_source_tracer_total += flux;
            } else {
                state.book_evap_loss(itrc, flux, etroot_aquifer, EvapKind::Transpiration);
                state.acc[itrc].transp_src += flux;
            }
        }
        if surface_et_water > TRC_TINY && !nonvolatile {
            let flux = surface_et_tracer.min(p.wdsrf.max(0.0));
            p.wdsrf -= flux;
            root_gross_tracer = flux + root_gross_tracer;
            if transp_frac_active {
                transp_source_tracer_total += flux;
            } else {
                state.book_evap_loss(itrc, flux, surface_et_water, EvapKind::Transpiration);
                state.acc[itrc].transp_src += flux;
            }
        }

        let mut return_ratio = xylem_ratio;
        let mut excess_ratio = 0.0;
        if root_return_water > TRC_TINY && !nonvolatile {
            excess_ratio =
                ctx.deposition_ratio_for(input.tleaf.unwrap_or_else(|| layer_temp(1)), false);
            return_ratio = if root_gross_water > TRC_TINY && root_return_excess <= 0.0 {
                root_gross_tracer / root_gross_water
            } else if root_gross_water > TRC_TINY {
                root_return_excess.mul_add(
                    excess_ratio,
                    root_gross_tracer * (root_return_water - root_return_excess) / root_gross_water,
                ) / root_return_water
            } else {
                excess_ratio
            };
            xylem_ratio = return_ratio;
            transp_ratio = return_ratio;
        }
        // 叶片非稳态（NSS）蒸腾比值（`:539-563`）。
        if let (true, Some(tleaf), Some(forc_q), Some(forc_psrf), Some(lai), Some(rst)) = (
            transp_frac_active,
            input.tleaf,
            input.forc_q,
            input.forc_psrf,
            input.lai,
            input.rst,
        ) {
            let ra = input.ra.map_or(0.0, |ra| ra.max(0.0));
            let has_vapor = input.has_vapor.is_some_and(|flags| flags[itrc]);
            let vapor = if has_vapor {
                ctx.r_atm
            } else {
                xylem_ratio / physics.alpha_liq_vap(tracer, tleaf).max(TRC_TINY)
            };
            let relhum_leaf = crate::tracer::frac::surface_relhum(forc_q, forc_psrf, tleaf, false);
            let out = physics.transpiration_nss_ratio(
                tracer,
                &crate::tracer::frac::NssInput {
                    source_ratio: xylem_ratio,
                    vapor_ratio: vapor,
                    temp_k: tleaf,
                    relhum: relhum_leaf,
                    psrf: forc_psrf,
                    transp_water: transp_water_total,
                    deltim: dt,
                    leaf_area: lai,
                    aerodynamic_resistance: ra,
                    stomatal_resistance: rst,
                    prev_delta_e: p.leaf_delta_e,
                    prev_delta_b: p.leaf_delta_b,
                    prev_peclet: p.leaf_peclet,
                    prev_leaf_moles: p.leaf_water_moles,
                },
            );
            transp_ratio = out.trans_ratio;
            p.leaf_delta_e = out.new_delta_e;
            p.leaf_delta_b = out.new_delta_b;
            p.leaf_peclet = out.new_peclet;
            p.leaf_water_moles = out.new_leaf_moles;
        }

        if root_return_water > TRC_TINY {
            let mut root_return_tracer_total = 0.0;
            for j in 1..=nl {
                let k = soil_slot(j);
                let e = input.etroot_actual[k];
                if e >= -TRC_TINY {
                    continue;
                }
                water_shadow[k] -= e;
                if nonvolatile {
                    continue;
                }
                p.wliq_soisno[s(j)] = (-e).mul_add(return_ratio, p.wliq_soisno[s(j)]);
                root_return_tracer_total = (-e).mul_add(return_ratio, root_return_tracer_total);
            }
            if etroot_aquifer < -TRC_TINY && !nonvolatile {
                p.wa = (-etroot_aquifer).mul_add(return_ratio, p.wa);
                root_return_tracer_total =
                    (-etroot_aquifer).mul_add(return_ratio, root_return_tracer_total);
            }
            if surface_root_return > TRC_TINY && !nonvolatile {
                p.wdsrf = surface_root_return.mul_add(return_ratio, p.wdsrf);
                root_return_tracer_total =
                    surface_root_return.mul_add(return_ratio, root_return_tracer_total);
            }
            if !nonvolatile {
                let limit = root_return_excess.mul_add(excess_ratio, root_gross_tracer)
                    + (return_ratio.abs() * 1.0e-9).max(1.0e-12);
                if root_return_tracer_total > limit {
                    bail!("plant hydraulic isotope return exceeds actual donor isotope");
                }
                if transp_frac_active {
                    transp_source_tracer_total -= root_return_tracer_total;
                } else {
                    let acc = &mut state.acc[itrc];
                    acc.evap -= root_return_tracer_total;
                    acc.transp -= root_return_tracer_total;
                    acc.transp_src -= root_return_tracer_total;
                    acc.water_transp -= root_return_water;
                    acc.water_evap_gross -= root_return_water;
                }
            }
        }
        check(p.wa, wa_after_root, "after aquifer root exchange")?;

        // 基流按 VSF 交换的来源（积水/各层/含水层）离开。
        if resolved_rsub {
            let layer = input.rsub_source_layer.unwrap_or(&[0.0; SOIL_LAYERS]);
            for j in 1..=nl {
                let k = soil_slot(j);
                let donor_water = layer[k].max(0.0);
                if donor_water <= TRC_TINY {
                    continue;
                }
                let ratio =
                    current_liq_ratio(&water_shadow, &p.wliq_soisno, &ratio_layer, nonvolatile, j);
                let flux = p.wliq_soisno[s(j)].max(0.0).min(ratio * donor_water);
                p.wliq_soisno[s(j)] -= flux;
                water_shadow[k] -= donor_water;
                book_rsub(state, itrc, flux);
            }
            let donor_water = input.rsub_source_surface.unwrap_or(0.0).max(0.0);
            if donor_water > TRC_TINY {
                let mut donor_ratio = surface_et_ratio;
                let donor_carrier = (input.wdsrf_bef - surface_et_water) + surface_root_return;
                if donor_carrier > TRC_WATER_MIN_FOR_RATIO {
                    donor_ratio = p.wdsrf / donor_carrier;
                }
                let flux = p.wdsrf.max(0.0).min(donor_ratio * donor_water);
                p.wdsrf -= flux;
                book_rsub(state, itrc, flux);
            }
            let donor_water = input.rsub_source_aquifer.unwrap_or(0.0).max(0.0);
            if donor_water > TRC_TINY {
                let mut donor_ratio = aquifer_ratio;
                check(p.wa, wa_after_root, "before aquifer baseflow")?;
                if aquifer_actual_water(wa_after_root, aquifer_ref_water).abs()
                    > TRC_WATER_MIN_FOR_RATIO
                {
                    donor_ratio = aquifer_isotope_ratio(
                        wa_after_root,
                        p.wa,
                        aquifer_ref_water,
                        aquifer_ref_mass,
                        aquifer_ratio,
                    );
                }
                let mut flux = donor_ratio * donor_water;
                if aquifer_actual_water(wa_after_root, aquifer_ref_water) > TRC_TINY {
                    flux = flux.min(aquifer_actual_mass(p.wa, aquifer_ref_mass).max(0.0));
                }
                p.wa -= flux;
                check(p.wa, wa_after_root - donor_water, "after aquifer baseflow")?;
                book_rsub(state, itrc, flux);
            }
        }

        // `:658-670`：NSS 只改蒸腾汽的同位素组成；根系取出的与蒸腾带走的差额留在叶片储量里。
        if transp_frac_active {
            state.acc[itrc].transp_src += transp_source_tracer_total;
            let transp_output_tracer = transp_water_total * transp_ratio;
            state.book_evap_loss(
                itrc,
                transp_output_tracer,
                transp_water_total,
                EvapKind::Transpiration,
            );
            p.leaf_iso_storage =
                (p.leaf_iso_storage + transp_source_tracer_total) - transp_output_tracer;
        }

        // 0b/0c. 雪顶层外部通量与渗流。
        let mut d_wice_ext_snow = 0.0;
        let mut trc_gwat_snow = 0.0;
        if input.snl < 0 {
            let col = SnowColumn {
                snl: input.snl,
                deltim: dt,
                split_soilsnow: input.split_soilsnow,
                fsno: input.fsno,
                pg_rain: input.pg_rain,
                qseva_in: input.qseva_in,
                qsdew_in: input.qsdew_in,
                qsubl_in: input.qsubl_in,
                qfros_in: input.qfros_in,
                qseva_snow: input.qseva_snow,
                qsdew_snow: input.qsdew_snow,
                qsubl_snow: input.qsubl_snow,
                qfros_snow: input.qfros_snow,
                wliq_soisno: input.wliq_soisno,
                wice_soisno: input.wice_soisno,
                wliq_soisno_bef: wliq_bef,
                wice_soisno_bef: wice_bef,
                snow_qout_layer: input.snow_qout_layer,
                layer_temp: &layer_temp,
                snowmelt_equilibration: options.snowmelt_equilibration,
            };
            let outcome = snow_column(&ctx, state, &mut p, &col, FrostShape::Shared);
            d_wice_ext_snow = outcome.d_wice_ext_snow;
            trc_gwat_snow = outcome.trc_gwat_snow;
        }

        // 1. 地表混合池。
        let mut trc_soil_upflow = 0.0;
        let mut top_soil_evap_water = 0.0;
        let imperv_wdsrf_loss = input.imperv_evap_wdsrf.map_or(0.0, |x| x.max(0.0));
        let imperv_soil_loss = input.imperv_evap_soil.map_or(0.0, |x| x.max(0.0));
        let imperv_subl_loss = input.imperv_subl_soil.map_or(0.0, |x| x.max(0.0));
        let (eff_qseva, eff_qsdew_topliq, eff_qsubl_top, eff_qfros_top) =
            if input.snl < 0 && !input.split_soilsnow {
                (0.0, 0.0, 0.0, 0.0)
            } else if input.split_soilsnow {
                (
                    input.qseva_soil,
                    input.qsdew_soil,
                    input.qsubl_soil,
                    input.qfros_soil,
                )
            } else {
                (
                    input.qseva_in,
                    input.qsdew_in,
                    input.qsubl_in,
                    input.qfros_in,
                )
            };
        let eff_qsdew_topliq = (eff_qsdew_topliq - dew_surface_water / dt_floor).max(0.0);
        let mut surface_base_water = (wdsrf_pos + rsur_water) + top_infil_water;
        let mut surface_base_balance = surface_base_water;
        if flood_water > 0.0 {
            surface_base_balance = dt.mul_add(input.qinfl, rsur_water + wdsrf_pos) - flood_water;
            surface_base_water = surface_base_balance.max(0.0);
        }
        if late_surface_water > TRC_TINY {
            surface_base_water = (surface_base_water - late_surface_water).max(0.0);
            surface_base_balance -= late_surface_water;
        }

        let top_boundary_out_water = dt * (-input.qinfl).max(0.0);
        if top_boundary_out_water > TRC_TINY {
            let qgtop_est = match input.qgtop_solver {
                Some(qgtop) => qgtop - flood_water / dt_floor,
                None => {
                    ((input.wdsrf - input.wdsrf_bef) / dt_floor + input.qinfl)
                        + ((late_runoff_water - flood_water) - late_surface_water) / dt_floor
                }
            };
            if eff_qseva > TRC_TINY && qgtop_est < -TRC_TINY {
                top_soil_evap_water = top_boundary_out_water;
                let flux = ctx.atmospheric_loss_soil_surface(
                    p.wliq_soisno[s(1)],
                    water_shadow[0].max(0.0),
                    top_soil_evap_water,
                    layer_temp(1),
                );
                p.wliq_soisno[s(1)] -= flux;
                state.book_evap_loss(itrc, flux, top_soil_evap_water, EvapKind::SoilEvaporation);
                water_shadow[0] -= top_soil_evap_water;
            } else {
                let top_exfil_water = top_boundary_out_water;
                trc_soil_upflow =
                    (ratio_layer[0] * top_exfil_water).min(p.wliq_soisno[s(1)].max(0.0));
                p.wliq_soisno[s(1)] -= trc_soil_upflow;
                water_shadow[0] -= top_exfil_water;
            }
        }

        let pg_rain_ground = state.step[itrc].pg_rain_ground;
        let mut trc_pool_total = if input.snl < 0 {
            let total = trc_gwat_snow + p.wdsrf;
            if input.split_soilsnow {
                pg_rain_ground.mul_add(1.0 - input.fsno, total)
            } else {
                total
            }
        } else {
            pg_rain_ground + p.wdsrf
        };
        if input.snl >= 0 && input.sm > TRC_TINY {
            trc_pool_total += state.step[itrc].sm_carry;
        }

        let gwat_evap = (dt.mul_add(eff_qseva.max(0.0), -top_soil_evap_water)
            - imperv_wdsrf_loss
            - imperv_soil_loss
            - imperv_subl_loss)
            .max(0.0);
        let mut flood_ground_evap_water = 0.0;
        if flood_water > 0.0 {
            flood_ground_evap_water = (-surface_base_balance)
                .max(0.0)
                .min(flood_water)
                .min(gwat_evap);
            if flood_destination_water <= 0.0 {
                flood_ground_evap_water = flood_water;
            }
        }
        let gwat_evap = (gwat_evap - flood_ground_evap_water).max(0.0);
        let mut flood_ground_evap_tracer = 0.0;
        if flood_ground_evap_water > TRC_TINY {
            if let Some(flood) = input.flood_tracer_input {
                flood_ground_evap_tracer = ctx.atmospheric_loss(
                    flood[itrc],
                    flood_water,
                    flood_ground_evap_water,
                    layer_temp(1),
                    false,
                );
                state.book_evap_loss(
                    itrc,
                    flood_ground_evap_tracer,
                    flood_ground_evap_water,
                    EvapKind::SoilEvaporation,
                );
            }
        }
        if imperv_soil_loss > TRC_TINY {
            let flux = ctx.atmospheric_loss_soil_surface(
                p.wliq_soisno[s(1)],
                water_shadow[0].max(0.0),
                imperv_soil_loss,
                layer_temp(1),
            );
            p.wliq_soisno[s(1)] -= flux;
            state.book_evap_loss(itrc, flux, imperv_soil_loss, EvapKind::SoilEvaporation);
            water_shadow[0] -= imperv_soil_loss;
        }
        if imperv_subl_loss > TRC_TINY {
            let flux = ctx.atmospheric_loss(
                p.wice_soisno[s(1)],
                wice_bef[s(1)].max(0.0),
                imperv_subl_loss,
                layer_temp(1),
                true,
            );
            p.wice_soisno[s(1)] -= flux;
            state.book_evap_loss(itrc, flux, imperv_subl_loss, EvapKind::Sublimation);
        }
        if gwat_evap > TRC_TINY && trc_pool_total > TRC_TINY {
            let water_pool_total = gwat_evap + surface_base_water;
            let flux = ctx.atmospheric_loss(
                trc_pool_total,
                water_pool_total,
                gwat_evap,
                layer_temp(1),
                false,
            );
            trc_pool_total -= flux;
            state.book_evap_loss(itrc, flux, gwat_evap, EvapKind::SoilEvaporation);
        }
        if imperv_wdsrf_loss > TRC_TINY && trc_pool_total > TRC_TINY {
            let water_pool_total = imperv_wdsrf_loss + surface_base_water;
            let flux = ctx.atmospheric_loss(
                trc_pool_total,
                water_pool_total,
                imperv_wdsrf_loss,
                layer_temp(1),
                false,
            );
            trc_pool_total -= flux;
            state.book_evap_loss(itrc, flux, imperv_wdsrf_loss, EvapKind::SoilEvaporation);
        }
        // 土壤出渗是 VSF 解出来的，晚于本步地面蒸发。
        if flood_water <= 0.0 {
            trc_pool_total += trc_soil_upflow;
        }

        // 滴灌/漫灌/水田灌溉：从 `waterstorage` 内部转移，不算大气输入。
        if input.qflx_irrig_ground > TRC_TINY {
            let mut storage_ratio = r_precip;
            if let Some(storage) = input.waterstorage_patch {
                tracer.equilibrate_dissolved(
                    storage.max(0.0),
                    &mut p.waterstorage,
                    &mut p.waterstorage_solid,
                );
                if storage > TRC_WATER_MIN_FOR_RATIO {
                    storage_ratio = p.waterstorage / storage;
                }
            }
            let flux = (dt * input.qflx_irrig_ground * storage_ratio)
                .max(0.0)
                .min(p.waterstorage.max(0.0));
            trc_pool_total = flux + trc_pool_total;
            p.waterstorage -= flux;
        }

        let water_pool_total = surface_base_water;
        if nonvolatile
            && (water_pool_total > TRC_WATER_MIN_FOR_RATIO || tracer.has_dissolved_limit())
            && p.surface_residue > TRC_TINY
        {
            trc_pool_total = p.surface_residue + trc_pool_total;
            p.surface_residue = 0.0;
        }
        tracer.equilibrate_dissolved(water_pool_total, &mut trc_pool_total, &mut p.surface_solid);

        let ratio = if water_pool_total > TRC_WATER_MIN_FOR_RATIO && trc_pool_total > TRC_TINY {
            trc_pool_total / water_pool_total
        } else if trc_pool_total <= TRC_TINY {
            0.0
        } else {
            // 没有可解析的水去处：显式记为数值残差（溶质记作地表残留）。
            if nonvolatile {
                p.surface_residue += trc_pool_total;
            } else {
                state.step[itrc].numerical_residual_step -= trc_pool_total;
            }
            trc_pool_total = 0.0;
            0.0
        };

        if early_runoff_water > TRC_TINY {
            let flux = ratio * early_runoff_water;
            book_rsur(state, itrc, flux);
            trc_pool_total = (trc_pool_total - flux).max(0.0);
        }

        let mut late_ratio = ratio;
        if flood_water > 0.0 {
            let mut late_tracer = trc_pool_total + trc_soil_upflow;
            let mut late_scale = trc_pool_total.abs() + trc_soil_upflow.abs();
            if let Some(flood) = input.flood_tracer_input {
                late_tracer = (flood[itrc] + late_tracer) - flood_ground_evap_tracer;
                late_scale = (flood[itrc].abs() + late_scale) + flood_ground_evap_tracer.abs();
            }
            let late_water = flood_destination_water;
            let isotope_microcarrier = is_isotope
                && late_water > 0.0
                && late_tracer > TRC_TINY
                && late_tracer <= tracer.ref_ratio * 3.0 * late_water;
            if late_water > TRC_WATER_MIN_FOR_RATIO || isotope_microcarrier {
                if nonvolatile && p.surface_residue > TRC_TINY {
                    late_tracer = p.surface_residue + late_tracer;
                    p.surface_residue = 0.0;
                }
                late_ratio = late_tracer.max(0.0) / late_water;
            } else {
                if nonvolatile {
                    p.surface_residue += late_tracer.max(0.0);
                } else if late_tracer
                    > (late_scale * 1.0e-10)
                        .max((tracer.ref_ratio * TRC_WATER_MIN_FOR_RATIO * 3.0).max(TRC_TINY))
                {
                    bail!("grid flood tracer: unresolved dry isotope surface pool");
                } else if late_tracer > 0.0 {
                    state.step[itrc].numerical_residual_step -= late_tracer;
                }
                late_ratio = 0.0;
                late_tracer = 0.0;
            }
            trc_pool_total = late_tracer;
        }
        p.wdsrf = late_ratio * wdsrf_pos;

        let mut pending_surface_tracer = 0.0;
        if late_surface_water > TRC_TINY {
            pending_surface_tracer = (-dt)
                .mul_add(late_ratio * input.qinfl.max(0.0), trc_pool_total)
                .max(0.0);
        }

        let infil_tracer = dt * (late_ratio * input.qinfl);
        if input.qinfl > TRC_TINY {
            state.acc[itrc].qinfl += infil_tracer;
        } else if input.qinfl < -TRC_TINY {
            let acc = &mut state.acc[itrc];
            acc.qinfl = dt.mul_add(ratio_layer[0] * input.qinfl, acc.qinfl);
        }
        if let Some(flood) = input.flood_tracer_input {
            state.acc[itrc].precip += flood[itrc];
        }

        // 2. 地表→第 1 层的入渗（用 qinfl，不用 qlayer(0)），再按 qlayer 逐界面搬运。
        if input.qinfl > TRC_TINY {
            p.wliq_soisno[s(1)] = infil_tracer + p.wliq_soisno[s(1)];
            water_shadow[0] = dt.mul_add(input.qinfl, water_shadow[0]);
        }
        let mut layer_transport_ratio = [0.0; SOIL_LAYERS];
        for j in 1..=nl {
            layer_transport_ratio[soil_slot(j)] =
                current_liq_ratio(&water_shadow, &p.wliq_soisno, &ratio_layer, nonvolatile, j);
        }
        for j in 1..nl {
            let (k, kn) = (soil_slot(j), soil_slot(j + 1));
            let q = input.qlayer[j as usize];
            if q > TRC_TINY {
                let flux = (dt * (q * layer_transport_ratio[k])).min(p.wliq_soisno[s(j)].max(0.0));
                p.wliq_soisno[s(j)] -= flux;
                p.wliq_soisno[s(j + 1)] += flux;
                water_shadow[k] = (-dt).mul_add(q, water_shadow[k]);
                water_shadow[kn] = dt.mul_add(q, water_shadow[kn]);
            } else if q < -TRC_TINY {
                let flux = (dt * (q.abs() * layer_transport_ratio[kn]))
                    .min(p.wliq_soisno[s(j + 1)].max(0.0));
                p.wliq_soisno[s(j + 1)] -= flux;
                p.wliq_soisno[s(j)] += flux;
                water_shadow[kn] = dt.mul_add(q, water_shadow[kn]);
                water_shadow[k] = (-dt).mul_add(q, water_shadow[k]);
            }
        }

        // 液膜 + 孔隙气相扩散（层间内部交换，不进收支）。
        if soil_diffusion_block {
            if let (Some(dz_soi), Some(porsl)) = (input.dz_soi, input.porsl) {
                soil_diffusion(
                    &ctx,
                    &mut p,
                    &water_shadow,
                    input.wice_soisno,
                    dz_soi,
                    porsl,
                    &layer_temp,
                    options.soil_diffusion,
                    options.soil_vapor_diffusion,
                    input.forc_psrf,
                    dt,
                );
            }
        }
        // 雪层（冰载体）的气相扩散。
        if snow_vapor_block {
            if let (Some(dz_sno), Some(psrf)) = (input.dz_sno, input.forc_psrf) {
                snow_vapor_diffusion(
                    &ctx,
                    &mut p,
                    input.snl,
                    input.wliq_soisno,
                    input.wice_soisno,
                    dz_sno,
                    &layer_temp,
                    psrf,
                    dt,
                );
            }
        }

        // 3. 含水层：根系交换与解析的基流之后，`wa_bef` 已不是当前载体。
        let mut aquifer_water_pre_qcharge = wa_after_root;
        if resolved_rsub {
            aquifer_water_pre_qcharge -= input.rsub_source_aquifer.unwrap_or(0.0).max(0.0);
        }
        let bottom = s(nl);
        let kb = soil_slot(nl);
        if qcharge_eff > TRC_TINY {
            let flux = (dt * (qcharge_eff * layer_transport_ratio[kb]))
                .min(p.wliq_soisno[bottom].max(0.0));
            p.wliq_soisno[bottom] -= flux;
            p.wa += flux;
            state.acc[itrc].qcharge += flux;
            water_shadow[kb] = (-dt).mul_add(qcharge_eff, water_shadow[kb]);
        } else if qcharge_eff < -TRC_TINY {
            check(p.wa, aquifer_water_pre_qcharge, "qcharge export")?;
            let ratio_src = if aquifer_actual_water(aquifer_water_pre_qcharge, aquifer_ref_water)
                .abs()
                > TRC_WATER_MIN_FOR_RATIO
            {
                aquifer_isotope_ratio(
                    aquifer_water_pre_qcharge,
                    p.wa,
                    aquifer_ref_water,
                    aquifer_ref_mass,
                    source_fallback_ratio,
                )
            } else if nonvolatile {
                0.0
            } else {
                source_fallback_ratio
            };
            let mut flux = dt * (qcharge_eff.abs() * ratio_src);
            if aquifer_actual_water(aquifer_water_pre_qcharge, aquifer_ref_water) > TRC_TINY {
                flux = flux.min(aquifer_actual_mass(p.wa, aquifer_ref_mass).max(0.0));
            }
            p.wa -= flux;
            p.wliq_soisno[bottom] += flux;
            state.acc[itrc].qcharge -= flux;
            water_shadow[kb] = (-dt).mul_add(qcharge_eff, water_shadow[kb]);
        }

        if nonvolatile && input.wa > TRC_WATER_MIN_FOR_RATIO && p.subsurface_residue > TRC_TINY {
            p.wa = p.subsurface_residue + p.wa;
            p.subsurface_residue = 0.0;
        }
        tracer.equilibrate_dissolved(input.wa, &mut p.wa, &mut p.subsurface_solid);
        if is_isotope
            && !aquifer_isotope_state_valid(
                input.wa,
                p.wa,
                tracer.ref_ratio,
                aquifer_ref_water,
                aquifer_ref_mass,
            )
        {
            let mut aquifer_orphan_mass = aquifer_actual_mass(p.wa, aquifer_ref_mass);
            let actual_water = aquifer_actual_water(input.wa, aquifer_ref_water);
            if actual_water < -TRC_WATER_MIN_FOR_RATIO {
                aquifer_orphan_mass = (-actual_water).mul_add(aquifer_ratio, aquifer_orphan_mass);
            }
            p.wa -= aquifer_orphan_mass;
            state.step[itrc].numerical_residual_step -= aquifer_orphan_mass;
        }
        check(p.wa, input.wa, "after qcharge")?;

        // 4. 未解析来源时，基流取更新后的底层浓度。
        if input.rsub > TRC_TINY && !resolved_rsub {
            let rsub_water = dt * input.rsub;
            let pre_rsub_water = input.wliq_soisno[bottom] + rsub_water;
            let ratio_src = if pre_rsub_water > TRC_WATER_MIN_FOR_RATIO {
                p.wliq_soisno[bottom] / pre_rsub_water
            } else if nonvolatile {
                0.0
            } else {
                ratio_layer[kb]
            };
            let flux = (dt * (input.rsub * ratio_src)).min(p.wliq_soisno[bottom].max(0.0));
            p.wliq_soisno[bottom] -= flux;
            book_rsub(state, itrc, flux);
            water_shadow[kb] -= rsub_water;
        }

        // 4a. 求解之后才到的积水：被霜挤出的旧液与超出孔隙的露。
        if late_surface_water > TRC_TINY {
            if frost_surface_water > TRC_TINY {
                let ratio =
                    current_liq_ratio(&water_shadow, &p.wliq_soisno, &ratio_layer, nonvolatile, 1);
                let flux = p.wliq_soisno[s(1)]
                    .max(0.0)
                    .min(ratio * frost_surface_water);
                p.wliq_soisno[s(1)] -= flux;
                water_shadow[0] -= frost_surface_water;
                pending_surface_tracer += flux;
            }
            if dew_surface_water > TRC_TINY {
                let r_dep = ctx.deposition_ratio_for(layer_temp(1), false);
                pending_surface_tracer = dew_surface_water.mul_add(r_dep, pending_surface_tracer);
                let acc = &mut state.acc[itrc];
                acc.precip = dew_surface_water.mul_add(r_dep, acc.precip);
                acc.water_precip += dew_surface_water;
            }
            let late_water = late_runoff_water + wdsrf_pos;
            let mut late_surface_ratio = 0.0;
            if late_water > TRC_WATER_MIN_FOR_RATIO {
                if nonvolatile && p.surface_residue > TRC_TINY {
                    pending_surface_tracer = p.surface_residue + pending_surface_tracer;
                    p.surface_residue = 0.0;
                }
                tracer.equilibrate_dissolved(
                    late_water,
                    &mut pending_surface_tracer,
                    &mut p.surface_solid,
                );
                late_surface_ratio = pending_surface_tracer / late_water;
            } else if pending_surface_tracer > TRC_TINY {
                if !nonvolatile {
                    bail!("late surface isotope water has no resolved store");
                }
                p.surface_residue = pending_surface_tracer + p.surface_residue;
            }
            p.wdsrf = late_surface_ratio * wdsrf_pos;
            book_rsur(state, itrc, late_runoff_water * late_surface_ratio);
        }

        // 4b. 第 1 层液相的露（WATER 直接加到 wliq_soisno(1)）。
        if eff_qsdew_topliq > TRC_TINY {
            let flux = dt * (eff_qsdew_topliq * ctx.deposition_ratio_for(layer_temp(1), false));
            p.wliq_soisno[s(1)] = flux + p.wliq_soisno[s(1)];
            let acc = &mut state.acc[itrc];
            acc.precip = flux + acc.precip;
            acc.water_precip = dt.mul_add(eff_qsdew_topliq, acc.water_precip);
            water_shadow[0] = dt.mul_add(eff_qsdew_topliq, water_shadow[0]);
        }

        // 5. 第 1 层冰相的外部通量（霜/升华）；先记下实际的外部冰量变化。
        let wice_soil1_after_imperv = (wice_bef[s(1)] - imperv_subl_loss).max(0.0);
        let frost_top_water = dt * eff_qfros_top.max(0.0);
        let wice_after_frost = frost_top_water + wice_soil1_after_imperv;
        let d_wice_ext_soil1 = (frost_top_water - imperv_subl_loss)
            - (dt * eff_qsubl_top.max(0.0)).min(wice_after_frost);
        if eff_qfros_top > TRC_TINY {
            let flux = dt * (eff_qfros_top * ctx.deposition_ratio_for(layer_temp(1), true));
            p.wice_soisno[s(1)] = flux + p.wice_soisno[s(1)];
            let acc = &mut state.acc[itrc];
            acc.precip = flux + acc.precip;
            acc.water_precip = dt.mul_add(eff_qfros_top, acc.water_precip);
        }
        if eff_qsubl_top > TRC_TINY {
            let wice_pre_phase = wice_after_frost.max(0.0);
            let subl_water = dt * eff_qsubl_top;
            if wice_pre_phase - subl_water > TRC_WATER_MIN_FOR_RATIO {
                let flux = ctx.atmospheric_loss(
                    p.wice_soisno[s(1)],
                    wice_pre_phase,
                    subl_water,
                    layer_temp(1),
                    true,
                );
                p.wice_soisno[s(1)] -= flux;
                state.book_evap_loss(itrc, flux, subl_water, EvapKind::Sublimation);
            } else if subl_water > TRC_TINY {
                exhaust_surface_phase(
                    tracer,
                    state,
                    itrc,
                    &mut p.wice_soisno[s(1)],
                    subl_water.min(wice_pre_phase.max(0.0)),
                    EvapKind::Sublimation,
                );
            }
        }

        // 5b. WATER 内部的冻融：总冰量变化减去外部通量。
        for j in lb..=nl {
            let slot = s(j);
            let mut d_wice = input.wice_soisno[slot] - wice_bef[slot];
            if j == 1 {
                d_wice -= d_wice_ext_soil1;
            }
            if j < 1 && j == lb && input.snl < 0 {
                d_wice -= d_wice_ext_snow;
            }
            if j >= 1 {
                let sink = input.wblc_ice_sink[soil_slot(j)];
                if sink > TRC_TINY {
                    // wblc 补蒸散亏缺抽走的冰：是蒸散出口，不是融化。
                    if !nonvolatile {
                        let wice_pre_phase = (input.wice_soisno[slot] + sink).max(0.0);
                        if wice_pre_phase > TRC_TINY {
                            let ratio_src = p.wice_soisno[slot] / wice_pre_phase;
                            let flux = (sink * ratio_src).min(p.wice_soisno[slot].max(0.0));
                            p.wice_soisno[slot] -= flux;
                            state.book_evap_loss(itrc, flux, sink, EvapKind::SoilEvaporation);
                        }
                    }
                    d_wice = sink + d_wice;
                }
            }
            if d_wice > TRC_TINY {
                let wliq_pre_phase = (input.wliq_soisno[slot] + d_wice).max(0.0);
                if wliq_pre_phase > TRC_TINY {
                    let flux = physics
                        .rayleigh_freezing_loss(
                            tracer,
                            p.wliq_soisno[slot],
                            wliq_pre_phase,
                            d_wice,
                            layer_temp(j),
                        )
                        .min(p.wliq_soisno[slot].max(0.0));
                    p.wliq_soisno[slot] -= flux;
                    p.wice_soisno[slot] += flux;
                    if j >= 1 {
                        water_shadow[soil_slot(j)] -= d_wice;
                    }
                }
            } else if d_wice < -TRC_TINY {
                let wice_pre_phase = (input.wice_soisno[slot] - d_wice).max(0.0);
                if wice_pre_phase > TRC_TINY {
                    let ratio_src = p.wice_soisno[slot] / wice_pre_phase;
                    let flux = (d_wice.abs() * ratio_src).min(p.wice_soisno[slot].max(0.0));
                    p.wice_soisno[slot] -= flux;
                    p.wliq_soisno[slot] += flux;
                    if j >= 1 {
                        water_shadow[soil_slot(j)] -= d_wice;
                    }
                }
            }
        }

        // VSF 在连通的饱和层之间搬了水却没记进 qlayer：先按封闭块重建。
        if let Some(permeable) = input.permeable_soil {
            reconcile::reconcile_internal_soil_flow(
                tracer,
                &mut p,
                &mut water_shadow,
                input.wliq_soisno,
                permeable,
            );
        }

        // 余下的数值失配按层当前浓度搬运，并显式记为数值源汇。
        for j in 1..=nl {
            let k = soil_slot(j);
            let water_resid = input.wliq_soisno[s(j)] - water_shadow[k];
            if water_resid.abs() > TRC_TINY {
                let water_shadow_ratio =
                    current_liq_ratio(&water_shadow, &p.wliq_soisno, &ratio_layer, nonvolatile, j);
                soil_resid_water += water_resid;
                let resid_tracer = water_resid * water_shadow_ratio;
                if water_resid >= 0.0 {
                    p.wliq_soisno[s(j)] = resid_tracer + p.wliq_soisno[s(j)];
                    soil_resid_trc += resid_tracer;
                } else {
                    let flux = p.wliq_soisno[s(j)].max(0.0).min(-resid_tracer);
                    p.wliq_soisno[s(j)] -= flux;
                    soil_resid_trc -= flux;
                }
            }
        }
        if soil_resid_trc.abs() > TRC_TINY {
            let step = &mut state.step[itrc];
            step.numerical_residual_step += soil_resid_trc;
            step.numerical_water_step += soil_resid_water;
        }
        if aquifer_actual_water(input.wa, aquifer_ref_water).abs() <= TRC_WATER_MIN_FOR_RATIO {
            check(p.wa, input.wa, "soil end")?;
            if is_isotope && aquifer_ref_water > 0.0 {
                // 有限参考载体真正耗尽时，-Mref 就是零实际质量。
                p.wa = -aquifer_ref_mass;
            } else if tracer.has_dissolved_limit() && p.wa > TRC_TINY {
                p.subsurface_solid += p.wa;
            } else if nonvolatile && p.wa > TRC_TINY {
                p.subsurface_residue += p.wa;
            } else if p.wa.abs() > TRC_TINY {
                state.step[itrc].numerical_residual_step -= p.wa;
            }
            if !is_isotope || aquifer_ref_water <= 0.0 {
                p.wa = 0.0;
            }
        }

        // 6. 标准土壤支不应改动湿地水。
        let d_wetwat = input.wetwat - input.wetwat_bef;
        if options.colm_debug && d_wetwat.abs() > TRC_TINY {
            eprintln!(
                " WARNING tracer_soil_water: wetwat changed in standard soil path ipatch={:8} \
                 itrc={:3} d_wetwat={:12.5E} wetwat_bef={:12.5E} wetwat={:12.5E}",
                input.ipatch,
                itrc + 1,
                d_wetwat,
                input.wetwat_bef,
                input.wetwat
            );
        }

        for j in lb..=nl {
            tracer.equilibrate_dissolved(
                input.wliq_soisno[s(j)].max(0.0),
                &mut p.wliq_soisno[s(j)],
                &mut p.solid_soisno[s(j)],
            );
        }
        tracer.equilibrate_dissolved(input.wa, &mut p.wa, &mut p.subsurface_solid);
        tracer.equilibrate_dissolved(wdsrf_pos, &mut p.wdsrf, &mut p.surface_solid);
        if let Some(storage) = input.waterstorage_patch {
            tracer.equilibrate_dissolved(
                (-dt)
                    .mul_add(input.qflx_irrig_ground.max(0.0), storage)
                    .max(0.0),
                &mut p.waterstorage,
                &mut p.waterstorage_solid,
            );
        }
        state.pools[itrc] = p;
    }
    Ok(())
}

/// 地下径流的三处累加（`a_trc_rsub`、`a_trc_rnof`、`trc_rnof_step`）。
fn book_rsub(state: &mut PatchTracerState, itrc: usize, flux: f64) {
    let acc = &mut state.acc[itrc];
    acc.rsub += flux;
    acc.rnof += flux;
    state.step[itrc].rnof_step += flux;
}

/// 地表径流的三处累加（`a_trc_rsur`、`a_trc_rnof`、`trc_rnof_step`）。
fn book_rsur(state: &mut PatchTracerState, itrc: usize, flux: f64) {
    let acc = &mut state.acc[itrc];
    acc.rsur = flux + acc.rsur;
    acc.rnof = flux + acc.rnof;
    let step = &mut state.step[itrc];
    step.rnof_step = flux + step.rnof_step;
}

#[cfg(test)]
#[path = "soil_water_tests.rs"]
mod tests;
