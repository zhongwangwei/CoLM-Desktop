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
        frequency: HistoryFrequency::Monthly,
        ..SidecarConfig::default()
    }
}

/// 一个"月中"的重启时刻：MONTHLY 窗口在这里算不出长度，不会被判成完整窗口。
fn mid_month() -> CalendarTime {
    CalendarTime {
        year: 2010,
        julian_day: 46,
        seconds: 0,
    }
}

/// 分配条件：单点草地（无 BGC/PFT/城市）旁车里是 `nac_ln`、`nac_dt` 与 124 个 `a_*`
/// （实测 Fortran `hs` 旁车 130 个变量 = 4 个标记 + 126）；BGC、作物、城市、PFT 各自再加
/// （BGC 的 297 = 原 193 + PR #504 的 104 个火诊断累加量）。
#[test]
fn the_allocated_set_follows_the_configuration() {
    let count = |config: SidecarConfig| allocated_fields(&config).count();
    assert_eq!(count(config()), 126);
    let bgc = SidecarConfig {
        bgc: true,
        ..config()
    };
    assert_eq!(count(bgc), 126 + 297);
    assert_eq!(count(SidecarConfig { crop: true, ..bgc }), 126 + 297 + 48);
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
    assert_eq!(value("hacc_nac"), vec![48.0]);
    assert_eq!(value("hacc_freq"), vec![4.0]);
    assert_eq!(value("hacc_complete"), vec![1.0]);
    assert!(file.variable("history_nac").is_none());
    assert!(file.variable("history_freq").is_none());
    assert!(file.variable("history_complete").is_none());
    assert_eq!(value("a_q"), vec![colm_core::MISSING]);
    assert_eq!(value("nac_dt"), vec![16.0]);
    // 三维量的盘上顺序是 `(patch, d2, d1)`，与 Rust 列的展开一致。
    let alb = file.variable("a_alb").unwrap();
    let dims: Vec<_> = alb.dimensions().iter().map(|d| d.name()).collect();
    assert_eq!(dims, ["patch", "d2_a_alb", "d1_a_alb"]);
    drop(file);

    let read = read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0)
        .unwrap()
        .unwrap();
    assert_eq!(read.windows, vec![window()]);
    assert!(read.restored);
}

/// 区间与重启对齐时旁车只有三个标记，读回是空窗口。
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
    assert_eq!(names, ["hacc_complete", "hacc_freq", "hacc_nac"]);
    let read = read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0)
        .unwrap()
        .unwrap();
    assert_eq!(read.windows, vec![HistoryWindow::default()]);
    assert!(!read.restored);
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
    assert!(
        read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0)
            .unwrap()
            .is_none()
    );

    {
        let mut file = netcdf::append(&primary).unwrap();
        file.add_variable::<f64>("history_sidecar_required", &["patch"])
            .unwrap()
            .put_values(&[1.0], ..)
            .unwrap();
    }
    assert!(read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0).is_err());

    write_sidecar(&sidecar, 1, &config(), &[window()]).unwrap();
    // 频率变了：上游警告后从重启处重新开窗，不中止分段跑。
    let daily = SidecarConfig {
        frequency: HistoryFrequency::Daily,
        ..config()
    };
    assert!(
        read_sidecar(&primary, &sidecar, &daily, &[], mid_month(), 1800.0)
            .unwrap()
            .is_none()
    );

    // 主重启要求旁车，但旁车是空的（没有任何标记）：这是自家文件损坏，报错而不是静默重开。
    let empty = directory.join("empty.nc");
    netcdf::create(&empty)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    assert!(read_sidecar(&primary, &empty, &config(), &[], mid_month(), 1800.0).is_err());

    // 旁车按草地写、续跑按 BGC 读：BGC 的累加器缺席。
    let bgc = SidecarConfig {
        bgc: true,
        ..config()
    };
    let error = read_sidecar(&primary, &sidecar, &bgc, &[], mid_month(), 1800.0).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("missing land-history accumulator"),
        "{error}"
    );
}

