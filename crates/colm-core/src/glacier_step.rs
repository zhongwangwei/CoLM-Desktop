//! `CoLMMAIN` 的冰川/冰盖分支（`patchtype == 3`，`CoLMMAIN.F90:1595-1770`）：
//! 首层溢出 → `newsnow` → `GLACIER_TEMP`（`groundfluxes_glacier` + `groundtem_glacier`
//! + `meltf`）→ `GLACIER_WATER` → 地表径流 → 水量闭合。
//!
//! 与规则土壤共用同一份状态（[`StandardLctSnowSoilState`]）：冰层就放在"土壤"那一段，
//! 于是续跑写出、表面光学准备、雪列的合并/分裂都不必再造一份。植被那部分状态在
//! 冰川上不参与计算（上游在 `CoLMMAIN` 末尾把它们清零，见 [`clear_vegetation`]）。
//!
//! 收缩形状逐句对照 `MOD_Glacier.F90` 与 `CoLMMAIN.F90` 的
//! `-fdump-tree-optimized-lineno`（`-O2 -ffp-contract=fast`）；每处 `mul_add` 旁注了行号。

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::standard_lct_step::packed_snow_soil_state;
use crate::{
    add_new_snow, glacier_water, initialize_monin_obukhov, monin_obukhov_with_scheme, net_solar,
    phase_change, saturation_specific_humidity, GlacierSurfaceWater, GlacierWaterInput,
    MoninObukhovInitialInput, MoninObukhovInput, NetSolarFluxes, NetSolarInput, NewSnowInput,
    PhaseChangeInput, PrecipitationState, StandardLctSnowSoilInput, StandardLctSnowSoilState,
    SurfaceLayerScheme,
};

const HVAP: f64 = 2.5104e6;
const HSUB: f64 = 2.8440e6;
const STEFNC: f64 = 5.67e-8;
const TFRZ: f64 = 273.16;
const CPLIQ: f64 = 4188.0;
const CPICE: f64 = 2117.27;
const CPAIR: f64 = 1004.64;
const RGAS: f64 = 287.04;
const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;
const TKWAT: f64 = 0.6;
const TKICE: f64 = 2.290;
const TKAIR: f64 = 0.023;
const VONKAR: f64 = 0.4;
const GRAV: f64 = 9.80616;
/// `GLACIER_TEMP` 里写死的 `emg = 0.97`（不走 `ground_emissivity`）。
const GLACIER_EMISSIVITY: f64 = 0.97;

/// `GLACIER_TEMP` 的全部输出（上游的同名变量）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GlacierThermalFluxes {
    pub taux: f64,
    pub tauy: f64,
    pub fsena: f64,
    pub fevpa: f64,
    pub lfevpa: f64,
    pub fseng: f64,
    pub fevpg: f64,
    pub olrg: f64,
    pub fgrnd: f64,
    pub qseva: f64,
    pub qsdew: f64,
    pub qsubl: f64,
    pub qfros: f64,
    /// `sm`：无雪层时的薄雪融化速率。
    pub sm: f64,
    pub tref: f64,
    pub qref: f64,
    pub trad: f64,
    pub errore: f64,
    pub emis: f64,
    pub z0m: f64,
    pub zol: f64,
    pub rib: f64,
    pub ustar: f64,
    pub qstar: f64,
    pub tstar: f64,
    pub fm: f64,
    pub fh: f64,
    pub fq: f64,
    /// `xmf`：相变潜热。
    pub xmf: f64,
}

/// 一步冰川分支的诊断。
#[derive(Debug, Clone, PartialEq)]
pub struct GlacierStepOutput {
    pub precipitation: PrecipitationState,
    pub shortwave: NetSolarFluxes,
    /// 溢出并入之后的 `pg_rain`/`pg_snow` 与 `t_precip`。
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub thermal: GlacierThermalFluxes,
    /// `imelt`，打包列顺序（雪层在前）。
    pub phase_flag: Vec<i32>,
    /// `gwat`。
    pub water_input_mm_s: f64,
    pub surface_runoff_mm_s: f64,
    pub total_runoff_mm_s: f64,
    /// `errorw`（mm）与 `xerr = errorw/deltim`（VSF 关闭时上游写 0）。
    pub water_balance_error_mm: f64,
    pub water_balance_error_mm_s: f64,
    /// `totwb`：溢出扣除之后的步首总水量，`history` 的 `xerr` 不再需要另算。
    pub initial_total_water_mm: f64,
}

