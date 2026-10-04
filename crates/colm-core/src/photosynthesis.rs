//! Leaf photosynthesis and stomatal conductance from `MOD_AssimStomataConductance.F90`.
//!
//! The leaf-temperature solver supplies air state and shared canopy resistances;
//! this module owns only the biochemistry/CO₂ iteration so it is reusable by the
//! ordinary and plant-hydraulic canopy paths.

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::f77;

const ITERATIONS: usize = 6;

/// Land-cover/PFT biochemical parameters shared by `stomata` and `update_photosyn`.
///
/// `PartialEq` 只给测试用：装配层要逐项核对它等于地类表的取值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafBiochemistry {
    pub quantum_efficiency: f64,
    pub maximum_carboxylation_25c_mol_m2_s: f64,
    /// One for C3 and zero for C4, matching CoLM's `c3c4` flag.
    pub c3c4: i32,
    /// 单点 LCT 的 `DEF_LC_RESPCP`：设了就取代 `0.015*c3 + 0.025*c4`（upstream-bugs 第 32 条，vendor 已修）。
    pub respiration_fraction_override: Option<f64>,
    pub low_temperature_slope: f64,
    pub low_temperature_half_k: f64,
    pub high_temperature_slope: f64,
    pub high_temperature_half_k: f64,
    pub respiration_temperature_slope: f64,
    pub respiration_temperature_half_k: f64,
    pub optimum_temperature_k: f64,
    pub medlyn_g1: f64,
    pub medlyn_g0: f64,
    pub ball_berry_slope: f64,
    pub ball_berry_intercept: f64,
}

/// Inputs common to CoLM's private `calc_photo_params` and public photo updates.
#[derive(Debug, Clone, Copy)]
pub struct LeafPhotosynthesisInput {
    pub biochemistry: LeafBiochemistry,
    /// 冠层积分因子，上游 `MOD_AssimStomataConductance:stomata` 的 `cint(1:3)`。
    ///
    /// 它**不是**生化参数：`MOD_LeafTemperature.F90:460-466` 每步由 `lai`/`extkb`/`extkd`
    /// 现算 `cintsun` 与 `cintsha` 两个不同三元素，分别给阳叶与阴叶。放在
    /// `LeafBiochemistry` 里会让它看起来像地类常量，而且每次都被调用点覆盖。
    pub canopy_integration: [f64; 3],
    pub leaf_temperature_k: f64,
    pub oxygen_partial_pressure_pa: f64,
    pub absorbed_par_w_m2: f64,
    pub air_pressure_pa: f64,
    pub soil_water_stress: f64,
    pub leaf_boundary_resistance_s_m: f64,
}

/// Runtime namelist choices affecting CoLM stomatal conductance.
#[derive(Debug, Clone, Copy, Default)]
pub struct StomataOptions {
    pub use_medlyn: bool,
    pub use_wue: bool,
    pub medlyn_g1_override: Option<f64>,
    pub medlyn_g0_override: Option<f64>,
    pub wue_lambda_override: Option<f64>,
    pub ball_berry_slope_override: Option<f64>,
    pub ball_berry_intercept_override: Option<f64>,
}

/// Outputs of `calc_photo_params` used by both CoLM photo pathways.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotosynthesisParameters {
    pub maximum_carboxylation_mol_m2_s: f64,
    pub electron_transport_mol_m2_s: f64,
    pub respiration_mol_m2_s: f64,
    pub sink_limit_mol_m2_s_pa: f64,
    pub boundary_conductance_h2o_mol_m2_s: f64,
    pub co2_compensation_pa: f64,
    pub rubisco_co2_constant_pa: f64,
    pub c3_fraction: f64,
    pub c4_fraction: f64,
}

/// Inputs to CoLM's `stomata` routine.  Its aerodynamic-resistance and ozone
/// arguments are absent because the upstream routine receives but does not use them.
#[derive(Debug, Clone, Copy)]
pub struct StomataInput {
    pub photosynthesis: LeafPhotosynthesisInput,
    pub atmospheric_co2_pa: f64,
    pub canopy_air_co2_pa: f64,
    pub canopy_air_vapor_pressure_pa: f64,
    pub leaf_saturation_vapor_pressure_pa: f64,
    pub wue_lambda: f64,
}

/// Canopy-scaled outputs of CoLM's `stomata` routine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StomataState {
    pub assimilation_mol_m2_s: f64,
    pub respiration_mol_m2_s: f64,
    pub stomatal_resistance_s_m: f64,
}

