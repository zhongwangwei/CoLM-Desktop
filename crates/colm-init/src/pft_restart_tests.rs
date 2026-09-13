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
    assert_eq!(
        file.variable("o3coefg_sha_p")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [1.0, 2.0]
    );
    assert_eq!(file.variables().count(), 33);
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pft_bgc_restart_writes_every_upstream_carbon_nitrogen_field_in_order() {
    let fixture = Fixture::new();
    let values = vec![fixture.pft.as_slice(); PFT_BGC_F64_VARIABLES.len()];
    let mut input = fixture.time_input();
    input.bgc = Some(PftBgcFields {
        values: &values,
        active_crop_years: &[2, 3],
    });
    let root = temp_dir("bgc");
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
        input,
    )
    .unwrap();
    let file = netcdf::open(&path).unwrap();
    let names = file
        .variables()
        .map(|variable| variable.name())
        .collect::<Vec<_>>();
    let mut expected = PFT_BGC_F64_VARIABLES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    expected.insert(BGC_ACTIVE_CROP_YEARS_AFTER, "nyrs_crop_active_p".to_owned());
    let bgc_start = names.iter().position(|name| name == "leafc_p").unwrap();
    assert_eq!(&names[bgc_start..], expected);
    assert!(
        names
            .iter()
            .position(|name| name == "irrig_method_p")
            .unwrap()
            < bgc_start,
        "upstream writes optional ozone and irrigation state before BGC PFT state"
    );
    assert_eq!(
        file.variable("nyrs_crop_active_p")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [2, 3]
    );
    assert_eq!(
        file.variable("npool_p")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [1.0, 2.0]
    );
    drop(file);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires the local BGC kernel and CoLMruntime cnsteadystate.nc reference data"]
