//! Native Rust preprocessing integration test against the checked-in CN-Cng case.

use colm_init::{
    prepare_single_point_case, write_single_point_hyperspectral_cold_time_restarts,
    write_single_point_hyperspectral_constant_restarts, SinglePointHyperspectralConfig,
    SinglePointPreprocessFiles,
};
use colm_srfdata::SiteMode;

// Keep each NetCDF writer + external Fortran reader lifecycle serial, as in
// the production stage runner. Concurrent tests can make existing HDF5 block
// files transiently unopenable in the child (reported as "variable not found").
static NATIVE_PIPELINE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
#[ignore = "requires the locally generated CN-Cng source case"]
fn native_surface_and_cold_start_write_the_common_restart_family() {
    let _guard = NATIVE_PIPELINE_LOCK.lock().unwrap();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let directory = std::env::temp_dir().join(format!(
        "colm-native-preprocess-pipeline-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("out");
    let template = root.join("oracle/work/generated/case.nml");
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let namelist = std::fs::read_to_string(&template).unwrap().replace(
        &format!("DEF_dir_output = '{original_output}'"),
        &format!("DEF_dir_output = '{}/'", output.display()),
    );
    assert!(
        !namelist.contains(&original_output),
        "test case did not redirect DEF_dir_output"
    );
    let case = directory.join("case.nml");
    std::fs::write(&case, namelist).unwrap();

    let (_, files) = prepare_single_point_case(&case, Some(SiteMode::Igbp), false, None).unwrap();

    let restart = output.join("CN-Cng/restart");
    for path in [
        files.surface,
        restart.join("const/CN-Cng_restart_const_lc2005.nc"),
        restart.join("const/CN-Cng_restart_const_lc2005_w180_s90.nc"),
        restart.join("2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc"),
    ] {
        assert!(path.is_file(), "missing {}", path.display());
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires local default kernel, generated CN-Cng case, and PLUMBER2 forcing"]
fn rust_preprocess_restart_runs_in_the_unchanged_fortran_runtime() {
    rust_preprocess_runs_in_fortran_runtime("common", Some(SiteMode::Igbp), "default", "");
}

#[test]
#[ignore = "requires local default/BGC kernels, CoLMruntime SNICAR tables, CN-Cng case and forcing"]
fn snicar_single_point_preprocessing_runs_in_unchanged_fortran_runtime() {
    for (label, land_cover, kernel, mode) in [
        ("snicar-lct", Some(SiteMode::Igbp), "default", ""),
        (
            "snicar-pft",
            None,
            "bgc",
            "DEF_USE_LCT=.false.\nDEF_USE_PFT=.true.",
        ),
        (
            "snicar-pc",
            None,
            "bgc",
            "DEF_USE_LCT=.false.\nDEF_USE_PC=.true.",
        ),
    ] {
        rust_preprocess_runs_in_fortran_runtime(
            label,
            land_cover,
            kernel,
            &format!("{mode}\nDEF_USE_SNICAR=.true.\nDEF_Aerosol_Readin=.false."),
        );
    }
}

#[test]
#[ignore = "requires local default kernel, generated CN-Cng case, and PLUMBER2 forcing"]
fn inactive_runoff_preprocessors_run_in_the_unchanged_fortran_runtime() {
    let vic = std::env::temp_dir().join(format!(
        "colm-inactive-runoff-vic-{}.txt",
        std::process::id()
    ));
    std::fs::write(&vic, "VIC parameters\n0.3 1.5 0.2 0.8 2.0\n").unwrap();
    for scheme in 0..=2 {
        rust_preprocess_runs_in_fortran_runtime(
            &format!("runoff{scheme}"),
            Some(SiteMode::Igbp),
            "default",
            &format!(
                "DEF_Runoff_SCHEME = {scheme}\nDEF_file_VIC_para = '{}'",
                vic.display()
            ),
        );
    }
    std::fs::remove_file(vic).unwrap();
}

#[test]
#[ignore = "requires local BGC kernel, CoLMruntime, generated CN-Cng case, and PLUMBER2 forcing"]
fn rust_pft_bgc_preprocess_restart_runs_in_the_unchanged_fortran_runtime() {
    rust_preprocess_runs_in_fortran_runtime(
        "pft-bgc",
        None,
        "bgc",
        "DEF_USE_LCT = .false.\nDEF_USE_PFT = .true.\nDEF_USE_BGC = .true.\nDEF_USE_CN_INIT = .true.",
    );
}

#[test]
#[ignore = "requires local BGC kernel, CoLMruntime, generated CN-Cng case, and PLUMBER2 forcing"]
fn rust_pc_bgc_preprocess_restart_runs_in_the_unchanged_fortran_runtime() {
    rust_preprocess_runs_in_fortran_runtime(
        "pc-bgc",
        None,
        "bgc",
        "DEF_USE_LCT = .false.\nDEF_USE_PC = .true.\nDEF_USE_BGC = .true.\nDEF_USE_CN_INIT = .true.",
    );
}

#[test]
#[ignore = "requires the locally generated CN-Cng source case"]
fn native_pft_pc_nonnatural_singlepoint_pipeline_writes_pftless_restarts() {
    let _guard = NATIVE_PIPELINE_LOCK.lock().unwrap();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let template = root.join("oracle/work/generated/case.nml");
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let cases = [(11, 2), (13, 1), (15, 3), (17, 4)];

    for (mode, use_pft, use_pc) in [("pft", true, false), ("pc", false, true)] {
        for bgc in [false, true] {
            for (class, kind) in cases {
                let directory = std::env::temp_dir().join(format!(
                    "colm-native-pftless-{mode}-bgc{bgc}-igbp{class}-{}",
                    std::process::id()
                ));
                let _ = std::fs::remove_dir_all(&directory);
                std::fs::create_dir_all(&directory).unwrap();
                let output = directory.join("out");
                let namelist = std::fs::read_to_string(&template).unwrap().replace(
                    &format!("DEF_dir_output = '{original_output}'"),
                    &format!("DEF_dir_output = '{}/'", output.display()),
                );
                let mut document = colm_namelist::parse(&namelist).unwrap();
                for (field, value) in [
                    ("DEF_USE_LCT", false),
                    ("DEF_USE_PFT", use_pft),
                    ("DEF_USE_PC", use_pc),
                    ("DEF_USE_BGC", bgc),
                    ("DEF_USE_CN_INIT", false),
                    ("USE_SITE_landtype", false),
                ] {
                    document
                        .insert(field, colm_namelist::Value::Bool(value), "nl_colm")
                        .unwrap();
                }
                document
                    .insert(
                        "SITE_landtype",
                        colm_namelist::Value::Int(i64::from(class)),
                        "nl_colm",
                    )
                    .unwrap();
                let case = directory.join("case.nml");
                std::fs::write(&case, document.to_string()).unwrap();

                let (run, files) = prepare_single_point_case(&case, None, false, None).unwrap();
                let surface = netcdf::open(&files.surface).unwrap();
                assert_eq!(values_i32(&surface, "IGBP_classification"), [class]);
                assert!(surface.dimension("pft").is_none(), "{mode} IGBP {class}");
                for name in [
                    "pfttyp",
                    "pctpfts",
                    "canopy_height_pfts",
                    "LAI_pfts_monthly",
                    "SAI_pfts_monthly",
                ] {
                    assert!(
                        surface.variable(name).is_none(),
                        "{mode} IGBP {class} must omit {name}"
                    );
                }
                assert!(surface.variable("LAI_monthly").is_some());
                assert!(surface.variable("SAI_monthly").is_some());
                drop(surface);

                let common_const = netcdf::open(&files.constants.common.block).unwrap();
                assert_eq!(values_i32(&common_const, "patchclass"), [class]);
                assert_eq!(values_i32(&common_const, "patchtype"), [kind]);
                assert_eq!(values_i8(&common_const, "patchmask"), [1]);
                drop(common_const);

                let pft_const_path = files
                    .constants
                    .pft
                    .as_ref()
                    .expect("PFT/PC nonnatural constants still write an empty PFT block");
                let pft_const = netcdf::open(pft_const_path).unwrap();
                assert_eq!(pft_const.dimension("pft").unwrap().len(), 0);
                for name in ["pftclass", "pftfrac", "htop_p", "hbot_p"] {
                    assert_eq!(
                        pft_const.variable(name).unwrap().len(),
                        0,
                        "{mode} IGBP {class} {name}"
                    );
                }
                drop(pft_const);

                assert!(
                    files.time.pft.is_none(),
                    "PFT/PC nonnatural time restart must not write dynamic PFT fields"
                );
                assert_eq!(files.constants.bgc.is_some(), bgc);
                assert_eq!(files.time.bgc.is_some(), bgc);
                let common_time = netcdf::open(&files.time.common.block).unwrap();
                for name in ["tlai", "tsai", "lai", "sai"] {
                    assert_eq!(values_f64(&common_time, name).len(), 1, "{name}");
                }

                append_test_soil_hyper_albedo(&files.surface);
                let highres_restart = directory.join("highres-restart");
                let mut highres_run = run.cold_start.clone();
                highres_run.static_run.restart_dir = highres_restart.clone();
                let missing = directory.join("does-not-exist.nc");
                let time_error = write_single_point_hyperspectral_cold_time_restarts(
                    &highres_run,
                    SinglePointHyperspectralConfig {
                        leaf_optics: Some(&missing),
                        water_optics: Some(&missing),
                        radiation: Some(&missing),
                        urban_albedo: &missing,
                    },
                )
                .unwrap_err()
                .to_string();
                assert!(
                    time_error.contains("urban") || time_error.contains("radiation"),
                    "highres must read its actual spectral inputs: {time_error:#}"
                );
                assert!(
                    !highres_restart.exists(),
                    "missing tables must fail before writing"
                );
                let constants = write_single_point_hyperspectral_constant_restarts(&highres_run)
                    .expect("nonnatural highres constants retain the zero-PFT contract");
                assert!(constants.pft.is_some());
                let common = netcdf::open(constants.common.block).unwrap();
                assert_eq!(values_f64(&common, "soil_alb").len(), 211);

                std::fs::remove_dir_all(directory).unwrap();
            }
        }
    }
}

#[test]
#[ignore = "requires the local urban kernel, AU-Preston input data, and CoLMruntime"]
fn rust_urban_preprocess_restart_runs_in_the_unchanged_fortran_runtime() {
    let _guard = NATIVE_PIPELINE_LOCK.lock().unwrap();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let data = std::path::PathBuf::from(
        "/Users/zhongwangwei/Desktop/Data/ex03_site_urban/AU-Preston_data",
    );
    let template = std::path::PathBuf::from(
        "/Users/zhongwangwei/Desktop/Data/ex03_site_urban/Site_AU-Preston.nml",
    );
    let runtime = std::path::PathBuf::from("/Users/zhongwangwei/Desktop/Data/CoLMruntime");
    for path in [
        data.join("AU-Preston_site_v1.nc"),
        data.join("AU-Preston_metforcing_v1.nc"),
        template.clone(),
        runtime.join("urban/LUCY_rawdata.nc"),
    ] {
        assert!(path.is_file(), "missing {}", path.display());
    }
    let colm = root.join("kernels/urban/colm.x");
    assert!(colm.is_file(), "missing {}", colm.display());

    let directory = std::env::temp_dir().join(format!(
        "colm-native-preprocess-urban-runtime-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("out");
    let forcing = directory.join("forcing.nml");
    std::fs::write(
        &forcing,
        std::fs::read_to_string(data.join("AU-Preston.nml"))
            .unwrap()
            .replace(
                "/share/home/dq089/training2026/ex03_site_urban/AU-Preston_data/",
                &format!("{}/", data.display()),
            ),
    )
    .unwrap();
    let mut case = std::fs::read_to_string(&template).unwrap();
    for (source, replacement) in [
        (
            "'/share/home/dq089/training2026/ex03_site_urban/AU-Preston_data/AU-Preston_site_v1.nc'",
            &format!("'{}/AU-Preston_site_v1.nc'", data.display()),
        ),
        (
            "'/share/home/dq013/zhwei/colm/data/CoLMrawdata/'",
            &format!("'{}/rawdata-unused/'", directory.display()),
        ),
        (
            "'/share/home/dq013/zhwei/colm/data/CoLMruntime/'",
            &format!("'{}/'", runtime.display()),
        ),
        (
            "'/share/home/dq089/training2026/ex03_site_urban/'",
            &format!("'{}/'", output.display()),
        ),
        (
            "'/share/home/dq089/training2026/ex03_site_urban/AU-Preston_data/AU-Preston.nml'",
            &format!("'{}'", forcing.display()),
        ),
    ] {
        assert!(case.contains(source), "missing source path {source}");
        case = case.replace(source, replacement);
    }
    for (source, replacement) in [
        (
            "DEF_simulation_time%start_year    = 1992",
            "DEF_simulation_time%start_year = 2003",
        ),
        (
            "DEF_simulation_time%start_month   = 12",
            "DEF_simulation_time%start_month = 8",
        ),
        (
            "DEF_simulation_time%start_day     = 31",
            "DEF_simulation_time%start_day = 12",
        ),
        (
            "DEF_simulation_time%start_sec     = 84600",
            "DEF_simulation_time%start_sec = 0",
        ),
        (
            "DEF_simulation_time%end_year      = 2004",
            "DEF_simulation_time%end_year = 2003",
        ),
        (
            "DEF_simulation_time%end_month     = 11",
            "DEF_simulation_time%end_month = 8",
        ),
        (
            "DEF_simulation_time%end_day       = 28",
            "DEF_simulation_time%end_day = 13",
        ),
        (
            "DEF_simulation_time%end_sec       = 45000",
            "DEF_simulation_time%end_sec = 0",
        ),
    ] {
        assert!(case.contains(source), "missing simulation field {source}");
        case = case.replace(source, replacement);
    }
    let case_path = directory.join("case.nml");
    let mut document = colm_namelist::parse(&case).unwrap();
    for field in ["DEF_URBAN_RUN", "DEF_URBAN_ONLY"] {
        document
            .insert(field, colm_namelist::Value::Bool(true), "nl_colm")
            .unwrap();
    }
    std::fs::write(&case_path, document.to_string()).unwrap();

    let (_, files) = prepare_single_point_case(&case_path, None, false, None).unwrap();
    assert!(files.constants.urban.is_some());
    assert!(files.time.urban.is_some());
    {
        let common = netcdf::open(&files.constants.common.block).unwrap();
        assert_eq!(values_f64(&common, "patchmask"), vec![1.0]);
    }
    let result = std::process::Command::new(&colm)
        .arg(&case_path)
        .current_dir(&directory)
        .output()
        .unwrap();
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.status.success(), "colm failed:\n{log}");
    assert!(
        log.contains("CoLM Execution Completed."),
        "colm failed:\n{log}"
    );
    assert!(
        output
            .join("AU-Preston/history/AU-Preston_hist_2003-08.nc")
            .is_file(),
        "colm completed without its expected history file:\n{log}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn rust_preprocess_runs_in_fortran_runtime(
    label: &str,
    land_cover: Option<SiteMode>,
    kernel: &str,
    additions: &str,
) {
    let _guard = NATIVE_PIPELINE_LOCK.lock().unwrap();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let forcing_dir =
        std::path::PathBuf::from("/Users/zhongwangwei/Desktop/Data/PLUMBER2s/Forcing");
    let runtime = std::path::PathBuf::from("/Users/zhongwangwei/Desktop/Data/CoLMruntime");
    assert!(forcing_dir
        .join("CN-Cng_2008-2009_FLUXNET2015_Met.nc")
        .is_file());
    assert!(runtime.join("cnsteadystate.nc").is_file());
    let colm = root.join(format!("kernels/{kernel}/colm.x"));
    assert!(colm.is_file(), "missing {}", colm.display());

    let directory = std::env::temp_dir().join(format!(
        "colm-native-preprocess-{label}-runtime-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("out");
    let forcing = directory.join("forcing.nml");
    let original_forcing_dir = "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s/Forcing/";
    std::fs::write(
        &forcing,
        std::fs::read_to_string(root.join("oracle/work/generated/forcing.nml"))
            .unwrap()
            .replace(original_forcing_dir, &format!("{}/", forcing_dir.display())),
    )
    .unwrap();
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let original_runtime = format!("{}/oracle/work/generated/runtime_unused/", root.display());
    let original_forcing = format!("{}/oracle/work/generated/forcing.nml", root.display());
    let case = directory.join("case.nml");
    std::fs::write(
        &case,
        std::fs::read_to_string(root.join("oracle/work/generated/case.nml"))
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
                &format!("DEF_forcing_namelist = '{original_forcing}'"),
                &format!("DEF_forcing_namelist = '{}'", forcing.display()),
            )
            .replace(
                "&nl_colm",
                &format!(
                    "&nl_colm\n{additions}\nDEF_file_cn_init = '{}/cnsteadystate.nc'",
                    runtime.display()
                ),
            ),
    )
    .unwrap();

    let (_, files) = prepare_single_point_case(&case, land_cover, false, None).unwrap();
    let document = colm_namelist::parse(&std::fs::read_to_string(&case).unwrap()).unwrap();
    let use_texture = match document.get("DEF_Runoff_SCHEME") {
        Some(colm_namelist::Value::Int(scheme)) => *scheme == 3,
        None => true,
        value => panic!("unexpected runoff scheme: {value:?}"),
    };
    let surface = netcdf::open(&files.surface).unwrap();
    assert_eq!(surface.variable("soil_texture").is_some(), use_texture);
    drop(surface);
    if !use_texture {
        let constants = netcdf::open(&files.constants.common.block).unwrap();
        assert!(values_f64(&constants, "soiltext")
            .iter()
            .all(|&value| value == 0.0));
        assert!(values_f64(&constants, "BVIC")
            .iter()
            .all(|&value| value == 1.0));
    }
    {
        let restart = netcdf::open(&files.time.common.block).unwrap();
        for name in ["alb", "ssun", "ssha", "ssoi", "ssno", "snw_rds", "ssno_lyr"] {
            assert!(
                values_f64(&restart, name).iter().all(|v| v.is_finite()),
                "{label}: nonfinite initial {name}"
            );
        }
    }
    if matches!(
        document.get("DEF_USE_PC"),
        Some(colm_namelist::Value::Bool(true))
    ) {
        assert_pc_common_absorption_matches_pft_outputs(&files);
    }
    let result = std::process::Command::new(&colm)
        .arg(&case)
        .current_dir(&directory)
        .output()
        .unwrap();
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.status.success(), "colm failed:\n{log}");
    assert!(
        log.contains("CoLM Execution Completed."),
        "colm failed:\n{log}"
    );
    assert!(
        output
            .join("CN-Cng/history/CN-Cng_hist_2008-01.nc")
            .is_file(),
        "colm completed without its expected history file:\n{log}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn assert_pc_common_absorption_matches_pft_outputs(files: &SinglePointPreprocessFiles) {
    let pft_const_path = files
        .constants
        .pft
        .as_ref()
        .expect("PC preprocessing should write a PFT constant restart");
    let pft_time_path = files
        .time
        .pft
        .as_ref()
        .expect("PC preprocessing should write a PFT time restart");
    let pft_const = netcdf::open(pft_const_path).unwrap();
    let pft_time = netcdf::open(pft_time_path).unwrap();
    let common = netcdf::open(&files.time.common.block).unwrap();
    let fractions = values_f64(&pft_const, "pftfrac");
    let pfts = fractions.len();
    for (common_name, pft_name) in [("ssun", "ssun_p"), ("ssha", "ssha_p")] {
        let common_values = values_f64(&common, common_name);
        let pft_values = values_f64(&pft_time, pft_name);
        assert_eq!(common_values.len(), 4, "{common_name}");
        assert_eq!(pft_values.len(), pfts * 4, "{pft_name}");
        for rtyp in 0..2 {
            for band in 0..2 {
                let mut expected = 0.0;
                for (pft, &fraction) in fractions.iter().enumerate() {
                    let value = pft_values[(pft * 2 + rtyp) * 2 + band];
                    expected = value.mul_add(fraction, expected);
                }
                let actual = common_values[rtyp * 2 + band];
                assert_eq!(
                    actual.to_bits(),
                    expected.to_bits(),
                    "{common_name}[rtyp={rtyp},band={band}] should be reduced from {pft_name}"
                );
            }
        }
    }
}

fn values_f64(file: &netcdf::File, name: &str) -> Vec<f64> {
    file.variable(name)
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap()
}

fn append_test_soil_hyper_albedo(surface: &std::path::Path) {
    let mut file = netcdf::append(surface).unwrap();
    file.add_dimension("wavelength", 211).unwrap();
    file.add_variable::<f64>("soil_hyper_albedo", &["wavelength"])
        .unwrap()
        .put_values(&[0.2; 211], ..)
        .unwrap();
    file.close().unwrap();
}

fn values_i32(file: &netcdf::File, name: &str) -> Vec<i32> {
    file.variable(name)
        .unwrap()
        .get_values::<i32, _>(..)
        .unwrap()
}

fn values_i8(file: &netcdf::File, name: &str) -> Vec<i8> {
    file.variable(name)
        .unwrap()
        .get_values::<i8, _>(..)
        .unwrap()
}
