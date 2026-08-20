# 前处理阶段 A2：强迫场子栏（实施计划）

> **给执行者：** 用 `superpowers:subagent-driven-development` 按任务逐条实施。
> 步骤用 `- [ ]` 复选框标记。

**目标：** 把 A1 做出来的转换管道搬到界面上 —— 用户在前处理页选一个
netCDF，看到自动猜出来的槽位映射，改掉猜错的，补上缺的高度，转出一份
下一步能扫到的文件。

**架构：** 后端已经齐了（`colm-forcing::convert` / `units` / `slots`），
A2 加两个 Tauri 命令与一个子栏。唯一的后端改动是**让手填的高度进产物**。

**技术栈：** Rust（`gui/src-tauri`）+ 纯静态 ES module 前端（**无 npm、
无构建工具**）。

**范围：** 只做强迫场子栏。站点属性是阶段 B，表格导入是阶段 C。

---

## 0. 先读这一节

### A1 撞出来的那个 bug 正是这一阶段的需求

`convert()` 曾经不搬 `reference_height_v/t/q`，产物缺了它们，
`met::summarize` 静默回落成 `NaN`，写进 `forcing.nml` 之后 CoLM 的
RangeCheck 直接 `SIGILL` —— 而报出来的是「内核编进了 CoLMDEBUG」。
已修（`191fea7`：槽位没消费的变量全搬）。

**但那只解决了源文件里有的情况。** 实测：

| 数据集 | 有 `reference_height_*` |
|---|---|
| PLUMBER2（90 个站） | **全有** |
| Urban-PLUMBER（21 个站） | **全没有** |

城市站现在不受影响，因为它们用自带的 `.nml`（`HEIGHT = 48.05`）。
**但走转换这条路的用户没有那份 nml。** 所以设计文档 §2.1 那条
「高度：用户数据可能没有，要能手填」不是可选项，是必需品。

### 用户确认映射是必经一步

设计文档 §2.1：

> **用户确认映射是必经一步**，不能全自动。变量名猜错的后果是
> 「跑得完、结果全错」—— 比如把 `Rainf`（降水）猜成别的槽位。
> 界面要把「猜出来的映射」摆出来让人过目。

**不要做一个「一键转换」按钮。** 自动匹配的结果必须先显示、
让人能改，才允许转换。

### 已经核实过的事实（别再花时间查）

`MetSummary`（`crates/colm-forcing/src/check.rs`）已经有：

```rust
pub struct MetSummary {
    pub time_units: String,
    pub start: Stamp,
    pub steps: usize,
    pub step_seconds: f64,
    pub step_uniform: bool,      // 时间轴均匀性，已经在算了
    pub height_v: f64,           // 缺失时是 NaN
    pub height_t: f64,
    pub height_q: f64,
    pub variables: Vec<String>,
    pub time_shown_in: Option<String>,
}
```

`slots::resolve_with(&variables, &overrides)` 返回 `(Resolved, Vec<String>)`，
`Resolved.vname` 是 `[Option<&'static str>; 8]`，`overrides` 是
`&[(usize, String)]`（1-based 槽位号）。

`convert::Plan { slots: Vec<SlotPlan> }`，
`SlotPlan { index, source_name, source_units, also_add }`。

---

## Task 1: 手填的高度要进产物

**Files:**
- Modify: `crates/colm-forcing/src/convert.rs`
- Modify: `crates/colm-forcing/src/convert_tests.rs`

- [ ] **Step 1: 写失败的测试**

