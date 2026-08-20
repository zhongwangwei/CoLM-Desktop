# 前处理阶段 B：站点属性子栏（实施计划）

> **给执行者：** 用 `superpowers:subagent-driven-development` 按任务逐条实施。

**目标：** 用户只给经纬度就能建出一份能跑的 `site.nc` —— 其余每一类
属性由他选来源（从栅格抽 / 手填 / 模块默认值），**每个值都标出来自哪里**。

**架构：** 现有 `colm_srfdata::site::fill` 是「补齐一份已有的站点文件」，
阶段 B 要把它扩成「从经纬度出发建一份」。加一个 Tauri 命令与一个子栏。

**范围：** 只做站点属性子栏。表格导入是阶段 C。

**A+B 之后**：用户拿一份变量名不同的 netCDF 加一个经纬度，就能建算例
并跑出结果 —— 那是设计文档说的「最小可跑闭环」。

---

## 0. 先读这一节：现有能力的边界（已核实，2026-08-20）

### `site::fill` 的三级回落，覆盖的只是那 12 个字段

```rust
pub const REQUIRED_FIELDS: [&str; 12] = [
    "elevation", "elvstd", "lakedepth", "sloperatio",
    "soil_s_v_alb", "soil_d_v_alb", "soil_s_n_alb", "soil_d_n_alb",
    "soil_texture", "soil_vf_clay", "soil_wf_clay", "soil_wf_om",
];
```

三级：**站点文件自己有 → rawdata 栅格抽 → 模块默认值**。
实测 CN-Cng 上 7 个走到第三级（`soil_*_alb` 四个、`lakedepth`、
`elvstd`、`sloperatio`），结果与黄金文件逐位相同。

### 但土壤剖面是硬性输入，**没有回落**

`read_inputs`（`site.rs:348`）要求这些都在，缺一个就 `?` 掉：

```rust
let lon = scalar("longitude")?;
let lat = scalar("latitude")?;
let landtype = scalar("IGBP_classification")?;     // 城市站没有这个
let col = SoilColumn {
    vf_sand: layers("soil_vf_sand")?,              // 六个 8 层数组
    vf_gravels: layers("soil_vf_gravels")?,
    vf_om: layers("soil_vf_om")?,
    wf_sand: layers("soil_wf_sand")?,
    om_density: layers("soil_OM_density")?,
    bd_all: layers("soil_BD_all")?,
};
```

那六个数组用来推导 `soil_texture` / `soil_vf_clay` / `soil_wf_clay` /
`soil_wf_om` 这四个。**用户只给经纬度时它们不存在** —— 这条路现在
根本走不通，`fill` 会在第一步就报 `soil_vf_sand missing`。

### `USE_SITE_*` 的语义不是「回落到默认值」

`build.rs:230` 那段注释说清楚了：

> `USE_SITE_lakedepth` / `USE_SITE_soilreflectance` /
> `USE_SITE_soilparameters` 三项保持 CoLM 默认的 `.true.`
> （「站点文件里有，就用它」）……于是 `ncio_var_exist` 为假、
> CoLM 照旧**回落到 `lake_depth.nc` 与 `soil_brightness.nc`**

**站点文件缺字段 → CoLM 去读全球栅格**，而那是 240 GB 的东西。
所以「让 CoLM 自己回落」不是一条可用的路 —— 除非用户有 rawdata。

**这就是为什么 `site::fill` 要主动把默认值写进 `site.nc`**：
写进去，CoLM 就不去读栅格了。

---

## Task 1: 土壤剖面缺失时也能补齐

**Files:**
- Modify: `crates/colm-srfdata/src/site.rs`
- Modify: `crates/colm-srfdata/src/site_tests.rs`

- [ ] **Step 1: 写失败的测试**

造一个**只有经纬度**的 `site.nc`（无 `IGBP_classification`、无土壤剖面），
跑 `fill`，期望：

- 不报错
- 12 个 `REQUIRED_FIELDS` 全都在产物里
- `Report` 说得出每个值来自哪里

