# colm-desktop 里程碑 0–1 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 `colm-desktop` 仓库骨架，并把 CoLM SinglePoint 的两个黄金输出（冬季 + 湿季）固化成可在 CI 里重跑的回归基准。

**Architecture:** 独立仓库，两个 Cargo workspace（引擎 / GUI 分离）。本计划只建引擎侧，且只建两个东西：`crates/colm-kernel` 中的**成败判定**（因为 CoLM 失败时退出码是 0，这是整个编排层的地基），以及 `oracle/` 中的**黄金输出运行器与判官**。判官用 `netcdf` crate 实现，因此本计划顺带在三个平台上验证了整个方案最大的依赖风险点。

**Tech Stack:** Rust 2021、`netcdf` 0.12（`static` feature 用于 Windows）、`toml`、`serde`、`sha2`、GitHub Actions；Fortran 内核经 git submodule 从 CoLM202X 构建。

---

## 计划序列（本文档是第 1 个）

A 阶段的 12 个里程碑放不进一个计划。拆成 6 个，每个自身产出可测试的软件：

| 计划 | 里程碑 | 产出 |
|---|---|---|
| **1（本文档）** | 0–1 | 仓库骨架 + 成败判定 + 两个黄金输出的回归基准 |
| 2 | 2–4 | `colm-namelist` / `colm-schema` / `colm-srfdata` / `colm-forcing`（输入侧） |
| 3 | 5–7 | `colm-kernel` 编排 / `colm-hist` / `colm-cli`（端到端命令行可用） |
| 4 | 8 | GUI（三栏工作台 + 新建向导） |
| 5 | 9–10 | Windows 原生构建 + 三个物理预设打包 |
| 6 | 11–12 | 批量 / 敏感性 / 算例管理 + 分发 |

**为什么里程碑 1 排在最前**：它是 C 阶段唯一的验收标准来源，而现在已经有材料。设计文档
`docs/colm-desktop-design.md` §2.8 / §2.8b 记录了这两次运行的完整实测结果。

---

## 前置条件

执行者必须先确认这些，否则本计划无法完成：

- [ ] `gfortran` ≥ 12、`netcdf-fortran`、`cargo` ≥ 1.75、`cmake`、C++ 编译器
- [ ] PLUMBER2 数据在本机。参考位置：`/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s`，
      需要 `Forcing/`、`Sitedata/`、`Observation/` 三个子目录。
      **数据不入库**（15 MB + 2.1 MB，且属第三方数据集），通过环境变量 `PLUMBER2_ROOT` 定位。
- [ ] CoLM202X 仓库在本机，用作 submodule 来源

---

**代码块的约定**：每个代码块的内容**就是文件的全部内容**，第一行就是文件第一行。
代码块不含任何「这是哪个文件」的定位注释 —— 文件路径只在步骤标题与 `**Files:**` 列表里
声明一次，避免出现两个真相源。（首版曾在代码块首行写路径注释，实现者据此把它当成了
文件内容，于是仓库里出现了一个自我命名的注释行。）

## 文件结构

```
colm-desktop/
├── Cargo.toml                          引擎 workspace 根；version/rust-version 用 workspace.package 继承
├── Cargo.lock                          入库：黄金回归依赖可复现的依赖版本
├── .gitignore
├── README.md
├── docs/
│   ├── design.md                       从 CoLM202X 复制过来的设计文档
│   └── plan-m0-m1.md                   本文档
├── crates/
│   └── colm-kernel/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs                  只 pub mod outcome;
│           ├── outcome.rs              成败判定（本计划唯一的库代码）
│           └── outcome_tests.rs        #[cfg(test)] #[path] 挂进来的行为测试
├── oracle/
│   ├── Cargo.toml                      两个 bin：golden-run、golden-compare
│   ├── src/
│   │   ├── bin/golden_run.rs           构建/定位内核，跑三段，判成败
│   │   └── bin/golden_compare.rs       NetCDF 判官（变量+维度+属性，create_time 白名单）
│   ├── cases/
│   │   ├── CN-Cng/
│   │   │   ├── case.nml                冬季窗口 2008-01-01 → 01-11
│   │   │   ├── forcing.nml.in          含 @PLUMBER2_ROOT@ 占位
│   │   │   └── site.nc                 增广站点文件（37 KB，入库）
│   │   └── CN-Cng-wet/
│   │       ├── case.nml                湿季窗口 2008-07-01 → 07-16
│   │       └── forcing.nml.in
│   ├── fixtures/
│   │   ├── inputs.sha256               外部输入的校验和清单
│   │   └── PROVENANCE.md               增广站点文件的 12 个合成字段及其出处
│   ├── golden/                         黄金 history 文件（首次运行后写入，入库）
│   ├── tolerances.toml                 §8.1 的 Tier 分级，机读形式
│   └── scripts/
│       ├── build_kernel.sh             从 submodule 构建一个预设并生成 manifest
│       └── make_site_nc.py             增广站点文件的生成器（Plan 2 会由 colm-srfdata 取代）
├── vendor/CoLM202X/                    submodule
└── .github/workflows/ci.yml
```

**边界说明**：`colm-kernel` 在本计划里**只有成败判定**，没有进程编排。编排在 Plan 3 加。
这样做的理由是判定逻辑是纯函数、可完整单测，而编排需要真实子进程；先把地基打牢。

---

## Task 1: 建仓库与两个 workspace 骨架

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `crates/colm-kernel/Cargo.toml`
- Create: `crates/colm-kernel/src/lib.rs`
- Create: `crates/colm-kernel/src/outcome.rs`（占位，Task 2 覆盖）

`README.md` 在 Task 10 写，不在本任务。

- [ ] **Step 1: 建目录并初始化 git**

```bash
mkdir -p ~/Desktop/colm-rust/colm-desktop
cd ~/Desktop/colm-rust/colm-desktop
git init
mkdir -p crates/colm-kernel/src oracle/src/bin oracle/cases oracle/fixtures oracle/golden oracle/scripts docs .github/workflows
```

- [ ] **Step 2: 写 workspace 根 `Cargo.toml`**

GUI 侧**故意不在这个 workspace 里**（Plan 4 建 `gui/` 作为独立 workspace），
这样 `cargo test --workspace` 永远不会把 webkit2gtk 拖进来。

```toml
[workspace]
# resolver 3 是 MSRV 感知的。实测它不会硬失败 —— 无 MSRV 兼容版本时按
# incompatible-rust-versions = "fallback" 回退并标注。启用它当场就暴露出
# hdf5-metno-src 0.10.4 requires Rust 1.85.1，故下面的 MSRV 是 1.85.1 而非 1.85。
resolver = "3"
# oracle 在 Task 6 加入。此处只列已存在的 member，
# 否则 Task 1 结束时 cargo 会因找不到 oracle/Cargo.toml 而失败。
members = ["crates/colm-kernel"]

[workspace.package]
version = "0.1.0"
edition = "2021"
# 用 MSRV 而不是 rustup 的工具链固定文件：本机 rust 由 Homebrew 安装，没有 rustup，
# 那类文件会被静默忽略。rust-version 由 cargo 本体检查。
#
# 警告：[workspace.package] 与 [workspace.dependencies] 都只是模板，成员必须逐个写
# `field.workspace = true` 才会继承。不写的话这些声明和它们要替换掉的那个 rustup
# 文件一样完全无效 —— 实测：MSRV 写 1.99 而成员未 opt-in 时，1.97.1 上照样编译成功。
# 本项目在同一轮里被这个机制咬了两次（rust-version，以及一个失效的 netcdf-sys 钉子）。
rust-version = "1.85.1"
license = "MIT OR Apache-2.0"
repository = "https://github.com/zhongwangwei/colm-desktop"
# 这是研究模型的私有提取，永不上 crates.io。
publish = false

# 本项目没有任何需要 unsafe 的地方（netcdf crate 已封装 FFI）。
# 注意 forbid 比 deny 强：局部 #[allow(unsafe_code)] 会被 E0453 拒绝。
# 将来若某成员确实需要 FFI，出口在 workspace 层面（改 deny，或该成员不 opt-in），
# 不是局部 allow。
[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.dependencies]
# 不钉版本。曾经写过 `netcdf = "=0.12.0"` + `netcdf-sys = "=0.9.0"`，两点都错：
#   1. 后者完全无效 —— 没有成员引用 netcdf-sys，模板条目不会被解析或钉住，
#      它照样浮到 0.9.2（与上面 [workspace.package] 的坑同一类型）。
#   2. 钉版本的动机基于一个被污染的实验。曾断言 netcdf 0.12 + static
#      「无法解析、报 links 冲突」，实为复制目录时带过来的旧 Cargo.lock 把
#      netcdf-src 钉在 0.5.0 所致。干净环境下最新组合解析正常。
# 两个依赖图（netcdf-sys 0.9.0→HDF5 2.0.0，0.9.2→HDF5 2.2.0）都已静态构建并
# 成功读取真实黄金文件（129/129 变量），所以不需要偏好任何一个。
# 由入库的 Cargo.lock 冻结实际解析结果。
#
# default-features = false 是关掉 ndarray：判官只用 Vec<f64>。
netcdf = { version = "0.12", default-features = false }
serde = { version = "1", features = ["derive"] }
toml = "1"
sha2 = "0.10"
anyhow = "1"

[profile.release]
lto = "thin"
codegen-units = 1
debug = 1
```

- [ ] **Step 3: 写 `.gitignore`**

`oracle/golden/` **不**在忽略列表里——黄金文件必须入库。

```gitignore
# 不加前导斜杠：Plan 4 的 gui/ 是独立 workspace，会有自己的 gui/target/。
# `/target` 只锚定仓库根，实测漏掉 gui/target/ 与 gui/src-tauri/target/ ——
# 在一个以「提交黄金 NetCDF 文件」为产物的仓库里，误提交构建产物代价最高。
target/
**/*.rs.bk
# 内核构建产物：可从 submodule 重建，不入库
/kernels/
# 黄金运行的工作目录
/oracle/work/
.DS_Store
```

- [ ] **Step 4: 写 `crates/colm-kernel/Cargo.toml`**

```toml
[package]
name = "colm-kernel"
version.workspace = true
edition.workspace = true
rust-version.workspace = true   # 必须显式 opt-in，否则 MSRV 声明无效
license.workspace = true
publish.workspace = true

[dependencies]

[lints]
workspace = true
```

无依赖是刻意的：成败判定是纯逻辑。
三个 `.workspace = true` 的 opt-in 不是样板：`[workspace.package]` 的字段
不会自动继承，漏掉就等于没声明。

- [ ] **Step 5: 写 `crates/colm-kernel/src/lib.rs`**

```rust
//! CoLM 内核的调用与结果判定。
//!
//! 本 crate 存在的理由：CoLM 在单点模式下，成功与失败**都以退出码 0 结束，
//! 但走的是两条不同的路**：
//!
//! - 失败走 `share/MOD_SPMD_Task.F90` 的 `CoLM_stop`，其 `#ifndef USEMPI`
//!   分支是裸 `STOP`，退出码 0。
//! - 成功不执行任何收尾调用，直接跑到 `main/CoLM.F90:764` 的 `END PROGRAM CoLM`
//!   正常终止（`spmd_exit` 只定义并调用于 `#ifdef USEMPI` 内）。
//!
//! 退出码相同是两条路径的巧合，不是共用一条路径。
//! 因此调用方绝不能依赖退出码判断成败。
//!
//! 附带结论：既然 `CoLM_stop` 是失败专用的，把那个裸 `STOP` 改成 `STOP 1`
//! 是安全的上游修复。即便上游改了，本模块仍然必要 —— 产物硬校验能抓住
//! 「跑完了但没写出该写的文件」，错误标记扫描能抓住部分失败。

