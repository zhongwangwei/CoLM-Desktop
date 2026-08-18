# 里程碑 8 实施计划：GUI —— 三栏工作台，单站点闭环

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `design.md` §10 里程碑 8 的验收 —— **macOS 与 Linux 上双击可跑并出图**。

**Architecture:** Tauri v2 + 无 npm 的静态前端 + vendored uPlot。后端**完全不链接 netcdf/hdf5**，凡要读 NetCDF 的一律走 `colm-cli` sidecar。

**Tech Stack:** tauri 2.11.3、tauri-plugin-dialog 2.7.1、wry 0.55.1、uPlot 1.6.31（MIT）。不引入 Node。

---

## 已实测的事实基础

### 分层已经天然支持「GUI 不碰 HDF5」

实测各 crate 的 netcdf/hdf5 依赖节点数：

| crate | 节点数 |
|---|---|
| `colm-namelist` / `colm-schema` / `colm-case` / `colm-kernel` / `colm-hist`（默认） | **0** |
| `colm-forcing` / `colm-srfdata` | 7 |
| `colm-cli` | 9 |

所以 Tauri 后端链接前五个仍然是 HDF5-free 的，而这不需要任何重构 ——
EarthMesh 那套「GUI 进程绝不链接 netcdf」的架构在我们这边是从已有分层里
**自然掉出来的**。

后端能在进程内做的：读写 namelist、按 schema 描述字段、判内核清单。
必须走 sidecar 的：补站点文件、读强迫场元数据、跑模型、算指标、**取绘图数据**。

最后一项是本轮要给 `colm-cli` 补的缺口（Task 1）。

### Tauri 骨架与流式输出已预跑

最小骨架在本机编过（Xcode CLT + `cargo tauri` 2.11.4 已装），
子进程流式输出那段也编过、零警告。依赖规模：

| | crate 数 |
|---|---|
| 引擎 workspace | **70** |
| Tauri 最小骨架 | **429** |

这就是 `design.md` §4.1「两个 workspace 刻意分离」的数字依据：不分离的话
`cargo test --workspace` 每次要面对 429 个 crate 而不是 70 个。
debug 二进制 33 MB，增量重编 1.27 秒。

**一个坑**：即便 `bundle.active: false`，`tauri::generate_context!` 仍然要求
`icons/icon.png` 存在，否则编译期报 `failed to open icon`。

### uPlot 三条断言已核实

`uPlot.iife.min.js` **50,312 字节**、MIT（带 LICENSE 文件）、**零依赖**、
Canvas 2D（`getContext('2d')` 一处，`webgl` 与 `wasm` 各 0 次）。
IIFE 形式与 `script-src 'self'` 兼容，不需要模块加载器。

数据规模远在它的能力之内：一个站点两年 35088 个点，而 uPlot 的实测能力是
166,650 点交互 25 ms。

### 配置页的形状

20 个真实主算例一共只用到 **118 个不同字段**：

| | 字段数 |
|---|---|
| 全部 20 个文件都设 | **23** |
| ≥16 个文件设 | 35 |
| 只出现在 1 个文件 | 34 |

所以配置页是「核心约 35 + 长尾约 34 + 中间 49」，不是把 202 个顶层字段摊开。

而**新建向导只需要问 3 件事**：选哪个站、算例叫什么、（可选）窗口收窄到哪。
其余全部推导得出 —— `colm-cli new` 已经这么做了：经纬度与地类读自站点文件、
时间步长读自强迫场、窗口默认取强迫场的完整覆盖。

### 日志必须在过 IPC 之前降速

实测 CN-Cng 冬季窗口（528 个模型步）：

| | |
|---|---|
| `colm.log` | 39215 行 / 3.3 MB |
| 其中 `Check vector data`（RangeCheck） | **33357 行，占 85%** |
| 真正有内容的 | 2685 行 |
| 进度行 `TIMESTEP = N \| DATE = N` | 每模型步一行 |

外推到完整两年（35088 步）：**约 260 万行 / 220 MB**。
按每步 9 毫秒算，进度行约 **110 行/秒**。

所以后端逐行读、但**不逐行发**：RangeCheck 行直接丢弃（它只在越界时才有信息，
而越界会被 `colm-kernel` 的失败标记抓住），进度行解析成百分比后节流到
约 10 次/秒，其余行进环形缓冲区，前端要看时再取。

### 覆盖消息与缺失变量要连起来说

