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

### 16. VIC 产流那一支不给 `frcsat` 赋值

- **位置**：`main/MOD_SoilSnowHydrology.F90:999-1012`（`WATER_VSF` 的 `DEF_Runoff_SCHEME == 1`）。
- **原因**：`frcsat` 在 `WATER_VSF` 里是 `intent(out)`（`:741`），其余三个产流方案都给它赋值，
  VIC 这一支没有。按标准是未定义值；gfortran 下实际保留数组里原来的值（分配时的 `spval`），
  所以 `f_frcsat` 整列是填充值。
- **处理**：`vendor/` 未改（改了会改变输出）；Rust 照内核的实际结果，VIC 时不写 `f_frcsat`。

### 17. `-fdefault-real-8` 不带 `-fdefault-double-8`：带 `d0` 的表达式被提升成四倍精度

- **位置**：全部 `Makeoptions*`；受影响的是源码里写了 `1.0d0` 之类 `DOUBLE PRECISION` 字面量的表达式。
  全内核 GIMPLE 里有 7 个模块含 `real(kind=16)`：`HYDRO/MOD_Hydro_VIC.F90`（`calc_Q12`）、
  `MOD_3DCanopyRadiation.F90`、`MOD_IncompleteGamma.F90`、`URBAN/MOD_Urban_Shortwave.F90`、
  `URBAN/MOD_Urban_Longwave.F90`、`MOD_Utils.F90`（`lmder` 等）、`MOD_prospect_DB.F90`。
- **原因**：`-fdefault-real-8` 把默认 `REAL` 变成 8 字节的同时把 `DOUBLE PRECISION` 提升成 16 字节，
  这些表达式于是整条走 binary128（`powq` 等软件实现）。结果依赖编译器与 libquadmath：换 ifort，
  或加上 `-fdefault-double-8`，数值就不同；而且明显更慢。
- **影响**：`calc_Q12` 在"可排水量几乎为零"时（`tmp_liq ≈ resid_moist`）最后一步相消超过 53 位，
  Fortran 结果的末几位由 `powq` 的舍入决定。
- **处理**：`vendor/` 未改。Rust 用双倍双精度（`colm-core/src/extended.rs`，约 106 位）复现这些表达式；
  相消超过约 50 位的极少数情形不保证逐位一致。建议上游加 `-fdefault-double-8`，或把这些 `d0` 改成 `_r8`。

### 18. `VIC_IceLay` 在 4 层分组时把未初始化的 `intent(out)` 当累加器

- **位置**：`HYDRO/MOD_Hydro_VIC_Variables.F90` 的 `VIC_IceLay`，`colm_lay > 3` 那一支。
- **原因**：`vic_ice(1) = vic_ice(1) + …`、`vic_ice(3) = vic_ice(3) + …` 读的是 `intent(out)` 的 `vic_ice`，
  之前没有清零。默认 10 层按 3/3/4 分组，最深一组（土层 7–10）走这一支；gfortran 下读到的是调用方
  `vic_para` 同一个局部数组里上一组（土层 4–6）刚写下的冻土区冰量。于是最深 VIC 层三个冻土区的冰是
  `wice(4)+wice(7)`、`总和-两端`、`wice(6)+wice(10)`，中间那一区可能为负，也不守恒于本组。
  另外这一组本意的拆分也有问题：`multiplier = merge((colm_lay-idx*vic_lay)/vic_lay, 0, …)` 是整数除法，恒为 0。
- **处理**：`vendor/` 未改；Rust 照内核的实际行为复现（`vic.rs::partition_ice`），以保持两个引擎可比。
  建议上游在循环前 `vic_ice = 0`，并重写这一支的拆分。

### 19. `adjust_lake_layer` 对零深度读未初始化数组、对极薄湖层跳过重映射

- **位置**：`main/MOD_Lake.F90` 的 `adjust_lake_layer`（上游 `3c799bae` 的 `:2102-2130`、`:2176-2182`）。
- **原因**：`dz_lake_new`/`t_lake_new`/`lake_icefrac_new` 只在总深度为正时赋值，总深度为 0 时
  整层读回未初始化的值；重叠循环用固定的 `DO WHILE (resi > 1.e-8)`，当新层厚度本身小于 `1e-8`
  （实测总深 `1e-20`）时一次都不进，新层的温度与冰量没有来源。负厚度与 NaN 也不拒绝。
  只有 `DEF_USE_Dynamic_Lake` 会走到这里（定深湖不调它）。
