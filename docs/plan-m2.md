# 里程碑 2 实施计划：colm-namelist 与 colm-schema

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Rust 能够**保真地读写 CoLM 的 namelist**，并且知道每个 `DEF_*` 字段的类型、默认值与说明——这两件事是 GUI 配置界面、算例管理与强迫场生成的共同前提。

**Architecture:** 两个 crate。`colm-namelist` 是一个**保留格式的**解析器：解析 → 修改 → 序列化必须逐字节还原原文（注释、空行、对齐一并保住），因为用户的算例文件里的注释是他们自己的笔记，工具无权丢弃。`colm-schema` 不手写字段表，而是**从 `MOD_Namelist.F90` 代码生成**，产物入库并由测试守住漂移——CoLM 会持续演进，手写的 schema 必然静默失配。

**Tech Stack:** Rust 2021、无第三方解析库（手写扫描器）、`insta` 之外不引入快照框架；生成器是一个 `xtask` 二进制。

---

## 本计划的位置

| 计划 | 里程碑 | 产出 | 状态 |
|---|---|---|---|
| `plan-m0-m1.md` | 0–1 | 仓库骨架、成败判定、黄金回归基准 | ✅ 已完成 |
| **本文档** | **2** | **`colm-namelist` + `colm-schema`** | 待执行 |
| 后续 | 3–4 | `colm-srfdata` + `colm-forcing` | |
| 后续 | 5–7 | `colm-kernel` 编排 / `colm-hist` / `colm-cli` | |
| 后续 | 8 | GUI | |

**为什么先做这两个**：`colm-forcing` 要写出 forcing namelist，`colm-cli` 要读写算例 namelist，GUI 要按 schema 渲染配置界面。三者都依赖本里程碑。

---

## 实测出来的表面（本计划的全部数字都来自这里）

在 `vendor/CoLM202X` 上实测，不是估计。

### namelist 侧：55 个文件 / 4167 行 / 最长 354 行

总体定义：`vendor/CoLM202X/run/**/*.nml`，即 submodule 里**被追踪**的那些。
本机上 `run/aas_riverlake/` 还有 11 个 `.nml`，但 `.gitignore` 排除了整个目录
（它们是 `gen_experiments.sh` 的生成物），别的机器上并不存在。早先的
「66 个文件 / 6740 行」正是把它们算了进去 —— 重测时若又数到 66，先看
`git ls-files '*.nml' | wc -l`。

**必须支持的语法**：

| 特性 | 出现 | 实例 |
|---|---|---|
| 派生类型成员 `x%y =` | 50/55 | `DEF_forcing%dataset = 'POINT'` |
| 下标赋值 `v(1) =` | 24/55 | `DEF_forcing%fprefix(1) = '...'` |
| 空格分隔多字符串 | 26/55 | `vname = 'Tair' 'Qair' 'Psurf'` |
| 逗号分隔多值 | 9/55 | `= "wevap,winfilt,rivout"` |
| 整行注释 | 54/55 | `! ----- forcing -----` |
| 行尾注释 | 54/55 | `= 8        ! variable number` |
| `/` 单独成行结束 group | 55/55 | |

**确认不存在，因而不必支持**（若日后出现，解析器应当明确报错而不是猜）：

| 特性 | 出现 |
|---|---|
| 重复计数 `3*0.0` | 0/55 |
| 数组切片 `v(1:3) =` | 0/55 |
| 续行符 `&` | 0/55 |
| 裸值续行 | 0/55 |

**group 名**共 17 种，但只有 3 种属于 CoLM 本体且在本里程碑范围内：
`nl_colm`（20 个文件）、`nl_colm_forcing`（24）、`nl_colm_history`（1）。
其余 10 种（`NRUNVER` `NDIMTIME` `NPARAM` `NSIMTIME` `NMAP` `NRESTART` `NFORCE`
`NOUTPUT` `NDAMOUT` `NAMSED`）是 CaMa-Flood 的，SinglePoint 下 CaMa 被强制关闭；
`nl_colm_tracer_*` / `nl_colm_methane_parameter` / `nl_colm_sediment_parameter`
属 TRACER，本轮明确搁置。**解析器不区分 group 名**（它只认语法），但往返测试
覆盖全部 55 个文件，包括范围外的那些——语法是共通的，多覆盖不花钱。

### schema 侧：745 行声明，生成器收录其中 713 个字段

| 项 | 数量 |
|---|---|
| 全文件形如声明的行 | 745 |
| 生成器实际收录 | **713**（声明区内，见下方陷阱） |
| 顶层 `DEF_*` 标量 | 178，**全部带默认值** |
| 派生类型 | 4 个，共 535 个成员 |
| └ `history_var_type` | 482 |
| └ `nl_forcing_type` | 34 |
| └ `nl_simulation_time_type` | 15 |
| └ `nl_domain_type` | 4 |
| 带行尾注释（可作字段说明） | 108/713（15%） |

**默认值的形态**（生成器要认全这些）：

| 形态 | 数量 | 例 |
|---|---|---|
| logical | 589 | `.true.` |
| 单引号字符串 | 56 | `'MONTHLY'` |
| 整数 | 53 | `8` |
| 实数 | 25 | `1800.` `-1.e36` |
| 单行数组字面量 | 3 | `(/ 1, 2, 3 /)` |
| **跨行数组字面量** | **4** | `(/ &` 换行续到 `/)` |
| 双引号字符串 | 3 | `"H2_18O,HDO"` |

**另一件已知但本轮不处理的事**：178 个顶层 `DEF_` 里有 2 个被 CPP 条件包着 ——
`DEF_file_GIEMS` 与 `DEF_wetland_finundation_scheme`，守卫是
`#if (defined TRACER) && (defined BGC)`（`MOD_Namelist.F90:445-449`）。生成器扫的是
**文本**而不是预处理结果，所以它们照样进表。这在本里程碑无害（TRACER 本轮搁置，
两个字段在我们构建的任何配置里都不存在），但 GUI 里程碑必须知道：把一个当前构建
没声明的字段写进 namelist，CoLM 会以 `Cannot match namelist object name` 失败 ——
那正是 `colm-kernel` 已登记的失败标记之一。届时的解法是给 `Field` 记一个可选的
守卫表达式，而不是在这里提前造。

**一个必须处理的陷阱**：全文件有 8 个声明不含 `=`（7 个不同名字，
`set_defaults` 出现两次），它们**不是 namelist 字段**，而是子程序内的
局部变量与哑元：

```
character(len=*), intent(in) :: nlfile        ! 1137
logical :: fexists                            ! 1140
integer :: ivar                               ! 1141
integer :: ierr                               ! 1142
character(len=256) :: iomesg                  ! 1143
logical, intent(in) :: set_defaults           ! 2227
logical, intent(inout) :: onoff               ! 2741
logical, intent(in)    :: set_defaults        ! 2742
```

**生成器必须只扫描模块的声明区与 `type ... end type` 块，遇到 `SUBROUTINE`
就停止**。第一个 `SUBROUTINE` 在第 1132 行，上面这些全在它之后，而 178 个
`DEF_` 声明全在它之前——实测它之后 `DEF_` 出现 0 次，所以这条截断是安全的。
带 `intent(...)` 属性是哑元的可靠特征，但 `fexists`/`ivar`/`ierr`/`iomesg`
没有，所以靠属性过滤不够——必须靠作用域。

---

## 文件结构

```
crates/
├── colm-namelist/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          仅 pub mod {value,parse,document}; 与 crate 级文档
│       ├── value.rs        Value 与 Path（字段寻址），无 I/O
│       ├── parse.rs        扫描器：文本 -> Document
│       ├── document.rs     Document：保留原文的项列表，get/set/to_string
│       └── parse_tests.rs  #[cfg(test)] #[path] 挂进 parse.rs
├── colm-schema/
│   ├── Cargo.toml
│   ├── build-notes.md      生成器怎么跑、产物为什么入库
│   └── src/
│       ├── lib.rs          pub mod {field,generated}; 与查询函数
│       ├── field.rs        Field/FieldKind/Default 的定义，手写
│       └── generated.rs    **代码生成产物，入库，禁止手改**
└── xtask/
    ├── Cargo.toml
    └── src/main.rs         gen-schema 子命令：读 MOD_Namelist.F90 -> 写 generated.rs

crates/colm-namelist/tests/roundtrip.rs   对 55 个真实 .nml 的往返测试
crates/colm-schema/tests/drift.rs         重新生成必须与入库产物一致
```

**边界**：`colm-namelist` 完全不知道 `DEF_*` 是什么，它只认 namelist 语法；
`colm-schema` 完全不解析 namelist 文件，它只描述字段。两者在 `colm-cli`
（后续里程碑）里才相遇。这样拆是为了让往返测试和漂移测试各自独立成立。

---

## Task 1: 两个 crate 的骨架

**Files:**
- Create: `crates/colm-namelist/Cargo.toml`
- Create: `crates/colm-namelist/src/lib.rs`
- Create: `crates/colm-schema/Cargo.toml`
- Create: `crates/colm-schema/src/lib.rs`
- Modify: `Cargo.toml`（workspace members）

- [ ] **Step 1: 写 `crates/colm-namelist/Cargo.toml`**

```toml
[package]
name = "colm-namelist"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
anyhow.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: 写 `crates/colm-namelist/src/lib.rs`**

```rust
//! Fortran namelist 的读写，**保留原文格式**。
//!
//! 解析 → 修改 → 序列化必须逐字节还原未改动的部分：注释、空行、缩进、
//! 等号对齐都要保住。理由不是审美 —— 用户算例文件里的注释是他们自己的
//! 研究笔记，一个工具没有权力在保存时把它们丢掉。
//!
//! 语法支持范围是**实测**出来的，不是照抄 Fortran 标准：对 55 个真实
//! `.nml` 统计后，派生类型成员（50/55）、下标赋值（24/55）、空格分隔
//! 多字符串（26/55）、行尾注释（54/55）必须支持；而重复计数 `3*0.0`、
//! 数组切片 `v(1:3)=`、续行符 `&` 在 55 个文件里出现 0 次，因此不支持，
//! 遇到时明确报错而不是猜。
//!
//! 类型与函数的重导出在 Task 5（namelist）与 Task 8（schema）里加上，
//! 那时它们指向的东西才存在。

pub mod document;
pub mod parse;
pub mod value;
```

- [ ] **Step 3: 写 `crates/colm-schema/Cargo.toml`**

```toml
[package]
name = "colm-schema"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]

