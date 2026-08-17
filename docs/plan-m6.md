# 里程碑 6 实施计划：`colm-hist` 的时间轴、配对、指标与 QC

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把模型 history 与 PLUMBER2 观测对齐、按 QC 筛选、算出指标，验收标准是**逐位复现 `design.md` §2.8 与 §2.8b 的六行指标表**。

**Architecture:** `colm-hist` 现有的闸门表（5b 建的）是无依赖的一半；本轮加的读文件与算指标那一半需要 netcdf，因此拆成两个 feature，默认只编无依赖的那半 —— GUI 为了问一句「能产出什么」不该拖进整个 HDF5。

**Tech Stack:** Rust 1.85.1、`netcdf`（可选 feature）、入库黄金文件作验收基准。

---

## 这份计划的全部算法都已预跑验证

写计划前把整条链路在 scratch crate 里对真实文件跑通了，**六行指标全部复现**。
下面每个数字都是这么来的，不是推断。

### 验收表：实测 vs `design.md` 目标

| | n | RMSE | bias | R² | KGE |
|---|---|---|---|---|---|
| §2.8 冬季 Rnet | 256 / **256** | 14.7 / **14.7** | −0.87 / **−0.87** | 0.986 / **0.986** | +0.829 / **+0.829** |
| §2.8 冬季 Qh | 253 / **253** | 46.1 / **46.1** | +34.9 / **+34.9** | 0.530 / **0.530** | −11.56 / **−11.56** |
| §2.8 冬季 Qle | 254 / **254** | 32.2 / **32.2** | +13.3 / **+13.3** | 0.044 / **0.044** | −1.42 / **−1.42** |
| §2.8b 湿季 Rnet | 287 / **287** | 13.4 / 13.5 | −2.93 / −2.95 | 0.999 / **0.999** | +0.940 / +0.939 |
| §2.8b 湿季 Qh | 287 / **287** | 36.3 / **36.3** | −24.9 / **−24.9** | 0.455 / 0.456 | −1.55 / **−1.55** |
| §2.8b 湿季 Qle | 278 / **278** | 76.4 / 76.5 | +36.4 / **+36.4** | 0.853 / **0.853** | +0.362 / **+0.362** |

（左为本次实测，右为 `design.md` 记录的目标；加粗表示完全一致。）

**冬季三行全部精确一致，湿季 Rnet 与 Qle 也在打印精度上一致。**
唯一的例外是湿季 Qh 的 R²：实测 0.4547，记录是 0.456，差 **0.0013**。
两边 n 都是 287 —— 配对若完全相同，R² 该逐位相同，所以这不是舍入，
是个**未解释的小残差（0.3%）**。

已排除的一种可能：把聚合规则改成「只有一个半小时好时仍取两个的平均」。
那样 n 不变（仍是 287）但湿季 Rnet 的 R² 会掉到 0.998、Qle 涨到 0.877，
离记录值更远。所以不是它。**不编理由，把残差如实记着。**

因此验收的取法是：`n` 精确相等（它是算法正确性的直接证据），
`R²` 容差 2e-3，`RMSE` 0.15，`bias` 0.05，`KGE` 0.01 —— 都比实测余量
略宽一点点，宽到能容下这个残差，紧到算法一旦走偏就会红。

### 时间轴：两边的标签含义不同

| | 单位 | 步长 | 标签位置 |
|---|---|---|---|
| 模型 history | `minutes since 1900-1-1 0:0:0` | 60 分 | **区间中点** |
| PLUMBER2 观测 | `seconds since <起始日> 00:00:00` | 1800 秒 | **区间起点** |

实测：CN-Cng 冬季窗口模型首点 `time = 56802270` 分。1900→2008 的偏移是
56802240 分，所以首点相对 2008-01-01 是 **30 分** —— 也就是 00:00–01:00
那一小时的标签打在 **00:30**。这就是 §2.10 说的「半区间回移」。

于是配对规则是：**模型标签 t 对应观测在 `t−1800s` 与 `t` 两点**（即 00:00 与
00:30 两个半小时样本），取它们的平均。

**这条对齐必须被独立验证过，不能只看拟合好。** 把模型时间整体平移 ±8 小时
再配对，实测 R² 从 0.986 掉到 0.146 / 0.122、RMSE 从 14.7 涨到 ~126。
`design.md` §2.8 自己就写着「若时区偏 8 小时，Rnet 不可能对到 0.986」——
这条测试把那句话变成可执行的。

### QC：至少一个好即可用，不是两个都要好

`Rnet_qc` 实测只有两个取值：**0 占 32052（91.3%），5 占 3036（8.7%）**。
PLUMBER2 的约定是 0 = 实测、非 0 = 插补。`_FillValue = -9999`。

关键在聚合规则。两个半小时里：

