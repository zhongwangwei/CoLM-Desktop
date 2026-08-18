# GUI 重构实施计划（里程碑 13）

> **给执行者：** 用 `superpowers:subagent-driven-development`（推荐）或
> `superpowers:executing-plans` 按任务逐条实施。步骤用 `- [ ]` 复选框标记。

**目标：** 把现在这个 353 行、一屏三栏的单站点工作台，做成能扫描、分类、
批量运行、自动比对并出评估图的完整桌面界面。

**架构：** 后端仍是 Tauri 命令 + `colm-cli` sidecar 两层分工不变 ——
窗口进程不链接 netcdf，凡要读 NetCDF 的一律走 sidecar（`design.md` §4.2）。
本次改动主要在前端与几个新命令；**已有的 13 个命令一个都不删**，只增补。

**技术栈：** Tauri v2 2.11.x、无 npm 的静态前端、内置 uPlot 1.6.31。
**不引入任何前端框架、构建工具或 npm 依赖** —— 这条是既有约束，见 §4.2 与
`xtask check-gui`（前端是纯静态 JS，没有类型检查器，接口靠它静态守住）。

---

## 0. 先读这一节：现状与已量到的数

写这份计划之前量过的数，**执行时直接用，不要重新猜**：

| 事实 | 数 | 来源 |
|---|---|---|
| schema 字段总数 | 737 | `colm_schema::all().len()` |
| ├ `nl_colm` | 214 | 生成表的 `group` 字段 |
| ├ `nl_colm_forcing` | 35 | 同上 |
| ├ `nl_colm_history` | **482** | 同上，全部是 `DEF_hist_vars%*` |
| └ 无 group（派生值，设了没用） | 6 | 同上 |
| 顶层字段（名字不含 `%`） | 202 | |
| 现有 Tauri 命令 | 13 | `xtask check-gui` |
| 前端 | 353 行单文件 | `gui/dist/index.html` |
| PLUMBER2 站点 | 90 | `PLUMBER2s/Sitedata/` |
| Urban-PLUMBER 站点 | 21 | `Urban-PLUMBER/Sitedata/` |
| 物理预设 | 3 | `kernels/{waterheat,bgc,urban}` |
| 一次真实运行的 stdout | 34180 行 | AU-Preston，528 步 |
| └ 其中进日志窗的 | 2850 行 | 丢弃 RangeCheck 28152 + 空行 2650 + 进度 528 |

顶层 202 个字段的前缀分布（**这是分类的依据，不要另起一套**）：

| 前缀 | 数 |
|---|---|
| `DEF_` 其他 | 119 |
| `DEF_USE_*` | 40 |
| `USE_SITE_*` | 17 |
| `DEF_HIST*` / `DEF_hist*` | 11 |
| `DEF_dir*` | 8 |
| `SITE_*` | 4 |
| 无前缀 | 3 |

已有的可复用件：

- `colm_schema::{all, find, Field, FieldKind, Default}` —— 类型、默认值、
  所属组、行尾注释、`arity`。
- `colm_namelist::{parse, Doc, Value}` —— **往返保留原文**：改一个字段，
  其余行逐字节不变。用户算例里的注释是他们自己的笔记。
- `colm_case::{fields, minimal::required, render}` —— 造算例，且**只写偏离
  默认值的字段**。
- `colm_hist::metric::Metrics` —— `n / rmse / mae / bias / r2 / kge /
  obs_mean / obs_sd / beta / beta_warning`。
- `colm_hist::pair::pair()` —— 模型与观测按时间轴配对。
- `colm-cli` 五个子命令：`new / run / metrics / series / all`。
  `run` 有 `--stream 1`，`new` 城市站点自动识别且要 `--rawdata`/`--runtime`。

---

## 1. 八个必须先定的设计决策

这些不是可以边写边定的细节。**先按这里定的做**，要改先说。

### 1.1 「482 个输出变量」不能和其他字段同等对待

`nl_colm_history` 一组就占 737 个字段里的 482 个，全部是
`DEF_hist_vars%<变量名>` 形式的开关。把它们铺进一个表格，界面就废了。

**定：输出变量单独一个页面**，不进「参数分类」那套。它是一个带搜索框的
开关列表，按 `colm-hist` 的闸门表分组（那张表已经知道每个变量在什么条件下
才会被写出来），并标出「当前配置下这个变量根本不会被写」。

**不要**给 482 个开关各做一个输入框。它们全是 logical。

### 1.2 普通模式 / 专家模式的分界是「算例实际设了什么」

不是按字段重要性主观挑。**定：**

- **普通模式**：只显示这份 `.nml` **实际设了的字段**（一个城市算例是 24 个，
  一个水热算例是 21 个），加上一组固定的「常改项」白名单（时间窗口、
  输出频率、路径四项）。
- **专家模式**：显示全部 202 个顶层字段，按 §0 的前缀分类，未设的显示
  schema 默认值并标灰。

