# 里程碑 4 实施计划：colm-forcing

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给定一个 PLUMBER2 强迫场文件，产出 CoLM 单点能直接用的 `nl_colm_forcing` namelist，并在开跑之前把「能跑但结果错」的几种情形拦下来。

**Architecture:** 一个 `colm-forcing` crate。`met.rs` 读强迫场文件的元数据（时间轴、变量、三个参考高度）并做契约校验；`spec.rs` 是纯计算的日历换算与槽位映射；`render.rs` 生成 namelist 文本，并用里程碑 2 的 `colm-namelist` 把自己的产物解析回来自证。命令行是薄壳。

**Tech Stack:** Rust 2021、`netcdf` 0.12（静态链接）、`anyhow`、`colm-namelist`（本仓库）。无新增外部依赖。

---

## 这个里程碑不做数据转换

先说清楚，否则名字会误导人。**CoLM 直接读 PLUMBER2 的 Met 文件**，
`MOD_UserSpecifiedForcing.F90:683` 的 `CASE ('POINT')` 是
`metfilename = '/'//trim(fprefix(1))` —— 文件原样喂给模型，没有中间产物。

所以 `colm-forcing` 产出的「强迫场」是**那份 namelist**，加上一组开跑前的校验。
把 8 个变量映射到 CoLM 的固定槽位、把时间轴翻译成 `startyr/endyr`、
确认文件满足 CoLM 那些不成文的契约 —— 这些才是这一层的工作。

---

## 已实测的事实基础

每一条都在本机量过。其中三条推翻或修正了先前写下的东西。

### 强迫场语料：90 个文件，变量集完全一致

`$PLUMBER2_ROOT/Forcing/*_Met.nc` 共 **90** 个，变量集**完全相同**：

```
Tair  Qair  Psurf  Precip  SWdown  LWdown  Wind  Wind2
reference_height_v  reference_height_t  reference_height_q
time  x  y
```

| 项 | 实测 |
|---|---|
| 时间单位 | 90/90 都是 `"seconds since YYYY-MM-DD HH:MM:SS"` |
| 单位起点年 | 90/90 等于文件名里的起始年 |
| 时间步长 | **1800 s 有 88 个，3600 s 有 2 个** |
| 步长是否文件内均匀 | 90/90 均匀 |
| 时间步数 | 35041 – 333121 |
| NaN / 填充值 | 90/90 为零 |

**时间步长不是普适的 1800 s。** 两个站点是逐小时，而算例 namelist 里
`DEF_simulation_time%timestep = 1800.` 是写死的 —— 这两者必须对上。

**`Wind2` 的维度是 `(time, x, y)`**，与其余变量的 `(time, y, x)` **转置**。
CoLM 只读 `Wind`（第 6 槽），不碰 `Wind2`，但任何顺手去读它的代码都会拿到转置的数组。

### 三个参考高度，且 namelist 里的值到不了模型

Met 文件自带 `reference_height_v/t/q` 三个标量。实测：

- **30/90 个站点三者互不相同**（CA-SF1 是 v=12.1 而 t=q=1.5，差 8 倍）
- **4/90 与站点文件的单一 `reference_height` 不一致**（DK-Sor 43 vs 57）

CoLM 的取值链是：

```
MOD_UserSpecifiedForcing.F90:116   HEIGHT_V = DEF_forcing%HEIGHT_V      ← namelist 赋值
MOD_Forcing.F90:294-310            POINT 下若文件有 reference_height_v，覆盖之
MOD_Forcing.F90:557                flush_block_data(forc_xy_hgt_u, HEIGHT_V)  ← 喂给模型
```

**所以 namelist 里的 `HEIGHT_V/T/Q` 在本语料下到不了模型** —— 90 个文件都带那三个
变量，覆盖必然发生。现有模板里的 `HEIGHT_* = 6.0` 因此不是物理错误，而是**误导**：
它看起来像一个站点相关的设置，实际被立刻覆盖。（CN-Cng 的参考高度确实是 6.0，
90 个里仅 3 个如此 —— 又一个「对唯一验证过的站点成立」的巧合。）

本 crate 仍然写这三个字段，但值取自文件，并在注释里写明 CoLM 会覆盖它们。
理由是这份 namelist 也是给人看的。

### CoLM 的槽位契约

槽位固定为 `1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW`。PLUMBER2 只有标量
`Wind`，所以：

```
vname    = 'Tair' 'Qair' 'Psurf' 'Precip' 'NULL' 'Wind' 'SWdown' 'LWdown'
tintalgo = 'linear' 'linear' 'linear' 'nearest' 'NULL' 'linear' 'linear' 'linear'
```

`tintalgo` 的合法取值只有四个：`NULL` / `linear` / `nearest` / `coszen`
（`MOD_Forcing.F90:448,461,474,492`）。

**POINT 下只有 `fprefix(1)` 被读**，2–8 槽从不使用。现有模板把 8 个都填成同一个
文件名，无害但会让人以为它们各有用处。

### 时间单位的解析是硬编码字符偏移 —— 但失败是响亮的

`MOD_Forcing.F90:1253-1255`：

```fortran
timestr = timeunit(15:18) // ' ' // timeunit(20:21) // ' ' // timeunit(23:24) &
   // ' ' // timeunit(26:27) // ' ' // timeunit(29:30) // ' ' // timeunit(32:33)
read(timestr,*) year, month, day, hour, minute, second
```

对 `"seconds since 2008-01-01 00:00:00"` 正好切出年月日时分秒。把同一段逻辑抄成
独立程序实测：

| 单位串 | `iostat` |
|---|---|
| `seconds since 2008-01-01 00:00:00` | 0 |
| `hours since 2008-01-01 00:00:00` | **5010** |
| `days since 2008-01-01 00:00:00` | **5010** |
| `seconds since 2008-1-1 0:0:0`（不补零） | **5010** |

CoLM 的调用**没有 `iostat`**，所以这三种都会以 Fortran 运行期错误终止，而
`Fortran runtime error` 正在 `colm-kernel` 的失败标记里。

**这修正了 design.md 先前的说法**（原写「不报错」）。脆是真脆，静默则不是。
校验仍然值得做 —— 在开跑前给出一句人话，好过让用户去读 Fortran 的崩溃栈。

### 真正静默的那一个：跑过强迫场的末端

`MOD_Forcing.F90:1107` 的注释是它自己写的：

```fortran
! when reaching the END of forcing data, show a Warning but still try to run
```

模拟窗口超出 `startyr/endyr` 时，CoLM **打印一句 Warning 然后继续跑**，产出完整的
history 文件。而 `colm-kernel` 的失败标记里没有 `Warning:` —— 这样的运行会被判成功。

**这是本里程碑要拦的头号情形。** 校验必须在开跑之前比对模拟窗口与强迫场覆盖。

### 与算例 namelist 的三处耦合

| 算例里的字段 | 约束 |
|---|---|
| `DEF_simulation_time%timestep` | 应与强迫场步长一致；实测 88 个站点 1800 s、2 个 3600 s |
| `DEF_simulation_time%greenwich` | 必须 `.FALSE.`：PLUMBER2 的时间轴是地方时，而 `MOD_TimeManager.F90:74-79` 的强制覆盖在 `#ifndef SinglePoint` 内 |
| `start_*` / `end_*` | 必须落在强迫场覆盖范围内，见上一条 |