- **处理**：`vendor/` 已改（`:2084-2136`）：拒绝非有限与负厚度，总深为 0 时原样返回，
  循环条件改成 `resi > 0._r8`（`olp = min(resi, resj)` 必然耗尽 `resi` 或推进 `j`，精确 0 就是终点）。
  正深度的重映射结果不变。Rust `lake.rs::adjust_lake_layers` 同样处理。
  验证见 `docs/audit-2026-09-08-physics.md` 与 `oracle/scripts/test_physics_audit.py:291-403`。
  建议上游采纳同样的三处改动。

### 20. `UrbanTHERMAL` 更新 `fwsun` 之后仍用旧的 `fwsha` 求 `twall`

- **位置**：`main/URBAN/MOD_Urban_Thermal.F90:605`（`fwsha = 1. - fwsun`）、`:621`（`fwsun = fwsun + dfwsun`）、
  `:1002`（`twall = (twsun*fwsun + twsha*fwsha)/(fwsun + fwsha)`）。
- **原因**：`fwsha` 只在 `fwsun` 更新之前算一次（GIMPLE 里 `:605` 到 `:1002` 全程是同一个 `fwsha_1091`）。
  墙温在 `:607-617` 已经按**新**面积重分配过，`twall` 却用"新阳面 + 旧阴面"的权重；`dfwsun > 0` 时
  阴面权重偏大 `dfwsun`，反之偏小。分母做了归一，所以结果仍是加权平均，只是权重错了。
- **影响**：只有诊断量 `twall`（history `f_t_wall`）。`:621` 之后 `fwsha` 再无别的有效用处
  （`:798/956/1258` 里只出现在注释中）。AU-Preston 第 1 步差 1.4e-3 K。
- **处理**：`vendor/` 未改（改了会改变输出）；Rust 照内核的实际行为，**不**在更新后重算 `fwsha`
  （`urban_thermal.rs`）。建议上游在 `:621` 之后补 `fwsha = 1. - fwsun`。

### 21. `UrbanTHERMAL` 读未初始化的 `dT(5)`

- **位置**：`main/URBAN/MOD_Urban_Thermal.F90:749`（`allocate (dT(0:5))`）、`:1048-1052`（只给 `dT(0:4)` 赋值）、
  `:1235`（`dX = matmul(Ainv, dBdT*dT(1:))`）。
- **原因**：有树时 `dT` 有 6 个元素，但 `dT(5)`（树冠温度变化）从不赋值；`allocate` 不清零，读到的是堆上残值。
- **影响**：`dX` 的每个分量都含 `Ainv(i,5)*dBdT(5)*dT(5)`，进而进 `dlw*`、`lout`、`olrg`。
  实测 AU-Preston 1488 步 Fortran 读到的全是 0（macOS 的新分配页），所以结果恰好等于 `dT(5) = 0`；
  换平台或换分配器可能不同。
- **处理**：`vendor/` 未改；Rust 取 0（`urban_thermal.rs`）。建议上游赋 `dT(5) = 0.`（叶温已在
  `UrbanVegFlux` 里闭合，长波增量不应再计一次）。

### 22. 城市冷启动把未初始化的 `t_roof`/`t_wall` 写进重启

- **位置**：`main/URBAN/MOD_Urban_Vars_TimeVariables.F90:204`（`allocate (t_roof(numurban))`）、`:396`
  （`ncio_write_vector (…, 't_roof', …)`）；`mkinidata/` 里没有任何地方给它们赋值。
- **原因**：`t_roof`/`t_wall` 是诊断量，只在 `UrbanTHERMAL` 里算；冷启动 `allocate` 后直接写出。
- **影响**：只有冷启动重启里的这两个值（实测 macOS 上是 0）；第一步 `UrbanTHERMAL` 会覆盖它们。
- **处理**：`vendor/` 未改；colm-init 照实测写 0（`urban_restart.rs`）。建议上游初始化为 `tref` 或 0。

### 23. 单点历史写回模式下 `DEF_HIST_FREQ = 'none'` 读未初始化的 `secs_write`（SIGILL）

- **位置**：`main/MOD_HistSingle.F90:59-72`（`hist_single_init`）。
- **原因**：`USE_SITE_HistWriteBack` 为真时按 `DEF_HIST_FREQ` 选 `secs_write`，`SELECT CASE` 没有 `CASE DEFAULT`；
  `'none'`（声明默认值）走不到任何分支，随即 `ntime_mem = ceiling(secs_group / secs_write) + 2` 读未初始化的
  `secs_write`。gfortran -O2 把这条路径当未定义行为，编出陷阱指令。