理由：`minimal::required` 已经把「偏离默认值的」与「等于默认值的」分开了，
这个分界是现成的、可解释的，且和 `.nml` 文件里看到的一致。

### 1.3 批量运行必须改事件模型

现在的 `run://progress` / `run://lines` / `run://done` 是**全局**事件，
一次只能跑一个算例。批量跑 90 个站点时，前端分不清哪条消息属于哪个站。

**定：三个事件的 payload 都加 `case: String` 字段**（算例目录，唯一标识）。
前端按它分发。**不新增事件名** —— 现有三个 `listen` 是
`xtask check-gui` 静态守着的接口，加字段不破坏它，改名会。

**并发上限定为 2。** 理由：一次运行 528 步就打 34180 行，三段串行约 40 秒；
90 站顺序跑约 1 小时。并发能缩短，但每个子进程都要读同一份 rawdata 并写各自
的输出，磁盘是瓶颈而不是 CPU。**2 是起点，不是结论** —— 实现时把它做成常量
并在注释里写明它没被测过，别写成「经过调优」。

### 1.4 观测文件靠命名约定找，不靠用户指

`colm-cli` 的 `LAYOUTS` 已经知道两套约定：

| 数据集 | 站点 | 强迫场 | 观测 |
|---|---|---|---|
| PLUMBER2 | `<X>_site.nc` | `<X>_Met.nc` | `<X>_Flux.nc` |
| Urban-PLUMBER | `<X>_site_v1.nc` | `<X>_metforcing_v1.nc` | `<X>_clean_observations_v1.nc` |

**定：扫描站点目录时同时探测观测文件是否存在**，在站点列表里显示「有观测 /
无观测」。有观测的站点，跑完自动出指标；没有的，评估相关的按钮直接置灰并
说明原因（不是静默不动）。

**不要**做「让用户为每个站点手工指定观测文件」的界面。要留一个整体的
「观测目录」输入框作为兜底，用于命名不合约定的数据。

### 1.5 参数校验在后端，不在前端

`config::set_field` 已经按 schema 的 `FieldKind` 做了类型校验，
并返回具体的错误文本（「`X` 是 logical；`"abc"` 既不是 `.true.` 也不是
`.false.`」）。

**定：前端不自己判断合法性**，一律调后端、把返回的错误原样显示在字段旁边。
前端只做一件后端做不到的事：**在输入过程中**（`input` 事件）做一次同样规则的
预判并高亮，但**保存与否仍由后端说了算**。前端那份预判如果和后端不一致，
以后端为准 —— 所以它只能高亮，不能阻止提交。

### 1.5b 窗口进程**可以**读 NetCDF —— 但要独立成一层

`design.md` §4.2 原先的规矩是「窗口进程不链接 netcdf/hdf5，一律 shell out
给 sidecar」。**这条已被放宽**：窗口进程可以直接读，代价先量清楚了。

| | 值 |
|---|---|
| 现在的 GUI release 二进制 | 9.85 MB |
| 加了 netcdf 依赖但没人调用 | 9.90 MB（**+50 KB**） |
| 真读一次 NetCDF 之后 | **13.94 MB（+4.1 MB）** |
| GUI workspace 首次构建静态 netcdf/HDF5 | 约 60 秒（之后走缓存） |

中间那一行是个陷阱：只加依赖不调用，链接器会把整个静态库丢掉，量出来
「几乎不要钱」。**必须带着真调用量**，否则得到的是一个假的便宜。

**定：读 NetCDF 的能力放进一个独立模块 `gui/src-tauri/src/nc.rs`**，
遵守三条：

1. **只读，且只读元信息与序列。** 不在窗口进程里做配对、算指标、写文件 ——
   那些仍走 sidecar。这一层的存在理由是「点一下就要看到的东西」
   （变量列表、时间范围、一条曲线），不是把 CLI 搬进来。
2. **所有读走一个专用工作线程，串行排队。** HDF5 默认不是线程安全的，
   而窗口进程里没有第二个 HDF5 使用者可以帮忙暴露竞态 —— 一旦并发读出问题，
   表现会是偶发崩溃而不是报错。串行的代价是可接受的：单点站点文件最大 15 MB。
3. **绝不阻塞 UI 线程。** 命令写成 `async`，读的部分丢给上面那个工作线程。

**保留 sidecar 那条路，不要删。** 两条路并存的分工：窗口进程读「小而立刻要看
的」，sidecar 跑「大而可以等的」（`metrics`、`series` 的全量导出、批量）。
`nc.rs` 是可摘除的 —— 删掉它 GUI 仍然能用，只是每次看变量列表要多一次进程启动。

### 1.6 三个可执行文件在界面上必须分开，而且能分别跳过

一次运行是三个程序串行，**职责与依赖各不相同**：

| 阶段 | 产物 | 取决于 | 换了时间窗口还要重跑吗 |
|---|---|---|---|
| `mksrfdata.x` | `landdata/srfdata.nc` | 站点文件、rawdata、地表分类与土壤方案 | **不用** |
| `mkinidata.x` | `restart/const/*.nc`、`restart/<起始日>/*.nc` | srfdata + 起始日期 | 要 |
| `colm.x` | `history/*.nc` | 前两者 + 全部物理与输出配置 | 要 |

