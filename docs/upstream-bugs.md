# 上游 CoLM 的缺陷与不一致（待报上游）

对象：`https://github.com/zhongwangwei/CoLM-SYSU-integration`（本地
`/Users/zhongwangwei/Desktop/Github/CoLM-SYSU-integration`）。行号按 **`3c799bae`**。
每条写清楚：在哪、为什么是错的、影响范围、本仓库 `vendor/CoLM202X` 怎么处理。
新发现的追加在末尾，已被上游修掉的挪到「已修」一节并注明上游提交。

证据与数值影响的细节以及上游同步过程在 `docs/implementation-verification.md` 对应轮次。

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
- **处理**：删去这两行（vendor 原来漏删，第 560 轮补上）。

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
- **处理**：迭代前无条件置 1（臭氧打开时迭代后的 `CalcOzoneStress` 照常覆盖）。LCT（`MOD_LeafTemperature`）早已如此；
  PC（`MOD_LeafTemperaturePC`）原来仍只在臭氧关闭时置 1，第 560 轮补齐。Rust 尚未移植臭氧胁迫，`DEF_USE_OZONESTRESS = .true.` 现在明确拒绝。

### 16. VIC 产流那一支不给 `frcsat` 赋值

- **位置**：`main/MOD_SoilSnowHydrology.F90:999-1012`（`WATER_VSF` 的 `DEF_Runoff_SCHEME == 1`）。
- **原因**：`frcsat` 在 `WATER_VSF` 里是 `intent(out)`（`:741`），其余三个产流方案都给它赋值，
  VIC 这一支没有。按标准是未定义值；gfortran 下实际保留数组里原来的值（分配时的 `spval`），
  所以 `f_frcsat` 整列是填充值。
- **处理**：vendor 已修（第 555 轮）：`Runoff_VIC` 新增 `intent(out) frcsat = cell%asat`，`WATER_VSF` 的 VIC 支传进
  `frcsat`，另外三处不保留它的调用传局部 `frcsat_vic`；动态湿地支补 `frcsat = 1.`。Rust 的 VIC 返回饱和面积比
  （`water_2014.rs::vic_runoff_for`），VSF 写出它。只有干湖的 `f_frcsat` 仍是填充值（`CoLMMAIN.F90:1237`）。

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
- **处理**：vendor 已修（第 558 轮）：所有 gnu 的 `Makeoptions*`（及 `oracle/scripts` 的对照脚本）加 `-fdefault-double-8`，
  这些表达式回到 double，与 ifort `-r8` 的语义一致。受影响的是 VIC `calc_Q12`、`MOD_IncompleteGamma`、`MOD_Utils`
  的 `lmder` 与 `MOD_prospect_DB`（GIMPLE 里 kind=16 的行数 21/32/97/118 → 0）；Rust 对应改成普通 f64（去掉 GRATIO 的
  `F128` 与 VIC 用的双倍双精度），逐位对照全部为 0 不一致。`MOD_3DCanopyRadiation` 与城市长短波源码里显式写
  `real(r16)`，不受这个选项影响，Rust 仍用 `extended.rs` 复现。

### 18. `VIC_IceLay` 在 4 层分组时把未初始化的 `intent(out)` 当累加器

- **位置**：`HYDRO/MOD_Hydro_VIC_Variables.F90` 的 `VIC_IceLay`，`colm_lay > 3` 那一支。
- **原因**：`vic_ice(1) = vic_ice(1) + …`、`vic_ice(3) = vic_ice(3) + …` 读的是 `intent(out)` 的 `vic_ice`，
  之前没有清零。默认 10 层按 3/3/4 分组，最深一组（土层 7–10）走这一支；gfortran 下读到的是调用方
  `vic_para` 同一个局部数组里上一组（土层 4–6）刚写下的冻土区冰量。于是最深 VIC 层三个冻土区的冰是
  `wice(4)+wice(7)`、`总和-两端`、`wice(6)+wice(10)`，中间那一区可能为负，也不守恒于本组。
  另外这一组本意的拆分也有问题：`multiplier = merge((colm_lay-idx*vic_lay)/vic_lay, 0, …)` 是整数除法，恒为 0。
- **处理**：vendor 已修（第 555 轮）：`VIC_IceLay` 的 ELSE 支先 `vic_ice = 0.`。Rust `vic.rs::partition_ice`
  从零起算，不再读上一组的残值。整数除法那一处的拆分逻辑保持上游原意不动（它只决定分到哪一区）。

### 19. `adjust_lake_layer` 对零深度读未初始化数组、对极薄湖层跳过重映射

- **位置**：`main/MOD_Lake.F90` 的 `adjust_lake_layer`（上游 `3c799bae` 的 `:2102-2130`、`:2176-2182`）。
- **原因**：`dz_lake_new`/`t_lake_new`/`lake_icefrac_new` 只在总深度为正时赋值，总深度为 0 时
  整层读回未初始化的值；重叠循环用固定的 `DO WHILE (resi > 1.e-8)`，当新层厚度本身小于 `1e-8`
  （实测总深 `1e-20`）时一次都不进，新层的温度与冰量没有来源。负厚度与 NaN 也不拒绝。
  只有 `DEF_USE_Dynamic_Lake` 会走到这里（定深湖不调它）。
- **处理**：`vendor/` 已改（`:2084-2136`）：拒绝非有限与负厚度，总深为 0 时原样返回，
  循环条件改成 `resi > 0._r8`（`olp = min(resi, resj)` 必然耗尽 `resi` 或推进 `j`，精确 0 就是终点）。
  正深度的重映射结果不变。Rust `lake.rs::adjust_lake_layers` 同样处理。
  验证见 `oracle/scripts/test_physics_audit.py:291-403`。
  建议上游采纳同样的三处改动。

### 20. `UrbanTHERMAL` 更新 `fwsun` 之后仍用旧的 `fwsha` 求 `twall`

- **位置**：`main/URBAN/MOD_Urban_Thermal.F90:605`（`fwsha = 1. - fwsun`）、`:621`（`fwsun = fwsun + dfwsun`）、
  `:1002`（`twall = (twsun*fwsun + twsha*fwsha)/(fwsun + fwsha)`）。
- **原因**：`fwsha` 只在 `fwsun` 更新之前算一次（GIMPLE 里 `:605` 到 `:1002` 全程是同一个 `fwsha_1091`）。
  墙温在 `:607-617` 已经按**新**面积重分配过，`twall` 却用"新阳面 + 旧阴面"的权重；`dfwsun > 0` 时
  阴面权重偏大 `dfwsun`，反之偏小。分母做了归一，所以结果仍是加权平均，只是权重错了。
- **影响**：只有诊断量 `twall`（history `f_t_wall`）。`:621` 之后 `fwsha` 再无别的有效用处
  （`:798/956/1258` 里只出现在注释中）。AU-Preston 第 1 步差 1.4e-3 K。
- **处理**：vendor 已修（第 554 轮）：`fwsun = fwsun + dfwsun` 之后补 `fwsha = 1. - fwsun`；Rust `urban_thermal.rs` 同样重算。

### 21. `UrbanTHERMAL` 读未初始化的 `dT(5)`

- **位置**：`main/URBAN/MOD_Urban_Thermal.F90:749`（`allocate (dT(0:5))`）、`:1048-1052`（只给 `dT(0:4)` 赋值）、
  `:1235`（`dX = matmul(Ainv, dBdT*dT(1:))`）。
- **原因**：有树时 `dT` 有 6 个元素，但 `dT(5)`（树冠温度变化）从不赋值；`allocate` 不清零，读到的是堆上残值。
- **影响**：`dX` 的每个分量都含 `Ainv(i,5)*dBdT(5)*dT(5)`，进而进 `dlw*`、`lout`、`olrg`。
  实测 AU-Preston 1488 步 Fortran 读到的全是 0（macOS 的新分配页），所以结果恰好等于 `dT(5) = 0`；
  换平台或换分配器可能不同。
- **处理**：vendor 已有 `IF (doveg) dT(5) = 0.`（与 Rust 一致），第 554 轮核对过，无需再改。

### 22. 城市冷启动把未初始化的 `t_roof`/`t_wall` 写进重启

- **位置**：`main/URBAN/MOD_Urban_Vars_TimeVariables.F90:204`（`allocate (t_roof(numurban))`）、`:396`
  （`ncio_write_vector (…, 't_roof', …)`）；`mkinidata/` 里没有任何地方给它们赋值。
- **原因**：`t_roof`/`t_wall` 是诊断量，只在 `UrbanTHERMAL` 里算；冷启动 `allocate` 后直接写出。
- **影响**：只有冷启动重启里的这两个值（实测 macOS 上是 0）；第一步 `UrbanTHERMAL` 会覆盖它们。
- **处理**：vendor 已修（第 554 轮）：分配后 `t_roof(:) = 0.; t_wall(:) = 0.`，与 colm-init 写的 0 一致。

### 23. 单点历史写回模式下 `DEF_HIST_FREQ = 'none'` 读未初始化的 `secs_write`（SIGILL）

- **位置**：`main/MOD_HistSingle.F90:59-72`（`hist_single_init`）。
- **原因**：`USE_SITE_HistWriteBack` 为真时按 `DEF_HIST_FREQ` 选 `secs_write`，`SELECT CASE` 没有 `CASE DEFAULT`；
  `'none'`（声明默认值）走不到任何分支，随即 `ntime_mem = ceiling(secs_group / secs_write) + 2` 读未初始化的
  `secs_write`。gfortran -O2 把这条路径当未定义行为，编出陷阱指令。
