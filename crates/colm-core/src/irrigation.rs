//! CROP 灌溉（`main/MOD_Irrigation.F90`）：BGC driver 里的 `CalIrrigationNeeded` 与 `CoLMMAIN` 里的
//! `CalIrrigationApplicationFluxes`。
//!
//! 浮点收缩按 CROP 内核的 GIMPLE（`MOD_Irrigation.F90.273t.optimized`）：全模块只有三处——
//! 施灌的 `waterstorage - irrig_rate*deltim`（两次，都是 FNMA）与阈值
//! `wilt + THRESHOLD*(target - wilt)`（FMA）。

use anyhow::{ensure, Result};

use crate::{soil_vliq_from_psi, LibmPow, SoilHydraulicModel, FREEZING_K};

pub const IRRIGATION_DRIP: i32 = 1;
pub const IRRIGATION_SPRINKLER: i32 = 2;
pub const IRRIGATION_FLOOD: i32 = 3;
pub const IRRIGATION_PADDY: i32 = 4;

/// `npcropmin`：第一个作物 PFT 类别。
const FIRST_CROP_CLASS: i32 = 17;
/// `nbedrock`（`MOD_Vars_Global`，常数 10）：土层 `j > nbedrock` 视为基岩。
const BEDROCK_LAYER: usize = 10;

/// 灌溉的 namelist 设置（`DEF_TUNING_IRRIGATION_*`、`DEF_IRRIGATION_ALLOCATION` 与两个土壤开关）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IrrigationSettings {
    pub start_seconds: f64,
    pub duration_seconds: f64,
    pub max_depth_m: f64,
    pub threshold_fraction: f64,
    pub supply_fraction: f64,
    pub min_crop_phase: f64,
    pub max_crop_phase: f64,
    /// `DEF_TUNING_IRRIGATION_PONDMX`：水田的积水上限 [mm]。
    pub paddy_ponding_limit_mm: f64,
    /// `DEF_IRRIGATION_ALLOCATION`（1、2、3）。
    pub allocation: i32,
    pub variably_saturated_flow: bool,
    pub campbell: bool,
    /// `DEF_simulation_time%greenwich`：开始时刻按地方时比。
    pub greenwich: bool,
}

/// 一个 patch 的灌溉状态（`MOD_Vars_TimeVariables` 的灌溉量与 `irrig_method_p`）。
///
/// 前八项进重启；`deficit`…`runoff_supply` 每次 `CalIrrigationNeeded` 先清零，只供历史。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IrrigationState {
    /// `irrig_method_p`（patch 内按 PFT）。水稻的漫灌在检查时被**就地**改成水田，并随重启保留。
    pub methods: Vec<i32>,
    pub rate_mm_s: f64,
    pub steps_left: i32,
    pub water_storage_mm: f64,
    pub sum_mm: f64,
    pub sum_deficit_mm: f64,
    pub sum_count: f64,
    pub zwt_stand_m: f64,
    pub groundwater_allocation: f64,
    pub surface_water_allocation: f64,
    pub deficit_mm: f64,
    pub actual_mm: f64,
    pub groundwater_demand_mm: f64,
    pub groundwater_supply_mm: f64,
    pub reservoirriver_demand_mm: f64,
    pub reservoirriver_supply_mm: f64,
    pub reservoir_supply_mm: f64,
    pub river_supply_mm: f64,
    pub runoff_supply_mm: f64,
}

/// 本步施到地面的灌溉通量 [mm/s]。喷灌走冠层截留，其余三种直接进土壤水的 `gwat`。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct IrrigationApplicationFluxes {
    pub drip_mm_s: f64,
    pub sprinkler_mm_s: f64,
    pub flood_mm_s: f64,
    pub paddy_mm_s: f64,
}

/// 土壤水分步要的灌溉量（`MOD_SoilSnowHydrology.F90` 的 `#ifdef CROP` 各段）。
///
/// `DEF_USE_IRRIGATION` 打开时**每个** patch 都给（非土壤 patch 的通量为 0、`methods` 为空）：
/// `WATER_2014` 的 `gwat = gwat + wdsrf/deltim` 不看 patch 类型。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilIrrigation<'a> {
    pub drip_mm_s: f64,
    pub flood_mm_s: f64,
    pub paddy_mm_s: f64,
    /// 只有 `patchtype == 0` 的水田规则看它；其余 patch 给空切片。
    pub methods: &'a [i32],
    pub paddy_ponding_limit_mm: f64,
}

impl SoilIrrigation<'_> {
    /// `gwat` 加上滴灌、漫灌、水田三项（`MOD_SoilSnowHydrology.F90:298-300`）。
    pub fn ground_water_input(&self, gwat: f64) -> f64 {
        ((gwat + self.drip_mm_s) + self.flood_mm_s) + self.paddy_mm_s
    }

    pub fn has_paddy(&self) -> bool {
        self.methods.contains(&IRRIGATION_PADDY)
    }
}

