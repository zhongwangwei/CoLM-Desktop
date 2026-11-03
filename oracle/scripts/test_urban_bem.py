#!/usr/bin/env python3
"""Executable SimpleBEM and LUCY references, with no model datasets."""
from pathlib import Path
import ast
import math
import platform
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
VENDOR = ROOT / "vendor/CoLM202X"


def procedure(text, kind, name):
    match = re.search(rf"(?im)^\s*(?:logical\s+)?{kind}\s+{name}\b[\s\S]*?^\s*END\s+{kind}\s+{name}\b", text)
    assert match, name
    return match.group(0) + "\n"


def main():
    compiler = shutil.which("gfortran")
    assert compiler, "gfortran is required for the independent BEM reference"
    time = (VENDOR / "share/MOD_TimeManager.F90").read_text(encoding="utf-8")
    shortwave = (VENDOR / "main/URBAN/MOD_Urban_Shortwave.F90").read_text(encoding="utf-8")
    bem = (VENDOR / "main/URBAN/MOD_Urban_BEM.F90").read_text(encoding="utf-8")
    lucy = (VENDOR / "main/URBAN/MOD_Urban_LUCY.F90").read_text(encoding="utf-8")
    stubs = """
module MOD_Precision
 integer, parameter :: r8 = kind(1.d0)
end module
module MOD_Vars_Global
end module
module MOD_Const_Physical
 use MOD_Precision
 real(r8), parameter :: PI = acos(-1._r8), cpair = 1004.64_r8
end module
module MOD_Namelist
 type :: SimulationTime
  logical :: greenwich = .false.
 end type
 type(SimulationTime) :: DEF_simulation_time
end module
module MOD_Urban_Const_LCZ
end module
module MOD_TimeManager
 use MOD_Precision
 integer, parameter :: daysofmonth_leap(0:12) = (/0,31,29,31,30,31,30,31,31,30,31,30,31/)
 integer, parameter :: daysofmonth_noleap(0:12) = (/0,31,28,31,30,31,30,31,31,30,31,30,31/)
 integer, parameter :: accdaysofmonth_leap(0:12) = (/0,31,60,91,121,152,182,213,244,274,305,335,366/)
 integer, parameter :: accdaysofmonth_noleap(0:12) = (/0,31,59,90,120,151,181,212,243,273,304,334,365/)
 contains
""" + procedure(time, "FUNCTION", "isleapyear") + procedure(time, "SUBROUTINE", "gmt2local") + procedure(time, "SUBROUTINE", "julian2monthday") + procedure(time, "SUBROUTINE", "timeweek") + """
end module
module MOD_Urban_Shortwave
 use MOD_Precision
 contains
 subroutine CoLM_stop(message)
  character(len=*), intent(in) :: message
  print *, message
  error stop 1
 end subroutine
""" + procedure(shortwave, "FUNCTION", "MatrixInverse") + "\nend module\n"
    driver = """
program reference
 use MOD_Precision
 use MOD_Namelist
 use MOD_Urban_BEM
 use MOD_Urban_LUCY
 implicit none
 real(r8) :: weekend(24), ceiling, floor, taf, room, roof, sun, sha, ac, waste, air, heating
 integer :: i, j, idate(3)
 real(r8) :: human(24), weekday(24), lon, anthropogenic, vehicle, metabolism
 idate = (/2024, 1, 43200/)
 DEF_simulation_time%greenwich = .false.
 do i = 1, 8
  ceiling = 290._r8; floor = 288._r8; taf = 295._r8
  weekend = 1._r8
  if (i == 1) ceiling = 298._r8
  if (i == 3 .or. i == 6 .or. i == 7 .or. i == 8) weekend(12) = 0.5_r8
  if (i == 4) weekend = 0._r8
  if (i == 5 .or. i == 6 .or. i == 8) then
   ceiling = 298._r8; floor = 295._r8
  endif
  if (i == 7) taf = 280._r8
  if (i == 8) taf = 310._r8
  room = 294._r8; roof = 293._r8; sun = 293._r8; sha = 293._r8
  call SimpleBEM(idate, 1800._r8, 0._r8, 1.2_r8, (/0.4_r8,0.35_r8,0.25_r8/),10._r8, &
    ceiling,floor,weekend,291._r8,290._r8,289._r8,292._r8,291._r8,290._r8, &
    12._r8,15._r8,14._r8,taf,room,roof,sun,sha,ac,waste,air,heating)
  write(*,'(I2,8(1X,ES25.17))') i, room,roof,sun,sha,ac,waste,air,heating
 enddo
 do j = 1, 24
  human(j) = real(j, r8)
  weekday(j) = real(j, r8)/100._r8
 enddo
 do i = 1, 6
  idate = (/2025,1,0/)
  lon = 0._r8
  DEF_simulation_time%greenwich = .false.
  if (i == 2) then
   DEF_simulation_time%greenwich = .true.; lon = 113.5_r8*acos(-1._r8)/180._r8
  endif
  if (i == 3 .or. i == 4) idate(3) = 3600
  if (i == 4) then
   DEF_simulation_time%greenwich = .true.; lon = -113.5_r8*acos(-1._r8)/180._r8
  endif
  if (i == 5) idate(3) = 86400
  if (i == 6) then
   idate(3) = 43200
   DEF_simulation_time%greenwich = .true.; lon = acos(-1._r8)
  endif
  call LUCY(idate,1800._r8,lon,spread(0._r8,1,365),spread(0._r8,1,7),human,weekday,weekday,1000._r8, &
    (/1._r8,2._r8,3._r8/),anthropogenic,vehicle,metabolism)
  write(*,'(A,I2,3(1X,ES25.17))') 'L',i,anthropogenic,vehicle,metabolism
 enddo
end program
"""
    with tempfile.TemporaryDirectory(prefix="colm-bem-reference-") as tmp:
        tmp = Path(tmp)
        source = tmp / "reference.F90"
        source.write_text(stubs + "\n" + bem + "\n" + lucy + "\n" + driver, encoding="utf-8")
        executable = tmp / "reference"
        link = ["-framework", "Accelerate"] if platform.system() == "Darwin" else ["-llapack", "-lblas"]
        subprocess.run([compiler, "-O2", "-cpp", "-fdefault-real-8", "-fdefault-double-8", "-ffree-line-length-none", "-ffpe-trap=invalid,zero,overflow", "-fcheck=all", "-I", str(VENDOR / "include"), str(source), "-o", str(executable), *link], cwd=tmp, check=True)
        output = subprocess.check_output([str(executable)], text=True)
    rows = [list(map(float, line.split()[1:])) for line in output.splitlines() if not line.startswith("L")]
    lucy_rows = [list(map(float, line.split()[2:])) for line in output.splitlines() if line.startswith("L")]
    assert len(rows) == 8 and all(math.isfinite(value) for row in rows for value in row)
    rust_test = (ROOT / "crates/colm-core/src/urban_bem_tests.rs").read_text(encoding="utf-8")
    match = re.search(r"let expected: \[\[f64; 8\]; 8\] = (\[[\s\S]*?\n    \]);", rust_test)
    assert match, "Rust reference table is missing"
    expected = ast.literal_eval(match.group(1))
    for case, (actual, reference) in enumerate(zip(rows, expected), 1):
        for component, (value, pinned) in enumerate(zip(actual, reference)):
            assert abs(value - pinned) < 5.0e-12, (case, component, value, pinned)

    assert rows[1] == rows[3], "disabled LUCY retains constant AC without 0/0"
    # Area weighting and waste scaling occur in different multiplication order.
    assert abs(rows[1][5] - rows[1][4] * 0.3) <= 2 * math.ulp(rows[1][5])
    assert rows[4][7] > 0 and rows[4][4] == 0
    lucy_test = (ROOT / "crates/colm-core/src/urban_lucy_tests.rs").read_text(encoding="utf-8")
    match = re.search(r"let expected: \[\[f64; 3\]; 6\] = (\[[\s\S]*?\n    \]);", lucy_test)
    assert match, "Rust LUCY reference table is missing"
    expected = ast.literal_eval(match.group(1))
    assert len(lucy_rows) == len(expected) == 6
    for case, (actual, reference) in enumerate(zip(lucy_rows, expected), 1):
        for component, (value, pinned) in enumerate(zip(actual, reference)):
            assert abs(value - pinned) < 1.0e-14, (case, component, value, pinned)
    print(output, end="")
    print("Urban BEM/LUCY executable references: PASS")


if __name__ == "__main__":
    main()
