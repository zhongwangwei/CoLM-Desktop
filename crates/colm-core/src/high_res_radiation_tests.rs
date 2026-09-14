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
fn leafless_high_resolution_cold_start_reduces_ground_absorption_once() {
    let ground = (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|_| [0.2, 0.3])
        .collect::<Vec<_>>();
    let fractions = HighResolutionRadiationFractions {
        direct: vec![1.0; HIGH_RES_WAVELENGTHS],
        diffuse: vec![1.0; HIGH_RES_WAVELENGTHS],
    };

    let state = high_resolution_pft_cold_start_state(None, &ground, &fractions).unwrap();

    for band in 0..2 {
        assert!((state.albedo[band][0] - 0.2).abs() < 1.0e-12);
        assert!((state.albedo[band][1] - 0.3).abs() < 1.0e-12);
    }
    assert_eq!(state.sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.shaded_absorption, [[0.0; 2]; 2]);
    for band in 0..2 {
        assert!((state.soil_absorption[band][0] - 0.8).abs() < 1.0e-12);
        assert!((state.soil_absorption[band][1] - 0.7).abs() < 1.0e-12);
    }
    assert_eq!(state.snow_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.transmission, None);
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

#[test]
fn lct_high_resolution_solver_matches_the_broadband_two_stream_kernel() {
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
    let high_res = lct_high_resolution_radiation(
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
        false,
    )
    .unwrap();
    let broadband = crate::cold_start_broadband_radiation_with_snow(
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
        true,
        false,
        false,
        0.0,
        0.0,
        273.16,
    )
    .unwrap();
    for wavelength in [0, 29, HIGH_RES_WAVELENGTHS - 1] {
        for incidence in 0..2 {
            let index = wavelength * 2 + incidence;
            assert!((high_res.albedo[index] - broadband.albedo[0][incidence]).abs() < 1.0e-12);
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
        assert!(
            (high_res.transmission[wavelength * 3 + 2]
                - (-high_res.direct_extinction * 0.65).exp())
            .abs()
                < 1.0e-12
        );
    }
}

fn nonuniform_fractions() -> HighResolutionRadiationFractions {
    HighResolutionRadiationFractions {
        direct: (0..HIGH_RES_WAVELENGTHS)
            .map(|wavelength| 1.0 + wavelength as f64 * 0.01)
            .collect(),
        diffuse: (0..HIGH_RES_WAVELENGTHS)
            .map(|wavelength| 2.0 + (HIGH_RES_WAVELENGTHS - wavelength) as f64 * 0.005)
            .collect(),
    }
}

fn nonuniform_ground() -> Vec<f64> {
    (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|wavelength| {
            let wave = wavelength as f64;
            [0.11 + wave * 0.0003, 0.17 + wave * 0.0002]
        })
        .collect()
}

fn canopy_optics() -> LeafOptics {
    LeafOptics {
        chil: 0.01,
        reflectance: [[0.10, 0.16], [0.45, 0.39]],
        transmittance: [[0.07, 0.001], [0.25, 0.001]],
    }
}

fn expected_soil_absorption(
    ground: &[f64],
    transmission: &[f64],
    fractions: &HighResolutionRadiationFractions,
) -> [[f64; 2]; 2] {
    let soil_direct = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| {
            transmission[wavelength * 3] * (1.0 - ground[wavelength * 2 + 1])
                + transmission[wavelength * 3 + 2] * (1.0 - ground[wavelength * 2])
        })
        .collect::<Vec<_>>();
    let soil_diffuse = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| transmission[wavelength * 3 + 1] * (1.0 - ground[wavelength * 2 + 1]))
        .collect::<Vec<_>>();
    let direct = weighted_high_resolution_bands(&soil_direct, &fractions.direct).unwrap();
    let diffuse = weighted_high_resolution_bands(&soil_diffuse, &fractions.diffuse).unwrap();
    [[direct[0], diffuse[0]], [direct[1], diffuse[1]]]
}

fn expanded_broadband_transmission(transmission: [[f64; 3]; 2]) -> Vec<f64> {
    (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|wavelength| transmission[usize::from(wavelength >= 29)])
        .collect()
}

