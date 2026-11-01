//! 一个土壤/湿地 patch 一步之内的材料记账顺序（`CoLMMAIN.F90` 的 `mt_any_enabled` 段）。
//!
//! 宿主在各时刻交出快照，这里按上游顺序调各过程：
//! 截留后 `material_save_storage` + `material_precip` → 新雪 `material_newsnow` → THERMAL 前快照与
//! （关 `DEF_VEG_SNOW` 时）冠层材料按叶温归相 → THERMAL 后 `material_evapo` 与雪融携带 →
//! WATER 后 `material_soil_water`/`material_wetland` → 雪层合并/分裂与冻结重分配。步末的反应、
//! 收支检查与 history 累加要用整柱水量收支，由运行时层调用。

use anyhow::Result;

use super::conservation::{material_save_storage, BalanceSnapshot, SaveStorageInput};
use super::evapo::{material_evapo, material_snow_melt_carry, EvapoInput};
use super::precip::{material_precip, PrecipInput};
use super::snow::{material_newsnow, NewSnowInput};
use super::soil_water::SoilWaterOptions;
use super::{
    MaterialPhysics, MaterialSet, PatchMaterialState, MAX_SNOW_LAYERS, SOIL_LAYERS, SOISNO_LAYERS,
};

/// 一步之内所有过程共享的配置与本步强迫比值。
#[derive(Debug, Clone, Copy)]
pub struct MaterialStepContext<'a> {
    pub set: &'a MaterialSet,
    pub physics: MaterialPhysics,
    pub soil_options: SoilWaterOptions,
    /// `DEF_ISO_CANOPY_EQUILIBRATION`。
    pub canopy_equilibration: f64,
    /// 逐材料的 `material_forcing_rain_value`/`_snow_value`/`_vapor_value`。
    ///
    /// 没有相态回填的物种雨雪两列相同（都等于记住的降水比值），与旧的单列实现逐位一致。
    pub rain_ratio: &'a [f64],
    pub snow_ratio: &'a [f64],
    pub vapor_ratio: &'a [f64],
    /// 逐材料的 `material_forcing_has_vapor`（配置了 vapor 强迫即为真）。
    pub has_vapor: &'a [bool],
    /// `trc_runtime_forced`。
    pub runtime_forced: &'a [bool],
    /// `DEF_USE_CoLMDEBUG`。
    pub debug: bool,
    /// `DEF_VEG_SNOW`。
    pub vegetation_snow: bool,
    /// 网格河湖漫滩回馈发布给本 patch 的可用水量与其中的材料（`flood_credit_patch*1000`、
    /// `flood_material_credit_patch(:,ipatch)`）；没开回馈时为 `None`。
    pub flood: Option<FloodMaterialCredit<'a>>,
}

/// 漫滩回馈发布给一个 patch 的水量（mm）与材料（mm·比值，逐材料）。
#[derive(Debug, Clone, Copy)]
pub struct FloodMaterialCredit<'a> {
    pub water_credit_mm: f64,
    pub material_credit: &'a [f64],
}

/// 本步陆面从漫滩水池取走的材料：蒸发损失（`flood_material_evap_patch`，负值是大气同位素
/// 吸收）与随入渗进土壤的量（`flood_material_land_patch`），交给河道扣账。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FloodMaterialExchange {
    pub evap: Vec<f64>,
    pub land: Vec<f64>,
}

/// 随 patch 保存的材料状态与本步开头的收支快照。
#[derive(Debug, Clone, PartialEq)]
pub struct PatchMaterialTrack {
    pub state: PatchMaterialState,
    pub snapshot: BalanceSnapshot,
    /// 开了漫滩回馈时本步的材料交换（见 [`FloodMaterialExchange`]）。
    pub flood_exchange: Option<FloodMaterialExchange>,
}

