//! 把每步的 LCT 状态接到 history 写出器上。
//!
//! `colm-hist` 已经有闸门表（哪些变量写得出来）与调度（什么时候写），也已经有写出器；
//! 缺的是"每步的字段值"这一层。这里只填**状态真正拥有**的那些量 —— 与续跑写回用的是
//! 同一份来源（见 `assembly.rs` 的 `evolved_overrides`），所以两处不会各说一套。
//!
//! **没填的变量不声明。** `HistoryBuffers::declare` 只接受调用方点名的变量，写出的文件
//! 因此只包含本层能负责的那些。补齐一个变量要么需要更多内核输出，要么需要先核对上游对
//! 那个诊断量的定义（例如 `h2osoi` 是液态还是液+固态），不是把名字填上就算数。
//!
//! 黄金算例（CN-Cng，126 个变量）此前有缺口，现在**逐名对齐**：本层声明的集合与
//! 黄金文件的变量集合完全相同。剩下的两个"声明了但没有值"的槽位
//! （[`DECLARED_BUT_UNFILLED`] / [`DECLARED_ONLY`]）是**上游在本算例里也留空**，
//! 不是移植缺 —— 这两件事历史上被混为一谈四次，见 [`UNFILLED`] 的注释。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_core::{surface_budget, CalendarTime, StandardLctSoilOutput};
use colm_hist::history::{HistoryBuffers, HistoryDimensions, HistorySite};
use colm_hist::schedule::{
    schedule_records, HistoryFrequency, HistoryGrouping, ScheduledRecord, SimulationWindow,
};

use crate::assembly::{LandPhysicsParameters, StandardLctRestartTemplate};
use colm_core::{StandardLctSnowSoilState, StandardLctSoilState};

/// 本层能填的 history 变量（闸门表写法，不带 `f_` 前缀）。
///
/// 十三个都是**状态量**：三根土柱、地表与叶温、水位三项、雪深/雪水当量、LAI/SAI、雪盖比。
pub const LCT_STATE_VARIABLES: [&str; 13] = [
    "t_soisno",
    "wliq_soisno",
    "wice_soisno",
    "t_grnd",
    "tleaf",
    "zwt",
    "wa",
    "wdsrf",
    "snowdp",
    "scv",
    "lai",
    "sai",
    "fsno",
];

/// 本层能填的**能量侧**诊断量，四个，逐项对照过上游的赋值表达式。
///
/// | 变量 | 上游（`MOD_Thermal.F90`） | 本仓库 |
/// |---|---|---|
/// | `fsena` | `fsena = fsenl + fseng`（:1331） | `energy.total_sensible_heat_w_m2` |
/// | `fevpa` | `fevpa = fevpl + fevpg`（:1332） | `energy.total_evaporation_kg_m2_s` |
/// | `etr` | 叶面蒸腾（`MOD_LeafTemperature.F90:839`） | `energy.leaf.transpiration_kg_m2_s` |
/// | `sabg` | 地面吸收的短波 | `energy.shortwave.ground_absorbed_w_m2` |
///
/// **`lfevpa` 刻意不在此列**，尽管它看起来就是 `fevpa * hvap`。上游写的是
/// `lfevpa = hvap*fevpl + htvp*fevpg`（:1333，注释写着 "accounting for sublimation"）——
/// 地面那一项用的是升华潜热 `htvp`，不是汽化潜热。按名字配上就会在积雪算例里给出
/// 偏高的潜热通量，而海平面无雪算例看不出来。
pub const LCT_SURFACE_BUDGET_VARIABLES: [&str; 8] = [
    "sabvsun", "sabvsha", "rnet", "olrg", "emis", "trad", "fgrnd", "lfevpa",
];

/// 本层能填的**能量侧通量**。
///
/// `fsenl`/`fseng` 与 `fevpl`/`fevpg` 是 `fsena`/`fevpa` 的**叶/地面拆分**，
/// 上游的拆法来自 `MOD_LeafTemperature`：
/// `fevpl = etr + evplwet`（内核对得上：`leaf_evaporation = transpiration + wet_evaporation`，
/// 所以名字里的 "evaporation+transpiration from leaves" 不是笔误），
/// `fevpg = rhoair*cgw*(qg-qaf)`。两者单独写出来的理由是它们各自能差出几十 W/m²，
/// 而和（`fevpa`）却可能几乎抵消 —— 实测正是如此（两边 `fevpa` 都是 0，`lfevpa` 差 29）。
pub const LCT_ENERGY_VARIABLES: [&str; 8] = [
    "fsena", "fevpa", "etr", "sabg", "fsenl", "fseng", "fevpl", "fevpg",
];

/// 本层能填的**地表诊断**量，十三个，全部来自叶温/地表层求解的直接输出。
///
/// 上游把每一个都原样累加后写出（`MOD_Vars_1DAccFluxes.F90` 的
/// `CALL acc1d (x, a_x)`），所以它们与本仓库内核的字段是同一批量：
/// `z0m = z0mv`（`MOD_LeafTemperature.F90:1034`）、`tref`/`qref` 由 `:1261-1262` 算出，
/// 其余是 Monin-Obukhov 诊断。名字一一对应，不经过任何换算。
///
/// **两个看起来很像的量刻意不在此列：**
/// - `emis` 是**平均体积发射率**（`MOD_Thermal.F90:1360` 的 `emis = olru/olrb`），
///   不是算例里那个固定的地表发射率；
/// - `rss` 在方案 4 下被赋成 `1.`（LP92 的**电导标志**），其余方案才是阻力
///   （`MOD_Thermal.F90:618-628`）。同一个变量两种含义，条件映射得先核对
///   `SoilSurfaceResistance` 的输出语义。
pub const LCT_SURFACE_VARIABLES: [&str; 13] = [
    "taux", "tauy", "tref", "qref", "z0m", "zol", "rib", "ustar", "qstar", "tstar", "fm", "fh",
    "fq",
];

/// 本层能填的**水文诊断**量：全部来自 `WATER_2014` 的输出，共六个。
///
/// 每一个的单位都与闸门表核对过（`qinfl`/`rnof`/`rsub`/`rsur`/`qcharge` 是 `mm/s`，
/// `frcsat` 是 `-`），不是按名字猜的。闸门表里没有的量（例如 `smp`）不在此列 ——
/// 它不是默认产出量。
pub const LCT_FLUX_VARIABLES: [&str; 5] = ["qinfl", "rnof", "rsub", "rsur", "frcsat"];

/// `qcharge`：**只有 VSF 关掉时**才产出的水文诊断。
///
/// 上游把它写在 `IF (.not. DEF_USE_VariablySaturatedFlow)` 里
/// （`MOD_Hist.F90:698`），所以 VSF 打开时这一列根本不存在 —— 实测 VSF 黄金
/// 没有 `f_qcharge`、对齐黄金有。这与 `f_vegwp` 是同一类条件声明。
pub const LCT_QCHARGE_VARIABLES: [&str; 1] = ["qcharge"];

/// `qlayer`：**只有 VSF 打开时**才产出的逐界面通量。
///
/// 上游只在 `WATER_VSF` 里填 `qlayer`（`MOD_SoilSnowHydrology.F90:1101` 的
/// `soil_water_vertical_movement` 输出），闸门表的运行时条件就是
/// `DEF_USE_VariablySaturatedFlow`。维度是 `soilinterface`（`nl_soil + 1`）。
pub const LCT_VSF_FLUX_VARIABLES: [&str; 1] = ["qlayer"];

/// VSF 打开时**实填**的、平时留空的三个量。
///
/// `rsur_se`/`rsur_ie` 平时挂在 [`DECLARED_ONLY`]（上游单点算例里整列 `spval`），
/// `frcsat` 挂在 [`DECLARED_BUT_UNFILLED`]；VSF 一开这三项就都有值。
pub const LCT_VSF_FILLED_VARIABLES: [&str; 3] = ["rsur_se", "rsur_ie", "frcsat"];

/// 本层能填的**收支残差**，两项。累加规则普通（`acc1d` + `filter`/`nac`），
/// 但每一项都要把上游在 `MOD_Thermal`/`CoLMMAIN` 里现拼的算式原样搬过来 ——
/// 它们不是状态量，也没有任何现成输出能顶替。见 [`set_lct_balance_errors`]。
///
/// 两个量的量级分别是 ~1e-10（`zerr`）与 ~1e-16（`xerr`）：**它们几乎恒为 0**，
/// 所以验证靠的不是精度而是"拼错项会当场炸成大数"。
pub const LCT_BALANCE_VARIABLES: [&str; 2] = ["xerr", "zerr"];

/// **只有 `DEF_USE_PLANTHYDRAULICS` 打开时**才产出的 history 变量。
///
/// 上游把 `f_vegwp` 整段写在 `IF (DEF_USE_PLANTHYDRAULICS)` 里
/// （`MOD_Hist.F90:4372-4378`），所以 PHS 关掉的算例里这一列**根本不存在**。
/// 实测两份黄金：`CN-Cng-aligned`（显式关掉）没有 `f_vegwp`，
/// `CN-Cng-campbell`（用声明默认值 `.true.`）有 —— 4 个节点 × 264 条。
/// 因此它不能进 [`declare_lct_variables`] 的固定清单：多声明一列会让文件
/// schema 在两个算例上都对不上（关掉的那个会多出一列全填充）。
///
/// 累加规则是**普通的 `acc2d` + `nac`**（`MOD_Vars_1DAccFluxes.F90:2494`），
/// 所以写出的是区间平均，不是末步瞬时值 —— 别把它塞进 `INSTANTANEOUS_VARIABLES`。
pub const LCT_PLANT_HYDRAULIC_VARIABLES: [&str; 1] = ["vegwp"];

/// 本层能填的**瞬时**水量诊断，三项。
///
/// 这三项与其它变量的累加规则又不同：上游先把 `vecacc = wat` 再
/// `WHERE (vecacc /= spval) vecacc = vecacc * nac`，然后交给写出器
/// `acc_vec = acc_vec / nac`（`MOD_Hist.F90:680-684`）—— 乘一个 `nac`
/// 再除一个 `nac`，结果是**写出时刻那一步的瞬时值**，不是区间平均。
///
/// 所以它们必须走 [`HistoryAccumulator`] 的"最后一次覆盖"路径，
/// 而不是累加求和；用求和会得到区间平均，在这个窗口上两者只差 0.001%，
/// 但物理含义不同，且在蓄量快变时会明显分叉。
///
/// 定义取 `CoLMMAIN.F90:2254-2258`（非 VSF 分支）：
/// `wat = sum(wliq+wice) + ldew + scv + wa`，`wa` 是含水层蓄量，
/// `wdsrf` 是地表积水深；`wa_inst`/`wdsrf_inst` 就是这两个标量本身。
/// `wat` 也在这组里，但它**不是**瞬时的：上游用的是普通 `acc1d(wat, a_wat)`
/// 与 `filter`/`nac`（`MOD_Vars_1DAccFluxes.F90:2155`），写的是区间平均。
/// 所以同一个算式在这里出两种口径 —— `wat_inst` 取末步、`wat` 取均值 ——
/// 而这正是单测 `the_instantaneous_water_variables_take_the_last_step_not_the_mean`
/// 要钉住的东西。
pub const LCT_WATER_STORAGE_VARIABLES: [&str; 4] = ["wa_inst", "wdsrf_inst", "wat_inst", "wat"];

/// 本层能填的**10 m 诊断**，四项。
///
/// 它们不是另一支 routine：`MOD_Vars_1DAccFluxes.F90:2733-2790` 那一段在
/// **同一次** `moninobuk` 调用上取 `r_ustar2`/`r_fm10m`，再算
/// `r_us10m = us/um * r_ustar2/vonkar * r_fm10m`（`:2789-2790`）。
/// 本仓库的 [`colm_core::history_diagnostics`] 早就在做那次调用
/// （`MoninObukhovState` 本来就带 `friction_velocity_m_s` 与 `momentum_at_10m`），
/// 缺的只是把它们带出来。
///
/// **注意 `ustar2` 与 `ustar` 不是同一个量**：前者来自这次 MO 调用，
/// 后者由 `tau/rho` 反算（`MOD_Vars_1DAccFluxes.F90:2742`），上游分别写
/// `f_ustar` 与 `f_ustar2`。
pub const LCT_SIMILARITY_10M_VARIABLES: [&str; 4] = ["us10m", "vs10m", "fm10m", "ustar2"];

/// 本层能填的**土壤表面阻力**，一项。
///
/// 上游 `rss` 由 `MOD_SoilSurfaceResistance` 按 `DEF_RSS_SCHEME` 分档算出，
/// 本仓库对应 `StandardLctEnergyOutput::soil_surface_resistance_s_m`。
/// 本算例（Campbell 土壤）用的是**方案 1**（`rss = dsl/dg`）。
///
/// **注意方案 0 那条例外：** `MOD_Namelist.F90:1948-1950` 在
/// `DEF_USE_Campbell_SOIL_MODEL` 为假时把 `DEF_RSS_SCHEME` **强制置 0**
/// （"Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme"）。
/// 本仓库目前只从算例/默认读这个字段，不复制那条强制规则 ——
/// 所以在 VG 土壤算例上两边会分叉（上游 0、本仓库仍按文档里的档位算）。
/// 这里先按 Campbell 算例（强制规则不触发）对齐，例外本身记在
/// `docs/implementation-verification.md`。
pub const LCT_SOIL_RESISTANCE_VARIABLES: [&str; 1] = ["rss"];

/// 本层能填的**冠层截留**量，三项。
///
/// * `ldew`：冠层持水深（mm），就是 [`colm_core::CanopyWater::total_mm`]；
/// * `qintr`：截留率（mm/s），上游 `qintr = pinf/deltim`
///   （`MOD_LeafInterception.F90:335`），即内核的 `retained_kg_m2_s`；
/// * `qdrip`：到达地面的降水（mm/s），上游 `qdrip = pg_rain + pg_snow`
///   （`CoLMMAIN.F90:930`），即内核的 `ground_rain + ground_snow`。
///
/// 三项都是普通 `acc1d` + `filter`/`nac`（区间平均），没有特殊规则。
pub const LCT_CANOPY_WATER_VARIABLES: [&str; 3] = ["ldew", "qintr", "qdrip"];

/// 上游写的是**瞬时值**（乘 `nac` 再被除 `nac`）而不是区间平均的那些变量。
///
/// 累加器对它们走"最后一次覆盖"，`write_means` 的除数因此恒为 1。
pub const INSTANTANEOUS_VARIABLES: [&str; 4] = ["wa_inst", "wdsrf_inst", "wat_inst", "wetwat_inst"];

/// 本层能填的**宽带反照率**，一项。
///
/// `alb` 是黄金文件里**唯一的四维**变量，维度 `(time, patch, rtyp, band)`。
/// 盘上每个 patch 的四项按 `rtyp*2 + band` 排（`rtyp` 0=direct/1=diffuse，
/// `band` 0=visible/1=NIR），即 `[dir_vis, dir_nir, dif_vis, dif_nir]`；
/// 内核的 `ColdStartRadiation::albedo` 是 `[band][rtyp]`，所以取
/// `albedo[0][0], albedo[1][0], albedo[0][1], albedo[1][1]`。
///
/// 值不用另算：`prepare_surface_optics` 每步都把 `albedo` 写进
/// `state.energy.radiation`（`assembly.rs:1359` 那次 `&mut state.energy.radiation`），
/// 续跑写回用的也是同一份。
pub const LCT_ALBEDO_VARIABLES: [&str; 1] = ["alb"];

/// 本层能填的**派生土壤**量，一项。
///
/// `h2osoi` 是体积含水率，上游 `CoLMMAIN.F90:2253`：
///
/// ```fortran
/// h2osoi = wliq_soisno(1:)/(dz_soisno(1:)*denh2o) + wice_soisno(1:)/(dz_soisno(1:)*denice)
/// ```
///
/// **液相与固相各用自己的密度**（1000 与 917 kg/m³），不是统一除以 1000 ——
/// 写成 `(wliq+wice)/(dz*1000)` 会把冰的贡献低估约 8%，而 1 月算例表层是
/// `wice = 4.3` 对 `wliq = 6.0`，正好落在会被看出来的量级。
/// 层是**土层**那 `nl_soil` 层（雪层在负下标），所以维度是 `(time, patch, soil)`。
pub const LCT_DERIVED_SOIL_VARIABLES: [&str; 1] = ["h2osoi"];

/// 上游 `MOD_Const_Physical.F90` 的 `denh2o`/`denice`。两者在 f32 里都能精确表示，
/// 所以写十进制字面量不会引入误差。
const WATER_DENSITY_KG_M3: f64 = 1000.0;
const ICE_DENSITY_KG_M3: f64 = 917.0;

/// 本层能填的**冠层几何**量，三项。
///
/// `sigf` 是"未被雪埋的植被比例"（`MOD_SnowFraction`），本来就在步状态里
/// （`StandardLctEnergyState::vegetation_free_fraction`，续跑写回用的也是它）；
/// `laisun`/`laisha` 是 `lai*fsun` 与 `lai*(1-fsun)`，上游逐 patch 累加
/// （`MOD_Vars_1DAccFluxes.F90:2147-2148`），内核在叶温求解里就已经算好。
///
/// **`green` 不在此列。** 上游 `green` 由 `MOD_LAIEmpirical.F90:132-135` 从
/// `fveg = vegc(ivt)` 得出（`green = 0.; IF (fveg > 0.) green = 1.`），
/// 而 `vegc` 是 `MOD_Const_LC` 的地类表列，本仓库还没把它搬进来 ——
/// 在这个算例上它恒为 1（`fveg > 0`），但"恒为 1"不是实现依据。
pub const LCT_CANOPY_VARIABLES: [&str; 4] = ["sigf", "laisun", "laisha", "green"];

/// 本层能填的**短波分带**量，十七项。
///
/// 上游 `MOD_NetSolar.F90:279-315` 逐定义：
/// `solvd = forc_sols`、`solvi = forc_solsd`、`solnd = forc_soll`、`solni = forc_solld`
/// （`d` = direct、`i` = indirect/diffuse），`sr*` 是同一波段乘反照率后的**反射**，
/// `sr = srvd+srvi+srnd+srni`。
/// `*ln` 是"本地正午"版本：**只在 `local_secs == 43200` 那一步有值，其余是 `spval`**
/// （`:299-313`）。所以它们必须走 [`HistoryAccumulator`] 的"跳过 `spval`"路径 ——
/// 上游的除数是 `nac_ln`，只数 `solvdln /= spval` 的步（`:2041`）。
/// 逐位实测：264 条里 11 条真值、253 条 spval，真值约等于同小时的 `f_solvd`（比值 1.02），
/// 不是它的一半。
pub const LCT_RADIATION_VARIABLES: [&str; 17] = [
    "sr", "solvd", "solvi", "solnd", "solni", "srvd", "srvi", "srnd", "srni", "solvdln", "solviln",
    "solndln", "solniln", "srvdln", "srviln", "srndln", "srniln",
];

/// 本层能填的**驱动场镜像**，七项。
///
/// 上游把 `forc_*` 原样 `acc1d` 进 `a_xy_*` 再写出（`MOD_Vars_1DAccFluxes.F90`），
/// 不做任何换算，所以本仓库也是照抄本步的 `RuntimeForcing`。`f_xy_us`/`f_xy_vs`
/// 要注意标量风：上游标量风下 `forc_vs = 0.`、`forc_us` 就是风速本身。
///
/// **本层刻意不声明 `f_xy_rain`/`f_xy_snow`**：它们是雨雪**相态拆分**的结果，
/// 属于内核下游（`MOD_RainSnowTemp`）而不是驱动场本身，上游也是在那之后才累加的。
pub const LCT_FORCING_VARIABLES: [&str; 11] = [
    "xy_t",
    "xy_q",
    "xy_pbot",
    "xy_us",
    "xy_vs",
    "xy_solarin",
    "xy_frl",
    "xy_prc",
    "xy_prl",
    // `CoLMMAIN.F90:793` 的 `forc_rain = prc_rain + prl_rain`（雪同理），
    // 即**相态拆分之后**的驱动降水 —— 所以它们不来自 `forc_prc`/`forc_prl` 两列，
    // 而来自本步的 `PrecipitationState`（截留之前的那一份）。
    "xy_rain",
    "xy_snow",
];

/// 本层能填的**冠层光合/气孔链**量，十一项。
///
/// 这十一项此前一直挂在 [`UNFILLED`] 的"需要冠层分层输出"名下，其实内核早就算好了：
/// `LeafTemperatureOutput` 的 `sunlit_/shaded_assimilation_mol_m2_s`、
/// `sunlit_/shaded_transpiration_kg_m2_s`、`sunlit_/shaded_stomatal_conductance_mol_m2_s`
/// 与上游 `LeafTemperature` 的 `assimsun_out`/`etrsun_out`/`gssun_out` 是同一个出口，
/// `assim`/`respc` 是它们的冠层和（上游 `MOD_LeafTemperature.F90:1048-1049`
/// `assim = assimsun + assimsha`、`respc = respcsun + respcsha`）。
///
/// **单位是 mol m-2 s-1。** 上游声明处把 `assimsun` 写成 `[umol co2 /m**2/ s]`
/// （`MOD_LeafTemperature.F90:274` 一带），实测黄金值在 1e-9~1e-7 量级 —— 那条注释是错的，
/// 照它乘 1e6 会得到一份偏 6 个数量级的文件。
///
/// `rstfacsun`/`rstfacsha` 在 LCT 分支里是**同一个** `MOD_Eroot` 的 `rstfac`
/// （`MOD_Thermal.F90:674-675` 两行同源），所以 LCT 下必然相等；PFT/PC 分支才逐 PFT
/// 分开（`:1113-1114`）。`rootr` 是 `eroot` 的分层根阻力权重（各层之和为 1），
/// 维度是 `(time, patch, soil)`。
pub const LCT_STOMATAL_VARIABLES: [&str; 11] = [
    "assim",
    "assimsun",
    "assimsha",
    "respc",
    "etrsun",
    "etrsha",
    "gssun",
    "gssha",
    "rstfacsun",
    "rstfacsha",
    "rootr",
];

/// 声明了但**按上游的 `WATER_2014` 不该有值**的量。
///
/// `frcsat` 只在 `WATER_VSF` 里由 `Runoff_*` 算出（`MOD_SoilSnowHydrology.F90:135`），
/// `WATER_2014` 从不设它，所以上游对齐算例那一列整列是 `spval`。声明是为了让文件
/// 的 schema 与上游一致，不填才是数值上一致。
pub const DECLARED_BUT_UNFILLED: [&str; 1] = ["frcsat"];

/// 声明了但**上游在单点算例里本来就不填**的量。
///
/// `sensors` 是用户自定义诊断槽（`MOD_Hist.F90:4671` 的 `nsensor` 槽位），
/// 上游只有在算例主动往里写东西时才有值。对齐黄金算例 264×1×1 条**全是**
/// `missing_value`，`oracle/tolerances.toml` 也把它钉在 tier0 并注明这一点。
/// 所以"声明 + 留空"才是忠实：填任何东西都是无中生有。
pub const DECLARED_ONLY: [&str; 9] = [
    "sensors",
    "rsur_ie",
    "rsur_se",
    // 湖泊与湿地六个量：上游只在**对应的 patch 类型**上写它们
    // （湖 `patchtype == 4`，`MOD_Hist.F90:4499`；湿地 `patchtype == 2`），
    // 所以在植被 patch 上它们整列是填充值 —— 声明 + 留空才是与上游一致的那一列。
    // 湖/湿地 patch 上由 `push_lake`/`push_lct_snow` 各自填值。
    "t_lake",
    "lake_icefrac",
    "lake_deficit",
    "wetwat",
    "wetwat_inst",
    "wetzwt",
];

/// 黄金算例（CN-Cng）里没有、但本层仍会声明的量。
///
/// 闸门表允许写不等于这个算例会产出：`qcharge` 受运行时条件控制，黄金算例没触发。
/// schema 测试因此按"两边都有"来比，并把跳过的名字记下来。
pub const NOT_IN_GOLDEN: [&str; 1] = ["qcharge"];

/// 黄金算例里有、但本层还填不出来的量。
///
/// **现在是空的。** 最后一对是 `xerr`/`zerr`（见 [`LCT_BALANCE_VARIABLES`]）。
/// 上一轮留在这里的两个残差的完整项表已随实现搬进 [`set_lct_balance_errors`] 的注释。
///
/// 这个常量留着不删，是因为它承载的规矩比它的内容重要：**"文件里没有"与
/// "内核产不出"是两件事**。历史上四次把"上游在本算例里就没这一列"（`alb` 的四维通路、
/// 10 m 的廓线、`green` 的 `vegc` 表、湖/湿地六项）误读成"要写一支新物理"——
/// 要动这个清单之前，先数一遍黄金文件里那一列的真值个数。
pub const UNFILLED: [&str; 0] = [];

/// 只在城市上累加的量（`MOD_Vars_1DAccFluxes.F90:2219-2243`；`fahe` 上游不写出）。
pub const URBAN_VARIABLES: [&str; 20] = [
    "t_room",
    "tafu",
    "fhac",
    "fwst",
    "fach",
    "fhah",
    "fvehc",
    "fmeta",
    "fsenroof",
    "fsenwsun",
    "fsenwsha",
    "fsengimp",
    "fsengper",
    "fsenurbl",
    "lfevproof",
    "lfevpgimp",
    "lfevpgper",
    "lfevpurbl",
    "t_roof",
    "t_wall",
];

/// 声明本层能填的全部变量：见 [`LCT_STATE_VARIABLES`] 起的一组常量，
/// 外加 [`DECLARED_ONLY`]（上游在本算例里也留空的那几个槽位）。
/// `DEF_USE_BGC` 下 `MOD_Hist.F90` 写出的变量：闸门表里运行时条件为 `DEF_USE_BGC`
/// （NITRIF 打开时再加 `(DEF_USE_BGC) .and. (DEF_USE_NITRIF)` 那两个）、且不带编译期宏
/// （`#ifdef CROP` 那批）的全部名字，按表中顺序去重。FIRE/DiagMatrix/臭氧的组合条件对应的
/// 分支在运行期被拒绝，这里不必列。
pub fn bgc_history_variables(switches: colm_core::bgc_driver::BgcSwitches) -> Vec<&'static str> {
    let (nitrif, diag_matrix, crop) = (switches.nitrif, switches.diag_matrix, switches.crop);
    let mut names: Vec<&'static str> = Vec::new();
    for var in colm_hist::generated::VARS {
        let wanted = match var.runtime {
            Some("DEF_USE_BGC") => true,
            Some("(DEF_USE_BGC) .and. (DEF_USE_NITRIF)") => nitrif,
            Some("(DEF_USE_BGC) .and. (DEF_USE_DiagMatrix)") => diag_matrix,
            Some("(DEF_USE_BGC) .and. (DEF_USE_FIRE)") => switches.fire,
            Some("(DEF_USE_BGC) .and. (DEF_USE_IRRIGATION)") => switches.irrigation,
            _ => false,
        };
        // `#ifdef CROP` 的一批（64 个）只在 CROP 内核里存在。
        let compiled =
            var.macros.is_empty() || (crop && var.macros == [colm_hist::Cond::AnyOf(&["CROP"])]);
        // `manunitro` 声明默认为关，只有 `DEF_USE_FERT` 时 `sync_hist_vars` 才把它同步成
        // `DEF_HIST_vars_out_default`（`MOD_Namelist.F90:3315-3326`）；`fertnitro_*` 声明默认即为开。
        let enabled = var.name != "manunitro" || switches.fert;
        if wanted && compiled && enabled && !names.contains(&var.name) {
            names.push(var.name);
        }
    }
    names
}

