# colm-desktop 设计文档

**日期**：2026-08-17
**状态**：设计已确认，待评审
**作者**：Zhongwang Wei + Claude

---

## 0. 摘要

把 CoLM202X 的 SinglePoint（单站点）模式提取成一个跨平台（Windows / macOS / Linux）的
Rust 桌面程序，让不会编译 Fortran、不会写 namelist 的人也能跑单站点模拟并评估结果。

路线分三阶段，共用同一个 sidecar 进程边界：

- **A**：Rust GUI + Rust 编排 CLI + 预编译 Fortran 内核。物理移植量 0，数值与今天的 CoLM 完全一致。
- **B**：用仓库已有的 shadow-compile 手法替换 Fortran 的 NetCDF 层，Fortran 侧只剩 gfortran。
- **C**：按 20 个移植单元把物理逐组换成 Rust，Fortran 退为开发期数值基准，交付物 100% Rust。

**选 C 作为终点不等于放弃 Fortran。** C 的致命风险是「没有验收标准」，解法是把 Fortran
永久保留为开发期对照基准（仓库已有 13 个 pytest 在用这个手法）。交付物 100% Rust，
开发过程 100% 有 Fortran 对账。

---

## 1. 目标与非目标

### 目标

1. **降低使用门槛** —— 分发一个安装包即可运行，无需 NetCDF / MPI / 编译器环境。
2. **交互式科研工作台** —— 改参数、重跑、看曲线、与观测对比、批量跑站点、参数敏感性。
3. **摆脱 Fortran 技术栈** —— 长期把单点物理内核变成可维护、有测试、内存安全的 Rust。
4. **教学 / 演示** —— 首次使用有引导，能讲清每一步在做什么。

### 非目标

- 不做网格 / 流域 / 非结构模式。SinglePoint 强制 `#undef USEMPI`、`CaMa_Flood`、
  `GridRiverLakeFlow`、`CatchLateralFlow`，这些路径不在范围内。
- 不做 TRACER（本轮明确搁置）。
- 不做 CROP。SinglePoint 下 URBAN 会级联关掉 BGC 再关掉 CROP（见 §2）。
- 不改 CoLM 的输出文件格式。单站点结果要能和社区其他结果对比，换格式就毁掉这个价值。

---

## 2. 已验证的事实基础

本节每一条都在 macOS ARM / gfortran 16.1.0 / netcdf-fortran 4.6.3 上实测过，不是推断。

### 2.1 SinglePoint 今天可以编译，三个物理预设各自成立

| 预设 | 宏（`gfortran -E` 实测） | 结果 | `colm.x` |
|---|---|---|---|
| 水热 | `SinglePoint` `LULC_IGBP` `vanGenuchten_Mualem_SOIL_MODEL` `extend_interception` | rc=0 | 7.9 MB |
| +BGC | 同上但 `LULC_IGBP_PFT` + `BGC` | rc=0, 0 错误 | 13 MB |
| +URBAN | `LULC_IGBP` + `URBAN_MODEL` | rc=0, 0 错误 | 8.6 MB |

三个预设的 `GridRiverLakeFlow` / `USEMPI` / `CROP` 实测均为 OFF。三者合计约 30 MB 进安装包。

调研此前的结论是「没人建过 SinglePoint、CI 零覆盖、可能早已 link 不过」——这条前提已被否证。

唯一失败的目标是 `river_hist_concatenate.x`（postprocess，与单点无关，链接期 `_MAIN__`
符号缺失）。

### 2.2 BGC 与 URBAN 在单点下互斥

`define.h:19-24`：`URBAN_MODEL && SinglePoint` → 强制 `#define LULC_IGBP`；
`:74-78` → `#undef BGC`；`:82-84` → `#undef CROP`。

所以这不是「一个开关」，而是**三个预设**，GUI 里做成单选组，各自对应一个预编译二进制。

### 2.3 MPI 可以彻底甩掉

`make FF=gfortran` 构建 rc=0，`otool -L` 中 4 个 open-mpi dylib 全部消失。
剩余动态依赖即 Windows 上要解决的全部问题：

| 依赖 | 处理 |
|---|---|
| `libnetcdff` / `libnetcdf` | 唯一的真麻烦。Windows 走 MSYS2 的 `mingw-w64-x86_64-netcdf-fortran` |
| LAPACK / BLAS | macOS 走系统 Accelerate（白送）；Windows 走 MSYS2 OpenBLAS |
| `libgfortran` / `libgomp` / `libquadmath` | gcc 运行时，可静态链接 |

`Makeoptions.github:5` 用 `FF ?= mpif90`（`?=` 可被环境覆盖）；`Makeoptions.Mac-arm:6`
是硬 `FF =`，需改。

### 2.4 失败时退出码是 0

| 情形 | 退出码 |
|---|---|
| namelist 文件不存在 | `2`（gfortran runtime error） |
| **namelist 里有未声明的变量名** | **`0`** |
| **`Netcdf error: ... cannot open`（缺 rawdata）** | **`0`** |
| **`OZONE-setgrid.nc does not exist`** | **`0`** |

根因：成功与失败走的是**两条不同的路，却都以 0 结束**。
失败走 `MOD_SPMD_Task.F90:331-349` 的 `CoLM_stop`，其 `#ifndef USEMPI` 分支是裸 `STOP`；
成功不执行任何收尾调用，直接跑到 `main/CoLM.F90:764` 的 `END PROGRAM CoLM`
（`spmd_exit` 只定义并调用于 `#ifdef USEMPI` 内）。详见 §2.15d —— 本节初稿曾写成
「两者都走 `CoLM_stop`」，那是错的。

**这条约束驱动整个 §6 的设计。**

### 2.5 CoLM 的默认 namelist 值假设 HPC 数据树存在

`DEF_USE_OZONEDATA` 默认 `.true.`，会去 `<runtime>/Ozone/Global/OZONE-setgrid.nc`；
`DEF_Runoff_SCHEME` 默认 `3`（Simple VIC），要求站点文件里有 `soil_texture`。

**所以 GUI 的默认值必须和 CoLM 的默认值不同**，否则「双击就能跑」不成立。

### 2.6 静默覆盖是真实存在的，且用户看不到

一次 `mksrfdata` 运行就打印了这些：

```
Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true. when using vanGenuchten_Mualem_SOIL_MODEL.
Note: DEF_TOPMOD_method is set to 0 in SinglePoint.
Note: Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme.
Warning: Nitrification-Denitrification is on when BGC is off. DEF_USE_NITRIF is set to false automatically.
Warning: Latitude mismatch: 44.593299865722656 in data file and 44.593299999999999 in namelist.
```

最后一条印证了「坐标以文件为准、覆盖 namelist、仅打印警告」。

**GUI 必须显示「你要求了 X，模型实际用了 Y」。**

### 2.7 PLUMBER2 站点文件不足以驱动 CoLM 单点

`/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s` 有 90 个站点（5.5 GB），
目录结构正好是 `SiteList` 期望的形式，另有 `Observation/`（评估用观测通量）
和 `Forcingnml/`（每站预制的强迫场 namelist）。

但站点文件缺 12 个 CoLM 无条件读取的字段。`MOD_SingleSrfdata` 的逻辑是
`readflag = ((.not. mksrfdata) .or. USE_SITE_x)`，`u_site_x = readflag .and. ncio_var_exist(...)`
—— **变量缺失时没有第三条路，直接回落到全球 rawdata**。

实测跑通所需的合成字段（附录 A 有可复现步骤）：

| 字段 | 取值 | 依据 |
|---|---|---|
| `lakedepth` | 1.0 | `MOD_SingleSrfdata.F90:47` 模块默认值 |
| `elevation` | 138.0 | 取自同站 `Observation` 文件的 `elevation` |
| `elvstd` | 0.0 | `MOD_SingleSrfdata.F90:88` 模块默认值 |
| `sloperatio` | 0.0 | `MOD_SingleSrfdata.F90:89` 模块默认值（平地） |
| `soil_s_v_alb` / `soil_d_v_alb` / `soil_s_n_alb` / `soil_d_n_alb` | 0.14 / 0.25 / 0.28 / 0.39 | `MOD_SoilColorRefl` 的 L=10 档 |
| `soil_vf_clay` / `soil_wf_clay` | 非砂/砾/有机质剩余量的 25% | 壤土的 1:3 黏/粉比例假设 |
| `soil_wf_om` | `vf_om × OM_density / BD_all` | 由文件已有量推导 |

**`soil_texture` 是第 12 个必需字段**（因为本项目采用 CoLM 默认的 `DEF_Runoff_SCHEME = 3`
Simple VIC）。它不是一个可以随便填的数：`MOD_Initialize.F90:420` 是
`BVIC(ipatch) = BVIC_USDA(soiltext(ipatch))`，而 `BVIC_USDA(0:12)` 是
`(1., 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180, 0.100, 0.090, 0.150, 0.080, 0.050)`
—— 即 **USDA 12 类质地的索引**（1=Sand … 12=Clay）。

因此 `colm-srfdata` 必须实现一个**真正的 USDA 质地三角分类器**：
取 0–60 cm 深度加权的砂/粉/黏重量分数（对应 rawdata 的
`soiltexture_0cm-60cm_mean.nc`），归一化到细土部分后查 USDA 三角。
CN-Cng 实测：砂 14.3% / 粉 64.3% / 黏 21.4% → **第 4 类 Silt loam** → `BVIC = 0.23`
（`mkinidata` 日志确认 `BVIC [-] is in (0.23, 0.23)`）。

`depth_to_bedrock` 由 `DEF_USE_BEDROCK`（默认 `.false.`）门控，不需要。
降尺度字段（`SITE_svf` / `SITE_cur` / `SITE_sf_lut` / `SITE_slp_type` / `SITE_asp_type` /
`SITE_area_type`）由 `DEF_USE_Forcing_Downscaling`（默认 `.false.`）门控。

**这是 `colm-srfdata` crate 存在的核心理由，也是 GUI 里必须有的一个「补全缺失地表参数」环节。**
调研完全没预见到这一点，因为它只有真去跑才会暴露。