[lints]
workspace = true
```

- [ ] **Step 4: 写 `crates/colm-schema/src/lib.rs`**

```rust
//! CoLM `DEF_*` 配置字段的元数据：类型、默认值、所属 group、说明。
//!
//! **本 crate 的字段表是代码生成的，不是手写的。** 生成器是
//! `xtask gen-schema`，输入是 `vendor/CoLM202X/share/MOD_Namelist.F90`，
//! 产物 `generated.rs` 入库，并由 `tests/drift.rs` 守住：重新生成必须
//! 与入库产物逐字节一致。
//!
//! 这样做的理由是 CoLM 会持续演进。手写的字段表在上游加一个 `DEF_` 之后
//! 不会报错，只会静默地少一项 —— 而 GUI 依赖这张表决定渲染什么，
//! 少一项意味着用户永远看不到那个选项。
//!
//! `all()` / `find()` 与重导出在 Task 8 里加上，那时字段表才存在。

pub mod field;
pub mod generated;
```

- [ ] **Step 5: 建占位模块文件**

`lib.rs` 声明了模块，文件就必须存在。这五个各写一行，内容会在后续 Task 被整体替换：

`crates/colm-namelist/src/{value,parse,document}.rs`：

```rust
//! 占位，Task 2/3/4 实现。
```

`crates/colm-schema/src/{field,generated}.rs`：

```rust
//! 占位，Task 7/8 实现。
```

- [ ] **Step 6: 把两个 crate 加入 workspace**

在根 `Cargo.toml` 把 members 改成：

```toml
members = ["crates/colm-kernel", "crates/colm-namelist", "crates/colm-schema", "oracle"]
```

- [ ] **Step 7: 三道门禁都必须过**

Run: `cargo build`
Expected: 编译通过，无警告。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 无输出。

Run: `cargo fmt --all --check`
Expected: 无输出。

Run: `cargo test --workspace 2>&1 | grep 'test result'`
Expected: 里程碑 0–1 的 21 个测试仍全绿（11 个 colm-kernel + 10 个 oracle judge）。
本 Task 不应触碰它们。

- [ ] **Step 8: 提交**

```bash
git add Cargo.toml crates/colm-namelist crates/colm-schema
git commit -m "Add colm-namelist and colm-schema crate skeletons"
```

---

## Task 2: Value 与 Path —— 先写失败的测试

**Files:**
- Create: `crates/colm-namelist/src/value_tests.rs`
- Modify: `crates/colm-namelist/src/value.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;

#[test]
fn path_parses_a_plain_field() {
    let p = Path::parse("DEF_CASE_NAME").unwrap();
    assert_eq!(p.segments, vec![Segment::Field("DEF_CASE_NAME".into())]);
    assert_eq!(p.to_string(), "DEF_CASE_NAME");
}

#[test]
fn path_parses_a_derived_type_member() {
    // 50/55 个真实文件里有这种写法
    let p = Path::parse("DEF_forcing%dataset").unwrap();
    assert_eq!(
        p.segments,
        vec![
            Segment::Field("DEF_forcing".into()),
            Segment::Member("dataset".into())
        ]
    );
    assert_eq!(p.to_string(), "DEF_forcing%dataset");
}

#[test]
fn path_parses_a_subscript() {
    // 24/55 个真实文件里有这种写法，且正是 forcing namelist 必需的
    let p = Path::parse("DEF_forcing%fprefix(1)").unwrap();
    assert_eq!(
        p.segments,
        vec![
            Segment::Field("DEF_forcing".into()),
            Segment::Member("fprefix".into()),
            Segment::Index(1)
        ]
    );
    assert_eq!(p.to_string(), "DEF_forcing%fprefix(1)");
}

#[test]
fn path_rejects_a_slice_rather_than_guessing() {
    // 数组切片在 55 个文件里出现 0 次。不支持，且必须明确报错 ——
    // 猜一个语义比拒绝更危险。
    let e = Path::parse("DEF_x(1:3)").unwrap_err();
    assert!(format!("{e:#}").contains("slice"), "{e:#}");
}

#[test]
fn values_render_in_fortran_form() {
    assert_eq!(Value::Bool(true).to_string(), ".true.");
    assert_eq!(Value::Bool(false).to_string(), ".false.");
    assert_eq!(Value::Int(-8).to_string(), "-8");
    assert_eq!(Value::Str("POINT".into()).to_string(), "'POINT'");
}

#[test]
fn real_keeps_the_exact_text_it_was_read_from() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，但往返必须还原原样，
    // 否则每次保存都会把用户的写法改掉，diff 里全是噪声。
    let v = Value::Real {
        text: "1800.".into(),
    };
    assert_eq!(v.to_string(), "1800.");
    assert_eq!(v.as_f64(), Some(1800.0));
}

#[test]
fn a_list_renders_space_separated_like_the_files_do() {
    // 26/55 个文件用空格分隔多字符串：vname = 'Tair' 'Qair'
    let v = Value::List(vec![Value::Str("Tair".into()), Value::Str("Qair".into())]);
    assert_eq!(v.to_string(), "'Tair' 'Qair'");
}
```

- [ ] **Step 2: 建空壳让测试编译失败**

`crates/colm-namelist/src/value.rs`：

```rust
//! namelist 的值与字段寻址。

#[cfg(test)]
#[path = "value_tests.rs"]
mod value_tests;
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p colm-namelist`
Expected: 编译失败，报 `cannot find type 'Path'`、`cannot find type 'Value'`、
`cannot find type 'Segment'`。这就是要的 RED 状态。

- [ ] **Step 4: 提交失败的测试**

```bash
git add crates/colm-namelist/src/value.rs crates/colm-namelist/src/value_tests.rs
git commit -m "Add failing tests for namelist values and field paths"
```

---

## Task 3: Value 与 Path —— 实现

**Files:**
- Modify: `crates/colm-namelist/src/value.rs`

- [ ] **Step 1: 写实现**

```rust
//! namelist 的值与字段寻址。
//!
//! `Value::Real` 保存**原始文本**而不是 `f64`：`1800.` `1800.0` `1.8e3`
//! 在 Fortran 里等价，但往返必须还原用户写的那一种，否则每次保存都会
//! 改写用户的文件，让 diff 里全是与改动无关的噪声。

use std::fmt;

use anyhow::{bail, Result};

/// 字段路径的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// 顶层名字，如 `DEF_CASE_NAME`
    Field(String),
    /// 派生类型成员，如 `%dataset`
    Member(String),
    /// 下标，如 `(1)`。Fortran 下标从 1 起，这里原样保存不做换算。
    Index(usize),
}

/// 一个字段的完整路径，如 `DEF_forcing%fprefix(1)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub segments: Vec<Segment>,
}

impl Path {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            bail!("empty field path");
        }
        if s.contains(':') {
            bail!("array slice syntax is not supported: {s}");
        }
        let mut segments = Vec::new();
        for (i, part) in s.split('%').enumerate() {
            let (name, index) = match part.find('(') {
                Some(p) => {
                    if !part.ends_with(')') {
                        bail!("unclosed subscript in {s}");
                    }
                    let inner = &part[p + 1..part.len() - 1];
                    let n: usize = inner
                        .trim()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("bad subscript {inner:?} in {s}"))?;
                    (&part[..p], Some(n))
                }
                None => (part, None),
            };
            let name = name.trim();
            if name.is_empty() {
                bail!("empty path segment in {s}");
            }
            segments.push(if i == 0 {
                Segment::Field(name.to_string())
            } else {
                Segment::Member(name.to_string())
            });
            if let Some(n) = index {
                segments.push(Segment::Index(n));
            }
        }
        Ok(Self { segments })
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for seg in &self.segments {
            match seg {
                Segment::Field(n) => write!(f, "{n}")?,
                Segment::Member(n) => write!(f, "%{n}")?,
                Segment::Index(n) => write!(f, "({n})")?,
            }
        }
        Ok(())
    }
}

/// 一个 namelist 值。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    /// 保留原始文本，见模块文档。
    Real {
        text: String,
    },
    Str(String),
    /// 空格或逗号分隔的多值。分隔符由 `Document` 在序列化时按原文还原。
    List(Vec<Value>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            // Fortran 用 d 表示双精度指数，Rust 不认，换成 e
            Value::Real { text } => text.replace(['d', 'D'], "e").parse().ok(),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(true) => write!(f, ".true."),
            Value::Bool(false) => write!(f, ".false."),
            Value::Int(i) => write!(f, "{i}"),
            Value::Real { text } => write!(f, "{text}"),
            Value::Str(s) => write!(f, "'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "{}", parts.join(" "))
            }
        }
    }
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod value_tests;
```

- [ ] **Step 2: 测试通过**

Run: `cargo test -p colm-namelist`
Expected: `test result: ok. 7 passed; 0 failed`

- [ ] **Step 3: 格式与 lint**

Run: `cargo fmt --all --check && cargo clippy -p colm-namelist --all-targets -- -D warnings`
Expected: 两条都无输出。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-namelist/src/value.rs
git commit -m "Represent namelist values keeping the text reals were written as"
```

---

## Task 4: 解析器 —— 先写失败的测试

**Files:**
- Create: `crates/colm-namelist/src/parse_tests.rs`
- Modify: `crates/colm-namelist/src/parse.rs`
- Modify: `crates/colm-namelist/src/document.rs`

- [ ] **Step 1: 写测试**

每条测试的输入都取自真实文件的写法。

