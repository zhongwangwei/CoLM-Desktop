#!/usr/bin/env python3
"""由上游 Fortran 与其 GIMPLE 重新转写 BGC 的规则化模块。

    GIMPLE=<*.273t.optimized 所在目录> python3 oracle/scripts/bgc_port/regen.py

表里的每个 Rust 文件都完全由本脚本生成（文件头注明），改动请改工具或上游后重跑。
"""
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BGC = ROOT / "vendor/CoLM202X/main/BGC"
SRC = ROOT / "crates/colm-core/src"

GENERATED = """//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 与"先声明、分支里赋值"都按上游结构保留，便于逐行对照。
#![allow(clippy::collapsible_if, clippy::collapsible_else_if, clippy::needless_late_init)]
// `a >= lo .and. a <= hi`、`max(lo, min(hi, x))` 照抄：改成 `contains`/`clamp` 会改变 NaN 的行为。
#![allow(clippy::manual_range_contains, clippy::manual_clamp)]
"""

# (输出文件, 模块说明, 额外 use, [(Fortran 模块, 子程序, Rust 函数名, 文档)])
MODULES = [
    ("bgc_c_state_update.rs", "`MOD_BGC_CNCStateUpdate1/2/3.F90`：植被与土壤 C 池按通量推进一步。", "", [
        ("MOD_BGC_CNCStateUpdate1", "CStateUpdate1", "c_state_update1",
         "`CStateUpdate1`：光合、物候转移、分配与维持呼吸引起的 C 池变化。"),
        ("MOD_BGC_CNCStateUpdate2", "CStateUpdate2", "c_state_update2",
         "`CStateUpdate2`：间隙死亡引起的 C 池变化。"),
        ("MOD_BGC_CNCStateUpdate3", "CStateUpdate3", "c_state_update3",
         "`CStateUpdate3`：火烧引起的 C 池变化。"),
    ]),
    ("bgc_n_state_update.rs", "`MOD_BGC_CNNStateUpdate1/2/3.F90`：植被 N 池按通量推进一步。", "", [
        ("MOD_BGC_CNNStateUpdate1", "NStateUpdate1", "n_state_update1",
         "`NStateUpdate1`：物候转移、分配与再转移引起的 N 池变化。"),
        ("MOD_BGC_CNNStateUpdate2", "NStateUpdate2", "n_state_update2",
         "`NStateUpdate2`：间隙死亡引起的 N 池变化。"),
        ("MOD_BGC_CNNStateUpdate3", "NStateUpdate3", "n_state_update3",
         "`NStateUpdate3`：淋溶/反硝化与火烧引起的 N 池变化。"),
    ]),
    ("bgc_soil_n_state_update.rs",
     "`MOD_BGC_Soil_BiogeochemNStateUpdate1.F90`：土壤矿质 N 与分解池 N 的推进。", "", [
         ("MOD_BGC_Soil_BiogeochemNStateUpdate1", "SoilBiogeochemNStateUpdate1",
          "soil_biogeochem_n_state_update1",
          "`SoilBiogeochemNStateUpdate1`：沉降、固氮、矿化/固持与植物吸收引起的矿质 N 变化。"),
     ]),
    ("bgc_gap_mortality.rs", "`MOD_BGC_Veg_CNGapMortality.F90`：间隙死亡（背景死亡率）。", "", [
        ("MOD_BGC_Veg_CNGapMortality", "CNGapMortality", "cn_gap_mortality",
         "`CNGapMortality`：按年死亡率把各植被池转成凋落物通量。"),
        ("MOD_BGC_Veg_CNGapMortality", "CNGap_VegToLitter", "_cn_gap_veg_to_litter",
         "`CNGap_VegToLitter`：把死亡通量按廓线分到土层的凋落物池。"),
    ]),
    ("bgc_annual_update.rs", "`MOD_BGC_CNAnnualUpdate.F90`：年末累计量的滚动。",
     "use crate::bgc_driver::is_end_of_year;", [
         ("MOD_BGC_CNAnnualUpdate", "CNAnnualUpdate", "cn_annual_update",
          "`CNAnnualUpdate`：年末把 `tempsum_*`/`tempmax_*` 转成 `annsum_*`/`annmax_*`。"),
     ]),
    ("bgc_n_leaching.rs", "`MOD_BGC_Soil_BiogeochemNLeaching.F90`：矿质 N 随径流的淋溶。", "", [
        ("MOD_BGC_Soil_BiogeochemNLeaching", "SoilBiogeochemNLeaching", "soil_biogeochem_n_leaching",
         "`SoilBiogeochemNLeaching`：按土壤水量与径流算出 N 淋溶通量。"),
    ]),
    ("bgc_summary.rs", "`MOD_BGC_CNSummary.F90`：C/N 池与通量的逐层积分与 patch 汇总。", "", [
        ("MOD_BGC_CNSummary", "CNDriverSummarizeStates", "cn_driver_summarize_states",
         "`CNDriverSummarizeStates`：状态量汇总。"),
        ("MOD_BGC_CNSummary", "CNDriverSummarizeFluxes", "cn_driver_summarize_fluxes",
         "`CNDriverSummarizeFluxes`：通量汇总。"),
        ("MOD_BGC_CNSummary", "soilbiogeochem_carbonstate_summary", "soilbiogeochem_carbonstate_summary",
         "土壤 C 池汇总（湿地 CH4 的 `CNDriverSummarizeNonvegetatedSoilStates` 也用）。"),
        ("MOD_BGC_CNSummary", "soilbiogeochem_nitrogenstate_summary", "soilbiogeochem_nitrogenstate_summary",
         "土壤 N 池汇总（湿地 CH4 的 `CNDriverSummarizeNonvegetatedSoilStates` 也用）。"),
        ("MOD_BGC_CNSummary", "cnveg_carbonstate_summary", "_cnveg_carbonstate_summary", "植被 C 池汇总。"),
        ("MOD_BGC_CNSummary", "cnveg_nitrogenstate_summary", "_cnveg_nitrogenstate_summary", "植被 N 池汇总。"),
        ("MOD_BGC_CNSummary", "soilbiogeochem_carbonflux_summary", "_soilbiogeochem_carbonflux_summary",
         "土壤 C 通量汇总。"),
        ("MOD_BGC_CNSummary", "soilbiogeochem_nitrogenflux_summary", "_soilbiogeochem_nitrogenflux_summary",
         "土壤 N 通量汇总。"),
        ("MOD_BGC_CNSummary", "cnveg_carbonflux_summary", "_cnveg_carbonflux_summary", "植被 C 通量汇总。"),
        ("MOD_BGC_CNSummary", "cnveg_nitrogenflux_summary", "_cnveg_nitrogenflux_summary", "植被 N 通量汇总。"),
    ]),
    ("bgc_balance.rs", "`MOD_BGC_CNBalanceCheck.F90`：步首记下 C/N 总量，步末检查收支。", "", [
        ("MOD_BGC_CNBalanceCheck", "BeginCNBalance", "begin_cn_balance", "`BeginCNBalance`：记下步首总量。"),
        ("MOD_BGC_CNBalanceCheck", "CBalanceCheck", "c_balance_check",
         "`CBalanceCheck`：C 收支检查（失败时上游 abort，这里返回错误）。"),
        ("MOD_BGC_CNBalanceCheck", "NBalanceCheck", "n_balance_check",
         "`NBalanceCheck`：N 收支检查（失败时上游 abort，这里返回错误）。"),
    ]),
    ("bgc_soil_competition.rs", "`MOD_BGC_Soil_BiogeochemCompetition.F90`：植物与微生物分配土壤矿质 N。", "", [
        ("MOD_BGC_Soil_BiogeochemCompetition", "SoilBiogeochemCompetition", "soil_biogeochem_competition",
         "`SoilBiogeochemCompetition`（NITRIF 开时 NH₄/NO₃ 分开竞争，关时合并为矿质 N）。"),
    ]),
    ("bgc_cn_phenology.rs", "`MOD_BGC_Veg_CNPhenology.F90`：物候（常绿/季节落叶/胁迫落叶/作物）与凋落物。",
     "use crate::calendar::is_leap_year;", [
         ("MOD_BGC_Veg_CNPhenology", "CNPhenology", "cn_phenology",
          "`CNPhenology`：`phase` 1 为气候统计与各类物候判定，2 为转移、凋落与落入土壤。"),
         ("MOD_BGC_Veg_CNPhenology", "CNPhenologyClimate", "_cn_phenology_climate", "气候统计（积温、降水滑动平均）。"),
         ("MOD_BGC_Veg_CNPhenology", "CNEvergreenPhenology", "_cn_evergreen_phenology", "常绿物候。"),
         ("MOD_BGC_Veg_CNPhenology", "CNSeasonDecidPhenology", "_cn_season_decid_phenology", "季节性落叶物候。"),
         ("MOD_BGC_Veg_CNPhenology", "CNStressDecidPhenology", "_cn_stress_decid_phenology", "胁迫落叶物候。"),
         ("MOD_BGC_Veg_CNPhenology", "CropPhenology", "_crop_phenology", "作物物候（`#ifdef CROP`）：播种、成熟与收获。"),
         ("MOD_BGC_Veg_CNPhenology", "CNOnsetGrowth", "_cn_onset_growth", "展叶期转移。"),
         ("MOD_BGC_Veg_CNPhenology", "CNOffsetLitterfall", "_cn_offset_litterfall", "落叶期凋落。"),
         ("MOD_BGC_Veg_CNPhenology", "CNBackgroundLitterfall", "_cn_background_litterfall", "背景凋落。"),
         ("MOD_BGC_Veg_CNPhenology", "CNLivewoodTurnover", "_cn_livewood_turnover", "活木转死木。"),
         ("MOD_BGC_Veg_CNPhenology", "CNLitterToColumn", "_cn_litter_to_column", "凋落物按廓线进入土层。"),
     ], {"vernalization": "crate::bgc_crop::vernalization"}),
    ("bgc_nutrient_competition.rs",
     "`MOD_BGC_Veg_NutrientCompetition.F90`：植物养分需求与竞争后的分配（含 `#ifdef CROP` 的作物分配）。", "", [
         ("MOD_BGC_Veg_NutrientCompetition", "calc_plant_nutrient_demand_CLM45_default",
          "calc_plant_nutrient_demand", "`calc_plant_nutrient_demand_CLM45_default`：分配系数与 N 需求。"),
         ("MOD_BGC_Veg_NutrientCompetition", "calc_plant_nutrient_competition_CLM45_default",
          "calc_plant_nutrient_competition", "`calc_plant_nutrient_competition_CLM45_default`：按 `fpg` 分配新生长。"),
     ]),
    ("bgc_crop_n_dynamics.rs", "`MOD_BGC_Veg_CNNDynamics.F90` 的作物部分：施肥与大豆固氮（`#ifdef CROP`）。", "", [
         ("MOD_BGC_Veg_CNNDynamics", "CNNFert", "cn_n_fert", "`CNNFert`：施肥进入土壤矿质 N。"),
         ("MOD_BGC_Veg_CNNDynamics", "CNSoyfix", "cn_soyfix", "`CNSoyfix`：大豆共生固氮（`DEF_USE_CNSOYFIXN`）。"),
     ]),
    ("bgc_fire.rs", "`MOD_BGC_Veg_CNFireLi2016.F90`/`MOD_BGC_Veg_CNFireBase.F90`：火烧面积与火烧通量（`DEF_USE_FIRE`）。", "", [
        ("MOD_BGC_Veg_CNFireLi2016", "CNFireArea", "cn_fire_area",
         "`CNFireArea`：Li et al. (2012–2017) 的火烧面积（农田、泥炭与其他火）。"),
        ("MOD_BGC_Veg_CNFireBase", "CNFireFluxes", "cn_fire_fluxes",
         "`CNFireFluxes`：按火烧面积算植被与凋落物/粗木质残体的燃烧与致死通量。"),
    ]),
    ("bgc_veg_struct.rs", "`MOD_BGC_Veg_CNVegStructUpdate.F90`：由 C 池更新 LAI/SAI。", "", [
        ("MOD_BGC_Veg_CNVegStructUpdate", "CNVegStructUpdate", "cn_veg_struct_update",
         "`CNVegStructUpdate`：更新 `tsai_p`（每步）与 LAI 反馈下的 `tlai_p`/`lai_p`，再汇总 patch LAI。"),
    ]),
]