```rust
#[test]
fn heights_given_by_hand_land_in_the_product() {
    // Urban-PLUMBER 的 21 个站都没有 reference_height_*，而 CoLM 要它们。
    // 界面上让人填，填了就要写进产物 —— **产物必须自包含**，不能只写
    // 进这一次的 forcing.nml，否则下次拿这份文件重建算例又是 NaN。
    let dir = std::env::temp_dir().join("colm-convert-heights");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let p = dir.join("noheight_Met.nc");
    {
        let mut f = netcdf::create(&p).unwrap();
        f.add_dimension("time", 2).unwrap();
        let mut t = f.add_variable::<f64>("time", &["time"]).unwrap();
        t.put_attribute("units", "seconds since 2008-01-01 00:00:00").unwrap();
        t.put_values(&[0.0, 1800.0], netcdf::Extents::All).unwrap();
        let mut v = f.add_variable::<f64>("Tair", &["time"]).unwrap();
        v.put_attribute("units", "K").unwrap();
        v.put_values(&[273.15, 274.15], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("out_Met.nc");
    let plan = super::Plan {
        slots: vec![super::SlotPlan {
            index: 1,
            source_name: "Tair".into(),
            source_units: "K".into(),
            also_add: Vec::new(),
        }],
        heights: Some(super::Heights { v: 48.05, t: 48.05, q: 48.05 }),
    };
    super::convert(&p, &dst, &plan).expect("convert");

    let f = netcdf::open(&dst).unwrap();
    for (name, want) in [
        ("reference_height_v", 48.05),
        ("reference_height_t", 48.05),
        ("reference_height_q", 48.05),
    ] {
        let got: Vec<f64> = f
            .variable(name)
            .unwrap_or_else(|| panic!("{name} 该被写进产物"))
            .get_values(netcdf::Extents::All)
            .unwrap();
        assert_eq!(got, vec![want]);
    }
}

#[test]
fn heights_already_in_the_source_are_not_overwritten() {
    // **源文件说了的，界面不该覆盖。** PLUMBER2 的 90 个站都带着这三个
    // 标量，转换时原样搬（`191fea7` 那条规则），手填只在源文件没有时用。
    // 反过来会让「量出来的」被「填进去的」悄悄换掉。
    // 这里断言：`heights: None` 时源文件的值原样在产物里。
    // （构造一个带 reference_height_t = 6.0 的源文件，转完还是 6.0。）
}
```

**第二条测试的函数体要自己写完整** —— 上面只给了意图。

- [ ] **Step 2: 跑，确认失败**

```bash
cargo test -p colm-forcing --lib heights 2>&1 | tail -8
```

期望：编译失败 —— `Plan` 没有 `heights` 字段，也没有 `Heights` 类型。

- [ ] **Step 3: 写实现**

```rust
/// 观测高度。源文件没有 `reference_height_*` 时由用户在界面上填。
///
/// **三个分开而不是一个值**：CoLM 的 `DEF_forcing%HEIGHT_V/T/Q` 本来
/// 就是三个，风的观测高度与温湿的常常不同（塔上不同层）。
pub struct Heights {
    pub v: f64,
    pub t: f64,
    pub q: f64,
}
```

`Plan` 加 `pub heights: Option<Heights>`。

`convert` 在末尾（搬完所有变量之后）写：

```rust
    // **手填的高度只在源文件没有时写。** 源文件说了的是量出来的，
    // 界面填的是人估的 —— 让后者覆盖前者是在拿估计换掉测量。
    if let Some(h) = &plan.heights {
        for (name, val) in [
            ("reference_height_v", h.v),
            ("reference_height_t", h.t),
            ("reference_height_q", h.q),
        ] {
            if fout.variable(name).is_some() {
                continue; // 源文件带着，已经搬过去了
            }
            let mut out = fout.add_variable::<f64>(name, &[])?;
            out.put_attribute("units", "m")?;
            out.put_attribute("source", "given by hand in the prep page")?;
            out.put_values(&[val], netcdf::Extents::All)?;
        }
    }
```

**现有构造 `Plan` 的地方都要补 `heights: None`** —— 编译器会点名：
`convert_tests.rs` 若干处、`src/bin/forcing-convert.rs`、
`oracle/tests/forcing_convert.rs`。

- [ ] **Step 4: 跑，确认五条旧的加两条新的都过**

```bash
cargo test -p colm-forcing --lib convert 2>&1 | tail -10
```

- [ ] **Step 5: CLI 也要能填**

`forcing-convert` 加 `--height V,T,Q`（一个参数三个值，逗号分隔 ——
三个独立参数在命令行上太啰嗦）：

