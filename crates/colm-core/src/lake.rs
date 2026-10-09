//! Runtime lake-column adjustment from `MOD_Lake.F90`.

use crate::LibmPow;
use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::snow::{snow_interface_slot, snow_layer_slot, validate_runtime_snow_column};
use crate::{
    compact_snow_layers, snow_water, RuntimeSnowColumn, SnowToSoilTransfer, SnowWaterInput,
};

const LAKE_LAYERS: usize = 10;
const DEFAULT_THICKNESS_M: [f64; LAKE_LAYERS] =
    [0.1, 1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 7.0, 10.45, 10.45];
use crate::FREEZING_K;
const LIQUID_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const FUSION_HEAT_J_KG: f64 = 0.3336e6;
/// `cwat = cpliq*denh2o`
const LAKE_WATER_HEAT_CAPACITY: f64 = LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0;
/// `tkice_eff = tkice*denice/denh2o`
const EFFECTIVE_ICE_CONDUCTIVITY: f64 = 2.29 * 917.0 / 1000.0;

/// Mutable ten-layer lake state in top-to-bottom order.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeColumn {
    pub thickness_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    /// Frozen mass fraction of each lake layer.
    pub ice_fraction: Vec<f64>,
}

/// Inputs to `MOD_Lake:newsnow_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeNewSnowInput {
    pub use_dynamic_lake: bool,
    pub time_step_seconds: f64,
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub new_snow_bulk_density_kg_m3: f64,
}

/// Precipitation left after lake-surface phase change.
///
/// This is the mutated `pg_rain`/`pg_snow` pair from `newsnow_lake`; callers
/// pass it on to CoLM's later lake snow-water step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeNewSnowOutcome {
    pub rainfall_kg_m2_s: f64,
    pub snowfall_kg_m2_s: f64,
}

/// Mutable soil state below a lake used by `MOD_Lake:snowwater_lake`.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeSnowWaterSoil {
    pub thickness_m: Vec<f64>,
    pub porosity: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
}

/// In/out surface fluxes used by `MOD_Lake:snowwater_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSnowWaterFluxes {
    pub sensible_heat_w_m2: f64,
    pub ground_heat_w_m2: f64,
    pub snow_melt_kg_m2_s: f64,
}

/// Inputs to `MOD_Lake:snowwater_lake` after `newsnow_lake` has partitioned precipitation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSnowWaterInput<'a> {
    pub use_dynamic_lake: bool,
    pub time_step_seconds: f64,
    pub irreducible_saturation: f64,
    pub impermeable_porosity: f64,
    pub rainfall_kg_m2_s: f64,
    pub evaporation_kg_m2_s: f64,
    pub sublimation_kg_m2_s: f64,
    pub dew_kg_m2_s: f64,
    pub frost_kg_m2_s: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    /// CoLM `imelt` over the currently active snow layers, surface first.
    pub melted: &'a [bool],
}

/// Water drained from the snow bottom during the lake hydrology step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeSnowWaterOutcome {
    pub bottom_drainage_kg_m2_s: f64,
}

/// Inputs to `MOD_Lake:roughness_lake` for one lake surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeRoughnessInput {
    /// CoLM's signed snow-layer count (`0`, `-1`, …).
    pub snow_layer_count: i32,
    pub ground_temperature_k: f64,
    pub lake_surface_temperature_k: f64,
    pub surface_pressure_pa: f64,
    pub charnock_parameter: f64,
    pub friction_velocity_m_s: f64,
}

/// Lake momentum, sensible-heat, and latent-heat roughness lengths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeRoughness {
    pub momentum_m: f64,
    pub sensible_heat_m: f64,
    pub latent_heat_m: f64,
}

/// Inputs to `MOD_Lake:hConductivity_lake`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeConductivityInput<'a> {
    pub snow_layer_count: i32,
    pub ground_temperature_k: f64,
    /// Lake-node depths (m), top to bottom.
    pub node_depth_m: &'a [f64],
    pub latitude_radians: f64,
    pub friction_velocity_m_s: f64,
    pub momentum_roughness_m: f64,
    /// `lakedepth`, retained separately from the mutable layer geometry.
    pub lake_depth_m: f64,
    pub deep_lake_threshold_m: f64,
}

/// Effective thermal conductivity of every lake layer and top eddy term.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeConductivity {
    pub thermal_conductivity_w_m_k: Vec<f64>,
    pub top_eddy_conductivity_w_m_k: f64,
}

