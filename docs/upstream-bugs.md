# 上游 CoLM 的缺陷与不一致（待报上游）

对象：`https://github.com/zhongwangwei/CoLM-SYSU-integration`（本地
`/Users/zhongwangwei/Desktop/Github/CoLM-SYSU-integration`）。行号按 **`3c799bae`**。
每条写清楚：在哪、为什么是错的、影响范围、本仓库 `vendor/CoLM202X` 怎么处理。
新发现的追加在末尾，已被上游修掉的挪到「已修」一节并注明上游提交。

证据与数值影响的细节在 `docs/implementation-verification.md` 对应轮次；同步过程在
`vendor/PROVENANCE.md`。

## 一、确定的缺陷

### 1. `eroot` / `SoilSurfaceResistance` 接到整列 `(lb:nl_soil)`，有雪时整列错位

- **位置**：`main/MOD_Thermal.F90:699`（`SoilSurfaceResistance`）、`:751`（LCT `eroot`）、
  `:905`（PFT `eroot`）；`main/URBAN/MOD_Urban_Thermal.F90:858`（`eroot`）。
- **原因**：实参 `dz_soisno`/`t_soisno`/`wliq_soisno`/`wice_soisno` 声明为 `(lb:nl_soil)`，
  而 `MOD_Eroot.F90:69-71`、`MOD_SoilSurfaceResistance.F90:27-30` 的哑元是 `(1:nl_soil)`。
  按序列关联，有雪层（`lb<1`）时 `dummy(i) = actual(lb+i-1)`：雪层被当成土壤第 1 层，
  配的却是土层的 `porsl/psi0/rootfr`；最下 `|snl|` 层土壤不参与根系吸水。
  作者本意有旁证：同一个 `eroot` 在 `main/BGC/MOD_BGC_Veg_CNFireLi2016.F90:109` 用的是
  `t_soisno(1:,i)` 切片。
- **影响**：凡有雪层的时段。PHS 关闭时直接改变分层吸水（AT-Neu 二月潜热差到 10 W/m²、
  地下水位 0.21 m）；PHS 打开时改变 `rootr` 诊断（最大差 0.48）。
- **处理**：全部改为传 `(1:)` 段。

### 2. `create_defineh.bash` 定义的是源码不读的 `LATERAL_FLOW`

- **位置**：`.github/workflows/create_defineh.bash:185-188`。
- **原因**：源码里读的是 `CatchLateralFlow`（`CoLMMAIN.F90`、`MOD_Vars_TimeVariables.F90` 等 20 多处），
  `LATERAL_FLOW` 在全部 `.F90` 里零引用。用这个脚本编出的 CATCHMENT 内核**静默关闭了侧向流**。
- **处理**：脚本改为定义 `CatchLateralFlow`。

### 3. 示例 namelist 含未声明的键

- **位置**：`run/examples/SiteSYSUAtmos_IGBP_VG.nml:19-20`（`USE_SITE_topostd`、`USE_SITE_BVIC`）。
- **原因**：`share/MOD_Namelist.F90` 没有这两个键；namelist 读取带 `iostat` 检查，读到未知键即 `CoLM_Stop`。
- **处理**：删去这两行。

### 4. `MOD_Thermal` 的 `pn` 在非 TRACER 构建里未定义

- **位置**：`main/MOD_Thermal.F90:1082-1086`。
- **原因**：`pn = ps - 1` 只在 `#ifdef TRACER` 分支里；`#else` 只剩一行注释。PFT 切片为空
  （`ps > pe`）时循环不执行，其后 `IF (DEF_USE_PC .and. pn.ge.ps)` 读到未定义的 `pn`。
- **影响**：PC 次网格、PFT 切片为空的 patch。
- **处理**：对所有构建执行 `pn = ps - 1`。

### 5. `o3coef*` 首次调用未初始化——只修了臭氧关闭的情形

- **位置**：`main/MOD_LeafTemperature.F90:468-479`（`FIX 2026-08-16 #4b`）、
  `main/MOD_LeafTemperaturePC.F90:570-580`（`#4`）。
