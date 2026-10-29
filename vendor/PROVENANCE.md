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