```rust
use super::*;
use crate::value::Value;

fn doc(src: &str) -> crate::Document {
    parse(src).expect("should parse")
}

#[test]
fn round_trips_a_minimal_group() {
    let src = "&nl_colm\n   DEF_CASE_NAME = 'x'\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn preserves_blank_lines_and_full_line_comments() {
    // 54/55 个真实文件有整行注释，它们是用户的笔记
    let src = "&nl_colm\n\n   ! ----- forcing -----\n   DEF_CASE_NAME = 'x'\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn preserves_trailing_comments_and_their_column() {
    // 54/55 个文件有行尾注释，且是对齐的
    let src = "&nl_colm\n   DEF_forcing%NVAR              = 8        ! variable number\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn reads_a_derived_type_member() {
    let d = doc("&nl_colm_forcing\n   DEF_forcing%dataset = 'POINT'\n/\n");
    let v = d.get("DEF_forcing%dataset").expect("field present");
    assert_eq!(v, &Value::Str("POINT".into()));
}

#[test]
fn reads_a_subscripted_entry() {
    let d = doc("&nl_colm_forcing\n   DEF_forcing%fprefix(1) = 'a.nc'\n/\n");
    let v = d.get("DEF_forcing%fprefix(1)").expect("field present");
    assert_eq!(v, &Value::Str("a.nc".into()));
}

#[test]
fn reads_space_separated_strings_as_a_list() {
    // 26/55 个文件这样写 vname / tintalgo
    let d = doc("&nl_colm_forcing\n   DEF_forcing%vname = 'Tair' 'Qair' 'NULL'\n/\n");
    match d.get("DEF_forcing%vname").expect("field present") {
        Value::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[2], Value::Str("NULL".into()));
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn reads_logicals_case_insensitively() {
    // 只管**读**成什么；写回时保持原写法由
    // keeps_the_case_a_logical_was_written_in 负责
    let d = doc("&nl_colm\n   a = .TRUE.\n   b = .false.\n/\n");
    assert_eq!(d.get("a"), Some(&Value::Bool(true)));
    assert_eq!(d.get("b"), Some(&Value::Bool(false)));
}

#[test]
fn keeps_the_text_of_reals() {
    // 真实文件里 1800. 与 50. 都是这种写法
    let d = doc("&nl_colm\n   t = 1800.\n/\n");
    assert_eq!(
        d.get("t"),
        Some(&Value::Real {
            text: "1800.".into()
        })
    );
}

#[test]
fn setting_a_value_leaves_everything_else_byte_identical() {
    // 这条是本 crate 存在的理由：改一个字段，其余原文一字不动
    let src = "&nl_colm\n\n   ! 注释\n   DEF_CASE_NAME = 'old'   ! 尾注\n   other = 1\n/\n";
    let mut d = doc(src);
    d.set("DEF_CASE_NAME", Value::Str("new".into())).unwrap();
    let out = d.to_string();
    assert!(out.contains("'new'"), "{out}");
    assert!(out.contains("! 注释"), "full-line comment lost:\n{out}");
    assert!(out.contains("! 尾注"), "trailing comment lost:\n{out}");
    assert_eq!(out.lines().count(), src.lines().count());
    assert_eq!(out.replace("'new'", "'old'"), src);
}

#[test]
fn setting_an_absent_field_is_an_error_not_a_silent_append() {
    // 静默追加会让用户以为改动生效了，而 CoLM 读到的却是另一回事
    let mut d = doc("&nl_colm\n   a = 1\n/\n");
    let e = d.set("DEF_nope", Value::Int(1)).unwrap_err();
    assert!(format!("{e:#}").contains("DEF_nope"), "{e:#}");
}

#[test]
fn keeps_the_case_a_logical_was_written_in() {
    // 真实文件里 .TRUE. 大写形式有 198 处。若按 Value::Bool 重新渲染，
    // 每一处都会变成 .true. —— 用户没改的行不该出现在 diff 里。
    let src = "&nl_colm\n   a = .TRUE.\n   b = .false.\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn accepts_a_logical_written_without_its_trailing_dot() {
    // cama_flood_10km.nml 与 cama_flood_US_30km.nml 里真的这么写，
    // 而同目录的 cama_flood.nml 写的是 .FALSE. —— 两种都要能读，
    // 且都要原样写回。
    let src = "&NOUTPUT\n   LOUTVEC  = .FALSE\n/\n";
    let d = doc(src);
    assert_eq!(d.get("LOUTVEC"), Some(&Value::Bool(false)));
    assert_eq!(d.to_string(), src);
}

#[test]
fn keeps_the_double_quotes_the_file_used() {
    // 156 处，集中在 CaMa 与 TRACER 的 namelist。Value::Str 只会写单引号。
    let src = "&NMAP\n   CDIMINFO = \"../CaMa/map/glb.txt\"\n/\n";
    let d = doc(src);
    assert_eq!(
        d.get("CDIMINFO"),
        Some(&Value::Str("../CaMa/map/glb.txt".into()))
    );
    assert_eq!(d.to_string(), src);
}

#[test]
fn keeps_comma_separators_in_a_list() {
    // 15 处。Value::List 只会用空格连接。
    let src = "&nl_colm\n   v = 'precip', 'vapor'\n/\n";
    let d = doc(src);
    match d.get("v").expect("field present") {
        Value::List(items) => assert_eq!(items.len(), 2),
        other => panic!("expected a list, got {other:?}"),
    }
    assert_eq!(d.to_string(), src);
}

#[test]
fn keeps_tabs_between_the_value_and_its_comment() {
    // 5 个 CaMa 文件用制表符对齐行尾注释
    let src = "&NSIMTIME\n   EYEAR   = 2024   \t\t!  end year\n/\n";
    assert_eq!(doc(src).to_string(), src);
}

#[test]
fn only_the_changed_field_is_rewritten_in_canonical_form() {
    // 这条画出分界：保留原文不等于不能改值。被 set 过的行按 Value 的
    // 规范形式重写，没被 set 的同写法的行仍然一字不动。
    let src = "&nl_colm\n   a = .TRUE.\n   b = .TRUE.\n/\n";
    let mut d = doc(src);
    d.set("a", Value::Bool(false)).unwrap();
    assert_eq!(
        d.to_string(),
        "&nl_colm\n   a = .false.\n   b = .TRUE.\n/\n"
    );
}

#[test]
fn rejects_repeat_count_rather_than_guessing() {
    // 0/55 个文件用它。不支持，且必须报错。
    let e = parse("&nl_colm\n   a = 3*0.0\n/\n").unwrap_err();
    // 注意用 {:#}：anyhow 的 Display 只给最外层 context，
    // 而 "repeat counts are not supported" 是被 with_context 包在里面的原因。
    assert!(format!("{e:#}").contains("repeat"), "{e:#}");
}

#[test]
fn rejects_a_group_that_is_never_closed() {
    let e = parse("&nl_colm\n   a = 1\n").unwrap_err();
    assert!(format!("{e:#}").contains("unterminated"), "{e:#}");
}
```

- [ ] **Step 2: 建空壳**

`crates/colm-namelist/src/parse.rs`：

```rust
//! 把 namelist 文本扫描成 `Document`。

#[cfg(test)]
#[path = "parse_tests.rs"]
mod parse_tests;
```

`crates/colm-namelist/src/document.rs`：

```rust
//! 保留原文的 namelist 文档模型。
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p colm-namelist 2>&1 | tail -20`
Expected: 编译失败，报找不到 `parse`、`Document`、`get`、`set`。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-namelist/src/parse.rs crates/colm-namelist/src/parse_tests.rs crates/colm-namelist/src/document.rs
git commit -m "Add failing tests for the format-preserving namelist parser"
```

---

## Task 5: Document 与解析器 —— 实现

**Files:**
- Modify: `crates/colm-namelist/src/document.rs`
- Modify: `crates/colm-namelist/src/parse.rs`

- [ ] **Step 1: 写 `document.rs`**

```rust
//! 保留原文的 namelist 文档模型。
//!
//! 文档是一个**按行的项列表**。每个项都记着它的原始文本，序列化时
//! 未被修改的项原样吐回，被修改的项只替换值那一段 —— 缩进、等号位置、
//! 行尾注释都从原始文本里切出来复用。
//!
//! 这样做而不是「解析成结构再重新排版」，是因为重新排版必然改写用户
//! 没有动过的行，让保存后的 diff 淹没在无关噪声里。
//!
//! 关键是 `Entry` 连**值本身的原文**也保留，而不只是缩进与注释。理由是
//! 同一个值在 Fortran 里有多种等价写法，而 `Value` 只能渲染出其中一种。
//! 实测 55 个真实文件里：`.TRUE.` 大写形式 198 处（`Value::Bool` 渲染成
//! `.true.`）、双引号字符串 156 处（渲染成单引号）、逗号分隔多值 15 处
//! （渲染成空格分隔）—— 合计约 369 行会在「读进来再写回去」时被改写。
//!
//! 于是分界是：**没被 `set` 过的行逐字节不动；被 `set` 过的行才按 `Value`
//! 的规范形式重写。** 用户改了哪一行，diff 里就只出现哪一行。

use anyhow::{bail, Result};

use crate::value::{Path, Value};

/// 文档里的一行。
#[derive(Debug, Clone)]
pub enum Item {
    /// 空行或整行注释，原样保存
    Verbatim(String),
    /// `&group_name`
    GroupStart(String),
    /// 单独成行的 `/`
    GroupEnd(String),
    /// 一个赋值
    Entry(Entry),
}

/// 一个 `name = value ! comment` 行。
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: Path,
    pub value: Value,
    /// 值那一段的**原文**。`set` 会用新值的渲染结果覆盖它；
    /// 没被改过就原样吐回，见模块文档。
    pub text: String,
    /// 从行首到 `=` 之后的那一段原文（含缩进与对齐空格）
    pub prefix: String,
    /// 值之后到行尾的原文（含空格与行尾注释）
    pub suffix: String,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub items: Vec<Item>,
}

impl Document {
    /// 按路径取值。路径写法与文件里一致，如 `DEF_forcing%fprefix(1)`。
    pub fn get(&self, path: &str) -> Option<&Value> {
        let want = Path::parse(path).ok()?;
        self.items.iter().find_map(|i| match i {
            Item::Entry(e) if e.path == want => Some(&e.value),
            _ => None,
        })
    }

    /// 就地改值。**字段不存在时报错，不追加** —— 静默追加会让调用方
    /// 以为改动生效，而 CoLM 读到的是另一回事。
    pub fn set(&mut self, path: &str, value: Value) -> Result<()> {
        let want = Path::parse(path)?;
        for item in &mut self.items {
            if let Item::Entry(e) = item {
                if e.path == want {
                    e.text = value.to_string();
                    e.value = value;
                    return Ok(());
                }
            }
        }
        bail!("no such field in this namelist: {path}")
    }

    /// 列出全部字段路径，按出现顺序。
    pub fn paths(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Entry(e) => Some(e.path.to_string()),
                _ => None,
            })
            .collect()
    }
}

