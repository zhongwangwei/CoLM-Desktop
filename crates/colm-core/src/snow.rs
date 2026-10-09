//! Runtime snow-column updates from MOD_NewSnow.F90.

use crate::LibmPow;
use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::FREEZING_K;

pub(crate) const MAX_SNOW_LAYERS: usize = 5;
// `MOD_Const_Physical:tfrz`, compiled by the Desktop reference with
// `-fdefault-real-8`.
pub(crate) const SNOW_AGE_FREEZING_K: f64 = 273.16;

use crate::f77;

#[derive(Clone, Copy, Default)]
struct SnowLayer {
    thickness_m: f64,
    temperature_k: f64,
    liquid_water_kg_m2: f64,
    ice_water_kg_m2: f64,
}

/// Mutable snow portion of CoLM's combined soil/snow column.
///
/// Layer vectors use Fortran indexes -4 through 0 in ascending order. Interface
/// vectors use -5 through 0. Soil layers intentionally remain in the caller's
/// separate state; MOD_NewSnow only changes this snow prefix.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSnowColumn {
    pub layer_count: i32,
    pub interface_depth_m: Vec<f64>,
    pub node_depth_m: Vec<f64>,
    pub thickness_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub previous_ice_fraction: Vec<f64>,
    pub age: f64,
    pub water_equivalent_kg_m2: f64,
    pub depth_m: f64,
    pub ground_snow_fraction: f64,
}

impl RuntimeSnowColumn {
    /// Allocates CoLM's compiled five-layer snow prefix.
    pub fn empty() -> Self {
        Self {
            layer_count: 0,
            interface_depth_m: vec![0.0; MAX_SNOW_LAYERS + 1],
            node_depth_m: vec![0.0; MAX_SNOW_LAYERS],
            thickness_m: vec![0.0; MAX_SNOW_LAYERS],
            temperature_k: vec![0.0; MAX_SNOW_LAYERS],
            liquid_water_kg_m2: vec![0.0; MAX_SNOW_LAYERS],
            ice_water_kg_m2: vec![0.0; MAX_SNOW_LAYERS],
            previous_ice_fraction: vec![0.0; MAX_SNOW_LAYERS],
            age: 0.0,
            water_equivalent_kg_m2: 0.0,
            depth_m: 0.0,
            ground_snow_fraction: 0.0,
        }
    }
}

/// 从时间重启读回的雪列，按 CoLM 的**槽位顺序**给出（Fortran 下标 `-4..0`）。
///
/// 上游把 `z_sno`/`dz_sno`/`t_soisno` 等的雪段整段写在 `snow` 维上，长度是
/// `-maxsnl = 5`，顺序就是 Fortran 的 `-4, -3, -2, -1, 0`，与
/// [`RuntimeSnowColumn`] 的下标一一对应，所以这里直接按序收。
#[derive(Debug, Clone, Copy)]
pub struct RestartSnowSlots<'a> {
    pub node_depth_m: &'a [f64; MAX_SNOW_LAYERS],
    pub thickness_m: &'a [f64; MAX_SNOW_LAYERS],
    pub temperature_k: &'a [f64; MAX_SNOW_LAYERS],
    pub liquid_water_kg_m2: &'a [f64; MAX_SNOW_LAYERS],
    pub ice_water_kg_m2: &'a [f64; MAX_SNOW_LAYERS],
    pub water_equivalent_kg_m2: f64,
    pub depth_m: f64,
    pub ground_snow_fraction: f64,
    pub age: f64,
}

impl RuntimeSnowColumn {
    /// 由重启里的雪段构造运行态雪列。
    ///
    /// **层数不在重启里**：上游 `CoLMMAIN.F90:816-818` 按水量现数 —— 从 `-4` 到 `0`，
    /// 只要 `wliq + wice > 0` 就把 `snl` 减一；界面深度再由
    /// `zi(j) = zi(j+1) - dz(j+1)`（`:820-826`）递推。两者都照做，不自作主张用雪深推。
    ///
    /// 由于 `standard_lct_snow_soil_step` 只认「最后 `|snl|` 个槽位是有雪层」，
    /// 而上面的数法允许中间空一层（那样数出来的层数与实际占用不符），这里**显式拒绝**
    /// 非连续的雪段：上游会带着一个对不上的列继续跑，我们宁可先报错。
    pub fn from_restart(patch_type: i32, slots: RestartSnowSlots<'_>) -> Result<Self> {
        let RestartSnowSlots {
            node_depth_m,
            thickness_m,
            temperature_k,
            liquid_water_kg_m2,
            ice_water_kg_m2,
            water_equivalent_kg_m2,
            depth_m,
            ground_snow_fraction,
            age,
        } = slots;
        for (name, values) in [
            ("water_equivalent_kg_m2", [water_equivalent_kg_m2]),
            ("depth_m", [depth_m]),
            ("ground_snow_fraction", [ground_snow_fraction]),
            ("age", [age]),
        ] {
            ensure!(values[0].is_finite(), "restart snow {name} is not finite");
        }
        ensure!(
            water_equivalent_kg_m2 >= 0.0 && depth_m >= 0.0,
            "restart snow has a negative water equivalent or depth"
        );
        ensure!(
            (0.0..=1.0).contains(&ground_snow_fraction),
            "restart snow cover fraction {ground_snow_fraction} is outside [0, 1]"
        );
        for index in 0..MAX_SNOW_LAYERS {
            for (name, value) in [
                ("z_sno", node_depth_m[index]),
                ("dz_sno", thickness_m[index]),
                ("t_soisno", temperature_k[index]),
                ("wliq_soisno", liquid_water_kg_m2[index]),
                ("wice_soisno", ice_water_kg_m2[index]),
            ] {
                ensure!(
                    value.is_finite(),
                    "restart snow {name} slot {index} is not finite"
                );
            }
            ensure!(
                liquid_water_kg_m2[index] >= 0.0 && ice_water_kg_m2[index] >= 0.0,
                "restart snow slot {index} has negative water"
            );
        }
        // 上游的数法：`snl = 0; IF (wliq(j)+wice(j) > 0) snl = snl - 1`，`j = -4..0`。
        let layer_count = (0..MAX_SNOW_LAYERS)
            .filter(|index| liquid_water_kg_m2[*index] + ice_water_kg_m2[*index] > 0.0)
            .count() as i32;
        let layer_count = -layer_count;
        // 有雪的槽位必须是最靠上的那几层，否则上面的计数与内核按「最后 |snl| 个槽位」
        // 的组织方式不一致。
        let used = MAX_SNOW_LAYERS - layer_count.unsigned_abs() as usize;
        for (index, (liquid, ice)) in liquid_water_kg_m2
            .iter()
            .zip(ice_water_kg_m2.iter())
            .enumerate()
        {
            let has_water = liquid + ice > 0.0;
            ensure!(
                has_water == (index >= used),
                "restart snow slot {index} breaks the column: layers above the count must be                  empty and the counted ones must hold water"
            );
        }
        ensure!(
            layer_count == 0 || depth_m > 0.0,
            "a restart with {layer_count} snow layers needs a positive depth"
        );
        if patch_type > 3 {
            ensure!(
                layer_count == 0,
                "a water body patch (patchtype {patch_type}) cannot carry a snow column"
            );
        }

        let mut column = Self::empty();
        column.layer_count = layer_count;
        column.water_equivalent_kg_m2 = water_equivalent_kg_m2;
        column.depth_m = depth_m;
        column.ground_snow_fraction = ground_snow_fraction;
        column.age = age;
        // 只有被数进去的槽位带值；其余保持 `empty()` 的 0，免得把上一轮的残值带进来。
        //
        // 槽位下标恰好等于数组下标：Fortran 的层 `-4..0` 对应 `RuntimeSnowColumn` 的
        // 层槽位 `0..4`，而重启里的雪段就是按 `-4..0` 写的。
        for index in used..MAX_SNOW_LAYERS {
            column.node_depth_m[index] = node_depth_m[index];
            column.thickness_m[index] = thickness_m[index];
            column.temperature_k[index] = temperature_k[index];
            column.liquid_water_kg_m2[index] = liquid_water_kg_m2[index];
            column.ice_water_kg_m2[index] = ice_water_kg_m2[index];
            let total = liquid_water_kg_m2[index] + ice_water_kg_m2[index];
            column.previous_ice_fraction[index] = ice_water_kg_m2[index] / total;
        }
        // `zi(0) = 0; zi(j) = zi(j+1) - dz(j+1)`，`j = -1..snl`（`CoLMMAIN.F90:820-826`）。
        // 界面槽位是 `j + 5`，层槽位是 `j + 4`，所以 `zi(j)` 用到的
        // `dz(j+1)` 落在界面下标同号的层槽位上：`interface[i] = interface[i+1] - thickness[i]`。
        // 循环从 `i = 4`（`j = -1`）走到 `i = snl + 5 = used`，**不含** `i = 5`（`zi(0)`）。
        column.interface_depth_m[snow_interface_slot(0)] = 0.0;
        for index in (used..MAX_SNOW_LAYERS).rev() {
            column.interface_depth_m[index] =
                column.interface_depth_m[index + 1] - column.thickness_m[index];
        }
        Ok(column)
    }
}