```rust
            "--height" => {
                let spec = args.next().context("--height needs V,T,Q")?;
                let n: Vec<f64> = spec
                    .split(',')
                    .map(|x| x.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .with_context(|| format!("--height {spec:?} is not V,T,Q"))?;
                let [v, t, q] = n[..] else {
                    bail!("--height needs exactly three numbers, got {}", n.len());
                };
                heights = Some(Heights { v, t, q });
            }
```

实测：

```bash
U=/Users/zhongwangwei/Desktop/colm-rust/Urban-PLUMBER
./target/debug/forcing-convert "$U/Forcing/FI-Kumpula_metforcing_v1.nc" \
  /tmp/fi-h.nc --slot 4=Rainf:kg/m2/s+Snowf --height 48.05,48.05,48.05
ncdump -h /tmp/fi-h.nc | grep -A3 reference_height_t
```

期望：产物里有三个 `reference_height_*`，值是 48.05。

- [ ] **Step 6: 提交**

```bash
cargo test --workspace 2>&1 | tail -4     # 基线 317 passed
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
git add crates/colm-forcing/src/convert.rs crates/colm-forcing/src/convert_tests.rs \
        crates/colm-forcing/src/bin/forcing-convert.rs oracle/tests/forcing_convert.rs
git commit -m "手填的高度要进产物，但不能覆盖源文件量出来的

Urban-PLUMBER 的 21 个站都没有 reference_height_*，PLUMBER2 的 90 个
全有。走转换这条路的用户没有城市站那份自带 nml，所以界面必须能填。

填了就写进产物而不是只写进这一次的 forcing.nml——产物要自包含，
否则下次拿这份文件重建算例又是 NaN，而 NaN 的下场是 SIGILL。

源文件带着的不覆盖：那是量出来的，界面填的是人估的。

Confidence: high
Scope-risk: narrow
Tested: 两条新单测; FI-Kumpula 实测产物带上 48.05"
```

---

## Task 2: 两个 Tauri 命令

**Files:**
- Create: `gui/src-tauri/src/forcing.rs`
- Modify: `gui/src-tauri/src/lib.rs`（注册命令）

### ⚠️ 原规格错了：GUI 不能直接调 `colm-forcing`

第一版规格假设 `probe_forcing` 直接调 `colm_forcing::summarize`。
**不行。** 实测：

```
gui/src-tauri 依赖的本仓库 crate：
  colm-namelist  colm-schema  colm-case  colm-kernel  colm-hist
```

**没有 `colm-forcing`，而且 `cargo tree -i netcdf` 在 GUI 里查无此包。**
`colm-hist` 在这里是不带 `io` feature 的 —— `oracle/Cargo.toml` 那条
注释说明了为什么：

> 依赖方向是 oracle → colm-hist，反过来不行 —— 闸门表那一半必须无依赖，
> 否则 netcdf 会跟着它一起被拖进 GUI。

加 `colm-forcing` 依赖会把静态 netcdf + HDF5 拖进 GUI，**正是这条注释
刻意避免的事**。

### 正确的路：走 sidecar

`scan_sites` 就是这么做的 —— `crate::sidecar::capture(&args)` 起
`colm-cli` 子进程，解析它的 JSON 输出。`capture` 固定调 `resolve_cli()`
解析出的 `colm-cli`，不能调别的二进制。

而 **`colm-cli` 已经依赖 `colm-forcing`**（`crates/colm-cli/Cargo.toml:15`）。

所以 Task 2 分三层：

| 层 | 做什么 |
|---|---|
| 2a | `colm-forcing` 抽出 `--slot` / `--height` 的解析函数（公开） |
| 2b | `colm-cli` 加 `forcing-probe`（JSON）与 `forcing-convert` 两个子命令 |
| 2c | `gui/src-tauri` 的两个 Tauri 命令走 `sidecar::capture` |

**2a 是为了不抄第二遍。** `crates/colm-forcing/src/bin/forcing-convert.rs`
已经有一份 `--slot N=名字:单位+另一个` 的解析，colm-cli 那边需要同样的
逻辑。抄一遍意味着两处要同步改 —— A1 刚因为「同一段代码抄三遍，错也
有三份」修过 `_FillValue` 那个 bug。

**独立 bin `forcing-convert` 保留**，它对纯命令行用户仍然有用，
且已经有实测覆盖。两边共用 2a 抽出来的解析函数。

