//! `CoLMMAIN` 的陆地水体分支（`patchtype == 4`，非动态湖，`CoLMMAIN.F90:1788-1998`）：
//! `newsnow_lake` → `laketem` → `snowwater_lake` → 径流与水量闭合。
//!
//! 与土壤/冰川共用同一份状态（[`StandardLctSnowSoilState`]）：雪列放在 `snow`，
//! 湖底土层放在"土壤"那一段；湖层自身（`dz_lake`/`t_lake`/`lake_icefrac`）、`savedtke1`
//! 与湖面温度 `t_grnd` 放在 [`RuntimeLakeState`]。`t_grnd` 在湖上**不是**任何一层的
//! 温度（`laketem` 自己解一个表面能量平衡），所以必须随状态保存，下一步迭代从它起步。
//!
//! 收缩形状逐句对照 `CoLMMAIN.F90` 的 `-fdump-tree-optimized-lineno`。

use anyhow::{ensure, Context, Result};

use crate::standard_lct_step::packed_snow_soil_state;
use crate::{
    add_lake_new_snow, lake_snow_water_with_snicar, lake_temperature, net_solar, GlacierColumn,
    LakeColumn, LakeNewSnowInput, LakeSnowWaterFluxes, LakeSnowWaterInput, LakeSnowWaterSoil,
    LakeTemperatureInput, LakeTemperatureState, LakeThermalFluxes, NetSolarFluxes, NetSolarInput,
    PrecipitationState, StandardLctSnowSoilInput, StandardLctSnowSoilState,
};

/// 湖 patch 的运行态（上游 `MOD_Vars_TimeVariables` 的湖量 + `t_grnd`）。
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeLakeState {
    /// `dz_lake`/`t_lake`/`lake_icefrac`，自上而下。
    pub column: LakeColumn,
    /// `savedtke1`：上一步表层的涡扩散导热率，开水面时作为表面导热率。
    pub saved_tke: f64,
    /// `t_grnd`
    pub ground_temperature_k: f64,
}

/// 湖 patch 的时不变量（常数重启里的 `patchlatr` 与 `lakedepth`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSite {
    pub latitude_radians: f64,
    pub depth_m: f64,
    /// `DEF_USE_Dynamic_Lake`：湖层厚随水量变，`wdsrf` 记湖水、超出湖深的部分溢出。
    pub dynamic: bool,
}

impl LakeSite {
    /// `is_dry_lake = DEF_USE_Dynamic_Lake .and. patchtype == 4 .and. (wdsrf < 100 .or. zwt > 0)`
    /// （`CoLMMAIN.F90:794-795`）：步首判定，整步走土壤分支。
    pub fn is_dry(&self, state: &StandardLctSnowSoilState) -> bool {
        self.dynamic
            && (state.soil_water.surface_water_mm < 100.0
                || state.soil_water.water_table_depth_m > 0.0)
    }
}

/// 干湖步末（`CoLMMAIN.F90:1455-1469`）：`t_grnd = t_soisno(lb)`，湖层由地表积水重建 ——
/// 等分 `wdsrf`、温度取第一层土温、冰比按冰点取 0/1，积水够 100 mm 时再按标准分层重排。
pub fn refill_dry_lake(state: &mut StandardLctSnowSoilState) -> Result<()> {
    let surface_water_mm = state.soil_water.surface_water_mm;
    let top_soil_temperature_k = state.soil_temperature_k[0];
    let surface_temperature_k = if state.snow.layer_count < 0 {
        state.snow.temperature_k[crate::snow::snow_layer_slot(state.snow.layer_count + 1)]
    } else {
        top_soil_temperature_k
    };
    let lake = state
        .lake
        .as_mut()
        .context("a dry lake step needs the lake state")?;
    refill_lake_column(
        lake,
        surface_water_mm,
        top_soil_temperature_k,
        surface_temperature_k,
    )
}

