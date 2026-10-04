//! `tracer_wetland`：静态湿地（`patchtype == 2 .and. .not. DEF_USE_Dynamic_Wetland`）。
//!
//! WATER_VSF 的湿地支把 `wdsrf + wa + wetwat + (gwat-etr+dew+frost-subl)*dt + Σwresi`
//! 并成一个 `wetwat` 池，再溢回 `wdsrf`、亏欠记到 `wa`。这里逐项镜像这次合并：
//! 先做雪顶层与渗流（与 `tracer_soil_water` 第 0b/0c 节同形，霜量见 [`FrostShape::Fused`]），
//! 再剥离超饱和的 `wresi`，建混合池，按池比值去掉蒸发/升华/蒸腾，最后按 WATER 之后的
//! `wetwat`/`wdsrf`/`wa`/`rsur` 重新分配，残差留在存量里。
//!
//! GIMPLE（不分馏）：
//! * `wresi_j = max(FNMA(porsl*dz, 1000, wliq_bef), 0)`；
//! * `pool_water`、`pool_tracer` 严格自左向右累加；
//! * 露/霜输入 `pool + q_dew*R` 先舍入，再 `FMA(q_frost, R, ·)`，`a_trc_precip` 同形；
//! * 湿地蒸发+升华的示踪物 `FMA(pool_ratio, q_evap, pool_ratio*q_subl)`（不单独舍入蒸发量）；
//! * `trc_wa = FMS(actual(wa), pool_ratio, ref_mass)`；
//! * 径流 `trc_rsur_local = (rsur*dt)*pool_ratio` 不舍入：三个累加器都是 `.FMA`，
//!   `redist_target = FNMA(rsur*dt, pool_ratio, pool_tracer)`。

use anyhow::{ensure, Result};

use super::super::{
    soisno_slot, EvapKind, PatchTracerState, TracerSet, MAX_SNOW_LAYERS, SOIL_LAYERS,
    SOISNO_LAYERS, TRC_TINY, TRC_WATER_MIN_FOR_RATIO,
};
use super::common::{
    aquifer_actual_water, check_isotope_aquifer, release_leaf_iso_storage, snow_column,
    snow_vapor_diffusion, soil_slot, EvapKinetics, FrostShape, SnowColumn, TracerCtx,
    DEFAULT_LAYER_TEMP_K,
};
use super::SoilWaterOptions;
use crate::tracer::TracerPhysics;
use crate::FREEZING_K;

/// `tracer_wetland` 的实参。层数组同 [`super::SoilWaterInput`]：雪+土为 `[f64; 15]`
/// 按 [`soisno_slot`] 取（只读 `snl+1..=nl_soil`），土壤层 `[f64; 10]` 下标 `j-1`，
/// 雪层 `[f64; 5]` 按 [`soisno_slot`] 取（只读 `snl+1..=0`）。
#[derive(Debug, Clone, Copy)]
pub struct WetlandInput<'a> {
    /// `ipatch`（只用于报错信息）。
    pub ipatch: usize,
    pub deltim: f64,
    pub snl: i32,
    pub rsur: f64,
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
    pub etr: f64,
    pub sm: f64,
    pub fsno: f64,
    /// `DEF_SPLIT_SOILSNOW`。
    pub split_soilsnow: bool,
    /// WATER 之后的 `wliq_soisno`/`wice_soisno`。
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
    /// `t_soisno(snl+1:nl_soil)`（必给；`layer_temp` 越界时取 273.15）。
    pub t_soisno: &'a [f64; SOISNO_LAYERS],
    /// `porsl(1:nl_soil)`。
    pub porsl: &'a [f64; SOIL_LAYERS],
    /// `dz_soisno(snl+1:nl_soil)` [m]。
    pub dz_soisno: &'a [f64; SOISNO_LAYERS],
    /// `qflx_irrig_drip + qflx_irrig_flood + qflx_irrig_paddy` [mm/s]。
    pub qflx_irrig_ground: f64,
    /// `forc_us`/`forc_vs`：开阔水面的动力分馏。
    pub forc_us: f64,
    pub forc_vs: f64,
    /// `waterstorage_trc_ground`（CoLMMAIN 总是给出）。
    pub waterstorage_patch: Option<f64>,
    /// `snow_qout_layer_trc(snl+1:0)`。
    pub snow_qout_layer: Option<&'a [f64; MAX_SNOW_LAYERS]>,
    /// `forc_q`：分馏的近地面相对湿度。
    pub forc_q: Option<f64>,
    /// `forc_psrf`：雪层气相扩散与分馏。
    pub forc_psrf: Option<f64>,
    /// `tleaf`、`rst`、`raw_trc`：叶片 NSS。
    pub tleaf: Option<f64>,
    /// `lai`：叶面积消失时释放叶片同位素异常。
    pub lai: Option<f64>,
    pub rst: Option<f64>,
    pub ra: Option<f64>,
    /// `dz_soisno(snl+1:0)` [m]：雪层气相扩散。
    pub dz_sno: Option<&'a [f64; MAX_SNOW_LAYERS]>,
    /// 逐示踪物的 `tracer_forcing_vapor_value(itrc, ipatch)`。
    pub vapor_ratio: &'a [f64],
    /// 逐示踪物的 `tracer_forcing_has_vapor(itrc, ipatch)`（`None` 即都没有）。
    pub has_vapor: Option<&'a [bool]>,
}

