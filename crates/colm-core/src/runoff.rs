//! Surface and subsurface runoff from `MOD_Runoff.F90`.
//!
//! These kernels consume and return rates in millimetres water per second.
//! They are deliberately independent of restart and forcing I/O so the Rust
//! time-step driver can feed their output straight into [`crate::soil_water`].

use crate::incomplete_gamma::gratio_fortran;
use crate::LibmPow;
use anyhow::{ensure, Context, Result};

use crate::{soil_vliq_from_psi, SoilHydraulicModel};

/// TOPMODEL's supported saturated-area/baseflow parameterizations (`DEF_TOPMOD_method`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TopmodelMethod {
    /// `DEF_TOPMOD_method = 0`: historic exponential baseflow.
    Exponential,
    /// `DEF_TOPMOD_method = 1`: conductivity-scaled exponential baseflow.
    /// 饱和面积与方法 0 是同一条式子（`MOD_Runoff.F90:90-92`）。
    Hydraulic { mean_topographic_index: f64 },
    /// `DEF_TOPMOD_method = 2`：TWI 服从三参数伽马分布（`alp_twi`/`chi_twi`/`mu_twi`），
    /// 饱和面积比与地下径流都由迭代出的临界地形指数 `eta` 决定（`MOD_Runoff.F90:94-130,218-219`）。
    Gamma {
        /// `topoweti`：平均地形指数，只作迭代初值（不高于 `mu` 时改用 `mu + alpha*chi`）。
        mean_topographic_index: f64,
        alpha: f64,
        chi: f64,
        mu: f64,
    },
}

/// Inputs shared by `SurfaceRunoff_TOPMOD` and its two active Desktop modes.
#[derive(Debug, Clone, Copy)]
pub struct TopmodelSurfaceInput<'a> {
    pub impermeable_porosity: f64,
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub effective_porosity: &'a [f64],
    pub ice_fraction: &'a [f64],
    pub saturated_fraction_max: f64,
    pub saturated_fraction_decay_m_inv: f64,
    pub decay_tuning: f64,
    pub water_table_depth_m: f64,
    pub water_input_mm_s: f64,
    /// `DEF_TOPMOD_method`。方法 0 与 1 的饱和面积相同；方法 2 走伽马分布迭代。
    pub method: TopmodelMethod,
}

/// Partition of TOPMODEL's surface runoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopmodelSurfaceState {
    pub surface_runoff_mm_s: f64,
    pub saturation_excess_runoff_mm_s: f64,
    pub infiltration_excess_runoff_mm_s: f64,
    pub saturated_fraction: f64,
    /// `eta_out`：方法 2 的临界地形指数，交给 `SubsurfaceRunoff_TOPMOD`；方法 0/1 上游不写它。
    pub critical_topographic_index: Option<f64>,
}

/// Inputs to `SubsurfaceRunoff_TOPMOD`.
#[derive(Debug, Clone, Copy)]
pub struct TopmodelSubsurfaceInput<'a> {
    pub method: TopmodelMethod,
    pub layer_thickness_m: &'a [f64],
    /// One more interface than layers, in increasing depth order.
    pub interface_depth_m: &'a [f64],
    pub ice_fraction: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub decay_tuning: f64,
    pub water_table_depth_m: f64,
    /// 方法 2 的 `eta`（[`TopmodelSurfaceState::critical_topographic_index`]）；其余方法不读。
    pub critical_topographic_index: Option<f64>,
}

/// Shared inputs to the XinAnJiang and SimpleVIC surface runoff schemes.
#[derive(Debug, Clone, Copy)]
pub struct StorageRunoffInput<'a> {
    pub layer_thickness_m: &'a [f64],
    pub effective_porosity: &'a [f64],
    pub liquid_volume_fraction: &'a [f64],
    pub water_input_mm_s: f64,
    pub time_step_seconds: f64,
}

/// Surface/subsurface runoff and the saturation diagnostic produced by a
/// storage-distribution scheme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StorageRunoffState {
    pub surface_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub saturated_fraction: f64,
}

/// Inputs to `SubsurfaceRunoff_SimpleVIC`.
#[derive(Debug, Clone, Copy)]
pub struct SimpleVicSubsurfaceInput<'a> {
    pub layer_center_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub porosity: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub residual_water: &'a [f64],
    pub hydraulic_model: &'a [SoilHydraulicModel],
    pub maximum_soil_potential_mm: f64,
    pub water_table_depth_m: f64,
    pub soil_ice_impedance: f64,
    pub baseflow_fraction: f64,
    pub baseflow_threshold: f64,
}

