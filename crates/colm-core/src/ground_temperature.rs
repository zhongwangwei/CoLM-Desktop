//! Ground-temperature conduction and phase change from `MOD_GroundTemperature.F90`.
//!
//! The state is packed top-to-bottom: leading snow layers, followed by soil.
//! This keeps the solver independent of Fortran's negative snow indices while
//! directly sharing `soil_thermal_properties`, `solve_tridiagonal`, and
//! `phase_change` with the rest of the Rust model.

use anyhow::{ensure, Result};

use crate::{
    phase_change, soil_thermal_properties, solve_tridiagonal, PhaseChangeInput, PhaseChangeState,
    SoilHydraulicModel, SoilThermalInput, ThermalConductivityScheme,
};

const WATER_DENSITY_KG_M3: f64 = 1000.0;
const ICE_DENSITY_KG_M3: f64 = 917.0;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const AIR_THERMAL_CONDUCTIVITY_W_M_K: f64 = 0.023;
const ICE_THERMAL_CONDUCTIVITY_W_M_K: f64 = 2.290;
const STEFAN_BOLTZMANN_W_M2_K4: f64 = 5.67e-8;

/// `MOD_Thermal.F90:485-486` 的地表比辐射率 `emg`。
///
/// **它是逐步骤推出的量，不是算例参数。** 上游每次进 THERMAL 先重置为土壤值，
/// 只有"雪水当量大于零"或"patch 是湖"时才抬到雪值：
///
/// ```fortran
/// emg = 0.96
/// IF (scv>0. .or. patchtype==3) emg = 0.97
/// ```
///
/// 把它当常量装配的后果是**雪完全融化之后仍然按雪面辐射**，而那一支算例里的
/// 能量收支看上去依旧闭合 —— 单点测试看不出来。
///
/// `snow_water_equivalent_kg_m2` 就是上游的 `scv`（mm 与 kg m⁻² 同值）。
pub fn ground_emissivity(snow_water_equivalent_kg_m2: f64, patch_type: i32) -> f64 {
    if snow_water_equivalent_kg_m2 > 0.0 || patch_type == 3 {
        0.97
    } else {
        0.96
    }
}

/// Inputs to one `MOD_GroundTemperature:GroundTemperature` update.
#[derive(Debug, Clone, Copy)]
pub struct GroundTemperatureInput<'a> {
    pub patch_type: i32,
    pub is_dry_lake: bool,
    pub time_step_seconds: f64,
    pub surface_temperature_factor: f64,
    pub crank_nicolson_factor: f64,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    /// One static soil-thermal input per soil layer. Its temperature and water
    /// fractions are replaced from the packed dynamic state for this update.
    pub soil_thermal_inputs: &'a [SoilThermalInput],
    pub soil_porosity: &'a [f64],
    pub soil_residual_water: &'a [f64],
    pub soil_suction_mm: &'a [f64],
    pub soil_hydraulic_model: &'a [SoilHydraulicModel],
    /// Leading snow layers followed by soil layers.
    pub snow_layers: usize,
    pub layer_thickness_m: &'a [f64],
    pub node_depth_m: &'a [f64],
    /// One more value than `node_depth_m`, from the top to the bottom interface.
    pub interface_depth_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub snow_cover_fraction: f64,
    pub use_split_soil_snow: bool,
    /// `Some` selects the SNICAR branch and supplies absorption for each packed
    /// layer; `None` selects the standard branch.
    pub snow_layer_absorption_w_m2: Option<&'a [f64]>,
    pub absorbed_ground_shortwave_w_m2: f64,
    pub absorbed_soil_shortwave_w_m2: f64,
    pub absorbed_snow_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    pub sensible_ground_w_m2: f64,
    pub sensible_soil_w_m2: f64,
    pub sensible_snow_w_m2: f64,
    pub evaporation_ground_kg_m2_s: f64,
    pub evaporation_soil_kg_m2_s: f64,
    pub evaporation_snow_kg_m2_s: f64,
    pub ground_flux_temperature_derivative_w_m2_k: f64,
    pub vaporization_heat_j_kg: f64,
    pub ground_emissivity: f64,
    pub rain_on_ground_kg_m2_s: f64,
    pub snow_on_ground_kg_m2_s: f64,
    pub precipitation_temperature_k: f64,
    pub ground_temperature_k: f64,
    pub soil_surface_temperature_k: f64,
    pub snow_surface_temperature_k: f64,
    pub supercool_water: bool,
}