# 局部标量在上游每条读取路径上都已赋值，但 Rust 的流分析看不出来（if/else-if 链无末尾 else、
# 或赋值与使用分处两个同条件的块），需要一个零初值才能通过编译。逐个登记、写明理由。
ZERO_INIT = {
    # `IF (sminn > t1) … ELSE IF (t2 < sminn <= t1) … ELSE IF (sminn <= t2)`：只有 NaN 会漏掉。
    "CNSoyfix": ["fxn"],
    # 赋值在 `#ifdef CROP` 的作物块里，使用在同一 `ivt >= npcropmin` 条件下的后一个块里。
    "calc_plant_nutrient_competition_CLM45_default": ["f5"],
    # 上游声明了 `ivt` 却从不赋值；两个内核的反汇编都按常数表第 0 项取（upstream-bugs 第 24 条）。
    # `btran2` 只在 PFT 循环里赋值，没有 PFT 时上游读的是未定义值（GIMPLE `btran2_824(D)`）；
    # 实际运行至少有一个 PFT。
    "CNFireArea": ["ivt", "btran2"],
    # `f` 在 PFT 循环里赋值（与 `m` 无关），循环之后的分解池燃烧还用它。
    "CNFireFluxes": ["ivt", "f"],
}

# 前向代入的局部变量（见 f2rs.py `FORWARD`），每个都由 GIMPLE 核实。
FORWARD = {
    # `watdry = porsl*pow(…)`（115 行）在 GIMPLE 里没有乘法：116/117 行是
    # `.FNMA (porsl, pow, h2osoi)` 与 `.FNMA (porsl, pow, porsl)`。
    "CNSoyfix": ["watdry"],
}


