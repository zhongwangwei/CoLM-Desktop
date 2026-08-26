use super::*;

fn set_batch(dirs: Vec<String>, path: String, value: String) -> Result<BatchWrite, String> {
    super::set_field_batch(dirs, path, value, None)
}

fn set_batches(dirs: Vec<String>, fields: Vec<FieldChange>) -> Result<BatchWrite, String> {
    super::set_fields_batch(dirs, fields, None)
}

fn set_test_spinup(dirs: Vec<String>, years: u32, repeat: u32) -> Result<BatchWrite, String> {
    super::set_spinup(dirs, years, repeat, None)
}

const SAMPLE: &str = "\
&nl_colm

! 用户自己的笔记 —— 保存一次不该把它冲掉
   DEF_CASE_NAME = 'CN-Cng'          ! 冬季窗口
   DEF_USE_OZONEDATA = .FALSE.
   DEF_simulation_time%start_year = 2008
   DEF_simulation_time%timestep = 1800.
   USE_SITE_topostd = .false.
/
";

#[test]
fn reading_a_case_marks_what_the_schema_knows() {
    let e = read_case(SAMPLE.into()).expect("parses");
    let by = |n: &str| e.iter().find(|x| x.path == n).expect(n);
    assert!(by("DEF_CASE_NAME").known);
    assert_eq!(by("DEF_CASE_NAME").group, Some("nl_colm"));
    assert!(!by("DEF_CASE_NAME").derived);
    // 上游删掉的字段。CoLM 读到会 `Cannot match namelist object name` 然后停 ——
    // 界面该在开跑前点名它，而不是让用户对着那句报错发呆。
    assert!(!by("USE_SITE_topostd").known);
}

#[test]
fn described_defaults_are_writable_fortran_literals() {
    let fields = describe_fields();
    let default = |name: &str| {
        fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .default
            .as_str()
    };
    assert_eq!(default("DEF_Runoff_SCHEME"), "3");
    assert_eq!(default("DEF_USE_SNICAR"), ".false.");
    assert_eq!(default("DEF_precip_phase_discrimination_scheme"), "I");
    assert_eq!(default("DEF_SSP"), "off");
    assert_eq!(default("DEF_simulation_time%timestep"), "1800.");
    for name in [
        "DEF_BALL_BERRY_GRADM",
        "DEF_BALL_BERRY_BINTER",
        "DEF_MEDLYN_G1",
        "DEF_MEDLYN_G0",
        "DEF_WUE_LAMBDA",
    ] {
        assert_eq!(default(name), "-1.0_r8", "{name}");
    }
    for name in [
        "DEF_TUNING_ZLND",
        "DEF_TUNING_CAPR",
        "DEF_PH_ROOT_RADIUS",
        "DEF_OZONE_KO3",
        "DEF_DS_SHORTWAVE_SIMPLE_LIMIT",
    ] {
        assert!(!default(name).is_empty(), "{name}");
    }
}

#[test]
fn unknown_fields_names_the_ones_colm_would_reject() {
    // USE_SITE_topostd 与 USE_SITE_BVIC 都在上游自己发布的单点示例
    // run/examples/SiteSYSUAtmos_IGBP_VG.nml 里，而两者都已从
    // MOD_Namelist.F90 删除 —— 那个示例现在根本跑不了。
    let u = unknown_fields(SAMPLE.into()).expect("parses");
    assert_eq!(u, ["USE_SITE_topostd"]);

    let pft =
        unknown_fields("&nl_colm\n DEF_PFT_C3C4(15)=0\n DEF_PFT_C3C4(80)=0\n/\n".into()).unwrap();
    assert_eq!(pft, ["DEF_PFT_C3C4(80)"], "only slots 1..=79 are valid");
}

#[test]
fn changing_one_field_leaves_every_other_line_byte_identical() {
    // colm-namelist 的往返保证。用户的注释与对齐是他们自己的东西。
    let out = set_field(SAMPLE.into(), "DEF_CASE_NAME".into(), "CN-Cng-wet".into()).expect("sets");
    let differing: Vec<usize> = SAMPLE
        .lines()
        .zip(out.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(differing.len(), 1, "changed lines: {differing:?}");
    assert_eq!(SAMPLE.lines().count(), out.lines().count());
    assert!(out.contains("'CN-Cng-wet'"));
    // 行尾注释还在
    assert!(out.contains("! 冬季窗口"));
}

#[test]
fn a_value_of_the_wrong_type_is_refused_before_it_reaches_the_file() {
    // 前端只送字符串；类型由 schema 决定。送错了要在这里就被拦下，
    // 而不是写进文件、等 CoLM 跑起来才报。
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%start_year".into(),
        "早点".into(),
    )
    .unwrap_err();
    assert!(e.contains("integer"), "{e}");
    let e = set_field(SAMPLE.into(), "DEF_USE_OZONEDATA".into(), "yes".into()).unwrap_err();
    assert!(e.contains("logical"), "{e}");
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%timestep".into(),
        "half".into(),
    )
    .unwrap_err();
    assert!(e.contains("real"), "{e}");
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%timestep".into(),
        "NaN".into(),
    )
    .unwrap_err();
    assert!(e.contains("finite"), "{e}");
}

#[test]
fn a_real_keeps_the_spelling_it_was_given() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，但往返要还原用户写的那种，
    // 否则每次保存都改写一遍用户没动过的写法。
    for spelling in ["3600.", "3600.0", "3.6e3"] {
        let out = set_field(
            SAMPLE.into(),
            "DEF_simulation_time%timestep".into(),
            spelling.into(),
        )
        .expect("sets");
        assert!(out.contains(spelling), "{spelling} not found in output");
    }
}

#[test]
fn a_string_longer_than_the_declared_length_is_refused() {
    // DEF_CASE_NAME 是 character(len=256)。超长会被 Fortran 悄悄截断，
    // 于是产物目录名与用户以为的不同 —— 在这里拦下说得清楚得多。
    let long = "x".repeat(300);
    let e = set_field(SAMPLE.into(), "DEF_CASE_NAME".into(), long).unwrap_err();
    assert!(e.contains("256") && e.contains("300"), "{e}");
}

#[test]
fn setting_a_field_the_file_does_not_have_is_an_error_not_an_append() {
    // 静默追加会写出一个同名字段出现两次的文件，Fortran 取最后一个，
    // 而用户在界面上看到的是第一个。
    let e = set_field(SAMPLE.into(), "DEF_HIST_FREQ".into(), "HOURLY".into()).unwrap_err();
    assert!(e.contains("no such field"), "{e}");
}

/// 造 n 个算例目录，每个一份 case.nml。`tag` 让各测试互不干扰 ——
/// 本项目没有引入 tempfile，与 `sites_tests` 一样自己套一层。
fn batch(tag: &str, texts: &[&str]) -> Vec<String> {
    let tmp = std::env::temp_dir().join(format!("colm-batch-{tag}"));
    let _ = std::fs::remove_dir_all(&tmp);
    batch_in(&tmp, texts)
}

fn batch_in(tmp: &std::path::Path, texts: &[&str]) -> Vec<String> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let d = tmp.join(format!("case{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("case.nml"), t).unwrap();
            d.display().to_string()
        })
        .collect()
}