**`DEF_dir_runtime` 则不同 —— 水热预设下实测零依赖**：

| runtime 数据 | 触发者 | 不提供的代价 |
|---|---|---|
| `Ozone/Global/OZONE-setgrid.nc` | `DEF_USE_OZONEDATA`（**默认 `.true.`**） | `MOD_Ozone.F90:82-84` 退回硬编码常数 `forc_ozone = 100._r8` ppbv。臭氧胁迫仍在起作用（本次输出 `f_o3uptakesun = 3.56` 非零），只是用的不是真实臭氧场。**这是唯一一个默认开、必须显式关的** |
| `snicar/snicar_optics_5bnd_mam_c211006.nc` + `snicar_drdt_bst_fit_60_c070416.nc` | `DEF_USE_SNICAR`（默认 `.false.`） | 默认即关。开启才需要；对高纬 / 积雪站点的雪反照率影响大 |
| `vic/vic_para.txt` | VIC 产流（`DEF_Runoff_SCHEME` 1 或 3） | 用 TOPMODEL（0）时不涉及 |
| `nitrif/CONC_O2_UNSAT/*.nc` | BGC + `DEF_USE_NITRIF`（默认 `.true.`） | 水热预设下 BGC 关 → `DEF_USE_NITRIF` 被自动关（日志实证）。**BGC 预设下需要** |
| `HydroLAKES_Reservoir.nc` | 水库 | 单点不涉及 |

结论：**水热预设零 runtime 依赖**（代价是臭氧用常数 100 ppbv）；**BGC 预设需要 nitrif 数据**。
真实臭氧场与 SNICAR 光学表应做成**可选数据包**下载，而非随安装包必装。

**更好的做法**：这些 rawdata 是全球网格（`colm_500m` = 86400×43200），单点只需 1 个像元。
不要搬几百 GB —— 给每个站点做一次**站点参数包抽取**，每站几 KB。附录 B 是完整文件清单。

### 2.7b 本机的 rawdata 校验过合成值，而里程碑 3 把真值烘了进去

本机存在：

```
~/Desktop/colm-rust/rawdata/   38 GB   lake_depth.nc (49 MB) / soil_brightness.nc (28 MB)
                                       / topography.nc (38 GB) / soil/ (26 个文件)
~/Desktop/colm-rust/runtime/   2.8 GB  Ozone/Global/OZONE-setgrid.nc (2.8 GB)
                                       / snicar/*.nc (468 KB) / nitrif/ (30 MB)
```

把 `USE_SITE_lakedepth` / `soilreflectance` / `topography` 置为 `.false.`、
`DEF_dir_rawdata` 指向真实数据重跑 `mksrfdata`，让 CoLM 自己去抽点（而不是我们
复现它的网格逻辑），得到的 `srfdata.nc` 与最初那版合成值**42 个变量里只有 4 个不同**：

| 变量 | 最初合成值 | 真实 rawdata | 说明 |
|---|---|---|---|
| `elevation` | 138 | 144.14 | 合成用的是塔的高程（取自 Observation 文件），真实值是 500 m 格点均值 |
| `elvstd` | 0 | 0.4963 | 次网格高程标准差 |
| `lakedepth` | 1 | 0 | 对草地斑块（IGBP 10）无影响，该量只用于湖泊斑块 |
| `sloperatio` | 0 | 0.003576 | 注意：`mksrfdata` 日志按 `F8.2` 打印为 `0.00`，看日志会误以为相同 |

**四个土壤亮度反照率完全命中**（0.14 / 0.25 / 0.28 / 0.39）。

**本节原先的决定（「黄金基准保持合成版」）已被里程碑 3 推翻。** 当时留了一句
「正确做法不是让用户装 rawdata，而是把上表那 4 个真实值烘进入库的 `site.nc`」——
`crates/colm-srfdata` 做的就是这件事，两个黄金文件已随之重新生成。`elevation`
是唯一的例外：它保持 138，因为**站点自有的值优先于栅格**（塔的实测高程，见
`oracle/fixtures/PROVENANCE.md`），144.14 是 500 m 格点均值而不是这个站点的高程。

`elevation` 之外的三项与四个反照率现在都取自栅格，fixture 仍是自包含的
37 KB，复现不需要那 38 GB。

**runtime 的可携带性**：`snicar/` 只有 468 KB，若要开启雪粒径演化辐射
（`DEF_USE_SNICAR`）可以随包分发；`Ozone/` 2.8 GB 不可能随包，所以
`DEF_USE_OZONEDATA = .false.` 与常数 100 ppbv 的偏离（§2.7 的 runtime 表）
继续成立，除非将来抽取单点臭氧时间序列。

### 2.8 端到端已跑通，且物理正确

站点 CN-Cng（2008-01-01 → 01-11，1800 s 步长，逐小时输出）：

```
mksrfdata.x  → Successful in surface data making.  → landdata/srfdata.nc
mkinidata.x  → CoLM Initialization Execution Completed → restart/{const,2008-001-00000}/*.nc
colm.x       → CoLM Execution Completed.          → history/CN-Cng_hist_2008-01.nc
```

耗时 **1.8 秒**（10 天）。history 文件 1.3 MB / 129 个变量 / 264 个时间步。
`f_xerr`（水量平衡误差）与 `f_zerr`（能量平衡误差）均为 0。
`VSF scheme all steps: 528 (implicit) 6 (explicit) 0 (wet2dry)` —— 变饱和流求解器
真的在跑，且有 6 步显式回退（正是 §2.11 的路径依赖来源）。

**与观测对比**（`Observation` 文件，仅用 `qc==0` 的实测点，剔除冷启动前 8 小时）。
两种产流方案都跑过，`DEF_Runoff_SCHEME = 3`（Simple VIC，CoLM 默认，本项目采用）为准：

| 变量 | n | RMSE | bias | R² | KGE |
|---|---|---|---|---|---|
| 净辐射 Rnet | 256 | 14.7 | **−0.87** | **0.986** | +0.829 |
| 感热 Qh | 253 | 46.1 | +34.9 | 0.530 | **−11.56** |
| 潜热 Qle | 254 | 32.2 | +13.3 | 0.044 | −1.42 |

（`DEF_Runoff_SCHEME = 0` TOPMODEL 对照：Rnet RMSE 15.2 / bias +0.22 / R² 0.986；
Qh RMSE 39.2 / bias +24.3 / R² 0.459；Qle RMSE 32.0 / bias +17.1 / R² 0.115。
`f_zwt` 从 TOPMODEL 的 `[0.108, 2.231]` 变为 Simple VIC 的 `[0.0009, 0.504]`
—— 产流方案确实改变土壤水文。）

**Rnet 的 R²=0.986 是关键验证信号**：它同时证明强迫场转换、时间轴对齐、时区处理、
经纬度定位、辐射物理全部正确。若任一环出错（时区偏 8 小时、单位错、坐标错位），
Rnet 不可能对到 0.986。

感热/潜热偏高是 10 天冷启动无预热 + 合成土壤参数的预期结果。

**两条由此暴露的设计约束：**

1. **黄金算例窗口必须包含降水事件。** 1 月 1–11 日窗口内 `f_rnof` / `f_rsur` /
   `f_rsub` / `f_rsur_ie` / `f_rsur_se` **全程为 0**（冻结期无降水），产流分支一行未执行。
   **已补湿季窗口**（见 §2.8b）。
2. **KGE 在观测均值接近 0 时不可用。** 感热 KGE = −11.56，纯粹因为冬季观测均值趋近 0
   使 β = 模拟均值 / 观测均值 爆掉。`colm-hist` 必须对 KGE 做保护：
   `|observed mean|` 低于阈值时报 N/A，而不是输出一个荒谬数字。

### 2.8b 第二份黄金输出：湿季窗口（已跑通）

在 2008–2009 全期（总降水 665.7 mm）中滑窗求 11 天累计降水最大值，得
**2008-07-05 起累计 101.0 mm**。据此跑 `CN-Cng-wet`：2008-07-01 → 07-16
（前 4 天作预热），`DEF_Runoff_SCHEME = 3`。三段全部 rc=0 / 成功标记齐 / 0 错误标记。

**产流路径覆盖（对比冬季窗口的全 0）**：

| 变量 | 非零步数 | 结论 |
|---|---|---|
| `f_xy_rain` / `f_rnof` / `f_rsur` / `f_rsur_se` | 44 / 384 | 饱和超渗产流**已覆盖** |
| `f_qinfl` | 340 / 384 | 入渗**已覆盖** |
| `f_zwt` | 377 / 384 | 地下水位动态**已覆盖** |
| `f_rsur_ie`（超渗产流） | **0 / 384** | **仍未覆盖** |
| `f_rsub`（地下产流） | **0 / 384** | **仍未覆盖** |

平衡误差：`|f_xerr|max = 3.70e-08`、`|f_zerr|max = 2.72e-10`，
远在 `CoLMDEBUG` 阈值（0.5 W/m²、1e-3 mm）内。

**与观测对比**（剔除前 4 天预热，观测仅 `qc==0`；7 月生长季通量量级大，指标比 1 月有意义）：

| 变量 | n | RMSE | bias | R² | KGE | 观测均值 |
|---|---|---|---|---|---|---|
| 净辐射 Rnet | 287 | 13.5 | −2.95 | **0.999** | +0.939 | 121.7 |
| 潜热 Qle | 278 | 76.5 | +36.4 | **0.853** | +0.362 | 84.4 |
| 感热 Qh | 287 | 36.3 | −24.9 | 0.456 | −1.55 | 9.9 |

Qle 偏高 +36.4、Qh 偏低 −24.9，净 +11.5，而 Rnet bias 仅 −2.95 ——
**模型把能量更多分配给潜热、更少给感热，但总量守恒**。这是连贯的物理行为
（默认参数 + 合成质地/黏粒的预期结果），不是缺陷。
Qh 的 KGE = −1.55 再次印证近零均值保护的必要（观测均值仅 9.9）。