/// Ports `MOD_Lake:newsnow_lake`.
///
/// The snow state is the shared CoLM snow prefix used by land patches.  It
/// keeps the initializer and runtime on one snow-column representation while
/// preserving the lake-specific rain/snow and surface-ice energy exchange.
pub fn add_lake_new_snow(
    input: LakeNewSnowInput,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
) -> Result<LakeNewSnowOutcome> {
    validate(lake)?;
    validate_runtime_snow_column(snow)?;
    ensure!(
        lake.thickness_m[0] > 0.0,
        "lake surface layer must have positive thickness"
    );
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.rainfall_kg_m2_s.is_finite()
            && input.rainfall_kg_m2_s >= 0.0
            && input.snowfall_kg_m2_s.is_finite()
            && input.snowfall_kg_m2_s >= 0.0
            && input.precipitation_temperature_k.is_finite()
            && input.new_snow_bulk_density_kg_m3.is_finite()
            && input.new_snow_bulk_density_kg_m3 > 0.0,
        "lake new-snow inputs are invalid"
    );

    let mut rainfall = input.rainfall_kg_m2_s;
    let mut snowfall = input.snowfall_kg_m2_s;
    let dt = input.time_step_seconds;
    // `MOD_Lake.F90:109-111`：`snowdp + deltim*dz_snowf`、`scv + pg_snow*deltim`，都不融合。
    let snowfall_depth_rate = snowfall / input.new_snow_bulk_density_kg_m3;
    let snowfall_depth = dt * snowfall_depth_rate;
    let snowfall_mass = snowfall * dt;
    snow.depth_m += snowfall_depth;
    snow.water_equivalent_kg_m2 += snowfall_mass;
    snow.interface_depth_m[snow_interface_slot(0)] = 0.0;
    let mut new_node = false;

    if snow.layer_count == 0 && snow.depth_m < 0.01 {
        snow.age = 0.0;
        exchange_precipitation_with_lake(
            dt,
            input.precipitation_temperature_k,
            input.new_snow_bulk_density_kg_m3,
            snowfall_depth,
            &mut rainfall,
            &mut snowfall,
            snow,
            lake,
        );
        if snow.depth_m >= 0.01 {
            new_lake_snow_node(snow, lake.temperature_k[0]);
            new_node = true;
        }
        if input.use_dynamic_lake && snow.layer_count == 0 {
            // `:232-235`：`.FMA (dz, 1-fi, (deltim*pg_rain)*1e-3)`。
            let liquid_depth =
                lake.thickness_m[0].contract(1.0 - lake.ice_fraction[0], (dt * rainfall) * 1.0e-3);
            let ice_depth = lake.thickness_m[0] * lake.ice_fraction[0];
            lake.thickness_m[0] = liquid_depth + ice_depth;
            lake.ice_fraction[0] = ice_depth / lake.thickness_m[0];
            adjust_lake_layers(lake)?;
        }
    } else if snow.layer_count == 0 {
        new_lake_snow_node(snow, FREEZING_K.min(input.precipitation_temperature_k));
        new_node = true;
    }

    if snow.layer_count < 0 && !new_node {
        let top_layer = snow.layer_count + 1;
        let top = snow_layer_slot(top_layer);
        let ice = snow.ice_water_kg_m2[top];
        let liquid = snow.liquid_water_kg_m2[top];
        // `:259-262` 的 GIMPLE：热容 `.FMA (wice, cpice, wliq*cpliq)`；降水热
        // `deltim*.FMA (pg_rain, cpliq, pg_snow*cpice)`；分母把两项降水各自收进 FMA。
        let old_heat_capacity = ice.contract(
            ICE_HEAT_CAPACITY_J_KG_K,
            liquid * LIQUID_HEAT_CAPACITY_J_KG_K,
        );
        let precipitation_heat = dt
            * rainfall.contract(
                LIQUID_HEAT_CAPACITY_J_KG_K,
                snowfall * ICE_HEAT_CAPACITY_J_KG_K,
            );
        let numerator = old_heat_capacity.contract(
            snow.temperature_k[top],
            precipitation_heat * input.precipitation_temperature_k,
        );
        let denominator = snowfall_mass.contract(
            ICE_HEAT_CAPACITY_J_KG_K,
            (dt * rainfall).contract(LIQUID_HEAT_CAPACITY_J_KG_K, old_heat_capacity),
        );
        ensure!(
            denominator > 0.0,
            "lake snow top layer has no heat capacity"
        );
        snow.temperature_k[top] = (numerator / denominator).min(FREEZING_K);
        snow.ice_water_kg_m2[top] = snowfall_mass + ice;
        snow.thickness_m[top] += snowfall_depth;
        // `:267` `.FNMA (dz, 0.5, zi(lb))`
        snow.node_depth_m[top] = (-snow.thickness_m[top])
            .contract(0.5, snow.interface_depth_m[snow_interface_slot(top_layer)]);
        snow.interface_depth_m[snow_interface_slot(top_layer - 1)] =
            snow.interface_depth_m[snow_interface_slot(top_layer)] - snow.thickness_m[top];
    }

    Ok(LakeNewSnowOutcome {
        rainfall_kg_m2_s: rainfall,
        snowfall_kg_m2_s: snowfall,
    })
}

fn new_lake_snow_node(snow: &mut RuntimeSnowColumn, temperature_k: f64) {
    snow.layer_count = -1;
    let top = snow_layer_slot(0);
    snow.thickness_m[top] = snow.depth_m;
    snow.node_depth_m[top] = -(snow.thickness_m[top] * 0.5);
    snow.interface_depth_m[snow_interface_slot(-1)] = -snow.thickness_m[top];
    snow.age = 0.0;
    snow.temperature_k[top] = temperature_k;
    snow.ice_water_kg_m2[top] = snow.water_equivalent_kg_m2;
    snow.liquid_water_kg_m2[top] = 0.0;
    snow.previous_ice_fraction[top] = 1.0;
}

