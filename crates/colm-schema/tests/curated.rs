//! 人工列出的宏依赖，每一条都要能证明自己还成立。
//!
//! `xtask/src/usage.rs` 的 `CURATED` 里每条都带出处（文件 + 那一行必须包含
//! 的文本）。手工表一定会烂 —— **除非它自己能发现自己烂了**。
//! 上游把那个守护挪走时，这里红；而不是界面悄悄多显示一个没用的字段。

use std::path::PathBuf;

/// 与 `xtask/src/usage.rs::CURATED` 保持一致。两处各写一份是因为 xtask 是
/// 二进制 crate，测试拿不到它的 const —— 由下面第二条测试把两份拴住。
///
/// **目前是空的。** 原先唯一的一条——`DEF_URBAN_type_scheme` 需要
/// `URBAN_MODEL`——在 LULC/BGC/CROP/URBAN/LULCC 那组改造里失效了：
/// `landurban_build` 的调用点从 `#ifdef URBAN_MODEL` 改成了运行时
/// `IF (DEF_URBAN_RUN) THEN`（mksrfdata/MKSRFDATA.F90），`URBAN_MODEL`
/// 本身也从 `include/define.h` 里彻底消失。详见 `xtask/src/usage.rs`
/// 的 `CURATED` 注释。
const CURATED: &[(&str, &str, &str, &str)] = &[];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn every_curated_gate_still_points_at_a_real_guard() {
    let root = repo().join("vendor/CoLM202X");
    for (field, macro_, file, needle) in CURATED {
        let path = root.join(file);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return, // submodule 没取下来就跳过
        };
        let lines: Vec<&str> = text.lines().collect();
        let at = lines.iter().position(|l| l.contains(needle));
        let Some(at) = at else {
            panic!("{field}: {file} 里已经找不到 {needle:?} —— 出处失效了，重新查证");
        };
        // 那一行往上找最近的 #ifdef，必须是所声明的宏。
        let guard = lines[..at]
            .iter()
            .rev()
            .find_map(|l| {
                let t = l.trim_start();
                if t.starts_with("#endif") {
                    return Some(String::new()); // 中间有闭合块，说明没被守
                }
                t.strip_prefix("#ifdef ").map(|m| m.trim().to_string())
            })
            .unwrap_or_default();
        assert_eq!(
            guard, *macro_,
            "{field}: {file} 里 {needle:?} 上方最近的守护是 {guard:?}，\
             而人工表声明的是 {macro_:?}。上游改了条件，这条要重新查证。"
        );
    }
}

#[test]
fn the_generated_table_agrees_with_the_curated_one() {
    // 人工表在 xtask 里，生成结果在 colm-schema 里。两边对不上就说明
    // 生成器没把人工表并进去 —— 而那种失败是静默的：表还在，只是没生效。
    for (field, macro_, _, _) in CURATED {
        let f = colm_schema::find(field).unwrap_or_else(|| panic!("schema 里没有 {field}"));
        assert!(
            f.requires.contains(macro_),
            "{field} 的 requires 是 {:?}，不含人工表声明的 {macro_}",
            f.requires
        );
    }
}

#[test]
fn field_names_in_diagnostics_do_not_create_fake_macro_dependencies() {
    // MOD_Mesh prints DEF_ny_blocks in an MPI-only help string, but the field
    // itself configures serial and single-point meshes too. String literals are
    // diagnostics, not executable field uses.
    assert!(colm_schema::find("DEF_ny_blocks")
        .expect("DEF_ny_blocks")
        .requires
        .is_empty());
}

