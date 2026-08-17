# 里程碑 6 实施计划：让 schema 说真话

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `colm-schema` 能完整描述一个单点算例，并让「这个内核能产出哪些输出变量」在**开跑之前**就答得出来。

**Architecture:** 两张生成表，判据都取自 CoLM 自己的源码而非命名约定。配置侧改用 `namelist /.../` 语句作权威定义（取代 `DEF_` 前缀白名单）；输出侧从 `MOD_Hist.F90` 的三道闸门生成「宏集合 → 可写变量集」，并用入库的黄金文件做经验校验。

**Tech Stack:** Rust 1.85.1、`xtask` 代码生成、产物入库 + drift 测试。不引入新依赖。

---

## 为什么现在做这个，而不是直接做 GUI

GUI 要渲染什么、要把用户的选择写进哪个文件、要在勾选输出变量时告诉用户哪些**这个内核根本产不出来** —— 三件事全都依赖这两张表。表不对，GUI 就是把错误信息渲染得更漂亮。

本轮**不含 GUI**。Tauri 骨架、命令层与 uPlot 归 `plan-m7`。

---

## 已实测的事实基础

本节每一条都在本机量过，命令与数字都在。

### 一个能跑的单点算例只设 43 个字段

`oracle/cases/CN-Cng/case.nml` 63 行、43 个不同字段。两个黄金算例之间**只差 4 行**
（算例名、站点文件路径、起止月日）。而 `colm-schema` 有 713 个字段、178 个顶层。
GUI 要渲染的是前者的量级，不是后者。

### schema 两头都不对

用 CoLM 自己的 `namelist` 语句作权威定义去对：

```
namelist /nl_colm/         197 个成员
namelist /nl_colm_forcing/   2 个成员（DEF_dir_forcing, DEF_forcing）
namelist /nl_colm_history/   1 个成员（DEF_hist_vars）
                          ---
                           200
```

| | 数量 | 内容 |
|---|---|---|
| 在 namelist 里、schema 没有 | 28 | 其中 4 个是派生类型容器（`DEF_domain` / `DEF_forcing` / `DEF_hist_vars` / `DEF_simulation_time`），schema 存的是它们的成员，**这 4 个不算缺**；真正缺的是 **24 个** |
| 真正缺的 24 个 | 21 + 3 | `SITE_*` 4 个 + `USE_SITE_*` 17 个（`MOD_Namelist.F90` 的 **Part 3: For Single Point** 整段）；外加 `USE_srfdata_from_3D_gridded_data` / `USE_srfdata_from_larger_region` / `USE_zip_for_aggregation` |
| schema 有、任何 namelist 组都没有 | 6 | `DEF_dir_history` / `DEF_dir_landdata` / `DEF_dir_restart` / `DEF_USE_IGBP` / `DEF_USE_USGS` / `DEF_Wetland_finundation_scheme` |

后果是具体的：43 个字段里 schema 只认得 30 个（大小写修好之后），而认不得的
13 个恰好是单点最关键的那一块。GUI 若照 schema 渲染，**会漏掉整个站点配置区，
同时给出 6 个改了也没用的输入框** —— 那 6 个里 `DEF_dir_history` 在
`MOD_Namelist.F90:1406` 被 `DEF_dir_output` + `DEF_CASE_NAME` 无条件覆盖。

根因在 `xtask/src/main.rs:77`：

```rust
// 顶层只收 DEF_ 开头的；类型成员全收
if owner.is_none() && !decl.name.starts_with("DEF_") {
```

**判据本身就错。** 一个名字是不是可设字段，取决于它在不在 `namelist` 语句里，
不取决于前缀。换掉这条判据，顺带把 `ieee_arithmetic`（`USE` 语句被误当成声明）
也自动排除掉 —— 一条规则替掉两个启发式。

### `namelist` 语句的解析有三个坑（已预跑验证）

1. **续行里夹空行**：`DEF_domain, &` 之后隔一个空行才是 `SITE_fsitedata`。
   按「上一行以 `&` 结尾就取下一行」会在这里断掉，只解析出 2 个成员。
2. **续行符之后带行尾注释**：`DEF_LAI_MONTHLY, & !add by zhongwang wei @ sysu`。
3. **有一个成员被宏包住**：`DEF_file_GIEMS` 在 `#if (defined TRACER) && (defined BGC)`
   之内。它是真字段，只是仅在那个预设下可设。

预跑结果（`namelist_groups()` 见 Task 2）：200 个名字、197/2/1 分三组、
24 个非 `DEF_` 前缀、`DEF_file_GIEMS` 命中、6 个不可设字段正确落空。

### 输出变量有三道闸门，不是一道

`history_var_type` 有 **482 个开关，343 个默认为真**。而 waterheat 预设的一次
真实运行只写出 **119 个**。差额不是 bug，是三道闸门：

| 闸门 | 位置 | waterheat 下的效果 |
|---|---|---|
| 1. 编译期宏 | `MOD_Hist.F90` 的 `#ifdef` | 456 个写出点 → **123** 个可写 |
| 2. 运行时 `DEF_*` 条件 | 内联 `.and.` + 外层 `IF (DEF_*) THEN` | 123 里有 **10** 个带条件；本次运行 6 真 4 假 |
| 3. 变量自己的开关 | `DEF_hist_vars%X` | 默认全开，未再减 |

**113（无条件）+ 6（条件成立）= 119**，与黄金文件逐个吻合。

带运行时条件的 10 个，条件原文与本次运行的真假：

| 变量 | 运行时条件 | 本次 |
|---|---|---|
| `qlayer` | `DEF_USE_VariablySaturatedFlow` | 真 |
| `vegwp` | `DEF_USE_PLANTHYDRAULICS` | 真 |
| `wetwat` | `DEF_USE_Dynamic_Wetland` | 真 |
| `o3uptakesha` / `o3uptakesun` | `DEF_USE_OZONESTRESS` | 真 |
| `lake_deficit` | `.not. DEF_USE_Dynamic_Lake` | 真 |
| `dz_lake` | `DEF_USE_Dynamic_Lake` | 假 |
| `qcharge` | `(.not.DEF_USE_VariablySaturatedFlow)` | 假 |
| `t2m_wmo` | `DEF_Output_2mWMO` | 假 |
| `xy_hpbl` | `DEF_USE_CBL_HEIGHT` | 假 |

`qlayer` 与 `qcharge` 是同一个条件的两侧，恰好演示了这道闸门是双向的 ——
不是「条件成立才加」，而是「条件决定写哪一个」。而那个条件正是 CoLM 打印的
9 条覆盖消息里第一条说的事：**「`DEF_USE_VariablySaturatedFlow` 被自动设为
`.true.`」**。覆盖消息与变量的有无是同一件事的两面，GUI 要能把这两头连起来说。

### 456 而不是 466：有 10 个写出点是被注释掉的

对 `MOD_Hist.F90` 直接 `grep "'f_...'"` 会得到 466 个字面量，但其中 **10 个
整段被 `!` 注释掉**：`cwddecomp`、`cwdprod`、以及 8 个 `pd*`
（`pdcorn` / `pdcotton` / `pdrice1` / `pdrice2` / `pdsoybean` / `pdsugarcane` /
`pdswheat` / `pdwwheat`）。它们永远产不出来。

