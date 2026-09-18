use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn ncdump_header(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("ncdump")
        .arg("-sh")
        .arg(path)
        .output();
    let Ok(output) = output else {
        eprintln!("skipping NetCDF compression metadata check: ncdump not found on PATH");
        return None;
    };
    assert!(
        output.status.success(),
        "ncdump -sh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8(output.stdout).expect("ncdump header is utf8"))
}

fn assert_deflate(path: &std::path::Path, variable: &str, level: u8) {
    if let Some(header) = ncdump_header(path) {
        assert!(
            header.contains(&format!("{variable}:_DeflateLevel = {level} ;")),
            "{variable} in {} did not have deflate level {level}\n{header}",
            path.display()
        );
    }
}

fn assert_no_deflate(path: &std::path::Path, variable: &str) {
    if let Some(header) = ncdump_header(path) {
        assert!(
            !header.contains(&format!("{variable}:_DeflateLevel")),
            "{variable} in {} unexpectedly had deflate metadata\n{header}",
            path.display()
        );
    }
}

#[test]
fn cold_start_bgc_constants_match_fortran_schema_values_and_layout() {
    let root = temp_dir("bgc-constants");
    let files =
        write_cold_start_bgc_constant_restart(&root, "CN-Cng", 2005, "w180_s90", 2, true, 1)
            .unwrap();
    assert_eq!(
        files.constants,
        root.join("const/CN-Cng_restart_bgc_const_lc2005.nc")
    );
    assert_eq!(
        files.block,
        root.join("const/CN-Cng_restart_bgc_const_lc2005_w180_s90.nc")
    );

    let constants = netcdf::open(&files.constants).unwrap();
    assert_eq!(constants.dimension_len("ndecomp_transitions"), Some(10));
    assert_eq!(constants.dimension_len("ndecomp_pools"), Some(7));
    assert_eq!(
        constants
            .variable("donor_pool")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [1, 2, 3, 5, 4, 4, 5, 6, 6, 7]
    );
    assert_eq!(
        constants
            .variable("floating_cn_ratio")
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap(),
        [1, 1, 1, 1, 0, 0, 0]
    );
    assert_eq!(
        constants
            .variable("nfix_timeconst")
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap(),
        10.0
    );
    assert_eq!(
        constants
            .variable("borealat")
            .unwrap()
            .get_value::<f64, _>(())
            .unwrap(),
        40.0 / std::f64::consts::PI
    );

    assert_deflate(&files.block, "rf_decomp", 1);
    let block = netcdf::open(&files.block).unwrap();
    assert_eq!(block.dimension_len("patch"), Some(2));
    assert_eq!(block.dimension_len("soil"), Some(10));
    assert_eq!(
        block
            .variable("rf_decomp")
            .unwrap()
            .dimensions()
            .iter()
            .map(|dimension| dimension.name())
            .collect::<Vec<_>>(),
        ["patch", "ndecomp_transitions", "soil"]
    );
    let rf = block
        .variable("rf_decomp")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(&rf[..10], &[0.55; 10]);
    assert_eq!(&rf[30..40], &[0.51; 10]);
    assert_eq!(
        block
            .variable("rice2pdt")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [-9_999, -9_999]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bgc_constant_restart_compresses_only_block_vectors() {
    let root = temp_dir("bgc-compression");
    let files = write_cold_start_bgc_constant_restart(&root, "case", 2005, "w180_s90", 2, false, 4)
        .unwrap();
    assert_deflate(&files.block, "rf_decomp", 4);
    assert_deflate(&files.block, "pathfrac_decomp", 4);
    assert_deflate(&files.block, "rice2pdt", 4);
    assert_no_deflate(&files.constants, "donor_pool");
    assert_no_deflate(&files.constants, "nfix_timeconst");

    let root0 = temp_dir("bgc-compression-zero");
    let zero = write_cold_start_bgc_constant_restart(&root0, "case", 2005, "w180_s90", 1, false, 0)
        .unwrap();
    assert_no_deflate(&zero.block, "rf_decomp");
    assert_eq!(
        netcdf::open(&zero.block)
            .unwrap()
            .variable("rice2pdt")
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap(),
        [-9_999]
    );

    let invalid = temp_dir("bgc-compression-invalid");
    assert!(write_cold_start_bgc_constant_restart(
        &invalid, "case", 2005, "w180_s90", 1, false, 10,
    )
    .is_err());
    assert!(!invalid.exists());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(root0).unwrap();
}

#[test]
fn bgc_constant_restart_rejects_invalid_output_contract() {
    let root = temp_dir("bgc-invalid");
    assert!(write_cold_start_bgc_constant_restart(
        &root, "bad/name", 2005, "w180_s90", 1, false, 1
    )
    .is_err());
    assert!(
        write_cold_start_bgc_constant_restart(&root, "case", 10_000, "w180_s90", 1, false, 1)
            .is_err()
    );
    assert!(
        write_cold_start_bgc_constant_restart(&root, "case", 2005, "w180_s90", 0, false, 1)
            .is_err()
    );
    assert!(!root.exists());
}

#[test]
#[ignore = "requires the local BGC kernel and CoLMruntime cnsteadystate.nc reference data"]
fn cold_start_bgc_constants_match_the_upstream_fortran_reference() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let runtime = std::path::Path::new("/Volumes/Data01/Data/CoLMruntime");
    assert!(runtime.join("cnsteadystate.nc").is_file());
    let upstream_surface = root.join("kernels/bgc/mksrfdata.x");
    let upstream_init = root.join("kernels/bgc/mkinidata.x");
    assert!(upstream_surface.is_file());
    assert!(upstream_init.is_file());

    let directory = temp_dir("bgc-fortran-reference");
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("out");
    let template = root.join("oracle/work/generated/case.nml");
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let original_runtime = format!("{}/oracle/work/generated/runtime_unused/", root.display());
    let namelist = std::fs::read_to_string(&template)
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

    let native = write_cold_start_bgc_constant_restart(
        directory.join("native/restart"),
        "CN-Cng",
        2005,
        "w180_s90",
        1,
        true,
        1,
    )
    .unwrap();
    let upstream = output.join("CN-Cng/restart/const");
    compare_netcdf(
        &native.constants,
        &upstream.join("CN-Cng_restart_bgc_const_lc2005.nc"),
    );
    compare_netcdf(
        &native.block,
        &upstream.join("CN-Cng_restart_bgc_const_lc2005_w180_s90.nc"),
    );
    std::fs::remove_dir_all(directory).unwrap();
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
            actual_var
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect::<Vec<_>>(),
            expected_var
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect::<Vec<_>>(),
            "dimension mismatch for {name}"
        );
        if matches!(name.as_str(), "donor_pool" | "receiver_pool" | "rice2pdt")
            || name.starts_with("i_")
        {
            assert_eq!(
                actual_var.get_values::<i32, _>(..).unwrap(),
                expected_var.get_values::<i32, _>(..).unwrap(),
                "value mismatch for {name}"
            );
        } else if matches!(
            name.as_str(),
            "floating_cn_ratio" | "is_cwd" | "is_litter" | "is_soil"
        ) {
            assert_eq!(
                actual_var.get_values::<i8, _>(..).unwrap(),
                expected_var.get_values::<i8, _>(..).unwrap(),
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

fn temp_dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "colm-init-{name}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}
