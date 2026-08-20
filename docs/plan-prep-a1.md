# 前处理阶段 A1：强迫场转换管道（实施计划）

> **给执行者：** 用 `superpowers:subagent-driven-development`（推荐）或
> `superpowers:executing-plans` 按任务逐条实施。步骤用 `- [ ]` 复选框标记。

**目标：** 把用户自己的 netCDF 强迫场（变量名/单位与 PLUMBER2 不同）转成
一份符合 PLUMBER2 约定的文件，从此走与内置数据集同一条路。

**架构：** 新增 `colm-forcing::convert`（转换管道）与 `::units`（单位换算）。
`slots::resolve` 扩展成接受用户指定的映射覆盖。**PLUMBER2 与 Urban-PLUMBER
继续直读，不进这条管道** —— 黄金基准不动。

**技术栈：** Rust，`netcdf`（static，已在依赖里）、`anyhow`。**不引入新依赖。**

**范围：** 只做后端与命令行。界面（前处理页的强迫场子栏）是 A2，另一份计划。

---

## 0. 先读这一节

**这条管道的判据是「转出来的与直读逐位相同」，不是「跑通了」。**

设计文档（`docs/design-prep.md` §1.1）把这条列为核心：

> 拿 CN-Cng 的原始 Met 文件走一遍转换管道，转出来的文件跑出的 history
> 应当与直读逐位相同。

**Task 1 先把这个对照搭起来，再写任何转换逻辑。** 用一个「恒等转换」
（读进来原样写出去）作为第一个被验证的对象 —— 它必须通过对照，
否则说明对照框架本身有问题，而不是转换逻辑有问题。

前车之鉴：上一轮预抽土壤点值时，`serde_json` 默认浮点解析差 1 ULP
（`1.8337343205163141` → `1.833734320516314`），三段照样跑通、曲线照样
好看，是逐位比对把它抓出来的。**跑通不等于对。**

### 现有代码的事实（直接用，不要重新摸索）

| 事实 | 位置 |
|---|---|
| 八个槽位 `1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW` | `slots.rs` 的 `SLOTS` |
| `resolve(&[String]) -> (Resolved, Vec<String>)`，按候选名匹配，只有第 5 槽 `optional` | `slots.rs:136` |
| 7 个必需变量、时间轴、覆盖窗口的校验 | `check.rs` |
| 读元数据（时间单位、步长、高度、`time_shown_in`） | `met.rs::summarize` |
| 时区判据：**只有文件明说 `UTC` 才是格林尼治时** | `check.rs::is_greenwich` |
| 依赖：`anyhow`、`colm-namelist`、`netcdf`（static） | `Cargo.toml` |

**环境**：PLUMBER2 在 `~/Desktop/colm-rust/PLUMBER2s`，CN-Cng 的三个文件
sha256 与 `oracle/fixtures/inputs.sha256` 对得上。内核在 `kernels/default`
与 `kernels/urban`。基线：`cargo test --workspace` 299 passed。

---

## Task 1: 先立对照 —— 恒等转换

**Files:**
- Create: `crates/colm-forcing/src/convert.rs`
- Create: `crates/colm-forcing/src/convert_tests.rs`
- Modify: `crates/colm-forcing/src/lib.rs`

- [ ] **Step 1: 写失败的测试**

`crates/colm-forcing/src/convert_tests.rs`：

```rust
//! 转换管道的测试。
//!
//! **这一组测试的地位与别处不同**：转换器改的是数值，而数值错了
//! 模型照样跑得完 —— 所以每一条都要断言具体的数，不能只断言「没报错」。

use std::path::PathBuf;

/// 造一个最小的强迫场文件：一个变量、三个时刻。
///
/// 不用真实的 PLUMBER2 文件（15 MB，且测试要能离线跑），但**维度与属性
/// 的形状照抄它** —— 转换器读的正是这些。
fn tiny_met(dir: &std::path::Path, var: &str, values: &[f64]) -> PathBuf {
    let p = dir.join("tiny_Met.nc");
    let mut f = netcdf::create(&p).expect("create");
    f.add_dimension("time", values.len()).expect("dim");
    let mut t = f.add_variable::<f64>("time", &["time"]).expect("time var");
    t.put_attribute("units", "seconds since 2008-01-01 00:00:00").expect("units");
    let secs: Vec<f64> = (0..values.len()).map(|i| (i as f64) * 1800.0).collect();
    t.put_values(&secs, netcdf::Extents::All).expect("put time");
    let mut v = f.add_variable::<f64>(var, &["time"]).expect("var");
    v.put_values(values, netcdf::Extents::All).expect("put values");
    p
}

#[test]
fn an_identity_conversion_reproduces_every_value_bit_for_bit() {
    let dir = std::env::temp_dir().join("colm-convert-identity");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 刻意用一组**不能被二进制精确表示**的值 —— 若管道中途做了
    // 十进制往返（比如经过字符串），这里会差最后一位。
    let vals = [1.8337343205163141, 273.15, 0.1 + 0.2];
    let src = tiny_met(&dir, "Tair", &vals);
    let dst = dir.join("out_Met.nc");

    super::identity(&src, &dst).expect("identity conversion");

    let f = netcdf::open(&dst).unwrap();
    let got: Vec<f64> = f.variable("Tair").unwrap()
        .get_values(netcdf::Extents::All).unwrap();
    assert_eq!(got, vals, "恒等转换必须逐位复现，差一个 ULP 都算失败");
}
```

