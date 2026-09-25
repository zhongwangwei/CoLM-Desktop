//! Standard land-cover canopy energy balance from `MOD_LeafTemperature.F90`.
//!
//! This is the reusable LCT/PFT two-big-leaf path used by the native runtime;
//! it contains no NetCDF or process orchestration. Plant hydraulics shares the
//! root-to-leaf network in [`crate::plant_hydraulics`]; ozone remains a
//! separate upstream feature branch and is not silently approximated here.

use anyhow::{ensure, Context, Result};

use crate::{
    balance_phs_rootflux, canopy_diffusivity_resistance_analytic, canopy_monin_obukhov_with_scheme,
    canopy_roughness, canopy_wetness, effective_canopy_wind, initialize_monin_obukhov,
    plant_hydraulic_stress, saturation_specific_humidity, stomata, update_photosynthesis,
    CanopyDiffusivityProfileInput, CanopyMoninObukhovInput, CanopyWater, CanopyWindProfileInput,
    LeafBiochemistry, LeafPhotosynthesisInput, MoninObukhovInitialInput, MoninObukhovInput,
    PhotosynthesisUpdateInput, PlantHydraulicInput, PlantHydraulicParameters, PlantHydraulicState,
    StomataInput, StomataOptions, StomataState, SurfaceLayerScheme, FREEZING_K,
};

const VON_KARMAN: f64 = 0.4;
const GRAVITY_M_S2: f64 = 9.80616;
const LATENT_HEAT_VAPORIZATION_J_KG: f64 = 2.5104e6;
/// `MOD_Const_Physical.F90:18` 的 `hsub`，叶温求解里的 `htvpl` 用它。
///
/// 上游把它声明成**独立常数**（数值上恰好等于 `hvap + hfus`），这里照抄数值而不是
/// 相加 —— `hfus` 一改就会让相加版悄悄跟着变，而独立常数不会。
const LATENT_HEAT_SUBLIMATION_J_KG: f64 = 2.8440e6;
const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const STEFAN_BOLTZMANN: f64 = 5.67e-8;
const FUSION_HEAT_J_KG: f64 = 0.3336e6;
const MAX_ITERATIONS: usize = 40;
const MIN_ITERATIONS: usize = 6;
const MAX_TEMPERATURE_STEP_K: f64 = 3.0;
const TEMPERATURE_TOLERANCE_K: f64 = 0.01;
const FLUX_TOLERANCE_W_M2: f64 = 0.1;

/// Whether forcing observation heights are absolute elevations or heights above
/// the canopy top, matching `DEF_forcing%HEIGHT_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationHeightMode {
    Absolute,
    RelativeToCanopy,
}

/// Runtime switches that materially change the standard leaf-temperature path.
#[derive(Debug, Clone, Copy)]
pub struct LeafTemperatureOptions {
    pub observation_height_mode: ObservationHeightMode,
    pub vegetation_snow: bool,
    pub split_soil_snow: bool,
    /// `DEF_RSS_SCHEME == 4`: `soil_surface_resistance` is a conductance factor.
    pub soil_resistance_is_conductance: bool,
    pub surface_layer_scheme: SurfaceLayerScheme,
    pub stomata: StomataOptions,
}

impl Default for LeafTemperatureOptions {
    fn default() -> Self {
        Self {
            observation_height_mode: ObservationHeightMode::Absolute,
            vegetation_snow: false,
            split_soil_snow: false,
            soil_resistance_is_conductance: false,
            surface_layer_scheme: SurfaceLayerScheme::Standard,
            stomata: StomataOptions::default(),
        }
    }
}

/// CoLM 在 `MOD_Thermal.F90:543` 用的固定温度递减率（K/m）。
const REFERENCE_LAPSE_RATE_K_M: f64 = 0.0098;

/// 上游的 `thm`：`MOD_Thermal.F90:550` 的 `forc_t + 0.0098*forc_hgt_t`。
///
/// **它不是位温。** 位温是同处的 `th = forc_t*(100000/psrf)**(rgas/cpair)`
/// （本仓库的 [`LeafTemperatureInput::potential_temperature_k`]），而 `thm` 只是
/// "把 reference height 处的气温按固定递减率抬到观测高度"的近似。
/// 叶温模块两者都要：`dth = thm - taf`、`taf = wta0*thm + ...` 用 `thm`，
/// 而 `dthv = dth*(1+0.61*qm) + 0.61*th*dqh` 与 `moninobukini(ur, th, thm, thv, ...)`
/// 里的那一项用 `th`。混用在本算例是 **0.0588 K** 的常数偏差
/// （`forc_hgt_t = 6` m），而那正是叶温残差的主项 —— 实测换过来之后第一步叶温差
/// 从 0.0316 K 降到 ~1e-3 K 量级。
///
/// 与潜热同类的教训：`thm` 这个名字看起来像"potential temperature"的缩写，
/// 但上游把它定义成了别的东西。**照抄表达式，不要按名字推断。**
pub fn reference_height_temperature_k(air_temperature_k: f64, temperature_height_m: f64) -> f64 {
    // `:550` 的 GIMPLE 是 `thm = .FMA(forc_hgt_t, 9.8e-3, forc_t)`：那条乘积被收进加法。
    REFERENCE_LAPSE_RATE_K_M.mul_add(temperature_height_m, air_temperature_k)
}

/// Soil profiles and fixed hydraulic parameters for the two-leaf PHS branch.
///
/// This is present only when `DEF_USE_PLANTHYDRAULICS` is active. Meteorology,
/// leaf area, canopy geometry, and unstressed stomatal conductance remain
/// inputs of [`LeafTemperatureInput`], which prevents the runtime from
/// duplicating the source hand-off.
#[derive(Debug, Clone, Copy)]
pub struct LeafPlantHydraulicInput<'a> {
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub root_fraction: &'a [f64],
    pub soil_matric_potential_mm: &'a [f64],
    pub soil_hydraulic_conductivity_mm_s: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub maximum_sunlit_leaf_hydraulic_conductance: f64,
    pub maximum_shaded_leaf_hydraulic_conductance: f64,
    pub maximum_xylem_hydraulic_conductance: f64,
    pub maximum_root_hydraulic_conductance: f64,
    pub sunlit_leaf_psi50_mm: f64,
    pub shaded_leaf_psi50_mm: f64,
    pub xylem_psi50_mm: f64,
    pub root_psi50_mm: f64,
    pub vulnerability_shape: f64,
    /// `DEF_RSS_SCHEME`; scheme 4 uses a conductance-style soil factor.
    pub soil_surface_resistance_scheme: i32,
    pub parameters: PlantHydraulicParameters,
}

/// Immutable forcing, surface, and vegetation parameters for one canopy step.
#[derive(Debug, Clone, Copy)]
pub struct LeafTemperatureInput<'a> {
    pub time_step_seconds: f64,
    pub maximum_dew_mm: f64,
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub canopy_top_height_m: f64,
    pub inverse_sqrt_leaf_dimension_m_neg_half: f64,
    pub biochemistry: LeafBiochemistry,
    pub soil_water_stress_sunlit: f64,
    pub soil_water_stress_shaded: f64,
    pub wue_lambda: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub reference_air_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub virtual_potential_temperature_k: f64,
    pub reference_specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub air_density_kg_m3: f64,
    pub sunlit_absorbed_par_w_m2: f64,
    pub shaded_absorbed_par_w_m2: f64,
    pub canopy_absorbed_solar_w_m2: f64,
    pub atmospheric_longwave_w_m2: f64,
    pub sunlit_fraction: f64,
    pub canopy_longwave_gap_fraction: f64,
    pub oxygen_partial_pressure_pa: f64,
    pub atmospheric_co2_pa: f64,
    pub soil_roughness_m: f64,
    pub snow_roughness_m: f64,
    /// `hpbl`：本步的大气边界层高度，只有 `LargeEddy` 近地层方案会读它。
    pub boundary_layer_height_m: Option<f64>,
    pub snow_cover_fraction: f64,
    pub ground_obukhov_length_m: f64,
    pub transpiration_limit_kg_m2_s: f64,
    pub ground_temperature_k: f64,
    pub soil_surface_temperature_k: f64,
    pub snow_surface_temperature_k: f64,
    pub ground_specific_humidity: f64,
    pub soil_specific_humidity: f64,
    pub snow_specific_humidity: f64,
    pub ground_humidity_temperature_slope_k: f64,
    pub soil_surface_resistance_s_m: f64,
    pub ground_emissivity: f64,
    pub precipitation_temperature_k: f64,
    pub intercepted_rain_kg_m2_s: f64,
    pub intercepted_snow_kg_m2_s: f64,
    pub ground_latent_heat_j_kg: f64,
    /// Optional default LCT PHS branch. Its persistent potential lives in
    /// [`LeafTemperatureState::plant_hydraulics`].
    pub plant_hydraulics: Option<LeafPlantHydraulicInput<'a>>,
    pub options: LeafTemperatureOptions,
}

/// Persistent state updated by one leaf-temperature solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafTemperatureState {
    pub leaf_temperature_k: f64,
    pub canopy_water: CanopyWater,
    /// Persistent sunlit/shaded/xylem/root water potentials for PHS.
    pub plant_hydraulics: Option<PlantHydraulicState>,
}

/// Fluxes and diagnostic state produced by one converged canopy solve.
#[derive(Debug, Clone, PartialEq)]
pub struct LeafTemperatureOutput {
    pub wet_snow_fraction: f64,
    pub eastward_stress_kg_m_s2: f64,
    pub northward_stress_kg_m_s2: f64,
    pub ground_sensible_heat_w_m2: f64,
    pub soil_sensible_heat_w_m2: f64,
    pub snow_sensible_heat_w_m2: f64,
    pub ground_evaporation_kg_m2_s: f64,
    pub soil_evaporation_kg_m2_s: f64,
    pub snow_evaporation_kg_m2_s: f64,
    pub ground_flux_temperature_slope_w_m2_k: f64,
    pub ground_sensible_temperature_slope_w_m2_k: f64,
    pub ground_latent_temperature_slope_kg_m2_s_k: f64,
    pub air_temperature_2m_k: f64,
    pub air_specific_humidity_2m: f64,
    pub canopy_stomatal_resistance_s_m: f64,
    /// 本步地面蒸发用的潜热（`MOD_Thermal.F90:539-540` 的 `htvp`）。
    ///
    /// 随输入原样带出来，好让 history 的 `lfevpa`/`fgrnd` 与物理用的是**同一个**
    /// `htvp` —— 自己再判一次表层冰水比就会有两份判据。
    pub ground_latent_heat_j_kg: f64,
    /// 本步冠层蒸发/凝华用的潜热，即上游的 **`lfevpl / fevpl`**（`htvpl`）。
    ///
    /// 上游在 `MOD_LeafTemperature_Extended.F90:1584` 导出 `lfevpl = htvpl*fevpl`，
    /// 而编译进来的 `extends/interception/MOD_Thermal_CanopyPhase_Extended.F90:1343`
    /// 写的是 **`lfevpa = lfevpl + htvp*fevpg`** —— 叶面那一项用的是 `htvpl`
    /// （叶温在冰点以下时按升华计价），**不是** `hvap`。
    ///
    /// `main/MOD_Thermal.F90:1333` 的 `lfevpa = hvap*fevpl + htvp*fevpg` 是**旧版**，
    /// 从不参与编译（`Makefile` 用 `extends/` 顶掉了 `MOD_Thermal*`）。照旧版写会让
    /// `f_lfevpa` 差 `(hsub-hvap)*fevpl ≈ 3.3e5*fevpl`，实测冬季窗口最大 3.14 W/m²；
    /// 更隐蔽的是它还会让 `f_zerr` 差出同一个量（冠层能量收支是按 `htvpl` 闭合的）。
    pub leaf_latent_heat_j_kg: f64,
    /// `laisun = lai*fsun`、`laisha = lai*(1-fsun)`。
    ///
    /// 上游把它们当每步的 patch 量累加进 history（`MOD_Vars_1DAccFluxes.F90:2147-2148`），
    /// 内核里本来就是这两个值，只是先前没带出来。
    pub sunlit_leaf_area_index: f64,
    pub shaded_leaf_area_index: f64,
    pub sunlit_stomatal_conductance_mol_m2_s: f64,
    pub shaded_stomatal_conductance_mol_m2_s: f64,
    pub assimilation_mol_m2_s: f64,
    pub respiration_mol_m2_s: f64,
    pub leaf_sensible_heat_w_m2: f64,
    pub leaf_evaporation_kg_m2_s: f64,
    pub transpiration_kg_m2_s: f64,
    pub sunlit_transpiration_kg_m2_s: f64,
    pub shaded_transpiration_kg_m2_s: f64,
    /// PHS soil-layer uptake. Empty when plant hydraulics is disabled.
    pub root_flux_kg_m2_s: Vec<f64>,
    /// 冠层胁迫因子，写进 history 的 `f_rstfacsun`/`f_rstfacsha`。
    ///
    /// PHS 关掉时就是调用方给的 `eroot`（LCT 分支两行同源）；PHS 打开时是
    /// [`crate::plant_hydraulic_stress`] 最后一轮算出的因子 —— 上游那两个变量
    /// 是 `intent(inout)`，`stomata` 只读不写，所以最终值确实是 PHS 的。
    pub sunlit_soil_water_stress: f64,
    pub shaded_soil_water_stress: f64,
    /// `gs0sun`/`gs0sha` [µmol m-2 s-1]：PHS 的最大叶导度，上游写进重启的两个
    /// 时间变量。关掉 PHS 时是 `None`（上游那时从不给它们赋值）。
    pub maximum_sunlit_leaf_conductance_umol_m2_s: Option<f64>,
    pub maximum_shaded_leaf_conductance_umol_m2_s: Option<f64>,
    pub sunlit_assimilation_mol_m2_s: f64,
    pub shaded_assimilation_mol_m2_s: f64,
    pub downward_longwave_w_m2: f64,
    pub upward_longwave_w_m2: f64,
    pub precipitation_heat_w_m2: f64,
    pub canopy_heat_storage_w_m2: f64,
    pub momentum_roughness_m: f64,
    pub zol: f64,
    pub bulk_richardson: f64,
    pub friction_velocity_m_s: f64,
    pub humidity_scale: f64,
    pub temperature_scale_k: f64,
    pub momentum_similarity: f64,
    pub heat_similarity: f64,
    pub moisture_similarity: f64,
    pub reference_to_canopy_moisture_resistance_s_m: f64,
    pub energy_balance_error_w_m2: f64,
    pub iterations: usize,
}

