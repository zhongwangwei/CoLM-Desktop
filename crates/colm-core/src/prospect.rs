//! CoLM's PROSPECT-D green-leaf optical-property update.
//!
//! This is a direct Rust translation of `MOD_prospect_DB.F90`,
//! `MOD_tav_abs.F90`, and `update_params_PROSPECT` in
//! `MOD_HighRes_Parameters.F90`.  It returns the 211 CoLM wavelengths
//! (400--2500 nm, every 10 nm) while retaining the supplied dead-stem spectrum.

use anyhow::{ensure, Result};

use crate::{HighResolutionLeafOptics, HIGH_RES_WAVELENGTHS};

const PROSPECT_WAVELENGTHS: usize = 2101;
const PFT_CLASSES: usize = 16;
const SAMPLE_INTERVAL: usize = 10;

mod data {
    use super::PROSPECT_WAVELENGTHS;

    include!("prospect_data.rs");
}

/// The 211-band output of CoLM's `update_params_PROSPECT`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProspectLeafOptics {
    /// `(wavelength, green-leaf-or-dead-stem)` reflectance.
    pub reflectance: Vec<f64>,
    /// `(wavelength, green-leaf-or-dead-stem)` transmittance.
    pub transmittance: Vec<f64>,
}

/// Updates a PFT's green-leaf spectrum through CoLM's PROSPECT-D parameterization.
///
/// `soil_moisture` is the upstream volumetric `ssw` value.  The green tissue is
/// regenerated from the fixed 16-PFT PROSPECT parameter table; the dead stem is
/// copied unchanged from the leaf-optics input, exactly as `update_params_PROSPECT`.
pub fn prospect_leaf_optics(
    pft_class: usize,
    soil_moisture: f64,
    input: HighResolutionLeafOptics<'_>,
) -> Result<ProspectLeafOptics> {
    ensure!(
        pft_class > 0 && pft_class < PFT_CLASSES,
        "PROSPECT requires a vegetated CoLM PFT class in 1..{PFT_CLASSES}, got {pft_class}"
    );
    ensure!(
        soil_moisture.is_finite(),
        "PROSPECT soil moisture must be finite"
    );
    ensure!(
        input.reflectance.len() == HIGH_RES_WAVELENGTHS * 2
            && input.transmittance.len() == HIGH_RES_WAVELENGTHS * 2
            && input
                .reflectance
                .iter()
                .chain(input.transmittance)
                .all(|value| value.is_finite()),
        "PROSPECT leaf optics must contain 211 finite wavelength-by-tissue values"
    );

    // These update_params_PROSPECT literals are default Fortran REAL before
    // assignment to r8, so retain their f32-to-f64 rounding exactly.
    let r8 = f64::from;
    let sla = r8([
        0.0_f32, 0.0100, 0.0100, 0.0202, 0.0190, 0.0190, 0.0308, 0.0308, 0.0308, 0.0180, 0.0307,
        0.0307, 0.0402, 0.0402, 0.0385, 0.0402,
    ][pft_class]);
    let vmax25 = r8([
        52.0_f32, 55.0, 42.0, 29.0, 41.0, 51.0, 36.0, 30.0, 40.0, 36.0, 30.0, 19.0, 21.0, 26.0,
        25.0, 57.0,
    ][pft_class]
        * 1.0e-6_f32);
    let n =
        (r8(0.9_f32) * (sla * r8(10.0_f32)) + r8(0.025_f32)) / (sla * r8(10.0_f32) - r8(0.01_f32));
    let cm = r8(1.0_f32) / (sla * r8(1.0e4_f32));
    let cab = (vmax25 * r8(1.0e6_f32) - r8(3.72_f32)) / r8(1.3_f32);
    let cw = r8(0.01_f32) - (r8(0.01_f32) - r8(0.0_f32)) * (-r8(5.5_f32) * soil_moisture).exp();
    let (reflectance_green, transmittance_green) =
        prospect_spectrum(n, cab, r8(8.0_f32), r8(0.0_f32), r8(0.01_f32), cw, cm)?;

    let mut reflectance = Vec::with_capacity(HIGH_RES_WAVELENGTHS * 2);
    let mut transmittance = Vec::with_capacity(HIGH_RES_WAVELENGTHS * 2);
    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        reflectance.extend([
            reflectance_green[wavelength],
            input.reflectance[wavelength * 2 + 1],
        ]);
        transmittance.extend([
            transmittance_green[wavelength],
            input.transmittance[wavelength * 2 + 1],
        ]);
    }
    Ok(ProspectLeafOptics {
        reflectance,
        transmittance,
    })
}