- **影响**：单点、`USE_SITE_HistWriteBack = .true.`、`DEF_HIST_FREQ = 'none'` 时 `colm.x` 一启动就 SIGILL。
  回溯（按 ASLR 偏移 `0x980000` 符号化）：`hist_single_init + 175` ← `hist_init + 227` ← `MAIN__`。
  关掉写回（`USE_SITE_HistWriteBack = .false.`）即可正常跑，历史累加照常进行、只是不写文件。
- **处理**：`vendor/` 未改；Rust 引擎不走写回缓冲，`'none'` 下照上游语义只累加、不写历史，续跑旁车存整段原始窗口
  （与关掉写回的 Fortran 逐位一致，第 431 轮）。建议上游补 `CASE DEFAULT`（或 `'none'` 时不建写回缓冲）。

### 24. `CNFireArea`/`CNFireFluxes` 的 `ivt` 从不赋值

- **位置**：`main/BGC/MOD_BGC_Veg_CNFireLi2016.F90:66`（声明，`:159` 有注释 "Warning : ivt is not initialized."）起
  `isnatveg(ivt)`/`isbare(ivt)`/`fsr_pft(ivt)`/`fd_pft(ivt)`/`iscrop(ivt)`；`MOD_BGC_Veg_CNFireBase.F90:101` 起
  `cc_*(ivt)`/`fm_*(ivt)`/`lf_*(ivt)`/`fr_*(ivt)`。
- **原因**：两个过程都声明了局部 `ivt` 却从不赋值（应当是逐 PFT 的 `pftclass(m)`）。GIMPLE 里是未定义值
  `ivt_858(D)`；两份内核（default、crop）的反汇编都直接读各常数表的**第 0 项**（`isbare`、`isnatveg`、`cc_leaf`… 的
  基址、无下标偏移），即按裸土处理。
- **影响**：火烧面积只剩泥炭火（`fsr_pft(0) = fd_pft(0) = 0`），植被燃烧系数 `cc_*(0) = 0`；纯作物 patch
  （`cropf = 1`）也走自然植被那一支，`1/(1-cropf)`、`fd_pft(0)*3600/(1-cropf)` 必然除零——内核带
  `-ffpe-trap=invalid,zero,overflow`，CROP 内核打开 FIRE 第一步即 SIGILL（lldb：`cnfirearea+33892` 的 `fdiv`）。
- **处理**：`vendor/` 未改；Rust 照编译产物按 0（`regen.py` 的 `ZERO_INIT`），并在 `CNFireArea` 之后检查输出是否
  有限、不有限就同样终止（`bgc_fire_support::ensure_no_fp_trap`）。建议上游在 PFT 循环里取 `ivt = pftclass(m)`。

### 25. FIRE 的其他记账错误（`CNFireFluxes`）

- **位置**：`main/BGC/MOD_BGC_Veg_CNFireBase.F90`。
- **原因与影响**：
  - `m_deadstemc_to_litter_fire_p = deadstemc_p(m) * f * m * …`（及 `deadcrootc`/`deadstemn` 同式）：乘的是 PFT 下标 `m`，
    应当是 `mort`；
  - `fire_mortality_to_cel_n`/`lig_n` 用 `lf_fcel(i)`/`fr_flig(i)`：下标是 patch 号而不是 PFT 类别；
  - 每步只清零 `fire_mortality_to_cwdc/cwdn/met_c` 三项，`cel_c`/`lig_c`/`met_n`/`cel_n`/`lig_n` 靠别处清零；
  - 凋落物与粗木质残体的燃烧（`m_decomp_cpools_to_fire_vr`）从池里扣掉，却不计入 `CBalanceCheck` 的输出项
    （`fire_closs` 恒为 0），收支误差随火烧面积累积：合成数据下 AT-Neu 2010-08-15 第 10885 步
    `column cbalance error = 1.0002e-7` 超限 abort。
- **处理**：`vendor/` 未改；Rust 照转写（生成代码逐字复现），在同一步以同样的收支误差终止。

### 26. FIRE 的五个历史量写的是临时数组 `vecacc`

- **位置**：`main/MOD_Hist.F90:1937-1957`（`f_abm`/`f_gdp`/`f_peatf`/`f_hdm`/`f_lnfm`）。
- **原因**：五次都把 `vecacc` 传给 `write_history_variable_2d`，而不是 `a_abm` 等累加器；后者还会原地
  `vecacc = vecacc/nac`（非 `spval` 处）并把过滤掉的 patch 置 `spval`。
