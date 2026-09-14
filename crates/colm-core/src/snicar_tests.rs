use super::*;

fn table(ssa: f64, g: f64, ext: f64) -> SnicarSpectralTable {
    let mut single = Vec::with_capacity(SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN);
    let mut asym = Vec::with_capacity(SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN);
    let mut mass = Vec::with_capacity(SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN);
    for band in 0..SNICAR_BANDS {
        for radius in 0..SNICAR_RADIUS_TABLE_LEN {
            single.push((ssa - 0.003 * band as f64 - 0.000_001 * radius as f64).clamp(0.8, 0.999));
            asym.push((g - 0.01 * band as f64).clamp(0.1, 0.98));
            mass.push(ext + band as f64 * 0.5 + radius as f64 * 0.0001);
        }
    }
    SnicarSpectralTable::new(single, asym, mass).unwrap()
}

fn optics() -> SnicarOptics {
    let mut aer_ssa = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    let mut aer_g = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    let mut aer_ext = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    for species in 0..SNICAR_AEROSOLS {
        for band in 0..SNICAR_BANDS {
            aer_ssa[species][band] = 0.35 + 0.03 * species as f64 + 0.01 * band as f64;
            aer_g[species][band] = 0.45 + 0.02 * species as f64;
            aer_ext[species][band] = 0.8 + 0.1 * species as f64 + 0.05 * band as f64;
        }
    }
    SnicarOptics::new(
        table(0.995, 0.82, 12.0),
        table(0.985, 0.75, 10.0),
        aer_ssa,
        aer_g,
        aer_ext,
    )
    .unwrap()
}

fn optics_with_ice_asymmetry(value: f64) -> SnicarOptics {
    let mut direct = table(0.995, 0.82, 12.0);
    let mut diffuse = table(0.985, 0.75, 10.0);
    direct.asymmetry_parameter.fill(value);
    diffuse.asymmetry_parameter.fill(value);
    let mut aer_ssa = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    let mut aer_g = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    let mut aer_ext = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    for species in 0..SNICAR_AEROSOLS {
        for band in 0..SNICAR_BANDS {
            aer_ssa[species][band] = 0.35 + 0.03 * species as f64 + 0.01 * band as f64;
            aer_g[species][band] = 0.45 + 0.02 * species as f64;
            aer_ext[species][band] = 0.8 + 0.1 * species as f64 + 0.05 * band as f64;
        }
    }
    SnicarOptics::new(direct, diffuse, aer_ssa, aer_g, aer_ext).unwrap()
}

fn input(incident: SnicarIncident) -> SnicarInput {
    let mut aerosol = [[0.0; SNICAR_AEROSOLS]; SNICAR_MAX_LAYERS];
    aerosol[0][0] = 1.0e-8;
    aerosol[1][4] = 2.0e-8;
    SnicarInput {
        incident,
        cosine_zenith: 0.55,
        snow_water_equivalent_kg_m2: 16.0,
        active_layers: 2,
        liquid_water_kg_m2: [0.5, 0.2, 0.0, 0.0, 0.0],
        ice_water_kg_m2: [8.0, 7.3, 0.0, 0.0, 0.0],
        snow_radius_microns: [120, 300, 0, 0, 0],
        aerosol_mass_concentration: aerosol,
        underlying_albedo_5band: [0.2, 0.25, 0.3, 0.35, 0.4],
    }
}

#[test]
fn constructor_validates_table_shapes_once() {
    assert!(SnicarSpectralTable::new(vec![0.9; 4], vec![0.8; 4], vec![1.0; 4]).is_err());
    assert!(SnicarSpectralTable::new(
        vec![0.9; SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN],
        vec![1.0; SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN],
        vec![1.0; SNICAR_BANDS * SNICAR_RADIUS_TABLE_LEN],
    )
    .is_ok());
    let mut bad = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    bad[0][0] = f64::NAN;
    assert!(
        SnicarOptics::new(table(0.99, 0.8, 1.0), table(0.99, 0.8, 1.0), bad, bad, bad).is_err()
    );
}

#[test]
fn direct_and_diffuse_return_five_band_and_broadband_absorption() {
    let optics = optics();
    let direct = snicar_ad_rt(&optics, &input(SnicarIncident::Direct)).unwrap();
    let diffuse = snicar_ad_rt(&optics, &input(SnicarIncident::Diffuse)).unwrap();

    assert_eq!(direct.absorption_rows, 3);
    assert_eq!(diffuse.absorption_rows, 3);
    assert!(direct
        .albedo_5band
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0 && *value <= 1.0));
    assert_ne!(direct.albedo_5band, diffuse.albedo_5band);
    for band in 0..SNICAR_BANDS {
        let absorbed: f64 = direct.absorbed_5band[..direct.absorption_rows]
            .iter()
            .map(|row| row[band])
            .sum();
        assert!((absorbed + direct.albedo_5band[band] - 1.0).abs() < 1.0e-8);
    }
}