/// `newsnow_lake` 无雪层且新雪不足 1 cm 时的降水—湖面能量交换（`MOD_Lake.F90:118-228`）。
///
/// 每个式子的收缩形状都对着 `-fdump-tree-optimized-lineno` 抄：`a`..`h` 八个能量项
/// 全是左结合的纯乘积（`dt*(pg_rain*cpliq)` 这类重排不改值），`a+b` 被收成
/// `.FMA (deltim*pg_rain, hfus, a)`；各分支的温度分子里，最后一个乘积收进 FMA。
#[allow(clippy::too_many_arguments)]
fn exchange_precipitation_with_lake(
    dt: f64,
    precipitation_temperature_k: f64,
    new_snow_bulk_density_kg_m3: f64,
    snowfall_depth: f64,
    rainfall: &mut f64,
    snowfall: &mut f64,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
) {
    let tp = precipitation_temperature_k;
    let dz = lake.thickness_m[0];
    let fi = lake.ice_fraction[0];
    let tl = lake.temperature_k[0];
    let pr = *rainfall;
    let ps = *snowfall;
    let rain_heat = dt * (pr * LIQUID_HEAT_CAPACITY_J_KG_K);
    let snow_heat = dt * (ps * ICE_HEAT_CAPACITY_J_KG_K);
    let rain_mass = dt * pr;
    let snow_mass = ps * dt;
    let ice_heat_column = dz * (ICE_HEAT_CAPACITY_J_KG_K * 1000.0);
    let liquid_heat_column = dz * (LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0);
    let water_column = dz * 1000.0;
    let a = rain_heat * (tp - FREEZING_K);
    let c = (ice_heat_column * fi) * (FREEZING_K - tl);
    let d = (fi * water_column) * FUSION_HEAT_J_KG;
    let e = snow_heat * (FREEZING_K - tp);
    let f = snow_mass * FUSION_HEAT_J_KG;
    let g = (liquid_heat_column * (1.0 - fi)) * (tl - FREEZING_K);
    let h = (water_column * (1.0 - fi)) * FUSION_HEAT_J_KG;
    // `:130` 与 `:133` 共用 `.FMA (deltim*pg_rain, hfus, a)`。
    let a_plus_b = rain_mass.contract(FUSION_HEAT_J_KG, a);

    if fi > 0.999 {
        if a_plus_b <= c {
            let tw = FREEZING_K.min(tp);
            let precipitation_heat = dt * ((pr + ps) * ICE_HEAT_CAPACITY_J_KG_K);
            // `:133` `.FMA (fi, dz*cice*1000*t, .FMA (dt*(..)*cpice, tw, a+b))`
            lake.temperature_k[0] = fi.contract(
                ice_heat_column * tl,
                precipitation_heat.contract(tw, a_plus_b),
            ) / (ice_heat_column * fi + precipitation_heat);
            snow.water_equivalent_kg_m2 += rain_mass;
            snow.depth_m += rain_mass / new_snow_bulk_density_kg_m3;
            *snowfall = ps + pr;
            *rainfall = 0.0;
        } else if a <= c {
            lake.temperature_k[0] = FREEZING_K;
            let heat = c - a;
            snow.water_equivalent_kg_m2 += heat / FUSION_HEAT_J_KG;
            snow.depth_m += heat / (new_snow_bulk_density_kg_m3 * FUSION_HEAT_J_KG);
            let frozen_rate = heat / (dt * FUSION_HEAT_J_KG);
            *snowfall = ps + pr.min(frozen_rate);
            *rainfall = (pr - frozen_rate).max(0.0);
        } else if a <= c + d {
            lake.temperature_k[0] = FREEZING_K;
            let liquid = (a - c) / FUSION_HEAT_J_KG;
            let ice = water_column - liquid;
            lake.ice_fraction[0] = ice / (liquid + ice);
        } else {
            // `:154` `(.FMA (dt*pr*cpliq, tp, dz*cwat*tfrz) - c - d) / (…+…)`
            lake.temperature_k[0] =
                (rain_heat.contract(tp, liquid_heat_column * FREEZING_K) - c - d)
                    / (rain_heat + liquid_heat_column);
            lake.ice_fraction[0] = 0.0;
        }
    } else if fi >= 0.001 {
        if pr > 0.0 && ps > 0.0 {
            lake.temperature_k[0] = FREEZING_K;
        } else if pr > 0.0 {
            if a >= d {
                lake.temperature_k[0] = (rain_heat.contract(tp, liquid_heat_column * FREEZING_K)
                    - d)
                    / (rain_heat + liquid_heat_column);
                lake.ice_fraction[0] = 0.0;
            } else {
                lake.temperature_k[0] = FREEZING_K;
                let melted = a / FUSION_HEAT_J_KG;
                let ice = fi * water_column - melted;
                let liquid = water_column * (1.0 - fi) + melted;
                lake.ice_fraction[0] = ice / (ice + liquid);
            }
        } else if ps > 0.0 {
            if e >= h {
                // `:189` `.FMA (dt*ps*cpice, tp, .FMA (dz*cice*1000, tfrz, h)) / (…+…)`
                lake.temperature_k[0] = snow_heat
                    .contract(tp, ice_heat_column.contract(FREEZING_K, h))
                    / (ice_heat_column + snow_heat);
                lake.ice_fraction[0] = 1.0;
            } else {
                lake.temperature_k[0] = FREEZING_K;
                let frozen = e / FUSION_HEAT_J_KG;
                let ice = fi * water_column + frozen;
                let liquid = water_column * (1.0 - fi) - frozen;
                lake.ice_fraction[0] = ice / (ice + liquid);
            }
        }
    } else if e + f <= g {
        let tw = FREEZING_K.max(tp);
        let precipitation_heat = dt * ((ps + pr) * LIQUID_HEAT_CAPACITY_J_KG_K);
        let liquid_heat = liquid_heat_column * (1.0 - fi);
        // `:204` `(.FMA (1-fi, t*dz*cwat, dt*(..)*cpliq*tw) - e - f) / (…+…)`
        lake.temperature_k[0] =
            ((1.0 - fi).contract(tl * liquid_heat_column, precipitation_heat * tw) - e - f)
                / (liquid_heat + precipitation_heat);
        snow.water_equivalent_kg_m2 -= snow_mass;
        snow.depth_m -= snowfall_depth;
        *rainfall = pr + ps;
        *snowfall = 0.0;
    } else if e <= g {
        lake.temperature_k[0] = FREEZING_K;
        let heat = g - e;
        snow.water_equivalent_kg_m2 -= heat / FUSION_HEAT_J_KG;
        snow.depth_m -= heat / (new_snow_bulk_density_kg_m3 * FUSION_HEAT_J_KG);
        let melted_rate = heat / (dt * FUSION_HEAT_J_KG);
        *rainfall = pr + ps.min(melted_rate);
        *snowfall = (ps - melted_rate).max(0.0);
    } else if e <= g + h {
        lake.temperature_k[0] = FREEZING_K;
        let ice = (e - g) / FUSION_HEAT_J_KG;
        let liquid = water_column - ice;
        lake.ice_fraction[0] = ice / (ice + liquid);
    } else {
        lake.temperature_k[0] = snow_heat.contract(tp, ice_heat_column.contract(FREEZING_K, g + h))
            / (ice_heat_column + snow_heat);
        lake.ice_fraction[0] = 1.0;
    }
}

/// Ports `MOD_Lake:snowwater_lake` without SNICAR aerosols.
///
/// It reuses the common snow percolation, compaction, combining, and division
/// kernels, then applies only the lake-specific melting and saturated-bed rules.
pub fn lake_snow_water(
    input: LakeSnowWaterInput<'_>,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
    soil: &mut LakeSnowWaterSoil,
    fluxes: &mut LakeSnowWaterFluxes,
) -> Result<LakeSnowWaterOutcome> {
    lake_snow_water_with_snicar(input, snow, lake, soil, fluxes, None)
}

