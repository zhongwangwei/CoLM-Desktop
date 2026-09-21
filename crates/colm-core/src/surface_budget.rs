//! `MOD_Thermal.F90` 收尾处的**地表收支中间量**，history 与续跑写回共用一份。
//!
//! 上游在 `MOD_Thermal.F90:1331-1362` 一次算完 `olrg`/`emis`/`trad`/`fgrnd`/`lfevpa`，
//! 然后**两个消费者**读它：history 写出（`MOD_Hist.F90` 的 `a_*`）与重启写出
//! （`MOD_Vars_TimeVariables` 的 `trad`/`emis`）。本仓库原先把它放在
//! `colm-runtime/history.rs` 里，于是重启写回拿不到 —— 实测重启里 `trad`/`emis`
//! 一直是入参那份（283.0 / 1.0），而上游同时刻是 254.261338 / 1.000315。
//!
//! 放在 `colm-core` 是因为它只依赖内核输出，且**必须**只有一份实现：分成两份
//! 就会出现"同一份文件里的 `f_zerr` 与 `f_olrg` 互相矛盾"这种只在拼错时才暴露的坑。

use anyhow::{Context, Result};

use crate::{ground_emissivity, StandardLctEnergyOutput};

/// `MOD_Thermal.F90` 收尾处的物理常数（`stefnc`/`cpliq`/`cpice`）。
const STEFAN_BOLTZMANN_W_M2_K4: f64 = 5.67e-8;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;

/// `MOD_Thermal.F90:1331-1362` 收尾处的那一组地表中间量。
///
/// 抽出来是因为 `f_fgrnd`/`f_lfevpa`/`f_olrg`（`set_lct_surface_budget`）与
/// `f_zerr`（`set_lct_balance_errors`）读的是**同一组项**：分成两份实现，
/// 迟早会出现"同一份文件里的 `f_zerr` 与 `f_olrg` 互相矛盾"这种只在拼错时才暴露的坑。
pub struct SurfaceBudget {
    pub outgoing_longwave_w_m2: f64,
    pub bulk_emissivity: f64,
    pub radiative_temperature_k: f64,
    pub latent_heat_w_m2: f64,
    pub ground_heat_w_m2: f64,
    /// `cpliq*pg_rain*(t_precip-t_grnd) + cpice*pg_snow*(t_precip-t_grnd)`。
    ///
    /// 单独留一份是因为 `zerr` 要把它原样加回去：它在 `fgrnd` 里是长表达式的一部分，
    /// 而 `errore` 把它写在末尾 —— 从 `fgrnd` 里反解出来会引入第二套算式。
    pub precipitation_heat_w_m2: f64,
    pub net_radiation_w_m2: f64,
}