pub mod outcome;
```

- [ ] **Step 6: 写 `crates/colm-kernel/src/outcome.rs` 占位**

`lib.rs` 声明了 `pub mod outcome;`，该文件必须存在，否则 Step 7 的 `cargo build`
会报 `file not found for module 'outcome'`。Task 2 会覆盖它。

```rust
//! 判定一段 CoLM 运行是成功还是失败。实现见 Task 2/3。
```

- [ ] **Step 7: 确认能编译**

Run: `cargo build`
Expected: `Compiling colm-kernel v0.1.0` 后成功，无警告。

Run: `cargo --version`
Expected: ≥ 1.85（`rust-version` 声明的下限）。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 无输出。**这一步不能留到最后**：`unsafe_code = "forbid"` 与
`doc_lazy_continuation` 之类的 lint 会让「只有注释」的文件也编不过 ——
本计划的 doc comment 就曾因列表项后紧跟未缩进续行而被 clippy 拒绝。
Markdown 列表在 doc comment 里前后都要有空 `//!` 行，续行要缩进到 bullet 之下。

Run: `cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys; [print(p['name'], p['rust_version'], p['publish']) for p in json.load(sys.stdin)['packages']]"`
Expected: `colm-kernel 1.85.1 []`
**这一步是必须的**：若打印出 `None`，说明成员没有 opt-in，MSRV 与 publish 声明
都完全无效（实测过：MSRV 写 1.99 而成员未 opt-in 时，1.97.1 上照样编译成功）。

- [ ] **Step 8: 提交（含 `Cargo.lock`）**

`cargo build` 会在 workspace 根生成 `Cargo.lock`。**它必须入库**，理由不止「产出二进制的
项目按惯例提交 lockfile」：本计划的黄金输出回归依赖可复现构建，`netcdf` crate 的版本
浮动会改变行为并使黄金比对失效。所以 lockfile 在这里是载荷性的，不是惯例性的。
`.gitignore` 里因此**没有** `Cargo.lock`。

```bash
git add Cargo.toml Cargo.lock .gitignore crates/
git commit -m "Add workspace skeleton with colm-kernel crate"
git status --short   # 必须为空输出
```

---

## Task 2: 成败判定 —— 写失败的测试

判定规则来自设计文档 §2.4 与 §6.3。**每个测试用例都是实测观察到的情形，不是设想的。**

**Files:**
- Create: `crates/colm-kernel/src/outcome_tests.rs`
- Modify: `crates/colm-kernel/src/outcome.rs`（Task 3 才写实现，本任务只建空壳让测试能编译失败）

- [ ] **Step 1: 写测试文件**

```rust
use super::*;
use std::path::PathBuf;

/// 一个必然存在的路径，用于「产物齐全」的用例。
///
/// 用 `CARGO_MANIFEST_DIR` 而不是 `file!()`：后者是相对路径，而 `cargo test`
/// 的工作目录是 package 根还是 workspace 根随版本而异，相对路径会时不时不存在，
/// 让这些用例变成假失败。`CARGO_MANIFEST_DIR` 是绝对路径，`Cargo.toml` 必然在。
fn existing_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn missing_path() -> PathBuf {
    PathBuf::from("/nonexistent/definitely-not-here.nc")
}

#[test]
fn test_helpers_are_sane() {
    // 若这条失败，说明下面所有「产物齐全」的用例都是假通过/假失败。
    assert!(
        existing_path().exists(),
        "{} should exist",
        existing_path().display()
    );
    assert!(!missing_path().exists());
}

#[test]
fn unrecognised_namelist_variable_is_a_failure_despite_exit_zero() {
    // 实测：namelist 里写了未声明的变量名，colm.x 打印错误后 CoLM_stop -> 裸 STOP -> 退出码 0
    let stdout = " ERROR in /tmp/bad.nml : Cannot match namelist object name def_not_a_real_var\n\
                  \x20 ***** ERROR: Problem reading namelist: /tmp/bad.nml\n";
    let got = adjudicate(Stage::Colm, Some(0), stdout, &[existing_path()]);
    match got {
        Outcome::Failed(Failure::ErrorMarker { marker, .. }) => {
            assert_eq!(marker, "Cannot match namelist object name");
        }
        other => panic!("expected ErrorMarker, got {other:?}"),
    }
}

#[test]
fn missing_rawdata_is_a_failure_despite_exit_zero() {
    // 实测：站点文件缺 soil_vf_clay -> 回落到 rawdata -> 打不开 -> 退出码仍是 0
    let stdout =
        "Netcdf error: No such file or directory /x/rawdata/soil/vf_clay_s.nc cannot open\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert!(
        matches!(got, Outcome::Failed(Failure::ErrorMarker { .. })),
        "got {got:?}"
    );
}

#[test]
fn invalid_time_window_malloc_failure_is_a_failure_despite_exit_zero() {
    // 实测：结束时间早于开始时间 -> NetCDF malloc failure -> 退出码 0
    let stdout = "Netcdf error: NetCDF: Memory allocation (malloc) failure\n";
    let got = adjudicate(Stage::Colm, Some(0), stdout, &[existing_path()]);
    assert!(
        matches!(got, Outcome::Failed(Failure::ErrorMarker { .. })),
        "got {got:?}"
    );
}

#[test]
fn benign_null_history_namelist_line_is_not_a_failure() {
    // 实测：没设 DEF_HIST_vars_namelist 时必然出现这行，它长得像失败但无害。
    // 这个用例防止判官过度敏感 —— 它是三件套里最容易做错的一环。
    let stdout = "History namelist file: null does not exist.\n\
                  Successful in surface data making.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert_eq!(
        got,
        Outcome::Succeeded,
        "benign line must not be treated as failure"
    );
}

#[test]
fn missing_success_marker_is_a_failure() {
    let stdout = "Blocks : Set (360 longitude x 180 latitude) blocks for Single Point.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert_eq!(
        got,
        Outcome::Failed(Failure::MissingSuccessMarker(Stage::MkSrfData))
    );
}

#[test]
fn missing_artifact_is_a_failure_even_with_success_marker() {
    let stdout = "Successful in surface data making.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[missing_path()]);
    match got {
        Outcome::Failed(Failure::MissingArtifact(p)) => assert_eq!(p, missing_path()),
        other => panic!("expected MissingArtifact, got {other:?}"),
    }
}

#[test]
fn nonzero_exit_is_a_failure() {
    // 实测：namelist 文件本身不存在 -> gfortran runtime error -> 退出码 2
    let stdout = "Fortran runtime error: Cannot open file '': No such file or directory\n";
    let got = adjudicate(Stage::Colm, Some(2), stdout, &[existing_path()]);
    assert!(matches!(got, Outcome::Failed(_)), "got {got:?}");
}

#[test]
fn all_three_stages_have_distinct_success_markers() {
    let markers = [
        Stage::MkSrfData.success_marker(),
        Stage::MkIniData.success_marker(),
        Stage::Colm.success_marker(),
    ];
    assert_eq!(markers[0], "Successful in surface data making.");
    assert_eq!(markers[1], "CoLM Initialization Execution Completed");
    assert_eq!(markers[2], "CoLM Execution Completed.");
    // 三者必须互不为子串，否则一段的成功标记会误判另一段
    for (i, a) in markers.iter().enumerate() {
        for (j, b) in markers.iter().enumerate() {
            if i != j {
                assert!(!a.contains(b), "{a:?} contains {b:?}");
            }
        }
    }
}

#[test]
fn happy_path_succeeds() {
    let stdout = "Elevation :   138.00 (from SITE)\n\
                  Successful in surface data making.\n";
    let got = adjudicate(Stage::MkSrfData, Some(0), stdout, &[existing_path()]);
    assert_eq!(got, Outcome::Succeeded);
}

#[test]
fn stage_program_names_match_the_built_executables() {
    // build_kernel.sh 拷出的是 run/{mksrfdata,mkinidata,colm}.x，
    // 而 golden-run 用 format!("{}.x", stage.program()) 拼路径。
    // 这三个字符串是两者之间唯一的契约，错一个字母的表现是「内核找不到」，
    // 而那要到 Task 6 才会暴露。
    assert_eq!(Stage::MkSrfData.program(), "mksrfdata");
    assert_eq!(Stage::MkIniData.program(), "mkinidata");
    assert_eq!(Stage::Colm.program(), "colm");
}
```

- [ ] **Step 2: 建空的 `outcome.rs` 让测试能编译到「未定义」为止**

```rust
#[cfg(test)]
#[path = "outcome_tests.rs"]
mod outcome_tests;
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p colm-kernel`
Expected: 编译失败，报 `cannot find function 'adjudicate'`、`cannot find type 'Stage'` 等。
**这就是我们要的失败** —— 确认测试真的在跑，而不是被静默跳过。

- [ ] **Step 4: 提交失败的测试**

```bash
git add crates/colm-kernel/src/outcome.rs crates/colm-kernel/src/outcome_tests.rs
git commit -m "Add failing tests for CoLM outcome adjudication"
```

---

## Task 3: 成败判定 —— 实现

**Files:**
- Modify: `crates/colm-kernel/src/outcome.rs`

- [ ] **Step 1: 写实现**

```rust
//! 判定一段 CoLM 运行是成功还是失败。
//!
//! 退出码不是证据。实测（设计文档 §2.4）：
//!
//! - namelist 文件不存在：退出码 2
//! - namelist 里有未声明的变量：退出码 0
//! - 缺 rawdata、NetCDF 打不开：退出码 0
//! - 时间窗非法、malloc failure：退出码 0
//!
//! 因此判定必须同时满足三件事：无错误标记、有正向成功标记、产物齐全。

use std::path::{Path, PathBuf};

/// CoLM 单点流程的三段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    MkSrfData,
    MkIniData,
    Colm,
}

impl Stage {
    /// 该段成功时必然打印的行。缺了它就不算成功，无论退出码是什么。
    pub fn success_marker(self) -> &'static str {
        match self {
            Stage::MkSrfData => "Successful in surface data making.",
            Stage::MkIniData => "CoLM Initialization Execution Completed",
            Stage::Colm => "CoLM Execution Completed.",
        }
    }

    /// 可执行文件名（不含平台后缀）。
    pub fn program(self) -> &'static str {
        match self {
            Stage::MkSrfData => "mksrfdata",
            Stage::MkIniData => "mkinidata",
            Stage::Colm => "colm",
        }
    }
}

/// 出现即判失败的子串。
///
/// **顺序是语义的**：同一行可能命中多个标记，报告的是第一个命中的，
/// 所以具体的标记必须排在笼统的之前。实例：CoLM 那行
/// `ERROR in /x.nml : Cannot match namelist object name def_foo`
/// 同时含 `ERROR in` 与 `Cannot match namelist object name`，
/// 后者信息量大得多，必须先命中。
///
/// 新增条目前先确认它不会命中 `BENIGN_LINES` 里的行。
const FAILURE_MARKERS: &[&str] = &[
    // 具体
    "Cannot match namelist object name",
    "Memory allocation (malloc) failure",
    "Fortran runtime error",
    "Error termination",
    // 笼统
    "Netcdf error",
    "***** ERROR",
    "ERROR in",
];

/// 长得像失败但无害的整行。逐行**完全匹配去空白后**的文本，不做子串匹配，
/// 以免一条宽松的豁免掩盖真实错误。
const BENIGN_LINES: &[&str] = &[
    // 没有设置 DEF_HIST_vars_namelist 时必然出现
    "History namelist file: null does not exist.",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Failed(Failure),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// 进程以非零状态退出。
    NonZeroExit { status: i32, last_line: String },
    /// stdout 命中了已知的失败标记。
    ErrorMarker { marker: &'static str, line: String },
    /// 该段的成功标记从未出现。
    MissingSuccessMarker(Stage),
    /// 该段应产出的文件不存在。
    MissingArtifact(PathBuf),
}

/// 判定一段运行的结果。
///
/// `exit_status` 为 `None` 表示进程被信号终止（例如用户取消）。
/// `artifacts` 是该段必须产出的文件；顺序即检查顺序，第一个缺失者被报告。
pub fn adjudicate(
    stage: Stage,
    exit_status: Option<i32>,
    stdout: &str,
    artifacts: &[PathBuf],
) -> Outcome {
    // 1. 非零退出：直接失败。零退出**不构成**成功的证据。
    match exit_status {
        Some(0) => {}
        Some(status) => {
            return Outcome::Failed(Failure::NonZeroExit {
                status,
                last_line: last_nonempty_line(stdout).to_string(),
            });
        }
        None => {
            return Outcome::Failed(Failure::NonZeroExit {
                status: -1,
                last_line: last_nonempty_line(stdout).to_string(),
            });
        }
    }

    // 2. 错误标记扫描，逐行进行，先排除无害行。
    for line in stdout.lines() {
        if is_benign(line) {
            continue;
        }
        if let Some(marker) = FAILURE_MARKERS.iter().find(|m| line.contains(**m)) {
            return Outcome::Failed(Failure::ErrorMarker {
                marker,
                line: line.trim().to_string(),
            });
        }
    }

    // 3. 正向成功标记必须出现。
    if !stdout.contains(stage.success_marker()) {
        return Outcome::Failed(Failure::MissingSuccessMarker(stage));
    }

    // 4. 产物硬校验。
    for path in artifacts {
        if !path_is_present(path) {
            return Outcome::Failed(Failure::MissingArtifact(path.clone()));
        }
    }

    Outcome::Succeeded
}

fn is_benign(line: &str) -> bool {
    BENIGN_LINES.contains(&line.trim())
}

fn path_is_present(path: &Path) -> bool {
    path.exists()
}

fn last_nonempty_line(stdout: &str) -> &str {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod outcome_tests;
```