- **影响**：单点、`USE_SITE_HistWriteBack = .true.`、`DEF_HIST_FREQ = 'none'` 时 `colm.x` 一启动就 SIGILL。
  回溯（按 ASLR 偏移 `0x980000` 符号化）：`hist_single_init + 175` ← `hist_init + 227` ← `MAIN__`。
  关掉写回（`USE_SITE_HistWriteBack = .false.`）即可正常跑，历史累加照常进行、只是不写文件。
- **处理**：vendor 已修（第 555 轮）：补 `CASE DEFAULT`（`secs_write = secs_group`），`secs_group` 的 ELSE 支取一年。
  Rust 引擎本来不走写回缓冲，行为不变。

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
- **处理**：并入 CoLM-SYSU/CoLM PR #504（第 556 轮）：两个过程都在 PFT 循环里取 `ivt = pftclass(m)`。Rust 由
  `regen.py` 从新的 GIMPLE 重新生成 `bgc_fire.rs`，不再按 0 处理。

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
- **处理**：并入 PR #504（第 556 轮）：`m` → `mort`、下标改成 PFT 类别、各项每步清零，凋落物/粗木质残体燃烧计入
  `fire_closs` 进收支检查；另把泥炭火排放 `somc_fire` 记为诊断量（不从土壤池扣）。Rust 由生成器同步。

### 26. FIRE 的五个历史量写的是临时数组 `vecacc`

- **位置**：`main/MOD_Hist.F90:1937-1957`（`f_abm`/`f_gdp`/`f_peatf`/`f_hdm`/`f_lnfm`）。
- **原因**：五次都把 `vecacc` 传给 `write_history_variable_2d`，而不是 `a_abm` 等累加器；后者还会原地
  `vecacc = vecacc/nac`（非 `spval` 处）并把过滤掉的 patch 置 `spval`。
- **影响**：写出的是上一次用 `vecacc` 写历史后的残留、每个量再多除一次 `nac`：默认内核里上一次是 `f_wetzwt`
  （湿地过滤，BGC patch 恒为土壤 → 五个量全是缺测），CROP 内核里灌溉关时是 `f_grainc_to_cropprodc`、
  灌溉开时是 `f_runoff_supply`（`filter_irrig`）。
- **处理**：并入 PR #504（第 556 轮）：五个量改传 `a_abm` 等累加器，同时新增 104 个火诊断累加量（`a_farea_burned`…）。
  Rust 去掉复现 `vecacc` 残留的特例，全部走普通时间平均（`history.rs` 的 `FIRE_SOURCES`/`FIRE_DIAGNOSTICS`）。

### 27. 灌溉历史量：`f_sum_deficit_irrig` 恒缺测，三个"年累计"量被除以 `nac`

- **位置**：`main/MOD_Vars_1DAccFluxes.F90:2423-2432`、`main/MOD_Hist.F90:1507-1530`。
- **原因**：`accumulate_fluxes` 的灌溉段写 `a_sum_irrig = sum_irrig`、`a_sum_irrig_count = sum_irrig_count`、
  `a_waterstorage = waterstorage`（**赋值**，不是 `acc1d`），却漏了 `a_sum_deficit_irrig`；写历史时四个量照样按
  `write_history_variable_2d` 的 `/nac` 处理。
- **影响**：`f_sum_deficit_irrig` 永远是缺测；`f_sum_irrig`/`f_sum_irrig_count`/`f_waterstorage` 写出的是
  "窗口末步的年累计值 / 窗口步数"（日输出时是 /24），既不是累计也不是平均。
- **处理**：vendor 已修（第 555 轮）：补 `a_sum_deficit_irrig = sum_deficit_irrig`，四个量按瞬时量写（先乘 `nac`
  再交给除 `nac` 的写出器，即写末值）。Rust 的 `ASSIGNED_VARIABLES` 同样写末值。

### 28. 灌溉的逐 PFT 循环不适合多 PFT patch（潜在）

- **位置**：`main/MOD_Irrigation.F90:188-240`（`CalIrrigationPotentialNeeded`）、`:262-281`
  （`CalIrrigationApplicationFluxes`）、`:308-327`（`PointNeedsCheckForIrrig`）。
- **原因**：需水量的 PFT 循环不重置 `reached_max_depth` 与各总量，第二个 PFT 起只在第一个没碰到深度上限时再累加
  一遍；`deficit_irrig`/`check_for_irrig` 都取最后一个 PFT 的结果；施灌时每个 PFT 各减一次
  `n_irrig_steps_left`、各从 `waterstorage` 扣一次。
- **影响**：CROP 构建里作物 patch 只有一个 PFT，碰不到；若 patch 内有多个 PFT，灌溉量与持续步数都不对。
- **处理**：vendor 已修（第 557 轮）：土柱各总量只算一遍，patch 的灌溉方式取份额最大的 PFT（`dominant_irrig_pft`，
  并列取第一个），施灌每步只扣一次存水，`check_for_irrig` 改为"任一可灌作物 PFT 在窗口内"。作物 patch 只有一个 PFT，
  结果不变。Rust `irrigation.rs` 同步（`IrrigationState::dominant_pft`）。

### 29. `f_irrig_method_corn` 只写雨养玉米

- **位置**：`main/MOD_Hist.F90:2803-2826`。
- **原因**：玉米的过滤条件是 `pftclass == 17`（雨养温带玉米），其余七种作物都是"雨养 + 灌溉"两个类别
  （如春小麦 19/20）。
- **影响**：灌溉玉米（18）patch 上 `f_irrig_method_corn` 是缺测，而这正是唯一会灌溉的玉米。
- **处理**：vendor 已修（第 555 轮）：过滤条件改为 17 或 18；Rust `CROP_TYPE_HISTORY` 同步。

### 30. 单点 mksrfdata 正常结束时退出码为 1（`85cf2328` 引入）

- **位置**：`mksrfdata/MKSRFDATA.F90:150`（单点分支）、`share/MOD_SPMD_Task.F90:339`。
- **原因**：单点分支写完 `'Successful in surface data making.'` 后调 `CoLM_stop()` 结束；PR #17 把
  `CoLM_stop` 的非 MPI 实现从 `STOP` 改成 `STOP 1`，以区分出错退出。
- **影响**：每次成功的单点 mksrfdata 都以退出码 1 结束，调用方（`colm-cli`）按失败处理，后续阶段不跑。
- **处理**：本地把这一处改回 `STOP`，应当报给上游。

### 31. TOPMODEL 方法 0 把未赋值的 `topoweti`/`alp_twi`/`chi_twi`/`mu_twi` 写进常数重启

- **位置**：`mkinidata/MOD_Initialize.F90:513-564`。
- **原因**：`DEF_TOPMOD_method == 0` 只赋 `fsatmax`/`fsatdcf`，另外四个量只在方法 1、2 读文件；分配后没有初值。
- **影响**：写进常数重启的是未定义内存（单点纯 Fortran 实测为 0）。方法 0 下它们不参与计算，结果不受影响。
- **处理**：vendor 已修（第 555 轮）：`DEF_Runoff_SCHEME == 0` 时先把四个量置 0 再按方法赋值；Rust 写 0，与之一致。

### 32. `DEF_LC_RESPCP` 不起作用

- **位置**：`main/MOD_Const_LC.F90:902`（覆盖）、`main/MOD_AssimStomataConductance.F90:592`。
- **原因**：`stomata` 里的 `respcp` 是局部量，每次按 `0.015*c3 + 0.025*c4` 重算，地类表的 `respcp` 从未传进去。
- **影响**：设了 `DEF_LC_RESPCP` 也不改变任何结果。
- **处理**：vendor 已修（第 557 轮）：`calc_photo_params` 在单点 LCT、设了 `DEF_LC_RESPCP` 时用它，否则照旧按
  `0.015*c3 + 0.025*c4`（`DEF_LC_C3C4` 覆盖的语义不变）。Rust `LeafBiochemistry::respiration_fraction_override` 同步。
- **上游适用性**：`DEF_LC_RESPCP`/`LC_OVERRIDE_UNSET` 这套单点地类覆盖是本仓库加的，上游没有；上游分支不改。

### 33. 多作物单点每个 patch 的 `tlai`/`tsai` 是各作物之和

- **位置**：`main/MOD_LAIReadin.F90:171-175`（单点 PFT 段）、`mksrfdata/MOD_SingleSrfdata.F90:405-411`。
- **原因**：`tlai(:) = sum(SITE_LAI_pfts_monthly(:,time,iyear) * SITE_pctpfts)` 给所有 patch 赋同一个全站和；
  农田站点 `SITE_pctpfts = 1.`（不是 `pctcrop`），于是是各作物 LAI 直接相加。
- **影响**：只有一种作物时无害；多作物站点每个 patch 的 `tlai`/`tsai`（以及由它们折算的 `lai`/`sai`、辐射、
  冠层）都偏大，且所有 patch 相同，与各自的 `tlai_p` 不一致。`DEF_USE_LAIFEEDBACK` 下 `tlai` 不走这里，`tsai` 仍然。
- **处理**：vendor 已修（第 557 轮）：每个 patch 只对自己的 PFT 区间求 `sum(SITE_LAI_pfts_monthly(ps:pe)*SITE_pctpfts(ps:pe))`；
  自然站点只有一个 patch，与原来逐项相同。Rust `pft.rs` 同步。

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
- **处理**：vendor 已修（第 555 轮）：分配后 `filter_irrig(:) = .false.`；Rust 写 0，与之一致。

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
- **处理**：vendor 已修（第 557 轮）：`init_user_specified_forcing` 遇到 `groupby = 'day'` 直接停机并说明原因
  （不再静默读错记录）；Rust 同样拒绝。