impl IrrigationState {
    /// `CalIrrigationApplicationFluxes`（`patchtype == 0` 才调）。
    pub fn application_fluxes(&mut self, time_step_seconds: f64) -> IrrigationApplicationFluxes {
        let mut fluxes = IrrigationApplicationFluxes::default();
        for &method in &self.methods {
            if self.steps_left > 0 {
                self.steps_left -= 1;
                // GIMPLE：两处 `waterstorage - irrig_rate*deltim` 都是 FNMA。
                if (-self.rate_mm_s).mul_add(time_step_seconds, self.water_storage_mm) < 0.0 {
                    self.rate_mm_s = self.water_storage_mm / time_step_seconds;
                }
                self.water_storage_mm = (-self.rate_mm_s)
                    .mul_add(time_step_seconds, self.water_storage_mm)
                    .max(0.0);
                match method {
                    IRRIGATION_DRIP => fluxes.drip_mm_s = self.rate_mm_s,
                    IRRIGATION_FLOOD => fluxes.flood_mm_s = self.rate_mm_s,
                    IRRIGATION_PADDY => fluxes.paddy_mm_s = self.rate_mm_s,
                    // 喷灌与未知方式都按喷灌。
                    _ => fluxes.sprinkler_mm_s = self.rate_mm_s,
                }
            } else {
                self.rate_mm_s = 0.0;
            }
        }
        fluxes
    }
}

/// `CalIrrigationNeeded` 读写的土壤柱与时间（BGC driver 的物理量，`1:nl_soil`）。
pub struct IrrigationColumn<'a> {
    /// BGC 的 `idate`（年、年内日、日内秒）。
    pub idate: [i32; 3],
    pub time_step_seconds: f64,
    /// `dlon`（度）。
    pub longitude_deg: f64,
    pub pft_class: &'a [i32],
    pub crop_phase: &'a [f64],
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    /// `zi_soi(1:nl_soil)`：各层**底**界面深度。
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub alpha_vgm: &'a [f64],
    pub n_vgm: &'a [f64],
    pub l_vgm: &'a [f64],
    pub sc_vgm: &'a [f64],
    pub fc_vgm: &'a [f64],
    /// 非 VSF 的地下水取水会改这三项。
    pub liquid_water_kg_m2: &'a mut [f64],
    pub water_table_depth_m: &'a mut f64,
    pub aquifer_water_mm: &'a mut f64,
}

/// `CalIrrigationNeeded`（`MOD_BGC_driver.F90:115-117`，在 `CNGResp` 之后）。
pub fn irrigation_needed(
    state: &mut IrrigationState,
    settings: IrrigationSettings,
    column: IrrigationColumn<'_>,
) -> Result<()> {
    let npft = state.methods.len();
    ensure!(
        column.pft_class.len() == npft && column.crop_phase.len() == npft,
        "irrigation: the PFT classes and crop phases do not match the irrigation methods"
    );
    ensure!(npft > 0, "irrigation: a patch needs at least one PFT");
    let dt = column.time_step_seconds;
    state.deficit_mm = 0.0;
    state.actual_mm = 0.0;
    state.groundwater_demand_mm = 0.0;
    state.groundwater_supply_mm = 0.0;
    state.reservoirriver_demand_mm = 0.0;
    state.reservoirriver_supply_mm = 0.0;
    state.reservoir_supply_mm = 0.0;
    state.river_supply_mm = 0.0;
    state.runoff_supply_mm = 0.0;

    // 年初第一步（`idate(2) == 1 .and. idate(3) == deltim`）清零年累计并取当时的水位当基准。
    if column.idate[1] == 1 && f64::from(column.idate[2]) == dt {
        state.sum_mm = 0.0;
        state.sum_deficit_mm = 0.0;
        state.sum_count = 0.0;
        state.zwt_stand_m = (*column.water_table_depth_m + 1.0).clamp(0.0, 80.0);
    }

    let check = needs_check(state, settings, &column);
    if check {
        potential_needed(state, settings, &column)?;
        limited_supply(state, settings, column)?;
    }
    if check && state.deficit_mm > 0.0 {
        state.sum_deficit_mm += state.deficit_mm;
    }
    if check && state.actual_mm > 0.0 {
        let steps = (settings.duration_seconds / dt).round();
        state.rate_mm_s = state.actual_mm / dt / steps;
        state.steps_left = steps as i32;
        state.sum_mm += state.actual_mm;
        state.sum_count += 1.0;
    }
    Ok(())
}