impl std::fmt::Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for item in &self.items {
            match item {
                Item::Verbatim(s) | Item::GroupStart(s) | Item::GroupEnd(s) => writeln!(f, "{s}")?,
                Item::Entry(e) => writeln!(f, "{}{}{}", e.prefix, e.text, e.suffix)?,
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 2: 写 `parse.rs`**

```rust
//! 把 namelist 文本扫描成 `Document`。
//!
//! 逐行扫描即可：实测 55 个真实文件里**续行符 `&` 出现 0 次**，
//! 所以不需要处理跨行的赋值。若日后出现，本模块会在遇到行尾 `&` 时报错，
//! 而不是悄悄把它当成普通字符。

use anyhow::{bail, Context, Result};

use crate::document::{Document, Entry, Item};
use crate::value::{Path, Value};

pub fn parse(src: &str) -> Result<Document> {
    let mut items = Vec::new();
    let mut in_group = false;

    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let t = line.trim();

        if t.is_empty() || t.starts_with('!') {
            items.push(Item::Verbatim(line.to_string()));
            continue;
        }
        if t.starts_with('&') {
            if in_group {
                bail!("line {}: group opened inside a group", lineno + 1);
            }
            in_group = true;
            items.push(Item::GroupStart(line.to_string()));
            continue;
        }
        if t == "/" {
            in_group = false;
            items.push(Item::GroupEnd(line.to_string()));
            continue;
        }
        if line.trim_end().ends_with('&') {
            bail!("line {}: continuation lines are not supported", lineno + 1);
        }

        let eq = line
            .find('=')
            .with_context(|| format!("line {}: expected an assignment: {line}", lineno + 1))?;
        let name = &line[..eq];
        let path = Path::parse(name.trim())
            .with_context(|| format!("line {}: bad field name", lineno + 1))?;

        // 值与行尾注释：`!` 在引号外才是注释起点
        let rest = &line[eq + 1..];
        let cut = comment_start(rest).unwrap_or(rest.len());
        let head = &rest[..cut];
        let lead = head.len() - head.trim_start().len();
        let text = head.trim();

        let value =
            parse_value(text).with_context(|| format!("line {}: bad value: {text}", lineno + 1))?;

        // prefix + text + suffix 必须逐字节等于原行，这是往返的全部依据。
        items.push(Item::Entry(Entry {
            path,
            value,
            text: text.to_string(),
            prefix: format!("{}={}", &line[..eq], &rest[..lead]),
            suffix: rest[lead + text.len()..].to_string(),
        }));
    }

    if in_group {
        bail!("unterminated group: the file ends without a closing '/'");
    }
    Ok(Document { items })
}

/// 引号外第一个 `!` 的位置。
fn comment_start(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '\'') | (None, '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '!') => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_value(s: &str) -> Result<Value> {
    if s.is_empty() {
        bail!("empty value");
    }
    let items = split_values(s)?;
    if items.len() == 1 {
        parse_scalar(&items[0])
    } else {
        Ok(Value::List(
            items
                .iter()
                .map(|x| parse_scalar(x))
                .collect::<Result<_>>()?,
        ))
    }
}

/// 按空格或逗号切分，引号内不切。
fn split_values(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (None, '\'') | (None, '"') => {
                quote = Some(c);
                cur.push(c);
            }
            (Some(q), c2) if c2 == q => {
                quote = None;
                cur.push(c2);
            }
            (None, ' ') | (None, '\t') | (None, ',') => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if quote.is_some() {
        bail!("unterminated string");
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        bail!("empty value");
    }
    Ok(out)
}

/// Fortran 的逻辑值输入比 `.true.` 宽松：前导点与结尾点都可以省。
///
/// 这不是理论上的宽容 —— 实测 `LOUTVEC = .FALSE`（无结尾点）出现在
/// cama_flood_10km.nml 与 cama_flood_US_30km.nml 里，而同目录的
/// cama_flood.nml 写的是 `.FALSE.`。上游自己就不一致，gfortran 两种
/// 都读成假，所以两种都得接受，否则这两个文件根本解析不了。
///
/// 但只放宽到这里：`.TRUEISH` 之类仍然拒绝，宁可报错也不猜。
fn parse_logical(s: &str) -> Option<bool> {
    let t = s.strip_prefix('.').unwrap_or(s);
    let t = t.strip_suffix('.').unwrap_or(t);
    match t.to_ascii_lowercase().as_str() {
        "t" | "true" => Some(true),
        "f" | "false" => Some(false),
        _ => None,
    }
}

fn parse_scalar(s: &str) -> Result<Value> {
    if let Some(b) = parse_logical(s) {
        return Ok(Value::Bool(b));
    }
    let low = s.to_ascii_lowercase();
    if (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
        || (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
    {
        return Ok(Value::Str(s[1..s.len() - 1].to_string()));
    }
    if s.contains('*') {
        bail!("repeat counts are not supported: {s}");
    }
    if let Ok(i) = s.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    // 只要能当实数读出来就按实数存，但保留原始文本
    if low.replace(['d'], "e").parse::<f64>().is_ok() {
        return Ok(Value::Real {
            text: s.to_string(),
        });
    }
    bail!("unrecognised value: {s}")
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod parse_tests;
```

- [ ] **Step 3: 给 `crates/colm-namelist/src/lib.rs` 加上重导出**

在文件末尾追加（Task 1 只写了 `pub mod`）：

```rust
pub use document::Document;
pub use parse::parse;
pub use value::{Path, Segment, Value};
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-namelist`
Expected: `test result: ok. 25 passed; 0 failed`（7 个 value + 18 个 parse）

- [ ] **Step 5: 格式与 lint**

Run: `cargo fmt --all --check && cargo clippy -p colm-namelist --all-targets -- -D warnings`
Expected: 无输出。

- [ ] **Step 6: 提交**

```bash
git add crates/colm-namelist/src
git commit -m "Parse namelists into a model that can be written back unchanged"
```

---

## Task 6: 对 55 个真实文件的往返测试

这是本 crate 的真正验收：**能不能不动用户的文件**。

**Files:**
- Create: `crates/colm-namelist/tests/roundtrip.rs`

- [ ] **Step 1: 写测试**

```rust
//! 对 vendor/CoLM202X 里全部 55 个真实 .nml 做往返测试。
//!
//! 合成用例能证明语法被支持，只有真实文件能证明**用户的文件不会被改动**。
//! 55 个文件共 4167 行，最长的 354 行；覆盖 17 种 group 名，
//! 包括 CaMa-Flood 与 TRACER 那些本里程碑范围外的 —— 语法是共通的，
//! 多覆盖不花钱，而少覆盖会让"范围外"的文件在将来某天被悄悄改坏。

use std::path::{Path, PathBuf};

fn nml_files() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run")
        .canonicalize()
        .expect("vendor/CoLM202X/run must exist; run git submodule update --init");
    let mut out = Vec::new();
    collect(&root, &mut out);
    out.sort();
    assert!(
        out.len() >= 50,
        "expected ~55 namelists, found {}",
        out.len()
    );
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("readable dir") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "nml") {
            out.push(p);
        }
    }
}

#[test]
fn every_real_namelist_round_trips_byte_for_byte() {
    let mut failures = Vec::new();
    let files = nml_files();
    for f in &files {
        let src = std::fs::read_to_string(f).expect("readable file");
        match colm_namelist::parse(&src) {
            Ok(doc) => {
                let out = doc.to_string();
                if out != src {
                    let at = first_difference(&src, &out);
                    failures.push(format!("{}: differs at line {at}", f.display()));
                }
            }
            Err(e) => failures.push(format!("{}: parse failed: {e}", f.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} namelists did not round-trip:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn every_real_namelist_yields_at_least_one_field() {
    // 一个「什么都没解析出来但也没报错」的解析器会让往返测试全绿而毫无意义
    let mut empty = Vec::new();
    for f in nml_files() {
        let src = std::fs::read_to_string(&f).expect("readable file");
        let doc = colm_namelist::parse(&src).expect("parses");
        if doc.paths().is_empty() {
            empty.push(f.display().to_string());
        }
    }
    assert!(empty.is_empty(), "these parsed to zero fields:\n{empty:#?}");
}

#[test]
fn changing_one_field_changes_exactly_one_line() {
    // 拿一个真实的 forcing namelist：它同时含派生类型成员、下标赋值、
    // 空格分隔多字符串与行尾注释，是最能暴露格式丢失的样本
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run/forcing/POINT.nml");
    let src = std::fs::read_to_string(&f).expect("POINT.nml must exist");
    let mut doc = colm_namelist::parse(&src).expect("parses");
    doc.set(
        "DEF_forcing%dataset",
        colm_namelist::Value::Str("CHANGED".into()),
    )
    .expect("field exists");
    let out = doc.to_string();

    let differing: Vec<_> = src
        .lines()
        .zip(out.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(
        differing.len(),
        1,
        "expected one changed line, got {differing:?}"
    );
    assert_eq!(src.lines().count(), out.lines().count());
}

fn first_difference(a: &str, b: &str) -> usize {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return i + 1;
        }
    }
    a.lines().count().min(b.lines().count()) + 1
}
```

- [ ] **Step 2: 跑，并如实报告失败的文件**

Run: `cargo test -p colm-namelist --test roundtrip 2>&1 | tail -40`

写这个计划时已经把这三条测试对着真实语料跑过一遍，全绿——**所以全绿是预期结果，
不是可疑结果**。Task 4/5 里那几条关于 `.TRUE.` 大小写、双引号、逗号分隔、制表符、
以及无结尾点的 `.FALSE` 的测试，正是那次预跑发现后补进去的。

万一仍有文件不通过，**逐个看它是什么语法**，然后：
- 若是真实存在的语法（如某个文件用了双引号字符串），**在解析器里支持它**，
  并在 `parse_tests.rs` 补一条对应的单元测试；
- 若是本计划开头列为「不存在」的语法（重复计数、切片、续行），
  说明测量有误——**先重新测量，再决定支持还是继续拒绝**，不要直接放宽。

**不要**用跳过文件的方式让测试变绿。跳过一个文件就是承认工具会改坏它。

- [ ] **Step 3: 全绿后提交**

Run: `cargo test -p colm-namelist`
Expected: 全部通过，且 roundtrip 那三条都在列表里。

```bash
git add crates/colm-namelist
git commit -m "Round-trip all 55 real namelists byte for byte"
```

---

## Task 7: schema 的数据类型

**Files:**
- Modify: `crates/colm-schema/src/field.rs`
- Create: `crates/colm-schema/src/field_tests.rs`

- [ ] **Step 1: 写 `field.rs`**

```rust
//! 一个配置字段的元数据。手写；字段表本身是生成的。

/// 字段的存储类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Logical,
    Integer,
    Real,
    /// Fortran 的 `character(len=N)`，N 一并记下来：GUI 要用它限制输入长度
    Character {
        len: usize,
    },
}

/// 字段的默认值，保留 Fortran 原文。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Default {
    Logical(bool),
    Integer(i64),
    /// 原始文本，如 `"1800."`
    Real(&'static str),
    Str(&'static str),
    /// 数组字面量的原文，如 `"(/ 'a','b' /)"`
    Array(&'static str),
}

/// 一个 `DEF_*` 字段。
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// 全名，如 `DEF_forcing%dataset`
    pub name: &'static str,
    pub kind: FieldKind,
    pub default: Default,
    /// 声明处 `=` 之后的行尾注释，可作为 GUI 的字段说明。713 个字段里 108 个有。
    pub doc: Option<&'static str>,
    /// 数组长度，如 `fprefix(8)` 是 `Some(8)`
    pub arity: Option<usize>,
    /// 所属派生类型名；顶层字段为 `None`
    pub owner: Option<&'static str>,
    /// `MOD_Namelist.F90` 中的行号，便于回查
    pub line: u32,
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod field_tests;
```

- [ ] **Step 2: 写 `field_tests.rs`**

这些测试针对**生成出来的表**，所以它们同时是生成器的验收。

```rust
use crate::{all, find, Default, FieldKind};

#[test]
fn the_table_has_the_measured_number_of_fields() {
    // 实测：178 个顶层 DEF_ 标量 + 4 个派生类型共 535 个成员，合计 713。
    // 若这个数变了，要么上游改了，要么生成器漏了 —— 两种都必须有人看一眼。
    let total = all().len();
    assert!(
        (700..=760).contains(&total),
        "expected roughly 713 fields, got {total}"
    );
    let top = all().iter().filter(|f| f.owner.is_none()).count();
    assert_eq!(top, 178, "top-level DEF_ count changed");
}

#[test]
fn a_known_scalar_is_described_correctly() {
    let f = find("DEF_CASE_NAME").expect("DEF_CASE_NAME must be in the schema");
    assert!(matches!(f.kind, FieldKind::Character { .. }));
    assert!(f.owner.is_none());
}

#[test]
fn a_derived_type_member_carries_its_owner() {
    let f = find("DEF_forcing%dataset").expect("must be in the schema");
    assert_eq!(f.owner, Some("nl_forcing_type"));
}

#[test]
fn an_array_field_records_its_arity() {
    // fprefix(8) —— GUI 要知道它有 8 槽，且第 5 槽在 POINT 下是 'NULL'
    let f = find("DEF_forcing%fprefix").expect("must be in the schema");
    assert_eq!(f.arity, Some(8));
}

#[test]
fn defaults_that_differ_from_colm_are_visible_here() {
    // 这两个默认值正是「GUI 的默认值必须与 CoLM 的默认值不同」的原因：
    // 见 design.md §2.5。schema 必须如实记录 CoLM 的原值，
    // 偏离由上层决定并解释，而不是在这里偷偷改掉。
    assert_eq!(
        find("DEF_USE_OZONEDATA").map(|f| f.default),
        Some(Default::Logical(true))
    );
    assert_eq!(
        find("DEF_Runoff_SCHEME").map(|f| f.default),
        Some(Default::Integer(3))
    );
}

#[test]
fn no_local_variable_leaked_into_the_schema() {
    // MOD_Namelist.F90 里有 8 个不含 '=' 的声明（7 个不同名字），
    // 它们是子程序局部变量与哑元
    // （nlfile / fexists / ivar / ierr / iomesg / set_defaults / onoff），
    // 不是配置字段。生成器必须靠作用域排除它们 —— 靠 intent(...) 属性过滤
    // 是不够的，因为 fexists / ivar / ierr / iomesg 都没有 intent。
    for leaked in [
        "nlfile",
        "fexists",
        "ivar",
        "ierr",
        "iomesg",
        "set_defaults",
        "onoff",
    ] {
        assert!(
            find(leaked).is_none(),
            "{leaked} is a subroutine local, not a config field"
        );
    }
}

#[test]
fn the_history_type_contributes_the_bulk_of_the_table() {
    let n = all()
        .iter()
        .filter(|f| f.owner == Some("history_var_type"))
        .count();
    assert_eq!(n, 482, "history_var_type member count changed");
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p colm-schema`
Expected: 编译失败。具体是 `use crate::{all, find, Default, FieldKind};` 四个名字
都解析不了 —— Task 1 写的 `lib.rs` 只有 `pub mod` 声明，重导出与 `all()`/`find()`
要到 Task 8 Step 4b 才加上，那时 `generated::FIELDS` 才存在。这是 RED 状态。

**不要**为了让它编译而提前给 `lib.rs` 加导出：那两行属于 Task 8，
且在字段表存在之前它们无处可指。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-schema/src
git commit -m "Add failing tests describing the generated schema table"
```

---

## Task 8: schema 生成器

**Files:**
- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`
- Create: `crates/colm-schema/build-notes.md`
- Modify: 根 `Cargo.toml`（加入 `xtask` member）

- [ ] **Step 1: 写 `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
anyhow.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: 写生成器**

```rust
//! 代码生成：把 `MOD_Namelist.F90` 的声明变成 `colm-schema` 的字段表。
//!
//! 用法: cargo run -p xtask -- gen-schema
//!
//! 产物 `crates/colm-schema/src/generated.rs` **入库**，由
//! `crates/colm-schema/tests/drift.rs` 守住：重新生成必须逐字节一致。
//! 入库而不是 build.rs 现生成，是为了让 schema 的变化出现在 code review 的
//! diff 里 —— 上游加一个 DEF_ 或改一个默认值，应当是一次可见的改动，
//! 而不是某次构建之后悄悄换掉的东西。

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    if cmd != "gen-schema" {
        bail!("usage: cargo run -p xtask -- gen-schema");
    }
    let root = repo_root()?;
    let src = root.join("vendor/CoLM202X/share/MOD_Namelist.F90");
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let fields = extract(&text)?;
    let out = render(&fields);
    let dst = root.join("crates/colm-schema/src/generated.rs");
    std::fs::write(&dst, out)?;
    println!("wrote {} fields to {}", fields.len(), dst.display());
    Ok(())
}

#[derive(Debug)]
struct Field {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
    owner: Option<String>,
    line: u32,
}

/// 扫描模块的声明区与 type 块，**遇到 SUBROUTINE / FUNCTION 即停止**。
///
/// 这条是必须的：文件里有 8 个不含 `=` 的声明（7 个不同名字：nlfile /
/// fexists / ivar / ierr / iomesg / set_defaults / onoff），全部是子程序
/// 局部变量与哑元。靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
fn extract(text: &str) -> Result<Vec<Field>> {
    let mut out = Vec::new();
    let mut owner: Option<String> = None;
    let mut lines = text.lines().enumerate().peekable();

    while let Some((i, raw)) = lines.next() {
        let line = raw.trim();
        let low = line.to_ascii_lowercase();

        if low.starts_with("subroutine ") || low.starts_with("function ") {
            break; // 声明区到此为止
        }
        if let Some(rest) = low.strip_prefix("type ") {
            let n = rest.trim_start_matches(":: ").trim();
            if !n.is_empty() && !n.contains('(') {
                owner = Some(n.to_string());
            }
            continue;
        }
        if low.starts_with("end type") {
            owner = None;
            continue;
        }

        let Some(decl) = parse_decl(line) else {
            continue;
        };
        // 顶层只收 DEF_ 开头的；类型成员全收
        if owner.is_none() && !decl.name.starts_with("DEF_") {
            continue;
        }

        // 跨行数组字面量：实测 4 处，形如 `= (/ &` 续到 `/)`
        let mut default = decl.default.clone();
        if default.trim_end().ends_with('&') {
            let mut acc = default
                .trim_end()
                .trim_end_matches('&')
                .trim_end()
                .to_string();
            for (_, more) in lines.by_ref() {
                let m = more.trim();
                acc.push(' ');
                acc.push_str(m.trim_end().trim_end_matches('&').trim_end());
                if m.contains("/)") {
                    break;
                }
            }
            default = acc;
        }

        out.push(Field {
            name: decl.name.clone(),
            kind: decl.kind,
            default: default.trim().to_string(),
            doc: decl.doc,
            arity: decl.arity,
            owner: owner.clone(),
            line: (i + 1) as u32,
        });
    }

    if out.is_empty() {
        bail!("extracted zero fields — the declaration format must have changed");
    }
    Ok(out)
}

struct Decl {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
}

fn parse_decl(line: &str) -> Option<Decl> {
    let (head, tail) = line.split_once("::")?;
    let head_low = head.to_ascii_lowercase();
    let kind = if head_low.starts_with("logical") {
        "FieldKind::Logical".to_string()
    } else if head_low.starts_with("integer") {
        "FieldKind::Integer".to_string()
    } else if head_low.starts_with("real") {
        "FieldKind::Real".to_string()
    } else if head_low.starts_with("character") {
        let len = head_low
            .split_once("len=")
            .and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(1);
        format!("FieldKind::Character {{ len: {len} }}")
    } else {
        return None;
    };
    // 哑元与局部变量：没有 `=` 的一律跳过（配置字段实测 100% 带默认值）
    let (lhs, rhs) = tail.split_once('=')?;
    let (rhs, doc) = match rhs.find('!') {
        Some(p) => (&rhs[..p], Some(rhs[p + 1..].trim().to_string())),
        None => (rhs, None),
    };
    let lhs = lhs.trim();
    let (name, arity) = match lhs.split_once('(') {
        Some((n, a)) => (
            n.trim().to_string(),
            a.trim_end_matches(')').trim().parse::<usize>().ok(),
        ),
        None => (lhs.to_string(), None),
    };
    Some(Decl {
        name,
        kind,
        default: rhs.to_string(),
        doc,
        arity,
    })
}

fn render(fields: &[Field]) -> String {
    let mut s = String::new();
    s.push_str(
        "//! 由 `cargo run -p xtask -- gen-schema` 生成。**不要手改。**\n\
         //!\n\
         //! 源：vendor/CoLM202X/share/MOD_Namelist.F90\n\
         //! 漂移由 crates/colm-schema/tests/drift.rs 守住。\n\n\
         use crate::field::{Default, Field, FieldKind};\n\n\
         pub static FIELDS: &[Field] = &[\n",
    );
    for f in fields {
        let full = match &f.owner {
            Some(o) => format!("{}%{}", owner_prefix(o), f.name),
            None => f.name.clone(),
        };
        let doc = match &f.doc {
            Some(d) => format!("Some({:?})", d),
            None => "None".to_string(),
        };
        let arity = match f.arity {
            Some(n) => format!("Some({n})"),
            None => "None".to_string(),
        };
        let owner = match &f.owner {
            Some(o) => format!("Some({o:?})"),
            None => "None".to_string(),
        };
        let _ = writeln!(
            s,
            "    Field {{ name: {full:?}, kind: {}, default: {}, doc: {doc}, arity: {arity}, owner: {owner}, line: {} }},",
            f.kind,
            render_default(&f.kind, &f.default),
            f.line
        );
    }
    s.push_str("];\n");
    s
}

/// 派生类型名 -> 它在 namelist 里的实例名。
///
/// 手工映射，因为 Fortran 的类型定义与变量声明是分开的，而 namelist 文件里
/// 出现的是变量名。四个类型全在这里，新增类型时生成器会报错提醒。
fn owner_prefix(type_name: &str) -> &'static str {
    match type_name {
        "nl_domain_type" => "DEF_domain",
        "nl_simulation_time_type" => "DEF_simulation_time",
        "nl_forcing_type" => "DEF_forcing",
        "history_var_type" => "DEF_hist_vars",
        other => panic!("unknown derived type {other}: add it to owner_prefix"),
    }
}