**两个窗口共同构成里程碑 1 的回归基准。** `f_rsur_ie` 与 `f_rsub` 仍需第三个窗口
或人工构造的强降水/深湿润算例才能覆盖 —— 记入未决问题。

### 2.9 冷启动瞬变必须处理

`f_fsena` 逐小时：`736.2 → 334.5 → 307.8 → 305.7 → 262.8 → 236.8 → 217.5 → 227.4`，
第 9 小时起落到 `[-37.7, 231.2]`。`f_fgrnd` 同样：`-1051 → -510 → ... → -335`。

**GUI 必须处理 spin-up**：跑预热期（`DEF_simulation_time%spinup_*`），或从评估中剔除
前 N 天。默认应当二者都做，并把「评估用了哪些时段」显式告诉用户。

### 2.10 I/O 契约的实测细节

**强迫场（`Forcing/<SITE>_<yr1>-<yr2>_<network>_Met.nc`）**

- `NVAR=8`，`vname = 'Tair' 'Qair' 'Psurf' 'Precip' 'NULL' 'Wind' 'SWdown' 'LWdown'`
  —— **第 5 槽是 `'NULL'`**，标量风速进第 6 槽。
- `tintalgo = 'linear' 'linear' 'linear' 'nearest' 'NULL' 'linear' 'linear' 'linear'`。
- 变量形状 `(time, y, x)`，Fortran 侧看即 `(1,1,time)`；维度名必须字面是 `time`。
- 时间单位实测是 `"seconds since 2008-01-01 00:00:00"`。Fortran 在**硬编码字符偏移**
  处解析（`MOD_Forcing.F90:1253-1255`）：第 15:18 位取年、…、第 32:33 位取秒。
  验算：`s e c o n d s _ s i n c e _ 2 0 0 8` → 第 15–18 位正是 `2008`。
  所以只有 `"seconds since YYYY-MM-DD HH:MM:SS"` 这一种写法可用。
  **纠正**：先前这里写「换成 `hours since` 会错位且不报错」——**不对**。
  照抄那段逻辑实测：`hours since` 与 `days since` 都让 `read` 返回 `iostat=5010`，
  连不补零的 `"seconds since 2008-1-1 0:0:0"` 也是。而 CoLM 的调用没有 `iostat`，
  于是直接以 Fortran 运行期错误终止 —— 失败是响亮的，且 `Fortran runtime error`
  正在 `colm-kernel` 的失败标记里。脆是真脆，静默则不是。
  实测 90 个 PLUMBER2 强迫场文件的单位全是 `seconds since`，且 `history` 属性显示
  是有人用 `ncatted` 显式改成这样的 —— 这个语料被预处理过，不是天然如此。
- 数据质量：35089 个半小时步、零 NaN、量级合理。
- 总降水按 2/3 大尺度 + 1/3 对流拆分；POINT 唯一的预处理是对 q 做 `qsadv` 饱和钳制，
  而该钳制在 `#ifdef SinglePoint` 内 —— `dataset='POINT'` 但 SinglePoint 关闭时静默不生效。

**站点地表数据**：`soil = 10` 层。

**生成的 `srfdata.nc`**：`patch = 1`，**`soil = 8` 层**。

**history**：`soil = 10` 层。

→ **10 → 8 → 10 的有损往返已实测证实**（日志中每个 `soil_*` 只打印 8 个值）。
`mksrfdata` 只取前 8 层，`MOD_SoilParametersReadin` 再广播回 10 层。
这是 C 阶段必须精确复刻的载荷性数值行为。

**restart 文件名**：block 后缀实测是 `_w180_s90`（不是按站点经纬度派生）。
同时产出带后缀和不带后缀两个 const 文件。

**history 时间轴**：`minutes since 1900-1-1 0:0:0`，首值 56802270，步长 60 分钟。
2008-01-01 00:00 = 56802240，故**逐小时输出回移了正好 30 分钟**，印证
「按平均区间一半回移（HOURLY −30 / DAILY −720 / MONTHLY −21600 / YEARLY −262800）」。

**history 维度**：`patch` `soil=10` `soilinterface=11` `soilsnow=15` `lake=10`
`vegnodes=4` `band=2` `rtyp=2` `time` `sensor=1`（`sensor` 是调研未提及的）。

### 2.11 C 阶段的数值陷阱（已定位）

- `main/HYDRO/MOD_Hydro_SoilWater.F90`（3,657 行 / 2,628 有效，单点路径最大文件）——
  变饱和 Richards，自适应子步进（`:766`）、嵌套 Newton（`:795`）、容差 `8.e-8`，
  **三个割线迭代在 50 次后放弃并打印警告而不是报错**（`:2836/2874`、`:3015/3044`、`:3187`），
  携带 `integer(8), SAVE` 迭代计数器（`:3604-3606`）。
- `share/MOD_IncompleteGamma.F90`（943/685）—— ACM Algorithm 654 原样照抄：
  **87 个 GOTO、51 个 DATA 语句、无 `IMPLICIT NONE`**，裸 `REAL` 被 `-fdefault-real-8`
  静默提升为 f64。在单点执行路径上（`MOD_Runoff.F90:108` 调 `GRATIO`）。
- `main/MOD_AssimStomataConductance.F90`（844/444）—— 唯一带 F77 编号 DO 循环 + 计算 GOTO
  的文件；对胞内 CO₂ 做插入排序 + 二分。
- `extends/interception/MOD_LeafTemperature_Extended.F90`（1,940/1,246）—— 40 次准 Newton
  迭代耦合叶片能量平衡与 Monin-Obukhov，Obukhov 长度在 4 次符号振荡后硬重置。
- `main/MOD_SnowSnicar.F90`（3,000/1,691）—— Toon 两流 / Delta-Eddington，42 个模块级
  光学表、15 个 DATA 语句，**反照率算成负数时切换两流近似并把太阳天顶角扰动 0.02**
  重试至多 20 次。
- **状态量本身就是难点**：单点执行集有 598 个过程、**748 个模块级 `allocatable` 声明**
  （`MOD_Vars_1DAccFluxes` 一个文件就有 422 个累加数组）、59 处 `SAVE`。
  Rust 不能镜像成全局量 —— **显式状态结构体的设计是第一天的决定，不是后续重构。**
- `share/MOD_Utils.F90:2533` 的 `tridia` 是 6 个模块 / 8 处调用背后唯一的数值内核
  （地温、土壤/雪水文、湖泊、冰川、海洋、植物水力 ×3），其舍入行为决定所有隐式列求解。
- 全仓库 **0 个 EQUIVALENCE / COMMON / ENTRY**。遗留别名不是风险，迭代收敛和全局
  可变状态才是。
- 构建带 `-ffpe-trap=invalid,zero,overflow`，Fortran 基准依赖守卫子句永不产生 NaN/Inf。
  Rust 若在 Fortran 会中止的地方静默产出 NaN，就会无声偏离。

### 2.12 shadow-compile 陷阱

仓库默认 `#define extend_interception`，Makefile 会把 4 个 `main/` 目标过滤掉，
改用 `extends/interception/*_Extended.F90` 以**相同模块名**重建。
`main/` 里有 5,441 行是死代码，**不得移植**。
而且 extends 版不是重构：它是 **8 个运行时可选的截留方案**
（`DEF_Interception_scheme` 1..8：CoLM2014 / CoLM202x / CLM4 / CLM5 / Noah-MP /
MATSIRO / VIC / JULES），`main/` 版只有 1 个。

### 2.13 移植面（实测 LOC）

仓库总计 381 个 `.F90` / 258,821 行。单点最小配置（BGC/CROP/TRACER 关）下，
`colm.x` 编译 228 个文件 / 168,576 原始行 → CPP 后 94,691 / 有效 61,272 行。

**执行路径**（从 `CoLM.F90` 出发的 `USE` + 外部 `CALL` 闭包，在预处理后文本上计算）：
**91 文件 / 89,975 原始行 / 71,708 预处理行 / 46,746 行有效代码**。
这是链接可达性闭包，即上界。4 个文件在 `LULC_IGBP` 下可证不执行，扣除后
约 87 文件 / 44,436 有效行（减法得出，未独立测量）。

前 6 大移植单元：

| 移植单元 | 文件 | 有效行 |
|---|---|---|
| 网格/网/pixelset/空间映射 | 13 | 5,164 |
| 叶温 / 光合 / 植物水力 | 7 | 4,795 |
| 土壤水文 / Richards | 4 | 4,201 |
| NetCDF I/O | 6 | 3,699 |
| 大气强迫 + 边界数据集 | 8 | 3,124 |
| 地表辐射 / 反照率 / 雪光学 | 6 | 3,030 |

**约 40% 是基础设施而非物理**：namelist/IO/网格/单点读入/全局容器/history/驱动
共 18,632 有效行，物理 28,114 行。若 Rust 内核以数组进、数组出的薄适配层工作，
物理目标约 30,000 有效行。

加上 BGC 需再算 `main/BGC` 17,829 原始行（有效行未测）。

### 2.14 验收基准的现状

**已有**：
- **差分基准模式**（85 个 pytest 中的 13 个）—— 把真实 Fortran 函数原样抽出、套 stub
  模块 + stub `define.h` + `MOD_Precision` 编译、stdin 喂输入元组、`ES24.16` 打印、
  Python 侧带容差比对。`tests/test_tracer_isotope_frac_runtime.py` 是模板。
  **这是最可迁移的资产，且对任何模块都能按函数使用。**
- `tests/river_hist_compare.py` —— 通用的、基于发现的 NetCDF 目录差分器（先逐位再
  rtol/atol）。除河道分片文件名跳过逻辑外可直接复用。
  `tests/river_hist_schema_lock.py` 明确是 producer-independent 的。
- **内建物理不变量** —— `CoLMMAIN.F90:1545` 的 `|errore| > 0.5` W/m²、`:1620` 的
  `|errorw| > 1.e-3` mm → `CoLM_stop()`。仅在 `CoLMDEBUG` 下武装，仓库默认 `define.h`
  关闭，但 `run/scripts/create_newcase` 生成的站点算例里是 `#define` 的。