### 42. 双线性强迫映射会取到区域块覆盖之外的格子

- **位置**：`share/MOD_SpatialMapping.F90:537-989`（`spatial_mapping_build_bilinear`），越界点在 `:753` 的 `gblock%pio(xblk,yblk)`。
- **原因**：
  - `grid_set_blocks`（`share/MOD_Grid.F90:496-679`）只给与 `DEF_domain` 相交的强迫行列分配块号，其余是 0；
  - 双线性按 patch 中心找两侧格心，中心落在最外一排格心与区域边界之间时，另一侧的格心在区域外，于是 `xblk/yblk = 0`。
- **证据**：g1f 底（区域 23–25°N、113–115°E，合成 CMFD 强迫）加 `DEF_Forcing_Interp_Method = 'bilinear'`。
  - 生产内核：在 "Building bilinear interpolation" 之后 SIGSEGV；
  - 带越界检查的 debug 内核：`At line 753 of file share/MOD_SpatialMapping.F90: Index '0' of dimension 2 of array 'gblock%pio' below lower bound of 1`。
- **影响**：几乎所有区域算例都会撞上。即使不越界，那些格子的强迫也不在读入的块里。只有全球区域才能跑。
- **处理**：vendor 已修（第 557 轮）：选出的邻行/邻列若不在块覆盖里（`yblk/xblk = 0`），该方向退化为只用覆盖内那一侧
  （权重 1/0）。全球区域每格都有块号，结果不变。Rust `build_bilinear` 接收 `DEF_domain`，按 `domain_window` 同样处理，
  入口不再拒绝 `bilinear`。

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
- **处理**：vendor 已修（第 557 轮）：`SAVE/REST_LulccTimeVariables` 把 `scale_baseflow` 与优化器的
  `zwt_init/rchg_year/rsub_year` 按 SAT 配对搬到新布局，新出现的 patch 取 `Opt_Baseflow_init` 的缺省（1、spval，
  `zwt_init` 在 `LulccDriver` 末尾取新年的 `zwt`）。另见第 54 条（优化器与 LULCC 的先后）。
  Rust：`lulcc_transition` 按 `match_patches` 重映射 `scale_baseflow`，优化器以 `BaseflowOptimizer::carried_over`
  跨段延续，不再拒绝 `DEF_Optimize_Baseflow` 与 LULCC 同开。续跑重读标定文件时仍按文件本身的布局（文件不带年份）。

### 44. MEC 城市段在两种情况下读失效的来源下标

- **位置**：`main/LULCC/MOD_Lulcc_MassEnergyConserve.F90:950-1096`。
- **情况一（旧单元里没有城市 patch）**：
  - `selfu_ = -1` 与 `gu_` 只在 `nurb > 0` 时设，`u_` 也只在 `selfu_ > 0` 或 `nurb > 0` 时赋值；
  - `nurb == 0` 时，两者都沿用**上一个城市 patch**（可能在别的单元）的值，`u.le.0 .or. u_.le.0` 也拦不住，于是新城市从别处的城市单元抄状态；
  - 若这是本 worker 的第一个城市 patch，读到的就是未初始化值。
  - 注释写的是"保留冷启动值"，代码没做到。
- **情况二（份额没变、旧单元里又没有同城市类型）**：`FROM_SOIL` 遍历 `frnp_(1:num)` 判断来源里有没有土壤 patch，可 `frnp_` 只在份额有变化的分支里赋值，这里读的是刚 `allocate` 的未定义内容。
- **影响**：只在城市布局逐年变化时出现。现有算例 g3um 两年城市布局相同，走不到。
- **处理**：vendor 已修（第 557 轮）：每个城市 patch 先复位 `selfu_ = u_ = -1`；旧单元没有城市 patch 时不抄（保留冷启动值）；
  `FROM_SOIL` 只在份额有变化（`frnp_` 已赋值）时判断。Rust `lulcc_mec.rs::urban_tail` 同步，不再拒绝。

### 45. MEC 把 `get_zwt_from_wa` 的毫米结果直接写进以米计的 `zwt`

- **位置**：`main/LULCC/MOD_Lulcc_MassEnergyConserve.F90:786-807`。
- **原因**：
  - `get_zwt_from_wa` 全程用毫米，`zmin = sp_zi(nl_soil)` 也是毫米，所以返回的水位埋深是毫米；
  - 正常步进（`MOD_SoilSnowHydrology.F90:1131-1141`）先把 `zwt` 换成 `zwtmm` 再调用，之后换回米；
  - MEC 却把结果直接写进 `zwt(np)`，`zwt` 的单位是米。
- **证据**：干旱区算例 g3a（38–40°N、100–102°E，MEC；初值把土壤 patch 设为 `wa = -300 mm`、`zwt = 4 m`）。未修内核换年后，44 个份额有变化的 patch 里 `zwt` 最大为 **5865.23**，修后为 **5.865 m**。
- **影响**：只在 MEC 混合后 `wa < 0`（水位落到土柱以下）时出现，即干旱、深水位区的 LULCC。水位被放到几公里深，此后的 VSF 与含水层交换都错。
- **处理**：vendor 已修，结果先存进局部 `zwt_mm`，再 `zwt(np) = zwt_mm / 1000.0`（第 535 轮）。Rust 按修后的写法移植。

### 46. `snow_ini` 的界面深度递推符号写反，初始雪层节点深度不单调

- **位置**：`mkinidata/MOD_IniTimeVariable.F90:1371-1375`（`snow_ini`）。
- **原因**：自上而下的递推写成 `zi = -zi-dz_soisno(i)`，应为 `zi = zi-dz_soisno(i)`。
  - 第一层（`i = 0`）不受影响，第二层也碰巧对；从第三层起符号翻转。
  - 例如雪深 0.333 m（4 层）时，`z_sno = [0, -0.103, 0.018, -0.208, -0.0765]`：出现正的节点深度，且不单调；正确值是 `[0, -0.323, -0.288, -0.208, -0.0765]`。
- **影响**：
  - 凡 `DEF_USE_SnowInit` 给出的初始雪深 > 0.07 m（3 层及以上）的冷启动都受影响。
  - 前几步雪层热传导用的是错误的节点间距（Fortran 不检查，照样跑）；直到雪层合并/细分重算 `z` 才恢复。
  - Rust 的 `ground_temperature` 要求节点深度递增，会直接拒绝（"ground-temperature node depths must increase"）。
- **证据**：blsn0（AT-Neu，BGC/PFT，1 月雪深 0.333 m）。未修时：Fortran 跑完，Rust 首步拒绝；冷启动重启两侧逐位相同，说明 Rust 原本照抄了这个 bug。
- **处理**：vendor 已修（第 537 轮），Rust 的 `initialize_snow_layers` 按修后的写，单测补了 5 层的节点深度。

### 47. `forc_rain/forc_snow` 分配后未初始化，被强迫缺测遮蔽的 patch 把垃圾值累加进历史

- **位置**：`main/MOD_Vars_1DForcing.F90:79-80` 分配；`main/MOD_Vars_1DAccFluxes.F90:2196-2197` 每步 `acc1d(forc_rain, a_rain)`、`acc1d(forc_snow, a_snow)`。
- **原因**：
  - 这两个数组只由 `CoLMMAIN`/`CoLMMAIN_Urban` 按 patch 写；
  - 被遮蔽 patch（`forcmask_pch = .false.`）在 `CoLMDRIVER` 里整步跳过，从不写它们；
  - 而分配后又没有赋初值。
- **证据**：g1urbmm（城市模型，`CMFDmb` 遮蔽 48 个 patch，月历史，5 天 240 步）。Fortran 旁车里被遮蔽 patch 的 `a_snow/nac` 在 2.05–3.45 mm/s 之间，每个 patch 都不同；同时 `a_rain` 全为 0，1 月华南也不可能有这么大的降雪。
  - 此前的算例（g1fmm、g1bgcm 等）里碰巧是 0，所以第 523 轮记成了"`rain/snow = 0`"。
- **影响**：
  - 历史文件不受影响，因为各 `filter` 都与上了 `forcmask_pch`；
  - 但月历史续跑的旁车会带进随机值。分配到哪块内存决定了值是多少，结果不可复现。
- **处理**：vendor 已修，分配后 `forc_rain(:) = 0`、`forc_snow(:) = 0`（第 543 轮）。这与 Rust 对被遮蔽 patch 一直交 0 的做法一致。三套内核（default、latlon、latlon-crop）已重编。

### 48. 区域单元流域在单进程 MPI 下死锁

- **位置**：`mksrfdata/MOD_UnitCatchmentRegional.F90:66-90`（`unitcatchment_regional_build` 的 `USEMPI` 段）。
- **原因**：
  - master 逐个 `mpi_recv` 各 worker 发来的 `remap%ids_me`；worker 的发送写在 `ELSEIF (p_is_worker)` 里。
  - 扁平 SPMD 只有一个 rank 时，它既是 master 又是 worker：只走 master 分支，向自己收一条永远不会发出的消息。
- **证据**：g1reg（g1all + `DEF_UnitCatchment_regional`，`prterun -n 1`，colm-cli 跑 Fortran 的默认方式）。
  - mksrfdata 跑到 35 分钟 CPU 仍不结束，`sample` 显示停在 `unitcatchment_regional_build → ompi_recv_f → PMPI_Recv`；
  - 修后整条纯 Fortran 链路 85 秒跑完。
- **影响**：单进程 MPI 下开区域单元流域时，mksrfdata 永远不结束。多 rank、master 不兼 worker 的布局不受影响。
- **处理**：vendor 已修（第 547 轮）。master 遇到自己那个 worker 时直接读本地 `remap%ids_me`，其余照旧收发。
- **上游适用性**：上游的 `spmd_init` 不会让一个 rank 同时当 master 与 worker，只有本仓库的扁平 SPMD（FLAT_SPMD）会；上游分支不改。

