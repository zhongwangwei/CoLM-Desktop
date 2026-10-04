//! `MOD_Ozone.F90:CalcOzoneStress` —— 臭氧对光合（`o3coefv`）与气孔（`o3coefg`）的胁迫。
//!
//! 上游在叶温迭代**收敛之后**才调它（`MOD_LeafTemperature.F90:1027-1037`、
//! `MOD_LeafTemperaturePC.F90:1787-1799`）：迭代里 `stomata` 拿到的 `o3coef*` 恒为 1
//! （迭代前无条件置 1，`:452-455`），所以 `o3coefg` 对本步的物理没有任何作用（`rssun/o3coefg`
//! 那两行在上游是注释），`o3coefv` 只在收敛后乘到 `assimsun`/`assimsha` 上。它们照样进重启与
//! 下一步——下一步一进 `LeafTemperature` 又被置 1。
//!
//! 每条浮点语句的舍入形状取自 `MOD_Ozone.F90.273t.optimized`（行号见注释）。

use crate::LibmPow;

/// `DEF_USE_OZONEDATA = .false.` 时 `CalcOzoneStress` 写回 `forc_ozone` 的常数 [ppbv]（`MOD_Ozone.F90:80`）。
pub const CONSTANT_OZONE_PPBV: f64 = 100.0;

/// 一片叶（或一个 PFT）的臭氧时间变量（`MOD_Vars_TimeVariables` / `MOD_Vars_PFTimeVariables`）。
///
/// 前七个进时间重启，名字与次序同上游：`lai_old`、`o3uptakesun`、`o3uptakesha`、
/// `o3coefv_sun`、`o3coefv_sha`、`o3coefg_sun`、`o3coefg_sha`（PFT 重启加 `_p`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OzoneState {
    /// `lai_old`：上一次 `CalcOzoneStress` 时的 `lai`。
    pub previous_leaf_area_index: f64,
    /// `o3uptakesun`/`o3uptakesha` [mmol O3 m-2]：累积吸收量。
    pub sunlit_uptake_mmol_m2: f64,
    pub shaded_uptake_mmol_m2: f64,
    /// `o3coefv_sun`/`o3coefv_sha`：光合（`vcmax`）的胁迫因子。
    pub sunlit_photosynthesis_factor: f64,
    pub shaded_photosynthesis_factor: f64,
    /// `o3coefg_sun`/`o3coefg_sha`：气孔导度的胁迫因子。
    pub sunlit_conductance_factor: f64,
    pub shaded_conductance_factor: f64,
    /// patch 的 `forc_ozone` [ppbv]：**不进重启**。`DEF_USE_OZONEDATA` 时由数据每 3 小时更新；
    /// 否则 `CalcOzoneStress` 第一次被调用时写成 [`CONSTANT_OZONE_PPBV`]，此前是 [`crate::MISSING`]
    /// （vendor 在分配后置 `spval`，upstream-bugs 第 70 条）。PFT 列上的这一项只是 patch 值的临时副本。
    pub concentration_ppbv: f64,
}

impl OzoneState {
    /// 迭代前的 `o3coef* = 1`（`MOD_LeafTemperature.F90:452-455`、`MOD_LeafTemperaturePC.F90:564-569`）。
    pub fn reset_factors(&mut self) {
        self.sunlit_photosynthesis_factor = 1.0;
        self.shaded_photosynthesis_factor = 1.0;
        self.sunlit_conductance_factor = 1.0;
        self.shaded_conductance_factor = 1.0;
    }
}

/// `CalcOzoneStress` 的植被类别参数与两个 namelist 量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OzoneParameters {
    /// `ivt`：PFT 下是 `pftclass`；**LCT 下上游写死成 1**（`MOD_Thermal.F90:718` 的
    /// `CALL LeafTemperature(ipatch,1,...)`），即所有地类都按温带常绿针叶林取参数。
    pub vegetation_type: i32,
    /// `isevg(ivt)`
    pub evergreen: bool,
    /// `leaf_long(ivt)` [年]（PFT/PC 下含 `DEF_PFT_LEAF_LONG` 覆盖）。
    pub leaf_longevity_years: f64,
    /// `DEF_OZONE_KO3`：气孔阻抗对臭氧的放大系数。
    pub stomatal_resistance_factor: f64,
    /// `DEF_USE_OZONEDATA`（只在 `DEF_USE_OZONESTRESS` 打开时可能为真）。
    pub use_data: bool,
}