/// Dynamic state and diagnostic fields returned by [`ground_temperature`].
#[derive(Debug, Clone, PartialEq)]
pub struct GroundTemperatureState {
    pub temperature_k: Vec<f64>,
    /// 本步**开始前**的整列温度，与 `temperature_k` 同形状同顺序。
    ///
    /// 上游 `MOD_Thermal.F90` 用 `t_soisno_bef` 与 `tinc = t - t_bef` 推三个诊断量
    /// （`:1337` 的 `fgrnd`、`:1354` 的 `olrg`、`:1362` 的 `emis`/`trad`），它们都进
    /// history。没有这两个量就没法把地表能量收支的每一项对上，所以状态把它留着。
    pub previous_temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    pub snow_water_equivalent_kg_m2: f64,
    pub snow_depth_m: f64,
    pub snow_melt_rate_kg_m2_s: f64,
    pub latent_heat_flux_w_m2: f64,
    pub phase_flag: Vec<i32>,
    pub thaw_mass_kg_m2: Vec<f64>,
    pub freeze_mass_kg_m2: Vec<f64>,
    /// Snow-only positive freezing rate, in top-to-bottom snow-layer order.
    pub snow_freezing_rate_kg_m2_s: Vec<f64>,
    pub layer_factor_seconds_per_j_m2_k: Vec<f64>,
    pub interface_conductivity_w_m_k: Vec<f64>,
}