/// 一步的 BGC 历史量（上游 `accumulate_fluxes` 的 `IF (DEF_USE_BGC)` 段）。
///
/// 值与上游 `acc1d`/`acc2d` 的来源一一对应：绝大多数就是同名的 BGC patch 变量；
/// `f_hr` 累加的是 `decomp_hr`，`f_retrasn`（上游拼写）累加 `retransn`；`*_vr` 分池廓线是
/// `decomp_cpools_vr`/`decomp_npools_vr` 按池切片的 `1:nl_soil`；`BD_all`/`OM_density`/`wfc`
/// 是常数重启里的土壤参数；`lai_*` 是 `CNDriverSummarizeStates` 写的分 PFT 类型 LAI。
/// 历史累加器旁车的文件名：`<case>_restart_<date>_lc<year>_<block>.nc` →
/// `<case>_restart_hist_<date>_<block>.nc`（上游 `history_acc_file` 不带 `_lc<year>`，块后缀由向量 I/O 追加）。
pub fn history_sidecar_name(restart: &str) -> Result<String> {
    let hist = restart.replacen("_restart_", "_restart_hist_", 1);
    let lc = hist
        .find("_lc")
        .filter(|&at| {
            hist.get(at + 3..at + 7)
                .is_some_and(|year| year.bytes().all(|b| b.is_ascii_digit()))
        })
        .context("the restart name has no _lc<year> part")?;
    Ok(format!("{}{}", &hist[..lc], &hist[lc + 7..]))
}

/// 按作物类型分列的 CROP 历史量（`MOD_Hist.F90` 作物段）：`(历史名, 累加的 patch 量, 作物类别)`。
/// 只在 `patchclass == 12` 且 patch 的首个 PFT 类别在列表里时写出，否则是填充值。
const CROP_TYPE_HISTORY: &[(&str, &str, &[i32])] = &[
    ("huiswheat", "hui", &[19, 20]),
    // `irrig_method_*`（`MOD_Hist.F90` 作物段）：玉米原来只认雨养类别 17，灌溉玉米 18 恰是唯一会灌溉的
    // 玉米却是缺测（upstream-bugs 第 29 条，vendor 已修成 17/18）。
    ("irrig_method_corn", "irrig_method_corn", &[17, 18]),
    ("irrig_method_swheat", "irrig_method_swheat", &[19, 20]),
    ("irrig_method_wwheat", "irrig_method_wwheat", &[21, 22]),
    (
        "irrig_method_soybean",
        "irrig_method_soybean",
        &[23, 24, 77, 78],
    ),
    ("irrig_method_cotton", "irrig_method_cotton", &[41, 42]),
    ("irrig_method_rice1", "irrig_method_rice1", &[61, 62]),
    ("irrig_method_rice2", "irrig_method_rice2", &[61, 62]),
    (
        "irrig_method_sugarcane",
        "irrig_method_sugarcane",
        &[67, 68],
    ),
    ("fertnitro_corn", "fertnitro_corn", &[17, 18, 75, 76]),
    ("fertnitro_swheat", "fertnitro_swheat", &[19, 20]),
    ("fertnitro_wwheat", "fertnitro_wwheat", &[21, 22]),
    ("fertnitro_soybean", "fertnitro_soybean", &[23, 24, 77, 78]),
    ("fertnitro_cotton", "fertnitro_cotton", &[41, 42]),
    ("fertnitro_rice1", "fertnitro_rice1", &[61, 62]),
    ("fertnitro_rice2", "fertnitro_rice2", &[61, 62]),
    ("fertnitro_sugarcane", "fertnitro_sugarcane", &[67, 68]),
    ("plantdate_rainfed_temp_corn", "plantdate", &[17]),
    ("plantdate_irrigated_temp_corn", "plantdate", &[18]),
    ("plantdate_rainfed_spwheat", "plantdate", &[19]),
    ("plantdate_irrigated_spwheat", "plantdate", &[20]),
    ("plantdate_rainfed_wtwheat", "plantdate", &[21]),
    ("plantdate_irrigated_wtwheat", "plantdate", &[22]),
    ("plantdate_rainfed_temp_soybean", "plantdate", &[23]),
    ("plantdate_irrigated_temp_soybean", "plantdate", &[24]),
    ("plantdate_rainfed_cotton", "plantdate", &[41]),
    ("plantdate_irrigated_cotton", "plantdate", &[42]),
    ("plantdate_rainfed_rice", "plantdate", &[61]),
    ("plantdate_irrigated_rice", "plantdate", &[62]),
    ("plantdate_rainfed_sugarcane", "plantdate", &[67]),
    ("plantdate_irrigated_sugarcane", "plantdate", &[68]),
    ("plantdate_rainfed_trop_corn", "plantdate", &[75]),
    ("plantdate_irrigated_trop_corn", "plantdate", &[76]),
    ("plantdate_rainfed_trop_soybean", "plantdate", &[77]),
    ("plantdate_irrigated_trop_soybean", "plantdate", &[78]),
    ("plantdate_unmanagedcrop", "plantdate", &[15]),
    ("cropprodc_rainfed_temp_corn", "grainc_to_cropprodc", &[17]),
    (
        "cropprodc_irrigated_temp_corn",
        "grainc_to_cropprodc",
        &[18],
    ),
    ("cropprodc_rainfed_spwheat", "grainc_to_cropprodc", &[19]),
    ("cropprodc_irrigated_spwheat", "grainc_to_cropprodc", &[20]),
    ("cropprodc_rainfed_wtwheat", "grainc_to_cropprodc", &[21]),
    ("cropprodc_irrigated_wtwheat", "grainc_to_cropprodc", &[22]),
    (
        "cropprodc_rainfed_temp_soybean",
        "grainc_to_cropprodc",
        &[23],
    ),
    (
        "cropprodc_irrigated_temp_soybean",
        "grainc_to_cropprodc",
        &[24],
    ),
    ("cropprodc_rainfed_cotton", "grainc_to_cropprodc", &[41]),
    ("cropprodc_irrigated_cotton", "grainc_to_cropprodc", &[42]),
    ("cropprodc_rainfed_rice", "grainc_to_cropprodc", &[61]),
    ("cropprodc_irrigated_rice", "grainc_to_cropprodc", &[62]),
    ("cropprodc_rainfed_sugarcane", "grainc_to_cropprodc", &[67]),
    (
        "cropprodc_irrigated_sugarcane",
        "grainc_to_cropprodc",
        &[68],
    ),
    ("cropprodc_rainfed_trop_corn", "grainc_to_cropprodc", &[75]),
    (
        "cropprodc_irrigated_trop_corn",
        "grainc_to_cropprodc",
        &[76],
    ),
    (
        "cropprodc_rainfed_trop_soybean",
        "grainc_to_cropprodc",
        &[77],
    ),
    (
        "cropprodc_irrigated_trop_soybean",
        "grainc_to_cropprodc",
        &[78],
    ),
    ("cropprodc_unmanagedcrop", "grainc_to_cropprodc", &[15]),
];

fn set_bgc_history(
    sink: &mut impl HistorySink,
    record: usize,
    runtime: &crate::bgc_step::BgcRuntime,
    s: &colm_core::bgc_state::BgcState,
    first_pft_class: Option<i32>,
    irrigation: Option<&colm_core::IrrigationState>,
    soil: bool,
) -> Result<()> {
    let nl = s.dims.nl_soil;
    let full = s.dims.nl_soil_full;
    let c = &s.constants;
    let switches = runtime.switches;
    // 历史里写的量，外加只为旁车累加的：上游在 `IF (DEF_USE_BGC)` 段无条件 `acc1d`，历史却按
    // `DEF_hist_vars`/开关才写（如 `t_scalar` 只在 DiagMatrix 下写、`pd*` 与 `irrig_method_*` 从不写）。
    // 植被 `*Cap` 只在 DiagMatrix 下累加（`:2442`），O2 两项只在 NITRIF 下累加（`:2380`）。
    let mut names = bgc_history_variables(switches);
    for entry in crate::history_manifest::MANIFEST.iter() {
        use crate::history_sidecar::Requires;
        let key = entry.window_key();
        let allocated = match entry.requires {
            Requires::Bgc => true,
            Requires::BgcCrop => switches.crop,
            _ => false,
        };
        let unaccumulated = BGC_UNACCUMULATED.contains(&key) && irrigation.is_none();
        let accumulated = !unaccumulated
            && (switches.fire || !fire_only(key))
            && (switches.diag_matrix || !(entry.rank == 1 && key.ends_with("Cap")))
            && (switches.nitrif || !matches!(key, "CONC_O2_UNSAT" | "O2_DECOMP_DEPTH_UNSAT"));
        if allocated && accumulated && !names.contains(&key) {
            names.push(key);
        }
    }
    for name in names {
        // 火诊断量在 `DEF_USE_BGC` 下声明，但只在 `DEF_USE_FIRE` 下累加；FIRE 关时 CNSummary 照样把
        // `fire_closs` 等算成 0，不能交给累加器。
        if !switches.fire && fire_only(name) {
            continue;
        }
        let (name, source) = match CROP_TYPE_HISTORY.iter().find(|(field, ..)| *field == name) {
            Some((field, source, classes)) => {
                // 上游对每个 patch 都 `acc1d` 这些量，写历史时才按作物类别过滤（`filter_crop`）。
                let cropland = runtime.statics.patchclass == 12;
                let matching =
                    cropland && first_pft_class.is_some_and(|class| classes.contains(&class));
                if !matching && !sink.keep_filtered(field) {
                    continue;
                }
                (*field, *source)
            }
            None => (name, name),
        };
        // 按 14 类自然 PFT 分列的 `gpp_*`/`leafc_*`/`lai_*`/`npp_*`/`npptoleafc_*`
        // （`MOD_Hist.F90` 里紧接 `w_scalar` 的那段）只写 `patchclass /= 12 .and. patchtype == 0`
        // 的 patch：农田 patch 上是填充值。BGC patch 恒为 `patchtype == 0`。
        let natural_type = ["gpp_", "leafc_", "lai_", "npp_", "npptoleafc_"]
            .iter()
            .any(|prefix| {
                name.strip_prefix(prefix).is_some_and(|kind| {
                    colm_core::bgc_state::LAI_DIAGNOSTICS
                        .iter()
                        .any(|lai| lai.strip_prefix("lai_") == Some(kind))
                })
            });
        // 同样照累加、写出时过滤；非土壤 patch（`patchtype /= 0`，空间算例里与土壤 patch 同格）也不计入。
        if natural_type && (runtime.statics.patchclass == 12 || !soil) && !sink.keep_filtered(name)
        {
            continue;
        }
        // `BD_all`/`wfc`/`OM_density` 每个算例都累加，统一由 `set_sidecar_only` 写。
        if SOIL_STATICS.contains(&name) {
            continue;
        }
        if let Some(value) = irrigation.and_then(|state| irrigation_history_value(name, state)) {
            // `filter_irrig`（`MOD_Hist.F90:1482-1500`）：农田 patch、首个 PFT 是灌溉作物（≥ 17 的偶数类别）。
            let irrigated = runtime.statics.patchclass == 12
                && first_pft_class.is_some_and(|class| class >= 17 && class % 2 == 0);
            if !irrigated && !sink.keep_filtered(name) {
                continue;
            }
            sink.scalar(name, record, value)?;
            continue;
        }
        if let Some(k) = colm_core::bgc_state::IRRIGATION_DIAGNOSTICS
            .iter()
            .position(|n| *n == name)
        {
            sink.scalar(name, record, s.irrigation_diagnostics[k])?;
            continue;
        }
        if let Some((_, values)) = runtime
            .statics
            .soil
            .iter()
            .find(|(field, _)| *field == name)
        {
            sink.layer(name, record, values)?;
            continue;
        }
        if let Some(k) = colm_core::bgc_state::LAI_DIAGNOSTICS
            .iter()
            .position(|n| *n == name)
        {
            sink.scalar(name, record, s.lai_diagnostics[k])?;
            continue;
        }
        let pool = |prefix: &str| -> Option<i32> {
            Some(match prefix {
                "litr1" => c.i_met_lit,
                "litr2" => c.i_cel_lit,
                "litr3" => c.i_lig_lit,
                "soil1" => c.i_soil1,
                "soil2" => c.i_soil2,
                "soil3" => c.i_soil3,
                "cwd" => c.i_cwd,
                _ => return None,
            })
        };
        // `*Cap_vr`（DiagMatrix）取 `decomp_{c,n}pools_vr_Cap` 的同一切片。
        let (stem, capacity) = match name.strip_suffix("Cap_vr") {
            Some(stem) => (Some(stem), true),
            None => (name.strip_suffix("_vr"), false),
        };
        if let Some(stem) = stem {
            let (prefix, element) = stem.split_at(stem.len() - 1);
            if let (Some(index), "c" | "n") = (pool(prefix), element) {
                let values = match (element, capacity) {
                    ("c", false) => &s.patch.decomp_cpools_vr,
                    ("c", true) => &s.patch.decomp_cpools_vr_Cap,
                    (_, false) => &s.patch.decomp_npools_vr,
                    (_, true) => &s.patch.decomp_npools_vr_Cap,
                };
                let start = full * usize::try_from(index - 1).context("pool index")?;
                sink.layer(name, record, &values[start..start + nl])?;
                continue;
            }
        }
        let source = match FIRE_SOURCES.iter().find(|(field, _)| *field == source) {
            Some((_, state)) => state,
            None => source,
        };
        let source = match source {
            "hr" => "decomp_hr",
            "retrasn" => "retransn",
            "CONC_O2_UNSAT" => "tconc_o2_unsat",
            "O2_DECOMP_DEPTH_UNSAT" => "to2_decomp_depth_unsat",
            other => other,
        };
        let values = s
            .f64_field(source)
            .with_context(|| format!("BGC history variable {name} has no source {source}"))?;
        if values.len() == 1 {
            sink.scalar(name, record, values[0])?;
        } else {
            sink.layer(name, record, &values[..nl])?;
        }
    }
    Ok(())
}

pub fn declare_lct_variables(
    buffer: &mut HistoryBuffers,
    plant_hydraulics: bool,
    variably_saturated: bool,
) -> Result<()> {
    let mut names = LCT_STATE_VARIABLES.to_vec();
    names.extend_from_slice(&LCT_FLUX_VARIABLES);
    names.extend_from_slice(&LCT_ENERGY_VARIABLES);
    names.extend_from_slice(&LCT_SURFACE_BUDGET_VARIABLES);
    names.extend_from_slice(&LCT_SURFACE_VARIABLES);
    names.extend_from_slice(&LCT_STOMATAL_VARIABLES);
    names.extend_from_slice(&LCT_FORCING_VARIABLES);
    names.extend_from_slice(&LCT_RADIATION_VARIABLES);
    names.extend_from_slice(&LCT_CANOPY_VARIABLES);
    names.extend_from_slice(&LCT_DERIVED_SOIL_VARIABLES);
    names.extend_from_slice(&LCT_ALBEDO_VARIABLES);
    names.extend_from_slice(&LCT_WATER_STORAGE_VARIABLES);
    names.extend_from_slice(&LCT_CANOPY_WATER_VARIABLES);
    names.extend_from_slice(&LCT_SOIL_RESISTANCE_VARIABLES);
    names.extend_from_slice(&LCT_SIMILARITY_10M_VARIABLES);
    names.extend_from_slice(&LCT_BALANCE_VARIABLES);
    if plant_hydraulics {
        names.extend_from_slice(&LCT_PLANT_HYDRAULIC_VARIABLES);
    }
    // 两个互斥的条件列：`qcharge` 只在 VSF 关掉时存在，`qlayer` 只在打开时存在。
    if variably_saturated {
        names.extend_from_slice(&LCT_VSF_FLUX_VARIABLES);
    } else {
        names.extend_from_slice(&LCT_QCHARGE_VARIABLES);
    }
    names.extend_from_slice(&DECLARED_ONLY);
    buffer.declare(&names)
}

/// history 的写出目标。
///
/// 两个实现：[`HistoryBuffers`]（直接落到第 `record` 条记录）与
/// [`HistoryAccumulator`]（每步累加）。**为什么要有第二个**：上游写出的每一条记录都是
/// **区间平均**，不是写出时刻的瞬时值 —— `MOD_Hist.F90:227` 每步
/// `accumulate_fluxes`，到写出的那一步在 `write_history_variable_2d` 里
/// `acc_vec = acc_vec / nac`，写完再 `CALL FLUSH_acc_fluxes ()`（`:4746`）清零。
pub trait HistorySink {
    fn scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()>;
    fn layer(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()>;
    /// 往**同一个**累加器里再加一项，并声明它算不算一步。
    ///
    /// 上游 `acc1d` 一步里可能被调用多次而步数只记一次：短波四波段
    /// （`MOD_Vars_1DAccFluxes.F90:2060-2063`）分别累加到同一个 `a_solarin`，
    /// 而 `nac = nac + 1` 在 `:2038` 每步只执行一次。于是"先各步求和、再相加"
    /// 与"八项从左到右连加"会差 1 ULP，而 `f_xy_solarin` 是 **tier0**
    /// （实测干 11/264、湿 23/384、雪 22/360 条差 1 ULP）。
    ///
    /// 两个 sink 都**必须**显式实现：直接写缓冲的那种没有步数概念，
    /// 给它一个"忽略 `counts_as_step`"的默认实现会静默把后续项覆盖掉。
    fn accumulate(
        &mut self,
        name: &str,
        record: usize,
        value: f64,
        counts_as_step: bool,
    ) -> Result<()>;

    /// 一个按 `patchtype` 过滤掉的量：返回 `true` 表示仍要收下它的值。
    ///
    /// 上游对每个 patch 都照样 `acc1d`，过滤只发生在写历史时（`write_history_variable_2d`
    /// 的 `filter`）。累加器因此收下原值、写平均时再跳过 —— 续跑旁车里的 `a_*` 才对得上；
    /// 直接写缓冲的一方没有"写出时"这一步，默认丢弃。
    fn keep_filtered(&mut self, _name: &str) -> bool {
        false
    }
}

impl HistorySink for HistoryBuffers {
    fn scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        // 只为旁车累加、本文件没声明的量：直写没有旁车可去，丢弃。
        if !self.declares(name) && crate::history_sidecar::is_window_key(name) {
            return Ok(());
        }
        self.set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))
    }

    fn layer(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        if !self.declares(name) && crate::history_sidecar::is_window_key(name) {
            return Ok(());
        }
        self.set_layered(name, record, values)
            .with_context(|| format!("cannot write {name} into the history buffers"))
    }

    fn accumulate(
        &mut self,
        name: &str,
        record: usize,
        value: f64,
        counts_as_step: bool,
    ) -> Result<()> {
        if counts_as_step {
            self.scalar(name, record, value)
        } else {
            self.add_patch_scalar(name, record, value)
                .with_context(|| format!("cannot add a contribution to {name}"))
        }
    }
}

/// 一个输出区间里逐变量的和与步数（上游的 `a_*` 与 `nac`）。
#[derive(Debug, Default)]
struct HistoryAccumulator {
    sums: std::collections::BTreeMap<String, Accumulated>,
    steps: usize,
    /// 本区间里按 `patchtype` 过滤、写平均时留作填充值的量（见 [`HistorySink::keep_filtered`]）。
    filtered: std::collections::BTreeSet<String>,
    /// 本区间里交过值的量，含只交过 `spval` 的（上游每个 patch 都有 `a_*`，过滤只看 `patchtype`）：
    /// 网格聚合的分母按它算——湿地的 `a_qlayer` 一直是 `spval`，却照样占 `sumarea`。
    offered: std::collections::BTreeSet<String>,
}

#[derive(Debug)]
enum Accumulated {
    Scalar { sum: f64, count: usize },
    Column { sum: Vec<f64>, count: usize },
}

/// 按自身有效步数平均的历史量：`f_alb`（`nac_dt`）与本地正午的 8 个短波量（`nac_ln`）。
const OWN_COUNT_VARIABLES: [&str; 9] = [
    "alb", "solvdln", "solviln", "solndln", "solniln", "srvdln", "srviln", "srndln", "srniln",
];

impl HistoryAccumulator {
    /// 一个变量的平均除数：[`OWN_COUNT_VARIABLES`] 用自己的有效步数，其余用全局步数 `nac`。
    fn divisor(&self, name: &str, count: usize) -> f64 {
        if OWN_COUNT_VARIABLES.contains(&name) {
            count as f64
        } else {
            self.steps as f64
        }
    }

    /// 按变量所属的计数器取平均后写进第 `record` 条。标量/列由**累加时**
    /// 的形态决定，不在这里猜 —— 猜错会把一根土柱按标量写出去。
    ///
    /// **除数按变量分组。** 上游 `acc1d` 会跳过 `spval`
    /// （`MOD_Vars_1DAccFluxes.F90:2895` `IF (var(i) /= spval)`），而除数各组一个计数器：
    /// `nac_dt`（只数白天，只用于 `f_alb`）、`nac_ln`（只数 `solvdln /= spval` 的步，`:2041`，
    /// 8 个本地正午量）——这两组写出的是**它自己的平均**（[`OWN_COUNT_VARIABLES`]）；
    /// 逐位实测 `f_solvdln` 在 264 条里 11 条真值、253 条 spval，真值约等于同小时的 `f_solvd`。
    /// **其余变量一律除以全局 `nac`**，即使某些步是 spval 被跳过：DiagMatrix 的 `*Cap` 在年末
    /// 那一小时里前一步还是 spval、后一步才有值，上游写出的是值的一半（第 421 轮）。
    fn write_means(&self, buffer: &mut HistoryBuffers, record: usize) -> Result<()> {
        self.write_plain_means(buffer, record)
    }

    /// `DEF_USE_Dynamic_Wetland`：`f_wetwat` 写的是 `a_wdsrf / nac`（`MOD_Hist.F90:893-898`），
    /// 与 `f_wdsrf` 同一次除法；`a_wetwat` 照常累加进旁车。只在湿地上写（过滤同 `f_wetwat`）。
    fn write_dynamic_wetland_storage(
        &self,
        buffer: &mut HistoryBuffers,
        record: usize,
    ) -> Result<()> {
        if !buffer.declares("wetwat") || self.filtered.contains("wetwat") {
            return Ok(());
        }
        let value = match self.sums.get("wdsrf") {
            Some(Accumulated::Scalar { sum, count }) if *count > 0 => sum / self.steps as f64,
            _ => colm_core::MISSING,
        };
        buffer.include("wetwat", record)?;
        buffer
            .set_patch_scalar("wetwat", record, value)
            .context("cannot write the dynamic wetland storage")
    }

    fn write_plain_means(&self, buffer: &mut HistoryBuffers, record: usize) -> Result<()> {
        self.write_plain_means_where(buffer, record, |_| true)
    }

    /// 被强迫缺测遮蔽的 patch：上游各 `filter` 都与上了 `forcmask_pch`，唯独 CROP 段按作物类别
    /// 现建的那几个没有（`MOD_Hist.F90:2356-2900` 的 `f_manunitro`、`f_huiswheat`、`f_fertnitro_*`、
    /// `f_irrig_method_*`），被遮蔽的作物 patch 照样进它们的分子分母。
    fn write_masked_crop_means(&self, buffer: &mut HistoryBuffers, record: usize) -> Result<()> {
        self.write_plain_means_where(buffer, record, |name| {
            matches!(name, "manunitro" | "huiswheat")
                || name.starts_with("fertnitro_")
                || name.starts_with("irrig_method_")
        })
    }

    fn write_plain_means_where(
        &self,
        buffer: &mut HistoryBuffers,
        record: usize,
        keep: impl Fn(&str) -> bool,
    ) -> Result<()> {
        ensure!(
            self.steps > 0,
            "the history accumulator reached a write step without accumulating anything"
        );
        // 网格聚合的分母（上游的 `filter`）：交过值、没被过滤就计入，哪怕整段都是 `spval`；
        // 只有 `f_alb`（`filter_dt`）与本地正午量（`nac_ln > 0`）还要求自己的计数非零。
        for name in &self.offered {
            if !keep(name) || !buffer.declares(name) || self.filtered.contains(name) {
                continue;
            }
            let count = match self.sums.get(name) {
                Some(Accumulated::Scalar { count, .. } | Accumulated::Column { count, .. }) => {
                    *count
                }
                None => 0,
            };
            if count > 0 || !OWN_COUNT_VARIABLES.contains(&name.as_str()) {
                buffer.include(name, record)?;
            }
        }
        buffer.set_steps(record, self.steps as f64)?;
        for (name, accumulated) in &self.sums {
            if !keep(name) {
                continue;
            }
            // 只为旁车累加、本算例历史文件里没有的量（上游照样 `acc1d`，只是不写出）。
            if !buffer.declares(name) && crate::history_sidecar::is_window_key(name) {
                continue;
            }
            if self.filtered.contains(name) {
                continue;
            }
            match accumulated {
                Accumulated::Scalar { sum, count } => {
                    // 整条记录里一次有效值都没有的变量沿用缓冲区的填充值，
                    // 与上游"累加器一直是 spval、除以 0 个样本仍是 spval"同效。
                    if *count == 0 {
                        continue;
                    }
                    // 上游 `acc_vec = acc_vec / nac`（`MOD_HistSingle.F90:272`）是**除法**；
                    // 乘倒数只在 `nac` 为 2 的幂时相同 —— 小时记录（两子步）看不出来，
                    // 日记录（48 步）就差 1 ulp（第 406 轮）。
                    // 瞬时量上游写的是 `vecacc = x*nac`、再按均值那条路 `/nac`
                    // （`MOD_Hist.F90:741-743`）：往返在 `nac` 不是 2 的幂时不一定精确。灌溉的四个
                    // 赋值量（[`ASSIGNED_VARIABLES`]）vendor 修复后同样写法。
                    let mean = if INSTANTANEOUS_VARIABLES.contains(&name.as_str())
                        || ASSIGNED_VARIABLES.contains(&name.as_str())
                    {
                        let steps = self.steps as f64;
                        sum * steps / steps
                    } else {
                        sum / self.divisor(name, *count)
                    };
                    buffer
                        .set_patch_scalar(name, record, mean)
                        .with_context(|| format!("cannot write {name} into the history buffers"))?;
                }
                Accumulated::Column { sum, count } => {
                    if *count == 0 {
                        continue;
                    }
                    // 向量写出的一维层量：交原始累加，聚合后再 `/sumwt/nac`。
                    if buffer.wants_raw_layers(name) {
                        buffer.set_layered(name, record, sum).with_context(|| {
                            format!("cannot write {name} into the history buffers")
                        })?;
                        continue;
                    }
                    let steps = self.divisor(name, *count);
                    // `WHERE (acc /= spval) acc = acc / nac`：从未有效的元素保持 spval。
                    let mean = sum
                        .iter()
                        .map(|&value| {
                            if value == colm_core::MISSING {
                                value
                            } else {
                                value / steps
                            }
                        })
                        .collect::<Vec<_>>();
                    buffer
                        .set_layered(name, record, &mean)
                        .with_context(|| format!("cannot write {name} into the history buffers"))?;
                }
            }
        }
        Ok(())
    }
}