所以提取器**必须在剥掉注释之后、并且只在 `CALL write_history_variable` 调用
内部**取字面量。按行 grep 会让表多报 10 个变量，而多报在 GUI 里表现为
「勾了却没有」，是最难查的一类。剥注释时要注意 `!` 可能出现在字符串里，
判断得跳过引号内的。

### 三道闸门一起提取已预跑，零漏报

把 waterheat 的宏集合
（`CoLMDEBUG,LULC_IGBP,RangeCheck,SinglePoint,extend_interception,vanGenuchten_Mualem_SOIL_MODEL`）
作用于生成的映射：

```
表 456 个；宏放行 123；其中无运行时条件 113

带运行时条件的 10 个:
  dz_lake        <- DEF_USE_Dynamic_Lake
  lake_deficit   <- .not. DEF_USE_Dynamic_Lake
  o3uptakesha    <- DEF_USE_OZONESTRESS
  o3uptakesun    <- DEF_USE_OZONESTRESS
  qcharge        <- (.not.DEF_USE_VariablySaturatedFlow)
  qlayer         <- DEF_USE_VariablySaturatedFlow
  t2m_wmo        <- DEF_Output_2mWMO
  vegwp          <- DEF_USE_PLANTHYDRAULICS
  wetwat         <- DEF_USE_Dynamic_Wetland
  xy_hpbl        <- DEF_USE_CBL_HEIGHT

无条件放行但没写出 (0): []
写出了但无条件集里没有 (6): [lake_deficit, o3uptakesha, o3uptakesun, qlayer, vegwp, wetwat]
```

最后一行不是漏报 —— 那 6 个恰是运行时条件**成立**的那些，113 + 6 = 119，
与黄金文件逐个吻合。

**零漏报**是关键：静态映射不会漏掉任何真实产出的变量。它只会在
「宏放行但运行时条件不成立」时多报，而那 4 个已被逐一定位并连条件原文
一起记进表里，所以 GUI 说得出「为什么没有」。

其他宏集合的实测值（Task 3 的测试要断言它们）：

| 宏集合 | 宏放行 | 其中无条件 |
|---|---|---|
| waterheat | 123 | 113 |
| waterheat + `BGC` | 326 | 256 |
| waterheat + `CatchLateralFlow` | 124 | 114 |

### 宏条件的文法只有 4 种

`MOD_Hist.F90` 里出现的全部条件形态：

```
#ifdef X                                  13+9+8+7+5+4+4+2+1+1 处
#ifndef X                                 2+1 处
#if (defined A || defined B)              7+1 处
#if (defined X)                           4 处
```

**没有 `&&`，没有更深的嵌套。** 所以不需要通用表达式求值器，一个四分支的
`parse_cond` 就够。新出现的形态必须让生成器**报错**而不是静默当成真。

### 开关名与写出字面量不是一一对应

不要按名字配对。实测：

- `bedout` 有开关，写出点是 `'f_bedout_'//...` 的**拼接**
- `fsen_gimp` 有开关，写出字面量是 `'f_fsengimp'`（**下划线位置不同**）
- 482 个开关里 50 个在全仓库找不到同名字面量；683 个字面量里 251 个没有同名开关

正确做法是**从同一个调用里同时读两者**：

```fortran
CALL write_history_variable_2d ( DEF_hist_vars%deadstemc, &
    a_deadstemc, file_hist, 'f_deadstemc', itime_in_file, sumarea, filter, &
```

467 处调用（`_2d` 388 / `_3d` 54 / `_urb_2d` 20 / `_4d` 5），461 个不同的
`DEF_hist_vars%X`。本计划只需要字面量与它的闸门，不需要配对开关名 ——
但**闸门 2 的内联 `.and.` 就写在首参里**，所以调用必须整体读取，不能只抓字面量行。

### 黄金文件入库，所以经验闸门能在 CI 三平台跑

`oracle/golden/*.nc` 共 2.7 MB，已入库。`oracle` 的 `judge` 测试本来就读它们。

---

## 文件结构

```
xtask/src/
├── main.rs              命令分发；新增 gen-histmap
├── namelist.rs          【新】namelist 语句 → 「名字 -> 组名」
├── schema.rs            【新】从 main.rs 拆出：声明区扫描与渲染
└── hist.rs              【新】MOD_Hist.F90 → 变量的三道闸门

crates/colm-schema/src/
├── field.rs             Field 新增 group 字段
└── generated.rs         重新生成：713 -> 737

crates/colm-hist/        【新 crate】
├── src/lib.rs           writable(macros) 与 Gate 模型
├── src/generated.rs     入库产物
├── src/lib_tests.rs
└── tests/drift.rs       重新生成必须逐字节一致

oracle/
├── fixtures/waterheat_hist_vars.txt   【新】黄金文件里的 119 个变量名
└── tests/histmap.rs                   【新】fixture 必须与黄金文件一致
```

拆 `xtask/src/main.rs`（253 行）是因为本轮要往里加两个独立的扫描器；
再往单文件里堆会让三个扫描器互相干扰阅读。

---

## Task 1: `Field` 增加 `group`，先写失败的测试

**Files:**
- Modify: `crates/colm-schema/src/field.rs`
- Modify: `crates/colm-schema/src/field_tests.rs`

- [ ] **Step 1: 给 `Field` 加字段**

在 `field.rs` 的 `Field` 结构里，`line` 之前插入：

```rust
    /// 这个字段可以从**哪个 namelist 组**设置。
    ///
    /// `None` 意味着它在 `MOD_Namelist.F90` 里有声明、有默认值，但不出现在
    /// 任何 `namelist /.../` 语句里 —— 也就是**用户改不了它**。实测 6 个：
    /// `DEF_dir_history` / `DEF_dir_landdata` / `DEF_dir_restart` 由
    /// `DEF_dir_output` 派生（`MOD_Namelist.F90:1406` 无条件覆盖），
    /// `DEF_USE_IGBP` / `DEF_USE_USGS` / `DEF_Wetland_finundation_scheme` 由宏决定。
    /// GUI 应当把它们显示成只读的派生值，而不是给一个改了没用的输入框。
    ///
    /// 派生类型成员继承容器所在的组，所以 `DEF_forcing%dataset` 是
    /// `nl_colm_forcing`、`DEF_hist_vars%*` 是 `nl_colm_history`。
    /// **这正是 GUI 需要知道的「这个字段该写进哪个文件」。**
    pub group: Option<&'static str>,
```

- [ ] **Step 2: 写失败的测试**

追加到 `crates/colm-schema/src/field_tests.rs`：