/// Inputs to CoLM's `update_photosyn` plant-hydraulic recomputation.
#[derive(Debug, Clone, Copy)]
pub struct PhotosynthesisUpdateInput {
    pub photosynthesis: LeafPhotosynthesisInput,
    pub atmospheric_co2_pa: f64,
    pub canopy_air_co2_pa: f64,
    /// 冠层水汽导度，**µmol m⁻² s⁻¹**。
    ///
    /// 上游 `update_photosyn` 的哑元 `gsh2o` 注释写的是 "mol m-2 s-1"，但调用点
    /// `MOD_LeafTemperature_Extended.F90:908-911` 把 PHS 的**逐叶 µmol** 输出乘
    /// `laisun` 后**原样**传进去（第 293 轮探针：CN-Cng 干窗第 1 次调用实测
    /// `gsh2o = 42.51463425289875`）。照抄的是调用点的数值，不是哑元的注释 ——
    /// 原先这里按注释除了 `1e6`，白天的 `1.6*assmt/gsh2o` 被放大 `1e6` 倍，
    /// `pco2in`/`eyy` 偏掉，`assim` 在第一个有光的迭代就分叉。
    pub canopy_conductance_h2o_umol_m2_s: f64,
}

/// Outputs of CoLM's `update_photosyn` routine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotosynthesisUpdateState {
    pub assimilation_mol_m2_s: f64,
    pub respiration_mol_m2_s: f64,
}

/// Port of `MOD_AssimStomataConductance:calc_photo_params`.
pub fn photosynthesis_parameters(
    input: LeafPhotosynthesisInput,
) -> Result<PhotosynthesisParameters> {
    validate_photosynthesis(input)?;
    let b = input.biochemistry;
    let c3_fraction = if b.c3c4 == 1 { 1.0 } else { 0.0 };
    let c4_fraction = 1.0 - c3_fraction;
    let temperature_factor = f77(0.1) * (input.leaf_temperature_k - b.optimum_temperature_k);
    let kc = f77(30.0) * f77(2.1).lpow(temperature_factor);
    let ko = f77(30_000.0) * f77(1.2).lpow(temperature_factor);
    let co2_compensation_pa = f77(0.5) * input.oxygen_partial_pressure_pa
        / (f77(2600.0) * f77(0.57).lpow(temperature_factor))
        * c3_fraction;
    let rubisco_co2_constant_pa = kc * (1.0 + input.oxygen_partial_pressure_pa / ko) * c3_fraction;
    let low_inhibition = 1.0
        + (b.low_temperature_slope * (b.low_temperature_half_k - input.leaf_temperature_k)).exp();
    let high_inhibition = 1.0
        + (b.high_temperature_slope * (input.leaf_temperature_k - b.high_temperature_half_k)).exp();
    let mut maximum_carboxylation =
        b.maximum_carboxylation_25c_mol_m2_s * f77(2.1).lpow(temperature_factor);
    // `:570 vm = vm/temph*rstfac*c3 + vm/(templ*temph)*rstfac*c4` 的 GIMPLE 是
    // `.FMA(vm/temph*rstfac, c3, vm/(templ*temph)*rstfac*c4)` —— `c4` 那条链整体
    // 独立舍入当加数，`c3` 那一乘收进 FMA。
    let high_term = maximum_carboxylation / high_inhibition * input.soil_water_stress;
    let low_term = maximum_carboxylation / (low_inhibition * high_inhibition)
        * input.soil_water_stress
        * c4_fraction;
    maximum_carboxylation = high_term.mul_add(c3_fraction, low_term) * input.canopy_integration[0];
    let gas_constant = 8.314_467_591;
    let jmax25 = f77(1.97) * b.maximum_carboxylation_25c_mol_m2_s;
    let mut jmax = jmax25
        * (f77(37.0e3) * (input.leaf_temperature_k - b.optimum_temperature_k)
            / (gas_constant * b.optimum_temperature_k * input.leaf_temperature_k))
            .exp()
        // `:579-580` 的两个分子 `710.*t-220.e3` 各自是一条 FMA（`:580` 的 GIMPLE
        // `_67 = .FMA(t, 7.1e2, -2.2e5)`）。
        * (1.0
            + (f77(710.0)
                .mul_add(b.optimum_temperature_k, -f77(220.0e3))
                / (gas_constant * b.optimum_temperature_k))
                .exp())
        / (1.0
            + (f77(710.0).mul_add(input.leaf_temperature_k, -f77(220.0e3))
                / (gas_constant * input.leaf_temperature_k))
                .exp());
    // `:583-584` 是两句：`jmax = jmax*rstfac` 再 `jmax = jmax*cint(2)`，两次舍入
    // （GIMPLE `jmax_85 = jmax_84*rstfac; jmax_87 = jmax_85*cint2`）。写成
    // `jmax *= rstfac*cint2` 在 `rstfac = 1` 时恰好相同，所以只在 PHS 回写的
    // `update_photosyn`（`rstfac` 是 PHS 胁迫）且 `epar` 被 `jmax` 限制时才露出来：
    // AT-Neu 2010-07-08 正午 `ome` 差 1 ulp（第 406 轮）。
    jmax = jmax * input.soil_water_stress * input.canopy_integration[1];
    let electron_transport =
        (f77(4.6e-6) * input.absorbed_par_w_m2 * b.quantum_efficiency).min(jmax);
    let respiration_fraction = b
        .respiration_fraction_override
        .unwrap_or(f77(0.015) * c3_fraction + f77(0.025) * c4_fraction);
    let respiration = respiration_fraction
        * b.maximum_carboxylation_25c_mol_m2_s
        * f77(2.0).lpow(temperature_factor)
        / (1.0
            + (b.respiration_temperature_slope
                * (input.leaf_temperature_k - b.respiration_temperature_half_k))
                .exp())
        * input.soil_water_stress
        * input.canopy_integration[0];
    // `:597-598 omss = (vmax25/2)*1.8**qt/templ*rstfac*c3 + (vmax25/5)*1.8**qt*rstfac*c4`
    // 与 `:570` 同形（`.FMA(_118, cstore_128, _124)`）。
    let low_sink = (b.maximum_carboxylation_25c_mol_m2_s / f77(2.0))
        * f77(1.8).lpow(temperature_factor)
        / low_inhibition
        * input.soil_water_stress;
    let high_sink = (b.maximum_carboxylation_25c_mol_m2_s / f77(5.0))
        * f77(1.8).lpow(temperature_factor)
        * input.soil_water_stress
        * c4_fraction;
    let sink_limit = low_sink.mul_add(c3_fraction, high_sink) * input.canopy_integration[0];
    let pressure_conversion = f77(44.6 * 273.16) * input.air_pressure_pa / f77(1.013e5);
    // `MOD_AssimStomataConductance.F90:608` 是 `gbh2o = 1./rb * tprcor/tlef`
    // —— 从左到右 `((1/rb)*tprcor)/tlef`，不是 `tprcor/(rb*tlef)`。
    let boundary_conductance_h2o =
        (1.0 / input.leaf_boundary_resistance_s_m) * pressure_conversion / input.leaf_temperature_k;
    Ok(PhotosynthesisParameters {
        maximum_carboxylation_mol_m2_s: maximum_carboxylation,
        electron_transport_mol_m2_s: electron_transport,
        respiration_mol_m2_s: respiration,
        sink_limit_mol_m2_s_pa: sink_limit,
        boundary_conductance_h2o_mol_m2_s: boundary_conductance_h2o,
        co2_compensation_pa,
        rubisco_co2_constant_pa,
        c3_fraction,
        c4_fraction,
    })
}

