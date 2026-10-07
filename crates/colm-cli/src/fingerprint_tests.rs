use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const CASE: &str = "\
&nl_colm
   DEF_CASE_NAME = 'X'
   SITE_fsitedata = '/no/such/site.nc'
   DEF_dir_rawdata = '/data/raw/'
   DEF_simulation_time%greenwich = .FALSE.
   DEF_simulation_time%start_year = 2008
   DEF_simulation_time%start_month = 1
   DEF_simulation_time%start_day = 1
   DEF_simulation_time%start_sec = 0
   DEF_simulation_time%end_year = 2008
   DEF_simulation_time%end_month = 12
   DEF_simulation_time%end_day = 31
   DEF_simulation_time%end_sec = 86400
   DEF_simulation_time%spinup_year = 2007
   DEF_simulation_time%spinup_month = 1
   DEF_simulation_time%spinup_day = 1
   DEF_simulation_time%spinup_sec = 0
   DEF_simulation_time%spinup_repeat = 1
   DEF_simulation_time%timestep = 1800.
   DEF_HIST_FREQ = 'HOURLY'
   DEF_dir_output = '/out/'
/
";

fn write(name: &str, text: &str) -> std::path::PathBuf {
    let d = temp_dir(name);
    let p = d.join("case.nml");
    std::fs::write(&p, text).unwrap();
    p
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "colm-fingerprint-{name}-{}-{}",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_surface_stage_follows_the_lai_date_window() {
    // mksrfdata 里的 LAI/城市聚合会用起止年月日秒；只看站点和 rawdata
    // 会把换了地表时段的 srfdata.nc 错当成可复用。
    let a = compute("mksrfdata", &write("a", CASE), "default@abc").unwrap();
    let b = compute(
        "mksrfdata",
        &write("b", &CASE.replace("end_year = 2008", "end_year = 2010")),
        "default@abc",
    )
    .unwrap();
    let d = first_difference(&a, &b).expect("地表时段必须让地表数据失效");
    assert!(d.contains("DEF_simulation_time%end_year"), "{d}");
}

#[test]
fn the_surface_stage_ignores_spinup_and_timestep() {
    let a = compute("mksrfdata", &write("aa", CASE), "default@abc").unwrap();
    let b = compute(
        "mksrfdata",
        &write(
            "ab",
            &CASE
                .replace("spinup_repeat = 1", "spinup_repeat = 5")
                .replace("timestep = 1800.", "timestep = 900."),
        ),
        "default@abc",
    )
    .unwrap();
    assert_eq!(
        first_difference(&a, &b),
        None,
        "spin-up 和 timestep 不该让 mksrfdata 失效"
    );
}

#[test]
fn utc_surface_stage_ignores_month_day_and_second() {
    let utc = CASE.replace(
        "DEF_simulation_time%greenwich = .FALSE.",
        "DEF_simulation_time%greenwich = .TRUE.",
    );
    let a = compute("mksrfdata", &write("utc-a", &utc), "default@abc").unwrap();
    let b = compute(
        "mksrfdata",
        &write("utc-b", &utc.replace("start_day = 1", "start_day = 2")),
        "default@abc",
    )
    .unwrap();
    assert_eq!(first_difference(&a, &b), None);
}

#[test]
fn the_surface_stage_notices_a_different_rawdata_directory() {
    // 这条正是「只看产物在不在」漏掉的那种：文件还在，内容却已经不对。
    let a = compute("mksrfdata", &write("c", CASE), "default@abc").unwrap();
    let b = compute(
        "mksrfdata",
        &write("d", &CASE.replace("/data/raw/", "/data/other/")),
        "default@abc",
    )
    .unwrap();
    let d = first_difference(&a, &b).expect("必须发现");
    assert!(
        d.contains("DEF_dir_rawdata") || d.contains("外部输入"),
        "{d}"
    );
}

#[test]
fn a_different_kernel_invalidates_everything() {
    // 换个预设就是换了一套编译期宏，地表数据也跟着不同。
    let a = compute("mksrfdata", &write("e", CASE), "default@abc").unwrap();
    let b = compute("mksrfdata", &write("f", CASE), "urban@abc").unwrap();
    let d = first_difference(&a, &b).expect("必须发现");
    assert!(d.contains("内核换了"), "{d}");
}

#[test]
fn the_initial_stage_follows_the_start_date_but_not_the_end() {
    // 初始场取决于起始时刻。结束时刻改了不必重做初始场，
    // 但起始时刻改了必须重做 —— 这两条要分开验，否则「永远重做」也能过一条。
    let base = write("g", CASE);
    let a = compute("mkinidata", &base, "k").unwrap();
    let end = write("h", &CASE.replace("end_year = 2008", "end_year = 2010"));
    assert_eq!(
        first_difference(&a, &compute("mkinidata", &end, "k").unwrap()),
        None,
        "结束时刻不该让初始场失效"
    );
    let start = write("i", &CASE.replace("start_year = 2008", "start_year = 2009"));
    assert!(
        first_difference(&a, &compute("mkinidata", &start, "k").unwrap()).is_some(),
        "起始时刻必须让初始场失效"
    );
}

#[test]
fn greenwich_invalidates_the_first_two_stages() {
    let a = write("greenwich-a", CASE);
    let b = write(
        "greenwich-b",
        &CASE.replace(
            "DEF_simulation_time%greenwich = .FALSE.",
            "DEF_simulation_time%greenwich = .TRUE.",
        ),
    );
    for stage in ["mksrfdata", "mkinidata"] {
        let d = first_difference(
            &compute(stage, &a, "k").unwrap(),
            &compute(stage, &b, "k").unwrap(),
        )
        .expect("greenwich 必须触发重跑");
        assert!(d.contains("DEF_simulation_time%greenwich"), "{stage}: {d}");
    }
}

#[test]
fn output_settings_never_invalidate_the_first_two_stages() {
    // 改一个输出频率就重跑 mksrfdata 是纯浪费，而那是最常改的东西之一。
    let a = write("j", CASE);
    let b = write("k", &CASE.replace("'HOURLY'", "'DAILY'"));
    for stage in ["mksrfdata", "mkinidata"] {
        assert_eq!(
            first_difference(
                &compute(stage, &a, "k").unwrap(),
                &compute(stage, &b, "k").unwrap()
            ),
            None,
            "{stage} 不该被输出频率影响"
        );
    }
    // 但主程序必须重跑 —— 它决定写出什么。
    assert!(
        first_difference(
            &compute("colm", &a, "k").unwrap(),
            &compute("colm", &b, "k").unwrap()
        )
        .is_some(),
        "colm 必须跟着输出设置重跑"
    );
}

#[test]
fn runtime_directory_content_changes_the_fingerprint() {
    let d = temp_dir("runtime-dir-content");
    let runtime = d.join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(runtime.join("table.txt"), b"first").unwrap();
    let case = d.join("case.nml");
    std::fs::write(
        &case,
        format!(
            "&nl_colm\n SITE_fsitedata='/no/such/site.nc'\n DEF_dir_runtime='{}'\n/\n",
            runtime.display()
        ),
    )
    .unwrap();

    let a = compute("colm", &case, "k").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::write(runtime.join("table.txt"), b"second").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert!(first_difference(&a, &b).unwrap().contains("外部输入"));
}

#[test]
fn compact_directory_field_names_are_still_tracked() {
    let d = temp_dir("compact-dir-field");
    let topography = d.join("topography");
    std::fs::create_dir_all(&topography).unwrap();
    std::fs::write(topography.join("grid.nc"), b"first").unwrap();
    let case = d.join("case.nml");
    std::fs::write(
        &case,
        format!(
            "&nl_colm\n SITE_fsitedata='/no/such/site.nc'\n DEF_DS_HiresTopographyDataDir='{}'\n/\n",
            topography.display()
        ),
    )
    .unwrap();

    let a = compute("colm", &case, "k").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::write(topography.join("grid.nc"), b"second").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert!(first_difference(&a, &b).unwrap().contains("外部输入"));
}

#[test]
fn rawdata_directory_content_changes_the_surface_fingerprint() {
    let d = temp_dir("rawdata-dir-content");
    let raw = d.join("rawdata");
    std::fs::create_dir_all(raw.join("soil")).unwrap();
    std::fs::write(raw.join("soil/a.nc"), b"first").unwrap();
    let case = d.join("case.nml");
    std::fs::write(
        &case,
        format!(
            "&nl_colm\n SITE_fsitedata='/no/such/site.nc'\n DEF_dir_rawdata='{}'\n/\n",
            raw.display()
        ),
    )
    .unwrap();

    let a = compute("mksrfdata", &case, "k").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::write(raw.join("soil/a.nc"), b"second").unwrap();
    let b = compute("mksrfdata", &case, "k").unwrap();
    assert!(first_difference(&a, &b).unwrap().contains("外部输入"));
}

#[test]
fn forcing_namelist_content_changes_the_fingerprint() {
    let d = temp_dir("forcing-content");
    let case = d.join("case.nml");
    std::fs::write(&case, CASE).unwrap();
    std::fs::write(d.join("forcing.nml"), "a = 1\n").unwrap();
    let a = compute("colm", &case, "k").unwrap();
    std::fs::write(d.join("forcing.nml"), "a = 2\n").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert!(
        first_difference(&a, &b).unwrap().contains("外部输入"),
        "forcing.nml 内容变化必须触发重跑"
    );
}

#[test]
fn large_data_content_changes_the_fingerprint() {
    let d = temp_dir("large-data-content");
    let case = d.join("case.nml");
    let data = d.join("forcing.nc");
    std::fs::write(&data, b"first").unwrap();
    std::fs::write(
        &case,
        format!(
            "&nl_colm\n DEF_CASE_NAME='X'\n DEF_mesh_data='{}'\n/\n",
            data.display()
        ),
    )
    .unwrap();
    let a = compute("colm", &case, "k").unwrap();
    let key = data.canonicalize().unwrap().to_string_lossy().into_owned();
    assert!(a.files[&key].starts_with("sample-sha256:"));
    std::fs::write(&data, b"second").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert_ne!(a.files[&key], b.files[&key]);
}

#[test]
fn forcing_payload_content_changes_the_fingerprint() {
    let d = temp_dir("forcing-payload");
    let forcing = d.join("met.nc");
    std::fs::write(&forcing, b"first").unwrap();
    let case = d.join("case.nml");
    std::fs::write(
        &case,
        "&nl_colm\n SITE_fsitedata='/no/such/site.nc'\n DEF_forcing_namelist='forcing.nml'\n/\n",
    )
    .unwrap();
    std::fs::write(
        d.join("forcing.nml"),
        format!(
            "&nl_colm_forcing\n DEF_dir_forcing='{}'\n DEF_forcing%fprefix='met.nc'\n/\n",
            d.display()
        ),
    )
    .unwrap();
    let a = compute("colm", &case, "k").unwrap();
    let marker = a
        .files
        .get(
            &forcing
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
        .unwrap();
    assert!(marker.starts_with("sample-sha256:"), "{marker}");
    std::fs::write(&forcing, b"second").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert!(first_difference(&a, &b).unwrap().contains("外部输入"));
}

#[test]
fn nested_forcing_prefix_payload_changes_the_fingerprint() {
    let d = temp_dir("forcing-nested-prefix");
    let forcing_dir = d.join("temp");
    std::fs::create_dir_all(&forcing_dir).unwrap();
    let forcing = forcing_dir.join("met_1992.nc");
    std::fs::write(&forcing, b"first").unwrap();
    let case = d.join("case.nml");
    std::fs::write(&case, "&nl_colm\n DEF_forcing_namelist='forcing.nml'\n/\n").unwrap();
    std::fs::write(
        d.join("forcing.nml"),
        format!(
            "&nl_colm_forcing\n DEF_dir_forcing='{}'\n DEF_forcing%fprefix='temp/met_'\n/\n",
            d.display()
        ),
    )
    .unwrap();

    let a = compute("colm", &case, "k").unwrap();
    let key = forcing
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(a.files[&key].starts_with("sample-sha256:"));
    std::fs::write(&forcing, b"second").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert!(first_difference(&a, &b).unwrap().contains("外部输入"));
}

#[test]
fn study_sample_stamp_changes_the_main_stage_fingerprint() {
    let d = temp_dir("study-sample-stamp");
    let case = d.join("case.nml");
    std::fs::write(&case, CASE).unwrap();
    std::fs::write(d.join("forcing.nml"), "&forcing\n/\n").unwrap();
    std::fs::write(d.join(".colm-study-sample.sha256"), "first\n").unwrap();
    let a = compute("colm", &case, "k").unwrap();
    std::fs::write(d.join(".colm-study-sample.sha256"), "second\n").unwrap();
    let b = compute("colm", &case, "k").unwrap();
    assert!(first_difference(&a, &b)
        .unwrap()
        .contains(".colm-study-sample.sha256"));
}

#[test]
fn the_same_external_file_is_hashed_once_even_with_path_aliases() {
    let d = temp_dir("path-alias");
    let p = d.join("params.txt");
    std::fs::write(&p, "x=1\n").unwrap();
    let case = d.join("case.nml");
    std::fs::write(
        &case,
        format!(
            "&nl_colm\n DEF_CASE_NAME='X'\n DEF_file_mesh='{}'\n DEF_BlockInfoFile='params.txt'\n/\n",
            p.display()
        ),
    )
    .unwrap();
    let f = compute("colm", &case, "k").unwrap();
    let canonical = p.canonicalize().unwrap().to_string_lossy().into_owned();
    assert!(f.files.contains_key(&canonical));
    assert_eq!(f.files.keys().filter(|key| *key == &canonical).count(), 1);
}

#[test]
fn old_stage_json_without_file_hashes_still_loads() {
    let d = temp_dir("old-json");
    std::fs::write(
        d.join("stages.json"),
        r#"{"colm":{"inputs":{},"site_sha256":"","kernel":"k"}}"#,
    )
    .unwrap();
    let loaded = load(&d);
    assert!(loaded.get("colm").unwrap().files.is_empty());
}

#[test]
fn large_trees_track_names_and_directories_without_per_file_stat() {
    let d = temp_dir("large-tree");
    std::fs::create_dir_all(d.join("soil")).unwrap();
    std::fs::write(d.join("soil/a.nc"), b"first").unwrap();
    std::fs::write(d.join("soil/b.nc"), b"first").unwrap();
    let small = tree_fingerprint(&d, usize::MAX).unwrap();
    let a = tree_fingerprint(&d, 1).unwrap();
    assert_ne!(small, a, "the two modes hash differently");
    // 同名原地改写：大树不算变更（小树仍然算，见上面几条）。
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::write(d.join("soil/a.nc"), b"second").unwrap();
    assert_eq!(tree_fingerprint(&d, 1).unwrap(), a);
    assert_ne!(tree_fingerprint(&d, usize::MAX).unwrap(), small);
    // 增删文件：大树也能发现。
    std::fs::write(d.join("soil/c.nc"), b"new").unwrap();
    assert_ne!(tree_fingerprint(&d, 1).unwrap(), a);
}

/// 甲烷算例：CH4 参数文件里 `allowlakeprod`/`use_spatial_ph` 决定 mksrfdata 生成什么，
/// 改了就得重做地表数据；只调其它系数（如 Q10）不重做。
#[test]
fn methane_preprocessing_switches_invalidate_the_surface_stage() {
    let case = CASE.replace(
        "   DEF_dir_output = '/out/'\n",
        "   DEF_dir_output = '/out/'\n   DEF_USE_BGC = .true.\n   DEF_USE_TRACER = .true.\n   \
         DEF_TRACER_NUM = 1\n   DEF_TRACER_NAMES = 'CH4'\n   DEF_TRACER_TYPES = 'gas'\n   \
         DEF_TRACER_PARAM_FILES = 'CH4:ch4.nml'\n",
    );
    let path = write("ch4", &case);
    let dir = path.parent().unwrap();
    let params = |lake: &str, q10: &str| {
        std::fs::write(
            dir.join("ch4.nml"),
            format!(
                "&nl_colm_methane_parameter\n DEF_METHANE%allowlakeprod = {lake}\n \
                 DEF_METHANE%use_spatial_ph = .false.\n DEF_METHANE%q10ch4 = {q10}\n/\n"
            ),
        )
        .unwrap();
    };
    params(".false.", "1.33");
    let off = compute("mksrfdata", &path, "default@abc").unwrap();
    // 两个开关都关时不记这一项：与不带甲烷的指纹同一形状，现有算例不会因此重做前处理。
    assert!(!off
        .inputs
        .keys()
        .any(|key| key.starts_with("CH4 preprocessing")));
    params(".false.", "2.0");
    let tuned = compute("mksrfdata", &path, "default@abc").unwrap();
    assert!(
        first_difference(&off, &tuned).is_none(),
        "Q10 must not redo mksrfdata"
    );
    params(".true.", "2.0");
    let lake = compute("mksrfdata", &path, "default@abc").unwrap();
    let d = first_difference(&off, &lake).expect("allowlakeprod must redo mksrfdata");
    assert!(d.contains("CH4 preprocessing"), "{d}");
    // mkinidata 读湖泊土壤碳，也要跟着重做。
    params(".false.", "2.0");
    let ini_off = compute("mkinidata", &path, "default@abc").unwrap();
    params(".true.", "2.0");
    let ini_lake = compute("mkinidata", &path, "default@abc").unwrap();
    assert!(
        first_difference(&ini_off, &ini_lake).is_some(),
        "allowlakeprod must redo mkinidata"
    );
}

/// 参数文件用 `;` 分隔时，colm 阶段也要记到每个文件的内容。
#[test]
fn semicolon_separated_parameter_files_are_all_recorded() {
    let path = write(
        "semi",
        &CASE.replace(
            "   DEF_dir_output = '/out/'\n",
            "   DEF_dir_output = '/out/'\n   DEF_TRACER_PARAM_FILES = 'a_param.nml; B:b.nml'\n",
        ),
    );
    let dir = path.parent().unwrap();
    std::fs::write(dir.join("a_param.nml"), "&x\n/\n").unwrap();
    std::fs::write(dir.join("b.nml"), "&x\n/\n").unwrap();
    let before = compute("colm", &path, "default@abc").unwrap();
    std::fs::write(dir.join("b.nml"), "&x\n a = 1\n/\n").unwrap();
    let after = compute("colm", &path, "default@abc").unwrap();
    let d = first_difference(&before, &after).expect("b.nml change must be seen");
    assert!(d.contains("b.nml"), "{d}");
}

/// 混合模型配置只影响 colm 段：加上 `hybrid.toml` 或换了它引用的模型，colm 指纹变、前处理两段不变。
#[test]
fn a_hybrid_config_and_its_model_only_touch_the_colm_stage() {
    let case = write("hybrid", CASE);
    let dir = case.parent().unwrap();
    let before: Vec<_> = ["mksrfdata", "mkinidata", "colm"]
        .iter()
        .map(|stage| compute(stage, &case, "k").unwrap())
        .collect();
    std::fs::write(dir.join("model.onnx"), b"first").unwrap();
    let toml = |sha: &str| {
        format!(
            "[[slot]]\nname = \"land_class\"\nkind = \"param\"\nmodel = \"model.onnx\"\nsha256 = \"{sha}\"\nfeatures = [\"patchclass\"]\noutputs = [{{ name = \"DEF_LC_VMAX25\" }}]\n"
        )
    };
    std::fs::write(
        dir.join("hybrid.toml"),
        toml(&colm_hybrid::sha256_hex(b"first")),
    )
    .unwrap();
    let with_first: Vec<_> = ["mksrfdata", "mkinidata", "colm"]
        .iter()
        .map(|stage| compute(stage, &case, "k").unwrap())
        .collect();
    assert!(before[0] == with_first[0]);
    assert!(before[1] == with_first[1]);
    assert!(before[2] != with_first[2]);

    std::fs::write(dir.join("model.onnx"), b"second").unwrap();
    std::fs::write(
        dir.join("hybrid.toml"),
        toml(&colm_hybrid::sha256_hex(b"second")),
    )
    .unwrap();
    let with_second = compute("colm", &case, "k").unwrap();
    assert!(with_first[2] != with_second);
    assert!(first_difference(&with_first[2], &with_second)
        .unwrap()
        .contains("hybrid"));

    // 气候特征文件：新算出来或重算过都换 colm 指纹。
    std::fs::create_dir_all(dir.join("hybrid_climate")).unwrap();
    std::fs::write(dir.join("hybrid_climate/climate.nc"), b"one").unwrap();
    let with_climate = compute("colm", &case, "k").unwrap();
    assert!(first_difference(&with_second, &with_climate)
        .unwrap()
        .contains("hybrid climate"));
    std::fs::write(dir.join("hybrid_climate/climate.nc"), b"two").unwrap();
    assert!(with_climate != compute("colm", &case, "k").unwrap());
    assert!(before[1] == compute("mkinidata", &case, "k").unwrap());
}