**现在界面上看不出这三者。** 更实际的问题是：`run://progress` 只解析
`TIMESTEP =`，而**只有 `colm.x` 打这一行** —— 前两段跑的时候进度条完全不动。
城市算例里 `mksrfdata` 恰恰是慢的那个（它要读全球栅格）。

**定：**

1. 进度区改成三段式，每段自己的状态（待运行 / 运行中 / 成功 / 失败），
   `colm.x` 那一段内部再显示步进度。`run://progress` 的 payload 加 `stage`
   字段（与 §1.3 的 `case` 一起加，一次改完）。
2. **允许跳过已完成的阶段。** `run_stage` 的 `artifacts` 参数已经精确列出了
   每一段必须产出的文件，「产物齐全 → 可跳过」是现成可算的。
3. **但不能只看文件在不在。** 改了 `SITE_fsitedata`、`DEF_dir_rawdata` 或
   土壤/地表方案，srfdata 就失效了，而文件还在那儿。**定：在算例目录里写一份
   `stages.json`，记下每段完成时其输入的指纹**（相关 namelist 字段的值 +
   站点文件的 sha256）。指纹不一致就必须重跑，并在界面上说明是哪一项变了。
   界面默认「自动」，另给「强制全部重跑」。

**不要**把跳过做成一个用户随手勾的复选框而不校验指纹 —— 那等于把
「结果是用旧地表数据算的」这种错误交给用户自己记住。

### 1.7 两个都叫 spin-up 的东西，必须在界面上分开

这是本项目最容易混的一对概念，**它们不是一回事**：

| | 模型 spin-up | 评估丢弃 |
|---|---|---|
| 在哪 | `DEF_simulation_time%spinup_*`（CoLM 自己） | `colm-cli metrics --spinup N` |
| 干什么 | 起始日之前的那段反复跑 N 遍，让土壤温湿等状态趋于平衡 | 算指标时丢掉**前 N 条输出记录** |
| 单位 | 循环次数 | 记录条数 |
| 影响输出吗 | **不影响** —— 见下 | 只影响指标，不改文件 |

CoLM 的实现（本次读源码确认）：`ptstamp` 由 `spinup_year/month/day/sec`
拼出，`is_spinup = (起始时刻 < ptstamp)`，循环 `spinup_repeat` 次
（`n_spinupcycle = max(spinup_repeat, 1)`，所以 0 与 1 都是一遍）。
关键一条：**`MOD_Hist.F90:235` 在 `itstamp <= ptstamp` 时直接 `RETURN`** ——
spin-up 期间只累积、不写 history。所以模型 spin-up 不会污染输出，
两个概念的作用域确实是分开的。

**现状：两个都没进 GUI。**

- 模型 spin-up：`colm-case` 显式把它关掉（写 `spinup_repeat = 0`），
  界面上没有任何入口。
- 评估丢弃：`colm-cli metrics --spinup N` 有，但 **GUI 里根本没有 metrics
  这个命令**（`grep -c metrics gui/src-tauri/src/*.rs` 全是 0）——
  整条评估链目前只有命令行能用。

**还有一个已确认的缺陷。** spin-up 期间 CoLM 打的是另一种格式
（`CoLM.F90:747`）：

```
TIMESTEP = 1 | DATE = 2008-01-01-00000 Spinup (cycle 1 of 3)
```

而 `parse_progress` 用 `strip_prefix("DATE =")` 之后整段尾巴都留在 `date` 里
（本次实测：`date` 变成 `"2008-01-01-00000 Spinup (cycle 1 of 3)"`）。
不崩，但界面分不出正在 spin-up，且进度条的步数会跨循环单调增长而看不出重来过。

**定：**`parse_progress` 认这两种格式，`Progress` 加
`spinup: Option<(u32, u32)>`（第几轮 / 共几轮）。界面在 spin-up 期间显示
「预热 2/3 轮」而不是把它混进正常进度。

---

## 2. 文件结构

前端从一个 353 行的文件拆成多个模块。**用原生 ES module**
（`<script type="module">`），不引入打包器。Tauri 的 CSP 里
`script-src 'self'` 允许同源模块。

```
gui/dist/
├── index.html          仅骨架 + <script type="module" src="app/main.js">
├── app/
│   ├── main.js         启动、路由、全局 state
│   ├── ipc.js          invoke/listen 的唯一入口（便于 check-gui 扫描）
│   ├── sites.js        站点扫描、站点列表、批量队列 UI
│   ├── params.js       参数分类表格、普通/专家模式、校验高亮
│   ├── histvars.js     482 个输出变量的搜索 + 开关列表
│   ├── runner.js       运行控制、进度、日志窗
│   ├── results.js      指标表 + uPlot 图（含观测对比）
│   ├── presets.js      参数预设保存/加载
│   └── ui.js           小组件：表格、开关、搜索框、Toast
└── app/style.css       从 index.html 的 <style> 抽出
```

