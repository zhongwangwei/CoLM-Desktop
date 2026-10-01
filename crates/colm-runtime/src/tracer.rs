//! 示踪物的运行时部分：namelist 与参数文件 → [`TracerSet`]，陆面示踪物重启的读写
//! （`MOD_Tracer_Rest`）。

use std::path::Path;

use anyhow::{Context, Result};
use colm_core::tracer::{
    PatchTracerState, TracerNamelist, TracerParameterOverrides, TracerPools, TracerSet,
    DESCRIPTOR_IDENTITY_WIDTH, SOISNO_LAYERS,
};
use colm_namelist::{Document, Value};

use crate::physics::{integer, logical, real, text};

/// `LAND_TRACER_RESTART_SCHEMA_VERSION`。
const LAND_TRACER_RESTART_SCHEMA: i32 = 5;
/// `TRC_FORC_CACHE_SCHEMA`。
const FORCING_CACHE_SCHEMA: i32 = 1;

/// 算例开了 `DEF_USE_TRACER` 时注册示踪物（`tracer_defs_init`）；关着返回 `None`。
pub fn tracer_set_from_document(document: &Document) -> Result<Option<TracerSet>> {
    if !logical(document, "DEF_USE_TRACER")? {
        return Ok(None);
    }
    let namelist = TracerNamelist {
        num: integer(document, "DEF_TRACER_NUM")?,
        names: text(document, "DEF_TRACER_NAMES")?,
        types: text(document, "DEF_TRACER_TYPES")?,
        mrat: text(document, "DEF_TRACER_MRAT")?,
        ref_ratio: text(document, "DEF_TRACER_REF_RATIO")?,
        init_delta: text(document, "DEF_TRACER_INIT_DELTA")?,
        reactive_decay_rate: text(document, "DEF_TRACER_REACTIVE_DECAY_RATE")?,
        param_files: text(document, "DEF_TRACER_PARAM_FILES")?,
        use_bgc: logical(document, "DEF_USE_BGC")?,
        variably_saturated_flow: logical(document, "DEF_USE_VariablySaturatedFlow")?,
        aquifer_mixing_water_mm: real(document, "DEF_TRACER_AQUIFER_MIXING_WATER_MM")?,
    };
    TracerSet::build(&namelist, read_tracer_parameter_file).map(Some)
}

/// `read_tracer_parameter_file`：文件必须存在；没有 `&nl_colm_tracer_parameter` 组时
/// 什么都不改（`tracer_parameter_group_present`）。
fn read_tracer_parameter_file(path: &str) -> Result<Option<TracerParameterOverrides>> {
    let source = std::fs::read_to_string(path).with_context(|| {
        format!("read_tracer_parameter_file: missing tracer parameter file: {path}")
    })?;
    let present = source.lines().any(|line| {
        let line = line.trim_start().to_ascii_lowercase();
        !line.starts_with('!')
            && (line.starts_with("&nl_colm_tracer_parameter")
                || line.starts_with("$nl_colm_tracer_parameter"))
    });
    if !present {
        return Ok(None);
    }
    let document = colm_namelist::parse(&source)
        .with_context(|| format!("invalid &nl_colm_tracer_parameter in {path}"))?;
    let real_field = |name: &str| -> Result<Option<f64>> {
        match document.get(&format!("DEF_TRACER%{name}")) {
            None => Ok(None),
            Some(value) => value
                .as_f64()
                .map(Some)
                .with_context(|| format!("DEF_TRACER%{name} in {path} is not a real")),
        }
    };
    let charge = match document.get("DEF_TRACER%charge") {
        None => None,
        Some(Value::Int(value)) => Some(i32::try_from(*value)?),
        Some(other) => {
            anyhow::bail!("DEF_TRACER%charge in {path} must be an integer, got {other:?}")
        }
    };
    let unit_kind = match document.get("DEF_TRACER%unit_kind") {
        None => None,
        Some(Value::Str(value)) => Some(value.clone()),
        Some(other) => {
            anyhow::bail!("DEF_TRACER%unit_kind in {path} must be a string, got {other:?}")
        }
    };
    Ok(Some(TracerParameterOverrides {
        unit_kind,
        mol_weight: real_field("mol_weight")?,
        ref_ratio: real_field("ref_ratio")?,
        init_delta: real_field("init_delta")?,
        init_conc: real_field("init_conc")?,
        precip_default_conc: real_field("precip_default_conc")?,
        vapor_default_conc: real_field("vapor_default_conc")?,
        max_dissolved_conc: real_field("max_dissolved_conc")?,
        reactive_decay_rate: real_field("reactive_decay_rate")?,
        charge,
    }))
}