/// 冰川一步。`input` 与规则土壤同一个装配结果：本函数只取它的强迫、时间步、
/// 参考高度、层几何与常数，不用任何植被/土壤水力参数。
pub fn glacier_snow_step(
    input: StandardLctSnowSoilInput<'_>,
    state: &mut StandardLctSnowSoilState,
) -> Result<GlacierStepOutput> {
    let ground = input.energy.ground_temperature;
    ensure!(
        ground.patch_type == 3,
        "the glacier step needs patchtype 3, got {}",
        ground.patch_type
    );
    let dt = ground.time_step_seconds;
    let forcing = input.energy.forcing;
    let variably_saturated = input.soil_water.variably_saturated;
    let ice_layers = state.soil_temperature_k.len();
    ensure!(
        ice_layers >= 2
            && state.soil_water.liquid_water_kg_m2.len() == ice_layers
            && state.soil_water.ice_water_kg_m2.len() == ice_layers,
        "the glacier column needs at least two ice layers with matching water arrays"
    );
    let first_ice_thickness_m = input.soil_water.layer_thickness_m[0];

    // `netsolar`（`CoLMMAIN.F90:772`）用上一步末的 `fsno` 与 `lai/sai`：冰川上后两者
    // 在上一步末已被清零（冷启动重启同样是 0）。
    let shortwave = net_solar(
        NetSolarInput {
            snow_fraction: state.snow.ground_snow_fraction,
            leaf_area_index: state.energy.canopy.leaf_area_index,
            stem_area_index: state.energy.canopy.stem_area_index,
            ..input.energy.solar
        },
        &mut state.energy.radiation,
    )?;
    // `rain_snow_temp (patchtype, ...)`：`patchtype == 3` 打开冰川那一支。
    let precipitation = forcing.partition_precipitation(3, input.energy.precipitation_scheme)?;

    // 步首：`fiold`、`totwb`（`:1606-1626`）。
    remember_snow_ice_fraction(state);
    let mut total_water_before = state.snow.water_equivalent_kg_m2 + soil_water_sum(state);
    if variably_saturated {
        total_water_before += state.soil_water.surface_water_mm;
    }

    let mut rainfall =
        precipitation.convective_rain_kg_m2_s + precipitation.large_scale_rain_kg_m2_s;
    let mut snowfall =
        precipitation.convective_snow_kg_m2_s + precipitation.large_scale_snow_kg_m2_s;
    let mut precipitation_temperature = precipitation.precipitation_temperature_k;

    // 首冰层装不下的液/固水当作本步的降水从顶上重新进来（`:1632-1654`）。
    let mut rain_temperature = precipitation_temperature;
    let liquid_capacity = first_ice_thickness_m * DENH2O;
    if state.soil_water.liquid_water_kg_m2[0] > liquid_capacity {
        let extra = (state.soil_water.liquid_water_kg_m2[0] - liquid_capacity) / dt;
        // `:1635` `.FMA (pg_rain, t_rain, t1*wextra) / (pg_rain + wextra)`
        rain_temperature = rainfall.mul_add(rain_temperature, state.soil_temperature_k[0] * extra)
            / (rainfall + extra);
        rainfall += extra;
        state.soil_water.liquid_water_kg_m2[0] = liquid_capacity;
        total_water_before -= dt * extra;
    }
    let mut snow_temperature = precipitation_temperature;
    let ice_capacity = first_ice_thickness_m * DENICE;
    if state.soil_water.ice_water_kg_m2[0] > ice_capacity {
        let extra = (state.soil_water.ice_water_kg_m2[0] - ice_capacity) / dt;
        // `:1647` 同形。
        snow_temperature = snowfall.mul_add(snow_temperature, state.soil_temperature_k[0] * extra)
            / (snowfall + extra);
        snowfall += extra;
        state.soil_water.ice_water_kg_m2[0] = ice_capacity;
        total_water_before -= dt * extra;
    }
    if rainfall + snowfall > 0.0 {
        // `:1657` `.FMA (pg_rain*cpliq, t_rain, (pg_snow*cpice)*t_snow) / (…+…)`
        let rain_heat = rainfall * CPLIQ;
        let snow_heat = snowfall * CPICE;
        precipitation_temperature = rain_heat
            .mul_add(rain_temperature, snow_heat * snow_temperature)
            / (rain_heat + snow_heat);
    }

    add_new_snow(
        NewSnowInput {
            patch_type: 3,
            time_step_seconds: dt,
            ground_temperature_k: surface_temperature(state),
            ground_snowfall_kg_m2_s: snowfall,
            new_snow_bulk_density_kg_m3: precipitation.new_snow_bulk_density_kg_m3,
            precipitation_temperature_k: precipitation_temperature,
            variably_saturated_flow: variably_saturated,
        },
        &mut state.snow,
    )?;

    // ---- GLACIER_TEMP ----
    let snow_layers = state.snow.layer_count.unsigned_abs() as usize;
    let packed = packed_snow_soil_state(ground, state, snow_layers, ground.snow_layers);
    let column = GlacierColumn {
        thickness_m: packed.layer_thickness_m,
        node_depth_m: packed.node_depth_m,
        interface_depth_m: packed.interface_depth_m,
        temperature_k: packed.temperature_k,
        liquid_water_kg_m2: packed.liquid_water_kg_m2,
        ice_water_kg_m2: packed.ice_water_kg_m2,
    };
    let flux_input = input.energy.ground_flux;
    let temperature = glacier_temperature(
        GlacierTemperatureInput {
            time_step_seconds: dt,
            surface_temperature_factor: ground.surface_temperature_factor,
            crank_nicolson_factor: ground.crank_nicolson_factor,
            wind_height_m: flux_input.wind_height_m,
            temperature_height_m: flux_input.temperature_height_m,
            humidity_height_m: flux_input.humidity_height_m,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            air_temperature_k: forcing.air_temperature_k,
            specific_humidity: forcing.specific_humidity,
            boundary_layer_height_m: forcing.boundary_layer_height_m,
            air_density_kg_m3: forcing.air_density_kg_m3,
            surface_pressure_pa: forcing.surface_pressure_pa,
            absorbed_shortwave_w_m2: shortwave.ground_absorbed_w_m2,
            downward_longwave_w_m2: forcing.downward_longwave_w_m2,
            snow_cover_fraction: state.snow.ground_snow_fraction,
            rainfall_kg_m2_s: rainfall,
            snowfall_kg_m2_s: snowfall,
            precipitation_temperature_k: precipitation_temperature,
            snow_layers,
            snow_water_equivalent_kg_m2: state.snow.water_equivalent_kg_m2,
            snow_depth_m: state.snow.depth_m,
            surface_layer_scheme: flux_input.surface_layer_scheme,
            split_soil_snow: ground.use_split_soil_snow,
            supercool_water: ground.supercool_water,
            soil_porosity: ground.soil_porosity,
            soil_residual_water: ground.soil_residual_water,
            soil_suction_mm: ground.soil_suction_mm,
            soil_hydraulic_model: ground.soil_hydraulic_model,
        },
        column,
    )?;
    let GlacierTemperatureOutput {
        column,
        fluxes: thermal,
        phase_flag,
        snow_water_equivalent_kg_m2,
        snow_depth_m,
    } = temperature;
    for (relative, index) in (state.snow.layer_count + 1..=0).enumerate() {
        let slot = crate::snow::snow_layer_slot(index);
        state.snow.temperature_k[slot] = column.temperature_k[relative];
        state.snow.liquid_water_kg_m2[slot] = column.liquid_water_kg_m2[relative];
        state.snow.ice_water_kg_m2[slot] = column.ice_water_kg_m2[relative];
    }
    state.snow.water_equivalent_kg_m2 = snow_water_equivalent_kg_m2;
    state.snow.depth_m = snow_depth_m;
    state.soil_temperature_k = column.temperature_k[snow_layers..].to_vec();
    state.soil_water.liquid_water_kg_m2 = column.liquid_water_kg_m2[snow_layers..].to_vec();
    state.soil_water.ice_water_kg_m2 = column.ice_water_kg_m2[snow_layers..].to_vec();

    // ---- GLACIER_WATER ----
    let melted = phase_flag[..snow_layers]
        .iter()
        .map(|flag| *flag == 1)
        .collect::<Vec<_>>();
    let mut surface = GlacierSurfaceWater {
        liquid_water_kg_m2: state.soil_water.liquid_water_kg_m2[0],
        ice_water_kg_m2: state.soil_water.ice_water_kg_m2[0],
    };
    let water_input = glacier_water(
        GlacierWaterInput {
            time_step_seconds: dt,
            irreducible_saturation: input.snow_water.irreducible_saturation,
            impermeable_porosity: input.snow_water.impermeable_porosity,
            rainfall_kg_m2_s: rainfall,
            snow_melt_kg_m2_s: thermal.sm,
            evaporation_kg_m2_s: thermal.qseva,
            dew_kg_m2_s: thermal.qsdew,
            sublimation_kg_m2_s: thermal.qsubl,
            frost_kg_m2_s: thermal.qfros,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            melted: &melted,
        },
        &mut state.snow,
        &mut surface,
    )?;
    state.soil_water.liquid_water_kg_m2[0] = surface.liquid_water_kg_m2;
    state.soil_water.ice_water_kg_m2[0] = surface.ice_water_kg_m2;
    // `GLACIER_WATER` 末尾：`snl > maxsnl` 时把空出来的雪槽清零（`MOD_Glacier.F90:972-978`）。
    for index in -(crate::snow::MAX_SNOW_LAYERS as i32) + 1..=state.snow.layer_count {
        let slot = crate::snow::snow_layer_slot(index);
        state.snow.ice_water_kg_m2[slot] = 0.0;
        state.snow.liquid_water_kg_m2[slot] = 0.0;
        state.snow.temperature_k[slot] = 0.0;
        state.snow.node_depth_m[slot] = 0.0;
        state.snow.thickness_m[slot] = 0.0;
    }

    // ---- 地表径流（`CoLMMAIN.F90:1718-1742`）----
    let (surface_runoff, total_runoff) = if variably_saturated {
        // `:1723` `a = .FMA (deltim, gwat, wdsrf + wliq(1))`
        let available = dt.mul_add(
            water_input,
            state.soil_water.surface_water_mm + state.soil_water.liquid_water_kg_m2[0],
        );
        if available > liquid_capacity {
            state.soil_water.liquid_water_kg_m2[0] = liquid_capacity;
            state.soil_water.surface_water_mm = available - liquid_capacity;
        } else {
            state.soil_water.surface_water_mm = 0.0;
            state.soil_water.liquid_water_kg_m2[0] = available.max(1.0e-8);
        }
        let ponding_limit = input.soil_water.ponding_limit_mm;
        let runoff = if state.soil_water.surface_water_mm > ponding_limit {
            let runoff = (state.soil_water.surface_water_mm - ponding_limit) / dt;
            state.soil_water.surface_water_mm = ponding_limit;
            runoff
        } else {
            0.0
        };
        (runoff, runoff)
    } else {
        let runoff = water_input.max(0.0);
        (runoff, runoff)
    };

    // ---- 水量闭合（`:1745-1770`）----
    let mut total_water_after = state.snow.water_equivalent_kg_m2 + soil_water_sum(state);
    if variably_saturated {
        total_water_after += state.soil_water.surface_water_mm;
    }
    // `:1759` `.FNMA (deltim, pg_rain+pg_snow-fevpa-rnof, endwb-totwb)`
    let water_balance_error_mm = (-dt).mul_add(
        snowfall + rainfall - thermal.fevpa - total_runoff,
        total_water_after - total_water_before,
    );
    let water_balance_error_mm_s = if variably_saturated {
        water_balance_error_mm / dt
    } else {
        0.0
    };

    Ok(GlacierStepOutput {
        precipitation,
        shortwave,
        rainfall_kg_m2_s: rainfall,
        snowfall_kg_m2_s: snowfall,
        precipitation_temperature_k: precipitation_temperature,
        thermal,
        phase_flag,
        water_input_mm_s: water_input,
        surface_runoff_mm_s: surface_runoff,
        total_runoff_mm_s: total_runoff,
        water_balance_error_mm,
        water_balance_error_mm_s,
        initial_total_water_mm: total_water_before,
    })
}

