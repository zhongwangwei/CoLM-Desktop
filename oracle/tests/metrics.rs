//! design.md §2.8 与 §2.8b 的六行指标表必须能被复现。
//!
//! 观测文件不入库（15 MB + 2.1 MB 的第三方数据），所以本测试需要
//! `PLUMBER2_ROOT`，与 `real_sites.rs` / `real_forcing.rs` 同一档 ——
//! 在自托管 runner 上跑，不进 per-PR 的三平台 job。

use colm_hist::metric::compute;
use colm_hist::obs::read_1d;
use colm_hist::pair::{pair, Series};
use colm_hist::time::model_seconds;
use std::path::PathBuf;

fn plumber2() -> Option<PathBuf> {
    std::env::var("PLUMBER2_ROOT").ok().map(PathBuf::from)
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("golden")
        .join(name)
}

/// 一行期望值。`n` 精确比，其余给打印精度对应的容差。
struct Row {
    obs: &'static str,
    model: &'static str,
    n: usize,
    rmse: f64,
    bias: f64,
    r2: f64,
    kge: f64,
}

fn check(hist: &str, spinup: usize, rows: &[Row]) {
    let Some(root) = plumber2() else {
        eprintln!("PLUMBER2_ROOT not set — skipping");
        return;
    };
    let obs_path = root.join("Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc");
    let hist_path = golden(hist);
    let o_t = read_1d(&obs_path, "time").expect("obs time");
    let m_t = read_1d(&hist_path, "time").expect("model time");
    let m_sec = model_seconds(&m_t, 2008);

    for r in rows {
        let o_v = read_1d(&obs_path, r.obs).expect("obs values");
        let o_q = read_1d(&obs_path, &format!("{}_qc", r.obs)).expect("obs qc");
        let m_v = read_1d(&hist_path, r.model).expect("model values");
        let s = Series {
            seconds: &o_t,
            values: &o_v,
            qc: &o_q,
        };
        let m = compute(&pair(&m_sec, &m_v, &s, spinup)).expect("enough pairs");

        assert_eq!(m.n, r.n, "{} n", r.obs);
        // design.md 只记录到 3 位小数；2e-3 足以覆盖显示舍入，仍能拦住
        // 时间轴、配对规则或物理默认改变造成的真实漂移。
        assert!(
            (m.r2 - r.r2).abs() < 2e-3,
            "{} R² {} vs {}",
            r.obs,
            m.r2,
            r.r2
        );
        assert!(
            (m.rmse - r.rmse).abs() < 0.15,
            "{} RMSE {} vs {}",
            r.obs,
            m.rmse,
            r.rmse
        );
        assert!(
            (m.bias - r.bias).abs() < 0.05,
            "{} bias {} vs {}",
            r.obs,
            m.bias,
            r.bias
        );
        assert!(
            (m.kge - r.kge).abs() < 0.01,
            "{} KGE {} vs {}",
            r.obs,
            m.kge,
            r.kge
        );
    }
}

#[test]
fn the_winter_window_reproduces_section_2_8() {
    // design.md §2.8：剔除冷启动前 8 小时。
    check(
        "CN-Cng_hist_2008-01.nc",
        8,
        &[
            Row {
                obs: "Rnet",
                model: "f_rnet",
                n: 256,
                rmse: 15.05,
                bias: -0.39,
                r2: 0.986,
                kge: 0.828,
            },
            Row {
                obs: "Qh",
                model: "f_fsena",
                n: 253,
                rmse: 46.37,
                bias: 35.09,
                r2: 0.530,
                kge: -11.64,
            },
            Row {
                obs: "Qle",
                model: "f_lfevpa",
                n: 254,
                rmse: 32.47,
                bias: 13.53,
                r2: 0.047,
                kge: -1.45,
            },
        ],
    );
}