/// 一次运行里共享的示踪物配置（所有 patch 同一份）。
#[derive(Debug)]
pub struct TracerRuntime {
    pub set: TracerSet,
    pub physics: colm_core::tracer::TracerPhysics,
    pub soil_options: colm_core::tracer::soil_water::SoilWaterOptions,
    pub canopy_equilibration: f64,
    /// 逐示踪物的降水/水汽比值。没有示踪物强迫文件时就是描述符的默认比值
    /// （`tracer_forcing_precip_value` 回落到 `tracer_precip_default_ratio`）。
    pub precip_ratio: Vec<f64>,
    pub vapor_ratio: Vec<f64>,
    pub runtime_forced: Vec<bool>,
    pub debug: bool,
    pub vegetation_snow: bool,
    pub variably_saturated_flow: bool,
    pub aquifer_mixing_water_mm: f64,
    /// `DEF_TRACER_BALANCE_ABORT_NBAD`、`DEF_TRACER_RESID_ABORT_NBAD`。
    pub balance_abort_nbad: i32,
    pub resid_abort_nbad: i32,
    /// `MOD_Tracer_Conservation` 的模块级"最差/计数"跟踪（整次运行共享）。
    pub tracker: std::sync::Mutex<colm_core::tracer::conservation::BalanceTracker>,
}

/// 分馏开关与参数（`MOD_Namelist.F90:421-442`，方案名的规范化同 `:1703-1717`）。
fn tracer_physics_from_document(document: &Document) -> Result<colm_core::tracer::TracerPhysics> {
    use colm_core::tracer::frac::{KineticScheme, OpenWaterKinetic};
    let kinetic = text(document, "DEF_TRACER_KINETIC_SCHEME")?;
    let open_water = text(document, "DEF_TRACER_OPEN_WATER_KINETIC")?;
    let physics = colm_core::tracer::TracerPhysics {
        fractionation: logical(document, "DEF_TRACER_USE_FRACTIONATION")?,
        kinetic_scheme: KineticScheme::parse(&kinetic)
            .with_context(|| format!("Invalid DEF_TRACER_KINETIC_SCHEME: {kinetic}"))?,
        ice_supersat_slope: real(document, "DEF_TRACER_ICE_SUPERSAT_SLOPE")?,
        cg_relhum_max: real(document, "DEF_TRACER_CG_RELHUM_MAX")?,
        open_water_kinetic: OpenWaterKinetic::parse(&open_water)
            .with_context(|| format!("Invalid DEF_TRACER_OPEN_WATER_KINETIC: {open_water}"))?,
        nss_leaf_water_per_lai: real(document, "DEF_TRACER_NSS_LEAF_WATER_PER_LAI")?,
        nss_leaf_path_length: real(document, "DEF_TRACER_NSS_LEAF_PATH_LENGTH")?,
        nss_leaf_rb: real(document, "DEF_TRACER_NSS_LEAF_RB")?,
    };
    anyhow::ensure!(
        physics.cg_relhum_max > 0.0 && physics.cg_relhum_max < 1.0 && physics.ice_supersat_slope >= 0.0,
        "Invalid tracer fractionation parameters (DEF_TRACER_CG_RELHUM_MAX in (0,1), \
         DEF_TRACER_ICE_SUPERSAT_SLOPE >= 0)"
    );
    Ok(physics)
}

/// `DEF_TRACER_SOIL_KINETIC`（`MOD_Namelist.F90:1718-1725`）：`RESISTANCE` 为真、`EXPONENT` 为假。
fn soil_kinetic_resistance(document: &Document) -> Result<bool> {
    let scheme = text(document, "DEF_TRACER_SOIL_KINETIC")?;
    match scheme.trim() {
        "RESISTANCE" | "resistance" => Ok(true),
        "EXPONENT" | "exponent" => Ok(false),
        other => anyhow::bail!("Invalid DEF_TRACER_SOIL_KINETIC: {other}"),
    }
}