/// Port of `MOD_LeafTemperature:LeafTemperature` for the normal LCT/PFT path.
///
/// This solver deliberately uses the existing pure canopy, Monin-Obukhov,
/// humidity, and stomata kernels.  The caller owns interception and ground
/// thermal updates; this function only updates the leaf temperature and dew
/// pools that belong to the leaf energy balance.
pub fn leaf_temperature(
    input: LeafTemperatureInput<'_>,
    state: &mut LeafTemperatureState,
) -> Result<LeafTemperatureOutput> {
    validate(input, *state)?;
    let lai = input.leaf_area_index;
    let sai = input.stem_area_index;
    let lsai = lai + sai;
    let fsha = 1.0 - input.sunlit_fraction;
    let laisun = lai * input.sunlit_fraction;
    let laisha = lai * fsha;
    let cintsun = sunlit_canopy_integration(input.direct_extinction, input.diffuse_extinction, lai);
    // `MOD_LeafTemperature.F90:464-466` 的 `cintsha`。
    let cintsha = [
        integrated_extinction(0.110, lai) - cintsun[0],
        integrated_extinction(input.diffuse_extinction, lai) - cintsun[1],
        lai - cintsun[2],
    ];
    // `MOD_LeafTemperature_Extended.F90:542` 的 GIMPLE 是两条 FMA：
    //   `_53 = .FMA(ldew_rain, cpliq, 0.2*(lai+sai)*cpliq)`、
    //   `clai = .FMA(ldew_snow, cpice, _53)`
    // —— `0.2*(lai+sai)*cpliq` 那条链独立舍入，另两个乘积各自熔进当时的累加值。
    let clai = if input.options.vegetation_snow {
        let canopy_capacity = 0.2 * lsai * WATER_HEAT_CAPACITY_J_KG_K;
        let canopy_capacity = state
            .canopy_water
            .rain_mm
            .mul_add(WATER_HEAT_CAPACITY_J_KG_K, canopy_capacity);
        state
            .canopy_water
            .snow_mm
            .mul_add(ICE_HEAT_CAPACITY_J_KG_K, canopy_capacity)
    } else {
        0.0
    };
    let wetness = canopy_wetness(
        lai,
        sai,
        input.maximum_dew_mm,
        state.canopy_water,
        input.options.vegetation_snow,
    )?;
    let fwet = wetness.wet_fraction;
    let roughness = canopy_roughness(lsai, input.canopy_top_height_m, 1.0)?;
    let z0mv = roughness.momentum_roughness_m;
    let displacement = roughness.displacement_height_m;
    let displasink = (input.canopy_top_height_m / 2.0).max(displacement);
    let hsink = z0mv + displasink;
    // `:606 z0mg = (1-fsno)*zlnd + fsno*zsno` 的 GIMPLE 是
    // `.FMA(1-fsno, zlnd, fsno*zsno)` —— 第一个乘积进 FMA，第二个独立舍入。
    let z0mg = (1.0 - input.snow_cover_fraction).mul_add(
        input.soil_roughness_m,
        input.snow_cover_fraction * input.snow_roughness_m,
    );
    let frontal_area = 1.0 - (-0.5 * lsai).exp();
    // `:639 sqrtdragc = min((0.003+0.3*fai)**0.5, 0.3)` 的 GIMPLE 是 `.FMA(fai, 0.3, 0.003)`。
    let sqrt_drag = 0.3_f64.mul_add(frontal_area, 0.003).sqrt().min(0.3);
    let attenuation = input.canopy_top_height_m
        / (input.canopy_top_height_m - displacement)
        / (VON_KARMAN / sqrt_drag);
    let (wind_height, temperature_height, humidity_height) =
        match input.options.observation_height_mode {
            ObservationHeightMode::Absolute => (
                input.wind_height_m.max(input.canopy_top_height_m + 1.0),
                input
                    .temperature_height_m
                    .max(input.canopy_top_height_m + 1.0),
                input.humidity_height_m.max(input.canopy_top_height_m + 1.0),
            ),
            ObservationHeightMode::RelativeToCanopy => (
                input.canopy_top_height_m + input.wind_height_m,
                input.canopy_top_height_m + input.temperature_height_m,
                input.canopy_top_height_m + input.humidity_height_m,
            ),
        };
    let reference_height = wind_height - displacement;
    let mut canopy_air_temperature =
        0.5 * (input.ground_temperature_k + input.reference_air_temperature_k);
    let mut canopy_air_humidity =
        0.5 * (input.reference_specific_humidity + input.ground_specific_humidity);
    let mut canopy_air_co2 = input.atmospheric_co2_pa;
    // `MOD_LeafTemperature.F90:556`：`ur = max(0.1, sqrt(us*us+vs*vs))`（us 在前）。
    let reference_wind = input
        .eastward_wind_m_s
        .mul_add(
            input.eastward_wind_m_s,
            input.northward_wind_m_s * input.northward_wind_m_s,
        )
        .sqrt()
        .max(0.1);
    let mut temperature_difference = input.reference_air_temperature_k - canopy_air_temperature;
    let mut humidity_difference = input.reference_specific_humidity - canopy_air_humidity;
    // `(1.+0.61*qm)` 在内核里是**算一次、处处复用**的（`lt_ext.s`：`:657` 处
    // `fmadd(qm, 0.61, 1.0)` 存进 `[x29,472]`，`:1256` 的 `thvstar` 直接 load 回来），
    // 而且那一次乘法是**融合**的。两个用处（`dthv` 与循环里的 `thvstar`）形状相同：
    //   `dthv    = fma(dth, _149, dqh*(0.61*th))`
    //   `thvstar = fma(tstar, _149, qstar*(0.61*th))`
    // 原先两处都写成平铺，循环里的那处在 it=6 让 `thvstar` 差 1 ULP。
    let one_plus_0_61_reference_humidity = 0.61f64.mul_add(input.reference_specific_humidity, 1.0);
    let virtual_temperature_difference = temperature_difference.mul_add(
        one_plus_0_61_reference_humidity,
        (0.61 * input.potential_temperature_k) * humidity_difference,
    );
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: reference_wind,
        potential_temperature_k: input.potential_temperature_k,
        reference_temperature_k: input.reference_air_temperature_k,
        virtual_potential_temperature_k: input.virtual_potential_temperature_k,
        temperature_difference_k: temperature_difference,
        humidity_difference_kg_kg: humidity_difference,
        virtual_temperature_difference_k: virtual_temperature_difference,
        reference_height_m: reference_height,
        momentum_roughness_m: z0mv,
    })?;
    let mut stability_wind = initial.stability_adjusted_wind_m_s;
    let mut obukhov = initial.obukhov_length_m;
    let mut prior_obukhov = 0.0;
    let mut obukhov_sign_changes = 0;
    let mut prior_temperature_change = 0.0;
    let mut prior_flux_change = 0.0;
    let mut prior_leaf_evaporation = 0.0;
    let mut previous_leaf_temperature = state.leaf_temperature_k;
    let mut dtl = [0.0; MAX_ITERATIONS + 2];
    let mut iteration = 1;
    let mut last = Iteration::default();

    // 上游对**净**截留率取 `max(0, ·)`：`MOD_LeafTemperature_Extended.F90:1174-1177`
    // 的分子与分母、循环后 `fsenl` 的修正项、以及 `hprl`（`:1335` 的
    // "VIC-qintr: clamp qintr_* to max(0,·) — must match dtl denom"）。
    //
    // **净截留率确实可以为负**：`qintr_rain = (prc_rain+prl_rain+qflx_irrig)
    // - thru_rain/deltim`，而 `thru_rain` 里含冠层排水 `tex_rain`，排水超过截留时
    // 它就是负的。负值不能反向"注入" `t_precip - tl` 的能量，所以要夹。
    //
    // 本仓库把**原始**净通量放在 `LeafTemperatureInput` 上（与上游 `qintr_*` 同义），
    // 夹在这里做 —— 与上游同位置。以前 `validate` 直接要求它非负，那会在任何
    // 排水步上**报错**，而正确行为是按上游夹掉。
    let intercepted_rain = input.intercepted_rain_kg_m2_s.max(0.0);
    let intercepted_snow = input.intercepted_snow_kg_m2_s.max(0.0);

    while iteration <= MAX_ITERATIONS {
        previous_leaf_temperature = state.leaf_temperature_k;
        // `htvpl`：**叶面**的潜热随叶温在汽化与升华之间切换
        // （`MOD_LeafTemperature_Extended.F90:693-697`，每轮迭代开头按**当前** `tl` 重算）。
        // 上游在本模块里用它而不是硬写 `hvap` 的地方有七处：增量的分子与分母、
        // `dele` 收敛判据、循环后 `fsenl` 的三项修正、以及 `err` 那条能量残差。
        // 写死 `hvap` 会让零下冠层的凝华按汽化计价，差 13%。
        //
        // **注意与 `lfevpa` 的区别**：`MOD_Thermal.F90:1333` 的
        // `lfevpa = hvap*fevpl + htvp*fevpg` 对叶面那一项用的是 `hvap`，
        // 不是 `htvpl` —— 上游自己就不一致，本仓库两边照各自的写法抄。
        let leaf_latent_heat_j_kg = if previous_leaf_temperature > FREEZING_K {
            LATENT_HEAT_VAPORIZATION_J_KG
        } else {
            LATENT_HEAT_SUBLIMATION_J_KG
        };
        let profile = canopy_monin_obukhov_with_scheme(
            CanopyMoninObukhovInput {
                surface: MoninObukhovInput {
                    wind_height_m: wind_height,
                    temperature_height_m: temperature_height,
                    humidity_height_m: humidity_height,
                    displacement_height_m: displacement,
                    momentum_roughness_m: z0mv,
                    heat_roughness_m: z0mv,
                    moisture_roughness_m: z0mv,
                    obukhov_length_m: obukhov,
                    stability_adjusted_wind_m_s: stability_wind,
                    boundary_layer_height_m: input.boundary_layer_height_m,
                },
                top_layer_displacement_m: displasink,
                top_layer_roughness_m: z0mv,
                canopy_top_height_m: input.canopy_top_height_m,
            },
            input.options.surface_layer_scheme,
        )?;
        let surface = profile.surface;
        let ram = 1.0 / (surface.friction_velocity_m_s.powi(2) / stability_wind);
        let rah = 1.0
            / (VON_KARMAN / (surface.heat - profile.heat_at_top_layer)
                * surface.friction_velocity_m_s);
        let raw = 1.0
            / (VON_KARMAN / (surface.moisture - profile.moisture_at_top_layer)
                * surface.friction_velocity_m_s);
        let z0hg = z0mg / (0.13 * (surface.friction_velocity_m_s * z0mg / 1.5e-5).powf(0.45)).exp();
        let z0qg = z0hg;
        let wind_at_top =
            surface.friction_velocity_m_s / VON_KARMAN * profile.momentum_at_canopy_top;
        let effective_wind = effective_canopy_wind(CanopyWindProfileInput {
            wind_at_canopy_top_m_s: wind_at_top,
            canopy_cover_fraction: 1.0,
            canopy_blend_weight: 1.0,
            attenuation_coefficient: attenuation,
            ground_momentum_roughness_m: z0mg,
            canopy_top_height_m: input.canopy_top_height_m,
            canopy_bottom_height_m: z0mg,
        })?;
        let leaf_boundary_resistance =
            1.0 / (0.01 * input.inverse_sqrt_leaf_dimension_m_neg_half * effective_wind.sqrt());
        let ktop =
            VON_KARMAN * (input.canopy_top_height_m - displacement) * surface.friction_velocity_m_s
                / profile.canopy_top_heat_similarity;
        let ground_to_canopy_resistance = canopy_diffusivity_resistance_analytic(
            CanopyDiffusivityProfileInput {
                diffusivity_at_canopy_top_m2_s: ktop,
                canopy_cover_fraction: 1.0,
                canopy_blend_weight: 1.0,
                attenuation_coefficient: attenuation,
                displacement_height_m: displacement / input.canopy_top_height_m,
                canopy_top_height_m: input.canopy_top_height_m,
                canopy_bottom_height_m: z0qg,
                obukhov_length_m: input.ground_obukhov_length_m,
                friction_velocity_m_s: surface.friction_velocity_m_s,
            },
            hsink,
            z0qg,
            z0qg,
        )?;
        let leaf_saturation =
            saturation_specific_humidity(state.leaf_temperature_k, input.surface_pressure_pa)?;
        // `eah = qaf*psrf/(0.622 + 0.378*qaf)`（`MOD_LeafTemperature.F90:688`）：
        // 分母同样是 `a + b*c`，被收缩成 `fma(0.378,qaf,0.622)`（实测 4000/4000
        // 相同；不收缩时 4000 组里有 33 组不同）。`eah` 进 `stomata` 的
        // `D = max(ei-ea,50)/psrf`，而这处分母在两个水汽压相近时很小 ——
        // 同一个 1 ULP 在这里会被放大。分子是单个乘除，无可收缩点。
        let canopy_vapor_pressure = canopy_air_humidity * input.surface_pressure_pa
            / 0.378_f64.mul_add(canopy_air_humidity, 0.622);
        let stomatal_soil_stress = if input.plant_hydraulics.is_some() {
            1.0
        } else {
            input.soil_water_stress_sunlit
        };
        // 上游调用前把叶尺度 `rb` 折成冠层尺度：`rbsun = rb/laisun`、`rbsha = rb/laisha`
        // （`MOD_LeafTemperature_Extended.F90:797-798`），返回后再 `rssun = rssun*laisun`
        // （`:941`）。两处的 `laisun` 在 `gssun = (laisun/rssun)*(tprcor/tlbef)`（`:1320`）
        // 里相消，所以这一处**数值上是惰性的**（实测步级口径逐位不变、三个窗口的容差口径
        // 也不变），但形状必须照抄 —— PHS 那条路传的是 `rb` 本身（`:745-752`），不折。
        let mut sunlit_resistance = stomatal_resistance(
            input,
            StomataStep {
                leaf_temperature_k: state.leaf_temperature_k,
                leaf_boundary_resistance_s_m: leaf_boundary_resistance / laisun,
                absorbed_par_w_m2: input.sunlit_absorbed_par_w_m2,
                soil_water_stress: stomatal_soil_stress,
                canopy_integration: cintsun,
                canopy_air_co2_pa: canopy_air_co2,
                canopy_vapor_pressure_pa: canopy_vapor_pressure,
                leaf_vapor_pressure_pa: leaf_saturation.vapor_pressure_pa,
            },
        )?;
        let mut shaded_resistance = stomatal_resistance(
            input,
            StomataStep {
                leaf_temperature_k: state.leaf_temperature_k,
                leaf_boundary_resistance_s_m: leaf_boundary_resistance / laisha,
                absorbed_par_w_m2: input.shaded_absorbed_par_w_m2,
                soil_water_stress: if input.plant_hydraulics.is_some() {
                    1.0
                } else {
                    input.soil_water_stress_shaded
                },
                canopy_integration: cintsha,
                canopy_air_co2_pa: canopy_air_co2,
                canopy_vapor_pressure_pa: canopy_vapor_pressure,
                leaf_vapor_pressure_pa: leaf_saturation.vapor_pressure_pa,
            },
        )?;
        let mut root_flux_kg_m2_s = Vec::new();
        // `MOD_PlantHydraulic.F90:353-368` 的 `calcstress_twoleaf` 把
        // `rstfacsun`/`rstfacsha` 重写成 **PHS 自己的**胁迫因子
        // （`amax1(gssun/gs0sun, 1e-2)` 或 `amax1(plc(psi,psi50,ck), 1e-2)`），
        // 而且是 `intent(inout)`：`stomata` 只读不写，所以循环结束后上游的
        // `rstfacsun` 就是最后一轮 PHS 给的值。关掉 PHS 时它保持调用方的
        // `eroot`。两者必须分开记，否则 history 的 `f_rstfacsun` 在 PHS 下
        // 写的是**没被 PHS 改过**的那个数 —— 实测黄金 0.99988 对 0.0454。
        let mut sunlit_soil_water_stress = input.soil_water_stress_sunlit;
        let mut shaded_soil_water_stress = input.soil_water_stress_shaded;
        // `gs0sun`/`gs0sha`：`MOD_LeafTemperature_Extended.F90:817-818` 的
        // 「最大叶导度」，`rstfacsun = gssun/gs0sun` 的分母，也是上游写进重启的
        // 两个时间变量。**只在 PHS 分支里赋值**，所以关掉 PHS 时它们是 `None`
        // —— 那时上游根本不碰这两个变量，写回任何数都是编的。
        let mut gs0sun = None;
        let mut gs0sha = None;
        if let Some(hydraulic) = input.plant_hydraulics {
            let pressure_conversion = 44.6 * 273.16 * input.surface_pressure_pa / 1.013e5;
            let maximum_sunlit_leaf_conductance_umol_m2_s = (1.0
                / (sunlit_resistance.stomatal_resistance_s_m * state.leaf_temperature_k
                    / pressure_conversion))
                .min(1.0e6)
                / laisun
                * 1.0e6;
            let maximum_shaded_leaf_conductance_umol_m2_s = (1.0
                / (shaded_resistance.stomatal_resistance_s_m * state.leaf_temperature_k
                    / pressure_conversion))
                .min(1.0e6)
                / laisha
                * 1.0e6;
            gs0sun = Some(maximum_sunlit_leaf_conductance_umol_m2_s);
            gs0sha = Some(maximum_shaded_leaf_conductance_umol_m2_s);
            let hydraulic_state = state.plant_hydraulics.as_mut().ok_or_else(|| {
                anyhow::anyhow!("plant hydraulics input requires persistent plant hydraulic state")
            })?;
            let hydraulic_output = plant_hydraulic_stress(
                PlantHydraulicInput {
                    node_depth_m: hydraulic.node_depth_m,
                    layer_thickness_m: hydraulic.layer_thickness_m,
                    root_fraction: hydraulic.root_fraction,
                    soil_matric_potential_mm: hydraulic.soil_matric_potential_mm,
                    soil_hydraulic_conductivity_mm_s: hydraulic.soil_hydraulic_conductivity_mm_s,
                    saturated_hydraulic_conductivity_mm_s: hydraulic
                        .saturated_hydraulic_conductivity_mm_s,
                    surface_pressure_pa: input.surface_pressure_pa,
                    leaf_saturation_specific_humidity: leaf_saturation.specific_humidity,
                    canopy_air_specific_humidity: canopy_air_humidity,
                    ground_specific_humidity: input.ground_specific_humidity,
                    reference_specific_humidity: input.reference_specific_humidity,
                    leaf_temperature_k: state.leaf_temperature_k,
                    leaf_boundary_resistance_s_m: leaf_boundary_resistance,
                    soil_surface_resistance_s_m: input.soil_surface_resistance_s_m,
                    reference_to_canopy_moisture_resistance_s_m: raw,
                    ground_to_canopy_moisture_resistance_s_m: ground_to_canopy_resistance,
                    air_density_kg_m3: input.air_density_kg_m3,
                    wet_canopy_fraction: fwet,
                    sunlit_leaf_area_index: laisun,
                    shaded_leaf_area_index: laisha,
                    stem_area_index: sai.max(0.1),
                    canopy_top_height_m: input.canopy_top_height_m,
                    maximum_sunlit_leaf_conductance_umol_m2_s,
                    maximum_shaded_leaf_conductance_umol_m2_s,
                    maximum_sunlit_leaf_hydraulic_conductance: hydraulic
                        .maximum_sunlit_leaf_hydraulic_conductance,
                    maximum_shaded_leaf_hydraulic_conductance: hydraulic
                        .maximum_shaded_leaf_hydraulic_conductance,
                    maximum_xylem_hydraulic_conductance: hydraulic
                        .maximum_xylem_hydraulic_conductance,
                    maximum_root_hydraulic_conductance: hydraulic
                        .maximum_root_hydraulic_conductance,
                    sunlit_leaf_psi50_mm: hydraulic.sunlit_leaf_psi50_mm,
                    shaded_leaf_psi50_mm: hydraulic.shaded_leaf_psi50_mm,
                    xylem_psi50_mm: hydraulic.xylem_psi50_mm,
                    root_psi50_mm: hydraulic.root_psi50_mm,
                    vulnerability_shape: hydraulic.vulnerability_shape,
                    soil_surface_resistance_scheme: hydraulic.soil_surface_resistance_scheme,
                    parameters: hydraulic.parameters,
                },
                hydraulic_state,
            )?;
            ensure!(
                (hydraulic_output.sunlit_transpiration_kg_m2_s
                    + hydraulic_output.shaded_transpiration_kg_m2_s
                    - hydraulic_output.root_flux_kg_m2_s.iter().sum::<f64>())
                .abs()
                    <= 1.0e-7,
                "plant hydraulic solve violates its root-water balance"
            );
            sunlit_resistance = hydraulic_stomatal_resistance(
                input,
                StomataStep {
                    leaf_temperature_k: state.leaf_temperature_k,
                    leaf_boundary_resistance_s_m: leaf_boundary_resistance,
                    absorbed_par_w_m2: input.sunlit_absorbed_par_w_m2,
                    soil_water_stress: hydraulic_output.sunlit_stress,
                    canopy_integration: cintsun,
                    canopy_air_co2_pa: canopy_air_co2,
                    canopy_vapor_pressure_pa: canopy_vapor_pressure,
                    leaf_vapor_pressure_pa: leaf_saturation.vapor_pressure_pa,
                },
                hydraulic_output.sunlit_stomatal_conductance_umol_m2_s * laisun,
            )?;
            shaded_resistance = hydraulic_stomatal_resistance(
                input,
                StomataStep {
                    leaf_temperature_k: state.leaf_temperature_k,
                    leaf_boundary_resistance_s_m: leaf_boundary_resistance,
                    absorbed_par_w_m2: input.shaded_absorbed_par_w_m2,
                    soil_water_stress: hydraulic_output.shaded_stress,
                    canopy_integration: cintsha,
                    canopy_air_co2_pa: canopy_air_co2,
                    canopy_vapor_pressure_pa: canopy_vapor_pressure,
                    leaf_vapor_pressure_pa: leaf_saturation.vapor_pressure_pa,
                },
                hydraulic_output.shaded_stomatal_conductance_umol_m2_s * laisha,
            )?;
            root_flux_kg_m2_s = hydraulic_output.root_flux_kg_m2_s;
            sunlit_soil_water_stress = hydraulic_output.sunlit_stress;
            shaded_soil_water_stress = hydraulic_output.shaded_stress;
        }
        let leaf_sunlit_resistance = sunlit_resistance.stomatal_resistance_s_m * laisun;
        let leaf_shaded_resistance = shaded_resistance.stomatal_resistance_s_m * laisha;
        let evaporation_sign = if leaf_saturation.specific_humidity > canopy_air_humidity {
            1.0
        } else {
            0.0
        };
        let canopy_air_heat_conductance = 1.0 / rah;
        let ground_heat_conductance = 1.0 / ground_to_canopy_resistance;
        let leaf_heat_conductance = lsai / leaf_boundary_resistance;
        let canopy_air_moisture_conductance = 1.0 / raw;
        let ground_moisture_conductance = if input.ground_specific_humidity < canopy_air_humidity {
            1.0 / ground_to_canopy_resistance
        } else if input.options.soil_resistance_is_conductance {
            input.soil_surface_resistance_s_m / ground_to_canopy_resistance
        } else {
            1.0 / (ground_to_canopy_resistance + input.soil_surface_resistance_s_m)
        };
        // 上游的 `wet_area_cfw = lai + sai` / `wet_cond_cfw = wet_area_cfw / rb`
        // （`MOD_LeafTemperature_Extended.F90:972-973`）—— 与 `cfh`、`evplwet` 共用的
        // 那个"湿面积导度"。**必须先把 `lsai/rb` 算成一项**：
        // `cfw` 的汇合点（`lt_ext.s` 的 `L166`）是
        //   `_382 = (dry_factor*delta) * SUM`
        //   `cfw  = fma(coef, wet_cond_cfw, _382)`   ← 融合的是**前一项**
        // 而原先写成 `coef * lsai / rb + …`，既是"先乘后除"（与 `cfh` 的写法不一致），
        // 加法那一侧也没有收缩。
        let wet_conductance = lsai / leaf_boundary_resistance;
        let dry_factor = 1.0 - fwet;
        let leaf_moisture_transfer_sum = laisun
            / (leaf_boundary_resistance + leaf_sunlit_resistance)
            + laisha / (leaf_boundary_resistance + leaf_shaded_resistance);
        let leaf_moisture_conductance = (1.0 - evaporation_sign * dry_factor).mul_add(
            wet_conductance,
            dry_factor * evaporation_sign * leaf_moisture_transfer_sum,
        );
        let heat_weight =
            1.0 / (canopy_air_heat_conductance + ground_heat_conductance + leaf_heat_conductance);
        let moisture_weight = 1.0
            / (canopy_air_moisture_conductance
                + ground_moisture_conductance
                + leaf_moisture_conductance);
        let air_heat_weight = canopy_air_heat_conductance * heat_weight;
        let ground_heat_weight = ground_heat_conductance * heat_weight;
        let leaf_heat_weight = leaf_heat_conductance * heat_weight;
        let air_moisture_weight = canopy_air_moisture_conductance * moisture_weight;
        let ground_moisture_weight = ground_moisture_conductance * moisture_weight;
        let leaf_moisture_weight = leaf_moisture_conductance * moisture_weight;
        let longwave_factor = 1.0 - input.canopy_longwave_gap_fraction;
        let (net_longwave, net_longwave_temperature_slope) =
            longwave(input, state.leaf_temperature_k, longwave_factor);
        // `fsenl = rhoair*cpair*cfh*( (wta0+wtg0)*tl - wta0*thm - wtg0*tg )`
        // （`MOD_LeafTemperature_Extended.F90:1120`）。括号里是三级减法链
        // `((W*T) - a*A) - b*B`，gfortran **每一级都吸收掉那个乘积**：
        // 实测 `fma(-b,B, fma(W,T, -(a*A)))` 与内核 4000/4000 组逐位相同，
        // 而不收缩的写法只有 345/4000 —— 这个形状每轮迭代出现 5 次
        // （`fsenl`/`etr`/`etrsun`/`etrsha`/`evplwet`），是 `dtl` 的直接输入。
        // 括号 `( (wta0 + wtg0)*tl - wta0*thm - wtg0*tg )` 的**逐位**形状（`lt_ext.s`
        // 的 `.loc 1 1116`）：
        //   `_459 = wta0 + wtg0`
        //   `_461 = thm * wta0`（`fmul`，**先舍入**）
        //   `tmp1960 = fmsub(_459, tl, _461)` → `_459*tl - _461`，**只有 `_459*tl` 融合**
        //   `_463 = tg * wtg0`（`fmul`，**先舍入**）
        //   `_464 = tmp1960 - _463`
        // 所以只有中间那个乘积是 fma，`wta0*thm` 与 `wtg0*tg` 都各自先舍入。
        // 原先把 `-wtg0*tg + X` 也写成 `mul_add`（依据是随机 4000 组的统计），
        // 逐迭代位型探针在第 1 轮就否掉了它：`fsenl` 差 1 ULP，而同一轮的
        // `fsenl_dtl`（无 fma 的 `rhoair*cpair*cfh*(wta0+wtg0)`）逐位相同。
        let heat_weight_sum = air_heat_weight + ground_heat_weight;
        let leaf_sensible_heat = input.air_density_kg_m3
            * AIR_HEAT_CAPACITY_J_KG_K
            * leaf_heat_conductance
            * (heat_weight_sum.mul_add(
                state.leaf_temperature_k,
                -(air_heat_weight * input.reference_air_temperature_k),
            ) - ground_heat_weight * input.ground_temperature_k);
        let leaf_sensible_temperature_slope = input.air_density_kg_m3
            * AIR_HEAT_CAPACITY_J_KG_K
            * leaf_heat_conductance
            * (air_heat_weight + ground_heat_weight);
        // 与 `fsenl` 同一个坑，`lt_ext.s` 的 `.loc 1 1125` 逐条：
        //   `_481 = wtaq0 * qm`（`fmul`，先舍入）
        //   `_479 = wtaq0 + wtgq0`
        //   `_483 = wtgq0 * qg`（`fmul`，先舍入）
        //   `tmp1963 = fnmsub(_479, qsatl, _481)` → `_479*qsatl - _481`，**只有它融合**
        //   `_485 = tmp1963 - _483`
        // 原先把 `-wtgq0*qg` 也写成 `mul_add`。第 1 轮迭代的位型探针在
        // `rssun` 修好之后只剩 `etr`/`fevpl` 差 1 ULP，而同一条语句里的
        // `etr_dtl`（不经过这个括号）逐位相同 —— 差别只可能在这个括号里。
        let moisture_weight_sum = air_moisture_weight + ground_moisture_weight;
        let humidity_gradient = moisture_weight_sum.mul_add(
            leaf_saturation.specific_humidity,
            -(air_moisture_weight * input.reference_specific_humidity),
        ) - ground_moisture_weight * input.ground_specific_humidity;
        let mut transpiration = input.air_density_kg_m3
            * (1.0 - fwet)
            * evaporation_sign
            * (laisun / (leaf_boundary_resistance + leaf_sunlit_resistance)
                + laisha / (leaf_boundary_resistance + leaf_shaded_resistance))
            * humidity_gradient;
        // `etrsun`/`etrsha` 的**结合顺序**与上游不同：上游是
        // `rhoair*dry_factor*delta*( laisun/(rb+rssun) )*( … )`
        // —— 先算 `laisun/(rb+rssun)` 再乘；原先写成 `… * laisun / (rb+rssun) * …`
        // 是**先乘后除**，两者差 1 ULP。`etr`（上面那条）上游本来就是
        // `( a/(…) + b/(…) )` 的整体因子，所以只有逐叶这两条要加括号。
        let mut sunlit_transpiration = input.air_density_kg_m3
            * (1.0 - fwet)
            * evaporation_sign
            * (laisun / (leaf_boundary_resistance + leaf_sunlit_resistance))
            * humidity_gradient;
        let mut shaded_transpiration = input.air_density_kg_m3
            * (1.0 - fwet)
            * evaporation_sign
            * (laisha / (leaf_boundary_resistance + leaf_shaded_resistance))
            * humidity_gradient;
        let mut transpiration_temperature_slope = input.air_density_kg_m3
            * (1.0 - fwet)
            * evaporation_sign
            * (laisun / (leaf_boundary_resistance + leaf_sunlit_resistance)
                + laisha / (leaf_boundary_resistance + leaf_shaded_resistance))
            * (air_moisture_weight + ground_moisture_weight)
            * leaf_saturation.specific_humidity_temperature_slope_k;
        // 上面这一组 `transpiration`/`sunlit_transpiration`/`shaded_transpiration`
        // 就是上游的 `etr`/`etrsun`/`etrsha`（`MOD_LeafTemperature_Extended.F90:1108-1118`）。
        // **PHS 分支不把它们换成 PHS 自己解出来的蒸腾量。** 上游那一支
        // （`:1120-1131`）只做两件事：按逐叶胁迫因子 `rstfacsun`/`rstfacsha`
        // 与符号把 `etrsun`/`etrsha` 清零，再把 `rootflux` 缩放去对上 `etr`
        // ——`balance_phs_rootflux` 的第一个参数 `etr` 是 **`intent(in)`**，
        // 一个字都不回写（`MOD_PHSRootfluxBalance.F90:23-40`）。PHS 对气孔的
        // 影响走的是 `rssun`/`rssha`（`:904` 由 `gssun`/`gssha` 反算），不是
        // 直接把"根供得起多少"当成"冠层蒸多少"。
        // 本仓库此前把它换成 `hydraulic_output.sunlit_transpiration + shaded`，
        // 等于把"根供得起多少"当成"冠层蒸多少"。**实测这三份窗口里两者贴得很近**：
        // 改回上游语义后总超差只从 13738/25873/31929 变成 13726/25872/31931
        // （`f_vegwp` 条数 747→735），所以它不是那些红条的量级来源 —— 改它是为了
        // 语义对齐，不是为了刷数字。真正把第一步叶温拉偏的是上游 `o3coefg_*`
        // 的 `spval`（见 docs/implementation-verification.md 对应小节）。
        // 根通量与 `etr` 的一致性由两处保证：这里 `balance_phs_rootflux`
        // （上游 `:1145` 的 `'post-PHS'`），以及循环后的按比例缩放
        // （上游 `:1352-1367`）。两者都已移植。
        if let Some(hydraulic) = input.plant_hydraulics {
            if sunlit_soil_water_stress < 1.0e-2 || sunlit_transpiration <= 0.0 {
                sunlit_transpiration = 0.0;
            }
            if shaded_soil_water_stress < 1.0e-2 || shaded_transpiration <= 0.0 {
                shaded_transpiration = 0.0;
            }
            transpiration = sunlit_transpiration + shaded_transpiration;
            balance_phs_rootflux(
                transpiration,
                &mut root_flux_kg_m2_s,
                hydraulic.root_fraction,
                "post-PHS",
            );
        } else if transpiration >= input.transpiration_limit_kg_m2_s {
            let scale = if transpiration > 0.0 {
                input.transpiration_limit_kg_m2_s / transpiration
            } else {
                0.0
            };
            transpiration = input.transpiration_limit_kg_m2_s;
            sunlit_transpiration *= scale;
            shaded_transpiration *= scale;
            transpiration_temperature_slope = 0.0;
        }
        // 上游 `evplwet = rhoair * evp_weight * wet_cond * ( … )`
        // （`:1150-1151`）—— `wet_cond = wet_area/rb` 是**先除好的一项**，
        // 不是"链尾再除 `rb`"。原先写成 `rhoair * coef * lsai / rb * X`，
        // 既是先乘后除、也与 `cfw` 那条不一致。**括号照抄上游的左结合**
        // （`… * (wtaq0+wtgq0)*qsatlDT` 是 `(…*(W))*qsatlDT`，不是 `…*(W*qsatlDT)`）。
        // 本条在 CN-Cng 干窗里恒等于 0（`evp_weight = 1-delta*(1-fwet)` 在
        // `fwet=0, delta=1` 时正好为 0），位型探针**测不到**它 —— 属于按上游源码
        // 形状照抄的保真度改动，由湿窗/雪窗黄金回归把关。
        let evaporation_weight = 1.0 - evaporation_sign * dry_factor;
        let mut wet_evaporation =
            input.air_density_kg_m3 * evaporation_weight * wet_conductance * humidity_gradient;
        let mut wet_evaporation_temperature_slope = input.air_density_kg_m3
            * evaporation_weight
            * wet_conductance
            * (air_moisture_weight + ground_moisture_weight)
            * leaf_saturation.specific_humidity_temperature_slope_k;
        if wet_evaporation >= state.canopy_water.total_mm / input.time_step_seconds {
            wet_evaporation = state.canopy_water.total_mm / input.time_step_seconds;
            wet_evaporation_temperature_slope = 0.0;
        }
        let leaf_evaporation_unadjusted = transpiration + wet_evaporation;
        let mut leaf_evaporation = leaf_evaporation_unadjusted;
        let leaf_evaporation_temperature_slope =
            transpiration_temperature_slope + wet_evaporation_temperature_slope;
        let mut evaporation_imbalance = 0.0;
        if leaf_evaporation * prior_leaf_evaporation < 0.0 {
            evaporation_imbalance = -0.9 * leaf_evaporation;
            leaf_evaporation *= 0.1;
        }
        // `dtl(it) = (…)/(…)`（`MOD_LeafTemperature_Extended.F90:1188-1196`）——
        // **这就是准 Newton 的步长、也就是退出判据 `|dtl| < 0.01` 里的那个量**，
        // 每差 1 ULP 都可能让迭代次数差一次（PHS 的 `vegwp` 因此整层偏移）。
        //
        // 分子和分母的收缩形状**不一样**，而且必须先看内核汇编再写（下面紧邻的
        // 注释给出逐条 `_NNN`）。曾经据独立差分驱动器的 4000 组统计把两边都写成
        // 全融合，方向是错的 —— 驱动器里没有分母共享乘积这个共同子表达式，
        // 编译器于是替它做了另一种收缩选择。这条教训与第 166/167 轮同源。
        let precipitation_temperature_difference =
            input.precipitation_temperature_k - state.leaf_temperature_k;
        // 分子/分母共用 `cpliq*max(0,qintr_rain)` 与 `cpice*max(0,qintr_snow)` 这两个
        // 乘积 —— 正是这一点决定了 gfortran 的收缩选择：**两个乘积必须先各自
        // 舍入成一项**（它们在分母里也要用，共同子表达式被 CSE 掉之后，
        // `x*y + z` 里的那个乘积已经存在，后一个加法无从收缩）。实测
        // `extends/interception/MOD_LeafTemperature_Extended.F90` 编出来的汇编
        // （`gfortran -S -O2` 的生产 flag）里，分母是**整条平铺**：
        //   `_537 = clai/deltim`、`_538 = _537 - dirab_dtl`、`_539 = _538 + fsenl_dtl`
        //   `_540 = htvpl*fevpl_dtl`(fmul)、`_541 = _539 + _540`(fadd)
        //   `_526 = cpliq*rain`(fmul)、`_533 = cpice*snow`(fmul，与分子共用)
        //   `_542 = _526 + _541`(fadd)、`_543 = _542 + _533`(fadd)
        // 而分子那两处加法**是**收缩的（`fmadd(_526,_530,_525)`、
        // `fmadd(_530,_533,…)`）—— 注意收缩的是**外层**的 `(cpliq*rain)*(t_precip-tl)`，
        // 内层的 `cpliq*rain` 仍然是先算好的那一项。
        //
        // 先前这里写成三层 `mul_add` 嵌套（分母全融合、分子把 `cpliq*rain` 也融进去），
        // 依据是独立差分驱动器的 3988/4000 —— 但驱动器里没有分母共享那个乘积，
        // 编译器于是做出了**不同的**收缩选择，属于本仓库记过的
        // "独立编译的 dump 不能代表内核二进制"（第 166/167 轮）。以汇编为准。
        let water_heat_capacity_flux = WATER_HEAT_CAPACITY_J_KG_K * intercepted_rain;
        let ice_heat_capacity_flux = ICE_HEAT_CAPACITY_J_KG_K * intercepted_snow;
        let denominator = ((clai / input.time_step_seconds - net_longwave_temperature_slope
            + leaf_sensible_temperature_slope)
            + leaf_latent_heat_j_kg * leaf_evaporation_temperature_slope)
            + water_heat_capacity_flux
            + ice_heat_capacity_flux;
        ensure!(
            denominator.is_finite() && denominator != 0.0,
            "leaf energy denominator is invalid"
        );
        let numerator = ice_heat_capacity_flux.mul_add(
            precipitation_temperature_difference,
            water_heat_capacity_flux.mul_add(
                precipitation_temperature_difference,
                (-leaf_latent_heat_j_kg).mul_add(
                    leaf_evaporation,
                    input.canopy_absorbed_solar_w_m2 + net_longwave - leaf_sensible_heat,
                ),
            ),
        );
        dtl[iteration] = numerator / denominator;
        let unbounded_temperature_change = dtl[iteration];
        if dtl[iteration].abs() > MAX_TEMPERATURE_STEP_K {
            // 上游 `:1203` 是 `dtl(it) = delmax*dtl(it)/abs(dtl(it))` —— **不是**
            // `delmax*sign(dtl)`：先乘、再除，两次舍入。用 `signum()` 会得到**恰好**
            // ±3.0，而上游在 `|dtl|` 比 3 大 1 ULP 时给的是 `-2.9999999999999996`。
            // 逐迭代位型探针在 it=5 抓到：
            //   `dtl` kernel=C007FFFFFFFFFFFF(=-(3-1ulp))  rust=C008000000000000(=-3.0)
            // 前四轮迭代改对之后，这是第 5 轮唯一的源头（`del`/`dele` 是它的下游）。
            dtl[iteration] = MAX_TEMPERATURE_STEP_K * dtl[iteration] / dtl[iteration].abs();
        }
        if iteration >= 2 && dtl[iteration - 1] * dtl[iteration] <= 0.0 {
            dtl[iteration] = 0.5 * (dtl[iteration - 1] + dtl[iteration]);
        }
        state.leaf_temperature_k = previous_leaf_temperature + dtl[iteration];
        let temperature_change = dtl[iteration].abs();
        // `dele = dtl*dtl*( dirab_dtl**2 + fsenl_dtl**2 + (hvap*fevpl_dtl)**2 )`
        // （`MOD_LeafTemperature_Extended.F90:1291` 上面那两行）。**这就是收敛判据本身**
        // —— `dee` 卡在 `dlemin = 0.1` 上，差 1 ULP 就会让迭代次数差一次。里层是
        // 三项平方链，按实测规则收缩成 `fma(C,C, fma(A,A, B*B))`：
        // 4000 组逐位全同，不收缩只有 3112/4000。`temperature_change` 那边
        // 上游写的是 `sqrt(dtl*dtl)`，但 `dtl` 是叶温增量（`delmax` 限幅），
        // `sqrt(x*x)` 与 `|x|` 在 4e6 组随机位型里只在 `x*x` 上溢时有别，等价。
        let latent_flux_slope = leaf_latent_heat_j_kg * leaf_evaporation_temperature_slope;
        let flux_change = (dtl[iteration].powi(2)
            * latent_flux_slope.mul_add(
                latent_flux_slope,
                net_longwave_temperature_slope.mul_add(
                    net_longwave_temperature_slope,
                    leaf_sensible_temperature_slope * leaf_sensible_temperature_slope,
                ),
            ))
        .sqrt();
        let updated_saturation =
            saturation_specific_humidity(state.leaf_temperature_k, input.surface_pressure_pa)?;
        // 下面三处 `mul_add` 是**照抄 gfortran 的收缩**（内核 `-O2` 下
        // `-ffp-contract=fast` 是默认，Rust 不自动收缩），不是优化。
        //
        // `taf = wta0*thm + wtg0*tg + wtl0*tl` / `qaf = wtaq0*qm + wtgq0*qg +
        // wtlq0*qsatl` 这种三项乘积链，实测收缩成
        // `fma(w2,v2, fma(w0,v0, w1*v1))`：4000 组随机输入里逐位全同，
        // 而不收缩的写法只有 2580/4000 相同 —— 也就是说**每轮迭代都有约 1/3 的
        // 概率差 1 ULP**，而 `qaf` 直接经 `eah` 进 `stomata`。这正是叶温迭代
        // 正反馈（`pco2a ↔ assim`）的入口那一侧。
        // `taf = wta0*thm + wtg0*tg + wtl0*tl` 与 `qaf = wtaq0*qm + wtgq0*qg +
        // wtlq0*qsatl` 是**两条形状不同**的语句，不能一起照抄（原先按"4000 组里
        // 收缩成 fma(w2,v2, fma(w0,v0, w1*v1))"把两条都写成两层 mul_add）。
        // `lt_ext.s` 的 `.loc 1 1233/1234` 逐条：
        //   `_461 = thm*wta0`、`_463 = tg*wtg0`（**二者都是复用 `fsenl` 里算好的**）
        //   `_564 = _461 + _463`（fadd）、`taf = fmadd(tl, wtl0, _564)`
        //   —— 只有 `wtl0*tl` 融合；
        //   `_481 = wtaq0*qm`、`_483 = wtgq0*qg`（复用 `etr` 的湿度括号）
        //   `_567 = _481 + _483`（fadd）、`_569 = qsatl*wtlq0`（fmul）
        //   `_570 = _567 + _569`（fadd）—— **整条平铺，一个都没融**。
        // 差别来自"哪些乘积已经被别处算过"：`taf` 的 `tl*wtl0` 是新鲜的所以被吸收，
        // `qaf` 的 `qsatl*wtlq0` 也是新鲜的却没有 —— 这是 GCC 自己的选择，以汇编为准。
        canopy_air_temperature = leaf_heat_weight.mul_add(
            state.leaf_temperature_k,
            air_heat_weight * input.reference_air_temperature_k
                + ground_heat_weight * input.ground_temperature_k,
        );
        canopy_air_humidity = leaf_moisture_weight * updated_saturation.specific_humidity
            + (air_moisture_weight * input.reference_specific_humidity
                + ground_moisture_weight * input.ground_specific_humidity);
        let pressure_conversion = 44.6 * 273.16 * input.surface_pressure_pa / 1.013e5;
        let air_conductance = 1.0 / raw * pressure_conversion / input.reference_air_temperature_k;
        // `:652` 的 `rsoil = 0.22 * 1.e-6` 是**两个字面量的乘积**，不是 `0.22e-6`：
        //   内核 `fl(0.22)*fl(1e-6)` 再舍一次 = `3E8D87247702C0CF`
        //   （GIMPLE 里就是 `_582 = _581 - 2.1999999999999998475...e-7`，位型 `…C0CF`）
        //   而 Rust 原先的 `0.22e-6` = 正确舍入的十进制 = `3E8D87247702C0D0`，**大 1 ULP**。
        // 它就减在 `pco2a` 的括号和里 ⇒ 第 351 轮的 hex 探针量到的 `pco2a` 1 ULP 分叉
        // （20/1004 次调用）有一部分来自这里。
        let soil_respiration = 0.22 * 1.0e-6;
        // `MOD_LeafTemperature_Extended.F90:1244` 的 GIMPLE 是
        //   `_585 = .FNMA(1.37*psrf/max(0.446,gah2o), 括号和, pco2m)`
        // —— 那条"抽走率 × 括号和"的乘积**被收进减法**（一次舍入），平铺会多舍一次。
        let co2_drawdown_rate = 1.37 * input.surface_pressure_pa / air_conductance.max(0.446);
        let co2_sink = sunlit_resistance.assimilation_mol_m2_s
            + shaded_resistance.assimilation_mol_m2_s
            - sunlit_resistance.respiration_mol_m2_s
            - shaded_resistance.respiration_mol_m2_s
            - soil_respiration;
        canopy_air_co2 = (-co2_drawdown_rate).mul_add(co2_sink, input.atmospheric_co2_pa);
        temperature_difference = input.reference_air_temperature_k - canopy_air_temperature;
        humidity_difference = input.reference_specific_humidity - canopy_air_humidity;
        let temperature_scale =
            VON_KARMAN / (surface.heat - profile.heat_at_top_layer) * temperature_difference;
        let humidity_scale =
            VON_KARMAN / (surface.moisture - profile.moisture_at_top_layer) * humidity_difference;
        // 与循环外那个 `dthv` 共用同一个 `(1.+0.61*qm)`（内核 CSE 成一项），
        // 两个乘积各自融合 —— 见函数上方 `one_plus_0_61_reference_humidity` 的注释。
        let virtual_temperature_scale = temperature_scale.mul_add(
            one_plus_0_61_reference_humidity,
            (0.61 * input.potential_temperature_k) * humidity_scale,
        );
        let mut zeta = reference_height * VON_KARMAN * GRAVITY_M_S2 * virtual_temperature_scale
            / (surface.friction_velocity_m_s.powi(2) * input.virtual_potential_temperature_k);
        zeta = if zeta >= 0.0 {
            zeta.clamp(1.0e-6, 2.0)
        } else {
            zeta.clamp(-100.0, -1.0e-6)
        };
        obukhov = reference_height / zeta;
        stability_wind = if zeta >= 0.0 {
            reference_wind.max(0.1)
        } else {
            let boundary_height = match input.options.surface_layer_scheme {
                SurfaceLayerScheme::Standard => 1000.0,
                SurfaceLayerScheme::LargeEddy => {
                    (5.0 * wind_height).max(input.boundary_layer_height_m.context(
                        "the large-eddy surface-layer scheme needs the forcing's boundary-layer \
                         height (forc_hpbl), which this case does not provide",
                    )?)
                }
            };
            let convective_velocity = (-GRAVITY_M_S2
                * surface.friction_velocity_m_s
                * virtual_temperature_scale
                * boundary_height
                / input.virtual_potential_temperature_k)
                .powf(1.0 / 3.0);
            // 上游 `:1272-1273` 是 `wc2 = beta*beta*(wc*wc)`、`um = sqrt(ur*ur+wc2)`，
            // 内核编出来是（`.loc 1 1273`）：
            //   `_613 = wc*wc`（`fmul`）
            //   `tmp2112 = fmadd(ur, ur, _613)`   ← `ur*ur` **是融合的**
            //   `um = sqrt(tmp2112)`
            // `beta = 1.` 且 `zii = 1000.`（`:604-605`），所以 Rust 侧省掉 beta 是对的。
            // 原先写成平铺的 `ur.powi(2) + wc.powi(2)`，在 it=5 让 `um` 差 1 ULP ——
            // `um` 不在当时那 34 个探针量里，于是它到 **it=6** 才以 `ustar` 的形式暴露。
            reference_wind
                .mul_add(reference_wind, convective_velocity * convective_velocity)
                .sqrt()
        };
        if prior_obukhov * obukhov < 0.0 {
            obukhov_sign_changes += 1;
        }
        if obukhov_sign_changes >= 4 {
            obukhov = reference_height / -0.01;
        }
        prior_obukhov = obukhov;
        last = Iteration {
            ram,
            raw,
            surface,
            top_heat: profile.heat_at_top_layer,
            top_moisture: profile.moisture_at_top_layer,
            heat_at_2m: surface.heat_at_2m,
            moisture_at_2m: surface.moisture_at_2m,
            zeta,
            leaf_sensible_heat,
            leaf_sensible_temperature_slope,
            leaf_evaporation_unadjusted,
            leaf_evaporation_temperature_slope,
            transpiration,
            transpiration_temperature_slope,
            sunlit_transpiration,
            shaded_transpiration,
            wet_evaporation,
            wet_evaporation_temperature_slope,
            net_longwave,
            net_longwave_temperature_slope,
            unbounded_temperature_change,
            evaporation_imbalance,
            canopy_air_temperature,
            canopy_air_humidity,
            air_heat_weight,
            ground_heat_weight,
            leaf_heat_weight,
            air_moisture_weight,
            ground_moisture_weight,
            leaf_moisture_weight,
            ground_heat_conductance,
            ground_moisture_conductance,
            sunlit_resistance,
            shaded_resistance,
            leaf_sunlit_resistance,
            leaf_shaded_resistance,
            root_flux_kg_m2_s,
            sunlit_soil_water_stress,
            shaded_soil_water_stress,
            maximum_sunlit_leaf_conductance_umol_m2_s: gs0sun,
            maximum_shaded_leaf_conductance_umol_m2_s: gs0sha,
        };
        iteration += 1;
        if iteration > MIN_ITERATIONS {
            prior_leaf_evaporation = leaf_evaporation;
            if temperature_change.max(prior_temperature_change) < TEMPERATURE_TOLERANCE_K
                && flux_change.max(prior_flux_change) < FLUX_TOLERANCE_W_M2
            {
                break;
            }
        }
        prior_temperature_change = temperature_change;
        prior_flux_change = flux_change;
    }

    let final_temperature_change = dtl[iteration - 1];
    // 循环后这三处（`fsenl` 的两项修正、`elwdif` 的显热补偿、`err` 残差）用的是
    // **最后一轮迭代算出的 `htvpl`**，而那一轮是按 `tlbef`（= 此刻的
    // `previous_leaf_temperature`）取值的 —— 上游的 `htvpl` 正是在更新 `tl`
    // **之前**算的（`:693-697` 在 `tl = tlbef + dtl(it)` 之前）。别拿最终的
    // `state.leaf_temperature_k` 重算：那会晚半个增量，在 `tfrz` 附近翻错相。
    let leaf_latent_heat_j_kg = if previous_leaf_temperature > FREEZING_K {
        LATENT_HEAT_VAPORIZATION_J_KG
    } else {
        LATENT_HEAT_SUBLIMATION_J_KG
    };
    // `MOD_LeafTemperature.F90:1062-1093` 的收敛后收尾。GIMPLE（dump 第 2636-2650、
    // 2720-2733 处）是一串**逐级 FMA**：
    //   `_552 = FMA(fsenl_dtl, dtl, fsenl旧值)`
    //   `_569 = FMA(dtl_noadj-dtl, 括号, _552)`
    //   `_571 = FMA(erre, hvap, _569)`
    //   `_595 = FMA(elwdif, hvap, _571)`
    //   `etr`/`evplwet`/`fevpl` 各是一处 `FMA(x_dtl, dtl, 旧值)`
    // —— 原先是平铺加法，这一串少了 7 次融合。
    let leaf_sensible_heat = {
        let slope_applied = last
            .leaf_sensible_temperature_slope
            .mul_add(final_temperature_change, last.leaf_sensible_heat);
        let bracket = clai / input.time_step_seconds - last.net_longwave_temperature_slope
            + last.leaf_sensible_temperature_slope
            + leaf_latent_heat_j_kg * last.leaf_evaporation_temperature_slope
            + WATER_HEAT_CAPACITY_J_KG_K * intercepted_rain
            + ICE_HEAT_CAPACITY_J_KG_K * intercepted_snow;
        let bracket_applied = (last.unbounded_temperature_change - final_temperature_change)
            .mul_add(bracket, slope_applied);
        leaf_latent_heat_j_kg.mul_add(last.evaporation_imbalance, bracket_applied)
    };
    let mut transpiration = last
        .transpiration_temperature_slope
        .mul_add(final_temperature_change, last.transpiration);
    let mut wet_evaporation = last
        .wet_evaporation_temperature_slope
        .mul_add(final_temperature_change, last.wet_evaporation);
    let leaf_evaporation = last
        .leaf_evaporation_temperature_slope
        .mul_add(final_temperature_change, last.leaf_evaporation_unadjusted);
    let wet_evaporation_limit = state.canopy_water.total_mm / input.time_step_seconds;
    let excessive_wet_evaporation = (wet_evaporation - wet_evaporation_limit).max(0.0);
    wet_evaporation = wet_evaporation.min(wet_evaporation_limit);
    let mut leaf_evaporation = leaf_evaporation - excessive_wet_evaporation;
    let mut leaf_sensible_heat =
        leaf_latent_heat_j_kg.mul_add(excessive_wet_evaporation, leaf_sensible_heat);
    // 残差必须**在这一步**算：上游 `:1449` 的 `err` 在冠层持水更新（`:1464`）之前，
    // 所以它吃的是 `elwdif` 修正之后的 `fsenl`/`fevpl`、**没有**吃下面那两笔退回，
    // 也没有吃相变段（`dheatl` 的 Niu(2004) 项、`tl` 拉回）—— 那两样都在它之后。
    // 移到更新之后会同时踩到这两点，`precipitation_heat` 还会用到已被相变改过的
    // `leaf_temperature_k`。
    let precipitation_heat = WATER_HEAT_CAPACITY_J_KG_K
        * intercepted_rain
        * (input.precipitation_temperature_k - state.leaf_temperature_k)
        + ICE_HEAT_CAPACITY_J_KG_K
            * intercepted_snow
            * (input.precipitation_temperature_k - state.leaf_temperature_k);
    let canopy_heat_storage = clai / input.time_step_seconds * final_temperature_change;
    let energy_balance_error = input.canopy_absorbed_solar_w_m2
        + last.net_longwave
        + last.net_longwave_temperature_slope * final_temperature_change
        - leaf_sensible_heat
        - leaf_latent_heat_j_kg * leaf_evaporation
        + precipitation_heat
        - canopy_heat_storage;
    let sunlit_transpiration = last.sunlit_transpiration;
    let shaded_transpiration = last.shaded_transpiration;
    let mut root_flux_kg_m2_s = last.root_flux_kg_m2_s;
    if let Some(hydraulic) = input.plant_hydraulics {
        if last.transpiration.abs() >= 1.0e-15 {
            // 上游是 `rootflux = rootflux * etr / etr0`
            // （`MOD_LeafTemperature_Extended.F90:1358`），左结合 —— **先乘后除**。
            // 抽成 `scale = etr/etr0` 再 `flux *= scale` 在浮点上不等价（差 1 ULP）：
            // CN-Cng 干窗口第 11 步的第 2、3 层就是这么偏出去的（第 274 轮探针：
            // `soil_water_vertical_movement` 入场时 `etroot` 只有这两层差 1 ULP，
            // 其和 `deficit` 跟着差，第 12 步水位才翻出去）。
            for flux in &mut root_flux_kg_m2_s {
                *flux = *flux * transpiration / last.transpiration;
            }
        } else {
            let total_depth = hydraulic.layer_thickness_m.iter().sum::<f64>();
            ensure!(
                total_depth.is_finite() && total_depth > 0.0,
                "plant hydraulic layer thickness has no positive total depth"
            );
            for (flux, depth) in root_flux_kg_m2_s
                .iter_mut()
                .zip(hydraulic.layer_thickness_m)
            {
                // 上游 `rootflux + dz_soi / sum(dz_soi) * etr_dtl * dtl(it-1)`
                // （`:1360`）也是左结合（`((dz/sum)*etr_dtl)*dtl`），且
                // `变量 + 乘积` 会被内核的 `-ffp-contract=fast` 收缩成 FMA。
                let rate = depth / total_depth * last.transpiration_temperature_slope;
                *flux = rate.mul_add(final_temperature_change, *flux);
            }
        }
        // `MOD_LeafTemperature_Extended.F90:1367` 的 `'post-leaf-temperature'`：
        // 缩放后再按冠层 `etr` 配平一次（默认三份窗口里每步都早退）。
        balance_phs_rootflux(
            transpiration,
            &mut root_flux_kg_m2_s,
            hydraulic.root_fraction,
            "post-leaf-temperature",
        );
    }
    state.canopy_water.total_mm =
        (state.canopy_water.total_mm - wet_evaporation * input.time_step_seconds).max(0.0);
    let (wet_snow_fraction, phase_flux_deficit, flux_deficit) =
        update_canopy_water(input, state, wet_evaporation)?;
    // `:1470-1474`（`bb203`）与 `:1485-1491`（`bb206`）：截留/相变把蒸发量削下来的
    // 部分要**原样退回** `fevpl`/`fsenl`，否则这些能量凭空消失。两处的出货形状不同，
    // 不能合并成一次减法：第一处是 `fevpl - pfd` 配 **`FMA(htvpl, pfd, fsenl)`**；
    // 第二处的感热项是 `fsenl + (htvpl*fd)/deltim` —— GIMPLE 里它是
    // `_892 = htvpl*flux_deficit; _893 = _892/_508; _894 = _891+_893`，**没融合**
    // （除法的结果不是乘积，`-ffp-contract=fast` 也收缩不了）；Rust 默认同样不融合。
    // `evplwet` 自己也要减这两笔，但它在 `:1489` 之后再没被读过 —— 是死存，
    // 所以这里不动 `wet_evaporation`。
    if phase_flux_deficit > 0.0 {
        leaf_evaporation -= phase_flux_deficit;
        leaf_sensible_heat = leaf_latent_heat_j_kg.mul_add(phase_flux_deficit, leaf_sensible_heat);
    }
    if flux_deficit > 0.0 {
        leaf_evaporation -= flux_deficit / input.time_step_seconds;
        leaf_sensible_heat += leaf_latent_heat_j_kg * flux_deficit / input.time_step_seconds;
    }
    let ground_sensible_heat = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * (input.ground_temperature_k - last.canopy_air_temperature);
    let soil_sensible_heat = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * ((1.0 - last.ground_heat_weight) * input.soil_surface_temperature_k
            - last.air_heat_weight * input.reference_air_temperature_k
            - last.leaf_heat_weight * state.leaf_temperature_k);
    let snow_sensible_heat = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * ((1.0 - last.ground_heat_weight) * input.snow_surface_temperature_k
            - last.air_heat_weight * input.reference_air_temperature_k
            - last.leaf_heat_weight * state.leaf_temperature_k);
    let ground_evaporation = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * (input.ground_specific_humidity - last.canopy_air_humidity);
    let soil_evaporation = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * ((1.0 - last.ground_moisture_weight) * input.soil_specific_humidity
            - last.air_moisture_weight * input.reference_specific_humidity
            - last.leaf_moisture_weight
                * saturation_specific_humidity(
                    state.leaf_temperature_k,
                    input.surface_pressure_pa,
                )?
                .specific_humidity);
    let snow_evaporation = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * ((1.0 - last.ground_moisture_weight) * input.snow_specific_humidity
            - last.air_moisture_weight * input.reference_specific_humidity
            - last.leaf_moisture_weight
                * saturation_specific_humidity(
                    state.leaf_temperature_k,
                    input.surface_pressure_pa,
                )?
                .specific_humidity);
    // `MOD_LeafTemperature_Extended.F90:1406-1407` 的 GIMPLE（census dump 第 4386-4394 处）：
    //   _761 = fac*stefnc ; powmult_1033 = t*t ; powmult_1034 = powmult_1033*t
    //   _763 = _761 * powmult_1034            ← `stefnc*fac*tlbef**3` 左结合
    //   _765 = .FMA(dtl, 4.0, tlbef)          ← `tlbef + 4.*dtl` 那一乘被吸收
    //   _767 = _763 * _765
    //   _768 = .FMA(_389, pretmp_3195, _767)  ← `_389 = thermk`（1098 处读的）、
    //                                           `pretmp_3195 = frl`
    // 所以**被吸收的是 `thermk*frl` 那一项，叶发射项 `_767` 是加数** —— 方向与直觉相反。
    //
    // 注意 `thermk` 是**冠层缝隙率**（`MOD_LeafTemperature_Extended.F90:203` 的声明注释
    // 就写着 "canopy gap fraction for tir radiation"），`fac = 1 - thermk` 才是叶发射权重
    // （`:1098`）。第一版改动把两边的权重对调了，干窗 16 步的 restart 立刻 31/68 分歧
    // （对照：未改是 0/68）—— 权重对调是量级错误，不是舍入差。
    // `4.*dtl` 那一乘按 2 的幂（4.0）所以熔不熔逐位等价，但仍然照 GIMPLE 写上。
    let leaf_emission = STEFAN_BOLTZMANN
        * (1.0 - input.canopy_longwave_gap_fraction)
        * previous_leaf_temperature.powi(3)
        * 4.0f64.mul_add(final_temperature_change, previous_leaf_temperature);
    let downward_longwave = input
        .canopy_longwave_gap_fraction
        .mul_add(input.atmospheric_longwave_w_m2, leaf_emission);
    let upward_longwave = upward_longwave(
        input,
        previous_leaf_temperature,
        final_temperature_change,
        1.0 - input.canopy_longwave_gap_fraction,
    );
    let canopy_stomatal_resistance =
        1.0 / (laisun / last.leaf_sunlit_resistance + laisha / last.leaf_shaded_resistance);
    let pressure_conversion = 44.6 * 273.16 * input.surface_pressure_pa / 1.013e5;
    // `MOD_LeafTemperature.F90:1040` 是 `gssun = (laisun/rssun) * (tprcor/tlbef)`
    // —— **右边的除法先算**。平铺成 `a/r * c / t` 会算成 `((a/r)*c)/t`，
    // 在 `f_gssun`/`f_gssha` 上留下 ~1 ULP 的第 0 步种子（2026 年 `window_divergence.py`
    // 量到 `f_gssun` 的 maxrel ≈ 2.95e-16，正是 1 ULP 的签名）。
    let resistance_conversion = pressure_conversion / previous_leaf_temperature;
    let sunlit_stomatal_conductance = laisun / last.leaf_sunlit_resistance * resistance_conversion;
    let shaded_stomatal_conductance = laisha / last.leaf_shaded_resistance * resistance_conversion;
    let bulk_richardson = (last.zeta * last.surface.friction_velocity_m_s.powi(2)
        / (VON_KARMAN.powi(2) / last.surface.heat * stability_wind.powi(2)))
    .min(5.0);
    // `MOD_LeafTemperature_Extended.F90:1435-1437`：
    //   `cgrnds = cpair*rhoair*cgh*(1.-wtg0)`
    //   `cgrndl = rhoair*cgw*(1.-wtgq0)*dqgdT`
    //   `cgrnd  = cgrnds + cgrndl*htvp`      ← 最后这个乘积被收进加法
    // `cgrndl` 在上游是**先算好、舍入过**的独立变量，所以这里也必须先落成局部量，
    // 再把 `cgrndl*htvp` 收进加法；直接写成一条平铺长链会多一次舍入。
    let ground_sensible_slope_w_m2_k = AIR_HEAT_CAPACITY_J_KG_K
        * input.air_density_kg_m3
        * last.ground_heat_conductance
        * (1.0 - last.ground_heat_weight);
    let ground_latent_slope_kg_m2_s_k = input.air_density_kg_m3
        * last.ground_moisture_conductance
        * (1.0 - last.ground_moisture_weight)
        * input.ground_humidity_temperature_slope_k;
    let ground_flux_temperature_slope_w_m2_k = ground_latent_slope_kg_m2_s_k
        .mul_add(input.ground_latent_heat_j_kg, ground_sensible_slope_w_m2_k);
    transpiration = transpiration.max(0.0);
    Ok(LeafTemperatureOutput {
        ground_latent_heat_j_kg: input.ground_latent_heat_j_kg,
        leaf_latent_heat_j_kg,
        sunlit_leaf_area_index: laisun,
        shaded_leaf_area_index: laisha,
        wet_snow_fraction,
        eastward_stress_kg_m_s2: -input.air_density_kg_m3 * input.eastward_wind_m_s / last.ram,
        northward_stress_kg_m_s2: -input.air_density_kg_m3 * input.northward_wind_m_s / last.ram,
        ground_sensible_heat_w_m2: ground_sensible_heat,
        soil_sensible_heat_w_m2: soil_sensible_heat,
        snow_sensible_heat_w_m2: snow_sensible_heat,
        ground_evaporation_kg_m2_s: ground_evaporation,
        soil_evaporation_kg_m2_s: soil_evaporation,
        snow_evaporation_kg_m2_s: snow_evaporation,
        ground_flux_temperature_slope_w_m2_k,
        ground_sensible_temperature_slope_w_m2_k: AIR_HEAT_CAPACITY_J_KG_K
            * input.air_density_kg_m3
            * last.ground_heat_conductance
            * (1.0 - last.ground_heat_weight),
        ground_latent_temperature_slope_kg_m2_s_k: input.air_density_kg_m3
            * last.ground_moisture_conductance
            * (1.0 - last.ground_moisture_weight)
            * input.ground_humidity_temperature_slope_k,
        // `MOD_LeafTemperature.F90:1271-1272` 的 `tref`/`qref`：GIMPLE（dump 第
        // 3169/3181 处）是 `_825 = dth*fl(vonkar/(fh-fht))`、
        // `_832 = FMA(_825, fh2m/vonkar-fh/vonkar, thm)` —— 左边那个乘积被吸收、
        // `thm`/`qm` 是已舍入的加数。原先是平铺加法，少一次融合。
        air_temperature_2m_k: (VON_KARMAN / (last.surface.heat - last.top_heat)
            * temperature_difference)
            .mul_add(
                last.heat_at_2m / VON_KARMAN - last.surface.heat / VON_KARMAN,
                input.reference_air_temperature_k,
            ),
        air_specific_humidity_2m: (VON_KARMAN / (last.surface.moisture - last.top_moisture)
            * humidity_difference)
            .mul_add(
                last.moisture_at_2m / VON_KARMAN - last.surface.moisture / VON_KARMAN,
                input.reference_specific_humidity,
            ),
        canopy_stomatal_resistance_s_m: canopy_stomatal_resistance,
        sunlit_stomatal_conductance_mol_m2_s: sunlit_stomatal_conductance,
        shaded_stomatal_conductance_mol_m2_s: shaded_stomatal_conductance,
        assimilation_mol_m2_s: last.sunlit_resistance.assimilation_mol_m2_s
            + last.shaded_resistance.assimilation_mol_m2_s,
        respiration_mol_m2_s: last.sunlit_resistance.respiration_mol_m2_s
            + last.shaded_resistance.respiration_mol_m2_s,
        leaf_sensible_heat_w_m2: leaf_sensible_heat,
        leaf_evaporation_kg_m2_s: leaf_evaporation,
        transpiration_kg_m2_s: transpiration,
        sunlit_transpiration_kg_m2_s: sunlit_transpiration,
        shaded_transpiration_kg_m2_s: shaded_transpiration,
        root_flux_kg_m2_s,
        sunlit_soil_water_stress: last.sunlit_soil_water_stress,
        shaded_soil_water_stress: last.shaded_soil_water_stress,
        maximum_sunlit_leaf_conductance_umol_m2_s: last.maximum_sunlit_leaf_conductance_umol_m2_s,
        maximum_shaded_leaf_conductance_umol_m2_s: last.maximum_shaded_leaf_conductance_umol_m2_s,
        sunlit_assimilation_mol_m2_s: last.sunlit_resistance.assimilation_mol_m2_s,
        shaded_assimilation_mol_m2_s: last.shaded_resistance.assimilation_mol_m2_s,
        downward_longwave_w_m2: downward_longwave,
        upward_longwave_w_m2: upward_longwave,
        precipitation_heat_w_m2: precipitation_heat,
        canopy_heat_storage_w_m2: canopy_heat_storage,
        momentum_roughness_m: z0mv,
        zol: last.zeta,
        bulk_richardson,
        friction_velocity_m_s: last.surface.friction_velocity_m_s,
        humidity_scale: VON_KARMAN / (last.surface.moisture - last.top_moisture)
            * humidity_difference,
        temperature_scale_k: VON_KARMAN / (last.surface.heat - last.top_heat)
            * temperature_difference,
        momentum_similarity: last.surface.momentum,
        heat_similarity: last.surface.heat,
        moisture_similarity: last.surface.moisture,
        reference_to_canopy_moisture_resistance_s_m: last.raw.max(0.0),
        energy_balance_error_w_m2: energy_balance_error,
        iterations: iteration - 1,
    })
}

