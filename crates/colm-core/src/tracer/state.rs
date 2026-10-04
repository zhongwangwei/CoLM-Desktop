//! 每个 patch 的示踪物状态（`MOD_Tracer_Vars`）。
//!
//! 上游是 `(ntracers, numpatch)` 的模块数组；这里每个 patch 一份 [`PatchTracerState`]，
//! 其中逐示踪物（与 [`super::TracerSet::tracers`] 同序，含不走通用输运的）各一份
//! [`TracerPools`]、[`TracerStep`] 与 [`TracerAccumulators`]。雪+土各层按 Fortran 下标
//! `maxsnl+1..=nl_soil`（`-4..=10`）连续存放，用 [`soisno_slot`] 换算。

use super::TracerSet;

/// `maxsnl`。
pub const MAX_SNOW_LAYERS: usize = 5;
/// `nl_soil`。
pub const SOIL_LAYERS: usize = 10;
/// `maxsnl+1..=nl_soil` 的层数。
pub const SOISNO_LAYERS: usize = MAX_SNOW_LAYERS + SOIL_LAYERS;

/// Fortran 层下标 `j`（`-4..=10`）在 15 层数组里的位置。
pub const fn soisno_slot(j: i32) -> usize {
    (j + MAX_SNOW_LAYERS as i32 - 1) as usize
}

/// 一个示踪物的预报量（`trc_*`，重启里存的那些）。
#[derive(Debug, Clone, PartialEq)]
pub struct TracerPools {
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    pub wliq_soisno: [f64; SOISNO_LAYERS],
    pub wice_soisno: [f64; SOISNO_LAYERS],
    pub solid_soisno: [f64; SOISNO_LAYERS],
    pub wa: f64,
    pub aquifer_ref_mass: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub surface_residue: f64,
    pub subsurface_residue: f64,
    pub canopy_solid: f64,
    pub surface_solid: f64,
    pub subsurface_solid: f64,
    pub waterstorage_solid: f64,
    pub scv: f64,
    pub waterstorage: f64,
    pub leaf_delta_e: f64,
    pub leaf_delta_b: f64,
    pub leaf_peclet: f64,
    pub leaf_water_moles: f64,
    pub leaf_iso_storage: f64,
}

impl TracerPools {
    /// `allocate_Tracer_Vars` 的初值：全 0，`leaf_peclet = 1`，叶水 δ 取 `init_delta`。
    pub fn allocated(init_delta: f64) -> Self {
        Self {
            ldew_rain: 0.0,
            ldew_snow: 0.0,
            wliq_soisno: [0.0; SOISNO_LAYERS],
            wice_soisno: [0.0; SOISNO_LAYERS],
            solid_soisno: [0.0; SOISNO_LAYERS],
            wa: 0.0,
            aquifer_ref_mass: 0.0,
            wdsrf: 0.0,
            wetwat: 0.0,
            surface_residue: 0.0,
            subsurface_residue: 0.0,
            canopy_solid: 0.0,
            surface_solid: 0.0,
            subsurface_solid: 0.0,
            waterstorage_solid: 0.0,
            scv: 0.0,
            waterstorage: 0.0,
            leaf_delta_e: init_delta,
            leaf_delta_b: init_delta,
            leaf_peclet: 1.0,
            leaf_water_moles: 0.0,
            leaf_iso_storage: 0.0,
        }
    }
}

/// 一步之内在各过程间传递的量（`trc_pg_*_ground`、`trc_rnof_step`、`trc_sm_carry`、
/// 收支快照与源汇）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TracerStep {
    pub pg_rain_ground: f64,
    pub pg_snow_ground: f64,
    pub rnof_step: f64,
    pub sm_carry: f64,
    pub storage_beg: f64,
    pub balance_err: f64,
    pub reactive_source_step: f64,
    pub numerical_residual_step: f64,
    pub numerical_water_step: f64,
}

