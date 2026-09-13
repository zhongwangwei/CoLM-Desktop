use super::*;

#[test]
fn lcz_materials_match_the_upstream_class_lookup_and_vector_layout() {
    let values = UrbanMaterialParameters::from_lcz_classes(&[1, 7]).unwrap();
    values.validate(2).unwrap();
    assert!((values.pervious_road_fraction[0] - 0.1).abs() < 1e-12);
    assert!((values.pervious_road_fraction[1] - 0.75).abs() < 1e-12);
    assert_eq!(values.roof_emissivity, vec![0.91, 0.28]);
    assert_eq!(values.roof_heat_capacity[0], 1.8e6);
    assert_eq!(values.roof_heat_capacity[1], 2.0e6);
    assert_eq!(values.roof_heat_capacity[2], 1.8e6);
    assert_eq!(values.impervious_albedo[1], 0.18);
    assert_eq!(values.impervious_albedo[3], 0.18);
}

#[test]
fn lcz_materials_reject_invalid_classes() {
    assert!(UrbanMaterialParameters::from_lcz_classes(&[]).is_err());
    assert!(UrbanMaterialParameters::from_lcz_classes(&[11]).is_err());
}