**`ipc.js` 是硬要求。** `xtask check-gui` 靠正则扫 `invoke('name'` 与
`listen('name'` 来核对前后端接口。全部调用集中到一处，扫描才可靠 ——
现在它已经因为 rustfmt 把 `emit("run://done")` 拆行而漏过一次。
**任务 1 必须同步更新 `xtask/src/gui.rs` 让它扫 `app/` 下的所有 `.js`。**

后端新增（`gui/src-tauri/src/`）：

```
sites.rs      扫描站点目录、探测强迫场与观测、批量队列状态
presets.rs    参数预设的读写（JSON，存在算例目录之外的用户配置目录）
```

---

## 3. 任务

### 任务 1：`check-gui` 学会扫多文件（**已完成**）

`xtask/src/gui.rs` 现在递归扫 `gui/dist/` 下的 `.html` 与 `.js`（排除
`vendor/`），后端扫 `gui/src-tauri/src/` 下所有 `.rs`（不再写死三个文件名 ——
加一个 `sites.rs` 就会漏掉里面注册的命令，而检查照样是绿的）。
文件按路径排序后拼接，免得报错顺序随文件系统变。

四条测试守着：扫到 `app/*.js`、不扫 `vendor/`、新后端模块自动被收进来、
拼接顺序稳定。真跑仍是 `13 commands registered, 12 called`。

**这一步必须在拆前端之前做完** —— 反过来的话，拆完那一刻检查就会把
「已被调用」的命令报成「没人调用」，而一条假警报比没有警报更糟。

---

### 任务 1b：前端拆成模块

**文件：**
- 新建：`gui/dist/app/{main,ipc,ui}.js`、`gui/dist/app/style.css`
- 修改：`gui/dist/index.html`
- 修改：`xtask/src/gui.rs`
- 测试：`xtask/src/gui.rs` 内的单元测试

- [x] **步骤 1-4 已完成**（见任务 1）。以下保留原文以备查：

- [x] **步骤 1：先让 `check-gui` 能扫多文件，并写一个会失败的测试**

在 `xtask/src/gui.rs` 里加：

```rust
#[test]
fn it_scans_every_js_module_not_just_index_html() {
    // 前端拆模块之后，invoke 调用不再全在 index.html 里。
    // 只扫那一个文件的话，check-gui 会把「已被调用」的命令报成「没人调用」，
    // 而那正是它唯一的用处。
    let dir = std::env::temp_dir().join("check-gui-multifile");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join("index.html"), "<script type=module src=app/main.js></script>").unwrap();
    std::fs::write(dir.join("app/main.js"), "invoke('list_cases')").unwrap();
    let found = super::called_commands(&dir).unwrap();
    assert!(found.contains("list_cases"), "没扫到 app/main.js 里的调用");
}
```

- [x] **步骤 2：跑它，确认失败**

`cargo test -p xtask it_scans_every_js_module -- --nocapture`
预期：`no function or associated item named 'called_commands'`

- [x] **步骤 3：把扫描改成遍历目录**

`called_commands(dir)` 遍历 `dir` 下所有 `.html` 与 `.js`（递归），
对每个文件套用现有的正则。**保留现有的「跨行容忍」处理**（正则不要用 `^` 锚定，
现在的实现因为 rustfmt 拆行漏过一次）。

- [x] **步骤 4：跑通**，`cargo test -p xtask`

- [ ] **步骤 5：拆前端**

`index.html` 只留 DOM 骨架与 `<link rel=stylesheet href="app/style.css">`、
`<script type="module" src="app/main.js"></script>`。
把现有 353 行里的 `<style>` 移进 `style.css`，脚本按 §2 拆开。
**这一步不改任何行为** —— 拆完之后 Chromium 里走一遍，页面与拆之前一致。

- [ ] **步骤 6：`ipc.js` 收口**

```js
// 全部 IPC 只从这里出去。check-gui 靠扫这些字面量核对前后端接口，
// 散落各处就扫不全 —— 而扫不全的表现是「命令明明有人调用却报成没人调用」，
// 一条假警报比没有警报更糟。
const T = window.__TAURI__;
export const invoke = T?.core?.invoke;
export const listen = T?.event?.listen;
export const hasBackend = !!invoke;
```

- [ ] **步骤 7：验收 + 提交**

```bash
cargo run -p xtask -- check-gui      # 命令数与拆分前一致
cargo test -p xtask
git add gui/dist xtask && git commit
```

---

### 任务 2：站点扫描与批量队列（后端）

**文件：**
- 新建：`gui/src-tauri/src/sites.rs`
- 修改：`gui/src-tauri/src/lib.rs`（注册命令）

- [ ] **步骤 1：写失败的测试**

新建 `gui/src-tauri/src/sites_tests.rs`：