```rust
#[test]
fn a_site_with_only_coordinates_can_still_be_filled() {
    // **这是阶段 B 的地基。** 用户只给经纬度时，`read_inputs` 会在
    // `soil_vf_sand missing` 上直接失败 —— 那六个 8 层数组是它的硬性
    // 输入，而用户手边多半没有。
    //
    // 期望：那四个由剖面推导的字段（soil_texture / vf_clay / wf_clay /
    // wf_om）走 rawdata 或模块默认值，与另外八个一样。
    let dir = std::env::temp_dir().join(format!("colm-site-bare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let src = dir.join("bare_site.nc");
    {
        let mut f = netcdf::create(&src).unwrap();
        let mut lon = f.add_variable::<f64>("longitude", &[]).unwrap();
        lon.put_values(&[123.5092], netcdf::Extents::All).unwrap();
        let mut lat = f.add_variable::<f64>("latitude", &[]).unwrap();
        lat.put_values(&[44.5933], netcdf::Extents::All).unwrap();
    }

    let dst = dir.join("filled.nc");
    let rep = super::fill(&src, &dst, None, None).expect("只有经纬度也该能补齐");

    let missing = super::missing_fields(&dst).expect("readable");
    assert!(missing.is_empty(), "12 个字段该齐全，缺：{missing:?}");
    // 每个值都要说得出来自哪里 —— 这是 site.rs 已经立下的规矩。
    assert_eq!(rep.sources.len(), 12, "每个字段都要有 source：{:?}", rep.sources);
}
```

**`Report` 的实际字段名以代码为准**（可能不叫 `sources`）。
先读 `pub struct Report`。

- [ ] **Step 2: 跑，确认失败**

期望：`soil_vf_sand missing`（或 `IGBP_classification missing`）。
**贴实际报错。**

- [ ] **Step 3: 实现**

`read_inputs` 改成**容缺**：

```rust
/// 站点文件里读得到什么就读什么。
///
/// **土壤剖面与地类都可能不在。** 用户只给经纬度是阶段 B 的主路径，
/// 而 PLUMBER2 那种带完整剖面的文件是幸运情况，不是前提。
///
/// 城市站点文件也不带 `IGBP_classification` —— `Location` 的注释早就
/// 写明了这件事，只是 `read_inputs` 没跟上。
fn read_inputs(file: &Path) -> Result<Inputs> { ... }

struct Inputs {
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    col: Option<SoilColumn>,
    soil_dim: Option<String>,
}
```

`col` 是 `None` 时，那四个推导字段与另外八个走同一条回落链
（rawdata → 模块默认值）。

**经纬度仍然是硬性的** —— 没有它连栅格都抽不了，
而且用户「只给经纬度」这句话里就它是必给的。

- [ ] **Step 4: 确认既有路径没变**

```bash
cargo test -p colm-srfdata 2>&1 | tail -5
cargo test -p colm-srfdata --test real_sites 2>&1 | tail -5
```

`real_sites.rs` 拿真站点文件跑 `fill`，**它一条都不能红** ——
PLUMBER2 那条路（有完整剖面）必须与改之前逐位相同。

**再跑一次黄金判据**：

```bash
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo test -p oracle --test forcing_convert -- --ignored 2>&1 | tail -5
```

期望 `identical: 129 variables`。**红了就是回归。**

---

## Task 2: 从经纬度建一份最小站点文件

**Files:**
- Modify: `crates/colm-srfdata/src/site.rs`
- Modify: `crates/colm-cli/src/main.rs`（加子命令）

- [ ] **Step 1: `site::skeleton`**

```rust
/// 从经纬度写出一份最小的站点文件，交给 `fill` 补齐。
///
/// **地类是可选的，而且不填比猜一个好。** `build.rs` 那条规矩：
///
/// > 地类只在站点文件说得出时才写。说不出就整条不写 ——
/// > 写一个猜的值比不写更糟，而 CoLM 有自己的回落路径。
pub fn skeleton(dst: &Path, lon: f64, lat: f64, landtype: Option<i32>) -> Result<()>
```