/// `CoLMMAIN.F90:2178-2230`：`patchtype > 2` 时把植被与冠层量置零、`tleaf = forc_t`、
/// `zwt = 0`、非 VSF 时 `wa = 4800`、PHS 时 `vegwp = -2.5e4`。
/// 在表面光学（`albland`，它用的是 `tlai`/`tsai`）**之后**做。
pub fn clear_non_soil_patch(
    state: &mut StandardLctSnowSoilState,
    air_temperature_k: f64,
    variably_saturated_flow: bool,
) {
    state.energy.canopy.leaf_area_index = 0.0;
    state.energy.canopy.stem_area_index = 0.0;
    state.energy.canopy.vegetation_free_fraction = 0.0;
    state.energy.leaf.leaf_temperature_k = air_temperature_k;
    state.energy.leaf.canopy_water = crate::CanopyWater {
        total_mm: 0.0,
        rain_mm: 0.0,
        snow_mm: 0.0,
    };
    if let Some(plant) = state.energy.leaf.plant_hydraulics.as_mut() {
        plant.vegetation_water_potential_mm = [-2.5e4; crate::VEGETATION_SEGMENTS];
    }
    state.energy.radiation.sunlit_absorption = Default::default();
    state.energy.radiation.shaded_absorption = Default::default();
    state.energy.radiation.thermal_gap_fraction = 0.0;
    state.energy.radiation.direct_extinction = 0.0;
    state.energy.radiation.diffuse_extinction = 0.0;
    state.soil_water.water_table_depth_m = 0.0;
    if !variably_saturated_flow {
        state.soil_water.aquifer_water_mm = 4800.0;
    }
}