- `run/scripts/SiteList` —— 90 站机读清单。
  `run/scripts/create_test_standard-sites` —— 10 个启用算例 × 90 站、切换 13 个 `DEF_*`。

**缺失**：
- **单点零测试覆盖**（85 个 pytest 里 `SinglePoint` 出现 0 次，仅 2 处无关字符串）。
- **零 CI 覆盖**（`TestCaseLists` 全部 91 行都是 GRID；矩阵作业只编译不运行）。
  `create_defineh.bash` 已接受 `SinglePoint` 作为第 1 参数，加一行即可。
- **无任何标准输出**。本文档 §2.8 的 CN-Cng 运行是单点模式历史上第一份。
- **无容差策略**。任何重排浮点求和顺序的移植都不会逐位一致，而 `MOD_Hydro_SoilWater`
  的迭代次数退出和 `MOD_SnowSnicar` 的天顶角扰动让基准答案**本身依赖**浮点细节。
- **唯一提交的单点 namelist 是坏的**：`run/examples/SiteSYSUAtmos_IGBP_VG.nml:19-20`
  设置了 `USE_SITE_topostd` 和 `USE_SITE_BVIC`，而 `MOD_Namelist.F90` 只声明 17 个
  `USE_SITE_*`，两者都不存在 → `iostat` 检查触发 `CoLM_Stop`。两行的修复。

### 2.15 顺手发现并已修复的 bug

- `create_defineh.bash:228-229` 发出 `#error "TRACER requires GridRiverLakeFlow"`，
  而 `include/define.h:121-126` 已明确说明陆面示踪物不需要河道路由、因此不报硬错。
  **生成脚本挡住了源码本来支持的配置。** 已删除该 guard 并换成 `define.h` 的说明注释，
  验证方式是真去编了一遍 `SinglePoint + TRACER`：`TRACER_ON` / `GRLF_OFF` / rc=0 /
  `colm.x` 9.7 MB。
- `.gitignore` 已加入 `.superpowers/`。

**未修复（需决策）**：`create_defineh.bash` 发出 `LATERAL_FLOW`，而该宏在 `.F90` 中
出现 **0 次**（真名 `CatchLateralFlow`，23 个文件在用）。因此所有生成的头文件里侧向流
都是静默关闭的，包括 CI 那 91 个用例。修正它会让 CATCHMENT 用例第一次真的编译侧向流
代码路径，可能直接使 CI 失败 —— 属于需要单独评估的变更。

### 2.15b netcdf 必须静态链接；依赖钉版本的必要性被我自己的实验误判过

**先纠正一个我发布过的错误结论。** 我曾断言 `netcdf = "0.12"` 配 `features = ["static"]`
「根本无法解析，实测报 links 冲突」。**这是错的。** 在全新目录、无任何 lockfile 的条件下
它解析得很干净：

```
netcdf 0.12.1 / netcdf-sys 0.9.2 / netcdf-src 0.5.3 / hdf5-metno-sys 0.12.2
```

那次冲突是实验被污染造成的：我用 `cp -r` 复制探针目录时把上一次的 `Cargo.lock` 一起带了过去，
而它把 `netcdf-src` 钉在 **0.5.0** —— 只有 0.5.0 及以前才要求 `hdf5-metno-sys ^0.11`，
`netcdf-src 0.5.2` 起已改为 `^0.12.2`，与 `netcdf-sys 0.9.2` 自洽。
**教训：复制目录做依赖实验时 `Cargo.lock` 会跟着走，把结果锁成上一次的形状。**

**真正成立的结论是：静态链接是必需的，理由与依赖解析无关。**

| 项 | 动态链接 | 静态链接 |
|---|---|---|
| 构建 | 通过 | 通过，约 45 秒（编 HDF5 + netcdf-c + zlib） |
| 产物动态依赖 | libnetcdf / libhdf5 / … | **只有 `libiconv` 与 `libSystem`** |
| 运行 | **失败**：`Library not loaded: @rpath/libnetcdf.22.dylib … no LC_RPATH's found` | 清空所有环境变量后直接读黄金文件成功 |

动态链接的产物连开发机上都跑不起来（链接期通过 `nc-config` 找到了库，却没写入 rpath），
打包出去自然更不可能。回退方案（运行时 `DYLD_LIBRARY_PATH`、构建期嵌 rpath）实测都可行，
但都不满足「分发一个安装包即可运行」这一第一目标。

**两个依赖图都已验证可用**（都是静态构建、都读了真实黄金文件、都 129/129 变量成功）：

| 组合 | netcdf-sys | 静态链入的 HDF5 | 构建 | 读黄金文件 |
|---|---|---|---|---|
| 最新 | 0.9.2 → netcdf-src 0.5.3 | 2.2.0 | 44.6 s | ✓ 129/129 |
| 钉住 | 0.9.0 → netcdf-src 0.5.1 | 2.0.0 | 45.5 s | ✓ 129/129 |

因此**不钉版本**：两者都能用，由入库的 `Cargo.lock` 冻结实际解析结果即可。
曾经写过的 `netcdf-sys = "=0.9.0"` 是**无效的**——`[workspace.dependencies]` 只是模板，
没有任何成员在自己的 `[dependencies]` 里引用 `netcdf-sys`，于是它照样浮到 0.9.2。
这与 §2.15c 的 MSRV 是同一个 bug 类型。

**方向性纠正：黄金文件的字节由 Fortran 侧决定，不由 Rust 侧决定。**
我原先用来论证 `Cargo.lock` 入库的理由是「浮动的 netcdf crate 会改变产出文件」——
方向错了。黄金文件是 `colm.x` 写出来的，它链接的是**系统**的 netcdf-fortran；
Rust 判官只负责读。本机实测的 Fortran 侧版本：

```
netCDF-C 4.9.3 / netcdf-fortran 4.6.3 / HDF5 1.14.6   （均来自 miniforge）
```

而 Rust 判官用静态链入的 HDF5 2.x 读这些文件，实测正常（前向兼容成立）。

所以真正的风险是：**Fortran 侧的 netcdf/HDF5 一旦变化，黄金文件的字节就会变，
而 `Cargo.lock` 对此毫无约束。** 这三个版本号必须记入 Task 5 的内核清单
（`manifest.json`），那才是它们该待的地方。`Cargo.lock` 入库依然正确，
但它保证的是「判官本身可复现地构建」，不是「黄金文件字节不变」。

**API 已实测可用**（`netcdf 0.12.x`）：`open` / `dimensions()` / `attributes()` /
`attribute(n).value()` / `AttributeValue` / `variables()` / `variable(n)` / `name()` /
`get_values::<f64, _>(Extents::All)`。黄金文件的 **129/129 个变量都能按 `f64` 读出**
（含 8 个 `int` 变量），`create_time` 读出为 `Str("20260817-16:27:52 UTC+08:00")`。

### 2.15c `[workspace.package]` 与 `[workspace.dependencies]` 都只是模板

成员必须逐字段写 `field.workspace = true` 才继承。实测：workspace 声明
`rust-version = "1.99"` 而成员未 opt-in 时，在 rustc 1.97.1 上**照样编译成功**；
加上 `rust-version.workspace = true` 后才正确报 `requires rustc 1.99`。

`[workspace.dependencies]` 同理：没被任何成员引用的条目不会被解析、也不会被钉住。

本项目在同一轮里被这个机制咬了两次（先是 `rust-version`，然后是 `netcdf-sys`），
所以采用 `resolver = "3"`：它是 MSRV 感知的，且实测**不会硬失败**——
无兼容版本时按 `incompatible-rust-versions = "fallback"` 回退并标注。
它当场暴露出一处不准确：`hdf5-metno-src 0.10.4 (requires Rust 1.85.1)`，
而我们原本声明 1.85，故 MSRV 下限改为 **1.85.1**。

### 2.15d `CoLM_stop` 是失败专用的（纠正）

我曾写「成功路径和所有失败路径都走 `CoLM_stop`」。**成功路径那半句是错的**，已验证：

- `spmd_exit` 只定义在 `share/MOD_SPMD_Task.F90` 的 `#ifdef USEMPI` 内（第 313 行）。
- 唯一的 `CALL spmd_exit`（`main/CoLM.F90:761`）同样在 `#ifdef USEMPI` 内。
- `END PROGRAM CoLM`（第 764 行）在任何宏之外。

所以 SinglePoint 下成功路径不执行任何收尾调用，直接跑到 `END PROGRAM` 正常终止（退出码 0，
实测确认）；`CoLM_stop` 只在失败时被走到。

结论（退出码无法区分成败）依然成立，但成立的方式是**两条不同路径巧合地都返回 0**，
而不是共用一条路径。这个区别有实际后果：**既然 `CoLM_stop` 是失败专用的，
把它 `#ifndef USEMPI` 分支的裸 `STOP` 改成 `STOP 1` 就是安全的** ——
那是个能让单点模式退出码真正有意义的上游修复，值得给 CoLM 单独提 PR。
原先那个错误描述会让人以为「那样会连成功路径一起破坏」，从而过度论证日志解析式判定。

即便上游修好了，§6.3 的三件套仍然需要：产物硬校验能抓住「跑完了但没写出该写的文件」，
错误标记扫描能抓住部分失败。退出码只是多了一条廉价的第一道信号。

### 2.16 被本次验证否证的调研结论

| 调研结论 | 实测 |
|---|---|
| 「没人建过 SinglePoint，可能早已不能编」 | 三个预设全部 rc=0 |
| 「`SITE_soil_BA_alpha`/`BA_beta` 从不从任何文件读取，在代码里推导」 | 站点文件里存在，且日志显示 `(from SITE)` |
| 「restart block 后缀按站点经纬度派生，如 `e113_n22`」 | 实测为 `_w180_s90` |
| 「`define.h` 只有两个 `#error` guard」 | 生成的头文件有第三个（已修） |

---

## 3. 三阶段路线