/// Inputs to MOD_NewSnow.F90:newsnow that affect its snow state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewSnowInput {
    pub patch_type: i32,
    pub time_step_seconds: f64,
    pub ground_temperature_k: f64,
    pub ground_snowfall_kg_m2_s: f64,
    pub new_snow_bulk_density_kg_m3: f64,
    pub precipitation_temperature_k: f64,
    pub variably_saturated_flow: bool,
}

/// Water transferred to a warm wetland's external storage by fresh snow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewSnowOutcome {
    pub wetland_water_added_mm: f64,
}

/// Surface fluxes consumed by `MOD_SoilSnowHydrology:snowwater`.
///
/// CoLM's `mm h2o/s` fluxes are numerically equal to `kg m-2 s-1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnowWaterInput {
    pub time_step_seconds: f64,
    pub irreducible_saturation: f64,
    pub impermeable_porosity: f64,
    pub rainfall_kg_m2_s: f64,
    pub evaporation_kg_m2_s: f64,
    pub dew_kg_m2_s: f64,
    pub sublimation_kg_m2_s: f64,
    pub frost_kg_m2_s: f64,
}

/// Per-layer drainage produced by `snowwater` in surface-to-bottom order.
#[derive(Debug, Clone, PartialEq)]
pub struct SnowWaterOutcome {
    pub bottom_drainage_kg_m2_s: f64,
    pub layer_drainage_kg_m2: Vec<f64>,
}

/// `MOD_SnowFraction:snowfraction` 的三个输出。
///
/// `wt`/`sigf` 只影响冠层几何（`CoLMMAIN.F90:2096-2102`：`sai = tsai*sigf`，
/// `DEF_VEG_SNOW` 打开时还有 `lai = tlai*sigf`），`fsno` 影响地面反照率与
/// `netsolar` 的土壤/雪吸收拆分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnowFraction {
    /// `wt`：被积雪埋住的植被比例 [-]。
    pub vegetation_snow_fraction: f64,
    /// `sigf`：未被雪埋的植被比例，无冠层时为 1 [-]。
    pub vegetation_free_fraction: f64,
    /// `fsno`：被雪盖住的地面比例 [-]。
    pub ground_snow_fraction: f64,
}

/// 移植 `MOD_SnowFraction:snowfraction`。
///
/// 上游无冠层（`lai+sai <= 1e-6`）时把 `wt` 置 0、`sigf` 置 1；有冠层时按
/// `wt = 0.1*snowdp/z0m` 的埋没比例折算。地面雪盖用
/// `fsno = tanh(snowdp/(2.5*zlnd*fmelt))`、`fmelt = (scv/snowdp/100)^exponent`，
/// 只在 `snowdp > 0` 时计算，否则 `fsno = 0`。
///
/// **`z0m` 是本步 `THERMAL` 刚算出的冠层动量粗糙度，不是常数。** 上游在每步末尾
/// 才调用本函数（`CoLMMAIN.F90:2095`），用的正是这一步更新后的 `z0m`；用装配期的
/// 固定值会让 `fsno` 的埋没项与地面反照率都停在启动时刻。
///
/// `zlnd` 是**裸土**粗糙度（`param%z0s`，本仓库 `physics.soil_roughness_m`），
/// 与 `z0m` 不是一回事。
pub fn snow_fraction(
    leaf_area_index: f64,
    stem_area_index: f64,
    momentum_roughness_m: f64,
    soil_roughness_m: f64,
    snow_water_equivalent_mm: f64,
    snow_depth_m: f64,
    cover_exponent: f64,
) -> Result<SnowFraction> {
    ensure!(
        leaf_area_index.is_finite()
            && leaf_area_index >= 0.0
            && stem_area_index.is_finite()
            && stem_area_index >= 0.0
            && momentum_roughness_m.is_finite()
            && momentum_roughness_m > 0.0
            && soil_roughness_m.is_finite()
            && soil_roughness_m > 0.0
            && snow_water_equivalent_mm.is_finite()
            && snow_water_equivalent_mm >= 0.0
            && snow_depth_m.is_finite()
            && snow_depth_m >= 0.0
            && cover_exponent.is_finite()
            && cover_exponent > 0.0,
        "snow-fraction inputs are invalid"
    );

    let (vegetation_snow_fraction, vegetation_free_fraction) =
        if leaf_area_index + stem_area_index > 1.0e-6 {
            let buried = 0.1 * snow_depth_m / momentum_roughness_m;
            let buried = buried / (1.0 + buried);
            (buried, 1.0 - buried)
        } else {
            (0.0, 1.0)
        };

    let ground_snow_fraction = if snow_depth_m > 0.0 {
        let melting_factor = (snow_water_equivalent_mm / snow_depth_m / 100.0).lpow(cover_exponent);
        (snow_depth_m / (2.5 * soil_roughness_m * melting_factor)).tanh()
    } else {
        0.0
    };

    Ok(SnowFraction {
        vegetation_snow_fraction,
        vegetation_free_fraction,
        ground_snow_fraction,
    })
}

