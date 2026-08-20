//! 生成的闸门表必须与一次真实运行对得上。
//!
//! 黄金文件（2.7 MB）入库，所以这条测试在 CI 的三个平台都能跑，
//! 不需要 PLUMBER2 数据也不需要 gfortran。

use std::collections::BTreeSet;

// `CoLMDEBUG` / `RangeCheck` 不在这里——调试三件套改成运行时开关之后，
// `kernels/default/manifest.json` 的 `macros` 不再列出它们了（见
// `create_defineh.bash`）。`vanGenuchten_Mualem_SOIL_MODEL` 同理——土壤
// 水力方案也改成运行时开关了。`extend_interception` 还在：它是编译期的
// 文件选择（main/ vs extends/interception/ 两套不同签名的实现），不是
// 简单的 body-level 分支，没有对应的运行时开关，`create_defineh.bash`
// 仍然无条件 `#define` 它。没有任何闸门表条目按
// `vanGenuchten_Mualem_SOIL_MODEL` 过滤，去掉它不改变下面测的行为。
const WATERHEAT: [&str; 3] = ["LULC_IGBP", "SinglePoint", "extend_interception"];

fn golden_vars() -> BTreeSet<String> {
    let p =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/default_hist_vars.txt");
    std::fs::read_to_string(&p)
        .expect("the fixture must exist")
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn the_fixture_still_matches_the_golden_file() {
    // fixture 是从黄金文件抄出来的，这条守住它没有跑偏。
    // 用 netcdf 直接读，而不是相信当初那次 ncdump。
    let f =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/CN-Cng_hist_2008-01.nc");
    let nc = netcdf::open(&f).expect("golden file opens");
    // v.name() 返回 String（不是 &str），所以先绑定再切前缀，
    // 免得在链式调用里对临时值取借用。
    let actual: BTreeSet<String> = nc
        .variables()
        .filter_map(|v| {
            let n = v.name();
            n.strip_prefix("f_").map(str::to_string)
        })
        .collect();
    assert_eq!(actual, golden_vars());
    assert_eq!(actual.len(), 119);
}

#[test]
fn the_static_map_never_misses_a_variable_that_was_actually_written() {
    // **零漏报是硬要求。** 多报（宏放行但运行时条件挡住）是可以接受的，
    // 且那 4 个已被逐一定位；漏报意味着 GUI 会告诉用户「这个内核产不出 X」
    // 而它其实产得出 —— 那是在用一张表去否定一次真实运行。
    let macros: BTreeSet<&str> = WATERHEAT.into_iter().collect();
    let predicted = colm_hist::writable(&macros);
    // 先绑定：直接 `golden_vars().iter()` 会借一个到语句末尾就被丢掉的临时值，
    // 而 `missed` 要活到下一行的断言里。
    let golden = golden_vars();
    let missed: Vec<&String> = golden
        .iter()
        .filter(|v| !predicted.contains(v.as_str()))
        .collect();
    assert!(missed.is_empty(), "the map missed {missed:?}");
}

#[test]
fn the_only_over_prediction_is_the_four_runtime_gated_ones() {
    // 多报的必须**恰好**是那 4 个有运行时条件的。多出别的，说明宏闸门
    // 有一处判错了，而这条测试会指名道姓。
    let macros: BTreeSet<&str> = WATERHEAT.into_iter().collect();
    let golden = golden_vars();
    let over: Vec<&str> = colm_hist::writable(&macros)
        .into_iter()
        .filter(|v| !golden.contains(*v))
        .collect();
    assert_eq!(over, ["dz_lake", "qcharge", "t2m_wmo", "xy_hpbl"]);
}

#[test]
fn the_second_window_agrees_with_the_first() {
    // 同一个预设、不同的模拟窗口，输出变量集必须相同 —— 变量集取决于
    // 预设与配置，不取决于季节。两个黄金文件都在库里，比一下不花钱。
    let mut sets = Vec::new();
    for name in [
        "golden/CN-Cng_hist_2008-01.nc",
        "golden/CN-Cng-wet_hist_2008-07.nc",
    ] {
        let f = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
        let nc = netcdf::open(&f).expect("golden file opens");
        sets.push(
            nc.variables()
                .filter_map(|v| {
                    let n = v.name();
                    n.strip_prefix("f_").map(str::to_string)
                })
                .collect::<BTreeSet<String>>(),
        );
    }
    assert_eq!(sets[0], sets[1]);
}