impl TracerRuntime {
    /// 由算例文档建立；`DEF_USE_TRACER` 关着返回 `None`。示踪物强迫文件
    /// （`&nl_colm_tracer_forcing`）尚未移植：有输运示踪物时一律用默认比值。
    pub fn from_document(document: &Document) -> Result<Option<Self>> {
        let Some(set) = tracer_set_from_document(document)? else {
            return Ok(None);
        };
        let physics = tracer_physics_from_document(document)?;
        let precip_ratio = set.tracers.iter().map(|t| t.precip_default_ratio()).collect();
        let vapor_ratio = set.tracers.iter().map(|t| t.vapor_default_ratio()).collect();
        let runtime_forced = vec![false; set.len()];
        Ok(Some(Self {
            physics,
            soil_options: colm_core::tracer::soil_water::SoilWaterOptions {
                subl_skin_mm: real(document, "DEF_TRACER_SUBL_SKIN_MM")?,
                soil_diffusion: logical(document, "DEF_TRACER_SOIL_DIFFUSION")?,
                soil_vapor_diffusion: logical(document, "DEF_TRACER_SOIL_VAPOR_DIFFUSION")?,
                colm_debug: logical(document, "DEF_USE_CoLMDEBUG")?,
                soil_kinetic_resistance: soil_kinetic_resistance(document)?,
                snowmelt_equilibration: real(document, "DEF_TRACER_SNOWMELT_EQUILIBRATION")?,
            },
            canopy_equilibration: real(document, "DEF_TRACER_CANOPY_EQUILIBRATION")?,
            precip_ratio,
            vapor_ratio,
            runtime_forced,
            debug: logical(document, "DEF_USE_CoLMDEBUG")?,
            vegetation_snow: logical(document, "DEF_VEG_SNOW")?,
            variably_saturated_flow: logical(document, "DEF_USE_VariablySaturatedFlow")?,
            aquifer_mixing_water_mm: real(document, "DEF_TRACER_AQUIFER_MIXING_WATER_MM")?,
            balance_abort_nbad: i32::try_from(integer(document, "DEF_TRACER_BALANCE_ABORT_NBAD")?)?,
            resid_abort_nbad: i32::try_from(integer(document, "DEF_TRACER_RESID_ABORT_NBAD")?)?,
            tracker: Default::default(),
            set,
        }))
    }

    /// 有没有要逐 patch 记账的输运示踪物。
    pub fn has_transport(&self) -> bool {
        self.set.transport_indices().next().is_some()
    }

    pub fn context(&self) -> colm_core::tracer::step::TracerStepContext<'_> {
        colm_core::tracer::step::TracerStepContext {
            set: &self.set,
            physics: self.physics,
            soil_options: self.soil_options,
            canopy_equilibration: self.canopy_equilibration,
            precip_ratio: &self.precip_ratio,
            vapor_ratio: &self.vapor_ratio,
            runtime_forced: &self.runtime_forced,
            debug: self.debug,
            vegetation_snow: self.vegetation_snow,
        }
    }

    /// 起跑时的示踪物状态：续跑文件里有可用的示踪物事务就读，否则按水量冷启动
    /// （`tracer_init_from_arrays`）。
    pub fn initial_state(
        &self,
        restart: &colm_init::RestartFile,
        patch: usize,
        patches: usize,
        water: colm_core::tracer::WaterInventory<'_>,
    ) -> Result<PatchTracerState> {
        if let Some(mut states) = read_land_tracer_restart(restart, &self.set, patches)? {
            return Ok(states.swap_remove(patch));
        }
        let mut state = PatchTracerState::allocated(&self.set);
        state.init_from_water(
            &self.set,
            water,
            colm_core::tracer::TracerColdStart {
                variably_saturated_flow: self.variably_saturated_flow,
                aquifer_mixing_water_mm: self.aquifer_mixing_water_mm,
            },
        )?;
        Ok(state)
    }
}

