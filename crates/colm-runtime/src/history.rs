//! 把每步的 LCT 状态接到 history 写出器上。
//!
//! `colm-hist` 已经有闸门表（哪些变量写得出来）与调度（什么时候写），也已经有写出器；
//! 缺的是"每步的字段值"这一层。这里只填**状态真正拥有**的那些量 —— 与续跑写回用的是
//! 同一份来源（见 `assembly.rs` 的 `evolved_overrides`），所以两处不会各说一套。
//!
//! **没填的变量不声明。** `HistoryBuffers::declare` 只接受调用方点名的变量，写出的文件
//! 因此只包含本层能负责的那些；`UNFILLED` 列出还差什么，免得"文件里没有"被当成
//! "这个内核产不出"。补齐它们要么需要更多内核输出，要么需要先核对上游对每个诊断量的
//! 定义（例如 `h2osoi` 是液态还是液+固态），不是把名字填上就算数。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_core::{CalendarTime, StandardLctSoilOutput};
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
pub const LCT_FLUX_VARIABLES: [&str; 6] = ["qinfl", "rnof", "rsub", "rsur", "qcharge", "frcsat"];

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
pub const INSTANTANEOUS_VARIABLES: [&str; 3] = ["wa_inst", "wdsrf_inst", "wat_inst"];

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
pub const LCT_CANOPY_VARIABLES: [&str; 3] = ["sigf", "laisun", "laisha"];

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
pub const LCT_FORCING_VARIABLES: [&str; 9] = [
    "xy_t",
    "xy_q",
    "xy_pbot",
    "xy_us",
    "xy_vs",
    "xy_solarin",
    "xy_frl",
    "xy_prc",
    "xy_prl",
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
pub const DECLARED_ONLY: [&str; 3] = ["sensors", "rsur_ie", "rsur_se"];

/// 黄金算例（CN-Cng）里没有、但本层仍会声明的量。
///
/// 闸门表允许写不等于这个算例会产出：`qcharge` 受运行时条件控制，黄金算例没触发。
/// schema 测试因此按"两边都有"来比，并把跳过的名字记下来。
pub const NOT_IN_GOLDEN: [&str; 1] = ["qcharge"];

/// 黄金算例里有、但本层还填不出来的量（按用途分组，便于下一步挑）。
///
/// 这份清单不参与写出，只是把"缺口"写死在代码里：改它就得同时改注释。
pub const UNFILLED: [&str; 4] = [
    "`green`：上游由 `MOD_LAIEmpirical.F90:132-135` 从 `fveg = vegc(ivt)` 得出，而 `vegc` 是该模块内的硬编码表（IGBP 那支 17 项：15=Snow/Ice、17=Water 为 0，其余 1），本仓库还没搬；本算例地类 10 恒为 1，但\"恒为 1\"不是实现依据",
    "`rss`：普通 `acc1d` + `filter`/`nac`，规则已明确；卡在方案 4 下上游写的是电导标志而不是阻力，条件映射待核对",
    "`xerr`/`zerr`/`xy_rain`/`xy_snow`：四项都是普通 `acc1d` + `filter`/`nac`，值也在（水平衡残差、能量平衡残差、雨雪拆分），只差接线与各自残差的定义核对",
    "`us10m`/`vs10m`/`fm10m`/`ustar2`（另一支 `Shaofeng, 2023` 廓线 routine）、`t_lake`/`lake_icefrac`/`lake_deficit`（湖泊分支）、`wetwat`/`wetwat_inst`/`wetzwt`（湿地分支）：整支 routine 或分支尚未驱动",
];
/// 声明本层能填的全部变量：状态十三项 + 水文六项 + 能量四项 + 地表十三项。
pub fn declare_lct_variables(buffer: &mut HistoryBuffers) -> Result<()> {
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
}

impl HistorySink for HistoryBuffers {
    fn scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        self.set_patch_scalar(name, record, value)
            .with_context(|| format!("cannot write {name} into the history buffers"))
    }

    fn layer(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        self.set_layered(name, record, values)
            .with_context(|| format!("cannot write {name} into the history buffers"))
    }
}