/// `sum(wice_soisno(1:)+wliq_soisno(1:))`：逐层先加冰液、再累加。
fn soil_water_sum(state: &StandardLctSnowSoilState) -> f64 {
    state
        .soil_water
        .ice_water_kg_m2
        .iter()
        .zip(&state.soil_water.liquid_water_kg_m2)
        .fold(0.0, |sum, (ice, liquid)| sum + (ice + liquid))
}

fn surface_temperature(state: &StandardLctSnowSoilState) -> f64 {
    if state.snow.layer_count < 0 {
        state.snow.temperature_k[crate::snow::snow_layer_slot(state.snow.layer_count + 1)]
    } else {
        state.soil_temperature_k[0]
    }
}

fn remember_snow_ice_fraction(state: &mut StandardLctSnowSoilState) {
    let snow = &mut state.snow;
    for index in snow.layer_count + 1..=0 {
        let slot = crate::snow::snow_layer_slot(index);
        snow.previous_ice_fraction[slot] = snow.ice_water_kg_m2[slot]
            / (snow.liquid_water_kg_m2[slot] + snow.ice_water_kg_m2[slot]);
    }
}

/// 打包的雪 + 冰列（上游下标 `lb..nl_ice`，界面 `lb-1..nl_ice`）。
#[derive(Debug, Clone, PartialEq)]
pub struct GlacierColumn {
    pub thickness_m: Vec<f64>,
    pub node_depth_m: Vec<f64>,
    pub interface_depth_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
}

/// `GLACIER_TEMP` 的标量输入。
#[derive(Debug, Clone, Copy)]
pub struct GlacierTemperatureInput<'a> {
    pub time_step_seconds: f64,
    /// `capr`
    pub surface_temperature_factor: f64,
    /// `cnfac`
    pub crank_nicolson_factor: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub boundary_layer_height_m: Option<f64>,
    pub air_density_kg_m3: f64,
    pub surface_pressure_pa: f64,
    /// `sabg`
    pub absorbed_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    /// `fsno`：只决定冰面/雪面粗糙度（`groundfluxes_glacier`）。
    pub snow_cover_fraction: f64,
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub snow_layers: usize,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub surface_layer_scheme: SurfaceLayerScheme,
    pub split_soil_snow: bool,
    pub supercool_water: bool,
    /// `meltf` 的土壤参数：冰川上它们只在 `patchtype <= 2` 的支路里被读，照传。
    pub soil_porosity: &'a [f64],
    pub soil_residual_water: &'a [f64],
    pub soil_suction_mm: &'a [f64],
    pub soil_hydraulic_model: &'a [crate::SoilHydraulicModel],
}

/// `GLACIER_TEMP` 的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct GlacierTemperatureOutput {
    pub column: GlacierColumn,
    pub fluxes: GlacierThermalFluxes,
    pub phase_flag: Vec<i32>,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
}

struct TurbulentFluxes {
    cgrnd: f64,
    cgrndl: f64,
    cgrnds: f64,
    taux: f64,
    tauy: f64,
    fseng: f64,
    fevpg: f64,
    tref: f64,
    qref: f64,
    z0m: f64,
    zol: f64,
    rib: f64,
    ustar: f64,
    qstar: f64,
    tstar: f64,
    fm: f64,
    fh: f64,
    fq: f64,
}