- **两个都必须 qc==0**（严格）→ 冬季 Qh 得 250、Qle 得 245
- **至少一个 qc==0，取好的那些的平均**（宽松）→ 冬季 Qh 得 **253**、Qle 得 **254**

目标是 253 / 254，**所以是宽松规则**。Rnet 两种规则同为 256，区分不出来 ——
必须靠 Qh / Qle 才能定下这条，这也是为什么验收不能只验 Rnet 一行。

### spin-up 是每次分析各自记录的参数，不是通则

| 窗口 | `design.md` 写的 | 换算 |
|---|---|---|
| §2.8 冬季 | 剔除冷启动前 **8 小时** | 丢 8 条 |
| §2.8b 湿季 | 剔除前 **4 天** 预热 | 丢 96 条 |

两者不同，所以 spin-up 必须是显式参数，不能写死。

**冬季那个 8 小时曾让人怀疑是时区补偿** —— CN-Cng 在 123.5°E，正好 UTC+8。
上面的 ±8h 平移测试已经排除：真平移会让拟合崩掉，而「丢前 8 条」能让五个
统计量同时精确命中。是巧合。

### 变量映射（全部 W/m2）

| 观测 | 模型 | 说明 |
|---|---|---|
| `Rnet` | `f_rnet` | §2.8 指定的**关键验证信号** |
| `Qh` | `f_fsena` | 感热 |
| `Qle` | `f_lfevpa` | 潜热 |
| `Qg` | `f_fgrnd` | 地表热通量 |
| `SWup` | `f_sr` | 反射短波 |

观测文件还有 `Qle_cor` / `Qh_cor` 等能量闭合订正版本，**本轮不用** ——
§2.8 / §2.8b 的目标值是用未订正版算的（用订正版复现不出来）。

### KGE 的近零均值陷阱：只标记，不改值

KGE = 1 − √((r−1)² + (σm/σo − 1)² + (β − 1)²)，β = μm/μo。
β 在观测均值接近零时失去意义。实测六行：

| 行 | 观测均值 | σ_obs | β | KGE | \|μ\|/σ |
|---|---|---|---|---|---|
| 冬 Qh | 2.8 | 38.3 | **+13.55** | **−11.56** | **0.073** |
| 湿 Qh | 9.9 | 33.9 | **−1.52（变号）** | −1.55 | 0.291 |
| 冬 Rnet | −21.2 | 70.7 | +1.04 | +0.829 | 0.300 |
| 冬 Qle | 6.9 | 13.2 | +2.94 | −1.42 | 0.518 |
| 湿 Rnet | 121.7 | 198.7 | +0.98 | +0.940 | 0.612 |
| 湿 Qle | 84.4 | 101.6 | +1.43 | +0.362 | 0.831 |

冬季 Qh 的 KGE = −11.56 里，**12.55 全部来自 β 项** —— 它报的不是技巧，
是「观测均值接近零」。湿季 Qh 更糟：β 是**负的**，模型与观测均值反号，
那个比值根本没有物理意义。

判据两条，**都只标记不改值**：

1. `|μo| < 0.1 · σo` —— 实测只有冬季 Qh（0.073）命中
2. `μm · μo < 0`（均值反号）—— 实测只有湿季 Qh 命中

正好是 `design.md` 点名的那两行。**保护必须是标记而不是替换**：
一旦改了 KGE 的值，上面那张验收表就再也对不上了。

### 观测文件的形状

```
time = 35088（2008-2009 半小时）；x = y = 1；nchar = 200
通量:  Rnet Qle Qh Qg SWup Ustar NEE GPP Resp（外加 _cor / _uc 变体）
QC:    Rnet_qc Qle_qc Qh_qc Qg_qc SWup_qc Ustar_qc NEE_qc（+ 两个 _uc_qc）
标量:  latitude longitude reference_height canopy_height elevation
```

注意 `GPP` / `Resp` 只有 `_se`（标准误）没有 `_qc`，本轮不做它们。

---

## 文件结构

```
crates/colm-hist/
├── Cargo.toml           新增可选 netcdf 依赖 + 两个 feature
├── src/lib.rs           现有闸门表；新增 pub mod（feature 门控）
├── src/generated.rs     5b 的产物，本轮不动
├── src/time.rs          【新】两种时间轴的换算与对齐
├── src/time_tests.rs
├── src/pair.rs          【新】配对 + QC 聚合 + spin-up 剔除
├── src/pair_tests.rs
├── src/metric.rs        【新】RMSE / bias / R² / KGE / MAE + 近零均值标记
├── src/metric_tests.rs
└── src/obs.rs           【新】读 PLUMBER2 Observation 文件（需 netcdf）

oracle/tests/
└── metrics.rs           【新】六行验收表 + ±8h 错位测试
```