### 49. 单点降尺度写 srfdata 时用了未定义的维度名 `type`

- **位置**：`mksrfdata/MOD_SingleSrfdata.F90:3227/3231/3235`（`write_surface_data_single`）与 `:3528/3532/3536`（城市版）。
- **原因**：`:2950` 与 `:3271` 定义的维度叫 `slope_type`，写 `SITE_slp_type/asp_type/area_type` 时却传 `'type'`。
- **证据**：单点 pds（站点挪到黑河 101°E、38°N，`DEF_USE_Forcing_Downscaling`，地形因子取 `topo_factor/heihe`）。
  mksrfdata 把地形因子全部算完、打印出来之后报 `Netcdf error: NetCDF: Invalid dimension ID or name`，`STOP 1`。
- **影响**：单点开完整降尺度时 mksrfdata 必然失败（LCT/PFT 与城市单点都是）。
- **处理**：vendor 已修（第 549 轮）：六处都改成 `'slope_type'`，与 Rust 写出的维度名一致。`kernels/default` 已重编。

### 50. LULCC 下多遍预热回卷不回退土地覆盖

- **位置**：`main/CoLM.F90:711-724`（预热回卷）与 `:579-610`（年末 `LulccDriver`），`main/MOD_LAIReadin.F90`。
- **原因**：回卷只重置时钟与强迫（`forcing_reset`），土地覆盖、patch 布局与状态停留在新一年。之后：
  - `LAI_readin (jdate(1), …)` 按旧年份读 `landdata/LAI/<旧年>/`，却用新一年的 `landpatch` 去取向量；
  - 再到旧年年末又调一次 `LulccDriver`，把已经是新年布局的状态当成旧年布局去做 SAT 转换。
- **证据**：g3sc（g3 区域，2005-12-30 → 2006-01-01，预热到 2006-01-01、`spinup_repeat = 2`）。纯 Fortran 跑完不报错，日志里 `LULCC: initializing` 出现 2 次；2005 年 175 个 patch、2006 年 172 个。
- **影响**：只要多遍预热的区间跨过 LULCC 年末，第二遍起的结果就没有意义（不报错）。单遍预热、或预热在第一个年末之前结束的情形不受影响。
- **处理**：vendor 已修（第 557 轮先改成停机；第 564 轮改为真正回退）：预热回卷时若本轮里做过 LULCC，就按年末 LULCC
  同样的流程（`deallocate`/`hist_final` → `LulccDriver` → `grid_riverlake_flow_lulcc` → `forcing_init`/`hist_init`）把土地覆盖
  换回起始年，`jdate` 取运行起点，所以 `LulccInitialize` 读的是起始年、起始月的 LAI。mksrfdata 只写"上一年 → 本年"的转移矩阵，
  没有从后一年回到起始年的，所以这次换年一律用 SAT（`LulccDriver` 新加可选参数 `rewind`）。不开 LULCC、或预热不跨 LULCC 年末时
  行为不变。Rust 把每一轮拆成一条段链（前几轮跑到预热终点后换回起始年，最后一轮跑到运行终点），回卷的冷启动写进临时目录、不盖初值；
  预热终点恰在 LULCC 年末时同一步先换到新一年再换回，基流优化器的配对按两次复合。

### 51. LULCC 转移轨迹按像元数接收 zip 后的样本

- **位置**：`mksrfdata/MOD_Lulcc_TransferTrace.F90:194-204`。
- **原因**：`aggregation_request_data (…, zip = .true., …)` 返回的是去重后的 500 m 源格（同一源格的像元面积相加），
  长度是源格数；随后 `lcfrbuff(ipxstt:ipxend) = lcdatafr_one(:)`、`areabuff(ipxstt:ipxend) = area_one(:)` 按 patch 的**像元数**接收。
- **影响**：像元比 500 m 格细时（例如开了完整降尺度、地形因子网格并入像元，或网格边界不在 500 m 格线上），源格数少于像元数，赋值越界读，
  `lccpct_patches` 是垃圾值（不报错）。像元与 500 m 格一一对应时只是样本次序变成（列、行）升序，结果有定义。
- **处理**：vendor 已修（第 557 轮）：直接遍历 zip 后的样本（`size(area_one)`），不再拷进按像元数分配的缓冲。
  像元与源格一一对应时元素与次序都不变。Rust 本来就按 zip 样本算，去掉拒绝。

### 52. `UrbanTHERMAL` 合并阳面/阴面墙内温时用了阳面自己的温度

- **位置**：`main/URBAN/MOD_Urban_Thermal.F90:608`。
- **原因**：`fwsun` 变化后按面积重分配内墙温，写成 `twsun_inner = (fwsun*twsun_inner + dfwsun*twsun_inner)/(fwsun+dfwsun)`，
  第二项应为阴面 `twsha_inner`（阴面转为阳面的那部分带来的是阴面的温度）。式子因此（除舍入外）等于原值，重分配不起作用。
- **影响**：城市模型 `fwsun` 变化（太阳高度角变化）的每一步，阳面内墙温度都少了阴面那部分的混合。
- **处理**：vendor 已修（第 554 轮），Rust `urban_thermal.rs` 同步。

### 53. `GRATIO` 在 `x < 0` 时只写 `ANS = 2`，TOPMODEL 方法 2 读到未定义的 `qgr`

- **位置**：`share/MOD_IncompleteGamma.F90:400-403`（出错返回）；调用方 `main/MOD_Runoff.F90:107-126`、
  PR #504 的 `main/BGC/MOD_BGC_Veg_CNFireLi2016.F90`（饱和面积比）。
- **原因**：`x = (eta - mu_twi)/chi_twi`，初值 `eta = topoweti` 小于 `mu_twi` 时 `x < 0`，`GRATIO` 走出错返回、不写 `QANS`，
  调用方接着把未赋值（或上一次调用残留）的 `qgr` 当饱和面积比；牛顿迭代里 `pgr0` 也无定义。
- **影响**：`DEF_TOPMOD_method = 2` 下平均地形指数低于伽马分布下界的 patch，饱和面积比与地表产流不确定。
- **处理**：vendor 已修（第 556 轮）：三处调用都传 `max(0, x)`（分布下界以下即全饱和，`x = 0` 时 `Q = 1`）；`MOD_Runoff` 的迭代
  补 `IF (pgr0 <= 0.) EXIT` 避免除零。Rust `incomplete_gamma.rs` 逐位移植了 `GRATIO`（6464 组与 gfortran 逐位一致），
  调用用保留 `qans` 语义的 `gratio_fortran`。

### 54. 年末先做 LULCC 再结算基流优化：最后一步丢失，结算落在新布局上

- **位置**：`main/CoLM.F90`：`LulccDriver`（年末）在前，`ParameterOptimization` 在约 120 行之后。
- **原因**：LULCC 释放并重新分配通量数组（`fevpa/rsur/rsub = spval`），随后的 `BaseFlow_Optimize` 在这一步的累加被
  `spval` 掩掉；年末结算用的 `zwt`/`zwt_init` 已经是新布局（而且第 43 条的数组还没重映射）。
- **影响**：`DEF_Optimize_Baseflow` 与 LULCC 同开时，每年最后一步的补给与基流不计入，结算错位。
- **处理**：vendor 已修（第 557 轮）：把 `ParameterOptimization` 挪到 LULCC 之前（两处之间只有 LAI 读入与写续跑，
  不碰它的输入，所以不开 LULCC 时结果不变）。Rust 的优化器本来就在旧段最后一步结算。

### 55. PR #504 新增的 `prec30`/`rh30_today` 冷启动是 `spval`

- **位置**：`main/BGC/MOD_BGC_Vars_TimeVariables.F90`（分配为 `spval`）、`mkinidata/MOD_IniTimeVariable.F90`（只初始化
  `prec10/prec60/prec365/prec_today/rh30` 等）。
- **原因**：两个新量没有加进 `IniTimeVariable` 的参数表，冷启动重启里是 `spval`，与同类滑动平均（置 0）不一致。
  第一步 `nsteps = 1` 时 `spval*0` 恰好不影响结果。
- **处理**：vendor 已修（第 556 轮）：加进参数表并置 0；Rust 冷启动写出器同步写这两个量。

### 56. WATER_2014 与两处漫滩再入渗不传 TWI 统计量：方法 2 解引用缺省的可选参数

- **位置**：`main/MOD_SoilSnowHydrology.F90`：`WATER_2014` 调 `SurfaceRunoff_TOPMOD`（约 `:334`）、`WATER_2014` 与
  `WATER_VSF` 的漫滩再入渗（约 `:404`、`:1082`）。
- **原因**：`SurfaceRunoff_TOPMOD` 的 `topoweti/alp_twi/chi_twi/mu_twi` 是可选参数，方法 2 无条件读它们；这三处调用都不传。
  GIMPLE 里就是直接读 `*mu_twi`，即空指针。
- **影响**：`DEF_Runoff_SCHEME = 0`、`DEF_TOPMOD_method = 2` 时，经典 Richards（WATER_2014）或开了河湖漫滩回馈就崩溃；
  城市透水地面（也走 WATER_2014）同样。
- **处理**：vendor 已修（第 559 轮）：WATER_2014 新增可选参数 `topoweti/alp_twi/chi_twi/mu_twi`（CoLMMAIN 与城市水文按关键字传入），
  三处调用都带上。Rust 同步（TOPMODEL 方法 1/2 原来只接了 VSF，见第 559 轮）。

