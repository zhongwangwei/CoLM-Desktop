# vendor/CoLM202X 的来源与本地改动

## 来源

- 2026-08-20（提交 `acfb5965`）从 submodule 改成入库副本。分出来的是上游 fork 的
  `2f91b435`（分支 `fix/tracer-singlepoint`），当时已含两处上游修复：PR #14（城市站从站点文件读湖深与
  LAI）、PR #15（去掉 TRACER 那条过严的 `#error`）。
- 改成入库副本是为了把编译期宏改成运行时开关（约 1100 处条件编译）；在 submodule 里做，每次同步上游都要
  rebase 上千处改动。
- 入库 591 个文件、磁盘上 709 个：`vendor/CoLM202X/.gitignore` 的 `/tests` 是上游自己的规则，那 118 个
  文件在 submodule 时期同样未跟踪。
- 2026-10-30 追平上游 `7d2fca93`（此前后者在 `1d7f3b25`）：`1d7f3b25..7d2fca93` 之间 256 个提交、
  280 个文件，主体是 MaterialTransport 重构、colm30/rawdata 前处理重构与汇流/CaMa 修复。
  下游的 `main/TRACER/` 目录被 `main/MATERIAL/` 整体取代。
- 2026-10-30（同日第二轮）再追平上游 `e766527a`：`ddb7bc1a..e766527a` **22 个提交**、
  上游动过 **70 个文件**——其中 64 个有内容改动、6 个只是上游把行尾从 LF 复原回 CRLF（内容
  一字未变，见下）。主体是 MaterialTransport 的汇流/泥沙/重启/history 窗口修复、全局
  `tracer*`→`material*` 改名、H22 水库方案与 mksrfdata 区域裁剪修复。比对与逐条取舍见
  `docs/implementation-verification.md`「上游 e766527a 并入」一节；本文件下面只记改变了
  对外约定的部分。

## 本地改动在哪里查

- 逐条改动：`git log -- vendor/CoLM202X`。每个提交正文写了为什么改、怎么验证。
- 上游缺陷及其修法：`docs/upstream-bugs.md`。
- 实测结论与回归证据：`docs/implementation-verification.md`。

下面只记**改变了模型对外约定**的本地改动：namelist 的写法或语义变了，旧算例要跟着改。

## 改变对外约定的本地改动

### 2026-10-30：tracers 改名 MaterialTransport，namelist 按 family 拆开

并入上游 `7d2fca93`。**旧算例必须迁移**：

- `main/TRACER/` 整个目录换成 `main/MATERIAL/`（`GENERIC_WATER` / `FAMILY_ISOTOPE` /
  `FAMILY_SOLUTE` / `FAMILY_PARTICLE` / `FAMILY_GAS`），模块名 `MOD_Tracer_*` → `MOD_MT_*`。
- 编译期宏 `TRACER` 改名 `MaterialTransport`，并和其它跑时开关一样在这里被展开：所有 MATERIAL
  模块始终编译，运行期由 family 开关决定跑不跑。
- namelist：`&nl_colm` 里的 `DEF_USE_TRACER` + 一组 `DEF_TRACER_*` 整块删除，改为
  `&nl_colm_material`（新文件 `main/MATERIAL/MOD_MT_Namelist.F90`）里的
  `DEF_USE_ISOTOPE` / `DEF_ISOTOPE_SPECIES` / `DEF_ISOTOPE_PARAM_FILE` / `DEF_SOLUTE_NAMES` /
  `DEF_SOLUTE_PARAM_FILES` / `DEF_USE_METHANE` / `DEF_METHANE_PARAM_FILE` /
  `DEF_SEDIMENT_PARAM_FILE` / `DEF_MT_*`；同位素物理参数移进 `DEF_ISOTOPE_PARAM_FILE` 指向的
  `&nl_colm_isotope_parameter`（`DEF_ISO_*`），甲烷在 `&nl_colm_methane_parameter`。
  `DEF_USE_SEDIMENT` 仍由 `share/MOD_Namelist.F90` 拥有（河道代码直接读）。
- 参数文件也换了：`run/standard_isotope_parameter.nml`、`standard_methane_parameter.nml`、
  `standard_sediment_parameter.nml`、`standard_chloride_parameter.nml`、`standard_nh4_parameter.nml`、
  `standard_no3_parameter.nml` 取代旧的 `standard_O18_parameter.nml` / `standard_HDO_parameter.nml` /
  `standard_ch4_parameter.nml`（`run/` 下 8 个旧样例已在本轮迁到新写法）。