```rust
#[test]
fn the_single_point_section_is_in_the_table() {
    // MOD_Namelist.F90 的 Part 3 用 SITE_ / USE_SITE_ 前缀，而生成器原先
    // 按 `DEF_` 白名单收字段，于是把整个单点段滤掉了 —— 在一个专做单点的
    // 项目里。这 21 个是那一段的全部。
    let want = [
        "SITE_fsitedata",
        "SITE_lon_location",
        "SITE_lat_location",
        "SITE_landtype",
        "USE_SITE_landtype",
        "USE_SITE_pctpfts",
        "USE_SITE_pctcrop",
        "USE_SITE_htop",
        "USE_SITE_LAI",
        "USE_SITE_lakedepth",
        "USE_SITE_soilreflectance",
        "USE_SITE_soilparameters",
        "USE_SITE_dbedrock",
        "USE_SITE_topography",
        "USE_SITE_urban_geometry",
        "USE_SITE_urban_ecology",
        "USE_SITE_urban_radiation",
        "USE_SITE_urban_thermal",
        "USE_SITE_urban_human",
        "USE_SITE_HistWriteBack",
        "USE_SITE_ForcingReadAhead",
    ];
    for n in want {
        let f = find(n).unwrap_or_else(|| panic!("{n} missing from the schema"));
        assert_eq!(f.group, Some("nl_colm"), "{n}");
    }
}

#[test]
fn the_three_aggregation_switches_are_in_the_table() {
    // 与单点段一样，只因为不叫 DEF_ 就被滤掉了。
    for n in [
        "USE_srfdata_from_3D_gridded_data",
        "USE_srfdata_from_larger_region",
        "USE_zip_for_aggregation",
    ] {
        assert_eq!(find(n).and_then(|f| f.group), Some("nl_colm"), "{n}");
    }
}

#[test]
fn a_field_nobody_can_set_is_marked_as_such() {
    // 这 6 个有声明、有默认值，但不在任何 namelist 组里。
    // DEF_dir_history 更进一步：MOD_Namelist.F90:1406 用 DEF_dir_output 与
    // DEF_CASE_NAME 无条件把它覆盖掉。GUI 给这种字段一个输入框就是在骗人。
    for n in [
        "DEF_dir_history",
        "DEF_dir_landdata",
        "DEF_dir_restart",
        "DEF_USE_IGBP",
        "DEF_USE_USGS",
        "DEF_Wetland_finundation_scheme",
    ] {
        let f = find(n).unwrap_or_else(|| panic!("{n} should still be in the table"));
        assert_eq!(f.group, None, "{n} is not settable from any namelist");
    }
}

#[test]
fn members_inherit_the_group_of_their_container() {
    // 这条决定 GUI 把一个字段写进哪个文件：强迫场字段进 nl_colm_forcing，
    // 输出变量开关进 nl_colm_history，其余进主 namelist。
    assert_eq!(
        find("DEF_forcing%dataset").unwrap().group,
        Some("nl_colm_forcing")
    );
    assert_eq!(
        find("DEF_hist_vars%xy_us").unwrap().group,
        Some("nl_colm_history")
    );
    assert_eq!(
        find("DEF_simulation_time%start_year").unwrap().group,
        Some("nl_colm")
    );
    assert_eq!(find("DEF_domain%edges").unwrap().group, Some("nl_colm"));
}

#[test]
fn the_macro_guarded_member_is_still_a_field() {
    // DEF_file_GIEMS 在 namelist 语句里被 #if (defined TRACER) && (defined BGC)
    // 包着。它是真字段，只是仅在那个预设下可设 —— 不能因为解析器看见
    // 一行 `#if` 就把它丢掉。
    assert_eq!(
        find("DEF_file_GIEMS").and_then(|f| f.group),
        Some("nl_colm")
    );
}

#[test]
fn a_use_statement_is_not_a_field() {
    // `USE, intrinsic :: ieee_arithmetic` 会被声明扫描器当成一个声明。
    // 原先靠 `DEF_` 白名单顺带挡住；改用 namelist 判据之后，它自然落选 ——
    // 因为它不在任何 namelist 组里。这条守住那个「顺带」不再是巧合。
    assert!(find("ieee_arithmetic").is_none());
}
```

- [ ] **Step 3: 跑，确认它们失败**

Run: `cargo test -p colm-schema --lib`
Expected: 上面 6 条全红（`group` 字段不存在 → 编译失败，或字段缺失 → panic）。
先让它编译不过是可以接受的红：本步的目的是确认测试确实在验新东西。

- [ ] **Step 4: 提交红状态**

```bash
git add crates/colm-schema/src/field.rs crates/colm-schema/src/field_tests.rs
git commit -m "Add failing tests for what the schema cannot yet describe"
```

单独提交红状态，与里程碑 5 的 Task 2 同一个理由：让「测试先于实现」这件事
在历史里看得见，而不是只在描述里声称。

---

## Task 2: `namelist` 语句作为权威判据

**Files:**
- Create: `xtask/src/namelist.rs`
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: 写扫描器**

新建 `xtask/src/namelist.rs`。**下面这段已在 scratch crate 里对真实
`MOD_Namelist.F90` 跑通**，输出 200 个名字、按组 197/2/1，与独立的 awk
测量完全吻合：

```rust
//! `namelist /group/ a, b, c &` 语句 —— CoLM 自己对「什么是可设字段」的定义。
//!
//! 原先按 `DEF_` 前缀白名单收顶层字段，那条判据是错的：`MOD_Namelist.F90`
//! 的 **Part 3: For Single Point** 整段用 `SITE_` / `USE_SITE_` 前缀，
//! 于是一个专做单点的项目，schema 恰好缺了单点那一节（21 个字段），
//! 另外还缺 3 个 `USE_srfdata_*` / `USE_zip_*`。
//!
//! 换成 namelist 判据还顺带解决了反方向的问题：有 6 个字段有声明有默认值
//! 但不在任何组里（用户改不了），以及 `ieee_arithmetic` 这种 `USE` 语句
//! 被误当成声明的情况。

use std::collections::BTreeMap;