- [ ] **Step 1: 命令的形状（已核实，2026-08-20）**

不用再去翻了，形状是这样的：

```rust
// gui/src-tauri/src/forcing.rs
#[derive(serde::Serialize)]      // 返回结构要这个
pub struct Probe { ... }

#[tauri::command]
pub async fn probe_forcing(path: String) -> Result<Probe, String> { ... }
```

`lib.rs` 里两处：`mod forcing;` 与 `use forcing::*;`（照第 12–28 行
现有那批的样子），再把命令名加进 `generate_handler![...]`（第 37 行起）。

**参数名 snake_case，前后端一一对应。** 实测 `scan_sites(dir, quick)`
对应前端 `invoke('scan_sites', { dir, quick: true })` —— 没有
camelCase 转换。

**两个命令都要 `async`。** `scan_sites` 就是 `pub async fn`，理由在
`met.rs` 的模块注释里：

> 时间轴要全读一遍，因为「步长是否均匀」只能这样确认

FI-Kumpula 是 245469 步，同步读会卡住界面。`convert_forcing` 要写
整个文件，更慢。

`xtask check-gui` 检查五件事（`xtask/src/gui.rs`）：前端 `invoke` 的
命令名必须已注册、`listen` 的事件必须有人 `emit`、参数名要对得上、
import 要解析得了、**模块不许成环**。写完必须跑。

- [ ] **Step 2: `probe_forcing`**

```rust
/// 探一份强迫场文件：变量列表、自动猜出来的槽位映射、时间轴、高度。
///
/// **只探不改。** 用户要先看到猜的结果、能改，才允许转换 ——
/// 变量名猜错的后果是「跑得完、结果全错」。
///
/// 走 sidecar 而不是直接调库：GUI 不依赖 `colm-forcing`，
/// 那会把 netcdf 拖进来（见上面那节）。
#[tauri::command]
pub async fn probe_forcing(path: String) -> Result<Probe, String> {
    let json = crate::sidecar::capture(&[
        "forcing-probe".into(),
        path,
        "--json".into(),
        "1".into(),
    ])?;
    serde_json::from_str(&json).map_err(|e| {
        // 说清楚是**解析**失败而不是探测失败 —— 照 `scan_sites` 的措辞，
        // 两者的处置完全不同。
        format!("colm-cli forcing-probe 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
}
```

**`Probe` 与 `SlotGuess` 要 `Deserialize` 而不只是 `Serialize`** ——
它们现在是从 JSON 读进来再转发给前端的。colm-cli 那边输出同样形状的
JSON（字段名一致），两边各写各的结构体：`sites_tests.rs` 那条注释说了
为什么这样反而更安全 ——

> 两个 crate 不互相依赖；`Site` 与 `SiteInfo` 各写各的。哪天那边改了
> 字段名，只有拿真输出跑一遍才发现得了。

所以**必须有一条拿真数据跑的测试**，否则字段脱钩了没人知道。

返回结构（字段名要与前端一致，`check-gui` 会验）：

```rust
pub struct Probe {
    /// 文件里所有变量名 —— 用户改映射时要从这里选。
    pub variables: Vec<String>,
    /// 八个槽位各自猜到了什么。`None` 表示没猜到。
    pub slots: Vec<SlotGuess>,
    /// 时间轴。`uniform` 为假时界面要标出来（重采样不在这一阶段）。
    pub steps: usize,
    pub step_seconds: f64,
    pub step_uniform: bool,
    pub time_units: String,
    /// 高度。源文件没有时是 `None`，界面要让人填。
    pub height_v: Option<f64>,
    pub height_t: Option<f64>,
    pub height_q: Option<f64>,
}

pub struct SlotGuess {
    pub index: usize,      // 1-based
    pub meaning: String,   // "air temperature"
    pub optional: bool,
    pub guessed: Option<String>,   // 猜到的变量名
    pub units: Option<String>,     // 那个变量的 units 属性原文
    pub wants: String,             // CoLM 期望的单位
}
```