- [ ] **Step 2: 运行测试确认通过**

Run: `cargo test -p colm-kernel`
Expected: `test result: ok. 11 passed; 0 failed`

- [ ] **Step 3: 格式与 clippy 干净**

Run: `cargo fmt --all --check`
Expected: 无输出。**这一步不能推到 Task 9** —— 本计划的代码块是 rustfmt 格式化过的，
若这里就报差异，说明抄写时引入了格式偏移，应当当场 `cargo fmt --all` 并核对
差异确实只是换行、没有内容变化。首版计划的代码块**没有**格式化过，
结果四个文件一路带着漂移直到 Task 9 才暴露。

Run: `cargo clippy -p colm-kernel --all-targets -- -D warnings`
Expected: 无输出。**`--all-targets` 不能省** —— 少了它 clippy 不检查测试目标，
而本任务的 10 个测试正是刚写的代码。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-kernel/src/outcome.rs
git commit -m "Adjudicate CoLM run outcomes without trusting the exit code"
```

---

## Task 4: 黄金算例的输入 fixture

**Files:**
- Create: `oracle/cases/CN-Cng/site.nc`（由 Step 1 的脚本生成，36,250 字节，入库）
- Create: `oracle/cases/CN-Cng/case.nml`
- Create: `oracle/cases/CN-Cng/forcing.nml.in`
- Create: `oracle/cases/CN-Cng-wet/case.nml`
- Create: `oracle/cases/CN-Cng-wet/forcing.nml.in`
- Create: `oracle/fixtures/inputs.sha256`
- Create: `oracle/fixtures/PROVENANCE.md`
- Create: `oracle/scripts/make_site_nc.py`

- [ ] **Step 1: 写增广站点文件的生成器**

这是 Plan 2 里 `colm-srfdata` 的前身。**保留它**：Plan 2 的 `colm-srfdata` 必须有一个
测试断言它重新生成出的文件与 `oracle/cases/CN-Cng/site.nc` 逐位相同。

```python
#!/usr/bin/env python3
"""生成增广站点文件：注入 CoLM 无条件读取但 PLUMBER2 不提供的 12 个字段。

Plan 2 中会由 colm-srfdata crate 取代本脚本；届时 colm-srfdata 必须能
逐位重现本脚本的输出。取值出处见 PROVENANCE.md。

用法: make_site_nc.py <PLUMBER2_ROOT> <输出路径>
"""
import shutil
import sys

import netCDF4 as nc
import numpy as np

SITE = "CN-Cng_2008-2009_FLUXNET2015"

# MOD_SoilColorRefl 的第 10 档（20 档中的中间值）
SOIL_ALB = {
    "soil_s_v_alb": 0.14,
    "soil_d_v_alb": 0.25,
    "soil_s_n_alb": 0.28,
    "soil_d_n_alb": 0.39,
}

# CoLM 标准 10 层土壤厚度 (m)；srfdata 只用前 8 层
DZ_SOIL = np.array(
    [0.0175, 0.0276, 0.0455, 0.0750, 0.1236, 0.2038, 0.3360, 0.5539, 0.9133, 1.5058]
)[:8]

# MOD_Initialize.F90:271 的 BVIC_USDA(0:12)
BVIC_USDA = [1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180,
             0.100, 0.090, 0.150, 0.080, 0.050]


def usda_class(sand, silt, clay):
    """USDA 12 类质地。编号与 BVIC_USDA(1..12) 对齐：1=Sand … 12=Clay。"""
    if clay >= 40 and silt < 40 and sand <= 45:
        return 12, "Clay"
    if clay >= 40 and silt >= 40:
        return 11, "Silty clay"
    if clay >= 35 and sand >= 45:
        return 10, "Sandy clay"
    if 27 <= clay < 40 and 20 < sand <= 45:
        return 9, "Clay loam"
    if 27 <= clay < 40 and sand <= 20:
        return 8, "Silty clay loam"
    if 20 <= clay < 35 and silt < 28 and sand > 45:
        return 7, "Sandy clay loam"
    if silt >= 80 and clay < 12:
        return 5, "Silt"
    if silt >= 50 and (12 <= clay < 27 or clay < 12):
        return 4, "Silt loam"
    if 7 <= clay < 27 and 28 <= silt < 50 and sand <= 52:
        return 6, "Loam"
    if sand > 85 and (silt + 1.5 * clay) < 15:
        return 1, "Sand"
    if 70 <= sand <= 91 and (silt + 1.5 * clay) >= 15 and (silt + 2 * clay) < 30:
        return 2, "Loamy sand"
    return 3, "Sandy loam"


