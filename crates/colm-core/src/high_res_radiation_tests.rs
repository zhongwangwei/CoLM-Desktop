use super::*;
use crate::{cold_start_pft_broadband_radiation_with_snow, LeafOptics, SoilReflectance};

#[test]
fn broadband_spectra_preserve_the_visible_near_infrared_boundary() {
    let optics = LeafOptics {
        chil: 0.01,
        reflectance: [[0.1, 0.2], [0.3, 0.4]],
        transmittance: [[0.5, 0.6], [0.7, 0.8]],
    };
    let (reflectance, transmittance) = expand_broadband_leaf_optics(optics);
    let ground = expand_broadband_ground_albedo([[0.11, 0.12], [0.21, 0.22]]);

    assert_eq!(&reflectance[..2], &[0.1, 0.2]);
    assert_eq!(&transmittance[..2], &[0.5, 0.6]);
    assert_eq!(&reflectance[28 * 2..30 * 2], &[0.1, 0.2, 0.3, 0.4]);
    assert_eq!(&transmittance[28 * 2..30 * 2], &[0.5, 0.6, 0.7, 0.8]);
    assert_eq!(&ground[28 * 2..30 * 2], &[0.11, 0.12, 0.21, 0.22]);
}

#[test]
fn dry_bsm_soil_preserves_the_static_spectrum_for_both_radiation_types() {
    let dry: Vec<f64> = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| wavelength as f64 / 1_000.0)
        .collect();
    let wet = bsm_soil_moisture(
        5.0,
        40.0,
        &dry,
        &vec![0.2; HIGH_RES_WAVELENGTHS],
        &vec![1.34; HIGH_RES_WAVELENGTHS],
    )
    .unwrap();

    assert_eq!(wet.len(), HIGH_RES_WAVELENGTHS * 2);
    for wavelength in [0, 29, HIGH_RES_WAVELENGTHS - 1] {
        assert_eq!(
            &wet[wavelength * 2..wavelength * 2 + 2],
            &[dry[wavelength]; 2]
        );
    }
}

#[test]
fn wet_bsm_soil_produces_a_finite_spectrum_for_both_radiation_types() {
    let dry = vec![0.2; HIGH_RES_WAVELENGTHS];
    let wet = bsm_soil_moisture(
        20.0,
        40.0,
        &dry,
        &vec![0.3; HIGH_RES_WAVELENGTHS],
        &vec![1.34; HIGH_RES_WAVELENGTHS],
    )
    .unwrap();

    assert!(wet.iter().all(|value| value.is_finite()));
    assert_ne!(wet[0], dry[0]);
    for wavelength in [0, 29, HIGH_RES_WAVELENGTHS - 1] {
        assert_eq!(wet[wavelength * 2], wet[wavelength * 2 + 1]);
    }
}

#[test]
fn spectral_band_reduction_normalizes_visible_and_near_infrared_independently() {
    let mut values = vec![2.0; HIGH_RES_WAVELENGTHS];
    values[0] = 8.0;
    values[29] = 20.0;
    let mut weights = vec![1.0; HIGH_RES_WAVELENGTHS];
    weights[0] = 3.0;
    weights[29] = 4.0;

    let bands = weighted_high_resolution_bands(&values, &weights).unwrap();

    assert_eq!(bands[0], (8.0 * 3.0 + 2.0 * 28.0) / 31.0);
    assert_eq!(bands[1], (20.0 * 4.0 + 2.0 * 181.0) / 185.0);
}

#[test]
fn uniform_spectrum_reduces_to_the_shared_pft_two_stream_solution() {
    let optics = LeafOptics {
        chil: 0.01,
        reflectance: [[0.10, 0.16], [0.45, 0.39]],
        transmittance: [[0.07, 0.001], [0.25, 0.001]],
    };
    let mut reflectance = Vec::with_capacity(HIGH_RES_WAVELENGTHS * 2);
    let mut transmittance = Vec::with_capacity(HIGH_RES_WAVELENGTHS * 2);
    let mut ground = Vec::with_capacity(HIGH_RES_WAVELENGTHS * 2);
    for _ in 0..HIGH_RES_WAVELENGTHS {
        reflectance.extend([optics.reflectance[0][0], optics.reflectance[0][1]]);
        transmittance.extend([optics.transmittance[0][0], optics.transmittance[0][1]]);
        ground.extend([0.2, 0.2]);
    }
    let high_res = pft_high_resolution_radiation(
        optics.chil,
        HighResolutionLeafOptics {
            reflectance: &reflectance,
            transmittance: &transmittance,
        },
        0.2,
        0.45,
        0.0,
        0.5,
        &ground,
        false,
    )
    .unwrap();
    let broadband = cold_start_pft_broadband_radiation_with_snow(
        0,
        SoilReflectance {
            saturated_visible: 0.2,
            dry_visible: 0.2,
            saturated_near_infrared: 0.2,
            dry_near_infrared: 0.2,
        },
        0.0,
        0.1,
        optics,
        0.2,
        0.45,
        0.0,
        0.5,
        false,
        0.0,
        0.0,
        273.16,
    )
    .unwrap();
    for wavelength in [0, 29, HIGH_RES_WAVELENGTHS - 1] {
        for incidence in 0..2 {
            let index = wavelength * 2 + incidence;
            assert!(
                (high_res.sunlit_absorption[index] - broadband.sunlit_absorption[0][incidence])
                    .abs()
                    < 1.0e-12
            );
            assert!(
                (high_res.shaded_absorption[index] - broadband.shaded_absorption[0][incidence])
                    .abs()
                    < 1.0e-12
            );
        }
    }
    assert!((high_res.thermal_gap_fraction - broadband.thermal_gap_fraction).abs() < 1.0e-12);
    assert!((high_res.direct_extinction - broadband.direct_extinction).abs() < 1.0e-12);
}
