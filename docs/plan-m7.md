# 里程碑 7 实施计划：`colm-cli` —— 一条命令从 PLUMBER2 文件到指标表

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把已经各自成型的五个环节接成一条命令，验收标准是**生成的算例复现黄金 history 逐位相同**。

**Architecture:** 新增 `colm-case`（生成算例文件）与 `colm-cli`（唯一的编排可执行文件，`design.md` §4.2：「GUI 只跟它说话」）。生成 namelist 的判据是**逐算例地与 `colm-schema` 的声明默认值相比，不同才写** —— 不是照一张固定清单。

**Tech Stack:** Rust 1.85.1，复用已有的五个 crate，不引入新依赖。

> **预跑纪律**：本计划的代码块抽出来跑过 `cargo test` 与 `cargo fmt --check`，
> 但**第一版漏跑了 `cargo clippy -- -D warnings`** —— 于是 `required` 上一处
> 多余的显式生命周期（`clippy::needless_lifetimes`）一路留到执行时才被门禁拦下。
> 已按 clippy 的建议消除并同步回本文。往后预跑三条都要跑。

---

## 这条链路已经跑通过一次

`oracle/src/bin/golden_run.rs`（293 行）就是端到端的原型。里程碑 7 不是从零写，
是把其中通用的部分提出来，并补上唯一缺的一环。

| 环节 | 归属 | 状态 |
|---|---|---|
| 补齐站点文件 | `colm-srfdata` | ✅ CI 里已有「可复现」检查 |
| 生成强迫场 namelist | `colm-forcing` | ✅ |
| **生成 case.nml** | **`colm-case`** | **缺，本轮做** |
| 跑三段并判成败 | `colm-kernel` | ✅ |
| 算指标 | `colm-hist` | ✅ 里程碑 6 |

留在 `golden_run.rs` 里不搬的：输入 sha256 校验、内核溯源比对、`--write-golden`。
那三件是黄金回归专有的，不属于用户的命令行。

---

## 已实测的事实基础

### 一个算例只需要 21 个字段，而且这一点是被逐位验证过的

`oracle/cases/CN-Cng/case.nml` 设 43 个字段，其中 **22 个的值与 CoLM 声明的默认值
完全相同**。把那 22 行删掉重跑，产出的 history 与黄金文件
**`identical: 129 variables`** —— 冗余字段确实冗余，CoLM 用的就是它声明的默认值。

所以 `colm-case` 只需要写约 21 行。

### 但「哪 21 个」是逐算例算出来的，不是固定清单

`DEF_simulation_time%timestep` 的默认值是 `1800.`。实测 90 个 PLUMBER2 强迫场：
**88 个是 1800 秒，2 个是 3600 秒（`US-Ne3` 与 `US-MMS`）**。在 CN-Cng 上它是
冗余的，在那两个站点上**必须写**，否则模型会按 1800 秒推进而强迫场是 3600 秒。

所以判据是「这份输入的正确值 ≠ 声明默认值」，逐算例算。这也正是里程碑 5b
必须先做的原因：**schema 不真，这个 diff 就不可信** —— 5b 之前它认不得
`SITE_*` 整段，那 6 个字段会被当成「不认识」而无条件写出。

### Real 必须按数值比，不能按文本比

已预跑验证：`1800.` 与 `1800.0` 与 `1.8e3` 在 Fortran 里等价。按文本比会把
`1800.0` 判成偏离默认而多写一行 —— 不致命，但每个生成的算例都会带上一堆
本可省略的行，diff 里全是噪声。`colm-namelist` 的 `Value::Real` 保存原文，
所以比较时要各自 parse 成 `f64`。

预跑结果（`crates/colm-case` 的核心判据）：

```
必须写 21 个 / 可省 22 个 / schema 不认识 0 个
  timestep = 1800.    -> 可省
  timestep = 1800.0   -> 可省
  timestep = 3600.    -> 必须写
```

### 站点身份三项直接读自站点文件

实测 `CN-Cng_2008-2009_FLUXNET2015_site.nc`：