/// Port of `MOD_AssimStomataConductance:stomata`.
pub fn stomata(input: StomataInput, options: StomataOptions) -> Result<StomataState> {
    validate_stomata(input, options)?;
    let photo = photosynthesis_parameters(input.photosynthesis)?;
    let b = input.photosynthesis.biochemistry;
    let (g1, g0, gradm, binter, lambda) = selected_parameters(b, input.wue_lambda, options)?;
    let bintc = binter
        * input.photosynthesis.soil_water_stress.max(f77(0.1))
        * input.photosynthesis.canopy_integration[2];
    // `:223` 出货是 `fnmsub d31,d31,d29,d8` ⇒ `pco2m*(1-1.6/gradm) - gammas` 的乘积进 FMA。
    let range = input
        .atmospheric_co2_pa
        .mul_add(1.0 - f77(1.6) / gradm, -photo.co2_compensation_pa);
    let mut errors = [0.0; ITERATIONS];
    let mut co2_guesses = [0.0; ITERATIONS];
    let mut assimilation = 0.0;
    let mut conductance = 0.0;
    for iteration in 1..=ITERATIONS {
        let (internal_co2, rubisco_co2, electron_co2) =
            if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
                sortin(
                    &mut errors,
                    &mut co2_guesses,
                    range,
                    photo.co2_compensation_pa,
                    iteration,
                );
                let internal = co2_guesses[iteration - 1];
                (internal, internal, internal)
            } else {
                let (rubisco, electron) = wue_internal_co2(
                    photo.co2_compensation_pa,
                    lambda,
                    input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa,
                    input.leaf_saturation_vapor_pressure_pa,
                    input.canopy_air_vapor_pressure_pa,
                    input.photosynthesis.air_pressure_pa,
                );
                (rubisco, rubisco, electron)
            };
        // `:258/:259/:262` 三条都是 `商*c3 + 整条 c4 链`：
        //   `omc = .FMA(vm*(pco2i_c-gammas)/(pco2i_c+rrkk), c3, vm*c4)`
        //   `ome = .FMA(c3, epar*(…)/(pco2i_e+2*gammas), epar*c4)`，分母那一条
        //         `.FMA(gammas, 2.0, pco2i_e)`（第 336 轮就定死的那处）
        //   `oms = .FMA(c3, omss, (omss*pco2i)*c4)`
        let omc_quotient = photo.maximum_carboxylation_mol_m2_s
            * (rubisco_co2 - photo.co2_compensation_pa)
            / (rubisco_co2 + photo.rubisco_co2_constant_pa);
        let omc = omc_quotient.mul_add(
            photo.c3_fraction,
            photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction,
        );
        let ome_denominator = f77(2.0).mul_add(photo.co2_compensation_pa, electron_co2);
        let ome_quotient = photo.electron_transport_mol_m2_s
            * (electron_co2 - photo.co2_compensation_pa)
            / ome_denominator;
        let ome = photo.c3_fraction.mul_add(
            ome_quotient,
            photo.electron_transport_mol_m2_s * photo.c4_fraction,
        );
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.c3_fraction.mul_add(
                photo.sink_limit_mol_m2_s_pa,
                photo.sink_limit_mol_m2_s_pa * internal_co2 * photo.c4_fraction,
            );
            assimilation = coupled_assimilation(omc, ome, oms);
        } else {
            assimilation = omc.min(ome).max(0.0);
        }
        let net_assimilation = assimilation - photo.respiration_mol_m2_s;
        let co2_surface = input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa
            - f77(1.37) * net_assimilation / photo.boundary_conductance_h2o_mol_m2_s;
        // `MOD_AssimStomataConductance.F90:318-321` 是两个量，不能合并：
        //   `co2s`（未钳制）只进 `:366` 的 `pco2in`；
        //   `co2st = max(min(co2s,co2a),1e-5)` 只进 Medlyn 的 `acp`（`:341`）
        //     与 Ball-Berry 的 `hcdma`（`:350`）。
        // 原先用一个钳制后的 `co2_surface` 兼两职：夜间 `assimn<0` ⇒ `co2s>co2a`，
        // `pco2in` 与内核差 ~1e-4 相对（第 293 轮 UPIT 探针）；白天会把
        // `1.6*assimn/conductance` 的 `co2s` 也换成钳制值。
        let co2_surface_clamped = co2_surface
            .min(input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa)
            .max(f77(1.0e-5));
        let positive_assimilation = net_assimilation.max(f77(1.0e-12));
        let next_co2 = if options.use_wue && (photo.c4_fraction - 1.0).abs() >= f77(0.001) {
            // **WUE 分支的 `pco2i` 要按 `omc < ome` 选一支。**
            // `MOD_AssimStomataConductance.F90:333-337` 在算 `gsh2o` 之前会把
            // `pco2i` 重写成 `pco2i_c`（Rubisco 限制）或 `pco2i_e`（电子传输限制），
            // `gsh2o` 与 `pco2in` 用的都是重写后的那一支。原先这里写死 `internal_co2`
            // （= `pco2i_c`）：实测 CN-Cng-wet 第 10 步起 `gssun`/`gssha` 恰差 **2.03 倍**、
            // `etr`/`etrsun`/`etrsha` 同倍，而同一份文件里的 `f_assim`/`f_respc` 只差 1%。
            let selected_internal_co2 = if omc < ome { rubisco_co2 } else { electron_co2 };
            conductance = positive_assimilation
                / (input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa
                    - selected_internal_co2 / input.photosynthesis.air_pressure_pa)
                * f77(1.6);
            selected_internal_co2
        } else if options.use_medlyn {
            let vapor_deficit_kpa = (input.leaf_saturation_vapor_pressure_pa
                - input.canopy_air_vapor_pressure_pa)
                .max(f77(50.0))
                * f77(1.0e-3);
            let acp = f77(1.6) * positive_assimilation / co2_surface_clamped;
            let a = 1.0;
            // `:343` 里 `(g0*1e-6 + acp)` 的乘积是一条 FMA（`_108 = .FMA(g0, 1e-6, acp)`）。
            let bracket = g0 * f77(1.0e-6) + acp;
            let quotient =
                (g1 * acp).powi(2) / (photo.boundary_conductance_h2o_mol_m2_s * vapor_deficit_kpa);
            let bq = (-bracket).mul_add(f77(2.0), -quotient);
            let g0_scaled = g0 * f77(1.0e-6);
            let one_minus_g1_squared = (-g1).mul_add(g1, 1.0);
            let c_tail = (f77(2.0) * g0)
                .mul_add(f77(1.0e-6), one_minus_g1_squared * acp / vapor_deficit_kpa);
            let c = g0_scaled.mul_add(g0_scaled, c_tail * acp);
            // `:346` `sqrtin = max(0, bquad**2 - 4*aquad*cquad)`：`bquad**2` 进 FMA。
            conductance =
                (-bq + bq.mul_add(bq, -(f77(4.0) * a * c)).max(0.0).sqrt()) / (f77(2.0) * a);
            (co2_surface - f77(1.6) * net_assimilation / conductance)
                * input.photosynthesis.air_pressure_pa
        } else {
            let hcdma = input.leaf_saturation_vapor_pressure_pa * co2_surface_clamped
                / (gradm * positive_assimilation);
            let a = hcdma;
            // `:353 bquad = gbh2o*hcdma - ei - bintc*hcdma` 是**两条** `fmsub`：
            // 先 `fma(hcdma, gbh2o, -ei)`，再 `fma(-bintc, hcdma, 上一步)`。
            let first = hcdma.mul_add(
                photo.boundary_conductance_h2o_mol_m2_s,
                -input.leaf_saturation_vapor_pressure_pa,
            );
            let bq = (-bintc).mul_add(hcdma, first);
            // `:354 cquad = -gbh2o*(ea + hcdma*bintc)`：括号里是一条 FMA。
            let c = -photo.boundary_conductance_h2o_mol_m2_s
                * bintc.mul_add(hcdma, input.canopy_air_vapor_pressure_pa);
            // `:356` 同 `:346`。
            conductance =
                (-bq + bq.mul_add(bq, -(f77(4.0) * a * c)).max(0.0).sqrt()) / (f77(2.0) * a);
            let surface_vapor = ((conductance - bintc) * hcdma)
                .min(input.leaf_saturation_vapor_pressure_pa)
                .max(f77(1.0e-2));
            conductance = surface_vapor / hcdma + bintc;
            (co2_surface - f77(1.6) * net_assimilation / conductance)
                * input.photosynthesis.air_pressure_pa
        };
        errors[iteration - 1] = internal_co2 - next_co2;
        if errors[iteration - 1].abs() < f77(0.1) {
            break;
        }
    }
    let pressure_conversion =
        f77(44.6 * 273.16) * input.photosynthesis.air_pressure_pa / f77(1.013e5);
    Ok(StomataState {
        assimilation_mol_m2_s: assimilation,
        respiration_mol_m2_s: photo.respiration_mol_m2_s,
        stomatal_resistance_s_m: (1.0
            / (conductance * input.photosynthesis.leaf_temperature_k / pressure_conversion))
            .min(f77(1.0e6)),
    })
}

