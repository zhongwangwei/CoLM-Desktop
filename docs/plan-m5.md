# 里程碑 5 实施计划：colm-kernel 的编排层

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把三段编排、内核清单握手、静默覆盖回报收进 `colm-kernel`，让 `oracle` 的黄金脚本退化成一层薄壳，也让将来的 GUI 有一个不必重写的编排入口。

**Architecture:** `colm-kernel` 现在只有 `outcome.rs`（成败判定）。本轮加三个模块：`manifest.rs` 读并校验构建期清单，`overrides.rs` 从日志里抽出 CoLM 的静默覆盖，`run.rs` 把「跑一段 → 收日志 → 判成败 → 抽覆盖」合成一个函数。`golden_run.rs` 里 344 行中的编排部分搬过去。

**Tech Stack:** Rust 2021、`anyhow`、`sha2`、`serde` + `serde_json`（新增，理由见下）、`colm-namelist`（本仓库）。

---

## 已实测的事实基础

### 现状：编排逻辑住在一个二进制里

`oracle/src/bin/golden_run.rs` 有 344 行，其中可复用的部分是：

| 现有函数 | 职责 | 本轮去向 |
|---|---|---|
| `verify_kernel` | 三个可执行文件在不在 | `colm_kernel::Kernel::open` |
| `check_kernel_provenance` | 读 manifest.json 比对字段 | `colm_kernel::manifest` |
| `sha256_file` | 算校验和 | 同上 |
| `extract_json_string` / `extract_json_array` | **手写 JSON 提取** | 删除，见下 |
| `read_case_name` | **手写 namelist 解析** | 改用 `colm-namelist` |
| 三段循环 + `adjudicate` | 编排 | `colm_kernel::run_stage` |

### 手写 JSON 提取有一个潜伏的 bug

```rust
fn extract_json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let q1 = rest.find('"')? + 1;       // ← 找到的是**下一个键名**的引号
    let q2 = rest[q1..].find('"')? + q1;
    Some(rest[q1..q2].to_string())
}
```

对 `"sha256": { "mksrfdata": "…" }` 调用它会返回 `mksrfdata` —— 键名而不是值。
它至今没炸，只是因为从没这么调用过。没有转义处理，也不认嵌套。

**所以本轮引入 `serde` + `serde_json`。** 这与里程碑 4 拒绝 chrono 不是同一件事：
那里要的是两个函数，这里要的是一个正确的 JSON 解析器，而手写它比引入它更糟。
两者都是纯 Rust，不含 C 依赖，不威胁三平台静态链接。

### 清单的实际形态（本机实测）

```json
{
  "schema": 1,
  "preset": "waterheat",
  "platform": "Darwin-arm64",
  "colm_git_sha": "72dd76b9",
  "generator_args": "SinglePoint LULC_IGBP URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF",
  "macros": ["CoLMDEBUG","LULC_IGBP","RangeCheck","SinglePoint","extend_interception","vanGenuchten_Mualem_SOIL_MODEL"],
  "built_with": "GNU Fortran (Homebrew GCC 16.1.0) 16.1.0",
  "netcdf_c": "netCDF 4.9.3", "netcdf_fortran": "4.6.3", "hdf5": "1.14.6",
  "sha256": { "mksrfdata": "…", "mkinidata": "…", "colm": "…" }
}
```

两组字段职责不同（design.md §6.1）：`macros` / `colm_git_sha` / `generator_args`
可复现，认定**配置身份**；`sha256` 每次构建都变（Fortran 构建不逐字节可复现，
实测同一路径连跑两次三个二进制的 sha256 全不同），只认定**完整性** ——
二进制自其清单写出以来未被替换。

**「不存在」与「存在但对不上」必须是两种不同的报错**（§6.1）。

### 静默覆盖：实测 9 种，前缀不统一

跑一次 CN-Cng，三个阶段的日志里共出现 9 种不同消息：

```
Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.
Note: Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme.
Warning: Nitrification-Denitrification is on when BGC is off.
Warning: Fertilization is on when CROP is off.
Warning: Soy nitrogen fixation is on when CROP is off.
Warning: DEF_Aerosol_Readin is not needed for DEF_USE_SNICAR off.
Warning: Latitude mismatch:   44.593299865722656  in data file and  44.593299999999999  in namelist.
Warning: Longitude mismatch:  123.50920104980469  in data file and  123.50920000000001  in namelist.
Warning : restart data scale_baseflow in <path>/CN-Cng_baseflow.nc not found, default value is used.
```