/// `PointNeedsCheckForIrrig`：水稻（62）的漫灌改成水田；结果取**最后一个** PFT 的判定。
fn needs_check(
    state: &mut IrrigationState,
    settings: IrrigationSettings,
    column: &IrrigationColumn<'_>,
) -> bool {
    for (method, &class) in state.methods.iter_mut().zip(column.pft_class) {
        if class == 62 && *method == IRRIGATION_FLOOD {
            *method = IRRIGATION_PADDY;
        }
    }
    let dt = column.time_step_seconds;
    let seconds = if settings.greenwich {
        local_seconds(column.idate[2], column.longitude_deg)
    } else {
        f64::from(column.idate[2])
    };
    let elapsed = seconds - settings.start_seconds + dt;
    let mut check = false;
    for (m, &class) in column.pft_class.iter().enumerate() {
        check = class >= FIRST_CROP_CLASS
            && irrigated_crop(class)
            && column.crop_phase[m] >= settings.min_crop_phase
            && column.crop_phase[m] < settings.max_crop_phase
            && elapsed >= 0.0
            && elapsed < dt;
    }
    check
}

/// `gmt2local` 的 `ldate(3)`：`idate(3) + dlon/15*3600`，再卷回 `[0, 86400]`。
fn local_seconds(seconds: i32, longitude_deg: f64) -> f64 {
    let local = f64::from(seconds) + longitude_deg / 15.0 * 3600.0;
    if local < 0.0 {
        86_400.0 + local
    } else if local > 86_400.0 {
        local - 86_400.0
    } else {
        local
    }
}

/// `irrig_crop`（`MOD_Const_PFT`）：16 起的偶数类别。
fn irrigated_crop(class: i32) -> bool {
    (16..=78).contains(&class) && class % 2 == 0
}

/// `CalIrrigationPotentialNeeded`。
fn potential_needed(
    state: &mut IrrigationState,
    settings: IrrigationSettings,
    column: &IrrigationColumn<'_>,
) -> Result<()> {
    let nl = column.porosity.len();
    // 上游 `smpswc = -1.5e5`、`smpsfc = -3.3e3`（单精度字面量，都能精确表示）。
    let (wilting_psi, field_psi) = (-1.5e5, -3.3e3);
    let mut wilting = vec![0.0; nl];
    let mut field = vec![0.0; nl];
    let mut saturation = vec![0.0; nl];
    for j in 0..nl {
        if column.temperature_k[j] > FREEZING_K && column.porosity[j] >= 1.0e-6 {
            // GIMPLE：`denh2o*dz*x` 取成 `x * (dz*1000)`。
            let capacity = column.layer_thickness_m[j] * 1000.0;
            let porosity = column.porosity[j];
            if settings.campbell {
                let exponent = -(1.0 / column.clapp_hornberger_b[j]);
                let pore = porosity * capacity;
                wilting[j] = pore * (wilting_psi / column.saturated_potential_mm[j]).lpow(exponent);
                field[j] = pore * (field_psi / column.saturated_potential_mm[j]).lpow(exponent);
                saturation[j] = pore;
            } else {
                let model = SoilHydraulicModel::VanGenuchten {
                    alpha_vgm: column.alpha_vgm[j],
                    n_vgm: column.n_vgm[j],
                    l_vgm: column.l_vgm[j],
                    sc_vgm: column.sc_vgm[j],
                    fc_vgm: column.fc_vgm[j],
                };
                let vliq = |psi: f64| {
                    soil_vliq_from_psi(
                        psi,
                        porosity,
                        column.residual_water[j],
                        column.saturated_potential_mm[j],
                        model,
                    )
                };
                wilting[j] = vliq(wilting_psi) * capacity;
                field[j] = capacity * vliq(field_psi);
                saturation[j] = porosity * capacity;
            }
        }
    }

    // 上游的 PFT 循环不重置 `reached_max_depth` 与各总量：第二个 PFT 起只在第一个没碰到深度
    // 上限时才会再累加一遍。照搬。
    let mut reached_max_depth = false;
    let (mut liquid, mut wilting_total, mut field_total, mut saturation_total) =
        (0.0, 0.0, 0.0, 0.0);
    let mut target = 0.0;
    for &method in &state.methods {
        for j in 0..nl {
            if reached_max_depth {
                continue;
            }
            if column.node_depth_m[j] > settings.max_depth_m
                || j + 1 > BEDROCK_LAYER
                || column.temperature_k[j] <= FREEZING_K
            {
                reached_max_depth = true;
            } else {
                liquid += column.liquid_water_kg_m2[j];
                wilting_total += wilting[j];
                field_total += field[j];
                saturation_total += saturation[j];
            }
        }
        target = if method == IRRIGATION_PADDY {
            saturation_total
        } else {
            field_total
        };
    }

    let threshold = (target - wilting_total).mul_add(settings.threshold_fraction, wilting_total);
    state.deficit_mm = 0.0;
    for &method in &state.methods {
        state.deficit_mm = if liquid < threshold {
            let goal = if method == IRRIGATION_FLOOD {
                saturation_total
            } else {
                field_total
            };
            settings.supply_fraction * (goal - liquid)
        } else {
            0.0
        };
    }
    Ok(())
}