def main(plumber2_root, out_path):
    src = f"{plumber2_root}/Sitedata/{SITE}_site.nc"
    obs = f"{plumber2_root}/Observation/{SITE}_Flux.nc"
    shutil.copy(src, out_path)

    with nc.Dataset(obs) as o:
        elevation = float(np.ravel(o["elevation"][:])[0])

    d = nc.Dataset(out_path, "a")

    def put_scalar(name, value, source):
        v = d.createVariable(name, "f8")
        v[:] = value
        v.setncattr("source", source)

    put_scalar("lakedepth", 1.0, "synthesized: MOD_SingleSrfdata.F90:47 module default")
    put_scalar("elevation", elevation, f"synthesized: from {SITE}_Flux.nc elevation")
    put_scalar("elvstd", 0.0, "synthesized: MOD_SingleSrfdata.F90:88 module default")
    put_scalar("sloperatio", 0.0, "synthesized: MOD_SingleSrfdata.F90:89 module default (flat)")
    for name, val in SOIL_ALB.items():
        put_scalar(name, val, "synthesized: MOD_SoilColorRefl class L=10")

    vf_sand = np.asarray(d["soil_vf_sand"][:], dtype=float)
    vf_grav = np.asarray(d["soil_vf_gravels"][:], dtype=float)
    vf_om = np.asarray(d["soil_vf_om"][:], dtype=float)
    wf_sand = np.asarray(d["soil_wf_sand"][:], dtype=float)
    wf_grav = np.asarray(d["soil_wf_gravels"][:], dtype=float)
    omd = np.asarray(d["soil_OM_density"][:], dtype=float)
    bd = np.asarray(d["soil_BD_all"][:], dtype=float)

    wf_om = np.clip(vf_om * omd / np.where(bd > 0, bd, 1.0), 0.0, 1.0)
    vf_clay = 0.25 * np.clip(1.0 - vf_sand - vf_grav - vf_om, 0.0, 1.0)
    wf_clay = 0.25 * np.clip(1.0 - wf_sand - wf_grav - wf_om, 0.0, 1.0)

    note = ("synthesized: clay = 25% of the non-sand/gravel/OM remainder "
            "(loam 1:3 clay:silt); wf_om = vf_om * OM_density / BD_all")
    for name, arr in [("soil_vf_clay", vf_clay), ("soil_wf_clay", wf_clay),
                      ("soil_wf_om", wf_om)]:
        v = d.createVariable(name, "f8", ("soil",))
        v[:] = arr
        v.setncattr("source", note)

    # soil_texture：0-60cm 深度加权，归一化到细土后查 USDA 三角
    top = np.concatenate([[0.0], np.cumsum(DZ_SOIL)])[:8]
    bot = np.cumsum(DZ_SOIL)
    w = np.clip(np.minimum(bot, 0.60) - np.minimum(top, 0.60), 0.0, None)
    silt8 = 1.0 - wf_sand[:8] - wf_clay[:8] - wf_grav[:8] - wf_om[:8]
    tot = wf_sand[:8] + silt8 + wf_clay[:8]
    sand_pct = 100 * np.average(wf_sand[:8] / tot, weights=w)
    clay_pct = 100 * np.average(wf_clay[:8] / tot, weights=w)
    silt_pct = 100 * np.average(silt8 / tot, weights=w)
    cls, name = usda_class(sand_pct, silt_pct, clay_pct)

    v = d.createVariable("soil_texture", "i4")
    v[:] = cls
    v.setncattr(
        "source",
        f"synthesized: USDA triangle on 0-60cm depth-weighted "
        f"sand {sand_pct:.1f}% / silt {silt_pct:.1f}% / clay {clay_pct:.1f}% "
        f"-> class {cls} ({name}), BVIC {BVIC_USDA[cls]}",
    )
    d.close()
    print(f"{out_path}: soil_texture = {cls} ({name}), BVIC = {BVIC_USDA[cls]}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
```

- [ ] **Step 2: 生成站点文件**

Task 1 只建了 `oracle/cases/`，没有建两个算例的子目录，而脚本用 `shutil.copy`
写入目标路径 —— 目录不存在会直接 `FileNotFoundError`。先建目录：

```bash
mkdir -p oracle/cases/CN-Cng oracle/cases/CN-Cng-wet
export PLUMBER2_ROOT=~/Desktop/colm-rust/PLUMBER2s
python3 oracle/scripts/make_site_nc.py "$PLUMBER2_ROOT" oracle/cases/CN-Cng/site.nc
```

Expected 输出：`oracle/cases/CN-Cng/site.nc: soil_texture = 4 (Silt loam), BVIC = 0.23`

产出的 sha256 必须是 `6132cf1e56e57b01ec7129558eef5c51bb56cdf8c42d85f45bf7a49f3534f507`
（`shasum -a 256 oracle/cases/CN-Cng/site.nc`）。对不上先按 PROVENANCE.md 的指引
逐变量比对，再怀疑数据。

- [ ] **Step 3: 写冬季算例 namelist**

注释里必须写清每一处**偏离 CoLM 默认值**的原因——这是设计文档 §2.5 的直接产物。

```fortran
&nl_colm

! 黄金回归算例：冬季窗口。见 docs/design.md §2.8
   DEF_CASE_NAME = 'CN-Cng'

   SITE_fsitedata    = '@CASE_DIR@/site.nc'
   SITE_lon_location = 123.50920
   SITE_lat_location = 44.59330
   SITE_landtype     = 10

! 全部地表参数取自站点文件；本仓库不携带全球 rawdata 树
   USE_SITE_landtype         = .true.
   USE_SITE_pctpfts          = .true.
   USE_SITE_htop             = .true.
   USE_SITE_LAI              = .true.
   USE_SITE_lakedepth        = .true.
   USE_SITE_soilreflectance  = .true.
   USE_SITE_soilparameters   = .true.
   USE_SITE_dbedrock         = .true.
   USE_SITE_topography       = .true.

! PLUMBER2 的时间轴是地方时。SinglePoint 是唯一允许非格林尼治时的配置
! (MOD_TimeManager.F90:74-79 的强制覆盖在 #ifndef SinglePoint 内)
   DEF_simulation_time%greenwich     = .FALSE.
   DEF_simulation_time%start_year    = 2008
   DEF_simulation_time%start_month   = 1
   DEF_simulation_time%start_day     = 1
   DEF_simulation_time%start_sec     = 0
   DEF_simulation_time%end_year      = 2008
   DEF_simulation_time%end_month     = 1
   DEF_simulation_time%end_day       = 11
   DEF_simulation_time%end_sec       = 86400
   DEF_simulation_time%spinup_year   = 0
   DEF_simulation_time%spinup_month  = 1
   DEF_simulation_time%spinup_day    = 365
   DEF_simulation_time%spinup_sec    = 86400
   DEF_simulation_time%spinup_repeat = 0
   DEF_simulation_time%timestep      = 1800.

   DEF_dir_rawdata = '@WORK_DIR@/rawdata_unused/'
   DEF_dir_runtime = '@WORK_DIR@/runtime_unused/'
   DEF_dir_output  = '@WORK_DIR@/out/'

   DEF_USE_SoilInit = .false.
   DEF_USE_BEDROCK  = .false.

   DEF_LAI_MONTHLY       = .true.
   DEF_LAI_CHANGE_YEARLY = .true.

! Simple VIC (CoLM 默认)。需要站点文件里有 soil_texture
   DEF_Runoff_SCHEME = 3

! CoLM 默认 .true.，会去 DEF_dir_runtime 读 Ozone/Global/OZONE-setgrid.nc。
! 关闭后 MOD_Ozone.F90:83 用常数 forc_ozone = 100 ppbv，臭氧胁迫仍生效。
   DEF_USE_OZONEDATA = .false.

   DEF_forcing_namelist = '@WORK_DIR@/forcing.nml'

   DEF_WRST_FREQ    = 'MONTHLY'
   DEF_HIST_FREQ    = 'HOURLY'
   DEF_HIST_groupby = 'MONTH'
   DEF_hist_vars_out_default = .true.
/
```

- [ ] **Step 4: 写强迫场 namelist 模板**

`vname` 第 5 槽是 `'NULL'`，标量风速进第 6 槽——这是 POINT reader 的固定约定，
设计文档 §2.10。**不要「修正」成把风速放第 5 槽。**

```fortran
&nl_colm_forcing

   DEF_dir_forcing              = '@PLUMBER2_ROOT@/Forcing/'

   DEF_forcing%dataset          = 'POINT'
   DEF_forcing%solarin_all_band = .true.
   DEF_forcing%HEIGHT_V         = 6.0
   DEF_forcing%HEIGHT_T         = 6.0
   DEF_forcing%HEIGHT_Q         = 6.0

   DEF_forcing%NVAR             = 8
   DEF_forcing%startyr          = 2008
   DEF_forcing%startmo          = 1
   DEF_forcing%endyr            = 2009
   DEF_forcing%endmo            = 12

   DEF_forcing%fprefix(1)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(2)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(3)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(4)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(5)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(6)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(7)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'
   DEF_forcing%fprefix(8)       = 'CN-Cng_2008-2009_FLUXNET2015_Met.nc'

! 槽位固定为 1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW。
! PLUMBER2 只有标量 Wind，故第 5 槽为 'NULL'，Wind 进第 6 槽。
   DEF_forcing%vname            = 'Tair' 'Qair' 'Psurf' 'Precip' 'NULL' 'Wind' 'SWdown' 'LWdown'
   DEF_forcing%tintalgo         = 'linear' 'linear' 'linear' 'nearest' 'NULL' 'linear' 'linear' 'linear'
/
```

- [ ] **Step 5: 写湿季算例**

用 `oracle/cases/CN-Cng-wet/case.nml`，内容与冬季完全相同，**只改这 5 行**：

```fortran
   DEF_CASE_NAME = 'CN-Cng-wet'
   DEF_simulation_time%start_month  = 7
   DEF_simulation_time%start_day    = 1
   DEF_simulation_time%end_month    = 7
   DEF_simulation_time%end_day      = 16
```

`SITE_fsitedata` 指向 `@CASE_DIR@/../CN-Cng/site.nc`（复用同一份站点文件）。
`forcing.nml.in` 与冬季逐字相同。

**窗口选择依据**：在 2008–2009 全期（总降水 665.7 mm）滑窗求 11 天累计降水最大值，
得 2008-07-05 起 101.0 mm。7-01 起跑给出 4 天预热余量。

- [ ] **Step 6: 写外部输入校验和清单**

```
# PLUMBER2 数据不入库（第三方数据集，15 MB + 2.1 MB）。
# golden-run 在使用前校验这些文件，防止「换了份数据然后黄金文件对不上」。
fef856bc4fde5025e32a6a5c69cb28ead33007c0e345bc0e66f1c3555849b02b  Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc
506e4b285ced83cebbca04bd6400aaea942391ec6a1a67570ec29f304ee0dfc9  Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc
393a035c09e90cb52a34cff6c51abd14ae78cae20d3f8bf95646df8f42cfea53  Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc
```

- [ ] **Step 7: 写 PROVENANCE.md**

```markdown
# oracle/cases/CN-Cng/site.nc 的来历

由 `oracle/scripts/make_site_nc.py` 从
`$PLUMBER2_ROOT/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc` 生成。
sha256: 6132cf1e56e57b01ec7129558eef5c51bb56cdf8c42d85f45bf7a49f3534f507

该脚本**逐字节可复现**（同一输入连跑两次得到同一 sha256，已实测），所以下面那条
「colm-srfdata 必须逐位重现本文件」的要求是可达成的 —— 但**前提是单次写完**。
注意：把同样的数据分多次追加进 NetCDF 会得到数据逐位相同、字节不同的文件
（HDF5 布局差异）。本 fixture 的首版就是那样建的，与脚本产出相差 51 个变量
数据全同、仅文件布局不同。若日后 sha256 对不上，先逐变量比对再怀疑数据。

PLUMBER2 的站点文件不足以驱动 CoLM 单点：`MOD_SingleSrfdata` 对每个字段做
`u_site_x = USE_SITE_x .and. ncio_var_exist(...)`，变量缺失时**没有第三条路**，
直接回落到全球 rawdata 树。本仓库不携带那几百 GB，故合成以下 12 个字段。

**每个字段在 NetCDF 里都带 `source` 属性，标明是合成值而非观测值。**

| 字段 | 值 | 出处 |
|---|---|---|
| `lakedepth` | 1.0 | `MOD_SingleSrfdata.F90:47` 模块默认值 |
| `elevation` | 138.0 | 同站 `Observation/*_Flux.nc` 的 `elevation` |
| `elvstd` | 0.0 | `MOD_SingleSrfdata.F90:88` 模块默认值 |
| `sloperatio` | 0.0 | `MOD_SingleSrfdata.F90:89` 模块默认值（平地） |
| `soil_s_v_alb` / `soil_d_v_alb` / `soil_s_n_alb` / `soil_d_n_alb` | 0.14 / 0.25 / 0.28 / 0.39 | `MOD_SoilColorRefl` 第 10 档 |
| `soil_vf_clay` / `soil_wf_clay` | 非砂/砾/有机质剩余量的 25% | 壤土 1:3 黏:粉假设 |
| `soil_wf_om` | `vf_om × OM_density / BD_all` | 由文件已有量推导 |
| `soil_texture` | 4（Silt loam） | USDA 三角，0–60 cm 深度加权：砂 14.3% / 粉 64.3% / 黏 21.4% |

未合成的字段及原因：
- `depth_to_bedrock` —— `DEF_USE_BEDROCK` 默认 `.false.`，不读
- 降尺度字段（`SITE_svf` / `SITE_cur` / `SITE_sf_lut` / `SITE_slp_type` /
  `SITE_asp_type` / `SITE_area_type`）—— `DEF_USE_Forcing_Downscaling` 默认 `.false.`

**Plan 2 的约束**：`colm-srfdata` 必须能逐位重现本文件，并有测试断言之。
`soil_wf_om` 的推导有一处未确证的语义分歧（`OM_density` 是否已是「每单位土体的
有机质质量」，若是则应为 `OM_density / BD_all` ≈ 0.0154 而非 0.0005）——
见 design.md §11 第 5 条。改动它会使黄金文件失效，必须同时更新黄金文件。
```

- [ ] **Step 8: 提交**

```bash
git add oracle/cases oracle/fixtures oracle/scripts
git commit -m "Add golden case inputs for the CN-Cng winter and wet windows"
```

---

## Task 5: 内核构建脚本

**Files:**
- Create: `oracle/scripts/build_kernel.sh`
- Create: `.gitmodules`（由 `git submodule add` 生成）

- [ ] **Step 1: 加 submodule**

从**本地路径**加 submodule 必须显式放开 `file` 传输：git 为 CVE-2022-39253 默认禁用了它，
不加这个选项会直接 `fatal: transport 'file' not allowed`（已实测）。

```bash
git -c protocol.file.allow=always submodule add \
    ~/Desktop/colm-rust/CoLM202X vendor/CoLM202X
git -C vendor/CoLM202X checkout 72dd76b9
git add .gitmodules vendor/CoLM202X
```

固定到具体 commit 而非跟随分支：黄金输出与内核版本是绑定的。

注意这个 `-c` 只作用于这一条命令。日后 `git submodule update --init` 若也从本地路径拉，
同样需要它；克隆到别处并从 GitHub 拉取时则不需要。

- [ ] **Step 2: 写构建脚本**

```bash
#!/usr/bin/env bash
# 从 vendor/CoLM202X 构建一个 SinglePoint 物理预设，并写出内核清单。
#
# colm.x 只接受一个参数（namelist 路径，getarg(1)），没有 --version。
# 因此版本握手靠构建期生成的 manifest.json + sha256，而不是问二进制。
set -euo pipefail

PRESET="${1:?usage: build_kernel.sh <waterheat|bgc|urban> [outdir]}"
OUTDIR="${2:-kernels}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$REPO_ROOT/vendor/CoLM202X"

case "$PRESET" in
  waterheat) ARGS=(SinglePoint LULC_IGBP     URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF) ;;
  bgc)       ARGS=(SinglePoint LULC_IGBP_PFT URBANOFF vanGenu CaMaOFF BGCON  CROPOFF TRACEROFF) ;;
  urban)     ARGS=(SinglePoint LULC_IGBP     URBANON  vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF) ;;
  *) echo "unknown preset: $PRESET" >&2; exit 2 ;;
esac

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) MAKEOPTS=Makeoptions.Mac-arm ;;
  Linux-*)      MAKEOPTS=Makeoptions.github  ;;
  *) echo "unsupported host; add a Makeoptions preset" >&2; exit 2 ;;
esac

BUILD="$REPO_ROOT/$OUTDIR/build-$PRESET"
rm -rf "$BUILD"
git -C "$SRC" worktree add --detach --force "$BUILD" HEAD >/dev/null
trap 'git -C "$SRC" worktree remove --force "$BUILD" >/dev/null 2>&1 || true' EXIT

cd "$BUILD"
ln -sf "$MAKEOPTS" include/Makeoptions
./.github/workflows/create_defineh.bash "${ARGS[@]}" >/dev/null

# FF=gfortran 而非 mpif90：SinglePoint 已 #undef USEMPI，用 mpif90 只会白链 4 个 MPI 库。
# 实测去掉后依赖只剩 netcdff/netcdf/LAPACK/BLAS/libgfortran/libgomp/libquadmath。
make FF="gfortran -fopenmp" mksrfdata.x mkinidata.x colm.x

DEST="$REPO_ROOT/$OUTDIR/$PRESET"
mkdir -p "$DEST"
cp run/mksrfdata.x run/mkinidata.x run/colm.x "$DEST/"

