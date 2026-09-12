use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
static PATCH: [f64; 2] = [10.0, 20.0];
static SNOW: [f64; 4] = [1.0, 10.0, 2.0, 20.0];
static SOIL: [f64; 4] = [3.0, 30.0, 4.0, 40.0];
static SOILSNOW: [f64; 8] = [1.0, 11.0, 2.0, 12.0, 3.0, 13.0, 4.0, 14.0];
static RADIATION: [f64; 8] = [0.0, 100.0, 1.0, 101.0, 10.0, 110.0, 11.0, 111.0];
static SNOW_RADIATION: [f64; 24] = [
    0.0, 100.0, 1.0, 101.0, 2.0, 102.0, 10.0, 110.0, 11.0, 111.0, 12.0, 112.0, 20.0, 120.0, 21.0,
    121.0, 22.0, 122.0, 30.0, 130.0, 31.0, 131.0, 32.0, 132.0,
];

#[test]
fn time_restart_matches_fortran_filename_dimensions_and_axis_order() {
    let root = temp_dir("core");
    let result = write_time_restart(
        &root,
        "CN-Cng",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        "w180_s90",
        input(),
    )
    .unwrap();
    assert_eq!(
        result.block,
        root.join("2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc")
    );

    let file = netcdf::open(&result.block).unwrap();
    assert_eq!(file.dimension_len("patch"), Some(2));
    assert_eq!(file.dimension_len("snow"), Some(2));
    assert_eq!(file.dimension_len("snowp1"), Some(3));
    assert_eq!(file.dimension_len("soilsnow"), Some(4));
    assert_eq!(file.dimension_len("vegnodes"), Some(2));
    assert_eq!(
        file.variable("z_sno")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [1.0, 2.0, 10.0, 20.0]
    );
    let albedo = file.variable("alb").unwrap();
    assert_eq!(dimension_names(&albedo), ["patch", "rtyp", "band"]);
    assert_eq!(
        albedo.get_values::<f64, _>(..).unwrap(),
        patch_last_3d(&RADIATION, 2, 2, 2)
    );
    let snow_layers = file.variable("ssno_lyr").unwrap();
    assert_eq!(
        dimension_names(&snow_layers),
        ["patch", "snowp1", "rtyp", "band"]
    );
    assert_eq!(
        snow_layers.get_values::<f64, _>(..).unwrap(),
        patch_last_4d(&SNOW_RADIATION, 2, 2, 3, 2)
    );
    for name in [
        "dz_lake",
        "vegwp",
        "o3coefg_sha",
        "irrig_method_sugarcane",
        "zwt_stand",
    ] {
        assert!(file.variable(name).is_some(), "missing {name}");
    }
    assert_eq!(file.variables().count(), 93);
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn time_restart_rejects_invalid_date_and_incomplete_axis_data_before_writing() {
    let root = temp_dir("invalid");
    assert!(write_time_restart(
        &root,
        "case",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 0,
            seconds: 0,
        },
        "w180_s90",
        input(),
    )
    .is_err());
    let mut invalid = input();
    invalid.snow_soil.snow_node_depth_m = &[1.0];
    let path = root.join("block.nc");
    assert!(write_time_restart_block(&path, invalid).is_err());
    assert!(!path.exists());
}