/// `CalIrrigationLimitedSupply`。
fn limited_supply(
    state: &mut IrrigationState,
    settings: IrrigationSettings,
    column: IrrigationColumn<'_>,
) -> Result<()> {
    if state.deficit_mm <= 0.0 {
        return Ok(());
    }
    match settings.allocation {
        1 => {
            state.actual_mm = state.deficit_mm;
            state.water_storage_mm += state.actual_mm;
        }
        2 | 3 => {
            let from_storage = state.water_storage_mm.min(state.deficit_mm).max(0.0);
            state.actual_mm += from_storage;
            if state.deficit_mm > state.actual_mm {
                let shortfall = state.deficit_mm - state.actual_mm;
                state.groundwater_demand_mm = if settings.allocation == 2 {
                    shortfall.max(0.0)
                } else {
                    (shortfall * state.groundwater_allocation).max(0.0)
                };
                if settings.variably_saturated_flow {
                    state.groundwater_supply_mm = state.groundwater_demand_mm;
                } else {
                    withdraw_groundwater(state, column);
                }
                state.actual_mm += state.groundwater_supply_mm;
                state.water_storage_mm += state.groundwater_supply_mm;
            }
        }
        other => anyhow::bail!("DEF_IRRIGATION_ALLOCATION must be 1, 2 or 3, got {other}"),
    }
    Ok(())
}

/// `CalWithdrawalWATER`：非 VSF 时从地下水（含含水层）抽水，改 `wliq`/`zwt`/`wa`。
///
/// 比水量用 Campbell 的 `psi0`/`bsw`（上游不看土壤模型）。上游开头算的 `vol_ice`/`eff_porosity`
/// 之后没有用到，这里省掉。
fn withdraw_groundwater(state: &mut IrrigationState, column: IrrigationColumn<'_>) {
    let nl = column.porosity.len();
    let zwt = column.water_table_depth_m;
    let wa = column.aquifer_water_mm;
    let wliq = column.liquid_water_kg_m2;
    let zi = column.interface_depth_m;
    let specific_yield = |j: usize, zwt: f64| {
        let ratio = 1.0 - zwt * 1000.0 / column.saturated_potential_mm[j];
        let s = column.porosity[j] * (1.0 - ratio.lpow(-(1.0 / column.clapp_hornberger_b[j])));
        s.max(0.02)
    };
    let water_table_layer = |zwt: f64| zi.iter().position(|&z| zwt <= z).unwrap_or(nl);
    let jwt = water_table_layer(*zwt);
    let rous = specific_yield(nl - 1, *zwt);

    if jwt == nl {
        let max_supply = ((state.zwt_stand_m - *zwt) * 1000.0 * rous).max(0.0);
        state.groundwater_supply_mm = state.groundwater_demand_mm.min(max_supply);
        *wa -= state.groundwater_supply_mm;
        *zwt = (*zwt + state.groundwater_supply_mm / 1000.0 / rous).max(0.0);
        wliq[nl - 1] += (*wa - 5000.0).max(0.0);
        *wa = wa.min(5000.0);
    } else {
        let mut pump_total = -state.groundwater_demand_mm;
        for j in jwt..nl {
            let s_y = specific_yield(j, *zwt);
            let pump_layer = pump_total.max(-(s_y * (zi[j] - *zwt) * 1000.0)).min(0.0);
            wliq[j] += pump_layer;
            pump_total -= pump_layer;
            state.groundwater_supply_mm -= pump_layer;
            if pump_total >= 0.0 {
                *zwt = (*zwt - pump_layer / s_y / 1000.0).max(0.0);
                break;
            }
            *zwt = zi[j];
        }
        let max_supply = ((state.zwt_stand_m - *zwt) * 1000.0 * rous).max(0.0);
        let pump = (-pump_total).min(max_supply).max(0.0);
        state.groundwater_supply_mm += pump;
        *zwt = (*zwt + pump / 1000.0 / rous).max(0.0);
        *wa -= pump;
    }
    *zwt = zwt.clamp(0.0, 80.0);

    let mut deficit = 0.0;
    for value in wliq.iter_mut() {
        if *value < 0.0 {
            deficit += *value;
            *value = 0.0;
        }
    }
    *wa += deficit;
}

#[cfg(test)]
#[path = "irrigation_tests.rs"]
mod tests;