三件要紧的事：

1. **前缀不统一**：`Note:`、`Warning:`，以及最后一条的 `Warning :`（冒号前有空格）。
   按前缀匹配必须容忍这个空格。
2. **CoLM 自己的拼写错误**（`automaticlly`）说明**不能按消息文本匹配** ——
   上游改个错字就会让匹配失效。按前缀抽，把整行原样留给上层。
3. 经纬度那两条是真信息：`case.nml` 写的是 5 位小数（`44.59330`），站点文件是
   完整 float 精度（`44.593299865722656`）。CoLM 报了不匹配并**用数据文件的值**。
   这正是 §6.4 说的「你要求了 X，模型实际用了 Y」。

**这 9 条与 `outcome.rs` 的 7 个失败标记零碰撞**（实测），所以抽取覆盖不会与
判定成败互相干扰。

### 日志规模

`colm.log` 实测 **39215 行**（10 天窗口、逐小时输出）。覆盖消息集中在前 20 行，
但抽取要扫全文 —— 长跑里 CoLM 可能在中途再打印。逐行扫一遍 39215 行不值得优化。

### 成败判定已经有了，本轮不动它

`outcome.rs` 的 `adjudicate` 实现了三件套里的全部三条（正向标记、产物硬校验、
错误标记扫描），有 11 条测试。**本轮不改它**，只是把调用它的编排搬进同一个 crate。

---

## 文件结构

```
crates/colm-kernel/
   Cargo.toml            新增 serde / serde_json / sha2 / colm-namelist
   src/lib.rs
   src/outcome.rs        成败判定（已有，不动）      + outcome_tests.rs
   src/manifest.rs       清单读取与完整性校验        + manifest_tests.rs
   src/overrides.rs      静默覆盖抽取（纯计算）      + overrides_tests.rs
   src/run.rs            三段编排                    + run_tests.rs
```

`manifest.rs` 与 `overrides.rs` 是纯计算（前者只读文件、算 sha256），
测试用临时目录与固定文本，不需要真内核。`run.rs` 需要真内核，
它的集成测试归 `golden` 作业。

---

## Task 1: 依赖与模块骨架

**Files:**
- Modify: `crates/colm-kernel/Cargo.toml`
- Modify: `crates/colm-kernel/src/lib.rs`
- Create: `crates/colm-kernel/src/{manifest,overrides,run}.rs`（占位）
- Modify: 根 `Cargo.toml`（`[workspace.dependencies]` 加 serde / serde_json）

- [ ] **Step 1: 根 `Cargo.toml` 加两个 workspace 依赖**

在 `[workspace.dependencies]` 里加（保持字母序）：

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: `crates/colm-kernel/Cargo.toml` 的 `[dependencies]`**

```toml
[dependencies]
anyhow.workspace = true
colm-namelist = { path = "../colm-namelist" }
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
```

`sha2` 已在 `[workspace.dependencies]` 里（`oracle` 在用）。执行时确认一下。

- [ ] **Step 3: `crates/colm-kernel/src/lib.rs`**

在现有的 `pub mod outcome;` 旁加三行（rustfmt 会排序）：

```rust
pub mod manifest;
pub mod overrides;
pub mod run;
```

**保留 lib.rs 现有的模块文档不动。**

- [ ] **Step 4: 建三个占位模块**

`src/{manifest,overrides,run}.rs` 各一行：

```rust
//! 占位，后续 Task 实现。
```

- [ ] **Step 5: 三道门禁**

Run: `cargo build`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Run: `cargo fmt --all --check`
Run: `cargo test --workspace 2>&1 | grep 'test result'`
Expected: 里程碑 0–4 的 124 个测试仍全绿。

- [ ] **Step 6: 提交**

```bash
git add Cargo.toml Cargo.lock crates/colm-kernel
git commit -m "Add the module skeleton for kernel orchestration"
```

---

## Task 2: 清单 —— 先写失败的测试

**Files:**
- Create: `crates/colm-kernel/src/manifest_tests.rs`
- Modify: `crates/colm-kernel/src/manifest.rs`

- [ ] **Step 1: 写测试**