- GUI 侧带旧算例迁移：`gui/src-tauri/src/config.rs::migrate_legacy_material` 会把旧的
  `DEF_TRACER_NAMES/TYPES/PARAM_FILES` 拆成四个 family 字段，缺 `&nl_colm_material` 时自动补组。

### 2026-10-21：地类表改值 `DEF_LC_X` 改成逐地类、所有尺度生效

- 原来：39 个 `DEF_LC_X` 是标量，只在 `#ifdef SinglePoint` 里生效，只改 `SITE_landtype` 那一类
  （`MOD_Const_LC.F90` 的 `apply_lc_scalar_overrides`）。流域、区域、全球算例写了也不起作用。
- 现在：`DEF_LC_X(LC_OVERRIDE_SLOTS)`，24 格（USGS 24 类，IGBP 用前 17 格），每格对应一个地类；哨兵
  `LC_OVERRIDE_UNSET = -1.e36`（`DEF_LC_C3C4` 是 `-1`）。
  - `MOD_Namelist.F90`：声明、逐格检查（报错带 `name(lc)`）、MPI 整个数组广播。
  - `MOD_Const_LC.F90`：`apply_lc_overrides` 在 `DEF_USE_LCT` 时逐地类覆盖，不再有 `SinglePoint` 门；
    仍在根系分布之前调用。`MOD_HtopReadin` 等读的是覆盖后的表，自动跟上。
  - `DEF_LC_RESPCP` 是在 `calc_photo_params` 里由 c3/c4 重新算出来的，要知道当前 patch 的地类：
    `MOD_AssimStomataConductance` 加 `lc_current_class`（`THREADPRIVATE`，因为 `CoLMDRIVER` 对 patch
    做 OpenMP 并行），`CoLMDRIVER` 在 `m = patchclass(i)` 之后设它。
- 对旧算例的影响：不带下标的 `DEF_LC_X = v` 在 namelist 里只落到第 1 格，**改的是地类 1**。Desktop 在
  三处兜住：迁移旧算例时把它归到 `SITE_landtype`（格点算例报错）；运行前拒绝不带下标的写法；Rust 引擎
  读到不带下标的也报错。
- 气孔五张表（`gradm`、`binter`、`g1`、`g0`、`lambda`）同样按地类改：`DEF_LC_GRADM/BINTER/G1/G0/LAMBDA(lc)`，与 PFT 侧的
  `DEF_PFT_GRADM(k)` 等对称。上游原有的整算例标量 `DEF_BALL_BERRY_GRADM`、`DEF_MEDLYN_G1` 一类不变：只在对应方案下、写了时
  盖过各地类与各 PFT 的表值。注意 `gradm`/`binter` 表不只 Ball–Berry 用（三种方案共用的迭代区间、WUE 下的 C4 植物）。

### 2026-10-21：格点城市 LCZ 方案按每个 patch 自己的像元面积平均材料参数

- `mksrfdata/Aggregation_Urban.F90`：LCZ 原来不请求像元面积，沿用人口那一循环最后一个城市 patch 的面积，材料参数差最后几位、MPI 下随进程划分变。
  现在与 NCAR 方案一样按 patch 请求面积。详见 `docs/upstream-bugs.md` 第 86 条。

### 2026-10-30：并入上游 `7d2fca93..ddb7bc1a`（N2O 死锁、history 窗口、掩码初始化）

基线是先前的 `7d2fca93` 并入；这三个提交逐个三方应用，共 16 个文件，其中 12 个与上游该基线
逐行一致、可直接套用补丁：

- `MOD_MT_RiverLake.F90`：N2O 累加器只在 worker 上分配，却被用来守五处集合调用，跑到第一个
  history 窗口边界就挂死；现在条件测试移进叶子过程，`f_mt_n2o_flux_*` 与 `mt_hist_n2o_*` 始终写出。
  同时**重启后不再恢复** `mt_hist_stor/levsto/out/bifout/acctime`：重启写在窗口关闭步的末尾，
  行总是整窗，恢复会让重启后的第一个窗口把旧窗口再算一遍（上游实测 day2 报成两日均值）。
- `MOD_MT_Defs.F90`：描述符指纹加入 `n2o_ef5`（9 个实数），旧重启因此不再匹配、按冷启动处理。
- `MOD_NH4_LEACHING_MODE` → `DEF_NH4_LEACHING`（文档、参数文件与停机消息一直用后者，照文档写会让
  namelist 读取失败）；`DEF_MT_N2O_EF5_NH4/NO3` 默认从 0 改为 -1（未设置），`>= 0` 才算覆盖，
  0 才是"关闭换算"。