/// Ports `MOD_GroundTemperature:GroundTemperature` as a pure packed-column
/// update. The result may be supplied directly to the shared hydrology,
/// snow, and restart paths without a second phase-change implementation.
pub fn ground_temperature(input: GroundTemperatureInput<'_>) -> Result<GroundTemperatureState> {
    let layers = validate(input)?;
    let soil_offset = input.snow_layers;
    let use_snicar = input.snow_layer_absorption_w_m2.is_some();
    let previous_temperature = input.temperature_k.to_vec();
    let snow_ice_before = input.ice_water_kg_m2[..input.snow_layers].to_vec();

    let (heat_capacity, mut conductivity) = layer_thermal_properties(input)?;
    let mut layer_capacity = heat_capacity;
    if input.snow_layers == 0 && input.snow_water_equivalent_kg_m2 > 0.0 {
        // `cv(1) = cv(1) + cpice*scv`（`MOD_GroundTemperature.F90:202`）：GIMPLE 是
        // `.FMA (scv, cpice, cv(1))`。平铺写法让 `fact(1)` 差 1 ULP —— AT-Neu 1 月第 89 步
        // 薄雪融化时 `scv`/`snowdp`/`qinfl` 由此偏开（第 402 轮探针）。
        layer_capacity[0] = input
            .snow_water_equivalent_kg_m2
            .mul_add(ICE_HEAT_CAPACITY_J_KG_K, layer_capacity[0]);
    }
    ensure!(
        layer_capacity
            .iter()
            .all(|value| *value > 0.0 && value.is_finite()),
        "ground-temperature layer heat capacities must be positive"
    );

    let mut interface_conductivity = vec![0.0; layers];
    for layer in 0..layers - 1 {
        let interface = layer + 1;
        interface_conductivity[layer] = if layer + 1 == input.snow_layers
            && input.node_depth_m[layer + 1] - input.interface_depth_m[interface]
                < input.interface_depth_m[interface] - input.node_depth_m[layer]
        {
            let harmonic = 2.0 * conductivity[layer] * conductivity[layer + 1]
                / (conductivity[layer] + conductivity[layer + 1]);
            harmonic.max(0.5 * conductivity[layer + 1])
        } else {
            // 分母**有一个乘积被吸收**。`MOD_GroundTemperature.F90:243-244` 是
            // `thk(i)*(z(i+1)-zi(i)) + thk(i+1)*(zi(i)-z(i))`；出货内核
            // （`gt.o` 的 `groundtemperature`，`0xc94-0xcb4`）编出来是
            //   `d18 = (zi(i)-z(i))*thk(i+1)`（`fmul`）
            //   `d18 = fmadd(thk(i), z(i+1)-zi(i), d18)`   ← **第一项被融合**
            // 原先是平铺。逐元素位型探针（`/tmp/gf/gtcoef_probe.sh`）在干窗第 0 步
            // 抓到 `tk(3)/tk(5)/tk(7)` 各差 1 ULP，`at`/`ct` 里随之偏 ——
            // `bt`/`rt` 全同，所以问题只在这一条分母上。
            let denominator = conductivity[layer].mul_add(
                input.node_depth_m[layer + 1] - input.interface_depth_m[interface],
                conductivity[layer + 1]
                    * (input.interface_depth_m[interface] - input.node_depth_m[layer]),
            );
            conductivity[layer]
                * conductivity[layer + 1]
                * (input.node_depth_m[layer + 1] - input.node_depth_m[layer])
                / denominator
        };
    }
    conductivity[layers - 1] = 0.0;

    let (surface_heat_flux, soil_heat_flux, snow_heat_flux, flux_derivative) =
        surface_fluxes(input, use_snicar)?;
    let mut factor = vec![0.0; layers];
    // `fact(lb) = deltim/cv*dz_soisno/(0.5*(z(j)-zi(j-1)+capr*(z(j+1)-zi(j-1))))`
    // （`MOD_GroundTemperature.F90:311-312`）：括号里的 `capr*(…)` 被吸收
    // （GIMPLE：`FMA(capr, z(j+1)-zi(j-1), z(j)-zi(j-1))`）。
    factor[0] = input.time_step_seconds / layer_capacity[0] * input.layer_thickness_m[0]
        / (0.5
            * input.surface_temperature_factor.mul_add(
                input.node_depth_m[1] - input.interface_depth_m[0],
                input.node_depth_m[0] - input.interface_depth_m[0],
            ));
    for layer in 1..layers {
        factor[layer] = input.time_step_seconds / layer_capacity[layer];
    }
    ensure!(
        factor.iter().all(|value| *value > 0.0 && value.is_finite()),
        "ground-temperature layer factors must be positive"
    );

    let before_flux = interface_fluxes(
        &interface_conductivity,
        input.node_depth_m,
        input.temperature_k,
    )?;
    let (subdiagonal, diagonal, superdiagonal, rhs) = temperature_system(
        input,
        &factor,
        &interface_conductivity,
        &before_flux,
        surface_heat_flux,
        soil_heat_flux,
        snow_heat_flux,
        flux_derivative,
        use_snicar,
    );
    let solved_temperature = solve_tridiagonal(&subdiagonal, &diagonal, &superdiagonal, &rhs)
        .map_err(anyhow::Error::msg)?;
    let after_flux = interface_fluxes(
        &interface_conductivity,
        input.node_depth_m,
        &solved_temperature,
    )?;
    let residual_heat_flux =
        residual_heat_fluxes(input.crank_nicolson_factor, &before_flux, &after_flux);
    let phase = phase_change(PhaseChangeInput {
        patch_type: input.patch_type,
        is_dry_lake: input.is_dry_lake,
        time_step_seconds: input.time_step_seconds,
        fact_seconds_per_j_m2_k: &factor,
        residual_heat_flux_w_m2: &residual_heat_flux,
        snow_layer_absorption_w_m2: input.snow_layer_absorption_w_m2,
        surface_heat_flux_w_m2: surface_heat_flux,
        soil_heat_flux_w_m2: soil_heat_flux,
        snow_heat_flux_w_m2: snow_heat_flux,
        snow_cover_fraction: input.snow_cover_fraction,
        surface_heat_flux_temperature_derivative_w_m2_k: flux_derivative,
        previous_temperature_k: &previous_temperature,
        temperature_k: &solved_temperature,
        liquid_water_kg_m2: input.liquid_water_kg_m2,
        ice_water_kg_m2: input.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: input.snow_water_equivalent_kg_m2,
        snow_depth_m: input.snow_depth_m,
        snow_layers: input.snow_layers,
        split_soil_snow: input.use_split_soil_snow,
        supercool_water: input.supercool_water,
        soil_layer_thickness_m: &input.layer_thickness_m[soil_offset..],
        soil_porosity: input.soil_porosity,
        soil_residual_water: input.soil_residual_water,
        soil_suction_mm: input.soil_suction_mm,
        soil_hydraulic_model: input.soil_hydraulic_model,
    })?;
    Ok(state_from_phase(
        phase,
        snow_ice_before,
        input.time_step_seconds,
        previous_temperature,
        factor,
        interface_conductivity,
    ))
}

