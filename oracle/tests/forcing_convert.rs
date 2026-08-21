//! 转换管道的端到端判据：**转出来的与直读逐位相同**。
//!
//! CN-Cng 是黄金回归站点，直读的结果有 `identical: 129 variables` 钉着。
//! 把它的原始 Met 文件走一遍转换管道（变量名与单位都不变，所以这是一次
//! 恒等转换），拿转出来的文件建算例并跑完三段 —— history 应当与黄金文件
//! 逐位相同。
//!
//! **转换器若引入任何误差 —— 单位换算、时间轴取整、精度损失 —— 这个
//! 对照立刻会露出来。** 没有它，正确性只能靠肉眼看曲线。
//!
//! 前车之鉴：预抽土壤点值那一轮，`serde_json` 默认浮点解析差 1 ULP，
//! 三段照样跑通、曲线照样好看，是逐位比对把它抓出来的。
//!
//! **注意这条测的是恒等路径。** CN-Cng 的单位本来就是规范单位，
//! `convert_units` 走 `from == to` 的原样返回，逐位相同是必然而非验证
//! 所得。它真正验的是管道其余部分不动数：维度复制、属性搬运、读写往返、
//! 时间轴。换算路径没法用同样的判据 —— `K → degC → K` 往返不可能逐位
//! 复原（`273.15` 与 `233.15` 各自独立舍入、方向可能相反），那条路的
//! 判据只能是「容差之内」。别把两者混起来。
//!
//! 需要 `PLUMBER2_ROOT` 与已构建的 `kernels/default`，与
//! `generated_case.rs` 同一档 —— 没有就跳过。

use std::path::{Path, PathBuf};

use anyhow::Result;
use colm_case::{fields, minimal::required, render, CaseSpec, Dirs, Layout, Spinup, Window};
use colm_forcing::convert::{convert, Plan, SlotPlan};
use colm_forcing::{resolve_with, summarize, SLOTS};
use colm_kernel::outcome::Stage;
use colm_kernel::Kernel;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn plumber2() -> Option<PathBuf> {
    std::env::var("PLUMBER2_ROOT").ok().map(PathBuf::from)
}

fn s(p: &Path) -> String {
    p.to_str().expect("utf-8 path").to_string()
}

/// 给 CN-Cng 建一份「全部槽位自动匹配、单位取文件自己的」转换方案 ——
/// 等价于恒等转换（变量名与单位都不变）。两条测试都要产出同一份转换
/// 结果，抽成一个函数避免抄两遍、改一遍漏一遍。
fn identity_plan(src: &Path) -> Result<Plan> {
    let summary = summarize(src)?;
    let (resolved, missing) = resolve_with(&summary.variables, &[]);
    anyhow::ensure!(missing.is_empty(), "CN-Cng 的槽位不该缺：{missing:?}");

    let f = netcdf::open(src)?;
    let mut plan = Plan {
        slots: Vec::new(),
        heights: None,
    };
    for (i, slot) in SLOTS.iter().enumerate() {
        let Some(name) = resolved.vname[i] else {
            continue;
        };
        // 用 `attribute_value`：`attribute` 借用那个 `Variable`，而 `v`
        // 是按值移进闭包的，闭包一结束就 drop，编译不过。
        let units = f
            .variable(name)
            .and_then(|v| v.attribute_value("units"))
            .and_then(|r| r.ok())
            .and_then(|v| match v {
                netcdf::AttributeValue::Str(s) => Some(s),
                _ => None,
            })
            .unwrap_or_default();
        plan.slots.push(SlotPlan {
            index: slot.index,
            source_name: name.to_string(),
            source_units: units,
            also_add: Vec::new(),
        });
    }
    Ok(plan)
}