/// 扫全文，得出「字段名 -> 它所属的 namelist 组名」。
///
/// 三处需要当心，都在真实文件里实测到：
/// 1. 续行里夹**空行**（`DEF_domain, &` 之后隔一行才是 `SITE_fsitedata`）——
///    按「上一行以 `&` 结尾就取下一行」会在这里断掉，只解析出 2 个成员；
/// 2. 续行符之后带**行尾注释**（`DEF_LAI_MONTHLY, & !add by ...`）；
/// 3. 有一个成员被宏包住（`DEF_file_GIEMS` 在 TRACER+BGC 下才可设）——
///    守卫行本身跳过，成员照收，因为它确实是字段。
pub fn groups(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut lines = text.lines();
    while let Some(raw) = lines.next() {
        let line = strip_comment(raw);
        let trimmed = line.trim_start();
        let low = trimmed.to_ascii_lowercase();
        let Some(rest) = low.strip_prefix("namelist /") else {
            continue;
        };
        let Some(slash) = rest.find('/') else {
            continue;
        };
        let group = rest[..slash].trim().to_string();

        let mut body = trimmed["namelist /".len() + slash + 1..].to_string();
        loop {
            let t = body.trim_end();
            if !t.ends_with('&') {
                break;
            }
            body = t.trim_end_matches('&').to_string();
            let mut next = None;
            for l in lines.by_ref() {
                let s = strip_comment(l);
                if s.trim().is_empty() || s.trim_start().starts_with('#') {
                    continue;
                }
                next = Some(s.to_string());
                break;
            }
            let Some(next) = next else { break };
            body.push(' ');
            body.push_str(&next);
        }
        for name in body.split(',') {
            let n = name.trim();
            if !n.is_empty() {
                out.insert(n.to_string(), group.clone());
            }
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    match line.find('!') {
        Some(p) => &line[..p],
        None => line,
    }
}
```

- [ ] **Step 2: 换掉 `main.rs` 的判据**

在 `main.rs` 顶部加 `mod namelist;`。

`extract` 的签名改成接收组表：

```rust
fn extract(text: &str, groups: &BTreeMap<String, String>) -> Result<Vec<Field>> {
```

把第 76-79 行那段：

```rust
        // 顶层只收 DEF_ 开头的；类型成员全收
        if owner.is_none() && !decl.name.starts_with("DEF_") {
            continue;
        }
```

换成：

```rust
        // 顶层字段的判据是「它出现在某个 namelist 语句里」，不是名字前缀。
        // 前缀白名单会滤掉整个 SITE_ / USE_SITE_ 单点段（21 个），
        // 也放不掉 6 个谁都设不了的字段。类型成员全收，组由容器继承。
        //
        // 大小写不敏感：Fortran 的 namelist 名字如此，且上游自己就混用
        // （DEF_hist_lat_res / DEF_HIST_lat_res 两种拼法都在库里）。
        let group = if owner.is_none() {
            let g = lookup_ci(groups, &decl.name);
            if g.is_none() && !decl.name.starts_with("DEF_") {
                continue; // 既不在 namelist 里，也不是 DEF_ —— 不是字段
            }
            g
        } else {
            None // 成员的组在 render 时由容器补上
        };
```

`Field` 结构加 `group: Option<String>`，`out.push` 里带上它。

新增查找辅助：

```rust
/// 大小写不敏感地查组表。
fn lookup_ci(groups: &BTreeMap<String, String>, name: &str) -> Option<String> {
    groups
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}
```

- [ ] **Step 3: 成员继承容器的组**

`render` 里，成员的组从 `owner_prefix` 得到的容器名再查一次组表。
把 `owner_prefix` 与组表一起传进 `render`：

```rust
fn render(fields: &[Field], groups: &BTreeMap<String, String>) -> String {
```

在写每一行之前算出 `group`：

```rust
        // 成员继承容器所在的组：DEF_forcing 在 nl_colm_forcing 里，
        // 所以 DEF_forcing%dataset 也该写进那个文件。这正是 GUI 要的信息。
        let group = match &f.owner {
            Some(o) => lookup_ci(groups, owner_prefix(o)),
            None => f.group.clone(),
        };
        let group = match &group {
            Some(g) => format!("Some({g:?})"),
            None => "None".to_string(),
        };
```

并把 `group: {group}` 加进 `Field {{ ... }}` 的渲染串里，放在 `line` 之前。

- [ ] **Step 4: `main` 里串起来**

```rust
    let groups = namelist::groups(&text);
    if groups.len() < 150 {
        bail!(
            "only {} namelist members found — the statement format must have changed",
            groups.len()
        );
    }
    let fields = extract(&text, &groups)?;
    let out = render(&fields, &groups);
```

那个下限守卫的理由与 `extract` 里「零字段即报错」一条相同：解析器悄悄
只认出 2 个成员是本计划实测踩过的坑（续行夹空行），必须炸而不是生成一张
少了 195 个字段的表。

- [ ] **Step 5: 重新生成并检查 diff**

```bash
cargo run -p xtask -- gen-schema
git diff --stat crates/colm-schema/src/generated.rs
```

Expected: `wrote 737 fields`（713 + 24）。diff 里每一行都多一个 `group:`，
另有 24 行新增。**逐个看一眼那 24 行**，确认全是 `SITE_*` / `USE_SITE_*` /
`USE_srfdata_*` / `USE_zip_*`，没有别的东西混进来。

- [ ] **Step 6: 更新计数断言**

`crates/colm-schema/src/field_tests.rs` 的 `the_table_has_the_measured_number_of_fields`：
总数那条是区间 `(700..=760)`，737 落在里面**不用改**；要改的是
`assert_eq!(top, 178)` → `202`，以及注释里的 178/713 → 202/737。

区间那条留着区间是对的：它防的是「生成器漏了一半」这类塌方，不是精确计数。
精确计数由本轮新增的那几条按名字断言的测试负责 —— 它们说得出**少了谁**。

- [ ] **Step 7: 门禁**

Run: `cargo test -p colm-schema --lib`
Expected: `test result: ok. 15 passed`（原 9 + 新 6）

Run: `cargo test -p colm-schema --test drift`
Expected: `1 passed` —— 重新生成必须与 Step 5 入库的产物逐字节一致。

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`

- [ ] **Step 8: 提交**

```bash
git add xtask crates/colm-schema
git commit -m "Let the namelist statement decide what counts as a field"
```

---

## Task 3: `colm-hist` crate 与闸门模型

**Files:**
- Create: `crates/colm-hist/Cargo.toml`
- Create: `crates/colm-hist/src/lib.rs`
- Create: `crates/colm-hist/src/lib_tests.rs`
- Modify: `Cargo.toml`（workspace members）

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "colm-hist"
version.workspace = true
edition.workspace = true
rust-version.workspace = true   # 必须显式 opt-in
license.workspace = true
publish.workspace = true

# 无依赖。本 crate 只是一张生成的表加一个求值器；
# 与黄金文件的经验校验放在 oracle 里，netcdf 不进这里。
[dependencies]

[lints]
workspace = true
```

- [ ] **Step 2: 写 `lib.rs`**

```rust
//! 「这个内核能产出哪些输出变量」—— 在开跑之前答得出来。
//!
//! `history_var_type` 有 482 个开关、343 个默认为真，而 waterheat 预设的一次
//! 真实运行只写出 119 个。差额不是 bug，是三道闸门：
//!
//! 1. **编译期宏** —— `MOD_Hist.F90` 里的 `#ifdef`。456 个写出点在 waterheat
//!    下剩 123 个。这道闸门由本 crate 回答，输入是内核清单里的 `macros`。
//! 2. **运行时 `DEF_*` 条件** —— 内联 `.and.` 与外层 `IF (DEF_*) THEN`。123 个里
//!    有 10 个带条件，本次运行 6 真 4 假，于是 113 + 6 = 119。本 crate 把条件
//!    原样记下来，由调用方结合算例配置求值。
//! 3. **变量自己的开关** `DEF_hist_vars%X` —— 在 `colm-schema` 里，默认全开。
//!
//! `qlayer` 与 `qcharge` 挂在同一个条件的两侧：CoLM 打印的第一条覆盖消息正是
//! 「`DEF_USE_VariablySaturatedFlow` 被自动设为 `.true.`」，于是有了 `qlayer`、
//! 没了 `qcharge`。**覆盖消息与变量的有无是同一件事的两面**，GUI 该连起来说。
//!
//! 表是生成的（`cargo run -p xtask -- gen-histmap`），产物入库，
//! `tests/drift.rs` 守住它不与上游脱节。

pub mod generated;

use std::collections::BTreeSet;

/// 一个编译期条件。
///
/// `MOD_Hist.F90` 里只出现四种形态：`#ifdef X`、`#ifndef X`、
/// `#if (defined X)`、`#if (defined A || defined B)`。**没有 `&&`，
/// 没有更深的嵌套**，所以不需要通用表达式求值器。
/// 生成器遇到不认识的形态会报错，不会静默当成真。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    /// 列出的宏里有任意一个被定义即成立。单个 `#ifdef` 也用这个变体。
    AnyOf(&'static [&'static str]),
    /// `#ifndef X`
    Not(&'static str),
}

impl Cond {
    pub fn holds(&self, macros: &BTreeSet<&str>) -> bool {
        match self {
            Cond::AnyOf(v) => v.iter().any(|m| macros.contains(m)),
            Cond::Not(m) => !macros.contains(m),
        }
    }
}

/// 一个输出变量的三道闸门。
#[derive(Debug, Clone, Copy)]
pub struct Var {
    /// NetCDF 里的变量名，去掉 `f_` 前缀。
    pub name: &'static str,
    /// 全部要同时成立的编译期条件。空表示无条件。
    pub macros: &'static [Cond],
    /// 运行时条件的**原文**，如 `DEF_USE_CBL_HEIGHT` 或
    /// `.not.DEF_USE_VariablySaturatedFlow`。`None` 表示没有。
    ///
    /// 刻意保留原文而不解析成表达式：这一层的职责是「如实报出 CoLM 写了什么
    /// 条件」，求值需要一份具体的算例配置，那是调用方的事。
    pub runtime: Option<&'static str>,
    /// `MOD_Hist.F90` 里的行号，便于回查。
    pub line: u32,
}

/// 全部变量，按名字排序。
pub fn all() -> &'static [Var] {
    generated::VARS
}

/// 给定宏集合，哪些变量**过得了第一道闸门**。
///
/// 注意这是「可能产出」，不是「一定产出」：运行时条件（闸门 2）与变量开关
/// （闸门 3）还会再减。实测 waterheat 下本函数返回 123，其中 10 个带运行时
/// 条件（`unconditional` 给出剩下的 113 个），实际写出 119。
///
/// 多报的方向是安全的 —— GUI 说「这个内核可能产出 X」而实际没有，
/// 比反过来漏掉一个真实产出要好。
pub fn writable(macros: &BTreeSet<&str>) -> BTreeSet<&'static str> {
    all()
        .iter()
        .filter(|v| v.macros.iter().all(|c| c.holds(macros)))
        .map(|v| v.name)
        .collect()
}

/// 过得了第一道闸门、且**没有**运行时条件的那些 —— 也就是「只要开关开着就一定有」。
pub fn unconditional(macros: &BTreeSet<&str>) -> BTreeSet<&'static str> {
    all()
        .iter()
        .filter(|v| v.runtime.is_none() && v.macros.iter().all(|c| c.holds(macros)))
        .map(|v| v.name)
        .collect()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
```

- [ ] **Step 3: 写 `lib_tests.rs`**

```rust
use super::*;

/// waterheat 预设的宏集合，取自 `kernels/waterheat/manifest.json`。
fn waterheat() -> BTreeSet<&'static str> {
    [
        "CoLMDEBUG",
        "LULC_IGBP",
        "RangeCheck",
        "SinglePoint",
        "extend_interception",
        "vanGenuchten_Mualem_SOIL_MODEL",
    ]
    .into_iter()
    .collect()
}