/// [`lake_snow_water`] 带 `DEF_USE_SNICAR`：`snowwater_SNICAR` 随融水搬气溶胶并把本步沉降
/// 加到最上层，合并/分裂雪层时气溶胶跟着走（`MOD_Lake.F90:1697-1740`）。
/// 之后单层全液雪的移除与落进未冻湖的融雪不碰 `mss_*` —— 空出来的槽由步末
/// `AerosolMasses` 清零。
pub fn lake_snow_water_with_snicar(
    input: LakeSnowWaterInput<'_>,
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
    soil: &mut LakeSnowWaterSoil,
    fluxes: &mut LakeSnowWaterFluxes,
    mut snicar: Option<(
        &mut crate::SnicarColumnState,
        &[f64; crate::AEROSOL_DEPOSITION_FIELDS],
    )>,
) -> Result<LakeSnowWaterOutcome> {
    validate(lake)?;
    validate_lake_soil(soil)?;
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.irreducible_saturation.is_finite()
            && (0.0..=1.0).contains(&input.irreducible_saturation)
            && input.impermeable_porosity.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.rainfall_kg_m2_s.is_finite()
            && input.evaporation_kg_m2_s.is_finite()
            && input.sublimation_kg_m2_s.is_finite()
            && input.dew_kg_m2_s.is_finite()
            && input.frost_kg_m2_s.is_finite()
            && input.eastward_wind_m_s.is_finite()
            && input.northward_wind_m_s.is_finite()
            && fluxes.sensible_heat_w_m2.is_finite()
            && fluxes.ground_heat_w_m2.is_finite()
            && fluxes.snow_melt_kg_m2_s.is_finite(),
        "lake snow-water inputs are invalid"
    );
    let had_snow = snow.layer_count < 0;
    ensure!(
        input.melted.len()
            == if had_snow {
                snow.layer_count.unsigned_abs() as usize
            } else {
                0
            },
        "lake snow-water melt flags do not match the snow column"
    );
    let mut bottom_drainage = 0.0;

    if had_snow {
        let outcome = snow_water(
            SnowWaterInput {
                time_step_seconds: input.time_step_seconds,
                irreducible_saturation: input.irreducible_saturation,
                impermeable_porosity: input.impermeable_porosity,
                rainfall_kg_m2_s: input.rainfall_kg_m2_s,
                evaporation_kg_m2_s: input.evaporation_kg_m2_s,
                dew_kg_m2_s: input.dew_kg_m2_s,
                sublimation_kg_m2_s: input.sublimation_kg_m2_s,
                frost_kg_m2_s: input.frost_kg_m2_s,
            },
            snow,
        )?;
        bottom_drainage = outcome.bottom_drainage_kg_m2_s;
        if let Some((state, deposition)) = snicar.as_mut() {
            crate::snicar_snow_water_aerosols(
                state,
                snow.layer_count.unsigned_abs() as usize,
                &snow.liquid_water_kg_m2,
                &snow.ice_water_kg_m2,
                &outcome.layer_drainage_kg_m2,
                deposition,
                input.time_step_seconds,
            )?;
        }
        compact_snow_layers(
            snow,
            input.time_step_seconds,
            input.eastward_wind_m_s,
            input.northward_wind_m_s,
            input.melted,
        )?;
        let mut lake_surface = SnowToSoilTransfer {
            liquid_water_kg_m2: soil.liquid_water_kg_m2[0],
            ice_water_kg_m2: soil.ice_water_kg_m2[0],
        };
        let mut aerosols = snicar
            .as_mut()
            .map(|(state, _)| &mut state.aerosol_mass_kg_m2);
        crate::combine_snow_layers_with_aerosols(snow, &mut lake_surface, aerosols.as_deref_mut())?;
        soil.liquid_water_kg_m2[0] = lake_surface.liquid_water_kg_m2;
        soil.ice_water_kg_m2[0] = lake_surface.ice_water_kg_m2;
        if snow.layer_count < 0 {
            crate::divide_snow_layers_with_aerosols(snow, aerosols)?;
        }
        remove_all_liquid_snow(snow, fluxes, input.time_step_seconds, &mut bottom_drainage);
    }

    melt_snow_into_unfrozen_lake(
        snow,
        lake,
        fluxes,
        input.time_step_seconds,
        &mut bottom_drainage,
    );
    let soil_water_change = saturate_lake_soil(soil);
    if input.use_dynamic_lake {
        adjust_dynamic_lake_water(
            had_snow,
            input,
            bottom_drainage,
            soil_water_change,
            lake,
            fluxes,
        )?;
    }
    Ok(LakeSnowWaterOutcome {
        bottom_drainage_kg_m2_s: bottom_drainage,
    })
}

fn remove_all_liquid_snow(
    snow: &mut RuntimeSnowColumn,
    fluxes: &mut LakeSnowWaterFluxes,
    time_step_seconds: f64,
    bottom_drainage: &mut f64,
) {
    if snow.layer_count != -1 || snow.ice_water_kg_m2[snow_layer_slot(0)] != 0.0 {
        return;
    }
    let top = snow_layer_slot(0);
    let heat = LIQUID_HEAT_CAPACITY_J_KG_K
        * snow.liquid_water_kg_m2[top]
        * (snow.temperature_k[top] - FREEZING_K);
    fluxes.sensible_heat_w_m2 += heat / time_step_seconds;
    fluxes.ground_heat_w_m2 -= heat / time_step_seconds;
    *bottom_drainage += snow.liquid_water_kg_m2[top] / time_step_seconds;
    snow.layer_count = 0;
    snow.water_equivalent_kg_m2 = 0.0;
    snow.depth_m = 0.0;
    snow.liquid_water_kg_m2[top] = 0.0;
}