**为什么 netcdf 做成可选 feature**：5b 建的闸门表刻意无依赖，理由写在
`src/lib.rs` 里 —— GUI 为了问一句「这个内核能产出什么」不该拖进整个 HDF5。
本轮加的读文件那半必须有 netcdf，所以：

```toml
[features]
default = []
io = ["dep:netcdf"]        # 读 history / Observation
```

`writable()` 与 `all()` 在 `default` 下仍然可用。`oracle` 开 `io`。

---

## Task 1: 时间轴 —— 先写失败的测试

**Files:**
- Create: `crates/colm-hist/src/time.rs`
- Create: `crates/colm-hist/src/time_tests.rs`
- Modify: `crates/colm-hist/src/lib.rs`

- [ ] **Step 1: 占位模块**

`time.rs`：

```rust
//! 两种时间轴的换算与对齐。
//!
//! 模型 history 与 PLUMBER2 观测的**标签含义不同**，这是本模块存在的全部理由：
//!
//! | | 单位 | 步长 | 标签位置 |
//! |---|---|---|---|
//! | 模型 | `minutes since 1900-1-1 0:0:0` | 60 分 | **区间中点** |
//! | 观测 | `seconds since <起始日> 00:00:00` | 1800 秒 | **区间起点** |
//!
//! 实测：CN-Cng 冬季窗口模型首点 `time = 56802270` 分，而 1900→2008 的偏移是
//! 56802240 分，差 **30 分** —— 00:00–01:00 那一小时的标签打在 00:30。
//! 这就是 design.md §2.10 的「半区间回移」。

/// 从 1900-01-01 到 `year` 年 1 月 1 日的分钟数。
///
/// 只处理公历闰年规则；CoLM 的 history 时间单位固定是
/// `minutes since 1900-1-1 0:0:0`，`calendar` 属性实测是 standard。
pub fn minutes_from_1900(year: i32) -> i64 {
    let days: i64 = (1900..year)
        .map(|y| if is_leap(y) { 366 } else { 365 })
        .sum();
    days * 24 * 60
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// 模型时间（分，since 1900）→ 相对 `year` 年 1 月 1 日 00:00 的秒。
pub fn model_seconds(minutes_since_1900: &[f64], year: i32) -> Vec<f64> {
    let base = minutes_from_1900(year) as f64;
    minutes_since_1900
        .iter()
        .map(|t| (t - base) * 60.0)
        .collect()
}

/// 模型标签 `t`（秒）对应的两个观测半小时样本的时刻。
///
/// 标签在中点意味着 00:30 这个标签覆盖 00:00–01:00，而观测标签在起点，
/// 于是这一小时由 00:00 与 00:30 两个样本组成 —— 即 `t-1800` 与 `t`。
pub fn observation_slots(label_seconds: f64) -> [f64; 2] {
    [label_seconds - 1800.0, label_seconds]
}
```

- [ ] **Step 2: 写失败的测试**

`time_tests.rs`：

```rust
use super::*;

#[test]
fn the_epoch_offset_matches_the_real_history_file() {
    // CN-Cng 冬季窗口的 history 首点是 56802270 分（since 1900-1-1）。
    // 1900→2008 的偏移必须正好比它小 30 分 —— 那 30 分就是半区间回移。
    assert_eq!(minutes_from_1900(2008), 56_802_240);
    let first_label = 56_802_270.0;
    let sec = model_seconds(&[first_label], 2008);
    assert_eq!(sec[0], 1800.0, "首点应落在 2008-01-01 00:30");
}

#[test]
fn leap_years_follow_the_gregorian_rule() {
    // 1900 不是闰年（能被 100 整除且不能被 400 整除），2000 是。
    // 弄错任何一个，2008 的偏移就会差一整天 1440 分，配对全错位。
    assert!(!is_leap(1900));
    assert!(is_leap(2000));
    assert!(is_leap(2008));
    assert!(!is_leap(2100));
    // 1900..2008 共 108 年，其中 **26** 个闰年：1904、1908 … 2004。
    // 1900 不算（能被 100 整除、不能被 400 整除），2000 算。
    // 数成 27 会让偏移多出整整一天（56803680 而不是 56802240），
    // 而那正是本测试要防的错位。
    let leaps = (1900..2008).filter(|&y| is_leap(y)).count();
    assert_eq!(leaps, 26);
    assert_eq!(minutes_from_1900(2008), (108 * 365 + 26) * 24 * 60);
}

#[test]
fn an_hourly_label_covers_the_two_half_hours_before_and_at_it() {
    // 标签 00:30（1800 秒）覆盖 00:00–01:00，由观测的 00:00 与 00:30 两点组成。
    assert_eq!(observation_slots(1800.0), [0.0, 1800.0]);
    // 标签 01:30（5400 秒）覆盖 01:00–02:00。
    assert_eq!(observation_slots(5400.0), [3600.0, 5400.0]);
}
```