/// Ports `MOD_Albedo:snowage` for the non-SNICAR broadband path.
///
/// `snow_water_equivalent_mm` and `previous_snow_water_equivalent_mm` are
/// CoLM's `scv` and `scvold`; the returned value is the next `sag` state.
pub fn update_snow_age(
    time_step_seconds: f64,
    ground_temperature_k: f64,
    snow_water_equivalent_mm: f64,
    previous_snow_water_equivalent_mm: f64,
    snow_age: f64,
) -> Result<f64> {
    ensure!(
        time_step_seconds.is_finite()
            && time_step_seconds > 0.0
            && ground_temperature_k.is_finite()
            && snow_water_equivalent_mm.is_finite()
            && snow_water_equivalent_mm >= 0.0
            && previous_snow_water_equivalent_mm.is_finite()
            && previous_snow_water_equivalent_mm >= 0.0
            && snow_age.is_finite()
            && snow_age >= 0.0,
        "snow-age inputs are invalid"
    );
    if snow_water_equivalent_mm == 0.0 || snow_water_equivalent_mm > 800.0 {
        return Ok(0.0);
    }
    let argument = 5.0e3 * (1.0 / SNOW_AGE_FREEZING_K - 1.0 / ground_temperature_k);
    // `MOD_Albedo.F90:1323-1326` 的 `snowage`：GIMPLE 把 `sge = (sag+dela)*(1-dels)`
    // 拆成 `FMA(deltim*1e-6, 增长项, sag) * FNMA(max(0,scv-scvold), 0.1, 1.0)`
    // —— `dela` 的乘积与 `dels` 的乘积**都没有单独舍入**，各自被收进一次加法。
    let aging_rate = 1.0e-6 * time_step_seconds;
    let growth = argument.exp() + (10.0 * argument).min(0.0).exp() + 0.3;
    let fresh_snow = (snow_water_equivalent_mm - previous_snow_water_equivalent_mm).max(0.0);
    Ok((aging_rate.contract(growth, snow_age) * (-fresh_snow).contract(0.1, 1.0)).max(0.0))
}

/// Port of MOD_NewSnow.F90:newsnow.
///
/// Rainfall is intentionally absent: the upstream routine receives it but does not
/// use it. The returned wetland transfer lets the runtime own its wetland store.
pub fn add_new_snow(input: NewSnowInput, state: &mut RuntimeSnowColumn) -> Result<NewSnowOutcome> {
    validate(input, state)?;
    let snowfall_depth_rate_m_s = input.ground_snowfall_kg_m2_s / input.new_snow_bulk_density_kg_m3;
    state.depth_m += snowfall_depth_rate_m_s * input.time_step_seconds;
    state.water_equivalent_kg_m2 += input.ground_snowfall_kg_m2_s * input.time_step_seconds;

    if input.patch_type == 2 && input.ground_temperature_k > FREEZING_K && state.layer_count == 0 {
        let wetland_water_added_mm = if input.variably_saturated_flow {
            state.water_equivalent_kg_m2
        } else {
            0.0
        };
        state.water_equivalent_kg_m2 = 0.0;
        state.depth_m = 0.0;
        state.age = 0.0;
        state.ground_snow_fraction = 0.0;
        return Ok(NewSnowOutcome {
            wetland_water_added_mm,
        });
    }

    let interface_zero = interface_slot(0);
    state.interface_depth_m[interface_zero] = 0.0;
    let mut new_node = false;
    if state.layer_count == 0 && input.ground_snowfall_kg_m2_s > 0.0 && state.depth_m >= f77(0.01) {
        state.layer_count = -1;
        new_node = true;
        let top = layer_slot(0);
        state.thickness_m[top] = state.depth_m;
        state.node_depth_m[top] = -f77(0.5) * state.thickness_m[top];
        state.interface_depth_m[interface_slot(-1)] = -state.thickness_m[top];
        state.age = 0.0;
        state.temperature_k[top] = FREEZING_K.min(input.precipitation_temperature_k);
        state.ice_water_kg_m2[top] = state.water_equivalent_kg_m2;
        state.liquid_water_kg_m2[top] = 0.0;
        state.previous_ice_fraction[top] = 1.0;
        state.ground_snow_fraction =
            (f77(0.1) * input.ground_snowfall_kg_m2_s * input.time_step_seconds)
                .tanh()
                .min(f77(1.0));
    }

    if state.layer_count < 0 && !new_node {
        let top_index = state.layer_count + 1;
        let top = layer_slot(top_index);
        state.ice_water_kg_m2[top] += input.time_step_seconds * input.ground_snowfall_kg_m2_s;
        state.thickness_m[top] += snowfall_depth_rate_m_s * input.time_step_seconds;
        state.node_depth_m[top] =
            state.interface_depth_m[interface_slot(top_index)] - f77(0.5) * state.thickness_m[top];
        state.interface_depth_m[interface_slot(top_index - 1)] =
            state.interface_depth_m[interface_slot(top_index)] - state.thickness_m[top];
        // `fsno = 1. - (1. - tanh(0.1*pg_snow*deltim))*(1. - fsno)`（`MOD_NewSnow.F90:121`）：
        // GIMPLE 是 `.FNMA (1-tanh, 1-fsno, 1.0)`。平铺写法让 AT-Neu 1 月第 131 步
        // （雪层建出后第一次往层里加雪）的 `fsno` 差 1 ULP（第 402 轮）。
        state.ground_snow_fraction = (-(f77(1.0)
            - (f77(0.1) * input.ground_snowfall_kg_m2_s * input.time_step_seconds).tanh()))
        .contract(f77(1.0) - state.ground_snow_fraction, f77(1.0));
        state.ground_snow_fraction = state.ground_snow_fraction.min(f77(1.0));
    }
    Ok(NewSnowOutcome {
        wetland_water_added_mm: 0.0,
    })
}

