#!/usr/bin/env python3
"""Exercise production material guards without enabling the old preprocessor macros."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "vendor/CoLM202X"


def compile_probe(work, compiler, name, text):
    source = work / f"{name}.f90"
    source.write_text(text, encoding="utf-8")
    executable = work / (name + (".exe" if os.name == "nt" else ""))
    subprocess.run([compiler, "-cpp", "-fcheck=all", "-ffree-line-length-none", str(source), "-o", str(executable)], cwd=work, check=True)
    return executable


def main():
    compiler = shutil.which("gfortran")
    if not compiler:
        raise SystemExit("gfortran is required for material runtime-switch regression")
    defs = (SOURCE / "main/MATERIAL/MOD_MT_Defs.F90").read_text(encoding="utf-8")
    begin = defs.index("      real(r8) FUNCTION mass_to_delta")
    end = defs.index("END FUNCTION mass_to_delta", begin) + len("END FUNCTION mass_to_delta")
    delta = defs[begin:end]
    begin = defs.index("      ngas = 0")
    end = defs.index("      nmts = ", begin)
    methane = defs[begin:end]
    ini = (SOURCE / "mkinidata/CoLMINI.F90").read_text(encoding="utf-8")
    begin = ini.index("      IF (DEF_UnitCatchment_regional) THEN")
    end = ini.index("      CALL mesh_load_from_file", begin)
    regional = ini[begin:end]
    with tempfile.TemporaryDirectory(prefix="colm-material-runtime-") as directory:
        work = Path(directory)
        debug = compile_probe(work, compiler, "debug", """
module MOD_Vars_Global
  implicit none
  real(8), parameter :: spval = -1.e36_8
end module
module probe
  use MOD_Vars_Global
  implicit none
  integer, parameter :: r8=kind(1.d0)
  real(r8), parameter :: mt_tiny=1.e-30_r8
  logical :: DEF_USE_CoLMDEBUG=.false.
contains
""" + delta + """
end module
program test
  use probe
  implicit none
  real(r8) :: quiet, verbose
  quiet=mass_to_delta(-1._r8,1._r8,0.002_r8)
  DEF_USE_CoLMDEBUG=.true.
  verbose=mass_to_delta(-1._r8,1._r8,0.002_r8)
  if (quiet /= spval .or. verbose /= spval) error stop 2
  print *, 'debug toggled without changing invalid-ratio output'
end program
""")
        result = subprocess.run([str(debug)], check=True, capture_output=True, text=True)
        assert result.stdout.count("WARNING mass_to_delta") == 1, result.stdout
        bgc = compile_probe(work, compiler, "bgc", """
program test
  implicit none
  logical :: DEF_USE_METHANE, DEF_USE_BGC
  integer :: ngas
  character(1) :: mode
  call get_command_argument(1,mode)
  DEF_USE_METHANE=mode/='0'
  DEF_USE_BGC=mode=='1'
""" + methane + """
  print *, ngas
contains
  subroutine CoLM_stop(message)
    character(*), intent(in) :: message
    print *, message
    error stop 1
  end subroutine
end program
""")
        for mode, expected in [("0", 0), ("1", 1)]:
            result = subprocess.run([str(bgc), mode], check=True, capture_output=True, text=True)
            assert int(result.stdout.strip()) == expected, result.stdout
        result = subprocess.run([str(bgc), "2"], capture_output=True, text=True)
        assert result.returncode != 0 and "BGC" in result.stdout, result
        lulcc = compile_probe(work, compiler, "lulcc", """
program test
  implicit none
  type time_type
    integer :: end_year=2005
  end type
  type(time_type) :: DEF_simulation_time
  logical :: DEF_UnitCatchment_regional=.true., DEF_USE_LULCC
  integer :: lc_year=2000
  character(1) :: dir_landdata='.'
  DEF_USE_LULCC=.false.
""" + regional + """
  DEF_USE_LULCC=.true.
""" + regional + """
contains
  subroutine unitcatchment_regional_build(directory,first_year,last_year)
    character(*), intent(in) :: directory
    integer, intent(in) :: first_year,last_year
    print *, first_year,last_year
  end subroutine
end program
""")
        result = subprocess.run([str(lulcc)], check=True, capture_output=True, text=True)
        assert [int(value) for value in result.stdout.split()] == [2000, 2000, 2000, 2005], result.stdout
    print("material runtime switches: debug, BGC rejection, LULCC year range PASS")


if __name__ == "__main__":
    main()