#[test]
fn zero_layer_snow_uses_temporary_minimum_radius_layer() {
    let optics = optics();
    let mut case = input(SnicarIncident::Direct);
    case.active_layers = 0;
    case.snow_water_equivalent_kg_m2 = 3.0;
    let result = snicar_ad_rt(&optics, &case).unwrap();

    assert!(result.temporary_snow_layer);
    assert_eq!(result.absorption_rows, 2);
    assert!(result.albedo_broadband[0] > 0.0);
}

#[test]
fn no_sun_or_tiny_snow_follow_fortran_terminal_branches() {
    let optics = optics();
    let mut case = input(SnicarIncident::Direct);
    case.cosine_zenith = 0.0;
    let dark = snicar_ad_rt(&optics, &case).unwrap();
    assert_eq!(dark.albedo_broadband, [0.0, 0.0]);

    case.cosine_zenith = 0.5;
    case.snow_water_equivalent_kg_m2 = 1.0e-40;
    let tiny = snicar_ad_rt(&optics, &case).unwrap();
    assert_eq!(tiny.albedo_5band, case.underlying_albedo_5band);
}

#[test]
fn threshold_snow_mass_matches_fortran_strict_branch() {
    let optics = optics();
    let mut case = input(SnicarIncident::Direct);
    case.snow_water_equivalent_kg_m2 = 1.0e-30;
    let at_threshold = snicar_ad_rt(&optics, &case).unwrap();
    assert_eq!(at_threshold.albedo_broadband, [0.0, 0.0]);
    assert!(at_threshold.absorbed_5band[..at_threshold.absorption_rows]
        .iter()
        .all(|row| row.iter().all(|value| *value == 0.0)));

    case.snow_water_equivalent_kg_m2 = 0.5e-30;
    let below_threshold = snicar_ad_rt(&optics, &case).unwrap();
    assert_eq!(below_threshold.albedo_5band, case.underlying_albedo_5band);
}

#[test]
fn zero_layer_temporary_snow_uses_source_minimum_radius() {
    let optics = optics();
    let mut thin = input(SnicarIncident::Direct);
    thin.active_layers = 0;
    thin.snow_water_equivalent_kg_m2 = 3.0;

    let mut explicit = thin.clone();
    explicit.active_layers = 1;
    explicit.ice_water_kg_m2[0] = thin.snow_water_equivalent_kg_m2;
    explicit.liquid_water_kg_m2[0] = 0.0;
    explicit.snow_radius_microns[0] = SNICAR_TEMPORARY_SNOW_RADIUS_MICRONS;

    let thin_result = snicar_ad_rt(&optics, &thin).unwrap();
    let explicit_result = snicar_ad_rt(&optics, &explicit).unwrap();
    assert!(thin_result.temporary_snow_layer);
    assert_eq!(thin_result.albedo_5band, explicit_result.albedo_5band);
    assert_eq!(SNICAR_TEMPORARY_SNOW_RADIUS_MICRONS, 55);

    thin.aerosol_mass_concentration[0][0] = f64::NAN;
    assert!(snicar_ad_rt(&optics, &thin).is_err());
}

#[test]
fn low_zenith_correction_uses_clamped_mu_not() {
    let optics = optics();
    let mut almost_horizon = input(SnicarIncident::Direct);
    almost_horizon.cosine_zenith = 0.001;
    let mut clamped = almost_horizon.clone();
    clamped.cosine_zenith = 0.01;

    let almost = snicar_ad_rt(&optics, &almost_horizon).unwrap();
    let exactly = snicar_ad_rt(&optics, &clamped).unwrap();
    assert_eq!(almost.albedo_broadband, exactly.albedo_broadband);
    assert_eq!(almost.absorbed_broadband, exactly.absorbed_broadband);
}

#[test]
fn ice_asymmetry_one_is_accepted_and_runtime_clamped() {
    let mut at_one = table(0.995, 1.0, 12.0);
    let mut at_clamp = table(0.995, 0.99, 12.0);
    // Make every radius/band exactly exercise the source clamp boundary.
    at_one.asymmetry_parameter.fill(1.0);
    at_clamp.asymmetry_parameter.fill(0.99);
    let mut aer_ssa = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    let mut aer_g = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    let mut aer_ext = [[0.0; SNICAR_BANDS]; SNICAR_AEROSOLS];
    for species in 0..SNICAR_AEROSOLS {
        for band in 0..SNICAR_BANDS {
            aer_ssa[species][band] = 0.4;
            aer_g[species][band] = 1.0;
            aer_ext[species][band] = 1.0;
        }
    }
    let optics_one = SnicarOptics::new(at_one.clone(), at_one, aer_ssa, aer_g, aer_ext).unwrap();
    let optics_clamped =
        SnicarOptics::new(at_clamp.clone(), at_clamp, aer_ssa, aer_g, aer_ext).unwrap();

    let one = snicar_ad_rt(&optics_one, &input(SnicarIncident::Direct)).unwrap();
    let clamped = snicar_ad_rt(&optics_clamped, &input(SnicarIncident::Direct)).unwrap();
    assert_eq!(one.albedo_5band, clamped.albedo_5band);
    assert_eq!(one.absorbed_5band, clamped.absorbed_5band);
}