/// Ports `MOD_SoilSnowHydrology:snowwater` for an existing snow column.
///
/// This deliberately does not update the aggregate `scv`/`snowdp` fields:
/// CoLM updates those in the following snow-layer combine step, which callers
/// must run after this percolation step.
pub fn snow_water(
    input: SnowWaterInput,
    state: &mut RuntimeSnowColumn,
) -> Result<SnowWaterOutcome> {
    validate_active_snow_layers(state)?;
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.irreducible_saturation.is_finite()
            && (0.0..=1.0).contains(&input.irreducible_saturation)
            && input.impermeable_porosity.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.rainfall_kg_m2_s.is_finite()
            && input.evaporation_kg_m2_s.is_finite()
            && input.dew_kg_m2_s.is_finite()
            && input.sublimation_kg_m2_s.is_finite()
            && input.frost_kg_m2_s.is_finite(),
        "snow-water inputs are invalid"
    );
    ensure!(
        state.layer_count < 0,
        "snow-water requires at least one active snow layer"
    );

    let first_layer = state.layer_count + 1;
    let top = layer_slot(first_layer);
    // `MOD_SoilSnowHydrology` 的 `snowwater`：表层冰/液更新都是
    // `wice/wliq = wice/wliq + (通量)*deltim`，GIMPLE 把 `通量*deltim`
    // 收进加法（`FMA(deltim, 通量, 原值)`）—— 两处都是。
    let top_ice_after_surface_flux = input.time_step_seconds.contract(
        input.frost_kg_m2_s - input.sublimation_kg_m2_s,
        state.ice_water_kg_m2[top],
    );
    state.ice_water_kg_m2[top] = top_ice_after_surface_flux.max(0.0);
    if top_ice_after_surface_flux < 0.0 {
        state.liquid_water_kg_m2[top] += top_ice_after_surface_flux;
    }
    state.liquid_water_kg_m2[top] = input.time_step_seconds.contract(
        input.rainfall_kg_m2_s + input.dew_kg_m2_s - input.evaporation_kg_m2_s,
        state.liquid_water_kg_m2[top],
    );
    if state.liquid_water_kg_m2[top] < 0.0 {
        state.ice_water_kg_m2[top] =
            (state.ice_water_kg_m2[top] + state.liquid_water_kg_m2[top]).max(0.0);
        state.liquid_water_kg_m2[top] = 0.0;
    }

    let active_layers = state.layer_count.unsigned_abs() as usize;
    let mut ice_volume_fraction = vec![0.0; active_layers];
    let mut liquid_volume_fraction = vec![0.0; active_layers];
    let mut effective_porosity = vec![0.0; active_layers];
    for (relative, fortran_layer) in (first_layer..=0).enumerate() {
        let slot = layer_slot(fortran_layer);
        ice_volume_fraction[relative] =
            (state.ice_water_kg_m2[slot] / (state.thickness_m[slot] * f77(917.0))).min(1.0);
        effective_porosity[relative] = (1.0 - ice_volume_fraction[relative]).max(0.01);
        liquid_volume_fraction[relative] = (state.liquid_water_kg_m2[slot]
            / (state.thickness_m[slot] * f77(1000.0)))
        .min(effective_porosity[relative]);
    }

    let mut inflow = 0.0;
    let mut layer_drainage_kg_m2 = Vec::with_capacity(active_layers);
    for (relative, fortran_layer) in (first_layer..=0).enumerate() {
        let slot = layer_slot(fortran_layer);
        state.liquid_water_kg_m2[slot] += inflow;
        // `MOD_SoilSnowHydrology.F90:1447/1452`：两条 `j` 分支都是
        // `qout = max(0., (vol_liq - ssi*eff_porosity)*dz)`。GIMPLE 是
        // `_48 = FNMA(ssi, eff, vol_liq); _131 = _48*dz; MAX(_131, 0)`
        // —— 不可约含水那一步（`vol_liq - ssi*eff`）被吸收，`max` 留在乘法**外**。
        // 这两条分支在 dump 里各出现一次（`bb24`/`bb25`），两处都融合。
        let excess_depth_m = ((-input.irreducible_saturation).contract(
            effective_porosity[relative],
            liquid_volume_fraction[relative],
        ) * state.thickness_m[slot])
            .max(0.0);
        let outflow = if fortran_layer < 0 {
            let next = relative + 1;
            if effective_porosity[relative] < input.impermeable_porosity
                || effective_porosity[next] < input.impermeable_porosity
            {
                0.0
            } else {
                excess_depth_m.min(
                    (1.0 - ice_volume_fraction[next] - liquid_volume_fraction[next])
                        * state.thickness_m[layer_slot(fortran_layer + 1)],
                )
            }
        } else {
            excess_depth_m
        } * f77(1000.0);
        layer_drainage_kg_m2.push(outflow);
        state.liquid_water_kg_m2[slot] -= outflow;
        inflow = outflow;
    }

    Ok(SnowWaterOutcome {
        bottom_drainage_kg_m2_s: inflow / input.time_step_seconds,
        layer_drainage_kg_m2,
    })
}

/// Snow mass transferred into the upper soil node when a snow layer vanishes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnowToSoilTransfer {
    pub liquid_water_kg_m2: f64,
    pub ice_water_kg_m2: f64,
}

/// Applies MOD_SnowLayersCombineDivide:snowlayerscombine without SNICAR or tracers.
///
/// The upper soil node is an explicit argument because the upstream routine
/// deposits vanished snow mass there rather than discarding it.
pub fn combine_snow_layers(
    state: &mut RuntimeSnowColumn,
    soil_surface: &mut SnowToSoilTransfer,
) -> Result<()> {
    combine_snow_layers_with_aerosols(state, soil_surface, None)
}

/// 雪层每槽八种气溶胶质量 `[slot][species]`（Fortran `-4:0`）。
pub type SnowAerosolMasses = [[f64; 8]; 5];

/// `SnowLayersCombine_snicar`：水热部分与 [`combine_snow_layers`] 相同，另把气溶胶质量随层
/// 搬运（下移并入 `j<0` 时加到下一层、平移复制、相邻层合并相加；雪全没了时整列清零）。
/// 空出来的槽里的气溶胶上游不清，留给步末 `AerosolMasses`。
pub fn combine_snow_layers_with_aerosols(
    state: &mut RuntimeSnowColumn,
    soil_surface: &mut SnowToSoilTransfer,
    aerosols: Option<&mut SnowAerosolMasses>,
) -> Result<()> {
    let mut unused = [[0.0; 8]; 5];
    let aerosols = aerosols.unwrap_or(&mut unused);
    validate_snow_topology(state, soil_surface)?;
    if state.layer_count == 0 {
        return Ok(());
    }

    let initial_count = state.layer_count;
    let mut layer_count = state.layer_count;
    for fortran_layer in initial_count + 1..=0 {
        if state.ice_water_kg_m2[layer_slot(fortran_layer)] > f77(0.1) {
            continue;
        }
        transfer_layer_down(state, soil_surface, fortran_layer);
        if fortran_layer < 0 {
            let upper = aerosols[layer_slot(fortran_layer)];
            add_masses(&mut aerosols[layer_slot(fortran_layer + 1)], &upper);
        }
        if fortran_layer > layer_count + 1 && layer_count < -1 {
            for destination in (layer_count + 2..=fortran_layer).rev() {
                copy_layer(state, destination - 1, destination);
                aerosols[layer_slot(destination)] = aerosols[layer_slot(destination - 1)];
            }
        }
        layer_count += 1;
    }
    state.layer_count = layer_count;
    if layer_count == 0 {
        state.water_equivalent_kg_m2 = 0.0;
        state.depth_m = 0.0;
        clear_snow_layers(state);
        *aerosols = [[0.0; 8]; 5];
        return Ok(());
    }

    let (snow_mass, snow_depth, ice_mass, liquid_mass) = snow_totals(state);
    state.water_equivalent_kg_m2 = snow_mass;
    state.depth_m = snow_depth;
    if snow_depth < f77(0.01) {
        state.layer_count = 0;
        state.water_equivalent_kg_m2 = ice_mass;
        state.depth_m = if ice_mass <= 0.0 { 0.0 } else { snow_depth };
        soil_surface.liquid_water_kg_m2 += liquid_mass;
        clear_snow_layers(state);
        *aerosols = [[0.0; 8]; 5];
        return Ok(());
    }

    if layer_count < -1 {
        let mut minimum_index = 0;
        let initial_count = layer_count;
        for fortran_layer in initial_count + 1..=0 {
            let slot = layer_slot(fortran_layer);
            if state.thickness_m[slot]
                >= [f77(0.010), f77(0.015), f77(0.025), f77(0.055), f77(0.115)][minimum_index]
            {
                minimum_index += 1;
                continue;
            }

            let neighbor = if fortran_layer == layer_count + 1 {
                fortran_layer + 1
            } else if fortran_layer == 0
                || state.thickness_m[layer_slot(fortran_layer - 1)] + state.thickness_m[slot]
                    < state.thickness_m[layer_slot(fortran_layer + 1)] + state.thickness_m[slot]
            {
                fortran_layer - 1
            } else {
                fortran_layer + 1
            };
            let (target, other) = if neighbor > fortran_layer {
                (neighbor, fortran_layer)
            } else {
                (fortran_layer, neighbor)
            };
            combine_layer_pair(state, target, other);
            let other_masses = aerosols[layer_slot(other)];
            add_masses(&mut aerosols[layer_slot(target)], &other_masses);
            if target - 1 > layer_count + 1 {
                for destination in (layer_count + 2..=target - 1).rev() {
                    copy_layer(state, destination - 1, destination);
                    aerosols[layer_slot(destination)] = aerosols[layer_slot(destination - 1)];
                }
            }
            layer_count += 1;
            state.layer_count = layer_count;
            if layer_count >= -1 {
                break;
            }
        }
    }

    // 合并相邻层之后上游**不**重算 `scv`/`snowdp`（`MOD_SnowLayersCombineDivide.F90`
    // 只重建节点/界面深度）：两者停在合并之前那次累加的值。合并本身守恒质量与厚度，
    // 但重新累加的舍入顺序不同 —— 实测第二轮预热 1 月 14 日 `scv` 差 1 ulp（第 406 轮）。
    rebuild_snow_geometry(state);
    Ok(())
}

