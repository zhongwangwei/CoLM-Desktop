//! 前处理子栏的端到端判据：**用户自己转换出来的强迫场真的能跑出结果，
//! 而且他在界面上做的选择真的传到了模型。**
//!
//! `forcing_convert.rs`（A1）已经钉住了转换管道本身：CN-Cng 走一遍恒等
//! 转换，history 与黄金文件 `identical: 129 variables`。但那条测试只覆盖
//! 了转换管道最省心的一种情况——单位不变、高度源文件自带、降水只有一个
//! `Precip`、内核是 `default`。这一条补的是它没走过的三条路：
//!
//! | | A1 的 `forcing_convert.rs` | 这一条 |
//! |---|---|---|
//! | 数据 | PLUMBER2 CN-Cng | Urban-PLUMBER FI-Kumpula |
//! | 高度 | 源文件有（6.0） | 源文件没有，手填 48.05 |
//! | 降水 | 单个 `Precip` | `Rainf` + `Snowf` 合成 |
//! | 内核 | `default` | `urban` |
//!
//! **走 `colm-cli` 子进程，不直接构造 `CaseSpec`。** 城市站要
//! `colm_srfdata::site::prepare_urban`（经 `colm-cli new`）走两张预抽表
//! （21 个站的土壤剖面、LCZ、湖深），直接构造 `CaseSpec` 会绕开那一整套，
//! 测的就不是用户真实会走的路径了。
//!
//! **`--met` 是这条链的关键**（commit `20e3bd1`）。`colm-cli new` 不给
//! `--met` 时会用 `sibling()` 静默推出**原始**强迫场——那样这条测试
//! 验的是原始文件，而且不会失败，价值就是空的。
//!
//! 判据三条，缺一条这测试就是空的：
//!
//! 1. 三段（mksrfdata / mkinidata / colm）跑完，history 里 `f_tref`
//!    在物理范围内 —— 证明这条路能跑通。
//! 2. 算例的 `forcing.nml` 里 `HEIGHT_T` 是手填的 `48.05`，不是 `NaN` ——
//!    证明手填的高度真的传到了模型。那个 `NaN` 正是会让 CoLM 的
//!    RangeCheck 当场 `SIGILL`、却报成「内核编进了 CoLMDEBUG」的东西，
//!    `forcing_convert.rs` 的文档注释里记着这段历史。
//! 3. 产物的 `Precip` 总量 == 源文件 `Rainf` + `Snowf` 总量 —— 证明降水
//!    合成真的生效了。
//!
//! 后两条才是这条测试的价值所在。只验「跑通了」是空的——用原始强迫场
//! （不合并降水、不带手填高度）也一样跑得通，那正是 `--met` 缺席时会
//! 发生的事。
//!
//! 需要 `URBAN_PLUMBER_ROOT`、已构建的 `kernels/urban`、以及已构建的
//! `target/debug/colm-cli`（`cargo build -p colm-cli`）——三样有一样不在
//! 就跳过。标 `#[ignore]`：它要建 srfdata/initdata 并跑一段模拟，几分钟。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn urban_plumber() -> Option<PathBuf> {
    std::env::var("URBAN_PLUMBER_ROOT").ok().map(PathBuf::from)
}

fn cli() -> Option<PathBuf> {
    let p = repo().join("target/debug/colm-cli");
    p.is_file().then_some(p)
}

fn s(p: &Path) -> String {
    p.to_str().expect("utf-8 path").to_string()
}

/// 跑一条 `colm-cli` 子命令，失败就带上 stderr 整段中止。
fn run(cli: &Path, args: &[&str], what: &str) -> std::process::Output {
    let out = Command::new(cli)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{what}: 起不了子进程: {e}"));
    if !out.status.success() {
        panic!(
            "{what} 失败 (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out
}

/// 一个日志文件的最后几行，找不到就说明这段还没跑到。
fn tail(path: &Path, n: usize) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(n);
            lines[start..].join("\n")
        }
        Err(_) => format!("({} 不存在)", path.display()),
    }
}

