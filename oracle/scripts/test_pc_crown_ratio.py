#!/usr/bin/env python3
"""Run the checked-in ThreeDCanopy with real non-unit crown aspect ratios."""

from pathlib import Path
import tempfile
import subprocess

ROOT = Path(__file__).resolve().parents[2]
root = ROOT
src = (root / "vendor/CoLM202X/main/MOD_3DCanopyRadiation.F90").read_text(
    encoding="utf-8"
)
start = src.index("   SUBROUTINE ThreeDCanopy(ps, pe,")
tail = src[start : src.index("END MODULE MOD_3DCanopyRadiation")]
prefix = """module MOD_Precision
integer, parameter :: r8=selected_real_kind(12), r16=selected_real_kind(33)
end module
module ref_pc
use MOD_Precision
implicit none
contains
"""
driver = """program reference
use ref_pc
implicit none
integer :: canlay(3)=[1,2,3], i
real(r8) :: fc(3)=[.3_r8,.4_r8,.3_r8], size(3)=[1._r8,3._r8,5._r8], height(3)=[1.2_r8,5._r8,12._r8]
real(r8) :: ratio(3)=[.5_r8,1._r8,2._r8], chil(3)=[.1_r8,.2_r8,.3_r8], lsai(3)=[2._r8,3._r8,4._r8]
real(r8) :: rho(3,2), tau(3,2), agd(2)=[.1_r8,.2_r8], agi(2)=[.15_r8,.25_r8]
real(r8) :: albd(3,2),albi(3,2),fabd(3,2),fabi(3,2),ftdd(3,2),ftid(3,2),ftii(3,2),fadd(3,2)
real(r8) :: psun(3),fsid(3),fsii(3),thermk(3),shade(3)
rho(:,1)=[.1_r8,.12_r8,.14_r8]; rho(:,2)=[.3_r8,.32_r8,.34_r8]
tau(:,1)=[.05_r8,.06_r8,.07_r8]; tau(:,2)=[.2_r8,.21_r8,.22_r8]
call ThreeDCanopy(1,3,canlay,fc,size,height,ratio,chil,.6_r8,lsai,rho,tau,agd,agi, &
 albd,albi,fabd,fabi,ftdd,ftid,ftii,fadd,psun,fsid,fsii,thermk,shade)
write(*,'(6(es25.17e3,1x))') albd(1,:),albi(1,:),ftdd(1,:)
write(*,'(4(es25.17e3,1x))') ftid(1,:),ftii(1,:)
write(*,'(6(es25.17e3,1x))') fabd
write(*,'(6(es25.17e3,1x))') fabi
end program
"""


def main():
    import csv
    import platform
    import shutil
    import struct

    compiler = shutil.which("gfortran")
    if not compiler:
        raise SystemExit("gfortran is required for the PC crown-ratio reference")
    lines = (
        (ROOT / "oracle/fixtures/pc_crown_reference.csv")
        .read_text(encoding="utf-8")
        .splitlines()
    )
    expected = list(
        csv.reader(line for line in lines if line and not line.startswith("#"))
    )
    with tempfile.TemporaryDirectory(prefix="pc-crown-ref-") as work:
        d = Path(work)
        (d / "reference.f90").write_text(
            prefix + tail + "end module\n" + driver, encoding="utf-8"
        )
        for fused, column in [(True, 1), (False, 2)]:
            flags = [
                "-O2",
                "-fdefault-real-8",
                "-fdefault-double-8",
                "-ffree-line-length-none",
                "-ffp-contract=fast" if fused else "-ffp-contract=off",
            ]
            if fused and platform.machine().lower() in ("x86_64", "amd64"):
                flags.append("-mfma")
            subprocess.run(
                [compiler]
                + flags
                + [str(d / "reference.f90"), "-o", str(d / "reference")],
                cwd=d,
                check=True,
            )
            result = subprocess.run(
                [str(d / "reference")], check=True, capture_output=True, text=True
            )
            actual = [float(value) for value in result.stdout.split()]
            assert len(actual) == len(expected), (len(actual), len(expected))
            max_ulp = 0
            for value, row in zip(actual, expected):
                reference = float(row[column])
                got = struct.unpack(">Q", struct.pack(">d", value))[0]
                want = struct.unpack(">Q", struct.pack(">d", reference))[0]
                error = abs(got - want)
                max_ulp = max(max_ulp, error)
                assert error <= (0 if platform.system() == "Darwin" else 16), (
                    row[0],
                    value,
                    reference,
                    fused,
                    error,
                )
            print(
                f"PC crown reference {'fused' if fused else 'unfused'}: {len(actual)} outputs, max {max_ulp} ULP"
            )


if __name__ == "__main__":
    main()
