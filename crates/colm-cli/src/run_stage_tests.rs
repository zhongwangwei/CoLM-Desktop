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

fn gridriver_kernel() -> Kernel {
    let mut kernel = hyperspectral_kernel();
    kernel.manifest.macros = vec!["GridRiverLakeFlow".into(), "LULC_IGBP".into()];
    kernel
}

fn catch_lateral_kernel() -> Kernel {
    let mut kernel = hyperspectral_kernel();
    kernel.manifest.macros = vec!["CatchLateralFlow".into(), "LULC_IGBP".into()];
    kernel
}

fn data_assimilation_kernel() -> Kernel {
    let mut kernel = hyperspectral_kernel();
    kernel.manifest.macros = vec!["DataAssimilation".into(), "LULC_IGBP".into()];
    kernel
}

fn pc_kernel() -> Kernel {
    let mut kernel = hyperspectral_kernel();
    kernel.manifest.macros = vec!["LULC_IGBP_PC".into()];
    kernel
}

fn hyperspectral_pft_namelist(root: &Path) -> PathBuf {
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        "&nl_colm\nDEF_file_mesh='mesh.nc'\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_PC=.false.\nDEF_HighResUrban_albedo='urban_albedo.nc'\n/\n",
    )
    .unwrap();
    std::fs::write(root.join("urban_albedo.nc"), "urban").unwrap();
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
    assert_eq!(
        requested_preprocessors(None).unwrap(),
        PreprocessorMode::Rust
    );
    assert_eq!(
        requested_preprocessors(Some("fortran")).unwrap(),
        PreprocessorMode::Fortran
    );
    assert!(requested_preprocessors(Some("other")).is_err());
}

