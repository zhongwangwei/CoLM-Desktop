#!/usr/bin/env python3
"""Regression for future-reservoir cold-start restart files."""
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
TIMEVARS = ROOT / "vendor/CoLM202X/main/HYDRO/MOD_Grid_RiverLakeTimeVars.F90"
INIT = ROOT / "vendor/CoLM202X/mkinidata/MOD_Initialize.F90"
FLOW = ROOT / "vendor/CoLM202X/main/HYDRO/MOD_Grid_RiverLakeFlow.F90"


def main() -> None:
    timevars = TIMEVARS.read_text(encoding="utf-8")
    init = INIT.read_text(encoding="utf-8")
    flow = FLOW.read_text(encoding="utf-8")

    assert "USE MOD_Vars_Global,           only: spval" in timevars
    assert "volresv(i) < 0._r8 .and. volresv(i) /= spval" in timevars
    assert "volresv = spval" in init
    assert "IF (volresv(irsv) == spval) THEN" in flow
    print("GridRiverLake future-reservoir restart sentinel: PASS")


if __name__ == "__main__":
    main()