#[test]
fn vendored_source_keeps_upstream_numeric_fixes() {
    let root = repo().join("vendor/CoLM202X");
    let topo = std::fs::read_to_string(root.join("mksrfdata/Aggregation_TopographyFactors.F90"))
        .expect("Aggregation_TopographyFactors.F90");
    assert!(topo.contains("index = 2"));

    let mapping = std::fs::read_to_string(root.join("share/MOD_SpatialMapping.F90"))
        .expect("MOD_SpatialMapping.F90");
    assert!(mapping.contains(".and.(sumdata%blk(xblk,yblk)%val /= 0.)"));

    let generator = std::fs::read_to_string(root.join(".github/workflows/create_defineh.bash"))
        .expect("create_defineh.bash");
    let here_doc = generator
        .split_once("cat>include/define.h<<EOF")
        .expect("define.h heredoc")
        .1;
    assert!(!here_doc.contains('`'));

    let default_define = std::fs::read_to_string(root.join("include/define.h")).expect("define.h");
    let directives: Vec<(&str, &str)> = default_define
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let action = words.next()?;
            matches!(action, "#define" | "#undef")
                .then(|| (action, words.next().unwrap_or_default()))
        })
        .collect();
    for retired in [
        "LULC_IGBP_PFT",
        "LULC_IGBP_PC",
        "BGC",
        "LULCC",
        "TRACER",
        "Campbell_SOIL_MODEL",
        "vanGenuchten_Mualem_SOIL_MODEL",
        "LATERAL_FLOW",
    ] {
        assert!(
            directives.iter().all(|(_, name)| *name != retired),
            "static define.h restored retired or nonexistent macro {retired}"
        );
    }
    assert!(directives.contains(&("#define", "URBAN_MODEL")));
    assert!(directives.contains(&("#define", "CatchLateralFlow")));
}

#[test]
fn urban_classification_cannot_be_an_arbitrary_integer() {
    assert_eq!(
        colm_schema::find("DEF_URBAN_type_scheme")
            .expect("DEF_URBAN_type_scheme")
            .values,
        &["1", "2"]
    );
}

#[test]
fn stomatal_overrides_are_optional_and_used_by_the_shared_solver() {
    for name in [
        "DEF_BALL_BERRY_GRADM",
        "DEF_BALL_BERRY_BINTER",
        "DEF_MEDLYN_G1",
        "DEF_MEDLYN_G0",
        "DEF_WUE_LAMBDA",
    ] {
        assert_eq!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Real("-1.0_r8")),
            "{name} must preserve the land-cover/PFT table unless explicitly tuned"
        );
    }

    let solver = std::fs::read_to_string(
        repo().join("vendor/CoLM202X/main/MOD_AssimStomataConductance.F90"),
    )
    .expect("MOD_AssimStomataConductance.F90");
    for assignment in [
        "gradm_used = DEF_BALL_BERRY_GRADM",
        "binter_used = DEF_BALL_BERRY_BINTER",
        "g1_used = DEF_MEDLYN_G1",
        "g0_used = DEF_MEDLYN_G0",
        "lambda_used = DEF_WUE_LAMBDA",
    ] {
        assert!(
            solver.contains(assignment),
            "shared solver lost {assignment}"
        );
    }
}