fn pft_bgc_restart_matches_the_upstream_fortran_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let runtime = std::path::Path::new("/Users/zhongwangwei/Desktop/Data/CoLMruntime");
    let upstream_surface = root.join("kernels/bgc/mksrfdata.x");
    let upstream_init = root.join("kernels/bgc/mkinidata.x");
    assert!(runtime.join("cnsteadystate.nc").is_file());
    assert!(upstream_surface.is_file());
    assert!(upstream_init.is_file());

    let directory = temp_dir("bgc-fortran-reference");
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("out");
    let template = root.join("oracle/work/generated/case.nml");
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let original_runtime = format!("{}/oracle/work/generated/runtime_unused/", root.display());
    let namelist = std::fs::read_to_string(template)
        .unwrap()
        .replace(
            &format!("DEF_dir_output = '{original_output}'"),
            &format!("DEF_dir_output = '{}/'", output.display()),
        )
        .replace(
            &format!("DEF_dir_runtime = '{original_runtime}'"),
            &format!("DEF_dir_runtime = '{}/'", runtime.display()),
        )
        .replace(
            "&nl_colm",
            &format!(
                "&nl_colm\nDEF_USE_LCT = .false.\nDEF_USE_PFT = .true.\nDEF_USE_BGC = .true.\nDEF_USE_CN_INIT = .true.\nDEF_file_cn_init = '{}/cnsteadystate.nc'",
                runtime.display()
            ),
        );
    let case = directory.join("case.nml");
    std::fs::write(&case, namelist).unwrap();
    for executable in [&upstream_surface, &upstream_init] {
        let result = std::process::Command::new(executable)
            .arg(&case)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{} failed:\n{}\n{}",
            executable.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let reference_path = output
        .join("CN-Cng/restart/2008-001-00000/CN-Cng_restart_pft_2008-001-00000_lc2005_w180_s90.nc");
    let reference = netcdf::open(&reference_path).unwrap();
    let pfts = reference.dimension_len("pft").unwrap();
    let values = |name| {
        reference
            .variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    };
    let base_names = [
        "tleaf_p",
        "ldew_p",
        "ldew_rain_p",
        "ldew_snow_p",
        "fwet_snow_p",
        "sigf_p",
        "tlai_p",
        "lai_p",
        "tsai_p",
        "sai_p",
        "thermk_p",
        "fshade_p",
        "extkb_p",
        "extkd_p",
        "tref_p",
        "qref_p",
        "rst_p",
        "z0m_p",
    ];
    let base = base_names
        .iter()
        .map(|name| values(name))
        .collect::<Vec<_>>();
    let sunlit = inverse_pft_last(values("ssun_p"), BANDS, RADIATION_TYPES, pfts);
    let shaded = inverse_pft_last(values("ssha_p"), BANDS, RADIATION_TYPES, pfts);
    let vegetation_nodes = reference.dimension_len("vegnodes").unwrap();
    let water_potential = inverse_axis(values("vegwp_p"), vegetation_nodes, pfts);
    let sunlit_conductance = values("gs0sun_p");
    let shaded_conductance = values("gs0sha_p");
    let bgc_values = PFT_BGC_F64_VARIABLES
        .iter()
        .map(|name| values(name))
        .collect::<Vec<_>>();
    let bgc_refs = bgc_values.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let active_crop_years = reference
        .variable("nyrs_crop_active_p")
        .unwrap()
        .get_values::<i32, _>(..)
        .unwrap();
    let native = write_pft_time_restart(
        directory.join("native/restart"),
        "CN-Cng",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        "w180_s90",
        PftTimeRestartInput {
            fields: PftTimeFields {
                leaf_temperature_k: &base[0],
                canopy_water_mm: &base[1],
                canopy_rain_mm: &base[2],
                canopy_snow_mm: &base[3],
                wet_snow_fraction: &base[4],
                vegetation_fraction: &base[5],
                total_lai: &base[6],
                lai: &base[7],
                total_sai: &base[8],
                sai: &base[9],
                sunlit_absorption: &sunlit,
                shaded_absorption: &shaded,
                thermal_gap_fraction: &base[10],
                shade_fraction: &base[11],
                direct_extinction: &base[12],
                diffuse_extinction: &base[13],
                reference_temperature_k: &base[14],
                reference_humidity: &base[15],
                stomatal_resistance_s_m: &base[16],
                roughness_length_m: &base[17],
            },
            hyperspectral: None,
            plant_hydraulics: Some(PftPlantHydraulicFields {
                water_potential_mm: &water_potential,
                sunlit_stomatal_conductance: &sunlit_conductance,
                shaded_stomatal_conductance: &shaded_conductance,
                vegetation_nodes,
            }),
            bgc: Some(PftBgcFields {
                values: &bgc_refs,
                active_crop_years: &active_crop_years,
            }),
            ozone: None,
            irrigation_method: None,
        },
    )
    .unwrap();
    compare_netcdf(&native, &reference_path);
    std::fs::remove_dir_all(directory).unwrap();
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
    let mut invalid_bgc = fixture.time_input();
    invalid_bgc.bgc = Some(PftBgcFields {
        values: &[],
        active_crop_years: &[1, 2],
    });
    assert!(write_pft_time_restart_block(&path, invalid_bgc).is_err());
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
            bgc: None,
            ozone: Some(PftOzoneFields {
                lai_old: pft,
                sunlit_uptake: pft,
                shaded_uptake: pft,
                sunlit_vegetation_coefficient: pft,
                shaded_vegetation_coefficient: pft,
                sunlit_stomatal_coefficient: pft,
                shaded_stomatal_coefficient: pft,
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

fn inverse_axis(on_disk: Vec<f64>, axis: usize, pfts: usize) -> Vec<f64> {
    let mut fortran = vec![0.0; on_disk.len()];
    for pft in 0..pfts {
        for index in 0..axis {
            fortran[index * pfts + pft] = on_disk[pft * axis + index];
        }
    }
    fortran
}

fn inverse_pft_last(on_disk: Vec<f64>, first: usize, second: usize, pfts: usize) -> Vec<f64> {
    let mut fortran = vec![0.0; on_disk.len()];
    for pft in 0..pfts {
        for second_index in 0..second {
            for first_index in 0..first {
                fortran[(first_index * second + second_index) * pfts + pft] =
                    on_disk[(pft * second + second_index) * first + first_index];
            }
        }
    }
    fortran
}

fn compare_netcdf(actual_path: &std::path::Path, expected_path: &std::path::Path) {
    let actual = netcdf::open(actual_path).unwrap();
    let expected = netcdf::open(expected_path).unwrap();
    let dimensions = |file: &netcdf::File| {
        file.dimensions()
            .map(|dimension| (dimension.name(), dimension.len()))
            .collect::<Vec<_>>()
    };
    let names = |file: &netcdf::File| {
        file.variables()
            .map(|variable| variable.name())
            .collect::<Vec<_>>()
    };
    assert_eq!(dimensions(&actual), dimensions(&expected));
    assert_eq!(names(&actual), names(&expected));
    for name in names(&expected) {
        let actual_var = actual.variable(&name).unwrap();
        let expected_var = expected.variable(&name).unwrap();
        assert_eq!(
            dimension_names(&actual_var),
            dimension_names(&expected_var),
            "dimension mismatch for {name}"
        );
        if name == "nyrs_crop_active_p" {
            assert_eq!(
                actual_var.get_values::<i32, _>(..).unwrap(),
                expected_var.get_values::<i32, _>(..).unwrap(),
                "value mismatch for {name}"
            );
        } else {
            assert_eq!(
                actual_var.get_values::<f64, _>(..).unwrap(),
                expected_var.get_values::<f64, _>(..).unwrap(),
                "value mismatch for {name}"
            );
        }
    }
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