#[derive(Debug, Clone)]
struct Iteration {
    ram: f64,
    raw: f64,
    surface: crate::MoninObukhovState,
    top_heat: f64,
    top_moisture: f64,
    heat_at_2m: f64,
    moisture_at_2m: f64,
    zeta: f64,
    leaf_sensible_heat: f64,
    leaf_sensible_temperature_slope: f64,
    leaf_evaporation_unadjusted: f64,
    leaf_evaporation_temperature_slope: f64,
    transpiration: f64,
    transpiration_temperature_slope: f64,
    sunlit_transpiration: f64,
    shaded_transpiration: f64,
    wet_evaporation: f64,
    wet_evaporation_temperature_slope: f64,
    net_longwave: f64,
    net_longwave_temperature_slope: f64,
    unbounded_temperature_change: f64,
    evaporation_imbalance: f64,
    canopy_air_temperature: f64,
    canopy_air_humidity: f64,
    air_heat_weight: f64,
    ground_heat_weight: f64,
    leaf_heat_weight: f64,
    air_moisture_weight: f64,
    ground_moisture_weight: f64,
    leaf_moisture_weight: f64,
    ground_heat_conductance: f64,
    ground_moisture_conductance: f64,
    sunlit_resistance: crate::StomataState,
    shaded_resistance: crate::StomataState,
    leaf_sunlit_resistance: f64,
    leaf_shaded_resistance: f64,
    root_flux_kg_m2_s: Vec<f64>,
    sunlit_soil_water_stress: f64,
    shaded_soil_water_stress: f64,
    maximum_sunlit_leaf_conductance_umol_m2_s: Option<f64>,
    maximum_shaded_leaf_conductance_umol_m2_s: Option<f64>,
}