/// 一个输出区间里逐变量的和与步数（上游的 `a_*` 与 `nac`）。
#[derive(Debug, Default)]
struct HistoryAccumulator {
    sums: std::collections::BTreeMap<String, Accumulated>,
    steps: usize,
}

#[derive(Debug)]
enum Accumulated {
    Scalar { sum: f64, count: usize },
    Column { sum: Vec<f64>, count: usize },
}

impl HistoryAccumulator {
    /// 按**每个变量自己的**有效步数取平均后写进第 `record` 条。标量/列由**累加时**
    /// 的形态决定，不在这里猜 —— 猜错会把一根土柱按标量写出去。
    ///
    /// **除数不是全局步数是刻意的。** 上游 `acc1d` 会跳过 `spval`
    /// （`MOD_Vars_1DAccFluxes.F90:2895` `IF (var(i) /= spval)`），而除数按变量分组
    /// 各有一个计数器：`nac`（每步 +1）、`nac_dt`（只数白天）、
    /// `nac_ln`（只数 `solvdln /= spval` 的步，`:2041`）。
    /// 于是"局部有效"的量写出来是**它自己的平均**而不是被无效步稀释的值 ——
    /// 逐位实测：`f_solvdln` 在 264 条里有 11 条是本地正午的真值、其余 253 条是 spval，
    /// 而那 11 条的值约等于同一小时的 `f_solvd`（比值 1.02），不是它的一半。
    fn write_means(&self, buffer: &mut HistoryBuffers, record: usize) -> Result<()> {
        ensure!(
            self.steps > 0,
            "the history accumulator reached a write step without accumulating anything"
        );
        for (name, accumulated) in &self.sums {
            match accumulated {
                Accumulated::Scalar { sum, count } => {
                    // 整条记录里一次有效值都没有的变量沿用缓冲区的填充值，
                    // 与上游"累加器一直是 spval、除以 0 个样本仍是 spval"同效。
                    if *count == 0 {
                        continue;
                    }
                    let scale = 1.0 / *count as f64;
                    buffer
                        .set_patch_scalar(name, record, sum * scale)
                        .with_context(|| format!("cannot write {name} into the history buffers"))?;
                }
                Accumulated::Column { sum, count } => {
                    if *count == 0 {
                        continue;
                    }
                    let scale = 1.0 / *count as f64;
                    let mean = sum.iter().map(|value| value * scale).collect::<Vec<_>>();
                    buffer
                        .set_layered(name, record, &mean)
                        .with_context(|| format!("cannot write {name} into the history buffers"))?;
                }
            }
        }
        Ok(())
    }
}