- **影响**：写出的是上一次用 `vecacc` 写历史后的残留、每个量再多除一次 `nac`：默认内核里上一次是 `f_wetzwt`
  （湿地过滤，BGC patch 恒为土壤 → 五个量全是缺测），CROP 内核里灌溉关时是 `f_grainc_to_cropprodc`、
  灌溉开时是 `f_runoff_supply`（`filter_irrig`）。
- **处理**：`vendor/` 未改；Rust 照写（`history.rs` 的 `FIRE_HISTORY` 与 `write_fire_history`），`a_abm` 等照常累加进旁车。

### 27. 灌溉历史量：`f_sum_deficit_irrig` 恒缺测，三个"年累计"量被除以 `nac`

- **位置**：`main/MOD_Vars_1DAccFluxes.F90:2423-2432`、`main/MOD_Hist.F90:1507-1530`。
- **原因**：`accumulate_fluxes` 的灌溉段写 `a_sum_irrig = sum_irrig`、`a_sum_irrig_count = sum_irrig_count`、
  `a_waterstorage = waterstorage`（**赋值**，不是 `acc1d`），却漏了 `a_sum_deficit_irrig`；写历史时四个量照样按
  `write_history_variable_2d` 的 `/nac` 处理。
- **影响**：`f_sum_deficit_irrig` 永远是缺测；`f_sum_irrig`/`f_sum_irrig_count`/`f_waterstorage` 写出的是
  "窗口末步的年累计值 / 窗口步数"（日输出时是 /24），既不是累计也不是平均。
- **处理**：`vendor/` 未改；Rust 照写（`history.rs` 的 `ASSIGNED_VARIABLES` 与 `irrigation_history_value`），
  旁车里存末值，与上游 `a_*` 一致。

### 28. 灌溉的逐 PFT 循环不适合多 PFT patch（潜在）

- **位置**：`main/MOD_Irrigation.F90:188-240`（`CalIrrigationPotentialNeeded`）、`:262-281`
  （`CalIrrigationApplicationFluxes`）、`:308-327`（`PointNeedsCheckForIrrig`）。
- **原因**：需水量的 PFT 循环不重置 `reached_max_depth` 与各总量，第二个 PFT 起只在第一个没碰到深度上限时再累加
  一遍；`deficit_irrig`/`check_for_irrig` 都取最后一个 PFT 的结果；施灌时每个 PFT 各减一次
  `n_irrig_steps_left`、各从 `waterstorage` 扣一次。
- **影响**：CROP 构建里作物 patch 只有一个 PFT，碰不到；若 patch 内有多个 PFT，灌溉量与持续步数都不对。
- **处理**：`vendor/` 未改；Rust 逐字照搬（`colm-core/src/irrigation.rs`）。

### 29. `f_irrig_method_corn` 只写雨养玉米

- **位置**：`main/MOD_Hist.F90:2803-2826`。
- **原因**：玉米的过滤条件是 `pftclass == 17`（雨养温带玉米），其余七种作物都是"雨养 + 灌溉"两个类别
  （如春小麦 19/20）。
- **影响**：灌溉玉米（18）patch 上 `f_irrig_method_corn` 是缺测，而这正是唯一会灌溉的玉米。
- **处理**：`vendor/` 未改；Rust 照写（`CROP_TYPE_HISTORY` 的 `irrig_method_corn` 只认 17）。

### 30. 单点 mksrfdata 正常结束时退出码为 1（`85cf2328` 引入）

- **位置**：`mksrfdata/MKSRFDATA.F90:150`（单点分支）、`share/MOD_SPMD_Task.F90:339`。
- **原因**：单点分支写完 `'Successful in surface data making.'` 后调 `CoLM_stop()` 结束；PR #17 把
  `CoLM_stop` 的非 MPI 实现从 `STOP` 改成 `STOP 1`，以区分出错退出。
- **影响**：每次成功的单点 mksrfdata 都以退出码 1 结束，调用方（`colm-cli`）按失败处理，后续阶段不跑。
- **处理**：本地把这一处改回 `STOP`（`vendor/PROVENANCE.md`），应当报给上游。

### 31. TOPMODEL 方法 0 把未赋值的 `topoweti`/`alp_twi`/`chi_twi`/`mu_twi` 写进常数重启