/// Port of `MOD_AssimStomataConductance:update_photosyn`.
pub fn update_photosynthesis(
    input: PhotosynthesisUpdateInput,
    options: StomataOptions,
) -> Result<PhotosynthesisUpdateState> {
    validate_photosynthesis(input.photosynthesis)?;
    ensure!(
        [
            input.atmospheric_co2_pa,
            input.canopy_air_co2_pa,
            input.canopy_conductance_h2o_umol_m2_s,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.atmospheric_co2_pa >= 0.0
            && input.canopy_air_co2_pa >= 0.0
            && input.canopy_conductance_h2o_umol_m2_s > 0.0,
        "photosynthesis update inputs are invalid"
    );
    let photo = photosynthesis_parameters(input.photosynthesis)?;
    let (_, _, gradm, _, _) = selected_parameters(input.photosynthesis.biochemistry, 1.0, options)?;
    // `:223` 出货是 `fnmsub d31,d31,d29,d8` ⇒ `pco2m*(1-1.6/gradm) - gammas` 的乘积进 FMA。
    let range = input
        .atmospheric_co2_pa
        .mul_add(1.0 - f77(1.6) / gradm, -photo.co2_compensation_pa);
    let mut errors = [0.0; ITERATIONS];
    let mut co2_guesses = [0.0; ITERATIONS];
    let mut assimilation = 0.0;
    for iteration in 1..=ITERATIONS {
        sortin(
            &mut errors,
            &mut co2_guesses,
            range,
            photo.co2_compensation_pa,
            iteration,
        );
        let internal = co2_guesses[iteration - 1];
        // `:737/:738/:740` 与 `stomata` 的 `:258/:259/:262` 同形（WUE 关着时
        // `pco2i_c = pco2i_e = pco2i`）。
        let omc_quotient = photo.maximum_carboxylation_mol_m2_s
            * (internal - photo.co2_compensation_pa)
            / (internal + photo.rubisco_co2_constant_pa);
        let omc = omc_quotient.mul_add(
            photo.c3_fraction,
            photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction,
        );
        let ome_denominator = f77(2.0).mul_add(photo.co2_compensation_pa, internal);
        let ome_quotient = photo.electron_transport_mol_m2_s
            * (internal - photo.co2_compensation_pa)
            / ome_denominator;
        let ome = photo.c3_fraction.mul_add(
            ome_quotient,
            photo.electron_transport_mol_m2_s * photo.c4_fraction,
        );
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.c3_fraction.mul_add(
                photo.sink_limit_mol_m2_s_pa,
                photo.sink_limit_mol_m2_s_pa * internal * photo.c4_fraction,
            );
            assimilation = coupled_assimilation(omc, ome, oms);
        } else {
            assimilation = omc.min(ome).max(0.0);
        }
        let net_assimilation = assimilation - photo.respiration_mol_m2_s;
        // `MOD_AssimStomataConductance.F90:796-803`：`update_photosyn` 的 `pco2in`
        // 用的是**未钳制**的 `co2s`（`:803`）。同一处 `:797-798` 还算了一个
        // `co2st = max(min(co2s,co2a),1e-5)`，但它在整个子程序里**从未被读** ——
        // 那是死代码。原先把两者合成一个 `co2_surface`（钳制），夜间
        // （`assimn<0` ⇒ `co2s>co2a`）就与内核差 ~1e-4 相对（第 293 轮 UPIT 探针）。
        let co2_surface = input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa
            - f77(1.37) * net_assimilation / photo.boundary_conductance_h2o_mol_m2_s;
        let positive_assimilation = net_assimilation.max(f77(1.0e-12));
        // `:805 eyy(ic) = pco2i - (co2s - 1.6*assmt/gsh2o)*psrf` 出货是
        // `fsub` + `fmsub` ⇒ `pco2i - bracket*psrf` 收成一条 FMA（加数是 `pco2i`）。
        let bracket =
            co2_surface - f77(1.6) * positive_assimilation / input.canopy_conductance_h2o_umol_m2_s;
        errors[iteration - 1] = (-bracket).mul_add(input.photosynthesis.air_pressure_pa, internal);
        if errors[iteration - 1].abs() < f77(0.1) {
            break;
        }
    }
    Ok(PhotosynthesisUpdateState {
        assimilation_mol_m2_s: assimilation,
        respiration_mol_m2_s: photo.respiration_mol_m2_s,
    })
}