fn layer_thermal_properties(input: GroundTemperatureInput<'_>) -> Result<(Vec<f64>, Vec<f64>)> {
    let layers = input.temperature_k.len();
    let mut heat_capacity = vec![0.0; layers];
    let mut conductivity = vec![0.0; layers];
    for soil in 0..input.soil_thermal_inputs.len() {
        let layer = input.snow_layers + soil;
        let mut thermal = input.soil_thermal_inputs[soil];
        thermal.temperature_k = input.temperature_k[layer];
        thermal.liquid_volume_fraction = input.liquid_water_kg_m2[layer]
            / (input.layer_thickness_m[layer] * WATER_DENSITY_KG_M3);
        thermal.ice_volume_fraction =
            input.ice_water_kg_m2[layer] / (input.layer_thickness_m[layer] * ICE_DENSITY_KG_M3);
        let properties = soil_thermal_properties(thermal, input.thermal_conductivity_scheme)?;
        heat_capacity[layer] = properties.heat_capacity_j_m3_k * input.layer_thickness_m[layer];
        conductivity[layer] = properties.conductivity_w_m_k;
    }
    for layer in 0..input.snow_layers {
        // `MOD_GroundTemperature.F90:206/216` 的 GIMPLE：
        //   `cv = .FMA (wliq, cpliq, wice*cpice)`
        //   `thk = .FMA (.FMA (rho, 7.75e-5, (rho*1.105e-6)*rho), tkice-tkair, tkair)`
        // 平铺写法在 AT-Neu 1 月第 130 步（第一个雪层）让 `tk(0)` 与 `cv(0)` 各差 1 ULP，
        // 土壤 1-3 层温度随之偏开（第 402 轮探针）。
        heat_capacity[layer] = input.liquid_water_kg_m2[layer].mul_add(
            WATER_HEAT_CAPACITY_J_KG_K,
            input.ice_water_kg_m2[layer] * ICE_HEAT_CAPACITY_J_KG_K,
        );
        let density = (input.liquid_water_kg_m2[layer] + input.ice_water_kg_m2[layer])
            / input.layer_thickness_m[layer];
        conductivity[layer] = density
            .mul_add(7.75e-5, density * 1.105e-6 * density)
            .mul_add(
                ICE_THERMAL_CONDUCTIVITY_W_M_K - AIR_THERMAL_CONDUCTIVITY_W_M_K,
                AIR_THERMAL_CONDUCTIVITY_W_M_K,
            );
    }
    Ok((heat_capacity, conductivity))
}