fn melt_snow_into_unfrozen_lake(
    snow: &mut RuntimeSnowColumn,
    lake: &mut LakeColumn,
    fluxes: &mut LakeSnowWaterFluxes,
    time_step_seconds: f64,
    bottom_drainage: &mut f64,
) {
    if snow.layer_count >= 0 || lake.temperature_k[0] <= FREEZING_K || lake.ice_fraction[0] >= 0.001
    {
        return;
    }
    let mut ice = 0.0;
    let mut liquid = 0.0;
    let mut heat = 0.0;
    for layer in snow.layer_count + 1..=0 {
        let slot = snow_layer_slot(layer);
        let layer_ice = snow.ice_water_kg_m2[slot];
        let layer_liquid = snow.liquid_water_kg_m2[slot];
        ice += layer_ice;
        liquid += layer_liquid;
        // `MOD_Lake.F90:1789` 两个乘积各收一条 FMA：
        // `.FMA (tfrz-t, wliq*cpliq, .FMA (wice*cpice, tfrz-t, heatsum))`。
        let deficit = FREEZING_K - snow.temperature_k[slot];
        heat = (layer_ice * ICE_HEAT_CAPACITY_J_KG_K).contract(deficit, heat);
        heat = deficit.contract(layer_liquid * LIQUID_HEAT_CAPACITY_J_KG_K, heat);
    }
    let fusion = ice * FUSION_HEAT_J_KG;
    let a_plus_b = fusion + heat;
    let lake_t = lake.temperature_k[0];
    let water_column = lake.thickness_m[0] * 1000.0;
    // `:1802` `((t-tfrz)*cpliq*1000)*dz`，`:1803` `(dz*1000)*hfus`。
    let c = (lake_t - FREEZING_K) * LIQUID_HEAT_CAPACITY_J_KG_K * 1000.0 * lake.thickness_m[0];
    let d = water_column * FUSION_HEAT_J_KG;
    if c >= a_plus_b {
        // `:1807` `(.FMS (.FMA (dz*1000, t, (liq+ice)*tfrz), cpliq, heatsum) - b) /
        // (((dz*1000 + ice) + liq)*cpliq)`
        lake.temperature_k[0] = (water_column
            .contract(lake_t, (liquid + ice) * FREEZING_K)
            .contract(LIQUID_HEAT_CAPACITY_J_KG_K, -heat)
            - fusion)
            / (((water_column + ice) + liquid) * LIQUID_HEAT_CAPACITY_J_KG_K);
    } else if c + d >= a_plus_b {
        lake.temperature_k[0] = FREEZING_K;
        lake.ice_fraction[0] = (a_plus_b - c) / d;
    } else {
        return;
    }
    let melt_rate = snow.water_equivalent_kg_m2 / time_step_seconds;
    fluxes.snow_melt_kg_m2_s += melt_rate;
    *bottom_drainage += melt_rate;
    snow.layer_count = 0;
    snow.water_equivalent_kg_m2 = 0.0;
    snow.depth_m = 0.0;
}

fn validate_lake_soil(soil: &LakeSnowWaterSoil) -> Result<()> {
    ensure!(
        !soil.thickness_m.is_empty()
            && soil.thickness_m.len() == soil.porosity.len()
            && soil.thickness_m.len() == soil.liquid_water_kg_m2.len()
            && soil.thickness_m.len() == soil.ice_water_kg_m2.len()
            && soil
                .thickness_m
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && soil
                .porosity
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            && soil
                .liquid_water_kg_m2
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && soil
                .ice_water_kg_m2
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "lake soil state is invalid"
    );
    Ok(())
}

fn saturate_lake_soil(soil: &mut LakeSnowWaterSoil) -> f64 {
    let mut water_change = 0.0;
    for layer in 0..soil.thickness_m.len() {
        let thickness = soil.thickness_m[layer];
        let porosity = soil.porosity[layer];
        water_change =
            (soil.liquid_water_kg_m2[layer] + water_change) + soil.ice_water_kg_m2[layer];
        let saturation = soil.liquid_water_kg_m2[layer] / (thickness * 1000.0)
            + soil.ice_water_kg_m2[layer] / (thickness * 917.0);
        let pore_volume = thickness * porosity;
        if saturation < porosity {
            soil.liquid_water_kg_m2[layer] =
                ((pore_volume - soil.ice_water_kg_m2[layer] / 917.0) * 1000.0).max(0.0);
        } else {
            // `MOD_Lake.F90:1851` `.FNMA (dz, (a-porsl)*1000, wliq)`
            soil.liquid_water_kg_m2[layer] = (-thickness)
                .contract(
                    (saturation - porosity) * 1000.0,
                    soil.liquid_water_kg_m2[layer],
                )
                .max(0.0);
        }
        soil.ice_water_kg_m2[layer] =
            ((pore_volume - soil.liquid_water_kg_m2[layer] / 1000.0) * 917.0).max(0.0);
        let limit = thickness * (porosity * 1000.0);
        if soil.liquid_water_kg_m2[layer] > limit {
            soil.liquid_water_kg_m2[layer] = limit;
            soil.ice_water_kg_m2[layer] = 0.0;
        }
        water_change =
            (water_change - soil.liquid_water_kg_m2[layer]) - soil.ice_water_kg_m2[layer];
    }
    water_change
}

