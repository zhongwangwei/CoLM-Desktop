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

fn patches() -> FlatPatches {
    FlatPatches::new(
        vec![1, 7],
        vec![0, 2, 4],
        vec![0, 1, 2, 3],
        vec![None, None],
    )
    .unwrap()
}

#[test]
fn lcz_geometry_uses_lookup_values_and_upstream_missing_masks() {
    let values = aggregate_lcz_urban_geometry(
        &patches(),
        &[1, 7],
        &[1.0, 3.0, 2.0, 2.0],
        LczUrbanRawFields {
            roof_fraction: &[-1.0, 0.25, 0.5, 0.0],
            roof_height_m: &[0.0, 8.0, 6.0, -1.0],
            tree_percent: &[0.2, -1.0, 0.1, 0.3],
            tree_top_m: &[10.0, 9.0, 5.0, -1.0],
            water_percent: &[-1.0, 0.4, 0.2, -1.0],
            population_density: &[100.0, -2.0, 10.0, 30.0],
        },
        false,
    )
    .unwrap();
    assert!((values.roof_fraction[0] - 0.3125).abs() < 1e-12);
    assert!((values.roof_height_m[0] - 17.25).abs() < 1e-12);
    assert!((values.building_height_to_width[0] - 1.035_533_905_932_737_5).abs() < 1e-12);
    assert_eq!(values.tree_percent, vec![0.2, 0.1]);
    assert_eq!(values.tree_top_m, vec![10.0, 5.0]);
    assert!((values.water_percent[0] - 0.4).abs() < 1e-12);
    assert_eq!(values.water_percent[1], 0.2);
    assert_eq!(values.population_density, vec![100.0, 20.0]);
    assert_eq!(values.roof_fraction[1], 0.65);
    assert_eq!(values.roof_height_m[1], 4.5);
}

#[test]
fn urban_tree_indices_and_lucy_ids_follow_their_distinct_weights() {
    let layout = patches();
    let index = aggregate_urban_tree_index(
        &layout,
        &[1.0, 3.0, 2.0, 2.0],
        &[0.2, -1.0, 0.1, 0.3],
        &[2.0, 100.0, 4.0, 8.0],
    )
    .unwrap();
    assert_eq!(index[0], 2.0);
    assert!((index[1] - 7.0).abs() < 1e-12);
    assert_eq!(
        aggregate_urban_region_ids(&layout, &[5, 5, 8, 7]).unwrap(),
        vec![5, 7]
    );
}
