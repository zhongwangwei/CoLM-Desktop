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
    # `extends/interception` 自 CoLM-SYSU-integration 同步起不再参与编译（上游
    # d6de53e9 去掉了 extend_interception 的接线，CoLM2024 截获并入 main/），
    # 所以这里只守 main/ 下真正编进内核的两份。当年扩展版漏掉 `intent(inout)`
    # 的教训见 docs/implementation-verification.md「找到蒸腾链的真凶」一节。
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
    assert "gssun = (laisun / rssun) * (tprcor / tlbef)" in leaf
    assert "gssun(i) = (laisun(i) / rssun(i)) * (tprcor / tlbef(i))" in leaf_pc
    # 本仓库的本地修复（见 docs/upstream-bugs.md）：`o3coef*` 必须在迭代**之前**置 1。
    # 上游只在迭代之后的非臭氧分支里赋值，而循环体已经把 `o3coefg_*` 交给 `stomata`；
    # `intent(inout)` 的哑元第一次调用读到的是调用方 SAVE 变量的未定义值。实测
    # CN-Cng 第一步因此 `rssun` 大约 10 倍、冠层蒸腾 `etr` 约 4800 倍。
    o3_init = (
        "o3coefv_sun = 1.0_r8\n"
        "      o3coefv_sha = 1.0_r8\n"
        "      o3coefg_sun = 1.0_r8\n"
        "      o3coefg_sha = 1.0_r8"
    )
    for text in (leaf,):
        assert text.index(o3_init) < text.index("DO WHILE (it .le. itmax)")
        assert text.count("o3coefv_sun = 1.0_r8") == 1
    assert "CALL mg2p_ozone%grid2pset (f_ozone, forc_ozone)" in ozone
    assert "logical :: DEF_USE_OZONESTRESS = .false." in namelist
    assert "logical :: DEF_USE_OZONEDATA   = .false." in namelist
    # 不许再接回扩展截获：它与 main/ 是两份有意不同的实现，Rust 只对齐 main/。
    makefile = read("Makefile")
    assert "extend_interception" not in makefile and "_Extended.F90" not in makefile
    assert "#define extend_interception" not in read(".github/workflows/create_defineh.bash")
    # 2026-09 同步 CoLM-SYSU-integration@3c799bae 时保住的本地修复（见 docs/upstream-bugs.md）。
    thermal = read("main/MOD_Thermal.F90")
    assert thermal.count("t_soisno(1:),wliq_soisno(1:)") == 3   # eroot x2 + SoilSurfaceResistance
    assert "dz_gpersno(1:),t_gpersno(1:),wliq_gpersno(1:)" in read("main/URBAN/MOD_Urban_Thermal.F90")
    assert "\n      pn = ps - 1\n" in thermal                     # 无条件，不在 TRACER 分支里
    defineh = read(".github/workflows/create_defineh.bash")
    assert "#define  CatchLateralFlow" in defineh and "LATERAL_FLOW" not in defineh.replace("CatchLateralFlow", "")
    # 2026-09-29 同步 CoLM-SYSU-integration@85cf2328：示踪物含水层交换只计透水层；
    # 单点 mksrfdata 的正常结束保持退出码 0（上游改 `STOP 1` 后的本地修复）。
    soil_water = read("main/HYDRO/MOD_Hydro_SoilWater.F90")
    assert soil_water.count("IF (is_permeable(ilev) .and. exchange_zwt_before < sp_zi(ilev))") == 2
    assert soil_water.count("IF (is_permeable(ilev) .and. zwt < sp_zi(ilev))") == 3
    assert "      STOP 1\n" in read("share/MOD_SPMD_Task.F90")
    mksrfdata = read("mksrfdata/MKSRFDATA.F90")
    assert "'Successful in surface data making.'\n" in mksrfdata
    assert "CALL CoLM_stop()\n#endif" not in mksrfdata
    # 8 个维度是运行时开关：上游新代码里的这些宏必须在同步时转换掉，不能漏进源码。
    import re
    converted = r"\b(TRACER|BGC|LULC_IGBP_PFT|LULC_IGBP_PC|LULCC|CoLMDEBUG|RangeCheck|SrfdataDiag|Campbell_SOIL_MODEL|vanGenuchten_Mualem_SOIL_MODEL)\b"
    for sub in ("main", "share", "mksrfdata", "mkinidata", "include"):
        for path in (ROOT / sub).rglob("*"):
            if path.suffix not in (".F90", ".inc") or path.name == "define.h":
                continue
            for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
                if re.match(r"\s*#\s*(if|ifdef|ifndef|elif)\b", line) and re.search(converted, line):
                    raise AssertionError(f"compile-time switch left in {path.relative_to(ROOT)}: {line.strip()}")
    print("CoLM202X f48 production sync: PASS")


if __name__ == "__main__":
    main()