#[test]
fn a_converted_forcing_matches_the_source_bit_for_bit() {
    let Some(plumber2) = plumber2() else {
        eprintln!("PLUMBER2_ROOT not set — skipping");
        return;
    };
    let repo = repo();
    let kernel_dir = repo.join("kernels/default");
    if !kernel_dir.join("manifest.json").exists() {
        eprintln!("no kernel at {} — skipping", kernel_dir.display());
        return;
    }

    let src = plumber2.join("Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc");
    let work = repo.join("oracle/work/forcing-convert");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");
    let dst = work.join("CN-Cng_converted_Met.nc");

    let plan = identity_plan(&src).expect("build identity plan");
    convert(&src, &dst, &plan).expect("convert");

    // 逐位比对：转出来的每个槽位变量都要与源文件一模一样。
    // **先在文件层面比，跑模型之前就该露馅。**
    let a = netcdf::open(&src).expect("open src");
    let b = netcdf::open(&dst).expect("open dst");
    for sp in &plan.slots {
        let want: Vec<f64> = a
            .variable(&sp.source_name)
            .expect("src var")
            .get_values(netcdf::Extents::All)
            .expect("src values");
        let slot = SLOTS.iter().find(|s| s.index == sp.index).unwrap();
        let got: Vec<f64> = b
            .variable(slot.candidates[0])
            .expect("dst var")
            .get_values(netcdf::Extents::All)
            .expect("dst values");
        assert_eq!(
            got, want,
            "slot {} ({}) 转换后与源文件不逐位相同",
            sp.index, slot.meaning
        );
    }
    println!(
        "identical (file level): {} slot variable(s) bit-for-bit",
        plan.slots.len()
    );
}