# 宏集合必须取**预处理后的生效集**，不能 grep define.h 的 #define 原文。
# 实测：原文 grep 会把 USEMPI / GridRiverLakeFlow / LATERAL_FLOW 都报成已定义，
# 而三者在 SinglePoint 下实际都是关闭的（前两个被 conflict 块 #undef，
# 第三个是个源码里根本不存在的宏名）。manifest 的全部作用是记录构建配置供
# Task 6 做版本握手，谎报 MPI 是开的会让这份记录反过来误导人。
printf '#include <define.h>\n' > "$BUILD/.macro_probe.F90"
# LC_ALL=C 不是装饰：裸 sort 用 locale 的字典序，en_US.UTF-8 下
# extend_interception 会排在 CoLMDEBUG 与 LULC_IGBP 之间，而 C locale 用字节序
# 把大写排在小写前。manifest 是版本握手的记录物，顺序随环境变意味着同一份构建
# 在不同机器上产出的 manifest 字节不同，任何「这两个内核配置一样吗」的比较都会假报差异。
MACROS=$(gfortran -E -dM -cpp -ffree-form -I include "$BUILD/.macro_probe.F90" 2>/dev/null \
  | awk '$1=="#define" && $2 !~ /^(_|__)/ && NF==2 {print "\""$2"\""}' | LC_ALL=C sort | paste -sd, -)
rm -f "$BUILD/.macro_probe.F90"
GIT_SHA=$(git -C "$SRC" rev-parse --short HEAD)
# macOS 有 shasum 没 sha256sum，多数 Linux 反之。两者都不通用，所以先探测。
if command -v shasum >/dev/null 2>&1; then
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  echo "need shasum or sha256sum on PATH" >&2; exit 2
fi

cat > "$DEST/manifest.json" <<JSON
{
  "schema": 1,
  "preset": "$PRESET",
  "platform": "$(uname -s)-$(uname -m)",
  "colm_git_sha": "$GIT_SHA",
  "generator_args": "${ARGS[*]}",
  "macros": [$MACROS],
  "built_with": "$(gfortran --version | head -1)",
  "netcdf_c": "$(nc-config --version 2>/dev/null)",
  "netcdf_fortran": "$(nf-config --version 2>/dev/null)",
  "hdf5": "$(H=$(nc-config --includedir 2>/dev/null)/H5public.h; [ -f "$H" ] && grep -hE '#define H5_VERS_(MAJOR|MINOR|RELEASE)' "$H" | awk '{printf "%s.", $3}' | sed 's/[.]$//')",
  "sha256": {
    "mksrfdata": "$(sha "$DEST/mksrfdata.x")",
    "mkinidata": "$(sha "$DEST/mkinidata.x")",
    "colm":      "$(sha "$DEST/colm.x")"
  }
}
JSON

echo "built $PRESET -> $DEST"
cat "$DEST/manifest.json"
```

- [ ] **Step 3: 跑一次，确认三个二进制都出来**

```bash
chmod +x oracle/scripts/build_kernel.sh
./oracle/scripts/build_kernel.sh waterheat
```

Expected: `built waterheat -> .../kernels/waterheat`，随后打印 manifest。

`macros` 数组必须**正好**是这 6 项，且顺序也必须一致（`LC_ALL=C` 的字节序，
大写在前）。顺序若是 `CoLMDEBUG, extend_interception, LULC_IGBP, ...`，
说明 `LC_ALL=C` 没生效，manifest 会随环境 locale 变化：

```json
["CoLMDEBUG","LULC_IGBP","RangeCheck","SinglePoint","extend_interception","vanGenuchten_Mualem_SOIL_MODEL"]
```

其中 `CoLMDEBUG` 的出现是好事而非意外：`create_defineh.bash` 默认发出它，
于是 `CoLMMAIN.F90:1545` 的 `|errore| > 0.5` W/m² 与 `:1620` 的 `|errorw| > 1.e-3` mm
平衡检查是**武装状态**（设计文档 §6.5 要的正是这个）。

**`macros` 里绝不能出现 `USEMPI`、`GridRiverLakeFlow` 或 `LATERAL_FLOW`。**
若出现，说明宏集合是 grep 原文得来的而不是预处理生效集，manifest 在谎报配置。

`manifest.json` 还必须是合法 JSON 且 `hdf5` 形如 `1.14.6`：

```bash
python3 -c "import json;d=json.load(open('kernels/waterheat/manifest.json'));print(d['hdf5'],d['netcdf_c'],len(d['macros']))"
```

如果 `make` 失败，先单独跑
`gfortran -fsyntax-only -cpp -ffree-form -I include vendor/CoLM202X/main/CoLM.F90`
判断是环境缺库还是别的问题。

**Fortran 构建不是逐字节可复现的**（实测：同一 `OUTDIR`、同一路径连跑两次，
三个二进制的 sha256 全不同；大概源于 `-g` 或 `.mod` 里的时间戳）。这决定了
manifest 里两组字段的分工，别搞混：

| 字段 | 可复现 | 用来回答 |
|---|---|---|
| `macros` / `colm_git_sha` / `generator_args` | 是 | **配置身份**：这是不是我要的那个预设 |
| `sha256` | 否，每次构建都变 | **完整性**：这三个二进制自它们的 manifest 写出以来有没有被换过 |

推论（都已体现在设计里，写出来免得日后有人误用）：
manifest 必须与二进制**一起**产生、一起存放，不能分开分发；
CI 不能缓存 `kernels/` 再拿一份入库的 manifest 去校验；
两个开发者构建同一预设会得到不同的 `sha256`，那不是错误。

- [ ] **Step 4: 提交**

```bash
git add .gitmodules vendor/CoLM202X oracle/scripts/build_kernel.sh
git commit -m "Build SinglePoint kernel presets from the pinned CoLM202X submodule"
```

---

## Task 6: golden-run —— 跑黄金算例

**Files:**
- Create: `oracle/Cargo.toml`
- Create: `oracle/src/bin/golden_run.rs`
- Modify: `Cargo.toml`（把 `oracle` 加回 workspace members）

- [ ] **Step 1: 写 `oracle/Cargo.toml`**

```toml
[package]
name = "oracle"
version.workspace = true
edition.workspace = true
rust-version.workspace = true   # 必须显式 opt-in
license.workspace = true
publish.workspace = true

[dependencies]
colm-kernel = { path = "../crates/colm-kernel" }
anyhow.workspace = true
sha2.workspace = true
# static 是无条件的，不做成可选 feature。三个平台走同一条构建路径。
# 实测依据（macOS ARM）：静态构建 45.5 秒，产物的动态依赖只剩 libiconv 与
# libSystem 两个系统库，清空所有环境变量后能直接读黄金文件。
# 而动态链接的构建虽然能编过，产出的二进制运行时报
# `Library not loaded: @rpath/libnetcdf.22.dylib ... no LC_RPATH's found`
# —— 开发构建自己都找不到 dylib，打包出去的程序更找不到。
netcdf = { workspace = true, features = ["static"] }
toml.workspace = true
serde = { workspace = true }

[lints]
workspace = true