/// 常数重启里的三个土壤参数：每个算例都累加（`MOD_Vars_1DAccFluxes.F90:2496-2498`），
/// 由 [`set_sidecar_only`] 统一写；BGC 历史里声明了它们，`set_bgc_history` 不再重复写。
const SOIL_STATICS: [&str; 3] = ["BD_all", "wfc", "OM_density"];

/// CROP 下分配的灌溉账目：灌溉关掉时上游数组一直是 `spval`，不累加；打开时见
/// [`irrigation_history_value`]。
const BGC_UNACCUMULATED: [&str; 11] = [
    "sum_irrig",
    "sum_deficit_irrig",
    "sum_irrig_count",
    "waterstorage",
    "groundwater_demand",
    "groundwater_supply",
    "reservoirriver_demand",
    "reservoirriver_supply",
    "reservoir_supply",
    "river_supply",
    "runoff_supply",
];

/// 灌溉打开时 `accumulate_fluxes` 的灌溉段（`MOD_Vars_1DAccFluxes.F90:2423-2432`）交给累加器的值。
/// 四个生长季累计量是**赋值**（见 [`ASSIGNED_VARIABLES`]），其余 `acc1d`。
fn irrigation_history_value(name: &str, state: &colm_core::IrrigationState) -> Option<f64> {
    Some(match name {
        "sum_irrig" => state.sum_mm,
        "sum_irrig_count" => state.sum_count,
        "waterstorage" => state.water_storage_mm,
        "sum_deficit_irrig" => state.sum_deficit_mm,
        "groundwater_demand" => state.groundwater_demand_mm,
        "groundwater_supply" => state.groundwater_supply_mm,
        "reservoirriver_demand" => state.reservoirriver_demand_mm,
        "reservoirriver_supply" => state.reservoirriver_supply_mm,
        "reservoir_supply" => state.reservoir_supply_mm,
        "river_supply" => state.river_supply_mm,
        "runoff_supply" => state.runoff_supply_mm,
        _ => return None,
    })
}

/// 每步**赋值**而不是 `acc1d` 的生长季累计量（`a_sum_irrig = sum_irrig` 等）：累加器里只留末步的值，
/// 写出时与 `*_inst` 一样先乘 `nac` 再交给除以 `nac` 的写出器，即写末值；旁车里也是末值。上游原来漏了
/// `a_sum_deficit_irrig`、且写出"末值 / nac"（upstream-bugs 第 27 条，vendor 已修）。
const ASSIGNED_VARIABLES: [&str; 4] = [
    "sum_irrig",
    "sum_deficit_irrig",
    "sum_irrig_count",
    "waterstorage",
];

/// `DEF_USE_FIRE` 下才累加（`MOD_Vars_1DAccFluxes.F90` 的 `IF (DEF_USE_FIRE)` 段）、来源名与历史名不同的量。
///
/// 上游原来五次都把临时数组 `vecacc` 传给 `write_history_variable_2d`，写出的是上一个历史量的残留；
/// PR #504 改成传 `a_abm` 等，现在都是普通的时间平均。
const FIRE_SOURCES: [(&str, &str); 7] = [
    ("abm", "abm_lf"),
    ("gdp", "gdp_lf"),
    ("peatf", "peatf_lf"),
    ("hdm", "hdm_lf"),
    ("btran2", "fire_btran2"),
    ("col_fire_closs", "fire_closs"),
    ("col_fire_nloss", "fire_nloss"),
];

/// PR #504 的火诊断量（`MOD_Vars_1DAccFluxes.F90` 的 `IF (DEF_USE_FIRE)` 段 `acc1d`，按历史名）。
/// 历史在 `DEF_USE_BGC` 下就声明（`MOD_Hist.F90` 的 Fire diagnostics 段），火关闭时是填充值。
const FIRE_DIAGNOSTICS: [&str; 104] = [
    "farea_burned",
    "baf_crop",
    "baf_peatf",
    "nfire",
    "fuelc",
    "btran2",
    "pft_fire_closs",
    "pft_fire_nloss",
    "col_fire_closs",
    "col_fire_nloss",
    "somc_fire",
    "litfire",
    "somfire",
    "totfire",
    "m_leafc_to_fire",
    "m_frootc_to_fire",
    "m_livestemc_to_fire",
    "m_deadstemc_to_fire",
    "m_livecrootc_to_fire",
    "m_deadcrootc_to_fire",
    "m_leafc_storage_to_fire",
    "m_frootc_storage_to_fire",
    "m_livestemc_storage_to_fire",
    "m_deadstemc_storage_to_fire",
    "m_livecrootc_storage_to_fire",
    "m_deadcrootc_storage_to_fire",
    "m_gresp_storage_to_fire",
    "m_leafc_xfer_to_fire",
    "m_frootc_xfer_to_fire",
    "m_livestemc_xfer_to_fire",
    "m_deadstemc_xfer_to_fire",
    "m_livecrootc_xfer_to_fire",
    "m_deadcrootc_xfer_to_fire",
    "m_gresp_xfer_to_fire",
    "m_livestemc_to_deadstemc_fire",
    "m_livecrootc_to_deadcrootc_fire",
    "m_leafc_to_litter_fire",
    "m_frootc_to_litter_fire",
    "m_livestemc_to_litter_fire",
    "m_deadstemc_to_litter_fire",
    "m_livecrootc_to_litter_fire",
    "m_deadcrootc_to_litter_fire",
    "m_leafc_storage_to_litter_fire",
    "m_frootc_storage_to_litter_fire",
    "m_livestemc_storage_to_litter_fire",
    "m_deadstemc_storage_to_litter_fire",
    "m_livecrootc_storage_to_litter_fire",
    "m_deadcrootc_storage_to_litter_fire",
    "m_gresp_storage_to_litter_fire",
    "m_leafc_xfer_to_litter_fire",
    "m_frootc_xfer_to_litter_fire",
    "m_livestemc_xfer_to_litter_fire",
    "m_deadstemc_xfer_to_litter_fire",
    "m_livecrootc_xfer_to_litter_fire",
    "m_deadcrootc_xfer_to_litter_fire",
    "m_gresp_xfer_to_litter_fire",
    "m_leafn_to_fire",
    "m_frootn_to_fire",
    "m_livestemn_to_fire",
    "m_deadstemn_to_fire",
    "m_livecrootn_to_fire",
    "m_deadcrootn_to_fire",
    "m_leafn_storage_to_fire",
    "m_frootn_storage_to_fire",
    "m_livestemn_storage_to_fire",
    "m_deadstemn_storage_to_fire",
    "m_livecrootn_storage_to_fire",
    "m_deadcrootn_storage_to_fire",
    "m_leafn_xfer_to_fire",
    "m_frootn_xfer_to_fire",
    "m_livestemn_xfer_to_fire",
    "m_deadstemn_xfer_to_fire",
    "m_livecrootn_xfer_to_fire",
    "m_deadcrootn_xfer_to_fire",
    "m_livestemn_to_deadstemn_fire",
    "m_livecrootn_to_deadcrootn_fire",
    "m_retransn_to_fire",
    "m_leafn_to_litter_fire",
    "m_frootn_to_litter_fire",
    "m_livestemn_to_litter_fire",
    "m_deadstemn_to_litter_fire",
    "m_livecrootn_to_litter_fire",
    "m_deadcrootn_to_litter_fire",
    "m_leafn_storage_to_litter_fire",
    "m_frootn_storage_to_litter_fire",
    "m_livestemn_storage_to_litter_fire",
    "m_deadstemn_storage_to_litter_fire",
    "m_livecrootn_storage_to_litter_fire",
    "m_deadcrootn_storage_to_litter_fire",
    "m_leafn_xfer_to_litter_fire",
    "m_frootn_xfer_to_litter_fire",
    "m_livestemn_xfer_to_litter_fire",
    "m_deadstemn_xfer_to_litter_fire",
    "m_livecrootn_xfer_to_litter_fire",
    "m_deadcrootn_xfer_to_litter_fire",
    "m_retransn_to_litter_fire",
    "m_litr1_c_to_fire",
    "m_litr1_n_to_fire",
    "m_litr2_c_to_fire",
    "m_litr2_n_to_fire",
    "m_litr3_c_to_fire",
    "m_litr3_n_to_fire",
    "m_cwd_c_to_fire",
    "m_cwd_n_to_fire",
];

/// 只在 `DEF_USE_FIRE` 下累加的历史量。
fn fire_only(key: &str) -> bool {
    key == "lnfm"
        || FIRE_SOURCES.iter().any(|(name, _)| *name == key)
        || FIRE_DIAGNOSTICS.contains(&key)
}

/// 只为续跑旁车累加、Rust 的步输出里没有的量：上游 `accumulate_fluxes` 对每个 patch 每步都
/// `acc2d` 它们（`MOD_Vars_1DAccFluxes.F90:2496-2506`），而它们在一次运行里不变。
///
/// 旧的重启里可能没有这些变量：缺哪个就不累加哪个（旁车里是 `spval`）。
#[derive(Debug, Clone, Default)]
pub struct SidecarStatics {
    pub bulk_density: Option<Vec<f64>>,
    pub field_capacity: Option<Vec<f64>>,
    pub organic_matter_density: Option<Vec<f64>>,
    /// 非湖 patch 的 `t_lake`/`lake_icefrac`：没有湖过程改它们，一直是重启值。
    pub lake_temperature_k: Option<Vec<f64>>,
    pub lake_ice_fraction: Option<Vec<f64>>,
}

impl SidecarStatics {
    pub fn read(
        constant: &colm_init::RestartFile,
        time: &colm_init::RestartFile,
        patch: usize,
    ) -> Result<Self> {
        let column = |file: &colm_init::RestartFile, name: &str| -> Result<Option<Vec<f64>>> {
            let Ok(dims) = file.variable_dimensions(name) else {
                return Ok(None);
            };
            let layers = file.dimension(&dims[dims.len() - 1])?;
            file.layer_column(name, patch, layers).map(Some)
        };
        Ok(Self {
            bulk_density: column(constant, "BD_all")?,
            field_capacity: column(constant, "wfc")?,
            organic_matter_density: column(constant, "OM_density")?,
            lake_temperature_k: column(time, "t_lake")?,
            lake_ice_fraction: column(time, "lake_icefrc")?,
        })
    }
}

/// 各 patch 分支共用：只进续跑旁车的那几项（见 [`ACCUMULATED_ONLY`] 与 [`SidecarStatics`]）。
///
/// 湖 patch 的 `t_lake`/`lake_icefrac` 由湖分支自己写；其余 patch 写重启值并按 `patchtype == 4`
/// 的历史过滤标记。`t2m_wmo`：单点只有一个 patch、没有 WMO patch，恒等于 `tref`
/// （`MOD_Vars_1DAccFluxes.F90:2186-2196`）。
#[allow(clippy::too_many_arguments)]
fn set_sidecar_only(
    sink: &mut impl HistorySink,
    template: &StandardLctRestartTemplate,
    water: &colm_core::Water2014SoilState,
    lake_state: Option<&colm_core::RuntimeLakeState>,
    tref: f64,
    canopy_rain_mm: f64,
    canopy_snow_mm: f64,
    recharge_mm_s: Option<f64>,
) -> Result<()> {
    let statics = &template.sidecar_statics;
    let lake = template.patch_type == 4;
    // `wetwat` 每个 patch 都累加（`:2127`）；`f_wetwat*`/`f_wetzwt`（写的是 `a_zwt`）只在
    // 湿地上写（`MOD_Hist.F90` 的 `filter = patchtype == 2`）。
    {
        let wetland_filter: &'static [&'static str] = if template.patch_type == 2 {
            &[]
        } else {
            &["wetwat", "wetwat_inst", "wetzwt"]
        };
        let sink = &mut PatchFilteredSink {
            inner: &mut *sink,
            skipped: wetland_filter,
        };
        for (name, value) in [
            ("wetwat", water.wetland_water_mm),
            ("wetwat_inst", water.wetland_water_mm),
            ("wetzwt", water.water_table_depth_m),
        ] {
            sink.scalar(name, 0, value)?;
        }
    }
    // `qcharge`：`WATER_VSF` 从不给土壤/湿地赋值，那里一直是分配时的 `spval`，`acc1d` 跳过它；
    // 冰川/湖在 `patchtype > 2` 那一节清零（`CoLMMAIN.F90:2254`）。
    for (name, value) in [
        ("t2m_wmo", Some(tref)),
        ("ldew_rain", Some(canopy_rain_mm)),
        ("ldew_snow", Some(canopy_snow_mm)),
        ("qcharge", recharge_mm_s),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| (name, value)))
    {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, 0, value)?;
    }
    for (name, values) in [
        ("BD_all", &statics.bulk_density),
        ("wfc", &statics.field_capacity),
        ("OM_density", &statics.organic_matter_density),
    ] {
        if let Some(values) = values {
            sink.layer(name, 0, values)?;
        }
    }
    if !lake {
        let sink = &mut PatchFilteredSink {
            inner: sink,
            skipped: &["t_lake", "lake_icefrac"],
        };
        // 城市 patch 自带的水体每步演化（`CoLMMAIN_Urban` 的 `t_lake`/`lake_icefrac`）；
        // 其余非湖 patch 没有湖过程，一直是重启值。
        let (temperature, ice) = match lake_state {
            Some(lake) => (
                Some(&lake.column.temperature_k),
                Some(&lake.column.ice_fraction),
            ),
            None => (
                statics.lake_temperature_k.as_ref(),
                statics.lake_ice_fraction.as_ref(),
            ),
        };
        if let Some(values) = temperature {
            sink.layer("t_lake", 0, values)?;
        }
        if let Some(values) = ice {
            sink.layer("lake_icefrac", 0, values)?;
        }
    }
    Ok(())
}

/// `DEF_USE_OZONESTRESS` 的三个累加量：`acc1d(o3uptakesun/sha)`（`MOD_Vars_1DAccFluxes.F90:2616-2619`）与
/// `acc1d(forc_ozone, a_ozone)`（`:3002-3004`），对所有 patch 做。不开臭氧数据时，没被 `LeafTemperature`
/// 碰过的 patch 上 `forc_ozone` 是分配时的 `spval`（vendor 修补，upstream-bugs 第 70 条），`acc1d` 跳过。
fn set_ozone_history(
    sink: &mut impl HistorySink,
    ozone: Option<&colm_core::OzoneState>,
) -> Result<()> {
    let Some(ozone) = ozone else {
        return Ok(());
    };
    sink.scalar("o3uptakesun", 0, ozone.sunlit_uptake_mmol_m2)?;
    sink.scalar("o3uptakesha", 0, ozone.shaded_uptake_mmol_m2)?;
    sink.scalar("xy_ozone", 0, ozone.concentration_ppbv)
}

impl HistoryAccumulator {
    /// 导出当前区间的原始状态。本地正午量共用 `nac_ln`、`alb` 用 `nac_dt`：取它们自己的步数。
    fn window(&self) -> crate::history_sidecar::HistoryWindow {
        use crate::history_sidecar::WindowValue;
        let count_of = |name: &str| match self.sums.get(name) {
            Some(Accumulated::Scalar { count, .. } | Accumulated::Column { count, .. }) => *count,
            None => 0,
        };
        let sums = self
            .sums
            .iter()
            .filter(|(name, _)| !INSTANTANEOUS_VARIABLES.contains(&name.as_str()))
            .map(|(name, accumulated)| {
                let value = match accumulated {
                    Accumulated::Scalar { sum, .. } => WindowValue::Scalar(*sum),
                    Accumulated::Column { sum, .. } => WindowValue::Column(sum.clone()),
                };
                (name.clone(), value)
            })
            .collect();
        crate::history_sidecar::HistoryWindow {
            steps: self.steps,
            local_noon_steps: count_of("solvdln"),
            daytime_steps: count_of("alb"),
            sums,
        }
    }

    /// 由旁车窗口重建。计数只影响两件事：[`OWN_COUNT_VARIABLES`] 的除数（取 `nac_ln`/`nac_dt`），
    /// 以及"一次都没有效就不写出"（读回的量都有效过，取 `nac`）。
    fn from_window(window: &crate::history_sidecar::HistoryWindow) -> Self {
        use crate::history_sidecar::WindowValue;
        let count_for = |name: &str| {
            if name == "alb" {
                window.daytime_steps
            } else if OWN_COUNT_VARIABLES.contains(&name) {
                window.local_noon_steps
            } else {
                window.steps
            }
        };
        // 同一个上游累加器被 Rust 记在几个键下：`f_wetzwt` 写的就是 `a_zwt`，按作物类型分列的
        // `plantdate_*`/`huiswheat` 写的是 `a_plantdate`/`a_hui`。旁车只存累加器本身，读回时照抄。
        let aliases = std::iter::once(("wetzwt", "zwt"))
            .chain(
                CROP_TYPE_HISTORY
                    .iter()
                    .filter(|(field, source, _)| field != source)
                    .map(|(field, source, _)| (*field, *source)),
            )
            .filter_map(|(alias, source)| window.sums.get(source).map(|value| (alias, value)));
        let sums: std::collections::BTreeMap<String, Accumulated> = window
            .sums
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .chain(aliases)
            .map(|(name, value)| {
                let count = count_for(name);
                let accumulated = match value {
                    WindowValue::Scalar(sum) => Accumulated::Scalar { sum: *sum, count },
                    WindowValue::Column(sum) => Accumulated::Column {
                        sum: sum.clone(),
                        count,
                    },
                };
                (name.to_owned(), accumulated)
            })
            .collect();
        Self {
            offered: sums.keys().cloned().collect(),
            sums,
            steps: window.steps,
            filtered: std::collections::BTreeSet::new(),
        }
    }
}

impl HistorySink for HistoryAccumulator {
    fn scalar(&mut self, name: &str, _record: usize, value: f64) -> Result<()> {
        self.accumulate(name, 0, value, true)
    }

    fn keep_filtered(&mut self, name: &str) -> bool {
        self.filtered.insert(name.to_owned());
        true
    }

    fn accumulate(
        &mut self,
        name: &str,
        _record: usize,
        value: f64,
        counts_as_step: bool,
    ) -> Result<()> {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        self.offered.insert(name.to_owned());
        // `spval` 步不计入：既不进和，也不进计数（上游 `acc1d` 的 `IF (var(i) /= spval)`）。
        // 一步都不有效的变量因此**不会**在 `sums` 里建条目，也就不会被写出，
        // 缓冲区留给它的是填充值 —— 与上游一致。
        let assigned = ASSIGNED_VARIABLES.contains(&name);
        if value == colm_core::MISSING {
            // 赋值的量照样被 `spval` 覆盖：写出时是填充值。
            if assigned {
                self.sums.remove(name);
            }
            return Ok(());
        }
        let instantaneous = INSTANTANEOUS_VARIABLES.contains(&name) || assigned;
        // 累加器从 spval 起步、首个有效值直接赋值（`acc1d` 的 `IF (s /= spval) … ELSE s = var`），
        // 不是从 `+0.0` 起加：`0.0 + (-0.0)` 会把上游保留的 `-0.0` 变成 `+0.0`。
        match self
            .sums
            .entry(name.to_owned())
            .or_insert(Accumulated::Scalar {
                sum: colm_core::MISSING,
                count: 0,
            }) {
            Accumulated::Scalar { sum, count } => {
                if instantaneous {
                    // "最后一次覆盖"：`count` 保持 1，于是除数为 1、写出的是末步的值。
                    *sum = value;
                    *count = 1;
                } else {
                    *sum = if *sum == colm_core::MISSING {
                        value
                    } else {
                        *sum + value
                    };
                    // 一步里的多次 `acc1d`（短波四波段）只记一次步数。
                    if counts_as_step {
                        *count += 1;
                    }
                }
            }
            Accumulated::Column { .. } => {
                bail!("{name} was accumulated as a column, now as a scalar")
            }
        }
        Ok(())
    }

    fn layer(&mut self, name: &str, _record: usize, values: &[f64]) -> Result<()> {
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "the history value for {name} is not finite"
        );
        self.offered.insert(name.to_owned());
        // 分层量按**整列**是否有效来算（上游的计数器是每 patch 一个，
        // `nac_ln(i)`，不是每层一个）。整列全无效就整列跳过。
        if values.iter().all(|value| *value == colm_core::MISSING) {
            return Ok(());
        }
        let entry = self
            .sums
            .entry(name.to_owned())
            .or_insert_with(|| Accumulated::Column {
                sum: vec![colm_core::MISSING; values.len()],
                count: 0,
            });
        match entry {
            Accumulated::Column { sum, count } => {
                ensure!(
                    sum.len() == values.len(),
                    "{name} changed width between steps"
                );
                // 逐元素照 `acc2d`：spval 元素不入和，和为 spval 的元素首次直接赋值。
                for (sum, &value) in sum.iter_mut().zip(values) {
                    if value != colm_core::MISSING {
                        *sum = if *sum == colm_core::MISSING {
                            value
                        } else {
                            *sum + value
                        };
                    }
                }
                *count += 1;
            }
            Accumulated::Scalar { .. } => {
                bail!("{name} was accumulated as a scalar, now as a column")
            }
        }
        Ok(())
    }
}

/// 重算 history 的近地层诊断所需的**参考层**强迫量。
///
/// 它们不在步输出里（步输出只带 `tref`/`qref` 那种 2 m 诊断），但
/// `accumulate_fluxes` 的重算要用参考层的风、温、湿、气压与边界层高度。
#[derive(Debug, Clone, Copy)]
pub struct HistoryReferenceState {
    pub wind_speed_eastward_m_s: f64,
    pub wind_speed_northward_m_s: f64,
    pub air_temperature_k: f64,
    pub specific_humidity_kg_kg: f64,
    pub surface_pressure_pa: f64,
    pub boundary_layer_height_m: Option<f64>,
    /// `forc_sols`/`forc_soll`/`forc_solsd`/`forc_solld` —— **四个波段原样**带进来。
    ///
    /// 不要在这里先加起来再传：上游 `MOD_Vars_1DAccFluxes.F90:2060-2063` 是
    /// **四次 `acc1d` 累加到同一个 `a_solarin`**，而 `nac = nac + 1` 在 `:2038`
    /// 每步只执行一次 —— 一个两子步的小时是**八项从左到右连加**。先按步求和再
    /// 相加会差 1 ULP（数学上相同），而 `f_xy_solarin` 是 **tier0**：
    /// 实测干 11/264、湿 23/384、雪 22/360 条差 1 ULP，且四个波段的**小时均值
    /// 逐位相等**（`均值×2` 精确），所以差只能出在这一步的结合顺序上。
    /// 累加顺序固定 `sols → soll → solsd → solld`。
    pub direct_visible_w_m2: f64,
    pub direct_near_infrared_w_m2: f64,
    pub diffuse_visible_w_m2: f64,
    pub diffuse_near_infrared_w_m2: f64,
    /// `forc_frl`：`f_xy_frl` 照抄它。
    pub downward_longwave_w_m2: f64,
    /// 步末的太阳天顶角余弦（`CoLMMAIN` 那一份），即上游的 `coszen`。
    ///
    /// `MOD_Vars_1DAccFluxes` 的 `filter_dt(:) = coszen(:) > 0` 用的是它，
    /// 而 `f_alb` 是**只累加白天步**的（见 [`set_lct_albedo`]）。
    pub surface_cosine_zenith: f64,
    /// `forc_prc`：对流降水。
    pub convective_precipitation_kg_m2_s: f64,
    /// `forc_prl`：层状降水。
    pub large_scale_precipitation_kg_m2_s: f64,
    /// 上游的 `deltim`（`= DEF_simulation_time%timestep`），秒。
    ///
    /// 只有 `xerr` 用得到：`errorw` 是一条**质量**收支，要乘 `deltim` 才和蓄量同量纲。
    pub time_step_seconds: f64,
    /// 步首的 `totwb`（`CoLMMAIN.F90:831`），mm。
    ///
    /// 由调用方在**内核动手之前**从状态上取（见 `colm_core::total_water_storage_mm`）——
    /// 步末的值在内核跑完后已经无从还原，所以只能从外面递进来。
    pub initial_total_water_mm: f64,
}

impl HistoryReferenceState {
    /// 取本步的 `forc_*`。
    ///
    /// `time_step_seconds` 与 `initial_total_water_mm` 不是 forcing 的量，
    /// 但它们和 forcing 一样是"这一步的上下文"，且都必须在调用 kernel **之前**取好，
    /// 所以一并从这里进来 —— 分成两个构造函数只会让调用点更容易漏一个。
    pub fn from_forcing(
        forcing: &colm_core::RuntimeForcing,
        surface_cosine_zenith: f64,
        time_step_seconds: f64,
        initial_total_water_mm: f64,
    ) -> Self {
        Self {
            wind_speed_eastward_m_s: forcing.eastward_wind_m_s,
            wind_speed_northward_m_s: forcing.northward_wind_m_s,
            air_temperature_k: forcing.air_temperature_k,
            specific_humidity_kg_kg: forcing.specific_humidity,
            surface_pressure_pa: forcing.surface_pressure_pa,
            boundary_layer_height_m: forcing.boundary_layer_height_m,
            // 四个波段**原样**带出去，由 `set_lct_forcing_mirrors` 按上游的四次
            // `acc1d` 顺序累加：sols → soll → solsd → solld。
            direct_visible_w_m2: forcing.shortwave.direct_visible_w_m2,
            direct_near_infrared_w_m2: forcing.shortwave.direct_near_infrared_w_m2,
            diffuse_visible_w_m2: forcing.shortwave.diffuse_visible_w_m2,
            diffuse_near_infrared_w_m2: forcing.shortwave.diffuse_near_infrared_w_m2,
            downward_longwave_w_m2: forcing.downward_longwave_w_m2,
            convective_precipitation_kg_m2_s: forcing.convective_precipitation_kg_m2_s,
            large_scale_precipitation_kg_m2_s: forcing.large_scale_precipitation_kg_m2_s,
            surface_cosine_zenith,
            time_step_seconds,
            initial_total_water_mm,
        }
    }
}

