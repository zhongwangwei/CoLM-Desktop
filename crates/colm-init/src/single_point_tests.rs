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