pub fn surface_budget(
    energy: &StandardLctEnergyOutput,
    soil_layers: usize,
) -> Result<SurfaceBudget> {
    let ground = &energy.ground;
    // 打包列里第一个**土层**的下标：列长减去土层数。**不能**写 0 —— 带雪时
    // `temperature_k[0]` 是雪面温度，而 `t_grnd_bef`/`tinc` 要的是地表那一层。
    let soil_surface = ground
        .temperature_k
        .len()
        .checked_sub(soil_layers)
        .filter(|index| *index < ground.temperature_k.len())
        .context("the packed ground temperature column is shorter than the soil")?;
    let surface_temperature_k = ground.temperature_k[soil_surface];
    let previous_surface_temperature_k =
        ground
            .previous_temperature_k
            .get(soil_surface)
            .copied()
            .context("the ground temperature state carries no previous surface layer")?;
    let temperature_change_k = surface_temperature_k - previous_surface_temperature_k;

    let emissivity = ground_emissivity(ground.snow_water_equivalent_kg_m2, 0);
    let upward_longwave = energy.leaf.upward_longwave_w_m2;
    let blackbody_change = STEFAN_BOLTZMANN_W_M2_K4
        * previous_surface_temperature_k.powi(3)
        * (4.0 * temperature_change_k);
    let outgoing_longwave = upward_longwave + emissivity * blackbody_change;
    let bulk_emissivity =
        (upward_longwave + emissivity * blackbody_change) / (upward_longwave + blackbody_change);
    let radiative_temperature_k = (outgoing_longwave / STEFAN_BOLTZMANN_W_M2_K4).powf(0.25);

    // 上游的 `htvp`（`MOD_Thermal.F90:539-540`）由内核按**表层是否纯冰**定好，
    // 随步输出带出来；这里照抄，不再自己判一次。写成无条件的 `hvap + hfus`
    // 会把所有液态地表的地面蒸发按升华计价 —— 实测冬季窗口 `f_lfevpa` 差 34 W/m²。
    let sublimation_heat = energy.leaf.ground_latent_heat_j_kg;
    // 叶面那一项同样是**内核自己判定**的 `htvpl`（`lfevpl = htvpl*fevpl`），
    // 不是 `hvap`。见 [`colm_core::LeafTemperatureOutput::leaf_latent_heat_j_kg`]。
    let leaf_latent_heat = energy.leaf.leaf_latent_heat_j_kg;
    let leaf_evaporation = energy.leaf.leaf_evaporation_kg_m2_s;
    // **必须取订正后的地面蒸发**，与 `set_lct_energy_fluxes` 写进 `f_fevpg` 的那一列同源。
    // `leaf.ground_evaporation_kg_m2_s` 是叶温求解**之前**的初步值，两者在 CN-Cng
    // 首条记录上差 2.5 倍（9.3e-5 对 2.3e-4）。用初步值会让
    // `lfevpa = hvap*fevpl + htvp*fevpg` 与同一份文件里的 `f_fevpl`/`f_fevpg`
    // 自相矛盾 —— 实测 Rust 的 `f_lfevpa` 峰值 615 W/m² 而 `hvap*(f_fevpl+f_fevpg)`
    // 只有 187 W/m²；改用订正后立刻落到 196 W/m²（Fortran 184.65）。
    let ground_evaporation = energy.corrected_ground_evaporation_kg_m2_s;
    let latent_heat = leaf_latent_heat * leaf_evaporation + sublimation_heat * ground_evaporation;

    let precipitation_temperature_k = energy.precipitation.precipitation_temperature_k;
    let precipitation_heat = WATER_HEAT_CAPACITY_J_KG_K
        * energy.interception.ground_rain_kg_m2_s
        * (precipitation_temperature_k - surface_temperature_k)
        + ICE_HEAT_CAPACITY_J_KG_K
            * energy.interception.ground_snow_kg_m2_s
            * (precipitation_temperature_k - surface_temperature_k);
    let ground_heat = energy.shortwave.ground_absorbed_w_m2
        + energy.leaf.downward_longwave_w_m2 * emissivity
        - emissivity * STEFAN_BOLTZMANN_W_M2_K4 * previous_surface_temperature_k.powi(4)
        - emissivity * blackbody_change
        - (energy.corrected_ground_sensible_heat_w_m2 + ground_evaporation * sublimation_heat)
        + precipitation_heat;
    // 地表能量收支恒等式：`rnet = H + LE + G`。用它而不是再拼一遍辐射项，
    // 是因为前者的每一项都已经由内核算过，重复拼装只会引入第二套公式。
    let net_radiation = energy.total_sensible_heat_w_m2 + latent_heat + ground_heat;

    Ok(SurfaceBudget {
        outgoing_longwave_w_m2: outgoing_longwave,
        bulk_emissivity,
        radiative_temperature_k,
        latent_heat_w_m2: latent_heat,
        ground_heat_w_m2: ground_heat,
        precipitation_heat_w_m2: precipitation_heat,
        net_radiation_w_m2: net_radiation,
    })
}