impl Default for Iteration {
    fn default() -> Self {
        Self {
            ram: 0.0,
            raw: 0.0,
            surface: crate::MoninObukhovState {
                friction_velocity_m_s: 0.0,
                heat_at_2m: 0.0,
                moisture_at_2m: 0.0,
                momentum_at_10m: 0.0,
                momentum: 0.0,
                heat: 0.0,
                moisture: 0.0,
            },
            top_heat: 0.0,
            top_moisture: 0.0,
            heat_at_2m: 0.0,
            moisture_at_2m: 0.0,
            zeta: 0.0,
            leaf_sensible_heat: 0.0,
            leaf_sensible_temperature_slope: 0.0,
            leaf_evaporation_unadjusted: 0.0,
            leaf_evaporation_temperature_slope: 0.0,
            transpiration: 0.0,
            transpiration_temperature_slope: 0.0,
            sunlit_transpiration: 0.0,
            shaded_transpiration: 0.0,
            wet_evaporation: 0.0,
            wet_evaporation_temperature_slope: 0.0,
            net_longwave: 0.0,
            net_longwave_temperature_slope: 0.0,
            unbounded_temperature_change: 0.0,
            evaporation_imbalance: 0.0,
            canopy_air_temperature: 0.0,
            canopy_air_humidity: 0.0,
            air_heat_weight: 0.0,
            ground_heat_weight: 0.0,
            leaf_heat_weight: 0.0,
            air_moisture_weight: 0.0,
            ground_moisture_weight: 0.0,
            leaf_moisture_weight: 0.0,
            ground_heat_conductance: 0.0,
            ground_moisture_conductance: 0.0,
            sunlit_resistance: crate::StomataState {
                assimilation_mol_m2_s: 0.0,
                respiration_mol_m2_s: 0.0,
                stomatal_resistance_s_m: 0.0,
            },
            shaded_resistance: crate::StomataState {
                assimilation_mol_m2_s: 0.0,
                respiration_mol_m2_s: 0.0,
                stomatal_resistance_s_m: 0.0,
            },
            leaf_sunlit_resistance: 0.0,
            leaf_shaded_resistance: 0.0,
            root_flux_kg_m2_s: Vec::new(),
            sunlit_soil_water_stress: 0.0,
            shaded_soil_water_stress: 0.0,
            maximum_sunlit_leaf_conductance_umol_m2_s: None,
            maximum_shaded_leaf_conductance_umol_m2_s: None,
        }
    }
}