```
longitude = 123.5092 ;   latitude = 44.5933 ;   IGBP_classification = 10 ;
```

与手写 case.nml 里的 `SITE_lon_location = 123.50920` / `SITE_lat_location = 44.59330`
/ `SITE_landtype = 10` **逐位吻合**。所以这三项不该问用户。

`colm-srfdata::site::Report` 现在不暴露它们（它只管土壤与地表字段），
本轮在 `colm-srfdata` 里加一个读取函数 —— 「读站点文件」这件事和 netcdf
依赖都已经归它。

### 21 个必写字段的来源分组

| 组 | 个数 | 来源 |
|---|---|---|
| 站点身份 | 6 | `SITE_lon/lat/landtype` 读自站点文件；`SITE_fsitedata` 是补齐后的那个文件；`USE_SITE_landtype=.true.`；`DEF_CASE_NAME` 由用户或站点名推出 |
| 时间窗口 | 8 | 起止年月日秒 —— `colm-forcing::summarize` 已经算得出强迫场覆盖的范围 |
| 路径 | 4 | `DEF_dir_rawdata` / `DEF_dir_runtime` / `DEF_dir_output` / `DEF_forcing_namelist`，全由算例目录布局决定 |
| 预设级 | 3 | `DEF_USE_OZONEDATA=.false.`（本项目唯一必须显式关的默认，见 §2.7）、`DEF_WRST_FREQ`、`DEF_HIST_FREQ` |

`DEF_simulation_time%greenwich=.false.` 属于站点身份那一组：PLUMBER2 的时间轴
是地方时，而 SinglePoint 是唯一允许非格林尼治时的配置
（`MOD_TimeManager.F90:74-79` 的强制覆盖在 `#ifndef SinglePoint` 内）。

### 运行代价

CN-Cng 冬季窗口（528 个模型步、11 天）三段合计 **4.71 秒**。
一次完整两年运行约 35088 步，外推约 5 分钟。所以命令行需要进度输出，
但不需要后台化 —— 那是 GUI（里程碑 8）的事。

---

## 文件结构

```
crates/colm-case/               【新】
├── Cargo.toml                  依赖 colm-namelist + colm-schema + colm-forcing
├── src/lib.rs
├── src/minimal.rs              与声明默认值比对，决定哪些字段要写
├── src/minimal_tests.rs
├── src/build.rs                从站点 + 窗口造出算例的字段集合
├── src/build_tests.rs
└── src/layout.rs               算例目录布局（case.nml / forcing.nml / out/）

crates/colm-srfdata/src/
└── site.rs                     新增 `location()`：读经纬度与 IGBP 类别

crates/colm-cli/                【新】
├── Cargo.toml
└── src/main.rs                 子命令分发

oracle/tests/
└── generated_case.rs           【新】生成的算例必须复现黄金 history
```

**`colm-case` 不依赖 `colm-kernel`**：造文件与跑模型是两件事，混在一个 crate 里
会让「只想看看会生成什么」也得先有一个能跑的内核。`colm-cli` 负责把它们串起来。

---

## Task 1: `colm-case` 的核心判据 —— 与默认值比对

**Files:**
- Create: `crates/colm-case/Cargo.toml`
- Create: `crates/colm-case/src/lib.rs`
- Create: `crates/colm-case/src/minimal.rs`
- Create: `crates/colm-case/src/minimal_tests.rs`
- Modify: `Cargo.toml`（workspace members）

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "colm-case"
version.workspace = true
edition.workspace = true
rust-version.workspace = true   # 必须显式 opt-in
license.workspace = true
publish.workspace = true

[dependencies]
# 造文件与跑模型是两件事，所以这里**不依赖 colm-kernel** ——
# 「只想看看会生成什么」不该先要一个能跑的内核。
colm-namelist = { path = "../colm-namelist" }
colm-schema = { path = "../colm-schema" }
anyhow.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: 写 `minimal.rs`**