fn refill_lake_column(
    lake: &mut RuntimeLakeState,
    surface_water_mm: f64,
    top_soil_temperature_k: f64,
    surface_temperature_k: f64,
) -> Result<()> {
    // 湖 patch 的 `t_grnd` 存在湖状态里。
    lake.ground_temperature_k = surface_temperature_k;
    let layers = lake.column.thickness_m.len();
    // `dz_lake = wdsrf*1.e-3/nl_lake`：GIMPLE 是 `(wdsrf*1e-3)/10`。
    let thickness_m = surface_water_mm * 1.0e-3 / layers as f64;
    lake.column.thickness_m.fill(thickness_m);
    lake.column.temperature_k.fill(top_soil_temperature_k);
    let ice_fraction = if top_soil_temperature_k >= crate::FREEZING_K {
        0.0
    } else {
        1.0
    };
    lake.column.ice_fraction.fill(ice_fraction);
    if surface_water_mm >= 100.0 {
        crate::adjust_lake_layers(&mut lake.column)?;
    }
    Ok(())
}

/// 一步湖分支的诊断。
#[derive(Debug, Clone, PartialEq)]
pub struct LakeStepOutput {
    pub precipitation: PrecipitationState,
    pub shortwave: NetSolarFluxes,
    /// `newsnow_lake` 之后的 `pg_rain`/`pg_snow`。
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
    /// `laketem` 的通量；`fseng`/`fgrnd`/`sm` 已含 `snowwater_lake` 的修正。
    pub thermal: LakeThermalFluxes,
    /// `imelt`，打包列顺序（雪层在前）。
    pub phase_flag: Vec<i32>,
    pub surface_runoff_mm_s: f64,
    pub total_runoff_mm_s: f64,
    /// `lake_deficit`：蒸发超过来水时从湖里"借"的水（mm/s）。
    pub lake_deficit_mm_s: f64,
    /// `errorw`（mm）；非动态湖上 `xerr` 恒为 0。
    pub water_balance_error_mm: f64,
    pub initial_total_water_mm: f64,
    /// `endwb`（非动态湖已扣 `lake_deficit*deltim`）。
    pub final_total_water_mm: f64,
}

