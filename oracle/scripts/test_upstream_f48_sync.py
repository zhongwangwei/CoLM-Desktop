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
    assert "CALL mg2p_ozone%grid2pset (f_ozone, forc_ozone)" in ozone
    assert "logical :: DEF_USE_OZONESTRESS = .false." in namelist
    assert "logical :: DEF_USE_OZONEDATA   = .false." in namelist
    print("CoLM202X f48 production sync: PASS")


if __name__ == "__main__":
    main()