# 显式声明，否则 cargo 会按文件名生成 `golden_run`（下划线），
# 而本计划所有命令用的是连字符名。
#
# 这里只声明 golden-run 一个。golden_compare.rs 要 Task 7 才写、
# tier_check.rs 要 Task 8 才写，提前声明会让 cargo 因源文件不存在而报错。
# 每个 Task 在写出自己的源文件时，再往这里追加对应的 [[bin]] 段。
[[bin]]
name = "golden-run"
path = "src/bin/golden_run.rs"
```

**首次构建会花约 45 秒**编译 HDF5 与 netcdf-c 源码，需要 `cmake` 与一个 C++ 编译器。
之后由 cargo 缓存。这是为「分发物自包含」付的代价，且它同时消除了系统上
conda 与 Homebrew 两份 netcdf 该用哪份的歧义。

- [ ] **Step 2: 写 golden_run.rs**

```rust
//! 跑一个黄金算例的三段，用 colm-kernel 判成败。
//!
//! 用法: golden-run <case-name> [--kernel <dir>] [--write-golden]
//!
//! 环境变量 PLUMBER2_ROOT 必须指向含 Forcing/ Sitedata/ Observation/ 的目录。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use colm_kernel::outcome::{adjudicate, Outcome, Stage};
use sha2::{Digest, Sha256};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let case = args
        .next()
        .context("usage: golden-run <case-name> [--kernel <dir>] [--write-golden]")?;
    let mut kernel = PathBuf::from("kernels/waterheat");
    let mut write_golden = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--kernel" => kernel = PathBuf::from(args.next().context("--kernel needs a path")?),
            "--write-golden" => write_golden = true,
            other => bail!("unknown argument: {other}"),
        }
    }

    let repo = repo_root()?;
    let plumber2 =
        PathBuf::from(std::env::var("PLUMBER2_ROOT").context("PLUMBER2_ROOT is not set")?);

    verify_inputs(&repo, &plumber2)?;
    verify_kernel(&repo.join(&kernel))?;

    let case_dir = repo.join("oracle/cases").join(&case);
    if !case_dir.is_dir() {
        bail!("no such case: {}", case_dir.display());
    }
    let work = repo.join("oracle/work").join(&case);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("out"))?;

    // namelist 模板展开。用 @NAME@ 而不是 shell 变量，是为了让模板本身
    // 就是一份合法的 namelist 文本，便于人眼审阅。
    let subst = |s: &str| -> String {
        s.replace("@PLUMBER2_ROOT@", plumber2.to_str().unwrap())
            .replace("@CASE_DIR@", case_dir.to_str().unwrap())
            .replace("@WORK_DIR@", work.to_str().unwrap())
    };
    fs::write(
        work.join("case.nml"),
        subst(&fs::read_to_string(case_dir.join("case.nml"))?),
    )?;
    fs::write(
        work.join("forcing.nml"),
        subst(&fs::read_to_string(case_dir.join("forcing.nml.in"))?),
    )?;

    let case_name = read_case_name(&work.join("case.nml"))?;
    let out = work.join("out").join(&case_name);

    let stages = [
        (Stage::MkSrfData, vec![out.join("landdata/srfdata.nc")]),
        (Stage::MkIniData, vec![out.join("restart/const")]),
        (Stage::Colm, vec![]), // history 文件名含日期，单独发现
    ];

    for (stage, artifacts) in &stages {
        let exe = repo.join(&kernel).join(format!("{}.x", stage.program()));
        let output = Command::new(&exe)
            .arg(work.join("case.nml"))
            .current_dir(&work)
            .output()
            .with_context(|| format!("failed to spawn {}", exe.display()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let log = work.join(format!("{}.log", stage.program()));
        fs::write(&log, stdout.as_bytes())?;

        let verdict = adjudicate(*stage, output.status.code(), &stdout, artifacts);
        match verdict {
            Outcome::Succeeded => println!("  {:<10} ok", stage.program()),
            Outcome::Failed(f) => {
                eprintln!("  {:<10} FAILED: {f:?}", stage.program());
                eprintln!("  log: {}", log.display());
                bail!("stage {} failed", stage.program());
            }
        }
    }

    // history 文件：唯一一个 *_hist_*.nc
    let hist_dir = out.join("history");
    let mut hists: Vec<PathBuf> = fs::read_dir(&hist_dir)
        .with_context(|| format!("no history dir at {}", hist_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_hist_") && n.ends_with(".nc"))
        })
        .collect();
    hists.sort();
    if hists.len() != 1 {
        bail!(
            "expected exactly one history file, found {}: {hists:?}",
            hists.len()
        );
    }
    let produced = &hists[0];
    println!("  history: {}", produced.display());

    let golden = repo
        .join("oracle/golden")
        .join(produced.file_name().unwrap());
    if write_golden {
        fs::create_dir_all(golden.parent().unwrap())?;
        fs::copy(produced, &golden)?;
        println!("  wrote golden: {}", golden.display());
    } else {
        println!(
            "  compare with: golden-compare {} {}",
            golden.display(),
            produced.display()
        );
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !out.status.success() {
        bail!("not inside a git repository");
    }
    Ok(PathBuf::from(String::from_utf8(out.stdout)?.trim()))
}

/// `DEF_CASE_NAME = 'X'` -> `X`。够用即止：Plan 2 的 colm-namelist 会做完整解析。
fn read_case_name(nml: &Path) -> Result<String> {
    let text = fs::read_to_string(nml)?;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("DEF_CASE_NAME") {
            if let Some(v) = rest.split('=').nth(1) {
                return Ok(v.trim().trim_matches('\'').trim_matches('"').to_string());
            }
        }
    }
    bail!("DEF_CASE_NAME not found in {}", nml.display())
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = fs::File::open(p).with_context(|| format!("cannot open {}", p.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 校验外部 PLUMBER2 文件。换了份数据就该在这里炸，而不是等黄金文件对不上。
fn verify_inputs(repo: &Path, plumber2: &Path) -> Result<()> {
    let manifest = fs::read_to_string(repo.join("oracle/fixtures/inputs.sha256"))?;
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (want, rel) = line.split_once("  ").context("malformed sha256 line")?;
        let path = plumber2.join(rel);
        let got = sha256_file(&path)?;
        if got != want {
            bail!(
                "input checksum mismatch for {}\n  expected {want}\n  got      {got}",
                path.display()
            );
        }
    }
    println!("  inputs verified");
    Ok(())
}

/// 校验内核。「不存在」和「存在但不是我们构建的那个」是两种不同的情况。
fn verify_kernel(dir: &Path) -> Result<()> {
    let manifest_path = dir.join("manifest.json");
    if !manifest_path.exists() {
        bail!(
            "no kernel manifest at {}\n  run: ./oracle/scripts/build_kernel.sh waterheat",
            manifest_path.display()
        );
    }
    let text = fs::read_to_string(&manifest_path)?;
    for prog in ["mksrfdata", "mkinidata", "colm"] {
        let exe = dir.join(format!("{prog}.x"));
        if !exe.exists() {
            bail!("kernel manifest present but {} is missing", exe.display());
        }
        let want = extract_json_string(&text, prog)
            .with_context(|| format!("manifest has no sha256 for {prog}"))?;
        let got = sha256_file(&exe)?;
        if got != want {
            bail!(
                "kernel binary {} does not match its manifest\n  expected {want}\n  got      {got}\n  rebuild with: ./oracle/scripts/build_kernel.sh",
                exe.display()
            );
        }
    }
    println!("  kernel verified ({})", dir.display());
    Ok(())
}

/// 从 manifest.json 里取 `"key": "value"`。刻意不引入 serde_json：
/// manifest 是我们自己按固定格式生成的，两行字符串查找足够，且少一个依赖。
fn extract_json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let q1 = rest.find('"')? + 1;
    let q2 = rest[q1..].find('"')? + q1;
    Some(rest[q1..q2].to_string())
}
```

- [ ] **Step 3: 把 oracle 加入 workspace 并编译**

先把根 `Cargo.toml` 的 members 改成 `["crates/colm-kernel", "oracle"]`。

Run: `cargo build -p oracle`
Expected: 首次约 45 秒（编译 HDF5 与 netcdf-c 源码），最终 `Finished`。

**这一步之后才是检查 lockfile 的时机**（Task 1 时检查不到，见那里的注释）：

Run: `python3 -c "import re;t=open('Cargo.lock').read();[print(*m) for m in re.findall(r'name = \"(netcdf[^\"]*|hdf5[^\"]*)\"\nversion = \"([^\"]+)\"',t)]"`
Expected: 出现 `netcdf` / `netcdf-sys` / `netcdf-src` / `hdf5-metno-sys` /
`hdf5-metno-src` 五项。**不要**期待特定版本号 —— 我们刻意不钉版本，
两个已知依赖图（`netcdf-sys` 0.9.0→HDF5 2.0.0、0.9.2→HDF5 2.2.0）都已实测
可静态构建并成功读取黄金文件。这一步要确认的是：这五项**出现在 lockfile 里**
（即 static feature 真的把它们拉进来了），并把实际版本记入提交说明，
使日后依赖图变化在 `Cargo.lock` 的 diff 里可见。
**不需要**设置 `NETCDF_DIR`、`HDF5_DIR` 或 `DYLD_LIBRARY_PATH` —— 静态构建
不依赖系统上的 netcdf。若失败，检查 `cmake --version` 与 C++ 编译器是否可用。

- [ ] **Step 4: 跑冬季算例并写出黄金文件**

```bash
export PLUMBER2_ROOT=~/Desktop/colm-rust/PLUMBER2s
cargo run -p oracle --bin golden-run -- CN-Cng --write-golden
```

Expected:
```
  inputs verified
  kernel verified (.../kernels/waterheat)
  mksrfdata  ok
  mkinidata  ok
  colm       ok
  history: .../out/CN-Cng/history/CN-Cng_hist_2008-01.nc
  wrote golden: .../oracle/golden/CN-Cng_hist_2008-01.nc
```

- [ ] **Step 5: 跑湿季算例**

```bash
cargo run -p oracle --bin golden-run -- CN-Cng-wet --write-golden
```

Expected: 同上，产出 `oracle/golden/CN-Cng-wet_hist_2008-07.nc`。

- [ ] **Step 6: 提交**

把 `oracle` 加进 workspace 必然改动 `Cargo.lock`（它现在第一次有了真实依赖），
所以要一并提交，否则 `git status --short` 不会是空的。

```bash
git add Cargo.toml Cargo.lock oracle/Cargo.toml oracle/src oracle/golden
git commit -m "Run the golden cases and record their history output"
```

提交说明里记下 lockfile 解析出的五个版本（`netcdf` / `netcdf-sys` / `netcdf-src` /
`hdf5-metno-sys` / `hdf5-metno-src`），使日后依赖图变化在历史里可追。

---

## Task 7: golden-compare —— NetCDF 判官

**关键实测事实**：黄金输出**不是**逐字节可复现的。重跑同一算例得到的文件有 8 字节差异，
全部来自全局属性 `create_time`（CoLM 写入的墙上时钟）。
**129 个变量的数据、所有维度、所有其他属性、所有变量级属性均逐位相同。**

因此判官比对变量 + 维度 + 属性，并把 `create_time` 放入**显式**易变白名单。
白名单必须是精确名单而非通配：新出现的不一致属性要能让测试失败。

**Files:**
- Create: `oracle/src/bin/golden_compare.rs`

- [ ] **Step 1: 写判官**

```rust
//! 比对两个 CoLM history 文件：变量数据、维度、属性。
//!
//! 用法: golden-compare <golden.nc> <produced.nc>
//!
//! 不做字节比对。实测重跑会产生 8 字节差异，全部来自全局属性 create_time
//! （CoLM 写入的墙上时钟）；129 个变量的数据逐位相同。

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

/// 允许不同的属性名。**精确名单，不做前缀或通配匹配。**
/// 新增条目必须说明为什么该属性天然易变。
const VOLATILE_ATTRIBUTES: &[&str] = &[
    // CoLM 写入的文件创建墙上时钟，例如 "20260817-16:16:27 UTC+08:00"
    "create_time",
];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let a_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced>")?,
    );
    let b_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced>")?,
    );

    let a = netcdf::open(&a_path).with_context(|| format!("cannot open {}", a_path.display()))?;
    let b = netcdf::open(&b_path).with_context(|| format!("cannot open {}", b_path.display()))?;

    let mut problems: Vec<String> = Vec::new();

    // --- 维度 ---
    let dims_a: BTreeSet<(String, usize)> = a.dimensions().map(|d| (d.name(), d.len())).collect();
    let dims_b: BTreeSet<(String, usize)> = b.dimensions().map(|d| (d.name(), d.len())).collect();
    for d in dims_a.difference(&dims_b) {
        problems.push(format!("dimension only in golden: {d:?}"));
    }
    for d in dims_b.difference(&dims_a) {
        problems.push(format!("dimension only in produced: {d:?}"));
    }

    // --- 全局属性 ---
    compare_attrs(
        "global",
        &a.attributes()
            .map(|x| x.name().to_string())
            .collect::<Vec<_>>(),
        &b.attributes()
            .map(|x| x.name().to_string())
            .collect::<Vec<_>>(),
        |n| a.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
        |n| b.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
        &mut problems,
    );

    // --- 变量 ---
    let names_a: BTreeSet<String> = a.variables().map(|v| v.name()).collect();
    let names_b: BTreeSet<String> = b.variables().map(|v| v.name()).collect();
    for n in names_a.difference(&names_b) {
        problems.push(format!("variable only in golden: {n}"));
    }
    for n in names_b.difference(&names_a) {
        problems.push(format!("variable only in produced: {n}"));
    }

    let mut compared = 0usize;
    for name in names_a.intersection(&names_b) {
        let va = a.variable(name).unwrap();
        let vb = b.variable(name).unwrap();

        if va.dimensions().len() != vb.dimensions().len() {
            problems.push(format!("{name}: rank differs"));
            continue;
        }

        // 全部按 f64 读出后逐位比较。NaN 视为相等（两边都 NaN 才算相等）。
        let xa: Vec<f64> = va
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read golden {name}"))?;
        let xb: Vec<f64> = vb
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read produced {name}"))?;
        if xa.len() != xb.len() {
            problems.push(format!("{name}: length {} vs {}", xa.len(), xb.len()));
            continue;
        }
        // 变量级属性也要比。units / long_name / missing_value 是文件契约的一部分，
        // 下游的评估与绘图都依赖它们；只比全局属性会让这类回归静默通过。
        compare_attrs(
            name,
            &va.attributes()
                .map(|x| x.name().to_string())
                .collect::<Vec<_>>(),
            &vb.attributes()
                .map(|x| x.name().to_string())
                .collect::<Vec<_>>(),
            |n| va.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
            |n| vb.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
            &mut problems,
        );

        let mut first_bad: Option<(usize, f64, f64)> = None;
        let mut n_bad = 0usize;
        for (i, (p, q)) in xa.iter().zip(xb.iter()).enumerate() {
            let same = (p.is_nan() && q.is_nan()) || p.to_bits() == q.to_bits();
            if !same {
                n_bad += 1;
                if first_bad.is_none() {
                    first_bad = Some((i, *p, *q));
                }
            }
        }
        if let Some((i, p, q)) = first_bad {
            problems.push(format!(
                "{name}: {n_bad}/{} values differ; first at index {i}: {p:?} vs {q:?}",
                xa.len()
            ));
        }
        compared += 1;
    }

    if problems.is_empty() {
        println!(
            "identical: {compared} variables, {} dimensions (ignoring {:?})",
            dims_a.len(),
            VOLATILE_ATTRIBUTES
        );
        return Ok(());
    }
    eprintln!("{} problem(s):", problems.len());
    for p in &problems {
        eprintln!("  {p}");
    }
    bail!("golden comparison failed");
}

fn compare_attrs<FA, FB>(
    scope: &str,
    names_a: &[String],
    names_b: &[String],
    get_a: FA,
    get_b: FB,
    problems: &mut Vec<String>,
) where
    FA: Fn(&str) -> Option<String>,
    FB: Fn(&str) -> Option<String>,
{
    let sa: BTreeSet<&String> = names_a.iter().collect();
    let sb: BTreeSet<&String> = names_b.iter().collect();
    for n in sa.symmetric_difference(&sb) {
        problems.push(format!("{scope} attribute present on only one side: {n}"));
    }
    for n in sa.intersection(&sb) {
        if VOLATILE_ATTRIBUTES.contains(&n.as_str()) {
            continue;
        }
        let (x, y) = (get_a(n), get_b(n));
        if x != y {
            problems.push(format!("{scope} attribute {n}: {x:?} vs {y:?}"));
        }
    }
}

fn fmt_attr(v: netcdf::AttributeValue) -> String {
    format!("{v:?}")
}
```

- [ ] **Step 2: 在 `oracle/Cargo.toml` 追加这个 bin 的声明**

```toml
[[bin]]
name = "golden-compare"
path = "src/bin/golden_compare.rs"
```

- [ ] **Step 3: 编译**

Run: `cargo build -p oracle --bin golden-compare`
Expected: 通过。本文件用到的 netcdf API 名已在 0.12.0 上实测编译通过：
`netcdf::open` / `.dimensions()` / `.attributes()` / `.attribute(n)` / `.value()`
/ `netcdf::AttributeValue` / `.variables()` / `.variable(n)` / `.name()`
/ `.get_values::<f64, _>(netcdf::Extents::All)`。
另已实测：黄金文件的 **129/129 个变量都能按 `f64` 读出**（含 8 个 `int` 变量），
且 `create_time` 读出为 `Str("20260817-16:27:52 UTC+08:00")`。
若仍有 API 不符，以 `cargo doc -p netcdf --open` 为准调整，
**不要**改成逐字节比对。

- [ ] **Step 4: 自比对必须通过**

```bash
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc oracle/golden/CN-Cng_hist_2008-01.nc
```

Expected: `identical: 129 variables, 10 dimensions (ignoring ["create_time"])`

- [ ] **Step 5: 重跑再比对 —— 这一步验证「重跑可复现」这个前提本身**

```bash
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