/// 湖一步。`input` 与规则土壤同一个装配结果：本函数只取它的强迫、时间步、参考高度、
/// 层几何、湖底土层的热参数与孔隙度，不用任何植被参数。
pub fn lake_snow_step(
    input: StandardLctSnowSoilInput<'_>,
    site: LakeSite,
    state: &mut StandardLctSnowSoilState,
) -> Result<LakeStepOutput> {
    let ground = input.energy.ground_temperature;
    ensure!(
        ground.patch_type == 4,
        "the lake step needs patchtype 4, got {}",
        ground.patch_type
    );
    ensure!(
        state.lake.is_some(),
        "a lake patch needs its lake state (t_lake, lake_icefrac, dz_lake, savedtke1)"
    );
    let dt = ground.time_step_seconds;
    let forcing = input.energy.forcing;
    let soil_layers = state.soil_temperature_k.len();
    ensure!(
        state.soil_water.liquid_water_kg_m2.len() == soil_layers
            && state.soil_water.ice_water_kg_m2.len() == soil_layers,
        "the lake-bed soil columns disagree on their layer count"
    );

    ensure!(
        input.snicar.is_some() == state.snicar.is_some(),
        "the SNICAR step input and the SNICAR snow state disagree"
    );
    // `snofrz(:) = 0`（`CoLMMAIN.F90:743`）。
    if let Some(snicar) = state.snicar.as_mut() {
        snicar.refreezing_kg_m2_s = [0.0; 5];
    }
    // `netsolar`（`CoLMMAIN.F90:772`）：`lai/sai` 在上一步末已被清零。
    let mut shortwave = net_solar(
        NetSolarInput {
            snow_fraction: state.snow.ground_snow_fraction,
            leaf_area_index: state.energy.canopy.leaf_area_index,
            stem_area_index: state.energy.canopy.stem_area_index,
            ..input.energy.solar
        },
        &mut state.energy.radiation,
    )?;
    // SNICAR：`netsolar` 末段的分层吸收（夜间全 0，`ssno_lyr` 不动）。
    let snow_layer_absorption = state.snicar.as_mut().map(|snicar| {
        if input.energy.solar.forcing.total() > 0.0 {
            crate::snicar_net_solar(
                &mut snicar.layer_absorption,
                state.energy.radiation.snow_absorption,
                input.energy.solar.forcing,
                state.snow.ground_snow_fraction,
                &mut shortwave.soil_absorbed_w_m2,
                &mut shortwave.snow_absorbed_w_m2,
            )
        } else {
            [0.0; 6]
        }
    });
    let precipitation = forcing.partition_precipitation(4, input.energy.precipitation_scheme)?;

    // 步首（`:1793-1822`）：`totwb = scv + sum(wice+wliq) + wa`，逐层先加冰液再累加；
    // `w_old = sum(wliq) + sum(wice)` 是两个独立的和。
    let mut total_water_before = (state.snow.water_equivalent_kg_m2 + soil_water_sum(state))
        + state.soil_water.aquifer_water_mm;
    // `totwb = totwb + wdsrf`（动态湖，`CoLMMAIN.F90:1800-1802`）。
    if site.dynamic {
        total_water_before += state.soil_water.surface_water_mm;
    }
    remember_snow_ice_fraction(state);
    let water_before = liquid_sum(state) + ice_sum(state);
    let snow_before = state.snow.water_equivalent_kg_m2;

    let lake = state.lake.as_mut().expect("checked above");
    let new_snow = add_lake_new_snow(
        LakeNewSnowInput {
            use_dynamic_lake: site.dynamic,
            time_step_seconds: dt,
            rainfall_kg_m2_s: precipitation.convective_rain_kg_m2_s
                + precipitation.large_scale_rain_kg_m2_s,
            snowfall_kg_m2_s: precipitation.convective_snow_kg_m2_s
                + precipitation.large_scale_snow_kg_m2_s,
            precipitation_temperature_k: precipitation.precipitation_temperature_k,
            new_snow_bulk_density_kg_m3: precipitation.new_snow_bulk_density_kg_m3,
        },
        &mut state.snow,
        &mut lake.column,
    )?;
    let rainfall = new_snow.rainfall_kg_m2_s;
    let snowfall = new_snow.snowfall_kg_m2_s;

    // ---- laketem ----
    let snow_layers = state.snow.layer_count.unsigned_abs() as usize;
    let packed = packed_snow_soil_state(ground, state, snow_layers, ground.snow_layers);
    // SNICAR 的 `wice_soisno_bef(lb:0)`。
    let snow_ice_before = packed.ice_water_kg_m2[..snow_layers].to_vec();
    let mut column = GlacierColumn {
        thickness_m: packed.layer_thickness_m,
        node_depth_m: packed.node_depth_m,
        interface_depth_m: packed.interface_depth_m,
        temperature_k: packed.temperature_k,
        liquid_water_kg_m2: packed.liquid_water_kg_m2,
        ice_water_kg_m2: packed.ice_water_kg_m2,
    };
    let flux_input = input.energy.ground_flux;
    let mut snow_water_equivalent = state.snow.water_equivalent_kg_m2;
    let mut snow_depth = state.snow.depth_m;
    let lake = state.lake.as_mut().expect("checked above");
    let temperature = lake_temperature(
        LakeTemperatureInput {
            time_step_seconds: dt,
            latitude_radians: site.latitude_radians,
            wind_height_m: flux_input.wind_height_m,
            temperature_height_m: flux_input.temperature_height_m,
            humidity_height_m: flux_input.humidity_height_m,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            air_temperature_k: forcing.air_temperature_k,
            specific_humidity: forcing.specific_humidity,
            air_density_kg_m3: forcing.air_density_kg_m3,
            surface_pressure_pa: forcing.surface_pressure_pa,
            shortwave: input.energy.solar.forcing,
            absorbed_shortwave_w_m2: shortwave.ground_absorbed_w_m2,
            downward_longwave_w_m2: forcing.downward_longwave_w_m2,
            boundary_layer_height_m: forcing.boundary_layer_height_m,
            surface_layer_scheme: flux_input.surface_layer_scheme,
            lake_depth_m: site.depth_m,
            thermal_conductivity_scheme: ground.thermal_conductivity_scheme,
            soil_thermal_inputs: ground.soil_thermal_inputs,
            snow_layers,
            snow_layer_absorption_w_m2: snow_layer_absorption,
        },
        LakeTemperatureState {
            lake: &mut lake.column,
            saved_tke: &mut lake.saved_tke,
            ground_temperature_k: &mut lake.ground_temperature_k,
            snow_water_equivalent_kg_m2: &mut snow_water_equivalent,
            snow_depth_m: &mut snow_depth,
            column: &mut column,
        },
    )?;
    let mut thermal = temperature.fluxes;
    let phase_flag = temperature.phase_flag;
    // SNICAR（`MOD_Lake.F90:1335-1416`）：相变前后雪层冰量之差给出 `snofrz`。`laketem` 在相变
    // 之前不改雪层冰量，所以"相变前"就是进 `laketem` 时的量。
    if let Some(snicar) = state.snicar.as_mut() {
        crate::snow_refreezing_rate(
            snicar,
            snow_layers,
            &snow_ice_before,
            &column.ice_water_kg_m2[..snow_layers],
            &phase_flag,
            dt,
        );
    }
    state.snow.water_equivalent_kg_m2 = snow_water_equivalent;
    state.snow.depth_m = snow_depth;
    for (relative, index) in (state.snow.layer_count + 1..=0).enumerate() {
        let slot = crate::snow::snow_layer_slot(index);
        state.snow.temperature_k[slot] = column.temperature_k[relative];
        state.snow.liquid_water_kg_m2[slot] = column.liquid_water_kg_m2[relative];
        state.snow.ice_water_kg_m2[slot] = column.ice_water_kg_m2[relative];
    }
    state.soil_temperature_k = column.temperature_k[snow_layers..].to_vec();

    // ---- snowwater_lake ----
    let melted = phase_flag[..snow_layers]
        .iter()
        .map(|flag| *flag == 1)
        .collect::<Vec<_>>();
    let mut soil = LakeSnowWaterSoil {
        thickness_m: input.soil_water.layer_thickness_m.to_vec(),
        porosity: ground.soil_porosity.to_vec(),
        liquid_water_kg_m2: column.liquid_water_kg_m2[snow_layers..].to_vec(),
        ice_water_kg_m2: column.ice_water_kg_m2[snow_layers..].to_vec(),
    };
    let mut fluxes = LakeSnowWaterFluxes {
        sensible_heat_w_m2: thermal.fseng,
        ground_heat_w_m2: thermal.fgrnd,
        snow_melt_kg_m2_s: thermal.sm,
    };
    let lake = state.lake.as_mut().expect("checked above");
    let deposition = input
        .snicar
        .as_ref()
        .map(|step| step.aerosol_deposition_kg_m2_s);
    let snicar = state.snicar.as_deref_mut().zip(deposition.as_ref());
    lake_snow_water_with_snicar(
        LakeSnowWaterInput {
            use_dynamic_lake: site.dynamic,
            time_step_seconds: dt,
            irreducible_saturation: input.snow_water.irreducible_saturation,
            impermeable_porosity: input.snow_water.impermeable_porosity,
            rainfall_kg_m2_s: rainfall,
            evaporation_kg_m2_s: thermal.qseva,
            sublimation_kg_m2_s: thermal.qsubl,
            dew_kg_m2_s: thermal.qsdew,
            frost_kg_m2_s: thermal.qfros,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            melted: &melted,
        },
        &mut state.snow,
        &mut lake.column,
        &mut soil,
        &mut fluxes,
        snicar,
    )?;
    // `snowwater_lake` 只改 `fseng`/`fgrnd`/`sm`，`fsena` 仍是 `laketem` 写下的值。
    thermal.fseng = fluxes.sensible_heat_w_m2;
    thermal.fgrnd = fluxes.ground_heat_w_m2;
    thermal.sm = fluxes.snow_melt_kg_m2_s;
    state.soil_water.liquid_water_kg_m2 = soil.liquid_water_kg_m2;
    state.soil_water.ice_water_kg_m2 = soil.ice_water_kg_m2;

    // ---- 径流（`:1932-1942`）----
    let storage_change = ((((liquid_sum(state) + ice_sum(state))
        + state.snow.water_equivalent_kg_m2)
        - water_before)
        - snow_before)
        / dt;
    let (runoff, lake_deficit) = if site.dynamic {
        // 动态湖（`CoLMMAIN.F90:1942-1957`）：湖水就是 `wdsrf = sum(dz_lake)*1000`，超出湖深的部分
        // 这一步全部溢出，湖层按比例缩回湖深再重分层。GIMPLE 这里没有收缩。
        let lake = state.lake.as_mut().expect("checked above");
        let depth_mm = lake.column.thickness_m.iter().fold(0.0, |sum, dz| sum + dz) * 1.0e3;
        state.soil_water.surface_water_mm = depth_mm;
        let limit_mm = site.depth_m * 1.0e3;
        let runoff = if depth_mm > limit_mm {
            let runoff = (depth_mm - limit_mm) / dt;
            state.soil_water.surface_water_mm = limit_mm;
            let total = lake.column.thickness_m.iter().fold(0.0, |sum, dz| sum + dz);
            for dz in &mut lake.column.thickness_m {
                *dz = *dz * site.depth_m / total;
            }
            crate::adjust_lake_layers(&mut lake.column)?;
            runoff
        } else {
            0.0
        };
        (runoff, 0.0)
    } else {
        let evaporation = ((thermal.qseva + thermal.qsubl) - thermal.qsdew) - thermal.qfros;
        let excess = ((rainfall + snowfall) - evaporation) - storage_change;
        (excess.max(0.0), -excess.min(0.0))
    };

    // ---- 水量闭合（`:1962-1971`）----
    let total_water_after = (state.snow.water_equivalent_kg_m2 + soil_water_sum(state))
        + state.soil_water.aquifer_water_mm;
    let total_water_after = if site.dynamic {
        // `endwb = endwb + wdsrf`
        total_water_after + state.soil_water.surface_water_mm
    } else {
        // `:1966` `.FNMA (lake_deficit, deltim, endwb)`
        (-lake_deficit).mul_add(dt, total_water_after)
    };
    // `:1969-1971` `.FMA (rnof, dt, .FNMA (prc+prl-fevpa, dt, endwb-totwb))`
    let water_balance_error_mm = runoff.mul_add(
        dt,
        (-((forcing.convective_precipitation_kg_m2_s + forcing.large_scale_precipitation_kg_m2_s)
            - thermal.fevpa))
            .mul_add(dt, total_water_after - total_water_before),
    );

    // `snl > maxsnl` 时把空出来的雪槽清零（`:1983-1989`）。
    for index in -(crate::snow::MAX_SNOW_LAYERS as i32) + 1..=state.snow.layer_count {
        let slot = crate::snow::snow_layer_slot(index);
        state.snow.ice_water_kg_m2[slot] = 0.0;
        state.snow.liquid_water_kg_m2[slot] = 0.0;
        state.snow.temperature_k[slot] = 0.0;
        state.snow.node_depth_m[slot] = 0.0;
        state.snow.thickness_m[slot] = 0.0;
    }

    Ok(LakeStepOutput {
        precipitation,
        shortwave,
        rainfall_kg_m2_s: rainfall,
        snowfall_kg_m2_s: snowfall,
        thermal,
        phase_flag,
        surface_runoff_mm_s: runoff,
        total_runoff_mm_s: runoff,
        lake_deficit_mm_s: lake_deficit,
        water_balance_error_mm,
        initial_total_water_mm: total_water_before,
        final_total_water_mm: total_water_after,
    })
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

fn liquid_sum(state: &StandardLctSnowSoilState) -> f64 {
    state
        .soil_water
        .liquid_water_kg_m2
        .iter()
        .fold(0.0, |sum, value| sum + value)
}

fn ice_sum(state: &StandardLctSnowSoilState) -> f64 {
    state
        .soil_water
        .ice_water_kg_m2
        .iter()
        .fold(0.0, |sum, value| sum + value)
}

/// `fiold(snl+1:0) = wice/(wliq+wice)`（`:1817-1820`）。
fn remember_snow_ice_fraction(state: &mut StandardLctSnowSoilState) {
    let snow = &mut state.snow;
    for index in snow.layer_count + 1..=0 {
        let slot = crate::snow::snow_layer_slot(index);
        snow.previous_ice_fraction[slot] = snow.ice_water_kg_m2[slot]
            / (snow.liquid_water_kg_m2[slot] + snow.ice_water_kg_m2[slot]);
    }
}

#[cfg(test)]
#[path = "lake_step_tests.rs"]
mod lake_step_tests;
