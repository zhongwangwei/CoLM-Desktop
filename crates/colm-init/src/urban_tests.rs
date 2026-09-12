use super::*;

fn input() -> UrbanInput<'static> {
    UrbanInput {
        urban_to_patch: &[1, 0],
        roof_fraction: &[0.7, 0.3],
        roof_height_m: &[8.0, 5.0],
        building_height_to_width: &[1.0, 2.0],
        pervious_road_fraction: &[0.2, 0.4],
        water_percent: &[20.0, 0.0],
        tree_percent: &[60.0, 30.0],
        tree_top_m: &[10.0, 1.0],
        impervious_heat_capacity: &[0.0, 5.0, 0.0, 5.0],
        impervious_layers: 2,
        roof_thickness_m: &[1.0, 2.0],
        wall_thickness_m: &[2.0, 4.0],
        room_max_k: &[280.0, 281.0],
        room_min_k: &[275.0, 276.0],
    }
}

#[test]
fn urban_geometry_matches_fraction_height_and_layer_rules() {
    let state = derive_urban_geometry(
        input(),
        UrbanConfig {
            water_enabled: true,
            trees_enabled: true,
            building_energy_model: false,
        },
        2,
        2,
        0.5,
        0.0,
        2,
    )
    .unwrap();
    assert_eq!(state.roof_fraction, vec![0.7, 0.3]);
    assert_eq!(state.roof_height_m, vec![8.0, 5.0]);
    assert_eq!(state.building_height_to_width, vec![1.0, 2.0]);
    assert_eq!(state.pervious_road_fraction, vec![1.0, 0.4]);
    assert_eq!(state.water_fraction, vec![0.2, 0.0]);
    assert!((state.tree_fraction[0] - 0.3).abs() < 1e-14);
    assert!((state.tree_fraction[1] - 0.3).abs() < 1e-14);
    assert!((state.patch_tree_fraction[0] - 0.3).abs() < 1e-14);
    assert!((state.patch_tree_fraction[1] - 0.3).abs() < 1e-14);
    assert_eq!(state.tree_top_m, vec![8.0, 2.0]);
    assert_eq!(state.tree_bottom_m, vec![1.0, 1.0]);
    assert_eq!(state.roof_node_depth_m, vec![0.25, 0.5, 0.75, 1.5]);
    assert_eq!(state.roof_layer_thickness_m, vec![0.5, 1.0, 0.5, 1.0]);
    assert_eq!(state.wall_layer_thickness_m, vec![1.0, 2.0, 1.0, 2.0]);
    assert_eq!(state.room_max_k, vec![373.16, 373.16]);
    assert_eq!(state.room_min_k, vec![180.0, 180.0]);
}

#[test]
fn disabled_urban_features_zero_their_fractions_and_preserve_bem_temperatures() {
    let state = derive_urban_geometry(
        input(),
        UrbanConfig {
            water_enabled: false,
            trees_enabled: false,
            building_energy_model: true,
        },
        2,
        2,
        0.5,
        0.0,
        2,
    )
    .unwrap();
    assert_eq!(state.water_fraction, vec![0.0, 0.0]);
    assert_eq!(state.tree_fraction, vec![0.0, 0.0]);
    assert_eq!(state.room_max_k, vec![280.0, 281.0]);
}

#[test]
fn urban_geometry_rejects_invalid_shapes_and_single_layer_stencils() {
    assert!(derive_urban_geometry(
        input(),
        UrbanConfig {
            water_enabled: true,
            trees_enabled: true,
            building_energy_model: false,
        },
        1,
        2,
        0.5,
        0.0,
        2,
    )
    .is_err());
}
