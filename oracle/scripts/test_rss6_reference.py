#!/usr/bin/env python3
"""Check the RSS 6 fixture against the checked-in Fortran branch, with/without FMA."""
import argparse
import csv
import os
from pathlib import Path
import platform
import shlex
import subprocess
import struct
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "oracle/fixtures/rss6_reference.csv"


def reference_source():
    source = (ROOT / "vendor/CoLM202X/main/MOD_SoilSurfaceResistance.F90").read_text(encoding="utf-8")
    start = source.rindex("      CASE (6)") + len("      CASE (6)")
    end = source.index("      ENDSELECT", start)
    tail = source[end + len("      ENDSELECT"):source.index("   END Subroutine SoilSurfaceResistance", end)]
    return """module reference
implicit none
integer, parameter :: r8=kind(1.d0), DEF_RSS_SCHEME=6
contains
subroutine rss6(wliq_soisno,wice_soisno,porsl,dz_soisno,vf_sand,fsno,rss)
real(r8),intent(in)::wliq_soisno(1),wice_soisno(1),porsl(1),dz_soisno(1),vf_sand(1),fsno
real(r8),intent(out)::rss
real(r8),parameter::denh2o=1000._r8,denice=917._r8
real(r8)::wx,fac,f_transition,f_sigmoid,rss_basic,rss_adjusted
""" + source[start:end] + tail + """
end subroutine
end module
program main
use reference
implicit none
real(r8)::liq,ice,por,dz,sand,snow,rss
integer::ios
 do
 read(*,*,iostat=ios) liq,ice,por,dz,sand,snow
 if(ios/=0) exit
 call rss6([liq],[ice],[por],[dz],[sand],snow,rss)
 write(*,'(ES26.17E3)') rss
 enddo
end program
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="record reference values after an intentional source change")
    args = parser.parse_args()
    rows = list(csv.reader(FIXTURE.read_text(encoding="utf-8").splitlines()))
    data = rows[1:]
    inputs = "\n".join(" ".join(row[:6]) for row in data) + "\n"
    with tempfile.TemporaryDirectory(prefix="colm-rss6-reference-") as directory:
        directory = Path(directory)
        (directory / "reference.f90").write_text(reference_source(), encoding="utf-8")
        for fused, column in [(True, 6), (False, 7)]:
            flags = ["-O2", "-fdefault-real-8", "-fdefault-double-8", "-ffp-contract=fast" if fused else "-ffp-contract=off"]
            if fused and platform.machine().lower() in ("x86_64", "amd64"):
                flags.append("-mfma")
            subprocess.run(shlex.split(os.environ.get("FC", "gfortran")) + flags + ["reference.f90", "-o", "reference"], cwd=directory, check=True)
            output = subprocess.run([str(directory / "reference")], input=inputs, text=True, capture_output=True, check=True)
            values = output.stdout.split()
            assert len(values) == len(data)
            for row, value in zip(data, values):
                if args.write:
                    row[column] = value
                else:
                    # Different platform libm implementations can round exp/log differently.
                    actual = struct.unpack(">Q", struct.pack(">d", float(value)))[0]
                    expected = struct.unpack(">Q", struct.pack(">d", float(row[column])))[0]
                    allowance = 0 if platform.system() == "Darwin" else 16
                    assert abs(actual - expected) <= allowance, (row, value, fused)
    if args.write:
        with FIXTURE.open("w", encoding="utf-8", newline="") as output:
            csv.writer(output, lineterminator="\n").writerows(rows)
    print(f"RSS 6: {len(data)} current-Fortran cases pass with and without FMA")


if __name__ == "__main__":
    main()
