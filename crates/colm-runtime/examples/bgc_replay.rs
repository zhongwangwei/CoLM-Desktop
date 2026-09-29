//! 逐过程回放 Fortran BGC 追踪：对每条记录，拿它前一条记录的状态与物理输入当起点，只跑这一个
//! 阶段，再与这条记录逐位比。每个过程因此能脱离上游误差单独验证。
//!
//! ```text
//! cargo run --release -p colm-runtime --example bgc_replay -- \
//!     <trace.bin> <case.nml> <全局 BGC 常数重启 *_restart_bgc_const_lc*.nc> [阶段名]
//! ```
//!
//! 追踪由 `oracle/scripts/gen_bgc_trace.py` 插桩的 `colm.x` 在 `COLM_BGC_TRACE` 下写出。

use std::path::Path;

use anyhow::{Context, Result};
use colm_core::bgc_driver::{run_stage, BgcPhysics, BgcStep, BgcSwitches};
use colm_core::bgc_state::{BgcDims, BgcState};
use colm_core::bgc_trace::{read_trace, TraceRecord};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| arg != "--crop")
        .collect();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: bgc_replay <trace.bin> <case.nml> <bgc const restart> [stage]"
    );
    let records = read_trace(&std::fs::read(&args[0]).context("cannot read trace")?)?;
    let text = std::fs::read_to_string(&args[1]).context("cannot read case namelist")?;
    let document = colm_namelist::parse(&text)?;
    let constants = colm_runtime::bgc::read_bgc_constants(Path::new(&args[2]))?;
    let pft = colm_runtime::bgc::bgc_pft_constants(&document)?;
    let flag = |name: &str, default: bool| match document.get(name) {
        Some(colm_namelist::Value::Bool(value)) => *value,
        _ => default,
    };
    // `CROP` 是内核宏，namelist 里没有，两类内核的追踪字段也完全相同：由调用者按内核给 `--crop`。
    let crop = std::env::args().any(|arg| arg == "--crop");
    let switches = BgcSwitches {
        crop,
        nitrif: flag("DEF_USE_NITRIF", true),
        fire: flag("DEF_USE_FIRE", false),
        sasu: flag("DEF_USE_SASU", false),
        diag_matrix: flag("DEF_USE_DiagMatrix", false),
        cnsoyfixn: crop && flag("DEF_USE_CNSOYFIXN", true),
        fert: crop && flag("DEF_USE_FERT", true),
        irrigation: crop && flag("DEF_USE_IRRIGATION", false),
        laifeedback: flag("DEF_USE_LAIFEEDBACK", false),
        nostressnitrogen: flag("DEF_USE_NOSTRESSNITROGEN", false),
        campbell: flag("DEF_USE_Campbell_SOIL_MODEL", false),
        rstfac: match document.get("DEF_RSTFAC") {
            Some(colm_namelist::Value::Int(value)) => i32::try_from(*value)?,
            _ => 1,
        },
    };
    let only = args.get(3);
    let mut call = 0;
    let (mut identical, mut differing, mut skipped) = (0, 0, 0);
    for k in 1..records.len() {
        let record = &records[k];
        if record.tag == "begin" {
            call += 1;
            continue;
        }
        if record.tag == "end" || only.is_some_and(|stage| *stage != record.tag) {
            continue;
        }
        let before = &records[k - 1];
        let npft = before.input("pftfrac").context("pftfrac")?.len();
        let mut state = BgcState::new(npft, BgcDims::default());
        state.constants = constants.clone();
        state.load_trace_fields(&before.state)?;
        let mut physics = BgcPhysics::from_trace(before)?;
        // 跳过收支检查的那一步，驱动在两个被追踪的阶段之间把 `skip_balance_check` 复位
        // （`MOD_BGC_driver.F90:157-159`），回放要补上这一句。
        if record.tag == "CNVegStructUpdate" && before.tag != "NBalanceCheck" {
            state.patch.skip_balance_check[0] = false;
        }
        let mut step = BgcStep {
            state: &mut state,
            physics: &mut physics,
            pft: &pft,
            switches,
            irrigation: None,
        };
        if let Err(error) = run_stage(&record.tag, &mut step) {
            println!("call {} {:<34} skipped: {error}", call + 1, record.tag);
            skipped += 1;
            continue;
        }
        let diffs = compare(&state, &physics, record);
        if diffs.is_empty() {
            println!("call {} {:<34} identical", call + 1, record.tag);
            identical += 1;
        } else {
            println!(
                "call {} {:<34} {} fields differ",
                call + 1,
                record.tag,
                diffs.len()
            );
            for line in diffs.iter().take(12) {
                println!("    {line}");
            }
            differing += 1;
        }
    }
    println!("{identical} identical, {differing} differing, {skipped} not ported");
    Ok(())
}

/// 与 Fortran 记录逐位比；Fortran 一侧 n=0（未分配/宏关闭）的字段跳过。
fn compare(state: &BgcState, physics: &BgcPhysics, record: &TraceRecord) -> Vec<String> {
    let mut out = Vec::new();
    let expected_state = record
        .state
        .iter()
        .map(|(name, values)| (name.as_str(), values));
    let actual: std::collections::HashMap<&str, Vec<f64>> = state
        .trace_fields()
        .into_iter()
        .chain(physics.trace_inputs())
        .collect();
    let expected_inputs = record
        .inputs
        .iter()
        .map(|(name, values)| (name.as_str(), values));
    for (name, expected) in expected_inputs.chain(expected_state) {
        if expected.is_empty() {
            continue;
        }
        let Some(got) = actual.get(name) else {
            out.push(format!("{name}: missing on the Rust side"));
            continue;
        };
        if got.len() != expected.len() {
            out.push(format!(
                "{name}: length {} vs Fortran {}",
                got.len(),
                expected.len()
            ));
            continue;
        }
        let bad: Vec<usize> = (0..got.len())
            .filter(|&index| got[index].to_bits() != expected[index].to_bits())
            .collect();
        if let Some(&first) = bad.first() {
            out.push(format!(
                "{name}[{first}]: rust {:e} fortran {:e} ({}/{} differ)",
                got[first],
                expected[first],
                bad.len(),
                got.len()
            ));
        }
    }
    out
}