/// Ports `SurfaceRunoff_TOPMOD`（方法 0/1/2，`MOD_Runoff.F90:24-157`）。
pub fn topmodel_surface_runoff(input: TopmodelSurfaceInput<'_>) -> Result<TopmodelSurfaceState> {
    let layers = validate_topmodel_surface(input)?;
    let (saturated_fraction, critical_topographic_index) = match input.method {
        TopmodelMethod::Exponential | TopmodelMethod::Hydraulic { .. } => (
            input.saturated_fraction_max
                * (-input.saturated_fraction_decay_m_inv
                    * input.decay_tuning
                    * input.water_table_depth_m)
                    .exp(),
            None,
        ),
        TopmodelMethod::Gamma {
            mean_topographic_index,
            alpha,
            chi,
            mu,
        } => {
            let (fraction, eta) = gamma_saturated_fraction(
                input.water_table_depth_m,
                input.decay_tuning,
                mean_topographic_index,
                alpha,
                chi,
                mu,
            );
            (fraction, Some(eta))
        }
    };
    let maximum_infiltration = input.saturated_hydraulic_conductivity_mm_s[..layers.min(3)]
        .iter()
        .zip(&input.ice_fraction[..layers.min(3)])
        .map(|(&conductivity, &ice)| 10_f64.lpow(-6.0 * ice) * conductivity)
        .fold(f64::INFINITY, f64::min);
    let maximum_infiltration = if input.effective_porosity[0] < input.impermeable_porosity {
        0.0
    } else {
        maximum_infiltration
    };
    let saturation_excess = saturated_fraction * input.water_input_mm_s.max(0.0);
    let infiltration_excess =
        (1.0 - saturated_fraction) * (input.water_input_mm_s - maximum_infiltration).max(0.0);
    Ok(TopmodelSurfaceState {
        surface_runoff_mm_s: saturation_excess + infiltration_excess,
        saturation_excess_runoff_mm_s: saturation_excess,
        infiltration_excess_runoff_mm_s: infiltration_excess,
        saturated_fraction,
        critical_topographic_index,
    })
}

/// 方法 2 的饱和面积比 `fsat` 与临界地形指数 `eta`（`MOD_Runoff.F90:96-138`，vendor 已改：
/// 初值不低于分布下界、传给 `GRATIO` 的 x 截到 `max(0, (eta-mu)/chi)`、`pgr0 <= 0` 时退出）。
///
/// 照 GIMPLE（`MOD_Runoff.F90.273t.optimized`）：
/// * `chi_twi*alp_twi*pgr1` 是 `(alp*chi)*pgr1`，先算出来，`gfun` 与更新式共用；
/// * `gfun = FMS(eta-mu, pgr0, 上面那个乘积) / DECAY - zwt` —— 乘积被吸收进 fms；
/// * 更新式 `eta = FMA(DECAY, zwt, 上面那个乘积) / pgr0 + mu`；
/// * 传给 `GRATIO` 的 x 由未截断的 `eta-mu` 除 `chi` 再截 0，`gfun` 里用的是未截断的差。
///
/// `pgr0`/`pgr1`/`qgr` 是未初始化的局部量：`GRATIO` 出错（`ANS = 2`）时不写 `QANS`，
/// `fsat = qgr` 便是上一次成功调用留下的值 —— 用 [`gratio_fortran`] 保留这一语义。
/// 循环至少跑一次，第一次调用的 `a = alp+1 >= 1`，所以只要 `alp >= -1`，`qgr` 总被写过；
/// 这里的 NaN 初值只在 `alp < -1` 这种上游也没有定义的输入里露出来。
///
/// 上游不收敛时 `write(*,*) 'Fail to converge in TOPModel: ...'`，结果照用；这里不打印。
fn gamma_saturated_fraction(
    water_table_depth_m: f64,
    decay_tuning: f64,
    mean_topographic_index: f64,
    alpha: f64,
    chi: f64,
    mu: f64,
) -> (f64, f64) {
    if water_table_depth_m <= 0.0 {
        return (1.0, mu);
    }
    // 初值（upstream-bugs 第 58 条，vendor 已修）：`topoweti <= mu_twi` 时改从分布均值
    // `mu + alp*chi` 起步 —— 否则第一轮 x 截到 0、`pgr0 = 0` 立即退出，整块当全饱和。
    // GIMPLE：`.FMA (alp_twi, chi_twi, mu_twi)`。
    let mut eta = if mean_topographic_index > mu {
        mean_topographic_index
    } else {
        alpha.mul_add(chi, mu)
    };
    let (mut pgr0, mut pgr1, mut qgr) = (0.0, 0.0, f64::NAN);
    for _ in 0..20 {
        let excess = eta - mu;
        let x = (excess / chi).max(0.0);
        gratio_fortran(alpha + 1.0, x, &mut pgr1, &mut qgr, 0);
        gratio_fortran(alpha, x, &mut pgr0, &mut qgr, 0);
        let spread = alpha * chi * pgr1;
        let gfun = excess.mul_add(pgr0, -spread) / decay_tuning - water_table_depth_m;
        // x 截到 0（eta <= mu）时 pgr0 = 0，更新式会除以 0。
        if pgr0 <= 0.0 {
            break;
        }
        if gfun.abs() > 1.0e-6 {
            eta = decay_tuning.mul_add(water_table_depth_m, spread) / pgr0 + mu;
        } else {
            break;
        }
    }
    gratio_fortran(alpha, ((eta - mu) / chi).max(0.0), &mut pgr0, &mut qgr, 0);
    (qgr, eta)
}