```rust
use super::*;

/// 造一个假的 PLUMBER2 目录树：Sitedata / Forcing / Observation 三个同级目录。
fn fake_tree(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("colm-sites-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["Sitedata", "Forcing", "Observation"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    for (sub, f) in files {
        std::fs::write(d.join(sub).join(f), b"x").unwrap();
    }
    d
}

#[test]
fn a_site_reports_whether_its_forcing_and_observation_are_there() {
    // 「有观测」决定跑完能不能自动评估。列表里就得说清楚，
    // 而不是等用户点了「评估」才报错。
    let d = fake_tree("plumber2", &[
        ("Sitedata", "AT-Neu_2002-2012_FLUXNET2015_site.nc"),
        ("Forcing",  "AT-Neu_2002-2012_FLUXNET2015_Met.nc"),
        // 故意不放 Observation
    ]);
    let s = scan_sites(d.join("Sitedata").to_string_lossy().into_owned()).unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].name, "AT-Neu");
    assert!(s[0].has_forcing);
    assert!(!s[0].has_observation);
}

#[test]
fn it_recognises_the_urban_naming_convention_too() {
    // Urban-PLUMBER 三个后缀全不一样。两套约定都认，否则城市站点
    // 会被报成「没有强迫场」而其实就在旁边。
    let d = fake_tree("urban", &[
        ("Sitedata",    "AU-Preston_site_v1.nc"),
        ("Forcing",     "AU-Preston_metforcing_v1.nc"),
        ("Observation", "AU-Preston_clean_observations_v1.nc"),
    ]);
    let s = scan_sites(d.join("Sitedata").to_string_lossy().into_owned()).unwrap();
    assert_eq!(s[0].name, "AU-Preston");
    assert!(s[0].has_forcing && s[0].has_observation);
    assert!(s[0].urban, "没有 IGBP_classification 的形状就是城市站点");
}
```

- [ ] **步骤 2：跑它，确认失败**（`scan_sites` 不存在）

- [ ] **步骤 3：实现**

```rust
//! 站点目录扫描。
//!
//! 「有没有强迫场 / 有没有观测」必须在列表里就说清楚 —— 让用户点了运行
//! 才发现缺文件，是把一次可以立刻回答的检查推迟到最贵的时刻。

use serde::Serialize;

/// 两套数据集的命名约定。与 `colm-cli` 的 `LAYOUTS` 是同一张表，
/// **改一处必须改另一处**（两个 crate 不互相依赖，这是刻意的分层代价）。
const LAYOUTS: [(&str, &str, &str); 2] = [
    ("_site.nc", "_Met.nc", "_Flux.nc"),
    ("_site_v1.nc", "_metforcing_v1.nc", "_clean_observations_v1.nc"),
];

#[derive(Serialize)]
pub struct Site {
    /// 站点代号，词干的第一段（`AT-Neu_2002-2012_..._site.nc` -> `AT-Neu`）
    pub name: String,
    pub site_file: String,
    pub has_forcing: bool,
    pub has_observation: bool,
    /// 城市形状：站点文件不带 `IGBP_classification`。城市算例必须给
    /// `--rawdata` / `--runtime`，界面要据此决定问不问。
    pub urban: bool,
}

#[tauri::command]
pub fn scan_sites(dir: String) -> Result<Vec<Site>, String> { /* … */ }
```

判定 `urban` 用 `colm_srfdata::site::location(..).landtype.is_some()`。
**但 `colm-srfdata` 要 netcdf，而窗口进程不链接 netcdf**（§4.2）——
所以这一项**走 sidecar**：给 `colm-cli` 加一个 `scan` 子命令输出 JSON，
`scan_sites` 调它。假文件在测试里读不出经纬度，所以测试里
`urban` 的判据退化为「文件名后缀是 `_site_v1.nc`」，实现时两条都要：
后缀是快路径，sidecar 的读取是准路径。

- [ ] **步骤 4：跑通测试**

- [ ] **步骤 5：提交**

---

### 任务 3：`colm-cli scan` 子命令

**文件：** 修改 `crates/colm-cli/src/main.rs`

- [ ] **步骤 1：测试先行** —— 在 `oracle/tests/` 加一条，用真 PLUMBER2 目录
      （`PLUMBER2_ROOT` 未设时跳过，与既有测试一致）。

- [ ] **步骤 2：实现**

```
colm-cli scan --dir <Sitedata 目录> [--out sites.json]
```

输出 JSON 数组，每项：`{name, site_file, met_file, obs_file, urban,
lon, lat, landtype, start, end, step_seconds}`。
经纬度与时间范围顺手一起读出来 —— 界面要显示，而这是**唯一**一次
打开这些文件的机会，分两次读是浪费。

**90 个站点全读一遍要多久？实现完先量，把数写进 README。** 如果超过 3 秒，
加一句进度输出，别让界面干等。

- [ ] **步骤 3：验收**

```bash
colm-cli scan --dir $PLUMBER2_ROOT/Sitedata | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d), '个站点')"
# 预期：90 个站点
```