/// 一个土壤/湿地 patch 步末的示踪物记账（`CoLMMAIN.F90:1528-1561` 与 `tracer_report`）：一阶衰减、
/// 收支检查、history 累加，再按 `DEF_TRACER_*_ABORT_NBAD` 决定是否中止。
pub fn end_of_step(
    runtime: &TracerRuntime,
    patch_type: i32,
    state: &mut colm_core::StandardLctSnowSoilState,
    output: &colm_core::StandardLctSnowSoilOutput,
    forcing: &colm_core::RuntimeForcing,
    deltim: f64,
    initial_total_water_mm: f64,
) -> Result<()> {
    use colm_core::tracer::{conservation, hist, step::pack_soisno};
    let Some(track) = state.tracer.as_deref_mut() else {
        return Ok(());
    };
    let snl = state.snow.layer_count;
    conservation::tracer_apply_reactive_processes(&runtime.set, &mut track.state, snl, deltim);
    // `endwb`（`CoLMMAIN.F90:1496-1504`）与 `errorw`：与 history 的 `xerr` 同一套式子。
    // 与 history 的 `endwb` 同一套（`history.rs` 的 LCT 分支）：灌溉打开时取 `bgc_driver`
    // 之前的土壤水与 `waterstorage`；VSF 湿地再加 `wetwat`（`CoLMMAIN.F90:1496-1506`）。
    let (balance_water, irrigation_storage) = match &output.irrigation_balance {
        Some(balance) => (&balance.soil_water, Some(balance.storage_mm)),
        None => (&state.soil_water, None),
    };
    let mut end_total = colm_core::total_water_storage_mm(
        balance_water,
        state.energy.leaf.canopy_water.total_mm,
        state.snow.water_equivalent_kg_m2,
        irrigation_storage,
    );
    if patch_type == 2 && runtime.variably_saturated_flow {
        end_total += state.soil_water.wetland_water_mm;
    }
    let precipitation =
        forcing.convective_precipitation_kg_m2_s + forcing.large_scale_precipitation_kg_m2_s;
    let evaporation_wb = output.energy.total_evaporation_kg_m2_s;
    let runoff = output.water.soil.total_runoff_mm_s;
    let errorw = (-(((precipitation + 0.0) - evaporation_wb) - runoff))
        .mul_add(deltim, end_total - initial_total_water_mm);
    {
        let mut tracker = runtime
            .tracker
            .lock()
            .map_err(|_| anyhow::anyhow!("the tracer balance tracker lock is poisoned"))?;
        conservation::tracer_balance_check(
            &runtime.set,
            runtime.physics,
            &mut track.state,
            &track.snapshot,
            &mut tracker,
            &conservation::BalanceCheckInput {
                ipatch: 1,
                snl,
                deltim,
                patchtype: Some(patch_type),
                water_err: Some(errorw),
                water_ds: Some(end_total - initial_total_water_mm),
                water_input: Some((precipitation + 0.0) * deltim),
                water_output: Some((evaporation_wb + runoff) * deltim),
                water_evap: Some(evaporation_wb * deltim),
                water_rnof: Some(runoff * deltim),
                flood_heterogeneous: None,
                catch_lateral_flow: false,
                runtime_forced: &runtime.runtime_forced,
            },
        );
    }
    let wliq = pack_soisno(&state.snow.liquid_water_kg_m2, &state.soil_water.liquid_water_kg_m2);
    let wice = pack_soisno(&state.snow.ice_water_kg_m2, &state.soil_water.ice_water_kg_m2);
    hist::tracer_hist_accumulate(
        &runtime.set,
        &mut track.state,
        &hist::HistAccumulateInput {
            snl,
            ldew_rain: state.energy.leaf.canopy_water.rain_mm,
            ldew_snow: state.energy.leaf.canopy_water.snow_mm,
            wliq_soisno: &wliq,
            wice_soisno: &wice,
            wa: state.soil_water.aquifer_water_mm,
            wdsrf: state.soil_water.surface_water_mm,
            wetwat: state.soil_water.wetland_water_mm,
            scv: state.snow.water_equivalent_kg_m2,
        },
    );
    Ok(())
}

/// 一步里所有 patch 都推进完之后（`CoLMDRIVER.F90:392-393`，在 `hist_out` 之前）调一次
/// `tracer_report`。各 patch 共用同一个 [`TracerRuntime`]，取第一个挂了示踪物的模板。
pub fn report_after_patches(templates: &[crate::assembly::StandardLctRestartTemplate]) -> Result<()> {
    match templates.iter().find_map(|template| template.tracer.as_ref()) {
        Some((runtime, _)) => report_step(runtime),
        None => Ok(()),
    }
}

/// `tracer_report`：打印本步的收支/签名报告，按 `DEF_TRACER_*_ABORT_NBAD` 决定是否中止。
fn report_step(runtime: &TracerRuntime) -> Result<()> {
    let report = runtime
        .tracker
        .lock()
        .map_err(|_| anyhow::anyhow!("the tracer balance tracker lock is poisoned"))?
        .report(runtime.set.len(), runtime.balance_abort_nbad, runtime.resid_abort_nbad);
    for line in &report.lines {
        println!("{line}");
    }
    if let Some(message) = report.abort {
        anyhow::bail!("{message}");
    }
    Ok(())
}