- **原因**：`o3coef{v,g}_{sun,sha}` 是 `intent(inout)`，由调用方的 SAVE 变量传入，迭代循环
  （`:625`）内就交给了 `stomata`。上游的修复只在 `.not. DEF_USE_OZONESTRESS` 时于迭代前置 1；
  **臭氧胁迫打开时**，一次运行的第一次调用读到的仍是未定义值（实测关闭时第 1 步 `etr` 小 4800 倍，
  打开时同源）。
- **处理**：迭代前无条件置 1（臭氧打开时迭代后的 `CalcOzoneStress` 照常覆盖）。

## 二、TRACER 编译开关改变了物理（需要上游确认哪一边是对的）

这一版上游在很多地方给 TRACER 构建和非 TRACER 构建写了**不同的物理**，不只是记账不同。
结果是同一个算例，编译时开不开 TRACER（哪怕一个示踪物都不注册）会得到不同的水文/冠层结果。
本仓库把 TRACER 改成运行时开关 `DEF_USE_TRACER` 时，一律**保留两边各自的行为**（开关关 =
上游非 TRACER 构建），没有擅自统一。

| # | 位置（3c799bae） | TRACER 构建 | 非 TRACER 构建 |
|---|---|---|---|
| 6 | `MOD_Thermal.F90:1033`、`MOD_LeafTemperature.F90` 的 `ipft_index` | 截获方案 8 用逐 PFT 的 `ncd_p/ncw_p/bcw_p` 算冠层储水能力 | 退回 patch 级 `ncd/ncw/bcw` |
| 7 | `MOD_LeafTemperaturePC.F90` 的 `dewfraction` 调用与露水更新 | 所有方案都用 colm2014 写法；方案 8 传逐 PFT 储水能力；VEG_SNOW 关闭时按比例分相 | 方案 2–7 各自的写法 |
| 8 | `HYDRO/MOD_Hydro_SoilWater.F90:375-465` 含水层交换 | PHS 根系净回水（`deficit<0`）与基流分**两次**交换 | 合成**一次**交换（交换是非线性的，结果不同） |
| 9 | `HYDRO/MOD_Hydro_SoilWater.F90:989` Richards 收敛 | 收敛后做液态水质量投影 `project_richards_liquid_water` | 不投影 |
| 10 | `MOD_SoilSnowHydrology.F90:1255-1292` 不透水顶层 `qgtop<0` | 按冰/液分配扣减 | **改回**只扣液态水（分叉点 `CoLM202X@2f91b435` 对所有构建都是冰/液分配） |
| 11 | `MOD_SoilSnowHydrology.F90:~505-535`、`~1360-1385` 露水/霜 | 受第 1 层孔隙容量限制，超出部分与被新霜挤出的液态水进地表积水 | 直接加进第 1 层 |
| 12 | `CoLMMAIN.F90:1506` | 调用 `relocate_soil_frost_ice`（土壤表层霜冰重分配） | 不调用 |
| 13 | `CoLM.F90:414/419` 氮沉降初始化年份 | `s_year` | `sdate(1)`——1 月 1 日 0 时起算时被 `adj2end` 推到**前一年**，读错一年的文件 |
| 14 | `MOD_NewSnow.F90:81` 湿地暖地面降雪 | 另要求 `snl==0` | 无此条件 |
| 15 | `MOD_LeafInterception.F90`（CoLM2014 入口） | 修复 `ldew` 与雨/雪分量的不一致 | 不修复 |

第 13 条里非 TRACER 那一边看起来本身就是 bug（读的是前一年的氮沉降）；第 10 条是上游把
非 TRACER 构建**退回**到了分叉点之前的写法，是否有意需要确认。

## 三、已修（上游已修掉，留作记录）

- `MOD_LeafTemperaturePC` 的 `o3coef*` 迭代后才赋值：上游 `FIX 2026-08-16 #4` 已在迭代前置 1
  （臭氧关闭时，见第 5 条的遗留）。
- 扩展截获 `MOD_LeafTemperature_Extended.F90` 的 `rstfacsun/sha` 为 `intent(out)`：上游 `d6de53e9`
  起不再编译扩展截获，问题随之消失。
- `create_defineh.bash` 的 `#error "TRACER requires GridRiverLakeFlow"`：上游已去掉。