#[test]
fn the_table_covers_every_live_write_site() {
    // 456，不是直接 grep 得到的 466 —— 后者含 10 个整段被注释掉的写出点
    // （cwddecomp / cwdprod / 8 个 pd*）。它们永远产不出来，进表就是多报。
    assert_eq!(all().len(), 456);
    for dead in ["cwddecomp", "cwdprod", "pdcorn", "pdwwheat"] {
        assert!(
            all().iter().all(|v| v.name != dead),
            "{dead} is commented out in MOD_Hist.F90 and must not be in the table"
        );
    }
}

#[test]
fn the_waterheat_preset_can_write_one_hundred_and_twenty_three() {
    // 第一道闸门（编译期宏）之后剩 123 个，其中 113 个没有运行时条件。
    // 实际写出 119 = 113 + 那 10 个里条件成立的 6 个。
    assert_eq!(writable(&waterheat()).len(), 123);
    assert_eq!(unconditional(&waterheat()).len(), 113);
}

#[test]
fn every_runtime_gated_variable_carries_its_condition() {
    // 10 个过得了宏这一关但还挂着运行时条件。每个的条件原文都记在表里，
    // 所以 GUI 能说清「为什么你勾了它却没有」，而不是只说「没有」。
    let w = writable(&waterheat());
    let u = unconditional(&waterheat());
    let gated: Vec<&str> = w.difference(&u).cloned().collect();
    assert_eq!(
        gated,
        [
            "dz_lake",
            "lake_deficit",
            "o3uptakesha",
            "o3uptakesun",
            "qcharge",
            "qlayer",
            "t2m_wmo",
            "vegwp",
            "wetwat",
            "xy_hpbl",
        ]
    );

    let cond = |n: &str| all().iter().find(|v| v.name == n).unwrap().runtime.unwrap();
    // qlayer 与 qcharge 挂在同一个条件的两侧 —— 这道闸门是双向的，
    // 不是「条件成立才加」，而是「条件决定写哪一个」。
    // 而那个条件正是 CoLM 打印的第一条覆盖消息说的事：
    // `DEF_USE_VariablySaturatedFlow is automaticlly set to .true.`
    assert!(cond("qlayer").contains("DEF_USE_VariablySaturatedFlow"));
    assert!(cond("qcharge").contains("DEF_USE_VariablySaturatedFlow"));
    assert!(cond("qcharge").contains(".not."));
    assert!(cond("dz_lake").contains("DEF_USE_Dynamic_Lake"));
    assert!(cond("lake_deficit").contains(".not."));
    assert!(cond("t2m_wmo").contains("DEF_Output_2mWMO"));
    assert!(cond("xy_hpbl").contains("DEF_USE_CBL_HEIGHT"));
    assert!(cond("vegwp").contains("DEF_USE_PLANTHYDRAULICS"));
    assert!(cond("wetwat").contains("DEF_USE_Dynamic_Wetland"));
    for n in ["o3uptakesha", "o3uptakesun"] {
        assert!(cond(n).contains("DEF_USE_OZONESTRESS"));
    }
}

#[test]
fn turning_on_bgc_adds_variables_and_never_removes_any() {
    // 加一个宏不会让已有变量消失 —— 这个直觉只在该宏没有 #ifndef 侧时成立。
    // BGC 实测只有一处 `#ifdef BGC`、零处 `#ifndef BGC`，所以在它上面成立。
    // （CatchLateralFlow 两侧都有，就不成立 —— 见 ifndef_really_does_subtract。）
    let base = writable(&waterheat());
    let mut with_bgc = waterheat();
    with_bgc.insert("BGC");
    let more = writable(&with_bgc);
    assert!(base.is_subset(&more));
    assert_eq!(more.len(), 326); // 123 -> 326，BGC 那一块很大
}

