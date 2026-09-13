use super::*;
use crate::{cold_start_pft_broadband_radiation_with_snow, LeafOptics, SoilReflectance};

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