Expected: `identical: 129 variables, ...`
若报告 `global attribute create_time: ...` 之类，说明白名单没生效——修白名单逻辑，
**不要**把整个属性比较关掉。

- [ ] **Step 6: 负向测试 —— 判官必须能抓到真实差异**

一个只会说「相同」的判官比没有判官更糟（EarthMesh 的 grep gate 就是这么
「通过了好几个月却什么都没检查」）。四种情形都要验，**且退出码要单独取**——
接了管道之后 `$?` 是管道末端命令的退出码，不是判官的。

```bash
G=oracle/golden/CN-Cng_hist_2008-01.nc
B() { cargo run -q -p oracle --bin golden-compare -- "$@"; }

# (a) 数值被改 -> 必须报错
cp "$G" /tmp/tampered.nc
python3 -c "
import netCDF4 as nc
d = nc.Dataset('/tmp/tampered.nc','a'); d['f_fsena'][0] = d['f_fsena'][0]+1.0; d.close()"
B "$G" /tmp/tampered.nc; echo "rc=$?"

# (b) 变量属性被改 -> 必须报错（units 是文件契约，下游评估与绘图依赖它）
cp "$G" /tmp/attrchg.nc
python3 -c "
import netCDF4 as nc
d = nc.Dataset('/tmp/attrchg.nc','a'); d['f_fsena'].setncattr('units','BOGUS'); d.close()"
B "$G" /tmp/attrchg.nc; echo "rc=$?"

# (c) 只有 create_time 不同 -> 必须仍报相同（白名单生效）
cp "$G" /tmp/timeonly.nc
python3 -c "
import netCDF4 as nc
d = nc.Dataset('/tmp/timeonly.nc','a')
d.setncattr('create_time','19700101-00:00:00 UTC+00:00'); d.close()"
B "$G" /tmp/timeonly.nc; echo "rc=$?"

# (d) 文件不存在 -> 必须报错，而不是当成"没有差异"
B /nonexistent.nc /nonexistent.nc; echo "rc=$?"
```

Expected（全部实测过）：

| 情形 | rc | 输出 |
|---|---|---|
| (a) 改数值 | 1 | `f_fsena: 1/264 values differ; first at index 0: 736.23… vs 737.23…` |
| (b) 改变量 units | 1 | `f_fsena attribute units: Some("Str(\"W/m2\")") vs Some("Str(\"BOGUS\")")` |
| (c) 只改 create_time | **0** | `identical: 129 variables, 10 dimensions (ignoring ["create_time"])` |
| (d) 文件不存在 | 1 | `cannot open /nonexistent.nc` |

(b) 尤其重要：判官的首版**只比全局属性、不比变量级属性**，改 `units` 会静默通过。
是这条负向测试把它揪出来的。

- [ ] **Step 7: 提交**

Step 2 改了 `oracle/Cargo.toml`，要一并提交。

```bash
git add oracle/Cargo.toml oracle/src/bin/golden_compare.rs
git commit -m "Judge golden output on variables and attributes, not on bytes"
```

---

## Task 8: tolerances.toml 与完备性测试

设计文档 §8.1 定了四层容差分级。本任务**不实现容差比较**（里程碑 1 只需要逐位），
但把分级本身机读化并加一个完备性测试。理由：Tier 归属是现在掌握的知识，
不落盘就会丢失；而完备性测试能在 history 变量增减时立刻提醒补分类。

**Files:**
- Create: `oracle/tolerances.toml`
- Create: `oracle/src/bin/tier_check.rs`

- [ ] **Step 1: 写分级文件**

```toml
# CoLM history 变量的容差分级。见 docs/design.md §8.1。
#
# 里程碑 1 只做逐位比较，本文件此时不参与比较，只被 tier-check 校验完备性。
# 计划 3 的 colm-hist 与 C 阶段的 Rust-vs-Fortran 对账会真正消费它。
#
# 禁止在测试里内联容差魔数 —— EarthMesh 就是在两个测试里对同一个网格
# 用了 2.0e-6 和 2.0e-4。

[tier0]
description = "纯函数、无迭代、无收敛。任何差异都是 bug。"
rule = "bitwise"
variables = [
  "time", "lat", "lon",
  # 维度坐标：常量整数索引，不参与任何计算
  "band", "lake", "rtyp", "soil", "soilinterface", "soilsnow", "vegnodes",
  # 未启用的用户自定义诊断槽：两个黄金文件里全是 -1e36 的 missing_value
  "f_sensors",
  "f_lai", "f_sai", "f_green", "f_sigf", "f_z0m",
  "f_xy_t", "f_xy_q", "f_xy_pbot", "f_xy_us", "f_xy_vs",
  "f_xy_solarin", "f_xy_frl", "f_xy_prc", "f_xy_prl",
  "f_xy_rain", "f_xy_snow",
]

[tier1]
description = "确定性代数、无迭代物理。"
rule = "relative"
rtol = 1e-12
variables = [
  "f_solvd", "f_solvi", "f_solnd", "f_solni",
  "f_solvdln", "f_solviln", "f_solndln", "f_solniln",
  "f_srvd", "f_srvi", "f_srnd", "f_srni",
  "f_srvdln", "f_srviln", "f_srndln", "f_srniln",
  "f_sr", "f_alb", "f_sabg", "f_sabvsun", "f_sabvsha",
  "f_olrg", "f_rnet", "f_emis", "f_trad",
  "f_fm", "f_fh", "f_fq", "f_fm10m",
  "f_snowdp", "f_fsno", "f_scv",
]

[tier2]
description = """
含迭代收敛的模块。容差不得紧于求解器自身的收敛容差。
比较时必须同时报告迭代计数与回退次数（如 VSF implicit/explicit 计数）；
回退次数变化即为红旗，即使数值落在容差内。
"""
rule = "absolute_and_relative"
atol = 1e-7
rtol = 1e-7
solver_tolerance_floor = 8e-8   # MOD_Hydro_SoilWater 的 Newton 容差
variables = [
  "f_fsena", "f_fseng", "f_fsenl",
  "f_lfevpa", "f_fevpa", "f_fevpg", "f_fevpl",
  "f_etr", "f_etrsun", "f_etrsha",
  "f_assim", "f_assimsun", "f_assimsha",
  "f_gssun", "f_gssha", "f_rstfacsun", "f_rstfacsha",
  "f_tleaf", "f_t_grnd", "f_tref", "f_qref",
  "f_fgrnd", "f_taux", "f_tauy", "f_ustar", "f_ustar2",
  "f_tstar", "f_qstar", "f_rib", "f_zol", "f_us10m", "f_vs10m",
  "f_zwt", "f_wat", "f_wat_inst", "f_wa", "f_wa_inst",
  # 多层状态量：全部来自隐式列求解或相变，逐层值依赖迭代细节
  "f_h2osoi", "f_t_soisno", "f_wice_soisno", "f_wliq_soisno",
  "f_qlayer", "f_rootr", "f_vegwp",
  "f_t_lake", "f_lake_icefrac", "f_lake_deficit",
  "f_rnof", "f_rsur", "f_rsub", "f_rsur_ie", "f_rsur_se",
  "f_qinfl", "f_qdrip", "f_qintr", "f_ldew",
  "f_frcsat", "f_rss", "f_respc",
  "f_wdsrf", "f_wdsrf_inst", "f_wetwat", "f_wetwat_inst", "f_wetzwt",
  "f_o3uptakesun", "f_o3uptakesha",
  "f_laisun", "f_laisha",
  "f_xerr", "f_zerr",
]

[tier3]
description = "整场统计等价。判据挂在 design.md §2.8/§2.8b 的实测基线上。"
rule = "statistical"
variables = []          # 逐变量判据由 colm-hist 在计划 3 中定义

[tier3.baselines]
# 湿季窗口 CN-Cng-wet，剔除前 4 天预热，观测仅 qc==0
rnet_r2_min = 0.999
qle_r2_min  = 0.85
```

- [ ] **Step 2: 写完备性检查**

```rust
//! 校验 oracle/tolerances.toml 覆盖了黄金文件里的每一个变量。
//!
//! 用法: tier-check <golden.nc> [<golden.nc> ...]
//!
//! 这不是容差比较器（里程碑 1 只做逐位）。它保证 CoLM 增删 history 变量时
//! 分类不会静默变得不完整。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct Tolerances {
    tier0: Tier,
    tier1: Tier,
    tier2: Tier,
    tier3: Tier,
}

#[derive(Deserialize)]
struct Tier {
    variables: Vec<String>,
}

fn main() -> Result<()> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        bail!("usage: tier-check <golden.nc> [...]");
    }
    let text = std::fs::read_to_string("oracle/tolerances.toml")
        .context("run from the repository root")?;
    let t: Tolerances = toml::from_str(&text)?;

    let mut assigned: BTreeMap<String, &str> = BTreeMap::new();
    let mut duplicates = Vec::new();
    for (tier, list) in [
        ("tier0", &t.tier0.variables),
        ("tier1", &t.tier1.variables),
        ("tier2", &t.tier2.variables),
        ("tier3", &t.tier3.variables),
    ] {
        for v in list {
            if let Some(prev) = assigned.insert(v.clone(), tier) {
                duplicates.push(format!("{v} in both {prev} and {tier}"));
            }
        }
    }

    let mut present: BTreeSet<String> = BTreeSet::new();
    for f in &files {
        let nc = netcdf::open(f).with_context(|| format!("cannot open {f}"))?;
        for v in nc.variables() {
            present.insert(v.name());
        }
    }

    let unclassified: Vec<&String> = present
        .iter()
        .filter(|v| !assigned.contains_key(*v))
        .collect();
    let stale: Vec<&String> = assigned.keys().filter(|v| !present.contains(*v)).collect();

    let mut bad = false;
    if !duplicates.is_empty() {
        eprintln!("variables assigned to more than one tier:");
        for d in &duplicates {
            eprintln!("  {d}");
        }
        bad = true;
    }
    if !unclassified.is_empty() {
        eprintln!(
            "{} variable(s) in the golden files have no tier assignment:",
            unclassified.len()
        );
        for v in &unclassified {
            eprintln!("  {v}");
        }
        eprintln!("add each to a tier in oracle/tolerances.toml (see design.md §8.1)");
        bad = true;
    }
    if !stale.is_empty() {
        eprintln!(
            "{} tier entry/entries name variables that no longer exist:",
            stale.len()
        );
        for v in &stale {
            eprintln!("  {v}");
        }
        bad = true;
    }
    if bad {
        bail!("tolerance classification is incomplete");
    }
    println!(
        "all {} golden variables have a tier assignment",
        present.len()
    );
    Ok(())
}
```

- [ ] **Step 2b: 在 `oracle/Cargo.toml` 追加这个 bin 的声明**

```toml
[[bin]]
name = "tier-check"
path = "src/bin/tier_check.rs"
```

- [ ] **Step 3: 跑一遍，应当一次报告完备**

```bash
cargo run -p oracle --bin tier-check -- \
  oracle/golden/CN-Cng_hist_2008-01.nc oracle/golden/CN-Cng-wet_hist_2008-07.nc
```

Expected: `all 129 golden variables have a tier assignment`，退出码 0。

上面的分类表是拿这两个黄金文件实测补全过的：首版漏了 19 个，已按下述依据补齐 ——