```rust
use std::path::PathBuf;

use super::*;

/// 本机实测的清单，一字不改地当作固件。
const SAMPLE: &str = r#"{
  "schema": 1,
  "preset": "waterheat",
  "platform": "Darwin-arm64",
  "colm_git_sha": "72dd76b9",
  "generator_args": "SinglePoint LULC_IGBP URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF",
  "macros": ["CoLMDEBUG","LULC_IGBP","RangeCheck","SinglePoint","extend_interception","vanGenuchten_Mualem_SOIL_MODEL"],
  "built_with": "GNU Fortran (Homebrew GCC 16.1.0) 16.1.0",
  "netcdf_c": "netCDF 4.9.3",
  "netcdf_fortran": "4.6.3",
  "hdf5": "1.14.6",
  "sha256": {
    "mksrfdata": "053ba92bfbe62d2c74a2d866afe458eeb878b4d557bbe01aecd7e6a9b6e0c0bb",
    "mkinidata": "a707e8c030b650d242ddaa09e4aed8c1e14938afe494bc5b47817b817279fff2",
    "colm":      "8dc6a40aabc704da4a49941779cfa3369e3d5d5125a76d684ad45b8a282140e4"
  }
}"#;

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("colm-kernel-manifest-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create workdir");
    d
}

/// 建一个假内核目录：清单 + 三个内容已知的「二进制」。
fn fake_kernel(name: &str, bodies: &[(&str, &str)]) -> PathBuf {
    let d = workdir(name);
    let mut m = SAMPLE.to_string();
    for (prog, body) in bodies {
        std::fs::write(d.join(format!("{prog}.x")), body).expect("write");
        // 把清单里的占位 sha256 换成这个内容的真实值
        let want = sha256_hex(body.as_bytes());
        let old = match *prog {
            "mksrfdata" => "053ba92bfbe62d2c74a2d866afe458eeb878b4d557bbe01aecd7e6a9b6e0c0bb",
            "mkinidata" => "a707e8c030b650d242ddaa09e4aed8c1e14938afe494bc5b47817b817279fff2",
            _ => "8dc6a40aabc704da4a49941779cfa3369e3d5d5125a76d684ad45b8a282140e4",
        };
        m = m.replace(old, &want);
    }
    std::fs::write(d.join("manifest.json"), m).expect("write manifest");
    d
}

const ALL: &[(&str, &str)] = &[
    ("mksrfdata", "fake mksrfdata"),
    ("mkinidata", "fake mkinidata"),
    ("colm", "fake colm"),
];

#[test]
fn the_sample_manifest_parses_into_its_fields() {
    let m: Manifest = serde_json::from_str(SAMPLE).expect("parses");
    assert_eq!(m.schema, 1);
    assert_eq!(m.preset, "waterheat");
    assert_eq!(m.colm_git_sha, "72dd76b9");
    assert_eq!(m.netcdf_fortran, "4.6.3");
    assert_eq!(m.macros.len(), 6);
    assert!(m.macros.iter().any(|x| x == "SinglePoint"));
    assert_eq!(m.sha256.len(), 3);
}

#[test]
fn the_nested_sha256_object_is_read_as_values_not_keys() {
    // 先前的手写提取对 "sha256" 会返回 "mksrfdata" —— 键名而不是值。
    // 这条测试是那个 bug 的墓碑。
    let m: Manifest = serde_json::from_str(SAMPLE).expect("parses");
    assert_eq!(
        m.sha256.get("colm").map(String::as_str),
        Some("8dc6a40aabc704da4a49941779cfa3369e3d5d5125a76d684ad45b8a282140e4")
    );
}

#[test]
fn a_matching_kernel_opens() {
    let d = fake_kernel("ok", ALL);
    let k = Kernel::open(&d).expect("opens");
    assert_eq!(k.manifest.preset, "waterheat");
    assert_eq!(k.dir, d);
}

#[test]
fn a_missing_binary_and_a_wrong_one_are_different_errors() {
    // design.md §6.1：「不存在」和「存在但版本不对」是两种不同情况，
    // 不能混成一句「内核不可用」。用户对这两种的处置完全不同。
    let d = fake_kernel("missing", &ALL[..2]); // 少 colm.x
    let e = Kernel::open(&d).unwrap_err();
    let s = format!("{e:#}");
    assert!(s.contains("colm.x"), "{s}");
    assert!(s.contains("missing"), "{s}");

    let d = fake_kernel("tampered", ALL);
    std::fs::write(d.join("colm.x"), "tampered").expect("write");
    let e = Kernel::open(&d).unwrap_err();
    let s = format!("{e:#}");
    assert!(s.contains("colm.x"), "{s}");
    assert!(s.contains("sha256"), "{s}");
    assert!(
        !s.contains("missing"),
        "a tampered binary is not a missing one: {s}"
    );
}

#[test]
fn an_unreadable_manifest_says_so_rather_than_blaming_a_binary() {
    let d = workdir("nomanifest");
    for (p, b) in ALL {
        std::fs::write(d.join(format!("{p}.x")), b).expect("write");
    }
    let e = Kernel::open(&d).unwrap_err();
    assert!(format!("{e:#}").contains("manifest"), "{e:#}");
}

#[test]
fn a_manifest_from_a_different_schema_is_refused() {
    // 清单格式将来会变。读到不认识的 schema 就停下，好过按旧字段去解释新文件。
    let d = fake_kernel("schema", ALL);
    let m = std::fs::read_to_string(d.join("manifest.json"))
        .unwrap()
        .replace("\"schema\": 1", "\"schema\": 99");
    std::fs::write(d.join("manifest.json"), m).unwrap();
    let e = Kernel::open(&d).unwrap_err();
    assert!(format!("{e:#}").contains("schema"), "{e:#}");
}

#[test]
fn the_three_programs_are_the_ones_colm_ships() {
    assert_eq!(PROGRAMS, ["mksrfdata", "mkinidata", "colm"]);
}
```