fn pft_test_kernel(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-pft-kernel-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut hashes = serde_json::Map::new();
    for program in colm_kernel::manifest::PROGRAMS {
        let bytes = format!("fake {program}");
        std::fs::write(
            dir.join(colm_kernel::manifest::program_file(program)),
            bytes.as_bytes(),
        )
        .unwrap();
        hashes.insert(
            program.into(),
            serde_json::Value::String(colm_kernel::manifest::sha256_hex(bytes.as_bytes())),
        );
    }
    let manifest = serde_json::json!({
        "schema": 1,
        "preset": "pft-test",
        "platform": std::env::consts::OS,
        "colm_git_sha": "test",
        "generator_args": "SinglePoint LULC_IGBP CaMaOFF CROPOFF",
        "build_profile": "debug",
        "macros": ["SinglePoint", "LULC_IGBP", "URBAN_MODEL"],
        "built_with": "test",
        "netcdf_c": "test",
        "netcdf_fortran": "test",
        "hdf5": "",
        "sha256": hashes,
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    dir
}

const NML_A: &str = "&nl_colm\n   DEF_simulation_time%start_year = 2002\n   DEF_simulation_time%end_year = 2013\n   DEF_HIST_FREQ = 'HOURLY'\n/\n";
const NML_B: &str = "&nl_colm\n   DEF_simulation_time%start_year = 2005\n   DEF_simulation_time%end_year = 2008\n   DEF_HIST_FREQ = 'DAILY'\n/\n";

#[test]
fn one_change_lands_in_every_case_of_the_batch() {
    // 勾了 20 个站点是要配"这一次运行"，不是配其中第一个。只改第一个的话，
    // 另外 19 个会带着未改的配置跑完，而界面上看不出任何异常。
    let dirs = batch("every", &[NML_A, NML_B]);
    let r = set_batch(dirs.clone(), "DEF_HIST_FREQ".into(), "MONTHLY".into()).unwrap();
    assert_eq!(r.written, 2);
    for d in &dirs {
        let t = std::fs::read_to_string(std::path::Path::new(d).join("case.nml")).unwrap();
        assert!(t.contains("MONTHLY"), "{d} 没被改到：{t}");
    }
    // 回传的是代表算例（第一个）的新内容，前端拿它继续显示。
    assert!(r.text.contains("MONTHLY"));
    assert!(r.text.contains("2002"), "代表算例应当是第一个");
}

#[test]
fn related_changes_land_atomically_in_every_case() {
    let dirs = batch("related", &[NML_A, NML_B]);
    let topo = std::env::temp_dir().join("colm-batch-related-topography");
    std::fs::create_dir_all(&topo).unwrap();
    let r = set_batches(
        dirs.clone(),
        vec![
            FieldChange {
                path: "DEF_USE_Forcing_Downscaling".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_Forcing_Downscaling_Simple".into(),
                value: ".false.".into(),
            },
            FieldChange {
                path: "DEF_DS_HiresTopographyDataDir".into(),
                value: topo.display().to_string(),
            },
        ],
    )
    .unwrap();
    assert_eq!(r.written, 2);
    for d in &dirs {
        let text = std::fs::read_to_string(std::path::Path::new(d).join("case.nml")).unwrap();
        assert!(text.contains("DEF_USE_Forcing_Downscaling = .true."));
        assert!(text.contains("DEF_USE_Forcing_Downscaling_Simple = .false."));
        assert!(text.contains(&format!(
            "DEF_DS_HiresTopographyDataDir = '{}'",
            topo.display()
        )));
    }
}

#[test]
fn runtime_contracts_are_checked_before_batch_write() {
    let dirs = batch(
        "contract-tracer",
        &["&nl_colm\n DEF_USE_TRACER=.true.\n DEF_USE_Campbell_SOIL_MODEL=.true.\n/\n"],
    );
    let before = std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap();
    let err = set_batch(dirs.clone(), "DEF_HIST_FREQ".into(), "MONTHLY".into()).unwrap_err();
    assert!(
        err.contains("TRACER") && err.contains("van Genuchten"),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap(),
        before
    );

    let dirs = batch(
        "contract-lct",
        &["&nl_colm\n DEF_USE_LCT=.true.\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n/\n"],
    );
    let err = set_batch(dirs, "DEF_HIST_FREQ".into(), "MONTHLY".into()).unwrap_err();
    assert!(err.contains("DEF_USE_LCT"), "{err}");

    // 水同位素属于通用 TRACER，不依赖 BGC；只有 CH4 需要 PFT/PC 的碳氮池。
    let dirs = batch(
        "contract-isotope",
        &["&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=1\n DEF_TRACER_NAMES='H218O'\n DEF_TRACER_TYPES='isotope'\n/\n"],
    );
    set_batch(dirs, "DEF_HIST_FREQ".into(), "MONTHLY".into()).unwrap();

    let dirs = batch(
        "contract-urban-pc",
        &["&nl_colm\n SITE_fsitedata='site.nc'\n DEF_USE_LCT=.false.\n DEF_USE_PC=.true.\n DEF_URBAN_RUN=.true.\n/\n"],
    );
    set_batch(dirs, "DEF_HIST_FREQ".into(), "MONTHLY".into()).unwrap();
}

#[test]
fn stomatal_tuning_rejects_inactive_or_unsafe_values() {
    let dirs = batch(
        "stomatal-inactive",
        &["&nl_colm\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.false.\n/\n"],
    );
    let before = std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap();
    let err = set_batch(dirs.clone(), "DEF_MEDLYN_G1".into(), "4.0".into()).unwrap_err();
    assert!(err.contains("Medlyn"), "{err}");
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap(),
        before
    );

    set_batches(
        dirs,
        vec![
            FieldChange {
                path: "DEF_USE_MEDLYNST".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_WUEST".into(),
                value: ".false.".into(),
            },
            FieldChange {
                path: "DEF_MEDLYN_G1".into(),
                value: "4.0".into(),
            },
        ],
    )
    .expect("scheme and its coefficients are one valid atomic edit");

    let conflict =
        colm_namelist::parse("&nl_colm\n DEF_USE_MEDLYNST=.true.\n DEF_USE_WUEST=.true.\n/\n")
            .unwrap();
    let err = super::validate_runtime_contract(&conflict, &std::env::temp_dir(), None).unwrap_err();
    assert!(err.contains("不能同时开启"), "{err}");

    for (tag, field, value) in [
        ("gradm", "DEF_BALL_BERRY_GRADM", "1.6"),
        ("binter", "DEF_BALL_BERRY_BINTER", "-0.1"),
        ("g1", "DEF_MEDLYN_G1", "-0.1"),
        ("g0", "DEF_MEDLYN_G0", "NaN"),
        ("lambda", "DEF_WUE_LAMBDA", "0.0"),
    ] {
        let text = format!("&nl_colm\n {field}={value}\n/\n");
        let doc = colm_namelist::parse(&text).unwrap();
        let err = super::validate_runtime_contract(&doc, &std::env::temp_dir(), None).unwrap_err();
        assert!(err.contains(field), "{tag}: {err}");
    }
}

#[test]
fn every_model_fatal_runtime_combination_is_rejected_before_write() {
    for (tag, text, expected) in [
        (
            "bgc-lct",
            "&nl_colm\n DEF_USE_BGC=.true.\n/\n",
            "PFT 或 DEF_USE_PC",
        ),
        (
            "methane-no-bgc",
            "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=1\n DEF_TRACER_NAMES='CH4'\n DEF_TRACER_TYPES='gas'\n/\n",
            "甲烷 TRACER",
        ),
        (
            "tracer-campbell",
            "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_BGC=.true.\n DEF_USE_TRACER=.true.\n DEF_USE_Campbell_SOIL_MODEL=.true.\n/\n",
            "van Genuchten",
        ),
        (
            "tracer-bifurcation",
            "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_BGC=.true.\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=1\n DEF_USE_BIFURCATION=.true.\n/\n",
            "河道分汊",
        ),
        (
            "tracer-negative",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=1\n DEF_TRACER_BALANCE_ABORT_NBAD=-1\n/\n",
            "非负整数",
        ),
        (
            "tracer-count-negative",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=-1\n/\n",
            "0 到 1000",
        ),
        (
            "tracer-count-huge",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_NUM=1001\n/\n",
            "0 到 1000",
        ),
        (
            "tracer-relative-humidity",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_CG_RELHUM_MAX=1.0\n/\n",
            "严格位于 0 与 1",
        ),
        (
            "tracer-snow-equilibrium",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_SNOWMELT_EQUILIBRATION=1.1\n/\n",
            "SNOWMELT_EQUILIBRATION",
        ),
        (
            "tracer-canopy-equilibrium",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_CANOPY_EQUILIBRATION=-0.1\n/\n",
            "CANOPY_EQUILIBRATION",
        ),
        (
            "tracer-sublimation-skin",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_SUBL_SKIN_MM=-0.1\n/\n",
            "SUBL_SKIN_MM",
        ),
        (
            "tracer-supersaturation",
            "&nl_colm\n DEF_USE_TRACER=.true.\n DEF_TRACER_ICE_SUPERSAT_SLOPE=-0.1\n/\n",
            "ICE_SUPERSAT_SLOPE",
        ),
        (
            "timestep-zero",
            "&nl_colm\n DEF_simulation_time%timestep=0.0\n/\n",
            "不超过 3600",
        ),
        (
            "timestep-too-long",
            "&nl_colm\n DEF_simulation_time%timestep=7200.0\n/\n",
            "不超过 3600",
        ),
        (
            "lulcc-site",
            "&nl_colm\n SITE_fsitedata='site.nc'\n DEF_USE_LULCC=.true.\n/\n",
            "SinglePoint",
        ),
        (
            "lulcc-bgc",
            "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_BGC=.true.\n DEF_USE_LULCC=.true.\n/\n",
            "USGS 或 BGC",
        ),
        (
            "urban-bgc-site",
            "&nl_colm\n SITE_fsitedata='site.nc'\n DEF_USE_LCT=.true.\n DEF_URBAN_RUN=.true.\n DEF_USE_BGC=.true.\n/\n",
            "DEF_USE_BGC 需要 DEF_USE_PFT 或 DEF_USE_PC",
        ),
    ] {
        let dirs = batch(tag, &[text]);
        let err = set_batch(dirs, "DEF_HIST_FREQ".into(), "MONTHLY".into())
            .unwrap_err();
        assert!(err.contains(expected), "{tag}: {err}");
    }
}

#[test]
fn compile_time_classification_and_crop_constraints_use_kernel_facts() {
    let dir = std::env::temp_dir();
    let lulcc = colm_namelist::parse("&nl_colm\n DEF_USE_LULCC=.true.\n/\n").unwrap();
    let err = super::validate_runtime_contract(
        &lulcc,
        &dir,
        Some(super::KernelFacts {
            single: false,
            usgs: true,
            crop: false,
        }),
    )
    .unwrap_err();
    assert!(err.contains("USGS"), "{err}");

    let crop = colm_namelist::parse("&nl_colm\n/\n").unwrap();
    let err = super::validate_runtime_contract(
        &crop,
        &dir,
        Some(super::KernelFacts {
            single: false,
            usgs: false,
            crop: true,
        }),
    )
    .unwrap_err();
    assert!(err.contains("CROP") && err.contains("BGC"), "{err}");
}

#[test]
fn crop_management_runtime_files_are_checked_before_write() {
    let root = std::env::temp_dir().join(format!("colm-crop-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("crop")).unwrap();
    let ndep = root.join("ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
    std::fs::create_dir_all(ndep.parent().unwrap()).unwrap();
    std::fs::write(ndep, []).unwrap();
    let facts = Some(super::KernelFacts {
        single: true,
        usgs: false,
        crop: true,
    });
    let doc = |fields: &str| {
        colm_namelist::parse(&format!(
            "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_BGC=.true.\n DEF_USE_NITRIF=.false.\n DEF_dir_runtime='{}'\n {fields}\n/\n",
            root.display()
        ))
        .unwrap()
    };

    super::validate_runtime_contract(
        &doc(
            "DEF_TUNING_CROP_PLANTING_DAY=120.\n DEF_USE_FERT=.false.\n DEF_USE_IRRIGATION=.false.",
        ),
        &root,
        facts,
    )
    .unwrap();

    let err = super::validate_runtime_contract(
        &doc("DEF_USE_FERT=.false.\n DEF_USE_IRRIGATION=.false."),
        &root,
        facts,
    )
    .unwrap_err();
    assert!(err.contains("CROP 播种日期"), "{err}");
    std::fs::write(root.join("crop/plantdt-colm-64cfts-rice2_fillcoast.nc"), []).unwrap();

    let source_one = doc("DEF_USE_FERT=.true.\n DEF_USE_IRRIGATION=.false.\n DEF_FERT_SOURCE=1");
    let err = super::validate_runtime_contract(&source_one, &root, facts).unwrap_err();
    assert!(err.contains("fertnitro_fillcoast.nc"), "{err}");
    std::fs::write(root.join("crop/fertnitro_fillcoast.nc"), []).unwrap();
    super::validate_runtime_contract(&source_one, &root, facts).unwrap();

    let source_two = doc("DEF_USE_FERT=.true.\n DEF_USE_IRRIGATION=.false.\n DEF_FERT_SOURCE=2");
    let err = super::validate_runtime_contract(&source_two, &root, facts).unwrap_err();
    assert!(err.contains("fertilizer_2015soc.nc"), "{err}");
    std::fs::write(root.join("crop/fertilizer_2015soc.nc"), []).unwrap();
    super::validate_runtime_contract(&source_two, &root, facts).unwrap();

    let allocation_one =
        doc("DEF_USE_FERT=.false.\n DEF_USE_IRRIGATION=.true.\n DEF_IRRIGATION_ALLOCATION=1");
    let err = super::validate_runtime_contract(&allocation_one, &root, facts).unwrap_err();
    assert!(
        err.contains("surfdata_irrigation_method_96x144.nc"),
        "{err}"
    );
    std::fs::write(root.join("crop/surfdata_irrigation_method_96x144.nc"), []).unwrap();
    super::validate_runtime_contract(&allocation_one, &root, facts).unwrap();

    let allocation_three =
        doc("DEF_USE_FERT=.false.\n DEF_USE_IRRIGATION=.true.\n DEF_IRRIGATION_ALLOCATION=3");
    let err = super::validate_runtime_contract(&allocation_three, &root, facts).unwrap_err();
    assert!(err.contains("surfdata_irrigation_allocation.nc"), "{err}");
    std::fs::write(root.join("crop/surfdata_irrigation_allocation.nc"), []).unwrap();
    super::validate_runtime_contract(&allocation_three, &root, facts).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bgc_runtime_contract_checks_selected_ndep_nitrif_and_fire_inputs() {
    let root = std::env::temp_dir().join(format!("colm-bgc-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let doc = |fields: &str| {
        colm_namelist::parse(&format!(
            "&nl_colm\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n DEF_USE_BGC=.true.\n DEF_dir_runtime='{}'\n {fields}\n/\n",
            root.display()
        ))
        .unwrap()
    };

    let err =
        super::validate_runtime_contract(&doc("DEF_USE_NITRIF=.false."), &root, None).unwrap_err();
    assert!(err.contains("氮沉降"), "{err}");
    let annual = root.join("ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
    std::fs::create_dir_all(annual.parent().unwrap()).unwrap();
    std::fs::write(annual, []).unwrap();
    super::validate_runtime_contract(&doc("DEF_USE_NITRIF=.false."), &root, None).unwrap();

    let monthly = root.join("ndep/fndep_colm_monthly.nc");
    let err = super::validate_runtime_contract(
        &doc("DEF_NDEP_FREQUENCY=2\n DEF_USE_NITRIF=.false."),
        &root,
        None,
    )
    .unwrap_err();
    assert!(err.contains("fndep_colm_monthly.nc"), "{err}");
    std::fs::write(monthly, []).unwrap();

    let err = super::validate_runtime_contract(
        &doc("DEF_NDEP_FREQUENCY=2\n DEF_USE_NITRIF=.false.\n DEF_USE_FIRE=.true."),
        &root,
        None,
    )
    .unwrap_err();
    assert!(err.contains("abm_colm_double_fillcoast.nc"), "{err}");
    for name in [
        "fire/abm_colm_double_fillcoast.nc",
        "fire/peatf_colm_360x720_c100428.nc",
        "fire/gdp_colm_360x720_c100428.nc",
        "fire/colmforc.Li_2017_HYDEv3.2_CMIP6_hdm_0.5x0.5_AVHRR_simyr1850-2016_c180202.nc",
        "fire/clmforc.Li_2012_climo1995-2011.T62.lnfm_Total_c140423.nc",
    ] {
        let file = root.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, []).unwrap();
    }
    super::validate_runtime_contract(
        &doc("DEF_NDEP_FREQUENCY=2\n DEF_USE_NITRIF=.false.\n DEF_USE_FIRE=.true."),
        &root,
        None,
    )
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn child_paths_are_not_written_to_inactive_cases_in_a_batch() {
    let on = "&nl_colm
 DEF_USE_SoilInit=.true.
 DEF_file_SoilInit='null'
/
";
    let off = "&nl_colm
 DEF_USE_SoilInit=.false.
/
";
    let dirs = batch("mixed-path", &[on, off]);
    let file = std::path::Path::new(&dirs[0]).join("soil-init.nc");
    std::fs::write(&file, "stub").unwrap();
    let err = set_batch(dirs, "DEF_file_SoilInit".into(), file.display().to_string()).unwrap_err();
    assert!(err.contains("case1") && err.contains("土壤初始场"), "{err}");
}

#[test]
fn enabled_picker_paths_must_exist_and_match_kind() {
    let dir = batch("contract-path", &[NML_A]).remove(0);
    let missing = std::env::temp_dir().join("colm-missing-soil-init.nc");
    let err = set_batches(
        vec![dir.clone()],
        vec![
            FieldChange {
                path: "DEF_USE_SoilInit".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_file_SoilInit".into(),
                value: missing.display().to_string(),
            },
        ],
    )
    .unwrap_err();
    assert!(err.contains("DEF_file_SoilInit"), "{err}");

    let file = std::path::Path::new(&dir).join("soil-init.nc");
    std::fs::write(&file, "stub").unwrap();
    set_batches(
        vec![dir],
        vec![
            FieldChange {
                path: "DEF_USE_SoilInit".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_file_SoilInit".into(),
                value: file.display().to_string(),
            },
        ],
    )
    .unwrap();
}

#[test]
fn write_all_rolls_back_files_already_written_on_late_failure() {
    let dirs = batch("write-rollback", &[NML_A, NML_B]);
    let first = std::path::Path::new(&dirs[0]).join("case.nml");
    let second = std::path::Path::new(&dirs[1]).join("case.nml");
    let before = std::fs::read_to_string(&first).unwrap();
    let original_permissions = std::fs::metadata(&second).unwrap().permissions();
    let mut perms = original_permissions.clone();
    perms.set_readonly(true);
    std::fs::set_permissions(&second, perms).unwrap();

    let err = super::write_all(&[
        (dirs[0].clone(), "first was changed".into()),
        (dirs[1].clone(), "second cannot be changed".into()),
    ])
    .unwrap_err();

    std::fs::set_permissions(&second, original_permissions).unwrap();
    assert!(err.contains("case.nml"), "{err}");
    assert_eq!(std::fs::read_to_string(first).unwrap(), before);
}

#[test]
fn a_batch_write_that_cannot_finish_writes_nothing() {
    // 半批配置好的算例与整批配置好的在界面上长得一样，而它们跑出来的
    // 东西不一样 —— 所以宁可一份都不写。
    let dirs = batch("nothing", &[NML_A, "&nl_colm\n   这不是 namelist"]);
    let before = std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap();
    let e = set_batch(dirs.clone(), "DEF_HIST_FREQ".into(), "MONTHLY".into())
        .expect_err("坏文件必须让整批失败");
    assert!(e.contains("case1"), "错误要说清是哪一个：{e}");
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap(),
        before,
        "第一个算例不该被动过"
    );
}

#[test]
fn wizard_fields_are_written_together() {
    let dir = batch("wizard", &[SAMPLE]).remove(0);
    let runtime = std::path::Path::new(&dir).join("runtime");
    let ndep = runtime.join("ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
    std::fs::create_dir_all(ndep.parent().unwrap()).unwrap();
    std::fs::write(ndep, []).unwrap();
    super::apply_fields(
        &dir,
        &[
            FieldChange {
                path: "DEF_USE_PFT".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_LCT".into(),
                value: ".false.".into(),
            },
            FieldChange {
                path: "DEF_USE_BGC".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_NITRIF".into(),
                value: ".false.".into(),
            },
            FieldChange {
                path: "DEF_dir_runtime".into(),
                value: runtime.display().to_string(),
            },
            FieldChange {
                path: "DEF_TRACER_NUM".into(),
                value: "1".into(),
            },
            FieldChange {
                path: "DEF_TRACER_NAMES".into(),
                value: "CH4".into(),
            },
            FieldChange {
                path: "DEF_TRACER_TYPES".into(),
                value: "gas".into(),
            },
            FieldChange {
                path: "DEF_TRACER_PARAM_FILES".into(),
                value: "CH4:standard_ch4_parameter.nml".into(),
            },
        ],
    )
    .unwrap();
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("case.nml")).unwrap();
    let doc = colm_namelist::parse(&text).unwrap();
    for (name, value) in [
        ("DEF_USE_PFT", ".true."),
        ("DEF_USE_LCT", ".false."),
        ("DEF_USE_BGC", ".true."),
        ("DEF_TRACER_NUM", "1"),
        ("DEF_TRACER_NAMES", "'CH4'"),
        ("DEF_TRACER_TYPES", "'gas'"),
        ("DEF_TRACER_PARAM_FILES", "'CH4:standard_ch4_parameter.nml'"),
    ] {
        assert_eq!(doc.get(name).unwrap().to_string(), value, "{name}");
    }
}

#[test]
fn methane_wizard_stages_safe_single_point_parameters() {
    let dir = batch("wizard-ch4", &[SAMPLE]).remove(0);
    super::apply_fields(
        &dir,
        &[FieldChange {
            path: "DEF_TRACER_PARAM_FILES".into(),
            value: "CH4:standard_ch4_parameter.nml".into(),
        }],
    )
    .unwrap();
    let ch4 =
        std::fs::read_to_string(std::path::Path::new(&dir).join("standard_ch4_parameter.nml"))
            .unwrap();
    assert!(ch4.contains("DEF_METHANE%inundation_mode  = 'wetwat'"));
    assert!(ch4.contains("DEF_METHANE%enable_rice_paddy = .false."));
    assert!(ch4.contains("DEF_METHANE%use_spatial_ph   = .false."));
    assert!(ch4.contains("DEF_METHANE%write_ch4_history = .true."));
}

#[test]
fn expert_process_parameters_are_read_from_case_local_files() {
    let dir = batch("expert-process", &[SAMPLE]).remove(0);
    super::apply_fields(
        &dir,
        &[FieldChange {
            path: "DEF_TRACER_PARAM_FILES".into(),
            value: "CH4:standard_ch4_parameter.nml".into(),
        }],
    )
    .unwrap();
    let files = process_parameter_files(dir.clone()).unwrap();
    let ch4 = files
        .iter()
        .find(|file| file.title == "standard_ch4_parameter.nml")
        .expect("CH4 parameter file");
    assert_eq!(ch4.section, "示踪剂");
    let q10 = ch4
        .entries
        .iter()
        .find(|entry| entry.path == "DEF_METHANE%q10methane")
        .expect("q10 methane");
    assert_eq!(q10.value, "2.0");
    assert_eq!(q10.default.as_deref(), Some("2."));
    let mode = ch4
        .entries
        .iter()
        .find(|entry| entry.path == "DEF_METHANE%inundation_mode")
        .expect("inundation mode");
    assert_eq!(mode.value, "'wetwat'");
    assert_eq!(mode.default.as_deref(), Some("'hybrid'"));
    let biome = ch4
        .entries
        .iter()
        .find(|entry| entry.path == "DEF_METHANE%use_biome_f_methane")
        .expect("biome methane yield");
    assert_eq!(biome.value, ".true.");
    assert_eq!(biome.default.as_deref(), Some(".false."));
    let scalar = ch4
        .entries
        .iter()
        .find(|entry| entry.path == "DEF_METHANE%f_methane")
        .expect("omitted code-default scalar");
    assert!(scalar.unset);
    assert_eq!(scalar.value, "0.2");
    let hydrology = ch4
        .entries
        .iter()
        .find(|entry| entry.path == "DEF_METHANE_hydrology%vdcf")
        .expect("omitted hydrology default");
    assert!(hydrology.unset);
    assert_eq!(hydrology.default.as_deref(), Some("2."));
}

#[test]
fn expert_process_parameter_writes_only_that_case_file() {
    let dir = batch("expert-process-write", &[SAMPLE]).remove(0);
    super::apply_fields(
        &dir,
        &[FieldChange {
            path: "DEF_TRACER_PARAM_FILES".into(),
            value: "CH4:standard_ch4_parameter.nml".into(),
        }],
    )
    .unwrap();
    let r = set_process_parameter_field_batch(
        vec![dir.clone()],
        "standard_ch4_parameter.nml".into(),
        "DEF_METHANE%f_methane".into(),
        "0.25".into(),
    )
    .unwrap();
    assert_eq!(r.written, 1);
    let text =
        std::fs::read_to_string(std::path::Path::new(&dir).join("standard_ch4_parameter.nml"))
            .unwrap();
    assert!(text.contains("DEF_METHANE%f_methane = 0.25"));
    assert!(
        std::fs::read_to_string(std::path::Path::new(&dir).join("case.nml"))
            .unwrap()
            .contains("CN-Cng")
    );
}

#[test]
fn expert_process_parameter_batch_write_updates_all_cases() {
    let dirs = batch("expert-process-batch-write", &[SAMPLE, SAMPLE]);
    for dir in &dirs {
        super::apply_fields(
            dir,
            &[FieldChange {
                path: "DEF_TRACER_PARAM_FILES".into(),
                value: "CH4:standard_ch4_parameter.nml".into(),
            }],
        )
        .unwrap();
    }
    let r = set_process_parameter_field_batch(
        dirs.clone(),
        "standard_ch4_parameter.nml".into(),
        "DEF_METHANE%f_methane".into(),
        "0.31".into(),
    )
    .unwrap();
    assert_eq!(r.written, 2);
    for dir in &dirs {
        let text =
            std::fs::read_to_string(std::path::Path::new(dir).join("standard_ch4_parameter.nml"))
                .unwrap();
        assert!(
            text.contains("DEF_METHANE%f_methane = 0.31"),
            "{dir}: {text}"
        );
    }
}

#[test]
fn expert_core_tuning_batch_write_updates_all_cases() {
    let dirs = batch("expert-core-batch-write", &[SAMPLE, SAMPLE]);
    let r = set_batch(dirs.clone(), "DEF_TUNING_ZLND".into(), "0.02".into()).unwrap();
    assert_eq!(r.written, 2);
    for dir in dirs {
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join("case.nml")).unwrap();
        assert!(text.contains("DEF_TUNING_ZLND = 0.02"), "{dir}: {text}");
    }
}

#[test]
fn expert_process_parameter_batch_write_is_all_or_nothing() {
    let dirs = batch("expert-process-batch-atomic", &[SAMPLE, SAMPLE]);
    for dir in &dirs {
        super::apply_fields(
            dir,
            &[FieldChange {
                path: "DEF_TRACER_PARAM_FILES".into(),
                value: "CH4:standard_ch4_parameter.nml".into(),
            }],
        )
        .unwrap();
    }
    let first = std::path::Path::new(&dirs[0]).join("standard_ch4_parameter.nml");
    let before = std::fs::read_to_string(&first).unwrap();
    std::fs::remove_file(std::path::Path::new(&dirs[1]).join("standard_ch4_parameter.nml"))
        .unwrap();
    let err = set_process_parameter_field_batch(
        dirs.clone(),
        "standard_ch4_parameter.nml".into(),
        "DEF_METHANE%f_methane".into(),
        "0.41".into(),
    )
    .unwrap_err();
    assert!(err.contains("standard_ch4_parameter.nml"), "{err}");
    assert_eq!(std::fs::read_to_string(first).unwrap(), before);
}

#[test]
fn every_standard_process_parameter_has_a_fortran_code_default() {
    let known: std::collections::BTreeSet<String> = super::process_code_defaults()
        .into_iter()
        .map(|field| field.path.to_ascii_lowercase())
        .collect();
    for (name, text) in [
        (
            "CH4",
            include_str!("../../../vendor/CoLM202X/run/standard_ch4_parameter.nml"),
        ),
        (
            "chloride",
            include_str!("../../../vendor/CoLM202X/run/standard_chloride_parameter.nml"),
        ),
        (
            "HDO",
            include_str!("../../../vendor/CoLM202X/run/standard_HDO_parameter.nml"),
        ),
        (
            "O18",
            include_str!("../../../vendor/CoLM202X/run/standard_O18_parameter.nml"),
        ),
        (
            "sediment",
            include_str!("../../../vendor/CoLM202X/run/standard_sediment_parameter.nml"),
        ),
    ] {
        let doc = colm_namelist::parse(text).unwrap();
        let missing: Vec<_> = doc
            .paths()
            .into_iter()
            .filter(|path| !known.contains(&path.to_ascii_lowercase()))
            .collect();
        assert!(missing.is_empty(), "{name}: {missing:?}");
    }
}

#[test]
fn process_group_not_filename_decides_the_expert_page() {
    let dir = batch("expert-group", &[SAMPLE]).remove(0);
    super::apply_fields(
        &dir,
        &[FieldChange {
            path: "DEF_TRACER_PARAM_FILES".into(),
            value: "CH4:standard_ch4_parameter.nml".into(),
        }],
    )
    .unwrap();
    let source = std::path::Path::new(&dir).join("standard_ch4_parameter.nml");
    let opaque = std::path::Path::new(&dir).join("opaque_parameter.nml");
    std::fs::copy(source, &opaque).unwrap();
    let file = super::process_entries(&opaque, "opaque_parameter.nml".into()).unwrap();
    assert_eq!(file.section, "示踪剂");
}

#[test]
fn process_writes_validate_the_fortran_type_not_a_malformed_file_value() {
    let dir = batch("expert-code-type", &[SAMPLE]).remove(0);
    super::apply_fields(
        &dir,
        &[FieldChange {
            path: "DEF_TRACER_PARAM_FILES".into(),
            value: "CH4:standard_ch4_parameter.nml".into(),
        }],
    )
    .unwrap();
    let path = std::path::Path::new(&dir).join("standard_ch4_parameter.nml");
    let original = std::fs::read_to_string(&path).unwrap();
    let malformed = original.replace(
        "DEF_METHANE%q10methane       = 2.0",
        "DEF_METHANE%q10methane       = 'wrong'",
    );
    std::fs::write(&path, &malformed).unwrap();
    let err = set_process_parameter_field_batch(
        vec![dir],
        "standard_ch4_parameter.nml".into(),
        "DEF_METHANE%q10methane".into(),
        "still-wrong".into(),
    )
    .unwrap_err();
    assert!(err.contains("实数"), "{err}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), malformed);
}

#[test]
fn invalid_wizard_field_leaves_the_case_unchanged() {
    let dir = batch("wizard-invalid", &[SAMPLE]).remove(0);
    let path = std::path::Path::new(&dir).join("case.nml");
    let before = std::fs::read_to_string(&path).unwrap();
    let err = super::apply_fields(
        &dir,
        &[
            FieldChange {
                path: "DEF_USE_PFT".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_RangeCheck".into(),
                value: "yes".into(),
            },
        ],
    )
    .unwrap_err();
    assert!(err.contains("logical"), "{err}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[test]
fn urban_classification_rejects_values_outside_ncar_and_lcz() {
    let err = super::typed("DEF_URBAN_type_scheme", "-18").unwrap_err();
    assert!(err.contains("1, 2"), "{err}");
    assert_eq!(
        super::typed("DEF_URBAN_type_scheme", "2").unwrap(),
        colm_namelist::Value::Int(2)
    );
}

#[test]
fn fields_the_batch_disagrees_on_are_reported() {
    let dirs = batch("varies", &[NML_A, NML_B]);
    let v = super::varying_fields(dirs).unwrap();
    assert!(v.iter().any(|p| p == "DEF_HIST_FREQ"), "{v:?}");
    assert!(
        v.iter().any(|p| p == "DEF_simulation_time%start_year"),
        "{v:?}"
    );

    // 单个算例永远没有分歧 —— 而不是"每个字段都算分歧"。
    assert!(super::varying_fields(batch("alone", &[NML_A]))
        .unwrap()
        .is_empty());
}

#[test]
fn spin_up_is_computed_per_case_not_shared() {
    // 各站点的强迫场起点不同。用同一个绝对年份会让一部分算例的预热落在
    // 窗口之外（等于没预热），另一部分落得过深（等于把输出砍掉一大截）。
    let dirs = batch("spinup", &[NML_A, NML_B]);
    set_test_spinup(dirs.clone(), 1, 10).unwrap();
    let year = |d: &str| {
        let t = std::fs::read_to_string(std::path::Path::new(d).join("case.nml")).unwrap();
        t.lines()
            .find(|l| l.contains("spinup_year"))
            .map(|l| l.split('=').nth(1).unwrap().trim().to_string())
            .expect("写了 spinup_year")
    };
    assert_eq!(year(&dirs[0]), "2003", "起点 2002 的算例预热到 2003");
    assert_eq!(year(&dirs[1]), "2006", "起点 2005 的算例预热到 2006");

    // 读回来：窗口不一致要报出来，预热设置一致则不报。
    let t = super::read_timing(dirs.clone()).unwrap();
    assert_eq!(t.count, 2);
    assert!(t.window_varies, "两个算例的窗口本来就不同");
    assert!(!t.spinup_varies);
    assert_eq!(t.spinup_years, 1);
    assert_eq!(t.spinup_repeat, 10);
    // 输出从预热结束处才开始 —— 这是预热的代价，界面必须能说出来。
    assert_eq!(t.output_start, "2003-01-01");
}

#[test]
fn spinup_that_covers_the_whole_window_is_rejected_without_erasing_the_old_value() {
    let dirs = batch("spinup-too-long", &[NML_B]);
    let path = std::path::Path::new(&dirs[0]).join("case.nml");
    let before = std::fs::read_to_string(&path).unwrap();

    let err = set_test_spinup(dirs, 3, 10).unwrap_err();
    assert!(err.contains("预热截止时间必须早于模拟结束时间"), "{err}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[test]
fn one_spinup_cycle_is_not_erased() {
    let dirs = batch("spinup_one", &[NML_A]);
    set_test_spinup(dirs.clone(), 1, 1).unwrap();
    let t = super::read_timing(dirs).unwrap();
    assert_eq!(t.spinup_years, 1);
    assert_eq!(t.spinup_repeat, 1);
    assert_eq!(t.output_start, "2003-01-01");
}

#[test]
fn either_zero_spinup_input_still_means_disabled() {
    let dirs = batch("spinup_zero_repeat", &[NML_B]);
    set_test_spinup(dirs.clone(), 30, 0).unwrap();
    let t = super::read_timing(dirs).unwrap();
    assert_eq!((t.spinup_years, t.spinup_repeat), (0, 0));
}

#[test]
fn total_model_steps_include_every_spinup_cycle() {
    let doc = colm_namelist::parse(
        "&nl_colm\n\
         DEF_simulation_time%start_year = 1992\n\
         DEF_simulation_time%start_month = 12\n\
         DEF_simulation_time%start_day = 31\n\
         DEF_simulation_time%start_sec = 84600\n\
         DEF_simulation_time%end_year = 2004\n\
         DEF_simulation_time%end_month = 11\n\
         DEF_simulation_time%end_day = 28\n\
         DEF_simulation_time%end_sec = 45000\n\
         DEF_simulation_time%spinup_year = 1993\n\
         DEF_simulation_time%spinup_month = 12\n\
         DEF_simulation_time%spinup_day = 31\n\
         DEF_simulation_time%spinup_sec = 84600\n\
         DEF_simulation_time%spinup_repeat = 10\n\
         DEF_simulation_time%timestep = 1800.\n/\n",
    )
    .unwrap();
    assert_eq!(super::one_timing(&doc).5, 366_458);
}

#[test]
fn every_source_namelist_field_has_a_named_ui_section() {
    // 参数页不再用「其他」兜底。上游新增字段时应该在 CI 里
    // 点名，迫使我们读它的 namelist 语义后再归类，而不是随手堆进杂物箱。
    let missing: Vec<_> = colm_schema::all()
        .iter()
        .filter(|f| !f.name.starts_with("DEF_hist_vars%"))
        .filter(|f| super::field_section(f.name, f.group).is_none())
        .map(|f| f.name)
        .collect();
    assert!(missing.is_empty(), "还没归类的源码字段：{missing:?}");
}

#[test]
fn forcing_namelist_path_is_used_by_colm_and_stays_visible() {
    let f = colm_schema::find("DEF_forcing_namelist").expect("源码里还有这个字段");
    assert_eq!(super::field_section(f.name, f.group), Some("文件与目录"));

    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/share/MOD_Namelist.F90");
    let Ok(text) = std::fs::read_to_string(source) else {
        return; // 未取 submodule 时由 schema drift 测试负责
    };
    assert!(
        text.contains("file=trim(DEF_forcing_namelist)"),
        "CoLM 已不再用 DEF_forcing_namelist，重新评估是否从界面和算例里删掉"
    );
}

#[test]
fn kernel_macros_decide_which_parameters_are_relevant() {
    let default = ["SinglePoint"].into_iter().collect();
    let with_da = ["SinglePoint", "DataAssimilation"].into_iter().collect();
    let with_river = ["SinglePoint", "GridRiverLakeFlow"].into_iter().collect();

    let relevant = |name, have| {
        let f = colm_schema::find(name).expect(name);
        super::field_is_relevant(f, have)
    };
    assert!(relevant("DEF_CASE_NAME", &default));
    assert!(relevant("DEF_dir_output", &default));
    // LULC/BGC/CROP/URBAN/LULCC 那组改造之后：URBAN_MODEL 与 BGC 不再是
    // 编译期宏（main/URBAN/、main/BGC/ 始终编译进去，DEF_URBAN_RUN/
    // DEF_USE_BGC 在 MOD_Namelist.F90 里改成运行时开关），所以
    // `DEF_URBAN_RUN`/`DEF_USE_CN_INIT` 现在在**每个**内核下都相关——
    // 不再有一个「URBAN_MODEL 没编进去」或「BGC 没编进去」的内核让它们
    // 变得不相关，用户设了就会在运行时生效（或不生效，那是 DEF_URBAN_RUN/
    // DEF_USE_BGC 本身的值决定的，不是这个函数管的编译期相关性）。
    assert!(relevant("DEF_URBAN_RUN", &default));
    assert!(relevant("DEF_USE_CN_INIT", &default));
    assert!(relevant("DEF_USE_TRACER", &default));
    // DataAssimilation 仍然是真正的编译期宏（这组改造没有碰它），
    // 用来验证「宏决定相关性」这条机制本身还成立。
    assert!(!relevant("DEF_DA_TWS", &default));
    assert!(relevant("DEF_DA_TWS", &with_da));
    // 上游不止一项漏了 requires；单点内核没有任何河网过程时，整个分栏
    // 必须为空，不能让任意一个漏标字段把它撑出来。
    for field in colm_schema::all()
        .iter()
        .filter(|field| super::field_section(field.name, field.group) == Some("河道与水库"))
    {
        assert!(
            !super::field_is_relevant(field, &default),
            "单点内核仍显示河道字段：{}",
            field.name
        );
    }
    assert!(!relevant("DEF_ElementNeighbour_file", &default));
    assert!(!relevant("DEF_Reservoir_Method", &default));
    assert!(relevant("DEF_ElementNeighbour_file", &with_river));
    assert!(relevant("DEF_Reservoir_Method", &with_river));
    for field in colm_schema::all()
        .iter()
        .filter(|field| super::field_section(field.name, field.group) == Some("网格与并行"))
    {
        assert!(
            !super::field_is_relevant(field, &default),
            "单点内核仍显示网格/MPI 字段：{}",
            field.name
        );
    }
}

fn runtime_states(text: &str, macros: &[&str]) -> Vec<FieldState> {
    let have = macros.iter().copied().collect();
    super::field_states_for(text, &have).expect("runtime field states")
}

fn mode<'a>(states: &'a [FieldState], name: &str) -> &'a FieldMode {
    &states
        .iter()
        .find(|state| state.name == name)
        .expect(name)
        .mode
}

fn runtime_state<'a>(states: &'a [FieldState], name: &str) -> &'a FieldState {
    states.iter().find(|state| state.name == name).expect(name)
}

fn runtime_state_mut<'a>(states: &'a mut [FieldState], name: &str) -> &'a mut FieldState {
    states
        .iter_mut()
        .find(|state| state.name == name)
        .expect(name)
}

#[test]
fn natural_lct_singlepoint_hides_unreachable_and_overwritten_fields() {
    let states = runtime_states(
        "&nl_colm\n\
         SITE_landtype = 10\n\
         DEF_USE_LCT = .true.\n\
         DEF_USE_PFT = .false.\n\
         DEF_USE_PC = .false.\n\
         DEF_USE_Campbell_SOIL_MODEL = .false.\n\
         DEF_USE_BGC = .false.\n\
         DEF_URBAN_RUN = .false.\n\
         DEF_USE_TRACER = .false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "SITE_fsitedata",
        "SITE_lon_location",
        "SITE_lat_location",
        "SITE_landtype",
        "USE_SITE_landtype",
        "USE_SITE_pctpfts",
        "USE_SITE_pctcrop",
        "USE_SITE_lakedepth",
        "USE_SITE_dbedrock",
        "USE_srfdata_from_larger_region",
        "DEF_dir_existing_srfdata",
        "USE_srfdata_from_3D_gridded_data",
        "DEF_SOLO_PFT",
        "DEF_FAST_PC",
        "DEF_PC_CROP_SPLIT",
        "DEF_SUBGRID_SCHEME",
        "DEF_LANDONLY",
        "DEF_USE_DOMINANT_PATCHTYPE",
        "DEF_USE_SOILPAR_UPS_FIT",
        "USE_zip_for_aggregation",
        "DEF_Srfdata_CompressLevel",
        "DEF_USE_LAIFEEDBACK",
        "DEF_LULCC_SCHEME",
        "DEF_HighResSoil",
        "DEF_HighResVeg",
        "DEF_PROSPECT",
        "DEF_HighResUrban_albedo",
        "DEF_RSS_SCHEME",
        "DEF_USE_VariablySaturatedFlow",
        "DEF_TOPMOD_method",
        "DEF_USE_Dynamic_Lake",
        "DEF_USE_Dynamic_Wetland",
        "DEF_Forcing_Interp_Method",
        "DEF_HISTORY_IN_VECTOR",
        "DEF_HIST_grid_as_forcing",
        "DEF_HIST_lon_res",
        "DEF_HIST_lat_res",
        "DEF_HIST_mode",
        "DEF_HIST_WriteBack",
        "DEF_HIST_CompressLevel",
        "DEF_USE_SrfdataDiag",
        "DEF_USE_ClimForcing_for_Spinup",
    ] {
        assert_eq!(mode(&states, name), &FieldMode::Hidden, "{name}");
    }
    for name in [
        "USE_SITE_htop",
        "USE_SITE_LAI",
        "DEF_USE_SoilInit",
        "DEF_USE_SnowInit",
        "DEF_USE_WaterTableInit",
        "DEF_USE_Forcing_Downscaling",
        "DEF_USE_Forcing_Downscaling_Simple",
        "DEF_HIST_FREQ",
        "DEF_WRST_FREQ",
        "USE_SITE_HistWriteBack",
    ] {
        assert_eq!(mode(&states, name), &FieldMode::Editable, "{name}");
    }
    let hidden_in_main_pages = states
        .iter()
        .filter(|state| state.mode == FieldMode::Hidden)
        .filter(|state| {
            let field = colm_schema::find(&state.name).expect(&state.name);
            matches!(
                super::field_section(field.name, field.group),
                Some("站点" | "地表数据" | "初始场" | "强迫场" | "水热过程" | "输出与重启")
            )
        })
        .count();
    assert!(
        hidden_in_main_pages >= 46,
        "单点自然站仍只隐藏了 {hidden_in_main_pages} 项"
    );
}

#[test]
fn singlepoint_pc_hides_fast_pc_because_that_switch_is_not_in_the_site_path() {
    let states = runtime_states(
        "&nl_colm\n\
         SITE_landtype = 10\n\
         DEF_USE_LCT = .false.\n\
         DEF_USE_PFT = .false.\n\
         DEF_USE_PC = .true.\n\
         DEF_URBAN_RUN = .false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );

    assert_eq!(mode(&states, "DEF_FAST_PC"), &FieldMode::Hidden);
}

#[test]
fn child_fields_follow_initial_forcing_runoff_and_process_switches() {
    let off = runtime_states(
        "&nl_colm\n\
         DEF_USE_PFT = .true.\n\
         DEF_USE_LCT = .false.\n\
         DEF_USE_BGC = .true.\n\
         DEF_USE_SoilInit = .false.\n\
         DEF_USE_SnowInit = .false.\n\
         DEF_USE_CN_INIT = .false.\n\
         DEF_USE_WaterTableInit = .false.\n\
         DEF_USE_Forcing_Downscaling = .false.\n\
         DEF_USE_Forcing_Downscaling_Simple = .false.\n\
         DEF_Runoff_SCHEME = 3\n\
         DEF_USE_SNICAR = .false.\n\
         DEF_USE_OZONESTRESS = .false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_file_SoilInit",
        "DEF_file_SnowInit",
        "DEF_file_cn_init",
        "DEF_file_WaterTable",
        "DEF_DS_HiresTopographyDataDir",
        "DEF_DS_precipitation_adjust_scheme",
        "DEF_DS_longwave_adjust_scheme",
        "DEF_VIC_OPT",
        "DEF_file_VIC_para",
        "DEF_file_VIC_OPT",
        "DEF_Aerosol_Readin",
        "DEF_Aerosol_Clim",
        "DEF_USE_OZONEDATA",
        "DEF_file_Ozone",
    ] {
        assert_eq!(mode(&off, name), &FieldMode::Hidden, "{name}");
    }

    let on = runtime_states(
        "&nl_colm\n\
         DEF_USE_PFT = .true.\n\
         DEF_USE_LCT = .false.\n\
         DEF_USE_BGC = .true.\n\
         DEF_USE_SoilInit = .true.\n\
         DEF_USE_SnowInit = .true.\n\
         DEF_USE_CN_INIT = .true.\n\
         DEF_USE_WaterTableInit = .true.\n\
         DEF_USE_Forcing_Downscaling = .true.\n\
         DEF_Runoff_SCHEME = 1\n\
         DEF_VIC_OPT = .true.\n\
         DEF_USE_SNICAR = .true.\n\
         DEF_Aerosol_Readin = .true.\n\
         DEF_USE_OZONESTRESS = .true.\n\
         DEF_USE_OZONEDATA = .false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_file_SoilInit",
        "DEF_file_SnowInit",
        "DEF_file_cn_init",
        "DEF_DS_HiresTopographyDataDir",
        "DEF_DS_precipitation_adjust_scheme",
        "DEF_DS_longwave_adjust_scheme",
        "DEF_VIC_OPT",
        "DEF_USE_OZONEDATA",
        "DEF_Aerosol_Readin",
        "DEF_Aerosol_Clim",
    ] {
        assert_eq!(mode(&on, name), &FieldMode::Editable, "{name}");
    }
    assert_eq!(mode(&on, "DEF_file_Ozone"), &FieldMode::Hidden);
    let ozone_file = runtime_states(
        "&nl_colm\n DEF_USE_OZONESTRESS=.true.\n DEF_USE_OZONEDATA=.true.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&ozone_file, "DEF_file_Ozone"), &FieldMode::Editable);
    // SoilInit wins over a separate water-table file; VIC parameter paths are
    // derived from DEF_dir_runtime by MOD_Namelist.
    for name in [
        "DEF_file_WaterTable",
        "DEF_file_VIC_para",
        "DEF_file_VIC_OPT",
    ] {
        assert_eq!(mode(&on, name), &FieldMode::Hidden, "{name}");
    }

    let simple = runtime_states(
        "&nl_colm\n\
         DEF_USE_Forcing_Downscaling = .false.\n\
         DEF_USE_Forcing_Downscaling_Simple = .true.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(
        mode(&simple, "DEF_DS_HiresTopographyDataDir"),
        &FieldMode::Hidden,
        "简化降尺度使用 CoLM 自带地形数据，不应要求额外目录"
    );
}

#[test]
fn methane_hides_isotope_only_process_parameters() {
    let states = runtime_states(
        "&nl_colm\n\
         DEF_USE_TRACER=.true.\n\
         DEF_TRACER_TYPES='gas'\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_TRACER_USE_FRACTIONATION",
        "DEF_TRACER_KINETIC_SCHEME",
        "DEF_TRACER_SOIL_DIFFUSION",
        "DEF_TRACER_SOIL_INIT_FILE",
    ] {
        assert_eq!(mode(&states, name), &FieldMode::Hidden, "{name}");
    }
}

#[test]
fn singlepoint_surface_fields_follow_the_actual_lai_and_albedo_sources() {
    assert_eq!(
        super::field_section("DEF_USE_LAIFEEDBACK", Some("nl_colm")),
        Some("生态与生地化")
    );

    let from_site = runtime_states(
        "&nl_colm\n\
         USE_SITE_LAI=.true.\n\
         DEF_USE_BGC=.false.\n\
         DEF_SOIL_REFL_SCHEME=2\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_LC_YEAR",
        "DEF_LAI_START_YEAR",
        "DEF_LAI_END_YEAR",
        "DEF_LAI_MONTHLY",
        "DEF_LAI_CHANGE_YEARLY",
    ] {
        assert_eq!(mode(&from_site, name), &FieldMode::Hidden, "{name}");
    }
    assert_eq!(
        mode(&from_site, "DEF_SOIL_REFL_SCHEME"),
        &FieldMode::Editable
    );
    assert_eq!(
        mode(&from_site, "USE_SITE_soilreflectance"),
        &FieldMode::Editable
    );

    let static_fallback = runtime_states(
        "&nl_colm\n\
         USE_SITE_LAI=.false.\n\
         DEF_LAI_CHANGE_YEARLY=.false.\n\
         DEF_SOIL_REFL_SCHEME=1\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in ["DEF_LC_YEAR", "DEF_LAI_MONTHLY", "DEF_LAI_CHANGE_YEARLY"] {
        assert_eq!(mode(&static_fallback, name), &FieldMode::Editable, "{name}");
    }
    for name in [
        "DEF_LAI_START_YEAR",
        "DEF_LAI_END_YEAR",
        "USE_SITE_soilreflectance",
    ] {
        assert_eq!(mode(&static_fallback, name), &FieldMode::Hidden, "{name}");
    }

    let yearly_fallback = runtime_states(
        "&nl_colm\n\
         USE_SITE_LAI=.false.\n\
         DEF_LAI_CHANGE_YEARLY=.true.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&yearly_fallback, "DEF_LC_YEAR"), &FieldMode::Hidden);
    assert_eq!(
        mode(&yearly_fallback, "DEF_LAI_START_YEAR"),
        &FieldMode::Editable
    );
    assert_eq!(
        mode(&yearly_fallback, "DEF_LAI_END_YEAR"),
        &FieldMode::Editable
    );
}

#[test]
fn water_balance_equilibrium_is_not_a_bgc_only_control() {
    assert_eq!(
        super::field_section("DEF_CheckEquilibrium", Some("nl_colm")),
        Some("水热过程")
    );
    let states = runtime_states(
        "&nl_colm\n SITE_landtype=10\n DEF_USE_BGC=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&states, "DEF_CheckEquilibrium"), &FieldMode::Editable);
}

#[test]
fn subgrid_bgc_crop_urban_and_landtype_constraints_are_composed() {
    let pft_bgc = runtime_states(
        "&nl_colm\n\
         SITE_landtype = 10\n\
         DEF_USE_LCT = .false.\n\
         DEF_USE_PFT = .true.\n\
         DEF_USE_PC = .false.\n\
         DEF_USE_BGC = .true.\n\
         DEF_URBAN_RUN = .false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&pft_bgc, "USE_SITE_pctpfts"), &FieldMode::Editable);
    assert_eq!(mode(&pft_bgc, "DEF_USE_NITRIF"), &FieldMode::Editable);
    assert_eq!(mode(&pft_bgc, "DEF_USE_CN_INIT"), &FieldMode::Editable);
    for crop_only in [
        "USE_SITE_pctcrop",
        "DEF_USE_FERT",
        "DEF_FERT_SOURCE",
        "DEF_USE_CNSOYFIXN",
        "DEF_USE_IRRIGATION",
    ] {
        assert_eq!(mode(&pft_bgc, crop_only), &FieldMode::Hidden, "{crop_only}");
    }

    let pc_crop_urban = runtime_states(
        "&nl_colm\n\
         SITE_landtype = 13\n\
         DEF_USE_LCT = .false.\n\
         DEF_USE_PFT = .false.\n\
         DEF_USE_PC = .true.\n\
         DEF_USE_BGC = .true.\n\
         DEF_URBAN_RUN = .true.\n/\n",
        &["SinglePoint", "LULC_IGBP", "CROP"],
    );
    // 城市单点只有 urban patch；即使内核带 PC/CROP/BGC，也没有 PFT/crop patch
    // 可以消费这些比例和生地化子过程。
    for natural_only in [
        "DEF_PC_CROP_SPLIT",
        "USE_SITE_pctpfts",
        "USE_SITE_pctcrop",
        "DEF_USE_FERT",
        "DEF_USE_NITRIF",
    ] {
        assert_eq!(
            mode(&pc_crop_urban, natural_only),
            &FieldMode::Hidden,
            "{natural_only}"
        );
    }
    assert_eq!(mode(&pc_crop_urban, "DEF_URBAN_BEM"), &FieldMode::Editable);
    assert_eq!(mode(&pc_crop_urban, "DEF_URBAN_ONLY"), &FieldMode::Hidden);
    for forced_off in [
        "DEF_VEG_SNOW",
        "DEF_USE_MEDLYNST",
        "DEF_USE_WUEST",
        "DEF_USE_SUPERCOOL_WATER",
        "DEF_USE_PLANTHYDRAULICS",
        "DEF_USE_OZONESTRESS",
        "DEF_USE_OZONEDATA",
        "DEF_SPLIT_SOILSNOW",
    ] {
        assert_eq!(
            mode(&pc_crop_urban, forced_off),
            &FieldMode::Hidden,
            "{forced_off}"
        );
    }
}

#[test]
fn ozone_is_leaf_physiology_not_a_bgc_only_process() {
    let natural = runtime_states(
        "&nl_colm\n SITE_landtype=1\n DEF_USE_BGC=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&natural, "DEF_USE_OZONESTRESS"), &FieldMode::Editable);
    let lake = runtime_states(
        "&nl_colm\n SITE_landtype=17\n DEF_USE_BGC=.true.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_USE_OZONESTRESS",
        "DEF_USE_OZONEDATA",
        "DEF_USE_MEDLYNST",
        "DEF_USE_WUEST",
        "DEF_BALL_BERRY_GRADM",
        "DEF_BALL_BERRY_BINTER",
        "DEF_MEDLYN_G1",
        "DEF_MEDLYN_G0",
        "DEF_WUE_LAMBDA",
        "DEF_VEG_SNOW",
    ] {
        assert_eq!(mode(&lake, name), &FieldMode::Hidden, "{name}");
    }
}

#[test]
fn stomatal_tuning_fields_follow_the_selected_scheme() {
    for (name, text, visible) in [
        (
            "Ball–Berry",
            "&nl_colm\n SITE_landtype=1\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.false.\n/\n",
            ["DEF_BALL_BERRY_GRADM", "DEF_BALL_BERRY_BINTER"].as_slice(),
        ),
        (
            "Medlyn",
            "&nl_colm\n SITE_landtype=1\n DEF_USE_MEDLYNST=.true.\n DEF_USE_WUEST=.false.\n/\n",
            ["DEF_MEDLYN_G1", "DEF_MEDLYN_G0"].as_slice(),
        ),
        (
            "WUE",
            "&nl_colm\n SITE_landtype=1\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.true.\n/\n",
            ["DEF_WUE_LAMBDA"].as_slice(),
        ),
    ] {
        let states = runtime_states(text, &["SinglePoint", "LULC_IGBP"]);
        for field in [
            "DEF_BALL_BERRY_GRADM",
            "DEF_BALL_BERRY_BINTER",
            "DEF_MEDLYN_G1",
            "DEF_MEDLYN_G0",
            "DEF_WUE_LAMBDA",
        ] {
            let expected = if visible.contains(&field) {
                &FieldMode::Editable
            } else {
                &FieldMode::Hidden
            };
            assert_eq!(mode(&states, field), expected, "{name}: {field}");
        }
    }

    let ball_berry = runtime_states(
        "&nl_colm\n SITE_landtype=1\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    let medlyn = runtime_states(
        "&nl_colm\n SITE_landtype=1\n DEF_USE_MEDLYNST=.true.\n DEF_USE_WUEST=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    let mixed = super::merge_field_states(&[ball_berry, medlyn]);
    for field in [
        "DEF_BALL_BERRY_GRADM",
        "DEF_BALL_BERRY_BINTER",
        "DEF_MEDLYN_G1",
        "DEF_MEDLYN_G0",
        "DEF_WUE_LAMBDA",
    ] {
        assert_eq!(mode(&mixed, field), &FieldMode::Hidden, "mixed: {field}");
    }

    let conflict = runtime_states(
        "&nl_colm\n SITE_landtype=1\n DEF_USE_MEDLYNST=.true.\n DEF_USE_WUEST=.true.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for field in [
        "DEF_BALL_BERRY_GRADM",
        "DEF_BALL_BERRY_BINTER",
        "DEF_MEDLYN_G1",
        "DEF_MEDLYN_G0",
        "DEF_WUE_LAMBDA",
    ] {
        assert_eq!(
            mode(&conflict, field),
            &FieldMode::Hidden,
            "conflict: {field}"
        );
    }
}

#[test]
fn land_cover_expert_defaults_follow_classification_and_site_landtype() {
    let igbp = runtime_states(
        "&nl_colm
 SITE_landtype=1
 DEF_USE_LCT=.true.
 DEF_USE_PFT=.false.
 DEF_USE_PC=.false.
 DEF_USE_PLANTHYDRAULICS=.true.
 DEF_USE_MEDLYNST=.false.
 DEF_USE_WUEST=.false.
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    let default = |states: &[FieldState], name: &str| {
        parse_real(
            runtime_state(states, name)
                .context_default
                .as_deref()
                .unwrap_or_else(|| panic!("missing contextual default for {name}")),
        )
        .expect("numeric contextual default")
    };
    assert_eq!(mode(&igbp, "DEF_LC_HTOP0"), &FieldMode::Editable);
    assert_eq!(default(&igbp, "DEF_LC_HTOP0"), 17.0);
    assert_eq!(default(&igbp, "DEF_LC_VMAX25"), 54.0);
    assert_eq!(default(&igbp, "DEF_BALL_BERRY_GRADM"), 9.0);
    assert_eq!(default(&igbp, "DEF_LC_KMAX_SUN"), 2.0e-8);
    assert_eq!(
        runtime_state(&igbp, "DEF_LC_C3C4").allowed_values,
        ["0", "1"]
    );

    let usgs = runtime_states(
        "&nl_colm
 SITE_landtype=13
 DEF_USE_LCT=.true.
 DEF_USE_PLANTHYDRAULICS=.true.
/
",
        &["SinglePoint", "LULC_USGS"],
    );
    assert_eq!(default(&usgs, "DEF_LC_HTOP0"), 35.0);
    assert_eq!(default(&usgs, "DEF_LC_VMAX25"), 72.0);

    let pft = runtime_states(
        "&nl_colm
 SITE_landtype=1
 DEF_USE_LCT=.false.
 DEF_USE_PFT=.true.
 DEF_USE_PC=.false.
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&pft, "DEF_LC_HTOP0"), &FieldMode::Hidden);
    assert!(runtime_state(&pft, "DEF_LC_HTOP0")
        .context_default
        .is_none());

    let no_hydraulics = runtime_states(
        "&nl_colm
 SITE_landtype=1
 DEF_USE_LCT=.true.
 DEF_USE_PLANTHYDRAULICS=.false.
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&no_hydraulics, "DEF_LC_HTOP0"), &FieldMode::Editable);
    assert_eq!(mode(&no_hydraulics, "DEF_LC_KMAX_SUN"), &FieldMode::Hidden);
}

#[test]
fn all_sites_keeps_land_cover_defaults_editable_but_marks_different_tables() {
    let first = runtime_states(
        "&nl_colm
 SITE_landtype=1
 DEF_USE_LCT=.true.
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    let second = runtime_states(
        "&nl_colm
 SITE_landtype=2
 DEF_USE_LCT=.true.
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    let merged = super::merge_field_states(&[first, second]);
    let height = runtime_state(&merged, "DEF_LC_HTOP0");
    assert_eq!(height.mode, FieldMode::Editable);
    assert!(
        !height.mixed,
        "different defaults are not applicability conflicts"
    );
    assert!(height.default_mixed);
    assert!(!runtime_state(&merged, "DEF_LC_Z0MR").default_mixed);
}

#[test]
fn land_cover_expert_override_batch_write_is_validated_and_atomic() {
    let dirs = batch(
        "lc-expert-all",
        &[
            "&nl_colm\n SITE_landtype=1\n/\n",
            "&nl_colm\n SITE_landtype=2\n/\n",
        ],
    );
    let written = set_batch(dirs.clone(), "DEF_LC_VMAX25".into(), "60".into()).unwrap();
    assert_eq!(written.written, 2);
    for dir in &dirs {
        let text = std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap();
        assert!(text.contains("DEF_LC_VMAX25 = 60"), "{dir}: {text}");
    }

    let before: Vec<_> = dirs
        .iter()
        .map(|dir| std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap())
        .collect();
    let error = set_batch(dirs.clone(), "DEF_LC_FVEG0".into(), "1.1".into()).unwrap_err();
    assert!(error.contains("DEF_LC_FVEG0"), "{error}");
    for (dir, expected) in dirs.iter().zip(before) {
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap(),
            expected
        );
    }
}

#[test]
fn pft_expert_defaults_and_sparse_batch_overrides_use_fortran_slots() {
    const CASE: &str = "&nl_colm\n SITE_landtype=10\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n DEF_USE_BGC=.false.\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.false.\n/\n";
    let dirs = batch("pft-expert-all", &[CASE, CASE]);
    let kernel = pft_test_kernel("states");
    let kernel_dir = kernel.display().to_string();

    let states = pft_parameter_states(dirs.clone(), 13, kernel_dir.clone()).unwrap();
    let height = states
        .iter()
        .find(|state| state.name == "DEF_PFT_HTOP0")
        .expect("PFT height");
    assert_eq!(parse_real(&height.default), Some(0.5));
    assert!(states.iter().any(|state| state.name == "DEF_PFT_GRADM"));
    assert!(!states.iter().any(|state| state.name == "DEF_PFT_G1"));

    let written = set_pft_parameter_batch(
        dirs.clone(),
        13,
        "DEF_PFT_VMAX25".into(),
        Some("60".into()),
        kernel_dir.clone(),
    )
    .unwrap();
    assert_eq!(written.written, 2);
    for dir in &dirs {
        let text = std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap();
        assert!(text.contains("DEF_PFT_VMAX25(14) = 60"), "{dir}: {text}");
    }

    set_pft_parameter_batch(
        dirs.clone(),
        13,
        "DEF_PFT_VMAX25".into(),
        None,
        kernel_dir.clone(),
    )
    .unwrap();
    for dir in &dirs {
        let text = std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap();
        assert!(!text.contains("DEF_PFT_VMAX25"), "{dir}: {text}");
    }

    let err = set_pft_parameter_batch(
        dirs.clone(),
        13,
        "DEF_PFT_MXMAT".into(),
        Some("1e20".into()),
        kernel_dir.clone(),
    )
    .unwrap_err();
    assert!(err.contains("i32") || err.contains("Fortran"), "{err}");

    let before: Vec<_> = dirs
        .iter()
        .map(|dir| std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap())
        .collect();
    assert!(set_pft_parameter_batch(
        dirs.clone(),
        13,
        "DEF_PFT_SQRTDI".into(),
        Some("0".into()),
        kernel_dir,
    )
    .is_err());
    for (dir, expected) in dirs.iter().zip(before) {
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(dir).join("case.nml")).unwrap(),
            expected
        );
    }
}

#[test]
fn a_case_wide_stomatal_override_hides_the_ignored_pft_coefficient() {
    let dirs = batch(
        "pft-global-stomata",
        &["&nl_colm\n SITE_landtype=10\n DEF_USE_LCT=.false.\n DEF_USE_PFT=.true.\n DEF_USE_PC=.false.\n DEF_USE_MEDLYNST=.false.\n DEF_USE_WUEST=.false.\n DEF_BALL_BERRY_GRADM=9.5\n/\n"],
    );
    let kernel = pft_test_kernel("global-stomata");
    let states = pft_parameter_states(dirs, 13, kernel.display().to_string()).unwrap();
    assert!(!states.iter().any(|state| state.name == "DEF_PFT_GRADM"));
    assert!(states.iter().any(|state| state.name == "DEF_PFT_BINTER"));
}

#[test]
fn pft_expert_visibility_follows_pft_structure_and_available_defaults() {
    let natural_doc = colm_namelist::parse(
        "&nl_colm\n SITE_landtype=10\n DEF_USE_BGC=.true.\n DEF_USE_PFT=.true.\n/\n",
    )
    .unwrap();
    let natural_macros = ["SinglePoint", "LULC_IGBP"].into_iter().collect();
    let natural = VisibilityContext::new(&natural_doc, &natural_macros);
    let meta = |name| colm_case::pft::parameter(name).unwrap();

    assert!(colm_case::pft::all_parameters()
        .iter()
        .all(|parameter| !pft_parameter_applies(parameter, &natural, 0)));
    assert!(pft_parameter_applies(meta("DEF_PFT_LIVEWDCN"), &natural, 1));
    assert!(!pft_parameter_applies(
        meta("DEF_PFT_LIVEWDCN"),
        &natural,
        13
    ));
    assert!(pft_parameter_applies(
        meta("DEF_PFT_STEM_LEAF"),
        &natural,
        1
    ));
    assert!(!pft_parameter_applies(
        meta("DEF_PFT_STEM_LEAF"),
        &natural,
        13
    ));
    assert!(pft_parameter_has_default(meta("DEF_PFT_PSI50_ROOT"), &natural, 13).unwrap());

    let crop_doc = colm_namelist::parse(
        "&nl_colm\n SITE_landtype=12\n DEF_USE_BGC=.true.\n DEF_USE_PFT=.true.\n DEF_USE_FERT=.true.\n DEF_FERT_SOURCE=1\n/\n",
    )
    .unwrap();
    let crop_macros = ["SinglePoint", "LULC_IGBP", "CROP"].into_iter().collect();
    let crop = VisibilityContext::new(&crop_doc, &crop_macros);
    assert!(pft_parameter_applies(meta("DEF_PFT_LFEMERG"), &crop, 15));
    assert!(pft_parameter_applies(meta("DEF_PFT_LFEMERG"), &crop, 16));
    assert!(pft_parameter_applies(meta("DEF_PFT_LFEMERG"), &crop, 17));
    assert!(pft_parameter_applies(meta("DEF_PFT_MANURE"), &crop, 15));
    let no_manure_doc = colm_namelist::parse(
        "&nl_colm\n DEF_USE_BGC=.true.\n DEF_USE_FERT=.true.\n DEF_FERT_SOURCE=2\n/\n",
    )
    .unwrap();
    let no_manure = VisibilityContext::new(&no_manure_doc, &crop_macros);
    assert!(!pft_parameter_applies(
        meta("DEF_PFT_MANURE"),
        &no_manure,
        15
    ));
    assert!(pft_parameter_has_default(meta("DEF_PFT_LFEMERG"), &crop, 17).unwrap());
    assert!(!pft_parameter_has_default(meta("DEF_PFT_LFEMERG"), &crop, 33).unwrap());
}

#[test]
fn expert_core_tuning_fields_follow_runtime_switches() {
    let natural = runtime_states(
        "&nl_colm
\
         SITE_landtype=1
\
         DEF_Runoff_SCHEME=3
\
         DEF_USE_BGC=.false.
\
         DEF_USE_PLANTHYDRAULICS=.true.
\
         DEF_USE_OZONESTRESS=.true.
\
         DEF_USE_Forcing_Downscaling=.false.
\
         DEF_USE_Forcing_Downscaling_Simple=.false.
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_TUNING_ZLND",
        "DEF_TUNING_CAPR",
        "DEF_TUNING_CSOILC",
        "DEF_TUNING_SMPMAX",
        "DEF_TUNING_SOIL_ICE_IMPEDANCE",
        "DEF_TUNING_SIMPLE_VIC_DS",
        "DEF_TUNING_SIMPLE_VIC_WS",
        "DEF_TUNING_SNOW_COVER_EXPONENT",
        "DEF_PH_ROOT_RADIUS",
        "DEF_OZONE_KO3",
    ] {
        assert_eq!(mode(&natural, name), &FieldMode::Editable, "{name}");
    }
    for name in [
        "DEF_TUNING_WETWATMAX",
        "DEF_TUNING_SMPMAX_HR",
        "DEF_TUNING_TOPMOD_DECAY",
        "DEF_TUNING_IRRIGATION_START_SEC",
        "DEF_DS_TEMP_LAPSE_RATE",
        "DEF_DS_SHORTWAVE_LIMIT",
        "DEF_DS_SHORTWAVE_SIMPLE_LIMIT",
    ] {
        assert_eq!(mode(&natural, name), &FieldMode::Hidden, "{name}");
    }

    let lake = runtime_states(
        "&nl_colm
 SITE_landtype=17
 DEF_USE_PLANTHYDRAULICS=.true.
 DEF_USE_OZONESTRESS=.true.
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_TUNING_CSOILC",
        "DEF_TUNING_SOIL_ICE_IMPEDANCE",
        "DEF_TUNING_SIMPLE_VIC_DS",
        "DEF_TUNING_SIMPLE_VIC_WS",
        "DEF_PH_ROOT_RADIUS",
        "DEF_OZONE_KO3",
    ] {
        assert_eq!(mode(&lake, name), &FieldMode::Hidden, "{name}");
    }

    let dry_lake = runtime_states(
        "&nl_colm
 SITE_landtype=17
 DEF_USE_Dynamic_Lake=.true.
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(
        mode(&dry_lake, "DEF_TUNING_SOIL_ICE_IMPEDANCE"),
        &FieldMode::Editable
    );

    let urban = runtime_states(
        "&nl_colm
 SITE_landtype=13
 DEF_URBAN_RUN=.true.
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&urban, "DEF_TUNING_CSOILC"), &FieldMode::Editable);
    assert_eq!(
        mode(&urban, "DEF_TUNING_SOIL_ICE_IMPEDANCE"),
        &FieldMode::Editable,
        "urban pervious ground uses WATER_2014 soil hydraulics"
    );

    let wetland = runtime_states(
        "&nl_colm
 SITE_landtype=11
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(mode(&wetland, "DEF_TUNING_WETWATMAX"), &FieldMode::Editable);

    let dynamic_wetland = runtime_states(
        "&nl_colm
 SITE_landtype=1
 DEF_USE_Dynamic_Wetland=.true.
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(
        mode(&dynamic_wetland, "DEF_TUNING_WETWATMAX"),
        &FieldMode::Editable
    );

    let full = runtime_states(
        "&nl_colm
 DEF_USE_Forcing_Downscaling=.true.
 DEF_USE_Forcing_Downscaling_Simple=.false.
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_DS_TEMP_LAPSE_RATE",
        "DEF_DS_LONGWAVE_LIMIT",
        "DEF_DS_SHORTWAVE_LIMIT",
    ] {
        assert_eq!(mode(&full, name), &FieldMode::Editable, "{name}");
    }
    assert_eq!(
        mode(&full, "DEF_DS_SHORTWAVE_SIMPLE_LIMIT"),
        &FieldMode::Hidden
    );
    assert_eq!(
        mode(&full, "DEF_DS_LONGWAVE_LAPSE_RATE"),
        &FieldMode::Hidden,
        "a non-glacier site does not use the glacier longwave lapse rate"
    );

    let glacier_longwave_ii = runtime_states(
        "&nl_colm
 SITE_landtype=15
 DEF_USE_Forcing_Downscaling=.true.
 DEF_DS_longwave_adjust_scheme='II'
/
",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(
        mode(&glacier_longwave_ii, "DEF_DS_LONGWAVE_LAPSE_RATE"),
        &FieldMode::Editable
    );

    let simple = runtime_states(
        "&nl_colm
 DEF_USE_Forcing_Downscaling=.false.
 DEF_USE_Forcing_Downscaling_Simple=.true.
/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    for name in [
        "DEF_DS_TEMP_LAPSE_RATE",
        "DEF_DS_LONGWAVE_LIMIT",
        "DEF_DS_SHORTWAVE_SIMPLE_LIMIT",
    ] {
        assert_eq!(mode(&simple, name), &FieldMode::Editable, "{name}");
    }
    assert_eq!(mode(&simple, "DEF_DS_SHORTWAVE_LIMIT"), &FieldMode::Hidden);

    let irrigated_crop = runtime_states(
        "&nl_colm
 SITE_landtype=12
 DEF_USE_IRRIGATION=.true.
/
",
        &["SinglePoint", "LULC_IGBP", "CROP"],
    );
    for name in [
        "DEF_TUNING_CROP_PLANTING_DAY",
        "DEF_TUNING_IRRIGATION_START_SEC",
        "DEF_TUNING_IRRIGATION_DURATION_SEC",
        "DEF_TUNING_IRRIGATION_MAX_DEPTH",
        "DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION",
        "DEF_TUNING_IRRIGATION_SUPPLY_FRACTION",
        "DEF_TUNING_IRRIGATION_MIN_CPHASE",
        "DEF_TUNING_IRRIGATION_MAX_CPHASE",
        "DEF_TUNING_IRRIGATION_PONDMX",
    ] {
        assert_eq!(mode(&irrigated_crop, name), &FieldMode::Editable, "{name}");
    }
}

#[test]
fn expert_core_tuning_validation_matches_fortran_limits() {
    let ok = colm_namelist::parse(
        "&nl_colm
\
         DEF_TUNING_ZLND=.01
\
         DEF_TUNING_CAPR=2.0
\
         DEF_TUNING_CNFAC=.5
\
         DEF_TUNING_WIMP=.5
\
         DEF_TUNING_SMPMAX=-10.
\
         DEF_TUNING_SMPMIN=-20.
\
         DEF_TUNING_SMPMAX_HR=-2.
\
         DEF_TUNING_SMPMIN_HR=-3.
\
         DEF_TUNING_SIMPLE_VIC_DS=.1
\
         DEF_TUNING_SIMPLE_VIC_WS=.6
\
         DEF_TUNING_IRRIGATION_START_SEC=21600.
\
         DEF_TUNING_IRRIGATION_DURATION_SEC=3600.
\
         DEF_TUNING_IRRIGATION_MIN_CPHASE=1.
\
         DEF_TUNING_IRRIGATION_MAX_CPHASE=4.
\
         DEF_TUNING_CROP_PLANTING_DAY=120.
\
         DEF_PH_ROOT_RADIUS=2.9e-4
\
         DEF_OZONE_KO3=0.
\
         DEF_DS_TEMP_LAPSE_RATE=0.
\
         DEF_DS_SHORTWAVE_LIMIT=1.
/\n",
    )
    .unwrap();
    validate_expert_tuning(
        &ok,
        [
            "DEF_TUNING_ZLND",
            "DEF_TUNING_CAPR",
            "DEF_TUNING_CNFAC",
            "DEF_TUNING_WIMP",
            "DEF_TUNING_SMPMAX",
            "DEF_TUNING_SMPMIN",
            "DEF_TUNING_SMPMAX_HR",
            "DEF_TUNING_SMPMIN_HR",
            "DEF_TUNING_SIMPLE_VIC_DS",
            "DEF_TUNING_SIMPLE_VIC_WS",
            "DEF_TUNING_IRRIGATION_START_SEC",
            "DEF_TUNING_IRRIGATION_DURATION_SEC",
            "DEF_TUNING_IRRIGATION_MIN_CPHASE",
            "DEF_TUNING_IRRIGATION_MAX_CPHASE",
            "DEF_TUNING_CROP_PLANTING_DAY",
            "DEF_PH_ROOT_RADIUS",
            "DEF_OZONE_KO3",
            "DEF_DS_TEMP_LAPSE_RATE",
            "DEF_DS_SHORTWAVE_LIMIT",
        ]
        .into_iter()
        .map(String::from),
    )
    .unwrap();

    for (name, value, message) in [
        ("DEF_TUNING_ZLND", "0.", "大于 0"),
        ("DEF_TUNING_CNFAC", "1.1", "0 到 1"),
        ("DEF_TUNING_WIMP", "1.", "小于 1"),
        ("DEF_TUNING_SIMPLE_VIC_DS", "0.", "大于 0"),
        ("DEF_TUNING_IRRIGATION_START_SEC", "86400.", "86400（不含）"),
        ("DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION", "1.1", "0 到 1"),
        ("DEF_TUNING_CROP_PLANTING_DAY", "367.", "1 到 366"),
        ("DEF_TUNING_CROP_PLANTING_DAY", "120.5", "整数"),
        ("DEF_OZONE_KO3", "-0.1", "不小于 0"),
        ("DEF_DS_TEMP_LAPSE_RATE", "-0.1", "不小于 0"),
    ] {
        let text = format!("&nl_colm\n {name}={value}\n/\n");
        let doc = colm_namelist::parse(&text).unwrap();
        let err = validate_expert_tuning(&doc, [name.to_string()]).unwrap_err();
        assert!(err.contains(message), "{name}: {err}");
    }

    let bad_pair = colm_namelist::parse(
        "&nl_colm
 DEF_TUNING_SMPMAX=-20.
 DEF_TUNING_SMPMIN=-10.
/\n",
    )
    .unwrap();
    let err = validate_expert_tuning(
        &bad_pair,
        ["DEF_TUNING_SMPMAX".into(), "DEF_TUNING_SMPMIN".into()],
    )
    .unwrap_err();
    assert!(err.contains("DEF_TUNING_SMPMIN"), "{err}");

    for (text, names, message) in [
        (
            "&nl_colm\n DEF_TUNING_SIMPLE_VIC_DS=.8\n DEF_TUNING_SIMPLE_VIC_WS=.6\n/\n",
            ["DEF_TUNING_SIMPLE_VIC_DS", "DEF_TUNING_SIMPLE_VIC_WS"],
            "小于等于",
        ),
        (
            "&nl_colm\n DEF_TUNING_IRRIGATION_MIN_CPHASE=3.\n DEF_TUNING_IRRIGATION_MAX_CPHASE=2.\n/\n",
            [
                "DEF_TUNING_IRRIGATION_MIN_CPHASE",
                "DEF_TUNING_IRRIGATION_MAX_CPHASE",
            ],
            "起始作物阶段",
        ),
    ] {
        let doc = colm_namelist::parse(text).unwrap();
        let err = validate_expert_tuning(&doc, names.into_iter().map(String::from)).unwrap_err();
        assert!(err.contains(message), "{err}");
    }
}

#[test]
fn interception_choices_follow_the_kernel_file_selection_macro() {
    let extended = runtime_states(
        "&nl_colm\n/\n",
        &["SinglePoint", "LULC_IGBP", "extend_interception"],
    );
    assert_eq!(
        runtime_state(&extended, "DEF_Interception_scheme").allowed_values,
        ["1", "2", "3", "4", "5", "6", "7", "8"]
    );

    let fallback = runtime_states("&nl_colm\n/\n", &["SinglePoint", "LULC_IGBP"]);
    assert_eq!(
        runtime_state(&fallback, "DEF_Interception_scheme").allowed_values,
        ["1"]
    );
}

#[test]
fn lake_wetland_and_hyperspectral_fields_follow_actual_capabilities() {
    // IGBP constants in MOD_Vars_Global.F90: WATERBODY=17, WETLAND=11.
    for (landtype, visible, hidden) in [
        (17, "DEF_USE_Dynamic_Lake", "DEF_USE_Dynamic_Wetland"),
        (11, "DEF_USE_Dynamic_Wetland", "DEF_USE_Dynamic_Lake"),
    ] {
        let text = format!("&nl_colm\n SITE_landtype={landtype}\n/\n");
        let states = runtime_states(&text, &["SinglePoint", "LULC_IGBP"]);
        assert_eq!(
            mode(&states, visible),
            &FieldMode::Editable,
            "landtype {landtype}"
        );
        assert_eq!(
            mode(&states, hidden),
            &FieldMode::Hidden,
            "landtype {landtype}"
        );
    }
    let regular = runtime_states("&nl_colm\n/\n", &["SinglePoint", "LULC_IGBP"]);
    let spectral = runtime_states(
        "&nl_colm\n DEF_URBAN_RUN=.true.\n/\n",
        &["SinglePoint", "LULC_IGBP", "HYPERSPECTRAL"],
    );
    for name in [
        "DEF_HighResSoil",
        "DEF_HighResVeg",
        "DEF_PROSPECT",
        "DEF_HighResUrban_albedo",
    ] {
        assert_eq!(mode(&regular, name), &FieldMode::Hidden, "{name}");
    }
    for name in [
        "DEF_HighResSoil",
        "DEF_HighResVeg",
        "DEF_PROSPECT",
        "DEF_HighResUrban_albedo",
    ] {
        assert_eq!(mode(&spectral, name), &FieldMode::Editable, "{name}");
    }
}

#[test]
fn runtime_choices_prevent_invalid_singlepoint_combinations() {
    let states = runtime_states(
        "&nl_colm\n\
         DEF_USE_Forcing_Downscaling=.true.\n\
         DEF_USE_Forcing_Downscaling_Simple=.false.\n\
         DEF_USE_MEDLYNST=.true.\n\
         DEF_USE_WUEST=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    assert_eq!(
        runtime_state(&states, "DEF_USE_Forcing_Downscaling_Simple").allowed_values,
        [".false."]
    );
    assert_eq!(
        runtime_state(&states, "DEF_DS_precipitation_adjust_scheme").allowed_values,
        ["I", "II"]
    );
    assert_eq!(
        runtime_state(&states, "DEF_USE_WUEST").allowed_values,
        [".false."]
    );
}

#[test]
fn batch_visibility_uses_every_case_instead_of_a_representative() {
    let bgc_off = runtime_states(
        "&nl_colm\n\
         DEF_USE_LCT=.false.\n\
         DEF_USE_PFT=.true.\n\
         DEF_USE_BGC=.false.\n\
         DEF_USE_SoilInit=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    let bgc_on = runtime_states(
        "&nl_colm\n\
         DEF_USE_LCT=.false.\n\
         DEF_USE_PFT=.true.\n\
         DEF_USE_BGC=.true.\n\
         DEF_USE_SoilInit=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    let merged = super::merge_field_states(&[bgc_off, bgc_on]);

    let nitrif = runtime_state(&merged, "DEF_USE_NITRIF");
    assert_eq!(nitrif.mode, FieldMode::Editable);
    assert!(nitrif.mixed, "批量条件差异必须显式警告");
    assert_eq!(
        mode(&merged, "DEF_file_SoilInit"),
        &FieldMode::Hidden,
        "所有算例都不使用的子字段才应隐藏"
    );
}

#[test]
fn batch_runtime_choices_are_intersected_and_empty_intersections_are_locked() {
    let full = runtime_states(
        "&nl_colm\n\
         DEF_USE_Forcing_Downscaling=.true.\n\
         DEF_USE_Forcing_Downscaling_Simple=.false.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    let simple = runtime_states(
        "&nl_colm\n\
         DEF_USE_Forcing_Downscaling=.false.\n\
         DEF_USE_Forcing_Downscaling_Simple=.true.\n/\n",
        &["SinglePoint", "LULC_IGBP"],
    );
    let merged = super::merge_field_states(&[full, simple]);
    assert_eq!(
        runtime_state(&merged, "DEF_USE_Forcing_Downscaling").allowed_values,
        [".false."]
    );
    assert_eq!(
        runtime_state(&merged, "DEF_USE_Forcing_Downscaling_Simple").allowed_values,
        [".false."]
    );

    // 人工构造两个互斥的合法值，锁定“空交集不能被误解为无限制”的语义。
    let mut left = runtime_states("&nl_colm\n/\n", &["SinglePoint", "LULC_IGBP"]);
    let mut right = left.clone();
    runtime_state_mut(&mut left, "DEF_USE_WUEST").allowed_values = vec![".true."];
    runtime_state_mut(&mut right, "DEF_USE_WUEST").allowed_values = vec![".false."];
    let merged = super::merge_field_states(&[left, right]);
    let wuest = runtime_state(&merged, "DEF_USE_WUEST");
    assert_eq!(wuest.mode, FieldMode::Disabled);
    assert!(wuest.allowed_values.is_empty());
    assert!(wuest.mixed);
    assert!(wuest.reason.unwrap().contains("没有共同合法值"));
}