#[test]
fn ifndef_really_does_subtract() {
    // 守住 Cond::Not 没有被当成恒真 —— 那样表会静默多报。
    //
    // 用 CatchLateralFlow 而**不是** SinglePoint 来验：实测 `#ifdef SinglePoint`
    // 的 13 处与 `#ifndef SinglePoint` 的 2 处区块里，`'f_*'` 字面量**一个都没有**
    // （那些块管的是文件命名与 IO 路径，不是变量写出），所以加减 SinglePoint
    // 对本表毫无影响，拿它做断言会得到一条永远为真的假测试。
    //
    // `#ifndef CatchLateralFlow` 则实实在在管着 f_rsur_ie 与 f_rsur_se ——
    // 两个都在黄金文件里（README 记着它们「两窗口恒为 0」）。
    let base = writable(&waterheat());
    assert!(base.contains("rsur_ie") && base.contains("rsur_se"));

    let mut with_catch = waterheat();
    with_catch.insert("CatchLateralFlow");
    let after = writable(&with_catch);
    assert!(!after.contains("rsur_ie"), "#ifndef must subtract");
    assert!(!after.contains("rsur_se"));
    // 同一个宏的 #ifdef 侧又放行了三个，所以净变化是 +1 而不是 -2。
    assert!(after.contains("fldarea") && after.contains("xwsub") && after.contains("xwsur"));
    assert_eq!(after.len(), 124);
}
```

- [ ] **Step 4: 占位 `generated.rs` 让它能编译**

```rust
//! 由 `cargo run -p xtask -- gen-histmap` 生成。**不要手改。**

use crate::{Cond, Var};

pub static VARS: &[Var] = &[];
```

- [ ] **Step 5: 加进 workspace**

根 `Cargo.toml` 的 `members` 里加 `"crates/colm-hist"`。

- [ ] **Step 6: 跑，确认测试红**

Run: `cargo test -p colm-hist`
Expected: 5 条全红（表是空的）。

- [ ] **Step 7: 提交红状态**

```bash
# Cargo.lock 是入库文件，加 workspace 成员会改写它 —— 不带上就会留一个脏工作区
git add crates/colm-hist Cargo.toml Cargo.lock
git commit -m "Add failing tests for which variables a preset can produce"
```

---

## Task 4: `xtask gen-histmap`

**Files:**
- Create: `xtask/src/hist.rs`
- Modify: `xtask/src/main.rs`

- [ ] **Step 1: 写提取器**

新建 `xtask/src/hist.rs`。**三道闸门一起提取已对真实源码预跑验证**
（表 456、waterheat 下宏放行 123 / 无条件 113、零漏报），包括本步的
`parse_cond` 与 Task 5 的运行时条件跟踪。

本步只写到 `parse_cond`，`main.rs` 里只加 `mod hist;`。此时没有调用方，
所以文件顶部要临时加 `#![allow(dead_code)]` 并注明「下一步接上」——
Task 5 接上 `gen-histmap` 分发之后删掉。`gen-histmap` 的分发臂放在 Task 5，
放在本步会得到一个无事可做的分支。

```rust
//! `MOD_Hist.F90` -> 每个输出变量的三道闸门。
//!
//! **不要按名字把开关和字面量配对。** 实测：`bedout` 的写出点是
//! `'f_bedout_'//...` 的拼接，`fsen_gimp` 的字面量是 `'f_fsengimp'`
//! （下划线位置不同），482 个开关里 50 个找不到同名字面量。
//! 正确做法是整体读取一个 `CALL write_history_variable_*` 调用 ——
//! 顺带也必须这么做，因为**闸门 2 的内联 `.and.` 就写在首参里**。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{bail, Result};