- **位置**：`mkinidata/MOD_Initialize.F90:513-564`。
- **原因**：`DEF_TOPMOD_method == 0` 只赋 `fsatmax`/`fsatdcf`，另外四个量只在方法 1、2 读文件；分配后没有初值。
- **影响**：写进常数重启的是未定义内存（单点纯 Fortran 实测为 0）。方法 0 下它们不参与计算，结果不受影响。
- **处理**：`vendor/` 未改；Rust 写 0（原先写的是自拟占位值 9.27/1.34/1.61/6.95），与实测一致。

### 32. `DEF_LC_RESPCP` 不起作用

- **位置**：`main/MOD_Const_LC.F90:902`（覆盖）、`main/MOD_AssimStomataConductance.F90:592`。
- **原因**：`stomata` 里的 `respcp` 是局部量，每次按 `0.015*c3 + 0.025*c4` 重算，地类表的 `respcp` 从未传进去。
- **影响**：设了 `DEF_LC_RESPCP` 也不改变任何结果。
- **处理**：Rust 照样解析、保存，不使用。

### 33. 多作物单点每个 patch 的 `tlai`/`tsai` 是各作物之和

- **位置**：`main/MOD_LAIReadin.F90:171-175`（单点 PFT 段）、`mksrfdata/MOD_SingleSrfdata.F90:405-411`。
- **原因**：`tlai(:) = sum(SITE_LAI_pfts_monthly(:,time,iyear) * SITE_pctpfts)` 给所有 patch 赋同一个全站和；
  农田站点 `SITE_pctpfts = 1.`（不是 `pctcrop`），于是是各作物 LAI 直接相加。
- **影响**：只有一种作物时无害；多作物站点每个 patch 的 `tlai`/`tsai`（以及由它们折算的 `lai`/`sai`、辐射、
  冠层）都偏大，且所有 patch 相同，与各自的 `tlai_p` 不一致。`DEF_USE_LAIFEEDBACK` 下 `tlai` 不走这里，`tsai` 仍然。
- **处理**：`vendor/` 未改；Rust 照写（`colm-runtime/src/pft.rs` 的 `refresh_monthly_leaf_area_index`）。

### 34. 零示踪物时雪层合并/分裂传入未分配的示踪物数组切片

- **位置**：`main/CoLMMAIN.F90` 的 `snowlayerscombine[_snicar]`、`snowlayersdivide[_snicar]` 四处调用。
- **原因**：条件只看 `DEF_USE_TRACER`，就把 `trc_wliq_soisno(:, lb:1, ipatch)` 等切片作为可选实参传进去；
  而 `trc_*` 只在 `ntracers > 0` 时分配（`MOD_Tracer_Vars.F90:244`）。`DEF_TRACER_NUM = 0` 时是对未分配数组取切片，
  属未定义行为（紧随其后的 `relocate_soil_frost_ice` 调用有 `ntracers > 0` 判断，这四处没有）。
- **影响**：没有雪层合并/分裂的算例碰不到；有雪时可能崩溃或读到垃圾。
- **处理**：`vendor/` 已修（第 479 轮）：四处条件改为 `DEF_USE_TRACER .and. ntracers > 0`。

### 35. 河道示踪物供体限幅后的"负质量"判据是绝对量，恰好排空的单元因 1 ulp 舍入停机

- **位置**：`main/TRACER/MOD_Tracer_RiverLake.F90` 的 `tracer_substep` 第 8 节（可见池与防洪堤保护池两处更新后的检查）。
- **原因**：判据是 `trc_mass_new < -TRC_RESTART_NEGATIVE_DUST`（`1e-12`，**绝对**量）。供体限幅把恰好排空的单元的速率定成
  `(mass+inflow)/outflow`，更新后的质量在数学上是 0、数值上落在这次更新各项量级的几个 ulp 之内；河道单元的示踪物
  质量是"体积×浓度"，量级 1e4–1e8，ulp 远大于 1e-12。
- **证据**：`g1ts`（2003 年 1 月、IsoGSM 驱动、1 个溶质、GRID + 河道）第 2 步停在
  `negative river tracer mass after coupled donor limiter`；调试打印：单元 4458，`mass = 3.48749877929687500E+04`，
  `trc_out_mass` 与之相等（rate = 1），`trc_mass_new = -3.4955E-12`，相对 `-1.0E-16`（1 ulp）。
- **影响**：任何开示踪物的 GRID 空间算例都会在第一次有单元被排空时停机（上游 `Amazon_*` 示例同样会碰到）。
- **处理**：`vendor/` 已修（第 487 轮）：两处判据改成 `-max(DUST, 1e-12 × (|旧质量| + (|flux|+|flux_ups|+|bif_net|)·dt))`，
  其余不变（仍 `max(·,0)` 夹到 0）；`g1ts` 此后跑完 2 天。