- [ ] **Step 2: 建空壳**

`crates/colm-kernel/src/manifest.rs`：

```rust
//! 构建期清单：认定配置身份与二进制完整性。

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod manifest_tests;
```

- [ ] **Step 3: 确认失败**

Run: `cargo test -p colm-kernel 2>&1 | tail -20`
Expected: 编译失败，找不到 `Manifest` / `Kernel` / `PROGRAMS` / `sha256_hex`。

- [ ] **Step 4: 提交**

```bash
git add crates/colm-kernel/src/manifest.rs crates/colm-kernel/src/manifest_tests.rs
git commit -m "Add failing tests for the kernel manifest"
```

---

## Task 3: 清单 —— 实现

**Files:**
- Modify: `crates/colm-kernel/src/manifest.rs`
- Modify: `crates/colm-kernel/src/lib.rs`

- [ ] **Step 1: 写实现**

```rust
//! 构建期清单：认定配置身份与二进制完整性。
//!
//! `colm.x` / `mkinidata.x` / `mksrfdata.x` 都以 `getarg(1)` 取 namelist 路径，
//! **不接受其他参数，没有 `--version`**（`main/CoLM.F90:185` 等）。所以版本握手
//! 靠构建期写出的 `manifest.json`，而不是问二进制。
//!
//! 清单里两组字段职责不同。`macros` / `colm_git_sha` / `generator_args` 可复现，
//! 认定**配置身份** —— 单点模式最容易搞错的正是编译期宏集合。`sha256` 每次构建
//! 都变（实测：同一路径连跑两次，三个二进制的 sha256 全不同），只认定**完整性**：
//! 二进制自其清单写出以来未被替换。后者要求清单与二进制同生同存，不能分开分发，
//! 也不能拿一份入库的清单去校验重新构建的二进制。
//!
//! 「不存在」与「存在但对不上」是两种不同的失败，分开报 —— 用户对这两种的处置
//! 完全不同：前者是没构建，后者是构建过但被换了。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// CoLM 单点的三个可执行文件，按运行顺序。
pub const PROGRAMS: [&str; 3] = ["mksrfdata", "mkinidata", "colm"];

/// 本 crate 认识的清单版本。
pub const SCHEMA: u32 = 1;

/// `kernels/<preset>/manifest.json` 的内容。
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub preset: String,
    pub platform: String,
    pub colm_git_sha: String,
    pub generator_args: String,
    pub macros: Vec<String>,
    pub built_with: String,
    pub netcdf_c: String,
    pub netcdf_fortran: String,
    pub hdf5: String,
    /// 程序名 -> sha256。用 `BTreeMap` 让报错里的顺序稳定。
    pub sha256: BTreeMap<String, String>,
}

/// 一个已校验的内核目录。
#[derive(Debug, Clone)]
pub struct Kernel {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Kernel {
    /// 读清单并校验三个二进制。任一不符即失败。
    pub fn open(dir: &Path) -> Result<Kernel> {
        let mpath = dir.join("manifest.json");
        let text = std::fs::read_to_string(&mpath)
            .with_context(|| format!("cannot read the kernel manifest at {}", mpath.display()))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse the kernel manifest at {}", mpath.display()))?;

        if manifest.schema != SCHEMA {
            bail!(
                "the manifest at {} has schema {}, and this build understands {SCHEMA}; \
                 refusing to read it with the wrong field meanings",
                mpath.display(),
                manifest.schema
            );
        }

        for prog in PROGRAMS {
            let exe = dir.join(format!("{prog}.x"));
            if !exe.exists() {
                bail!(
                    "{prog}.x is missing from the kernel at {}; the preset has not been built",
                    dir.display()
                );
            }
            let want = manifest.sha256.get(prog).with_context(|| {
                format!(
                    "the manifest at {} records no sha256 for {prog}",
                    mpath.display()
                )
            })?;
            let got = sha256_file(&exe)?;
            if &got != want {
                bail!(
                    "{prog}.x does not match its manifest sha256\n  expected {want}\n  actual   {got}\n\
                     the binary has been replaced since the manifest was written; rebuild the preset"
                );
            }
        }

        Ok(Kernel {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    /// 某一段的可执行文件路径。
    pub fn program(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.x"))
    }
}

/// 一段字节的 sha256，小写十六进制。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn sha256_file(p: &Path) -> Result<String> {
    let bytes =
        std::fs::read(p).with_context(|| format!("cannot read {} to hash it", p.display()))?;
    Ok(sha256_hex(&bytes))
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod manifest_tests;
```

