//! Native Rust preprocessing integration test against the checked-in CN-Cng case.

use colm_init::prepare_single_point_case;
use colm_srfdata::SiteMode;

#[test]
#[ignore = "requires the locally generated CN-Cng source case"]
fn native_surface_and_cold_start_write_the_common_restart_family() {
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
#[ignore = "requires local BGC kernel, CoLMruntime, generated CN-Cng case, and PLUMBER2 forcing"]
fn rust_pft_bgc_preprocess_restart_runs_in_the_unchanged_fortran_runtime() {
    rust_preprocess_runs_in_fortran_runtime(
        "pft-bgc",
        None,
        "bgc",
        "DEF_USE_LCT = .false.\nDEF_USE_PFT = .true.\nDEF_USE_BGC = .true.\nDEF_USE_CN_INIT = .true.",
    );
}

fn rust_preprocess_runs_in_fortran_runtime(
    label: &str,
    land_cover: Option<SiteMode>,
    kernel: &str,
    additions: &str,
) {
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

    prepare_single_point_case(&case, land_cover, false, None).unwrap();
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