- [ ] **Step 2: 跑，确认它因为 `identity` 不存在而失败**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo test -p colm-forcing --lib identity 2>&1 | tail -5
```

期望：编译失败，`cannot find function 'identity'`。

- [ ] **Step 3: 写最小实现**

`crates/colm-forcing/src/convert.rs`：

```rust
//! 把一份强迫场文件转成 CoLM 认的约定。
//!
//! **只转认不出来的数据。** PLUMBER2 与 Urban-PLUMBER 继续直读
//! （`lib.rs` 开头那段说明），转它们只会多出一份 50 MB 拷贝，
//! 还会让黄金基准失去意义 —— 那是目前唯一能证明「改动没弄坏结果」的判据。
//!
//! **产物与源文件分开存放，原始数据永不改动**（前处理页立的约束）。

use std::path::Path;

use anyhow::{Context, Result};

/// 原样复制一份强迫场文件。
///
/// **这是转换管道的地基，也是它的第一条判据。** 恒等转换必须逐位复现 ——
/// 若这一步就丢精度，后面所有换算的正确性都无从谈起。
///
/// 实现上是「读出来再写进去」而不是 `std::fs::copy`：`fs::copy` 复现的是
/// 字节，证明不了「我们的读写路径不丢精度」，而后者才是要验的东西。
pub fn identity(src: &Path, dst: &Path) -> Result<()> {
    let fin = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let mut fout =
        netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;

    for d in fin.dimensions() {
        fout.add_dimension(&d.name(), d.len())
            .with_context(|| format!("cannot add dimension {}", d.name()))?;
    }

    for v in fin.variables() {
        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let values: Vec<f64> = v
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read {}", v.name()))?;
        let mut out = fout
            .add_variable::<f64>(&v.name(), &dim_refs)
            .with_context(|| format!("cannot add variable {}", v.name()))?;
        for a in v.attributes() {
            if let Ok(netcdf::AttributeValue::Str(s)) = a.value() {
                out.put_attribute(&a.name(), s.as_str())?;
            }
        }
        out.put_values(&values, netcdf::Extents::All)
            .with_context(|| format!("cannot write {}", v.name()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod convert_tests;
```

`crates/colm-forcing/src/lib.rs` 的模块列表里加：

```rust
pub mod convert;
```

- [ ] **Step 4: 跑，确认通过**

```bash
cargo test -p colm-forcing --lib identity 2>&1 | tail -5
```

期望：`test result: ok. 1 passed`。

**若那个 `0.1 + 0.2` 的断言失败**，说明读写路径丢了精度 —— 那是这条管道的
根本问题，停下来查清楚再往下，不要调松断言。

- [ ] **Step 5: 提交**

```bash
git status --short          # 这个仓库有多个会话并发工作，先看暂存区
git add crates/colm-forcing/src/convert.rs \
        crates/colm-forcing/src/convert_tests.rs \
        crates/colm-forcing/src/lib.rs
git commit -m "强迫场转换管道的地基：恒等转换

转换器改的是数值，而数值错了模型照样跑得完 —— 所以先立判据再写逻辑。
恒等转换是第一个被验证的对象：它必须逐位复现，否则说明读写路径本身
丢精度，后面所有换算的正确性都无从谈起。

用读出来再写进去而不是 fs::copy —— 后者复现的是字节，证明不了我们的
读写路径不丢精度，而那才是要验的东西。

Constraint: 断言用不能被二进制精确表示的值，差一个 ULP 就该红
Confidence: high
Scope-risk: narrow
Tested: cargo test -p colm-forcing --lib identity"
```

---

## Task 2: 单位换算

**Files:**
- Create: `crates/colm-forcing/src/units.rs`
- Create: `crates/colm-forcing/src/units_tests.rs`
- Modify: `crates/colm-forcing/src/lib.rs`

- [ ] **Step 1: 写失败的测试**

`crates/colm-forcing/src/units_tests.rs`：

```rust
//! 单位换算的测试。
//!
//! **换算是这条管道里最容易出错、也最难发现的一环** —— 温度差 273.15、
//! 降水差 3600 倍，跑出来的结果都还在「看着像那么回事」的范围内。

#[test]
fn celsius_becomes_kelvin() {
    // **这里不能用 `assert_eq!` 比字面量。** `-40.0 + 273.15` 与直接写
    // 下的字面量 `233.15` 差 1 ULP —— `273.15` 的最近 f64 比真值小，
    // `233.15` 的最近 f64 比真值大，两边独立舍入的方向不一致，加法
    // 补不平这个缝。`0.0` 与 `25.0` 那两个恰好逐位相同，所以这**不是**
    // 「浮点都得用容差」，而是具体撞上了 -40 这个值。
    let v = super::convert_units("degC", "K", &[0.0, 25.0, -40.0]).unwrap();
    let want = [273.15, 298.15, 233.15];
    for (got, want) in v.iter().zip(want.iter()) {
        assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
    }
}

#[test]
fn an_already_correct_unit_is_returned_untouched() {
    // **不是「换算成自己」，是原样返回。** 乘 1.0 再加 0.0 会让
    // 非规格化的浮点值发生变化，而这条管道的地基正是逐位复现。
    let vals = [1.8337343205163141, 0.1 + 0.2];
    let v = super::convert_units("K", "K", &vals).unwrap();
    assert_eq!(v, vals.to_vec());
}

#[test]
fn hourly_accumulated_precipitation_becomes_a_rate() {
    // mm/hr → mm/s（CoLM 要的是率）
    let v = super::convert_units("mm/hr", "mm/s", &[3.6]).unwrap();
    assert!((v[0] - 0.001).abs() < 1e-12, "got {}", v[0]);
}

#[test]
fn an_unknown_pair_is_refused_rather_than_silently_passed_through() {
    // **拒绝比放行安全。** 放行一个不认识的单位，模型会拿着量纲错误的
    // 数跑完，而界面上什么都看不出来。
    let e = super::convert_units("furlongs", "K", &[1.0]).unwrap_err();
    assert!(e.to_string().contains("furlongs"), "报错要点名那个单位：{e}");
}
```

- [ ] **Step 2: 跑，确认失败**

```bash
cargo test -p colm-forcing --lib units 2>&1 | tail -5
```

期望：编译失败，`cannot find function 'convert_units'`。

- [ ] **Step 3: 写实现**

`crates/colm-forcing/src/units.rs`：

```rust
//! 单位换算。
//!
//! **认识的才换，不认识的报错。** 放行一个不认识的单位，模型会拿着量纲
//! 错误的数跑完，而界面上什么都看不出来 —— 那正是这个项目反复要避免的
//! 「跑得完却给出错误结果」。
//!
//! 换算表按 `(from, to)` 精确匹配，不做别名归一化 —— `degC` 与 `celsius`
//! 是两条独立的表项。名字的模糊匹配放在调用方（界面上让人确认），
//! 这里只做确定的算术。

use anyhow::{bail, Result};

/// `(from, to, scale, offset)`：`out = in * scale + offset`
const TABLE: &[(&str, &str, f64, f64)] = &[
    // 温度
    ("degC", "K", 1.0, 273.15),
    ("celsius", "K", 1.0, 273.15),
    ("C", "K", 1.0, 273.15),
    // 气压
    ("hPa", "Pa", 100.0, 0.0),
    ("mb", "Pa", 100.0, 0.0),
    ("kPa", "Pa", 1000.0, 0.0),
    // 降水：CoLM 要率（mm/s，等价于 kg/m2/s）
    ("mm/hr", "mm/s", 1.0 / 3600.0, 0.0),
    ("mm/h", "mm/s", 1.0 / 3600.0, 0.0),
    ("mm/day", "mm/s", 1.0 / 86400.0, 0.0),
    // 比湿：无量纲与 g/kg
    ("g/kg", "kg/kg", 0.001, 0.0),
];

/// 把 `values` 从 `from` 换算成 `to`。
///
/// `from == to` 时**原样返回**，不做 `* 1.0 + 0.0` —— 那会让
/// 非规格化的浮点值发生变化，而这条管道的地基正是逐位复现。
pub fn convert_units(from: &str, to: &str, values: &[f64]) -> Result<Vec<f64>> {
    if from == to {
        return Ok(values.to_vec());
    }
    match TABLE
        .iter()
        .find(|(f, t, _, _)| *f == from && *t == to)
    {
        Some((_, _, scale, offset)) => {
            Ok(values.iter().map(|v| v * scale + offset).collect())
        }
        None => bail!(
            "no known conversion from {from:?} to {to:?}; \
             add it to units::TABLE or fix the unit attribute in the source file"
        ),
    }
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod units_tests;
```

`lib.rs` 加 `pub mod units;`。

- [ ] **Step 4: 跑，确认四条全过**

```bash
cargo test -p colm-forcing --lib units 2>&1 | tail -6
```

期望：`test result: ok. 4 passed`。

- [ ] **Step 5: 提交**

```bash
git status --short
git add crates/colm-forcing/src/units.rs \
        crates/colm-forcing/src/units_tests.rs \
        crates/colm-forcing/src/lib.rs
git commit -m "强迫场单位换算：认识的才换

换算是这条管道里最容易出错也最难发现的一环 —— 温度差 273.15、
降水差 3600 倍，跑出来都还在「看着像那么回事」的范围内。

不认识的单位报错而不是放行：放行会让模型拿着量纲错误的数跑完，
界面上什么都看不出来。from == to 时原样返回而不是乘 1.0 加 0.0，
后者会让非规格化浮点值变化，而这条管道的地基正是逐位复现。

Confidence: high
Scope-risk: narrow
Tested: cargo test -p colm-forcing --lib units（4 条）"
```

---

## Task 3: 槽位映射接受用户指定

`resolve` 现在只按候选名自动匹配。用户的数据可能叫 `TA_F` / `air_temp`，
需要能手工指定。

**Files:**
- Modify: `crates/colm-forcing/src/slots.rs`
- Modify: `crates/colm-forcing/src/slots_tests.rs`

- [ ] **Step 1: 写失败的测试**

在 `slots_tests.rs` 末尾加：

```rust
#[test]
fn a_user_override_wins_over_the_built_in_candidates() {
    // 文件里既有 PLUMBER2 的 `Tair`，用户又指定了别的 —— 以用户为准。
    // 这不是假想：同一份文件里可能有 `Tair`（塔顶）与 `Tair_2m`（2 米），
    // 而候选名表只认前者。
    let vars: Vec<String> = ["Tair", "Tair_2m", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown"]
        .iter().map(|s| s.to_string()).collect();
    let overrides = [(1usize, "Tair_2m".to_string())];
    let (r, missing) = super::resolve_with(&vars, &overrides);
    assert!(missing.is_empty(), "不该缺槽位：{missing:?}");
    assert_eq!(r.vname[0], Some("Tair_2m"), "第 1 槽应当用用户指定的名字");
}

#[test]
fn an_override_naming_a_variable_the_file_does_not_have_is_refused() {
    // **指定一个不存在的变量必须报错**，不能悄悄回落到自动匹配 ——
    // 那样用户以为自己选了 A，实际跑的是 B。
    let vars: Vec<String> = ["Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown"]
        .iter().map(|s| s.to_string()).collect();
    let overrides = [(1usize, "does_not_exist".to_string())];
    let (_, missing) = super::resolve_with(&vars, &overrides);
    assert!(
        missing.iter().any(|m| m.contains("does_not_exist")),
        "报错要点名那个变量：{missing:?}"
    );
}
```

- [ ] **Step 2: 跑，确认失败**

```bash
cargo test -p colm-forcing --lib slots 2>&1 | tail -5
```

期望：编译失败，`cannot find function 'resolve_with'`。

- [ ] **Step 3: 写实现**

在 `slots.rs` 的 `resolve` **之后**加：

```rust
/// 与 `resolve` 相同，但允许用户为某些槽位**指定**变量名。
///
/// `overrides` 是 `(槽位序号 1-based, 变量名)`。指定的名字文件里没有时
/// **报错而不是回落到自动匹配** —— 回落会让用户以为自己选了 A、
/// 实际跑的是 B，而那是「跑得完却给出错误结果」的典型。
///
/// `resolve` 保留为 `resolve_with(vars, &[])` 的薄封装：现有调用点不动。
pub fn resolve_with(variables: &[String], overrides: &[(usize, String)]) -> (Resolved, Vec<String>) {
    let has = |n: &str| variables.iter().any(|v| v == n);
    let mut vname = [None; 8];
    let mut missing = Vec::new();

    for (i, s) in SLOTS.iter().enumerate() {
        // 用户指定优先。
        if let Some((_, name)) = overrides.iter().find(|(idx, _)| *idx == s.index) {
            if has(name) {
                // 名字来自调用方而不是 'static 表，所以要 leak 成 'static。
                // 这条路径每次运行只走 8 次，代价可以忽略。
                vname[i] = Some(Box::leak(name.clone().into_boxed_str()) as &'static str);
            } else {
                missing.push(format!(
                    "slot {} ({}) was told to use {:?}, which the file does not have",
                    s.index, s.meaning, name
                ));
            }
            continue;
        }
        match s.candidates.iter().find(|c| has(c)) {
            Some(c) => vname[i] = Some(*c),
            None if s.optional => {}
            None => missing.push(format!(
                "slot {} ({}) has none of {:?}",
                s.index, s.meaning, s.candidates
            )),
        }
    }
    (Resolved { vname }, missing)
}
```

把原 `resolve` 的函数体换成一行：

```rust
pub fn resolve(variables: &[String]) -> (Resolved, Vec<String>) {
    resolve_with(variables, &[])
}
```

`lib.rs` 的重导出加上 `resolve_with`。

- [ ] **Step 4: 跑，确认新旧测试都过**

```bash
cargo test -p colm-forcing --lib slots 2>&1 | tail -6
```

期望：原有的槽位测试**一条都不能红** —— `resolve` 的行为必须不变。

- [ ] **Step 5: 提交**

```bash
git status --short
git add crates/colm-forcing/src/slots.rs crates/colm-forcing/src/slots_tests.rs \
        crates/colm-forcing/src/lib.rs
git commit -m "槽位映射接受用户指定

用户的数据可能叫 TA_F / air_temp，候选名表认不出来。同一份文件里也
可能既有 Tair（塔顶）又有 Tair_2m，而表只认前者。

指定一个文件里没有的变量必须报错，不能悄悄回落到自动匹配 —— 回落会让
用户以为自己选了 A 实际跑的是 B。resolve 保留为 resolve_with(vars, &[])
的薄封装，现有调用点不动。

Confidence: high
Scope-risk: narrow
Tested: cargo test -p colm-forcing --lib slots（新 2 条 + 原有全过）"
```

---

## Task 4: 转换管道

**Files:**
- Modify: `crates/colm-forcing/src/convert.rs`
- Modify: `crates/colm-forcing/src/convert_tests.rs`

- [ ] **Step 1: 写失败的测试**

在 `convert_tests.rs` 末尾加：

```rust
#[test]
fn a_renamed_and_rescaled_variable_lands_in_the_slot_with_the_canonical_name() {
    let dir = std::env::temp_dir().join("colm-convert-rename");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 用户的文件：变量叫 TA_F，单位是摄氏度
    let p = dir.join("user_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00").unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("TA_F", &["time"]).unwrap();
        v.put_attribute("units", "degC").unwrap();
        v.put_values(&[0.0, 25.0], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "TA_F".into(),
            source_units: "degC".into(),
        }],
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    // 落地时用的是**规范名**（槽位的第一个候选名），不是用户的名字
    let got: Vec<f64> = f.variable("Tair").unwrap()
        .get_values(netcdf::Extents::All).unwrap();
    assert_eq!(got, vec![273.15, 298.15]);

    // **换算过的要标出来** —— 否则读文件的人以为那就是源数据里的值
    let note = f.variable("Tair").unwrap()
        .attribute("source").and_then(|a| a.value().ok());
    let note = match note {
        Some(netcdf::AttributeValue::Str(s)) => s,
        other => panic!("Tair 应当带一条 source 属性，得到 {other:?}"),
    };
    assert!(note.contains("TA_F"), "要说出原变量名：{note}");
    assert!(note.contains("degC"), "要说出原单位：{note}");
}
```

- [ ] **Step 2: 跑，确认失败**

```bash
cargo test -p colm-forcing --lib convert 2>&1 | tail -5
```

期望：编译失败，`cannot find type 'Plan'`。

- [ ] **Step 3: 写实现**

在 `convert.rs` 里，`identity` **之后**加：

```rust
/// 一个槽位怎么从源文件取。
pub struct SlotPlan {
    /// 1-based，与 `slots::SLOTS` 的 `index` 对齐。
    pub index: usize,
    /// 源文件里的变量名。
    pub source_name: String,
    /// 源文件里的单位（`units` 属性的原文）。
    pub source_units: String,
}

/// 整份转换方案。
pub struct Plan {
    pub slots: Vec<SlotPlan>,
}

/// 按方案把源文件转成 CoLM 认的约定。
///
/// **落地用规范名**（槽位候选名的第一个），不是用户的名字 —— 转换的
/// 目的正是让下游只认一套约定。
///
/// 每个转换过的变量带一条 `source` 属性，说出它从哪个变量、哪个单位来。
/// **换算过的必须标出来**，否则读文件的人会以为那就是源数据里的值。
pub fn convert(src: &Path, dst: &Path, plan: &Plan) -> Result<()> {
    use crate::slots::SLOTS;
    use crate::units::convert_units;

    let fin = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let mut fout =
        netcdf::create(dst).with_context(|| format!("cannot create {}", dst.display()))?;

    for d in fin.dimensions() {
        fout.add_dimension(&d.name(), d.len())?;
    }

    // 时间轴原样搬过去 —— 重采样不在这一阶段（见 design-prep.md §6）。
    if let Some(t) = fin.variable("time") {
        let dims: Vec<String> = t.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let vals: Vec<f64> = t.get_values(netcdf::Extents::All)?;
        let mut out = fout.add_variable::<f64>("time", &dim_refs)?;
        if let Some(netcdf::AttributeValue::Str(u)) =
            t.attribute("units").and_then(|a| a.value().ok())
        {
            out.put_attribute("units", u.as_str())?;
        }
        out.put_values(&vals, netcdf::Extents::All)?;
    }

    for sp in &plan.slots {
        let slot = SLOTS
            .iter()
            .find(|s| s.index == sp.index)
            .with_context(|| format!("no slot {}", sp.index))?;
        let canonical = slot.candidates[0];

        let v = fin
            .variable(&sp.source_name)
            .with_context(|| format!("{} has no variable {}", src.display(), sp.source_name))?;
        let raw: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        let want_units = canonical_units(slot.index);
        let vals = convert_units(&sp.source_units, want_units, &raw)?;

        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let mut out = fout.add_variable::<f64>(canonical, &dim_refs)?;
        out.put_attribute("units", want_units)?;
        out.put_attribute(
            "source",
            format!(
                "converted from {:?} ({}) by colm-forcing",
                sp.source_name, sp.source_units
            )
            .as_str(),
        )?;
        out.put_values(&vals, netcdf::Extents::All)?;
    }
    Ok(())
}

/// CoLM 期望每个槽位用什么单位。
fn canonical_units(index: usize) -> &'static str {
    match index {
        1 => "K",        // 气温
        2 => "kg/kg",    // 比湿
        3 => "Pa",       // 气压
        4 => "mm/s",     // 降水率
        5 | 6 => "m/s",  // 风
        7 | 8 => "W/m2", // 辐射
        _ => "",
    }
}
```

- [ ] **Step 4: 跑，确认通过**

```bash
cargo test -p colm-forcing --lib convert 2>&1 | tail -6
```

期望：2 条全过（恒等 + 改名换算）。

- [ ] **Step 5: 提交**

```bash
git status --short
git add crates/colm-forcing/src/convert.rs crates/colm-forcing/src/convert_tests.rs
git commit -m "转换管道：按方案改名与换算

落地用规范名（槽位候选名的第一个）而不是用户的名字 —— 转换的目的正是
让下游只认一套约定。每个转换过的变量带 source 属性说出从哪个变量、
哪个单位来：换算过的必须标出来，否则读文件的人会以为那就是源数据里的值。

时间轴原样搬，不重采样 —— 那是后续的事，现在做只会让判据变复杂。

Confidence: high
Scope-risk: moderate
Tested: cargo test -p colm-forcing --lib convert（恒等 + 改名换算）"
```

---

## Task 4b: 多源合成一个槽位（并保留相态）

**这一条修的是一个既有 bug，不只是加功能。**

`slots.rs` 第 4 槽的候选名是 `["Precip", "Rainf"]`，命中 `Rainf` 之后
就不再看 `Snowf` —— Urban-PLUMBER 的 21 个城市站**现在丢掉了全部降雪**。
实测占总降水的比例：

| 站点 | 丢掉 |
|---|---|
| FI-Kumpula / FI-Torni | **24.7%** |
| US-Minneapolis1 / 2 | **16.5%** |
| PL-Lipowa / PL-Narutowicza | **14.9%** |
| CA-Sunset | 9.8% |
| AU-Preston、SG、MX、US-WestPhoenix | **0%** |

**PLUMBER2 不受影响**：90 个站全部只有 `Precip`，零个含 `Snowf`（已实测）。

**AU-Preston 是 0%**，所以它那条基准（264 条、`f_tref` 峰值
311.964998337472 K）修完仍然成立 —— 继续当对照锚点。

### 为什么不能相加了事

CoLM 接收总降水后**自己按湿球温度判相态**（`MOD_RainSnowTemp.F90` 的
`rain_snow_temp`，三种方案，默认 `II`）。所以：

```
观测给的相态（Rainf / Snowf）   ← 实测的事实
CoLM 判出的相态（湿球温度方案）  ← 参数化的推断
```

简单相加会把前者永久换成后者。**合并进第 4 槽供 CoLM 用，同时把
`Rainf` / `Snowf` 原样保留在产物文件里。**

规矩：**转换可以增加信息，不能减少信息。**

**Files:**
- Modify: `crates/colm-forcing/src/convert.rs`
- Modify: `crates/colm-forcing/src/convert_tests.rs`

- [ ] **Step 1: 写失败的测试**

在 `convert_tests.rs` 末尾加：

```rust
#[test]
fn two_sources_sum_into_one_slot_and_both_survive_in_the_output() {
    let dir = std::env::temp_dir().join("colm-convert-sum");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("split_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 3).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00").unwrap();
        t.put_values(&[0.0, 1800.0, 3600.0], netcdf::Extents::All).unwrap();
        for (n, vals) in [("Rainf", [1.0, 0.0, 2.0]), ("Snowf", [0.0, 3.0, 0.5])] {
            let mut v = f.add_variable::<f64>(n, &["time"]).unwrap();
            v.put_attribute("units", "mm/s").unwrap();
            v.put_values(&vals, netcdf::Extents::All).unwrap();
        }
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 4,
            source_name: "Rainf".into(),
            source_units: "mm/s".into(),
            also_add: vec!["Snowf".into()],
        }],
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();

    // 合成的总降水进第 4 槽的规范名
    let precip: Vec<f64> = f.variable("Precip").unwrap()
        .get_values(netcdf::Extents::All).unwrap();
    assert_eq!(precip, vec![1.0, 3.0, 2.5], "总降水应当是两者之和");

    // **两个源变量都要还在** —— 转换可以增加信息，不能减少信息
    let rain: Vec<f64> = f.variable("Rainf")
        .expect("Rainf 必须保留在产物里").get_values(netcdf::Extents::All).unwrap();
    let snow: Vec<f64> = f.variable("Snowf")
        .expect("Snowf 必须保留在产物里").get_values(netcdf::Extents::All).unwrap();
    assert_eq!(rain, vec![1.0, 0.0, 2.0]);
    assert_eq!(snow, vec![0.0, 3.0, 0.5]);

    // source 属性要说出它是合成的，以及 CoLM 会重新判相态
    let note = match f.variable("Precip").unwrap()
        .attribute("source").and_then(|a| a.value().ok()) {
        Some(netcdf::AttributeValue::Str(s)) => s,
        other => panic!("Precip 应当带 source 属性，得到 {other:?}"),
    };
    assert!(note.contains("Rainf"), "要说出来源：{note}");
    assert!(note.contains("Snowf"), "要说出来源：{note}");
}
```

- [ ] **Step 2: 跑，确认失败**

```bash
cargo test -p colm-forcing --lib two_sources 2>&1 | tail -5
```

期望：编译失败 —— `SlotPlan` 还没有 `also_add` 字段。

- [ ] **Step 3: 改实现**

`SlotPlan` 加一个字段：

```rust
pub struct SlotPlan {
    /// 1-based，与 `slots::SLOTS` 的 `index` 对齐。
    pub index: usize,
    /// 源文件里的变量名。
    pub source_name: String,
    /// 源文件里的单位（`units` 属性的原文）。
    pub source_units: String,
    /// 还要**加到这个槽位上**的变量（同单位）。
    ///
    /// 为降水而设：Urban-PLUMBER 把降水分成 `Rainf` 与 `Snowf`，
    /// 而槽位机制一个槽位只能指向一个变量名。不合并就丢掉全部降雪 ——
    /// 实测 FI-Kumpula 少 24.7%。
    ///
    /// **合并之后源变量仍然原样保留在产物里**（见 `convert` 里那段）。
    pub also_add: Vec<String>,
}
```

`convert` 里取值那一段改成先累加：

```rust
        let v = fin
            .variable(&sp.source_name)
            .with_context(|| format!("{} has no variable {}", src.display(), sp.source_name))?;
        let mut raw: Vec<f64> = v.get_values(netcdf::Extents::All)?;

        // 多源合成：同单位相加。
        for extra in &sp.also_add {
            let e = fin.variable(extra).with_context(|| {
                format!("{} has no variable {} (named in also_add)", src.display(), extra)
            })?;
            let add: Vec<f64> = e.get_values(netcdf::Extents::All)?;
            if add.len() != raw.len() {
                anyhow::bail!(
                    "{} has {} steps but {} has {} — cannot add them",
                    sp.source_name, raw.len(), extra, add.len()
                );
            }
            for (a, b) in raw.iter_mut().zip(add.iter()) {
                *a += *b;
            }
        }
```

`source` 属性跟着说清楚：

```rust
        let note = if sp.also_add.is_empty() {
            format!(
                "converted from {:?} ({}) by colm-forcing",
                sp.source_name, sp.source_units
            )
        } else {
            format!(
                "sum of {:?} and {:?} ({}), all kept in this file; \
                 CoLM re-derives phase by wet-bulb temperature (MOD_RainSnowTemp.F90)",
                sp.source_name, sp.also_add, sp.source_units
            )
        };
        out.put_attribute("source", note.as_str())?;
```

**在函数末尾加上「保留源变量」那一段** —— 把 `plan` 里提到过、
但还没写进产物的源变量原样搬过去：

```rust
    // **源变量原样保留。** 转换可以增加信息，不能减少信息 ——
    // 观测给的相态是实测事实，而 CoLM 判出来的是参数化推断，
    // 合成之后把原变量丢掉等于用后者永久换掉前者。
    let mut kept: Vec<String> = Vec::new();
    for sp in &plan.slots {
        if sp.also_add.is_empty() {
            continue;
        }
        kept.push(sp.source_name.clone());
        kept.extend(sp.also_add.iter().cloned());
    }
    for name in kept {
        if fout.variable(&name).is_some() {
            continue;
        }
        let Some(v) = fin.variable(&name) else { continue };
        let dims: Vec<String> = v.dimensions().iter().map(|d| d.name()).collect();
        let dim_refs: Vec<&str> = dims.iter().map(|s| s.as_str()).collect();
        let vals: Vec<f64> = v.get_values(netcdf::Extents::All)?;
        let mut out = fout.add_variable::<f64>(&name, &dim_refs)?;
        for a in v.attributes() {
            if let Ok(netcdf::AttributeValue::Str(s)) = a.value() {
                out.put_attribute(&a.name(), s.as_str())?;
            }
        }
        out.put_values(&vals, netcdf::Extents::All)?;
    }
```

**Task 4、5、6 里构造 `SlotPlan` 的地方都要补 `also_add: Vec::new()`** ——
编译器会点名，按它说的加。

- [ ] **Step 4: 跑，确认三条都过**

```bash
cargo test -p colm-forcing --lib convert 2>&1 | tail -6
```

期望：恒等、改名换算、多源合成三条全过。

- [ ] **Step 5: 实测 Urban-PLUMBER 的降水真的补回来了**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo build -p colm-forcing
U=/Users/zhongwangwei/Desktop/colm-rust/Urban-PLUMBER
./target/debug/forcing-convert "$U/Forcing/FI-Kumpula"*.nc /tmp/fi-kumpula.nc \
  --slot 4=Rainf:mm/s+Snowf
```

（`--slot N=名字:单位+另一个名字` 的语法在 Task 5 里加；若那边还没做，
先写一个临时的 Rust 例子调 `convert`。）

用 python 核对：产物里 `Precip` 的总量应当等于源文件 `Rainf + Snowf`，
且 `Rainf`、`Snowf` 两个变量都还在。**FI-Kumpula 的雪占 24.7%**，
所以合成前后总量差别很明显，肉眼就能看出来。

- [ ] **Step 6: 提交**

```bash
git status --short
git add crates/colm-forcing/src/convert.rs crates/colm-forcing/src/convert_tests.rs
git commit -m "多源合成一个槽位，并保留相态

第 4 槽的候选名是 [Precip, Rainf]，命中 Rainf 之后就不再看 Snowf ——
Urban-PLUMBER 的 21 个城市站现在丢掉了全部降雪，实测 FI-Kumpula 与
FI-Torni 各少 24.7% 的降水，US-Minneapolis 少 16.5%，而模型照样跑得完。
PLUMBER2 的 90 个站全部只有 Precip，不受影响。

不相加了事：CoLM 接收总降水后自己按湿球温度判相态，简单相加等于把
「观测说这是雪」永久换成「模型猜这是不是雪」。所以合并进第 4 槽的同时，
Rainf 与 Snowf 原样保留在产物里，source 属性说明合成方式。

Constraint: 转换可以增加信息，不能减少信息
Confidence: high
Scope-risk: moderate
Tested: 三条单测; FI-Kumpula 实测降水补回 24.7%"
```

---

## Task 5: 命令行入口

**Files:**
- Create: `crates/colm-forcing/src/bin/forcing-convert.rs`

- [ ] **Step 1: 写工具**

参数解析手写，不引 clap —— 与 `forcing-nml.rs` 同一风格（`colm-cli/Cargo.toml`
的注释写明了理由：四个子命令十来个参数，手写比多一个依赖划算）。

`crates/colm-forcing/src/bin/forcing-convert.rs`：

```rust
//! 把一份变量名/单位与 PLUMBER2 不同的强迫场，转成 CoLM 认的约定。
//!
//! 用法: forcing-convert <源文件> <产物> [--slot N=名字:单位 ...]
//!
//! 没给 `--slot` 的槽位走自动匹配。匹配不上就**列出文件里有哪些变量**
//! 再退出 —— 那正是用户下一步要用的信息，只说「缺第 3 槽」帮不上忙。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_forcing::convert::{convert, Plan, SlotPlan};
use colm_forcing::{resolve_with, summarize, SLOTS};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let src = PathBuf::from(
        args.next()
            .context("usage: forcing-convert <src.nc> <dst.nc> [--slot N=name:units ...]")?,
    );
    let dst = PathBuf::from(args.next().context("usage: forcing-convert <src.nc> <dst.nc>")?);

    // --slot 1=TA_F:degC
    let mut given: Vec<SlotPlan> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--slot" => {
                let spec = args.next().context("--slot needs N=name:units")?;
                let (idx, rest) = spec
                    .split_once('=')
                    .with_context(|| format!("--slot {spec:?} is not N=name:units"))?;
                let (name, units) = rest
                    .split_once(':')
                    .with_context(|| format!("--slot {spec:?} is missing :units"))?;
                given.push(SlotPlan {
                    index: idx.parse().with_context(|| format!("{idx:?} is not a slot number"))?,
                    source_name: name.to_string(),
                    source_units: units.to_string(),
                });
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let summary = summarize(&src)?;

    // 没给的槽位交给自动匹配。
    let overrides: Vec<(usize, String)> = given
        .iter()
        .map(|s| (s.index, s.source_name.clone()))
        .collect();
    let (resolved, missing) = resolve_with(&summary.variables, &overrides);
    if !missing.is_empty() {
        for m in &missing {
            eprintln!("  {m}");
        }
        // **把文件里有什么列出来。** 只说「缺第 3 槽」用户无从下手。
        eprintln!("  {} has: {}", src.display(), summary.variables.join(", "));
        bail!("{} slot(s) unresolved", missing.len());
    }

    // 自动匹配到的槽位，单位取文件自己的 `units` 属性。
    let mut plan = Plan { slots: given };
    let f = netcdf::open(&src)?;
    for (i, slot) in SLOTS.iter().enumerate() {
        if plan.slots.iter().any(|s| s.index == slot.index) {
            continue;
        }
        let Some(name) = resolved.vname[i] else { continue };
        let units = f
            .variable(name)
            .and_then(|v| v.attribute("units"))
            .and_then(|a| a.value().ok())
            .and_then(|v| match v {
                netcdf::AttributeValue::Str(s) => Some(s),
                _ => None,
            })
            .unwrap_or_default();
        plan.slots.push(SlotPlan {
            index: slot.index,
            source_name: name.to_string(),
            source_units: units,
        });
    }

    convert(&src, &dst, &plan)?;
    println!("wrote {}", dst.display());
    for s in &plan.slots {
        println!("  slot {} <- {} ({})", s.index, s.source_name, s.source_units);
    }
    Ok(())
}
```

`Cargo.toml` 里声明这个 bin（`forcing-nml` 是怎么声明的就怎么写；
若那边靠 `src/bin/` 自动发现，这里也不用写）。

- [ ] **Step 2: 手工验一次**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo build -p colm-forcing
P=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
./target/debug/forcing-convert \
  "$P/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc" /tmp/cn-cng-converted.nc
```

期望：打印 `wrote /tmp/cn-cng-converted.nc` 与 8 行（或 7 行，标量风没有第 5 槽）
`slot N <- 名字 (单位)`。**这一步等于把 CN-Cng 做了一次恒等转换**，
Task 6 要拿它去跑。

再验报错路径：

```bash
./target/debug/forcing-convert "$P/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc" /tmp/x.nc
```

期望：列出缺哪些槽位，**并列出那个文件里有哪些变量**，非零退出。

- [ ] **Step 3: 提交**

```bash
git status --short
git add crates/colm-forcing/src/bin/forcing-convert.rs crates/colm-forcing/Cargo.toml
git commit -m "强迫场转换的命令行入口

没给 --slot 的槽位走自动匹配，单位取文件自己的 units 属性。
匹配不上时列出文件里有哪些变量 —— 只说「缺第 3 槽」用户无从下手，
而那正是他下一步要用的信息。

参数解析手写不引 clap，与 forcing-nml 同一风格。

Confidence: high
Scope-risk: narrow
Tested: CN-Cng 恒等转换跑通; 拿站点文件当输入验报错路径"

---

## Task 6: CN-Cng 对照 —— 这条管道的真正判据

**Files:**
- Create: `oracle/tests/forcing_convert.rs`

- [ ] **Step 1: 写测试**

**先读 `oracle/tests/generated_case.rs`** —— 它已经有「取 PLUMBER2、
开内核、跑三段、与黄金文件比对」的全套骨架，本测试照它的形状写，
不要另起一套。它的跳过条件是 `PLUMBER2_ROOT` 未设 + 内核不存在。

`oracle/tests/forcing_convert.rs`：

```rust
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
//! 需要 `PLUMBER2_ROOT` 与已构建的 `kernels/default`，与
//! `generated_case.rs` 同一档 —— 没有就跳过。

use std::path::{Path, PathBuf};

use colm_forcing::convert::{convert, Plan, SlotPlan};
use colm_forcing::{resolve_with, summarize, SLOTS};
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

#[test]
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
    let _kernel = Kernel::open(&kernel_dir).expect("kernel opens");

    let src = plumber2.join("Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc");
    let work = repo.join("oracle/work/forcing-convert");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");
    let dst = work.join("CN-Cng_converted_Met.nc");

    // 全部槽位自动匹配、单位取文件自己的 —— 等价于恒等转换。
    let summary = summarize(&src).expect("summarize");
    let (resolved, missing) = resolve_with(&summary.variables, &[]);
    assert!(missing.is_empty(), "CN-Cng 的槽位不该缺：{missing:?}");

    let f = netcdf::open(&src).expect("open src");
    let mut plan = Plan { slots: Vec::new() };
    for (i, slot) in SLOTS.iter().enumerate() {
        let Some(name) = resolved.vname[i] else { continue };
        let units = f
            .variable(name)
            .and_then(|v| v.attribute("units"))
            .and_then(|a| a.value().ok())
            .and_then(|v| match v {
                netcdf::AttributeValue::Str(s) => Some(s),
                _ => None,
            })
            .unwrap_or_default();
        plan.slots.push(SlotPlan {
            index: slot.index,
            source_name: name.to_string(),
            source_units: units,
        });
    }
    drop(f);
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
}
```

**这一版先只比文件层面。** 跑完三段再比 history 是更强的判据，
但它要建算例、要几分钟；文件层面的比对能在秒级抓住绝大多数误差，
先把它钉住。

- [ ] **Step 1b: 再加一条跑完三段的对照**

文件层面相同不等于模型跑出来相同（比如属性丢了会让 CoLM 走别的分支）。
照 `generated_case.rs` 的做法建算例、跑三段、与
`oracle/golden/CN-Cng_hist_2008-01.nc` 比对。

**期望**：`identical: 129 variables, 10 dimensions (ignoring ["create_time"])`

这一条慢（几分钟），标成 `#[ignore]` 或用环境变量开关都可以 ——
**但必须存在**，且在报告里给出实际跑过一次的输出。

- [ ] **Step 2: 跑**

```bash
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo test -p oracle --test forcing_convert -- --nocapture 2>&1 | tail -15
```

**若不 identical**：把差异的变量与首个不同的点报出来，**不要调松判据**。
差在最后一位通常意味着某处做了十进制往返或多余的算术。

- [ ] **Step 3: 全量测试与提交**

```bash
cargo test --workspace 2>&1 | tail -6      # 基线 299 passed
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo fmt --all -- --check && echo "fmt 干净"
```

---

## 附：这份计划**不做**什么

- **不转内置数据集**（PLUMBER2 / Urban-PLUMBER 继续直读，黄金基准不动）
- **不做界面**（前处理页的强迫场子栏是 A2）
- **不做时间轴重采样**（判据会变复杂，等真需求）
- **不做表格导入**（阶段 C）
- **不引入新依赖**