#[test]
fn gridriver_kernel_enables_the_matching_rust_mkinidata_branch() {
    let root = test_directory("gridriver-args");
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\nDEF_USE_LCT=.true.\n/\n").unwrap();
    assert_eq!(
        rust_preprocessor_arguments(Stage::MkIniData, &namelist, &gridriver_kernel(), None, None)
            .unwrap(),
        vec!["--grid-river", "--land-cover", "igbp"]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn catch_lateral_kernel_enables_the_matching_rust_mkinidata_branch() {
    let root = test_directory("catch-lateral-args");
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\nDEF_USE_LCT=.true.\n/\n").unwrap();
    assert_eq!(
        rust_preprocessor_arguments(
            Stage::MkIniData,
            &namelist,
            &catch_lateral_kernel(),
            None,
            None,
        )
        .unwrap(),
        vec!["--catch-lateral", "--land-cover", "igbp"]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn data_assimilation_kernel_enables_the_matching_rust_mkinidata_branch() {
    let root = test_directory("data-assimilation-args");
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\nDEF_USE_LCT=.true.\n/\n").unwrap();
    assert_eq!(
        rust_preprocessor_arguments(
            Stage::MkIniData,
            &namelist,
            &data_assimilation_kernel(),
            None,
            None,
        )
        .unwrap(),
        vec!["--data-assimilation", "--land-cover", "igbp"]
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// 旧 namelist 没写 `DEF_USE_LCT/PFT/PC`，而内核是 PFT/PC 时，预处理器不能
/// 「跟着内核走」：`colm` 只读 namelist，会把沉默解析成 LCT，于是 PC/PFT 的
/// 地表与 restart 被喂给 LCT 运行时 —— 跑得完、结果错。拒绝并点名要补哪个键。
#[test]
fn legacy_namelist_with_a_pc_kernel_is_refused_by_both_rust_preprocessors() {
    let root = test_directory("legacy-pc-subgrid");
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\nDEF_file_mesh='mesh.nc'\n/\n").unwrap();
    for stage in [Stage::MkSrfData, Stage::MkIniData] {
        let error = rust_preprocessor_arguments(stage, &namelist, &pc_kernel(), None, None)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("DEF_USE_LCT, DEF_USE_PFT or DEF_USE_PC"),
            "{error}"
        );
        assert!(error.contains("LCT"), "{error}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 沉默 namelist + LCT 内核（上游默认就是 LCT）无需声明，也不该下发任何
/// subgrid 参数；算例自己声明了 PC 同样放行。
#[test]
fn silent_lct_kernel_and_declared_namelists_need_no_subgrid_argument() {
    let root = test_directory("declared-subgrid");
    let namelist = root.join("case.nml");
    let no_subgrid = |stage: Stage, kernel: &Kernel| {
        let arguments = rust_preprocessor_arguments(stage, &namelist, kernel, None, None).unwrap();
        assert!(
            !arguments.iter().any(|argument| argument == "--subgrid"),
            "{arguments:?}"
        );
    };

    std::fs::write(&namelist, "&nl_colm\nDEF_file_mesh='mesh.nc'\n/\n").unwrap();
    for stage in [Stage::MkSrfData, Stage::MkIniData] {
        no_subgrid(stage, &data_assimilation_kernel());
    }

    std::fs::write(
        &namelist,
        "&nl_colm\nDEF_file_mesh='mesh.nc'\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.false.\nDEF_USE_PC=.true.\n/\n",
    )
    .unwrap();
    for stage in [Stage::MkSrfData, Stage::MkIniData] {
        no_subgrid(stage, &pc_kernel());
    }
    std::fs::remove_dir_all(root).unwrap();
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
            "--highres-urban-albedo".to_owned(),
            root.join("urban_albedo.nc")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
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
fn hyperspectral_single_point_pft_uses_the_same_rust_sidecars() {
    let root = test_directory("hyperspectral-single-point");
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
    let namelist = root.join("case.nml");
    std::fs::write(
        &namelist,
        [
            "&nl_colm",
            "DEF_USE_LCT=.false.",
            "DEF_USE_PFT=.true.",
            "DEF_USE_PC=.false.",
            "DEF_HighResUrban_albedo='urban_albedo.nc'",
            "/",
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(root.join("urban_albedo.nc"), "urban").unwrap();

    let mksrfdata = rust_preprocessor_arguments(
        Stage::MkSrfData,
        &namelist,
        &hyperspectral_kernel(),
        None,
        Some(&soil),
    )
    .unwrap();
    assert_eq!(mksrfdata[0], "--soil-hyper-albedo-dir");
    let mkinidata = rust_preprocessor_arguments(
        Stage::MkIniData,
        &namelist,
        &hyperspectral_kernel(),
        Some(&params),
        None,
    )
    .unwrap();
    assert_eq!(mkinidata[0], "--hyperspectral");
    assert!(mkinidata
        .iter()
        .any(|argument| argument == "--highres-radiation"));
    assert!(mkinidata
        .iter()
        .any(|argument| argument == "--highres-leaf-optics"));
    assert!(mkinidata
        .iter()
        .any(|argument| argument == "--highres-water-optics"));
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
    let urban = root.join("urban_albedo.nc");
    let first = rust_preprocessor_input_identity(&[
        "--highres-urban-albedo".into(),
        urban.to_string_lossy().into_owned(),
    ])
    .unwrap();
    std::fs::write(&urban, "urban-updated").unwrap();
    let second = rust_preprocessor_input_identity(&[
        "--highres-urban-albedo".into(),
        urban.to_string_lossy().into_owned(),
    ])
    .unwrap();
    assert_ne!(first, second);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hyperspectral_pc_without_soil_or_pft_optics_needs_radiation_and_urban_albedo() {
    let root = test_directory("hyperspectral-pc-urban");
    let namelist = hyperspectral_pft_namelist(&root);
    std::fs::write(
        &namelist,
        "&nl_colm\nDEF_file_mesh='mesh.nc'\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.false.\nDEF_USE_PC=.true.\nDEF_HighResSoil=.false.\nDEF_HighResUrban_albedo='urban_albedo.nc'\n/\n",
    )
    .unwrap();
    let params = root.join("params");
    std::fs::create_dir_all(params.join("fsds")).unwrap();
    std::fs::write(params.join("fsds/swnb_480bnd_fsds.nc"), "radiation").unwrap();
    let arguments = rust_preprocessor_arguments(
        Stage::MkIniData,
        &namelist,
        &hyperspectral_kernel(),
        Some(&params),
        None,
    )
    .unwrap();
    assert_eq!(
        arguments,
        vec![
            "--hyperspectral".to_owned(),
            "--highres-urban-albedo".to_owned(),
            root.join("urban_albedo.nc")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--highres-radiation".to_owned(),
            params
                .join("fsds/swnb_480bnd_fsds.nc")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_changed_hyperspectral_surface_source_invalidates_downstream_stages() {
    for stage in [Stage::MkSrfData, Stage::MkIniData, Stage::Colm] {
        assert_eq!(
            stage_preprocessor_input_identity(stage, Some(""), Some("")),
            None
        );
        assert_eq!(
            stage_preprocessor_input_identity(stage, None, Some("")),
            None
        );
    }
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

#[test]
fn rust_model_engine_is_the_default_and_fortran_is_selectable() {
    assert_eq!(requested_engine(None).unwrap(), ModelEngine::Rust);
    assert_eq!(
        requested_engine(Some("fortran")).unwrap(),
        ModelEngine::Fortran
    );
    assert!(requested_engine(Some("auto")).is_err());
}

#[test]
fn model_engine_identity_only_changes_the_colm_stage_fingerprint() {
    let fortran = RustPreprocessorIdentities {
        surface: Some("mksrfdata=a".into()),
        initial: Some("mkinidata=b".into()),
        model: None,
    };
    let rust = RustPreprocessorIdentities {
        surface: Some("mksrfdata=a".into()),
        initial: Some("mkinidata=b".into()),
        model: Some("colm-rs=c".into()),
    };
    for stage in [Stage::MkSrfData, Stage::MkIniData] {
        assert_eq!(
            stage_kernel_identity("k", stage, &fortran),
            stage_kernel_identity("k", stage, &rust),
            "{stage:?} must not rerun when only the model engine changes"
        );
    }
    let fortran_colm = stage_kernel_identity("k", Stage::Colm, &fortran);
    let rust_colm = stage_kernel_identity("k", Stage::Colm, &rust);
    assert_ne!(fortran_colm, rust_colm);
    // Fortran 引擎的身份与引入 `model` 字段之前逐字节相同：既有算例不因升级而重跑。
    assert_eq!(
        fortran_colm,
        "k;rust-preprocessor-inputs=mksrfdata=a;mkinidata=b"
    );
    assert!(rust_colm.ends_with(";model-engine=colm-rs=c"));
}

#[test]
fn rust_model_engine_needs_an_lct_kernel() {
    let mut kernel = hyperspectral_kernel();
    kernel.manifest.macros = vec!["LULC_USGS".into()];
    assert_eq!(rust_model_land_cover(&kernel).unwrap(), "usgs");
    kernel.manifest.macros = vec!["LULC_IGBP".into(), "URBAN_MODEL".into()];
    assert_eq!(rust_model_land_cover(&kernel).unwrap(), "igbp");
    kernel.manifest.macros = vec!["LULC_IGBP_PFT".into()];
    let error = rust_model_land_cover(&kernel).unwrap_err().to_string();
    assert!(error.contains("--engine fortran"), "{error}");
}