fn surface_fluxes(
    input: GroundTemperatureInput<'_>,
    use_snicar: bool,
) -> Result<(f64, f64, f64, f64)> {
    // `MOD_GroundTemperature.F90:250-305`。这一段里 GCC 把四个乘积提到函数开头
    // 当公共量（GIMPLE 的 `_292 _258 _301 _309`），它们**各自先舍入一次**再参与
    // 后面的加减 —— 所以不能把"降水热"写成 `cpliq*rain*Δ + cpice*snow*Δ` 那种
    // 合成子式和：上游是**顺序**加两个已舍入的乘积（`_25`、`_30`）。
    let emissivity_stefan = input.ground_emissivity * STEFAN_BOLTZMANN_W_M2_K4;
    let rain_heat_capacity = WATER_HEAT_CAPACITY_J_KG_K * input.rain_on_ground_kg_m2_s;
    let snow_heat_capacity = ICE_HEAT_CAPACITY_J_KG_K * input.snow_on_ground_kg_m2_s;
    let snow_top_absorption = input
        .snow_layer_absorption_w_m2
        .map(|values| values[0])
        .unwrap_or(0.0);
    // `fseng+fevpg*htvp` 三处都是 `FMA(fevpg, htvp, fseng)`（`_297/_105/_129`）。
    let ground_sensible = input
        .evaporation_ground_kg_m2_s
        .mul_add(input.vaporization_heat_j_kg, input.sensible_ground_w_m2);
    let precipitation_delta_ground = input.precipitation_temperature_k - input.ground_temperature_k;
    // **第 374 轮更正**：这一段原先的注释写着"四个乘积各自先舍入一次再参与后面的加减"、
    // 于是 `dlrad*emg` 与两个降水热项都写成了平铺 —— 但 `-fdump-tree-optimized-lineno`
    // 在 `:263` 上给出的是一条**四段全熔**的链：
    //   `_147 = .FMA(dlrad, emg, sabg)`
    //   `_152 = .FMA(fevpg, htvp, fseng)`、`_154 = _147 - _152`
    //   `_156 = pg_rain*cpliq`、`_162 = .FMA(_156, Δ, _154)`
    //   `_164 = pg_snow*cpice`、`_166 = .FMA(Δ, _164, _162)`
    // （`hs_soil`/`hs_snow` 的 `:279`/`:287` 同形。）旧注释大概是从另一版 dump 抄的。
    let surface = if use_snicar && input.snow_layers > 0 {
        snow_top_absorption + input.absorbed_soil_shortwave_w_m2
    } else {
        input.absorbed_ground_shortwave_w_m2
    };
    let surface = input
        .downward_longwave_w_m2
        .mul_add(input.ground_emissivity, surface);
    let surface = surface - ground_sensible;
    let surface = rain_heat_capacity.mul_add(precipitation_delta_ground, surface);
    let mut surface = precipitation_delta_ground.mul_add(snow_heat_capacity, surface);
    // `dhsdT` 的辐射项是 `FNMS(stefnc*(emg*4), (t*t)*t, cgrnd)`（`_261/_262/_264/_266`）。
    let stefan_factor = input.ground_emissivity * 4.0 * STEFAN_BOLTZMANN_W_M2_K4;
    let derivative = (-input.ground_temperature_k.powi(3)).mul_add(
        stefan_factor,
        -input.ground_flux_temperature_derivative_w_m2_k,
    ) - rain_heat_capacity
        - snow_heat_capacity;
    if !input.use_split_soil_snow {
        // `hs = hs - emg*stefnc*t_grnd**4` ⇒ `FNMA(t**4, stefnc*emg, hs)`（`_64`）。
        surface = (-input.ground_temperature_k.powi(4)).mul_add(emissivity_stefan, surface);
        return Ok((surface, 0.0, 0.0, derivative));
    }

    // 雪/土分开时辐射项按 `(fsno*emg)*stefnc` 分组后再吸收（`_83/_85`、`_91/_92`），
    // 与 `hs_soil`/`hs_snow` 里用的 `stefnc*emg`（`_258`）**不是**同一个分组。
    surface = (-input.snow_surface_temperature_k.powi(4)).mul_add(
        input.snow_cover_fraction * input.ground_emissivity * STEFAN_BOLTZMANN_W_M2_K4,
        surface,
    );
    surface = (-input.soil_surface_temperature_k.powi(4)).mul_add(
        (1.0 - input.snow_cover_fraction) * input.ground_emissivity * STEFAN_BOLTZMANN_W_M2_K4,
        surface,
    );
    // 两个降水热项在这里**是** FMA（`_115`、`_120`），与顶层 `hs` 相反：
    // 顶层的 `cpliq*rain*Δ` 被 CSE 成公共量，这里的没有被提。
    let soil_sensible = input
        .evaporation_soil_kg_m2_s
        .mul_add(input.vaporization_heat_j_kg, input.sensible_soil_w_m2);
    let soil_delta = input.precipitation_temperature_k - input.soil_surface_temperature_k;
    // `hs_soil`/`hs_snow` 的第一段（`main/MOD_GroundTemperature.F90:275-287`）GIMPLE 是
    // `.FMS (dlrad, emg, t**4*(emg*stefnc))` —— 熔进去的是 `dlrad*emg`，不是黑体项
    // （旧内核相反）。AT-Neu split 1 月第 233 步的雪层温度 1 ULP 由此而来。
    let soil_base = input.downward_longwave_w_m2.mul_add(
        input.ground_emissivity,
        -(input.soil_surface_temperature_k.powi(4) * emissivity_stefan),
    ) - soil_sensible;
    let soil = soil_delta.mul_add(
        snow_heat_capacity,
        soil_delta.mul_add(rain_heat_capacity, soil_base),
    );
    let soil = (1.0 - input.snow_cover_fraction).mul_add(soil, input.absorbed_soil_shortwave_w_m2);
    let snow_absorption = if use_snicar && input.snow_layers > 0 {
        snow_top_absorption
    } else {
        input.absorbed_snow_shortwave_w_m2
    };
    let snow_sensible = input
        .evaporation_snow_kg_m2_s
        .mul_add(input.vaporization_heat_j_kg, input.sensible_snow_w_m2);
    let snow_delta = input.precipitation_temperature_k - input.snow_surface_temperature_k;
    let snow_base = input.downward_longwave_w_m2.mul_add(
        input.ground_emissivity,
        -(input.snow_surface_temperature_k.powi(4) * emissivity_stefan),
    ) - snow_sensible;
    let snow_inner = snow_delta.mul_add(
        snow_heat_capacity,
        snow_delta.mul_add(rain_heat_capacity, snow_base),
    );
    // `hs_snow = hs_snow*fsno + sabg_snow`：乘积**没有**融合 —— GCC 把它提成
    // 两个 SNICAR 分支共用的 `_256`，而紧邻的 `hs_soil` 那一支 `FMA(1-fsno, ·, ·)`
    // 却是融合的。同一段代码里形状相同的两条语句可以有相反的结论。
    let snow = input.snow_cover_fraction * snow_inner + snow_absorption;
    ensure!(
        (input.absorbed_soil_shortwave_w_m2 + input.absorbed_snow_shortwave_w_m2
            - input.absorbed_ground_shortwave_w_m2)
            .abs()
            <= 1.0e-6
            && (soil + snow - surface).abs() <= 1.0e-6,
        "split soil and snow surface fluxes are not energy-consistent"
    );
    Ok((surface, soil, snow, derivative))
}