| | 内容 | 交付物 | Windows 难度 | 物理移植 |
|---|---|---|---|---|
| **A** | GUI + Rust 编排 CLI + 预编译 Fortran 内核 | 能装能用的桌面程序 | 中（打通一次 MSYS2 netcdf-fortran） | 0 行 |
| **B** | shadow-compile 替换 Fortran 的 NetCDF 层 | Fortran 侧只剩 gfortran | 低 | 0 行，但要写 I/O shim |
| **C** | 20 个移植单元逐组换 Rust | 100% Rust 交付物 | 最低 | ~44,400 有效行 |

三者共用同一个 sidecar 边界，因此是同一条路上的三个里程碑，不是三选一。
**GUI 在 B 和 C 阶段一行都不用改** —— 这正是 EarthMesh 的 GUI 能完整活过引擎重写的原因。

每个阶段各自走一遍 spec → 实施计划 → 实施。本文档只覆盖 A 阶段。

---

## 4. A 阶段架构

### 4.1 仓库与 workspace

独立仓库 `colm-desktop`，`vendor/CoLM202X` 作为 submodule pin 在具体 commit。

**两个 Cargo workspace**（照抄 EarthMesh 的刻意分离，避免 `cargo test --workspace`
把 webkit2gtk 拖进来）：

```
colm-desktop/
├── Cargo.toml              引擎 workspace，version 用 workspace.package 继承
├── crates/
│   ├── colm-namelist/
│   ├── colm-schema/
│   ├── colm-forcing/
│   ├── colm-srfdata/
│   ├── colm-kernel/
│   ├── colm-hist/
│   ├── colm-case/
│   └── colm-cli/
├── gui/                    独立 workspace
│   ├── src-tauri/
│   └── dist/               无 npm 的静态前端 + vendored uPlot
├── kernels/                各平台各预设的 Fortran 二进制 + manifest.json（构建产物，不入 git）
├── oracle/                 Fortran 对账基准工具（永久资产，见 §8）
└── vendor/CoLM202X/        submodule
```

### 4.2 crate 边界

| crate | 职责 | 为什么独立 |
|---|---|---|
| `colm-namelist` | Fortran namelist 读写，**保留注释与未识别字段** | 手写解析器。边界已验证有限：17 个 group 名、55 个文件、最长 354 行、无 slice / 无 repeat count / 无 sequence-to-subscripted 赋值。带注释往返是现有 crate 都不提供的 |
| `colm-schema` | `DEF_*` 元数据（名、类型、默认值、范围、依赖、所属 group、静默覆盖规则） | **从 `vendor/CoLM202X/share/MOD_Namelist.F90` 离线代码生成**，产物入库，CI 校验漂移。CoLM 会持续演进，手写 schema 必然静默失配 |
| `colm-forcing` | PLUMBER2/FLUXNET → CoLM POINT 强迫场 | 要精确复刻 §2.10 的全部怪癖 |
| `colm-srfdata` | 站点地表数据读写 + **缺失字段合成** + **USDA 质地三角分类器** + 站点参数包抽取 | §2.7 是它存在的理由 |
| `colm-kernel` | 内核发现 + sha256 清单握手、三段编排、run lease、PID kill、日志事件、**成败判定三件套** | §2.4 是它存在的全部理由 |
| `colm-hist` | **「这个内核能产出哪些变量」的闸门表（已实现，§5.3）**；以及读 `_hist_*.nc`、时间轴还原、抽稀、指标（RMSE/R²/bias/KGE/MAE，**KGE 需近零均值保护**）、按 `_qc` 筛选观测、剔除 spin-up（尚未实现） | §2.10 的时间轴、§2.8 的 QC 筛选与 KGE 陷阱；闸门表另见 §5.3。闸门表这一半刻意**无依赖** —— netcdf 不进这个 crate，否则 GUI 为了问一句「能产出什么」要拖进整个 hdf5 |
| `colm-case` | 算例布局、`run_manifest.json`、批量调度、敏感性矩阵、复现包导出 | 「批量」「敏感性」「可复现」三个需求的归属 |
| `colm-cli` | 唯一的编排可执行文件 | GUI 只跟它说话 |

### 4.3 数据流

```
colm-gui  (MSVC/系统 webview，不链接 NetCDF、不链接 HDF5、不碰 Fortran)
   │  子进程 + stdout 逐行事件
   ▼
colm-cli  (MSVC，纯 Rust，netcdf crate + static feature)
   │  子进程 + namelist 文件（换工具链，不同进程 ⇒ 无 ABI 冲突）
   ▼
mksrfdata.x → mkinidata.x → colm.x   (MinGW / 系统 gfortran)
```

**GUI 进程既不链接 NetCDF 也不链接 HDF5 也不碰 Fortran** —— 这一条决定化解了
Windows 上 MSVC/MinGW 的 ABI 冲突（`netcdf` crate 的 static feature 上游 CI 只在
windows-msvc 上通过，windows-gnu 那行是注释掉的坏条目；而 Windows gfortran 产出
MinGW ABI）。

---

## 5. I/O 契约

Rust 侧必须遵守 §2.10 的全部实测细节。本节只补充设计层面的决定。

### 5.1 输入

- **用户可见的导入形式只有一种**：PLUMBER2/FLUXNET NetCDF（`Forcing/` + `Sitedata/`
  + `Observation/`）。CoLM 原生 srfdata/forcing/namelist 作为高级用户的直通路径保留，
  因为 Fortran 内核本来就读它。
- 站点浏览基于 `SiteList` 的 90 站清单（code、起止年、经纬度、forcing 目录、sitedata 文件名）。
- `DEF_dir_forcing` 等 HPC 绝对路径必须由 `colm-cli` 重写为本地路径。
  实测 `Forcingnml/CN-Cng.nml` 里是 `/tera12/yuanhua/data/CoLMpointdata/PLUMBER2s/Forcing/`。

### 5.2 缺失地表参数的合成

这是 GUI 里一个显式环节，不是隐藏的兜底：

1. 检测站点文件缺哪些 CoLM 无条件读取的字段。
2. 对每个缺失字段提供**三个选项**：从站点参数包取（若有）、用默认值（并显示该默认值
   的出处，如「`MOD_SingleSrfdata.F90:47` 模块默认值」）、手工填。
3. 土壤亮度以「土壤颜色等级 1–20」下拉呈现，而不是 4 个裸反射率数字。
4. 合成的字段必须在 NetCDF 属性里标记 `source = "synthesized ..."`，并进入
   `run_manifest.json`。**永远不能让用户以为合成值是观测值。**

### 5.3 输出

- 不改 history 文件格式。
- `colm-hist` 负责时间轴还原（§2.10 的半区间回移）与抽稀。
- 评估必须按 `Observation` 文件的 `_qc` 标记筛选，**只用实测点**，并显示用了多少点。
- 必须剔除 spin-up 时段（§2.9），且显示剔除了多少。

**哪些变量会被写出来，由三道闸门依次决定，而不是由 `DEF_hist_vars` 一张表决定。**
`history_var_type` 有 482 个开关、343 个默认为真，而 waterheat 预设的一次真实运行
只写出 119 个。差额全部落在前两道闸门上：

| 闸门 | 判据在哪 | 谁回答 | waterheat 下 |
|---|---|---|---|
| 1. 编译期宏 | `MOD_Hist.F90` 的 `#ifdef` / `#ifndef` | `colm-hist`，输入是内核清单的 `macros`（§6.1） | 456 个写出点 → **123** |
| 2. 运行时 `DEF_*` 条件 | 同一文件的内联 `.and.` 与外层 `IF (DEF_*) THEN` | 记下条件原文，由调用方结合算例配置求值 | 10 个带条件，本次 6 真 4 假 → **119** |
| 3. 变量开关 `DEF_hist_vars%X` | `MOD_Namelist.F90` | `colm-schema`（§4.2 的那张字段表） | 默认全开 |

闸门 1 的表是生成的（`xtask gen-histmap`），产物入库，drift 测试守住它不与上游脱节；
并由 `oracle/tests/histmap.rs` 拿入库的黄金文件做经验校验 —— **零漏报**是硬要求，
多报恰好是闸门 2 挡下的 `dz_lake` / `qcharge` / `t2m_wmo` / `xy_hpbl` 四个。
多报的方向是安全的（说「可能产出 X」而实际没有），漏报则是拿一张静态表去否定
一次真实运行。

闸门 2 刻意只记原文、不求值：求值需要一份具体的算例配置，那是命令层的事。
两张表也刻意不在生成期耦合，在 GUI 层合并即可。

这道闸门与 §6.4 的静默覆盖是同一件事的两面：`qlayer` 与 `qcharge` 挂在
`DEF_USE_VariablySaturatedFlow` 的两侧，而 CoLM 打印的第一条覆盖消息正是
`DEF_USE_VariablySaturatedFlow is automaticlly set to .true.` —— 于是有了 `qlayer`、
没了 `qcharge`。用户该看到的是这两句连起来的一句话。

---

## 6. 运行时契约与失败处理

> **实现状态**：§6.1–§6.4 已实现于 `crates/colm-kernel`（里程碑 5），
> 分别在 `manifest.rs` / `run.rs` / `outcome.rs` / `overrides.rs`。
> §6.5 是构建期开关，已默认打开。§6.6 进程生命周期属于 GUI 层，尚未实现。
> 下文与实现不符的地方已就地改正，改正处标注了理由。

### 6.1 内核清单代替 `--version`

`colm.x` / `mkinidata.x` / `mksrfdata.x` 均以 `getarg(1, nlfile)` 取 namelist 路径
（`main/CoLM.F90:185`、`mkinidata/CoLMINI.F90:86`、`mksrfdata/MKSRFDATA.F90:124`），
**不接受其他参数，没有 `--version`**。

因此改用构建期生成的清单。**它是每个预设目录一份，而不是一份全局文件**：
本节原先写的是 `kernels/manifest.json` 里一个 `kernels` 数组，实现时改成了
`kernels/<preset>/manifest.json`，理由见下一段的「同生同存」—— 清单认定的是
紧挨着它的那三个二进制，一份全局清单会让它在预设之间失去这个含义。
实测的 `kernels/waterheat/manifest.json`：