```rust
//! 决定一个字段要不要写进生成的 namelist。
//!
//! 判据是**这份输入的正确值是否等于 CoLM 声明的默认值** —— 相同就不写，
//! CoLM 会用同一个值。实测：`oracle/cases/CN-Cng/case.nml` 的 43 个字段里
//! 22 个与默认值相同，把它们删掉重跑，history 与黄金文件
//! `identical: 129 variables`。
//!
//! **「哪些字段冗余」是逐算例算出来的，不是一张固定清单。**
//! `DEF_simulation_time%timestep` 默认 `1800.`，实测 90 个 PLUMBER2 强迫场里
//! 88 个是 1800 秒、2 个是 3600 秒（`US-Ne3` 与 `US-MMS`）。在多数站点上它冗余，
//! 在那两个上必须写 —— 漏了的话模型按 1800 秒推进而强迫场是 3600 秒。

use colm_namelist::Value;
use colm_schema::{find, Default as D};

/// 这个取值是否与 CoLM 声明的默认值相同。
///
/// `None` 表示 `colm-schema` 不认识这个字段名 —— 那种情况必须写出去并让
/// CoLM 自己去拒绝，静默丢弃一个我们不认识的字段是最坏的处置。
pub fn is_default(path: &str, v: &Value) -> Option<bool> {
    let f = find(path)?;
    Some(match (&f.default, v) {
        (D::Logical(a), Value::Bool(b)) => a == b,
        (D::Integer(a), Value::Int(b)) => a == b,
        // Real 必须**按数值比**：1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，
        // 按文本比会把 1800.0 判成偏离，于是每个生成的算例都多带一堆
        // 本可省略的行，diff 里全是噪声。
        (D::Real(a), Value::Real { text }) => match (as_f64(a), as_f64(text)) {
            (Some(x), Some(y)) => x == y,
            _ => a.trim() == text.trim(),
        },
        (D::Str(a), Value::Str(b)) => a == b,
        // 类型对不上就当作「不同」，让它写出去 —— 这是我们理解错了字段类型，
        // 而 CoLM 报一个类型错远好过静默省略。
        _ => false,
    })
}

/// 从一组字段里筛出**必须写**的那些，保持传入顺序。
pub fn required(fields: &[(String, Value)]) -> Vec<&(String, Value)> {
    fields
        .iter()
        .filter(|(p, v)| is_default(p, v) != Some(true))
        .collect()
}

/// Fortran 的实数字面量 -> f64。`_r8` 后缀与 `d` 指数都要处理。
fn as_f64(s: &str) -> Option<f64> {
    s.trim()
        .trim_end_matches("_r8")
        .replace(['d', 'D'], "e")
        .parse()
        .ok()
}
```

- [ ] **Step 3: 写测试**