/// `tracer_wetland`。
pub fn tracer_wetland(
    set: &TracerSet,
    state: &mut PatchTracerState,
    physics: TracerPhysics,
    options: &SoilWaterOptions,
    input: &WetlandInput<'_>,
) -> Result<()> {
    if set.is_empty() {
        return Ok(());
    }
    ensure!(
        (-(MAX_SNOW_LAYERS as i32)..=0).contains(&input.snl),
        "tracer_wetland: snl={} outside -{}..=0",
        input.snl,
        MAX_SNOW_LAYERS
    );
    ensure!(
        input.vapor_ratio.len() >= set.len(),
        "tracer_wetland: vapor ratio has {} entries for {} tracers",
        input.vapor_ratio.len(),
        set.len()
    );
    // 开阔水面风速 `sqrt(FMA(us, us, vs*vs))`（GIMPLE 去掉了 `max(·,0)`）。
    let wind = input
        .forc_us
        .mul_add(input.forc_us, input.forc_vs * input.forc_vs)
        .sqrt();

    let dt = input.deltim;
    let nl = SOIL_LAYERS as i32;
    let lb = input.snl + 1;
    // `layer_temp`：`t_soisno` 的界内取值，否则 273.15。
    let layer_temp = |j: i32| {
        if (lb..=nl).contains(&j) {
            input.t_soisno[soisno_slot(j)]
        } else {
            DEFAULT_LAYER_TEMP_K
        }
    };
    let wliq_bef_soil = &input.wliq_soisno_bef[soisno_slot(1)..];

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
            EvapKinetics::OpenWater { wind },
        );
        let nonvolatile = ctx.nonvolatile;
        let mut p = state.pools[itrc].clone();
        let (aquifer_ref_water, aquifer_ref_mass) = if tracer.is_isotope() {
            (state.aquifer_ref_water, p.aquifer_ref_mass)
        } else {
            (0.0, 0.0)
        };
        let r_atm = input.vapor_ratio[itrc];
        let patch_ref_water = state.aquifer_ref_water;
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

        for j in lb..=nl {
            let slot = soisno_slot(j);
            tracer.equilibrate_dissolved(
                input.wliq_soisno_bef[slot].max(0.0),
                &mut p.wliq_soisno[slot],
                &mut p.solid_soisno[slot],
            );
        }
        if let Some(lai) = input.lai {
            if lai <= TRC_TINY || p.leaf_water_moles <= TRC_TINY {
                release_leaf_iso_storage(
                    tracer,
                    &mut p,
                    state.aquifer_ref_water,
                    wliq_bef_soil,
                    input.wa_bef,
                );
            }
        }
        if tracer.has_dissolved_limit() {
            p.surface_solid += p.subsurface_solid;
            p.subsurface_solid = 0.0;
        }
        check(p.wa, input.wa_bef, "wetland start")?;

        // 1) 雪顶层外部通量与渗流。
        let mut trc_gwat_snow_local = 0.0;
        let mut gwat_snow_local = 0.0;
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
                wliq_soisno_bef: input.wliq_soisno_bef,
                wice_soisno_bef: input.wice_soisno_bef,
                snow_qout_layer: input.snow_qout_layer,
                layer_temp: &layer_temp,
                snowmelt_equilibration: options.snowmelt_equilibration,
            };
            let outcome = snow_column(&ctx, state, &mut p, &col, FrostShape::Fused);
            trc_gwat_snow_local = outcome.trc_gwat_snow;
            gwat_snow_local = outcome.gwat_snow;
            if let (true, Some(dz_sno), Some(psrf)) =
                (options.soil_vapor_diffusion, input.dz_sno, input.forc_psrf)
            {
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

        // 2) 超过饱和的 wresi 并入混合池（`MOD_SoilSnowHydrology.F90` 湿地支）。
        let mut trc_wresi_sum = 0.0;
        let mut wresi_sum = 0.0;
        for j in 1..=nl {
            let slot = soisno_slot(j);
            if input.t_soisno[slot] > FREEZING_K {
                let wliq_bef = input.wliq_soisno_bef[slot];
                let wresi_j = (-(input.porsl[soil_slot(j)] * input.dz_soisno[slot]))
                    .mul_add(1000.0, wliq_bef)
                    .max(0.0);
                if wresi_j > TRC_TINY {
                    wresi_sum = wresi_j + wresi_sum;
                    if wliq_bef > TRC_WATER_MIN_FOR_RATIO {
                        let ratio_src = p.wliq_soisno[slot] / wliq_bef;
                        let trc_wresi_j = (wresi_j * ratio_src).min(p.wliq_soisno[slot].max(0.0));
                        p.wliq_soisno[slot] -= trc_wresi_j;
                        trc_wresi_sum = trc_wresi_j + trc_wresi_sum;
                    }
                }
            }
        }

        // 3) 混合池。
        let (q_rain_in, q_dew_in, q_frost_in, q_evap_out, q_subl_out, q_sm_in) = if input.snl < 0 {
            if input.split_soilsnow {
                (
                    (input.pg_rain * (1.0 - input.fsno)).max(0.0) * dt,
                    input.qsdew_soil.max(0.0) * dt,
                    input.qfros_soil.max(0.0) * dt,
                    input.qseva_soil.max(0.0) * dt,
                    input.qsubl_soil.max(0.0) * dt,
                    0.0,
                )
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            }
        } else {
            let (dew, frost, evap, subl) = if input.split_soilsnow {
                (
                    input.qsdew_soil,
                    input.qfros_soil,
                    input.qseva_soil,
                    input.qsubl_soil,
                )
            } else {
                (
                    input.qsdew_in,
                    input.qfros_in,
                    input.qseva_in,
                    input.qsubl_in,
                )
            };
            (
                input.pg_rain.max(0.0) * dt,
                dew.max(0.0) * dt,
                frost.max(0.0) * dt,
                evap.max(0.0) * dt,
                subl.max(0.0) * dt,
                input.sm.max(0.0) * dt,
            )
        };
        let q_etr_out = input.etr.max(0.0) * dt;

        let mut pool_water = (((((((input.wdsrf_bef + input.wa_bef) + input.wetwat_bef)
            + aquifer_ref_water)
            + wresi_sum)
            + q_rain_in)
            + q_sm_in)
            + q_dew_in)
            + q_frost_in;
        let mut pool_tracer = (((p.wdsrf + p.wa) + aquifer_ref_mass) + p.wetwat) + trc_wresi_sum;

        if nonvolatile
            && (input.wa_bef > TRC_WATER_MIN_FOR_RATIO || input.wa > TRC_WATER_MIN_FOR_RATIO)
            && p.subsurface_residue > TRC_TINY
        {
            pool_tracer += p.subsurface_residue;
            p.subsurface_residue = 0.0;
        }

        // 滴灌/漫灌/水田灌溉：从 `waterstorage` 内部转移，不算大气输入。
        if input.qflx_irrig_ground > TRC_TINY {
            let irrig_water = input.qflx_irrig_ground * dt;
            pool_water += irrig_water;
            let mut storage_ratio = r_atm;
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
            let trc_flux = (irrig_water * storage_ratio)
                .max(0.0)
                .min(p.waterstorage.max(0.0));
            pool_tracer = trc_flux + pool_tracer;
            p.waterstorage -= trc_flux;
        }

        if input.snl < 0 {
            if input.split_soilsnow {
                pool_tracer = state.step[itrc]
                    .pg_rain_ground
                    .mul_add(1.0 - input.fsno, pool_tracer);
            }
            pool_tracer = trc_gwat_snow_local + pool_tracer;
            pool_water = gwat_snow_local + pool_water;
        } else {
            pool_tracer =
                (state.step[itrc].pg_rain_ground + pool_tracer) + state.step[itrc].sm_carry;
        }

        // 露与霜：大气输入。
        if q_dew_in + q_frost_in > TRC_TINY {
            let trc_dew_input = q_dew_in * ctx.deposition_ratio_for(layer_temp(1), false);
            let frost_ratio = ctx.deposition_ratio_for(layer_temp(1), true);
            pool_tracer = q_frost_in.mul_add(frost_ratio, pool_tracer + trc_dew_input);
            let acc = &mut state.acc[itrc];
            acc.precip = q_frost_in.mul_add(frost_ratio, acc.precip + trc_dew_input);
            acc.water_precip = (acc.water_precip + q_dew_in) + q_frost_in;
        }

        if nonvolatile
            && (pool_water > TRC_WATER_MIN_FOR_RATIO || tracer.has_dissolved_limit())
            && p.surface_residue > TRC_TINY
        {
            pool_tracer = p.surface_residue + pool_tracer;
            p.surface_residue = 0.0;
        }
        tracer.equilibrate_dissolved(pool_water, &mut pool_tracer, &mut p.surface_solid);
        check(
            pool_tracer - aquifer_ref_mass,
            pool_water - aquifer_ref_water,
            "wetland mixed pool",
        )?;

        // 4) 蒸发/升华按大气交换的签名离开，蒸腾按池比值（分馏时经叶片 NSS）。
        let loss_water = (q_evap_out + q_subl_out) + q_etr_out;
        let pool_ratio = if pool_water.abs() > TRC_WATER_MIN_FOR_RATIO {
            pool_tracer / pool_water
        } else {
            r_atm
        };
        let fractionate_pool_loss =
            ctx.active && pool_water > TRC_WATER_MIN_FOR_RATIO && pool_tracer > TRC_TINY;
        let (trc_loss, trc_evap_subl, trc_etr_loss, transp_output_tracer) = if fractionate_pool_loss
        {
            // 对一个临时的有限池依次扣，单步大量损失也能积分残余水的富集（`:2715-2789`）。
            let mut pool_water_loss = pool_water;
            let mut pool_tracer_loss = pool_tracer;
            let loss_liq_avail = q_evap_out.min(pool_water_loss.max(0.0));
            let mut trc_evap_loss = ctx.atmospheric_loss(
                pool_tracer_loss,
                pool_water_loss,
                loss_liq_avail,
                layer_temp(1),
                false,
            );
            if q_evap_out > loss_liq_avail {
                trc_evap_loss = (q_evap_out - loss_liq_avail).mul_add(pool_ratio, trc_evap_loss);
            }
            pool_tracer_loss -= trc_evap_loss;
            pool_water_loss -= q_evap_out;
            let mut pool_ratio_loss = if pool_water_loss > TRC_WATER_MIN_FOR_RATIO {
                pool_tracer_loss / pool_water_loss
            } else {
                pool_ratio
            };
            let loss_ice_avail = q_subl_out.min(pool_water_loss.max(0.0));
            let mut trc_subl_loss = ctx.atmospheric_loss(
                pool_tracer_loss,
                pool_water_loss,
                loss_ice_avail,
                layer_temp(1),
                true,
            );
            if q_subl_out > loss_ice_avail {
                trc_subl_loss =
                    (q_subl_out - loss_ice_avail).mul_add(pool_ratio_loss, trc_subl_loss);
            }
            pool_tracer_loss -= trc_subl_loss;
            pool_water_loss -= q_subl_out;
            pool_ratio_loss = if pool_water_loss > TRC_WATER_MIN_FOR_RATIO {
                pool_tracer_loss / pool_water_loss
            } else {
                pool_ratio
            };
            let transp_source_tracer = q_etr_out * pool_ratio_loss;
            let mut transp_output_tracer = transp_source_tracer;
            if let (true, Some(tleaf), Some(forc_q), Some(forc_psrf), Some(lai), Some(rst)) = (
                q_etr_out > TRC_TINY,
                input.tleaf,
                input.forc_q,
                input.forc_psrf,
                input.lai,
                input.rst,
            ) {
                let ra = input.ra.map_or(0.0, |ra| ra.max(0.0));
                let has_vapor = input.has_vapor.is_some_and(|flags| flags[itrc]);
                let vapor = if has_vapor {
                    r_atm
                } else {
                    pool_ratio_loss / physics.alpha_liq_vap(tracer, tleaf).max(TRC_TINY)
                };
                let relhum_leaf =
                    crate::tracer::frac::surface_relhum(forc_q, forc_psrf, tleaf, false);
                let out = physics.transpiration_nss_ratio(
                    tracer,
                    &crate::tracer::frac::NssInput {
                        source_ratio: pool_ratio_loss,
                        vapor_ratio: vapor,
                        temp_k: tleaf,
                        relhum: relhum_leaf,
                        psrf: forc_psrf,
                        transp_water: q_etr_out,
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
                p.leaf_delta_e = out.new_delta_e;
                p.leaf_delta_b = out.new_delta_b;
                p.leaf_peclet = out.new_peclet;
                p.leaf_water_moles = out.new_leaf_moles;
                transp_output_tracer = q_etr_out * out.trans_ratio;
                p.leaf_iso_storage =
                    (p.leaf_iso_storage + transp_source_tracer) - transp_output_tracer;
            }
            (
                (trc_evap_loss + trc_subl_loss) + transp_source_tracer,
                trc_evap_loss + trc_subl_loss,
                transp_source_tracer,
                transp_output_tracer,
            )
        } else if nonvolatile {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            let trc_subl_loss = pool_ratio * q_subl_out;
            let trc_etr_loss = pool_ratio * q_etr_out;
            (
                pool_ratio * loss_water,
                pool_ratio.mul_add(q_evap_out, trc_subl_loss),
                trc_etr_loss,
                trc_etr_loss,
            )
        };
        let atmos_water = q_evap_out + q_subl_out;
        if atmos_water > TRC_TINY || trc_evap_subl.abs() > TRC_TINY {
            state.book_evap_loss(itrc, trc_evap_subl, atmos_water, EvapKind::Wetland);
        }
        if q_etr_out > TRC_TINY || transp_output_tracer.abs() > TRC_TINY {
            state.book_evap_loss(
                itrc,
                transp_output_tracer,
                q_etr_out,
                EvapKind::Transpiration,
            );
            state.acc[itrc].transp_src += trc_etr_loss;
        }
        pool_tracer -= trc_loss;
        pool_water -= loss_water;
        tracer.equilibrate_dissolved(pool_water, &mut pool_tracer, &mut p.surface_solid);
        check(
            pool_tracer - aquifer_ref_mass,
            pool_water - aquifer_ref_water,
            "wetland after loss",
        )?;

        // 5) 按 WATER 之后的 wetwat/wdsrf/wa/rsur 重新分配。
        let redist_target;
        if pool_tracer < 0.0 {
            // 亏欠只记在带符号的含水层上。
            p.wetwat = 0.0;
            p.wdsrf = 0.0;
            p.wa = pool_tracer;
            redist_target = pool_tracer;
        } else {
            let ratio = if pool_water > TRC_WATER_MIN_FOR_RATIO {
                pool_tracer / pool_water
            } else if pool_tracer <= TRC_TINY {
                0.0
            } else {
                if nonvolatile {
                    p.surface_residue += pool_tracer;
                } else {
                    state.step[itrc].numerical_residual_step -= pool_tracer;
                }
                pool_tracer = 0.0;
                0.0
            };
            p.wetwat = input.wetwat * ratio;
            p.wdsrf = input.wdsrf * ratio;
            p.wa =
                aquifer_actual_water(input.wa, aquifer_ref_water).mul_add(ratio, -aquifer_ref_mass);
            if input.rsur > TRC_TINY {
                let rsur_water = input.rsur * dt;
                let acc = &mut state.acc[itrc];
                acc.rsur = rsur_water.mul_add(ratio, acc.rsur);
                acc.rnof = rsur_water.mul_add(ratio, acc.rnof);
                let step = &mut state.step[itrc];
                step.rnof_step = rsur_water.mul_add(ratio, step.rnof_step);
                redist_target = (-rsur_water).mul_add(ratio, pool_tracer);
            } else {
                redist_target = pool_tracer;
            }
        }

        // 求解器留下的微小水量失配：保持在存量里，不进收支。
        let redist_storage = ((p.wetwat + p.wdsrf) + p.wa) + aquifer_ref_mass;
        let mut redist_resid = redist_storage - redist_target;
        if redist_resid.abs() > TRC_TINY {
            if redist_resid > 0.0 {
                let fix = redist_resid.min(p.wetwat);
                p.wetwat -= fix;
                redist_resid -= fix;
                let fix = redist_resid.min(p.wdsrf);
                p.wdsrf -= fix;
                redist_resid -= fix;
                p.wa -= redist_resid;
            } else if input.wetwat > TRC_TINY {
                p.wetwat -= redist_resid;
            } else if input.wdsrf > TRC_TINY {
                p.wdsrf -= redist_resid;
            } else {
                p.wa -= redist_resid;
            }
        }
        check(p.wa, input.wa, "wetland end")?;
        for j in lb..=nl {
            let slot = soisno_slot(j);
            tracer.equilibrate_dissolved(
                input.wliq_soisno[slot].max(0.0),
                &mut p.wliq_soisno[slot],
                &mut p.solid_soisno[slot],
            );
        }
        if let Some(storage) = input.waterstorage_patch {
            tracer.equilibrate_dissolved(
                (-input.qflx_irrig_ground.max(0.0))
                    .mul_add(dt, storage)
                    .max(0.0),
                &mut p.waterstorage,
                &mut p.waterstorage_solid,
            );
        }
        state.pools[itrc] = p;
    }
    Ok(())
}