fn render_default(kind: &str, raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("(/") {
        return format!("Default::Array({t:?})");
    }
    if kind.starts_with("FieldKind::Logical") {
        return format!(
            "Default::Logical({})",
            t.to_ascii_lowercase().contains("true")
        );
    }
    if kind.starts_with("FieldKind::Integer") {
        return match t.parse::<i64>() {
            Ok(i) => format!("Default::Integer({i})"),
            Err(_) => format!("Default::Str({t:?})"),
        };
    }
    if kind.starts_with("FieldKind::Real") {
        return format!("Default::Real({t:?})");
    }
    let unquoted = t.trim_matches(|c| c == '\'' || c == '"');
    format!("Default::Str({unquoted:?})")
}

fn repo_root() -> Result<PathBuf> {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !d.join(".git").exists() {
        if !d.pop() {
            bail!("not inside a git repository");
        }
    }
    Ok(d)
}
```

- [ ] **Step 3: 把 xtask 加入 workspace 并生成**

根 `Cargo.toml` 的 members 加上 `"xtask"`。

Run: `cargo run -p xtask -- gen-schema`
Expected: 打印 `wrote NNN fields to .../generated.rs`，NNN 在 700–760 之间。

若报 `extracted zero fields`，说明声明格式与实测不符——**去看 `MOD_Namelist.F90`
的实际写法再改正则**，不要放宽到能匹配任何东西。

- [ ] **Step 4: 让 Task 7 的测试通过**

Run: `cargo test -p colm-schema`
Expected: 7 条全部通过。

若 `no_local_variable_leaked_into_the_schema` 失败，说明作用域截断没生效——
检查 `extract` 是否在第一个 `SUBROUTINE` 处 `break`。**不要**改成按名字
黑名单过滤那 6 个：黑名单挡不住下一个新增的局部变量。

- [ ] **Step 4b: 给 `crates/colm-schema/src/lib.rs` 加上查询接口**

Task 1 只写了 `pub mod`。现在字段表存在了，追加：

```rust
pub use field::{Default, Field, FieldKind};