#[test]
fn nonnatural_high_resolution_rejects_non_nonnatural_patch_kinds() {
    let ground = nonuniform_ground();
    let fractions = nonuniform_fractions();

    for patch_type in [-1, 0, 5] {
        assert!(high_resolution_nonnatural_cold_start_state(
            patch_type,
            &ground,
            &fractions,
            canopy_optics(),
            0.0,
            0.0,
            0.5,
            false,
            false
        )
        .is_err());
    }
}

#[test]
fn leafless_nonnatural_high_resolution_reduces_ground_and_persists_it() {
    let ground = nonuniform_ground();
    let fractions = nonuniform_fractions();

    let (state, spectral_albedo) = high_resolution_nonnatural_cold_start_state(
        4,
        &ground,
        &fractions,
        canopy_optics(),
        0.0,
        0.0,
        0.5,
        false,
        false,
    )
    .unwrap();
    let expected = high_resolution_pft_cold_start_state(None, &ground, &fractions).unwrap();

    assert_eq!(spectral_albedo, ground);
    assert_eq!(state.albedo, expected.albedo);
    assert_eq!(state.soil_absorption, expected.soil_absorption);
    assert_eq!(state.sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.shaded_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.thermal_gap_fraction, 1.0);
    assert_eq!(state.transmission, None);
}

#[test]
fn positive_lai_urban_high_resolution_uses_broadband_canopy_but_persists_source_ones() {
    let ground = nonuniform_ground();
    let fractions = nonuniform_fractions();
    let optics = canopy_optics();
    let broadband_ground = weighted_two_stream(&ground, &fractions).unwrap();
    let broadband =
        crate::radiation::two_stream(optics, 0.3, 0.2, 0.0, 0.5, broadband_ground, false, false)
            .unwrap();

    let (state, spectral_albedo) = high_resolution_nonnatural_cold_start_state(
        1, &ground, &fractions, optics, 0.3, 0.2, 0.5, false, false,
    )
    .unwrap();
    let expanded_transmission = expanded_broadband_transmission(broadband.transmission);

    assert_eq!(spectral_albedo, vec![1.0; HIGH_RES_WAVELENGTHS * 2]);
    assert_eq!(state.albedo, broadband.albedo);
    assert_eq!(state.sunlit_absorption, broadband.sunlit_absorption);
    assert_eq!(state.shaded_absorption, broadband.shaded_absorption);
    assert_eq!(
        state.soil_absorption,
        expected_soil_absorption(&ground, &expanded_transmission, &fractions)
    );
    assert_eq!(state.transmission, Some(broadband.transmission));
    assert_eq!(state.thermal_gap_fraction, broadband.thermal_gap_fraction);
    assert_eq!(state.direct_extinction, broadband.direct_extinction);
    assert_eq!(state.diffuse_extinction, broadband.diffuse_extinction);
}

#[test]
fn positive_lai_ice_high_resolution_stays_ground_only_with_missing_thermal_gap() {
    let ground = nonuniform_ground();
    let fractions = nonuniform_fractions();

    let (state, spectral_albedo) = high_resolution_nonnatural_cold_start_state(
        3,
        &ground,
        &fractions,
        canopy_optics(),
        0.3,
        0.2,
        0.5,
        false,
        false,
    )
    .unwrap();
    let expected_albedo = weighted_two_stream(&ground, &fractions).unwrap();
    let default_transmission = (0..HIGH_RES_WAVELENGTHS)
        .flat_map(|_| [0.0, 1.0, 1.0])
        .collect::<Vec<_>>();

    assert_eq!(spectral_albedo, ground);
    assert_eq!(state.albedo, expected_albedo);
    assert_eq!(state.sunlit_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.shaded_absorption, [[0.0; 2]; 2]);
    assert_eq!(
        state.soil_absorption,
        expected_soil_absorption(&spectral_albedo, &default_transmission, &fractions)
    );
    assert_eq!(state.thermal_gap_fraction, crate::MISSING);
    assert_eq!(state.transmission, None);
}