fn validate_snow_topology(
    state: &RuntimeSnowColumn,
    soil_surface: &SnowToSoilTransfer,
) -> Result<()> {
    validate_active_snow_layers(state)?;
    // 表层液水允许舍入级的负值：新霜挤出 `wliq - min(max(wliq-cap,0), max(wliq,0))` 在
    // 容量接近 0 时会留下 -1 ulp 量级的残差（实测 bc 算例 -9e-54），上游不夹也不检查。
    ensure!(
        soil_surface.liquid_water_kg_m2.is_finite()
            && soil_surface.liquid_water_kg_m2 >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
            && soil_surface.ice_water_kg_m2.is_finite()
            && soil_surface.ice_water_kg_m2 >= 0.0,
        "soil-surface transfer state is invalid (wliq {}, wice {})",
        soil_surface.liquid_water_kg_m2,
        soil_surface.ice_water_kg_m2
    );
    Ok(())
}

fn validate_active_snow_layers(state: &RuntimeSnowColumn) -> Result<()> {
    validate_runtime_snow_column(state)?;
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        ensure!(
            state.thickness_m[slot].is_finite()
                && state.thickness_m[slot] > 0.0
                && state.temperature_k[slot].is_finite()
                && state.liquid_water_kg_m2[slot].is_finite()
                && state.liquid_water_kg_m2[slot] >= 0.0
                && state.ice_water_kg_m2[slot].is_finite()
                && state.ice_water_kg_m2[slot] >= 0.0,
            "active snow layer is invalid"
        );
    }
    Ok(())
}

fn transfer_layer_down(
    state: &mut RuntimeSnowColumn,
    soil_surface: &mut SnowToSoilTransfer,
    fortran_layer: i32,
) {
    let slot = layer_slot(fortran_layer);
    if fortran_layer == 0 {
        soil_surface.liquid_water_kg_m2 += state.liquid_water_kg_m2[slot];
        soil_surface.ice_water_kg_m2 += state.ice_water_kg_m2[slot];
    } else {
        let destination = layer_slot(fortran_layer + 1);
        state.liquid_water_kg_m2[destination] += state.liquid_water_kg_m2[slot];
        state.ice_water_kg_m2[destination] += state.ice_water_kg_m2[slot];
    }
}

fn copy_layer(state: &mut RuntimeSnowColumn, source: i32, destination: i32) {
    let source = layer_slot(source);
    let destination = layer_slot(destination);
    state.temperature_k[destination] = state.temperature_k[source];
    state.liquid_water_kg_m2[destination] = state.liquid_water_kg_m2[source];
    state.ice_water_kg_m2[destination] = state.ice_water_kg_m2[source];
    state.thickness_m[destination] = state.thickness_m[source];
}

fn combine_layer_pair(state: &mut RuntimeSnowColumn, target: i32, other: i32) {
    let target_slot = layer_slot(target);
    let other_slot = layer_slot(other);
    let target_layer = SnowLayer {
        thickness_m: state.thickness_m[target_slot],
        temperature_k: state.temperature_k[target_slot],
        liquid_water_kg_m2: state.liquid_water_kg_m2[target_slot],
        ice_water_kg_m2: state.ice_water_kg_m2[target_slot],
    };
    let other_layer = SnowLayer {
        thickness_m: state.thickness_m[other_slot],
        temperature_k: state.temperature_k[other_slot],
        liquid_water_kg_m2: state.liquid_water_kg_m2[other_slot],
        ice_water_kg_m2: state.ice_water_kg_m2[other_slot],
    };
    let combined = combine_snow_values(target_layer, other_layer);
    state.thickness_m[target_slot] = combined.thickness_m;
    state.temperature_k[target_slot] = combined.temperature_k;
    state.liquid_water_kg_m2[target_slot] = combined.liquid_water_kg_m2;
    state.ice_water_kg_m2[target_slot] = combined.ice_water_kg_m2;
}

fn combine_snow_values(target: SnowLayer, other: SnowLayer) -> SnowLayer {
    let thickness_m = target.thickness_m + other.thickness_m;
    let ice_water_kg_m2 = target.ice_water_kg_m2 + other.ice_water_kg_m2;
    let liquid_water_kg_m2 = target.liquid_water_kg_m2 + other.liquid_water_kg_m2;
    // `combo`（`MOD_SnowLayersCombineDivide.F90:900-916`）不被内联，GIMPLE 是：
    //   热容 `.FMA (wice, cpice, wliq*cpliq)`，`h = .FMA (热容, t-tfrz, wliq*hfus)`，两层各算一个 `h`
    //   再相加 `hc = h + h2`；合并后的热容同样 `.FMA (wicec, cpice, wliqc*cpliq)`。
    // 没有液水时每个 FMA 的加数都是 0，与平铺写法逐位相同；有液水时平铺（且四项连加）差 1 ULP
    // （AT-Neu 第二年 12 月 31 日雪层分裂，第 432 轮）。
    let capacity = |ice: f64, liquid: f64| ice.contract(f77(2117.27), liquid * f77(4188.0));
    let layer_enthalpy = |layer: SnowLayer| {
        capacity(layer.ice_water_kg_m2, layer.liquid_water_kg_m2).contract(
            layer.temperature_k - FREEZING_K,
            layer.liquid_water_kg_m2 * f77(0.3336e6),
        )
    };
    let enthalpy = layer_enthalpy(target) + layer_enthalpy(other);
    let heat_capacity = capacity(ice_water_kg_m2, liquid_water_kg_m2);
    let temperature_k = if enthalpy < 0.0 {
        enthalpy / heat_capacity + FREEZING_K
    } else if enthalpy <= liquid_water_kg_m2 * f77(0.3336e6) {
        FREEZING_K
    } else {
        (enthalpy - liquid_water_kg_m2 * f77(0.3336e6)) / heat_capacity + FREEZING_K
    };
    SnowLayer {
        thickness_m,
        temperature_k,
        liquid_water_kg_m2,
        ice_water_kg_m2,
    }
}