- `MOD_Hist.F90`：`filter_crop`/`filter_irrig` 在分配处清零。`irrigarea` 无条件写出，但灌溉关闭时
  掩码从不赋值，报出来的面积随堆布局变化（不同汇流开关差 39–42 个格点）。
- `MOD_Grid_RiverLakeFlow.F90`：`zgrad_dn` 在 -99（出口）与 -10（内陆洼地）分支置 0，此前未初始化
  的值被 `push_ups2ucat_sum` 收进 `sum_zgrad_riv`。
- `MOD_MT_Sediment.F90` / `MOD_MT_SoilInit.F90` / `MOD_MT_Lulcc.F90` / `MOD_MT_Methane_Physics.F90` /
  `MOD_MT_SoilWater.F90`：final 时释放 history 快照、显式释放 IO 秩的块缓冲、LULCC 报错补上
  "class 0 全量"的成因提示，以及两处过时注释。
- `CoLMDRIVER.F90`：删除 `USE omp_lib` 与 `#ifdef OPENMP` 的 patch 循环（MT 的模块态不是线程安全的；
  vendor 也从不定义 `OPENMP`，是死代码）。
- `CoLMMAIN.F90` / `MOD_MT_Conservation.F90`：`mt_balance_check` 与收支/签名核对里
  `#ifndef CatchLateralFlow` 的 catchment 专用分支整段删除，统一按"有汇流出流"的形式调用。
  上游用编译期 `#error` 拒绝 `MaterialTransport + CATCHMENT`；vendor 的 MT 始终编译，无法照搬
  编译期判断，改成 `MOD_MT_Namelist.F90` 的 `check_settings` 里运行时拒绝
  （`#ifdef CATCHMENT` 且 `mt_any_enabled()` → `CoLM_Stop`）。catchment 内核本身照常构建。
  Rust 入口另有 `check_catchment`（`catchment lateral flow with material transport is not ported`）。
  `CoLMMAIN` 其余 5 处 `#ifndef CatchLateralFlow` 上游也保留，照抄。
- `run/standard_nh4_parameter.nml`、`run/standard_no3_parameter.nml`：EF5 开关按新语义改。

**唯一不跟随上游的改动**：

- `include/define.h` 仍让 `CATCHMENT` 关闭 `GridRiverLakeFlow`。

**同一轮里补齐的上游改名（Rust 侧）**：上游 7d2fca93 已把 MT 的重启变量从 `trc_*` 改成
`mt_*`、history 输出从 `f_trc_*` 改成 `f_mt_*`（`a_trc_*` 累加器与 `tracer_*` 内部函数上游仍保留）。
vendor 的 Fortran 已随之更新；Rust 引擎此前还写旧名，本轮改名并加 `mt_forcing_cache_schema = 2`
与 `mt_forcing_precip_reliable`，同时落地 `DEF_MT_FORCING_MIN_PRECIP` / `MEMORY_PRECIP`
驱动的雨雪相态回填。

### 2026-10-30（第二轮）：material 命名收口、history 旁车落盘、H22 水库

并入上游 `ddb7bc1a..e766527a`。**旧算例/旧参数文件必须迁移**的项：

- 参数文件：`DEF_SOLUTE%unit_kind` 的取值 `'tracer_per_water'` → `'material_per_water'`
  （上游 `6a38f68f` 全量改名，不保留旧值；照旧写法写会让参数校验失败）。`run/` 下的样例
  已随之更新，GUI/Rust 侧同步改名。
- history 累加器旁车（`main/MOD_HistAccRestart.F90` 写的 sidecar）：字段名 `a_trc_*` →
  `a_mt_*`；读回时**按字段**回退旧名（`hacc_read_name`：新名不存在才读 `a_trc_*`），所以
  旧旁车仍可读，新旁车只有新名。旁车现在真的会被写：`save_history_acc_restart` 有了调用点，
  且窗口在重启时已完整就不写；读回丢弃“完整窗口”、只恢复“部分窗口”，material 河沙/泥沙
  history 行跟着主机同一个判定（`hist_acc_window_restored`）。
- `DEF_Reservoir_Method = 2`（H22，CaMa-Flood v4 PR124）从“保留未实现”变成真正可用，
  要求 `DEF_ReservoirPara_file` 里有 `dam_upreal [km2]`；方法 1 结果逐位不变（上游声明）。
  旧算例若写了 `DEF_Reservoir_Method = 2`，此前会在 `reservoir_init` 停机，现在会真跑。