#[test]
#[ignore = "requires the locally generated upstream CN-Cng reference restart"]
fn time_restart_schema_matches_the_upstream_fortran_reference() {
    let fixture = StandardFixture::new();
    let path = temp_dir("upstream-schema").join("restart.nc");
    write_time_restart_block(&path, fixture.input()).unwrap();

    let reference_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../oracle/work/generated/out/CN-Cng/restart/2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc",
    );
    let reference = netcdf::open(&reference_path)
        .unwrap_or_else(|error| panic!("cannot open {}: {error}", reference_path.display()));
    let actual = netcdf::open(&path).unwrap();
    assert_eq!(variable_names(&actual), variable_names(&reference));
    assert_eq!(dimension_lengths(&actual), dimension_lengths(&reference));
    drop(actual);
    drop(reference);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn input() -> TimeRestartInput<'static> {
    TimeRestartInput {
        dimensions: TimeRestartDimensions {
            soil_layers: 2,
            lake_layers: 2,
            snow_layers: 2,
            bands: 2,
            radiation_types: 2,
        },
        snow_soil: SnowSoilRestartFields {
            snow_node_depth_m: &SNOW,
            snow_layer_thickness_m: &SNOW,
            temperature_k: &SOILSNOW,
            liquid_water_kg_m2: &SOILSNOW,
            ice_water_kg_m2: &SOILSNOW,
            matric_potential_mm: &SOIL,
            hydraulic_conductivity_mm_s: &SOIL,
        },
        patch: TimePatchFields {
            ground_temperature_k: &PATCH,
            leaf_temperature_k: &PATCH,
            canopy_water_mm: &PATCH,
            canopy_rain_mm: &PATCH,
            canopy_snow_mm: &PATCH,
            wet_snow_fraction: &PATCH,
            snow_age: &PATCH,
            snow_water_equivalent_mm: &PATCH,
            snow_depth_m: &PATCH,
            vegetation_fraction: &PATCH,
            ground_snow_fraction: &PATCH,
            snow_free_vegetation_fraction: &PATCH,
            greenness: &PATCH,
            lai: &PATCH,
            total_lai: &PATCH,
            sai: &PATCH,
            total_sai: &PATCH,
            cosine_zenith: &PATCH,
            thermal_gap_fraction: &PATCH,
            direct_extinction: &PATCH,
            diffuse_extinction: &PATCH,
            water_table_depth_m: &PATCH,
            aquifer_water_mm: &PATCH,
            wetland_water_mm: &PATCH,
            surface_water_mm: &PATCH,
            soil_surface_resistance_s_m: &PATCH,
            saved_tke: &PATCH,
            radiative_temperature_k: &PATCH,
            reference_temperature_k: &PATCH,
            reference_humidity: &PATCH,
            stomatal_resistance_s_m: &PATCH,
            emissivity: &PATCH,
            roughness_length_m: &PATCH,
            monin_obukhov_height: &PATCH,
            bulk_richardson: &PATCH,
            friction_velocity: &PATCH,
            humidity_scale: &PATCH,
            temperature_scale_k: &PATCH,
            momentum_integral: &PATCH,
            heat_integral: &PATCH,
            moisture_integral: &PATCH,
        },
        radiation: TimeRadiationFields {
            albedo: &RADIATION,
            sunlit_absorption: &RADIATION,
            shaded_absorption: &RADIATION,
            soil_absorption: &RADIATION,
            snow_absorption: &RADIATION,
            snow_layer_absorption: &SNOW_RADIATION,
        },
        lake: TimeLakeFields {
            temperature_k: &SOIL,
            ice_fraction: &SOIL,
            layer_thickness_m: Some(&SOIL),
        },
        snow_aerosol: SnowAerosolFields {
            grain_radius: &SNOW,
            black_carbon_hydrophobic: &SNOW,
            black_carbon_hydrophilic: &SNOW,
            organic_carbon_hydrophobic: &SNOW,
            organic_carbon_hydrophilic: &SNOW,
            dust_1: &SNOW,
            dust_2: &SNOW,
            dust_3: &SNOW,
            dust_4: &SNOW,
        },
        plant_hydraulics: Some(PlantHydraulicFields {
            water_potential_mm: &SNOW,
            sunlit_stomatal_conductance: &PATCH,
            shaded_stomatal_conductance: &PATCH,
            vegetation_nodes: 2,
        }),
        ozone: Some(OzoneFields {
            lai_old: &PATCH,
            sunlit_uptake: &PATCH,
            shaded_uptake: &PATCH,
            sunlit_vegetation_coefficient: &PATCH,
            shaded_vegetation_coefficient: &PATCH,
            sunlit_ground_coefficient: &PATCH,
            shaded_ground_coefficient: &PATCH,
        }),
        irrigation: Some(IrrigationFields {
            rate: &PATCH,
            cumulative: &PATCH,
            cumulative_deficit: &PATCH,
            event_count: &PATCH,
            steps_left: &PATCH,
            water_storage: &PATCH,
            corn_method: &PATCH,
            spring_wheat_method: &PATCH,
            winter_wheat_method: &PATCH,
            soybean_method: &PATCH,
            cotton_method: &PATCH,
            rice_1_method: &PATCH,
            rice_2_method: &PATCH,
            sugarcane_method: &PATCH,
            groundwater_allocation: &PATCH,
            surface_water_allocation: &PATCH,
            standard_water_table_depth: &PATCH,
        }),
    }
}

struct StandardFixture {
    one: Vec<f64>,
    snow: Vec<f64>,
    soilsnow: Vec<f64>,
    soil: Vec<f64>,
    lake: Vec<f64>,
    plant_water_potential: Vec<f64>,
    radiation: Vec<f64>,
    snow_radiation: Vec<f64>,
}

impl StandardFixture {
    fn new() -> Self {
        Self {
            one: vec![0.0],
            snow: vec![0.0; 5],
            soilsnow: vec![0.0; 15],
            soil: vec![0.0; 10],
            lake: vec![0.0; 10],
            plant_water_potential: vec![0.0; 4],
            radiation: vec![0.0; 4],
            snow_radiation: vec![0.0; 24],
        }
    }