/// 冰川 patch 步末（`CoLMMAIN.F90:1773-1779` 的 `tracer_glacier_patch`）：上游在
/// `patchtype > 2` 的清零之前调，这里也在 `clear_non_soil_patch` 之前。
pub fn glacier_end_of_step(
    runtime: &TracerRuntime,
    state: &mut colm_core::StandardLctSnowSoilState,
    output: &colm_core::GlacierStepOutput,
    deltim: f64,
    forcing: &colm_core::RuntimeForcing,
) -> Result<()> {
    use colm_core::tracer::{special_patches, step::pack_soisno};
    let t_grnd = state.surface_temperature_k();
    let Some(track) = state.tracer.as_deref_mut() else {
        return Ok(());
    };
    let wliq = pack_soisno(&state.snow.liquid_water_kg_m2, &state.soil_water.liquid_water_kg_m2);
    let wice = pack_soisno(&state.snow.ice_water_kg_m2, &state.soil_water.ice_water_kg_m2);
    let precipitation = &output.precipitation;
    let thermal = &output.thermal;
    {
        let mut tracker = runtime
            .tracker
            .lock()
            .map_err(|_| anyhow::anyhow!("the tracer balance tracker lock is poisoned"))?;
        special_patches::tracer_glacier_patch(
            &runtime.set,
            runtime.physics,
            &mut track.state,
            &mut track.snapshot,
            &mut tracker,
            &special_patches::GlacierInput {
                ipatch: 1,
                deltim,
                prc_rain: precipitation.convective_rain_kg_m2_s,
                prl_rain: precipitation.large_scale_rain_kg_m2_s,
                prc_snow: precipitation.convective_snow_kg_m2_s,
                prl_snow: precipitation.large_scale_snow_kg_m2_s,
                rnof: output.total_runoff_mm_s,
                qseva: thermal.qseva,
                qsubl: thermal.qsubl,
                qsdew: thermal.qsdew,
                qfros: thermal.qfros,
                endwb: output.final_total_water_mm,
                totwb: output.initial_total_water_mm,
                glacier_overflow_mass: output.overflow_mass_mm,
                errorw: output.water_balance_error_mm,
                wdsrf: state.soil_water.surface_water_mm,
                scv: state.snow.water_equivalent_kg_m2,
                t_grnd,
                forc_q: forcing.specific_humidity,
                forc_psrf: forcing.surface_pressure_pa,
                wliq_soisno: &wliq,
                wice_soisno: &wice,
                subl_skin_mm: runtime.soil_options.subl_skin_mm,
                precip_ratio: &runtime.precip_ratio,
                vapor_ratio: &runtime.vapor_ratio,
                runtime_forced: &runtime.runtime_forced,
                catch_lateral_flow: false,
            },
        )?;
    }
    Ok(())
}

/// 湖 patch 步末（`CoLMMAIN.F90:1990-1996` 的 `tracer_waterbody_patch`）。单点没有
/// 河湖子步，每步都采样 history。
pub fn lake_end_of_step(
    runtime: &TracerRuntime,
    state: &mut colm_core::StandardLctSnowSoilState,
    output: &colm_core::LakeStepOutput,
    deltim: f64,
    forcing: &colm_core::RuntimeForcing,
    dynamic_lake: bool,
) -> Result<()> {
    use colm_core::tracer::{special_patches, step::pack_soisno};
    let t_grnd = state.surface_temperature_k();
    let Some(track) = state.tracer.as_deref_mut() else {
        return Ok(());
    };
    let wliq = pack_soisno(&state.snow.liquid_water_kg_m2, &state.soil_water.liquid_water_kg_m2);
    let wice = pack_soisno(&state.snow.ice_water_kg_m2, &state.soil_water.ice_water_kg_m2);
    let precipitation = &output.precipitation;
    let thermal = &output.thermal;
    {
        let mut tracker = runtime
            .tracker
            .lock()
            .map_err(|_| anyhow::anyhow!("the tracer balance tracker lock is poisoned"))?;
        special_patches::tracer_waterbody_patch(
            &runtime.set,
            runtime.physics,
            &mut track.state,
            &mut track.snapshot,
            &mut tracker,
            &special_patches::WaterbodyInput {
                ipatch: 1,
                snl: state.snow.layer_count,
                deltim,
                forc_rain: precipitation.convective_rain_kg_m2_s
                    + precipitation.large_scale_rain_kg_m2_s,
                forc_snow: precipitation.convective_snow_kg_m2_s
                    + precipitation.large_scale_snow_kg_m2_s,
                lake_deficit: output.lake_deficit_mm_s,
                rnof: output.total_runoff_mm_s,
                qseva: thermal.qseva,
                qsubl: thermal.qsubl,
                qsdew: thermal.qsdew,
                qfros: thermal.qfros,
                endwb: output.final_total_water_mm,
                totwb: output.initial_total_water_mm,
                errorw: output.water_balance_error_mm,
                wa: state.soil_water.aquifer_water_mm,
                wdsrf: state.soil_water.surface_water_mm,
                scv: state.snow.water_equivalent_kg_m2,
                t_grnd,
                forc_q: forcing.specific_humidity,
                forc_psrf: forcing.surface_pressure_pa,
                forc_us: forcing.eastward_wind_m_s,
                forc_vs: forcing.northward_wind_m_s,
                wliq_soisno: &wliq,
                wice_soisno: &wice,
                use_dynamic_lake: dynamic_lake,
                subl_skin_mm: runtime.soil_options.subl_skin_mm,
                hist_sample: true,
                precip_ratio: &runtime.precip_ratio,
                vapor_ratio: &runtime.vapor_ratio,
                runtime_forced: &runtime.runtime_forced,
                catch_lateral_flow: false,
            },
        )?;
    }
    Ok(())
}

