use super::*;

fn hyperspectral_kernel() -> Kernel {
    Kernel {
        dir: PathBuf::from("."),
        manifest: colm_kernel::Manifest {
            schema: colm_kernel::manifest::SCHEMA,
            preset: "test".into(),
            platform: "test".into(),
            colm_git_sha: "test".into(),
            generator_args: String::new(),
            build_profile: String::new(),
            macros: vec!["HYPERSPECTRAL".into()],
            built_with: String::new(),
            netcdf_c: String::new(),
            netcdf_fortran: String::new(),
            hdf5: String::new(),
            sha256: std::collections::BTreeMap::new(),
        },
    }
}

fn hyperspectral_pft_namelist(root: &Path) -> PathBuf {
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\nDEF_file_mesh='mesh.nc'\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_PC=.false.\n/\n",
    )
    .unwrap();
    namelist
}

fn test_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("colm-cli-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn only_the_three_real_kernel_stages_are_accepted() {
    assert_eq!(requested_run_stage(None).unwrap(), None);
    assert_eq!(
        requested_run_stage(Some("mksrfdata")).unwrap(),
        Some(Stage::MkSrfData)
    );
    assert_eq!(
        requested_run_stage(Some("mkinidata")).unwrap(),
        Some(Stage::MkIniData)
    );
    assert_eq!(
        requested_run_stage(Some("colm")).unwrap(),
        Some(Stage::Colm)
    );
    assert!(requested_run_stage(Some("all")).is_err());
}

#[test]
fn rust_preprocessors_are_the_default_and_fortran_is_an_explicit_fallback() {
    assert_eq!(requested_preprocessors(None).unwrap(), PreprocessorMode::Rust);
    assert_eq!(
        requested_preprocessors(Some("fortran")).unwrap(),
        PreprocessorMode::Fortran
    );
    assert!(requested_preprocessors(Some("other")).is_err());
}

#[test]
fn hyperspectral_pft_sidecars_receive_validated_optical_sources() {
    let root = test_directory("hyperspectral-args");
    let params = root.join("params");
    std::fs::create_dir_all(params.join("fsds")).unwrap();
    std::fs::create_dir_all(params.join("leaf_optical_properties")).unwrap();
    std::fs::write(params.join("fsds/swnb_480bnd_fsds.nc"), "radiation").unwrap();
    std::fs::write(
        params.join("leaf_optical_properties/colm_PFT_params.nc"),
        "leaf",
    )
    .unwrap();
    std::fs::write(params.join("water_params.txt"), "water").unwrap();
    let soil = root.join("soil");
    std::fs::create_dir(&soil).unwrap();
    let namelist = hyperspectral_pft_namelist(&root);
    let kernel = hyperspectral_kernel();

    let mksrfdata =
        rust_preprocessor_arguments(Stage::MkSrfData, &namelist, &kernel, None, Some(&soil))
            .unwrap();
    assert_eq!(
        mksrfdata,
        vec![
            "--soil-hyper-albedo-dir".to_owned(),
            soil.canonicalize().unwrap().to_string_lossy().into_owned(),
        ]
    );
    let mkinidata =
        rust_preprocessor_arguments(Stage::MkIniData, &namelist, &kernel, Some(&params), None)
            .unwrap();
    assert_eq!(
        mkinidata,
        vec![
            "--hyperspectral".to_owned(),
            "--highres-radiation".to_owned(),
            params
                .join("fsds/swnb_480bnd_fsds.nc")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--highres-leaf-optics".to_owned(),
            params
                .join("leaf_optical_properties/colm_PFT_params.nc")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--highres-water-optics".to_owned(),
            params
                .join("water_params.txt")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hyperspectral_optical_sources_are_required_and_fingerprinted() {
    let root = test_directory("hyperspectral-fingerprint");
    let namelist = hyperspectral_pft_namelist(&root);
    let kernel = hyperspectral_kernel();
    let error = rust_preprocessor_arguments(Stage::MkIniData, &namelist, &kernel, None, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("--highres-params"), "{error}");

    let source = root.join("water_params.txt");
    std::fs::write(&source, "first").unwrap();
    let first = rust_preprocessor_input_identity(&[
        "--highres-water-optics".into(),
        source.to_string_lossy().into_owned(),
    ])
    .unwrap();
    std::fs::write(&source, "second").unwrap();
    let second = rust_preprocessor_input_identity(&[
        "--highres-water-optics".into(),
        source.to_string_lossy().into_owned(),
    ])
    .unwrap();
    assert_ne!(first, second);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_changed_hyperspectral_surface_source_invalidates_downstream_stages() {
    assert_eq!(
        stage_preprocessor_input_identity(Stage::MkSrfData, Some("surface"), Some("initial")),
        Some("surface".into())
    );
    for stage in [Stage::MkIniData, Stage::Colm] {
        assert_eq!(
            stage_preprocessor_input_identity(stage, Some("surface"), Some("initial")),
            Some("surface;initial".into())
        );
    }
}