fn adjust_dynamic_lake_water(
    had_snow: bool,
    input: LakeSnowWaterInput<'_>,
    bottom_drainage: f64,
    soil_water_change: f64,
    lake: &mut LakeColumn,
    fluxes: &LakeSnowWaterFluxes,
) -> Result<()> {
    // GIMPLE（`MOD_Lake.F90:1865-1873`）：通量项先乘 `deltim`，再收成 `.FMA (flux*deltim, 1e-3, dz*(1-fi))`
    // 与 `.FMA (flux*deltim, 1e-3, dz*fi)`。这些项比 `dz` 小几个量级，不融合时绝大多数步舍入结果
    // 碰巧相同，只在个别步差 1 ULP（dc5：AT-Cold 强迫 3 月 19 日）。
    let dt = input.time_step_seconds;
    let liquid_before = lake.thickness_m[0] * (1.0 - lake.ice_fraction[0]);
    let ice_before = lake.thickness_m[0] * lake.ice_fraction[0];
    let (mut liquid_depth, mut ice_depth) = if had_snow {
        (
            (bottom_drainage * dt).contract(1.0e-3, liquid_before),
            ice_before,
        )
    } else {
        (
            (((fluxes.snow_melt_kg_m2_s + input.dew_kg_m2_s) - input.evaporation_kg_m2_s) * dt)
                .contract(1.0e-3, liquid_before),
            ((input.frost_kg_m2_s - input.sublimation_kg_m2_s) * dt).contract(1.0e-3, ice_before),
        )
    };
    if liquid_depth < 0.0 {
        ice_depth += liquid_depth;
        liquid_depth = 0.0;
    }
    if ice_depth < 0.0 {
        liquid_depth += ice_depth;
        ice_depth = 0.0;
    }
    lake.thickness_m[0] = (liquid_depth + ice_depth).max(1.0e-6);
    lake.ice_fraction[0] = (ice_depth / lake.thickness_m[0]).clamp(0.0, 1.0);
    let bottom = lake.thickness_m.len() - 1;
    // `dz_lake(nl_lake) + dw_soil/1.e3`：是除法，不能换成乘 1e-3。
    lake.thickness_m[bottom] += soil_water_change / 1.0e3;
    let mut layer = bottom;
    while lake.thickness_m[layer] < 0.0 {
        if layer > 0 {
            lake.thickness_m[layer - 1] += lake.thickness_m[layer];
        }
        lake.thickness_m[layer] = 0.0;
        if layer == 0 {
            break;
        }
        layer -= 1;
    }
    adjust_lake_layers(lake)
}

/// Ports `MOD_Lake:roughness_lake`.
pub fn lake_roughness(input: LakeRoughnessInput) -> Result<LakeRoughness> {
    ensure!(
        input.snow_layer_count <= 0
            && input.ground_temperature_k.is_finite()
            && input.lake_surface_temperature_k.is_finite()
            && input.surface_pressure_pa.is_finite()
            && input.surface_pressure_pa > 0.0
            && input.charnock_parameter.is_finite()
            && input.charnock_parameter >= 0.0
            && input.friction_velocity_m_s.is_finite()
            && input.friction_velocity_m_s >= 0.0,
        "lake roughness inputs are invalid"
    );
    if input.ground_temperature_k > FREEZING_K
        && input.lake_surface_temperature_k > FREEZING_K
        && input.snow_layer_count == 0
    {
        // `MOD_Lake.F90:1948-1955` 的 GIMPLE：`cur*ustar*ustar` 左结合；`sqre0` 走 `pow(·, 0.5)`；
        // `4*sqre0 - 3.2` 收成 `.FMA (sqre0, 4, -3.2)`。
        let ustar = input.friction_velocity_m_s;
        let viscosity = (input.ground_temperature_k / 293.15).lpow(1.5) * 1.51e-5 * 1.013e5
            / input.surface_pressure_pa;
        let momentum_m = ((viscosity * 0.1) / ustar.max(1.0e-4))
            .max(((input.charnock_parameter * ustar) * ustar) / 9.80616)
            .max(1.0e-5);
        let roughness_reynolds_sqrt = ((momentum_m * ustar) / viscosity).max(0.1).lpow(0.5);
        return Ok(LakeRoughness {
            momentum_m,
            sensible_heat_m: (momentum_m
                * (-(roughness_reynolds_sqrt.contract(4.0, -3.2) * (0.4 / 0.713))).exp())
            .max(1.0e-5),
            latent_heat_m: (momentum_m
                * (-(roughness_reynolds_sqrt.contract(4.0, -4.2) * (0.4 / 0.66))).exp())
            .max(1.0e-5),
        });
    }
    let momentum_m = if input.snow_layer_count == 0 {
        0.001
    } else {
        0.0024
    };
    let heat =
        momentum_m / (0.13 * (input.friction_velocity_m_s * momentum_m / 1.5e-5).lpow(0.45)).exp();
    Ok(LakeRoughness {
        momentum_m,
        sensible_heat_m: heat,
        latent_heat_m: heat,
    })
}