/// `MOD_LeafTemperature.F90:460-462` 的 `cintsun`。
///
/// 阳叶与阴叶用的是**两个不同**的三元素（`cintsha` 在调用点算，见
/// `MOD_LeafTemperature.F90:464-466`），所以它按群体传进
/// `LeafPhotosynthesisInput::canopy_integration`，不再是生化参数的一部分。
fn sunlit_canopy_integration(direct: f64, diffuse: f64, lai: f64) -> [f64; 3] {
    [
        integrated_extinction(0.110 + direct, lai),
        integrated_extinction(direct + diffuse, lai),
        integrated_extinction(direct, lai),
    ]
}

fn integrated_extinction(extinction: f64, lai: f64) -> f64 {
    if extinction.abs() <= f64::EPSILON {
        lai
    } else {
        (1.0 - (-extinction * lai).exp()) / extinction
    }
}

struct StomataStep {
    leaf_temperature_k: f64,
    leaf_boundary_resistance_s_m: f64,
    absorbed_par_w_m2: f64,
    soil_water_stress: f64,
    canopy_integration: [f64; 3],
    canopy_air_co2_pa: f64,
    canopy_vapor_pressure_pa: f64,
    leaf_vapor_pressure_pa: f64,
}

fn stomatal_resistance(
    input: LeafTemperatureInput<'_>,
    step: StomataStep,
) -> Result<crate::StomataState> {
    stomata(
        StomataInput {
            photosynthesis: LeafPhotosynthesisInput {
                biochemistry: input.biochemistry,
                canopy_integration: step.canopy_integration,
                leaf_temperature_k: step.leaf_temperature_k,
                oxygen_partial_pressure_pa: input.oxygen_partial_pressure_pa,
                absorbed_par_w_m2: step.absorbed_par_w_m2,
                air_pressure_pa: input.surface_pressure_pa,
                soil_water_stress: step.soil_water_stress,
                leaf_boundary_resistance_s_m: step.leaf_boundary_resistance_s_m,
            },
            atmospheric_co2_pa: input.atmospheric_co2_pa,
            canopy_air_co2_pa: step.canopy_air_co2_pa,
            canopy_air_vapor_pressure_pa: step.canopy_vapor_pressure_pa,
            leaf_saturation_vapor_pressure_pa: step.leaf_vapor_pressure_pa,
            wue_lambda: input.wue_lambda,
        },
        input.options.stomata,
    )
}

