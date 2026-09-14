#[test]
#[ignore = "requires the locally generated CN-Cng surface artifact"]
fn native_mkinidata_binary_writes_the_complete_common_restart_family() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let source = root.join("oracle/work/generated/out/CN-Cng/landdata/srfdata.nc");
    assert!(source.is_file(), "missing {}", source.display());
    let directory =
        std::env::temp_dir().join(format!("colm-init-native-binary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let output = directory.join("out");
    let landdata = output.join("CN-Cng/landdata");
    std::fs::create_dir_all(&landdata).unwrap();
    std::fs::copy(source, landdata.join("srfdata.nc")).unwrap();
    let namelist = std::fs::read_to_string(root.join("oracle/work/generated/case.nml"))
        .unwrap()
        .replace(
            &format!(
                "DEF_dir_output = '{}/oracle/work/generated/out/'",
                root.display()
            ),
            &format!("DEF_dir_output = '{}/'", output.display()),
        );
    let mut namelist = colm_namelist::parse(&namelist).unwrap();
    for (name, text) in [("DEF_TUNING_ZLND", "0.025"), ("DEF_TUNING_CAPR", "0.42")] {
        namelist
            .insert(
                name,
                colm_namelist::Value::Real { text: text.into() },
                "nl_colm",
            )
            .unwrap();
    }
    let case = directory.join("case.nml");
    std::fs::write(&case, namelist.to_string()).unwrap();

    let result = std::process::Command::new(env!("CARGO_BIN_EXE_mkinidata-rs"))
        .args([case.as_os_str(), "--land-cover".as_ref(), "igbp".as_ref()])
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "mkinidata-rs failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let restart = output.join("CN-Cng/restart");
    for path in [
        restart.join("const/CN-Cng_restart_const_lc2005.nc"),
        restart.join("const/CN-Cng_restart_const_lc2005_w180_s90.nc"),
        restart.join("2008-001-00000/CN-Cng_restart_2008-001-00000_lc2005_w180_s90.nc"),
    ] {
        assert!(path.is_file(), "missing {}", path.display());
    }
    let file = netcdf::open(restart.join("const/CN-Cng_restart_const_lc2005.nc")).unwrap();
    for (name, value) in [("zlnd", 0.025), ("capr", 0.42)] {
        assert_eq!(
            file.variable(name)
                .unwrap()
                .get_value::<f64, _>(())
                .unwrap(),
            value
        );
    }
    file.close().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