/// 把一步的地表诊断写进第 `record` 条记录。
///
/// **`ustar`/`tstar`/`qstar`/`zol`/`rib`/`fm`/`fh`/`fq` 八项要重算，不取内核的
/// `leaf.*`。** 上游 `MOD_Vars_1DAccFluxes:accumulate_fluxes`
/// （`:2733-2790`）在累加前用参考层量重算一遍，并且换了位移高度
/// （`2/3*z0m/0.07`）、加了观测高度下限（`max(hgt, 5+displa)`）——所以这八列
/// 与**重启里的同名量本来就不是一回事**（实测首条记录 `f_ustar = 0.56451`
/// 而重启的 `ustar = 0.68583`）。见 `colm_core::history_diagnostics`。
///
/// `taux`/`tauy`/`tref`/`qref`/`z0m` 则确实来自模型状态，直接写。
pub fn set_lct_surface_diagnostics(
    sink: &mut impl HistorySink,
    record: usize,
    energy: &colm_core::StandardLctEnergyOutput,
    reference: HistoryReferenceState,
    physics: &LandPhysicsParameters,
) -> Result<()> {
    set_lct_surface_diagnostics_with(sink, record, energy, reference, physics, None)
}

/// 一个 patch 的近地面诊断重算输入（[`set_lct_surface_diagnostics`] 用的那一份）。
///
/// `fsena`/`fevpa` 的重算输入是**总量**（`fsenl + fseng`、`fevpl + fevpg`），
/// 其中地面那一半必须是订正后的值 —— 用叶温求解前的初步值会让 `f_ustar`
/// 这类量跟着偏（见 `CORRECTED_GROUND` 那条注释）。
pub fn lct_surface_input(
    energy: &colm_core::StandardLctEnergyOutput,
    reference: HistoryReferenceState,
    physics: &LandPhysicsParameters,
) -> colm_core::HistoryDiagnosticsInput {
    let leaf = &energy.leaf;
    colm_core::HistoryDiagnosticsInput {
        wind_height_m: physics.wind_height_m,
        temperature_height_m: physics.temperature_height_m,
        humidity_height_m: physics.humidity_height_m,
        wind_speed_eastward_m_s: reference.wind_speed_eastward_m_s,
        wind_speed_northward_m_s: reference.wind_speed_northward_m_s,
        air_temperature_k: reference.air_temperature_k,
        specific_humidity_kg_kg: reference.specific_humidity_kg_kg,
        surface_pressure_pa: reference.surface_pressure_pa,
        eastward_stress_kg_m_s2: leaf.eastward_stress_kg_m_s2,
        northward_stress_kg_m_s2: leaf.northward_stress_kg_m_s2,
        sensible_heat_w_m2: energy.total_sensible_heat_w_m2,
        evaporation_kg_m2_s: energy.total_evaporation_kg_m2_s,
        momentum_roughness_m: leaf.momentum_roughness_m,
        surface_layer_scheme: physics.surface_layer_scheme,
        boundary_layer_height_m: reference.boundary_layer_height_m,
    }
}

/// 多 patch 单点的网格元聚合（`MOD_Vars_1DAccFluxes.F90:2698-2720`）：各输入按 patch 面积份额
/// `subfrc` 加权——GIMPLE 是从 0 起的 `FMA(x, subfrc, acc)` 链——再除以 `sumwt`（从 0 起逐项相加）。
/// 相同的强迫量也照样聚合：链与除法之后未必逐位回到原值。
pub fn element_surface_input(
    patches: &[(colm_core::HistoryDiagnosticsInput, f64)],
) -> Result<colm_core::HistoryDiagnosticsInput> {
    let (first, _) = patches
        .first()
        .context("an element needs at least one patch")?;
    let weight: f64 = patches
        .iter()
        .fold(0.0, |sum, (_, fraction)| sum + fraction);
    let mean = |value: fn(&colm_core::HistoryDiagnosticsInput) -> f64| {
        patches.iter().fold(0.0, |sum, (input, fraction)| {
            value(input).mul_add(*fraction, sum)
        }) / weight
    };
    Ok(colm_core::HistoryDiagnosticsInput {
        wind_height_m: mean(|input| input.wind_height_m),
        temperature_height_m: mean(|input| input.temperature_height_m),
        humidity_height_m: mean(|input| input.humidity_height_m),
        wind_speed_eastward_m_s: mean(|input| input.wind_speed_eastward_m_s),
        wind_speed_northward_m_s: mean(|input| input.wind_speed_northward_m_s),
        air_temperature_k: mean(|input| input.air_temperature_k),
        specific_humidity_kg_kg: mean(|input| input.specific_humidity_kg_kg),
        surface_pressure_pa: mean(|input| input.surface_pressure_pa),
        eastward_stress_kg_m_s2: mean(|input| input.eastward_stress_kg_m_s2),
        northward_stress_kg_m_s2: mean(|input| input.northward_stress_kg_m_s2),
        sensible_heat_w_m2: mean(|input| input.sensible_heat_w_m2),
        evaporation_kg_m2_s: mean(|input| input.evaporation_kg_m2_s),
        momentum_roughness_m: mean(|input| input.momentum_roughness_m),
        surface_layer_scheme: first.surface_layer_scheme,
        boundary_layer_height_m: first
            .boundary_layer_height_m
            .map(|_| mean(|input| input.boundary_layer_height_m.unwrap_or(0.0))),
    })
}

/// 城市 patch 交给近地面诊断的那组通量（`qref` 取城市出口）。
fn urban_surface_fluxes(output: &colm_core::UrbanStepOutput) -> colm_core::GlacierThermalFluxes {
    let thermal = &output.thermal;
    colm_core::GlacierThermalFluxes {
        taux: thermal.taux,
        tauy: thermal.tauy,
        fsena: thermal.fsena,
        fevpa: thermal.fevpa,
        tref: thermal.tref,
        qref: output.qref,
        z0m: thermal.z0m,
        ..Default::default()
    }
}

/// 任一种 patch 交给网格元聚合的近地面诊断输入（[`element_surface_input`] 的一项）。
pub fn patch_surface_input(
    template: &StandardLctRestartTemplate,
    output: &crate::PatchOutput,
    reference: HistoryReferenceState,
) -> colm_core::HistoryDiagnosticsInput {
    let physics = &template.physics;
    match output {
        crate::PatchOutput::Soil(output) | crate::PatchOutput::DryLakeSubstep { output, .. } => {
            lct_surface_input(&output.energy, reference, physics)
        }
        crate::PatchOutput::Glacier(output) => {
            glacier_surface_input(&output.thermal, reference, physics)
        }
        crate::PatchOutput::Lake(output) => {
            let thermal = &output.thermal;
            let surface = colm_core::GlacierThermalFluxes {
                taux: thermal.taux,
                tauy: thermal.tauy,
                fsena: thermal.fsena,
                fevpa: thermal.fevpa,
                z0m: thermal.z0m,
                ..Default::default()
            };
            glacier_surface_input(&surface, reference, physics)
        }
        crate::PatchOutput::Urban(output) => {
            glacier_surface_input(&urban_surface_fluxes(output), reference, physics)
        }
    }
}

/// [`set_lct_surface_diagnostics`]；`element` 给了就用网格元那一份重算结果（多 patch 单点）。
pub fn set_lct_surface_diagnostics_with(
    sink: &mut impl HistorySink,
    record: usize,
    energy: &colm_core::StandardLctEnergyOutput,
    reference: HistoryReferenceState,
    physics: &LandPhysicsParameters,
    element: Option<&colm_core::HistoryDiagnostics>,
) -> Result<()> {
    let leaf = &energy.leaf;
    let recomputed = match element {
        Some(element) => *element,
        None => colm_core::history_diagnostics(lct_surface_input(energy, reference, physics))
            .context("cannot recompute the history near-surface diagnostics")?,
    };
    for (name, value) in [
        ("taux", leaf.eastward_stress_kg_m_s2),
        ("tauy", leaf.northward_stress_kg_m_s2),
        ("tref", leaf.air_temperature_2m_k),
        ("qref", leaf.air_specific_humidity_2m),
        ("z0m", leaf.momentum_roughness_m),
        ("zol", recomputed.zol),
        ("rib", recomputed.bulk_richardson),
        ("ustar", recomputed.friction_velocity_m_s),
        ("qstar", recomputed.humidity_scale),
        ("tstar", recomputed.temperature_scale_k),
        ("fm", recomputed.momentum_similarity),
        ("fh", recomputed.heat_similarity),
        ("fq", recomputed.moisture_similarity),
        // 10 m 四项来自**同一次** MO 调用，见 `LCT_SIMILARITY_10M_VARIABLES`。
        ("us10m", recomputed.wind_10m_eastward_m_s),
        ("vs10m", recomputed.wind_10m_northward_m_s),
        ("fm10m", recomputed.momentum_at_10m),
        ("ustar2", recomputed.similarity_friction_velocity_m_s),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的能量侧诊断写进第 `record` 条记录。
///
/// 两支共用：积雪分支传整个输出，`leaf`/`shortwave`/总通量都在里面。
/// `MOD_Hist.F90:4396-4405` 的 `filter = patchtype <= 2`：这一组只在土壤/城市/湿地
/// patch 上写出，其余 patch 留 `spval`。
const VEGETATED_ONLY_VARIABLES: [&str; 5] = ["h2osoi", "qlayer", "rootr", "vegwp", "zwt"];

/// 湖上再多滤掉三根雪 + 土柱：`MOD_Hist.F90:4160` 的 `filter = patchtype <= 3`。
const LAKE_FILTERED_VARIABLES: [&str; 8] = [
    "h2osoi",
    "qlayer",
    "rootr",
    "vegwp",
    "zwt",
    "t_soisno",
    "wliq_soisno",
    "wice_soisno",
];

/// 冰川/湖共用的一步输出视图。
struct NonSoilStep<'a> {
    precipitation: &'a colm_core::PrecipitationState,
    shortwave: &'a colm_core::NetSolarFluxes,
    thermal: colm_core::GlacierThermalFluxes,
    surface_runoff_mm_s: f64,
    total_runoff_mm_s: f64,
    /// 已按上游规则取好的 `xerr`（冰川非 VSF、非动态湖都是 0）。
    water_balance_error_mm_s: f64,
    /// 这一支写不写 `rsur_se`/`rsur_ie`：冰川只在 VSF 下写（`CoLMMAIN.F90:1720-1740`），
    /// 湖分支无条件写 `rsur_se = rsur`、`rsur_ie = 0`（`:1937-1940`）。
    writes_runoff_split: bool,
}

/// 只在湖 patch 上写的量。
enum LakeLayers<'a> {
    Scalar(&'static str, f64),
    Layers(&'static str, &'a [f64]),
}

/// 整个丢掉列出的变量（不累加），其余原样转交：用于"同一个量改由调用方另写"的场合。
struct DroppedSink<'a, S: HistorySink> {
    inner: &'a mut S,
    dropped: &'static [&'static str],
}

impl<S: HistorySink> HistorySink for DroppedSink<'_, S> {
    fn scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        if self.dropped.contains(&name) {
            return Ok(());
        }
        self.inner.scalar(name, record, value)
    }

    fn layer(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        if self.dropped.contains(&name) {
            return Ok(());
        }
        self.inner.layer(name, record, values)
    }

    fn accumulate(
        &mut self,
        name: &str,
        record: usize,
        value: f64,
        counts_as_step: bool,
    ) -> Result<()> {
        if self.dropped.contains(&name) {
            return Ok(());
        }
        self.inner.accumulate(name, record, value, counts_as_step)
    }

    fn keep_filtered(&mut self, name: &str) -> bool {
        self.inner.keep_filtered(name)
    }
}

/// 按 `patchtype` 过滤的变量：累加器收下原值、写平均时跳过；直接写缓冲的丢弃。
struct PatchFilteredSink<'a, S: HistorySink> {
    inner: &'a mut S,
    skipped: &'static [&'static str],
}

impl<S: HistorySink> PatchFilteredSink<'_, S> {
    /// 过滤掉的量交给内层决定：累加器收下原值（写出时再跳过），直接写缓冲的丢弃。
    fn passes(&mut self, name: &str) -> bool {
        !self.skipped.contains(&name) || self.inner.keep_filtered(name)
    }
}

impl<S: HistorySink> HistorySink for PatchFilteredSink<'_, S> {
    fn scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        if !self.passes(name) {
            return Ok(());
        }
        self.inner.scalar(name, record, value)
    }

    fn layer(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        if !self.passes(name) {
            return Ok(());
        }
        self.inner.layer(name, record, values)
    }

    fn accumulate(
        &mut self,
        name: &str,
        record: usize,
        value: f64,
        counts_as_step: bool,
    ) -> Result<()> {
        if !self.passes(name) {
            return Ok(());
        }
        self.inner.accumulate(name, record, value, counts_as_step)
    }

    fn keep_filtered(&mut self, name: &str) -> bool {
        self.inner.keep_filtered(name)
    }
}

/// 被强迫缺测遮蔽的 patch 上 BGC 只累加"状态"一类（g1bgcm 实测，第 541 轮）：
/// - 从续跑读进来的汇总量与分层库：`tot*`、`*_vr`（含 `sminn_vr`；`totsoiln_vr` 只在土壤 patch）、O2 两项；
/// - 读数据时整列更新的驱动：`ndep_to_sminn` 与火灾的 `abm/gdp/peatf/hdm/lnfm`。
///
/// 其余（`gpp`、`leafc` 等在 `CoLMDRIVER` 里才算的量）停在分配时的 `spval`，`acc1d` 跳过。
struct MaskedBgcSink<'a, S: HistorySink> {
    inner: &'a mut S,
    soil: bool,
}

impl<S: HistorySink> MaskedBgcSink<'_, S> {
    fn passes(&self, name: &str) -> bool {
        const STATES: [&str; 17] = [
            "totvegc",
            "totlitc",
            "totcwdc",
            "totsomc",
            "totcolc",
            "totvegn",
            "totlitn",
            "totcwdn",
            "totsomn",
            "totcoln",
            "CONC_O2_UNSAT",
            "O2_DECOMP_DEPTH_UNSAT",
            "ndep_to_sminn",
            "abm",
            "gdp",
            "peatf",
            "hdm",
        ];
        if name == "totsoiln_vr" {
            return self.soil;
        }
        // CROP：`CROP_readin` 与作物汇总对整列赋值的量（g1cropm 实测，第 542 轮）。
        let crop = name == "cphase"
            || name == "pdrice2"
            || name.starts_with("fertnitro_")
            || name.starts_with("irrig_method_");
        STATES.contains(&name) || name == "lnfm" || name.ends_with("_vr") || crop
    }
}

impl<S: HistorySink> HistorySink for MaskedBgcSink<'_, S> {
    fn scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        if !self.passes(name) {
            return Ok(());
        }
        self.inner.scalar(name, record, value)
    }

    fn layer(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        if !self.passes(name) {
            return Ok(());
        }
        self.inner.layer(name, record, values)
    }

    fn accumulate(
        &mut self,
        name: &str,
        record: usize,
        value: f64,
        counts_as_step: bool,
    ) -> Result<()> {
        if !self.passes(name) {
            return Ok(());
        }
        self.inner.accumulate(name, record, value, counts_as_step)
    }

    fn keep_filtered(&mut self, name: &str) -> bool {
        self.inner.keep_filtered(name)
    }
}

/// 冰川的近地层诊断：`taux`/`tauy`/`tref`/`qref`/`z0m` 取 `GLACIER_TEMP`，其余与植被
/// 分支一样由 `accumulate_fluxes` 从 `taux`/`tauy`/`fsena`/`fevpa`/`z0m` 重算。
fn set_glacier_surface_diagnostics(
    sink: &mut impl HistorySink,
    thermal: &colm_core::GlacierThermalFluxes,
    reference: HistoryReferenceState,
    physics: &LandPhysicsParameters,
    element: Option<&colm_core::HistoryDiagnostics>,
) -> Result<()> {
    let recomputed = match element {
        Some(element) => *element,
        None => colm_core::history_diagnostics(glacier_surface_input(thermal, reference, physics))
            .context("cannot recompute the glacier history near-surface diagnostics")?,
    };
    set_glacier_surface_values(sink, thermal, &recomputed)
}

/// 非土壤 patch（冰川、湖、城市）的近地面诊断重算输入：`taux`/`tauy`/`fsena`/`fevpa`/`z0m` 取
/// 各自热通量的出口值。
fn glacier_surface_input(
    thermal: &colm_core::GlacierThermalFluxes,
    reference: HistoryReferenceState,
    physics: &LandPhysicsParameters,
) -> colm_core::HistoryDiagnosticsInput {
    colm_core::HistoryDiagnosticsInput {
        wind_height_m: physics.wind_height_m,
        temperature_height_m: physics.temperature_height_m,
        humidity_height_m: physics.humidity_height_m,
        wind_speed_eastward_m_s: reference.wind_speed_eastward_m_s,
        wind_speed_northward_m_s: reference.wind_speed_northward_m_s,
        air_temperature_k: reference.air_temperature_k,
        specific_humidity_kg_kg: reference.specific_humidity_kg_kg,
        surface_pressure_pa: reference.surface_pressure_pa,
        eastward_stress_kg_m_s2: thermal.taux,
        northward_stress_kg_m_s2: thermal.tauy,
        sensible_heat_w_m2: thermal.fsena,
        evaporation_kg_m2_s: thermal.fevpa,
        momentum_roughness_m: thermal.z0m,
        surface_layer_scheme: physics.surface_layer_scheme,
        boundary_layer_height_m: reference.boundary_layer_height_m,
    }
}

fn set_glacier_surface_values(
    sink: &mut impl HistorySink,
    thermal: &colm_core::GlacierThermalFluxes,
    recomputed: &colm_core::HistoryDiagnostics,
) -> Result<()> {
    for (name, value) in [
        ("taux", thermal.taux),
        ("tauy", thermal.tauy),
        ("tref", thermal.tref),
        ("qref", thermal.qref),
        ("z0m", thermal.z0m),
        ("zol", recomputed.zol),
        ("rib", recomputed.bulk_richardson),
        ("ustar", recomputed.friction_velocity_m_s),
        ("qstar", recomputed.humidity_scale),
        ("tstar", recomputed.temperature_scale_k),
        ("fm", recomputed.momentum_similarity),
        ("fh", recomputed.heat_similarity),
        ("fq", recomputed.moisture_similarity),
        ("us10m", recomputed.wind_10m_eastward_m_s),
        ("vs10m", recomputed.wind_10m_northward_m_s),
        ("fm10m", recomputed.momentum_at_10m),
        ("ustar2", recomputed.similarity_friction_velocity_m_s),
    ] {
        ensure!(
            value.is_finite(),
            "the glacier history value for {name} is not finite"
        );
        sink.scalar(name, 0, value)?;
    }
    Ok(())
}