/// Ports `SubsurfaceRunoff_TOPMOD`（方法 0/1/2，`MOD_Runoff.F90:160-224`）。
///
/// 上游按 `present(hksati) .and. present(topoweti/eta)` 选分支，缺参数时落进方法 0 的式子。
/// 现在 `WATER_VSF` 与 `groundwater`（`WATER_2014`）都带齐这三个（upstream-bugs 第 57 条，
/// vendor 已修），所以调用方直接传 `DEF_TOPMOD_method` 对应的方法。
pub fn topmodel_subsurface_runoff(input: TopmodelSubsurfaceInput<'_>) -> Result<f64> {
    let layers = validate_topmodel_subsurface(input)?;
    let start = water_table_layer(input.water_table_depth_m, input.interface_depth_m);
    // 上游先把 `dzmm(j) = dz_soisno(j)*1000.` 整列算出来，再
    // `dzsum = dzsum + dzmm(j)`、`icefracsum = icefracsum + icefrac(j)*dzmm(j)`
    // （`MOD_Runoff.F90:186-204`）。两处都不能省：
    // * **乘 1000 要先做**：`Σ round(dz*1000)` 与 `Σ dz` 不是一个数。实测 20000 组
    //   随机输入，用未缩放的 `dz` 求和与上游逐位相同 **0/20000**，先缩放 **20000/20000**；
    // * `icefracsum` 那个乘积进 fma（不收缩 15370/20000，收缩 20000/20000）。
    let mut thickness_mm = 0.0;
    let mut ice_sum = 0.0;
    for layer in start..layers {
        let layer_mm = input.layer_thickness_m[layer] * 1000.0;
        thickness_mm += layer_mm;
        ice_sum = input.ice_fraction[layer].mul_add(layer_mm, ice_sum);
    }
    let mean_ice = ice_sum / thickness_mm;
    let exp_minus_three = (-3.0_f64).exp();
    let ice_runoff_fraction =
        ((-3.0 * (1.0 - mean_ice)).exp() - exp_minus_three).max(0.0) / (1.0 - exp_minus_three);
    let ice_impedance = (1.0 - ice_runoff_fraction).max(0.0);
    // **`imped` 在乘法链的链首**：上游是 `imped*5.5e-3*exp(-2.5*zwt)`
    // （`((imped*5.5e-3)*exp)`），不能写成 `imped*(5.5e-3*exp(…))` ——
    // 实测 12715/20000 对 20000/20000。
    match input.method {
        TopmodelMethod::Exponential => {
            Ok(ice_impedance * 5.5e-3 * (-2.5 * input.water_table_depth_m).exp())
        }
        // GIMPLE（`:217`）：`((((imped*3e4)*sum)/nl)/DECAY)*exp(-topoweti))*exp(-DECAY*zwt)`。
        // `sum(hksati)/nl_soil` **不是**先算出的均值：除以层数排在乘 `imped*3e4` 之后。
        TopmodelMethod::Hydraulic {
            mean_topographic_index,
        } => Ok(ice_impedance * 3.0e4 * conductivity_sum(input)
            / layers as f64
            / input.decay_tuning
            * (-mean_topographic_index).exp()
            * (-input.decay_tuning * input.water_table_depth_m).exp()),
        // `:219`：`(((imped*3e3)*sum)/nl)/DECAY*exp(-eta)`。
        TopmodelMethod::Gamma { .. } => {
            let eta = input
                .critical_topographic_index
                .context("TOPMODEL method 2 baseflow needs eta from SurfaceRunoff_TOPMOD")?;
            Ok(
                ice_impedance * 3.0e3 * conductivity_sum(input)
                    / layers as f64
                    / input.decay_tuning
                    * (-eta).exp(),
            )
        }
    }
}