### 36. LAI 反馈下非土壤 patch 的初始 LAI 是 `spval`

- **位置**：`mkinidata/MOD_Initialize.F90` 的 `LAI_readin` 之后；`main/MOD_LAIReadin.F90` 的 PFT/PC 支。
- **原因**：`DEF_USE_LAIFEEDBACK` 时 PFT/PC 支只读 `SAI`，`tlai` 交给 BGC。土壤 patch 随后由 BGC 叶碳给出，
  湿地、城市、水体等没有 BGC 叶碳的 patch 一直保留分配时的 `spval`，`iniTimeVar` 再把它抄进 `lai`。
- **证据**：CROP 内核空间算例（`g1crop`，纯 Fortran 前处理）的初始续跑里，湿地与城市 patch 的 `tlai = lai = -1e36`。
  整段运行都带着它：`lai+sai` 为大负数，走无冠层支；截留等处则直接拿它做乘法。
- **影响**：空间 PFT/PC + BGC + LAI 反馈的所有非土壤 patch。Rust 的 mkinidata 在这里给 0，两种前处理因此不一致。
- **处理**：`vendor/` 已修（第 502 轮）：`LAI_readin` 之后把仍是 `spval` 的 `tlai` 置 0（裸地起步），与 Rust 前处理一致。

### 37. 不开灌溉时网格 history 的 `irrigarea` 用未初始化的 `filter_irrig`

- **位置**：`main/MOD_Hist.F90:446-477`（`#ifdef CROP`）。
- **原因**：`filter_irrig` 在 370 行 `allocate` 后只在 `IF (DEF_USE_IRRIGATION)` 里逐 patch 赋值，
  `get_sumarea (sumarea_irrig, filter_irrig)` 与 `irrigarea` 的写出却不受该开关约束。
- **证据**：`g1crop`（CROP 内核、灌溉关闭）的 `irrigarea` 恰好全为 0 —— 新分配的内存碰巧是零，
  换平台或换分配器就可能是任意值。
- **影响**：CROP 内核、`HistForm = 'Gridded'`、`DEF_USE_IRRIGATION = .false.` 时的 `irrigarea` 静态量（不影响物理）。
- **处理**：Rust 按"全假"写 0（第 503 轮），与本机观测一致；vendor 未改。

### 38. `run/forcing/GDAS.nml` 的 `missing_value_name` 漏了 `DEF_forcing%` 前缀

- **位置**：`run/forcing/GDAS.nml:16`（CoLM-SYSU-integration 当前版本相同）。
- **原因**：`nl_colm_forcing` 组只有 `DEF_dir_forcing` 与 `DEF_forcing` 两个对象
  （`share/MOD_Namelist.F90:1625`），裸写的 `missing_value_name` 不是组内对象名。
- **证据**：`MOD_Namelist.F90:1644` 带 `iostat` 读这一组，非零即 `CoLM_Stop`；原样的 GDAS.nml 读不进来。
  其余 20 份 vendor 强迫 namelist 逐行扫过，没有第二处裸写的键。
- **影响**：GDAS 强迫的算例启动即停。
- **处理**：vendor 改成 `DEF_forcing%missing_value_name`（第 508 轮）。实测原样的 namelist 在 Fortran 内核里报
  `Cannot match namelist object name missing_value_name` 后停机；Rust 的网格强迫读取现在也拒绝组外对象名。

### 39. `run/forcing/CRA40.nml` 的 u 分量有名字却 `tintalgo = 'NULL'`

- **位置**：`run/forcing/CRA40.nml:47`（CoLM-SYSU-integration 当前版本相同）：
  `vname(5) = 'U_GRD_GDS0_HTGL'`、`fprefix(5) = 'CRA40_sp_t_u_v'`、`dtime(5) = 21600`，`tintalgo(5)` 却是 `'NULL'`。
- **原因**：`MOD_Forcing.F90:445-448` 只按 `vname` 判断有没有这个变量：
  - `has_u` 仍为真；
  - `metreadLBUB` 照读上下界；
  - 插值循环因 `tintalgo == 'NULL'` 跳过，`forcn(5)` 从不赋值。
  `forcn` 由 `allocate_block_data` 分配、不初始化，于是 `forc_us` 取的是未初始化内存（`has_u .and. has_v` 分支直接用它）。