/// 每个输运示踪物逐 patch 的一个量（重启里的 `(patch, trc_land_transport)`）。
type PatchField = fn(&TracerPools) -> f64;
/// 逐层的量（`(patch, soilsnow, trc_land_transport)`）。
type LayerField = fn(&TracerPools) -> &[f64; SOISNO_LAYERS];
/// 读回时给逐 patch 量赋值。
type PatchSetter = fn(&mut TracerPools, f64);
/// 读回时取分层量的可变引用。
type LayerSetter = fn(&mut TracerPools) -> &mut [f64; SOISNO_LAYERS];

/// 上游 `write_land_tracer_restart` 的写出顺序（`trc_wa` 之后插 `trc_aquifer_ref_*`）。
const CANOPY_FIELDS: [(&str, PatchField); 2] = [
    ("trc_ldew_rain", |p| p.ldew_rain),
    ("trc_ldew_snow", |p| p.ldew_snow),
];
const LAYER_FIELDS: [(&str, LayerField); 3] = [
    ("trc_wliq_soisno", |p| &p.wliq_soisno),
    ("trc_wice_soisno", |p| &p.wice_soisno),
    ("trc_solid_soisno", |p| &p.solid_soisno),
];
const TAIL_FIELDS: [(&str, PatchField); 15] = [
    ("trc_wdsrf", |p| p.wdsrf),
    ("trc_wetwat", |p| p.wetwat),
    ("trc_surface_residue", |p| p.surface_residue),
    ("trc_subsurface_residue", |p| p.subsurface_residue),
    ("trc_canopy_solid", |p| p.canopy_solid),
    ("trc_surface_solid", |p| p.surface_solid),
    ("trc_subsurface_solid", |p| p.subsurface_solid),
    ("trc_waterstorage_solid", |p| p.waterstorage_solid),
    ("trc_scv", |p| p.scv),
    ("trc_waterstorage", |p| p.waterstorage),
    ("trc_leaf_delta_e", |p| p.leaf_delta_e),
    ("trc_leaf_delta_b", |p| p.leaf_delta_b),
    ("trc_leaf_peclet", |p| p.leaf_peclet),
    ("trc_leaf_water_moles", |p| p.leaf_water_moles),
    ("trc_leaf_iso_storage", |p| p.leaf_iso_storage),
];

