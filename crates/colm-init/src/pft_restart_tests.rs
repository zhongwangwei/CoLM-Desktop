use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn pft_constant_restart_matches_fortran_name_schema_and_crop_branch() {
    let root = temp_dir("constant");
    let path = write_pft_constant_restart(
        &root,
        "CN-Cng",
        2005,
        "w180_s90",
        PftConstantRestartInput {
            class: &[1, 3],
            fraction: &[0.25, 0.75],
            canopy_top_m: &[20.0, 5.0],
            canopy_bottom_m: &[2.0, 1.0],
            crop_fraction: Some(&[0.4, 0.6]),
        },
    )
    .unwrap();
    assert_eq!(
        path,
        root.join("const/CN-Cng_restart_pft_const_lc2005_w180_s90.nc")
    );

    let file = netcdf::open(&path).unwrap();
    assert_eq!(file.dimension_len("pft"), Some(2));
    assert_eq!(file.dimension_len("patch"), Some(2));
    assert_eq!(
        file.variable("pftclass")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [1, 3]
    );
    assert_eq!(
        file.variable("cropfrac")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.4, 0.6]
    );
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pft_time_restart_preserves_fortran_axis_order_and_feature_schema() {
    let fixture = Fixture::new();
    let root = temp_dir("time");
    let path = write_pft_time_restart(
        &root,
        "CN-Cng",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        "w180_s90",
        fixture.time_input(),
    )
    .unwrap();
    assert_eq!(
        path,
        root.join("2008-001-00000/CN-Cng_restart_pft_2008-001-00000_lc2005_w180_s90.nc")
    );

    let file = netcdf::open(&path).unwrap();
    assert_eq!(file.dimension_len("pft"), Some(2));
    assert_eq!(file.dimension_len("band"), Some(2));
    assert_eq!(file.dimension_len("rtyp"), Some(2));
    assert_eq!(file.dimension_len("wavelength"), Some(211));
    assert_eq!(file.dimension_len("vegnodes"), Some(2));
    let ssun = file.variable("ssun_p").unwrap();
    assert_eq!(dimension_names(&ssun), ["pft", "rtyp", "band"]);
    assert_eq!(
        ssun.get_values::<f64, _>(..).unwrap(),
        [0.0, 4.0, 2.0, 6.0, 1.0, 5.0, 3.0, 7.0]
    );
    let water_potential = file.variable("vegwp_p").unwrap();
    assert_eq!(dimension_names(&water_potential), ["pft", "vegnodes"]);
    assert_eq!(
        water_potential.get_values::<f64, _>(..).unwrap(),
        [0.0, 2.0, 1.0, 3.0]
    );
    assert_eq!(
        file.variable("irrig_method_p")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [2, 3]
    );
    assert_eq!(file.variables().count(), 29);
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pft_restart_rejects_invalid_feature_shapes_before_creating_a_file() {
    let root = temp_dir("invalid");
    assert!(write_pft_constant_restart(
        &root,
        "case",
        2005,
        "w180_s90",
        PftConstantRestartInput {
            class: &[1],
            fraction: &[1.0, 0.0],
            canopy_top_m: &[2.0],
            canopy_bottom_m: &[1.0],
            crop_fraction: None,
        },
    )
    .is_err());
    let fixture = Fixture::new();
    let mut invalid = fixture.time_input();
    invalid.plant_hydraulics = Some(PftPlantHydraulicFields {
        water_potential_mm: &[0.0],
        sunlit_stomatal_conductance: &fixture.pft,
        shaded_stomatal_conductance: &fixture.pft,
        vegetation_nodes: 2,
    });
    let path = root.join("invalid.nc");
    assert!(write_pft_time_restart_block(&path, invalid).is_err());
    assert!(!path.exists());
}

struct Fixture {
    pft: Vec<f64>,
    absorption: Vec<f64>,
    hyperspectral: Vec<f64>,
    water_potential: Vec<f64>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            pft: vec![1.0, 2.0],
            absorption: (0..8).map(f64::from).collect(),
            hyperspectral: (0..(WAVELENGTHS * RADIATION_TYPES * 2))
                .map(|value| value as f64)
                .collect(),
            water_potential: vec![0.0, 1.0, 2.0, 3.0],
        }
    }

    fn time_input(&self) -> PftTimeRestartInput<'_> {
        let pft = &self.pft;
        PftTimeRestartInput {
            fields: PftTimeFields {
                leaf_temperature_k: pft,
                canopy_water_mm: pft,
                canopy_rain_mm: pft,
                canopy_snow_mm: pft,
                wet_snow_fraction: pft,
                vegetation_fraction: pft,
                total_lai: pft,
                lai: pft,
                total_sai: pft,
                sai: pft,
                sunlit_absorption: &self.absorption,
                shaded_absorption: &self.absorption,
                thermal_gap_fraction: pft,
                shade_fraction: pft,
                direct_extinction: pft,
                diffuse_extinction: pft,
                reference_temperature_k: pft,
                reference_humidity: pft,
                stomatal_resistance_s_m: pft,
                roughness_length_m: pft,
            },
            hyperspectral: Some(PftHyperspectralFields {
                sunlit_absorption: &self.hyperspectral,
                shaded_absorption: &self.hyperspectral,
            }),
            plant_hydraulics: Some(PftPlantHydraulicFields {
                water_potential_mm: &self.water_potential,
                sunlit_stomatal_conductance: pft,
                shaded_stomatal_conductance: pft,
                vegetation_nodes: 2,
            }),
            ozone: Some(PftOzoneFields {
                lai_old: pft,
                sunlit_uptake: pft,
                shaded_uptake: pft,
            }),
            irrigation_method: Some(&[2, 3]),
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
        "colm-init-pft-restart-{label}-{}-{number}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