fn coupled_assimilation(rubisco: f64, electron: f64, sink: f64) -> f64 {
    // `:264`/`:266` 的 `max(0,(a+b)^2 - 4*theta*a*b)` 里 `(a+b)^2` 那个乘积进 FMA
    // （出货 `fnmsub d25,d29,d29,d25`）。
    //
    // **第二个因子的顺序不能照参数名的顺序写。** 内核 `:264` 的 GIMPLE 是
    //   `_63 = ome * 3.508 ; _64 = _63 * omc`   （`:266` 是 `_70 = omp * 3.8 ; _71 = _70 * oms`）
    // 即 `4.*atheta*ome*omc` 左结合成 `((4.*atheta)*ome)*omc` —— **先乘 `ome`（电子传输项）**，
    // 而 `:266` 先乘的是 `omp`（这里叫 `first`）。原先把两处都写成"常量 * rubisco * electron"，
    // 第一处就成了 `(3.508*rubisco)*electron`：乘法可交换但**舍入点不同**，
    // `(3.508*a)*b != (3.508*b)*a`。
    // 实测（`oracle/scripts/compare_stomata.sh` 第 515 例，Ball-Berry）：
    //   `(3.508*rubisco)*electron` ⇒ `omp=…E352`、`assim=3EF2E4791654E3CA`（正是改前 Rust 的值）
    //   `(3.508*electron)*rubisco` ⇒ `omp=…E3E4`、`assim=3EF2E4791654E451`（内核值）
    // 该例的 `omc/ome/oms/pco2i` 两侧**逐位相同**，所以差就出在这一处结合顺序上；
    // 又因为它落在 `(a+b)^2 - 4θab` 这个相消量上，135 ULP 被放大出来。
    let first = ((rubisco + electron)
        - (rubisco + electron)
            .mul_add(
                rubisco + electron,
                -(f77(4.0) * f77(0.877) * electron * rubisco),
            )
            .max(0.0)
            .sqrt())
        / (f77(2.0) * f77(0.877));
    ((sink + first)
        - (sink + first)
            .mul_add(sink + first, -(f77(4.0) * f77(0.95) * first * sink))
            .max(0.0)
            .sqrt())
    .max(0.0)
        / (f77(2.0) * f77(0.95))
}

