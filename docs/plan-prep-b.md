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

## 进度（2026-08-20）

| | 任务 | 状态 |
|---|---|---|
| 1 | `fill` 容缺（土壤剖面/地类可以不在） | ✅ `9aa6611` |
| 2 | 从经纬度建站点文件 + `colm-cli site-new` | ✅ `c420040` |
| 2b | `site-new --json`（给界面用） | ✅ `8520ed7` |
| 3 | 站点属性子栏 | ✅ `62d5526` + `ad74cb0` |
| 4 | 端到端 | 进行中 |

### 期间修的、不在原计划里的

**`a2aa222` 「已补齐」不等于「不是城市站」** —— Task 2 为了让
`site-new` 的产物不被误判成城市站，加了 `already_filled` 判据。
但一个**已经建过算例的城市 `site.nc` 同样是 12/12 齐全的**，于是它被
当成 PLUMBER2，`SITE_landtype = 13` 与 `DEF_URBAN_type_scheme = 2`
一个都不写。

**而且这是个退步**：改判据之前那条路会撞 `NC_ENAMEINUSE` 而失败，
加了 `already_filled` 之后变成静默产出错误配置。
**从报错退化成静默错误。**

最终判据拿 `LCZ_DOM` 当第二条证据 ——
**「文件里有什么」比「文件里缺什么」是更硬的证据**：
缺席可以有很多原因，在场只有一个。

### 一条规矩与一条启发式撞车

```
规矩：  地类说不出就不写 —— 写一个猜的值比不写更糟
启发式：站点文件没有 IGBP_classification → 这是城市站
```

两条各自都对。`site-new` 不给 `--landtype` 时**遵守了规矩**，
于是**触发了启发式** —— 产物被当成城市站，要求 240 GB 的 rawdata。

这类冲突查不出来，只能撞上。所以 Task 2 的规格里那条
「**拿产物建一个算例**」是必须的 —— 只验「12 个字段齐全」看不出来。

---

## 「只给经纬度」的实际边界（Task 4 BLOCKED + Task 5 查证）

**这是 B 最重要的一条结论，也是两次实测推翻我规格之后才清楚的。**

从零建一个能跑的单点算例，`mksrfdata` 硬性需要的东西**超出**
`REQUIRED_FIELDS` 那 12 个：

| 字段 | 有没有依据可查 | 结论 |
|---|---|---|
| 12 个 `REQUIRED_FIELDS` | 有（三级回落） | ✅ `fill` 补 |
| `canopy_height` | 有（`htop0_igbp` 查表，按地类） | ✅ `bd747b6` 补 |
| `canopy_bottom_height` | **CoLM 根本不读它** | ❌ 不写 |
| `SAI`（标量） | **CoLM 根本不读它** | ❌ 不写 |
| **`LAI_monthly` + `SAI_monthly`** | **没有表，只能来自数据** | ❌ **必须外部提供** |

### 我的规格错在哪

我在 `MOD_Const_LC.F90` 看到 `hbot0_igbp` 与 `sai0_igbp` 两张表，
就假设「有表 = CoLM 会读」。**错了**：

- 那两张表在 `main/`（**模型初始化**用），读取在 `mksrfdata/`
  （**建面数据**用）—— 两个不同阶段
- `hbot` 从来不从文件读，`mkinidata/MOD_HtopReadin.F90:89` 是
  `hbot(npatch) = htoplc(npatch)*hbot0(m)/htop0(m)` —— 从**已读的**
  `htop` 缩放算出来。写进 `site.nc` 是惰性的
- `SAI` 标量同理。`mksrfdata` 只读 `SAI_monthly`

**「源码里有一张表」不等于「这条路会用它」。** 查表的存在只说明
某个阶段需要那个量，不说明它从哪来。

### LAI 与 SAI 是绑在一起的一对

`MOD_SingleSrfdata.F90:505-506`：

```fortran
u_site_lai = readflag .and. ncio_var_exist(fsrfdata,'LAI_monthly',readflag) &
                     .and. ncio_var_exist(fsrfdata,'SAI_monthly',readflag)
```

**`.and.` —— 缺一个，两个都不用**，一起回落到 `plant_15s/` 栅格。
所以缺口是「LAI + SAI 月气候态」一对，不是「LAI 一个」。

**而且变量名随 LULC 变**（与未决问题 8 那条线直接相关）：

```fortran
#if (defined LULC_IGBP_PFT || defined LULC_IGBP_PC)
   'LAI_pfts_monthly' / 'SAI_pfts_monthly'
#else
   'LAI_monthly' / 'SAI_monthly'
#endif
```

### 所以 B 的承诺是

```
经纬度 + 地类  →  12 个必需字段 + canopy_height（都有依据）
LAI/SAI 月气候态 →  必须来自外部：<rawdata>/plant_15s/，或站点文件自带
```

**不编造季节曲线。** CoLM 的设计里 LAI 从来只从遥感或实测数据读，
伪造一条塞进 `site.nc` 是编造科学输入数据。

这个组合对应一个真实场景：**通量站通常测 LAI，但很少有完整的
土壤剖面**。用户有 LAI 观测、没有土壤数据，正是 B 该服务的人。

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
    // `Report` 用三个列表记，不是一个 map（已核实）：
    let total = rep.from_site.len() + rep.from_raster.len() + rep.from_default.len();
    assert_eq!(
        total, 12,
        "每个字段都要归到某一级：site={:?} raster={:?} default={:?}",
        rep.from_site, rep.from_raster, rep.from_default
    );
    // 只给了经纬度、也没给 rawdata，所以 12 个应当全在 default 里。
    assert!(rep.from_site.is_empty(), "站点文件里什么都没有：{:?}", rep.from_site);
}
```

`Report` 的字段（已核实，`site.rs`）：`texture` / `site_texture` /
`raster_texture` / `texture_name` / `bvic` / `fine_earth` /
**`from_site` / `from_raster` / `from_default`**。

`Source` 枚举有三个值：`Site` / `Raster` / `Default`，
写进产物的 `source` 属性措辞见 `site.rs:233`。

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
