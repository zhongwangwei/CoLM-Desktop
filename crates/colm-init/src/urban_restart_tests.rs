use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::RestartDate;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn urban_constant_restart_matches_fortran_filename_schema_and_axis_order() {
    let fixture = Fixture::new();
    let root = temp_dir("core");
    let path =
        write_urban_constant_restart(&root, "CN-Cng", 2005, "w180_s90", fixture.input()).unwrap();
    assert_eq!(
        path,
        root.join("const/CN-Cng_restart_urb_const_lc2005_w180_s90.nc")
    );

    let file = netcdf::open(&path).unwrap();
    for (name, length) in [
        ("urban", 2),
        ("numsolar", 2),
        ("numrad", 2),
        ("ulev", 10),
        ("ityp", 3),
        ("iweek", 7),
        ("ihour", 24),
        ("iday", 365),
    ] {
        assert_eq!(file.dimension_len(name), Some(length));
    }
    let albedo = file.variable("ALB_ROOF").unwrap();
    assert_eq!(dimension_names(&albedo), ["urban", "numrad", "numsolar"]);
    assert_eq!(
        albedo.get_values::<f64, _>(..).unwrap(),
        [0.0, 4.0, 2.0, 6.0, 1.0, 5.0, 3.0, 7.0]
    );
    let depth = file.variable("ROOF_DEPTH_L").unwrap();
    assert_eq!(dimension_names(&depth), ["urban", "ulev"]);
    assert_eq!(
        depth.get_values::<f64, _>(..).unwrap()[..4],
        [0.0, 2.0, 4.0, 6.0]
    );
    assert_eq!(file.variables().count(), 35);
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn urban_constant_restart_rejects_wrong_layer_shape_before_writing() {
    let fixture = Fixture::new();
    let mut thermal = fixture.thermal();
    thermal.roof_heat_capacity = &[0.0; 19];
    let root = temp_dir("invalid");
    let path = root.join("invalid.nc");
    assert!(write_urban_constant_restart_block(
        &path,
        UrbanConstantRestartInput {
            state: &fixture.state,
            lucy: &fixture.lucy,
            thermal,
        },
    )
    .is_err());
    assert!(!path.exists());
}

struct Fixture {
    state: UrbanState,
    lucy: UrbanLucyState,
    albedo: Vec<f64>,
    layers: Vec<f64>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            state: UrbanState {
                roof_fraction: vec![0.2, 0.3],
                roof_height_m: vec![10.0, 12.0],
                building_height_to_width: vec![1.0, 1.5],
                pervious_road_fraction: vec![0.4, 0.5],
                water_fraction: vec![0.1, 0.2],
                tree_fraction: vec![0.3, 0.2],
                patch_tree_fraction: vec![0.3, 0.2],
                tree_top_m: vec![4.0, 5.0],
                tree_bottom_m: vec![1.0, 1.5],
                roof_node_depth_m: (0..20).map(f64::from).collect(),
                roof_layer_thickness_m: vec![1.0; 20],
                wall_node_depth_m: vec![2.0; 20],
                wall_layer_thickness_m: vec![3.0; 20],
                room_max_k: vec![300.0, 301.0],
                room_min_k: vec![280.0, 281.0],
            },
            lucy: UrbanLucyState {
                population_density: vec![100.0, 200.0],
                vehicles_per_thousand: vec![1.0; 6],
                week_holiday: vec![1.0; 14],
                weekend_traffic_profile: vec![1.0; 48],
                weekday_traffic_profile: vec![1.0; 48],
                human_metabolic_profile: vec![1.0; 48],
                fixed_holiday: vec![1.0; 730],
            },
            albedo: (0..8).map(f64::from).collect(),
            layers: vec![1.0; 20],
        }
    }

    fn thermal(&self) -> UrbanThermalFields<'_> {
        UrbanThermalFields {
            roof_albedo: &self.albedo,
            wall_albedo: &self.albedo,
            impervious_albedo: &self.albedo,
            pervious_albedo: &self.albedo,
            roof_emissivity: &[0.9, 0.9],
            wall_emissivity: &[0.9, 0.9],
            impervious_emissivity: &[0.9, 0.9],
            pervious_emissivity: &[0.9, 0.9],
            roof_heat_capacity: &self.layers,
            wall_heat_capacity: &self.layers,
            impervious_heat_capacity: &self.layers,
            roof_thermal_conductivity: &self.layers,
            wall_thermal_conductivity: &self.layers,
            impervious_thermal_conductivity: &self.layers,
        }
    }

    fn input(&self) -> UrbanConstantRestartInput<'_> {
        UrbanConstantRestartInput {
            state: &self.state,
            lucy: &self.lucy,
            thermal: self.thermal(),
        }
    }
}

