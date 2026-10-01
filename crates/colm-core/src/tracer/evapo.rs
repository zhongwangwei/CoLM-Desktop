//! THERMAL 之后的蒸散示踪物（`MOD_Tracer_Evapo`）。
//!
//! [`tracer_evapo`] 比较 THERMAL 前后的冠层与雪+土各层水量：减少的是蒸发/升华（按
//! 有限水池损失离开，记进 `a_trc_evap` 及分类累加器），增加的是露/霜（带水汽比值进入，
//! 记进 `a_trc_precip`）；冠层与各层的融化/冻结是内部转移，先按 THERMAL 报告的相变
//! 质量搬运示踪物。雪层（`j < 1`）只做相变，大气交换由后面的 `snowwater` 镜像；
//! 深层（`j > 1`）的残差不是大气通量，记成数值残差。
//! [`tracer_flood_evap_loss`] 是洪水水池的蒸发损失（不进陆面 `a_trc_evap`）。
//! [`tracer_snow_melt_carry`] 是 CoLMMAIN 紧随其后的无雪层 `scv` 融化携带。
//!
//! 分馏（`tracer_fractionation_active`）时蒸发按 Craig-Gordon、凝结按平衡（冰面 JM84 有效）
//! 比值、冻结按 Rayleigh，湿叶另与环境水汽做平衡交换（[`super::frac`]）。
//!
//! 收缩形状取自 `MOD_Tracer_Evapo.F90` 的 GIMPLE：
//! * 冠层露/霜：`trc_flux = d_external * ratio` 单独舍入后两处相加（不收缩）；
//! * 第 1 层露/霜：`trc_wliq = FMA(trc_flux, ratio, trc_wliq)`、
//!   `a_trc_precip = FMA(trc_flux, ratio, a_trc_precip)`（`trc_wice` 同）；
//! * `r_max = ref_ratio * (1 + 2000/1000)` 折成 `ref_ratio * 3.0`；
//! * 洪水蒸发的风速 `sqrt(FMA(us, us, vs*vs))`（分馏路径）；
//! * 其余相加相减均按源码从左到右独立舍入。
//!
//! CoLMMAIN 的 `trc_sm_carry` 块：`max(scv_bef, trc_tiny)` 在该分支里被折成 `scv_bef`
//! （数值相同），`sm_carry = trc_scv * (1 - ratio)`、`trc_scv = trc_scv * ratio`，无收缩。

#![allow(clippy::manual_clamp)]


use anyhow::Result;

use super::evap_limit::{
    atmospheric_tracer_loss, evaporative_tracer_loss, skin_limited_tracer_loss,
};
use super::{
    soisno_slot, EvapKind, PatchTracerState, TracerDescriptor, TracerPhysics, TracerSet,
    SOIL_LAYERS, TRC_TINY,
};

/// `trc_delta_sanity_max` [‰]。
const DELTA_SANITY_MAX: f64 = 2.0e3;
/// 未给温度时的默认值（`canopy_temp`/`layer_temp`）。
const DEFAULT_TEMP_K: f64 = 273.15;

/// `tracer_evapo` 的实参（一个 patch）。雪+土各层数组按 Fortran 层下标 `-4..=10` 存放，
/// 只读 `snl+1..=nl_soil`。
#[derive(Debug, Clone, Copy)]
pub struct EvapoInput<'a> {
    /// `deltim`（上游不参与计算）。
    pub deltim: f64,
    /// `snl`（THERMAL 之后，THERMAL 不改层数）。
    pub snl: i32,
    /// THERMAL 之后的 `ldew_rain`/`ldew_snow`。
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    /// THERMAL 之前的 `ldew_rain_bef_th`/`ldew_snow_bef_th`。
    pub ldew_rain_bef: f64,
    pub ldew_snow_bef: f64,
    /// THERMAL 之后的 `wliq_soisno`/`wice_soisno`。
    pub wliq_soisno: &'a [f64; super::SOISNO_LAYERS],
    pub wice_soisno: &'a [f64; super::SOISNO_LAYERS],
    /// THERMAL 之前的 `wliq_soisno_old_trc`/`wice_soisno_old_trc`。
    pub wliq_soisno_bef: &'a [f64; super::SOISNO_LAYERS],
    pub wice_soisno_bef: &'a [f64; super::SOISNO_LAYERS],
    /// `canopy_smelt_mass_th`/`canopy_frzc_mass_th`（可选；两者都缺时用符号启发式）。
    pub canopy_smelt_mass: Option<f64>,
    pub canopy_frzc_mass: Option<f64>,
    /// `soil_thaw_mass_th`/`soil_frzc_mass_th`（可选；两者都缺时用符号启发式）。
    pub soil_thaw_mass: Option<&'a [f64; super::SOISNO_LAYERS]>,
    pub soil_frzc_mass: Option<&'a [f64; super::SOISNO_LAYERS]>,
    /// `tleaf_frac`（缺省 273.15 K）。
    pub tleaf: Option<f64>,
    /// `t_soisno_frac`（缺省 273.15 K）。
    pub t_soisno: Option<&'a [f64; super::SOISNO_LAYERS]>,
    /// `forc_q_frac`/`forc_psrf_frac`：任一缺失时蒸发比值不分馏。
    pub forc_q: Option<f64>,
    pub forc_psrf: Option<f64>,
    /// `DEF_TRACER_SUBL_SKIN_MM`。
    pub subl_skin_mm: f64,
    /// `DEF_TRACER_CANOPY_EQUILIBRATION`。
    pub canopy_equilibration: f64,
}