/// Port of `MOD_Glacier:GLACIER_TEMP`（非 SNICAR）。
pub fn glacier_temperature(
    input: GlacierTemperatureInput<'_>,
    mut column: GlacierColumn,
) -> Result<GlacierTemperatureOutput> {
    let n = column.temperature_k.len();
    ensure!(
        n >= 2
            && column.thickness_m.len() == n
            && column.node_depth_m.len() == n
            && column.interface_depth_m.len() == n + 1
            && column.liquid_water_kg_m2.len() == n
            && column.ice_water_kg_m2.len() == n
            && input.snow_layers < n,
        "the packed glacier column is inconsistent"
    );
    let dt = input.time_step_seconds;
    let previous_temperature = column.temperature_k.clone();
    let ground_temperature = column.temperature_k[0];

    let latent_heat = if column.liquid_water_kg_m2[0] <= 0.0 && column.ice_water_kg_m2[0] > 0.0 {
        HSUB
    } else {
        HVAP
    };
    // `:218` `thm = .FMA (forc_hgt_t, 0.0098, forc_t)`
    let reference_temperature =
        crate::reference_height_temperature_k(input.air_temperature_k, input.temperature_height_m);
    let potential_temperature =
        input.air_temperature_k * (100_000.0 / input.surface_pressure_pa).lpow(RGAS / CPAIR);
    // `:221` `.FMA (forc_q, 0.61, 1.0)`，与 `groundfluxes_glacier` 的 `(1.+0.61*qm)` 同一项。
    let one_plus_061_humidity = input.specific_humidity.mul_add(0.61, 1.0);
    let virtual_potential_temperature = potential_temperature * one_plus_061_humidity;
    // `:222` `sqrt(.FMA (us, us, vs*vs))`
    let reference_wind = input
        .eastward_wind_m_s
        .mul_add(
            input.eastward_wind_m_s,
            input.northward_wind_m_s * input.northward_wind_m_s,
        )
        .sqrt()
        .max(0.1);
    let saturation = saturation_specific_humidity(ground_temperature, input.surface_pressure_pa)?;
    let ground_humidity = saturation.specific_humidity;
    let ground_humidity_slope = saturation.specific_humidity_temperature_slope_k;

    let turbulent = glacier_ground_fluxes(
        input,
        reference_temperature,
        potential_temperature,
        virtual_potential_temperature,
        one_plus_061_humidity,
        reference_wind,
        ground_temperature,
        ground_humidity,
        ground_humidity_slope,
        latent_heat,
    )?;

    let solved = glacier_ground_temperature(
        input,
        &mut column,
        turbulent.fseng,
        turbulent.fevpg,
        turbulent.cgrnd,
        latent_heat,
    )?;

    let surface_temperature = column.temperature_k[0];
    let increment = surface_temperature - previous_temperature[0];
    // `:263-264`
    let mut fseng = increment.mul_add(turbulent.cgrnds, turbulent.fseng);
    let mut fevpg = increment.mul_add(turbulent.cgrndl, turbulent.fevpg);
    let evaporation_limit = (column.ice_water_kg_m2[0] + column.liquid_water_kg_m2[0]) / dt;
    let excess = (fevpg - evaporation_limit).max(0.0);
    fevpg = fevpg.min(evaporation_limit);
    // `:274` `.FMA (egidif, htvp, fseng)`
    fseng = excess.mul_add(latent_heat, fseng);
    let latent = fevpg * latent_heat;

    let (mut qseva, mut qsubl, mut qfros, mut qsdew) = (0.0, 0.0, 0.0, 0.0);
    if fevpg >= 0.0 {
        qseva = fevpg.min(column.liquid_water_kg_m2[0] / dt);
        qsubl = fevpg - qseva;
    } else if surface_temperature < TFRZ {
        qfros = fevpg.abs();
    } else {
        qsdew = fevpg.abs();
    }

    let t0 = previous_temperature[0];
    let t0_squared = t0 * t0;
    let t0_cubed = t0 * t0_squared;
    let emissive = GLACIER_EMISSIVITY * STEFNC;
    let precipitation_gap = input.precipitation_temperature_k - surface_temperature;
    let rain_heat = input.rainfall_kg_m2_s * CPLIQ;
    let snow_heat = input.snowfall_kg_m2_s * CPICE;
    // `:302` `fgrnd`：`sabg + emg*frl` 起头，`t^3*(4*tinc+t)` 的乘积进 FNMA，
    // 两个降水热项各自一条 FMA。
    let absorbed =
        input.absorbed_shortwave_w_m2 + input.downward_longwave_w_m2 * GLACIER_EMISSIVITY;
    let mut ground_heat =
        (-(t0_cubed * emissive)).mul_add(increment.mul_add(4.0, t0), absorbed) - (fseng + latent);
    ground_heat = precipitation_gap.mul_add(rain_heat, ground_heat);
    ground_heat = precipitation_gap.mul_add(snow_heat, ground_heat);
    // `:307` `olrg`
    let longwave_up = (t0_cubed * (4.0 * GLACIER_EMISSIVITY * STEFNC)).mul_add(
        increment,
        input.downward_longwave_w_m2.mul_add(
            1.0 - GLACIER_EMISSIVITY,
            (t0_squared * t0_squared) * emissive,
        ),
    );
    let radiative_temperature = (longwave_up / STEFNC).lpow(0.25);
    // `:321`
    let mut energy_error = input.absorbed_shortwave_w_m2 + input.downward_longwave_w_m2
        - longwave_up
        - fseng
        - latent
        - solved.latent_heat_flux_w_m2;
    energy_error = precipitation_gap.mul_add(rain_heat, energy_error);
    energy_error = precipitation_gap.mul_add(snow_heat, energy_error);
    for ((now, before), fact) in column
        .temperature_k
        .iter()
        .zip(&previous_temperature)
        .zip(&solved.fact)
    {
        energy_error -= (now - before) / fact;
    }

    Ok(GlacierTemperatureOutput {
        fluxes: GlacierThermalFluxes {
            taux: turbulent.taux,
            tauy: turbulent.tauy,
            fsena: fseng,
            fevpa: fevpg,
            lfevpa: latent,
            fseng,
            fevpg,
            olrg: longwave_up,
            fgrnd: ground_heat,
            qseva,
            qsdew,
            qsubl,
            qfros,
            sm: solved.snow_melt_rate_kg_m2_s,
            tref: turbulent.tref,
            qref: turbulent.qref,
            trad: radiative_temperature,
            errore: energy_error,
            emis: GLACIER_EMISSIVITY,
            z0m: turbulent.z0m,
            zol: turbulent.zol,
            rib: turbulent.rib,
            ustar: turbulent.ustar,
            qstar: turbulent.qstar,
            tstar: turbulent.tstar,
            fm: turbulent.fm,
            fh: turbulent.fh,
            fq: turbulent.fq,
            xmf: solved.latent_heat_flux_w_m2,
        },
        column,
        phase_flag: solved.phase_flag,
        snow_water_equivalent_kg_m2: solved.snow_water_equivalent_kg_m2,
        snow_depth_m: solved.snow_depth_m,
    })
}