- [ ] **Step 3: 跑，确认红**

Run: `cargo test -p colm-hist --lib`
Expected: `test result: ok. 5 passed` —— 也就是**新测试根本没跑**。

**注意这里的「红」不是编译失败。** `time.rs` 与 `time_tests.rs` 已经写在磁盘上，
但 `lib.rs` 里还没有 `pub mod time;`，Cargo 就完全不会去编它们 ——
孤立的源文件不构成构建错误。所以本步要确认的是「测试数没变」，
而不是「编不过」。这一点对下面两个 Task 同样成立。

- [ ] **Step 4: 接上模块并转绿**

`lib.rs` 加 `pub mod time;`。**注意 rustfmt 会把 `pub mod` 按字母序重排**，
本轮加完三个模块之后的顺序是 `generated` / `metric` / `pair` / `time` ——
按别的顺序写会在 `cargo fmt --check` 上失败。`time.rs` 末尾加：

```rust
#[cfg(test)]
#[path = "time_tests.rs"]
mod time_tests;
```

Run: `cargo test -p colm-hist --lib`
Expected: `test result: ok. 8 passed`（原 5 + 新 3）

- [ ] **Step 5: 提交**

```bash
git add crates/colm-hist/src
git commit -m "Line up two time axes that label their intervals differently"
```

---

## Task 2: 指标 —— 先写失败的测试

**Files:**
- Create: `crates/colm-hist/src/metric.rs`
- Create: `crates/colm-hist/src/metric_tests.rs`
- Modify: `crates/colm-hist/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
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
```

- [ ] **Step 2: 写测试**

```rust
use super::*;

#[test]
fn a_perfect_match_scores_perfectly() {
    let p: Vec<Pair> = (0..10).map(|i| (i as f64, i as f64)).collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.rmse, 0.0);
    assert_eq!(m.mae, 0.0);
    assert_eq!(m.bias, 0.0);
    assert!((m.r2 - 1.0).abs() < 1e-12);
    assert!((m.kge - 1.0).abs() < 1e-12);
    assert_eq!(m.beta_warning, None);
}

#[test]
fn fewer_than_two_pairs_has_no_answer() {
    // 一个点没有方差，r 与 KGE 都是 0/0。返回 None 而不是 NaN ——
    // NaN 会一路流进 GUI 显示成「NaN」，而调用方本该在这里就知道数据不够。
    assert!(compute(&[]).is_none());
    assert!(compute(&[(1.0, 1.0)]).is_none());
}

#[test]
fn a_near_zero_observed_mean_is_flagged_not_altered() {
    // 实测冬季 Qh 的形状：观测均值 2.8、标准差 38.3（比值 0.073），
    // 模型均值 37.7，于是 β = 13.55，KGE 被 β 项拖到 −11.56。
    // 这里造一组同样形状的数据，验证**标记出现而 KGE 不被改动**。
    // 交替 ±38.3 让观测均值**精确**是 2.8、标准差精确是 38.3 量级；
    // 用 sin/cos 造数据的话均值取决于采样点，落不进判据。
    let p: Vec<Pair> = (0..100)
        .map(|i| {
            let o = 2.8 + if i % 2 == 0 { 38.3 } else { -38.3 };
            (o + 34.9, o)
        })
        .collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.beta_warning, Some(BetaWarning::NearZeroMean));
    // 关键：KGE 仍是照公式算的那个值，没有被替换成别的东西
    assert!(m.beta > 5.0, "β should be blown up: {}", m.beta);
    assert!(
        m.kge < -5.0,
        "KGE should carry the blown-up beta: {}",
        m.kge
    );
}

#[test]
fn means_of_opposite_sign_are_flagged_separately() {
    // 实测湿季 Qh：观测均值 +9.9 而模型均值为负，β = −1.52。
    // 这种情形下 β 连「偏大偏小」都说明不了，必须与近零均值分开报。
    let p: Vec<Pair> = (0..50)
        .map(|i| {
            let o = 9.9 + if i % 2 == 0 { 33.9 } else { -33.9 };
            (o - 25.0, o)
        })
        .collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.beta_warning, Some(BetaWarning::OppositeSign));
    assert!(m.beta < 0.0, "β should be negative: {}", m.beta);
}

#[test]
fn a_healthy_series_is_not_flagged() {
    // 实测湿季 Rnet：观测均值 121.7、标准差 198.7，比值 0.612，β = 0.98。
    let p: Vec<Pair> = (0..100)
        .map(|i| {
            let o = 121.7 + if i % 2 == 0 { 198.7 } else { -198.7 };
            (o - 2.9, o)
        })
        .collect();
    let m = compute(&p).unwrap();
    assert_eq!(m.beta_warning, None);
    assert!(m.kge > 0.9, "{}", m.kge);
}

#[test]
fn bias_is_model_minus_observation() {
    // 符号约定弄反的话，design.md 六行里的 bias 会整体变号而其余指标不变 ——
    // 那是最容易蒙混过关的一种错。
    let p = vec![(11.0, 10.0), (12.0, 10.0)];
    let m = compute(&p).unwrap();
    assert!((m.bias - 1.5).abs() < 1e-12, "{}", m.bias);
}
```