impl PatchMaterialTrack {
    pub fn new(state: PatchMaterialState) -> Self {
        Self {
            state,
            snapshot: BalanceSnapshot::default(),
            flood_exchange: None,
        }
    }
}

/// 一步之内的宿主快照（上游的 `*_old_trc`、`*_bef_th`、`scv_bef_trc` 等局部量）。
#[derive(Debug, Clone, Default)]
pub struct MaterialStepScratch {
    /// 截留之前的 `ldew_rain`/`ldew_snow`。
    pub ldew_rain_old: f64,
    pub ldew_snow_old: f64,
    /// 新雪之前的雪层数与雪层冰（`wice_snow_bef_trc`）。
    pub snl_before_new_snow: i32,
    pub wice_snow_before: [f64; MAX_SNOW_LAYERS],
    /// 新雪之后的 `scv`（`scv_bef_trc`，THERMAL 后的雪融携带用）。
    pub scv_after_new_snow: f64,
    /// THERMAL 之前（关 VEG_SNOW 时为归相之后）的冠层水。
    pub ldew_rain_before_thermal: f64,
    pub ldew_snow_before_thermal: f64,
    /// 雪+土各层水与冰：THERMAL 之前，THERMAL 之后再换成 THERMAL 之后（WATER 之前）。
    pub wliq_old: [f64; SOISNO_LAYERS],
    pub wice_old: [f64; SOISNO_LAYERS],
    pub wa_old: f64,
    pub wdsrf_old: f64,
    pub wetwat_old: f64,
    /// `flood_evap_temp_trc = t_soisno(lb)`：THERMAL 之前的顶层温度（`CoLMMAIN.F90:1033`）。
    pub flood_evap_temp_k: f64,
}

/// 雪 5 层 + 土 10 层拼成 Fortran 的 `maxsnl+1:nl_soil`。
pub fn pack_soisno(snow: &[f64], soil: &[f64]) -> [f64; SOISNO_LAYERS] {
    let mut packed = [0.0; SOISNO_LAYERS];
    packed[..MAX_SNOW_LAYERS].copy_from_slice(&snow[..MAX_SNOW_LAYERS]);
    packed[MAX_SNOW_LAYERS..].copy_from_slice(&soil[..SOIL_LAYERS]);
    packed
}

/// 截留之后（`CoLMMAIN.F90:922-940`）：步首收支快照，再按截留结果搬运冠层材料。
pub fn after_interception(
    ctx: &MaterialStepContext<'_>,
    track: &mut PatchMaterialTrack,
    snl: i32,
    precip: &PrecipInput,
) -> Result<()> {
    material_save_storage(
        ctx.set,
        ctx.physics,
        &mut track.state,
        &mut track.snapshot,
        &SaveStorageInput {
            snl,
            waterstorage: None,
            runtime_forced: ctx.runtime_forced,
        },
    );
    material_precip(
        ctx.set,
        &mut track.state,
        ctx.rain_ratio,
        ctx.snow_ratio,
        precip,
    );
    Ok(())
}

/// 新雪之后（`CoLMMAIN.F90:964-975`）。
pub fn after_new_snow(
    ctx: &MaterialStepContext<'_>,
    track: &mut PatchMaterialTrack,
    scratch: &mut MaterialStepScratch,
    input: &NewSnowInput,
) {
    scratch.scv_after_new_snow = input.scv;
    let _warnings = material_newsnow(ctx.set, &mut track.state, input);
}