CoLM 会不声不响地改配置然后继续跑，一次运行有 9 种这样的消息。
其中第一条 `DEF_USE_VariablySaturatedFlow is automaticlly set to .true.`
同时解释了输出变量里为什么有 `qlayer` 没有 `qcharge` ——
`colm-hist` 的闸门表把这个条件原样记着。

**GUI 该把这两头连成一句话**，而不是一条淹没在日志里的 `Note:`
加一个莫名其妙空着的变量。这是本项目「报告实际产出了什么，
而不是你要求了什么」在界面上的落点。

---

## 文件结构

```
gui/                          【新】独立 workspace，不进引擎 workspace
├── dist/
│   ├── index.html            静态前端，无 npm、无打包器
│   └── vendor/uplot/         uPlot.iife.min.js + uPlot.min.css + LICENSE
└── src-tauri/
    ├── Cargo.toml            空的 [workspace]，把 429 个 crate 挡在外面
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── icons/icon.png        即便 bundle.active=false 也必需
    └── src/
        ├── main.rs           6 行，调 lib::run()
        ├── lib.rs            模块枢纽 + generate_handler!
        ├── sidecar.rs        起 colm-cli、逐行读、节流后发事件
        ├── config.rs         无状态配置往返（colm-namelist + colm-schema）
        └── project.rs        算例目录的发现与元数据

crates/colm-cli/src/
└── main.rs                   新增 series 子命令
```

---

## Task 1: `colm-cli series` —— 把绘图数据交出来

**Files:**
- Modify: `crates/colm-cli/src/main.rs`

GUI 要画曲线就要 history 里的数值，而那是 NetCDF。让后端去读会把整个 HDF5
拖进 GUI 进程，所以由 sidecar 导出：

```
colm-cli series <case-dir> --vars f_rnet,f_fsena [--out series.json]
```

输出 JSON：`{"time": [...], "vars": {"f_rnet": [...], ...}}`。
时间轴用**相对窗口起点的秒**，与 `colm-hist::time::model_seconds` 同一约定。

只导出 `(time, patch)` 形状的变量 —— 实测 119 个 `f_*` 里 108 个是这个形状，
剩下 11 个是剖面，需要另一种画法，本轮不做。

体量：一个站点两年 35088 点，5 条序列的 JSON 约 3.5 MB。够小，直接过 IPC。

- [ ] Step 1: 加子命令与 JSON 序列化（`serde_json` 已在 workspace deps 里）
- [ ] Step 2: 对黄金算例导出 `f_rnet`，断言 264 个点、与 `judge` 读到的值一致
- [ ] Step 3: 提交

---

## Task 2: Tauri 骨架

**Files:** `gui/` 整个目录

- [ ] **Step 1: 骨架**

照 EarthMesh 实测到的形态：`src-tauri/Cargo.toml` 里一个**空的 `[workspace]`**，
`withGlobalTauri: true`，`main.rs` 只调 `lib::run()`，
`crate-type = ["staticlib", "cdylib", "rlib"]`。

**自定义 `#[tauri::command]` 不需要在 capabilities 里声明权限**，只有插件命令需要。

- [ ] **Step 2: 确认它编得过并能起窗口**

Run: `cd gui/src-tauri && cargo build`
Expected: 编过。记得先放 `icons/icon.png`，否则 `generate_context!` 编译期就炸。

- [ ] **Step 3: 提交**

---

## Task 3: sidecar 桥

**Files:** `gui/src-tauri/src/sidecar.rs`

下面这段已在本机编过、零警告：

```rust
use std::io::{BufRead, BufReader};

use tauri::Emitter;

/// 起一个子进程，逐行把它的 stdout 发给前端。
///
/// 这是整个 GUI 赖以成立的机制：重活走 sidecar，进程本身不链接 netcdf。
/// 逐行抽取必须在**独立线程**里做 —— 管道满了之后不读就会死锁。
#[tauri::command]
async fn spawn_and_stream(
    app: tauri::AppHandle,
    program: String,
    args: Vec<String>,
) -> Result<i32, String> {
    let child = std::process::Command::new(&program)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {program}: {e}"))?;
    let mut child = child;
    let out = child.stdout.take().ok_or("no stdout")?;
    let h = app.clone();
    let t = std::thread::spawn(move || {
        let mut n = 0usize;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            n += 1;
            let _ = h.emit("proc://line", line);
        }
        n
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    let lines = t.join().map_err(|_| "reader thread panicked")?;
    let _ = app.emit("proc://done", lines);
    Ok(status.code().unwrap_or(-1))
}
```