fn hydraulic_stomatal_resistance(
    input: LeafTemperatureInput<'_>,
    step: StomataStep,
    canopy_conductance_umol_m2_s: f64,
) -> Result<StomataState> {
    let photosynthesis = LeafPhotosynthesisInput {
        biochemistry: input.biochemistry,
        canopy_integration: step.canopy_integration,
        leaf_temperature_k: step.leaf_temperature_k,
        oxygen_partial_pressure_pa: input.oxygen_partial_pressure_pa,
        absorbed_par_w_m2: step.absorbed_par_w_m2,
        air_pressure_pa: input.surface_pressure_pa,
        soil_water_stress: step.soil_water_stress,
        leaf_boundary_resistance_s_m: step.leaf_boundary_resistance_s_m,
    };
    let update = update_photosynthesis(
        PhotosynthesisUpdateInput {
            photosynthesis,
            atmospheric_co2_pa: input.atmospheric_co2_pa,
            canopy_air_co2_pa: step.canopy_air_co2_pa,
            // `MOD_LeafTemperature_Extended.F90:908-911`：调用方把 PHS 的逐叶
            // µmol 输出乘 `laisun` 后**原样**交给 `update_photosyn`（哑元注释写
            // "mol"，数值却是 µmol）。这里原先除 `1e6`，第 293 轮探针实测内核
            // `gsh2o = 42.51463425289875` 对本仓库 `4.251463425289875e-05`。
            canopy_conductance_h2o_umol_m2_s: canopy_conductance_umol_m2_s,
        },
        input.options.stomata,
    )?;
    let pressure_conversion = 44.6 * 273.16 * input.surface_pressure_pa / 1.013e5;
    // 上游 PHS 支路是 `MOD_LeafTemperature_Extended.F90:919` 的
    //   `rssun = tprcor/tl * 1.e6 / gssun`
    // 左结合 = `((tprcor/tl)*1e6)/gssun`：**先除 `tl`**，再乘 `1e6`，最后除 `gssun`。
    // 原先写成 `tprcor*1e6/(tl*gssun)` —— 一次乘、一次除、且分母先乘起来，
    // 三处舍入点都不同。逐迭代位型探针在第 1 轮抓到 `rssun`/`rssha` 差 1 ULP
    // （它是该轮唯一不是别人下游的量），`cfw`/`etr_dtl`/`fevpl_dtl` 都跟着它偏。
    Ok(StomataState {
        assimilation_mol_m2_s: update.assimilation_mol_m2_s,
        respiration_mol_m2_s: update.respiration_mol_m2_s,
        stomatal_resistance_s_m: pressure_conversion / step.leaf_temperature_k * 1.0e6
            / canopy_conductance_umol_m2_s,
    })
}

