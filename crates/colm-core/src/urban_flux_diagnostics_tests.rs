use super::*;

#[test]
fn urban_flux_diagnostics_match_the_fortran_allocate_and_set_contract() {
    let mut diagnostics = UrbanFluxDiagnostics::new(3, -1.0e36);
    assert_eq!(diagnostics.len(), 3);
    assert!(diagnostics
        .sensible_roof_w_m2
        .iter()
        .all(|&value| value == -1.0e36));
    assert!(diagnostics
        .latent_vegetation_w_m2
        .iter()
        .all(|&value| value == -1.0e36));
    diagnostics.fill(2.5);
    assert!(diagnostics
        .sensible_impervious_w_m2
        .iter()
        .all(|&value| value == 2.5));
    assert!(diagnostics
        .latent_roof_w_m2
        .iter()
        .all(|&value| value == 2.5));
}

#[test]
fn urban_flux_diagnostics_support_zero_urban_patches() {
    assert!(UrbanFluxDiagnostics::new(0, -1.0e36).is_empty());
}