/// Ports `MOD_Lake:hConductivity_lake`, including density-dependent mixing.
pub fn lake_thermal_conductivity(
    input: LakeConductivityInput<'_>,
    column: &LakeColumn,
) -> Result<LakeConductivity> {
    validate(column)?;
    ensure!(
        input.snow_layer_count <= 0
            && input.ground_temperature_k.is_finite()
            && input.node_depth_m.len() == LAKE_LAYERS
            && input.node_depth_m.iter().all(|value| value.is_finite())
            && input
                .node_depth_m
                .windows(2)
                .all(|depths| depths[1] > depths[0])
            && input.latitude_radians.is_finite()
            && input.latitude_radians.abs() <= std::f64::consts::FRAC_PI_2
            && input.friction_velocity_m_s.is_finite()
            && input.friction_velocity_m_s >= 0.0
            && input.momentum_roughness_m.is_finite()
            && input.momentum_roughness_m > 0.0
            && input.lake_depth_m.is_finite()
            && input.lake_depth_m >= 0.0
            && input.deep_lake_threshold_m.is_finite()
            && input.deep_lake_threshold_m >= 0.0,
        "lake conductivity inputs are invalid"
    );
    let water_density = column
        .temperature_k
        .iter()
        .zip(&column.ice_fraction)
        .map(|(&temperature, &ice_fraction)| lake_water_density(temperature, ice_fraction))
        .collect::<Vec<_>>();
    let z = input.node_depth_m;
    let molecular_diffusivity = 0.6 / LAKE_WATER_HEAT_CAPACITY;
    // `MOD_Lake.F90:2031-2033`：`u2m = max(ustar/vonkar*log(2/z0mg), 0.1)`，
    // `ks = sqrt(|sin lat|)*6.6*u2m**(-1.84)`，全部不融合。
    let wind_2m =
        (input.friction_velocity_m_s / 0.4 * (2.0 / input.momentum_roughness_m).ln()).max(0.1);
    let surface_water_velocity = wind_2m * 1.2e-3;
    let decay = input.latitude_radians.sin().abs().sqrt() * 6.6 * wind_2m.lpow(-1.84);
    let deep_lake = input.lake_depth_m >= input.deep_lake_threshold_m;
    let open_water = input.ground_temperature_k > FREEZING_K
        && column.temperature_k[0] > FREEZING_K
        && input.snow_layer_count == 0;
    let mut eddy_diffusivity = [0.0; LAKE_LAYERS];
    let mut thermal_conductivity_w_m_k = [0.0; LAKE_LAYERS];

    for layer in 0..LAKE_LAYERS - 1 {
        let density_gradient =
            (water_density[layer + 1] - water_density[layer]) / (z[layer + 1] - z[layer]);
        let buoyancy_frequency = (density_gradient * (9.80616 / water_density[layer])).max(7.5e-5);
        let scaled_depth = z[layer] * 0.4;
        let numerator = (scaled_depth * scaled_depth) * (buoyancy_frequency * 40.0);
        let decay_exponent = (-(z[layer] * (decay * 2.0))).max(-40.0);
        let denominator =
            ((surface_water_velocity * surface_water_velocity) * decay_exponent.exp()).max(1.0e-10);
        let richardson = ((numerator / denominator + 1.0).max(0.0).sqrt() - 1.0) / 20.0;
        // `:2050` `.FMA (pow(n2, -0.43), 1.039e-8, km + ke)`
        let fang_stefan = buoyancy_frequency.lpow(-0.43);
        let mut diffusivity = if open_water {
            let exponent = (-(z[layer] * decay)).max(-40.0);
            // `:2047` `ke = ((z*(ws*0.4))*exp)/.FMA (ri, 37*ri, 1)`
            let eddy = (z[layer] * (surface_water_velocity * 0.4)) * exponent.exp()
                / richardson.contract(richardson * 37.0, 1.0);
            fang_stefan.contract(1.039e-8, eddy + molecular_diffusivity)
        } else {
            fang_stefan.contract(1.039e-8, molecular_diffusivity)
        };
        if deep_lake {
            diffusivity *= 5.0;
        }
        eddy_diffusivity[layer] = diffusivity;
        thermal_conductivity_w_m_k[layer] = if open_water {
            diffusivity * LAKE_WATER_HEAT_CAPACITY
        } else {
            frozen_thermal_conductivity(diffusivity, column.ice_fraction[layer])
        };
    }
    eddy_diffusivity[LAKE_LAYERS - 1] = eddy_diffusivity[LAKE_LAYERS - 2];
    thermal_conductivity_w_m_k[LAKE_LAYERS - 1] = if open_water {
        thermal_conductivity_w_m_k[LAKE_LAYERS - 2]
    } else {
        frozen_thermal_conductivity(
            eddy_diffusivity[LAKE_LAYERS - 1],
            column.ice_fraction[LAKE_LAYERS - 1],
        )
    };
    Ok(LakeConductivity {
        thermal_conductivity_w_m_k: thermal_conductivity_w_m_k.to_vec(),
        top_eddy_conductivity_w_m_k: eddy_diffusivity[0] * LAKE_WATER_HEAT_CAPACITY,
    })
}

/// `rhow = (1-f)*1000*(1 - 1.9549e-5*|t-277|**1.68) + f*917`：
/// GIMPLE（`MOD_Lake.F90:952/1451/1510`）是 `.FMA ((1-f)*1000, .FNMA (pow, 1.9549e-5, 1), f*917)`。
pub(crate) fn lake_water_density(temperature_k: f64, ice_fraction: f64) -> f64 {
    ((1.0 - ice_fraction) * 1000.0).contract(
        (-(temperature_k - 277.0).abs().lpow(1.68)).contract(1.9549e-5, 1.0),
        ice_fraction * 917.0,
    )
}

/// 冰盖下的等效导热率：`kme*cwat*tkice_eff / ((1-f)*tkice_eff + kme*cwat*f)`，
/// 分母收成 `.FMA (1-f, tkice_eff, (kme*cwat)*f)`（`MOD_Lake.F90:2065/2076`）。
fn frozen_thermal_conductivity(diffusivity: f64, ice_fraction: f64) -> f64 {
    let conductivity = diffusivity * LAKE_WATER_HEAT_CAPACITY;
    (conductivity * EFFECTIVE_ICE_CONDUCTIVITY)
        / (1.0 - ice_fraction).contract(EFFECTIVE_ICE_CONDUCTIVITY, conductivity * ice_fraction)
}