pub fn set_lct_energy_fluxes(
    sink: &mut impl HistorySink,
    record: usize,
    output: &StandardLctSoilOutput,
) -> Result<()> {
    for (name, value) in [
        ("fsena", output.energy.total_sensible_heat_w_m2),
        ("fevpa", output.energy.total_evaporation_kg_m2_s),
        ("etr", output.energy.leaf.transpiration_kg_m2_s),
        ("sabg", output.energy.shortwave.ground_absorbed_w_m2),
        ("fsenl", output.energy.leaf.leaf_sensible_heat_w_m2),
        // 地面那一半要取**订正后**的：内核的 `total = leaf + corrected_ground`
        // （`standard_lct_step.rs:351`），而 `leaf.ground_sensible_heat_w_m2` 是
        // 叶温求解前的初步值。写初步值会让 `fsenl + fseng != fsena` ——
        // 实测 CN-Cng 首条记录 Fortran 是 32.183 + 670.575 = 702.758，
        // 而写初步值的 Rust 是 28.841 + 962.618 = 991.459 ≠ 702.041。
        ("fseng", output.energy.corrected_ground_sensible_heat_w_m2),
        // `leaf_evaporation` 在核心里就是 `transpiration + wet_evaporation`，
        // 与上游 `fevpl = etr + evplwet` 同一个量，**不要再加一次 `etr`**。
        ("fevpl", output.energy.leaf.leaf_evaporation_kg_m2_s),
        ("fevpg", output.energy.corrected_ground_evaporation_kg_m2_s),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的**地表能量收支**写进第 `record` 条记录。
///
/// 上游每一项都在 `MOD_Thermal.F90` 的收尾处算出（`htvp = hvap + hfus`）：
///
/// ```fortran
/// olrg   = ulrad + 4.*emg*stefnc*t_grnd_bef**3*tinc
/// olrb   = stefnc*t_grnd_bef**3*(4.*tinc)
/// emis   = (ulrad + emg*olrb) / (ulrad + olrb)
/// trad   = (olrg/stefnc)**0.25
/// fgrnd  = sabg + dlrad*emg - emg*stefnc*t_grnd_bef**4            &
///          - emg*stefnc*t_grnd_bef**3*(4.*tinc) - (fseng+fevpg*htvp) &
///          + cpliq*pg_rain*(t_precip-t_grnd) + cpice*pg_snow*(t_precip-t_grnd)
/// lfevpa = hvap*fevpl + htvp*fevpg
/// rnet   = sabg + sabvsun + sabvsha - olrg + forc_frl
/// ```
///
/// `tinc = t_grnd - t_grnd_bef` 用状态里新留的 `previous_temperature_k`；
/// 地表层在**打包列**里的下标是 `snow_layers`（雪层在前）。
///
/// `emis` 在这里是**平均体积发射率**，与算例里那个固定的地表发射率不是一回事；
/// 平衡状态下 `tinc` 很小，所以它接近 1 而不是 0.96/0.97 —— 实测黄金 history 里
/// 均值 1.0000 正是如此。
pub fn set_lct_surface_budget(
    sink: &mut impl HistorySink,
    record: usize,
    output: &StandardLctSoilOutput,
) -> Result<()> {
    let budget = surface_budget(&output.energy)?;
    let energy = &output.energy;
    for (name, value) in [
        ("sabvsun", energy.shortwave.sunlit_absorbed_w_m2),
        ("sabvsha", energy.shortwave.shaded_absorbed_w_m2),
        ("rnet", budget.net_radiation_w_m2),
        ("olrg", budget.outgoing_longwave_w_m2),
        ("emis", budget.bulk_emissivity),
        ("trad", budget.radiative_temperature_k),
        ("fgrnd", budget.ground_heat_w_m2),
        ("lfevpa", budget.latent_heat_w_m2),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}
/// 把一步的**收支残差**写进第 `record` 条记录。
///
/// 两个量都是上游自己拼的闭合性检查，既不是状态量，也没有任何一步输出能顶替。
/// 项表逐条搬自 **`main/MOD_Thermal.F90:1534-1543`**（`zerr`；同步到 CoLM-SYSU-integration@3c799bae
/// 之后内核不再编译 `extends/interception` 那一份，它多出的 `canopy_phase_heat` 一项随之去掉）
/// 与 `CoLMMAIN.F90:1529-1543`（`xerr`）：
///
/// | `zerr` 的项 | 上游 | 本仓库 |
/// |---|---|---|
/// | `sabv` | `sabvsun + sabvsha`（`:657`） | `shortwave.sunlit_absorbed_w_m2 + shaded_absorbed_w_m2` |
/// | `sabg` | 地面吸收短波 | `shortwave.ground_absorbed_w_m2` |
/// | `frl` | THERMAL 的**入参** = `forc_frl` | `reference.downward_longwave_w_m2` |
/// | `olrg` | `:1353` | [`SurfaceBudget::outgoing_longwave_w_m2`] |
/// | `fsena` | `fsenl + fseng`（`:1331`） | `total_sensible_heat_w_m2` |
/// | `lfevpa` | `hvap*fevpl + htvp*fevpg`（`:1333`） | [`SurfaceBudget::latent_heat_w_m2`] |
/// | `xmf` | `MOD_PhaseChange` 的相变潜热 | `ground.latent_heat_flux_w_m2` |
/// | `dheatl` | `sum(dheatl_p*pftfrac)`（`:1122`） | `leaf.canopy_heat_storage_w_m2` |
/// | `hprl` | `sum(hprl_p*pftfrac)`（`:1121`） | `leaf.precipitation_heat_w_m2` |
/// | 降水显热两项 | `:1539` | [`colm_core::add_precipitation_heat`]（**逐项熔进**累加值） |
/// | `Σ(t-t_bef)/fact` | `:1541-1543`，`j = lb:nl_soil` | 下面那条三列 `zip` |
///
/// **`frl` 是大气向下长波（`forc_frl`），不是冠层下方的 `dlrad`。** 两者只在
/// `MOD_Thermal.F90:517`（无冠层）相等；有冠层时 `dlrad` 多乘一份透过率。
/// `fgrnd` 用的是 `dlrad*emg`，这条收支用的是 `frl` —— 混用会差几十 W/m²。
///
/// **`:1534` 那一行 `errore`（减 `fgrnd`）是死代码**：`:1538` 立刻用减 `xmf` 的版本
/// 覆盖它。两条式子里的 `fgrnd` 与 `xmf` **不是同一个量**（前者含地面辐射收支，
/// 后者只有相变潜热），所以照抄第二行时不能把 `xmf` 换成 `fgrnd`。
///
/// 三个温度列的求和范围是**整根打包列**（雪层 + 土层）：上游写的是
/// `DO j = lb, nl_soil`，而 `lb` 就是最下一层雪的下标，合起来正是全列。
///
/// `xerr = errorw/deltim`，`errorw = (endwb - totwb) - (forc_prc + forc_prl - fevpa - rnof)*deltim`
/// （`#ifndef CatchLateralFlow` 那一支，也就是本仓库的构建配置）。`totwb` 只能由调用方
/// 在内核动手**之前**取好（[`HistoryReferenceState::initial_total_water_mm`]）；
/// `endwb` 就是这里传进来的 `end_water_storage_mm`，与 [`set_lct_water_storage`] 的
/// `wat` **不是**一条算式（`wat` 不含 `wdsrf`），别把两者合并。
pub fn set_lct_balance_errors(
    sink: &mut impl HistorySink,
    record: usize,
    output: &StandardLctSoilOutput,
    end_water_storage_mm: f64,
    reference: HistoryReferenceState,
) -> Result<()> {
    let budget = surface_budget(&output.energy)?;
    let energy = &output.energy;
    let ground = &energy.ground;
    ensure!(
        ground.temperature_k.len() == ground.previous_temperature_k.len()
            && ground.temperature_k.len() == ground.layer_factor_seconds_per_j_m2_k.len(),
        "the ground temperature state disagrees on depth between the current, previous \
         and factor columns"
    );
    // `MOD_Thermal.F90:1534-1543` 的 `errore`：前两个赋值里
    // 第一个是**死代码**（立刻被第二个覆盖，只差 `fgrnd` vs `xmf`），真正的链条
    // 以**逐层边加边减**收尾：
    //   `DO j = lb, nl_soil ; errore = errore - (t_soisno(j)-t_soisno_bef(j))/fact(j)`
    // 所以不是"先求和再整体减"，而是 `((…((T - a₁) - a₂) …) - aₙ)`。
    // 原实现先 `.sum()` 再 `- ground_heat_storage_w_m2`，结合顺序不同 ——
    // 这是干窗第 0 步最后一个差异（`f_zerr`，`errore` 本身 ~2.6e-11、差 8.9e-14）
    // 的第一候选。
    let zerr = energy.shortwave.sunlit_absorbed_w_m2
        + energy.shortwave.shaded_absorbed_w_m2
        + energy.shortwave.ground_absorbed_w_m2
        + reference.downward_longwave_w_m2
        - budget.outgoing_longwave_w_m2
        - energy.total_sensible_heat_w_m2
        - budget.latent_heat_w_m2
        - ground.latent_heat_flux_w_m2
        - energy.leaf.canopy_heat_storage_w_m2
        + energy.leaf.precipitation_heat_w_m2;
    // 降水显热两项：内核把**每一项**分别熔进当时的累加值（`_1871`/`_1872`），
    // 不是先求和再加 —— 与 `fgrnd` 共用 `add_precipitation_heat`。
    let mut zerr = colm_core::add_precipitation_heat(energy, zerr);
    for ((now, before), factor) in ground
        .temperature_k
        .iter()
        .zip(&ground.previous_temperature_k)
        .zip(&ground.layer_factor_seconds_per_j_m2_k)
    {
        zerr -= (now - before) / factor;
    }

    // 漫滩回馈（`CoLMMAIN.F90:1505-1521`，土壤 patch）：再入渗 `qinfl_fld` 作为显式输入项、
    // 漫滩蒸发 `fevpg_fld` 从 `fevpa` 里扣掉。GIMPLE（GridRiverLakeFlow 内核）：
    // `FNMA(deltim, (((prc+prl) + flood_input_wb) - fevpa_wb) - rnof, endwb-totwb)`，
    // `fevpa_wb = fevpa - fevpg_fld`。没开回馈时两项都是 0，与默认内核的
    // `((prc+prl)+0.0) - fevpa - rnof` 逐位相同。
    let flood_infiltration = output.water.flood_infiltration_mm_s;
    let flood_evaporation = energy.flood.map_or(0.0, |flood| flood.evaporation_mm_s);
    let dt = reference.time_step_seconds;
    let evaporation_wb = energy.total_evaporation_kg_m2_s - flood_evaporation;
    let errorw = (-((((reference.convective_precipitation_kg_m2_s
        + reference.large_scale_precipitation_kg_m2_s)
        + flood_infiltration)
        - evaporation_wb)
        - output.water.total_runoff_mm_s))
        .mul_add(dt, end_water_storage_mm - reference.initial_total_water_mm);
    let xerr = errorw / reference.time_step_seconds;

    for (name, value) in [("xerr", xerr), ("zerr", zerr)] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的冠层光合/气孔链诊断写进第 `record` 条记录。
///
/// 取值来源与 [`set_lct_surface_diagnostics`] 同一次 `LeafTemperature` 出口，
/// 所以叶温、气孔阻力与这里的同化/蒸腾永远是同一迭代的一致快照。
/// 见 [`LCT_STOMATAL_VARIABLES`] 对单位与维度顺序的说明。
pub fn set_lct_stomatal_diagnostics(
    sink: &mut impl HistorySink,
    record: usize,
    energy: &colm_core::StandardLctEnergyOutput,
) -> Result<()> {
    let leaf = &energy.leaf;
    for (name, value) in [
        ("assim", leaf.assimilation_mol_m2_s),
        ("assimsun", leaf.sunlit_assimilation_mol_m2_s),
        ("assimsha", leaf.shaded_assimilation_mol_m2_s),
        ("respc", leaf.respiration_mol_m2_s),
        ("etrsun", leaf.sunlit_transpiration_kg_m2_s),
        ("etrsha", leaf.shaded_transpiration_kg_m2_s),
        ("gssun", leaf.sunlit_stomatal_conductance_mol_m2_s),
        ("gssha", leaf.shaded_stomatal_conductance_mol_m2_s),
        // 这两个**不能**再取 `energy.root_uptake.soil_water_stress`：那一个是
        // `eroot`，即 PHS 进来之前的胁迫。上游的 `rstfacsun`/`rstfacsha` 是
        // `intent(inout)`，PHS 会把它重写成自己的因子，`stomata` 只读不写，
        // 所以 history 记的是 PHS 那一份。LCT 分支非 PHS 时两行同源，打开后不同源。
        ("rstfacsun", leaf.sunlit_soil_water_stress),
        ("rstfacsha", leaf.shaded_soil_water_stress),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    sink.layer("rootr", record, &energy.root_uptake.layer_fraction)?;
    Ok(())
}

/// 把一步的土壤表面阻力写进第 `record` 条记录。
///
/// 取值见 [`LCT_SOIL_RESISTANCE_VARIABLES`]，直接取内核输出，不做换算。
pub fn set_lct_soil_resistance(
    sink: &mut impl HistorySink,
    record: usize,
    energy: &colm_core::StandardLctEnergyOutput,
) -> Result<()> {
    let value = energy.soil_surface_resistance_s_m;
    ensure!(value.is_finite(), "the history value for rss is not finite");
    sink.scalar("rss", record, value)?;
    Ok(())
}

/// 把一步的冠层截留量写进第 `record` 条记录。
///
/// 定义见 [`LCT_CANOPY_WATER_VARIABLES`]；三项都是区间平均。
pub fn set_lct_canopy_water(
    sink: &mut impl HistorySink,
    record: usize,
    state: &colm_core::StandardLctEnergyState,
    energy: &colm_core::StandardLctEnergyOutput,
) -> Result<()> {
    let interception = &energy.interception;
    for (name, value) in [
        // `ldew` 是**状态**（冠层持水），不在步输出上；通量三项在输出上。
        ("ldew", state.leaf.canopy_water.total_mm),
        ("qintr", interception.retained_kg_m2_s),
        (
            "qdrip",
            interception.ground_rain_kg_m2_s + interception.ground_snow_kg_m2_s,
        ),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的瞬时水量诊断写进第 `record` 条记录。
///
/// `wat` 的算式见 [`LCT_WATER_STORAGE_VARIABLES`]：`sum(wliq+wice) + ldew + scv + 末项`，
/// 末项在 VSF 打开时是 `wetwat`、关着时是 `wa`（`CoLMMAIN.F90:2262-2266`），由调用方
/// 按分支传入 `storage_tail_mm`。液体与冰分开求和再相加，
/// 与上游 `sum(wice_soisno(1:)+wliq_soisno(1:))` 的元素级相加**不是**
/// 同一个结合顺序，所以这里也按元素级累加，别改成两个 `sum()` 相减。
pub fn set_lct_water_storage(
    sink: &mut impl HistorySink,
    record: usize,
    water: &colm_core::Water2014SoilState,
    canopy_water_mm: f64,
    snow_water_equivalent_kg_m2: f64,
    storage_tail_mm: f64,
) -> Result<()> {
    let total = lct_water_total(
        water,
        canopy_water_mm,
        snow_water_equivalent_kg_m2,
        storage_tail_mm,
    )?;
    set_water_storage_with_total(sink, record, water, total)
}

/// [`set_lct_water_storage`] 的 `wat`。
fn lct_water_total(
    water: &colm_core::Water2014SoilState,
    canopy_water_mm: f64,
    snow_water_equivalent_kg_m2: f64,
    storage_tail_mm: f64,
) -> Result<f64> {
    ensure!(
        water.liquid_water_kg_m2.len() == water.ice_water_kg_m2.len(),
        "the soil water columns disagree on depth"
    );
    let soil: f64 = water
        .liquid_water_kg_m2
        .iter()
        .zip(&water.ice_water_kg_m2)
        .map(|(wliq, wice)| wliq + wice)
        .sum();
    Ok(soil + canopy_water_mm + snow_water_equivalent_kg_m2 + storage_tail_mm)
}

/// [`set_lct_water_storage`] 的后半：`wat` 已由 patch 自己算好时直接写。
///
/// 城市 patch 的 `wat` 是 `CoLMMAIN_Urban.F90:1323-1324` 按屋顶/透水/不透水加权后
/// 再加 `wa*(1-froof)*fgper`，不能按土壤 patch 的公式从聚合后的 `wliq_soisno` 重算。
pub fn set_water_storage_with_total(
    sink: &mut impl HistorySink,
    record: usize,
    water: &colm_core::Water2014SoilState,
    total: f64,
) -> Result<()> {
    for (name, value) in [
        ("wa_inst", water.aquifer_water_mm),
        ("wdsrf_inst", water.surface_water_mm),
        ("wat_inst", total),
        // 同一算式，但 `wat` 不在 `INSTANTANEOUS_VARIABLES` 里，所以走区间平均。
        ("wat", total),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的宽带反照率写进第 `record` 条记录。
///
/// 排列顺序见 [`LCT_ALBEDO_VARIABLES`]：**盘上是 `rtyp` 快于 `band`**，
/// 与内核的 `[band][rtyp]` 相反，所以这里是一次显式的换序而不是直接展开。
/// **只累加白天步。** 上游那一处与其它变量不同：
/// `CALL acc3d (alb, a_alb, filter_dt)`（`MOD_Vars_1DAccFluxes.F90:2152`），
/// 而 `filter_dt = coszen > 0`、除数又是 `nac_dt`（白天步数，`:2044`），
/// 写出时还用 `filter_dt`/`nac_dt` 而不是 `filter`/`nac`（`MOD_Hist.F90:769-772`）。
/// 三者合起来的效果是：**夜间步既不入和也不入计数，整条记录都是夜间的就留填充值**
/// —— 正是本层累加器对 `MISSING` 的处理，所以这里把夜间步写成 `MISSING` 就够了，
/// 不需要再加一条"白天过滤器"的通路。
///
/// 反例（不分昼夜地平均）会安静地偏高：夜间 `alb = 1`（`MOD_Albedo.F90:217`
/// 先把 `alb` 置 1，`:295` 在天黑时直接 `RETURN`），把它平均进去实测偏高
/// 0.025，而且夜间记录会写出一个上游没有的数。
pub fn set_lct_albedo(
    sink: &mut impl HistorySink,
    record: usize,
    state: &colm_core::StandardLctEnergyState,
    surface_cosine_zenith: f64,
) -> Result<()> {
    if surface_cosine_zenith <= 0.0 {
        return sink.layer("alb", record, &[colm_core::MISSING; 4]);
    }
    let albedo = &state.radiation.albedo;
    let values = [albedo[0][0], albedo[1][0], albedo[0][1], albedo[1][1]];
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "the history value for alb is not finite"
    );
    sink.layer("alb", record, &values)?;
    Ok(())
}

/// 把一步的派生土壤量写进第 `record` 条记录。
///
/// 只做单位换算，公式见 [`LCT_DERIVED_SOIL_VARIABLES`]。层厚由模板提供，
/// 液体/冰由状态提供，长度都必须等于 `nl_soil`。
pub fn set_lct_derived_soil(
    sink: &mut impl HistorySink,
    record: usize,
    thickness: &[f64],
    water: &colm_core::Water2014SoilState,
) -> Result<()> {
    let liquid = &water.liquid_water_kg_m2;
    let ice = &water.ice_water_kg_m2;
    ensure!(
        thickness.len() == liquid.len() && thickness.len() == ice.len(),
        "the soil columns disagree on depth: {} thickness, {} liquid, {} ice",
        thickness.len(),
        liquid.len(),
        ice.len()
    );
    let h2osoi = thickness
        .iter()
        .zip(liquid)
        .zip(ice)
        .map(|((dz, wliq), wice)| {
            wliq / (dz * WATER_DENSITY_KG_M3) + wice / (dz * ICE_DENSITY_KG_M3)
        })
        .collect::<Vec<_>>();
    ensure!(
        h2osoi.iter().all(|value| value.is_finite()),
        "the derived h2osoi column is not finite"
    );
    sink.layer("h2osoi", record, &h2osoi)?;
    Ok(())
}

/// 把一步的冠层几何量写进第 `record` 条记录。
///
/// `sigf` 取步状态、`laisun`/`laisha` 取叶温出口 —— 两处都是内核已有的量，
/// 不做换算。见 [`LCT_CANOPY_VARIABLES`]。
pub fn set_lct_canopy_geometry(
    sink: &mut impl HistorySink,
    record: usize,
    state: &colm_core::StandardLctEnergyState,
    energy: &colm_core::StandardLctEnergyOutput,
    template: &StandardLctRestartTemplate,
) -> Result<()> {
    for (name, value) in [
        ("sigf", state.canopy.vegetation_free_fraction),
        // `green` 由静态配置定，装配期算好后存在模板上（见
        // `StandardLctRestartTemplate::vegetation_greenness`）。
        ("green", template.vegetation_greenness),
        ("laisun", energy.leaf.sunlit_leaf_area_index),
        ("laisha", energy.leaf.shaded_leaf_area_index),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的短波分带量写进第 `record` 条记录。
///
/// 十七项全部来自 `NetSolarFluxes`，不做换算；`*ln` 那一半在非本地正午时
/// 本来就是 `MISSING`，交给累加器跳过。见 [`LCT_RADIATION_VARIABLES`]。
pub fn set_lct_radiation_bands(
    sink: &mut impl HistorySink,
    record: usize,
    energy: &colm_core::StandardLctEnergyOutput,
) -> Result<()> {
    set_shortwave_bands(sink, record, &energy.shortwave)
}

/// `netsolar` 的反射/入射分波段诊断（植被与冰川共用）。
fn set_shortwave_bands(
    sink: &mut impl HistorySink,
    record: usize,
    shortwave: &colm_core::NetSolarFluxes,
) -> Result<()> {
    let noon = &shortwave.local_noon;
    for (name, value) in [
        ("sr", shortwave.reflected_w_m2),
        ("solvd", shortwave.direct_visible_w_m2),
        ("solvi", shortwave.diffuse_visible_w_m2),
        ("solnd", shortwave.direct_near_infrared_w_m2),
        ("solni", shortwave.diffuse_near_infrared_w_m2),
        ("srvd", shortwave.reflected_direct_visible_w_m2),
        ("srvi", shortwave.reflected_diffuse_visible_w_m2),
        ("srnd", shortwave.reflected_direct_near_infrared_w_m2),
        ("srni", shortwave.reflected_diffuse_near_infrared_w_m2),
        ("solvdln", noon.direct_visible_w_m2),
        ("solviln", noon.diffuse_visible_w_m2),
        ("solndln", noon.direct_near_infrared_w_m2),
        ("solniln", noon.diffuse_near_infrared_w_m2),
        ("srvdln", noon.reflected_direct_visible_w_m2),
        ("srviln", noon.reflected_diffuse_visible_w_m2),
        ("srndln", noon.reflected_direct_near_infrared_w_m2),
        ("srniln", noon.reflected_diffuse_near_infrared_w_m2),
    ] {
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 城市的短波诊断：入射四个波段取强迫场，反射量与当地正午量取 `netsolar_urban`。
fn set_urban_shortwave_bands(
    sink: &mut impl HistorySink,
    record: usize,
    shortwave: &colm_core::UrbanNetSolarFluxes,
    reference: HistoryReferenceState,
) -> Result<()> {
    let noon = &shortwave.local_noon;
    for (name, value) in [
        ("sr", shortwave.reflected_w_m2),
        ("solvd", reference.direct_visible_w_m2),
        ("solvi", reference.diffuse_visible_w_m2),
        ("solnd", reference.direct_near_infrared_w_m2),
        ("solni", reference.diffuse_near_infrared_w_m2),
        ("srvd", shortwave.reflected_direct_visible_w_m2),
        ("srvi", shortwave.reflected_diffuse_visible_w_m2),
        ("srnd", shortwave.reflected_direct_near_infrared_w_m2),
        ("srni", shortwave.reflected_diffuse_near_infrared_w_m2),
        ("solvdln", noon.direct_visible_w_m2),
        ("solviln", noon.diffuse_visible_w_m2),
        ("solndln", noon.direct_near_infrared_w_m2),
        ("solniln", noon.diffuse_near_infrared_w_m2),
        ("srvdln", noon.reflected_direct_visible_w_m2),
        ("srviln", noon.reflected_diffuse_visible_w_m2),
        ("srndln", noon.reflected_direct_near_infrared_w_m2),
        ("srniln", noon.reflected_diffuse_near_infrared_w_m2),
    ] {
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 把一步的驱动场镜像写进第 `record` 条记录。
///
/// 七项都是**照抄**本步的 `forc_*`，上游也是 `acc1d` 原值后写出，没有任何换算 ——
/// 所以它们在 `oracle/tolerances.toml` 里是 tier0（逐位）。抄错一次就会在该层
/// 直接红，这正是它们值得单独列出来的原因：它们量的是"驱动场读到内核里是不是
/// 同一份"，与物理无关，是排在其他 57 个变量之前该先对上的一组。
pub fn set_lct_forcing_mirrors(
    sink: &mut impl HistorySink,
    record: usize,
    reference: HistoryReferenceState,
    precipitation: &colm_core::PrecipitationState,
) -> Result<()> {
    for (name, value) in [
        ("xy_t", reference.air_temperature_k),
        ("xy_q", reference.specific_humidity_kg_kg),
        ("xy_pbot", reference.surface_pressure_pa),
        ("xy_us", reference.wind_speed_eastward_m_s),
        // 标量风算例里 `forc_vs` 恒为 0（`MOD_Forcing` 把标量风放在 `forc_us`）。
        // `HistoryReferenceState` 已经把标量情形折成 0，这里不再判一次。
        ("xy_vs", reference.wind_speed_northward_m_s),
        ("xy_frl", reference.downward_longwave_w_m2),
        ("xy_prc", reference.convective_precipitation_kg_m2_s),
        ("xy_prl", reference.large_scale_precipitation_kg_m2_s),
        // 相态拆分之后：`forc_rain = prc_rain + prl_rain`（`CoLMMAIN.F90:793`）。
        (
            "xy_rain",
            precipitation.convective_rain_kg_m2_s + precipitation.large_scale_rain_kg_m2_s,
        ),
        (
            "xy_snow",
            precipitation.convective_snow_kg_m2_s + precipitation.large_scale_snow_kg_m2_s,
        ),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    // `DEF_USE_CBL_HEIGHT`：`acc1d (forc_hpbl, a_hpbl)`（`MOD_Vars_1DAccFluxes.F90:2067-2069`）。
    if let Some(hpbl) = reference.boundary_layer_height_m {
        ensure!(
            hpbl.is_finite(),
            "the history value for xy_hpbl is not finite"
        );
        sink.scalar("xy_hpbl", record, hpbl)?;
    }
    // `xy_solarin` 单独走"四次 `acc1d` 进同一个累加器"的路径，不能并进上面的循环：
    // 上游 `MOD_Vars_1DAccFluxes.F90:2060-2063` 四个波段分别累加，而 `nac` 每步
    // 只加一次（`:2038`）。只有**第一项**算一步，后三项只进和 —— 见
    // [`HistorySink::accumulate`] 与 `HistoryReferenceState` 那四个字段的注释。
    for (index, value) in [
        reference.direct_visible_w_m2,
        reference.direct_near_infrared_w_m2,
        reference.diffuse_visible_w_m2,
        reference.diffuse_near_infrared_w_m2,
    ]
    .into_iter()
    .enumerate()
    {
        ensure!(
            value.is_finite(),
            "the history value for xy_solarin is not finite"
        );
        sink.accumulate("xy_solarin", record, value, index == 0)?;
    }
    Ok(())
}

/// 把一步的水文诊断写进第 `record` 条记录。
///
/// 两支共用：积雪分支把它 `WATER_2014` 输出里的 `soil` 那一半传进来。
pub fn set_lct_fluxes(
    sink: &mut impl HistorySink,
    record: usize,
    water: &colm_core::Water2014SoilOutput,
    variably_saturated: bool,
) -> Result<()> {
    // `qcharge` 与 VSF 互斥；`qlayer` 只在 VSF 打开时存在。
    if variably_saturated {
        sink.layer("qlayer", record, &water.soil_interface_flux_mm_s)?;
    }
    let mut scalars = vec![
        ("qinfl", water.infiltration_mm_s),
        ("rnof", water.total_runoff_mm_s),
        ("rsub", water.subsurface_runoff_mm_s),
        ("rsur", water.surface_runoff_mm_s),
    ];
    if variably_saturated {
        // VSF 打开时这三项有值（`Runoff_*` 的 `rsur_se`/`rsur_ie`/`frcsat`）。
        scalars.extend_from_slice(&[
            ("rsur_se", water.saturation_excess_runoff_mm_s),
            ("rsur_ie", water.infiltration_excess_runoff_mm_s),
            ("frcsat", water.saturated_fraction),
        ]);
    } else {
        // `WATER_2014` 不设这三项，它们停在分配时的 `spval`：`acc1d` 跳过、不进分子，但网格写出的分母
        // `sumarea` 只看 `filter`，这些 patch 的面积照样计入（与干湖同理）。原来干脆不交，网格里只剩水体
        // patch 的 `frcsat = 1`、`rsur_se = rsur` 被放大成整格的值（tm2w 实测）。
        scalars.extend_from_slice(&[
            ("rsur_se", f64::NAN),
            ("rsur_ie", f64::NAN),
            ("frcsat", f64::NAN),
        ]);
    }
    // `qcharge` 每步都累加（VSF 与否），由 `set_sidecar_only` 写；历史里只在 VSF 关掉时声明。
    // **`frcsat` 在 VSF 关掉时刻意不填。** 上游只有 `WATER_VSF` 走 `Runoff_*` 并传
    // `frcsat`（`MOD_SoilSnowHydrology.F90:880-925`，在 `WATER_VSF` 里），
    // `WATER_2014` 从不设它 —— 实测对齐算例 264 条记录**全是** `spval`，
    // 而开了 VSF 的黄金算例 264 条全有值。留空即与 Fortran 逐位相同
    // （`colm-hist` 的填充值与上游的 `spval` 都是 -1e36）。
    for (name, value) in scalars {
        // 干湖的 `frcsat` 是 `spval`（`CoLMMAIN.F90:1237`），`acc1d` 跳过它 —— 交 `spval`，留填充值。
        // 不能干脆不交：网格写出的分母 `sumarea` 只看 `filter`（`MOD_HistGridded.F90:203-220`），
        // 干湖的面积照样计入，只是分子里没有它。（VIC 与动态湿地原来也不赋值，见 upstream-bugs
        // 第 16 条，vendor 已修。）
        if matches!(name, "frcsat" | "rsur_se" | "rsur_ie") && value.is_nan() {
            sink.scalar(name, record, colm_core::MISSING)?;
            continue;
        }
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 无雪分支：把一步的状态写进第 `record` 条记录。
pub fn set_lct_state(
    sink: &mut impl HistorySink,
    record: usize,
    template: &StandardLctRestartTemplate,
    state: &StandardLctSoilState,
    ground_temperature_k: f64,
) -> Result<()> {
    set_plant_hydraulics(sink, record, &state.energy.leaf)?;
    set_columns(
        sink,
        record,
        template,
        // 无雪分支下雪段整段为零：history 的 `soilsnow` 恒有五个雪槽。
        &[
            vec![0.0; template.snow_slots()],
            vec![0.0; template.snow_slots()],
            vec![0.0; template.snow_slots()],
        ],
        &[
            state.temperature_k.as_slice(),
            state.water.liquid_water_kg_m2.as_slice(),
            state.water.ice_water_kg_m2.as_slice(),
        ],
        Scalars {
            ground_temperature_k,
            leaf_temperature_k: state.energy.leaf.leaf_temperature_k,
            water_table_depth_m: state.water.water_table_depth_m,
            aquifer_water_mm: state.water.aquifer_water_mm,
            surface_water_mm: state.water.surface_water_mm,
            snow_depth_m: 0.0,
            snow_water_equivalent_mm: 0.0,
            ground_snow_fraction: 0.0,
            leaf_area_index: state.energy.canopy.leaf_area_index,
            stem_area_index: state.energy.canopy.stem_area_index,
        },
    )
}

/// `f_vegwp`：四个节点的植被水势。
///
/// 只在 PHS 打开时写 —— 关掉时 `LeafTemperatureState::plant_hydraulics` 是
/// `None`，而累加器里根本没有这一列（见
/// [`LCT_PLANT_HYDRAULIC_VARIABLES`]）。给 `None` 编四个 `MISSING` 会让
/// 非 PHS 算例凭空多出一列全填充，与上游「那一列不存在」不同。
fn set_plant_hydraulics(
    sink: &mut impl HistorySink,
    record: usize,
    leaf: &colm_core::LeafTemperatureState,
) -> Result<()> {
    let Some(plant) = &leaf.plant_hydraulics else {
        return Ok(());
    };
    sink.layer("vegwp", record, &plant.vegetation_water_potential_mm)
}

/// 积雪分支：把一步的状态写进第 `record` 条记录。
pub fn set_lct_snow_state(
    sink: &mut impl HistorySink,
    record: usize,
    template: &StandardLctRestartTemplate,
    state: &StandardLctSnowSoilState,
    ground_temperature_k: f64,
) -> Result<()> {
    set_lct_snow_state_with(
        sink,
        record,
        template,
        state,
        ground_temperature_k,
        state.snow.temperature_k.clone(),
    )
}

/// 同 [`set_lct_snow_state`]，雪段温度由调用方给（被遮蔽的 patch 用重启原值）。
fn set_lct_snow_state_with(
    sink: &mut impl HistorySink,
    record: usize,
    template: &StandardLctRestartTemplate,
    state: &StandardLctSnowSoilState,
    ground_temperature_k: f64,
    snow_temperature_k: Vec<f64>,
) -> Result<()> {
    // **必须在积雪入口也写一次**：`standard_lct_snow_soil_step` 才是通用入口
    // （无雪起步的算例也走它，见它的文档），只补 `set_lct_state` 会让
    // `f_vegwp` 整列留填充值 —— 实测就是这样漏了一整轮。
    set_plant_hydraulics(sink, record, &state.energy.leaf)?;
    set_columns(
        sink,
        record,
        template,
        &[
            snow_temperature_k,
            state.snow.liquid_water_kg_m2.clone(),
            state.snow.ice_water_kg_m2.clone(),
        ],
        &[
            state.soil_temperature_k.as_slice(),
            state.soil_water.liquid_water_kg_m2.as_slice(),
            state.soil_water.ice_water_kg_m2.as_slice(),
        ],
        Scalars {
            ground_temperature_k,
            leaf_temperature_k: state.energy.leaf.leaf_temperature_k,
            water_table_depth_m: state.soil_water.water_table_depth_m,
            aquifer_water_mm: state.soil_water.aquifer_water_mm,
            surface_water_mm: state.soil_water.surface_water_mm,
            snow_depth_m: state.snow.depth_m,
            snow_water_equivalent_mm: state.snow.water_equivalent_kg_m2,
            ground_snow_fraction: state.snow.ground_snow_fraction,
            // 这两项来自**状态**而不是模板：上游每步末尾按雪盖重算它们，
            // 而 history 是在那之后写的（见 `run_restart_standard_lct_snow_with_history`）。
            leaf_area_index: state.energy.canopy.leaf_area_index,
            stem_area_index: state.energy.canopy.stem_area_index,
        },
    )
}

/// 每步只有一个值的那些量。
struct Scalars {
    ground_temperature_k: f64,
    leaf_temperature_k: f64,
    water_table_depth_m: f64,
    aquifer_water_mm: f64,
    surface_water_mm: f64,
    snow_depth_m: f64,
    snow_water_equivalent_mm: f64,
    ground_snow_fraction: f64,
    leaf_area_index: f64,
    stem_area_index: f64,
}

/// 把雪段与土段拼成 history 的 `soilsnow` 顺序（雪在前），再逐变量写进去。
fn set_columns(
    sink: &mut impl HistorySink,
    record: usize,
    template: &StandardLctRestartTemplate,
    snow: &[Vec<f64>; 3],
    soil: &[&[f64]; 3],
    scalars: Scalars,
) -> Result<()> {
    let slots = template.snow_slots();
    let layers = template.soil_layers();
    // 形状校验交给两个 sink：`HistoryBuffers::set_layered` 会拿声明过的层数比，
    // `HistoryAccumulator` 会拒绝同一步之间宽度变化。这里不重复取宽度 ——
    // 累加器根本没有"缓冲区宽度"这个概念。
    let column = |index: usize| -> Result<Vec<f64>> {
        ensure!(
            snow[index].len() == slots && soil[index].len() == layers,
            "the evolved columns do not match the template's {slots}+{layers} layers"
        );
        let mut values = snow[index].clone();
        values.extend_from_slice(soil[index]);
        Ok(values)
    };
    for (name, index) in [("t_soisno", 0usize), ("wliq_soisno", 1), ("wice_soisno", 2)] {
        sink.layer(name, record, &column(index)?)?;
    }
    for (name, value) in [
        ("t_grnd", scalars.ground_temperature_k),
        ("tleaf", scalars.leaf_temperature_k),
        ("zwt", scalars.water_table_depth_m),
        ("wa", scalars.aquifer_water_mm),
        ("wdsrf", scalars.surface_water_mm),
        ("snowdp", scalars.snow_depth_m),
        ("scv", scalars.snow_water_equivalent_mm),
        ("lai", scalars.leaf_area_index),
        ("sai", scalars.stem_area_index),
        ("fsno", scalars.ground_snow_fraction),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
    }
    Ok(())
}

/// 一次运行的 history 写出会话：按调度把每步的值填进记录，分组结束时落盘。
///
/// 对齐方式是**写入 tick**：`schedule_records` 给出每条记录的写入时刻（那一步的结束
/// tick），会话在某一步的结束 tick 命中时写下一条。判据与标签都取自 `colm-hist`，
/// 这里不重算 —— 重算就是埋一个会与上游漂开的副本。
#[derive(Debug)]
pub struct HistorySession {
    dimensions: HistoryDimensions,
    site: HistorySite,
    records: Vec<ScheduledRecord>,
    cursor: usize,
    directory: PathBuf,
    stem: String,
    open: Option<(String, HistoryBuffers)>,
    /// 当前输出区间的累加器（上游的 `a_*` 与 `nac`），每个 patch 一份。
    accumulators: Vec<HistoryAccumulator>,
    /// 本步下一个要累加的 patch：多 patch 单点每步按 0..N 依次 `push_*`，最后一个才计步写出。
    patch_cursor: usize,
    /// `DEF_USE_PLANTHYDRAULICS`：决定新开的文件里要不要声明 `f_vegwp`。
    ///
    /// 由第一次 `push_lct*` 的模板现场给出（`new` 收不到模板）。默认 `false`
    /// 是安全方向：少声明一列只会让 PHS 算例的文件少一列，而多声明一列会让
    /// 非 PHS 算例凭空多出一列全填充。
    plant_hydraulics: bool,
    /// `DEF_USE_VariablySaturatedFlow`：决定 `f_qcharge` 与 `f_qlayer` 谁在文件里。
    variably_saturated: bool,
    /// 城市 patch：多声明 [`URBAN_VARIABLES`]。
    urban: bool,
    /// `DEF_USE_BGC`：按 BGC 开关多声明 [`bgc_history_variables`]。
    bgc: Option<colm_core::bgc_driver::BgcSwitches>,
    /// 每步推进后当前区间的原始累加状态（上游的 `nac`、`nac_ln`、`nac_dt` 与 `a_*`）。
    /// 写续跑文件的一方要把它存进旁车，但那时会话正被运行循环独占，所以共享一份快照。
    window: std::sync::Arc<std::sync::Mutex<Vec<crate::history_sidecar::HistoryWindow>>>,
    /// 运行终点那条不在自然边界上的记录写出**之前**的窗口：上游此时先存原始窗口
    /// （`MOD_Hist.F90:265-274`），写完历史、清零之后的重启不再重存。
    raw_at_end: Option<Vec<crate::history_sidecar::HistoryWindow>>,
    /// 同一时刻示踪物与 CH4 累加器的原始值（它们存在 patch 状态里，写完最后一条就清零）。
    tracer_raw_at_end: crate::tracer_sidecar::RawTracersHandle,
    /// 多 patch 单点本步的网格元近地面诊断（见 [`element_surface_input`]）；单 patch 为 `None`。
    element_surface: Option<colm_core::HistoryDiagnostics>,
    /// `DEF_USE_Dynamic_Wetland`：`f_wetwat` 改写 `wdsrf` 的平均（由 `push_lct_snow` 现场给出）。
    dynamic_wetland: bool,
    /// `forcmask_pch`（空间算例、强迫有缺测时）：为假的 patch 不进任何网格聚合
    /// （上游各 `filter` 都与上了它）。`None` 即全部有效。
    forcing_mask: Option<Vec<bool>>,
    /// `DEF_USE_Dynamic_Lake`：多写 `f_dz_lake`、不写 `f_lake_deficit`（由 `push_lake` 现场给出）。
    dynamic_lake: bool,
    /// 空间算例：写文件时按面积聚合到经纬网格（`HistForm = 'Gridded'`）。
    grid: Option<std::sync::Arc<colm_hist::history::HistoryGrid>>,
    /// 由调用方聚合好的网格量（河道量）：每个新文件都声明它们。
    gridded_names: Vec<&'static str>,
    /// 本步要写的那条记录的预聚合值（[`Self::stage_gridded`]），写记录时放进缓冲。
    staged: Vec<(&'static str, Vec<f64>)>,
    /// 非结构网格的向量写出（`DEF_HISTORY_IN_VECTOR`）；与 `grid` 互斥。
    vector: Option<std::sync::Arc<colm_hist::history::HistoryVector>>,
    /// 向量写出时由调用方逐 patch 给值的量（河道量）：`(名字, input_mode = 'total')`。
    patch_field_names: Vec<(&'static str, bool)>,
    /// 向量写出时每个文件第一条记录写一次的无时间维量。
    vector_statics: Vec<(String, String, String, Vec<f64>)>,
    /// 本步那条记录的逐 patch 量（[`Self::stage_patch_field`]）。
    staged_patch: Vec<(&'static str, Vec<f64>, Vec<bool>)>,
    /// `DEF_USE_TRACER`（网格写出）：步长 [s]。打开时每条记录多写窗口变量，并另写
    /// 示踪物 history 文件（见 [`Self::with_tracer_history`]）。
    tracer_time_step_seconds: Option<f64>,
    /// 有输运示踪物时的示踪物 history（`tracer_hist_out`），见 [`Self::with_tracer_variables`]。
    tracer_variables: Option<TracerHistoryState>,
    /// 闸门 3（`DEF_hist_vars`）：开会话时取 [`install_selection`] 装好的那一份。
    selection: Option<std::sync::Arc<colm_hist::selection::HistorySelection>>,
}

/// 本进程的 `DEF_hist_vars` 开关状态。上游它是 namelist 模块里的全局量，读一次、整个运行不变；
/// 这里同样只装一次，之后开的每个 history 会话都按它过滤（不装就不过滤）。
static SELECTION: std::sync::OnceLock<std::sync::Arc<colm_hist::selection::HistorySelection>> =
    std::sync::OnceLock::new();

/// 装上本进程的 history 开关状态（只能装一次）。
pub fn install_selection(selection: colm_hist::selection::HistorySelection) -> Result<()> {
    SELECTION
        .set(std::sync::Arc::new(selection))
        .map_err(|_| anyhow::anyhow!("the history selection is installed twice"))
}

/// 示踪物 history 的调度与当前文件（与主 history 同一份记录表，自己计步）。
#[derive(Debug)]
struct TracerHistoryState {
    set: colm_core::tracer::TracerSet,
    /// 各 patch 的 `patchtype`（`filter`）。
    patch_types: Vec<i32>,
    cursor: usize,
    /// 上次写出以来的非预热步数（主 history 的 `nac`）。
    steps: usize,
    open: Option<(
        String,
        HistoryBuffers,
        Vec<colm_hist::history::TracerFileVariable>,
    )>,
}

/// 多 patch 时每个累加器只写自己那一格；单 patch 一次写全部（缓冲区只有一格）。
fn means_are_split(patches: usize) -> bool {
    patches > 1
}

impl HistorySession {
    /// 当前区间原始累加状态的共享句柄（见 `window` 字段）。
    pub fn window_handle(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<crate::history_sidecar::HistoryWindow>>> {
        std::sync::Arc::clone(&self.window)
    }

    /// 运行终点的示踪物/CH4 累加器快照（见 `tracer_raw_at_end` 字段）；写续跑旁车的一方取走。
    pub fn tracer_raw_handle(&self) -> crate::tracer_sidecar::RawTracersHandle {
        std::sync::Arc::clone(&self.tracer_raw_at_end)
    }

    /// `forcmask_pch`：被强迫缺测遮蔽的 patch 在写网格/向量均值时整个跳过。
    pub fn set_forcing_mask(&mut self, mask: Option<Vec<bool>>) {
        self.forcing_mask = mask;
    }

    /// 被强迫缺测遮蔽的 patch（`forcmask_pch = .false.`）的一步累加。
    ///
    /// `CoLMDRIVER` 整步跳过它，但 `accumulate_fluxes` 照常对整个数组 `acc1d`：
    /// - 通量数组在它上面一直是分配时的 `spval`，`acc1d` 跳过 —— 这里不交；
    /// - 时间变量（水、温度、雪、`lai`/`sai`、`sigf`/`green`、`alb` …）是起跑重启里的值，照常累加；
    /// - `tref`/`qref`/`z0m`/`emis` 是重启里的模块数组值，`t2m_wmo = tref`；
    /// - `forc_rain`/`forc_snow` 在它上面是 0；
    /// - `coszen` 停在重启值（`read_forcing` 跳过了它），所以 `alb` 按它的正负计入；
    /// - 近地面相似诊断取网格元的值（只用未遮蔽 patch 聚合，`MOD_Vars_1DAccFluxes.F90:2690-2790`），
    ///   全元都被遮蔽时 `CYCLE`，留 `spval`。
    ///
    /// 这些值只进续跑旁车；写历史时被遮蔽 patch 整个跳过（各 `filter` 都与上了 `forcmask_pch`）。
    pub fn push_masked(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
    ) -> Result<Option<PathBuf>> {
        // 五类 patch 都实测过：土壤、湿地、湖、城市（g1fmm、g1bgcm、g1urbmm）与冰川（g1glm）。
        ensure!(
            matches!(template.patch_type, 0..=4),
            "patch {} has an unknown patchtype {}",
            template.patch,
            template.patch_type
        );
        let restart = |name: &str| {
            template.restart_diagnostic(name).with_context(|| {
                format!(
                    "the restart has no {name} for masked patch {}",
                    template.patch
                )
            })
        };
        let (tref, qref, z0m, emis, coszen) = (
            restart("tref")?,
            restart("qref")?,
            restart("z0m")?,
            restart("emis")?,
            restart("coszen")?,
        );
        let ground = state.surface_temperature_k();
        let snow_temperature = template.restart_snow_temperature()?;
        self.plant_hydraulics = template.plant_hydraulics();
        self.variably_saturated = template.physics.variably_saturated_flow;
        self.dynamic_wetland = template.physics.dynamic_wetland;
        self.bgc = template.bgc.as_ref().map(|bgc| bgc.switches);
        let element_surface = self.element_surface;
        self.push(end, |accumulator| {
            set_lct_snow_state_with(accumulator, 0, template, state, ground, snow_temperature)?;
            for (name, value) in [
                ("ldew", state.energy.leaf.canopy_water.total_mm),
                ("sigf", state.energy.canopy.vegetation_free_fraction),
                ("green", template.vegetation_greenness),
                ("tref", tref),
                ("qref", qref),
                ("z0m", z0m),
                ("emis", emis),
                ("xy_rain", 0.0),
                ("xy_snow", 0.0),
            ] {
                accumulator.scalar(name, 0, value)?;
            }
            set_lct_albedo(accumulator, 0, &state.energy, coszen)?;
            if let Some(element) = element_surface.as_ref() {
                for (name, value) in [
                    ("zol", element.zol),
                    ("rib", element.bulk_richardson),
                    ("ustar", element.friction_velocity_m_s),
                    ("qstar", element.humidity_scale),
                    ("tstar", element.temperature_scale_k),
                    ("fm", element.momentum_similarity),
                    ("fh", element.heat_similarity),
                    ("fq", element.moisture_similarity),
                    ("us10m", element.wind_10m_eastward_m_s),
                    ("vs10m", element.wind_10m_northward_m_s),
                    ("fm10m", element.momentum_at_10m),
                    ("ustar2", element.similarity_friction_velocity_m_s),
                ] {
                    accumulator.scalar(name, 0, value)?;
                }
            }
            // 城市模型：城市状态量照常累加（重启值，g1urbmm 实测，第 543 轮）；城市各项通量停在 `spval`。
            if let Some(urban) = state.urban.as_ref() {
                for (name, value) in [
                    ("t_room", urban.t_room),
                    ("tafu", urban.tafu),
                    ("fhac", urban.fhac),
                    ("fwst", urban.fwst),
                    ("fach", urban.fach),
                    ("fhah", urban.fhah),
                    ("fahe", urban.fahe),
                    ("fvehc", urban.vehc),
                    ("fmeta", urban.meta),
                    ("t_roof", urban.t_roof),
                    ("t_wall", urban.t_wall),
                ] {
                    accumulator.scalar(name, 0, value)?;
                }
            }
            // 湖：重启里的湖层温度与冰比例照常累加。
            if let Some(lake) = state.lake.as_ref().filter(|_| template.patch_type == 4) {
                accumulator.layer("t_lake", 0, &lake.column.temperature_k)?;
                accumulator.layer("lake_icefrac", 0, &lake.column.ice_fraction)?;
            }
            if let (Some(runtime), Some(bgc)) = (&template.bgc, &state.bgc) {
                let first_pft_class = state
                    .energy
                    .pft
                    .as_ref()
                    .and_then(|pft| pft.parameters.first())
                    .map(|parameters| parameters.class);
                set_bgc_history(
                    &mut MaskedBgcSink {
                        inner: accumulator,
                        soil: template.patch_type == 0,
                    },
                    0,
                    runtime,
                    bgc,
                    first_pft_class,
                    state.irrigation.as_deref(),
                    template.patch_type == 0,
                )?;
            }
            set_ozone_history(accumulator, state.energy.leaf.ozone.as_ref())?;
            set_sidecar_only(
                accumulator,
                template,
                &state.soil_water,
                state.lake.as_ref(),
                tref,
                state.energy.leaf.canopy_water.rain_mm,
                state.energy.leaf.canopy_water.snow_mm,
                None,
            )
        })
    }

    /// 本步各 patch 共用的网格元近地面诊断（多 patch 单点）；`None` 时逐 patch 重算。
    pub fn set_element_surface(&mut self, element: Option<colm_core::HistoryDiagnostics>) {
        self.element_surface = element;
    }

    /// 多 patch 单点：历史文件的 `patch` 维与累加器份数（上游单点历史逐 patch 写，不做面积聚合，
    /// `MOD_HistSingle.F90:single_write_2d`）。要在第一步之前调。
    pub fn with_patches(mut self, patches: usize) -> Result<Self> {
        ensure!(patches > 0, "a history session needs at least one patch");
        ensure!(
            self.accumulators
                .iter()
                .all(|accumulator| accumulator.steps == 0),
            "the history patch count can only change before the first step"
        );
        self.dimensions.patch = patches;
        self.accumulators = (0..patches)
            .map(|_| HistoryAccumulator::default())
            .collect();
        *self.window.lock().expect("history window lock") =
            vec![crate::history_sidecar::HistoryWindow::default(); patches];
        Ok(self)
    }

    /// 续跑：从旁车读回的窗口接着累加（`read_history_acc_restart`），每个 patch 一份。
    pub fn restore(&mut self, windows: Vec<crate::history_sidecar::HistoryWindow>) -> Result<()> {
        ensure!(
            self.accumulators
                .iter()
                .all(|accumulator| accumulator.steps == 0),
            "the history window can only be restored before the first step"
        );
        ensure!(
            windows.len() == self.accumulators.len(),
            "the history sidecar holds {} patches, the run {}",
            windows.len(),
            self.accumulators.len()
        );
        self.accumulators = windows
            .iter()
            .map(HistoryAccumulator::from_window)
            .collect();
        if let (Some(tracer), Some(window)) = (self.tracer_variables.as_mut(), windows.first()) {
            tracer.steps = window.steps;
        }
        *self.window.lock().expect("history window lock") = windows;
        Ok(())
    }

    /// 开一个会话。文件名按上游约定拼成 `<stem>_hist_<后缀>.nc`。
    pub fn new(
        dimensions: HistoryDimensions,
        site: HistorySite,
        window: SimulationWindow,
        frequency: HistoryFrequency,
        grouping: HistoryGrouping,
        directory: impl AsRef<Path>,
        stem: impl Into<String>,
    ) -> Result<Self> {
        let records = schedule_records(window, frequency, grouping)?;
        // `DEF_HIST_FREQ = 'none'`：上游照样每步 `accumulate_fluxes`、从不写历史，区间一直不清零，
        // 续跑旁车存的是整段运行的原始累加（实测 `nac` = 48/96/144）。会话只累加、不调度。
        ensure!(
            !records.is_empty() || frequency == colm_hist::schedule::HistoryFrequency::None,
            "the history schedule produced no records; check DEF_HIST_FREQ against the window"
        );
        Ok(Self {
            dimensions,
            site,
            records,
            cursor: 0,
            directory: directory.as_ref().to_path_buf(),
            stem: stem.into(),
            open: None,
            accumulators: (0..dimensions.patch)
                .map(|_| HistoryAccumulator::default())
                .collect(),
            patch_cursor: 0,
            plant_hydraulics: false,
            variably_saturated: false,
            urban: false,
            bgc: None,
            window: std::sync::Arc::new(std::sync::Mutex::new(vec![
                crate::history_sidecar::HistoryWindow::default();
                dimensions.patch
            ])),
            raw_at_end: None,
            tracer_raw_at_end: std::sync::Arc::default(),
            element_surface: None,
            dynamic_wetland: false,
            forcing_mask: None,
            dynamic_lake: false,
            grid: None,
            gridded_names: Vec::new(),
            staged: Vec::new(),
            vector: None,
            patch_field_names: Vec::new(),
            vector_statics: Vec::new(),
            staged_patch: Vec::new(),
            tracer_time_step_seconds: None,
            tracer_variables: None,
            selection: SELECTION.get().cloned(),
        })
    }

    /// 每个 history 文件都多声明这些由调用方聚合好的网格量（见 [`Self::stage_gridded`]）。
    pub fn with_gridded(mut self, names: &[&'static str]) -> Self {
        self.gridded_names = names.to_vec();
        self
    }

    /// 这一步（结束于 `end`）是不是某条记录的写入时刻：是就返回它（上游 `hist_out` 此时写出）。
    pub fn pending_record(&self, end: CalendarTime) -> Result<Option<ScheduledRecord>> {
        let Some(record) = self.records.get(self.cursor) else {
            return Ok(None);
        };
        Ok((tick_of(end)? == record.write_at_tick).then(|| record.clone()))
    }

    /// `DEF_USE_TRACER`：主 history 多写 `history_window_seconds = nac*deltim` 与
    /// `history_window_end_minutes`，并另写 `<case>_hist_tracer_<cdate>.nc`
    /// （`MOD_Hist.F90:319-358`，只在 `HistForm == 'Gridded'` 时写窗口变量）。
    pub fn with_tracer_history(mut self, time_step_seconds: Option<f64>) -> Result<Self> {
        if time_step_seconds.is_some() {
            ensure!(
                self.grid.is_some(),
                "the tracer history windows are only written for gridded history"
            );
        }
        self.tracer_time_step_seconds = time_step_seconds;
        Ok(self)
    }

    /// 本步那条记录里一个预聚合网格量的值；随本步的记录一起写进缓冲。
    pub fn stage_gridded(&mut self, name: &'static str, values: Vec<f64>) -> Result<()> {
        ensure!(
            self.gridded_names.contains(&name),
            "{name} was not declared with with_gridded()"
        );
        self.staged.push((name, values));
        Ok(())
    }

    /// 非结构网格：history 写成单元向量（`HistForm = 'Vector'`）。`patch_fields` 是由调用方逐 patch
    /// 给值的量（[`Self::stage_patch_field`]），`statics` 是每个文件第一条记录写一次的无时间维量。
    pub fn with_vector(
        mut self,
        vector: std::sync::Arc<colm_hist::history::HistoryVector>,
        patch_fields: &[(&'static str, bool)],
        statics: Vec<(String, String, String, Vec<f64>)>,
    ) -> Result<Self> {
        ensure!(self.grid.is_none(), "history is either gridded or vector");
        self.vector = Some(vector);
        self.patch_field_names = patch_fields.to_vec();
        self.vector_statics = statics;
        Ok(self)
    }

    /// 本步那条记录里一个逐 patch 量的值与过滤（向量写出时的河道量）。
    pub fn stage_patch_field(
        &mut self,
        name: &'static str,
        values: Vec<f64>,
        included: Vec<bool>,
    ) -> Result<()> {
        ensure!(
            self.patch_field_names.iter().any(|(n, _)| *n == name),
            "{name} was not declared with with_vector()"
        );
        self.staged_patch.push((name, values, included));
        Ok(())
    }

    /// 是不是向量写出。
    pub fn is_vector(&self) -> bool {
        self.vector.is_some()
    }

    /// 空间算例：history 写成经纬网格（每个 patch 的份面积见 [`colm_hist::history::HistoryGrid`]）。
    pub fn with_grid(mut self, grid: std::sync::Arc<colm_hist::history::HistoryGrid>) -> Self {
        self.grid = Some(grid);
        self
    }

    /// 还有多少条记录没写（运行结束时应当为 0）。
    pub fn remaining(&self) -> usize {
        self.records.len() - self.cursor
    }

    /// 无雪分支：某一步结束后调用。命中写入时刻就填一条；分组写完就落盘。
    pub fn push_lct(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSoilState,
        output: &StandardLctSoilOutput,
        reference: HistoryReferenceState,
    ) -> Result<Option<PathBuf>> {
        let ground = output.energy.ground.temperature_k[0];
        // 在开文件之前定下 `f_vegwp`/`f_qcharge`/`f_qlayer` 谁在文件里。
        self.plant_hydraulics = template.plant_hydraulics();
        self.variably_saturated = template.physics.variably_saturated_flow;
        let variably_saturated = self.variably_saturated;
        self.push(end, |accumulator| {
            set_lct_state(accumulator, 0, template, state, ground)?;
            set_lct_fluxes(accumulator, 0, &output.water, variably_saturated)?;
            set_lct_energy_fluxes(accumulator, 0, output)?;
            set_lct_surface_budget(accumulator, 0, output)?;
            set_lct_surface_diagnostics(
                accumulator,
                0,
                &output.energy,
                reference,
                &template.physics,
            )?;
            set_lct_stomatal_diagnostics(accumulator, 0, &output.energy)?;
            set_lct_radiation_bands(accumulator, 0, &output.energy)?;
            set_lct_canopy_geometry(accumulator, 0, &state.energy, &output.energy, template)?;
            set_lct_derived_soil(
                accumulator,
                0,
                template.soil_layer_thickness_m(),
                &state.water,
            )?;
            set_lct_water_storage(
                accumulator,
                0,
                &state.water,
                state.energy.leaf.canopy_water.total_mm,
                0.0,
                template.water_storage_tail_mm(&state.water),
            )?;
            set_lct_canopy_water(accumulator, 0, &state.energy, &output.energy)?;
            set_lct_soil_resistance(accumulator, 0, &output.energy)?;
            set_lct_albedo(
                accumulator,
                0,
                &state.energy,
                reference.surface_cosine_zenith,
            )?;
            set_lct_forcing_mirrors(accumulator, 0, reference, &output.energy.precipitation)?;
            set_ozone_history(accumulator, state.energy.leaf.ozone.as_ref())?;
            set_sidecar_only(
                accumulator,
                template,
                &state.water,
                None,
                output.energy.leaf.air_temperature_2m_k,
                state.energy.leaf.canopy_water.rain_mm,
                state.energy.leaf.canopy_water.snow_mm,
                (!variably_saturated).then_some(output.water.recharge_mm_s),
            )?;
            set_lct_balance_errors(
                accumulator,
                0,
                output,
                colm_core::total_water_storage_mm(
                    &state.water,
                    state.energy.leaf.canopy_water.total_mm,
                    0.0,
                    None,
                ),
                reference,
            )
        })
    }

    /// 积雪分支：与 [`Self::push_lct`] 同构，走雪入口并把 `soil` 那一半当土壤诊断。
    /// 冰川 patch 的一步 history。
    ///
    /// 上游的 `accumulate_fluxes` 对所有 patch 读同一组全局量：冰川分支写
    /// `GLACIER_TEMP` 的通量，植被那一组在 `CoLMMAIN.F90:2178-2230` 被清零
    /// （`etr`/`fsenl`/`fevpl`/`assim`/`respc`/`rstfac*`/`gs*`/`laisun`/`laisha`/`green`/
    /// `qintr`/`qinfl`/`qlayer`/`rootr`/`qcharge` = 0，`frcsat = 1`，
    /// `qdrip = forc_rain + forc_snow`）。`rss` 冰川不重算，沿用重启里的值。
    pub fn push_glacier(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: &colm_core::GlacierStepOutput,
        reference: HistoryReferenceState,
    ) -> Result<Option<PathBuf>> {
        let water_balance_error = if template.physics.variably_saturated_flow {
            output.water_balance_error_mm_s
        } else {
            0.0
        };
        self.push_non_soil(
            end,
            template,
            state,
            NonSoilStep {
                precipitation: &output.precipitation,
                shortwave: &output.shortwave,
                thermal: output.thermal,
                surface_runoff_mm_s: output.surface_runoff_mm_s,
                total_runoff_mm_s: output.total_runoff_mm_s,
                water_balance_error_mm_s: water_balance_error,
                writes_runoff_split: template.physics.variably_saturated_flow,
            },
            reference,
            &VEGETATED_ONLY_VARIABLES,
            &[],
        )
    }

    /// 湖 patch 的一步 history：与冰川同一套"非土壤"写法（植被量清零、`frcsat = 1`），
    /// 另加 `MOD_Hist.F90:4497-4537` 只在 `patchtype == 4` 上写的 `t_lake`/`lake_icefrac`/
    /// `lake_deficit`。非动态湖 `xerr = 0`（`CoLMMAIN.F90:1978-1982`）。
    pub fn push_lake(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: &colm_core::LakeStepOutput,
        reference: HistoryReferenceState,
    ) -> Result<Option<PathBuf>> {
        let lake = state
            .lake
            .as_ref()
            .context("a lake history record needs the lake state")?;
        let thermal = output.thermal;
        let dynamic = template.physics.dynamic_lake;
        self.dynamic_lake = dynamic;
        // 动态湖：`xerr = errorw/deltim`（`CoLMMAIN.F90:1984-1988`），定深湖恒为 0；`a_lake_deficit` 不累加、
        // `a_dz_lake` 累加（`MOD_Vars_1DAccFluxes.F90:2485`、`:2502`）。
        // `deltim` 在 `CoLMMAIN` 里是 `deltim_phy`：水体按 `ceiling(deltim/1800)` 分子步时是子步步长。
        let water_balance_error = if dynamic {
            output.water_balance_error_mm / template.colmmain_step_seconds()
        } else {
            0.0
        };
        let deficit = [LakeLayers::Scalar("lake_deficit", output.lake_deficit_mm_s)];
        let thickness = [LakeLayers::Layers("dz_lake", &lake.column.thickness_m)];
        let layers = [
            LakeLayers::Layers("t_lake", &lake.column.temperature_k),
            LakeLayers::Layers("lake_icefrac", &lake.column.ice_fraction),
        ];
        let extra: Vec<LakeLayers<'_>> = if dynamic {
            thickness.into_iter().chain(layers).collect()
        } else {
            deficit.into_iter().chain(layers).collect()
        };
        self.push_non_soil(
            end,
            template,
            state,
            NonSoilStep {
                precipitation: &output.precipitation,
                shortwave: &output.shortwave,
                thermal: colm_core::GlacierThermalFluxes {
                    taux: thermal.taux,
                    tauy: thermal.tauy,
                    fsena: thermal.fsena,
                    fevpa: thermal.fevpa,
                    lfevpa: thermal.lfevpa,
                    fseng: thermal.fseng,
                    fevpg: thermal.fevpg,
                    olrg: thermal.olrg,
                    fgrnd: thermal.fgrnd,
                    qseva: thermal.qseva,
                    qsdew: thermal.qsdew,
                    qsubl: thermal.qsubl,
                    qfros: thermal.qfros,
                    sm: thermal.sm,
                    tref: thermal.tref,
                    qref: thermal.qref,
                    trad: thermal.trad,
                    errore: 0.0,
                    emis: thermal.emis,
                    z0m: thermal.z0m,
                    zol: thermal.zol,
                    rib: thermal.rib,
                    ustar: thermal.ustar,
                    qstar: thermal.qstar,
                    tstar: thermal.tstar,
                    fm: thermal.fm,
                    fh: thermal.fh,
                    fq: thermal.fq,
                    xmf: 0.0,
                },
                surface_runoff_mm_s: output.surface_runoff_mm_s,
                total_runoff_mm_s: output.total_runoff_mm_s,
                water_balance_error_mm_s: water_balance_error,
                writes_runoff_split: true,
            },
            reference,
            &LAKE_FILTERED_VARIABLES,
            &extra,
        )
    }

    /// 冰川与湖共用的"非土壤" patch 写法。
    ///
    /// 上游的 `accumulate_fluxes` 对所有 patch 读同一组全局量：冰川/湖分支写各自的
    /// 通量，植被那一组在 `CoLMMAIN.F90:2178-2230` 被清零
    /// （`etr`/`fsenl`/`fevpl`/`assim`/`respc`/`rstfac*`/`gs*`/`laisun`/`laisha`/`green`/
    /// `qintr`/`qinfl`/`qlayer`/`rootr`/`qcharge` = 0，`frcsat = 1`，
    /// `qdrip = forc_rain + forc_snow`）。`rss` 不重算，沿用状态里最近一次的值。
    #[allow(clippy::too_many_arguments)]
    fn push_non_soil(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: NonSoilStep<'_>,
        reference: HistoryReferenceState,
        skipped: &'static [&'static str],
        extra: &[LakeLayers<'_>],
    ) -> Result<Option<PathBuf>> {
        let ground = state.surface_temperature_k();
        self.plant_hydraulics = template.plant_hydraulics();
        self.variably_saturated = template.physics.variably_saturated_flow;
        // BGC 状态随所有 patch 存（`accumulate_fluxes` 对湖、冰川也累加 BGC 量）。
        self.bgc = template.bgc.as_ref().map(|bgc| bgc.switches);
        let thermal = output.thermal;
        let shortwave = output.shortwave;
        let element_surface = self.element_surface;
        self.push(end, |accumulator| {
            let accumulator = &mut PatchFilteredSink {
                inner: accumulator,
                skipped,
            };
            set_lct_snow_state(accumulator, 0, template, state, ground)?;
            if let (Some(runtime), Some(bgc)) = (&template.bgc, &state.bgc) {
                set_bgc_history(
                    accumulator,
                    0,
                    runtime,
                    bgc,
                    None,
                    state.irrigation.as_deref(),
                    false,
                )?;
            }
            // `frcsat = 1` 由末尾 `patchtype > 2` 那一节无条件写，与 VSF 无关。
            let mut fluxes = vec![
                ("qinfl", 0.0),
                ("rnof", output.total_runoff_mm_s),
                ("rsub", 0.0),
                ("rsur", output.surface_runoff_mm_s),
                ("frcsat", 1.0),
            ];
            if output.writes_runoff_split {
                fluxes.extend_from_slice(&[
                    ("rsur_se", output.surface_runoff_mm_s),
                    ("rsur_ie", 0.0),
                ]);
            }
            // `qcharge = 0`（`CoLMMAIN.F90:2254`）由 `set_sidecar_only` 写。
            // `rnet = sabg + sabvsun + sabvsha - olrg + forc_frl`（`MOD_Vars_1DAccFluxes.F90:2093`）
            let net_radiation = shortwave.ground_absorbed_w_m2
                + shortwave.sunlit_absorbed_w_m2
                + shortwave.shaded_absorbed_w_m2
                - thermal.olrg
                + reference.downward_longwave_w_m2;
            let water_balance_error = output.water_balance_error_mm_s;
            for (name, value) in fluxes.into_iter().chain([
                ("fsena", thermal.fsena),
                ("fevpa", thermal.fevpa),
                ("etr", 0.0),
                ("sabg", shortwave.ground_absorbed_w_m2),
                ("fsenl", 0.0),
                ("fseng", thermal.fseng),
                ("fevpl", 0.0),
                ("fevpg", thermal.fevpg),
                ("sabvsun", shortwave.sunlit_absorbed_w_m2),
                ("sabvsha", shortwave.shaded_absorbed_w_m2),
                ("rnet", net_radiation),
                ("olrg", thermal.olrg),
                ("emis", thermal.emis),
                // `r_trad = (olrg/stefnc)**0.25`，与 `GLACIER_TEMP` 的 `trad` 同式。
                ("trad", thermal.trad),
                ("fgrnd", thermal.fgrnd),
                ("lfevpa", thermal.lfevpa),
                ("xerr", water_balance_error),
                // 分支里 `zerr = errore`（`CoLMMAIN.F90:1751`），但末尾 `patchtype > 2` 那一节
                // 又把它清成 0（`:2230`），history 读到的是后者。
                ("zerr", 0.0),
                ("assim", 0.0),
                ("assimsun", 0.0),
                ("assimsha", 0.0),
                ("respc", 0.0),
                ("etrsun", 0.0),
                ("etrsha", 0.0),
                ("gssun", 0.0),
                ("gssha", 0.0),
                ("rstfacsun", 0.0),
                ("rstfacsha", 0.0),
                // 不重算，但它是 module 变量：动态湖的干湖步（走土壤分支）会改写它，之后的湿湖步
                // 读到的是那个值，不是起跑重启里的。
                ("rss", state.energy.soil_surface_resistance_s_m),
                ("ldew", 0.0),
                ("qintr", 0.0),
                (
                    "qdrip",
                    output.precipitation.convective_rain_kg_m2_s
                        + output.precipitation.large_scale_rain_kg_m2_s
                        + (output.precipitation.convective_snow_kg_m2_s
                            + output.precipitation.large_scale_snow_kg_m2_s),
                ),
                ("sigf", 0.0),
                ("green", 0.0),
                ("laisun", 0.0),
                ("laisha", 0.0),
            ]) {
                ensure!(
                    value.is_finite(),
                    "the non-soil history value for {name} is not finite"
                );
                accumulator.scalar(name, 0, value)?;
            }
            set_glacier_surface_diagnostics(
                accumulator,
                &thermal,
                reference,
                &template.physics,
                element_surface.as_ref(),
            )?;
            // `h2osoi` 对每个 patch 都按液/冰重算（`CoLMMAIN.F90:2260`），`qlayer`/`rootr` 在
            // `patchtype > 2` 那一节清零（`:2233`、`:2246`）；历史按 `patchtype` 过滤掉它们。
            set_lct_derived_soil(
                accumulator,
                0,
                template.soil_layer_thickness_m(),
                &state.soil_water,
            )?;
            let layers = template.soil_layer_thickness_m().len();
            accumulator.layer("qlayer", 0, &vec![0.0; layers + 1])?;
            accumulator.layer("rootr", 0, &vec![0.0; layers])?;
            // 冰川/湖：`ldew_rain = ldew_snow = qcharge = 0`（`CoLMMAIN.F90:2218-2254`）；
            // VSF 打开时 `acc1d(qcharge)` 整个不调（`MOD_Vars_1DAccFluxes.F90:2135`）。
            set_ozone_history(accumulator, state.energy.leaf.ozone.as_ref())?;
            set_sidecar_only(
                accumulator,
                template,
                &state.soil_water,
                state.lake.as_ref(),
                thermal.tref,
                0.0,
                0.0,
                (!template.physics.variably_saturated_flow).then_some(0.0),
            )?;
            set_shortwave_bands(accumulator, 0, shortwave)?;
            set_lct_water_storage(
                accumulator,
                0,
                &state.soil_water,
                0.0,
                state.snow.water_equivalent_kg_m2,
                template.water_storage_tail_mm(&state.soil_water),
            )?;
            set_lct_albedo(
                accumulator,
                0,
                &state.energy,
                reference.surface_cosine_zenith,
            )?;
            set_lct_forcing_mirrors(accumulator, 0, reference, output.precipitation)?;
            for entry in extra {
                match entry {
                    LakeLayers::Scalar(name, value) => accumulator.scalar(name, 0, *value)?,
                    LakeLayers::Layers(name, values) => accumulator.layer(name, 0, values)?,
                }
            }
            Ok(())
        })
    }

    /// 城市 patch 的一步 history（`CoLMMAIN_Urban` 写下的全局量，外加
    /// `MOD_Vars_1DAccFluxes.F90:2219-2243` 只在城市上累加的 20 个量）。
    pub fn push_urban(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: &colm_core::UrbanStepOutput,
        reference: HistoryReferenceState,
    ) -> Result<Option<PathBuf>> {
        let urban = state
            .urban
            .as_ref()
            .context("an urban history record needs the urban state")?;
        let ground = state.surface_temperature_k();
        self.plant_hydraulics = false;
        self.variably_saturated = template.physics.variably_saturated_flow;
        self.urban = true;
        let thermal = &output.thermal;
        let shortwave = &output.shortwave;
        let element_surface = self.element_surface;
        self.push(end, |accumulator| {
            set_lct_snow_state(accumulator, 0, template, state, ground)?;
            let fluxes = vec![
                ("rsur", output.rsur),
                ("rnof", output.rnof),
                ("rsub", output.rnof - output.rsur),
                ("qinfl", output.qinfl),
                ("qintr", output.qintr),
                ("qdrip", output.qdrip),
            ];
            set_ozone_history(accumulator, state.energy.leaf.ozone.as_ref())?;
            set_sidecar_only(
                accumulator,
                template,
                &state.soil_water,
                state.lake.as_ref(),
                thermal.tref,
                state.energy.leaf.canopy_water.rain_mm,
                state.energy.leaf.canopy_water.snow_mm,
                // 城市水文不走 `WATER_VSF`，`qcharge` 每步都有值；但 VSF 打开时
                // `acc1d(qcharge)` 整个不调（`MOD_Vars_1DAccFluxes.F90:2135`）。
                (!template.physics.variably_saturated_flow).then_some(output.qcharge),
            )?;
            // `rnet = sabg + sabvsun + sabvsha - olrg + forc_frl`
            let net_radiation = thermal.sabg + output.sabvsun + 0.0 - thermal.olrg
                + reference.downward_longwave_w_m2;
            for (name, value) in fluxes.into_iter().chain([
                ("fsena", thermal.fsena),
                ("fevpa", thermal.fevpa),
                ("lfevpa", thermal.lfevpa),
                ("fsenl", thermal.fsenl),
                ("fevpl", thermal.fevpl),
                ("etr", thermal.etr),
                ("fseng", thermal.fseng),
                ("fevpg", thermal.fevpg),
                ("fgrnd", thermal.fgrnd),
                ("sabvsun", output.sabvsun),
                ("sabvsha", 0.0),
                ("sabg", thermal.sabg),
                ("olrg", thermal.olrg),
                ("rnet", net_radiation),
                ("emis", thermal.emis),
                ("trad", thermal.trad),
                ("xerr", output.xerr),
                ("zerr", output.zerr),
                ("assim", thermal.assim),
                ("respc", thermal.respc),
                ("rss", thermal.rss),
                // `CoLMDRIVER.F90:354` 把城市的 `rstfac` 接到 `rstfacsun_out`；
                // `rstfacsha_out` 城市分支不碰，保持 `spval`。
                ("rstfacsun", thermal.rstfac),
                ("ldew", state.energy.leaf.canopy_water.total_mm),
                ("sigf", state.energy.canopy.vegetation_free_fraction),
                ("green", 1.0),
                ("laisun", state.energy.canopy.leaf_area_index),
                ("laisha", 0.0),
                ("t_room", urban.t_room),
                ("tafu", urban.tafu),
                ("fhac", urban.fhac),
                ("fwst", urban.fwst),
                ("fach", urban.fach),
                ("fhah", urban.fhah),
                // `a_fahe` 每步累加（`MOD_Vars_1DAccFluxes.F90:2225`）但不写出历史，只进旁车。
                ("fahe", urban.fahe),
                ("fvehc", urban.vehc),
                ("fmeta", urban.meta),
                ("fsenroof", thermal.fsen_roof),
                ("fsenwsun", thermal.fsen_wsun),
                ("fsenwsha", thermal.fsen_wsha),
                ("fsengimp", thermal.fsen_gimp),
                ("fsengper", thermal.fsen_gper),
                ("lfevproof", thermal.lfevp_roof),
                ("lfevpgimp", thermal.lfevp_gimp),
                ("lfevpgper", thermal.lfevp_gper),
                ("t_roof", urban.t_roof),
                ("t_wall", urban.t_wall),
            ]) {
                ensure!(
                    value.is_finite(),
                    "the urban history value for {name} is not finite"
                );
                accumulator.scalar(name, 0, value)?;
            }
            // 城市分支（`CoLMMAIN_Urban`）不碰这些 `*_out`/土壤水量，它们停在分配值 `spval`：
            // `acc1d` 跳过，但写网格时 `filter = patchtype < 99`（`qlayer` 是 `<= 2`）仍把城市 patch 的
            // 面积算进分母。交一次 `spval` 让它计入分母、不进和。`rootr` 在 `CoLMMAIN_Urban` 里是
            // 局部量（`CoLMDRIVER` 只把 patch 的 `rootr(1:,i)` 交给 `CoLMMAIN`），所以同样停在 `spval`。
            for name in [
                "assimsun",
                "assimsha",
                "etrsun",
                "etrsha",
                "gssun",
                "gssha",
                "rstfacsha",
                "frcsat",
                "rsur_se",
            ] {
                accumulator.scalar(name, 0, colm_core::MISSING)?;
            }
            let layers = template.soil_layer_thickness_m().len();
            accumulator.layer("qlayer", 0, &vec![colm_core::MISSING; layers + 1])?;
            accumulator.layer("rootr", 0, &vec![colm_core::MISSING; layers])?;
            // `fsen_urbl`/`lfevp_urbl` 是 `spval` 时 `acc1d` 跳过，文件里留填充值
            for (name, value) in [
                ("fsenurbl", urban.fsen_urbl),
                ("lfevpurbl", urban.lfevp_urbl),
            ] {
                if let Some(value) = value {
                    accumulator.scalar(name, 0, value)?;
                }
            }
            let surface = urban_surface_fluxes(output);
            set_glacier_surface_diagnostics(
                accumulator,
                &surface,
                reference,
                &template.physics,
                element_surface.as_ref(),
            )?;
            set_urban_shortwave_bands(accumulator, 0, shortwave, reference)?;
            set_lct_derived_soil(
                accumulator,
                0,
                template.soil_layer_thickness_m(),
                &state.soil_water,
            )?;
            set_water_storage_with_total(accumulator, 0, &state.soil_water, output.wat)?;
            set_lct_albedo(
                accumulator,
                0,
                &state.energy,
                reference.surface_cosine_zenith,
            )?;
            set_lct_forcing_mirrors(accumulator, 0, reference, &output.precipitation)
        })
    }

    pub fn push_lct_snow(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: &colm_core::StandardLctSnowSoilOutput,
        reference: HistoryReferenceState,
    ) -> Result<Option<PathBuf>> {
        let ground = state.surface_temperature_k();
        self.plant_hydraulics = template.plant_hydraulics();
        self.variably_saturated = template.physics.variably_saturated_flow;
        self.bgc = template.bgc.as_ref().map(|bgc| bgc.switches);
        self.dynamic_wetland = template.physics.dynamic_wetland;
        // 干湖步（`is_dry_lake`）也走这里：patchtype 仍是 4，历史的 `patchtype` 过滤照湖来。
        let dry_lake = template.patch_type == 4;
        if dry_lake {
            self.dynamic_lake = template.physics.dynamic_lake;
        }
        let variably_saturated = self.variably_saturated;
        let element_surface = self.element_surface;
        self.push(end, |accumulator| {
            let lake_filter: &'static [&'static str] = if dry_lake {
                &LAKE_FILTERED_VARIABLES
            } else {
                &[]
            };
            let accumulator = &mut PatchFilteredSink {
                inner: accumulator,
                skipped: lake_filter,
            };
            if let Some(lake) = state.lake.as_ref().filter(|_| dry_lake) {
                // 步末重建过的湖层（`CoLMMAIN.F90:1457-1469`）；动态湖不写 `lake_deficit`。
                accumulator.layer("dz_lake", 0, &lake.column.thickness_m)?;
                accumulator.layer("t_lake", 0, &lake.column.temperature_k)?;
                accumulator.layer("lake_icefrac", 0, &lake.column.ice_fraction)?;
            }
            if let (Some(runtime), Some(bgc)) = (&template.bgc, &state.bgc) {
                let first_pft_class = state
                    .energy
                    .pft
                    .as_ref()
                    .and_then(|pft| pft.parameters.first())
                    .map(|parameters| parameters.class);
                set_bgc_history(
                    accumulator,
                    0,
                    runtime,
                    bgc,
                    first_pft_class,
                    state.irrigation.as_deref(),
                    template.patch_type == 0,
                )?;
            }
            set_lct_snow_state(accumulator, 0, template, state, ground)?;
            set_lct_fluxes(accumulator, 0, &output.water.soil, variably_saturated)?;
            let as_soil = StandardLctSoilOutput {
                energy: output.energy.clone(),
                water: output.water.soil.clone(),
            };
            set_lct_energy_fluxes(accumulator, 0, &as_soil)?;
            set_lct_surface_budget(accumulator, 0, &as_soil)?;
            // 这一句原先漏了：十三个地表诊断量在 `declare_lct_variables` 里声明了，
            // 却从来没有被填过，写出来的 `f_taux`/`f_tauy`/`f_z0m`/`f_zol` … 一直是
            // NetCDF 的填充值。实测 CN-Cng 的积雪分支 history 里这三个是 NaN，
            // 而 Fortran 有值 —— "声明了但没人写"不会报错，只会静默留下一列空洞。
            set_lct_surface_diagnostics_with(
                accumulator,
                0,
                &output.energy,
                reference,
                &template.physics,
                element_surface.as_ref(),
            )?;
            set_lct_stomatal_diagnostics(accumulator, 0, &output.energy)?;
            set_lct_radiation_bands(accumulator, 0, &output.energy)?;
            set_lct_canopy_geometry(accumulator, 0, &state.energy, &output.energy, template)?;
            // `h2osoi`/`wat` 是 `CoLMMAIN` 末尾算的时间变量，在 `bgc_driver` 之前：灌溉的地下水取水
            // （非 VSF）改过的土壤水不进它们；`wa_inst`/`wdsrf_inst` 则是写历史时的状态。
            let main_water = output
                .irrigation_balance
                .as_ref()
                .map_or(&state.soil_water, |balance| &balance.soil_water);
            set_lct_derived_soil(
                accumulator,
                0,
                template.soil_layer_thickness_m(),
                main_water,
            )?;
            let total = lct_water_total(
                main_water,
                state.energy.leaf.canopy_water.total_mm,
                state.snow.water_equivalent_kg_m2,
                template.water_storage_tail_mm(main_water),
            )?;
            set_water_storage_with_total(accumulator, 0, &state.soil_water, total)?;
            set_lct_canopy_water(accumulator, 0, &state.energy, &output.energy)?;
            set_lct_soil_resistance(accumulator, 0, &output.energy)?;
            set_lct_albedo(
                accumulator,
                0,
                &state.energy,
                reference.surface_cosine_zenith,
            )?;
            set_lct_forcing_mirrors(accumulator, 0, reference, &output.energy.precipitation)?;
            // 湿地（patchtype 2）：`endwb` 在 VSF 时再加 `wetwat`（`CoLMMAIN.F90:1485-1489`），
            // 非 VSF 时 `errorw = 0`（`:1532`）；`f_wetwat*`/`f_wetzwt` 只在湿地上写
            // （`MOD_Hist.F90` 的 `filter = patchtype == 2`）。
            let wetland = template.patch_type == 2;
            set_ozone_history(accumulator, state.energy.leaf.ozone.as_ref())?;
            set_sidecar_only(
                accumulator,
                template,
                &state.soil_water,
                state.lake.as_ref(),
                output.energy.leaf.air_temperature_2m_k,
                state.energy.leaf.canopy_water.rain_mm,
                state.energy.leaf.canopy_water.snow_mm,
                (!variably_saturated).then_some(output.water.soil.recharge_mm_s),
            )?;
            // 灌溉打开时 `endwb` 取 `bgc_driver` 之前的土壤水与 `waterstorage`（见
            // `StandardLctSnowSoilOutput::irrigation_balance`）；其余历史量是 BGC 之后的状态。
            let (balance_water, irrigation_storage) = match &output.irrigation_balance {
                Some(balance) => (&balance.soil_water, Some(balance.storage_mm)),
                None => (&state.soil_water, None),
            };
            let mut end_water = colm_core::total_water_storage_mm(
                balance_water,
                state.energy.leaf.canopy_water.total_mm,
                state.snow.water_equivalent_kg_m2,
                irrigation_storage,
            );
            if wetland && variably_saturated {
                end_water += state.soil_water.wetland_water_mm;
            }
            if wetland && !variably_saturated {
                set_lct_balance_errors(
                    &mut DroppedSink {
                        inner: &mut *accumulator,
                        dropped: &["xerr"],
                    },
                    0,
                    &as_soil,
                    end_water,
                    reference,
                )?;
                accumulator.scalar("xerr", 0, 0.0)
            } else {
                set_lct_balance_errors(accumulator, 0, &as_soil, end_water, reference)
            }
        })
    }

    /// 收尾：把还开着的那个分组落盘。调度已经保证运行结束那一刻会写一条，所以正常
    /// 情况下这里只是把缓冲区写出。
    pub fn finish(&mut self) -> Result<Vec<PathBuf>> {
        let mut written = self.finish_main()?;
        if let Some(path) = self.finish_tracer()? {
            written.push(path);
        }
        Ok(written)
    }

    /// 只落盘主 history 当前的分组（推进过程中换组、组满时用；示踪物文件由 [`Self::push_tracer`]
    /// 自己在组满时写）。
    fn finish_main(&mut self) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        if let Some((suffix, buffer)) = self.open.take() {
            written.push(self.write(&suffix, &buffer)?);
        }
        Ok(written)
    }

    /// 打开示踪物 history（有输运示踪物时；`tracer_hist_out`）。
    pub fn with_tracer_variables(
        mut self,
        set: colm_core::tracer::TracerSet,
        patch_types: Vec<i32>,
    ) -> Self {
        // 示踪物文件的平均用主 history 的 `nac`：续跑接着累加时（先 `restore` 了窗口）从那里接上。
        let steps = self
            .accumulators
            .first()
            .map_or(0, |accumulator| accumulator.steps);
        self.tracer_variables = Some(TracerHistoryState {
            set,
            patch_types,
            cursor: 0,
            steps,
            open: None,
        });
        self
    }

    /// 一步结束、主 history 已推进之后调用（`hist_out` 里的 `tracer_hist_out` 与
    /// `flush_Tracer_Acc`）。预热期直接返回，既不计步也不清零（上游 `hist_out` 提前返回）。
    pub fn push_tracer(
        &mut self,
        end: CalendarTime,
        is_spinup: bool,
        states: &mut [colm_core::StandardLctSnowSoilState],
    ) -> Result<Option<PathBuf>> {
        use colm_core::tracer::hist;
        // `forcmask_pch`：示踪物历史的各 `filter` 都与上它（`MOD_Tracer_Hist.F90:229/261/644/657`）。
        let forcing_mask = self.forcing_mask.clone();
        let forcmask_ok = |patch: usize| {
            forcing_mask
                .as_ref()
                .is_none_or(|mask| mask.get(patch).copied().unwrap_or(true))
        };
        let Some(tracer) = self.tracer_variables.as_mut() else {
            return Ok(None);
        };
        if is_spinup {
            return Ok(None);
        }
        tracer.steps += 1;
        // CH4 `core` history：每步累加（`accumulate_methane_fluxes`），与示踪物同在预热期之后。
        for (patch, state) in states.iter_mut().enumerate() {
            if let Some(bgc) = state.bgc.as_deref_mut() {
                if let Some(methane) = bgc.methane.as_deref() {
                    bgc.methane_acc
                        .accumulate(methane, tracer.patch_types[patch]);
                }
            }
        }
        let Some(record) = self.records.get(tracer.cursor).cloned() else {
            return Ok(None);
        };
        if tick_of(end)? != record.write_at_tick {
            return Ok(None);
        }
        let records_in_group = self
            .records
            .iter()
            .filter(|scheduled| scheduled.suffix == record.suffix)
            .count();
        if tracer.open.as_ref().map(|(suffix, _, _)| suffix) != Some(&record.suffix) {
            let mut buffer = HistoryBuffers::new(self.dimensions, self.site, records_in_group);
            if let Some(grid) = &self.grid {
                buffer = buffer.with_grid(std::sync::Arc::clone(grid))?;
                // 网格的示踪物文件与主文件一样带窗口变量（`MOD_Hist.F90:319-361`）。
                if self.tracer_time_step_seconds.is_some() {
                    buffer.enable_windows();
                }
            }
            if let Some(vector) = &self.vector {
                buffer = buffer.with_vector(std::sync::Arc::clone(vector))?;
            }
            tracer.open = Some((record.suffix.clone(), buffer, Vec::new()));
        }
        let nac = tracer.steps as f64;
        let patches = states.len();
        let grid = self.grid.clone();
        let vector = self.vector.clone();
        let (_, buffer, variables) = tracer.open.as_mut().expect("just opened");
        buffer.set_time(record.record, i32::try_from(record.label_minutes)?)?;
        if let (Some(deltim), true) = (self.tracer_time_step_seconds, grid.is_some()) {
            let minutes = colm_hist::time::minutes_from_1900(end.year)
                + (i64::from(end.julian_day) - 1) * 1440
                + i64::from(end.seconds / 60);
            let end_minutes = minutes as f64 + f64::from(end.seconds % 60) / 60.0;
            buffer.set_window(record.record, nac * deltim, end_minutes)?;
        }
        // 文件里的变量：逐示踪物、逐表项（同 `tracer_hist_out` 的写出顺序）。
        let mut index = 0;
        for itrc in tracer.set.transport_indices() {
            let descriptor = &tracer.set.tracers[itrc];
            for variable in hist::TRACER_HISTORY_VARIABLES.iter() {
                let applies = match variable.scope {
                    hist::TracerScope::Isotope => descriptor.is_isotope(),
                    hist::TracerScope::NonIsotope => !descriptor.is_isotope(),
                    hist::TracerScope::NonvolatileSolute => descriptor.is_nonvolatile_solute(),
                    hist::TracerScope::Transport => true,
                };
                if !applies {
                    continue;
                }
                let layered = matches!(variable.dims, hist::TracerHistDims::SoilSnow);
                let width = if layered {
                    colm_core::tracer::SOISNO_LAYERS
                } else {
                    1
                };
                // 网格写出时第二维是格子（`lat*lon`），向量是单元，单点是 patch。
                let columns = match (grid.as_ref(), vector.as_ref()) {
                    (Some(grid), _) => grid.lat.len() * grid.lon.len(),
                    (None, Some(vector)) => vector.elmindex.len(),
                    (None, None) => patches,
                };
                if variables.len() == index {
                    variables.push(colm_hist::history::TracerFileVariable {
                        name: variable.variable_name(descriptor),
                        long_name: variable.long_name(descriptor),
                        units: variable.units(descriptor).to_owned(),
                        layered,
                        values: vec![colm_core::MISSING; records_in_group * columns * width],
                    });
                }
                let file_variable = &mut variables[index];
                if let Some(vector) = vector.as_ref() {
                    // `MOD_Tracer_Hist` 的 `Vector` 支（`write_history_tracer_vector_2d/3d` 与
                    // `aggregate_to_vector_and_write_2d`）：比值/δ 先在单元里对示踪物量与水量各做
                    // `sum(subfrc*x, mask)` 再相除；均值类按 `subfrc` 加权平均。`filter` 是变量自己的。
                    let track = |patch: usize| -> Result<&colm_core::tracer::PatchTracerState> {
                        states[patch]
                            .tracer
                            .as_deref()
                            .map(|track| &track.state)
                            .context("a tracer history needs every patch to carry tracer state")
                    };
                    let tracks = (0..patches).map(track).collect::<Result<Vec<_>>>()?;
                    let keep = |p: usize| {
                        variable
                            .patch_filter
                            .admits(tracer.patch_types[p], forcmask_ok(p), true)
                    };
                    let ref_ratio = descriptor.ref_ratio;
                    let elements = columns;
                    let base = record.record * elements * width;
                    if layered {
                        let pairs: Vec<_> = tracks
                            .iter()
                            .map(|state| hist::soisno_layer_pairs(itrc, state))
                            .collect();
                        // `layer` 索引的是每个 patch 的内层，不是 `pairs` 本身。
                        #[allow(clippy::needless_range_loop)]
                        for layer in 0..width {
                            let sums = vector.pair_sums(
                                |p| pairs[p][layer],
                                |p| {
                                    let (mass, water) = pairs[p][layer];
                                    water.abs() > colm_core::tracer::TRC_TINY
                                        && mass != colm_core::MISSING
                                        && keep(p)
                                },
                            );
                            for (element, (mass, water)) in sums.into_iter().enumerate() {
                                file_variable.values[base + element * width + layer] =
                                    if water.abs() > colm_core::tracer::TRC_TINY {
                                        mass / water
                                    } else {
                                        colm_core::MISSING
                                    };
                            }
                        }
                    } else {
                        let terms: Vec<_> = tracks
                            .iter()
                            .map(|state| hist::patch_term(variable, descriptor, itrc, state, nac))
                            .collect();
                        let pair = |p: usize| match terms[p] {
                            hist::PatchTerm::Pair { mass, water } => (mass, water),
                            hist::PatchTerm::Scalar(_) => (colm_core::MISSING, 0.0),
                        };
                        let values: Vec<f64> = match variable.kind {
                            hist::TracerHistKind::Ratio | hist::TracerHistKind::LayerRatio => {
                                let threshold = colm_core::tracer::TRC_TINY;
                                vector
                                    .pair_sums(pair, |p| {
                                        let (mass, water) = pair(p);
                                        water.abs() > threshold
                                            && mass != colm_core::MISSING
                                            && keep(p)
                                    })
                                    .into_iter()
                                    .map(|(mass, water)| {
                                        if water.abs() > threshold {
                                            mass / water
                                        } else {
                                            colm_core::MISSING
                                        }
                                    })
                                    .collect()
                            }
                            hist::TracerHistKind::Delta { water_min } => vector
                                .pair_sums(pair, |p| {
                                    let (mass, water) = pair(p);
                                    water > water_min && mass != colm_core::MISSING && keep(p)
                                })
                                .into_iter()
                                .map(|(mass, water)| {
                                    if water > water_min {
                                        let delta = hist::mass_to_delta(mass, water, ref_ratio);
                                        if delta != colm_core::MISSING
                                            && delta.abs() <= hist::TRC_DELTA_SANITY_MAX
                                        {
                                            return delta;
                                        }
                                    }
                                    colm_core::MISSING
                                })
                                .collect(),
                            hist::TracerHistKind::Mean | hist::TracerHistKind::AreaState => vector
                                .aggregate(
                                    |p| match terms[p] {
                                        hist::PatchTerm::Scalar(value) => value,
                                        hist::PatchTerm::Pair { .. } => colm_core::MISSING,
                                    },
                                    keep,
                                    false,
                                ),
                        };
                        file_variable.values[base..base + elements].copy_from_slice(&values);
                    }
                    index += 1;
                    continue;
                }
                if let Some(grid) = grid.as_ref() {
                    // `tracer_hist_out` 的网格支：`sumarea` 用陆面 `filter`（`patchtype < 99`，
                    // `patchmask` 在 Rust 空间运行里恒真），分子按各变量自己的 `filter`，
                    // `pset2grid` 按 patch 序、再按份序累加。
                    let mut cells = vec![hist::GridCell::default(); columns * width];
                    for (patch, state) in states.iter().enumerate() {
                        let track = state
                            .tracer
                            .as_deref()
                            .context("a tracer history needs every patch to carry tracer state")?;
                        let patch_type = tracer.patch_types[patch];
                        let land =
                            hist::PatchFilter::Land.admits(patch_type, forcmask_ok(patch), true);
                        let patch_ok =
                            variable
                                .patch_filter
                                .admits(patch_type, forcmask_ok(patch), true);
                        let pairs = layered.then(|| hist::soisno_layer_pairs(itrc, &track.state));
                        let term = (!layered).then(|| {
                            hist::patch_term(variable, descriptor, itrc, &track.state, nac)
                        });
                        for &(cell, area) in &grid.parts[patch] {
                            for layer in 0..width {
                                let target = &mut cells[layer * columns + cell];
                                if land {
                                    target.add_area(area);
                                }
                                match (&pairs, term) {
                                    (Some(pairs), _) => {
                                        let (mass, water) = pairs[layer];
                                        target.add_layer(mass, water, area, patch_ok);
                                    }
                                    (None, Some(term)) => {
                                        target.add(variable, term, area, patch_ok)
                                    }
                                    (None, None) => {
                                        unreachable!("a 2-D variable always has a term")
                                    }
                                }
                            }
                        }
                    }
                    let base = record.record * columns * width;
                    for (offset, cell) in cells.iter().enumerate() {
                        file_variable.values[base + offset] =
                            cell.finish(variable, descriptor.ref_ratio);
                    }
                    index += 1;
                    continue;
                }
                for (patch, state) in states.iter().enumerate() {
                    let track = state
                        .tracer
                        .as_deref()
                        .context("a tracer history needs every patch to carry tracer state")?;
                    let patch_ok = variable.patch_filter.admits(
                        tracer.patch_types[patch],
                        forcmask_ok(patch),
                        true,
                    );
                    if layered {
                        let values = hist::single_point_soisno(itrc, &track.state, nac, patch_ok);
                        let base =
                            (record.record * patches + patch) * colm_core::tracer::SOISNO_LAYERS;
                        file_variable.values[base..base + colm_core::tracer::SOISNO_LAYERS]
                            .copy_from_slice(&values);
                    } else {
                        file_variable.values[record.record * patches + patch] =
                            hist::single_point_value(
                                variable,
                                descriptor,
                                itrc,
                                &track.state,
                                nac,
                                patch_ok,
                            );
                    }
                }
                index += 1;
            }
        }
        // CH4 `core` 变量（`methane_reactive_history`，排在示踪物变量之后）：单点 patch 维。
        let has_methane = states.iter().any(|state| {
            state
                .bgc
                .as_deref()
                .is_some_and(|bgc| bgc.methane.is_some())
        });
        if has_methane {
            let template = colm_core::methane::driver::CoreAccumulator::default()
                .core_values(false, false, false);
            // 网格写出时第二维是格子（`lat*lon`），向量是单元，单点是 patch。
            let columns = match (grid.as_ref(), vector.as_ref()) {
                (Some(grid), _) => grid.lat.len() * grid.lon.len(),
                (None, Some(vector)) => vector.elmindex.len(),
                (None, None) => patches,
            };
            for (k, (name, long_name, units, _)) in template.iter().enumerate() {
                if variables.len() == index + k {
                    variables.push(colm_hist::history::TracerFileVariable {
                        name: (*name).to_owned(),
                        long_name: (*long_name).to_owned(),
                        units: (*units).to_owned(),
                        layered: false,
                        values: vec![colm_core::MISSING; records_in_group * columns],
                    });
                }
            }
            // 每个 patch 的 `core` 值：不在该变量 `filter` 里的已是 `spval`（`core_values` 的
            // `with_filter`），与 `pset2grid(msk = filter)` 跳过它们等价。
            let mut per_patch = Vec::with_capacity(patches);
            for (patch, state) in states.iter().enumerate() {
                let patch_type = tracer.patch_types[patch];
                // `methane_patch_active_mask`：湿地、（非 `only_wetland` 时的）土壤与有稻田的土壤。
                let active = state
                    .bgc
                    .as_deref()
                    .and_then(|bgc| bgc.methane.as_deref())
                    .map_or(patch_type == 0 || patch_type == 2, |methane| {
                        methane.history_active_or_default(patch_type)
                    });
                let land = patch_type < 99;
                let acc = state
                    .bgc
                    .as_deref()
                    .map(|bgc| bgc.methane_acc)
                    .unwrap_or_default();
                // 湖只有开了 `allowlakeprod` 才跑甲烷（跑过就有 `last`）。
                let lake = patch_type == 4
                    && state
                        .bgc
                        .as_deref()
                        .and_then(|bgc| bgc.methane.as_deref())
                        .is_some_and(|methane| methane.last.is_some());
                per_patch.push((active, land, acc.core_values(active, land, lake)));
            }
            if let Some(vector) = vector.as_ref() {
                // 向量支：`write_history_variable_2d` → `aggregate_to_vector_and_write_2d`（平均）。
                // 不在各变量 `filter` 里的 patch 值已是 `spval`，聚合时跳过。
                for k in 0..template.len() {
                    let values = vector.aggregate(|p| per_patch[p].2[k].3, |_| true, false);
                    let base = record.record * columns;
                    variables[index + k].values[base..base + columns].copy_from_slice(&values);
                }
            } else if let Some(grid) = grid.as_ref() {
                // `methane_reactive_history` 的网格支：前 9 个变量（活跃面积均值）的 `sumarea`
                // 取活跃掩膜（`get_sumarea(sumarea, filter)`），其后 10 个陆面面积均值换成
                // `filter_all_land`（`:821`）。两组都按 patch 序、再按份序累加（`pset2grid`）。
                const ACTIVE_MEAN: usize = 9;
                for k in 0..template.len() {
                    let mut cells = vec![hist::GridCell::default(); columns];
                    for (patch, (active, land, values)) in per_patch.iter().enumerate() {
                        let in_area = if k < ACTIVE_MEAN { *active } else { *land };
                        for &(cell, area) in &grid.parts[patch] {
                            if in_area {
                                cells[cell].add_area(area);
                            }
                            cells[cell].add_mean(values[k].3, area);
                        }
                    }
                    let base = record.record * columns;
                    for (offset, cell) in cells.iter().enumerate() {
                        variables[index + k].values[base + offset] = cell.finish_mean();
                    }
                }
            } else {
                for (patch, (_, _, values)) in per_patch.iter().enumerate() {
                    for (k, (_, _, _, value)) in values.iter().enumerate() {
                        variables[index + k].values[record.record * patches + patch] = *value;
                    }
                }
            }
        }
        // 运行终点不在自然边界上：上游先存原始窗口再写这一条（`MOD_Hist.F90:270`）。
        if !record.natural_boundary {
            let raw = states
                .iter()
                .map(|state| crate::tracer_sidecar::RawTracers {
                    tracer: state
                        .tracer
                        .as_deref()
                        .map(|track| crate::tracer_sidecar::PatchAccumulators::of(&track.state)),
                    methane: state
                        .bgc
                        .as_deref()
                        .filter(|bgc| bgc.methane.is_some())
                        .map(|bgc| bgc.methane_acc),
                })
                .collect();
            *self.tracer_raw_at_end.lock().expect("tracer raw lock") = Some(raw);
        }
        for state in states.iter_mut() {
            if let Some(track) = state.tracer.as_deref_mut() {
                track.state.flush_accumulators();
            }
            if let Some(bgc) = state.bgc.as_deref_mut() {
                bgc.methane_acc = colm_core::methane::driver::CoreAccumulator::default();
            }
        }
        tracer.cursor += 1;
        tracer.steps = 0;
        if record.record + 1 == records_in_group {
            return self.finish_tracer();
        }
        Ok(None)
    }

    fn finish_tracer(&mut self) -> Result<Option<PathBuf>> {
        let Some(tracer) = self.tracer_variables.as_mut() else {
            return Ok(None);
        };
        let Some((suffix, buffer, variables)) = tracer.open.take() else {
            return Ok(None);
        };
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create {}", self.directory.display()))?;
        let path = self
            .directory
            .join(format!("{}_hist_tracer_{suffix}.nc", self.stem));
        buffer.write_tracer_file(&path, &variables)?;
        Ok(Some(path))
    }

    /// 收下一步：**先累加**，到调度命中的那一步再写区间平均并清零。
    ///
    /// 顺序与上游一致：`MOD_Hist.F90:227` 每步 `accumulate_fluxes`（写出的那一步也算），
    /// 写出时 `acc_vec = acc_vec / nac`，写完后 `CALL FLUSH_acc_fluxes ()`（`:4746`）。
    fn push(
        &mut self,
        end: CalendarTime,
        accumulate: impl FnOnce(&mut HistoryAccumulator) -> Result<()>,
    ) -> Result<Option<PathBuf>> {
        let pushed = self.push_inner(end, accumulate);
        // 一步的全部 patch 都累加完才更新共享快照（写续跑文件在步末）。
        if self.patch_cursor == 0 {
            let windows = self.raw_at_end.take().unwrap_or_else(|| {
                self.accumulators
                    .iter()
                    .map(HistoryAccumulator::window)
                    .collect()
            });
            *self.window.lock().expect("history window lock") = windows;
        }
        pushed
    }

    fn push_inner(
        &mut self,
        end: CalendarTime,
        accumulate: impl FnOnce(&mut HistoryAccumulator) -> Result<()>,
    ) -> Result<Option<PathBuf>> {
        // 1. 每步累加。失败也要把累加器放回去，否则下一次调用从零开始，
        //    会静默丢掉这一段。
        let patch = self.patch_cursor;
        let mut accumulator = std::mem::take(&mut self.accumulators[patch]);
        accumulator.steps += 1;
        let filled = accumulate(&mut accumulator);
        self.accumulators[patch] = accumulator;
        if filled.is_err() {
            self.patch_cursor = 0;
        }
        filled?;
        // 多 patch：这一步还有 patch 没累加，写出等最后一个。
        if patch + 1 < self.accumulators.len() {
            self.patch_cursor = patch + 1;
            return Ok(None);
        }
        self.patch_cursor = 0;

        // 2. 没到期就到此为止。
        let Some(record) = self.records.get(self.cursor).cloned() else {
            // 记录写完之后的步不再产生输出 —— 这在"运行比窗口长"时是正常的收尾。
            return Ok(None);
        };
        let tick = tick_of(end)?;
        if tick != record.write_at_tick {
            ensure!(
                tick < record.write_at_tick,
                "the run reached {tick} but the next history record was due at {}; the schedule \
                 and the clock disagree",
                record.write_at_tick
            );
            return Ok(None);
        }

        // 3. 到期：取平均、写记录、把累加器清零（`mem::take` 就是清零）。
        //    运行终点不在自然边界上时，上游先把原始窗口存进旁车再写这一条。
        if !record.natural_boundary {
            self.raw_at_end = Some(
                self.accumulators
                    .iter()
                    .map(HistoryAccumulator::window)
                    .collect(),
            );
        }
        let means: Vec<HistoryAccumulator> =
            self.accumulators.iter_mut().map(std::mem::take).collect();
        let mut written = None;
        if self.open.as_ref().map(|(suffix, _)| suffix) != Some(&record.suffix) {
            written = self.finish_main()?.pop();
            let mut buffer = HistoryBuffers::new(
                self.dimensions,
                self.site,
                self.record_count(&record.suffix),
            );
            if let Some(selection) = &self.selection {
                buffer = buffer.with_selection(std::sync::Arc::clone(selection));
            }
            if let Some(grid) = &self.grid {
                buffer = buffer.with_grid(std::sync::Arc::clone(grid))?;
                if !self.gridded_names.is_empty() {
                    buffer.declare_gridded(&self.gridded_names)?;
                }
            }
            if let Some(vector) = &self.vector {
                // 上游 `write_history_variable_urb_2d` 的 `Vector` 支自己标了 TODO：把城市长度（`numurban`）的
                // 累加数组当 patch 长度的向量交给 `aggregate_to_vector_and_write_2d`（越界读、也不除 `nac`），
                // 没有确定的结果可对齐。
                ensure!(
                    !self.urban,
                    "vector history with urban variables has no defined upstream result (the Vector \
                     branch of write_history_variable_urb_2d passes urban-length arrays as patch \
                     vectors; upstream marks it TODO); use gridded history"
                );
                buffer = buffer.with_vector(std::sync::Arc::clone(vector))?;
                buffer.declare_patch_fields(&self.patch_field_names)?;
                for (name, long_name, units, values) in &self.vector_statics {
                    buffer.add_vector_static(name, long_name, units, values.clone())?;
                }
            }
            // 声明本层能负责的变量；写出的文件因此只包含它们。
            declare_lct_variables(&mut buffer, self.plant_hydraulics, self.variably_saturated)?;
            if self.urban {
                buffer.declare(&URBAN_VARIABLES)?;
            }
            if let Some(switches) = self.bgc {
                buffer.declare(&bgc_history_variables(switches))?;
            }
            // `MOD_Hist.F90:4518`/`:4533`：动态湖写 `f_dz_lake`、不写 `f_lake_deficit`。
            if self.dynamic_lake {
                buffer.declare(&["dz_lake"])?;
                buffer.undeclare("lake_deficit");
            }
            if self.tracer_time_step_seconds.is_some() {
                buffer.enable_windows();
            }
            // `DEF_USE_CBL_HEIGHT`：`acc1d (forc_hpbl, a_hpbl)` → `f_xy_hpbl`（`MOD_Hist.F90:538`）。
            // `DEF_USE_OZONESTRESS`：`f_o3uptakesun/sha`（`MOD_Hist.F90:545-553`）；`f_xy_ozone` 写在
            // `IF (DEF_USE_BGC)` 段里（`:2357-2362`），不开 BGC 时只累加进旁车。
            if means
                .iter()
                .any(|means| means.offered.contains("o3uptakesun"))
            {
                buffer.declare(&["o3uptakesun", "o3uptakesha"])?;
                if self.bgc.is_some() {
                    buffer.declare(&["xy_ozone"])?;
                }
            }
            if means.iter().any(|means| means.offered.contains("xy_hpbl")) {
                buffer.declare(&["xy_hpbl"])?;
            }
            self.open = Some((record.suffix.clone(), buffer));
        }
        let forcing_mask = self.forcing_mask.clone();
        let (_, buffer) = self.open.as_mut().expect("just opened");
        buffer.set_time(
            record.record,
            i32::try_from(record.label_minutes).with_context(|| {
                format!(
                    "the history label {} does not fit an i32",
                    record.label_minutes
                )
            })?,
        )?;
        if let Some(deltim) = self.tracer_time_step_seconds {
            // `history_window_seconds = max(nac,0)*deltim`；末时刻 `minutes_since_1900(idate)
            // + mod(sec,60)/60`。
            let steps = means.first().map_or(0, |means| means.steps);
            let minutes = colm_hist::time::minutes_from_1900(end.year)
                + (i64::from(end.julian_day) - 1) * 1440
                + i64::from(end.seconds / 60);
            let end_minutes = minutes as f64 + f64::from(end.seconds % 60) / 60.0;
            buffer.set_window(record.record, steps as f64 * deltim, end_minutes)?;
        }
        for (patch, means) in means.iter().enumerate() {
            // 被强迫缺测遮蔽的 patch 不进聚合（值与分母都没有它），只有 CROP 那几个没与上
            // `forcmask_pch` 的作物历史除外（见 [`PatchMeans::write_masked_crop_means`]）。
            if forcing_mask
                .as_ref()
                .is_some_and(|mask| !mask.get(patch).copied().unwrap_or(true))
            {
                if self.bgc.is_some_and(|bgc| bgc.crop) && means.steps > 0 {
                    if means_are_split(self.accumulators.len()) {
                        buffer.select_patch(Some(patch))?;
                    }
                    means.write_masked_crop_means(buffer, record.record)?;
                }
                continue;
            }
            if means_are_split(self.accumulators.len()) {
                buffer.select_patch(Some(patch))?;
            }
            means.write_means(buffer, record.record)?;
            if self.dynamic_wetland {
                means.write_dynamic_wetland_storage(buffer, record.record)?;
            }
        }
        buffer.select_patch(None)?;
        for (name, values) in self.staged.drain(..) {
            buffer.set_gridded(name, record.record, &values)?;
        }
        for (name, values, included) in self.staged_patch.drain(..) {
            buffer.set_patch_field(name, record.record, &values, &included)?;
        }
        self.cursor += 1;
        // 分组的最后一条写完就落盘（上游写回模式在这一刻把内存里的整组写出）；开着的缓冲区因此
        // 只会是写了一半的组，中途 abort 时由 [`Self::abandon`] 留下只有文件头的文件。
        if record.record + 1 == self.record_count(&record.suffix) {
            if let Some(path) = self.finish_main()?.pop() {
                written = Some(path);
            }
        }
        Ok(written)
    }

    /// 运行中途失败：写到一半的那组留下只有文件头的文件（与上游 abort 后磁盘上的状态一致）。
    pub fn abandon(&mut self) -> Result<Option<PathBuf>> {
        let Some((suffix, buffer)) = self.open.take() else {
            return Ok(None);
        };
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create {}", self.directory.display()))?;
        let path = self
            .directory
            .join(format!("{}_hist_{suffix}.nc", self.stem));
        buffer.write_header(&path)?;
        Ok(Some(path))
    }

    /// 某个后缀有多少条记录。
    fn record_count(&self, suffix: &str) -> usize {
        self.records
            .iter()
            .filter(|record| record.suffix == suffix)
            .count()
    }

    fn write(&self, suffix: &str, buffer: &HistoryBuffers) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("cannot create {}", self.directory.display()))?;
        let path = self
            .directory
            .join(format!("{}_hist_{suffix}.nc", self.stem));
        buffer.write(&path)?;
        if self.tracer_time_step_seconds.is_some() && self.tracer_variables.is_none() {
            buffer.write_tracer_skeleton(
                self.directory
                    .join(format!("{}_hist_tracer_{suffix}.nc", self.stem)),
            )?;
        }
        Ok(path)
    }
}

/// `CalendarTime` → tick（秒）。
fn tick_of(time: CalendarTime) -> Result<i64> {
    colm_hist::schedule::tick_seconds(
        time.year,
        i32::from(time.julian_day),
        i32::try_from(time.seconds)
            .with_context(|| format!("{} seconds does not fit an i32", time.seconds))?,
    )
}

/// 一个 POINT 算例的 history 维度：一个 patch、CoLM 编译期的层级长度。
///
/// 这些长度由内核的编译期常量决定（`nl_soil = 10`、`maxsnl = -5`、`nvegwcs = 4`…），
/// 不由算例文件携带。
pub fn point_dimensions() -> HistoryDimensions {
    HistoryDimensions {
        patch: 1,
        soil: 10,
        lake: 10,
        snow_layers: 5,
        vegnodes: 4,
        band: 2,
        radiation_types: 2,
        sensor: 1,
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
