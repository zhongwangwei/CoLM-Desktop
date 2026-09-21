//! 用 **整场统计判据**（`docs/design.md` §8.1 的 Tier 3）判一份 history。
//!
//! ```text
//! tier3-check <history.nc> --observation <obs.nc> [--tolerances oracle/tolerances.toml]
//! ```
//!
//! 预热小时数与两条 R² 下限都从 `[tier3.baselines]` 读，命令行上不给 ——
//! n 随预热变，两边分开写迟早对不上。基线里还写着它对哪些 history 文件名
//! 成立；不沾边的文件直接报"不在作用域内"并以 0 退出，而不是拿湿季的
//! 0.999/0.85 去卡冬季窗口、再报一个假的回归。
//!
//! 为什么需要它：`oracle/tests/metrics.rs` 把 §2.8/§2.8b 那六行指标钉在**黄金
//! 文件**上 —— 它证明的是"Fortran 产出与观测吻合"，也就是基线本身没写错。
//! 而"Rust 产出是否也落在同一条基线上"从来没有被测过：湿季窗口此前连跑都跑不
//! 起来，干季窗口虽然有 Rust 输出，却只与黄金逐变量比过。
//!
//! 逐变量比与整场统计是**互相独立**的两把尺子。实测 `CN-Cng`（干季）逐变量
//! tier2 有 38 条超容差，而它的 `f_rnet` 与观测的 R² 是 0.990 —— 一个忠实移植
//! 的舍入残差不影响统计等价，反过来统计等价也盖不住一条抄错的公式。
//!
//! 三条刻意的接口选择：
//!
//! * **观测文件必须显式给。** 与 `golden-run` 对 `PLUMBER2_ROOT` 的态度一致：
//!   第三方数据不入库，路径就不能是隐含约定。
//! * **判定阈值与作用域只从 `tolerances.toml` 的 `[tier3.baselines]` 读。**
//!   内联这两个数会让"基线"分裂成两份，而 `metrics.rs` 用的正是同一份来源。
//!
//! 退出码：落在作用域内且任一条基线不达标即非零；不在作用域内一律 0。

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use colm_hist::metric::compute;
use colm_hist::obs::{read_1d, time_units};
use colm_hist::pair::{pair, Series};
use colm_hist::time::model_seconds_from_units;
use oracle::tolerances::Tolerances;

