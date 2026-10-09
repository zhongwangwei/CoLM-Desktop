//! CoLM's PROSPECT-D green-leaf optical-property update.
//!
//! This is a direct Rust translation of `MOD_prospect_DB.F90`,
//! `MOD_tav_abs.F90`, and `update_params_PROSPECT` in
//! `MOD_HighRes_Parameters.F90`.  It returns the 211 CoLM wavelengths
//! (400--2500 nm, every 10 nm) while retaining the supplied dead-stem spectrum.
//!
//! 按 CoLM 构建选项（`-fdefault-real-8 -fdefault-double-8`，后者见 upstream-bugs 第 17 条）
//! 的 GIMPLE 逐条对齐：默认 REAL 字面量与 `d` 字面量都是 real(8)，`.FMA/.FMS/.FNMA` 写成
//! `mul_add`，其余照 GIMPLE 的次序分开舍入。

use crate::LibmPow;
use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::{HighResolutionLeafOptics, HIGH_RES_WAVELENGTHS};

const PROSPECT_WAVELENGTHS: usize = 2101;
const PFT_CLASSES: usize = 16;
const SAMPLE_INTERVAL: usize = 10;

// 光谱数据是实测表，个别值恰好接近 `LOG2_E`、`SQRT_2` 等常数。
#[allow(clippy::approx_constant)]
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

    // `update_params_PROSPECT`（`MOD_HighRes_Parameters.F90:296-347`）：`-fdefault-real-8` 下
    // 字面量都是 f64；`vmax25_p(...) * 1.e-6` 在编译期按 f64 乘法折叠。
    let sla: f64 = [
        0.0, 0.0100, 0.0100, 0.0202, 0.0190, 0.0190, 0.0308, 0.0308, 0.0308, 0.0180, 0.0307,
        0.0307, 0.0402, 0.0402, 0.0385, 0.0402,
    ][pft_class];
    let vmax25: f64 = [
        52.0, 55.0, 42.0, 29.0, 41.0, 51.0, 36.0, 30.0, 40.0, 36.0, 30.0, 19.0, 21.0, 26.0, 25.0,
        57.0,
    ][pft_class]
        * 1.0e-6;
    // `:338` `N = (0.9*(SLA*10.) + 0.025) / ((SLA*10.) - 0.01)`：分子是 `.FMA (sla*10, 0.9, 0.025)`
    let sla_10 = sla * 10.0;
    let n = sla_10.contract(0.9, 0.025) / (sla_10 - 0.01);
    let cm = 1.0 / (sla * 1.0e4);
    let cab = (vmax25 * 1.0e6 - 3.72) / 1.3;
    // `:347` `0.01 - ((0.01 - 0.)*exp(-5.5*soilmoisture))`：GIMPLE 为 `0.01 - exp(-(sm*5.5))*0.01`
    let cw = 0.01 - (-(soil_moisture * 5.5)).exp() * 0.01;
    let (reflectance_green, transmittance_green) =
        prospect_spectrum(n, cab, 8.0, 0.0, 0.01, cw, cm)?;

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
        // `:105`：`k_Car*Car` 先乘，其余逐项 `.FMA`，最后除以 N
        let k = data::DRY_MATTER_ABSORPTION[wavelength].contract(
            cm,
            data::WATER_ABSORPTION[wavelength].contract(
                cw,
                data::BROWN_PIGMENT_ABSORPTION[wavelength].contract(
                    cbrown,
                    data::ANTHOCYANIN_ABSORPTION[wavelength].contract(
                        anth,
                        data::CHLOROPHYLL_ABSORPTION[wavelength]
                            .contract(cab, data::CAROTENOID_ABSORPTION[wavelength] * car),
                    ),
                ),
            ),
        ) / n;
        let tau = absorption_transmittance(k);
        let refractive = data::REFRACTIVE_INDEX[wavelength];
        let t12 = tav_90[wavelength];
        let talf = tav_40[wavelength];
        let ralf = 1.0 - talf;
        let r12 = 1.0 - t12;
        let t21 = t12 / (refractive * refractive);
        let r21 = 1.0 - t21;
        // `:165` `1 - r21*r21*tau**2`：`.FNMA (tau², r21², 1)`
        let denominator = (-(tau * tau)).contract(r21 * r21, 1.0);
        let ta = talf * tau * t21 / denominator;
        let ra = (r21 * tau).contract(ta, ralf);
        let t = t12 * tau * t21 / denominator;
        let r = (r21 * tau).contract(t, r12);
        let one_plus_r = r + 1.0;
        let one_minus_r = 1.0 - r;
        let d =
            ((one_plus_r + t) * (one_plus_r - t) * (t + one_minus_r) * (one_minus_r - t)).sqrt();
        let rq = r * r;
        let tq = t * t;
        let a = (rq + 1.0 - tq + d) / (r * 2.0);
        let b = (1.0 - rq + tq + d) / (t * 2.0);
        let b_nm1 = b.lpow(n - 1.0);
        let b_n2 = b_nm1 * b_nm1;
        let a2 = a * a;
        // `where (r+t >= 1)` 只覆盖零吸收的波段；上游先按一般式算出 Rsub/Tsub 再覆盖
        let (rsub, tsub) = if t + r >= 1.0 {
            let tsub = t / (1.0 - t).contract(n - 1.0, t);
            (1.0 - tsub, tsub)
        } else {
            let denominator = a2.contract(b_n2, -1.0);
            (
                a * (b_n2 - 1.0) / denominator,
                b_nm1 * (a2 - 1.0) / denominator,
            )
        };
        // `:198-200`
        let denominator = (-rsub).contract(r, 1.0);
        let leaf_transmittance = tsub * ta / denominator;
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
    // `:120/135`：多项式整条是 `.FMA` 链；`tau = (1-k)*exp(-k) + k**2*yy` 为
    // `.FMA (1-k, exp(-k), yy*(k*k))`
    let y = if k <= 4.0 {
        horner(&SMALL_K_COEFFICIENTS, k.contract(0.5, -1.0)) - k.ln()
    } else {
        (-k).exp() * horner(&LARGE_K_COEFFICIENTS, 14.5 / (k + 3.25) - 1.0) / k
    };
    (1.0 - k).contract((-k).exp(), y * (k * k))
}