/// 全部字段，按声明顺序。
pub fn all() -> &'static [Field] {
    generated::FIELDS
}

/// 按全名查找，例如 `"DEF_forcing%dataset"`。
pub fn find(name: &str) -> Option<&'static Field> {
    generated::FIELDS.iter().find(|f| f.name == name)
}
```

- [ ] **Step 5: 写 `build-notes.md`**

（外层用四个反引号，因为文件内容自身含一个 ```bash 块）

````markdown
# colm-schema 的字段表是怎么来的

`src/generated.rs` 由 `cargo run -p xtask -- gen-schema` 从
`vendor/CoLM202X/share/MOD_Namelist.F90` 生成，**产物入库**。

## 为什么入库而不是 build.rs 现生成

上游加一个 `DEF_*` 或改一个默认值，应当是一次**在 code review 里看得见**的改动。
build.rs 会让它在某次构建之后悄悄换掉，没有人经手。

## 怎么更新

```bash
git -C vendor/CoLM202X checkout <新的 commit>
cargo run -p xtask -- gen-schema
cargo test -p colm-schema      # drift 测试确认产物与源一致
git add vendor/CoLM202X crates/colm-schema/src/generated.rs
```

## 生成器必须守住的两条

1. **作用域截断**：只扫描模块声明区与 `type ... end type`，遇到第一个
   `SUBROUTINE`（第 1132 行）就停。它之后有 8 个不含 `=` 的声明是子程序局部
   变量与哑元（`nlfile` `fexists` `ivar` `ierr` `iomesg` `set_defaults` `onoff`），
   靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
2. **派生类型名到实例名的映射**在 `owner_prefix` 里手工维护。
   Fortran 的类型定义与变量声明是分开的，而 namelist 文件里出现的是变量名。
   遇到未知类型时生成器会 panic，这是有意的：宁可停下来让人补，
   也不要生成一张名字错误的表。
````

- [ ] **Step 6: 提交**

```bash
git add crates/colm-namelist/src
git commit -m "Parse namelists into a model that can be written back unchanged"
```

---

## Task 6: 对 55 个真实文件的往返测试

这是本 crate 的真正验收：**能不能不动用户的文件**。

**Files:**
- Create: `crates/colm-namelist/tests/roundtrip.rs`

- [ ] **Step 1: 写测试**

```rust
//! 对 vendor/CoLM202X 里全部 55 个真实 .nml 做往返测试。
//!
//! 合成用例能证明语法被支持，只有真实文件能证明**用户的文件不会被改动**。
//! 55 个文件共 4167 行，最长的 354 行；覆盖 17 种 group 名，
//! 包括 CaMa-Flood 与 TRACER 那些本里程碑范围外的 —— 语法是共通的，
//! 多覆盖不花钱，而少覆盖会让"范围外"的文件在将来某天被悄悄改坏。

use std::path::{Path, PathBuf};

fn nml_files() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run")
        .canonicalize()
        .expect("vendor/CoLM202X/run must exist; run git submodule update --init");
    let mut out = Vec::new();
    collect(&root, &mut out);
    out.sort();
    assert!(
        out.len() >= 50,
        "expected ~55 namelists, found {}",
        out.len()
    );
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("readable dir") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "nml") {
            out.push(p);
        }
    }
}

#[test]
fn every_real_namelist_round_trips_byte_for_byte() {
    let mut failures = Vec::new();
    let files = nml_files();
    for f in &files {
        let src = std::fs::read_to_string(f).expect("readable file");
        match colm_namelist::parse(&src) {
            Ok(doc) => {
                let out = doc.to_string();
                if out != src {
                    let at = first_difference(&src, &out);
                    failures.push(format!("{}: differs at line {at}", f.display()));
                }
            }
            Err(e) => failures.push(format!("{}: parse failed: {e}", f.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} namelists did not round-trip:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn every_real_namelist_yields_at_least_one_field() {
    // 一个「什么都没解析出来但也没报错」的解析器会让往返测试全绿而毫无意义
    let mut empty = Vec::new();
    for f in nml_files() {
        let src = std::fs::read_to_string(&f).expect("readable file");
        let doc = colm_namelist::parse(&src).expect("parses");
        if doc.paths().is_empty() {
            empty.push(f.display().to_string());
        }
    }
    assert!(empty.is_empty(), "these parsed to zero fields:\n{empty:#?}");
}

#[test]
fn changing_one_field_changes_exactly_one_line() {
    // 拿一个真实的 forcing namelist：它同时含派生类型成员、下标赋值、
    // 空格分隔多字符串与行尾注释，是最能暴露格式丢失的样本
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run/forcing/POINT.nml");
    let src = std::fs::read_to_string(&f).expect("POINT.nml must exist");
    let mut doc = colm_namelist::parse(&src).expect("parses");
    doc.set(
        "DEF_forcing%dataset",
        colm_namelist::Value::Str("CHANGED".into()),
    )
    .expect("field exists");
    let out = doc.to_string();

    let differing: Vec<_> = src
        .lines()
        .zip(out.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(
        differing.len(),
        1,
        "expected one changed line, got {differing:?}"
    );
    assert_eq!(src.lines().count(), out.lines().count());
}

fn first_difference(a: &str, b: &str) -> usize {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return i + 1;
        }
    }
    a.lines().count().min(b.lines().count()) + 1
}
```

- [ ] **Step 2: 跑，并如实报告失败的文件**

Run: `cargo test -p colm-namelist --test roundtrip 2>&1 | tail -40`

写这个计划时已经把这三条测试对着真实语料跑过一遍，全绿——**所以全绿是预期结果，
不是可疑结果**。Task 4/5 里那几条关于 `.TRUE.` 大小写、双引号、逗号分隔、制表符、
以及无结尾点的 `.FALSE` 的测试，正是那次预跑发现后补进去的。

万一仍有文件不通过，**逐个看它是什么语法**，然后：
- 若是真实存在的语法（如某个文件用了双引号字符串），**在解析器里支持它**，
  并在 `parse_tests.rs` 补一条对应的单元测试；
- 若是本计划开头列为「不存在」的语法（重复计数、切片、续行），
  说明测量有误——**先重新测量，再决定支持还是继续拒绝**，不要直接放宽。

**不要**用跳过文件的方式让测试变绿。跳过一个文件就是承认工具会改坏它。

- [ ] **Step 3: 全绿后提交**

Run: `cargo test -p colm-namelist`
Expected: 全部通过，且 roundtrip 那三条都在列表里。

```bash
git add crates/colm-namelist
git commit -m "Round-trip all 55 real namelists byte for byte"
```

---

## Task 7: schema 的数据类型

**Files:**
- Modify: `crates/colm-schema/src/field.rs`
- Create: `crates/colm-schema/src/field_tests.rs`

- [ ] **Step 1: 写 `field.rs`**

```rust
//! 一个配置字段的元数据。手写；字段表本身是生成的。

/// 字段的存储类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Logical,
    Integer,
    Real,
    /// Fortran 的 `character(len=N)`，N 一并记下来：GUI 要用它限制输入长度
    Character {
        len: usize,
    },
}

/// 字段的默认值，保留 Fortran 原文。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Default {
    Logical(bool),
    Integer(i64),
    /// 原始文本，如 `"1800."`
    Real(&'static str),
    Str(&'static str),
    /// 数组字面量的原文，如 `"(/ 'a','b' /)"`
    Array(&'static str),
}

/// 一个 `DEF_*` 字段。
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// 全名，如 `DEF_forcing%dataset`
    pub name: &'static str,
    pub kind: FieldKind,
    pub default: Default,
    /// 声明处 `=` 之后的行尾注释，可作为 GUI 的字段说明。713 个字段里 108 个有。
    pub doc: Option<&'static str>,
    /// 数组长度，如 `fprefix(8)` 是 `Some(8)`
    pub arity: Option<usize>,
    /// 所属派生类型名；顶层字段为 `None`
    pub owner: Option<&'static str>,
    /// `MOD_Namelist.F90` 中的行号，便于回查
    pub line: u32,
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod field_tests;
```

- [ ] **Step 2: 写 `field_tests.rs`**

这些测试针对**生成出来的表**，所以它们同时是生成器的验收。

