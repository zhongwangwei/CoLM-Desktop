use super::*;

/// `update_params_PROSPECT` 与 gfortran 逐位比对：PFT 1..15 × 土壤湿度 {0.1, 0.7}，211 个波长的
/// 绿叶反射率与透射率。
#[test]
fn pft_parameterization_matches_gfortran_bitwise() {
    colm_numeric::skip_unless_fused!();
    let fixture = include_str!("../tests/data/prospect_pft_gfortran.txt");
    let hex = |s: &str| f64::from_bits(u64::from_str_radix(s, 16).unwrap());
    let reflectance = vec![0.1; HIGH_RES_WAVELENGTHS * 2];
    let transmittance = vec![0.05; HIGH_RES_WAVELENGTHS * 2];
    let input = HighResolutionLeafOptics {
        reflectance: &reflectance,
        transmittance: &transmittance,
    };
    let (mut total, mut bad) = (0, Vec::new());
    for line in fixture.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let pft: usize = fields[1].parse().unwrap();
        let moisture = hex(fields[2]);
        let optics = prospect_leaf_optics(pft, moisture, input).unwrap();
        let got = if fields[0] == "R" {
            &optics.reflectance
        } else {
            &optics.transmittance
        };
        for (band, want) in fields[3..].iter().enumerate() {
            total += 1;
            if !crate::reference_bits_match(got[band * 2], hex(want), 0.0) {
                bad.push(format!(
                    "{} pft {pft} ssw {moisture} band {band}",
                    fields[0]
                ));
            }
        }
    }
    eprintln!(
        "update_params_PROSPECT：{total} 组，按位不一致 {} 组",
        bad.len()
    );
    assert_eq!(total, 15 * 2 * 2 * HIGH_RES_WAVELENGTHS);
    assert!(bad.is_empty(), "{:#?}", &bad[..bad.len().min(8)]);
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

/// `prospect_DB` 与 gfortran（CoLM 构建选项 + `-fdefault-double-8`）逐位比对：30 组参数 × 211 个
/// 取样波长的反射率与透射率。
#[test]
fn spectrum_matches_gfortran_bitwise() {
    colm_numeric::skip_unless_fused!();
    let fixture = include_str!("../tests/data/prospect_gfortran.txt");
    let hex = |s: &str| f64::from_bits(u64::from_str_radix(s, 16).unwrap());
    let lines: Vec<Vec<&str>> = fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.split_whitespace().collect())
        .collect();
    let (mut total, mut bad) = (0, Vec::new());
    for group in lines.chunks(3) {
        let p: Vec<f64> = group[0][1..].iter().map(|v| hex(v)).collect();
        let (reflectance, transmittance) =
            prospect_spectrum(p[0], p[1], p[2], p[3], p[4], p[5], p[6]).unwrap();
        for (row, got) in [(&group[1], &reflectance), (&group[2], &transmittance)] {
            for (band, (want, got)) in row[1..].iter().zip(got.iter()).enumerate() {
                total += 1;
                if !crate::reference_bits_match(*got, hex(want), 0.0) {
                    bad.push(format!(
                        "{} {p:?} band {band}: {got:e} vs {:e}",
                        row[0],
                        hex(want)
                    ));
                }
            }
        }
    }
    eprintln!("PROSPECT：{total} 组，按位不一致 {} 组", bad.len());
    assert_eq!(total, 30 * 211 * 2);
    assert!(bad.is_empty(), "{:#?}", &bad[..bad.len().min(8)]);
}
