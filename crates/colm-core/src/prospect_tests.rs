use super::*;

#[test]
#[allow(clippy::excessive_precision)]
fn spectrum_matches_the_vendored_fortran_prospect_oracle() {
    // Values emitted by a local gfortran build of the vendored
    // MOD_prospect_DB.F90 with these exact inputs.
    let (reflectance, transmittance) =
        prospect_spectrum(1.8, 35.0, 8.0, 0.0, 0.01, 0.007, 0.006).unwrap();
    for (wavelength, expected_reflectance, expected_transmittance) in [
        (0, 4.342_781_225_769_463_3e-2, 4.691_942_258_038_408_3e-4),
        (29, 6.737_247_764_146_725_4e-2, 3.348_260_396_970_516_3e-2),
        (105, 2.465_030_032_459_742_4e-1, 2.301_833_642_061_820_7e-1),
        (210, 7.076_390_892_989_949_1e-2, 8.800_230_853_238_931_4e-2),
    ] {
        assert!(
            (reflectance[wavelength] - expected_reflectance).abs() < 2.0e-13,
            "{wavelength}: reflectance got {:.17e}, expected {expected_reflectance:.17e}",
            reflectance[wavelength]
        );
        assert!(
            (transmittance[wavelength] - expected_transmittance).abs() < 2.0e-13,
            "{wavelength}: transmittance got {:.17e}, expected {expected_transmittance:.17e}",
            transmittance[wavelength]
        );
    }
}

#[test]
#[allow(clippy::excessive_precision)]
fn pft_parameterization_matches_the_vendored_fortran_oracle() {
    let reflectance = vec![0.1; HIGH_RES_WAVELENGTHS * 2];
    let transmittance = vec![0.05; HIGH_RES_WAVELENGTHS * 2];
    let optics = prospect_leaf_optics(
        7,
        0.3,
        HighResolutionLeafOptics {
            reflectance: &reflectance,
            transmittance: &transmittance,
        },
    )
    .unwrap();
    for (wavelength, expected_reflectance, expected_transmittance) in [
        (0, 4.315_729_254_294_031_8e-2, 6.552_043_576_987_233_6e-3),
        (29, 6.353_325_053_949_612_8e-2, 1.462_306_044_938_804_8e-1),
        (105, 1.383_286_192_843_577_2e-1, 3.329_044_422_150_699_3e-1),
        (210, 3.151_063_511_307_480_2e-2, 1.449_978_446_661_720_8e-1),
    ] {
        assert!(
            (optics.reflectance[wavelength * 2] - expected_reflectance).abs() < 2.0e-13,
            "{wavelength}: reflectance got {:.17e}, expected {expected_reflectance:.17e}",
            optics.reflectance[wavelength * 2]
        );
        assert!(
            (optics.transmittance[wavelength * 2] - expected_transmittance).abs() < 2.0e-13,
            "{wavelength}: transmittance got {:.17e}, expected {expected_transmittance:.17e}",
            optics.transmittance[wavelength * 2]
        );
    }
}

#[test]
fn pft_update_replaces_only_green_leaf_optics() {
    let input = HighResolutionLeafOptics {
        reflectance: &(0..HIGH_RES_WAVELENGTHS)
            .flat_map(|wavelength| [0.1 + wavelength as f64 * 1.0e-4, 0.7])
            .collect::<Vec<_>>(),
        transmittance: &(0..HIGH_RES_WAVELENGTHS)
            .flat_map(|wavelength| [0.2 + wavelength as f64 * 1.0e-4, 0.8])
            .collect::<Vec<_>>(),
    };
    let low_moisture = prospect_leaf_optics(7, 0.1, input).unwrap();
    let high_moisture = prospect_leaf_optics(7, 0.7, input).unwrap();

    assert_eq!(low_moisture.reflectance.len(), HIGH_RES_WAVELENGTHS * 2);
    assert_eq!(low_moisture.transmittance.len(), HIGH_RES_WAVELENGTHS * 2);
    for wavelength in [0, 29, HIGH_RES_WAVELENGTHS - 1] {
        assert_eq!(low_moisture.reflectance[wavelength * 2 + 1], 0.7);
        assert_eq!(low_moisture.transmittance[wavelength * 2 + 1], 0.8);
    }
    assert_ne!(
        low_moisture.reflectance[105 * 2],
        high_moisture.reflectance[105 * 2]
    );
    assert!(low_moisture
        .reflectance
        .iter()
        .chain(&low_moisture.transmittance)
        .all(|value| value.is_finite()));
}