fn selected_parameters(
    b: LeafBiochemistry,
    lambda: f64,
    options: StomataOptions,
) -> Result<(f64, f64, f64, f64, f64)> {
    let mut g1 = b.medlyn_g1;
    let mut g0 = b.medlyn_g0;
    let mut gradm = b.ball_berry_slope;
    let mut binter = b.ball_berry_intercept;
    let mut lambda = lambda;
    if options.use_medlyn {
        if let Some(value) = options.medlyn_g1_override.filter(|value| *value >= 0.0) {
            g1 = value;
        }
        if let Some(value) = options.medlyn_g0_override.filter(|value| *value >= 0.0) {
            g0 = value;
        }
    } else if options.use_wue {
        if let Some(value) = options.wue_lambda_override.filter(|value| *value > 0.0) {
            lambda = value;
        }
    } else {
        if let Some(value) = options
            .ball_berry_slope_override
            .filter(|value| *value > f77(1.6))
        {
            gradm = value;
        }
        if let Some(value) = options
            .ball_berry_intercept_override
            .filter(|value| *value >= 0.0)
        {
            binter = value;
        }
    }
    ensure!(
        [g1, g0, gradm, binter, lambda]
            .iter()
            .all(|value| value.is_finite())
            && gradm != 0.0
            && (!options.use_wue || lambda > 0.0),
        "stomatal options are invalid"
    );
    Ok((g1, g0, gradm, binter, lambda))
}