#[allow(clippy::too_many_arguments)]
fn prospect_spectrum(
    n: f64,
    cab: f64,
    car: f64,
    anth: f64,
    cbrown: f64,
    cw: f64,
    cm: f64,
) -> Result<(Vec<f64>, Vec<f64>)> {
    ensure!(
        [n, cab, car, anth, cbrown, cw, cm]
            .into_iter()
            .all(f64::is_finite)
            && n > 0.0,
        "PROSPECT parameters must be finite and N must be positive"
    );

    let tav_90 = tav(90.0)?;
    let tav_40 = tav(40.0)?;
    let mut reflectance = Vec::with_capacity(HIGH_RES_WAVELENGTHS);
    let mut transmittance = Vec::with_capacity(HIGH_RES_WAVELENGTHS);
    for wavelength in (0..PROSPECT_WAVELENGTHS).step_by(SAMPLE_INTERVAL) {
        let k = (cab * data::CHLOROPHYLL_ABSORPTION[wavelength]
            + car * data::CAROTENOID_ABSORPTION[wavelength]
            + anth * data::ANTHOCYANIN_ABSORPTION[wavelength]
            + cbrown * data::BROWN_PIGMENT_ABSORPTION[wavelength]
            + cw * data::WATER_ABSORPTION[wavelength]
            + cm * data::DRY_MATTER_ABSORPTION[wavelength])
            / n;
        let tau = absorption_transmittance(k);
        let refractive = data::REFRACTIVE_INDEX[wavelength];
        let t12 = tav_90[wavelength];
        let talf = tav_40[wavelength];
        let ralf = 1.0 - talf;
        let r12 = 1.0 - t12;
        let t21 = t12 / refractive.powi(2);
        let r21 = 1.0 - t21;
        let denominator = 1.0 - r21 * r21 * tau * tau;
        ensure!(
            denominator.is_finite() && denominator.abs() > f64::EPSILON,
            "PROSPECT encountered a singular leaf-layer denominator"
        );
        let ta = talf * tau * t21 / denominator;
        let ra = ralf + r21 * tau * ta;
        let t = t12 * tau * t21 / denominator;
        let r = r12 + r21 * tau * t;
        let d = ((1.0 + r + t) * (1.0 + r - t) * (1.0 - r + t) * (1.0 - r - t)).sqrt();
        let rq = r * r;
        let tq = t * t;
        ensure!(
            r.abs() > f64::EPSILON && t.abs() > f64::EPSILON,
            "PROSPECT encountered a singular leaf-stack coefficient"
        );
        let a = (1.0 + rq - tq + d) / (2.0 * r);
        let b = (1.0 - rq + tq + d) / (2.0 * t);
        let b_nm1 = b.powf(n - 1.0);
        let b_n2 = b_nm1 * b_nm1;
        let a2 = a * a;
        let denominator = a2 * b_n2 - 1.0;
        ensure!(
            denominator.is_finite() && denominator.abs() > f64::EPSILON,
            "PROSPECT encountered a singular leaf-stack denominator"
        );
        let (rsub, tsub) = if r + t >= 1.0 {
            let denominator = t + (1.0 - t) * (n - 1.0);
            ensure!(
                denominator.is_finite() && denominator.abs() > f64::EPSILON,
                "PROSPECT encountered a singular zero-absorption leaf stack"
            );
            (1.0 - t / denominator, t / denominator)
        } else {
            (
                a * (b_n2 - 1.0) / denominator,
                b_nm1 * (a2 - 1.0) / denominator,
            )
        };
        let denominator = 1.0 - rsub * r;
        ensure!(
            denominator.is_finite() && denominator.abs() > f64::EPSILON,
            "PROSPECT encountered a singular leaf reflectance denominator"
        );
        let leaf_transmittance = ta * tsub / denominator;
        let leaf_reflectance = ra + ta * rsub * t / denominator;
        ensure!(
            leaf_reflectance.is_finite() && leaf_transmittance.is_finite(),
            "PROSPECT produced a non-finite leaf spectrum"
        );
        reflectance.push(leaf_reflectance);
        transmittance.push(leaf_transmittance);
    }
    debug_assert_eq!(reflectance.len(), HIGH_RES_WAVELENGTHS);
    Ok((reflectance, transmittance))
}