- `Makefile` 的 material-transport 依赖边按上游 `e17ce149` 精简为 16 条。上游是按
  `MaterialTransport` 宏条件编译推导的；vendor 的 MT 始终编译、对象表也不同，因此这一条
  必须靠实建验证：`default` / `latlon` / `catchment` 三条预设串行 `make` 全部通过（见
  `docs/implementation-verification.md`）。

**同一轮里没有跟随上游的项（有意）**：

- `include/define.h` 保持本地版本：上游这一轮只改了第 13 节注释里的一个词
  （`land runoff tracers` → `materials`），而 vendor 的整段 13 是“编译期宏→运行时开关”的
  说明文档，替换掉它会把这条约定丢掉。
- 上游 `95485682` 把 6 个文件的 CRLF 行尾复原（`main/BGC/MOD_BGC_Veg_CNFireBase.F90`、
  `...CNFireLi2016.F90`、`main/MOD_OrbCosazi.F90`、`MOD_dataSpec_PDB.F90`、
  `MOD_prospect_DB.F90`、`MOD_tav_abs.F90`）。vendor 保持 LF：仓库 `.gitattributes` 把整棵树
  钉成 `eol=lf`，多条逐字节测试依赖它。内容与上游逐字节一致（忽略行尾）。
- 上游 `.gitignore`（`e0315e26` 把上游自己的 `/docs/`、`/tests` 两条本地规则去掉）不覆盖
  vendor 的 `.gitignore`：这里保留 `/tests`、`*.py`、`docs/*.docx` 等本地规则。
- 上游强制跟踪的 13 个 `.py`（`extends/CaMa/` 下 12 个 + `preprocess/Forcings/GDAS_GPCP/
  step2_Merge_Data.py`）在 vendor 里一直不存在：vendor `.gitignore` 有 `*.py`。本轮不改变这
  一约定，13 个文件也不参与任何内核构建。

## 2026-11-01：纳入 4c28107 的材料模块源码（宿主层尚未接线）

上游 fork `zhongwangwei/CoLM202X` 的 master 已到 `4c28107`（在 `e766527a` 之后 4 个提交，
主体是 `8de029c` 把材料职责从宿主例程搬进 `main/MATERIAL`）。本轮先把**新增的 12 个
源文件**按桌面转换风格（宏→运行期开关、ELSE IF 修复后的版本）入库：

- `MOD_MT_Runtime/LandWork/HostHistory/HistoryRestart/SnowHost/SnowRemap/SurfaceData/
  CanopyDiagnostics/HydroDiagnostics/ThermalDiagnostics`、`FAMILY_SOLUTE/MOD_MT_BGC_NH4`
  与 `main/MATERIAL/Makefile.material`；
- 宿主层（CoLM/CoLMDRIVER/CoLMMAIN/Hist*/Leaf*/Thermal/Ground*/PhaseChange/
  SoilSnowHydrology/Hydro_SoilWater/MKSRFDATA/RegionClip）**仍保留桌面版**：4c 的
  facade 化宿主与本地 A 组物理修补（露水/霜冰容量、雪卸载、TOPMODEL、土壤阻抗等）
  写在互斥的签名上，必须整圈重写，不能逐文件替换；因此这批新模块目前**不参与编译**
  （桌面 Makefile 未列它们的对象），只是把上游源码快照入库存档，供下一步接线。
- 迁移方案、差距清单与验收方式见 `docs/implementation-verification.md`
  "上游 4c28107 模块化盘点的结论与同步方案" 及后续段落。

## 2026-11-01：facade 模块进入构建，CoLM 编排切到 MOD_MT_Runtime

在上一节"入库但未接线"的基础上，把 4c28107 的 11 个 `MOD_MT_*` 新模块接入 Makefile
（按上游的分配：`MOD_MT_SurfaceData` 进 OBJS_SHARED，`Hydro/Thermal/CanopyDiagnostics`、
`SnowRemap`、`HostHistory`、`BGC_NH4`、`LandWork`、`HistoryRestart` 进 OBJS_BASIC，
`SnowHost`/`Runtime` 进 OBJS_MAIN；并 `include main/MATERIAL/Makefile.material` 取得上游
的依赖边）。它们与桌面宿主层的旧接口**可以同时编译/链接**（已实测 default/latlon/crop
三预设均 exit=0）。

