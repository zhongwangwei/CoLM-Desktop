use super::*;

/// **这条才是两边结构体不脱钩的保证。** `colm-cli` 在另一个 workspace，
/// 两个 crate 不互相依赖；`Probe`/`SlotGuess` 与 `ForcingProbe`/`SlotProbe`
/// 各写各的。哪天那边改了字段名，只有拿真输出跑一遍才发现得了。
///
/// CN-Cng 是黄金回归站点（`forcing_convert.rs` 等测试同款），PLUMBER2 的
/// 90 个站点只有标量 `Wind`，第 5 槽（东向风）猜不到，三个观测高度都在。
///
/// 没有数据集或没建 `colm-cli` 就跳过 —— 与本仓库其余需要 PLUMBER2 的
/// 测试一致（照 `sites_tests.rs:40`）。
#[test]
fn it_parses_what_the_real_cli_prints_for_plumber2() {
    let Ok(root) = std::env::var("PLUMBER2_ROOT") else {
        return;
    };
    let met = format!("{root}/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc");
    if !std::path::Path::new(&met).is_file() {
        return;
    }
    let json = match crate::sidecar::capture(&[
        "forcing-probe".into(),
        met,
        "--json".into(),
        "1".into(),
    ]) {
        Ok(j) => j,
        // colm-cli 的构建产物不一定在 —— 那是另一条判据，不是这条要测的。
        Err(_) => return,
    };
    let probe: Probe = serde_json::from_str(&json).expect("真 CLI 的输出必须能解析成 Probe");
    assert_eq!(probe.slots.len(), 8);
    assert_eq!(
        probe.slots[0].guessed.as_deref(),
        Some("Tair"),
        "第 1 槽是气温"
    );
    assert_eq!(probe.height_v, Some(6.0));
    assert_eq!(probe.height_t, Some(6.0));
    assert_eq!(probe.height_q, Some(6.0));
}

/// 城市站那份。实测 Urban-PLUMBER 的 21 个站点没有三个观测高度标量，
/// 而风是 `Wind_E` + `Wind_N` 两个分量 —— 第 5 槽（东向风）**有**值，
/// 这与 PLUMBER2 的标量 `Wind`（第 5 槽为空）正好相反。两种形式都要
/// 覆盖，只测一种会漏（上一条测的是 PLUMBER2 那种）。
///
/// `URBAN_PLUMBER_ROOT` 未设就跳过。
#[test]
fn it_parses_what_the_real_cli_prints_for_urban_plumber() {
    let Ok(root) = std::env::var("URBAN_PLUMBER_ROOT") else {
        return;
    };
    let met = format!("{root}/Forcing/AU-Preston_metforcing_v1.nc");
    if !std::path::Path::new(&met).is_file() {
        return;
    }
    let json = match crate::sidecar::capture(&[
        "forcing-probe".into(),
        met,
        "--json".into(),
        "1".into(),
    ]) {
        Ok(j) => j,
        Err(_) => return,
    };
    let probe: Probe = serde_json::from_str(&json).expect("真 CLI 的输出必须能解析成 Probe");
    assert_eq!(probe.height_v, None, "城市站没有观测高度标量");
    assert_eq!(probe.height_t, None);
    assert_eq!(probe.height_q, None);
    assert_eq!(
        probe.slots[3].guessed.as_deref(),
        Some("Rainf"),
        "第 4 槽（降水）"
    );
    assert_eq!(
        probe.slots[4].guessed.as_deref(),
        Some("Wind_E"),
        "第 5 槽（东向风）：Urban-PLUMBER 是 Wind_E + Wind_N 两个分量"
    );
}

#[test]
fn build_convert_args_joins_also_add_with_plus_and_omits_height_when_none() {
    let slots = vec![
        SlotChoice {
            index: 1,
            name: "Tair".into(),
            units: "K".into(),
            also_add: vec![],
        },
        SlotChoice {
            index: 4,
            name: "Rainf".into(),
            units: "kg/m2/s".into(),
            also_add: vec!["Snowf".into()],
        },
    ];
    let args = build_convert_args("src.nc", "dst.nc", &slots, None);
    assert_eq!(
        args,
        vec![
            "forcing-convert",
            "src.nc",
            "dst.nc",
            "--slot",
            "1=Tair:K",
            "--slot",
            "4=Rainf:kg/m2/s+Snowf",
        ]
    );
    assert!(
        !args.iter().any(|a| a == "--height"),
        "没给高度就不该出现 --height"
    );
}

