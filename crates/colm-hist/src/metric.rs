//! 模型-观测配对的统计指标。
//!
//! **KGE 的 β 项在观测均值接近零时失去意义，本模块只标记不改值。**
//! 改值会让 design.md §2.8 / §2.8b 那六行参考指标再也对不上 ——
//! 而那六行正是里程碑 6 的验收标准。

/// 一对配对样本：`(模型, 观测)`。
pub type Pair = (f64, f64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub n: usize,
    pub rmse: f64,
    pub mae: f64,
    /// `mean(模型) - mean(观测)`
    pub bias: f64,
    /// Pearson r 的平方
    pub r2: f64,
    pub kge: f64,
    pub obs_mean: f64,
    pub obs_sd: f64,
    /// KGE 的 β = mean(模型) / mean(观测)
    pub beta: f64,
    /// β 是否不可信，见 `BetaWarning`
    pub beta_warning: Option<BetaWarning>,
}

/// β 项失效的两种情形。实测六行参考指标里各命中一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetaWarning {
    /// 观测均值相对其标准差接近零（`|μo| < 0.1 σo`）。
    /// 实测冬季 Qh：μo=2.8、σo=38.3、比值 0.073，β 涨到 13.55，
    /// KGE 的 −11.56 里有 12.55 来自这一项。
    NearZeroMean,
    /// 模型与观测均值**反号**，比值没有物理意义。
    /// 实测湿季 Qh：μo=9.9 而模型均值为负，β = −1.52。
    OppositeSign,
}

pub fn compute(pairs: &[Pair]) -> Option<Metrics> {
    let n = pairs.len();
    if n < 2 {
        return None; // 一个点算不出方差，也就算不出 r 与 KGE
    }
    let nf = n as f64;
    let mm = pairs.iter().map(|p| p.0).sum::<f64>() / nf;
    let om = pairs.iter().map(|p| p.1).sum::<f64>() / nf;
    let rmse = (pairs.iter().map(|(m, o)| (m - o).powi(2)).sum::<f64>() / nf).sqrt();
    let mae = pairs.iter().map(|(m, o)| (m - o).abs()).sum::<f64>() / nf;
    let cov = pairs.iter().map(|(m, o)| (m - mm) * (o - om)).sum::<f64>();
    let sm_ss = pairs.iter().map(|(m, _)| (m - mm).powi(2)).sum::<f64>();
    let so_ss = pairs.iter().map(|(_, o)| (o - om).powi(2)).sum::<f64>();
    let r = cov / (sm_ss.sqrt() * so_ss.sqrt());
    let beta = mm / om;
    let kge = 1.0
        - ((r - 1.0).powi(2) + (sm_ss.sqrt() / so_ss.sqrt() - 1.0).powi(2) + (beta - 1.0).powi(2))
            .sqrt();
    // 样本标准差（n-1）—— 报给人看的那个，也是判据里的 σo
    let obs_sd = (so_ss / (nf - 1.0)).sqrt();
    let beta_warning = if mm * om < 0.0 {
        Some(BetaWarning::OppositeSign)
    } else if om.abs() < 0.1 * obs_sd {
        Some(BetaWarning::NearZeroMean)
    } else {
        None
    };
    Some(Metrics {
        n,
        rmse,
        mae,
        bias: mm - om,
        r2: r * r,
        kge,
        obs_mean: om,
        obs_sd,
        beta,
        beta_warning,
    })
}

#[cfg(test)]
#[path = "metric_tests.rs"]
mod metric_tests;