fn horner(coefficients: &[f64], x: f64) -> f64 {
    coefficients[1..]
        .iter()
        .copied()
        .fold(coefficients[0], |value, coefficient| {
            value.contract(x, coefficient)
        })
}

/// `MOD_tav_abs::tav_abs`，按其 GIMPLE 逐条对齐。
fn tav(theta_degrees: f64) -> Result<Vec<f64>> {
    // `pi = atan(1.)*4.`、`rd = pi/180.` 在 real(8) 里折成常数 `theta*0.017453292519943295`
    let sin_theta = crate::atmosphere::fortran_sin(theta_degrees * 1.745_329_251_994_329_5e-2);
    let sin_theta_squared = sin_theta * sin_theta;
    let mut values = Vec::with_capacity(PROSPECT_WAVELENGTHS);
    for &refractive in &data::REFRACTIVE_INDEX {
        let n2 = refractive * refractive;
        let np = n2 + 1.0;
        let nm = n2 - 1.0;
        let a = (refractive + 1.0) * (refractive + 1.0) * 0.5;
        let k = -((n2 - 1.0) * (n2 - 1.0) * 0.25);
        // `sa**2 - np/2` 是 `.FMS (sa, sa, np*0.5)`
        let b2 = sin_theta.contract(sin_theta, -(np * 0.5));
        let b1 = if theta_degrees == 90.0 {
            0.0
        } else {
            b2.contract(b2, k).sqrt()
        };
        let b = b1 - b2;
        let b3 = b * (b * b);
        let a3 = a * a * a;
        let k2 = k * k;
        let ts = (-b).contract(0.5, k / b + k2 / (b3 * 6.0))
            - (-a).contract(0.5, k / a + k2 / (a3 * 6.0));
        let nm2 = nm * nm;
        let tp1 = -((b - a) * (n2 * 2.0) / (np * np));
        let tp2 = -(n2 * 2.0 * np * (b / a).ln() / nm2);
        let tp3 = (1.0 / b - 1.0 / a) * n2 * 0.5;
        let n2_sq = n2 * n2;
        let np_2 = np * 2.0;
        let tp4 =
            n2_sq * 16.0 * (n2_sq + 1.0) * (np_2.contract(b, -nm2) / np_2.contract(a, -nm2)).ln()
                / (np.lpow(3.0) * nm2);
        let tp5 = n2.lpow(3.0)
            * 16.0
            * (1.0 / (-nm).contract(nm, np_2 * b) - 1.0 / (-nm).contract(nm, np_2 * a))
            / np.lpow(3.0);
        let tp = tp2 + tp1 + tp3 + tp4 + tp5;
        let value = (tp + ts) / (sin_theta_squared * 2.0);
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