#[test]
#[allow(clippy::excessive_precision)] // Preserve the original Fortran probe output.
fn source_oracle_golden_cases_match_synthetic_unit_probe() {
    // Source: /tmp/colm-snicar-port/unit_reference.txt generated from
    // /tmp/colm-snicar-port/unit_reference_driver.F90 against generated original
    // MOD_SnowSnicar.F90 SHA256 0bc9989fc0a6e2674da6c7b0bfaa9e81f05dea500ed9a12b3aa6df80a159b035,
    // compiled with -O2 -fdefault-real-8. This is a compact source
    // regression for the synthetic unit optics, not a full executable proof.
    struct Case {
        name: &'static str,
        optics: SnicarOptics,
        input: SnicarInput,
        albedo: [f64; 2],
        absorption: &'static [[f64; 2]],
    }

    let mut direct = input(SnicarIncident::Direct);
    direct.underlying_albedo_5band = [0.2, 0.25, 0.25, 0.25, 0.25];
    let mut diffuse = input(SnicarIncident::Diffuse);
    diffuse.underlying_albedo_5band = [0.2, 0.25, 0.25, 0.25, 0.25];
    let mut horizon = direct.clone();
    horizon.cosine_zenith = 0.001;
    let mut thin = direct.clone();
    thin.active_layers = 0;
    thin.snow_water_equivalent_kg_m2 = 3.0;
    let iceg1 = direct.clone();

    let cases = [
        Case {
            name: "two_layer_direct",
            optics: optics(),
            input: direct,
            albedo: [7.0499306640504333E-01, 6.1900152456467727E-01],
            absorption: &[
                [2.9369195772596007E-01, 3.8079742049841137E-01],
                [1.2989463489255565E-03, 2.0105493691138812E-04],
                [1.6029520071021057E-05, 0.0000000000000000E+00],
            ],
        },
        Case {
            name: "two_layer_diffuse",
            optics: optics(),
            input: diffuse,
            albedo: [5.7656892195131149E-01, 5.4170240143646653E-01],
            absorption: &[
                [4.2336567292028626E-01, 4.5826521406614962E-01],
                [6.5405128402275128E-05, 3.2384497383915135E-05],
                [0.0000000000000000E+00, 0.0000000000000000E+00],
            ],
        },
        Case {
            name: "horizon_direct",
            optics: optics(),
            input: horizon,
            albedo: [8.2366181572782760E-01, 8.5953327448863837E-01],
            absorption: &[
                [1.7561643971264598E-01, 1.4034800893758820E-01],
                [7.1295456508231668E-04, 1.1871657377342270E-04],
                [8.7899944440964901E-06, 0.0000000000000000E+00],
            ],
        },
        Case {
            name: "thin_direct",
            optics: optics(),
            input: thin,
            albedo: [7.0038121658531771E-01, 6.1913719069763784E-01],
            absorption: &[
                [2.3081653189487600E-01, 3.5592300346850053E-01],
                [6.8802251519806290E-02, 2.4939805833861559E-02],
            ],
        },
        Case {
            name: "iceg1_direct",
            optics: optics_with_ice_asymmetry(1.0),
            input: iceg1,
            albedo: [2.6032432847735454E-01, 1.6207421781439724E-01],
            absorption: &[
                [5.8990391784438745E-01, 7.8607927535594391E-01],
                [1.0835816241342251E-01, 4.5374845101881257E-02],
                [4.1413591264835473E-02, 6.4716617277776195E-03],
            ],
        },
    ];

    for case in cases {
        let result = snicar_ad_rt(&case.optics, &case.input).unwrap();
        assert_eq!(
            result.absorption_rows,
            case.absorption.len(),
            "{} rows",
            case.name
        );
        assert_close(result.albedo_broadband[0], case.albedo[0], case.name);
        assert_close(result.albedo_broadband[1], case.albedo[1], case.name);
        for (actual, expected) in result.absorbed_broadband[..result.absorption_rows]
            .iter()
            .zip(case.absorption)
        {
            assert_close(actual[0], expected[0], case.name);
            assert_close(actual[1], expected[1], case.name);
        }
        for trailing in &result.absorbed_broadband[result.absorption_rows..] {
            assert_eq!(*trailing, [0.0, 0.0], "{} trailing rows", case.name);
        }
        for trailing in &result.absorbed_5band[result.absorption_rows..] {
            assert_eq!(
                *trailing, [0.0; SNICAR_BANDS],
                "{} trailing 5band rows",
                case.name
            );
        }
    }
}

fn assert_close(actual: f64, expected: f64, context: &str) {
    let tolerance = 1.0e-12 + 1.0e-12 * expected.abs();
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: actual {actual:.17e} expected {expected:.17e} tolerance {tolerance:.3e}"
    );
}

#[test]
fn invalid_active_layer_topology_is_rejected() {
    let optics = optics();
    let mut case = input(SnicarIncident::Direct);
    case.active_layers = 6;
    assert!(snicar_ad_rt(&optics, &case).is_err());
    case.active_layers = 1;
    case.snow_radius_microns[0] = 29;
    assert!(snicar_ad_rt(&optics, &case).is_err());
}