/// 空间构建（编进 `GridRiverLakeFlow`）：旁车多一个 `hacc_river`（河道累加器有值时为 1），
/// `sidecar_river_required` 按它决定要不要读 `<旁车>.river`。
#[test]
fn river_lake_builds_write_hacc_river() {
    let directory = scratch_directory("river");
    let sidecar = directory.join("s.nc");
    let river = SidecarConfig {
        frequency: HistoryFrequency::Monthly,
        river_lake_flow: true,
        ..SidecarConfig::default()
    };
    write_sidecar(&sidecar, 2, &river, &[HistoryWindow::default()]).unwrap();
    let file = netcdf::open(&sidecar).unwrap();
    let names = file
        .variables()
        .map(|variable| variable.name())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["hacc_complete", "hacc_freq", "hacc_nac", "hacc_river"]
    );
    let values = |name: &str| {
        file.variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    };
    assert_eq!(values("hacc_complete"), vec![1.0, 1.0]);
    assert_eq!(values("hacc_river"), vec![0.0, 0.0]);
    drop(file);
    assert!(!sidecar_river_required(&sidecar).unwrap());

    // 区间跨过重启、河道累加器有值：标记为 1，调用方据此读 `.river`（由调用方另写）。
    let open = sidecar.with_file_name("open.nc");
    write_sidecar_with_river(&open, 1, &river, &[window()], true, &[false]).unwrap();
    let file = netcdf::open(&open).unwrap();
    let required = file
        .variable("hacc_river")
        .unwrap()
        .get_values::<f64, _>(..)
        .unwrap();
    assert_eq!(required, vec![1.0]);
    drop(file);
    assert!(sidecar_river_required(&open).unwrap());

    // 河湖构建读到非河湖构建写的旁车（没有 `hacc_river`）：警告后重开窗，不中止分段跑。
    let dry_primary = directory.join("dry-primary.nc");
    netcdf::create(&dry_primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let dry = sidecar.with_file_name("dry.nc");
    write_sidecar(&dry, 1, &config(), &[window()]).unwrap();
    assert!(
        read_sidecar(&dry_primary, &dry, &river, &[false], mid_month(), 1800.0)
            .unwrap()
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// 多 patch 空间块：城市累加量排在 `urban` 维上，只对应城市 patch；读回时落回原来那个 patch。
#[test]
fn urban_accumulators_follow_the_urban_patches_of_a_block() {
    let directory = scratch_directory("urban-block");
    let primary = directory.join("case_restart_2010-002-00000_lc2005_e110_n20.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 3)
        .unwrap();
    let sidecar = directory.join("case_restart_hist_2010-002-00000_e110_n20.nc");
    let urban_config = SidecarConfig {
        urban_run: true,
        urban_patches: 1,
        ..config()
    };
    let flags = [false, true, false];
    let mut windows = vec![window(); 3];
    windows[1]
        .sums
        .insert("t_room".to_owned(), WindowValue::Scalar(67920.0));
    write_sidecar_with_river(&sidecar, 3, &urban_config, &windows, false, &flags).unwrap();

    let file = netcdf::open(&sidecar).unwrap();
    assert_eq!(file.dimension("urban").unwrap().len(), 1);
    let t_room = file.variable("a_t_room").unwrap();
    assert_eq!(t_room.get_values::<f64, _>(..).unwrap(), vec![67920.0]);
    drop(file);

    let read = read_sidecar(
        &primary,
        &sidecar,
        &urban_config,
        &flags,
        mid_month(),
        1800.0,
    )
    .unwrap()
    .unwrap();
    assert_eq!(read.windows[1], windows[1]);
    assert_eq!(read.windows[0], window());
    assert!(!read.windows[2].sums.contains_key("t_room"));
}

#[test]
fn window_seconds_follows_the_upstream_calendar() {
    let at = |year: i32, julian_day: u16| CalendarTime {
        year,
        julian_day,
        seconds: 0,
    };
    // 非边界频率直接给定长窗口；none 定不下来。
    assert_eq!(
        window_seconds(HistoryFrequency::Timestep, 1800.0, at(2010, 46)).unwrap(),
        1800.0
    );
    assert_eq!(
        window_seconds(HistoryFrequency::Hourly, 1800.0, at(2010, 46)).unwrap(),
        3600.0
    );
    assert_eq!(
        window_seconds(HistoryFrequency::Daily, 1800.0, at(2010, 46)).unwrap(),
        86_400.0
    );
    assert_eq!(
        window_seconds(HistoryFrequency::None, 1800.0, at(2010, 46)).unwrap(),
        -1.0
    );
    // MONTHLY 只在每月 1 日算得出：窗口是上一个月，2 月按上一年判闰。
    assert_eq!(
        window_seconds(HistoryFrequency::Monthly, 1800.0, at(2010, 46)).unwrap(),
        -1.0
    );
    assert_eq!(
        window_seconds(HistoryFrequency::Monthly, 1800.0, at(2010, 32)).unwrap(),
        31.0 * 86_400.0
    );
    // 2012-03-01 结束的窗口是 2012 年 2 月（闰年 29 天）。
    assert_eq!(
        window_seconds(HistoryFrequency::Monthly, 1800.0, at(2012, 61)).unwrap(),
        29.0 * 86_400.0
    );
    // 2011-01-01 结束的窗口是 2010 年（365 天）；2013-01-01 是 2012 年（366 天）。
    assert_eq!(
        window_seconds(HistoryFrequency::Yearly, 1800.0, at(2011, 1)).unwrap(),
        365.0 * 86_400.0
    );
    assert_eq!(
        window_seconds(HistoryFrequency::Yearly, 1800.0, at(2013, 1)).unwrap(),
        366.0 * 86_400.0
    );
    // 不在 1 月 1 日的 YEARLY 定不下来：1 月里别的日子也不行（上游判的是儒略日 1）。
    assert_eq!(
        window_seconds(HistoryFrequency::Yearly, 1800.0, at(2011, 46)).unwrap(),
        -1.0
    );
    assert_eq!(
        window_seconds(HistoryFrequency::Yearly, 1800.0, at(2011, 10)).unwrap(),
        -1.0
    );
}

/// 重启点上的完整窗口读回时被丢弃：不续接，`restored = false`，下游的材料历史窗口跟着重开。
#[test]
fn a_complete_window_is_discarded_on_read() {
    let directory = scratch_directory("complete");
    let primary = directory.join("p.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let sidecar = directory.join("s.nc");
    let daily = SidecarConfig {
        frequency: HistoryFrequency::Daily,
        ..SidecarConfig::default()
    };
    // 重启落在 00:00、48 步 × 1800 s = 86400 s：一个完整日窗口。
    let midnight = CalendarTime {
        year: 2010,
        julian_day: 46,
        seconds: 0,
    };
    write_sidecar(&sidecar, 1, &daily, &[window()]).unwrap();
    let read = read_sidecar(&primary, &sidecar, &daily, &[], midnight, 1800.0)
        .unwrap()
        .unwrap();
    assert!(!read.restored);
    assert_eq!(read.windows, vec![HistoryWindow::default()]);

    // 半日累加（24 × 1800 s）不是完整窗口：续接。判据只看日期，与当天的秒数无关
    // （上游 `hacc_window_seconds` 的 jdate 只有年月日）。
    let partial = HistoryWindow {
        steps: 24,
        ..window()
    };
    write_sidecar(&sidecar, 1, &daily, std::slice::from_ref(&partial)).unwrap();
    let read = read_sidecar(&primary, &sidecar, &daily, &[], midnight, 1800.0)
        .unwrap()
        .unwrap();
    assert!(read.restored);
    assert_eq!(read.windows, vec![partial]);
}

// ---------------------------------------------------------------------------
// 上游布局 / 旧 Rust 布局的固定装置
// ---------------------------------------------------------------------------

/// 按上游 `MOD_HistAccRestart.F90` 的 `write_sidecar` 布局手写一份旁车：
/// 三个标记在前（`(patch)` 的 f64 向量），然后按 [`MANIFEST`] 顺序写全部已分配累加器
/// （未累加过的是 `spval`，`nac_ln`/`nac_dt` 是真值），河湖构建最后写 `hacc_river`。
/// 维度名与上游一致：`d1_<name>`、秩 3 时还带 `d2_<name>`。
fn write_fortran_layout(
    path: &Path,
    config: &SidecarConfig,
    patches: usize,
    complete: f64,
    freq: f64,
    nac: f64,
    river: Option<f64>,
) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    let per_patch = |value: f64| vec![value; patches];
    for (name, value) in [
        ("hacc_complete", complete),
        ("hacc_freq", freq),
        ("hacc_nac", nac),
    ] {
        file.add_variable::<f64>(name, &["patch"])
            .unwrap()
            .put_values(&per_patch(value), ..)
            .unwrap();
    }
    if nac > 0.0 {
        for entry in allocated_fields(config) {
            let mut values = Vec::new();
            for _ in 0..patches {
                match entry.name {
                    "nac_ln" => values.push(1.0),
                    "nac_dt" => values.push(16.0),
                    _ => values.extend(std::iter::repeat_n(colm_core::MISSING, entry.width())),
                }
            }
            let d1 = format!("d1_{}", entry.name);
            let d2 = format!("d2_{}", entry.name);
            let dims: Vec<&str> = match entry.rank {
                1 => vec!["patch"],
                2 => {
                    file.add_dimension(&d1, entry.n1).unwrap();
                    vec!["patch", &d1]
                }
                _ => {
                    file.add_dimension(&d1, entry.n1).unwrap();
                    file.add_dimension(&d2, entry.n2).unwrap();
                    vec!["patch", &d2, &d1]
                }
            };
            file.add_variable::<f64>(entry.name, &dims)
                .unwrap()
                .put_values(&values, ..)
                .unwrap();
        }
    }
    if let Some(value) = river {
        file.add_variable::<f64>("hacc_river", &["patch"])
            .unwrap()
            .put_values(&per_patch(value), ..)
            .unwrap();
    }
    file.close().unwrap();
}

/// 把一份新格式旁车复制成旧 Rust 布局：`hacc_*` 标记改名回 `history_*`，再加
/// `history_schema`（非河湖构建 = 1，河湖 = 2）。累加器与维度原样保留。
fn legacy_copy(source: &Path, target: &Path, schema: f64) {
    let source = netcdf::open(source).unwrap();
    let mut target = netcdf::create(target).unwrap();
    let mut defined = std::collections::HashSet::new();
    for variable in source.variables() {
        let name = variable.name();
        let dims: Vec<(String, usize)> = variable
            .dimensions()
            .iter()
            .map(|dim| (dim.name(), dim.len()))
            .collect();
        for (dim, len) in &dims {
            if defined.insert(dim.clone()) {
                target.add_dimension(dim, *len).unwrap();
            }
        }
        let legacy = match name.as_str() {
            "hacc_complete" => "history_complete".to_owned(),
            "hacc_freq" => "history_freq".to_owned(),
            "hacc_nac" => "history_nac".to_owned(),
            "hacc_river" => "history_river_required".to_owned(),
            other => other.to_owned(),
        };
        let dim_names: Vec<&str> = dims.iter().map(|(dim, _)| dim.as_str()).collect();
        let values = variable.get_values::<f64, _>(..).unwrap();
        target
            .add_variable::<f64>(&legacy, &dim_names)
            .unwrap()
            .put_values(&values, ..)
            .unwrap();
    }
    let patches = target.dimension_len("patch").unwrap();
    target
        .add_variable::<f64>("history_schema", &["patch"])
        .unwrap()
        .put_values(&vec![schema; patches], ..)
        .unwrap();
    target.close().unwrap();
}

/// 手写装置对应的窗口：样本数 48、`nac_ln`=1、`nac_dt`=16，其余累加器都是 `spval`（不入窗口）。
fn fortran_window(nac: usize) -> HistoryWindow {
    HistoryWindow {
        steps: nac,
        local_noon_steps: 1,
        daytime_steps: 16,
        sums: BTreeMap::new(),
    }
}

/// (a) 写出顺序与名字必须与上游一致：标记在前、`hacc_complete` 先 0 后 1、河湖构建
/// `hacc_river` 在累加器之后。全程用 netcdf 检查落盘结果，不看源码文本。
#[test]
fn the_writer_emits_the_upstream_fortran_layout() {
    let directory = scratch_directory("fortran-layout");
    let sidecar = directory.join("s.nc");
    let river = SidecarConfig {
        frequency: HistoryFrequency::Monthly,
        river_lake_flow: true,
        ..SidecarConfig::default()
    };
    write_sidecar_uncommitted(&sidecar, 1, &river, &[window()], true, &[false]).unwrap();

    // 第一步（`write_sidecar`）：完整标记为 0，累加器已在，河湖标记最后。
    let file = netcdf::open(&sidecar).unwrap();
    let names: Vec<String> = file.variables().map(|variable| variable.name()).collect();
    assert_eq!(&names[..3], ["hacc_complete", "hacc_freq", "hacc_nac"]);
    assert_eq!(names.last().unwrap(), "hacc_river");
    assert!(names.iter().any(|name| name == "a_alb"));
    let values = |name: &str| {
        file.variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    };
    assert_eq!(values("hacc_complete"), vec![0.0]);
    assert_eq!(values("hacc_freq"), vec![4.0]);
    assert_eq!(values("hacc_nac"), vec![48.0]);
    // 标记是 `(patch)` 向量；秩 3 的累加器维度名是 `(patch, d2, d1)`，与 Fortran 落盘一致。
    let marker_dims: Vec<String> = file
        .variable("hacc_complete")
        .unwrap()
        .dimensions()
        .iter()
        .map(|dim| dim.name())
        .collect();
    assert_eq!(marker_dims, ["patch"]);
    let alb = file.variable("a_alb").unwrap();
    let alb_dims: Vec<String> = alb.dimensions().iter().map(|d| d.name()).collect();
    assert_eq!(alb_dims, ["patch", "d2_a_alb", "d1_a_alb"]);
    // 旧 Rust 的标记一个都不写。
    for old in [
        "history_schema",
        "history_freq",
        "history_nac",
        "history_complete",
        "history_river_required",
    ] {
        assert!(file.variable(old).is_none(), "{old} must not be written");
    }
    drop(file);

    // 第二步（`save_history_acc_restart`）：完成标记从 0 改成 1。
    commit_sidecar(&sidecar).unwrap();
    let file = netcdf::open(&sidecar).unwrap();
    assert_eq!(
        file.variable("hacc_complete")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        vec![1.0]
    );
    drop(file);
    let _ = std::fs::remove_dir_all(&directory);
}

/// (b) 一份完全按 Fortran 布局手写的旁车 + `<旁车>.river` 求和能整套读回：陆面窗口
/// `restored = true`，河道求和由 `sidecar_river_required` 指路后读回。
#[test]
fn a_fortran_layout_sidecar_restores_with_its_river_sums() {
    let directory = scratch_directory("fortran-read");
    let primary = directory.join("case_restart_2010-002-00000_lc2005_w180_s90.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let config = SidecarConfig {
        frequency: HistoryFrequency::Monthly,
        river_lake_flow: true,
        ..SidecarConfig::default()
    };
    let sidecar = directory.join("case_restart_hist_2010-002-00000_w180_s90.nc");
    write_fortran_layout(&sidecar, &config, 1, 1.0, 4.0, 48.0, Some(1.0));
    // 河道求和：上游 `river_sums` 的字段名与维度。
    let river_path = sidecar.with_extension("nc.river");
    let mut river = crate::river::RiverHistory::zeros_for(3, false, None, None);
    river.acctime.copy_from_slice(&[100.0, 200.0, 300.0]);
    river.wdsrf.copy_from_slice(&[1.0, 2.0, 3.0]);
    crate::river::restart::write_river_history(&river_path, &river).unwrap();

    assert!(sidecar_river_required(&sidecar).unwrap());
    let sums =
        crate::river::restart::read_river_history(&river_path, 3, false, None, None).unwrap();
    assert_eq!(sums.acctime, river.acctime);
    assert_eq!(sums.wdsrf, river.wdsrf);

    let read = read_sidecar(&primary, &sidecar, &config, &[], mid_month(), 1800.0)
        .unwrap()
        .unwrap();
    assert!(read.restored);
    assert_eq!(read.windows, vec![fortran_window(48)]);
}

/// (c) 旧 Rust 布局（`history_*` 标记 + `history_schema`）仍然读得回来，且是逐标记回退。
#[test]
fn an_old_rust_sidecar_still_reads() {
    let directory = scratch_directory("legacy-read");
    let primary = directory.join("p.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let sidecar = directory.join("s.nc");
    write_sidecar(&sidecar, 1, &config(), &[window()]).unwrap();
    let legacy = directory.join("legacy.nc");
    legacy_copy(&sidecar, &legacy, 1.0);
    let file = netcdf::open(&legacy).unwrap();
    assert!(file.variable("hacc_complete").is_none());
    assert!(file.variable("history_complete").is_some());
    assert!(file.variable("history_schema").is_some());
    drop(file);

    let read = read_sidecar(&primary, &legacy, &config(), &[], mid_month(), 1800.0)
        .unwrap()
        .unwrap();
    assert!(read.restored);
    assert_eq!(read.windows, vec![window()]);
}

/// (d) 上游语义：缺/未提交完成标记、频率变了、标记集不认识 → 警告后从重启处重开窗，而不是崩。
#[test]
fn an_unknown_or_incomplete_marker_set_starts_a_fresh_window() {
    let directory = scratch_directory("fresh-window");
    let primary = directory.join("p.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let sidecar = directory.join("s.nc");

    // 完成标记还是 0：被 kill 的运行留下的半成品。
    write_fortran_layout(&sidecar, &config(), 1, 0.0, 4.0, 48.0, None);
    assert!(
        read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0)
            .unwrap()
            .is_none()
    );

    // 完成标记是 1、频率对不上：上游警告后重开窗（不是报错）。
    write_fortran_layout(&sidecar, &config(), 1, 1.0, 3.0, 48.0, None);
    assert!(
        read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0)
            .unwrap()
            .is_none()
    );

    // 标记集完全对不上（例如别的实现写的旁车）：同样警告后重开窗。
    let foreign = directory.join("foreign.nc");
    netcdf::create(&foreign)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    assert!(
        read_sidecar(&primary, &foreign, &config(), &[], mid_month(), 1800.0)
            .unwrap()
            .is_none()
    );
}

/// 新标记在、但形状/取值损坏：仍要报错（上游 `require_var`/`read_marker` 会停机）。
#[test]
fn corrupt_new_markers_are_still_rejected() {
    let directory = scratch_directory("corrupt-markers");
    let primary = directory.join("p.nc");
    netcdf::create(&primary)
        .unwrap()
        .add_dimension("patch", 1)
        .unwrap();
    let sidecar = directory.join("s.nc");

    // `nac` 不是非负整数。
    write_fortran_layout(&sidecar, &config(), 1, 1.0, 4.0, 1.5, None);
    assert!(read_sidecar(&primary, &sidecar, &config(), &[], mid_month(), 1800.0).is_err());

    // 缺 `hacc_freq`：上游 `require_var` 会停机。
    let missing_freq = directory.join("missing-freq.nc");
    {
        let mut file = netcdf::create(&missing_freq).unwrap();
        file.add_dimension("patch", 1).unwrap();
        file.add_variable::<f64>("hacc_complete", &["patch"])
            .unwrap()
            .put_values(&[1.0], ..)
            .unwrap();
        file.add_variable::<f64>("hacc_nac", &["patch"])
            .unwrap()
            .put_values(&[0.0], ..)
            .unwrap();
    }
    assert!(read_sidecar(&primary, &missing_freq, &config(), &[], mid_month(), 1800.0).is_err());
}