- [ ] **步骤 4：提交**

---

### 任务 4：事件加 `case` 字段，支持批量

**文件：** 修改 `gui/src-tauri/src/sidecar.rs`

- [ ] **步骤 1：写失败的测试**

```rust
#[test]
fn every_run_event_carries_which_case_it_came_from() {
    // 批量跑 90 个站点时，三个事件是全局的 —— 前端分不清哪条属于哪个站。
    // 加字段而不是改事件名：现有三个 listen 是 check-gui 守着的接口。
    let p = Progress { case: "/tmp/a".into(), step: 1, date: "2008-01-01-00000".into() };
    let j = serde_json::to_string(&p).unwrap();
    assert!(j.contains("\"case\":\"/tmp/a\""), "{j}");
    let d = Done { case: "/tmp/a".into(), code: 0, total: 10, dropped: 3 };
    assert!(serde_json::to_string(&d).unwrap().contains("\"case\""));
}
```

`run://lines` 现在的 payload 是 `Vec<String>`，加字段就得改成结构体
`{case, lines}`。**这是破坏性改动**，前端要同步改，且注释里写明为什么值得。

- [ ] **步骤 2-3：实现并跑通**

- [ ] **步骤 4：批量队列命令**

```rust
/// 排队跑一批算例。返回后立刻结束，进度靠事件。
///
/// 并发上限 2。**这个数没被测过** —— 每个子进程都读同一份 rawdata、
/// 写各自的输出，瓶颈大概率在磁盘而不是 CPU，但没量过。
/// 调之前先量，别把猜的数写成结论。
const MAX_CONCURRENT: usize = 2;

#[tauri::command]
pub async fn run_batch(app: tauri::AppHandle, cases: Vec<String>, kernel: String)
    -> Result<(), String>
```

每个算例结束时发 `run://done`（带 `case`），前端据此更新那一行的状态。

- [ ] **步骤 5：提交**

---

### 任务 5：参数分类 + 普通/专家模式（前端）

**文件：** 新建 `gui/dist/app/params.js`

- [ ] **步骤 1：分类表**

按 §0 的前缀分布，**九个分类**，顺序固定：

```js
// 分类依据是字段名前缀，不是主观的「重要性」—— 前缀是 CoLM 自己的命名，
// 会随上游一起演进；主观分类会在下一次上游加字段时立刻过时。
export const GROUPS = [
  { id: 'site',     label: '站点',     match: n => n.startsWith('SITE_') || n.startsWith('USE_SITE_') },
  { id: 'time',     label: '时间',     match: n => n.startsWith('DEF_simulation_time') },
  { id: 'dirs',     label: '路径',     match: n => n.startsWith('DEF_dir') || n === 'DEF_forcing_namelist' },
  { id: 'physics',  label: '物理开关', match: n => n.startsWith('DEF_USE_') },
  { id: 'soil',     label: '土壤',     match: n => /SOIL|Soil|soil/.test(n) },
  { id: 'urban',    label: '城市',     match: n => n.includes('URBAN') || n.includes('Urban') },
  { id: 'forcing',  label: '强迫场',   match: (n, f) => f.group === 'nl_colm_forcing' },
  { id: 'output',   label: '输出',     match: n => /^DEF_(HIST|hist|WRST)/.test(n) },
  { id: 'other',    label: '其他',     match: () => true },   // 兜底，必须最后
];
```

**每个字段只进第一个匹配上的分类**（顺序即优先级）。
`other` 是兜底：新字段进来不会消失，而是显眼地堆在「其他」里 ——
那正是提醒有人该给它归类的信号。

- [ ] **步骤 2：写一个会失败的检查**

在 `xtask` 里加一条静态检查（**不是前端测试** —— 前端没有测试框架，
这是既定约束）：

```rust
#[test]
fn every_schema_field_lands_in_exactly_one_group() {
    // 分类表在 JS 里，字段表在 Rust 里。没有这条，上游加一个 DEF_ 之后
    // 界面上会静默地少一个字段，而两边的测试都还是绿的。
    let js = std::fs::read_to_string(root().join("gui/dist/app/params.js")).unwrap();
    for g in ["site", "time", "dirs", "physics", "soil", "urban", "forcing", "output", "other"] {
        assert!(js.contains(&format!("id: '{g}'")), "分类 {g} 不见了");
    }
    // 兜底必须在最后一条，否则前面的分类会被它吃掉
    let last = js.rfind("id: '").unwrap();
    assert!(js[last..].starts_with("id: 'other'"), "other 必须是最后一个分类");
}
```

- [ ] **步骤 3：普通/专家模式**

普通模式的字段集 = 这份 `.nml` 里实际出现的 + 固定白名单：

```js
// 「常改项」白名单。判据是：一个人跑一个新站点时几乎一定要看的东西。
// 保持短 —— 长白名单等于没有普通模式。
const ALWAYS_SHOWN = [
  'DEF_simulation_time%start_year', 'DEF_simulation_time%end_year',
  'DEF_HIST_FREQ', 'DEF_dir_output',
];
```