impl HistorySink for HistoryAccumulator {
    fn scalar(&mut self, name: &str, _record: usize, value: f64) -> Result<()> {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        // `spval` 步不计入：既不进和，也不进计数（上游 `acc1d` 的 `IF (var(i) /= spval)`）。
        // 一步都不有效的变量因此**不会**在 `sums` 里建条目，也就不会被写出，
        // 缓冲区留给它的是填充值 —— 与上游一致。
        if value == colm_core::MISSING {
            return Ok(());
        }
        let instantaneous = INSTANTANEOUS_VARIABLES.contains(&name);
        match self
            .sums
            .entry(name.to_owned())
            .or_insert(Accumulated::Scalar { sum: 0.0, count: 0 })
        {
            Accumulated::Scalar { sum, count } => {
                if instantaneous {
                    // "最后一次覆盖"：`count` 保持 1，于是除数为 1、写出的是末步的值。
                    *sum = value;
                    *count = 1;
                } else {
                    *sum += value;
                    *count += 1;
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
        // 分层量按**整列**是否有效来算（上游的计数器是每 patch 一个，
        // `nac_ln(i)`，不是每层一个）。整列全无效就整列跳过。
        if values.iter().all(|value| *value == colm_core::MISSING) {
            return Ok(());
        }
        let entry = self
            .sums
            .entry(name.to_owned())
            .or_insert_with(|| Accumulated::Column {
                sum: vec![0.0; values.len()],
                count: 0,
            });
        match entry {
            Accumulated::Column { sum, count } => {
                ensure!(
                    sum.len() == values.len(),
                    "{name} changed width between steps"
                );
                for (sum, value) in sum.iter_mut().zip(values) {
                    *sum += value;
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
    /// `forc_solarin`：`f_xy_solarin` 照抄它，不经过任何换算。
    ///
    /// `RuntimeForcing` 只带四个波段，总量由它们相加 —— `MOD_Forcing` 反着来
    /// （先有总量再拆波段），所以这里要看相加能不能逐位回到上游的总量。
    pub downward_shortwave_w_m2: f64,
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
}

impl HistoryReferenceState {
    /// 取本步的 `forc_*`。
    pub fn from_forcing(forcing: &colm_core::RuntimeForcing, surface_cosine_zenith: f64) -> Self {
        Self {
            wind_speed_eastward_m_s: forcing.eastward_wind_m_s,
            wind_speed_northward_m_s: forcing.northward_wind_m_s,
            air_temperature_k: forcing.air_temperature_k,
            specific_humidity_kg_kg: forcing.specific_humidity,
            surface_pressure_pa: forcing.surface_pressure_pa,
            boundary_layer_height_m: forcing.boundary_layer_height_m,
            downward_shortwave_w_m2: forcing.shortwave.direct_visible_w_m2
                + forcing.shortwave.direct_near_infrared_w_m2
                + forcing.shortwave.diffuse_visible_w_m2
                + forcing.shortwave.diffuse_near_infrared_w_m2,
            downward_longwave_w_m2: forcing.downward_longwave_w_m2,
            convective_precipitation_kg_m2_s: forcing.convective_precipitation_kg_m2_s,
            large_scale_precipitation_kg_m2_s: forcing.large_scale_precipitation_kg_m2_s,
            surface_cosine_zenith,
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
    // `fsena`/`fevpa` 的重算输入是**总量**（`fsenl + fseng`、`fevpl + fevpg`），
    // 其中地面那一半必须是订正后的值 —— 用叶温求解前的初步值会让 `f_ustar`
    // 这类量跟着偏（见 `CORRECTED_GROUND` 那条注释）。
    let leaf = &energy.leaf;
    let recomputed = colm_core::history_diagnostics(colm_core::HistoryDiagnosticsInput {
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
    })
    .context("cannot recompute the history near-surface diagnostics")?;
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
/// rnet   = fsena + lfevpa + fgrnd        ! 与 sabv+sabg+lw_net 恒等
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
    vaporization_heat_j_kg: f64,
    soil_layers: usize,
) -> Result<()> {
    const STEFAN_BOLTZMANN_W_M2_K4: f64 = 5.67e-8;
    const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
    const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;

    let energy = &output.energy;
    let ground = &energy.ground;
    // 打包列里第一个**土层**的下标：列长减去土层数。**不能**写 0 —— 带雪时
    // `temperature_k[0]` 是雪面温度，而 `t_grnd_bef`/`tinc` 要的是地表那一层。
    let soil_surface = ground
        .temperature_k
        .len()
        .checked_sub(soil_layers)
        .filter(|index| *index < ground.temperature_k.len())
        .context("the packed ground temperature column is shorter than the soil")?;
    let surface_temperature_k = ground.temperature_k[soil_surface];
    let previous_surface_temperature_k =
        ground
            .previous_temperature_k
            .get(soil_surface)
            .copied()
            .context("the ground temperature state carries no previous surface layer")?;
    let temperature_change_k = surface_temperature_k - previous_surface_temperature_k;

    let emissivity = colm_core::ground_emissivity(ground.snow_water_equivalent_kg_m2, 0);
    let upward_longwave = energy.leaf.upward_longwave_w_m2;
    let blackbody_change = STEFAN_BOLTZMANN_W_M2_K4
        * previous_surface_temperature_k.powi(3)
        * (4.0 * temperature_change_k);
    let outgoing_longwave = upward_longwave + emissivity * blackbody_change;
    let bulk_emissivity =
        (upward_longwave + emissivity * blackbody_change) / (upward_longwave + blackbody_change);
    let radiative_temperature_k = (outgoing_longwave / STEFAN_BOLTZMANN_W_M2_K4).powf(0.25);

    // 上游的 `htvp`（`MOD_Thermal.F90:539-540`）由内核按**表层是否纯冰**定好，
    // 随步输出带出来；这里照抄，不再自己判一次。写成无条件的 `hvap + hfus`
    // 会把所有液态地表的地面蒸发按升华计价 —— 实测冬季窗口 `f_lfevpa` 差 34 W/m²。
    let sublimation_heat = energy.leaf.ground_latent_heat_j_kg;
    let leaf_evaporation = energy.leaf.leaf_evaporation_kg_m2_s;
    // **必须取订正后的地面蒸发**，与 `set_lct_energy_fluxes` 写进 `f_fevpg` 的那一列同源。
    // `leaf.ground_evaporation_kg_m2_s` 是叶温求解**之前**的初步值，两者在 CN-Cng
    // 首条记录上差 2.5 倍（9.3e-5 对 2.3e-4）。用初步值会让
    // `lfevpa = hvap*fevpl + htvp*fevpg` 与同一份文件里的 `f_fevpl`/`f_fevpg`
    // 自相矛盾 —— 实测 Rust 的 `f_lfevpa` 峰值 615 W/m² 而 `hvap*(f_fevpl+f_fevpg)`
    // 只有 187 W/m²；改用订正后立刻落到 196 W/m²（Fortran 184.65）。
    let ground_evaporation = energy.corrected_ground_evaporation_kg_m2_s;
    let latent_heat =
        vaporization_heat_j_kg * leaf_evaporation + sublimation_heat * ground_evaporation;

    let precipitation_temperature_k = energy.precipitation.precipitation_temperature_k;
    let ground_heat = energy.shortwave.ground_absorbed_w_m2
        + energy.leaf.downward_longwave_w_m2 * emissivity
        - emissivity * STEFAN_BOLTZMANN_W_M2_K4 * previous_surface_temperature_k.powi(4)
        - emissivity * blackbody_change
        - (energy.corrected_ground_sensible_heat_w_m2 + ground_evaporation * sublimation_heat)
        + WATER_HEAT_CAPACITY_J_KG_K
            * energy.interception.ground_rain_kg_m2_s
            * (precipitation_temperature_k - surface_temperature_k)
        + ICE_HEAT_CAPACITY_J_KG_K
            * energy.interception.ground_snow_kg_m2_s
            * (precipitation_temperature_k - surface_temperature_k);
    // 地表能量收支恒等式：`rnet = H + LE + G`。用它而不是再拼一遍辐射项，
    // 是因为前者的每一项都已经由内核算过，重复拼装只会引入第二套公式。
    let net_radiation = energy.total_sensible_heat_w_m2 + latent_heat + ground_heat;

    for (name, value) in [
        ("sabvsun", energy.shortwave.sunlit_absorbed_w_m2),
        ("sabvsha", energy.shortwave.shaded_absorbed_w_m2),
        ("rnet", net_radiation),
        ("olrg", outgoing_longwave),
        ("emis", bulk_emissivity),
        ("trad", radiative_temperature_k),
        ("fgrnd", ground_heat),
        ("lfevpa", latent_heat),
    ] {
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
    let stress = energy.root_uptake.soil_water_stress;
    for (name, value) in [
        ("assim", leaf.assimilation_mol_m2_s),
        ("assimsun", leaf.sunlit_assimilation_mol_m2_s),
        ("assimsha", leaf.shaded_assimilation_mol_m2_s),
        ("respc", leaf.respiration_mol_m2_s),
        ("etrsun", leaf.sunlit_transpiration_kg_m2_s),
        ("etrsha", leaf.shaded_transpiration_kg_m2_s),
        ("gssun", leaf.sunlit_stomatal_conductance_mol_m2_s),
        ("gssha", leaf.shaded_stomatal_conductance_mol_m2_s),
        // LCT 分支两行同源（`MOD_Thermal.F90:674-675`），所以这里必然相等。
        ("rstfacsun", stress),
        ("rstfacsha", stress),
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
/// `wat` 的算式见 [`LCT_WATER_STORAGE_VARIABLES`]（非 VSF 分支：
/// `sum(wliq+wice) + ldew + scv + wa`）。液体与冰分开求和再相加，
/// 与上游 `sum(wice_soisno(1:)+wliq_soisno(1:))` 的元素级相加**不是**
/// 同一个结合顺序，所以这里也按元素级累加，别改成两个 `sum()` 相减。
pub fn set_lct_water_storage(
    sink: &mut impl HistorySink,
    record: usize,
    water: &colm_core::Water2014SoilState,
    canopy_water_mm: f64,
    snow_water_equivalent_kg_m2: f64,
) -> Result<()> {
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
    let total = soil + canopy_water_mm + snow_water_equivalent_kg_m2 + water.aquifer_water_mm;
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
) -> Result<()> {
    for (name, value) in [
        ("sigf", state.canopy.vegetation_free_fraction),
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
    let shortwave = &energy.shortwave;
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
) -> Result<()> {
    for (name, value) in [
        ("xy_t", reference.air_temperature_k),
        ("xy_q", reference.specific_humidity_kg_kg),
        ("xy_pbot", reference.surface_pressure_pa),
        ("xy_us", reference.wind_speed_eastward_m_s),
        // 标量风算例里 `forc_vs` 恒为 0（`MOD_Forcing` 把标量风放在 `forc_us`）。
        // `HistoryReferenceState` 已经把标量情形折成 0，这里不再判一次。
        ("xy_vs", reference.wind_speed_northward_m_s),
        ("xy_solarin", reference.downward_shortwave_w_m2),
        ("xy_frl", reference.downward_longwave_w_m2),
        ("xy_prc", reference.convective_precipitation_kg_m2_s),
        ("xy_prl", reference.large_scale_precipitation_kg_m2_s),
    ] {
        ensure!(
            value.is_finite(),
            "the history value for {name} is not finite"
        );
        sink.scalar(name, record, value)?;
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
) -> Result<()> {
    for (name, value) in [
        ("qinfl", water.infiltration_mm_s),
        ("rnof", water.total_runoff_mm_s),
        ("rsub", water.subsurface_runoff_mm_s),
        ("rsur", water.surface_runoff_mm_s),
        ("qcharge", water.recharge_mm_s),
        // **`frcsat` 刻意不填。** 上游只有 `WATER_VSF` 走 `Runoff_*` 并传 `frcsat`
        // （`MOD_SoilSnowHydrology.F90:880-925`，在 `WATER_VSF` 里），
        // `WATER_2014`（本仓库唯一的编排）从不设它 —— 实测对齐算例 264 条记录**全是**
        // `spval`，而开了 VSF 的黄金算例 264 条全有值。本仓库给 `Runoff_*` 传了
        // `frcsat`，于是写出了一个上游没有的量。留空即与 Fortran 逐位相同
        // （`colm-hist` 的填充值与上游的 `spval` 都是 -1e36）。
    ] {
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

/// 积雪分支：把一步的状态写进第 `record` 条记录。
pub fn set_lct_snow_state(
    sink: &mut impl HistorySink,
    record: usize,
    template: &StandardLctRestartTemplate,
    state: &StandardLctSnowSoilState,
    ground_temperature_k: f64,
) -> Result<()> {
    set_columns(
        sink,
        record,
        template,
        &[
            state.snow.temperature_k.clone(),
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
    /// 当前输出区间的累加器（上游的 `a_*` 与 `nac`）。
    accumulator: HistoryAccumulator,
}

impl HistorySession {
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
        ensure!(
            !records.is_empty(),
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
            accumulator: HistoryAccumulator::default(),
        })
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
        self.push(end, |accumulator| {
            set_lct_state(accumulator, 0, template, state, ground)?;
            set_lct_fluxes(accumulator, 0, &output.water)?;
            set_lct_energy_fluxes(accumulator, 0, output)?;
            set_lct_surface_budget(
                accumulator,
                0,
                output,
                template.physics.vaporization_heat_j_kg,
                template.soil_layers(),
            )?;
            set_lct_surface_diagnostics(
                accumulator,
                0,
                &output.energy,
                reference,
                &template.physics,
            )?;
            set_lct_stomatal_diagnostics(accumulator, 0, &output.energy)?;
            set_lct_radiation_bands(accumulator, 0, &output.energy)?;
            set_lct_canopy_geometry(accumulator, 0, &state.energy, &output.energy)?;
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
            )?;
            set_lct_canopy_water(accumulator, 0, &state.energy, &output.energy)?;
            set_lct_albedo(
                accumulator,
                0,
                &state.energy,
                reference.surface_cosine_zenith,
            )?;
            set_lct_forcing_mirrors(accumulator, 0, reference)
        })
    }

    /// 积雪分支：与 [`Self::push_lct`] 同构，走雪入口并把 `soil` 那一半当土壤诊断。
    pub fn push_lct_snow(
        &mut self,
        end: CalendarTime,
        template: &StandardLctRestartTemplate,
        state: &StandardLctSnowSoilState,
        output: &colm_core::StandardLctSnowSoilOutput,
        reference: HistoryReferenceState,
    ) -> Result<Option<PathBuf>> {
        let ground = output.energy.ground.temperature_k[0];
        self.push(end, |accumulator| {
            set_lct_snow_state(accumulator, 0, template, state, ground)?;
            set_lct_fluxes(accumulator, 0, &output.water.soil)?;
            let as_soil = StandardLctSoilOutput {
                energy: output.energy.clone(),
                water: output.water.soil.clone(),
            };
            set_lct_energy_fluxes(accumulator, 0, &as_soil)?;
            set_lct_surface_budget(
                accumulator,
                0,
                &as_soil,
                template.physics.vaporization_heat_j_kg,
                template.soil_layers(),
            )?;
            // 这一句原先漏了：十三个地表诊断量在 `declare_lct_variables` 里声明了，
            // 却从来没有被填过，写出来的 `f_taux`/`f_tauy`/`f_z0m`/`f_zol` … 一直是
            // NetCDF 的填充值。实测 CN-Cng 的积雪分支 history 里这三个是 NaN，
            // 而 Fortran 有值 —— "声明了但没人写"不会报错，只会静默留下一列空洞。
            set_lct_surface_diagnostics(
                accumulator,
                0,
                &output.energy,
                reference,
                &template.physics,
            )?;
            set_lct_stomatal_diagnostics(accumulator, 0, &output.energy)?;
            set_lct_radiation_bands(accumulator, 0, &output.energy)?;
            set_lct_canopy_geometry(accumulator, 0, &state.energy, &output.energy)?;
            set_lct_derived_soil(
                accumulator,
                0,
                template.soil_layer_thickness_m(),
                &state.soil_water,
            )?;
            set_lct_water_storage(
                accumulator,
                0,
                &state.soil_water,
                state.energy.leaf.canopy_water.total_mm,
                state.snow.water_equivalent_kg_m2,
            )?;
            set_lct_canopy_water(accumulator, 0, &state.energy, &output.energy)?;
            set_lct_albedo(
                accumulator,
                0,
                &state.energy,
                reference.surface_cosine_zenith,
            )?;
            set_lct_forcing_mirrors(accumulator, 0, reference)
        })
    }

    /// 收尾：把还开着的那个分组落盘。调度已经保证运行结束那一刻会写一条，所以正常
    /// 情况下这里只是把缓冲区写出。
    pub fn finish(&mut self) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        if let Some((suffix, buffer)) = self.open.take() {
            written.push(self.write(&suffix, &buffer)?);
        }
        Ok(written)
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
        // 1. 每步累加。失败也要把累加器放回去，否则下一次调用从零开始，
        //    会静默丢掉这一段。
        let mut accumulator = std::mem::take(&mut self.accumulator);
        accumulator.steps += 1;
        let filled = accumulate(&mut accumulator);
        self.accumulator = accumulator;
        filled?;

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
        let means = std::mem::take(&mut self.accumulator);
        let mut written = None;
        if self.open.as_ref().map(|(suffix, _)| suffix) != Some(&record.suffix) {
            written = self.finish()?.pop();
            let mut buffer = HistoryBuffers::new(
                self.dimensions,
                self.site,
                self.record_count(&record.suffix),
            );
            // 声明本层能负责的变量；写出的文件因此只包含它们。
            declare_lct_variables(&mut buffer)?;
            self.open = Some((record.suffix.clone(), buffer));
        }
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
        means.write_means(buffer, record.record)?;
        self.cursor += 1;
        Ok(written)
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
