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

use anyhow::{ensure, Result};

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

pub fn surface_budget(energy: &StandardLctEnergyOutput) -> Result<SurfaceBudget> {
    let ground = &energy.ground;
    ensure!(
        ground.temperature_k.len() == ground.previous_temperature_k.len(),
        "the ground temperature column disagrees with its previous step"
    );
    let surface_temperature_k = energy.surface_temperature_k;
    let previous_surface_temperature_k = energy.surface_temperature_k_before;
    let temperature_change_k = surface_temperature_k - previous_surface_temperature_k;

    let emissivity = ground_emissivity(ground.snow_water_equivalent_kg_m2, 0);
    let upward_longwave = energy.leaf.upward_longwave_w_m2;
    let blackbody_change = STEFAN_BOLTZMANN_W_M2_K4
        * previous_surface_temperature_k.powi(3)
        * (4.0 * temperature_change_k);
    // `olrg = ulrad + 4.*emg*stefnc*t_grnd_bef**3*tinc`（`…Thermal…_Extended.F90:1364`）。
    // dump（`th_ext.opt`）显示内核把它收成
    // `_1813=emg*4; _1814=_1813*stefnc; _1816=_1814*t**3; olrg=FMA(_1816,tinc,ulrad)` ——
    // 写成 `ulrad + emg*(stefnc*t**3*(4*tinc))` 既换了结合顺序也少了收缩。
    let outgoing_longwave_coefficient =
        emissivity * 4.0 * STEFAN_BOLTZMANN_W_M2_K4 * previous_surface_temperature_k.powi(3);
    let outgoing_longwave =
        outgoing_longwave_coefficient.mul_add(temperature_change_k, upward_longwave);
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
    // `MOD_Vars_1DAccFluxes.F90:2087`：`rnet = sabg + sabvsun + sabvsha - olrg + forc_frl`。
    //
    // **曾经写成 `fsena + lfevpa + fgrnd`**，理由是"与辐射式恒等"。那个恒等只在
    // 能量收支**精确闭合**时成立，而这里每一步都有 ~1e-11 的残差；更关键的是
    // `fgrnd` 本身就含辐射项（见上），`H_total + LE_total + G` 与辐射式并不是同一个
    // 表达式的两种写法。实测 US-NR1-snow 逐点输出：第 1 步起 `f_rnet` 差 19%，
    // 第 4 步差 43%，整窗 360/360 条超差，而同一份文件里的 `f_fgrnd`/`f_lfevpa`/
    // `f_olrg`/`f_sabg` 都是逐位相同的 —— 差的只是这一个诊断量。
    let net_radiation = energy.shortwave.ground_absorbed_w_m2
        + energy.shortwave.sunlit_absorbed_w_m2
        + energy.shortwave.shaded_absorbed_w_m2
        - outgoing_longwave
        + energy.forcing_longwave_w_m2;

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