- [ ] **Step 2: 给 `lib.rs` 加重导出**

```rust
pub use manifest::{sha256_hex, Kernel, Manifest, PROGRAMS};
```

- [ ] **Step 3: 测试通过**

Run: `cargo test -p colm-kernel`
Expected: `test result: ok. 18 passed; 0 failed`（11 outcome + 7 manifest）

- [ ] **Step 4: 格式与 lint，然后提交**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/colm-kernel/src
git commit -m "Read the kernel manifest with a parser instead of by hand"
```

---

## Task 4: 静默覆盖 —— 测试与实现

**Files:**
- Create: `crates/colm-kernel/src/overrides_tests.rs`
- Modify: `crates/colm-kernel/src/overrides.rs`
- Modify: `crates/colm-kernel/src/lib.rs`

- [ ] **Step 1: 写测试**

```rust
use super::*;

/// 一次真实 CN-Cng 运行里出现过的全部 9 种消息，一字不改。
const REAL: &str = r#"
 Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.
 Note: Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme.
 Warning: Nitrification-Denitrification is on when BGC is off.
 Warning: Fertilization is on when CROP is off.
 Warning: Soy nitrogen fixation is on when CROP is off.
 Warning: DEF_Aerosol_Readin is not needed for DEF_USE_SNICAR off.
 Warning: Latitude mismatch:    44.593299865722656       in data file and    44.593299999999999      in namelist.
 Warning: Longitude mismatch:    123.50920104980469       in data file and    123.50920000000001      in namelist.
 Warning : restart data scale_baseflow in /w/CN-Cng_baseflow.nc not found, default value is used.
 CoLM Execution Completed.
"#;

#[test]
fn all_nine_real_messages_are_found() {
    let v = extract(REAL);
    assert_eq!(v.len(), 9, "{v:#?}");
    assert_eq!(v.iter().filter(|o| o.kind == Kind::Note).count(), 2);
    assert_eq!(v.iter().filter(|o| o.kind == Kind::Warning).count(), 7);
}

#[test]
fn a_space_before_the_colon_still_counts() {
    // 实测最后一条是 `Warning :`，不是 `Warning:`。按前缀匹配必须容忍这个空格，
    // 否则会漏掉一整类消息而毫无迹象。
    let v = extract(" Warning : restart data scale_baseflow not found, default value is used.\n");
    assert_eq!(v.len(), 1, "{v:#?}");
    assert_eq!(v[0].kind, Kind::Warning);
}

#[test]
fn the_whole_line_is_kept_not_a_parsed_summary() {
    // CoLM 自己把 automatically 拼成了 automaticlly。按消息文本匹配的代码会在
    // 上游改错字的那天静默失效；按前缀抽、把整行原样交给上层就不会。
    let v = extract(" Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.\n");
    assert_eq!(v.len(), 1);
    assert!(v[0].text.contains("automaticlly"), "{:?}", v[0].text);
    assert!(v[0].text.starts_with("Note:"), "{:?}", v[0].text);
}

#[test]
fn ordinary_lines_are_not_mistaken_for_overrides() {
    let v = extract(
        " CoLM Execution Completed.\n Successful in surface data making.\n\
         note that this is not a Note: line because it does not start with one\n",
    );
    assert!(v.is_empty(), "{v:#?}");
}

