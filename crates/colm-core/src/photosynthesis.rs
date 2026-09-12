//! Leaf photosynthesis and stomatal conductance from `MOD_AssimStomataConductance.F90`.
//!
//! The leaf-temperature solver supplies air state and shared canopy resistances;
//! this module owns only the biochemistry/CO₂ iteration so it is reusable by the
//! ordinary and plant-hydraulic canopy paths.

use anyhow::{ensure, Result};

const fn f77(value: f32) -> f64 {
    value as f64
}

const ITERATIONS: usize = 6;

/// Land-cover/PFT biochemical parameters shared by `stomata` and `update_photosyn`.
#[derive(Debug, Clone, Copy)]
pub struct LeafBiochemistry {
    pub quantum_efficiency: f64,
    pub maximum_carboxylation_25c_mol_m2_s: f64,
    /// One for C3 and zero for C4, matching CoLM's `c3c4` flag.
    pub c3c4: i32,
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
    pub canopy_scaling: [f64; 3],
}

/// Inputs common to CoLM's private `calc_photo_params` and public photo updates.
#[derive(Debug, Clone, Copy)]
pub struct LeafPhotosynthesisInput {
    pub biochemistry: LeafBiochemistry,
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
    pub canopy_conductance_h2o_mol_m2_s: f64,
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
    let kc = f77(30.0) * f77(2.1).powf(temperature_factor);
    let ko = f77(30_000.0) * f77(1.2).powf(temperature_factor);
    let co2_compensation_pa = f77(0.5) * input.oxygen_partial_pressure_pa
        / (f77(2600.0) * f77(0.57).powf(temperature_factor))
        * c3_fraction;
    let rubisco_co2_constant_pa = kc * (1.0 + input.oxygen_partial_pressure_pa / ko) * c3_fraction;
    let low_inhibition = 1.0
        + (b.low_temperature_slope * (b.low_temperature_half_k - input.leaf_temperature_k)).exp();
    let high_inhibition = 1.0
        + (b.high_temperature_slope * (input.leaf_temperature_k - b.high_temperature_half_k)).exp();
    let mut maximum_carboxylation =
        b.maximum_carboxylation_25c_mol_m2_s * f77(2.1).powf(temperature_factor);
    maximum_carboxylation =
        (maximum_carboxylation / high_inhibition * input.soil_water_stress * c3_fraction
            + maximum_carboxylation / (low_inhibition * high_inhibition)
                * input.soil_water_stress
                * c4_fraction)
            * b.canopy_scaling[0];
    let gas_constant = 8.314_467_591;
    let jmax25 = f77(1.97) * b.maximum_carboxylation_25c_mol_m2_s;
    let mut jmax = jmax25
        * (f77(37.0e3) * (input.leaf_temperature_k - b.optimum_temperature_k)
            / (gas_constant * b.optimum_temperature_k * input.leaf_temperature_k))
            .exp()
        * (1.0
            + ((f77(710.0) * b.optimum_temperature_k - f77(220.0e3))
                / (gas_constant * b.optimum_temperature_k))
                .exp())
        / (1.0
            + ((f77(710.0) * input.leaf_temperature_k - f77(220.0e3))
                / (gas_constant * input.leaf_temperature_k))
                .exp());
    jmax *= input.soil_water_stress * b.canopy_scaling[1];
    let electron_transport =
        (f77(4.6e-6) * input.absorbed_par_w_m2 * b.quantum_efficiency).min(jmax);
    let respiration_fraction = f77(0.015) * c3_fraction + f77(0.025) * c4_fraction;
    let respiration = respiration_fraction
        * b.maximum_carboxylation_25c_mol_m2_s
        * f77(2.0).powf(temperature_factor)
        / (1.0
            + (b.respiration_temperature_slope
                * (input.leaf_temperature_k - b.respiration_temperature_half_k))
                .exp())
        * input.soil_water_stress
        * b.canopy_scaling[0];
    let sink_limit = ((b.maximum_carboxylation_25c_mol_m2_s / f77(2.0))
        * f77(1.8).powf(temperature_factor)
        / low_inhibition
        * input.soil_water_stress
        * c3_fraction
        + (b.maximum_carboxylation_25c_mol_m2_s / f77(5.0))
            * f77(1.8).powf(temperature_factor)
            * input.soil_water_stress
            * c4_fraction)
        * b.canopy_scaling[0];
    let pressure_conversion = f77(44.6_f32 * 273.16_f32) * input.air_pressure_pa / f77(1.013e5);
    let boundary_conductance_h2o =
        pressure_conversion / (input.leaf_boundary_resistance_s_m * input.leaf_temperature_k);
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
    let bintc = binter * input.photosynthesis.soil_water_stress.max(f77(0.1)) * b.canopy_scaling[2];
    let range = input.atmospheric_co2_pa * (1.0 - f77(1.6) / gradm) - photo.co2_compensation_pa;
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
        let omc = photo.maximum_carboxylation_mol_m2_s * (rubisco_co2 - photo.co2_compensation_pa)
            / (rubisco_co2 + photo.rubisco_co2_constant_pa)
            * photo.c3_fraction
            + photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction;
        let ome = photo.electron_transport_mol_m2_s * (electron_co2 - photo.co2_compensation_pa)
            / (electron_co2 + f77(2.0) * photo.co2_compensation_pa)
            * photo.c3_fraction
            + photo.electron_transport_mol_m2_s * photo.c4_fraction;
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.sink_limit_mol_m2_s_pa * photo.c3_fraction
                + photo.sink_limit_mol_m2_s_pa * internal_co2 * photo.c4_fraction;
            assimilation = coupled_assimilation(omc, ome, oms);
        } else {
            assimilation = omc.min(ome).max(0.0);
        }
        let net_assimilation = assimilation - photo.respiration_mol_m2_s;
        let co2_surface = input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa
            - f77(1.37) * net_assimilation / photo.boundary_conductance_h2o_mol_m2_s;
        let co2_surface = co2_surface
            .min(input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa)
            .max(f77(1.0e-5));
        let positive_assimilation = net_assimilation.max(f77(1.0e-12));
        let next_co2 = if options.use_wue && (photo.c4_fraction - 1.0).abs() >= f77(0.001) {
            conductance = positive_assimilation
                / (input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa
                    - internal_co2 / input.photosynthesis.air_pressure_pa)
                * f77(1.6);
            internal_co2
        } else if options.use_medlyn {
            let vapor_deficit_kpa = (input.leaf_saturation_vapor_pressure_pa
                - input.canopy_air_vapor_pressure_pa)
                .max(f77(50.0))
                * f77(1.0e-3);
            let acp = f77(1.6) * positive_assimilation / co2_surface;
            let a = 1.0;
            let bq = -f77(2.0) * (g0 * f77(1.0e-6) + acp)
                - (g1 * acp).powi(2)
                    / (photo.boundary_conductance_h2o_mol_m2_s * vapor_deficit_kpa);
            let c = (g0 * f77(1.0e-6)).powi(2)
                + (f77(2.0) * g0 * f77(1.0e-6) + acp * (1.0 - g1.powi(2)) / vapor_deficit_kpa)
                    * acp;
            conductance = (-bq + (bq.powi(2) - f77(4.0) * a * c).max(0.0).sqrt()) / (f77(2.0) * a);
            (co2_surface - f77(1.6) * net_assimilation / conductance)
                * input.photosynthesis.air_pressure_pa
        } else {
            let hcdma = input.leaf_saturation_vapor_pressure_pa * co2_surface
                / (gradm * positive_assimilation);
            let a = hcdma;
            let bq = photo.boundary_conductance_h2o_mol_m2_s * hcdma
                - input.leaf_saturation_vapor_pressure_pa
                - bintc * hcdma;
            let c = -photo.boundary_conductance_h2o_mol_m2_s
                * (input.canopy_air_vapor_pressure_pa + hcdma * bintc);
            conductance = (-bq + (bq.powi(2) - f77(4.0) * a * c).max(0.0).sqrt()) / (f77(2.0) * a);
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
        f77(44.6_f32 * 273.16_f32) * input.photosynthesis.air_pressure_pa / f77(1.013e5);
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
            input.canopy_conductance_h2o_mol_m2_s,
        ]
        .iter()
        .all(|value| value.is_finite())
            && input.atmospheric_co2_pa >= 0.0
            && input.canopy_air_co2_pa >= 0.0
            && input.canopy_conductance_h2o_mol_m2_s > 0.0,
        "photosynthesis update inputs are invalid"
    );
    let photo = photosynthesis_parameters(input.photosynthesis)?;
    let (_, _, gradm, _, _) = selected_parameters(input.photosynthesis.biochemistry, 1.0, options)?;
    let range = input.atmospheric_co2_pa * (1.0 - f77(1.6) / gradm) - photo.co2_compensation_pa;
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
        let omc = photo.maximum_carboxylation_mol_m2_s * (internal - photo.co2_compensation_pa)
            / (internal + photo.rubisco_co2_constant_pa)
            * photo.c3_fraction
            + photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction;
        let ome = photo.electron_transport_mol_m2_s * (internal - photo.co2_compensation_pa)
            / (internal + f77(2.0) * photo.co2_compensation_pa)
            * photo.c3_fraction
            + photo.electron_transport_mol_m2_s * photo.c4_fraction;
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.sink_limit_mol_m2_s_pa * photo.c3_fraction
                + photo.sink_limit_mol_m2_s_pa * internal * photo.c4_fraction;
            assimilation = coupled_assimilation(omc, ome, oms);
        } else {
            assimilation = omc.min(ome).max(0.0);
        }
        let net_assimilation = assimilation - photo.respiration_mol_m2_s;
        let co2_surface = (input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa
            - f77(1.37) * net_assimilation / photo.boundary_conductance_h2o_mol_m2_s)
            .min(input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa)
            .max(f77(1.0e-5));
        let positive_assimilation = net_assimilation.max(f77(1.0e-12));
        let next = (co2_surface
            - f77(1.6) * positive_assimilation / input.canopy_conductance_h2o_mol_m2_s)
            * input.photosynthesis.air_pressure_pa;
        errors[iteration - 1] = internal - next;
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
    let first = ((rubisco + electron)
        - ((rubisco + electron).powi(2) - f77(4.0) * f77(0.877) * rubisco * electron)
            .max(0.0)
            .sqrt())
        / (f77(2.0) * f77(0.877));
    ((sink + first)
        - ((sink + first).powi(2) - f77(4.0) * f77(0.95) * first * sink)
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

fn sortin(
    errors: &mut [f64; ITERATIONS],
    co2: &mut [f64; ITERATIONS],
    range: f64,
    gamma: f64,
    iteration: usize,
) {
    if iteration < 4 {
        let error_sign = if errors[0] < 0.0 { -1.0 } else { 1.0 };
        co2[0] = gamma + f77(0.5) * range;
        co2[1] = gamma + range * (f77(0.5) - f77(0.3) * error_sign);
        co2[2] = co2[0] - (co2[0] - co2[1]) / (errors[0] - errors[1] + f77(1.0e-10)) * errors[0];
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
        let linear =
            co2[is] - (co2[is] - co2[isp]) / (errors[is] - errors[isp] + f77(1.0e-10)) * errors[is];
        let ac1 = errors[i1].powi(2) - errors[i2].powi(2);
        let ac2 = errors[i2].powi(2) - errors[i3].powi(2);
        let bc1 = errors[i1] - errors[i2];
        let bc2 = errors[i2] - errors[i3];
        let cc1 = co2[i1] - co2[i2];
        let cc2 = co2[i2] - co2[i3];
        let bterm = (cc1 * ac2 - cc2 * ac1) / (bc1 * ac2 - ac1 * bc2 + f77(1.0e-10));
        let aterm = (cc1 - bc1 * bterm) / (ac1 + f77(1.0e-10));
        let quadratic = (co2[i2] - aterm * errors[i2].powi(2) - bterm * errors[i2]).max(lower);
        co2[iteration - 1] = f77(0.5) * (linear + quadratic);
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
    let electron = canopy_co2
        - canopy_co2
            / (1.0 + f77(1.37) * (lambda * gamma / air_pressure / vapor_difference).sqrt());
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
            b.canopy_scaling[0],
            b.canopy_scaling[1],
            b.canopy_scaling[2],
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