/// `sum(hksati(1:nl_soil))`：从 0 起顺序累加（GIMPLE `val = h + val`）。
fn conductivity_sum(input: TopmodelSubsurfaceInput<'_>) -> f64 {
    input
        .saturated_hydraulic_conductivity_mm_s
        .iter()
        .fold(0.0, |sum, &conductivity| conductivity + sum)
}

/// Ports `Runoff_XinAnJiang`（`MOD_Runoff.F90:224-282`）。
///
/// **不能借用 `storage_distribution_runoff`。** 上游这一支写的是另一套代数等价的
/// 形式（`wtmp`/`infil`），而浮点不认代数等价：实测 20000 组随机输入里，
/// 借用通用式的写法与上游 **0%** 逐位相同，照抄上游这一支才是 **100%**。
/// 另外这里**没有** `[0, watin]` 的钳位（只有 `infil = min(infil, watin)`），
/// 与 `Runoff_SimpleVIC` 不同。
pub fn xinanjiang_runoff(
    input: StorageRunoffInput<'_>,
    elevation_standard_deviation_m: f64,
) -> Result<StorageRunoffState> {
    ensure!(
        elevation_standard_deviation_m.is_finite(),
        "elevation standard deviation must be finite"
    );
    let (water, capacity) = storage(input)?;
    let shape = ((elevation_standard_deviation_m - 100.0)
        / (elevation_standard_deviation_m + 1000.0))
        .clamp(0.01, 0.5);
    let saturated_fraction = 1.0 - (1.0 - water / capacity).lpow(shape / (1.0 + shape));
    let input_depth = input.water_input_mm_s * input.time_step_seconds / 1000.0;
    if input_depth <= 0.0 {
        return Ok(StorageRunoffState {
            surface_runoff_mm_s: 0.0,
            subsurface_runoff_mm_s: 0.0,
            saturated_fraction,
        });
    }
    let shape_plus_one = shape + 1.0;
    // `wtmp = (1-w_int/wsat_int)**(1/(btopo+1)) - watin/((btopo+1)*wsat_int)`
    let wtmp = (1.0 - water / capacity).lpow(1.0 / shape_plus_one)
        - input_depth / (shape_plus_one * capacity);
    // `infil = wsat_int - w_int - wsat_int*max(0,wtmp)**(btopo+1)`：
    // 乘积被吸收成 `FNMS(ws, pow, ws-w)` —— 就是上面那 0% → 100% 的那一处。
    let infiltration = (-capacity)
        .mul_add(wtmp.max(0.0).lpow(shape_plus_one), capacity - water)
        .min(input_depth);
    Ok(StorageRunoffState {
        surface_runoff_mm_s: (input_depth - infiltration) * 1000.0 / input.time_step_seconds,
        subsurface_runoff_mm_s: 0.0,
        saturated_fraction,
    })
}

/// Ports `Runoff_SimpleVIC`.
pub fn simple_vic_runoff(input: StorageRunoffInput<'_>, bvic: f64) -> Result<StorageRunoffState> {
    ensure!(
        bvic.is_finite() && bvic > 0.0,
        "BVIC must be positive and finite"
    );
    let (water, capacity) = storage(input)?;
    storage_distribution_runoff(input, water, capacity, bvic)
}