**本轮要加的是节流**，上面那段还没有：

- `Check vector data` 行直接丢弃（85% 的量，且越界会被失败标记抓住）
- 进度行 `TIMESTEP = N | DATE = N` 解析成百分比，**节流到约 10 次/秒**
- 其余行进环形缓冲区（上限几千行），前端按需取

不加节流的话，完整两年运行会往 webview 发 260 万个事件。

`colm-cli` 的发现顺序照 EarthMesh 的 `resolve_mkgrd`：
`$COLM_CLI` → 应用自身目录（含打包进去的 sidecar）→ 仓库 `target/{release,debug}`
→ `PATH`。

---

## Task 4: 配置层命令（进程内）

**Files:** `gui/src-tauri/src/config.rs`

照抄 EarthMesh 的**无状态配置往返**：每个修改命令接收**整份配置文本**加一个
改动字段，返回重新校验后的规范化文本。Rust 拥有 schema，前端从不自行构造配置。

命令：

| 命令 | 作用 |
|---|---|
| `describe_fields` | 返回 `colm-schema` 的字段元数据，含 `group` 与「谁都设不了」的标记 |
| `read_case` | 读一份 case.nml，返回其字段与值 |
| `set_field` | 整份文本 + 一个字段 → 校验后的整份文本 |
| `unknown_fields` | 文本里 schema 不认识的字段 —— 上游删掉的旧字段在这里现形 |

`unknown_fields` 不是装饰：上游**自己发布的**单点示例
`run/examples/SiteSYSUAtmos_IGBP_VG.nml` 就设了 `USE_SITE_topostd` 与
`USE_SITE_BVIC` 两个已从 `MOD_Namelist.F90` 删除的字段，CoLM 读到会
`Cannot match namelist object name` 然后中止。界面该在开跑前就说
「这里有 2 个字段 CoLM 已经不认了」，而不是让用户对着那句报错发呆。

**格式保留**：`colm-namelist` 的往返保证未改动的行逐字节不变 ——
用户算例文件里的注释是他们自己的笔记。

---

## Task 5: 三栏工作台与新建向导

**Files:** `gui/dist/index.html`

- 左：算例库（扫描一个目录下的算例）
- 中：配置分页（站点/时间/物理/输出变量）+ 日志
- 右：结果（曲线）

**新建向导只问 3 件事**（选站、命名、可选窗口），其余推导 ——
后端直接调 `colm-cli new`。

输出变量页渲染的是 `colm_hist::writable(manifest.macros)` 的结果，
**不是 482 个开关**。勾了但产不出来的要说清为什么（闸门表里记着条件原文）。

---

## Task 6: 出图

**Files:** `gui/dist/vendor/uplot/`、`gui/dist/index.html`

vendored uPlot（50 KB + 2 KB CSS + LICENSE，随包分发，`script-src 'self'` 禁止 CDN）。
数据来自 Task 1 的 `colm-cli series`。

先只做 `(time, patch)` 形状的 108 个变量，按单位分组叠加
（实测 30 个 W/m2、19 个 mm/s、12 个无量纲、11 个 mm）。

---

## Task 7: 打包与文档

`bundle.externalBin` 打包 `colm-cli`，暂存脚本用 **xtask 写，不引入 Node**。
sidecar 运行前先拷成带 PID 与源哈希后缀的临时副本（EarthMesh 实测：
源码树里的静态 netcdf 二进制直接跑会被 SIGKILL）。

---

## 完成判据

- [ ] macOS 与 Linux 上双击启动，选一个站点、跑完、看到曲线
- [ ] GUI 进程**不链接 netcdf/hdf5**（`cargo tree` 验证）
- [ ] 完整两年运行不会把 260 万行日志逐行发给 webview
- [ ] 输出变量页渲染的是这个内核能产出的那些，不是 482 个开关
- [ ] 打开一份含已删除字段的 namelist，界面在开跑前点名它们
- [ ] 未改动的配置行在保存后逐字节不变
- [ ] `cargo test --workspace` 与 GUI 的 `cargo build` 各自通过；两个 workspace 不互相拖累

---

## 明确不做

- **Windows** —— `design.md` §10 归里程碑 9，它要单独打通 MSYS2 内核 + MSVC GUI。
- **批量与敏感性** —— 里程碑 11。
- **剖面变量的画法** —— 119 个里 11 个是 `(time, patch, soil)` 之类，需要热图或剖面图，等有人要了再做。
- **多预设** —— 里程碑 10。本轮只有 `waterheat`。