/// `write_land_tracer_restart`（含 `tracer_forcing_write_restart` 的计数）：把示踪物
/// 预报量追加进一个已写好的陆面时间重启（`patch`/`soilsnow` 维已在）。没有输运示踪物时
/// 只写空事务（见 `colm_init::write_empty_land_tracer_transaction`）。
pub fn write_land_tracer_restart(
    path: &Path,
    set: &TracerSet,
    states: &[&PatchTracerState],
    aquifer_mixing_water_mm: f64,
) -> Result<()> {
    let transport: Vec<usize> = set.transport_indices().collect();
    if transport.is_empty() {
        colm_init::write_empty_land_tracer_transaction(path, aquifer_mixing_water_mm)?;
    } else {
        let mut file =
            netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
        let ntransport = transport.len();
        put_scalar_i32(&mut file, "trc_land_restart_complete", 0)?;
        put_scalar_i32(
            &mut file,
            "trc_land_restart_schema",
            LAND_TRACER_RESTART_SCHEMA,
        )?;
        put_scalar_i32(
            &mut file,
            "trc_land_transport_count",
            i32::try_from(ntransport)?,
        )?;
        put_scalar_f64(
            &mut file,
            "trc_aquifer_mixing_water_mm",
            aquifer_mixing_water_mm,
        )?;
        ensure_dimension(
            &mut file,
            "trc_land_descriptor_field",
            DESCRIPTOR_IDENTITY_WIDTH,
        )?;
        ensure_dimension(&mut file, "trc_land_transport", ntransport)?;
        let identity: Vec<i32> = set.descriptor_identity().into_iter().flatten().collect();
        put_array_i32(
            &mut file,
            "trc_land_descriptor_identity",
            &["trc_land_transport", "trc_land_descriptor_field"],
            &identity,
        )?;
        let patch_field = |field: PatchField| -> Vec<f64> {
            states
                .iter()
                .flat_map(|state| transport.iter().map(move |&itrc| field(&state.pools[itrc])))
                .collect()
        };
        for (name, field) in &CANOPY_FIELDS {
            put_array_f64(
                &mut file,
                name,
                &["patch", "trc_land_transport"],
                &patch_field(*field),
            )?;
        }
        for (name, field) in LAYER_FIELDS {
            let mut values = Vec::with_capacity(states.len() * SOISNO_LAYERS * transport.len());
            for state in states {
                for slot in 0..SOISNO_LAYERS {
                    for &itrc in &transport {
                        values.push(field(&state.pools[itrc])[slot]);
                    }
                }
            }
            put_array_f64(
                &mut file,
                name,
                &["patch", "soilsnow", "trc_land_transport"],
                &values,
            )?;
        }
        put_array_f64(
            &mut file,
            "trc_wa",
            &["patch", "trc_land_transport"],
            &patch_field(|p| p.wa),
        )?;
        let reference: Vec<f64> = states.iter().map(|state| state.aquifer_ref_water).collect();
        put_array_f64(&mut file, "trc_aquifer_ref_water", &["patch"], &reference)?;
        put_array_f64(
            &mut file,
            "trc_aquifer_ref_mass",
            &["patch", "trc_land_transport"],
            &patch_field(|p| p.aquifer_ref_mass),
        )?;
        for (name, field) in &TAIL_FIELDS {
            put_array_f64(
                &mut file,
                name,
                &["patch", "trc_land_transport"],
                &patch_field(*field),
            )?;
        }
        put_scalar_i32(&mut file, "trc_forcing_cache_schema", FORCING_CACHE_SCHEMA)?;
        put_scalar_i32(&mut file, "trc_forcing_cache_count", 0)?;
        put_scalar_i32(&mut file, "trc_land_restart_complete", 1)?;
    }
    Ok(())
}