/// Ports `SubsurfaceRunoff_SimpleVIC`.
pub fn simple_vic_subsurface_runoff(input: SimpleVicSubsurfaceInput<'_>) -> Result<f64> {
    validate_simple_vic_subsurface(input)?;
    let mut wilting_water = 0.0;
    let mut maximum_water = 0.0;
    let mut liquid_water = 0.0;
    let mut maximum_flow: f64 = 0.0;
    for layer in 7..9 {
        let ice_volume = (input.ice_water_kg_m2[layer] / (input.layer_thickness_m[layer] * 917.0))
            .clamp(0.0, input.porosity[layer]);
        let effective_porosity = input.porosity[layer] - ice_volume;
        wilting_water += input.layer_thickness_m[layer]
            * 1000.0
            * soil_vliq_from_psi(
                input.maximum_soil_potential_mm,
                effective_porosity,
                input.residual_water[layer],
                input.saturated_potential_mm[layer],
                input.hydraulic_model[layer],
            );
        maximum_water += effective_porosity * input.layer_thickness_m[layer] * 1000.0;
        let potential = input.saturated_potential_mm[layer]
            - ((input.water_table_depth_m - input.layer_center_depth_m[layer]) * 1000.0).max(0.0);
        liquid_water += input.layer_thickness_m[layer]
            * 1000.0
            * soil_vliq_from_psi(
                potential,
                effective_porosity,
                input.residual_water[layer],
                input.saturated_potential_mm[layer],
                input.hydraulic_model[layer],
            );
        let ice_fraction = ice_volume / input.porosity[layer];
        let conductivity = 10_f64.lpow(-input.soil_ice_impedance * ice_fraction)
            * input.saturated_hydraulic_conductivity_mm_s[layer];
        maximum_flow = maximum_flow.max(conductivity);
    }
    let relative_storage =
        ((liquid_water - wilting_water) / (maximum_water - wilting_water)).clamp(0.0, 1.0);
    let runoff = if relative_storage <= input.baseflow_threshold {
        maximum_flow * input.baseflow_fraction * (relative_storage / input.baseflow_threshold)
    } else {
        maximum_flow * input.baseflow_fraction * (relative_storage / input.baseflow_threshold)
            + maximum_flow
                * (1.0 - input.baseflow_fraction / input.baseflow_threshold)
                * ((relative_storage - input.baseflow_threshold) / (1.0 - input.baseflow_threshold))
                    .powi(2)
    };
    Ok(runoff)
}

/// `Runoff_SimpleVIC` 与 `Runoff_XinAnJiang` **不是**同一条式子，所以这个只能给
/// SimpleVIC 用（见 [`xinanjiang_runoff`] 的说明）。`bvic` 是上游的 `BVIC` 本身，
/// 不再从 `exponent` 反解 —— `exponent/(1-exponent)` 只有 43% 的输入能原样还原
/// `BVIC`，反解一次就白丢 1 ULP。
fn storage_distribution_runoff(
    input: StorageRunoffInput<'_>,
    water: f64,
    capacity: f64,
    bvic: f64,
) -> Result<StorageRunoffState> {
    let saturated_fraction = 1.0 - (1.0 - water / capacity).lpow(bvic / (1.0 + bvic));
    let input_depth = input.water_input_mm_s * input.time_step_seconds / 1000.0;
    if input_depth <= 0.0 {
        return Ok(StorageRunoffState {
            surface_runoff_mm_s: 0.0,
            subsurface_runoff_mm_s: 0.0,
            saturated_fraction,
        });
    }
    let maximum_depth = (1.0 + bvic) * capacity;
    // `:342/:346` 的 GIMPLE 是 `_30 = .FMA(_29, waterdepthmax, watin)` ——
    // `WaterDepthInit + watin` 这条加法把 `WaterDepthMax*(1-(1-SSF)**(1/BVIC))` 那个乘积
    // **收进 FMA**（一次舍入）；`WaterDepthInit` 单独并不存在（CSE 后只有这一个值）。
    // 这一个 ULP 同时进 `ELSEIF ((WaterDepthInit+watin) > WaterDepthMax)` 的**分支判定**
    // 与 `InfilVarTmp` 的分子 ⇒ 湿季（`watin > 0`、土壤接近饱和）能把整支翻掉，
    // 而干季根本走不到这里（`watin <= 0` 直接返回）。
    let depth_factor = 1.0 - (1.0 - saturated_fraction).lpow(1.0 / bvic);
    let depth_with_input = depth_factor.mul_add(maximum_depth, input_depth);
    let surface_depth = if depth_with_input > maximum_depth {
        input_depth - capacity + water
    } else {
        let remaining = 1.0 - depth_with_input / maximum_depth;
        // `RunoffSurface = watin - wsat_int + w_int + wsat_int*(InfilVarTmp**(1+BVIC))`：
        // 最后一个乘积被吸收（`FMA`）。实测 18185 组随机输入：不收缩 2/18185，
        // 收缩后 **18185/18185**。
        capacity.mul_add(remaining.lpow(1.0 + bvic), input_depth - capacity + water)
    }
    .clamp(0.0, input_depth);
    Ok(StorageRunoffState {
        surface_runoff_mm_s: surface_depth * 1000.0 / input.time_step_seconds,
        subsurface_runoff_mm_s: 0.0,
        saturated_fraction,
    })
}