本里程碑只产出 `nl_colm_forcing`，算例 namelist 归后续里程碑；但校验要能拿到
模拟窗口并据此判断。

---

## 文件结构

```
crates/colm-forcing/
   Cargo.toml
   src/lib.rs
   src/civil.rs            儒略日 <-> 年月日（纯计算）      + civil_tests.rs
   src/met.rs              读强迫场文件的元数据（netcdf）   + met_tests.rs
   src/check.rs            契约校验（纯计算）                + check_tests.rs
   src/render.rs           生成 namelist 文本                + render_tests.rs
   src/bin/forcing-nml.rs  命令行
   tests/real_forcing.rs   对 90 个真实文件的集成测试
```

`civil.rs`、`check.rs`、`render.rs` 是纯计算，不碰 netcdf，测试在任何机器上都能跑。
`met.rs` 与 `tests/real_forcing.rs` 需要真实数据，与里程碑 2/3 的同类测试一样归入
`golden` 作业。

---

## Task 1: crate 骨架

**Files:**
- Create: `crates/colm-forcing/Cargo.toml`
- Create: `crates/colm-forcing/src/lib.rs`
- Create: `crates/colm-forcing/src/{civil,met,check,render}.rs`（占位）
- Modify: 根 `Cargo.toml`（workspace members）

- [ ] **Step 1: 写 `crates/colm-forcing/Cargo.toml`**

```toml
[package]
name = "colm-forcing"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
anyhow.workspace = true
colm-namelist = { path = "../colm-namelist" }
netcdf = { workspace = true, features = ["static"] }

[lints]
workspace = true
```

`features = ["static"]` 不能省，理由同 `colm-srfdata`：根 manifest 只是模板，
静态链接由各成员自己开。

- [ ] **Step 2: 写 `crates/colm-forcing/src/lib.rs`**

```rust
//! 把一个 PLUMBER2 强迫场文件翻译成 CoLM 的 `nl_colm_forcing`，并在开跑前校验。
//!
//! **本 crate 不转换数据。** CoLM 直接读 PLUMBER2 的 Met 文件
//! （`MOD_UserSpecifiedForcing.F90:683`，POINT 下 `metfilename = fprefix(1)`），
//! 所以这一层产出的是那份 namelist 加一组校验，不是新的强迫场文件。
//!
//! 校验的重点不在「文件坏了」——90 个真实文件零 NaN、零填充值、步长均匀——
//! 而在几种**能跑完却给出错误结果**的配置。其中最要紧的一种 CoLM 自己写在
//! 注释里：`MOD_Forcing.F90:1107` 说跑过强迫场末端时「show a Warning but still
//! try to run」，而 `colm-kernel` 的失败标记里没有 `Warning:`。
//!
//! 各模块的重导出在后续 Task 里加上，那时它们指向的东西才存在。

pub mod check;
pub mod civil;
pub mod met;
pub mod render;
```

- [ ] **Step 3: 建四个占位模块**

`src/{civil,met,check,render}.rs` 各一行：

```rust
//! 占位，后续 Task 实现。
```

- [ ] **Step 4: 加入 workspace**

根 `Cargo.toml` 的 members 加 `"crates/colm-forcing"`，保持字母序 ——
它排在 `crates/colm-kernel` 之前，即列表的第一项。

- [ ] **Step 5: 三道门禁**

Run: `cargo build`
Expected: 通过，无警告。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Run: `cargo fmt --all --check`
Expected: 都无输出。

Run: `cargo test --workspace 2>&1 | grep 'test result'`
Expected: 里程碑 0–3 的 96 个测试仍全绿。

- [ ] **Step 6: 提交**

```bash
git add Cargo.toml Cargo.lock crates/colm-forcing
git commit -m "Add the colm-forcing crate skeleton"
```

---

## Task 2: 日历换算 —— 先写失败的测试

强迫场的时间轴是「自某个时刻起的秒数」，而 namelist 要的是年月。中间需要一次
日历换算，且**不引入新依赖**。

**Files:**
- Create: `crates/colm-forcing/src/civil_tests.rs`
- Modify: `crates/colm-forcing/src/civil.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;

#[test]
fn the_epoch_round_trips() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(civil_from_days(0), (1970, 1, 1));
}

#[test]
fn a_leap_day_is_a_real_day() {
    // 2008 是闰年；2100 不是（能被 100 整除而不能被 400 整除）
    assert_eq!(civil_from_days(days_from_civil(2008, 2, 29)), (2008, 2, 29));
    assert_eq!(
        days_from_civil(2008, 3, 1) - days_from_civil(2008, 2, 28),
        2
    );
    assert_eq!(
        days_from_civil(2100, 3, 1) - days_from_civil(2100, 2, 28),
        1
    );
}

#[test]
fn every_day_across_two_centuries_round_trips() {
    // 逐日全覆盖，不抽样。日历换算的错法几乎全在边界上（月末、闰年、世纪），
    // 抽样正好躲开它们。
    let from = days_from_civil(1900, 1, 1);
    let to = days_from_civil(2100, 1, 1);
    for d in from..to {
        let (y, m, day) = civil_from_days(d);
        assert_eq!(days_from_civil(y, m, day), d, "day {d} -> {y}-{m}-{day}");
        assert!((1..=12).contains(&m), "month {m} out of range at day {d}");
        assert!((1..=31).contains(&day), "day {day} out of range at day {d}");
    }
}

#[test]
fn a_stamp_advances_by_whole_seconds() {
    // 强迫场时间轴是「自起点的秒数」，所以这是本模块唯一被外部调用的入口。
    let start = Stamp {
        year: 2008,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    assert_eq!(start.plus_seconds(0), start);
    assert_eq!(
        start.plus_seconds(1800),
        Stamp {
            year: 2008,
            month: 1,
            day: 1,
            hour: 0,
            minute: 30,
            second: 0
        }
    );
    // 2008 是闰年：366 天 = 17568 个半小时步
    assert_eq!(
        start.plus_seconds(17568 * 1800),
        Stamp {
            year: 2009,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0
        }
    );
}

#[test]
fn crossing_a_year_boundary_lands_on_the_right_month() {
    // endyr/endmo 就是这么算出来的，错一天就会写错 namelist 的结束月
    let s = Stamp {
        year: 2008,
        month: 12,
        day: 31,
        hour: 23,
        minute: 30,
        second: 0,
    };
    let t = s.plus_seconds(1800);
    assert_eq!((t.year, t.month, t.day), (2009, 1, 1));
    assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
}
```

- [ ] **Step 2: 建空壳**

`crates/colm-forcing/src/civil.rs`：

```rust
//! 儒略日与公历日期的互换。

#[cfg(test)]
#[path = "civil_tests.rs"]
mod civil_tests;
```

- [ ] **Step 3: 确认失败**