#[allow(clippy::too_many_arguments)]
fn temperature_system(
    input: GroundTemperatureInput<'_>,
    factor: &[f64],
    conductivity: &[f64],
    flux: &[f64],
    surface_heat_flux: f64,
    soil_heat_flux: f64,
    snow_heat_flux: f64,
    derivative: f64,
    use_snicar: bool,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let layers = input.temperature_k.len();
    let mut sub = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut super_ = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    let implicit = 1.0 - input.crank_nicolson_factor;
    let top_distance = input.node_depth_m[1] - input.node_depth_m[0];
    super_[0] = -implicit * factor[0] * conductivity[0] / top_distance;
    // 顶层矩阵元（`MOD_GroundTemperature.F90:331-335`）。收缩点（GIMPLE）：
    // `bt = 1+Q` 里的 Q 是**商**（不收缩），减去的那个乘积被吸收
    // （`FNMA(fsno*fact, dhsdT, 1+Q)` / `FNMA(fact, dhsdT, 1+Q)`）；
    // `rt` 括号里 `hs - dhsdT*t` 被吸收（`FNMA(dhsdT,t,hs)`），
    // 而 `+ cnfac*fn` 是**被 CSE 成公共临时量**、**不**融合的那一项
    // （同一个 `cnfac*fn(0)` 两个分支都用），最后 `FMA(括号, fact, t)`。
    let top_quotient = 1.0 + implicit * factor[0] * conductivity[0] / top_distance;
    if input.snow_layers > 0 && input.use_split_soil_snow {
        diagonal[0] = (-(input.snow_cover_fraction * factor[0])).mul_add(derivative, top_quotient);
        rhs[0] = factor[0].mul_add(
            (-(input.snow_cover_fraction * derivative))
                .mul_add(input.temperature_k[0], snow_heat_flux)
                + input.crank_nicolson_factor * flux[0],
            input.temperature_k[0],
        );
    } else {
        diagonal[0] = (-factor[0]).mul_add(derivative, top_quotient);
        rhs[0] = factor[0].mul_add(
            (-derivative).mul_add(input.temperature_k[0], surface_heat_flux)
                + input.crank_nicolson_factor * flux[0],
            input.temperature_k[0],
        );
    }
    for layer in 1..layers - 1 {
        let lower_distance = input.node_depth_m[layer] - input.node_depth_m[layer - 1];
        let upper_distance = input.node_depth_m[layer + 1] - input.node_depth_m[layer];
        let fortran_layer = layer as isize - input.snow_layers as isize + 1;
        sub[layer] = -implicit * factor[layer] * conductivity[layer - 1] / lower_distance;
        super_[layer] = -implicit * factor[layer] * conductivity[layer] / upper_distance;
        // 内层矩阵元（`:344-372`）。`bt = 1 + (1-cnfac)*fact*sum` 里那个乘积被吸收
        // （GIMPLE：`FMA(sum, (1-cnfac)*fact, 1.0)`），其中
        // `sum = tk/dzp + tk1/dzm` 是商之和、本身不参与收缩。
        let conductivity_sum =
            conductivity[layer] / upper_distance + conductivity[layer - 1] / lower_distance;
        let diagonal_sum = conductivity_sum.mul_add(implicit * factor[layer], 1.0);
        // 但 `rt = t + cnfac*fact*(fn-fn1)` **不融合**：那个乘积在三个内层分支
        // （雪层／`j==1 && split`／其它）里共用，GCC 把它 CSE 成一个临时量
        // （GIMPLE 的 `_462`）再与 `t` 相加。**相邻两条语句的结论可以相反**，
        // 只能逐条看 dump：`bt` 收了，`rt` 没收。
        let transient = input.temperature_k[layer]
            + input.crank_nicolson_factor * factor[layer] * (flux[layer] - flux[layer - 1]);
        if fortran_layer < 1 {
            diagonal[layer] = diagonal_sum;
            rhs[layer] = transient
                + if use_snicar {
                    input.snow_layer_absorption_w_m2.unwrap()[layer] * factor[layer]
                } else {
                    0.0
                };
        } else if fortran_layer == 1 && input.use_split_soil_snow {
            // `bt = 1+P - (1-fsno)*dhsdT*fact` ⇒ 末项吸收（`FNMA((1-fsno)*dhsdT, fact, 1+P)`）。
            diagonal[layer] = (-((1.0 - input.snow_cover_fraction) * derivative))
                .mul_add(factor[layer], diagonal_sum);
            rhs[layer] = factor[layer].mul_add(
                (-((1.0 - input.snow_cover_fraction) * derivative))
                    .mul_add(input.temperature_k[layer], soil_heat_flux),
                transient,
            );
        } else {
            diagonal[layer] = diagonal_sum;
            rhs[layer] = transient;
        }
    }
    let bottom = layers - 1;
    let lower_distance = input.node_depth_m[bottom] - input.node_depth_m[bottom - 1];
    sub[bottom] = -implicit * factor[bottom] * conductivity[bottom - 1] / lower_distance;
    diagonal[bottom] = 1.0 + implicit * factor[bottom] * conductivity[bottom - 1] / lower_distance;
    // `rt = t - cnfac*fact*fn1` ⇒ 乘积被吸收（GIMPLE：`FNMA(cnfac*fact, fn1, t)`）。
    // `bt = 1 + (1-cnfac)*fact*tk/dzm` 是**商**，不收缩，原样保留。
    rhs[bottom] = (-(input.crank_nicolson_factor * factor[bottom]))
        .mul_add(flux[bottom - 1], input.temperature_k[bottom]);
    (sub, diagonal, super_, rhs)
}

