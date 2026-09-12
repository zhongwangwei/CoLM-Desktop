use std::path::PathBuf;

use super::*;

#[test]
fn static_config_uses_the_upstream_namelist_defaults() {
    assert_eq!(RestartTuning::default().zlnd, 0.01);
    assert_eq!(RestartTuning::default().wetwatmax, 200.0);
    assert_eq!(patch_type(LandCoverScheme::Igbp, 10).unwrap(), 0);
    assert_eq!(patch_type(LandCoverScheme::Igbp, 11).unwrap(), 2);
    assert_eq!(patch_type(LandCoverScheme::Usgs, 16).unwrap(), 4);
    assert!(patch_type(LandCoverScheme::Igbp, 0).is_err());
}

#[test]
fn namelist_static_run_uses_colm_paths_defaults_and_surface_contract() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-namelist-static-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.add_variable::<i32>("USGS_classification", &[])
        .unwrap()
        .put_values(&[19], ..)
        .unwrap();
    file.close().unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'CN-Cng'\n DEF_dir_output = '{}'\n /\n",
            directory.join("output").display()
        ),
    )
    .unwrap();

    assert!(single_point_static_run_from_namelist(&namelist, None, None).is_err());
    let run = single_point_static_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
        .unwrap();
    assert_eq!(run.surface, surface);
    assert_eq!(run.restart_dir, directory.join("output/CN-Cng/restart"));
    assert_eq!(run.case_name, "CN-Cng");
    assert_eq!(run.land_cover_year, 2005);
    assert_eq!(run.block_label, "w180_s90");
    assert_eq!(run.land_cover, LandCoverScheme::Igbp);
    assert_eq!(run.hydraulic_model, HydraulicModel::VanGenuchten);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pc_subgrid_is_resolved_exclusively_and_uses_fortran_canopy_layers() {
    let document = parse(
        "&nl_colm
 DEF_USE_LCT = .false.
 DEF_USE_PFT = .false.
 DEF_USE_PC = .true.
 /
",
    )
    .unwrap();
    assert_eq!(
        single_point_subgrid(&document).unwrap(),
        SinglePointSubgrid::Pc
    );
    assert_eq!(pc_canopy_layer(1).unwrap(), 2);
    assert_eq!(pc_canopy_layer(8).unwrap(), 2);
    assert_eq!(pc_canopy_layer(9).unwrap(), 1);
    assert_eq!(pc_canopy_layer(15).unwrap(), 1);
    assert!(pc_canopy_layer(16).is_err());
    assert!(single_point_subgrid(
        &parse(
            "&nl_colm
 DEF_USE_LCT=.true.
 DEF_USE_PC=.true.
 /
"
        )
        .unwrap()
    )
    .is_err());
}

#[test]
fn pft_optics_honor_the_native_indexed_namelist_override() {
    let document = parse("&nl_colm\n DEF_PFT_CHIL(2) = 0.25\n /\n").unwrap();
    let optics = pft_leaf_optics(&document, 1, HydraulicModel::VanGenuchten).unwrap();
    assert_eq!(optics.chil, 0.25);
    assert_eq!(optics.reflectance[0][0], 0.07);
    assert!(pft_leaf_optics(
        &parse("&nl_colm\n DEF_PFT_CHIL(2) = 1.1\n /\n").unwrap(),
        1,
        HydraulicModel::VanGenuchten,
    )
    .is_err());
}