fn rebuild_snow_geometry(state: &mut RuntimeSnowColumn) {
    state.interface_depth_m[interface_slot(0)] = 0.0;
    for fortran_layer in (state.layer_count + 1..=0).rev() {
        let slot = layer_slot(fortran_layer);
        state.node_depth_m[slot] = state.interface_depth_m[interface_slot(fortran_layer)]
            - f77(0.5) * state.thickness_m[slot];
        state.interface_depth_m[interface_slot(fortran_layer - 1)] =
            state.interface_depth_m[interface_slot(fortran_layer)] - state.thickness_m[slot];
    }
}

/// `scv = scv + wice(j) + wliq(j)`：逐层、按这个结合顺序累加（`:384`），
/// 不是分别求和冰与液再相加 —— 两种写法在多层时舍入不同。
fn snow_totals(state: &RuntimeSnowColumn) -> (f64, f64, f64, f64) {
    let mut snow_mass = 0.0;
    let mut ice_mass = 0.0;
    let mut liquid_mass = 0.0;
    let mut depth = 0.0;
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        snow_mass = snow_mass + state.ice_water_kg_m2[slot] + state.liquid_water_kg_m2[slot];
        ice_mass += state.ice_water_kg_m2[slot];
        liquid_mass += state.liquid_water_kg_m2[slot];
        depth += state.thickness_m[slot];
    }
    (snow_mass, depth, ice_mass, liquid_mass)
}

fn clear_snow_layers(state: &mut RuntimeSnowColumn) {
    for field in [
        &mut state.node_depth_m,
        &mut state.thickness_m,
        &mut state.temperature_k,
        &mut state.liquid_water_kg_m2,
        &mut state.ice_water_kg_m2,
        &mut state.previous_ice_fraction,
    ] {
        field.fill(0.0);
    }
    state.interface_depth_m.fill(0.0);
}

/// Applies MOD_SnowLayersCombineDivide:snowlayersdivide without SNICAR or tracers.
pub fn divide_snow_layers(state: &mut RuntimeSnowColumn) -> Result<()> {
    divide_snow_layers_with_aerosols(state, None)
}

/// `SnowLayersDivide_snicar`：气溶胶按冰量同样的比例随层拆分（对半、`propor` 拆出、
/// `z_mss + mss(next)` 并入下一层），最后写回活动槽。
pub fn divide_snow_layers_with_aerosols(
    state: &mut RuntimeSnowColumn,
    aerosols: Option<&mut SnowAerosolMasses>,
) -> Result<()> {
    let mut unused = [[0.0; 8]; 5];
    let aerosols = aerosols.unwrap_or(&mut unused);
    validate_snow_topology(state, &SnowToSoilTransfer::default())?;
    if state.layer_count == 0 {
        return Ok(());
    }

    let mut layer_count = state.layer_count.unsigned_abs() as usize;
    let mut layers = [SnowLayer::default(); MAX_SNOW_LAYERS];
    let mut masses = [[0.0; 8]; MAX_SNOW_LAYERS];
    for (position, layer) in layers.iter_mut().enumerate().take(layer_count) {
        let slot = layer_slot(position as i32 + state.layer_count + 1);
        masses[position] = aerosols[slot];
        *layer = SnowLayer {
            thickness_m: state.thickness_m[slot],
            temperature_k: state.temperature_k[slot],
            liquid_water_kg_m2: state.liquid_water_kg_m2[slot],
            ice_water_kg_m2: state.ice_water_kg_m2[slot],
        };
    }

    if layer_count == 1 && layers[0].thickness_m > f77(0.03) {
        layer_count = 2;
        halve_layer(&mut layers[0]);
        halve_masses(&mut masses[0]);
        layers[1] = layers[0];
        masses[1] = masses[0];
    }
    let mut stack = LayerStack {
        layers: &mut layers,
        masses: &mut masses,
    };
    split_and_combine(&mut stack, &mut layer_count, 0, f77(0.02), f77(0.07), 1);
    split_and_combine(&mut stack, &mut layer_count, 1, f77(0.05), f77(0.18), 2);
    split_and_combine(&mut stack, &mut layer_count, 2, f77(0.11), f77(0.41), 3);
    if layer_count > 4 && stack.layers[3].thickness_m > f77(0.23) {
        move_excess_to_next(&mut stack, 3, f77(0.23));
    }

    state.layer_count = -(layer_count as i32);
    for (position, layer) in layers.iter().enumerate().take(layer_count) {
        let slot = layer_slot(position as i32 + state.layer_count + 1);
        aerosols[slot] = masses[position];
        state.thickness_m[slot] = layer.thickness_m;
        state.temperature_k[slot] = layer.temperature_k;
        state.liquid_water_kg_m2[slot] = layer.liquid_water_kg_m2;
        state.ice_water_kg_m2[slot] = layer.ice_water_kg_m2;
    }
    rebuild_snow_geometry(state);
    Ok(())
}

/// 分裂时的工作列：水热量与气溶胶质量同步变换。
struct LayerStack<'a> {
    layers: &'a mut [SnowLayer; MAX_SNOW_LAYERS],
    masses: &'a mut [[f64; 8]; MAX_SNOW_LAYERS],
}

fn split_and_combine(
    stack: &mut LayerStack<'_>,
    layer_count: &mut usize,
    position: usize,
    retained_thickness_m: f64,
    split_threshold_m: f64,
    next_position: usize,
) {
    if *layer_count <= position + 1 || stack.layers[position].thickness_m <= retained_thickness_m {
        return;
    }
    move_excess_to_next(stack, position, retained_thickness_m);
    if *layer_count <= next_position + 1
        && stack.layers[next_position].thickness_m > split_threshold_m
    {
        *layer_count += 1;
        halve_layer(&mut stack.layers[next_position]);
        halve_masses(&mut stack.masses[next_position]);
        stack.layers[next_position + 1] = stack.layers[next_position];
        stack.masses[next_position + 1] = stack.masses[next_position];
    }
}

fn move_excess_to_next(stack: &mut LayerStack<'_>, position: usize, retained_thickness_m: f64) {
    let layers = &mut *stack.layers;
    let fraction =
        (layers[position].thickness_m - retained_thickness_m) / layers[position].thickness_m;
    let excess = SnowLayer {
        thickness_m: layers[position].thickness_m - retained_thickness_m,
        temperature_k: layers[position].temperature_k,
        liquid_water_kg_m2: fraction * layers[position].liquid_water_kg_m2,
        ice_water_kg_m2: fraction * layers[position].ice_water_kg_m2,
    };
    let retained_fraction = retained_thickness_m / layers[position].thickness_m;
    layers[position].thickness_m = retained_thickness_m;
    layers[position].liquid_water_kg_m2 *= retained_fraction;
    layers[position].ice_water_kg_m2 *= retained_fraction;
    layers[position + 1] = combine_snow_values(layers[position + 1], excess);
    // `z_mss = propor*mss(pos)`；`mss(pos) = propor'*mss(pos)`；`mss(next) = z_mss + mss(next)`。
    let (retained, next) = stack.masses.split_at_mut(position + 1);
    for (mass, next_mass) in retained[position].iter_mut().zip(next[0].iter_mut()) {
        let moved = fraction * *mass;
        *mass *= retained_fraction;
        *next_mass += moved;
    }
}