fn interface_fluxes(conductivity: &[f64], depth: &[f64], temperature: &[f64]) -> Result<Vec<f64>> {
    let mut flux = vec![0.0; temperature.len()];
    for layer in 0..temperature.len() - 1 {
        let distance = depth[layer + 1] - depth[layer];
        ensure!(
            distance > 0.0 && distance.is_finite(),
            "ground-temperature node depths must increase"
        );
        flux[layer] =
            conductivity[layer] * (temperature[layer + 1] - temperature[layer]) / distance;
    }
    Ok(flux)
}

fn residual_heat_fluxes(cnfac: f64, before: &[f64], after: &[f64]) -> Vec<f64> {
    let mut residual = vec![cnfac * before[0] + (1.0 - cnfac) * after[0]];
    for layer in 1..before.len() {
        residual.push(
            cnfac * (before[layer] - before[layer - 1])
                + (1.0 - cnfac) * (after[layer] - after[layer - 1]),
        );
    }
    residual
}

fn state_from_phase(
    phase: PhaseChangeState,
    snow_ice_before: Vec<f64>,
    time_step_seconds: f64,
    previous_temperature_k: Vec<f64>,
    layer_factor_seconds_per_j_m2_k: Vec<f64>,
    interface_conductivity_w_m_k: Vec<f64>,
) -> GroundTemperatureState {
    let snow_freezing_rate_kg_m2_s = phase.ice_water_kg_m2[..snow_ice_before.len()]
        .iter()
        .zip(snow_ice_before)
        .zip(&phase.phase_flag)
        .map(|((after, before), flag)| {
            if *flag == 2 {
                (after - before).max(0.0) / time_step_seconds
            } else {
                0.0
            }
        })
        .collect();
    GroundTemperatureState {
        temperature_k: phase.temperature_k.clone(),
        previous_temperature_k,
        liquid_water_kg_m2: phase.liquid_water_kg_m2,
        ice_water_kg_m2: phase.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: phase.snow_water_equivalent_kg_m2,
        snow_depth_m: phase.snow_depth_m,
        snow_melt_rate_kg_m2_s: phase.snow_melt_rate_kg_m2_s,
        latent_heat_flux_w_m2: phase.latent_heat_flux_w_m2,
        phase_flag: phase.phase_flag,
        thaw_mass_kg_m2: phase.thaw_mass_kg_m2,
        freeze_mass_kg_m2: phase.freeze_mass_kg_m2,
        snow_freezing_rate_kg_m2_s,
        layer_factor_seconds_per_j_m2_k,
        interface_conductivity_w_m_k,
    }
}

