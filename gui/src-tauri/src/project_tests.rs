use super::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("colm-gui-project-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn make_case(root: &Path, dir: &str, name: &str) -> PathBuf {
    let d = root.join(dir);
    std::fs::create_dir_all(&d).expect("mkdir");
    std::fs::write(
        d.join("case.nml"),
        format!("&nl_colm\n   DEF_CASE_NAME = '{name}'\n/\n"),
    )
    .expect("write");
    d
}

#[test]
fn a_directory_with_a_case_nml_is_a_case() {
    let root = tmp("basic");
    make_case(&root, "one", "CN-Cng");
    make_case(&root, "two", "AT-Neu");
    // 没有 case.nml 的目录不算
    std::fs::create_dir_all(root.join("not-a-case")).expect("mkdir");

    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert_eq!(cases.len(), 2);
    // 按算例名排序，不是按目录名 —— 界面上用户看到的是算例名
    assert_eq!(cases[0].name, "AT-Neu");
    assert_eq!(cases[1].name, "CN-Cng");
}

#[test]
fn the_name_comes_from_the_namelist_not_the_directory() {
    // 目录叫 whatever，算例叫 CN-Cng。产物路径由后者决定，
    // 所以界面必须显示后者，否则用户在磁盘上找不到自己的东西。
    let root = tmp("naming");
    make_case(&root, "whatever", "CN-Cng");
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert_eq!(cases[0].name, "CN-Cng");
    assert!(cases[0].dir.ends_with("whatever"));
}

#[test]
fn a_case_without_a_history_file_is_marked_as_not_run() {
    let root = tmp("unrun");
    let d = make_case(&root, "one", "CN-Cng");
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert!(!cases[0].has_history);

    // 放一个 history 进去就算跑过
    let h = d.join("out/CN-Cng/history");
    std::fs::create_dir_all(&h).expect("mkdir");
    std::fs::write(h.join("CN-Cng_hist_2008-01.nc"), b"not really netcdf").expect("write");
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert!(cases[0].has_history);

    mark_results_stale(vec![d.to_string_lossy().into_owned()]).unwrap();
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert!(
        !cases[0].has_history,
        "stale history must stay hidden after a rescan"
    );
    colm_case::clear_results_stale(&d).unwrap();
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert!(cases[0].has_history);
}

#[test]
fn history_lookup_honors_custom_output_roots_and_stale_markers() {
    let root = tmp("custom-output-history");
    for (folder, output) in [
        ("relative", "../relative-results".to_string()),
        (
            "absolute",
            root.join("absolute-results").to_string_lossy().into_owned(),
        ),
    ] {
        let case = make_case(&root, folder, "site");
        std::fs::write(
            case.join("case.nml"),
            format!("&nl_colm\n DEF_CASE_NAME='site'\n DEF_dir_output='{output}'\n/\n"),
        )
        .unwrap();
        let history = case.join(&output).join("site/history");
        std::fs::create_dir_all(&history).unwrap();
        std::fs::write(history.join("site_hist_2003-01.nc"), b"fixture").unwrap();
        assert!(history_of(&case, "site").is_some(), "{output}");
        assert!(
            list_cases(root.to_string_lossy().into_owned())
                .unwrap()
                .iter()
                .find(|entry| entry.dir == case.to_string_lossy())
                .unwrap()
                .has_history
        );
        mark_results_stale(vec![case.to_string_lossy().into_owned()]).unwrap();
        assert!(
            !list_cases(root.to_string_lossy().into_owned())
                .unwrap()
                .iter()
                .find(|entry| entry.dir == case.to_string_lossy())
                .unwrap()
                .has_history
        );
    }
}

#[test]
fn a_path_like_case_name_cannot_escape_history_lookup() {
    let root = tmp("escaped-history");
    make_case(&root, "case", "../escape");
    let outside = root.join("escape/history");
    std::fs::create_dir_all(&outside).expect("mkdir outside history");
    std::fs::write(outside.join("escape_hist_2008-01.nc"), b"not really netcdf").expect("write");

    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert_eq!(cases[0].name, "case");
    assert!(
        !cases[0].has_history,
        "invalid DEF_CASE_NAME must not read ../escape/history"
    );
    assert!(validate_case_name("../escape").is_err());
}