/// `mss(j) = mss(j) + mss(l)`，逐物种。
fn add_masses(target: &mut [f64; 8], source: &[f64; 8]) {
    for (mass, added) in target.iter_mut().zip(source) {
        *mass += added;
    }
}

fn halve_masses(masses: &mut [f64; 8]) {
    for mass in masses {
        *mass /= f77(2.0);
    }
}

fn halve_layer(layer: &mut SnowLayer) {
    layer.thickness_m /= f77(2.0);
    layer.liquid_water_kg_m2 /= f77(2.0);
    layer.ice_water_kg_m2 /= f77(2.0);
}

/// Applies MOD_SnowLayersCombineDivide:snowcompaction to the active snow layers.
///
/// Melt flags are ordered from the snow surface to its base, matching the active
/// Fortran range layer_count + 1 through 0.
pub fn compact_snow_layers(
    state: &mut RuntimeSnowColumn,
    time_step_seconds: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    melted: &[bool],
) -> Result<()> {
    validate_compaction(
        state,
        time_step_seconds,
        eastward_wind_m_s,
        northward_wind_m_s,
        melted,
    )?;
    if state.layer_count == 0 {
        return Ok(());
    }

    let mut burden = 0.0;
    let mut pseudo_depth = 0.0;
    let mut mobile = true;
    // `MOD_SnowLayersCombineDivide.F90:153` / `MOD_RainSnowTemp.F90:203`：
    // `forc_wind = sqrt(forc_us**2 + forc_vs**2)`。
    let wind_speed = eastward_wind_m_s
        .contract(eastward_wind_m_s, northward_wind_m_s * northward_wind_m_s)
        .sqrt();
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        let water_mass = state.ice_water_kg_m2[slot] + state.liquid_water_kg_m2[slot];
        let void_fraction = 1.0
            - (state.ice_water_kg_m2[slot] / f77(917.0)
                + state.liquid_water_kg_m2[slot] / f77(1000.0))
                / state.thickness_m[slot];
        if void_fraction <= f77(0.001) || state.ice_water_kg_m2[slot] <= f77(0.1) {
            burden += water_mass;
            mobile = false;
            continue;
        }

        let ice_density = state.ice_water_kg_m2[slot] / state.thickness_m[slot];
        let ice_fraction = state.ice_water_kg_m2[slot] / water_mass;
        let temperature_deficit = FREEZING_K - state.temperature_k[slot];
        let mut destructive = -f77(2.777e-6) * (-f77(0.04) * temperature_deficit).exp();
        if ice_density > f77(100.0) {
            destructive *= (-f77(46.0e-3) * (ice_density - f77(100.0))).exp();
        }
        if state.liquid_water_kg_m2[slot] > f77(0.01) * state.thickness_m[slot] {
            destructive *= f77(2.0);
        }

        let liquid_factor = 1.0
            / (1.0
                + f77(60.0) * state.liquid_water_kg_m2[slot]
                    / (f77(1000.0) * state.thickness_m[slot]));
        let viscosity = liquid_factor
            * f77(4.0)
            * (ice_density / f77(450.0))
            // `exp(0.1*td + c2*bi)`：GIMPLE 是 `exp(.FMA (td, 0.1, bi*0.023))`。
            * temperature_deficit
                .contract(f77(0.1), ice_density * f77(23.0e-3))
                .exp()
            * f77(7.62237e6);
        let overburden = -(burden + water_mass / f77(2.0)) / viscosity;
        let relative = (fortran_layer - (state.layer_count + 1)) as usize;
        // `ddz3 = - 1.0/deltim * max(0.0,(fiold(j) - fi)/fiold(j))`：左结合，先算 `1/deltim`
        // 再乘（GIMPLE `_47 = 1/deltim ; _48 = _47*max(...)`），不是除以 `deltim`。
        let melt = if melted[relative] {
            -(1.0 / time_step_seconds
                * ((state.previous_ice_fraction[slot] - ice_fraction)
                    / state.previous_ice_fraction[slot])
                    .max(0.0))
        } else {
            0.0
        };
        let wind = wind_drift_compaction(
            ice_density,
            wind_speed,
            state.thickness_m[slot],
            &mut pseudo_depth,
            &mut mobile,
        );
        let compaction_rate = destructive + overburden + melt + wind;
        let minimum_thickness =
            state.ice_water_kg_m2[slot] / f77(917.0) + state.liquid_water_kg_m2[slot] / f77(1000.0);
        // `dz*(1.0+pdzdtc*deltim)`：GIMPLE 是 `dz*.FMA (pdzdtc, deltim, 1.0)`。
        state.thickness_m[slot] = (state.thickness_m[slot]
            * compaction_rate.contract(time_step_seconds, 1.0))
        .max(minimum_thickness);
        burden += water_mass;
    }
    Ok(())
}

fn wind_drift_compaction(
    ice_density_kg_m3: f64,
    wind_speed_m_s: f64,
    thickness_m: f64,
    pseudo_depth_m: &mut f64,
    mobile: &mut bool,
) -> f64 {
    if !*mobile {
        return 0.0;
    }
    // `winddriftcompaction`（内联进 `snowcompaction`）的 GIMPLE：
    //   `frho = .FNMA (max(bi,50)-50, 0.0042, 1.25)`
    //   `mo   = .FMA (frho, 0.66, 0.34*(-0.583*gs-0.833*sp+0.833))`（后者编译期折成常数）
    //   `si   = mo + .FNMA (exp(-0.085*wind), 2.868, 1.0)`
    //   `zpseudo += .FMA (dz*0.5, 3.25-si, zpseudo)` 的两次累加也都是 FMA。
    let density_factor = (-(ice_density_kg_m3.max(50.0) - 50.0)).contract(0.0042, 1.25);
    let mobility_index =
        density_factor.contract(0.66, 0.34 * (-0.583 * 0.35e-3 - 0.833 * 1.0 + 0.833));
    let mut driftability = mobility_index + (-(-0.085 * wind_speed_m_s).exp()).contract(2.868, 1.0);
    if driftability <= 0.0 {
        *mobile = false;
        return 0.0;
    }
    driftability = driftability.min(3.25);
    *pseudo_depth_m = (0.5 * thickness_m).contract(3.25 - driftability, *pseudo_depth_m);
    let rate = -((350.0 - ice_density_kg_m3).max(0.0))
        * (driftability * (-*pseudo_depth_m / 0.1).exp() / (48.0 * 3600.0));
    *pseudo_depth_m = (0.5 * thickness_m).contract(3.25 - driftability, *pseudo_depth_m);
    rate
}