/// 逐示踪物的 history 累加器（`a_trc_*` 与按示踪物分的 `a_water_*`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TracerAccumulators {
    pub precip: f64,
    pub vapor_exchange: f64,
    pub water_precip: f64,
    pub evap: f64,
    pub water_evap_gross: f64,
    pub transp: f64,
    pub transp_src: f64,
    pub water_transp: f64,
    pub soilevap: f64,
    pub water_soilevap: f64,
    pub canopyevap: f64,
    pub water_canopyevap: f64,
    pub subl: f64,
    pub water_subl: f64,
    pub wetland_evap: f64,
    pub water_wetland_evap: f64,
    pub rsur: f64,
    pub rsub: f64,
    pub rnof: f64,
    pub water_rnof: f64,
    pub qinfl: f64,
    pub qcharge: f64,
    pub ldew_mass: f64,
    pub soil_mass: [f64; SOIL_LAYERS],
    pub snow_mass: [f64; MAX_SNOW_LAYERS],
    pub wa_mass: f64,
    pub wa_debt_mass: f64,
    pub aquifer_actual_mass: f64,
    pub wdsrf_mass: f64,
    pub wetwat_mass: f64,
    pub surface_residue_mass: f64,
    pub subsurface_residue_mass: f64,
    pub layer_dry_mass: f64,
    pub solid_mass: f64,
    pub scv_mass: f64,
}

/// 按 patch（不分示踪物）的水量累加器（`a_water_ldew/soil/snow/wa/...`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaterAccumulators {
    pub ldew: f64,
    pub soil: [f64; SOIL_LAYERS],
    pub snow: [f64; MAX_SNOW_LAYERS],
    pub wa: f64,
    pub wa_debt: f64,
    pub aquifer_actual: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub scv: f64,
}

/// 蒸发损失的类别（`TRC_EVAP_KIND_*`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvapKind {
    Total,
    Transpiration,
    SoilEvaporation,
    CanopyEvaporation,
    Sublimation,
    Wetland,
}

/// 一个 patch 的全部示踪物状态。
#[derive(Debug, Clone, PartialEq)]
pub struct PatchTracerState {
    pub pools: Vec<TracerPools>,
    /// `trc_aquifer_ref_water`（每 patch 一个，同位素的含水层参考水量）。
    pub aquifer_ref_water: f64,
    pub step: Vec<TracerStep>,
    pub acc: Vec<TracerAccumulators>,
    pub water_acc: WaterAccumulators,
}

impl PatchTracerState {
    /// 新分配的状态（`allocate_Tracer_Vars`）。
    pub fn allocated(set: &TracerSet) -> Self {
        let n = set.len();
        Self {
            pools: set
                .tracers
                .iter()
                .map(|tracer| {
                    TracerPools::allocated(if tracer.uses_land_water_transport() {
                        tracer.init_delta
                    } else {
                        0.0
                    })
                })
                .collect(),
            aquifer_ref_water: 0.0,
            step: vec![TracerStep::default(); n],
            acc: vec![TracerAccumulators::default(); n],
            water_acc: WaterAccumulators::default(),
        }
    }

    /// `flush_Tracer_Acc`：history 写出后清零累加器与本步临时量。
    pub fn flush_accumulators(&mut self) {
        for acc in &mut self.acc {
            *acc = TracerAccumulators::default();
        }
        self.water_acc = WaterAccumulators::default();
        for step in &mut self.step {
            step.pg_rain_ground = 0.0;
            step.pg_snow_ground = 0.0;
            step.rnof_step = 0.0;
            step.sm_carry = 0.0;
            step.reactive_source_step = 0.0;
            step.numerical_residual_step = 0.0;
            step.numerical_water_step = 0.0;
        }
    }

    /// `tracer_book_evap_loss`。
    pub fn book_evap_loss(
        &mut self,
        itrc: usize,
        tracer_mass: f64,
        water_mass: f64,
        kind: EvapKind,
    ) {
        let acc = &mut self.acc[itrc];
        let water = water_mass.max(0.0);
        acc.evap += tracer_mass;
        acc.water_evap_gross += water;
        let (tracer, water_acc) = match kind {
            EvapKind::Total => return,
            EvapKind::Transpiration => (&mut acc.transp, &mut acc.water_transp),
            EvapKind::SoilEvaporation => (&mut acc.soilevap, &mut acc.water_soilevap),
            EvapKind::CanopyEvaporation => (&mut acc.canopyevap, &mut acc.water_canopyevap),
            EvapKind::Sublimation => (&mut acc.subl, &mut acc.water_subl),
            EvapKind::Wetland => (&mut acc.wetland_evap, &mut acc.water_wetland_evap),
        };
        *tracer += tracer_mass;
        *water_acc += water;
    }
}

/// `tracer_init_from_water` 的水量输入（一个 patch）。
#[derive(Debug, Clone, Copy)]
pub struct WaterInventory<'a> {
    pub patch_type: i32,
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    /// `wliq_soisno(maxsnl+1:nl_soil)`。
    pub wliq_soisno: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno: &'a [f64; SOISNO_LAYERS],
    pub wa: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub scv: f64,
    /// 灌溉打开时的 `waterstorage`。
    pub waterstorage: Option<f64>,
}