```json
{
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
  "sha256": { "mksrfdata": "…", "mkinidata": "…", "colm": "…" }
}
```

`schema` 不匹配即拒绝读，而不是按旧字段含义解释一份新格式的清单。

**为什么必须记 `netcdf_c` / `netcdf_fortran` / `hdf5`**：黄金文件的字节由
**Fortran 侧**写出（`colm.x` 链接系统 netcdf-fortran），Rust 判官只负责读。
`Cargo.lock` 对 Fortran 侧库版本毫无约束，所以这三个版本号是黄金文件可复现性的
唯一记录点。本机实测值即上所示（均来自 miniforge）。Rust 侧静态链入 HDF5 2.x，
读 1.14.6 写出的文件实测正常。

**Fortran 构建不是逐字节可复现的**（实测：同一路径连跑两次，三个二进制 sha256 全不同）。
故 manifest 里两组字段职责不同：`macros`/`colm_git_sha`/`generator_args` 可复现，
认定**配置身份**；`sha256` 每次构建都变，只认定**完整性**（二进制自其 manifest
写出以来未被替换）。后者正是这里需要的性质，但它要求 manifest 与二进制同生同存，
不能分开分发，也不能拿一份入库的 manifest 去校验重新构建的二进制。

启动时校验 sha256。不匹配则明确报「哪个预设、哪个文件、期望 vs 实际」——
**「不存在」和「存在但版本不对」是两种不同情况**，不能混成一句「内核不可用」。
GUI 只渲染校验通过的预设（backend-owned UI vocabulary）。

这比 `--version` 更强：它把**编译期宏集合**也记录下来了，而那是单点模式最容易搞错的东西。

### 6.2 三段编排与产物硬校验

| 段 | 必须产出（实测路径） |
|---|---|
| `mksrfdata.x` | `<output>/<case>/landdata/srfdata.nc`，且能被 Rust 侧 netcdf 打开、含 `latitude`/`longitude`、`patch=1`、`soil=8` |
| `mkinidata.x` | `<output>/<case>/restart/const/<case>_restart_const_lc<YYYY>_w180_s90.nc` 及不带后缀的同名文件 |
| `colm.x` | `<output>/<case>/history/<case>_hist_<cdate>.nc` |

### 6.3 成败判定三件套

因为 §2.4 证明退出码不可信，三条缺一不可：

1. **正向成功标记** —— 每段必须出现的字符串：
   `Successful in surface data making.` /
   `CoLM Initialization Execution Completed` /
   `CoLM Execution Completed.`
2. **输出产物硬校验** —— §6.2 的表。
3. **错误标记扫描** —— 实现里的十条，顺序即报告优先级（同一行命中多个时报第一个，
   所以具体的排在笼统的前面）：

   | 标记 | 来源 | CoLM 自己是否中止 |
   |---|---|---|
   | `Cannot match namelist object name` | namelist 里有未声明的变量 | 是 |
   | `Memory allocation (malloc) failure` | 非法时间窗口等 | 是 |
   | `Fortran runtime error` | gfortran 运行期，**只走 stderr** | 是 |
   | `Error termination` | 同上 | 是 |
   | `balance violation` | `CoLMMAIN.F90:1545/1620`，10 种文本 | **否，只警告**（见 §6.5） |
   | ` with NAN` / ` Out of Range!` | `MOD_RangeCheck.F90:139/144` | 仅当定义了 `CoLMDEBUG` |
   | `Netcdf error` / `***** ERROR` / `ERROR in` | 笼统兜底 | 视来源而定 |
   | `does not exist` | `MOD_NetCDFSerial.F90:163` 缺输入文件 | 是，但走**无参数**的 `CoLM_stop()`，一句话都不再打印 |

   **注意排除已知的无害行**：`History namelist file: null does not exist.`
   —— 没配 `DEF_HIST_vars_namelist` 时三段日志各出现一次。豁免是**整行精确匹配**，
   所以指向一个不存在的真实路径不被豁免：那种情况 CoLM 会静默回落到默认变量集。

   本节原先写的是「stdout 扫描」，实现时改成 **stdout 与 stderr 都收**：
   gfortran 的运行期错误只走 stderr，所以 `Fortran runtime error` 与
   `Error termination` 这两个标记在只读 stdout 时**永远不可能命中**。
   实测 namelist 文件缺失时 stdout 是 0 字节而 stderr 有 302 字节 ——
   只收 stdout 的话，日志会空得看不出任何原因。

任何一条不满足即判失败，并报出**是哪一条触发的**加最后 N 行日志。绝不静默成功。

### 6.4 静默覆盖必须回报

解析日志里 `Note:` / `Warning:` 开头的覆盖消息（§2.6），在 GUI 中以
「你要求了 X，模型实际用了 Y」呈现。这是 EarthMesh 的
「报告实际产出了什么，而不是你要求了什么」在配置层的应用。

实现时实测出三件本节原先没写的事：

1. **前缀不统一**。一次 CN-Cng 运行出现 9 种消息，其中最后一条是
   `Warning :` —— 冒号前有个空格。按 `"Warning:"` 匹配会漏掉一整类消息，
   而且毫无迹象。所以匹配的是「关键词 + 可选空白 + 冒号」。
2. **只认前缀，不认文本**。CoLM 把 automatically 拼成了 `automaticlly`。
   按消息文本匹配的代码会在上游改错字的那天静默失效，所以整行原样交给上层，
   语义解析留给需要它的调用方。
3. **抽覆盖与判成败必须零碰撞**。这 9 条消息里没有一条命中 §6.3 的 7 个
   失败标记 —— 这不是自明的，两边都会各自增长，所以有一条测试守着它。

三段各自都会打印这些消息（前两段各 8 条，`colm` 段 9 条），所以呈现时按段归属，
不跨段去重。

### 6.5 默认武装 `CoLMDEBUG`

它武装 `CoLMMAIN.F90:1545` 的 `|errore| > 0.5` W/m² 与 `:1620` 的 `|errorw| > 1.e-3` mm。
GUI 场景下宁可炸也不要给出错的数。

**本节原先写「它同样走 `CoLM_stop` → 退出码 0，仍靠 §6.3 捕获」，那是错的。**
实测源码：这两处是 `write(6,*) 'Warning: ... balance violation ...'`，
**打印之后继续跑**，没有 `CoLM_stop`。所以一次能量不守恒的运行会跑到底、
写出完整产物、打出成功标记 —— CoLM 自己不执行「宁可炸」这条政策，
执行它的必须是 §6.3。十种消息文本共享 `balance violation` 一个子串。

同一个宏还武装 `MOD_RangeCheck.F90`，但那边的行为不同：它在
`len_trim(exception) > 0` 时确实调 `CoLM_stop(' ***** ERROR: ...')`。
两者的差别意味着**不能拿「CoLMDEBUG 开着」当作一条统一的保险**，
必须逐个检查各自的行为。

### 6.6 进程生命周期

- 单算例：全局单一 run lease（`Mutex<Option<RunState>>`），`Drop` 时保留活着的子进程
  PID，使取消在 future 被放弃后仍然有效。
- 批量：换成 N 槽信号量。
- 取消：按 PID kill（`kill -KILL` / `taskkill /PID /F /T`）；kill 失败保留 PID 以便重试。
- 日志：stdout/stderr 各一个 `std::thread` 逐行抽取（避免管道死锁），发一个自定义事件。
  进度从 `colm` 打印的日期行解析。
- 内核二进制先暂存到临时副本再运行（EarthMesh 的静态 netcdf 二进制在源码树中运行时
  被 SIGKILL），带体积与 mtime 陈旧性检查。

---

## 7. GUI 结构

**A 为主，吸收 B 的向导和 C 的卡片流**：

- **三栏工作台**是常驻界面 —— 左：算例库；中：配置（站点/时间/物理/输出变量 分页）+ 日志；
  右：结果（曲线/指标/对比）。改参数按 ⌘R 重跑，图当场变。
  - **「输出变量」页渲染的是 `colm_hist::writable(manifest.macros)`，不是 482 个
    `DEF_hist_vars%*` 开关。** 当前预设下铺出来的是 123 个而不是 482 个（§5.3）。
    把这个内核编译期就没编进去的那些也画成复选框，等于让用户勾一个永远不会出现的
    变量，然后去日志里找原因。带运行时条件的那 10 个另行标注，条件原文照抄给用户看。
- **步骤向导**只在「新建算例」时出现一次 —— 服务「降低门槛」与课堂首次演示。
  每步有解释，宏互斥（§2.2）这类坑当场讲清。
- **算例卡片流**是左栏中「批量」与「敏感性」算例点开后的详情视图 —— 服务横向对比。

技术栈：Tauri v2（2.11.x）+ 无 npm 的静态前端 + **vendored uPlot**
（166,650 点交互 25 ms、~50 KB、MIT、Canvas 2D，无 WebGL/WASM）。
`tauri.conf.json` 的 `script-src: 'self'` 禁止 CDN，绘图库必须随包并附 LICENSE。

配置状态管理照抄 EarthMesh：**无状态配置往返** —— 每个修改命令接收整份配置文档加一个
改动字段，返回重新校验后的规范化文本。Rust 拥有 schema，前端从不自行构造配置。
`preserve_unexposed_project_fields`：打开旧配置再保存，不得静默丢掉当前 UI 无法渲染的字段。

### 7.1 照抄 EarthMesh 的具体形态（实测其 `gui-tauri/`）

**egui 已经被试过并被换掉了。** `gui-tauri/README.md` 开篇即写
「This replaces the egui GUI, whose immediate-mode styling could not match the
static redesign」—— 立即模式 GUI 做不出想要的排版。这是选 Tauri 的实证依据，
不是偏好。

实测到的配置，逐条照搬：