fn validate_compaction(
    state: &RuntimeSnowColumn,
    time_step_seconds: f64,
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    melted: &[bool],
) -> Result<()> {
    validate(
        NewSnowInput {
            patch_type: 0,
            time_step_seconds: 1.0,
            ground_temperature_k: FREEZING_K,
            ground_snowfall_kg_m2_s: 0.0,
            new_snow_bulk_density_kg_m3: 1.0,
            precipitation_temperature_k: FREEZING_K,
            variably_saturated_flow: false,
        },
        state,
    )?;
    ensure!(
        time_step_seconds.is_finite()
            && time_step_seconds > 0.0
            && eastward_wind_m_s.is_finite()
            && northward_wind_m_s.is_finite()
            && melted.len() == state.layer_count.unsigned_abs() as usize,
        "snow-compaction inputs do not match the active snow column"
    );
    for fortran_layer in state.layer_count + 1..=0 {
        let slot = layer_slot(fortran_layer);
        ensure!(
            state.thickness_m[slot].is_finite()
                && state.thickness_m[slot] > 0.0
                && state.temperature_k[slot].is_finite()
                && state.liquid_water_kg_m2[slot].is_finite()
                && state.liquid_water_kg_m2[slot] >= 0.0
                && state.ice_water_kg_m2[slot].is_finite()
                && state.ice_water_kg_m2[slot] >= 0.0
                && state.previous_ice_fraction[slot].is_finite()
                && state.previous_ice_fraction[slot] >= 0.0,
            "active snow layer is invalid"
        );
        let relative = (fortran_layer - (state.layer_count + 1)) as usize;
        ensure!(
            !melted[relative] || state.previous_ice_fraction[slot] > 0.0,
            "melting snow layer requires a positive previous ice fraction"
        );
    }
    Ok(())
}

fn validate(input: NewSnowInput, state: &RuntimeSnowColumn) -> Result<()> {
    ensure!(
        input.patch_type >= 0
            && input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.ground_temperature_k.is_finite()
            && input.ground_snowfall_kg_m2_s.is_finite()
            && input.ground_snowfall_kg_m2_s >= 0.0
            && input.new_snow_bulk_density_kg_m3.is_finite()
            && input.new_snow_bulk_density_kg_m3 > 0.0
            && input.precipitation_temperature_k.is_finite(),
        "new-snow inputs are invalid"
    );
    validate_runtime_snow_column(state)
}

pub(crate) fn validate_runtime_snow_column(state: &RuntimeSnowColumn) -> Result<()> {
    ensure!(
        (-5..=0).contains(&state.layer_count)
            && state.interface_depth_m.len() == MAX_SNOW_LAYERS + 1
            && state.node_depth_m.len() == MAX_SNOW_LAYERS
            && state.thickness_m.len() == MAX_SNOW_LAYERS
            && state.temperature_k.len() == MAX_SNOW_LAYERS
            && state.liquid_water_kg_m2.len() == MAX_SNOW_LAYERS
            && state.ice_water_kg_m2.len() == MAX_SNOW_LAYERS
            && state.previous_ice_fraction.len() == MAX_SNOW_LAYERS
            && state.age.is_finite()
            && state.water_equivalent_kg_m2.is_finite()
            && state.water_equivalent_kg_m2 >= 0.0
            && state.depth_m.is_finite()
            && state.depth_m >= 0.0
            && state.ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&state.ground_snow_fraction),
        "new-snow state is invalid"
    );
    Ok(())
}

pub(crate) fn snow_layer_slot(index: i32) -> usize {
    debug_assert!((-4..=0).contains(&index));
    (index + MAX_SNOW_LAYERS as i32 - 1) as usize
}

pub(crate) fn snow_interface_slot(index: i32) -> usize {
    debug_assert!((-5..=0).contains(&index));
    (index + MAX_SNOW_LAYERS as i32) as usize
}

fn layer_slot(index: i32) -> usize {
    snow_layer_slot(index)
}

/// 表层土的新霜：`relocate_soil_frost_ice` 的土壤侧输入/输出。
#[derive(Debug, Clone, Copy)]
pub struct SoilFrostTop {
    pub porosity: f64,
    pub thickness_m: f64,
    pub temperature_k: f64,
    pub ice_water_kg_m2: f64,
}

/// `relocate_soil_frost_ice`（`MOD_NewSnow.F90:130-213`，土壤 patch 每步雪层合并/分裂之后）。
///
/// 表层土冰超过孔隙 `denice*porsl*dz` 的部分（`max(FNMA(denice*porsl, dz, wice), 0)`）挪进雪：
/// 无雪层时加到 `scv`/`snowdp`，雪深够 1 cm 就建一层（与新雪建层同式，温度取土层 1，SNICAR
/// 打开时粒径与气溶胶重置）；有雪层时并进最上层，温度按热容加权
/// `FMA(t_top, hc, (excess*cpice)*t1) / (excess*cpice + hc)`，`hc = FMA(wice, cpice, wliq*cpliq)`。
pub fn relocate_soil_frost_ice(
    state: &mut RuntimeSnowColumn,
    soil: &mut SoilFrostTop,
    snicar: Option<&mut crate::SnicarColumnState>,
) {
    const ICE_DENSITY_KG_M3: f64 = 917.0;
    const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
    const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
    let excess = (-(ICE_DENSITY_KG_M3 * soil.porosity))
        .contract(soil.thickness_m, soil.ice_water_kg_m2)
        .max(0.0);
    if excess <= 0.0 {
        return;
    }
    let added_depth = excess / ICE_DENSITY_KG_M3;
    if state.layer_count == 0 {
        state.water_equivalent_kg_m2 += excess;
        state.depth_m += added_depth;
        if state.depth_m >= f77(0.01) {
            state.layer_count = -1;
            let top = layer_slot(0);
            state.interface_depth_m[interface_slot(0)] = 0.0;
            state.thickness_m[top] = state.depth_m;
            state.node_depth_m[top] = -(state.depth_m * 0.5);
            state.interface_depth_m[interface_slot(-1)] = -state.depth_m;
            state.temperature_k[top] = soil.temperature_k;
            state.ice_water_kg_m2[top] = state.water_equivalent_kg_m2;
            state.liquid_water_kg_m2[top] = 0.0;
            state.previous_ice_fraction[top] = 1.0;
            if let Some(snicar) = snicar {
                snicar.refreezing_kg_m2_s[top] = 0.0;
                snicar.grain_radius_um[top] = 54.526;
                snicar.aerosol_mass_kg_m2[top] = [0.0; crate::SNICAR_AEROSOL_SPECIES];
            }
        }
    } else {
        let top_index = state.layer_count + 1;
        let top = layer_slot(top_index);
        let ice = state.ice_water_kg_m2[top];
        let liquid = state.liquid_water_kg_m2[top];
        let heat_capacity = ice.contract(
            ICE_HEAT_CAPACITY_J_KG_K,
            liquid * WATER_HEAT_CAPACITY_J_KG_K,
        );
        let excess_heat_capacity = excess * ICE_HEAT_CAPACITY_J_KG_K;
        state.temperature_k[top] = state.temperature_k[top]
            .contract(heat_capacity, excess_heat_capacity * soil.temperature_k)
            / (excess_heat_capacity + heat_capacity);
        state.ice_water_kg_m2[top] = ice + excess;
        state.thickness_m[top] += added_depth;
        let interface_top = state.interface_depth_m[interface_slot(top_index)];
        state.node_depth_m[top] = (-state.thickness_m[top]).contract(0.5, interface_top);
        state.interface_depth_m[interface_slot(top_index - 1)] =
            interface_top - state.thickness_m[top];
        state.previous_ice_fraction[top] =
            state.ice_water_kg_m2[top] / (liquid + state.ice_water_kg_m2[top]);
        state.water_equivalent_kg_m2 += excess;
        state.depth_m += added_depth;
    }
    soil.ice_water_kg_m2 -= excess;
}

fn interface_slot(index: i32) -> usize {
    snow_interface_slot(index)
}

#[cfg(test)]
#[path = "snow_tests.rs"]
mod snow_tests;