impl OzoneParameters {
    /// `CalcOzoneStress` 开头：没有臭氧数据时 `forc_ozone = 100`（`intent(inout)`，写回调用方）。
    pub fn effective_concentration_ppbv(&self, concentration_ppbv: f64) -> f64 {
        if self.use_data {
            concentration_ppbv
        } else {
            CONSTANT_OZONE_PPBV
        }
    }
}

/// 一次 `CalcOzoneStress` 调用的气象与冠层量。
#[derive(Debug, Clone, Copy)]
pub struct OzoneUptakeInput {
    /// 已经过 [`OzoneParameters::effective_concentration_ppbv`] 的 `forc_ozone` [ppbv]。
    pub concentration_ppbv: f64,
    /// `forc_psrf` [Pa]
    pub surface_pressure_pa: f64,
    /// `th`：位温 [K]
    pub potential_temperature_k: f64,
    /// `ram` [s m-1]：最后一轮迭代的动量空气动力学阻抗。
    pub aerodynamic_resistance_s_m: f64,
    /// `rs`：叶尺度气孔阻抗（`rssun`/`rssha`，已乘回 `laisun`/`laisha`）。
    pub stomatal_resistance_s_m: f64,
    /// `rb`：叶边界层阻抗。
    pub boundary_resistance_s_m: f64,
    pub leaf_area_index: f64,
    /// `lai_old`
    pub previous_leaf_area_index: f64,
    /// `sabv`：冠层吸收的短波 [W m-2]，只用来判断白天。
    pub absorbed_solar_w_m2: f64,
    pub time_step_seconds: f64,
}

/// `CalcOzoneStress`：更新累积吸收量 `o3uptake`，返回 `(o3coefv, o3coefg)`。
///
/// 两个因子在开头先置 1（upstream-bugs 第 71 条：`ivt` 落在五个类别之外——裸地 `ivt = 0`——且吸收量
/// 非零时，上游原来两个 `intent(out)` 哑元都不赋值）。
// `f64::clamp` 遇 NaN 返回 NaN、且对 `-0.0` 的取舍与 `MIN_EXPR`/`MAX_EXPR` 链不同，这里照抄上游的 min/max 次序。
#[allow(clippy::manual_clamp)]
pub fn ozone_stress(
    parameters: OzoneParameters,
    input: OzoneUptakeInput,
    uptake_mmol_m2: &mut f64,
) -> (f64, f64) {
    let ivt = parameters.vegetation_type;
    // `:83` `_5 = th*8.314; _7 = psrf/_5; o3concnmolm3 = _7*forc_ozone`
    let concentration_nmol_m3 = input.surface_pressure_pa / (input.potential_temperature_k * 8.314)
        * input.concentration_ppbv;
    // `:86` `_13 = .FMA (rs, DEF_OZONE_KO3, rb); o3flux = o3conc/(_13 + ram)`
    let flux = concentration_nmol_m3
        / (input.stomatal_resistance_s_m.mul_add(
            parameters.stomatal_resistance_factor,
            input.boundary_resistance_s_m,
        ) + input.aerodynamic_resistance_s_m);
    // `:89-97`：常绿 0，温带灌木（`ivt == 10`）0.3，其余落叶 0.5。
    let lai_threshold = if parameters.evergreen {
        0.0
    } else if ivt == 10 {
        0.3
    } else {
        0.5
    };
    // `:101-116`
    let flux_threshold = match ivt {
        1..=3 => 0.8,
        4..=8 => 1.0,
        9..=11 => 6.0,
        12..=14 => 1.6,
        15.. => 0.5,
        _ => 10.0,
    };
    // `:119-123`：GIMPLE 是 `if (threshold > o3flux)`。
    let critical_flux = if flux_threshold > flux {
        0.0
    } else {
        flux - flux_threshold
    };
    // `:126-130` `_27 = deltim*o3fluxcrit; o3fluxperdt = _27*1e-6`
    let flux_per_step = if input.absorbed_solar_w_m2 > 0.0 {
        input.time_step_seconds * critical_flux * 1.0e-6
    } else {
        0.0
    };
    // `:132-146`
    if input.leaf_area_index > lai_threshold {
        let decay = if parameters.evergreen {
            // `:135` `leafturn = 2/((leaf_long*365)*24)`；`:136` `((leafturn*o3uptake)*deltim)/3600`
            let turnover = 2.0 / (parameters.leaf_longevity_years * 365.0 * 24.0);
            turnover * *uptake_mmol_m2 * input.time_step_seconds / 3600.0
        } else {
            // `:138` `MAX_EXPR <1 - lai_old/lai, 0> * o3uptake`
            (1.0 - input.previous_leaf_area_index / input.leaf_area_index).max(0.0)
                * *uptake_mmol_m2
        };
        // `:142` `_42 = o3fluxperdt + o3uptake; MIN_EXPR <MAX_EXPR <_42 - decay, 0>, 90>`
        *uptake_mmol_m2 = (flux_per_step + *uptake_mmol_m2 - decay).max(0.0).min(90.0);
    } else {
        *uptake_mmol_m2 = 0.0;
    }
    let uptake = *uptake_mmol_m2;
    if uptake == 0.0 {
        return (1.0, 1.0);
    }
    // `:154-173`：每条都是 `MAX_EXPR <MIN_EXPR <x, 1>, 0>`；`a - b*x` 一律是 `.FNMA (x, b, a)`。
    let clamp = |value: f64| value.min(1.0).max(0.0);
    match ivt {
        1..=3 => (
            clamp((-uptake).mul_add(0.0064, 1.005)),
            // `:156` `_45 = __builtin_pow (o3uptake, -0.041); _45*0.965`
            clamp(uptake.lpow(-0.041) * 0.965),
        ),
        4..=8 => (
            clamp((-(uptake * 0.0085)).exp() * 0.943),
            clamp((-(uptake * 0.0058)).exp() * 0.943),
        ),
        9..=11 => {
            // `:163-164` 两条共用同一个 `__builtin_log`。
            let log = uptake.ln();
            (
                clamp((-log).mul_add(0.074, 1.0)),
                clamp((-log).mul_add(0.060, 0.991)),
            )
        }
        12..=14 => (
            clamp((-uptake).mul_add(0.016, 0.997)),
            clamp((-uptake.ln()).mul_add(0.045, 0.989)),
        ),
        15.. => (
            clamp((-uptake.ln()).mul_add(0.028, 0.909)),
            clamp((-uptake.tanh()).mul_add(0.169, 1.005)),
        ),
        _ => (1.0, 1.0),
    }
}

