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
