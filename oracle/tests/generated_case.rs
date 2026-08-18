//! `colm-case` 生成的算例必须跑出与黄金文件**逐位相同**的 history。
//!
//! 这条比「生成的文件长得对」强得多：它证明生成的配置与手写那份**语义等价**，
//! 而不是看起来等价。等价性的前提已经单独验证过一次 —— 把手写算例里 22 个
//! 等于 CoLM 默认值的字段删掉重跑，history 与黄金文件 `identical: 129 variables`。
//! 所以生成版若对不上，只可能是生成错了。
//!
//! 需要 `PLUMBER2_ROOT` 与一个已构建的内核（`kernels/waterheat`），
//! 与 `golden-run` 同属自托管档。

use std::path::{Path, PathBuf};

use colm_case::{fields, minimal::required, render, CaseSpec, Dirs, Layout, Window};
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

#[test]
fn a_generated_case_reproduces_the_golden_history() {
    let Some(plumber2) = plumber2() else {
        eprintln!("PLUMBER2_ROOT not set — skipping");
        return;
    };
    let repo = repo();
    let kernel_dir = repo.join("kernels/waterheat");
    if !kernel_dir.join("manifest.json").exists() {
        eprintln!("no kernel at {} — skipping", kernel_dir.display());
        return;
    }
    let kernel = Kernel::open(&kernel_dir).expect("kernel opens");

    // 干净的工作目录，与 golden-run 的 oracle/work/<case> 分开，免得互相覆盖
    let work = repo.join("oracle/work/generated");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");
    let layout = Layout::new(&work);
    std::fs::create_dir_all(layout.out()).expect("out dir");

    // 强迫场 namelist：与 golden-run 用同一个生成器
    let met_name = "CN-Cng_2008-2009_FLUXNET2015_Met.nc";
    let forcing_dir = plumber2.join("Forcing");
    let summary = colm_forcing::summarize(&forcing_dir.join(met_name)).expect("met summary");
    assert!(
        colm_forcing::check(&summary, None).is_empty(),
        "the forcing file itself must be clean"
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
        // 黄金算例的冬季窗口
        window: Window {
            start_year: 2008,
            start_month: 1,
            start_day: 1,
            end_year: 2008,
            end_month: 1,
            end_day: 11,
        },
        timestep_seconds: summary.step_seconds,
        dirs: Dirs {
            rawdata: s(&work.join("rawdata_unused/")),
            runtime: s(&work.join("runtime_unused/")),
            output: s(&layout.out()) + "/",
            forcing_namelist: s(&layout.forcing_nml()),
        },
    };

    let all = fields(&spec);
    let req = required(&all);
    assert_eq!(
        req.len(),
        21,
        "expected 21 non-default fields, got {}",
        req.len()
    );
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
    assert!(
        report.is_identical(),
        "generated case differs from the golden run:\n{}",
        report.problems.join("\n")
    );
    assert_eq!(report.compared, 129);
}