/// Port of `MOD_Glacier:groundfluxes_glacier`。
#[allow(clippy::too_many_arguments)]
fn glacier_ground_fluxes(
    input: GlacierTemperatureInput<'_>,
    reference_temperature: f64,
    potential_temperature: f64,
    virtual_potential_temperature: f64,
    one_plus_061_humidity: f64,
    reference_wind: f64,
    ground_temperature: f64,
    ground_humidity: f64,
    ground_humidity_slope: f64,
    latent_heat: f64,
) -> Result<TurbulentFluxes> {
    // Table 1 of Brock et al. (2006)
    let momentum_roughness = if input.snow_cover_fraction > 0.0 {
        0.002
    } else {
        0.001
    };
    let mut heat_roughness = momentum_roughness;
    let temperature_difference = reference_temperature - ground_temperature;
    let humidity_difference = input.specific_humidity - ground_humidity;
    let potential_061 = potential_temperature * 0.61;
    // `:480` `.FMA (1+0.61*qm, dth, dqh*(th*0.61))`
    let virtual_difference =
        one_plus_061_humidity.mul_add(temperature_difference, humidity_difference * potential_061);
    let reference_height = input.wind_height_m;
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: reference_wind,
        potential_temperature_k: potential_temperature,
        reference_temperature_k: reference_temperature,
        virtual_potential_temperature_k: virtual_potential_temperature,
        temperature_difference_k: temperature_difference,
        humidity_difference_kg_kg: humidity_difference,
        virtual_temperature_difference_k: virtual_difference,
        reference_height_m: reference_height,
        momentum_roughness_m: momentum_roughness,
    })?;
    let mut stability_wind = initial.stability_adjusted_wind_m_s;
    let mut obukhov = initial.obukhov_length_m;
    let mut previous_obukhov = 0.0;
    let mut sign_changes = 0;
    let mut boundary_height = 1000.0;
    let mut profile;
    let mut temperature_scale;
    let mut humidity_scale;
    let mut zeta;
    let mut iteration = 0;
    loop {
        iteration += 1;
        profile = monin_obukhov_with_scheme(
            MoninObukhovInput {
                wind_height_m: input.wind_height_m,
                temperature_height_m: input.temperature_height_m,
                humidity_height_m: input.humidity_height_m,
                displacement_height_m: 0.0,
                momentum_roughness_m: momentum_roughness,
                heat_roughness_m: heat_roughness,
                moisture_roughness_m: heat_roughness,
                obukhov_length_m: obukhov,
                stability_adjusted_wind_m_s: stability_wind,
                boundary_layer_height_m: input.boundary_layer_height_m,
            },
            input.surface_layer_scheme,
        )?;
        let ustar = profile.friction_velocity_m_s;
        temperature_scale = temperature_difference * (VONKAR / profile.heat);
        humidity_scale = humidity_difference * (VONKAR / profile.moisture);
        heat_roughness =
            momentum_roughness / ((momentum_roughness * ustar / 1.5e-5).lpow(0.45) * 0.13).exp();
        // `:506` `.FMA (1+0.61*qm, tstar, (th*0.61)*qstar)`
        let virtual_scale =
            one_plus_061_humidity.mul_add(temperature_scale, potential_061 * humidity_scale);
        zeta = virtual_scale * ((reference_height * VONKAR) * GRAV)
            / (virtual_potential_temperature * (ustar * ustar));
        zeta = if zeta >= 0.0 {
            zeta.clamp(1.0e-6, 2.0)
        } else {
            zeta.clamp(-100.0, -1.0e-6)
        };
        obukhov = reference_height / zeta;
        stability_wind = if zeta >= 0.0 {
            reference_wind.max(0.1)
        } else {
            if input.surface_layer_scheme == SurfaceLayerScheme::LargeEddy {
                let hpbl = input.boundary_layer_height_m.ok_or_else(|| {
                    anyhow::anyhow!(
                        "the large-eddy surface-layer scheme needs the forcing's boundary-layer height"
                    )
                })?;
                boundary_height = (5.0 * input.wind_height_m).max(hpbl);
            }
            let convective = (-((ustar * GRAV) * virtual_scale * boundary_height
                / virtual_potential_temperature))
                .lpow(1.0 / 3.0);
            // `:523` `sqrt(.FMA (ur, ur, wc*wc))`
            reference_wind
                .mul_add(reference_wind, convective * convective)
                .sqrt()
        };
        if previous_obukhov * obukhov < 0.0 {
            sign_changes += 1;
        }
        if sign_changes >= 4 || iteration == 6 {
            break;
        }
        previous_obukhov = obukhov;
    }

    let ustar = profile.friction_velocity_m_s;
    let ram = 1.0 / ((ustar * ustar) / stability_wind);
    let rah = 1.0 / ((VONKAR / profile.heat) * ustar);
    let raw = 1.0 / ((VONKAR / profile.moisture) * ustar);
    let raih = (input.air_density_kg_m3 * CPAIR) / rah;
    let raiw = input.air_density_kg_m3 / raw;
    let cgrndl = ground_humidity_slope * raiw;
    // `:544` `.FMA (cgrndl, htvp, cgrnds)`
    let cgrnd = cgrndl.mul_add(latent_heat, raih);
    let richardson = ((ustar * ustar) * zeta)
        / ((stability_wind * stability_wind) * ((VONKAR * VONKAR) / profile.heat));
    Ok(TurbulentFluxes {
        cgrnd,
        cgrndl,
        cgrnds: raih,
        taux: -((input.eastward_wind_m_s * input.air_density_kg_m3) / ram),
        tauy: -((input.northward_wind_m_s * input.air_density_kg_m3) / ram),
        fseng: -(temperature_difference * raih),
        fevpg: -(humidity_difference * raiw),
        // `:560-561` `.FMA (tstar, fh2m/vonkar - fh/vonkar, thm)`
        tref: temperature_scale.mul_add(
            profile.heat_at_2m / VONKAR - profile.heat / VONKAR,
            reference_temperature,
        ),
        qref: humidity_scale.mul_add(
            profile.moisture_at_2m / VONKAR - profile.moisture / VONKAR,
            input.specific_humidity,
        ),
        z0m: momentum_roughness,
        zol: zeta,
        rib: richardson.min(5.0),
        ustar,
        qstar: humidity_scale,
        tstar: temperature_scale,
        fm: profile.momentum,
        fh: profile.heat,
        fq: profile.moisture,
    })
}

