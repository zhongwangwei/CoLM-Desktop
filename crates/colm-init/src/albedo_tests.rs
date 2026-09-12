use super::*;

#[test]
fn land_cover_reflectance_matches_fortran_colour_tables() {
    assert_eq!(
        land_cover_soil_reflectance(LandCoverScheme::Igbp, 10).unwrap(),
        SoilReflectance {
            saturated_visible: 0.24,
            dry_visible: 0.35,
            saturated_near_infrared: 0.48,
            dry_near_infrared: 0.59,
        }
    );
    assert_eq!(
        land_cover_soil_reflectance(LandCoverScheme::Usgs, 11)
            .unwrap()
            .saturated_visible,
        0.05
    );
    assert_eq!(
        land_cover_soil_reflectance(LandCoverScheme::Igbp, 17)
            .unwrap()
            .saturated_visible,
        0.26
    );
    assert!(land_cover_soil_reflectance(LandCoverScheme::Igbp, 18).is_err());
}