def main():
    gimple = Path(os.environ["GIMPLE"])
    only = set(sys.argv[1:])
    with tempfile.TemporaryDirectory() as tmp:
        for out, title, extra_use, subs, *external in MODULES:
            if only and out not in only:
                continue
            header = Path(tmp) / "header.txt"
            header.write_text(
                f"//! {title}\n{GENERATED}\n{extra_use + chr(10) if extra_use else ''}"
                "use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};\n"
                "use crate::bgc_state::BgcState;\nuse crate::MISSING;\n")
            args = []
            names = ",".join([f"{sub}={rust}" for _, sub, rust, _ in subs]
                             + [f"{sub}={rust}" for sub, rust in (external[0] if external else {}).items()])
            for module, sub, rust, doc in subs:
                draft = Path(tmp) / f"{rust}.rs"
                with open(draft, "w") as fh:
                    extra = ["--zero-init", ",".join(ZERO_INIT[sub])] if sub in ZERO_INIT else []
                    if sub in FORWARD:
                        extra += ["--forward", ",".join(FORWARD[sub])]
                    subprocess.run([sys.executable, HERE / "f2rs.py", BGC / f"{module}.F90", sub,
                                    "--gimple", gimple / f"{module}.F90.273t.optimized", "--names", names,
                                    *extra],
                                   check=True, stdout=fh)
                args += [str(draft), rust, doc]
            subprocess.run([sys.executable, HERE / "mkmod.py", SRC / out, header, *args], check=True)
            print("wrote", out)


if __name__ == "__main__":
    main()