#[test]
fn core_expert_tuning_preserves_defaults_and_reaches_the_model() {
    let defaults = [
        ("DEF_TUNING_ZLND", "0.01_r8"),
        ("DEF_TUNING_ZSNO", "0.0024_r8"),
        ("DEF_TUNING_CSOILC", "0.004_r8"),
        ("DEF_TUNING_DEWMX", "0.1_r8"),
        ("DEF_TUNING_CAPR", "0.34_r8"),
        ("DEF_TUNING_CNFAC", "0.5_r8"),
        ("DEF_TUNING_SSI", "0.033_r8"),
        ("DEF_TUNING_WIMP", "0.05_r8"),
        ("DEF_TUNING_PONDMX", "10.0_r8"),
        ("DEF_TUNING_SMPMAX", "-1.5e5_r8"),
        ("DEF_TUNING_SMPMIN", "-1.e8_r8"),
        ("DEF_TUNING_SMPMAX_HR", "-2.e2_r8"),
        ("DEF_TUNING_SMPMIN_HR", "-2.e5_r8"),
        ("DEF_TUNING_TRSMX0", "2.e-4_r8"),
        ("DEF_TUNING_WETWATMAX", "200.0_r8"),
        ("DEF_TUNING_SOIL_ICE_IMPEDANCE", "6.0_r8"),
        ("DEF_TUNING_TOPMOD_DECAY", "2.0_r8"),
        ("DEF_TUNING_SIMPLE_VIC_DS", "0.061_r8"),
        ("DEF_TUNING_SIMPLE_VIC_WS", "0.646_r8"),
        ("DEF_TUNING_SNOW_COVER_EXPONENT", "1.0_r8"),
        ("DEF_TUNING_IRRIGATION_START_SEC", "21600.0_r8"),
        ("DEF_TUNING_IRRIGATION_DURATION_SEC", "14400.0_r8"),
        ("DEF_TUNING_IRRIGATION_MAX_DEPTH", "1.0_r8"),
        ("DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION", "1.0_r8"),
        ("DEF_TUNING_IRRIGATION_SUPPLY_FRACTION", "1.0_r8"),
        ("DEF_TUNING_IRRIGATION_MIN_CPHASE", "1.0_r8"),
        ("DEF_TUNING_IRRIGATION_MAX_CPHASE", "4.0_r8"),
        ("DEF_TUNING_IRRIGATION_PONDMX", "100.0_r8"),
        ("DEF_TUNING_CROP_PLANTING_DAY", "0.0_r8"),
        ("DEF_PH_CROOT_LATERAL_LENGTH", "0.25_r8"),
        ("DEF_PH_K_AXS", "2.0e-1_r8"),
        ("DEF_PH_FROOT_CARBON", "288.392056287006_r8"),
        ("DEF_PH_ROOT_RADIUS", "2.9e-4_r8"),
        ("DEF_PH_ROOT_DENSITY", "310000._r8"),
        ("DEF_PH_FROOT_LEAF", "1.5_r8"),
        ("DEF_PH_KRMAX", "3.981071705534969e-009_r8"),
        ("DEF_OZONE_KO3", "1.51_r8"),
        ("DEF_DS_TEMP_LAPSE_RATE", "0.006_r8"),
        ("DEF_DS_LONGWAVE_LAPSE_RATE", "0.032_r8"),
        ("DEF_DS_LONGWAVE_LIMIT", "0.5_r8"),
        ("DEF_DS_SHORTWAVE_LIMIT", "0.5_r8"),
        ("DEF_DS_SHORTWAVE_SIMPLE_LIMIT", "0.2_r8"),
    ];
    for (name, default) in defaults {
        let field = colm_schema::find(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(field.group, Some("nl_colm"), "{name} must be case-local");
        assert_eq!(field.default, colm_schema::Default::Real(default));
    }

    let consumers = [
        (
            "vendor/CoLM202X/mkinidata/MOD_Initialize.F90",
            "DEF_TUNING_",
            15,
        ),
        ("vendor/CoLM202X/main/MOD_PlantHydraulic.F90", "DEF_PH_", 7),
        ("vendor/CoLM202X/main/MOD_Ozone.F90", "DEF_OZONE_KO3", 1),
        (
            "vendor/CoLM202X/main/MOD_ForcingDownscaling.F90",
            "DEF_DS_",
            5,
        ),
    ];
    for (path, prefix, minimum) in consumers {
        let source =
            std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(
            source.matches(prefix).count() >= minimum,
            "{path} no longer consumes all {prefix} expert parameters"
        );
    }
    for (path, names) in [
        (
            "vendor/CoLM202X/main/MOD_Runoff.F90",
            &[
                "DEF_TUNING_SOIL_ICE_IMPEDANCE",
                "DEF_TUNING_TOPMOD_DECAY",
                "DEF_TUNING_SIMPLE_VIC_DS",
                "DEF_TUNING_SIMPLE_VIC_WS",
            ][..],
        ),
        (
            "vendor/CoLM202X/main/MOD_SnowFraction.F90",
            &["DEF_TUNING_SNOW_COVER_EXPONENT"][..],
        ),
        (
            "vendor/CoLM202X/main/MOD_Irrigation.F90",
            &[
                "DEF_TUNING_IRRIGATION_START_SEC",
                "DEF_TUNING_IRRIGATION_DURATION_SEC",
                "DEF_TUNING_IRRIGATION_MAX_DEPTH",
                "DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION",
                "DEF_TUNING_IRRIGATION_SUPPLY_FRACTION",
                "DEF_TUNING_IRRIGATION_MIN_CPHASE",
                "DEF_TUNING_IRRIGATION_MAX_CPHASE",
            ][..],
        ),
        (
            "vendor/CoLM202X/main/MOD_SoilSnowHydrology.F90",
            &[
                "DEF_TUNING_SOIL_ICE_IMPEDANCE",
                "DEF_TUNING_IRRIGATION_PONDMX",
            ][..],
        ),
        (
            "vendor/CoLM202X/main/HYDRO/MOD_Catch_SubsurfaceFlow.F90",
            &["DEF_TUNING_SOIL_ICE_IMPEDANCE"][..],
        ),
    ] {
        let source =
            std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        for name in names {
            assert!(source.contains(name), "{path} no longer consumes {name}");
        }
    }
    assert!(colm_schema::find("DEF_TUNING_TCRIT").is_none());
}

#[test]
fn disabled_crop_management_does_not_read_optional_runtime_files() {
    let source = std::fs::read_to_string(repo().join("vendor/CoLM202X/main/MOD_CropReadin.F90"))
        .expect("MOD_CropReadin.F90");
    let fertilizer = source
        .find("IF (DEF_USE_FERT) THEN")
        .expect("fertilizer guard");
    let fertilizer_file = source
        .find("/crop/fertnitro_fillcoast.nc")
        .expect("fertilizer input");
    let irrigation = source
        .find("IF (DEF_USE_IRRIGATION) THEN")
        .expect("irrigation guard");
    let irrigation_file = source
        .find("/crop/surfdata_irrigation_method_96x144.nc")
        .expect("irrigation input");
    assert!(fertilizer < fertilizer_file);
    assert!(irrigation < irrigation_file);
    assert!(source.contains("DEF_TUNING_CROP_PLANTING_DAY"));
}

#[test]
fn paddy_excess_becomes_runoff_before_ponding_is_clamped() {
    let source =
        std::fs::read_to_string(repo().join("vendor/CoLM202X/main/MOD_SoilSnowHydrology.F90"))
            .expect("MOD_SoilSnowHydrology.F90");
    let branch = source
        .split("IF(wdsrf.gt.DEF_TUNING_IRRIGATION_PONDMX)THEN")
        .nth(1)
        .expect("paddy ponding branch")
        .split("ENDIF")
        .next()
        .unwrap();
    let runoff = branch.find("rsur = rsur +").expect("excess runoff");
    let clamp = branch
        .find("wdsrf = DEF_TUNING_IRRIGATION_PONDMX")
        .expect("ponding clamp");
    assert!(
        runoff < clamp,
        "clamping first would make excess runoff zero"
    );
}

#[test]
fn model_schemes_expose_complete_discrete_choices_to_the_gui() {
    let expected: &[(&str, &[&str])] = &[
        ("DEF_SOIL_REFL_SCHEME", &["1", "2"]),
        ("DEF_LULCC_SCHEME", &["1", "2"]),
        (
            "DEF_Interception_scheme",
            &["1", "2", "3", "4", "5", "6", "7", "8"],
        ),
        (
            "DEF_THERMAL_CONDUCTIVITY_SCHEME",
            &["1", "2", "3", "4", "5", "6", "7", "8"],
        ),
        ("DEF_RSS_SCHEME", &["0", "1", "2", "3", "4", "5"]),
        ("DEF_Runoff_SCHEME", &["0", "1", "2", "3"]),
        ("DEF_TOPMOD_method", &["0", "1", "2"]),
        ("DEF_NDEP_FREQUENCY", &["1", "2"]),
        ("DEF_Reservoir_Method", &["0", "1"]),
        (
            "DEF_wetland_finundation_scheme",
            &["1", "2", "3", "4", "5", "6", "7"],
        ),
        ("DEF_SSP", &["126", "245", "370", "585", "off"]),
        ("DEF_IRRIGATION_ALLOCATION", &["1", "2", "3"]),
        ("DEF_RSTFAC", &["1", "2"]),
        ("DEF_FERT_SOURCE", &["1", "2"]),
        ("DEF_DA_RTM_diel", &["0", "1", "2", "3"]),
        ("DEF_DA_RTM_rough", &["0", "1", "2", "3"]),
        ("DEF_DS_longwave_adjust_scheme", &["I", "II"]),
        (
            "DEF_WRST_FREQ",
            &["DAILY", "HOURLY", "MONTHLY", "TIMESTEP", "YEARLY", "none"],
        ),
        (
            "DEF_HIST_FREQ",
            &["DAILY", "HOURLY", "MONTHLY", "TIMESTEP", "YEARLY", "none"],
        ),
    ];
    for (name, values) in expected {
        assert_eq!(
            colm_schema::find(name)
                .unwrap_or_else(|| panic!("schema 里没有 {name}"))
                .values,
            *values,
            "{name} 应渲染为有限选项而不是自由输入"
        );
    }
}