#[allow(clippy::excessive_precision)]
fn absorption_transmittance(k: f64) -> f64 {
    const SMALL_K_COEFFICIENTS: [f64; 19] = [
        -3.603_112_304_826_122_24e-13,
        3.463_485_265_540_874_24e-12,
        -2.996_273_996_041_289_73e-11,
        2.577_478_071_069_885_89e-10,
        -2.093_305_684_354_883_03e-9,
        1.595_013_299_369_878_18e-8,
        -1.137_179_002_854_288_95e-7,
        7.552_928_853_091_529_56e-7,
        -4.649_807_514_806_194_31e-6,
        2.638_303_656_754_081_29e-5,
        -1.370_898_709_788_305_76e-4,
        6.476_865_037_281_034e-4,
        -2.760_601_413_436_279_83e-3,
        1.053_060_346_874_495_05e-2,
        -3.571_913_487_536_319_56e-2,
        1.077_745_279_389_786_92e-1,
        -2.969_970_751_450_809_63e-1,
        8.646_647_167_633_873_11e-1,
        7.420_476_912_680_064_29e-1,
    ];
    const LARGE_K_COEFFICIENTS: [f64; 20] = [
        -1.628_065_708_684_607_49e-12,
        -8.954_005_793_182_842_88e-13,
        -4.083_527_028_381_515_78e-12,
        -1.451_329_882_485_374_98e-11,
        -8.350_869_189_407_578_52e-11,
        -2.136_386_789_537_662_89e-10,
        -1.103_024_314_670_697_70e-9,
        -3.671_289_156_334_554_84e-9,
        -1.669_805_443_041_047_26e-8,
        -6.117_743_864_012_951_25e-8,
        -2.703_061_636_102_714_97e-7,
        -1.055_650_069_928_912_61e-6,
        -4.720_904_672_037_114_84e-6,
        -1.950_763_750_899_559_37e-5,
        -9.164_504_829_312_214_53e-5,
        -4.058_921_304_521_286_77e-4,
        -2.142_130_550_003_347_18e-3,
        -1.063_748_751_165_696_57e-2,
        -8.506_991_549_845_718_71e-2,
        9.237_553_078_077_840_58e-1,
    ];

    if k <= 0.0 {
        return 1.0;
    }
    if k > 85.0 {
        return 0.0;
    }
    let exp = (-k).exp();
    let y = if k <= 4.0 {
        horner(&SMALL_K_COEFFICIENTS, 0.5 * k - 1.0) - k.ln()
    } else {
        exp * horner(&LARGE_K_COEFFICIENTS, 14.5 / (k + 3.25) - 1.0) / k
    };
    (1.0 - k) * exp + k * k * y
}

fn horner(coefficients: &[f64], x: f64) -> f64 {
    coefficients[1..]
        .iter()
        .copied()
        .fold(coefficients[0], |value, coefficient| {
            value * x + coefficient
        })
}

fn tav(theta_degrees: f64) -> Result<Vec<f64>> {
    // `MOD_tav_abs` evaluates `atan(1.) * 4.` in default REAL before assigning to r8.
    let upstream_pi = f64::from((1.0_f32).atan() * 4.0_f32);
    let sin_theta = (theta_degrees * upstream_pi / 180.0).sin();
    let sin_theta_squared = sin_theta * sin_theta;
    let mut values = Vec::with_capacity(PROSPECT_WAVELENGTHS);
    for &refractive in &data::REFRACTIVE_INDEX {
        let refractive_squared = refractive * refractive;
        let np = refractive_squared + 1.0;
        let nm = refractive_squared - 1.0;
        let a = (refractive + 1.0).powi(2) / 2.0;
        let k = -nm.powi(2) / 4.0;
        let b1 = if theta_degrees == 90.0 {
            0.0
        } else {
            ((sin_theta_squared - np / 2.0).powi(2) + k).sqrt()
        };
        let b = b1 - (sin_theta_squared - np / 2.0);
        let b3 = b.powi(3);
        let a3 = a.powi(3);
        let ts =
            (k.powi(2) / (6.0 * b3) + k / b - b / 2.0) - (k.powi(2) / (6.0 * a3) + k / a - a / 2.0);
        let denominator = np.powi(3) * nm.powi(2);
        ensure!(
            b.is_finite()
                && a.is_finite()
                && denominator.is_finite()
                && denominator.abs() > f64::EPSILON,
            "PROSPECT encountered a singular interface transmittance"
        );
        let tp = -2.0 * refractive_squared * (b - a) / np.powi(2)
            - 2.0 * refractive_squared * np * (b / a).ln() / nm.powi(2)
            + refractive_squared * (1.0 / b - 1.0 / a) / 2.0
            + 16.0
                * refractive_squared.powi(2)
                * (refractive_squared.powi(2) + 1.0)
                * ((2.0 * np * b - nm.powi(2)) / (2.0 * np * a - nm.powi(2))).ln()
                / denominator
            + 16.0
                * refractive_squared.powi(3)
                * (1.0 / (2.0 * np * b - nm.powi(2)) - 1.0 / (2.0 * np * a - nm.powi(2)))
                / np.powi(3);
        let value = (ts + tp) / (2.0 * sin_theta_squared);
        ensure!(
            value.is_finite(),
            "PROSPECT produced a non-finite interface transmittance"
        );
        values.push(value);
    }
    Ok(values)
}

#[cfg(test)]
#[path = "prospect_tests.rs"]
mod prospect_tests;