fn validate(input: GroundTemperatureInput<'_>) -> Result<usize> {
    let layers = input.temperature_k.len();
    let soil_layers = input.soil_thermal_inputs.len();
    ensure!(
        layers >= 2 && layers == input.snow_layers + soil_layers,
        "ground temperature needs at least two packed layers with all soil layers present"
    );
    for values in [
        input.layer_thickness_m,
        input.node_depth_m,
        input.temperature_k,
        input.liquid_water_kg_m2,
        input.ice_water_kg_m2,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "ground-temperature packed vectors must be finite and match"
        );
    }
    ensure!(
        input.interface_depth_m.len() == layers + 1
            && input
                .interface_depth_m
                .iter()
                .all(|value| value.is_finite()),
        "ground-temperature interfaces must be finite and have one extra boundary"
    );
    ensure!(
        input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0),
        "ground-temperature thicknesses and water masses are invalid"
    );
    for values in [
        input.soil_porosity,
        input.soil_residual_water,
        input.soil_suction_mm,
    ] {
        ensure!(
            values.len() == soil_layers && values.iter().all(|value| value.is_finite()),
            "ground-temperature soil vectors must match the soil layers"
        );
    }
    ensure!(
        input.soil_hydraulic_model.len() == soil_layers
            && input.soil_porosity.iter().all(|value| *value >= 0.0)
            && input.soil_residual_water.iter().all(|value| *value >= 0.0)
            && input.soil_suction_mm.iter().all(|value| *value < 0.0),
        "ground-temperature soil hydraulics are invalid"
    );
    if let Some(values) = input.snow_layer_absorption_w_m2 {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "SNICAR absorption must be finite and match packed layers"
        );
    }
    let scalars = [
        input.time_step_seconds,
        input.surface_temperature_factor,
        input.crank_nicolson_factor,
        input.snow_water_equivalent_kg_m2,
        input.snow_depth_m,
        input.snow_cover_fraction,
        input.absorbed_ground_shortwave_w_m2,
        input.absorbed_soil_shortwave_w_m2,
        input.absorbed_snow_shortwave_w_m2,
        input.downward_longwave_w_m2,
        input.sensible_ground_w_m2,
        input.sensible_soil_w_m2,
        input.sensible_snow_w_m2,
        input.evaporation_ground_kg_m2_s,
        input.evaporation_soil_kg_m2_s,
        input.evaporation_snow_kg_m2_s,
        input.ground_flux_temperature_derivative_w_m2_k,
        input.vaporization_heat_j_kg,
        input.ground_emissivity,
        input.rain_on_ground_kg_m2_s,
        input.snow_on_ground_kg_m2_s,
        input.precipitation_temperature_k,
        input.ground_temperature_k,
        input.soil_surface_temperature_k,
        input.snow_surface_temperature_k,
    ];
    ensure!(
        scalars.iter().all(|value| value.is_finite())
            && input.time_step_seconds > 0.0
            && input.surface_temperature_factor > 0.0
            && (0.0..=1.0).contains(&input.crank_nicolson_factor)
            && input.snow_water_equivalent_kg_m2 >= 0.0
            && input.snow_depth_m >= 0.0
            && (0.0..=1.0).contains(&input.snow_cover_fraction)
            && input.ground_emissivity >= 0.0,
        "ground-temperature scalar inputs are invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "ground_temperature_tests.rs"]
mod ground_temperature_tests;
