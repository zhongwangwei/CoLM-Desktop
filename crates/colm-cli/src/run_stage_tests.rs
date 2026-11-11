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

#[cfg(windows)]
#[test]
fn output_dir_normalization_matches_windows_verbatim_prefixes() {
    for (plain, verbatim) in [
        (r"D:\cases\probe-case", r"\\?\D:\cases\probe-case"),
        (
            r"\\server\share\probe-case",
            r"\\?\UNC\server\share\probe-case",
        ),
    ] {
        // Test UNC syntax without probing a fictitious network share.
        let expected = normalize_path_components(&Path::new(verbatim).join("out"));
        assert_eq!(
            normalize_path_components(&Path::new(plain).join("out")),
            expected
        );
        assert_ne!(
            normalize_path_components(&Path::new(plain).join("elsewhere")),
            expected
        );
    }
}

#[test]
fn cli_output_dir_accepts_canonical_case_paths_before_output_exists() {
    let root = test_directory("canonical-output-dir");
    #[cfg(not(windows))]
    let root = root.canonicalize().unwrap();
    let canonical = root.canonicalize().unwrap();
    let case_nml = canonical.join("case.nml");
    for suffix in [Path::new("out"), Path::new("missing").join("out").as_path()] {
        assert!(!root.join(suffix).exists());
        for (actual, expected) in [(&root, &canonical), (&canonical, &root)] {
            std::fs::write(
                &case_nml,
                format!(
                    "&nl_colm\nDEF_dir_output = '{}'\n/\n",
                    actual.join(suffix).display()
                ),
            )
            .unwrap();
            ensure_cli_output_dir(&case_nml, &expected.join(suffix)).unwrap();
            let error = ensure_cli_output_dir(&case_nml, &expected.join("elsewhere")).unwrap_err();
            assert!(error.to_string().contains("custom output"), "{error}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
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
    let first = rust_preprocessor_input_identity(
        &root,
        &[
            "--highres-water-optics".into(),
            source.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    std::fs::write(&source, "second").unwrap();
    let second = rust_preprocessor_input_identity(
        &root,
        &[
            "--highres-water-optics".into(),
            source.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    assert_ne!(first, second);
    let urban = root.join("urban_albedo.nc");
    let first = rust_preprocessor_input_identity(
        &root,
        &[
            "--highres-urban-albedo".into(),
            urban.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    std::fs::write(&urban, "urban-updated").unwrap();
    let second = rust_preprocessor_input_identity(
        &root,
        &[
            "--highres-urban-albedo".into(),
            urban.to_string_lossy().into_owned(),
        ],
    )
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

#[test]
fn rust_kernel_arguments_keep_grid_classification_and_crop_independent() {
    for (grid_macro, grid_kind, grid_flag) in [
        ("SinglePoint", None, None),
        ("GRIDBASED", Some("latlon"), None),
        ("UNSTRUCTURED", Some("unstructured"), Some("--unstructured")),
        ("CATCHMENT", Some("catchment"), Some("--catchment")),
    ] {
        for (classification, land_cover) in [("LULC_IGBP", "igbp"), ("LULC_USGS", "usgs")] {
            for crop in [false, true] {
                if crop && classification == "LULC_USGS" {
                    continue;
                }
                let mut kernel = hyperspectral_kernel();
                kernel.manifest.macros = vec![grid_macro.into(), classification.into()];
                if crop {
                    kernel.manifest.macros.push("CROP".into());
                }
                assert_eq!(kernel_grid_kind(&kernel), grid_kind);
                assert_eq!(rust_model_land_cover(&kernel).unwrap(), land_cover);
                let mut expected = Vec::new();
                if crop {
                    expected.push("--crop");
                }
                if let Some(flag) = grid_flag {
                    expected.push(flag);
                }
                assert_eq!(rust_model_crop_arguments(&kernel, false), expected);
            }
        }
    }
}

#[test]
fn isolated_single_stage_runs_check_their_required_upstream_stages() {
    assert!(required_upstream_stages(None).is_empty());
    assert!(required_upstream_stages(Some(Stage::MkSrfData)).is_empty());
    assert_eq!(
        required_upstream_stages(Some(Stage::MkIniData)),
        [Stage::MkSrfData]
    );
    assert_eq!(
        required_upstream_stages(Some(Stage::Colm)),
        [Stage::MkSrfData, Stage::MkIniData]
    );
}

#[test]
fn single_point_multiple_rank_preprocessing_is_rejected_before_launcher_or_outputs() {
    let root = test_directory("singlepoint-multiple-ranks");
    let namelist = root.join("case.nml");
    std::fs::write(&namelist, "&nl_colm\nDEF_CASE_NAME='single'\n/\n").unwrap();
    let error = preflight_rust_preprocessor_ranks(&namelist, 2).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("single-point cases must use --ranks 1"),
        "{error}"
    );
    preflight_rust_preprocessor_ranks(&namelist, 1).unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_surface_clipping_uses_one_rank_without_reducing_other_stages() {
    let root = test_directory("clipping-stage-ranks");
    let namelist = root.join("case.nml");
    for (setting, surface_ranks) in [(".true.", 1), (".false.", 4)] {
        std::fs::write(
            &namelist,
            format!(
                "&nl_colm\nDEF_file_mesh='mesh.nc'\nUSE_srfdata_from_larger_region={setting}\n/\n"
            ),
        )
        .unwrap();
        for (stage, expected) in [
            (Stage::MkSrfData, surface_ranks),
            (Stage::MkIniData, 4),
            (Stage::Colm, 4),
        ] {
            assert_eq!(
                rust_preprocessor_stage_ranks(&namelist, stage, 4).unwrap(),
                expected
            );
            assert_eq!(
                rust_preprocessor_stage_ranks(&namelist, stage, 1).unwrap(),
                1
            );
        }
    }
    std::fs::write(&namelist, "&nl_colm\nDEF_file_mesh='mesh.nc'\n/\n").unwrap();
    assert_eq!(
        rust_preprocessor_stage_ranks(&namelist, Stage::MkSrfData, 4).unwrap(),
        4
    );
    std::fs::write(
        &namelist,
        "&nl_colm\nDEF_file_mesh='mesh.nc'\nUSE_srfdata_from_larger_region='true'\n/\n",
    )
    .unwrap();
    assert!(
        rust_preprocessor_stage_ranks(&namelist, Stage::MkSrfData, 4)
            .unwrap_err()
            .to_string()
            .contains("must be logical")
    );
    std::fs::write(
        &namelist,
        "&nl_colm\nUSE_srfdata_from_larger_region=.true.\n/\n",
    )
    .unwrap();
    assert_eq!(
        rust_preprocessor_stage_ranks(&namelist, Stage::MkSrfData, 4).unwrap(),
        4
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rust_multi_rank_frozen_long_path_keeps_shared_namelist_and_cwd() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "COLM_TEST_SHARED_RUST_CASE";
    const TEST: &str =
        "run_stage_tests::rust_multi_rank_frozen_long_path_keeps_shared_namelist_and_cwd";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        let case = root
            .join("shared-case")
            .join("long-project-segment-".repeat(6))
            .join("long-operation-segment-".repeat(6));
        let executable = root.join("colm-rs");
        let nml = case.join("case.nml");
        let before = fingerprint::sha256_file(&nml).unwrap();
        let work = run_case_execution_path(
            &case,
            ModelEngine::Rust,
            PreprocessorMode::Rust,
            Some(Stage::Colm),
        )
        .unwrap();
        colm_kernel::run_stage_streaming_external_ranks(
            &hyperspectral_kernel(),
            Stage::Colm,
            &executable,
            &work.join("case.nml"),
            &work,
            &[],
            &["--case-outputs".into()],
            4,
            &mut |_| {},
        )
        .unwrap();
        let captured = std::fs::read_to_string(root.join("launch.txt")).unwrap();
        let lines: Vec<_> = captured.lines().collect();
        assert_eq!(
            lines,
            vec![
                case.to_str().unwrap(),
                "-n",
                "4",
                executable.to_str().unwrap(),
                nml.to_str().unwrap(),
                "--case-outputs"
            ]
        );
        assert_eq!(fingerprint::sha256_file(&nml).unwrap(), before);
        return;
    }
    let root = test_directory("shared-rust-frozen-launch")
        .canonicalize()
        .unwrap();
    let case = root
        .join("shared-case")
        .join("long-project-segment-".repeat(6))
        .join("long-operation-segment-".repeat(6));
    std::fs::create_dir_all(&case).unwrap();
    assert!(case.to_string_lossy().len() > 256);
    let nml = format!(
        "&nl_colm\nDEF_CASE_NAME='demo'\nDEF_dir_output='{}'\n/\n",
        case.join("out").display()
    );
    std::fs::write(case.join("case.nml"), &nml).unwrap();
    let hash = colm_kernel::sha256_hex(nml.as_bytes());
    let run = colm_model::MaterializedRun {
        schema: 1,
        operation_id: "operation".into(),
        source_case: root.join("source"),
        run_case: case.clone(),
        version: colm_model::VersionRef {
            id: "official".into(),
            revision: 1,
        },
        code: colm_model::CodeRef {
            kind: colm_model::CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        },
        case_sha256: hash.clone(),
        effective_nml_sha256: hash,
        overlay_sha256: "b".repeat(64),
        revision_fingerprint: "c".repeat(64),
        artifact_paths: vec![],
        inputs: Default::default(),
    };
    std::fs::write(
        case.join(colm_model::RUN_MANIFEST),
        serde_json::to_vec(&run).unwrap(),
    )
    .unwrap();
    let launcher = root.join("capture-launcher");
    std::fs::write(
        &launcher,
        "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$@\" > \"$COLM_TEST_SHARED_RUST_CASE/launch.txt\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, &root)
        .env("COLM_MPIEXEC", &launcher)
        .env_remove("COLM_MPIEXEC_ARGS")
        .output()
        .unwrap();
    // Existing native alias support remains available for Fortran under the same long frozen path.
    let native = run_case_execution_path(
        &case,
        ModelEngine::Fortran,
        PreprocessorMode::Rust,
        Some(Stage::Colm),
    )
    .unwrap();
    assert_ne!(native, case);
    assert_eq!(native.canonicalize().unwrap(), case);
    validate_native_case_paths(&native, "demo").unwrap();
    assert!(validate_native_case_paths(&case, "demo").is_err());
    std::fs::remove_file(native).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_path_limits_follow_the_stages_that_will_actually_run() {
    for engine in [ModelEngine::Rust, ModelEngine::Fortran] {
        for preprocessors in [PreprocessorMode::Rust, PreprocessorMode::Fortran] {
            for stage in [
                None,
                Some(Stage::Colm),
                Some(Stage::MkSrfData),
                Some(Stage::MkIniData),
            ] {
                let expected = match stage {
                    Some(Stage::Colm) => engine == ModelEngine::Fortran,
                    Some(_) => preprocessors == PreprocessorMode::Fortran,
                    None => {
                        engine == ModelEngine::Fortran || preprocessors == PreprocessorMode::Fortran
                    }
                };
                assert_eq!(runs_native_stage(engine, preprocessors, stage), expected);
            }
        }
    }
}

#[test]
fn rust_crop_detection_uses_effective_site_landtype() {
    let dir = test_directory("rust-crop-effective-landtype");
    std::fs::create_dir_all(&dir).unwrap();
    let site = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc");
    let case = dir.join("case.nml");
    std::fs::write(
        &case,
        format!("&nl_colm\nSITE_fsitedata = '{}'\n/\n", site.display()),
    )
    .unwrap();
    assert!(rust_case_is_crop(&case).unwrap());
}

#[test]
fn a_default_zero_planting_day_does_not_enable_rust_crop() {
    let dir = test_directory("rust-crop-zero-planting");
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.join("case.nml");
    std::fs::write(&case, "&nl_colm\nDEF_TUNING_CROP_PLANTING_DAY = 0.0\n/\n").unwrap();
    assert!(!rust_case_is_crop(&case).unwrap());
}

#[test]
fn a_negative_site_landtype_still_reads_the_crop_site_classification() {
    let dir = test_directory("rust-crop-negative-landtype");
    std::fs::create_dir_all(&dir).unwrap();
    let site = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc");
    let case = dir.join("case.nml");
    std::fs::write(
        &case,
        format!(
            "&nl_colm\nSITE_fsitedata = '{}'\nSITE_landtype = -1\n/\n",
            site.display()
        ),
    )
    .unwrap();
    assert!(rust_case_is_crop(&case).unwrap());
}

#[test]
fn default_crop_side_switches_do_not_enable_rust_crop() {
    let dir = test_directory("rust-crop-side-switches");
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.join("case.nml");
    std::fs::write(
        &case,
        "&nl_colm\nDEF_USE_FERT = .true.\nDEF_USE_CNSOYFIXN = .true.\n/\n",
    )
    .unwrap();
    assert!(!rust_case_is_crop(&case).unwrap());
}

#[test]
fn crop_planting_day_accepts_fortran_real_spellings() {
    let dir = test_directory("rust-crop-planting-spellings");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, value) in [("int", "120"), ("real", "120.0"), ("double", "1.2D2")] {
        let case = dir.join(format!("case-{name}.nml"));
        std::fs::write(
            &case,
            format!("&nl_colm\nDEF_TUNING_CROP_PLANTING_DAY = {value}\n/\n"),
        )
        .unwrap();
        assert!(rust_case_is_crop(&case).unwrap(), "{name} should be crop");
    }
}

#[cfg(unix)]
fn preprocessor_run_fixture(label: &str, script: &[u8]) -> (PathBuf, PathBuf, Kernel) {
    use std::os::unix::fs::PermissionsExt;
    let root = test_directory(label);
    let case = root.join("case");
    let output = case.join("out/demo");
    std::fs::create_dir_all(output.join("history")).unwrap();
    let old_history = output.join("history/demo_hist_2000.nc");
    std::fs::write(&old_history, b"old scientific result").unwrap();
    std::fs::write(
        case.join("case.nml"),
        format!(
            "&nl_colm\nDEF_CASE_NAME='demo'\nDEF_dir_output='{}'\n/\n",
            case.join("out").display()
        ),
    )
    .unwrap();
    let kernel_dir = root.join("kernel");
    std::fs::create_dir_all(&kernel_dir).unwrap();
    let mut hashes = std::collections::BTreeMap::new();
    for program in ["mksrfdata", "mkinidata", "colm"] {
        let executable = kernel_dir.join(colm_kernel::program_file(program));
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        hashes.insert(program, colm_kernel::sha256_hex(script));
    }
    let manifest = serde_json::json!({
        "schema": colm_kernel::manifest::SCHEMA,
        "preset": "test",
        "platform": "test",
        "colm_git_sha": "test",
        "generator_args": "SinglePoint",
        "macros": ["SinglePoint", "LULC_IGBP"],
        "built_with": "fixture",
        "netcdf_c": "",
        "netcdf_fortran": "",
        "hdf5": "",
        "sha256": hashes,
    });
    std::fs::write(
        kernel_dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let kernel =
        Kernel::open(&kernel_dir).expect("all three fixture programs match their manifest hashes");
    (case, old_history, kernel)
}

#[cfg(unix)]
#[test]
fn failed_preprocessor_reruns_invalidate_old_results_without_deleting_history() {
    for stage in [Stage::MkSrfData, Stage::MkIniData] {
        let (case, old_history, kernel) = preprocessor_run_fixture(
            &format!("stale-{}", stage.program()),
            b"#!/bin/sh\necho preprocessor-started\nexit 7\n",
        );
        let error = cmd_run(
            &case,
            &kernel.dir,
            false,
            true,
            Some(stage),
            1,
            PreprocessorMode::Fortran,
            ModelEngine::Fortran,
            None,
            None,
            false,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("stage {} failed", stage.program())),
            "{error:#}"
        );
        assert!(colm_case::results_are_stale(&case));
        assert_eq!(
            std::fs::read(&old_history).unwrap(),
            b"old scientific result"
        );
        std::fs::remove_dir_all(case.parent().unwrap()).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn successful_preprocessing_is_not_failed_by_an_unwritable_launch_record() {
    let (case, old_history, kernel) = preprocessor_run_fixture(
        "launch-record-warning",
        b"#!/bin/sh\necho 'Successful in surface data making.'\nexit 0\n",
    );
    let surface = case.join("out/demo/landdata/srfdata.nc");
    std::fs::create_dir_all(surface.parent().unwrap()).unwrap();
    std::fs::write(&surface, b"fixture surface artifact").unwrap();
    std::fs::create_dir(case.join(provenance::EXECUTIONS_FILE)).unwrap();
    cmd_run(
        &case,
        &kernel.dir,
        false,
        true,
        Some(Stage::MkSrfData),
        1,
        PreprocessorMode::Fortran,
        ModelEngine::Fortran,
        None,
        None,
        false,
    )
    .unwrap();
    assert!(old_history.is_file());
    assert_eq!(
        std::fs::read(&surface).unwrap(),
        b"fixture surface artifact"
    );
    std::fs::remove_dir_all(case.parent().unwrap()).unwrap();
}