/// `tracer_evapo`。`vapor_ratio[itrc]` 是本步本 patch 的水汽比值
/// （`tracer_forcing_vapor_value`）。
pub fn tracer_evapo(
    set: &TracerSet,
    state: &mut PatchTracerState,
    physics: TracerPhysics,
    vapor_ratio: &[f64],
    input: &EvapoInput<'_>,
) -> Result<()> {
    if set.is_empty() {
        return Ok(());
    }
    let lb = input.snl + 1;
    let canopy_temp = input.tleaf.unwrap_or(DEFAULT_TEMP_K);
    let layer_temp = |j: i32| match input.t_soisno {
        Some(t) => t[soisno_slot(j)],
        None => DEFAULT_TEMP_K,
    };

    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let r_atm = vapor_ratio[itrc];
        let frac = Frac {
            tracer,
            physics,
            input,
            r_atm,
        };

        let d_rain = input.ldew_rain - input.ldew_rain_bef;
        let d_snow = input.ldew_snow - input.ldew_snow_bef;

        let mut thaw_amt = 0.0;
        let mut freeze_amt = 0.0;
        if input.canopy_smelt_mass.is_some() || input.canopy_frzc_mass.is_some() {
            if let Some(m) = input.canopy_smelt_mass {
                thaw_amt = m.max(0.0);
            }
            if let Some(m) = input.canopy_frzc_mass {
                freeze_amt = m.max(0.0);
            }
        } else {
            // 旧的符号启发式。
            if d_snow < -TRC_TINY && d_rain > TRC_TINY {
                thaw_amt = d_snow.abs().min(d_rain);
            }
            if d_rain < -TRC_TINY && d_snow > TRC_TINY {
                freeze_amt = d_rain.abs().min(d_snow);
            }
        }

        let d_rain_external = d_rain - thaw_amt + freeze_amt;
        let d_snow_external = d_snow + thaw_amt - freeze_amt;

        // 冠层雨：蒸发/露。
        if d_rain_external < -TRC_TINY {
            if input.ldew_rain_bef > TRC_TINY {
                let water_loss = d_rain_external.abs();
                let trc_flux = frac.evaporative_loss(
                    state.pools[itrc].ldew_rain,
                    input.ldew_rain_bef,
                    water_loss,
                    canopy_temp,
                    false,
                )?;
                state.pools[itrc].ldew_rain -= trc_flux;
                state.book_evap_loss(itrc, trc_flux, water_loss, EvapKind::CanopyEvaporation);
            }
        } else if d_rain_external > TRC_TINY {
            let ratio = frac.deposition_ratio(canopy_temp, false)?;
            let trc_flux = d_rain_external * ratio;
            state.pools[itrc].ldew_rain += trc_flux;
            state.acc[itrc].precip += trc_flux;
            state.acc[itrc].water_precip += d_rain_external;
        }
        state.pools[itrc].ldew_rain = state.pools[itrc].ldew_rain.max(0.0);

        // 冠层雪：升华/霜。
        if d_snow_external < -TRC_TINY {
            if input.ldew_snow_bef > TRC_TINY {
                let water_loss = d_snow_external.abs();
                let trc_flux = frac.evaporative_loss(
                    state.pools[itrc].ldew_snow,
                    input.ldew_snow_bef,
                    water_loss,
                    canopy_temp,
                    true,
                )?;
                state.pools[itrc].ldew_snow -= trc_flux;
                state.book_evap_loss(itrc, trc_flux, water_loss, EvapKind::Sublimation);
            }
        } else if d_snow_external > TRC_TINY {
            let ratio = frac.deposition_ratio(canopy_temp, true)?;
            let trc_flux = d_snow_external * ratio;
            state.pools[itrc].ldew_snow += trc_flux;
            state.acc[itrc].precip += trc_flux;
            state.acc[itrc].water_precip += d_snow_external;
        }
        state.pools[itrc].ldew_snow = state.pools[itrc].ldew_snow.max(0.0);

        // 外部交换之后的相变载体。
        let ldew_rain_pre_phase = (input.ldew_rain_bef + d_rain_external).max(0.0);
        let ldew_snow_pre_phase = (input.ldew_snow_bef + d_snow_external).max(0.0);
        let thaw_amt = thaw_amt.min(ldew_snow_pre_phase);
        let freeze_amt = freeze_amt.min(ldew_rain_pre_phase + thaw_amt);

        let pools = &mut state.pools[itrc];
        // 内部融化：雪 → 雨。
        if thaw_amt > TRC_TINY && ldew_snow_pre_phase > TRC_TINY {
            let ratio = pools.ldew_snow / ldew_snow_pre_phase;
            let trc_flux = (thaw_amt * ratio).min(pools.ldew_snow.max(0.0));
            pools.ldew_snow -= trc_flux;
            pools.ldew_rain += trc_flux;
        }
        // 内部冻结：雨 → 雪。
        if freeze_amt > TRC_TINY && ldew_rain_pre_phase + thaw_amt > TRC_TINY {
            let trc_flux = frac.rayleigh_freezing_loss(
                pools.ldew_rain,
                ldew_rain_pre_phase + thaw_amt,
                freeze_amt,
                canopy_temp,
            );
            pools.ldew_rain -= trc_flux;
            pools.ldew_snow += trc_flux;
        }

        tracer.equilibrate_dissolved(
            input.ldew_rain.max(0.0),
            &mut pools.ldew_rain,
            &mut pools.canopy_solid,
        );

        // 湿叶与环境水汽的双向平衡交换（净水通量为零、示踪物通量非零，记进单独的
        // `a_trc_vapor_exchange`；只对液相，`:248-262`）。
        if input.canopy_equilibration > 0.0
            && physics.fractionation_active(tracer)
            && !tracer.is_nonvolatile_solute()
            && input.ldew_rain > TRC_TINY
        {
            let pools = &mut state.pools[itrc];
            let equil_gain = super::frac::equilibration_exchange(
                pools.ldew_rain,
                input.ldew_rain.max(0.0),
                frac.r_atm,
                physics.alpha_liq_vap(tracer, canopy_temp),
                input.canopy_equilibration,
            );
            pools.ldew_rain = (pools.ldew_rain + equil_gain).max(0.0);
            state.acc[itrc].vapor_exchange += equil_gain;
        }

        // 雪+土各层：液+冰。
        for j in lb..=SOIL_LAYERS as i32 {
            let s = soisno_slot(j);
            let wliq_bef = input.wliq_soisno_bef[s];
            let wice_bef = input.wice_soisno_bef[s];
            let d_wliq = input.wliq_soisno[s] - wliq_bef;
            let d_wice = input.wice_soisno[s] - wice_bef;

            let mut thaw_amt = 0.0;
            let mut freeze_amt = 0.0;
            if input.soil_thaw_mass.is_some() || input.soil_frzc_mass.is_some() {
                if let Some(m) = input.soil_thaw_mass {
                    thaw_amt = m[s].max(0.0).min(wice_bef.max(0.0));
                }
                if let Some(m) = input.soil_frzc_mass {
                    freeze_amt = m[s].max(0.0).min(wliq_bef.max(0.0) + thaw_amt);
                }
            } else {
                if d_wice < -TRC_TINY && d_wliq > TRC_TINY {
                    thaw_amt = d_wice.abs().min(d_wliq);
                }
                if d_wliq < -TRC_TINY && d_wice > TRC_TINY {
                    freeze_amt = d_wliq.abs().min(d_wice);
                }
            }

            let pools = &mut state.pools[itrc];
            // 融化：冰 → 液。
            if thaw_amt > TRC_TINY && wice_bef > TRC_TINY {
                let ratio = pools.wice_soisno[s] / wice_bef;
                let trc_flux = (thaw_amt * ratio).min(pools.wice_soisno[s].max(0.0));
                pools.wice_soisno[s] -= trc_flux;
                pools.wliq_soisno[s] += trc_flux;
            }
            // 冻结：液 → 冰，分母是冻结前（含刚融化）的液态水。
            if freeze_amt > TRC_TINY && wliq_bef + thaw_amt > TRC_TINY {
                let trc_flux = frac.rayleigh_freezing_loss(
                    pools.wliq_soisno[s],
                    wliq_bef + thaw_amt,
                    freeze_amt,
                    layer_temp(j),
                );
                pools.wliq_soisno[s] -= trc_flux;
                pools.wice_soisno[s] += trc_flux;
            }

            let wliq_post_phase = (wliq_bef + thaw_amt - freeze_amt).max(0.0);
            let wice_post_phase = (wice_bef - thaw_amt + freeze_amt).max(0.0);

            // 雪层的大气交换由 snowwater 之后镜像，这里只做相变。
            if j < 1 {
                tracer.equilibrate_dissolved(
                    input.wliq_soisno[s].max(0.0),
                    &mut pools.wliq_soisno[s],
                    &mut pools.solid_soisno[s],
                );
                continue;
            }

            // 液态净变化（扣除相变）= 蒸发/露。
            let trc_flux = d_wliq - thaw_amt + freeze_amt;
            if trc_flux < -TRC_TINY {
                if j == 1 {
                    if wliq_post_phase > TRC_TINY {
                        let water_loss = trc_flux.abs();
                        let loss = frac.evaporative_loss(
                            state.pools[itrc].wliq_soisno[s],
                            wliq_post_phase,
                            water_loss,
                            layer_temp(j),
                            false,
                        )?;
                        state.pools[itrc].wliq_soisno[s] -= loss;
                        state.book_evap_loss(itrc, loss, water_loss, EvapKind::SoilEvaporation);
                    }
                } else {
                    // 深层残差损失：不是大气蒸发。
                    let pools = &mut state.pools[itrc];
                    let ratio = if wliq_post_phase > TRC_TINY {
                        pools.wliq_soisno[s] / wliq_post_phase
                    } else {
                        tracer.init_water_ratio()
                    };
                    let trc_resid =
                        (trc_flux.abs() * ratio.max(0.0)).min(pools.wliq_soisno[s].max(0.0));
                    pools.wliq_soisno[s] -= trc_resid;
                    state.step[itrc].numerical_residual_step -= trc_resid;
                }
            } else if trc_flux > TRC_TINY {
                if j == 1 {
                    let ratio = frac.deposition_ratio(layer_temp(j), false)?;
                    let pools = &mut state.pools[itrc];
                    pools.wliq_soisno[s] = trc_flux.mul_add(ratio, pools.wliq_soisno[s]);
                    let acc = &mut state.acc[itrc];
                    acc.precip = trc_flux.mul_add(ratio, acc.precip);
                    acc.water_precip += trc_flux;
                } else {
                    // 深层残差增益：保留本层（或初始）比值。
                    let pools = &mut state.pools[itrc];
                    let ratio = if wliq_post_phase > TRC_TINY {
                        pools.wliq_soisno[s] / wliq_post_phase
                    } else {
                        tracer.init_water_ratio()
                    };
                    let trc_resid = trc_flux * ratio.max(0.0);
                    pools.wliq_soisno[s] += trc_resid;
                    state.step[itrc].numerical_residual_step += trc_resid;
                }
            }

            // 冰的净变化（扣除相变）= 升华/霜。
            let trc_flux = d_wice + thaw_amt - freeze_amt;
            if trc_flux < -TRC_TINY {
                if j == 1 {
                    if wice_post_phase > TRC_TINY {
                        let water_loss = trc_flux.abs();
                        let loss = frac.evaporative_loss(
                            state.pools[itrc].wice_soisno[s],
                            wice_post_phase,
                            water_loss,
                            layer_temp(j),
                            true,
                        )?;
                        state.pools[itrc].wice_soisno[s] -= loss;
                        state.book_evap_loss(itrc, loss, water_loss, EvapKind::Sublimation);
                    }
                } else {
                    let pools = &mut state.pools[itrc];
                    let ratio = if wice_post_phase > TRC_TINY {
                        pools.wice_soisno[s] / wice_post_phase
                    } else {
                        tracer.init_water_ratio()
                    };
                    let trc_resid =
                        (trc_flux.abs() * ratio.max(0.0)).min(pools.wice_soisno[s].max(0.0));
                    pools.wice_soisno[s] -= trc_resid;
                    state.step[itrc].numerical_residual_step -= trc_resid;
                }
            } else if trc_flux > TRC_TINY {
                if j == 1 {
                    let ratio = frac.deposition_ratio(layer_temp(j), true)?;
                    let pools = &mut state.pools[itrc];
                    pools.wice_soisno[s] = trc_flux.mul_add(ratio, pools.wice_soisno[s]);
                    let acc = &mut state.acc[itrc];
                    acc.precip = trc_flux.mul_add(ratio, acc.precip);
                    acc.water_precip += trc_flux;
                } else {
                    let pools = &mut state.pools[itrc];
                    let ratio = if wice_post_phase > TRC_TINY {
                        pools.wice_soisno[s] / wice_post_phase
                    } else {
                        tracer.init_water_ratio()
                    };
                    let trc_resid = trc_flux * ratio.max(0.0);
                    pools.wice_soisno[s] += trc_resid;
                    state.step[itrc].numerical_residual_step += trc_resid;
                }
            }
            let pools = &mut state.pools[itrc];
            tracer.equilibrate_dissolved(
                input.wliq_soisno[s].max(0.0),
                &mut pools.wliq_soisno[s],
                &mut pools.solid_soisno[s],
            );
        }
    }
    Ok(())
}

