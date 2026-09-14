#!/usr/bin/env python3
"""Execute CoLMMAIN's allocation block with PFT arrays absent in LCT mode."""
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    compiler = shutil.which("gfortran")
    if compiler is None:
        raise SystemExit("gfortran is required for this regression")
    source = (Path(__file__).resolve().parents[2] / "vendor/CoLM202X/main/CoLMMAIN.F90").read_text()
    end = source.index("         canopy_phase_heat_p(:) = 0._r8")
    start = source.rfind("         IF (patchtype == 0", 0, end)
    allocation = source[start:end] + "         canopy_phase_heat_p(:) = 0._r8\n"
    program = """program test_allocation
    implicit none
    integer, parameter :: r8 = selected_real_kind(12,50)
    integer :: mode, patchtype, ipatch, ps, pe
    integer, allocatable :: patch_pft_s(:), patch_pft_e(:)
    real(r8), allocatable :: canopy_phase_heat_p(:)
    logical :: DEF_USE_LCT, DEF_USE_PFT, DEF_USE_PC
    ipatch = 1
    do mode = 1, 3
        DEF_USE_LCT = mode == 1
        DEF_USE_PFT = mode == 2
        DEF_USE_PC = mode == 3
        if (.not. DEF_USE_LCT) then
            allocate(patch_pft_s(1), patch_pft_e(1))
            patch_pft_s = 4
            patch_pft_e = 6
        endif
        do patchtype = 0, 4
""" + allocation + """
            if (patchtype == 0 .and. .not. DEF_USE_LCT) then
                if (lbound(canopy_phase_heat_p, 1) /= 4) stop 1
                if (ubound(canopy_phase_heat_p, 1) /= 6) stop 2
            else
                if (size(canopy_phase_heat_p) /= 1) stop 3
            endif
            if (any(canopy_phase_heat_p /= 0._r8)) stop 4
            deallocate(canopy_phase_heat_p)
        enddo
        if (allocated(patch_pft_s)) deallocate(patch_pft_s, patch_pft_e)
    enddo
end program
"""
    with tempfile.TemporaryDirectory(prefix="colm-canopy-allocation-") as directory:
        root = Path(directory)
        (root / "check.f90").write_text(program)
        subprocess.run([compiler, "-O0", "-fcheck=all", "-ffree-line-length-none",
                        str(root / "check.f90"), "-o", str(root / "check")], check=True)
        subprocess.run([str(root / "check")], check=True, cwd=root)
    print("LCT/PFT/PC canopy allocation: PASS (15 combinations)")


if __name__ == "__main__":
    main()