/// 供 `sortin` 闭环探针调用（第 333 轮）。**不是**业务 API —— 只为把
/// `sortin` 的形状残差夹在一个函数里逐位判（`compare_sortin.sh`）。
#[doc(hidden)]
pub fn sortin_for_probe(
    errors: &mut [f64; ITERATIONS],
    guesses: &mut [f64; ITERATIONS],
    range: f64,
    gamma: f64,
    iteration: usize,
) {
    sortin(errors, guesses, range, gamma, iteration);
}

/// 第 335 轮：把 `sortin` 二次拟合分支的中间量导出，供闭环定位**第一处分叉**。
/// 顺序：`ac1, ac2, bc1, bc2, cc1, cc2, bterm, aterm, cterm`（`iteration < 4` 时全 0）。
#[doc(hidden)]
pub fn sortin_intermediates_for_probe(
    errors: &mut [f64; ITERATIONS],
    guesses: &mut [f64; ITERATIONS],
    range: f64,
    gamma: f64,
    iteration: usize,
) -> [f64; 9] {
    let mut debug = [0.0; 9];
    sortin_impl(errors, guesses, range, gamma, iteration, Some(&mut debug));
    debug
}

fn sortin(
    errors: &mut [f64; ITERATIONS],
    co2: &mut [f64; ITERATIONS],
    range: f64,
    gamma: f64,
    iteration: usize,
) {
    sortin_impl(errors, co2, range, gamma, iteration, None);
}

fn sortin_impl(
    errors: &mut [f64; ITERATIONS],
    co2: &mut [f64; ITERATIONS],
    range: f64,
    gamma: f64,
    iteration: usize,
    debug: Option<&mut [f64; 9]>,
) {
    if iteration < 4 {
        let error_sign = if errors[0] < 0.0 { -1.0 } else { 1.0 };
        co2[0] = f77(0.5).mul_add(range, gamma);
        co2[1] = range.mul_add(f77(0.5) - f77(0.3) * error_sign, gamma);
        let slope = (co2[0] - co2[1]) / (errors[0] - errors[1] + f77(1.0e-10));
        co2[2] = (-slope).mul_add(errors[0], co2[0]);
        let pmin = co2[0].min(co2[1]);
        let emin = errors[0].min(errors[1]);
        if emin > 0.0 && co2[2] > pmin {
            co2[2] = gamma;
        }
    } else {
        let n = iteration - 1;
        for j in 1..n {
            let error = errors[j];
            let guess = co2[j];
            let mut i = j;
            while i > 0 && errors[i - 1] > error {
                errors[i] = errors[i - 1];
                co2[i] = co2[i - 1];
                i -= 1;
            }
            errors[i] = error;
            co2[i] = guess;
        }
        let mut lower = 0.0;
        let mut index = 0;
        for ix in 0..n {
            if errors[ix] < 0.0 {
                lower = co2[ix];
                index = ix;
            }
        }
        let i1 = index.saturating_sub(1).clamp(0, n - 3);
        let i2 = i1 + 1;
        let i3 = i1 + 2;
        let isp = (index + 1).min(n - 1);
        let is = isp - 1;
        let slope = (co2[is] - co2[isp]) / (errors[is] - errors[isp] + f77(1.0e-10));
        let linear = (-slope).mul_add(errors[is], co2[is]);
        let error_two_squared = errors[i2] * errors[i2];
        let ac1 = errors[i1].mul_add(errors[i1], -error_two_squared);
        let ac2 = (-errors[i3]).mul_add(errors[i3], error_two_squared);
        let bc1 = errors[i1] - errors[i2];
        let bc2 = errors[i2] - errors[i3];
        let cc1 = co2[i1] - co2[i2];
        let cc2 = co2[i2] - co2[i3];
        let bterm =
            ac2.mul_add(cc1, -(cc2 * ac1)) / (ac2.mul_add(bc1, -(ac1 * bc2)) + f77(1.0e-10));
        let aterm = (-bc1).mul_add(bterm, cc1) / (ac1 + f77(1.0e-10));
        let first = (-(aterm * errors[i2])).mul_add(errors[i2], co2[i2]);
        let cterm = (-bterm).mul_add(errors[i2], first);
        let quadratic = cterm.max(lower);
        co2[iteration - 1] = f77(0.5) * (linear + quadratic);
        if let Some(slot) = debug {
            slot[0] = ac1;
            slot[1] = ac2;
            slot[2] = bc1;
            slot[3] = bc2;
            slot[4] = cc1;
            slot[5] = cc2;
            slot[6] = bterm;
            slot[7] = aterm;
            slot[8] = cterm;
        }
    }
    co2[iteration - 1] = co2[iteration - 1].max(f77(0.01));
}