/// `tracer_flood_evap_loss` 的标量实参。
#[derive(Debug, Clone, Copy, Default)]
pub struct FloodEvapInput {
    /// `water_credit`（洪水水池可蒸发水量 [mm]）。
    pub water_credit: f64,
    /// `water_evap`（本步洪水蒸发 [mm]）。
    pub water_evap: f64,
    pub temp_k: f64,
    pub forc_q: f64,
    pub forc_psrf: f64,
    pub forc_us: f64,
    pub forc_vs: f64,
}

/// `tracer_flood_evap_loss`：返回逐示踪物的 `tracer_loss`（不走通用输运与非挥发溶质为 0）。
/// `tracer_credit[itrc]` 是洪水水池里的示踪物。
pub fn tracer_flood_evap_loss(
    set: &TracerSet,
    physics: TracerPhysics,
    vapor_ratio: &[f64],
    tracer_credit: &[f64],
    input: &FloodEvapInput,
) -> Result<Vec<f64>> {
    let mut tracer_loss = vec![0.0; set.len()];
    if input.water_credit <= TRC_TINY || input.water_evap <= TRC_TINY {
        return Ok(tracer_loss);
    }
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() || tracer.is_nonvolatile_solute() {
            continue;
        }
        let active = physics.fractionation_active(tracer);
        // Craig-Gordon 比值用开阔水面动力学 α，风速 `sqrt(FMA(us, us, vs*vs))`。
        let wind = input.forc_us.mul_add(input.forc_us, input.forc_vs * input.forc_vs).sqrt();
        let evap_ratio = |source_ratio: f64, temp: f64, from_ice: bool| {
            if !active {
                return source_ratio;
            }
            let relhum = super::frac::surface_relhum(input.forc_q, input.forc_psrf, temp, from_ice);
            let alpha_k = physics.alpha_kinetic_open_water(tracer, wind);
            physics.craig_gordon_evap_ratio(
                tracer,
                source_ratio,
                vapor_ratio[itrc],
                temp,
                relhum,
                alpha_k,
                from_ice,
            )
        };
        let loss = atmospheric_tracer_loss(
            tracer_credit[itrc],
            input.water_credit,
            input.water_evap,
            input.temp_k,
            false,
            &evap_ratio,
            TRC_TINY,
            r_max(tracer, active),
            false,
            None,
        );
        tracer_loss[itrc] = loss;
    }
    Ok(tracer_loss)
}