Run: `cargo test -p colm-forcing 2>&1 | tail -20`
Expected: 编译失败，找不到 `days_from_civil` / `civil_from_days` / `Stamp`。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-forcing/src/civil.rs crates/colm-forcing/src/civil_tests.rs
git commit -m "Add failing tests for the calendar arithmetic"
```

---

## Task 3: 日历换算 —— 实现

**Files:**
- Modify: `crates/colm-forcing/src/civil.rs`
- Modify: `crates/colm-forcing/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
//! 儒略日与公历日期的互换。
//!
//! 强迫场的时间轴是「自某个时刻起的秒数」，而 namelist 要的是 `startyr`/`endyr`
//! 这样的年月。中间这一步换算自己写，不引入 chrono —— 本仓库的依赖每多一个，
//! 三个平台的静态构建就多一处可能出岔的地方，而这里要的只是两个函数。
//!
//! 算法是 Howard Hinnant 的 `days_from_civil` / `civil_from_days`，
//! 对公历前推有效，不做闰秒。`civil_tests.rs` 对 1900–2100 逐日往返验证。

/// 1970-01-01 起的天数。公历，可为负。
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// `days_from_civil` 的逆。
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}

/// 一个公历时刻。不带时区 —— PLUMBER2 的时间轴是**地方时**，
/// 而 CoLM 单点正是靠 `DEF_simulation_time%greenwich = .FALSE.` 接受这一点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Stamp {
    /// 往后走整数秒。
    pub fn plus_seconds(&self, secs: i64) -> Stamp {
        let day = days_from_civil(self.year, self.month, self.day);
        let tod = self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64;
        let total = day * 86400 + tod + secs;
        let (d, rem) = (total.div_euclid(86400), total.rem_euclid(86400));
        let (year, month, dayn) = civil_from_days(d);
        Stamp {
            year,
            month,
            day: dayn,
            hour: (rem / 3600) as u32,
            minute: ((rem % 3600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }
}

#[cfg(test)]
#[path = "civil_tests.rs"]
mod civil_tests;
```

- [ ] **Step 2: 给 `lib.rs` 加重导出**

加进 `pub use` 块（rustfmt 决定位置）：

```rust
pub use civil::{civil_from_days, days_from_civil, Stamp};
```

- [ ] **Step 3: 测试通过**

Run: `cargo test -p colm-forcing`
Expected: `test result: ok. 5 passed; 0 failed`

- [ ] **Step 4: 格式与 lint，然后提交**

```bash
cargo fmt --all && cargo fmt --all --check && cargo clippy -p colm-forcing --all-targets -- -D warnings
git add crates/colm-forcing/src
git commit -m "Do the calendar arithmetic without taking on a dependency"
```

---

## Task 4: 契约校验 —— 测试与实现

纯计算：输入是一份已经读出来的元数据描述，输出是一组问题。**不碰 netcdf**，
所以这些测试在任何机器上都能跑。

**Files:**
- Create: `crates/colm-forcing/src/check_tests.rs`
- Modify: `crates/colm-forcing/src/check.rs`
- Modify: `crates/colm-forcing/src/lib.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;
use crate::civil::Stamp;

fn ok_met() -> MetSummary {
    MetSummary {
        time_units: "seconds since 2008-01-01 00:00:00".into(),
        start: Stamp {
            year: 2008,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        steps: 35041,
        step_seconds: 1800.0,
        step_uniform: true,
        height_v: 6.0,
        height_t: 6.0,
        height_q: 6.0,
        variables: [
            "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect(),
    }
}

#[test]
fn a_healthy_file_has_no_problems() {
    assert!(check(&ok_met(), None).is_empty());
}

#[test]
fn a_units_string_colm_cannot_parse_is_reported() {
    // CoLM 在硬编码字符偏移处解析（MOD_Forcing.F90:1253）。实测 hours since、
    // days since、以及不补零的 seconds since 都让 read 返回 iostat 5010，
    // 而 CoLM 没有 iostat，于是以 Fortran 运行期错误终止。
    // 报错是响亮的，但一句人话好过一个崩溃栈。
    for bad in [
        "hours since 2008-01-01 00:00:00",
        "days since 2008-01-01 00:00:00",
        "seconds since 2008-1-1 0:0:0",
    ] {
        let mut m = ok_met();
        m.time_units = bad.into();
        let p = check(&m, None);
        assert!(p.iter().any(|x| x.contains("time units")), "{bad}: {p:?}");
    }
}

#[test]
fn a_missing_variable_is_reported_by_name() {
    let mut m = ok_met();
    m.variables.retain(|v| v != "LWdown");
    let p = check(&m, None);
    assert!(p.iter().any(|x| x.contains("LWdown")), "{p:?}");
}

#[test]
fn an_uneven_time_step_is_reported() {
    // CoLM 按固定步长在时间轴上取样；步长不均匀会让它取到错误的时刻，
    // 而不会报错。
    let mut m = ok_met();
    m.step_uniform = false;
    assert!(check(&m, None).iter().any(|x| x.contains("uniform")));
}

#[test]
fn a_window_past_the_end_of_the_forcing_is_reported() {
    // 这是本 crate 存在的头号理由。CoLM 自己的注释是
    // "when reaching the END of forcing data, show a Warning but still try to run"
    // （MOD_Forcing.F90:1107），而 colm-kernel 的失败标记里没有 Warning:。
    // 那样的运行会被判成功，产出一份完整而错误的 history。
    let m = ok_met(); // 覆盖 2008-01-01 起 35041 个半小时步，约到 2009-12-31
    let window = (
        Stamp {
            year: 2009,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Stamp {
            year: 2011,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
    );
    let p = check(&m, Some(window));
    assert!(p.iter().any(|x| x.contains("beyond the forcing")), "{p:?}");
}

#[test]
fn a_window_before_the_start_is_reported_too() {
    let m = ok_met();
    let window = (
        Stamp {
            year: 2007,
            month: 6,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Stamp {
            year: 2008,
            month: 6,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
    );
    assert!(check(&m, Some(window))
        .iter()
        .any(|x| x.contains("before the forcing")));
}

#[test]
fn a_window_inside_the_coverage_is_fine() {
    let m = ok_met();
    let window = (
        Stamp {
            year: 2008,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        Stamp {
            year: 2008,
            month: 1,
            day: 11,
            hour: 0,
            minute: 0,
            second: 0,
        },
    );
    assert!(check(&m, Some(window)).is_empty());
}

#[test]
fn an_hourly_file_is_not_a_problem_by_itself() {
    // 实测 90 个站点里有 2 个是 3600 s。那本身没问题 —— 但算例里的
    // DEF_simulation_time%timestep 必须跟着改，所以校验要说出步长。
    let mut m = ok_met();
    m.step_seconds = 3600.0;
    assert!(check(&m, None).is_empty());
    assert_eq!(m.timestep_hint(), 3600);
}
```

- [ ] **Step 2: 写实现**

```rust
//! 开跑之前的契约校验。
//!
//! 校验的重点不是「文件坏了」—— 实测 90 个真实强迫场文件零 NaN、零填充值、
//! 步长在文件内均匀。重点是几种**能跑完却给出错误结果**的配置。
//!
//! 最要紧的一种 CoLM 自己写在注释里（`MOD_Forcing.F90:1107`）：
//! 模拟窗口跑过强迫场末端时「show a Warning but still try to run」，
//! 而 `colm-kernel` 的失败标记里没有 `Warning:` —— 那样的运行会被判成功，
//! 产出一份完整而错误的 history 文件。

use crate::civil::Stamp;

/// CoLM 单点必需的 7 个强迫变量。第 5 槽（u 风）在 PLUMBER2 下是 `NULL`，
/// 标量 `Wind` 进第 6 槽，所以这里只有 7 个而不是 8 个。
pub const REQUIRED_VARS: [&str; 7] = [
    "Tair", "Qair", "Psurf", "Precip", "Wind", "SWdown", "LWdown",
];

/// 从强迫场文件读出来的元数据。`met.rs` 负责填它，本模块只做纯计算。
#[derive(Debug, Clone)]
pub struct MetSummary {
    pub time_units: String,
    pub start: Stamp,
    pub steps: usize,
    pub step_seconds: f64,
    pub step_uniform: bool,
    pub height_v: f64,
    pub height_t: f64,
    pub height_q: f64,
    pub variables: Vec<String>,
}

impl MetSummary {
    /// 强迫场覆盖的最后一个时刻。
    pub fn end(&self) -> Stamp {
        let n = self.steps.saturating_sub(1) as i64;
        self.start.plus_seconds(n * self.step_seconds as i64)
    }

    /// 算例 namelist 里 `DEF_simulation_time%timestep` 该取的值。
    pub fn timestep_hint(&self) -> i64 {
        self.step_seconds as i64
    }
}

/// 检查一份强迫场描述，可选地连同模拟窗口一起检查。返回全部问题；空即通过。
pub fn check(m: &MetSummary, window: Option<(Stamp, Stamp)>) -> Vec<String> {
    let mut p = Vec::new();

    // CoLM 按固定字符位置解析这个字符串，所以形状必须一模一样。
    if !units_parseable(&m.time_units) {
        p.push(format!(
            "time units {:?} is not the exact form CoLM parses; \
             it reads fixed character positions and needs \"seconds since YYYY-MM-DD HH:MM:SS\"",
            m.time_units
        ));
    }

    for v in REQUIRED_VARS {
        if !m.variables.iter().any(|x| x == v) {
            p.push(format!("required forcing variable {v} is missing"));
        }
    }

    if !m.step_uniform {
        p.push(
            "the time step is not uniform; CoLM samples the axis at a fixed stride and would \
             read the wrong instants without saying so"
                .to_string(),
        );
    }

    if m.steps == 0 {
        p.push("the forcing file has no time steps".to_string());
    }

    if let Some((from, to)) = window {
        let start = m.start;
        let end = m.end();
        if before(&from, &start) {
            p.push(format!(
                "the simulation starts {from:?} which is before the forcing begins at {start:?}"
            ));
        }
        if before(&end, &to) {
            p.push(format!(
                "the simulation ends {to:?} which is beyond the forcing, which stops at {end:?}; \
                 CoLM would print a warning and keep running"
            ));
        }
    }

    p
}

/// CoLM 的解析靠固定字符位置（`MOD_Forcing.F90:1253`），所以这里也按位置检查。
fn units_parseable(u: &str) -> bool {
    let b = u.as_bytes();
    if b.len() < 33 || !u.starts_with("seconds since ") {
        return false;
    }
    let digits = |a: usize, z: usize| b[a - 1..z].iter().all(|c| c.is_ascii_digit());
    digits(15, 18)
        && digits(20, 21)
        && digits(23, 24)
        && digits(26, 27)
        && digits(29, 30)
        && digits(32, 33)
}

fn before(a: &Stamp, b: &Stamp) -> bool {
    key(a) < key(b)
}

fn key(s: &Stamp) -> (i32, u32, u32, u32, u32, u32) {
    (s.year, s.month, s.day, s.hour, s.minute, s.second)
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod check_tests;
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

```rust
pub use check::{check, MetSummary, REQUIRED_VARS};
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-forcing`
Expected: `test result: ok. 13 passed; 0 failed`（5 civil + 8 check）

- [ ] **Step 5: 格式与 lint，然后提交**

```bash
git add crates/colm-forcing/src
git commit -m "Catch the forcing mistakes that would otherwise run to completion"
```

---

## Task 5: 生成 namelist —— 测试与实现

**Files:**
- Create: `crates/colm-forcing/src/render_tests.rs`
- Modify: `crates/colm-forcing/src/render.rs`
- Modify: `crates/colm-forcing/src/lib.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;
use crate::check::MetSummary;
use crate::civil::Stamp;

fn spec() -> ForcingSpec {
    ForcingSpec {
        dir: "/data/PLUMBER2s/Forcing/".into(),
        file: "CN-Cng_2008-2009_FLUXNET2015_Met.nc".into(),
        met: MetSummary {
            time_units: "seconds since 2008-01-01 00:00:00".into(),
            start: Stamp {
                year: 2008,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            },
            steps: 35041,
            step_seconds: 1800.0,
            step_uniform: true,
            height_v: 6.0,
            height_t: 6.0,
            height_q: 6.0,
            variables: crate::check::REQUIRED_VARS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        },
    }
}

/// 把生成的文本用 colm-namelist 解析回来，取一个字段。
///
/// 这样断言而不是比字符串，是因为要验的是**它说了什么**，不是它长什么样。
fn field(text: &str, path: &str) -> String {
    let doc = colm_namelist::parse(text).expect("our own output must parse");
    doc.get(path)
        .unwrap_or_else(|| panic!("{path} missing from:\n{text}"))
        .to_string()
}

#[test]
fn our_own_output_parses() {
    // 生成器写出的东西必须能被本仓库的解析器读回来。两边都是自己的代码，
    // 但它们是独立写的，互相验证比各自自证强。
    let text = render(&spec());
    colm_namelist::parse(&text).expect("must parse");
}

#[test]
fn the_slot_map_is_colms_fixed_one() {
    // 槽位固定为 1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW。
    // PLUMBER2 只有标量 Wind，所以第 5 槽是 NULL，Wind 进第 6 槽。
    let text = render(&spec());
    assert_eq!(
        field(&text, "DEF_forcing%vname"),
        "'Tair' 'Qair' 'Psurf' 'Precip' 'NULL' 'Wind' 'SWdown' 'LWdown'"
    );
    assert_eq!(
        field(&text, "DEF_forcing%tintalgo"),
        "'linear' 'linear' 'linear' 'nearest' 'NULL' 'linear' 'linear' 'linear'"
    );
}

#[test]
fn the_window_comes_from_the_time_axis_not_from_the_filename() {
    // 文件名里的 2008-2009 只是个约定；覆盖范围由时间轴决定。
    let text = render(&spec());
    assert_eq!(field(&text, "DEF_forcing%startyr"), "2008");
    assert_eq!(field(&text, "DEF_forcing%startmo"), "1");
    assert_eq!(field(&text, "DEF_forcing%endyr"), "2009");
    assert_eq!(field(&text, "DEF_forcing%endmo"), "12");
}

#[test]
fn only_the_first_fprefix_slot_is_written() {
    // POINT 下 CoLM 只读 fprefix(1)（MOD_UserSpecifiedForcing.F90:683）。
    // 先前的模板把 8 个槽都填成同一个文件名 —— 无害，但会让人以为
    // 它们各有用处。
    let text = render(&spec());
    assert_eq!(
        field(&text, "DEF_forcing%fprefix(1)"),
        "'CN-Cng_2008-2009_FLUXNET2015_Met.nc'"
    );
    // 只数字段本身；注释里也提到 fprefix，那是有意的。
    assert_eq!(text.matches("DEF_forcing%fprefix").count(), 1);
}

#[test]
fn the_heights_come_from_the_file_and_say_so() {
    // namelist 里的 HEIGHT_* 在 POINT 下会被文件里的 reference_height_*
    // 覆盖（MOD_Forcing.F90:294-310），所以这三行是给人看的。
    // 写文件里的真值而不是一个常数，才不会误导下一个读它的人。
    let mut s = spec();
    s.met.height_v = 12.1;
    s.met.height_t = 1.5;
    s.met.height_q = 1.5;
    let text = render(&s);
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_V"), "12.1");
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_T"), "1.5");
    assert!(
        text.contains("overwritten"),
        "the note about CoLM overwriting these must survive: {text}"
    );
}

#[test]
fn an_integer_valued_height_still_looks_like_a_real() {
    // Rust 的 Display 把 6.0 打成 "6"，写进 namelist 就会被读成整数。
    // CoLM 那三个字段是 real(r8)，Fortran 读得进去，但逐字段比对时
    // Int(6) 与 Real("6.0") 是两回事 —— 而这正是 Task 8 要做的比对。
    // 实测不少站点的高度是整数值（AU-Lit 是 31 / 33 / 33）。
    let mut s = spec();
    s.met.height_v = 6.0;
    s.met.height_t = 33.0;
    let text = render(&s);
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_V"), "6.0");
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_T"), "33.0");
}

#[test]
fn the_constants_colm_needs_are_present() {
    let text = render(&spec());
    assert_eq!(field(&text, "DEF_forcing%dataset"), "'POINT'");
    assert_eq!(field(&text, "DEF_forcing%NVAR"), "8");
    assert_eq!(field(&text, "DEF_forcing%solarin_all_band"), ".true.");
    assert_eq!(
        field(&text, "DEF_dir_forcing"),
        "'/data/PLUMBER2s/Forcing/'"
    );
}

#[test]
fn a_directory_without_a_trailing_slash_still_works() {
    // CoLM 拼路径是 dir//fprefix，中间不补斜杠。少一个斜杠会让它去找
    // ForcingCN-Cng_....nc，报的错与真正的原因无关。
    let mut s = spec();
    s.dir = "/data/PLUMBER2s/Forcing".into();
    let text = render(&s);
    assert_eq!(
        field(&text, "DEF_dir_forcing"),
        "'/data/PLUMBER2s/Forcing/'"
    );
}
```

- [ ] **Step 2: 写实现**

```rust
//! 生成 `nl_colm_forcing`。
//!
//! 生成的是文本而不是结构，因为这份 namelist 也要给人看：注释里那几句
//! 「为什么第 5 槽是 NULL」「为什么 HEIGHT_* 会被覆盖」比字段本身更容易丢。
//!
//! 但产物会被 `colm-namelist` 解析回来做断言（见 `render_tests.rs`），
//! 所以它不只是拼字符串 —— 拼错了测试会红。

use crate::check::MetSummary;

/// 生成一份 namelist 所需的一切。
#[derive(Debug, Clone)]
pub struct ForcingSpec {
    /// 强迫场目录。CoLM 拼路径时不补斜杠，所以这里保证结尾有一个。
    pub dir: String,
    /// 强迫场文件名（不含目录）。
    pub file: String,
    pub met: MetSummary,
}

/// CoLM 的固定槽位：1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW。
/// PLUMBER2 只有标量 `Wind`，所以第 5 槽是 `NULL`。
const VNAME: [&str; 8] = [
    "Tair", "Qair", "Psurf", "Precip", "NULL", "Wind", "SWdown", "LWdown",
];
const TINTALGO: [&str; 8] = [
    "linear", "linear", "linear", "nearest", "NULL", "linear", "linear", "linear",
];

/// 渲染成 namelist 文本。
pub fn render(s: &ForcingSpec) -> String {
    let dir = if s.dir.ends_with('/') {
        s.dir.clone()
    } else {
        format!("{}/", s.dir)
    };
    let end = s.met.end();
    let quoted = |xs: &[&str]| {
        xs.iter()
            .map(|x| format!("'{x}'"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "&nl_colm_forcing\n\
         \n\
         ! 由 colm-forcing 生成。CoLM 直接读 PLUMBER2 的 Met 文件，不做转换。\n\
         \n\
         \x20  DEF_dir_forcing              = '{dir}'\n\
         \n\
         \x20  DEF_forcing%dataset          = 'POINT'\n\
         \x20  DEF_forcing%solarin_all_band = .true.\n\
         \n\
         ! HEIGHT_* 取自强迫场文件的 reference_height_v/t/q。CoLM 在 POINT 下会用\n\
         ! 文件里的值 overwritten 掉这三行（MOD_Forcing.F90:294-310），所以它们是\n\
         ! 给人看的；写文件里的真值而不是常数，才不会误导下一个读它的人。\n\
         \x20  DEF_forcing%HEIGHT_V         = {hv:?}\n\
         \x20  DEF_forcing%HEIGHT_T         = {ht:?}\n\
         \x20  DEF_forcing%HEIGHT_Q         = {hq:?}\n\
         \n\
         \x20  DEF_forcing%NVAR             = 8\n\
         \x20  DEF_forcing%startyr          = {sy}\n\
         \x20  DEF_forcing%startmo          = {sm}\n\
         \x20  DEF_forcing%endyr            = {ey}\n\
         \x20  DEF_forcing%endmo            = {em}\n\
         \n\
         ! POINT 下 CoLM 只读 fprefix(1)（MOD_UserSpecifiedForcing.F90:683），\n\
         ! 其余 7 个槽从不使用。\n\
         \x20  DEF_forcing%fprefix(1)       = '{file}'\n\
         \n\
         ! 槽位固定为 1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW。\n\
         ! PLUMBER2 只有标量 Wind，故第 5 槽为 'NULL'，Wind 进第 6 槽。\n\
         \x20  DEF_forcing%vname            = {vname}\n\
         \x20  DEF_forcing%tintalgo         = {tint}\n\
         /\n",
        dir = dir,
        hv = s.met.height_v,
        ht = s.met.height_t,
        hq = s.met.height_q,
        sy = s.met.start.year,
        sm = s.met.start.month,
        ey = end.year,
        em = end.month,
        file = s.file,
        vname = quoted(&VNAME),
        tint = quoted(&TINTALGO),
    )
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

```rust
pub use render::{render, ForcingSpec};
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-forcing`
Expected: `test result: ok. 21 passed; 0 failed`（5 civil + 8 check + 8 render）

- [ ] **Step 5: 格式与 lint，然后提交**

```bash
git add crates/colm-forcing/src
git commit -m "Generate the forcing namelist and read it back to check it"
```

---

## Task 6: 读强迫场文件

从这里开始碰真实数据。

**Files:**
- Modify: `crates/colm-forcing/src/met.rs`
- Create: `crates/colm-forcing/src/met_tests.rs`
- Modify: `crates/colm-forcing/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
//! 读强迫场文件的元数据。
//!
//! 只读元数据与时间轴，不读那几十万步的场数据 —— 最大的文件有 333121 个
//! 时间步，而这一层要的只是「起点、步长、步数、三个高度、有哪些变量」。
//!
//! 时间轴要全读一遍，因为「步长是否均匀」只能这样确认，而不均匀的步长会让
//! CoLM 取到错误的时刻却不报错。实测 90 个文件都是均匀的。

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::check::MetSummary;
use crate::civil::Stamp;

/// 读一个 PLUMBER2 强迫场文件的元数据。
pub fn summarize(file: &Path) -> Result<MetSummary> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;

    let time = f
        .variable("time")
        .with_context(|| format!("no time variable in {}", file.display()))?;
    let units = time
        .attribute("units")
        .context("time has no units attribute")?
        .value()?;
    // NC_CHAR 与 NC_STRING 是两个变体。实测 PLUMBER2 的文件用的是前者
    // （HDF5 层面 |S33，正好是 "seconds since YYYY-MM-DD HH:MM:SS" 的长度），
    // 但那个语料是被 ncatted 预处理过的，别人的文件未必如此 —— 两种都接。
    let time_units = match units {
        netcdf::AttributeValue::Str(s) => s,
        netcdf::AttributeValue::Strs(v) => v
            .into_iter()
            .next()
            .context("time units attribute is an empty string array")?,
        other => bail!("time units is not a string: {other:?}"),
    };

    let t: Vec<f64> = time.get_values(netcdf::Extents::All)?;
    if t.is_empty() {
        bail!("{} has an empty time axis", file.display());
    }
    let step_seconds = if t.len() > 1 { t[1] - t[0] } else { 0.0 };
    let step_uniform = t
        .windows(2)
        .all(|w| (w[1] - w[0] - step_seconds).abs() < 1e-6);

    let scalar = |n: &str| -> Option<f64> {
        f.variable(n)
            .and_then(|v| v.get_values::<f64, _>(netcdf::Extents::All).ok())
            .and_then(|x: Vec<f64>| x.first().copied())
    };

    let variables: Vec<String> = f.variables().map(|v| v.name()).collect();

    Ok(MetSummary {
        start: parse_units_start(&time_units)?,
        time_units,
        steps: t.len(),
        step_seconds,
        step_uniform,
        height_v: scalar("reference_height_v").unwrap_or(f64::NAN),
        height_t: scalar("reference_height_t").unwrap_or(f64::NAN),
        height_q: scalar("reference_height_q").unwrap_or(f64::NAN),
        variables,
    })
}

/// 按 CoLM 的方式解析起点：固定字符位置，不做通用解析。
///
/// 刻意与 `MOD_Forcing.F90:1253-1255` 一致 —— 这里要回答的是「CoLM 会读到
/// 什么」，而不是「这个字符串按 CF 约定是什么意思」。两者在畸形输入上会分歧，
/// 而分歧的那一侧正是要报告的。
fn parse_units_start(u: &str) -> Result<Stamp> {
    let b = u.as_bytes();
    if b.len() < 33 {
        bail!("time units {u:?} is too short for CoLM's fixed-position parse");
    }
    let num = |a: usize, z: usize| -> Result<u32> {
        std::str::from_utf8(&b[a - 1..z])
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .with_context(|| format!("time units {u:?} has no number at characters {a}..={z}"))
    };
    Ok(Stamp {
        year: num(15, 18)? as i32,
        month: num(20, 21)?,
        day: num(23, 24)?,
        hour: num(26, 27)?,
        minute: num(29, 30)?,
        second: num(32, 33)?,
    })
}

#[cfg(test)]
#[path = "met_tests.rs"]
mod met_tests;
```

- [ ] **Step 2: 写 `met_tests.rs`**

```rust
use std::path::PathBuf;

use super::*;

/// 强迫场数据的位置。缺失时测试**失败**而不是跳过 ——
/// 跳过会被读成通过，这个仓库栽过一次。
fn forcing() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("PLUMBER2_ROOT")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s".to_string()),
    )
    .join("Forcing");
    assert!(p.is_dir(), "PLUMBER2 forcing not found at {}", p.display());
    p
}

#[test]
fn cn_cng_summarizes_to_the_measured_values() {
    let m = summarize(&forcing().join("CN-Cng_2008-2009_FLUXNET2015_Met.nc")).unwrap();
    assert_eq!(m.time_units, "seconds since 2008-01-01 00:00:00");
    assert_eq!(m.start.year, 2008);
    assert_eq!(m.start.month, 1);
    assert_eq!(m.step_seconds, 1800.0);
    assert!(m.step_uniform);
    // 实测 35089 步，末值 63158400 s = 731 天，即 2010-01-01 00:00 整。
    // （35041 是全语料的**最小**步数，属于别的站点，不是这里的。）
    assert_eq!(m.steps, 35089);
    assert_eq!(m.height_v, 6.0);
    assert_eq!(m.end().year, 2010);
    assert_eq!(m.end().month, 1);
}

#[test]
fn the_three_heights_are_read_separately() {
    // 实测 30/90 个站点三者不同。读成一个值会在三分之一的站点上出错。
    let m = summarize(&forcing().join("CA-SF1_2004-2006_FLUXNET2015_Met.nc")).unwrap();
    assert!(
        (m.height_v - 12.1).abs() < 1e-4,
        "height_v was {}",
        m.height_v
    );
    assert!(
        (m.height_t - 1.5).abs() < 1e-4,
        "height_t was {}",
        m.height_t
    );
    assert_eq!(m.height_t, m.height_q);
    assert_ne!(m.height_v, m.height_t);
}

#[test]
fn a_missing_file_is_an_error() {
    assert!(summarize(&forcing().join("no-such-site_Met.nc")).is_err());
}
```

并在 `met.rs` 末尾加：

```rust
#[cfg(test)]
#[path = "met_tests.rs"]
mod met_tests;
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

```rust
pub use met::summarize;
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-forcing`
Expected: `test result: ok. 24 passed; 0 failed`

文件名 `CA-SF1_2004-2006_FLUXNET2015_Met.nc` 与高度值 12.1 / 1.5 / 1.5 都是实测的。

- [ ] **Step 5: 格式与 lint，然后提交**

```bash
git add crates/colm-forcing/src
git commit -m "Read a forcing file the way CoLM will read it"
```

---

## Task 7: 命令行与对 90 个站点的集成测试

**Files:**
- Create: `crates/colm-forcing/src/bin/forcing-nml.rs`
- Create: `crates/colm-forcing/tests/real_forcing.rs`

- [ ] **Step 1: 写命令行**

```rust
//! 从一个 PLUMBER2 强迫场文件生成 CoLM 的 nl_colm_forcing。
//!
//! 用法: forcing-nml <Met 文件> [输出路径]
//!
//! 不给输出路径就打到标准输出。契约问题一律打到标准错误并以非零码退出 ——
//! 其中「模拟窗口跑过强迫场末端」那一种，CoLM 自己只会警告然后继续跑。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_forcing::{check, render, summarize, ForcingSpec};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let met = PathBuf::from(
        args.next()
            .context("usage: forcing-nml <Met.nc> [out.nml]")?,
    );
    let out = args.next().map(PathBuf::from);

    let summary = summarize(&met)?;
    let problems = check(&summary, None);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("  {p}");
        }
        bail!("{} problem(s) with {}", problems.len(), met.display());
    }

    let dir = met
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let file = met
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .to_string();
    let text = render(&ForcingSpec {
        dir,
        file,
        met: summary.clone(),
    });

    match out {
        Some(p) => {
            std::fs::write(&p, &text).with_context(|| format!("cannot write {}", p.display()))?;
            eprintln!(
                "wrote {} covering {}-{:02} to {}-{:02}, step {} s",
                p.display(),
                summary.start.year,
                summary.start.month,
                summary.end().year,
                summary.end().month,
                summary.timestep_hint()
            );
        }
        None => print!("{text}"),
    }
    Ok(())
}
```

- [ ] **Step 2: 写 `tests/real_forcing.rs`**

```rust
//! 对全部 90 个真实强迫场文件跑一遍。
//!
//! 合成用例能证明每一步的算术，只有真实文件能证明**它对所有站点都成立**。
//! 本仓库先前两次栽在「对唯一验证过的站点成立」的常数上：土壤颜色档写死为 10
//! （90 个里只有 1 个是 10），参考高度写死为 6.0（90 个里只有 3 个是 6.0）。

use std::path::PathBuf;

use colm_forcing::{check, render, summarize, ForcingSpec};

fn forcing_dir() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("PLUMBER2_ROOT")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s".to_string()),
    )
    .join("Forcing");
    assert!(p.is_dir(), "PLUMBER2 forcing not found at {}", p.display());
    p
}

fn met_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(forcing_dir())
        .expect("readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with("_Met.nc"))
        .collect();
    out.sort();
    assert!(
        out.len() >= 85,
        "expected ~90 forcing files, found {}",
        out.len()
    );
    out
}

#[test]
fn every_forcing_file_passes_the_contract_check() {
    let mut bad = Vec::new();
    for f in met_files() {
        match summarize(&f) {
            Ok(m) => {
                let p = check(&m, None);
                if !p.is_empty() {
                    bad.push(format!("{}: {p:?}", f.display()));
                }
            }
            Err(e) => bad.push(format!("{}: {e:#}", f.display())),
        }
    }
    assert!(
        bad.is_empty(),
        "{} file(s) failed:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn every_namelist_parses_back_and_names_its_own_file() {
    for f in met_files() {
        let m = summarize(&f).expect("summarizes");
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let text = render(&ForcingSpec {
            dir: forcing_dir().display().to_string(),
            file: name.clone(),
            met: m,
        });
        let doc = colm_namelist::parse(&text)
            .unwrap_or_else(|e| panic!("{}: our own output did not parse: {e:#}", f.display()));
        let got = doc
            .get("DEF_forcing%fprefix(1)")
            .unwrap_or_else(|| panic!("{}: no fprefix(1)", f.display()))
            .to_string();
        assert_eq!(got, format!("'{name}'"), "{}", f.display());
    }
}

#[test]
fn the_time_steps_are_the_two_measured_values() {
    // 实测 88 个站点 1800 s、2 个 3600 s。这条不是要求它们都一样 ——
    // 而是钉住「有两种」这件事，因为算例里的 timestep 必须跟着走。
    let mut counts = std::collections::BTreeMap::new();
    for f in met_files() {
        let m = summarize(&f).expect("summarizes");
        *counts.entry(m.timestep_hint()).or_insert(0usize) += 1;
    }
    println!("time steps across sites: {counts:?}");
    assert_eq!(counts.get(&1800), Some(&88), "{counts:?}");
    assert_eq!(counts.get(&3600), Some(&2), "{counts:?}");
}

#[test]
fn the_three_heights_differ_at_about_a_third_of_sites() {
    // 实测 30/90。这条钉住的是「必须分别读三个」这件事：若某天变成 0，
    // 说明读法退化成了一个值，而那会在三分之一的站点上出错。
    let mut differ = 0usize;
    for f in met_files() {
        let m = summarize(&f).expect("summarizes");
        if m.height_v != m.height_t || m.height_t != m.height_q {
            differ += 1;
        }
    }
    println!("sites where the three reference heights differ: {differ}");
    assert!((20..=40).contains(&differ), "measured 30, got {differ}");
}
```

- [ ] **Step 3: 跑**

Run: `cargo test -p colm-forcing`
Expected: 单元测试 24 个 + 集成测试 4 个全绿，并打出步长与高度的统计。

Run: `cargo run -p colm-forcing --bin forcing-nml -- \
  "$PLUMBER2_ROOT/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc"`
Expected: 打出一份 namelist，`startyr = 2008`、**`endyr = 2010`、`endmo = 1`**。

`endyr` 是 2010 而不是文件名里的 2009 —— 时间轴末值是 63158400 s，从 2008-01-01
起正好 731 天，落在 2010-01-01 00:00 整。文件名只是命名约定，时间轴才是数据。
另见 Task 8 的预期差异表。

三个 `HEIGHT_*` 应当是 `6.0` 而不是 `6`：CN-Cng 的三个高度都是整数值，
而 Rust 的 Display 会把 `6.0` 打成 `6`，那在 namelist 里会被读成整数。

- [ ] **Step 4: 格式与 lint，然后提交**

```bash
git add crates/colm-forcing
git commit -m "Generate a namelist for every real forcing file"
```

---

## Task 8: 接进黄金回归

黄金算例现在用手写的 `forcing.nml.in` 模板。换成生成的，并要求 history 逐位不变。

**Files:**
- Create: `oracle/cases/CN-Cng/met.txt`、`oracle/cases/CN-Cng-wet/met.txt`
- Delete: `oracle/cases/CN-Cng/forcing.nml.in`、`oracle/cases/CN-Cng-wet/forcing.nml.in`
- Modify: `oracle/Cargo.toml`、`oracle/src/bin/golden_run.rs`

- [ ] **Step 1: 先确认生成的与手写的等价**

```bash
cargo run -p colm-forcing --bin forcing-nml -- \
  "$PLUMBER2_ROOT/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc" /tmp/forcing-new.nml
```

用 `colm-namelist` 把新旧两份都解析出来，逐字段比对。**预期差异**：

| 字段 | 手写模板 | 生成的 | |
|---|---|---|---|
| `HEIGHT_V/T/Q` | 6.0 | 6.0 | 相同 —— CN-Cng 的文件里三个值恰好都是 6.0 |
| `fprefix(2..8)` | 各填一遍文件名 | **不写** | POINT 下从不读 |
| `endyr` / `endmo` | 2009 / 12 | **2010 / 1** | 见下 |
| 其余 | | | 相同 |

**`endyr`/`endmo` 的差异是对的，不要改回去。** 时间轴的末值是 63158400 s，
从 2008-01-01 起正好 731 天，落在 **2010-01-01 00:00:00** —— 文件确实有一个
落在 2010 年 1 月的采样点。手写模板里的 2009/12 是照文件名 `2008-2009` 填的，
而文件名只是命名约定。

生成器按**时间轴的末值**填，因为这两个字段唯一的用处是 CoLM 那句
「跑过末端就警告」的粗判据（`MOD_Forcing.F90:1108`），如实描述数据比照文件名
猜更有意义。本 crate 的 `check` 做的是对**实际末刻**的精确比较，比那条粗判据严。

两个黄金窗口都在 2008 年，所以这一项不影响本次回归。**若出现表外的差异，
停下来查清楚。**

- [ ] **Step 2: 让 `golden-run` 用生成的 namelist**

`golden_run.rs:59-60` 现在读 `forcing.nml.in` 并把 `@PLUMBER2_ROOT@` 替换掉。
改成调用 `colm_forcing::{summarize, render}` 现生成。这样黄金回归就同时成了
`colm-forcing` 的验收：**生成的 namelist 必须让两个窗口的 history 逐位不变。**

两处要先解决：

**一、`oracle` 要依赖 `colm-forcing`。** 在 `oracle/Cargo.toml` 加
`colm-forcing = { path = "../crates/colm-forcing" }`。

**二、生成器要知道用哪个 Met 文件。** 文件名现在藏在模板的 `fprefix(1)` 里，
而两个算例共用同一个 Met 文件、案例名却不同（`CN-Cng` 与 `CN-Cng-wet`）——
从案例名推不出来，靠通配符去猜更糟。所以每个算例目录放一个单行文件
`met.txt`，内容就是 Met 文件的基名：

```
CN-Cng_2008-2009_FLUXNET2015_Met.nc
```

`CN-Cng` 与 `CN-Cng-wet` 各放一份，内容相同。`golden_run.rs` 读它，
拼上 `$PLUMBER2_ROOT/Forcing/`，交给 `summarize` 与 `render`。
一行文件换掉一份 30 行模板，且哪个算例用哪份强迫场变成显式的。

**具体改法。** `golden_run.rs:57-60` 现在是：

```rust
    fs::write(
        work.join("forcing.nml"),
        subst(&fs::read_to_string(case_dir.join("forcing.nml.in"))?),
    )?;
```

换成：

```rust
    // 强迫场 namelist 现生成，而不是展开一份手写模板。让黄金回归用生成器，
    // 等于每次回归都在验它 —— 生成的 namelist 若改变了语义，history 会先变。
    let met_name = fs::read_to_string(case_dir.join("met.txt"))
        .with_context(|| format!("no met.txt in {}", case_dir.display()))?
        .trim()
        .to_string();
    let forcing_dir = plumber2.join("Forcing");
    let met = forcing_dir.join(&met_name);
    let summary = colm_forcing::summarize(&met)?;
    let problems = colm_forcing::check(&summary, None);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("  {p}");
        }
        bail!("{} problem(s) with {}", problems.len(), met.display());
    }
    fs::write(
        work.join("forcing.nml"),
        colm_forcing::render(&colm_forcing::ForcingSpec {
            dir: forcing_dir.display().to_string(),
            file: met_name,
            met: summary,
        }),
    )?;
```

`subst` 仍然用于 `case.nml`，只是不再用于强迫场那一份。

- [ ] **Step 3: 重跑两个窗口并比对**

```bash
./oracle/scripts/build_kernel.sh waterheat
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-run -- CN-Cng-wet
cargo run -p oracle --bin golden-compare -- oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

Expected: `identical: 129 variables`。**这里出现任何差异都必须解释清楚再继续** ——
namelist 的语义变了才会改变结果，而本 Task 的前提是它没变。

- [ ] **Step 4: 删掉手写模板**

生成器取代它之后，留着一份手写的会让人不知道该改哪个。

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "Generate the golden case's forcing namelist instead of templating it"
```

---

## Task 9: CI 与文档收尾

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`

- [ ] **Step 1: CI**

**实测结果：确实在 `--lib` 范围内，而且不只 `colm-forcing`。**
`cargo test --workspace --lib --bins` 会跑到 **8 条需要真实数据的测试**——
`colm-forcing` 的 `met_tests` 3 条，加上里程碑 3 留下的 `colm-srfdata`
`raster_tests` 5 条。托管 runner 上没有 PLUMBER2 与 rawdata，这 8 条会全红
（它们是断言失败而非跳过，那是有意的）。

所以两处一起移出 `src/`：

- `crates/colm-forcing/src/met_tests.rs` → `crates/colm-forcing/tests/met.rs`
- `crates/colm-srfdata/src/raster_tests.rs` → `crates/colm-srfdata/tests/raster.rs`

`use super::*` 改成走 crate 的公开路径（`colm_forcing::met::summarize`、
`colm_srfdata::raster::{point_f64, point_i32}`），并把 `src/` 那两个文件末尾的
`#[cfg(test)] #[path] mod ...;` 摘掉。移完 `--lib --bins` 对这两类零命中，
而 `golden` 作业的 `cargo test --workspace` 照样带上它们。

`rust` 作业的 yaml **不需要改动** —— 它跑的命令本来就对，是测试放错了地方。

- [ ] **Step 2: README 补一节**

在「站点地表参数」之后插入一节讲 `colm-forcing`：它不转换数据、只生成 namelist
加校验；它拦的是哪一种「能跑但结果错」；以及三个参考高度为什么必须分别读。

- [ ] **Step 3: 全量验证**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -q -p oracle --bin tier-check -- oracle/golden/*.nc
git diff --check
```

- [ ] **Step 4: 提交**

```bash
git add .github/workflows/ci.yml README.md
git commit -m "Document the forcing layer and wire it into CI"
```

---

## 完成判据

逐条可验证：

- [ ] `cargo test --workspace` 通过；`colm-forcing` 的 24 个单元测试
      + 4 个真实强迫场测试全部执行（不是跳过）
- [ ] **90/90 个真实强迫场文件通过契约校验**
- [ ] 90 份生成的 namelist 都能被 `colm-namelist` 解析回来，且 `fprefix(1)` 指向自己
- [ ] 时间步长统计为 **1800 s × 88 + 3600 s × 2**
- [ ] 三个参考高度不同的站点数落在 20–40（实测 30）
- [ ] 模拟窗口越出强迫场覆盖时**报告问题**，而不是留给 CoLM 打一句 Warning
- [ ] 非 `"seconds since YYYY-MM-DD HH:MM:SS"` 的时间单位被报告
- [ ] 用生成的 namelist 重跑，两个窗口的 history **逐位不变**
- [ ] `forcing.nml.in` 已删除
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 与
      `cargo fmt --all --check` 无输出；`git status --short` 为空

---

## 留给后续里程碑的

- **算例 namelist 的生成**（`nl_colm`）不在本轮。`timestep` 与强迫场步长的一致、
  `greenwich = .FALSE.`、模拟窗口的边界，本 crate 都能算，但把它们写进算例
  namelist 属于编排层。
- **非 PLUMBER2 的强迫场**：本 crate 的槽位映射写死了 PLUMBER2 的变量名。
  FLUXNET 原始文件或别的网络需要另一张映射表，届时再抽象。
- **`Wind2`**：维度是 `(time, x, y)`，与其余变量转置。CoLM 不读它，本 crate 也不读，
  但任何将来想用它的代码都要先转置。
- **降水的相态与拆分**：CoLM 内部把总降水按比例拆成大尺度与对流，本 crate 不介入。
