use super::*;

fn hist_vars(
    text: String,
    kernel_dir: String,
    dir: Option<String>,
) -> Result<Vec<HistVar>, String> {
    super::hist_vars(text, kernel_dir, dir, None)
}

fn kernel(preset: &str) -> String {
    format!("{}/../../kernels/{preset}", env!("CARGO_MANIFEST_DIR"))
}

fn have_kernel(preset: &str) -> bool {
    std::path::Path::new(&kernel(preset))
        .join("manifest.json")
        .is_file()
}

#[test]
fn invalid_history_members_are_reported_instead_of_showing_default_switches() {
    let dir = std::env::temp_dir().join(format!("colm-histvars-invalid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let kernel_dir = dir.join("kernel");
    std::fs::create_dir_all(&kernel_dir).unwrap();
    let mut hashes = serde_json::Map::new();
    for program in colm_kernel::PROGRAMS {
        std::fs::write(kernel_dir.join(colm_kernel::program_file(program)), b"").unwrap();
        hashes.insert(program.into(), colm_kernel::sha256_hex(b"").into());
    }
    let manifest = serde_json::json!({
        "schema": 1, "preset": "test", "platform": "test", "colm_git_sha": "test",
        "generator_args": "", "macros": ["SinglePoint", "LULC_IGBP"], "built_with": "test",
        "netcdf_c": "", "netcdf_fortran": "", "hdf5": "", "sha256": hashes,
    });
    std::fs::write(kernel_dir.join("manifest.json"), manifest.to_string()).unwrap();
    let text = "&nl_colm\n DEF_HIST_vars_out_default = .false.\n DEF_HIST_vars_namelist = 'history.nml'\n/\n";
    let history = "&nl_colm_history\n DEF_hist_vars%xy_us = .true.\n";
    std::fs::write(
        dir.join("history.nml"),
        format!("{history} DEF_hist_vars%not_a_real_member = .true.\n/\n"),
    )
    .unwrap();
    let run = || {
        hist_vars(
            text.into(),
            kernel_dir.to_string_lossy().into_owned(),
            Some(dir.to_string_lossy().into_owned()),
        )
    };
    let error = run()
        .err()
        .expect("unknown history member must be rejected");
    assert!(error.contains("not_a_real_member"), "{error}");
    std::fs::write(dir.join("history.nml"), format!("{history}/\n")).unwrap();
    let vars = run().expect("valid history must retain computed switch states");
    assert!(vars.iter().find(|var| var.name == "xy_us").unwrap().on);
    assert!(!vars.iter().find(|var| var.name == "xy_vs").unwrap().on);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_bgc_variable_is_unwritable_under_the_default_kernel() {
    // 勾了却没有输出是这个界面最该防的事。`ar` 的闸门是 `#ifdef BGC`，
    // 而 default 内核没编 BGC —— 界面必须说出来，而不是让人跑完
    // 一小时再去 history 文件里找一个不存在的变量。
    if !have_kernel("default") {
        return;
    }
    let v = hist_vars("&nl_colm\n/\n".into(), kernel("default"), None).expect("runs");
    let ar = v.iter().find(|x| x.name == "ar").expect("ar 在 schema 里");
    assert_eq!(ar.writable, Some(false));
    assert!(
        ar.blocked_by.as_deref().is_some_and(|s| s.contains("BGC")),
        "{:?}",
        ar.blocked_by
    );
}

#[test]
fn the_same_variable_is_writable_under_the_bgc_kernel() {
    // 同一个变量换个内核就该能写。这条与上一条成对 —— 只写一条的话，
    // 「永远报不能写」也能让上一条绿。
    //
    // BGC 现在是运行时开关（DEF_USE_BGC），不再是编译期宏 —— bgc 预设
    // 编出来的 define.h 跟 default 完全一样（三个预设都不再用 BGC 分岔，
    // 见 create_defineh.bash 头注释）。光换内核已经不够让 ar 变得可写，
    // 这份配置还得把 DEF_USE_BGC 显式打开，闸门表里 ar 的运行时条件
    // 才会求值为真。
    if !have_kernel("bgc") {
        return;
    }
    let v = hist_vars(
        "&nl_colm\n   DEF_USE_BGC = .true.\n/\n".into(),
        kernel("bgc"),
        None,
    )
    .expect("runs");
    let ar = v.iter().find(|x| x.name == "ar").expect("ar");
    assert_eq!(ar.writable, Some(true), "blocked_by={:?}", ar.blocked_by);
}

#[test]
fn a_runtime_switch_is_evaluated_against_this_case() {
    // 闸门表保留条件原文，求值要一份具体配置 —— 那正是这里的事。
    // `xerr` 的运行时条件是 DEF_USE_CBL_HEIGHT 之类；这里用一个确定的例子：
    // 把某个 DEF_USE_* 关掉，依赖它的变量就该报不可写。
    if !have_kernel("default") {
        return;
    }
    let with = hist_vars(
        "&nl_colm\n   DEF_USE_SNICAR = .true.\n/\n".into(),
        kernel("default"),
        None,
    )
    .expect("runs");
    let without = hist_vars(
        "&nl_colm\n   DEF_USE_SNICAR = .false.\n/\n".into(),
        kernel("default"),
        None,
    )
    .expect("runs");
    let pick = |v: &Vec<HistVar>| {
        v.iter()
            .find(|x| x.blocked_by.as_deref() == Some("需要 DEF_USE_SNICAR"))
            .map(|x| x.name.clone())
    };
    // 关掉时至少有一个变量因它而不可写；打开时那一条不该再出现。
    if let Some(name) = pick(&without) {
        let w = with.iter().find(|x| x.name == name).expect("同一个变量");
        assert_ne!(
            w.blocked_by.as_deref(),
            Some("需要 DEF_USE_SNICAR"),
            "{name} 在开关打开时仍报缺 DEF_USE_SNICAR"
        );
    }
}

#[test]
fn a_switch_the_gate_table_does_not_know_says_so() {
    // 有几十个开关在闸门表里没有对应条目（多为 DA_*）。
    // **不知道就说不知道** —— 当成能写会让人以为勾上就有输出。
    if !have_kernel("default") {
        return;
    }
    let v = hist_vars("&nl_colm\n/\n".into(), kernel("default"), None).expect("runs");
    let unknown: Vec<&HistVar> = v.iter().filter(|x| x.writable.is_none()).collect();
    assert!(!unknown.is_empty(), "一个未知都没有，判据大概失效了");
    assert!(
        unknown.iter().all(|x| x.blocked_by.is_some()),
        "未知也要有说法"
    );
    assert_eq!(v.len(), 586, "hist_vars 应当覆盖全部 586 个开关");
}

#[test]
fn an_expression_we_do_not_understand_is_not_guessed() {
    // 逻辑组合来自生成器，可以直接求；数值比较仍不猜。
    let t = |name: &str| match name {
        "DEF_USE_BGC" | "DEF_USE_SNICAR" => Some(true),
        "DEF_USE_NITRIF" => Some(false),
        _ => None,
    };
    assert_eq!(
        colm_hist::eval_runtime_gate("DEF_USE_SNICAR", &t),
        Some(true)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate(".not.DEF_USE_SNICAR", &t),
        Some(false)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate(".not.(DEF_USE_NITRIF)", &t),
        Some(true)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate("(DEF_USE_BGC) .and. (DEF_USE_NITRIF)", &t),
        Some(false)
    );
    assert_eq!(
        colm_hist::eval_runtime_gate("(DEF_USE_BGC) .or. (DEF_USE_NITRIF)", &t),
        Some(true)
    );
    assert_eq!(colm_hist::eval_runtime_gate("DEF_DA_ENS_NUM > 1", &t), None);
    assert_eq!(
        colm_hist::eval_runtime_gate("DEF_NOT_A_REAL_FIELD", &t),
        None,
        "不认识的字段不猜"
    );
}

#[test]
fn wetwat_is_writable_for_static_and_dynamic_wetland_cases() {
    if !have_kernel("default") {
        return;
    }
    for enabled in [false, true] {
        let text = format!(
            "&nl_colm\n DEF_USE_Dynamic_Wetland = {}\n/\n",
            if enabled { ".true." } else { ".false." }
        );
        let vars = hist_vars(text, kernel("default"), None).expect("hist vars");
        let wetwat = vars.iter().find(|v| v.name == "wetwat").expect("wetwat");
        assert_eq!(wetwat.writable, Some(true), "enabled={enabled}");
    }
}

#[test]
fn material_configuration_lists_methane_history_variables() {
    if !have_kernel("default") {
        return;
    }
    let vars = hist_vars(
        "&nl_colm\n DEF_USE_METHANE = .true.\n DEF_METHANE_PARAM_FILE = 'ch4.nml'\n/\n".into(),
        kernel("default"),
        None,
    )
    .expect("hist vars");
    let methane = vars
        .iter()
        .find(|v| v.name == "methane_surf_flux_tot")
        .expect("methane variable from MATERIAL writer");
    assert!(
        !methane.settable,
        "CH4 history is controlled outside DEF_hist_vars"
    );
}

#[test]
fn default_configuration_still_matches_the_measured_history_catalog() {
    if !have_kernel("default") {
        return;
    }
    let vars = hist_vars("&nl_colm\n/\n".into(), kernel("default"), None).expect("hist vars");
    let ready = vars
        .iter()
        .filter(|var| var.on && var.writable == Some(true))
        .count();
    let selected = colm_kernel::Kernel::open(std::path::Path::new(&kernel("default")))
        .expect("default kernel");
    let macros = selected
        .manifest
        .macros
        .iter()
        .map(String::as_str)
        .collect();
    let unconditional = colm_hist::unconditional(&macros);
    let ready_without_runtime_gate = vars
        .iter()
        .filter(|var| var.on && unconditional.contains(var.name.as_str()))
        .count();
    let runtime_gated_ready: Vec<&str> = vars
        .iter()
        .filter(|var| var.on && var.writable == Some(true))
        .filter(|var| !unconditional.contains(var.name.as_str()))
        .map(|var| var.name.as_str())
        .collect();
    assert_eq!(ready_without_runtime_gate, 114);
    assert_eq!(runtime_gated_ready, ["qlayer", "lake_deficit", "vegwp"]);
    assert_eq!(ready, 117);
}

/// CH4 history 变量的开关跟着 CH4 参数文件走：没有 CH4 时全关；`core` 时只有 18 个；
/// `write_ch4_history = .false.` 时全关。只开同位素的算例也不该显示成开启。
#[test]
fn methane_history_variables_follow_the_ch4_parameter_file() {
    if !have_kernel("default") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("colm-histvars-ch4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let isotope = "&nl_colm\n DEF_USE_ISOTOPE = .true.\n \
                   DEF_ISOTOPE_SPECIES = 'H2_18O,HDO'\n/\n";
    let methane = "&nl_colm\n DEF_USE_BGC = .true.\n DEF_USE_METHANE = .true.\n \
                   DEF_METHANE_PARAM_FILE = 'ch4.nml'\n/\n";
    let run = |text: &str| {
        hist_vars(
            text.into(),
            kernel("default"),
            Some(dir.to_string_lossy().into_owned()),
        )
        .expect("runs")
    };
    let on = |vars: &[HistVar], name: &str| vars.iter().find(|v| v.name == name).unwrap().on;
    let iso = run(isotope);
    // 只开同位素时甲烷 family 未启用，CH4 history 变量根本不在列表里。
    assert!(iso.iter().all(|v| v.name != "methane_surf_flux_tot"));
    assert!(iso.iter().all(|v| v.name != "conc_methane"));
    std::fs::write(
        dir.join("ch4.nml"),
        "&nl_colm_methane_parameter\n DEF_METHANE%ch4_history_vars = 'core'\n/\n",
    )
    .unwrap();
    let core = run(methane);
    assert!(on(&core, "methane_surf_flux_tot"));
    assert!(!on(&core, "conc_methane"));
    let core_count = core
        .iter()
        .filter(|v| v.on && colm_hist::methane::METHANE_CORE_HISTORY.contains(&v.name.as_str()))
        .count();
    assert_eq!(core_count, 18);
    std::fs::write(
        dir.join("ch4.nml"),
        "&nl_colm_methane_parameter\n DEF_METHANE%write_ch4_history = .false.\n/\n",
    )
    .unwrap();
    let off = run(methane);
    assert!(!on(&off, "methane_surf_flux_tot"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn river_switches_follow_output_mapping_and_kernel_runtime_gates() {
    let dir = std::env::temp_dir().join(format!("colm-histvars-river-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut hashes = serde_json::Map::new();
    for program in colm_kernel::PROGRAMS {
        std::fs::write(dir.join(colm_kernel::program_file(program)), b"").unwrap();
        hashes.insert(program.into(), colm_kernel::sha256_hex(b"").into());
    }
    for (preset, macros) in [
        (
            "latlon",
            vec!["GRIDBASED", "LULC_IGBP", "GridRiverLakeFlow"],
        ),
        ("default", vec!["SinglePoint", "LULC_IGBP"]),
    ] {
        let manifest = serde_json::json!({
            "schema": 1, "preset": preset, "platform": "test", "colm_git_sha": "test",
            "generator_args": "", "macros": macros, "built_with": "test",
            "netcdf_c": "", "netcdf_fortran": "", "hdf5": "", "sha256": hashes,
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        for enabled in [false, true] {
            let logical = if enabled { ".true." } else { ".false." };
            let vars = hist_vars(
                format!("&nl_colm\n DEF_USE_GridRiverLakeFlow = {logical}\n/\n"),
                dir.to_string_lossy().into_owned(),
                None,
            )
            .unwrap();
            for name in ["riv_height", "riv_veloct", "discharge", "floodfrc"] {
                let var = vars.iter().find(|v| v.name == name).unwrap();
                let writable = preset == "latlon" && enabled;
                assert_eq!(
                    var.writable,
                    Some(writable),
                    "{preset} {enabled} {name}: {:?}",
                    var.blocked_by
                );
                assert_eq!(var.blocked_by.is_none(), writable);
                assert!(var.settable);
            }
            let fortran_vars = super::hist_vars(
                format!("&nl_colm\n DEF_USE_GridRiverLakeFlow = {logical}\n/\n"),
                dir.to_string_lossy().into_owned(),
                None,
                Some("fortran".into()),
            )
            .unwrap();
            for name in ["volresv", "qresv_in", "qresv_out"] {
                // 这三个开关已有 Catch 和 Grid 两路真实 writer。单点缺宏即不可写；
                // 空间侧仍须保留不能由 logical 求值器核对的数值门禁。
                let writers: Vec<_> = colm_hist::generated::SPATIAL_VARS
                    .iter()
                    .filter(|var| var.switch == Some(name))
                    .collect();
                assert_eq!(writers.len(), 2, "both reservoir writer paths for {name}");
                for macro_name in ["CatchLateralFlow", "GridRiverLakeFlow"] {
                    assert!(writers.iter().any(|var| {
                        var.macros.iter().any(|condition| {
                            matches!(condition, colm_hist::Cond::AnyOf(names) if *names == [macro_name])
                        }) && var.runtime == Some("DEF_Reservoir_Method > 0")
                    }));
                }
                let var = vars.iter().find(|v| v.name == name).unwrap();
                let fortran = fortran_vars.iter().find(|v| v.name == name).unwrap();
                if preset == "latlon" {
                    assert_eq!(var.writable, None);
                    assert!(var.blocked_by.as_deref().unwrap().contains("当前运行引擎"));
                    assert_eq!(fortran.writable, None);
                    assert_eq!(
                        fortran.blocked_by.as_deref(),
                        Some("条件 DEF_Reservoir_Method > 0 需要人工判断")
                    );
                } else {
                    assert_eq!(var.writable, Some(false));
                    assert_eq!(fortran.writable, Some(false));
                    assert!(var
                        .blocked_by
                        .as_deref()
                        .unwrap()
                        .starts_with("本内核未编入"));
                }
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn multi_output_switches_preserve_true_unknown_and_false_gate_precedence() {
    let output = |name, switch, runtime, macros| colm_hist::Var {
        name,
        switch,
        runtime,
        macros,
        long_name: None,
        units: None,
        dims: &[],
        line: 0,
    };
    let blocked = output("blocked_output", Some("shared"), Some("DEF_DISABLED"), &[]);
    let unknown = output("unknown_output", Some("shared"), Some("DEF_UNKNOWN"), &[]);
    let ready = output("ready_output", Some("shared"), Some("DEF_ENABLED"), &[]);
    let absent_macro = output(
        "macro_output",
        Some("shared"),
        None,
        &[colm_hist::Cond::AnyOf(&["MISSING"])],
    );
    let unrelated = output("shared", Some("different_switch"), None, &[]);
    let truth = |name: &str| match name {
        "DEF_ENABLED" => Some(true),
        "DEF_DISABLED" => Some(false),
        _ => None,
    };
    let macros = std::collections::BTreeSet::new();
    let run = |vars: &[colm_hist::Var]| switch_writability("SHARED", vars, &macros, &truth);
    let all_blocked = run(&[blocked, absent_macro, unrelated]);
    assert_eq!(all_blocked.0, Some(false));
    assert!(all_blocked.1.is_some());
    for vars in [[blocked, unknown, unrelated], [unknown, blocked, unrelated]] {
        let result = run(&vars);
        assert_eq!(result.0, None);
        assert!(result.1.unwrap().contains("DEF_UNKNOWN"));
    }
    for vars in [[unknown, blocked, ready], [ready, blocked, unknown]] {
        assert_eq!(run(&vars), (Some(true), None));
    }
    assert_eq!(
        run(&[unrelated]).0,
        None,
        "an output name is not a switch association"
    );
}

#[test]
fn methane_diagnostics_exclude_aliases_and_unrelated_uncontrolled_outputs() {
    for var in colm_hist::generated::VARS {
        if var.switch.is_some() || !var.runtime.unwrap_or("").contains("DEF_USE_METHANE") {
            assert!(!is_methane_diagnostic(var), "{}", var.name);
        }
    }
    assert!(colm_hist::generated::VARS.iter().any(is_methane_diagnostic));
}

#[test]
fn spatial_writer_evidence_does_not_claim_rust_support() {
    let macros = std::collections::BTreeSet::from(["CatchLateralFlow"]);
    let truth = |_: &str| None;
    assert_eq!(
        configuration_writability("xsubs_bsn", &macros, &truth, Some("fortran")).0,
        Some(true)
    );
    for engine in [None, Some("rust")] {
        let (writable, reason) = configuration_writability("xsubs_bsn", &macros, &truth, engine);
        assert_eq!(writable, None);
        assert!(reason.unwrap().contains("Fortran"));
    }
    assert_eq!(
        configuration_writability("xsubs_bsn", &Default::default(), &truth, Some("fortran")).0,
        Some(false)
    );
    assert_eq!(
        configuration_writability("not_a_writer", &macros, &truth, Some("fortran")).0,
        None
    );
}