- [ ] **Step 2: `colm-cli site-new`**

```
colm-cli site-new --out <site.nc> --lon <度> --lat <度> [--landtype N]
                  [--rawdata <dir>]
                  # 建一份站点文件：经纬度必给，其余从 rawdata 抽或用
                  # 模块默认值。--landtype 不给就不写，让 CoLM 回落
```

输出要**逐字段说出来自哪里**（`fill` 的 `Report` 已经有这个信息）。

- [ ] **Step 3: 实测**

```bash
./target/debug/colm-cli site-new --out /tmp/mysite.nc --lon 123.5092 --lat 44.5933
ncdump -h /tmp/mysite.nc | grep -c "double\|float"     # 12 个字段该在
```

再拿它建算例跑一遍，确认能跑完。

---

## Task 3: Tauri 命令 + 子栏界面

**Files:**
- Create: `gui/src-tauri/src/sitedata.rs`
- Create: `gui/dist/app/sitedata.js`
- Modify: `gui/src-tauri/src/lib.rs`、`gui/dist/index.html`、`gui/dist/app/main.js`

**走 sidecar**，与 `probe_forcing` 同一条路 —— GUI 进程里不能有 netcdf
（`Cargo.toml` 那条量化过的注释）。

界面三张卡片：

| 卡片 | 内容 |
|---|---|
| ① 位置 | 经度、纬度（必填）、地类（可选，**说明不填会走 CoLM 的回落**） |
| ② 来源 | 每一类属性选来源：从 rawdata 抽 / 模块默认值 |
| ③ 生成 | 产物路径 + 「生成」按钮，产物里每个值标出来自哪里 |

**照 A2 踩过的坑做**：

1. **选文件/目录的卡片静态放在 `index.html`** —— `wirePickers()` 是
   一次性绑定，动态渲染的 `pick` 按钮不会被接线，点了没反应也不报错
2. `recent.js` 的 `REMEMBERED` 要加新字段 —— 那张表漏字段的后果见 `d892622`
3. 状态色只有 `warn` / `fail`，**没有 `.ok`**

---

## Task 4: 端到端 —— 只给经纬度也跑得出结果

**Files:**
- Create: `oracle/tests/site_prep.rs`

走 `colm-cli` 全流程：

```
① colm-cli site-new --out <site.nc> --lon 123.5092 --lat 44.5933
② colm-cli new --site <site.nc> --out <算例> --met <CN-Cng 的强迫场>
③ colm-cli run <算例> --kernel kernels/default
```

**判据三条**：

| 判据 | 证明了什么 |
|---|---|
| 三段跑完，`f_tref` 物理合理 | 这条路能跑通 |
| `site.nc` 里 12 个字段齐全，且每个都有 `source` 属性 | 补齐真的做了 |
| **与用 PLUMBER2 原站点文件跑出来的结果不同** | 用的确实是新建的那份 |

第三条容易漏。**只验「跑通了」是空的** —— 那是 A2 学到的：
用原始文件也跑得通。

---

## 附：这份计划**不做**什么

- **不做表格导入**（阶段 C）
- **不从栅格抽土壤剖面**（那需要 240 GB rawdata；剖面缺失时走模块默认值）
- **不动 A1/A2 的判据** —— 两条端到端必须一直绿
- **不替用户猜地类** —— 不填就不写，CoLM 有自己的回落

## 附：写这份计划时核实过的

- `REQUIRED_FIELDS` 那 12 个的名字（`site.rs`）
- `read_inputs` 的硬性输入（`site.rs:348`）—— **这条推翻了原设计的假设**
- `USE_SITE_*` 的语义（`build.rs:230` 的注释）—— 缺字段是回落到全球栅格，
  不是模块默认值
- 城市站不走 `fill`（`cmd_new` 里 `if looks_like_plumber2`）

**A2 的计划改错过三处，全是没核实就写的。** 这份里的代码片段
同样以实际代码为准，改了什么在报告里说明。