fn wue_internal_co2(
    gamma: f64,
    lambda: f64,
    canopy_co2: f64,
    leaf_vapor_pressure: f64,
    air_vapor_pressure: f64,
    air_pressure: f64,
) -> (f64, f64) {
    let vapor_difference = (leaf_vapor_pressure - air_vapor_pressure).max(f77(50.0)) / air_pressure;
    let rubisco = canopy_co2
        - (f77(1.6) * vapor_difference * (canopy_co2 - gamma / air_pressure).max(0.0) / lambda)
            .sqrt();
    // `:861` 的 GIMPLE 是 `_368 = .FMA(sqrt(lambda*gammas/psrf/D), 1.37, 1.0)`。
    let electron = canopy_co2
        - canopy_co2
            / f77(1.37).mul_add(
                (lambda * gamma / air_pressure / vapor_difference).sqrt(),
                1.0,
            );
    (rubisco * air_pressure, electron * air_pressure)
}

fn validate_photosynthesis(input: LeafPhotosynthesisInput) -> Result<()> {
    let b = input.biochemistry;
    ensure!(
        [
            b.quantum_efficiency,
            b.maximum_carboxylation_25c_mol_m2_s,
            b.low_temperature_slope,
            b.low_temperature_half_k,
            b.high_temperature_slope,
            b.high_temperature_half_k,
            b.respiration_temperature_slope,
            b.respiration_temperature_half_k,
            b.optimum_temperature_k,
            b.medlyn_g1,
            b.medlyn_g0,
            b.ball_berry_slope,
            b.ball_berry_intercept,
            input.canopy_integration[0],
            input.canopy_integration[1],
            input.canopy_integration[2],
            input.leaf_temperature_k,
            input.oxygen_partial_pressure_pa,
            input.absorbed_par_w_m2,
            input.air_pressure_pa,
            input.soil_water_stress,
            input.leaf_boundary_resistance_s_m,
        ]
        .iter()
        .all(|value| value.is_finite())
            && matches!(b.c3c4, 0 | 1)
            && b.maximum_carboxylation_25c_mol_m2_s >= 0.0
            && b.optimum_temperature_k > 0.0
            && input.leaf_temperature_k > 0.0
            && input.oxygen_partial_pressure_pa >= 0.0
            && input.absorbed_par_w_m2 >= 0.0
            && input.air_pressure_pa > 0.0
            && input.soil_water_stress >= 0.0
            && input.leaf_boundary_resistance_s_m > 0.0,
        "leaf photosynthesis inputs are invalid"
    );
    Ok(())
}

fn validate_stomata(input: StomataInput, options: StomataOptions) -> Result<()> {
    validate_photosynthesis(input.photosynthesis)?;
    ensure!(
        [
            input.atmospheric_co2_pa,
            input.canopy_air_co2_pa,
            input.canopy_air_vapor_pressure_pa,
            input.leaf_saturation_vapor_pressure_pa,
            input.wue_lambda,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.atmospheric_co2_pa >= 0.0
            && input.canopy_air_co2_pa >= 0.0
            && input.canopy_air_vapor_pressure_pa >= 0.0
            && input.leaf_saturation_vapor_pressure_pa >= 0.0,
        "stomata inputs are invalid"
    );
    selected_parameters(input.photosynthesis.biochemistry, input.wue_lambda, options)?;
    Ok(())
}

#[cfg(test)]
#[path = "photosynthesis_tests.rs"]
mod photosynthesis_tests;