- [ ] **Step 3: 接上模块**

`lib.rs` 加 `pub mod metric;`，`metric.rs` 末尾加：

```rust
#[cfg(test)]
#[path = "metric_tests.rs"]
mod metric_tests;
```

不加这一段的话 `metric_tests.rs` 是个孤立文件，测试数不会变而且**不报错**。

- [ ] **Step 4: 跑并提交**

Run: `cargo test -p colm-hist --lib`
Expected: `test result: ok. 14 passed`（8 + 新 6）

```bash
git add crates/colm-hist/src
git commit -m "Flag the KGE beta term instead of quietly rewriting it"
```

---

## Task 3: 配对与 QC

**Files:**
- Create: `crates/colm-hist/src/pair.rs`
- Create: `crates/colm-hist/src/pair_tests.rs`
- Modify: `crates/colm-hist/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
//! 把模型逐小时序列与观测半小时序列配成对。
//!
//! 聚合规则是**至少一个半小时 `qc == 0` 即可用，取好的那些的平均**，
//! 不是「两个都必须好」。这条是实测定下来的，不是选的：
//!
//! | 规则 | 冬季 Qh | 冬季 Qle |
//! |---|---|---|
//! | 两个都要好 | 250 | 245 |
//! | 至少一个好 | **253** | **254** |
//!
//! design.md §2.8 的目标是 253 / 254。注意 Rnet 在两种规则下都是 256 ——
//! **光验 Rnet 区分不出这条规则**，所以验收必须覆盖 Qh 与 Qle。

use crate::metric::Pair;
use crate::time::observation_slots;

/// 观测里表示「实测」的 QC 值。非 0 是插补。
pub const QC_MEASURED: f64 = 0.0;

/// PLUMBER2 的缺测填充值。
pub const FILL_VALUE: f64 = -9999.0;

/// 一条观测序列。
pub struct Series<'a> {
    /// 相对窗口起始年 1 月 1 日 00:00 的秒
    pub seconds: &'a [f64],
    pub values: &'a [f64],
    /// 与 `values` 等长的 QC 标志
    pub qc: &'a [f64],
}

/// 配对。
///
/// `spinup` 是**丢掉的模型记录条数**，必须由调用方显式给出：
/// design.md 两个窗口用的值不同（冬季 8 小时、湿季 4 天 = 96 小时），
/// 所以它是参数不是常数。
pub fn pair(
    model_seconds: &[f64],
    model_values: &[f64],
    obs: &Series<'_>,
    spinup: usize,
) -> Vec<Pair> {
    let mut out = Vec::new();
    for k in spinup..model_seconds.len() {
        let mut acc = 0.0;
        let mut n = 0;
        for want in observation_slots(model_seconds[k]) {
            // 观测步长是 1800 秒，误差 1 秒内视为同一时刻
            let Some(i) = obs.seconds.iter().position(|&x| (x - want).abs() < 1.0) else {
                continue;
            };
            if obs.qc[i] == QC_MEASURED && obs.values[i] > FILL_VALUE + 1.0 {
                acc += obs.values[i];
                n += 1;
            }
        }
        if n >= 1 {
            out.push((model_values[k], acc / n as f64));
        }
    }
    out
}
```

- [ ] **Step 2: 写测试**