/// 冷启动示踪物时的全局开关（`DEF_USE_VariablySaturatedFlow` 与
/// `DEF_TRACER_AQUIFER_MIXING_WATER_MM`）。
#[derive(Debug, Clone, Copy)]
pub struct TracerColdStart {
    pub variably_saturated_flow: bool,
    pub aquifer_mixing_water_mm: f64,
}

impl PatchTracerState {
    /// `tracer_init_from_water`（一个 patch）：每个存储量 = 水量 × 初始比值。
    pub fn init_from_water(
        &mut self,
        set: &TracerSet,
        water: WaterInventory<'_>,
        cold: TracerColdStart,
    ) -> anyhow::Result<()> {
        for pools in &mut self.pools {
            pools.surface_residue = 0.0;
            pools.subsurface_residue = 0.0;
            pools.solid_soisno = [0.0; SOISNO_LAYERS];
            pools.canopy_solid = 0.0;
            pools.surface_solid = 0.0;
            pools.subsurface_solid = 0.0;
            pools.waterstorage_solid = 0.0;
        }
        // 从雪层水量推回 `snl`（自 0 层往上数非空层，遇空即停）。
        let mut snl = 0;
        for j in (-(MAX_SNOW_LAYERS as i32) + 1..=0).rev() {
            let slot = soisno_slot(j);
            if water.wliq_soisno[slot] + water.wice_soisno[slot] > 0.0 {
                snl -= 1;
            } else {
                break;
            }
        }
        for (itrc, tracer) in set.tracers.iter().enumerate() {
            if !tracer.uses_land_water_transport() {
                continue;
            }
            let ratio = tracer.init_water_ratio();
            let pools = &mut self.pools[itrc];
            pools.ldew_rain = water.ldew_rain * ratio;
            pools.ldew_snow = water.ldew_snow * ratio;
            tracer.equilibrate_dissolved(
                water.ldew_rain.max(0.0),
                &mut pools.ldew_rain,
                &mut pools.canopy_solid,
            );
            for slot in 0..SOISNO_LAYERS {
                pools.wliq_soisno[slot] = water.wliq_soisno[slot].max(0.0) * ratio;
                pools.wice_soisno[slot] = water.wice_soisno[slot].max(0.0) * ratio;
                tracer.equilibrate_dissolved(
                    water.wliq_soisno[slot].max(0.0),
                    &mut pools.wliq_soisno[slot],
                    &mut pools.solid_soisno[slot],
                );
            }
            if tracer.is_isotope()
                && cold.variably_saturated_flow
                && cold.aquifer_mixing_water_mm > 0.0
            {
                if water.patch_type == 0 || water.patch_type == 2 {
                    self.aquifer_ref_water = cold.aquifer_mixing_water_mm;
                    pools.aquifer_ref_mass = self.aquifer_ref_water * tracer.ref_ratio;
                    anyhow::ensure!(
                        water.wa + self.aquifer_ref_water >= 0.0,
                        "negative isotope aquifer mixing water at cold start"
                    );
                }
                // GIMPLE `.FMS (wa + ref_water, R_init, ref_mass)`（`MOD_Tracer_Rest`）。
                pools.wa =
                    (water.wa + self.aquifer_ref_water).mul_add(ratio, -pools.aquifer_ref_mass);
            } else {
                pools.wa = water.wa * ratio;
            }
            pools.wdsrf = water.wdsrf.max(0.0) * ratio;
            pools.wetwat = water.wetwat.max(0.0) * ratio;
            tracer.equilibrate_dissolved(
                water.wdsrf.max(0.0),
                &mut pools.wdsrf,
                &mut pools.surface_solid,
            );
            tracer.equilibrate_dissolved(
                water.wetwat.max(0.0),
                &mut pools.wetwat,
                &mut pools.surface_solid,
            );
            tracer.equilibrate_dissolved(water.wa, &mut pools.wa, &mut pools.subsurface_solid);
            pools.scv = if snl == 0 {
                water.scv.max(0.0) * ratio
            } else {
                0.0
            };
            if let Some(storage) = water.waterstorage {
                pools.waterstorage = storage.max(0.0) * ratio;
                tracer.equilibrate_dissolved(
                    storage.max(0.0),
                    &mut pools.waterstorage,
                    &mut pools.waterstorage_solid,
                );
            }
        }
        Ok(())
    }
}