fn longwave(input: LeafTemperatureInput<'_>, leaf_temperature_k: f64, factor: f64) -> (f64, f64) {
    // 内核把"叶面四次方 + 地面长波"这一串**逐项融合进累加**，而且融合的是**后一项**：
    // `.loc 1 1105`（`lt_ext.s` 的 `L417` 与 `LBB245`）：
    //   `_3199 = frl - (2*stefnc)*tl**4`（`fmsub`）
    //   非分裂：`_400  = fma(emg*stefnc, tg**4, _3199)`
    //   分裂  ：`tmp1938 = fma(((1-fsno)*emg)*stefnc, t_soil**4, _3199)`
    //           `_429    = fma((fsno*emg)*stefnc, t_snow**4, tmp1938)`
    // 合并点：
    //   `tmp1923 = fma(_400, fac, _3205)`  ← `*factor + 第二项` 也是融合的
    //   `irab    = tmp1923 + _3210`        ← 只有最后一项是平铺加法
    // 原先先把地面两项加成 `ground_longwave`、再整体相加、再平铺 `*factor + …`，
    // 三处都没融。逐迭代位型探针在第 1 轮抓到 `irab` 差 1 ULP。
    let leaf_fourth_power = leaf_temperature_k.powi(4);
    // `_3199` 那一步**也是融合的**（`fmsub(_3197, tmp1913, frl)`），上一条注释写着它、
    // 代码却漏了 —— 用探针输入做的离线复算指出得很干净：kernel 与 Rust 在各个
    // 候选形状下唯一稳定不同的是这一处（
    // `/tmp/gf/irab_shapes.py` 的穷举：`base=fma` 命中 kernel 全部四轮、
    // `base=平铺` 命中 Rust 全部四轮，`tl4=(tl*tl)*(tl*tl)` 两侧都要）。
    let longwave_base =
        (-2.0 * STEFAN_BOLTZMANN).mul_add(leaf_fourth_power, input.atmospheric_longwave_w_m2);
    let radiated = if input.options.split_soil_snow {
        (input.snow_cover_fraction * input.ground_emissivity * STEFAN_BOLTZMANN).mul_add(
            input.snow_surface_temperature_k.powi(4),
            ((1.0 - input.snow_cover_fraction) * input.ground_emissivity * STEFAN_BOLTZMANN)
                .mul_add(input.soil_surface_temperature_k.powi(4), longwave_base),
        )
    } else {
        (input.ground_emissivity * STEFAN_BOLTZMANN)
            .mul_add(input.ground_temperature_k.powi(4), longwave_base)
    };
    (
        radiated.mul_add(
            factor,
            (1.0 - input.ground_emissivity)
                * input.canopy_longwave_gap_fraction
                * factor
                * input.atmospheric_longwave_w_m2,
        ) + (1.0 - input.ground_emissivity)
            * (1.0 - input.canopy_longwave_gap_fraction)
            * factor
            * STEFAN_BOLTZMANN
            * leaf_fourth_power,
        -8.0 * STEFAN_BOLTZMANN * leaf_temperature_k.powi(3) * factor
            + 4.0
                * (1.0 - input.ground_emissivity)
                * (1.0 - input.canopy_longwave_gap_fraction)
                * factor
                * STEFAN_BOLTZMANN
                * leaf_temperature_k.powi(3),
    )
}

fn upward_longwave(
    input: LeafTemperatureInput<'_>,
    previous_leaf_temperature_k: f64,
    leaf_temperature_change_k: f64,
    factor: f64,
) -> f64 {
    let leaf_canopy_term = factor
        * previous_leaf_temperature_k.powi(3)
        * (previous_leaf_temperature_k + 4.0 * leaf_temperature_change_k);
    // 非 split：`p1 + ther mk*emg*tg**4`（`…:1416` 括号里那一和）——**那个乘积被收进加法**
    // ⇒ `FMA(thermk*emg, tg**4, p1)`。12 步同时约束的离线穷举里，全部 48 个可行组合
    // 都带这一条（`/tmp/gf/ulrad12`），所以它和第 262 轮那条"左乘积"一样是实锤。
    let canopy_emission = if input.options.split_soil_snow {
        leaf_canopy_term
            + (1.0 - input.snow_cover_fraction)
                * input.canopy_longwave_gap_fraction
                * input.ground_emissivity
                * input.soil_surface_temperature_k.powi(4)
            + input.snow_cover_fraction
                * input.canopy_longwave_gap_fraction
                * input.ground_emissivity
                * input.snow_surface_temperature_k.powi(4)
    } else {
        (input.canopy_longwave_gap_fraction * input.ground_emissivity)
            .mul_add(input.ground_temperature_k.powi(4), leaf_canopy_term)
    };
    // `MOD_LeafTemperature_Extended.F90:1416` 的第一个加法**左边就是乘积**
    // `stefnc*(fac*t³*(t+4*dtl) + ther mk*emg*tg⁴)` ⇒ GCC 把它收进加法：
    // `FMA(stefnc, p1+p2, (1-emg)*thermk*thermk*frl)`。
    // 离线穷举（`/tmp/gf/ulrad` 的第 3 步两侧位型）实测：只有"左乘积融合"这一维能
    // 复现内核的 `ulrad`（`4070B00402BA216F`），平铺得到 `…216E`；后三个加法的
    // 收缩在本算例看不出来，保持平铺。
    let mut value = STEFAN_BOLTZMANN.mul_add(
        canopy_emission,
        (1.0 - input.ground_emissivity)
            * input.canopy_longwave_gap_fraction.powi(2)
            * input.atmospheric_longwave_w_m2,
    );
    value += (1.0 - input.ground_emissivity)
        * input.canopy_longwave_gap_fraction
        * factor
        * STEFAN_BOLTZMANN
        * previous_leaf_temperature_k.powi(4);
    value += 4.0
        * (1.0 - input.ground_emissivity)
        * input.canopy_longwave_gap_fraction
        * factor
        * STEFAN_BOLTZMANN
        * previous_leaf_temperature_k.powi(3)
        * leaf_temperature_change_k;
    value
}