```rust
use super::*;

#[test]
fn a_value_equal_to_the_declared_default_is_omitted() {
    // DEF_Runoff_SCHEME 默认 3（Simple VIC）。算例照抄 3 就不必写。
    assert_eq!(is_default("DEF_Runoff_SCHEME", &Value::Int(3)), Some(true));
    assert_eq!(is_default("DEF_Runoff_SCHEME", &Value::Int(0)), Some(false));
}

#[test]
fn reals_compare_numerically_not_textually() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里是同一个数。按文本比的话
    // 生成的每个算例都会多带一行 timestep，纯噪声。
    for t in ["1800.", "1800.0", "1.8e3", " 1800. "] {
        assert_eq!(
            is_default(
                "DEF_simulation_time%timestep",
                &Value::Real { text: t.into() }
            ),
            Some(true),
            "{t} should equal the declared default 1800."
        );
    }
}

#[test]
fn the_two_hourly_sites_must_write_their_timestep() {
    // 实测 90 个强迫场里 US-Ne3 与 US-MMS 是 3600 秒。漏写的话模型按
    // 1800 秒推进而强迫场是 3600 秒 —— 跑得完，结果全错。
    assert_eq!(
        is_default(
            "DEF_simulation_time%timestep",
            &Value::Real {
                text: "3600.".into()
            }
        ),
        Some(false)
    );
}

#[test]
fn an_unknown_field_is_written_out_rather_than_dropped() {
    // schema 不认识它，可能是上游新加的，也可能是用户拼错了。
    // 两种情况都该让 CoLM 自己去表态 —— 它对未声明的变量会明确报
    // `Cannot match namelist object name` 然后停，那是有用的报错。
    // 静默丢弃则会让用户以为自己设了。
    assert_eq!(is_default("DEF_NOT_A_REAL_FIELD", &Value::Bool(true)), None);
    let f = vec![("DEF_NOT_A_REAL_FIELD".to_string(), Value::Bool(true))];
    assert_eq!(required(&f).len(), 1);
}

#[test]
fn the_single_point_block_is_understood() {
    // 里程碑 5b 之前 schema 认不得整个 SITE_ 段，这些会全部落进
    // 「不认识」而被无条件写出。这条守住那次修复没有回退。
    assert_eq!(
        is_default("USE_SITE_landtype", &Value::Bool(false)),
        Some(true)
    );
    assert_eq!(
        is_default("USE_SITE_landtype", &Value::Bool(true)),
        Some(false)
    );
    assert_eq!(is_default("SITE_landtype", &Value::Int(-1)), Some(true));
    assert_eq!(is_default("SITE_landtype", &Value::Int(10)), Some(false));
}

#[test]
fn required_keeps_the_order_it_was_given() {
    // 生成的 namelist 里字段顺序应当稳定，否则每次重生成都是一个大 diff。
    let f = vec![
        ("DEF_CASE_NAME".to_string(), Value::Str("X".into())),
        ("DEF_Runoff_SCHEME".to_string(), Value::Int(3)), // 等于默认，会被滤掉
        ("DEF_USE_OZONEDATA".to_string(), Value::Bool(false)),
    ];
    let r = required(&f);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].0, "DEF_CASE_NAME");
    assert_eq!(r[1].0, "DEF_USE_OZONEDATA");
}
```

- [ ] **Step 4: 接上并跑**

`lib.rs`：

```rust
//! 从一个站点与一个时间窗口造出 CoLM 能跑的算例文件。
//!
//! 生成的 namelist **只包含真正偏离 CoLM 默认值的字段**，而「哪些偏离」
//! 逐算例算出来 —— 见 `minimal`。实测 CN-Cng 是 21 个字段（手写版有 43 个，
//! 删掉那 22 个冗余行之后 history 逐位不变）。
//!
//! 本 crate **不依赖 `colm-kernel`**：造文件与跑模型是两件事。

pub mod minimal;

pub use minimal::{is_default, required};
```

根 `Cargo.toml` 的 `members` 加 `"crates/colm-case"`。

Run: `cargo test -p colm-case`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: 提交**

```bash
git add crates/colm-case Cargo.toml Cargo.lock
git commit -m "Write only the fields that actually differ from CoLM's defaults"
```

---

## Task 2: 从站点文件读出身份三项

**Files:**
- Modify: `crates/colm-srfdata/src/site.rs`
- Modify: `crates/colm-srfdata/src/site_tests.rs`（若无则新建）

- [ ] **Step 1: 加读取函数**

```rust
/// 站点的身份：位置与地类。
///
/// 这三项 PLUMBER2 的站点文件自带，实测 CN-Cng 给出
/// `longitude = 123.5092` / `latitude = 44.5933` / `IGBP_classification = 10`，
/// 与手写算例里的 `SITE_lon_location` / `SITE_lat_location` / `SITE_landtype`
/// **逐位吻合**。所以新建算例时不该问用户要这三个数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub lon: f64,
    pub lat: f64,
    /// IGBP 分类号，直接对应 `SITE_landtype`。
    pub landtype: i32,
}

pub fn location(file: &Path) -> Result<Location> {
    let f = netcdf::open(file)
        .with_context(|| format!("cannot open {}", file.display()))?;
    let get = |name: &str| -> Result<f64> {
        let v = f
            .variable(name)
            .with_context(|| format!("{} has no {name}", file.display()))?;
        Ok(v.get_value::<f64, _>(())?)
    };
    Ok(Location {
        lon: get("longitude")?,
        lat: get("latitude")?,
        landtype: get("IGBP_classification")? as i32,
    })
}
```