/// 观测变量名 → history 变量名。与 `metrics.rs` 的六行表同源。
const ROWS: [(&str, &str); 3] = [("Rnet", "f_rnet"), ("Qh", "f_fsena"), ("Qle", "f_lfevpa")];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tier3-check: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let history = PathBuf::from(
        args.next()
            .context("usage: tier3-check <history.nc> --observation <obs.nc> --spinup-hours <N>")?,
    );
    let mut observation: Option<PathBuf> = None;
    let mut tolerances_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tolerances.toml");
    while let Some(a) = args.next() {
        match a.as_str() {
            "--observation" => {
                observation = Some(PathBuf::from(
                    args.next().context("--observation needs a path")?,
                ))
            }
            "--tolerances" => {
                tolerances_path = PathBuf::from(args.next().context("--tolerances needs a path")?)
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    let observation = observation.context("--observation is required")?;

    let baselines = Tolerances::load(&tolerances_path)?.tier3_baselines()?;
    // `!(x >= 0.0)` 而不是 `x < 0.0`：NaN 也要拦下来，否则它一路走到配对里
    // 只会表现为"n 少了几个"，看不出是表写错了。
    if baselines.spinup_hours.is_nan() || baselines.spinup_hours < 0.0 {
        bail!(
            "[tier3.baselines] spinup_hours must not be negative, got {}",
            baselines.spinup_hours
        );
    }
    if !baselines.covers(&history) {
        println!(
            "  {} is not covered by [tier3.baselines] (which names {:?}); nothing to judge",
            history.display(),
            baselines.history_suffixes
        );
        return Ok(());
    }
    let spinup_hours = baselines.spinup_hours;
    let obs_units = time_units(&observation)?;
    let o_t = read_1d(&observation, "time").context("observation time")?;
    let m_t = read_1d(&history, "time").context("history time")?;
    // 观测的时间原点写在它自己的 `units` 里，不一定是元旦（实测 AU-Preston 是
    // 从 03:30 起）。用年份硬推会把两条序列整体错开，而错开之后 R² 只会更低，
    // 于是错的那一方反而"看起来更保守" —— 所以这里必须解析 units。
    let m_sec = model_seconds_from_units(&m_t, &obs_units).with_context(|| {
        format!(
            "cannot understand the observation time units {obs_units:?}; \
             the model axis is minutes since 1900 and needs a `seconds since ...` origin"
        )
    })?;
    if m_sec.len() != m_t.len() {
        bail!(
            "history time axis has {} entries but converted to {} seconds",
            m_t.len(),
            m_sec.len()
        );
    }
    // 一步 30 分钟，所以"剔除 N 小时"= 剔除 2N 个模型步。这里按模型自己给出的
    // 步长算，而不是假设 1800 s：黄金窗口是 1800 s，别的算例未必。
    let step_seconds = match m_sec.windows(2).next() {
        Some(window) => window[1] - window[0],
        None => bail!("history time axis needs at least two entries to read its step"),
    };
    if step_seconds <= 0.0 {
        bail!("history time step is {step_seconds} s; the axis is not increasing");
    }
    let spinup_steps = (spinup_hours * 3600.0 / step_seconds).round() as usize;

    println!(
        "  window: {} -> {} ({} steps of {} s), spin-up {} h = {} steps",
        m_t.first().copied().unwrap_or_default(),
        m_t.last().copied().unwrap_or_default(),
        m_t.len(),
        step_seconds,
        spinup_hours,
        spinup_steps
    );

    let mut worst: Vec<String> = Vec::new();
    for (obs_name, model_name) in ROWS {
        let o_v = read_1d(&observation, obs_name).context("observation values")?;
        let o_q = read_1d(&observation, &format!("{obs_name}_qc")).context("observation qc")?;
        let m_v = read_1d(&history, model_name)
            .with_context(|| format!("{model_name} is missing from {}", history.display()))?;
        let series = Series {
            seconds: &o_t,
            values: &o_v,
            qc: &o_q,
        };
        let metrics = compute(&pair(&m_sec, &m_v, &series, spinup_steps))
            .with_context(|| format!("{obs_name}: fewer than two paired samples"))?;
        let (limit, verdict) = match obs_name {
            "Rnet" => (
                Some(baselines.rnet_r2_min),
                if metrics.r2 >= baselines.rnet_r2_min {
                    "ok"
                } else {
                    "BELOW BASELINE"
                },
            ),
            "Qle" => (
                Some(baselines.qle_r2_min),
                if metrics.r2 >= baselines.qle_r2_min {
                    "ok"
                } else {
                    "BELOW BASELINE"
                },
            ),
            // Qh 没有基线：§2.8/§2.8b 都只把它当作"参数未标定的预期偏差"。
            _ => (None, "no baseline"),
        };
        println!(
            "  {obs_name:>4} <- {model_name:<12} n={:3} RMSE={:8.2} bias={:+8.2} R2={:.4} KGE={:+7.3}  {}",
            metrics.n, metrics.rmse, metrics.bias, metrics.r2, metrics.kge, verdict
        );
        if metrics.beta_warning.is_some() {
            println!("       beta warning: the observation mean is near zero, KGE's beta term is not usable");
        }
        if let Some(limit) = limit {
            if metrics.r2 < limit {
                worst.push(format!("{obs_name} R2 {:.6} < {limit}", metrics.r2));
            }
        }
    }
    if !worst.is_empty() {
        bail!("tier3 baselines not met: {}", worst.join("; "));
    }
    Ok(())
}