/// `read_land_tracer_restart` 的判定与读入：已提交（`complete = 1`）、schema 5、
/// 输运示踪物个数与描述符指纹都一致才读；否则返回 `None`，由调用方从水量冷启动
/// （上游打印 "Generic land tracer restart is legacy/incompatible"）。
pub fn read_land_tracer_restart(
    restart: &colm_init::RestartFile,
    set: &TracerSet,
    patches: usize,
) -> Result<Option<Vec<PatchTracerState>>> {
    let transport: Vec<usize> = set.transport_indices().collect();
    let scalar = |name: &str| -> Option<i64> {
        restart
            .integers(name)
            .ok()
            .and_then(|values| values.first().copied())
    };
    let identity_matches = || -> bool {
        let Ok(stored) = restart.integers("trc_land_descriptor_identity") else {
            return false;
        };
        let expected: Vec<i64> = set
            .descriptor_identity()
            .into_iter()
            .flatten()
            .map(i64::from)
            .collect();
        stored == expected.as_slice()
    };
    let matches = scalar("trc_land_restart_complete") == Some(1)
        && scalar("trc_land_restart_schema") == Some(i64::from(LAND_TRACER_RESTART_SCHEMA))
        && scalar("trc_land_transport_count") == Some(transport.len() as i64)
        && (transport.is_empty() || identity_matches());
    if !matches {
        return Ok(None);
    }
    let mut states: Vec<PatchTracerState> = (0..patches)
        .map(|_| PatchTracerState::allocated(set))
        .collect();
    if transport.is_empty() {
        return Ok(Some(states));
    }
    let n = transport.len();
    let read_patch = |name: &str,
                      states: &mut [PatchTracerState],
                      set_value: PatchSetter|
     -> Result<()> {
        let values = restart.floats(name)?;
        anyhow::ensure!(values.len() == patches * n, "{name} has an unexpected size");
        for (patch, state) in states.iter_mut().enumerate() {
            for (k, &itrc) in transport.iter().enumerate() {
                set_value(&mut state.pools[itrc], values[patch * n + k]);
            }
        }
        Ok(())
    };
    let setters: [(&str, PatchSetter); 19] = [
        ("trc_ldew_rain", |p, v| p.ldew_rain = v),
        ("trc_ldew_snow", |p, v| p.ldew_snow = v),
        ("trc_wa", |p, v| p.wa = v),
        ("trc_aquifer_ref_mass", |p, v| p.aquifer_ref_mass = v),
        ("trc_wdsrf", |p, v| p.wdsrf = v),
        ("trc_wetwat", |p, v| p.wetwat = v),
        ("trc_surface_residue", |p, v| p.surface_residue = v),
        ("trc_subsurface_residue", |p, v| p.subsurface_residue = v),
        ("trc_canopy_solid", |p, v| p.canopy_solid = v),
        ("trc_surface_solid", |p, v| p.surface_solid = v),
        ("trc_subsurface_solid", |p, v| p.subsurface_solid = v),
        ("trc_waterstorage_solid", |p, v| p.waterstorage_solid = v),
        ("trc_scv", |p, v| p.scv = v),
        ("trc_waterstorage", |p, v| p.waterstorage = v),
        ("trc_leaf_delta_e", |p, v| p.leaf_delta_e = v),
        ("trc_leaf_delta_b", |p, v| p.leaf_delta_b = v),
        ("trc_leaf_peclet", |p, v| p.leaf_peclet = v),
        ("trc_leaf_water_moles", |p, v| p.leaf_water_moles = v),
        ("trc_leaf_iso_storage", |p, v| p.leaf_iso_storage = v),
    ];
    for (name, setter) in setters {
        read_patch(name, &mut states, setter)?;
    }
    let layer_setters: [(&str, LayerSetter); 3] = [
        ("trc_wliq_soisno", |p| &mut p.wliq_soisno),
        ("trc_wice_soisno", |p| &mut p.wice_soisno),
        ("trc_solid_soisno", |p| &mut p.solid_soisno),
    ];
    for (name, layers) in layer_setters {
        let values = restart.floats(name)?;
        anyhow::ensure!(
            values.len() == patches * SOISNO_LAYERS * n,
            "{name} has an unexpected size"
        );
        for (patch, state) in states.iter_mut().enumerate() {
            for slot in 0..SOISNO_LAYERS {
                for (k, &itrc) in transport.iter().enumerate() {
                    layers(&mut state.pools[itrc])[slot] =
                        values[(patch * SOISNO_LAYERS + slot) * n + k];
                }
            }
        }
    }
    let reference = restart.floats("trc_aquifer_ref_water")?;
    anyhow::ensure!(
        reference.len() == patches,
        "trc_aquifer_ref_water has an unexpected size"
    );
    for (state, &value) in states.iter_mut().zip(reference) {
        state.aquifer_ref_water = value;
    }
    Ok(Some(states))
}

fn ensure_dimension(file: &mut netcdf::FileMut, name: &str, len: usize) -> Result<()> {
    match file.dimension(name) {
        Some(dimension) => {
            anyhow::ensure!(
                dimension.len() == len,
                "{name} already has {} entries, expected {len}",
                dimension.len()
            );
        }
        None => {
            file.add_dimension(name, len)?;
        }
    }
    Ok(())
}

fn put_scalar_i32(file: &mut netcdf::FileMut, name: &str, value: i32) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<i32>(name, &[])?,
    };
    variable.put_value(value, ())?;
    Ok(())
}

fn put_scalar_f64(file: &mut netcdf::FileMut, name: &str, value: f64) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<f64>(name, &[])?,
    };
    variable.put_value(value, ())?;
    Ok(())
}

fn put_array_f64(
    file: &mut netcdf::FileMut,
    name: &str,
    dims: &[&str],
    values: &[f64],
) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<f64>(name, dims)?,
    };
    variable.put_values(values, netcdf::Extents::All)?;
    Ok(())
}

fn put_array_i32(
    file: &mut netcdf::FileMut,
    name: &str,
    dims: &[&str],
    values: &[i32],
) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<i32>(name, dims)?,
    };
    variable.put_values(values, netcdf::Extents::All)?;
    Ok(())
}

#[cfg(test)]
#[path = "tracer_tests.rs"]
mod tests;