| 补入 | 变量 | 依据 |
|---|---|---|
| tier0 | `band` `lake` `rtyp` `soil` `soilinterface` `soilsnow` `vegnodes` | 维度坐标，常量整数索引（实测 `band=[1,2]`、`soil=[1..10]`） |
| tier0 | `f_sensors` | 未启用的用户自定义诊断槽，两文件中全为 `-1e36` |
| tier1 | `f_alb` | 反照率，与已在 tier1 的 `f_sr`/`f_sabg` 同属确定性辐射代数 |
| tier2 | `f_h2osoi` `f_t_soisno` `f_wice_soisno` `f_wliq_soisno` `f_qlayer` `f_rootr` `f_vegwp` `f_t_lake` `f_lake_icefrac` `f_lake_deficit` | 多层状态量，来自隐式列求解或相变，逐层值依赖迭代细节 |

**若它仍报未分类**，把报出来的变量按同样依据归类：含迭代/收敛来源的进 tier2，
纯粹由强迫场或几何直接得出的进 tier0/tier1。**不要为了让它过而全塞进 tier2** ——
那等于放弃 Tier 0/1 的严格约束，而那正是分层的全部价值。

- [ ] **Step 4: 提交**

```bash
git add oracle/tolerances.toml oracle/src/bin/tier_check.rs
git commit -m "Record the tolerance tiers and assert they cover every golden variable"
```

---

## Task 9: CI

**Files:**
- Create: `.github/workflows/ci.yml`

CI 跑不了黄金算例——PLUMBER2 数据不在库里。所以 CI 分两层：
**每次 PR 都能跑的**（编译、单测、clippy、netcdf crate 在三平台可用），
和**需要数据、只能本地或自托管 runner 跑的**（黄金回归）。

**这个界限必须写在 CI 输出里**，否则「CI 全绿」会被误读成「黄金回归通过了」。

- [ ] **Step 1: 写 workflow**

```yaml
name: ci

on:
  pull_request:
  push:
    branches: [master]

jobs:
  # 每个 PR 都跑：纯 Rust 侧
  rust:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
        with:
          submodules: false      # 内核只在黄金作业里才需要

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      # 缓存尤其重要：没有它每个作业都要重新从源码编译 HDF5 与 netcdf-c。
      - uses: Swatinem/rust-cache@v2

      # netcdf 是本方案最大的依赖风险点，所以三平台都编一次 —— 为的是让
      # Windows 上的失败在第一天暴露，而不是等到里程碑 9。
      #
      # 三个平台走完全相同的一条命令：netcdf 静态链接是无条件的，
      # 不装任何系统 netcdf，也不设 NETCDF_DIR / HDF5_DIR。
      # 需要 cmake 与 C++ 编译器，三个 GitHub runner 都自带。
      # 首次构建约多花 45 秒编译 HDF5 与 netcdf-c 源码；cargo 会缓存。
      - name: Build
        run: cargo build --workspace --all-targets

      - name: Test
        run: cargo test --workspace

      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Format
        run: cargo fmt --all --check

      # 下面两条只读已入库的黄金文件与 tolerances.toml，不需要 PLUMBER2 数据、
      # 也不需要 Fortran 内核，所以能在每个 PR、三个平台上跑。
      #
      # tier-check 若只放在 golden 作业里，就会被 HAS_PLUMBER2 与自托管 runner
      # 双重门控，实际上几乎永不运行 —— 而它要防的正是"有人加了 history 变量却
      # 忘了给它定 tier"，那种改动在任何 PR 里都会发生。
      - name: Tolerance classification is complete
        run: cargo run -p oracle --bin tier-check -- oracle/golden/CN-Cng_hist_2008-01.nc oracle/golden/CN-Cng-wet_hist_2008-07.nc

      # 判官自比对在这里的价值不是"证明两个相同文件相同"，而是证明**静态链接的
      # netcdf 在这个平台上真的能读一个真实 CoLM 文件**，而不只是链接通过。
      # Windows 是本方案的头号风险点，这条让它在第一天就被覆盖。
      - name: Judge can read a real CoLM file on this platform
        run: |
          cargo run -p oracle --bin golden-compare -- oracle/golden/CN-Cng_hist_2008-01.nc oracle/golden/CN-Cng_hist_2008-01.nc
          cargo run -p oracle --bin golden-compare -- oracle/golden/CN-Cng-wet_hist_2008-07.nc oracle/golden/CN-Cng-wet_hist_2008-07.nc

  # 黄金回归：需要 PLUMBER2 数据与 Fortran 工具链，只在带该数据的 runner 上跑。
  golden:
    if: ${{ vars.HAS_PLUMBER2 == 'true' }}
    runs-on: [self-hosted, plumber2]
    steps:
      # submodule 目前指向本机绝对路径（见 Task 5 Step 1），所以这一步只在
      # 那个路径存在的自托管 runner 上成立，且同样需要放开 file 传输。
      # 待 CoLM202X 有可用的远端后，把 .gitmodules 的 url 改成 GitHub 地址，
      # 这两条限制会一起消失。
      - uses: actions/checkout@v4
        with:
          submodules: false
      - name: Init submodule from its local path
        run: git -c protocol.file.allow=always submodule update --init --recursive

      - uses: dtolnay/rust-toolchain@stable

      - name: Build kernel
        run: ./oracle/scripts/build_kernel.sh waterheat

      - name: Golden regression (both windows)
        env:
          PLUMBER2_ROOT: ${{ vars.PLUMBER2_ROOT }}
        run: |
          set -euo pipefail
          for case in CN-Cng CN-Cng-wet; do
            cargo run -p oracle --bin golden-run -- "$case"
          done
          cargo run -p oracle --bin golden-compare -- \
            oracle/golden/CN-Cng_hist_2008-01.nc \
            oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
          cargo run -p oracle --bin golden-compare -- \
            oracle/golden/CN-Cng-wet_hist_2008-07.nc \
            oracle/work/CN-Cng-wet/out/CN-Cng-wet/history/CN-Cng-wet_hist_2008-07.nc
          cargo run -p oracle --bin tier-check -- oracle/golden/*.nc

  # 让「黄金回归没跑」这件事在 PR 界面上可见，而不是静默缺席。
  golden-status:
    runs-on: ubuntu-latest
    needs: rust
    steps:
      - name: Report golden-regression coverage
        run: |
          if [ "${{ vars.HAS_PLUMBER2 }}" = "true" ]; then
            echo "Golden regression ran on the self-hosted runner."
          else
            echo "::warning::Golden regression was NOT run: no PLUMBER2 data on this runner."
            echo "::warning::Green CI here does not mean the numerics are unchanged."
          fi
```

- [ ] **Step 2: 本地模拟 rust 作业**

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Expected: 全部通过。`cargo fmt --all --check` 若报格式差异，跑 `cargo fmt --all` 再提交。

- [ ] **Step 3: 提交**

```bash
git add .github/workflows/ci.yml
git commit -m "Add CI and make the absence of golden coverage visible"
```

---

## Task 10: 文档与收尾

**Files:**
- Create: `README.md`
- Create: `docs/design.md`（从 CoLM202X 复制）
- Create: `docs/plan-m0-m1.md`（本文档）

- [ ] **Step 1: 复制设计文档与计划**

```bash
cp ~/Desktop/colm-rust/CoLM202X/docs/colm-desktop-design.md docs/design.md
cp ~/Desktop/colm-rust/CoLM202X/docs/colm-desktop-plan-m0-m1.md docs/plan-m0-m1.md
```

- [ ] **Step 2: 写 README**

```markdown
# colm-desktop

把 CoLM202X 的 SinglePoint 模式做成跨平台桌面程序。设计见 `docs/design.md`。

**当前状态**：里程碑 0–1。仓库骨架 + 成败判定 + 黄金输出回归基准。
还没有 GUI，也还没有编排层。

## 为什么有 `crates/colm-kernel/src/outcome.rs`

CoLM 在单点模式下，**成功与失败都以退出码 0 结束，但走的是两条不同的路**：

- 失败走 `share/MOD_SPMD_Task.F90` 的 `CoLM_stop`，其 `#ifndef USEMPI` 分支是裸 `STOP`。
- 成功不执行任何收尾调用，直接跑到 `main/CoLM.F90:764` 的 `END PROGRAM CoLM`
  （`spmd_exit` 只定义并调用于 `#ifdef USEMPI` 内）。

退出码相同是两条路径的巧合，不是共用一条路径。所以判定成败必须同时满足三件事：
无错误标记、有正向成功标记、产物齐全。

附带结论：既然 `CoLM_stop` 是失败专用的，把那个裸 `STOP` 改成 `STOP 1`
是安全的上游修复。即便上游改了，本模块仍然必要 —— 产物硬校验能抓住
「跑完了但没写出该写的文件」，错误标记扫描能抓住部分失败。

## 跑黄金回归

需要 PLUMBER2 数据（不入库）与 gfortran + netcdf-fortran。

```bash
export PLUMBER2_ROOT=/path/to/PLUMBER2s
./oracle/scripts/build_kernel.sh waterheat
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

## 两个窗口，以及它们各自不覆盖什么

| 算例 | 窗口 | 覆盖 | 不覆盖 |
|---|---|---|---|
| `CN-Cng` | 2008-01-01 → 01-11 | 冻结土壤、雪、辐射 | 产流与入渗（窗口内无降水） |
| `CN-Cng-wet` | 2008-07-01 → 07-16 | 饱和超渗产流、入渗、地下水位动态 | `f_rsur_ie`（超渗产流）、`f_rsub`（地下产流）—— 两个窗口都为 0 |

在 `f_rsur_ie` 与 `f_rsub` 被覆盖之前，不得声称产流模块已验证。
```

- [ ] **Step 3: 全套验证跑一遍**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p oracle --bin tier-check -- oracle/golden/*.nc
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
cargo run -p oracle --bin golden-run -- CN-Cng-wet
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng-wet_hist_2008-07.nc \
  oracle/work/CN-Cng-wet/out/CN-Cng-wet/history/CN-Cng-wet_hist_2008-07.nc
git diff --check
```

Expected: 全部通过；两次 `golden-compare` 都报 `identical: ... variables`。

- [ ] **Step 4: 提交**

```bash
git add README.md docs/
git commit -m "Document the milestone 0-1 state and the two golden windows"
```

---

## 完成判据

里程碑 0–1 达成的条件，逐条可验证：

- [ ] `cargo test --workspace` 通过，且 `colm-kernel` 的 11 个判定测试全部执行（不是跳过）
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 无输出
- [ ] `cargo fmt --all --check` 无输出（计划里的代码块已是 rustfmt 格式）
- [ ] `netcdf` crate 在 Linux / macOS / Windows 三个平台上都能编（CI 的 `rust` 作业全绿）
- [ ] `./oracle/scripts/build_kernel.sh waterheat` 产出三个二进制加 manifest，
      manifest 的 `macros` 含 `SinglePoint`
- [ ] `golden-run CN-Cng` 与 `golden-run CN-Cng-wet` 三段全部报 `ok`
- [ ] 两个窗口重跑后 `golden-compare` 均报 `identical`
- [ ] 篡改一个数值后 `golden-compare` 非零退出并指出变量与索引（负向测试）
- [ ] `tier-check` 报告黄金文件的每个变量都有 Tier 归属
- [ ] CI 在没有 PLUMBER2 数据时**显式警告**黄金回归未运行
- [ ] `git status --short` 在每个 Task 结束后都是空输出（`Cargo.lock` 已入库，见 Task 1 Step 8）

---

## 交给计划 2 的东西

- `oracle/cases/CN-Cng/site.nc` 是 `colm-srfdata` 的验收目标：
  它必须能逐位重现这个文件。
- `oracle/scripts/make_site_nc.py` 是 `colm-srfdata` 的参考实现，含 USDA 质地分类器。
  `colm-srfdata` 落地后删除本脚本，并把它的测试改为对比黄金站点文件。
- `oracle/cases/*/case.nml` 里每一处「偏离 CoLM 默认值」的注释是 `colm-schema` 的
  需求清单：GUI 的默认值必须与 CoLM 的默认值不同，且必须能解释为什么。
- `crates/colm-kernel/src/outcome.rs` 的三件套是计划 3 编排层的判定内核，不要重写。