#[derive(Clone, Copy)]
struct SourceProbeCase {
    label: &'static str,
    patch_type: i32,
    lai: f64,
    sai: f64,
    ground_temperature_k: f64,
    cosine_zenith: f64,
    albedo: [[f64; 2]; 2],
    spectral_albedo_weighted: [f64; 4],
    sunlit_absorption: [[f64; 2]; 2],
    shaded_absorption: [[f64; 2]; 2],
    thermal_gap_fraction: f64,
    direct_extinction: f64,
    diffuse_extinction: f64,
    soil_absorption: [[f64; 2]; 2],
}

const fn h(bits: u64) -> f64 {
    f64::from_bits(bits)
}

const SOURCE_PROBE_CASES: &[SourceProbeCase] = &[
    SourceProbeCase {
        label: "urban_ground",
        patch_type: 1,
        lai: h(0x0000000000000000),
        sai: h(0x0000000000000000),
        ground_temperature_k: 285.0,
        cosine_zenith: 0.37,
        albedo: [
            [h(0x3FBC45995BC1D27A), h(0x3FBC42DB52648885)],
            [h(0x3FC3AD9C13AF2278), h(0x3FC3A97EB59173E9)],
        ],
        spectral_albedo_weighted: [
            h(0x3FBC45995BC1D27A),
            h(0x3FC3AD9C13AF2278),
            h(0x3FBC42DB52648885),
            h(0x3FC3A97EB59173E9),
        ],
        sunlit_absorption: [[0.0, 0.0], [0.0, 0.0]],
        shaded_absorption: [[0.0, 0.0], [0.0, 0.0]],
        thermal_gap_fraction: h(0x3FF0000000000000),
        direct_extinction: h(0x3FF0000000000000),
        diffuse_extinction: h(0x3FE6F9DB22D0E560),
        soil_absorption: [
            [h(0x3FEC774CD487C5AE), h(0x3FEC77A495B36EF0)],
            [h(0x3FEB1498FB143768), h(0x3FEB15A0529BA30F)],
        ],
    },
    SourceProbeCase {
        label: "urban_canopy_alb_hires_quirk",
        patch_type: 1,
        lai: h(0x3FDAE147AE147AE1),
        sai: h(0x3FBC28F5C28F5C29),
        ground_temperature_k: 285.0,
        cosine_zenith: 0.37,
        albedo: [
            [h(0x3FB1764B4BA98822), h(0x3FB2A00ACDA8AF37)],
            [h(0x3FC25FAE2465ABF3), h(0x3FC2421AA7C749B9)],
        ],
        spectral_albedo_weighted: [
            h(0x3FF0000000000000),
            h(0x3FF0000000000000),
            h(0x3FF0000000000000),
            h(0x3FF0000000000000),
        ],
        sunlit_absorption: [
            [h(0x3FDDC06797465796), h(0x3FD12C982AA3EB86)],
            [h(0x3FD67C6B4A6D9DD0), h(0x3FCAABCD59EE192B)],
        ],
        shaded_absorption: [
            [h(0x3F8769A31B2A0D56), h(0x3FBA72824EFF521C)],
            [h(0x3F9133B6CE7A4722), h(0x3FB4C02A9169B742)],
        ],
        thermal_gap_fraction: h(0x3FE31CCF44FBBEF3),
        direct_extinction: h(0x3FF6B3D7610DB0B3),
        diffuse_extinction: h(0x3FE7020C49BA5E35),
        soil_absorption: [
            [h(0x3FDD26B15BD81DC9), h(0x3FE1C76247190A12)],
            [h(0x3FDF404A995BBAE8), h(0x3FE22C80AD657062)],
        ],
    },
    SourceProbeCase {
        label: "wetland_ground",
        patch_type: 2,
        lai: h(0x0000000000000000),
        sai: h(0x0000000000000000),
        ground_temperature_k: 286.0,
        cosine_zenith: 0.31,
        albedo: [
            [h(0x3FBF3B645A1CAC05), h(0x3FBF3B645A1CAC08)],
            [h(0x3FC9DB22D0E56046), h(0x3FC9DB22D0E5604A)],
        ],
        spectral_albedo_weighted: [
            h(0x3FBF3B645A1CAC05),
            h(0x3FC9DB22D0E56046),
            h(0x3FBF3B645A1CAC08),
            h(0x3FC9DB22D0E5604A),
        ],
        sunlit_absorption: [[0.0, 0.0], [0.0, 0.0]],
        shaded_absorption: [[0.0, 0.0], [0.0, 0.0]],
        thermal_gap_fraction: h(0x3FF0000000000000),
        direct_extinction: h(0x3FF0000000000000),
        diffuse_extinction: h(0x3FE6F9DB22D0E560),
        soil_absorption: [
            [h(0x3FEC189374BC6A78), h(0x3FEC189374BC6A7F)],
            [h(0x3FE989374BC6A7F3), h(0x3FE989374BC6A7F6)],
        ],
    },
    SourceProbeCase {
        label: "wetland_canopy_alb_hires_quirk",
        patch_type: 2,
        lai: h(0x3FD70A3D70A3D70A),
        sai: h(0x3FB47AE147AE147B),
        ground_temperature_k: 286.0,
        cosine_zenith: 0.31,
        albedo: [
            [h(0x3FB373685BEFF811), h(0x3FB4FE82CEDFB07C)],
            [h(0x3FC5B99ECFB670A4), h(0x3FC59954263AFD12)],
        ],
        spectral_albedo_weighted: [
            h(0x3FF0000000000000),
            h(0x3FF0000000000000),
            h(0x3FF0000000000000),
            h(0x3FF0000000000000),
        ],
        sunlit_absorption: [
            [h(0x3FDDB91CE7B38582), h(0x3FCDBC9FAC4778BA)],
            [h(0x3FD67DD7C4C3A6D0), h(0x3FC777424A9F39D1)],
        ],
        shaded_absorption: [
            [h(0x3F851D6A0F0CAFE1), h(0x3FB76FE664EF8DB8)],
            [h(0x3F90A478305FC340), h(0x3FB2B6D69AC1F862)],
        ],
        thermal_gap_fraction: h(0x3FE4DC527E5CB449),
        direct_extinction: h(0x3FFB7BBAFC253D24),
        diffuse_extinction: h(0x3FE7020C49BA5E35),
        soil_absorption: [
            [h(0x3FDCC11DB0D816F9), h(0x3FE3030AEE743A0A)],
            [h(0x3FDD9B11505B24AA), h(0x3FE264FF9071333B)],
        ],
    },
    SourceProbeCase {
        label: "glacier_positive_lai_no_canopy_solver",
        patch_type: 3,
        lai: h(0x3FDC28F5C28F5C29),
        sai: h(0x3FA999999999999A),
        ground_temperature_k: 268.0,
        cosine_zenith: 0.41,
        albedo: [
            [h(0x3FE9999999999998), h(0x3FE9999999999999)],
            [h(0x3FE199999999999E), h(0x3FE199999999999C)],
        ],
        spectral_albedo_weighted: [
            h(0x3FE9999999999998),
            h(0x3FE199999999999E),
            h(0x3FE9999999999999),
            h(0x3FE199999999999C),
        ],
        sunlit_absorption: [[0.0, 0.0], [0.0, 0.0]],
        shaded_absorption: [[0.0, 0.0], [0.0, 0.0]],
        thermal_gap_fraction: h(0xC76812F9CF7920E3),
        direct_extinction: h(0x3FF0000000000000),
        diffuse_extinction: h(0x3FE6F9DB22D0E560),
        soil_absorption: [
            [h(0x3FC9999999999997), h(0x3FC9999999999997)],
            [h(0x3FDCCCCCCCCCCCD1), h(0x3FDCCCCCCCCCCCD3)],
        ],
    },
    SourceProbeCase {
        label: "lake_unfrozen_positive_lai_no_canopy_solver",
        patch_type: 4,
        lai: h(0x3FE0A3D70A3D70A4),
        sai: h(0x3FB1EB851EB851EC),
        ground_temperature_k: 281.0,
        cosine_zenith: 0.23,
        albedo: [
            [h(0x3FC0D79435E50D76), h(0x3FB9999999999999)],
            [h(0x3FC0D79435E50D7C), h(0x3FB999999999999B)],
        ],
        spectral_albedo_weighted: [
            h(0x3FC0D79435E50D76),
            h(0x3FC0D79435E50D7C),
            h(0x3FB9999999999999),
            h(0x3FB999999999999B),
        ],
        sunlit_absorption: [[0.0, 0.0], [0.0, 0.0]],
        shaded_absorption: [[0.0, 0.0], [0.0, 0.0]],
        thermal_gap_fraction: h(0xC76812F9CF7920E3),
        direct_extinction: h(0x3FF0000000000000),
        diffuse_extinction: h(0x3FE6F9DB22D0E560),
        soil_absorption: [
            [h(0x3FEBCA1AF286BC9E), h(0x3FECCCCCCCCCCCCC)],
            [h(0x3FEBCA1AF286BCA4), h(0x3FECCCCCCCCCCCD4)],
        ],
    },
    SourceProbeCase {
        label: "lake_frozen_ground",
        patch_type: 4,
        lai: h(0x0000000000000000),
        sai: h(0x0000000000000000),
        ground_temperature_k: 270.0,
        cosine_zenith: 0.23,
        albedo: [
            [h(0x3FE3333333333330), h(0x3FE3333333333334)],
            [h(0x3FD999999999999A), h(0x3FD999999999999B)],
        ],
        spectral_albedo_weighted: [
            h(0x3FE3333333333330),
            h(0x3FD999999999999A),
            h(0x3FE3333333333334),
            h(0x3FD999999999999B),
        ],
        sunlit_absorption: [[0.0, 0.0], [0.0, 0.0]],
        shaded_absorption: [[0.0, 0.0], [0.0, 0.0]],
        thermal_gap_fraction: h(0x3FF0000000000000),
        direct_extinction: h(0x3FF0000000000000),
        diffuse_extinction: h(0x3FE6F9DB22D0E560),
        soil_absorption: [
            [h(0x3FD9999999999998), h(0x3FD9999999999999)],
            [h(0x3FE3333333333334), h(0x3FE3333333333338)],
        ],
    },
];