#[test]
fn cold_namelist_accepts_snicar_and_keeps_existing_state_sources() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-namelist-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let soil = directory.join("soilstate.nc");
    let snow = directory.join("snowstate.nc");
    let wtd = directory.join("wtd.nc");
    std::fs::write(&soil, []).unwrap();
    std::fs::write(&snow, []).unwrap();
    std::fs::write(&wtd, []).unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_USE_SNICAR=.true.\n DEF_USE_SoilInit=.true.\n DEF_file_SoilInit='{}'\n DEF_USE_SnowInit=.true.\n DEF_file_SnowInit='{}'\n DEF_TUNING_SNOW_COVER_EXPONENT=.75\n DEF_USE_WaterTableInit=.true.\n DEF_file_WaterTable='{}'\n /\n",
            directory.join("output").display(),
            soil.display(),
            snow.display(),
            wtd.display(),
        ),
    )
    .unwrap();
    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.soil_initial_state, Some(soil));
    assert_eq!(run.snow_initial_state, Some(snow));
    assert_eq!(run.water_table_initial_state, Some(wtd));
    assert!(run.variably_saturated_flow);
    assert_eq!(run.snow_cover_exponent, 0.75);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cold_namelist_uses_start_year_for_lulcc_restarts() {
    let directory =
        std::env::temp_dir().join(format!("colm-init-lulcc-start-year-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let surface = directory.join("output/CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    let mut file = netcdf::create(&surface).unwrap();
    file.add_variable::<i32>("IGBP_classification", &[])
        .unwrap()
        .put_values(&[10], ..)
        .unwrap();
    file.close().unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\n DEF_CASE_NAME='CN-Cng'\n DEF_dir_output='{}'\n DEF_LC_YEAR=2005\n DEF_USE_LULCC=.true.\n DEF_simulation_time%start_year=2008\n /\n",
            directory.join("output").display(),
        ),
    )
    .unwrap();

    let run = single_point_cold_start_run_from_namelist(&namelist, None, None).unwrap();
    assert_eq!(run.static_run.land_cover_year, 2008);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the locally generated CN-Cng upstream single-point restart artifact"]
fn native_single_point_static_restart_matches_the_upstream_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let surface = root.join("oracle/work/generated/out/CN-Cng/landdata/srfdata.nc");
    let reference = root.join(
        "oracle/work/generated/out/CN-Cng/restart/const/CN-Cng_restart_const_lc2005_w180_s90.nc",
    );
    let directory = std::env::temp_dir().join(format!(
        "colm-init-single-point-static-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let output = write_single_point_constant_restart(
        surface,
        directory.join("restart"),
        SinglePointStaticConfig::new(
            "CN-Cng",
            2005,
            "w180_s90",
            LandCoverScheme::Igbp,
            HydraulicModel::VanGenuchten,
        ),
    )
    .unwrap();
    let expected = netcdf::open(reference).unwrap();
    let actual = netcdf::open(output.block).unwrap();
    assert_eq!(variable_names(&actual), variable_names(&expected));
    assert_eq!(dimension_lengths(&actual), dimension_lengths(&expected));
    for name in F64_FIELDS {
        let expected = values_f64(&expected, name);
        let actual = values_f64(&actual, name);
        assert_eq!(actual.len(), expected.len(), "{name}");
        for (index, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
            let tolerance = 1.0e-10 * expected.abs().max(1.0);
            assert!(
                (actual - expected).abs() <= tolerance,
                "{name}[{index}]: got {actual:.17e}, expected {expected:.17e}"
            );
        }
    }
    for name in ["patchclass", "patchtype", "soiltext"] {
        assert_eq!(
            values_i32(&actual, name),
            values_i32(&expected, name),
            "{name}"
        );
    }
    assert_eq!(
        values_i8(&actual, "patchmask"),
        values_i8(&expected, "patchmask")
    );

    let expected_tuning = netcdf::open(
        root.join("oracle/work/generated/out/CN-Cng/restart/const/CN-Cng_restart_const_lc2005.nc"),
    )
    .unwrap();
    let actual_tuning = netcdf::open(output.constants).unwrap();
    for name in TUNING_FIELDS {
        assert_eq!(
            values_f64(&actual_tuning, name),
            values_f64(&expected_tuning, name)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires the locally generated CN-Cng upstream single-point restart artifact"]
fn native_single_point_cold_time_restart_matches_the_upstream_reference() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reference = root.join(
        "oracle/work/generated/out/CN-Cng/restart/2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc",
    );
    let directory = std::env::temp_dir().join(format!(
        "colm-init-single-point-cold-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let output = directory.join("output");
    let surface = output.join("CN-Cng/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    std::fs::copy(
        root.join("oracle/work/generated/out/CN-Cng/landdata/srfdata.nc"),
        &surface,
    )
    .unwrap();
    let namelist = directory.join("case.nml");
    std::fs::write(
        &namelist,
        format!(
            "&nl_colm\nDEF_CASE_NAME='CN-Cng'\nDEF_dir_output='{}'\nDEF_simulation_time%greenwich=.false.\nDEF_simulation_time%start_year=2008\nDEF_USE_OZONESTRESS=.false.\n/\n",
            output.display()
        ),
    )
    .unwrap();

    let run =
        single_point_cold_start_run_from_namelist(&namelist, Some(LandCoverScheme::Igbp), None)
            .unwrap();
    let actual_path = write_single_point_cold_time_restart(&run).unwrap().block;
    let expected = netcdf::open(reference).unwrap();
    let actual = netcdf::open(actual_path).unwrap();
    assert_eq!(
        ordered_dimension_lengths(&actual),
        ordered_dimension_lengths(&expected)
    );
    assert_eq!(
        ordered_variable_names(&actual),
        ordered_variable_names(&expected)
    );
    for name in ordered_variable_names(&expected) {
        let expected = values_f64(&expected, &name);
        let actual = values_f64(&actual, &name);
        assert_eq!(actual.len(), expected.len(), "{name}");
        for (index, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
            let tolerance = 1.0e-10 * expected.abs().max(1.0);
            assert!(
                (actual - expected).abs() <= tolerance,
                "{name}[{index}]: got {actual:.17e}, expected {expected:.17e}"
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

const F64_FIELDS: &[&str] = &[
    "patchlonr",
    "patchlatr",
    "lakedepth",
    "dz_lake",
    "soil_s_v_alb",
    "soil_d_v_alb",
    "soil_s_n_alb",
    "soil_d_n_alb",
    "vf_quartz",
    "vf_gravels",
    "vf_om",
    "vf_sand",
    "vf_clay",
    "wf_gravels",
    "wf_sand",
    "wf_clay",
    "wf_om",
    "OM_density",
    "BD_all",
    "wfc",
    "porsl",
    "psi0",
    "bsw",
    "theta_r",
    "BVIC",
    "alpha_vgm",
    "L_vgm",
    "n_vgm",
    "sc_vgm",
    "fc_vgm",
    "vic_b_infilt",
    "vic_Dsmax",
    "vic_Ds",
    "vic_Ws",
    "vic_c",
    "hksati",
    "csol",
    "k_solids",
    "dksatu",
    "dksatf",
    "dkdry",
    "BA_alpha",
    "BA_beta",
    "htop",
    "hbot",
    "elvmean",
    "elvstd",
    "slpratio",
];
const TUNING_FIELDS: &[&str] = &[
    "zlnd",
    "zsno",
    "csoilc",
    "dewmx",
    "capr",
    "cnfac",
    "ssi",
    "wimp",
    "pondmx",
    "smpmax",
    "smpmin",
    "smpmax_hr",
    "smpmin_hr",
    "trsmx0",
    "tcrit",
    "wetwatmax",
];

fn variable_names(file: &netcdf::File) -> Vec<String> {
    file.variables().map(|variable| variable.name()).collect()
}

fn dimension_lengths(file: &netcdf::File) -> Vec<(String, usize)> {
    file.dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect()
}

fn ordered_dimension_lengths(file: &netcdf::File) -> Vec<(String, usize)> {
    file.dimensions()
        .map(|dimension| (dimension.name(), dimension.len()))
        .collect()
}

fn ordered_variable_names(file: &netcdf::File) -> Vec<String> {
    file.variables().map(|variable| variable.name()).collect()
}

fn values_f64(file: &netcdf::File, name: &str) -> Vec<f64> {
    file.variable(name)
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap()
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