#[test]
fn the_wet_window_reproduces_section_2_8b() {
    // design.md §2.8b：剔除前 4 天 = 96 小时。spin-up 与冬季不同，
    // 所以它必须是参数 —— 写死 8 会让这一条整体错位。
    check(
        "CN-Cng-wet_hist_2008-07.nc",
        96,
        &[
            Row {
                obs: "Rnet",
                model: "f_rnet",
                n: 287,
                rmse: 12.99,
                bias: -2.60,
                r2: 0.999,
                kge: 0.943,
            },
            Row {
                obs: "Qh",
                model: "f_fsena",
                n: 287,
                rmse: 38.65,
                bias: -26.56,
                r2: 0.388,
                kge: -1.72,
            },
            Row {
                obs: "Qle",
                model: "f_lfevpa",
                n: 278,
                rmse: 79.47,
                bias: 38.37,
                r2: 0.852,
                kge: 0.327,
            },
        ],
    );
}

#[test]
fn shifting_the_model_clock_by_eight_hours_destroys_the_fit() {
    // design.md §2.8 写着「若时区偏 8 小时，Rnet 不可能对到 0.986」。
    // 这条把那句话变成可执行的 —— 也排除了「剔除前 8 小时」其实是在
    // 补偿一个 8 小时错位的可能（CN-Cng 在 123.5°E，正好 UTC+8）。
    // 实测：平移后 R² 从 0.986 掉到 0.146 / 0.122，RMSE 从约 15 涨到 ~126。
    let Some(root) = plumber2() else { return };
    let obs_path = root.join("Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc");
    let hist_path = golden("CN-Cng_hist_2008-01.nc");
    let o_t = read_1d(&obs_path, "time").expect("obs time");
    let o_v = read_1d(&obs_path, "Rnet").expect("Rnet");
    let o_q = read_1d(&obs_path, "Rnet_qc").expect("Rnet_qc");
    let m_t = read_1d(&hist_path, "time").expect("model time");
    let m_v = read_1d(&hist_path, "f_rnet").expect("f_rnet");
    let s = Series {
        seconds: &o_t,
        values: &o_v,
        qc: &o_q,
    };

    for shift_hours in [-8.0f64, 8.0] {
        let shifted: Vec<f64> = model_seconds(&m_t, 2008)
            .iter()
            .map(|t| t + shift_hours * 3600.0)
            .collect();
        let m = compute(&pair(&shifted, &m_v, &s, 8)).expect("enough pairs");
        assert!(
            m.r2 < 0.3,
            "shifting by {shift_hours}h should ruin R², got {}",
            m.r2
        );
        assert!(m.rmse > 100.0, "and RMSE, got {}", m.rmse);
    }
}

#[test]
fn the_beta_warning_fires_on_exactly_the_two_rows_design_md_calls_out() {
    // §2.8 的冬季 Qh（观测均值 2.8，β=13.64）与 §2.8b 的湿季 Qh
    // （均值 9.9 而模型均值为负，β=−1.69）。其余四行不该报警。
    let Some(root) = plumber2() else { return };
    let obs_path = root.join("Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc");
    let o_t = read_1d(&obs_path, "time").expect("obs time");

    let mut flagged = Vec::new();
    for (hist, spinup) in [
        ("CN-Cng_hist_2008-01.nc", 8),
        ("CN-Cng-wet_hist_2008-07.nc", 96),
    ] {
        let hist_path = golden(hist);
        let m_t = read_1d(&hist_path, "time").expect("model time");
        let m_sec = model_seconds(&m_t, 2008);
        for (o_name, m_name) in [("Rnet", "f_rnet"), ("Qh", "f_fsena"), ("Qle", "f_lfevpa")] {
            let o_v = read_1d(&obs_path, o_name).expect("obs");
            let o_q = read_1d(&obs_path, &format!("{o_name}_qc")).expect("qc");
            let m_v = read_1d(&hist_path, m_name).expect("model");
            let s = Series {
                seconds: &o_t,
                values: &o_v,
                qc: &o_q,
            };
            let m = compute(&pair(&m_sec, &m_v, &s, spinup)).expect("pairs");
            if m.beta_warning.is_some() {
                flagged.push(format!("{hist}:{o_name}"));
            }
        }
    }
    assert_eq!(
        flagged,
        ["CN-Cng_hist_2008-01.nc:Qh", "CN-Cng-wet_hist_2008-07.nc:Qh"]
    );
}