/// 冠层持水两侧的更新，返回 `(fwet_snow, phase_flux_deficit, flux_deficit)`。
///
/// 后两个是上游把"削掉的通量"退回 `fevpl`/`fsenl` 用的**超配量**：
/// `phase_flux_deficit` 是通量（kg m-2 s-1），`flux_deficit` 是水深
/// （kg m-2，调用方要再除 `deltim`）。上游按这个顺序各退一次，见
/// `:1470-1474` / `:1485-1491`；顺序不能合并（结合次序会变）。
fn update_canopy_water(
    input: LeafTemperatureInput<'_>,
    state: &mut LeafTemperatureState,
    wet_evaporation_kg_m2_s: f64,
) -> Result<(f64, f64, f64)> {
    if !input.options.vegetation_snow {
        let components = state.canopy_water.rain_mm + state.canopy_water.snow_mm;
        if components > 1.0e-10 {
            // 按比例分回两个分量。**上界要夹到 `total_mm`**：不夹的话
            // `rain_mm` 可能比 `total_mm` 大一点点（浮点结合律），于是
            // `snow_mm = total - rain` 变成 −3e-18 —— 一个纯舍入的负水深，
            // 实测跨月算例里它会把 `intercept_canopy` 的入参校验打掉。
            let total = state.canopy_water.total_mm;
            let rain = (state.canopy_water.rain_mm * total / components).clamp(0.0, total);
            state.canopy_water.rain_mm = rain;
            state.canopy_water.snow_mm = total - rain;
        } else if state.canopy_water.total_mm > 0.0 {
            if state.leaf_temperature_k > FREEZING_K {
                state.canopy_water.rain_mm = state.canopy_water.total_mm;
                state.canopy_water.snow_mm = 0.0;
            } else {
                state.canopy_water.rain_mm = 0.0;
                state.canopy_water.snow_mm = state.canopy_water.total_mm;
            }
        } else {
            state.canopy_water.rain_mm = 0.0;
            state.canopy_water.snow_mm = 0.0;
        }
        return Ok((0.0, 0.0, 0.0));
    }
    // `:1468-1495`（`partition_canopy_latent_flux` 内联后）。GIMPLE 逐句给出形状
    // （编的是 `extends/interception/MOD_LeafTemperature_Extended.F90`，
    // Makefile `:641` 把 `MOD_LeafTemperature.o` 指到它而不是 `main/`），
    // 第 4536-4745 处：
    //   `_3169 = MAX(evplwet,0)`、`_3171 = ABS(MIN(evplwet,0))`；
    //   暖支 `qevpl=_3169, qdewl=_3171, qsubl=qfrol=0`，`qevpl > ldew_rain/deltim`
    //   时把超出部分记进 `phase_flux_deficit` 并把 `qevpl` 砍到可用量；冷支对称。
    // **超配的那部分不改水体**：它进 `phase_flux_deficit`，由调用方退回
    // `fevpl`/`fsenl`（`bb203`）。旧实现照 `main/MOD_LeafTemperature.F90:1185-1197`
    // 把它从 `ldew_rain`（暖支则是 `ldew_snow`）里扣掉 —— 那是**没编进内核的那份
    // 源码**，实测湿窗 `CN-Cng-wet` 第 20 步的 `ldew`/`ldew_rain` 种子就出在这里
    // （第 348 轮）。
    let evaporation = wet_evaporation_kg_m2_s.max(0.0);
    let dew = (-wet_evaporation_kg_m2_s).max(0.0);
    let (qevpl, qdewl, qsubl, qfrol, phase_flux_deficit) = if state.leaf_temperature_k > FREEZING_K
    {
        let mut qevpl = evaporation;
        let mut deficit = 0.0;
        let available = state.canopy_water.rain_mm / input.time_step_seconds;
        if qevpl > available {
            deficit = qevpl - available;
            qevpl = available;
        }
        (qevpl, dew, 0.0, 0.0, deficit)
    } else {
        let mut qsubl = evaporation;
        let mut deficit = 0.0;
        let available = state.canopy_water.snow_mm / input.time_step_seconds;
        if qsubl > available {
            deficit = qsubl - available;
            qsubl = available;
        }
        (0.0, 0.0, qsubl, dew, deficit)
    };
    // `bb204` 的两条 FMA **无条件**执行（`_880 = .FMA(deltim, qdewl-qevpl, ldew_rain旧值)`、
    // `_887 = .FMA(deltim, qfrol-qsubl, ldew_snow旧值)`）：`deltim*通量` 被吸收、
    // 旧值是已舍入的加数。收敛后收尾那一族在 `dtl→0` 时惰性，这两句不是。
    let rain_mm = input
        .time_step_seconds
        .mul_add(qdewl - qevpl, state.canopy_water.rain_mm);
    let snow_mm = input
        .time_step_seconds
        .mul_add(qfrol - qsubl, state.canopy_water.snow_mm);
    // `bb205-207`：**两侧**都要夹到 0（`M.98`/`M.99`），再 `ldew = ldew_rain + ldew_snow`。
    // 漏掉任何一个夹取都是 1 ULP 级的种子：`qevpl = ldew_rain/deltim` 让
    // `FMA(deltim, -qevpl, ldew_rain)` 的理论值恰好是 0，实际会落到 **−1 ULP**
    // （实测 −1.0503208545953324e-19 mm，内核是干净的 0.0）。`flux_deficit` 取的是
    // **夹取前**两个负值之和（`M.96+M.97`），所以必须在夹取之前算。
    let flux_deficit = (-rain_mm).max(0.0) + (-snow_mm).max(0.0);
    state.canopy_water.rain_mm = rain_mm.max(0.0);
    state.canopy_water.snow_mm = snow_mm.max(0.0);
    state.canopy_water.total_mm = state.canopy_water.rain_mm + state.canopy_water.snow_mm;
    // `fwet_snow`（湿雪覆盖率）必须在**截留/凝结更新之后、相变之前**取值，
    // 不是在这段更新之前。上游 `MOD_LeafTemperature_Extended.F90:1532` 就是在
    // `LEAF_interception` 已经把 `ldew_snow` 写完之后才调
    // `canopy_snow_wetfrac(sigf, lai, sai, dewmx, tl, ldew_snow)`；紧跟着的相变块
    // （`:1551`/`:1564` 的 Niu(2004) 拉回）用的正是这一个值，此后没有任何一处
    // 重算。放在更新之前读到的是**上一步**的 `snow_mm`：实测 CN-Cng 第一步
    // （`tl`=265.88 K < 冰点，走凝华分支）黄金 `fwet_snow`=0.06138738，而这里算成
    // 0 —— 刚凝华进 `snow_mm` 的那 0.047 mm 完全没进覆盖率。它不是诊断量：
    // `MOD_Albedo` 的 `scat`/`beta0` 直接吃它，所以 `f_alb` → `f_sr*`/`f_sab*`
    // → `f_rnet` 那一族的差就是从这一步开始的。
    // 上游 `:1542` 的 `fwet_snow = canopy_snow_wetfrac(sigf, lai, sai, dewmx, tl, ldew_snow)`
    // —— 指数是 `2.0_r8/3.0_r8`、容量是 `48*dewmx*max(lai+sai,0)`、还带
    // `satcap > 1e-10` 的闸门。**`main/MOD_LeafTemperature.F90:1236` 那一份不同**
    // （截断字面量 `.666666666666`、`(10/(48*lsai))*ldew`），而内核编的是
    // `extends/interception/`。实测干窗第 0 步重启的 `fwet_snow` 差 2.79e-12 相对量，
    // 正是这个差别。
    let mut wet_snow_fraction = crate::interception::canopy_snow_wet_fraction(
        input.maximum_dew_mm,
        input.leaf_area_index,
        input.stem_area_index,
        state.canopy_water.snow_mm,
    );
    if state.canopy_water.snow_mm > 1.0e-6 && state.leaf_temperature_k > FREEZING_K {
        let melt = (state.canopy_water.snow_mm / input.time_step_seconds).min(
            (state.leaf_temperature_k - FREEZING_K)
                * ICE_HEAT_CAPACITY_J_KG_K
                * state.canopy_water.snow_mm
                / (input.time_step_seconds * FUSION_HEAT_J_KG),
        );
        state.canopy_water.snow_mm =
            (state.canopy_water.snow_mm - melt * input.time_step_seconds).max(0.0);
        state.canopy_water.rain_mm += melt * input.time_step_seconds;
        // `:1561/:1574 tl = fwet_snow*tfrz + (1-fwet_snow)*tl` 的 GIMPLE 是
        // `.FMA(fwet_snow, 2.7316e2, (1-fwet_snow)*tl)` —— `fwet_snow*tfrz` 进 FMA，
        // `(1-fwet_snow)*tl` 独立舍入当加数。
        state.leaf_temperature_k = wet_snow_fraction.mul_add(
            FREEZING_K,
            (1.0 - wet_snow_fraction) * state.leaf_temperature_k,
        );
    }
    if state.canopy_water.rain_mm > 1.0e-6 && state.leaf_temperature_k < FREEZING_K {
        let freeze = (state.canopy_water.rain_mm / input.time_step_seconds).min(
            (FREEZING_K - state.leaf_temperature_k)
                * WATER_HEAT_CAPACITY_J_KG_K
                * state.canopy_water.rain_mm
                / (input.time_step_seconds * FUSION_HEAT_J_KG),
        );
        state.canopy_water.rain_mm =
            (state.canopy_water.rain_mm - freeze * input.time_step_seconds).max(0.0);
        state.canopy_water.snow_mm += freeze * input.time_step_seconds;
        // `:1561/:1574 tl = fwet_snow*tfrz + (1-fwet_snow)*tl` 的 GIMPLE 是
        // `.FMA(fwet_snow, 2.7316e2, (1-fwet_snow)*tl)` —— `fwet_snow*tfrz` 进 FMA，
        // `(1-fwet_snow)*tl` 独立舍入当加数。
        state.leaf_temperature_k = wet_snow_fraction.mul_add(
            FREEZING_K,
            (1.0 - wet_snow_fraction) * state.leaf_temperature_k,
        );
    }
    state.canopy_water.total_mm = state.canopy_water.rain_mm + state.canopy_water.snow_mm;
    wet_snow_fraction = wet_snow_fraction.min(1.0);
    Ok((wet_snow_fraction, phase_flux_deficit, flux_deficit))
}

fn validate(input: LeafTemperatureInput<'_>, state: LeafTemperatureState) -> Result<()> {
    let scalars = [
        input.time_step_seconds,
        input.maximum_dew_mm,
        input.leaf_area_index,
        input.stem_area_index,
        input.canopy_top_height_m,
        input.inverse_sqrt_leaf_dimension_m_neg_half,
        input.soil_water_stress_sunlit,
        input.soil_water_stress_shaded,
        input.wue_lambda,
        input.direct_extinction,
        input.diffuse_extinction,
        input.wind_height_m,
        input.temperature_height_m,
        input.humidity_height_m,
        input.eastward_wind_m_s,
        input.northward_wind_m_s,
        input.reference_air_temperature_k,
        input.potential_temperature_k,
        input.virtual_potential_temperature_k,
        input.reference_specific_humidity,
        input.surface_pressure_pa,
        input.air_density_kg_m3,
        input.sunlit_absorbed_par_w_m2,
        input.shaded_absorbed_par_w_m2,
        input.canopy_absorbed_solar_w_m2,
        input.atmospheric_longwave_w_m2,
        input.sunlit_fraction,
        input.canopy_longwave_gap_fraction,
        input.oxygen_partial_pressure_pa,
        input.atmospheric_co2_pa,
        input.soil_roughness_m,
        input.snow_roughness_m,
        input.snow_cover_fraction,
        input.ground_obukhov_length_m,
        input.transpiration_limit_kg_m2_s,
        input.ground_temperature_k,
        input.soil_surface_temperature_k,
        input.snow_surface_temperature_k,
        input.ground_specific_humidity,
        input.soil_specific_humidity,
        input.snow_specific_humidity,
        input.ground_humidity_temperature_slope_k,
        input.soil_surface_resistance_s_m,
        input.ground_emissivity,
        input.precipitation_temperature_k,
        input.intercepted_rain_kg_m2_s,
        input.intercepted_snow_kg_m2_s,
        input.ground_latent_heat_j_kg,
        state.leaf_temperature_k,
        state.canopy_water.total_mm,
        state.canopy_water.rain_mm,
        state.canopy_water.snow_mm,
    ];
    // 逐条报出失败的判据。一个笼统的"输入非法"在真实算例上无法定位：这些量来自
    // 重启、强迫场与上一步的能量链，谁越界了必须当场知道是哪一个。
    let mut failed: Vec<String> = Vec::new();
    let mut check = |label: &str, ok: bool| {
        if !ok {
            failed.push(label.to_owned());
        }
    };
    check(
        "some scalar is not finite",
        scalars.iter().all(|v| v.is_finite()),
    );
    check("time_step_seconds > 0", input.time_step_seconds > 0.0);
    check("maximum_dew_mm > 0", input.maximum_dew_mm > 0.0);
    check("leaf_area_index > 0.001", input.leaf_area_index > 0.001);
    check("stem_area_index >= 0", input.stem_area_index >= 0.0);
    check(
        "canopy_top_height_m above the roughness lengths",
        input.canopy_top_height_m > input.soil_roughness_m.max(input.snow_roughness_m),
    );
    check(
        "inverse_sqrt_leaf_dimension_m_neg_half > 0",
        input.inverse_sqrt_leaf_dimension_m_neg_half > 0.0,
    );
    // 上游的 `rstfac`（`MOD_Eroot.F90:101-140`）**没有 1 的上界**：势梯度方案里
    // `rresis = (1 - smp_node/smpmax)/(1 - psi0/smpmax)` 在湿润层会大于 1，而它正是
    // `etrc = trsmx0*roota` 的倍数。只有下界是真的。
    check(
        &format!(
            "soil_water_stress_sunlit >= 0, got {}",
            input.soil_water_stress_sunlit
        ),
        input.soil_water_stress_sunlit >= 0.0,
    );
    check(
        &format!(
            "soil_water_stress_shaded >= 0, got {}",
            input.soil_water_stress_shaded
        ),
        input.soil_water_stress_shaded >= 0.0,
    );
    check("wue_lambda > 0", input.wue_lambda > 0.0);
    check("direct_extinction > 0", input.direct_extinction > 0.0);
    check("diffuse_extinction > 0", input.diffuse_extinction > 0.0);
    check(
        "reference_air_temperature_k > 0",
        input.reference_air_temperature_k > 0.0,
    );
    check(
        "potential_temperature_k > 0",
        input.potential_temperature_k > 0.0,
    );
    check(
        "virtual_potential_temperature_k > 0",
        input.virtual_potential_temperature_k > 0.0,
    );
    check(
        "reference_specific_humidity in 0..1",
        (0.0..1.0).contains(&input.reference_specific_humidity),
    );
    check("surface_pressure_pa > 0", input.surface_pressure_pa > 0.0);
    check("air_density_kg_m3 > 0", input.air_density_kg_m3 > 0.0);
    check(
        "sunlit_fraction in (0,1)",
        input.sunlit_fraction > 0.0 && input.sunlit_fraction < 1.0,
    );
    check(
        "canopy_longwave_gap_fraction in 0..=1",
        (0.0..=1.0).contains(&input.canopy_longwave_gap_fraction),
    );
    check(
        "oxygen_partial_pressure_pa >= 0",
        input.oxygen_partial_pressure_pa >= 0.0,
    );
    check("atmospheric_co2_pa >= 0", input.atmospheric_co2_pa >= 0.0);
    check("soil_roughness_m > 0", input.soil_roughness_m > 0.0);
    check("snow_roughness_m > 0", input.snow_roughness_m > 0.0);
    check(
        "snow_cover_fraction in 0..=1",
        (0.0..=1.0).contains(&input.snow_cover_fraction),
    );
    check(
        "ground_obukhov_length_m != 0",
        input.ground_obukhov_length_m != 0.0,
    );
    check(
        "transpiration_limit_kg_m2_s >= 0",
        input.transpiration_limit_kg_m2_s >= 0.0,
    );
    check(
        "ground_specific_humidity in 0..1",
        (0.0..1.0).contains(&input.ground_specific_humidity),
    );
    check(
        "soil_specific_humidity in 0..1",
        (0.0..1.0).contains(&input.soil_specific_humidity),
    );
    check(
        "snow_specific_humidity in 0..1",
        (0.0..1.0).contains(&input.snow_specific_humidity),
    );
    check(
        "soil_surface_resistance_s_m >= 0",
        input.soil_surface_resistance_s_m >= 0.0,
    );
    check(
        "ground_emissivity in 0..=1",
        (0.0..=1.0).contains(&input.ground_emissivity),
    );
    // 净截留率**允许为负**（冠层排水超过截留量），内核按上游取 `max(0, ·)`；
    // 这里只要求有限，不再要求非负 —— 要求非负会让任何排水步直接报错。
    check(
        "intercepted_rain_kg_m2_s is finite",
        input.intercepted_rain_kg_m2_s.is_finite(),
    );
    check(
        "intercepted_snow_kg_m2_s is finite",
        input.intercepted_snow_kg_m2_s.is_finite(),
    );
    check(
        "ground_latent_heat_j_kg > 0",
        input.ground_latent_heat_j_kg > 0.0,
    );
    check(
        "state.leaf_temperature_k > 0",
        state.leaf_temperature_k > 0.0,
    );
    // 冠层持水允许到 `-CANOPY_WATER_ROUNDOFF_MM`：`leaf_interception` 里
    // `pinf` 可以是 -1 ulp，上游不夹也不校验。判据与理由见那个常量的文档。
    for (name, value) in [
        (
            "state.canopy_water.total_mm >= -roundoff",
            state.canopy_water.total_mm,
        ),
        (
            "state.canopy_water.rain_mm >= -roundoff",
            state.canopy_water.rain_mm,
        ),
        (
            "state.canopy_water.snow_mm >= -roundoff",
            state.canopy_water.snow_mm,
        ),
    ] {
        check(
            name,
            value >= -crate::interception::CANOPY_WATER_ROUNDOFF_MM,
        );
    }
    check(
        "plant hydraulics must match its state",
        input.plant_hydraulics.is_none() || state.plant_hydraulics.is_some(),
    );
    ensure!(
        failed.is_empty(),
        "leaf-temperature inputs are invalid: {}",
        failed.join("; ")
    );
    Ok(())
}

#[cfg(test)]
#[path = "leaf_temperature_tests.rs"]
mod leaf_temperature_tests;