- [ ] **Step 2: 测试（需要 PLUMBER2，放 `tests/`）**

追加到 `crates/colm-srfdata/tests/real_sites.rs`：

```rust
#[test]
fn every_site_file_carries_its_own_location_and_landtype() {
    // 新建算例时这三项不该问用户。实测 90 个站点文件全都自带。
    let Some(root) = plumber2() else { return };
    let dir = root.join("Sitedata");
    let mut n = 0;
    let mut classes = std::collections::BTreeSet::new();
    for e in std::fs::read_dir(&dir).expect("Sitedata") {
        let p = e.expect("entry").path();
        if p.extension().and_then(|x| x.to_str()) != Some("nc") {
            continue;
        }
        let l = colm_srfdata::site::location(&p).expect("location");
        assert!((-180.0..=180.0).contains(&l.lon), "{p:?} lon {}", l.lon);
        assert!((-90.0..=90.0).contains(&l.lat), "{p:?} lat {}", l.lat);
        assert!((1..=17).contains(&l.landtype), "{p:?} IGBP {}", l.landtype);
        classes.insert(l.landtype);
        n += 1;
    }
    assert_eq!(n, 90);
    // 90 个站点不该全是同一种地类 —— 那说明读错了字段。
    assert!(classes.len() > 3, "only {} distinct IGBP classes", classes.len());
}

#[test]
fn the_golden_site_matches_the_hand_written_case() {
    // 手写的 oracle/cases/CN-Cng/case.nml 里那三个数就是从这里来的。
    let Some(root) = plumber2() else { return };
    let p = root.join("Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc");
    let l = colm_srfdata::site::location(&p).expect("location");
    assert!((l.lon - 123.5092).abs() < 1e-4, "{}", l.lon);
    assert!((l.lat - 44.5933).abs() < 1e-4, "{}", l.lat);
    assert_eq!(l.landtype, 10);
}
```

- [ ] **Step 3: 跑并提交**

```bash
export PLUMBER2_ROOT=/path/to/PLUMBER2s
cargo test -p colm-srfdata --test real_sites
```

```bash
git add crates/colm-srfdata
git commit -m "Read a site's own coordinates and land cover class"
```

---

## Task 3: 造出完整的算例字段集合

**Files:**
- Create: `crates/colm-case/src/build.rs`
- Create: `crates/colm-case/src/build_tests.rs`
- Modify: `crates/colm-case/src/lib.rs`

- [ ] **Step 1: 定义输入与输出**