/// CoLMMAIN 在 `tracer_evapo` 之后的无雪层 `scv` 融化携带：`scv` 减少的那部分示踪物
/// 进 `trc_sm_carry`（之后交给土壤水），有雪层时清零。
pub fn tracer_snow_melt_carry(
    set: &TracerSet,
    state: &mut PatchTracerState,
    snl: i32,
    scv: f64,
    scv_bef: f64,
) {
    let transport = set.transport_indices();
    if snl == 0 {
        if scv < TRC_TINY {
            for itrc in transport {
                state.step[itrc].sm_carry = state.pools[itrc].scv;
                state.pools[itrc].scv = 0.0;
            }
        } else if scv < scv_bef - TRC_TINY {
            let ratio = scv / scv_bef.max(TRC_TINY);
            let ratio = ratio.min(1.0).max(0.0);
            for itrc in transport {
                let trc_scv = state.pools[itrc].scv;
                state.step[itrc].sm_carry = trc_scv * (1.0 - ratio);
                state.pools[itrc].scv = trc_scv * ratio;
            }
        } else {
            for itrc in transport {
                state.step[itrc].sm_carry = 0.0;
            }
        }
    } else {
        for itrc in transport {
            state.step[itrc].sm_carry = 0.0;
        }
    }
}

/// `merge(ref_ratio*(1+trc_delta_sanity_max/1000), 0, tracer_fractionation_active(itrc))`。
fn r_max(tracer: &TracerDescriptor, active: bool) -> f64 {
    if active {
        tracer.ref_ratio * (1.0 + DELTA_SANITY_MAX / 1000.0)
    } else {
        0.0
    }
}