pub struct Var {
    pub name: String,
    pub macros: Vec<Cond>,
    pub runtime: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cond {
    AnyOf(Vec<String>),
    Not(String),
}

/// `#ifdef X` / `#ifndef X` / `#if (defined A || defined B)`。
///
/// 认不出来的形态**报错**，不静默当成真：静默当真会让表多报，
/// 而多报的变量在 GUI 里表现为「勾了却没有」，查起来毫无线索。
fn parse_cond(line: &str) -> Result<Option<Cond>> {
    let t = line.trim();
    if let Some(r) = t.strip_prefix("#ifdef ") {
        return Ok(Some(Cond::AnyOf(vec![r.trim().to_string()])));
    }
    if let Some(r) = t.strip_prefix("#ifndef ") {
        return Ok(Some(Cond::Not(r.trim().to_string())));
    }
    if let Some(r) = t.strip_prefix("#if ") {
        if r.contains("&&") {
            bail!("#if with && is not supported yet: {t}");
        }
        let names: Vec<String> = r
            .split("||")
            .filter_map(|p| {
                p.trim()
                    .trim_matches(|c| c == '(' || c == ')')
                    .trim()
                    .strip_prefix("defined")
                    .map(|n| {
                        n.trim()
                            .trim_matches(|c| c == '(' || c == ')')
                            .trim()
                            .to_string()
                    })
            })
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            bail!("cannot parse preprocessor condition: {t}");
        }
        return Ok(Some(Cond::AnyOf(names)));
    }
    Ok(None)
}
```

**其余部分（调用整体读取、外层 `IF` 跟踪、渲染）在 Task 5 的 Step 1 里，
因为它们要一起才跑得起来。** 本步只到 `parse_cond`，先让它单独可测。

- [ ] **Step 2: 提交**

```bash
git add xtask/src/hist.rs xtask/src/main.rs
git commit -m "Read the preprocessor conditions that gate each history variable"
```

---

## Task 5: 三道闸门一起提取，并让 `colm-hist` 的测试转绿

**Files:**
- Modify: `xtask/src/hist.rs`
- Modify: `crates/colm-hist/src/generated.rs`（生成）

- [ ] **Step 1: 整体读取调用，并跟踪外层 `IF`**

要处理的两种运行时条件形态（实测 2 + 21 处）：

```fortran
! 形态 A：内联在首参里
CALL write_history_variable_2d ( DEF_hist_vars%qcharge &
   .and. (.not.DEF_USE_VariablySaturatedFlow), &

! 形态 B：外层 IF；注意空格写法不统一，有 `IF(DEF_USE_DiagMatrix)THEN`
IF (DEF_USE_CBL_HEIGHT) THEN
  CALL write_history_variable_2d ( DEF_hist_vars%xy_hpbl, &
```

下面这段**已对真实 `MOD_Hist.F90` 预跑验证**，输出 456 / 123 / 113 与
那 10 条运行时条件。接到 Task 4 的 `parse_cond` 之后：

```rust
pub fn extract(text: &str) -> Result<Vec<Var>> {
    let mut out: BTreeMap<String, Var> = BTreeMap::new();
    let mut mstack: Vec<Option<Cond>> = Vec::new();
    let mut ifstack: Vec<Option<String>> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();

        if t.starts_with("#if") {
            mstack.push(parse_cond(t)?);
            i += 1;
            continue;
        }
        // #else 之后的分支条件是原条件的否定，而实测 MOD_Hist.F90 的 #else
        // 分支里没有写出调用。置 None（当作无条件）在这里是安全的，
        // 但若将来 #else 里出现了写出点，这就成了多报 —— 所以下面
        // Step 2 的核对数字是这条简化的看门人。
        if t.starts_with("#else") {
            if let Some(l) = mstack.last_mut() {
                *l = None;
            }
            i += 1;
            continue;
        }
        if t.starts_with("#endif") {
            mstack.pop();
            i += 1;
            continue;
        }

        // 块形式的 IF ... THEN。单行 IF 没有配对的 ENDIF，不能进栈。
        let low = t.to_ascii_lowercase();
        if low.starts_with("if") && low.replace(' ', "").ends_with(")then") {
            ifstack.push(runtime_if(t));
            i += 1;
            continue;
        }
        if low == "endif" || low == "end if" {
            ifstack.pop();
            i += 1;
            continue;
        }

        if t.contains("CALL write_history_variable") {
            let start = i;
            let mut depth = 0i32;
            let mut buf = String::new();
            while i < lines.len() {
                let l = strip_comment(lines[i]);
                for ch in l.chars() {
                    match ch {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                }
                buf.push(' ');
                buf.push_str(l.trim());
                i += 1;
                if depth <= 0 && buf.contains('(') {
                    break;
                }
            }
            // 内联条件优先；没有的话用最近的一层带 DEF_ 的外层 IF。
            let rt = inline_runtime(&buf).or_else(|| ifstack.iter().flatten().next_back().cloned());
            for name in literals(&buf) {
                let macros: Vec<Cond> = mstack.iter().flatten().cloned().collect();
                out.entry(name.clone()).or_insert(Var {
                    name,
                    macros,
                    runtime: rt.clone(),
                    line: (start + 1) as u32,
                });
            }
            continue;
        }
        i += 1;
    }
    if out.len() < 400 {
        bail!("only {} write sites found — the call format must have changed", out.len());
    }
    Ok(out.into_values().collect())
}

/// 外层 `IF (...) THEN` 中含 `DEF_` 的条件原文；其余返回 `None`。
///
/// 实测空格写法不统一：`IF (DEF_X) THEN` 与 `IF(DEF_X)THEN` 都有。
/// 只认含 `DEF_` 的，`IF (allocated(...)) THEN` 之类不算运行时闸门。
fn runtime_if(t: &str) -> Option<String> {
    let open = t.find('(')?;
    let close = t.rfind(')')?;
    let inner = t[open + 1..close].trim();
    inner.contains("DEF_").then(|| inner.to_string())
}

/// 首参 `DEF_hist_vars%X .and. <条件>` 里 `.and.` 之后到首个顶层逗号。
fn inline_runtime(call: &str) -> Option<String> {
    let p = call.find("DEF_hist_vars%")?;
    let rest = &call[p..];
    let a = rest.to_ascii_lowercase().find(".and.")?;
    let after = &rest[a + 5..];
    let mut depth = 0i32;
    for (k, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth <= 0 => return Some(after[..k].trim().to_string()),
            _ => {}
        }
    }
    None
}

/// 剥掉行尾注释。**必须跳过引号内的 `!`** —— 写出调用里带 long_name 字符串，
/// 里面出现感叹号就会把半行吃掉。
fn strip_comment(l: &str) -> &str {
    let mut quoted = false;
    for (k, c) in l.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '!' if !quoted => return &l[..k],
            _ => {}
        }
    }
    l
}

/// 取 `'f_…'` 字面量的名字部分。
///
/// 必须在 `strip_comment` 之后、且只在调用内部调用它：实测直接对全文
/// grep 会多出 10 个**被注释掉**的写出点（cwddecomp / cwdprod / 8 个 pd*），
/// 那些变量永远产不出来，进表就是多报。
///
/// 不需要为拼接写出做特殊处理：456 个写出点里以下划线结尾的（即
/// `'f_bedout_'//trim(x)` 那种前缀）一个都没有 —— 拼接都在本轮不扫的
/// 别的文件里（见「明确不做」）。将来若扫到了，以 `_` 结尾的名字要单独
/// 处理，因为它的真实变量名到运行时才成形。
fn literals(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = s[i..].find('\'') {
        let st = i + p + 1;
        let Some(e) = s[st..].find('\'') else { break };
        let lit = &s[st..st + e];
        if let Some(n) = lit.strip_prefix("f_") {
            if !n.is_empty() && lit.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                out.push(n.to_string());
            }
        }
        i = st + e + 1;
    }
    out
}
```

渲染成 `generated.rs` 的部分照 `colm-schema` 的 `render` 写：`Cond::AnyOf`
的宏名列表要渲染成 `&[&str]` 静态切片，`runtime` 是 `Option<&str>`。
**按 `name` 排序输出**，否则 `BTreeMap` 换成别的容器时 drift 测试会假红。

**产物要带 `#[rustfmt::skip]`。** 一个变量一行是刻意的 —— 上游改一处，
diff 就只有一行。但 rustfmt 会把每条拆成六行（456 条 → 近三千行），
那样 code review 里根本看不出改了什么。`colm-schema` 的同类产物不用写这条，
但那是**巧合不是设计**：它有一条 626 字符、断不开的数组默认值，
rustfmt 因此整块放弃；`colm-hist` 最长行只有 149 字符，没有这个豁免。
把这个理由写进生成的文件头，否则下一个人会以为这条属性是多余的。

- [ ] **Step 2: 生成并核对**

```bash
cargo run -p xtask -- gen-histmap
cargo test -p colm-hist
```

Expected: `wrote 456 variables`，5 条测试全绿。

若 `the_waterheat_preset_can_write_one_hundred_and_twenty_three` 给出的不是
123，**先查是不是新形态的宏条件被静默吞掉了**，而不是改断言迁就实现。

- [ ] **Step 3: drift 测试**

`crates/colm-hist/tests/drift.rs`，照抄 `colm-schema/tests/drift.rs` 的结构：
重新生成一次，与入库产物逐字节比较。

- [ ] **Step 4: 提交**

```bash
git add xtask crates/colm-hist
git commit -m "Answer which variables a preset can produce before it runs"
```

---

## Task 6: 经验闸门 —— 用黄金文件钉住这张表

**这一步是本计划的验收核心。** 静态映射再漂亮，也必须被一次真实运行钉住。

**Files:**
- Create: `oracle/fixtures/waterheat_hist_vars.txt`
- Create: `oracle/tests/histmap.rs`
- Modify: `oracle/Cargo.toml`（加 `colm-hist` 依赖）

- [ ] **Step 1: 从黄金文件生成 fixture**

```bash
ncdump -h oracle/golden/CN-Cng_hist_2008-01.nc \
  | grep -oE "^	(double|float|int) f_[a-z_0-9]+" \
  | awk '{print substr($2,3)}' | sort > oracle/fixtures/waterheat_hist_vars.txt
wc -l < oracle/fixtures/waterheat_hist_vars.txt   # 期望 119
```