- **证据**：vendor 21 份网格 namelist 逐项扫过 `vname` 与 `tintalgo` 的 NULL 是否一致，只有这一处不一致。
- **影响**：CRA40 强迫的东西向风是未定义值。
- **处理**：vendor 把 `tintalgo(5)` 改成 `'linear'`，与 v 分量相同（第 508 轮）。Rust 的网格强迫读取遇到"有名字但 `tintalgo = 'NULL'`"直接拒绝：这种组合没有确定的上游行为可以对齐。

### 40. GIEMS 的 NaN 填充值在标准构建下触发浮点陷阱

- **位置**：`main/TRACER/MOD_Tracer_Reactive_Methane_GIEMS.F90:599`（CoLM-SYSU-integration 当前版本相同）。
  原写法 `IF (v >= 0._r4 .and. v <= 1._r4) … ELSEIF (.not. ieee_is_nan(v) .and. …)`。
- **原因**：模块注释和这一段的注释都把 NaN 列为合法的填充值（当作物理上的 0），但先做的是有序比较 `v >= 0`。
  - 有序比较是带信号的比较（arm64 上是 `fcmpe`），遇到 NaN 会置 invalid；
  - 上游标准 Makeoptions 带 `-ffpe-trap=invalid,zero,overflow`，于是直接陷入；
  - 后面的 `ieee_is_nan` 分支永远到不了。
- **证据**：合成的 GIEMS 文件在站点像元的某些月份放 NaN。单点 `tc4g` 在 `GIEMS dims` 之后以 SIGILL 停机，
  lldb 定位在 `read_methane_giems+4052: fcmpe s31, #0.0`。空间 `g1ch4g` 取到的像元碰巧没有 NaN，所以没暴露。
- **影响**：只要选中的像元在任何一个月是 NaN，方案 5（`satellite`/`giems`）的初始化就会崩溃。
- **处理**：vendor 改为先判 `ieee_is_nan(v)`（计入该月的样本数、不进和），再做有序比较（第 520 轮）。
  对非 NaN 的输入，行为与原写法完全相同。Rust 的 `GiemsPatch::from_samples` 本来就先按区间判断，NaN 不会出错。

### 41. `DEF_forcing%groupby = 'day'` 没有对应的按天文件名

- **位置**：
  - `main/MOD_Forcing.F90:1551-1590`（`setstampLB`）与 `:1719-1752`（`setstampUB`）：都有 `groupby == 'day'` 分支；
  - `main/MOD_UserSpecifiedForcing.F90:153` 的 `metfilename(year, month, day, …)`：全部 24 个数据集分支都不用 `day` 参数。
- **原因**：`day` 分支把记录号算成日内的 `floor((sec - offset)/dtime) + 1`，默认每个文件只装一天；
  但文件名只到年或月，读的仍是按年/月分的文件。
- **影响**：
  - 选 `day` 时，每天都从年/月文件的开头几条记录读起，强迫静默错误（不停机）；
  - vendor 的 21 份网格强迫 namelist 没有一份用 `day`，所以默认配置不受影响。
- **处理**：vendor 未改，因为上游要么补按天的文件名，要么删掉这个选项。Rust 在解析强迫配置时直接拒绝 `day`，
  报错信息说明原因（第 522 轮）；此前它只笼统报"not ported"。

### 42. 双线性强迫映射会取到区域块覆盖之外的格子

- **位置**：`share/MOD_SpatialMapping.F90:537-989`（`spatial_mapping_build_bilinear`），越界点在 `:753` 的 `gblock%pio(xblk,yblk)`。
- **原因**：
  - `grid_set_blocks`（`share/MOD_Grid.F90:496-679`）只给与 `DEF_domain` 相交的强迫行列分配块号，其余是 0；
  - 双线性按 patch 中心找两侧格心，中心落在最外一排格心与区域边界之间时，另一侧的格心在区域外，于是 `xblk/yblk = 0`。
- **证据**：g1f 底（区域 23–25°N、113–115°E，合成 CMFD 强迫）加 `DEF_Forcing_Interp_Method = 'bilinear'`。
  - 生产内核：在 "Building bilinear interpolation" 之后 SIGSEGV；
  - 带越界检查的 debug 内核：`At line 753 of file share/MOD_SpatialMapping.F90: Index '0' of dimension 2 of array 'gblock%pio' below lower bound of 1`。
- **影响**：几乎所有区域算例都会撞上。即使不越界，那些格子的强迫也不在读入的块里。只有全球区域才能跑。
- **处理**：vendor 未改。正确的修法要上游决定：是把双线性的邻格扩进块覆盖，还是在区域边缘退化为最近格心。
  Rust 已按上游写法移植 `build_bilinear`（含单测），但入口拒绝 `bilinear` 并说明原因（第 525 轮）。

