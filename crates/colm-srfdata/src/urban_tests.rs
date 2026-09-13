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

#[test]
fn ncar_urban_uses_regional_fallbacks_and_masks_missing_impervious_layers() {
    let table = ncar_table();
    let layout = FlatPatches::new(vec![1], vec![0, 2], vec![0, 1], vec![None]).unwrap();
    let raw = LczUrbanRawFields {
        roof_fraction: &[-1.0, 0.4],
        roof_height_m: &[-1.0, 8.0],
        tree_percent: &[20.0, 40.0],
        tree_top_m: &[4.0, 8.0],
        water_percent: &[10.0, 20.0],
        population_density: &[1.0, 3.0],
    };
    let geometry = aggregate_ncar_urban_geometry(
        &layout,
        &[1.0, 3.0],
        NcarUrbanRawFields {
            region_id: &[0, 2],
            geometry: raw,
        },
        &table,
        false,
    )
    .unwrap();
    assert!((geometry.roof_fraction[0] - 0.35).abs() < 1.0e-12);
    assert!((geometry.roof_height_m[0] - 9.0).abs() < 1.0e-12);
    assert_eq!(geometry.tree_percent, [35.0]);
    assert_eq!(geometry.tree_top_m, [7.0]);
    assert_eq!(geometry.water_percent, [17.5]);
    assert_eq!(geometry.population_density, [2.5]);

    let material = aggregate_ncar_urban_material(&layout, &[1.0, 3.0], &[0, 2], &table).unwrap();
    assert_eq!(material.pervious_road_fraction, [0.6]);
    assert_eq!(material.impervious_heat_capacity[0], 5.0);
    assert_eq!(material.impervious_heat_capacity[1], 0.0);
}

#[test]
fn ncar_table_reader_normalizes_layer_and_spectral_axes_once() {
    let table = ncar_table();
    let path = std::env::temp_dir().join(format!("colm-ncar-table-{}.nc", std::process::id()));
    let _ = std::fs::remove_file(&path);
    write_ncar_table(&path, &table);
    assert_eq!(NcarUrbanProperties::read(&path).unwrap(), table);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn bundled_ncar_table_decodes_with_its_production_axis_order() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/NCAR_urban_properties.nc");
    let table = NcarUrbanProperties::read(path).unwrap();
    assert_eq!((table.classes, table.regions), (3, 33));
}

fn ncar_table() -> NcarUrbanProperties {
    let classes = 3;
    let regions = 30;
    let scalar = vec![1.0; classes * regions];
    let mut roof_fraction = scalar.clone();
    roof_fraction[1] = 0.2;
    let mut roof_height_m = scalar.clone();
    roof_height_m[1] = 12.0;
    let mut canyon_height_to_width = scalar.clone();
    canyon_height_to_width[1] = 0.5;
    let mut pervious_road_fraction = scalar.clone();
    pervious_road_fraction[1] = 0.6;
    let layer = vec![1.0; classes * regions * URBAN_LAYERS];
    let mut impervious_heat_capacity = vec![-999.0; layer.len()];
    impervious_heat_capacity[URBAN_LAYERS] = 5.0;
    let mut impervious_thermal_conductivity = vec![-999.0; layer.len()];
    impervious_thermal_conductivity[URBAN_LAYERS] = 3.0;
    NcarUrbanProperties {
        classes,
        regions,
        roof_fraction,
        roof_height_m,
        canyon_height_to_width,
        pervious_road_fraction,
        roof_emissivity: scalar.clone(),
        wall_emissivity: scalar.clone(),
        impervious_emissivity: scalar.clone(),
        pervious_emissivity: scalar.clone(),
        roof_thickness_m: scalar.clone(),
        wall_thickness_m: scalar.clone(),
        room_min_k: scalar.clone(),
        room_max_k: scalar,
        roof_heat_capacity: layer.clone(),
        wall_heat_capacity: layer.clone(),
        impervious_heat_capacity,
        roof_thermal_conductivity: layer.clone(),
        wall_thermal_conductivity: layer,
        impervious_thermal_conductivity,
        roof_albedo: vec![0.2; classes * regions * 4],
        wall_albedo: vec![0.2; classes * regions * 4],
        impervious_albedo: vec![0.2; classes * regions * 4],
        pervious_albedo: vec![0.2; classes * regions * 4],
    }
}

fn write_ncar_table(path: &std::path::Path, table: &NcarUrbanProperties) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("density_class", table.classes).unwrap();
    file.add_dimension("region", table.regions).unwrap();
    file.add_dimension("ulev", URBAN_LAYERS).unwrap();
    file.add_dimension("numrad", URBAN_RADIATION_TYPES).unwrap();
    file.add_dimension("numsolar", URBAN_SOLAR_BANDS).unwrap();
    for (name, values) in [
        ("WTLUNIT_ROOF", &table.roof_fraction),
        ("HT_ROOF", &table.roof_height_m),
        ("CANYON_HWR", &table.canyon_height_to_width),
        ("WTROAD_PERV", &table.pervious_road_fraction),
        ("EM_ROOF", &table.roof_emissivity),
        ("EM_WALL", &table.wall_emissivity),
        ("EM_IMPROAD", &table.impervious_emissivity),
        ("EM_PERROAD", &table.pervious_emissivity),
        ("THICK_ROOF", &table.roof_thickness_m),
        ("THICK_WALL", &table.wall_thickness_m),
        ("T_BUILDING_MIN", &table.room_min_k),
        ("T_BUILDING_MAX", &table.room_max_k),
    ] {
        file.add_variable::<f64>(name, &["density_class", "region"])
            .unwrap()
            .put_values(values, (.., ..))
            .unwrap();
    }
    for (name, values) in [
        ("CV_ROOF", &table.roof_heat_capacity),
        ("CV_WALL", &table.wall_heat_capacity),
        ("CV_IMPROAD", &table.impervious_heat_capacity),
        ("TK_ROOF", &table.roof_thermal_conductivity),
        ("TK_WALL", &table.wall_thermal_conductivity),
        ("TK_IMPROAD", &table.impervious_thermal_conductivity),
    ] {
        file.add_variable::<f64>(name, &["density_class", "region", "ulev"])
            .unwrap()
            .put_values(values, (.., .., ..))
            .unwrap();
    }
    for (name, values) in [
        ("ALB_ROOF", &table.roof_albedo),
        ("ALB_WALL", &table.wall_albedo),
        ("ALB_IMPROAD", &table.impervious_albedo),
        ("ALB_PERROAD", &table.pervious_albedo),
    ] {
        let mut disk = Vec::with_capacity(values.len());
        for class in 0..table.classes {
            for region in 0..table.regions {
                for radiation in 0..URBAN_RADIATION_TYPES {
                    for solar in 0..URBAN_SOLAR_BANDS {
                        disk.push(
                            values[((class * table.regions + region) * URBAN_SOLAR_BANDS + solar)
                                * URBAN_RADIATION_TYPES
                                + radiation],
                        );
                    }
                }
            }
        }
        file.add_variable::<f64>(name, &["density_class", "region", "numrad", "numsolar"])
            .unwrap()
            .put_values(&disk, (.., .., .., ..))
            .unwrap();
    }
    file.close().unwrap();
}