- [ ] **Step 2: 写测试**

```rust
//! 生成的闸门表必须与一次真实运行对得上。
//!
//! 黄金文件（2.7 MB）入库，所以这条测试在 CI 的三个平台都能跑，
//! 不需要 PLUMBER2 数据也不需要 gfortran。

use std::collections::BTreeSet;

const WATERHEAT: [&str; 6] = [
    "CoLMDEBUG",
    "LULC_IGBP",
    "RangeCheck",
    "SinglePoint",
    "extend_interception",
    "vanGenuchten_Mualem_SOIL_MODEL",
];

fn golden_vars() -> BTreeSet<String> {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/waterheat_hist_vars.txt");
    std::fs::read_to_string(&p)
        .expect("the fixture must exist")
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn the_fixture_still_matches_the_golden_file() {
    // fixture 是从黄金文件抄出来的，这条守住它没有跑偏。
    // 用 netcdf 直接读，而不是相信当初那次 ncdump。
    let f = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("golden/CN-Cng_hist_2008-01.nc");
    let nc = netcdf::open(&f).expect("golden file opens");
    // v.name() 返回 String（不是 &str），所以先绑定再切前缀，
    // 免得在链式调用里对临时值取借用。
    let actual: BTreeSet<String> = nc
        .variables()
        .filter_map(|v| {
            let n = v.name();
            n.strip_prefix("f_").map(str::to_string)
        })
        .collect();
    assert_eq!(actual, golden_vars());
    assert_eq!(actual.len(), 119);
}

#[test]
fn the_static_map_never_misses_a_variable_that_was_actually_written() {
    // **零漏报是硬要求。** 多报（宏放行但运行时条件挡住）是可以接受的，
    // 且那 4 个已被逐一定位；漏报意味着 GUI 会告诉用户「这个内核产不出 X」
    // 而它其实产得出 —— 那是在用一张表去否定一次真实运行。
    let macros: BTreeSet<&str> = WATERHEAT.into_iter().collect();
    let predicted = colm_hist::writable(&macros);
    let missed: Vec<&String> = golden_vars()
        .iter()
        .filter(|v| !predicted.contains(v.as_str()))
        .collect();
    assert!(missed.is_empty(), "the map missed {missed:?}");
}

#[test]
fn the_only_over_prediction_is_the_four_runtime_gated_ones() {
    // 多报的必须**恰好**是那 4 个有运行时条件的。多出别的，说明宏闸门
    // 有一处判错了，而这条测试会指名道姓。
    let macros: BTreeSet<&str> = WATERHEAT.into_iter().collect();
    let golden = golden_vars();
    let over: Vec<&str> = colm_hist::writable(&macros)
        .into_iter()
        .filter(|v| !golden.contains(*v))
        .collect();
    assert_eq!(over, ["dz_lake", "qcharge", "t2m_wmo", "xy_hpbl"]);
}

#[test]
fn the_second_window_agrees_with_the_first() {
    // 同一个预设、不同的模拟窗口，输出变量集必须相同 —— 变量集取决于
    // 预设与配置，不取决于季节。两个黄金文件都在库里，比一下不花钱。
    let mut sets = Vec::new();
    for name in [
        "golden/CN-Cng_hist_2008-01.nc",
        "golden/CN-Cng-wet_hist_2008-07.nc",
    ] {
        let f = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name);
        let nc = netcdf::open(&f).expect("golden file opens");
        sets.push(
            nc.variables()
                .filter_map(|v| {
                    let n = v.name();
                    n.strip_prefix("f_").map(str::to_string)
                })
                .collect::<BTreeSet<String>>(),
        );
    }
    assert_eq!(sets[0], sets[1]);
}
```

- [ ] **Step 3: CI 里点名这条**

`.github/workflows/ci.yml` 的 per-PR `rust` job 里，在三条集成测试之后加：

```yaml
          cargo test -p oracle --test histmap
          cargo test -p colm-hist --test drift
```

理由与已有那三条相同：它们只需要源码与入库的黄金文件，应当在三平台都跑，
而不是落进只有自托管 runner 才跑的那一档。

- [ ] **Step 4: 门禁**

Run: `cargo test --workspace`
Expected: 150 + 6（schema）+ 5（colm-hist）+ 4（histmap）+ 1（drift）= **166 passed**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`

- [ ] **Step 5: 提交**

```bash
git add oracle .github/workflows/ci.yml
git commit -m "Pin the generated gate table to a real run"
```

---

## Task 7: 文档收尾

**Files:**
- Modify: `README.md`
- Modify: `docs/design.md`

- [ ] **Step 1: README**

「配置层」一节补上：字段的判据是 namelist 语句而非前缀，`group` 告诉调用方
这个字段该写进哪个文件，6 个不可设字段被标出来。

新增「输出变量」一节：三道闸门，以及「覆盖消息与缺失变量是同一件事」
那个 `qcharge` 的例子。

测试数字 125/140 → 重新测量后更新。

- [ ] **Step 2: design.md**

§5.3「输出」补三道闸门；§7 里 GUI 的输出变量页要说明它渲染的是
`colm_hist::writable(manifest.macros)` 而不是 482 个开关。

- [ ] **Step 3: 全量验证与提交**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
git diff --check
```

---

## 完成判据

- [ ] `SITE_*` / `USE_SITE_*` 21 个字段全部在 schema 里，`group` 为 `nl_colm`
- [ ] 6 个不可设字段仍在表里，但 `group` 为 `None`
- [ ] `DEF_forcing%dataset` 的 group 是 `nl_colm_forcing`，`DEF_hist_vars%*` 是 `nl_colm_history`
- [ ] `ieee_arithmetic` 不在表里，且**不是靠前缀挡住的**
- [ ] `DEF_file_GIEMS` 在表里（宏守卫行不能把成员一起吞掉）
- [ ] `gen-schema` 在 namelist 成员数异常偏少时**报错**而不是生成一张残表
- [ ] `colm_hist::writable(waterheat)` = 123
- [ ] 对黄金文件的 119 个变量**零漏报**
- [ ] 多报的恰好是 `dz_lake` / `qcharge` / `t2m_wmo` / `xy_hpbl` 四个，且每个的运行时条件原文都记在表里
- [ ] 两个黄金窗口的变量集相同
- [ ] 两张生成表都有 drift 测试，且都在 per-PR 的三平台 job 里
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all --check` 无输出

---

## 明确不做

- **GUI** —— 归 `plan-m7`。
- **运行时条件的求值** —— 本轮只把条件原文记下来。求值需要一份具体的算例
  配置，而那是 GUI 命令层的事；现在做等于在没有调用方的情况下猜接口。
- **`MOD_HistSingle.F90` / `MOD_HistWriteBack.F90` 里的写出点** —— 实测
  `MOD_Hist.F90` 一个文件就覆盖了黄金文件的全部 119 个变量，零漏报。
  等到有变量漏掉时再扩，不要提前。
- **闸门 3（`DEF_hist_vars%X` 开关）的联动** —— 它已经在 `colm-schema` 里，
  两张表在 GUI 层合并即可，不需要在生成期耦合。