```rust
//! 从「一个站点 + 一个时间窗口 + 一份目录布局」造出算例的字段集合。
//!
//! 输出是有序的 `(路径, 值)` 列表，交给 `minimal::required` 过滤之后再序列化。
//! 顺序固定，否则每次重生成都是一个大 diff。

use colm_namelist::Value;

/// 造一个算例需要知道的全部东西。
///
/// 21 个必写字段里，只有 `name` 与 `window` 真正需要人来定；其余要么读自
/// 站点文件（位置与地类），要么由强迫场算出（可用的时间范围），
/// 要么由目录布局决定（四个路径），要么属于预设。
pub struct CaseSpec {
    pub name: String,
    /// 补齐之后的站点文件路径，写进 `SITE_fsitedata`
    pub site_file: String,
    pub lon: f64,
    pub lat: f64,
    pub landtype: i32,
    pub window: Window,
    /// 强迫场文件的时间步长（秒）。实测 88/90 个站点是 1800，
    /// `US-Ne3` 与 `US-MMS` 是 3600 —— 它必须跟着走。
    pub timestep_seconds: f64,
    pub dirs: Dirs,
}

#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub start_year: i32,
    pub start_month: u32,
    pub start_day: u32,
    pub end_year: i32,
    pub end_month: u32,
    pub end_day: u32,
}

pub struct Dirs {
    pub rawdata: String,
    pub runtime: String,
    pub output: String,
    pub forcing_namelist: String,
}

/// 造出字段集合。**不做过滤** —— 过滤是 `minimal::required` 的事，
/// 分开是为了让「本来会写什么」与「实际写了什么」都能被看到。
pub fn fields(s: &CaseSpec) -> Vec<(String, Value)> {
    let r = |x: f64| Value::Real {
        text: format!("{x:?}"),
    };
    vec![
        ("DEF_CASE_NAME".into(), Value::Str(s.name.clone())),
        // ---- 站点身份 ----
        ("SITE_fsitedata".into(), Value::Str(s.site_file.clone())),
        ("SITE_lon_location".into(), r(s.lon)),
        ("SITE_lat_location".into(), r(s.lat)),
        ("SITE_landtype".into(), Value::Int(s.landtype as i64)),
        ("USE_SITE_landtype".into(), Value::Bool(true)),
        // PLUMBER2 的时间轴是地方时。SinglePoint 是唯一允许非格林尼治时的
        // 配置（MOD_TimeManager.F90:74-79 的强制覆盖在 #ifndef SinglePoint 内）。
        ("DEF_simulation_time%greenwich".into(), Value::Bool(false)),
        // ---- 时间窗口 ----
        (
            "DEF_simulation_time%start_year".into(),
            Value::Int(s.window.start_year as i64),
        ),
        (
            "DEF_simulation_time%start_month".into(),
            Value::Int(s.window.start_month as i64),
        ),
        (
            "DEF_simulation_time%start_day".into(),
            Value::Int(s.window.start_day as i64),
        ),
        ("DEF_simulation_time%start_sec".into(), Value::Int(0)),
        (
            "DEF_simulation_time%end_year".into(),
            Value::Int(s.window.end_year as i64),
        ),
        (
            "DEF_simulation_time%end_month".into(),
            Value::Int(s.window.end_month as i64),
        ),
        (
            "DEF_simulation_time%end_day".into(),
            Value::Int(s.window.end_day as i64),
        ),
        ("DEF_simulation_time%end_sec".into(), Value::Int(86400)),
        ("DEF_simulation_time%timestep".into(), r(s.timestep_seconds)),
        // spin-up 关掉：这三项的默认值不是「不做 spin-up」，必须显式写。
        ("DEF_simulation_time%spinup_day".into(), Value::Int(365)),
        ("DEF_simulation_time%spinup_sec".into(), Value::Int(86400)),
        ("DEF_simulation_time%spinup_repeat".into(), Value::Int(0)),
        // ---- 路径 ----
        ("DEF_dir_rawdata".into(), Value::Str(s.dirs.rawdata.clone())),
        ("DEF_dir_runtime".into(), Value::Str(s.dirs.runtime.clone())),
        ("DEF_dir_output".into(), Value::Str(s.dirs.output.clone())),
        (
            "DEF_forcing_namelist".into(),
            Value::Str(s.dirs.forcing_namelist.clone()),
        ),
        // ---- 预设级 ----
        // 臭氧是本项目唯一必须显式关掉的默认开关：CoLM 默认 .true.，
        // 要读 2.8 GB 的 Ozone/Global/OZONE-setgrid.nc。关掉之后
        // MOD_Ozone.F90:83 用常数 100 ppbv，臭氧胁迫仍生效。见 design.md §2.7。
        ("DEF_USE_OZONEDATA".into(), Value::Bool(false)),
        ("DEF_WRST_FREQ".into(), Value::Str("MONTHLY".into())),
        ("DEF_HIST_FREQ".into(), Value::Str("HOURLY".into())),
    ]
}
```

- [ ] **Step 2: 测试**