#[test]
fn the_same_message_twice_is_reported_once() {
    // 长跑里 CoLM 可能在中途重复打印。用户要看的是「有哪些覆盖」，
    // 不是「打印了多少次」。
    let v = extract(" Note: a thing happened\n Note: a thing happened\n");
    assert_eq!(v.len(), 1, "{v:#?}");
}

#[test]
fn a_long_log_does_not_change_the_answer() {
    // 实测 colm.log 有 39215 行，覆盖消息集中在前 20 行 —— 但抽取必须扫全文，
    // 因为长跑里 CoLM 会在中途再打印。这条钉住「扫全文」这件事。
    let mut s = String::from(" Note: at the top\n");
    for i in 0..40_000 {
        s.push_str(&format!(" step {i}\n"));
    }
    s.push_str(" Warning: at the very bottom\n");
    let v = extract(&s);
    assert_eq!(v.len(), 2, "{v:#?}");
    assert!(v[1].text.contains("very bottom"));
}

#[test]
fn none_of_the_real_messages_trips_a_failure_marker() {
    // 抽覆盖与判成败必须互不干扰。实测这 9 条与 outcome.rs 的 7 个失败标记
    // 零碰撞 —— 这条测试守住它，因为两边都会各自增长。
    use crate::outcome::{adjudicate, Outcome, Stage};
    let stdout = format!("{REAL}\n CoLM Execution Completed.\n");
    assert_eq!(
        adjudicate(Stage::Colm, Some(0), &stdout, &[]),
        Outcome::Succeeded
    );
}
```

- [ ] **Step 2: 写实现**

```rust
//! 从内核日志里抽出 CoLM 的静默覆盖。
//!
//! CoLM 会在不声不响地改掉你的配置之后打印一行 `Note:` 或 `Warning:`，
//! 然后继续跑。实测一次 CN-Cng 运行有 9 种这样的消息，其中两条是真正的覆盖
//! （变饱和流被自动打开、VG + IGBP 下土壤阻抗被自动关掉），两条是站点坐标
//! 与 namelist 不一致而**以数据文件为准**。
//!
//! 抽取只认前缀，不认消息文本。CoLM 把 automatically 拼成了 automaticlly，
//! 而上游哪天改回来，按文本匹配的代码就会静默失效。整行原样交给上层，
//! 由上层决定怎么呈现 —— design.md §6.4 要的是「你要求了 X，模型实际用了 Y」。

use std::collections::BTreeSet;

/// 覆盖消息的类别。只按前缀分，不解释内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Note,
    Warning,
}

/// 一条覆盖消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub kind: Kind,
    /// 整行原文，已去掉两端空白。
    pub text: String,
}

/// 扫全文，按出现顺序返回去重后的覆盖消息。
pub fn extract(stdout: &str) -> Vec<Override> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        // 冒号前可能有空格：实测有 `Warning :` 这种写法。
        let Some(kind) = prefix_kind(t) else { continue };
        if seen.insert(t.to_string()) {
            out.push(Override {
                kind,
                text: t.to_string(),
            });
        }
    }
    out
}