/// THERMAL 之前（`CoLMMAIN.F90:982-1029`）：水量快照；关 `DEF_VEG_SNOW` 时冠层材料按
/// 叶温整体归到雨或雪（与水侧同一时刻）。`ldew_*` 是归相之后的冠层水。
#[allow(clippy::too_many_arguments)]
pub fn before_thermal(
    ctx: &MaterialStepContext<'_>,
    track: &mut PatchMaterialTrack,
    scratch: &mut MaterialStepScratch,
    leaf_temperature_k: f64,
    ldew_rain: f64,
    ldew_snow: f64,
    wliq: [f64; SOISNO_LAYERS],
    wice: [f64; SOISNO_LAYERS],
    wa: f64,
    wdsrf: f64,
    wetwat: f64,
) {
    scratch.wliq_old = wliq;
    scratch.wice_old = wice;
    scratch.wa_old = wa;
    scratch.wdsrf_old = wdsrf;
    scratch.wetwat_old = wetwat;
    if !ctx.vegetation_snow {
        for itrc in ctx.set.transport_indices() {
            let pools = &mut track.state.pools[itrc];
            if leaf_temperature_k > crate::FREEZING_K {
                pools.ldew_rain += pools.ldew_snow;
                pools.ldew_snow = 0.0;
            } else {
                pools.ldew_snow += pools.ldew_rain;
                pools.ldew_rain = 0.0;
            }
        }
    }
    scratch.ldew_rain_before_thermal = ldew_rain;
    scratch.ldew_snow_before_thermal = ldew_snow;
}

/// THERMAL 之后的宿主量（`material_evapo` 的实参与雪融携带）。
#[derive(Debug, Clone, Copy)]
pub struct AfterThermalHost<'a> {
    pub deltim: f64,
    pub snl: i32,
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    pub wliq: &'a [f64; SOISNO_LAYERS],
    pub wice: &'a [f64; SOISNO_LAYERS],
    pub canopy_melt_mass: f64,
    pub canopy_freeze_mass: f64,
    /// 逐层相变：`max(冰前-冰后, 0)` 与 `max(冰后-冰前, 0)`。
    pub soil_thaw_mass: &'a [f64; SOISNO_LAYERS],
    pub soil_freeze_mass: &'a [f64; SOISNO_LAYERS],
    pub tleaf: f64,
    pub t_soisno: &'a [f64; SOISNO_LAYERS],
    pub forc_q: f64,
    pub forc_psrf: f64,
    pub scv: f64,
}

/// THERMAL 之后（`CoLMMAIN.F90:1109-1151`）：冠层与表层的蒸发/凝结、薄雪融化携带，
/// 再把 WATER 之前的水量记下来。
pub fn after_thermal(
    ctx: &MaterialStepContext<'_>,
    track: &mut PatchMaterialTrack,
    scratch: &mut MaterialStepScratch,
    host: &AfterThermalHost<'_>,
) -> Result<()> {
    material_evapo(
        ctx.set,
        &mut track.state,
        ctx.physics,
        ctx.vapor_ratio,
        &EvapoInput {
            deltim: host.deltim,
            snl: host.snl,
            ldew_rain: host.ldew_rain,
            ldew_snow: host.ldew_snow,
            ldew_rain_bef: scratch.ldew_rain_before_thermal,
            ldew_snow_bef: scratch.ldew_snow_before_thermal,
            wliq_soisno: host.wliq,
            wice_soisno: host.wice,
            wliq_soisno_bef: &scratch.wliq_old,
            wice_soisno_bef: &scratch.wice_old,
            canopy_smelt_mass: Some(host.canopy_melt_mass),
            canopy_frzc_mass: Some(host.canopy_freeze_mass),
            soil_thaw_mass: Some(host.soil_thaw_mass),
            soil_frzc_mass: Some(host.soil_freeze_mass),
            tleaf: Some(host.tleaf),
            t_soisno: Some(host.t_soisno),
            forc_q: Some(host.forc_q),
            forc_psrf: Some(host.forc_psrf),
            subl_skin_mm: ctx.soil_options.subl_skin_mm,
            canopy_equilibration: ctx.canopy_equilibration,
        },
    )?;
    material_snow_melt_carry(
        ctx.set,
        &mut track.state,
        host.snl,
        host.scv,
        scratch.scv_after_new_snow,
    );
    scratch.wliq_old = *host.wliq;
    scratch.wice_old = *host.wice;
    Ok(())
}