### 57. WATER_2014 的地下径流不传 `hksati/topoweti`：方法 1 静默退化为方法 0

- **位置**：`main/MOD_SoilSnowHydrology.F90` 的 `groundwater` 调 `SubsurfaceRunoff_TOPMOD`（约 `:2563`）。
- **原因**：方法 1/2 的分支要求 `present(hksati) .and. present(topoweti)`（或 `eta`），这里都不传，于是落进方法 0 的式子。
  地表饱和面积在方法 0/1 本来同式，所以 WATER_2014 下方法 1 与方法 0 完全相同，用户不会察觉。
- **处理**：vendor 已修（第 559 轮）：`groundwater` 新增可选 `hksati/topoweti/eta` 并转传；WATER_2014 传入地表那次带出的 `eta`。Rust 同步。

### 58. TOPMODEL 方法 2 的牛顿初值 `eta = topoweti` 不高于 `mu_twi` 时直接判为全饱和

- **位置**：`main/MOD_Runoff.F90` 方法 2；PR #504 的 FIRE 饱和面积比照抄了同一段。
- **原因**：`x = (eta - mu_twi)/chi_twi` 在初值处 ≤ 0 时（截到 0 后）`pgr0 = 0`，迭代立即退出，`fsat = Q(alp, 0) = 1`，与水位无关。
  数据自洽时 `topoweti` 是分布均值 `mu + alp·chi`，不会出现；插值或缺测填充造出的 patch 会。
- **处理**：vendor 已修（第 559 轮）：`topoweti ≤ mu_twi` 时初值取 `mu_twi + alp_twi·chi_twi`（分布均值），Runoff 与 FIRE 两处一致。Rust 同步。

### 59. TOPMODEL 方法 2 不初始化 `fsatmax/fsatdcf`

- **位置**：`mkinidata/MOD_Initialize.F90` 方法 0/1/2 分支。
- **原因**：方法 2 只读四个 TWI 量，`fsatmax/fsatdcf` 分配后不赋值就写进常数重启；漫滩再入渗那次调用（修第 56 条之前）会读 `fsatdcf`。
- **处理**：vendor 已修（第 559 轮）：所有方法先赋方法 0 的值（0.38、0.125），方法 1 再读文件覆盖；Rust 的冷启动本来就这样写。

### 60. WATER_2014 在 `gwat <= 0` 的步里不算 `eta`，方法 2 的基流按 `exp(0)` 爆大

- **位置**：`main/MOD_SoilSnowHydrology.F90` WATER_2014 的 `IF (gwat > 0.) CALL SurfaceRunoff_TOPMOD` 与随后的 `groundwater`。
- **原因**：方法 2 的地下径流是 `imped*3e3*mean(hksati)/DECAY*exp(-eta)`，`eta` 由地表那次调用带出；`gwat <= 0`（无雨、蒸发大于
  入渗）时那次调用被跳过，`eta` 停在初值 0。修第 56/57 条之前方法 2 在 WATER_2014 下直接崩溃，修后才暴露。
- **影响**：方法 2 + WATER_2014 下每个干步的基流比正常大三四个量级，土柱被抽干。
- **处理**：vendor 已修（第 559 轮）：每步都调 `SurfaceRunoff_TOPMOD` 求 `eta`，`gwat` 不大于 0 时仍把 `rsur` 置 0（方法 0/1 结果逐位不变，
  包括 `-0.0` 的边角）。Rust 同步。

### 61. `CNFireArea` 对整个 `tsoi17` 数组赋值

- **位置**：`main/BGC/MOD_BGC_Veg_CNFireLi2016.F90`（PR #504 之前就有）：`tsoi17 = forc_t(i)`。
- **原因**：`tsoi17` 是逐 patch 的状态（进重启），这里漏了下标，每处理一个 patch 就把**所有** patch 的 `tsoi17` 改成这一个 patch 的气温。
- **影响**：多 patch（空间）算例里，非最后处理的 patch 的 `tsoi17` 与重启值都是别处的气温；本步泥炭火用的是自己刚写的值，不受影响。单点无影响。
- **处理**：vendor 已修（第 560 轮）：`tsoi17(i) = forc_t(i)`。Rust 原来在每步末用 `broadcast_fire_tsoi17` 复现整列赋值（第 501 轮加入），
  现已删掉，各 patch 只写自己的。

### 62. `CNFireFluxes` 拿弧度的 `patchlatr` 比以度计的 `borealat`

- **位置**：`main/BGC/MOD_BGC_Veg_CNFireBase.F90`：`IF (patchlatr(i) < borealat)`；`CNFireArea` 用的是以度计的 `dlat`。
- **原因**：`patchlatr` 是弧度（|值| ≤ π/2），与度比较恒为真，寒带分支永远走不到。
- **处理**：vendor 已修（第 560 轮）：换算成度再比。Rust 由生成器同步。

### 63. PR #504 的火参数被旧行覆盖；`troplat` 是错误的"换算"

- **位置**：`mkinidata/MOD_Initialize.F90` 火参数段。
- **原因**：
  - PR #504 在段首新加 `occur_hi_gdp_tree = 0.33`、`borealat = 60`，却没删 PR 之前的 `occur_hi_gdp_tree = 0.39`、
    `borealat = 40/(4*atan(1))`，后者排在后面，把新值覆盖了（`CNFireArea` 里另有局部 `parameter occur_hi_gdp_tree = 0.33`
    遮住了重启里的 0.39，两处不一致）；`non_boreal_peatfire_c`/`boreal_peatfire_c` 也各赋了两次（值相同）。
  - `borealat = 40/(4*atan(1))`（≈ 12.7）与 `troplat = 23.5/(4*atan(1))`（≈ 7.5）看起来想把度换成弧度却除错了，
    而使用处都与以度计的 `dlat` 比较：寒带泥炭火的分界落在 12.7°N，热带落叶树的热带判据只剩 ±7.5°。
- **处理**：vendor 已修（第 560 轮）：删掉重复的旧行，`borealat = 60`（度，按 PR 本意）、`troplat = 23.5`（度），
  `occur_hi_gdp_tree = 0.33` 与局部常数一致。Rust 常数表同步。

### 64. TOPMODEL 湿度指数偏度的 32 位整数溢出；只在部分方案下赋值的常数写进重启

- **位置**：`mksrfdata/Aggregation_TopoWetness.F90`（patch 与单元两处）、`main/MOD_Vars_TimeInvariants.F90`。
- **原因**：偏度的分母 `(npxl-1)*(npxl-2)` 是整数乘法，大 patch（子像元数过 46341）回绕，`alp/chi/mu_twi` 被夹到界上；
  `BVIC`、三参数伽马的 TWI 量、`fsat*`、VIC 参数只在部分产流方案下赋值，却总写进常数重启。
- **处理**：vendor 已修（第 466 轮）：分母改为 `real(npxl-1)*real(npxl-2)`；这些量分配时清零。

### 65. URBAN 内核、不开城市时 TOPMODEL 湿度指数聚合用到未建的 `elm_patch`

- **位置**：`mksrfdata/Aggregation_TopoWetness.F90`；`landpatch_build` 在 URBAN_MODEL 内核里把 `elm_patch` 留给 `landurban_build`。
- **原因**：`DEF_Runoff_SCHEME = 0`、URBAN 内核、不开城市（也无 CROP、2 m WMO）时没人建 `elm_patch`，聚合段错误。
- **处理**：vendor 已修（第 465 轮）：聚合前若未建就自己建（不改 `patchfrac_elm` 的写出条件）。
- **上游适用性**：复核后**只在本仓库出现**——上游有 `URBAN_MODEL` 宏时一定调 `landurban_build`（它会建 `elm_patch`），是本仓库把城市改成
  运行时开关 `DEF_URBAN_RUN` 后才可能跳过。第 465 轮记的"上游同样如此"有误；上游分支不改。

### 66. LULCC 重新初始化时水库表重复分配、堤防蓄水被清零

- **位置**：`mkinidata/MOD_Initialize.F90` 的 LULCC 再初始化路径（`reservoir_init`、`levee_init`）。
- **原因**：水库数组没释放就再 `allocate`，开水库并跨过换年即崩溃；`levee_init` 把 `levsto/levdph` 置 0，堤内水每次换年凭空消失。
- **处理**：vendor 已修（第 475 轮）：先释放水库表再初始化，堤防状态跨换年保留。

### 67. LULCC 重新初始化把河网整张冷启动

- **位置**：`main/LULCC/MOD_Lulcc_Initialize.F90`、`main/HYDRO/MOD_Grid_RiverLakeTimeVars.F90`。
- **原因**：`LulccInitialize` 释放并重分配全部时间变量（含河道状态）再调 `initialize`：先是重建河网时重复分配崩溃，绕过后每年把河网冷启动成
  `topo_rivhgt`，河道水量不守恒。单元流域不随土地覆盖变，状态本应保留（`grid_riverlake_flow_lulcc` 只补 `volwater_ucat` 也说明这一点）。
- **处理**：vendor 已修（第 459 轮）：重建网络前先释放，`move_alloc` 把河道状态挪开、初始化后挪回。

### 68. LCT 调 `LeafTemperature` 时 `ivt` 写死为 1：臭氧胁迫按温带常绿针叶林算

- **位置**：`main/MOD_Thermal.F90:718` 的 `CALL LeafTemperature(ipatch,1,...)`（PFT/PC 模式下的非土壤 patch 也走这一支）。
- **原因**：LCT 没有 PFT 类别，调用处直接传字面量 1。
- **影响**：`DEF_USE_OZONESTRESS` 下任何 IGBP/USGS 地类都按温带常绿针叶林算臭氧（常绿、`lai_thresh = 0`、`leaf_long(1)` 衰减、
  通量阈值 0.8 nmol m⁻² s⁻¹、针叶林的 `o3coefv/o3coefg` 公式）；草地、农田、落叶林因此按错误的类别计算。