struct SolvedTemperature {
    fact: Vec<f64>,
    latent_heat_flux_w_m2: f64,
    snow_melt_rate_kg_m2_s: f64,
    phase_flag: Vec<i32>,
    snow_water_equivalent_kg_m2: f64,
    snow_depth_m: f64,
}

/// Port of `MOD_Glacier:groundtem_glacier`（非 SNICAR）。
fn glacier_ground_temperature(
    input: GlacierTemperatureInput<'_>,
    column: &mut GlacierColumn,
    fseng: f64,
    fevpg: f64,
    cgrnd: f64,
    latent_heat: f64,
) -> Result<SolvedTemperature> {
    let n = column.temperature_k.len();
    let snow = input.snow_layers;
    let dt = input.time_step_seconds;
    let z = &column.node_depth_m;
    let zi = &column.interface_depth_m;
    let t = &column.temperature_k;

    // 热容（`:684-690`）：冰层 `.FMA (wice, cpice, wliq*cpliq)`，雪层 `.FMA (wliq, cpliq, wice*cpice)`。
    let mut heat_capacity = column
        .liquid_water_kg_m2
        .iter()
        .zip(&column.ice_water_kg_m2)
        .enumerate()
        .map(|(layer, (liquid, ice))| {
            if layer < snow {
                liquid.mul_add(CPLIQ, ice * CPICE)
            } else {
                ice.mul_add(CPICE, liquid * CPLIQ)
            }
        })
        .collect::<Vec<_>>();
    if snow == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        heat_capacity[0] = input
            .snow_water_equivalent_kg_m2
            .mul_add(CPICE, heat_capacity[0]);
    }

    // 导热率（`:692-705`）。
    let mut conductivity = vec![0.0; n];
    for layer in 0..n {
        conductivity[layer] = if t[layer] <= TFRZ {
            (-(t[layer] * 0.0057)).exp() * 9.828
        } else {
            TKWAT
        };
    }
    for (((value, ice), liquid), thickness) in conductivity[..snow]
        .iter_mut()
        .zip(&column.ice_water_kg_m2)
        .zip(&column.liquid_water_kg_m2)
        .zip(&column.thickness_m)
    {
        let density = (ice + liquid) / thickness;
        // `:704` `.FMA (.FMA (rho, 7.75e-5, rho*(rho*1.105e-6)), tkice-tkair, tkair)`
        *value = density
            .mul_add(7.75e-5, density * (density * 1.105e-6))
            .mul_add(TKICE - TKAIR, TKAIR);
    }
    let mut interface_conductivity = vec![0.0; n];
    for layer in 0..n - 1 {
        let below = zi[layer + 1];
        interface_conductivity[layer] =
            if snow > 0 && layer == snow - 1 && z[layer + 1] - below < below - z[layer] {
                let harmonic = 2.0 * conductivity[layer] * conductivity[layer + 1]
                    / (conductivity[layer] + conductivity[layer + 1]);
                harmonic.max(0.5 * conductivity[layer + 1])
            } else {
                // `:733` 分母 `.FMA (thk(j), z(j+1)-zi(j), thk(j+1)*(zi(j)-z(j)))`
                conductivity[layer] * conductivity[layer + 1] * (z[layer + 1] - z[layer])
                    / conductivity[layer].mul_add(
                        z[layer + 1] - below,
                        conductivity[layer + 1] * (below - z[layer]),
                    )
            };
    }

    // `hs`/`dhsdT`（`:747-750`）。
    let t0 = t[0];
    let t0_squared = t0 * t0;
    let emissive = GLACIER_EMISSIVITY * STEFNC;
    let rain_heat = input.rainfall_kg_m2_s * CPLIQ;
    let snow_heat = input.snowfall_kg_m2_s * CPICE;
    let precipitation_gap = input.precipitation_temperature_k - t0;
    let surface_flux = input.absorbed_shortwave_w_m2
        + input.downward_longwave_w_m2 * GLACIER_EMISSIVITY
        - (t0_squared * t0_squared) * emissive
        - latent_heat.mul_add(fevpg, fseng)
        + rain_heat * precipitation_gap
        + precipitation_gap * snow_heat;
    // `:750` `.FNMS (t^3, 4*emg*stefnc, cgrnd) - cpliq*pg_rain - cpice*pg_snow`
    let flux_derivative = (-(t0 * t0_squared)).mul_add(4.0 * GLACIER_EMISSIVITY * STEFNC, -cgrnd)
        - rain_heat
        - snow_heat;

    let previous = t.clone();
    let mut fact = vec![0.0; n];
    // `:755` `(deltim/cv)*dz / (.FMA (capr, z(lb+1)-zi(lb-1), z(lb)-zi(lb-1))*0.5)`
    fact[0] = dt / heat_capacity[0] * column.thickness_m[0]
        / (input
            .surface_temperature_factor
            .mul_add(z[1] - zi[0], z[0] - zi[0])
            * 0.5);
    for layer in 1..n {
        fact[layer] = dt / heat_capacity[layer];
    }
    let mut flux = vec![0.0; n];
    for layer in 0..n - 1 {
        flux[layer] =
            (t[layer + 1] - t[layer]) * interface_conductivity[layer] / (z[layer + 1] - z[layer]);
    }

    let cnfac = input.crank_nicolson_factor;
    let implicit = 1.0 - cnfac;
    let mut lower = vec![0.0; n];
    let mut diagonal = vec![0.0; n];
    let mut upper = vec![0.0; n];
    let mut rhs = vec![0.0; n];
    {
        let dzp = z[1] - z[0];
        let coupling = implicit * fact[0] * interface_conductivity[0] / dzp;
        // `:770` `.FNMA (dhsdT, fact, 1 + …)`；`:772` `.FMA (fact, .FNMA (dhsdT, t, hs) + cnfac*fn, t)`
        diagonal[0] = (-flux_derivative).mul_add(fact[0], coupling + 1.0);
        upper[0] = -coupling;
        rhs[0] = fact[0].mul_add(
            (-flux_derivative).mul_add(t0, surface_flux) + cnfac * flux[0],
            t0,
        );
    }
    // 雪层与冰层 1（`:775-784`）和内部冰层（`:787-796`）同形；非 SNICAR 时
    // `sabg_snow_lyr` 全为 0，`.FMA (0, fact, t) = t`。
    for layer in 1..n - 1 {
        let dzm = z[layer] - z[layer - 1];
        let dzp = z[layer + 1] - z[layer];
        let implicit_fact = implicit * fact[layer];
        lower[layer] = -(interface_conductivity[layer - 1] * implicit_fact / dzm);
        diagonal[layer] = (interface_conductivity[layer - 1] / dzm
            + interface_conductivity[layer] / dzp)
            .mul_add(implicit_fact, 1.0);
        upper[layer] = -(interface_conductivity[layer] * implicit_fact / dzp);
        rhs[layer] = (flux[layer] - flux[layer - 1]).mul_add(cnfac * fact[layer], t[layer]);
    }
    {
        let layer = n - 1;
        let dzm = z[layer] - z[layer - 1];
        let coupling = implicit * fact[layer] * interface_conductivity[layer - 1] / dzm;
        lower[layer] = -coupling;
        diagonal[layer] = coupling + 1.0;
        upper[layer] = 0.0;
        // `:802` `.FNMA (cnfac*fact, fn(j-1), t)`
        rhs[layer] = (-(cnfac * fact[layer])).mul_add(flux[layer - 1], t[layer]);
    }
    let solved = crate::linear::solve_tridiagonal(&lower, &diagonal, &upper, &rhs)
        .map_err(|message| anyhow::anyhow!(message))?;

    let mut solved_flux = vec![0.0; n];
    for layer in 0..n - 1 {
        solved_flux[layer] = (solved[layer + 1] - solved[layer]) * interface_conductivity[layer]
            / (z[layer + 1] - z[layer]);
    }
    let mut residual = vec![0.0; n];
    // `:818` `.FMA (1-cnfac, fn1, cnfac*fn)`；`:821` `.FMA (cnfac, Δfn, (1-cnfac)*Δfn1)`
    residual[0] = implicit.mul_add(solved_flux[0], cnfac * flux[0]);
    for layer in 1..n {
        residual[layer] = cnfac.mul_add(
            flux[layer] - flux[layer - 1],
            implicit * (solved_flux[layer] - solved_flux[layer - 1]),
        );
    }

    let phase = phase_change(PhaseChangeInput {
        patch_type: 3,
        is_dry_lake: false,
        time_step_seconds: dt,
        fact_seconds_per_j_m2_k: &fact,
        residual_heat_flux_w_m2: &residual,
        snow_layer_absorption_w_m2: None,
        surface_heat_flux_w_m2: surface_flux,
        soil_heat_flux_w_m2: surface_flux,
        snow_heat_flux_w_m2: surface_flux,
        snow_cover_fraction: 1.0,
        surface_heat_flux_temperature_derivative_w_m2_k: flux_derivative,
        previous_temperature_k: &previous,
        temperature_k: &solved,
        liquid_water_kg_m2: &column.liquid_water_kg_m2,
        ice_water_kg_m2: &column.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_layers: snow,
        split_soil_snow: input.split_soil_snow,
        supercool_water: input.supercool_water,
        soil_layer_thickness_m: &column.thickness_m[snow..],
        soil_porosity: input.soil_porosity,
        soil_residual_water: input.soil_residual_water,
        soil_suction_mm: input.soil_suction_mm,
        soil_hydraulic_model: input.soil_hydraulic_model,
    })?;
    column.temperature_k = phase.temperature_k;
    column.liquid_water_kg_m2 = phase.liquid_water_kg_m2;
    column.ice_water_kg_m2 = phase.ice_water_kg_m2;
    Ok(SolvedTemperature {
        fact,
        latent_heat_flux_w_m2: phase.latent_heat_flux_w_m2,
        snow_melt_rate_kg_m2_s: phase.snow_melt_rate_kg_m2_s,
        phase_flag: phase.phase_flag,
        snow_water_equivalent_kg_m2: phase.snow_water_equivalent_kg_m2,
        snow_depth_m: phase.snow_depth_m,
    })
}

#[cfg(test)]
#[path = "glacier_step_tests.rs"]
mod glacier_step_tests;