/// Remaps a lake column to CoLM's depth-scaled standard ten-layer geometry.
///
/// The overlap remap and mixed liquid/ice energy reconciliation preserve
/// `MOD_Lake:adjust_lake_layer`. A zero-depth column is intentionally untouched.
pub fn adjust_lake_layers(column: &mut LakeColumn) -> Result<()> {
    validate(column)?;
    let total_depth_m: f64 = column.thickness_m.iter().sum();
    ensure!(total_depth_m.is_finite(), "lake depth must be finite");
    if total_depth_m == 0.0 {
        return Ok(());
    }

    let target_thickness_m = target_thickness(total_depth_m);
    let mut target_temperature_k = vec![0.0; LAKE_LAYERS];
    let mut target_ice_fraction = vec![0.0; LAKE_LAYERS];
    let mut source_layer = 0;
    let mut source_remaining_m = column.thickness_m[source_layer];

    for target_layer in 0..LAKE_LAYERS {
        let mut target_remaining_m = target_thickness_m[target_layer];
        let mut ice_temperature_sum = 0.0;
        let mut liquid_temperature_sum = 0.0;
        let mut ice_mass = 0.0;
        let mut liquid_mass = 0.0;
        while target_remaining_m > 0.0 {
            if source_remaining_m == 0.0 {
                if source_layer + 1 == LAKE_LAYERS {
                    // MOD_Lake exits its overlap loop at the final source layer.
                    // Preserve that behavior when binary roundoff leaves only a
                    // sub-ulp target remainder after the conserved remap.
                    break;
                }
                source_layer += 1;
                source_remaining_m = column.thickness_m[source_layer];
                continue;
            }
            let overlap_m = target_remaining_m.min(source_remaining_m);
            let source_ice = column.ice_fraction[source_layer];
            let source_temperature = column.temperature_k[source_layer];
            // GIMPLE：`ticesum = .FMA (olp*fi, t, ticesum)`、`tliqsum = .FMA (t, (1-fi)*olp, tliqsum)`，
            // 两个质量和各自是普通加法（乘积被温度和复用，不收缩）。
            let ice_part = overlap_m * source_ice;
            let liquid_part = (1.0 - source_ice) * overlap_m;
            ice_temperature_sum = ice_part.contract(source_temperature, ice_temperature_sum);
            ice_mass += ice_part;
            liquid_temperature_sum =
                source_temperature.contract(liquid_part, liquid_temperature_sum);
            liquid_mass += liquid_part;
            target_remaining_m -= overlap_m;
            source_remaining_m -= overlap_m;
        }

        let (temperature_k, ice_mass) = reconcile_phase(
            target_thickness_m[target_layer],
            ice_temperature_sum,
            liquid_temperature_sum,
            ice_mass,
            liquid_mass,
        );
        target_temperature_k[target_layer] = temperature_k;
        target_ice_fraction[target_layer] = ice_mass / target_thickness_m[target_layer];
    }
    column.thickness_m = target_thickness_m;
    column.temperature_k = target_temperature_k;
    column.ice_fraction = target_ice_fraction;
    Ok(())
}

fn validate(column: &LakeColumn) -> Result<()> {
    ensure!(
        column.thickness_m.len() == LAKE_LAYERS
            && column.temperature_k.len() == LAKE_LAYERS
            && column.ice_fraction.len() == LAKE_LAYERS,
        "CoLM lake adjustment requires ten matching layers"
    );
    ensure!(
        column
            .thickness_m
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && column.temperature_k.iter().all(|value| value.is_finite())
            // 冰比只要求有限：动态湖的质量换算会让它差 1 ULP 地越过 1（实测 1.0000000000000002），
            // 上游照样带着算。
            && column.ice_fraction.iter().all(|value| value.is_finite()),
        "lake column is physically invalid: dz {:?}, t {:?}, icefrac {:?}",
        column.thickness_m,
        column.temperature_k,
        column.ice_fraction
    );
    Ok(())
}

fn target_thickness(total_depth_m: f64) -> Vec<f64> {
    if total_depth_m <= 1.0 {
        return vec![total_depth_m / LAKE_LAYERS as f64; LAKE_LAYERS];
    }
    let depth_ratio = total_depth_m / DEFAULT_THICKNESS_M.iter().sum::<f64>();
    let mut thickness = DEFAULT_THICKNESS_M
        .iter()
        .map(|value| value * depth_ratio)
        .collect::<Vec<_>>();
    thickness[0] = DEFAULT_THICKNESS_M[0];
    // `dzlak(nl)*dr - (dz_new(1) - dzlak(1)*dr)` 在 GIMPLE 里是
    // `.FMS (dzlak(nl), dr, .FNMA (dr, 0.1, 0.1))`。
    let top_excess = (-depth_ratio).contract(DEFAULT_THICKNESS_M[0], DEFAULT_THICKNESS_M[0]);
    thickness[LAKE_LAYERS - 1] =
        DEFAULT_THICKNESS_M[LAKE_LAYERS - 1].contract(depth_ratio, -top_excess);
    thickness
}

fn reconcile_phase(
    layer_thickness_m: f64,
    ice_temperature_sum: f64,
    liquid_temperature_sum: f64,
    mut ice_mass: f64,
    mut liquid_mass: f64,
) -> (f64, f64) {
    // 上游按 `wicesum > 0`、`wliqsum > 0` 分支：冰比差 1 ULP 越过 1 时 `wliqsum` 是极小的负数，
    // 这一层按纯冰处理。两者都不为正时上游不写 `t_lake_new`（未定义）。
    if ice_mass <= 0.0 {
        return (liquid_temperature_sum / liquid_mass, ice_mass);
    }
    if liquid_mass <= 0.0 {
        return (ice_temperature_sum / ice_mass, ice_mass);
    }
    let ice_temperature_k = ice_temperature_sum / ice_mass;
    let liquid_temperature_k = liquid_temperature_sum / liquid_mass;
    let liquid_heat =
        LIQUID_HEAT_CAPACITY_J_KG_K * liquid_mass * (liquid_temperature_k - FREEZING_K);
    let ice_heat = ICE_HEAT_CAPACITY_J_KG_K * ice_mass * (FREEZING_K - ice_temperature_k);
    let ice_fusion = ice_mass * FUSION_HEAT_J_KG;
    let liquid_fusion = liquid_mass * FUSION_HEAT_J_KG;
    if liquid_heat >= ice_heat + ice_fusion {
        ice_mass = 0.0;
        liquid_mass = layer_thickness_m;
        (
            FREEZING_K
                + (liquid_heat - ice_heat - ice_fusion)
                    / (liquid_mass * LIQUID_HEAT_CAPACITY_J_KG_K),
            ice_mass,
        )
    } else if liquid_heat >= ice_heat {
        ice_mass -= (liquid_heat - ice_heat) / FUSION_HEAT_J_KG;
        (FREEZING_K, ice_mass)
    } else if liquid_heat + liquid_fusion < ice_heat {
        ice_mass = layer_thickness_m;
        (
            FREEZING_K
                - (ice_heat - liquid_heat - liquid_fusion) / (ice_mass * ICE_HEAT_CAPACITY_J_KG_K),
            ice_mass,
        )
    } else {
        ice_mass += (ice_heat - liquid_heat) / FUSION_HEAT_J_KG;
        (FREEZING_K, ice_mass)
    }
}

#[cfg(test)]
#[path = "lake_tests.rs"]
mod lake_tests;
