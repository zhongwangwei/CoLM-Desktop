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

use crate::StandardLctEnergyOutput;

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
    pub net_radiation_w_m2: f64,
}

/// 把降水显热两项**逐项熔进**累加器。
///
/// 内核 `MOD_Thermal_CanopyPhase_Extended.F90:1405`（`fgrnd` 那一支是 `:1352`）的 GIMPLE
/// 对两条链给的是同一个形状：
/// ```text
/// _1762 = pg_rain*cpliq ; _1768 = .FMA(_1762, t_precip-t_grnd, 累加值)
/// _1770 = pg_snow*cpice ; _1772 = .FMA(t_precip-t_grnd, _1770, _1768)
/// _1871 = .FMA(pg_rain*cpliq, t_precip-t_grnd, _1870)     ← errore（f_zerr）那一支
/// _1872 = .FMA(t_precip-t_grnd, cpice*pg_snow, _1871)
/// ```
/// 即**两项各自熔进当时的累加值**，不是先求和再加。原先 `zerr` 那一支用的是
/// `SurfaceBudget` 里预先求和的 `precipitation_heat_w_m2` ⇒ 湿窗 `f_zerr` 残留 2 个
/// 1 ULP（干窗降水为 0，看不出来）。现在两条链共用这一个实现，`SurfaceBudget` 里
/// 那个"两项之和"的字段随之删除 —— 留着它只会再被误用一次。
pub fn add_precipitation_heat(energy: &StandardLctEnergyOutput, accumulator: f64) -> f64 {
    let difference =
        energy.precipitation.precipitation_temperature_k - energy.surface_temperature_k;
    let accumulator = (WATER_HEAT_CAPACITY_J_KG_K * energy.interception.ground_rain_kg_m2_s)
        .mul_add(difference, accumulator);
    difference.mul_add(
        ICE_HEAT_CAPACITY_J_KG_K * energy.interception.ground_snow_kg_m2_s,
        accumulator,
    )
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

    let emissivity = energy.ground_emissivity;
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
    let bulk_emissivity = emissivity.mul_add(blackbody_change, upward_longwave)
        / (upward_longwave + blackbody_change);
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
    // 内核 `MOD_Thermal…:1343` 是 `lfevpa = lfevpl + htvp*fevpg`，而 **`lfevpl` 是
    // 叶温例程里单独算好的一项**（`lfevpl = htvpl*fevpl`）—— 所以加数已经是舍入过的，
    // 只有 `htvp*fevpg` 会被吸收：`FMA(htvp, fevpg, lfevpl)`。
    // 原先把两项都平铺：干窗**第 1 步**的 `f_lfevpa` 差 1 ULP（第 0 步已全同、
    // 该步输入也全同 ⇒ 只能是形状）。
    let latent_heat =
        sublimation_heat.mul_add(ground_evaporation, leaf_latent_heat * leaf_evaporation);

    // 内核 `MOD_Thermal…:1352` 这一支是一条 6 段的累加链，**每一段的乘积都各自
    // 熔进当时的累加值**（不是先算一个两项之和再加上去）：
    //   _1747 = .FMA(dlrad, emg, sabg)
    //   _1751 = .FNMA(emg*stefnc, t_grnd_bef**4, _1747)
    //   _1753 = (emg*stefnc)*t_grnd_bef**3 ; _1757 = .FNMA(_1753, 4*tinc, _1751)
    //   _1758 = .FMA(fevpg, htvp, fseng)    ; _1760 = _1757 - _1758
    //   _1762 = pg_rain*cpliq ; _1768 = .FMA(_1762, t_precip-t_grnd, _1760)
    //   _1770 = pg_snow*cpice ; _1772 = .FMA(t_precip-t_grnd, _1770, _1768)
    // 所以只能一段一段写。旧写法有三段是平铺的；旧的"8 步离线穷举"看不见它们，
    // 因为探针那 8 行输入恰好 `sabg=0`、`fsno=0`、`t_soil==t_grnd_bef`、
    // `4*tinc` 那一乘在这几组数上熔不熔位型相同（见下）。
    let ground_heat = energy
        .leaf
        .downward_longwave_w_m2
        .mul_add(emissivity, energy.shortwave.ground_absorbed_w_m2);
    // `- emg*stefnc*t_grnd_bef**4`：这个乘积被**收进减法**。
    // 离线穷举那份 8 步位型实测：只有"这一项融合"能与内核全部对上，
    // 平铺则 6/8 步差 1 ULP（`C081C128CF597630` 对 `…631`）。
    let ground_heat = (-(emissivity * STEFAN_BOLTZMANN_W_M2_K4))
        .mul_add(previous_surface_temperature_k.powi(4), ground_heat);
    // 内核这一项是 `emg*stefnc*t_grnd_bef**3*(4.*tinc)`，**左结合**
    // `(((emg*stefnc)*t**3)*(4.*tinc))`；不能复用 `blackbody_change`
    // （那是 `stefnc*t**3*(4*tinc)`，少一层 `emg`，结合顺序不同）。
    // `_1757` 是 `.FNMA(_1753, 4*tinc, …)` —— **最后那一乘也熔进减法**。
    let ground_heat =
        (-(emissivity * STEFAN_BOLTZMANN_W_M2_K4 * previous_surface_temperature_k.powi(3)))
            .mul_add(4.0 * temperature_change_k, ground_heat);
    // `- (fseng+fevpg*htvp)`：那个乘积同样会被吸收 ⇒ `fma(fevpg, htvp, fseng)`。
    let ground_heat = ground_heat
        - sublimation_heat.mul_add(
            ground_evaporation,
            energy.corrected_ground_sensible_heat_w_m2,
        );
    // 降水显热两项：内核把**每一项**熔进累加器（`_1768`/`_1772`），不是先求和再加。
    // 这里与 `zerr` 共用 [`add_precipitation_heat`]，别再写第二套。
    let ground_heat = add_precipitation_heat(energy, ground_heat);
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
        net_radiation_w_m2: net_radiation,
    })
}