fn source_probe_fractions() -> HighResolutionRadiationFractions {
    HighResolutionRadiationFractions {
        direct: (1..=HIGH_RES_WAVELENGTHS)
            .map(|i| 0.37 + 0.0023 * ((i * 17) % 31) as f64 + 0.00001 * i as f64)
            .collect(),
        diffuse: (1..=HIGH_RES_WAVELENGTHS)
            .map(|i| {
                0.22 + 0.0017 * ((i * 11) % 37) as f64
                    + 0.00002 * (HIGH_RES_WAVELENGTHS + 1 - i) as f64
            })
            .collect(),
    }
}

fn source_probe_optics() -> LeafOptics {
    LeafOptics {
        chil: -0.12,
        reflectance: [[0.10, 0.16], [0.23, 0.31]],
        transmittance: [[0.06, 0.04], [0.17, 0.12]],
    }
}

fn source_probe_ground(case: &SourceProbeCase) -> Vec<f64> {
    match case.patch_type {
        1 => (1..=HIGH_RES_WAVELENGTHS)
            .flat_map(|i| {
                let albedo = 0.065 + 0.013 * 3.0 + 0.00041 * i as f64 + 0.00007 * (i % 9) as f64;
                [albedo, albedo]
            })
            .collect(),
        2 => {
            let soil_s_v_alb = 0.08;
            let soil_d_v_alb = 0.19;
            let soil_s_n_alb = 0.16;
            let soil_d_n_alb = 0.27;
            let ssw = 0.17;
            let alb_s_inc = f64::max(0.11 - 0.40 * ssw, 0.0);
            let visible = f64::min(soil_s_v_alb + alb_s_inc, soil_d_v_alb);
            let near_infrared = f64::min(soil_s_n_alb + alb_s_inc, soil_d_n_alb);
            expand_broadband_ground_albedo([[visible, visible], [near_infrared, near_infrared]])
        }
        3 => expand_broadband_ground_albedo([[0.8, 0.8], [0.55, 0.55]]),
        4 => {
            if case.ground_temperature_k < 273.16 {
                expand_broadband_ground_albedo([[0.6, 0.6], [0.4, 0.4]])
            } else {
                let albedo = 0.05 / (case.cosine_zenith.max(0.001) + 0.15);
                (0..HIGH_RES_WAVELENGTHS)
                    .flat_map(|_| [albedo, 0.1])
                    .collect()
            }
        }
        _ => unreachable!(),
    }
}