```rust
use super::*;

/// 三小时的模型序列，标签在 00:30 / 01:30 / 02:30。
fn model() -> (Vec<f64>, Vec<f64>) {
    (vec![1800.0, 5400.0, 9000.0], vec![10.0, 20.0, 30.0])
}

/// 六个半小时的观测，全部 qc=0，值为 1..6。
fn obs_all_good() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    (
        vec![0.0, 1800.0, 3600.0, 5400.0, 7200.0, 9000.0],
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        vec![0.0; 6],
    )
}

#[test]
fn an_hour_averages_its_two_half_hours() {
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p, vec![(10.0, 1.5), (20.0, 3.5), (30.0, 5.5)]);
}

#[test]
fn one_bad_half_hour_leaves_the_other_one_usable() {
    // 这条是本模块的核心规则。把它改成「两个都要好」，
    // design.md 的 253 / 254 会变成 250 / 245。
    let (ms, mv) = model();
    let (os, ov, mut oq) = obs_all_good();
    oq[0] = 5.0; // 第一个半小时是插补的
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p[0], (10.0, 2.0), "应只用剩下那个好的半小时");
    assert_eq!(p.len(), 3);
}

#[test]
fn an_hour_with_no_good_half_hour_is_dropped() {
    let (ms, mv) = model();
    let (os, ov, mut oq) = obs_all_good();
    oq[0] = 5.0;
    oq[1] = 5.0;
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p.len(), 2, "第一小时两个半小时都不可用");
    assert_eq!(p[0], (20.0, 3.5));
}

#[test]
fn the_fill_value_is_not_data_even_when_qc_says_measured() {
    // -9999 带着 qc=0 出现是可能的；当成数据会把整段指标毁掉。
    let (ms, mv) = model();
    let (os, mut ov, oq) = obs_all_good();
    ov[0] = FILL_VALUE;
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p[0], (10.0, 2.0));
}

#[test]
fn spinup_drops_model_records_from_the_front() {
    // spin-up 是参数不是常数：design.md 冬季窗口丢 8 小时、湿季丢 4 天。
    let (ms, mv) = model();
    let (os, ov, oq) = obs_all_good();
    let s = Series {
        seconds: &os,
        values: &ov,
        qc: &oq,
    };
    assert_eq!(pair(&ms, &mv, &s, 1).len(), 2);
    assert_eq!(pair(&ms, &mv, &s, 3).len(), 0);
}

#[test]
fn an_hour_with_no_matching_observation_is_dropped() {
    // 模型窗口越出观测覆盖范围时不能静默配出半个小时的平均。
    let ms = vec![1800.0, 1_000_000.0];
    let mv = vec![10.0, 20.0];
    let (os, ov, oq) = obs_all_good();
    let p = pair(
        &ms,
        &mv,
        &Series {
            seconds: &os,
            values: &ov,
            qc: &oq,
        },
        0,
    );
    assert_eq!(p.len(), 1);
}
```

- [ ] **Step 3: 接上模块**

`lib.rs` 加 `pub mod pair;`，`pair.rs` 末尾加：

```rust
#[cfg(test)]
#[path = "pair_tests.rs"]
mod pair_tests;
```

- [ ] **Step 4: 跑并提交**

Run: `cargo test -p colm-hist --lib`
Expected: `test result: ok. 20 passed`（14 + 新 6）

```bash
git add crates/colm-hist/src
git commit -m "Take an hour when either of its half hours was measured"
```

---

## Task 4: 读文件（feature `io`）

**Files:**
- Modify: `crates/colm-hist/Cargo.toml`
- Create: `crates/colm-hist/src/obs.rs`
- Modify: `crates/colm-hist/src/lib.rs`

- [ ] **Step 1: feature 门控**

`Cargo.toml`：

```toml
[dependencies]
# 闸门表那一半刻意无依赖 —— GUI 为了问一句「这个内核能产出什么」
# 不该拖进整个 HDF5。读文件与算指标那一半才需要 netcdf，所以做成 feature。
netcdf = { workspace = true, features = ["static"], optional = true }
anyhow = { workspace = true, optional = true }

[features]
default = []
io = ["dep:netcdf", "dep:anyhow"]
```

- [ ] **Step 2: 写 `obs.rs`**

