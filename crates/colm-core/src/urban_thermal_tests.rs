use super::*;

#[test]
fn component_diagnostics_match_fortran_patch_area_without_reweighting_totals() {
    let fixture = include_str!("../../../oracle/fixtures/urban_thermal_components_reference.csv");
    for (case, line) in fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .skip(1)
        .enumerate()
    {
        let expected: Vec<f64> = line
            .split(',')
            .map(|value| value.parse().unwrap())
            .collect();
        let mut out = UrbanThermalOutput {
            fsen_roof: 100.0 * 0.2,
            fsen_wsun: 80.0 * 0.12,
            fsen_wsha: 60.0 * 0.08,
            fsen_gimp: 40.0 * 0.42,
            fsen_gper: 20.0 * 0.38,
            lfevp_roof: (2_500_000.0 * 0.0002) * 0.2,
            lfevp_gimp: (2_500_000.0 * 0.0001) * 0.42,
            lfevp_gper: (2_500_000.0 * -0.00003) * 0.38,
            fsen_urbl: Some(10.0),
            lfevp_urbl: Some(HVAP * 0.000004),
            fseng: 71.0,
            fsena: 82.0,
            lfevpa: 93.0,
            fgrnd: 104.0,
            ..Default::default()
        };
        out.scale_component_diagnostics(1.0 - [0.0, 0.3, 1.0][case]);
        let actual = [
            out.fsen_roof,
            out.fsen_wsun,
            out.fsen_wsha,
            out.fsen_gimp,
            out.fsen_gper,
            out.lfevp_roof,
            out.lfevp_gimp,
            out.lfevp_gper,
            out.fsen_urbl.unwrap(),
            out.lfevp_urbl.unwrap(),
        ];
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 1.0e-12,
                "{actual} != {expected}"
            );
        }
        assert_eq!(
            (out.fseng, out.fsena, out.lfevpa, out.fgrnd),
            (71.0, 82.0, 93.0, 104.0)
        );
    }
    let mut bare = UrbanThermalOutput::default();
    bare.scale_component_diagnostics(0.5);
    assert_eq!(bare.fsen_urbl, None);
    assert_eq!(bare.lfevp_urbl, None);
}
