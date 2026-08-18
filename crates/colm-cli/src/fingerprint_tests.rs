use super::*;

const CASE: &str = "\
&nl_colm
   DEF_CASE_NAME = 'X'
   SITE_fsitedata = '/no/such/site.nc'
   DEF_dir_rawdata = '/data/raw/'
   DEF_simulation_time%start_year = 2008
   DEF_simulation_time%end_year = 2008
   DEF_HIST_FREQ = 'HOURLY'
   DEF_dir_output = '/out/'
/
";

fn write(name: &str, text: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("colm-fingerprint-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("case.nml");
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn the_surface_stage_ignores_the_time_window() {
    // 地表数据描述的是这个点长什么样，不是跑哪一段。换个时间窗口重跑
    // mksrfdata 是纯浪费 —— 城市算例里它是最慢的一段（要读全球栅格）。
    let a = compute("mksrfdata", &write("a", CASE), "waterheat@abc").unwrap();
    let b = compute(
        "mksrfdata",
        &write("b", &CASE.replace("end_year = 2008", "end_year = 2010")),
        "waterheat@abc",
    )
    .unwrap();
    assert_eq!(first_difference(&a, &b), None, "时间窗口不该让地表数据失效");
}

#[test]
fn the_surface_stage_notices_a_different_rawdata_directory() {
    // 这条正是「只看产物在不在」漏掉的那种：文件还在，内容却已经不对。
    let a = compute("mksrfdata", &write("c", CASE), "waterheat@abc").unwrap();
    let b = compute(
        "mksrfdata",
        &write("d", &CASE.replace("/data/raw/", "/data/other/")),
        "waterheat@abc",
    )
    .unwrap();
    let d = first_difference(&a, &b).expect("必须发现");
    assert!(d.contains("DEF_dir_rawdata"), "{d}");
}

#[test]
fn a_different_kernel_invalidates_everything() {
    // 换个预设就是换了一套编译期宏，地表数据也跟着不同。
    let a = compute("mksrfdata", &write("e", CASE), "waterheat@abc").unwrap();
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