/// 文件层面相同不等于模型跑出来相同——比如属性丢了会让 CoLM 走别的
/// 分支（上一个任务刚修过「三处搬属性都只搬字符串，把 `_FillValue`
/// 丢了」）。这一条把转出来的文件真正建算例、跑完三段，与黄金文件比对。
///
/// 慢（建 srfdata/initdata + 跑一段月，几分钟），标成 `#[ignore]`，
/// 但必须存在——它才是这条管道真正的判据。
///
/// **这条测试写出来的时候是红的，而那正是它的价值。** 当时
/// `convert::convert` 只搬时间轴与八个槽位变量，不搬
/// `reference_height_v/t/q` 这三个标量。CN-Cng 的原始文件带着它们
/// （值都是 6），转换产物没有；`met::summarize` 对缺失的高度回落成
/// `NaN`（`met.rs` 里 `scalar(...).unwrap_or(f64::NAN)`），于是
/// `DEF_forcing%HEIGHT_V/T/Q` 被渲染成 `NaN` 写进 `forcing.nml`。
/// 跑时 `MOD_Forcing.F90:299-309` 见文件里没有这三个变量就不覆盖，
/// `Height_V/T/Q` 一直是 `NaN`，被 CoLMDEBUG 内核的 RangeCheck 当场
/// 判成非法指令中止 —— **而报出来的是「内核编进了 CoLMDEBUG」，
/// 看不出问题在强迫场少了三个标量。**
///
/// 已修（`191fea7`）：规则改成「槽位没消费的变量全搬」，不是特判那三个。
/// 现在跑出 `identical: 129 variables, 10 dimensions`。
///
/// **文件层面那条测试当时是绿的。** 只比八个槽位变量，比不出少了三个
/// 标量 —— 这就是为什么这条慢的必须存在：文件里的数一个不差，模型
/// 照样起不来。
#[test]
#[ignore]
fn a_converted_forcing_reproduces_the_golden_history() {
    let Some(plumber2) = plumber2() else {
        eprintln!("PLUMBER2_ROOT not set — skipping");
        return;
    };
    let repo = repo();
    let kernel_dir = repo.join("kernels/default");
    if !kernel_dir.join("manifest.json").exists() {
        eprintln!("no kernel at {} — skipping", kernel_dir.display());
        return;
    }
    let kernel = Kernel::open(&kernel_dir).expect("kernel opens");

    let src = plumber2.join("Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc");
    let work = repo.join("oracle/work/forcing-convert-case");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");

    // 转换产物单独放一个目录，与 Layout 的 case.nml / out/ 分开。
    let forcing_dir = work.join("forcing");
    std::fs::create_dir_all(&forcing_dir).expect("forcing dir");
    let met_name = "CN-Cng_converted_Met.nc";
    let dst = forcing_dir.join(met_name);
    let plan = identity_plan(&src).expect("build identity plan");
    convert(&src, &dst, &plan).expect("convert");

    let layout = Layout::new(&work);
    std::fs::create_dir_all(layout.out()).expect("out dir");

    // 转换产物自己的元数据——变量名是落地后的规范名，不是源文件的名字
    // （比如 CN-Cng 的标量风 `Wind` 落地成第 6 槽候选名 `Wind_N`）。
    let summary = summarize(&dst).expect("converted met summary");
    assert!(
        colm_forcing::check(&summary, None).is_empty(),
        "the converted forcing file itself must be clean"
    );
    std::fs::write(
        layout.forcing_nml(),
        colm_forcing::render(&colm_forcing::ForcingSpec {
            dir: s(&forcing_dir),
            file: met_name.to_string(),
            met: summary.clone(),
        }),
    )
    .expect("write forcing.nml");

    // 站点身份读自站点文件，不是抄来的
    let site_file = repo.join("oracle/cases/CN-Cng/site.nc");
    let loc = colm_srfdata::site::location(&site_file).expect("site location");

    let spec = CaseSpec {
        name: "CN-Cng".into(),
        site_file: s(&site_file),
        lon: loc.lon,
        lat: loc.lat,
        landtype: loc.landtype,
        // 黄金算例的冬季窗口——照抄 generated_case.rs，否则 history 的
        // 起点会挪。
        window: Window {
            start_year: 2008,
            start_month: 1,
            start_day: 1,
            start_sec: 0,
            end_year: 2008,
            end_month: 1,
            end_day: 11,
            end_sec: 86400,
        },
        timestep_seconds: summary.step_seconds,
        greenwich: summary.is_greenwich(),
        urban: false,
        // 预热单独有测试；这条关心的是转换管道，不是预热——
        // 关掉才会落在黄金文件覆盖的那个月，否则跑 12 年且起点会挪。
        spinup: Spinup::OFF,
        dirs: Dirs {
            rawdata: s(&work.join("rawdata_unused/")),
            runtime: s(&work.join("runtime_unused/")),
            output: s(&layout.out()) + "/",
            forcing_namelist: s(&layout.forcing_nml()),
        },
    };

    let all = fields(&spec);
    let req = required(&all);
    std::fs::write(layout.case_nml(), render(&req)).expect("write case.nml");

    // 三段
    let out = layout.out().join("CN-Cng");
    let const_dir = out.join("restart/const");
    let stages = [
        (Stage::MkSrfData, vec![out.join("landdata/srfdata.nc")]),
        (
            Stage::MkIniData,
            vec![
                const_dir.join("CN-Cng_restart_const_lc2005_w180_s90.nc"),
                const_dir.join("CN-Cng_restart_const_lc2005.nc"),
            ],
        ),
        (Stage::Colm, vec![]),
    ];
    for (stage, artifacts) in &stages {
        let r = colm_kernel::run_stage(&kernel, *stage, &layout.case_nml(), &work, artifacts)
            .expect("stage runs");
        assert!(
            r.succeeded(),
            "{} failed: {:?}\nlog: {}",
            stage.program(),
            r.outcome,
            r.log.display()
        );
    }

    let produced = out.join("history/CN-Cng_hist_2008-01.nc");
    let golden = repo.join("oracle/golden/CN-Cng_hist_2008-01.nc");
    let report = oracle::judge::compare(&golden, &produced).expect("both files open");
    eprintln!(
        "identical: {} variables, {} dimensions (ignoring {:?})",
        report.compared,
        report.dimensions,
        oracle::judge::VOLATILE_ATTRIBUTES
    );
    assert!(
        report.is_identical(),
        "converted-forcing case differs from the golden run:\n{}",
        report.problems.join("\n")
    );
    assert_eq!(report.compared, 129);
}