    fn input(&self) -> TimeRestartInput<'_> {
        let patch = &self.one;
        TimeRestartInput {
            dimensions: TimeRestartDimensions::default(),
            snow_soil: SnowSoilRestartFields {
                snow_node_depth_m: &self.snow,
                snow_layer_thickness_m: &self.snow,
                temperature_k: &self.soilsnow,
                liquid_water_kg_m2: &self.soilsnow,
                ice_water_kg_m2: &self.soilsnow,
                matric_potential_mm: &self.soil,
                hydraulic_conductivity_mm_s: &self.soil,
            },
            patch: TimePatchFields {
                ground_temperature_k: patch,
                leaf_temperature_k: patch,
                canopy_water_mm: patch,
                canopy_rain_mm: patch,
                canopy_snow_mm: patch,
                wet_snow_fraction: patch,
                snow_age: patch,
                snow_water_equivalent_mm: patch,
                snow_depth_m: patch,
                vegetation_fraction: patch,
                ground_snow_fraction: patch,
                snow_free_vegetation_fraction: patch,
                greenness: patch,
                lai: patch,
                total_lai: patch,
                sai: patch,
                total_sai: patch,
                cosine_zenith: patch,
                thermal_gap_fraction: patch,
                direct_extinction: patch,
                diffuse_extinction: patch,
                water_table_depth_m: patch,
                aquifer_water_mm: patch,
                wetland_water_mm: patch,
                surface_water_mm: patch,
                soil_surface_resistance_s_m: patch,
                saved_tke: patch,
                radiative_temperature_k: patch,
                reference_temperature_k: patch,
                reference_humidity: patch,
                stomatal_resistance_s_m: patch,
                emissivity: patch,
                roughness_length_m: patch,
                monin_obukhov_height: patch,
                bulk_richardson: patch,
                friction_velocity: patch,
                humidity_scale: patch,
                temperature_scale_k: patch,
                momentum_integral: patch,
                heat_integral: patch,
                moisture_integral: patch,
            },
            radiation: TimeRadiationFields {
                albedo: &self.radiation,
                sunlit_absorption: &self.radiation,
                shaded_absorption: &self.radiation,
                soil_absorption: &self.radiation,
                snow_absorption: &self.radiation,
                snow_layer_absorption: &self.snow_radiation,
            },
            lake: TimeLakeFields {
                temperature_k: &self.lake,
                ice_fraction: &self.lake,
                layer_thickness_m: None,
            },
            snow_aerosol: SnowAerosolFields {
                grain_radius: &self.snow,
                black_carbon_hydrophobic: &self.snow,
                black_carbon_hydrophilic: &self.snow,
                organic_carbon_hydrophobic: &self.snow,
                organic_carbon_hydrophilic: &self.snow,
                dust_1: &self.snow,
                dust_2: &self.snow,
                dust_3: &self.snow,
                dust_4: &self.snow,
            },
            plant_hydraulics: Some(PlantHydraulicFields {
                water_potential_mm: &self.plant_water_potential,
                sunlit_stomatal_conductance: patch,
                shaded_stomatal_conductance: patch,
                vegetation_nodes: 4,
            }),
            ozone: None,
            irrigation: None,
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

fn variable_names(file: &netcdf::File) -> Vec<String> {
    let mut names = file
        .variables()
        .map(|variable| variable.name())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}

fn dimension_lengths(file: &netcdf::File) -> Vec<(String, usize)> {
    let mut dimensions = file
        .dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect::<Vec<_>>();
    dimensions.sort_unstable();
    dimensions
}

fn patch_last_3d(values: &[f64], first: usize, second: usize, patches: usize) -> Vec<f64> {
    (0..patches)
        .flat_map(|patch| {
            (0..second).flat_map(move |second_index| {
                (0..first).map(move |first_index| {
                    values[(first_index * second + second_index) * patches + patch]
                })
            })
        })
        .collect()
}

fn patch_last_4d(
    values: &[f64],
    first: usize,
    second: usize,
    third: usize,
    patches: usize,
) -> Vec<f64> {
    (0..patches)
        .flat_map(|patch| {
            (0..third).flat_map(move |third_index| {
                (0..second).flat_map(move |second_index| {
                    (0..first).map(move |first_index| {
                        values[((first_index * second + second_index) * third + third_index)
                            * patches
                            + patch]
                    })
                })
            })
        })
        .collect()
}

fn temp_dir(label: &str) -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "colm-init-time-restart-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
