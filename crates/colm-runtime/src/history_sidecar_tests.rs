use super::*;

/// 每个测试一个独立的临时目录（进程号 + 标签），并发的测试互不覆盖。
fn scratch_directory(tag: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("colm-history-sidecar-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn config() -> SidecarConfig {
    SidecarConfig {
        frequency_code: 4,
        ..SidecarConfig::default()
    }
}

/// 分配条件：单点草地（无 BGC/PFT/城市）旁车里是 `nac_ln`、`nac_dt` 与 124 个 `a_*`
/// （实测 Fortran `hs` 旁车 130 个变量 = 4 个标记 + 126）；BGC、作物、城市、PFT 各自再加。
#[test]
fn the_allocated_set_follows_the_configuration() {
    let count = |config: SidecarConfig| allocated_fields(&config).count();
    assert_eq!(count(config()), 126);
    let bgc = SidecarConfig {
        bgc: true,
        ..config()
    };
    assert_eq!(count(bgc), 126 + 193);
    assert_eq!(count(SidecarConfig { crop: true, ..bgc }), 126 + 193 + 48);
    // 没有 BGC 的 CROP 内核不分配作物量（条件是 `DEF_USE_BGC` 块里的 `#ifdef CROP`）。
    assert_eq!(
        count(SidecarConfig {
            crop: true,
            ..config()
        }),
        126
    );
    assert_eq!(
        count(SidecarConfig {
            urban_run: true,
            urban_patches: 1,
            ..config()
        }),
        126 + 21
    );
    // 城市单点里的非城市 patch：`numurban = 0`，城市量不分配。
    assert_eq!(
        count(SidecarConfig {
            urban_run: true,
            ..config()
        }),
        126
    );
    assert_eq!(
        count(SidecarConfig {
            pft_or_pc: true,
            ..config()
        }),
        126 + 14
    );
}

/// 清单里的形状与单点内核的编译期维度一致（`history::point_dimensions`）。
#[test]
fn the_manifest_shapes_match_the_point_dimensions() {
    let dims = crate::history::point_dimensions();
    let width = |name: &str| {
        MANIFEST
            .iter()
            .find(|entry| entry.name == name)
            .map(ManifestEntry::width)
            .unwrap()
    };
    assert_eq!(width("a_t_soisno"), dims.snow_layers + dims.soil);
    assert_eq!(width("a_qlayer"), dims.soil + 1);
    assert_eq!(width("a_h2osoi"), dims.soil);
    assert_eq!(width("a_t_lake"), dims.lake);
    assert_eq!(width("a_vegwp"), dims.vegnodes);
    assert_eq!(width("a_alb"), dims.band * dims.radiation_types);
    assert_eq!(width("a_sensors"), dims.sensor);
    assert_eq!(width("a_us"), 1);
}

/// 累加器键：直接写出的量用历史名（`a_us` → `xy_us`），只累加的用去掉 `a_` 的名字。
#[test]
fn window_keys_follow_the_history_names() {
    let key = |name: &str| {
        MANIFEST
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .window_key()
    };
    assert_eq!(key("a_us"), "xy_us");
    assert_eq!(key("a_solarin"), "xy_solarin");
    assert_eq!(key("a_ldew_rain"), "ldew_rain");
    assert_eq!(key("a_t2m_wmo"), "t2m_wmo");
    assert_eq!(key("a_retransn"), "retrasn");
}

fn window() -> HistoryWindow {
    let mut sums = BTreeMap::new();
    sums.insert("xy_t".to_owned(), WindowValue::Scalar(13145.0199585));
    sums.insert(
        "t_soisno".to_owned(),
        WindowValue::Column((0..15).map(f64::from).collect()),
    );
    sums.insert(
        "alb".to_owned(),
        WindowValue::Column(vec![1.0, 2.0, 3.0, 4.0]),
    );
    HistoryWindow {
        steps: 48,
        local_noon_steps: 1,
        daytime_steps: 16,
        sums,
    }
}

/// 写出再读回得到同一个窗口；未累加过的量在旁车里是 `spval`，读回时不进窗口。
#[test]
fn a_written_sidecar_reads_back_the_same_window() {
    let directory = scratch_directory("roundtrip");
    let primary = directory.join("case_restart_2010-002-00000_lc2005_w180_s90.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let sidecar = directory.join("case_restart_hist_2010-002-00000_w180_s90.nc");
    write_sidecar(&sidecar, 1, &config(), &[window()]).unwrap();

    let file = netcdf::open(&sidecar).unwrap();
    let value = |name: &str| {
        file.variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    };
    assert_eq!(value("history_nac"), vec![48.0]);
    assert_eq!(value("history_freq"), vec![4.0]);
    assert_eq!(value("history_complete"), vec![1.0]);
    assert_eq!(value("a_q"), vec![colm_core::MISSING]);
    assert_eq!(value("nac_dt"), vec![16.0]);
    // 三维量的盘上顺序是 `(patch, d2, d1)`，与 Rust 列的展开一致。
    let alb = file.variable("a_alb").unwrap();
    let dims: Vec<_> = alb.dimensions().iter().map(|d| d.name()).collect();
    assert_eq!(dims, ["patch", "d2_a_alb", "d1_a_alb"]);
    drop(file);

    let read = read_sidecar(&primary, &sidecar, &config())
        .unwrap()
        .unwrap();
    assert_eq!(read, vec![window()]);
}

/// 区间与重启对齐时旁车只有四个标记，读回是空窗口。
#[test]
fn an_aligned_sidecar_holds_only_the_markers() {
    let directory = scratch_directory("aligned");
    let primary = directory.join("p.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let sidecar = directory.join("s.nc");
    write_sidecar(&sidecar, 1, &config(), &[HistoryWindow::default()]).unwrap();
    let names: Vec<String> = netcdf::open(&sidecar)
        .unwrap()
        .variables()
        .map(|variable| variable.name())
        .collect();
    assert_eq!(
        names,
        [
            "history_schema",
            "history_freq",
            "history_nac",
            "history_complete"
        ]
    );
    let read = read_sidecar(&primary, &sidecar, &config())
        .unwrap()
        .unwrap();
    assert_eq!(read, vec![HistoryWindow::default()]);
}

/// 上游的几道校验：缺旁车而主重启要求它、历史频率变了、缺累加器，都报错；
/// 旧式重启（没有标记也没有旁车）返回 `None`。
#[test]
fn inconsistent_sidecars_are_refused() {
    let directory = scratch_directory("refused");
    let primary = directory.join("p.nc");
    {
        let mut file = netcdf::create(&primary).unwrap();
        file.add_dimension("patch", 1).unwrap();
    }
    let sidecar = directory.join("s.nc");
    assert!(read_sidecar(&primary, &sidecar, &config())
        .unwrap()
        .is_none());

    {
        let mut file = netcdf::append(&primary).unwrap();
        file.add_variable::<f64>("history_sidecar_required", &["patch"])
            .unwrap()
            .put_values(&[1.0], ..)
            .unwrap();
    }
    assert!(read_sidecar(&primary, &sidecar, &config()).is_err());

    write_sidecar(&sidecar, 1, &config(), &[window()]).unwrap();
    let daily = SidecarConfig {
        frequency_code: 3,
        ..config()
    };
    let error = read_sidecar(&primary, &sidecar, &daily).unwrap_err();
    assert!(error.to_string().contains("frequency changed"), "{error}");

    // 旁车按草地写、续跑按 BGC 读：BGC 的累加器缺席。
    let bgc = SidecarConfig {
        bgc: true,
        ..config()
    };
    let error = read_sidecar(&primary, &sidecar, &bgc).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("missing land-history accumulator"),
        "{error}"
    );
}