**`height_*` 用 `Option` 而不是 `NaN`。** `MetSummary` 里是 `f64::NAN`，
在这里转成 `None` —— `NaN` 过 JSON 会变成 `null` 或报错，而且前端拿到
`NaN` 也没法判断。**转换要在 Rust 侧做完，别把 `NaN` 送出去。**

- [ ] **Step 3: `convert_forcing`**

```rust
/// 按用户确认过的映射转换。
#[tauri::command]
pub fn convert_forcing(
    src: String,
    dst: String,
    slots: Vec<SlotChoice>,
    heights: Option<[f64; 3]>,
) -> Result<String, String> { ... }
```

`SlotChoice { index, name, units, also_add: Vec<String> }`。

**产物路径的约束**（设计文档写下的）：

> 转换产物要与源文件分开存放（**原始数据永远不动**）

所以 `dst` 必须不在 `src` 的目录里 —— 在命令里检查，同目录就拒绝。

- [ ] **Step 4: 测试**

`gui/src-tauri` 有自己的测试（44 passed）。照现有测试的形状写：

- `probe_forcing` 对 PLUMBER2 的 CN-Cng：8 个槽位猜中 7 个
  （第 5 槽 `Wind_E` 是 optional，PLUMBER2 只有标量 `Wind`），
  三个高度都是 `Some(6.0)`
- `probe_forcing` 对 Urban-PLUMBER 的 FI-Kumpula：三个高度都是 `None`
- `convert_forcing` 拒绝产物与源文件同目录

**前两条要跳过条件** —— 那两份数据不一定在（照 `oracle` 里
`PLUMBER2_ROOT` 未设就跳过的做法）。

- [ ] **Step 5: 提交**

```bash
cd gui/src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && cd ../..
cargo run -p xtask -- check-gui
```

---

## Task 3: 强迫场子栏

**Files:**
- Create: `gui/dist/app/forcing.js`
- Modify: `gui/dist/index.html`（换掉前处理页那张「还没有实现」的卡片）
- Modify: `gui/dist/app/main.js`（接线）

- [ ] **Step 1: 先读三个现有模块**

`sites.js`（选文件 + 列表 + 建算例的形状）、`timing.js`（一张卡片
自己渲染自己、改了值就写回后端的形状）、`kernel.js`（**为什么单独一个
模块** —— 循环依赖）。

**`xtask check-gui` 不许模块成环。** 新模块要 import 什么，先想清楚
方向。

### 文件选择器的约定（已核实）

HTML 里这样写就够了，`recent.js` 的 `wirePickers()` 会接线，
选完自动写进那个 input，还会记进 `recent.json` 下次打开时恢复：

```html
<input class="input" id="fsrc">
<button class="btn-ghost pick" data-for="fsrc" data-file="nc">选择…</button>
```

有 `data-file` 走 `pick_file`（filter 是扩展名），没有走 `pick_folder`。

### **但 `wirePickers()` 是一次性绑定，不是事件委托**

```js
for (const b of document.querySelectorAll('button.pick'))   // boot() 里跑一次
```

**动态渲染出来的 `pick` 按钮不会被接线** —— 点了没反应，而且不报错。

所以：**① 选文件那张卡片静态放在 `index.html` 里**，别动态渲染。
后面三张（探测结果、时间轴与高度、转换）在探完之后才有内容，
动态渲染没问题 —— 只要它们里面不含 `pick` 按钮。

若某张动态卡片确实需要 `pick`，渲染完要再调一次 `wirePickers()`，
**并在注释里写明为什么**。

这与之前修过的两个 bug 是同一类：`restoreRecent` 赋值不派发 `change`
（`main.js` 里补了一次显式调用）、`#kernel` 选了不触发 `onchange`
（`recent.js` 里补了 `dispatchEvent`）。**「做了动作，但依赖它的那
一半没跑」—— 这个形状在这个代码库里出现过三次了。**

- [ ] **Step 2: 界面形状**

四张卡片，顺序不可换（后一张依赖前一张的结果）：