| | EarthMesh 的做法 | 对 colm-desktop 的对应 |
|---|---|---|
| 依赖 | `tauri = "2"`（解析到 2.11.3）、`tauri-build = "2"`、`tauri-plugin-dialog = "2"`（2.7.1）、wry 0.55.1 | 同 |
| workspace | `src-tauri/Cargo.toml` 里一个**空的 `[workspace]`**，把 GUI 挡在引擎 workspace 之外 | 同 —— `cargo test --workspace` 不该把 webkit2gtk 拖进来 |
| 前端 | `dist/index.html` 单文件（290 KB）+ `dist/vendor/`，**无 `package.json`、无 `node_modules`、无打包器** | 同，vendor 里换成 uPlot |
| IPC | `withGlobalTauri: true`，页面调 `window.__TAURI__.core.invoke(...)` | 同 |
| 入口 | `main.rs` 6 行，只调 `lib::run()`；`crate-type = ["staticlib","cdylib","rlib"]` | 同 |
| 后端组织 | `lib.rs` 88 行只做模块枢纽与 `generate_handler!`，命令按职责分 13 个模块（最大 784 行），测试在 `lib_tests.rs`（3214 行） | 同 —— 与本仓库 `#[path = "*_tests.rs"]` 的惯例一致 |
| 权限 | `capabilities/default.json`。**自定义 `#[tauri::command]` 不需要声明权限，只有插件命令需要** | 同 |
| 重活 | GUI 进程**不链接 netcdf/hdf5**，一律 shell out 给 sidecar CLI | 同 —— 我们的 sidecar 是 `kernels/*.x`，已由 `colm-kernel` 封装 |
| 打包 sidecar | `bundle.externalBin` + 一个 release-only 的 `tauri.bundle.conf.json` 覆盖层 + 暂存脚本 | 同，但暂存脚本用 Rust 写（xtask），不引入 Node |
| 运行 sidecar | 先拷成带 `$PID-$SOURCE_HASH` 的临时副本再跑 | 同 —— §6.6 那条「先暂存到临时副本」就出自这里 |

两处刻意不照搬：EarthMesh 的 `make test-gui` 用 Node 解析内联 JS 做前端不变量
检查，而本项目不引入 Node（打包与前端检查都改用 xtask）；地图那一整套
（OpenLayers / MapLibre）与单点模型无关，我们只需要 uPlot 画时间序列。

---

## 8. 测试策略

四层。第 2 层是仓库今天完全没有的东西，且是 C 阶段唯一的验收标准来源，因此**不是可选项**。

1. **Rust 单元测试** —— namelist 带注释往返、forcing 转换的单位与时间轴、
   history 时间轴还原、指标算法、缺失字段合成的确定性。
2. **黄金文件回归** —— 以 §2.8 的 CN-Cng 运行为第一份基准。
   比对工具已存在：`tests/river_hist_compare.py`（通用、producer-independent 的
   NetCDF 目录差分器）。附录 A 是完整可复现步骤。
3. **失败注入测试** —— 坏 namelist（未声明变量）、缺文件、缺 rawdata、缺 runtime 数据、
   时间窗超出强迫场覆盖、跑到一半 kill。**每一种都必须被判为失败**。
   这层专门针对 §2.4。
4. **GUI 后端行为测试** —— run lease 重叠、内核 sha256 不匹配、内核缺失 vs 版本不对、
   Finder 启动时 `cwd=/` 的路径解析、kill 重试。

**四条纪律**（全部来自 EarthMesh 踩过的坑）：

- **对账工具是永久资产**。EarthMesh 移植完成后删掉了 reduced-Fortran 探针脚本、
  840 行黄金数对比文档、1,385 行迁移计划和 manifest+gate，结果失去了自己最强论断的
  可复现性。`oracle/` 目录永不删除。
- **不许有会静默跳过的对账测试**。EarthMesh 仅存的两个数值 fixture 测试是 `#[ignore]` 的、
  依赖被 gitignore 排除的 `.nc4`、缺失时 `eprintln!("skip")` 直接 return，
  而且只抽查了 81,921 个索引里的 7 个。
- **容差写下来并统一**。EarthMesh 同一个 NXP64 网格在两个测试里用了 `2.0e-6` 和 `2.0e-4`。
  本项目必须有一份成文的逐变量、逐时间尺度容差策略。
- **不许 `_ =>` 兜底的 dispatcher**。EarthMesh 的后端派发对 `harpdv`/`harp-dv`/`redgreen`/
  `method-c`/`HARP_DV` 全部静默跑 Method-C 且一声不响。
- **补充一条本项目特有的**：`_one_based` 索引基约定。EarthMesh 两个独立移植的相邻内核
  静默漂移成了相反的索引基，两边各自的测试都通过，只有组合时才暴露。C 阶段必须
  逐字保留 Fortran 的 1-based 索引，并用函数名后缀标记。

另外：给 `.github/workflows/TestCaseLists` 加一行 SinglePoint 用例，
终结零 CI 覆盖的状况。`create_defineh.bash` 已接受 `SinglePoint` 作为第 1 参数，
是一行的改动。

---

### 8.1 容差策略（已决定：分层，不追全局逐位）

全局逐位可比是不可达成的目标：`MOD_Hydro_SoilWater` 的三个割线迭代按**迭代次数上限**
退出、`MOD_SnowSnicar` 在反照率为负时**扰动太阳天顶角 0.02** 重试至多 20 次 ——
基准答案本身依赖浮点细节。但这不意味着放弃严格性：**大部分代码可以逐位，只有迭代模块不能。**

| 层级 | 适用范围 | 判据 |
|---|---|---|
| **Tier 0：逐位** | 纯函数、无迭代、无收敛：物理常数、时间管理、`qsadv` 饱和水汽压、`MOD_SoilColorRefl`、USDA 质地分类、单位换算、NetCDF 编解码、namelist 往返 | 完全相等。**任何差异都是 bug** |
| **Tier 1：`rtol = 1e-12`** | 确定性代数、无迭代的物理：辐射传输的解析部分、湍流通量的显式部分、相变的代数部分 | 相对误差 ≤ 1e-12。超出即视为实现差异而非舍入 |
| **Tier 2：以求解器自身收敛容差为下限** | 含迭代收敛的模块。**容差不得紧于求解器自己的容差** —— Richards 的 tol 是 `8.e-8`，故取 `1e-7`；叶温是 40 次准 Newton 且带 Obukhov 长度硬重置；`MOD_SnowSnicar` 带天顶角扰动重试 | 逐变量绝对+相对容差，且必须同时报告**迭代计数与回退次数**（如 `VSF scheme all steps: 528 implicit / 6 explicit`）。回退次数变化即为红旗，即使数值在容差内 |
| **Tier 3：统计等价** | 整场 history，逐变量、逐时间尺度 | 判据挂在 §2.8/§2.8b 的实测基线上：湿季 Rnet R² ≥ 0.999、Qle R² ≥ 0.85；且**不允许引入趋势偏移**（逐日均值的线性趋势差不显著） |

**分层的关键好处**：约 40% 的基础设施代码和大部分无迭代物理落在 Tier 0/1，可以逐位约束；
只有少数迭代模块需要软判据，而那些模块本来就是最需要小心对待的（§2.11）。

容差表必须成文并入库（`oracle/tolerances.toml`），**禁止在测试里内联魔数** ——
EarthMesh 就是在两个测试里对同一个网格用了 `2.0e-6` 和 `2.0e-4`。

## 9. 跨平台构建与分发

**打包**：Tauri bundler —— 唯一同时覆盖 MSI+NSIS / DMG+.app / deb+rpm+AppImage
且能用 `APPLE_*` 环境变量自动完成 macOS 公证的方案。

**Linux**：优先 `.deb` + `.rpm`。AppImage 是 WebKitGTK 唯一主动破坏的 Linux 目标
（helper 进程路径硬编码，`WebKitNetworkProcess` 不继承 AppImage 库路径），
按 best-effort 处理。Flatpak 可绕开 webkit2gtk-4.0/4.1（libsoup2/3）分裂，
因为 GNOME runtime 自带 4.1。

**Windows**：GUI 与 `colm-cli` 用 MSVC 目标；Fortran 内核用 MSYS2 MinGW64 单独构建
（`mingw-w64-x86_64-netcdf-fortran` + OpenBLAS），静态链接 gcc 运行时。
`FF=gfortran` 已验证可去掉 MPI（§2.3）。

**签名成本（需决策）**：Apple Developer Program **$99/年且公证强制**（免费账户无法公证）。
Windows OV $200–300/年、EV $300–900/年，均需 FIPS 硬件令牌；按 CA/B 规则
2026-02-23 起证书最长 459 天，多年购买每年换一个令牌。
**廉价方案不可用**：Azure Trusted Signing $9.99/月，但限定美国/加拿大法人且需 3 年
可核验纳税史，「无例外、无人工覆盖」—— 中山大学的团队被明确排除在外。
**已决定**：

- **买 Apple $99/年。** macOS 上不公证会让用户必须右键绕过 Gatekeeper，
  直接毁掉「降低使用门槛」这个第一目标 —— 这笔钱是必要成本，不是可选项。
- **Windows v1 不买证书。** $200–900/年 + FIPS 硬件令牌 + 每年换令牌，而唯一廉价路
  （Azure Trusted Signing）对中国机构明确不可用。接受 SmartScreen 警告，
  在 README 与首次运行页显式说明「为什么会有这个警告、如何继续」。
- **Linux 不需要签名。**
- **打包流程从第一天就把签名做成环境变量驱动的可选步骤**，这样以后买了 Windows 证书
  不需要改任何代码或 CI 结构。

---

## 10. 里程碑