/// `tracer_evapo` 的内部函数（`evaporative_tracer_loss`、`deposition_ratio_for`）与
/// 非分馏的 Rayleigh 冻结。
struct Frac<'a, 'b> {
    tracer: &'a TracerDescriptor,
    physics: TracerPhysics,
    input: &'a EvapoInput<'b>,
    r_atm: f64,
}

impl Frac<'_, '_> {
    fn active(&self) -> bool {
        self.physics.fractionation_active(self.tracer)
    }

    /// 内部函数 `evaporative_tracer_loss`：非挥发溶质不离开；升华走表层限制。
    fn evaporative_loss(
        &self,
        pool_trc: f64,
        pool_water: f64,
        water_loss: f64,
        temp_k: f64,
        from_ice: bool,
    ) -> Result<f64> {
        if self.tracer.is_nonvolatile_solute() {
            return Ok(0.0);
        }
        let active = self.active();
        let evap_ratio = |source_ratio: f64, temp: f64, ice: bool| self.evap_ratio_for(source_ratio, temp, ice);
        let r_max = r_max(self.tracer, active);
        Ok(if from_ice {
            skin_limited_tracer_loss(
                pool_trc,
                pool_water,
                water_loss,
                self.input.subl_skin_mm,
                temp_k,
                from_ice,
                &evap_ratio,
                TRC_TINY,
                r_max,
            )
        } else {
            evaporative_tracer_loss(
                pool_trc,
                pool_water,
                water_loss,
                temp_k,
                from_ice,
                &evap_ratio,
                TRC_TINY,
                r_max,
            )
        })
    }