```rust
use super::*;

fn cn_cng() -> CaseSpec {
    CaseSpec {
        name: "CN-Cng".into(),
        site_file: "/w/site.nc".into(),
        lon: 123.5092,
        lat: 44.5933,
        landtype: 10,
        window: Window {
            start_year: 2008,
            start_month: 1,
            start_day: 1,
            end_year: 2008,
            end_month: 1,
            end_day: 11,
        },
        timestep_seconds: 1800.0,
        dirs: Dirs {
            rawdata: "/w/rawdata_unused/".into(),
            runtime: "/w/runtime_unused/".into(),
            output: "/w/out/".into(),
            forcing_namelist: "/w/forcing.nml".into(),
        },
    }
}

#[test]
fn the_golden_case_needs_twenty_one_fields() {
    // 实测：手写的 oracle/cases/CN-Cng/case.nml 设 43 个字段，其中 22 个
    // 等于 CoLM 的声明默认值。删掉那 22 行重跑，history 与黄金文件
    // identical: 129 variables。
    let all = fields(&cn_cng());
    let req = crate::minimal::required(&all);
    assert_eq!(
        req.len(),
        21,
        "{:#?}",
        req.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
}

#[test]
fn a_half_hourly_site_omits_the_timestep_and_an_hourly_one_writes_it() {
    // 88/90 个站点是 1800 秒（等于默认，省略）；US-Ne3 与 US-MMS 是 3600，
    // 必须写出去。这条守住那两个站点不会被静默按 1800 秒跑。
    let has = |s: &CaseSpec| {
        crate::minimal::required(&fields(s))
            .iter()
            .any(|(p, _)| p == "DEF_simulation_time%timestep")
    };
    assert!(!has(&cn_cng()));
    let mut hourly = cn_cng();
    hourly.timestep_seconds = 3600.0;
    assert!(has(&hourly));
}

#[test]
fn a_real_renders_with_its_decimal_point() {
    // `{}` 会把 1800.0 印成 "1800"，而那在 namelist 里是**整数**，
    // 赋给 real 字段会让 CoLM 报类型错。里程碑 4 在 HEIGHT_* 上栽过一次。
    let all = fields(&cn_cng());
    let ts = all
        .iter()
        .find(|(p, _)| p == "DEF_simulation_time%timestep")
        .unwrap();
    assert_eq!(ts.1.to_string(), "1800.0");
    let lon = all.iter().find(|(p, _)| p == "SITE_lon_location").unwrap();
    assert!(lon.1.to_string().contains('.'), "{}", lon.1);
}

#[test]
fn every_generated_field_is_one_the_schema_knows() {
    // 生成一个 schema 不认识的字段名，说明我们拼错了 —— CoLM 会在
    // `Cannot match namelist object name` 上停，但那要等到跑起来才发现。
    for (p, _) in fields(&cn_cng()) {
        assert!(colm_schema::find(&p).is_some(), "schema does not know {p}");
    }
}

#[test]
fn every_generated_field_is_settable_from_the_main_namelist() {
    // 里程碑 5b 给每个字段记了它属于哪个 namelist 组。写进 case.nml 的
    // 必须全是 nl_colm 组的 —— 强迫场字段归 forcing.nml，输出变量开关
    // 归 history namelist，写错地方 CoLM 不会认。
    for (p, _) in fields(&cn_cng()) {
        let f = colm_schema::find(&p).unwrap();
        assert_eq!(f.group, Some("nl_colm"), "{p} belongs to {:?}", f.group);
    }
}
```

- [ ] **Step 3: 跑并提交**

Run: `cargo test -p colm-case`
Expected: `test result: ok. 11 passed`（6 + 新 5）

```bash
git add crates/colm-case/src
git commit -m "Derive a case from the site file instead of asking for its coordinates"
```

---

## Task 4: 序列化与目录布局

**Files:**
- Create: `crates/colm-case/src/layout.rs`
- Modify: `crates/colm-case/src/lib.rs`

- [ ] **Step 1: 写出 case.nml**

序列化用 `colm-namelist` 的 `Display`，包一层 `&nl_colm` / `/`。
每一组前面加一行注释说明它从哪来 —— 生成的文件是给人看的，
而「这个值哪来的」是用户第一个会问的问题。

```rust
//! 算例目录布局与序列化。
//!
//!
```text
//! <case>/
//! ├── case.nml       生成，只含偏离默认的字段
//! ├── forcing.nml    生成（colm-forcing）
//! ├── site.nc        补齐后的站点文件（colm-srfdata）
//! └── out/           模型产物
//! ```

use std::path::{Path, PathBuf};

pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> Layout {
        Layout { root: root.to_path_buf() }
    }
    pub fn case_nml(&self) -> PathBuf { self.root.join("case.nml") }
    pub fn forcing_nml(&self) -> PathBuf { self.root.join("forcing.nml") }
    pub fn site_nc(&self) -> PathBuf { self.root.join("site.nc") }
    pub fn out(&self) -> PathBuf { self.root.join("out") }
}

/// 把字段集合渲染成 `nl_colm` 组。
pub fn render(fields: &[&(String, colm_namelist::Value)]) -> String {
    let mut s = String::from("&nl_colm\n\n");
    for (p, v) in fields {
        s.push_str(&format!("   {p} = {v}\n"));
    }
    s.push_str("/\n");
    s
}
```

- [ ] **Step 2: 往返测试**

生成的 case.nml 必须能被 `colm-namelist` 解析回来，且每个字段读回的值与写出的相同。
这条挡住「渲染得好看但 CoLM 读不了」。

- [ ] **Step 3: 提交**

---

## Task 5: 生成的算例复现黄金 history

**这一步是本计划的验收核心。**

**Files:**
- Create: `oracle/tests/generated_case.rs`
- Modify: `oracle/Cargo.toml`（加 `colm-case` 依赖）

- [ ] **Step 1: 写测试**

用 `colm-case` 生成 CN-Cng 冬季窗口的算例，跑三段，与
`oracle/golden/CN-Cng_hist_2008-01.nc` 比对，必须
`identical: 129 variables`。

**这条比「生成的文件长得对」强得多**：它证明生成的配置在语义上与手写的
那份等价，而不是看起来等价。

- [ ] **Step 2: 跑**

```bash
export PLUMBER2_ROOT=/path/to/PLUMBER2s
cargo test -p oracle --test generated_case
```

**任何差异都要停下来查。** 已知的等价性已经验证过一次：手写算例删掉
22 个冗余字段之后 history 逐位不变，所以生成版对不上只可能是生成错了。

---

## Task 6: `colm-cli`

**Files:**
- Create: `crates/colm-cli/Cargo.toml`
- Create: `crates/colm-cli/src/main.rs`

子命令：

```
colm-cli new    --site <raw site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]
colm-cli run    <case-dir> --kernel <dir>
colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N]
colm-cli all    --site ... --kernel ... --obs ...      # §10 要的「一条命令」
```

`new` 依次做：`site-fill` → `colm-forcing::render` → `colm-case`。
`run` 调 `colm_kernel::run_stage` 三次。
`metrics` 调 `colm-hist`。
`all` 把三者串起来。

**不做后台化与取消** —— 那是 GUI（里程碑 8，`design.md` §6.6）的事。
命令行只需要把进度打出来；实测 CN-Cng 冬季窗口 4.71 秒，
完整两年约 5 分钟。

---

## Task 7: 文档收尾

README 新增「端到端」一节，`design.md` §10 里程碑 7 标记完成。

---

## 完成判据

- [ ] `colm-case` 生成的算例跑出的 history 与黄金文件 `identical: 129 variables`
- [ ] CN-Cng 的必写字段是 21 个（手写版 43 个里 22 个冗余）
- [ ] `timestep` 在 1800 秒站点上被省略、在 3600 秒站点上被写出
- [ ] 生成的每个字段 `colm-schema` 都认识，且 group 都是 `nl_colm`
- [ ] Real 值渲染带小数点（`1800.0` 而不是 `1800`）
- [ ] schema 不认识的字段被**写出去**而不是静默丢弃
- [ ] `colm-cli all` 一条命令从原始 PLUMBER2 站点文件产出指标表
- [ ] clippy `-D warnings` 与 fmt 无输出

---

## 明确不做

- **批量与敏感性矩阵** —— `design.md` §10 归里程碑 11。
- **`run_manifest.json`** —— 它服务「可复现打包导出」，等有第二个消费者再做。
- **后台化、进度事件、取消** —— 里程碑 8 的 GUI 才需要（§6.6）。
- **多预设** —— 里程碑 10。本轮只跑 `waterheat`。