专家模式显示全部 202 个顶层字段；未设的显示 schema 默认值、标灰，
并在编辑时才真正写进文件。

- [ ] **步骤 4：校验高亮**

`input` 时按 `FieldKind` 预判并高亮，`change` 时调 `set_field`，
失败则把**后端返回的原文**显示在字段下方（见 §1.5）。

- [ ] **步骤 5：验收 + 提交**

---

### 任务 6：482 个输出变量的独立页面

**文件：** 新建 `gui/dist/app/histvars.js`

- [ ] **步骤 1：搜索 + 开关列表**，虚拟滚动或分页（482 行一次性渲染在
      低端机上会卡，实现时量一下 —— 如果 482 个 `<tr>` 的首次渲染低于
      100 ms，就不要引入虚拟滚动这个复杂度）。

- [ ] **步骤 2：接上闸门表**

`colm-hist` 的闸门表知道每个变量在什么条件下才会被写出来
（456 个写点，三个预设上零漏报）。**把「当前配置下这个变量不会被写」
标出来** —— 用户勾了却没有输出，是这个界面最该防的事。

需要一个新命令：

```rust
/// 在当前配置下，哪些 history 变量实际会被写出来。
#[tauri::command]
pub fn writable_vars(text: String) -> Result<Vec<String>, String>
```

- [ ] **步骤 3：验收 + 提交**

---

### 任务 7：结果页 —— 指标表 + 观测对比图

**文件：** 新建 `gui/dist/app/results.js`

- [ ] **步骤 1：指标表**

`colm-cli metrics` 已经能出 `Metrics`，但**GUI 里还没有对应的命令**
（实测 `grep -c metrics gui/src-tauri/src/*.rs` 全为 0），所以这一步要先加
一个走 sidecar 的 `metrics` 命令，参数含 `--spinup N`（§1.7 的「评估丢弃」，
不是模型 spin-up）。指标字段：
`n / rmse / mae / bias / r2 / kge / obs_mean / obs_sd / beta / beta_warning`。
表格直接呈现，**`beta_warning` 非空时必须显示** —— 它标记 β 项不可信的两种
情形，藏起来等于给一个假指标。

- [ ] **步骤 2：模型 vs 观测的双线图**

uPlot 两条 series。**时间轴必须 `tzDate: uPlot.tzDate(d, 'Etc/UTC')`** ——
PLUMBER2 是地方时、模型按地方时推进，时间戳是「把地方时当 UTC」算出来的；
按浏览器本地时区格式化会把整条曲线平移一个时区（实测浏览器在 `Asia/Shanghai`
时首点显示 8:30 而不是 0:30）。

- [ ] **步骤 3：散点图 + 1:1 线**，用于一眼看偏差结构。

- [ ] **步骤 4：批量汇总**

多站点跑完之后，一张按站点排的指标表，可按任一列排序。

- [ ] **步骤 5：验收 + 提交**

---

### 任务 8：参数预设保存/加载

**文件：** 新建 `gui/src-tauri/src/presets.rs`

- [ ] **步骤 1：存在哪里**

**不存在算例目录里** —— 预设的用处正是跨算例复用。存到
`app.path().app_config_dir()` 下的 `presets/<名字>.json`。

- [ ] **步骤 2：存什么**

存 `(路径, 值)` 列表，**不存整份 `.nml`**。理由：套用预设时要能与算例已有的
设置合并，而不是整个覆盖掉站点身份与路径。预设里**禁止**包含
`SITE_*`、`DEF_dir*`、`DEF_forcing_namelist` —— 那些是算例身份，不是参数。
实现时显式过滤并在 UI 上说明。

- [ ] **步骤 3：测试、实现、提交**

---

### 任务 9：三阶段进度与阶段跳过

**文件：** 修改 `gui/src-tauri/src/sidecar.rs`、`crates/colm-cli/src/main.rs`、
新建 `gui/dist/app/runner.js`

见 §1.6。三步：

- [ ] **步骤 1：`Progress` 加 `stage` 字段，三段都发事件**

现在只有 `colm.x` 打 `TIMESTEP =`，所以前两段跑的时候进度条是死的。
`colm-cli run` 在每段开始/结束时**自己打一行**标记（例如
`=== stage mksrfdata begin ===`），sidecar 认它并发 `run://progress`。
**标记由我们自己打，不要去认 CoLM 的输出措辞** —— CoLM 把 automatically
拼成 automaticlly 这件事已经教过一次，上游随时会改措辞。

- [ ] **步骤 2：`stages.json` 指纹**

```rust
/// 一段完成时，它的输入是什么样子。
///
/// **只看产物在不在是不够的**：改了 `SITE_fsitedata` 或 `DEF_dir_rawdata`，
/// srfdata 就失效了，而文件还好好躺在那儿。跳过它等于拿旧地表数据算新算例，
/// 而且没有任何迹象。
#[derive(Serialize, Deserialize)]
pub struct StageFingerprint {
    pub stage: String,
    /// 这一段依赖的 namelist 字段及其值
    pub inputs: BTreeMap<String, String>,
    /// 站点文件的 sha256
    pub site_sha256: String,
}
```