    /// 内部函数 `evap_ratio_for`：冠层与地表都用 `tracer_alpha_kinetic_craig_gordon`。
    fn evap_ratio_for(&self, source_ratio: f64, temp_k: f64, from_ice: bool) -> f64 {
        if !self.active() {
            return source_ratio;
        }
        let (Some(forc_q), Some(forc_psrf)) = (self.input.forc_q, self.input.forc_psrf) else {
            return source_ratio;
        };
        let relhum = super::frac::surface_relhum(forc_q, forc_psrf, temp_k, from_ice);
        let alpha_k = self.physics.alpha_kinetic_craig_gordon(self.tracer, from_ice);
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

    /// 内部函数 `deposition_ratio_for`。
    fn deposition_ratio(&self, temp_k: f64, from_ice: bool) -> Result<f64> {
        if self.tracer.is_nonvolatile_solute() {
            return Ok(0.0);
        }
        if !self.active() {
            return Ok(self.r_atm);
        }
        Ok(self
            .physics
            .equilibrium_deposition_ratio(self.tracer, self.r_atm, temp_k, from_ice))
    }

    /// `tracer_rayleigh_freezing_loss`。
    fn rayleigh_freezing_loss(&self, pool_trc: f64, pool_water: f64, freeze_water: f64, temp_k: f64) -> f64 {
        self.physics
            .rayleigh_freezing_loss(self.tracer, pool_trc, pool_water, freeze_water, temp_k)
    }
}

#[cfg(test)]
#[path = "evapo_tests.rs"]
mod tests;
