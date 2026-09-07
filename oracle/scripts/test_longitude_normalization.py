#!/usr/bin/env python3
"""Run the production longitude routine, including inputs that formerly hung."""
from pathlib import Path
import math
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "vendor/CoLM202X/share"


def main():
    text = (SOURCE / "MOD_Utils.F90").read_text()
    routine = re.search(r"(?is)\bSUBROUTINE normalize_longitude\s*\(.*?END SUBROUTINE normalize_longitude", text).group()
    with tempfile.TemporaryDirectory(prefix="colm-longitude-") as directory:
        work = Path(directory)
        (work / "probe.f90").write_text(
            "module MOD_SPMD_Task\ncontains\nsubroutine CoLM_stop(message)\n"
            "character(*), intent(in) :: message\nprint *, message\nerror stop 1\n"
            "end subroutine\nend module\n"
            "module test_utils\ncontains\n" + routine + "\nend module\n"
            "program probe\nuse MOD_Precision\nuse test_utils\nimplicit none\n"
            "real(r8) :: lon\ncharacter(80) :: argument\ncall get_command_argument(1,argument)\n"
            "read(argument,*) lon\ncall normalize_longitude(lon)\nprint '(ES26.17)', lon\nend program\n"
        )
        executable = work / "probe"
        subprocess.run(["gfortran", str(SOURCE / "MOD_Precision.F90"), "probe.f90", "-o", str(executable)], cwd=work, check=True)
        # Warm the same executable before a hang check, excluding macOS first-exec verification.
        subprocess.run([str(executable), "0"], cwd=work, check=True, capture_output=True, timeout=300)
        for value in [0.0, -0.0, 123.5, -179.999, 180.0, -180.0, 359.5, -359.5, 720.125, 1e12]:
            result = subprocess.run([str(executable), repr(value)], cwd=work, check=True, capture_output=True, text=True, timeout=5)
            actual = float(result.stdout)
            expected = value if -180 <= value < 180 else (value + 180) % 360 - 180
            assert actual == expected, (value, actual, expected)
            if value == 0:
                assert math.copysign(1, actual) == math.copysign(1, value)
        for value in ["1e36", "-1e36", "NaN", "Infinity", "-Infinity"]:
            result = subprocess.run([str(executable), value], cwd=work, capture_output=True, text=True, timeout=5)
            assert result.returncode != 0 and "longitude" in result.stdout.lower(), result
    print("longitude: normal coordinates preserved; large and invalid inputs cannot hang")


if __name__ == "__main__":
    main()