- **处理**：vendor 已修（第 564 轮）：`MOD_Ozone` 新加 `ozone_pft_of_lct(patchclass)`，`MOD_Thermal` 的 LCT 调用改传它。
  按生活型与叶习性取最接近的 PFT（IGBP：ENF→1、EBF→4、DNF→3、DBF→7、混交林→1（保持原值）、郁闭灌丛→9、稀疏灌丛→10、
  木本稀树草原→7、稀树草原→14、草地/湿地/城市→13、农田与镶嵌→15；USGS 同理，苔原草本→12、木本苔原→11），
  无植被的地类（海洋、冰雪、裸地、水体）取 0，`CalcOzoneStress` 对 0 不施加胁迫（`o3coefv = o3coefg = 1`）。
  LCT 分支里 `ivt` 只被臭氧用（截留容量在 LCT 下取 `patchclass`），所以不开 `DEF_USE_OZONESTRESS` 时结果不变。
  这张表是一个科学取舍，上游可以按需要调整。Rust `lct_ozone_vegetation_type` 同表。

### 69. 冷启动漏赋臭氧状态

- **位置**：`mkinidata/MOD_IniTimeVariable.F90`：patch 级 `o3uptakesun/sha` 从不赋值；PFT 级 `o3uptake*_p`/`o3coef*_p` 只在 `IF (DEF_USE_BGC)` 段里赋值。
- **影响**：重启里写 `spval`；非 BGC 的 PFT/PC 算例里裸地 PFT 从不调 `LeafTemperature`，`o3uptakesun_p` 一直是 `spval`，
  patch 聚合后每条 `f_o3uptakesun/sha` 都约为 −2×10³⁵。
- **处理**：vendor 已修（第 561 轮）：`IniTimeVar` 循环之后对所有配置赋 `o3uptakesun/sha = 0`、`o3uptake*_p = 0`、`o3coef*_p = 1`。Rust 同步。

### 70. `forc_ozone` 未初始化就被 `a_ozone` 累加

- **位置**：`main/MOD_Vars_1DForcing.F90`（分配）；`main/MOD_Vars_1DAccFluxes.F90` 的 `acc1d(forc_ozone, a_ozone)`。
- **原因**：不用臭氧数据时 `forc_ozone` 只在该 patch 第一次调 `CalcOzoneStress` 时被写成 100，`acc1d` 却每步对所有 patch 累加。
- **影响**：冰川、湖、无冠层 patch 的 `a_ozone`/`f_xy_ozone` 与旁车是未初始化内存。
- **处理**：vendor 已修（第 561 轮）：分配后置 `spval`（`acc1d` 跳过）。Rust 原本就从 `spval` 起步。

### 71. `ivt` 不在 1..15 时 `CalcOzoneStress` 不给 `o3coefv/o3coefg` 赋值

- **位置**：`main/MOD_Ozone.F90` 的 `CalcOzoneStress`（两个 `intent(out)` 哑元）。
- **影响**：裸地等类别下返回值未定义（GIMPLE 在调用点放了 `CLOBBER`）。
- **处理**：vendor 已修（第 561 轮）：开头先置 1。Rust 同步。

### 72. 臭氧数据读晚一档、`itime` 可为 0、`f_xy_ozone` 单位标错

- **位置**：`main/MOD_Ozone.F90` 的 `init_ozone_data`/`update_ozone_data`；`main/MOD_Hist.F90` 的 `f_xy_ozone`。
- **原因**：启动读 `(sec-1800)/10800+…` 档，更新时读 `(sec-deltim)/10800+…` 档，而数据的 `time` 是窗口中点（第 1 档对应 00:00–03:00）。
- **影响**：所用臭氧浓度比所在窗口晚约一档；`deltim = 10800` 且年初 0 时起步时 `itime = 0`，读文件越界；单位标成 `mol/mol`，实际是 ppbv。
- **处理**：vendor 已修（第 561 轮）：新加 `ozone_record(year, day, sec)` 取步首所在窗口（恒 ≥ 1），与内存里那一档不同时才读；单位改 `ppbv`。Rust 同步。

### 73. USGS 下 `CROPLAND = 7`，而 GLCC 第 7 类是草地（潜在）

- **位置**：`main/MOD_Vars_Global.F90:27`（`#ifdef LULC_USGS` 段）；地类图例见 `main/MOD_Const_LC.F90:29-53`。
- **原因**：GLCC USGS 图例里 2–6 是农田与镶嵌（2 旱地农田与牧场），7 是草地；常数取成了 7。
- **影响**：`CROPLAND` 的使用处全在 PFT/PC/CROP 路径（`MOD_SingleSrfdata`、`MOD_LandPFT`、`MOD_LandCrop`、`Aggregation_*`、
  `MOD_Albedo_HiRes` 的 PC 分支、LULCC MEC 与示踪物的 `DEF_FAST_PC` 合并），而这些路径上游只支持 IGBP，所以合法的 USGS
  配置走不到它。一旦以后让 USGS 支持 PFT/PC/CROP，草地会被当成农田。
- **处理**：vendor 已修（第 564 轮）：改为 2，并在旁边注明。IGBP 构建不受影响；Rust 只有 IGBP 的 12。

### 74. van Genuchten 的 `alpha` 按 1/cm 读入却按 1/mm 使用（CoLM-SYSU/CoLM#507）

- **位置**：`mkinidata/MOD_SoilParametersReadin.F90`（`alpha_vgm = soil_alpha_vgm_l`）；`main/MOD_SoilSurfaceResistance.F90`
  （LP92 的 `wfc` 用 `alpha_vgm*339.9`）。
- **原因**：landdata（与单点 SITE 文件）里的 `alpha_vgm` 单位是 1/cm，而模式里吸力 `smp`/`psi0` 都是 mm，所有
  `soil_psi_from_vliq`/`soil_vliq_from_psi`/`sc_vgm` 等都把 `alpha` 当 1/mm 用；`wfc` 那处用 339.9（cm）与 1/cm 的 `alpha`
  相乘，换算后要配 3399（mm）。
- **影响**：默认的 van Genuchten 土壤方案下，持水曲线与导水率的吸力尺度差了 10 倍，土壤水分、蒸发与径流全部受影响。
  Campbell 方案不受影响。
- **处理**：按上游 PR #507 并入（第 564 轮）：读入时 `* 0.1`，`wfc` 常数改 3399；mkinidata 里由原始 `alpha` 配 339.9 cm 算的
  `wfc` 本来就对，不动。PR 同时加了 namelist 开关 `DEF_HIST_grid_as_model_mesh`（非 GRIDBASED 时自动关），目前没有代码读它，
  照样并入（schema 重新生成）。Rust：`derive_soil_parameters` 存 `alpha*0.1`、`sc_vgm/fc_vgm` 用换算后的值，
  `soil_surface_resistance` 的常数改 3399。
- **上游**：已推到 `zhongwangwei/CoLM202X` 的 `fix/colm-desktop-audit`（`7d8a4b4b`，PR #24）。
- **PR 的后续提交（第 657 轮并入）**：PR 在 10-06 又追加了三个提交（`1e7f8e079`、`9a86e4a34`、`945f68e15`，PR 仍未合并）：
  - `use_explicit_form` 的"自上而下削减出流"改为带回溯的 `DO WHILE`：某层被抽干（`wa_m1 + dwat < -tol_z`）时，若上界面通量向上
    且上一层没被抽干过，先把上界面的向上通量削到 `min(q(i) - wa_m1/dt, 0)`，退回上一层重查；否则削减下界面出流（并继承
    上一层的"抽干"标记）。最上层抽干时，降雨边界先用积水补，补不够才把顶界面截在 `dp_m1/dt + ubc_val`。旧写法一律从更深处
    补水。固定通量底边界那一段的比较由 `dwat <= -wa_m1` 改为 `wa_m1 < -dwat`（只差相等时）。VSF 默认开启，三份黄金分别
    从第 7、111、11 步起变化。
  - `SurfaceRunoff_TOPMOD`：方法 2 缺任一 TWI 可选参数时退回方法 0/1 的指数式，`eta` 先置 `spval`。vendor 的四处调用都传齐，
    Rust 的 `TopmodelMethod::Gamma` 在类型上就带着四个量，两侧都走不到，照样并入以与上游一致。
  - `Aggregation_TopoWetness` 的偏度分母改实数乘法、`initialize` 对所有方法先设方法 0 的 `fsatmax/fsatdcf`：vendor 早已按同样语义修过
    （第 56–60 条），不用再动。
  - Rust：`apply_variable_saturated_explicit_step` 同样改为回溯循环；`wa_m1 + dwat` 与降雨分支的 `dp_m1 + (ubc-q)*dt` 按 FMA 收缩写
    （arm64 上三份黄金逐位一致，证明与 gfortran 的收缩方式相同）。

### 75. `LeafTemperaturePC` 在无植被斑块上提前返回，intent(out) 输出全部未赋值

- **位置**：`main/MOD_LeafTemperaturePC.F90:545-547`（`IF (.not. is_vegetated_patch) RETURN`）；调用方 `main/MOD_Thermal.F90`
  的 PC 分支（`DEF_USE_PC` 且 `DEF_FAST_PC=.false.`）。