/// `sum(vol_liq(1:6)*dz(1:6))` / `sum(eff_porosity(1:6)*dz(1:6))`（`MOD_Runoff.F90:250-251`
/// 等）。**gfortran 把每个乘积都收进累加器**（`fma(v, dz, acc)`）：实测 20000 组
/// 随机输入，逐步 `acc += v*dz` 只有 14312/20000 与上游逐位相同，
/// `fma` 累加是 **20000/20000**。
///
/// 这条在 `SimpleVIC`（三个黄金窗口默认的水文方案）里每步都跑到 ——
/// `w_int`/`wsat_int` 差 1 ULP，`frcsat`(`f_frcsat`) 与 `rsur`(`f_rsur`) 就跟着差。
fn storage(input: StorageRunoffInput<'_>) -> Result<(f64, f64)> {
    validate_storage(input)?;
    let mut water = 0.0;
    let mut capacity = 0.0;
    for layer in 0..6 {
        water = input.liquid_volume_fraction[layer].mul_add(input.layer_thickness_m[layer], water);
        capacity =
            input.effective_porosity[layer].mul_add(input.layer_thickness_m[layer], capacity);
    }
    ensure!(
        capacity > 0.0,
        "storage runoff needs positive water capacity"
    );
    Ok((water.clamp(0.0, capacity), capacity))
}

fn water_table_layer(water_table_depth_m: f64, interface_depth_m: &[f64]) -> usize {
    let layers = interface_depth_m.len() - 1;
    for (layer, &depth) in interface_depth_m.iter().enumerate().skip(1) {
        if water_table_depth_m <= depth {
            // Fortran stores the layer immediately above this interface in
            // `jwt`, then integrates from max(jwt, 1).  Convert that one-based
            // lower bound directly to Rust's zero-based index.
            return layer.saturating_sub(2);
        }
    }
    layers - 1
}

fn validate_topmodel_surface(input: TopmodelSurfaceInput<'_>) -> Result<usize> {
    let layers = input.saturated_hydraulic_conductivity_mm_s.len();
    ensure!(layers > 0, "TOPMODEL needs at least one soil layer");
    ensure!(
        input.effective_porosity.len() == layers && input.ice_fraction.len() == layers,
        "TOPMODEL surface fields must have equal lengths"
    );
    ensure!(
        input
            .saturated_hydraulic_conductivity_mm_s
            .iter()
            .all(|value| *value >= 0.0 && value.is_finite())
            && input
                .effective_porosity
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input
                .ice_fraction
                .iter()
                .all(|value| (0.0..=1.0).contains(value)),
        "TOPMODEL surface layers are invalid"
    );
    ensure!(
        [
            input.impermeable_porosity,
            input.saturated_fraction_max,
            input.saturated_fraction_decay_m_inv,
            input.decay_tuning,
            input.water_table_depth_m,
            input.water_input_mm_s,
        ]
        .iter()
        .all(|value| value.is_finite()),
        "TOPMODEL surface scalars must be finite"
    );
    if let TopmodelMethod::Gamma {
        mean_topographic_index,
        alpha,
        chi,
        mu,
    } = input.method
    {
        ensure!(
            [mean_topographic_index, alpha, chi, mu]
                .iter()
                .all(|value| value.is_finite()),
            "TOPMODEL method 2 needs finite topoweti/alp_twi/chi_twi/mu_twi"
        );
        // 上游 `(eta-mu_twi)/chi_twi` 在 `-ffpe-trap=zero` 下除零即中止。`chi_twi = 0` 多半是
        // 拿方法 0/1 建的常数重启（那时四个 TWI 参数都置 0，`MOD_Initialize.F90`）去跑方法 2。
        // `zwt <= 0` 那一支不做这次除法，上游不会中止。
        ensure!(
            chi != 0.0 || input.water_table_depth_m <= 0.0,
            "TOPMODEL method 2 needs chi_twi != 0 (was the constant restart built for DEF_TOPMOD_method = 2?)"
        );
    }
    Ok(layers)
}