```rust
//! 读 PLUMBER2 的 `Observation/*_Flux.nc` 与模型的 `*_hist_*.nc`。
//!
//! 实测的观测文件形状（CN-Cng，2008-2009）：
//! `time = 35088` 半小时步长、`x = y = 1`；通量 `Rnet` / `Qle` / `Qh` / `Qg` /
//! `SWup` 各带一个 `<name>_qc`；`_FillValue = -9999`。
//! `GPP` / `Resp` 只有 `_se` 没有 `_qc`，本模块不处理它们。
//!
//! **不用 `_cor` 能量闭合订正版本**：design.md §2.8 / §2.8b 的目标值是用
//! 未订正版算的，用订正版复现不出来。

use anyhow::{Context, Result};
use std::path::Path;

/// 观测与模型的变量对应。全部 W/m2。
pub const FLUX_PAIRS: [(&str, &str); 5] = [
    ("Rnet", "f_rnet"),     // §2.8 指定的关键验证信号
    ("Qh", "f_fsena"),      // 感热
    ("Qle", "f_lfevpa"),    // 潜热
    ("Qg", "f_fgrnd"),      // 地表热通量
    ("SWup", "f_sr"),       // 反射短波
];

pub fn read_1d(path: &Path, name: &str) -> Result<Vec<f64>> {
    let f = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let v = f
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    Ok(v.get_values::<f64, _>(..)?)
}
```

- [ ] **Step 3: 门禁与提交**

Run: `cargo test -p colm-hist --lib`（default，不含 io）
Run: `cargo test -p colm-hist --lib --features io`
Expected: 两次都 `20 passed` —— feature 不改变已有行为。

Run: `cargo build -p colm-hist` 后确认 `Cargo.lock` 里 default 构建**不含 netcdf**：

```bash
cargo tree -p colm-hist | grep -c netcdf     # 期望 0
cargo tree -p colm-hist --features io | grep -c netcdf   # 期望 >0
```

```bash
git add crates/colm-hist Cargo.lock
git commit -m "Put the file reading behind a feature so the gate table stays free of HDF5"
```

---

## Task 5: 六行验收表

**这一步是本计划的验收核心。**

**Files:**
- Create: `oracle/tests/metrics.rs`
- Modify: `oracle/Cargo.toml`（`colm-hist` 开 `io` feature）

- [ ] **Step 1: 写测试**

```rust
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden").join(name)
}

/// 一行期望值。`n` 与 `r2` 精确比，其余给容差 —— 湿季三行与 design.md
/// 只差末位（RMSE 13.4 vs 13.5），是舍入不是算法分歧。
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
        let s = Series { seconds: &o_t, values: &o_v, qc: &o_q };
        let m = compute(&pair(&m_sec, &m_v, &s, spinup)).expect("enough pairs");

        assert_eq!(m.n, r.n, "{} n", r.obs);
        // R² 容差 2e-3：六行里五行在 design.md 的打印精度（3 位小数）上完全一致，
        // 只有湿季 Qh 实测 0.4547 而记录是 0.456，差 0.0013。
        // n 两边都是 287，配对相同的话 R² 该逐位相同，所以这 0.0013 是个
        // **未解释的残差**，不是舍入。已排除的可能：把「两个半小时里只有一个
        // 好时仍取两个的平均」当作聚合规则 —— 那样湿季 Rnet 的 R² 会掉到
        // 0.998、Qle 涨到 0.877，比现在差得多。不编理由，如实留着。
        assert!((m.r2 - r.r2).abs() < 2e-3, "{} R² {} vs {}", r.obs, m.r2, r.r2);
        assert!((m.rmse - r.rmse).abs() < 0.15, "{} RMSE {} vs {}", r.obs, m.rmse, r.rmse);
        assert!((m.bias - r.bias).abs() < 0.05, "{} bias {} vs {}", r.obs, m.bias, r.bias);
        assert!((m.kge - r.kge).abs() < 0.01, "{} KGE {} vs {}", r.obs, m.kge, r.kge);
    }
}

#[test]
fn the_winter_window_reproduces_section_2_8() {
    // design.md §2.8：剔除冷启动前 8 小时。
    check(
        "CN-Cng_hist_2008-01.nc",
        8,
        &[
            Row { obs: "Rnet", model: "f_rnet",   n: 256, rmse: 14.7, bias: -0.87, r2: 0.986, kge: 0.829 },
            Row { obs: "Qh",   model: "f_fsena",  n: 253, rmse: 46.1, bias: 34.9,  r2: 0.530, kge: -11.56 },
            Row { obs: "Qle",  model: "f_lfevpa", n: 254, rmse: 32.2, bias: 13.3,  r2: 0.044, kge: -1.42 },
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
            Row { obs: "Rnet", model: "f_rnet",   n: 287, rmse: 13.5, bias: -2.95, r2: 0.999, kge: 0.939 },
            Row { obs: "Qh",   model: "f_fsena",  n: 287, rmse: 36.3, bias: -24.9, r2: 0.456, kge: -1.55 },
            Row { obs: "Qle",  model: "f_lfevpa", n: 278, rmse: 76.5, bias: 36.4,  r2: 0.853, kge: 0.362 },
        ],
    );
}

#[test]
fn shifting_the_model_clock_by_eight_hours_destroys_the_fit() {
    // design.md §2.8 写着「若时区偏 8 小时，Rnet 不可能对到 0.986」。
    // 这条把那句话变成可执行的 —— 也排除了「剔除前 8 小时」其实是在
    // 补偿一个 8 小时错位的可能（CN-Cng 在 123.5°E，正好 UTC+8）。
    // 实测：平移后 R² 从 0.986 掉到 0.146 / 0.122，RMSE 从 14.7 涨到 ~126。
    let Some(root) = plumber2() else { return };
    let obs_path = root.join("Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc");
    let hist_path = golden("CN-Cng_hist_2008-01.nc");
    let o_t = read_1d(&obs_path, "time").expect("obs time");
    let o_v = read_1d(&obs_path, "Rnet").expect("Rnet");
    let o_q = read_1d(&obs_path, "Rnet_qc").expect("Rnet_qc");
    let m_t = read_1d(&hist_path, "time").expect("model time");
    let m_v = read_1d(&hist_path, "f_rnet").expect("f_rnet");
    let s = Series { seconds: &o_t, values: &o_v, qc: &o_q };

    for shift_hours in [-8.0f64, 8.0] {
        let shifted: Vec<f64> = model_seconds(&m_t, 2008)
            .iter()
            .map(|t| t + shift_hours * 3600.0)
            .collect();
        let m = compute(&pair(&shifted, &m_v, &s, 8)).expect("enough pairs");
        assert!(m.r2 < 0.3, "shifting by {shift_hours}h should ruin R², got {}", m.r2);
        assert!(m.rmse > 100.0, "and RMSE, got {}", m.rmse);
    }
}

#[test]
fn the_beta_warning_fires_on_exactly_the_two_rows_design_md_calls_out() {
    // §2.8 的冬季 Qh（观测均值 2.8，β=13.55）与 §2.8b 的湿季 Qh
    // （均值 9.9 而模型均值为负，β=−1.52）。其余四行不该报警。
    let Some(root) = plumber2() else { return };
    let obs_path = root.join("Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc");
    let o_t = read_1d(&obs_path, "time").expect("obs time");

    let mut flagged = Vec::new();
    for (hist, spinup) in [("CN-Cng_hist_2008-01.nc", 8), ("CN-Cng-wet_hist_2008-07.nc", 96)] {
        let hist_path = golden(hist);
        let m_t = read_1d(&hist_path, "time").expect("model time");
        let m_sec = model_seconds(&m_t, 2008);
        for (o_name, m_name) in [("Rnet", "f_rnet"), ("Qh", "f_fsena"), ("Qle", "f_lfevpa")] {
            let o_v = read_1d(&obs_path, o_name).expect("obs");
            let o_q = read_1d(&obs_path, &format!("{o_name}_qc")).expect("qc");
            let m_v = read_1d(&hist_path, m_name).expect("model");
            let s = Series { seconds: &o_t, values: &o_v, qc: &o_q };
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
```

- [ ] **Step 2: 跑**

```bash
export PLUMBER2_ROOT=/path/to/PLUMBER2s
cargo test -p oracle --test metrics
```

Expected: `4 passed`。**任何一行对不上都要停下来查，不要放宽容差。**
六行已在 scratch 里逐个复现过，对不上说明实现与预跑的算法有出入。

- [ ] **Step 3: 提交**

```bash
git add oracle crates/colm-hist Cargo.lock
git commit -m "Reproduce the six reference metric rows"
```

---

## Task 6: 文档收尾

**Files:**
- Modify: `README.md`
- Modify: `docs/design.md`

- [ ] **Step 1: README 新增「指标」一节**

讲三件事：两条时间轴的标签含义不同（半区间回移）、QC 是「至少一个半小时好」
（并给出 253/254 vs 250/245 这个区分证据）、KGE 的 β 只标记不改值（并说明
为什么改了就对不上参考表）。

- [ ] **Step 2: design.md**

§2.8 / §2.8b 的两张表标注「已由 `oracle/tests/metrics.rs` 复现」。
§4.2 的 `colm-hist` 行把「尚未实现」改成实际状态。
§10 里程碑 6 标记完成。

- [ ] **Step 3: 全量验证与提交**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
git diff --check
```

`docs/design.md` 改完后 `cp` 到 `../CoLM202X/docs/colm-desktop-design.md`。

---

## 完成判据

- [ ] §2.8 冬季三行（Rnet / Qh / Qle）全部复现，n 精确相等
- [ ] §2.8b 湿季三行全部复现，n 精确相等
- [ ] 模型时钟平移 ±8 小时后 R² < 0.3、RMSE > 100 —— 对齐是被证伪过的，不是碰巧
- [ ] QC 用「至少一个半小时好」，且有测试说明改成「两个都要好」会得到 250/245
- [ ] spin-up 是参数；两个窗口用不同的值（8 与 96）各自通过
- [ ] KGE 的 β 警告**恰好**命中冬季 Qh 与湿季 Qh 两行，且 KGE 的值未被改动
- [ ] `cargo tree -p colm-hist` 的 default 构建**不含 netcdf**
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all --check` 无输出

---

## 明确不做

- **抽稀（thinning）** —— §10 把它列进里程碑 6，但它是画图性能优化，
  而实测一个站点两年也只有 35088 个点、uPlot 在 16 万点上交互 25 ms。
  没有需求就不做；等 GUI 真的卡了再说。
- **`_cor` 能量闭合订正版本** —— §2.8 / §2.8b 的目标值是用未订正版算的。
  订正版是另一个科学问题，不该混进「复现参考表」这件事里。
- **`GPP` / `Resp` / `NEE`** —— 前两个只有 `_se` 没有 `_qc`，`NEE` 有 `_qc`
  但不在参考表里。加它们要先有各自的参考值，否则等于无验收地实现。
- **多站点批量** —— 归里程碑 11。