#[test]
#[ignore]
fn a_user_converted_forcing_actually_runs_and_its_choices_reach_the_model() {
    let Some(root) = urban_plumber() else {
        eprintln!("URBAN_PLUMBER_ROOT not set — skipping");
        return;
    };
    let repo = repo();
    let kernel_dir = repo.join("kernels/urban");
    if !kernel_dir.join("manifest.json").exists() {
        eprintln!("no kernel at {} — skipping", kernel_dir.display());
        return;
    }
    let Some(cli_bin) = cli() else {
        eprintln!(
            "no {} — build it first with `cargo build -p colm-cli` — skipping",
            repo.join("target/debug/colm-cli").display()
        );
        return;
    };

    let site = root.join("Sitedata/FI-Kumpula_site_v1.nc");
    let src_met = root.join("Forcing/FI-Kumpula_metforcing_v1.nc");
    assert!(site.is_file(), "站点文件不在: {}", site.display());
    assert!(src_met.is_file(), "强迫场文件不在: {}", src_met.display());

    let work = repo.join("oracle/work/forcing-prep");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");

    // 转换产物单独放一个目录，与算例目录（case.nml / forcing.nml / out/）分开，
    // 与 `forcing_convert.rs` 的 `a_converted_forcing_reproduces_the_golden_history`
    // 同一个理由。
    let forcing_dir = work.join("forcing");
    std::fs::create_dir_all(&forcing_dir).expect("forcing dir");
    let converted = forcing_dir.join("FI-Kumpula_converted_metforcing_v1.nc");
    let case_dir = work.join("case");

    // ① forcing-convert —— 第 4 槽由 Rainf + Snowf 合成，三个观测高度
    // 源文件没有，手填 48.05。
    let t0 = Instant::now();
    run(
        &cli_bin,
        &[
            "forcing-convert",
            &s(&src_met),
            &s(&converted),
            "--slot",
            "4=Rainf:kg/m2/s+Snowf",
            "--height",
            "48.05,48.05,48.05",
        ],
        "forcing-convert",
    );
    let t_convert = t0.elapsed();
    assert!(converted.is_file(), "转换没有产出 {}", converted.display());

    // ② new —— **必须显式给 `--met`**：不给的话 `sibling()` 会静默推出
    // 原始强迫场（`FI-Kumpula_metforcing_v1.nc`），这条测试就验不出
    // 任何东西了（见 commit `20e3bd1`）。窗口取强迫场覆盖范围
    // （1999-12-31 → 2013-12-31）里的 10 天，避免跑十几年；预热关掉，
    // 避免跑几小时。
    let t1 = Instant::now();
    run(
        &cli_bin,
        &[
            "new",
            "--site",
            &s(&site),
            "--out",
            &s(&case_dir),
            "--met",
            &s(&converted),
            "--start",
            "2010-01-01",
            "--end",
            "2010-01-11",
            "--spinup-years",
            "0",
            "--spinup-repeat",
            "0",
        ],
        "colm-cli new",
    );
    let t_new = t1.elapsed();

    // 判据 ②：手填的高度要真的落进这个算例的 forcing.nml 里，不是 NaN。
    // 在跑模型之前就查——这条要是不对，后面跑出来的一切都不能说明什么。
    let forcing_nml = case_dir.join("forcing.nml");
    let nml_text = std::fs::read_to_string(&forcing_nml)
        .unwrap_or_else(|e| panic!("读不了 {}: {e}", forcing_nml.display()));
    let height_line = nml_text
        .lines()
        .find(|l| l.contains("HEIGHT_T"))
        .unwrap_or_else(|| panic!("{} 里没有 HEIGHT_T 那一行", forcing_nml.display()));
    let height_t: f64 = height_line
        .split('=')
        .nth(1)
        .unwrap_or_else(|| panic!("HEIGHT_T 那行解析不出等号右边: {height_line:?}"))
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("HEIGHT_T 的值不是个数: {height_line:?} ({e})"));
    println!("forcing.nml: {}", height_line.trim());
    assert!(
        !height_t.is_nan(),
        "HEIGHT_T 是 NaN —— 手填的高度没有传到模型；这正是会让 CoLMDEBUG \
         内核的 RangeCheck 当场 SIGILL、却报成「内核编进了 CoLMDEBUG」的那种问题"
    );
    assert!(
        (height_t - 48.05).abs() < 1e-9,
        "HEIGHT_T = {height_t}，不是手填的 48.05"
    );

    // ③ run —— 城市内核，三段全跑。
    let run_start = SystemTime::now();
    let t2 = Instant::now();
    let out = Command::new(&cli_bin)
        .args(["run", &s(&case_dir), "--kernel", &s(&kernel_dir)])
        .output()
        .expect("colm-cli run 能起进程");
    let t_run = t2.elapsed();
    if !out.status.success() {
        eprintln!("stdout:\n{}", String::from_utf8_lossy(&out.stdout));
        eprintln!("stderr:\n{}", String::from_utf8_lossy(&out.stderr));
        for log in ["mksrfdata.log", "mkinidata.log", "colm.log"] {
            eprintln!(
                "--- {log} (最后 30 行) ---\n{}",
                tail(&case_dir.join(log), 30)
            );
        }
        panic!(
            "colm-cli run 失败 (exit {:?})——上面贴了三段各自的日志尾巴，\
             城市站的失败往往是缺栅格数据，与转换管道无关，要看清楚是哪一类",
            out.status.code()
        );
    }

    // 三段各自的耗时：拿各自 log 文件的 mtime 相对上一段结束时刻的差值近似，
    // 因为 `colm-cli run` 是一个子进程跑完三段，测试这边量不到中间点。
    let mut prev = run_start;
    for stage in ["mksrfdata", "mkinidata", "colm"] {
        let log = case_dir.join(format!("{stage}.log"));
        if let Ok(meta) = std::fs::metadata(&log) {
            if let Ok(mtime) = meta.modified() {
                let dur = mtime.duration_since(prev).unwrap_or_default();
                println!("  stage {stage}: ~{:.1}s", dur.as_secs_f64());
                prev = mtime;
            }
        }
    }
    println!(
        "elapsed — forcing-convert: {:.1}s, new: {:.1}s, run (三段合计): {:.1}s",
        t_convert.as_secs_f64(),
        t_new.as_secs_f64(),
        t_run.as_secs_f64()
    );

    // 判据 ①：history 里 f_tref 得在物理范围内 —— 证明这条路真的跑通了，
    // 不是编译能过、跑起来是垃圾数据。
    let hist = case_dir.join("out/FI-Kumpula/history/FI-Kumpula_hist_2010-01.nc");
    assert!(
        hist.is_file(),
        "跑完了但没有 history 文件: {}",
        hist.display()
    );
    let f = netcdf::open(&hist).unwrap_or_else(|e| panic!("打不开 {}: {e}", hist.display()));
    let var = f
        .variable("f_tref")
        .unwrap_or_else(|| panic!("{} 里没有 f_tref", hist.display()));
    let missing = match var.attribute_value("missing_value").and_then(|r| r.ok()) {
        Some(netcdf::AttributeValue::Double(x)) => Some(x),
        Some(netcdf::AttributeValue::Float(x)) => Some(f64::from(x)),
        _ => None,
    };
    let raw: Vec<f64> = var
        .get_values(netcdf::Extents::All)
        .expect("读 f_tref 的值");
    let vals: Vec<f64> = match missing {
        Some(m) => raw.into_iter().filter(|v| (*v - m).abs() > 1.0).collect(),
        None => raw,
    };
    assert!(!vals.is_empty(), "f_tref 全是缺测值，一个有效点都没有");
    let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    println!(
        "f_tref: {} 个有效值, min = {min:.2} K, max = {max:.2} K",
        vals.len()
    );
    assert!(
        (220.0..=320.0).contains(&min) && (220.0..=320.0).contains(&max),
        "f_tref 落在 [{min}, {max}] K，不在物理范围 [220, 320] K 之内"
    );

    // 判据 ③：转换产物的 Precip 总量该等于源文件 Rainf + Snowf 总量——
    // 证明多源合成真的生效了，不是漏合了一个变量。容差与
    // `crates/colm-forcing/examples/sum_urban_precip.rs` 一致：逐时刻相加
    // 再求总和，与分别求和再相加，浮点非结合性会带来极小的差，不强求逐位。
    let fin = netcdf::open(&src_met).expect("重开源文件");
    let rainf: Vec<f64> = fin
        .variable("Rainf")
        .expect("源文件没有 Rainf")
        .get_values(netcdf::Extents::All)
        .expect("读 Rainf");
    let snowf: Vec<f64> = fin
        .variable("Snowf")
        .expect("源文件没有 Snowf")
        .get_values(netcdf::Extents::All)
        .expect("读 Snowf");
    let src_total: f64 = rainf.iter().sum::<f64>() + snowf.iter().sum::<f64>();

    let fout = netcdf::open(&converted).expect("重开转换产物");
    let precip: Vec<f64> = fout
        .variable("Precip")
        .expect("产物没有 Precip")
        .get_values(netcdf::Extents::All)
        .expect("读 Precip");
    let precip_total: f64 = precip.iter().sum();

    let diff = (precip_total - src_total).abs();
    println!("降水总量 — 源 Rainf+Snowf: {src_total}, 产物 Precip: {precip_total}, 差值: {diff}");
    assert!(
        diff < 1e-3,
        "产物 Precip 总量应当约等于源 Rainf+Snowf 总量，实际差 {diff}"
    );

    println!(
        "OK: 三段跑完，f_tref 物理合理；HEIGHT_T = {height_t}（手填值真的传到了模型）；\
         Precip = Rainf + Snowf（降水合成真的生效了）"
    );
}