- **原因**：PC 斑块的每个 PFT 都 `fcover==0` 或 `lai+sai<=1e-6`（冬季落叶、雪埋）时，子程序只把 `tl` 设成气温就返回，
  `z0mpc/rst/assim/respc/fsenl/fevpl/etr/hprl/dheatl` 这些 intent(out) 一个都不赋，按标准是未定义值；可选的
  `raw_trc_out` 也不赋。
- **影响**：gfortran 实际不碰这些数组，调用方拿到的恰好是 `THERMAL` 在调用前放进去的初值（`rst_p=2e4`、其余 0、
  `z0m_p` 为地面粗糙度），所以现有结果没错，但换编译器或优化级别就可能变。全局非 fastPC 算例（g1pcs）每步都有这种斑块。
- **处理**（第 569 轮）：vendor 在 `RETURN` 前显式赋这些值（Fortran 结果逐位不变）。Rust 原先在这里拒绝运行，
  现在 `leaf_temperature_pc` 返回 `None`（只落地 `tl = forc_t`，臭氧系数重置在判断之后，不触发），`pft.rs` 的
  `pc_unvegetated_record` 按 Fortran 调用方的原值组装记录：patch 级湍流量取前置 `GroundFluxes`，`zol/rib/ustar/qstar/tstar`
  与 `raw` 取 `THERMAL` 开头的 0，`z0m = sum(z0m_p*pftfrac)`。
- **上游**：已推（`7d8a4b4b`）。上游的 `raw_trc_out` 只在 `#ifdef TRACER` 下声明，那一行赋值也包在里面。

### 76. 流域网格内流区水库的 `volresv/qresv_in/qresv_out` 未赋值就进时间平均

- **位置**：`main/HYDRO/MOD_Catch_Reservoir.F90:reservoir_init`（`allocate` 后不赋初值）；
  `main/HYDRO/MOD_Catch_RiverLakeFlow.F90:342`（调度只对 `riverdown /= -1` 的水库做）与 `:554-562`（时间平均对所有
  已建成的 `lake_type == 2` 流域做）。
- **原因**：下游为 -1（内流区）的水库从不进调度分支，`volresv/qresv_in/qresv_out` 一直是 `allocate` 出来的未定义值，
  却每个子步按 `dt` 累进 `*_ta`，写进 basin history。
- **影响**：珠江 250 km² 网格有一个这样的水库（hylak 1386330，`build_year = -99`）。macOS/gfortran 上新分配的内存恰好是 0，
  输出为 0；换平台或内存复用时可能是任意值。
- **处理**（第 579 轮）：vendor 在 `allocate` 时置 0（现有输出逐位不变）；Rust 的 `ReservoirFlow::new` 同样从 0 起。
- **上游**：已推（`7d8a4b4b`）。

### 77. 甲烷 `hybrid` 淹没方案不检查河湖汇流运行时开关

- **位置**：`main/TRACER/MOD_Tracer_Reactive_Methane_Const.F90:configure_methane_inundation_mode` 的 `CASE ('hybrid', …)`。
- **原因**：`routing` 分支在 GridRiverLakeFlow 内核里还会检查 `DEF_USE_GridRiverLakeFlow`，关掉就停机；`hybrid` 只用
  `#ifndef GridRiverLakeFlow` 检查内核，不查运行时开关。`hybrid` 照样打开 `use_routing_for_soil`，土壤柱去取河网洪泛比例，
  可河湖汇流关着时这个比例从不产生。
- **影响**：GridRiverLakeFlow 内核里关掉河湖汇流、甲烷用缺省的 `hybrid` 时，不报错，土壤淹水分量悄悄按零洪泛计算。
  另外，上游缺省 `hybrid` 要求动态湿地，而 `DEF_USE_Dynamic_Wetland` 缺省为关，所以只用缺省值时上游会直接停机。
- **处理**（第 588 轮）：vendor 的 `hybrid` 分支补上与 `routing` 相同的 `DEF_USE_GridRiverLakeFlow` 检查（只多一条停机，
  已能跑的配置结果不变）。Rust 的 `configure_inundation` 收到的 `grid_river` 本来就含运行时开关，两侧一致。GUI 的向导
  按内核与河湖开关给出可选方案，并让动态湿地跟随方案开关。
- **上游**：上游没有 `DEF_USE_GridRiverLakeFlow` 运行时开关，原来对 `routing` 与 `hybrid` 都不查内核。改为在没编进 `GridRiverLakeFlow` 的内核里两者都停机（洪泛比例只由 `MOD_Grid_RiverLakeFlow` 产生），已推（`7d8a4b4b`）。

### 78. 土壤顶界向上通量整份记成土壤蒸发

- **位置**：`main/TRACER/MOD_Tracer_SoilWater.F90` 的地表混合池段（`top_boundary_out_water` 分支，原 997-1026 行）。
- **原因**：只要 `qseva > 0` 且重建的 `qgtop < 0`，就把整个 `-qinfl·dt` 记作 `top_soil_evap_water`，交给
  `atmospheric_loss_tracer` 从第 1 层蒸掉。按注释，这一项本意是"`qseva` 超出地表供水、直接从第 1 层取的亏缺"，
  它不会超过 `qseva·dt`。但变饱和流下饱和土柱向地表渗出时，`-qinfl·dt` 可以远大于它。后面
  `gwat_evap = max(qseva·dt - top_soil_evap_water - …, 0)` 的截断又把这种超出藏起来了。
- **影响**：示踪物多蒸发，蒸发记账的水量也偏大。CN-Cng 单点（IGBP、van Genuchten）2008-01-01 第 16 步：
  - 记账的水量为 5.862 mm，宿主整步蒸发只有 0.0605 mm；
  - 示踪物蒸掉了相当于 1.45 mm 的水，是宿主的 24 倍；
  - 两种同位素的倍数相同。

  不分馏时被上游自己的 `TRC_SIG` 检查拦下而停机。分馏时没有这项检查，δ 会被悄悄扭曲。
- **处理**（第 594 轮）：蒸发只记 `min(-qinfl·dt, qseva·dt)`，余下的走原来的向上渗出支（`trc_soil_upflow` 进地表池）。
  vendor 与 Rust 同步修改。示踪物只记账、不改宿主，所以不带示踪物的结果不变。新增单测
  `exfiltration_beyond_soil_evaporation_is_not_booked_as_evaporation`。上游见 #79 末尾。

### 79. 向上渗出与层间搬运的示踪物被逐层截断（算子顺序）

- **位置**：`main/TRACER/MOD_Tracer_SoilWater.F90` 的三处，都是同一类算子顺序问题：
  - 地表混合池段的 `trc_soil_upflow = min(top_exfil_water * ratio_layer(1), trc_wliq_soisno(1))`；
  - 第 2 段的 `qlayer` 界面循环（自上而下，每个通量截在供水层的现存量上）；
  - 地表池蒸发的分母 `surface_base_water + gwat_evap`。
- **原因**：
  - **顶界截断**：宿主在同一个 Richards 解里从下层给第 1 层补水，再经顶界渗出。示踪物却先算顶界、后算层间。
    第 1 层液态水只有约 2 mm、渗出却有 5–8 mm 时，渗出被截在第 1 层的现存量上。
  - **层间截断**：水一步穿过几层时，`qlayer` 循环里上层先向上送水，它自己要等后面才从下层收到补充，于是每一层都被截一次。
  - **快照比值是噪声**：几乎没水的中转层（例如 1.8e-5 mm 水、却有 2.2 mm 穿过），快照比值没有意义。
  - **蒸发分母**：上游注释写明渗出不参与本步蒸发，所以分子里不加渗出示踪物。但分母用的是最终积水与地表径流，已含渗出的水，蒸发的示踪物因此被稀释。
- **影响**：CN-Cng 单点、不分馏，修了 #78 之后 1 月仍在第 17 步停机，7 月起跑在第 223 步停机。停机前那一步第 1 层比值偏高 85%，地表积水偏低 76%。分馏时同样存在，只是没有检查能发现。
- **处理**（第 595 轮，vendor 与 Rust 同步）：
  1. 顶界渗出整份送进地表池；第 1 层不够付的差额，等 `qlayer` 搬运完再由第 1 层付，仍付不清的记为显式数值源。
  2. `qlayer` 先按快照比值不截断地走一遍；只有某层因此变负、且超出"原有量 + 流经量"的 1e-12 时，才退回原来的逐层截断。被接受的舍入负值归零，并记为数值源。
  3. 一层送出的水多于它在快照时的存量时，送出比值取"存量 + 供水层流入"的混合比值（向上的链自下而上推，向下的链自上而下推）。
  4. 地表池蒸发的分母扣掉本步渗出的水。

  不挥发溶质完全保持原来的截断行为（它有真实的浓度梯度，推迟付款付不清会凭空造出溶质）。gas/particle 示踪物不走这段代码。截断不起作用的步，算术与原来逐位相同。
- **上游**：已推到 `zhongwangwei/CoLM202X` 的 `fix/colm-desktop-audit`（`160dc2cf`，PR #24），与 #78 一起提交。

### 80. 主河道推移质正反两向各算一份满强度、满时长

- **位置**：`main/TRACER/MOD_Tracer_Particle_Sediment.F90` 的 `calc_sediment_advection` 与 `ordinary_sediment_donor_demand`：
  时段平均流量拆成 `rivout_forward`/`rivout_reverse` 两次调用 `calc_sediment_advection_one_direction`，每次都用完整的 `dt`。
- **原因**：悬移质乘方向流量（`sedcon*rivout`），没问题。推移质只用 `rivout` 的符号定方向，强度取时段平均的
  `<v²>` 算出的剪切速度，与方向份额无关。分汊路径乘了 `sed_acc_bif_forward/reverse_time`，主河道没有。