| # | 内容 | 完成判据 |
|---|---|---|
| 0 | 建仓、两个 workspace、CI 骨架、`vendor/CoLM202X` submodule | `cargo test` 通过；CI 跑起来 |
| 1 | **固定黄金输出** —— 把 §2.8 的 CN-Cng 运行脚本化进 `oracle/`，纳入 CI。**必须有两个窗口**：冬季（现有）+ 一个含降水事件的湿季窗口，否则产流与入渗代码零覆盖（§2.8 约束 1） | 两个窗口重跑均逐位一致；`river_hist_compare.py` 报 0 差异；湿季窗口的 `f_rnof` 非零 |
| 2 | `colm-namelist` + `colm-schema`（含 `MOD_Namelist.F90` 代码生成与漂移校验） | 55 个 `.nml` 全部带注释往返一致 |
| 3 | `colm-srfdata`：缺失字段检测与合成、站点参数包抽取 | 能从裸 PLUMBER2 站点文件产出可运行的增广站点文件 |
| 4 | `colm-forcing`：PLUMBER2 → POINT | 产出的强迫场使 CN-Cng 结果与里程碑 1 逐位一致 |
| 5 | `colm-kernel`：清单握手 + 三段编排 + 成败三件套 | 失败注入测试全绿；配置错误必被判失败 |
| **5b** | **让两张生成表说真话**（计划文件是 `plan-m6.md`，见下方注） | 单点算例的字段 100% 被 schema 认得；闸门表对黄金文件零漏报 |
| 6 | `colm-hist`：时间轴、抽稀、指标、QC 筛选 | 复现 §2.8 的 Rnet R²=0.986 |
| 7 | `colm-cli` 打通，命令行端到端可用 | 一条命令从 PLUMBER2 文件到指标表 |
| 8 | GUI：三栏工作台 + 新建向导，单站点闭环 | macOS + Linux 上双击可跑并出图 |
| 9 | **Windows 原生构建打通** | MSYS2 内核 + MSVC GUI，Windows 上跑通 CN-Cng |
| 10 | 三个物理预设全部打包（BGC 预设需解决 `nitrif` runtime 数据依赖，见 §2.7） | 三个预设在三个平台上各自跑通 |
| 11 | 批量 / 敏感性 / 算例管理 | 12 站批量 + 一个参数扫出汇总表 |
| 12 | 打包分发（签名策略按 §9 决策） | 三平台安装包产出 |

里程碑 1 排在最前，因为它是 C 阶段唯一的验收标准来源，且现在**已经有材料可做**。

**5b 是本表写成之后才发现需要的，编号也因此与计划文件脱了节。** 为 GUI 做测量时
发现两张表都不能用：`colm-schema` 认不得单点算例 43 个字段里的 13 个（`SITE_*` /
`USE_SITE_*` 整段），同时给出 6 个用户设了也没用的字段；而「这个内核能产出哪些
输出变量」在开跑前根本答不出来。GUI 照着这样的表渲染，只会把错误信息渲染得更
漂亮，所以先修表。

计划文件按写作顺序编号，`plan-m6.md` 对应的是本表的 5b，不是里程碑 6。
2–5 的巧合到此为止，往后以本表为准。

**`colm-hist` 这个 crate 现在只做了一半**：5b 建的是闸门表（宏集合 → 可写变量集），
里程碑 6 要加的时间轴、抽稀、指标与 QC 还没有。§4.2 的 crate 表已标出这个分界。

---

## 11. 未决问题

1. **`LATERAL_FLOW` → `CatchLateralFlow` 是否修**（§2.15）。修正会让 91 个 CI 用例
   第一次真的编译侧向流路径，可能使 CI 失败。
2. ~~容差策略~~ —— **已决定，见 §8.1**（分层：Tier 0 逐位 / Tier 1 rtol 1e-12 /
   Tier 2 以求解器收敛容差为下限 / Tier 3 统计等价）。
3. ~~签名主体与预算~~ —— **已决定，见 §9**（买 Apple $99/年；Windows v1 未签名；
   签名做成可插拔）。
3b. **`f_rsur_ie`（超渗产流）与 `f_rsub`（地下产流）两条分支仍零覆盖**（§2.8b）。
   两个黄金窗口都没触发它们。需要第三个窗口（更强降水事件）或人工构造的深湿润算例。
   在这两条分支被覆盖之前，C 阶段不得声称产流模块已验证。
4. **`USE_SITE_HistWriteBack=.true.`（默认）是否产出与 `.false.` 逐字节不同的文件**。
   一条路写固定长度 time 维，另一条写 unlimited 维并原地追加。
   若不同，移植方需知道参照输出是在哪种模式下生成的。
5. **`soil_wf_om` 的正确推导**。本文档用 `vf_om × OM_density / BD_all` 得 ~0.0005，
   但若 `OM_density` 语义已是「每单位土体的有机质质量」，则应为 `OM_density / BD_all`
   ≈ 0.0154。未在代码中确证。影响合成值的物理保真度，不影响管线正确性。
6. **`f_xy_us` 与 `f_xy_vs` 实测完全相同**（标量风速被均分到两个分量）。
   Rust 强迫场转换器必须复现这个约定，具体公式未确证。
7. **BGC 与 URBAN 预设的移植面未测量**。§2.13 的 LOC 数字前提是 BGC/CROP/TRACER 全关。

---

## 附录 A：已验证的复现步骤（CN-Cng）

```bash
# 1. 构建 SinglePoint 水热预设（在独立 worktree 中，不动主工作树）
git worktree add --detach /tmp/wt HEAD && cd /tmp/wt
ln -sf Makeoptions.Mac-arm include/Makeoptions
./.github/workflows/create_defineh.bash \
    SinglePoint LULC_IGBP URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF
make FF="gfortran -fopenmp" mksrfdata.x mkinidata.x colm.x

# 2. 增广站点文件：注入 12 个 CoLM 无条件读取但 PLUMBER2 不提供的字段
#    （取值与出处见 §2.7）
#    lakedepth=1.0  elevation=138.0(取自 Observation)  elvstd=0.0  sloperatio=0.0
#    soil_s_v_alb=0.14 soil_d_v_alb=0.25 soil_s_n_alb=0.28 soil_d_n_alb=0.39
#    soil_vf_clay / soil_wf_clay = 非砂/砾/有机质剩余量的 25%
#    soil_wf_om = vf_om * OM_density / BD_all
#    soil_texture = USDA 质地三角(0-60cm 深度加权 砂14.3/粉64.3/黏21.4%) = 4 Silt loam
#    全部带 source 属性标记为 synthesized

# 3. 强迫场 namelist：照抄 PLUMBER2s/Forcingnml/CN-Cng.nml，把 DEF_dir_forcing
#    从 /tera12/... 改成本地路径

# 4. 算例 namelist 的关键偏离项（与 CoLM 默认值不同的部分）：
#      USE_SITE_* 全部 .true.（本机无 rawdata）
#      DEF_simulation_time%greenwich = .FALSE.   ← PLUMBER2 用地方时
#      DEF_Runoff_SCHEME = 3                     ← CoLM 默认 Simple VIC，要求 soil_texture
#      DEF_USE_OZONEDATA = .false.               ← 默认 .true. 要求 runtime 数据
#      DEF_USE_BEDROCK = .false. / DEF_USE_SoilInit = .false.

# 5. 三段依序运行
./run/mksrfdata.x case/CN-Cng.nml   # → Successful in surface data making.
./run/mkinidata.x case/CN-Cng.nml   # → CoLM Initialization Execution Completed
./run/colm.x      case/CN-Cng.nml   # → CoLM Execution Completed.  (1.8 秒 / 10 天)
```

结果：`history/CN-Cng_hist_2008-01.nc`，1.3 MB / 129 变量 / 264 时间步，
`f_xerr = f_zerr = 0`，Rnet 对观测 R²=0.986 / bias=+0.22 W/m²。

---

## 附录 B：rawdata / runtime 文件清单

单点路径引用的 `DEF_dir_rawdata` 文件（全球网格，`colm_500m` = 86400×43200；
单点只需 1 个像元，故应做站点参数包抽取而非整体搬运）：

```
<rawdata>/bedrock.nc                      <rawdata>/soil/lambda.nc
<rawdata>/elevation.nc                    <rawdata>/soil/OM_density_s.nc
<rawdata>/Forest_Height.nc                <rawdata>/soil/psi_s.nc
<rawdata>/lai_15s_8day/lai_8-day_15s_*    <rawdata>/soil/soiltexture_0cm-60cm_mean.nc
<rawdata>/lake_depth.nc                   <rawdata>/soil/theta_s.nc
<rawdata>/landtypes/landtype-igbp-modis-* <rawdata>/soil/tkdry.nc
<rawdata>/landtypes/landtype-usgs-update.nc  <rawdata>/soil/tksatf.nc
<rawdata>/soil_brightness.nc              <rawdata>/soil/tksatu.nc
<rawdata>/topography.nc                   <rawdata>/soil/vf_clay_s.nc
<rawdata>/urban/NCAR_urban_properties.nc  <rawdata>/soil/vf_gravels_s.nc
<rawdata>/soil/BD_all_s.nc                <rawdata>/soil/vf_om_s.nc
<rawdata>/soil/csol.nc                    <rawdata>/soil/vf_quartz_mineral_s.nc
<rawdata>/soil/k_s.nc                     <rawdata>/soil/vf_sand_s.nc
<rawdata>/soil/k_solids.nc                <rawdata>/soil/VGM_{alpha,L,n,theta_r}.nc
                                          <rawdata>/soil/wf_{clay,gravels,om,sand}_s.nc
```

`DEF_dir_runtime` 文件（仅在对应开关打开时）：

```
<runtime>/Ozone/Global/OZONE-setgrid.nc          DEF_USE_OZONEDATA (默认 .true.)
<runtime>/snicar/snicar_optics_5bnd_mam_c211006.nc  DEF_USE_SNICAR (默认 .false.)
<runtime>/snicar/snicar_drdt_bst_fit_60_c070416.nc  同上
<runtime>/vic/vic_para.txt                       VIC 产流方案
<runtime>/nitrif/CONC_O2_UNSAT/CONC_O2_UNSAT_l01.nc  BGC + DEF_USE_NITRIF
<runtime>/HydroLAKES_Reservoir.nc                水库
```

分层土壤文件的变量名带层号后缀，如 `vf_clay_s_l01` … `vf_clay_s_l08`。