fn dimension_names(variable: &netcdf::Variable<'_>) -> Vec<String> {
    variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect()
}

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-urban-restart-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

#[test]
fn urban_time_restart_writes_the_complete_upstream_field_family() {
    let root = temp_dir("time");
    let scalar_values = vec![10.0, 20.0];
    let radiative_values = (0..8).map(f64::from).collect::<Vec<_>>();
    let scalar_fields = URBAN_TIME_SCALARS
        .iter()
        .map(|name| UrbanNamedField {
            name,
            values: &scalar_values,
        })
        .collect::<Vec<_>>();
    let radiative_fields = URBAN_TIME_RADIATIVE
        .iter()
        .map(|name| UrbanNamedField {
            name,
            values: &radiative_values,
        })
        .collect::<Vec<_>>();
    let dimensions = UrbanTimeRestartDimensions {
        urban_count: 2,
        snow_layers: 2,
        soil_layers: 3,
        roof_layers: 4,
        wall_layers: 5,
    };
    let schema = urban_layer_schema(dimensions);
    let layer_values = schema
        .iter()
        .map(|(_, _, layers)| {
            (0..(layers * 2))
                .map(|value| value as f64)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let layer_fields = schema
        .iter()
        .zip(&layer_values)
        .map(|((name, _, _), values)| UrbanNamedField { name, values })
        .collect::<Vec<_>>();
    let path = write_urban_time_restart(
        &root,
        "CN-Cng",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        "w180_s90",
        UrbanTimeRestartInput {
            dimensions,
            scalar_fields: &scalar_fields,
            radiative_fields: &radiative_fields,
            layer_fields: &layer_fields,
        },
    )
    .unwrap();
    assert_eq!(
        path,
        root.join("2008-001-00000/CN-Cng_restart_urban_2008-001-00000_lc2005_w180_s90.nc")
    );

    let file = netcdf::open(&path).unwrap();
    assert_eq!(file.dimension_len("soilsnow"), Some(5));
    assert_eq!(file.dimension_len("roofsnow"), Some(6));
    assert_eq!(file.dimension_len("wallsnow"), Some(7));
    assert_eq!(file.variables().count(), 68);
    let shortwave = file.variable("sroof").unwrap();
    assert_eq!(dimension_names(&shortwave), ["urban", "rtyp", "band"]);
    assert_eq!(
        shortwave.get_values::<f64, _>(..).unwrap(),
        [0.0, 4.0, 2.0, 6.0, 1.0, 5.0, 3.0, 7.0]
    );
    let snow = file.variable("z_sno_roof").unwrap();
    assert_eq!(dimension_names(&snow), ["urban", "snow"]);
    assert_eq!(snow.get_values::<f64, _>(..).unwrap(), [0.0, 2.0, 1.0, 3.0]);
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn urban_time_restart_rejects_missing_schema_fields_before_writing() {
    let root = temp_dir("time-invalid");
    let values = vec![0.0];
    let mut scalar_fields = URBAN_TIME_SCALARS
        .iter()
        .map(|name| UrbanNamedField {
            name,
            values: values.as_slice(),
        })
        .collect::<Vec<_>>();
    scalar_fields[0].name = "not_fwsun";
    let radiative = vec![0.0; 4];
    let radiative_fields = URBAN_TIME_RADIATIVE
        .iter()
        .map(|name| UrbanNamedField {
            name,
            values: radiative.as_slice(),
        })
        .collect::<Vec<_>>();
    let dimensions = UrbanTimeRestartDimensions {
        urban_count: 1,
        snow_layers: 1,
        soil_layers: 1,
        roof_layers: 1,
        wall_layers: 1,
    };
    let schema = urban_layer_schema(dimensions);
    let layer_values = schema
        .iter()
        .map(|(_, _, layers)| vec![0.0; *layers])
        .collect::<Vec<_>>();
    let layer_fields = schema
        .iter()
        .zip(&layer_values)
        .map(|((name, _, _), values)| UrbanNamedField { name, values })
        .collect::<Vec<_>>();
    let path = root.join("invalid.nc");
    assert!(write_urban_time_restart_block(
        &path,
        UrbanTimeRestartInput {
            dimensions,
            scalar_fields: &scalar_fields,
            radiative_fields: &radiative_fields,
            layer_fields: &layer_fields,
        },
    )
    .is_err());
    assert!(!path.exists());
}