/// 收敛后的一对 `CalcOzoneStress`（阳叶、阴叶）加 `lai_old = lai`：返回 `(o3coefv_sun, o3coefv_sha)`，
/// 调用方拿它们去乘 `assimsun`/`assimsha`。`concentration_ppbv` 写回 [`OzoneState::concentration_ppbv`]。
#[allow(clippy::too_many_arguments)]
pub fn canopy_ozone_stress(
    parameters: OzoneParameters,
    state: &mut OzoneState,
    surface_pressure_pa: f64,
    potential_temperature_k: f64,
    aerodynamic_resistance_s_m: f64,
    sunlit_stomatal_resistance_s_m: f64,
    shaded_stomatal_resistance_s_m: f64,
    boundary_resistance_s_m: f64,
    leaf_area_index: f64,
    absorbed_solar_w_m2: f64,
    time_step_seconds: f64,
) -> (f64, f64) {
    let concentration = parameters.effective_concentration_ppbv(state.concentration_ppbv);
    state.concentration_ppbv = concentration;
    let input = |stomatal_resistance_s_m: f64| OzoneUptakeInput {
        concentration_ppbv: concentration,
        surface_pressure_pa,
        potential_temperature_k,
        aerodynamic_resistance_s_m,
        stomatal_resistance_s_m,
        boundary_resistance_s_m,
        leaf_area_index,
        previous_leaf_area_index: state.previous_leaf_area_index,
        absorbed_solar_w_m2,
        time_step_seconds,
    };
    let sunlit_input = input(sunlit_stomatal_resistance_s_m);
    let shaded_input = input(shaded_stomatal_resistance_s_m);
    (
        state.sunlit_photosynthesis_factor,
        state.sunlit_conductance_factor,
    ) = ozone_stress(parameters, sunlit_input, &mut state.sunlit_uptake_mmol_m2);
    (
        state.shaded_photosynthesis_factor,
        state.shaded_conductance_factor,
    ) = ozone_stress(parameters, shaded_input, &mut state.shaded_uptake_mmol_m2);
    state.previous_leaf_area_index = leaf_area_index;
    (
        state.sunlit_photosynthesis_factor,
        state.shaded_photosynthesis_factor,
    )
}

#[cfg(test)]
#[path = "ozone_tests.rs"]
mod ozone_tests;