| 卡片 | 内容 |
|---|---|
| ① 选文件 | 文件选择器 + 「探一探」按钮。探完才显示后面三张 |
| ② 槽位映射 | 8 行表格：槽位、含义、猜到的变量（下拉可改）、源单位、目标单位。**换算要标出来** |
| ③ 时间轴与高度 | 步长、步数、是否均匀；三个高度输入框（源文件有就填好且禁用，没有就空着等填） |
| ④ 转换 | 产物目录 + 「转换」按钮。**未确认映射不能点** |

**②的每一行要能看出三件事**：猜到了没有、单位要不要换、这一槽是不是
可选的。猜不到且非可选的，④ 的按钮要禁用并说明。

**降水那一行要能加 `also_add`** —— 城市站的 `Rainf` + `Snowf` 靠它。
界面上是「再加一个变量」的下拉。

- [ ] **Step 3: 「确认映射」这一步不能省**

④ 的按钮默认禁用，用户在 ② 上点过一次「这些映射我看过了」才启用。

**这不是形式主义。** 变量名猜错会让模型跑得完但结果全错，而那种错
在界面上看不出来 —— 曲线照样是曲线。让人过一次目是唯一的闸门。

- [ ] **Step 4: 转换产物接到第 3 步**

转换完成后，产物目录应当能被第 3 步（站点）扫到。
**在界面上说清楚下一步做什么**，别让人转完之后不知道往哪走。

- [ ] **Step 5: 验收**

用**Urban-PLUMBER 的 FI-Kumpula** 走一遍（它缺高度、降水分相态，
两条新路径都覆盖）：

1. 选文件 → 探一探
2. 确认第 4 槽是 `Rainf`，加上 `Snowf`
3. 三个高度都是空的 → 填 48.05
4. 确认映射 → 转换
5. 产物里 `Precip = Rainf + Snowf`、`reference_height_*` = 48.05

验收脚本在 scratchpad（`ax_card.sh` / `clickrev.sh` 等）。
**已知坑见 `docs/plan-gui3.md` 的「验收工具的第四个坑」** ——
参数页 AX 树太大会超时，绕法是直接读磁盘产物。

---

## Task 4: 端到端 —— 转出来的能跑

**Files:**
- Create: `oracle/tests/forcing_prep.rs`

- [ ] **Step 1: 拿转换产物建算例跑三段**

照 `oracle/tests/forcing_convert.rs` 的形状（它是 A1 的判据，
已经有全套骨架）。

**这一条用 Urban-PLUMBER**，与 A1 那条（用 PLUMBER2 的 CN-Cng）
互补：

| | A1 的 `forcing_convert.rs` | A2 的这一条 |
|---|---|---|
| 数据 | PLUMBER2 CN-Cng | Urban-PLUMBER FI-Kumpula |
| 单位 | 已是规范单位（恒等） | 同上（`kg/m2/s`） |
| 高度 | 源文件有 | **源文件没有，手填** |
| 降水 | 单个 `Precip` | **`Rainf` + `Snowf` 合成** |
| 内核 | `default` | **`urban`** |

**判据不是「与黄金逐位相同」** —— 城市站没有对应的黄金文件，
而且这条路走的是手填高度（人估的值，不是量出来的）。
判据是：**三段跑完、history 写出来、`f_tref` 在物理范围内**。

- [ ] **Step 2: 若城市内核不在就跳过**

`kernels/urban` 是构建产物，不一定在。照 `forcing_convert.rs`
检查 `manifest.json` 的做法。

- [ ] **Step 3: 提交**

---

## 附：这份计划**不做**什么

- **不做表格导入**（阶段 C）
- **不做时间轴重采样** —— `step_uniform` 为假时只报出来，不修
- **不做站点属性**（阶段 B）
- **不动 A1 的判据** —— `forcing_convert.rs` 那条必须一直绿
- **不引入前端构建工具** —— 纯静态 ES module 是硬约束

## 附：写这份计划时的预期

A1 的计划改错过五处，都是实测推翻的（不可满足的断言、单位写错、
借用链编译不过、缺字段、语法表达不出合成）。

**这份计划里的代码片段同样是照着记忆写的。** 已核实的只有
`MetSummary` 的字段与 `slots::resolve_with` 的签名（上面 §0 那段）。
其余以实际代码为准，**报告里说明改了什么**，不要改实现去迁就规格。