fn weighted_spectral_albedo(
    spectral_albedo: &[f64],
    fractions: &HighResolutionRadiationFractions,
) -> [f64; 4] {
    let direct = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| spectral_albedo[wavelength * 2])
        .collect::<Vec<_>>();
    let diffuse = (0..HIGH_RES_WAVELENGTHS)
        .map(|wavelength| spectral_albedo[wavelength * 2 + 1])
        .collect::<Vec<_>>();
    let direct = weighted_high_resolution_bands(&direct, &fractions.direct).unwrap();
    let diffuse = weighted_high_resolution_bands(&diffuse, &fractions.diffuse).unwrap();
    [direct[0], direct[1], diffuse[0], diffuse[1]]
}

fn assert_close_scalar(label: &str, actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-12,
        "{label}: actual={actual:?} expected={expected:?} delta={:?}",
        (actual - expected).abs()
    );
}

fn assert_close_matrix(label: &str, actual: [[f64; 2]; 2], expected: [[f64; 2]; 2]) {
    for band in 0..2 {
        for radiation_type in 0..2 {
            assert_close_scalar(
                &format!("{label}[{band}][{radiation_type}]"),
                actual[band][radiation_type],
                expected[band][radiation_type],
            );
        }
    }
}

#[test]
fn nonnatural_high_resolution_matches_source_probe_defined_fields() {
    // Expected values are from `/tmp/colm-nonnatural-highres-fix/source_probe.log`,
    // generated with `/opt/homebrew/bin/gfortran` from a synthetic no-snow driver
    // that embeds source-exact `twostream` and `calculate_wgt_variable` from
    // `/Users/zhongwangwei/Desktop/Github/CoLM202X/main/MOD_Albedo_HiRes.F90`.
    let fractions = source_probe_fractions();
    for case in SOURCE_PROBE_CASES {
        let ground = source_probe_ground(case);
        let (state, spectral_albedo) = high_resolution_nonnatural_cold_start_state(
            case.patch_type,
            &ground,
            &fractions,
            source_probe_optics(),
            case.lai,
            case.sai,
            case.cosine_zenith,
            false,
            false,
        )
        .unwrap();
        let spectral_weighted = weighted_spectral_albedo(&spectral_albedo, &fractions);

        assert_close_matrix(&format!("{} alb", case.label), state.albedo, case.albedo);
        assert_close_matrix(
            &format!("{} ssun", case.label),
            state.sunlit_absorption,
            case.sunlit_absorption,
        );
        assert_close_matrix(
            &format!("{} ssha", case.label),
            state.shaded_absorption,
            case.shaded_absorption,
        );
        assert_close_matrix(
            &format!("{} ssoi", case.label),
            state.soil_absorption,
            case.soil_absorption,
        );
        for (index, (&actual, &expected)) in spectral_weighted
            .iter()
            .zip(&case.spectral_albedo_weighted)
            .enumerate()
        {
            assert_close_scalar(
                &format!("{} alb_hires_weighted[{index}]", case.label),
                actual,
                expected,
            );
        }
        assert_close_scalar(
            &format!("{} thermk", case.label),
            state.thermal_gap_fraction,
            case.thermal_gap_fraction,
        );
        assert_close_scalar(
            &format!("{} extkb", case.label),
            state.direct_extinction,
            case.direct_extinction,
        );
        assert_close_scalar(
            &format!("{} extkd", case.label),
            state.diffuse_extinction,
            case.diffuse_extinction,
        );
    }
}

#[test]
fn nonnatural_high_resolution_usgs_suppresses_stem_optics() {
    let ground = nonuniform_ground();
    let fractions = nonuniform_fractions();
    let optics = canopy_optics();
    let expected = crate::radiation::two_stream(
        optics,
        0.3,
        0.2,
        0.0,
        0.5,
        weighted_two_stream(&ground, &fractions).unwrap(),
        true,
        false,
    )
    .unwrap();
    let (state, _) = high_resolution_nonnatural_cold_start_state(
        2, &ground, &fractions, optics, 0.3, 0.2, 0.5, true, false,
    )
    .unwrap();
    assert_eq!(state.albedo, expected.albedo);
    assert_eq!(state.sunlit_absorption, expected.sunlit_absorption);
    assert_eq!(state.transmission, Some(expected.transmission));
}