- **影响**：反向份额从 0 增到 1e-6，毛推移质从一份跳到两份（单测复现 11579 → 23158）。相邻单元流域之间照样守恒，
  一般的守恒检查看不出来。在广东 2 天的 `sed` 算例里，128 个单元流域中有 22 个结果变了。变化中位数约 3e-5，
  个别有潮汐回水的单元流域推移质变化 24%–39%。
- **处理**（第 599 轮，vendor 与 Rust 同步）：每个方向的推移质乘该方向的流量份额 `|rivout|/rivout_abs`。
  这与悬移质一致；单向流时份额精确为 1，结果逐位不变。没有用分汊那样的时间份额，因为那要新增累加量、
  改续跑格式，老续跑文件就读不进来了。新增单测 `bedload_scales_with_the_share_of_each_flow_direction`。
- **上游**：已推到 `zhongwangwei/CoLM202X` 的 `fix/colm-desktop-audit`（`401efae9`）。

### 81. 甲烷模块按"均匀密度"重建雪层厚度——未修

- **位置**：`main/TRACER/MOD_Tracer_Reactive_Methane_Impl.F90:240-256`（`dz = snowdp × 本层质量 / 总质量`）；
  用到它的是 `MOD_Tracer_Reactive_Methane_Physics.F90:3105-3118` 的雪层空气比例
  `airfrac = 1 - wice/denice/dz - wliq/denh2o/dz`。
- **原因**：重建出来的各层密度相同。宿主其实每步都有真实层厚 `dz_sno`（`MOD_Vars_TimeVariables.F90:460`，
  有重启读写，`CoLMMAIN.F90:2264` 每步写回）。重建处的注释说"driver 只有水量"，这个说法不对。
- **影响**：`airfrac > 0.05` 时用 Millington–Quirk 气相扩散，否则用液相扩散，两者差几个数量级。
  冰壳与松雪并存时（例如冰壳 1 cm、松雪 19 cm），真实空气比例 [0.02, 0.90] 重建后变成 [0.856, 0.856]：
  冰壳本该是扩散屏障，却按松雪算。审查报告里的数字只在这个层厚组合下成立，两层等厚时是 0.46。
- **现状**：记录在案，待维护者决定。要改得把 `dz_sno` 传进甲烷 driver，vendor 与 Rust 同步。
  有积雪的甲烷算例结果会变，属于物理改动。

### 82. 湖泊碳分解产量没有用库存设上限——未修

- **位置**：`MOD_Tracer_Reactive_Methane_Physics.F90:1965-1984`（`base_decomp = lake_decomp_fact·cnscalefactor·C·dz·q10因子·freeze/catomw`，
  线性，不是指数衰减）；`:1583-1586` 事后只做 `lake_soilc = max(0, C − (CH4+CO2)·deltim·catomw)`。
- **原因**：产量先算出来，库存再截到不小于 0，产量本身不回调。
- **影响**：`k·dt·Q10 因子 > 1` 时凭空造碳。例如 k = 1e-3 s⁻¹、dt = 1800 s、C = 100 gC/m³ 时，产出 180、库存只扣 100，多出 80。
  缺省 `lake_decomp_fact = 9e-11`，`k·dt ≈ 1.6e-7`，实际不会触发；但校验只查非负
  （`MOD_Tracer_Reactive_Methane_Const.F90:1005`，Rust `methane/config.rs` 同），参数调优扫到大值时会进这个区间。
- **现状**：记录在案，待维护者决定。可选做法有两种：一是给 `lake_decomp_fact` 加上限，让 `k·dt` 不超过 1；
  二是把分解改成 `C·(1−exp(−k·dt))`。后者会改变缺省参数下的数值（差在 1e-14 量级），vendor 与 Rust 同步。

### 83. `DEF_TRACER_OPEN_WATER_KINETIC` 的别名比声明长度长——已修

- **位置**：`share/MOD_Namelist.F90:437` 声明 `character(len=16) :: DEF_TRACER_OPEN_WATER_KINETIC`；`:1831-1838` 的
  `SELECT CASE` 把 `'MERLIVAT_JOUZEL1979'`（19 字符）当成 `MJ79` 的别名。
- **原因**：namelist 读入时值被截成 `MERLIVAT_JOUZEL1`，落进 `CASE DEFAULT`，以 `Invalid DEF_TRACER_OPEN_WATER_KINETIC` 停机。
  这个别名永远到不了。
- **影响**：照注释写全名的用户会碰到一个看不懂的停机。GUI 原来从 schema 取可选值，也把它列了出来，选了就保存失败
  （第 600 轮设定扫描发现）。
- **处理**（第 600 轮）：GUI 的 `describe_fields` 滤掉超过字符长度的可选值。
- **修复**（第 601 轮，维护者决定删别名）：vendor 与上游的 `SELECT CASE` 只留 `MJ79`/`mj79`；Rust 的
  `OpenWaterKinetic::parse` 同步删掉；schema 重新生成后可选值只有 `EXPONENT`、`MJ79`。GUI 的长度过滤保留作防线，
  测试改为检查 schema 里所有字符字段的可选值都不超过声明长度。
- **上游**：已推到 `zhongwangwei/CoLM202X` 的 `fix/colm-desktop-audit`（`0077d685`）。

### 84. LCT 模式加 van Genuchten 土壤没有校准：Vcmax 只有 Campbell 那一套，土壤阻抗又被强制关闭——未修

- **位置**：
  - `main/MOD_Const_PFT.F90:1788-1840`：`vmax25_p` 按 `DEF_USE_Campbell_SOIL_MODEL` 分两套。van Genuchten 那套注释写着 "Temporarily tune Vegetation parameter to match VGM model (soil too wet)"，北方常绿针叶树（PFT 2）54 → 26.5、常绿阔叶 56 → 25.2，等等。
  - `main/MOD_Const_LC.F90`：LCT 地类表的 `vmax25` 只有一套，IGBP 第 1 类是 54，与 PFT 的 Campbell 值相同，没有为 VG 下调。
  - `share/MOD_Namelist.F90:1983-1986`：LCT（USGS/IGBP）加 VG 时强制 `DEF_RSS_SCHEME = 0`，土壤蒸发阻抗关闭。PFT/PC 保留默认的 1。
- **影响**（PLUMBER2，两年窗口第二年，Rust 与 Fortran 引擎结果相同）：

| 站点 | 设置 | Qle KGE | Qle 均值 | GPP KGE | GPP 均值 |
|---|---|---|---|---|---|
| CA-Qfo（观测 Qle 21.6，GPP 1.80） | LCT（Vcmax 54、无阻抗） | −1.68 | 64.3 | −2.33 | 6.39 |
| | LCT，Vcmax 26.5 | −0.99 | 54.8 | −0.32 | 3.72 |
| | LCT，Vcmax 26.5，保留阻抗（诊断版） | −0.41 | 46.7 | −0.31 | 3.71 |
| | PC | 0.30 | 34.6 | 0.27 | 2.95 |
| DE-Obe（观测 Qle 31.9，GPP 4.55） | LCT（Vcmax 54、无阻抗） | −0.56 | 73.1 | −0.15 | 8.94 |
| | LCT，Vcmax 26.5 | −0.09 | 62.8 | 0.77 | 5.48 |
| | LCT，Vcmax 26.5，保留阻抗 | 0.24 | 54.2 | 0.77 | 5.49 |
| | PC | 0.60 | 33.5 | 0.77 | 4.73 |

  - LCT 默认设置下光合高 2.2 倍、气孔导度高 3 倍、蒸腾高 2.8 倍，而 LAI、吸收的辐射与水分胁迫都与 PC 相同。
  - 改用 VG 那套 Vcmax 后 GPP 基本回到 PC 水平；再保留土壤阻抗，土壤蒸发降约 8 W/m²。
  - 剩下的 Qle 差距（12–21 W/m²）主要在蒸腾：Vcmax 相同时 LCT 仍比 PC 多约 10 W/m²，应是单层大叶与 PC 三层冠层的结构差别。这一块不算缺陷。
- **官方原版同样如此**：
  - 用官方 `CoLM-SYSU/CoLM202X` master（`626347a9`）编 SinglePoint/LULC_IGBP/vanGenu 内核，前处理也用官方的，CA-Qfo 同一算例。
  - 结果：Qle KGE −1.641、均值 63.6；GPP KGE −2.329、均值 6.39；蒸腾 34.9、土壤蒸发 23.1 W/m²；`f_rss` 为 0。
  - 与本仓库 vendor（−1.683、64.3、6.39）只差已修上游缺陷带来的零头。这是上游本身的行为，不是本仓库改出来的。
  - 构建官方内核时遇到三处问题：
    - 官方缺 `include/Makeoptions.Mac-arm`，从 vendor 拷了一份。
    - `main/HYDRO/MOD_Hydro_VIC_Variables.F90:59` 的注释 `/**<` 没有收尾，cpp 会吞掉第 60 行的声明，补了 `*/`。
    - 官方 SinglePoint 开着 `URBAN_MODEL` 时只允许城市站点（`MOD_SingleSrfdata.F90:316-320`），非城市站点要用 `URBANOFF` 编。
- **处理**：
  - 未修，待维护者决定。可选做法：给 LCT 地类表加一套 VG 下的 `vmax25`（按各地类的主导 PFT 取 VG 值），以及重新评估 LCT 加 VG 时是否该关土壤阻抗。
  - 两者都改变 LCT 默认算例的结果，要重做黄金回归。
  - "保留阻抗"那一行用的是本地临时诊断版 colm-rs（`COLM_RS_DIAG_KEEP_RSS`，未提交）。

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