各段依赖的字段列表写成常量并注明依据；不确定的**宁可多列** ——
多列一项只是多重跑一次，少列一项是静默算错。

- [ ] **步骤 3：界面三段式 + 「强制全部重跑」**

---

### 任务 10：两个 spin-up 都进界面

见 §1.7。**先在 UI 文案上把它们分开命名**：模型侧叫「预热（spin-up）」，
评估侧叫「丢弃前 N 条记录」。**不要**在两处都写「spinup」。

- [ ] **步骤 1：`parse_progress` 认 spin-up 格式（写失败的测试先）**

```rust
#[test]
fn it_tells_a_spinup_step_from_a_normal_one() {
    // CoLM.F90:747 与 :749 是两种 format。只认后者的话，spin-up 行的整段尾巴
    // 会留在 `date` 里 —— 实测变成 "2008-01-01-00000 Spinup (cycle 1 of 3)"。
    // 不崩，但界面分不出正在预热，进度条还会跨循环单调增长。
    let normal = parse_progress("TIMESTEP = 1 | DATE = 2008-01-01-00000").unwrap();
    assert_eq!(normal.date, "2008-01-01-00000");
    assert_eq!(normal.spinup, None);

    let s = parse_progress("TIMESTEP = 1 | DATE = 2008-01-01-00000 Spinup (cycle 2 of 3)").unwrap();
    assert_eq!(s.date, "2008-01-01-00000", "日期不该混进循环计数");
    assert_eq!(s.spinup, Some((2, 3)));
}
```

- [ ] **步骤 2：实现，跑通**

- [ ] **步骤 3：模型 spin-up 的入口**

时间分类下加一组：轮数（`spinup_repeat`）与预热截止日期
（`spinup_year/month/day/sec`）。**旁边写一句它不写 history**
（`MOD_Hist.F90:235` 在 `itstamp <= ptstamp` 时直接 RETURN），
否则用户会以为预热期的输出被算进指标了。

- [ ] **步骤 4：评估丢弃的入口**

在结果页，紧挨指标表，改了立刻重算（纯计算，不重跑模型）。
默认 0，并说明它的单位是**输出记录条数**，不是天数、不是循环次数。

- [ ] **步骤 5：提交**

---

### 任务 11：界面重做与响应式

**文件：** `gui/dist/index.html`、`gui/dist/app/style.css`

- [ ] **步骤 1：布局**

现在是固定的 `grid-template-columns: 230px 1fr 420px`。改成：

```css
/* 三档。用 CSS Grid 的命名区域，不引入任何布局库。 */
@media (max-width: 900px)  { /* 单栏，页面间用顶部标签切换 */ }
@media (max-width: 1400px) { /* 双栏，结果区折叠成抽屉 */ }
@media (min-width: 1401px) { /* 三栏，即现在的形态 */ }
```

- [ ] **步骤 2：主题**

跟随系统深浅色（现在 `color-scheme: light dark` 已经在了），
**颜色全部用 CSS 变量定义在 `:root`**，深色只覆盖变量。

- [ ] **步骤 3：验收**

在 Chromium 里用 Playwright 走三个断点各截一次，确认没有横向滚动条、
没有元素重叠。**这是本项目验证前端的既定办法**：把 `gui/dist/` 原样复制出去、
只在 uPlot 那行 `<script>` 之前插一段 mock（其余逐字节相同，由脚本断言），
mock 的载荷全部取自真后端导出。

- [ ] **步骤 4：提交**

---

## 4. 全局验收

每个任务结束都要过的：

```bash
cd gui/src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --all --check
cd ../.. && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p xtask -- check-gui
```

全部任务完成之后：

- [ ] 黄金回归两个窗口仍逐字节相同
      （`golden-run` + `golden-compare`，`identical: 129 variables, 10 dimensions`）
- [ ] 三个预设各跑通一次
- [ ] 打包一次 `.app`，在**藏起仓库 `kernels/` 与 `target/*/colm-cli`** 的条件下
      启动，确认它自报用的是包里那份
- [ ] Playwright 走完整流程：扫描 → 选站 → 改参数 → 运行 → 出图 → 出指标

## 5. 明确不做

写下来是为了不被反复提起：

- **不引入前端框架、打包器或任何 npm 依赖。** 既定约束。
- **不做算例的版本管理 / diff。** 那是 git 的事。
- ~~不在窗口进程里读 NetCDF~~ —— **这条已撤销**，见 §1.5b。
  但仍**不在窗口进程里做计算**：配对、指标、批量导出一律走 sidecar。
- **不做远程/集群提交。** 本项目是单机桌面程序。
- **不动 `vendor/CoLM202X`。** 一行都不改，submodule 保持在干净的上游 commit。