#[test]
fn build_convert_args_adds_height_when_given() {
    let args = build_convert_args("src.nc", "dst.nc", &[], Some([10.0, 12.5, 12.5]));
    let i = args
        .iter()
        .position(|a| a == "--height")
        .expect("给了高度就该有 --height");
    assert_eq!(args[i + 1], "10,12.5,12.5");
}

#[test]
fn gap_repair_args_keep_science_options_explicit_and_auditable() {
    let slots = vec![SlotChoice {
        index: 4,
        name: "Rainf".into(),
        units: "kg/m2/s".into(),
        also_add: vec!["Snowf".into()],
    }];
    let options = GapOptions {
        short_gap: 3,
        utc_offset: Some(9.5),
        latitude: Some(-37.73),
        longitude: Some(145.01),
        era5: Some("/cache/era5".into()),
        min_overlap: 24,
    };
    let args = build_gap_args(
        "forcing-repair",
        "source.nc",
        Some("repaired.nc"),
        &slots,
        &options,
    );
    for pair in [
        ["--slot", "4=Rainf:kg/m2/s+Snowf"],
        ["--short-gap", "3"],
        ["--utc-offset", "9.5"],
        ["--lat", "-37.73"],
        ["--lon", "145.01"],
        ["--era5", "/cache/era5"],
        ["--min-overlap", "24"],
        ["--json", "1"],
    ] {
        assert!(
            args.windows(2).any(|window| window == pair),
            "missing {pair:?} in {args:?}"
        );
    }
}

fn probe_with_shape(name: &str, dimensions: &[(&str, usize)]) -> Probe {
    Probe {
        variables: vec![name.into()],
        shapes: vec![VariableShape {
            name: name.into(),
            dimensions: dimensions
                .iter()
                .map(|(name, len)| DimensionShape {
                    name: (*name).into(),
                    len: *len,
                })
                .collect(),
        }],
        slots: vec![],
        steps: 10,
        step_seconds: 1800.0,
        step_uniform: true,
        time_units: "seconds since 2000-01-01 00:00:00".into(),
        time_first: 0.0,
        time_last: 16200.0,
        height_v: None,
        height_t: None,
        height_q: None,
        suggest_dst: String::new(),
    }
}

#[test]
fn point_cbl_accepts_known_names_but_rejects_a_regional_grid() {
    let point = probe_with_shape("PBLH", &[("time", 10), ("y", 1), ("x", 1)]);
    assert_eq!(cbl_variable(&point).unwrap(), "PBLH");

    let regional = probe_with_shape("blh", &[("time", 10), ("lat", 180), ("lon", 360)]);
    let error = cbl_variable(&regional).expect_err("区域场不能静默取第一个像元");
    assert!(
        error.contains("SinglePoint") && error.contains("180"),
        "{error}"
    );
}

#[test]
fn point_cbl_requires_an_explicit_time_dimension() {
    let no_time = probe_with_shape("boundary_layer_height", &[("record", 10), ("site", 1)]);
    assert!(cbl_variable(&no_time).unwrap_err().contains("time"));
}

#[test]
fn reject_same_dir_rejects_same_directory_and_the_tmp_private_tmp_alias() {
    let scratch = std::env::temp_dir().join("colm-forcing-reject-same-dir-test");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let src = scratch.join("src.nc");
    std::fs::write(&src, b"x").unwrap();

    // 同目录：拒绝。
    let dst_same = scratch.join("dst.nc");
    assert!(
        reject_same_dir(&src, &dst_same).is_err(),
        "产物与源文件同目录必须拒绝"
    );

    // 不同目录：放行。
    let elsewhere = scratch.join("out");
    std::fs::create_dir_all(&elsewhere).unwrap();
    assert!(
        reject_same_dir(&src, &elsewhere.join("dst.nc")).is_ok(),
        "不同目录不该被拒绝"
    );

    // macOS：`/tmp` 是指向 `/private/tmp` 的符号链接，两条路径字面上不同、
    // 磁盘上是同一处。不 canonicalize 就识破不了这层别名。
    if std::path::Path::new("/private/tmp").is_dir() {
        let real_src = std::path::Path::new("/tmp").join("colm-forcing-alias-src.nc");
        std::fs::write(&real_src, b"x").unwrap();
        let alias_dst = std::path::Path::new("/private/tmp").join("colm-forcing-alias-dst.nc");
        assert!(
            reject_same_dir(&real_src, &alias_dst).is_err(),
            "/tmp 与 /private/tmp 是同一个地方，该被识破"
        );
        let _ = std::fs::remove_file(&real_src);
    }
}
