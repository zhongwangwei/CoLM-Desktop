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
// 嵌套 IF 按上游结构保留，便于逐行对照。
#![allow(clippy::collapsible_if, clippy::collapsible_else_if)]
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
        ("MOD_BGC_CNSummary", "soilbiogeochem_carbonstate_summary", "_soilbiogeochem_carbonstate_summary",
         "土壤 C 池汇总。"),
        ("MOD_BGC_CNSummary", "soilbiogeochem_nitrogenstate_summary", "_soilbiogeochem_nitrogenstate_summary",
         "土壤 N 池汇总。"),
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
    ("bgc_veg_struct.rs", "`MOD_BGC_Veg_CNVegStructUpdate.F90`：由 C 池更新 LAI/SAI。", "", [
        ("MOD_BGC_Veg_CNVegStructUpdate", "CNVegStructUpdate", "cn_veg_struct_update",
         "`CNVegStructUpdate`：更新 `tsai_p`（每步）与 LAI 反馈下的 `tlai_p`/`lai_p`，再汇总 patch LAI。"),
    ]),
]


def main():
    gimple = Path(os.environ["GIMPLE"])
    only = set(sys.argv[1:])
    with tempfile.TemporaryDirectory() as tmp:
        for out, title, extra_use, subs in MODULES:
            if only and out not in only:
                continue
            header = Path(tmp) / "header.txt"
            header.write_text(
                f"//! {title}\n{GENERATED}\n{extra_use + chr(10) if extra_use else ''}"
                "use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches, NPCROPMIN};\n"
                "use crate::bgc_state::BgcState;\nuse crate::MISSING;\n")
            args = []
            names = ",".join(f"{sub}={rust}" for _, sub, rust, _ in subs)
            for module, sub, rust, doc in subs:
                draft = Path(tmp) / f"{rust}.rs"
                with open(draft, "w") as fh:
                    subprocess.run([sys.executable, HERE / "f2rs.py", BGC / f"{module}.F90", sub,
                                    "--gimple", gimple / f"{module}.F90.273t.optimized", "--names", names],
                                   check=True, stdout=fh)
                args += [str(draft), rust, doc]
            subprocess.run([sys.executable, HERE / "mkmod.py", SRC / out, header, *args], check=True)
            print("wrote", out)


if __name__ == "__main__":
    main()
