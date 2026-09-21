#!/usr/bin/env python3
"""Static regression for the CoLM202X f48fbf9 production sync."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2] / "vendor" / "CoLM202X"


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def main() -> None:
    reservoir = read("main/HYDRO/MOD_Grid_Reservoir.F90")
    bif = read("main/HYDRO/MOD_Grid_RiverLakeBifurcation.F90")
    flow = read("main/HYDRO/MOD_Grid_RiverLakeFlow.F90")
    hist = read("main/HYDRO/MOD_Grid_RiverLakeHist.F90")
    leaf = read("main/MOD_LeafTemperature.F90")
    leaf_pc = read("main/MOD_LeafTemperaturePC.F90")
    # `extend_interception` 打开时（default 预设就是）真正参与链接的是 extends/ 下的
    # 这四份同名替代模块，`main/` 的两份**根本不编译**。第一次同步漏了它们，
    # 于是 `rstfacsun` 在扩展版里仍是 `intent(out)`：`stomata` 读到的是
    # `rstfacsun_out` 的 spval(-1e36)，`vm`/`epar`/`respc` 全被它缩放成垃圾，
    # 冠层光合恒为 0、气孔阻力恒为 0（= 无限制潜在蒸腾）。实测证据见
    # docs/implementation-verification.md「找到蒸腾链的真凶：扩展截获模块漏了
    # intent(inout)」一节。
    leaf_ext = read("extends/interception/MOD_LeafTemperature_Extended.F90")
    leaf_pc_ext = read("extends/interception/MOD_LeafTemperaturePC_Extended.F90")
    ozone = read("main/MOD_Ozone.F90")
    namelist = read("share/MOD_Namelist.F90")

    assert "catalogue_to_active(i) = totalnumresv" in reservoir
    assert "IF (numresv > 0) icache(1:numresv) = resv_global_id" in reservoir
    assert "PURE FUNCTION bif_limiter_fraction" in bif
    assert "ELSEIF (transfer > available) THEN" in bif
    assert "Reapply the path cap to the final net flux" in bif
    assert "restart_levee_enabled_in .neqv. DEF_USE_LEVEE" in bif
    assert "rivsto_hist = min(volwater, floodplain_curve(i)%rivstomax)" in flow
    assert "(volwater - rivsto_hist)" in flow
    assert "below-bank river channel storage" in hist
    assert "visible overbank storage excluding levee-protected storage" in hist
    assert "real(r8), intent(inout) :: &\n        rstfacsun" in leaf
    assert "real(r8), intent(inout) :: &\n        rstfacsun" in leaf_ext
    assert "gssun = (laisun / rssun) * (tprcor / tlbef)" in leaf
    assert "gssun = (laisun / rssun) * (tprcor / tlbef)" in leaf_ext
    assert "gssun(i) = (laisun(i) / rssun(i)) * (tprcor / tlbef(i))" in leaf_pc
    assert "gssun(i) = (laisun(i) / rssun(i)) * (tprcor / tlbef(i))" in leaf_pc_ext
    # 本仓库的本地修复（见 vendor/PROVENANCE.md）：`o3coef*` 必须在迭代**之前**置 1。
    # 上游只在迭代之后的非臭氧分支里赋值，而循环体已经把 `o3coefg_*` 交给 `stomata`；
    # `intent(inout)` 的哑元第一次调用读到的是调用方 SAVE 变量的未定义值。实测
    # CN-Cng 第一步因此 `rssun` 大约 10 倍、冠层蒸腾 `etr` 约 4800 倍。
    o3_init = (
        "o3coefv_sun = 1.0_r8\n"
        "      o3coefv_sha = 1.0_r8\n"
        "      o3coefg_sun = 1.0_r8\n"
        "      o3coefg_sha = 1.0_r8"
    )
    for text in (leaf, leaf_ext):
        assert text.index(o3_init) < text.index("DO WHILE (it .le. itmax)")
        assert text.count("o3coefv_sun = 1.0_r8") == 1
    assert "CALL mg2p_ozone%grid2pset (f_ozone, forc_ozone)" in ozone
    assert "logical :: DEF_USE_OZONESTRESS = .false." in namelist
    assert "logical :: DEF_USE_OZONEDATA   = .false." in namelist
    print("CoLM202X f48 production sync: PASS")


if __name__ == "__main__":
    main()