```rust
use crate::{all, find, Default, FieldKind};

#[test]
fn the_table_has_the_measured_number_of_fields() {
    // 实测：178 个顶层 DEF_ 标量 + 4 个派生类型共 535 个成员，合计 713。
    // 若这个数变了，要么上游改了，要么生成器漏了 —— 两种都必须有人看一眼。
    let total = all().len();
    assert!(
        (700..=760).contains(&total),
        "expected roughly 713 fields, got {total}"
    );
    let top = all().iter().filter(|f| f.owner.is_none()).count();
    assert_eq!(top, 178, "top-level DEF_ count changed");
}

#[test]
fn a_known_scalar_is_described_correctly() {
    let f = find("DEF_CASE_NAME").expect("DEF_CASE_NAME must be in the schema");
    assert!(matches!(f.kind, FieldKind::Character { .. }));
    assert!(f.owner.is_none());
}

#[test]
fn a_derived_type_member_carries_its_owner() {
    let f = find("DEF_forcing%dataset").expect("must be in the schema");
    assert_eq!(f.owner, Some("nl_forcing_type"));
}

#[test]
fn an_array_field_records_its_arity() {
    // fprefix(8) —— GUI 要知道它有 8 槽，且第 5 槽在 POINT 下是 'NULL'
    let f = find("DEF_forcing%fprefix").expect("must be in the schema");
    assert_eq!(f.arity, Some(8));
}

#[test]
fn defaults_that_differ_from_colm_are_visible_here() {
    // 这两个默认值正是「GUI 的默认值必须与 CoLM 的默认值不同」的原因：
    // 见 design.md §2.5。schema 必须如实记录 CoLM 的原值，
    // 偏离由上层决定并解释，而不是在这里偷偷改掉。
    assert_eq!(
        find("DEF_USE_OZONEDATA").map(|f| f.default),
        Some(Default::Logical(true))
    );
    assert_eq!(
        find("DEF_Runoff_SCHEME").map(|f| f.default),
        Some(Default::Integer(3))
    );
}

#[test]
fn no_local_variable_leaked_into_the_schema() {
    // MOD_Namelist.F90 里有 8 个不含 '=' 的声明（7 个不同名字），
    // 它们是子程序局部变量与哑元
    // （nlfile / fexists / ivar / ierr / iomesg / set_defaults / onoff），
    // 不是配置字段。生成器必须靠作用域排除它们 —— 靠 intent(...) 属性过滤
    // 是不够的，因为 fexists / ivar / ierr / iomesg 都没有 intent。
    for leaked in [
        "nlfile",
        "fexists",
        "ivar",
        "ierr",
        "iomesg",
        "set_defaults",
        "onoff",
    ] {
        assert!(
            find(leaked).is_none(),
            "{leaked} is a subroutine local, not a config field"
        );
    }
}

#[test]
fn the_history_type_contributes_the_bulk_of_the_table() {
    let n = all()
        .iter()
        .filter(|f| f.owner == Some("history_var_type"))
        .count();
    assert_eq!(n, 482, "history_var_type member count changed");
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p colm-schema`
Expected: 编译失败。具体是 `use crate::{all, find, Default, FieldKind};` 四个名字
都解析不了 —— Task 1 写的 `lib.rs` 只有 `pub mod` 声明，重导出与 `all()`/`find()`
要到 Task 8 Step 4b 才加上，那时 `generated::FIELDS` 才存在。这是 RED 状态。

**不要**为了让它编译而提前给 `lib.rs` 加导出：那两行属于 Task 8，
且在字段表存在之前它们无处可指。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-schema/src
git commit -m "Add failing tests describing the generated schema table"
```

---

## Task 8: schema 生成器

**Files:**
- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`
- Create: `crates/colm-schema/build-notes.md`
- Modify: 根 `Cargo.toml`（加入 `xtask` member）

- [ ] **Step 1: 写 `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
anyhow.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: 写生成器**

```rust
//! 代码生成：把 `MOD_Namelist.F90` 的声明变成 `colm-schema` 的字段表。
//!
//! 用法: cargo run -p xtask -- gen-schema
//!
//! 产物 `crates/colm-schema/src/generated.rs` **入库**，由
//! `crates/colm-schema/tests/drift.rs` 守住：重新生成必须逐字节一致。
//! 入库而不是 build.rs 现生成，是为了让 schema 的变化出现在 code review 的
//! diff 里 —— 上游加一个 DEF_ 或改一个默认值，应当是一次可见的改动，
//! 而不是某次构建之后悄悄换掉的东西。

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    if cmd != "gen-schema" {
        bail!("usage: cargo run -p xtask -- gen-schema");
    }
    let root = repo_root()?;
    let src = root.join("vendor/CoLM202X/share/MOD_Namelist.F90");
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let fields = extract(&text)?;
    let out = render(&fields);
    let dst = root.join("crates/colm-schema/src/generated.rs");
    std::fs::write(&dst, out)?;
    println!("wrote {} fields to {}", fields.len(), dst.display());
    Ok(())
}

#[derive(Debug)]
struct Field {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
    owner: Option<String>,
    line: u32,
}

/// 扫描模块的声明区与 type 块，**遇到 SUBROUTINE / FUNCTION 即停止**。
///
/// 这条是必须的：文件里有 8 个不含 `=` 的声明（7 个不同名字：nlfile /
/// fexists / ivar / ierr / iomesg / set_defaults / onoff），全部是子程序
/// 局部变量与哑元。靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
fn extract(text: &str) -> Result<Vec<Field>> {
    let mut out = Vec::new();
    let mut owner: Option<String> = None;
    let mut lines = text.lines().enumerate().peekable();

    while let Some((i, raw)) = lines.next() {
        let line = raw.trim();
        let low = line.to_ascii_lowercase();

        if low.starts_with("subroutine ") || low.starts_with("function ") {
            break; // 声明区到此为止
        }
        if let Some(rest) = low.strip_prefix("type ") {
            let n = rest.trim_start_matches(":: ").trim();
            if !n.is_empty() && !n.contains('(') {
                owner = Some(n.to_string());
            }
            continue;
        }
        if low.starts_with("end type") {
            owner = None;
            continue;
        }

        let Some(decl) = parse_decl(line) else {
            continue;
        };
        // 顶层只收 DEF_ 开头的；类型成员全收
        if owner.is_none() && !decl.name.starts_with("DEF_") {
            continue;
        }

        // 跨行数组字面量：实测 4 处，形如 `= (/ &` 续到 `/)`
        let mut default = decl.default.clone();
        if default.trim_end().ends_with('&') {
            let mut acc = default
                .trim_end()
                .trim_end_matches('&')
                .trim_end()
                .to_string();
            for (_, more) in lines.by_ref() {
                let m = more.trim();
                acc.push(' ');
                acc.push_str(m.trim_end().trim_end_matches('&').trim_end());
                if m.contains("/)") {
                    break;
                }
            }
            default = acc;
        }

        out.push(Field {
            name: decl.name.clone(),
            kind: decl.kind,
            default: default.trim().to_string(),
            doc: decl.doc,
            arity: decl.arity,
            owner: owner.clone(),
            line: (i + 1) as u32,
        });
    }

    if out.is_empty() {
        bail!("extracted zero fields — the declaration format must have changed");
    }
    Ok(out)
}

struct Decl {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
}

fn parse_decl(line: &str) -> Option<Decl> {
    let (head, tail) = line.split_once("::")?;
    let head_low = head.to_ascii_lowercase();
    let kind = if head_low.starts_with("logical") {
        "FieldKind::Logical".to_string()
    } else if head_low.starts_with("integer") {
        "FieldKind::Integer".to_string()
    } else if head_low.starts_with("real") {
        "FieldKind::Real".to_string()
    } else if head_low.starts_with("character") {
        let len = head_low
            .split_once("len=")
            .and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(1);
        format!("FieldKind::Character {{ len: {len} }}")
    } else {
        return None;
    };
    // 哑元与局部变量：没有 `=` 的一律跳过（配置字段实测 100% 带默认值）
    let (lhs, rhs) = tail.split_once('=')?;
    let (rhs, doc) = match rhs.find('!') {
        Some(p) => (&rhs[..p], Some(rhs[p + 1..].trim().to_string())),
        None => (rhs, None),
    };
    let lhs = lhs.trim();
    let (name, arity) = match lhs.split_once('(') {
        Some((n, a)) => (
            n.trim().to_string(),
            a.trim_end_matches(')').trim().parse::<usize>().ok(),
        ),
        None => (lhs.to_string(), None),
    };
    Some(Decl {
        name,
        kind,
        default: rhs.to_string(),
        doc,
        arity,
    })
}

fn render(fields: &[Field]) -> String {
    let mut s = String::new();
    s.push_str(
        "//! 由 `cargo run -p xtask -- gen-schema` 生成。**不要手改。**\n\
         //!\n\
         //! 源：vendor/CoLM202X/share/MOD_Namelist.F90\n\
         //! 漂移由 crates/colm-schema/tests/drift.rs 守住。\n\n\
         use crate::field::{Default, Field, FieldKind};\n\n\
         pub static FIELDS: &[Field] = &[\n",
    );
    for f in fields {
        let full = match &f.owner {
            Some(o) => format!("{}%{}", owner_prefix(o), f.name),
            None => f.name.clone(),
        };
        let doc = match &f.doc {
            Some(d) => format!("Some({:?})", d),
            None => "None".to_string(),
        };
        let arity = match f.arity {
            Some(n) => format!("Some({n})"),
            None => "None".to_string(),
        };
        let owner = match &f.owner {
            Some(o) => format!("Some({o:?})"),
            None => "None".to_string(),
        };
        let _ = writeln!(
            s,
            "    Field {{ name: {full:?}, kind: {}, default: {}, doc: {doc}, arity: {arity}, owner: {owner}, line: {} }},",
            f.kind,
            render_default(&f.kind, &f.default),
            f.line
        );
    }
    s.push_str("];\n");
    s
}

/// 派生类型名 -> 它在 namelist 里的实例名。
///
/// 手工映射，因为 Fortran 的类型定义与变量声明是分开的，而 namelist 文件里
/// 出现的是变量名。四个类型全在这里，新增类型时生成器会报错提醒。
fn owner_prefix(type_name: &str) -> &'static str {
    match type_name {
        "nl_domain_type" => "DEF_domain",
        "nl_simulation_time_type" => "DEF_simulation_time",
        "nl_forcing_type" => "DEF_forcing",
        "history_var_type" => "DEF_hist_vars",
        other => panic!("unknown derived type {other}: add it to owner_prefix"),
    }
}

fn render_default(kind: &str, raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("(/") {
        return format!("Default::Array({t:?})");
    }
    if kind.starts_with("FieldKind::Logical") {
        return format!(
            "Default::Logical({})",
            t.to_ascii_lowercase().contains("true")
        );
    }
    if kind.starts_with("FieldKind::Integer") {
        return match t.parse::<i64>() {
            Ok(i) => format!("Default::Integer({i})"),
            Err(_) => format!("Default::Str({t:?})"),
        };
    }
    if kind.starts_with("FieldKind::Real") {
        return format!("Default::Real({t:?})");
    }
    let unquoted = t.trim_matches(|c| c == '\'' || c == '"');
    format!("Default::Str({unquoted:?})")
}

fn repo_root() -> Result<PathBuf> {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !d.join(".git").exists() {
        if !d.pop() {
            bail!("not inside a git repository");
        }
    }
    Ok(d)
}
```

- [ ] **Step 3: 把 xtask 加入 workspace 并生成**

根 `Cargo.toml` 的 members 加上 `"xtask"`。

Run: `cargo run -p xtask -- gen-schema`
Expected: 打印 `wrote NNN fields to .../generated.rs`，NNN 在 700–760 之间。

若报 `extracted zero fields`，说明声明格式与实测不符——**去看 `MOD_Namelist.F90`
的实际写法再改正则**，不要放宽到能匹配任何东西。

- [ ] **Step 4: 让 Task 7 的测试通过**

Run: `cargo test -p colm-schema`
Expected: 7 条全部通过。

若 `no_local_variable_leaked_into_the_schema` 失败，说明作用域截断没生效——
检查 `extract` 是否在第一个 `SUBROUTINE` 处 `break`。**不要**改成按名字
黑名单过滤那 6 个：黑名单挡不住下一个新增的局部变量。

- [ ] **Step 4b: 给 `crates/colm-schema/src/lib.rs` 加上查询接口**

Task 1 只写了 `pub mod`。现在字段表存在了，追加：

```rust
pub use field::{Default, Field, FieldKind};