fn validate_topmodel_subsurface(input: TopmodelSubsurfaceInput<'_>) -> Result<usize> {
    let layers = input.layer_thickness_m.len();
    ensure!(layers > 0, "TOPMODEL needs at least one soil layer");
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input.ice_fraction.len() == layers
            && input.saturated_hydraulic_conductivity_mm_s.len() == layers,
        "TOPMODEL subsurface fields must have equal lengths"
    );
    ensure!(
        input
            .layer_thickness_m
            .iter()
            .all(|value| *value > 0.0 && value.is_finite())
            && input
                .interface_depth_m
                .windows(2)
                .all(|pair| { pair[0].is_finite() && pair[1].is_finite() && pair[1] > pair[0] })
            && input
                .ice_fraction
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input.decay_tuning.is_finite()
            && input.decay_tuning > 0.0
            && input.water_table_depth_m.is_finite(),
        "TOPMODEL subsurface inputs are invalid"
    );
    if let TopmodelMethod::Hydraulic {
        mean_topographic_index,
    } = input.method
    {
        ensure!(
            mean_topographic_index.is_finite(),
            "topographic index must be finite"
        );
    }
    if let TopmodelMethod::Gamma { .. } = input.method {
        ensure!(
            input
                .critical_topographic_index
                .is_some_and(|eta| eta.is_finite()),
            "TOPMODEL method 2 baseflow needs a finite eta"
        );
    }
    Ok(layers)
}

fn validate_storage(input: StorageRunoffInput<'_>) -> Result<()> {
    ensure!(
        input.layer_thickness_m.len() >= 6
            && input.effective_porosity.len() == input.layer_thickness_m.len()
            && input.liquid_volume_fraction.len() == input.layer_thickness_m.len(),
        "storage runoff needs six equally sized soil layers"
    );
    ensure!(
        input
            .layer_thickness_m
            .iter()
            .all(|value| *value > 0.0 && value.is_finite())
            && input
                .effective_porosity
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input
                .liquid_volume_fraction
                .iter()
                .all(|value| *value >= 0.0 && value.is_finite())
            && input.water_input_mm_s.is_finite()
            && input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0,
        "storage runoff inputs are invalid"
    );
    Ok(())
}

fn validate_simple_vic_subsurface(input: SimpleVicSubsurfaceInput<'_>) -> Result<()> {
    let layers = input.layer_center_depth_m.len();
    ensure!(
        layers >= 9,
        "SimpleVIC baseflow needs soil layers eight and nine"
    );
    for values in [
        input.layer_thickness_m,
        input.ice_water_kg_m2,
        input.porosity,
        input.saturated_potential_mm,
        input.saturated_hydraulic_conductivity_mm_s,
        input.residual_water,
    ] {
        ensure!(
            values.len() == layers,
            "SimpleVIC fields must have equal lengths"
        );
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "SimpleVIC fields must be finite"
        );
    }
    ensure!(
        input.hydraulic_model.len() == layers
            && input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.porosity.iter().all(|value| *value > 0.0)
            && input
                .saturated_potential_mm
                .iter()
                .all(|value| *value < 0.0)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0)
            && input
                .residual_water
                .iter()
                .zip(input.porosity)
                .all(|(&residual, &porosity)| residual >= 0.0 && residual <= porosity)
            && input.maximum_soil_potential_mm.is_finite()
            && input.water_table_depth_m.is_finite()
            && input.soil_ice_impedance.is_finite()
            && input.baseflow_fraction.is_finite()
            && input.baseflow_threshold.is_finite()
            && input.baseflow_threshold > 0.0
            && input.baseflow_threshold < 1.0,
        "SimpleVIC baseflow inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "runoff_tests.rs"]
mod runoff_tests;