#[test]
fn a_missing_directory_says_so_rather_than_returning_nothing() {
    // 返回空列表会被界面渲染成「这里没有算例」，而真相是路径写错了。
    let e = list_cases("/no/such/place/at/all".into()).unwrap_err();
    assert!(e.contains("/no/such/place"), "{e}");
}

#[test]
fn spatial_metadata_uses_real_mesh_paths_not_site_template_defaults() {
    let root = tmp("spatial-metadata");
    for (name, fields, expected) in [
        ("site", "SITE_fsrfdata = 'site.nc'\n DEF_domain%edgew = -180\n DEF_file_mesh = ''\n DEF_CatchmentMesh_data = ' NuLl '\n ! DEF_file_mesh = 'comment.nc'", false),
        ("template", "SITE_fsrfdata = 'site.nc'\n DEF_file_mesh = 'path/to/mesh/file'\n DEF_CatchmentMesh_data = 'path/to/catchment/data'", false),
        ("region", "DEF_file_mesh = 'mesh.nc'", true),
        ("catchment", "DEF_CatchmentMesh_data = 'catchment.nc'", true),
    ] {
        let case = make_case(&root, name, name);
        std::fs::write(case.join("case.nml"), format!("&nl_colm\n DEF_CASE_NAME = '{name}'\n {fields}\n/\n")).unwrap();
        assert_eq!(colm_case::is_spatial_case(&case.join("case.nml")).unwrap(), expected);
    }
    let entries = list_cases(root.to_string_lossy().into_owned()).unwrap();
    for entry in entries {
        assert_eq!(
            entry.spatial,
            matches!(entry.name.as_str(), "region" | "catchment"),
            "{}",
            entry.name
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_opened_spatial_case_recovers_its_wizard_choices() {
    let text = "&nl_colm\n DEF_CASE_NAME = 'c8'\n DEF_domain%edgew = 96.25\n DEF_domain%edgee = 104.25\n \
                DEF_domain%edges = 37.75\n DEF_domain%edgen = 43.25\n DEF_file_mesh = '/w/mesh.nc'\n \
                DEF_GRIDBASED_lon_res = 0.25\n DEF_GRIDBASED_lat_res = 0.25\n DEF_USE_PFT = .true.\n \
                DEF_USE_GridRiverLakeFlow = .false.\n DEF_USE_BGC = .true.\n/\n";
    let stages = r#"{"colm":{"kernel":"preset=latlon;platform=Darwin-arm64;args=GRID LULC_IGBP;macros=GRIDBASED,GridRiverLakeFlow,LULC_IGBP"}}"#;
    let p = case_profile(text, true, Some(stages), None).expect("profile");
    assert_eq!(p.grid, Some("latlon"));
    assert_eq!(p.subgrid, "PFT");
    assert!(p.bgc && !p.river && !p.urban);
    assert_eq!(p.domain, Some([96.25, 104.25, 37.75, 43.25]));
    assert_eq!(p.resolution, Some([0.25, 0.25]));
    assert_eq!(p.kernel_preset.as_deref(), Some("latlon"));
}

#[test]
fn an_opened_case_brings_back_its_inputs() {
    let text =
        "&nl_colm\n DEF_CASE_NAME = 'spatial-case9'\n DEF_simulation_time%start_year = 2003\n \
                DEF_simulation_time%start_month = 3\n DEF_simulation_time%end_year = 2003\n \
                DEF_simulation_time%end_month = 4\n DEF_simulation_time%end_day = 2\n \
                DEF_simulation_time%end_sec = 86400\n DEF_simulation_time%timestep = 1800.\n \
                DEF_dir_rawdata = '/d/raw/'\n DEF_dir_runtime = '/d/run/'\n \
                DEF_forcing_namelist = '/c/forcing.nml'\n/\n";
    let p = case_profile(text, true, None, None).expect("profile");
    assert_eq!(
        p.inputs,
        CaseInputs {
            name: "spatial-case9".into(),
            rawdata: "/d/raw/".into(),
            runtime: "/d/run/".into(),
            forcing_namelist: "/c/forcing.nml".into(),
            start: "2003-03-01".into(),
            end: "2003-04-02".into(),
            timestep: 1800.0,
        }
    );
}

#[test]
fn an_opened_site_case_reads_usgs_and_methane_from_the_last_run() {
    let text =
        "&nl_colm\n DEF_CASE_NAME = 's'\n DEF_USE_TRACER = .true.\n DEF_TRACER_NAMES = 'CH4'\n/\n";
    let stages = r#"{"mksrfdata":{"kernel":"preset=usgs;macros=SinglePoint,LULC_USGS"}}"#;
    let p = case_profile(text, false, Some(stages), None).expect("profile");
    assert_eq!((p.grid, p.subgrid), (None, "USGS"));
    assert!(p.methane && !p.river);
    assert_eq!(p.domain, None);
    let fresh = case_profile(text, false, None, None).expect("profile");
    assert_eq!((fresh.subgrid, fresh.kernel_preset), ("IGBP", None));
}

#[test]
fn classification_survives_pft_and_pc_runtime_structure() {
    let stages = r#"{"colm":{"kernel":"preset=usgs;macros=SinglePoint,LULC_USGS"}}"#;
    for structure in ["PFT", "PC"] {
        let text = format!("&nl_colm\n DEF_USE_{structure}=.true.\n/\n");
        let ran = case_profile(&text, false, Some(stages), Some("igbp")).unwrap();
        assert_eq!(ran.subgrid, structure);
        assert_eq!(ran.classification, "IGBP");
        assert_eq!(ran.classification_source, "creation");
        let created = case_profile(&text, false, None, Some("usgs")).unwrap();
        assert_eq!(created.classification, "USGS");
        assert_eq!(created.classification_source, "creation");
    }
    let legacy = case_profile("&nl_colm\n/\n", false, None, None).unwrap();
    assert_eq!(legacy.classification_source, "legacy-default");
}

#[test]
fn selected_case_scheme_takes_precedence_over_prior_kernel_classification() {
    let stages = r#"{"colm":{"kernel":"preset=usgs;macros=SinglePoint,LULC_USGS"}}"#;
    for scheme in ["IGBP", "USGS", "PFT", "PC"] {
        let text = format!(
            "&nl_colm\n DEF_SUBGRID_SCHEME='{scheme}'\n DEF_USE_LCT=.{}.\n DEF_USE_PFT=.{}.\n DEF_USE_PC=.{}.\n/\n",
            matches!(scheme, "IGBP" | "USGS"), scheme == "PFT", scheme == "PC",
        );
        let profile = case_profile(&text, false, Some(stages), Some("usgs")).unwrap();
        assert_eq!(profile.subgrid, scheme);
        assert_eq!(
            profile.classification,
            if scheme == "IGBP" { "IGBP" } else { "USGS" }
        );
        assert_eq!(
            profile.classification_source,
            if matches!(scheme, "IGBP" | "USGS") {
                "case"
            } else {
                "creation"
            }
        );
        assert_eq!(profile.kernel_preset.as_deref(), Some("usgs"));
    }
}

#[test]
fn opening_a_directory_without_case_nml_is_refused() {
    let root = tmp("open-none");
    assert!(open_case(root.to_string_lossy().into_owned()).is_err());
    let dir = make_case(&root, "c", "CN-Cng");
    let opened = open_case(dir.to_string_lossy().into_owned()).expect("opens");
    assert_eq!(opened.entry.name, "CN-Cng");
    assert_eq!(Path::new(&opened.root), root.as_path());
}

#[test]
fn a_never_run_case_recovers_usgs_and_crop_from_creation() {
    let text = "&nl_colm\n DEF_CASE_NAME = 's'\n DEF_USE_BGC = .true.\n DEF_USE_PFT = .true.\n \
                DEF_TUNING_CROP_PLANTING_DAY = 120\n/\n";
    let p = case_profile(text, false, None, Some("pft")).expect("profile");
    assert!(p.crop, "the planting-day field marks a crop case");
    let lct = "&nl_colm\n DEF_CASE_NAME = 'u'\n DEF_USE_LCT = .true.\n/\n";
    assert_eq!(
        case_profile(lct, false, None, Some("usgs"))
            .unwrap()
            .subgrid,
        "USGS"
    );
    assert_eq!(
        case_profile(lct, false, None, Some("urban-usgs"))
            .unwrap()
            .subgrid,
        "USGS"
    );
    assert_eq!(
        case_profile(lct, false, None, Some("igbp"))
            .unwrap()
            .subgrid,
        "IGBP"
    );
    // 算例分类选择优先于上次内核；内核仍提供作物能力。
    let stages = r#"{"colm":{"kernel":"preset=crop;macros=SinglePoint,LULC_IGBP,CROP"}}"#;
    let ran = case_profile(lct, false, Some(stages), Some("usgs")).unwrap();
    assert_eq!(ran.subgrid, "USGS");
    assert!(ran.crop);
}

#[test]
fn the_creation_record_round_trips() {
    let root = tmp("mode-record");
    let dir = make_case(&root, "c", "CN-Cng");
    let record = CaseRecord {
        mode: Some("usgs".into()),
        domain: Some("watershed".into()),
        shapefile: Some("/d/basin.shp".into()),
        engine: None,
        ..Default::default()
    };
    record_case(&dir.to_string_lossy(), &record).unwrap();
    assert_eq!(recorded_case(&dir), record);
    let opened = open_case(dir.to_string_lossy().into_owned()).expect("opens");
    assert_eq!(opened.profile.domain_kind.as_deref(), Some("watershed"));
    assert_eq!(opened.profile.shapefile.as_deref(), Some("/d/basin.shp"));
    assert_eq!(opened.profile.subgrid, "USGS");
}

#[test]
fn run_host_round_trips_and_local_selection_preserves_case_metadata() {
    let root = tmp("run-host-record");
    let dir = make_case(&root, "c", "CN-Cng");
    let dir_arg = dir.to_string_lossy().into_owned();
    assert_eq!(open_case(dir_arg.clone()).unwrap().profile.run_host, None);
    let mut record = CaseRecord {
        mode: Some("usgs".into()),
        domain: Some("watershed".into()),
        shapefile: Some("/d/basin.shp".into()),
        engine: Some("rust".into()),
        site: Some("/d/site.nc".into()),
        obs_file: Some("/d/obs.nc".into()),
        ..Default::default()
    };
    record_case(&dir_arg, &record).unwrap();
    set_case_run_host(dir_arg.clone(), Some("7920l".into())).unwrap();
    record.run_host = Some("7920l".into());
    assert_eq!(recorded_case(&dir), record);
    assert_eq!(
        open_case(dir_arg.clone())
            .unwrap()
            .profile
            .run_host
            .as_deref(),
        Some("7920l")
    );
    for host in [Some(String::new()), None] {
        set_case_run_host(dir_arg.clone(), Some("7920l".into())).unwrap();
        set_case_run_host(dir_arg.clone(), host).unwrap();
        record.run_host = None;
        assert_eq!(recorded_case(&dir), record);
    }
}

#[test]
fn run_host_rejects_invalid_hosts_and_broken_metadata_without_writing() {
    let root = tmp("run-host-invalid");
    assert!(set_case_run_host(root.to_string_lossy().into_owned(), None).is_err());
    assert!(!root.join(PROFILE_FILE).exists());
    let dir = make_case(&root, "c", "CN-Cng");
    let dir_arg = dir.to_string_lossy().into_owned();
    for host in [" ", "host name", "host\nname", "host\0name", "-option"] {
        assert!(set_case_run_host(dir_arg.clone(), Some(host.into())).is_err());
        assert!(!dir.join(PROFILE_FILE).exists());
    }
    std::fs::write(dir.join(PROFILE_FILE), "{broken").unwrap();
    assert!(set_case_run_host(dir_arg.clone(), Some("7920l".into())).is_err());
    assert_eq!(
        std::fs::read_to_string(dir.join(PROFILE_FILE)).unwrap(),
        "{broken"
    );
    std::fs::remove_file(dir.join(PROFILE_FILE)).unwrap();
    set_case_run_host(dir_arg, Some("7920l".into())).unwrap();
    assert_eq!(recorded_case(&dir).run_host.as_deref(), Some("7920l"));
}

#[test]
fn creation_record_write_errors_are_returned_to_the_creation_transaction() {
    let root = tmp("record-write-error");
    let missing = root.join("missing");
    assert!(record_case(&missing.to_string_lossy(), &CaseRecord::default()).is_err());
    assert!(!missing.exists());
}

#[test]
fn opened_meshes_keep_their_files_and_no_implied_global_domain() {
    let text = "&nl_colm\n DEF_CASE_NAME = 'm'\n DEF_file_mesh = '/d/mesh.nc'\n/\n";
    let p = case_profile(text, true, None, None).expect("profile");
    assert_eq!(p.grid, Some("unstructured"));
    assert_eq!(p.mesh_file.as_deref(), Some("/d/mesh.nc"));
    assert_eq!(
        p.domain, None,
        "schema defaults are not an explicit global domain"
    );
    let catch = "&nl_colm\n DEF_CASE_NAME = 'c'\n DEF_CatchmentMesh_data = '/d/basins.nc'\n/\n";
    let p = case_profile(catch, true, None, None).expect("profile");
    assert_eq!(p.catchment_file.as_deref(), Some("/d/basins.nc"));
}

#[test]
fn default_observation_matches_site_and_forcing() {
    let root = tmp("default-obs");
    let dir = make_case(&root, "obs-case", "CN-Cng");
    let data_root = root.join("dataset");
    let forcing_dir = data_root.join("Forcing");
    let obs_dir = data_root.join("Observation");
    std::fs::create_dir_all(&forcing_dir).unwrap();
    std::fs::create_dir_all(&obs_dir).unwrap();
    let obs_file = obs_dir.join("CN-Cng_2008-2009_FLUXNET2015_Flux.nc");
    std::fs::write(&obs_file, b"netcdf").unwrap();

    let forcing_nml = format!(
        "&nl_colm_forcing\n DEF_dir_forcing = '{}'\n DEF_forcing%fprefix(1) = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'\n/\n",
        forcing_dir.display()
    );
    std::fs::write(dir.join("forcing.nml"), forcing_nml).unwrap();

    let opened = open_case(dir.to_string_lossy().into_owned()).expect("opens");
    assert_eq!(
        opened.profile.obs_file.as_deref(),
        Some(obs_file.to_str().unwrap())
    );
}

#[test]
fn server_case_cannot_change_its_owning_host() {
    let root = tmp("server-owned-host");
    let dir = make_case(&root, "site", "CN-Cng");
    let argument = dir.to_string_lossy().into_owned();
    let record = CaseRecord {
        run_host: Some("compute".into()),
        remote_directory: Some("/server/project/site".into()),
        ..Default::default()
    };
    record_case(&argument, &record).unwrap();
    let listed = list_cases(root.to_string_lossy().into_owned()).unwrap();
    assert_eq!(listed[0].run_host.as_deref(), Some("compute"));
    assert_eq!(
        listed[0].remote_directory.as_deref(),
        Some("/server/project/site")
    );
    let opened = open_case(argument.clone()).unwrap();
    assert_eq!(opened.entry.run_host, listed[0].run_host);
    assert_eq!(opened.entry.remote_directory, listed[0].remote_directory);
    for host in [None, Some("other".into())] {
        assert!(set_case_run_host(argument.clone(), host).is_err());
        assert_eq!(recorded_case(&dir), record);
    }
    set_case_run_host(argument.clone(), Some("compute".into())).unwrap();
    assert_eq!(
        open_case(argument)
            .unwrap()
            .profile
            .remote_directory
            .as_deref(),
        Some("/server/project/site")
    );
}

#[test]
fn shell_open_receives_plain_windows_paths() {
    assert_eq!(
        super::plain_windows_path(r"\\?\C:\Users\me\out.nc"),
        r"C:\Users\me\out.nc"
    );
    assert_eq!(
        super::plain_windows_path(r"\\?\UNC\server\share\out.nc"),
        r"\\server\share\out.nc"
    );
    assert_eq!(
        super::plain_windows_path("/Users/me/out.nc"),
        "/Users/me/out.nc"
    );
}