/// 全部字段，按声明顺序。
pub fn all() -> &'static [Field] {
    generated::FIELDS
}

/// 按全名查找，例如 `"DEF_forcing%dataset"`。
pub fn find(name: &str) -> Option<&'static Field> {
    generated::FIELDS.iter().find(|f| f.name == name)
}
```

- [ ] **Step 5: 写 `build-notes.md`**

```markdown
# colm-schema 的字段表是怎么来的

`src/generated.rs` 由 `cargo run -p xtask -- gen-schema` 从
`vendor/CoLM202X/share/MOD_Namelist.F90` 生成，**产物入库**。

## 为什么入库而不是 build.rs 现生成

上游加一个 `DEF_*` 或改一个默认值，应当是一次**在 code review 里看得见**的改动。
build.rs 会让它在某次构建之后悄悄换掉，没有人经手。

## 怎么更新

```bash
git -C vendor/CoLM202X checkout <新的 commit>
cargo run -p xtask -- gen-schema
cargo test -p colm-schema      # drift 测试确认产物与源一致
git add vendor/CoLM202X crates/colm-schema/src/generated.rs
```

## 生成器必须守住的两条

1. **作用域截断**：只扫描模块声明区与 `type ... end type`，遇到第一个
   `SUBROUTINE`（第 1132 行）就停。它之后有 8 个不含 `=` 的声明是子程序局部
   变量与哑元（`nlfile` `fexists` `ivar` `ierr` `iomesg` `set_defaults` `onoff`），
   靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
2. **派生类型名到实例名的映射**在 `owner_prefix` 里手工维护。
   Fortran 的类型定义与变量声明是分开的，而 namelist 文件里出现的是变量名。
   遇到未知类型时生成器会 panic，这是有意的：宁可停下来让人补，
   也不要生成一张名字错误的表。
```

- [ ] **Step 6: 提交**

```bash
git add xtask Cargo.toml crates/colm-schema
git commit -m "Generate the schema table from MOD_Namelist rather than hand-writing it"
```

---

## Task 9: 漂移测试

生成器的产物入库，就必须有东西守住「产物与源一致」。

**Files:**
- Create: `crates/colm-schema/tests/drift.rs`

- [ ] **Step 1: 写测试**

```rust
//! 入库的 generated.rs 必须与现在重新生成的结果一致。
//!
//! 没有这条，上游改了 MOD_Namelist.F90 之后 schema 会静默过时：
//! 编译照过、测试照绿，只有 GUI 少显示一个选项、或显示一个错误的默认值。

use std::path::PathBuf;
use std::process::Command;

#[test]
fn regenerating_the_schema_produces_the_committed_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let committed = root.join("crates/colm-schema/src/generated.rs");
    let before = std::fs::read_to_string(&committed).expect("generated.rs must exist");

    let out = Command::new(env!("CARGO"))
        .args(["run", "-q", "-p", "xtask", "--", "gen-schema"])
        .current_dir(&root)
        .output()
        .expect("run xtask");
    assert!(
        out.status.success(),
        "gen-schema failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = std::fs::read_to_string(&committed).expect("still readable");
    if before != after {
        // 还原，免得一次失败的测试把工作树弄脏
        std::fs::write(&committed, &before).expect("restore");
        panic!(
            "generated.rs is out of date with MOD_Namelist.F90.\n\
             Run: cargo run -p xtask -- gen-schema, then commit the result."
        );
    }
}
```

- [ ] **Step 2: 确认它能失败**

手工在 `generated.rs` 末尾加一行注释，跑测试，必须失败并给出那条提示；
然后确认工作树被还原（`git status` 干净）。

```bash
echo '// tamper' >> crates/colm-schema/src/generated.rs
cargo test -p colm-schema --test drift 2>&1 | tail -6
git status --short crates/colm-schema/src/generated.rs
git checkout crates/colm-schema/src/generated.rs
```

Expected: 测试失败并打印 `generated.rs is out of date`。

**注意还原语义**：测试还原的是它**读进来时**的内容。在真实的漂移场景里
（入库产物是干净的、上游变了），那正是入库的版本，于是一次失败的测试不会
把工作树弄脏 —— 这是它要防的。但在这个人为的篡改场景里，读进来的就是被
篡改的版本，所以测试失败之后 `git status --short` **仍然显示该文件被修改**，
这不是 bug。上面那条 `git checkout` 才是把它清掉的东西。

**一个不能失败的漂移测试等于没有漂移测试。**

- [ ] **Step 3: 全绿后提交**

```bash
git add crates/colm-schema/tests/drift.rs
git commit -m "Fail the build when the committed schema drifts from its source"
```

---

## Task 10: CI 与文档收尾

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`

- [ ] **Step 1: CI 加一步**

`rust` 作业在 `Format` 之后插入：

```yaml
      # 漂移测试要跑 xtask，而 xtask 读 vendor/CoLM202X。默认 checkout 不带
      # submodule，所以这一步必须自己拉 —— 否则测试会因为找不到源文件而失败，
      # 看起来像 schema 坏了，实际是 CI 没有源可比。
      - name: Fetch the CoLM source the schema is generated from
        run: git -c protocol.file.allow=always submodule update --init vendor/CoLM202X
```

**先确认 `.gitmodules` 的 url**（`cat .gitmodules`）。写这个计划时它是
`/Users/zhongwangwei/Desktop/colm-rust/CoLM202X` —— 一个**本机绝对路径**，
GitHub 托管 runner 上不存在。只要它还是本机路径，上面这一步就必须放进
`golden` 作业（自托管，路径存在），而不是 `rust` 作业。

但只挪 yaml 里的这一步**不够**：`rust` 作业跑的是 `cargo test --workspace`，
它照样会执行 roundtrip 与 drift，然后在 `canonicalize` 上炸掉。所以 `rust`
作业的 Test 步骤要改成只跑不需要 submodule 的部分：

```yaml
      # 不需要 vendor/CoLM202X 的全部测试。roundtrip 与 drift 要读 CoLM 源码，
      # 而 .gitmodules 还指向本机路径，托管 runner 上取不到 —— 它们在 golden
      # 作业里跑。oracle 的 judge 是集成测试但只读已入库的黄金文件，所以显式
      # 点名跑它，否则 --lib --bins 会把三平台上最有价值的那条覆盖丢掉。
      - name: Test
        run: |
          cargo test --workspace --lib --bins
          cargo test -p oracle --test judge
```

`golden` 作业里在既有步骤之后加一条跑完整套：

```yaml
      - name: Tests that need the CoLM source
        run: cargo test --workspace
```

并把 `golden-status` 的告警文案补上这两条，让「没跑」在 PR 界面上可见：

```yaml
            echo "::warning::The namelist round-trip and schema drift checks did NOT run either."
```

**不要**用 `#[ignore]` 达成同样效果。那会让本机开发者的 `cargo test` 也跳过
本 crate 的验收测试，而他们的 submodule 是好的 —— 代价落在了错误的人身上。

待 `.gitmodules` 的 url 改成远端后，这三处一起回退成 `rust` 作业里的
`cargo test --workspace` 加一步 submodule checkout。

- [ ] **Step 2: README 补一节**

在「跑黄金回归」之后插入：

```markdown
## 配置层

`crates/colm-namelist` 读写 CoLM 的 namelist，**保留原文格式**：解析→修改→
序列化后，未改动的行逐字节不变。验收是对 `vendor/CoLM202X` 里全部 55 个真实
`.nml`（4167 行）做往返测试。理由是用户算例文件里的注释是他们自己的笔记。

`crates/colm-schema` 描述每个 `DEF_*` 字段的类型、默认值与说明。这张表
**由 `cargo run -p xtask -- gen-schema` 从 `MOD_Namelist.F90` 生成**，产物入库，
`tests/drift.rs` 保证它不会与上游脱节。详见 `crates/colm-schema/build-notes.md`。

注意 schema 记录的是 **CoLM 的**默认值，不是本项目建议用户使用的值。
两者确实不同（`DEF_USE_OZONEDATA` 默认 `.true.` 但需要 2.8 GB 的臭氧数据；
`DEF_Runoff_SCHEME` 默认 `3` 但需要站点文件里有 `soil_texture`），
偏离由上层决定并解释，见 `docs/design.md` §2.5。
```

- [ ] **Step 3: 全量验证**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -q -p oracle --bin tier-check -- oracle/golden/*.nc
git diff --check
```

Expected: 全部通过。里程碑 0–1 的 21 个测试必须仍然全绿——本里程碑不应
触碰 `colm-kernel` 或 `oracle`。

- [ ] **Step 4: 提交**

```bash
git add .github/workflows/ci.yml README.md
git commit -m "Document the configuration layer and wire it into CI"
```

---

## 完成判据

逐条可验证：

- [ ] `cargo test --workspace` 通过；`colm-namelist` 的 25 个单元测试 + 3 个往返测试、
      `colm-schema` 的 7 个字段测试 + 1 个漂移测试全部执行（不是跳过）
- [ ] **55/55 个真实 `.nml` 逐字节往返**，没有任何文件被跳过
- [ ] 改一个字段后**恰好一行**发生变化，行数不变
- [ ] 改不存在的字段**报错**而不是静默追加
- [ ] 重复计数、数组切片、续行符、未闭合 group 各自**报错**而不是猜
- [ ] schema 表含 178 个顶层 `DEF_*` 与 482 个 `history_var_type` 成员
- [ ] 7 个子程序局部变量与哑元**没有**混进 schema
- [ ] 篡改 `generated.rs` 后漂移测试失败并给出重新生成的提示（篡改场景下需 `git checkout` 清理；
      真实漂移场景下测试自己会还原，不留脏树）
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all --check` 无输出
- [ ] `git status --short` 为空

---

## 留给后续里程碑的

- **schema 的取值范围**（`range_min`/`range_max`）本里程碑不做。`MOD_Namelist.F90`
  里没有这些信息，它散落在 `read_namelist` 的校验分支与各物理模块的注释里。
  GUI 需要它们来限制输入，但那属于 GUI 里程碑，届时再决定是补充一张手写的
  覆盖表还是从校验分支里提取。
- **静默覆盖规则**（design.md §2.6：`DEF_TOPMOD_method` 在 SinglePoint 下被强制为 0
  等）同样不在 schema 里。它们在 `read_namelist` 的代码里，不在声明里。
  GUI 必须显示「你要求了 X，模型实际用了 Y」，那需要解析 CoLM 的运行时输出，
  是 `colm-kernel` 编排层的事。
- **CaMa-Flood 与 TRACER 的 group**：解析器已经能读它们（往返测试覆盖了），
  但 schema 不描述它们的字段，因为 `MOD_Namelist.F90` 里没有。