同时把 `main/CoLM.F90` 的编排切到上游 `MOD_MT_Runtime` facade：`mt_runtime_land_init`/
`forcing_init`/`history_restore`/`lulcc_forcing_save|restore`/`land_final`/`registry_final`；
本地 FIX #50（LULCC 下预热回卷回退土地覆盖）保留，并把其中的旧 `mt_forcing_*` 调用改为
对应的 runtime facade。`CoLMDRIVER`/`CoLMMAIN` 仍保留桌面实现——逐程序步的材料调用暂
不换，等宿主圈一次性重写。

验证：三站点黄金在切 facade 后仍 `within tolerance`（127 变量，tier3=0）；
default/latlon/crop 三预设全部构建成功。

## 2026-11-01：CoLMDRIVER 切到 mt_land_after_bgc facade

在 `MOD_MT_LandPhase` 里补上游 4c 的 `mt_land_after_bgc` 入口（内部仍是桌面版的
`mt_wetland_decomp` → `mt_soil_step` → 运行期 `IF (DEF_USE_BGC) CALL mt_bgc_n_leaching`，
BGC 保持运行期开关而不是编译期守卫），`CoLMDRIVER` 的逐步材料调用改走这个 facade；
`mt_lake_step` 仍沿用桌面的 step 解析方式。三站点黄金仍 within tolerance。

## 2026-11-02：恢复被 4c 合并丢失的 HYDRO/MOD_Hydro_SoilWater 桌面补丁

`d79ddcbf`（上游 CoLM202X MaterialTransport 重构合并）把
`main/HYDRO/MOD_Hydro_SoilWater.F90` 换成了 4c 版本，但该文件里有两类桌面宿主补丁
（第 477 轮定案为"对所有算例无条件生效"）：

- PHS 根系回水与基流分两次与含水层交换，并给入渗 `qinfl` 计入 `pond_exchange`；
- Richards 方程质量收支投影（`project_richards_liquid_water`，水量平衡误差 1e-9→1e-16）。

`git show 1deb5ee2` 里该文件有 8 处 `mass_budget`、7 处 `pond_exchange`，`d79ddcbf` 之后为 0。
本轮把文件恢复为 `d79ddcbf^` 的桌面版本；接口与 4c 版本兼容，default/latlon/crop 三预设
构建通过。恢复后 Rust 与 Fortran 的站点对拍为 0/0/77（此前 1/35/76），黄金已按恢复后的
内核重新生成。细节与证据见 `docs/implementation-verification.md` 同轮小节。

## 2026-11-02（续）：恢复 MOD_SoilSnowHydrology 的不透水表层冰/液分配补丁

与 HYDRO 文件同类的 4c 合并回归：`WATER_VSF` 里"不透水表层蒸发亏缺"块被 4c 版本换成
只扣液态水，丢了桌面补丁的"冻结时先扣冰再扣液"（以及 `imperv_subl_soil` 记账）。
恢复为 `d79ddcbf^` 的桌面版本后，单步隔离逐位一致；三站点黄金重生成后 Rust
全部 within tolerance（127 变量）。细节见 `docs/implementation-verification.md`。

## 2026-11-02（续二）：三向审计补回另外 3 条 fork 补丁

用"基准导入 `1deb5ee2` → 4c 合并前桌面树 `d79ddcbf^` → 当前 HEAD"三向比对功能行，
又确认三处静默丢失并恢复：`MOD_Runoff` 的 TOPMODEL Newton 起点兜底、
`MOD_SpatialMapping` 的区域边缘行/列兜底、`MOD_Vars_TimeInvariants` 的
`fsatmax/fsatdcv/topoweti/alp_twi/chi_twi/mu_twi/vic_*` 清零。
其余缺失行经逐项核对属 material/tracer 重构或上游新实现，无需恢复。

## 2026-11-02（续三）：恢复宿主补丁后的 MaterialTransport 适配

恢复 `HYDRO/MOD_Hydro_SoilWater` 与 `MOD_SoilSnowHydrology` 的桌面补丁后，
4c MaterialTransport 的 `qinfl_mt` 仍按 4c 语义再扣一次池水交换，构成双重扣减，
同位素算例在第 55 步签名检查失败。修法：`CoLMMAIN` 在 `DEF_USE_VariablySaturatedFlow`
打开时直接用桌面语义的 `qinfl`，非 VSF 保持原式。同位素 11 天两引擎通过，
material 量差 ≤2.8e-12，三站点黄金数值不变。