### 43. LULCC 之后 `scale_baseflow` 仍按旧年 patch 编号取值

- **位置**：
  - `main/ParaOpt/MOD_Opt_Baseflow.F90:23-58`（`Opt_Baseflow_init`）：启动时按当年 `landpatch` 读 `ParaOpt/<case>_baseflow.nc` 的 `scale_baseflow`，缺文件时取 1；
  - `main/MOD_SoilSnowHydrology.F90:1022`：每步 `rsubst = rsubst * scale_baseflow(ipatch)`，**与 `DEF_Optimize_Baseflow` 无关**；
  - `main/LULCC/*`：换年后不重分配、也不重映射这个数组。
- **原因**：标定文件不带年份，按旧年 patch 布局存放；LULCC 改了 patch 布局，数组却原样保留。
- **影响**：
  - 新年 patch 少于旧年时：新 patch `ipatch` 拿到旧年第 `ipatch` 个 patch 的系数，系数与 patch 错位，结果静默错误；
  - 多于旧年时：越界读（生产构建不查界）；
  - 没有标定文件（全是 1）时无影响，现有 LULCC 算例都属于这种（g3：175→172；g3p、g3c 的 patch 数不变）。
  - 下一段续跑重新读文件时仍按旧布局，错位依旧。
- **处理**：vendor 未改。正确的修法需要上游定：要么标定文件按土地覆盖年份分开，要么在 LULCC 里按 SAT 配对重映射、新类型 patch 取 1。
  - Rust 与 Fortran 的取值方式相同：按新 patch 编号读文件，文件太短时报错，而不是越界读。
  - `DEF_Optimize_Baseflow`（只在 spinup 里迭代）与 LULCC 同开时，Rust 仍拒绝，理由改为本条（第 534 轮）。

### 44. MEC 城市段在两种情况下读失效的来源下标

- **位置**：`main/LULCC/MOD_Lulcc_MassEnergyConserve.F90:950-1096`。
- **情况一（旧单元里没有城市 patch）**：
  - `selfu_ = -1` 与 `gu_` 只在 `nurb > 0` 时设，`u_` 也只在 `selfu_ > 0` 或 `nurb > 0` 时赋值；
  - `nurb == 0` 时，两者都沿用**上一个城市 patch**（可能在别的单元）的值，`u.le.0 .or. u_.le.0` 也拦不住，于是新城市从别处的城市单元抄状态；
  - 若这是本 worker 的第一个城市 patch，读到的就是未初始化值。
  - 注释写的是"保留冷启动值"，代码没做到。
- **情况二（份额没变、旧单元里又没有同城市类型）**：`FROM_SOIL` 遍历 `frnp_(1:num)` 判断来源里有没有土壤 patch，可 `frnp_` 只在份额有变化的分支里赋值，这里读的是刚 `allocate` 的未定义内容。
- **影响**：只在城市布局逐年变化时出现。现有算例 g3um 两年城市布局相同，走不到。
- **处理**：vendor 未改。情况一的正确做法应是跳过（保留冷启动值）；情况二应只在有来源时判断。Rust 在这两种情况下都拒绝并指向本条（第 534 轮）。

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

**本仓库的处理（第 477、478 轮）**：维护者决定示踪物只记账、不改宿主，于是在 vendor 里把上表
TRACER 那一列（以及卸雪速率、`fwet` 容量、负蒸腾记为凝露、氮分解合并项、水量平衡显式漫滩项等同类分支）
**改为无条件生效**，覆盖原写法；Rust 侧同步移植。上表第 6 条（方案 8 的逐 PFT 储水能力）Rust 的
PFT/PC 路径仍拒绝方案 8，未受影响。开关现在只控制示踪物自身的记账。

## 三、已修（上游已修掉，留作记录）

- `MOD_LeafTemperaturePC` 的 `o3coef*` 迭代后才赋值：上游 `FIX 2026-08-16 #4` 已在迭代前置 1
  （臭氧关闭时，见第 5 条的遗留）。
- 扩展截获 `MOD_LeafTemperature_Extended.F90` 的 `rstfacsun/sha` 为 `intent(out)`：上游 `d6de53e9`
  起不再编译扩展截获，问题随之消失。
- `create_defineh.bash` 的 `#error "TRACER requires GridRiverLakeFlow"`：上游已去掉。