fn prefix_kind(line: &str) -> Option<Kind> {
    for (word, kind) in [("Note", Kind::Note), ("Warning", Kind::Warning)] {
        if let Some(rest) = line.strip_prefix(word) {
            if rest.trim_start().starts_with(':') {
                return Some(kind);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod overrides_tests;
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

```rust
pub use overrides::{extract as extract_overrides, Kind, Override};
```

- [ ] **Step 4: 测试通过**

Run: `cargo test -p colm-kernel`
Expected: `test result: ok. 25 passed; 0 failed`（11 outcome + 7 manifest + 7 overrides）

- [ ] **Step 5: 格式与 lint，然后提交**

```bash
git add crates/colm-kernel/src
git commit -m "Surface the configuration changes CoLM makes behind your back"
```

---

## Task 5: 三段编排

**Files:**
- Modify: `crates/colm-kernel/src/run.rs`
- Modify: `crates/colm-kernel/src/lib.rs`

本模块要跑真二进制，所以它的测试在 Task 6 通过黄金回归间接完成 —— 这里不写
单元测试，而是把行为完全交给已有的 `adjudicate`（11 条测试）与 `extract`
（7 条测试）。**本模块自己只负责「跑」与「收」。**

- [ ] **Step 1: 写实现**

```rust
//! 三段编排：mksrfdata → mkinidata → colm。
//!
//! 每一段都是「跑 → 收日志 → 判成败 → 抽覆盖」。判成败在 `outcome`，
//! 抽覆盖在 `overrides`，本模块只负责把它们串起来并落一份日志。
//!
//! stdout 与 stderr 都要收。gfortran 运行时的错误只走 stderr，所以
//! `FAILURE_MARKERS` 里的 `Fortran runtime error` 与 `Error termination`
//! 在只读 stdout 时**永远不可能命中**；实测 namelist 文件缺失时 stdout 是
//! 0 字节而 stderr 有 302 字节，日志会空得看不出原因。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::manifest::Kernel;
use crate::outcome::{adjudicate, Outcome, Stage};
use crate::overrides::{extract, Override};

/// 一段跑完之后知道的一切。
#[derive(Debug, Clone)]
pub struct StageReport {
    pub stage: Stage,
    pub outcome: Outcome,
    /// 日志落盘的位置。失败时报给用户看的就是它。
    pub log: PathBuf,
    /// CoLM 在这一段里声明的静默覆盖。
    pub overrides: Vec<Override>,
}

impl StageReport {
    pub fn succeeded(&self) -> bool {
        matches!(self.outcome, Outcome::Succeeded)
    }
}

/// 跑一段。
///
/// `artifacts` 是这一段必须产出的文件，交给 `adjudicate` 做硬校验 ——
/// 必须列到**文件**，不能只列目录：目录在程序写任何东西之前就已存在，
/// 于是「跑完了但什么都没写」恰好抓不到。
pub fn run_stage(
    kernel: &Kernel,
    stage: Stage,
    namelist: &Path,
    work: &Path,
    artifacts: &[PathBuf],
) -> Result<StageReport> {
    let exe = kernel.program(stage.program());
    let output = Command::new(&exe)
        .arg(namelist)
        .current_dir(work)
        .output()
        .with_context(|| format!("failed to spawn {}", exe.display()))?;

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&stderr);
    }

    let log = work.join(format!("{}.log", stage.program()));
    std::fs::write(&log, text.as_bytes())
        .with_context(|| format!("cannot write {}", log.display()))?;

    Ok(StageReport {
        stage,
        outcome: adjudicate(stage, output.status.code(), &text, artifacts),
        log,
        overrides: extract(&text),
    })
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
```

- [ ] **Step 2: 写 `crates/colm-kernel/src/run_tests.rs`**

```rust
use super::*;

#[test]
fn a_report_knows_whether_it_succeeded() {
    // run_stage 本身要跑真二进制，由黄金回归验；这里只钉住这个小判据，
    // 免得它将来被改成「只要没崩就算成功」。
    let r = StageReport {
        stage: Stage::Colm,
        outcome: Outcome::Succeeded,
        log: PathBuf::from("/tmp/colm.log"),
        overrides: Vec::new(),
    };
    assert!(r.succeeded());
}
```

- [ ] **Step 3: 给 `lib.rs` 加重导出**

```rust
pub use run::{run_stage, StageReport};
```

- [ ] **Step 4: 门禁**

Run: `cargo test -p colm-kernel`
Expected: `test result: ok. 26 passed; 0 failed`

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`

- [ ] **Step 5: 提交**

```bash
git add crates/colm-kernel/src
git commit -m "Put the three-stage orchestration where other callers can reach it"
```

---

## Task 6: `golden_run.rs` 退化成薄壳

**这一步会改动黄金回归的执行路径。两个窗口的 history 必须逐位不变。**

**Files:**
- Modify: `oracle/src/bin/golden_run.rs`
- Modify: `oracle/Cargo.toml`（若不再需要 `sha2`）

- [ ] **Step 1: 删掉搬走的部分**

从 `golden_run.rs` 删除：`verify_kernel`、`check_kernel_provenance`、
`sha256_file`、`extract_json_string`、`extract_json_array`，以及三段循环里
自己拼命令与收日志的那一段。

`read_case_name` 改用 `colm-namelist`：

```rust
fn read_case_name(nml: &Path) -> Result<String> {
    let text = std::fs::read_to_string(nml)
        .with_context(|| format!("cannot read {}", nml.display()))?;
    let doc = colm_namelist::parse(&text)
        .with_context(|| format!("cannot parse {}", nml.display()))?;
    match doc.get("DEF_CASE_NAME") {
        Some(colm_namelist::Value::Str(s)) => Ok(s.clone()),
        Some(other) => bail!("DEF_CASE_NAME is {other:?}, not a string"),
        None => bail!("no DEF_CASE_NAME in {}", nml.display()),
    }
}
```

- [ ] **Step 2: 用新接口重写三段循环**

```rust
    let kernel = colm_kernel::Kernel::open(&repo.join(&kernel))?;
    println!("  kernel: {} ({})", kernel.manifest.preset, kernel.manifest.colm_git_sha);

    for (stage, artifacts) in &stages {
        let r = colm_kernel::run_stage(&kernel, *stage, &work.join("case.nml"), &work, artifacts)?;
        for o in &r.overrides {
            println!("  {:<10} {:?}: {}", "", o.kind, o.text);
        }
        if r.succeeded() {
            println!("  {:<10} ok", stage.program());
        } else {
            eprintln!("  {:<10} FAILED: {:?}", stage.program(), r.outcome);
            eprintln!("  log: {}", r.log.display());
            bail!("stage {} failed", stage.program());
        }
    }
```

**注意**：现有的 `check_kernel_provenance` 除了 sha256 还比对了 `colm_git_sha`
与入库的 `oracle/golden/kernel-manifest.json`。执行时先读那个文件，确认这层
比对是否还需要 —— 若需要，它属于 `oracle`（黄金回归特有的「内核没换过」检查），
不属于 `colm-kernel`（通用的「二进制没被替换」检查）。**两者不是一回事，
不要合并。**

- [ ] **Step 3: 重跑两个窗口**

```bash
./oracle/scripts/build_kernel.sh waterheat
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-run -- CN-Cng-wet
cargo run -p oracle --bin golden-compare -- oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
cargo run -p oracle --bin golden-compare -- oracle/golden/CN-Cng-wet_hist_2008-07.nc \
  oracle/work/CN-Cng-wet/out/CN-Cng-wet/history/CN-Cng-wet_hist_2008-07.nc
```

Expected: 两条都 `identical: 129 variables`。**任何差异都必须解释清楚再继续** ——
本 Task 只搬代码，不改语义。

- [ ] **Step 4: 覆盖消息应当出现在输出里**

`golden-run` 现在会打出那 9 条。确认它们出现了，且没有把 `ok` 那行挤掉。

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "Let golden-run call the orchestration instead of owning it"
```

---

## Task 7: CI 与文档收尾

**Files:**
- Modify: `README.md`
- Modify: `docs/design.md`（§6.1 / §6.4 标注已实现）

- [ ] **Step 1: README 补一节**

讲 `colm-kernel` 现在管三件事：判成败（退出码不可信，见 §2.4）、认内核
（清单代替 `--version`）、报覆盖（CoLM 会改你的配置然后继续跑）。

- [ ] **Step 2: 全量验证**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -q -p oracle --bin tier-check -- oracle/golden/*.nc
git diff --check
```

- [ ] **Step 3: 提交**

---

## 完成判据

- [ ] `cargo test --workspace` 通过；`colm-kernel` 的 26 个单元测试全部执行
- [ ] 清单的嵌套 `sha256` 对象**读出的是值不是键名**（手写提取的那个 bug 有墓碑测试）
- [ ] 「二进制不存在」与「二进制被替换」是两条不同的报错，各自说得出是哪个文件
- [ ] 未知 `schema` 被拒绝，而不是按旧字段解释
- [ ] 一次真实运行的 **9 条覆盖消息全部被抽出**，含 `Warning :`（冒号前有空格）那条
- [ ] 覆盖消息与失败标记**零碰撞**（有测试守住）
- [ ] `golden_run.rs` 不再含手写 JSON 提取与手写 namelist 解析
- [ ] 两个窗口的 history **逐位不变**
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all --check` 无输出

---

## 留给后续里程碑的

- **进程生命周期**（design.md §6.6：run lease、按 PID 取消、逐行日志抽取、
  二进制暂存到临时副本）本轮不做。黄金回归是一次性同步调用，用不上它们；
  它们属于 GUI 里程碑。
- **覆盖消息的语义化**：本轮只抽出整行。把「你要求了 X，模型实际用了 Y」
  解析成结构化的一对，需要知道每条消息对应哪个 `DEF_` 字段 —— 那要一张
  手写的对照表，且上游改一个字就会失效。等 GUI 真要渲染它时再做，
  届时至少知道要渲染成什么样。
- **`oracle/golden/kernel-manifest.json` 的角色**：它记的是「黄金文件是用哪个
  内核跑出来的」，与 `colm-kernel` 的完整性校验不是一回事。Task 6 会确认它
  是否仍需保留在 `oracle` 侧。
